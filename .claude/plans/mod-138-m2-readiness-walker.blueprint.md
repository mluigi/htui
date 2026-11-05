# Blueprint: MOD-138 M2, the readiness walker (T1 to T5)

> Written by `code-architect` on 2026-11-05 from the confirmed plan
> `.claude/plans/mod-138-m2-readiness-walker.plan.md` (D1 to D12, Q-A to Q-D, V1 to V23), the PRD
> `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md` (milestone 2) and `docs/ANA-33.md` §7.3 and §7.7.
> The tree was read at `31131717` (branch `hr/MOD-138`, `38949409` plus the plan commit).
>
> The plan stays the spec. §0 lists every place where the tree disagrees with the plan or where the plan says nothing:
> - rows marked **adopted** are implementation detail or a corrected claim;
> - rows marked **A1 to A9** need the maintainer, and are collected with the smallest fix for each under "Plan amendments needed" at the end.
>
> Nothing was probed against Postgres. §4's two SQL items marked *probe* are T1's first job.

## 0. Plan claims contradicted or extended by the tree

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | **T1 cannot add `RunStep.branch` without the column.** Three PgStore statements are `sqlx::query_as!(RunStep, …)`: `run_steps`, the waiting-candidates read and `create_step … RETURNING`. `query_as!` builds a struct literal from the selected columns, so a new struct field that no statement selects fails to compile (E0063). T1's file list has no `pg/*.rs`, no migration and no `.sqlx`, so T1's own workspace check fails. | `pg/read.rs:745-781`, `:1405-1437`; `pg/write.rs:6961-7000` | **A1**: T1 owns the whole column: migration `0025`, the three statements, cache `0008` with its read and sync, `.sqlx`, the migration pins, the migration test and the mirror test. T2 owns the verb only. T2 ∩ T3 stays empty (§5). |
| F2 | **Q-B and D8 contradict each other on "accept artifact".** §4.8's accept is `Command::AcceptArtifact`. It ends in `answer_guarded(.., GateAnswer::Approved)`, so passing it "through unchanged" (D8) records `approved` and *takes* every `asks_you` edge, while Q-B says accept *declines* them. `GateAnswer::Skipped` is never sent by any UI. | `engine.rs:1620`, `:1741`; `command.rs:145-162`; `git grep GateAnswer::Skipped` finds tests only | **A2**: `accept_artifact` answers `Skipped` on a node with any outgoing `asks_you` edge. Q-B wins. |
| F3 | **A human reject is ambiguous on a node with an `asks_you` edge and also a `changes_requested` or `fails` edge.** D7 loops it, D8 declines it. Both sites are in `rejection`/`answer_guarded`. | D7, D8 text; `engine.rs:956-1027`, `:1045-1103` | **A3**: precedence `changes_requested` (loop) > `asks_you` (decline, mapped to `Skipped`) > `fails` (loop) > end the run `rejected: <phase>`. |
| F4 | **A loop whose target has no row has to stay `NoTarget`.** If the third `NoTarget` site (`latest_at(target)` is `None`) is dropped, as Q-D's wording implies, the target's attempt reads as 0 and the budget `may_attempt(1, ..)` holds. On a branching graph a *skipped* target is never re-run, because its in-edges are outside the region. So every turn re-runs the rest of the region, the target attempt never grows, the no-progress check needs `attempt >= 2` and never fires, and the loop never ends. | `gate.rs:776-787`; D7 "Region"; Q-D bullet 2 | **A4**: `NoTarget` (`no_loop_target`) when the edge names a missing key **or** when the target has no row (it never ran: unreached or skipped). Q-D's rewrite of `review_loop.rs` keeps that intent (§T4.6). |
| F5 | **A back `asks_you` loop that stops on "no progress" strands the run.** The plan checks `commits_are_identical` inside `edge_loop`, which runs at the `Loop` cursor. By then the source is `done` + `approved`. An escalation parks the run, but `ready()` still yields `Loop` (the budget holds), so every Unblock escalates again. `retry` refuses a `done` step (`retry_enabled`), so cancel is the only way out. `ready()` is pure and cannot read commits. | `command.rs:775-800`; D7 bullet "Back `asks_you`" | **A5**: a back `asks_you` loop is bounded by its budget alone, because a human approves every turn. `edge_loop` skips the no-progress test for `AsksYou`. The stop and `ready()` share one pure budget predicate (`walk::loop_budget_left`). |
| F6 | **A crash part-way through retirement can loop twice.** `ready()` in position order puts `Create(target)`, the target having just been retired, before `Loop` at the source's higher position. The engine then re-creates the target while the source is still `done` + `approved`, and later loops again. Today's linear review loop has the same hazard, but parity forbids changing it there. | `gate.rs:972-983` (ascending retirement) | **A6** (new variants only): `Decide` and `Loop` sort **before** every node cursor. They are writes the engine owes, not work. `edge_loop` retires the region in forward-topological order, target first and **source last**, so `Loop` keeps showing until the region is fully retired. No lifted graph has either variant, so parity is untouched. |
| F7 | **Fan-out bypasses D8 and the `fails` sites.** A `fan_out > 1` node is routed by `select_stage`/`route(gate_effective, ..)` and never runs `gate::apply` on its winner. Its winner's `gate_outcome` is therefore never `approved`, so every forward `asks_you` edge would be declined in silence. A group failure (`NoSurvivingCandidate`) never reaches a `fails` edge. | `engine.rs:4766`, `:4804`; `fanout.rs:216` | **A7**: resolve refuses a fan-out node with an outgoing `asks_you` or `fails` edge, as `NotYetRunnable { reason: NotRunnable::FanOutEdge }` (M3 or MOD-139 lifts it). D10's widened `ReviewFanOut` covers `changes_requested`. |
| F8 | `settle_failed`'s budget branch also serves interrupted steps (`gate_note` `interrupted…`). A `fails` loop there would loop on a crash rather than on a failure of the node itself. | `engine.rs:2951-3001`; `command.rs` `interrupted()` | **adopted**: the `fails` branch fires only when `!note.starts_with(INTERRUPTED)`, which also excludes `NOT_RESET`. It sits right after the `rejected` branch. `reset_interrupted` (D92) keeps `retry_limit`. |
| F9 | `tests/review_loop.rs` has **four** cases, not three, and all of them call `gate::review_loop`, which D7 removes. Two more tests lose their subject:<br>- `gate.rs` `the_loop_target_is_the_named_implement_phase` (`loop_target` is removed);<br>- `graph.rs` `non_default_edges_are_refused` (D10 lifts the refusal). | `review_loop.rs:177,236,289,334`; `gate.rs:1508-1525`; `graph.rs:5011-5045` | **A8**: three `review_loop.rs` call sites become `edge_loop(.., &edge)` with their expectations unchanged. The Q-D case is rewritten (A4). The two named unit tests are replaced (§T3, §T4). The Acceptance line "the only exception" grows to these three. |
| F10 | Every PgStore read that builds a `RunStep` is one of F1's three. The Runs-summary reads (`StepRow`, `pg/read.rs:~430`, and the cache summary at `cache/read.rs:690-760`) build a `RunStepSummary`, which has no `branch`, and stay unchanged. The mirror's sync is `cache/refresh.rs` (`RUN_STEP_COLUMNS`, `refresh_run_step`'s `query!` and its binds), which the plan's file list leaves out. `pg/demo.rs:495` seeds `run_step` with a column list and needs nothing, since the column is nullable. | as cited | **adopted** (§4). |
| F11 | **Migration pins.** There are nine numeric pins plus their prose: `tests/migrations.rs` `:115` (the version list), `:3905`, `:4006`, `:4015`, `:4038`, `:4190`; `tests/connect.rs` `:140`, `:156`, `:242`. `TABLES` does not move (no new table). The comment-contract test covers `run_step` (`migrations.rs:1052-1133`), so a `COMMENT ON COLUMN` would need a new `MOD138_COLUMN_COMMENTS` const. | as cited | **adopted**: no comment (the `0014` precedent), so seven pins move from 24 to 25 plus the list. `HANDOFF.md` records `0025` and `0008` as taken. |
| F12 | `SkillGlob`'s bare glob matches in **any** repo (`skill_glob.rs:1-5`). D3 says a bare glob means the **primary** repo (§4.7). The facts must therefore carry the primary repo's name. | `skill_glob.rs:199-213` | **adopted**: `BranchFacts.primary`. Parsing reuses `SkillGlob::parse` and `skill_glob::matcher`; `first_match` is not used. `SkillGlob::compiled` becomes `pub(crate)`. |
| F13 | `GixIsolator::changed_paths` answers `Ok(empty)` when `git` is unusable, so "undecidable" reads as "nothing changed". | `isolate/real.rs:1538-1565` | **adopted**: `decide` calls the isolator only when some `if` predicate of the node `needs_paths()`. The residual (no `git` means the path predicates are false) is documented in Risks. |
| F14 | `pass_step` writes `gate_outcome = 'skipped'` on every automatic pass. "Skipped" therefore means both "auto-passed" and (Q-B) "declined". | `traits.rs:1856-1869`; `pg/write.rs` `pass_step` | **adopted**: this is safe because D8 forces every `asks_you` source through a park. `land_recovered` uses the same `gate::apply`, so recovery is covered. Pinned by T5's auto-mode case. |
| F15 | `Cursor` derives `Eq`, and `SnapshotEdge` does not (`run.rs` derives `PartialEq` only). | `status.rs:197`; `run.rs` `SnapshotEdge` | **adopted**: `Loop { step, to: String }`. The edge is the `(step's phase, to, AsksYou)` triple, unique by MOD-137 rule (c). |
| F16 | `LoopOutcome::Escalated` is built literally by `review_loop.rs` case 2, so adding a field breaks it. | `review_loop.rs:242` | **adopted**: no new field. `gate::loop_failure(edge, attempts)` gives the `RunFailure`. |
| F17 | The widened `ReviewFanOut` needs edges, but `snapshot_phase` runs per row before any edge is built. | `graph.rs:1438`; `:455-470` | **adopted**: a new `fan_out_edge_refusal(&phases, &edges)` runs after `snapshot_walk_refusal`. `ReviewFanOut` is a plain `Err`, so resume keeps it in the "not comparable" arm. The name check in `snapshot_phase` stays. |
| F18 | `htui_orch::walk` lands beside crate-root re-exports of `select::walk` (a fn) and `Walk` (a type), `lib.rs:77`. This is legal (different namespaces) but easy to misread. | `lib.rs:77` | **adopted**: no type in `walk.rs` is called `Walk`. `engine.rs` and `gate.rs` call `crate::walk::…` by full path. |
| F19 | The V18 count ("47 literal sites") includes `-> RunStep {` signatures. The real struct literals number fewer than 30. | `git grep '\bRunStep \{'` | **adopted**: the sweep follows the compiler, with `cargo check --workspace --all-targets --all-features`. |
| F20 | `reviews_are_identical` is not a no-op for a `fails` loop entered through a human's reject: those source rows are `rejected`. This is harmless (the bodies are compared as for a review). | `gate.rs:923-965` | **adopted**: doc only. |
| F21 | A node can carry two loop edges of one kind. | MOD-137 rule (c) is unique per `(from, to, kind)` only | **adopted**: the first in `sort_snapshot_edges` order, documented on `Graph::loop_edge`. |
| F22 | The budget-spent `asks_you` stop blocks the item (D7), but `answer_gate` has no item guard. A human can approve over a `blocked` item, and the run then walks while the item stays blocked. | `command.rs:728-750` | **A9**: the `asks_you` escalation writes the note and parks, but **does not** block the item. The stop is already the checkpoint. |
| F23 | Out of M2 scope, recorded for M3 or MOD-141:<br>- `recover::frontier` (Q-C);<br>- `GraphSnapshot::last_output_kind` (close-out's default resolution, by highest position). | `recover.rs:365-377`; `run.rs:631-648` | Both are wrong only on a branching graph, which only tests can store before MOD-141. |

---

## 1. Shared decisions for every task

### 1.1 Where things live

| Item | File | Task |
|---|---|---|
| `IfPredicate`, `PathGlob`, `BranchFacts`, `PredicateError` | `crates/htui-core/src/model/predicate.rs` (new), `pub mod predicate` + re-export in `model/mod.rs` | T1 |
| `GraphSnapshot::walk_edges`, `GraphSnapshot::reads_verdict`, `RunStep.branch` | `crates/htui-core/src/model/run.rs` | T1 |
| `forward_mask`, W4 in `graph_body_walk_refusal` | `crates/htui-core/src/store/graph_rules.rs`, re-exported in `store/mod.rs` | T1 |
| W4 sentence `if_predicate_outside_the_closed_set` | `traits.rs`, new `// ---- MOD-138 M2 ----` block after M1's | T1 |
| `record_branch` + `branch_is_not_an_object_of_booleans` | `traits.rs` (WriteStore), `store/worker.rs` (WorkerStore + MemStore fwd), `mem.rs`, `pg/write.rs`, `htui-store/src/{worker.rs,writer.rs}`, htui-agent spies | T2 |
| `walk` module: `NodeState`, `EdgeState`, `Reach`, `Graph`, `ready`, `loop_budget_left`, `required_inputs` | `crates/htui-orch/src/walk.rs` (new); `lib.rs`: `pub mod walk;`, `pub use walk::required_inputs` (moved out of the `engine::` list) | T3 |
| `Cursor::{Decide, Loop}`, `cursor` wrapper, `cursor_by_position` (`#[cfg(test)]`), `group_cursor` `pub(crate)` | `status.rs` | T3 |
| `RunFailure::LoopExhausted` | `status.rs` | T4 |
| `edge_loop`, `loop_failure`, `escalate` generalised; `review_loop`, `loop_target`, `IMPLEMENT_PHASE` removed; `SettleInput.is_review` → `reads_verdict` | `gate.rs` | T4 |
| `asks_you_stop` and D8's arm in `apply` | `gate.rs` | T5 |
| `decide`, `loop_back`, D8 in `answer_guarded`, A2 in `accept_artifact` | `engine.rs` | T5 |

### 1.2 htui-core types (T1)

```rust
// model/predicate.rs
//! MOD-138 M2 D3 (ANA-33 §7.3): the closed `if` set, parsed from the opaque `EdgePredicate`.

/// One `repo:glob` (§4.7): `repo: None` is a bare glob, read against the **primary** repo (F12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGlob { pub repo: Option<String>, pub glob: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyWant { Passed, Failed }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IfPredicate {
    AllChangedPathsMatch(Vec<PathGlob>),   // {"all_changed_paths_match": ["..", ..]}
    AnyChangedPathMatches(Vec<PathGlob>),  // {"any_changed_path_matches": [..]}
    Verify(VerifyWant),                    // {"verify": "passed" | "failed"}
    ItemKind(String),                      // {"item_kind": "<key_prefix>"}
    ItemTag(String),                       // {"item_tag": "<tag>"}
    All(Vec<IfPredicate>), Any(Vec<IfPredicate>), Not(Box<IfPredicate>),
}

/// What a decision reads (Q-A). Paths are `(repo name, repo-relative path)`, in the run's scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchFacts {
    pub paths: Vec<(String, String)>,
    pub primary: Option<String>,       // F12: the primary repo's name when it is in scope
    pub truncated: bool,
    pub verify: Option<VerifyOutcome>,
    pub item_prefix: String,           // item.key_prefix
    pub item_tags: Vec<String>,        // item.required_tags (V13)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PredicateError {
    #[error("a predicate is a JSON object with exactly one key")]            NotOneKey,
    #[error("`{0}` is not in the closed set (all_changed_paths_match, any_changed_path_matches, verify, item_kind, item_tag, all, any, not)")]
                                                                             Unknown(String),
    #[error("`{key}` takes {want}")]                                         BadValue { key: &'static str, want: &'static str },
    #[error(transparent)]                                                    Glob(#[from] GlobError),
    #[error("a predicate nests deeper than {MAX_DEPTH}")]                    TooDeep,
}
pub const MAX_DEPTH: usize = 32;

impl IfPredicate {
    /// # Errors
    /// [`PredicateError`]: the first refusal, depth first, in array order.
    pub fn parse(value: &serde_json::Value) -> Result<Self, PredicateError>;
    /// Every glob list must be non-empty, as must `all` and `any`; `item_kind` and `item_tag`
    /// take non-empty strings; globs use `SkillGlob::parse` (trimmed).
    #[must_use] pub fn evaluate(&self, facts: &BranchFacts) -> bool;
    /// Whether any leaf is a path predicate, so whether `changed_paths` must be read (F13).
    #[must_use] pub fn needs_paths(&self) -> bool;
}
```

Evaluation rules (D3):
- **`all_changed_paths_match`** is true when `paths` is not empty, `truncated` is false, and every path is matched by some glob of the list in that path's repo.
- **`any_changed_path_matches`** is true when some listed path is matched.
- **Repo of a glob.** A glob matches in its own repo. A bare glob matches in `primary`, and in no repo when `primary` is `None`.
- **Matching** uses `Candidate::from_bytes`, so there is no case folding.
- **`verify`**: `"passed"` is `Some(Pass)` and `"failed"` is `Some(Fail)`. `unavailable` or `None` matches neither.
- **`item_kind`** compares bytes with `item_prefix`.
- **`item_tag`** is true when `item_tags` contains the value.

`model/run.rs`:

```rust
// RunStep, appended after `promoted_at` (column order), before `updated_at`:
    /// `run_step.branch` (MOD-138 M2 D5): `{"<to key>": bool}` per `if` edge, written once by
    /// `record_branch`; `None` until decided and on every node with no `if` edge.
    #[serde(default)]
    pub branch: Option<Value>,

impl GraphSnapshot {
    /// MOD-138 M2 D1: the edges every walker reads: the stored ones, or for a `v == V1` blob
    /// holding none (decoded by serde, not through `snapshot_of`) the lift, computed here. A v2
    /// snapshot with no edges is **not** lifted: its nodes are roots.
    #[must_use] pub fn walk_edges(&self) -> Cow<'_, [SnapshotEdge]>;
    /// MOD-138 M2 D7: `node` reads its verdict iff it has an outgoing `changes_requested` edge.
    #[must_use] pub fn reads_verdict(&self, node: &str) -> bool;
}
```

`store/graph_rules.rs`:

```rust
/// MOD-138 M2 D2: per edge, whether it is forward: `next`, `if`, and an `asks_you` whose target
/// does not reach its source over `next`/`if` (W2, F18 of M1). An edge on a palette node, or
/// naming an index outside `nodes`, is `false`. `walk_refusal` is rewritten over it, so the
/// validator and the walker cannot disagree.
#[must_use]
pub fn forward_mask(nodes: &[WalkNode<'_>], edges: &[WalkEdge]) -> Vec<bool>;
```

`graph_body_walk_refusal` gains **W4**. It fires after W1 to W3, per body edge in order, and only when `kind == If` and `edge_predicate(edge)` is `Some`, because rule (b) refuses a missing one first. `IfPredicate::parse(&p.0)` failing gives:

```rust
// traits.rs, new MOD-138 M2 block
pub fn if_predicate_outside_the_closed_set(from: &str, to: &str, graph: StepGraphId, reason: &str) -> String
// "the `if` edge from `{from}` to `{to}` in graph `{graph}` has a predicate outside the closed set: {reason}"
pub fn branch_is_not_an_object_of_booleans(step: StepId) -> String          // T2
// "run_step {step}: branch must be a JSON object whose every member is true or false"
```

### 1.3 The store verb (T2)

```rust
/// MOD-138 M2 D5 (Q-A): `run_step.branch = branch`, the `if` decision of a done step; written
/// once, under `fence`, as `pass_step` is. `updated_at` moves (the trigger; MemStore: `now`).
///
/// # Errors
/// In this order: `Constraint` (`branch_is_not_an_object_of_booleans`) when `branch` is not an
/// object whose every member is a boolean, judged before any read; `NotFound { "run_step" }`;
/// `Fenced`. `Ok(false)`: the step is not `done`, or its `branch` is already set; nothing written.
async fn record_branch(&self, fence: StepFence, step: StepId, branch: &Value) -> Result<bool>;
```

It has eight implementations, all next to `record_commits` (V17):
- `traits.rs` (WriteStore);
- `store/worker.rs` (WorkerStore declaration, and the MemStore forward at `:879`);
- `mem.rs`: `State::record_branch` beside `pass_step` at `:6678`, then the WriteStore impl near `:10108`;
- `pg/write.rs`;
- `htui-store/src/worker.rs` `:301` and `:691`;
- `writer.rs` `:1228` (`Memory`/`Online`);
- `htui-agent/src/conformance.rs:1317`;
- `htui-agent/tests/recorder.rs:1043` (both forward to `inner`).

PgStore uses `pass_step`'s one-statement shape, then `fenced_miss`:

```sql
WITH locked AS (
    SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id
     WHERE s.id = $1 AND s.status = 'done' AND s.branch IS NULL
       AND r.lease_owner IS NOT DISTINCT FROM $3
       FOR NO KEY UPDATE OF s FOR SHARE OF r)
UPDATE run_step SET branch = $2 FROM locked
 WHERE run_step.id = locked.id AND run_step.branch IS NULL
```

If `rows_affected` is 1 the answer is `Ok(true)`. Otherwise it is `Box::pin(fenced_miss(&self.pool, step, fence))`.

### 1.4 htui-orch: the walker (T3)

```rust
// walk.rs
//! MOD-138 M2 D2 (ANA-33 §7.3): the readiness walker, pure, re-derived from rows (plan D16).

/// A node's latest rows, by `status.rs`'s rules moved here, not rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeState<'s> {
    /// `fan_out = 1`: `latest_at` is `done`. `fan_out > 1`: `group_cursor` answered `None`
    /// (a `selected` `done` candidate); the winner is `status::winner_at` of that slot.
    Done(&'s RunStep),
    /// `Run`, `Rest`, `Fan` or `Select`.
    Live(Cursor),
    /// The latest slot is retired: `Create` at `attempt` (`next_attempt`, or the group's + 1).
    Retired { attempt: i32 },
    /// No row: `Create` at 1.
    Absent,
}
#[must_use] pub fn node_state<'s>(phase: &SnapshotPhase, steps: &'s [RunStep]) -> NodeState<'s>;

/// D2's edge table for one forward in-edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum EdgeState { Unresolved, Taken, Declined }
/// A node once its forward in-edges are read. `Done` wins over everything ("rows win").
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Reach { Done, Reached, Skipped, Waiting }

/// The snapshot as the walker reads it: `walk_edges()`, the non-palette nodes, the forward mask
/// (`graph_rules::forward_mask`, so it cannot disagree with the validator), and a forward
/// topological order (Kahn, ties by `position`); `None` on a forward cycle.
pub struct Graph<'a> { /* snapshot, edges: Cow<'a, [SnapshotEdge]>, nodes, ends, forward, topo */ }

impl<'a> Graph<'a> {
    #[must_use] pub fn of(snapshot: &'a GraphSnapshot) -> Self;
    #[must_use] pub fn edges(&self) -> &[SnapshotEdge];
    /// The first `kind` edge out of `source` in snapshot edge order (F21); for
    /// `ChangesRequested` and `Fails`.
    #[must_use] pub fn loop_edge(&self, source: &str, kind: EdgeKind) -> Option<&SnapshotEdge>;
    /// `asks_you` edges out of `source`, split by the mask.
    pub fn forward_asks_you(&self, source: &str) -> impl Iterator<Item = &SnapshotEdge>;
    pub fn back_asks_you(&self, source: &str) -> impl Iterator<Item = &SnapshotEdge>;
    pub fn if_edges(&self, source: &str) -> impl Iterator<Item = &SnapshotEdge>;
    /// D7's region: every node on a forward path from `target` to `source`, both included
    /// (forward-reach(target) ∩ backward-reach(source)), as positions in forward-topological
    /// order: target first, source last (A6). Empty when either key is unknown.
    #[must_use] pub fn region(&self, target: &str, source: &str) -> Vec<i32>;
    /// D9: nodes reachable from `node` over forward edges, `node` excluded.
    #[must_use] pub fn reachable_from(&self, node: &str) -> Vec<&'a SnapshotPhase>;
}

/// D2. Never empty. Order (A6): every `Decide`, then every `Loop`, then each reached, non-done
/// node's cursor (`Absent` → `Create{p,1}`, `Retired{a}` → `Create{p,a}`, `Live(c)` → `c`), each
/// group in position order; `[Finished]` when every walked node is `Done` or `Skipped` and
/// nothing is owed.
/// - `Decide { step: winner }`: a `Done` node with ≥1 `if` edge and `winner.branch == None`.
/// - `Loop { step: winner, to }`: a `Done` node whose winner is `approved`, for its **first**
///   back `asks_you` edge with `loop_budget_left`.
/// - Edge resolution, source by its `Reach`: `Done` → `next` Taken; `if` Taken iff
///   `branch[to] == true`, Declined for `false` **or a missing key**, Unresolved for `branch ==
///   None`; forward `asks_you` Taken iff `gate_outcome == Approved`, else Declined. `Skipped` →
///   Declined. Otherwise Unresolved.
/// - Reach, in topological order: `Done` from rows; else a node with no forward in-edge is
///   `Reached`; all in-edges resolved → `Reached` if any `Taken`, else `Skipped`; else `Waiting`.
///   Non-done rows of a `Skipped` node are ignored (only `done` rows win).
/// - Fallback, reachable only on a forward cycle (the validator refuses one, so only a
///   hand-built snapshot): the first node in position order that is neither `Done` nor
///   `Skipped`, read as reached. That is the old position rule, and it keeps `ready` total.
#[must_use] pub fn ready(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Vec<Cursor>;

/// D7's budget, pure, shared by `ready`'s `Loop`, the `asks_you` stop and `edge_loop`:
/// `latest_at(target)` exists and `may_attempt(attempt + 1, edge.loop_limit.unwrap_or(
/// target.retry_limit))`. Unknown target or no row: `false`.
#[must_use] pub fn loop_budget_left(snapshot: &GraphSnapshot, steps: &[RunStep], edge: &SnapshotEdge) -> bool;

/// D9 (moved from `engine.rs:7504`): a kind is optional iff a node reachable from `phase` over
/// forward edges produces it. A `phase` the snapshot does not name (a judge phase, whose
/// `input_kinds` are empty anyway) reaches nothing.
#[must_use] pub fn required_inputs(phase: &SnapshotPhase, snapshot: &GraphSnapshot) -> Vec<String>;
```

`status.rs` (T3):

```rust
pub enum Cursor {
    Create { .. }, Run(StepId), Rest { .. }, Fan { .. }, Select { .. },
    /// MOD-138 M2 D6: `step` is `done` and its node's `if` edges are undecided.
    Decide { step: StepId },
    /// MOD-138 M2 D7: `step` is `done` + `approved` and its back `asks_you` edge to `to` has budget.
    Loop { step: StepId, to: String },
    Finished,
}
#[must_use] pub fn cursor(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor {
    crate::walk::ready(snapshot, steps).into_iter().next().unwrap_or(Cursor::Finished) // never empty
}
#[cfg(test)] pub(crate) fn cursor_by_position(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor; // today's body, verbatim
pub(crate) fn group_cursor(..) -> Option<Cursor>;  // was private
pub fn resumable_park(c: &Cursor) -> bool { matches!(c, Create{..} | Run(_) | Decide{..} | Loop{..} | Finished) }
```

`RunFailure` (T4):

```rust
/// MOD-138 M2 D7: a `fails` or `asks_you` loop ran out of budget (or, for `fails`, made no progress).
LoopExhausted { kind: EdgeKind, from: String, to: String, attempts: i32 },
// Display: "{kind} loop exhausted after {attempts} attempts: `{from}` to `{to}`"
```

**Every `Cursor` match site.** Only two are exhaustive:
- `engine.rs` `run_to_rest:3068` gets two arms: T3 placeholders, then T4 or T5 real bodies.
- `engine.rs` `resting:6964`: `Decide { step } | Loop { step, .. }` resolve the row's position.

The rest are not exhaustive and need no change:
- `recover_run:2672` (`let Cursor::Rest{Failed}`);
- `status.rs` `crashed_rejection:385` (`let`) and `resumable_park` (gains the two variants);
- `htui-worker/src/views.rs:744` (`_ => None`);
- `views.rs:475`, `engine.rs:2353` and `:2544` (call `resumable`);
- `command.rs:2382` (a test literal).

No `htui` (TUI) code matches on `Cursor`.

### 1.5 gate.rs (T4, T5)

```rust
pub struct SettleInput<'a> { …, pub reads_verdict: bool }   // renamed; `settle`'s `if !input.is_review`

/// D7: one loop over `edge` (`changes_requested`, `fails` or a back `asks_you`) from `source`.
/// 1. `target` = the phase whose `name == edge.to`, else NoTarget; `latest_at(target)` is a row,
///    else NoTarget (A4). Each `NoTarget` first writes `finish_run(Failed, "no_loop_target")`.
/// 2. Unless `edge.kind == AsksYou` (A5): `no_progress(ctx, &steps, target.position,
///    source.position, attempt)` → escalate with that reason.
/// 3. `!loop_budget_left` → escalate `Exhausted`.
/// 4. `retire` over `Graph::of(snapshot).region(&edge.to, &source.phase_name)` (target first,
///    source last), each position through `retire_slot` (status split unchanged).
/// 5. `Resumed { position: target.position, attempt: attempt + 1 }`.
pub async fn edge_loop<S: WorkerStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>, source: &RunStep, edge: &SnapshotEdge,
) -> Result<LoopOutcome, EngineError>;

/// `ReviewLoopExhausted(attempts)` for `changes_requested` (bytes kept); `LoopExhausted` otherwise.
#[must_use] pub fn loop_failure(edge: &SnapshotEdge, attempts: i32) -> RunFailure;

// escalate(ctx, source, target, edge, attempts, reason, now): run CAS, item open→blocked, note
// "{loop_failure}: phase `{target.name}`, attempt {attempts}, stop reason `{reason}`"
// (byte-identical to today for changes_requested).
```

`apply` (T4):
- **`(Never, Failed(f))`**: when `Graph::of(ctx.snapshot).loop_edge(&phase.name, Fails)` is `Some(e)`, write `note_step`, then `move_step(Running → Failed)`, then `reread`, then `Box::pin(edge_loop(ctx, &row, e))`, then `landing(outcome, e, &row)`. Otherwise `retry_or_fail`, as today.
- **`(Never, Rejected)`**: as today up to `reread`, then `edge_loop` over the `changes_requested` edge. Without one, `NoTarget` (unreachable, since `Rejected` implies `reads_verdict`).
- **`fn landing`**:
  - `Resumed` maps to `Landing::Retired`.
  - `Escalated { attempts, .. }` maps to `Rest { AwaitingApproval, Some(source.position), Some(loop_failure(e, attempts)) }`.
  - `NoTarget` maps to `Rest { Failed, .., Some(NoLoopTarget) }`.

`apply` (T5) gets a new first arm, `(_, Settle::Ok { note }) if Graph::of(..).out asks_you edges non-empty`, which calls `Box::pin(asks_you_stop(ctx, step, phase, note))`. That function:
1. writes `note_step(note)`;
2. writes the stop note (§3.4);
3. calls `park(ctx, step, phase)`;
4. if the node has a back `asks_you` edge and `!loop_budget_left` for all of them, writes the `LoopExhausted` note, with no item move (A9), and returns `Rest { AwaitingApproval, .., Some(loop_failure) }`.

A failed or rejected settle keeps the node's own gate. Stage 1's interlock (`select.rs:210`) and the CLI port check (`engine.rs:6835`) keep reading `gate_effective`: D8 changes only the post-settle stop.

---

## 2. Data flow

### 2.1 A linear lifted run is unchanged (proof)

The graph is a chain `0 → 1 → … → n−1` over `next` edges, plus `changes_requested` edges, which are never forward.

1. No `if` edge and no `asks_you` edge exist, so no `Decide` or `Loop` cursor exists, and no edge is ever `Declined`. Hence no node is `Skipped`, by induction from the root.
2. Let `k` be the first position that is not `Done`. Every position below `k` is `Done` (rows win), so `k`'s only in-edge is `Taken` and `k` is `Reached` (or is the root). Its cursor is `node_state`'s, which is the old per-position rule: the same `latest_at`, `next_attempt` and `group_cursor`.
3. A node `j > k` may be `Reached`, if `j−1` is `Done` by its rows. Its cursor then sorts after `k`'s. `ready()[0]` is therefore `k`'s cursor, which is exactly what the old scan returned.
4. When every node is `Done`, the answer is `Finished`, as before.

**One difference, not reachable from resolve.** The old scan walked `phases` in **slice** order; the new one walks in **position** order. They agree on every snapshot `resolve` writes, because resolve sorts and densifies (`graph.rs:458-461`). The parity oracle runs on sorted snapshots only. `review_loop.rs`'s unsorted case never calls `cursor`.

The remaining linear call sites behave as before:
- `required_inputs` on the chain equals "a later position" (D9).
- `reads_verdict` on a lifted `review` holds exactly when `loop_target` was `Some`. That is true except for a review placed first or with a gap, which Q-D settles.
- `edge_loop` on the lift: target = the lift's target (V4, V8), budget = `loop_limit` = the target's `retry_limit`, region = `target..=review` in ascending order. That is today's `retire(from..=to)` over the same rows.

### 2.2 An `if` decision

1. Source `S` settles `Ok`. `apply` → `Landing::Advance` → `reconcile_done_step` → `None`.
2. Next pass: `ready()` yields `Decide { step: winner }` first (A6). The `run_to_rest` arm calls `Box::pin(self.decide(&row, &snapshot, winner))`, which does the following:
   1. Parse each of `S`'s `if` predicates. A parse error is `EngineError::Snapshot`. It cannot happen, because W4 runs at save and at resolve.
   2. If any predicate `needs_paths`, read `step_trees(winner)` and `step_commits(winner)`, then `isolator.changed_paths`. Repo ids map to names over `run.repo_scope` and `repos(project)`, as in `engine.rs:5937-5946`. `primary` is the `is_primary` repo's name when it is in scope.
   3. On an `Err(e)`, call `park_on_resume(row, item, format!("branch undecided: `{node}`: changed paths unavailable: {e}"))`. That writes the note and moves the run `running → awaiting_approval`. The item is not written. The function returns `Some(rest)`.
   4. Otherwise build `BranchFacts { verify: winner.verify_outcome, item_prefix, item_tags: item.required_tags, .. }`.
   5. Build `branch = {to: pred.evaluate(&facts)}` and call `record_branch(Lease(owner), winner, &branch)`. `Ok(true)` and `Ok(false)` (a replay, or the step moved) both return `None`.
3. Next pass: the `if` edges resolve from the row. `Taken` targets are `Reached`. Declined targets are `Skipped`, unless they have another taken in-edge, and the skip propagates.
4. **Unblock after a park.** The item is still `open` and `resumable(Decide)` holds, so `unblock_enabled` answers `Resume`. Then `resume` → `walk_resumed_from` → `unpark` → frontier reconcile (`S` again, idempotent) → `run_to_rest` → `Decide` retried.
5. **A restart** reads the stored `branch`, so `changed_paths` is not called again.

### 2.3 A forward `asks_you`: approve or decline

1. `S` settles `Ok`. `apply`'s D8 arm parks, with the stop note, whatever `gate_effective` says (auto mode included).
2. **Approve:** `answer_gate(Approved)` → unpark → reconcile `S` → `run_to_rest`. The edge is `Taken`, so the target is `Reached` and `Create` follows.
3. **Reject:** `answer_guarded` maps it to `(Skipped, Some(note), Done)` (A3's precedence). The tail branches on the **mapped outcome** (today it branches on `answer`) and does reconcile, so the work lands. Then `run_to_rest`: the edge is `Declined`, so the target is `Skipped`, which propagates.
4. **Accept artifact (A2):** `accept_artifact` calls `answer_guarded(.., GateAnswer::Skipped)`. The rest is the same as reject.
5. **Retry:** as today.

### 2.4 A back `asks_you` loop, including the crash replay

1. `S` parks as in §2.3. If no back `asks_you` edge has budget, the stop is the escalation (§1.5 step 4) and approve then simply walks forward.
2. **Approve with budget:** `answer_gate(Approved)` → unpark → reconcile `S` (its work lands, as for any approve) → `run_to_rest`. `ready()` yields `Loop { S, to: T }` first (A6). The `run_to_rest` arm calls `Box::pin(self.loop_back(..))`, which looks up the edge and calls `edge_loop`: budget only (A5), then `retire` with `T` first and `S` last. `Resumed` lets the next pass create `T` at `attempt + 1`.
3. **Crash after `answer_gate` and before the unpark.** The run is `awaiting_approval` and the cursor is `Loop`, which is resumable. Unblock answers `Resume`, and `walk_resumed_from` unparks. The frontier is `S`, with no live rows after it, so the reconcile it owed now happens, then `Loop`.
4. **Crash after the unpark.** The sweep's `recover_run` reconciles the frontier `S` → `Next::Walk` → `Loop`.
5. **Crash part-way through `retire`.** `S` is still `done` + `approved`, so `Loop` sorts first and the retirement replays idempotently (`retire_slot` leaves retired rows alone).

### 2.5 A `fails` loop

1. `S` is gated `never`, has a `fails` edge to `T`, and settles `Failed`. `apply` writes `note_step`, then `S` `running → failed`, then `Box::pin(edge_loop)`:
   - no progress on `T` → escalate;
   - budget spent → escalate (`LoopExhausted { kind: Fails }`, run `awaiting_approval`, item `blocked`);
   - otherwise retire the region `T..S` and return `Landing::Retired`.
2. **Crash between `failed` and the loop.** `run_to_rest` or `recover_run` sees `Rest { Failed }`, calls `settle_failed`, and the new F8 branch runs `Box::pin(self.fails_loop(..))`.
3. **A gated `S`** (`always` or `on_failure`) parks. A human reject → `rejection()` → no `changes_requested` edge → the `fails` edge → `edge_loop`, per A3's order.

### 2.6 `changes_requested` on a diamond

The graph is `A → B`, `A → C`, `B → D`, `C → D` (all `next`), plus `D → B` `changes_requested`.

1. `ready` gives `[A]`, then `[B, C]`, and drives `B` first. Then `[C]`. `D` waits on both (a join), so `D` follows.
2. `D` settles `Rejected` and `edge_loop` targets `B`. The region is `{B, D}`; `C` is not on a `B → D` path and keeps its `done` row ("rows win").
3. Next pass: `B` is `Retired`, so `Create(B, 2)`. `D` waits for `B`. `C` stays done.

---

## 3. Specifics

### 3.1 The parity oracle (D2), in `walk.rs` `#[cfg(test)] mod parity`

`status::cursor_by_position` is today's body, verbatim, behind `#[cfg(test)]`. It is not compiled in a featureless or production build. The oracle asserts both of these for every case:

```rust
walk::ready(&s, &rows)[0] == cursor_by_position(&s, &rows)
status::cursor(&s, &rows) == cursor_by_position(&s, &rows)
```

It also asserts that no `Decide` or `Loop` appears.

**Snapshots.** For each `seed::KINDS` entry (ANA 2 phases, FEAT 4, FIX 3, CLEAN 3, TOOL 3), phases are built from the RUN_1 fixture phase with `name`, `position` and `retry_limit` replaced, in three forms:
- `v: 1` with no edges, so `walk_edges` lifts;
- `v: 2` with `edges = default_snapshot_edges(..)`;
- a gapped `v: 1` with positions multiplied by 10.

**Sweep A1** (`fan_out = 1` everywhere). The per-node alphabet has 17 values, swept as the full cross product (FEAT: 17⁴ = 83 521; total about 98 560 per form):
- ∅;
- one row `(attempt 1, index 0)` at each of the 8 `StepStatus` values;
- `attempt 1 Superseded` plus `attempt 2` at each of the 8.

**Sweep A2.** One node per kind gets `fan_out = 3`. Its alphabet:
- ∅;
- 3 candidates, each in {Pending, Running, Done, Failed, Superseded, Cancelled};
- × selection {none, the first `Done` one `selected = Some(true)`, the rest `Some(false)`};
- × judge row {none, `Running`};
- the short groups {1 or 2 candidates in {Done, Pending}};
- a whole group retired at attempt 1 with attempt 2 short.

The other nodes use {∅, Done, Pending, Failed, Superseded}.

Rows are cloned from a fixture `RunStep` with fresh `StepId`s. The full suite should run in a few seconds in debug; if not, cut the A1 cross product to all pairs per node, not to random samples.

### 3.2 Engine wiring, by site

| Site | Change | Task |
|---|---|---|
| `run_to_rest:3068` | `Decide` and `Loop` arms: T3 → `Err(EngineError::Snapshot { run, reason: "cursor `decide`/`loop` is not walked yet (MOD-138 M2 T5)" })`; T5 → `Box::pin(self.decide(..))` / `Box::pin(self.loop_back(..))` | T3, T5 |
| `resting:6964` | the two arms report the step's position | T3 |
| `phase_spec:6028` | `walk::required_inputs(phase, snapshot)`; engine tests `:8072`, `:8083` change their call site only | T3 |
| `walk_live_step:3887`, `candidate_live:4680` | `reads_verdict: snapshot.reads_verdict(&phase.name)` | T4 |
| `recover_step:2754` | `resettle(.., snapshot.reads_verdict(&phase.name), ..)`; `recover.rs:327` renames the parameter `reads_verdict` | T4 |
| `rejection:1045` | A3's order: `changes_requested` edge → `edge_loop`; else `fails` edge → `edge_loop`; else `finish_run(rejected)`. Map with `gate::landing` semantics plus `cleanup_run` on `NoTarget`. `REVIEW_PHASE` (`:91`) is deleted | T4 |
| `settle_failed:2951` | F8 branch after `rejected` | T4 |
| `resume_window:3233` | D11: `Ok(Resolved(live)) => (v1_topology(&snapshot.phases)?, v1_topology(&live.snapshot.phases)?)`; update the `Resume` docs (`:553-570`) | T4 |
| `answer_guarded:956` | D8 mapping into `(outcome, note, to)`; the tail branches on `outcome == Rejected` | T5 |
| `accept_artifact:1741` | A2 | T5 |

### 3.3 Resolve (D10, T4, `graph.rs`)

- Delete `NotRunnable::NonDefaultEdges` and block (c). `not_yet_runnable(phases)` loses `rows` and `stored`.
- Add `NotRunnable::FanOutEdge` (A7): "a fanned-out node's `asks_you` or `fails` edge runs from MOD-138 M3".
- Add `fn fan_out_edge_refusal(phases: &[SnapshotPhase], edges: &[SnapshotEdge]) -> Result<(), ResolveError>`. It runs after `snapshot_walk_refusal` and before `not_yet_runnable`, over the first `fan_out > 1` node in position order:
  - reads a verdict → `Err(ReviewFanOut { phase, fan_out })`;
  - has an `asks_you` or `fails` out-edge → `Ok(Resolution::Refused { NotYetRunnable { FanOutEdge } })`.
- `non_default_edges_are_refused` becomes `a_graph_with_authored_edges_resolves`: the same body resolves, and `snapshot.edges` carries the `asks_you`.

### 3.4 Note bytes

| Note | Format |
|---|---|
| Stop (D8) | ``"step `{name}` attempt {attempt} waits on you: approve takes {targets}; reject or accept artifact declines {them}, and the step's work lands either way; retry redoes the step"``. `targets` is ``"`deploy`, `plan` (loops back)"``. |
| D6 park | ``"branch undecided: `{node}`: changed paths unavailable: {err}"`` (the plan's text). |
| Escalation | ``"{loop_failure}: phase `{target}`, attempt {attempts}, stop reason `{reason}`"`` (unchanged for `changes_requested`). |

---

## 4. Migrations, store queries, the mirror, `.sqlx`

`crates/htui-store/migrations/0025_run_step_branch.sql` (T1). *Probe* that `jsonb_path_exists` is accepted in a CHECK on PG 16. If it is not, keep the object test only; MemStore's Rust check stays the same either way.

```sql
-- 0025_run_step_branch.sql - MOD-138 milestone 2 (plan D5, Q-A). Forward-only (R-STO-5).
--
-- run_step.branch records, once, the readiness walker's decision on the step's `if` edges:
-- {"<target node key>": true | false} for each `if` edge leaving the step's node. record_branch
-- writes it only while the step is done and the column is NULL, so the first decision wins and
-- every reader, offline ones included, reads the same answer. NULL on every step whose node has
-- no `if` edge and on every row written before MOD-138 M2; nothing is backfilled. No COMMENT, as
-- 0014. The mirror gains the column in cache_migrations/0008_run_step_branch.sql; schema_version
-- becomes 25, so each box rebuilds its mirror once on first start. A headless worker never
-- migrates: migrate from a TUI first.
ALTER TABLE run_step ADD COLUMN branch JSONB
  CONSTRAINT chk_run_step_branch
  CHECK (branch IS NULL
         OR (jsonb_typeof(branch) = 'object'
             AND NOT jsonb_path_exists(branch, 'strict $.* ? (@.type() != "boolean")')));
```

*T1 verify (as landed, `8f9f49fa`):* the path must be `strict`. In lax mode an array member is unwrapped before the filter runs, so `{"a":[true]}` and `{"a":[]}` got through. MemStore's `record_branch` check (T2) must refuse array members too, so that it matches.

`crates/htui-store/cache_migrations/0008_run_step_branch.sql` (T1):

```sql
-- Mirror side of MOD-138 milestone 2 (plan D5): run_step.branch (migrations/0025_run_step_branch.sql).
-- TEXT holding the JSON, as trim_record and usage are mirrored; no CHECK. schema_version moves to
-- 25 through PgStore::schema_version(), which forces a full rebuild, so no backfill is written.
ALTER TABLE run_step ADD COLUMN branch TEXT;
```

**Statements that carry `branch`.**

T1:
- `pg/read.rs` `run_steps` (`:745`): add `branch,` before `updated_at`;
- `pg/read.rs` waiting candidates (`:1406`): the same;
- `pg/write.rs` `create_step … RETURNING` (`:6961`): the same;
- `cache/read.rs` `RUN_STEP_SELECT` (`:308`, read by `:989` and `:1431`) and `run_step_of` (`branch: opt_json_col("run_step.branch", opt_text(row, "branch")?.as_deref())?`);
- `cache/refresh.rs` `RUN_STEP_COLUMNS` (`:1183`), `refresh_run_step`'s `query!` (`:1214`) and the bind `opt_json_text(row.branch.as_ref())?`.

T2: `pg/write.rs` `record_branch` (§1.3).

**`.sqlx`.** T1 replaces 4 entries: the 3 `query_as!` plus the refresh `query!`. T2 adds 1 (`record_branch`). Both run `cargo sqlx prepare` against a migrated scratch database (memory: the compose `htui` database is empty):

```bash
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE prep138m2"
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/prep138m2 sqlx migrate run && \
 DATABASE_URL=postgres://postgres@localhost:5439/prep138m2 cargo sqlx prepare -- --all-targets --all-features)
(cd crates/htui-store && cargo sqlx prepare --check)
```

**Pins that move (T1).**
- 24 → 25: `tests/migrations.rs` `:115` (append `25` to the list) plus prose `:127`; `:3905`, `:4006`, `:4015`, `:4038`, `:4190` plus prose `:3906`, `:4007`, `:4016`, `:4039`, `:4191-4192` ("twenty-five … through MOD-138's 0025_run_step_branch.sql").
- `tests/connect.rs` `:140`, `:156`, `:242` plus prose `:141-142`, `:157-158`, `:243-244`.
- `TABLES` unchanged. Comment-contract test unchanged (no comment).

**Store conformance pins.**

| Pin | After T1 | After T2 |
|---|---|---|
| `htui-core/tests/mem_store.rs:37` | 230 (W4) | 233 |
| `htui-store/tests/pg_conformance.rs:49` `EXPECTED_CASES` (and its doc and message) | 230 | 233 |

---

## 5. Per-task build order and file sets

The order is **T1 → {T2 ∥ T3} → T4 → T5** (V22 is confirmed after A1).

T2 ∩ T3 = ∅:
- T2 is entirely in htui-core store, htui-store (`pg/write.rs`, `worker.rs`, `writer.rs`, `.sqlx`) and htui-agent.
- T3 is entirely in `htui-orch/src/{walk,status,lib,engine}.rs`.

There is no hidden coupling:
- `.sqlx` is T2's alone after T1;
- `Cargo.lock` does not move (`globset` is already a dependency, V12);
- no snapshot or seed is shared;
- T3 never calls `record_branch` (its tests set `RunStep.branch` directly).

Merge in the order T2 then T3. T4 and T5 both touch `engine.rs`, `gate.rs`, `conformance.rs` and `tests/fake_conformance.rs`, so they stay serial.

### T1: core model, predicates, validator, the column (serial, first)

**Commit 1** (no `RunStep` change):
- `model/predicate.rs`, `model/mod.rs`;
- `model/skill_glob.rs` (`compiled` → `pub(crate)`);
- `model/run.rs` (`walk_edges`, `reads_verdict`);
- `store/graph_rules.rs` (`forward_mask`, W4, V21 at `:1458` → `{"verify":"passed"}`);
- `store/traits.rs` (W4 sentence), `store/mod.rs` (re-exports);
- `store/conformance.rs`: V21 at `:25159` → `{"all":[{"verify":"passed"},{"item_kind":"HTUI"}]}`, plus the W4 case;
- pins `mem_store.rs` and `pg_conformance.rs` → 230.

**Commit 2** (the column):
- `RunStep.branch`, and the literal sweep (compiler-driven; the plan's T1 list plus `htui-store/src/cache/read.rs`);
- `0025`, `0008`;
- `pg/read.rs`, `pg/write.rs` (F1's three), `cache/read.rs`, `cache/refresh.rs`, `.sqlx`;
- `tests/{migrations.rs, connect.rs, cache.rs}`.

`graph.rs:5155` (`topology_v2_ignores_edge_order`) never goes through the validator: leave it.

### T2: the verb (after T1; worktree; parallel with T3)

Files:
- `htui-core/src/store/{traits.rs, worker.rs, mem.rs, conformance.rs, mod.rs}`, `htui-core/tests/mem_store.rs`;
- `htui-store/src/{pg/write.rs, worker.rs, writer.rs}`, `htui-store/tests/{pg_conformance.rs, pg_criteria.rs}`, `.sqlx/`;
- `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`.

### T3: the pure walker (after T1; worktree; parallel with T2)

Files:
- `htui-orch/src/walk.rs` (new);
- `status.rs`;
- `lib.rs` (`pub mod walk;`; `required_inputs` re-exported from `walk`);
- `engine.rs`: only `run_to_rest`'s two placeholder arms, `resting`'s arms, `phase_spec:6028`, the `required_inputs` body removed, and the two test call sites at `:8072` and `:8083`.

### T4: loops over edges, the name rule removed, D10, D11 (after T2 and T3; serial)

Files:
- `htui-orch/src/{gate.rs, engine.rs, recover.rs, status.rs, graph.rs, conformance.rs}`;
- `tests/{fake_conformance.rs, review_loop.rs}`.

`fake.rs` is not needed: `Orchestrate::isolator()` and `store()` already reach every seam (`conformance.rs:60-160`). The helpers belong next to `add_asks_you` (`conformance.rs:8628`):
- `add_edge(orch, item, from, to, kind, predicate, loop_limit)`;
- `author_graph(orch, item, nodes, edges)`, over `repoint`/`create_phase` (`:1457`) plus `replace_graph`.

New cases go in a new `graph_walker_case(name, harness)`, chained in `case()` (`:744-760`).

### T5: `if` decisions and `asks_you` (after T4; serial)

Files: `htui-orch/src/{engine.rs, gate.rs, conformance.rs}`, `tests/fake_conformance.rs`.

### Stack (D12)

The new futures are boxed at these points:
- `run_to_rest`'s `decide` and `loop_back`;
- `settle_failed`'s `fails_loop`;
- `apply`'s `edge_loop` (the `fails` arm) and `asks_you_stop`;
- `answer_guarded`'s pre-answer reads.

Every new conformance case is a boxed arm. Gate with `--no-fail-fast` and `grep -c SIGABRT`, which must be 0.

---

## 6. Tests per task and `CASES`

| Task | Unit tests (first) | Conformance cases | Orch `CASES` | Store `CASES` |
|---|---|---|---|---|
| T1 | `predicate.rs`: `parse_accepts_each_closed_form`, `parse_refuses_unknown_keys_and_shapes`, `a_bare_glob_means_the_primary_repo`, `no_changed_paths_makes_both_path_predicates_false`, `a_truncated_list_fails_all_and_keeps_any`, `verify_unavailable_or_none_matches_neither`, `item_kind_is_the_key_prefix_and_item_tag_is_membership`, `combinators_all_any_not`, `needs_paths_finds_a_nested_path_predicate`, `nesting_past_the_cap_is_refused`. `run.rs`: `walk_edges_lifts_a_raw_v1_blob_and_borrows_a_v2`, `a_v2_snapshot_with_no_edges_is_not_lifted`, `reads_verdict_is_having_a_changes_requested_edge`, `a_review_first_in_its_graph_reads_no_verdict`, `run_step_branch_defaults_when_absent`. `graph_rules.rs`: `forward_mask_is_the_validators_classification`, `walk_refuses_an_if_predicate_outside_the_closed_set`, `w4_fires_after_the_loop_rules`. `migrations.rs`: `the_branch_column_admits_an_object_of_booleans_and_null`. `cache.rs`: `the_0008_branch_reaches_the_mirror` (writes the value by raw SQL, as MOD-146 F-5 did) | store: `replace_graph_refuses_an_if_predicate_outside_the_closed_set` | 118 | 230 |
| T2 | `pg_criteria.rs` (optional): `record_branch_takes_the_step_first` | store: `record_branch_first_write_wins` (true, then a second write is `Ok(false)` with the value unchanged; an unknown step is `NotFound`; a non-object is `Constraint`), `record_branch_refuses_a_step_not_done` (each of pending, running, failed, superseded and cancelled gives `Ok(false)` and writes nothing), `record_branch_refuses_a_stale_fence` | 118 | 233 |
| T3 | `walk.rs`: the parity oracle `a_linear_lift_walks_like_the_position_scan`, `a_fanned_out_lift_walks_like_the_position_scan`, `a_gapped_lift_walks_like_the_position_scan`; then `a_join_waits_for_every_forward_in_edge`, `a_join_runs_on_one_taken_edge_once_all_resolve`, `all_declined_in_edges_skip_a_node`, `a_skip_propagates`, `an_if_edge_follows_the_recorded_branch`, `an_undecided_if_source_yields_decide_first`, `a_branch_missing_an_edge_key_reads_as_declined`, `a_forward_asks_you_is_taken_only_when_approved`, `a_back_asks_you_approved_with_budget_yields_loop_first`, `a_spent_back_asks_you_yields_no_loop`, `a_root_is_reached`, `finished_counts_skipped_tails`, `topological_order_not_position_decides_reach`, `rows_win_a_done_node_stays_done`, `region_over_a_diamond_is_every_node_on_a_path`, `region_on_a_lift_is_target_through_source_source_last`, `loop_edge_on_the_lift_targets_the_named_implement` (replaces `gate.rs:1508`'s subject), `required_inputs_follows_forward_reach_on_a_branch`, `required_inputs_on_a_lift_is_the_later_position_rule`, `a_forward_cycle_falls_back_to_the_position_rule`. `status.rs`: `decide_and_loop_are_resumable_parks`. Every existing `status.rs` cursor test passes unedited | none (the full fake suite, unchanged) | 118 | 233 |
| T4 | `gate.rs`: `edge_loop_retires_only_rows_in_the_region`, `edge_loop_to_a_rowless_target_is_no_target` (A4), `an_asks_you_loop_skips_the_no_progress_test` (A5), `loop_failure_is_review_for_changes_requested`; delete `the_loop_target_is_the_named_implement_phase` (A8). `status.rs`: `loop_exhausted_display`. `graph.rs`: `a_graph_with_authored_edges_resolves` (A8, replaces `:5011`), `a_verdict_reading_node_may_not_fan_out`, `an_asks_you_or_fails_node_may_not_fan_out` (A7). `review_loop.rs`: three call sites changed; Q-D's case becomes `a_sparse_snapshot_loops_to_the_phase_before_it_in_order` (give `prd` a done row: `Resumed { 0, 2 }`) plus `a_loop_to_a_target_that_never_ran_is_no_loop_target` (A4) | `a_changes_requested_edge_loops_a_node_not_named_review`, `a_review_with_no_changes_requested_edge_passes_unread`, `a_fails_edge_loops_and_retires_its_region`, `a_human_reject_of_a_fails_node_loops`, `a_fails_loop_cut_short_is_replayed_by_settle_failed`, `a_spent_fails_loop_escalates`, `a_loop_on_a_diamond_retires_only_its_region`, `an_edge_loop_without_progress_escalates`, `a_graph_with_authored_edges_resolves`. D11 is pinned by the two existing A6 cases, expectations unedited; their doc comments are updated | 127 | 233 |
| T5 | `gate.rs`: `an_asks_you_node_parks_an_ok_settle_whatever_its_gate`, `a_spent_back_asks_you_stop_writes_the_escalation_without_blocking` (A9) | `each_if_predicate_decides_from_its_facts`, `a_declined_if_branch_is_skipped_and_the_join_runs`, `an_undecided_branch_parks_on_an_isolator_error_and_unblock_resumes_it`, `a_forward_asks_you_approved_runs_its_target`, `a_forward_asks_you_rejected_skips_its_target_and_lands_the_work`, `auto_mode_never_downgrades_an_asks_you_stop`, `a_back_asks_you_approved_loops`, `a_crash_between_an_asks_you_approve_and_its_loop_is_replayed`, `a_spent_back_asks_you_escalates_and_approve_walks_on`, `a_branch_decision_is_read_from_its_row_after_a_restart` (counts `FakeIsolator::changed_path_requests`), `accepting_an_artifact_on_an_asks_you_node_declines_its_edges` (A2), `a_reject_on_a_verdict_reading_asks_you_node_loops` (A3) | 139 (137 if A2 and A3 are declined) | 233 |

The orch pins are `conformance.rs:8995` and `tests/fake_conformance.rs:18`; extend the history prose in both. Also extend `CASES`' own doc with an M2 paragraph.

---

## Plan amendments needed

- **A1 (F1): T1 takes the column.** `RunStep.branch` cannot compile without `branch` in the three `query_as!(RunStep, …)` statements. T1 therefore lands all of the following:
  - migration `0025` and cache `0008`;
  - those three statements, plus the mirror's read and sync;
  - the `.sqlx` regeneration;
  - the migration pins (9 numbers), the migration test and the mirror test.

  T2 keeps the `record_branch` verb, its 8 implementations, its 3 store cases and 1 `.sqlx` entry. T2 ∩ T3 stays empty.
- **A2 (F2): accept artifact declines.** `accept_artifact` answers `GateAnswer::Skipped` on a node with an outgoing `asks_you` edge, honouring Q-B over D8's "pass through unchanged".
- **A3 (F3): precedence of a human reject.** On a node with more than one kind of edge: `changes_requested` (loop) > `asks_you` (decline, `skipped`, work lands) > `fails` (loop) > end the run.
- **A4 (F4): keep `NoTarget` for a target that never ran.** `no_loop_target` also covers a target with no row, to stop an unbounded loop over a skipped target. Q-D's rewritten case gives `prd` a row so it shows the lift's target, and a second case pins the row-less `NoTarget`.
- **A5 (F5): a back `asks_you` loop is bounded by its budget alone.** There is no no-progress test, because a human approves every turn and `ready()` cannot read commits. Without this, a no-progress escalation leaves a `done` + `approved` source that nothing can retry.
- **A6 (F6): owed writes go first.** `Decide` and `Loop` sort before node cursors in `ready()`. `edge_loop` retires the region with the source last, so a crash part-way through retirement replays instead of looping twice. Linear lifts are unaffected.
- **A7 (F7): refuse fan-out `asks_you` and `fails` in M2.** Resolve refuses a `fan_out > 1` node with an outgoing `asks_you` or `fails` edge (`NotRunnable::FanOutEdge`). Group routing never runs D8 or the `fails` sites.
- **A8 (F9): three tests get rewritten.** The "no expectation edited" exceptions are `review_loop.rs`'s Q-D case, `gate.rs` `the_loop_target_is_the_named_implement_phase` and `graph.rs` `non_default_edges_are_refused`. All three lose their subject. The other three `review_loop.rs` cases change their call site only.
- **A9 (F22): the `asks_you` escalation does not block the item.** The source is still parked at a gate a human answers, and `answer_gate` has no item guard. Blocking would let the run walk under a `blocked` item.

The relevant files are:
- `/home/mluigi/projects/htui/.claude/plans/mod-138-m2-readiness-walker.plan.md` (the spec);
- `/home/mluigi/projects/htui/crates/htui-orch/src/{status.rs,engine.rs,gate.rs,graph.rs,recover.rs,command.rs,lib.rs,conformance.rs}`;
- `/home/mluigi/projects/htui/crates/htui-orch/tests/{review_loop.rs,fake_conformance.rs}`;
- `/home/mluigi/projects/htui/crates/htui-core/src/model/{run.rs,skill_glob.rs}`;
- `/home/mluigi/projects/htui/crates/htui-core/src/store/{graph_rules.rs,traits.rs,worker.rs,mem.rs,conformance.rs}`;
- `/home/mluigi/projects/htui/crates/htui-store/src/{pg/read.rs,pg/write.rs,cache/read.rs,cache/refresh.rs,worker.rs,writer.rs}`;
- `/home/mluigi/projects/htui/crates/htui-store/tests/{migrations.rs,connect.rs,cache.rs,pg_conformance.rs}`;
- `/home/mluigi/projects/htui/crates/htui-worker/src/views.rs`.
