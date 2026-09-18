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
use htui_core::model::AgentId;
use htui_core::store::MemStore;

use crate::command::{Command, CommandOutcome, EngineError};
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

/// ANA-2 §12 criterion 1 (`docs/ANA-2.md:2085`). **Stub**: T5 lands the body (blueprint §6.3).
async fn feat_walks_end_to_end<H: CaseHarness>(harness: &H) {
    fresh_harness_is_usable(harness, "feat_walks_end_to_end").await;
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
