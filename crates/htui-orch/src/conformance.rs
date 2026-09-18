//! The walk's transport-neutral conformance suite (plan D18), behind `test-support`.
//!
//! The shape is the one both existing suites already use
//! (`crates/htui-agent/src/conformance.rs:146-238`,
//! `crates/htui-core/src/store/conformance.rs:36-239`): a `CASES` list that is the suite's API, a
//! `run_case` dispatcher whose `match` panics on a name it does not know, a `run_all` that walks
//! the list, and a unit test that runs every name through the dispatcher so the two cannot drift.
//!
//! T3 lands the list, the harness traits and the case stubs; T5 replaces each stub body with the
//! validation criterion it is named for.

use chrono::TimeDelta;
use htui_agent::conformance::epoch;
use htui_core::fixtures::ids;
use htui_core::model::{
    AgentId, GateOutcome, Item, ItemId, Run, RunId, RunMode, RunStatus, RunStep, Status, StepStatus,
};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

use crate::command::{Command, CommandOutcome, EngineError, GateAnswer};
use crate::engine::Resume;
use crate::fake::{FakeIsolator, FakeOrchestrator, ScriptedStep, TestClock};
use crate::isolate::Clock as _;

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

/// What every case body is until T5 replaces it, one at a time.
///
/// It asserts the one thing T3 can honestly assert — that the harness hands out a usable
/// orchestrator — and it touches every knob [`Orchestrate`] exposes, so a binding whose `fresh()`
/// returns something half-built fails here rather than three cases later inside a criterion. It
/// deliberately does **not** call `dispatch`: `engine.rs` is T4's, and a stub that pretended
/// otherwise would be a green that means nothing.
///
/// # Panics
/// When the harness is not usable, which is the whole content of the stub.
async fn fresh_harness_is_usable<H: CaseHarness>(harness: &H, case: &str) {
    let orch = harness.fresh();
    assert_eq!(
        orch.clock().now(),
        epoch(),
        "`{case}`: a fresh clock starts at the fake driver's origin"
    );
    assert!(
        orch.store().item_count() > 0,
        "`{case}`: a fresh harness holds a seeded store"
    );
    orch.script(case, 1, ScriptedStep::done_with_output(case));
    orch.with_candidates(case, vec![(ids::AGENT_CLAUDE, "sonnet")]);
    orch.advance_after_done(TimeDelta::zero());
    orch.isolator().script_after(None);
    assert_eq!(
        orch.clock().now(),
        epoch(),
        "`{case}`: scripting a harness moves no clock"
    );
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

/// `StartRun` on `item` in manual mode with no requested scope, unwrapped to its run id and rest.
///
/// # Panics
/// When the command is refused, which every caller of this helper expects not to be.
async fn start<O: Orchestrate>(orch: &O, item: ItemId) -> (RunId, crate::command::Rest) {
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
async fn answer<O: Orchestrate>(orch: &O, run: RunId, answer: GateAnswer) -> RunStep {
    let steps = steps_of(orch, run).await;
    let parked = steps
        .iter()
        .find(|step| step.status == StepStatus::AwaitingApproval)
        .expect("the walk parked exactly one step")
        .clone();
    orch.dispatch(Command::AnswerGate {
        run,
        step: parked.id,
        answer,
    })
    .await
    .expect("the parked gate is answerable");
    parked
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
        let parked = answer(&orch, run, GateAnswer::Approved).await;
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

/// Criterion 2 (`:2088`). **Stub**: T5 lands the body.
async fn live_run_ignores_a_gate_edit<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "live_run_ignores_a_gate_edit").await;
}

/// Criterion 3 (`:2090`). **Stub**: T5 lands the body, and with it `Engine::resume`.
async fn topology_mismatch_parks_on_resume<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "topology_mismatch_parks_on_resume").await;
}

/// Criterion 5 (`:2096`). **Stub**: T5 lands the body.
async fn approve_needs_the_output_document<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "approve_needs_the_output_document").await;
}

/// Criterion 6 (`:2098`). **Stub**: T5 lands the body.
async fn review_rejection_loops_then_escalates<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "review_rejection_loops_then_escalates").await;
}

/// Criterion 7 (`:2103`). **Stub**: T5 lands the body.
async fn identical_after_hash_stops_the_loop<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "identical_after_hash_stops_the_loop").await;
}

/// `docs/ANA-2.md:412-415`. **Stub**: T5 lands the body.
async fn missing_input_fails_before_a_token<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "missing_input_fails_before_a_token").await;
}

/// `docs/ANA-2.md:429-430`. **Stub**: T5 lands the body.
async fn missing_output_settles_failed<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "missing_output_settles_failed").await;
}

/// `docs/ANA-2.md:476-482`. **Stub**: T5 lands the body.
async fn cli_agent_is_refused_at_a_gated_phase<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "cli_agent_is_refused_at_a_gated_phase").await;
}

/// `docs/ANA-2.md:439` and plan D8. **Stub**: T5 lands the body.
async fn step_deadline_settles_failed<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "step_deadline_settles_failed").await;
}

/// Plan D7's composite writer, read from the walk's end. **Stub**: T5 lands the body.
async fn finish_run_is_the_last_write<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "finish_run_is_the_last_write").await;
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
