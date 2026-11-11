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
//! per step, document kinds and their `produced_by_step_id`, the item's derived state and its
//! `closed_at`, the run's `finished_at` — and never merely that the walk did not error.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::TimeDelta;
use futures::future::Either;
use htui_agent::driver::AgentSessionRef;
use htui_agent::driver::{PermissionMatch, PermissionPolicy, PermissionRule, SessionSpec};
use htui_agent::event::PermissionOptionKind;
use htui_core::fixtures::ids;
use htui_core::model::GraphScope;
use htui_core::model::kind::HEAVY_COMMAND_PREFIXES;
use htui_core::model::{
    AgentBox, AgentId, BatchId, Billing, CommandRun, CommandRunStatus, DocumentId, EdgeId,
    EdgeKind, EventKind, Gate, GateOutcome, Isolation, Item, ItemId, ItemPatch, ItemState,
    NewDocument, NewRepo, NewStepGraph, PhaseId, PhasePatch, RepoId, Resolution, Run, RunId,
    RunMode, RunStatus, RunStep, Status, StepGraphEdge, StepGraphId, StepGraphPhase, StepId,
    StepRole, StepStatus, VerifyOutcome,
};
use htui_core::model::{
    BoxEdit, Claim, NewItem, OverlapRule, Quota, QuotaSource, RunKind, Scope, Spend, WorkspaceId,
};
use htui_core::model::{
    CommandQueue, GraphSnapshot, NewPersona, Persona, PersonaId, PersonaPatch, PersonaPermission,
    PersonaTools, SnapshotPersona,
};
use htui_core::model::{
    MemoryBody, MemoryEntry, MemoryEntryId, MemoryKind, MemoryPatch, MemorySubject,
    MemorySupersede, MemoryUpdate, NewMemoryEntry, ProjectId,
};
use htui_core::prompt::DiffBlock;
use htui_core::store::{
    CasOutcome, MemStore, ReadStore as _, StepFence, StoreError, WriteStore as _,
};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::command::{
    Command, CommandOutcome, EngineError, GateAnswer, OpeningPath, Rest, UnblockCase,
};
use crate::engine::{Adopted, Next, Resume, SessionKey, Tails};
use crate::fake::{
    FakeIsolator, FakeOrchestrator, FakeVerifier, IsolatorCall, RESTART_GAP, ScriptedStep,
    SessionHold, TestClock, rewrite_as_v1,
};
use crate::isolate::{ChangedPaths, Clock as _};
use crate::promote;
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
/// `store()` is a `MemStore` rather than an `impl WorkerStore` because a case asserts over rows and
/// the suite's only store is the in-memory one. The *engine* is what stays generic over
/// `S: WorkerStore`; nothing here weakens that.
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

    /// `Engine::claim` (plan D84): re-attempt a run a refusal left `queued`.
    ///
    /// Out of the blueprint's §4.5 list, and needed (blueprint F-J): criterion 15's first half
    /// ends with the refused run claimed once its holder is gone, and nothing else re-attempts a
    /// `queued` run.
    ///
    /// # Errors
    /// Whatever the engine refuses with.
    async fn claim(&self, run: RunId) -> Result<CommandOutcome, EngineError>;

    /// `Engine::enqueue_in_batch` (MOD-12 M2): the only way a case makes a batch run, which
    /// `Command::StartRun` never does. Claim it with [`claim`](Orchestrate::claim).
    ///
    /// # Errors
    /// Whatever the engine refuses with.
    async fn enqueue_in_batch(
        &self,
        item: ItemId,
        batch: BatchId,
        workspace: Option<WorkspaceId>,
    ) -> Result<RunId, EngineError>;

    /// A second orchestrator over the same store, as a process started after this one died
    /// (plan D118): a new `owner`, fresh isolator and verifier, a clock past every lease.
    #[must_use]
    fn restarted(&self) -> Self
    where
        Self: Sized;

    /// The lease owner this orchestrator writes (blueprint F-J), so a case can tell whose lease
    /// a run carries.
    fn owner(&self) -> Uuid;

    /// MOD-41 plan D12: who walks a command's tail from now on. [`Tails::HandBack`] is the TUI on
    /// a `worker` box: the window is written, the lease released, and the box's worker adopts.
    fn set_tails(&self, tails: Tails);

    /// `Engine::sweep` (plan D98): adopt every expired lease on the box, adjudicate, walk nothing.
    ///
    /// # Errors
    /// Whatever the adoption itself refuses with.
    async fn sweep(&self) -> Result<Vec<Adopted>, EngineError>;

    /// Make one session's `after_done` never return, after writing its document when
    /// `write_output` (plan D100, blueprint A-2, F-O): the crash every recovery case starts from.
    /// `slot` is `script_candidate`'s `(fanout_index, call)`, or `None` for any session of
    /// `(phase, attempt)`. Drive the walk with [`until_stalled`] and the returned signal.
    #[must_use]
    fn stall_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
        write_output: bool,
    ) -> Arc<Notify>;

    /// MOD-40 T2: [`stall_after_done`](Self::stall_after_done) with no document, as a
    /// **suspend** rather than a death — the session signals the first [`Notify`], then awaits
    /// the second and returns `Ok(None)`, and the walk settles on. A laptop lid, not a kill.
    #[must_use]
    fn suspend_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
    ) -> (Arc<Notify>, Arc<Notify>);

    /// Make stage 3 refuse `phase`'s own prompt (MOD-4 plan D162): its pinned template names a
    /// placeholder no role has, which is ANA-5 criterion 3's literal case.
    fn refuse_prompt(&self, phase: &str);

    /// Blueprint D203: write `step`'s output document of `kind`, as a promoted step's chat would.
    ///
    /// # Errors
    /// The store's own refusals.
    async fn author(
        &self,
        item: ItemId,
        step: &RunStep,
        kind: &str,
        body: &str,
    ) -> Result<(), StoreError>;

    /// MOD-11 D16, D17: the spec the session at `key` was started with — its exposure and its
    /// policy — or `None` before it started. Out of the blueprint's §4.5 list for the reason
    /// [`verifier`](Orchestrate::verifier) is: the exposure and the denials are not rows, and
    /// nothing else on this trait can say what the agent was handed.
    fn spec_for(&self, key: &SessionKey<'_>) -> Option<SessionSpec>;

    /// MOD-42 plan D9: `agent`'s own permission policy in every engine this orchestrator builds,
    /// so a case can tell the agent's rules from the ones the walk splices in (MOD-11 D17).
    fn set_policy(&self, agent: AgentId, policy: PermissionPolicy);

    /// MOD-11 R1 C: this orchestrator with a tool host lent to every engine it builds. Only a
    /// hosted engine offers `command_run`, so every case about the command queue's exposure starts
    /// here.
    #[must_use]
    fn hosted(self) -> Self;

    /// MOD-138 M3 T3 (F14): hold every session of the listed `(phase, attempt)`s before its first
    /// event until the hold is released or the session cancelled. A serial walk never has two
    /// held sessions live, so a case awaiting them runs under `tokio::time::timeout`.
    #[must_use]
    fn hold_sessions(&self, keys: &[(&str, i32)]) -> SessionHold;

    /// MOD-138 M3 T3: the most sessions live at once in this orchestrator's engines.
    fn live_sessions_high_water(&self) -> usize;

    /// MOD-138 M3 T3 (F14): every session yields once before its first event, so admitted node
    /// futures interleave.
    fn interleave_sessions(&self);

    /// MOD-139 M2 D16 (blueprint P-10): a queued run of `item`'s divided graph, resolved through
    /// the test seam that lifts only the divide refusal; claim it with
    /// [`claim`](Orchestrate::claim). Production resolve (`StartRun`) still refuses it.
    ///
    /// # Errors
    /// Whatever the seam or the store refuses with.
    async fn enqueue_divided(&self, item: ItemId) -> Result<RunId, EngineError>;

    /// MOD-42 plan D10: every walk of this orchestrator is asked to stop, `grace` given to each
    /// live session. Out of the blueprint's §4.5 list, and needed (MOD-139 M2 R1, F-7): a case
    /// that pins what a cancel does to a parts round has to send one while a part is live.
    fn cancel_walks(&self, grace: std::time::Duration);
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

    async fn claim(&self, run: RunId) -> Result<CommandOutcome, EngineError> {
        Self::claim(self, run).await
    }

    async fn enqueue_in_batch(
        &self,
        item: ItemId,
        batch: BatchId,
        workspace: Option<WorkspaceId>,
    ) -> Result<RunId, EngineError> {
        Self::enqueue_in_batch(self, item, batch, workspace).await
    }

    fn restarted(&self) -> Self {
        Self::restarted(self)
    }

    fn owner(&self) -> Uuid {
        Self::owner(self)
    }

    fn set_tails(&self, tails: Tails) {
        Self::set_tails(self, tails);
    }

    async fn sweep(&self) -> Result<Vec<Adopted>, EngineError> {
        Self::sweep(self).await
    }

    fn stall_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
        write_output: bool,
    ) -> Arc<Notify> {
        Self::stall_after_done(self, phase, attempt, slot, write_output)
    }

    fn suspend_after_done(
        &self,
        phase: &str,
        attempt: i32,
        slot: Option<(i32, u32)>,
    ) -> (Arc<Notify>, Arc<Notify>) {
        Self::suspend_after_done(self, phase, attempt, slot)
    }

    fn refuse_prompt(&self, phase: &str) {
        Self::refuse_prompt(self, phase);
    }

    async fn author(
        &self,
        item: ItemId,
        step: &RunStep,
        kind: &str,
        body: &str,
    ) -> Result<(), StoreError> {
        Self::author(self, item, step, kind, body).await.map(drop)
    }

    fn spec_for(&self, key: &SessionKey<'_>) -> Option<SessionSpec> {
        Self::spec_for(self, key)
    }

    fn set_policy(&self, agent: AgentId, policy: PermissionPolicy) {
        Self::set_policy(self, agent, policy);
    }

    fn hosted(self) -> Self {
        self.with_tool_host(Arc::new(crate::fake::FakeToolHost::default()))
    }

    fn hold_sessions(&self, keys: &[(&str, i32)]) -> SessionHold {
        Self::hold_sessions(self, keys)
    }

    fn live_sessions_high_water(&self) -> usize {
        Self::live_sessions_high_water(self)
    }

    fn interleave_sessions(&self) {
        Self::interleave_sessions(self);
    }

    async fn enqueue_divided(&self, item: ItemId) -> Result<RunId, EngineError> {
        crate::engine::enqueue_divided_fake(self, item).await
    }

    fn cancel_walks(&self, grace: std::time::Duration) {
        Self::cancel_walks(self, grace);
    }
}

/// Blueprint A-2: poll `fut` until `stalled` fires, then **drop it** where it stands — a process
/// killed mid-walk, leaving every row exactly as the walk last wrote it (plan D100).
///
/// Both futures are boxed so `select` sees `Unpin` and the dropped walk really goes (blueprint
/// H-11). `pub` so the real-git suite (`tests/gix_isolator.rs`) crashes its walks the same way.
///
/// # Panics
/// When `fut` finishes before the stall fires: the case scripted a stall the walk never reached.
pub async fn until_stalled<F: Future>(fut: F, stalled: &Notify) {
    match futures::future::select(Box::pin(fut), Box::pin(stalled.notified())).await {
        Either::Left(_) => panic!("the walk finished instead of stalling"),
        Either::Right(((), walk)) => drop(walk),
    }
}

/// Case names in run order. A name never changes: every binding reports per case.
///
/// One hundred, and the count is pinned in two places on purpose — here by
/// `cases_are_unique_and_counted` and out of crate by `tests/fake_conformance.rs` —
/// because a binding that silently ran one fewer of them would still be green.
///
/// Recounted, not appended: 18 + 5 + 13 + 6 + 10 + 18 + 2 + 1 + 1 + 1 + 11 + 1 + 5 + 1 + 1 + 6.
///
/// **Eighteen before milestone 4.** Six are `docs/ANA-2.md` §12's validation criteria (1, 2, 3,
/// 5, 6 and 7); four are contract lines §12 does not number but §4.2 states outright; one is the
/// `finish_run` seam T1 shipped for this walk to use; three are the cells of §4.2's gate table
/// nothing else reaches — plan D4's automatic loop entry and both halves of `on_failure` — which
/// D4 predicted would go unexecuted in a manual-mode milestone and did; one is plan D5's
/// intermediate position, which had a topology digest pinning it and no walk executing it; and
/// three are milestone 3's — the settle's two readings of a `verify_command` that ran (`fail`) and
/// one that could not (`unavailable`), and `CancelRun`, which is criterion 13's command half
/// (plan D45).
///
/// **Five for milestone 4's stage 1**: `R-AGT-8`'s walk selecting an `allowed_warning` row and
/// falling through a skipped one (plan D60, D61), both halves of the empty-candidate refusal (plan
/// D62), and the loop's forwarded `verify_failure` and `previous_diff` (plan D67).
///
/// **Thirteen for milestone 4's fan-out**: criterion 8 once, 9 twice and 10 twice (plan D49-D53,
/// D65); the gated and judgeless routes to a human (D49, D50); a failed candidate (D48); a group
/// with no survivor (D49(1)); the review loop over a group (D66); the two caps refused at
/// `StartRun` (D63); and a `shared_serialized` sibling's deadline, which counts from its own
/// `prepare` (D48).
///
/// **Six for milestone 5's lease and overlap**: criterion 15's three halves (plan D83, D84);
/// criterion 16, where overlap and the slot reach the walk; and the lease's two cross-process
/// answers — a park releases it for another process's answer, and a live one refuses that answer
/// before anything is written (plan D87, D108).
///
/// **Ten for milestone 5's recovery sweep**, every one a crash the stall harness makes (blueprint
/// A-2) and a second process's sweep: criterion 18's two halves, a finished step through its gate
/// and an unfinished one reset and retried (plan D90-D92); the out-of-budget park a human may
/// still retry (D92, blueprint A-8); criterion 12's dirty tree, never reset (D93); two lost
/// merges re-reconciled once, the second after a review rejection (D97, C129); an interrupted
/// candidate and an interrupted judge (D95); a half-written park (D96); and the runs no sweep
/// adopts — a parked one, a live lease, and this process's own (D88).
///
/// **Eighteen for milestone 6** (MOD-4 plan D159-D167, D179, D211): ANA-5 criterion 3's refused
/// prompt at `walk_step` and at `drive_group`; criterion 3's running run that parks on a topology
/// mismatch; a cancel that meets a live lease; four promotions (criterion 17's first half, a
/// failed step of a parked run, a finished run refused, a dropped running step parked); five
/// accepts (criterion 17's second half, the promotion and the document it needs, a failed
/// verify, a verify on a deadline measured from the accept, an `unavailable` verify noted);
/// `Unblock`'s three cases (rung 4's reopen, R-4's escalated run, R-7's refused reconcile); and
/// criterion 20's close-out and its refusal while a run is live.
///
/// **Two for MOD-7 milestone 3** (plan D86, D87): criterion 14's capability half, a refusal at
/// `StartRun` that writes no run and that `Unblock` reopens, and ANA-2 §4.10's claim-time half, a
/// run failed by name when its tag was withdrawn after enqueue.
///
/// **One for MOD-40 milestone 1** (plan D1, D2): a suspended walk, woken after another process
/// adopted its run, writes nothing to the step.
///
/// **One for CLEAN-4** (plan D4): criterion 7's review half, which a reader that saw only the
/// latest version per kind had left unreachable: two identical reviews stop the loop on
/// `no_progress_review` with no repo in scope.
///
/// **One for MOD-41 T1** (plan D1, D2): a walk woken after its session's `Done`, whose run
/// another process adopted, records no commits.
///
/// **Eleven for MOD-41 T9** (plan D12, OQ-1, OQ-6): each walking command handed back on a
/// `worker` box — approve, reject (loopable and terminal), select, retry of a failed step, resume,
/// accept, retry of an awaiting step, group retry, retry past the budget — adopted and walked by
/// the box's worker to the in-process rest; the changed-graph park the crash path's topology gate
/// makes; and the hand-back's window, which writes no step past itself.
///
/// **Five for MOD-26 milestone 1** (plan D9, D12, D13, OQ-2): the persona frame of a bound
/// phase, a persona edit that does not reach a started run, the judge without a persona, a
/// persona without `command_run` dropping the `command_queue` section, and the agent row a
/// persona-bound run leaves untouched.
///
/// **One for MOD-37 milestone 4** (R-49, closed by admission): a promoted `shared_serialized`
/// step's parked run refuses another run's claim on its repo with rule I.
///
/// **One for MOD-73** (plan D1, D2): a version of the parked step's output kind written by hand
/// at the gate, then approved, is what the next phase's prompt carries.
///
/// **Six for MOD-11 T8** (plan D16, D17, OQ-6): `fan_out_only` without a fan-out or a
/// `heavy_build` item exposes nothing, a fan-out of two and a `heavy_build` item do; an exposed
/// step's policy opens with the R-MCP-4 denials with and without a persona; a judge never gets
/// `command_run`.
///
/// **Three for MOD-12 M1** (plan D10; ANA-2 criteria 23, 24): an auto run snapshots every phase
/// whose gate is not hard `never` and walks it to `done` with no human, an auto run still parks at
/// a hard gate, and a manual snapshot keeps every gate.
///
/// **Four for MOD-12 M2** (plan D5, D6; PRD metric "batch spend overshoot"): a batch at its cap
/// starts no attempt, a batch run's session is cut at the batch's remainder, a manual run reads no
/// batch figure, and the run cap spans steps.
///
/// **Six for MOD-138 M1** (plan D5, blueprint A6): a v2 run is pinned, so a live edit parks it on
/// none of the three resume paths (mid-walk, adopted `running`, handed back); a v2 snapshot that
/// no longer hashes to its own `topology` parks with both digests; a v1 run whose live graph
/// gained a non-default edge walks on; and a v1 run on a refused live graph whose phases changed
/// parks with the topology note.
///
/// **Three for MOD-144** (ANA-35 §7.1): a queued cancel writes no item, a stopped run holds its
/// item, and a rework run on a finished item is admitted.
///
/// **Nine for MOD-138 M2 T4** (plan D7, D10, Q-D; blueprint A3, A6, F8): loops follow edges, not
/// names. A `changes_requested` edge loops a node not called `review`, and a `review` with no
/// such edge passes its verdict unread; a `fails` edge loops on the node's own failure retiring
/// its region, on a human's reject, and in `settle_failed`'s replay of a loop a crash cut short,
/// and escalates once spent; a loop on a diamond retires only the nodes on a path from its
/// target to its source; no progress stops an edge loop before its `loop_limit`; and a graph
/// with authored edges resolves (D6(c) lifted).
///
/// **Thirteen for MOD-138 M2 T5** (plan D6, D7, D8, Q-A, Q-B; blueprint A2, A3, A5, A9; the
/// thirteenth, past blueprint §6's twelve, came with T5's first repair): `if` edges
/// and `asks_you` edges. Each predicate of the closed set decides as specified and the decision
/// is stored once on the source's `branch`; a declined branch is skipped and the join after it
/// runs; an isolator error parks the branch undecided and `Unblock` resumes it; a node with an
/// `asks_you` edge always stops (auto mode included), approve takes its edges and reject or
/// accept artifact declines them with the work landed; a back `asks_you` approved loops, a crash
/// between the approve and its loop is replayed, and a spent one escalates without blocking the
/// item and walks on when approved; a decision is read from its row after a restart; a reject on
/// a node that also reads its verdict loops over `changes_requested`; and a reject of a failed
/// settle's park on an `asks_you` node is that gate's own and ends the run (D8).
///
/// **Seventeen for MOD-138 M3 T3** (plan D1-D5; blueprint A2-A6, A11; the T1 verify's owed
/// selection): concurrent nodes. A chain's isolator calls are unchanged; incomparable branches
/// are live at once up to `max_parallel_sessions`, a heavier node alone; a join waits for both
/// landings; worktree branches both land, in-place ones never overlap anything, and an owed
/// selection lands before an in-place node; a park drains its siblings, a sibling's park on the
/// parked run parks its step only, and one answer of two parks re-parks; a node ending the run
/// cuts its live sibling, an end after a park cancels the parked step, and the cleanup waits for
/// the drain; a loop on one branch retires only
/// its region; and an owed loop runs before its comparable successor.
///
/// **Fourteen for MOD-138 M3 T4** (plan D6; blueprint A7-A10, the hand-back twin of T3's
/// repair 2, T4 repair 1's two and T4 repair 2's one): recovery among concurrent landings. Every done winner that may not have landed is
/// re-landed, in topological order, by the sweep, by a crash-cut unpark's resume and, before its
/// own step, by an answer (A10): two unlanded branches, two answered parks, the node a reordered
/// chain walked last, a done branch beside a still running one, an already landed branch
/// (reconciled again), only the later of two in-place landings, and a loop's re-run target beside
/// its side successor's older row. Every running row is adjudicated, and running branches
/// recovered into parks park each without `StaleWrite`. A rejection handed back beside another
/// parked step ends the run and cancels that step, as the in-process answer does. And T4
/// repair 1's two: a looping rejection, answered or resumed, lands a crashed answer's winner
/// first. And T4 repair 2's one: a looping rejection handed back walks its loop as the in-process
/// answer does.
///
/// **Six for MOD-138 M3 R1** (the review's findings, maintainer 2026-11-08). Five for finding 1's
/// input order ("order by inputs"): a node listing a kind an incomparable node produces waits for
/// that producer to finish, whichever position is lower; a two-way pair runs the lower position
/// first, a cycle of waits its lowest once nothing else can run, and a skipped producer leaves the
/// kind optional. And finding 6's one: a rejection handed back beside an A7 recovery's
/// interrupted park ends the run as the in-process answer does.
///
/// **Six for MOD-163 M2** (plan D11, D12, D18): the project memory frozen at `StartRun` reaches
/// every phase step of its project and no other project's; retired and superseded entries never
/// reach a prompt; an amend and a retire after `StartRun` do not reach the run (the next step
/// and a retry render the run-start bytes); fan-out siblings carry byte-identical memory and the
/// judge none; a pre-MOD-163 snapshot resumes with no memory section; and a project without
/// memory writes no `memory` row.
///
/// **Eighteen for MOD-139 M2 T3** (plan D1-D5, D7, D8, D10, D13, D15-D17): divided nodes. Built
/// through the D16 resolve seam, since production resolve still refuses them (the last case
/// pins both refusals, and D17's planner agent first). A divided node admits its default parts
/// as plan revision 0 from the primary's `base_refs`; each part runs in its own tree from those
/// refs with its own prompt section, agent and session, and never lands; part documents stay out
/// of input resolution; the integrate node is offered only once every part is done (where an M2
/// run stops, D15); the parts share the session cap, run one at a time under a cap of one or
/// under `shared_serialized` (which resets to the refs), and a lone `local` part prepares with no
/// slot; `local` with two parts, an agent outside the candidates, a part persona, an over-cap and
/// an empty plan each fail admission; `command_run` reaches a part beside a live sibling and a
/// parallel branch beside a live one; the operator verbs refuse a part row; and stage rows are
/// invisible to the walk.
///
/// **Nine for MOD-139 M2 T4** (plan D6, D11, D14): a part that fails is retried alone from the
/// plan's `base_refs`; a part out of attempts fails the run `part_exhausted`, or takes the divided
/// node's `fails` edge, which retires the planner's parts, abandons its plan and admits the next
/// revision; and recovery per role: a crash before admission admits on the same planner row, one
/// after it re-plans nothing and lands no part, a crashed part with its output is `done`, one
/// without is reset and retried alone, one in a dirty `local` tree parks, and a `local` divided
/// node lands no part.
///
/// **Six for MOD-139 M2 R1** (F-7, F-8, blueprint §T4.3): one part at a time, a cancel or a lost
/// lease while the first part is live leaves the second `pending`, never prepared and with no
/// session, and a refusal of the first ends the run with the second `cancelled`; a `failed`
/// planner on recovery (an admission refusal a crash cut short) is neither retried nor looped,
/// whatever its budget: the run ends on `retry_budget_spent`; and a `fails` loop's retirement of
/// a planner slot cut short (by a store error at a part, or before the planner row) replays on
/// the resumed walk.
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
    // CLEAN-4 plan D4: criterion 7's other half, two identical review bodies with no repo in
    // scope, stop the loop on `no_progress_review` before its budget does.
    "identical_reviews_stop_the_loop",
    // `docs/ANA-2.md:414`: a missing required input fails the run before a token is spent.
    "missing_input_fails_before_a_token",
    // `:430`: a step that produced no document of its `output_kind` settles `failed`.
    "missing_output_settles_failed",
    // `:482`: a gated phase whose only candidate cannot answer a permission request is refused
    // with `missing_capability: inline_approval`.
    "cli_agent_is_refused_at_a_gated_phase",
    // `:439` and plan D8: a step that outlives its deadline settles `failed`.
    "step_deadline_settles_failed",
    // Plan D7: the last write of a finished walk is `finish_run`, and the item reads it (MOD-144).
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
    // ANA-2 §12 criterion 14 (`:2121-2122`, MOD-7 milestone 3 D86): a capability refusal writes
    // no run, blocks the item with a note naming exactly the missing tags, and `Unblock` reopens
    // it.
    "a_capability_refusal_writes_no_run_and_unblock_reopens",
    // ANA-2 §4.10's claim-time half (MOD-7 milestone 3 D87): a tag withdrawn between enqueue and
    // claim fails the run by name, before the overlap.
    "a_capability_refusal_at_claim_fails_the_run_by_name",
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
    // CLEAN-9 T2: a malformed cap key refuses the start rather than running uncapped.
    "a_malformed_cap_is_refused_at_start",
    // Plan D63 (OQ-1): `max_agents_per_run` is refused at `StartRun`.
    "max_agents_per_run_is_refused_at_start",
    // Plan D48 under `shared_serialized`: a candidate's deadline counts from its own `prepare`,
    // not from the siblings it queued behind.
    "a_serialized_sibling_is_not_charged_for_the_ones_before_it",
    // Milestone 5, criterion 15 (`:2125`, plan D83, D84): two runs declaring `src/**` on one
    // repo serialise, and `Engine::claim` walks the second once the first is gone.
    "overlapping_touched_paths_serialise",
    // Criterion 15 (`:2125-2126`): the same paths in two repos run concurrently.
    "the_same_paths_in_two_repos_run_concurrently",
    // Criterion 15 (`:2126-2127`): an undeclared item holds its whole primary repo.
    "an_undeclared_item_holds_its_whole_primary_repo",
    // Criterion 16 (`:2128-2129`): a parked run holds no slot but still blocks an overlap, and a
    // full box refuses before any overlap is read.
    "a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap",
    // Plan D87: a park releases the lease, and another process's answer takes it.
    "a_parked_run_releases_its_lease_and_an_answer_takes_it",
    // Plan D87, D108: a live lease refuses another process's answer before its first write.
    "a_live_lease_blocks_an_answer_from_another_process",
    // Milestone 5, criterion 18 (`:2135-2136`, plan D90, D91): a finished step is adopted through
    // the gate it would have met.
    "a_finished_step_is_adopted_through_its_gate",
    // Criterion 18 (`:2136-2137`, plan D92): an unfinished step is reset and retried.
    "an_unfinished_step_is_reset_and_retried",
    // Plan D92 out of budget and blueprint A-8: the run parks, and `RetryStep` still admits.
    "an_interrupted_step_out_of_budget_parks",
    // Criterion 12 (`:2118-2119`, plan D93), the fake's half: a dirty tree is never reset.
    "a_dirty_tree_is_never_reset",
    // Plan D97 (blueprint F-A, A-3): a winner whose merge was lost is reconciled once.
    "a_crash_before_reconcile_is_reconciled_on_adoption",
    // Plan D97 as redefined (C129): the frontier ignores a review rejection's retired rows.
    "a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled",
    // Plan D95: an interrupted candidate fails alone.
    "an_interrupted_candidate_fails_alone",
    // Plan D95: an interrupted judge parks the slot for a human.
    "an_interrupted_judge_parks_for_selection",
    // Plan D96: a half-written park is completed.
    "a_half_written_park_is_completed",
    // `:1306` and plan D88: a parked run, a live lease and this process's own are never adopted.
    "the_sweep_never_touches_a_parked_run_or_a_live_lease",
    // MOD-4 milestone 6, ANA-5 criterion 3 (`docs/ANA-5.md:2271-2273`, plan D162) at `walk_step`:
    // a refused prompt fails the step, blocks the item and starts no session.
    "a_prompt_refusal_blocks_the_item_and_starts_no_session",
    // The same at `drive_group` (blueprint D195): every candidate fails, and the item is blocked.
    "a_prompt_refusal_in_a_fan_out_blocks_the_item",
    // Criterion 3 (`:2090-2091`, plan D159): a `running` run whose graph moved parks on resume.
    "a_topology_mismatch_parks_a_running_run",
    // R-28 (plan D179): a cancel meets a live lease elsewhere and writes nothing.
    "cancel_meets_a_live_lease_and_writes_nothing",
    // Criterion 17, first half (`:2131-2134`, plan D163): a promotion keeps the step's row and
    // writes no chat run.
    "promote_keeps_the_step_and_writes_no_chat_run",
    // `:1563`: a failed step under a parked run is promotable.
    "promote_a_failed_step_of_a_parked_run",
    // `:1563`: a finished run has nothing to promote, and nothing is written.
    "promote_refuses_a_terminal_run",
    // Plan D163's engine half: a step a dropped walk left `running` is parked by the promotion.
    "promote_moves_a_dropped_running_step_to_awaiting",
    // Criterion 17, second half (`:2131-2134`, plan D166): accept verifies, captures, approves
    // and walks on at `position + 1`.
    "accept_artifact_verifies_captures_and_resumes_at_the_next_position",
    // `:1229-1231`: accept needs the promotion, the document and no live chat on the step.
    "accept_artifact_needs_the_document_and_the_promotion",
    // Blueprint D194: a failed verify refuses the accept and keeps the step promoted.
    "accept_artifact_refuses_a_failed_verify",
    // MOD-4 plan D211 (review H2): the accept's verify runs on a fresh phase deadline measured
    // from the accept, even past the step's own.
    "accept_artifact_verifies_on_a_deadline_from_the_accept",
    // MOD-4 plan D211: an `unavailable` verify is recorded and noted, not refused.
    "accept_artifact_notes_an_unavailable_verify",
    // Rung 4's `Unblock` round trip (plan D161): its blocked item reopens. Criterion 14 itself is
    // MOD-7 milestone 3's case above.
    "unblock_opens_a_blocked_item_with_no_run",
    // R-4 (plan D161 case 2): an escalated item follows its parked run, which is then promoted
    // and approved.
    "unblock_lets_an_escalated_run_be_promoted_and_approved",
    // R-7 (plan D161 case 3): a run parked by a refused reconcile is resumed.
    "unblock_resumes_a_reconcile_refused_park",
    // Criterion 20 (`:2141-2143`, plan D167): close-out writes one summary and closes the item.
    "close_out_writes_one_summary_and_closes_the_item",
    // Criterion 20: close-out is refused while a run of the item is live, and writes nothing.
    "close_out_is_refused_while_a_run_is_live",
    // MOD-40 plan D1, D2: a walk that wakes after another process adopted its run writes nothing
    // to the step.
    "a_suspended_walk_cannot_write_after_adoption",
    // MOD-41 plan D1, D2: a walk woken after its session's `Done`, whose run another process
    // adopted, records no commits: its capture is fenced.
    "a_walk_woken_after_done_records_no_commits",
    // MOD-41 plan D12, OQ-1, R-1: each walking command handed back on a `worker` box, adopted and
    // walked by the box's worker, rests the run where the in-process tail did.
    "an_approval_handed_back_is_walked_by_the_adopter",
    "a_rejection_handed_back_is_walked_by_the_adopter",
    "a_selection_handed_back_is_walked_by_the_adopter",
    "a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter",
    "a_resume_handed_back_is_walked_by_the_adopter",
    // MOD-37 R-31 (review L1): `u` handed back over a rejection a crash left parked unparks it,
    // and the adopter owes the rejection (plan D131).
    "a_crashed_rejection_handed_back_is_failed_by_the_adopter",
    "an_accept_handed_back_is_walked_by_the_adopter",
    "a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter",
    "a_group_retry_handed_back_is_walked_by_the_adopter",
    "a_retry_past_the_budget_handed_back_is_walked_not_reparked",
    // MOD-41 OQ-6 (blueprint F-24): the adopter's tail is the crash path, topology gate and all.
    "a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note",
    // MOD-41 plan D12: a hand-back writes its window and nothing past it.
    "a_hand_back_writes_no_step_past_the_window",
    // MOD-26 D13: a persona-bound phase's prompt opens with the persona frame, `sections[1]`.
    "a_persona_bound_phase_prompt_carries_the_persona_section",
    // MOD-26 I-3 (D9): a persona edited after `StartRun` does not reach the run.
    "a_persona_edit_after_start_run_does_not_reach_the_run",
    // MOD-26 OQ-2: the judge runs without a persona; its task replays the candidates' frame.
    "the_judge_step_runs_without_a_persona",
    // MOD-26 D13: a persona without `command_run` drops the `command_queue` section.
    "a_persona_without_command_run_drops_the_command_queue_section",
    // MOD-26 PRD metric: a persona-bound run leaves the agent row untouched.
    "a_persona_run_leaves_the_agent_row_untouched",
    // MOD-37 M4 (R-49, closed by admission): a promoted `shared_serialized` step's parked run keeps
    // every other run off its repo through `claim_run`'s rule I.
    "a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo",
    // MOD-73 plan D1, D2 (ANA-2 §4.2 as amended): a version written by hand at a gate, then
    // approved, is the next phase's stage-3 input.
    "a_gate_edit_is_what_the_next_phase_reads",
    // MOD-11 D16, OQ-6: `fan_out_only` exposes `command_run` (and renders its section) only to a
    // fanned-out step or a `heavy_build` item.
    "a_single_fan_out_fan_out_only_step_has_no_command_queue_section",
    "a_fan_out_two_step_has_it",
    "a_heavy_build_item_has_it",
    // MOD-11 D17: an exposed step's policy opens with the R-MCP-4 denials, persona or not.
    "a_persona_less_exposed_step_gets_the_denials_first",
    "a_persona_step_gets_the_denials_first_too",
    // MOD-11 D16: the judge's phase is `off`; a judge never gets `command_run`.
    "judges_never_get_command_run",
    // MOD-12 M1 (plan D10; ANA-2 criteria 23, 24): an auto run skips soft gates, parks at hard ones,
    // and a manual snapshot keeps every gate.
    "auto_mode_skips_soft_gates",
    "auto_mode_parks_at_a_hard_gate",
    "a_manual_snapshot_keeps_its_gates",
    // MOD-12 M2 (plan D5, D6; PRD metric "batch spend overshoot"): a batch at its cap starts no
    // attempt, a batch run's session is cut at the batch's remainder, a manual run reads no batch
    // figure, and the run cap spans steps.
    "a_batch_at_its_cap_starts_no_attempt",
    "a_batch_run_session_is_capped_at_the_batch_remainder",
    "a_manual_run_ignores_batch_figures",
    "the_run_cap_spans_steps",
    // MOD-12 M2 R1 (review M1): the session's run term is the snapshot's cap, as the walk's is.
    "a_lowered_live_run_cap_does_not_cut_a_snapshotted_run",
    // MOD-138 M1 (plan D5): a v2 run is pinned, so a live edit parks none of the three resume
    // paths; a tampered v2 snapshot parks. A v1 run whose live graph M1 refuses is compared on
    // its phases alone (blueprint A6): an added edge walks on, an edited phase parks.
    "a_v2_run_walks_on_after_a_live_edit",
    "a_running_v2_run_walks_on_after_a_live_edit",
    "a_handed_back_v2_run_on_a_changed_graph_walks_on",
    "a_tampered_v2_snapshot_parks_with_both_digests",
    "a_v1_run_whose_live_graph_gained_a_non_default_edge_walks_on",
    "a_v1_run_on_a_refused_graph_whose_phases_changed_parks",
    // MOD-144 (ANA-35 §7.1): the item's state is derived from its latest graph run. A queued
    // cancel writes no item; a stopped run holds its item against every verb; a finished item
    // admits a rework run, and cancelling it leaves the item finished.
    "a_queued_cancel_writes_no_item",
    "a_stopped_run_holds_its_item",
    "a_rework_run_on_a_finished_item_is_admitted",
    // MOD-138 M2 T4 (plan D7, D10, Q-D; blueprint A3, A6, F8): loops follow edges, not names. A
    // `changes_requested` edge loops a node not called `review`, and a `review` without one
    // passes unread; a `fails` edge loops on a failure, on a human's reject and in a crash's
    // replay, and escalates out of budget; a loop on a diamond retires only its region; no
    // progress stops an edge loop; and a graph with authored edges resolves.
    "a_changes_requested_edge_loops_a_node_not_named_review",
    "a_review_with_no_changes_requested_edge_passes_unread",
    "a_fails_edge_loops_and_retires_its_region",
    "a_human_reject_of_a_fails_node_loops",
    "a_fails_loop_cut_short_is_replayed_by_settle_failed",
    "a_spent_fails_loop_escalates",
    "a_loop_on_a_diamond_retires_only_its_region",
    "an_edge_loop_without_progress_escalates",
    "a_graph_with_authored_edges_resolves",
    // MOD-138 M2 T5 (plan D6, D7, D8, Q-A, Q-B; blueprint A2, A3, A5, A9): `if` edges are decided
    // once from their facts and stored, a declined branch is skipped and its join still runs, an
    // undecided branch parks and resumes; an `asks_you` node always stops, approve takes its
    // edges and reject or accept declines them with the work landed, auto mode keeps the stop; a
    // back `asks_you` loops on approve, is replayed after a crash and escalates once spent; a
    // decision is read from its row after a restart; a verdict-reading `asks_you` node's reject
    // loops; and a reject of a failed settle's park on an `asks_you` node is the ordinary one.
    "each_if_predicate_decides_from_its_facts",
    "a_declined_if_branch_is_skipped_and_the_join_runs",
    "an_undecided_branch_parks_on_an_isolator_error_and_unblock_resumes_it",
    "a_forward_asks_you_approved_runs_its_target",
    "a_forward_asks_you_rejected_skips_its_target_and_lands_the_work",
    "auto_mode_never_downgrades_an_asks_you_stop",
    "a_back_asks_you_approved_loops",
    "a_crash_between_an_asks_you_approve_and_its_loop_is_replayed",
    "a_spent_back_asks_you_escalates_and_approve_walks_on",
    "a_branch_decision_is_read_from_its_row_after_a_restart",
    "accepting_an_artifact_on_an_asks_you_node_declines_its_edges",
    "a_reject_on_a_verdict_reading_asks_you_node_loops",
    "a_reject_of_a_failed_park_on_an_asks_you_node_ends_the_run",
    // MOD-138 M3 T3 (plan D1, D3, D4; blueprint A2-A6, A11): concurrent nodes.
    "a_linear_graph_keeps_its_call_order",
    "two_independent_branches_are_live_at_once",
    "three_branches_under_a_cap_of_two_never_run_three",
    "a_fan_out_heavier_than_the_cap_runs_alone",
    "a_join_waits_for_both_predecessors_to_land",
    "two_worktree_landings_into_one_repository_both_land",
    "shared_serialized_branches_never_overlap",
    "an_in_place_node_never_overlaps_a_worktree_node",
    "an_owed_selection_lands_before_an_in_place_node",
    "a_park_on_one_branch_drains_the_other",
    "a_sibling_parking_on_a_parked_run_parks_its_step_only",
    "one_answer_of_two_parks_re_parks_without_walking_on",
    "a_node_ending_the_run_cuts_its_live_sibling",
    "a_park_then_an_end_cancels_the_parked_step",
    "a_terminal_branch_defers_cleanup_until_its_sibling_settles",
    "a_loop_on_one_branch_retires_only_its_region",
    "a_loop_owed_beside_its_successor_runs_first",
    // MOD-138 M3 T3 repair 1 (blueprint §3.6 step 4, §3.10, A4).
    "a_terminal_branch_defers_cleanup_until_its_sibling_is_cut",
    "a_terminal_end_cancels_a_sibling_row_created_beside_it",
    // MOD-138 M3 T3 repair 2 (blueprint §3.6 step 4 on the answer path).
    "a_rejection_ending_the_run_cancels_the_other_parked_step",
    // MOD-138 M3 T4 (plan D6; blueprint A7, A8, A9): recovery among concurrent landings.
    "a_crash_with_two_unlanded_branches_lands_both_in_order",
    "a_crash_on_a_reordered_chain_lands_the_node_walked_last",
    "a_crash_beside_a_running_branch_lands_the_done_one",
    "a_landed_branch_without_a_successor_row_is_re_landed",
    "of_two_shared_serialized_landings_only_the_later_is_re_landed",
    "a_crash_with_two_running_branches_adjudicates_both",
    "running_branches_recovered_into_parks_park_each",
    "a_re_run_loop_target_is_re_landed_beside_an_older_side_successor",
    "a_crash_cut_unpark_on_a_branching_run_lands_every_frontier",
    "answering_one_of_two_parks_lands_a_crashed_answers_winner",
    "a_rejection_handed_back_cancels_the_other_parked_step",
    "a_looping_rejection_lands_a_crashed_answers_winner_first",
    "a_resumed_looping_rejection_lands_a_crashed_answers_winner_first",
    // MOD-138 M3 T4 repair 2: the looping twin of the hand-back above.
    "a_looping_rejection_handed_back_walks_its_loop_as_in_process",
    // MOD-138 M3 R1 (review finding 1, "order by inputs"): a node listing a kind an incomparable
    // node produces waits for it, either way round; a two-way pair runs the lower position first,
    // a cycle of waits its lowest, and a skipped producer leaves the kind optional.
    "a_consumer_waits_for_an_incomparable_producer_of_its_input",
    "a_lower_position_waits_for_a_higher_siblings_input",
    "a_two_way_input_pair_runs_the_lower_position_first",
    "a_cycle_of_input_waits_runs_its_lowest_position_first",
    "a_skipped_producer_leaves_its_kind_optional",
    // MOD-138 M3 R1 (review finding 6, T4's leftover): the hand-back twin of a rejection beside
    // an A7 recovery park ends the run as the in-process answer does.
    "a_rejection_handed_back_beside_an_interrupted_park_ends_the_run_as_in_process",
    // MOD-163 M2 (plan D11, D12, D18): the project's active memory, frozen at `StartRun`, reaches
    // every phase step of its project and only them; closed entries stay out; an edit after
    // `StartRun` does not reach the run; fan-out siblings carry the same bytes; a pre-MOD-163
    // snapshot resumes with none; and a project without memory writes no row.
    "a_memory_entry_reaches_every_phase_prompt_of_its_project",
    "closed_memory_entries_never_reach_a_prompt",
    "a_memory_edit_after_start_run_does_not_reach_the_run",
    "fan_out_siblings_carry_byte_identical_memory_sections",
    "a_snapshot_without_memory_resumes_with_no_memory_section",
    "a_project_without_memory_writes_no_memory_row",
    // MOD-139 M2 T3 (plan D1-D5, D7, D8, D10, D13, D15-D17): divided nodes, admission and parts.
    "a_divided_node_admits_its_default_parts_as_revision_zero",
    "each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent",
    "part_documents_stay_out_of_input_resolution",
    "integrate_is_offered_only_once_every_part_is_done",
    "two_parts_are_live_at_once_under_the_cap",
    "max_parallel_sessions_one_runs_the_parts_one_at_a_time",
    "shared_serialized_parts_reset_to_the_base_refs",
    "a_single_local_part_prepares_with_no_slot",
    "local_with_two_parts_fails_admission",
    "a_part_agent_outside_the_candidates_fails_admission",
    "a_part_persona_fails_admission",
    "an_over_cap_plan_fails_admission",
    "an_empty_divided_node_fails_admission",
    "a_part_with_a_live_sibling_gets_command_run",
    "a_fan_out_only_branch_beside_a_live_sibling_gets_command_run",
    "the_operator_verbs_refuse_a_part_row",
    "stage_rows_are_ignored_by_the_walk",
    "start_run_refuses_a_divided_graph_and_first_a_planner_agent",
    // MOD-139 M2 T4 (plan D6, D11, D14): part retries, the exhausted divided step, loops over a
    // divided node, and recovery per role.
    "a_failed_part_retries_alone_from_the_base_refs",
    "an_exhausted_part_fails_the_run_with_part_exhausted",
    "an_exhausted_part_takes_the_fails_edge_and_re_plans",
    "a_crash_before_admission_re_admits_on_the_same_planner_row",
    "a_crash_after_admission_re_plans_nothing_and_lands_no_part",
    "a_crashed_part_with_its_output_is_done_on_recovery",
    "a_crashed_part_without_output_retries_alone",
    "a_crashed_part_in_a_dirty_local_tree_parks",
    "a_local_divided_node_lands_no_part_on_recovery",
    // MOD-139 M2 R1 (F-7, blueprint §T4.3): a parts round stops pulling queued parts after a
    // cancel, a lost fence or a refusal, and a `failed` planner on recovery repeats nothing.
    "a_cancel_during_a_part_leaves_the_queued_part_pending",
    "a_lost_lease_during_a_part_leaves_the_queued_part_pending",
    "a_part_refusal_cancels_the_queued_parts",
    "a_failed_planner_on_recovery_ends_on_the_retry_budget",
    // MOD-139 M2 R1 (F-8, blueprint A6): a `fails` loop's retirement a store error or a crash
    // cut short replays on the resumed walk.
    "a_cut_short_part_retirement_replays_the_loop",
    "a_planner_left_done_replays_the_loop",
];

/// Run one case by name.
///
/// # Panics
/// On a name [`CASES`] holds and this `match` does not — the drift `every_case_name_dispatches`
/// exists to catch — and on any assertion the case itself makes.
pub async fn run_case<H: CaseHarness>(name: &str, harness: &H) {
    case(name, harness).await;
}

/// The named case's future, boxed.
///
/// A plain function rather than a `match` inside [`run_case`]'s own body, and that is about the
/// stack, not style: an unoptimised build gives every arm's case future its own stack slot, so a
/// seventy-four-arm `match` in an `async fn` puts all seventy-four in the one frame every case is
/// then polled beneath, and the recovery cases' walks overflowed a test thread's 2 MiB. Here the
/// slots are gone before the first poll.
///
/// # Panics
/// On a name [`CASES`] holds and this `match` does not.
fn case<'a, H: CaseHarness>(name: &str, harness: &'a H) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
    hand_back_case(name, harness)
        .or_else(|| persona_case(name, harness))
        .or_else(|| memory_case(name, harness))
        .or_else(|| hardening_case(name, harness))
        .or_else(|| input_case(name, harness))
        .or_else(|| command_queue_case(name, harness))
        .or_else(|| auto_mode_case(name, harness))
        .or_else(|| spend_guard_case(name, harness))
        .or_else(|| pinned_resume_case(name, harness))
        .or_else(|| item_state_case(name, harness))
        .or_else(|| graph_walker_case(name, harness))
        .or_else(|| branch_case(name, harness))
        .or_else(|| concurrency_case(name, harness))
        .or_else(|| concurrent_recovery_case(name, harness))
        .or_else(|| review_fix_case(name, harness))
        .or_else(|| divided_case(name, harness))
        .or_else(|| divided_recovery_case(name, harness))
        .unwrap_or_else(|| earlier_case(name, harness))
}

/// [`case`] for every name before MOD-41 T9's.
///
/// # Panics
/// On a name [`CASES`] holds and neither this `match` nor any of the frames [`case`] tries first
/// does.
fn earlier_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
    match name {
        "feat_walks_end_to_end" => Box::pin(feat_walks_end_to_end(harness)),
        "live_run_ignores_a_gate_edit" => Box::pin(live_run_ignores_a_gate_edit(harness)),
        "topology_mismatch_parks_on_resume" => Box::pin(topology_mismatch_parks_on_resume(harness)),
        "approve_needs_the_output_document" => Box::pin(approve_needs_the_output_document(harness)),
        "review_rejection_loops_then_escalates" => {
            Box::pin(review_rejection_loops_then_escalates(harness))
        }
        "identical_after_hash_stops_the_loop" => {
            Box::pin(identical_after_hash_stops_the_loop(harness))
        }
        "identical_reviews_stop_the_loop" => Box::pin(identical_reviews_stop_the_loop(harness)),
        "missing_input_fails_before_a_token" => {
            Box::pin(missing_input_fails_before_a_token(harness))
        }
        "missing_output_settles_failed" => Box::pin(missing_output_settles_failed(harness)),
        "cli_agent_is_refused_at_a_gated_phase" => {
            Box::pin(cli_agent_is_refused_at_a_gated_phase(harness))
        }
        "step_deadline_settles_failed" => Box::pin(step_deadline_settles_failed(harness)),
        "finish_run_is_the_last_write" => Box::pin(finish_run_is_the_last_write(harness)),
        "a_never_gate_rejection_loops_the_review" => {
            Box::pin(a_never_gate_rejection_loops_the_review(harness))
        }
        "a_never_gate_rejection_escalates_when_the_budget_is_out" => Box::pin(
            a_never_gate_rejection_escalates_when_the_budget_is_out(harness),
        ),
        "on_failure_passes_ok_and_parks_failed" => {
            Box::pin(on_failure_passes_ok_and_parks_failed(harness))
        }
        "an_intermediate_position_is_retired_by_the_loop" => {
            Box::pin(an_intermediate_position_is_retired_by_the_loop(harness))
        }
        "verify_fail_settles_failed" => Box::pin(verify_fail_settles_failed(harness)),
        "verify_unavailable_never_fails" => Box::pin(verify_unavailable_never_fails(harness)),
        "cancel_cleans_up_once" => Box::pin(cancel_cleans_up_once(harness)),
        "allowed_warning_candidate_is_selected" => {
            Box::pin(allowed_warning_candidate_is_selected(harness))
        }
        "a_skipped_candidate_falls_through_to_the_next" => {
            Box::pin(a_skipped_candidate_falls_through_to_the_next(harness))
        }
        "no_candidate_agent_blocks_the_item" => {
            Box::pin(no_candidate_agent_blocks_the_item(harness))
        }
        "a_capability_refusal_writes_no_run_and_unblock_reopens" => Box::pin(
            a_capability_refusal_writes_no_run_and_unblock_reopens(harness),
        ),
        "a_capability_refusal_at_claim_fails_the_run_by_name" => {
            Box::pin(a_capability_refusal_at_claim_fails_the_run_by_name(harness))
        }
        "every_candidate_skipped_refuses_the_run" => {
            Box::pin(every_candidate_skipped_refuses_the_run(harness))
        }
        "a_second_attempt_carries_verify_failure_and_previous_diff" => Box::pin(
            a_second_attempt_carries_verify_failure_and_previous_diff(harness),
        ),
        "fan_out_three_with_a_judge_selects_one_winner" => {
            Box::pin(fan_out_three_with_a_judge_selects_one_winner(harness))
        }
        "a_failing_verify_is_eliminated_before_the_judge" => {
            Box::pin(a_failing_verify_is_eliminated_before_the_judge(harness))
        }
        "one_passing_candidate_wins_without_a_judge" => {
            Box::pin(one_passing_candidate_wins_without_a_judge(harness))
        }
        "judge_orderings_that_disagree_park_for_selection" => {
            Box::pin(judge_orderings_that_disagree_park_for_selection(harness))
        }
        "an_unparseable_verdict_parks_and_select_fanout_completes" => Box::pin(
            an_unparseable_verdict_parks_and_select_fanout_completes(harness),
        ),
        "a_gated_fan_out_parks_for_human_selection" => {
            Box::pin(a_gated_fan_out_parks_for_human_selection(harness))
        }
        "no_judge_means_human_selection" => Box::pin(no_judge_means_human_selection(harness)),
        "a_failed_candidate_does_not_fail_its_siblings" => {
            Box::pin(a_failed_candidate_does_not_fail_its_siblings(harness))
        }
        "a_group_with_no_survivor_retries_then_fails" => {
            Box::pin(a_group_with_no_survivor_retries_then_fails(harness))
        }
        "the_review_loop_reruns_a_fanned_out_implement" => {
            Box::pin(the_review_loop_reruns_a_fanned_out_implement(harness))
        }
        "fan_out_above_max_fan_out_is_refused_at_start" => {
            Box::pin(fan_out_above_max_fan_out_is_refused_at_start(harness))
        }
        "a_malformed_cap_is_refused_at_start" => {
            Box::pin(a_malformed_cap_is_refused_at_start(harness))
        }
        "max_agents_per_run_is_refused_at_start" => {
            Box::pin(max_agents_per_run_is_refused_at_start(harness))
        }
        "a_serialized_sibling_is_not_charged_for_the_ones_before_it" => Box::pin(
            a_serialized_sibling_is_not_charged_for_the_ones_before_it(harness),
        ),
        "overlapping_touched_paths_serialise" => {
            Box::pin(overlapping_touched_paths_serialise(harness))
        }
        "the_same_paths_in_two_repos_run_concurrently" => {
            Box::pin(the_same_paths_in_two_repos_run_concurrently(harness))
        }
        "an_undeclared_item_holds_its_whole_primary_repo" => {
            Box::pin(an_undeclared_item_holds_its_whole_primary_repo(harness))
        }
        "a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap" => {
            Box::pin(a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap(harness))
        }
        "a_parked_run_releases_its_lease_and_an_answer_takes_it" => Box::pin(
            a_parked_run_releases_its_lease_and_an_answer_takes_it(harness),
        ),
        "a_live_lease_blocks_an_answer_from_another_process" => {
            Box::pin(a_live_lease_blocks_an_answer_from_another_process(harness))
        }
        "a_finished_step_is_adopted_through_its_gate" => {
            Box::pin(a_finished_step_is_adopted_through_its_gate(harness))
        }
        "an_unfinished_step_is_reset_and_retried" => {
            Box::pin(an_unfinished_step_is_reset_and_retried(harness))
        }
        "an_interrupted_step_out_of_budget_parks" => {
            Box::pin(an_interrupted_step_out_of_budget_parks(harness))
        }
        "a_dirty_tree_is_never_reset" => Box::pin(a_dirty_tree_is_never_reset(harness)),
        "a_crash_before_reconcile_is_reconciled_on_adoption" => {
            Box::pin(a_crash_before_reconcile_is_reconciled_on_adoption(harness))
        }
        "a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled" => Box::pin(
            a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled(harness),
        ),
        "an_interrupted_candidate_fails_alone" => {
            Box::pin(an_interrupted_candidate_fails_alone(harness))
        }
        "an_interrupted_judge_parks_for_selection" => {
            Box::pin(an_interrupted_judge_parks_for_selection(harness))
        }
        "a_half_written_park_is_completed" => Box::pin(a_half_written_park_is_completed(harness)),
        "the_sweep_never_touches_a_parked_run_or_a_live_lease" => Box::pin(
            the_sweep_never_touches_a_parked_run_or_a_live_lease(harness),
        ),
        "a_prompt_refusal_blocks_the_item_and_starts_no_session" => Box::pin(
            a_prompt_refusal_blocks_the_item_and_starts_no_session(harness),
        ),
        "a_prompt_refusal_in_a_fan_out_blocks_the_item" => {
            Box::pin(a_prompt_refusal_in_a_fan_out_blocks_the_item(harness))
        }
        "a_topology_mismatch_parks_a_running_run" => {
            Box::pin(a_topology_mismatch_parks_a_running_run(harness))
        }
        "cancel_meets_a_live_lease_and_writes_nothing" => {
            Box::pin(cancel_meets_a_live_lease_and_writes_nothing(harness))
        }
        "promote_keeps_the_step_and_writes_no_chat_run" => {
            Box::pin(promote_keeps_the_step_and_writes_no_chat_run(harness))
        }
        "promote_a_failed_step_of_a_parked_run" => {
            Box::pin(promote_a_failed_step_of_a_parked_run(harness))
        }
        "promote_refuses_a_terminal_run" => Box::pin(promote_refuses_a_terminal_run(harness)),
        "promote_moves_a_dropped_running_step_to_awaiting" => {
            Box::pin(promote_moves_a_dropped_running_step_to_awaiting(harness))
        }
        "accept_artifact_verifies_captures_and_resumes_at_the_next_position" => {
            Box::pin(accept_artifact_verifies_captures_and_resumes_at_the_next_position(harness))
        }
        "accept_artifact_needs_the_document_and_the_promotion" => Box::pin(
            accept_artifact_needs_the_document_and_the_promotion(harness),
        ),
        "accept_artifact_refuses_a_failed_verify" => {
            Box::pin(accept_artifact_refuses_a_failed_verify(harness))
        }
        "accept_artifact_verifies_on_a_deadline_from_the_accept" => Box::pin(
            accept_artifact_verifies_on_a_deadline_from_the_accept(harness),
        ),
        "accept_artifact_notes_an_unavailable_verify" => {
            Box::pin(accept_artifact_notes_an_unavailable_verify(harness))
        }
        "unblock_opens_a_blocked_item_with_no_run" => {
            Box::pin(unblock_opens_a_blocked_item_with_no_run(harness))
        }
        "unblock_lets_an_escalated_run_be_promoted_and_approved" => Box::pin(
            unblock_lets_an_escalated_run_be_promoted_and_approved(harness),
        ),
        "unblock_resumes_a_reconcile_refused_park" => {
            Box::pin(unblock_resumes_a_reconcile_refused_park(harness))
        }
        "close_out_writes_one_summary_and_closes_the_item" => {
            Box::pin(close_out_writes_one_summary_and_closes_the_item(harness))
        }
        "close_out_is_refused_while_a_run_is_live" => {
            Box::pin(close_out_is_refused_while_a_run_is_live(harness))
        }
        "a_suspended_walk_cannot_write_after_adoption" => {
            Box::pin(a_suspended_walk_cannot_write_after_adoption(harness))
        }
        "a_walk_woken_after_done_records_no_commits" => {
            Box::pin(a_walk_woken_after_done_records_no_commits(harness))
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

/// MOD-41 T9's cases, in a frame of their own. An unoptimised build gives every arm of a `match`
/// a stack slot for its case future before the future is boxed, and eighty-five of them in one
/// frame overflowed a test thread's 2 MiB before the first poll. [`case`] calls this and
/// [`earlier_case`] in turn and holds no slot itself, so neither frame is live under the other.
fn hand_back_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "an_approval_handed_back_is_walked_by_the_adopter" => {
            Box::pin(an_approval_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_rejection_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_rejection_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_selection_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_selection_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_resume_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_resume_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_crashed_rejection_handed_back_is_failed_by_the_adopter" => Box::pin(
            a_crashed_rejection_handed_back_is_failed_by_the_adopter(harness),
        ),
        "an_accept_handed_back_is_walked_by_the_adopter" => {
            Box::pin(an_accept_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_group_retry_handed_back_is_walked_by_the_adopter" => {
            Box::pin(a_group_retry_handed_back_is_walked_by_the_adopter(harness))
        }
        "a_retry_past_the_budget_handed_back_is_walked_not_reparked" => Box::pin(
            a_retry_past_the_budget_handed_back_is_walked_not_reparked(harness),
        ),
        "a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note" => {
            Box::pin(a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note(harness))
        }
        "a_hand_back_writes_no_step_past_the_window" => {
            Box::pin(a_hand_back_writes_no_step_past_the_window(harness))
        }
        _ => return None,
    })
}

/// MOD-26 milestone 1's cases, in a frame of their own for [`hand_back_case`]'s reason (blueprint
/// F-23): each arm's future is boxed before it is polled, so none of them adds a slot to the
/// frames the other cases run beneath.
fn persona_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_persona_bound_phase_prompt_carries_the_persona_section" => Box::pin(
            a_persona_bound_phase_prompt_carries_the_persona_section(harness),
        ),
        "a_persona_edit_after_start_run_does_not_reach_the_run" => Box::pin(
            a_persona_edit_after_start_run_does_not_reach_the_run(harness),
        ),
        "the_judge_step_runs_without_a_persona" => {
            Box::pin(the_judge_step_runs_without_a_persona(harness))
        }
        "a_persona_without_command_run_drops_the_command_queue_section" => {
            Box::pin(a_persona_without_command_run_drops_the_command_queue_section(harness))
        }
        "a_persona_run_leaves_the_agent_row_untouched" => {
            Box::pin(a_persona_run_leaves_the_agent_row_untouched(harness))
        }
        _ => return None,
    })
}

/// MOD-163 M2's cases, in a frame of their own for [`hand_back_case`]'s reason.
fn memory_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_memory_entry_reaches_every_phase_prompt_of_its_project" => Box::pin(
            a_memory_entry_reaches_every_phase_prompt_of_its_project(harness),
        ),
        "closed_memory_entries_never_reach_a_prompt" => {
            Box::pin(closed_memory_entries_never_reach_a_prompt(harness))
        }
        "a_memory_edit_after_start_run_does_not_reach_the_run" => Box::pin(
            a_memory_edit_after_start_run_does_not_reach_the_run(harness),
        ),
        "fan_out_siblings_carry_byte_identical_memory_sections" => Box::pin(
            fan_out_siblings_carry_byte_identical_memory_sections(harness),
        ),
        "a_snapshot_without_memory_resumes_with_no_memory_section" => Box::pin(
            a_snapshot_without_memory_resumes_with_no_memory_section(harness),
        ),
        "a_project_without_memory_writes_no_memory_row" => {
            Box::pin(a_project_without_memory_writes_no_memory_row(harness))
        }
        _ => return None,
    })
}

/// MOD-37's cases, in a frame of their own for [`hand_back_case`]'s reason.
fn hardening_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo" => {
            Box::pin(a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo(harness))
        }
        _ => return None,
    })
}

/// MOD-73's case, in a frame of its own for [`hand_back_case`]'s reason.
fn input_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_gate_edit_is_what_the_next_phase_reads" => {
            Box::pin(a_gate_edit_is_what_the_next_phase_reads(harness))
        }
        _ => return None,
    })
}

/// [`case`] for MOD-11 T8's six (plan D16, D17), boxed for [`case`]'s reason.
fn command_queue_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_single_fan_out_fan_out_only_step_has_no_command_queue_section" => {
            Box::pin(a_single_fan_out_fan_out_only_step_has_no_command_queue_section(harness))
        }
        "a_fan_out_two_step_has_it" => Box::pin(a_fan_out_two_step_has_it(harness)),
        "a_heavy_build_item_has_it" => Box::pin(a_heavy_build_item_has_it(harness)),
        "a_persona_less_exposed_step_gets_the_denials_first" => {
            Box::pin(a_persona_less_exposed_step_gets_the_denials_first(harness))
        }
        "a_persona_step_gets_the_denials_first_too" => {
            Box::pin(a_persona_step_gets_the_denials_first_too(harness))
        }
        "judges_never_get_command_run" => Box::pin(judges_never_get_command_run(harness)),
        _ => return None,
    })
}

/// MOD-12 M1's three (ANA-2 criteria 23, 24), boxed for [`case`]'s reason.
fn auto_mode_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "auto_mode_skips_soft_gates" => Box::pin(auto_mode_skips_soft_gates(harness)),
        "auto_mode_parks_at_a_hard_gate" => Box::pin(auto_mode_parks_at_a_hard_gate(harness)),
        "a_manual_snapshot_keeps_its_gates" => Box::pin(a_manual_snapshot_keeps_its_gates(harness)),
        _ => return None,
    })
}

/// MOD-12 M2's five (plan D5, D6, and review R1's M1), boxed for [`case`]'s reason.
fn spend_guard_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_batch_at_its_cap_starts_no_attempt" => {
            Box::pin(a_batch_at_its_cap_starts_no_attempt(harness))
        }
        "a_batch_run_session_is_capped_at_the_batch_remainder" => Box::pin(
            a_batch_run_session_is_capped_at_the_batch_remainder(harness),
        ),
        "a_manual_run_ignores_batch_figures" => {
            Box::pin(a_manual_run_ignores_batch_figures(harness))
        }
        "the_run_cap_spans_steps" => Box::pin(the_run_cap_spans_steps(harness)),
        "a_lowered_live_run_cap_does_not_cut_a_snapshotted_run" => Box::pin(
            a_lowered_live_run_cap_does_not_cut_a_snapshotted_run(harness),
        ),
        _ => return None,
    })
}

/// MOD-138 M1's six pinned-resume cases (plan D5), boxed for [`case`]'s reason.
fn pinned_resume_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_v2_run_walks_on_after_a_live_edit" => {
            Box::pin(a_v2_run_walks_on_after_a_live_edit(harness))
        }
        "a_running_v2_run_walks_on_after_a_live_edit" => {
            Box::pin(a_running_v2_run_walks_on_after_a_live_edit(harness))
        }
        "a_handed_back_v2_run_on_a_changed_graph_walks_on" => {
            Box::pin(a_handed_back_v2_run_on_a_changed_graph_walks_on(harness))
        }
        "a_tampered_v2_snapshot_parks_with_both_digests" => {
            Box::pin(a_tampered_v2_snapshot_parks_with_both_digests(harness))
        }
        "a_v1_run_whose_live_graph_gained_a_non_default_edge_walks_on" => {
            Box::pin(a_v1_run_whose_live_graph_gained_a_non_default_edge_walks_on(harness))
        }
        "a_v1_run_on_a_refused_graph_whose_phases_changed_parks" => Box::pin(
            a_v1_run_on_a_refused_graph_whose_phases_changed_parks(harness),
        ),
        _ => return None,
    })
}

/// MOD-144's three, boxed for [`case`]'s reason.
fn item_state_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_queued_cancel_writes_no_item" => Box::pin(a_queued_cancel_writes_no_item(harness)),
        "a_stopped_run_holds_its_item" => Box::pin(a_stopped_run_holds_its_item(harness)),
        "a_rework_run_on_a_finished_item_is_admitted" => {
            Box::pin(a_rework_run_on_a_finished_item_is_admitted(harness))
        }
        _ => return None,
    })
}

/// MOD-138 M2 T4's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn graph_walker_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_changes_requested_edge_loops_a_node_not_named_review" => Box::pin(
            a_changes_requested_edge_loops_a_node_not_named_review(harness),
        ),
        "a_review_with_no_changes_requested_edge_passes_unread" => Box::pin(
            a_review_with_no_changes_requested_edge_passes_unread(harness),
        ),
        "a_fails_edge_loops_and_retires_its_region" => {
            Box::pin(a_fails_edge_loops_and_retires_its_region(harness))
        }
        "a_human_reject_of_a_fails_node_loops" => {
            Box::pin(a_human_reject_of_a_fails_node_loops(harness))
        }
        "a_fails_loop_cut_short_is_replayed_by_settle_failed" => {
            Box::pin(a_fails_loop_cut_short_is_replayed_by_settle_failed(harness))
        }
        "a_spent_fails_loop_escalates" => Box::pin(a_spent_fails_loop_escalates(harness)),
        "a_loop_on_a_diamond_retires_only_its_region" => {
            Box::pin(a_loop_on_a_diamond_retires_only_its_region(harness))
        }
        "an_edge_loop_without_progress_escalates" => {
            Box::pin(an_edge_loop_without_progress_escalates(harness))
        }
        "a_graph_with_authored_edges_resolves" => {
            Box::pin(a_graph_with_authored_edges_resolves(harness))
        }
        _ => return None,
    })
}

/// MOD-138 M2 T5's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn branch_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "each_if_predicate_decides_from_its_facts" => {
            Box::pin(each_if_predicate_decides_from_its_facts(harness))
        }
        "a_declined_if_branch_is_skipped_and_the_join_runs" => {
            Box::pin(a_declined_if_branch_is_skipped_and_the_join_runs(harness))
        }
        "an_undecided_branch_parks_on_an_isolator_error_and_unblock_resumes_it" => {
            Box::pin(an_undecided_branch_parks_on_an_isolator_error_and_unblock_resumes_it(harness))
        }
        "a_forward_asks_you_approved_runs_its_target" => {
            Box::pin(a_forward_asks_you_approved_runs_its_target(harness))
        }
        "a_forward_asks_you_rejected_skips_its_target_and_lands_the_work" => {
            Box::pin(a_forward_asks_you_rejected_skips_its_target_and_lands_the_work(harness))
        }
        "auto_mode_never_downgrades_an_asks_you_stop" => {
            Box::pin(auto_mode_never_downgrades_an_asks_you_stop(harness))
        }
        "a_back_asks_you_approved_loops" => Box::pin(a_back_asks_you_approved_loops(harness)),
        "a_crash_between_an_asks_you_approve_and_its_loop_is_replayed" => {
            Box::pin(a_crash_between_an_asks_you_approve_and_its_loop_is_replayed(harness))
        }
        "a_spent_back_asks_you_escalates_and_approve_walks_on" => Box::pin(
            a_spent_back_asks_you_escalates_and_approve_walks_on(harness),
        ),
        "a_branch_decision_is_read_from_its_row_after_a_restart" => Box::pin(
            a_branch_decision_is_read_from_its_row_after_a_restart(harness),
        ),
        "accepting_an_artifact_on_an_asks_you_node_declines_its_edges" => {
            Box::pin(accepting_an_artifact_on_an_asks_you_node_declines_its_edges(harness))
        }
        "a_reject_on_a_verdict_reading_asks_you_node_loops" => {
            Box::pin(a_reject_on_a_verdict_reading_asks_you_node_loops(harness))
        }
        "a_reject_of_a_failed_park_on_an_asks_you_node_ends_the_run" => Box::pin(
            a_reject_of_a_failed_park_on_an_asks_you_node_ends_the_run(harness),
        ),
        _ => return None,
    })
}

/// MOD-144 (ANA-35 §7.1): cancelling the seeded `queued` `RUN_2` writes no item row. The item was
/// stored `open` all along and read `queued` from its run; the cancelled run is skipped by the
/// latest-run rule, so it reads `open` again and a new `StartRun` is admitted.
async fn a_queued_cancel_writes_no_item<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let before = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        before.status,
        Status::Open,
        "the fixture stores `FEAT-3` open"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Queued,
        "and it reads its queued `RUN_2`"
    );

    let outcome = orch
        .dispatch(Command::CancelRun { run: ids::RUN_2 })
        .await
        .expect("a queued run is cancellable");
    assert!(
        matches!(outcome, CommandOutcome::Cancelled { ref rest } if rest.run == RunStatus::Cancelled),
        "{outcome:?}"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await,
        before,
        "the cancel wrote no item row"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Open,
        "the cancelled run is skipped"
    );

    let (_, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "the item is free");
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
}

/// MOD-144 (Δ1, Δ2; MOD-125): a `stopped` run is live. Its item reads `stopped`, and every verb
/// that needs the item free refuses: `StartRun` with `startable`'s sentence and no note, `Unblock`
/// naming the walking run, `CloseOut` naming the live run. Nothing blocks the item.
async fn a_stopped_run_holds_its_item<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    // MOD-125's stop, by hand: the parked run back to `running`, then `stopped`.
    let now = orch.clock().now();
    for (from, to) in [
        (RunStatus::AwaitingApproval, RunStatus::Running),
        (RunStatus::Running, RunStatus::Stopped),
    ] {
        assert!(
            orch.store()
                .transition_run(run, from, to, now)
                .await
                .expect("MemStore takes the move"),
            "the run was `{from}`"
        );
    }
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Stopped);
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await.len();

    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        })
        .await
        .expect_err("a stopped run is live");
    assert!(
        matches!(&refused, EngineError::Store(StoreError::Constraint(sentence))
            if *sentence == htui_core::store::not_startable("FEAT-3", ItemState::Stopped)),
        "{refused}"
    );

    let refused = orch
        .dispatch(Command::Unblock {
            item: ids::HTUI_FEAT_3,
        })
        .await
        .expect_err("a stopped run is resumed, not unblocked");
    assert!(
        matches!(&refused, EngineError::NotBlocked { state: ItemState::Stopped, why, .. }
            if *why == crate::command::run_is_walking(run, RunStatus::Stopped)),
        "{refused}"
    );

    let refused = orch
        .dispatch(Command::CloseOut {
            item: ids::HTUI_FEAT_3,
            resolution: Resolution::Withdrawn,
        })
        .await
        .expect_err("a live run");
    assert!(
        matches!(
            refused,
            EngineError::RunStatus {
                run: live,
                status: RunStatus::Stopped,
                ..
            } if live == run
        ),
        "{refused}"
    );

    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "no refusal blocked the item"
    );
    assert_eq!(
        notes_of(&orch, ids::HTUI_FEAT_3).await.len(),
        notes,
        "no refusal wrote a note (D98)"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::Stopped);
}

/// MOD-144 (ANA-35 §7.1): a finished item admits a rework run, which is then the latest graph run
/// (`needs you` at its first gate). Cancelling it leaves the item finished — the cancelled run is
/// skipped — and close-out takes `done`.
async fn a_rework_run_on_a_finished_item_is_admitted<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (first, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve(&orch, first, 4).await;
    assert_eq!(run_of(&orch, first).await.status, RunStatus::Done);
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Done);

    orch.clock().advance(TimeDelta::seconds(1));
    let (rework, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "the rework run is the latest graph run"
    );

    let outcome = orch
        .dispatch(Command::CancelRun { run: rework })
        .await
        .expect("a parked run is cancellable");
    assert!(
        matches!(outcome, CommandOutcome::Cancelled { ref rest } if rest.run == RunStatus::Cancelled),
        "{outcome:?}"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Done,
        "the cancelled rework is skipped: the item reads its finished run"
    );

    let outcome = orch
        .dispatch(Command::CloseOut {
            item: ids::HTUI_FEAT_3,
            resolution: Resolution::Done,
        })
        .await
        .expect("a finished item closes `done`");
    assert!(
        matches!(outcome, CommandOutcome::ClosedOut { item, .. } if item == ids::HTUI_FEAT_3),
        "{outcome:?}"
    );
    let item = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (item.status, item.resolution),
        (Status::Closed, Some(Resolution::Done))
    );
    assert!(item.closed_at.is_some(), "close-out stamps `closed_at`");
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Closed);
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

/// The item's derived display state (MOD-144): its stored status over its latest graph run.
///
/// # Panics
/// When the item is not there, which in a case means the fixture moved.
async fn state_of<O: Orchestrate>(orch: &O, id: ItemId) -> ItemState {
    crate::fake::state_in(orch.store(), id).await
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

/// `ids::HTUI_FEAT_3` is seeded `open` with a live `queued` `RUN_2` on it, so `create_run`
/// refuses `StartRun` (MOD-144: `startable`, no live graph run) until `RUN_2` ends. Cancelling
/// it writes no item row, and is the one-line prologue every case that walks `FEAT-3` needs.
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
/// **`PhasePatch` carries six fields — `name`, `position`, `template_name`, `gate_hard`,
/// `input_kinds` and `persona` (MOD-26 D5) — and `gate`, `retry_limit`, `isolation`, `fan_out`
/// and `token_budget` are none of them** (`crates/htui-core/src/model/kind.rs:230-251`), so the
/// blueprint's `update_phase(review, expected, PhasePatch { gate: Some(Gate::Never), .. })`
/// recipe does not compile. `create_phase` takes a whole `StepGraphPhase`, so a clone carrying the
/// row a case wants is the reachable edit. (`graph::override_graph` itself writes its clone
/// through one `replace_graph` since MOD-137.)
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
            scope: GraphScope::Project(row.project_id),
            name: format!("{}-edited", row.key),
            description: "a conformance case's edit of the live graph".to_owned(),
            is_override: false,
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
        // ck_phase_split_fan_out (MOD-137 D7): `split` follows the edited `fan_out`.
        if edited.split != htui_core::model::Split::Divide {
            edited.split = htui_core::model::Split::for_fan_out(edited.fan_out);
        }
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
            scope: GraphScope::Project(row.project_id),
            name: format!("{}-with-verify", row.key),
            description: "an intermediate position between implement and review".to_owned(),
            is_override: false,
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
        // ck_phase_split_fan_out (MOD-137 D7): `split` follows the edited `fan_out`.
        if edited.split != htui_core::model::Split::Divide {
            edited.split = htui_core::model::Split::for_fan_out(edited.fan_out);
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
    start_in(orch, item, RunMode::Manual).await
}

/// `StartRun` on `item` in `mode` with no requested scope, unwrapped to its run id and rest.
///
/// # Panics
/// When the command is refused, which every caller of this helper expects not to be.
async fn start_in<O: Orchestrate>(orch: &O, item: ItemId, mode: RunMode) -> (RunId, Rest) {
    let outcome = orch
        .dispatch(Command::StartRun {
            item,
            mode,
            repo_scope: None,
            workspace: None,
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
/// The two writes are `Engine::answer_gate`'s own, minus its closing `run_to_rest`
/// (`crates/htui-orch/src/engine.rs`): `answer_gate`, then the run, in that order (MOD-144: the
/// item is not written).
///
/// It exists for criterion 3. `run_to_rest` returns at its `AwaitingApproval` arm before it reads a
/// cursor, so "resume advanced nothing" asserted on a *parked* run holds however the resume behaves
/// — including with the topology guard deleted. A run that is `running` with a position to create is
/// the only state in which the assertion is about the guard.
///
/// # Panics
/// When no step is parked, or when either write is refused.
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
    assert_eq!(
        state_of(orch, item).await,
        ItemState::Running,
        "the item reads its unparked run"
    );
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
/// `produced_by_step_id` of the step that earned it, and the item reading the run (MOD-144).
///
/// `prompt_digest` is asserted `Some` rather than re-hashed: stage 3's `set_step_prompt` and the
/// recorder's `record_prompt_digesting` write the same digest by construction (blueprint H-13),
/// both over the digest text ANA-5 supplies (MOD-33 D271), and a case that re-hashed a scrubbed
/// variant would be pinning the scrubber, not the walk.
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
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "the park moves step and run; the parked run reads `needs you` (blueprint H-10, MOD-144)"
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

    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Done,
        "MOD-144: the item reads its finished run"
    );
    let item = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(item.status, Status::Open, "`finish_run` writes no item");
    assert!(
        item.closed_at.is_none(),
        "only close-out stamps `closed_at` (MOD-144 Δ10)"
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
            ("research", StepStatus::Done, Some(GateOutcome::Skipped)),
            ("verdict", StepStatus::Done, Some(GateOutcome::Skipped)),
        ],
        "`never` passes a step `running -> done` with `gate_outcome = 'skipped'` (MOD-37 R-5, \
         ANA-2 §4.2)"
    );
    assert_eq!(state_of(&orch, ids::HTUI_ANA_2).await, ItemState::Done);
}

/// ANA-2 §12 criterion 3 (`docs/ANA-2.md:2090`): the graph moved under a parked **v1** run.
///
/// MOD-138 D5: only a v1 run (a snapshot written before MOD-138, here a [`rewrite_as_v1`]
/// fixture) still compares against the live graph; `a_v2_run_walks_on_after_a_live_edit` is the
/// v2 twin, which walks on.
///
/// `input_kinds` is the edit because it is both one of `PhasePatch`'s six fields and a
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
    rewrite_as_v1(orch.store(), run).await;
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
        (RunStatus::AwaitingApproval, Some(1)),
        "the run is parked where it was, owing the position it owed: a mismatch is a human's \
         decision, not the engine's (MOD-4 plan D159)"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "the item follows its run to the gate"
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

    // An unedited run resumes into the walk instead — and the walk does something. A v1 one too:
    // the lifted snapshot's edges are the live graph's default edges (MOD-138 D4, D5).
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    rewrite_as_v1(orch.store(), run).await;
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
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
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

/// MOD-73 (ANA-2 §4.2 as amended; §4.2's gate-answer row "edits the artifact"): a version of the
/// parked step's `output_kind` written by hand at the gate (`produced_by_step_id` `NULL`, as the
/// Backlog's Docs `v` writes it since MOD-13) and then `approved` is what the next phase's stage 3
/// reads, because it is newer than the step's own output.
///
/// The observable is the next step's recorded prompt, the bytes its agent was sent: the
/// `documents:prd` section carries the edit's version and body, and the step's own body appears
/// nowhere in it. The resolver is asserted at the run's seat too, so the prompt and §4.2 cannot
/// disagree unnoticed.
async fn a_gate_edit_is_what_the_next_phase_reads<H: CaseHarness>(harness: &H) {
    const WRITTEN: &str = "PRD as the prd step wrote it.";
    const EDITED: &str = "PRD as the human edited it at the gate.";
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.script("prd", 1, ScriptedStep::done_with_output(WRITTEN));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "`prd` parks at its `always` gate with its output written"
    );
    let steps = steps_of(&orch, run).await;
    let parked = at(&steps, 0, 1).clone();
    let written = orch
        .store()
        .documents(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|head| head.kind == "prd" && head.produced_by_step_id == Some(parked.id))
        .expect("the parked step wrote its `prd`");

    // The edit: a new version, by hand, while the run is parked (R-ENT-12: append-only).
    let edited = orch
        .store()
        .write_document(NewDocument {
            id: DocumentId::new(),
            item_id: ids::HTUI_FEAT_3,
            kind: "prd".to_owned(),
            title: "prd (edited at the gate)".to_owned(),
            body: EDITED.to_owned(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: orch.clock().now(),
        })
        .await
        .expect("the item and the user exist");
    assert!(
        edited.version > written.version,
        "the edit is the newer version: v{} after the step's v{}",
        edited.version,
        written.version
    );

    let (answered, rest) = answer(&orch, run, GateAnswer::Approved).await;
    assert_eq!(
        answered.id, parked.id,
        "the approve answered the edited step"
    );
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "approve walked on: `plan` ran and parked"
    );

    let steps = steps_of(&orch, run).await;
    let prompt = prompt_text(&orch, at(&steps, 1, 1).id).await;
    let section = format!(
        "<section name=\"documents:prd\" kind=\"prd\" version=\"{}\">\n{EDITED}\n</section>",
        edited.version
    );
    assert!(
        prompt.contains(&section),
        "`plan`'s stage 3 read the edit, v{}:\n{prompt}",
        edited.version
    );
    assert!(
        !prompt.contains(WRITTEN),
        "and not the step's own output, which the edit superseded:\n{prompt}"
    );

    let inputs = orch
        .store()
        .resolve_inputs(ids::HTUI_FEAT_3, run, &["prd".to_owned()])
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        inputs
            .first()
            .and_then(|input| input.document.as_ref())
            .map(|document| document.id),
        Some(edited.id),
        "§4.2's resolver at the run's seat answers the edit"
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

/// CLEAN-4 (plan D4): two identical review documents stop the loop *before* its retry budget says
/// so, on `no_progress_review`, criterion 7's other half.
///
/// The twin of `identical_after_hash_stops_the_loop` with the halves swapped. **No repo is in
/// scope**, so the hash half answers false on the empty `repo_scope` and cannot be what stops
/// the loop. Both review attempts are scripted with one body, so the review half does.
/// `retry_limit = 3` on `implement` again means the budget would have permitted a third attempt.
async fn identical_reviews_stop_the_loop<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "implement" {
            phase.retry_limit = 3;
        }
    })
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "no tests"),
    );
    orch.script(
        "review",
        2,
        ScriptedStep::review("request-changes", "no tests"),
    );

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        run_of(&orch, run).await.repo_scope.is_empty(),
        "plan D14: no primary repo, so `repo_scope: None` resolves to nothing and the hash half \
         cannot decide"
    );

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
    .await; // review 2 -> the same review, so the loop stops

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
                && body.contains("stop reason `no_progress_review`")
        }),
        "the budget permitted a third attempt and no repo was in scope; the review half is what \
         stopped it: {notes:?}"
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
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Failed);

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
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Failed);

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
/// a closed enum of eleven that does not carry the key and `ProjectPatch` has no `settings` field, so
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

/// Plan D7's terminal writer, read from both ends of the walk (MOD-144: it writes no item).
///
/// The prologue is the first end: `finish_run(RUN_2, Cancelled)` is what frees `FEAT-3`, and it
/// frees it without touching the item row — the cancelled run is skipped by the latest-run rule,
/// so the item reads `open` again. Criterion 1's tail is the second: one `finish_run` at the last
/// approve stamps `run.finished_at`, the item reads `done`, and no separate `transition_run` is
/// needed to get there.
async fn finish_run_is_the_last_write<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let seeded = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(seeded.status, Status::Open);
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Queued,
        "the fixture seeds `FEAT-3` open behind the live `RUN_2`"
    );
    free_feat_3(&orch).await;
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Open,
        "the cancelled run is skipped: the item reads `open`"
    );
    let freed = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (freed.version, freed.updated_at),
        (seeded.version, seeded.updated_at),
        "MOD-144: cancelling the run wrote no item row"
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

    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::Done,
        "the item reads its finished run"
    );
    let item = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(item.status, Status::Open, "`finish_run` writes no item");
    assert!(
        item.closed_at.is_none(),
        "only close-out stamps `closed_at` (MOD-144 Δ10)"
    );
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
        Some(GateOutcome::Skipped),
        "`never` passes a step `running -> done` with `gate_outcome = 'skipped'` (MOD-37 R-5)"
    );
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Done);
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
        Some(GateOutcome::Skipped),
        "`on_failure` passes an ok settle with `gate_outcome = 'skipped'` (MOD-37 R-5)"
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
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
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
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Done);
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
        "`finish_run` writes no item; it stays `open` (plan D7, MOD-144)"
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
            workspace: None,
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

/// ANA-2 §12 criterion 14 (`docs/ANA-2.md:2121-2122`), its capability half (MOD-7 milestone 3,
/// D86): an item requiring tags the box has neither probed nor declared is refused at `StartRun`
/// with no `run` row, the item `blocked` and one note whose body is exactly the missing tags,
/// written on this box; `Unblock` reopens it, and once the box declares the tags `StartRun` walks.
async fn a_capability_refusal_writes_no_run_and_unblock_reopens<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    require_tags(&orch, ids::HTUI_FEAT_3, &["rust", "vulkan", "docker"]).await;
    let runs_before = orch
        .store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .len();
    let notes_before = orch
        .store()
        .notes(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .len();

    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        })
        .await
        .expect_err("the demo box has neither `docker` nor `vulkan`");
    assert!(
        matches!(
            &refused,
            EngineError::MissingTags { item, run: None, missing }
                if *item == ids::HTUI_FEAT_3 && missing == &["docker", "vulkan"]
        ),
        "{refused:?}"
    );
    assert_eq!(
        refused.to_string(),
        format!("item {}: missing tags: docker, vulkan", ids::HTUI_FEAT_3)
    );
    assert_eq!(
        orch.store()
            .runs(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .len(),
        runs_before,
        "a queue-time refusal writes no run row (ANA-2 §4.10)"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = orch
        .store()
        .notes(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read");
    let added = &notes[notes_before..];
    assert_eq!(added.len(), 1, "exactly one note: {added:?}");
    assert_eq!(added[0].body, "missing tags: docker, vulkan");
    assert_eq!(added[0].box_id, Some(ids::BOX), "the note names the box");
    assert_eq!(added[0].via_step_id, None, "no step exists");

    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((case, rest), (UnblockCase::Reopen, None));
    assert_eq!(item_of(&orch, ids::HTUI_FEAT_3).await.status, Status::Open);

    declare_tags(&orch, &["docker", "gpu", "vulkan"]).await;
    let (_, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "a box with those tags runs the reopened item"
    );
}

/// ANA-2 §4.10's claim-time half (MOD-7 milestone 3, D87; the PRD's "tag check at claim races a
/// re-probe" risk): a run queued behind an overlap while its tag was declared, whose tag is then
/// withdrawn, is failed at its next claim with `missing tags: gpu`, and the check comes before
/// the overlap that still stands. The item is `blocked` with the note, and `Unblock` reopens it.
async fn a_capability_refusal_at_claim_fails_the_run_by_name<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    // Blueprint P-5: without a primary repo the resolved scope is empty and never overlaps.
    primary_repo(&orch).await;
    let first = mint_feat(&orch, "Holder", &["src/**"]).await;
    let second = mint_feat(&orch, "Needs a GPU", &["src/**"]).await;
    // `gpu` is declared on the fixture box, so the enqueue passes.
    require_tags(&orch, second, &["gpu"]).await;

    let (holder, rest) = start(&orch, first).await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "the holder parks, and a parked run still holds its scope"
    );
    let (run, claim) = start_refused(&orch, second).await;
    assert_eq!(
        claim,
        Claim::Overlaps {
            with: holder,
            rule: OverlapRule::Paths,
        }
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::Queued);
    let notes_before = orch
        .store()
        .notes(second)
        .await
        .expect("MemStore never fails a read")
        .len();

    // The re-probe: the box withdraws its declared `gpu` while the run waits in the queue.
    declare_tags(&orch, &[]).await;
    let refused = orch
        .claim(run)
        .await
        .expect_err("the box no longer has `gpu`");
    assert!(
        matches!(
            &refused,
            EngineError::MissingTags { item, run: Some(failed), missing }
                if *item == second && *failed == run && missing == &["gpu"]
        ),
        "the tags, before the overlap that still stands (D81): {refused:?}"
    );

    let failed = run_of(&orch, run).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.failure.as_deref(), Some("missing tags: gpu"));
    assert_eq!(failed.executing_box_id, None, "no box executed it");
    assert_eq!(failed.started_at, None, "the run never started");
    assert_eq!(failed.lease_expires_at, None, "no lease was taken");
    assert!(failed.finished_at.is_some(), "a failed run is finished");
    assert!(
        steps_of(&orch, run).await.is_empty(),
        "nothing walked the failed run"
    );
    assert_eq!(item_of(&orch, second).await.status, Status::Blocked);
    let notes = orch
        .store()
        .notes(second)
        .await
        .expect("MemStore never fails a read");
    let added = &notes[notes_before..];
    assert_eq!(added.len(), 1, "exactly one note: {added:?}");
    assert_eq!(added[0].body, "missing tags: gpu");
    assert_eq!(added[0].box_id, Some(ids::BOX), "the note names the box");
    assert_eq!(
        run_of(&orch, holder).await.status,
        RunStatus::AwaitingApproval,
        "the holder is untouched"
    );

    let (case, rest) = unblock(&orch, second).await;
    assert_eq!(
        (case, rest),
        (UnblockCase::Reopen, None),
        "the failed run is not active, so the item reopens"
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
    assert_eq!(state_of(&orch, ids::HTUI_ANA_2).await, ItemState::NeedsYou);
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
    assert_eq!(state_of(&orch, ids::HTUI_ANA_2).await, ItemState::Done);
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
            workspace: None,
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

/// CLEAN-9 T2: a cap that is not a non-negative integer is refused at `StartRun`, before a run row
/// exists, rather than read as "no cap" and run uncapped.
async fn a_malformed_cap_is_refused_at_start<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let project = item_of(&orch, ids::HTUI_ANA_2).await.project_id;
    orch.store()
        .set_project_settings(project, serde_json::json!({ "per_token_cap_run": "x" }));
    let refused = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_ANA_2,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        })
        .await
        .expect_err("a string cap is refused");
    assert!(
        matches!(
            &refused,
            EngineError::Resolve(crate::graph::ResolveError::ProjectCaps { cause, .. })
                if cause.key == "per_token_cap_run"
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
/// an explicit `max_agents_per_run = 6`, refused at `StartRun` naming both figures.
///
/// The 6 is planted: the default is 8 (`0004_max_agents_per_run_default.sql`), which admits this
/// graph, and ANA-2's original 6 is what refuses it.
async fn max_agents_per_run_is_refused_at_start<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.store()
        .set_app_setting("max_agents_per_run", serde_json::json!(6));
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
            workspace: None,
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

// -- MOD-4 milestone 5: overlap reaches the walk, and the engine holds its lease ------------------

/// Sets `item.touched_paths` through the patch writer, so the next `StartRun` resolves it.
///
/// # Panics
/// When the item's version moved under the case.
async fn touch<O: Orchestrate>(orch: &O, item: ItemId, paths: &[&str]) {
    let row = item_of(orch, item).await;
    orch.store()
        .update_item(
            item,
            row.version,
            ItemPatch {
                touched_paths: Some(paths.iter().map(|path| (*path).to_owned()).collect()),
                author_id: row.created_by,
                reason: "a conformance case's declared paths".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// Sets `item.required_tags` through the patch writer, so the next `StartRun` checks them (MOD-7
/// milestone 3, D97).
///
/// # Panics
/// When the item's version moved under the case.
async fn require_tags<O: Orchestrate>(orch: &O, item: ItemId, tags: &[&str]) {
    let row = item_of(orch, item).await;
    orch.store()
        .update_item(
            item,
            row.version,
            ItemPatch {
                required_tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
                author_id: row.created_by,
                reason: "a conformance case's required tags".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// Replaces the demo box's `declared_tags` through `edit_box`, at its current token (MOD-7
/// milestone 3, D86): the same box after a declaration is "a box with those tags".
///
/// # Panics
/// When the box is gone or the edit is stale or refused, which means the fixture moved.
async fn declare_tags<O: Orchestrate>(orch: &O, tags: &[&str]) {
    let token = orch
        .store()
        .boxes()
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|record| record.row.id == ids::BOX)
        .expect("the fixture holds the demo box")
        .row
        .edit_version;
    let outcome = orch
        .store()
        .edit_box(
            ids::BOX,
            token,
            BoxEdit {
                declared_tags: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
                quirks: None,
                executor: None,
            },
        )
        .await
        .expect("the demo box accepts the tags");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the token is current: {outcome:?}"
    );
}

/// A fresh `open` FEAT item on the HTUI project declaring `paths`: criterion 16 needs five
/// startable items and the fixture seeds three (`ANA-2`, `FEAT-3` once freed, `CLEAN-1`).
///
/// # Panics
/// When the mint is refused, which means the fixture moved.
async fn mint_feat<O: Orchestrate>(orch: &O, title: &str, paths: &[&str]) -> ItemId {
    let user = orch
        .store()
        .this_user()
        .expect("the demo fixture seeds exactly one `app_user`");
    orch.store()
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: title.to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
            priority: 0,
            step_graph_id: None,
            created_by: user,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("the FEAT kind mints on the HTUI project")
        .id
}

/// `StartRun` that the claim refuses, unwrapped to the queued run and the claim's answer.
///
/// # Panics
/// When the command is not a [`EngineError::ClaimRefused`].
async fn start_refused<O: Orchestrate>(orch: &O, item: ItemId) -> (RunId, Claim) {
    let refused = orch
        .dispatch(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        })
        .await
        .expect_err("the claim is refused");
    let EngineError::ClaimRefused { run, claim } = refused else {
        panic!("a claim refusal, not {refused}");
    };
    (run, claim)
}

/// ANA-2 §12 criterion 15, first half (`docs/ANA-2.md:2125`, plan D83, D84): two items
/// declaring `src/**` on one repo do not run together. The second `StartRun` is refused with the
/// rule and the holder, the run and its item stay `queued`, and once the holder ends
/// `Engine::claim` walks it.
async fn overlapping_touched_paths_serialise<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    touch(&orch, ids::HTUI_FEAT_3, &["src/**"]).await;
    touch(&orch, ids::HTUI_ANA_2, &["src/**"]).await;

    let (first, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "the first run parks at its first gate, and a parked run still holds its scope"
    );

    let (second, claim) = start_refused(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        claim,
        Claim::Overlaps {
            with: first,
            rule: OverlapRule::Paths,
        }
    );
    assert_eq!(run_of(&orch, second).await.status, RunStatus::Queued);
    assert_eq!(
        state_of(&orch, ids::HTUI_ANA_2).await,
        ItemState::Queued,
        "plan D84: a refusal writes nothing"
    );
    assert!(
        steps_of(&orch, second).await.is_empty(),
        "nothing walked the refused run"
    );

    orch.dispatch(Command::CancelRun { run: first })
        .await
        .expect("the holder is cancellable");
    let outcome = orch
        .claim(second)
        .await
        .expect("the holder is gone, so the claim is admitted");
    assert!(
        matches!(outcome, CommandOutcome::Started { run, .. } if run == second),
        "{outcome:?}"
    );
    assert_ne!(run_of(&orch, second).await.status, RunStatus::Queued);
}

/// Criterion 15, second half (`:2125-2126`): the same paths in two repos are two scopes, so
/// both runs are admitted, and each run's `repo_scope` is the repo its item qualified.
async fn the_same_paths_in_two_repos_run_concurrently<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let htui = primary_repo(&orch).await;
    let web = RepoId::new();
    orch.store()
        .create_repo(NewRepo {
            id: web,
            project_id: ids::PROJECT_HTUI,
            name: "web".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: false,
        })
        .await
        .expect("the demo project has no `web` repo yet");
    free_feat_3(&orch).await;
    touch(&orch, ids::HTUI_FEAT_3, &["htui:src/**"]).await;
    touch(&orch, ids::HTUI_ANA_2, &["web:src/**"]).await;

    let (first, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let (second, _) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(run_of(&orch, first).await.repo_scope, [htui]);
    assert_eq!(run_of(&orch, second).await.repo_scope, [web]);
}

/// Criterion 15, `:2126-2127`: an item that declares nothing overlaps the whole of its primary repo,
/// so a run on `docs/**` holds it off.
async fn an_undeclared_item_holds_its_whole_primary_repo<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    touch(&orch, ids::HTUI_FEAT_3, &["docs/**"]).await;
    touch(&orch, ids::HTUI_ANA_2, &[]).await;

    let (first, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let (_, claim) = start_refused(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        claim,
        Claim::Overlaps {
            with: first,
            rule: OverlapRule::Paths,
        }
    );
}

/// ANA-2 §12 criterion 16 (`:2128-2129`, plan D83): a parked run holds no slot but still holds
/// its scope, and `SlotFull` is decided before overlap — C declares `src/**`, which overlaps the
/// running A, and is refused for the slot, not for A.
///
/// **A and B stand in for two live walks by hand** — both are started, park at their first gate,
/// and their runs are moved `awaiting_approval -> running`. A walk that stays
/// `running` across a `dispatch` does not exist in the fake (every script finishes before its
/// first beat, plan D86), and a `queued` run with a resolved scope has no other constructor on
/// the case's surface. The clock moves a second between starts so `queued_at` orders the runs
/// (blueprint H-5).
async fn a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    let parked = mint_feat(&orch, "P", &["lib/**"]).await;
    let waiting = mint_feat(&orch, "E", &["lib/**"]).await;
    let first = mint_feat(&orch, "A", &["src/**"]).await;
    let second = mint_feat(&orch, "B", &["docs/**"]).await;
    let third = mint_feat(&orch, "C", &["src/**"]).await;

    let (holder, rest) = start(&orch, parked).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    orch.clock().advance(TimeDelta::seconds(1));

    let (_, claim) = start_refused(&orch, waiting).await;
    assert_eq!(
        claim,
        Claim::Overlaps {
            with: holder,
            rule: OverlapRule::Paths,
        },
        "no run is `running`, so the refusal is the parked run's scope and not a slot"
    );

    for item in [first, second] {
        orch.clock().advance(TimeDelta::seconds(1));
        let (run, rest) = start(&orch, item).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        let now = orch.clock().now();
        assert!(
            orch.store()
                .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
                .await
                .expect("MemStore takes the move"),
            "the run was parked"
        );
    }

    orch.clock().advance(TimeDelta::seconds(1));
    let (queued, claim) = start_refused(&orch, third).await;
    assert_eq!(
        claim,
        Claim::SlotFull {
            running: 2,
            limit: 2,
        },
        "`src/**` overlaps the running A, and the full box is what refuses it"
    );
    assert_eq!(run_of(&orch, queued).await.status, RunStatus::Queued);
}

/// Plan D87: a walk that parks releases its lease (`lease_expires_at = now`), so a process
/// started after this one takes it at once with its answer, and the lease is then the new
/// owner's. Its own park releases it again, which only the owner can (plan D139), and the old
/// owner's refresh touches nothing.
async fn a_parked_run_releases_its_lease_and_an_answer_takes_it<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    assert_eq!(
        run_of(&orch, run).await.lease_expires_at,
        Some(orch.clock().now()),
        "the park released the lease"
    );

    let other = orch.restarted();
    assert_ne!(other.owner(), orch.owner(), "a restart is a new owner");
    let (_, rest) = answer(&other, run, GateAnswer::Approved).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the second process walked `plan` and parked at its gate"
    );

    assert_eq!(
        run_of(&orch, run).await.lease_expires_at,
        Some(other.clock().now()),
        "the lease was the second process's: its park released it"
    );
    assert!(
        !orch
            .store()
            .refresh_lease(run, orch.owner(), TimeDelta::minutes(1))
            .await
            .expect("MemStore refreshes"),
        "and no longer the first's"
    );
}

/// Plan D87 and D108: an answer from another process meets a live lease it does not hold, and is
/// refused **before** its first write — the step is still parked and unanswered.
async fn a_live_lease_blocks_an_answer_from_another_process<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let other = orch.restarted();
    assert!(
        orch.store()
            .take_lease(
                run,
                ids::BOX,
                orch.owner(),
                TimeDelta::days(1) + RESTART_GAP
            )
            .await
            .expect("MemStore takes the lease"),
        "the first process takes its released lease back, for a day"
    );
    let parked = steps_of(&orch, run)
        .await
        .into_iter()
        .find(|step| step.status == StepStatus::AwaitingApproval)
        .expect("the walk parked `prd`");

    let refused = other
        .dispatch(Command::AnswerGate {
            run,
            step: parked.id,
            answer: GateAnswer::Approved,
        })
        .await
        .expect_err("a live lease elsewhere");
    assert!(
        matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
        "{refused}"
    );
    let step = steps_of(&orch, run)
        .await
        .into_iter()
        .find(|step| step.id == parked.id)
        .expect("the step is still there");
    assert_eq!(step.status, StepStatus::AwaitingApproval);
    assert_eq!(step.gate_outcome, None, "plan D108: nothing written");
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
}

/// MOD-40 plan D1, D2 (ANA-16 C1): a walk suspended between its session and its settle, whose run
/// another process adopted meanwhile, wakes and writes nothing to the step. Its `finish_step` is
/// fenced by the owner it walked under, and so is a recorder built the way the engine builds one.
/// The adopter's row and log stand. The fence reads as a taken lease: the caller gets `LeaseLost`
/// and the isolator releases the run's guards once, as a heartbeat's abandon would.
async fn a_suspended_walk_cannot_write_after_adoption<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let (stalled, wake) = orch.suspend_after_done("prd", 1, None);
    let mut walk = Box::pin(orch.dispatch(Command::StartRun {
        item: ids::HTUI_FEAT_3,
        mode: RunMode::Manual,
        repo_scope: None,
        workspace: None,
    }));
    if let Either::Left(_) =
        futures::future::select(walk.as_mut(), Box::pin(stalled.notified())).await
    {
        panic!("the walk finished instead of suspending");
    }
    let run = orch
        .store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|run| run.status == RunStatus::Running)
        .expect("the suspended walk left the run `running`")
        .id;
    let prd = step_at(&orch, run, 0, 1).await;

    let other = orch.restarted();
    assert_eq!(
        other.sweep().await.expect("the sweep adopts"),
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    let adopted = step_at(&other, run, 0, 1).await;
    assert_eq!(
        (adopted.status, adopted.gate_note.as_deref()),
        (StepStatus::Failed, Some("interrupted")),
        "the adopter settled `prd` as interrupted (plan D89)"
    );
    let log = orch
        .store()
        .step_events(prd.id)
        .await
        .expect("MemStore never fails a read");

    wake.notify_one();
    let woke = walk.await;
    assert!(
        matches!(&woke, Err(EngineError::LeaseLost { run: lost }) if *lost == run),
        "{woke:?}"
    );
    assert_eq!(
        orch.isolator().releases(),
        1,
        "the fenced walk's guards are released, once"
    );
    assert_eq!(
        step_at(&other, run, 0, 1).await,
        adopted,
        "the woken settle wrote nothing"
    );
    assert_eq!(
        orch.store()
            .step_events(prd.id)
            .await
            .expect("MemStore never fails a read"),
        log
    );

    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let mut recorder =
        htui_agent::record::Recorder::new(orch.store(), &scrubber, prd.id, false, None)
            .with_fence(StepFence::Lease(orch.owner()));
    let refused = recorder
        .record_prompt("woken", serde_json::json!([]), orch.clock().now())
        .await;
    assert!(
        matches!(&refused, Err(htui_agent::record::RecordError::Store(
            StoreError::Fenced { step })) if *step == prd.id),
        "{refused:?}"
    );
    assert_eq!(
        orch.store()
            .step_events(prd.id)
            .await
            .expect("MemStore never fails a read"),
        log
    );
    assert!(
        other
            .store()
            .refresh_lease(run, other.owner(), TimeDelta::minutes(1))
            .await
            .expect("MemStore refreshes"),
        "the adopter still holds the lease"
    );
}

/// MOD-41 plan D1, D2: a walk suspended after its session's `Done`, whose run another process
/// adopted meanwhile, wakes into stage 5 and records no commits. Its capture's
/// `record_commits(Lease(a))` is the first write after the wake, and the fence refuses it: the
/// walk ends `LeaseLost` through `heartbeaten`'s `is_fenced` arm, its guards are released once,
/// and the step's trees and commits are the ones the adopter found.
async fn a_walk_woken_after_done_records_no_commits<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let (stalled, wake) = orch.suspend_after_done("prd", 1, None);
    let mut walk = Box::pin(orch.dispatch(Command::StartRun {
        item: ids::HTUI_FEAT_3,
        mode: RunMode::Manual,
        repo_scope: None,
        workspace: None,
    }));
    if let Either::Left(_) =
        futures::future::select(walk.as_mut(), Box::pin(stalled.notified())).await
    {
        panic!("the walk finished instead of suspending");
    }
    let run = orch
        .store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|run| run.status == RunStatus::Running)
        .expect("the suspended walk left the run `running`")
        .id;
    let prd = step_at(&orch, run, 0, 1).await;
    let rows_of_step = async |step: StepId| {
        (
            orch.store()
                .step_trees(step)
                .await
                .expect("MemStore never fails a read"),
            orch.store()
                .step_commits(step)
                .await
                .expect("MemStore never fails a read"),
        )
    };
    let (trees, commits) = rows_of_step(prd.id).await;
    assert!(
        !trees.is_empty() && commits.iter().all(|row| row.after_hash.is_none()),
        "stage 2 recorded the tree and the base, and no capture yet: {trees:?} {commits:?}"
    );

    let other = orch.restarted();
    assert_eq!(
        other.sweep().await.expect("the sweep adopts"),
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    let adopted = rows_of_step(prd.id).await;

    wake.notify_one();
    let woke = walk.await;
    assert!(
        matches!(&woke, Err(EngineError::LeaseLost { run: lost }) if *lost == run),
        "{woke:?}"
    );
    assert_eq!(
        orch.isolator().releases(),
        1,
        "the fenced walk's guards are released, once"
    );
    assert_eq!(
        rows_of_step(prd.id).await,
        adopted,
        "the woken capture recorded no commits and no trees (MOD-41 plan D1)"
    );
    assert!(
        other
            .store()
            .refresh_lease(run, other.owner(), TimeDelta::minutes(1))
            .await
            .expect("MemStore refreshes"),
        "the adopter still holds the lease: the woken walk released none"
    );
}

// -- MOD-4 milestone 5: the recovery sweep (plan D89-D98, blueprint A-2, A-3) -----------------------

/// A process killed mid-walk (plan D100, blueprint A-2): `StartRun` on `item` is polled until
/// `stalled` fires and then dropped, and the run it left `running` is handed back.
///
/// # Panics
/// When the walk finishes instead of stalling, or leaves no `running` run on the item.
async fn crash<O: Orchestrate>(orch: &O, item: ItemId, stalled: &Notify) -> RunId {
    until_stalled(
        orch.dispatch(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        }),
        stalled,
    )
    .await;
    orch.store()
        .runs(item)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|run| run.status == RunStatus::Running)
        .expect("the crash left the run `running`")
        .id
}

/// Every phase of `FEAT-3` at `gate`, `mutate` applied after, and both review attempts scripted
/// to approve so an ungated walk reaches `done` (blueprint F-S: every seeded phase is `always`).
async fn feat_3_gated<O: Orchestrate>(orch: &O, gate: Gate, mutate: impl Fn(&mut StepGraphPhase)) {
    free_feat_3(orch).await;
    repoint(orch, ids::HTUI_FEAT_3, |phase| {
        phase.gate = gate;
        mutate(phase);
    })
    .await;
    for attempt in [1, 2] {
        orch.script("review", attempt, ScriptedStep::review("approve", "fine"));
    }
}

/// The step at `(position, attempt)` as the store has it now.
async fn step_at<O: Orchestrate>(orch: &O, run: RunId, position: i32, attempt: i32) -> RunStep {
    at(&steps_of(orch, run).await, position, attempt).clone()
}

/// `{repo} {path} before_hash {base_ref}`, blueprint §9.3's `{tree}` for a tree with no label.
async fn tree_text<O: Orchestrate>(orch: &O, step: StepId) -> String {
    orch.store()
        .step_trees(step)
        .await
        .expect("MemStore never fails a read")
        .iter()
        .map(|tree| {
            format!(
                "{} {} before_hash {}",
                tree.repo_id, tree.path, tree.base_ref
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The run's row and its steps, for "the sweep touched nothing".
async fn rows_of<O: Orchestrate>(orch: &O, run: RunId) -> (Run, Vec<RunStep>) {
    (run_of(orch, run).await, steps_of(orch, run).await)
}

/// ANA-2 §12 criterion 18, first half (`docs/ANA-2.md:2135-2136`, plan D90, D91): a step whose
/// session finished and whose artefacts both landed — the document and every repo's
/// `after_hash` — is adopted through the gate table it would have met. Ungated it is `done` and
/// the run walks on; gated it parks, and is not `done`.
async fn a_finished_step_is_adopted_through_its_gate<H: CaseHarness>(harness: &H) {
    for gate in [Gate::Never, Gate::Always] {
        let orch = harness.fresh();
        let repo = primary_repo(&orch).await;
        feat_3_gated(&orch, Gate::Never, |phase| {
            if phase.name == "prd" {
                phase.gate = gate;
            }
        })
        .await;
        let stalled = orch.stall_after_done("prd", 1, None, true);
        let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
        let prd = step_at(&orch, run, 0, 1).await;
        assert_eq!(prd.status, StepStatus::Running, "the crash left `prd` live");
        let before = orch
            .store()
            .step_commits(prd.id)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|row| row.repo_id == repo)
            .expect("stage 2 recorded the base")
            .before_hash;
        // The crash left the run leased to `orch`: the capture lands under its lease.
        orch.store()
            .record_commits(
                StepFence::Lease(orch.owner()),
                prd.id,
                &[htui_core::model::RunStepCommit {
                    run_step_id: prd.id,
                    repo_id: repo,
                    before_hash: before,
                    after_hash: Some("fake:after:x".to_owned()),
                }],
            )
            .await
            .expect("MemStore records the capture that landed");

        let other = orch.restarted();
        let adopted = other.sweep().await.expect("the sweep adopts");
        let prd = step_at(&other, run, 0, 1).await;
        if gate == Gate::Never {
            assert_eq!(
                adopted,
                [Adopted {
                    run,
                    next: Next::Walk
                }]
            );
            assert_eq!(prd.status, StepStatus::Done, "D91: re-settled and passed");
            assert_eq!(
                other.isolator().reconciles(),
                [(prd.id, Vec::new())],
                "D91's `Advance` merges the winner, and exactly once (blueprint A-3)"
            );
            let resumed = other.resume(run).await.expect("the adopter walks it");
            assert!(
                matches!(
                    resumed,
                    Resume::Walked(Rest {
                        run: RunStatus::Done,
                        ..
                    })
                ),
                "{resumed:?}"
            );
        } else {
            assert_eq!(adopted.len(), 1);
            assert!(
                matches!(
                    &adopted[0].next,
                    Next::Parked(Rest {
                        run: RunStatus::AwaitingApproval,
                        ..
                    })
                ),
                "{adopted:?}"
            );
            assert_eq!(
                prd.status,
                StepStatus::AwaitingApproval,
                "gated, so not `done`"
            );
            assert_eq!(
                run_of(&other, run).await.status,
                RunStatus::AwaitingApproval
            );
        }
    }
}

/// Criterion 18, second half (`:2136-2137`, plan D92): a step with no document is unfinished.
/// Its `worktree` tree is resettable (the reset touches nothing, OQ-8), so the step is failed
/// `interrupted`, a note names it and its tree, attempt 2 is admitted, and the walk finishes.
/// Base equality between the attempts is real git's to assert (blueprint F-D).
async fn an_unfinished_step_is_reset_and_retried<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    let trees = tree_text(&orch, prd.id).await;

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    let steps = steps_of(&other, run).await;
    let first = at(&steps, 0, 1);
    assert_eq!(
        (first.status, first.gate_note.as_deref(), first.gate_outcome),
        (StepStatus::Failed, Some("interrupted"), None),
        "plan D89: a crash is not a gate answer"
    );
    assert_eq!(at(&steps, 0, 2).status, StepStatus::Pending);
    assert_eq!(other.isolator().resets(), 1);
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    let n1 = format!(
        "interrupted: step {} (`prd` attempt 1) did not finish; trees: {trees}; retrying as \
         attempt 2",
        prd.id
    );
    assert!(notes.contains(&n1), "{n1}\n{notes:?}");

    let resumed = other.resume(run).await.expect("the adopter walks it");
    assert!(
        matches!(
            resumed,
            Resume::Walked(Rest {
                run: RunStatus::Done,
                ..
            })
        ),
        "{resumed:?}"
    );
}

/// Plan D92's out-of-budget branch and blueprint A-8: with `retry_limit = 0` the interrupted
/// step parks the run `interrupted: prd` rather than failing it, and a human's `RetryStep`
/// admits attempt 2 anyway — the budget bounds automatic retries, and a crash is not one.
async fn an_interrupted_step_out_of_budget_parks<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| phase.retry_limit = 0).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    let trees = tree_text(&orch, prd.id).await;

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Parked(Rest {
                run: RunStatus::AwaitingApproval,
                position: Some(0),
                failure: Some(RunFailure::Interrupted {
                    phase: "prd".to_owned(),
                    reset: true,
                }),
            }),
        }]
    );
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(
        run_of(&other, run).await.failure,
        None,
        "R-3: parked, not failed"
    );
    assert_eq!(
        state_of(&other, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou
    );
    let n3 = format!(
        "interrupted: retry budget spent: step {} (`prd` attempt 1); trees: {trees}; the run is \
         parked",
        prd.id
    );
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    assert!(notes.contains(&n3), "{n3}\n{notes:?}");

    let outcome = other
        .dispatch(Command::RetryStep { run, step: prd.id })
        .await
        .expect("A-8: an interrupted step is retryable past its budget");
    assert!(
        matches!(outcome, CommandOutcome::Retried { step, .. } if step == prd.id),
        "{outcome:?}"
    );
    assert_eq!(
        step_at(&other, run, 0, 2).await.status,
        StepStatus::AwaitingApproval,
        "attempt 2 was admitted and walked to its `always` gate"
    );
}

/// ANA-2 §12 criterion 12, the fake's half (`:2118-2119`, plan D93): a `local` tree recorded
/// dirty at step start is never reset. The step fails `interrupted, tree not reset`, the run
/// parks, the isolator is never asked, and the note names the tree's path and `before_hash`.
async fn a_dirty_tree_is_never_reset<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.isolation = Some(Isolation::Local);
        }
    })
    .await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    let mut trees = orch
        .store()
        .step_trees(prd.id)
        .await
        .expect("MemStore never fails a read");
    assert!(!trees.is_empty(), "stage 2 recorded the tree");
    for tree in &mut trees {
        tree.dirty = true;
    }
    orch.store()
        .upsert_step_tree(StepFence::Lease(orch.owner()), prd.id, &trees)
        .await
        .expect("MemStore rewrites the rows");
    let listed = tree_text(&orch, prd.id).await;

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(adopted.len(), 1);
    assert!(
        matches!(
            &adopted[0].next,
            Next::Parked(Rest {
                run: RunStatus::AwaitingApproval,
                failure: Some(RunFailure::Interrupted { reset: false, .. }),
                ..
            })
        ),
        "{adopted:?}"
    );
    let prd = step_at(&other, run, 0, 1).await;
    assert_eq!(
        (prd.status, prd.gate_note.as_deref()),
        (StepStatus::Failed, Some("interrupted, tree not reset"))
    );
    assert_eq!(other.isolator().resets(), 0, "no `git` write on this path");
    let n2 = format!(
        "interrupted, tree not reset: step {} (`prd` attempt 1); trees: {listed}; reason: dirty \
         at step start; the run is parked for a human",
        prd.id
    );
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    assert!(notes.contains(&n2), "{n2}\n{notes:?}");
    assert!(n2.contains(&trees[0].path) && n2.contains(&trees[0].base_ref));
}

/// Plan D97 (blueprint F-A, A-3): a crash between stage 6's `done` and the merge leaves an
/// unmerged winner the cursor would walk past. The adopter reconciles it exactly once before
/// the walk goes on.
async fn a_crash_before_reconcile_is_reconciled_on_adoption<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    let stalled = orch.isolator().stall_nth_reconcile(1);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::Done, "stage 6 moved it `done`");
    assert_eq!(run_of(&orch, run).await.status, RunStatus::Running);

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    assert_eq!(
        other.isolator().reconciles(),
        [(prd.id, Vec::new())],
        "exactly one reconcile of the winner (blueprint A-3)"
    );
}

/// Plan D97 as redefined (C129): after a review rejection the retired review of attempt 1 sits
/// at the next position, and the frontier is still implement attempt 2. The adopter reconciles
/// it once, and the walk runs review attempt 2 over it.
async fn a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled<
    H: CaseHarness,
>(
    harness: &H,
) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    // prd, plan and implement 1 each reconcile once; review 1 does not, because stage 6's
    // automatic rejection retires it through the loop and lands `Retired` (plan D135), which
    // merges nothing. Implement 2's is the fourth.
    let stalled = orch.isolator().stall_nth_reconcile(4);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let steps = steps_of(&orch, run).await;
    let implement = at(&steps, 2, 2).clone();
    assert_eq!(implement.status, StepStatus::Done);
    assert_eq!(at(&steps, 3, 1).status, StepStatus::Cancelled);
    assert_eq!(
        orch.isolator().reconciles().last(),
        Some(&(implement.id, Vec::new())),
        "the stalled reconcile is implement attempt 2's"
    );

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    assert_eq!(other.isolator().reconciles(), [(implement.id, Vec::new())]);

    let resumed = other.resume(run).await.expect("the adopter walks it");
    assert!(
        matches!(
            resumed,
            Resume::Walked(Rest {
                run: RunStatus::Done,
                ..
            })
        ),
        "{resumed:?}"
    );
    assert_eq!(step_at(&other, run, 3, 2).await.status, StepStatus::Done);
}

/// Plan D95: a `running` candidate with no document fails alone, `interrupted`, with no reset
/// and no park, and the judge then compares the two survivors.
async fn an_interrupted_candidate_fails_alone<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 0, "complete");
    let stalled = orch.stall_after_done("research", 1, Some((1, 0)), false);
    let run = crash(&orch, ids::HTUI_ANA_2, &stalled).await;

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );
    let steps = steps_of(&other, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1)
            .into_iter()
            .map(|(index, status, _)| (index, status))
            .collect::<Vec<_>>(),
        [
            (0, StepStatus::Done),
            (1, StepStatus::Failed),
            (2, StepStatus::Done),
        ]
    );
    assert_eq!(
        candidate(&steps, 0, 1, 1).gate_note.as_deref(),
        Some("interrupted")
    );
    assert_eq!(
        other.isolator().resets(),
        0,
        "D95: no reset for a candidate"
    );

    other.resume(run).await.expect("the adopter walks it");
    let steps = steps_of(&other, run).await;
    let judge = judge_of(&steps, 0, 1).expect("two survived, so the judge ran");
    let sections = prompt_sections(&other, judge.id).await;
    for (index, expected) in [(0, true), (1, false), (2, true)] {
        assert_eq!(
            sections
                .iter()
                .any(|section| *section == format!("judge_candidate:{index}")),
            expected,
            "candidate {index}: {sections:?}"
        );
    }
}

/// Plan D95 for the judge: a `running` judge is failed `interrupted`, never re-judged, and the
/// walk's `Select` arm parks the slot for a human with that reason (M4 D59).
async fn an_interrupted_judge_parks_for_selection<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 0, "complete");
    let stalled = orch.stall_after_done("research:judge", 1, Some((-1, 0)), false);
    let run = crash(&orch, ids::HTUI_ANA_2, &stalled).await;

    let other = orch.restarted();
    other.sweep().await.expect("the sweep adopts");
    let steps = steps_of(&other, run).await;
    let judge = judge_of(&steps, 0, 1).expect("the judge is a step");
    assert_eq!(
        (judge.status, judge.gate_note.as_deref()),
        (StepStatus::Failed, Some("interrupted"))
    );

    let resumed = other.resume(run).await.expect("the adopter walks it");
    assert!(
        matches!(
            resumed,
            Resume::Walked(Rest {
                run: RunStatus::AwaitingApproval,
                ..
            })
        ),
        "{resumed:?}"
    );
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );
    let parks = selection_parks(&other, ids::HTUI_ANA_2).await;
    assert!(
        parks
            .iter()
            .any(|body| body.contains(" awaits selection: interrupted; ")),
        "{parks:?}"
    );
}

/// Plan D96: a park written before R-5's one-transaction writer could leave a waiting step under
/// a `running` run. The sweep completes the park — run, then item — and moves no step.
async fn a_half_written_park_is_completed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let stalled = orch.stall_after_done("prd", 1, None, true);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    assert!(
        orch.store()
            .transition_step(
                prd.id,
                StepStatus::Running,
                StepStatus::AwaitingApproval,
                orch.clock().now()
            )
            .await
            .expect("MemStore takes the move"),
        "the park's first write landed"
    );
    let before = steps_of(&orch, run).await;

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(adopted.len(), 1);
    assert!(
        matches!(
            &adopted[0].next,
            Next::Parked(Rest {
                run: RunStatus::AwaitingApproval,
                ..
            })
        ),
        "{adopted:?}"
    );
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(
        state_of(&other, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou
    );
    assert_eq!(steps_of(&other, run).await, before, "no step moved");
}

/// ANA-2 §4.9 `:1306` and plan D88: the sweep adopts only an expired lease on a `running` run.
/// A parked run and a live lease are untouched, row for row — and a process never adopts its
/// own lease, however stale.
async fn the_sweep_never_touches_a_parked_run_or_a_live_lease<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    touch(&orch, ids::HTUI_FEAT_3, &["src/**"]).await;
    touch(&orch, ids::HTUI_ANA_2, &["docs/**"]).await;
    let (parked, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let stalled = orch.stall_after_done("research", 1, None, false);
    let live = crash(&orch, ids::HTUI_ANA_2, &stalled).await;

    let other = orch.restarted();
    assert!(
        orch.store()
            .refresh_lease(live, orch.owner(), TimeDelta::days(1) + RESTART_GAP)
            .await
            .expect("MemStore refreshes"),
        "the first process's heartbeat is still beating"
    );
    let before = (rows_of(&orch, parked).await, rows_of(&orch, live).await);

    assert_eq!(other.sweep().await.expect("the sweep runs"), []);
    assert_eq!(
        (rows_of(&orch, parked).await, rows_of(&orch, live).await),
        before
    );

    orch.clock().advance(TimeDelta::days(2));
    assert_eq!(
        orch.sweep().await.expect("the sweep runs"),
        [],
        "plan D88: never this process's own lease"
    );
    assert_eq!(
        (rows_of(&orch, parked).await, rows_of(&orch, live).await),
        before
    );
}

// -- MOD-4 milestone 6: the carried engine fixes (plan D159, D162, D179) ---------------------------

/// The sentence `assemble` refuses [`crate::fake::REFUSED_TEMPLATE_BODY`] with.
const REFUSED_PROMPT: &str = "unknown prompt placeholder: {{no_such_placeholder}}";

/// ANA-5 criterion 3 (`docs/ANA-5.md:2271-2273`, MOD-4 plan D162) at `walk_step`: stage 3's
/// assembler refuses `prd`'s own prompt. The step fails with no `gate_note`, the item is
/// `blocked` — not `failed`: the block lands before `finish_run`, which writes no item — the
/// run fails with the refusal, one note says so, and no session ever started.
async fn a_prompt_refusal_blocks_the_item_and_starts_no_session<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.refuse_prompt("prd");

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    let expected = RunFailure::PromptRefused {
        phase: "prd".to_owned(),
        reason: REFUSED_PROMPT.to_owned(),
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(0), Some(&expected))
    );
    let row = run_of(&orch, run).await;
    assert_eq!(
        row.failure.as_deref(),
        Some("prompt refused at `prd`: unknown prompt placeholder: {{no_such_placeholder}}")
    );
    assert!(row.finished_at.is_some());
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked,
        "ANA-5 criterion 3: the item is blocked"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 1, "stage 3 fails the one step it reached");
    assert_eq!(steps[0].status, StepStatus::Failed);
    assert_eq!(
        steps[0].gate_note, None,
        "the reason is the run's and the note's, not a gate note"
    );
    assert!(steps[0].prompt_digest.is_none(), "no prompt was recorded");
    assert!(
        orch.store()
            .step_events(steps[0].id)
            .await
            .expect("MemStore never fails a read")
            .unwrap_or_default()
            .is_empty(),
        "and no session started: zero `session_event` rows"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        notes
            .iter()
            .filter(|body| **body == expected.to_string())
            .count(),
        1,
        "one note a human reads: {notes:?}"
    );
    assert_eq!(
        harness_cleanups(&orch),
        1,
        "plan D36: a failed run is cleaned up"
    );
}

/// ANA-5 criterion 3 at `drive_group` (blueprint D195): the group's one prompt is refused before
/// any candidate is live, so every candidate goes `pending -> running -> failed`, the item is
/// `blocked`, the run fails with the refusal, and no candidate started a session.
async fn a_prompt_refusal_in_a_fan_out_blocks_the_item<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "research" {
            phase.fan_out = 2;
        }
    })
    .await;
    orch.refuse_prompt("research");

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    let expected = RunFailure::PromptRefused {
        phase: "research".to_owned(),
        reason: REFUSED_PROMPT.to_owned(),
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(0), Some(&expected))
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        slot_of(&steps, 0, 1),
        [(0, StepStatus::Failed, None), (1, StepStatus::Failed, None)],
        "both candidates failed before a token"
    );
    for step in &steps {
        assert!(step.prompt_digest.is_none());
        assert!(
            orch.store()
                .step_events(step.id)
                .await
                .expect("MemStore never fails a read")
                .unwrap_or_default()
                .is_empty(),
            "no candidate started a session"
        );
    }
    assert_eq!(
        item_of(&orch, ids::HTUI_ANA_2).await.status,
        Status::Blocked
    );
    assert_eq!(
        run_of(&orch, run).await.failure.as_deref(),
        Some(expected.to_string().as_str())
    );
    assert!(
        notes_of(&orch, ids::HTUI_ANA_2)
            .await
            .contains(&expected.to_string()),
        "the item's note names the refusal"
    );
}

/// ANA-2 §12 criterion 3 (`docs/ANA-2.md:2090-2091`, MOD-4 plan D159): a run another process's
/// sweep adopted as `Walk`, whose graph moved meanwhile, **parks** on resume — run and item
/// `awaiting_approval` — rather than being handed back to the next sweep as a `running` run
/// (R-36). It replaces milestone 5's engine test of the release-only answer. A **v1** run
/// (MOD-138 D5); `a_running_v2_run_walks_on_after_a_live_edit` is the v2 twin.
async fn a_topology_mismatch_parks_a_running_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    rewrite_as_v1(orch.store(), run).await;

    let other = orch.restarted();
    set_input_kinds(&other, ids::HTUI_FEAT_3, 0, &["spec"]).await;
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );

    let resumed = other.resume(run).await.expect("the run is readable");
    let Resume::TopologyChanged {
        snapshot,
        live,
        rest,
    } = resumed
    else {
        panic!("the live graph moved: {resumed:?}");
    };
    assert_ne!(snapshot, live);
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "parked, not left running"
    );
    let row = run_of(&other, run).await;
    assert_eq!(row.status, RunStatus::AwaitingApproval);
    assert_eq!(
        row.lease_expires_at,
        Some(other.clock().now()),
        "a parked run holds no lease"
    );
    assert_eq!(
        state_of(&other, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou
    );
    assert!(
        notes_of(&other, ids::HTUI_FEAT_3)
            .await
            .iter()
            .any(|body| body.contains("topology mismatch")),
        "the human reads why"
    );
    assert_eq!(
        other.sweep().await.expect("the sweep runs"),
        [],
        "R-36: no sweep adopts the parked run again"
    );
}

/// R-28 (MOD-4 plan D179): `CancelRun` takes the run's lease after its guard and before its first
/// write, so a live lease another process holds refuses it with `LeaseHeld` and nothing moves.
async fn cancel_meets_a_live_lease_and_writes_nothing<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let other = orch.restarted();
    assert!(
        orch.store()
            .take_lease(
                run,
                ids::BOX,
                orch.owner(),
                TimeDelta::days(1) + RESTART_GAP
            )
            .await
            .expect("MemStore takes the lease"),
        "the first process takes its released lease back, for a day"
    );
    let before = rows_of(&orch, run).await;

    let refused = other
        .dispatch(Command::CancelRun { run })
        .await
        .expect_err("a live lease elsewhere");
    assert!(
        matches!(refused, EngineError::LeaseHeld { run: held } if held == run),
        "{refused}"
    );
    assert_eq!(
        rows_of(&orch, run).await,
        before,
        "every step and the run are unchanged"
    );
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
    assert_eq!(harness_cleanups(&other), 0, "nothing was cleaned up");
}

// -- MOD-4 milestone 6: the maintainer drives it (plan D161, D163, D166, D167) -------------------

/// The HTUI project's active runs, graph and chat alike: a chat run is created `running`, so a
/// promotion that minted one would show here.
async fn active_runs_of<O: Orchestrate>(orch: &O) -> usize {
    orch.store()
        .active_runs(&Scope {
            workspace_id: WorkspaceId::new(),
            project_ids: vec![ids::PROJECT_HTUI],
        })
        .await
        .expect("MemStore never fails a read")
}

/// `PromoteStep` on `step`, unwrapped to its answer.
///
/// # Panics
/// When the promotion is refused, which every caller of this helper expects not to be.
async fn promote<O: Orchestrate>(
    orch: &O,
    run: RunId,
    step: StepId,
) -> (StepId, Rest, crate::command::Opening) {
    let outcome = orch
        .dispatch(Command::PromoteStep {
            run,
            step,
            chat_open: false,
        })
        .await
        .expect("the step is promotable");
    let CommandOutcome::Promoted {
        step,
        rest,
        opening,
    } = outcome
    else {
        panic!("`PromoteStep` answers `Promoted`, not {outcome:?}");
    };
    (step, rest, *opening)
}

/// ANA-2 §12 criterion 17, first half (`docs/ANA-2.md:2131-2134`, MOD-4 plan D163): `prd`
/// parked at its gate is promoted. The step keeps its id and its `prompt_digest`, gains
/// `promoted_at`, and waits `awaiting_approval` under a parked run (the item reads `needs you`).
/// No chat run is
/// minted, and the opening is a handoff — the fake's `claude` row does not resume (blueprint
/// D192) — in the step's own tree.
async fn promote_keeps_the_step_and_writes_no_chat_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let repo = primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::AwaitingApproval);
    let (runs_before, active_before) = (
        orch.store()
            .runs(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .len(),
        active_runs_of(&orch).await,
    );

    let (step, rest, opening) = promote(&orch, run, prd.id).await;
    assert_eq!(step, prd.id, "the same `run_step` (criterion 17)");
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    let after = step_at(&orch, run, 0, 1).await;
    assert_eq!(after.id, prd.id);
    assert_eq!(after.status, StepStatus::AwaitingApproval);
    assert!(after.promoted_at.is_some(), "`promote_step` stamped it");
    assert_eq!(
        after.prompt_digest, prd.prompt_digest,
        "the step's own prompt is never rewritten"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
    let runs = orch
        .store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(runs.len(), runs_before);
    assert!(runs.iter().all(|row| row.kind == RunKind::Graph));
    assert_eq!(
        active_runs_of(&orch).await,
        active_before,
        "zero `run(kind = 'chat')` rows: no run was minted"
    );
    assert_eq!(
        run_of(&orch, run).await.lease_expires_at,
        Some(orch.clock().now()),
        "a parked run holds no lease"
    );
    assert!(
        notes_of(&orch, ids::HTUI_FEAT_3).await.contains(&format!(
            "step {} (`prd` attempt 1) promoted to chat",
            prd.id
        )),
        "the promotion is a note"
    );

    let tree = orch
        .store()
        .step_trees(prd.id)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|tree| tree.repo_id == repo)
        .expect("stage 2 recorded the primary's tree");
    assert_eq!(
        (
            opening.agent_id,
            opening.agent_name.as_str(),
            opening.model.as_deref(),
            opening.phase.as_str()
        ),
        (ids::AGENT_CLAUDE, "claude", Some("sonnet"), "prd")
    );
    assert_eq!(opening.cwd, std::path::PathBuf::from(&tree.path));
    assert!(opening.extra_dirs.is_empty());
    // MOD-37 M5 (R-48): the seed `claude` row is ACP with `acp.session.resume`, so a step whose
    // log holds a banner resumes its own session, and the handoff rides along as the fallback.
    let banner = promote::banner(
        &orch
            .store()
            .step_events(prd.id)
            .await
            .expect("MemStore never fails a read")
            .unwrap_or_default(),
    );
    assert_eq!(
        banner,
        Some(AgentSessionRef::new(format!("fake-{}", prd.id))),
        "the fake walk recorded its banner"
    );
    assert!(
        matches!(
            &opening.path,
            OpeningPath::Resume { session_ref, text, .. }
                if session_ref.as_str() == format!("fake-{}", prd.id)
                    && text == promote::RESUME_OPENING
        ),
        "{:?}",
        opening.path
    );
    let (text, digest) = opening.path.handoff().expect("the fallback is built");
    assert!(!text.is_empty() && !digest.is_empty());
}

/// MOD-37 M4, R-49 closed by admission (maintainer amendment, 2026-10-03): a promoted
/// `shared_serialized` step's run is `awaiting_approval` for its chat's whole life, and
/// `claim_run`'s rule I (`NotIsolated`) refuses every other run on that repo of the box, in any
/// process. The second item declares disjoint paths, so rule P would admit it: the refusal is the
/// shared checkout's. The promoted run stays parked. What an in-process hold would add (a same-run
/// command between `Promoted` and the chat bind, a second promotion of the step, an isolator
/// rebuild) is recorded LOW in HANDOFF.
async fn a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.isolation = Some(Isolation::SharedSerialized);
        }
    })
    .await;
    touch(&orch, ids::HTUI_FEAT_3, &["src/**"]).await;
    touch(&orch, ids::HTUI_ANA_2, &["docs/**"]).await;

    let (first, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "`prd` parks at its gate"
    );
    let prd = step_at(&orch, first, 0, 1).await;
    let trees = orch
        .store()
        .step_trees(prd.id)
        .await
        .expect("MemStore never fails a read");
    assert!(
        !trees.is_empty()
            && trees
                .iter()
                .all(|tree| tree.mode == Isolation::SharedSerialized),
        "the parked step works in the shared checkout: {trees:?}"
    );
    let (step, rest, _) = promote(&orch, first, prd.id).await;
    assert_eq!(step, prd.id);
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    orch.clock().advance(TimeDelta::seconds(1));

    let (second, claim) = start_refused(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        claim,
        Claim::Overlaps {
            with: first,
            rule: OverlapRule::NotIsolated,
        },
        "the promoted step's shared checkout keeps the second run off the repo"
    );
    assert_eq!(run_of(&orch, second).await.status, RunStatus::Queued);
    assert_eq!(
        run_of(&orch, first).await.status,
        RunStatus::AwaitingApproval,
        "the promoted run is still parked"
    );
    assert!(
        step_at(&orch, first, 0, 1).await.promoted_at.is_some(),
        "the step is still promoted"
    );
}

/// `docs/ANA-2.md:1563`: a `failed` step under a parked run is promotable. Here it is a step the
/// sweep failed `interrupted` and parked out of budget (plan D92, D94); the promotion moves it
/// `failed -> awaiting_approval`, and the handoff says why it stopped.
async fn promote_a_failed_step_of_a_parked_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| phase.retry_limit = 0).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let other = orch.restarted();
    other.sweep().await.expect("the sweep adopts");
    let prd = step_at(&other, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::Failed);
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );

    let (step, rest, opening) = promote(&other, run, prd.id).await;
    assert_eq!(step, prd.id);
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let after = step_at(&other, run, 0, 1).await;
    assert_eq!(after.status, StepStatus::AwaitingApproval);
    assert!(after.promoted_at.is_some());
    let (text, _) = opening.path.handoff().expect("the handoff is built");
    assert!(
        text.contains("interrupted"),
        "the failure reason is the step's gate note: {text}"
    );
}

/// `docs/ANA-2.md:1563`: a finished run has no step to chat with. The refusal names the run, and
/// nothing is written.
async fn promote_refuses_a_terminal_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::Failed);
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::Failed);
    let before = rows_of(&orch, run).await;
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;

    let refused = orch
        .dispatch(Command::PromoteStep {
            run,
            step: prd.id,
            chat_open: false,
        })
        .await
        .expect_err("a finished run");
    assert!(
        matches!(
            refused,
            EngineError::RunStatus {
                status: RunStatus::Failed,
                ..
            }
        ),
        "{refused}"
    );
    assert_eq!(rows_of(&orch, run).await, before, "nothing was written");
    assert_eq!(notes_of(&orch, ids::HTUI_FEAT_3).await, notes);
}

/// MOD-4 plan D163's engine half: a walk dropped mid-session (the worker's preemption, a crash
/// here) leaves `prd` `running` under this process's own live lease. The promotion renews that
/// lease, parks the step `running -> awaiting_approval`, and stamps it.
async fn promote_moves_a_dropped_running_step_to_awaiting<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let stalled = orch.stall_after_done("prd", 1, None, true);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    assert_eq!(step_at(&orch, run, 0, 1).await.status, StepStatus::Running);

    let prd = step_at(&orch, run, 0, 1).await;
    let (step, rest, _) = promote(&orch, run, prd.id).await;
    assert_eq!(step, prd.id);
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let after = step_at(&orch, run, 0, 1).await;
    assert_eq!(after.status, StepStatus::AwaitingApproval);
    assert!(after.promoted_at.is_some());
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
}

/// `AcceptArtifact` on `step`, from a chat that has ended.
async fn accept<O: Orchestrate>(
    orch: &O,
    run: RunId,
    step: StepId,
) -> Result<CommandOutcome, EngineError> {
    orch.dispatch(Command::AcceptArtifact {
        run,
        step,
        chat_live: false,
    })
    .await
}

/// ANA-2 §12 criterion 17, second half (`docs/ANA-2.md:1223-1233`, MOD-4 plan D166): a promoted
/// `prd` is accepted. Its verify runs and is recorded, its tree is captured into one commit row,
/// the step is `done` with `approved`, the merge runs, and the walk goes on to `plan` at
/// `position + 1`, which parks at its gate.
async fn accept_artifact_verifies_captures_and_resumes_at_the_next_position<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    let repo = primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    promote(&orch, run, prd.id).await;
    orch.verifier().script_report(FakeVerifier::pass());

    let outcome = accept(&orch, run, prd.id)
        .await
        .expect("a promoted step with its document");
    let CommandOutcome::Accepted { rest } = outcome else {
        panic!("`AcceptArtifact` answers `Accepted`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the walk went on to `plan` and parked at its gate"
    );

    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(
        (prd.status, prd.gate_outcome, prd.verify_outcome),
        (
            StepStatus::Done,
            Some(GateOutcome::Approved),
            Some(VerifyOutcome::Pass)
        )
    );
    assert!(prd.promoted_at.is_some(), "it stays a promoted step");
    let commits = orch
        .store()
        .step_commits(prd.id)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(commits.len(), 1, "one repo, one commit row");
    assert_eq!(commits[0].repo_id, repo);
    assert!(commits[0].after_hash.is_some(), "the capture landed");
    assert_eq!(
        command_runs_of(&orch, prd.id)
            .await
            .iter()
            .map(|row| row.status)
            .collect::<Vec<_>>(),
        [CommandRunStatus::Done],
        "the verify is recorded"
    );
    assert_eq!(
        step_at(&orch, run, 1, 1).await.status,
        StepStatus::AwaitingApproval,
        "`plan` exists at position + 1"
    );
    assert!(
        orch.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == prd.id),
        "the accepted step was merged"
    );
}

/// `docs/ANA-2.md:1229-1231` (MOD-4 plan D166): accept is refused for a step that was never
/// promoted, for a promoted step with no document, and while a chat is live on the step — and
/// none of the three writes anything. Once the chat wrote the document (blueprint D203's author)
/// and ended, the same accept goes through.
async fn accept_artifact_needs_the_document_and_the_promotion<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    orch.script("prd", 1, ScriptedStep::done_without_output());
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::AwaitingApproval);

    let before = rows_of(&orch, run).await;
    let refused = accept(&orch, run, prd.id)
        .await
        .expect_err("never promoted");
    assert!(
        matches!(refused, EngineError::NotPromoted { step } if step == prd.id),
        "{refused}"
    );
    assert_eq!(rows_of(&orch, run).await, before, "nothing written");

    promote(&orch, run, prd.id).await;
    let before = rows_of(&orch, run).await;
    let refused = accept(&orch, run, prd.id)
        .await
        .expect_err("no `prd` document");
    assert!(
        matches!(&refused, EngineError::MissingOutputForApproval { kind, .. } if kind == "prd"),
        "{refused}"
    );
    assert_eq!(rows_of(&orch, run).await, before, "nothing written");

    orch.author(ids::HTUI_FEAT_3, &prd, "prd", "written in the chat")
        .await
        .expect("the store takes the document");
    let before = rows_of(&orch, run).await;
    let refused = orch
        .dispatch(Command::AcceptArtifact {
            run,
            step: prd.id,
            chat_live: true,
        })
        .await
        .expect_err("the chat on the step is live");
    assert!(
        matches!(refused, EngineError::ChatLive { step: Some(step) } if step == prd.id),
        "{refused}"
    );
    assert_eq!(rows_of(&orch, run).await, before, "nothing written");

    let outcome = accept(&orch, run, prd.id)
        .await
        .expect("the chat ended and left its document");
    assert!(
        matches!(outcome, CommandOutcome::Accepted { .. }),
        "{outcome:?}"
    );
    assert_eq!(step_at(&orch, run, 0, 1).await.status, StepStatus::Done);
}

/// Blueprint D194: an accept whose verify fails is refused with the exit code. The outcome is
/// recorded, a note says so, the step stays promoted and parked, the lease is given back, and
/// the walk does not go on.
async fn accept_artifact_refuses_a_failed_verify<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    promote(&orch, run, prd.id).await;
    orch.verifier().script_report(FakeVerifier::fail(1));

    let refused = accept(&orch, run, prd.id)
        .await
        .expect_err("the verify failed");
    assert!(
        matches!(
            refused,
            EngineError::AcceptVerifyFailed {
                step,
                exit_code: Some(1)
            } if step == prd.id
        ),
        "{refused}"
    );
    let after = step_at(&orch, run, 0, 1).await;
    assert_eq!(
        (after.status, after.verify_outcome, after.gate_outcome),
        (
            StepStatus::AwaitingApproval,
            Some(VerifyOutcome::Fail),
            None
        )
    );
    assert!(after.promoted_at.is_some(), "the step stays promoted");
    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::AwaitingApproval);
    assert_eq!(
        row.lease_expires_at,
        Some(orch.clock().now()),
        "the lease was given back"
    );
    assert_eq!(
        steps_of(&orch, run).await.len(),
        1,
        "the walk did not go on"
    );
    assert!(
        notes_of(&orch, ids::HTUI_FEAT_3)
            .await
            .iter()
            .any(|body| body.starts_with("accept refused: ")),
        "a human reads why"
    );
}

/// MOD-4 plan D211 (review H2): the accept's verify runs on a fresh copy of the phase deadline,
/// measured from the accept. A promoted step accepted three hours after its agent started —
/// past the seeded two-hour deadline — still hands the verifier the whole window; measured from
/// the step's `started_at` the remainder would be zero, which the real verifier answers
/// `unavailable` to without running the command, and the chat's edits would merge unverified.
async fn accept_artifact_verifies_on_a_deadline_from_the_accept<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    promote(&orch, run, prd.id).await;
    orch.clock().advance(TimeDelta::hours(3));
    orch.verifier().script_report(FakeVerifier::pass());

    let outcome = accept(&orch, run, prd.id)
        .await
        .expect("a promoted step with its document");
    assert!(
        matches!(outcome, CommandOutcome::Accepted { .. }),
        "{outcome:?}"
    );
    let windows = orch
        .verifier()
        .remaining()
        .into_iter()
        .filter(|(step, _)| *step == prd.id)
        .map(|(_, remaining)| remaining)
        .collect::<Vec<_>>();
    assert_eq!(windows.len(), 2, "the walk's verify, then the accept's");
    assert!(
        windows[0].is_some_and(|window| window > std::time::Duration::ZERO),
        "the phase has a deadline, whole when the walk verified: {windows:?}"
    );
    assert_eq!(
        windows[1], windows[0],
        "the accept's verify gets the whole deadline again, measured from the accept"
    );
    assert_eq!(
        step_at(&orch, run, 0, 1).await.verify_outcome,
        Some(VerifyOutcome::Pass)
    );
}

/// MOD-4 plan D211: an accept whose verify comes back `unavailable` is not refused — plan D30's
/// `unavailable` never fails a step, and some of its causes (`no primary tree`) are permanent —
/// but it is recorded, and a note tells the human the merge went in unverified.
async fn accept_artifact_notes_an_unavailable_verify<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = step_at(&orch, run, 0, 1).await;
    promote(&orch, run, prd.id).await;
    // `ShellVerifier`'s timed-out, signalled and cannot-wait reports carry the reason line and
    // then the command's captured tail; the tail belongs to `command_run.output`, not the note.
    let mut report = FakeVerifier::unavailable("verify_command timed out");
    report.output = format!("{}\nrunning 12 tests\ntest parse ... ok", report.output);
    orch.verifier().script_report(report);

    let outcome = accept(&orch, run, prd.id)
        .await
        .expect("an unavailable verify does not refuse the accept");
    let CommandOutcome::Accepted { rest } = outcome else {
        panic!("`AcceptArtifact` answers `Accepted`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the walk went on to `plan`"
    );
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(
        (prd.status, prd.gate_outcome, prd.verify_outcome),
        (
            StepStatus::Done,
            Some(GateOutcome::Approved),
            Some(VerifyOutcome::Unavailable)
        )
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.contains(&"accept: verify unavailable: verify_command timed out".to_owned()),
        "a human reads that the merge went in unverified, and only the reason: {notes:?}"
    );
    assert!(
        !notes.iter().any(|note| note.contains("running 12 tests")),
        "the command's tail stays in `command_run.output`: {notes:?}"
    );
}

/// `Unblock` on `item`, unwrapped to its case and rest.
///
/// # Panics
/// When the command is refused, which every caller of this helper expects not to be.
async fn unblock<O: Orchestrate>(orch: &O, item: ItemId) -> (UnblockCase, Option<Rest>) {
    let outcome = orch
        .dispatch(Command::Unblock { item })
        .await
        .expect("the item is held by something `Unblock` clears");
    let CommandOutcome::Unblocked {
        item: unblocked,
        case,
        rest,
    } = outcome
    else {
        panic!("`Unblock` answers `Unblocked`, not {outcome:?}");
    };
    assert_eq!(unblocked, item);
    (case, rest)
}

/// Rung 4's `Unblock` round trip (MOD-4 plan D161 case 1): rung 4 at `StartRun` leaves the item
/// `blocked` with no run; `Unblock` reopens it, and once the phase has a candidate again
/// `StartRun` walks it. Criterion 14 itself is
/// `a_capability_refusal_writes_no_run_and_unblock_reopens` (MOD-7 milestone 3, D86).
async fn unblock_opens_a_blocked_item_with_no_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    orch.with_candidates("prd", Vec::new());
    orch.dispatch(Command::StartRun {
        item: ids::HTUI_FEAT_3,
        mode: RunMode::Manual,
        repo_scope: None,
        workspace: None,
    })
    .await
    .expect_err("rung 4 refuses before a run row exists");
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );

    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((case, rest), (UnblockCase::Reopen, None));
    assert_eq!(item_of(&orch, ids::HTUI_FEAT_3).await.status, Status::Open);
    assert!(
        notes_of(&orch, ids::HTUI_FEAT_3)
            .await
            .contains(&"unblocked: back to `open`".to_owned())
    );
    let refused = orch
        .dispatch(Command::Unblock {
            item: ids::HTUI_FEAT_3,
        })
        .await
        .expect_err("an open item is not blocked");
    assert!(
        matches!(
            refused,
            EngineError::NotBlocked {
                state: ItemState::Open,
                ..
            }
        ),
        "{refused}"
    );

    orch.with_candidates("prd", vec![(ids::AGENT_CLAUDE, "sonnet")]);
    let (_, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        rest.run,
        RunStatus::AwaitingApproval,
        "`R` starts a new run on the reopened item"
    );
}

/// R-4 (MOD-4 plan D161 case 2, `docs/ANA-2.md:551`): the review loop escalates — the item
/// `blocked`, the run parked over the rejected review — and no gate verb reaches the run. `Unblock`
/// moves the item back to `open`, where its parked run reads `needs you` (MOD-144); the review is
/// then promoted and approved, and the run finishes.
async fn unblock_lets_an_escalated_run_be_promoted_and_approved<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    primary_repo(&orch).await;
    orch.script("review", 1, ScriptedStep::review("approve", "first"));
    orch.script("review", 2, ScriptedStep::review("approve", "second"));
    for hash in ["prd", "plan", "h1", "review-1", "h2", "review-2"] {
        orch.isolator().script_after(Some(hash));
    }
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve(&orch, run, 3).await;
    for note in ["no tests", "still no tests"] {
        answer(
            &orch,
            run,
            GateAnswer::Rejected {
                note: note.to_owned(),
            },
        )
        .await;
        if note == "no tests" {
            approve(&orch, run, 1).await;
        }
    }
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked,
        "the loop escalated"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    let review = step_at(&orch, run, 3, 2).await;
    assert_eq!(review.status, StepStatus::Failed);

    let refused = orch
        .dispatch(Command::PromoteStep {
            run,
            step: review.id,
            chat_open: false,
        })
        .await
        .expect_err("a blocked item is not walked on");
    assert!(
        matches!(refused, EngineError::ItemBlocked { .. }),
        "{refused}"
    );

    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((case, rest), (UnblockCase::FollowRun(run), None));
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "MOD-144: FollowRun moves the item `blocked -> open`"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "its parked run reads `needs you`"
    );

    let (step, _, _) = promote(&orch, run, review.id).await;
    assert_eq!(step, review.id);
    let outcome = orch
        .dispatch(Command::AnswerGate {
            run,
            step: review.id,
            answer: GateAnswer::Approved,
        })
        .await
        .expect("the promoted review has its document");
    let CommandOutcome::Answered { rest } = outcome else {
        panic!("`AnswerGate` answers `Answered`, not {outcome:?}");
    };
    assert_eq!(
        rest.run,
        RunStatus::Done,
        "the review was the last position"
    );
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Done);
}

/// R-7 (MOD-4 plan D161 case 3): a refused reconcile parks the run over a `done` step, which
/// no gate verb reaches. `Unblock` resumes it: the merge is tried again, and the walk goes on to
/// `plan`'s gate.
async fn unblock_resumes_a_reconcile_refused_park<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    orch.isolator().refuse_reconcile("dirty_primary_tree");
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(prd.status, StepStatus::Done);
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);

    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(case, UnblockCase::Resume(run));
    assert_eq!(
        rest,
        Some(Rest {
            run: RunStatus::AwaitingApproval,
            position: Some(1),
            failure: None,
        }),
        "the walk went on to `plan`'s gate"
    );
    assert_eq!(
        orch.isolator().reconciles(),
        [(prd.id, Vec::new()), (prd.id, Vec::new())],
        "refused once, merged on the resume"
    );
    assert_eq!(
        step_at(&orch, run, 1, 1).await.status,
        StepStatus::AwaitingApproval
    );
    assert!(
        notes_of(&orch, ids::HTUI_FEAT_3)
            .await
            .contains(&format!("unblocked: resuming run {run}"))
    );
}

/// The item's `summary` document heads.
async fn summaries_of<O: Orchestrate>(
    orch: &O,
    item: ItemId,
) -> Vec<htui_core::model::DocumentHead> {
    orch.store()
        .documents(item)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .filter(|head| head.kind == "summary")
        .collect()
}

/// ANA-2 §12 criterion 20 (`docs/ANA-2.md:2141-2143`, MOD-4 plan D167): a `done` item is closed
/// out. One `summary` document lands at version 1, produced by no step, with a table row per
/// `(repo, step)` that committed; the item is `closed` as the command's resolution with
/// `closed_at` set.
async fn close_out_writes_one_summary_and_closes_the_item<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let rest = approve(&orch, run, 4).await;
    assert_eq!(rest.run, RunStatus::Done);
    let key = item_of(&orch, ids::HTUI_FEAT_3).await.key;

    let outcome = orch
        .dispatch(Command::CloseOut {
            item: ids::HTUI_FEAT_3,
            resolution: Resolution::Done,
        })
        .await
        .expect("a done item with no live run");
    let CommandOutcome::ClosedOut {
        item,
        summary,
        version,
    } = outcome
    else {
        panic!("`CloseOut` answers `ClosedOut`, not {outcome:?}");
    };
    assert_eq!((item, version), (ids::HTUI_FEAT_3, 1));
    let document = orch
        .store()
        .document(summary)
        .await
        .expect("MemStore never fails a read")
        .expect("the summary was written");
    assert_eq!(
        (
            document.kind.as_str(),
            document.version,
            document.produced_by_step_id
        ),
        ("summary", 1, None)
    );
    assert_eq!(document.title, format!("Close-out {key}"));
    assert_eq!(
        document
            .body
            .lines()
            .filter(|line| line.starts_with("| htui | "))
            .count(),
        4,
        "one row per step, each committed to the primary:\n{}",
        document.body
    );
    assert_eq!(summaries_of(&orch, ids::HTUI_FEAT_3).await.len(), 1);
    let row = item_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(row.status, Status::Closed);
    assert_eq!(
        row.resolution,
        Some(Resolution::Done),
        "the command's resolution (MOD-38 D2)"
    );
    assert!(row.closed_at.is_some());
}

/// Criterion 20's refusal: a run of the item is live, so close-out names it, and neither a
/// document nor a move is written.
async fn close_out_is_refused_while_a_run_is_live<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;

    let refused = orch
        .dispatch(Command::CloseOut {
            item: ids::HTUI_FEAT_3,
            resolution: Resolution::Done,
        })
        .await
        .expect_err("a live run");
    assert!(
        matches!(
            refused,
            EngineError::RunStatus {
                run: live,
                status: RunStatus::AwaitingApproval,
                ..
            } if live == run
        ),
        "{refused}"
    );
    assert!(summaries_of(&orch, ids::HTUI_FEAT_3).await.is_empty());
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
}

// -- MOD-41 T9: the hand-back (plan D12, OQ-1, OQ-6) ---------------------------------------------

/// A step as the hand-back cases compare two runs' rows: `(position, attempt, fanout_index,
/// phase_name, status)`. Ids, instants and notes differ between two harnesses by construction.
type Shape = (i32, i32, i32, String, StepStatus);

/// The run's rows as [`Shape`]s, in `(position, attempt, fanout_index)` order.
fn shape(steps: &[RunStep]) -> Vec<Shape> {
    steps
        .iter()
        .map(|step| {
            (
                step.position,
                step.attempt,
                step.fanout_index,
                step.phase_name.clone(),
                step.status,
            )
        })
        .collect()
}

/// The command a hand-back case compares (MOD-41 plan D12's table).
#[derive(Debug, Clone, Copy)]
enum Act {
    /// `a`: the one parked step approved.
    Approve,
    /// `x`: the one parked step rejected.
    Reject,
    /// `s`: candidate `index` of the parked slot at `position`, attempt 1.
    Select { position: i32, index: i32 },
    /// `r`: the latest attempt at `position` (fan-out index 0 of a group).
    Retry { position: i32 },
    /// `u` in its resume case, on `item`.
    Unblock(ItemId),
    /// `A`: the promoted step at position 0, attempt 1.
    Accept,
}

/// `what` on `run`, unwrapped to the rest its answer carries.
///
/// # Panics
/// When the command is refused, which no hand-back case expects.
async fn act<O: Orchestrate>(orch: &O, run: RunId, what: Act) -> Rest {
    match what {
        Act::Approve => answer(orch, run, GateAnswer::Approved).await.1,
        Act::Reject => {
            answer(
                orch,
                run,
                GateAnswer::Rejected {
                    note: "not this".to_owned(),
                },
            )
            .await
            .1
        }
        Act::Select { position, index } => {
            let winner = candidate(&steps_of(orch, run).await, position, 1, index).id;
            let outcome = orch
                .dispatch(Command::SelectFanout {
                    run,
                    position,
                    attempt: 1,
                    winner,
                })
                .await
                .expect("the parked slot is selectable");
            let CommandOutcome::Selected { rest } = outcome else {
                panic!("`SelectFanout` answers `Selected`, not {outcome:?}");
            };
            rest
        }
        Act::Retry { position } => {
            let steps = steps_of(orch, run).await;
            let latest = steps
                .iter()
                .filter(|step| step.position == position && step.fanout_index >= 0)
                .max_by_key(|step| (step.attempt, std::cmp::Reverse(step.fanout_index)))
                .expect("the position has a step to retry");
            let outcome = orch
                .dispatch(Command::RetryStep {
                    run,
                    step: latest.id,
                })
                .await
                .expect("the step is retryable");
            let CommandOutcome::Retried { rest, .. } = outcome else {
                panic!("`RetryStep` answers `Retried`, not {outcome:?}");
            };
            rest
        }
        Act::Unblock(item) => {
            let (case, rest) = unblock(orch, item).await;
            assert_eq!(case, UnblockCase::Resume(run));
            rest.expect("the resume case answers a rest")
        }
        Act::Accept => {
            let prd = step_at(orch, run, 0, 1).await;
            let outcome = accept(orch, run, prd.id)
                .await
                .expect("a promoted step with its document");
            let CommandOutcome::Accepted { rest } = outcome else {
                panic!("`AcceptArtifact` answers `Accepted`, not {outcome:?}");
            };
            rest
        }
    }
}

/// The hand-back's own half (MOD-41 plan D12): the command's window is written and its answer is
/// the run `running` with its lease released, so no process holds it and A's owner is gone.
async fn assert_handed_back<O: Orchestrate>(a: &O, run: RunId, rest: &Rest) {
    assert_eq!(
        rest.run,
        RunStatus::Running,
        "plan D12: the hand-back walks no tail; the run is left `running` for the box's worker"
    );
    let row = run_of(a, run).await;
    assert_eq!(row.status, RunStatus::Running);
    assert!(
        row.lease_expires_at
            .is_some_and(|until| until <= a.clock().now()),
        "the lease was released: {:?}",
        row.lease_expires_at
    );
    assert!(
        !a.store()
            .refresh_lease(run, a.owner(), TimeDelta::minutes(1))
            .await
            .expect("MemStore refreshes"),
        "and it is no longer the handing process's"
    );
}

/// The adopter's half (MOD-41 plan D12): `b.sweep()` adopts `run`, and a [`Next::Walk`] is
/// walked through `b.resume(run)`, as the runtime does (`sweep` alone never walks). A recovery
/// that lands the run itself — a park or a finish — answers [`Resume::Walked`] with that rest.
///
/// # Panics
/// When the sweep does not adopt the run, or its recovery errs.
async fn adopted<O: Orchestrate>(b: &O, run: RunId) -> Resume {
    let adopted = b.sweep().await.expect("the sweep adopts");
    let next = adopted
        .into_iter()
        .find(|adopted| adopted.run == run)
        .unwrap_or_else(|| panic!("the handed-back run {run} was adopted"))
        .next;
    match next {
        Next::Walk => b.resume(run).await.expect("the adopter walks it"),
        Next::Parked(rest) | Next::Finished(rest) => Resume::Walked(rest),
        Next::Error(err) => panic!("the adopter could not recover the handed-back run: {err}"),
    }
}

/// MOD-41 plan D12, R-1: `what` in-process on `control`, and handed back on `a` then adopted and
/// walked by `a.restarted()`, rest where they rest and write the same rows. Answers the adopter.
async fn hand_back_matches<O: Orchestrate>(
    control: (&O, RunId),
    handed: (&O, RunId),
    what: Act,
) -> O {
    let (c, c_run) = control;
    let expected = act(c, c_run, what).await;
    let expected_steps = shape(&steps_of(c, c_run).await);

    let (a, run) = handed;
    a.set_tails(Tails::HandBack);
    let rest = act(a, run, what).await;
    assert_handed_back(a, run, &rest).await;

    let b = a.restarted();
    let Resume::Walked(rest) = adopted(&b, run).await else {
        panic!("{what:?}: the live graph did not move, so the adopter walks its snapshot");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (expected.run, expected.position, expected.failure),
        "{what:?}: the adopter rests the run where the in-process command did"
    );
    assert_eq!(
        shape(&steps_of(&b, run).await),
        expected_steps,
        "{what:?}: and writes the same rows"
    );
    b
}

/// `FEAT-3` started and parked at `prd`'s gate.
async fn parked_at_prd<O: Orchestrate>(orch: &O) -> RunId {
    free_feat_3(orch).await;
    start(orch, ids::HTUI_FEAT_3).await.0
}

/// How many merges of `step` the two processes' isolators recorded between them.
fn merges_of<O: Orchestrate>(a: &O, b: &O, step: StepId) -> usize {
    a.isolator()
        .reconciles()
        .iter()
        .chain(b.isolator().reconciles().iter())
        .filter(|(winner, _)| *winner == step)
        .count()
}

/// MOD-41 plan D12 (`a`): an approval handed back is finished by the adopter's frontier
/// reconcile, which merges `prd`, and its walk parks at `plan`'s gate, as the in-process tail did.
async fn an_approval_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_at_prd(&control).await;
    let run = parked_at_prd(&a).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Approve,
    ))
    .await;
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        b.isolator().reconciles(),
        [(prd.id, Vec::new())],
        "the adopter's frontier reconcile finished the approval (plan D97)"
    );
}

/// `FEAT-3` walked to its first `review` gate, both review attempts scripted to approve.
async fn parked_at_review<O: Orchestrate>(orch: &O) -> RunId {
    free_feat_3(orch).await;
    orch.script("review", 1, ScriptedStep::review("approve", "first"));
    orch.script("review", 2, ScriptedStep::review("approve", "second"));
    let run = start(orch, ids::HTUI_FEAT_3).await.0;
    approve(orch, run, 3).await;
    run
}

/// MOD-41 plan D12 (`x`): a rejection handed back is finished by D131's `settle_failed`. A
/// loopable review's rejection runs the review loop and parks the next `implement`; a rejection
/// with no loop fails the run `rejected`. Both as the in-process tail.
async fn a_rejection_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_at_review(&control).await;
    let run = parked_at_review(&a).await;
    let b = hand_back_matches((&control, c_run), (&a, run), Act::Reject).await;
    assert_eq!(
        step_at(&b, run, 2, 2).await.status,
        StepStatus::AwaitingApproval,
        "the loop resumed and parked the re-run implement"
    );

    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_at_prd(&control).await;
    let run = parked_at_prd(&a).await;
    let b = hand_back_matches((&control, c_run), (&a, run), Act::Reject).await;
    let row = run_of(&b, run).await;
    assert_eq!(row.status, RunStatus::Failed, "no loop: the run ends");
    assert_eq!(
        row.failure,
        Some(
            RunFailure::Rejected {
                phase: "prd".to_owned()
            }
            .to_string()
        )
    );
}

/// `ANA-2`'s `research` fanned out three ways with no judge, parked for a human's selection.
async fn selection_parked<O: Orchestrate>(orch: &O) -> RunId {
    fan_research(orch, Gate::Never, false).await;
    research_candidates(orch, 1);
    let (run, rest) = start(orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    run
}

/// MOD-41 plan D12 (`s`): a selection handed back is finished by the adopter's frontier
/// reconcile, which merges the winner over its siblings, and the walk finishes the run.
async fn a_selection_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = selection_parked(&control).await;
    let run = selection_parked(&a).await;
    let what = Act::Select {
        position: 0,
        index: 1,
    };

    let b = hand_back_matches((&control, c_run), (&a, run), what).await;
    let winner = candidate(&steps_of(&b, run).await, 0, 1, 1).id;
    let merges: Vec<_> = b
        .isolator()
        .reconciles()
        .into_iter()
        .filter(|(merged, _)| *merged == winner)
        .collect();
    assert_eq!(
        merges.len(),
        1,
        "the human's pick was merged once: {merges:?}"
    );
    assert_eq!(merges[0].1.len(), 2, "over its two siblings");
}

/// `FEAT-3`'s `prd` crashed with a `local` tree recorded dirty, and a second process's sweep
/// parked the run over the failed step (plan D93): the process that acts next, and the run.
async fn parked_over_a_dirty_tree<H: CaseHarness>(harness: &H) -> (H::Orch, RunId) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.isolation = Some(Isolation::Local);
        }
    })
    .await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let prd = step_at(&orch, run, 0, 1).await;
    let mut trees = orch
        .store()
        .step_trees(prd.id)
        .await
        .expect("MemStore never fails a read");
    for tree in &mut trees {
        tree.dirty = true;
    }
    orch.store()
        .upsert_step_tree(StepFence::Lease(orch.owner()), prd.id, &trees)
        .await
        .expect("MemStore rewrites the rows");
    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert!(
        matches!(
            &adopted[..],
            [Adopted {
                next: Next::Parked(_),
                ..
            }]
        ),
        "{adopted:?}"
    );
    assert_eq!(step_at(&other, run, 0, 1).await.status, StepStatus::Failed);
    (other, run)
}

/// MOD-41 plan D12 (`r` on a `failed` step): the retry's `admit` is in the handed-back window,
/// so the adopter walks the new attempt rather than reading D131's unfinished failure.
async fn a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter<H: CaseHarness>(
    harness: &H,
) {
    let (control, c_run) = parked_over_a_dirty_tree(harness).await;
    let (a, run) = parked_over_a_dirty_tree(harness).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Retry { position: 0 },
    ))
    .await;
    assert_eq!(
        step_at(&b, run, 0, 2).await.status,
        StepStatus::AwaitingApproval,
        "attempt 2 was walked to its gate"
    );
}

/// `FEAT-3` parked over a reconcile its isolator refused (R-7): `prd` is `done`, unmerged.
async fn parked_over_a_refused_reconcile<O: Orchestrate>(orch: &O) -> RunId {
    primary_repo(orch).await;
    free_feat_3(orch).await;
    repoint(orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    orch.isolator().refuse_reconcile("dirty_primary_tree");
    let (run, rest) = start(orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    run
}

/// MOD-41 plan D12 (`u`, resume case): the handed-back unblock only unparks, and the adopter's
/// `resume` merges `prd` and walks on to `plan`'s gate, as the in-process resume did.
async fn a_resume_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_over_a_refused_reconcile(&control).await;
    let run = parked_over_a_refused_reconcile(&a).await;
    assert_eq!(state_of(&a, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Unblock(ids::HTUI_FEAT_3),
    ))
    .await;
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        b.isolator().reconciles(),
        [(prd.id, Vec::new())],
        "the adopter merged `prd` once"
    );
}

/// `FEAT-3` parked at `prd`'s gate and rejected by a process that crashed before its unpark
/// (MOD-37 R-31): `prd` is `failed` + `rejected`, the run still `awaiting_approval` (the item
/// reads `needs you`).
async fn parked_over_a_crashed_rejection<O: Orchestrate>(orch: &O) -> RunId {
    let run = parked_at_prd(orch).await;
    let prd = step_at(orch, run, 0, 1).await;
    assert!(
        orch.store()
            .answer_gate(
                prd.id,
                GateOutcome::Rejected,
                Some("not like this".to_owned()),
                orch.clock().now()
            )
            .await
            .expect("MemStore takes the answer"),
        "the rejection's first write landed and its unpark did not"
    );
    assert_eq!(run_of(orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(state_of(orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
    run
}

/// MOD-37 R-31 (review L1): `u` handed back over a rejection a crash left parked unparks it
/// (`hand_back_resume` reads `resumable`, not only `resumable_park`), and the adopter's recovery
/// owes the rejection through plan D131's `settle_failed`: `prd` cannot loop, so the run fails
/// `Rejected`, as the in-process resume did.
async fn a_crashed_rejection_handed_back_is_failed_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_over_a_crashed_rejection(&control).await;
    let run = parked_over_a_crashed_rejection(&a).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Unblock(ids::HTUI_FEAT_3),
    ))
    .await;
    let row = run_of(&b, run).await;
    assert_eq!(
        row.status,
        RunStatus::Failed,
        "the adopter owed the rejection"
    );
    assert_eq!(
        row.failure,
        Some(
            RunFailure::Rejected {
                phase: "prd".to_owned()
            }
            .to_string()
        )
    );
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        (prd.status, prd.gate_outcome, prd.gate_note.as_deref()),
        (
            StepStatus::Failed,
            Some(GateOutcome::Rejected),
            Some("not like this")
        ),
        "the crashed rejection's row is kept"
    );
    assert_eq!(
        state_of(&b, ids::HTUI_FEAT_3).await,
        ItemState::Failed,
        "the item reads the run's `failed` after the unpark (MOD-144)"
    );
}

/// `FEAT-3`'s `prd` promoted at its gate, with a passing verify scripted for the accept.
async fn promoted_prd<O: Orchestrate>(orch: &O) -> RunId {
    primary_repo(orch).await;
    let run = parked_at_prd(orch).await;
    let prd = step_at(orch, run, 0, 1).await;
    promote(orch, run, prd.id).await;
    orch.verifier().script_report(FakeVerifier::pass());
    run
}

/// MOD-41 plan D12 (`A`): the accept's verify runs in the handing process (its window), then its
/// approval is handed back and finished by the adopter.
async fn an_accept_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = promoted_prd(&control).await;
    let run = promoted_prd(&a).await;

    let b = hand_back_matches((&control, c_run), (&a, run), Act::Accept).await;
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        (prd.gate_outcome, prd.verify_outcome),
        (Some(GateOutcome::Approved), Some(VerifyOutcome::Pass)),
        "the verify ran in the handing process"
    );
    assert!(
        b.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == prd.id),
        "the adopter merged the accepted step"
    );
}

/// MOD-41 plan D12 (`r` on an `awaiting_approval` step): the new attempt is admitted inside the
/// window, so the adopter walks it and never reconciles the previous winner a second time.
async fn a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter<H: CaseHarness>(
    harness: &H,
) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = parked_at_prd(&control).await;
    approve(&control, c_run, 1).await;
    let run = parked_at_prd(&a).await;
    approve(&a, run, 1).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Retry { position: 1 },
    ))
    .await;
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        merges_of(&a, &b, prd.id),
        1,
        "the previous winner was merged once, by its own approval"
    );
}

/// `ANA-2` with `research` ungated and `verdict` fanned out three ways with no judge: the walk
/// merges `research` and parks the `verdict` slot for a human.
async fn verdict_slot_parked<O: Orchestrate>(orch: &O) -> RunId {
    repoint(orch, ids::HTUI_ANA_2, |phase| {
        phase.gate = Gate::Never;
        if phase.name == "verdict" {
            phase.fan_out = 3;
        }
    })
    .await;
    let (run, rest) = start(orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the `verdict` slot awaits a human"
    );
    run
}

/// MOD-41 plan D12 (`r` on a group): the slot is retired and the next group admitted inside the
/// window, so the adopter walks it and never reconciles the previous winner a second time.
async fn a_group_retry_handed_back_is_walked_by_the_adopter<H: CaseHarness>(harness: &H) {
    let (control, a) = (harness.fresh(), harness.fresh());
    let c_run = verdict_slot_parked(&control).await;
    let run = verdict_slot_parked(&a).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Retry { position: 1 },
    ))
    .await;
    let research = step_at(&b, run, 0, 1).await;
    assert_eq!(
        merges_of(&a, &b, research.id),
        1,
        "the previous winner was merged once, by its own walk"
    );
    assert_eq!(
        slot_of(&steps_of(&b, run).await, 1, 2).len(),
        3,
        "the next group was admitted whole"
    );
}

/// `FEAT-3` with no retry budget, whose `prd` crashed unfinished and was parked `interrupted` by
/// a second process's sweep (plan D92, blueprint A-8): the process that acts next, and the run.
async fn parked_interrupted_out_of_budget<H: CaseHarness>(harness: &H) -> (H::Orch, RunId) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| phase.retry_limit = 0).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert!(
        matches!(
            &adopted[..],
            [Adopted {
                next: Next::Parked(Rest {
                    failure: Some(RunFailure::Interrupted { .. }),
                    ..
                }),
                ..
            }]
        ),
        "{adopted:?}"
    );
    (other, run)
}

/// MOD-41 plan D12 (`r` past the budget): without the window's `admit`, the adopter would read
/// the retried step as D131's unfinished failure and re-park it `interrupted`. With it, the new
/// attempt walks, as in-process.
async fn a_retry_past_the_budget_handed_back_is_walked_not_reparked<H: CaseHarness>(harness: &H) {
    let (control, c_run) = parked_interrupted_out_of_budget(harness).await;
    let (a, run) = parked_interrupted_out_of_budget(harness).await;

    let b = Box::pin(hand_back_matches(
        (&control, c_run),
        (&a, run),
        Act::Retry { position: 0 },
    ))
    .await;
    assert_eq!(
        step_at(&b, run, 0, 2).await.status,
        StepStatus::AwaitingApproval,
        "attempt 2 walked to its gate; the run was not re-parked `interrupted`"
    );
    assert_eq!(run_of(&b, run).await.failure, None);
}

/// MOD-41 OQ-6, blueprint F-24: the adopter's tail is the crash path. A handed-back approval
/// whose item's graph gained a phase meanwhile is merged by the frontier reconcile first, and
/// then the resume's topology gate parks the run and the item with one `topology mismatch` note.
/// A **v1** run (MOD-138 D5); `a_handed_back_v2_run_on_a_changed_graph_walks_on` is the v2 twin.
async fn a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note<H: CaseHarness>(
    harness: &H,
) {
    let a = harness.fresh();
    let run = parked_at_prd(&a).await;
    rewrite_as_v1(a.store(), run).await;
    a.set_tails(Tails::HandBack);
    let rest = act(&a, run, Act::Approve).await;
    assert_handed_back(&a, run, &rest).await;
    insert_verify_phase(&a, ids::HTUI_FEAT_3).await;

    let b = a.restarted();
    let resumed = adopted(&b, run).await;
    assert!(
        matches!(
            resumed,
            Resume::TopologyChanged {
                rest: Rest {
                    run: RunStatus::AwaitingApproval,
                    ..
                },
                ..
            }
        ),
        "{resumed:?}"
    );
    assert_eq!(run_of(&b, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(state_of(&b, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
    let notes = notes_of(&b, ids::HTUI_FEAT_3).await;
    assert_eq!(
        notes
            .iter()
            .filter(|body| body.starts_with("topology mismatch"))
            .count(),
        1,
        "{notes:?}"
    );
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        b.isolator().reconciles(),
        [(prd.id, Vec::new())],
        "F-24: the frontier merge happened before the topology gate"
    );
}

/// MOD-41 plan D12: a hand-back writes its window and nothing past it. After the approval the
/// rows are the ones before with the answered step moved, no step exists at a later position,
/// and the run is `running` with its lease released.
async fn a_hand_back_writes_no_step_past_the_window<H: CaseHarness>(harness: &H) {
    let a = harness.fresh();
    let run = parked_at_prd(&a).await;
    let before = shape(&steps_of(&a, run).await);
    assert_eq!(
        before,
        [(0, 1, 0, "prd".to_owned(), StepStatus::AwaitingApproval)]
    );

    a.set_tails(Tails::HandBack);
    let rest = act(&a, run, Act::Approve).await;
    assert_handed_back(&a, run, &rest).await;
    assert_eq!(
        shape(&steps_of(&a, run).await),
        [(0, 1, 0, "prd".to_owned(), StepStatus::Done)],
        "the answered step moved, and nothing else was written"
    );
    assert!(
        a.isolator().reconciles().is_empty(),
        "the merge is the adopter's"
    );
}

// -- MOD-26 milestone 1: personas reach the walk (plan D9, D12, D13, OQ-2) -----------------------

/// MOD-26: binds `persona` (by name, read through `personas()`) to `item`'s phase `phase` with
/// `update_phase`, [`repoint`]'s shape; answers the persona row.
///
/// # Panics
/// When the persona or the phase is not there, or the write is refused.
async fn bind_persona<O: Orchestrate>(
    orch: &O,
    item: ItemId,
    phase: &str,
    persona: &str,
) -> Persona {
    let row = orch
        .store()
        .personas()
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|row| row.name == persona)
        .unwrap_or_else(|| panic!("the store holds the `{persona}` persona"));
    let graph = orch
        .store()
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph");
    let target = &graph
        .phases
        .iter()
        .find(|candidate| candidate.phase.name == phase)
        .unwrap_or_else(|| panic!("the graph names a `{phase}` phase"))
        .phase;
    let outcome = orch
        .store()
        .update_phase(
            target.id,
            target.updated_at,
            PhasePatch {
                persona: Some(Some(row.id)),
                ..PhasePatch::default()
            },
        )
        .await
        .expect("the persona names a row");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the phase's version is current: {outcome:?}"
    );
    row
}

/// The frame a persona section opens with (MOD-26 B-20).
fn frame_of(persona: &str) -> String {
    format!("<section name=\"persona\" persona=\"{persona}\">\n")
}

/// The run's stored snapshot, decoded.
///
/// # Panics
/// When the run carries no snapshot, which the walk always writes.
async fn snapshot_of<O: Orchestrate>(orch: &O, run: RunId) -> GraphSnapshot {
    serde_json::from_value(
        run_of(orch, run)
            .await
            .graph_snapshot
            .expect("the walk created the run with a snapshot"),
    )
    .expect("the engine wrote a `GraphSnapshot`")
}

/// MOD-26 D13: `reviewer` bound to `FEAT-3`'s first phase. The step's prompt sections are
/// `template`, then `persona` (`template` keeps `sections[0]`), and its recorded text opens with
/// the persona's frame.
async fn a_persona_bound_phase_prompt_carries_the_persona_section<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let reviewer = bind_persona(&orch, ids::HTUI_FEAT_3, "prd", "reviewer").await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0))
    );
    let prd = at(&steps_of(&orch, run).await, 0, 1).clone();
    let sections = prompt_sections(&orch, prd.id).await;
    assert_eq!(
        sections.get(..2),
        Some(["template".to_owned(), "persona".to_owned()].as_slice()),
        "{sections:?}"
    );
    let text = prompt_text(&orch, prd.id).await;
    assert!(
        text.starts_with(&format!("{}{}", frame_of("reviewer"), reviewer.body)),
        "the prompt opens with the persona frame and its body:\n{text}"
    );
}

/// MOD-26 I-3 (D9): `reviewer` bound to `FEAT-3`'s second phase, edited after `StartRun`. The
/// second step's prompt carries the body frozen at `StartRun` and the snapshot keeps the
/// pre-edit digest.
async fn a_persona_edit_after_start_run_does_not_reach_the_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let reviewer = bind_persona(&orch, ids::HTUI_FEAT_3, "plan", "reviewer").await;
    let frozen = SnapshotPersona::freeze(&reviewer).expect("a seed persona freezes");

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.position, Some(0), "the run parks at `prd`");
    let edited = orch
        .store()
        .update_persona(
            reviewer.id,
            reviewer.updated_at,
            PersonaPatch {
                body: Some("changed\n".to_owned()),
                ..PersonaPatch::default()
            },
        )
        .await
        .expect("the edit is valid");
    assert!(matches!(edited, CasOutcome::Applied(_)), "{edited:?}");

    let (parked, _) = answer(&orch, run, GateAnswer::Approved).await;
    assert_eq!(parked.position, 0);
    let plan = at(&steps_of(&orch, run).await, 1, 1).clone();
    let text = prompt_text(&orch, plan.id).await;
    assert!(
        text.starts_with(&format!("{}{}", frame_of("reviewer"), reviewer.body)),
        "the second step reads the body frozen at `StartRun`:\n{text}"
    );
    assert!(!text.contains("changed\n</section>"), "{text}");
    let snapshot = snapshot_of(&orch, run).await;
    assert_eq!(
        snapshot
            .personas
            .iter()
            .map(|persona| (persona.name.as_str(), persona.digest.as_str()))
            .collect::<Vec<_>>(),
        [("reviewer", frozen.digest.as_str())],
        "the snapshot keeps the pre-edit digest"
    );
}

/// MOD-26 OQ-2: a judged fan-out phase with `reviewer` bound. Every candidate's prompt carries the
/// frame at `sections[1]`; the judge's prompt has no `persona` section, while its task replays a
/// candidate's prompt, frame included.
async fn the_judge_step_runs_without_a_persona<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 1, "the most thorough");
    let reviewer = bind_persona(&orch, ids::HTUI_ANA_2, "research", "reviewer").await;

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );
    let steps = steps_of(&orch, run).await;
    for index in 0..3 {
        let sections = prompt_sections(&orch, candidate(&steps, 0, 1, index).id).await;
        assert_eq!(
            sections.get(1).map(String::as_str),
            Some("persona"),
            "candidate {index}: {sections:?}"
        );
    }
    let judge = judge_of(&steps, 0, 1).expect("the judge is a step");
    let sections = prompt_sections(&orch, judge.id).await;
    assert!(
        !sections.iter().any(|section| section == "persona"),
        "the judge runs without a persona: {sections:?}"
    );
    assert!(
        sections.iter().any(|section| section == "judge_task"),
        "{sections:?}"
    );
    let first_line = reviewer
        .body
        .lines()
        .next()
        .expect("the seed body has a line");
    let text = prompt_text(&orch, judge.id).await;
    assert!(
        text.contains(first_line),
        "the judge's task replays the candidates' frame:\n{text}"
    );
}

/// MOD-26 D13: a persona with `command_run: false` bound to a phase whose `command_queue` is on
/// drops the `command_queue` section; the same phase unbound keeps it.
async fn a_persona_without_command_run_drops_the_command_queue_section<H: CaseHarness>(
    harness: &H,
) {
    let queued = |phase: &mut StepGraphPhase| {
        if phase.name == "prd" {
            phase.command_queue = CommandQueue::Always;
        }
    };

    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, queued).await;
    orch.store()
        .create_persona(NewPersona {
            id: PersonaId::new(),
            name: "no-commands".to_owned(),
            description: "queues no command".to_owned(),
            body: "You queue no command.\n".to_owned(),
            tools: PersonaTools {
                command_run: false,
                ..PersonaTools::default()
            },
            permission: PersonaPermission::default(),
        })
        .await
        .expect("a valid persona");
    bind_persona(&orch, ids::HTUI_FEAT_3, "prd", "no-commands").await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let sections = prompt_sections(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(
        sections.iter().any(|section| section == "persona"),
        "the persona ran: {sections:?}"
    );
    assert!(
        !sections.iter().any(|section| section == "command_queue"),
        "a persona without `command_run` drops the section: {sections:?}"
    );

    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, queued).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let sections = prompt_sections(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(
        sections.iter().any(|section| section == "command_queue"),
        "the unbound phase keeps its section: {sections:?}"
    );
}

/// MOD-26 PRD metric "agent row untouched": a persona narrows the step, never the row. The
/// candidate agent's `updated_at` and `settings` are the same before and after a persona-bound
/// run walked to `done`.
async fn a_persona_run_leaves_the_agent_row_untouched<H: CaseHarness>(harness: &H) {
    async fn claude<O: Orchestrate>(
        orch: &O,
    ) -> (chrono::DateTime<chrono::Utc>, serde_json::Value) {
        let agent = orch
            .store()
            .agents()
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .map(|row| row.agent)
            .find(|agent| agent.id == ids::AGENT_CLAUDE)
            .expect("the fixture seeds `claude`");
        (agent.updated_at, agent.settings)
    }

    let orch = harness.fresh();
    free_feat_3(&orch).await;
    bind_persona(&orch, ids::HTUI_FEAT_3, "prd", "reviewer").await;
    let before = claude(&orch).await;

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let rest = approve(&orch, run, 4).await;
    assert_eq!(rest.run, RunStatus::Done);
    let steps = steps_of(&orch, run).await;
    let prd = at(&steps, 0, 1);
    assert_eq!(
        prd.agent_id,
        Some(ids::AGENT_CLAUDE),
        "the candidate agent drove"
    );
    assert!(
        prompt_sections(&orch, prd.id)
            .await
            .iter()
            .any(|section| section == "persona"),
        "the persona ran"
    );
    assert_eq!(claude(&orch).await, before, "the agent row is untouched");
}

// -- MOD-163 M2: project memory reaches the walk (plan D11, D12, D18) ----------------------------

/// MOD-163: writes one active memory entry of `kind` into `project` through `WriteStore`, as the
/// M3 pane would (a person's, no box).
///
/// # Panics
/// When the text is refused or the store refuses the write.
async fn remember<O: Orchestrate>(
    orch: &O,
    project: ProjectId,
    kind: MemoryKind,
    subject: &str,
    body: &str,
) -> MemoryEntry {
    orch.store()
        .create_memory_entry(project, new_memory(kind, subject, body))
        .await
        .expect("the store takes a plain entry")
}

/// A person's new memory entry with a fresh id, its text checked by an empty scrubber.
///
/// # Panics
/// When the text is refused, which a case's plain text never is.
fn new_memory(kind: MemoryKind, subject: &str, body: &str) -> NewMemoryEntry {
    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    NewMemoryEntry {
        id: MemoryEntryId::new(),
        kind,
        subject: MemorySubject::new(subject, &scrubber).expect("a plain subject"),
        body: MemoryBody::new(body, &scrubber).expect("a plain body"),
        created_by: ids::USER,
        box_id: None,
    }
}

/// The `memory` section of a prompt's text, from its opening tag to its `</section>`, or `None`
/// when the prompt carries none.
fn memory_section(text: &str) -> Option<&str> {
    const OPEN: &str = "<section name=\"memory\">";
    const CLOSE: &str = "</section>";
    let start = text.find(OPEN)?;
    let end = start + text[start..].find(CLOSE)? + CLOSE.len();
    Some(&text[start..end])
}

/// The `memory` section of `step`'s recorded prompt.
///
/// # Panics
/// When the prompt carries none, which is what the caller asserts it does.
async fn memory_of_step<O: Orchestrate>(orch: &O, step: StepId) -> String {
    let text = prompt_text(orch, step).await;
    memory_section(&text)
        .unwrap_or_else(|| panic!("the step's prompt carries a memory section:\n{text}"))
        .to_owned()
}

/// MOD-163 D11, D12, D14: an active entry of `FEAT-3`'s project reaches every phase step's prompt
/// as `sections[1]` (no persona), the same frozen bytes in each; another project's entry reaches
/// none of them.
async fn a_memory_entry_reaches_every_phase_prompt_of_its_project<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let ours = remember(
        &orch,
        ids::PROJECT_HTUI,
        MemoryKind::Gotcha,
        "Build cache",
        "Clear target/ when the sqlx offline data goes stale.",
    )
    .await;
    let theirs = remember(
        &orch,
        ids::PROJECT_AGY,
        MemoryKind::Fact,
        "Elsewhere",
        "Another project's note stays in that project.",
    )
    .await;

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let rest = approve(&orch, run, 4).await;
    assert_eq!(rest.run, RunStatus::Done, "the four phases walked");
    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 4, "one step per phase");
    let first = memory_of_step(&orch, steps[0].id).await;
    for step in &steps {
        let sections = prompt_sections(&orch, step.id).await;
        assert_eq!(
            sections.get(..2),
            Some(["template".to_owned(), "memory".to_owned()].as_slice()),
            "`{}`: {sections:?}",
            step.phase_name
        );
        let text = prompt_text(&orch, step.id).await;
        assert!(text.contains(&ours.body), "`{}`:\n{text}", step.phase_name);
        assert!(
            !text.contains(&theirs.body),
            "`{}`:\n{text}",
            step.phase_name
        );
        assert_eq!(
            memory_of_step(&orch, step.id).await,
            first,
            "`{}` renders the run's one frozen memory",
            step.phase_name
        );
    }
}

/// MOD-163 D11 (blueprint P-8): a retired entry and a superseded one never reach the prompt; the
/// successor does, and the snapshot's `total` counts only it.
async fn closed_memory_entries_never_reach_a_prompt<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let retired = remember(
        &orch,
        ids::PROJECT_HTUI,
        MemoryKind::Fact,
        "Retired fact",
        "This note was retired before the run.",
    )
    .await;
    let retire = orch
        .store()
        .retire_memory_entry(retired.id, retired.version, ids::USER, None)
        .await
        .expect("the entry is live");
    assert!(matches!(retire, MemoryUpdate::Updated(_)), "{retire:?}");
    let old = remember(
        &orch,
        ids::PROJECT_HTUI,
        MemoryKind::Decision,
        "Error crates",
        "The superseded wording of the decision.",
    )
    .await;
    let successor = new_memory(
        MemoryKind::Decision,
        "Error crates",
        "Library crates use thiserror; binaries use anyhow.",
    );
    let replaced = orch
        .store()
        .supersede_memory_entry(old.id, old.version, successor.clone())
        .await
        .expect("the old entry is live");
    assert!(
        matches!(replaced, MemorySupersede::Superseded { .. }),
        "{replaced:?}"
    );

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = at(&steps_of(&orch, run).await, 0, 1).clone();
    let text = prompt_text(&orch, prd.id).await;
    assert!(text.contains(successor.body.as_str()), "{text}");
    assert!(!text.contains(&retired.body), "{text}");
    assert!(!text.contains(&old.body), "{text}");
    let memory = snapshot_of(&orch, run)
        .await
        .memory
        .expect("the successor is active");
    assert_eq!(memory.total, 1, "only the successor is active");
    assert_eq!(
        memory
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [successor.id]
    );
}

/// MOD-163 D11 (`R-PRM-1`), the persona case's twin: an entry amended and then retired after
/// `StartRun` does not reach the run. The next step, a retry and a resume render the run-start
/// bytes, and the snapshot keeps version 1.
async fn a_memory_edit_after_start_run_does_not_reach_the_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let entry = remember(
        &orch,
        ids::PROJECT_HTUI,
        MemoryKind::Lesson,
        "Small commits",
        "Commit at every blueprint boundary.",
    )
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.position, Some(0), "the run parks at `prd`");
    let frozen = memory_of_step(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(frozen.contains(&entry.body), "{frozen}");

    let scrubber = htui_core::scrub::MinimalScrubber::new([]);
    let amended = orch
        .store()
        .amend_memory_entry(
            entry.id,
            entry.version,
            MemoryPatch {
                body: Some(MemoryBody::new("An amended body.", &scrubber).expect("plain")),
                author_id: ids::USER,
                ..MemoryPatch::default()
            },
        )
        .await
        .expect("the entry is live");
    let MemoryUpdate::Updated(amended) = amended else {
        panic!("the token is current: {amended:?}");
    };

    let (parked, _) = answer(&orch, run, GateAnswer::Approved).await;
    assert_eq!(parked.position, 0);
    let plan = at(&steps_of(&orch, run).await, 1, 1).clone();
    assert_eq!(
        memory_of_step(&orch, plan.id).await,
        frozen,
        "the next step reads the memory frozen at `StartRun`"
    );

    let retired = orch
        .store()
        .retire_memory_entry(amended.id, amended.version, ids::USER, None)
        .await
        .expect("the entry is live");
    assert!(matches!(retired, MemoryUpdate::Updated(_)), "{retired:?}");
    let outcome = orch
        .dispatch(Command::RetryStep { run, step: plan.id })
        .await
        .expect("a parked step is retryable within its budget");
    assert!(
        matches!(outcome, CommandOutcome::Retried { .. }),
        "{outcome:?}"
    );
    let retry = at(&steps_of(&orch, run).await, 1, 2).clone();
    assert_eq!(
        memory_of_step(&orch, retry.id).await,
        frozen,
        "a retry reads the memory frozen at `StartRun`"
    );

    // Review R2-2: a resume walks from the stored snapshot, not from the live (now retired)
    // entry, so the step it starts renders the run-start bytes too.
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("memory is outside the topology, so the snapshot still hashes: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(2)),
        "the resume walked to the third phase's gate"
    );
    let third = at(&steps_of(&orch, run).await, 2, 1).clone();
    assert_eq!(
        memory_of_step(&orch, third.id).await,
        frozen,
        "a resumed run reads the memory frozen at `StartRun`"
    );
    let snapshot = snapshot_of(&orch, run).await;
    let memory = snapshot.memory.expect("the run froze the entry");
    assert_eq!(
        memory
            .entries
            .iter()
            .map(|entry| (entry.id, entry.version, entry.body.as_str()))
            .collect::<Vec<_>>(),
        [(entry.id, 1, entry.body.as_str())],
        "the snapshot keeps the run-start version and text"
    );
}

/// MOD-163 D18: the three candidates of a judged fan-out carry byte-identical memory sections;
/// the judge's own sections carry none (its task replays a candidate's prompt).
async fn fan_out_siblings_carry_byte_identical_memory_sections<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 1, "the most thorough");
    let project = item_of(&orch, ids::HTUI_ANA_2).await.project_id;
    remember(
        &orch,
        project,
        MemoryKind::Fact,
        "Research sources",
        "Cite the ANA document a finding comes from.",
    )
    .await;

    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );
    let steps = steps_of(&orch, run).await;
    let first = memory_of_step(&orch, candidate(&steps, 0, 1, 0).id).await;
    for index in 1..3 {
        assert_eq!(
            memory_of_step(&orch, candidate(&steps, 0, 1, index).id).await,
            first,
            "candidate {index}"
        );
    }
    let judge = judge_of(&steps, 0, 1).expect("the judge is a step");
    let sections = prompt_sections(&orch, judge.id).await;
    assert!(
        !sections.iter().any(|section| section == "memory"),
        "the judge carries no memory (D18): {sections:?}"
    );
}

/// MOD-163 D11: a snapshot written before MOD-163 has no `memory` key. Such a run resumes (no
/// `SnapshotTampered`: memory is outside the topology) and its next step renders no memory, even
/// though the project now has an active entry.
async fn a_snapshot_without_memory_resumes_with_no_memory_section<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    remember(
        &orch,
        ids::PROJECT_HTUI,
        MemoryKind::Fact,
        "Present",
        "An active entry the old run never froze.",
    )
    .await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    let mut blob = run_of(&orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    assert!(
        blob.as_object_mut()
            .expect("a snapshot is an object")
            .remove("memory")
            .is_some(),
        "the run froze the entry: {blob}"
    );
    assert!(orch.store().set_run_snapshot(run, blob), "the run exists");

    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("a pre-MOD-163 snapshot still hashes to its topology: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the resume walked to `plan`'s gate"
    );
    let plan = at(&steps_of(&orch, run).await, 1, 1).clone();
    let sections = prompt_sections(&orch, plan.id).await;
    assert!(
        !sections.iter().any(|section| section == "memory"),
        "{sections:?}"
    );
    let text = prompt_text(&orch, plan.id).await;
    assert_eq!(memory_section(&text), None, "{text}");
}

/// MOD-163 D11, D14: a project with no active entry (another project's aside) writes no `memory`
/// row, freezes `None` and stores no `memory` key.
async fn a_project_without_memory_writes_no_memory_row<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    remember(
        &orch,
        ids::PROJECT_AGY,
        MemoryKind::Fact,
        "Elsewhere",
        "Another project's note stays in that project.",
    )
    .await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let prd = at(&steps_of(&orch, run).await, 0, 1).clone();
    let sections = prompt_sections(&orch, prd.id).await;
    assert!(
        !sections.iter().any(|section| section == "memory"),
        "{sections:?}"
    );
    assert_eq!(memory_section(&prompt_text(&orch, prd.id).await), None);
    assert_eq!(snapshot_of(&orch, run).await.memory, None);
    let blob = run_of(&orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    assert!(blob.get("memory").is_none(), "{blob}");
}

// -- MOD-11 T8: the command queue's exposure and the R-MCP-4 denials (plan D16, D17) --------

/// The session key of `phase`'s first attempt, candidate `fanout_index`, first call.
fn session(phase: &str, fanout_index: i32) -> SessionKey<'_> {
    SessionKey {
        phase,
        attempt: 1,
        fanout_index,
        call: 0,
    }
}

/// The spec the session at `key` started with.
///
/// # Panics
/// When that session never started.
fn spec_of<O: Orchestrate>(orch: &O, key: &SessionKey<'_>) -> SessionSpec {
    orch.spec_for(key)
        .unwrap_or_else(|| panic!("the session {key:?} started"))
}

/// MOD-11 D17: the fifteen rules an exposed step's policy opens with, one per OQ-5 prefix, in its
/// order.
fn heavy_denials() -> Vec<PermissionRule> {
    HEAVY_COMMAND_PREFIXES
        .iter()
        .map(|prefix| PermissionRule {
            matcher: PermissionMatch {
                tool_kind: Some("execute".to_owned()),
                command_prefix: Some((*prefix).to_owned()),
                command_word: true,
                ..PermissionMatch::default()
            },
            answer: PermissionOptionKind::RejectOnce,
            reason: format!("run `{prefix}` through htui's `command_run` tool (R-MCP-4)"),
        })
        .collect()
}

/// An agent rule a case gives `claude`, so the denials can be told from the agent's own.
fn agent_rule() -> PermissionRule {
    PermissionRule {
        matcher: PermissionMatch {
            tool_name: Some("Write".to_owned()),
            ..PermissionMatch::default()
        },
        answer: PermissionOptionKind::AllowOnce,
        reason: "the agent's own rule".to_owned(),
    }
}

/// Whether a session's spec carries any of the R-MCP-4 denials.
fn has_denials(spec: &SessionSpec) -> bool {
    let denials = heavy_denials();
    spec.permission
        .rules
        .iter()
        .any(|rule| denials.contains(rule))
}

/// MOD-11 D16, OQ-6: the seeded `prd` phase is `fan_out_only` with one agent and `FEAT-3` carries
/// no `heavy_build`: no `command_queue` section (which the engine rendered for every phase not
/// `off` before MOD-11), no `command_run`, no denial.
async fn a_single_fan_out_fan_out_only_step_has_no_command_queue_section<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let sections = prompt_sections(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(
        !sections.iter().any(|section| section == "command_queue"),
        "fan_out_only at one agent renders no section: {sections:?}"
    );
    let spec = spec_of(&orch, &session("prd", 0));
    assert!(!spec.tools.command_run, "nor exposes `command_run`");
    assert!(!has_denials(&spec), "nor denies a heavy command");
}

/// MOD-11 D16: `research` fanned out to two agents under `fan_out_only` exposes `command_run` to
/// both candidates, with the section and the denials.
async fn a_fan_out_two_step_has_it<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "research" {
            phase.fan_out = 2;
        }
        phase.gate = Gate::Never;
    })
    .await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    for index in 0..2 {
        orch.script_candidate(
            "research",
            1,
            index,
            0,
            ScriptedStep::done_with_output(&format!("research by candidate {index}")),
        );
    }
    judge_both(&orch, "research", 1, 1, "the more thorough");

    let (run, _) = start(&orch, ids::HTUI_ANA_2).await;
    let steps = steps_of(&orch, run).await;
    for index in 0..2 {
        let sections = prompt_sections(&orch, candidate(&steps, 0, 1, index).id).await;
        assert!(
            sections.iter().any(|section| section == "command_queue"),
            "candidate {index} of two renders the section: {sections:?}"
        );
        let spec = spec_of(&orch, &session("research", index));
        assert!(
            spec.tools.command_run,
            "candidate {index} gets `command_run`"
        );
        assert!(has_denials(&spec), "and the denials");
    }
}

/// MOD-11 D16, R-MCP-3: a `heavy_build` item queues its commands under `fan_out_only` without a
/// fan-out.
async fn a_heavy_build_item_has_it<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    declare_tags(&orch, &["heavy_build"]).await;
    require_tags(&orch, ids::HTUI_FEAT_3, &["heavy_build"]).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let sections = prompt_sections(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(
        sections.iter().any(|section| section == "command_queue"),
        "a heavy_build item renders the section at one agent: {sections:?}"
    );
    let spec = spec_of(&orch, &session("prd", 0));
    assert!(spec.tools.command_run, "and exposes `command_run`");
    assert!(has_denials(&spec), "with the denials");
}

/// MOD-11 D17: a persona-less exposed step's rules are the fifteen denials, in OQ-5's order, then
/// the agent's own, untouched.
async fn a_persona_less_exposed_step_gets_the_denials_first<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.command_queue = CommandQueue::Always;
        }
    })
    .await;
    orch.set_policy(
        ids::AGENT_CLAUDE,
        PermissionPolicy {
            rules: vec![agent_rule()],
            ..PermissionPolicy::default()
        },
    );
    start(&orch, ids::HTUI_FEAT_3).await;
    let spec = spec_of(&orch, &session("prd", 0));
    assert!(spec.tools.command_run, "`always` exposes `command_run`");
    let mut expected = heavy_denials();
    expected.push(agent_rule());
    assert_eq!(
        spec.permission.rules, expected,
        "the denials first, then the agent's rule"
    );
}

/// MOD-11 D17: with a persona bound the denials still come first — ahead of the persona's
/// narrowing rules and the agent's own — because they are spliced after `narrow`, not in it.
async fn a_persona_step_gets_the_denials_first_too<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "prd" {
            phase.command_queue = CommandQueue::Always;
        }
    })
    .await;
    orch.set_policy(
        ids::AGENT_CLAUDE,
        PermissionPolicy {
            rules: vec![agent_rule()],
            ..PermissionPolicy::default()
        },
    );
    bind_persona(&orch, ids::HTUI_FEAT_3, "prd", "reviewer").await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let sections = prompt_sections(&orch, at(&steps_of(&orch, run).await, 0, 1).id).await;
    assert!(
        sections.iter().any(|section| section == "persona"),
        "the persona ran: {sections:?}"
    );
    let spec = spec_of(&orch, &session("prd", 0));
    assert!(
        spec.tools.command_run,
        "the seed persona keeps `command_run`"
    );
    let denials = heavy_denials();
    assert_eq!(
        spec.permission.rules.get(..denials.len()),
        Some(denials.as_slice()),
        "the denials open the policy"
    );
    assert_eq!(
        spec.permission.rules.last(),
        Some(&agent_rule()),
        "and the agent's own rule closes it, after the persona's"
    );
}

/// MOD-11 D16: a judged fan-out exposes `command_run` to its candidates, never to the judge,
/// whose phase is `off`: no section, no tool, no denial.
async fn judges_never_get_command_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    fan_research(&orch, Gate::Never, false).await;
    set_judge(&orch, ids::HTUI_ANA_2).await;
    research_candidates(&orch, 1);
    judge_both(&orch, "research", 1, 1, "the most thorough");
    let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
    assert_eq!(rest.run, RunStatus::Done, "the judged run walks to done");
    let steps = steps_of(&orch, run).await;
    assert!(
        spec_of(&orch, &session("research", 0)).tools.command_run,
        "a candidate of three is exposed"
    );
    let judge = judge_of(&steps, 0, 1).expect("the judge is a step");
    let sections = prompt_sections(&orch, judge.id).await;
    assert!(
        !sections.iter().any(|section| section == "command_queue"),
        "the judge renders no section: {sections:?}"
    );
    for call in 0..2 {
        let spec = spec_of(
            &orch,
            &SessionKey {
                phase: "research:judge",
                attempt: 1,
                fanout_index: -1,
                call,
            },
        );
        assert!(
            !spec.tools.command_run,
            "judge call {call}: no `command_run`"
        );
        assert!(!has_denials(&spec), "judge call {call}: no denial");
    }
}

// -- MOD-12 milestone 1: the auto-mode gate downgrade (plan D10, ANA-2 §4.10) ---------------------

/// ANA-2 §12 criterion 23, first half (MOD-12 D10): an auto run of a graph whose every phase gates
/// `always` and none is hard snapshots every phase `never`, so it walks to `done` with no human and
/// every step lands `done` with `gate_outcome = skipped`.
async fn auto_mode_skips_soft_gates<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Always, |phase| phase.gate_hard = false).await;

    let (run, rest) = start_in(&orch, ids::HTUI_FEAT_3, RunMode::Auto).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None),
        "an auto run with no hard gate never stops for a human"
    );
    assert_eq!(run_of(&orch, run).await.mode, RunMode::Auto);

    let snapshot = snapshot_of(&orch, run).await;
    assert_eq!(snapshot.mode, RunMode::Auto);
    assert_eq!(
        snapshot
            .phases
            .iter()
            .map(|phase| (phase.gate, phase.gate_effective))
            .collect::<Vec<_>>(),
        [(Gate::Always, Gate::Never); 4],
        "the snapshot keeps the phase's own gate and downgrades only the effective one"
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.position, step.status, step.gate_outcome))
            .collect::<Vec<_>>(),
        (0..4)
            .map(|position| (position, StepStatus::Done, Some(GateOutcome::Skipped)))
            .collect::<Vec<_>>(),
        "`never` passes each step `running -> done` with `gate_outcome = 'skipped'`"
    );
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::Done);
}

/// ANA-2 §12 criterion 23, second half (MOD-12 D10): an auto run still parks at every hard gate,
/// and only there; its soft steps are skipped and its hard ones approved by the human.
async fn auto_mode_parks_at_a_hard_gate<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Always, |_| {}).await;

    let (run, mut rest) = start_in(&orch, ids::HTUI_FEAT_3, RunMode::Auto).await;
    let snapshot = snapshot_of(&orch, run).await;
    let hard: Vec<i32> = snapshot
        .phases
        .iter()
        .filter(|phase| phase.gate_hard)
        .map(|phase| phase.position)
        .collect();
    assert!(!hard.is_empty(), "the seeded graph keeps a hard phase");
    for phase in &snapshot.phases {
        let expected = if phase.gate_hard {
            Gate::Always
        } else {
            Gate::Never
        };
        assert_eq!(
            phase.gate_effective, expected,
            "`{}` (gate_hard = {}) under auto mode",
            phase.name, phase.gate_hard
        );
    }

    let mut parked = Vec::new();
    while rest.run == RunStatus::AwaitingApproval {
        assert!(
            parked.len() < snapshot.phases.len(),
            "the walk parked more often than it has phases: {parked:?}"
        );
        let (step, next) = answer(&orch, run, GateAnswer::Approved).await;
        parked.push(step.position);
        rest = next;
    }
    assert_eq!(
        parked, hard,
        "the walk parked at the hard gates and nowhere else"
    );
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::Done, None, None)
    );

    let steps = steps_of(&orch, run).await;
    assert_eq!(steps.len(), 4, "one step per position, no retry");
    for step in &steps {
        let outcome = if hard.contains(&step.position) {
            GateOutcome::Approved
        } else {
            GateOutcome::Skipped
        };
        assert_eq!(
            (step.status, step.gate_outcome),
            (StepStatus::Done, Some(outcome)),
            "`{}` at position {}",
            step.phase_name,
            step.position
        );
    }
}

/// ANA-2 §12 criterion 24 (MOD-12 D10): a manual run of the same soft-gated graph keeps every
/// gate — the snapshot says `always`, and the walk parks at position 0 and then at 1.
/// Criterion 24 holds by construction past this case: no `UPDATE` writes `run.mode` (only the
/// run's insert does), and the walk reads only its snapshot's `gate_effective`.
async fn a_manual_snapshot_keeps_its_gates<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Always, |phase| phase.gate_hard = false).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "a manual run parks at its first `always` gate"
    );
    assert_eq!(run_of(&orch, run).await.mode, RunMode::Manual);

    let snapshot = snapshot_of(&orch, run).await;
    assert_eq!(snapshot.mode, RunMode::Manual);
    for phase in &snapshot.phases {
        assert_eq!(
            (phase.gate_hard, phase.gate_effective),
            (false, Gate::Always),
            "`{}`: a manual snapshot never downgrades",
            phase.name
        );
    }

    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the next soft gate parks too: no gate was skipped"
    );
}

// -- MOD-12 milestone 2: the spend guard's walk and session halves (plan D5, D6) -----------------

/// Opens the demo box's queue batch as the runner's resume does (MOD-12 D2).
///
/// # Panics
/// When the demo box refuses a batch, which means the fixture moved.
async fn open_batch<O: Orchestrate>(orch: &O) -> BatchId {
    let user = orch
        .store()
        .this_user()
        .expect("the demo fixture seeds exactly one `app_user`");
    orch.store()
        .open_batch(ids::BOX, user, orch.clock().now())
        .await
        .expect("the demo box opens a batch")
        .id
}

/// `Engine::enqueue_in_batch` then `Engine::claim`, the runner's two calls, unwrapped to the run
/// and its rest.
///
/// # Panics
/// When either is refused, which every caller of this helper expects not to be.
async fn run_in_batch<O: Orchestrate>(orch: &O, item: ItemId, batch: BatchId) -> (RunId, Rest) {
    let run = orch
        .enqueue_in_batch(item, batch, None)
        .await
        .expect("the batch is open and the item startable");
    let outcome = orch.claim(run).await.expect("the box admits the run");
    let CommandOutcome::Started { run: started, rest } = outcome else {
        panic!("`claim` answers `Started`, not {outcome:?}");
    };
    assert_eq!(started, run, "the claim walked the run it was handed");
    (run, rest)
}

/// Merges `key = micros` into the settings of `item`'s project, keeping every other key:
/// `set_project_settings` replaces the whole blob (blueprint H-9).
async fn plant_setting<O: Orchestrate>(orch: &O, item: ItemId, key: &str, micros: i64) {
    let project = item_of(orch, item).await.project_id;
    let mut settings = orch
        .store()
        .project_settings(project)
        .await
        .expect("MemStore never fails a read")
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    settings[key] = serde_json::json!(micros);
    orch.store().set_project_settings(project, settings);
}

/// A fresh `open` ANA item on the HTUI project (`research`, then the hard `verdict`), declaring
/// nothing: the demo project has no repo, so no two of its runs overlap.
///
/// # Panics
/// When the mint is refused, which means the fixture moved.
async fn mint_ana<O: Orchestrate>(orch: &O, title: &str) -> ItemId {
    let user = orch
        .store()
        .this_user()
        .expect("the demo fixture seeds exactly one `app_user`");
    orch.store()
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_ANA,
            title: title.to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            created_by: user,
            box_id: Some(ids::BOX),
        })
        .await
        .expect("the ANA kind mints on the HTUI project")
        .id
}

/// `batch`'s spend as the store computes it (plan D1).
async fn spend_of<O: Orchestrate>(orch: &O, batch: BatchId) -> Option<i64> {
    orch.store()
        .batch_spend(batch)
        .await
        .expect("MemStore never fails a read")
}

/// `ANA-2` admitted under a fresh batch, its `research` costing 600: the auto run skips research's
/// soft gate and parks at `verdict`'s hard one, leaving the batch at 600.
async fn a_batch_that_spent_600<O: Orchestrate>(orch: &O) -> (BatchId, RunId) {
    let batch = open_batch(orch).await;
    orch.script(
        "research",
        1,
        ScriptedStep::done_costing("the research", 600),
    );
    let (run, rest) = run_in_batch(orch, ids::HTUI_ANA_2, batch).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(1), None),
        "auto skips `research`'s soft gate and parks at `verdict`'s hard one"
    );
    assert_eq!(spend_of(orch, batch).await, Some(600));
    (batch, run)
}

/// MOD-12 M2 D5: a batch at its project's `per_token_cap_batch` starts no new attempt. The walk
/// refuses the second run's first phase with rule 6, so it takes `refuse_no_candidate`'s path:
/// the run fails `no_candidate_agent`, no step row is written, and the item is `blocked` with a
/// note naming the batch, an escalation for M3 with no new state.
async fn a_batch_at_its_cap_starts_no_attempt<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let (batch, first) = a_batch_that_spent_600(&orch).await;

    plant_setting(&orch, ids::HTUI_ANA_2, "per_token_cap_batch", 500).await;
    let second = mint_ana(&orch, "a second question").await;
    let (run, rest) = run_in_batch(&orch, second, batch).await;
    let cause = "batch cap reached (600 of 500 micros)";
    assert_eq!((rest.run, rest.position), (RunStatus::Failed, Some(0)));
    let Some(RunFailure::NoCandidateAgent { phase, detail }) = &rest.failure else {
        panic!("a `no_candidate_agent` refusal, not {:?}", rest.failure);
    };
    assert_eq!(phase, "research");
    assert!(detail.contains(cause), "the walk names the batch: {detail}");
    assert!(
        steps_of(&orch, run).await.is_empty(),
        "stage 1 refuses before a step row exists"
    );
    assert_eq!(item_of(&orch, second).await.status, Status::Blocked);
    let notes = notes_of(&orch, second).await;
    assert!(
        notes.iter().any(|note| note.contains(cause)),
        "a refusal a human can read: {notes:?}"
    );
    assert_eq!(
        run_of(&orch, first).await.status,
        RunStatus::AwaitingApproval,
        "the run that spent the batch is left parked"
    );
    assert_eq!(
        spend_of(&orch, batch).await,
        Some(600),
        "the refusal spent nothing"
    );
}

/// MOD-12 M2 D6, D7 (PRD metric "batch spend overshoot", Mem half): a batch run's session is
/// allowed what the batch had left when it started. `verdict` (ungated here, so the breach fails
/// the step instead of parking it) is handed the 400 `research` left of the 1 000 cap, spends 600,
/// and is cut with a failure naming the batch; the overshoot is that one attempt, and the retry
/// meets the walk's rule 6.
async fn a_batch_run_session_is_capped_at_the_batch_remainder<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    repoint(&orch, ids::HTUI_ANA_2, |phase| {
        if phase.name == "verdict" {
            phase.gate = Gate::Never;
        }
    })
    .await;
    // Planted before the run is enqueued: the snapshot freezes it (plan D2).
    plant_setting(&orch, ids::HTUI_ANA_2, "per_token_cap_batch", 1_000).await;
    let batch = open_batch(&orch).await;
    orch.script(
        "research",
        1,
        ScriptedStep::done_costing("the research", 600),
    );
    orch.script("verdict", 1, ScriptedStep::done_costing("the verdict", 600));

    let (run, rest) = run_in_batch(&orch, ids::HTUI_ANA_2, batch).await;
    assert_eq!(
        spec_of(&orch, &session("research", 0)).budget_micros,
        Some(1_000),
        "nothing spent yet: the whole batch cap"
    );
    assert_eq!(
        spec_of(&orch, &session("verdict", 0)).budget_micros,
        Some(400),
        "what `research` left of the batch"
    );
    let verdict = step_at(&orch, run, 1, 1).await;
    assert_eq!(verdict.status, StepStatus::Failed, "the recorder cut it");
    let breached = format!("cap breached (batch {batch})");
    let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
    assert!(
        notes.contains(&format!("step `verdict` attempt 1: {breached}")),
        "the failure names the batch: {notes:?}"
    );
    assert_eq!(
        spend_of(&orch, batch).await,
        Some(1_200),
        "the overshoot is the one attempt in flight"
    );

    // The retry's stage 1 sees the batch over its cap and refuses (rule 6).
    assert_eq!((rest.run, rest.position), (RunStatus::Failed, Some(1)));
    let Some(RunFailure::NoCandidateAgent { phase, detail }) = &rest.failure else {
        panic!("a `no_candidate_agent` refusal, not {:?}", rest.failure);
    };
    assert_eq!(phase, "verdict");
    assert!(
        detail.contains("batch cap reached (1200 of 1000 micros)"),
        "{detail}"
    );
    assert_eq!(
        steps_of(&orch, run).await.len(),
        2,
        "no second `verdict` attempt was written"
    );
}

/// MOD-12 M2 D5, D6: a manual run is in no batch, so it reads no batch figure. Its project's
/// `per_token_cap_batch` is 0 and the box's batch has spent 600, and still the manual run walks to
/// its first gate, its session gets no cap at all, and its own 600 is not the batch's.
async fn a_manual_run_ignores_batch_figures<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let (batch, _) = a_batch_that_spent_600(&orch).await;
    plant_setting(&orch, ids::HTUI_ANA_2, "per_token_cap_batch", 0).await;

    let manual = mint_ana(&orch, "a manual question").await;
    let (run, rest) = start(&orch, manual).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "a manual run walks to its first gate: no batch rule refused it"
    );
    assert_eq!(
        orch.store()
            .run_batch_spend(run)
            .await
            .expect("MemStore never fails a read"),
        None,
        "a manual run is in no batch"
    );
    assert_eq!(
        spec_of(&orch, &session("research", 0)).budget_micros,
        None,
        "no run cap and no batch: no cap at all"
    );
    let research = step_at(&orch, run, 0, 1).await;
    assert_eq!(research.status, StepStatus::AwaitingApproval);
    assert!(
        !notes_of(&orch, manual)
            .await
            .iter()
            .any(|note| note.contains("cap breached")),
        "nothing was cut"
    );
    assert_eq!(
        spend_of(&orch, batch).await,
        Some(600),
        "the manual run's spend is not the batch's"
    );
}

/// MOD-12 M2 D6 (the per-step reading of `per_token_cap_run`, fixed): `plan`'s allowance is the
/// 1 000 cap less the 600 `prd` spent, so its 500 breaches it. Before D6 each session had the
/// whole 1 000 and `plan` passed.
async fn the_run_cap_spans_steps<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    plant_setting(&orch, ids::HTUI_FEAT_3, "per_token_cap_run", 1_000).await;
    orch.script("prd", 1, ScriptedStep::done_costing("the prd", 600));
    orch.script("plan", 1, ScriptedStep::done_costing("the plan", 500));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        spec_of(&orch, &session("prd", 0)).budget_micros,
        Some(1_000)
    );
    assert_eq!(
        spec_of(&orch, &session("plan", 0)).budget_micros,
        Some(400),
        "the cap less what `prd` spent"
    );
    assert_eq!(step_at(&orch, run, 0, 1).await.status, StepStatus::Done);
    assert_eq!(step_at(&orch, run, 1, 1).await.status, StepStatus::Failed);
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.contains(&"step `plan` attempt 1: cap breached".to_owned()),
        "the run's own cap: no batch named: {notes:?}"
    );
    assert_eq!((rest.run, rest.position), (RunStatus::Failed, Some(1)));
    let Some(RunFailure::NoCandidateAgent { phase, detail }) = &rest.failure else {
        panic!("a `no_candidate_agent` refusal, not {:?}", rest.failure);
    };
    assert_eq!(phase, "plan");
    assert!(
        detail.contains("quota: cap reached (1100 of 1000 micros)"),
        "the retry's stage 1 sees the run over its cap: {detail}"
    );
}

/// MOD-12 M2 D6 (review R1, M1): a session's run term is the run's snapshotted
/// `per_token_cap_run`, the figure the walk's rule 2 reads. The run starts under a 10 000 cap,
/// `prd` spends 2 000 and parks at its gate, and the live cap is then lowered to 1 000 (as
/// Settings > Queue would). The walk admits `plan` (2 000 of 10 000), so its session is handed
/// the snapshot's remainder, 8 000, and is not cut: a live term would have handed it 0 and cut it
/// at its first costed row, on every retry.
async fn a_lowered_live_run_cap_does_not_cut_a_snapshotted_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    // Planted before the run starts: the snapshot freezes it (plan D2).
    plant_setting(&orch, ids::HTUI_FEAT_3, "per_token_cap_run", 10_000).await;
    orch.script("prd", 1, ScriptedStep::done_costing("the prd", 2_000));
    orch.script("plan", 1, ScriptedStep::done_costing("the plan", 500));

    let (run, parked) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (parked.run, parked.position, parked.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "`prd` parks at its gate"
    );
    assert_eq!(
        spec_of(&orch, &session("prd", 0)).budget_micros,
        Some(10_000)
    );

    // The live cap is lowered below what the run has already spent; the snapshot keeps 10 000.
    plant_setting(&orch, ids::HTUI_FEAT_3, "per_token_cap_run", 1_000).await;
    let rest = approve(&orch, run, 1).await;

    assert_eq!(
        spec_of(&orch, &session("plan", 0)).budget_micros,
        Some(8_000),
        "the snapshot's cap less what `prd` spent, not the live cap's floor of 0"
    );
    assert_eq!(
        step_at(&orch, run, 1, 1).await.status,
        StepStatus::Done,
        "`plan` spent 500 of its 8 000 and was not cut"
    );
    assert!(
        !notes_of(&orch, ids::HTUI_FEAT_3)
            .await
            .iter()
            .any(|note| note.contains("cap breached")),
        "nothing was cut"
    );
    assert_eq!(rest.failure, None, "the run walked on: {rest:?}");
}

// -- MOD-138 M1 T5: pinned resume (plan D5) ---------------------------------------------------------

/// The `v` the run's stored snapshot carries, as written (never lifted).
///
/// # Panics
/// When the run carries no snapshot, or one that does not decode.
async fn stored_v<O: Orchestrate>(orch: &O, run: RunId) -> u32 {
    let blob = run_of(orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    serde_json::from_value::<GraphSnapshot>(blob)
        .expect("the engine wrote a `GraphSnapshot`")
        .v
}

/// Adds an `asks_you` edge `from → to` to `item`'s live graph through `replace_graph`: a back
/// edge the validator accepts (MOD-138 D9). M1's resolve refused it (D6(c)); M2's resolves it (D10).
///
/// # Panics
/// When the item resolves to no graph, a phase is missing, or `replace_graph` refuses the body.
async fn add_asks_you<O: Orchestrate>(orch: &O, item: ItemId, from: &str, to: &str) {
    let graph = orch
        .store()
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph")
        .graph
        .id;
    let mut body = orch
        .store()
        .graph_body(graph)
        .await
        .expect("MemStore never fails a read")
        .expect("the item's graph is there");
    let id = |name: &str| {
        body.nodes
            .iter()
            .find(|node| node.phase.name == name)
            .map(|node| node.phase.id)
            .unwrap_or_else(|| panic!("the graph has a `{name}` phase"))
    };
    let (from, to) = (id(from), id(to));
    body.edges.push(StepGraphEdge {
        id: EdgeId::new(),
        graph_id: graph,
        from,
        to,
        kind: EdgeKind::AsksYou,
        predicate: None,
        loop_limit: None,
    });
    let outcome = orch
        .store()
        .replace_graph(graph, body.graph.revision, body)
        .await
        .expect("the store takes a back `asks_you` edge");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
}

/// `(position, attempt, status)` of every step of the run.
async fn step_shape<O: Orchestrate>(orch: &O, run: RunId) -> Vec<(i32, i32, StepStatus)> {
    steps_of(orch, run)
        .await
        .iter()
        .map(|step| (step.position, step.attempt, step.status))
        .collect()
}

/// MOD-138 D5, the v2 twin of `topology_mismatch_parks_on_resume`: a v2 run is pinned, so the
/// same live edit under the same mid-walk run is not read at all. `resume` walks on and creates
/// position 1, and no `topology mismatch` note is written.
async fn a_v2_run_walks_on_after_a_live_edit<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        stored_v(&orch, run).await,
        GraphSnapshot::V,
        "a new run is v2"
    );
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;

    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("a v2 run never compares against the live graph: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the walk ran position 1 and parked at its gate"
    );
    assert_eq!(
        step_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
        ]
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        !notes.iter().any(|body| body.contains("topology mismatch")),
        "{notes:?}"
    );
}

/// MOD-138 D5, the v2 twin of `a_topology_mismatch_parks_a_running_run`: the adopter's resume of
/// a crashed v2 run whose live graph moved walks its snapshot on, and the run is not parked.
async fn a_running_v2_run_walks_on_after_a_live_edit<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    let stalled = orch.stall_after_done("prd", 1, None, false);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    assert_eq!(stored_v(&orch, run).await, GraphSnapshot::V);

    let other = orch.restarted();
    set_input_kinds(&other, ids::HTUI_FEAT_3, 0, &["spec"]).await;
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(
        adopted,
        [Adopted {
            run,
            next: Next::Walk
        }]
    );

    let resumed = other.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("a v2 run walks its snapshot: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "every phase is `never`, so the snapshot walks to the end"
    );
    assert_eq!(run_of(&other, run).await.status, RunStatus::Done);
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    assert!(
        !notes.iter().any(|body| body.contains("topology mismatch")),
        "{notes:?}"
    );
}

/// MOD-138 D5, the v2 twin of `a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note`:
/// the frontier merge still happens first, and then the v2 run walks on to `plan`'s gate with no
/// topology note.
async fn a_handed_back_v2_run_on_a_changed_graph_walks_on<H: CaseHarness>(harness: &H) {
    let a = harness.fresh();
    let run = parked_at_prd(&a).await;
    assert_eq!(stored_v(&a, run).await, GraphSnapshot::V);
    a.set_tails(Tails::HandBack);
    let rest = act(&a, run, Act::Approve).await;
    assert_handed_back(&a, run, &rest).await;
    insert_verify_phase(&a, ids::HTUI_FEAT_3).await;

    let b = a.restarted();
    let resumed = adopted(&b, run).await;
    let Resume::Walked(rest) = resumed else {
        panic!("a v2 run never compares against the live graph: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the adopter walked to `plan`'s gate"
    );
    let notes = notes_of(&b, ids::HTUI_FEAT_3).await;
    assert!(
        !notes
            .iter()
            .any(|body| body.starts_with("topology mismatch")),
        "{notes:?}"
    );
    let prd = step_at(&b, run, 0, 1).await;
    assert_eq!(
        b.isolator().reconciles(),
        [(prd.id, Vec::new())],
        "F-24: the frontier merge happened"
    );
}

/// MOD-138 D5: a v2 snapshot edited after it was frozen no longer hashes to its own `topology`.
/// `resume` parks the run (run and item `awaiting_approval`), gives the lease back, writes one
/// `snapshot tampered` note naming both digests and walks nothing; no sweep adopts the run again.
async fn a_tampered_v2_snapshot_parks_with_both_digests<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    let mut blob = run_of(&orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    let stored_topology = blob["topology"]
        .as_str()
        .expect("a snapshot carries its topology")
        .to_owned();
    blob["phases"][0]["input_kinds"] = serde_json::json!(["spec"]);
    assert!(orch.store().set_run_snapshot(run, blob), "the run exists");
    let before = step_shape(&orch, run).await;

    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::SnapshotTampered {
        stored,
        recomputed,
        rest,
    } = resumed
    else {
        panic!("the snapshot no longer hashes to its own topology: {resumed:?}");
    };
    assert_eq!(stored, stored_topology);
    assert_ne!(
        stored, recomputed,
        "two digests, and they are not the same one"
    );
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "parked where it was"
    );
    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::AwaitingApproval);
    assert_eq!(
        row.lease_expires_at,
        Some(orch.clock().now()),
        "the lease `resume` took is given back"
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "the item reads its parked run (MOD-144)"
    );
    assert_eq!(
        step_shape(&orch, run).await,
        before,
        "the walk is not advanced"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    let tampered: Vec<_> = notes
        .iter()
        .filter(|body| body.starts_with("snapshot tampered"))
        .collect();
    assert_eq!(tampered.len(), 1, "{notes:?}");
    assert!(
        tampered[0].contains(&stored) && tampered[0].contains(&recomputed),
        "the note names both digests: {}",
        tampered[0]
    );
    assert!(
        !notes.iter().any(|body| body.contains("topology mismatch")),
        "the live graph was never compared: {notes:?}"
    );
    assert_eq!(
        orch.restarted().sweep().await.expect("the sweep runs"),
        [],
        "no sweep adopts the parked run"
    );
}

/// MOD-138 D5 as amended by blueprint A6 and M2 D11: a v1 run whose live graph gained a
/// non-default edge. M1 refused that live graph (D6(c)); M2 resolves it (D10), and a v1 run walks
/// the lift of its phases either way, so it is compared on the v1 phase-only digest; its phases
/// did not change, so `resume` walks on to `plan`'s gate and writes no topology note.
async fn a_v1_run_whose_live_graph_gained_a_non_default_edge_walks_on<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    rewrite_as_v1(orch.store(), run).await;
    assert_eq!(stored_v(&orch, run).await, GraphSnapshot::V1);
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    add_asks_you(&orch, ids::HTUI_FEAT_3, "review", "plan").await;

    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("a refused live graph whose phases did not move is walked: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the walk ran position 1 and parked at its gate"
    );
    assert_eq!(
        step_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
        ],
        "the walk advanced"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        !notes
            .iter()
            .any(|body| body.starts_with("topology mismatch")
                || body.starts_with("live graph not comparable")),
        "{notes:?}"
    );
}

/// MOD-138 D5 as amended by blueprint A6 and M2 D11: a v1 run on a live graph with an added
/// `asks_you` edge (refused by M1, resolved since M2) whose phases also moved (`prd`'s
/// `input_kinds`) parks with the topology note. Both digests are v1 phase-only ones: `snapshot`
/// is the run's stored v1 `topology`, and `live` the live phases' digest, never a `not runnable`
/// sentence.
async fn a_v1_run_on_a_refused_graph_whose_phases_changed_parks<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    rewrite_as_v1(orch.store(), run).await;
    let stored_topology = run_of(&orch, run).await.graph_snapshot.expect("a snapshot")["topology"]
        .as_str()
        .expect("a snapshot carries its topology")
        .to_owned();
    approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
    add_asks_you(&orch, ids::HTUI_FEAT_3, "review", "plan").await;
    set_input_kinds(&orch, ids::HTUI_FEAT_3, 0, &["spec"]).await;
    let before = step_shape(&orch, run).await;

    let resumed = orch.resume(run).await.expect("the run is readable");
    let Resume::TopologyChanged {
        snapshot,
        live,
        rest,
    } = resumed
    else {
        panic!("the live phases moved, so the v1 digests differ: {resumed:?}");
    };
    assert_eq!(
        snapshot, stored_topology,
        "the v1 digest of the run's own phases"
    );
    assert!(
        live.starts_with("sha256:"),
        "a digest, not a refusal: {live}"
    );
    assert_ne!(snapshot, live, "two digests, and they are not the same one");
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1))
    );
    assert_eq!(
        state_of(&orch, ids::HTUI_FEAT_3).await,
        ItemState::NeedsYou,
        "the item reads its parked run (MOD-144)"
    );
    assert_eq!(
        step_shape(&orch, run).await,
        before,
        "the walk is not advanced"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        notes
            .iter()
            .filter(|body| body.starts_with("topology mismatch")
                && body.contains(&snapshot)
                && body.contains(&live))
            .count(),
        1,
        "{notes:?}"
    );
}

// -- MOD-138 M2 T4: loops over edges (plan D7, D10; blueprint A3, A4, A6, A8) ------------------

/// Rewrites `item`'s live graph through `replace_graph` after `edit`: the body the store holds,
/// edited, and checked by the shared validator as any save is (MOD-138 D9).
///
/// # Panics
/// When the item resolves to no graph, or `replace_graph` refuses the body.
async fn edit_graph<O: Orchestrate>(
    orch: &O,
    item: ItemId,
    edit: impl FnOnce(&mut htui_core::model::GraphBody),
) {
    let graph = orch
        .store()
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph")
        .graph
        .id;
    let mut body = orch
        .store()
        .graph_body(graph)
        .await
        .expect("MemStore never fails a read")
        .expect("the item's graph is there");
    edit(&mut body);
    let outcome = orch
        .store()
        .replace_graph(graph, body.graph.revision, body)
        .await
        .expect("the store takes the edited graph");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
}

/// The phase id of `name` in `body`.
///
/// # Panics
/// When the graph has no such phase.
fn node_id(body: &htui_core::model::GraphBody, name: &str) -> PhaseId {
    body.nodes
        .iter()
        .find(|node| node.phase.name == name)
        .map(|node| node.phase.id)
        .unwrap_or_else(|| panic!("the graph has a `{name}` phase"))
}

/// One authored edge `from → to` of `kind` on `item`'s live graph (MOD-138 M2), beside the ones
/// it has. `predicate` is `Some` iff `kind` is `if` (the validator refuses anything else).
///
/// # Panics
/// As [`edit_graph`].
async fn add_edge<O: Orchestrate>(
    orch: &O,
    item: ItemId,
    from: &str,
    to: &str,
    kind: EdgeKind,
    predicate: Option<htui_core::model::EdgePredicate>,
    loop_limit: Option<i32>,
) {
    edit_graph(orch, item, |body| {
        let (from, to) = (node_id(body, from), node_id(body, to));
        body.edges.push(StepGraphEdge {
            id: EdgeId::new(),
            graph_id: body.graph.id,
            from,
            to,
            kind,
            predicate,
            loop_limit,
        });
    })
    .await;
}

/// One edge as [`author_edges`] takes it: `(from, to, kind, predicate, loop_limit)`, the
/// arguments of [`add_edge`] after the item.
type AuthoredEdge<'a> = (
    &'a str,
    &'a str,
    EdgeKind,
    Option<htui_core::model::EdgePredicate>,
    Option<i32>,
);

/// `item`'s live graph with **every** edge replaced by `edges`, the way an author draws a graph
/// that is not a chain. The nodes stay as they are: no T4 case authors one.
///
/// # Panics
/// As [`edit_graph`].
async fn author_edges<O: Orchestrate>(orch: &O, item: ItemId, edges: &[AuthoredEdge<'_>]) {
    edit_graph(orch, item, |body| {
        body.edges = edges
            .iter()
            .map(|(from, to, kind, predicate, loop_limit)| StepGraphEdge {
                id: EdgeId::new(),
                graph_id: body.graph.id,
                from: node_id(body, from),
                to: node_id(body, to),
                kind: *kind,
                predicate: predicate.clone(),
                loop_limit: *loop_limit,
            })
            .collect();
    })
    .await;
}

/// `(position, attempt, status)` of every step of the run, sorted.
async fn sorted_shape<O: Orchestrate>(orch: &O, run: RunId) -> Vec<(i32, i32, StepStatus)> {
    let mut shape = step_shape(orch, run).await;
    shape.sort_by_key(|&(position, attempt, _)| (position, attempt));
    shape
}

/// MOD-138 M2 D7 (plan Q-D): a node reads its verdict because it has a `changes_requested` edge,
/// not because it is called `review`. FEAT-3's `review` renamed `audit` gets no loop from the
/// default rule; an authored `audit → implement` edge loops it like the review loop: the
/// request-changes verdict retires `implement` and the rejecting `audit`, and the second audit's
/// approval walks the run to `done`.
async fn a_changes_requested_edge_loops_a_node_not_named_review<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "review" {
            phase.name = "audit".to_owned();
        }
    })
    .await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "audit",
        "implement",
        EdgeKind::ChangesRequested,
        None,
        Some(1),
    )
    .await;
    orch.script("audit", 1, ScriptedStep::review("request-changes", "first"));
    orch.script("audit", 2, ScriptedStep::review("approve", "second"));

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "the loop ran once and the second audit approved: {rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::Done),
            (3, 1, StepStatus::Cancelled),
            (3, 2, StepStatus::Done),
        ]
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(at(&steps, 3, 1).gate_outcome, Some(GateOutcome::Rejected));
    assert_eq!(
        at(&steps, 3, 1).gate_note.as_deref(),
        Some("verdict: request-changes")
    );
}

/// MOD-138 M2 Q-D, the other half: a node called `review` with no `changes_requested` edge does
/// not read its verdict. Its `request-changes` passes unread, as any `ok` settle does, and the
/// run finishes with one review.
async fn a_review_with_no_changes_requested_edge_passes_unread<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    author_edges(
        &orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
        ],
    )
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "unread"),
    );

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    let review = at(&steps_of(&orch, run).await, 3, 1).clone();
    assert_eq!(review.gate_outcome, Some(GateOutcome::Skipped));
    assert_eq!(
        review.gate_note, None,
        "no verdict was read, so none was noted"
    );
}

/// MOD-138 M2 D7: a `never` node with a `fails` edge loops over it on a failure, in place of its
/// own `retry_limit`. `review → plan` `fails`: the review's missing output retires the region
/// `plan, implement, review` (target first), `prd` keeps its row, and the second pass finishes.
async fn a_fails_edge_loops_and_retires_its_region<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "review",
        "plan",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;
    orch.script("review", 1, ScriptedStep::done_without_output());

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Superseded),
            (1, 2, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::Done),
            (3, 1, StepStatus::Cancelled),
            (3, 2, StepStatus::Done),
        ],
        "the failed review is cancelled (`failed -> superseded` is illegal), and its own retry \
         would have left `plan` and `implement` alone"
    );
}

/// MOD-138 M2 D7, blueprint A3: a human's reject of a node with no `changes_requested` edge
/// loops over its `fails` edge, where it used to end the run `rejected: plan`. `plan → prd`
/// `fails`: `prd` and the rejected `plan` are retired and `prd` parks again at attempt 2.
async fn a_human_reject_of_a_fails_node_loops<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "plan",
        "prd",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    approve(&orch, run, 1).await; // prd -> plan parks
    let rest = act(&orch, run, Act::Reject).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "the loop resumed at `prd`, whose `always` gate parked it"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Superseded),
            (0, 2, StepStatus::AwaitingApproval),
            (1, 1, StepStatus::Cancelled),
        ]
    );
    assert_eq!(
        at(&steps_of(&orch, run).await, 1, 1).gate_outcome,
        Some(GateOutcome::Rejected),
        "the human's answer stays on the retired row"
    );
}

/// MOD-138 M2 D7, blueprint F8: a crash between `gate::apply`'s `running -> failed` and its
/// `fails` loop leaves a `failed` row with no gate note under a `running` run. The adopter's
/// sweep reaches `settle_failed`, which runs the loop the crash cut short rather than retrying
/// the step on its own budget.
///
/// `apply` writes that `running -> failed` only for a `never` node (an `always` one parks on every
/// outcome), so `plan` is `never` here, as the crash needs it. `prd` stays `always` only so the
/// run rests where the hand-built rows begin.
async fn a_fails_loop_cut_short_is_replayed_by_settle_failed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "plan",
        "prd",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let blob = run_of(&orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    let snapshot: GraphSnapshot =
        serde_json::from_value(blob).expect("the snapshot decodes as a `GraphSnapshot`");
    let gate_of = |name: &str| {
        snapshot
            .phases
            .iter()
            .find(|phase| phase.name == name)
            .map(|phase| phase.gate_effective)
    };
    assert_eq!(
        (gate_of("prd"), gate_of("plan")),
        (Some(Gate::Always), Some(Gate::Never)),
        "only a `never` node reaches `apply`'s `fails` loop"
    );

    // The rows the crash left: `prd` approved, `plan` failed by the settle, the run `running`.
    let store = orch.store();
    let now = orch.clock().now();
    let prd = step_at(&orch, run, 0, 1).await;
    assert!(
        store
            .answer_gate(prd.id, GateOutcome::Approved, None, now)
            .await
            .expect("MemStore takes the answer")
    );
    let plan = store
        .create_step(htui_core::model::NewRunStep {
            id: StepId::new(),
            run_id: run,
            position: 1,
            attempt: 1,
            fanout_index: 0,
            phase_name: "plan".to_owned(),
            agent_id: None,
            model: None,
            role: StepRole::Step,
            parent_step_id: None,
            node_key: None,
        })
        .await
        .expect("`(1, 1, 0)` is free");
    for (from, to) in [
        (StepStatus::Pending, StepStatus::Running),
        (StepStatus::Running, StepStatus::Failed),
    ] {
        assert!(
            store
                .transition_step(plan.id, from, to, now)
                .await
                .expect("MemStore takes the move")
        );
    }
    assert!(
        store
            .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
            .await
            .expect("MemStore takes the move")
    );

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(adopted.len(), 1, "{adopted:?}");
    assert_eq!(adopted[0].run, run);
    let resumed = other.resume(run).await.expect("the run is readable");
    let Resume::Walked(rest) = resumed else {
        panic!("a v2 run walks its snapshot: {resumed:?}");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(0), None),
        "the replayed loop resumed at `prd`"
    );
    assert_eq!(
        sorted_shape(&other, run).await,
        [
            (0, 1, StepStatus::Superseded),
            (0, 2, StepStatus::AwaitingApproval),
            (1, 1, StepStatus::Cancelled),
        ],
        "a retry of `plan` would have left `prd` done and created `(1, 2)`"
    );
}

/// MOD-138 M2 D7: a `fails` loop out of budget escalates as the review loop does: the run parks
/// at the failing node, the item is blocked and the note names the loop, with
/// `RunFailure::LoopExhausted` on the rest.
async fn a_spent_fails_loop_escalates<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "review",
        "implement",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;
    for attempt in [1, 2] {
        orch.script("review", attempt, ScriptedStep::done_without_output());
    }

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    let failure = RunFailure::LoopExhausted {
        kind: EdgeKind::Fails,
        from: "review".to_owned(),
        to: "implement".to_owned(),
        attempts: 2,
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(3), Some(failure.clone())),
        "`loop_limit = 1` permits two attempts of `implement`"
    );
    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::AwaitingApproval);
    assert!(
        row.failure.is_none(),
        "`run.failure` stays NULL on a parked run"
    );
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    let expected = format!("{failure}: phase `implement`, attempt 2, stop reason `exhausted`");
    assert!(notes.contains(&expected), "{notes:?}");
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::Done),
            (3, 1, StepStatus::Cancelled),
            (3, 2, StepStatus::Failed),
        ],
        "the loop stopped before retiring the second failure"
    );
}

/// MOD-138 M2 D7, ANA-33 §7.3 (blueprint §2.6): on the diamond `prd → {plan, implement} →
/// review` with `review → plan` `changes_requested`, the loop retires only the region on a path
/// from `plan` to `review`. `implement`, a side branch, keeps its `done` row and is not re-run;
/// the join waits for `plan`'s second attempt and the second review finishes the run.
async fn a_loop_on_a_diamond_retires_only_its_region<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    author_edges(
        &orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("prd", "implement", EdgeKind::Next, None, None),
            ("plan", "review", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            ("review", "plan", EdgeKind::ChangesRequested, None, Some(1)),
        ],
    )
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Superseded),
            (1, 2, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Cancelled),
            (3, 2, StepStatus::Done),
        ],
        "`implement` is on no path from `plan` to `review`: rows win"
    );
}

/// MOD-138 M2 D7: no progress stops an edge loop before its budget does. `audit → implement`
/// `changes_requested` with `loop_limit = 5`, no repo in scope, and two identical audits: the
/// second turn escalates on `no_progress_review` with the review loop's bytes.
async fn an_edge_loop_without_progress_escalates<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "review" {
            phase.name = "audit".to_owned();
        }
    })
    .await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "audit",
        "implement",
        EdgeKind::ChangesRequested,
        None,
        Some(5),
    )
    .await;
    for attempt in [1, 2] {
        orch.script(
            "audit",
            attempt,
            ScriptedStep::review("request-changes", "same words"),
        );
    }

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (
            RunStatus::AwaitingApproval,
            Some(3),
            Some(RunFailure::ReviewLoopExhausted(2))
        ),
        "the budget allows six attempts; no progress stopped the second"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body
            == "review loop exhausted after 2 attempts: phase `implement`, attempt 2, stop \
                reason `no_progress_review`"),
        "{notes:?}"
    );
    assert!(
        !steps_of(&orch, run)
            .await
            .iter()
            .any(|step| step.attempt > 2),
        "no third attempt"
    );
}

/// MOD-138 M2 D10: D6(c) is lifted. A live graph with an authored back `asks_you` edge, which
/// M1 refused as not yet runnable, starts a v2 run whose snapshot carries the edge.
async fn a_graph_with_authored_edges_resolves<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "review",
        "plan",
        EdgeKind::AsksYou,
        None,
        None,
    )
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "the run started and parked at `prd`'s gate"
    );
    let blob = run_of(&orch, run)
        .await
        .graph_snapshot
        .expect("the walk created the run with a snapshot");
    let snapshot: GraphSnapshot =
        serde_json::from_value(blob).expect("the engine wrote a `GraphSnapshot`");
    assert_eq!(snapshot.v, GraphSnapshot::V);
    assert!(
        snapshot
            .edges
            .iter()
            .any(|edge| (edge.from.as_str(), edge.to.as_str(), edge.kind)
                == ("review", "plan", EdgeKind::AsksYou)),
        "{:?}",
        snapshot.edges
    );
}

// -- MOD-138 M2 T5: `if` decisions and `asks_you` (plan D6, D7, D8; blueprint A2, A3, A5, A9) ---

/// An `if` edge's predicate, from its JSON.
fn when(value: serde_json::Value) -> Option<htui_core::model::EdgePredicate> {
    Some(htui_core::model::EdgePredicate(value))
}

/// The `branch` `record_branch` wrote on the step at `(position, 1)`, if any.
async fn branch_at<O: Orchestrate>(
    orch: &O,
    run: RunId,
    position: i32,
) -> Option<serde_json::Value> {
    step_at(orch, run, position, 1).await.branch
}

/// `ChangedPaths` in `repo`, sorted as the isolator answers them.
fn changed(repo: RepoId, paths: &[&str], truncated: bool) -> ChangedPaths {
    ChangedPaths {
        paths: paths
            .iter()
            .map(|path| (repo, (*path).to_owned()))
            .collect(),
        truncated,
    }
}

/// FEAT-3's diamond `prd → {plan, implement} → review`: `prd → plan` is `if` on `predicate`,
/// `prd → implement` on its negation, and both join at `review` over `next`.
async fn feat_3_diamond<O: Orchestrate>(orch: &O, predicate: &serde_json::Value) {
    author_edges(
        orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::If, when(predicate.clone()), None),
            (
                "prd",
                "implement",
                EdgeKind::If,
                when(serde_json::json!({ "not": predicate })),
                None,
            ),
            ("plan", "review", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
        ],
    )
    .await;
}

/// FEAT-3's chain with its `plan → implement` link an `asks_you` edge, and the review loop's
/// `changes_requested` edge kept.
async fn feat_3_asks_at_plan<O: Orchestrate>(orch: &O) {
    author_edges(
        orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::AsksYou, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            (
                "review",
                "implement",
                EdgeKind::ChangesRequested,
                None,
                Some(1),
            ),
        ],
    )
    .await;
}

/// FEAT-3's chain over `next` with a back `asks_you` edge `review → plan` of `loop_limit`, and
/// no `changes_requested` edge, so `review` reads no verdict and a reject declines.
async fn feat_3_asks_back<O: Orchestrate>(orch: &O, loop_limit: i32) {
    author_edges(
        orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            ("review", "plan", EdgeKind::AsksYou, None, Some(loop_limit)),
        ],
    )
    .await;
}

/// The shape of a FEAT-3 run after one turn of the back `asks_you` loop `review → plan`, its
/// second `review` parked.
const ONE_ASKS_YOU_TURN: [(i32, i32, StepStatus); 7] = [
    (0, 1, StepStatus::Done),
    (1, 1, StepStatus::Superseded),
    (1, 2, StepStatus::Done),
    (2, 1, StepStatus::Superseded),
    (2, 2, StepStatus::Done),
    (3, 1, StepStatus::Superseded),
    (3, 2, StepStatus::AwaitingApproval),
];

/// One round of [`each_if_predicate_decides_from_its_facts`]: the facts, then each of `prd`'s
/// three `if` edges with the decision it must get.
struct Round {
    why: &'static str,
    /// The changed paths in the primary repo `htui`; `None` leaves the isolator unscripted.
    paths: Option<(&'static [&'static str], bool)>,
    /// `prd` runs a `verify_command` that passes; else it has none (`verify_outcome` `NULL`).
    verify: bool,
    edges: [(&'static str, serde_json::Value, bool); 3],
}

/// MOD-138 M2 D3, D6 (plan Q-A): each predicate of the closed `if` set decides as specified.
/// One fresh run per round: `prd` (`never`) settles, the walk owes `Decide`, and the decision is
/// written once to `prd`'s `branch` as `{"<to>": bool}` per `if` edge. Paths are read only when
/// a predicate needs them (blueprint F13). The item requires the tag `gpu` and its key prefix is
/// the fixture's.
#[allow(clippy::too_many_lines)]
async fn each_if_predicate_decides_from_its_facts<H: CaseHarness>(harness: &H) {
    use serde_json::json;
    let prefix = item_of(&harness.fresh(), ids::HTUI_FEAT_3).await.key_prefix;
    let rounds = [
        Round {
            why: "paths: `all` over every path, `all` over some, a repo-qualified `any`",
            paths: Some((&["src/a.rs", "src/b.rs"], false)),
            verify: false,
            edges: [
                ("plan", json!({"all_changed_paths_match": ["src/**"]}), true),
                (
                    "implement",
                    json!({"all_changed_paths_match": ["src/a.rs"]}),
                    false,
                ),
                (
                    "review",
                    json!({"any_changed_path_matches": ["docs/**", "htui:src/b.rs"]}),
                    true,
                ),
            ],
        },
        Round {
            why: "no changed paths: no vacuous truth",
            paths: None,
            verify: false,
            edges: [
                ("plan", json!({"all_changed_paths_match": ["**"]}), false),
                (
                    "implement",
                    json!({"any_changed_path_matches": ["**"]}),
                    false,
                ),
                (
                    "review",
                    json!({"not": {"any_changed_path_matches": ["**"]}}),
                    true,
                ),
            ],
        },
        Round {
            why: "a truncated list: `all` cannot be proved, `any` reads the listed paths",
            paths: Some((&["src/a.rs"], true)),
            verify: false,
            edges: [
                (
                    "plan",
                    json!({"all_changed_paths_match": ["src/**"]}),
                    false,
                ),
                (
                    "implement",
                    json!({"any_changed_path_matches": ["src/**"]}),
                    true,
                ),
                (
                    "review",
                    json!({"any_changed_path_matches": ["docs/**"]}),
                    false,
                ),
            ],
        },
        Round {
            why: "a passing verify",
            paths: None,
            verify: true,
            edges: [
                ("plan", json!({"verify": "passed"}), true),
                ("implement", json!({"verify": "failed"}), false),
                ("review", json!({"not": {"verify": "failed"}}), true),
            ],
        },
        Round {
            why: "no verify outcome matches neither value",
            paths: None,
            verify: false,
            edges: [
                ("plan", json!({"verify": "passed"}), false),
                ("implement", json!({"verify": "failed"}), false),
                (
                    "review",
                    json!({"any": [{"verify": "passed"}, {"verify": "failed"}]}),
                    false,
                ),
            ],
        },
        Round {
            why: "the item's kind and tags",
            paths: None,
            verify: false,
            edges: [
                ("plan", json!({"item_kind": prefix}), true),
                ("implement", json!({"item_kind": "NOPE"}), false),
                ("review", json!({"item_tag": "gpu"}), true),
            ],
        },
        Round {
            why: "the combinators",
            paths: None,
            verify: false,
            edges: [
                (
                    "plan",
                    json!({"all": [{"item_tag": "gpu"}, {"item_kind": prefix}]}),
                    true,
                ),
                (
                    "implement",
                    json!({"any": [{"item_tag": "cuda"}, {"item_kind": "NOPE"}]}),
                    false,
                ),
                ("review", json!({"not": {"item_tag": "cuda"}}), true),
            ],
        },
    ];
    for round in rounds {
        let why = round.why;
        let orch = harness.fresh();
        let repo = primary_repo(&orch).await;
        require_tags(&orch, ids::HTUI_FEAT_3, &["gpu"]).await;
        declare_tags(&orch, &["gpu"]).await;
        let verify = round.verify;
        // `prd` walks on by itself; every target parks at its own gate, so the run stops at the
        // first one the decision takes.
        feat_3_gated(&orch, Gate::Always, |phase| {
            phase.input_kinds.clear();
            if phase.name == "prd" {
                phase.gate = Gate::Never;
                if verify {
                    phase.verify_command = Some("cargo test".to_owned());
                }
            }
        })
        .await;
        if verify {
            orch.verifier().script_report(FakeVerifier::pass());
        }
        if let Some((paths, truncated)) = round.paths {
            orch.isolator()
                .script_changed_paths(changed(repo, paths, truncated));
        }
        let edges: Vec<AuthoredEdge<'_>> = round
            .edges
            .iter()
            .map(|(to, predicate, _)| ("prd", *to, EdgeKind::If, when(predicate.clone()), None))
            .collect();
        author_edges(&orch, ids::HTUI_FEAT_3, &edges).await;

        let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
        let expected: serde_json::Map<String, serde_json::Value> = round
            .edges
            .iter()
            .map(|(to, _, taken)| ((*to).to_owned(), serde_json::Value::Bool(*taken)))
            .collect();
        assert_eq!(
            branch_at(&orch, run, 0).await,
            Some(serde_json::Value::Object(expected)),
            "{why}"
        );
        let needs_paths = round
            .edges
            .iter()
            .any(|(_, predicate, _)| predicate.to_string().contains("changed_path"));
        assert_eq!(
            orch.isolator().changed_path_requests().len(),
            usize::from(needs_paths),
            "{why}: the paths are read once, and only when a predicate needs them"
        );
    }
}

/// MOD-138 M2 D2: on the diamond `prd → {plan, implement} → review`, the decision takes `plan`
/// and declines `implement`. `implement` is skipped (no row is ever written for it), and the
/// join `review` runs once its taken in-edge's source is done: a join waits for every in-edge
/// to resolve, not for every one to be taken.
async fn a_declined_if_branch_is_skipped_and_the_join_runs<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| phase.input_kinds.clear()).await;
    let prefix = item_of(&orch, ids::HTUI_FEAT_3).await.key_prefix;
    feat_3_diamond(&orch, &serde_json::json!({ "item_kind": prefix })).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ],
        "`implement` is skipped and never gets a row"
    );
    assert_eq!(
        branch_at(&orch, run, 0).await,
        Some(serde_json::json!({"plan": true, "implement": false}))
    );
    assert!(
        orch.isolator().changed_path_requests().is_empty(),
        "no predicate needs paths"
    );
}

/// MOD-138 M2 D6: a decision is never guessed. The isolator cannot read `prd`'s changed paths,
/// so nothing is written to `branch`; the run parks with the `branch undecided` note, the item
/// is not blocked, and `Unblock` resumes the run (`Decide` is resumable), which decides again,
/// now with the paths, and walks on.
async fn an_undecided_branch_parks_on_an_isolator_error_and_unblock_resumes_it<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    let repo = primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |phase| phase.input_kinds.clear()).await;
    feat_3_diamond(
        &orch,
        &serde_json::json!({"any_changed_path_matches": ["src/**"]}),
    )
    .await;
    orch.isolator().fail_changed_paths("git is gone");

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "{rest:?}"
    );
    assert_eq!(sorted_shape(&orch, run).await, [(0, 1, StepStatus::Done)]);
    assert_eq!(branch_at(&orch, run, 0).await, None, "nothing was guessed");
    assert_eq!(item_of(&orch, ids::HTUI_FEAT_3).await.status, Status::Open);
    assert_eq!(state_of(&orch, ids::HTUI_FEAT_3).await, ItemState::NeedsYou);
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body
            .starts_with("branch undecided: `prd`: changed paths unavailable: ")
            && body.contains("git is gone")),
        "{notes:?}"
    );

    orch.isolator()
        .script_changed_paths(changed(repo, &["src/lib.rs"], false));
    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(case, UnblockCase::Resume(run));
    let rest = rest.expect("the resume case walks the run");
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        branch_at(&orch, run, 0).await,
        Some(serde_json::json!({"plan": true, "implement": false}))
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    assert_eq!(orch.isolator().changed_path_requests().len(), 2);
}

/// MOD-138 M2 D8 (plan Q-B): `plan → implement` is an `asks_you` edge, so `plan` stops after its
/// `ok` settle although every phase gates `never`, with the stop note. Approve takes the edge:
/// `implement` runs, and the run finishes.
async fn a_forward_asks_you_approved_runs_its_target<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    feat_3_asks_at_plan(&orch).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(1), None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval)
        ]
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body
            == "step `plan` attempt 1 waits on you: approve takes `implement`; reject or accept \
                artifact declines it, and the step's work lands either way; retry redoes the step"),
        "{notes:?}"
    );

    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    assert_eq!(at(&steps, 1, 1).gate_outcome, Some(GateOutcome::Approved));
}

/// MOD-138 M2 D8 (plan Q-B): a human's reject of a node whose only loop-free edge out is
/// `asks_you` declines it: the step is `done` with `skipped` and the reject's note, its work is
/// merged, `implement` and `review` are skipped (the skip propagates), and the run finishes.
async fn a_forward_asks_you_rejected_skips_its_target_and_lands_the_work<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    feat_3_asks_at_plan(&orch).await;

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let plan = step_at(&orch, run, 1, 1).await;
    assert_eq!(plan.status, StepStatus::AwaitingApproval);
    let rest = act(&orch, run, Act::Reject).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "a declined link ends nothing: {rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [(0, 1, StepStatus::Done), (1, 1, StepStatus::Done)],
        "`implement` and `review` are skipped"
    );
    let plan = step_at(&orch, run, 1, 1).await;
    assert_eq!(
        (plan.gate_outcome, plan.gate_note.as_deref()),
        (Some(GateOutcome::Skipped), Some("not this"))
    );
    assert!(
        orch.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == plan.id),
        "the declined step's work landed"
    );
}

/// MOD-138 M2 D8, blueprint F14: auto mode downgrades every soft gate to `never`, and an
/// `asks_you` node still stops; approve walks on, and the other steps pass `skipped`.
async fn auto_mode_never_downgrades_an_asks_you_stop<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Always, |phase| phase.gate_hard = false).await;
    feat_3_asks_at_plan(&orch).await;

    let (run, rest) = start_in(&orch, ids::HTUI_FEAT_3, RunMode::Auto).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(1), None),
        "{rest:?}"
    );
    let snapshot = snapshot_of(&orch, run).await;
    assert!(
        snapshot
            .phases
            .iter()
            .all(|phase| phase.gate_effective == Gate::Never),
        "auto mode downgraded every gate"
    );
    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        (0..4)
            .map(|position| at(&steps, position, 1).gate_outcome)
            .collect::<Vec<_>>(),
        [
            Some(GateOutcome::Skipped),
            Some(GateOutcome::Approved),
            Some(GateOutcome::Skipped),
            Some(GateOutcome::Skipped),
        ]
    );
}

/// MOD-138 M2 D7: a back `asks_you` edge `review → plan` loops on approve. The approved `review`
/// is `done`, the walk owes `Loop`, and the loop retires `plan`, `implement` and `review` (the
/// region) and walks them again, `review` stopping at attempt 2 with budget left. A reject there
/// declines the edge and the run finishes.
async fn a_back_asks_you_approved_loops<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    feat_3_asks_back(&orch, 2).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(3), None),
        "{rest:?}"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body
            == "step `review` attempt 1 waits on you: approve takes `plan` (loops back); reject \
                or accept artifact declines it, and the step's work lands either way; retry \
                redoes the step"),
        "{notes:?}"
    );

    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(3), None),
        "{rest:?}"
    );
    assert_eq!(sorted_shape(&orch, run).await, ONE_ASKS_YOU_TURN);
    assert_eq!(
        step_at(&orch, run, 3, 1).await.gate_outcome,
        Some(GateOutcome::Approved),
        "the approve stays on the retired row"
    );

    let rest = act(&orch, run, Act::Reject).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "a reject declines the back edge: {rest:?}"
    );
    assert_eq!(
        step_at(&orch, run, 3, 2).await.gate_outcome,
        Some(GateOutcome::Skipped)
    );
}

/// MOD-138 M2 D7, blueprint §2.4: a crash between the approve's `answer_gate` and its unpark
/// leaves the run parked over a `done` + `approved` `review`, whose cursor is `Loop`, which is
/// resumable. `Unblock` resumes the run: the unpark, the merge the approve owed, then the loop.
async fn a_crash_between_an_asks_you_approve_and_its_loop_is_replayed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    feat_3_asks_back(&orch, 2).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    let review = step_at(&orch, run, 3, 1).await;
    assert!(
        orch.store()
            .answer_gate(review.id, GateOutcome::Approved, None, orch.clock().now())
            .await
            .expect("MemStore takes the answer"),
        "the crash: answered, never unparked"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);

    let (case, rest) = unblock(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(case, UnblockCase::Resume(run));
    let rest = rest.expect("the resume case walks the run");
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(3), None),
        "{rest:?}"
    );
    assert_eq!(sorted_shape(&orch, run).await, ONE_ASKS_YOU_TURN);
    assert!(
        orch.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == review.id),
        "the approved review's merge was replayed"
    );
}

/// MOD-138 M2 D7, blueprint A9: with `loop_limit = 1` the second `review` stops as the
/// escalation (`asks_you loop exhausted`), the item is not blocked, and approve then walks on:
/// no `Loop` is owed once the budget is spent, so the run finishes with no third attempt.
async fn a_spent_back_asks_you_escalates_and_approve_walks_on<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    feat_3_asks_back(&orch, 1).await;
    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;

    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (
            RunStatus::AwaitingApproval,
            Some(3),
            Some(RunFailure::LoopExhausted {
                kind: EdgeKind::AsksYou,
                from: "review".to_owned(),
                to: "plan".to_owned(),
                attempts: 2,
            })
        ),
        "{rest:?}"
    );
    assert_eq!(sorted_shape(&orch, run).await, ONE_ASKS_YOU_TURN);
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "A9: the escalation does not block the item"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| body
            == "asks_you loop exhausted after 2 attempts: `review` to `plan`: phase `plan`, \
                attempt 2, stop reason `exhausted`"),
        "{notes:?}"
    );

    let rest = approve(&orch, run, 1).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert!(
        steps_of(&orch, run)
            .await
            .iter()
            .all(|step| step.attempt <= 2),
        "no third attempt"
    );
    assert_eq!(
        step_at(&orch, run, 3, 2).await.gate_outcome,
        Some(GateOutcome::Approved)
    );
}

/// MOD-138 M2 plan Q-A: a decision is written once and read from its row. `prd` decides on its
/// changed paths and the run parks at `plan`'s gate; a restarted process (a fresh isolator)
/// approves it, and walks the stored decision with no second `changed_paths` call, although its
/// isolator would now answer no paths at all.
async fn a_branch_decision_is_read_from_its_row_after_a_restart<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let repo = primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |phase| {
        phase.input_kinds.clear();
        if phase.name == "plan" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    feat_3_diamond(
        &orch,
        &serde_json::json!({"any_changed_path_matches": ["src/**"]}),
    )
    .await;
    orch.isolator()
        .script_changed_paths(changed(repo, &["src/lib.rs"], false));
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "{rest:?}"
    );
    let decided = Some(serde_json::json!({"plan": true, "implement": false}));
    assert_eq!(branch_at(&orch, run, 0).await, decided);
    assert_eq!(orch.isolator().changed_path_requests().len(), 1);

    let other = orch.restarted();
    other
        .isolator()
        .script_changed_paths(ChangedPaths::default());
    let rest = approve(&other, run, 1).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert!(
        other.isolator().changed_path_requests().is_empty(),
        "the decision was read from its row"
    );
    assert_eq!(branch_at(&other, run, 0).await, decided);
    assert_eq!(
        sorted_shape(&other, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
}

/// Blueprint A2 (plan Q-B over D8): accept artifact on a node with an `asks_you` edge declines
/// it. `prd → plan` is `asks_you`; `prd` is promoted and accepted: it lands `done` with
/// `skipped`, its work is merged, every later node is skipped, and the run finishes.
async fn accepting_an_artifact_on_an_asks_you_node_declines_its_edges<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    free_feat_3(&orch).await;
    author_edges(
        &orch,
        ids::HTUI_FEAT_3,
        &[
            ("prd", "plan", EdgeKind::AsksYou, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            (
                "review",
                "implement",
                EdgeKind::ChangesRequested,
                None,
                Some(1),
            ),
        ],
    )
    .await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(0)),
        "{rest:?}"
    );
    let prd = step_at(&orch, run, 0, 1).await;
    promote(&orch, run, prd.id).await;
    orch.verifier().script_report(FakeVerifier::pass());

    let outcome = accept(&orch, run, prd.id)
        .await
        .expect("a promoted step with its document");
    let CommandOutcome::Accepted { rest } = outcome else {
        panic!("`AcceptArtifact` answers `Accepted`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let prd = step_at(&orch, run, 0, 1).await;
    assert_eq!(
        (prd.status, prd.gate_outcome),
        (StepStatus::Done, Some(GateOutcome::Skipped))
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [(0, 1, StepStatus::Done)],
        "`plan` and everything after it are skipped"
    );
    assert!(
        orch.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == prd.id),
        "the accepted work landed"
    );
}

/// Blueprint A3: a human's reject on a node with both a `changes_requested` and an `asks_you`
/// edge loops over `changes_requested`, which wins over the decline. FEAT-3's `review` keeps
/// its review loop and gains a back `asks_you` to `plan`: the reject retires `implement` and the
/// rejected `review` (not `plan`), and the second `review` stops again.
async fn a_reject_on_a_verdict_reading_asks_you_node_loops<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "review",
        "plan",
        EdgeKind::AsksYou,
        None,
        Some(2),
    )
    .await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(3)),
        "the approving review stops at its `asks_you` edge: {rest:?}"
    );

    let rest = act(&orch, run, Act::Reject).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(3), None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Superseded),
            (2, 2, StepStatus::Done),
            (3, 1, StepStatus::Cancelled),
            (3, 2, StepStatus::AwaitingApproval),
        ]
    );
    let review = step_at(&orch, run, 3, 1).await;
    assert_eq!(
        (review.gate_outcome, review.gate_note.as_deref()),
        (Some(GateOutcome::Rejected), Some("not this"))
    );
}

/// MOD-138 M2 D8 ("on a failed settle its own gate applies"): only the stop of an `ok` settle
/// declines on a reject. `plan → implement` is `asks_you` and `plan` gates `always`; `plan`
/// refuses, so it parks at its own gate with no stop note, and a human's reject there is the
/// ordinary one (blueprint A3, with no `changes_requested` or `fails` edge out of `plan`): the
/// step is `failed` with `rejected`, the run ends `rejected: plan`, and none of `plan`'s work
/// lands.
async fn a_reject_of_a_failed_park_on_an_asks_you_node_ends_the_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    primary_repo(&orch).await;
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "plan" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    feat_3_asks_at_plan(&orch).await;
    orch.script(
        "plan",
        1,
        ScriptedStep::failing(htui_agent::event::StopReason::Refusal),
    );

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (RunStatus::AwaitingApproval, Some(1), None),
        "the failed `plan` parks at its own gate: {rest:?}"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        !notes.iter().any(|body| body.contains(" waits on you: ")),
        "a failed settle writes no stop note: {notes:?}"
    );

    let rest = act(&orch, run, Act::Reject).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (
            RunStatus::Failed,
            Some(1),
            Some(RunFailure::Rejected {
                phase: "plan".to_owned()
            })
        ),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [(0, 1, StepStatus::Done), (1, 1, StepStatus::Failed)]
    );
    let plan = step_at(&orch, run, 1, 1).await;
    assert_eq!(
        (plan.gate_outcome, plan.gate_note.as_deref()),
        (Some(GateOutcome::Rejected), Some("not this"))
    );
    assert!(
        !orch
            .isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == plan.id),
        "the failed step's work did not land"
    );
}

// -- MOD-138 M3 T3: concurrent nodes (plan D1, D3, D4; blueprint A2-A6, A11) --------------------

/// MOD-138 M3 T3's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn concurrency_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_linear_graph_keeps_its_call_order" => {
            Box::pin(a_linear_graph_keeps_its_call_order(harness))
        }
        "two_independent_branches_are_live_at_once" => {
            Box::pin(two_independent_branches_are_live_at_once(harness))
        }
        "three_branches_under_a_cap_of_two_never_run_three" => {
            Box::pin(three_branches_under_a_cap_of_two_never_run_three(harness))
        }
        "a_fan_out_heavier_than_the_cap_runs_alone" => {
            Box::pin(a_fan_out_heavier_than_the_cap_runs_alone(harness))
        }
        "a_join_waits_for_both_predecessors_to_land" => {
            Box::pin(a_join_waits_for_both_predecessors_to_land(harness))
        }
        "two_worktree_landings_into_one_repository_both_land" => {
            Box::pin(two_worktree_landings_into_one_repository_both_land(harness))
        }
        "shared_serialized_branches_never_overlap" => {
            Box::pin(shared_serialized_branches_never_overlap(harness))
        }
        "an_in_place_node_never_overlaps_a_worktree_node" => {
            Box::pin(an_in_place_node_never_overlaps_a_worktree_node(harness))
        }
        "an_owed_selection_lands_before_an_in_place_node" => {
            Box::pin(an_owed_selection_lands_before_an_in_place_node(harness))
        }
        "a_park_on_one_branch_drains_the_other" => {
            Box::pin(a_park_on_one_branch_drains_the_other(harness))
        }
        "a_sibling_parking_on_a_parked_run_parks_its_step_only" => Box::pin(
            a_sibling_parking_on_a_parked_run_parks_its_step_only(harness),
        ),
        "one_answer_of_two_parks_re_parks_without_walking_on" => {
            Box::pin(one_answer_of_two_parks_re_parks_without_walking_on(harness))
        }
        "a_node_ending_the_run_cuts_its_live_sibling" => {
            Box::pin(a_node_ending_the_run_cuts_its_live_sibling(harness))
        }
        "a_park_then_an_end_cancels_the_parked_step" => {
            Box::pin(a_park_then_an_end_cancels_the_parked_step(harness))
        }
        "a_terminal_branch_defers_cleanup_until_its_sibling_settles" => Box::pin(
            a_terminal_branch_defers_cleanup_until_its_sibling_settles(harness),
        ),
        "a_loop_on_one_branch_retires_only_its_region" => {
            Box::pin(a_loop_on_one_branch_retires_only_its_region(harness))
        }
        "a_terminal_branch_defers_cleanup_until_its_sibling_is_cut" => Box::pin(
            a_terminal_branch_defers_cleanup_until_its_sibling_is_cut(harness),
        ),
        "a_terminal_end_cancels_a_sibling_row_created_beside_it" => Box::pin(
            a_terminal_end_cancels_a_sibling_row_created_beside_it(harness),
        ),
        "a_rejection_ending_the_run_cancels_the_other_parked_step" => Box::pin(
            a_rejection_ending_the_run_cancels_the_other_parked_step(harness),
        ),
        "a_loop_owed_beside_its_successor_runs_first" => {
            Box::pin(a_loop_owed_beside_its_successor_runs_first(harness))
        }
        _ => return None,
    })
}

/// The isolator's call log, each step named `position/attempt/fanout_index` and the run `run`.
async fn call_log_of<O: Orchestrate>(orch: &O, run: RunId) -> Vec<String> {
    let steps = steps_of(orch, run).await;
    let name = |step: StepId| {
        steps.iter().find(|row| row.id == step).map_or_else(
            || "?".to_owned(),
            |row| format!("{}/{}/{}", row.position, row.attempt, row.fanout_index),
        )
    };
    orch.isolator()
        .call_log()
        .into_iter()
        .map(|call| match call {
            IsolatorCall::Prepare(step) => format!("prepare {}", name(step)),
            IsolatorCall::Capture(step) => format!("capture {}", name(step)),
            IsolatorCall::Reconcile(step) => format!("reconcile {}", name(step)),
            IsolatorCall::Reset(step) => format!("reset {}", name(step)),
            IsolatorCall::Cleanup(id) => format!("cleanup {}", if id == run { "run" } else { "?" }),
            IsolatorCall::Release(id) => format!("release {}", if id == run { "run" } else { "?" }),
        })
        .collect()
}

/// MOD-138 M3 D1 (plan risk "a linear graph's behaviour shifts"): a chain still runs one node
/// future at a time, so FEAT-3 walked ungated over a repository makes exactly the isolator calls
/// it made before the concurrent scheduler, in the same order. The literal was recorded on the
/// serial walk (blueprint §7's first TDD step).
async fn a_linear_graph_keeps_its_call_order<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    primary_repo(&orch).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        call_log_of(&orch, run).await,
        [
            "prepare 0/1/0",
            "capture 0/1/0",
            "reconcile 0/1/0",
            "prepare 1/1/0",
            "capture 1/1/0",
            "reconcile 1/1/0",
            "prepare 2/1/0",
            "capture 2/1/0",
            "reconcile 2/1/0",
            "prepare 3/1/0",
            "capture 3/1/0",
            "reconcile 3/1/0",
            "cleanup run",
        ],
        "one node future at a time, in position order, cleanup once at the end"
    );
    assert_eq!(orch.live_sessions_high_water(), 1);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 T3: how long a concurrency case waits on held sessions before it calls the walk
/// serial. Real time: the sessions are held by the fake, never by the clock.
const HELD: std::time::Duration = std::time::Duration::from_secs(20);

/// `fut` under [`HELD`]: a serial walk never has two held sessions live, so it would wait for ever.
async fn within<F: Future>(fut: F, what: &str) -> F::Output {
    tokio::time::timeout(HELD, fut)
        .await
        .unwrap_or_else(|_| panic!("{what}: the walk did not get there; is it serial?"))
}

/// Yields until `check` holds over the run's steps: the case's half of a `join` with the walk.
async fn until_steps<O: Orchestrate>(orch: &O, run: RunId, check: impl Fn(&[RunStep]) -> bool) {
    while !check(&steps_of(orch, run).await) {
        tokio::task::yield_now().await;
    }
}

/// The FEAT-3 run the case started, once `StartRun` wrote it (the seeded `RUN_2` aside).
async fn feat_3_run<O: Orchestrate>(orch: &O) -> Option<RunId> {
    orch.store()
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|run| run.id != ids::RUN_2)
        .map(|run| run.id)
}

/// FEAT-3 ungated, every phase's `input_kinds` cleared and `mutate` applied, over `edges`.
async fn feat_3_graph<O: Orchestrate>(
    orch: &O,
    edges: &[AuthoredEdge<'_>],
    mutate: impl Fn(&mut StepGraphPhase),
) {
    feat_3_gated(orch, Gate::Never, |phase| {
        phase.input_kinds.clear();
        mutate(phase);
    })
    .await;
    author_edges(orch, ids::HTUI_FEAT_3, edges).await;
}

/// The diamond `prd → {plan, implement} → review`, every edge `next`.
const DIAMOND: &[AuthoredEdge<'static>] = &[
    ("prd", "plan", EdgeKind::Next, None, None),
    ("prd", "implement", EdgeKind::Next, None, None),
    ("plan", "review", EdgeKind::Next, None, None),
    ("implement", "review", EdgeKind::Next, None, None),
];

/// `prd → {plan, implement, review}`: three branches, every one a sink.
const FAN_OF_THREE: &[AuthoredEdge<'static>] = &[
    ("prd", "plan", EdgeKind::Next, None, None),
    ("prd", "implement", EdgeKind::Next, None, None),
    ("prd", "review", EdgeKind::Next, None, None),
];

/// `prd → plan → review` and `prd → implement`, a sink beside the chain.
const CHAIN_AND_SINK: &[AuthoredEdge<'static>] = &[
    ("prd", "plan", EdgeKind::Next, None, None),
    ("plan", "review", EdgeKind::Next, None, None),
    ("prd", "implement", EdgeKind::Next, None, None),
];

/// Answers the parked step at `position` (fan-out index 0, its latest attempt).
async fn answer_at<O: Orchestrate>(
    orch: &O,
    run: RunId,
    position: i32,
    answer: GateAnswer,
) -> Rest {
    let step = steps_of(orch, run)
        .await
        .into_iter()
        .filter(|step| step.position == position && step.fanout_index == 0)
        .max_by_key(|step| step.attempt)
        .expect("the position has a step");
    assert_eq!(step.status, StepStatus::AwaitingApproval, "{step:?}");
    let outcome = orch
        .dispatch(Command::AnswerGate {
            run,
            step: step.id,
            answer,
        })
        .await
        .expect("the parked gate is answerable");
    let CommandOutcome::Answered { rest } = outcome else {
        panic!("`AnswerGate` answers `Answered`, not {outcome:?}");
    };
    rest
}

/// The latest row at `position` (fan-out index 0), if any.
fn latest_row(steps: &[RunStep], position: i32) -> Option<&RunStep> {
    steps
        .iter()
        .filter(|step| step.position == position && step.fanout_index == 0)
        .max_by_key(|step| step.attempt)
}

/// MOD-138 M3 D1: on the diamond `prd → {plan, implement} → review`, `plan` and `implement` are
/// incomparable, so both sessions are live at once (held by the fake until both are), and the
/// join runs once both landed.
async fn two_independent_branches_are_live_at_once<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |_| {}).await;
    let hold = orch.hold_sessions(&[("plan", 1), ("implement", 1)]);

    let release = async {
        within(hold.live(2), "both branches live").await;
        hold.release();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        orch.live_sessions_high_water(),
        2,
        "the two branches, and never more"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D1: three branches under a frozen `max_parallel_sessions = 2` never have more than
/// two sessions live; the third runs once one of them ended, and all three finish.
async fn three_branches_under_a_cap_of_two_never_run_three<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(2));
    feat_3_graph(&orch, FAN_OF_THREE, |_| {}).await;
    let hold = orch.hold_sessions(&[("plan", 1), ("implement", 1), ("review", 1)]);

    let release = async {
        within(hold.live(2), "two branches live").await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(hold.reached(), 2, "the cap holds the third back");
        hold.release();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(orch.live_sessions_high_water(), 2);
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D1: a node whose `fan_out` (3) is above the cap (2) still runs, alone: its three
/// candidates are live together and `implement` (weight 1) is not even created beside the group.
/// The judgeless group parks for a human, which drains the walk; the selection walks on.
async fn a_fan_out_heavier_than_the_cap_runs_alone<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(2));
    orch.store()
        .set_app_setting("max_agents_per_run", serde_json::json!(16));
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" {
            phase.fan_out = 3;
        }
    })
    .await;
    let hold = orch.hold_sessions(&[("plan", 1), ("implement", 1)]);

    let release = async {
        within(hold.live(3), "the group live").await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            hold.reached(),
            3,
            "`implement` waits beside the heavy group"
        );
        hold.release();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the judgeless group parks for a human: {rest:?}"
    );
    assert_eq!(orch.live_sessions_high_water(), 3);
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        latest_row(&steps, 2).map(|step| step.status),
        Some(StepStatus::Pending),
        "`implement` was created beside the weightless selection, and the park drained the walk"
    );
    assert!(latest_row(&steps, 3).is_none(), "the join waits");

    let rest = act(
        &orch,
        run,
        Act::Select {
            position: 1,
            index: 0,
        },
    )
    .await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(orch.live_sessions_high_water(), 3);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D2: a join is not created until both predecessors landed. `plan`'s landing is held
/// inside its reconcile; `implement` finishes and lands meanwhile, and `review` has no row until
/// `plan`'s reconcile answers.
async fn a_join_waits_for_both_predecessors_to_land<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |_| {}).await;
    primary_repo(&orch).await;
    // `prd`'s landing is the first reconcile, `plan`'s the second: `implement`'s session is held
    // until `plan` is inside it.
    let implement = orch.hold_sessions(&[("implement", 1)]);
    let (reached, resume) = orch.isolator().pause_nth_reconcile(2);

    let meddle = async {
        within(reached.notified(), "plan's landing").await;
        implement.release();
        let run = feat_3_run(&orch).await.expect("the run exists");
        within(
            until_steps(&orch, run, |steps| {
                latest_row(steps, 2).is_some_and(|step| step.status == StepStatus::Done)
                    && orch.isolator().reconciles().len() == 3
            }),
            "implement landed",
        )
        .await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            latest_row(&steps, 1).map(|step| step.status),
            Some(StepStatus::Done),
            "`plan` is done, its landing held"
        );
        assert!(
            latest_row(&steps, 3).is_none(),
            "no join row while a predecessor's landing is in flight"
        );
        resume.notify_one();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), meddle);
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    let log = call_log_of(&orch, run).await;
    let first = |entry: &str| {
        log.iter()
            .position(|call| call == entry)
            .unwrap_or_else(|| panic!("{entry} in {log:?}"))
    };
    assert!(first("reconcile 1/1/0") < first("prepare 3/1/0"), "{log:?}");
    assert!(first("reconcile 2/1/0") < first("prepare 3/1/0"), "{log:?}");
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D5: two `worktree` branches into one repository both land, each its own merge, and
/// the join runs after both. The fake has no admin lock, so their landings may interleave here;
/// the real isolator's per-repository lock is what serialises them (`tests/gix_isolator.rs`).
async fn two_worktree_landings_into_one_repository_both_land<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        phase.isolation = Some(Isolation::Worktree);
    })
    .await;
    primary_repo(&orch).await;
    orch.isolator().interleave_reconciles();
    orch.interleave_sessions();

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    let log = call_log_of(&orch, run).await;
    for landing in ["reconcile 1/1/0", "reconcile 2/1/0"] {
        assert_eq!(
            log.iter().filter(|call| *call == landing).count(),
            1,
            "{landing} once: {log:?}"
        );
    }
    assert_eq!(
        orch.live_sessions_high_water(),
        2,
        "both branches were live together"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D4 (A3): two `shared_serialized` branches work in the primary checkout, so they
/// never overlap: one tree-touching future at a time, sessions and landings alike, even with
/// every session and landing yielding.
async fn shared_serialized_branches_never_overlap<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.isolation = Some(Isolation::SharedSerialized);
        }
    })
    .await;
    primary_repo(&orch).await;
    orch.isolator().interleave_reconciles();
    orch.interleave_sessions();

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        orch.isolator().in_place_overlaps(),
        0,
        "{:?}",
        call_log_of(&orch, run).await
    );
    assert_eq!(orch.isolator().reconcile_high_water(), 1);
    assert_eq!(orch.live_sessions_high_water(), 1);
}

/// MOD-138 M3 A3: an in-place (`shared_serialized`) branch never overlaps a `worktree` one: a
/// worktree landing merges into, and refuses on, the checkout the in-place agent edits.
async fn an_in_place_node_never_overlaps_a_worktree_node<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        phase.isolation = Some(match phase.name.as_str() {
            "plan" => Isolation::SharedSerialized,
            _ => Isolation::Worktree,
        });
    })
    .await;
    primary_repo(&orch).await;
    orch.isolator().interleave_reconciles();
    orch.interleave_sessions();

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        orch.isolator().in_place_overlaps(),
        0,
        "{:?}",
        call_log_of(&orch, run).await
    );
    assert_eq!(orch.isolator().reconcile_high_water(), 1);
    assert_eq!(orch.live_sessions_high_water(), 1);
}

/// MOD-138 M3 (T1 verify, A9): an owed selection lands before an in-place node touches its trees.
/// `audit` (FEAT-3's `review`, renamed so it may fan out) fans out two `worktree` candidates off
/// `prd`; `implement` (`shared_serialized`) waits behind `plan`. When `audit`'s group settles,
/// `implement`'s run and `audit`'s auto-win are both owed and `implement` sorts first; the
/// winner still lands before `implement` prepares its checkout.
async fn an_owed_selection_lands_before_an_in_place_node<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(
        &orch,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("prd", "audit", EdgeKind::Next, None, None),
        ],
        |phase| {
            phase.isolation = Some(Isolation::Worktree);
            if phase.name == "implement" {
                phase.isolation = Some(Isolation::SharedSerialized);
            }
            if phase.name == "review" {
                phase.name = "audit".to_owned();
                phase.fan_out = 2;
                phase.verify_command = Some("cargo test".to_owned());
            }
        },
    )
    .await;
    primary_repo(&orch).await;
    set_judge(&orch, ids::HTUI_FEAT_3).await;
    // The fake answers every step's verify in call order: `prd`, then `plan` and the group in
    // whichever order they reach it; exactly one candidate passes either way.
    for report in [
        FakeVerifier::pass(),
        FakeVerifier::pass(),
        FakeVerifier::fail(1),
        FakeVerifier::pass(),
    ] {
        orch.verifier().script_report(report);
    }

    // The group is held until `implement` is created, so its selection and `implement`'s run are
    // owed in one pass.
    let group = orch.hold_sessions(&[("audit", 1)]);
    let release = async {
        within(group.live(2), "the group live").await;
        let run = feat_3_run(&orch).await.expect("the run exists");
        within(
            until_steps(&orch, run, |steps| latest_row(steps, 2).is_some()),
            "implement created",
        )
        .await;
        group.release();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?} {:?}",
        notes_of(&orch, ids::HTUI_FEAT_3).await
    );
    let log = call_log_of(&orch, run).await;
    let first = |entry: &str| {
        log.iter()
            .position(|call| call.starts_with(entry))
            .unwrap_or_else(|| panic!("{entry} in {log:?}"))
    };
    assert!(
        first("reconcile 3/1/") < first("prepare 2/1/0"),
        "the auto-win landed before the in-place node prepared: {log:?}"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0, "{log:?}");
}

/// MOD-138 M3 D3 (Q-2): a park on one branch drains the other. `plan` parks at its gate;
/// `implement`, already admitted, runs on through its session, gate and landing; the run rests
/// at `plan`'s park and the join is not created.
async fn a_park_on_one_branch_drains_the_other<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    primary_repo(&orch).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(1), None)
    );
    let steps = steps_of(&orch, run).await;
    let implement = latest_row(&steps, 2).expect("`implement` was admitted");
    assert_eq!(implement.status, StepStatus::Done, "the sibling ran on");
    assert!(
        orch.isolator()
            .reconciles()
            .iter()
            .any(|(winner, _)| *winner == implement.id),
        "and landed"
    );
    assert!(latest_row(&steps, 3).is_none(), "nothing new was admitted");
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D3 (V15): a sibling parking on a run its sibling already parked parks its step
/// only. Both are answered in turn: the first answer parks the run again on the second (A6, no
/// note), and the second walks the run on to `done`.
async fn a_sibling_parking_on_a_parked_run_parks_its_step_only<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the first park's rest (A11)"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
            (2, 1, StepStatus::AwaitingApproval),
        ]
    );

    let rest = answer_at(&orch, run, 1, GateAnswer::Approved).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(2)),
        "parked again on `implement`"
    );
    let rest = answer_at(&orch, run, 2, GateAnswer::Approved).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (2, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ]
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 A6 (V20): after a drain the run rests on two parked steps. Answering one walks no
/// further: `plan`'s successor `review` is not created while `implement` still waits, the run is
/// parked again with no second note, and the second answer walks on.
async fn one_answer_of_two_parks_re_parks_without_walking_on<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, CHAIN_AND_SINK, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;

    let rest = answer_at(&orch, run, 1, GateAnswer::Approved).await;
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (RunStatus::AwaitingApproval, Some(2), None),
        "re-parked on `implement`"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    let steps = steps_of(&orch, run).await;
    assert!(
        latest_row(&steps, 3).is_none(),
        "`review` is not created while a step still waits (A6)"
    );
    assert_eq!(
        notes_of(&orch, ids::HTUI_FEAT_3).await,
        notes,
        "the re-park writes no note"
    );

    let rest = answer_at(&orch, run, 2, GateAnswer::Approved).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    assert_eq!(
        latest_row(&steps_of(&orch, run).await, 3).map(|step| step.status),
        Some(StepStatus::Done)
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// FEAT-3 on the diamond with `implement` failing for good: it writes no output document and
/// has no retry left, so its settle ends the run.
async fn diamond_with_a_failing_implement<O: Orchestrate>(orch: &O) {
    feat_3_graph(orch, DIAMOND, |phase| {
        if phase.name == "implement" {
            phase.retry_limit = 0;
        }
    })
    .await;
    orch.script("implement", 1, ScriptedStep::done_without_output());
}

/// MOD-138 M3 D3 (A5): a node ending the run cuts its live sibling. `plan`'s session is held
/// live; `implement`'s session (held until `plan`'s is live) ends with no output and no retry
/// left, which fails the run. The walk's cut cancels `plan`'s session, its row ends `cancelled`
/// with a note naming `implement`, and cleanup runs once.
async fn a_node_ending_the_run_cuts_its_live_sibling<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    diamond_with_a_failing_implement(&orch).await;
    let plan = orch.hold_sessions(&[("plan", 1)]);
    let implement = orch.hold_sessions(&[("implement", 1)]);

    let release = async {
        within(plan.live(1), "plan live").await;
        implement.release();
    };
    let ((run, rest), ()) = within(
        async { tokio::join!(start(&orch, ids::HTUI_FEAT_3), release) },
        "the cut",
    )
    .await;
    assert_eq!(rest.run, RunStatus::Failed, "{rest:?}");
    assert_eq!(
        plan.reached(),
        1,
        "`plan`'s session was live when the run ended"
    );
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran");
    assert_eq!(plan.status, StepStatus::Cancelled);
    assert_eq!(
        latest_row(&steps, 2).map(|step| step.status),
        Some(StepStatus::Failed),
        "the ender keeps its failure"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "step `plan` attempt 1 cancelled: the run ended at `implement`"),
        "{notes:?}"
    );
    assert_eq!(orch.isolator().cleanups(), 1);
    assert_eq!(run_of(&orch, run).await.status, RunStatus::Failed);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 D3 (blueprint §3.6 step 4): a park and then an end. `plan` parks at its gate while
/// `implement`'s session is held; released, `implement` fails the run with nothing else in
/// flight. The run ended under a parked step, so the walk cancels `plan`'s row with a note naming
/// `implement` and answers the terminal rest, which wins over the park (A11).
async fn a_park_then_an_end_cancels_the_parked_step<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| match phase.name.as_str() {
        "plan" => phase.gate = Gate::Always,
        "implement" => phase.retry_limit = 0,
        _ => {}
    })
    .await;
    orch.script("implement", 1, ScriptedStep::done_without_output());
    let implement = orch.hold_sessions(&[("implement", 1)]);

    let release = async {
        within(implement.live(1), "implement live").await;
        let run = feat_3_run(&orch).await.expect("the run exists");
        within(
            until_steps(&orch, run, |steps| {
                latest_row(steps, 1).is_some_and(|step| step.status == StepStatus::AwaitingApproval)
            }),
            "plan parked",
        )
        .await;
        implement.release();
    };
    let ((run, rest), ()) = within(
        async { tokio::join!(start(&orch, ids::HTUI_FEAT_3), release) },
        "the end",
    )
    .await;
    assert_eq!(
        rest.run,
        RunStatus::Failed,
        "the end wins over the park: {rest:?}"
    );
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        latest_row(&steps, 1).map(|step| step.status),
        Some(StepStatus::Cancelled),
        "no step waits on a finished run"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "step `plan` attempt 1 cancelled: the run ended at `implement`"),
        "{notes:?}"
    );
    assert_eq!(orch.isolator().cleanups(), 1);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 A4: a terminal path inside one node's future leaves the cleanup to the walk while
/// a sibling is in flight. `plan`'s landing is held inside its reconcile when `implement`'s
/// session (held until then) fails the run: no cleanup yet. Once the landing answers, the walk
/// drains and cleans up, once.
async fn a_terminal_branch_defers_cleanup_until_its_sibling_settles<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    diamond_with_a_failing_implement(&orch).await;
    primary_repo(&orch).await;
    let implement = orch.hold_sessions(&[("implement", 1)]);
    // `prd`'s landing is the first reconcile, `plan`'s the second.
    let (reached, resume) = orch.isolator().pause_nth_reconcile(2);

    let meddle = async {
        within(reached.notified(), "plan's landing").await;
        implement.release();
        let run = feat_3_run(&orch).await.expect("the run exists");
        loop {
            if run_of(&orch, run).await.status == RunStatus::Failed {
                break;
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            orch.isolator().cleanups(),
            0,
            "the run failed with `plan` landing: its trees stay"
        );
        resume.notify_one();
    };
    let ((run, rest), ()) = within(
        async { tokio::join!(start(&orch, ids::HTUI_FEAT_3), meddle) },
        "the drain",
    )
    .await;
    assert_eq!(rest.run, RunStatus::Failed, "{rest:?}");
    assert_eq!(orch.isolator().cleanups(), 1, "once, when the walk drained");
    let log = call_log_of(&orch, run).await;
    assert_eq!(
        log.last().map(String::as_str),
        Some("cleanup run"),
        "{log:?}"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 A4 (blueprint §3.10, T3 repair 1): the cut half of the deferral. `implement` fails
/// the run while `plan`'s session is live; the walk's cut reaches `plan`, whose cancel is held. While
/// `plan` still drains, its row is `running` and nothing is cleaned up. Once it settles the walk
/// cancels its row and cleans up, once and last.
async fn a_terminal_branch_defers_cleanup_until_its_sibling_is_cut<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    diamond_with_a_failing_implement(&orch).await;
    let plan = orch.hold_sessions(&[("plan", 1)]);
    plan.hold_cancels();
    let implement = orch.hold_sessions(&[("implement", 1)]);

    let meddle = async {
        within(plan.live(1), "plan live").await;
        implement.release();
        within(plan.cancelled(1), "the cut reaches `plan`").await;
        let run = feat_3_run(&orch).await.expect("the run exists");
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(run_of(&orch, run).await.status, RunStatus::Failed);
        assert_eq!(
            latest_row(&steps_of(&orch, run).await, 1).map(|step| step.status),
            Some(StepStatus::Running),
            "`plan` is still draining its cut"
        );
        assert_eq!(
            orch.isolator().cleanups(),
            0,
            "the run failed with `plan` still in flight: its trees stay"
        );
        plan.release();
    };
    let ((run, rest), ()) = within(
        async { tokio::join!(start(&orch, ids::HTUI_FEAT_3), meddle) },
        "the cut",
    )
    .await;
    assert_eq!(rest.run, RunStatus::Failed, "{rest:?}");
    assert_eq!(
        latest_row(&steps_of(&orch, run).await, 1).map(|step| step.status),
        Some(StepStatus::Cancelled)
    );
    assert_eq!(orch.isolator().cleanups(), 1, "once, when the walk drained");
    let log = call_log_of(&orch, run).await;
    assert_eq!(
        log.last().map(String::as_str),
        Some("cleanup run"),
        "{log:?}"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 (blueprint §3.6 step 4, T3 repair 1): no row waits on a finished run, even when
/// nothing else is in flight at the end. On the diamond, `plan` (`shared_serialized`) and
/// `implement` (`worktree`) are both created; `implement`'s session is held back while `plan`'s
/// in-place session runs (A3), and `plan` then fails the run alone. `implement`'s `pending` row,
/// which a serial walk would never have created, is cancelled with a note naming `plan`.
async fn a_terminal_end_cancels_a_sibling_row_created_beside_it<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        phase.isolation = Some(if phase.name == "plan" {
            Isolation::SharedSerialized
        } else {
            Isolation::Worktree
        });
        if phase.name == "plan" {
            phase.retry_limit = 0;
        }
    })
    .await;
    primary_repo(&orch).await;
    orch.script("plan", 1, ScriptedStep::done_without_output());
    let plan = orch.hold_sessions(&[("plan", 1)]);

    let release = async {
        within(plan.live(1), "plan live").await;
        let run = feat_3_run(&orch).await.expect("the run exists");
        within(
            until_steps(&orch, run, |steps| latest_row(steps, 2).is_some()),
            "implement created",
        )
        .await;
        plan.release();
    };
    let ((run, rest), ()) = within(
        async { tokio::join!(start(&orch, ids::HTUI_FEAT_3), release) },
        "the end",
    )
    .await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::Failed, Some(1)),
        "{rest:?}"
    );
    let shape = sorted_shape(&orch, run).await;
    assert_eq!(
        shape,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Failed),
            (2, 1, StepStatus::Cancelled),
        ],
        "no row waits on a finished run"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "step `implement` attempt 1 cancelled: the run ended at `plan`"),
        "{notes:?}"
    );
    assert_eq!(orch.isolator().cleanups(), 1);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 (blueprint §3.6 step 4, T3 repair 2): a human's reject that ends the run is
/// answered outside any walk, so `settle_walk` never sees it; the answer's tail cancels the rows
/// the drain left waiting. On the diamond `plan` and `implement` both park at their gates; `plan`
/// is rejected and has no loop edge, so the run fails at `plan`, and `implement`'s parked row is
/// cancelled with a note naming `plan` rather than left waiting on a finished run.
async fn a_rejection_ending_the_run_cancels_the_other_parked_step<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
            (2, 1, StepStatus::AwaitingApproval),
        ],
        "both branches parked"
    );

    let rest = answer_at(
        &orch,
        run,
        1,
        GateAnswer::Rejected {
            note: "wrong plan".to_owned(),
        },
    )
    .await;
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (
            RunStatus::Failed,
            Some(1),
            Some(RunFailure::Rejected {
                phase: "plan".to_owned()
            })
        ),
        "{rest:?}"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::Failed);
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Failed),
            (2, 1, StepStatus::Cancelled),
        ],
        "no row waits on a finished run"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "step `implement` attempt 1 cancelled: the run ended at `plan`"),
        "{notes:?}"
    );
    assert_eq!(orch.isolator().cleanups(), 1);
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 (F2a): a loop on one branch while the other is live retires only its region.
/// `review → plan` is `changes_requested` on the chain `prd → plan → review`, beside the sink
/// `implement`, whose session is held live throughout; the loop re-runs `plan` and `review` and
/// never touches `implement`.
async fn a_loop_on_one_branch_retires_only_its_region<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let mut edges = CHAIN_AND_SINK.to_vec();
    edges.push(("review", "plan", EdgeKind::ChangesRequested, None, Some(1)));
    feat_3_graph(&orch, &edges, |_| {}).await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    let hold = orch.hold_sessions(&[("implement", 1)]);

    let release = async {
        within(hold.live(1), "implement live").await;
        let run = feat_3_run(&orch).await.expect("the run exists");
        within(
            until_steps(&orch, run, |steps| {
                steps.iter().any(|step| {
                    step.position == 3 && step.attempt == 2 && step.status == StepStatus::Done
                })
            }),
            "the loop's second review",
        )
        .await;
        hold.release();
    };
    let ((run, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let shape = sorted_shape(&orch, run).await;
    assert_eq!(
        shape
            .iter()
            .filter(|(position, _, _)| *position == 2)
            .collect::<Vec<_>>(),
        [&(2, 1, StepStatus::Done)],
        "`implement` ran once and was never retired: {shape:?}"
    );
    assert!(
        shape.contains(&(1, 2, StepStatus::Done)) && shape.contains(&(3, 2, StepStatus::Done)),
        "{shape:?}"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

/// MOD-138 M3 A2 (F2): on the chain `prd → plan → implement → review` with a back `asks_you`
/// edge `implement → plan`, approving `implement` owes the loop **and** takes `implement →
/// review`, so `ready` is `[Loop, Create(review)]`. `review` is comparable with `implement`, so
/// only the loop runs: `review` is never created while the loop retires `plan..implement`.
async fn a_loop_owed_beside_its_successor_runs_first<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(
        &orch,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            ("implement", "plan", EdgeKind::AsksYou, None, Some(1)),
        ],
        |_| {},
    )
    .await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(2)),
        "`implement` stops to ask"
    );
    let rest = answer_at(&orch, run, 2, GateAnswer::Approved).await;
    assert_eq!(
        (rest.run, rest.position),
        (RunStatus::AwaitingApproval, Some(2)),
        "the loop re-ran `plan` and `implement`, which stops again: {rest:?}"
    );
    let shape = sorted_shape(&orch, run).await;
    assert!(
        !shape.iter().any(|(position, _, _)| *position == 3),
        "`review` was never created beside the owed loop: {shape:?}"
    );
    assert!(shape.contains(&(1, 2, StepStatus::Done)), "{shape:?}");
    assert!(
        shape.contains(&(2, 2, StepStatus::AwaitingApproval)),
        "{shape:?}"
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
}

// -- MOD-138 M3 T4: recovery among concurrent landings (plan D6; blueprint A7-A10) -------------

/// MOD-138 M3 T4's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn concurrent_recovery_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_crash_with_two_unlanded_branches_lands_both_in_order" => Box::pin(
            a_crash_with_two_unlanded_branches_lands_both_in_order(harness),
        ),
        "a_crash_on_a_reordered_chain_lands_the_node_walked_last" => Box::pin(
            a_crash_on_a_reordered_chain_lands_the_node_walked_last(harness),
        ),
        "a_crash_beside_a_running_branch_lands_the_done_one" => {
            Box::pin(a_crash_beside_a_running_branch_lands_the_done_one(harness))
        }
        "a_landed_branch_without_a_successor_row_is_re_landed" => Box::pin(
            a_landed_branch_without_a_successor_row_is_re_landed(harness),
        ),
        "of_two_shared_serialized_landings_only_the_later_is_re_landed" => {
            Box::pin(of_two_shared_serialized_landings_only_the_later_is_re_landed(harness))
        }
        "a_crash_with_two_running_branches_adjudicates_both" => {
            Box::pin(a_crash_with_two_running_branches_adjudicates_both(harness))
        }
        "running_branches_recovered_into_parks_park_each" => {
            Box::pin(running_branches_recovered_into_parks_park_each(harness))
        }
        "a_re_run_loop_target_is_re_landed_beside_an_older_side_successor" => {
            Box::pin(a_re_run_loop_target_is_re_landed_beside_an_older_side_successor(harness))
        }
        "a_crash_cut_unpark_on_a_branching_run_lands_every_frontier" => Box::pin(
            a_crash_cut_unpark_on_a_branching_run_lands_every_frontier(harness),
        ),
        "answering_one_of_two_parks_lands_a_crashed_answers_winner" => Box::pin(
            answering_one_of_two_parks_lands_a_crashed_answers_winner(harness),
        ),
        "a_rejection_handed_back_cancels_the_other_parked_step" => Box::pin(
            a_rejection_handed_back_cancels_the_other_parked_step(harness),
        ),
        "a_looping_rejection_lands_a_crashed_answers_winner_first" => Box::pin(
            a_looping_rejection_lands_a_crashed_answers_winner_first(harness),
        ),
        "a_resumed_looping_rejection_lands_a_crashed_answers_winner_first" => {
            Box::pin(a_resumed_looping_rejection_lands_a_crashed_answers_winner_first(harness))
        }
        "a_looping_rejection_handed_back_walks_its_loop_as_in_process" => {
            Box::pin(a_looping_rejection_handed_back_walks_its_loop_as_in_process(harness))
        }
        _ => return None,
    })
}

/// A process killed mid-walk with several node futures stalled (MOD-138 M3 T4): `StartRun` on
/// `item` is polled until every signal in `stalled` fired, then dropped, and the run it left
/// `running` is handed back. A `within` bound, since a serial walk would never stall them all.
///
/// # Panics
/// When the walk finishes instead of stalling, or leaves no `running` run on the item.
async fn crash_on_all<O: Orchestrate>(orch: &O, item: ItemId, stalled: &[Arc<Notify>]) -> RunId {
    let every = async {
        for signal in stalled {
            signal.notified().await;
        }
    };
    let walk = orch.dispatch(Command::StartRun {
        item,
        mode: RunMode::Manual,
        repo_scope: None,
        workspace: None,
    });
    match within(
        futures::future::select(Box::pin(walk), Box::pin(every)),
        "every stall",
    )
    .await
    {
        Either::Left(_) => panic!("the walk finished instead of stalling"),
        Either::Right(((), walk)) => drop(walk),
    }
    orch.store()
        .runs(item)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|run| run.status == RunStatus::Running)
        .expect("the crash left the run `running`")
        .id
}

/// The adopter walks `run` on to `done`.
async fn walked_to_done<O: Orchestrate>(orch: &O, run: RunId) {
    let resumed = orch.resume(run).await.expect("the adopter walks it");
    assert!(
        matches!(
            resumed,
            Resume::Walked(Rest {
                run: RunStatus::Done,
                ..
            })
        ),
        "{resumed:?}"
    );
}

/// `orch.sweep()` adopts exactly `run`, and answers its `Next`.
async fn adopted_next<O: Orchestrate>(orch: &O, run: RunId) -> Next {
    let adopted = orch.sweep().await.expect("the sweep adopts");
    assert_eq!(adopted.len(), 1, "{adopted:?}");
    assert_eq!(adopted[0].run, run);
    adopted[0].next.clone()
}

/// The diamond with both branches `worktree`, so nothing serialises them (A3).
async fn worktree_diamond<O: Orchestrate>(orch: &O, mutate: impl Fn(&mut StepGraphPhase)) {
    feat_3_graph(orch, DIAMOND, |phase| {
        phase.isolation = Some(Isolation::Worktree);
        mutate(phase);
    })
    .await;
}

/// MOD-138 M3 D6 (Q-1): both branches of the diamond are `done` and neither merged when the
/// process dies (each landing stalled inside its reconcile). The adopter lands both, in
/// topological order, and the walk goes on to the join.
async fn a_crash_with_two_unlanded_branches_lands_both_in_order<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |_| {}).await;
    // `prd`'s landing is the first reconcile; the branches' are the second and third.
    let stalls = [
        orch.isolator().stall_nth_reconcile(2),
        orch.isolator().stall_nth_reconcile(3),
    ];
    let run = crash_on_all(&orch, ids::HTUI_FEAT_3, &stalls).await;
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran").clone();
    let implement = latest_row(&steps, 2).expect("`implement` ran").clone();
    assert_eq!(
        (plan.status, implement.status),
        (StepStatus::Done, StepStatus::Done)
    );
    assert!(latest_row(&steps, 3).is_none(), "the join waits");

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(plan.id, Vec::new()), (implement.id, Vec::new())],
        "both branches land, in topological order"
    );
    walked_to_done(&other, run).await;
    assert_eq!(
        latest_row(&steps_of(&other, run).await, 3).map(|step| step.status),
        Some(StepStatus::Done)
    );
}

/// MOD-138 M3 D6 (the M2 Q-C gap): on the chain `prd → implement → plan → review` the walk
/// finishes `plan` (position 1) after `implement` (position 2). A crash inside `plan`'s landing
/// is recovered by landing `plan`, the node walked last, not `implement`, the highest position.
async fn a_crash_on_a_reordered_chain_lands_the_node_walked_last<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(
        &orch,
        &[
            ("prd", "implement", EdgeKind::Next, None, None),
            ("implement", "plan", EdgeKind::Next, None, None),
            ("plan", "review", EdgeKind::Next, None, None),
        ],
        |_| {},
    )
    .await;
    // `prd`, `implement`, then `plan`: its landing is the third reconcile.
    let stalled = orch.isolator().stall_nth_reconcile(3);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran").clone();
    assert_eq!(plan.status, StepStatus::Done);
    assert_eq!(
        orch.isolator().reconciles().last(),
        Some(&(plan.id, Vec::new())),
        "the stalled landing is `plan`'s"
    );

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(plan.id, Vec::new())],
        "the node walked last lands, and only it"
    );
    walked_to_done(&other, run).await;
}

/// MOD-138 M3 D6 (A-3 per node): `plan` is `done` and unlanded while `implement`'s session is
/// still live when the process dies. The adopter lands `plan` and adjudicates `implement` as
/// ever (its document landed, so it is re-settled and lands too); the global "no running step"
/// guard no longer skips the done branch.
async fn a_crash_beside_a_running_branch_lands_the_done_one<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |_| {}).await;
    // `implement` never reaches its landing, so `plan`'s is the second reconcile.
    let stalls = [
        orch.isolator().stall_nth_reconcile(2),
        orch.stall_after_done("implement", 1, None, true),
    ];
    let run = crash_on_all(&orch, ids::HTUI_FEAT_3, &stalls).await;
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran").clone();
    let implement = latest_row(&steps, 2).expect("`implement` ran").clone();
    assert_eq!(
        (plan.status, implement.status),
        (StepStatus::Done, StepStatus::Running)
    );

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(plan.id, Vec::new()), (implement.id, Vec::new())],
        "the done branch lands first, then the adjudicated one"
    );
    assert_eq!(
        step_at(&other, run, 2, 1).await.status,
        StepStatus::Done,
        "D91: re-settled and passed"
    );
    walked_to_done(&other, run).await;
}

/// MOD-138 M3 D6 (Q-1, A8): a branch that already landed but whose successor has no row yet is
/// on the frontier, so recovery reconciles it again. `implement` lands before `plan`'s session
/// is let go; `plan`'s landing then stalls. The adopter lands `plan` and re-lands `implement`,
/// and the walk still reaches `done`. That the re-landing merges nothing is the real isolator's
/// (`merge_of`, H-3): the fake has no merge to count, so this case does not claim it.
async fn a_landed_branch_without_a_successor_row_is_re_landed<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |_| {}).await;
    primary_repo(&orch).await;
    let plan_session = orch.hold_sessions(&[("plan", 1)]);
    // `prd`, `implement`, then `plan`: its landing is the third reconcile.
    let stalled = [orch.isolator().stall_nth_reconcile(3)];
    let release = async {
        within(plan_session.live(1), "plan live").await;
        within(
            async {
                while orch.isolator().reconciles().len() < 2 {
                    tokio::task::yield_now().await;
                }
            },
            "implement landed",
        )
        .await;
        plan_session.release();
    };
    let (run, ()) = tokio::join!(crash_on_all(&orch, ids::HTUI_FEAT_3, &stalled), release);
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran").clone();
    let implement = latest_row(&steps, 2).expect("`implement` ran").clone();
    assert_eq!(
        orch.isolator()
            .reconciles()
            .iter()
            .filter(|(winner, _)| *winner == implement.id)
            .count(),
        1,
        "`implement` landed before the crash"
    );
    let landed = orch
        .store()
        .step_commits(implement.id)
        .await
        .expect("MemStore never fails a read");
    assert!(
        landed.iter().all(|row| row.after_hash.is_some()) && !landed.is_empty(),
        "{landed:?}"
    );

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(plan.id, Vec::new()), (implement.id, Vec::new())],
        "the landed sink-to-be is re-landed beside the unlanded one"
    );
    walked_to_done(&other, run).await;
}

/// MOD-138 M3 D6 (V5(e), A9): two `shared_serialized` branches land one after the other (A3).
/// A crash inside the second's landing re-lands only it: the first landed before the second
/// started, and re-landing it in place would refuse `primary_moved`.
async fn of_two_shared_serialized_landings_only_the_later_is_re_landed<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.isolation = Some(Isolation::SharedSerialized);
        }
    })
    .await;
    // A9's ties keep a member: each session ends a minute after it started.
    orch.advance_after_done(TimeDelta::minutes(1));
    // `prd`, `plan`, then `implement` (A3 serialises the in-place branches).
    let stalled = orch.isolator().stall_nth_reconcile(3);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let steps = steps_of(&orch, run).await;
    let implement = latest_row(&steps, 2).expect("`implement` ran").clone();
    assert_eq!(
        orch.isolator().reconciles().last(),
        Some(&(implement.id, Vec::new())),
        "the stalled landing is the later in-place branch's"
    );
    assert_eq!(
        latest_row(&steps, 1).map(|step| step.status),
        Some(StepStatus::Done)
    );

    let other = orch.restarted();
    assert_eq!(
        adopted_next(&other, run).await,
        Next::Walk,
        "no `primary_moved` park"
    );
    assert_eq!(
        other.isolator().reconciles(),
        [(implement.id, Vec::new())],
        "only the later in-place landing is re-landed"
    );
    walked_to_done(&other, run).await;
}

/// MOD-138 M3 A7 (F9): two sessions are live when the process dies. The adopter adjudicates
/// both: `plan` re-settles and parks at its gate, and that park no longer ends the recovery,
/// so `implement` re-settles and lands as well.
async fn a_crash_with_two_running_branches_adjudicates_both<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |phase| {
        if phase.name == "plan" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    let stalls = [
        orch.stall_after_done("plan", 1, None, true),
        orch.stall_after_done("implement", 1, None, true),
    ];
    let run = crash_on_all(&orch, ids::HTUI_FEAT_3, &stalls).await;
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Running),
            (2, 1, StepStatus::Running),
        ]
    );

    let other = orch.restarted();
    assert_eq!(
        adopted_next(&other, run).await,
        Next::Parked(Rest {
            run: RunStatus::AwaitingApproval,
            position: Some(1),
            failure: None,
        }),
        "the first park is the recovery's answer"
    );
    assert_eq!(
        sorted_shape(&other, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
            (2, 1, StepStatus::Done),
        ],
        "every running row adjudicated"
    );
    let implement = step_at(&other, run, 2, 1).await;
    assert_eq!(other.isolator().reconciles(), [(implement.id, Vec::new())]);
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );

    let rest = answer_at(&other, run, 1, GateAnswer::Approved).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
}

/// MOD-138 M3 A7 (T3 verify carry-over): three sessions are live when the process dies, and
/// each one's recovery parks. `plan` and `implement` wrote no document and have no retry left
/// (D94's park, twice through the engine), and `review` re-settles and parks at its gate on
/// the run already parked (the gate's park). The recovery's own parks never meet `StaleWrite`.
async fn running_branches_recovered_into_parks_park_each<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, FAN_OF_THREE, |phase| match phase.name.as_str() {
        "plan" | "implement" => phase.retry_limit = 0,
        "review" => phase.gate = Gate::Always,
        _ => {}
    })
    .await;
    let stalls = [
        orch.stall_after_done("plan", 1, None, false),
        orch.stall_after_done("implement", 1, None, false),
        orch.stall_after_done("review", 1, None, true),
    ];
    let run = crash_on_all(&orch, ids::HTUI_FEAT_3, &stalls).await;
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` ran").clone();
    let implement = latest_row(&steps, 2).expect("`implement` ran").clone();

    let other = orch.restarted();
    assert_eq!(
        adopted_next(&other, run).await,
        Next::Parked(Rest {
            run: RunStatus::AwaitingApproval,
            position: Some(1),
            failure: Some(RunFailure::Interrupted {
                phase: "plan".to_owned(),
                reset: true,
            }),
        }),
        "the first park is the recovery's answer"
    );
    assert_eq!(
        sorted_shape(&other, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Failed),
            (2, 1, StepStatus::Failed),
            (3, 1, StepStatus::AwaitingApproval),
        ],
        "every running row adjudicated into its park"
    );
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    for step in [&plan, &implement] {
        let spent = format!("interrupted: retry budget spent: step {}", step.id);
        assert!(
            notes.iter().any(|body| body.starts_with(&spent)),
            "{spent}\n{notes:?}"
        );
    }
}

/// MOD-138 M3 A8 (F10): a loop whose region holds `plan` but not its side successor
/// `implement` re-runs `plan` and leaves `implement`'s older row. On `prd → plan → {review,
/// implement}` with `review → plan` a `changes_requested` edge, `review` asks for changes once;
/// `plan` attempt 2 finishes after `implement` started, and the process dies inside its landing.
/// `implement`'s older row proves nothing, so the adopter re-lands `plan` attempt 2 (and the sink
/// `implement`, idempotently) rather than walking on over a tree without it.
async fn a_re_run_loop_target_is_re_landed_beside_an_older_side_successor<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    feat_3_graph(
        &orch,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("plan", "review", EdgeKind::Next, None, None),
            ("plan", "implement", EdgeKind::Next, None, None),
            ("review", "plan", EdgeKind::ChangesRequested, None, Some(1)),
        ],
        |phase| phase.isolation = Some(Isolation::Worktree),
    )
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    // A8 reads `started_at` against `finished_at`: every session ends a minute after it started.
    orch.advance_after_done(TimeDelta::minutes(1));
    // `prd`, `plan` 1 and `implement` land; `review` 1 is retired by its loop and lands nothing
    // (plan D135); `plan` attempt 2's landing is the fourth.
    let stalled = orch.isolator().stall_nth_reconcile(4);
    let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
    let steps = steps_of(&orch, run).await;
    let plan = at(&steps, 1, 2).clone();
    let implement = at(&steps, 2, 1).clone();
    assert_eq!(
        (plan.status, implement.status),
        (StepStatus::Done, StepStatus::Done)
    );
    assert_eq!(
        orch.isolator().reconciles().last(),
        Some(&(plan.id, Vec::new())),
        "the stalled landing is `plan` attempt 2's"
    );
    assert!(
        implement.started_at < plan.finished_at,
        "`implement`'s row is older than the re-run winner"
    );

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(plan.id, Vec::new()), (implement.id, Vec::new())],
        "the re-run winner lands; the sink re-lands, idempotently"
    );
    walked_to_done(&other, run).await;
    assert_eq!(step_at(&other, run, 3, 2).await.status, StepStatus::Done);
}

/// MOD-138 M3 D6 (`walk_resumed_from`): both branches of the diamond parked at their gates, and
/// a command answered each and crashed before its unpark (both rows `done`, the run still
/// parked, neither merged). The resume unparks the run and lands every frontier member, in
/// topological order, before the walk creates the join.
async fn a_crash_cut_unpark_on_a_branching_run_lands_every_frontier<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` parked").clone();
    let implement = latest_row(&steps, 2).expect("`implement` parked").clone();
    let now = orch.clock().now();
    for step in [&plan, &implement] {
        assert!(
            orch.store()
                .answer_gate(step.id, GateOutcome::Approved, None, now)
                .await
                .expect("MemStore takes the answer"),
            "a command's answer, its unpark cut off by a crash"
        );
    }
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    let landed = orch.isolator().reconciles().len();

    walked_to_done(&orch, run).await;
    let review = latest_row(&steps_of(&orch, run).await, 3)
        .expect("the join ran")
        .clone();
    assert_eq!(
        orch.isolator().reconciles()[landed..],
        [
            (plan.id, Vec::new()),
            (implement.id, Vec::new()),
            (review.id, Vec::new())
        ],
        "both branches land, in topological order, before the join"
    );
}

/// MOD-138 M3 A10 (F12): both branches of the diamond parked at their gates; a command answered
/// `plan` and crashed before its unpark (`plan` `done`, unmerged, the run still parked). A human
/// then answers `implement`: the answer lands `plan` first, then `implement`, and only then does
/// the walk create the join.
async fn answering_one_of_two_parks_lands_a_crashed_answers_winner<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    worktree_diamond(&orch, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");
    let steps = steps_of(&orch, run).await;
    let plan = latest_row(&steps, 1).expect("`plan` parked").clone();
    let implement = latest_row(&steps, 2).expect("`implement` parked").clone();
    assert!(
        orch.store()
            .answer_gate(plan.id, GateOutcome::Approved, None, orch.clock().now())
            .await
            .expect("MemStore takes the answer"),
        "a command's answer, its unpark cut off by a crash"
    );
    let landed = orch.isolator().reconciles().len();

    let rest = answer_at(&orch, run, 2, GateAnswer::Approved).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let review = latest_row(&steps_of(&orch, run).await, 3)
        .expect("the join ran")
        .clone();
    assert_eq!(
        orch.isolator().reconciles()[landed..],
        [
            (plan.id, Vec::new()),
            (implement.id, Vec::new()),
            (review.id, Vec::new())
        ],
        "the crashed answer's winner lands before the answered one, and both before the join"
    );
}

/// MOD-138 M3 (T3 repair 2's hand-back twin, MOD-41 plan D12): on a `worker` box the answer is
/// only written and handed back. `plan` and `implement` both parked on the diamond, `plan` is
/// rejected and has no loop edge: the adopter finishes the rejection as the in-process tail
/// does. The run fails at `plan`, and `implement`'s parked row is cancelled with a note naming
/// `plan` rather than parked again over a rejected sibling.
async fn a_rejection_handed_back_cancels_the_other_parked_step<H: CaseHarness>(harness: &H) {
    let a = harness.fresh();
    feat_3_graph(&a, DIAMOND, |phase| {
        if phase.name == "plan" || phase.name == "implement" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    let (run, rest) = start(&a, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");

    a.set_tails(Tails::HandBack);
    let rest = answer_at(
        &a,
        run,
        1,
        GateAnswer::Rejected {
            note: "wrong plan".to_owned(),
        },
    )
    .await;
    assert_handed_back(&a, run, &rest).await;

    let b = a.restarted();
    let Resume::Walked(rest) = adopted(&b, run).await else {
        panic!("the live graph did not move, so the adopter walks its snapshot");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.clone()),
        (
            RunStatus::Failed,
            Some(1),
            Some(RunFailure::Rejected {
                phase: "plan".to_owned()
            })
        ),
        "the adopter ends the run where the in-process tail does: {rest:?}"
    );
    assert_eq!(run_of(&b, run).await.status, RunStatus::Failed);
    assert_eq!(
        sorted_shape(&b, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Failed),
            (2, 1, StepStatus::Cancelled),
        ],
        "no row waits on a finished run"
    );
    let notes = notes_of(&b, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|body| body == "step `implement` attempt 1 cancelled: the run ended at `plan`"),
        "{notes:?}"
    );
    assert_eq!(b.isolator().cleanups(), 1);
}

/// The diamond with `plan` and `implement` both gated `Always` and a `fails` edge looping `plan`
/// back over `prd` once. `start` parks on both branches; answers the run.
async fn parked_on_a_looping_diamond<O: Orchestrate>(orch: &O) -> RunId {
    feat_3_graph(
        orch,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("prd", "implement", EdgeKind::Next, None, None),
            ("plan", "review", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            ("plan", "prd", EdgeKind::Fails, None, Some(1)),
        ],
        |phase| {
            if phase.name == "plan" || phase.name == "implement" {
                phase.gate = Gate::Always;
            }
        },
    )
    .await;
    let (run, rest) = start(orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");
    assert_eq!(
        sorted_shape(orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
            (2, 1, StepStatus::AwaitingApproval),
        ],
        "both branches parked"
    );
    run
}

/// MOD-138 M3 T4 repair 2, the looping twin of
/// [`a_rejection_handed_back_cancels_the_other_parked_step`]: `plan`'s rejection loops it over
/// `prd` while `implement` still waits. In process the answer's tail walks the loop target before
/// it re-parks (A6); handed back on a `worker` box, the adopter walks it too rather than
/// re-parking over the unwalked loop (D96). Both rest where they rest and write the same rows,
/// then and after `implement`'s approval.
async fn a_looping_rejection_handed_back_walks_its_loop_as_in_process<H: CaseHarness>(harness: &H) {
    let rejected = || GateAnswer::Rejected {
        note: "wrong plan".to_owned(),
    };
    let control = harness.fresh();
    let c_run = parked_on_a_looping_diamond(&control).await;
    let c_landed = control.isolator().reconciles().len();
    let expected = answer_at(&control, c_run, 1, rejected()).await;
    assert_eq!(
        (expected.run, expected.position),
        (RunStatus::AwaitingApproval, Some(1)),
        "the loop target walked, and the run re-parked on `implement`: {expected:?}"
    );
    assert_eq!(
        sorted_shape(&control, c_run).await,
        [
            (0, 1, StepStatus::Superseded),
            (0, 2, StepStatus::Done),
            (1, 1, StepStatus::Cancelled),
            (2, 1, StepStatus::AwaitingApproval),
        ],
        "the loop re-ran `prd` before the re-park"
    );

    let a = harness.fresh();
    let run = parked_on_a_looping_diamond(&a).await;
    a.set_tails(Tails::HandBack);
    let rest = answer_at(&a, run, 1, rejected()).await;
    assert_handed_back(&a, run, &rest).await;

    let b = a.restarted();
    let Resume::Walked(rest) = adopted(&b, run).await else {
        panic!("the live graph did not move, so the adopter walks its snapshot");
    };
    assert_eq!(
        rest, expected,
        "the adopter rests where the in-process tail does"
    );
    assert_eq!(
        sorted_shape(&b, run).await,
        sorted_shape(&control, c_run).await,
        "and writes the same rows: the loop target ran"
    );
    assert_eq!(
        b.isolator().reconciles().len(),
        control.isolator().reconciles().len() - c_landed,
        "the loop target landed once on each side"
    );
    assert_eq!(run_of(&b, run).await.status, RunStatus::AwaitingApproval);

    let expected = answer_at(&control, c_run, 2, GateAnswer::Approved).await;
    let rest = answer_at(&b, run, 2, GateAnswer::Approved).await;
    assert_eq!(rest, expected, "{rest:?}");
    assert_eq!(
        sorted_shape(&b, run).await,
        sorted_shape(&control, c_run).await,
        "the approval converges on the same rows"
    );
}

/// The diamond with every branch in its own worktree, `plan` gated `Always` and `implement`
/// `OnFailure`, its first attempt settling `failed` (no output) and a `fails` edge looping it
/// back over `prd` once. `start` parks on both branches; `plan`'s answer is then written and
/// its unpark cut off by a crash (`plan` `done`, unmerged, the run still parked). Returns the
/// run and `plan`'s row.
async fn crashed_approval_beside_a_looping_park<O: Orchestrate>(orch: &O) -> (RunId, RunStep) {
    feat_3_graph(
        orch,
        &[
            ("prd", "plan", EdgeKind::Next, None, None),
            ("prd", "implement", EdgeKind::Next, None, None),
            ("plan", "review", EdgeKind::Next, None, None),
            ("implement", "review", EdgeKind::Next, None, None),
            ("implement", "prd", EdgeKind::Fails, None, Some(1)),
        ],
        |phase| {
            phase.isolation = Some(Isolation::Worktree);
            if phase.name == "plan" {
                phase.gate = Gate::Always;
            }
            if phase.name == "implement" {
                phase.gate = Gate::OnFailure;
            }
        },
    )
    .await;
    orch.script("implement", 1, ScriptedStep::done_without_output());
    let (run, rest) = start(orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval, "{rest:?}");
    assert_eq!(
        sorted_shape(orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::AwaitingApproval),
            (2, 1, StepStatus::AwaitingApproval),
        ],
        "both branches parked"
    );
    let plan = latest_row(&steps_of(orch, run).await, 1)
        .expect("`plan` parked")
        .clone();
    assert!(
        orch.store()
            .answer_gate(plan.id, GateOutcome::Approved, None, orch.clock().now())
            .await
            .expect("MemStore takes the answer"),
        "a command's answer, its unpark cut off by a crash"
    );
    (run, plan)
}

/// MOD-138 M3 A10 (F12), T4 repair 1: the crashed approval's winner lands before any tail that
/// can walk, a looping rejection's included. `plan` was approved by a command that crashed
/// before its unpark; a human rejects `implement`, whose `fails` edge loops it over `prd`. The
/// answer lands `plan` first, so the join is created over a tree that has its work (else the
/// join's row would count as proof that `plan` landed, A8, and nothing would ever land it).
async fn a_looping_rejection_lands_a_crashed_answers_winner_first<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    let (run, plan) = crashed_approval_beside_a_looping_park(&orch).await;
    let landed = orch.isolator().reconciles().len();

    let rest = answer_at(
        &orch,
        run,
        2,
        GateAnswer::Rejected {
            note: "redo".to_owned(),
        },
    )
    .await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_landed_before_the_join(&orch, run, &plan, landed).await;
}

/// The resumed twin of [`a_looping_rejection_lands_a_crashed_answers_winner_first`]: both
/// answers were written and both unparks cut off. The resume lands `plan` before it finishes
/// the crashed rejection (plan D131, R-31), as `recover_run` does: frontiers first.
async fn a_resumed_looping_rejection_lands_a_crashed_answers_winner_first<H: CaseHarness>(
    harness: &H,
) {
    let orch = harness.fresh();
    let (run, plan) = crashed_approval_beside_a_looping_park(&orch).await;
    let implement = latest_row(&steps_of(&orch, run).await, 2)
        .expect("`implement` parked")
        .clone();
    assert!(
        orch.store()
            .answer_gate(
                implement.id,
                GateOutcome::Rejected,
                Some("redo".to_owned()),
                orch.clock().now()
            )
            .await
            .expect("MemStore takes the answer"),
        "the second command's rejection, its unpark cut off too"
    );
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    let landed = orch.isolator().reconciles().len();

    walked_to_done(&orch, run).await;
    assert_landed_before_the_join(&orch, run, &plan, landed).await;
}

/// The crashed approval's `plan` is the first reconcile after `landed`, the join's the last,
/// and `plan` is reconciled once.
async fn assert_landed_before_the_join<O: Orchestrate>(
    orch: &O,
    run: RunId,
    plan: &RunStep,
    landed: usize,
) {
    let review = latest_row(&steps_of(orch, run).await, 3)
        .expect("the join ran")
        .clone();
    let after = orch.isolator().reconciles()[landed..].to_vec();
    assert_eq!(
        after.first(),
        Some(&(plan.id, Vec::new())),
        "the crashed answer's winner lands first: {after:?}"
    );
    assert_eq!(
        after.last(),
        Some(&(review.id, Vec::new())),
        "the join lands last: {after:?}"
    );
    assert_eq!(
        after
            .iter()
            .filter(|(winner, _)| *winner == plan.id)
            .count(),
        1,
        "{after:?}"
    );
}

// -- MOD-138 M3 R1: review fixes (maintainer decisions, 2026-11-08) ----------------------------

/// MOD-138 M3 R1's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn review_fix_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_consumer_waits_for_an_incomparable_producer_of_its_input" => Box::pin(
            a_consumer_waits_for_an_incomparable_producer_of_its_input(harness),
        ),
        "a_lower_position_waits_for_a_higher_siblings_input" => {
            Box::pin(a_lower_position_waits_for_a_higher_siblings_input(harness))
        }
        "a_two_way_input_pair_runs_the_lower_position_first" => {
            Box::pin(a_two_way_input_pair_runs_the_lower_position_first(harness))
        }
        "a_cycle_of_input_waits_runs_its_lowest_position_first" => Box::pin(
            a_cycle_of_input_waits_runs_its_lowest_position_first(harness),
        ),
        "a_skipped_producer_leaves_its_kind_optional" => {
            Box::pin(a_skipped_producer_leaves_its_kind_optional(harness))
        }
        "a_rejection_handed_back_beside_an_interrupted_park_ends_the_run_as_in_process" => {
            Box::pin(
                a_rejection_handed_back_beside_an_interrupted_park_ends_the_run_as_in_process(
                    harness,
                ),
            )
        }
        _ => return None,
    })
}

/// Scripts each FEAT-3 phase in `phases` to finish with a document naming it and this case, so a
/// prompt that carries it read this run's document and not an older run's.
fn script_marked<O: Orchestrate>(orch: &O, phases: &[&str]) {
    for phase in phases {
        orch.script(phase, 1, ScriptedStep::done_with_output(&marked(phase)));
    }
}

/// The body [`script_marked`] gives `phase`'s document.
fn marked(phase: &str) -> String {
    format!("R1 `{phase}` of this run")
}

/// Holds `held`'s session until `waiting` is shown to have no row, then releases it: the case's
/// half of a `join` with the walk.
async fn hold_until_shown_waiting<O: Orchestrate>(
    orch: &O,
    hold: &SessionHold,
    held: &str,
    waiting: i32,
) {
    within(hold.live(1), held).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let run = feat_3_run(orch).await.expect("the run exists");
    let steps = steps_of(orch, run).await;
    assert!(
        latest_row(&steps, waiting).is_none(),
        "position {waiting} waits while `{held}` runs: {steps:?}"
    );
    hold.release();
}

/// Asserts that `log` lands `before` (a `position/attempt/index`) ahead of preparing `after`.
fn lands_before(log: &[String], before: &str, after: &str) {
    let first = |entry: &str| {
        log.iter()
            .position(|call| call == entry)
            .unwrap_or_else(|| panic!("{entry} in {log:?}"))
    };
    assert!(
        first(&format!("reconcile {before}")) < first(&format!("prepare {after}")),
        "{before} lands before {after} starts: {log:?}"
    );
}

/// MOD-138 M3 R1 ("order by inputs", review finding 1): the diamond `prd → {plan, implement} →
/// review` over FEAT-3's seeded `input_kinds`, kept: `implement` lists `plan`, which the
/// incomparable `plan` produces. A2 alone admitted both, and `implement`'s stage 3 failed the
/// run `missing_input`. Now `implement` waits (no row) while `plan`'s session is held, `plan`
/// lands before `implement` starts, every stage 3 reads this run's documents, and the run ends
/// `done`.
async fn a_consumer_waits_for_an_incomparable_producer_of_its_input<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |_| {}).await;
    author_edges(&orch, ids::HTUI_FEAT_3, DIAMOND).await;
    primary_repo(&orch).await;
    script_marked(&orch, &["prd", "plan", "implement"]);
    let plan = orch.hold_sessions(&[("plan", 1)]);

    let ((run, rest), ()) = tokio::join!(
        start(&orch, ids::HTUI_FEAT_3),
        hold_until_shown_waiting(&orch, &plan, "plan", 2)
    );
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    lands_before(&call_log_of(&orch, run).await, "1/1/0", "2/1/0");
    let steps = steps_of(&orch, run).await;
    for (position, reads) in [(1, "prd"), (2, "plan"), (3, "implement")] {
        let prompt = prompt_text(&orch, at(&steps, position, 1).id).await;
        assert!(
            prompt.contains(&marked(reads)),
            "position {position} read this run's `{reads}`:\n{prompt}"
        );
    }
}

/// MOD-138 M3 R1, the reversed direction: `plan` (position 1) lists the kind of its higher
/// sibling `implement` (2), so `plan` waits for `implement` and reads its document.
async fn a_lower_position_waits_for_a_higher_siblings_input<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| {
        if phase.name == "plan" {
            phase.input_kinds = vec!["implement".to_owned()];
        }
    })
    .await;
    primary_repo(&orch).await;
    script_marked(&orch, &["implement"]);
    let implement = orch.hold_sessions(&[("implement", 1)]);

    let ((run, rest), ()) = tokio::join!(
        start(&orch, ids::HTUI_FEAT_3),
        hold_until_shown_waiting(&orch, &implement, "implement", 1)
    );
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    lands_before(&call_log_of(&orch, run).await, "2/1/0", "1/1/0");
    let steps = steps_of(&orch, run).await;
    let prompt = prompt_text(&orch, at(&steps, 1, 1).id).await;
    assert!(prompt.contains(&marked("implement")), "{prompt}");
}

/// MOD-138 M3 R1, two-way: `plan` and `implement` each list the other's kind. The lower position
/// (`plan`) goes first, M2's order, with `implement`'s kind optional on its attempt; `implement`
/// waits for it and reads its document. Before R1 `plan` failed the run `missing_input`.
async fn a_two_way_input_pair_runs_the_lower_position_first<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, DIAMOND, |phase| match phase.name.as_str() {
        "plan" => phase.input_kinds = vec!["implement".to_owned()],
        "implement" => phase.input_kinds = vec!["plan".to_owned()],
        _ => {}
    })
    .await;
    primary_repo(&orch).await;
    script_marked(&orch, &["plan"]);
    let plan = orch.hold_sessions(&[("plan", 1)]);

    let ((run, rest), ()) = tokio::join!(
        start(&orch, ids::HTUI_FEAT_3),
        hold_until_shown_waiting(&orch, &plan, "plan", 2)
    );
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    lands_before(&call_log_of(&orch, run).await, "1/1/0", "2/1/0");
    let steps = steps_of(&orch, run).await;
    let prompt = prompt_text(&orch, at(&steps, 2, 1).id).await;
    assert!(prompt.contains(&marked("plan")), "{prompt}");
}

/// MOD-138 M3 R1, no deadlock: on `prd → {plan, implement, review}` `plan` lists `implement`'s
/// kind, `implement` `review`'s and `review` `plan`'s, a cycle of waits among incomparable nodes.
/// With nothing admissible and nothing in flight the lowest position (`plan`) runs alone, its
/// input optional; then `review` (it waited for `plan`), then `implement`. No graph is refused.
async fn a_cycle_of_input_waits_runs_its_lowest_position_first<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_graph(&orch, FAN_OF_THREE, |phase| match phase.name.as_str() {
        "plan" => phase.input_kinds = vec!["implement".to_owned()],
        "implement" => phase.input_kinds = vec!["review".to_owned()],
        "review" => phase.input_kinds = vec!["plan".to_owned()],
        _ => {}
    })
    .await;
    primary_repo(&orch).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    let log = call_log_of(&orch, run).await;
    lands_before(&log, "1/1/0", "3/1/0");
    lands_before(&log, "3/1/0", "2/1/0");
    assert_eq!(orch.live_sessions_high_water(), 1, "one at a time: {log:?}");
}

/// MOD-138 M3 R1 with D11: on the diamond the `if` takes `plan` and declines `implement`, and
/// `plan` lists `implement`'s kind. A skipped producer orders nothing and leaves its kind
/// optional, so `plan` runs and the run ends `done` with no `implement` row.
async fn a_skipped_producer_leaves_its_kind_optional<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        phase.input_kinds.clear();
        if phase.name == "plan" {
            phase.input_kinds = vec!["implement".to_owned()];
        }
    })
    .await;
    let prefix = item_of(&orch, ids::HTUI_FEAT_3).await.key_prefix;
    feat_3_diamond(&orch, &serde_json::json!({ "item_kind": prefix })).await;

    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(
        (rest.run, rest.failure.clone()),
        (RunStatus::Done, None),
        "{rest:?}"
    );
    assert_eq!(
        sorted_shape(&orch, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Done),
            (3, 1, StepStatus::Done),
        ],
        "`implement` is skipped and never gets a row"
    );
}

/// The diamond with `plan` out of retries and `implement` gated `always`, crashed with both
/// sessions live (each stalled after its `done`, only `implement` with its document) and adopted
/// by a second process: A7 parks the run over `plan`'s interrupted row (D94) and `implement`
/// re-settles into its gate's park. Returns the adopter and the run.
async fn parked_beside_an_interrupted_branch<O: Orchestrate>(orch: &O) -> (O, RunId) {
    feat_3_graph(orch, DIAMOND, |phase| match phase.name.as_str() {
        "plan" => phase.retry_limit = 0,
        "implement" => phase.gate = Gate::Always,
        _ => {}
    })
    .await;
    let stalls = [
        orch.stall_after_done("plan", 1, None, false),
        orch.stall_after_done("implement", 1, None, true),
    ];
    let run = crash_on_all(orch, ids::HTUI_FEAT_3, &stalls).await;
    let other = orch.restarted();
    assert!(
        matches!(
            adopted_next(&other, run).await,
            Next::Parked(Rest {
                failure: Some(RunFailure::Interrupted { .. }),
                position: Some(1),
                ..
            })
        ),
        "A7 parks over `plan`'s interrupted row"
    );
    assert_eq!(
        sorted_shape(&other, run).await,
        [
            (0, 1, StepStatus::Done),
            (1, 1, StepStatus::Failed),
            (2, 1, StepStatus::AwaitingApproval),
        ],
        "`implement` parked at its gate beside the interrupted `plan`"
    );
    (other, run)
}

/// MOD-138 M3 R1 (T4 leftover, review finding 6): the hand-back twin of a rejection beside an
/// A7 recovery park. `implement`'s gate is rejected (no loop edge) while `plan`'s interrupted row
/// keeps the run parked. In process the answer's tail fails the run at `implement`; handed back
/// on a `worker` box, the adopter's `recover_run` now finishes the rejection too, rather than
/// reading D131's lowest `failed` row (`plan`'s) and re-parking on it.
async fn a_rejection_handed_back_beside_an_interrupted_park_ends_the_run_as_in_process<
    H: CaseHarness,
>(
    harness: &H,
) {
    let rejected = || GateAnswer::Rejected {
        note: "wrong implementation".to_owned(),
    };
    let (control, c_run) = parked_beside_an_interrupted_branch(&harness.fresh()).await;
    let expected = answer_at(&control, c_run, 2, rejected()).await;
    assert_eq!(
        (expected.run, expected.position, expected.failure.clone()),
        (
            RunStatus::Failed,
            Some(2),
            Some(RunFailure::Rejected {
                phase: "implement".to_owned()
            })
        ),
        "in process the rejection ends the run"
    );

    let (a, run) = parked_beside_an_interrupted_branch(&harness.fresh()).await;
    a.set_tails(Tails::HandBack);
    let rest = answer_at(&a, run, 2, rejected()).await;
    assert_handed_back(&a, run, &rest).await;

    let b = a.restarted();
    let Resume::Walked(rest) = adopted(&b, run).await else {
        panic!("the live graph did not move, so the adopter walks its snapshot");
    };
    assert_eq!(
        rest, expected,
        "the adopter ends the run where the in-process tail does"
    );
    assert_eq!(run_of(&b, run).await.status, RunStatus::Failed);
    assert_eq!(
        sorted_shape(&b, run).await,
        sorted_shape(&control, c_run).await,
        "and writes the same rows"
    );
}

// -- MOD-139 M2 T3: divided nodes (plan D1-D5, D7, D8, D10, D13, D15-D17) ----------------------

/// MOD-139 M2 T3's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn divided_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_divided_node_admits_its_default_parts_as_revision_zero" => Box::pin(
            a_divided_node_admits_its_default_parts_as_revision_zero(harness),
        ),
        "each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent" => {
            Box::pin(each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent(harness))
        }
        "part_documents_stay_out_of_input_resolution" => {
            Box::pin(part_documents_stay_out_of_input_resolution(harness))
        }
        "integrate_is_offered_only_once_every_part_is_done" => {
            Box::pin(integrate_is_offered_only_once_every_part_is_done(harness))
        }
        "two_parts_are_live_at_once_under_the_cap" => {
            Box::pin(two_parts_are_live_at_once_under_the_cap(harness))
        }
        "max_parallel_sessions_one_runs_the_parts_one_at_a_time" => Box::pin(
            max_parallel_sessions_one_runs_the_parts_one_at_a_time(harness),
        ),
        "shared_serialized_parts_reset_to_the_base_refs" => {
            Box::pin(shared_serialized_parts_reset_to_the_base_refs(harness))
        }
        "a_single_local_part_prepares_with_no_slot" => {
            Box::pin(a_single_local_part_prepares_with_no_slot(harness))
        }
        "local_with_two_parts_fails_admission" => {
            Box::pin(local_with_two_parts_fails_admission(harness))
        }
        "a_part_agent_outside_the_candidates_fails_admission" => {
            Box::pin(a_part_agent_outside_the_candidates_fails_admission(harness))
        }
        "a_part_persona_fails_admission" => Box::pin(a_part_persona_fails_admission(harness)),
        "an_over_cap_plan_fails_admission" => Box::pin(an_over_cap_plan_fails_admission(harness)),
        "an_empty_divided_node_fails_admission" => {
            Box::pin(an_empty_divided_node_fails_admission(harness))
        }
        "a_part_with_a_live_sibling_gets_command_run" => {
            Box::pin(a_part_with_a_live_sibling_gets_command_run(harness))
        }
        "a_fan_out_only_branch_beside_a_live_sibling_gets_command_run" => {
            Box::pin(a_fan_out_only_branch_beside_a_live_sibling_gets_command_run(harness))
        }
        "the_operator_verbs_refuse_a_part_row" => {
            Box::pin(the_operator_verbs_refuse_a_part_row(harness))
        }
        "stage_rows_are_ignored_by_the_walk" => {
            Box::pin(stage_rows_are_ignored_by_the_walk(harness))
        }
        "start_run_refuses_a_divided_graph_and_first_a_planner_agent" => Box::pin(
            start_run_refuses_a_divided_graph_and_first_a_planner_agent(harness),
        ),
        _ => return None,
    })
}

/// FEAT-3 ungated with every phase's `input_kinds` cleared, and `implement` divided (`split =
/// divide`, `fan_out` 1, `mutate` applied) into `parts`, each `(key, agent)` with the title `The
/// <key>`, the brief `Build the <key> part.`, the owned path `src/<key>/**` and the acceptance
/// `the <key> tests pass`.
///
/// # Panics
/// When the store refuses the edited graph.
async fn divided_feat_3<O: Orchestrate>(
    orch: &O,
    parts: &[(&str, Option<AgentId>)],
    mutate: impl Fn(&mut StepGraphPhase),
) {
    feat_3_gated(orch, Gate::Never, |phase| phase.input_kinds.clear()).await;
    edit_graph(orch, ids::HTUI_FEAT_3, |body| {
        let node = body
            .nodes
            .iter_mut()
            .find(|node| node.phase.name == "implement")
            .expect("FEAT-3's graph has `implement`");
        node.phase.split = htui_core::model::Split::Divide;
        node.phase.fan_out = 1;
        mutate(&mut node.phase);
        let phase = node.phase.id;
        node.parts = parts
            .iter()
            .enumerate()
            .map(|(position, (key, agent))| htui_core::model::PhasePart {
                phase_id: phase,
                position: i32::try_from(position).expect("a handful of parts"),
                key: (*key).to_owned(),
                title: format!("The {key}"),
                brief: format!("Build the {key} part."),
                owned_paths: vec![format!("src/{key}/**")],
                acceptance: Some(format!("the {key} tests pass")),
                agent_id: *agent,
                model: None,
                persona_id: None,
            })
            .collect();
    })
    .await;
}

/// The divided FEAT-3 run through the D16 seam, then claimed: its id and the claim's answer.
///
/// # Panics
/// When the seam refuses the graph.
async fn start_divided<O: Orchestrate>(orch: &O) -> (RunId, Result<CommandOutcome, EngineError>) {
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    (run, orch.claim(run).await)
}

/// MOD-139 M2 D15 (F-2): an M2 divided run ends at its integrate node with the existing divide
/// refusal, named by the divided node.
fn stops_at_integrate<T: core::fmt::Debug>(result: &Result<T, EngineError>) {
    assert!(
        matches!(
            result,
            Err(EngineError::Resolve(crate::graph::ResolveError::NotYetRunnable {
                node,
                reason: crate::graph::NotRunnable::Divide,
            })) if node == "implement"
        ),
        "the walk stops at the integrate node: {result:?}"
    );
}

/// The rows under `planner`, in `(position, attempt)` order.
fn parts_of(steps: &[RunStep], planner: StepId) -> Vec<RunStep> {
    let mut parts: Vec<RunStep> = steps
        .iter()
        .filter(|step| step.parent_step_id == Some(planner))
        .cloned()
        .collect();
    parts.sort_by_key(|step| (step.position, step.attempt));
    parts
}

/// The latest planner row.
///
/// # Panics
/// When the run holds none.
fn planner_of(steps: &[RunStep]) -> RunStep {
    steps
        .iter()
        .filter(|step| step.role == StepRole::Planner)
        .max_by_key(|step| step.attempt)
        .cloned()
        .expect("the walk created the planner row")
}

/// The latest part row named `phase_name`.
///
/// # Panics
/// When the run holds none.
fn part_named<'s>(steps: &'s [RunStep], phase_name: &str) -> &'s RunStep {
    steps
        .iter()
        .filter(|step| step.role == StepRole::Part && step.phase_name == phase_name)
        .max_by_key(|step| step.attempt)
        .unwrap_or_else(|| panic!("the run holds the part `{phase_name}`"))
}

/// The run's plan revisions.
async fn plans_of<O: Orchestrate>(orch: &O, run: RunId) -> Vec<htui_core::model::RunPlan> {
    orch.store()
        .run_plans(run)
        .await
        .expect("MemStore never fails a read")
}

/// The admitted plan's `base_refs` entry for `repo`.
///
/// # Panics
/// When the run has no admitted plan or the plan names no such repo.
async fn base_ref_of<O: Orchestrate>(orch: &O, run: RunId, repo: RepoId) -> String {
    plans_of(orch, run)
        .await
        .into_iter()
        .find(|plan| plan.status == htui_core::model::PlanStatus::Admitted)
        .and_then(|plan| plan.base_refs)
        .and_then(|refs| refs.get(repo.to_string()).cloned())
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("the admitted plan names the repo's base")
}

/// The trees a step recorded.
async fn trees_of<O: Orchestrate>(orch: &O, step: StepId) -> Vec<htui_core::model::RunStepTree> {
    orch.store()
        .step_trees(step)
        .await
        .expect("MemStore never fails a read")
}

/// MOD-139 M2 D2, D9: when the planner node is ready, the engine admits the node's default parts
/// as plan revision 0 in one store write — the plan `admitted` with `base_refs`, the part rows
/// `pending` past the frozen node list, the planner `done` — and both parts then run. The walk
/// stops at the integrate node (D15), the next cursor once both parts are done.
async fn a_divided_node_admits_its_default_parts_as_revision_zero<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    let repo = primary_repo(&orch).await;

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!(
        (
            planner.position,
            planner.role,
            planner.node_key.as_deref(),
            planner.status
        ),
        (
            2,
            StepRole::Planner,
            Some("implement:plan"),
            StepStatus::Done
        )
    );
    assert!(planner.finished_at.is_some(), "{planner:?}");

    let plans = plans_of(&orch, run).await;
    assert_eq!(plans.len(), 1, "{plans:?}");
    let plan = &plans[0];
    assert_eq!(
        (
            plan.revision,
            plan.status,
            plan.base_position,
            plan.planner_step_id
        ),
        (0, htui_core::model::PlanStatus::Admitted, 5, planner.id)
    );
    let mut base = serde_json::Map::new();
    base.insert(
        repo.to_string(),
        serde_json::Value::String(format!("fake:group-base:{repo}")),
    );
    assert_eq!(plan.base_refs, Some(serde_json::Value::Object(base)));
    let snapshot = snapshot_of(&orch, run).await;
    let defaults = snapshot.phases[2]
        .divide
        .as_ref()
        .expect("the planner node carries its divide block")
        .default_parts
        .clone();
    let body = crate::divide::plan_body(&defaults);
    assert_eq!(plan.body, body);
    assert_eq!(plan.digest, crate::divide::plan_digest(&body));

    let shape: Vec<_> = parts_of(&steps, planner.id)
        .into_iter()
        .map(|part| {
            (
                part.position,
                part.phase_name.clone(),
                part.node_key.clone(),
                part.role,
                part.attempt,
                part.fanout_index,
                part.status,
            )
        })
        .collect();
    let part = |position: i32, name: &str| {
        (
            position,
            name.to_owned(),
            Some(name.to_owned()),
            StepRole::Part,
            1,
            0,
            StepStatus::Done,
        )
    };
    assert_eq!(shape, [part(5, "implement/api"), part(6, "implement/ui")]);
    assert!(
        steps.iter().all(|step| step.position != 3),
        "integrate waits for M3"
    );
    assert_eq!(
        crate::walk::ready(&snapshot, &steps),
        [crate::status::Cursor::Create {
            position: 3,
            attempt: 1
        }]
    );
}

/// MOD-139 M2 D4, D5, D10: each part runs in its own tree branched from the plan's `base_refs`,
/// under its own agent, with its own prompt (the internal `part` section) and its own session;
/// a part is never reconciled into the primary tree.
async fn each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    orch.with_candidates(
        "implement",
        vec![
            (ids::AGENT_CLAUDE, "sonnet"),
            (ids::AGENT_AGY, "gemini-3.7-flash-high"),
        ],
    );
    divided_feat_3(
        &orch,
        &[("api", None), ("ui", Some(ids::AGENT_AGY))],
        |_| {},
    )
    .await;
    let repo = primary_repo(&orch).await;

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let api = part_named(&steps, "implement/api").clone();
    let ui = part_named(&steps, "implement/ui").clone();
    let base = base_ref_of(&orch, run, repo).await;
    let (api_trees, ui_trees) = (trees_of(&orch, api.id).await, trees_of(&orch, ui.id).await);
    for trees in [&api_trees, &ui_trees] {
        assert_eq!(trees.len(), 1, "{trees:?}");
        assert_eq!(trees[0].base_ref, base, "a part branches from base_refs");
    }
    assert_ne!(api_trees[0].path, ui_trees[0].path, "one tree per part");

    let log = call_log_of(&orch, run).await;
    for entry in [
        "prepare 5/1/0",
        "capture 5/1/0",
        "prepare 6/1/0",
        "capture 6/1/0",
    ] {
        assert!(log.iter().any(|call| call == entry), "{entry}: {log:?}");
    }
    assert!(
        !log.iter()
            .any(|call| call.starts_with("reconcile 5/") || call.starts_with("reconcile 6/")),
        "a part never lands: {log:?}"
    );

    assert_eq!(api.agent_id, Some(ids::AGENT_CLAUDE));
    assert_eq!(
        (ui.agent_id, ui.model.as_deref()),
        (Some(ids::AGENT_AGY), Some("gemini-3.7-flash-high"))
    );
    for part in [&api, &ui] {
        let sections = prompt_sections(&orch, part.id).await;
        assert_eq!(
            sections.get(1).map(String::as_str),
            Some("part"),
            "{sections:?}"
        );
    }
    let text = prompt_text(&orch, api.id).await;
    assert!(text.contains("key=\"api\""), "{text}");
    assert!(text.contains("Build the api part."), "{text}");
    assert!(!text.contains("Build the ui part."), "{text}");
    assert_ne!(api.prompt_digest, ui.prompt_digest);
    spec_of(&orch, &session("implement/api", 0));
    spec_of(&orch, &session("implement/ui", 0));
}

/// MOD-139 M2 D12: both parts write the divided node's `output_kind`, and neither document is a
/// generic input: with no hand-written one, the kind resolves to nothing.
async fn part_documents_stay_out_of_input_resolution<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let mut parts: Vec<StepId> = parts_of(&steps, planner_of(&steps).id)
        .iter()
        .map(|part| part.id)
        .collect();
    parts.sort();
    let mut producers: Vec<StepId> = orch
        .store()
        .documents(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .filter(|head| head.kind == "implement")
        .filter_map(|head| head.produced_by_step_id)
        .collect();
    producers.sort();
    assert_eq!(producers, parts, "each part wrote an `implement` document");
    let resolved = orch
        .store()
        .resolve_inputs(ids::HTUI_FEAT_3, run, &["implement".to_owned()])
        .await
        .expect("MemStore never fails a read");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].kind, "implement");
    assert_eq!(resolved[0].document, None, "a part document is no input");
}

/// MOD-139 M2 D3: while a part is live it rests its node and nothing downstream is created; once
/// every part is done the integrate node is the next cursor.
async fn integrate_is_offered_only_once_every_part_is_done<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    let hold = orch.hold_sessions(&[("implement/ui", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");

    let check = async {
        until_steps(&orch, run, |steps| {
            steps
                .iter()
                .any(|step| step.phase_name == "implement/api" && step.status == StepStatus::Done)
        })
        .await;
        within(hold.live(1), "`ui` held").await;
        let steps = steps_of(&orch, run).await;
        let snapshot = snapshot_of(&orch, run).await;
        let ui = part_named(&steps, "implement/ui");
        assert_eq!(
            crate::walk::ready(&snapshot, &steps),
            [crate::status::Cursor::Rest {
                step: ui.id,
                status: StepStatus::Running
            }],
            "a live part rests its node"
        );
        assert!(steps.iter().all(|step| step.position != 3), "{steps:?}");
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), check);
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        crate::walk::ready(&snapshot_of(&orch, run).await, &steps),
        [crate::status::Cursor::Create {
            position: 3,
            attempt: 1
        }]
    );
}

/// MOD-139 M2 D4: a round runs its parts at once up to the session cap.
async fn two_parts_are_live_at_once_under_the_cap<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    let hold = orch.hold_sessions(&[("implement/api", 1), ("implement/ui", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let release = async {
        within(hold.live(2), "both parts live").await;
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), release);
    stops_at_integrate(&result);
    assert_eq!(orch.live_sessions_high_water(), 2);
    let steps = steps_of(&orch, run).await;
    for name in ["implement/api", "implement/ui"] {
        assert_eq!(part_named(&steps, name).status, StepStatus::Done, "{name}");
    }
}

/// MOD-139 M2 D4: `max_parallel_sessions` 1 runs the parts one at a time.
async fn max_parallel_sessions_one_runs_the_parts_one_at_a_time<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(1));
    let hold = orch.hold_sessions(&[("implement/api", 1), ("implement/ui", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let release = async {
        within(hold.live(1), "one part live").await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(hold.reached(), 1, "the cap holds the second part back");
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), release);
    stops_at_integrate(&result);
    assert_eq!(orch.live_sessions_high_water(), 1);
    let steps = steps_of(&orch, run).await;
    for name in ["implement/api", "implement/ui"] {
        assert_eq!(part_named(&steps, name).status, StepStatus::Done, "{name}");
    }
}

/// MOD-139 M2 D4, D7: under `shared_serialized` the parts run one at a time, each prepared from
/// the plan's `base_refs`, and the primary checkout is reset to them after the last part.
async fn shared_serialized_parts_reset_to_the_base_refs<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.isolation = Some(Isolation::SharedSerialized);
    })
    .await;
    let repo = primary_repo(&orch).await;

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let base = base_ref_of(&orch, run, repo).await;
    for name in ["implement/api", "implement/ui"] {
        let trees = trees_of(&orch, part_named(&steps, name).id).await;
        assert_eq!(trees[0].base_ref, base, "{name}");
    }
    let parts: Vec<String> = call_log_of(&orch, run)
        .await
        .into_iter()
        .filter(|call| call.contains(" 5/") || call.contains(" 6/"))
        .collect();
    assert_eq!(
        parts,
        [
            "prepare 5/1/0",
            "capture 5/1/0",
            "prepare 6/1/0",
            "capture 6/1/0",
            "reset 6/1/0",
        ]
    );
    assert_eq!(orch.isolator().in_place_overlaps(), 0);
    assert_eq!(orch.live_sessions_high_water(), 1);
}

/// MOD-139 M2 D4, D7: one part under `local` runs in place with no fan-out slot (the real
/// isolator refuses one there), so its tree is a fresh base, not the plan's.
async fn a_single_local_part_prepares_with_no_slot<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None)], |phase| {
        phase.isolation = Some(Isolation::Local);
    })
    .await;
    let repo = primary_repo(&orch).await;

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let api = part_named(&steps, "implement/api");
    assert_eq!(api.status, StepStatus::Done);
    let trees = trees_of(&orch, api.id).await;
    let base = base_ref_of(&orch, run, repo).await;
    assert!(
        trees[0].base_ref.starts_with("fake:base:"),
        "{}",
        trees[0].base_ref
    );
    assert_ne!(trees[0].base_ref, base, "no slot, so no plan base");
}

/// MOD-139 M2 D2, P-8: an admission refusal fails the planner step, writes no plan and no part,
/// blocks the item with the sentence as its last note and fails the run on it.
async fn assert_admission_refused<O: Orchestrate>(orch: &O, failure: &RunFailure) {
    let (run, result) = start_divided(orch).await;
    let outcome = result.expect("an admission refusal is a rest, not an error");
    let CommandOutcome::Started { rest, .. } = outcome else {
        panic!("`claim` answers `Started`, not {outcome:?}");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(2), Some(failure))
    );
    assert_eq!(run_of(orch, run).await.failure, Some(failure.to_string()));
    let steps = steps_of(orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!(planner.status, StepStatus::Failed);
    assert!(parts_of(&steps, planner.id).is_empty(), "{steps:?}");
    assert!(plans_of(orch, run).await.is_empty());
    assert_eq!(
        item_of(orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    assert_eq!(
        notes_of(orch, ids::HTUI_FEAT_3).await.last(),
        Some(&failure.to_string())
    );
}

/// MOD-139 M2 D7: `local` runs one tree, so two parts fail admission.
async fn local_with_two_parts_fails_admission<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.isolation = Some(Isolation::Local);
    })
    .await;
    assert_admission_refused(
        &orch,
        &RunFailure::LocalCannotDivide {
            node: "implement".to_owned(),
            parts: 2,
        },
    )
    .await;
}

/// MOD-139 M2 D5: a part's agent must be one of the node's candidates; no silent fallback.
async fn a_part_agent_outside_the_candidates_fails_admission<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(
        &orch,
        &[("api", None), ("ui", Some(ids::AGENT_AGY))],
        |_| {},
    )
    .await;
    assert_admission_refused(
        &orch,
        &RunFailure::PartAgentNotCandidate {
            node: "implement".to_owned(),
            part: "ui".to_owned(),
            agent: ids::AGENT_AGY,
        },
    )
    .await;
}

/// MOD-139 M2 D5: per-part personas are MOD-140's; a part naming one fails admission.
async fn a_part_persona_fails_admission<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    edit_graph(&orch, ids::HTUI_FEAT_3, |body| {
        let node = body
            .nodes
            .iter_mut()
            .find(|node| node.phase.name == "implement")
            .expect("FEAT-3's graph has `implement`");
        node.parts[1].persona_id = Some(ids::PERSONA_ARCHITECT);
    })
    .await;
    assert_admission_refused(
        &orch,
        &RunFailure::PartPersonaUnsupported {
            node: "implement".to_owned(),
            part: "ui".to_owned(),
        },
    )
    .await;
}

/// MOD-139 M2 D8: a plan over the node's `max_parts` fails admission.
async fn an_over_cap_plan_fails_admission<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.max_parts = Some(1);
    })
    .await;
    assert_admission_refused(
        &orch,
        &RunFailure::PlanOverCaps {
            node: "implement".to_owned(),
            cap: crate::status::PlanCap::MaxParts,
            planned: 2,
            max: 1,
        },
    )
    .await;
}

/// MOD-139 M2 D2 (ANA-33 §7.6): a divided node with no default part fails admission.
async fn an_empty_divided_node_fails_admission<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[], |_| {}).await;
    assert_admission_refused(
        &orch,
        &RunFailure::EmptyPlan {
            node: "implement".to_owned(),
        },
    )
    .await;
}

/// MOD-139 M2 D13: a `fan_out_only` part beside a live sibling part gets `command_run`, the
/// denials and the section; a lone plain step of the same run does not. Both parts are held until
/// both are live, so each one's prompt is assembled beside a live sibling whatever the poll order.
async fn a_part_with_a_live_sibling_gets_command_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.command_queue = CommandQueue::FanOutOnly;
    })
    .await;
    let hold = orch.hold_sessions(&[("implement/api", 1), ("implement/ui", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let release = async {
        within(hold.live(2), "both parts live").await;
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), release);
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    for name in ["implement/api", "implement/ui"] {
        let spec = spec_of(&orch, &session(name, 0));
        assert!(spec.tools.command_run, "{name} gets `command_run`");
        assert!(has_denials(&spec), "{name} gets the denials");
        let sections = prompt_sections(&orch, part_named(&steps, name).id).await;
        assert!(
            sections.iter().any(|section| section == "command_queue"),
            "{name}: {sections:?}"
        );
    }
    let prd = spec_of(&orch, &session("prd", 0));
    assert!(!prd.tools.command_run, "a lone plain step");
    assert!(!has_denials(&prd));
}

/// MOD-139 M2 D13 (blueprint G-9): the deliberate change — a `fan_out_only` step of a parallel
/// branch gets `command_run` while the other branch is live; the root `prd`, alone, does not.
async fn a_fan_out_only_branch_beside_a_live_sibling_gets_command_run<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh().hosted();
    feat_3_graph(&orch, DIAMOND, |phase| {
        phase.command_queue = CommandQueue::FanOutOnly;
    })
    .await;
    let hold = orch.hold_sessions(&[("plan", 1), ("implement", 1)]);
    let release = async {
        within(hold.live(2), "both branches live").await;
        hold.release();
    };
    let ((_, rest), ()) = tokio::join!(start(&orch, ids::HTUI_FEAT_3), release);
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    for phase in ["plan", "implement"] {
        let spec = spec_of(&orch, &session(phase, 0));
        assert!(spec.tools.command_run, "{phase} beside a live branch");
        assert!(has_denials(&spec), "{phase} gets the denials");
    }
    let prd = spec_of(&orch, &session("prd", 0));
    assert!(!prd.tools.command_run, "the root runs alone");
    assert!(!has_denials(&prd));
}

/// MOD-139 M2 D1: the four row-taking operator verbs refuse a part row in `operator_row`'s
/// sentence, and nothing moves.
async fn the_operator_verbs_refuse_a_part_row<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let before = steps_of(&orch, run).await;
    let api = part_named(&before, "implement/api").id;
    for command in [
        Command::AnswerGate {
            run,
            step: api,
            answer: GateAnswer::Approved,
        },
        Command::RetryStep { run, step: api },
        Command::PromoteStep {
            run,
            step: api,
            chat_open: false,
        },
        Command::AcceptArtifact {
            run,
            step: api,
            chat_live: false,
        },
    ] {
        let refused = orch
            .dispatch(command.clone())
            .await
            .expect_err("a part is driven by its divided node");
        assert!(
            matches!(
                refused,
                EngineError::RoleRow {
                    step,
                    role: StepRole::Part
                } if step == api
            ),
            "{command:?}: {refused:?}"
        );
        assert_eq!(
            refused.to_string(),
            format!("step {api} is a part; a part is driven by its divided node")
        );
    }
    assert_eq!(steps_of(&orch, run).await, before, "no row moved");
}

/// MOD-139 M2 (C3): htui's stage rows (negative positions) are invisible to the walk: a run
/// holding three finishes over them, and they stay `pending`.
async fn stage_rows_are_ignored_by_the_walk<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    feat_3_gated(&orch, Gate::Never, |phase| {
        if phase.name == "prd" {
            phase.gate = Gate::Always;
        }
    })
    .await;
    let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
    assert_eq!(rest.run, RunStatus::AwaitingApproval);
    let mut stages = Vec::new();
    for (position, key) in [(-3, "pick"), (-2, "prepare"), (-1, "collect")] {
        let stage = orch
            .store()
            .create_step(htui_core::model::NewRunStep {
                id: StepId::new(),
                run_id: run,
                position,
                attempt: 1,
                fanout_index: 0,
                phase_name: "stage".to_owned(),
                agent_id: None,
                model: None,
                role: StepRole::Stage,
                parent_step_id: None,
                node_key: Some(key.to_owned()),
            })
            .await
            .expect("a stage row is a row of the run");
        stages.push(stage.id);
    }
    let rest = answer_at(&orch, run, 0, GateAnswer::Approved).await;
    assert_eq!((rest.run, rest.failure), (RunStatus::Done, None));
    let steps = steps_of(&orch, run).await;
    for stage in &stages {
        let row = steps
            .iter()
            .find(|step| step.id == *stage)
            .expect("the stage row is there");
        assert_eq!(row.status, StepStatus::Pending, "{row:?}");
    }
    assert_eq!(
        crate::walk::ready(&snapshot_of(&orch, run).await, &steps),
        [crate::status::Cursor::Finished]
    );
}

/// MOD-139 M2 D16, D17: production resolve (`StartRun`) refuses a divided graph, and first a
/// configured planner agent, which the seam refuses too. No run is created.
async fn start_run_refuses_a_divided_graph_and_first_a_planner_agent<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    let start = || {
        orch.dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: None,
            workspace: None,
        })
    };
    fn refused<T>(result: &Result<T, EngineError>, reason: crate::graph::NotRunnable) -> bool {
        matches!(
            result,
            Err(EngineError::Resolve(crate::graph::ResolveError::NotYetRunnable {
                node,
                reason: found,
            })) if node == "implement" && *found == reason
        )
    }
    let divide = start().await;
    assert!(
        refused(&divide, crate::graph::NotRunnable::Divide),
        "{divide:?}"
    );

    edit_graph(&orch, ids::HTUI_FEAT_3, |body| {
        let node = body
            .nodes
            .iter_mut()
            .find(|node| node.phase.name == "implement")
            .expect("FEAT-3's graph has `implement`");
        node.phase.planner_agent_id = Some(ids::AGENT_CLAUDE);
    })
    .await;
    let agent = start().await;
    assert!(
        refused(&agent, crate::graph::NotRunnable::PlannerAgent),
        "{agent:?}"
    );
    let seam = orch.enqueue_divided(ids::HTUI_FEAT_3).await;
    assert!(
        refused(&seam, crate::graph::NotRunnable::PlannerAgent),
        "the seam keeps D17: {seam:?}"
    );
    assert_eq!(feat_3_run(&orch).await, None, "no run was created");
}

// -- MOD-139 M2 T4: part retries, the exhausted divided step, loops and recovery (D6, D11, D14) --

/// MOD-139 M2 T4's cases, in a frame of their own (each arm boxed, the stack rule of [`case`]).
fn divided_recovery_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>> {
    Some(match name {
        "a_failed_part_retries_alone_from_the_base_refs" => {
            Box::pin(a_failed_part_retries_alone_from_the_base_refs(harness))
        }
        "an_exhausted_part_fails_the_run_with_part_exhausted" => {
            Box::pin(an_exhausted_part_fails_the_run_with_part_exhausted(harness))
        }
        "an_exhausted_part_takes_the_fails_edge_and_re_plans" => {
            Box::pin(an_exhausted_part_takes_the_fails_edge_and_re_plans(harness))
        }
        "a_crash_before_admission_re_admits_on_the_same_planner_row" => Box::pin(
            a_crash_before_admission_re_admits_on_the_same_planner_row(harness),
        ),
        "a_crash_after_admission_re_plans_nothing_and_lands_no_part" => Box::pin(
            a_crash_after_admission_re_plans_nothing_and_lands_no_part(harness),
        ),
        "a_crashed_part_with_its_output_is_done_on_recovery" => {
            Box::pin(a_crashed_part_with_its_output_is_done_on_recovery(harness))
        }
        "a_crashed_part_without_output_retries_alone" => {
            Box::pin(a_crashed_part_without_output_retries_alone(harness))
        }
        "a_crashed_part_in_a_dirty_local_tree_parks" => {
            Box::pin(a_crashed_part_in_a_dirty_local_tree_parks(harness))
        }
        "a_local_divided_node_lands_no_part_on_recovery" => {
            Box::pin(a_local_divided_node_lands_no_part_on_recovery(harness))
        }
        "a_cancel_during_a_part_leaves_the_queued_part_pending" => Box::pin(
            a_cancel_during_a_part_leaves_the_queued_part_pending(harness),
        ),
        "a_lost_lease_during_a_part_leaves_the_queued_part_pending" => Box::pin(
            a_lost_lease_during_a_part_leaves_the_queued_part_pending(harness),
        ),
        "a_part_refusal_cancels_the_queued_parts" => {
            Box::pin(a_part_refusal_cancels_the_queued_parts(harness))
        }
        "a_failed_planner_on_recovery_ends_on_the_retry_budget" => Box::pin(
            a_failed_planner_on_recovery_ends_on_the_retry_budget(harness),
        ),
        "a_cut_short_part_retirement_replays_the_loop" => {
            Box::pin(a_cut_short_part_retirement_replays_the_loop(harness))
        }
        "a_planner_left_done_replays_the_loop" => {
            Box::pin(a_planner_left_done_replays_the_loop(harness))
        }
        _ => return None,
    })
}

/// A process killed while the divided FEAT-3 run's parts are live (`crash_on_all`'s shape): the
/// run is enqueued through the D16 seam and claimed, the claim is polled until every signal in
/// `stalled` fired, then dropped, and the run it left `running` is handed back.
///
/// # Panics
/// When the seam refuses the graph, the walk finishes instead of stalling, or the run is not
/// left `running`.
async fn crash_divided<O: Orchestrate>(orch: &O, stalled: &[Arc<Notify>]) -> RunId {
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let every = async {
        for signal in stalled {
            signal.notified().await;
        }
    };
    match within(
        futures::future::select(Box::pin(orch.claim(run)), Box::pin(every)),
        "every stall",
    )
    .await
    {
        Either::Left(_) => panic!("the walk finished instead of stalling"),
        Either::Right(((), walk)) => drop(walk),
    }
    assert_eq!(
        run_of(orch, run).await.status,
        RunStatus::Running,
        "the crash left the run `running`"
    );
    run
}

/// The rows of `steps` with `role`, in attempt order.
fn rows_of_role(steps: &[RunStep], role: StepRole) -> Vec<RunStep> {
    let mut rows: Vec<RunStep> = steps
        .iter()
        .filter(|step| step.role == role)
        .cloned()
        .collect();
    rows.sort_by_key(|step| step.attempt);
    rows
}

/// `(phase_name, attempt, status, position)` of each part under `planner`, in `(position,
/// attempt)` order.
fn part_shape(steps: &[RunStep], planner: StepId) -> Vec<(String, i32, StepStatus, i32)> {
    parts_of(steps, planner)
        .into_iter()
        .map(|part| (part.phase_name, part.attempt, part.status, part.position))
        .collect()
}

/// A part shape row, for the `assert_eq!`s over [`part_shape`].
fn shaped(
    name: &str,
    attempt: i32,
    status: StepStatus,
    position: i32,
) -> (String, i32, StepStatus, i32) {
    (name.to_owned(), attempt, status, position)
}

/// MOD-139 M2 D6: a part that fails on its own terms is retried alone at its own position, from
/// the same `base_refs` slot, while its attempts last; its sibling runs once.
async fn a_failed_part_retries_alone_from_the_base_refs<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 1;
    })
    .await;
    let repo = primary_repo(&orch).await;
    orch.script("implement/ui", 1, ScriptedStep::done_without_output());

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Failed, 6),
            shaped("implement/ui", 2, StepStatus::Done, 6),
        ]
    );
    let parts = parts_of(&steps, planner.id);
    let (first, retry) = (&parts[1], &parts[2]);
    let same = |part: &RunStep| {
        (
            part.node_key.clone(),
            part.phase_name.clone(),
            part.agent_id,
            part.model.clone(),
            part.role,
            part.parent_step_id,
            part.fanout_index,
        )
    };
    assert_eq!(
        same(retry),
        same(first),
        "the retry copies its failed attempt"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|note| note.starts_with("part `ui` of `implement` attempt 1: missing_output")),
        "{notes:?}"
    );

    let base = base_ref_of(&orch, run, repo).await;
    let trees = trees_of(&orch, retry.id).await;
    assert_eq!(trees.len(), 1, "{trees:?}");
    assert_eq!(trees[0].base_ref, base, "the retry branches from base_refs");
    let key = |phase, attempt| SessionKey {
        phase,
        attempt,
        fanout_index: 0,
        call: 0,
    };
    assert!(orch.spec_for(&key("implement/ui", 2)).is_some());
    assert!(
        orch.spec_for(&key("implement/api", 2)).is_none(),
        "the done sibling is not re-run"
    );
}

/// MOD-139 M2 D6: a part out of attempts fails the divided step, never a part alone: with no
/// `fails` edge the run fails `part_exhausted` at the planner node.
async fn an_exhausted_part_fails_the_run_with_part_exhausted<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 0;
    })
    .await;
    orch.script("implement/ui", 1, ScriptedStep::done_without_output());

    let (run, result) = start_divided(&orch).await;
    let outcome = result.expect("an exhausted part is a rest, not an error");
    let CommandOutcome::Started { rest, .. } = outcome else {
        panic!("`claim` answers `Started`, not {outcome:?}");
    };
    let failure = RunFailure::PartExhausted {
        node: "implement".to_owned(),
        part: "ui".to_owned(),
        attempts: 1,
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure.as_ref()),
        (RunStatus::Failed, Some(2), Some(&failure))
    );
    let row = run_of(&orch, run).await;
    assert_eq!(
        (row.status, row.failure),
        (RunStatus::Failed, Some(failure.to_string()))
    );
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!(planner.status, StepStatus::Done);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Failed, 6),
        ]
    );
    assert!(steps.iter().all(|step| step.position != 3), "{steps:?}");
    let plans = plans_of(&orch, run).await;
    assert_eq!(
        plans.iter().map(|plan| plan.status).collect::<Vec<_>>(),
        [htui_core::model::PlanStatus::Admitted]
    );
}

/// MOD-139 M2 D6, D11: an exhausted part with a `fails` edge on the divided node loops: the
/// region (`plan` and the planner node) is retired, the planner's parts with it and its plan
/// abandoned, and the planner's next attempt admits revision 1 past every earlier position.
async fn an_exhausted_part_takes_the_fails_edge_and_re_plans<H: CaseHarness>(harness: &H) {
    use htui_core::model::PlanStatus;

    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 0;
    })
    .await;
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "implement",
        "plan",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;
    orch.script("implement/ui", 1, ScriptedStep::done_without_output());

    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let plan: Vec<(i32, StepStatus)> = {
        let mut rows: Vec<&RunStep> = steps.iter().filter(|step| step.position == 1).collect();
        rows.sort_by_key(|step| step.attempt);
        rows.iter()
            .map(|step| (step.attempt, step.status))
            .collect()
    };
    assert_eq!(
        plan,
        [(1, StepStatus::Superseded), (2, StepStatus::Done)],
        "the loop target ran again"
    );
    let planners = rows_of_role(&steps, StepRole::Planner);
    assert_eq!(
        planners
            .iter()
            .map(|row| (row.attempt, row.status))
            .collect::<Vec<_>>(),
        [(1, StepStatus::Superseded), (2, StepStatus::Done)]
    );
    let (first, second) = (&planners[0], &planners[1]);
    assert_eq!(
        part_shape(&steps, first.id),
        [
            shaped("implement/api", 1, StepStatus::Superseded, 5),
            shaped("implement/ui", 1, StepStatus::Cancelled, 6),
        ],
        "D11: the retired planner's parts are retired with it"
    );
    assert_eq!(
        part_shape(&steps, second.id),
        [
            shaped("implement/api@1", 1, StepStatus::Done, 7),
            shaped("implement/ui@1", 1, StepStatus::Done, 8),
        ]
    );

    let mut plans = plans_of(&orch, run).await;
    plans.sort_by_key(|plan| plan.revision);
    assert_eq!(
        plans
            .iter()
            .map(|plan| (
                plan.revision,
                plan.status,
                plan.planner_step_id,
                plan.base_position
            ))
            .collect::<Vec<_>>(),
        [
            (0, PlanStatus::Abandoned, first.id, 5),
            (1, PlanStatus::Admitted, second.id, 7),
        ]
    );
    for phase in ["implement/ui", "implement/ui@1"] {
        assert!(
            orch.spec_for(&session(phase, 0)).is_some(),
            "{phase} ran its own session"
        );
    }
}

/// MOD-139 M2 D14: a crash before admission (the planner `running`, no plan, no part) is
/// admitted on recovery on the same planner row: no new row, no attempt spent.
async fn a_crash_before_admission_re_admits_on_the_same_planner_row<H: CaseHarness>(harness: &H) {
    use htui_core::model::PlanStatus;
    use htui_core::store::mem::MemFault;

    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    orch.store().set_fault(MemFault::AdmitPlan, true);
    let (run, result) = start_divided(&orch).await;
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Unreachable(_)))),
        "{result:?}"
    );
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!((planner.status, planner.attempt), (StepStatus::Running, 1));
    assert!(parts_of(&steps, planner.id).is_empty(), "{steps:?}");
    assert!(plans_of(&orch, run).await.is_empty());
    orch.store().set_fault(MemFault::AdmitPlan, false);

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    let steps = steps_of(&other, run).await;
    let planners = rows_of_role(&steps, StepRole::Planner);
    assert_eq!(
        planners
            .iter()
            .map(|row| (row.id, row.status))
            .collect::<Vec<_>>(),
        [(planner.id, StepStatus::Done)],
        "the same planner row, admitted"
    );
    let plans = plans_of(&other, run).await;
    assert_eq!(
        plans
            .iter()
            .map(|plan| (plan.revision, plan.status, plan.planner_step_id))
            .collect::<Vec<_>>(),
        [(0, PlanStatus::Admitted, planner.id)]
    );
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Pending, 5),
            shaped("implement/ui", 1, StepStatus::Pending, 6),
        ]
    );

    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Done, 6),
        ],
        "each part ran one session"
    );
    for phase in ["implement/api", "implement/ui"] {
        assert!(other.spec_for(&session(phase, 0)).is_some(), "{phase}");
    }
}

/// MOD-139 M2 D14: a crash after admission re-plans nothing; the `done` planner is a frontier
/// with no tree (it reconciles none), and no part is ever landed.
async fn a_crash_after_admission_re_plans_nothing_and_lands_no_part<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    primary_repo(&orch).await;
    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let planner = planner_of(&steps_of(&orch, run).await);

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    assert_eq!(
        other.isolator().reconciles(),
        [(planner.id, Vec::new())],
        "the planner frontier reconciles no tree, and no part lands"
    );
    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    assert_eq!(
        rows_of_role(&steps, StepRole::Planner).len(),
        1,
        "{steps:?}"
    );
    assert_eq!(plans_of(&other, run).await.len(), 1);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Done, 6),
        ]
    );
}

/// MOD-139 M2 D14: a part the crash caught after its session wrote its output is `done` on
/// recovery, with its settle columns written, and never reconciled; its sibling runs on resume.
async fn a_crashed_part_with_its_output_is_done_on_recovery<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {}).await;
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(1));
    let stalled = orch.stall_after_done("implement/api", 1, None, true);
    let run = crash_divided(&orch, &[stalled]).await;
    let steps = steps_of(&orch, run).await;
    assert_eq!(
        (
            part_named(&steps, "implement/api").status,
            part_named(&steps, "implement/ui").status
        ),
        (StepStatus::Running, StepStatus::Pending)
    );

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    let steps = steps_of(&other, run).await;
    let api = part_named(&steps, "implement/api").clone();
    assert_eq!(api.status, StepStatus::Done);
    assert!(api.finished_at.is_some(), "{api:?}");
    assert!(
        !other
            .isolator()
            .reconciles()
            .iter()
            .any(|(winner, siblings)| *winner == api.id || siblings.contains(&api.id)),
        "a part never lands"
    );

    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    let planner = planner_of(&steps);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Done, 6),
        ]
    );
}

/// MOD-139 M2 D14, D6: a part the crash caught before its output is reset and failed
/// `interrupted`; the next round retries it alone, as any failed part with an attempt left.
async fn a_crashed_part_without_output_retries_alone<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 1;
    })
    .await;
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(1));
    let stalled = orch.stall_after_done("implement/api", 1, None, false);
    let run = crash_divided(&orch, &[stalled]).await;

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    let steps = steps_of(&other, run).await;
    let api = part_named(&steps, "implement/api");
    assert_eq!(
        (api.attempt, api.status, api.gate_note.as_deref()),
        (1, StepStatus::Failed, Some("interrupted"))
    );
    assert_eq!(
        part_named(&steps, "implement/ui").status,
        StepStatus::Pending
    );
    assert_eq!(other.isolator().resets(), 1, "the part's tree was reset");

    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    let planner = planner_of(&steps);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Failed, 5),
            shaped("implement/api", 2, StepStatus::Done, 5),
            shaped("implement/ui", 1, StepStatus::Done, 6),
        ]
    );
}

/// MOD-139 M2 D14 (plan D93): a part whose `local` tree was dirty at its start is never reset:
/// it fails `interrupted, tree not reset` and the run parks for a human.
async fn a_crashed_part_in_a_dirty_local_tree_parks<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None)], |phase| {
        phase.isolation = Some(Isolation::Local);
    })
    .await;
    primary_repo(&orch).await;
    let stalled = orch.stall_after_done("implement/api", 1, None, false);
    let run = crash_divided(&orch, &[stalled]).await;
    let api = part_named(&steps_of(&orch, run).await, "implement/api").clone();
    let mut trees = trees_of(&orch, api.id).await;
    assert!(!trees.is_empty(), "stage 2 recorded the tree");
    for tree in &mut trees {
        tree.dirty = true;
    }
    orch.store()
        .upsert_step_tree(StepFence::Lease(orch.owner()), api.id, &trees)
        .await
        .expect("MemStore rewrites the rows");

    let other = orch.restarted();
    let adopted = other.sweep().await.expect("the sweep adopts");
    assert_eq!(adopted.len(), 1, "{adopted:?}");
    // T4 repair 2: the park stands on the divided node (the planner's position, as
    // `status::position_of` reads a part) and names the part, not the planner node.
    let planner = planner_of(&steps_of(&other, run).await);
    let parked = Rest {
        run: RunStatus::AwaitingApproval,
        position: Some(planner.position),
        failure: Some(RunFailure::Interrupted {
            phase: "implement/api".to_owned(),
            reset: false,
        }),
    };
    assert_eq!(adopted[0].next, Next::Parked(parked), "{adopted:?}");
    let api = part_named(&steps_of(&other, run).await, "implement/api").clone();
    assert_eq!(
        (api.status, api.gate_note.as_deref()),
        (StepStatus::Failed, Some("interrupted, tree not reset"))
    );
    assert_eq!(other.isolator().resets(), 0, "no `git` write on this path");
    let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|note| note.starts_with(&format!(
            "interrupted, tree not reset: step {} (`implement/api` attempt 1); trees: ",
            api.id
        ))),
        "{notes:?}"
    );

    // T4 repair 2 (plan D93, D14): the park holds. A tree the sweep left alone is never walked
    // over: neither a resume nor `Unblock` creates the part's next attempt, as for a plain step.
    let resumed = other
        .resume(run)
        .await
        .expect("a parked run resumes to its rest");
    assert!(
        matches!(
            resumed,
            Resume::Walked(Rest {
                run: RunStatus::AwaitingApproval,
                position,
                ..
            }) if position == Some(planner.position)
        ),
        "{resumed:?}"
    );
    let refused = other
        .dispatch(Command::Unblock {
            item: ids::HTUI_FEAT_3,
        })
        .await
        .expect_err("the park waits on a human");
    assert!(
        matches!(&refused, EngineError::NotBlocked { state: ItemState::NeedsYou, why, .. }
            if *why == crate::command::run_waits_at_a_gate(run)),
        "{refused}"
    );
    let steps = steps_of(&other, run).await;
    assert_eq!(
        part_shape(&steps, planner.id),
        [shaped("implement/api", 1, StepStatus::Failed, 5)],
        "no attempt 2 over the tree"
    );
    assert_eq!(
        run_of(&other, run).await.status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(other.isolator().resets(), 0);
}

/// MOD-139 M2 D14 (blueprint P-21): under `local` no part is ever reconciled on recovery, and the
/// planner at most with no sibling (it has no tree); nothing is re-planned or re-run.
async fn a_local_divided_node_lands_no_part_on_recovery<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None)], |phase| {
        phase.isolation = Some(Isolation::Local);
    })
    .await;
    primary_repo(&orch).await;
    let (run, result) = start_divided(&orch).await;
    stops_at_integrate(&result);
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    let api = part_named(&steps, "implement/api").clone();

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    let reconciles = other.isolator().reconciles();
    assert!(
        reconciles
            .iter()
            .all(|(winner, siblings)| *winner == planner.id && siblings.is_empty()),
        "at most the planner, with no sibling: {reconciles:?}"
    );
    assert!(
        !reconciles
            .iter()
            .any(|(winner, siblings)| *winner == api.id || siblings.contains(&api.id)),
        "{reconciles:?}"
    );
    for log in [
        call_log_of(&orch, run).await,
        call_log_of(&other, run).await,
    ] {
        assert!(
            !log.iter().any(|call| call.starts_with("reconcile 5/")),
            "{log:?}"
        );
    }
    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    assert_eq!(
        part_shape(&steps, planner.id),
        [shaped("implement/api", 1, StepStatus::Done, 5)]
    );
    assert_eq!(plans_of(&other, run).await.len(), 1);
}

// -- MOD-139 M2 R1: a parts round stops pulling, a failed planner repeats nothing ----------------

/// The divided FEAT-3 graph (`api`, `ui`) over a primary repo with one session at a time, so the
/// round runs `api` and holds `ui` in its queue.
async fn divided_width_one<O: Orchestrate>(orch: &O) {
    divided_feat_3(orch, &[("api", None), ("ui", None)], |_| {}).await;
    primary_repo(orch).await;
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(1));
}

/// That the queued `ui` part was never pulled: still `pending`, never prepared, no session.
async fn assert_ui_never_pulled<O: Orchestrate>(orch: &O, run: RunId) {
    let steps = steps_of(orch, run).await;
    assert_eq!(
        part_named(&steps, "implement/ui").status,
        StepStatus::Pending,
        "the queued part stays `pending`"
    );
    let log = call_log_of(orch, run).await;
    assert!(
        !log.iter().any(|call| call.starts_with("prepare 6/")),
        "the queued part is never prepared: {log:?}"
    );
    assert!(
        orch.spec_for(&session("implement/ui", 0)).is_none(),
        "the queued part starts no session"
    );
}

/// MOD-139 M2 R1 (F-7, MOD-42 I-6): a cancel while the round's first part is live stops the round
/// pulling. The live part settles nothing, the queued one is never moved, prepared or started,
/// and the walk answers `Cancelled`.
async fn a_cancel_during_a_part_leaves_the_queued_part_pending<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_width_one(&orch).await;
    let hold = orch.hold_sessions(&[("implement/api", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let cancel = async {
        within(hold.live(1), "`api` held").await;
        orch.cancel_walks(std::time::Duration::from_millis(10));
    };
    let (result, ()) = tokio::join!(orch.claim(run), cancel);
    assert!(
        matches!(&result, Err(EngineError::Cancelled { run: stopped }) if *stopped == run),
        "{result:?}"
    );
    assert_ui_never_pulled(&orch, run).await;
}

/// MOD-139 M2 R1 (F-7, D4): a lease another process took while the round's first part is live
/// stops the round pulling: the queued part never moves `pending -> running` on a run that is
/// no longer this process's, and the caller gets `LeaseLost`.
async fn a_lost_lease_during_a_part_leaves_the_queued_part_pending<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_width_one(&orch).await;
    let hold = orch.hold_sessions(&[("implement/api", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let other = orch.restarted();
    let steal = async {
        within(hold.live(1), "`api` held").await;
        assert!(
            orch.store()
                .release_lease(run, orch.owner())
                .await
                .expect("MemStore releases"),
            "the walk held the lease"
        );
        assert!(
            other
                .store()
                .take_lease(run, ids::BOX, other.owner(), TimeDelta::days(1))
                .await
                .expect("MemStore takes the lease"),
            "another process takes the free lease"
        );
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), steal);
    assert!(
        matches!(&result, Err(EngineError::LeaseLost { run: lost }) if *lost == run),
        "{result:?}"
    );
    assert_ui_never_pulled(&orch, run).await;
    assert!(
        other
            .store()
            .refresh_lease(run, other.owner(), TimeDelta::minutes(1))
            .await
            .expect("MemStore refreshes"),
        "the other process still holds the lease"
    );
}

/// MOD-139 M2 R1 (F-7, P-11): a deterministic refusal of the round's first part (its prepare)
/// ends the run on that refusal, and the round pulls no further part: the queued one starts no
/// session and is `cancelled` with the cut's note, never left `pending` under the `failed` run.
async fn a_part_refusal_cancels_the_queued_parts<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    divided_width_one(&orch).await;
    // `plan` is the last prepare before the parts round, which admission makes none of.
    let hold = orch.hold_sessions(&[("plan", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let refuse = async {
        within(hold.live(1), "`plan` held").await;
        orch.isolator().refuse_prepare("the api tree is locked");
        hold.release();
    };
    let (result, ()) = tokio::join!(orch.claim(run), refuse);
    let outcome = result.expect("a refused part is a rest, not an error");
    let CommandOutcome::Started { rest, .. } = outcome else {
        panic!("`claim` answers `Started`, not {outcome:?}");
    };
    assert_eq!((rest.run, rest.position), (RunStatus::Failed, Some(2)));
    let row = run_of(&orch, run).await;
    assert_eq!(row.status, RunStatus::Failed);
    assert!(
        row.failure
            .as_deref()
            .is_some_and(|text| text.contains("the api tree is locked")),
        "the run fails on the first part's refusal: {row:?}"
    );
    let steps = steps_of(&orch, run).await;
    let planner = planner_of(&steps);
    assert_eq!(
        part_shape(&steps, planner.id),
        [
            shaped("implement/api", 1, StepStatus::Failed, 5),
            shaped("implement/ui", 1, StepStatus::Cancelled, 6),
        ]
    );
    let log = call_log_of(&orch, run).await;
    assert!(
        !log.iter().any(|call| call.starts_with("prepare 6/")),
        "the queued part is never prepared: {log:?}"
    );
    assert!(
        orch.spec_for(&session("implement/ui", 0)).is_none(),
        "the queued part starts no session"
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes
            .iter()
            .any(|note| note
                == "step `implement/ui` attempt 1 cancelled: the run ended at `implement`"),
        "{notes:?}"
    );
}

/// MOD-139 M2 D6 (blueprint §T4.3, R1): a `failed` planner on recovery is an admission refusal
/// whose `block_and_fail` a crash cut off. Every admission refusal is deterministic (P-8), so
/// neither the node's retry nor its `fails` edge repeats it, whatever the budget: with both a
/// retry left and a `fails` edge on the divided node, the run ends on `retry_budget_spent`, takes
/// no loop and creates no planner attempt 2. The `fails` half holds twice over: `settle_failed`
/// skips the edge for a planner, and the planner node `implement:plan` sources no `fails` edge
/// (`expand_divide` keeps the divided node's out-edges on `implement`), so the edge here pins the
/// behaviour should the lookup ever move to the divided name (R1 repair 1).
async fn a_failed_planner_on_recovery_ends_on_the_retry_budget<H: CaseHarness>(harness: &H) {
    use htui_core::store::mem::MemFault;

    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 1;
    })
    .await;
    // A `fails` edge on the divided node, which a part's exhaustion would take (D6, D11): the
    // refusal takes neither it nor the retry.
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "implement",
        "plan",
        EdgeKind::Fails,
        None,
        Some(1),
    )
    .await;
    // The walk stops with the planner `running` and no plan; the planner is then failed by hand,
    // as a refusal's `running -> failed` would leave it with no `block_and_fail` after it.
    orch.store().set_fault(MemFault::AdmitPlan, true);
    let (run, result) = start_divided(&orch).await;
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Unreachable(_)))),
        "{result:?}"
    );
    orch.store().set_fault(MemFault::AdmitPlan, false);
    let planner = planner_of(&steps_of(&orch, run).await);
    assert!(
        orch.store()
            .transition_step(
                planner.id,
                StepStatus::Running,
                StepStatus::Failed,
                orch.clock().now()
            )
            .await
            .expect("a legal move"),
        "the planner was `running`"
    );

    let other = orch.restarted();
    let next = adopted_next(&other, run).await;
    let Next::Finished(rest) = next else {
        panic!("the sweep ends the run, not {next:?}");
    };
    assert_eq!(
        (rest.run, rest.position, rest.failure),
        (
            RunStatus::Failed,
            Some(2),
            Some(RunFailure::RetryBudgetSpent {
                phase: planner.phase_name.clone(),
                attempt: 1,
            })
        )
    );
    assert_eq!(run_of(&other, run).await.status, RunStatus::Failed);
    let steps = steps_of(&other, run).await;
    assert_eq!(
        rows_of_role(&steps, StepRole::Planner)
            .iter()
            .map(|row| (row.id, row.attempt, row.status))
            .collect::<Vec<_>>(),
        [(planner.id, 1, StepStatus::Failed)],
        "no planner attempt 2"
    );
    assert!(
        steps
            .iter()
            .all(|step| step.status != StepStatus::Superseded),
        "the `fails` edge looped nothing: {steps:?}"
    );
    assert!(parts_of(&steps, planner.id).is_empty(), "{steps:?}");
    assert!(plans_of(&other, run).await.is_empty());
}

/// MOD-139 M2 R1 (F-8, blueprint A6): a store error cuts the `fails` loop's retirement of an
/// exhausted part's planner slot short: `plan` is retired, the exhausted `api` part's
/// `failed -> cancelled` is refused, and `ui` and the plan are left as they were. The planner,
/// retired last, is still `done` and its exhausted part still `failed`, so the resumed walk takes
/// the loop again and finishes it, end to end: see [`cut_retirement_replays`].
async fn a_cut_short_part_retirement_replays_the_loop<H: CaseHarness>(harness: &H) {
    cut_retirement_replays(harness, false).await;
}

/// MOD-139 M2 R1 (F-8): [`a_cut_short_part_retirement_replays_the_loop`] one write later, as a
/// crash between `abandon_plans` and the planner row's own retirement leaves it: the parts are
/// retired and revision 0 is `abandoned`, the planner alone still `done`. A retired part keeps
/// its attempt, and `cancelled` is a failed part (`divide::exhausted`), so the exhausted part
/// still rests the planner `failed` and the resumed walk takes the loop again.
async fn a_planner_left_done_replays_the_loop<H: CaseHarness>(harness: &H) {
    cut_retirement_replays(harness, true).await;
}

/// The two R1 (F-8) replay cases. `api` is exhausted (`retry_limit 0`), the divided node has a
/// `fails` edge back to `plan` with a budget of two, and `api`'s `failed -> cancelled` is
/// faulted, so the loop's retirement stops after `plan`. With `by_hand`, the rest of the planner
/// slot but the planner row is then retired by hand. The resumed walk takes the loop again and
/// finishes it: the old parts are retired, revision 0 is `abandoned`, the planner's attempt 2
/// admits revision 1, and no row is left `pending`.
async fn cut_retirement_replays<H: CaseHarness>(harness: &H, by_hand: bool) {
    use htui_core::model::PlanStatus;
    use htui_core::store::mem::MemFault;

    let orch = harness.fresh();
    divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
        phase.retry_limit = 0;
    })
    .await;
    orch.store()
        .set_app_setting("max_parallel_sessions", serde_json::json!(1));
    add_edge(
        &orch,
        ids::HTUI_FEAT_3,
        "implement",
        "plan",
        EdgeKind::Fails,
        None,
        Some(2),
    )
    .await;
    orch.script("implement/api", 1, ScriptedStep::done_without_output());
    // `api` runs first and fails; while `ui` is live, `api`'s retirement (`failed -> cancelled`)
    // is faulted.
    let hold = orch.hold_sessions(&[("implement/ui", 1)]);
    let run = orch
        .enqueue_divided(ids::HTUI_FEAT_3)
        .await
        .expect("the seam resolves the divided graph");
    let fault = async {
        within(hold.live(1), "`ui` held").await;
        let api = part_named(&steps_of(&orch, run).await, "implement/api").id;
        orch.store().set_fault(MemFault::TransitionStep(api), true);
        hold.release();
        api
    };
    let (result, api) = tokio::join!(orch.claim(run), fault);
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Unreachable(_)))),
        "the retirement is cut short: {result:?}"
    );
    orch.store().set_fault(MemFault::TransitionStep(api), false);
    let steps = steps_of(&orch, run).await;
    let first = planner_of(&steps);
    assert_eq!(
        (first.attempt, first.status),
        (1, StepStatus::Done),
        "the planner, the loop's source, is retired last"
    );
    assert_eq!(
        part_shape(&steps, first.id),
        [
            shaped("implement/api", 1, StepStatus::Failed, 5),
            shaped("implement/ui", 1, StepStatus::Done, 6),
        ]
    );
    let statuses = |plans: &[htui_core::model::RunPlan]| {
        plans.iter().map(|plan| plan.status).collect::<Vec<_>>()
    };
    assert_eq!(
        statuses(&plans_of(&orch, run).await),
        [PlanStatus::Admitted]
    );
    if by_hand {
        let now = orch.clock().now();
        let parts = parts_of(&steps, first.id);
        assert!(
            orch.store()
                .transition_step(parts[0].id, StepStatus::Failed, StepStatus::Cancelled, now)
                .await
                .expect("a legal move"),
            "`api` was `failed`"
        );
        orch.store()
            .supersede_step(parts[1].id)
            .await
            .expect("`ui` was `done`");
        orch.store()
            .abandon_plans(StepFence::Unleased, first.id)
            .await
            .expect("revision 0 was `admitted`");
    }

    let other = orch.restarted();
    assert_eq!(adopted_next(&other, run).await, Next::Walk);
    let resumed = other.resume(run).await;
    stops_at_integrate(&resumed);
    let steps = steps_of(&other, run).await;
    // The resumed walk re-creates `plan`, the retired target, before the planner node is read
    // again, so the replay costs one more turn of the loop's budget: MOD-138 M2 F6's hazard,
    // which A6 cures only for `asks_you` loops, and which a failed-source loop keeps.
    let mut plan: Vec<(i32, StepStatus)> = steps
        .iter()
        .filter(|step| step.position == 1)
        .map(|step| (step.attempt, step.status))
        .collect();
    plan.sort_by_key(|(attempt, _)| *attempt);
    assert_eq!(
        plan,
        [
            (1, StepStatus::Superseded),
            (2, StepStatus::Superseded),
            (3, StepStatus::Done)
        ],
        "the target ran again before the replayed loop retired it"
    );
    let planners = rows_of_role(&steps, StepRole::Planner);
    assert_eq!(
        planners
            .iter()
            .map(|row| (row.id, row.attempt, row.status))
            .collect::<Vec<_>>(),
        [
            (first.id, 1, StepStatus::Superseded),
            (planners[1].id, 2, StepStatus::Done)
        ],
        "the loop ran again: {steps:?}"
    );
    assert_eq!(
        part_shape(&steps, first.id),
        [
            shaped("implement/api", 1, StepStatus::Cancelled, 5),
            shaped("implement/ui", 1, StepStatus::Superseded, 6),
        ],
        "the replay retired the old parts"
    );
    assert_eq!(
        part_shape(&steps, planners[1].id),
        [
            shaped("implement/api@1", 1, StepStatus::Done, 7),
            shaped("implement/ui@1", 1, StepStatus::Done, 8),
        ]
    );
    let mut plans = plans_of(&other, run).await;
    plans.sort_by_key(|plan| plan.revision);
    assert_eq!(
        plans
            .iter()
            .map(|plan| (plan.revision, plan.status, plan.planner_step_id))
            .collect::<Vec<_>>(),
        [
            (0, PlanStatus::Abandoned, first.id),
            (1, PlanStatus::Admitted, planners[1].id),
        ]
    );
    assert!(
        steps.iter().all(|step| step.status != StepStatus::Pending),
        "no part is left `pending`: {steps:?}"
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
    fn cases_are_unique_and_counted() {
        let mut sorted: Vec<&&str> = CASES.iter().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
        assert_eq!(
            CASES.len(),
            219,
            "18 + 5 + 13 + 6 + 10 + 18 + 2 + 1 + 1 + 1 + 11 + 1 + 5 + 1 + 1 + 6 + 3 + 5 + 1 + 6 + 3 + 9 + 13 + 17 + 2 + 1 + 10 + 1 + 2 + 1 + 5 + 1 + 6 + 18 + 9 + 6: eighteen before milestone 4 (six ANA-2 §12 criteria, four §4.2 \
             contract lines, the `finish_run` seam, the three gate-table cells only an edited \
             gate reaches, plan D5's intermediate position, milestone 3's two verify outcomes \
             and `CancelRun`), milestone 4's five stage-1 cases (the `allowed_warning` \
             selection, the fall-through to the next candidate, the rung-4 refusal at \
             `StartRun`, the stage-1 walk that skips every candidate, and the second attempt's \
             forwarded `verify_failure` and `previous_diff`), its thirteen fan-out cases \
             (criteria 8, 9 twice and 10 twice, the gated and judgeless human selections, a \
             failed candidate, a group with no survivor, the review loop over a group, the two \
             caps, and a `shared_serialized` sibling's own deadline), and milestone 5's six \
             (criterion 15's serialised overlap, its two repos and its undeclared item, \
             criterion 16's slot and parked overlap, a park's released lease taken by another \
             process's answer, and a live lease refusing that answer), and milestone 5's ten \
             recovery cases (criterion 18's finished and unfinished steps, the out-of-budget \
             park, criterion 12's dirty tree, two lost merges, an interrupted candidate and \
             judge, a half-written park, and the runs no sweep adopts), and milestone 6's \
             eighteen (ANA-5 criterion 3's refused prompt at `walk_step` and at `drive_group`, \
             criterion 3's running run parked on a topology mismatch, a cancel meeting a live \
             lease, four promotions, five accepts, `Unblock`'s three cases, and criterion \
             20's close-out and its refusal), and MOD-7 milestone 3's two (criterion 14's \
             capability half and §4.10's claim-time half), and MOD-40 milestone 1's one (a \
             suspended walk fenced after adoption), CLEAN-4's one (identical reviews stop \
             the loop on `no_progress_review`), and MOD-41 T1's fenced capture (a walk \
             woken after `Done` records no commits, plan D1), and MOD-41 T9's eleven hand-back cases (plan D12: \
             one per walking command and retry route walked by the adopter, the changed-graph \
             park of OQ-6, and the window that writes nothing past itself), and MOD-37 review L1's \
             one (a crashed rejection's `u` handed back and failed by the adopter), and MOD-26's five \
             persona cases (plan D9, D12, D13, OQ-2: the persona frame, an edit that does not \
             reach a started run, the judge without a persona, `command_run` dropping the \
             `command_queue` section, and the agent row left untouched), and MOD-37 milestone \
             4's one (a promoted `shared_serialized` step's run refusing another claim, R-49), \
             and MOD-73's one (a gate edit read by the next phase, plan D2), and MOD-11 T8's six \
             (plan D16, D17: `fan_out_only` at one agent, at two and on a `heavy_build` item, \
             the denials with and without a persona, and the judge never exposed), and MOD-12 \
             M1's three (criterion 23's two halves, criterion 24), and MOD-12 M2's five (the \
             batch walk refusal, the batch remainder, the manual run, the run cap across steps, \
             and review R1's snapshotted run cap against a lowered live one), and CLEAN-9 T2's one \
             (a malformed cap key refused at start), and MOD-138 M1's six pinned-resume cases \
             (plan D5: a v2 run walking on after a live edit on the three resume paths, a \
             tampered v2 snapshot parked; blueprint A6: a v1 run on a refused live graph \
             walking on, and parked once its phases moved), and MOD-144's three (a queued cancel \
             writing no item, a stopped run holding its item, a rework run on a finished item), \
             and MOD-138 M2 T4's nine loops over edges (plan D7, D10, Q-D: a `changes_requested` \
             edge on a node not called `review`, a `review` with none passing unread, a `fails` \
             loop retiring its region, on a human's reject, replayed after a crash and spent, a \
             diamond's region, no progress on an edge loop, and a graph with authored edges \
             resolving), and MOD-138 M2 T5's thirteen `if` and `asks_you` cases (plan D6, D7, D8; \
             blueprint A2, A3, A9: each `if` predicate, a declined branch and its join, an \
             undecided branch parked and resumed, a forward `asks_you` approved and rejected, \
             auto mode's stop, a back `asks_you` loop, its crash replay and its spent budget, a \
             decision read from its row after a restart, accept declining, a verdict-reading \
             `asks_you` node's reject looping, and a failed settle's park rejected as ever), and \
             MOD-138 M3 T3's seventeen concurrency cases (plan D1-D5, blueprint A2-A6, A11: a linear \
             graph's isolator call order unchanged, two branches live at once, a cap of two over three \
             branches, a fan-out heavier than the cap alone, a join waiting for both landings, two \
             worktree landings into one repository, shared_serialized branches never overlapping, \
             an in-place node never overlapping a worktree node, an owed selection landing before an \
             in-place node, a park draining its sibling, a sibling parking on a parked run, one answer \
             of two parks re-parking, a node ending the run cutting its live sibling, a park then \
             an end cancelling the parked step, cleanup deferred until the sibling settled, a loop on one branch retiring only its region, and \
             an owed loop running before its successor), and T3 repair 1's two (cleanup deferred \
             until a cut sibling settled, and a terminal end cancelling a sibling row created \
             beside it), and T3 repair 2's one (a rejection ending the run cancelling the other \
             parked step), and MOD-138 M3 T4's ten recovery cases (plan D6, blueprint A7-A9: two \
             unlanded branches landed in order, a reordered chain landing the node walked last, a \
             done branch landed beside a running one, an already landed branch reconciled \
             again, only the later of two shared_serialized landings re-landed, two running \
             branches both adjudicated, running branches recovered into a park each, and a re-run \
             loop target re-landed beside an older side successor, and a crash-cut unpark on a \
             branching run landing every frontier member, and an answer to one of two parks \
             landing the other, crashed answer's winner first), and T4's hand-back twin of T3 \
             repair 2 (a rejection handed back cancelling the other parked step), and T4 repair \
             1's two (a looping rejection landing a crashed answer's winner first, in process \
             and resumed), and T4 repair 2's one (a looping rejection handed back walking its \
             loop as in process), and MOD-138 M3 R1's five input-order cases (review finding 1: \
             a consumer waiting for an incomparable producer, the reversed direction, a two-way \
             pair, a cycle of waits, and a skipped producer), and R1's finding-6 one (a \
             rejection handed back beside an interrupted park ending the run as in process), \
             and MOD-163 M2's six memory cases (plan D11, D12, D18: an entry reaching every \
             phase of its project, closed entries kept out, an edit after `StartRun`, fan-out \
             siblings alike, a pre-MOD-163 snapshot, no entries no row), and MOD-139 M2 T3's \
             eighteen divided-node cases (plan D1-D5, D7, D8, D10, D13, D15-D17), and MOD-139 M2 \
             T4's nine (a part retried alone, the exhausted divided step, its `fails` loop and \
             re-plan, and recovery per role), and MOD-139 M2 R1's six (a cancel, a lost lease \
             and a refusal stopping a parts round's pulls, a failed planner on recovery, and a \
             `fails` loop's cut-short retirement replayed twice over)"
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
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        Gate, Isolation, NewRunStep, RunStatus, RunStep, RunStepCommit, Status, StepId, StepRole,
        StepStatus,
    };
    use htui_core::store::{ReadStore as _, StepFence, WriteStore as _};

    use super::{
        FakeOrchestrator, Orchestrate, RunFailure, ScriptedStep, candidate, fan_research,
        free_feat_3, item_of, judge_both, judge_of, latch_quota, notes_of, primary_repo, repoint,
        research_candidates, run_of, selection_parks, set_input_kinds, set_judge, slot_of, start,
        steps_of,
    };
    use crate::engine::Resume;
    use crate::isolate::Clock as _;

    /// A crash's leftover run: `awaiting_approval -> running`, by hand, so `resume` walks rows no
    /// command answered.
    async fn unpark_by_hand(orch: &FakeOrchestrator, run: htui_core::model::RunId) {
        let now = orch.clock().now();
        assert!(
            orch.store()
                .transition_run(run, RunStatus::AwaitingApproval, RunStatus::Running, now)
                .await
                .expect("MemStore takes the move"),
            "the run was parked"
        );
    }

    /// `resume`, unwrapped to the rest of a walk that ran.
    async fn resumed(orch: &FakeOrchestrator, run: htui_core::model::RunId) -> super::Rest {
        match orch.resume(run).await.expect("the walk resumes") {
            Resume::Walked(rest) => rest,
            Resume::TopologyChanged { .. } => panic!("nothing edited the graph"),
            Resume::SnapshotTampered { .. } => panic!("nothing edited the snapshot"),
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
                role: StepRole::Step,
                parent_step_id: None,
                node_key: None,
            })
            .await
            .expect("the slot has no judge yet")
            .id
    }

    /// Candidate `index` of `(0, 1)`'s id.
    fn candidate_id(steps: &[RunStep], index: i32) -> StepId {
        candidate(steps, 0, 1, index).id
    }

    /// MOD-10 D5: one turn whose assistant text the scrubber refuses, so the recorder writes a
    /// `scrub_residue` row and its `finish` answers `Unmasked`; then a document with `body`. The
    /// key matches under both the bare-prefix and the whole-token rules (blueprint H-7).
    fn leaks(body: &str) -> ScriptedStep {
        ScriptedStep {
            script: Script::one_turn(vec![
                ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
                    text: "the key is sk-ant-api03-abcdefghijklmnopqrstuvwx".into(),
                    message_id: None,
                })),
                ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                })),
            ]),
            output: Some(body.into()),
            spawn_failure: None,
        }
    }

    /// The `ScrubRefused` sentence for an Anthropic key at `phase`, built from the variant so the
    /// bytes are pinned once, in `status.rs` (blueprint H-12).
    fn scrub_sentence(phase: &str) -> String {
        RunFailure::ScrubRefused {
            phase: phase.to_owned(),
            rule: "anthropic_api_key".to_owned(),
        }
        .to_string()
    }

    /// MOD-10 D5, candidate path: a candidate whose recorder refuses is `failed` with the typed
    /// scrub sentence in its note; the run is not failed by it, its siblings carry on.
    #[tokio::test]
    async fn a_candidate_whose_recorder_refuses_fails_with_the_scrub_sentence() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        orch.script_candidate("research", 1, 0, 0, leaks("research by candidate 0"));
        judge_both(&orch, "research", 1, 1, "best");

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (RunStatus::Done, None, None),
            "the surviving sibling wins and the run completes"
        );
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Failed, Some(false)),
                (1, StepStatus::Done, Some(true)),
                (2, StepStatus::Superseded, Some(false)),
            ],
            "the refused candidate keeps `failed`; its sibling is selected"
        );
        let expected = format!(
            "fan-out candidate 0 of `research` attempt 1: {}",
            scrub_sentence("research")
        );
        let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
        assert!(
            notes.contains(&expected),
            "the candidate's note carries the sentence: {notes:?}"
        );
        assert!(
            notes.iter().all(|note| !note.contains("sk-ant-")),
            "no note repeats the key: {notes:?}"
        );
        assert_eq!(
            run_of(&orch, run).await.failure,
            None,
            "the candidate's refusal does not fail the run"
        );
    }

    /// MOD-10 D5, judge path: a judge whose recorder refuses is `failed` with
    /// `judge_session_failed: <sentence>` as its `gate_note`, and the run parks with no failure.
    /// Both calls succeed, because `judge_sessions` reads `finish` only then.
    #[tokio::test]
    async fn a_judge_whose_recorder_refuses_parks_with_the_scrub_sentence() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        let verdict = "Compared.\n\n```json\n{\"winner\": 1, \"reasons\": {\"1\": \"best\"}}\n```";
        orch.script_candidate("research:judge", 1, -1, 0, leaks(verdict));
        orch.script_candidate("research:judge", 1, -1, 1, ScriptedStep::judge(1, &[]));

        let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        assert_eq!(run_of(&orch, run).await.failure, None);
        let steps = steps_of(&orch, run).await;
        let judge = judge_of(&steps, 0, 1).expect("the judge ran");
        let expected = format!("judge_session_failed: {}", scrub_sentence("research:judge"));
        assert_eq!(
            (judge.status, judge.gate_note.as_deref()),
            (StepStatus::Failed, Some(expected.as_str()))
        );
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
                role: StepRole::Step,
                parent_step_id: None,
                node_key: None,
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
        // The park released the lease: the hand-written slot is unleased.
        orch.store()
            .record_commits(
                StepFence::Unleased,
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
            done: &DoneEvent,
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
        let scrubber = crate::secrets::RunSecrets::new(orch.secret_source());
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
            policy: parts.policy,
            control: parts.control,
            scrubber: parts.scrubber,
            secrets: parts.secrets,
            app: parts.app,
            box_profile: parts.box_profile,
            box_id: parts.box_id,
            owner: parts.owner,
            dead_walks: parts.dead_walks,
            user: parts.user,
            tails: crate::engine::Tails::Walk,
            tools: None,
        });

        let err = engine
            .dispatch(crate::command::Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: htui_core::model::RunMode::Manual,
                repo_scope: None,
                workspace: None,
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

    /// Invariant 2: an app setting lowered under an in-flight run makes the live graph refuse to
    /// resolve (plan D63), which is not the run's business — it walks its snapshot. `resume`
    /// notes that the live graph could not be compared and walks on instead of erroring. Only a
    /// **v1** run resolves the live graph at all (MOD-138 D5, F3), so the run is one.
    #[tokio::test]
    async fn a_lowered_cap_does_not_strand_a_run_on_resume() {
        for (key, cap) in [("max_fan_out", 2), ("max_agents_per_run", 2)] {
            let orch = FakeOrchestrator::demo();
            fan_research(&orch, Gate::Always, false).await;
            research_candidates(&orch, 1);
            let (run, rest) = start(&orch, ids::HTUI_ANA_2).await;
            assert_eq!(rest.run, RunStatus::AwaitingApproval);
            crate::fake::rewrite_as_v1(orch.store(), run).await;

            orch.store().set_app_setting(key, serde_json::json!(cap));
            let rest = resumed(&orch, run).await;
            assert_eq!(rest.run, RunStatus::AwaitingApproval, "{key}");
            let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
            assert!(
                notes.iter().any(|note| note.starts_with(&format!(
                    "live graph not comparable: run {run} walks its snapshot"
                ))),
                "{key}: {notes:?}"
            );
        }
    }

    // -- MOD-10 M3 T4 (D11-D13, blueprint A-1, A-2, H-14): the walk's secrets on the fan-out and
    // judge paths. Never in `CASES` (blueprint H-1: `every_case_name_dispatches` sits near the
    // stack).

    /// An Infisical scope the fake provider resolves.
    const SECRET_SCOPE: &str = r#"{"project_id":"p1","environment":"dev","path":"/"}"#;
    /// Long and not pattern-shaped, so a refusal can never stand in for a mask.
    const SECRET_VALUE: &str = "zq7-resolved-value-0123456789";

    /// Candidate `index`'s first session at `research` attempt 1.
    fn research_key(index: i32) -> crate::engine::SessionKey<'static> {
        crate::engine::SessionKey {
            phase: "research",
            attempt: 1,
            fanout_index: index,
            call: 0,
        }
    }

    /// `ANA-2`'s project.
    async fn ana_2_project(orch: &FakeOrchestrator) -> htui_core::model::ProjectId {
        item_of(orch, ids::HTUI_ANA_2).await.project_id
    }

    /// The source a walk of `orch` resolves through, over `provider`.
    fn secret_source(
        orch: &FakeOrchestrator,
        provider: htui_core::secret::fake::FakeSecretProvider,
    ) -> std::sync::Arc<htui_core::secret::fake::FakeSecretSource> {
        let source = std::sync::Arc::new(htui_core::secret::fake::FakeSecretSource::new(
            std::sync::Arc::new(provider),
        ));
        orch.set_secret_source(source.clone());
        source
    }

    /// MOD-10 D11, D12 (blueprint A-1): the group's entry resolves once for all three candidates,
    /// and each candidate's session gets the resolved map.
    #[tokio::test]
    async fn a_fan_out_resolves_once_and_every_candidate_gets_the_env() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        research_candidates(&orch, 1);
        let project = ana_2_project(&orch).await;
        orch.store().set_project_secret_columns(
            project,
            Some(htui_core::secret::INFISICAL),
            Some(SECRET_SCOPE),
        );
        let provider = std::sync::Arc::new(htui_core::secret::fake::FakeSecretProvider::resolving(
            &[("API_KEY", SECRET_VALUE)],
        ));
        orch.set_secret_source(std::sync::Arc::new(
            htui_core::secret::fake::FakeSecretSource::new(provider.clone()),
        ));

        let _ = Box::pin(start(&orch, ids::HTUI_ANA_2)).await;

        let expected =
            std::collections::BTreeMap::from([("API_KEY".to_owned(), SECRET_VALUE.to_owned())]);
        for index in 0..3 {
            let spec = orch
                .spec_for(&research_key(index))
                .unwrap_or_else(|| panic!("candidate {index} started"));
            assert_eq!(spec.env, expected, "candidate {index}");
        }
        assert_eq!(provider.resolves(), 1, "one resolution for the group");
    }

    /// MOD-10 D13 (blueprint A-2, H-13): a refusal at the group's entry fails every pending
    /// candidate with the sentence and the run with it; no candidate starts, no attempt 2 follows,
    /// and the item is not blocked.
    #[tokio::test]
    async fn a_fan_out_whose_secrets_refuse_fails_the_run_before_any_candidate_starts() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        research_candidates(&orch, 1);
        let project = ana_2_project(&orch).await;
        orch.store().set_project_secret_columns(
            project,
            Some(htui_core::secret::INFISICAL),
            Some(SECRET_SCOPE),
        );
        let cause = htui_core::secret::SecretError::BadCredentials;
        let source = secret_source(
            &orch,
            htui_core::secret::fake::FakeSecretProvider::failing(cause.clone()),
        );

        let (run, rest) = Box::pin(start(&orch, ids::HTUI_ANA_2)).await;

        assert_eq!(
            (rest.run, rest.position, rest.failure),
            (
                RunStatus::Failed,
                Some(0),
                Some(RunFailure::SecretsRefused {
                    cause: cause.clone()
                })
            )
        );
        let row = run_of(&orch, run).await;
        assert_eq!(
            (row.status, row.failure),
            (RunStatus::Failed, Some(cause.refusal()))
        );
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Failed, None),
                (1, StepStatus::Failed, None),
                (2, StepStatus::Failed, None),
            ]
        );
        assert!(
            steps.iter().all(|step| step.attempt == 1),
            "no retry: {steps:?}"
        );
        let notes = notes_of(&orch, ids::HTUI_ANA_2).await;
        for index in 0..3 {
            let expected = format!(
                "fan-out candidate {index} of `research` attempt 1: {}",
                cause.refusal()
            );
            assert!(notes.contains(&expected), "{expected}: {notes:?}");
            assert!(
                orch.spec_for(&research_key(index)).is_none(),
                "candidate {index} never started"
            );
        }
        assert_eq!(source.calls(), 1);
        assert_ne!(
            item_of(&orch, ids::HTUI_ANA_2).await.status,
            Status::Blocked,
            "`block = false`: the item reads its `failed` run"
        );
    }

    /// MOD-10 D13 (blueprint A-2, H-14): a judge that is its walk's first resolution and is
    /// refused fails the run with the sentence; it does not park the selection. The project is
    /// given a provider while candidate 2 is suspended, so the candidates ran provider-less.
    #[tokio::test]
    async fn a_judge_whose_secrets_refuse_fails_the_run_not_the_selection() {
        let orch = FakeOrchestrator::demo();
        fan_research(&orch, Gate::Never, false).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        let project = ana_2_project(&orch).await;
        let cause = htui_core::secret::SecretError::BadCredentials;
        let source = secret_source(
            &orch,
            htui_core::secret::fake::FakeSecretProvider::failing(cause.clone()),
        );
        let (stalled, wake) = orch.suspend_after_done("research", 1, Some((2, 0)));

        let (walked, ()) = tokio::join!(
            Box::pin(orch.dispatch(crate::command::Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: htui_core::model::RunMode::Manual,
                repo_scope: None,
                workspace: None,
            })),
            async {
                stalled.notified().await;
                orch.store().set_project_secret_columns(
                    project,
                    Some(htui_core::secret::INFISICAL),
                    Some(SECRET_SCOPE),
                );
                wake.notify_one();
            }
        );

        let walked = walked.expect("the walk settles the refusal");
        let crate::command::CommandOutcome::Started { run, rest } = walked else {
            panic!("`StartRun` answers `Started`, not {walked:?}");
        };
        let steps = steps_of(&orch, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1),
            [
                (0, StepStatus::Done, None),
                (1, StepStatus::Done, None),
                (2, StepStatus::Failed, None),
            ],
            "the suspended candidate wrote no document: two survivors, so the judge route"
        );
        assert_eq!(
            (rest.run, rest.failure),
            (
                RunStatus::Failed,
                Some(RunFailure::SecretsRefused {
                    cause: cause.clone()
                })
            )
        );
        let row = run_of(&orch, run).await;
        assert_eq!(
            (row.status, row.failure),
            (RunStatus::Failed, Some(cause.refusal()))
        );
        assert!(
            orch.spec_for(&crate::engine::SessionKey {
                phase: "research:judge",
                attempt: 1,
                fanout_index: -1,
                call: 0,
            })
            .is_none(),
            "the judge never started a session"
        );
        assert_eq!(source.calls(), 1, "the judge's entry is the one resolution");
        for index in 0..3 {
            let spec = orch
                .spec_for(&research_key(index))
                .unwrap_or_else(|| panic!("candidate {index} started"));
            assert!(spec.env.is_empty(), "candidate {index} ran provider-less");
        }
    }
}

/// Recovery paths no [`CASES`] entry reaches: an isolator that refuses the reset of a clean tree
/// (plan D93, D99, blueprint A-6) and a candidate whose artefacts all landed (plan D95, M4 D48).
///
/// Unit tests rather than cases: each seeds rows or scripts the fake isolator's answer, which no
/// transport-neutral binding promises.
#[cfg(test)]
mod recovery_paths {
    use htui_core::fixtures::ids;
    use htui_core::model::{
        CommandRunId, CommandRunStatus, Gate, Isolation, NewCommandRun, RunStatus, RunStepCommit,
        StepStatus, VerifyOutcome,
    };
    use htui_core::store::{ReadStore as _, StepFence, WriteStore as _};

    use super::{
        Adopted, FakeOrchestrator, Next, Orchestrate, Rest, RunFailure, candidate, crash,
        crash_divided, divided_feat_3, fan_research, free_feat_3, judge_both, notes_of, part_named,
        primary_repo, repoint, research_candidates, run_of, set_judge, slot_of, step_at, steps_of,
        tree_text,
    };
    use crate::isolate::Clock as _;
    use crate::verify::VERIFY_CLASS;

    /// §9.3's `refused` row: a clean `local` tree classifies resettable, but the isolator refuses
    /// the reset, so the step takes D93's path with the refusal as its reason — failed
    /// `interrupted, tree not reset` and the run parked, never retried on the unreset tree.
    #[tokio::test]
    async fn a_refused_reset_is_never_reset_with_the_refusal_as_its_reason() {
        let orch = FakeOrchestrator::demo();
        primary_repo(&orch).await;
        free_feat_3(&orch).await;
        repoint(&orch, ids::HTUI_FEAT_3, |phase| {
            if phase.name == "prd" {
                phase.isolation = Some(Isolation::Local);
            }
        })
        .await;
        let stalled = orch.stall_after_done("prd", 1, None, false);
        let run = crash(&orch, ids::HTUI_FEAT_3, &stalled).await;
        let prd = step_at(&orch, run, 0, 1).await;
        let trees = orch
            .store()
            .step_trees(prd.id)
            .await
            .expect("MemStore never fails a read");
        assert!(
            !trees.is_empty() && trees.iter().all(|tree| !tree.dirty),
            "stage 2 recorded a clean tree, so D92 would reset it"
        );
        let listed = tree_text(&orch, prd.id).await;

        let other = orch.restarted();
        let refusal = format!("local_moved: {} moved since step start", trees[0].path);
        other.isolator().script_reset_refusal(&refusal);
        let adopted = other.sweep().await.expect("the sweep adopts");
        assert_eq!(
            adopted,
            [Adopted {
                run,
                next: Next::Parked(Rest {
                    run: RunStatus::AwaitingApproval,
                    position: Some(0),
                    failure: Some(RunFailure::Interrupted {
                        phase: "prd".to_owned(),
                        reset: false,
                    }),
                }),
            }]
        );
        assert_eq!(other.isolator().resets(), 1, "the isolator was asked, once");
        let steps = steps_of(&other, run).await;
        let prd = super::at(&steps, 0, 1);
        assert_eq!(
            (prd.status, prd.gate_note.as_deref()),
            (StepStatus::Failed, Some("interrupted, tree not reset"))
        );
        assert!(
            steps.iter().all(|step| step.attempt == 1),
            "no attempt 2 on a tree the isolator refused to reset"
        );
        let n2 = format!(
            "interrupted, tree not reset: step {} (`prd` attempt 1); trees: {listed}; reason: \
             {refusal}; the run is parked for a human",
            prd.id
        );
        let notes = notes_of(&other, ids::HTUI_FEAT_3).await;
        assert!(notes.contains(&n2), "{n2}\n{notes:?}");
    }

    /// Plan D95 with M4 D48's settle: a `running` candidate whose document and every repo's
    /// `after_hash` landed is finished with the verify [`crate::recover::verify_of`] derived
    /// from its `command_run`, and moved `done` — not failed `interrupted`.
    #[tokio::test]
    async fn a_finished_candidate_is_settled_done() {
        let orch = FakeOrchestrator::demo();
        let repo = primary_repo(&orch).await;
        fan_research(&orch, Gate::Never, true).await;
        set_judge(&orch, ids::HTUI_ANA_2).await;
        research_candidates(&orch, 1);
        judge_both(&orch, "research", 1, 0, "complete");
        let stalled = orch.stall_after_done("research", 1, Some((1, 0)), true);
        let run = crash(&orch, ids::HTUI_ANA_2, &stalled).await;
        let steps = steps_of(&orch, run).await;
        let stuck = candidate(&steps, 0, 1, 1).clone();
        assert_eq!(stuck.status, StepStatus::Running, "the crash left it live");
        assert_eq!(stuck.finished_at, None, "the crash beat `finish_step`");
        let before = orch
            .store()
            .step_commits(stuck.id)
            .await
            .expect("MemStore never fails a read")
            .into_iter()
            .find(|row| row.repo_id == repo)
            .expect("stage 2 recorded the base")
            .before_hash;
        orch.store()
            .record_commits(
                StepFence::Lease(orch.owner()),
                stuck.id,
                &[RunStepCommit {
                    run_step_id: stuck.id,
                    repo_id: repo,
                    before_hash: before,
                    after_hash: Some("fake:after:candidate".to_owned()),
                }],
            )
            .await
            .expect("MemStore records the capture that landed");
        let now = orch.clock().now();
        orch.store()
            .record_command_run(NewCommandRun {
                id: CommandRunId::new(),
                run_step_id: stuck.id,
                box_id: ids::BOX,
                class: VERIFY_CLASS.to_owned(),
                command: "true".to_owned(),
                cwd: "/fake".to_owned(),
                status: CommandRunStatus::Done,
                exit_code: Some(0),
                output: Some(String::new()),
                queued_at: now,
                started_at: Some(now),
                finished_at: Some(now),
            })
            .await
            .expect("MemStore records the verify that ran");

        let other = orch.restarted();
        let adopted = other.sweep().await.expect("the sweep adopts");
        assert_eq!(
            adopted,
            [Adopted {
                run,
                next: Next::Walk
            }]
        );
        let steps = steps_of(&other, run).await;
        assert_eq!(
            slot_of(&steps, 0, 1)
                .into_iter()
                .map(|(index, status, _)| (index, status))
                .collect::<Vec<_>>(),
            [
                (0, StepStatus::Done),
                (1, StepStatus::Done),
                (2, StepStatus::Done),
            ]
        );
        let settled = candidate(&steps, 0, 1, 1);
        assert_eq!(
            (
                settled.verify_outcome,
                settled.verify_exit_code,
                settled.gate_note.as_deref()
            ),
            (Some(VerifyOutcome::Pass), Some(0), None),
            "M4 D48: the derived verify is recorded, and nothing interrupted it"
        );
        assert!(settled.finished_at.is_some(), "plan D90's `finish_step`");
        assert_eq!(
            other.isolator().resets(),
            0,
            "D95: no reset for a candidate"
        );
    }

    /// Ruling F-1 beside a pending sibling: a part crashed without its output, with
    /// `retry_limit 0`, spends its one attempt on recovery, so the divided step is exhausted
    /// and the run fails `part_exhausted` (D131's branch). The sibling that never started is
    /// cut with the run, `cancelled` with the cut's note, never left `pending` under a terminal
    /// run.
    #[tokio::test]
    async fn an_exhausted_part_on_recovery_cuts_its_pending_sibling() {
        let orch = FakeOrchestrator::demo();
        divided_feat_3(&orch, &[("api", None), ("ui", None)], |phase| {
            phase.retry_limit = 0;
        })
        .await;
        orch.store()
            .set_app_setting("max_parallel_sessions", serde_json::json!(1));
        let stalled = orch.stall_after_done("implement/api", 1, None, false);
        let run = crash_divided(&orch, &[stalled]).await;

        let other = orch.restarted();
        let adopted = other.sweep().await.expect("the sweep adopts");
        assert_eq!(adopted.len(), 1, "{adopted:?}");
        assert!(
            matches!(
                &adopted[0].next,
                Next::Finished(Rest {
                    run: RunStatus::Failed,
                    failure: Some(RunFailure::PartExhausted { node, part, attempts: 1 }),
                    ..
                }) if node == "implement" && part == "api"
            ),
            "{adopted:?}"
        );
        let row = run_of(&other, run).await;
        assert_eq!(row.status, RunStatus::Failed);
        let steps = steps_of(&other, run).await;
        let api = part_named(&steps, "implement/api");
        assert_eq!(
            (api.attempt, api.status, api.gate_note.as_deref()),
            (1, StepStatus::Failed, Some("interrupted"))
        );
        let ui = part_named(&steps, "implement/ui");
        assert_eq!(
            (ui.attempt, ui.status),
            (1, StepStatus::Cancelled),
            "the never-started sibling is cut with the run"
        );
        let item = row.item_id.expect("the divided run is an item's");
        assert!(
            notes_of(&other, item).await.iter().any(|note| note
                == "step `implement/ui` attempt 1 cancelled: the run ended at `implement`"),
            "the cut's note names the divided node"
        );
    }
}

/// MOD-138 M3 T3: engine paths around concurrency no [`CASES`] entry reaches.
#[cfg(test)]
mod concurrency_paths {
    use htui_core::fixtures::ids;
    use htui_core::model::{EdgeKind, GraphSnapshot, RunStatus, StepStatus};

    use super::{
        FakeOrchestrator, Orchestrate, Resume, approve_without_walking, author_edges, item_of,
        notes_of, start, step_shape, stored_v,
    };

    /// T2 repair 1, pinned at the engine (MOD-138 M3 D7, M2 D11): an in-flight **v1** run of a
    /// `local` project whose live graph became a diamond. The live graph now refuses at resolve
    /// (`LocalConcurrent`), which `resolve_or_refused` answers as `Resolution::Refused` with the
    /// live phases; they still hash to the run's own v1 digest, so `resume` walks the run's lift
    /// on, as under M2, and writes no topology note.
    #[tokio::test]
    async fn a_v1_run_whose_live_graph_became_a_local_diamond_walks_on() {
        let orch = FakeOrchestrator::demo();
        super::free_feat_3(&orch).await;
        let project = item_of(&orch, ids::HTUI_FEAT_3).await.project_id;
        orch.store()
            .set_project_settings(project, serde_json::json!({"default_isolation": "local"}));
        let (run, rest) = start(&orch, ids::HTUI_FEAT_3).await;
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(0)),
            "a `local` chain starts and parks at `prd`'s gate"
        );
        crate::fake::rewrite_as_v1(orch.store(), run).await;
        assert_eq!(stored_v(&orch, run).await, GraphSnapshot::V1);
        approve_without_walking(&orch, run, ids::HTUI_FEAT_3).await;
        author_edges(
            &orch,
            ids::HTUI_FEAT_3,
            &[
                ("prd", "plan", EdgeKind::Next, None, None),
                ("prd", "implement", EdgeKind::Next, None, None),
                ("plan", "review", EdgeKind::Next, None, None),
                ("implement", "review", EdgeKind::Next, None, None),
            ],
        )
        .await;
        let live = crate::graph::resolve_or_refused(
            orch.store(),
            &orch.graphs(),
            &item_of(&orch, ids::HTUI_FEAT_3).await,
            htui_core::model::RunMode::Manual,
            &std::collections::BTreeMap::new(),
            None,
            ids::BOX,
        )
        .await
        .expect("the live graph's phases build");
        assert!(
            matches!(
                live,
                crate::graph::Resolution::Refused {
                    refusal: crate::graph::ResolveError::LocalConcurrent { .. },
                    ..
                }
            ),
            "the live graph is a `local` diamond, refused at resolve"
        );

        let resumed = orch.resume(run).await.expect("the run is readable");
        let Resume::Walked(rest) = resumed else {
            panic!("a refused live graph whose phases did not move is walked: {resumed:?}");
        };
        assert_eq!(
            (rest.run, rest.position),
            (RunStatus::AwaitingApproval, Some(1)),
            "the run walks its own lift: position 1 ran and parked at its gate"
        );
        assert_eq!(
            step_shape(&orch, run).await,
            [
                (0, 1, StepStatus::Done),
                (1, 1, StepStatus::AwaitingApproval),
            ]
        );
        let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
        assert!(
            !notes
                .iter()
                .any(|body| body.starts_with("topology mismatch")
                    || body.starts_with("live graph not comparable")),
            "{notes:?}"
        );
    }
}
