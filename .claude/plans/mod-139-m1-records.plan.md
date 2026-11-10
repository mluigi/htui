# Plan: MOD-139 M1 — Records

**Source PRD**: `.claude/prds/mod-139-divided-fan-out.prd.md`
**Selected Milestone**: M1 — Records
**Complexity**: Medium (wide but mechanical: one migration pair, two model types, two store verbs on
every store, a 31-site literal sweep; no engine behaviour changes)

## Summary

The schema and store half of ANA-33 §7.4 and §7.11: `run_step` gains `role`, `parent_step_id` and
`node_key`, a new `run_plan` table records plan revisions, both are mirrored to the box cache, and
every store reads and writes them. The `stage` role and the reserved positions -3..-1 are enforced by
CHECKs, and a phase whose name ends in `:plan` is refused at save time. Runs behave exactly as before.
The engine does not yet write any role other than `step`; M2 does.

## Decisions

- **D1 — Migration `0028_divided_fanout.sql`** (renumbered at collect if MOD-97's `0028` lands first):
  - `run_step.role TEXT NOT NULL DEFAULT 'step'`, `chk_run_step_role` over
    `('step','planner','part','integrate','subgraph','consult','stage')` (ANA-33 §7.4, verbatim);
    `run_step.parent_step_id UUID NULL REFERENCES run_step(id) ON DELETE CASCADE` with an index;
    `run_step.node_key TEXT NULL`.
  - `chk_run_step_part`: a `part` row has both `parent_step_id` and `node_key`.
  - `chk_run_step_stage`: `(role = 'stage') = (position < 0)`, and a stage row sits at -3..-1 with
    `node_key IN ('pick','prepare','collect')`. That is how the reservation is enforced (probed, C9).
    Negative positions belong to stage rows only; no existing writer produces one (C8).
  - `run_plan (id UUID PK, run_id → run ON DELETE CASCADE, revision INT ≥ 0, planner_step_id →
    run_step ON DELETE CASCADE NOT NULL, scope TEXT CHECK IN PlanScope, status TEXT CHECK IN
    ('proposed','admitted','refused','abandoned'), base_position INT ≥ 0, base_refs JSONB NULL,
    body JSONB NOT NULL, digest TEXT NOT NULL, created_at, updated_at)`, `UNIQUE (run_id, revision)`,
    `CHECK (status <> 'admitted' OR base_refs IS NOT NULL)`, an `updated_at` index, and the
    `set_updated_at()` trigger (0022's shape).
  - **`updated_at` is an addition to ANA-33's column list.** The table is append-only in revisions, but
    a revision's `status` moves (proposed → admitted/refused/abandoned), and the mirror's cursor needs a
    timestamp that moves with it.
  - `COMMENT ON` the new table and every new column, pinned in a `MOD139_COLUMN_COMMENTS` list chained
    into the comments test (the MOD-163 pattern).
- **D2 — Cache `0010_divided_fanout.sql`**: the three `run_step` columns, no CHECKs; `run_plan` with JSON
  as TEXT. A cursor-driven mirrored table on its own `updated_at`, after `run_step` in `run_pass`'s
  foreign-key order with its own explicit match arm. The schema-version bump forces one mirror rebuild
  per box (C10).
- **D3 — Model.**
  - `StepRole` is a `str_enum!` over the seven values, `#[default] Step`. It is not `NodeRole`, which
    is the snapshot's three-value node role and stays as it is.
  - `RunStep` and `NewRunStep` gain `role: StepRole`, `parent_step_id: Option<StepId>` and
    `node_key: Option<String>`. On `RunStep` these carry `#[serde(default)]`, as `branch` does.
  - `PlanStatus` is a `str_enum!`. `RunPlan` is the row; `NewRunPlan` carries `id`, `run_id`,
    `revision`, `planner_step_id`, `scope`, `base_position`, `body` and `digest`.
  - `body` stays a `serde_json::Value` in M1. Its typed shape is MOD-140's validator's, and M2 types it
    for default parts. `run_plan.scope` reuses `PlanScope` (`model/kind.rs:312`).
- **D4 — Verbs.**
  - `run_plans(run) -> Vec<RunPlan>`, ordered by `revision`. It goes on `ReadStore` and `WorkerStore`.
    The cache answers it from the mirror, offline (MOD-142 needs that).
  - `insert_run_plan(fence: StepFence, plan: NewRunPlan) -> RunPlan` goes on `WriteStore` and
    `WorkerStore`. It is fenced like `write_step_document` and always inserts at `proposed`.
    - Refusals, in this order: an unknown run → `NotFound`; a lease mismatch → `Fenced`;
      `planner_step_id` not a step of the run → `Constraint`; a taken `(run_id, revision)` →
      `Constraint`.
    - MOD-140's `plan_submit` and the Q1 test path both write through it.
    - Admission (status move, `base_refs`, part rows) is M2's single-transaction verb, not this one.
  - `create_step` writes the three new columns. MemStore mirrors the three CHECKs as `Constraint`, so
    neither store accepts a row the other refuses.
- **D5 — Reach.** `DeleteReach` gains `run_plans`, counted wherever `run_steps` is counted, on both
  stores. `hierarchy.rs` `labelled()` and `pg_criteria.rs`'s measured cascade (a seeded `run_plan` row,
  so `0 == 0` cannot pass) follow, as MOD-163 D8 did.
- **D6 — `:plan` suffix at save.**
  - The suffix moves out of `htui-orch/src/graph.rs` (private) into `htui-core` as `PLAN_SUFFIX`, and
    `graph.rs` imports it.
  - A new `phase_name_refusal(name) -> Option<String>` returns the reserved-template-name refusal
    first, then `plan_suffix_phase_name(name)`.
  - It replaces the `TemplateRole::of_name` test at its five sites: `graph_body_shape_refusal` rule c,
    and MemStore and PgStore `create_phase` and `update_phase`.
  - Resolve's own collision refusal in `expand_divide` stays, since a v1 graph saved before M1 could
    still hold such a name.
- **D7 — The engine is unchanged.**
  - Every engine `NewRunStep` literal writes `StepRole::Step`, `None`, `None`. Planner and integrate
    rows get their roles in M2, which is also where the engine starts reading rows by role.
  - Ignoring `stage` rows is part of that M2 work, because part rows also sit at positions the frozen
    node list does not name. The PRD's M2 row is amended to say so.
  - The resolve refusal of divide nodes stays.
- **D8 — No demo rows.** The conformance cases build their own runs and plans; `DemoData` and
  `pg/demo.rs` are untouched.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| One run_step column end to end | `f6da9680` (MOD-138 M2 T1, `run_step.branch`) | migration + cache migration + `RunStep` field + the three `query_as!(RunStep)` selects + `RUN_STEP_SELECT`/`run_step_of`/`refresh_run_step` + `.sqlx` + migration pins |
| New table + verbs + reach | MOD-163 M1 (`f098f069`, `22b14503`, `8a66a1db`; plan `mod-163-m1-entry-store.plan.md`) | `0022_memory.sql` shape (CHECK lists, cascade FKs, trigger, comments); stubs-then-logic; forwarders in `writer.rs`, `backend.rs`, UsageSpy, SpyStore |
| Enums | `htui-core/src/model/run.rs:950` `NodeRole`; `kind.rs:310` `PlanScope` | `str_enum!` with doc'd variants; CHECK-list pin test |
| Fenced write | `WriteStore::record_branch` / `write_step_document`; `StepFence` `traits.rs:3706` | `Fenced` on lease mismatch, nothing written |
| Mirrored cursor table | `cache/refresh.rs:254-305` list + explicit arms; `refresh_run_step` :1215 | own `updated_at` cursor, `upsert_sql`, no `_` arm |
| Save-time name refusal | `graph_rules.rs:125-131` rule c; `mem.rs:3900`, `:4065`; `pg/write.rs:4015`, `:4139`; `traits.rs:2734` | one refusal-text helper in `traits.rs`, Mem and Pg give the same sentence |
| Conformance | `store/conformance.rs` `CASES`, `run_case`; counts `mem_store.rs`, `pg_conformance.rs` | generic `async fn case<S: WriteStore>(store)`, run on Mem and Pg |
| Tests | `tests/migrations.rs` (CHECK probes, comments, pins), `tests/cache.rs` `the_0008_branch_reaches_the_mirror` | one test per CHECK and per mirrored column |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-store/migrations/0028_divided_fanout.sql` | CREATE | T1 |
| `crates/htui-store/cache_migrations/0010_divided_fanout.sql` | CREATE | T1 |
| `crates/htui-core/src/model/run.rs` (`StepRole`, `PlanStatus`, `RunPlan`, `NewRunPlan`, fields, `PLAN_SUFFIX`) | UPDATE | T1 |
| `crates/htui-core/src/model/mod.rs` (re-exports, enum pin tests) | UPDATE | T1 |
| `crates/htui-core/src/store/traits.rs` (verbs, `DeleteReach.run_plans`, refusal helpers) | UPDATE | T1 |
| `crates/htui-core/src/store/worker.rs` (`WorkerStore` verbs, MemStore forwarders) | UPDATE | T1 |
| `crates/htui-core/src/store/graph_rules.rs` (rule c → `phase_name_refusal`) | UPDATE | T1 |
| `crates/htui-core/src/store/mem.rs` (`create_step` copies fields; `create_phase`/`update_phase` refusal; verb stubs) | UPDATE | T1, then T2 |
| `crates/htui-store/src/pg/read.rs`, `pg/write.rs` (`create_step` columns, `RunStep` selects, `create_phase`/`update_phase` refusal; verb stubs) | UPDATE | T1, then T3 |
| `crates/htui-store/src/cache/read.rs`, `cache/refresh.rs` (`run_step` columns; `run_plans` stub) | UPDATE | T1, then T3 |
| `crates/htui-store/src/writer.rs`, `backend.rs`, `worker.rs` | UPDATE (forward) | T1 |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`, `crates/htui-worker/src/runtime.rs` (`Failing`) | UPDATE (forward / stub) | T1 |
| `crates/htui-orch/src/graph.rs` (import `PLAN_SUFFIX`) | UPDATE | T1 |
| `crates/htui/src/hierarchy.rs` (`labelled()` gains `run_plans`) | UPDATE | T1 |
| 31 `NewRunStep { … }` and 7 `RunStep { … }` literal sites (C5, C6) | UPDATE (sweep) | T1 |
| `crates/htui-store/.sqlx/*` | REGENERATE | T1, T3 |
| `crates/htui-store/tests/migrations.rs`, `tests/connect.rs` (27 → 28, `TABLES` 51 → 52) | UPDATE | T1 (pins), T3 (CHECK and comment tests) |
| `crates/htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs` | UPDATE | T2 |
| `crates/htui-store/tests/pg_criteria.rs` (cascade `TABLES` 30 → 31, seeded `run_plan`) | UPDATE | T3 |
| `crates/htui-store/tests/cache.rs` (mirror tests) | UPDATE | T3 |
| `.claude/prds/mod-139-divided-fan-out.prd.md` (M2 row amended per D7) | UPDATE | done with this plan |

## Tasks

Every task leaves `cargo check --workspace --all-targets --all-features` green. Order: **T1 → (T2 ∥ T3) →
T4**. T1 is wide because adding a `RunStep` field forces the Pg selects, the migration and `.sqlx` in
the same task (`f6da9680` did the same). T2 and T3 have disjoint file sets (C12).

### Task 1: Schema, types and the compile-green skeleton (serial)
- **Action**:
  - Write both migrations (D1, D2) and the model (D3). Add the `RunStep`/`NewRunStep` fields and sweep
    the literals; the engine writes `Step`/`None`/`None` (D7).
  - Pg `create_step` inserts the columns, and every `query_as!(RunStep)` selects them. The mirror's
    run_step SELECT, `run_step_of` and `refresh_run_step` carry them.
  - Add the trait verbs and forwarders (D4), with stub bodies in Mem, Pg and Cache
    (`StoreError::Backend("MOD-139 M1: not yet")`). Add the `DeleteReach.run_plans` field at zero, and
    `labelled()` (D5).
  - Add `PLAN_SUFFIX` and `phase_name_refusal` at all five sites (D6).
  - Move the migration pins (27 → 28 in `migrations.rs` and `connect.rs`, `TABLES` + `run_plan`).
  - Regenerate `.sqlx` against a migrated scratch database.
- **Tests first**:
  - the `StepRole`/`PlanStatus` CHECK-list pins;
  - `RunStep` decoding with the three fields absent (serde default);
  - `graph_rules` rule c refusing `implement:plan`;
  - Mem and Pg `create_phase`/`update_phase` refusing a `:plan` name with the same sentence (a Mem unit
    test now; the Pg twin rides `htui/tests/kinds.rs` beside `create_phase_with_a_reserved_name_is_refused`);
  - a run_step role round trip through the mirror (`tests/cache.rs`, the `0008` test's twin).
- **Validate**: `cargo check --workspace --all-targets --all-features`; `cargo clippy --workspace -- -D
  warnings`; `cargo test -p htui-core model::`; `cargo test -p htui-core graph_rules`; in
  `crates/htui-store`: `cargo sqlx prepare --check -- --all-targets --all-features` against the scratch
  DB; `cargo test -p htui-store --all-features --test migrations --test connect -- --test-threads=1`.

### Task 2: MemStore and the conformance cases (parallel with T3)
- **Action**: write the conformance cases first, red on MemStore:
  - a part, a planner and a stage row round-trip `role`, `parent_step_id` and `node_key`;
  - a `part` without a parent or a key → `Constraint`; a `stage` at position 0, at -4, or with key
    `nope` → `Constraint`; a `step` at -1 → `Constraint`;
  - `insert_run_plan` writes `proposed` with no `base_refs`, and `run_plans` answers in revision order;
  - a repeated revision → `Constraint`; an unknown run → `NotFound`; a planner step from another run →
    `Constraint`; a stale lease → `Fenced` with nothing written;
  - `run_plans` of an unknown run → empty;
  - project delete reach counts and removes the plans.

  Then the Mem logic, CHECK mirrors, reach and delete. Bump `CASES` counts in `mem_store.rs` and
  `pg_conformance.rs`.
- **Mirror**: `State::create_step`; MOD-163 T2.
- **Validate**: `cargo test -p htui-core --test mem_store`; `cargo test -p htui-core
  conformance::run_case_accepts_every_name_in_cases`.

### Task 3: PgStore, the mirror and the migration tests (parallel with T2)
- **Action**:
  - Write the `pg/read.rs` `run_plans` and the `pg/write.rs` `insert_run_plan`: fence check,
    planner-step-in-run check, insert, and the unique violation mapped to `Constraint`. Add the
    `project_reach` subselect.
  - Mirror: a `RUN_PLAN` cursor table with its own arm, `refresh_run_plan`, and a cache read of
    `run_plans`.
  - Tests:
    - `migrations.rs`: one probe per CHECK, the `MOD139_COLUMN_COMMENTS` pin, and a default `role`
      of `step` on rows that existed before the migration;
    - `pg_criteria.rs`: the cascade case seeds one `run_plan` row and measures it;
    - `tests/cache.rs`: a plan written to Postgres is read back from the mirror after its status
      moves (`updated_at` cursor).
  - Regenerate `.sqlx`.
- **Mirror**: `refresh_run_step_tree` / `refresh_document`; MOD-163 T3.
- **Validate**: `cargo sqlx prepare --check`; `cargo test -p htui-store --all-features --test migrations
  --test cache --test pg_criteria -- --test-threads=1`; `--test pg_conformance` once T2's cases land.

### Task 4: Integration gate (serial, after T2 and T3)
- **Action**: merge the lanes, run the full gate, fix what it finds. No new behaviour.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m1-gate.log
grep -c SIGABRT /tmp/mod139-m1-gate.log   # expect 0
(cd crates/htui-store && DATABASE_URL=<migrated scratch db> cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Sandbox note: the live-Qdrant suites time out here and are owed on the host. Report them, but do not
treat them as a regression of this milestone.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| MOD-97 (another sandbox) also takes `0028`, and its pins collide | High | Renumber at collect as before: whichever merges second moves its file and its 28 → 29 pins |
| T1 is wide (literal sweep, `.sqlx`) and slow to get green | Medium | Mechanical, compiler-led; one implementer, incremental commits |
| T2/T3 discover a signature change late | Medium | T1 fixes every signature; a needed change goes back through the coordinator, never edited in both lanes |
| Mem and Pg CHECK mirrors disagree (stage key list, part rule) | Medium | The conformance cases name each refusal on both stores |
| A mirror table added to the list but not to an arm | Low | Explicit arms, no `_` fallback (`refresh.rs:286-290`) |
| `role` on chat steps (`start_chat_run`) | Low | The column default covers Pg; Mem's literal sets `Step` |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Runs behave exactly as before (no snapshot drift, linear and compete suites green)

## Verified claims

Plan fact-check, 2026-11-10, at `6368b9a0` (sandbox hr/MOD-139; Postgres 16.15 at localhost:5439,
probes in a scratch database, then dropped).

| Claim | Verdict | Evidence |
|---|---|---|
| C1 Resolve refuses divide nodes today and names MOD-139 | TRUE | `htui-orch/src/graph.rs:710-728` `not_yet_runnable`; test `a_divide_node_is_expanded_then_refused` :5146 |
| C2 `PLAN_SUFFIX` is private to `graph.rs`; the expansion refuses a `:plan` collision only at resolve | TRUE | `graph.rs:839`, `:899-908` |
| C3 Save-time name checks sit at five sites, all on `TemplateRole::of_name` | TRUE | `graph_rules.rs:125-131`; `mem.rs:3900`, `:4065`; `pg/write.rs:4015`, `:4139` |
| C4 `PlanScope` exists with `parts`/`successors`/`graph`; there is no `StepRole` | TRUE | `model/kind.rs:310-320`; symbol search for `StepRole` finds none |
| C5 `NewRunStep` has 31 literal sites | TRUE | text search `NewRunStep {` = 34 hits, minus the struct (`run.rs:515`) and two `fn … -> NewRunStep {` signatures (`conformance.rs:5438`, `mem.rs:14138`) |
| C6 `RunStep` has 7 struct-literal sites | TRUE | `fixtures.rs:1807,1867,1904`; `conformance.rs:23833`; `mem.rs:2897,6710`; `recover.rs:936` (`isolate/git.rs:2168` is another struct) |
| C7 Adding a `RunStep` field forces the Pg selects, `.sqlx` and the migration in one task | TRUE | `f6da9680` stat: migration, cache migration, `run.rs`, `pg/read.rs`, `pg/write.rs`, `cache/*`, four `.sqlx` files, pins |
| C8 No code writes a negative `run_step.position` | TRUE | text search `position: -` = 0; judge rows use `fanout_index = -1`, not position (`run.rs:331`) |
| C9 D1's CHECKs accept existing rows and the reserved stage rows, and refuse the bad ones | TRUE (probe) | existing rows → `step`; `(-1,'stage','collect')` accepted; -4, `step` at -1, a part without parent, key `nope` → `chk_run_step_stage` / `chk_run_step_part` |
| C10 A Pg migration bump rebuilds every mirror once | TRUE | MOD-163 M1 C3 (`pg/mod.rs:724-726`, `cache/mod.rs:140-152`), unchanged since |
| C11 Migration pins are 27 and `TABLES`; cascade `TABLES` is 30 | TRUE | `migrations.rs:116`, `:3919`, `:4020`, `:4029`, `:4052`, `:4204`; `connect.rs:140,156,242`; `pg_criteria.rs:3329` `[&str; 30]` |
| C12 T2 and T3 file sets are disjoint | TRUE | T2 {`store/mem.rs`, `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`}; T3 {`pg/read.rs`, `pg/write.rs`, `cache/read.rs`, `cache/refresh.rs`, `.sqlx/*`, `tests/migrations.rs`, `tests/pg_criteria.rs`, `tests/cache.rs`}; intersection empty |
| C13 `DeleteReach` is one struct with per-table fields, and `run_steps` is one of them | TRUE | `traits.rs:3759-3790` |
| C14 `StepFence` is `Lease(owner)` / `Unleased`, and `Fenced` is the refusal | TRUE | `traits.rs:3690-3724` |
| C15 Migrations comment new columns, and tests pin them per item | TRUE | `0020_step_graph_v2.sql:170-203`, `0022_memory.sql:81-96`; `migrations.rs:230`, `:809` (`MOD163_M2_COLUMN_COMMENTS`) |
