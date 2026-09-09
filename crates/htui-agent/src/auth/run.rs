//! One login, end to end (plan MOD-21 D9): the operation `AcpDriver::authenticate` delegates to.
//!
//! The wire half ([`crate::acp::auth`]) starts at an open pair of byte streams and ends at an
//! outcome. Everything on either side of that — which launch to spawn, what to put in its
//! environment before it starts, what to do with the lines it writes to stderr while a human is in
//! a browser, and when to give up on that human — is here, because none of it is ACP. A CLI
//! transport that one day grows a login of its own reuses this shape without importing a schema.
//!
//! **What T3 built and what T5 adds.** Today this resolves the launch, spawns it, and runs the
//! wire over the result: the flow's `events` carries the agent's method list and nothing else.
//! [`AuthFlow::browser`] and [`AuthFlow::idle`] are carried and unread — the browser policy is a
//! value written into this frame's own copy of the launch, and the idle clock is a `select!` over
//! the stderr tap, and both arrive with T5. They are named in the destructuring below rather than
//! ignored wholesale so that the day they are read is a change to two lines and not to a signature.
//!
//! **One owner for the child.** The `ChildGuard` lives in [`crate::acp::auth::run`]'s frame, so
//! dropping *this* future drops that one and the guard's `Drop` signals the process group
//! (blueprint H-1). There is deliberately no second guard here: two owners of one child is how a
//! kill gets skipped.

use crate::acp::{AcpDriver, AcpIo, AuthSource, HANDSHAKE_TIMEOUT, WireFlow};
use crate::auth::{AuthFlow, AuthOutcome};
use crate::error::Result;

/// Logs one agent in (or out) over its own protocol, off any session.
///
/// In order:
///
/// 1. `AcpDriver::auth_source` for the flow's `cwd` — the row's launch resolved (the probe's
///    recording when it is still usable, D58), or the prepared pair a test handed the driver.
/// 2. A spawn over that launch, and its streams as an [`AcpIo`]. The launch is this frame's own
///    value and dies with it: what T5's browser policy writes into it reaches that one child and
///    is never recorded in `agent_box.probe.resolved` (blueprint H-11).
/// 3. [`crate::acp::run_auth`] over the pair, bounded at the handshake by
///    [`HANDSHAKE_TIMEOUT`] and at nothing else but the flow's token (plan D3, D13).
///
/// The outcome comes back exactly as the wire reported it. In particular a JSON-RPC error to the
/// call is [`AuthOutcome::Refused`] and an `Ok`, because it names what the user has to do next
/// (plan D5), and none of the five outcomes says anything about whether the box is now usable —
/// that is the probe's verdict and this operation writes no row at all (plan D6, `R-AGT-6`).
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
    // T5 reads both: `browser` is applied to the launch below, and `idle` runs the clock over the
    // stderr tap that does not exist yet. Naming them here keeps the flow's shape settled.
    let _ = (idle, browser);

    let io = match driver.auth_source(&cwd).await? {
        AuthSource::Launch(launch) => {
            // T5: `browser.apply(&mut launch)` goes here, and the tap is taken off `spawned`
            // *before* `from_spawned` moves it (blueprint H-21).
            let spawned = crate::launch::spawn(&launch, &cwd).await?;
            AcpIo::from_spawned(spawned)?
        }
        // A duplex has no process, no environment and no stderr: there is nothing between the
        // source and the wire for this arm to do.
        #[cfg(feature = "test-support")]
        AuthSource::Prepared(io) => io,
    };

    crate::acp::run_auth(
        io,
        driver.acp_settings(),
        WireFlow {
            events,
            choice,
            cancel,
            handshake_timeout: HANDSHAKE_TIMEOUT,
        },
    )
    .await
}
