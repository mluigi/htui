# Plan: MOD-138 M2 — The readiness walker

**Source PRD**: `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md`
**Selected Milestone**: 2 — The readiness walker
**Requirements**: `R-ORCH-1`, `R-ORCH-2` (asks_you park), `R-ORCH-3` (per-edge loop target), `R-ORCH-11`.
**Design**: `docs/ANA-33.md` §7.3 (and §7.7 for what M2 does *not* change); M1 record
`docs/decisions/mod/mod-138.md`.
**Complexity**: Large. The walk itself changes, so every engine path is affected. Adds one store verb,
one migration and one cache migration. Linear graphs must walk exactly as before.
**Routing**: PRD path (continuing), ultracode for implement and review (maintainer, 2026-11-05).
**Status**: CONFIRMED 2026-11-05 (maintainer). Blueprint `.claude/plans/mod-138-m2-readiness-walker.blueprint.md`
(findings F1-F23). Blueprint amendments **A1-A9 accepted by the maintainer 2026-11-05**, binding over the text below:
A1 T1 owns the `run_step.branch` column end to end (migrations `0025`/`0008`, the three `query_as!(RunStep)`
statements, mirror read and sync, `.sqlx`, migration pins); T2 keeps only `record_branch`. A2 accept-artifact on an
`asks_you` node answers `Skipped` (declines). A3 a human reject's precedence: `changes_requested` loop > `asks_you`
decline > `fails` loop > end the run. A4 `no_loop_target` also when the loop target has no row. A5 a back `asks_you`
loop is bounded by budget alone (no no-progress test). A6 `Decide`/`Loop` sort before node cursors; `edge_loop`
retires target first, source last. A7 resolve refuses a fan-out node with an `asks_you` or `fails` out-edge
(`NotRunnable::FanOutEdge`). A8 three tests lose their subject and are rewritten (`review_loop.rs`'s Q-D case,
`gate.rs` `the_loop_target_is_the_named_implement_phase`, `graph.rs` `non_default_edges_are_refused`). A9 the spent
`asks_you` escalation does not block the item. Pins after M2: orch `CASES` 139, store 233.

**Landed 2026-11-05** (T1-T5 and review round R1 on `hr/MOD-138`; record in
`docs/decisions/mod/mod-138.md`). Later amendments: **A10** (T5 verify): a reject declines only the
`asks_you` stop after an ok settle (`gate::stopped_to_ask`); a reject of a failed park on an `asks_you` node takes its
own gate's path (D8), pinned by a 13th T5 case, so **orch `CASES` is 140**, store 233. **A11** (review, maintainer
2026-11-05): Q-B's "every `asks_you` edge is taken" cannot hold while a back edge has budget (the loop retires the
source), so resolve refuses a node with a back `asks_you` edge beside another `asks_you` edge
(`NotRunnable::MixedAsksYou`) until M3; the stop note covers only the shapes left (forward targets, or one back edge:
open, spent or never ran). **A12** (T1 verify): `chk_run_step_branch` is a strict, silent jsonpath. The Q-C gap is
wider than written: any graph whose forward order differs from position order (a reordered chain, not only a loop).
Owed to M3/MOD-141 (D9 gap): a join listing a kind whose only producer an `if` declined still requires it.

## Summary

`status::cursor`'s position scan is replaced by a pure `walk::ready(snapshot, steps)` over the
snapshot's nodes and edges:
- **Joins.** A node becomes ready only when every forward in-edge is resolved and at least one was
  taken.
- **Skips.** A node whose in-edges are all not taken is skipped. The skip is derived, never stored,
  and it propagates.
- **`if` links.** Decided once from a closed predicate set when the source finishes, and stored on
  the source's row.
- **`asks_you` links.** Answered through the existing gate.
- **Loops** follow `changes_requested`, `fails` and back `asks_you` edges. Each loop retires the
  region between its two ends, is bounded by the edge's `loop_limit`, and stops early when it makes
  no progress.
- **The review name rule goes**: a node reads a verdict because it has a `changes_requested` edge,
  not because it is called `review`.

M2 still drives **one node at a time**: `cursor()` keeps its signature and becomes "the first of
`ready()` in position order", so the eight callers do not change. On every linear graph the answer
is byte-for-byte the old one, which an oracle test enforces. Resolve stops refusing authored edges.
Divide and palette nodes are still refused (MOD-139, MOD-140).

## Maintainer decisions (2026-11-05, before drafting)

- **Q-A, `if` inputs: decide once, store it.** The walker decides a source's `if` edges once, from
  `Isolator::changed_paths` over the winner's recorded ranges (the same ranges `diff` uses), its
  `verify_outcome` and the item. It writes the answer to a new `run_step.branch` column: migration
  `0025`, mirrored by cache `0008`. Every reader, offline ones included, then reads the same answer.
  Settles the PRD open question "`if` predicate inputs".
- **Q-B, `asks_you` versus the gate: one prompt.** A node with an outgoing `asks_you` edge always
  stops after a successful settle. Auto mode never downgrades that stop. Its answers:
  - **approve**: `approved`, done, and every `asks_you` edge of the node is taken;
  - **reject** or **accept artifact**: `skipped`, done, with the work kept and landed, and the edges
    declined;
  - **retry**: as today.

  No new keys. Settles the PRD open question "gate interplay on `asks_you`".
- **Q-C, recovery frontier: stays in M3** as the PRD has it. M2 keeps `recover::frontier`'s position
  rule. Known M2 gap: any graph whose forward order differs from position order (a reordered chain,
  or a loop on a branching graph; widened at review) can leave a node at a higher position than the
  step that just landed, so crash recovery would re-reconcile the wrong node. Branching
  graphs can only be stored by tests until MOD-141. Recorded under Risks, and owed to M3.
- **Q-D, the name rule goes, both edge cases accepted.**
  - A node named `review` with no `changes_requested` edge (first in its graph) no longer reads its
    verdict, so it no longer fails `no_loop_target`.
  - `tests/review_loop.rs` `a_sparse_snapshot_with_no_predecessor_refuses_by_name` is rewritten: the
    lift loops a gapped review to the phase before it in order (MOD-137's rule). `no_loop_target`
    stays only for an edge naming a key missing from the snapshot.

## Decisions

- **D1 — Every walker read goes through `GraphSnapshot::walk_edges()`** (htui-core). It returns
  `Cow<[SnapshotEdge]>`: the stored edges, or for a v1 blob with no edges the lift
  (`default_snapshot_edges`) computed on the spot. Reason: `tests/review_loop.rs` and others decode
  a v1 snapshot with serde and skip `snapshot_of`'s `lift_v1` (V7), so a walker that read `.edges`
  directly would see no loop edge. `lift_v1` stays.
- **D2 — `walk.rs`, the pure walker** (htui-orch, new module):
  - **Node state** is derived from the latest slot at the node's position. The rules are
    `status.rs`'s, moved, not rewritten:
    - `Done(winner)`: `selected`, or index 0 when `fan_out = 1`;
    - `Live(cursor)`: `Run`, `Rest`, `Fan` or `Select`, from the slot rule;
    - `Retired`: the latest row is superseded or cancelled, so `Create` at `next_attempt`;
    - `Absent`: `Create` at attempt 1.
  - **Forward edges** are `next`, `if`, and every `asks_you` edge that is not a back edge. A back edge
    is one whose target reaches its source over `next`/`if`: the validator's own classification,
    exposed from `graph_rules` as `forward_mask(nodes, edges)` so the two cannot disagree.
  - **Edge resolution:**

    | Source state | `next` | `if` | forward `asks_you` |
    |---|---|---|---|
    | done | taken | the decision on `branch` | taken if `approved`; not taken if `skipped` or anything else |
    | skipped | resolved, not taken | resolved, not taken | resolved, not taken |
    | anything else | unresolved | unresolved | unresolved |

    A done source whose `if` decision is missing (`branch IS NULL`) leaves its `if` edges unresolved
    and yields `Cursor::Decide { step }`.
  - **Reached and skipped nodes.** A node is *reached* when it is a root (no forward in-edges), or
    when all its forward in-edges are resolved and at least one is taken. It is *skipped* when they
    are all resolved and none is taken. States are computed in topological order over the forward
    edges (positions are only a display hint), so a skip propagates.
  - **Rows win.** A node whose latest row is `done` counts as done whatever its in-edges say now. ANA
    §7.3 retires only the region on a loop, so other done nodes keep their rows.
  - **`ready()`** returns, in position order, the cursor of every reached node that is not done or
    skipped. It also returns `Decide`/`Loop` (D7) at the source's position. When every non-palette
    node is done or skipped it returns `[Finished]`.
  - **`status::cursor(snapshot, steps)`** becomes `ready(..)[0]`. The new variants `Decide` and
    `Loop` join `resumable_park`'s walk-on set: a crash cut off writes the engine still owed.
  - **Parity oracle.** The old position scan is kept, test-only, as `cursor_by_position`. A test runs
    old and new side by side over every seeded kind's lifted snapshot and an exhaustive set of row
    states per position (every `StepStatus`, retired slots, fan-out slots and gaps), and they must
    agree.
- **D3 — The closed `if` set** (htui-core `model/predicate.rs`). `IfPredicate` is parsed from the
  opaque `EdgePredicate` value. The value keeps its type, so no column, serde or digest change.
  - `{"all_changed_paths_match": [..]}` and `{"any_changed_path_matches": [..]}` take globs in
    `repo:glob` form (a bare glob means the primary repo), matched with `globset`, which htui-core
    already uses.
  - `{"verify": "passed" | "failed"}`.
  - `{"item_kind": "<prefix>"}`: the item's `key_prefix`.
  - `{"item_tag": "<tag>"}`: membership in `item.required_tags`, the only tag column until MOD-107,
    which may widen it to labels.
  - `{"all": [..]}`, `{"any": [..]}` and `{"not": p}` combine them.

  Evaluation over `BranchFacts { paths, truncated, verify, item_prefix, item_tags }`:
  - **No changed paths:** both path predicates are false. There is no vacuous truth.
  - **Truncated path list:** `all_…` is false, since it cannot be proved; `any_…` is true if a listed
    path matches.
  - **Verify:** `unavailable` or none matches neither value.
- **D4 — Validator rule W4** (the shared `graph_body_walk_refusal`, so `replace_graph` in both stores
  and resolve all get it): an `if` predicate outside the closed set is refused, with one sentence
  naming the edge. Test literals that use `{"verdict": ...}` move to closed-set predicates (V21).
- **D5 — `run_step.branch`.**
  - **Migration `0025_run_step_branch.sql`:** a nullable `branch JSONB` holding `{"<to key>": true |
    false}` for each `if` edge, with a CHECK that it is an object. Cache migration `0008` mirrors the
    column.
  - **`RunStep.branch: Option<Value>`**, `#[serde(default)]`. Adding it touches 47 literal sites (V18).
  - **New verb `record_branch(fence, step, &Value) -> bool`** on `WriteStore` and `WorkerStore`. It
    writes only while the step is `done` and `branch IS NULL`, so the first decision wins and a replay
    is `Ok(false)`. It sits wherever `record_commits` sits (V17), the htui-agent test spies included.
- **D6 — Deciding** (the engine, on `Cursor::Decide`):
  1. Reads the winner's trees and commits and calls `changed_paths`, mapping repo ids to scope names
     as `Engine::changed_paths` does.
  2. Evaluates every `if` edge of the node.
  3. Writes `record_branch` under the walk's lease.

  An isolator error writes no decision. Instead an item note is written ("branch undecided:
  `<node>`: changed paths unavailable: …") and the run moves `running → awaiting_approval`. Unblock
  resumes it (`Decide` is resumable) and tries again. A decision is never guessed.
- **D7 — Loops over edges.** The new `gate::edge_loop(ctx, source, edge)` replaces `review_loop`:
  - **Target:** `edge.to`. A key missing from the snapshot gives `NoTarget` / `no_loop_target`.
  - **Region:** every node on a forward path from target to source, both ends included. Each is
    retired by `retire_slot`, the same status split as today.
  - **Budget:** `may_attempt(target latest attempt + 1, edge.loop_limit.unwrap_or(target
    retry_limit))`. On a lifted v1 graph `loop_limit` is the target's `retry_limit`, so it is today's
    budget.
  - **No progress:** `commits_are_identical` on the target, as today. `reviews_are_identical` on the
    source's `Rejected` rows, which is a no-op for a `fails` or `asks_you` loop.
  - **Escalation:** `ReviewLoopExhausted(n)` for `changes_requested`, with its bytes kept. A new
    `RunFailure::LoopExhausted { kind, from, to, attempts }` for `fails` and `asks_you`.

  Which loop runs:
  - **`changes_requested`:** a node *reads its verdict* exactly when it has an outgoing
    `changes_requested` edge (`snapshot.reads_verdict(node)`, replacing all four `REVIEW_PHASE`
    sites: `walk_live_step`, `candidate_live`, `recover_step`, `rejection`). A request-changes
    verdict, or a human's reject, loops over that edge.
  - **`fails`:** a node with a `fails` edge loops over it on a failure, instead of retrying itself, in
    three places:
    - `gate::apply`'s `never` × failed arm, in place of `retry_or_fail`;
    - a human's reject of a node with no `changes_requested` edge;
    - `settle_failed`'s budget branch (the D131 crash path).

    A node with no `fails` edge keeps `retry_limit`, as today.
  - **Back `asks_you`:** approve takes the loop. Because the source is now `done` + `approved` until
    the loop retires it, `ready()` yields `Cursor::Loop { step, edge }` for a done, approved source
    whose back `asks_you` edge still has budget. The engine runs `edge_loop`, and a crash in between
    replays it. Once the budget is spent, the source still stops at its gate, now as the escalation
    (run `awaiting_approval`, item blocked, `LoopExhausted` note). Approve then counts as declined.
- **D8 — `asks_you` stop (Q-B).** In `gate::apply`, a node with any outgoing `asks_you` edge treats
  an `Ok` settle as `Gate::Always`, whatever its `gate_effective` is, and writes an item note naming
  the edges. On a failed settle its own gate applies. `answer_guarded` maps a human's reject on such
  a node to the store's `answer_gate(Skipped, note)`, which leaves the step done and keeps its
  `gate_note`. Approve and accept artifact pass through unchanged. The reconcile then lands the work
  as for any done step.
- **D9 — `required_inputs(phase, snapshot)` follows the graph.** A kind is optional exactly when a
  node reachable from this one over forward edges (itself excluded) produces it. On a lifted linear
  graph that is "a later position", today's rule. `phase_spec` is the one caller; the `lib.rs`
  re-export keeps its name.
- **D10 — Resolve.**
  - D6(c) is lifted: graphs with authored edges resolve. Divide and palette are still refused.
  - `ReviewFanOut` is widened: a node that reads a verdict may not fan out. The name check stays too,
    so every old refusal is still refused.
- **D11 — v1 resume compares phases only.** With D6(c) lifted, a v1 run's live graph with authored
  edges now resolves. M1's edge-aware compare would then park runs that M1 walked on (V20). A v1 run
  always walks its lift, which is derived from its phases, so `resume_window`'s v1 branch always
  compares the v1 phase-only digest (`v1_topology`). For every graph M1 could resolve this gives the
  same answer: its edges were the default edges of its phases. v2 pinning is unchanged.
- **D12 — No concurrency, no new boxing.** One node at a time (PRD M2). Boxing per-node futures is
  M3's. Any new engine future that sits under `settle_failed` or `walk_resumed_from` is boxed as its
  neighbours are, because the stack is tight (memory: htui-orch stack headroom).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Pure walk, every status matched | `crates/htui-orch/src/status.rs:302-352`, `:408-462` | re-derived from rows each pass (plan D16); every `StepStatus` arm explicit |
| Loop retirement | `crates/htui-orch/src/gate.rs:972-1045` (`retire`, `retire_slot`) | status split: supersede live/done, cancel running/failed |
| Escalation | `gate.rs:1053` (`escalate`) | run CAS first, then item `open → blocked`, then the note |
| One sentence per refusal | `crates/htui-core/src/store/graph_rules.rs:326-377` (`walk_refusal`) | `Option<String>`, first refusal wins, sentence fns in `traits.rs` |
| Fenced step write | `record_commits` (`store/traits.rs:1788`, `pg/write.rs:7591`) | `StepFence::Lease`, `Ok(false)` on a stale write |
| Additive column | `0014_run_step_opening.sql` + cache `0005_run_step_opening.sql` | nullable column, mirrored, `tests/migrations.rs` pins |
| Glob matching | `crates/htui-core/src/model/skill_glob.rs` | `globset`, bytes, no case folding |
| Engine conformance | `crates/htui-orch/src/conformance.rs` `CASES` (118) + `tests/fake_conformance.rs` | named case fns over `CaseHarness`; pin moves |
| Store conformance | `crates/htui-core/src/store/conformance.rs` `CASES` (229) | one case on MemStore and PgStore; pins in `mem_store.rs`, `pg_conformance.rs` |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/predicate.rs` | CREATE | D3 |
| `crates/htui-core/src/model/run.rs` | UPDATE | D1 `walk_edges`, `reads_verdict`; D5 `RunStep.branch` |
| `crates/htui-core/src/model/mod.rs` | UPDATE | export D3 |
| `crates/htui-core/src/store/graph_rules.rs` | UPDATE | D4 W4; `forward_mask` made public (D2) |
| `crates/htui-core/src/store/traits.rs`, `store/worker.rs`, `store/mem.rs` | UPDATE | D4 sentence; D5 `record_branch` |
| `crates/htui-core/src/store/conformance.rs` (+ `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs` pins) | UPDATE | V21 predicate literals; W4 and `record_branch` cases |
| `crates/htui-store/migrations/0025_run_step_branch.sql`, `cache_migrations/0008_run_step_branch.sql` | CREATE | D5 |
| `crates/htui-store/src/{pg/write.rs,pg/read.rs,worker.rs,writer.rs,cache/*}` | UPDATE | D5 column read and write; the verb |
| `crates/htui-store/tests/migrations.rs`, `.sqlx/` | UPDATE | pins (memory: new migration moves count pins); prepare |
| `crates/htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs` | UPDATE | the spies implement `record_branch` |
| The 47 `RunStep {` literal sites (V18) | UPDATE | D5 field (mechanical) |
| `crates/htui-orch/src/walk.rs` | CREATE | D2, D9 |
| `crates/htui-orch/src/status.rs` | UPDATE | `cursor` wrapper, `Decide`/`Loop`, `resumable_park`, `RunFailure::LoopExhausted` |
| `crates/htui-orch/src/gate.rs` | UPDATE | D7, D8, `reads_verdict` in place of `is_review` |
| `crates/htui-orch/src/engine.rs` | UPDATE | D6, D7, D8 wiring; `rejection`, `settle_failed`, `required_inputs` call; D11 |
| `crates/htui-orch/src/recover.rs` | UPDATE | `resettle`'s `is_review` → `reads_verdict` |
| `crates/htui-orch/src/graph.rs` | UPDATE | D10 |
| `crates/htui-orch/src/lib.rs` | UPDATE | `walk` module, exports |
| `crates/htui-orch/src/fake.rs` | UPDATE | helpers: a graph with authored edges, scripted changed paths |
| `crates/htui-orch/src/conformance.rs`, `tests/fake_conformance.rs` | UPDATE | new cases; `CASES` 118 moves |
| `crates/htui-orch/tests/review_loop.rs` | UPDATE | Q-D rewrite of one case; the other two pass unchanged through D1 |

## Tasks

TDD per task: failing tests first, then the change, then `cargo check --workspace --all-targets
--all-features`. Each implementer commits incrementally. The worktree tasks follow the "workflow
worktree implementers" memory.

### T1: Core model, predicates and the validator (serial; first)
- **Action:**
  - D3 `IfPredicate` (parse plus evaluate), with a unit test per variant, the combinators and the
    empty/truncated rules.
  - D4 W4 in `graph_body_walk_refusal` and its sentence; `forward_mask` made public.
  - D1 `walk_edges` and `reads_verdict`.
  - D5's `RunStep.branch` field, and the 47-site literal sweep.
  - V21's predicate literals amended to closed-set ones.
- **Files:** `htui-core/src/model/{predicate.rs,run.rs,mod.rs}`, `htui-core/src/store/{graph_rules.rs,
  traits.rs,conformance.rs}`, and the V18 literal files: `htui-agent/src/record.rs`,
  `htui-core/src/{fixtures.rs,model/waiting.rs,store/mem.rs}`,
  `htui-orch/src/{conformance.rs,engine.rs,fake.rs,gate.rs,recover.rs,select.rs}`,
  `htui-orch/tests/{gix_isolator.rs,review_loop.rs}`, `htui-store/src/cache/read.rs`,
  `htui-worker/src/{runtime.rs,views.rs}`, `htui/src/run_worker.rs`,
  `htui/tests/{chat.rs,runs_pg.rs,worker_crash_pg.rs,worker_pg.rs}`, and `graph.rs`'s `if` test
  literal.
- **Validate:** `cargo test -p htui-core --all-features predicate graph_rules model::run`; workspace check.

### T2: `run_step.branch`, its verb and migrations (after T1; worktree; parallel with T3)
- **Action:** D5's migrations, the column in every PgStore `run_step` read and in the cache mirror's
  read and sync, and `record_branch` at every `record_commits` site.
- **Conformance:**
  - the first write wins;
  - a step that is not `done` refuses;
  - a stale fence refuses;
  - the cache mirror reads the column.
- **Pins:** `CASES` 229; `tests/migrations.rs`. Then `cargo sqlx prepare` against a migrated scratch
  database (memories: sqlx prepare needs a migrated scratch DB; sandbox `psql -h localhost -p 5439`).
- **Files:** `htui-core/src/store/{traits.rs,worker.rs,mem.rs,conformance.rs}`,
  `htui-core/tests/mem_store.rs`, `htui-store/migrations/0025_*.sql`, `htui-store/cache_migrations/0008_*.sql`,
  `htui-store/src/{pg/write.rs,pg/read.rs,worker.rs,writer.rs,cache/read.rs}` and the cache sync
  file, `htui-store/tests/{migrations.rs,pg_conformance.rs,cache.rs}`, `.sqlx/`,
  `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`.
- **Validate:** both stores' conformance with `--test-threads=1`;
  `cargo test -p htui-store --all-features --test migrations`; `cargo sqlx prepare --check`.

### T3: The pure walker (after T1; worktree; parallel with T2)
- **Action:**
  - D2's `walk.rs`: `ready()`, edge resolution, skip propagation, `region(target, source)` and loop
    lookups.
  - D9's `required_inputs` over the snapshot.
  - `status::cursor` becomes the wrapper; `Cursor::{Decide, Loop}`; `resumable_park`.
  - `run_to_rest` gets placeholder arms for the two new cursors, which T4 and T5 fill. They return
    `EngineError::Snapshot` naming the cursor, which no lifted graph can reach.
  - The parity oracle test (D2).
  - Unit tests: joins; all-not-taken skip and its propagation; `if` taken, declined and undecided
    (`Decide`); forward `asks_you` approved and declined; a root; `Finished` with skipped tails;
    topological order differing from position order; `region` over a diamond; `required_inputs` on
    a branch.
- **Files:** `htui-orch/src/{walk.rs,status.rs,lib.rs,engine.rs}`. In `engine.rs`, only `run_to_rest`'s
  match arms and `phase_spec`'s `required_inputs` call.
- **Validate:** `cargo test -p htui-orch --all-features walk:: status::`, then the full fake conformance
  suite: unchanged, 118 cases.

### T4: Loops over edges, and the name rule removed (after T2 and T3; serial)
- **Action:**
  - D7's `edge_loop` with region retirement, per-edge budget and escalation. It covers the
    `changes_requested` and `fails` paths across `gate::apply`, `rejection`, `settle_failed` and
    recovery.
  - `reads_verdict` at the four `REVIEW_PHASE` sites, and `resettle`.
  - `RunFailure::LoopExhausted`.
  - D10 at resolve; D11 in `resume_window`.
  - `review_loop.rs` per Q-D.
- **New conformance cases:**
  - a loop on a non-`review` node with a `changes_requested` edge;
  - a `fails` loop with region retirement;
  - a `fails` loop budget spent → escalation;
  - a loop on a diamond retiring only the region;
  - no progress on an edge loop;
  - a graph with authored edges resolves (D6(c) lifted);
  - a v1 run on a live graph with authored edges walks on (D11, the existing A6 case kept as is).
- **Files:** `htui-orch/src/{gate.rs,engine.rs,recover.rs,status.rs,graph.rs,conformance.rs,fake.rs}`,
  `htui-orch/tests/{fake_conformance.rs,review_loop.rs}`.
- **Validate:** `cargo test -p htui-orch --all-features -- --test-threads=1`.

### T5: `if` decisions and `asks_you` (after T4; serial)
- **Action:** D6's `Decide` arm (facts, `record_branch`, the park on an isolator error), D7's `Loop`
  arm, and D8's forced stop and the answer mapping in `answer_guarded`.
- **New conformance cases:**
  - each `if` predicate decides as specified, using scripted `changed_paths`;
  - a skipped branch and the join after it;
  - an undecided branch parks on an isolator error and resumes;
  - a forward `asks_you` approved runs the target, and rejected skips it with the work landed;
  - auto mode never downgrades the `asks_you` stop;
  - a back `asks_you` approved loops, a crash between approve and retire replays it, and a spent
    budget escalates;
  - a predicate decision is identical after a restart (read from the row, with no second `changed_paths`
    call).
- **Files:** `htui-orch/src/{engine.rs,gate.rs,conformance.rs,fake.rs}`, `htui-orch/tests/fake_conformance.rs`.
- **Validate:** `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo test -p htui-worker --all-features`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                                  # featureless (memory)
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m2-test.log
grep -c SIGABRT /tmp/m2-test.log                                         # must be 0
grep -n 'REVIEW_PHASE' crates/htui-orch/src/{engine,gate,recover,walk,status}.rs  # must be empty (PRD metric)
cargo insta test --workspace --all-features --check
(cd crates/htui-store && cargo sqlx prepare --check)
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The swap changes a linear behaviour the position scan gave for free (lazy re-insertion, slot reading, resumable parks) | High | D2's parity oracle over exhaustive row states; the 118 existing cases pass unedited; `cursor` keeps its signature |
| A walker read misses the v1 lift (raw serde-decoded snapshots) | Medium | D1: every walker read goes through `walk_edges()`; `review_loop.rs` runs unchanged on raw v1 |
| **Q-C gap:** recovery re-reconciles the wrong node after a loop on a branching graph | Low (tests are the only place a branching graph can be stored until MOD-141) | Recorded in the M2 phase note and owed to M3; M3's plan settles `reconciled_at` |
| An `asks_you` reject lands work the human meant to throw away | Medium | Q-B's semantics shown in the stop's item note ("reject declines the link; the work lands; use retry to redo it") |
| The new migration renumbers at merge (MOD-139 also wants a cache migration) | Medium | The usual collect-time renumber; the "new migration moves count pins" memory |
| The stack overflows on new engine futures | Low | D12; gate `--no-fail-fast` plus the SIGABRT grep |
| T2 ∥ T3 merge conflicts | Low | Disjoint file sets (V22); separate worktrees, merged in order T2 → T3 |
| Suite flakiness hides a regression | Medium | `--test-threads=1` (memory); known sandbox Qdrant timeouts are excluded by name |

## Verified claims (plan fact-check, step 3.5; read at `38949409`)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `status::cursor` is a position scan with 8 direct non-test callers | true | `status.rs:316-352`; Gortex callers: `engine.rs` `run_to_rest:3068`, `walk_resumed_from:3346`, `unblock_case:2544`, `resting:6964`, `recover_run:2675`, `hand_back_resume:2353`; `htui-worker/src/views.rs:475`, `:744` |
| V2 | The review rule is the phase name at four engine sites | true | `engine.rs:1054` (`rejection`), `:2754` (`recover_step`), `:3887` (`walk_live_step`), `:4680` (`candidate_live`); `gate.rs:277` reads it; `recover.rs:330` |
| V3 | The loop target and retirement work by position | true | `gate.rs:808-824` (`loop_target`: greatest earlier `implement`, else `position - 1`); `gate.rs:972-983` (`retire` over `from..=to`) |
| V4 | Today's loop budget is the target's `retry_limit`; the lift's `loop_limit` is that same value | true | `gate.rs:792`; `seed.rs:352-357` |
| V5 | `required_inputs` is the later-position rule, with one caller | true | `engine.rs:7504-7515`; `phase_spec:6028` |
| V6 | `recover::frontier` takes the highest-position done winner | true | `recover.rs:355-376` |
| V7 | `lift_v1` runs only in `snapshot_of`; `review_loop.rs` decodes a v1 blob through serde, with no edges | true | `run.rs:621-625`; `command.rs:1034`; `review_loop.rs:32-40`; `fixtures.rs:1459` (`V1`, `edges: Vec::new()`) |
| V8 | The lift and `loop_target` disagree on a gapped snapshot and on a review placed first | true | `seed.rs:337-357` (previous in order; first in order gets no loop) vs `gate.rs:815-823`; `review_loop.rs:269-305` pins `no_loop_target` |
| V9 | `answer_gate`: Approved/Skipped → done, Rejected → failed, Retried → superseded; only Approved needs output | true | `mem.rs:6726-6730`; `command.rs:741-748` |
| V10 | A done step cannot be re-parked (so the stop happens before done, Q-B) | true | `run.rs:218` (`Done → Superseded` only) |
| V11 | `Isolator::changed_paths` exists and the fake scripts it | true | `isolate.rs:206-212`; `fake.rs:708-731`; engine use `engine.rs:6256-6292` |
| V12 | `globset` is already an htui-core dependency | true | `Cargo.toml:95`; `crates/htui-core/Cargo.toml:23`; `model/skill_glob.rs` |
| V13 | Items have no labels; `required_tags` is the only tag column | true | `item.rs:157-197` |
| V14 | `EdgePredicate` is an opaque `Value`; only `if` takes a predicate | true | `kind.rs:352-356`, `:339-343` |
| V15 | The validator's forward/back classification is index-based and reusable | true | `graph_rules.rs:342-377` (`walk_refusal`, `reaches`) |
| V16 | Next free migration numbers are `0025` and cache `0008` | true | `git ls-files`: migrations through `0024_item_status.sql`; cache through `0007_project_type.sql` |
| V17 | `record_commits` sites, which `record_branch` mirrors | true | `traits.rs:1788`; `store/worker.rs:334`, `:879`; `mem.rs:6926`, `:10108`; `pg/write.rs:7591`; `htui-store/src/worker.rs:301`, `:691`; `writer.rs:1228`; `htui-agent/src/conformance.rs:1317`; `htui-agent/tests/recorder.rs:1043` |
| V18 | 47 `RunStep {` literal or pattern sites across 22 files | true | `git grep -c '\bRunStep \{'` (file list under T1) |
| V19 | `CASES` today: orch 118, store 229 (MOD-144 and MOD-158 have merged since M1's 115/218) | true | `conformance.rs:478-728` (118 entries); `store/conformance.rs:80-318` (229) |
| V20 | Lifting D6(c) would flip `a_v1_run_whose_live_graph_gained_a_non_default_edge_walks_on` under M1's edge-aware v1 compare | true | `conformance.rs:8868-8906`; `engine.rs:3222-3243` (refused → phase-only; resolved → v2 digest) → D11 |
| V21 | `if` predicates outside the closed set are used in tests that go through the validator | true | `store/conformance.rs:25159`; `graph_rules.rs:1458`; `graph.rs:5155` area (amend); `run.rs:2116` is a serde/digest test that never goes through the validator (left alone) |
| V22 | Task independence: T2 ∩ T3 = ∅ | true | T2's files are all in htui-core/store, htui-store and htui-agent; T3's are all in `htui-orch/src/{walk,status,lib,engine}.rs`. T1 touches both sets, so it is serial and first. T4 and T5 both touch `engine.rs`, `gate.rs` and `conformance.rs`, so they are serial |
| V23 | `WorkerStore` has three implementations | true | `store/worker.rs:694` (MemStore); `htui-store/src/worker.rs:116` (PgStore), `:506` (Writer) |

## Acceptance

- [ ] Every task done, each with failing-first tests
- [ ] The 118 existing orch cases and every engine test pass with no expectation edited (`review_loop.rs`'s
      one rewrite per Q-D is the only exception)
- [ ] The parity oracle agrees on every lifted linear snapshot
- [ ] Joins, derived skips, each `if` predicate, `asks_you` take and decline (forward and back), loops on
      `changes_requested`/`fails` with region retirement, limits and no progress: each has a passing case
- [ ] There is no `REVIEW_PHASE` in the walk; `required_inputs` follows the graph
- [ ] Validation passes (both stores, `--test-threads=1`, no SIGABRT)
- [ ] Patterns mirrored, not reinvented
