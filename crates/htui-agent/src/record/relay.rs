//! MOD-42 plan D6: one turn of a session, with the permission relay and the run's control.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{
    OpenPermission, PermissionChoice, PermissionId, PermissionStatus, RelayOption, RelayOptionKind,
    RelaySessionId, RunId, StepId, StepPermission,
};
use htui_core::scrub::Scrubber;
use htui_core::store::{Result as StoreResult, StoreError};
use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

use super::{AnsweredBy, Recorder, enforce_breach};
use crate::driver::{AgentSession, PermissionAnswer, PermissionPolicy, PermissionRequestId};
use crate::error::DriverError;
use crate::event::{
    DoneEvent, DriverEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent,
    ToolCallEvent,
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
/// # Errors
/// [`DriverError::Cancelled`] after a graceful cancel (I-7); [`DriverError::Store`]
/// `(Fenced { step })` when an answer can no longer be applied under the lease (D4), or a write
/// of the cancel sequence is fenced; [`DriverError::Closed`] for a stream that ended without
/// `done`; the recorder's failures.
pub async fn drive<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
) -> Result<DoneEvent, DriverError> {
    let mut opened = false;
    let out = turn(session, recorder, relay, control, &mut opened).await;
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
    out
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
