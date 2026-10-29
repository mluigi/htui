# Plan: MOD-138 M1 — Snapshot v2, the shared validator and the v1 lift

**Source PRD**: `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md`
**Selected Milestone**: 1 — Snapshot v2, the validator and the lift
**Requirements**: `R-ORCH-1`, `R-ORCH-3` (data half), `R-ORCH-11`. **Design**: `docs/ANA-33.md` §7.1, §7.2, §7.3
(validation only), §10; MOD-137 D4 (`docs/decisions/mod/mod-137.md`).
**Complexity**: Large (a model change on the run's frozen record, read by every engine path; both stores'
graph writers; a migration; resolve; resume). Every task leaves
`cargo check --workspace --all-targets --all-features` green.
**Status**: CONFIRMED 2026-10-27; blueprint `.claude/plans/mod-138-m1-snapshot-v2.blueprint.md` (`c3daa9c5`).
Blueprint amendments **A1-A4 accepted by the maintainer 2026-10-28** and binding over the text below:
A1 `split` is a derived accessor, not a snapshot field; A2 T1→T2→T3→T4 serial on the primary tree (the lift and
`snapshot_of`'s 1|2 read move to T1), then T5 ∥ T6 in worktrees; A3 `delete_workspace` refuses while a kind or
item holds one of its graphs; A4 a new `WriteStore::usable_graphs(project)` read. Seven (not six) v1-resume tests
move to v1 fixtures; the orch conformance suite is fake-only. **A5** (found by T3's adversarial verifier; accepted
by the maintainer 2026-10-29): palette phases are left out of D7's default-edge rule and of `0023`'s repair, so a
palette node never gains an edge (MOD-137 rule e). **A6** (found by T5's adversarial verifier; maintainer chose the
recommended option 2026-10-29): D5's v1 bullet is amended. When the live resolve of a v1 run is refused as
`NotYetRunnable` or `InvalidGraph`, the run is compared on the v1 phase-only digest (`graph::topology` of the live
phases against the stored phases), as before MOD-138: equal walks on, different parks with the topology note. So a
v1 run started before the upgrade on a graph with a palette phase or authored edges is not stranded, and edges added
under a v1 run no longer park it (v1 runs always walked by position). The edge-aware v2-digest compare stays for every
v1 run whose live graph resolves.

## Summary

New runs freeze a **v2 snapshot**: the phases as nodes (with role, split, palette, plan scope and a frozen
divide block), the graph's **edges** by node key, the **bounds**, the graph's scope and revision, and a
`topology` digest over nodes *and* edges. A v2 run is **pinned**: resume recomputes the digest from the
snapshot's own content and never compares against the live graph. v1 snapshots stay readable forever:
`snapshot_of` lifts them (linear `next` edges plus the default review loop, through `seed::default_edges`'
rule), and a v1 run keeps today's live-topology park. One **pure graph validator** in `htui-core` refuses
bad graphs at save (`replace_graph`) and at resolve. Items and kinds may use **workspace and library graphs**.

The walk itself does **not** change in M1: it is still `status::cursor`'s position scan. So that this is
honest, resolve refuses (with a named error) any graph the position scan cannot honour: a `divide` node
(PRD Q1), a palette node, or an edge set other than `default_edges` of its phases. M2 lifts the edge
refusal when the readiness walker lands; MOD-139 and MOD-140 lift the other two.

## Findings that shaped the plan (read 2026-10-27 at `41f7b234`)

1. **Stored edges can already be stale.** Settings › Kinds adds and edits phases one row at a time through
   `create_phase` and `update_phase` (`crates/htui/src/catalogue.rs:270`, `:278-285`; `PhasePatch` carries
   `name` and `position`, `crates/htui-core/src/model/kind.rs`). Neither writer touches `step_graph_edge`
   (MemStore `State::create_phase` pushes the row and bumps the graph, `crates/htui-core/src/store/mem.rs:3649-3712`).
   So any phase added, moved or renamed in Settings since `0020` left that graph's edges out of step with
   its positions, and **today's position walk hides it**. The moment anything trusts edges, such a graph
   would run in the wrong order. M1 therefore (a) makes the per-row writers keep a default-edged graph
   default-edged (D7) and (b) repairs existing graphs once in migration `0023` (D8). Nothing authors
   non-default edges yet (no editor until MOD-141; `replace_graph`'s only production caller is the override
   clone, which copies edges), so re-deriving default edges loses nothing.
2. **Every reader decodes through one function.** `htui_orch::command::snapshot_of`
   (`crates/htui-orch/src/command.rs:1008-1029`) is the only decoder: the engine (`engine.rs:6912`),
   `htui-worker/src/views.rs` (`:360`, `:449`, `:476`, `:740`) and `htui/src/agent_worker.rs:2680` all call
   it. The stores carry `graph_snapshot` as an untyped `Value`; `overlap::scope_of` reads only its `scope`
   key (`crates/htui-core/src/model/overlap.rs:112`). So the v1 lift has one home.
3. **`resolve` cannot reach the edges today.** `resolve<S: WorkerStore, G: GraphSource>` reads the graph
   through `GraphSource::resolve_graph` (`crates/htui-orch/src/graph.rs:74`), and `WorkerStore` deliberately
   omits `graph_body` (`crates/htui-core/src/store/worker.rs:137-143`). `ResolvedGraph { graph, phases }` and
   `ResolvedPhase { phase, agents, persona }` (`kind.rs:671-690`) carry no edges or parts.
4. **The v1 digest is a measured trap.** `graph::topology` serialises the typed `&[SnapshotPhase]` directly,
   never through `Value`, because `serde_json`'s `preserve_order` differs between workspace and per-crate
   builds (`graph.rs:265-299`). The v2 digest must follow the same rule.
5. **Five resume tests pin the live-topology park** and one pins the "live graph not comparable" note:
   `conformance.rs` `topology_mismatch_parks_on_resume` (`:1757`), `a_topology_mismatch_parks_a_running_run`
   (`:5627`), `a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note` (`:7283`); `engine.rs`
   `a_topology_mismatch_parks_on_resume` (`:12884`), `rebinding_a_phase_persona_parks_a_resumed_run`
   (`:13199`), `resume_walks_a_run_whose_touched_paths_name_an_unknown_repo` (`:12248`). Under v2 pinning
   these would walk on. They move to v1 fixtures (they pin v1 behaviour, which is kept), and each gets a v2
   twin that asserts the opposite.

## Decisions

- **D1 — One type, two versions.** `GraphSnapshot` stays one struct; `GraphSnapshot::V` becomes `2` (the
  version written). `snapshot_of` accepts `v` of 1 or 2 and refuses anything else as today
  (`EngineError::SnapshotVersion`). The JSON keeps the field name **`phases`** for the nodes (ANA-33's
  `nodes` is design notation): every reader keeps working, and a v1 blob decodes as a v2 one with defaults.
  New fields, all `#[serde(default)]`:
  - `GraphSnapshot.edges: Vec<SnapshotEdge>` and `GraphSnapshot.source: Option<SnapshotSource>`
    (`how: pin | kind_default`; `pick` is MOD-126's and is added there);
  - `SnapshotGraph.scope` (project, workspace or library, with the owner id) and `SnapshotGraph.revision`;
  - `SnapshotSettings` gains the bounds `max_parallel_sessions` (default 4), `max_plan_revisions` (3),
    `max_child_runs` (8) and `runtime_changes` (`ask`). Frozen here, read by M3, MOD-140 and MOD-143.
    `bounds.agents` (the planner's allowed agents) is left to MOD-139/MOD-140, additively;
  - `SnapshotPhase` gains `role` (`step | planner | integrate`), `split`, `palette`, `plan_scope` and
    `divide: Option<SnapshotDivide>` (`planner`, `integrate` candidates and personas, `max_parts`,
    `default_parts`). Each is **skipped when at its default** (`role = step`, `split =
    Split::for_fan_out(fan_out)`, `palette = false`, the options `None`), so a linear phase serialises
    **byte-identically** to v1. That is the MOD-26 persona precedent (`run.rs:631-636`), and it keeps the
    v1 digest valid for v1 runs.
  - `subgraph` is **not** added. Nothing stores it yet; MOD-143 adds it, additively.
- **D2 — Edges by node key.** `SnapshotEdge { from, to, kind, predicate, loop_limit }`. `from` and `to` are
  node **keys**, the phase `name` (`UNIQUE (graph_id, name)`; ANA-33 §7.1 "the name is the node key"), not
  phase ids or positions. `predicate` stays MOD-137's opaque `EdgePredicate`; M2 types the closed set.
- **D3 — The v2 digest.** `topology` for v2 is `sha256:` over the typed `{ phases, edges }` (a borrowed
  struct, serialised directly; finding 4). `graph::topology(phases)` stays as the v1 digest. The envelope
  (`v`, `graph`, `source`, `mode`, `settings`, `scope`, `personas`) stays outside both digests, as today.
- **D4 — The v1 lift.** `snapshot_of` lifts a v1 snapshot by filling `edges` with `default_edges`' rule over
  the snapshot's own phases (a key-based twin of `seed::default_edges`, or that function over a
  phase projection; one rule, pinned equal by a test). It keeps `v == 1`, so resume can tell a v1 run from
  a v2 one. Compete fan-out is carried as it is.
- **D5 — Resume splits on `v`.**
  - **v1:** today's live compare, made edge-aware: the live resolve's v2 `topology` is compared with the
    v2 digest of the *lifted* snapshot. An unchanged graph gives equal digests, because the lift and
    `0023`'s repair produce the same default edges. A live graph that M1 refuses as not yet runnable (D6)
    counts as a mismatch and parks with the topology note: its topology did change.
  - **v2:** no live resolve. The digest is recomputed from the snapshot's own `phases` and `edges`. If it
    matches, the run walks on. If not, the run parks with an item note naming both digests
    (`Resume::SnapshotTampered { stored, recomputed, rest }`), never walks, and a `running` run moves to
    `awaiting_approval` as the v1 park does (so no sweep re-adopts it). The new variant is matched at
    `engine.rs:1784` and `htui-worker/src/runtime.rs:2629`.
- **D6 — What M1 refuses at resolve.** A new `ResolveError::NotYetRunnable { node, reason }` for
  (a) a `divide` node, after its expansion is frozen (PRD Q1; lifted by MOD-139), (b) a `palette` node
  (lifted by MOD-140), and (c) a graph whose stored edges are not exactly `default_edges` of its phases,
  compared as (from key, to key, kind, loop limit) sets (lifted by M2). `ResolveError::InvalidGraph` carries
  the shared validator's sentence (D9). Both are pre-run refusals, like the others in `ResolveError`.
- **D7 — Per-row writers keep a default-edged graph default-edged.** In both stores, `create_phase`,
  `update_phase` (any column `default_edges` reads: `position`, `name`, `retry_limit`) and `delete_phase`
  do the following in the same write: **if** the graph's edges equal `default_edges` of its phases
  *before* the write, they are replaced with `default_edges` of the phases *after* it. A graph whose edges
  were authored (not default) keeps them untouched; its new phase is an unlinked root, which the
  validator accepts. Palette phases are left out of the rule (blueprint A5: a palette node has no edges).
  Seeders, which write edges themselves, do not go through this path. The rule lives in
  `graph_rules.rs` beside MOD-137's pure rules, so the two stores share one function.
- **D8 — Migration `0023_default_edges_repair.sql`.** For every graph with no `if`, `asks_you` or `fails`
  edge, delete its `next` and `changes_requested` edges and re-insert `0020`'s backfill: the same SQL rule,
  with `loop_limit` from the target's current `retry_limit`, and (blueprint A5) palette phases left out, as
  D7's rule leaves them out. Forward-only, idempotent. It adds no table or
  column, but `tests/migrations.rs`' count pins move (see the "new migration moves count pins" memory).
  There is no cache migration, because `step_graph_edge` is not mirrored. Renumber at merge if another
  branch takes `0023`.
- **D9 — The shared validator.** `graph_body_walk_refusal(graph, body) -> Option<String>` in
  `crates/htui-core/src/store/graph_rules.rs`. It is pure, first refusal wins, and every refusal is one
  sentence fn, as MOD-137's are. On top of MOD-137's shape and topology rules it checks:
  - the forward edges (`next`, `if` and every `asks_you` that is not a back edge) are acyclic;
  - every cycle is closed by a `changes_requested`, `fails` or `asks_you` edge;
  - a `changes_requested` or `fails` edge is always a back edge, i.e. its target reaches its source over
    forward edges;
  - palette nodes are not counted.

  It is called in both stores' `replace_graph` refusal order straight after `graph_body_topology_refusal`,
  so the shared refusal-order case grows a step, and by `resolve` over the body it reads, as
  `ResolveError::InvalidGraph`. `local` isolation on two nodes that can be live together is M3's rule, not
  this one's.
- **D10 — `resolve` reads edges and parts through `resolve_graph`.** `ResolvedGraph` gains
  `edges: Vec<StepGraphEdge>` and `ResolvedPhase` gains `parts: Vec<PhasePart>`. Both stores fill them from
  their existing reads (`graph_edges`, `phase_parts`), so no new SQL is needed. Like `resolve_graph`'s
  phase and candidate reads today, this is several round trips (`pg/read.rs:1978-1984`). An edge naming a
  phase the read did not return is refused as "the graph moved while it was being resolved", the
  `override_graph` precedent. `GraphSource` keeps its shape; `FakeGraphSource`, `TestSource` and
  `HostGraphs` pass the new fields through.
- **D11 — Divide expansion is frozen, then refused.** A pure `expand_divide` turns a `divide` node into
  `<name>:plan` (role `planner`) and `<name>` (role `integrate`) joined by a synthetic `next` edge. In-edges
  are re-pointed at `:plan` and out-edges leave from `<name>`. `default_parts` comes from `phase_part`.
  Positions stay dense, so later nodes shift by one. It is unit-tested, and `resolve` runs it before D6's
  refusal, so MOD-139 only deletes the refusal.
- **D12 — Workspace and library graphs (MOD-137 D4).** An item's `step_graph_id` and a kind's
  `default_graph_id` may name: a graph of the item's or kind's project; a graph of a workspace that contains
  that project; or a library graph. Overrides stay project-only, and an item may never name another item's
  override. `check_graph`'s context, both stores' kind checks (`graph_not_in_project`,
  `mem.rs:3371`, `pg/write.rs:3551`, `:9755`) and any item-side store check widen to that rule, with a
  renamed sentence. `template_name` and personas keep resolving in the run's project, as `resolve` already
  does (`prompt_template(project.id, …)`). An override of a workspace or library graph is cloned into the
  item's project; the `override_graph` source read handles any scope.
- **D13 — Bounds come from `app_setting` with code defaults.** They are read like `max_fan_out`
  (`app_u32(app, key).unwrap_or(DEFAULT)`, `graph.rs:378-384`). No rows are seeded and there is no
  migration; MOD-123's Settings surface can seed them later.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Additive snapshot field | `crates/htui-core/src/model/run.rs:631-636` (`persona`) | `#[serde(default, skip_serializing_if)]`, digest unchanged when absent, `V` not bumped for additive fields (here `V` is bumped once, deliberately) |
| Digest | `crates/htui-orch/src/graph.rs:265-299` | Typed struct straight to `serde_json::to_string`, never through `Value`; `Store(Constraint)` on a serialiser error |
| Pure shared rules, one sentence fn each | `crates/htui-core/src/store/graph_rules.rs:171-233` | `Option<String>` refusal, documented order, unit tests per rule and one order test |
| Pre-run refusal | `crates/htui-orch/src/graph.rs:154-263` (`ResolveError`) | `thiserror` variant naming the phase, decided before a run row exists |
| Store parity | `crates/htui-core/src/store/conformance.rs` `replace_graph_refusal_order` | one case run on MemStore and PgStore; `CASES` pin moves |
| Resume park | `crates/htui-orch/src/engine.rs:3218-3248` | note first, `running → awaiting_approval` for run then item, `rest` returned |
| Engine conformance | `crates/htui-orch/src/conformance.rs` + `htui-orch/tests/fake_conformance.rs` `CASES` (109) | new cases run on the fake and PgStore orchestrators |
| Migration | `crates/htui-store/migrations/0020_step_graph_v2.sql` backfill | header comment naming plan decisions; forward-only; `tests/migrations.rs` pins |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/run.rs` | UPDATE | D1, D2: v2 fields, `SnapshotEdge`, `SnapshotSource`, `SnapshotDivide`, `NodeRole`, `V = 2`; round-trip and byte-identity tests |
| `crates/htui-core/src/model/kind.rs` | UPDATE | D10: `ResolvedGraph.edges`, `ResolvedPhase.parts` |
| `crates/htui-core/src/store/graph_rules.rs` | UPDATE | D9 validator; D7 `default_edges_follow` rule |
| `crates/htui-core/src/seed.rs` | UPDATE | D4: the key-based twin of `default_edges` (or a projection), pinned equal |
| `crates/htui-core/src/store/mem.rs` | UPDATE | D7, D9, D10, D12: per-row writers, `replace_graph` order, `resolve_graph` fields, kind/item graph rule |
| `crates/htui-store/src/pg/write.rs`, `pg/read.rs` | UPDATE | the same in PgStore; `.sqlx` if any query changes |
| `crates/htui-store/migrations/0023_default_edges_repair.sql` | CREATE | D8 |
| `crates/htui-store/tests/migrations.rs` | UPDATE | D8 pins; a repair test (drifted graph → default edges; authored graph untouched) |
| `crates/htui-core/src/store/conformance.rs` (+ `htui-core/tests/mem_store.rs`, `htui-store` pg conformance pin) | UPDATE | cases for D7, D9, D10, D12; `CASES` 206 moves |
| `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/model/item_spec.rs` | UPDATE | D12 sentence and rule |
| `crates/htui/src/item_writes.rs` and the kind form's graph list | UPDATE | D12: offer workspace and library graphs (lists only; no new UI) |
| `crates/htui-orch/src/graph.rs` (and a new `crates/htui-orch/src/snapshot.rs` if `graph.rs` grows past reason) | UPDATE/CREATE | D3, D6, D9-D11, D13: resolve v2, expansion, digest, refusals |
| `crates/htui-orch/src/command.rs` | UPDATE | D4: `snapshot_of` reads 1|2 and lifts |
| `crates/htui-orch/src/engine.rs` | UPDATE | D5: `resume_window` split; `Resume::SnapshotTampered`; test twins |
| `crates/htui-orch/src/conformance.rs`, `htui-orch/tests/fake_conformance.rs` | UPDATE | finding 5's cases on v1 fixtures; v2 twins; `CASES` 109 moves |
| `crates/htui-orch/src/fake.rs`, `crates/htui-worker/src/graphs.rs` | UPDATE | D10 pass-through |
| `crates/htui-worker/src/runtime.rs` | UPDATE | D5 match arm |
| ~20 test literals of `GraphSnapshot` / `SnapshotPhase` / `SnapshotSettings` across `htui-agent/tests/relay.rs`, `htui-core` (`fixtures.rs`, `conformance.rs`, `mem.rs`, `run.rs`), `htui-mcp/tests/tools_backlog.rs`, `htui-store/tests/{cache,pg_criteria}.rs`, `htui-orch` (`fanout.rs`, `recover.rs`, `overlap.rs`, `gate.rs`, `status.rs`, `engine.rs`, `tests/review_loop.rs`), `htui/src/{queue_overview,run_worker}.rs`, `htui/tests/backlog.rs` | UPDATE | D1's new fields (mechanical) |
| `docs/ANA-2.md` (invariant 2) | UPDATE at close-out | v2 runs are pinned; v1 runs keep the compare (ANA-33 §10 text edit) |

## Tasks

TDD per task: failing tests first, then the change, then `cargo check --workspace --all-targets --all-features`.
Each task commits incrementally (memory: implementers commit incrementally).

### T1: v2 snapshot types and the literal sweep (serial; first)
- **Action**: D1 and D2 in `run.rs`: the new types and fields, `V = 2`, and the skip rules. Tests: a v1
  JSON fixture (`htui-orch/tests/fixtures/*.snapshot.json`) decodes to defaults; a linear phase
  serialises byte-identically to its v1 form; a v2 snapshot with edges, source, bounds and a divide block
  round-trips. Then sweep every struct literal (Files table) with the new fields at their defaults. A
  small test-only constructor is fine where literals repeat.
- **Mirror**: the `persona` field.
- **Files**: `htui-core/src/model/run.rs` plus the literal sites in the Files table.
- **Validate**: `cargo test -p htui-core --all-features model::run`; workspace check.

### T2: the shared validator and the default-edge rule (after T1)
- **Action**: D9's `graph_body_walk_refusal` and D7's pure `default_edges_follow(before_phases,
  before_edges, after_phases)` in `graph_rules.rs`, with a unit test per rule plus order tests. Wire the
  validator into both stores' `replace_graph` refusal order. Extend `replace_graph_refusal_order` and add
  cases (a forward cycle, a forward `changes_requested`, a forward `fails`, an `asks_you` back edge
  accepted). First confirm MOD-137's existing `replace_graph` cases' edge sets pass the new rules
  (claim V9); amend any test data that does not, with a note.
- **Files**: `graph_rules.rs`, `mem.rs` (`replace_graph`), `pg/write.rs` (`replace_graph`),
  `store/conformance.rs`, the `CASES` pins.
- **Validate**: `cargo test -p htui-core --all-features graph_rules`, then the MemStore and PgStore
  conformance suites (`--test-threads=1`).

### T3: per-row writers keep default edges, plus migration `0023` (after T2; store lane)
- **Action**: D7 in both stores' `create_phase`, `update_phase` and `delete_phase`, in the same write
  (PgStore: in the write's transaction, after the revision bump that locks the graph row). D8's
  migration. Cases: adding, moving, renaming (`implement`/`review`) and deleting a phase on a default graph
  leave default edges; the same on an authored graph leaves its edges alone; the `0023` repair turns a
  drifted graph default and leaves an authored one untouched. Re-run `cargo sqlx prepare` against a
  migrated scratch DB (see the sqlx-prepare memory; sandbox `psql -h localhost -p 5439`).
- **Files**: `mem.rs` (the three writers), `pg/write.rs` (the three writers), `migrations/0023_*.sql`,
  `tests/migrations.rs`, `store/conformance.rs`, `.sqlx/`.
- **Validate**: conformance on both stores; `cargo test -p htui-store --all-features --test migrations`;
  `cargo sqlx prepare --check`.

### T4: resolve builds v2 (after T2; orch lane, parallel with T3)
- **Action**: D10's `ResolvedGraph`/`ResolvedPhase` fields, filled in both stores' `resolve_graph` and
  passed through the fakes and `HostGraphs`. Then in `graph::resolve`: nodes with D1's fields; edges by key
  from the read; D11's expansion; D13's bounds; `source` and the graph's scope and revision; D3's digest;
  D9's validator; D6's refusals. Tests in `graph.rs`: each seeded kind resolves to a v2 snapshot whose
  edges equal its default edges; each D6 refusal; `InvalidGraph`; an expansion unit test; a "moved while
  resolving" refusal.
- **Files**: `htui-core/src/model/kind.rs`, `mem.rs` (`resolve_graph` only), `pg/read.rs`
  (`resolve_graph` only), `htui-orch/src/graph.rs` (+ `snapshot.rs`), `htui-orch/src/fake.rs`,
  `htui-worker/src/graphs.rs`, `htui-orch/src/lib.rs`.
- **Intersection with T3**: `mem.rs` and the PgStore files are touched by both, in different functions
  (T3 the three per-row writers; T4 `resolve_graph`). Per the hard rule this strips the parallel marking:
  **T3 and T4 run in separate worktrees** (memory: workflow worktree implementers) and merge in order T3
  → T4. Run them serially if the split is not worth the ~10G target per worktree.
- **Validate**: `cargo test -p htui-orch --all-features graph::`; the fake conformance suite.

### T5: the lift and pinned resume (after T4)
- **Action**: D4 in `command::snapshot_of`; D5 in `resume_window` plus the new `Resume` variant and its
  matches. Move finding 5's six tests onto v1 fixtures, through a test helper that rewrites a run's
  snapshot as v1 (`v: 1`, no edges, the v1 digest). Add the v2 twins: an edited live graph does not park a
  v2 run; a tampered v2 digest parks with the note; a v1 run whose live graph gained a non-default edge
  parks. Pin `htui-orch/tests/fake_conformance.rs` `CASES`.
- **Files**: `htui-orch/src/command.rs`, `htui-orch/src/engine.rs`, `htui-orch/src/conformance.rs`,
  `htui-orch/tests/fake_conformance.rs`, `htui-worker/src/runtime.rs`.
- **Validate**: `cargo test -p htui-orch --all-features`; `cargo test -p htui-worker --all-features`.

### T6: workspace and library graphs for items and kinds (after T3 and T5; serial)
- **Action**: D12 across `item_spec.rs`, `traits.rs` (the sentence), both stores' kind and item checks, the
  form graph lists (`item_writes.rs`, the kind form), and `override_graph` for a non-project source.
  Conformance cases: a kind takes a library graph; an item takes a workspace graph of a workspace holding
  its project; a workspace graph of another workspace is refused; an item's run resolves a library graph
  with the project's templates.
- **Files**: `item_spec.rs`, `traits.rs`, `mem.rs`, `pg/write.rs`, `store/conformance.rs`,
  `htui/src/item_writes.rs`, the kinds form (`htui/src/ui/tabs/settings/kinds.rs`), `htui-orch/src/graph.rs`
  (`override_graph`).
- **Validate**: conformance on both stores; `cargo test -p htui --all-features --features testkit`
  (memory: integration tests need testkit).

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                                  # featureless (memory)
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m1-test.log
grep -c SIGABRT /tmp/m1-test.log                                         # must be 0 (stack memory)
cargo insta test --workspace --all-features --check
(cd crates/htui-store && cargo sqlx prepare --check)
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Making the new phase fields skippable gets a predicate wrong, so v1 digests change and every in-flight v1 run parks on resume | Medium | T1's byte-identity test over every seeded kind's v1 fixture; T5's v1 resume cases on unchanged graphs |
| `0023` and `default_edges` disagree on one graph (gaps, a leading `review`, the `bug` fallback) | Medium | the MOD-137 migration test already pins the SQL rule equal to `default_edges` on every seeded graph; extend it to drifted shapes |
| D7's "default before" check races a concurrent per-row write in PgStore | Low | it runs inside the write's transaction after the revision bump that locks the graph row (MOD-137's ordering) |
| The validator refuses a graph a user can reach today | Low | only `replace_graph` can author edges; T2 checks every existing writer's output (override clone, seeds, demo) |
| T3 ∥ T4 merge conflicts in `mem.rs` and the PgStore files | Medium | worktrees with merges in order, or serial; disjoint functions |
| Stack depth from larger snapshots in the engine | Low | no new futures in M1; gate `--no-fail-fast` + SIGABRT grep |
| The literal sweep touches many crates at once | High (certain) | T1 is serial and first; mechanical; workspace check before commit |

## Verified claims (plan fact-check, step 3.5)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `GraphSnapshot::V = 1` is "the only one it reads"; `snapshot_of` refuses `v != V` | true | `run.rs:551-552`; `command.rs:1008-1029` |
| V2 | Every typed reader decodes through `htui_orch::command::snapshot_of` | true | search `snapshot_of`: `engine.rs:6912`, `htui-worker/src/views.rs:360,449,476,740`, `htui/src/agent_worker.rs:2680`; no `from_value::<GraphSnapshot>` elsewhere |
| V3 | Stores keep `graph_snapshot` untyped; `scope_of` reads only `scope` | true | `cache/read.rs:253`, `pg/write.rs:5835`; `overlap.rs:112` |
| V4 | `WorkerStore` omits `graph_body`; `resolve` is generic over `S: WorkerStore` | true | `store/worker.rs:137-143`; `graph.rs:315` |
| V5 | `ResolvedGraph`/`ResolvedPhase` carry no edges or parts | true | `kind.rs:671-690` |
| V6 | `create_phase` (MemStore) writes no edges; Settings › Kinds calls `create_phase`/`update_phase` | true | `mem.rs:3649-3712`; `catalogue.rs:270`, `:278-285` |
| V7 | `seed::default_edges` is the rule MOD-137's backfill implements, pinned equal by a migration test | true | `seed.rs:265-325` doc; `docs/decisions/mod/mod-137.md` "a migration test pins the two equal" |
| V8 | The v1 digest serialises the typed slice, never `Value` (measured trap) | true | `graph.rs:265-299` |
| V9 | MOD-137's `replace_graph` cases' edge sets pass D9's rules | true | `store/conformance.rs:21943-21945` (`next` ×2 forward, `asks_you` back); `:22351-22380` (the mended body's `build → plan` `asks_you` is a back edge over `plan → build`); `:22502-22505` (`asks_you` forward `plan → review`); `:23204-23254` (`if` + `next` forward); `graph.rs:2784-2785` (`docs → review` `fails` is a back edge over `review → docs`). M2 note: `:23205`'s predicate `{"verdict":"approve","min":2}` is outside ANA-33's closed set and must be amended when M2 types it |
| V10 | Five tests pin `TopologyChanged`, one the "not comparable" note | true | finding 5 (search `Resume::TopologyChanged`) |
| V11 | `Resume` is matched outside the engine only at `htui-worker/src/runtime.rs:2629` | true | search `Resume::(TopologyChanged\|Walked)` |
| V12 | Next migration is `0023`; `step_graph_edge` is not mirrored | true | migrations through `0022_memory.sql`; `HANDOFF.md` coordinates; `cache_migrations` holds no edge table (MOD-137 "not mirrored") |
| V13 | Bounds keys do not exist yet; caps are read by `app_u32` with code defaults | true | no match for `max_parallel_sessions\|max_plan_revisions\|runtime_changes\|max_child_runs`; `graph.rs:378-384` |
| V14 | PgStore `resolve_graph` is several round trips, not one transaction | true | `pg/read.rs:1978-1984` doc |
| V15 | Kinds are held to their own project's graphs in both stores | true | `traits.rs:2556-2561` `graph_not_in_project`; `mem.rs:3371`; `pg/write.rs:3551`, `:9755` |
| V16 | Item forms pick graphs from a `SpecContext.graphs` list that refuses overrides | true | `item_spec.rs:306-331`; `htui/src/item_writes.rs:77-90` |
| V17 | Task independence: T3 ∩ T4 = {`mem.rs`, PgStore files} ≠ ∅ → not parallel on one tree | true (stripped) | the Files lines of T3 and T4 |
| V18 | T1 ∩ every later task ≠ ∅ (the literal sweep touches `conformance.rs`, `mem.rs`, `engine.rs`) → T1 is serial and first | true | the T1 Files line |

## Acceptance

- [ ] All tasks complete, each with failing-first tests
- [ ] Validation passes (both stores, `--test-threads=1`, no SIGABRT)
- [ ] Every seeded kind resolves to a v2 snapshot whose edges are its default edges; linear runs behave as before
- [ ] v1 runs: resume compares and parks exactly as today (finding 5's cases unchanged in substance)
- [ ] v2 runs: an edited live graph never parks; a tampered digest parks with a note
- [ ] One validator refuses the same bad body at `replace_graph` and at resolve
- [ ] Patterns mirrored, not reinvented
