//! The walk's transport-neutral conformance suite (plan D18), behind `test-support`.
//!
//! The shape is the one both existing suites already use
//! (`crates/htui-agent/src/conformance.rs:146-238`,
//! `crates/htui-core/src/store/conformance.rs:36-239`): a `CASES` list that is the suite's API, a
//! `run_case` dispatcher whose `match` panics on a name it does not know, a `run_all` that walks
//! the list, and a unit test that runs every name through the dispatcher so the two cannot drift.
//!
//! T3 landed the list, the harness traits and the case stubs; T5 replaced every stub body with the
//! validation criterion it is named for. A case asserts the *shape* its criterion names — statuses
//! per step, document kinds and their `produced_by_step_id`, the item's `closed_at`, the run's
//! `finished_at` — and never merely that the walk did not error.

use chrono::TimeDelta;
use htui_core::fixtures::ids;
use htui_core::model::{
    AgentId, Gate, GateOutcome, Item, ItemId, ItemPatch, NewRepo, NewStepGraph, PhaseId,
    PhasePatch, RepoId, Run, RunId, RunMode, RunStatus, RunStep, Status, StepGraphId,
    StepGraphPhase, StepStatus,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

use crate::command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
use crate::engine::Resume;
use crate::fake::{FakeIsolator, FakeOrchestrator, ScriptedStep, TestClock};
use crate::isolate::Clock as _;
use crate::status::RunFailure;

/// What a case needs to be handed, and nothing more (plan D18).
///
/// One fresh orchestrator per case, because `claim_run` counts a box's live runs against
/// `max_concurrent_items` — 2 on the fixture box (`crates/htui-core/src/fixtures.rs:418-438`) — and
/// a suite that shared one store would start refusing claims at its third case for a reason no
/// criterion is about (blueprint H-15).
///
/// The trait names no concrete driver, isolator or store, which is the point: milestone 3's harness
/// swaps a real `Isolator` in behind the same eleven cases and adds none of its own.
pub trait CaseHarness: Sync {
    /// The thing a case drives.
    type Orch: Orchestrate;

    /// A new orchestrator, over a store nothing else has written to.
    fn fresh(&self) -> Self::Orch;
}

/// The surface a case drives.
///
/// `store()` is a `MemStore` rather than an `impl WriteStore` because a case asserts over rows and
/// the suite's only store is the in-memory one. The *engine* is what stays generic over
/// `S: WriteStore`; nothing here weakens that.
#[allow(async_fn_in_trait)]
pub trait Orchestrate {
    /// Send one command through the walk.
    ///
    /// # Errors
    /// Whatever the engine refuses with.
    async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError>;

    /// Re-resolve a run's graph and walk it on, or report that the topology moved under it.
    ///
    /// Not in the blueprint's §4.5 list either, and needed for the same reason the other two
    /// out-of-list methods are: `topology_mismatch_parks_on_resume` is ANA-2 §12 criterion 3 and
    /// criterion 3 *is* the resume path. Milestone 5's sweep drives the same entry point.
    ///
    /// # Errors
    /// Whatever the engine refuses with.
    async fn resume(&self, run: RunId) -> Result<Resume, EngineError>;

    /// The store every assertion reads.
    fn store(&self) -> &MemStore;

    /// Script one `(phase name, attempt)`.
    fn script(&self, phase: &str, attempt: i32, step: ScriptedStep);

    /// Name a phase's candidate agents, for the stage-1 interlock.
    ///
    /// Not in the blueprint's §4.5 list, and needed: `cli_agent_is_refused_at_a_gated_phase` is a
    /// [`CASES`] entry whose body is written against this trait and nothing else.
    fn with_candidates(&self, phase: &str, agents: Vec<(AgentId, &str)>);

    /// Elapse time between the driver's `done` and the settle, for the step-deadline case. Also not
    /// in §4.5's list, and needed for the same reason.
    fn advance_after_done(&self, by: TimeDelta);

    /// The isolator, for scripting `after_hash` values.
    fn isolator(&self) -> &FakeIsolator;

    /// The clock, for reading the instants the walk stamped.
    fn clock(&self) -> &TestClock;
}

impl Orchestrate for FakeOrchestrator {
    async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError> {
        Self::dispatch(self, command).await
    }

    async fn resume(&self, run: RunId) -> Result<Resume, EngineError> {
        Self::resume(self, run).await
    }

    fn store(&self) -> &MemStore {
        &self.store
    }

    fn script(&self, phase: &str, attempt: i32, step: ScriptedStep) {
        Self::script(self, phase, attempt, step);
    }

    fn with_candidates(&self, phase: &str, agents: Vec<(AgentId, &str)>) {
        Self::with_candidates(self, phase, agents);
    }

    fn advance_after_done(&self, by: TimeDelta) {
        Self::advance_after_done(self, by);
    }

    fn isolator(&self) -> &FakeIsolator {
        &self.isolator
    }

    fn clock(&self) -> &TestClock {
        &self.clock
    }
}

/// Case names in run order. A name never changes: every binding reports per case.
///
/// Eleven, and the count is pinned in two places on purpose — here by
/// `cases_are_unique_and_eleven` and out of crate by `tests/fake_conformance.rs` (T5) — because a
/// binding that silently ran ten of them would still be green.
///
/// Six are `docs/ANA-2.md` §12's validation criteria (1, 2, 3, 5, 6, 7); four are contract lines
/// §12 does not number but §4.2 states outright; the last is the `finish_run` seam T1 shipped for
/// this walk to use.
pub const CASES: &[&str] = &[
    // ANA-2 §12 criterion 1 (`docs/ANA-2.md:2085`): a FEAT graph walks its four phases.
    "feat_walks_end_to_end",
    // Criterion 2 (`:2088`): invariant 2 — a live run reads its snapshot, never the graph.
    "live_run_ignores_a_gate_edit",
    // Criterion 3 (`:2090`): a graph edited under a parked run is a topology mismatch on resume.
    "topology_mismatch_parks_on_resume",
    // Criterion 5 (`:2096`): approving a step that produced no output document is refused.
    "approve_needs_the_output_document",
    // Criterion 6 (`:2098`): the review loop runs its budget out and escalates to a human.
    "review_rejection_loops_then_escalates",
    // Criterion 7 (`:2103`): two identical `after_hash` values mean the loop stopped making
    // progress, and it is stopped before its retry budget says so.
    "identical_after_hash_stops_the_loop",
    // `docs/ANA-2.md:414`: a missing required input fails the run before a token is spent.
    "missing_input_fails_before_a_token",
    // `:430`: a step that produced no document of its `output_kind` settles `failed`.
    "missing_output_settles_failed",
    // `:482`: a gated phase whose only candidate cannot answer a permission request is refused
    // with `missing_capability: inline_approval`.
    "cli_agent_is_refused_at_a_gated_phase",
    // `:439` and plan D8: a step that outlives its deadline settles `failed`.
    "step_deadline_settles_failed",
    // Plan D7: the last write of a finished walk is `finish_run`, and the item mirrors it.
    "finish_run_is_the_last_write",
];

/// Run one case by name.
///
/// # Panics
/// On a name [`CASES`] holds and this `match` does not — the drift `every_case_name_dispatches`
/// exists to catch — and on any assertion the case itself makes.
pub async fn run_case<H: CaseHarness>(name: &str, harness: &H) {
    match name {
        "feat_walks_end_to_end" => feat_walks_end_to_end(harness).await,
        "live_run_ignores_a_gate_edit" => live_run_ignores_a_gate_edit(harness).await,
        "topology_mismatch_parks_on_resume" => topology_mismatch_parks_on_resume(harness).await,
        "approve_needs_the_output_document" => approve_needs_the_output_document(harness).await,
        "review_rejection_loops_then_escalates" => {
            review_rejection_loops_then_escalates(harness).await;
        }
        "identical_after_hash_stops_the_loop" => identical_after_hash_stops_the_loop(harness).await,
        "missing_input_fails_before_a_token" => missing_input_fails_before_a_token(harness).await,
        "missing_output_settles_failed" => missing_output_settles_failed(harness).await,
        "cli_agent_is_refused_at_a_gated_phase" => {
            cli_agent_is_refused_at_a_gated_phase(harness).await;
        }
        "step_deadline_settles_failed" => step_deadline_settles_failed(harness).await,
        "finish_run_is_the_last_write" => finish_run_is_the_last_write(harness).await,
        other => panic!("unknown conformance case `{other}`; CASES and run_case disagree"),
    }
}

/// Run every case in [`CASES`] order, each against its own fresh orchestrator.
pub async fn run_all<H: CaseHarness>(harness: &H) {
    for name in CASES {
        run_case(name, harness).await;
    }
}

// -- what every case needs, written once ---------------------------------------------------------

/// The run's steps in `(position, attempt, fanout_index)` order.
///
/// A free function over [`Orchestrate`] rather than a method on it: `store()` is the whole seam a
/// case reads through, and a trait that also carried the three obvious projections of it would be
/// a trait milestone 3's real-isolator harness has to implement four more times for nothing.
///
/// # Panics
/// Never: `MemStore` fails no read.
async fn steps_of<O: Orchestrate>(orch: &O, run: RunId) -> Vec<RunStep> {
    orch.store()
        .run_steps(run)
        .await
        .expect("MemStore never fails a read")
}

/// One item row.
///
/// # Panics
/// When the item is not there, which in a case means the fixture moved.
async fn item_of<O: Orchestrate>(orch: &O, id: ItemId) -> Item {
    orch.store()
        .item(id)
        .await
        .expect("MemStore never fails a read")
        .expect("the case names an item the fixture holds")
}

/// One run row.
///
/// # Panics
/// When the run is not there, which in a case means the walk never created it.
async fn run_of<O: Orchestrate>(orch: &O, id: RunId) -> Run {
    orch.store()
        .run(id)
        .await
        .expect("MemStore never fails a read")
        .expect("the case names a run the walk created")
}

/// The item's notes, which is where plan D10 puts every settle reason (`deadline elapsed`, a
/// refusing `stop_reason`, a breached cap, an unrecognised verdict) with `via_step_id` set.
///
/// **`gate_note` is not the place to look for those.** `answer_gate` is the only writer of
/// `run_step.gate_note` and it is `awaiting_approval`-only, so a step the *engine* parked or failed
/// carries no note of its own; the reason is an item note or it is nowhere — which is plan D10 as
/// T4 corrected it.
///
/// # Panics
/// Never: `MemStore` fails no read.
async fn notes_of<O: Orchestrate>(orch: &O, item: ItemId) -> Vec<String> {
    orch.store()
        .notes(item)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .map(|note| note.body)
        .collect()
}

/// `ids::HTUI_FEAT_3` is seeded **`queued`** with a live `RUN_2` on it
/// (`crates/htui-core/src/fixtures.rs:899`, `:1374-1380`), and `create_run` moves an item
/// `open | failed -> queued` only — so `StartRun` on it is a `Constraint` until the seeded run is
/// ended. Cancelling `RUN_2` moves the item `queued -> open` inside `finish_run`'s own transaction,
/// which is the one-line prologue every case that walks `FEAT-3` needs.
///
/// # Panics
/// When `RUN_2` is not cancellable, which means the fixture moved.
async fn free_feat_3<O: Orchestrate>(orch: &O) {
    orch.store()
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock().now())
        .await
        .expect("the seeded run is queued and cancellable");
}

/// Repoints `item` at a clone of its graph whose phases `mutate` has edited.
///
/// **`PhasePatch` carries five fields — `name`, `position`, `template_name`, `gate_hard` and
/// `input_kinds` — and `gate`, `retry_limit`, `isolation`, `fan_out` and `token_budget` are none of
/// them** (`crates/htui-core/src/model/kind.rs:221-232`), so the blueprint's
/// `update_phase(review, expected, PhasePatch { gate: Some(Gate::Never), .. })` recipe does not
/// compile. `create_phase` takes a whole `StepGraphPhase`, so a clone carrying the row a case wants
/// is the reachable edit — and it is the same pair of writers `graph::override_graph` uses.
///
/// # Panics
/// When any of the three writes is refused, which means the fixture moved under the case.
async fn repoint<O: Orchestrate>(orch: &O, item: ItemId, mutate: impl Fn(&mut StepGraphPhase)) {
    let row = item_of(orch, item).await;
    let graph = orch
        .store()
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph");
    let clone = orch
        .store()
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: row.project_id,
            name: format!("{}-edited", row.key),
            description: "a conformance case's edit of the live graph".to_owned(),
        })
        .await
        .expect("the name is fresh");
    for phase in &graph.phases {
        let mut edited = StepGraphPhase {
            id: PhaseId::new(),
            graph_id: clone.id,
            ..phase.phase.clone()
        };
        mutate(&mut edited);
        orch.store()
            .create_phase(&edited)
            .await
            .expect("the clone accepts its phases");
    }
    orch.store()
        .update_item(
            item,
            row.version,
            ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: row.created_by,
                reason: "a conformance case's edit of the live graph".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// Replaces one live phase's `input_kinds`, through the patch writer rather than through a clone.
///
/// The two cases that need it — a required input nothing produces, and a topology that moved under
/// a parked run — both want the edit on the graph the run already resolved, which is what
/// separates them from [`repoint`]'s clone-and-point.
///
/// # Panics
/// When the item resolves to no graph, or the phase's `updated_at` has moved.
async fn set_input_kinds<O: Orchestrate>(orch: &O, item: ItemId, position: usize, kinds: &[&str]) {
    let graph = orch
        .store()
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph");
    let phase = &graph.phases[position].phase;
    orch.store()
        .update_phase(
            phase.id,
            phase.updated_at,
            PhasePatch {
                input_kinds: Some(kinds.iter().map(|kind| (*kind).to_owned()).collect()),
                ..PhasePatch::default()
            },
        )
        .await
        .expect("the phase exists and its version is current");
}

/// The project's primary repository, which the `after_hash` half of §4.4's no-progress predicate
/// needs to have anything to compare (plan D14's happy path: `repo_scope: None` resolves to it).
///
/// # Panics
/// When the project already holds a repo of this name, which the demo fixture never does
/// (`crates/htui-core/src/store/mem.rs:196`: it seeds none at all).
async fn primary_repo<O: Orchestrate>(orch: &O) -> RepoId {
    let repo = RepoId::new();
    orch.store()
        .create_repo(NewRepo {
            id: repo,
            project_id: ids::PROJECT_HTUI,
            name: "htui".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("the demo project has no repo yet");
    repo
}

/// `StartRun` on `item` in manual mode with no requested scope, unwrapped to its run id and rest.
///
/// # Panics
/// When the command is refused, which every caller of this helper expects not to be.
async fn start<O: Orchestrate>(orch: &O, item: ItemId) -> (RunId, Rest) {
    let outcome = orch
        .dispatch(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
        })
        .await
        .expect("the graph resolves and the box has a slot");
    let CommandOutcome::Started { run, rest } = outcome else {
        panic!("`StartRun` answers `Started`, not {outcome:?}");
    };
    (run, rest)
}

/// Answer the run's one parked step, and give back the step that was answered.
///
/// Every case that drives more than one gate goes through here rather than re-finding the parked
/// step by hand: "the step that is `awaiting_approval`" is the walk's own invariant — at most one
/// per run this milestone — and a case that looked it up by position would be asserting the
/// walk's ordering twice.
///
/// # Panics
/// When no step is parked, or when the answer is refused.
async fn answer<O: Orchestrate>(orch: &O, run: RunId, answer: GateAnswer) -> (RunStep, Rest) {
    let steps = steps_of(orch, run).await;
    let parked = steps
        .iter()
        .find(|step| step.status == StepStatus::AwaitingApproval)
        .expect("the walk parked exactly one step")
        .clone();
    let outcome = orch
        .dispatch(Command::AnswerGate {
            run,
            step: parked.id,
            answer,
        })
        .await
        .expect("the parked gate is answerable");
    let CommandOutcome::Answered { rest } = outcome else {
        panic!("`AnswerGate` answers `Answered`, not {outcome:?}");
    };
    (parked, rest)
}

/// [`answer`] with `Approved`, `times` over, keeping the last walk's rest.
///
/// # Panics
/// As [`answer`].
async fn approve<O: Orchestrate>(orch: &O, run: RunId, times: usize) -> Rest {
    let mut rest = None;
    for _ in 0..times {
        rest = Some(answer(orch, run, GateAnswer::Approved).await.1);
    }
    rest.expect("a case never approves zero times")
}

/// The step at `(position, attempt)`, fan-out index 0.
///
/// # Panics
/// When the walk never created it, which is what the caller is asserting it did.
fn at(steps: &[RunStep], position: i32, attempt: i32) -> &RunStep {
    steps
        .iter()
        .find(|step| step.position == position && step.attempt == attempt)
        .unwrap_or_else(|| panic!("the walk created a step at ({position},{attempt})"))
}

// -- the eleven cases ----------------------------------------------------------------------------

/// ANA-2 §12 criterion 1 (`docs/ANA-2.md:2085`): a FEAT graph walks its four phases.
///
/// The shape asserted is blueprint §7's data-flow table read from the store: one step per position
/// in `prd, plan, implement, review` order, every one `done`, one document per position carrying
/// `produced_by_step_id` of the step that earned it, and the item mirroring the run at row 35.
///
/// `prompt_digest` is asserted `Some` rather than re-hashed: stage 3's `set_step_prompt` and the
/// recorder's `record_prompt` write the same digest by construction (blueprint H-13), and a case
/// that re-hashed a scrubbed variant would be pinning the scrubber, not the walk.
async fn feat_walks_end_to_end<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    assert_eq!(rest.position, Some(0), "every seeded phase gates `always`");
    assert_eq!(rest.failure, None);

    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1, "the walk stopped at the first gate");
    assert_eq!(
        (
            steps[0].position,
            steps[0].attempt,
            steps[0].fanout_index,
            steps[0].phase_name.as_str(),
            steps[0].status
        ),
        (0, 1, 0, "prd", StepStatus::AwaitingApproval)
    );
    assert!(
        steps[0].prompt_digest.is_some(),
        "stage 3 wrote the digest and the recorder rewrote the same one (blueprint H-13)"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::AwaitingApproval,
        "the three-write park moves step, run and item (blueprint H-10)"
    );

    // Four approvals: positions 0, 1 and 2 each park the next, and the fourth finishes the run.
    for position in 0..4 {
        let (parked, _) = answer(&orch, run, GateAnswer::Approved).await;
        assert_eq!(
            parked.position, position,
            "the walk parks positions in order"
        );
    }

    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::Done);
    assert_eq!(row.finished_at, Some(orch.clock().now()));
    assert_eq!(row.failure, None);

    let item = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        item.status,
        Status::Done,
        "plan D7: `finish_run` mirrors it"
    );
    assert!(
        item.closed_at.is_some(),
        "a `done` item carries the instant it closed"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 4, "one step per position, no retry");
    assert_eq!(
        steps
            .iter()
            .map(|step| (
                step.position,
                step.attempt,
                step.fanout_index,
                step.phase_name.as_str(),
                step.status,
                step.gate_outcome
            ))
            .collect::<Vec<_>>(),
        [
            (
                0,
                1,
                0,
                "prd",
                StepStatus::Done,
                Some(GateOutcome::Approved)
            ),
            (
                1,
                1,
                0,
                "plan",
                StepStatus::Done,
                Some(GateOutcome::Approved)
            ),
            (
                2,
                1,
                0,
                "implement",
                StepStatus::Done,
                Some(GateOutcome::Approved)
            ),
            (
                3,
                1,
                0,
                "review",
                StepStatus::Done,
                Some(GateOutcome::Approved)
            ),
        ],
        "blueprint §7's data-flow table, read back from the store"
    );

    let documents = orch
        .store()
        .documents(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read");
    for step in &steps {
        let head = documents
            .iter()
            .find(|head| head.kind == step.phase_name)
            .unwrap_or_else(|| {
                panic!("the sink wrote a `{}` document (plan D13)", step.phase_name)
            });
        assert_eq!(
            head.produced_by_step_id,
            Some(step.id),
            "the `{}` document names the step that produced it",
            step.phase_name
        );
    }
}

/// ANA-2 §12 criterion 2 (`docs/ANA-2.md:2088`): a live run reads its snapshot, never the graph.
///
/// Two halves, and the second is the one that makes the first mean anything. A run started *before*
/// the edit keeps parking at every remaining position because its `graph_snapshot` says `always`;
/// a run started *after* it reads `never` and walks to `done` without parking once. Without the
/// second half a walk that had simply stopped reading gates at all would pass.
///
/// The second run is on a **different item** because the first item's run is still the one under
/// test: `create_run` moves an item `open | failed -> queued`, so the same item cannot start a
/// second run until the first is finished, and finishing it first would lose the live-run half.
async fn live_run_ignores_a_gate_edit<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        rest.position,
        Some(0),
        "the snapshot's `prd` gates `always`"
    );

    // The live graph now says `never` at every phase. The run does not read it.
    repoint(&orch, ids::HTUI_FEAT_3, |phase| phase.gate = Gate::Never).await;

    for position in 1..4 {
        approve(&orch, run, 1).await;
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            at(&steps, position, 1).status,
            StepStatus::AwaitingApproval,
            "position {position} parked on the snapshot's `always`, not the live graph's `never`"
        );
    }
    let rest = approve(&orch, run, 1).await;
    assert_eq!(rest.run, RunStatus::Done);

    // A second item, repointed the same way, started *after* the edit: `never` all the way down.
    repoint(&orch, ids::HTUI_ANA_2, |phase| phase.gate = Gate::Never).await;
    let (second, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None),
        "a run created after the edit walks its own snapshot, and that one says `never`"
    );

    let steps = steps_of(&orch, second).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.phase_name.as_str(), step.status, step.gate_outcome))
            .collect::<Vec<_>>(),
        [
            ("research", StepStatus::Done, None),
            ("verdict", StepStatus::Done, None),
        ],
        "`never` passes a step `running -> done` and leaves `gate_outcome` NULL: `answer_gate` is \
         the only writer of it and it is `awaiting_approval`-only (blueprint H-9)"
    );
    assert_eq!(item_of(&orch, ids::HTUI_ANA_2).await.status, Status::Done);
}

/// ANA-2 §12 criterion 3 (`docs/ANA-2.md:2090`): the graph moved under a parked run.
///
/// `input_kinds` is the edit because it is both one of `PhasePatch`'s five fields and a
/// `SnapshotPhase` field, so patching it moves the `topology` digest — which is the whole content of
/// the criterion. The assertion is that **nothing advanced**: the same step rows, plus a note an
/// operator can read. The second half proves the comparison is a comparison and not a constant: an
/// unedited run resumes into the walk.
async fn topology_mismatch_parks_on_resume<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;

    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;

    let before = steps_of(&orch, run).await;
    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::TopologyChanged {
        snapshot,
        live,
        rest,
    } = resumed
    else {
        panic!("the live graph moved, so the digests differ: {resumed:?}");
    };
    assert_ne!(snapshot, live, "two digests, and they are not the same one");
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "the run is where it was: a mismatch is a human's decision, not the engine's"
    );

    let after = steps_of(&orch, run).await;
    assert_eq!(after.len(), before.len(), "the walk is not advanced");
    assert_eq!(
        after
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>()
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body.contains("topology mismatch")),
        "ANA-2 invariant 7: a refusal a human can read: {notes:?}"
    );

    // An unedited run resumes into the walk instead.
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        matches!(
            orch.resume(run).await.expect("the run is readable"),
            Resume::Walked(_)
        ),
        "an untouched graph still hashes to the run's own topology"
    );
}

/// ANA-2 §12 criterion 5 (`docs/ANA-2.md:2096`): approve is refused without the artefact, and the
/// refusal writes nothing.
///
/// The step still settled `Failed(MissingOutput)` and `always` still parked it — that is stage 6's
/// job and it is not what the criterion is about. What the criterion is about is the *next* command:
/// `AnswerGate { Approved }` names the missing kind and leaves every row exactly where it was.
async fn approve_needs_the_output_document<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.script("prd", 1, ScriptedStep::done_without_output());

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "`always` parks it");

    let before = steps_of(&orch, run).await;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].status, StepStatus::AwaitingApproval);
    let documents_before = orch
        .store()
        .documents(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .len();

    let refused = orch
        .dispatch(Command::AnswerGate {
            run,
            step: before[0].id,
            answer: GateAnswer::Approved,
        })
        .await
        .expect_err("there is no `prd` document to approve");
    assert!(
        matches!(&refused, EngineError::MissingOutputForApproval { kind, step }
            if kind == "prd" && *step == before[0].id),
        "the refusal names the kind the human has to supply: {refused}"
    );

    let after = steps_of(&orch, run).await;
    assert_eq!(after.len(), 1, "a refused approve creates no row");
    assert_eq!(after[0].status, StepStatus::AwaitingApproval);
    assert_eq!(after[0].gate_outcome, None, "and answers no gate");
    assert_eq!(
        run_of(&orch, run).await.status,
        RunStatus::AwaitingApproval,
        "nor does it move the run"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::AwaitingApproval
    );
    assert_eq!(
        orch.store()
            .documents(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .len(),
        documents_before,
        "and writes no document of its own"
    );
}

/// ANA-2 §12 criterion 6 (`docs/ANA-2.md:2098`): a human's rejection loops once, then escalates.
///
/// The case runs with a **primary repo in scope** and two *different* `after_hash` values on the two
/// implement attempts, so §4.4's no-progress predicate answers false both times and the only thing
/// left that can stop the loop is the retry budget — `Exhausted` at `retry_limit = 1`, which is what
/// the criterion names. `identical_after_hash_stops_the_loop` is the same walk with the predicate
/// allowed to fire, and the pair is only a pair because this one pins the other reason.
///
/// One scripted hash is queued per step the walk takes, in walk order: the isolator's queue is a
/// FIFO consumed once per `capture`, and `capture` is called once per step.
async fn review_rejection_loops_then_escalates<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let repo = primary_repo(&orch).await;

    orch.script("review", 1, ScriptedStep::review("approve", "first"));
    orch.script("review", 2, ScriptedStep::review("approve", "second"));
    for hash in ["prd", "plan", "h1", "review-1", "h2", "review-2"] {
        orch.isolator().script_after(Some(hash));
    }

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        run_of(&orch, run).await.repo_scope,
        vec![repo],
        "plan D14: `repo_scope: None` resolves to the primary repo"
    );

    // prd, plan, implement 1; the walk then parks at `review`.
    approve(&orch, run, 3).await;
    let steps = steps_of(&orch, run).await;
    assert_eq!(at(&steps, 3, 1).status, StepStatus::AwaitingApproval);

    let (_, rest) = answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "no tests".to_owned(),
        },
    )
    .await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "the loop resumed and parked the new implement"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        at(&steps, 2, 1).status,
        StepStatus::Superseded,
        "plan D5: the implement being redone is retired"
    );
    assert_eq!(
        at(&steps, 3, 1).status,
        StepStatus::Cancelled,
        "plan D5 as T4 corrected it: `failed -> superseded` is illegal, so the rejecting review is \
         retired by the one legal move it has; left `failed` it would rest the walk on it forever"
    );
    assert_eq!(
        at(&steps, 3, 1).gate_outcome,
        Some(GateOutcome::Rejected),
        "`transition_step` moves `status` and nothing else, so the verdict survives the retire"
    );
    assert_eq!(at(&steps, 3, 1).gate_note.as_deref(), Some("no tests"));
    assert_eq!(at(&steps, 2, 2).status, StepStatus::AwaitingApproval);

    // The re-run implement reads the rejecting review through the ordinary `input_kinds`.
    let inputs = orch
        .store()
        .resolve_inputs(
            ids::HTUI_FEAT_3,
            run,
            &["plan".to_owned(), "review".to_owned()],
        )
        .await
        .expect("MemStore never fails a read");
    assert!(
        inputs
            .iter()
            .any(|input| input.kind == "review" && input.document.is_some()),
        "§4.4's carried review arrives through §4.2's resolver, not through a side channel"
    );

    // Approve the second implement; the second review parks; reject it and the budget is out.
    approve(&orch, run, 1).await;
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        at(&steps, 3, 2).status,
        StepStatus::AwaitingApproval,
        "the review re-ran at its own attempt"
    );

    let (_, rest) = answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "still no tests".to_owned(),
        },
    )
    .await;
    assert_eq!(
        rest.failure,
        Some(RunFailure::ReviewLoopExhausted(2)),
        "`retry_limit = 1` on `implement` permits two attempts and no more"
    );

    let row = run_of(&orch, run).await;
    assert_eq!(
        row.status,
        RunStatus::AwaitingApproval,
        "escalate is not terminate"
    );
    assert!(
        row.failure.is_none(),
        "blueprint A-4: `run.failure` stays NULL on a parked run, and `Rest.failure` is the only \
         place the reason is readable"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| {
            body.contains("review loop exhausted after 2 attempts")
                && body.contains("phase `implement`")
                && body.contains("attempt 2")
                && body.contains("stop reason `exhausted`")
        }),
        "criterion 6's escalation note, verbatim: {notes:?}"
    );
    assert!(
        !steps_of(&orch, run)
            .await
            .iter()
            .any(|step| step.attempt > 2),
        "the loop stopped rather than creating a third attempt"
    );
}

/// ANA-2 §12 criterion 7 (`docs/ANA-2.md:2103`): two identical `after_hash` values stop the loop
/// *before* its retry budget says so.
///
/// `retry_limit` is raised to 3 on `implement`, so `may_attempt(3, 3)` would have permitted a third
/// attempt: if the loop stops after two, the predicate is what stopped it and not the budget. The
/// hash half of the predicate answers false on an empty scope, so the case needs a primary repo —
/// which is also plan D14's happy path.
async fn identical_after_hash_stops_the_loop<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let repo = primary_repo(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "implement" {
            phase.retry_limit = 3;
        }
    })
    .await;
    // Every capture reports the same hash; the two the predicate reads are the two implements'.
    for _ in 0..16 {
        orch.isolator().script_after(Some("same"));
    }

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(run_of(&orch, run).await.repo_scope, vec![repo]);

    approve(&orch, run, 3).await; // prd, plan, implement 1
    answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "attempt 1 is no better".to_owned(),
        },
    )
    .await; // review 1 -> the loop resumes at implement 2
    approve(&orch, run, 1).await; // implement 2
    let (_, rest) = answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "attempt 2 is no better".to_owned(),
        },
    )
    .await; // review 2 -> identical `after_hash`, so the loop stops

    assert_eq!(rest.failure, Some(RunFailure::ReviewLoopExhausted(2)));
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| {
            body.contains("review loop exhausted after 2 attempts")
                && body.contains("stop reason `no_progress_hash`")
        }),
        "the budget permitted a third attempt; the predicate is what stopped it: {notes:?}"
    );
    assert!(
        !steps_of(&orch, run)
            .await
            .iter()
            .any(|step| step.attempt == 3),
        "no third implement attempt was created, though `retry_limit = 3` would have allowed one"
    );
}

/// `docs/ANA-2.md:412-415`: a required input that resolves to nothing fails the run before a token
/// is spent, and the step's empty `prompt_digest` is what proves no prompt was ever assembled.
///
/// The kind is `spec` because nothing in the `feature` graph produces one: §5.3's back-edge rule
/// excuses an input a *later* position outputs (that is how the seeded `implement`'s `review` input
/// is legal on attempt 1, blueprint H-8), and a kind no position outputs cannot be excused by it.
async fn missing_input_fails_before_a_token<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Failed);
    assert_eq!(rest.position, Some(0));
    assert_eq!(
        rest.failure,
        Some(RunFailure::MissingInput("spec".to_owned()))
    );

    let row = run_of(&orch, run).await;
    assert_eq!(row.failure.as_deref(), Some("missing input document: spec"));
    assert!(row.finished_at.is_some(), "a failed run is a finished run");
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Failed
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1, "stage 3 fails the one step it reached");
    assert_eq!(steps[0].status, StepStatus::Failed);
    assert!(
        steps[0].prompt_digest.is_none(),
        "no prompt was assembled, so no token was spent"
    );
    assert!(steps[0].usage.is_none(), "and no session was opened");
}

/// `docs/ANA-2.md:429-430`: a step that produced no document of its `output_kind` settles `failed`,
/// and with the gate out of the way the walk spends the retry budget and then fails the run.
///
/// The gate has to be `never` for the retry cell to be reachable at all: under `always` the failed
/// step parks and waits for a human, which is `approve_needs_the_output_document`'s case. Two
/// attempts against `retry_limit = 1` is plan D3's count — the limit is the number of *additional*
/// attempts.
async fn missing_output_settles_failed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    orch.script("prd", 1, ScriptedStep::done_without_output());
    orch.script("prd", 2, ScriptedStep::done_without_output());

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Failed);
    assert_eq!(rest.failure, Some(RunFailure::MissingOutput));
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some("missing_output")
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Failed
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [(0, 1, StepStatus::Failed), (0, 2, StepStatus::Failed)],
        "plan D3: `retry_limit = 1` permits two attempts and no more"
    );
    assert!(
        orch.store()
            .documents(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .iter()
            .all(|head| head.kind != "prd"),
        "neither attempt produced the `prd` the phase names"
    );
}

/// `docs/ANA-2.md:476-482`: a gated phase whose only candidate cannot answer a permission request
/// is refused, the item is `blocked` **and not** `failed`, and the reason is a note a human reads.
///
/// The interlock reads `registry::caps_for` from the *agent row* and not from the driver (plan D6),
/// so the candidates map is the knob: `AGENT_CLAUDE_CLI` has `permission_requests: false` and
/// `edit_proposals: false` (`crates/htui-agent/src/registry.rs:178-181`). The refusal lands at
/// stage 1, **before any step row exists** — which is the assertion that separates it from a step
/// that ran and then failed.
async fn cli_agent_is_refused_at_a_gated_phase<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.with_candidates("prd", vec![(ids::AGENT_CLAUDE_CLI, "default")]);

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Failed);
    assert_eq!(rest.failure, Some(RunFailure::MissingCapability));
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some("missing_capability: inline_approval")
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked,
        "the item is blocked before the run is failed (blueprint H-16): the reverse order would \
         leave it `failed`, and `blocked` is what a human can act on"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body.contains("missing_capability: inline_approval")),
        "ANA-2 invariant 7: a refusal a human can read: {notes:?}"
    );
    assert!(
        steps_of(&orch, run).await.is_empty(),
        "stage 1 refuses before a step row exists, so no token was spent and none can be"
    );
}

/// `docs/ANA-2.md:439` and plan D8: a step that outlives its deadline settles `failed`, with no
/// sleep anywhere, and the reason is readable on the item.
///
/// `set_app_setting("step_deadline_seconds", 1)` is the only rung a case can reach: `SettingKey` is
/// a closed enum of ten that does not carry the key and `ProjectPatch` has no `settings` field, so
/// the app rung is the one that can be planted. The elapse happens between the driver's `done` and
/// the settle, which is exactly where a real overrun would be noticed.
///
/// The reason is asserted on the **item's notes** and not on `gate_note`: plan D10 as T4 corrected
/// it writes every settle reason as an item note with `via_step_id`, because `answer_gate` is the
/// only writer of `gate_note` and it never runs on a step the engine itself failed.
async fn step_deadline_settles_failed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.store()
        .set_app_setting("step_deadline_seconds", serde_json::json!(1));
    orch.advance_after_done(TimeDelta::seconds(5));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "`always` parks the failed step rather than failing the run"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, StepStatus::AwaitingApproval);
    assert_eq!(
        steps[0].gate_note, None,
        "plan D10 as T4 corrected it: a parked step carries no note of its own"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body.contains("deadline elapsed")),
        "the settle's reason is readable on the item: {notes:?}"
    );
}

/// Plan D7's composite writer, read from both ends of the walk.
///
/// `finish_run` moves the run *and* its item in one transaction, and this case is the only one that
/// asserts both of its directions. The prologue is the first: `finish_run(RUN_2, Cancelled)` is what
/// frees `FEAT-3`, and the item it moves `queued -> open` is the proof the mirror is not
/// terminal-only. Criterion 1's tail is the second: one `finish_run` at the last approve stamps
/// `run.finished_at` and `item.closed_at` with the same instant, and no separate `transition_run`
/// is needed to get there.
async fn finish_run_is_the_last_write<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Queued,
        "the fixture seeds `FEAT-3` queued behind the live `RUN_2`"
    );
    free_feat_3(&orch).await;
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "plan D7: cancelling the run moved the item with it, in `finish_run`'s own transaction"
    );
    assert_eq!(run_of(&orch, ids::RUN_2).await.status, RunStatus::Cancelled);

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve(&orch, run, 4).await;

    let finished = orch.clock().now();
    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::Done);
    assert_eq!(
        row.finished_at,
        Some(finished),
        "the last write of the walk is `finish_run`, at the last approve's instant"
    );
    assert_eq!(row.failure, None, "a `done` run names no failure");

    let item = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(item.status, Status::Done, "one write moved both rows");
    assert!(item.closed_at.is_some(), "and the item closed with the run");
    // `closed_at` is deliberately *not* asserted equal to `finished`: `finish_run` takes the run's
    // instant as an argument and stamps the item's mirror from `Utc::now()`
    // (`crates/htui-core/src/store/mem.rs:4466-4469`), so the two are one transaction but not one
    // instant. Pinning them equal would be pinning the store's clock, not the walk's.
}

#[cfg(test)]
mod tests {
    use super::{CASES, CaseHarness, FakeOrchestrator, run_all, run_case};

    /// The suite's own binding, and the one `tests/fake_conformance.rs` mirrors out of crate (T5).
    struct Demo;

    impl CaseHarness for Demo {
        type Orch = FakeOrchestrator;

        fn fresh(&self) -> FakeOrchestrator {
            FakeOrchestrator::demo()
        }
    }

    /// The list is the suite's API, and its length is a claim a binding is allowed to check.
    #[test]
    fn cases_are_unique_and_eleven() {
        let mut sorted: Vec<&&str> = CASES.iter().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
        assert_eq!(
            CASES.len(),
            11,
            "six ANA-2 §12 criteria, four §4.2 contract lines and the `finish_run` seam"
        );
    }

    /// `run_case` panics on a name it does not know, which is the whole point of its `match`:
    /// running the list through it is what keeps [`CASES`] and the dispatcher in step. It is also
    /// what makes T5's work mechanical — replacing a stub body cannot silently orphan its name.
    #[tokio::test]
    async fn every_case_name_dispatches() {
        for name in CASES {
            run_case(name, &Demo).await;
        }
    }

    /// And the loop a binding actually calls.
    #[tokio::test]
    async fn run_all_walks_the_list() {
        run_all(&Demo).await;
    }
}
