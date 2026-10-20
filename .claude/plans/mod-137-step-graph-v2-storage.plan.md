# Plan: MOD-137 — Step graph v2 storage: edges, owner scope, revision, atomic edits

**Source**: HANDOFF `MOD-137` (from ANA-33, `docs/ANA-33.md` §7.1, §10); routed as **plan by maintainer
override** (the router said PRD at exactly the 2-criteria threshold; ANA-33 §7.1 already settles the design).
**Requirements**: `R-ORCH-1`, `R-ORCH-3`, `R-TUI-8` (storage half). **Answers**: ANA-40 P7.
**Complexity**: Large (a migration, a model change on two core rows, five `WriteStore` implementors, seeds,
two catalogue verbs, the re-override). Every task leaves `cargo check --workspace --all-targets --all-features` green.
**Status**: confirmed 2026-10-20 (maintainer CONFIRM); blueprint next.

## Summary

Give step graphs a topology of their own and an owner other than a project, and make an edit to a
graph one atomic, versioned write. The migration adds `step_graph_edge`, `phase_part`, the node
properties a v2 snapshot will freeze (`split`, `palette`, `plan_scope`, the divide planner/integrate
columns), a workspace/library owner scope and a whole-graph `revision`, and backfills today's implied
topology (linear `next` edges plus the review loop's `changes_requested` edge) as data. The stores gain
`replace_graph` (one transaction, CAS on `revision`) and deleters for phases, edges and candidates;
seeds write edges; `Settings > Kinds` gets the verbs; the re-override owed since MOD-4 lands on the new
writer. **Nothing reads edges at run time yet** — the walker, snapshot v2 and the shared graph validator
are MOD-138; today's engine keeps its position walk and name rule, so behaviour is unchanged.

## Decisions

- **D1 — Migration `0020_step_graph_v2.sql`, not `0019`.** `0019_stop_runs.sql` (MOD-125) exists;
  ANA-33 §10 and the HANDOFF line predate it. `schema_version` becomes 20 automatically
  (`pg/mod.rs:724-726`), so each box rebuilds its mirror once. `step_graph` is not mirrored
  (`model/run.rs:511`; no `cache_migrations` table), so **no cache migration**.
- **D2 — The loop bound column is `loop_limit`, not `limit`.** `limit` is a reserved word in Postgres
  (probe: `CREATE TABLE bad(limit int)` → syntax error); quoting it in every `query!` is a standing
  hazard. ANA-33's design name `limit` maps to `loop_limit` in SQL and `loop_limit` in Rust; ANA-33 is
  amended by a one-line note at close-out.
- **D3 — Owner scope.** `step_graph.project_id` becomes nullable; `workspace_id UUID NULL REFERENCES
  workspace(id) ON DELETE CASCADE` is added; `ck_step_graph_owner CHECK (project_id IS NULL OR
  workspace_id IS NULL)`; neither set = the global workflow library. `UNIQUE (project_id, name)` is
  replaced by three partial unique indexes `uq_step_graph_project_name` / `uq_step_graph_workspace_name`
  / `uq_step_graph_library_name` (probe-verified). `ck_step_graph_override_in_project CHECK (NOT
  is_override OR project_id IS NOT NULL)`: overrides stay per-item, so per-project. In Rust,
  `StepGraph.project_id: ProjectId` and `NewStepGraph.project_id` become `scope: GraphScope`
  (`Project(ProjectId) | Workspace(WorkspaceId) | Library`) with a `project_id() -> Option<ProjectId>`
  helper; the ~9 production readers of `.project_id` use the helper. `delete_workspace` cascades its
  graphs (Mem by hand, mirroring `delete_project`'s `gone.graphs`), and `DeleteReach` reports them.
- **D4 — Who may *use* a non-project graph is out of scope.** `check_graph` (`item_spec.rs:321-331`)
  and `create/update_item_kind`'s same-project rule (`write.rs:2745,2831`, `mem.rs:3341`) are
  **unchanged**: an item or kind can still name only its project's graphs. Using a workspace or library
  graph needs `template_name` to resolve in the run's project at snapshot time (ANA-33 §7.1), which is
  MOD-138's snapshot work. MOD-137 stores and edits such graphs; MOD-138 lets runs use them (a HANDOFF
  note is added to MOD-138 at close-out).
- **D5 — `revision` is the whole-graph CAS token.** `step_graph.revision BIGINT NOT NULL DEFAULT 0`.
  **Every** write to a graph's content bumps it by one: `update_step_graph`, `create_phase`,
  `update_phase`, `create_phase_agents`, `set_setting` on the `Phase` rung (`token_budget` is a node
  field), the three deleters, and `replace_graph`. The existing per-row writers keep their `updated_at`
  CAS (no caller changes); only `replace_graph` CASes on `revision`. In Postgres the bump is an explicit
  `UPDATE step_graph SET revision = revision + 1` in the writer's statement/transaction (not a trigger),
  so MemStore and PgStore follow one written rule. It also moves `step_graph.updated_at` (trigger), which
  makes a concurrent per-row graph-name CAS go stale — conservative and accepted.
- **D6 — `step_graph_edge`.**
  `(id UUID PK, graph_id UUID NOT NULL REFERENCES step_graph ON DELETE CASCADE, from_phase_id UUID NOT
  NULL, to_phase_id UUID NOT NULL, kind TEXT NOT NULL, predicate JSONB NULL, loop_limit INTEGER NULL,
  created_at)`:
  - `FOREIGN KEY (graph_id, from_phase_id)` and `(graph_id, to_phase_id) REFERENCES
    step_graph_phase(graph_id, id) ON DELETE CASCADE`, over a new `uq_step_graph_phase_graph_id UNIQUE
    (graph_id, id)`: an edge can never point into another graph, and deleting a phase removes its edges
    (probe-verified);
  - `ck_step_graph_edge_kind` (`next`, `if`, `asks_you`, `changes_requested`, `fails`);
    `ck_step_graph_edge_predicate CHECK ((kind = 'if') = (predicate IS NOT NULL))`;
    `ck_step_graph_edge_loop_limit CHECK (loop_limit IS NULL OR loop_limit >= 0)` and
    `CHECK (kind NOT IN ('changes_requested','fails') OR loop_limit IS NOT NULL)` (`asks_you` may be
    forward, so its limit stays optional here; MOD-138's validator requires it on a back edge);
    `ck_step_graph_edge_not_self CHECK (from_phase_id <> to_phase_id)`;
    `uq_step_graph_edge UNIQUE (from_phase_id, to_phase_id, kind)`.
  - No `updated_at`: edges are replaced, never edited (like `phase_agent`).
  - Rust: `StepGraphEdge { id, graph_id, from, to, kind: EdgeKind, predicate: Option<EdgePredicate>,
    loop_limit: Option<i32> }`. `EdgePredicate` is a transparent newtype over `serde_json::Value` in
    MOD-137; MOD-138 replaces its inside with the closed predicate set (§7.3) without a column change.
  - **Store-level checks only** (both stores, same wording): the CHECKs above, endpoints in the graph,
    and a palette node has no edges. Acyclicity of forward edges, "a back edge is only
    `changes_requested`/`fails`/`asks_you`", and reachability belong to MOD-138's shared pure validator,
    which `replace_graph` will call once it exists (MOD-138 already says "used by save and resolve").
- **D7 — New phase columns** (all with defaults, so existing rows and inserts stay valid):
  - `split TEXT NOT NULL DEFAULT 'none'`, `ck_phase_split CHECK (split IN ('none','compete','divide'))`,
    backfilled `'compete'` where `fan_out > 1`; `ck_phase_split_fan_out CHECK ((split = 'compete') =
    (fan_out > 1))` keeps the two from drifting;
  - `palette BOOLEAN NOT NULL DEFAULT false`;
  - `plan_scope TEXT NULL CHECK (plan_scope IN ('parts','successors','graph'))`;
  - divide columns `planner_persona_id`, `planner_agent_id`, `planner_model`, `integrate_persona_id`,
    `integrate_agent_id`, `integrate_model`, `max_parts INTEGER NULL CHECK (max_parts >= 1)`; persona FKs
    `ON DELETE RESTRICT` like `fk_step_graph_phase_persona`; agent FKs like `judge_agent_id`;
    `ck_phase_divide_columns CHECK (split = 'divide' OR (all seven IS NULL))`.
  - Rust field shape (flat columns vs a `DivideSpec` group) is the blueprint's call; the row maps 1:1.
  - `delete_persona`'s reference check (`mem.rs:4024`, `write.rs:3870`) also counts the two divide
    persona columns and `phase_part.persona_id`.
- **D8 — `phase_part`** (a divided node's optional default parts, ANA-33 §7.4/§7.6 part shape):
  `(phase_id UUID NOT NULL REFERENCES step_graph_phase ON DELETE CASCADE, position INTEGER NOT NULL,
  key TEXT NOT NULL, title TEXT NOT NULL, brief TEXT NOT NULL DEFAULT '', owned_paths TEXT[] NOT NULL
  DEFAULT '{}', acceptance TEXT NULL, agent_id UUID NULL REFERENCES agent(id), model TEXT NULL,
  persona_id UUID NULL REFERENCES persona(id) ON DELETE RESTRICT, PRIMARY KEY (phase_id, position),
  UNIQUE (phase_id, key))`. Written only through `replace_graph` (store check: parts only on a `divide`
  node). No `updated_at`.
- **D9 — Backfill is one rule in two places, pinned equal.** The SQL backfill writes, per graph,
  a `next` edge between each pair of consecutive positions, and for each phase named `review` a
  `changes_requested` edge to the greatest earlier phase named `implement`, else to the immediately
  previous position, with `loop_limit` = the target's `retry_limit` — exactly `gate::loop_target`
  (`gate.rs:807-823`) and `review_loop`'s bound (`may_attempt(attempt+1, impl_phase.retry_limit)`).
  A pure `seed::default_edges(&[StepGraphPhase]) -> Vec<StepGraphEdge>` states the same rule in Rust; the
  seeders (Mem `create_project`, Pg `seed_project`, the demo loader `pg/demo.rs`) call it, and a
  migration test asserts SQL backfill == `default_edges` on every seeded graph, including `bug`
  (`reproduce, fix, review` → `review → fix`, the only previous-phase fallback). MOD-138's v1 lift reuses
  `default_edges`. Edge ids in the backfill are `gen_random_uuid()` (v4; no ordering depends on them).
- **D10 — `replace_graph`.**
  `replace_graph(graph: StepGraphId, expected_revision: i64, body: GraphBody) -> Result<CasOutcome<GraphBody>>`
  where `GraphBody { graph: StepGraph (name, description; scope and is_override are not editable),
  nodes: Vec<NodeBody { phase: StepGraphPhase, agents: Vec<PhaseAgent>, parts: Vec<PhasePart> }>,
  edges: Vec<StepGraphEdge> }`:
  - stale `revision` → `Stale(current body)`, writes nothing;
  - nodes diff **by phase id**: a kept id is updated in place (its `skill_binding` rows survive), a new
    id is inserted, a missing id is deleted (cascading its candidates, parts, edges and phase bindings);
  - candidates, parts and edges are replaced wholesale;
  - `revision + 1`; answers `Applied(new body)`.
  - Postgres: one transaction; the position and name uniques are re-created as
    `uq_step_graph_phase_position` / `uq_step_graph_phase_name DEFERRABLE INITIALLY IMMEDIATE` and the
    transaction runs `SET CONSTRAINTS … DEFERRED`, so swaps and renames commit (probe-verified). No
    statement uses `ON CONFLICT` on these constraints (checked), so deferrability costs nothing.
  - MemStore: `write()` has no rollback (`mem.rs:1355`), so every refusal is decided on the final state
    before the first mutation (the `create_phase_agents` all-or-nothing pattern, `mem.rs:3623-3660`).
  - Refusal order (both stores, one wording, conformance-pinned): unknown graph (`NotFound`), stale
    revision, then reserved names, position/name clashes, persona/agent FKs, edge endpoints, CHECKs,
    palette edges, parts on a non-divide node.
- **D11 — Deleters.** `delete_phase(id)` (cascades candidates, parts, edges, phase bindings; deleting a
  graph's last phase is allowed — `create_step_graph` already makes empty graphs), `delete_edge(id)`, `delete_phase_agents(phase)` (all of a phase's candidates; the
  editor re-adds through `create_phase_agents`). Each bumps `revision`; unknown id → `NotFound`.
  Reads: `graph_edges(graph)` (ordered by from position, to position, kind) and `phase_parts(phase)`
  (by position) on `WriteStore`, beside `phases(graph)`.
- **D12 — Catalogue verbs.** `StoreRequest::ReplaceGraph { scope, graph, expected_revision, body }` and
  `StoreRequest::DeletePhase { scope, phase }` on `catalogue::serve` (stale → `CatalogueStale`, as
  `cas()` does); `REQUEST_NAMES` grows 9 → 11. `GraphEntry` carries `edges` (and the graph's
  `revision` rides on `StepGraph`). Edge and candidate edits go through `ReplaceGraph` only — the
  MOD-141 editor's one write. No TUI key or form in MOD-137 (MOD-141).
- **D13 — Re-override.** A second `override_graph` on an item whose `step_graph_id` already names an
  `is_override` graph re-clones **from the kind's default graph** (`item_kind.default_graph_id`; the
  override itself is not a source) into the existing override graph through one `replace_graph`
  (fresh phase ids, edges remapped), leaving `item.step_graph_id` untouched (ANA-2 `:317-320`). The
  first override also copies **edges** now (remapped to the cloned phase ids). The phase-binding copy
  stays outside the transaction as today (`graph.rs:609-610` already documents the orphan gap); the
  "Re-override is still owed" doc paragraph (`graph.rs:458-461`) is replaced.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Migration | `migrations/0019_stop_runs.sql:1-23`, `0012_persona.sql` | Header `-- 00NN_name.sql - MOD-x (plan Dx). Forward-only (R-STO-5).` + prose (backfill, mirror, schema_version, headless worker); named constraints `ck_`/`uq_`/`fk_`; `COMMENT ON` prefixed `'MOD-137 Dn: …'` |
| Migration tests | `htui-store/tests/migrations.rs` `TABLES` (`:30-84`, count 46 at `:133,:144`), versions `1..19` (`:105-115`), `Pending(19)` (`:1903,:2004`), `MOD125_COLUMN_COMMENTS` (`:600`), `migration_0019_*` staged at v18 (`:1465`) | Same shapes for 0020; `plant_project_kind` (`:1716`) and the trigger test (`:2347`) insert `step_graph` directly |
| MemStore writer | `mem.rs:3475-3731` (`State` fns + async wrappers `:8635-8675`) | Validate everything, then mutate; `CasOutcome::Stale(current)`; `references_no_row(...)` |
| PgStore writer | `pg/write.rs:2966-3248`; tx `create_project` → `seed_project` (`:2419-2452`, `:7966-8065`) | `query!` macros; `pool.begin()` … `tx.commit()`; `map_sqlx`/`map_sqlx_for` (`error.rs:17-52`); `cas_miss` (`:102`) |
| Struct-order SELECTs | `pg/read.rs:2924-2954` (comment `:2933-2936`) | A new `StepGraph`/`StepGraphPhase` field sits in struct order in every SELECT and RETURNING |
| Wrappers | `Writer` (`writer.rs:331`, graph arms `:679-736`), `UsageSpy` (`htui-agent/src/conformance.rs:969-998`), `SpyStore` (`htui-agent/tests/recorder.rs:696-725`) | One delegating arm per new method |
| Conformance | `store/conformance.rs` `CASES` (`:61`), `run_case` (`:248`), `step_graph_and_phase_round_trip` (`:4121`) | `async fn x<S: WriteStore>(store: &S)` with `const CASE`; counts in `htui-core/tests/mem_store.rs:36` and `htui-store/tests/pg_conformance.rs:38` (175 today) |
| Seeds | `seed.rs` `KINDS` (`:54`), `graph_row`/`phase_row`/`kind_row` (`:187-245`) | Pure row builders shared by both seeders |
| Catalogue verb | `store_worker.rs:654-758` enum, `:2164-2172` routing, `catalogue.rs:145` serve, `:318-338` `reread`/`cas`, `REQUEST_NAMES` (`:345`) | Arm + name + route + serve; tests in `htui/tests/kinds.rs` incl. `offline_refuses_every_catalogue_request_by_name` (`:264`) |

## Tasks

Order: **T1 → T2 → T3 → (T4 ∥ T5) → T6.** TDD per repo convention: each task writes its tests first.

### T1: Migration 0020 and its tests
- **Action**: write `0020_step_graph_v2.sql` per D1–D3, D5–D9 (owner scope, partial indexes, override
  CHECK, `revision`, phase columns + CHECKs + `split` backfill, deferrable position/name uniques
  replacing `step_graph_phase_graph_id_position_key` / `_graph_id_name_key`, `uq_step_graph_phase_graph_id`,
  `step_graph_edge`, `phase_part`, edge backfill, COMMENTs). Tests first in `tests/migrations.rs`:
  table list 46 → 48, versions to 20, `Pending(20)`, `MOD137_COLUMN_COMMENTS`, and staged-at-v19
  backfill tests: linear graph → `n-1` `next` edges; `feature`/`refactor`/`tooling` → `review → implement`;
  `bug` → `review → fix`; a graph with no `review` gets no loop edge; `loop_limit` = target `retry_limit`;
  `fan_out > 1` → `split = 'compete'`; scope CHECK and the three partial uniques refuse what they should.
- **Files**: `crates/htui-store/migrations/0020_step_graph_v2.sql` (new), `crates/htui-store/tests/migrations.rs`,
  `crates/htui-store/tests/connect.rs` (`Pending(19)` at `:140,:242`, "nineteen" at `:156-158`).
- **Validate**: apply 0001..0020 with `psql -h localhost -p 5439 -U postgres` to a scratch DB; `\d step_graph_edge`;
  `cargo test -p htui-store --all-features --test migrations --test connect`.

### T2: Model, scope and the compile fan-out (after T1)
- **Action**: tests first (model unit tests: `GraphScope` round-trip, `EdgeKind`/`Split`/`PlanScope`
  `str_enum!` match their CHECK lists like `model/mod.rs:296-321`). Add `GraphScope`, `StepGraph.scope` +
  `revision`, `NewStepGraph.scope`, the D7 phase fields, `StepGraphEdge`, `EdgeKind`, `EdgePredicate`,
  `PhasePart`, `GraphBody`/`NodeBody`. Fix every literal (22 `StepGraphPhase {` sites in 11 files, 5
  `StepGraph {`, 17 `NewStepGraph {`), every `.project_id` reader (D3 helper), the Pg SELECT/RETURNING
  lists in struct order, the existing writers' new columns and D5's revision bump in both stores,
  `delete_persona`'s new references, `delete_workspace`'s cascade. `cargo sqlx prepare` against the
  migrated scratch DB.
- **Files**: `crates/htui-core/src/{model/kind.rs, model/mod.rs, model/item_spec.rs, seed.rs, fixtures.rs,
  store/mem.rs, store/conformance.rs (literals only)}`, `crates/htui-store/src/{pg/read.rs, pg/write.rs,
  pg/demo.rs}`, `crates/htui-store/.sqlx/`, `crates/htui-store/tests/pg_criteria.rs`,
  `crates/htui-orch/src/{conformance.rs, engine.rs, graph.rs (literals only)}`,
  `crates/htui-orch/tests/{fixtures.rs, gix_isolator.rs}`, `crates/htui-worker/tests/auto_queue.rs`,
  `crates/htui/src/{catalogue.rs, item_writes.rs, skills.rs, ui/tabs/backlog/item_form.rs}`,
  `crates/htui/tests/kinds.rs` — the compiler is the final list.
- **Validate**: `cargo check --workspace --all-targets --all-features`; `cargo test -p htui-core --all-features`;
  Pg suite on a scratch DB.

### T3: Store verbs, edges in seeds (after T2)
- **Action**: conformance cases first (`CASES` + `run_case` each; counts 175 → 175+N in both runners):
  `replace_graph_applies_nodes_edges_and_bumps_revision`, `replace_graph_refuses_a_stale_revision_and_writes_nothing`,
  `replace_graph_keeps_bindings_of_kept_phases`, `replace_graph_swaps_positions_and_names`,
  `replace_graph_refuses_an_edge_into_another_graph`, `replace_graph_refuses_edges_on_a_palette_node`,
  `replace_graph_refuses_parts_on_a_non_divide_node`, `replace_graph_refusal_order`,
  `delete_phase_cascades_edges_candidates_parts_and_bindings`, `delete_edge_and_delete_phase_agents_bump_revision`,
  `every_graph_write_bumps_revision`, `graph_scopes_are_unique_per_scope`, `an_override_is_project_scoped`,
  `delete_workspace_takes_its_graphs`, `project_create_seeds_default_edges` (extends
  `project_create_seeds_the_catalogue`). Then `seed::default_edges` (+ unit tests incl. `bug`), the trait
  methods with full docs (D10, D11), MemStore, PgStore, `Writer`, `UsageSpy`, `SpyStore`; seeders and
  the demo loader write edges. `cargo sqlx prepare`.
- **Files**: `crates/htui-core/src/{store/traits.rs, store/mem.rs, store/conformance.rs, seed.rs, fixtures.rs}`,
  `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/{pg/write.rs, pg/read.rs, pg/demo.rs, writer.rs}`,
  `crates/htui-store/.sqlx/`, `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-agent/src/conformance.rs`,
  `crates/htui-agent/tests/recorder.rs`.
- **Validate**: `cargo test -p htui-core --all-features`; `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store
  --all-features --test pg_conformance`; `cargo test -p htui-agent --all-features`.

### T4: Catalogue verbs (after T3; parallel with T5)
- **Action**: tests first in `htui/tests/kinds.rs` (replace round-trip, stale → `CatalogueStale`, delete
  phase re-reads, offline refusal names both new verbs, `GraphEntry.edges` populated); then D12.
- **Files**: `crates/htui/src/{store_worker.rs, catalogue.rs}`, `crates/htui/tests/kinds.rs`.
- **Validate**: `cargo test -p htui --features testkit --test kinds` (testkit or it runs 0 tests).

### T5: Override copies edges; re-override (after T3; parallel with T4)
- **Action**: tests first in htui-orch (first override copies remapped edges; second override re-clones
  from the kind default into the same graph id, bumps revision, keeps `item.step_graph_id`; a re-override
  after the kind default changed picks up the change); then D13.
- **Files**: `crates/htui-orch/src/graph.rs` (incl. its unit tests), `crates/htui-orch/src/conformance.rs`
  (the override cases that live there).
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1` (grep SIGABRT).

### T6: Docs and full gate (after T4, T5)
- **Action**: ANA-33 §7.1/§10 one-line amendments (`0020`, `loop_limit`); ANA-2 `:317-320` re-override
  now built; HANDOFF MOD-138 note (D4: runs may use workspace/library graphs there; `replace_graph`
  calls the shared validator once it exists). Run the full gate.
- **Files**: `docs/ANA-33.md`, `docs/ANA-2.md`, `HANDOFF.md` (close-out per lifecycle).
- **Validate**: the Validation block below; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Independence

Only **T4 ∥ T5** run in parallel. File sets: T4 = `htui/src/store_worker.rs`, `htui/src/catalogue.rs`,
`htui/tests/kinds.rs`; T5 = `htui-orch/src/graph.rs`, `htui-orch/src/conformance.rs`. Intersection: **∅**.
`override_graph` has no production caller in the `htui` crate (only the re-export at
`htui-orch/src/lib.rs:67`), so T5's behaviour change does not reach T4's files. T1→T2→T3 are serial: they
share `mem.rs`, `pg/write.rs`, `conformance.rs` and `.sqlx/`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod137-gate.log; grep -c SIGABRT /tmp/mod137-gate.log
cargo insta test --workspace --all-features --check
cargo test -p htui-store --all-features --test migrations --test connect
HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres cargo test -p htui-store --all-features --test pg_conformance
cargo sqlx prepare --workspace -- --all-targets --all-features   # DATABASE_URL = scratch DB migrated to 0020; then check SQLX_OFFLINE build
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `StepGraph.scope` fan-out wider than counted (tests, TUI) | Medium | T2 is compiler-driven; no catch-all `unwrap` of `project_id()` in production — each site states why a project is implied |
| Pg struct-order SELECTs drift silently | Medium | `pg/read.rs:2933` rule; round-trip conformance reads every new field |
| SQL backfill and `default_edges` diverge | Low | D9's equality test over every seeded graph |
| MemStore partial write on a refused `replace_graph` | Medium | Decide-then-mutate; `replace_graph_refuses_…_and_writes_nothing` cases re-read the whole body |
| Deferred uniques hide a clash until commit (error surfaces at COMMIT, not the statement) | Low | Map the commit-time `23505` through `map_sqlx_for`; a conformance case asserts the same `Constraint` text as MemStore |
| Revision bump moves `updated_at`, staling a concurrent per-row CAS | Low | Accepted (D5); a stale answer is a re-read, never a wrong write |
| Stack depth in orch tests | Low | No new async depth in the walk; gate `--no-fail-fast`, grep SIGABRT |
| Migration rebuilds every box mirror | Certain | Expected per migration; header says so |

## Acceptance

- [ ] T1–T6 complete; every task left `cargo check --workspace --all-targets --all-features` green
- [ ] A migrated database holds the backfilled edges and `split`, equal to `seed::default_edges`
- [ ] `replace_graph` + deleters + reads on all five implementors, conformance-pinned on Mem and Pg
- [ ] New projects and the demo load seed edges
- [ ] `ReplaceGraph` / `DeletePhase` served by `catalogue::serve`
- [ ] Re-override works; the owed note at `graph.rs:458-461` is gone
- [ ] Engine behaviour unchanged (full workspace gate green, no snapshot churn)
- [ ] Validation block passes; validator green

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| ANA-33/HANDOFF: the migration is `0019` | **False → D1** | `crates/htui-store/migrations/0019_stop_runs.sql` exists (MOD-125); next free is `0020` |
| A column can be named `limit` | **False → D2** | Postgres 16.15 probe: `CREATE TABLE bad(limit int)` → `syntax error at or near "limit"` |
| `schema_version` follows the max migration; no cache migration needed | True | `pg/mod.rs:724-726`; no `step_graph` in `cache_migrations/0001..0006`; `model/run.rs:511` |
| `step_graph` today: `project_id NOT NULL`, unnamed `UNIQUE (project_id, name)` | True | `0001_init.sql:214-222`; scratch DB `pg_constraint`: `step_graph_project_id_name_key` |
| Phase uniques are `step_graph_phase_graph_id_position_key` / `_graph_id_name_key`, non-deferrable | True | scratch DB (0001..0019 applied) `pg_constraint`, `condeferrable = f` |
| No code or test pins those constraint names | True | text search `step_graph_(project_id_name_key\|phase_graph_id_(name\|position)_key)` → 0 hits |
| No `ON CONFLICT` targets the graph/phase uniques (deferrable is safe) | True | text search of `ON CONFLICT` in `htui-store/src`: none on `step_graph*` |
| Partial unique indexes give per-scope uniqueness; owner CHECK refuses both set | True | Postgres probe: library duplicate → `uq_l` violation; `(1,1)` → `ck_owner` violation |
| Deferrable uniques let one transaction swap positions and names | True | probe: swap under `SET CONSTRAINTS … DEFERRED` commits |
| Composite FK `(graph_id, phase_id)` refuses a cross-graph edge and cascades on phase delete | True | probe: cross-graph insert → FK violation; `DELETE` phase → 0 edges left |
| `updated_at` triggers cover `step_graph` and `step_graph_phase` only | True | scratch DB `pg_trigger`: `trg_step_graph_updated_at`, `trg_step_graph_phase_updated_at`; `0001:570,574-580` |
| Deleting a phase cascades only to `phase_agent` and `skill_binding`; `run_step` has no phase FK | True | `0001:254-260,436`; scratch DB FK list |
| Today's loop target: greatest earlier `implement`, else previous position; bound = target's `retry_limit` | True | `gate.rs:807-823`, `review_loop` `gate.rs:734`; ANA-33 §7.1 matches |
| Seeded `bug` graph has no `implement` (`reproduce, fix, review`) | True | `seed.rs:54` `KINDS` (survey table, all five kinds) |
| `step_graph` / `phase` writers exist on 5 implementors (trait, Mem, Pg, Writer, UsageSpy, SpyStore) | True | `traits.rs:884-952`, `mem.rs:8635`, `pg/write.rs:1013`, `writer.rs:331`, `htui-agent/src/conformance.rs:751`, `htui-agent/tests/recorder.rs:439` |
| MemStore `write()` has no rollback | True | `mem.rs:1355`; all-or-nothing by pre-check at `mem.rs:3623-3660` |
| `graph.rs:455-461` owes re-override for want of a phase deleter | True | `graph.rs:458-461` verbatim (survey §9) |
| ANA-2: re-override deletes the override's phases and re-clones, `item.step_graph_id` untouched | True | `docs/ANA-2.md:317-320` |
| Literal fan-out: 22 `StepGraphPhase {` sites / 11 files; 5 `StepGraph {`; 17 `NewStepGraph {` | True | `git grep -c` over `*.rs` |
| `override_graph` has no production caller in the `htui` crate | True | `git grep -l override_graph`: htui hits are a re-export (`htui-orch/src/lib.rs:67`) and a test name (`item_writes.rs:506`) |
| T4 ∥ T5 file sets disjoint | True | Independence section; intersection ∅ |
| `connect.rs` pins the migration count 19 | True | `connect.rs:140,156-158,242` |
| `override_graph` is exercised only inside htui-orch (`graph.rs`, `conformance.rs`, `engine.rs`) | True | `git grep -l override_graph -- crates/htui-orch/` |
| Conformance runners pin the case count (175) | True | `htui-core/tests/mem_store.rs:36`, `htui-store/tests/pg_conformance.rs:38` |
| Without `--features testkit`, `htui` integration tests run 0 tests | True | repo memory (htui-integration-tests-need-testkit); T4 validates with `--features testkit` |
