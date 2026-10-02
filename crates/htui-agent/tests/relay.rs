//! MOD-42 T2: `drive`, the permission relay and the run's control (plan D6, D8, I-5, I-7, I-8;
//! blueprint B-2, B-11, B-14, B-15, B-16).
//!
//! Every case drives a [`FakeSession`] through `drive` into a `MemStore::demo()` whose run was
//! created and claimed with an owner, as the store conformance's `leased_step` does. The "second
//! client" is a future joined with `drive` on the same task: it polls `relay_view(item)` on the
//! same store until a row appears and then answers, cancels or supersedes it, which is all a
//! Runs pane on another box can do (I-1: it writes no `session_event`). Waits run on a paused
//! clock with `Relay.poll` at 10 ms, so no case sleeps for real.

#![cfg(feature = "test-support")]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
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
};
use htui_agent::fake::FakeDriver;
use htui_agent::record::{
    Control, NoRelay, RELAY_GRACE, Recorder, Relay, Signal, control_channel, drive, pump,
};
use htui_core::fixtures::ids;
use htui_core::model::{
    AnswerOutcome, Claim, EventKind, GraphSnapshot, Isolation, NewRun, NewRunStep, OpenPermission,
    PermissionChoice, PermissionId, PermissionStatus, RelayOptionKind, RelaySessionId, RunId,
    RunMode, SessionEvent, SnapshotGraph, SnapshotSettings, StepId, StepPermission,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{
    MemStore, ReadStore, Result as StoreResult, StepFence, StoreError, WriteStore,
};
use serde_json::{Value, json};
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
    }
}

/// The gated call: an `execute` whose title carries `secret`.
fn call(title: &str) -> ScriptEvent {
    ScriptEvent::Emit(call_event(title))
}

/// [`call`]'s event.
fn call_event(title: &str) -> DriverEvent {
    DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: CALL.to_owned(),
        title: title.to_owned(),
        tool_kind: ToolKind::Execute,
        input: json!({ "command": "cargo test" }),
        locations: Vec::new(),
    })
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
}

/// `MemStore`'s recorder surface, except that every `append_events` answers `Unreachable` while
/// `down` is set. The recorder keeps the refused rows owed (MOD-40 D3), so a later flush lands
/// them.
#[derive(Debug)]
struct FlakyAppends<'a> {
    store: &'a MemStore,
    down: AtomicBool,
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
        down: AtomicBool::new(false),
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
