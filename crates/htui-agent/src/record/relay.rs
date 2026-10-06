//! MOD-42 plan D6: one turn of a session, with the permission relay and the run's control;
//! MOD-70 plan D6: the turns a step's queued follow-ups add to its live session.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{
    FOLLOW_UP_SESSION_CANCELLED, FOLLOW_UP_SESSION_ENDED, FollowUpSettle, OpenPermission,
    PermissionChoice, PermissionId, PermissionStatus, QueuedFollowUp, RelayOption, RelayOptionKind,
    RelaySessionId, RunCommandId, RunId, SettleOutcome, StepId, StepPermission,
    executor_scrub_refusal,
};
use htui_core::scrub::{Scrubber, Unmasked};
use htui_core::store::{Result as StoreResult, StoreError};
use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

use super::{AnsweredBy, Recorder, enforce_breach};
use crate::driver::{AgentSession, PermissionAnswer, PermissionPolicy, PermissionRequestId};
use crate::error::DriverError;
use crate::event::{
    DoneEvent, DriverEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent,
    StopReason, ToolCallEvent,
};

/// D8: how often a parked request's row is read. One primary-key read per tick, only while parked.
pub const RELAY_POLL: Duration = Duration::from_secs(1);
/// B-16: the grace a cancel `drive` discovers itself (a `stale`/`cancelled` row) is given.
pub const RELAY_GRACE: Duration = Duration::from_secs(2);
/// B-11: how long past the grace the post-cancel drain may run.
const DRAIN_SLACK: Duration = Duration::from_secs(1);
/// D8: the consecutive transient failures (`Unreachable`, `Backend`) of a parked row's read that
/// end the park; every one before it is a `warn` and the poll goes on, as the lease heartbeat
/// rides out a store blip (D123). Half a minute at [`RELAY_POLL`].
const TRANSIENT_READS: u32 = 30;

/// What a walk is asked to do (D10, D11). `Copy`, read with [`Control::signal`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Signal {
    /// Carry on.
    #[default]
    Run,
    /// Stop gracefully: answer parked requests `cancelled`, `session.cancel(grace)`, drain.
    Cancel {
        /// The window `AgentSession::cancel` is given.
        grace: Duration,
    },
}

impl Signal {
    /// Whether this is [`Signal::Cancel`].
    #[must_use]
    pub const fn is_cancel(self) -> bool {
        matches!(self, Self::Cancel { .. })
    }
}

/// One walk's view of its run's [`Signal`] (D10): a `watch` receiver, or none ("never").
#[derive(Debug, Clone)]
pub struct Control(Option<watch::Receiver<Signal>>);

impl Control {
    /// A control no signal ever reaches: `pump`, and every engine no runtime cancels.
    #[must_use]
    pub const fn never() -> Self {
        Self(None)
    }

    /// Over a receiver of the run's channel.
    #[must_use]
    pub const fn new(receiver: watch::Receiver<Signal>) -> Self {
        Self(Some(receiver))
    }

    /// The current signal. R-8: `changed()` is edge-triggered, so every wait reads this first.
    #[must_use]
    pub fn signal(&self) -> Signal {
        self.0.as_ref().map_or(Signal::Run, |rx| *rx.borrow())
    }

    /// Resolves when the value changes. Pending for ever for [`Control::never`] and once the
    /// sender is gone (a dropped sender is not a cancel).
    pub async fn changed(&mut self) {
        // Not a match guard: a binding is immutable inside its guard, and `changed` takes `&mut`.
        if let Some(rx) = &mut self.0
            && rx.changed().await.is_ok()
        {
            return;
        }
        std::future::pending::<()>().await;
    }
}

/// A fresh channel at [`Signal::Run`] and a control over it (tests, the fake orchestrator).
#[must_use]
pub fn control_channel() -> (watch::Sender<Signal>, Control) {
    let (sender, receiver) = watch::channel(Signal::Run);
    (sender, Control::new(receiver))
}

/// What `drive` needs to relay a stage-3 request (D6). Built per session by the engine.
pub struct Relay<'a, R: htui_core::store::RelayStore> {
    /// The executor's store.
    pub store: &'a R,
    /// The executor's lease owner (`EngineParts::owner`), the row's `owner` (I-2).
    pub owner: Uuid,
    /// The run.
    pub run: RunId,
    /// The step the session drives.
    pub step: StepId,
    /// This session (minted per `drive_once`).
    pub session: RelaySessionId,
    /// The agent's policy, stages 1-2 (D9).
    pub policy: &'a PermissionPolicy,
    /// Parked-row poll interval: [`RELAY_POLL`] in production (D8, B-18).
    pub poll: Duration,
    /// B-16's grace.
    pub grace: Duration,
    /// The instant source for the rows `htui` authors (`record_permission_answer`'s `at`).
    pub now: &'a (dyn Fn() -> DateTime<Utc> + Sync),
    /// MOD-70 D6: whether this session opens a follow-up window; `drive_once` sets it for main
    /// and candidate sessions (D8).
    pub follow_ups: bool,
}

impl<R: htui_core::store::RelayStore> core::fmt::Debug for Relay<'_, R> {
    /// The ids and the timings. The store, the policy and the clock are not printed: a store need
    /// not be `Debug`, and neither of the other two is what a relay log is about.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Relay")
            .field("owner", &self.owner)
            .field("run", &self.run)
            .field("step", &self.step)
            .field("session", &self.session)
            .field("poll", &self.poll)
            .field("grace", &self.grace)
            .field("follow_ups", &self.follow_ups)
            .finish_non_exhaustive()
    }
}

/// `pump`'s relay type (D6): uninhabited, so `None::<&Relay<'_, NoRelay>>` names a relay that
/// cannot exist.
#[derive(Debug, Clone, Copy)]
pub enum NoRelay {}

impl htui_core::store::RelayStore for NoRelay {
    async fn open_permission(&self, _open: OpenPermission) -> StoreResult<PermissionId> {
        match *self {}
    }

    async fn permission(&self, _id: PermissionId) -> StoreResult<Option<StepPermission>> {
        match *self {}
    }

    async fn apply_permission(
        &self,
        _id: PermissionId,
        _owner: Uuid,
    ) -> StoreResult<Option<PermissionChoice>> {
        match *self {}
    }

    async fn settle_permissions(
        &self,
        _session: RelaySessionId,
        _to: PermissionStatus,
    ) -> StoreResult<u64> {
        match *self {}
    }

    async fn open_follow_ups(
        &self,
        _run: RunId,
        _step: StepId,
        _session: RelaySessionId,
        _owner: Uuid,
    ) -> StoreResult<bool> {
        match *self {}
    }

    async fn next_follow_up(
        &self,
        _step: StepId,
        _session: RelaySessionId,
    ) -> StoreResult<Option<QueuedFollowUp>> {
        match *self {}
    }

    async fn settle_follow_up(
        &self,
        _id: RunCommandId,
        _owner: Uuid,
        _to: FollowUpSettle,
    ) -> StoreResult<SettleOutcome> {
        match *self {}
    }

    async fn close_follow_ups(
        &self,
        _step: StepId,
        _session: RelaySessionId,
        _reason: &str,
    ) -> StoreResult<u64> {
        match *self {}
    }

    async fn close_dropped_follow_ups(
        &self,
        _run: RunId,
        _owner: Uuid,
        _reason: &str,
    ) -> StoreResult<u64> {
        match *self {}
    }
}

/// D6: one turn of `session` into `recorder`, ending at its `done`, a graceful cancel, or an error.
///
/// Without a relay it behaves exactly as `pump` always has: a `permission_request` is recorded
/// and the pull goes on, so the transport raises "is parked". With one: stage 1-2 policy answers
/// at once (`by: policy`); stage 3 flushes the recorder, opens a scrubbed row (I-5) and polls it
/// at `relay.poll` until it is answered (applied under the lease, D4, echoed `by: user`), read
/// back `stale`/`cancelled` (a cancel), or `control` signals a cancel. The control is read at the
/// top of every iteration and selected against every pull (B-2).
///
/// **Rows (B-15).** A graceful cancel settles the session's rows `cancelled` inside its sequence
/// (I-7). Every other exit that opened a row marks the leftovers `stale` (D5), best-effort: a
/// failure there is a `warn`, never the returned error. A fenced exit is the one exception: the
/// lease is another process's, and a fenced writer writes nothing more (MOD-40 D1), so an
/// answered row it could not apply stays `answered`.
///
/// **The cancel (I-7).** `session.cancel(grace)` first — ACP answers every parked responder
/// `cancelled` itself, the fake drops it — then the parked request's `permission_answer {option_id:
/// null, by: "policy", cancelled: true}`, which no transport writes, then the rows, then a drain of
/// what the cancel produced up to the turn's `done`, bounded by `grace` plus one second (B-11).
/// A request the drain pulls was emitted before the cancel (ACP parks a request when it emits it,
/// so its cancel answered that one too): it is recorded with the same `cancelled` answer after
/// it. Every store write of the sequence is best-effort (I-6): a failure is a `warn` and the
/// answer stays `Cancelled`, unless it is a fence.
///
/// **A parked poll** rides out up to [`TRANSIENT_READS`] consecutive `Unreachable`/`Backend`
/// failures of its row's read, each a `warn`; the next one is the answer.
///
/// **Follow-ups (MOD-70 D6).** With `relay.follow_ups`, the step's window opens before the first
/// pull (`false` is a fence) and, at each turn's `done` that is not a cancel or a breach, the
/// step's pending follow-up is pre-scrubbed, claimed under the lease, recorded at `turn + 1` and
/// sent; the next turn is driven the same way. Every exit but a fence closes the window, refusing
/// what is still queued. Without it, `drive` makes no follow-up call (I-8).
///
/// # Errors
/// [`DriverError::Cancelled`] after a graceful cancel (I-7); [`DriverError::Store`]
/// `(Fenced { step })` when an answer can no longer be applied under the lease (D4), a write
/// of the cancel sequence is fenced, or the follow-up window's open or a follow-up's claim is
/// (MOD-70 D6); [`DriverError::Closed`] for a stream that ended without `done`; the recorder's
/// failures; a follow-up's `send_follow_up` failure.
pub async fn drive<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
) -> Result<DoneEvent, DriverError> {
    let mut opened = false;
    // MOD-70 D6 step 1: before the first pull.
    let window = match relay {
        Some(relay) if relay.follow_ups => open_window(relay).await?,
        _ => Window::Off,
    };
    // D6 steps 2-7.
    let out = turns(session, recorder, relay, control, &mut opened, window).await;
    if let Some(relay) = relay
        && opened
        && settles_stale(&out)
        && let Err(err) = htui_core::store::RelayStore::settle_permissions(
            relay.store,
            relay.session,
            PermissionStatus::Stale,
        )
        .await
    {
        tracing::warn!(%err, "the session's leftover permission rows were not marked stale");
    }
    // D6 step 8: every exit but a fence (MOD-40 D1), after the session ended and before
    // `drive_once`'s `recorder.finish`, verify, capture, `finish_step` and gate (I-6). `drive`
    // cannot tell a deadline cut from a run cancel: both close with the cancelled sentence.
    if let Some(relay) = relay
        && window != Window::Off
        && !is_fenced(&out)
    {
        let reason = match &out {
            Err(DriverError::Cancelled) => FOLLOW_UP_SESSION_CANCELLED,
            _ => FOLLOW_UP_SESSION_ENDED,
        };
        close_window(relay, window, reason).await;
    }
    out
}

/// MOD-70 D6, B-10: this session's follow-up window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Window {
    /// No relay, or `follow_ups: false`: no follow-up store call at all (I-8).
    Off,
    /// `open_follow_ups` answered `true`: checked at every turn end, closed with retries.
    Open,
    /// `open_follow_ups` failed (not fenced): never checked, one close attempted (B-10).
    Unknown,
}

/// D6 step 1: opens the step's window under the lease. `false` is the walk's fence; any other
/// failure is a `warn`, and the session runs without a window (an enqueue is then refused
/// `NotStarted`; the step itself is unaffected).
async fn open_window<R: htui_core::store::RelayStore>(
    relay: &Relay<'_, R>,
) -> Result<Window, DriverError> {
    match htui_core::store::RelayStore::open_follow_ups(
        relay.store,
        relay.run,
        relay.step,
        relay.session,
        relay.owner,
    )
    .await
    {
        Ok(true) => Ok(Window::Open),
        Ok(false) | Err(StoreError::Fenced { .. }) => {
            Err(StoreError::Fenced { step: relay.step }.into())
        }
        Err(err) => {
            tracing::warn!(
                %err,
                "the step's follow-up window did not open; this session takes no follow-up"
            );
            Ok(Window::Unknown)
        }
    }
}

/// The turns of one session (D6 steps 2-7): a turn, then — with an open window — the step's
/// queued follow-up recorded and sent, and the next turn, until a turn end finds none.
async fn turns<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
    opened: &mut bool,
    window: Window,
) -> Result<DoneEvent, DriverError> {
    loop {
        // B-20: boxed per turn, so the loop adds no `turn`-sized slot to `drive`'s future.
        let done = Box::pin(turn(session, recorder, relay, control, opened)).await?;
        let Some(relay) = relay.filter(|_| window == Window::Open) else {
            return Ok(done);
        };
        let Some(queued) = take_follow_up(recorder, relay, control, &done).await? else {
            return Ok(done);
        };
        // D6 step 6 (PRD Q1, R-HIS-1): `turn + 1`, scrubbed by the scrubber the pre-scrub used,
        // so `record_follow_up`'s `refuse` path is unreachable from here.
        recorder
            .record_follow_up(&queued.text, (relay.now)())
            .await?;
        // D6 step 7: a failed send ends the session as a failed turn would.
        session.send_follow_up(queued.text).await?;
    }
}

/// D6 steps 3-5 at one turn end: the row claimed for the next turn, or `None` to end the
/// session. Only a fence is an error (B-11).
async fn take_follow_up<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    recorder: &Recorder<'_, S>,
    relay: &Relay<'_, R>,
    control: &mut Control,
    done: &DoneEvent,
) -> Result<Option<QueuedFollowUp>, DriverError> {
    loop {
        // D6 step 3, D7: a cancelled stop (a breach's included), a breach or a cancel applies
        // nothing; the window's close refuses the row.
        if done.stop_reason == StopReason::Cancelled
            || recorder.cap_breach().is_some()
            || control.signal().is_cancel()
        {
            return Ok(None);
        }
        let Some(queued) = next_follow_up(relay, control).await else {
            return Ok(None);
        };
        // D6 step 4: the executor's scrubber over the payload `record_follow_up` scrubs.
        let to = match prescrub(recorder.scrubber, &queued.text) {
            // B-12: a cancel that landed during the read claims nothing.
            Ok(()) if control.signal().is_cancel() => return Ok(None),
            Ok(()) => FollowUpSettle::Applied,
            Err(unmasked) => FollowUpSettle::Refused(executor_scrub_refusal(unmasked.rule)),
        };
        let applying = to == FollowUpSettle::Applied;
        // D6 step 5.
        match htui_core::store::RelayStore::settle_follow_up(
            relay.store,
            queued.id,
            relay.owner,
            to,
        )
        .await
        {
            Ok(SettleOutcome::Settled) if applying => return Ok(Some(queued)),
            // Refused here (the executor's scrub), or by a cancel or a newer window: look again.
            Ok(SettleOutcome::Settled | SettleOutcome::NotPending) => {}
            Ok(SettleOutcome::Fenced) | Err(StoreError::Fenced { .. }) => {
                return Err(StoreError::Fenced { step: relay.step }.into());
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    "a follow-up was not claimed; the session ends and its window's close \
                     refuses it"
                );
                return Ok(None);
            }
        }
    }
}

/// B-11: the step's pending follow-up, riding out up to [`TRANSIENT_READS`] consecutive
/// `Unreachable`/`Backend` failures at `relay.poll`, selected against the control. `None` when
/// nothing is pending, on a cancel, on exhaustion or on any other error (each failure a `warn`).
async fn next_follow_up<R: htui_core::store::RelayStore>(
    relay: &Relay<'_, R>,
    control: &mut Control,
) -> Option<QueuedFollowUp> {
    let mut failures = 0_u32;
    loop {
        match htui_core::store::RelayStore::next_follow_up(relay.store, relay.step, relay.session)
            .await
        {
            Ok(next) => return next,
            Err(err @ (StoreError::Unreachable(_) | StoreError::Backend(_)))
                if failures + 1 < TRANSIENT_READS =>
            {
                failures += 1;
                tracing::warn!(%err, failures, "the step's follow-up was not read; reading again");
            }
            Err(err) => {
                tracing::warn!(%err, "the step's follow-up was not read; the session ends");
                return None;
            }
        }
        tokio::select! {
            biased;
            () = control.changed() => {}
            () = tokio::time::sleep(relay.poll) => {}
        }
        if control.signal().is_cancel() {
            return None;
        }
    }
}

/// D6 step 4: `{"text": text}` through `scrubber`, the payload `record_follow_up` scrubs. The
/// masked copy is dropped: only the verdict matters here.
fn prescrub(scrubber: &dyn Scrubber, text: &str) -> Result<(), Unmasked> {
    scrubber.scrub(&mut json!({ "text": text }))
}

/// D6 step 8, B-10: `close_follow_ups` with `reason`, best-effort. An [`Window::Open`] window
/// retries `Unreachable`/`Backend` up to [`TRANSIENT_READS`] attempts at `relay.poll`; an
/// [`Window::Unknown`] one is tried once. A final failure is a `warn` (R-3): the window stays open
/// over what it holds until the engine settles the run terminal (review M-3), a cancel refuses it,
/// or a lease re-take closes it (D9).
async fn close_window<R: htui_core::store::RelayStore>(
    relay: &Relay<'_, R>,
    window: Window,
    reason: &str,
) {
    let attempts = if window == Window::Open {
        TRANSIENT_READS
    } else {
        1
    };
    let mut failures = 0_u32;
    loop {
        match htui_core::store::RelayStore::close_follow_ups(
            relay.store,
            relay.step,
            relay.session,
            reason,
        )
        .await
        {
            Ok(_) => return,
            Err(err @ (StoreError::Unreachable(_) | StoreError::Backend(_)))
                if failures + 1 < attempts =>
            {
                failures += 1;
                tracing::warn!(%err, failures, "the step's follow-up window was not closed; retrying");
                tokio::time::sleep(relay.poll).await;
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    "the step's follow-up window was not closed; the run's end, a cancel or a \
                     lease re-take refuses what it holds"
                );
                return;
            }
        }
    }
}

/// `Err(Store(Fenced))`: the one exit that writes nothing more (MOD-40 D1).
const fn is_fenced(out: &Result<DoneEvent, DriverError>) -> bool {
    matches!(out, Err(DriverError::Store(StoreError::Fenced { .. })))
}

/// B-15: whether an exit marks the session's leftover rows `stale`. Not a cancel (it settled them
/// `cancelled` itself) and not a fence (the run is no longer this process's to write).
const fn settles_stale(out: &Result<DoneEvent, DriverError>) -> bool {
    !matches!(
        out,
        Err(DriverError::Cancelled | DriverError::Store(StoreError::Fenced { .. }))
    )
}

/// The loop of [`drive`]; `opened` says whether a relay row of this session was written.
async fn turn<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
    opened: &mut bool,
) -> Result<DoneEvent, DriverError> {
    // The calls seen so far, by id: what a policy rule matches on and what the summary names.
    let mut calls: HashMap<String, ToolCallEvent> = HashMap::new();
    loop {
        // R-8, B-2: a cancel sent before this call, or while the last event was being recorded.
        if let Signal::Cancel { grace } = control.signal() {
            return cancel(session, recorder, relay, *opened, None, grace).await;
        }
        // Both transports' `next_event` is cancel-safe (an mpsc `recv`; the fake resolves at its
        // first poll), so a cancel that wins drops nothing.
        let pulled = tokio::select! {
            biased;
            () = control.changed() => None,
            pulled = session.next_event() => Some(pulled),
        };
        // The control changed: the loop top reads the new value.
        let Some(pulled) = pulled else { continue };
        let Some(envelope) = pulled? else {
            return Err(DriverError::Closed);
        };
        let done = match &envelope.event {
            DriverEvent::Done(done) => Some(*done),
            _ => None,
        };
        // Only with a relay: the call (for policy matching and the summary) and the request.
        let (call, request) = match (&envelope.event, relay.is_some()) {
            (DriverEvent::ToolCall(call), true) => (Some(call.clone()), None),
            (DriverEvent::PermissionRequest(request), true) => (None, Some(request.clone())),
            _ => (None, None),
        };
        if let Some(breach) = recorder.record(envelope).await? {
            return enforce_breach(session, recorder, breach).await;
        }
        if let Some(done) = done {
            return Ok(done);
        }
        // `pump`: pull on, and the transport says "parked".
        let Some(relay) = relay else { continue };
        if let Some(call) = call {
            calls.insert(call.tool_call_id.clone(), call);
        }
        if let Some(request) = request {
            match park(session, recorder, relay, control, &calls, &request, opened).await? {
                Parked::Resumed => {}
                Parked::Cancel { grace } => {
                    return cancel(
                        session,
                        recorder,
                        Some(relay),
                        *opened,
                        Some(&request.request_id),
                        grace,
                    )
                    .await;
                }
            }
        }
    }
}

/// How a request [`park`] handled left the turn.
enum Parked {
    /// Answered (by policy or by a client): pull on.
    Resumed,
    /// Cancel the session with this grace.
    Cancel {
        /// The window `AgentSession::cancel` is given.
        grace: Duration,
    },
}

/// One request: stages 1-2 answer at once (D9); stage 3 is relayed and waited on (D3-D5, D8).
async fn park<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: &Relay<'_, R>,
    control: &mut Control,
    calls: &HashMap<String, ToolCallEvent>,
    request: &PermissionRequestEvent,
    opened: &mut bool,
) -> Result<Parked, DriverError> {
    let call = request.tool_call_id.as_ref().and_then(|id| calls.get(id));
    // Stages 1-2 (D9): the agent's own policy, as chat's `run_turn` does.
    if let Some(answer) = crate::permission::evaluate(relay.policy, call, &request.options) {
        session
            .answer_permission(
                request.request_id.clone(),
                PermissionAnswer::Selected(answer.option_id.clone()),
            )
            .await?;
        recorder
            .record_permission_answer(
                &request.request_id,
                Some(&answer.option_id),
                AnsweredBy::Policy,
                false,
                (relay.now)(),
            )
            .await?;
        tracing::info!(
            stage = ?answer.stage,
            reason = %answer.reason,
            "a permission request was answered by policy"
        );
        return Ok(Parked::Resumed);
    }

    // Stage 3: the request row is durable before a relay row names it.
    recorder.flush().await?;
    let (summary, options) = scrubbed(recorder.scrubber, call, &request.options);
    let id = PermissionId::new();
    htui_core::store::RelayStore::open_permission(
        relay.store,
        OpenPermission {
            id,
            run_id: relay.run,
            run_step_id: relay.step,
            session: relay.session,
            request_id: request.request_id.as_str().to_owned(),
            tool_call_id: request.tool_call_id.clone(),
            summary,
            options,
            owner: relay.owner,
        },
    )
    .await?;
    *opened = true;

    // The consecutive transient read failures so far ([`TRANSIENT_READS`]).
    let mut failures = 0_u32;
    loop {
        if let Signal::Cancel { grace } = control.signal() {
            return Ok(Parked::Cancel { grace });
        }
        let status = match htui_core::store::RelayStore::permission(relay.store, id).await {
            Ok(row) => {
                failures = 0;
                let row = row.ok_or_else(|| StoreError::NotFound {
                    entity: "step_permission",
                    id: id.to_string(),
                })?;
                Some(row.status)
            }
            Err(err @ (StoreError::Unreachable(_) | StoreError::Backend(_)))
                if failures + 1 < TRANSIENT_READS =>
            {
                failures += 1;
                tracing::warn!(%err, failures, "a parked request's row was not read; polling on");
                None
            }
            Err(err) => return Err(err.into()),
        };
        match status {
            None | Some(PermissionStatus::Pending) => {}
            Some(PermissionStatus::Answered) => {
                // D4: apply under the lease, answer the live session, echo through the recorder.
                // `None` right after the row read `answered` is the lease gone (B-16).
                let Some(choice) =
                    htui_core::store::RelayStore::apply_permission(relay.store, id, relay.owner)
                        .await?
                else {
                    return Err(StoreError::Fenced { step: relay.step }.into());
                };
                session
                    .answer_permission(
                        request.request_id.clone(),
                        PermissionAnswer::Selected(choice.option_id.clone()),
                    )
                    .await?;
                recorder
                    .record_permission_answer(
                        &request.request_id,
                        Some(&choice.option_id),
                        AnsweredBy::User,
                        false,
                        (relay.now)(),
                    )
                    .await?;
                return Ok(Parked::Resumed);
            }
            // B-16: superseded by a newer session of the step, or cancelled by someone else.
            Some(PermissionStatus::Stale | PermissionStatus::Cancelled) => {
                return Ok(Parked::Cancel { grace: relay.grace });
            }
            // Applied, and not by this call: the row is no longer this session's to apply.
            Some(PermissionStatus::Applied) => {
                return Err(StoreError::Fenced { step: relay.step }.into());
            }
        }
        tokio::select! {
            biased;
            () = control.changed() => {}
            () = tokio::time::sleep(relay.poll) => {}
        }
    }
}

/// The graceful cancel (D6, I-7; see [`drive`]'s doc for the order).
async fn cancel<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    opened: bool,
    parked: Option<&PermissionRequestId>,
    grace: Duration,
) -> Result<DoneEvent, DriverError> {
    if let Err(err) = session.cancel(grace).await {
        tracing::warn!(%err, "the cancel took the kill path; the closing rows are written anyway");
    }
    if let Some(relay) = relay {
        if let Some(request_id) = parked {
            answer_cancelled(recorder, relay, request_id).await?;
        }
        if opened {
            let settled = htui_core::store::RelayStore::settle_permissions(
                relay.store,
                relay.session,
                PermissionStatus::Cancelled,
            )
            .await;
            best_effort(
                settled.map(drop).map_err(DriverError::from),
                "the session's permission rows were not settled cancelled",
            )?;
        }
    }
    // B-11 bounds the pulls, never a recording: `Recorder::flush` is not cancel-safe (it numbers
    // its rows before it appends them), so a `record` the bound interrupted would lose a row whose
    // `seq` is already spent. A recording in flight completes; the next pull sees the bound.
    let deadline = tokio::time::Instant::now() + grace.saturating_add(DRAIN_SLACK);
    loop {
        // `None` is the bound. Checked before the pull too: `timeout_at` polls its future first,
        // so a session that is always ready would never time out.
        let pulled = if tokio::time::Instant::now() < deadline {
            tokio::time::timeout_at(deadline, session.next_event())
                .await
                .ok()
        } else {
            None
        };
        match pulled {
            Some(Ok(Some(envelope))) => {
                let done = matches!(envelope.event, DriverEvent::Done(_));
                // I-7: a request the session emitted before the cancel and `drive` had not pulled
                // yet. ACP parked it at the emit, so its cancel answered it `cancelled` with the
                // rest; no transport writes that answer.
                let drained = match (&envelope.event, relay) {
                    (DriverEvent::PermissionRequest(request), Some(relay)) => {
                        Some((request.request_id.clone(), relay))
                    }
                    _ => None,
                };
                // A breach verdict here is spent: the session is already closing.
                let recorded = recorder.record(envelope).await;
                best_effort(
                    recorded.map(drop).map_err(DriverError::from),
                    "an event drained after the cancel was not recorded",
                )?;
                if let Some((request_id, relay)) = drained {
                    answer_cancelled(recorder, relay, &request_id).await?;
                }
                if done {
                    break;
                }
            }
            Some(Ok(None) | Err(_)) => break,
            None => {
                tracing::warn!("the cancelled session did not end its turn within the grace");
                break;
            }
        }
    }
    Err(DriverError::Cancelled)
}

/// I-7's row for `request_id`: `permission_answer {option_id: null, by: "policy", cancelled:
/// true}`, best-effort ([`best_effort`]).
async fn answer_cancelled<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    recorder: &mut Recorder<'_, S>,
    relay: &Relay<'_, R>,
    request_id: &PermissionRequestId,
) -> Result<(), DriverError> {
    let recorded = recorder
        .record_permission_answer(request_id, None, AnsweredBy::Policy, true, (relay.now)())
        .await;
    best_effort(
        recorded.map_err(DriverError::from),
        "a cancelled request's answer was not recorded",
    )
}

/// I-6 inside the cancel sequence: a store failure there is a `warn`, so a graceful cancel still
/// ends `Cancelled` rather than as a failed session the walk would settle. A refused flush keeps
/// the rows it numbered owed, landed by the recorder's next flush (MOD-40 D3); an event refused
/// before it was buffered is lost with the `warn`, as it was with the error. A row left `pending`
/// is unanswerable once the lease is gone (D3, D5). A fence is the exception: the lease is another
/// process's, and a fenced writer writes nothing more (MOD-40 D1).
fn best_effort(result: Result<(), DriverError>, what: &str) -> Result<(), DriverError> {
    match result {
        Err(err @ DriverError::Store(StoreError::Fenced { .. })) => Err(err),
        Err(err) => {
            tracing::warn!(%err, "{what}");
            Ok(())
        }
        Ok(()) => Ok(()),
    }
}

/// B-14: `"<tool_kind>: <title>"` and the labels, scrubbed together as one document by the
/// recorder's scrubber. Fail-closed on [`Unmasked`](htui_core::scrub::Unmasked): no summary, and
/// each label becomes its kind's text. Option ids are never scrubbed: they are sent back verbatim.
fn scrubbed(
    scrubber: &dyn Scrubber,
    call: Option<&ToolCallEvent>,
    options: &[PermissionOption],
) -> (Option<String>, Vec<RelayOption>) {
    let summary = call.map(|call| format!("{}: {}", call.tool_kind.as_str(), call.title));
    let mut document = json!({
        "summary": summary,
        "labels": options.iter().map(|option| option.label.as_str()).collect::<Vec<_>>(),
    });
    let masked = match scrubber.scrub(&mut document) {
        Ok(()) => Some(document),
        Err(unmasked) => {
            tracing::warn!(%unmasked, "a relayed request's summary and labels were not persisted");
            None
        }
    };
    let summary = masked
        .as_ref()
        .and_then(|document| document.get("summary"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let labels = masked
        .as_ref()
        .and_then(|document| document.get("labels"))
        .and_then(Value::as_array);
    let options = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let kind = relay_kind(option.kind);
            let label = labels
                .and_then(|labels| labels.get(index))
                .and_then(Value::as_str)
                .map_or_else(|| kind.as_str().to_owned(), str::to_owned);
            RelayOption {
                id: option.id.clone(),
                label,
                kind,
            }
        })
        .collect();
    (summary, options)
}

/// `PermissionOptionKind` → `RelayOptionKind`, one arm per variant.
const fn relay_kind(kind: PermissionOptionKind) -> RelayOptionKind {
    match kind {
        PermissionOptionKind::AllowOnce => RelayOptionKind::AllowOnce,
        PermissionOptionKind::AllowAlways => RelayOptionKind::AllowAlways,
        PermissionOptionKind::RejectOnce => RelayOptionKind::RejectOnce,
        PermissionOptionKind::RejectAlways => RelayOptionKind::RejectAlways,
    }
}
