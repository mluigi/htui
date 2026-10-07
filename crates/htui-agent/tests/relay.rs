//! MOD-42 T2: `drive`, the permission relay and the run's control (plan D6, D8, I-5, I-7, I-8;
//! blueprint B-2, B-11, B-14, B-15, B-16).
//!
//! Every case drives a [`FakeSession`] through `drive` into a `MemStore::demo()` whose run was
//! created and claimed with an owner, as the store conformance's `leased_step` does. The "second
//! client" is a future joined with `drive` on the same task: it polls `relay_view(item)` on the
//! same store until a row appears and then answers, cancels or supersedes it, which is all a
//! Runs pane on another box can do (I-1: it writes no `session_event`). Waits run on a paused
//! clock with `Relay.poll` at 10 ms, so no case sleeps for real.
//!
//! MOD-70 T2 (plan D6, D7; blueprint §5.4) adds the follow-up loop's cases: a client queues a
//! follow-up while turn 0 is parked on a request (B-9's rendezvous), and two doubles reach the
//! edges no client can time — [`CancelAtDone`] and [`Hooked`]. Review M-4 gives [`Hooked`] a
//! failure switch per follow-up method and adds [`FailingSend`], for the loop's failure and retry
//! paths.

#![cfg(feature = "test-support")]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use htui_agent::conformance::{Script, ScriptEvent, epoch};
use htui_agent::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverFuture, PermissionAnswer, PermissionDefault,
    PermissionPolicy, PermissionRequestId, SessionSpec, ToolExposure,
};
use htui_agent::error::DriverError;
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, PermissionOption, PermissionOptionKind,
    PermissionRequestEvent, StopReason, ToolCallEvent, ToolKind, ToolResultEvent, ToolResultStatus,
    UsageEvent,
};
use htui_agent::fake::FakeDriver;
use htui_agent::permission::{PolicyStage, evaluate};
use htui_agent::persona::narrow;
use htui_agent::record::{
    Control, NoRelay, RELAY_GRACE, Recorder, Relay, RunCap, Signal, control_channel, drive, pump,
};
use htui_core::fixtures::ids;
use htui_core::model::persona::SnapshotPersona;
use htui_core::model::{
    AnswerOutcome, Claim, EventKind, FOLLOW_UP_SESSION_CANCELLED, FOLLOW_UP_SESSION_ENDED,
    FollowUpRefusal, FollowUpRequest, FollowUpSettle, FollowUpText, GraphSnapshot, Isolation,
    NewFollowUp, NewRun, NewRunStep, OpenPermission, PermissionChoice, PermissionId,
    PermissionStatus, QueuedFollowUp, RelayOptionKind, RelaySessionId, RunCommandId,
    RunCommandStatus, RunId, RunMode, SessionEvent, SettleOutcome, SnapshotGraph, SnapshotSettings,
    StepId, StepPermission, StepStatus, executor_scrub_refusal,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{
    MemStore, ReadStore, Result as StoreResult, StepFence, StoreError, WriteStore,
};
use serde_json::{Value, json};
use tokio::sync::watch;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// The secret every case's scrubber masks.
const SECRET: &str = "fake-secret-9f8e7d";

/// A credential no secret list can mask: the scrubber refuses it (`Unmasked`, fail-closed).
const RESIDUE: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwx";

/// `Relay.poll` in every case (B-18: a field so these cases can shorten it).
const POLL: Duration = Duration::from_millis(10);

/// How long a case lets `drive`, or a client, run on the paused clock before it calls it stuck.
const LIMIT: Duration = Duration::from_secs(60);

/// The gated call, its request and the two options it offers.
const CALL: &str = "call-1";
const REQUEST: &str = "perm-1";
const ALLOW: &str = "allow-once";
const REJECT: &str = "reject-once";

/// The instant every `htui`-authored row is stamped with.
fn at() -> DateTime<Utc> {
    epoch()
}

fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([SECRET.to_owned()])
}

/// The smallest snapshot `create_run` accepts (the store conformance's `run_snapshot`).
fn snapshot() -> GraphSnapshot {
    GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: ids::GRAPH_HTUI_FEAT,
            name: "feature".to_owned(),
            is_override: false,
        },
        topology: "sha256:relay".to_owned(),
        mode: RunMode::Manual,
        phases: Vec::new(),
        settings: SnapshotSettings {
            default_isolation: Isolation::Worktree,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            max_fan_out: 4,
            max_agents_per_run: 8,
        },
        scope: None,
        personas: Vec::new(),
    }
}

/// A run of `HTUI_ANA_2` claimed by `owner` for five minutes, with one step.
#[derive(Debug)]
struct Leased {
    store: MemStore,
    run: RunId,
    step: StepId,
    owner: Uuid,
}

/// [`Leased`] at the ids given, so two stores can hold the same run and step.
async fn leased_at(run: RunId, step: StepId) -> Leased {
    let store = MemStore::demo();
    let owner = Uuid::new_v4();
    store
        .create_run(NewRun {
            id: run,
            project_id: ids::PROJECT_HTUI,
            item_id: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            target_box_id: ids::BOX,
            started_by: ids::USER,
            graph_snapshot: snapshot(),
            repo_scope: Vec::new(),
            queued_at: at(),
            batch_id: None,
        })
        .await
        .expect("the run is created");
    assert_eq!(
        store
            .claim_run(run, ids::BOX, owner, at(), TimeDelta::minutes(5))
            .await
            .expect("the claim is answered"),
        Claim::Admitted,
        "the executor claims the run"
    );
    store
        .create_step(NewRunStep {
            id: step,
            run_id: run,
            position: 0,
            attempt: 1,
            fanout_index: 0,
            phase_name: "implement".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE),
            model: Some("opus".to_owned()),
        })
        .await
        .expect("the step is created");
    Leased {
        store,
        run,
        step,
        owner,
    }
}

async fn leased() -> Leased {
    leased_at(RunId::new(), StepId::new()).await
}

/// The relay every case drives with: the fixture's run and step, a fresh session, `policy`.
fn relay<'a>(fx: &'a Leased, policy: &'a PermissionPolicy) -> Relay<'a, MemStore> {
    relay_over(&fx.store, fx, policy)
}

/// [`relay`] over another relay store.
fn relay_over<'a, R: htui_core::store::RelayStore>(
    store: &'a R,
    fx: &Leased,
    policy: &'a PermissionPolicy,
) -> Relay<'a, R> {
    Relay {
        store,
        owner: fx.owner,
        run: fx.run,
        step: fx.step,
        session: RelaySessionId::new(),
        policy,
        poll: POLL,
        grace: RELAY_GRACE,
        now: &at,
        follow_ups: false,
    }
}

/// The step's recorder, writing under the executor's lease (MOD-40 D1), as `drive_once`'s does.
fn fenced_recorder<'a>(fx: &'a Leased, scrubber: &'a MinimalScrubber) -> Recorder<'a, MemStore> {
    Recorder::new(&fx.store, scrubber, fx.step, false, None).with_fence(StepFence::Lease(fx.owner))
}

fn spec(step: StepId) -> SessionSpec {
    SessionSpec {
        agent_id: ids::AGENT_CLAUDE,
        step_id: step,
        cwd: PathBuf::from("."),
        extra_dirs: Vec::new(),
        env: BTreeMap::new(),
        model: None,
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        retain_raw: false,
        resume: None,
        budget_micros: None,
        prompt: None,
    }
}

/// The gated call: an `execute` whose title carries `secret`.
fn call(title: &str) -> ScriptEvent {
    call_of(title, ToolKind::Execute)
}

/// [`call`] of another kind (MOD-26: a persona denies by kind).
fn call_of(title: &str, kind: ToolKind) -> ScriptEvent {
    ScriptEvent::Emit(call_event_of(title, kind))
}

/// [`call`]'s event.
fn call_event(title: &str) -> DriverEvent {
    call_event_of(title, ToolKind::Execute)
}

/// [`call_of`]'s event.
fn call_event_of(title: &str, kind: ToolKind) -> DriverEvent {
    DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: CALL.to_owned(),
        title: title.to_owned(),
        tool_kind: kind,
        input: json!({ "command": "cargo test" }),
        locations: Vec::new(),
    })
}

/// The `reviewer` seed persona as a run freezes it (MOD-26 D7, D9): it denies `edit`.
fn reviewer() -> SnapshotPersona {
    let row = htui_core::model::persona::seed_rows(at())
        .into_iter()
        .find(|row| row.name == "reviewer")
        .expect("the reviewer seed");
    SnapshotPersona::freeze(&row).expect("a seed freezes")
}

/// The request for [`CALL`], with an allow and a reject option labelled `allow_label`/"Reject".
fn park(allow_label: &str) -> ScriptEvent {
    ScriptEvent::ParkPermission(permission_request(REQUEST, allow_label))
}

/// [`park`]'s request under `request_id`.
fn permission_request(request_id: &str, allow_label: &str) -> PermissionRequestEvent {
    PermissionRequestEvent {
        request_id: PermissionRequestId::new(request_id),
        tool_call_id: Some(CALL.to_owned()),
        options: vec![
            PermissionOption {
                id: ALLOW.to_owned(),
                label: allow_label.to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: REJECT.to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ],
    }
}

fn finished() -> Vec<ScriptEvent> {
    vec![
        ScriptEvent::Emit(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: CALL.to_owned(),
            status: ToolResultStatus::Completed,
            output: None,
            locations: Vec::new(),
            terminal_reason: None,
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ]
}

/// A turn that asks for [`CALL`] and finishes once it is answered.
fn answered_script() -> Script {
    let mut events = vec![call(&format!("run the suite with {SECRET}")), park("Allow")];
    events.extend(finished());
    Script::one_turn(events)
}

/// A turn that asks for [`CALL`] and ends only by `cancel`.
fn cancelled_script() -> Script {
    Script::one_turn(vec![
        call("run the suite"),
        park("Allow"),
        ScriptEvent::ExpectCancel,
    ])
}

async fn fake(script: Script, step: StepId) -> Box<dyn AgentSession> {
    FakeDriver::scripted(script)
        .start(spec(step), "summarise the backlog".to_owned())
        .await
        .expect("the fake starts")
}

// ---------------------------------------------------------------------------------------------
// A recording wrapper over the session
// ---------------------------------------------------------------------------------------------

/// What [`Watched`] saw: every `cancel` grace and every answer it was sent.
#[derive(Debug, Default)]
struct Seen {
    cancels: Mutex<Vec<Duration>>,
    answers: Mutex<Vec<(PermissionRequestId, PermissionAnswer)>>,
}

impl Seen {
    fn cancels(&self) -> Vec<Duration> {
        self.cancels.lock().expect("not poisoned").clone()
    }

    fn answers(&self) -> Vec<(PermissionRequestId, PermissionAnswer)> {
        self.answers.lock().expect("not poisoned").clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Forward everything to the fake.
    Forward,
    /// Forward, except that every `answer_permission` answers `Closed`.
    FailAnswers,
    /// No fake: `next_event` pends until `cancel`, then a `done { cancelled }` and the end.
    Stall,
    /// No fake: `next_event` pends for ever, `cancel` or not (a turn that never ends).
    Hang,
    /// No fake: `next_event` pends until `cancel`, then a `done { cancelled }`, then pends for
    /// ever: ACP's stream outlives the turn.
    Linger,
}

#[derive(Debug)]
struct Watched {
    inner: Option<Box<dyn AgentSession>>,
    mode: Mode,
    seen: Arc<Seen>,
    cancelled: bool,
    closing: Option<DriverEnvelope>,
}

fn watched(inner: Option<Box<dyn AgentSession>>, mode: Mode) -> (Watched, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let session = Watched {
        inner,
        mode,
        seen: Arc::clone(&seen),
        cancelled: false,
        closing: None,
    };
    (session, seen)
}

impl AgentSession for Watched {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.inner.as_ref().and_then(|inner| inner.session_ref())
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            if let Some(inner) = &mut self.inner {
                return inner.next_event().await;
            }
            if self.cancelled {
                match (self.mode, self.closing.take()) {
                    (Mode::Stall, closing) => return Ok(closing),
                    (Mode::Linger, Some(closing)) => return Ok(Some(closing)),
                    _ => {}
                }
            }
            std::future::pending::<Result<Option<DriverEnvelope>, DriverError>>().await
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            match &mut self.inner {
                Some(inner) => inner.send_follow_up(text).await,
                None => Err(DriverError::Closed),
            }
        })
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            self.seen
                .answers
                .lock()
                .expect("not poisoned")
                .push((request_id.clone(), answer.clone()));
            match (&mut self.inner, self.mode) {
                (Some(inner), Mode::Forward) => inner.answer_permission(request_id, answer).await,
                _ => Err(DriverError::Closed),
            }
        })
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            self.seen.cancels.lock().expect("not poisoned").push(grace);
            if let Some(inner) = &mut self.inner {
                return inner.cancel(grace).await;
            }
            if !self.cancelled {
                self.cancelled = true;
                self.closing = Some(DriverEnvelope {
                    event: DriverEvent::Done(DoneEvent {
                        stop_reason: StopReason::Cancelled,
                    }),
                    raw: None,
                    at: at(),
                });
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// A relay store whose reads fail
// ---------------------------------------------------------------------------------------------

/// `MemStore`'s relay surface, except that `permission` fails: for ever ([`FailingReads::always`],
/// `Unreachable`), or its first `n` reads ([`FailingReads::times`], `Backend` and `Unreachable`
/// alternating), the transient errors a lease heartbeat also survives (D123).
#[derive(Debug)]
struct FailingReads {
    store: MemStore,
    /// Failures left; `usize::MAX` is "for ever".
    left: AtomicUsize,
}

impl FailingReads {
    fn always(store: MemStore) -> Self {
        Self {
            store,
            left: AtomicUsize::new(usize::MAX),
        }
    }

    fn times(store: MemStore, n: usize) -> Self {
        Self {
            store,
            left: AtomicUsize::new(n),
        }
    }
}

impl htui_core::store::RelayStore for FailingReads {
    async fn open_permission(&self, open: OpenPermission) -> StoreResult<PermissionId> {
        WriteStore::open_permission(&self.store, open).await
    }

    async fn permission(&self, id: PermissionId) -> StoreResult<Option<StepPermission>> {
        let left = self.left.load(Ordering::SeqCst);
        if left == 0 {
            return htui_core::store::RelayStore::permission(&self.store, id).await;
        }
        if left != usize::MAX {
            self.left.store(left - 1, Ordering::SeqCst);
        }
        if left.is_multiple_of(2) {
            Err(StoreError::Backend("deadlock detected".to_owned()))
        } else {
            Err(StoreError::Unreachable(
                "the relay read is switched off".to_owned(),
            ))
        }
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> StoreResult<Option<PermissionChoice>> {
        WriteStore::apply_permission(&self.store, id, owner).await
    }

    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> StoreResult<u64> {
        WriteStore::settle_permissions(&self.store, session, to).await
    }

    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> StoreResult<bool> {
        WriteStore::open_follow_ups(&self.store, run, step, session, owner).await
    }

    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> StoreResult<Option<QueuedFollowUp>> {
        WriteStore::next_follow_up(&self.store, step, session).await
    }

    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> StoreResult<SettleOutcome> {
        WriteStore::settle_follow_up(&self.store, id, owner, to).await
    }

    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> StoreResult<u64> {
        WriteStore::close_follow_ups(&self.store, step, session, reason).await
    }

    async fn close_dropped_follow_ups(
        &self,
        run: RunId,
        owner: Uuid,
        reason: &str,
    ) -> StoreResult<u64> {
        WriteStore::close_dropped_follow_ups(&self.store, run, owner, reason).await
    }
}

/// `MemStore`'s relay surface, except that `settle_permissions` answers `Unreachable`.
#[derive(Debug)]
struct FailingSettles(MemStore);

impl htui_core::store::RelayStore for FailingSettles {
    async fn open_permission(&self, open: OpenPermission) -> StoreResult<PermissionId> {
        WriteStore::open_permission(&self.0, open).await
    }

    async fn permission(&self, id: PermissionId) -> StoreResult<Option<StepPermission>> {
        htui_core::store::RelayStore::permission(&self.0, id).await
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> StoreResult<Option<PermissionChoice>> {
        WriteStore::apply_permission(&self.0, id, owner).await
    }

    async fn settle_permissions(
        &self,
        _session: RelaySessionId,
        _to: PermissionStatus,
    ) -> StoreResult<u64> {
        Err(StoreError::Unreachable(
            "the settle is switched off".to_owned(),
        ))
    }

    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> StoreResult<bool> {
        WriteStore::open_follow_ups(&self.0, run, step, session, owner).await
    }

    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> StoreResult<Option<QueuedFollowUp>> {
        WriteStore::next_follow_up(&self.0, step, session).await
    }

    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> StoreResult<SettleOutcome> {
        WriteStore::settle_follow_up(&self.0, id, owner, to).await
    }

    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> StoreResult<u64> {
        WriteStore::close_follow_ups(&self.0, step, session, reason).await
    }

    async fn close_dropped_follow_ups(
        &self,
        run: RunId,
        owner: Uuid,
        reason: &str,
    ) -> StoreResult<u64> {
        WriteStore::close_dropped_follow_ups(&self.0, run, owner, reason).await
    }
}

/// `MemStore`'s recorder surface, except that every `append_events` answers `Unreachable` while
/// `down` is set. The recorder keeps the refused rows owed (MOD-40 D3), so a later flush lands
/// them.
#[derive(Debug)]
struct FlakyAppends<'a> {
    store: &'a MemStore,
    down: Arc<AtomicBool>,
}

impl htui_core::store::RecorderStore for FlakyAppends<'_> {
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> StoreResult<usize> {
        if self.down.load(Ordering::SeqCst) {
            return Err(StoreError::Unreachable(
                "the log is switched off".to_owned(),
            ));
        }
        htui_core::store::RecorderStore::append_events(self.store, fence, events).await
    }

    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> StoreResult<()> {
        htui_core::store::RecorderStore::set_step_usage(
            self.store,
            fence,
            step,
            usage,
            prompt_digest,
        )
        .await
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: htui_core::model::AgentId,
        box_id: htui_core::model::BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        htui_core::store::RecorderStore::set_agent_box_quota(
            self.store, agent_id, box_id, quota, quota_at,
        )
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// The client and the readers
// ---------------------------------------------------------------------------------------------

/// `drive` within [`LIMIT`] of the paused clock.
async fn within<T>(drive: impl Future<Output = T>) -> T {
    tokio::time::timeout(LIMIT, drive)
        .await
        .expect("drive ended within the limit")
}

/// The client's view: the first row `relay_view` lists for the run's item, polled until one
/// appears; `None` when none appeared within half the [`LIMIT`].
async fn parked_row(store: &MemStore) -> Option<StepPermission> {
    let poll = async {
        loop {
            let view = store
                .relay_view(ids::HTUI_ANA_2)
                .await
                .expect("the relay view reads");
            if let Some(row) = view.permissions.into_iter().next() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(LIMIT / 2, poll).await.ok()
}

/// The client answers `option` as the demo user from the demo box.
async fn answer(store: &MemStore, row: &StepPermission, option: &str) {
    assert_eq!(
        store
            .answer_permission(row.id, option, ids::USER, ids::BOX)
            .await
            .expect("the answer is accepted"),
        AnswerOutcome::Answered,
        "the client's answer wins"
    );
}

async fn log(store: &MemStore, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("the log reads")
        .unwrap_or_default()
}

/// The `permission_answer` payloads of a log, in `seq` order.
fn answers_in(log: &[SessionEvent]) -> Vec<Value> {
    log.iter()
        .filter(|row| row.kind == EventKind::PermissionAnswer)
        .map(|row| row.payload.clone())
        .collect()
}

fn position(log: &[SessionEvent], kind: EventKind) -> Option<usize> {
    log.iter().position(|row| row.kind == kind)
}

fn the_one_row(store: &MemStore) -> StepPermission {
    let rows = store.relay_rows();
    assert_eq!(rows.len(), 1, "exactly one relay row: {rows:?}");
    rows.into_iter().next().expect("one row")
}

// ---------------------------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------------------------

/// D9 stages 1-2: a policy that allows answers at once, writes no relay row and records
/// `by: policy`.
#[tokio::test(start_paused = true)]
async fn a_policy_answer_writes_no_relay_row_and_records_by_policy() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(answered_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy {
        default: PermissionDefault::Allow,
        ..PermissionPolicy::default()
    };
    let relay = relay(&fx, &policy);

    let out = within(drive(
        &mut session,
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the policy's answer lets the turn finish"
    );
    assert!(
        fx.store.relay_rows().is_empty(),
        "a policy answer parks nothing"
    );
    assert_eq!(
        seen.answers(),
        vec![(
            PermissionRequestId::new(REQUEST),
            PermissionAnswer::Selected(ALLOW.to_owned())
        )],
        "the session got the allow option"
    );
    let answers = answers_in(&log(&fx.store, fx.step).await);
    assert_eq!(answers.len(), 1, "one recorded answer: {answers:?}");
    assert_eq!(answers[0]["by"], "policy");
    assert_eq!(answers[0]["option_id"], ALLOW);
    assert_eq!(answers[0]["cancelled"], false);
}

/// MOD-26 D10, D11 (I-1): under the `reviewer` persona a parked `edit` request is answered
/// `reject_once` by policy stage 1 — over a base that would have asked — so nothing is relayed and
/// the answer is recorded `by: policy`, the `a_policy_answer_writes_no_relay_row_and_records_by_policy`
/// shape. The recorded row carries no reason; the reason is the policy's, read back from it.
#[tokio::test(start_paused = true)]
async fn a_persona_denied_kind_is_rejected_by_policy() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut events = vec![call_of("edit the README", ToolKind::Edit), park("Allow")];
    events.extend(finished());
    let (mut session, seen) = watched(
        Some(fake(Script::one_turn(events), fx.step).await),
        Mode::Forward,
    );
    let base = PermissionPolicy::default();
    assert_eq!(base.default, PermissionDefault::Ask, "the base would ask");
    let policy = narrow(&ToolExposure::default(), &base, &reviewer()).1;
    let relay = relay(&fx, &policy);

    let out = within(drive(
        &mut session,
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the persona's answer lets the turn finish"
    );
    assert!(
        fx.store.relay_rows().is_empty(),
        "a persona's reject parks nothing"
    );
    assert_eq!(
        seen.answers(),
        vec![(
            PermissionRequestId::new(REQUEST),
            PermissionAnswer::Selected(REJECT.to_owned())
        )],
        "the session got the reject option"
    );
    let answers = answers_in(&log(&fx.store, fx.step).await);
    assert_eq!(answers.len(), 1, "one recorded answer: {answers:?}");
    assert_eq!(answers[0]["by"], "policy");
    assert_eq!(answers[0]["option_id"], REJECT);
    assert_eq!(answers[0]["cancelled"], false);

    let DriverEvent::ToolCall(edit) = call_event_of("edit the README", ToolKind::Edit) else {
        unreachable!("call_event_of builds a tool call")
    };
    let answer = evaluate(
        &policy,
        Some(&edit),
        &permission_request(REQUEST, "Allow").options,
    )
    .expect("the persona answers");
    assert_eq!(answer.stage, PolicyStage::Rule);
    assert_eq!(answer.reason, "persona reviewer denies edit");
}

/// D3, D4, I-5: a stage-3 request is relayed scrubbed, answered by a client, applied under the
/// lease, sent to the session and echoed `by: user` after its request.
#[tokio::test(start_paused = true)]
async fn a_stage_three_request_is_relayed_scrubbed_and_resumes_on_an_answer() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let script = {
        let mut events = vec![
            call(&format!("run the suite with {SECRET}")),
            park(&format!("Allow {SECRET}")),
        ];
        events.extend(finished());
        Script::one_turn(events)
    };
    let (mut session, seen) = watched(Some(fake(script, fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            // §5.2: the request row is durable before a relay row names it; the recorder's
            // buffer is not what a parked row may point at.
            let stored = log(&fx.store, fx.step).await;
            let durable = (
                stored
                    .iter()
                    .any(|e| e.kind == EventKind::ToolCall && e.payload["tool_call_id"] == CALL),
                stored.iter().any(|e| {
                    e.kind == EventKind::PermissionRequest && e.payload["request_id"] == REQUEST
                }),
            );
            answer(&fx.store, &row, &row.options[0].id).await;
            Some((row, durable))
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the answered turn finishes"
    );
    let (parked, durable) = parked.expect("the client saw the parked request");
    assert_eq!(
        durable,
        (true, true),
        "the call and its request are stored before the relay row is visible"
    );
    let summary = parked
        .summary
        .clone()
        .expect("the call's summary is relayed");
    assert!(
        summary.starts_with("execute: run the suite with "),
        "the summary is `<tool_kind>: <title>`: {summary}"
    );
    assert!(
        !summary.contains(SECRET),
        "the summary is scrubbed (I-5): {summary}"
    );
    assert!(
        parked
            .options
            .iter()
            .all(|option| !option.label.contains(SECRET)),
        "the labels are scrubbed (I-5): {:?}",
        parked.options
    );
    assert_eq!(
        parked
            .options
            .iter()
            .map(|option| (option.id.as_str(), option.kind))
            .collect::<Vec<_>>(),
        vec![
            (ALLOW, RelayOptionKind::AllowOnce),
            (REJECT, RelayOptionKind::RejectOnce)
        ],
        "option ids and kinds are relayed verbatim, in the agent's order"
    );
    assert_eq!(parked.request_id, REQUEST);
    assert_eq!(parked.tool_call_id.as_deref(), Some(CALL));

    let row = the_one_row(&fx.store);
    assert_eq!(
        row.status,
        PermissionStatus::Applied,
        "the executor applied it (D4)"
    );
    assert!(row.resolved_at.is_some());
    assert_eq!(
        seen.answers(),
        vec![(
            PermissionRequestId::new(REQUEST),
            PermissionAnswer::Selected(ALLOW.to_owned())
        )],
        "the live session got the client's option"
    );

    let log = log(&fx.store, fx.step).await;
    let answers = answers_in(&log);
    assert_eq!(answers.len(), 1, "one recorded answer: {answers:?}");
    assert_eq!(answers[0]["by"], "user");
    assert_eq!(answers[0]["option_id"], ALLOW);
    assert!(
        position(&log, EventKind::PermissionRequest) < position(&log, EventKind::PermissionAnswer),
        "the echo follows its request"
    );
}

/// B-14: a summary the scrubber refuses is not relayed, and every label falls back to its kind's
/// text; the request stays answerable, because ids are never scrubbed.
#[tokio::test(start_paused = true)]
async fn a_summary_the_scrubber_refuses_is_not_relayed() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let script = {
        let mut events = vec![call(&format!("push with {RESIDUE}")), park("Allow")];
        events.extend(finished());
        Script::one_turn(events)
    };
    let (mut session, _seen) = watched(Some(fake(script, fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            answer(&fx.store, &row, ALLOW).await;
            Some(row)
        }
    );
    // The recorder refused the `tool_call` row itself and says so at the close.
    let _ = recorder.finish().await;

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the refused summary does not cost the request"
    );
    let parked = parked.expect("the request was relayed");
    assert_eq!(
        parked.summary, None,
        "nothing unscrubbable is persisted (fail-closed)"
    );
    assert_eq!(
        parked
            .options
            .iter()
            .map(|option| (option.id.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![(ALLOW, "allow_once"), (REJECT, "reject_once")],
        "each label is its kind's text"
    );
}

/// I-7: a cancel while parked ends the turn `Cancelled`, records the parked request's
/// `cancelled` answer exactly once, settles the row `cancelled`, and cancels the session with the
/// signal's grace.
#[tokio::test(start_paused = true)]
async fn a_cancel_while_parked_answers_cancelled_once() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();
    let grace = Duration::from_secs(3);

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            signal.send_replace(Signal::Cancel { grace });
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a graceful cancel");
    assert!(
        parked.is_some(),
        "the request was relayed before the cancel"
    );
    assert_eq!(
        seen.cancels(),
        vec![grace],
        "one cancel, with the signal's grace"
    );
    assert!(seen.answers().is_empty(), "nothing was selected");
    assert_eq!(
        the_one_row(&fx.store).status,
        PermissionStatus::Cancelled,
        "the row is settled `cancelled` (I-7)"
    );

    let log = log(&fx.store, fx.step).await;
    let answers = answers_in(&log);
    assert_eq!(answers.len(), 1, "I-7's row exactly once: {answers:?}");
    assert_eq!(answers[0]["request_id"], REQUEST);
    assert_eq!(answers[0]["option_id"], Value::Null);
    assert_eq!(answers[0]["by"], "policy");
    assert_eq!(answers[0]["cancelled"], true);
    let last = log.last().expect("a log");
    assert_eq!(
        last.kind,
        EventKind::Done,
        "the log ends at the cancel's `done`"
    );
    assert_eq!(last.payload["stop_reason"], "cancelled");
}

/// R-8 (probe A3b): a cancel sent before `drive` is called is still read, and nothing is parked.
#[tokio::test(start_paused = true)]
async fn a_cancel_sent_before_the_park_still_ends_as_a_cancel() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();
    let grace = Duration::from_secs(3);
    signal.send_replace(Signal::Cancel { grace });

    let out = within(drive(
        &mut session,
        &mut recorder,
        Some(&relay),
        &mut control,
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "the early cancel is read");
    assert!(fx.store.relay_rows().is_empty(), "no request was parked");
    assert_eq!(seen.cancels(), vec![grace]);
    assert!(answers_in(&log(&fx.store, fx.step).await).is_empty());
}

/// B-2: a cancel that arrives while `drive` awaits the next event interrupts the pull and takes
/// the same graceful path.
#[tokio::test(start_paused = true)]
async fn a_cancel_mid_turn_cancels_the_session_gracefully() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(None, Mode::Stall);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();
    let grace = Duration::from_secs(3);

    let (out, ()) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            signal.send_replace(Signal::Cancel { grace });
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled));
    assert_eq!(
        seen.cancels(),
        vec![grace],
        "session.cancel once, with the grace"
    );
    let log = log(&fx.store, fx.step).await;
    assert_eq!(
        log.last().map(|row| row.payload["stop_reason"].clone()),
        Some(json!("cancelled")),
        "the drain recorded the cancel's `done`"
    );
}

/// Drives `session` with a relay and sends `Signal::Cancel { grace }` 50 ms in; answers `drive`'s
/// result and how long the paused clock ran from the cancel to `drive`'s return.
async fn cancel_mid_turn(
    fx: &Leased,
    session: &mut Watched,
    grace: Duration,
) -> (Result<DoneEvent, DriverError>, Duration) {
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(fx, &scrubber);
    let policy = PermissionPolicy::default();
    let relay = relay(fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, sent) = tokio::join!(
        within(drive(session, &mut recorder, Some(&relay), &mut control)),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            signal.send_replace(Signal::Cancel { grace });
            tokio::time::Instant::now()
        }
    );
    let took = sent.elapsed();
    recorder.finish().await.expect("the recorder closes");
    (out, took)
}

/// B-11: a cancelled session that never ends its turn holds `drive` for `grace` plus one second,
/// no longer, and the answer is still `Cancelled`.
#[tokio::test(start_paused = true)]
async fn a_cancelled_session_that_never_ends_its_turn_is_bounded() {
    let fx = leased().await;
    let (mut session, seen) = watched(None, Mode::Hang);
    let grace = Duration::from_secs(3);

    let (out, took) = cancel_mid_turn(&fx, &mut session, grace).await;

    assert_eq!(out, Err(DriverError::Cancelled), "still a graceful cancel");
    assert_eq!(
        seen.cancels(),
        vec![grace],
        "session.cancel once, with the grace"
    );
    assert_eq!(
        took,
        grace + Duration::from_secs(1),
        "the drain waits grace + 1 s for a turn that never ends (B-11)"
    );
}

/// The post-cancel drain stops at the cancel's `done` (`drive`'s doc; a refinement of D6's "drain
/// to the stream's end" that awaits its own §0a entry). A stream that outlives the turn (ACP) does
/// not hold `drive` for B-11's whole bound.
#[tokio::test(start_paused = true)]
async fn the_post_cancel_drain_stops_at_the_turns_done() {
    let fx = leased().await;
    let (mut session, seen) = watched(None, Mode::Linger);
    let grace = Duration::from_secs(3);

    let (out, took) = cancel_mid_turn(&fx, &mut session, grace).await;

    assert_eq!(out, Err(DriverError::Cancelled));
    assert_eq!(seen.cancels(), vec![grace]);
    assert_eq!(took, Duration::ZERO, "no wait past the turn's `done`");
    let log = log(&fx.store, fx.step).await;
    assert_eq!(
        log.last().map(|row| row.payload["stop_reason"].clone()),
        Some(json!("cancelled")),
        "the drain recorded the cancel's `done`"
    );
}

/// A session that, once cancelled, emits `tool_call`/`tool_result` pairs at once and for ever:
/// every `record` of the drain flushes the row before it.
#[derive(Debug, Default)]
struct Chatter {
    cancelled: bool,
    emitted: usize,
}

impl AgentSession for Chatter {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        None
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            if !self.cancelled {
                std::future::pending::<()>().await;
            }
            let id = format!("late-{}", self.emitted / 2);
            let event = if self.emitted.is_multiple_of(2) {
                DriverEvent::ToolCall(ToolCallEvent {
                    tool_call_id: id,
                    title: "late".to_owned(),
                    tool_kind: ToolKind::Execute,
                    input: json!({}),
                    locations: Vec::new(),
                })
            } else {
                DriverEvent::ToolResult(ToolResultEvent {
                    tool_call_id: id,
                    status: ToolResultStatus::Failed,
                    output: None,
                    locations: Vec::new(),
                    terminal_reason: None,
                })
            };
            self.emitted += 1;
            Ok(Some(DriverEnvelope {
                event,
                raw: None,
                at: at(),
            }))
        })
    }

    fn send_follow_up<'a>(&'a mut self, _text: String) -> DriverFuture<'a, ()> {
        Box::pin(async { Err(DriverError::Closed) })
    }

    fn answer_permission<'a>(
        &'a mut self,
        _request_id: PermissionRequestId,
        _answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async { Err(DriverError::Closed) })
    }

    fn cancel<'a>(&'a mut self, _grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            self.cancelled = true;
            Ok(())
        })
    }
}

/// `MemStore`'s recorder surface, except that every `append_events` takes [`SLOW_APPEND`] first.
#[derive(Debug)]
struct SlowAppends<'a>(&'a MemStore);

/// Longer than B-11's slack is short: a drain that times out mid-flush lands inside one.
const SLOW_APPEND: Duration = Duration::from_millis(700);

impl htui_core::store::RecorderStore for SlowAppends<'_> {
    async fn append_events(&self, fence: StepFence, events: &[SessionEvent]) -> StoreResult<usize> {
        tokio::time::sleep(SLOW_APPEND).await;
        htui_core::store::RecorderStore::append_events(self.0, fence, events).await
    }

    async fn set_step_usage(
        &self,
        fence: StepFence,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> StoreResult<()> {
        htui_core::store::RecorderStore::set_step_usage(self.0, fence, step, usage, prompt_digest)
            .await
    }

    async fn set_agent_box_quota(
        &self,
        agent_id: htui_core::model::AgentId,
        box_id: htui_core::model::BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        htui_core::store::RecorderStore::set_agent_box_quota(
            self.0, agent_id, box_id, quota, quota_at,
        )
        .await
    }
}

/// B-11 bounds the drain's pulls, never a recording: a slow store whose flush is still running
/// when the bound passes loses no row the recorder numbered, so the log holds every `seq`.
#[tokio::test(start_paused = true)]
async fn the_drain_bound_never_drops_a_row_mid_flush() {
    let fx = leased().await;
    let scrubber = scrubber();
    let slow = SlowAppends(&fx.store);
    let mut recorder = Recorder::new(&slow, &scrubber, fx.step, false, None)
        .with_fence(StepFence::Lease(fx.owner));
    let mut session = Chatter::default();
    let (signal, mut control) = control_channel();

    let (out, ()) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            None::<&Relay<'_, NoRelay>>,
            &mut control
        )),
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            signal.send_replace(Signal::Cancel {
                grace: Duration::ZERO,
            });
        }
    );
    let summary = recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "still a graceful cancel");
    let log = log(&fx.store, fx.step).await;
    assert!(log.len() >= 2, "the drain recorded the late rows: {log:?}");
    let seqs: Vec<i32> = log.iter().map(|row| row.seq).collect();
    let numbered: Vec<i32> = (0..summary.seq).collect();
    assert_eq!(seqs, numbered, "every seq the recorder numbered is stored");
    assert_eq!(summary.rows, log.len(), "and the recorder counted each one");
}

/// B-16, D5: a newer session of the step supersedes the parked row (`stale`), and `drive` reads
/// that back as a cancel with `Relay.grace`.
#[tokio::test(start_paused = true)]
async fn a_stale_row_read_back_ends_the_turn_as_a_cancel() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            fx.store
                .open_permission(OpenPermission {
                    id: PermissionId::new(),
                    run_id: fx.run,
                    run_step_id: fx.step,
                    session: RelaySessionId::new(),
                    request_id: "perm-newer".to_owned(),
                    tool_call_id: None,
                    summary: None,
                    options: Vec::new(),
                    owner: fx.owner,
                })
                .await
                .expect("a newer session of the step opens a row");
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a stale row is a cancel");
    let parked = parked.expect("the request was relayed");
    let row = fx
        .store
        .permission(parked.id)
        .await
        .expect("the row reads")
        .expect("the row exists");
    assert_eq!(
        row.status,
        PermissionStatus::Stale,
        "the newer session staled it (D5)"
    );
    assert_eq!(
        seen.cancels(),
        vec![RELAY_GRACE],
        "the cancel takes Relay.grace (B-16)"
    );
    let answers = answers_in(&log(&fx.store, fx.step).await);
    assert_eq!(answers.len(), 1, "I-7's row once: {answers:?}");
    assert_eq!(answers[0]["cancelled"], true);
}

/// D4: an answer that can no longer be applied under the lease is `Fenced { step }`, and the
/// session is never answered.
#[tokio::test(start_paused = true)]
async fn an_answer_whose_lease_is_gone_is_fenced() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(answered_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            answer(&fx.store, &row, ALLOW).await;
            assert!(
                fx.store
                    .release_lease(fx.run, fx.owner)
                    .await
                    .expect("the release is answered"),
                "the executor's lease is released before its next poll"
            );
            Some(row)
        }
    );

    assert_eq!(
        out,
        Err(DriverError::Store(StoreError::Fenced { step: fx.step })),
        "the apply is fenced (D4)"
    );
    assert!(parked.is_some(), "the request was relayed");
    assert!(seen.answers().is_empty(), "the session got no answer");
    assert_eq!(
        the_one_row(&fx.store).status,
        PermissionStatus::Answered,
        "a fenced executor writes nothing more: the row stays `answered`"
    );
}

/// B-15: a session that fails after its row was opened leaves no row of the session `pending` or
/// `answered`.
#[tokio::test(start_paused = true)]
async fn a_session_that_fails_while_parked_leaves_no_pending_row() {
    // The session refuses the applied answer.
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, _seen) = watched(
        Some(fake(answered_script(), fx.step).await),
        Mode::FailAnswers,
    );
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            answer(&fx.store, &row, ALLOW).await;
            Some(row)
        }
    );

    assert_eq!(
        out,
        Err(DriverError::Closed),
        "the session's failure is the answer"
    );
    assert!(parked.is_some(), "the request was relayed");
    let rows = fx.store.relay_rows();
    assert_eq!(
        rows.iter().map(|row| row.status).collect::<Vec<_>>(),
        vec![PermissionStatus::Applied],
        "the CAS ran, and nothing is left pending or answered"
    );

    // The relay store's read fails while parked.
    let fx = leased().await;
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, _seen) = watched(Some(fake(answered_script(), fx.step).await), Mode::Forward);
    let failing = FailingReads::always(fx.store.clone());
    let relay = Relay {
        store: &failing,
        owner: fx.owner,
        run: fx.run,
        step: fx.step,
        session: RelaySessionId::new(),
        policy: &policy,
        poll: POLL,
        grace: RELAY_GRACE,
        now: &at,
        follow_ups: false,
    };

    let out = within(drive(
        &mut session,
        &mut recorder,
        Some(&relay),
        &mut control,
    ))
    .await;

    assert!(
        matches!(out, Err(DriverError::Store(StoreError::Unreachable(_)))),
        "the failed read is the answer, got {out:?}"
    );
    assert_eq!(
        fx.store
            .relay_rows()
            .iter()
            .map(|row| row.status)
            .collect::<Vec<_>>(),
        vec![PermissionStatus::Stale],
        "the leftover row is marked stale (B-15)"
    );
}

/// I-8: `drive` with no relay and a control nothing signals is `pump`: the same answer and the
/// same rows over a script that parks.
#[tokio::test(start_paused = true)]
async fn drive_without_a_relay_is_pump() {
    let (run, step) = (RunId::new(), StepId::new());
    let scrubber = scrubber();

    let pumped = leased_at(run, step).await;
    let mut recorder = fenced_recorder(&pumped, &scrubber);
    let mut session = fake(answered_script(), step).await;
    let by_pump = within(pump(session.as_mut(), &mut recorder)).await;
    let _ = recorder.finish().await;

    let driven = leased_at(run, step).await;
    let mut recorder = fenced_recorder(&driven, &scrubber);
    let mut session = fake(answered_script(), step).await;
    let by_drive = within(drive(
        session.as_mut(),
        &mut recorder,
        None::<&Relay<'_, NoRelay>>,
        &mut Control::never(),
    ))
    .await;
    let _ = recorder.finish().await;

    assert!(
        matches!(&by_pump, Err(DriverError::Transport(message)) if message.contains("is parked")),
        "pump pulls past the park and the transport refuses: {by_pump:?}"
    );
    assert_eq!(by_drive, by_pump, "the same answer");
    assert_eq!(
        log(&driven.store, step).await,
        log(&pumped.store, step).await,
        "the same rows"
    );
    assert!(driven.store.relay_rows().is_empty(), "and no relay row");
}

// ---------------------------------------------------------------------------------------------
// ACP's shape: requests queued behind the parked one
// ---------------------------------------------------------------------------------------------

/// The request queued behind [`REQUEST`].
const QUEUED: &str = "perm-2";

/// ACP's shape (`acp/mod.rs`): the transport parks every request it emits, before `drive` pulls
/// it, so its cancel answers a request still queued in the channel `cancelled` on the wire with
/// the rest; the handle's `cancel` keeps the queued events and the turn then ends with `done
/// { cancelled }`. `next_event` hands `queue` out in order and pends once it is empty.
#[derive(Debug)]
struct Queued {
    queue: VecDeque<DriverEvent>,
    cancelled: bool,
}

impl Queued {
    fn new(events: impl IntoIterator<Item = DriverEvent>) -> Self {
        Self {
            queue: events.into_iter().collect(),
            cancelled: false,
        }
    }
}

impl AgentSession for Queued {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        None
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            match self.queue.pop_front() {
                Some(event) => Ok(Some(DriverEnvelope {
                    event,
                    raw: None,
                    at: at(),
                })),
                None => std::future::pending().await,
            }
        })
    }

    fn send_follow_up<'a>(&'a mut self, _text: String) -> DriverFuture<'a, ()> {
        Box::pin(async { Err(DriverError::Closed) })
    }

    fn answer_permission<'a>(
        &'a mut self,
        _request_id: PermissionRequestId,
        _answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async { Err(DriverError::Closed) })
    }

    fn cancel<'a>(&'a mut self, _grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if !self.cancelled {
                self.cancelled = true;
                self.queue.push_back(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::Cancelled,
                }));
            }
            Ok(())
        })
    }
}

/// I-7 over a whole log: the requests and answers are exactly `requests`, each answered once,
/// right after itself, `{option_id: null, by: "policy", cancelled: true}`; the log ends at the
/// cancel's `done`.
fn assert_each_answered_cancelled(log: &[SessionEvent], requests: &[&str]) {
    let rows: Vec<(EventKind, String)> = log
        .iter()
        .filter(|row| {
            matches!(
                row.kind,
                EventKind::PermissionRequest | EventKind::PermissionAnswer
            )
        })
        .map(|row| {
            let id = row.payload["request_id"].as_str().unwrap_or_default();
            (row.kind, id.to_owned())
        })
        .collect();
    let expected: Vec<(EventKind, String)> = requests
        .iter()
        .flat_map(|id| {
            [
                (EventKind::PermissionRequest, (*id).to_owned()),
                (EventKind::PermissionAnswer, (*id).to_owned()),
            ]
        })
        .collect();
    assert_eq!(rows, expected, "each request answered once, after itself");
    for answer in answers_in(log) {
        assert_eq!(answer["option_id"], Value::Null, "{answer}");
        assert_eq!(answer["by"], "policy", "{answer}");
        assert_eq!(answer["cancelled"], true, "{answer}");
    }
    assert_eq!(
        log.last().map(|row| row.payload["stop_reason"].clone()),
        Some(json!("cancelled")),
        "the log ends at the cancel's `done`"
    );
}

/// I-7: a request queued behind the parked one when the cancel lands was answered `cancelled` by
/// the transport with the parked one, so the drain records its answer too.
#[tokio::test(start_paused = true)]
async fn a_request_queued_behind_the_parked_one_is_answered_cancelled_too() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = Queued::new([
        call_event("run the suite"),
        DriverEvent::PermissionRequest(permission_request(REQUEST, "Allow")),
        DriverEvent::PermissionRequest(permission_request(QUEUED, "Allow")),
    ]);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            signal.send_replace(Signal::Cancel {
                grace: Duration::from_secs(3),
            });
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a graceful cancel");
    assert_eq!(
        parked.expect("a request was parked").request_id,
        REQUEST,
        "the first request is the parked one"
    );
    assert_each_answered_cancelled(&log(&fx.store, fx.step).await, &[REQUEST, QUEUED]);
}

/// I-7: a cancel that lands while a request is queued but not yet pulled parks nothing, and the
/// drain still records the request's `cancelled` answer. Without a relay (I-8) it records none.
#[tokio::test(start_paused = true)]
async fn a_request_queued_when_the_cancel_lands_is_answered_cancelled() {
    let script = || {
        Queued::new([
            call_event("run the suite"),
            DriverEvent::PermissionRequest(permission_request(REQUEST, "Allow")),
        ])
    };
    let cancel = Signal::Cancel {
        grace: Duration::from_secs(3),
    };
    let scrubber = scrubber();
    let policy = PermissionPolicy::default();

    let fx = leased().await;
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();
    signal.send_replace(cancel);
    let out = within(drive(
        &mut script(),
        &mut recorder,
        Some(&relay),
        &mut control,
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a graceful cancel");
    assert!(fx.store.relay_rows().is_empty(), "nothing was parked");
    assert_each_answered_cancelled(&log(&fx.store, fx.step).await, &[REQUEST]);

    let fx = leased().await;
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (signal, mut control) = control_channel();
    signal.send_replace(cancel);
    let out = within(drive(
        &mut script(),
        &mut recorder,
        None::<&Relay<'_, NoRelay>>,
        &mut control,
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a graceful cancel");
    let log = log(&fx.store, fx.step).await;
    assert!(
        position(&log, EventKind::PermissionRequest).is_some(),
        "the request is recorded: {log:?}"
    );
    assert!(answers_in(&log).is_empty(), "and no answer without a relay");
}

// ---------------------------------------------------------------------------------------------
// Store blips
// ---------------------------------------------------------------------------------------------

/// I-6: a store blip inside the cancel sequence does not turn the graceful cancel into a failure.
/// The settle and the recordings are best-effort there: the rows a refused flush numbered stay
/// owed and land at the recorder's next flush (MOD-40 D3), and a `pending` row left behind is
/// unanswerable once the lease is gone (D3, D5). Before the fix both ended `Err(Store(..))`.
#[tokio::test(start_paused = true)]
async fn a_store_blip_during_a_cancel_still_ends_it_as_a_cancel() {
    let scrubber = scrubber();
    let policy = PermissionPolicy::default();
    let grace = Duration::from_secs(3);

    // The settle fails.
    let fx = leased().await;
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, _seen) =
        watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let failing = FailingSettles(fx.store.clone());
    let relay = relay_over(&failing, &fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            signal.send_replace(Signal::Cancel { grace });
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Err(DriverError::Cancelled),
        "the failed settle is a warn"
    );
    assert!(parked.is_some(), "the request was relayed");
    assert_eq!(
        the_one_row(&fx.store).status,
        PermissionStatus::Pending,
        "the settle did not land"
    );
    assert_each_answered_cancelled(&log(&fx.store, fx.step).await, &[REQUEST]);

    // The log's appends fail from the cancel on.
    let fx = leased().await;
    let flaky = FlakyAppends {
        store: &fx.store,
        down: Arc::new(AtomicBool::new(false)),
    };
    let mut recorder = Recorder::new(&flaky, &scrubber, fx.step, false, None)
        .with_fence(StepFence::Lease(fx.owner));
    let (mut session, _seen) =
        watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let relay = relay_over(&fx.store, &fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            flaky.down.store(true, Ordering::SeqCst);
            signal.send_replace(Signal::Cancel { grace });
            Some(row)
        }
    );
    flaky.down.store(false, Ordering::SeqCst);
    recorder
        .finish()
        .await
        .expect("the owed rows land at the close");

    assert_eq!(
        out,
        Err(DriverError::Cancelled),
        "the failed appends are a warn"
    );
    assert!(parked.is_some(), "the request was relayed");
    assert_eq!(
        the_one_row(&fx.store).status,
        PermissionStatus::Cancelled,
        "the settle landed"
    );
    // The answer was numbered before its flush was refused, so it was owed and landed at the
    // close; an event the drain pulled while the log was down was refused before it was
    // numbered, and is lost with its `warn`, as it was with the error.
    let answers = answers_in(&log(&fx.store, fx.step).await);
    assert_eq!(answers.len(), 1, "I-7's row once: {answers:?}");
    assert_eq!(answers[0]["request_id"], REQUEST);
    assert_eq!(answers[0]["cancelled"], true);
}

/// A fenced write inside the cancel sequence is still the answer: the lease is another
/// process's, and a fenced writer writes nothing more (MOD-40 D1, A-2).
#[tokio::test(start_paused = true)]
async fn a_cancel_whose_lease_is_gone_is_fenced() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, _seen) =
        watched(Some(fake(cancelled_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            assert!(
                fx.store
                    .release_lease(fx.run, fx.owner)
                    .await
                    .expect("the release is answered"),
                "the executor's lease is released before the cancel"
            );
            signal.send_replace(Signal::Cancel {
                grace: Duration::from_secs(3),
            });
            Some(row)
        }
    );

    assert_eq!(
        out,
        Err(DriverError::Store(StoreError::Fenced { step: fx.step })),
        "the fence is the answer, not the cancel"
    );
    assert!(parked.is_some(), "the request was relayed");
}

/// D8: a parked request's poll survives a few transient read failures (`Unreachable`, `Backend`),
/// as the lease heartbeat does (D123), and resumes on the client's answer.
#[tokio::test(start_paused = true)]
async fn a_parked_poll_survives_a_few_failed_reads() {
    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (mut session, seen) = watched(Some(fake(answered_script(), fx.step).await), Mode::Forward);
    let policy = PermissionPolicy::default();
    let failing = FailingReads::times(fx.store.clone(), 4);
    let relay = relay_over(&failing, &fx, &policy);
    let mut control = Control::never();

    let (out, parked) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            answer(&fx.store, &row, ALLOW).await;
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the blips cost nothing"
    );
    assert!(parked.is_some(), "the request was relayed");
    assert_eq!(
        failing.left.load(Ordering::SeqCst),
        0,
        "every blip was read"
    );
    assert_eq!(the_one_row(&fx.store).status, PermissionStatus::Applied);
    assert_eq!(
        seen.answers(),
        vec![(
            PermissionRequestId::new(REQUEST),
            PermissionAnswer::Selected(ALLOW.to_owned())
        )],
        "the live session got the client's option"
    );
}

// ---------------------------------------------------------------------------------------------
// MOD-11 D18: a CLI session's prompt, relayed like any other request
// ---------------------------------------------------------------------------------------------

/// The gated call's tool-use id, as the CLI reports it and the prompt tool forwards it.
#[cfg(unix)]
const PROMPTED: &str = "toolu_relay_1";

/// A `cli_driver.rs`-style child double: `system/init`, the gated call, a mark that it is now
/// waiting on its prompt tool, then the turn's `result` once the case creates `$HTUI_GO_FILE`.
#[cfg(unix)]
const PROMPTING_CLI: &str = r#"#!/bin/sh
printf '%s\n' '{"type":"system","subtype":"init","session_id":"cli-picked","claude_code_version":"9.9.9","model":"scripted-model"}'
IFS= read -r line
printf '%s\n' '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_relay_1","name":"Bash","input":{"command":"cargo test"}}]}}'
: > "$HTUI_ASKING_FILE"
while [ ! -f "$HTUI_GO_FILE" ]; do sleep 0.05; done
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","total_cost_usd":0.000002,"modelUsage":{"m":{"inputTokens":1,"outputTokens":2,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}'
exit 0
"#;

/// Polls for `path` on the real clock, failing after half the [`LIMIT`].
#[cfg(unix)]
async fn appears(path: &std::path::Path) {
    let wait = async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(LIMIT / 2, wait)
        .await
        .expect("the child double got that far");
}

/// A CLI session over [`PROMPTING_CLI`] written into `dir`, started with `port`.
#[cfg(unix)]
async fn prompting_cli(
    dir: &std::path::Path,
    step: StepId,
    port: htui_agent::prompt_bridge::PromptPort,
) -> Box<dyn AgentSession> {
    use std::os::unix::fs::PermissionsExt;

    let command = dir.join("prompting-cli");
    std::fs::write(&command, PROMPTING_CLI).expect("write the child double");
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let row = htui_core::model::Agent {
        id: htui_core::model::AgentId::new(),
        name: "scripted-cli".to_owned(),
        transport: htui_core::model::Transport::Cli,
        launch: json!({
            "command": command.to_string_lossy(),
            "args": [],
            "env": {
                "HTUI_ASKING_FILE": dir.join("asking").to_string_lossy(),
                "HTUI_GO_FILE": dir.join("go").to_string_lossy(),
            },
            "discovery": { "handshake": false, "tools": {} },
        }),
        models: vec!["row-model".to_owned()],
        default_model: None,
        billing: htui_core::model::Billing::Subscription,
        enabled: true,
        settings: json!({
            "cli": { "stream": htui_agent::cli::STREAM, "permission_mode": "", "extra_args": [] },
            "usage": { "scope": "model_usage" },
        }),
        created_at: at(),
        updated_at: at(),
    };
    let mut factory = htui_agent::registry::DriverFactory::new();
    factory.register(
        htui_agent::cli::ADAPTER_ID,
        Box::new(htui_agent::cli::ClaudeStreamAdapter)
            as Box<dyn htui_agent::registry::TransportBuilder>,
    );
    let driver = factory
        .driver_for(&row, None)
        .expect("the cli adapter builds the row");
    let mut spec = spec(step);
    spec.cwd = dir.to_path_buf();
    spec.prompt = Some(port);
    driver
        .start(spec, "summarise the backlog".to_owned())
        .await
        .expect("the child double sends its `system/init`")
}

/// MOD-11 D18 end to end over the MOD-42 relay: the CLI's prompt tool call parks through the
/// port, `drive` relays it as a stage-3 row, a client answers it from the store, and the prompt
/// tool gets `allow` — the turn then finishes as any answered turn does. Real clock: the session
/// runs a child process.
#[cfg(unix)]
#[tokio::test]
async fn the_relay_answers_a_cli_prompt_from_the_store() {
    use htui_agent::prompt_bridge::{PromptCall, PromptVerdict, bridge};

    let fx = leased().await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let tmp = tempfile::tempdir().expect("temp box");
    let (port, ask) = bridge();
    let mut session = prompting_cli(tmp.path(), fx.step, port).await;
    let policy = PermissionPolicy::default();
    let relay = relay(&fx, &policy);
    let mut control = Control::never();

    let (out, (verdict, parked)) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            // The CLI asks once it has announced the call, as it does before running a tool.
            appears(&tmp.path().join("asking")).await;
            let call = PromptCall {
                tool_name: "Bash".to_owned(),
                input: json!({ "command": "cargo test" }),
                tool_use_id: Some(PROMPTED.to_owned()),
            };
            let client = async {
                let row = parked_row(&fx.store).await?;
                answer(&fx.store, &row, &row.options[0].id).await;
                Some(row)
            };
            let (verdict, parked) = tokio::join!(ask.ask(call), client);
            std::fs::write(tmp.path().join("go"), b"").expect("go");
            (verdict, parked)
        }
    );
    recorder.finish().await.expect("the recorder closes");
    session
        .cancel(Duration::ZERO)
        .await
        .expect("the ended session cancels");

    assert_eq!(
        verdict,
        Ok(PromptVerdict::Allow),
        "the prompt tool got the client's allow"
    );
    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the answered turn finishes"
    );
    let parked = parked.expect("the client saw the parked request");
    assert_eq!(parked.request_id, PROMPTED, "the CLI's tool-use id");
    assert_eq!(parked.tool_call_id.as_deref(), Some(PROMPTED));
    assert_eq!(
        parked
            .options
            .iter()
            .map(|option| (option.id.as_str(), option.kind))
            .collect::<Vec<_>>(),
        vec![
            ("allow", RelayOptionKind::AllowOnce),
            ("reject", RelayOptionKind::RejectOnce)
        ],
        "the bridge's two options, relayed verbatim"
    );
    assert_eq!(the_one_row(&fx.store).status, PermissionStatus::Applied);

    let log = log(&fx.store, fx.step).await;
    let answers = answers_in(&log);
    assert_eq!(answers.len(), 1, "one recorded answer: {answers:?}");
    assert_eq!(answers[0]["by"], "user");
    assert_eq!(answers[0]["option_id"], "allow");
    assert!(
        position(&log, EventKind::PermissionRequest) < position(&log, EventKind::PermissionAnswer),
        "the echo follows its request"
    );
}

// ---------------------------------------------------------------------------------------------
// MOD-70 T2: follow-ups for engine steps (plan D6, D7; blueprint B-9 to B-12, B-15)
// ---------------------------------------------------------------------------------------------

/// The text every happy path queues: the executor masks [`SECRET`] when it records it (R-HIS-1).
fn follow_up_text() -> String {
    format!("also run {SECRET} checks")
}

/// The request a follow-up turn parks on, so a client can act while that turn is live (B-9).
const SECOND: &str = "perm-again";

/// Moves the fixture's step `pending → running`, which [`leased_at`] leaves `pending`: only a
/// running step takes a follow-up (D3).
async fn running(fx: &Leased) {
    assert!(
        fx.store
            .transition_step(fx.step, StepStatus::Pending, StepStatus::Running, at())
            .await
            .expect("the step moves"),
        "the step is running"
    );
}

/// [`relay`] with a follow-up window (D6).
fn relay_with_follow_ups<'a>(fx: &'a Leased, policy: &'a PermissionPolicy) -> Relay<'a, MemStore> {
    Relay {
        follow_ups: true,
        ..relay(fx, policy)
    }
}

/// The client queues `text` for `step` as the demo user from the demo box (D3).
async fn enqueue(store: &MemStore, step: StepId, text: &str) -> FollowUpRequest {
    store
        .request_follow_up(NewFollowUp {
            id: RunCommandId::new(),
            run_step_id: step,
            text: FollowUpText::new(text.to_owned()).expect("the client accepts the text"),
            issued_by: ids::USER,
            issued_box: ids::BOX,
        })
        .await
        .expect("the enqueue is answered")
}

/// [`enqueue`], which must be queued; the row's id.
async fn queued(store: &MemStore, step: StepId, text: &str) -> RunCommandId {
    match enqueue(store, step, text).await {
        FollowUpRequest::Queued(id) => id,
        refused @ FollowUpRequest::Refused(_) => {
            panic!("the follow-up was not queued: {refused:?}")
        }
    }
}

/// [`answered_script`]'s turn: a call parked on [`REQUEST`], finished once answered.
fn parked_turn() -> Vec<ScriptEvent> {
    let mut events = vec![call(&format!("run the suite with {SECRET}")), park("Allow")];
    events.extend(finished());
    events
}

/// A turn that parks on `request_id` and finishes once answered.
fn parked_again(request_id: &str) -> Vec<ScriptEvent> {
    let mut events = vec![
        call("run the suite again"),
        ScriptEvent::ParkPermission(permission_request(request_id, "Allow")),
    ];
    events.extend(finished());
    events
}

/// A turn that ends at once.
fn ends() -> Vec<ScriptEvent> {
    vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
        stop_reason: StopReason::EndTurn,
    }))]
}

/// Turn 0 parks (the client's rendezvous, B-9); `turn1` is what a follow-up would start.
fn two_turns(turn1: Vec<ScriptEvent>) -> Script {
    Script::turns(vec![parked_turn(), turn1])
}

/// One turn that asks nothing: a call, its result and the `done`.
fn plain_script() -> Script {
    let mut events = vec![call("run the suite")];
    events.extend(finished());
    Script::one_turn(events)
}

/// The `follow_up` rows of a log as `(turn, payload)`, in `seq` order.
fn follow_ups_in(log: &[SessionEvent]) -> Vec<(i32, Value)> {
    log.iter()
        .filter(|row| row.kind == EventKind::FollowUp)
        .map(|row| (row.turn, row.payload.clone()))
        .collect()
}

/// How many `done` rows a log holds: one per turn.
fn dones_in(log: &[SessionEvent]) -> usize {
    log.iter().filter(|row| row.kind == EventKind::Done).count()
}

/// Follow-up `id`'s status, resolution and whether its text is still stored (B-1).
fn the_follow_up(store: &MemStore, id: RunCommandId) -> (RunCommandStatus, Option<String>, bool) {
    store
        .follow_up_rows()
        .into_iter()
        .find(|(row, _)| row.id == id)
        .map(|(row, text)| (row.status, row.resolution, text))
        .expect("the follow-up row exists")
}

/// Every follow-up row's status, resolution and stored-text flag, in id order.
fn follow_ups(store: &MemStore) -> Vec<(RunCommandStatus, Option<String>, bool)> {
    store
        .follow_up_rows()
        .into_iter()
        .map(|(row, text)| (row.status, row.resolution, text))
        .collect()
}

/// The client's view of the pending request `request_id`, polled as [`parked_row`] polls.
async fn parked_row_for(store: &MemStore, request_id: &str) -> Option<StepPermission> {
    let poll = async {
        loop {
            let view = store
                .relay_view(ids::HTUI_ANA_2)
                .await
                .expect("the relay view reads");
            if let Some(row) = view
                .permissions
                .into_iter()
                .find(|row| row.request_id == request_id)
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(LIMIT / 2, poll).await.ok()
}

/// Takes the run's lease away from the executor: a zero-TTL refresh, then a stranger's take.
async fn steal_lease(store: &MemStore, run: RunId, owner: Uuid) {
    assert!(
        store
            .refresh_lease(run, owner, TimeDelta::zero())
            .await
            .expect("the refresh is answered"),
        "the executor's lease expires now"
    );
    assert!(
        store
            .take_lease(run, ids::BOX, Uuid::new_v4(), TimeDelta::minutes(5))
            .await
            .expect("the take is answered"),
        "a stranger takes the lapsed lease"
    );
}

/// A session that signals a cancel on `sender` as it hands out each `done`: the cancel lands
/// between the turn's end and `drive`'s look for a follow-up (D6 step 3).
#[derive(Debug)]
struct CancelAtDone {
    inner: Box<dyn AgentSession>,
    sender: watch::Sender<Signal>,
    grace: Duration,
}

impl AgentSession for CancelAtDone {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.inner.session_ref()
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            let next = self.inner.next_event().await;
            if let Ok(Some(envelope)) = &next
                && matches!(envelope.event, DriverEvent::Done(_))
            {
                self.sender
                    .send_replace(Signal::Cancel { grace: self.grace });
            }
            next
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        self.inner.send_follow_up(text)
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        self.inner.answer_permission(request_id, answer)
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        self.inner.cancel(grace)
    }
}

/// What [`FailingSend`]'s `send_follow_up` answers.
const SEND_FAILED: &str = "the follow-up's send is switched off";

/// A session whose `send_follow_up` fails ([`SEND_FAILED`]) and which forwards everything else:
/// a follow-up claimed and recorded but never delivered (review M-4).
#[derive(Debug)]
struct FailingSend {
    inner: Box<dyn AgentSession>,
}

impl AgentSession for FailingSend {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.inner.session_ref()
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        self.inner.next_event()
    }

    fn send_follow_up<'a>(&'a mut self, _text: String) -> DriverFuture<'a, ()> {
        Box::pin(async { Err(DriverError::Transport(SEND_FAILED.to_owned())) })
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        self.inner.answer_permission(request_id, answer)
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        self.inner.cancel(grace)
    }
}

/// [`Hooked`]'s counters, one per follow-up method.
const OPEN: usize = 0;
const NEXT: usize = 1;
const SETTLE: usize = 2;
const CLOSE: usize = 3;
const DROPPED: usize = 4;

/// What a [`Hooked`] runs inside its first `next_follow_up`, against its store.
type Hook = Box<dyn FnOnce(MemStore) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Where a [`Hook`] runs relative to the read it rides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    /// The hook, then the read: the caller sees what the hook wrote.
    BeforeRead,
    /// The read, then the hook, then the read's answer: the caller misses what the hook wrote.
    AfterRead,
}

/// `record/relay.rs`'s private `TRANSIENT_READS`: how many reads of the step's follow-up, and how
/// many closes of an open window, `drive` attempts before it gives up (B-10, B-11).
const TRANSIENT_READS: usize = 30;

/// A [`Hooked`] failure switch that never runs out.
const ALWAYS: usize = usize::MAX;

/// `MemStore`'s relay surface, counting the five follow-up methods, running `on_next` once inside
/// the first `next_follow_up` (a [`When::BeforeRead`] hook runs even when that read then fails),
/// and failing each follow-up method `Unreachable` its switch's number of times first
/// ([`Hooked::failing`]; [`ALWAYS`] for ever). `claimed`, when set, goes `true` once a
/// `settle_follow_up` answers `Settled`: a recorder store watching it fails after a claim
/// (B-9, review M-4).
struct Hooked {
    store: MemStore,
    calls: [AtomicUsize; 5],
    on_next: Mutex<Option<(When, Hook)>>,
    /// Failures left per method; [`ALWAYS`] is for ever.
    fails: [AtomicUsize; 5],
    claimed: Option<Arc<AtomicBool>>,
}

impl std::fmt::Debug for Hooked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hooked")
            .field("calls", &self.calls)
            .field("fails", &self.fails)
            .finish_non_exhaustive()
    }
}

impl Hooked {
    fn new(store: MemStore) -> Self {
        Self {
            store,
            calls: Default::default(),
            on_next: Mutex::new(None),
            fails: Default::default(),
            claimed: None,
        }
    }

    /// `method` answers `Unreachable` its next `times` calls ([`ALWAYS`]: every call).
    fn failing(self, method: usize, times: usize) -> Self {
        self.fails[method].store(times, Ordering::SeqCst);
        self
    }

    /// `claimed` goes `true` at the first `Settled` claim.
    fn tripping(self, claimed: Arc<AtomicBool>) -> Self {
        Self {
            claimed: Some(claimed),
            ..self
        }
    }

    /// The switch of `method`: an `Unreachable` while failures are left (one spent), else `None`.
    fn fail(&self, method: usize) -> Option<StoreError> {
        let left = self.fails[method].load(Ordering::SeqCst);
        if left == 0 {
            return None;
        }
        if left != ALWAYS {
            self.fails[method].store(left - 1, Ordering::SeqCst);
        }
        Some(StoreError::Unreachable(format!(
            "follow-up method {method} is switched off"
        )))
    }

    fn with_hook(store: MemStore, when: When, hook: Hook) -> Self {
        Self {
            on_next: Mutex::new(Some((when, hook))),
            ..Self::new(store)
        }
    }

    fn count(&self, method: usize) {
        self.calls[method].fetch_add(1, Ordering::SeqCst);
    }

    /// `[open, next, settle, close, close_dropped]`.
    fn counts(&self) -> [usize; 5] {
        std::array::from_fn(|method| self.calls[method].load(Ordering::SeqCst))
    }
}

impl htui_core::store::RelayStore for Hooked {
    async fn open_permission(&self, open: OpenPermission) -> StoreResult<PermissionId> {
        WriteStore::open_permission(&self.store, open).await
    }

    async fn permission(&self, id: PermissionId) -> StoreResult<Option<StepPermission>> {
        htui_core::store::RelayStore::permission(&self.store, id).await
    }

    async fn apply_permission(
        &self,
        id: PermissionId,
        owner: Uuid,
    ) -> StoreResult<Option<PermissionChoice>> {
        WriteStore::apply_permission(&self.store, id, owner).await
    }

    async fn settle_permissions(
        &self,
        session: RelaySessionId,
        to: PermissionStatus,
    ) -> StoreResult<u64> {
        WriteStore::settle_permissions(&self.store, session, to).await
    }

    async fn open_follow_ups(
        &self,
        run: RunId,
        step: StepId,
        session: RelaySessionId,
        owner: Uuid,
    ) -> StoreResult<bool> {
        self.count(OPEN);
        if let Some(err) = self.fail(OPEN) {
            return Err(err);
        }
        WriteStore::open_follow_ups(&self.store, run, step, session, owner).await
    }

    async fn next_follow_up(
        &self,
        step: StepId,
        session: RelaySessionId,
    ) -> StoreResult<Option<QueuedFollowUp>> {
        self.count(NEXT);
        let hook = self.on_next.lock().expect("not poisoned").take();
        let hook = match hook {
            Some((When::BeforeRead, hook)) => {
                hook(self.store.clone()).await;
                None
            }
            other => other,
        };
        if let Some(err) = self.fail(NEXT) {
            // An `AfterRead` hook rides the first read that is answered.
            *self.on_next.lock().expect("not poisoned") = hook;
            return Err(err);
        }
        match hook {
            Some((_, hook)) => {
                let read = WriteStore::next_follow_up(&self.store, step, session).await;
                hook(self.store.clone()).await;
                read
            }
            None => WriteStore::next_follow_up(&self.store, step, session).await,
        }
    }

    async fn settle_follow_up(
        &self,
        id: RunCommandId,
        owner: Uuid,
        to: FollowUpSettle,
    ) -> StoreResult<SettleOutcome> {
        self.count(SETTLE);
        if let Some(err) = self.fail(SETTLE) {
            return Err(err);
        }
        let settled = WriteStore::settle_follow_up(&self.store, id, owner, to).await;
        if let (Ok(SettleOutcome::Settled), Some(claimed)) = (&settled, &self.claimed) {
            claimed.store(true, Ordering::SeqCst);
        }
        settled
    }

    async fn close_follow_ups(
        &self,
        step: StepId,
        session: RelaySessionId,
        reason: &str,
    ) -> StoreResult<u64> {
        self.count(CLOSE);
        if let Some(err) = self.fail(CLOSE) {
            return Err(err);
        }
        WriteStore::close_follow_ups(&self.store, step, session, reason).await
    }

    async fn close_dropped_follow_ups(
        &self,
        run: RunId,
        owner: Uuid,
        reason: &str,
    ) -> StoreResult<u64> {
        self.count(DROPPED);
        if let Some(err) = self.fail(DROPPED) {
            return Err(err);
        }
        WriteStore::close_dropped_follow_ups(&self.store, run, owner, reason).await
    }
}

/// D6 steps 2-7 (PRD Q1, R-HIS-1): a follow-up queued while turn 0 is parked is claimed at the
/// turn's `done`, recorded at `turn + 1` masked by the executor's scrubber and sent; turn 1 runs,
/// and the session ends at its `done` with the window closed.
#[tokio::test(start_paused = true)]
async fn a_follow_up_queued_during_the_turn_is_sent_at_its_end() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_turns(ends()), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let mut control = Control::never();

    let (out, id) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let id = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(id)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "turn 1's `done` ends the session"
    );
    let id = id.expect("the client saw the parked request");
    let log = log(&fx.store, fx.step).await;
    assert_eq!(
        follow_ups_in(&log),
        vec![(1, json!({ "text": "also run [REDACTED] checks" }))],
        "recorded at turn 1, masked by the executor (R-HIS-1)"
    );
    let sent = position(&log, EventKind::FollowUp).expect("a follow_up row");
    assert!(
        log.len() > sent + 1 && log[sent + 1..].iter().all(|row| row.turn == 1),
        "turn 1's rows follow it: {log:?}"
    );
    assert_eq!(dones_in(&log), 2, "two turns");
    assert_eq!(
        the_follow_up(&fx.store, id),
        (RunCommandStatus::Applied, None, false),
        "claimed for the next turn, its text cleared (I-5)"
    );
    let windows = fx.store.follow_up_windows();
    assert_eq!(windows.len(), 1, "one window: {windows:?}");
    assert!(windows[0].closed_at.is_some(), "the exit closed it");
}

/// PRD Q1, "repeating until none is pending": a second follow-up queued during the first
/// follow-up's turn starts a third turn.
#[tokio::test(start_paused = true)]
async fn two_follow_ups_run_two_more_turns() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let script = Script::turns(vec![parked_turn(), parked_again(SECOND), ends()]);
    let mut session = fake(script, fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let mut control = Control::never();

    let (out, ids) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let first = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            let row = parked_row_for(&fx.store, SECOND).await?;
            let second = queued(&fx.store, fx.step, "and the docs").await;
            answer(&fx.store, &row, ALLOW).await;
            Some((first, second))
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "turn 2's `done` ends the session"
    );
    let (first, second) = ids.expect("the client saw both parked requests");
    let log = log(&fx.store, fx.step).await;
    assert_eq!(
        follow_ups_in(&log),
        vec![
            (1, json!({ "text": "also run [REDACTED] checks" })),
            (2, json!({ "text": "and the docs" })),
        ],
        "one follow_up row per extra turn"
    );
    assert_eq!(dones_in(&log), 3, "three turns");
    assert_eq!(
        the_follow_up(&fx.store, first),
        (RunCommandStatus::Applied, None, false)
    );
    assert_eq!(
        the_follow_up(&fx.store, second),
        (RunCommandStatus::Applied, None, false)
    );
}

/// D6 step 3: with nothing queued the session ends at its first `done` after one look; the window
/// writes no `session_event`, so the log is the one `follow_ups: false` writes.
#[tokio::test(start_paused = true)]
async fn without_a_follow_up_the_session_ends_at_its_first_done() {
    let (run, step) = (RunId::new(), StepId::new());
    let scrubber = scrubber();
    let policy = PermissionPolicy::default();

    let fx = leased_at(run, step).await;
    running(&fx).await;
    let hooked = Hooked::new(fx.store.clone());
    let relay = Relay {
        follow_ups: true,
        ..relay_over(&hooked, &fx, &policy)
    };
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(plain_script(), step).await;
    let out = within(drive(
        session.as_mut(),
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    let bare = leased_at(run, step).await;
    running(&bare).await;
    let plain = self::relay(&bare, &policy);
    let mut recorder = fenced_recorder(&bare, &scrubber);
    let mut session = fake(plain_script(), step).await;
    let without = within(drive(
        session.as_mut(),
        &mut recorder,
        Some(&plain),
        &mut Control::never(),
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the turn's own `done`"
    );
    assert_eq!(out, without, "the same answer as without a window");
    assert_eq!(
        hooked.counts(),
        [1, 1, 0, 1, 0],
        "one open, one look, no claim, one close"
    );
    let windows = fx.store.follow_up_windows();
    assert_eq!(windows.len(), 1, "one window: {windows:?}");
    assert!(windows[0].closed_at.is_some(), "the exit closed it");
    let log = log(&fx.store, step).await;
    assert_eq!(dones_in(&log), 1, "one turn");
    assert_eq!(
        log,
        self::log(&bare.store, step).await,
        "row for row the log without a window"
    );
}

/// B-15: a follow-up that lands after the turn end's last look is not sent; the window's close
/// refuses it `FOLLOW_UP_SESSION_ENDED`.
#[tokio::test(start_paused = true)]
async fn a_follow_up_queued_after_the_last_check_is_refused_by_the_close() {
    let fx = leased().await;
    running(&fx).await;
    let step = fx.step;
    let hooked = Hooked::with_hook(
        fx.store.clone(),
        When::AfterRead,
        Box::new(move |store| {
            Box::pin(async move {
                queued(&store, step, &follow_up_text()).await;
            })
        }),
    );
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(plain_script(), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = Relay {
        follow_ups: true,
        ..relay_over(&hooked, &fx, &policy)
    };

    let out = within(drive(
        session.as_mut(),
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the session ends at its first `done`"
    );
    assert_eq!(hooked.counts()[NEXT], 1, "one look, which missed the row");
    assert_eq!(
        follow_ups(&fx.store),
        vec![(
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        )],
        "the close refused it"
    );
    let log = log(&fx.store, fx.step).await;
    assert!(follow_ups_in(&log).is_empty(), "nothing was sent");
    assert_eq!(dones_in(&log), 1, "one turn");
}

/// D6 step 3, D7: a run-cap breach ends the turn `cancelled` (`enforce_breach`), and the follow-up
/// queued while it was parked is never applied; the close refuses it.
#[tokio::test(start_paused = true)]
async fn a_cap_breach_applies_no_follow_up() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber).with_run_cap(RunCap {
        micros: 1_000,
        grace: Duration::ZERO,
    });
    let turn0 = {
        let mut events = vec![
            call(&format!("run the suite with {SECRET}")),
            park("Allow"),
            ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
                cost_micros: Some(5_000),
                ..UsageEvent::default()
            })),
        ];
        events.extend(finished());
        events
    };
    let mut session = fake(Script::turns(vec![turn0, ends()]), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let mut control = Control::never();

    let (out, id) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let id = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(id)
        }
    );
    let _ = recorder.finish().await;

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::Cancelled
        }),
        "the breach's `done`"
    );
    let id = id.expect("the client saw the parked request");
    assert_eq!(
        the_follow_up(&fx.store, id),
        (
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        ),
        "the close refused it"
    );
    assert!(
        follow_ups_in(&log(&fx.store, fx.step).await).is_empty(),
        "nothing was sent"
    );
}

/// D6 step 3: a turn whose own `done` says `cancelled` applies no follow-up.
#[tokio::test(start_paused = true)]
async fn a_cancelled_stop_applies_no_follow_up() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let turn0 = {
        let mut events = parked_turn();
        events.pop();
        events.push(ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::Cancelled,
        })));
        events
    };
    let mut session = fake(Script::turns(vec![turn0, ends()]), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let mut control = Control::never();

    let (out, id) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let id = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(id)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::Cancelled
        }),
        "the turn's own `done`"
    );
    let id = id.expect("the client saw the parked request");
    assert_eq!(
        the_follow_up(&fx.store, id),
        (
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        ),
        "the close refused it"
    );
    let log = log(&fx.store, fx.step).await;
    assert!(follow_ups_in(&log).is_empty(), "nothing was sent");
    assert_eq!(dones_in(&log), 1, "one turn");
}

/// D6 step 3: a cancel signalled as the turn ends applies no follow-up and starts no turn; the
/// session's answer is still that turn's `done`.
#[tokio::test(start_paused = true)]
async fn a_cancel_signalled_at_the_turn_end_applies_no_follow_up() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let (sender, mut control) = control_channel();
    let mut session = CancelAtDone {
        inner: fake(two_turns(ends()), fx.step).await,
        sender,
        grace: Duration::from_secs(3),
    };
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);

    let (out, id) = tokio::join!(
        within(drive(
            &mut session,
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let id = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(id)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "turn 0's `done`"
    );
    let id = id.expect("the client saw the parked request");
    assert_eq!(
        the_follow_up(&fx.store, id),
        (
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        ),
        "the close refused it"
    );
    let log = log(&fx.store, fx.step).await;
    assert!(follow_ups_in(&log).is_empty(), "nothing was sent");
    assert_eq!(dones_in(&log), 1, "no second turn");
}

/// D6 step 8: a cancel during a follow-up turn leaves through the graceful cancel, and the close
/// refuses what is still queued with the cancelled-session sentence.
#[tokio::test(start_paused = true)]
async fn a_cancel_during_a_follow_up_turn_closes_with_the_cancelled_sentence() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let turn1 = vec![
        call("run the suite again"),
        ScriptEvent::ParkPermission(permission_request(SECOND, "Allow")),
        ScriptEvent::ExpectCancel,
    ];
    let mut session = fake(two_turns(turn1), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let (signal, mut control) = control_channel();

    let (out, ids) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let first = queued(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            parked_row_for(&fx.store, SECOND).await?;
            let second = queued(&fx.store, fx.step, "and the docs").await;
            signal.send_replace(Signal::Cancel {
                grace: Duration::from_secs(3),
            });
            Some((first, second))
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Err(DriverError::Cancelled), "a graceful cancel");
    let (first, second) = ids.expect("the client saw both parked requests");
    assert_eq!(
        the_follow_up(&fx.store, first),
        (RunCommandStatus::Applied, None, false),
        "the first was sent"
    );
    assert_eq!(
        the_follow_up(&fx.store, second),
        (
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_CANCELLED.to_owned()),
            false
        ),
        "the close refused the second with the cancelled sentence"
    );
    let windows = fx.store.follow_up_windows();
    assert_eq!(windows.len(), 1, "one window: {windows:?}");
    assert!(
        windows[0].closed_at.is_some(),
        "the cancel's exit closed it"
    );
    assert_eq!(follow_ups_in(&log(&fx.store, fx.step).await).len(), 1);
}

/// D6 step 5, MOD-40 D1: a claim whose lease moved is the walk's fence; nothing more is written,
/// so the row stays pending with its text and the window stays open.
#[tokio::test(start_paused = true)]
async fn a_fenced_claim_ends_the_walk_and_closes_nothing() {
    let fx = leased().await;
    running(&fx).await;
    let (run, step, owner) = (fx.run, fx.step, fx.owner);
    let hooked = Hooked::with_hook(
        fx.store.clone(),
        When::BeforeRead,
        Box::new(move |store| {
            Box::pin(async move {
                queued(&store, step, &follow_up_text()).await;
                steal_lease(&store, run, owner).await;
            })
        }),
    );
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(
        Script::turns(vec![plain_script().turns[0].events.clone(), ends()]),
        step,
    )
    .await;
    let policy = PermissionPolicy::default();
    let relay = Relay {
        follow_ups: true,
        ..relay_over(&hooked, &fx, &policy)
    };

    let out = within(drive(
        session.as_mut(),
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;

    assert_eq!(
        out,
        Err(DriverError::Store(StoreError::Fenced { step })),
        "the fenced claim is the answer"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(RunCommandStatus::Pending, None, true)],
        "a fenced writer writes nothing more: the row stays pending"
    );
    assert_eq!(hooked.counts()[CLOSE], 0, "no close was attempted");
    let windows = fx.store.follow_up_windows();
    assert_eq!(windows.len(), 1, "one window: {windows:?}");
    assert_eq!(windows[0].closed_at, None, "the window stays open");
    assert!(follow_ups_in(&log(&fx.store, step).await).is_empty());
}

/// I-8: `follow_ups: false` makes no follow-up call, and the log is today's.
#[tokio::test(start_paused = true)]
async fn drive_without_follow_ups_makes_no_follow_up_call() {
    let (run, step) = (RunId::new(), StepId::new());
    let scrubber = scrubber();
    let policy = PermissionPolicy::default();

    let fx = leased_at(run, step).await;
    let hooked = Hooked::new(fx.store.clone());
    let relay = relay_over(&hooked, &fx, &policy);
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(answered_script(), step).await;
    let mut control = Control::never();
    let (out, parked) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            answer(&fx.store, &row, ALLOW).await;
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    let today = leased_at(run, step).await;
    let plain = self::relay(&today, &policy);
    let mut recorder = fenced_recorder(&today, &scrubber);
    let mut session = fake(answered_script(), step).await;
    let (by_today, _) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&plain),
            &mut control
        )),
        async {
            let row = parked_row(&today.store).await?;
            answer(&today.store, &row, ALLOW).await;
            Some(row)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert!(parked.is_some(), "the request was relayed");
    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        })
    );
    assert_eq!(out, by_today, "the same answer");
    assert_eq!(hooked.counts(), [0; 5], "no follow-up call (I-8)");
    assert!(fx.store.follow_up_windows().is_empty(), "no window");
    assert_eq!(
        log(&fx.store, step).await,
        log(&today.store, step).await,
        "the same rows"
    );
}

/// D6 step 4: a text the executor's scrubber refuses is never recorded: the row is refused with
/// the rule, no `follow_up` and no `scrub_residue` row is written, and the step is not failed.
#[tokio::test(start_paused = true)]
async fn an_executor_scrub_refusal_refuses_the_row_with_its_rule() {
    let fx = leased().await;
    running(&fx).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_turns(ends()), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);
    let mut control = Control::never();

    let (out, id) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            // The client's pattern-only check passes it (no token start before `sk-ant-`); the
            // executor masks the secret, which puts a token start right before it.
            let id = queued(&fx.store, fx.step, &format!("{SECRET}{RESIDUE}")).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(id)
        }
    );
    recorder
        .finish()
        .await
        .expect("a refused follow-up does not fail the step");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "turn 0's `done`"
    );
    let id = id.expect("the client saw the parked request");
    assert_eq!(
        the_follow_up(&fx.store, id),
        (
            RunCommandStatus::Refused,
            Some(executor_scrub_refusal("anthropic_api_key")),
            false
        ),
        "refused with the executor's rule"
    );
    let log = log(&fx.store, fx.step).await;
    assert!(follow_ups_in(&log).is_empty(), "nothing was sent");
    assert!(
        log.iter().all(|row| row.payload["code"] != "scrub_residue"),
        "no scrub_residue row: {log:?}"
    );
    assert_eq!(dones_in(&log), 1, "one turn");
}

/// D6 step 1, B-10: a window that did not open (a transient error) costs the step nothing: the
/// session runs, an enqueue is refused `NotStarted`, no turn end looks, and one close is tried.
#[tokio::test(start_paused = true)]
async fn a_failed_open_runs_the_session_without_a_window() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = Hooked::new(fx.store.clone()).failing(OPEN, ALWAYS);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_turns(ends()), fx.step).await;
    let policy = PermissionPolicy::default();
    let relay = Relay {
        follow_ups: true,
        ..relay_over(&hooked, &fx, &policy)
    };
    let mut control = Control::never();

    let (out, refusal) = tokio::join!(
        within(drive(
            session.as_mut(),
            &mut recorder,
            Some(&relay),
            &mut control
        )),
        async {
            let row = parked_row(&fx.store).await?;
            let refusal = enqueue(&fx.store, fx.step, &follow_up_text()).await;
            answer(&fx.store, &row, ALLOW).await;
            Some(refusal)
        }
    );
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(DoneEvent {
            stop_reason: StopReason::EndTurn
        }),
        "the session ran"
    );
    assert_eq!(
        refusal,
        Some(FollowUpRequest::Refused(FollowUpRefusal::NotStarted)),
        "no window, no follow-up"
    );
    assert_eq!(
        hooked.counts(),
        [1, 0, 0, 1, 0],
        "one open, no look, one close attempt"
    );
    assert_eq!(dones_in(&log(&fx.store, fx.step).await), 1, "one turn");
}

/// D6 step 1: a window the lease no longer allows is the walk's fence, before the first pull.
#[tokio::test(start_paused = true)]
async fn a_fenced_open_drives_nothing() {
    let fx = leased().await;
    running(&fx).await;
    steal_lease(&fx.store, fx.run, fx.owner).await;
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = Queued::new([
        call_event("run the suite"),
        DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        }),
    ]);
    let policy = PermissionPolicy::default();
    let relay = relay_with_follow_ups(&fx, &policy);

    let out = within(drive(
        &mut session,
        &mut recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await;

    assert_eq!(
        out,
        Err(DriverError::Store(StoreError::Fenced { step: fx.step })),
        "the fenced open is the answer"
    );
    assert_eq!(session.queue.len(), 2, "the session was never pulled");
    assert!(
        log(&fx.store, fx.step).await.is_empty(),
        "nothing was pulled or recorded"
    );
    assert!(fx.store.follow_up_windows().is_empty(), "no window");
}

// ---------------------------------------------------------------------------------------------
// MOD-70 review M-4: the follow-up loop's failure and retry paths, over [`Hooked`]'s switches
// ---------------------------------------------------------------------------------------------

/// A [`Hooked`] whose first look finds [`follow_up_text`] queued for `step`: the queue happens
/// inside that look, before its read (and before its switch fails it).
fn queues_before_the_look(store: MemStore, step: StepId) -> Hooked {
    Hooked::with_hook(
        store,
        When::BeforeRead,
        Box::new(move |store| {
            Box::pin(async move {
                queued(&store, step, &follow_up_text()).await;
            })
        }),
    )
}

/// A [`Hooked`] whose first look misses [`follow_up_text`], queued for `step` right after that
/// read: the row is pending when the window closes.
fn queues_after_the_look(store: MemStore, step: StepId) -> Hooked {
    Hooked::with_hook(
        store,
        When::AfterRead,
        Box::new(move |store| {
            Box::pin(async move {
                queued(&store, step, &follow_up_text()).await;
            })
        }),
    )
}

/// Two turns that end at once: turn 1 is what a sent follow-up starts.
fn two_plain_turns() -> Script {
    Script::turns(vec![ends(), ends()])
}

/// `drive` over `hooked` with a follow-up window, `session` and `recorder`, within [`LIMIT`].
async fn drive_hooked<S: htui_core::store::RecorderStore>(
    fx: &Leased,
    hooked: &Hooked,
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
) -> Result<DoneEvent, DriverError> {
    let policy = PermissionPolicy::default();
    let relay = Relay {
        follow_ups: true,
        ..relay_over(hooked, fx, &policy)
    };
    within(drive(
        session,
        recorder,
        Some(&relay),
        &mut Control::never(),
    ))
    .await
}

/// The turn's own `done`.
const END_TURN: DoneEvent = DoneEvent {
    stop_reason: StopReason::EndTurn,
};

/// `step`'s one window, closed or not.
fn window_closed(store: &MemStore, step: StepId) -> bool {
    let windows: Vec<_> = store
        .follow_up_windows()
        .into_iter()
        .filter(|window| window.run_step_id == step)
        .collect();
    assert_eq!(windows.len(), 1, "one window for the step: {windows:?}");
    windows[0].closed_at.is_some()
}

/// D6 steps 5-7: a send that fails after the claim and the record fails the session with the
/// send's error. The row stays `applied` (the claim precedes the send by design), its text gone,
/// and the follow-up is in the step's log; the window still closes, refusing nothing.
#[tokio::test(start_paused = true)]
async fn a_failed_send_after_the_claim_fails_the_session_and_keeps_the_row_applied() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = queues_before_the_look(fx.store.clone(), fx.step);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = FailingSend {
        inner: fake(two_plain_turns(), fx.step).await,
    };

    let out = drive_hooked(&fx, &hooked, &mut session, &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Err(DriverError::Transport(SEND_FAILED.to_owned())),
        "the send's failure is the session's"
    );
    assert_eq!(
        hooked.counts(),
        [1, 1, 1, 1, 0],
        "one open, one look, one claim, one close"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(RunCommandStatus::Applied, None, false)],
        "claimed before the send: applied, its text gone (I-5)"
    );
    let log = log(&fx.store, fx.step).await;
    assert_eq!(
        follow_ups_in(&log),
        vec![(1, json!({ "text": "also run [REDACTED] checks" }))],
        "recorded before the send"
    );
    assert_eq!(dones_in(&log), 1, "no second turn");
    assert!(window_closed(&fx.store, fx.step), "the exit closed it");
}

/// D6 steps 5-6: a record that fails after the claim (the log's append is down) fails the session
/// with the store's error before anything is sent. The row stays `applied`, its text gone.
#[tokio::test(start_paused = true)]
async fn a_failed_record_after_the_claim_fails_the_session_and_keeps_the_row_applied() {
    let fx = leased().await;
    running(&fx).await;
    let down = Arc::new(AtomicBool::new(false));
    let hooked = queues_before_the_look(fx.store.clone(), fx.step).tripping(Arc::clone(&down));
    let flaky = FlakyAppends {
        store: &fx.store,
        down: Arc::clone(&down),
    };
    let scrubber = scrubber();
    let mut recorder = Recorder::new(&flaky, &scrubber, fx.step, false, None)
        .with_fence(StepFence::Lease(fx.owner));
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    down.store(false, Ordering::SeqCst);
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Err(DriverError::Store(StoreError::Unreachable(
            "the log is switched off".to_owned()
        ))),
        "the record's failure is the session's"
    );
    assert_eq!(
        hooked.counts(),
        [1, 1, 1, 1, 0],
        "one open, one look, one claim, one close"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(RunCommandStatus::Applied, None, false)],
        "the claim stands: applied, its text gone (I-5)"
    );
    assert_eq!(
        dones_in(&log(&fx.store, fx.step).await),
        1,
        "nothing was sent: no second turn"
    );
    assert!(window_closed(&fx.store, fx.step), "the exit closed it");
}

/// B-11: a claim the store fails (not a fence) is a `warn`: the session ends at its turn's
/// `done`, and the window's close refuses the row it could not claim.
#[tokio::test(start_paused = true)]
async fn a_failed_claim_ends_the_session_and_the_close_refuses_the_row() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = queues_before_the_look(fx.store.clone(), fx.step).failing(SETTLE, ALWAYS);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Ok(END_TURN), "turn 0's `done` ends the session");
    assert_eq!(
        hooked.counts(),
        [1, 1, 1, 1, 0],
        "one open, one look, one failed claim, one close"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        )],
        "the close refused it, its text gone"
    );
    let log = log(&fx.store, fx.step).await;
    assert!(follow_ups_in(&log).is_empty(), "nothing was sent");
    assert_eq!(dones_in(&log), 1, "one turn");
}

/// B-11: the look rides out `TRANSIENT_READS - 1` transient failures and reads the row on the
/// last attempt; the follow-up is sent and turn 1 runs.
#[tokio::test(start_paused = true)]
async fn the_look_rides_out_transient_failures() {
    let fx = leased().await;
    running(&fx).await;
    let hooked =
        queues_before_the_look(fx.store.clone(), fx.step).failing(NEXT, TRANSIENT_READS - 1);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Ok(END_TURN), "turn 1's `done` ends the session");
    assert_eq!(
        hooked.counts(),
        [1, TRANSIENT_READS + 1, 1, 1, 0],
        "turn 0's look took every attempt, turn 1's one; one claim, one close"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(RunCommandStatus::Applied, None, false)],
        "sent, its text gone"
    );
    let log = log(&fx.store, fx.step).await;
    assert_eq!(follow_ups_in(&log).len(), 1, "one follow-up recorded");
    assert_eq!(dones_in(&log), 2, "two turns");
}

/// B-11: a look that keeps failing gives up after `TRANSIENT_READS` attempts; the session ends at
/// turn 0's `done`, and the window's close refuses the row nobody read.
#[tokio::test(start_paused = true)]
async fn the_look_gives_up_after_its_transient_reads() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = queues_before_the_look(fx.store.clone(), fx.step).failing(NEXT, ALWAYS);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Ok(END_TURN), "turn 0's `done` ends the session");
    assert_eq!(
        hooked.counts(),
        [1, TRANSIENT_READS, 0, 1, 0],
        "every attempt failed; no claim, one close"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        )],
        "the close refused it, its text gone"
    );
    assert_eq!(dones_in(&log(&fx.store, fx.step).await), 1, "one turn");
}

/// B-10: an open window's close retries transient failures and lands on its last attempt,
/// refusing the row queued after the last look.
#[tokio::test(start_paused = true)]
async fn an_open_windows_close_rides_out_transient_failures() {
    let fx = leased().await;
    running(&fx).await;
    let hooked =
        queues_after_the_look(fx.store.clone(), fx.step).failing(CLOSE, TRANSIENT_READS - 1);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Ok(END_TURN), "turn 0's `done` ends the session");
    assert_eq!(
        hooked.counts(),
        [1, 1, 0, TRANSIENT_READS, 0],
        "the close took every attempt"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(
            RunCommandStatus::Refused,
            Some(FOLLOW_UP_SESSION_ENDED.to_owned()),
            false
        )],
        "the last attempt refused it, its text gone"
    );
    assert!(window_closed(&fx.store, fx.step), "the close landed");
}

/// B-10, R-3: an open window's close that keeps failing gives up after `TRANSIENT_READS`
/// attempts with a `warn`. The row stays pending with its text and the window open: what the
/// engine closes when it settles the run terminal (review M-3), a cancel or the next re-take.
#[tokio::test(start_paused = true)]
async fn an_open_windows_close_gives_up_after_its_transient_reads() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = queues_after_the_look(fx.store.clone(), fx.step).failing(CLOSE, ALWAYS);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(
        out,
        Ok(END_TURN),
        "a close that fails is not the session's error"
    );
    assert_eq!(
        hooked.counts(),
        [1, 1, 0, TRANSIENT_READS, 0],
        "the close took every attempt"
    );
    assert_eq!(
        follow_ups(&fx.store),
        vec![(RunCommandStatus::Pending, None, true)],
        "stranded: pending, its text kept"
    );
    assert!(!window_closed(&fx.store, fx.step), "the window stays open");
}

/// B-10: a window whose open failed gets one close attempt, whose failure is a `warn`.
#[tokio::test(start_paused = true)]
async fn a_window_that_never_opened_tries_one_close() {
    let fx = leased().await;
    running(&fx).await;
    let hooked = Hooked::new(fx.store.clone())
        .failing(OPEN, ALWAYS)
        .failing(CLOSE, ALWAYS);
    let scrubber = scrubber();
    let mut recorder = fenced_recorder(&fx, &scrubber);
    let mut session = fake(two_plain_turns(), fx.step).await;

    let out = drive_hooked(&fx, &hooked, session.as_mut(), &mut recorder).await;
    recorder.finish().await.expect("the recorder closes");

    assert_eq!(out, Ok(END_TURN), "the session ran");
    assert_eq!(
        hooked.counts(),
        [1, 0, 0, 1, 0],
        "one failed open, no look, one failed close"
    );
    assert!(fx.store.follow_up_windows().is_empty(), "no window");
}
