//! One login, end to end (plan MOD-21 D9): the operation `AcpDriver::authenticate` delegates to.
//!
//! The wire half ([`crate::acp::auth`]) starts at an open pair of byte streams and ends at an
//! outcome. Everything on either side of that — which launch to spawn, what to put in its
//! environment before it starts, what to do with the lines it writes to stderr while a human is in
//! a browser, and when to give up on that human — is here, because none of it is ACP. A CLI
//! transport that one day grows a login of its own reuses this shape without importing a schema.
//!
//! **The loop is the whole of it.** Between the spawn and the outcome this frame owns four things
//! at once: the wire's future, the child's stderr, the caller's choice, and a clock. They are one
//! `select!` because each of the last three is a *sign of life* that has to restart the clock, and
//! a clock that ran anywhere else would be measuring elapsed time rather than silence (D13).
//!
//! **One owner for the child.** The `ChildGuard` lives in [`crate::acp::auth::run`]'s frame, so
//! dropping *this* future drops that one and the guard's `Drop` signals the process group
//! (blueprint H-1). There is deliberately no second guard here: two owners of one child is how a
//! kill gets skipped.

use std::collections::HashSet;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::acp::{AcpDriver, AcpIo, AuthSource, HANDSHAKE_TIMEOUT, WireFlow};
use crate::auth::{AuthEvent, AuthFlow, AuthOutcome, first_url};
use crate::error::Result;

/// How long the forwarding loop keeps reading stderr after the wire has answered.
///
/// Not a wait in the normal case and not meant to be one: every exit of the wire that returns has
/// already killed **and reaped** its child, so the tap is at end-of-stream by the time this runs
/// and the drain costs a poll. What the bound buys is the abnormal case — a grandchild that
/// inherited the pipe and outlived the kill would otherwise park a flow that has already answered.
/// The lines themselves are worth waiting a moment for: an adapter prints its link and then
/// answers, and a `Url` event dropped on the floor is the one the user was about to open.
const DRAIN_GRACE: Duration = Duration::from_millis(250);

/// Logs one agent in (or out) over its own protocol, off any session.
///
/// In order:
///
/// 1. `AcpDriver::auth_source` for the flow's `cwd` — the row's launch resolved (the probe's
///    recording when it is still usable, D58), or the prepared pair a test handed the driver.
/// 2. [`crate::auth::BrowserPolicy::apply`] on that launch, then a spawn, then the child's stderr
///    tapped **before** its streams are taken: `AcpIo::from_spawned` moves the `Spawned`, so a tap
///    asked for afterwards would have nothing to ask and a link printed during `initialize` would
///    be gone (blueprint H-21). The launch is this frame's own value and dies with it: what the
///    policy writes into it reaches that one child and is never recorded (H-11).
/// 3. [`crate::acp::run_auth`] over the pair, bounded at the handshake by [`HANDSHAKE_TIMEOUT`]
///    and at nothing else but a **child** of the flow's token, so that "the user cancelled" and
///    "the clock cancelled" stay distinguishable at the outcome (plan D3, D13).
/// 4. The loop: every tapped line becomes an [`AuthEvent::Line`] and every link it carries for the
///    first time an [`AuthEvent::Url`] (D15); the caller's choice is forwarded once; and each of
///    those restarts the idle sleep. When the sleep wins instead, the child token is cancelled and
///    the wire's [`AuthOutcome::Cancelled`] is reported as [`AuthOutcome::Idle`].
///
/// The outcome comes back otherwise exactly as the wire reported it. In particular a JSON-RPC
/// error to the call is [`AuthOutcome::Refused`] and an `Ok`, because it names what the user has to
/// do next (plan D5), and none of the five outcomes says anything about whether the box is now
/// usable — that is the probe's verdict and this operation writes no row at all (D6, `R-AGT-6`).
///
/// **Where the clock starts.** At the spawn, not at the first event: [`AuthEvent::Methods`] is sent
/// by the wire straight to the caller's channel and never crosses this frame. Nothing is lost by
/// that — the only thing between the spawn and that event is `initialize`, which
/// [`HANDSHAKE_TIMEOUT`] already bounds at a small fraction of any usable idle cap.
///
/// # Errors
/// [`DriverError::Unresolved`](crate::error::DriverError::Unresolved) and
/// [`DriverError::Transport`](crate::error::DriverError::Transport) from resolving the launch;
/// [`DriverError::Spawn`](crate::error::DriverError::Spawn) naming the command when the box cannot
/// start it; and [`crate::acp::auth::run`]'s transport failures, each of which has already killed
/// and reaped the child before it returns.
pub async fn authenticate(driver: &AcpDriver, flow: AuthFlow) -> Result<AuthOutcome> {
    let AuthFlow {
        cwd,
        events,
        choice,
        cancel,
        idle,
        browser,
    } = flow;

    let (io, mut stderr) = match driver.auth_source(&cwd).await? {
        AuthSource::Launch(mut launch) => {
            browser.apply(&mut launch);
            let mut spawned = crate::launch::spawn(&launch, &cwd).await?;
            // Blueprint H-21: the tap first, because the next line moves the child.
            let tapped = spawned.tap_stderr();
            (AcpIo::from_spawned(spawned)?, tapped)
        }
        // A duplex has no process, no environment and no stderr: there is nothing between the
        // source and the wire for this arm to do, and a closed receiver is how the loop's stderr
        // arm retires on its first poll rather than being written twice (blueprint H-24).
        #[cfg(feature = "test-support")]
        AuthSource::Prepared(io) => (io, closed_tap()),
    };

    // A child of the flow's token: a cancel from the pane trips both, and a trip from the clock
    // below trips only this one, which is what lets the outcome tell them apart.
    let clock = cancel.child_token();
    let (inner_tx, inner_rx) = oneshot::channel();
    let wire = crate::acp::run_auth(
        io,
        driver.acp_settings(),
        WireFlow {
            events: events.clone(),
            choice: inner_rx,
            cancel: clock.clone(),
            handshake_timeout: HANDSHAKE_TIMEOUT,
        },
    );
    tokio::pin!(wire);

    let mut choice = choice;
    let mut inner_tx = Some(inner_tx);
    let mut choosing = true;
    let mut listening = true;
    let mut seen = HashSet::new();
    let mut idle_fired = None;

    let answered = loop {
        // A fresh sleep per iteration *is* the reset: every other arm below is a sign of life, and
        // reaching this line again means one of them just happened.
        let quiet = tokio::time::sleep(idle);
        tokio::pin!(quiet);

        tokio::select! {
            biased;
            answered = &mut wire => break answered,
            line = stderr.recv(), if listening => match line {
                Some(line) => forward(&events, line, &mut seen),
                // End of stream, which a prepared transport reaches immediately: stop polling an
                // arm that will never speak again rather than spinning on `None`.
                None => listening = false,
            },
            chosen = &mut choice, if choosing => {
                choosing = false;
                // A choice that never came — the caller dropped its sender — drops ours too, and
                // the wire answers `Declined` for the same reason its own receiver would have.
                if let (Ok(chosen), Some(inner)) = (chosen, inner_tx.take()) {
                    let _ = inner.send(chosen);
                }
            }
            () = &mut quiet, if idle_fired.is_none() => {
                idle_fired = Some(idle);
                clock.cancel();
            }
        }
    };

    if listening {
        let _ = tokio::time::timeout(DRAIN_GRACE, async {
            while let Some(line) = stderr.recv().await {
                forward(&events, line, &mut seen);
            }
        })
        .await;
    }

    match (answered, idle_fired) {
        (Ok(AuthOutcome::Cancelled), Some(after)) => Ok(AuthOutcome::Idle { after }),
        (answered, _) => answered,
    }
}

/// One stderr line to the caller, and the link it carries if this flow has not shown it yet.
///
/// The line goes first and goes unconditionally: the pane shows what the adapter said whether or
/// not a link was found in it, and the `Url` event is the extra that lights up the `o` key.
/// A closed receiver is not a failure here — the caller has gone away, and its token will follow.
fn forward(events: &mpsc::UnboundedSender<AuthEvent>, line: String, seen: &mut HashSet<String>) {
    let url = first_url(&line).filter(|url| seen.insert(url.clone()));
    let _ = events.send(AuthEvent::Line(line));
    if let Some(url) = url {
        let _ = events.send(AuthEvent::Url(url));
    }
}

/// A tap that is already at end-of-stream, for a transport that has no stderr to tap.
#[cfg(feature = "test-support")]
fn closed_tap() -> mpsc::UnboundedReceiver<String> {
    let (_sender, receiver) = mpsc::unbounded_channel();
    receiver
}
