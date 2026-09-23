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
    AgentBox, AgentId, Billing, CommandRun, CommandRunStatus, EventKind, Gate, GateOutcome,
    Isolation, Item, ItemId, ItemPatch, NewRepo, NewStepGraph, PhaseId, PhasePatch, RepoId, Run,
    RunId, RunMode, RunStatus, RunStep, Status, StepGraphId, StepGraphPhase, StepId, StepStatus,
    VerifyOutcome,
};
use htui_core::model::{Quota, QuotaSource, Spend};
use htui_core::prompt::DiffBlock;
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

use crate::command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
use crate::engine::Resume;
use crate::fake::{FakeIsolator, FakeOrchestrator, FakeVerifier, ScriptedStep, TestClock};
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

    /// Script one session exactly (plan D68): candidate `fanout_index` of `(phase, attempt)`, or
    /// a judge's ordering `call` at `fanout_index = -1` with `phase` = `<phase>:judge`. Wins over
    /// [`script`](Orchestrate::script) for the same `(phase, attempt)`.
    fn script_candidate(
        &self,
        phase: &str,
        attempt: i32,
        fanout_index: i32,
        call: u32,
        step: ScriptedStep,
    );

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

    /// The verifier, for scripting stage 5's `verify_command` outcome.
    ///
    /// Out of the blueprint's §4.5 list for the same reason [`isolator`](Orchestrate::isolator)
    /// and [`clock`](Orchestrate::clock) are (blueprint F-P): a case that pins what a `pass`, a
    /// `fail` or an `unavailable` does to the settle has to be able to say which one happened,
    /// and nothing else on this trait can.
    fn verifier(&self) -> &FakeVerifier;

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

    fn script_candidate(
        &self,
        phase: &str,
        attempt: i32,
        fanout_index: i32,
        call: u32,
        step: ScriptedStep,
    ) {
        Self::script_candidate(self, phase, attempt, fanout_index, call, step);
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

    fn verifier(&self) -> &FakeVerifier {
        &self.verifier
    }

    fn clock(&self) -> &TestClock {
        &self.clock
    }
}

/// Case names in run order. A name never changes: every binding reports per case.
///
/// Thirty-six, and the count is pinned in two places on purpose — here by
/// `cases_are_unique_and_thirty_six` and out of crate by `tests/fake_conformance.rs` — because
/// a binding that silently ran thirty-five of them would still be green.
///
/// Seven are `docs/ANA-2.md` §12's validation criteria (1, 2, 3, 5, 6, 7 and 13's command half);
/// four are contract lines
/// §12 does not number but §4.2 states outright; one is the `finish_run` seam T1 shipped for this
/// walk to use; three are the cells of §4.2's gate table nothing else reaches — plan D4's automatic
/// loop entry and both halves of `on_failure` — which D4 predicted would go unexecuted in a
/// manual-mode milestone and did; one is plan D5's intermediate position, which had a topology
/// digest pinning it and no walk executing it; the last three are milestone 3's — the settle's two
/// readings of a `verify_command` that ran (`fail`) and one that could not (`unavailable`), and
/// `CancelRun`, which criterion 13 is stated in terms of (plan D45). Milestone 4 adds five: stage 1
/// as `R-AGT-8`'s walk selecting an `allowed_warning` row and falling through a skipped one (plan
/// D60, D61), both halves of the empty-candidate refusal (plan D62), and the loop's forwarded
/// `verify_failure` and `previous_diff` (plan D67) — and then twelve for fan-out: criteria 8, 9
/// and 10 (plan D49-D53, D65), the gated and judgeless routes to a human (D49, D50), a failed
/// candidate (D48), a group with no survivor (D49(1)), the review loop over a group (D66), and
/// the two caps refused at `StartRun` (D63) — and one more for a `shared_serialized` sibling's
/// deadline, which counts from its own `prepare` (D48).
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
    // Plan D4's *other* entry point (`:450`, the `never` × `rejected` cell): the walk itself
    // rejects a review and runs the loop, with no human in it.
    "a_never_gate_rejection_loops_the_review",
    // The same entry point out of budget: `:746`'s escalation, reached without a human.
    "a_never_gate_rejection_escalates_when_the_budget_is_out",
    // `:449`, both halves: `on_failure` passes a settle `ok` and parks a settle `failed`.
    "on_failure_passes_ok_and_parks_failed",
    // Plan D5's intermediate position (`:721-722`): a phase between `implement` and `review` is
    // retired with the chain and re-run at its own `attempt + 1`.
    "an_intermediate_position_is_retired_by_the_loop",
    // `:437-443` and plan D30: a `verify_command` that exits non-zero settles the step `failed`,
    // and the `command_run` row it is recorded in is `done` — the command ran.
    "verify_fail_settles_failed",
    // `:443`: `unavailable` never fails a step, and its `command_run` row is `failed` — the
    // command did not run. Both facts are one report and they disagree on purpose.
    "verify_unavailable_never_fails",
    // Criterion 13 (`:2120`) as far as the fake reaches it (the trees are `tests/gix_isolator.rs`'s
    // half): cancelling a run cancels every live step and cleans up exactly once.
    "cancel_cleans_up_once",
    // Milestone 4, plan D60/D61: stage 1 is `R-AGT-8`'s walk, and PRD D3's `allowed_warning` is a
    // quota status it selects.
    "allowed_warning_candidate_is_selected",
    // Plan D60: a skipped candidate falls through to the next, and the substitution is a note.
    "a_skipped_candidate_falls_through_to_the_next",
    // Plan D62, rung 4 at `StartRun`: no run row, the item `blocked`, a note naming the phase.
    "no_candidate_agent_blocks_the_item",
    // Plan D62, stage 1: a walk that skips every candidate fails the run `no_candidate_agent`.
    "every_candidate_skipped_refuses_the_run",
    // Plan D67: a second attempt's prompt carries `verify_failure` and `previous_diff`.
    "a_second_attempt_carries_verify_failure_and_previous_diff",
    // Criterion 8 (`:2106`): three candidates and a judge, one winner, the losers superseded.
    "fan_out_three_with_a_judge_selects_one_winner",
    // Criterion 9 (`:2109`): a verify failure is eliminated before the judge when two pass.
    "a_failing_verify_is_eliminated_before_the_judge",
    // Criterion 9 (`:2110`): exactly one passing candidate wins with no judge step.
    "one_passing_candidate_wins_without_a_judge",
    // Criterion 10 (`:2112`): the judge's two orderings disagree, and the group parks.
    "judge_orderings_that_disagree_park_for_selection",
    // Criterion 10 (`:2113`): an unparseable verdict parks, and `SelectFanout` completes it.
    "an_unparseable_verdict_parks_and_select_fanout_completes",
    // Plan D49(2): an `always` group parks for a human with no judge.
    "a_gated_fan_out_parks_for_human_selection",
    // Plan D49(3): no judge configured means a human selects.
    "no_judge_means_human_selection",
    // Plan D48: a failed candidate fails alone.
    "a_failed_candidate_does_not_fail_its_siblings",
    // Plan D49(1): a `never` group with no survivor retries whole, then fails the run.
    "a_group_with_no_survivor_retries_then_fails",
    // Plan D66: the review loop retires a fanned-out slot whole and judges the next group.
    "the_review_loop_reruns_a_fanned_out_implement",
    // Plan D63: `max_fan_out` is refused at `StartRun`.
    "fan_out_above_max_fan_out_is_refused_at_start",
    // Plan D63 (OQ-1): `max_agents_per_run` is refused at `StartRun`.
    "max_agents_per_run_is_refused_at_start",
    // Plan D48 under `shared_serialized`: a candidate's deadline counts from its own `prepare`,
    // not from the siblings it queued behind.
    "a_serialized_sibling_is_not_charged_for_the_ones_before_it",
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
        "a_never_gate_rejection_loops_the_review" => {
            a_never_gate_rejection_loops_the_review(harness).await;
        }
        "a_never_gate_rejection_escalates_when_the_budget_is_out" => {
            a_never_gate_rejection_escalates_when_the_budget_is_out(harness).await;
        }
        "on_failure_passes_ok_and_parks_failed" => {
            on_failure_passes_ok_and_parks_failed(harness).await;
        }
        "an_intermediate_position_is_retired_by_the_loop" => {
            an_intermediate_position_is_retired_by_the_loop(harness).await;
        }
        "verify_fail_settles_failed" => verify_fail_settles_failed(harness).await,
        "verify_unavailable_never_fails" => verify_unavailable_never_fails(harness).await,
        "cancel_cleans_up_once" => cancel_cleans_up_once(harness).await,
        "allowed_warning_candidate_is_selected" => {
            allowed_warning_candidate_is_selected(harness).await;
        }
        "a_skipped_candidate_falls_through_to_the_next" => {
            a_skipped_candidate_falls_through_to_the_next(harness).await;
        }
        "no_candidate_agent_blocks_the_item" => no_candidate_agent_blocks_the_item(harness).await,
        "every_candidate_skipped_refuses_the_run" => {
            every_candidate_skipped_refuses_the_run(harness).await;
        }
        "a_second_attempt_carries_verify_failure_and_previous_diff" => {
            a_second_attempt_carries_verify_failure_and_previous_diff(harness).await;
        }
        "fan_out_three_with_a_judge_selects_one_winner" => {
            fan_out_three_with_a_judge_selects_one_winner(harness).await;
        }
        "a_failing_verify_is_eliminated_before_the_judge" => {
            a_failing_verify_is_eliminated_before_the_judge(harness).await;
        }
        "one_passing_candidate_wins_without_a_judge" => {
            one_passing_candidate_wins_without_a_judge(harness).await;
        }
        "judge_orderings_that_disagree_park_for_selection" => {
            judge_orderings_that_disagree_park_for_selection(harness).await;
        }
        "an_unparseable_verdict_parks_and_select_fanout_completes" => {
            an_unparseable_verdict_parks_and_select_fanout_completes(harness).await;
        }
        "a_gated_fan_out_parks_for_human_selection" => {
            a_gated_fan_out_parks_for_human_selection(harness).await;
        }
        "no_judge_means_human_selection" => no_judge_means_human_selection(harness).await,
        "a_failed_candidate_does_not_fail_its_siblings" => {
            a_failed_candidate_does_not_fail_its_siblings(harness).await;
        }
        "a_group_with_no_survivor_retries_then_fails" => {
            a_group_with_no_survivor_retries_then_fails(harness).await;
        }
        "the_review_loop_reruns_a_fanned_out_implement" => {
            the_review_loop_reruns_a_fanned_out_implement(harness).await;
        }
        "fan_out_above_max_fan_out_is_refused_at_start" => {
            fan_out_above_max_fan_out_is_refused_at_start(harness).await;
        }
        "max_agents_per_run_is_refused_at_start" => {
            max_agents_per_run_is_refused_at_start(harness).await;
        }
        "a_serialized_sibling_is_not_charged_for_the_ones_before_it" => {
            a_serialized_sibling_is_not_charged_for_the_ones_before_it(harness).await;
        }
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

/// Repoints `item` at a clone of its graph carrying one **extra position between `implement` and
/// `review`**, shifting every later phase up by one.
///
/// The shape is `tests/fixtures/feature-with-verify.snapshot.json`'s, which plan D5's risk row calls
/// for ("a fixture graph with an intermediate position is one of T5's recorded graphs") and which
/// nothing walked: a `verify` phase cloned off `implement`, its `template_name` left at `review`
/// because the seed ships no `verify` template, and its `output_kind` its own name so the document
/// the stand-in producer writes is distinguishable from `implement`'s.
///
/// Separate from [`repoint`] rather than a parameter of it: `repoint` edits a phase in place and
/// every one of its five callers wants exactly that, while this one changes the *shape* of the
/// graph and has to renumber around the insertion.
///
/// # Panics
/// When any of the writes is refused, which means the fixture moved under the case.
async fn insert_verify_phase<O: Orchestrate>(orch: &O, item: ItemId) {
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
            name: format!("{}-with-verify", row.key),
            description: "an intermediate position between implement and review".to_owned(),
        })
        .await
        .expect("the name is fresh");
    let implement = graph
        .phases
        .iter()
        .find(|phase| phase.phase.name == "implement")
        .expect("the `feature` graph names an `implement` phase")
        .phase
        .position;
    for phase in &graph.phases {
        let mut edited = StepGraphPhase {
            id: PhaseId::new(),
            graph_id: clone.id,
            ..phase.phase.clone()
        };
        if edited.position > implement {
            edited.position += 1;
        }
        orch.store()
            .create_phase(&edited)
            .await
            .expect("the clone accepts its phases");
        if edited.name == "implement" {
            orch.store()
                .create_phase(&StepGraphPhase {
                    id: PhaseId::new(),
                    position: implement + 1,
                    name: "verify".to_owned(),
                    output_kind: "verify".to_owned(),
                    template_name: "review".to_owned(),
                    input_kinds: vec!["implement".to_owned()],
                    ..edited.clone()
                })
                .await
                .expect("the inserted position is free");
        }
    }
    orch.store()
        .update_item(
            item,
            row.version,
            ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: row.created_by,
                reason: "a conformance case's intermediate position".to_owned(),
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

/// Answers the run's one parked step `approved` and unparks the run **without walking it**, leaving
/// the run `running` with its next position uncreated.
///
/// The three writes are `Engine::answer_gate`'s own, minus its closing `run_to_rest`
/// (`crates/htui-orch/src/engine.rs`): `answer_gate`, then the run, then the item, in that order.
///
/// It exists for criterion 3. `run_to_rest` returns at its `AwaitingApproval` arm before it reads a
/// cursor, so "resume advanced nothing" asserted on a *parked* run holds however the resume behaves
/// — including with the topology guard deleted. A run that is `running` with a position to create is
/// the only state in which the assertion is about the guard.
///
/// # Panics
/// When no step is parked, or when any of the three writes is refused.
async fn approve_without_walking<O: Orchestrate>(orch: &O, run: RunId, item: ItemId) {
    let now = orch.clock().now();
    let steps = steps_of(orch, run).await;
    let parked = steps
        .iter()
        .find(|step| step.status == StepStatus::AwaitingApproval)
        .expect("the walk parked exactly one step");
    assert!(
        orch.store()
            .answer_gate(parked.id, GateOutcome::Approved, None, now)
            .await
            .expect("the step is awaiting"),
        "the compare-and-set found the step where the walk left it"
    );
    orch.store()
        .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
        .await
        .expect("the run is parked");
    orch.store()
        .transition(item, Status::AwaitingApproval, Status::InProgress)
        .await
        .expect("the item is parked");
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

/// The step's `command_run` rows, in `(queued_at, id)` order (T1's `WriteStore::command_runs`).
///
/// # Panics
/// Never: `MemStore` fails no read.
async fn command_runs_of<O: Orchestrate>(orch: &O, step: StepId) -> Vec<CommandRun> {
    orch.store()
        .command_runs(step)
        .await
        .expect("MemStore never fails a read")
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
/// unedited run resumes into the walk and creates the next position's step.
///
/// **The run is left `running` with a position to create, and that is the load-bearing part.** A
/// walk on a parked run returns at `run_to_rest`'s `AwaitingApproval` arm before it ever reads a
/// cursor, so with the run parked the step rows are equal before and after however `resume` behaves
/// — with the topology guard and without it. Approving the first gate through
/// [`approve_without_walking`] puts the run in the one state where "nothing advanced" is a claim
/// about the guard: position 0 is `done`, position 1 has no row, and a walk would create one.
async fn topology_mismatch_parks_on_resume<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    assert_eq!(
        run_of(&orch, run).await.status,
        RunStatus::Running,
        "the run is mid-walk: `resume` is the next thing that would advance it"
    );

    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;

    let before = steps_of(&orch, run).await;
    assert_eq!(
        before
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [(0, 1, StepStatus::Done)],
        "one answered step, and position 1 owed"
    );
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
        (rest.run, rest.position),
        (RunStatus::Running, Some(1)),
        "the run is where it was, owing the position it owed: a mismatch is a human's decision, \
         not the engine's"
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

    // An unedited run resumes into the walk instead — and the walk does something.
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("an untouched graph still hashes to the run's own topology: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the walk ran position 1 and parked at its gate"
    );
    assert_eq!(
        steps_of(&orch, run)
            .await
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
        ],
        "which is the row the edited half proves the guard withheld"
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

/// Plan D4's **automatic** entry point (`docs/ANA-2.md:450`, the `never` × `rejected` cell): the
/// walk reads `verdict: request-changes` off the review it just ran and loops with no human in it.
///
/// D4 predicted this path would be the unreached one — "the seeded `review` phase's gate is
/// `always`, so in manual mode the automatic path is the unreached one" — and every other case that
/// loops goes through `AnswerGate { Rejected }` instead. Repointing `review` to `never` is what
/// makes the cell reachable, and the shape asserted is the same retirement criterion 6 asserts for
/// the human path: identical rows, which is D4's "one routine, two callers" read back from the
/// store.
///
/// The two review attempts are scripted apart — `request-changes` then `approve` — so the loop is
/// entered once and then left by the review passing, which is the branch that ends in a `done` run.
async fn a_never_gate_rejection_loops_the_review<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "review" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    orch.script("review", 2, ScriptedStep::review("approve", "second"));

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;

    // prd, plan, implement 1. The walk then runs `review` itself, settles it `rejected`, and comes
    // back round to park the implement the loop re-created — no human answered the review.
    let rest = approve(&orch, run, 3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(2), None),
        "the loop resumed at `implement` and the `always` gate there parked it"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::AwaitingApproval),
            (3, 1, StepStatus::Cancelled),
        ],
        "plan D5's retirement, reached without a human: the implement is superseded and the \
         rejecting review is cancelled, because `failed -> superseded` is illegal"
    );
    assert_eq!(
        at(&steps, 3, 1).gate_outcome,
        Some(GateOutcome::Rejected),
        "the automatic path parks the review first precisely so `answer_gate` can record this: \
         nothing else writes `gate_outcome = 'rejected'` with its note"
    );
    assert_eq!(
        at(&steps, 3, 1).gate_note.as_deref(),
        Some("verdict: request-changes"),
        "the note is the review's own verdict line (`docs/ANA-2.md:748`), not a human's sentence"
    );

    // Approve the re-run implement; the second review approves itself and the run finishes.
    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None),
        "the loop terminates: a `never` review that approves walks the run straight to `done`"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        at(&steps, 3, 2).status,
        StepStatus::Done,
        "the second review ran at its own attempt and passed"
    );
    assert_eq!(
        at(&steps, 3, 2).gate_outcome,
        None,
        "`never` passes a step `running -> done` and leaves `gate_outcome` NULL (blueprint H-9)"
    );
    assert_eq!(item_of(&orch, ids::HTUI_FEAT_3).await.status, Status::Done);
}

/// The automatic entry point out of budget: `docs/ANA-2.md:746`'s escalation, reached with no human
/// in the loop.
///
/// The twin of `review_rejection_loops_then_escalates`, which drives the same escalation through
/// `AnswerGate { Rejected }`. Both review attempts reject and their bodies differ, so §4.4's
/// no-progress predicate answers false on both halves and `Exhausted` — `retry_limit = 1` on
/// `implement`, which permits two attempts — is the only thing left that can stop the loop. That is
/// what makes the `after N attempts` count in the note mean the budget and not the predicate.
async fn a_never_gate_rejection_escalates_when_the_budget_is_out<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "review" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    orch.script(
        "review",
        2,
        ScriptedStep::review("request-changes", "second"),
    );

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve(&orch, run, 3).await; // prd, plan, implement 1 -> review 1 rejects -> implement 2 parks
    let rest = approve(&orch, run, 1).await; // implement 2 -> review 2 rejects, and the budget is out

    assert_eq!(
        rest.failure,
        Some(RunFailure::ReviewLoopExhausted(2)),
        "`retry_limit = 1` on `implement` permits two attempts and no more"
    );
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(3)),
        "escalate is not terminate: the run parks at the review that rejected"
    );

    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::AwaitingApproval);
    assert!(
        row.failure.is_none(),
        "blueprint A-4: `run.failure` stays NULL on a parked run"
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
                && body.contains("stop reason `exhausted`")
        }),
        "`docs/ANA-2.md:746`'s wording, verbatim, on the automatic path too: {notes:?}"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        at(&steps, 3, 2).status,
        StepStatus::Failed,
        "`answer_gate(Rejected)` lands the review `failed`, and the loop stopped before retiring it"
    );
    assert_eq!(
        at(&steps, 3, 2).gate_outcome,
        Some(GateOutcome::Rejected),
        "the verdict is on the row a human is about to read"
    );
    assert!(
        !steps.iter().any(|step| step.attempt > 2),
        "the loop stopped rather than creating a third attempt"
    );
}

/// `docs/ANA-2.md:449`, both halves: `on_failure` passes a settle `ok` through and parks a settle
/// `failed`.
///
/// The only gate value no case reached. It is the one row of §4.2's table that *depends* on the
/// settle outcome without looping — `always` parks on all three and `never` parks on none — so a
/// single half would not pin it: a gate that always passed and a gate that always parked would each
/// satisfy one of the two assertions below.
///
/// Both halves use `prd`, which is position 0, so the failing half's park is the run's first stop
/// and nothing earlier can be mistaken for it.
async fn on_failure_passes_ok_and_parks_failed<H: CaseHarness>(harness: &H) {
    // Half 1: the settle is `ok`, so the gate does not stop and the walk moves on.
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::OnFailure;
        }
    })
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "`prd` did not stop, so the first gate the walk met is `plan`'s `always`"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.phase_name.as_str(), step.status))
            .collect::<Vec<_>>(),
        [
            (0, "prd", StepStatus::Done),
            (1, "plan", StepStatus::AwaitingApproval),
        ]
    );
    assert_eq!(
        at(&steps, 0, 1).gate_outcome,
        None,
        "`gate_outcome = 'skipped'` is not written: `answer_gate` is the only writer of the column \
         and it is `awaiting_approval`-only (blueprint H-9)"
    );

    // Half 2: the same gate, the same phase, a settle that failed — and now it stops.
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::OnFailure;
        }
    })
    .await;
    orch.script("prd", 1, ScriptedStep::done_without_output());

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "a failed settle under `on_failure` parks for a human rather than failing the run"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [(0, 1, StepStatus::AwaitingApproval)],
        "the gate stopped the walk, so no retry was admitted — which is what separates this row \
         from `never`'s, where the same settle spends the budget and then fails the run"
    );
    assert_eq!(
        run_of(&orch, run).await.status,
        RunStatus::AwaitingApproval,
        "the park moves step, run and item (blueprint H-10)"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::AwaitingApproval
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body.contains("missing_output")),
        "ANA-2 invariant 7: the parked step's reason is readable on the item: {notes:?}"
    );
}

/// Plan D5's intermediate position (`docs/ANA-2.md:721-722`), walked rather than described.
///
/// ANA-2 §4.4 step 3 retires the reviewed implement and the rejecting review and says nothing about
/// what sits between them, while step 5 has every position in between "re-run, at the same `attempt`
/// value" — which, taken literally, leaves a `done` step at the attempt the walk is about to
/// re-insert and trips `UNIQUE (run_id, position, attempt, fanout_index)`. D5 answers it with
/// per-position attempts and the whole chain retired; the answer had a topology digest pinning it
/// (`tests/fixtures.rs`) and no walk executing it, so neither the `done -> superseded` of an
/// intermediate position nor the collision it avoids was ever run.
///
/// The graph is `feature` with a `verify` phase at position 3, so `review` is at 4 and the chain
/// `retire` covers is three positions rather than two. The shape asserted is the one D5 prescribes:
/// `(2,1)` and `(3,1)` superseded, `(4,1)` cancelled, then `(2,2)`, `(3,2)` and `(4,2)` — every
/// position at *its own* next attempt.
async fn an_intermediate_position_is_retired_by_the_loop<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    insert_verify_phase(&orch, ids::HTUI_FEAT_3).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.position, Some(0));

    // prd, plan, implement, verify; the walk then parks at `review`, which is now position 4.
    approve(&orch, run, 4).await;
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.phase_name.as_str(), step.status))
            .collect::<Vec<_>>(),
        [
            (0, "prd", StepStatus::Done),
            (1, "plan", StepStatus::Done),
            (2, "implement", StepStatus::Done),
            (3, "verify", StepStatus::Done),
            (4, "review", StepStatus::AwaitingApproval),
        ],
        "the inserted position is walked like any other"
    );

    let (_, rest) = answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "no tests".to_owned(),
        },
    )
    .await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(2), None),
        "the loop resumed at `implement`, three positions back"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::AwaitingApproval),
            (3, 1, StepStatus::Superseded),
            (4, 1, StepStatus::Cancelled),
        ],
        "the whole chain retires: the intermediate `verify` is superseded with the implement it \
         depends on, and only the rejecting review — which is `failed` — takes the cancel"
    );

    // implement 2, verify 2; then the second review parks and approving it finishes the run.
    approve(&orch, run, 2).await;
    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.attempt, step.status))
            .collect::<Vec<_>>(),
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::Done),
            (3, 1, StepStatus::Superseded),
            (3, 2, StepStatus::Done),
            (4, 1, StepStatus::Cancelled),
            (4, 2, StepStatus::Done),
        ],
        "each re-run position takes its own `attempt + 1`, which is what keeps \
         `UNIQUE (run_id, position, attempt, fanout_index)` out of the way (plan D5)"
    );
    assert_eq!(item_of(&orch, ids::HTUI_FEAT_3).await.status, Status::Done);
}

/// ANA-2 `:437-443` through plan D30's verifier: a `verify_command` that exits non-zero settles
/// the step `failed`, and under a `never` gate with no retry budget that fails the run.
///
/// The two columns and the row are asserted together because they are three different claims: the
/// step's `verify_outcome`/`verify_exit_code` are what §4.2's settle reads, and the `command_run`
/// row is where the output lives for milestone 4's `verify_failure` prompt section (plan D32). The
/// row's `status` is `done` and not `failed` — the command *ran*, and produced a verdict.
async fn verify_fail_settles_failed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
            phase.retry_limit = 0;
            phase.verify_command = Some("cargo test".to_owned());
        }
    })
    .await;
    orch.verifier().script_report(FakeVerifier::fail(1));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::Failed, Some(0)),
        "a `never` gate with no budget ends the run on the failed settle"
    );
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some("verify_outcome: fail"),
        "`StepFailure::VerifyFailed` has no plan D12 word, so its own sentence is the run's"
    );

    let steps = steps_of(&orch, run).await;
    let step = at(&steps, 0, 1);
    assert_eq!(
        (step.status, step.verify_outcome, step.verify_exit_code),
        (StepStatus::Failed, Some(VerifyOutcome::Fail), Some(1)),
        "both columns stop being hard-coded `None` this milestone"
    );

    let rows = command_runs_of(&orch, step.id).await;
    assert_eq!(rows.len(), 1, "one verify, one row: {rows:?}");
    assert_eq!(
        (
            rows[0].class.as_str(),
            rows[0].command.as_str(),
            rows[0].status,
            rows[0].exit_code
        ),
        ("verify", "cargo test", CommandRunStatus::Done, Some(1)),
        "a command that ran and failed is a `done` row (plan D31)"
    );
}

/// ANA-2 `:443`: `unavailable` never fails a step — and its `command_run` row is `failed`, which
/// is the one place the two vocabularies deliberately disagree.
///
/// `prd` is `always`-gated in the seed, so the walk parks at position 0 either way; what this case
/// pins is that the settle read `Ok` and not `Failed` on the way there, and that the reason a
/// human reads is in `command_run.output`.
async fn verify_unavailable_never_fails<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.verify_command = Some("cargo test".to_owned());
        }
    })
    .await;
    orch.verifier()
        .script_report(FakeVerifier::unavailable("no `sh` on PATH"));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "the `always` gate parked; nothing failed"
    );

    let steps = steps_of(&orch, run).await;
    let step = at(&steps, 0, 1);
    assert_eq!(
        (step.status, step.verify_outcome, step.verify_exit_code),
        (
            StepStatus::AwaitingApproval,
            Some(VerifyOutcome::Unavailable),
            None
        ),
        "an `unavailable` verify has no exit code and does not fail the step"
    );

    let rows = command_runs_of(&orch, step.id).await;
    assert_eq!(rows.len(), 1, "one verify, one row: {rows:?}");
    assert_eq!(
        (rows[0].status, rows[0].exit_code, rows[0].output.as_deref()),
        (CommandRunStatus::Failed, None, Some("no `sh` on PATH")),
        "a command that could not run is a `failed` row carrying its reason (plan D31)"
    );
}

/// ANA-2 §12 criterion 13 (`docs/ANA-2.md:2120`) minus its filesystem half: `CancelRun` ends the
/// run, cancels every step that had not settled, frees the item, and cleans up **once**.
///
/// The trees are `tests/gix_isolator.rs`'s to assert — the fake makes none — so what this case
/// pins is the shape of the command: which statuses it is legal from, what it moves, and that the
/// cleanup plan D36 hangs on every terminal `finish_run` hangs on this one too.
async fn cancel_cleans_up_once<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "`prd` is `always`-gated in the seed, so the walk parks there"
    );

    let outcome = orch
        .dispatch(Command::CancelRun { run })
        .await
        .expect("a parked run is cancellable");
    let CommandOutcome::Cancelled { rest } = outcome else {
        panic!("`CancelRun` answers `Cancelled`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Cancelled, Some(0), None),
        "a cancel is a human's decision and not a failure"
    );

    assert_eq!(run_of(&orch, run).await.status, RunStatus::Cancelled);
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.status))
            .collect::<Vec<_>>(),
        [(0, StepStatus::Cancelled)],
        "every step that had not settled goes with the run"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "`finish_run` mirrors the item `awaiting_approval -> open` (plan D7)"
    );
    assert_eq!(
        harness_cleanups(&orch),
        1,
        "plan D45 ends with plan D36's cleanup, once"
    );

    let refused = orch
        .dispatch(Command::CancelRun { run })
        .await
        .expect_err("a cancelled run is not cancellable again");
    assert!(
        matches!(
            &refused,
            EngineError::RunStatus {
                status: RunStatus::Cancelled,
                expected: "queued | running | awaiting_approval",
                ..
            }
        ),
        "{refused}"
    );
    assert_eq!(
        harness_cleanups(&orch),
        1,
        "the refusal wrote nothing, cleanup included"
    );
}

/// The isolator's cleanup count, named once so the case above reads as one claim per line.
fn harness_cleanups<O: Orchestrate>(orch: &O) -> u32 {
    orch.isolator().cleanups()
}

// -- milestone 4: stage 1 is `R-AGT-8`'s walk (plan D60–D62, D67) --------------------------------

/// Latches an ANA-4 §7 quota document with `status` onto `agent`'s `agent_box` row on the fixture
/// box, creating the row first: `set_agent_box_quota` is the column's only writer and refuses a
/// row that is not there (`crates/htui-core/src/store/traits.rs:295-305`).
///
/// # Panics
/// When either write is refused, which means the fixture moved under the case.
async fn latch_quota<O: Orchestrate>(orch: &O, agent: AgentId, status: &str) {
    let now = orch.clock().now();
    orch.store()
        .upsert_agent_box(&AgentBox {
            agent_id: agent,
            box_id: ids::BOX,
            enabled: true,
            version: None,
            path: None,
            probed_at: None,
            quota: None,
            quota_at: None,
            updated_at: now,
            probe: None,
        })
        .await
        .expect("the fixture box takes an `agent_box` row");
    let quota = Quota {
        source: QuotaSource::AcpMetaRateLimit,
        billing: Billing::Subscription,
        status: Some(status.to_owned()),
        exhausted: false,
        windows: Vec::new(),
        spend: Spend::default(),
        observed_at: now,
    };
    orch.store()
        .set_agent_box_quota(agent, ids::BOX, quota.to_value(), now)
        .await
        .expect("the row was just written");
}

/// The item's notes that record a stage-1 substitution (plan D60), by their fixed opening.
async fn substitution_notes<O: Orchestrate>(orch: &O, item: ItemId) -> Vec<String> {
    notes_of(orch, item)
        .await
        .into_iter()
        .filter(|body| body.starts_with("stage 1 at "))
        .collect()
}

/// The section names of a step's seq-0 `prompt` event, as `Recorder::record_prompt` stored them.
///
/// # Panics
/// When the step recorded no session, which is what the caller is asserting it did.
async fn prompt_sections<O: Orchestrate>(orch: &O, step: StepId) -> Vec<String> {
    let events = orch
        .store()
        .step_events(step)
        .await
        .expect("MemStore never fails a read")
        .expect("the step recorded its session");
    let prompt = events
        .iter()
        .find(|event| event.seq == 0)
        .expect("seq 0 is the prompt");
    prompt.payload["sections"]
        .as_array()
        .expect("the prompt payload carries its sections")
        .iter()
        .filter_map(|section| section["name"].as_str().map(str::to_owned))
        .collect()
}

/// The scrubbed text of a step's seq-0 `prompt` event.
///
/// # Panics
/// When the step recorded no session, which is what the caller is asserting it did.
async fn prompt_text<O: Orchestrate>(orch: &O, step: StepId) -> String {
    let events = orch
        .store()
        .step_events(step)
        .await
        .expect("MemStore never fails a read")
        .expect("the step recorded its session");
    events
        .iter()
        .find(|event| event.seq == 0)
        .expect("seq 0 is the prompt")
        .payload["text"]
        .as_str()
        .expect("the prompt payload carries its text")
        .to_owned()
}

/// Plan D61 through the walk that reads it (plan D60): PRD D3's `allowed_warning` is a quota
/// status stage 1 **selects**, so the only candidate runs and parks at its gate with no
/// substitution to report. `exhausted`, a full window and every other status still skip; the
/// next case pins one of them.
async fn allowed_warning_candidate_is_selected<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    latch_quota(&orch, ids::AGENT_CLAUDE, "allowed_warning").await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "`allowed_warning` is selectable, so `prd` ran and parked at its `always` gate"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1);
    assert_eq!(
        (steps[0].agent_id, steps[0].status),
        (Some(ids::AGENT_CLAUDE), StepStatus::AwaitingApproval)
    );
    assert_eq!(
        substitution_notes(&orch, ids::HTUI_FEAT_3).await,
        Vec::<String>::new(),
        "nothing was skipped, so nothing was substituted"
    );
}

/// Plan D60: the walk skips a `rejected` first candidate and the selector takes the next one — and
/// the substitution is never silent (`R-ORCH-10`): one `item_note` names the skipped row, its
/// reason, and the row that ran instead.
async fn a_skipped_candidate_falls_through_to_the_next<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.with_candidates(
        "prd",
        vec![
            (ids::AGENT_CLAUDE, "sonnet"),
            (ids::AGENT_AGY, "gemini-3.7-flash-high"),
        ],
    );
    latch_quota(&orch, ids::AGENT_CLAUDE, "rejected").await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1);
    assert_eq!(
        (steps[0].agent_id, steps[0].model.as_deref()),
        (Some(ids::AGENT_AGY), Some("gemini-3.7-flash-high")),
        "the second candidate ran"
    );
    assert_eq!(
        substitution_notes(&orch, ids::HTUI_FEAT_3).await,
        [
            "stage 1 at `prd` attempt 1 (candidate 0): skipped claude (quota: status rejected); \
          chose agy/gemini-3.7-flash-high"
        ],
        "exactly one note, naming both rows (plan D60)"
    );
}

/// Plan D62, rung 4 at `StartRun`: `graph::resolve` finds no candidate, so no run row is created —
/// and the refusal is not invisible (invariant 7): the item moves `open -> blocked` and carries a
/// note naming the phase before the error is returned.
async fn no_candidate_agent_blocks_the_item<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.with_candidates("prd", Vec::new());
    let runs_before = orch
        .store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .len();

    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: None,
        })
        .await
        .expect_err("rung 4 refuses before a run row exists");
    assert!(
        matches!(
            &refused,
            EngineError::Resolve(crate::graph::ResolveError::NoCandidate { phase }) if phase == "prd"
        ),
        "{refused}"
    );
    assert_eq!(
        orch.store()
            .runs(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .len(),
        runs_before,
        "no run row was created"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "no_candidate_agent: phase `prd`"),
        "the rung-4 note has no detail suffix: {notes:?}"
    );
}

/// Plan D62, stage 1: the walk skips every candidate for a reason other than the inline-approval
/// interlock, so the run fails `no_candidate_agent` with each skip named, in blueprint H-16's
/// order — the item is `blocked` before the run is failed — and no step row exists.
async fn every_candidate_skipped_refuses_the_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    latch_quota(&orch, ids::AGENT_CLAUDE, "rejected").await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    let expected = RunFailure::NoCandidateAgent {
        phase: "prd".to_owned(),
        detail: "claude (quota: status rejected)".to_owned(),
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(0), Some(&expected))
    );
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some("no_candidate_agent: phase `prd`; claude (quota: status rejected)")
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked,
        "blocked before the run failed (blueprint H-16), or it would read `failed`"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| *body == expected.to_string()),
        "the refusal is a note a human reads: {notes:?}"
    );
    assert!(
        steps_of(&orch, run).await.is_empty(),
        "stage 1 refuses before a step row exists"
    );
}

/// Plan D67 (M3 D32's forward): a `never` phase whose attempt 1 fails its `verify_command` retries,
/// and attempt 2's prompt carries both loop sections — `verify_failure` from attempt 1's
/// `command_run` row, and `previous_diff` from `Isolator::diff` over attempt 1's rows.
///
/// **The phase renders with the `implement` template.** The seeded `prd` body has neither
/// placeholder (`crates/htui-core/src/prompt/defaults.rs:29-44`) and a section with no placeholder
/// is not rendered, so a `prd` prompt could never show them; `implement`'s body has both (`:66-67`).
async fn a_second_attempt_carries_verify_failure_and_previous_diff<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
            phase.retry_limit = 1;
            phase.verify_command = Some("cargo test".to_owned());
            phase.template_name = "implement".to_owned();
        }
    })
    .await;
    // A primary repo, so attempt 1 has tree and commit rows for `previous_diff` to be taken over.
    primary_repo(&orch).await;
    let mut report = FakeVerifier::fail(1);
    report.output = "test engine::walks ... FAILED".to_owned();
    orch.verifier().script_report(report);
    orch.isolator().script_diff(Some(DiffBlock {
        range: "fake:base:1..fake:after:1".to_owned(),
        stat: " src/lib.rs | 2 +-".to_owned(),
        diff: "diff --git a/src/lib.rs b/src/lib.rs\n".to_owned(),
    }));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "attempt 2 passed (the verifier has nothing left to fail) and `plan` parked"
    );
    let steps = steps_of(&orch, run).await;
    let first = at(&steps, 0, 1);
    assert_eq!(
        (first.status, first.verify_outcome),
        (StepStatus::Failed, Some(VerifyOutcome::Fail))
    );

    let first_sections = prompt_sections(&orch, first.id).await;
    assert!(
        !first_sections
            .iter()
            .any(|name| name == "verify_failure" || name == "previous_diff"),
        "attempt 1 has nothing to forward: {first_sections:?}"
    );
    let second_sections = prompt_sections(&orch, at(&steps, 0, 2).id).await;
    for name in ["verify_failure", "previous_diff"] {
        assert!(
            second_sections.iter().any(|section| section == name),
            "attempt 2's prompt carries `{name}`: {second_sections:?}"
        );
    }

    // The content, not just the names: attempt 1's exit code and `command_run.output`, and the
    // block `Isolator::diff` answered — over attempt 1's rows and nobody else's.
    let second = prompt_text(&orch, at(&steps, 0, 2).id).await;
    for needle in [
        "exit_code=\"1\"",
        "test engine::walks ... FAILED",
        "range=\"fake:base:1..fake:after:1\"",
        " src/lib.rs | 2 +-",
    ] {
        assert!(
            second.contains(needle),
            "attempt 2's prompt carries `{needle}`:\n{second}"
        );
    }
    let requests = orch.isolator().diff_requests();
    assert_eq!(requests.len(), 1, "one forward, one diff: {requests:?}");
    assert!(
        !requests[0].trees.is_empty() && !requests[0].commits.is_empty(),
        "attempt 1 left rows to diff: {requests:?}"
    );
    assert!(
        requests[0]
            .trees
            .iter()
            .chain(&requests[0].commits)
            .all(|step| *step == first.id),
        "`previous_diff` is taken over attempt 1's rows only: {requests:?}"
    );
}

// -- milestone 4: three candidates, one winner (plan D48-D53, D63, D65, D66) ---------------------

/// Plan D69: names `agy` the project's fan-out judge through the tests-only settings writer.
///
/// `agy` because it is the seeded agent with a model to run: `seeds/agent_agy.json` carries a
/// `default_model`, while `claude` has neither a default model nor a model list, so a `claude`
/// judge would be `judge_unavailable: no model`.
async fn set_judge<O: Orchestrate>(orch: &O, item: ItemId) {
    let project = item_of(orch, item).await.project_id;
    let mut settings = orch
        .store()
        .project_settings(project)
        .await
        .expect("MemStore never fails a read")
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    settings["judge_agent_id"] = serde_json::json!(ids::AGENT_AGY);
    orch.store().set_project_settings(project, settings);
}

/// `ANA-2` on the `analysis` graph with `research` fanned out three ways under `gate`, and
/// `verdict` ungated so a selected group walks on to `done` (blueprint F-E).
async fn fan_research<O: Orchestrate>(orch: &O, gate: Gate, verify: bool) {
    repoint(orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "research" {
            phase.fan_out = 3;
            phase.gate = gate;
            if verify {
                phase.verify_command = Some("true".to_owned());
            }
        } else {
            phase.gate = Gate::Never;
        }
    })
    .await;
}

/// Each of `research`'s three candidates at `attempt` writes a document naming itself.
fn research_candidates<O: Orchestrate>(orch: &O, attempt: i32) {
    for index in 0..3 {
        orch.script_candidate(
            "research",
            attempt,
            index,
            0,
            ScriptedStep::done_with_output(&format!("research by candidate {index}")),
        );
    }
}

/// Both of a judge's calls at `(phase, attempt)` answer `winner`.
fn judge_both<O: Orchestrate>(orch: &O, phase: &str, attempt: i32, winner: i32, reason: &str) {
    for call in 0..2 {
        orch.script_candidate(
            &format!("{phase}:judge"),
            attempt,
            -1,
            call,
            ScriptedStep::judge(winner, &[(winner, reason)]),
        );
    }
}

/// The candidates of slot `(position, attempt)`, in `fanout_index` order, as
/// `(fanout_index, status, selected)`.
fn slot_of(steps: &[RunStep], position: i32, attempt: i32) -> Vec<(i32, StepStatus, Option<bool>)> {
    let mut slot: Vec<_> = steps
        .iter()
        .filter(|step| step.position == position && step.attempt == attempt)
        .filter(|step| step.fanout_index >= 0)
        .map(|step| (step.fanout_index, step.status, step.selected))
        .collect();
    slot.sort_by_key(|(index, _, _)| *index);
    slot
}

/// The judge row of slot `(position, attempt)`, if the walk created one.
fn judge_of(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep> {
    steps.iter().find(|step| {
        step.position == position && step.attempt == attempt && step.fanout_index == -1
    })
}

/// The candidate at `(position, attempt, fanout_index)`.
///
/// # Panics
/// When the walk never created it.
fn candidate(steps: &[RunStep], position: i32, attempt: i32, index: i32) -> &RunStep {
    steps
        .iter()
        .find(|step| {
            step.position == position && step.attempt == attempt && step.fanout_index == index
        })
        .unwrap_or_else(|| panic!("the walk created ({position},{attempt},{index})"))
}

/// The item notes that D50's park writes, by their fixed opening.
async fn selection_parks<O: Orchestrate>(orch: &O, item: ItemId) -> Vec<String> {
    notes_of(orch, item)
        .await
        .into_iter()
        .filter(|body| body.starts_with("fan-out `") && body.contains(" awaits selection: "))
        .collect()
}

/// ANA-2 §12 criterion 8 (`docs/ANA-2.md:2106`): three candidates, a judge, one winner.
///
/// The judge ran as a real step at `fanout_index = -1` named `research:judge`, two sessions under
/// one recorder — the forward prompt at seq 0 and the reversed one as a `follow_up` opening turn 1
/// (plan D52) — and both answered candidate 1, whose reason became its `gate_note`. The losers are
/// `superseded`, every candidate's document survives, and the next phase reads only the winner's.
async fn fan_out_three_with_a_judge_selects_one_winner<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 1, "the most thorough");

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Done, Some(true)),
            (2, StepStatus::Superseded, Some(false)),
        ]
    );
    let judge = judge_of(&steps, 0, 1).expect("the judge is a step");
    assert_eq!(
        (
            judge.phase_name.as_str(),
            judge.status,
            judge.gate_note.as_deref()
        ),
        (
            "research:judge",
            StepStatus::Done,
            Some("the most thorough")
        )
    );
    assert!(judge.prompt_digest.is_some(), "the forward prompt's digest");
    let events = orch
        .store()
        .step_events(judge.id)
        .await
        .expect("MemStore never fails a read")
        .expect("the judge recorded its sessions");
    assert!(
        events
            .iter()
            .any(|event| event.seq == 0 && event.kind == EventKind::Prompt),
        "call 0 is the seq-0 prompt"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == EventKind::FollowUp)
            .map(|event| event.turn)
            .collect::<Vec<_>>(),
        [1],
        "call 1 is one follow_up, opening turn 1 (plan D52)"
    );

    let heads = orch
        .store()
        .documents(ids::HTUI_ANA_2)
        .await
        .expect("MemStore never fails a read");
    let research: Vec<_> = heads
        .iter()
        .filter(|head| head.kind == "research")
        .filter(|head| {
            steps
                .iter()
                .any(|step| step.position == 0 && Some(step.id) == head.produced_by_step_id)
        })
        .collect();
    assert_eq!(research.len(), 3, "every candidate's document survives");

    let verdict = steps
        .iter()
        .find(|step| step.phase_name == "verdict")
        .expect("the walk went on to `verdict`");
    let text = prompt_text(&orch, verdict.id).await;
    assert!(text.contains("research by candidate 1"), "{text}");
    for loser in ["research by candidate 0", "research by candidate 2"] {
        assert!(
            !text.contains(loser),
            "a loser's document is not an input: {text}"
        );
    }
}

/// Criterion 9's first half (`:2109`): with two or more passing, a `verify_command` failure is
/// eliminated before the judge — it is not in the judge's prompt at all — and the failed
/// candidate itself still settles `done` (plan D48), so it would have been selectable.
async fn a_failing_verify_is_eliminated_before_the_judge<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, true).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    for report in [
        FakeVerifier::pass(),
        FakeVerifier::fail(1),
        FakeVerifier::pass(),
    ] {
        orch.verifier().script_report(report);
    }
    judge_both(&orch, "research", 1, 2, "passes and is shorter");

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::Done);
    let steps = steps_of(&orch, run).await;
    let failed = candidate(&steps, 0, 1, 1);
    assert_eq!(
        (failed.status, failed.verify_outcome, failed.selected),
        (
            StepStatus::Superseded,
            Some(VerifyOutcome::Fail),
            Some(false)
        ),
        "the verify outcome is recorded and not applied to the settle (plan D48): the candidate \
         settled `done` and lost, rather than `failed`"
    );
    let judge = judge_of(&steps, 0, 1).expect("two passed, so the judge ran");
    let sections = prompt_sections(&orch, judge.id).await;
    for name in ["judge_candidate:0", "judge_candidate:2"] {
        assert!(
            sections.iter().any(|section| section == name),
            "{sections:?}"
        );
    }
    assert!(
        !sections
            .iter()
            .any(|section| section == "judge_candidate:1"),
        "the verify failure was eliminated: {sections:?}"
    );
    assert_eq!(candidate(&steps, 0, 1, 2).selected, Some(true));
}

/// Criterion 9's second half (`:2110`): exactly one candidate passes, so it wins outright — no
/// judge step is created — and the reason is an `item_note` naming the winner (plan D77).
async fn one_passing_candidate_wins_without_a_judge<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, true).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    for report in [
        FakeVerifier::fail(1),
        FakeVerifier::pass(),
        FakeVerifier::fail(2),
    ] {
        orch.verifier().script_report(report);
    }

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::Done);
    let steps = steps_of(&orch, run).await;
    assert!(
        steps.iter().all(|step| step.fanout_index != -1),
        "no judge row (plan D49(4))"
    );
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Done, Some(true)),
            (2, StepStatus::Superseded, Some(false)),
        ]
    );
    let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
    assert!(
        notes.iter().any(|body| body
            == "fan-out `research` attempt 1: candidate 1 wins as the only candidate whose \
                verify_command did not fail"),
        "the auto-win's reason is not dropped (plan D77): {notes:?}"
    );
}

/// Criterion 10's first half (`:2112`): the two orderings disagree, so the judge fails with the
/// reason as its `gate_note`, every candidate stays as it settled with `selected` NULL, and the
/// run and the item park for a human with `run.failure` NULL (plan D50).
async fn judge_orderings_that_disagree_park_for_selection<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    orch.script_candidate("research:judge", 1, -1, 0, ScriptedStep::judge(0, &[]));
    orch.script_candidate("research:judge", 1, -1, 1, ScriptedStep::judge(2, &[]));

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None)
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Done, None),
            (1, StepStatus::Done, None),
            (2, StepStatus::Done, None),
        ]
    );
    let judge = judge_of(&steps, 0, 1).expect("the judge ran");
    assert_eq!(
        (judge.status, judge.gate_note.as_deref()),
        (
            StepStatus::Failed,
            Some("judge_disagreement: forward 0, reversed 2")
        )
    );
    let row = run_of(&orch, run).await;
    assert_eq!(
        (row.status, row.failure),
        (RunStatus::AwaitingApproval, None)
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_ANA_2).await.status,
        Status::AwaitingApproval
    );
    assert_eq!(
        selection_parks(&orch, ids::HTUI_ANA_2).await,
        [
            "fan-out `research` attempt 1 awaits selection: judge_disagreement: forward 0, \
             reversed 2; candidates: 0 done (verify none), 1 done (verify none), 2 done (verify \
             none)"
        ]
    );
}

/// Criterion 10's second half (`:2113`): an unparseable verdict parks the group, and
/// `SelectFanout` completes it — the human's pick wins, the failed judge keeps its reason, and the
/// walk goes on to `done`.
async fn an_unparseable_verdict_parks_and_select_fanout_completes<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    orch.script_candidate(
        "research:judge",
        1,
        -1,
        0,
        ScriptedStep::done_with_output("Candidate 2 is clearly the better one."),
    );

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let steps = steps_of(&orch, run).await;
    let judge = judge_of(&steps, 0, 1).expect("the judge ran").clone();
    assert_eq!(judge.status, StepStatus::Failed);
    assert_eq!(
        judge.gate_note.as_deref(),
        Some("judge_unparseable: no fenced json block")
    );

    let winner = candidate(&steps, 0, 1, 2).id;
    let outcome = orch
        .dispatch(Command::SelectFanout {
            run,
            position: 0,
            attempt: 1,
            winner,
        })
        .await
        .expect("the parked group is selectable");
    let CommandOutcome::Selected { rest } = outcome else {
        panic!("`SelectFanout` answers `Selected`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Superseded, Some(false)),
            (2, StepStatus::Done, Some(true)),
        ]
    );
    let after = judge_of(&steps, 0, 1).expect("the judge is still there");
    assert_eq!(
        (after.status, after.gate_note.as_deref()),
        (StepStatus::Failed, judge.gate_note.as_deref()),
        "a judge that already failed is left alone, reason and all"
    );
    let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "fan-out `research` attempt 1: candidate 2 selected by a human"),
        "the human's pick is a note (plan D77): {notes:?}"
    );
    assert_eq!(item_of(&orch, ids::HTUI_ANA_2).await.status, Status::Done);
}

/// Plan D49(2): an `always` group goes straight to a human — no prefilter verdict, no judge row.
async fn a_gated_fan_out_parks_for_human_selection<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Always, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    let steps = steps_of(&orch, run).await;
    assert!(
        judge_of(&steps, 0, 1).is_none(),
        "a gated group runs no judge"
    );
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Done, None),
            (1, StepStatus::Done, None),
            (2, StepStatus::Done, None),
        ],
        "no candidate parks; the run does (plan D50)"
    );
    let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(parks.len(), 1, "{parks:?}");
    assert!(parks[0].contains(" awaits selection: gated; "), "{parks:?}");
}

/// Plan D49(3): an ungated group with no judge configured goes to a human.
async fn no_judge_means_human_selection<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    assert!(judge_of(&steps_of(&orch, run).await, 0, 1).is_none());
    let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(parks.len(), 1, "{parks:?}");
    assert!(
        parks[0].contains(" awaits selection: no judge configured; "),
        "{parks:?}"
    );
}

/// Plan D48: a candidate's failure is its own. Candidate 1 refuses; it lands `failed` with a note
/// naming it, its siblings settle `done`, the judge compares the two, and the run goes on.
async fn a_failed_candidate_does_not_fail_its_siblings<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    orch.script_candidate(
        "research",
        1,
        1,
        0,
        ScriptedStep::failing(htui_agent::event::StopReason::Refusal),
    );
    judge_both(&orch, "research", 1, 0, "complete");

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::Done, "the run continues");
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Done, Some(true)),
            (1, StepStatus::Failed, Some(false)),
            (2, StepStatus::Superseded, Some(false)),
        ],
        "a failed loser keeps `failed` (`store/traits.rs:808`)"
    );
    let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
    assert!(
        notes
            .iter()
            .any(|body| body.starts_with("fan-out candidate 1 of `research` attempt 1: ")),
        "the failure is a note naming the candidate: {notes:?}"
    );
    let judge = judge_of(&steps, 0, 1).expect("two survived, so the judge ran");
    assert!(
        !prompt_sections(&orch, judge.id)
            .await
            .iter()
            .any(|section| section == "judge_candidate:1"),
        "a failed candidate is not in the pool"
    );
}

/// Plan D49(1): a `never` group with no survivor is §4.2's `failed` cell for the whole group — it
/// retries at `attempt + 1` while the budget holds, retiring attempt 1, and then fails the run.
async fn a_group_with_no_survivor_retries_then_fails<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        phase.gate = Gate::Never;
        if phase.name == "research" {
            phase.fan_out = 3;
            phase.retry_limit = 1;
        }
    })
    .await;
    for attempt in [1, 2] {
        orch.script(
            "research",
            attempt,
            ScriptedStep::failing(htui_agent::event::StopReason::Refusal),
        );
    }

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    let failure = RunFailure::NoSurvivingCandidate {
        phase: "research".to_owned(),
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(0), Some(&failure))
    );
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some("no_surviving_candidate: research")
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [
            (0, StepStatus::Cancelled, None),
            (1, StepStatus::Cancelled, None),
            (2, StepStatus::Cancelled, None),
        ],
        "attempt 1 was retired whole"
    );
    assert_eq!(
        slot_of(&steps, 0, 2),
        [
            (0, StepStatus::Failed, None),
            (1, StepStatus::Failed, None),
            (2, StepStatus::Failed, None),
        ],
        "attempt 2 is a whole group"
    );
    assert_eq!(
        harness_cleanups(&orch),
        1,
        "a terminal run is cleaned up once"
    );
}

/// Plan D66 (ANA-2 `:754-758`): a review rejection of a fanned-out `implement` retires its whole
/// slot — winner, loser and judge — and the next attempt is a new group, judged again.
async fn the_review_loop_reruns_a_fanned_out_implement<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        phase.gate = Gate::Never;
        if phase.name == "implement" {
            phase.fan_out = 2;
        }
    })
    .await;
    set_judge(&orch, ids::HTUI_FEAT_3).await;
    for attempt in [1, 2] {
        judge_both(&orch, "implement", attempt, 0, "the smaller change");
    }
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "the error path is untested"),
    );
    orch.script("review", 2, ScriptedStep::review("approve", "good"));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    let steps = steps_of(&orch, run).await;
    assert!(
        slot_of(&steps, 2, 1)
            .iter()
            .all(|(_, status, _)| matches!(status, StepStatus::Superseded | StepStatus::Cancelled)),
        "slot (2,1) is retired whole: {:?}",
        slot_of(&steps, 2, 1)
    );
    assert_eq!(
        judge_of(&steps, 2, 1).map(|judge| judge.status),
        Some(StepStatus::Superseded),
        "the first judge is retired with its slot"
    );
    assert_eq!(
        slot_of(&steps, 2, 2),
        [
            (0, StepStatus::Done, Some(true)),
            (1, StepStatus::Superseded, Some(false)),
        ],
        "attempt 2 is a new group"
    );
    assert_eq!(
        judge_of(&steps, 2, 2).map(|judge| judge.status),
        Some(StepStatus::Done),
        "the judge runs again"
    );
}

/// Plan D63: a phase above `max_fan_out` (4) is refused at `StartRun`, before a run row exists.
async fn fan_out_above_max_fan_out_is_refused_at_start<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "research" {
            phase.fan_out = 5;
        }
    })
    .await;
    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            repo_scope: None,
        })
        .await
        .expect_err("fan_out 5 exceeds max_fan_out 4");
    assert!(
        matches!(
            &refused,
            EngineError::Resolve(crate::graph::ResolveError::FanOutCap { phase, fan_out: 5, max: 4 })
                if phase == "research"
        ),
        "{refused}"
    );
    assert!(
        orch.store()
            .runs(ids::HTUI_ANA_2)
            .await
            .expect("MemStore never fails a read")
            .is_empty(),
        "no run row"
    );
}

/// Plan D63 (OQ-1): `prd 1 + plan 1 + implement 3 + its judge 1 + review 1 = 7` agents against
/// `max_agents_per_run = 6`, refused at `StartRun` naming both figures.
async fn max_agents_per_run_is_refused_at_start<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "implement" {
            phase.fan_out = 3;
        }
    })
    .await;
    set_judge(&orch, ids::HTUI_FEAT_3).await;
    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: None,
        })
        .await
        .expect_err("seven agents exceed six");
    assert!(
        matches!(
            refused,
            EngineError::Resolve(crate::graph::ResolveError::AgentCap { planned: 7, max: 6 })
        ),
        "{refused}"
    );
}

/// Plan D48 under `shared_serialized`: every candidate goes `running` when the group starts, but
/// its `prepare` waits on the per-repo lock until the sibling before it is captured. Its deadline
/// and its verify budget (plan D30) are its own session's, so they count from the moment `prepare`
/// answered — otherwise the last of three 3000 s sessions under a 7200 s deadline would be charged
/// 9000 s, fail `deadline elapsed` or verify `unavailable`, and skew selection towards index 0.
async fn a_serialized_sibling_is_not_charged_for_the_ones_before_it<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "research" {
            phase.fan_out = 3;
            phase.isolation = Some(Isolation::SharedSerialized);
            phase.verify_command = Some("true".to_owned());
        }
        phase.gate = Gate::Never;
    })
    .await;
    orch.store()
        .set_app_setting("step_deadline_seconds", serde_json::json!(7200));
    orch.advance_after_done(TimeDelta::seconds(3000));
    research_candidates(&orch, 1);
    for _ in 0..3 {
        orch.verifier().script_report(FakeVerifier::pass());
    }

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "no judge, so a human selects"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1)
            .into_iter()
            .map(|(index, status, _)| (index, status))
            .collect::<Vec<_>>(),
        [
            (0, StepStatus::Done),
            (1, StepStatus::Done),
            (2, StepStatus::Done),
        ],
        "no sibling pays for the sessions queued ahead of it"
    );
    let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
    assert!(
        !notes.iter().any(|body| body.contains("deadline elapsed")),
        "{notes:?}"
    );
    let budget = Some(std::time::Duration::from_secs(7200 - 3000));
    assert_eq!(
        orch.verifier().remaining(),
        (0..3)
            .map(|index| (candidate(&steps, 0, 1, index).id, budget))
            .collect::<Vec<_>>(),
        "each verify runs under what its own session left of the deadline"
    );
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
    fn cases_are_unique_and_thirty_six() {
        let mut sorted: Vec<&&str> = CASES.iter().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
        assert_eq!(
            CASES.len(),
            36,
            "seven ANA-2 §12 criteria, four §4.2 contract lines, the `finish_run` seam, the three \
             gate-table cells only an edited gate reaches, plan D5's intermediate position, \
             milestone 3's two verify outcomes and `CancelRun`, milestone 4's five stage-1 cases \
             — the `allowed_warning` selection, the fall-through to the next candidate, the \
             rung-4 refusal at `StartRun`, the stage-1 walk that skips every candidate, and the \
             second attempt's forwarded `verify_failure` and `previous_diff` — and its twelve \
             fan-out cases: criteria 8, 9 (twice) and 10 (twice), the gated and judgeless human \
             selections, a failed candidate, a group with no survivor, the review loop over a \
             group, the two caps, and a `shared_serialized` sibling's own deadline"
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

/// Fan-out paths no [`CASES`] entry reaches: a candidate's own errors once live (plan D48, D78),
/// a group whose one prompt is missing an input (D76), the judge prompt's dropped candidate (D53),
/// the crash leftovers `Select` and `Fan` resume from (D59), the judge failures criteria 8-10 do
/// not name (D51, D52), the winner's siblings (D54(d)) and the `shared_serialized` park note.
///
/// Unit tests rather than cases: several seed a crash's rows straight into the store, which is a
/// `MemStore` write no transport-neutral binding promises.
#[cfg(test)]
mod fanout_paths {
    use htui_core::fixtures::ids;
    use htui_core::model::{
        Gate, Isolation, NewRunStep, RunStatus, RunStep, RunStepCommit, Status, StepId, StepStatus,
    };
    use htui_core::store::{ReadStore as _, WriteStore as _};

    use super::{
        FakeOrchestrator, Orchestrate, RunFailure, ScriptedStep, candidate, fan_research,
        free_feat_3, item_of, judge_both, judge_of, latch_quota, notes_of, primary_repo, repoint,
        research_candidates, run_of, selection_parks, set_input_kinds, set_judge, slot_of, start,
        steps_of,
    };
    use crate::engine::Resume;
    use crate::isolate::Clock as _;

    /// A crash's leftover run: `awaiting_approval -> running` and the item back to `in_progress`,
    /// by hand, so `resume` walks rows no command answered.
    async fn unpark_by_hand(orch: &FakeOrchestrator, run: htui_core::model::RunId) {
        let now = orch.clock().now();
        assert!(
            orch.store()
                .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
                .await
                .expect("MemStore takes the move"),
            "the run was parked"
        );
        let item = run_of(orch, run)
            .await
            .item_id
            .expect("the run has an item");
        assert!(
            orch.store()
                .transition(item, Status::AwaitingApproval, Status::InProgress)
                .await
                .expect("MemStore takes the move"),
            "the item was parked"
        );
    }

    /// `resume`, unwrapped to the rest of a walk that ran.
    async fn resumed(orch: &FakeOrchestrator, run: htui_core::model::RunId) -> super::Rest {
        match orch.resume(run).await.expect("the walk resumes") {
            Resume::Walked(rest) => rest,
            Resume::TopologyChanged { .. } => panic!("nothing edited the graph"),
        }
    }

    /// A `pending` judge row at `(0, 1)` as a crash between `create_judge` and its
    /// `pending -> running` leaves it.
    async fn seed_pending_judge(orch: &FakeOrchestrator, run: htui_core::model::RunId) -> StepId {
        orch.store()
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: -1,
                phase_name: "research:judge".to_owned(),
                agent_id: Some(ids::AGENT_AGY),
                model: Some("seeded".to_owned()),
            })
            .await
            .expect("the slot has no judge yet")
            .id
    }

    /// Candidate `index` of `(0, 1)`'s id.
    fn candidate_id(steps: &[RunStep], index: i32) -> StepId {
        candidate(steps, 0, 1, index).id
    }

    /// Plan D48's second clause and plan D78. Candidate 0's `prepare` errors (nothing to release)
    /// and candidate 1's driver refuses to start after its `prepare` succeeded: each lands
    /// `failed` with a note naming it, and candidate 1's trees are still captured on the way out.
    /// The run neither fails nor errors — the one survivor wins outright (D49(4)), reconciled with
    /// both failed siblings (D54(d)).
    #[tokio::test]
    async fn a_candidate_that_errors_once_live_fails_alone_and_is_captured() {
        let orch = FakeOrchestrator::demo();
        primary_repo(&orch).await;
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        orch.isolator().refuse_prepare("the worktree is locked");
        orch.script_candidate(
            "research",
            1,
            1,
            0,
            ScriptedStep::refusing_to_start("no `agy` on PATH"),
        );

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Failed, Some(false)),
                (1, StepStatus::Failed, Some(false)),
                (2, StepStatus::Done, Some(true)),
            ]
        );
        let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
        for (index, why) in [(0, "the worktree is locked"), (1, "no `agy` on PATH")] {
            let opening = format!("fan-out candidate {index} of `research` attempt 1: ");
            assert!(
                notes
                    .iter()
                    .any(|body| body.starts_with(&opening) && body.contains(why)),
                "candidate {index}'s error is its own note: {notes:?}"
            );
        }

        let never_prepared = orch
            .store()
            .step_commits(candidate_id(&steps, 0))
            .await
            .expect("MemStore never fails a read");
        assert!(never_prepared.is_empty(), "{never_prepared:?}");
        let released = orch
            .store()
            .step_commits(candidate_id(&steps, 1))
            .await
            .expect("MemStore never fails a read");
        assert!(!released.is_empty(), "candidate 1 was prepared");
        assert!(
            released.iter().all(|commit| commit.after_hash.is_some()),
            "a candidate failing between `prepare` and `capture` is captured best-effort (D78): \
             {released:?}"
        );

        let winner = candidate(&steps, 0, 1, 2).id;
        assert_eq!(
            orch.isolator().reconciles().first(),
            Some(&(
                winner,
                vec![candidate(&steps, 0, 1, 0).id, candidate(&steps, 0, 1, 1).id]
            )),
            "the auto-win is reconciled with its siblings (D54(d))"
        );
    }

    /// Plan D76 (blueprint A-5): a fanned-out phase whose one prompt is missing a required input
    /// fails every candidate `pending -> running -> failed` with a note, then the run, before any
    /// session or tree.
    #[tokio::test]
    async fn a_group_missing_an_input_fails_every_candidate_before_a_token() {
        let orch = FakeOrchestrator::demo();
        free_feat_3(&orch).await;
        repoint(&orch, ids::HTUI_FEAT_3, |phase| {
            if phase.position == 0 {
                phase.fan_out = 3;
            }
        })
        .await;
        set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;

        let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (
                RunStatus::Failed,
                Some(0),
                Some(RunFailure::MissingInput("spec".to_owned()))
            )
        );
        assert_eq!(
            run_of(&orch, run).await.failure.as_deref(),
            Some("missing input document: spec")
        );
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Failed, None),
                (1, StepStatus::Failed, None),
                (2, StepStatus::Failed, None),
            ],
            "no candidate is left `pending` under a failed run"
        );
        assert!(
            steps
                .iter()
                .all(|step| step.prompt_digest.is_none() && step.usage.is_none()),
            "no prompt, no session"
        );
        assert_eq!(orch.isolator().prepares(), 0, "no tree was prepared");
        let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
        let phase = &steps[0].phase_name;
        for index in 0..3 {
            let note = format!(
                "fan-out candidate {index} of `{phase}` attempt 1: missing input document: spec"
            );
            assert!(notes.contains(&note), "{note} in {notes:?}");
        }
    }

    /// Plan D53: a candidate the judge prompt's trimmer drops outright sends the group to a human
    /// with `judge_candidate_dropped: <i>`, and no judge row is written.
    #[tokio::test]
    async fn a_candidate_dropped_from_the_judge_prompt_parks_with_no_judge_row() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        let huge = "an exhaustive finding that goes on and on. ".repeat(40_000);
        orch.script_candidate("research", 1, 2, 0, ScriptedStep::done_with_output(&huge));

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (RunStatus::AwaitingApproval, Some(0), None)
        );
        let steps = steps_of(&orch, run).await;
        assert!(judge_of(&steps, 0, 1).is_none(), "no judge row (D53)");
        let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(parks.len(), 1, "{parks:?}");
        assert!(
            parks[0].contains(" awaits selection: judge_candidate_dropped: 2; "),
            "{parks:?}"
        );
    }

    /// Plan D59: a `failed` judge left by a crash between `fail_judge`'s writes and its park is
    /// re-parked with its own reason, and never re-run.
    #[tokio::test]
    async fn a_leftover_failed_judge_reparks_without_rejudging() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        orch.script_candidate("research:judge", 1, -1, 0, ScriptedStep::judge(0, &[]));
        orch.script_candidate("research:judge", 1, -1, 1, ScriptedStep::judge(2, &[]));
        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let judge = judge_of(&steps_of(&orch, run).await, 0, 1)
            .expect("the judge ran")
            .clone();
        let events = orch
            .store()
            .step_events(judge.id)
            .await
            .expect("MemStore never fails a read")
            .map_or(0, |events| events.len());

        unpark_by_hand(&orch, run).await;
        let rest = resumed(&orch, run).await;
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0))
        );
        let steps = steps_of(&orch, run).await;
        let judges: Vec<_> = steps
            .iter()
            .filter(|step| step.fanout_index == -1)
            .collect();
        assert_eq!(judges.len(), 1, "no second judge: {judges:?}");
        assert_eq!(
            (
                judges[0].id,
                judges[0].status,
                judges[0].gate_note.as_deref()
            ),
            (
                judge.id,
                StepStatus::Failed,
                Some("judge_disagreement: forward 0, reversed 2")
            )
        );
        assert_eq!(
            orch.store()
                .step_events(judge.id)
                .await
                .expect("MemStore never fails a read")
                .map_or(0, |events| events.len()),
            events,
            "the judge ran no new session"
        );
        let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(parks.len(), 2, "{parks:?}");
        assert_eq!(parks[0], parks[1], "the same reason, re-parked");
    }

    /// Plan D59: a `pending` judge left by a crash is **that** judge — run with the passing set
    /// recomputed, not replaced — and the winner is reconciled with its siblings (D54(d)).
    #[tokio::test]
    async fn a_leftover_pending_judge_is_the_one_that_runs() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Always, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        judge_both(&orch, "research", 1, 1, "the most thorough");
        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval, "gated: no judge yet");
        let judge = seed_pending_judge(&orch, run).await;

        unpark_by_hand(&orch, run).await;
        let rest = resumed(&orch, run).await;
        assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
        let steps = steps_of(&orch, run).await;
        let judges: Vec<_> = steps
            .iter()
            .filter(|step| step.fanout_index == -1)
            .collect();
        assert_eq!(judges.len(), 1, "{judges:?}");
        assert_eq!(
            (
                judges[0].id,
                judges[0].status,
                judges[0].gate_note.as_deref()
            ),
            (judge, StepStatus::Done, Some("the most thorough"))
        );
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Superseded, Some(false)),
                (1, StepStatus::Done, Some(true)),
                (2, StepStatus::Superseded, Some(false)),
            ]
        );
        assert_eq!(
            orch.isolator().reconciles().first(),
            Some(&(
                candidate(&steps, 0, 1, 1).id,
                vec![candidate(&steps, 0, 1, 0).id, candidate(&steps, 0, 1, 2).id]
            )),
            "the judged winner is reconciled with its siblings (D54(d))"
        );
    }

    /// Plan D51, blueprint F-I: a judge agent the walk skips is `judge_unavailable` — the judge
    /// row `failed` with the walk's cause as its `gate_note`, and the group parked.
    #[tokio::test]
    async fn a_judge_the_walk_skips_is_unavailable_and_parks() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Always, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        judge_both(&orch, "research", 1, 1, "never read");
        let (run, _) = start(&orch, ids::HTUI_ANA_2).await;
        let judge = seed_pending_judge(&orch, run).await;
        latch_quota(&orch, ids::AGENT_AGY, "rejected").await;

        unpark_by_hand(&orch, run).await;
        let rest = resumed(&orch, run).await;
        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (RunStatus::AwaitingApproval, Some(0), None)
        );
        let steps = steps_of(&orch, run).await;
        let row = judge_of(&steps, 0, 1).expect("the judge row stays");
        assert_eq!((row.id, row.status), (judge, StepStatus::Failed));
        let note = row.gate_note.clone().expect("the reason is on the row");
        assert!(note.starts_with("judge_unavailable: "), "{note}");
        assert!(
            orch.store()
                .step_events(judge)
                .await
                .expect("MemStore never fails a read")
                .is_none_or(|events| events.is_empty()),
            "a skipped judge opens no session"
        );
        let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
        assert!(
            parks
                .last()
                .is_some_and(|park| park.contains(&format!(" awaits selection: {note}; "))),
            "{parks:?}"
        );
        assert!(
            slot_of(&steps, 0, 1)
                .iter()
                .all(|(_, _, selected)| selected.is_none())
        );
    }

    /// Plan D52: call 1 must write a **new** `judge` document; re-reading call 0's is
    /// `judge_missing_document: call 1`.
    #[tokio::test]
    async fn a_judge_call_that_writes_nothing_is_a_missing_document() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        orch.script_candidate("research:judge", 1, -1, 0, ScriptedStep::judge(1, &[]));
        orch.script_candidate(
            "research:judge",
            1,
            -1,
            1,
            ScriptedStep::done_without_output(),
        );

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let steps = steps_of(&orch, run).await;
        let judge = judge_of(&steps, 0, 1).expect("the judge ran");
        assert_eq!(
            (judge.status, judge.gate_note.as_deref()),
            (StepStatus::Failed, Some("judge_missing_document: call 1"))
        );
    }

    /// Plan D52: the two calls share one recorder, so a run cap the pair breaches between them is
    /// `judge_session_failed: cap breached`, even when both verdicts agree.
    #[tokio::test]
    async fn a_judge_that_breaches_the_run_cap_fails() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        let project = item_of(&orch, ids::HTUI_ANA_2).await.project_id;
        orch.store().set_project_settings(
            project,
            serde_json::json!({ "judge_agent_id": ids::AGENT_AGY, "per_token_cap_run": 1_000 }),
        );
        research_candidates(&orch, 1);
        let verdict = "Compared.\n\n```json\n{\"winner\": 1, \"reasons\": {\"1\": \"best\"}}\n```";
        for call in 0..2 {
            orch.script_candidate(
                "research:judge",
                1,
                -1,
                call,
                ScriptedStep::done_costing(verdict, 600),
            );
        }

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let steps = steps_of(&orch, run).await;
        let judge = judge_of(&steps, 0, 1).expect("the judge ran");
        assert_eq!(judge.status, StepStatus::Failed);
        let note = judge.gate_note.as_deref().unwrap_or_default();
        assert!(
            note.starts_with("judge_session_failed: cap breached"),
            "{note}"
        );
    }

    /// Plan D51: a judge that names the seeded `claude`, which has no model to run, is
    /// `judge_unavailable: no model` on a judge row, and the group parks.
    #[tokio::test]
    async fn a_judge_with_no_model_is_unavailable() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        let project = item_of(&orch, ids::HTUI_ANA_2).await.project_id;
        orch.store().set_project_settings(
            project,
            serde_json::json!({ "judge_agent_id": ids::AGENT_CLAUDE }),
        );
        research_candidates(&orch, 1);

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let steps = steps_of(&orch, run).await;
        let judge = judge_of(&steps, 0, 1).expect("the reason is on a row");
        assert_eq!(
            (
                judge.status,
                judge.gate_note.as_deref(),
                judge.model.as_deref()
            ),
            (
                StepStatus::Failed,
                Some("judge_unavailable: no model"),
                None
            )
        );
    }

    /// Plan D59, `Fan`: a slot a crash left with only candidate 0 has the missing indices admitted
    /// and driven, and they start from the base candidate 0 already recorded (D54(b)).
    #[tokio::test]
    async fn a_partial_group_admits_the_missing_indices_from_the_recorded_base() {
        let orch = FakeOrchestrator::demo();
        let repo = primary_repo(&orch).await;
        fan_research(&orch, Gate::Always, false).await;
        research_candidates(&orch, 1);
        research_candidates(&orch, 2);
        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);

        // Attempt 1 retired, and attempt 2 created as far as candidate 0 before the crash.
        let now = orch.clock().now();
        let steps = steps_of(&orch, run).await;
        for index in 0..3 {
            orch.store()
                .transition_step(
                    candidate(&steps, 0, 1, index).id,
                    StepStatus::Done,
                    StepStatus::Superseded,
                    now,
                )
                .await
                .expect("MemStore takes the move");
        }
        let first = candidate(&steps, 0, 1, 0);
        let zero = orch
            .store()
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 2,
                fanout_index: 0,
                phase_name: first.phase_name.clone(),
                agent_id: first.agent_id,
                model: first.model.clone(),
            })
            .await
            .expect("the slot is new")
            .id;
        for (from, to) in [
            (StepStatus::Pending, StepStatus::Running),
            (StepStatus::Running, StepStatus::Done),
        ] {
            orch.store()
                .transition_step(zero, from, to, now)
                .await
                .expect("MemStore takes the move");
        }
        orch.store()
            .record_commits(
                zero,
                &[RunStepCommit {
                    run_step_id: zero,
                    repo_id: repo,
                    before_hash: "seeded:base".to_owned(),
                    after_hash: Some("seeded:after".to_owned()),
                }],
            )
            .await
            .expect("MemStore takes the rows");

        unpark_by_hand(&orch, run).await;
        let rest = resumed(&orch, run).await;
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "the completed group is gated"
        );
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 2),
            [
                (0, StepStatus::Done, None),
                (1, StepStatus::Done, None),
                (2, StepStatus::Done, None),
            ],
            "indices 1 and 2 were admitted and driven"
        );
        for index in [1, 2] {
            let step = steps
                .iter()
                .find(|step| step.attempt == 2 && step.fanout_index == index)
                .expect("admitted");
            let commits = orch
                .store()
                .step_commits(step.id)
                .await
                .expect("MemStore never fails a read");
            assert_eq!(
                commits
                    .iter()
                    .map(|commit| commit.before_hash.as_str())
                    .collect::<Vec<_>>(),
                ["seeded:base"],
                "candidate {index} starts from the slot's recorded base"
            );
        }
    }

    /// The plan's Risks row: a `shared_serialized` group's park note says the checkout stays at
    /// the last sibling's commit, naming the base and every sibling's `htui/<step>` label.
    #[tokio::test]
    async fn a_shared_serialized_park_names_the_checkout() {
        let orch = FakeOrchestrator::demo();
        let repo = primary_repo(&orch).await;
        repoint(&orch, ids::HTUI_ANA_2, |phase| {
            if phase.name == "research" {
                phase.fan_out = 3;
                phase.gate = Gate::Always;
                phase.isolation = Some(Isolation::SharedSerialized);
            } else {
                phase.gate = Gate::Never;
            }
        })
        .await;
        research_candidates(&orch, 1);

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let steps = steps_of(&orch, run).await;
        let labels = (0..3)
            .map(|index| format!("htui/{}", candidate(&steps, 0, 1, index).id))
            .collect::<Vec<_>>()
            .join(", ");
        let parks = selection_parks(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(parks.len(), 1, "{parks:?}");
        assert!(
            parks[0].ends_with(&format!(
                "; the shared checkout stays at the last sibling's commit (base \
                 {repo}@fake:group-base:{repo}; labels {labels})"
            )),
            "{parks:?}"
        );
    }

    /// A sink that supersedes the slot's candidate `winner` the moment the judge's first call
    /// ends, so the verdict names a row `select_fanout` then refuses as not settled — a store
    /// error once the judge is `running`, which no scripted session can reach.
    struct SupersedeWinner<'a> {
        orch: &'a FakeOrchestrator,
        winner: i32,
    }

    impl crate::engine::SessionSink for SupersedeWinner<'_> {
        async fn after_done(
            &self,
            item: htui_core::model::ItemId,
            step: &RunStep,
            phase: &htui_core::model::SnapshotPhase,
            key: &crate::engine::SessionKey<'_>,
            done: &htui_agent::event::DoneEvent,
        ) -> Result<(), htui_core::store::StoreError> {
            if step.fanout_index < 0 {
                let steps = self.orch.store().run_steps(step.run_id).await?;
                if let Some(winner) = steps.iter().find(|row| {
                    row.position == step.position
                        && row.attempt == step.attempt
                        && row.fanout_index == self.winner
                        && row.status != StepStatus::Superseded
                }) {
                    self.orch.store().supersede_step(winner.id).await?;
                }
            }
            crate::engine::SessionSink::after_done(self.orch, item, step, phase, key, done).await
        }
    }

    /// Plan D51: a store error after the judge went `running` — here `select_fanout`
    /// refusing a winner that is no longer settled — lands the judge `failed` and parks the group,
    /// and the walk still raises the error. Before, the `?` left the judge and the run `running`.
    #[tokio::test]
    async fn a_store_error_after_the_judge_started_fails_it_and_parks() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        judge_both(&orch, "research", 1, 1, "best");
        let before: Vec<_> = orch
            .store()
            .runs(ids::HTUI_ANA_2)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|run| run.id)
            .collect();

        let graphs = orch.graphs();
        let driver =
            |_candidate: &htui_core::model::SnapshotCandidate,
             key: &crate::engine::SessionKey<'_>| orch.driver_for_key(key);
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let parts = crate::engine::fake_parts(&orch, &graphs, &driver, &scrubber)
            .await
            .expect("the fake's parts build");
        let sink = SupersedeWinner {
            orch: &orch,
            winner: 1,
        };
        let engine = crate::engine::Engine::new(crate::engine::EngineParts {
            store: parts.store,
            graphs: parts.graphs,
            isolator: parts.isolator,
            verifier: parts.verifier,
            clock: parts.clock,
            selector: parts.selector,
            sink: &sink,
            driver: parts.driver,
            scrubber: parts.scrubber,
            app: parts.app,
            box_profile: parts.box_profile,
            box_id: parts.box_id,
            owner: parts.owner,
            user: parts.user,
        });

        let err = engine
            .dispatch(crate::command::Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: htui_core::model::RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect_err("the refused selection is raised");
        assert!(
            matches!(
                err,
                crate::command::EngineError::Store(htui_core::store::StoreError::Constraint(_))
            ),
            "{err:?}"
        );

        let run = orch
            .store()
            .runs(ids::HTUI_ANA_2)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|run| run.id)
            .find(|id| !before.contains(id))
            .expect("the run was created");
        assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
        let steps = steps_of(&orch, run).await;
        let judge = judge_of(&steps, 0, 1).expect("the judge ran");
        assert_eq!(judge.status, StepStatus::Failed);
        let note = judge.gate_note.as_deref().unwrap_or_default();
        assert!(note.starts_with("judge_session_failed: "), "{note}");
    }
}
