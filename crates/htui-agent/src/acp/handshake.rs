//! Tier 2 of the box probe: `initialize` and nothing else (`docs/ANA-4.md` §4.6, plan MOD-2 D49).
//!
//! One task owns the whole `connect_with` future, `block_task()` is called exactly once, and the
//! child is owned by the **caller's** frame — not by the task — so a timeout, a connection-actor
//! failure, a success and a caller that drops this future mid-await all reach the same kill.
//! [`run_session`] keeps its child inside the task (`acp/mod.rs`) because a session has commands to
//! serve after the handshake and the task outlives the call that started it; a probe has neither.
//!
//! Deliberately absent: `session/new`, a prompt, the mapper, the recorder and every `Inbound`
//! handler. A probe that opened a session would have to invent a `cwd` and a prompt it has no
//! business inventing, and would leave a session on the agent's side for a question that was only
//! ever "does this binary run".
//!
//! [`run_session`]: super::open_session

use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, InitializeResponse};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::acp::{AcpIo, client};
use crate::error::{DriverError, Result};
use crate::launch::{AcpSettings, ChildGuard};

/// What `initialize` answered: the `handshake` object of the `agent_box.probe` snapshot
/// (ANA-4 §4.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    /// When the response arrived, truncated to microseconds — `TIMESTAMPTZ`'s resolution, the same
    /// precision the recorder stamps its own rows with.
    pub at: DateTime<Utc>,
    /// `protocolVersion` as the agent echoed it. ANA-4 §4.6 warns that the echo is not proof of
    /// support; `htui` pins 1 and records what came back.
    pub protocol_version: u16,
    /// `agentInfo.name`.
    pub agent_name: Option<String>,
    /// `agentInfo.version` — the version the `agent_box` row records when tier 2 ran.
    pub agent_version: Option<String>,
    /// `agentCapabilities` in the SDK's own serde form (camelCase keys), verbatim.
    ///
    /// Reshaping it would be a second schema for a value no consumer reads: MOD-4's skip predicate
    /// and ANA-5's box profile both read `status` and nothing else.
    pub capabilities: Value,
    /// `authMethods[].id`, in the order the agent listed them. Empty for the `claude` adapter;
    /// non-empty is what makes a box `unauthenticated` rather than `ready` (plan D50).
    pub auth_methods: Vec<String>,
}

impl Handshake {
    /// The response, as the snapshot records it.
    ///
    /// `capabilities` falls back to [`Value::Null`] if the SDK's own type will not serialise:
    /// a capability document `htui` cannot render is still a box that answered `initialize`.
    #[must_use]
    pub fn from_response(init: &InitializeResponse, at: DateTime<Utc>) -> Self {
        Self {
            at,
            protocol_version: init.protocol_version.as_u16(),
            agent_name: init.agent_info.as_ref().map(|info| info.name.clone()),
            agent_version: init.agent_info.as_ref().map(|info| info.version.clone()),
            capabilities: serde_json::to_value(&init.agent_capabilities).unwrap_or(Value::Null),
            auth_methods: init
                .auth_methods
                .iter()
                .map(|method| method.id().0.to_string())
                .collect(),
        }
    }
}

/// Completes `initialize` over `io` and kills `io.child` on **every** exit path.
///
/// The child is taken out before the task is spawned and held in a [`ChildGuard`] whose `Drop`
/// signals it, so even a caller that drops this future mid-await (an aborted probe task at
/// shutdown) leaves no running process behind. The reader and the writer move into the task with
/// the request built
/// exactly as the session path builds it — [`ProtocolVersion::V1`], the row's
/// `client_capabilities`, this binary's `client_info` — and the foreground future sends the
/// outcome on a `oneshot` and returns, which is what ends `connect_with`.
///
/// # Errors
/// [`DriverError::Transport`]: `initialize failed: {err}` plus the child's stderr tail on a new
/// line when there is one; `the agent did not complete its handshake within {timeout:?}` on
/// timeout;
/// `the agent ended before answering initialize` when the connection actor failed first and
/// dropped the foreground future. In every case the child has been killed **and reaped** before
/// this returns.
pub async fn handshake(io: AcpIo, settings: &AcpSettings, timeout: Duration) -> Result<Handshake> {
    let AcpIo {
        reader,
        writer,
        child,
    } = io;
    let mut guard = ChildGuard::new(child);

    let (answer_tx, answer_rx) = oneshot::channel();
    let capabilities = client::client_capabilities(&settings.client_capabilities);
    let task = tokio::spawn(async move {
        let transport = ByteStreams::new(writer.compat_write(), reader.compat());
        let connected = Client
            .builder()
            .name("htui")
            .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
                let initialize = InitializeRequest::new(ProtocolVersion::V1)
                    .client_capabilities(capabilities)
                    .client_info(client::client_info());
                // ANA-4 §4.2 risk 11: `block_task` only here, and there is nothing else here.
                let outcome = cx.send_request(initialize).block_task().await;
                // A receiver that has gone away is the timeout arm having given up; the connection
                // ends either way, which is what returning from this closure does.
                let _ = answer_tx.send(outcome);
                Ok(())
            })
            .await;
        if let Err(err) = connected {
            tracing::debug!(%err, "the probe's ACP connection ended with an error");
        }
    });

    let outcome = match tokio::time::timeout(timeout, answer_rx).await {
        Err(_) => {
            task.abort();
            // `{:?}`, not `as_secs()`: a sub-second timeout renders as `within 0s` otherwise, which
            // reads as "it was never given a chance" in the one message whose whole job is to say
            // how long the agent had.
            Err(DriverError::Transport(format!(
                "the agent did not complete its handshake within {timeout:?}"
            )))
        }
        // The sender was dropped without being used: `connect_with` gave up on the foreground
        // future before `initialize` was answered.
        Ok(Err(_)) => Err(DriverError::Transport(
            "the agent ended before answering initialize".to_owned(),
        )),
        Ok(Ok(Err(err))) => Err(handshake_error(&err, &guard)),
        Ok(Ok(Ok(response))) => Ok(Handshake::from_response(
            &response,
            Utc::now().trunc_subsecs(6),
        )),
    };

    // Before the answer reaches the caller, whatever it is: "the probe is done" and "the adapter is
    // gone" are one fact, and the probe spawns more processes than a chat does.
    guard.kill_and_reap().await;
    // The connection task ends on its own once the streams close, but "on its own" is after this
    // function has already told its caller the child is killed and reaped. Aborting makes the
    // doc's guarantee literal on the three non-timeout paths too; the timeout arm aborted above,
    // and a second `abort()` on a finished task is a no-op.
    task.abort();
    outcome
}

/// An `initialize` failure, with the child's captured stderr appended when there is one — the shape
/// `acp/mod.rs`'s `handshake_error` already gives the session path.
fn handshake_error(err: &agent_client_protocol::Error, guard: &ChildGuard) -> DriverError {
    let stderr = guard.stderr_tail().join("\n");
    if stderr.is_empty() {
        DriverError::Transport(format!("initialize failed: {err}"))
    } else {
        DriverError::Transport(format!("initialize failed: {err}\n{stderr}"))
    }
}
