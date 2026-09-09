//! The wire half of a login (plan MOD-21 D12): `initialize`, the live method list, one
//! `authenticate` or `logout`, and the child killed on **every** exit.
//!
//! [`handshake`] with two more exits — the human's cancel and the human's choice — and the second
//! `send_request` site outside [`open_session`]. The kill discipline is copied from it in shape and
//! not merely in spirit: the `ChildGuard` lives in this frame, the connection task owns the streams
//! only, and every path out of here runs `kill_and_reap().await` and then `task.abort()` before it
//! answers its caller. What makes this child worth the care is how long it lives — a login can be
//! minutes of a human in a browser, with the adapter holding a loopback listener open the whole
//! time — so "the request failed" and "the adapter is gone" have to be one fact here too.
//!
//! Every SDK type a flow needs is spoken here and nowhere else under `auth/`: the transport-neutral
//! half (`crate::auth`) is ids, sentences and statuses, so a CLI transport that one day grows a
//! login of its own produces the same [`AuthEvent`]s without importing a schema.
//!
//! **Deadlock rule** (ANA-4 §4.2 risk 11): `SentRequest::block_task()` is called from the
//! `connect_with` foreground future and from nowhere else. There are two request *sites* below —
//! `initialize` and the one call — and the call has two spellings, so the literal appears three
//! times, all of them inside the one future. No dispatch handler is registered at all, which is
//! also what answers an `elicitation/create` from an agent that ignores the capabilities `htui`
//! advertises: the SDK answers it "method not found" itself, so the agent hears a refusal rather
//! than this flow hanging on a request it has no handler for (plan D21).
//!
//! [`handshake`]: super::handshake()
//! [`open_session`]: super::open_session

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthMethod, AuthMethodId, AuthenticateRequest, InitializeRequest, InitializeResponse,
    LogoutRequest,
};
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, is_incoming_transport_closed,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tokio_util::sync::CancellationToken;

use crate::acp::{AcpIo, client};
use crate::auth::{AuthCall, AuthChoice, AuthEvent, AuthMethodInfo, AuthOutcome};
use crate::error::{DriverError, Result};
use crate::launch::{AcpSettings, ChildGuard};

/// What the wire needs of an [`AuthFlow`](crate::auth::AuthFlow).
///
/// The subset, not the whole: there is no `cwd` because the child is already spawned, no `idle`
/// because the clock belongs to the caller that owns the stderr tap, and no
/// [`BrowserPolicy`](crate::auth::BrowserPolicy) because a policy is applied to a launch and this
/// half never sees one. `cancel` is the caller's **child** token when that caller runs a clock of
/// its own, so one token is all this half ever watches.
#[derive(Debug)]
pub struct WireFlow {
    /// The method list, and every later event the wire produces.
    pub events: mpsc::UnboundedSender<AuthEvent>,
    /// The one answer to [`AuthEvent::Methods`]. Dropped unused is [`AuthOutcome::Declined`].
    pub choice: oneshot::Receiver<AuthChoice>,
    /// Tripped by the caller, and observed both inside the foreground future and outside it.
    pub cancel: CancellationToken,
    /// Bounds `initialize` **only**: [`HANDSHAKE_TIMEOUT`](super::HANDSHAKE_TIMEOUT) in
    /// production, milliseconds in a test.
    ///
    /// Nothing bounds the call itself but the token and the caller's idle clock (plan D3, D13): a
    /// human OAuth round trip is minutes of nothing, and a timeout that killed it would be a
    /// feature that only ever fires on the user.
    pub handshake_timeout: Duration,
}

/// What the foreground future reports back, before the outer frame has had its say.
///
/// The stderr tail is the outer frame's to read — the future has no guard — so the two carriers
/// that are owed one arrive as text and get it appended there.
#[derive(Debug)]
enum Wire {
    /// The flow reached an outcome. A [`AuthOutcome::Refused`] here carries the agent's `Display`
    /// text only.
    Answered(AuthOutcome),
    /// The token tripped while the future was waiting for a choice.
    Cancelled,
    /// The transport reached end of file with this request outstanding: the child died, and the
    /// error the SDK synthesised to unblock the caller is not the agent's word for anything.
    Ended(Stage),
    /// `initialize` was answered with a JSON-RPC error, rendered.
    HandshakeFailed(String),
    /// `initialize` was not answered within this long.
    HandshakeTimeout(Duration),
}

/// Which request the foreground future is parked on.
///
/// The one fact the `oneshot` cannot carry. A sender dropped **unused** says only that the
/// connection actor gave up on the foreground future, and the message that reports it has to name
/// what that future was waiting for — "the agent ended before answering logout" is a sentence a
/// user can act on and "the agent ended" is not. Shared as an atomic because the outer frame reads
/// it exactly once, after the future that writes it is already gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The handshake.
    Initialize,
    /// `authenticate`.
    Authenticate,
    /// `logout`.
    Logout,
}

impl Stage {
    /// The value stored in the shared atomic.
    const fn code(self) -> u8 {
        match self {
            Self::Initialize => 0,
            Self::Authenticate => 1,
            Self::Logout => 2,
        }
    }

    /// The stage a stored code names. An unknown code reads as the first stage: a message that
    /// under-reports how far a flow got is better than one that invents a call.
    const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Authenticate,
            2 => Self::Logout,
            _ => Self::Initialize,
        }
    }

    /// The protocol method this stage is waiting on.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Authenticate => "authenticate",
            Self::Logout => "logout",
        }
    }
}

/// Runs one login over `io` and kills `io.child` on **every** exit path (plan D12).
///
/// In order: `initialize` under [`WireFlow::handshake_timeout`]; the agent's own method list as
/// [`AuthEvent::Methods`]; the caller's choice raced against the token; then `authenticate` or
/// `logout`, whose JSON-RPC error is an *answer* — [`AuthOutcome::Refused`] — and not a failure,
/// because it names what the user has to do next (plan D5).
///
/// The child is taken out before the connection task is spawned and held in a `ChildGuard` in this
/// frame, so the caller dropping this future mid-await — an aborted login at shutdown — leaves no
/// running process either. The outer frame races the answer against the token as well as the
/// foreground future doing so, which is what lets a cancel reach the kill while that future is
/// parked on a request the agent will never answer.
///
/// Says nothing about whether the box is now usable: that is the probe's verdict and never this
/// one (plan D6, `R-AGT-6`).
///
/// # Errors
/// [`DriverError::Transport`]: `initialize failed: {err}` plus the child's stderr tail on a new
/// line when there is one; `the agent did not complete its handshake within {timeout:?}` on
/// timeout; `the agent ended before answering {initialize|authenticate|logout}` when the connection
/// actor failed first and dropped the foreground future, plus the same tail — the child's own last
/// words are usually the only account of why it died. In every case the child has been killed
/// **and reaped** before this returns.
pub async fn run(io: AcpIo, settings: &AcpSettings, flow: WireFlow) -> Result<AuthOutcome> {
    let AcpIo {
        reader,
        writer,
        child,
    } = io;
    let mut guard = ChildGuard::new(child);
    let WireFlow {
        events,
        choice,
        cancel,
        handshake_timeout,
    } = flow;

    let (answer_tx, answer_rx) = oneshot::channel();
    let capabilities = client::client_capabilities(&settings.client_capabilities);
    let stage = Arc::new(AtomicU8::new(Stage::Initialize.code()));
    let waiting_on = Arc::clone(&stage);
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        let transport = ByteStreams::new(writer.compat_write(), reader.compat());
        let connected = Client
            .builder()
            .name("htui")
            .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
                let initialize = InitializeRequest::new(ProtocolVersion::V1)
                    .client_capabilities(capabilities)
                    .client_info(client::client_info());
                // ANA-4 §4.2 risk 11: `block_task()` from the foreground future, which is here.
                let handshake = cx.send_request(initialize).block_task();
                let wire = match tokio::time::timeout(handshake_timeout, handshake).await {
                    Err(_) => Wire::HandshakeTimeout(handshake_timeout),
                    Ok(Err(err)) => Wire::HandshakeFailed(err.to_string()),
                    Ok(Ok(init)) => {
                        let (methods, logout, hidden) = methods_of(&init);
                        // A closed receiver is not a failure: the caller is gone, and its cancel is
                        // already on its way to the arm below.
                        let _ = events.send(AuthEvent::Methods {
                            methods,
                            logout,
                            hidden: hidden.clone(),
                        });
                        // The choice is awaited *here*, inside the future, because this is the only
                        // place the SDK lets a request be sent from — and it is raced against the
                        // token so a cancel during the chooser ends the connection cleanly rather
                        // than leaving this future parked on a `oneshot` nobody will ever fire.
                        let chosen = tokio::select! {
                            // Biased towards the cancel: once the token has tripped, no new request
                            // may go out, and a random arm order would decide that by coin toss on
                            // the one tick where both are ready.
                            biased;
                            () = token.cancelled() => None,
                            chosen = choice => Some(chosen),
                        };
                        match chosen {
                            None => Wire::Cancelled,
                            Some(Err(_)) => Wire::Answered(AuthOutcome::Declined),
                            Some(Ok(AuthChoice::Method(id))) => {
                                if let Some(refusal) = terminal_refusal(&id, &hidden) {
                                    refusal
                                } else {
                                    waiting_on.store(Stage::Authenticate.code(), Ordering::Relaxed);
                                    let request =
                                        AuthenticateRequest::new(AuthMethodId::new(id.as_str()));
                                    let call = AuthCall::Authenticate(id);
                                    // The second request site, in the same future as the first.
                                    let answer = cx.send_request(request).block_task().await;
                                    answered(call, answer.map(|_| ()), Stage::Authenticate)
                                }
                            }
                            Some(Ok(AuthChoice::Logout)) => {
                                waiting_on.store(Stage::Logout.code(), Ordering::Relaxed);
                                // The second request site again, in its other spelling.
                                let answer =
                                    cx.send_request(LogoutRequest::new()).block_task().await;
                                answered(AuthCall::Logout, answer.map(|_| ()), Stage::Logout)
                            }
                        }
                    }
                };
                // A receiver that has gone away is the outer frame having already answered its own
                // caller; the connection ends either way, which is what returning from here does.
                let _ = answer_tx.send(wire);
                Ok(())
            })
            .await;
        if let Err(err) = connected {
            tracing::debug!(%err, "the login's ACP connection ended with an error");
        }
    });

    let outcome = tokio::select! {
        // Biased towards the answer: an agent that has *already* answered has said something the
        // user asked for, and a shutdown arriving on the same tick should not turn a completed
        // login into a cancelled one. The kill below runs either way.
        biased;
        answer = answer_rx => match answer {
            Ok(Wire::Answered(AuthOutcome::Refused { call, message })) => {
                // Plan H-10: the refusal the user reads is the agent's own sentence, and the tail
                // is where an adapter that explains itself on stderr rather than in the error
                // object gets to be heard.
                Ok(AuthOutcome::Refused { call, message: with_tail(message, &guard) })
            }
            Ok(Wire::Answered(outcome)) => Ok(outcome),
            Ok(Wire::Cancelled) => Ok(AuthOutcome::Cancelled),
            Ok(Wire::Ended(stage)) => Err(DriverError::Transport(with_tail(
                format!("the agent ended before answering {}", stage.as_str()),
                &guard,
            ))),
            Ok(Wire::HandshakeFailed(err)) => Err(DriverError::Transport(with_tail(
                format!("initialize failed: {err}"),
                &guard,
            ))),
            // `{:?}`, not `as_secs()`: a sub-second timeout renders as `within 0s` otherwise, which
            // reads as "it was never given a chance" in the one message whose whole job is to say
            // how long the agent had.
            Ok(Wire::HandshakeTimeout(timeout)) => Err(DriverError::Transport(format!(
                "the agent did not complete its handshake within {timeout:?}"
            ))),
            // The sender was dropped without being used: `connect_with` gave up on the foreground
            // future — the connection actor failed first — before the call was answered.
            Err(_) => Err(DriverError::Transport(with_tail(
                format!(
                    "the agent ended before answering {}",
                    Stage::from_code(stage.load(Ordering::Relaxed)).as_str()
                ),
                &guard,
            ))),
        },
        // The foreground future watches the same token, but it can only observe it between awaits
        // it is allowed to abandon; parked on a request, it cannot. This arm is what makes a cancel
        // during `authenticate` reach the kill at once.
        () = cancel.cancelled() => Ok(AuthOutcome::Cancelled),
    };

    // Before the answer reaches the caller, whatever it is: a login that has ended and an adapter
    // that is gone are one fact, and this child outlives every other one this crate spawns.
    guard.kill_and_reap().await;
    // The connection task ends on its own once the streams close, but "on its own" is after this
    // function has already told its caller the child is killed and reaped. Aborting makes the
    // doc's guarantee literal; a second `abort()` on a finished task is a no-op.
    task.abort();
    outcome
}

/// The agent's `authMethods` split by kind, and whether it advertised a logout verb.
///
/// The three halves in the order [`AuthEvent::Methods`] carries them: the offerable methods in the
/// agent's own order, the logout flag, and the `terminal`-typed methods the chooser may name but
/// this client must never send (plan D4, D21 — the spec forbids passing one to `authenticate`, and
/// `htui` advertises no terminal capability to earn one in the first place).
///
/// `logout` reads `agent_capabilities.auth.logout.is_some()`: `auth` is a plain
/// `AgentAuthCapabilities` and not an `Option`, so there is one level of optionality here, not two.
fn methods_of(init: &InitializeResponse) -> (Vec<AuthMethodInfo>, bool, Vec<AuthMethodInfo>) {
    let mut methods = Vec::new();
    let mut hidden = Vec::new();
    for method in &init.auth_methods {
        let info = AuthMethodInfo {
            id: method.id().0.to_string(),
            name: method.name().to_owned(),
            description: method.description().map(ToOwned::to_owned),
        };
        match method {
            AuthMethod::Terminal(_) => hidden.push(info),
            AuthMethod::Agent(_) => methods.push(info),
            // Unreachable for wire data, and kept because the enum is `#[non_exhaustive]`: the
            // `Agent` arm is `#[serde(untagged)]`, so it swallows every unrecognised `type` before
            // this match ever sees one — measured, `{"type":"future-kind",…}` deserialises as
            // `Agent`. What can reach this arm is a future *Rust* variant of the schema, and
            // offering it is the schema's own default for a method whose kind is unstated.
            _ => methods.push(info),
        }
    }
    (
        methods,
        init.agent_capabilities.auth.logout.is_some(),
        hidden,
    )
}

/// The refusal a `terminal`-typed choice earns, or `None` when the id is offerable.
///
/// Refused *here* rather than trusted to the chooser: `hidden` is advisory to a UI and binding to
/// the wire, and a client that passed a terminal method to `authenticate` would be violating the
/// spec on the say-so of whatever built the choice. The id in the message is the agent's own, as
/// data — this file names no method (`R-AGT-5`).
fn terminal_refusal(id: &str, hidden: &[AuthMethodInfo]) -> Option<Wire> {
    hidden.iter().find(|method| method.id == id).map(|_| {
        Wire::Answered(AuthOutcome::Refused {
            call: AuthCall::Authenticate(id.to_owned()),
            message: format!(
                "`{id}` is a terminal login method: it needs an interactive terminal that htui \
                 does not provide"
            ),
        })
    })
}

/// What one answered call becomes.
///
/// A JSON-RPC error is [`AuthOutcome::Refused`] and not an `Err`: the agent answered, and what it
/// said is the whole value of asking (plan D5).
///
/// **Except when nobody answered.** When the transport reaches end of file with the request
/// outstanding — the adapter crashed, or exited on its own — the SDK synthesises an error to
/// unblock the caller rather than hanging it, and marks that error with a stable discriminator it
/// exposes as [`is_incoming_transport_closed`]. Reporting *that* as a refusal would put words in a
/// dead process's mouth: the user would read "the agent refused your login: Incoming transport
/// closed" and the runtime would write a row for an answer that never came. It is a
/// [`Wire::Ended`], which the outer frame renders with the child's own last words attached.
fn answered(
    call: AuthCall,
    answer: std::result::Result<(), agent_client_protocol::Error>,
    stage: Stage,
) -> Wire {
    match answer {
        Ok(()) => Wire::Answered(AuthOutcome::Completed { call }),
        Err(err) if is_incoming_transport_closed(&err) => Wire::Ended(stage),
        Err(err) => Wire::Answered(AuthOutcome::Refused {
            call,
            message: err.to_string(),
        }),
    }
}

/// `message` with the child's captured stderr appended when there is any — the shape
/// `handshake.rs`'s own error text already has.
fn with_tail(message: String, guard: &ChildGuard) -> String {
    let tail = guard.stderr_tail().join("\n");
    if tail.is_empty() {
        message
    } else {
        format!("{message}\n{tail}")
    }
}
