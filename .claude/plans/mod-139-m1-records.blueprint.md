# Blueprint: MOD-139 M1 — Records (T1–T4)

> Produced by `code-architect` 2026-11-10 from the confirmed plan
> `.claude/plans/mod-139-m1-records.plan.md` (D1–D8, Verified claims C1–C15) and the PRD
> `.claude/prds/mod-139-divided-fan-out.prd.md` (M1 row). Design: `docs/ANA-33.md` §7.2, §7.4, §7.11.
> Tree read at `5a7046ac` (branch `hr/MOD-139`; no source change since the plan's `6368b9a0`).
> The plan stays the spec. §0 lists where the tree or a probe disagrees with it. Every row is
> **adopted**: each one is an implementation detail or the only reading that works. None reopens D1–D8.
> The `0028` SQL in §T1.1 was applied to a scratch database (`mod139_bp_probe` on the sandbox's
> `localhost:5439`, PostgreSQL 16, since dropped) on top of `0001..0027`, and the cache `0010` SQL to an
> SQLite file holding a populated `run_step`. Probe outcomes are in §T1.2.

## 0. Plan deltas

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | D3's `RunPlan.id` and `NewRunPlan.id` need a type. The model never holds a bare `Uuid` in a struct field: "a mixed-up id is a compile error". `model/ids.rs` is missing from Files to Change. | `model/ids.rs:1-6` (module doc), `:76-134` (every table has its newtype, `MemoryEntryId` last) | **adopted**: `RunPlanId` (`run_plan.id`) is added to `id_newtype!`, re-exported, and joins `id_newtypes_round_trip` (`model/mod.rs:561`). |
| P-2 | D1's `chk_run_step_stage`, written as the plan words it (`role <> 'stage' OR (position BETWEEN -3 AND -1 AND node_key IN (...))`), **admits a stage row with a NULL `node_key`**. `NULL IN (...)` is NULL, `false OR NULL` is NULL, and a CHECK passes on NULL. C9's probe did not try a NULL key. | Scratch probe: the naive CHECK accepted `(position -1, role 'stage', node_key NULL)`. With `node_key IS NOT NULL AND` spelled out, the same insert fails on `chk_run_step_stage`. | **adopted**: the CHECK in §T1.1 spells out `node_key IS NOT NULL`. `run_step_refusal` (P-7) refuses a key-less stage on both stores, and T3's probe test pins the NULL case. |
| P-3 | The plan's "Pg twin rides `htui/tests/kinds.rs` beside `create_phase_with_a_reserved_name_is_refused`" cannot hold, because `kinds.rs` never reaches Postgres: `demo()` is `Backend::memory(MemStore::demo())`. | `htui/tests/kinds.rs:39-40` | **adopted**: the Postgres twin is a **conformance case**, `phase_name_with_the_plan_suffix_is_refused` (T2, run by `pg_conformance` at T4). It covers `create_phase`, `update_phase` and `replace_graph` on both stores, with the exact sentence. `kinds.rs` keeps a seam test in T1 (`create_phase_with_the_plan_suffix_is_refused`, MemStore through `serve`), as its reserved-name twin does. T2 therefore has **nine** cases. |
| P-4 | Files to Change lists `htui-worker/src/runtime.rs` (`Failing`) as a forwarder. `Failing` implements `WorkerHost` only, whose surface does not grow. No other test store implements a store trait. | `runtime.rs:3976-4016` (`impl WorkerHost for Failing`); `git grep '(ReadStore\|WriteStore\|WorkerStore) for'` = the eleven impls of §T1.6 | **adopted**: `runtime.rs` is untouched. |
| P-5 | Four files the plan's Files to Change misses. (a) `htui-core/src/store/mod.rs` (the refusal helpers reach other crates through its `pub use traits::{…}`; MOD-163 P-5). (b) `htui-store/src/cache/mod.rs`: `MIRRORED_TABLES: [&str; 21]` must list `run_plan`, or `CacheStore::rebuild` leaves its rows behind. Its count is pinned at `tests/cache.rs:2297-2301`. (c) `tests/pg_criteria.rs` must compile in T1: its `DeleteReach` destructure has no `..` (MOD-163 P-6). (d) `htui-store/tests/migrations.rs` also gets a no-server cache-migration test (`cache_0009_applies_over_a_populated_0008_mirror`'s twin). | `store/mod.rs:22-52`; `cache/mod.rs:44-66`, `:176-184`; `pg_criteria.rs:3376-3438`; `migrations.rs:6507` | **adopted**: (a) T1. (b) T3, with `refresh_run_plan`, so the table joins the list when something fills it. (c) T1 edits `TABLES`/destructure/`claimed` for compile (still green: `0 == 0`), and T3 seeds and asserts. (d) T1. All four keep T2 ∩ T3 empty (§5). |
| P-6 | C6's line numbers drifted (+23), and one of its seven sites is not a full literal: `conformance.rs:23832` is `RunStep { branch, ..row.clone() }`. The mirror's `run_step_of` (`cache/read.rs:316`) is a full literal that C6 lists separately. | `git grep 'RunStep {'` at `5a7046ac` | **adopted**: §T1.8 lists the **seven** full literals (six model or test sites plus `run_step_of`) and the 31 `NewRunStep` sites at today's lines. The sweep follows the compiler. |
| P-7 | D4 says "MemStore mirrors the three CHECKs as `Constraint`" but names no sentence. The house rule (`graph_rules.rs:1-3`, `branch_refusal`) is one pure helper both stores call **before any read**, so the two stores refuse with one sentence. Postgres also refuses two inputs MemStore would keep: a NUL in `text` (`22021`) and `\u0000` anywhere in `jsonb` (`22P05`, probed: "unsupported Unicode escape sequence"). | `traits.rs:2828-2831` (`has_nul`), `:3404` (`branch_is_not_an_object_of_booleans`); probe | **adopted**: `run_step_refusal(&NewRunStep)` (part rule, stage rules, `node_key` NUL) and `run_plan_refusal(&NewRunPlan)` (`revision`/`base_position` ≥ 0, NUL in `digest` or `body`), §T1.3. PgStore's `create_step` calls the first **in T1**, since T1 already rewrites that statement. MemStore calls it in T2. Both calls of the second land with the verb bodies (T2, T3). So `insert_run_plan`'s refusal order **starts** with this shape check, before the plan's four (the `record_branch` order). |
| P-8 | `StoreError::Fenced` carries only `{ step }`, but `insert_run_plan` is fenced on a **run**. | `store/error.rs:23-31` | **adopted**: `Fenced { step: plan.planner_step_id }`, "the step the write named". PgStore locks the planner step and then the run (MOD-77's step → run order, `pg/write.rs:244-252`). |
| P-9 | `store/conformance.rs` self-scans: any **backticked** snake_case token with four or more underscores must be a fn in `conformance.rs` or `mem.rs` (`every_cross_referenced_test_name_exists`). | `conformance.rs` (`fn every_cross_referenced_test_name_exists`, MOD-163 P-8) | **adopted** (gotcha): every new helper name has at most three underscores (`run_step_refusal`, `run_plan_refusal`, `planner_not_in_run`, `plan_revision_taken`, `plan_suffix_phase_name`, `phase_name_refusal`). In `conformance.rs`, never backtick `chk_run_plan_admitted_base_refs`, `chk_run_plan_base_position`, `idx_run_step_parent_step` or any other constraint or index name of four or more underscores. Write "the stage CHECK" and similar instead. |
| P-10 | T2's validate commands run **zero tests**: `htui-core/tests/mem_store.rs` is `#![cfg(feature = "test-support")]`, and `htui-core` does not turn the feature on for itself. | `mem_store.rs:6`; `htui-core/Cargo.toml` `[features] test-support = ["demo"]` | **adopted**: every `-p htui-core` test command touching conformance carries `--features test-support` (§T2.6). |
| P-11 | D1 indexes `parent_step_id` but not `run_plan.planner_step_id`, whose `ON DELETE CASCADE` scans `run_plan` once per deleted `run_step` (a project delete deletes them all). | MOD-163 P-10 (`0022`'s `idx_memory_entry_superseded_by`, same reason) | **adopted**: `idx_run_plan_planner_step`. `idx_run_step_parent_step` is partial (`WHERE parent_step_id IS NOT NULL`): every pre-0028 row is NULL. |
| P-12 | D1's "COMMENT ON the new table and every new column" leaves open which `run_plan` columns get one. `0022` commented five of its columns, the ones whose meaning is not their name. | `0022_memory.sql` §4; `migrations.rs:777-805` | **adopted**: the table comment, the three `run_step` columns, and seven `run_plan` columns (`revision`, `planner_step_id`, `status`, `base_position`, `base_refs`, `digest`, `updated_at`): ten column comments, pinned as `MOD139_COLUMN_COMMENTS`. `id`, `run_id`, `scope`, `body` and `created_at` carry none. The comments test asserts "exactly these and no others" per table (`migrations.rs:1130-1176`), so the list is closed. |
| P-13 | The doc counts in `store/worker.rs` move. | `store/worker.rs:15-23`, `:147`, `:230` | **adopted**: "`WorkerStore` is 65 methods: 16 `ReadStore` reads (… MOD-139 M1 adds `run_plans`) … 27 writes (MOD-139 M1: `insert_run_plan`)". Section comments `// -- 16 ReadStore reads` and `// -- 27 writes`. |

**Not adopted. Flagged for the coordinator (no M1 change):**
- **F-1.** `chk_run_step_stage` does not bind a key to its position (`pick` ↔ −3, `prepare` ↔ −2,
  `collect` ↔ −1). ANA-33 §7.11 lists both, and D1 as written (probed in C9) does not bind them.
  Binding them is one more `AND` and one more refusal sentence. Left to MOD-126/MOD-132 if they want it.
- **F-2.** `insert_run_plan` does not check that `planner_step_id`'s `role` is `planner`. D4's refusal
  list does not include it, and M2's own tests insert revisions under steps M2 creates. MOD-140 may want it.

**Main-thread rulings (2026-11-10):**
- **F-1: not adopted.** The CHECK stays as above. If MOD-126 or MOD-132 want each key tied to its
  position, they add it with their own migration.
- **F-2: rejected.** ANA-33 §7.6 lets *any* agent step submit a `successors` plan when
  `runtime_changes` is not `off`, so `planner_step_id` is not always a `planner` row. A role check
  would refuse MOD-140's run-time changes.

---

## 1. Shared decisions for every task

- **Names.** Table `run_plan`. Rust: `RunPlan`, `NewRunPlan`, `RunPlanId`, `PlanStatus`, `StepRole`,
  `PLAN_SUFFIX`, `STAGE_KEYS`. Read `ReadStore::run_plans`, write `WriteStore::insert_run_plan`.
  `DeleteReach.run_plans`, label `"run plan"` / `"run plans"`. `NotFound` entities: `"run"` (an unknown
  run on `insert_run_plan`). Duplicate id: `already_exists("run_plan", id)`.
- **`insert_run_plan` decision order** (D4 with P-7, P-8), on every store:
  1. `Constraint(run_plan_refusal(..))`, before any read;
  2. `NotFound { entity: "run", id: run }`;
  3. `Fenced { step: plan.planner_step_id }` when the run's lease owner ≠ `fence.owner()`;
  4. `Constraint(planner_not_in_run(step, run))` when the step is unknown or belongs to another run;
  5. `Constraint(already_exists("run_plan", id))` for a duplicate id, or
     `Constraint(plan_revision_taken(run, revision))` for a taken `(run_id, revision)`. These two are
     not ordered between themselves (MemStore checks the id first, and PgStore answers whichever
     `23505` the insert raises). No case combines them.
- **`create_step` decision order.** `run_step_refusal` first, before any read, on both stores (P-7).
  After it, MemStore checks the duplicate id, then the run, the agent, the parent step
  (`references_no_row("run_step.parent_step_id", parent, "run_step")`) and the slot. PgStore answers
  whichever `23xxx` its insert raises, raw (`map_sqlx`), as it already does for the run and agent FKs.
  Conformance therefore matches `Constraint(_)` for an unknown parent, and the exact sentence for
  every `run_step_refusal` case.
- **Clock.** `insert_run_plan` writes `created_at == updated_at`: MemStore from one `self.now()`
  reading, PgStore from `now()` on both column defaults. A later status move (M2, or raw SQL in T3's
  test) moves `updated_at` through `trg_run_plan_updated_at` (`clock_timestamp()`).
- **The engine is unchanged (D7).** Every engine `NewRunStep` literal writes
  `role: StepRole::Step, parent_step_id: None, node_key: None`. Nothing reads `run_plans` or calls
  `insert_run_plan` in M1.
- **Stubs (T1).** MemStore, PgStore and CacheStore bodies of both verbs are
  `Err(StoreError::Backend("MOD-139 M1: not yet".into()))` with `_`-prefixed parameters. T2 and T3
  replace them. Every forwarder is real in T1.

---

## T1 — Schema, types and the compile-green skeleton (serial)

### T1.1 Migrations (CREATE)

`crates/htui-store/migrations/0028_divided_fanout.sql`. Write it **verbatim**: the comments are pinned
byte for byte by §T3.4's `MOD139_*` constants, and a migration's bytes are checksummed once applied.

```sql
-- 0028_divided_fanout.sql - MOD-139 milestone 1 (plan D1; ANA-33 §7.2, §7.4, §7.11): the records
-- of divided fan-out. Forward-only (R-STO-5). Depends on 0001_init.sql (run, run_step,
-- set_updated_at()) and on 0020_step_graph_v2.sql's plan_scope vocabulary.
--
-- run_step gains ANA-33 §7.4's three columns, each written once by create_step and never updated:
-- role ('step' on every row written before 0028 and on every compete candidate, so nothing is
-- backfilled), parent_step_id (a part's planner step) and node_key (a part's
-- '<divided>/<part_key>', a stage's 'pick' | 'prepare' | 'collect'). chk_run_step_part makes a part
-- name both. chk_run_step_stage reserves positions -3..-1 for htui's own stage rows (§7.11): a
-- negative position is a stage row's and only a stage row's, and a stage row names one of the
-- three stages. No writer before 0028 produced a negative position, so every existing row passes.
-- node_key IS NOT NULL is spelled out: a NULL key would make the IN list NULL, and a CHECK passes
-- on NULL (probed for the M1 blueprint). M1 writes only role = 'step'; MOD-139 M2 writes planner,
-- part and integrate rows, MOD-126 and MOD-132 the stage rows.
--
-- run_plan is one revision of a run's plan (§7.2), append-only in revisions: UNIQUE (run_id,
-- revision), inserted as 'proposed'; afterwards only status moves, and base_refs is set by the
-- admission. updated_at is an addition to §7.2's columns, for that status move and the mirror's
-- cursor, under 0001's set_updated_at() trigger. An admitted revision carries its base_refs.
--
-- Mirrored: cache_migrations/0010_divided_fanout.sql adds the three columns and run_plan.
-- cache_meta.schema_version moves to 28 through PgStore::schema_version(), so each box rebuilds its
-- mirror once on first start; a headless worker never migrates: migrate from a TUI first. MOD-97's
-- sandbox may also write 0028: whichever merges second renumbers at collect, with its count pins.

-- --------------------------------------------------------------------------------------------
-- 1. run_step: role, parent_step_id, node_key
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step
  ADD COLUMN role TEXT NOT NULL DEFAULT 'step'
    CONSTRAINT chk_run_step_role
    CHECK (role IN ('step','planner','part','integrate','subgraph','consult','stage')),
  ADD COLUMN parent_step_id UUID
    CONSTRAINT fk_run_step_parent_step REFERENCES run_step(id) ON DELETE CASCADE,
  ADD COLUMN node_key TEXT,
  ADD CONSTRAINT chk_run_step_part
    CHECK (role <> 'part' OR (parent_step_id IS NOT NULL AND node_key IS NOT NULL)),
  ADD CONSTRAINT chk_run_step_stage
    CHECK ((role = 'stage') = (position < 0)
           AND (role <> 'stage'
                OR (position BETWEEN -3 AND -1
                    AND node_key IS NOT NULL
                    AND node_key IN ('pick','prepare','collect'))));
-- ON DELETE CASCADE's lookup when a step is deleted, and M2's "the parts of this planner" read.
CREATE INDEX idx_run_step_parent_step ON run_step(parent_step_id)
  WHERE parent_step_id IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 2. run_plan
-- --------------------------------------------------------------------------------------------
CREATE TABLE run_plan (
  id              UUID PRIMARY KEY,                                 -- client-minted UUIDv7
  run_id          UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CONSTRAINT chk_run_plan_revision CHECK (revision >= 0),
  planner_step_id UUID NOT NULL
                  CONSTRAINT fk_run_plan_planner_step REFERENCES run_step(id) ON DELETE CASCADE,
  scope           TEXT NOT NULL
                  CONSTRAINT chk_run_plan_scope CHECK (scope IN ('parts','successors','graph')),
  status          TEXT NOT NULL DEFAULT 'proposed'
                  CONSTRAINT chk_run_plan_status
                  CHECK (status IN ('proposed','admitted','refused','abandoned')),
  base_position   INTEGER NOT NULL
                  CONSTRAINT chk_run_plan_base_position CHECK (base_position >= 0),
  base_refs       JSONB,
  body            JSONB NOT NULL,
  digest          TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT uq_run_plan_revision UNIQUE (run_id, revision),
  CONSTRAINT chk_run_plan_admitted_base_refs
    CHECK (status <> 'admitted' OR base_refs IS NOT NULL)
);
CREATE INDEX idx_run_plan_updated_at ON run_plan(updated_at);              -- the mirror's cursor
-- ON DELETE CASCADE's lookup when a run_step is deleted (as 0022's idx_memory_entry_superseded_by)
CREATE INDEX idx_run_plan_planner_step ON run_plan(planner_step_id);

-- --------------------------------------------------------------------------------------------
-- 3. updated_at trigger, 0001 §5.1's shape (BEFORE UPDATE only)
-- --------------------------------------------------------------------------------------------
CREATE TRIGGER trg_run_plan_updated_at BEFORE UPDATE ON run_plan
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- --------------------------------------------------------------------------------------------
-- 4. Comments (pinned verbatim by tests/migrations.rs's MOD139_* constants)
-- --------------------------------------------------------------------------------------------
COMMENT ON TABLE run_plan IS
  'MOD-139 D1 (ANA-33 7.2): one revision of a run''s plan, append-only in revisions. Inserted as '
  'proposed; admission sets status and base_refs and writes the part rows in one transaction '
  '(MOD-139 M2). Mirrored.';
COMMENT ON COLUMN run_step.role IS
  'MOD-139 D1 (ANA-33 7.4, 7.11): what the step is. step on every row written before 0028 and on '
  'every compete candidate; planner, part and integrate rows belong to a divided node; stage rows '
  'are htui''s own pick, prepare and collect. Written once.';
COMMENT ON COLUMN run_step.parent_step_id IS
  'MOD-139 D1 (ANA-33 7.4): the step this one was spawned under, a part''s planner step; NULL on '
  'every other row today. Written once.';
COMMENT ON COLUMN run_step.node_key IS
  'MOD-139 D1 (ANA-33 7.4, 7.11): a part''s <divided>/<part_key>, or a stage''s pick, prepare or '
  'collect; NULL on a node of the frozen snapshot, which position names. Written once.';
COMMENT ON COLUMN run_plan.revision IS
  'MOD-139 D1: 0 for default parts the engine admits itself, then one per planner submission; '
  'unique per run.';
COMMENT ON COLUMN run_plan.planner_step_id IS
  'MOD-139 D1: the step of the same run whose planner produced this revision; insert_run_plan '
  'refuses a step of another run.';
COMMENT ON COLUMN run_plan.status IS
  'MOD-139 D1: proposed when inserted; admitted, refused or abandoned once decided. Only an '
  'admitted revision''s parts run.';
COMMENT ON COLUMN run_plan.base_position IS
  'MOD-139 D1 (ANA-33 7.2): the run_step.position of this revision''s first planned node; the '
  'i-th takes base_position + i, after every frozen node and every earlier revision.';
COMMENT ON COLUMN run_plan.base_refs IS
  'MOD-139 D1 (ANA-33 7.4): the primary HEAD per repo at admission, which every part and the '
  'integrate step branch from. NULL until admitted; an admitted revision has it.';
COMMENT ON COLUMN run_plan.digest IS
  'MOD-139 D1 (ANA-33 7.2): sha256 over the canonical body; resume recomputes it to check an '
  'admitted plan.';
COMMENT ON COLUMN run_plan.updated_at IS
  'MOD-139 D1: an addition to ANA-33 7.2''s columns. Moves with status (set_updated_at) and is '
  'the mirror''s cursor.';
```

`crates/htui-store/cache_migrations/0010_divided_fanout.sql`:

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-139 milestone 1 (plan D2; migrations/0028_divided_fanout.sql): run_step's
-- role, parent_step_id and node_key, and run_plan. No CHECK and no foreign key, as everywhere in
-- the mirror (0001_mirror.sql); JSONB as TEXT, as trim_record and usage are mirrored. run_plan is
-- a cursor table on its own updated_at, refreshed after run_step, so an offline reader answers
-- ReadStore::run_plans from the last refresh (MOD-142's parts tree). role's DEFAULT keeps this
-- applicable over a populated 0009 file: CACHE_MIGRATOR runs before the schema_version check
-- (cache/mod.rs), and the rows it keeps go on the full rebuild below.
--
-- cache_meta.schema_version moves to 28 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN role TEXT NOT NULL DEFAULT 'step';
ALTER TABLE run_step ADD COLUMN parent_step_id TEXT;
ALTER TABLE run_step ADD COLUMN node_key TEXT;

CREATE TABLE run_plan (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL, revision INTEGER NOT NULL,
  planner_step_id TEXT NOT NULL, scope TEXT NOT NULL, status TEXT NOT NULL,
  base_position INTEGER NOT NULL, base_refs TEXT, body TEXT NOT NULL, digest TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_plan_run ON run_plan(run_id, revision);
```

Names this SQL fixes, which tests and refusal mappers use: `chk_run_step_role`, `chk_run_step_part`,
`chk_run_step_stage`, `fk_run_step_parent_step`, `idx_run_step_parent_step`; `run_plan_pkey`
(Postgres's default name), `uq_run_plan_revision`, `chk_run_plan_revision`, `chk_run_plan_scope`,
`chk_run_plan_status`, `chk_run_plan_base_position`, `chk_run_plan_admitted_base_refs`,
`fk_run_plan_planner_step`, `idx_run_plan_updated_at`, `idx_run_plan_planner_step`,
`trg_run_plan_updated_at`. `run_plan.run_id`'s FK keeps Postgres's default name
(`run_plan_run_id_fkey`). Cache: `idx_cache_run_plan_run`.

### T1.2 Probe results (scratch DB `mod139_bp_probe`, all 27 migrations, then the SQL above; dropped)

| # | Statement | Outcome |
|---|---|---|
| 1 | apply `0028` over `0001..0027` | applied; 52 base tables |
| 2 | a pre-shaped insert (no role) | `role = 'step'`, `parent_step_id`/`node_key` NULL |
| 3 | planner at 1; part at 5 with parent + `implement/api`; stages `(-1, collect)`, `(-3, pick)` | accepted |
| 4 | stage with NULL key at −1 (naive CHECK without `IS NOT NULL`) | **accepted** → P-2 |
| 5 | stage at −4; stage at 0; stage key `nope`; `step` at −1 (and the NULL key with the final CHECK) | `23514 chk_run_step_stage` each |
| 6 | part without parent; part without key | `23514 chk_run_step_part` each |
| 7 | role `swarm` | `23514 chk_run_step_role` |
| 8 | unknown `parent_step_id` | `23503 fk_run_step_parent_step` |
| 9 | `run_plan` insert (no status) | `status = 'proposed'`, `base_refs` NULL |
| 10 | repeat `(run_id, revision)` | `23505 uq_run_plan_revision` |
| 11 | revision −1 / base_position −1 / scope `nodes` / admitted without refs | `chk_run_plan_revision` / `chk_run_plan_base_position` / `chk_run_plan_scope` / `chk_run_plan_admitted_base_refs` |
| 12 | body `{"a":"\u0000"}` | `22P05` "unsupported Unicode escape sequence" → P-7 |
| 13 | `UPDATE … SET status='admitted', base_refs=…` | `updated_at > created_at` (trigger) |
| 14 | delete the planner step | its plans **and** its part rows cascade away |
| 15 | delete the run | plans and steps gone |
| 16 | cache `0010` over a populated `run_step` (SQLite 3.50) | existing row reads `role = 'step'`; `run_plan` insert/select OK |

### T1.3 `crates/htui-core/src/model/` (ids, run, mod)

**`model/ids.rs`**: append to `id_newtype!` after `MemoryEntryId` (`:133`):

```rust
    /// `run_plan.id` (MOD-139 plan D1, P-1): one revision of a run's plan.
    RunPlanId,
```

**`model/run.rs`**: add `RunPlanId` to the `ids` import (`:7-10`). `PlanScope` is already imported (`:11-13`).

`RunStep` (`:322-377`): insert after `branch` (`:370-374`), before `updated_at`:

```rust
    /// `run_step.role` (MOD-139 D3, ANA-33 §7.4, §7.11): what the row is. [`StepRole::Step`] on
    /// every row written before `0028_divided_fanout.sql` and on every compete candidate and judge.
    /// Written once, by `create_step`.
    #[serde(default)]
    pub role: StepRole,
    /// `run_step.parent_step_id` (MOD-139 D3): the step this one was spawned under, a part's
    /// planner step; `None` on every other row today. Written once.
    #[serde(default)]
    pub parent_step_id: Option<StepId>,
    /// `run_step.node_key` (MOD-139 D3, ANA-33 §7.4, §7.11): a part's `<divided>/<part_key>`, or a
    /// stage's `pick` | `prepare` | `collect` ([`STAGE_KEYS`]); `None` on a node of the frozen
    /// snapshot, which `position` names. Written once.
    #[serde(default)]
    pub node_key: Option<String>,
```

`NewRunStep` (`:511-532`): append after `model`, with **no** serde default (D3; it is never decoded
from storage). Extend the struct doc with "`role`, `parent_step_id` and `node_key` must satisfy
[`run_step_refusal`](crate::store::run_step_refusal) (MOD-139 D4)".

```rust
    /// `run_step.role` (MOD-139 D3); [`StepRole::Step`] at every engine site in M1 (D7).
    pub role: StepRole,
    /// `run_step.parent_step_id` (MOD-139 D3); a part's planner step.
    pub parent_step_id: Option<StepId>,
    /// `run_step.node_key` (MOD-139 D3); a part's `<divided>/<part_key>`, a stage's key.
    pub node_key: Option<String>,
```

After `NodeRole`'s `impl` (`:965-971`), add:

```rust
str_enum!(
    /// `run_step.role` (MOD-139 D3, ANA-33 §7.4, §7.11; `chk_run_step_role`): what a step **row**
    /// is. Not [`NodeRole`], the snapshot's three-value role of a frozen node: a node's role says
    /// what the walk will create, a row's role what was created. M1 writes only `step` (D7).
    #[derive(Default)]
    StepRole {
        /// An ordinary step: every row before `0028`, every compete candidate and its judge.
        #[default]
        Step => "step",
        /// A divided node's planner, `<name>:plan` ([`PLAN_SUFFIX`]); MOD-139 M2 writes it.
        Planner => "planner",
        /// One part of an admitted plan revision: `parent_step_id` is the planner step and
        /// `node_key` is `<divided>/<part_key>` (`chk_run_step_part`); MOD-139 M2 writes it.
        Part => "part",
        /// A divided node's integrate step; MOD-139 M2 writes it, M3 runs it.
        Integrate => "integrate",
        /// A subgraph node's step (MOD-143).
        Subgraph => "subgraph",
        /// A read-only consult-mode subagent (MOD-27).
        Consult => "consult",
        /// One of htui's own stages at positions -3..-1, keyed by [`STAGE_KEYS`] (ANA-33 §7.11,
        /// `chk_run_step_stage`); MOD-126 and MOD-132 write them.
        Stage => "stage",
    }
);

/// The `node_key`s a [`StepRole::Stage`] row may carry, in reserved-position order: `pick` at -3,
/// `prepare` at -2, `collect` at -1 (ANA-33 §7.11). `chk_run_step_stage` lists the same three; the
/// CHECK does not bind a key to its position (blueprint F-1).
pub const STAGE_KEYS: [&str; 3] = ["pick", "prepare", "collect"];

/// The suffix of an expanded `divide` node's planner key, `<name>:plan` (MOD-138 D11). A phase may
/// not be named with it (MOD-139 D6, [`phase_name_refusal`](crate::store::phase_name_refusal)), so a
/// v2 snapshot's planner key never collides with a saved phase.
pub const PLAN_SUFFIX: &str = ":plan";

str_enum!(
    /// `run_plan.status` (MOD-139 D3, ANA-33 §7.2; `chk_run_plan_status`).
    PlanStatus {
        /// Written by [`insert_run_plan`](crate::store::WriteStore::insert_run_plan); not yet
        /// decided.
        Proposed => "proposed",
        /// Admitted: `base_refs` is set and its parts are rows (MOD-139 M2's admission).
        Admitted => "admitted",
        /// Refused at admission: the validator or a person said no.
        Refused => "refused",
        /// Left undecided once a later revision was admitted, or once the run ended.
        Abandoned => "abandoned",
    }
);
```

After `StepOutcome` (`:534-552`), add:

```rust
/// A row of `run_plan` (MOD-139 D1, D3; ANA-33 §7.2): one revision of a run's plan. Append-only in
/// revisions; only `status` (and, at admission, `base_refs`) moves after the insert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunPlan {
    /// `run_plan.id`.
    pub id: RunPlanId,
    /// `run_plan.run_id`.
    pub run_id: RunId,
    /// `run_plan.revision`: 0 for default parts the engine admits itself, then one per planner
    /// submission; `UNIQUE (run_id, revision)`.
    pub revision: i32,
    /// `run_plan.planner_step_id`: a step of the same run.
    pub planner_step_id: StepId,
    /// `run_plan.scope` (`step_graph_phase.plan_scope`'s vocabulary).
    pub scope: PlanScope,
    /// `run_plan.status`.
    pub status: PlanStatus,
    /// `run_plan.base_position`: the `run_step.position` of the revision's first planned node.
    pub base_position: i32,
    /// `run_plan.base_refs` (`JSONB`): the primary HEAD per repo at admission; `None` until
    /// admitted, and always set on an admitted revision (`chk_run_plan_admitted_base_refs`).
    pub base_refs: Option<Value>,
    /// `run_plan.body` (`JSONB`): the plan itself (ANA-33 §7.6). Untyped in M1 (D3): MOD-140's
    /// validator owns its shape, and M2 types default parts.
    pub body: Value,
    /// `run_plan.digest`: sha256 over the canonical body, recomputed on resume (ANA-33 §7.2).
    pub digest: String,
    /// `run_plan.created_at`.
    pub created_at: DateTime<Utc>,
    /// `run_plan.updated_at` (an addition to §7.2, plan D1): moves with `status`; the mirror's
    /// cursor.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::insert_run_plan`] (MOD-139 D3, D4): a revision inserted
/// at [`PlanStatus::Proposed`] with no `base_refs`. Must pass
/// [`run_plan_refusal`](crate::store::run_plan_refusal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRunPlan {
    /// `run_plan.id`, client-minted.
    pub id: RunPlanId,
    /// `run_plan.run_id`.
    pub run_id: RunId,
    /// `run_plan.revision`, `>= 0`.
    pub revision: i32,
    /// `run_plan.planner_step_id`: a step of `run_id`.
    pub planner_step_id: StepId,
    /// `run_plan.scope`.
    pub scope: PlanScope,
    /// `run_plan.base_position`, `>= 0`.
    pub base_position: i32,
    /// `run_plan.body`.
    pub body: Value,
    /// `run_plan.digest`.
    pub digest: String,
}
```

**`model/mod.rs`**:
- `pub use ids::{…}` (`:131`): add `RunPlanId`.
- `pub use run::{…}` (`:193-200`): add `NewRunPlan, PLAN_SUFFIX, PlanStatus, RunPlan, STAGE_KEYS,
  StepRole` (rustfmt orders them).
- Tests (in the module's `mod tests`):
  - `id_newtypes_round_trip` (`:561` area): `check_id(RunPlanId::from_uuid(u), u);`.
  - new, beside `memory_state_matches_check_list` (`:435`):
    ```rust
    /// `0028_divided_fanout.sql`'s `chk_run_step_role` (MOD-139 plan D1), ANA-33 §7.4's order.
    #[test]
    fn step_role_matches_check_list() {
        check_enum(
            StepRole::ALL,
            &["step", "planner", "part", "integrate", "subgraph", "consult", "stage"],
        );
        assert_eq!(StepRole::default(), StepRole::Step);
    }

    /// `0028_divided_fanout.sql`'s `chk_run_plan_status` (MOD-139 plan D1).
    #[test]
    fn plan_status_matches_check_list() {
        check_enum(PlanStatus::ALL, &["proposed", "admitted", "refused", "abandoned"]);
    }
    ```

**`model/run.rs` tests** (end of `mod tests`, beside `run_step_branch_defaults_when_absent`):
`run_step_role_defaults_when_absent` in `f6da9680`'s shape. Take the first fixture step, set
`role = Part`, `parent_step_id = Some(StepId::new())` and `node_key = Some("implement/api")`. It
round-trips. Remove the three keys from its JSON and it decodes as `Step`/`None`/`None`, the rest
unchanged.

### T1.4 `crates/htui-core/src/store/traits.rs`

**Imports** (`use crate::model::{…}`, `:53-73`): add `NewRunPlan, PLAN_SUFFIX, RunPlan, STAGE_KEYS,
StepRole`. `NewRunStep`, `RunId`, `StepId`, `Value` and `TemplateRole` are already in scope.

**ReadStore**: append after `owner_documents` (`:367-368`), before the trait's `}`:

```rust
    // ---- MOD-139 M1: run plans (plan D4) ------------------------------------------------------

    /// The run's `run_plan` revisions in `revision` order; empty for an unknown run. Mirrored
    /// (plan D2), so the offline mirror answers from its last refresh, as MOD-142's parts tree
    /// needs. On [`WorkerStore`](crate::store::WorkerStore) too: MOD-139 M2's admission and
    /// recovery read it.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn run_plans(&self, run: RunId) -> Result<Vec<RunPlan>>;
```

**WriteStore**: insert right after `record_branch` (`:1851`):

```rust
    /// MOD-139 D4: inserts one `run_plan` revision of `plan.run_id` at
    /// [`PlanStatus::Proposed`](crate::model::PlanStatus::Proposed) with no `base_refs`, under
    /// `fence` (as [`write_step_document`](Self::write_step_document) writes under one), and answers
    /// the stored row, `created_at == updated_at`. Admission (the status move, `base_refs`, the part
    /// rows) is MOD-139 M2's one-transaction verb, not this one. MOD-140's `plan_submit` and M2's
    /// revision-1 test path write through it.
    ///
    /// # Errors
    /// In this order: [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// ([`run_plan_refusal`]) for a negative `revision` or `base_position` or a NUL, judged before
    /// any read; [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`;
    /// [`StoreError::Fenced`](crate::store::StoreError::Fenced) `{ step: plan.planner_step_id }`
    /// when the run does not carry `fence`'s lease, nothing written;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) ([`planner_not_in_run`])
    /// when `planner_step_id` is unknown or a step of another run; then `Constraint` for a
    /// duplicate id (`already_exists("run_plan", id)`) or a taken `(run_id, revision)`
    /// ([`plan_revision_taken`]). Every text is the same on every backend; the last two are not
    /// ordered between themselves.
    async fn insert_run_plan(&self, fence: StepFence, plan: NewRunPlan) -> Result<RunPlan>;
```

**`create_step`'s doc** (`:1673-1679`) becomes:

```rust
    /// Inserts a step at `pending`. `fanout_index = -1` is the judge and is accepted. `role`,
    /// `parent_step_id` and `node_key` (MOD-139 D3) are written once, here.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) first for a row
    /// [`run_step_refusal`] refuses (a part without its parent or key; a stage off -3..-1 or off
    /// [`STAGE_KEYS`](crate::model::STAGE_KEYS); a negative position on any other role; a NUL key),
    /// judged before any read and worded alike on every backend; then on an unknown run, agent or
    /// parent step (foreign key, like [`append_events`](WriteStore::append_events)), a duplicate
    /// id, or a repeat of `(run_id, position, attempt, fanout_index)`.
```

**`StepFence`'s doc** (`:3690-3704`): after "and [`WriteStore::record_branch`] (MOD-138 M2 D5)", add
", and [`WriteStore::insert_run_plan`] (MOD-139 D4, fenced on the plan's run and reported as
`Fenced { step: planner_step_id }`)".

**Refusal helpers**: put a new block after `reserved_phase_name` (`:2730-2736`):

```rust
// ---- MOD-139: divided fan-out's records (plan D1, D4, D6) ----

/// MOD-139 D6: a phase named `<x>:plan` would collide with the planner node a `divide` phase
/// expands into ([`PLAN_SUFFIX`], MOD-138 D11).
#[must_use]
pub fn plan_suffix_phase_name(name: &str) -> String {
    format!("`{name}` ends in `{PLAN_SUFFIX}`, which names a divided phase's planner, not a phase")
}

/// MOD-139 D6: why `name` may not name a phase, or `None` when it may: a reserved template name
/// first ([`reserved_phase_name`]), then the planner suffix ([`plan_suffix_phase_name`]). Rule c of
/// `graph_body_shape_refusal` and both stores' `create_phase` / `update_phase` ask it. Resolve keeps
/// its own collision refusal (`expand_divide`), since a graph saved before 0028 may hold one.
#[must_use]
pub fn phase_name_refusal(name: &str) -> Option<String> {
    if TemplateRole::of_name(name) != TemplateRole::Phase {
        return Some(reserved_phase_name(name));
    }
    name.ends_with(PLAN_SUFFIX)
        .then(|| plan_suffix_phase_name(name))
}

/// MOD-139 D1, D4 (blueprint P-7): `chk_run_step_part` and `chk_run_step_stage` of
/// `0028_divided_fanout.sql`, plus a `node_key` NUL Postgres `text` cannot hold, in the one sentence
/// both stores give; `None` when the row may be written. Judged before any read. In order: a part
/// names its parent step and its node key; a stage sits at -3..-1; a negative position is a stage's
/// only; a stage's key is one of [`STAGE_KEYS`]; then the NUL.
#[must_use]
pub fn run_step_refusal(new: &NewRunStep) -> Option<String> {
    let stage = new.role == StepRole::Stage;
    if new.role == StepRole::Part && (new.parent_step_id.is_none() || new.node_key.is_none()) {
        return Some("a part step names its parent step and its node key".to_owned());
    }
    if stage && !(-3..=-1).contains(&new.position) {
        return Some(format!(
            "a stage step sits at position -3, -2 or -1, not {}",
            new.position
        ));
    }
    if !stage && new.position < 0 {
        return Some(format!(
            "position {} is reserved for stage steps, not a {} step",
            new.position, new.role
        ));
    }
    if stage
        && !new
            .node_key
            .as_deref()
            .is_some_and(|key| STAGE_KEYS.contains(&key))
    {
        return Some(format!(
            "a stage step's node key is pick, prepare or collect, not {}",
            new.node_key
                .as_deref()
                .map_or_else(|| "none".to_owned(), |key| format!("`{key}`"))
        ));
    }
    new.node_key
        .as_deref()
        .is_some_and(|key| key.contains('\0'))
        .then(|| has_nul("run_step.node_key"))
}

/// MOD-139 D1, D4 (blueprint P-7): [`WriteStore::insert_run_plan`]'s input rules, judged before
/// any read, in the sentence both stores give: `chk_run_plan_revision`, the base-position CHECK,
/// then a NUL in `digest` (Postgres `text`, `22021`) or in any string or key of `body` (Postgres
/// `jsonb` refuses `\u0000`, `22P05`; probed).
#[must_use]
pub fn run_plan_refusal(plan: &NewRunPlan) -> Option<String> {
    if plan.revision < 0 {
        return Some(format!(
            "run_plan.revision must be 0 or more, not {}",
            plan.revision
        ));
    }
    if plan.base_position < 0 {
        return Some(format!(
            "run_plan.base_position must be 0 or more, not {}",
            plan.base_position
        ));
    }
    if plan.digest.contains('\0') {
        return Some(has_nul("run_plan.digest"));
    }
    json_has_nul(&plan.body).then(|| has_nul("run_plan.body"))
}

/// Whether any string or object key inside `value` holds U+0000.
fn json_has_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(json_has_nul),
        Value::Object(map) => map
            .iter()
            .any(|(key, item)| key.contains('\0') || json_has_nul(item)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// MOD-139 D4: `insert_run_plan`'s `planner_step_id` names no step of the plan's run (an unknown
/// step included).
#[must_use]
pub fn planner_not_in_run(step: StepId, run: RunId) -> String {
    format!("planner step {step} is not a step of run {run}")
}

/// MOD-139 D1, D4: `uq_run_plan_revision`.
#[must_use]
pub fn plan_revision_taken(run: RunId, revision: i32) -> String {
    format!("run {run} already has plan revision {revision}")
}
```

(`has_nul` is defined below this block, at `:2830`. Order inside the file does not matter.)

**Unit tests** (`traits.rs`'s `mod tests`, or the nearest existing one):
- `phase_name_refusal_names_the_template_role_first`: `judge` gives `reserved_phase_name`,
  `implement:plan` gives `plan_suffix_phase_name`, and `implement`/`plan` give `None`.
- `run_step_refusal_mirrors_the_0028_checks`: one row per §T1.2 probe 5–7 shape, with the exact
  sentence, plus `None` for a planner at 1, a part with both fields, each of the three stages at its
  position, and a `step` at 0.
- `run_plan_refusal_refuses_negatives_and_nul`: revision −1, base −1, digest with NUL, body
  `{"a": ["x\u{0}"]}` and body `{"k\u{0}": 1}`, and `None` for a clean plan.

**`DeleteReach`** (`:3759-3827`): append after `memory_revisions`:

```rust
    /// `run_plan` rows of the project's runs, every status (MOD-139 plan D5).
    pub run_plans: u64,
```

Add "MOD-139's `run_plans` is counted on both stores (plan D5)." to the type doc (`:3749-3757`).
`impl Add` (`:3830`): add `run_plans` to the destructure and `run_plans: run_plans + other.run_plans`.

**`crates/htui-core/src/store/mod.rs`** (P-5a): add `phase_name_refusal, plan_revision_taken,
plan_suffix_phase_name, planner_not_in_run, run_plan_refusal, run_step_refusal` to
`pub use traits::{…}` (`:22-52`). rustfmt orders them.


### T1.5 `crates/htui-core/src/store/worker.rs` (P-13)

- Module doc (`:15-23`): "`WorkerStore` is 65 methods: 16 [`ReadStore`] reads (MOD-163 D12 adds
  `memory_entries`, MOD-139 M1 `run_plans`), 7 [`WriteStore`] reads, 27 writes (MOD-139 M1 adds
  `insert_run_plan`), …". The rest stays.
- Imports (`:32-45`): add `NewRunPlan, RunPlan`.
- `WorkerStore` (`:146`): change `// -- 15 ReadStore reads` (`:147`) to `16`, and add after
  `memory_entries` (`:197-204`):
  ```rust
      /// [`ReadStore::run_plans`]: MOD-139 M2's admission and recovery read the run's plans.
      fn run_plans(&self, run: RunId) -> impl Future<Output = Result<Vec<RunPlan>>> + Send;
  ```
  Change `// -- 26 writes` (`:230`) to `27`, and add after `record_branch` (`:351-356`):
  ```rust
      /// [`WriteStore::insert_run_plan`].
      fn insert_run_plan(
          &self,
          fence: StepFence,
          plan: NewRunPlan,
      ) -> impl Future<Output = Result<RunPlan>> + Send;
  ```
- `impl WorkerStore for MemStore` (`:711`): after `memory_entries` (`:764-770`)
  `async fn run_plans(&self, run: RunId) -> Result<Vec<RunPlan>> { ReadStore::run_plans(self, run).await }`.
  After `record_branch` (`:911-913`)
  `async fn insert_run_plan(&self, fence: StepFence, plan: NewRunPlan) -> Result<RunPlan> { WriteStore::insert_run_plan(self, fence, plan).await }`.

### T1.6 Every implementor and forwarder (complete: `git grep '(ReadStore|WriteStore|WorkerStore) for'`)

| # | File | Anchor | T1 body |
|---|---|---|---|
| 1 | `htui-core/src/store/mem.rs` | `impl ReadStore for MemStore` `:9550`, after `owner_documents` `:9701` | stub `run_plans` |
| 2 | ″ | `impl WriteStore for MemStore` `:9706`, after `record_branch` `:10384` | stub `insert_run_plan` |
| 3 | `htui-core/src/store/worker.rs` | `impl WorkerStore for MemStore` `:711` | forward (§T1.5) |
| 4 | `htui-store/src/pg/read.rs` | `impl ReadStore for PgStore` `:83`, after `owner_documents` `:1169` | stub (no `query!`, so no `.sqlx` churn) |
| 5 | `htui-store/src/pg/write.rs` | `impl WriteStore for PgStore` `:1751`, after `record_branch` `:7796` | stub |
| 6 | `htui-store/src/cache/read.rs` | `impl ReadStore for CacheStore` `:397`, after `owner_documents` `:1308` | stub (T3 makes it real) |
| 7 | `htui-store/src/worker.rs` | `impl WorkerStore for PgStore` `:116`: after `memory_entries` `:169`, after `record_branch` `:316` | `ReadStore::run_plans(self, run).await` / `WriteStore::insert_run_plan(self, fence, plan).await` |
| 8 | ″ | `impl WorkerStore for Writer` `:516`: after `memory_entries` `:569`, after `record_branch` `:716` | same UFCS forwards |
| 9 | `htui-store/src/writer.rs` | `impl ReadStore for Writer` `:131`, after `owner_documents` `:357`; `impl WriteStore for Writer` `:368`, after `record_branch` `:1257` | `match self { Self::Memory(store) => store.…, Self::Online(pg) => pg.… }` |
| 10 | `htui-store/src/backend.rs` | `impl ReadStore for Backend` `:896`, after `owner_documents` `:1120` | three arms: `Self::Memory(store)`, `Self::Online { pg, .. }`, `Self::Offline { cache, .. }` (no `WriteStore` for `Backend`) |
| 11 | `htui-agent/src/conformance.rs` | `ReadStore for UsageSpy` `:645`, after `owner_documents` `:769`; `WriteStore` `:777`, after `record_branch` `:1334` | `self.inner.run_plans(run).await` / `self.inner.insert_run_plan(fence, plan).await` |
| 12 | `htui-agent/tests/recorder.rs` | `ReadStore for SpyStore` `:332`, after `owner_documents` `:457`; `WriteStore` `:465`, after `record_branch` `:1060` | same |

Each file's model import list gains `RunPlan` (and `NewRunPlan` where `insert_run_plan` is
written). `htui-worker/src/runtime.rs` is not touched (P-4).

### T1.7 The `:plan` refusal at its five sites (D6)

| Site | Today | T1 |
|---|---|---|
| `htui-core/src/store/graph_rules.rs:125-131` (rule c) | `.find(\|node\| TemplateRole::of_name(..) != TemplateRole::Phase)` → `reserved_phase_name` | `if let Some(refusal) = body.nodes.iter().find_map(\|node\| phase_name_refusal(&node.phase.name)) { return Some(refusal); }` |
| `htui-core/src/store/mem.rs:3900-3902` (`State::create_phase`) | `if TemplateRole::of_name(&phase.name) != …` | `if let Some(refusal) = phase_name_refusal(&phase.name) { return Err(StoreError::Constraint(refusal)); }` |
| `htui-core/src/store/mem.rs:4062-4066` (`State::update_phase`) | `if let Some(name) = &patch.name && TemplateRole::of_name(name) != …` | `if let Some(refusal) = patch.name.as_deref().and_then(phase_name_refusal) { return Err(StoreError::Constraint(refusal)); }`, at the same place (after the CAS read, as today) |
| `htui-store/src/pg/write.rs:4014-4016` (`create_phase`) | as Mem | as Mem |
| `htui-store/src/pg/write.rs:4127-4140` (`update_phase`) | `if let Some(name) = &patch.name && TemplateRole::of_name(name) != … { NotFound / Stale reads; Err(reserved_phase_name(name)) }` | `if let Some(refusal) = patch.name.as_deref().and_then(phase_name_refusal) { …the same NotFound / Stale reads…; return Err(StoreError::Constraint(refusal)); }` |

Also:
- `graph_rules.rs:77`'s doc: "c. per node, a name [`phase_name_refusal`] refuses (a reserved
  template name, or the `:plan` suffix, MOD-139 D6)". Import `phase_name_refusal` from
  `crate::store::traits`. Drop `reserved_phase_name` from the **non-test** import (`:19`). The tests
  module imports it on its own (`:688`). Drop the `TemplateRole` import (`:9`) if nothing else uses it
  (today rule c is its only use).
- `mem.rs` and `pg/write.rs`: drop `TemplateRole` and `reserved_phase_name` from their imports once
  unused (each file's only uses are these two sites), and import `phase_name_refusal`. Otherwise the
  featureless clippy fails on `unused_imports`.
- `htui-orch/src/graph.rs:838-839`: delete `const PLAN_SUFFIX` and its doc, and add `PLAN_SUFFIX` to the
  `use htui_core::model::{EdgeKind, …}` block (`:25-28`). `not_yet_runnable` (`:719`) and
  `expand_divide` (`:899`, `:920`) keep using the name. `expand_divide`'s collision refusal stays (D6).

### T1.8 `RunStep` end to end (the `f6da9680` shape)

**PgStore**: the three `query_as!(RunStep, …)` statements gain, after `branch,` and before
`updated_at`:

```sql
                   role                AS "role: StepRole",
                   parent_step_id      AS "parent_step_id: StepId",
                   node_key,
```

The three statements are `pg/read.rs:749` (`run_steps`), `pg/read.rs:1441` (`waiting_candidates`'
step read) and `pg/write.rs:7115` (`create_step`'s `RETURNING`). Import `StepRole` in both files.
`create_step` (`pg/write.rs:7113-7155`) becomes:

```rust
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep> {
        if let Some(refusal) = run_step_refusal(&new) {
            return Err(StoreError::Constraint(refusal));
        }
        sqlx::query_as!(
            RunStep,
            r#"
            INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name,
                                  agent_id, model, status, role, parent_step_id, node_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10, $11)
            RETURNING …every column as today…, branch,
                      role                AS "role: StepRole",
                      parent_step_id      AS "parent_step_id: StepId",
                      node_key,
                      updated_at
            "#,
            …the eight binds as today…,
            new.role.as_str(),
            new.parent_step_id.map(StepId::as_uuid),
            new.node_key,
        )
        …
```

Its doc gains "[`run_step_refusal`] first, before any statement (blueprint P-7)".

**Mirror read** (`cache/read.rs:308-345`): `RUN_STEP_SELECT` ends
`…, promoted_at, branch, role, parent_step_id, node_key, updated_at`. `run_step_of` gains:

```rust
        role: get::<StepRole>(row, "role")?,
        parent_step_id: opt_uuid_col::<StepId>(
            "run_step.parent_step_id",
            opt_text(row, "parent_step_id")?.as_deref(),
        )?,
        node_key: opt_text(row, "node_key")?,
```

**Mirror refresh** (`cache/refresh.rs:1187-1275`): `RUN_STEP_COLUMNS` gains `"role"`,
`"parent_step_id"` and `"node_key"` after `"branch"`. `refresh_run_step`'s SELECT adds
`role, parent_step_id, node_key` between `branch` and `updated_at`. The binds after
`opt_json_text(row.branch…)`, in that order, are `.bind(&row.role)`,
`.bind(row.parent_step_id.map(|id| id.to_string()))` and `.bind(row.node_key.as_deref())`.

**MemStore** (`mem.rs`): `State::create_step`'s literal (`:6687`) takes `role: new.role,
parent_step_id: new.parent_step_id, node_key: new.node_key`. The refusals are T2's.
`State::start_chat_run`'s literal (`:2874`) takes `role: StepRole::Step, parent_step_id: None,
node_key: None`. PgStore's `start_chat_run` SQL relies on the column default.

**`.sqlx`**: four entries are replaced (the three `query_as!(RunStep)` statements and
`refresh_run_step`'s `query!`): the four files `f6da9680` touched, now re-hashed. Recipe in §T3.6.

### T1.9 The literal sweep (D7; follow the compiler, these are today's sites)

`NewRunStep {`: 31 sites. Each one gains `role: StepRole::Step, parent_step_id: None, node_key: None`.
Import `StepRole` where the file imports `NewRunStep`, or write `htui_core::model::StepRole::Step`
where the literal is already path-qualified.

| File | Lines |
|---|---|
| `crates/htui-agent/tests/relay.rs` | 166 |
| `crates/htui-core/src/store/conformance.rs` | 5439 (`new_run_step`) |
| `crates/htui-core/src/store/mem.rs` | 14139 (`new_step`, tests) |
| `crates/htui-mcp/tests/tools_backlog.rs` | 110 |
| `crates/htui-orch/src/conformance.rs` | 10009, 13671, 14221 |
| `crates/htui-orch/src/engine.rs` | 4552, 6555 (production), 11036, 12515, 12826, 23378, 23466, 24019, 24229 |
| `crates/htui-orch/src/gate.rs` | 2754 |
| `crates/htui-orch/tests/review_loop.rs` | 112 |
| `crates/htui-store/tests/cache.rs` | 712 |
| `crates/htui-store/tests/pg_criteria.rs` | 7208, 7351, 7805, 8712, 10745, 11081 |
| `crates/htui/src/queue_overview.rs` | 345 |
| `crates/htui/src/run_worker.rs` | 1914, 4381 |
| `crates/htui/tests/backlog.rs` | 1693, 1730 |
| `crates/htui/tests/waiting.rs` | 134 |

`RunStep {` full literals: seven sites. Each one gains the same three fields.
- `crates/htui-core/src/fixtures.rs:1784`, `:1840`, `:1881`;
- `crates/htui-core/src/store/mem.rs:2874`, `:6687` (above);
- `crates/htui-orch/src/recover.rs:911`;
- `crates/htui-store/src/cache/read.rs:316` (`run_step_of`, above).

Struct-update literals compile unchanged: `conformance.rs:16466`, `:23832`; `engine.rs:17397`,
`:18125`; `select.rs:815`; `run.rs:2705`.

### T1.10 Reach, compile half (D5)

- MemStore `project_reach` literal (`mem.rs:5508-5646`, exhaustive): `run_plans: 0,` after
  `memory_revisions` (T2 counts).
- PgStore `project_reach` map (`pg/write.rs:678-709`, exhaustive): `run_plans: 0,` (T3 counts). The SQL
  is untouched in T1.
- `crates/htui/src/hierarchy.rs:795`: `labelled` returns `[(u64, &'static str, &'static str); 31]`. The
  destructure gains `run_plans`, and the array gains `(run_plans, "run plan", "run plans")` last (field
  order).
- `crates/htui-store/tests/pg_criteria.rs` (P-5c): `TABLES: [&str; 31]` (`:3329`) gains `"run_plan"`
  last. The destructure (`:3376`) and `claimed: [u64; 31]` (`:3408`) gain `run_plans` last (struct
  index 30, the positional zip). The doc becomes "Thirty-one entries for thirty-one fields … and
  MOD-139's `run_plan` closes it". It passes at runtime on `0 == 0` until T3 seeds a row.
- Every other `DeleteReach { … }` literal ends `..DeleteReach::default()` or `..**reach` and compiles
  unchanged (`conformance.rs:3512`, `:27210`, `:28075`; `mem.rs:5412`; `pg/write.rs:514`;
  `htui/src/hierarchy.rs` tests; `htui/src/ui/tabs/settings/hierarchy.rs:2028`;
  `htui/tests/hierarchy.rs`).

### T1.11 Migration pins (27 → 28, `TABLES` 51 → 52)

| File:line | Change |
|---|---|
| `htui-store/tests/migrations.rs:25-31` | doc "The 52 tables … then the two of `0022_memory.sql` (MOD-163 plan D1), then the one of `0028_divided_fanout.sql` (MOD-139 plan D1)." |
| `:32-90` (`TABLES`) | append `// 0028_divided_fanout.sql (MOD-139)` and `"run_plan",` |
| `:115-116` | applied list ends `25, 26, 27, 28`; message appends "…MOD-163 M2's 0027_trim_record_v5.sql and MOD-139's 0028_divided_fanout.sql, in ordinal order" |
| `:146` | `51` → `52`; message appends "and MOD-139's 0028_divided_fanout.sql adds run_plan" |
| `:158-160` | "the 52 tables …" + "(memory_entry, memory_entry_revision) and MOD-139's 0028_divided_fanout.sql (run_plan)" |
| `:3919-3920`, `:4020-4021`, `:4029-4030`, `:4052-4053`, `:4204-4206` | `27` → `28`; "twenty-seven" → "twenty-eight"; "through MOD-163's 0027_trim_record_v5.sql" → "through MOD-139's 0028_divided_fanout.sql" |
| `htui-store/tests/connect.rs:140-142`, `:156-158`, `:242-244` | the same three edits |

`migrations.rs:4957`'s `[(&str, usize); 27]` is the demo table count, not a migration count: no demo
rows (D8), so it is unchanged.

### T1.12 Tests to write first

| Test | File | Asserts |
|---|---|---|
| `step_role_matches_check_list`, `plan_status_matches_check_list` | `model/mod.rs` | CHECK order, `StepRole::default() == Step` |
| `id_newtypes_round_trip` (extended) | `model/mod.rs` | `RunPlanId` |
| `run_step_role_defaults_when_absent` | `model/run.rs` | §T1.3 |
| `phase_name_refusal_names_the_template_role_first`, `run_step_refusal_mirrors_the_0028_checks`, `run_plan_refusal_refuses_negatives_and_nul` | `store/traits.rs` | §T1.4 |
| `shape_c_plan_suffix` | `store/graph_rules.rs` (beside `shape_c_reserved_names` `:1080`) | `body.nodes[1].phase.name = "implement:plan"` → `Some(plan_suffix_phase_name("implement:plan"))`; `judge` still answers `reserved_phase_name` |
| `a_phase_name_with_the_plan_suffix_is_refused` | `store/mem.rs` tests | `create_phase` of `x:plan` and `update_phase` renaming to `x:plan` → `Constraint(plan_suffix_phase_name(..))`, nothing written |
| `create_phase_with_the_plan_suffix_is_refused` | `htui/tests/kinds.rs` (beside `:612`) | the reserved-name twin with `name: "implement:plan"`, message contains `plan_suffix_phase_name("implement:plan")` (MemStore through `serve`; P-3) |
| `reach_parts_names_the_run_plans` | `htui/src/hierarchy.rs` tests (beside `:1110`) | `DeleteReach { runs: 1, run_steps: 4, run_plans: 2, .. }` → `["1 run", "4 run steps", "2 run plans"]`; `reach_totals == (7, 3)` |
| `cache_0010_applies_over_a_populated_0009_mirror` | `htui-store/tests/migrations.rs` (beside `:6507`, no server, never skips) | `CACHE_MIGRATOR.run_to(9)`, insert one `run_step` row, `run` → the row reads `role = 'step'`, `parent_step_id`/`node_key` NULL; `run_plan` exists (insert + select one row) |
| `the_0010_step_role_reaches_the_mirror` | `htui-store/tests/cache.rs` (beside `:1423`) | Through `db.store.create_step` on `ids::RUN_1`: a planner at position 50, a part at 51 (parent = planner, key `implement/api`) and a stage at −1 (`collect`). Then `run_pass`. `cache.run_steps(RUN_1) == db.store.run_steps(RUN_1)`, and the three rows' role, parent and key come off the mirror. Every fixture step reads `Step`/`None`/`None` |

### T1.13 Validate

```bash
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cargo test -p htui-core --lib model::
cargo test -p htui-core --lib store::graph_rules
cargo test -p htui-core --lib store::traits
cargo test -p htui-core --lib store::mem::tests::a_phase_name_with_the_plan_suffix_is_refused
cargo test -p htui --lib hierarchy::tests::reach_parts
cargo test -p htui --features testkit --test kinds create_phase_with
# BASE and mod139_prepare: the recipe in §T3.6
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
cargo test -p htui-store --all-features --test migrations --test connect --test cache -- --test-threads=1
cargo test -p htui-store --all-features --test pg_criteria every_cascade_table_loses_exactly_what_the_report_names -- --test-threads=1
```

(`htui/tests/*.rs` need `--features testkit`, or they run zero tests.)

### T1.14 Commits (each compiles; run `cargo check --workspace --all-targets --all-features` before each)

1. `feat(model): MOD-139 M1 T1, StepRole, PlanStatus, RunPlan and the :plan suffix refused at save`.
   This commit holds `ids.rs` `RunPlanId`, the `run.rs` types (`StepRole`, `PlanStatus`, `STAGE_KEYS`,
   `PLAN_SUFFIX`, `RunPlan`, `NewRunPlan`; **no** `RunStep`/`NewRunStep` field yet), the `mod.rs`
   re-exports, the pins and the id test. From `traits.rs` it takes `plan_suffix_phase_name`,
   `phase_name_refusal`, `run_plan_refusal`, `json_has_nul`, `planner_not_in_run` and
   `plan_revision_taken`, with their tests, plus the `store/mod.rs` re-exports. It also carries the
   five sites of §T1.7, `graph.rs`'s import, `shape_c_plan_suffix`, the mem.rs unit test and the
   `kinds.rs` test. There is no DB change, so no `.sqlx`.
2. `feat(store): MOD-139 M1 T1, 0028_divided_fanout.sql and run_step role, parent and node key end to end`.
   This commit holds both migrations, the three fields on `RunStep`/`NewRunStep` and the §T1.9 sweep.
   It adds `run_step_refusal` with its test and Pg `create_step`, and covers the Pg selects, the mirror
   (`RUN_STEP_SELECT`, `run_step_of`, `RUN_STEP_COLUMNS`, `refresh_run_step`), the four `.sqlx`
   entries and the §T1.11 pins. Its tests are `run_step_role_defaults_when_absent`,
   `cache_0010_applies_over_a_populated_0009_mirror` and `the_0010_step_role_reaches_the_mirror`.
3. `feat(store): MOD-139 M1 T1, run_plans and insert_run_plan on every store, DeleteReach.run_plans`.
   This commit holds the trait verbs, the `create_step`/`StepFence` doc edits, `WorkerStore`, the
   twelve forwarder and stub sites of §T1.6, `DeleteReach.run_plans` and `Add`, the two zero counts,
   `labelled` with its test, and the `pg_criteria.rs` compile edit.

---

## T2 — MemStore and the conformance cases (parallel with T3)

Files: `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/tests/pg_conformance.rs`. Write the cases
first: they fail on MemStore's T1 stubs. Then make them pass.

### T2.1 Conformance cases (`conformance.rs`)

Append to `CASES` (`:87-347`) under `// MOD-139 M1: divided fan-out's records (plan D1, D4-D6)`. Add
one **boxed** arm each in `run_case_inline` (`:368-1096`), as the others are (the stack note at
`:356-363`). Put the bodies after the MOD-147 cases, with a
`// ---- MOD-139 M1: …` header. Fixtures: `leased_step(case, store, a, at)` (`:8351`, a run of
`HTUI_ANA_2` claimed by `a` with one step at position 0), `new_run_step` (`:5438`, now with
`Step`/`None`/`None`), `seam_clock`, `new_run`. A local helper
`fn new_run_plan(run: RunId, planner: StepId, revision: i32) -> NewRunPlan` gives
`scope: PlanScope::Parts`, `base_position: 5 + revision`, `body: json!({"scope": "parts", "parts": [{"key": "api"}]})`
and `digest: format!("sha256:case-{revision}")`.

| # | Case | Asserts |
|---|---|---|
| 1 | `create_step_writes_role_parent_and_node_key` | On a `leased_step` run, `create_step` writes a planner at 1, a part at 5 (parent = planner, key `implement/api`) and the three stages at −3/−2/−1 (`pick`/`prepare`/`collect`), and each returned row carries what was asked. `run_steps(run)` returns those rows equal to the returns, the stages first (position order). The `leased_step` row reads `Step`/`None`/`None`. |
| 2 | `create_step_refuses_a_part_without_parent_or_key` | A part without a parent, and a part without a key, are each `Constraint(run_step_refusal(&new).unwrap())`, compared **exactly**, and `run_steps` is unchanged after each. |
| 3 | `create_step_reserves_negative_positions_for_stages` | A stage at 0, at −4, with key `nope` and with key `None`, and a `step` at −1 and a `planner` at −2. Each is `Constraint` with the exact `run_step_refusal` sentence, and nothing is written. |
| 4 | `create_step_refuses_an_unknown_parent_step` | A part whose parent is `StepId::new()` is `Constraint(_)` (the FK text differs per store, §1). Nothing is written. |
| 5 | `insert_run_plan_writes_a_proposed_revision` | Insert revision 1, then revision 0 (out of order), under `StepFence::Lease(a)`. Each return has `status == Proposed`, `base_refs == None`, `created_at == updated_at`, and `id`/`run_id`/`revision`/`planner_step_id`/`scope`/`base_position`/`body`/`digest` as given. `run_plans(run) == [rev0, rev1]` (revision order, equal to the returns). `run_plans(RunId::new())` is `Ok(vec![])`. |
| 6 | `insert_run_plan_refuses_a_taken_revision_or_id` | After revision 0: a second revision 0 with a fresh id is `Constraint(plan_revision_taken(run, 0))`, exactly; revision 1 with the first plan's id is `Constraint(already_exists("run_plan", id))`, exactly. `run_plans(run)` is the one row after each. |
| 7 | `insert_run_plan_refusal_order` | (a) The shape comes first: revision −1 on an **unknown** run is `Constraint(run_plan_refusal(..))`, not `NotFound`. (b) An unknown run is `NotFound { entity: "run", .. }`. (c) `Lease(b)`, and then `Unleased`, on `a`'s run are both `Fenced { step }` with `step == planner_step_id`; this holds with a foreign planner too, so the fence is decided before the planner. (d) A planner that is a step of **another** run (`create_run(new_run(..))` + `create_step`, unclaimed) is `Constraint(planner_not_in_run(step, run))`, exactly, and so is an unknown planner step. (e) `run_plans(run)` stays empty throughout. |
| 8 | `project_delete_takes_the_run_plans` | Two plans on a `leased_step` run (project `htui`). `delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))` has `run_plans == 2`, the report of `delete_project` equals it, and `run_plans(run)` is then empty. |
| 9 | `phase_name_with_the_plan_suffix_is_refused` (P-3) | On a project graph: `create_phase` of `implement:plan` is `Constraint(plan_suffix_phase_name("implement:plan"))`, exactly. So is `update_phase` renaming a phase to it (with the right CAS token), and `replace_graph` with a node of that name (rule c). Nothing is written (the graph's `revision` and phases are unchanged). `judge` keeps answering `reserved_phase_name("judge")`, so the order holds. |

Never backtick a constraint or index name of four or more underscores in this file (P-9). Import
`NewRunPlan, PlanScope, PlanStatus, RunPlanId, StepRole` and the helpers `plan_revision_taken`,
`plan_suffix_phase_name`, `planner_not_in_run`, `run_plan_refusal` and `run_step_refusal`.

### T2.2 `State` (`mem.rs:210-`; `from_demo` `:445-490`)

- Field after `memory_revisions` (`:366`): `/// MOD-139 D1: \`run_plan\`, append-only in revisions.`
  `run_plans: Vec<RunPlan>,`. `from_demo`: `run_plans: Vec::new(),` (D8: no demo rows).
- `State::create_step` (`:6653`): **first**, `if let Some(refusal) = run_step_refusal(&new) { return
  Err(StoreError::Constraint(refusal)); }`. Then the existing duplicate-id, run and agent checks. After
  the agent check, add:
  ```rust
        if let Some(parent) = new.parent_step_id
            && !self.steps.contains_key(&parent)
        {
            return Err(StoreError::Constraint(references_no_row(
                "run_step.parent_step_id",
                parent,
                "run_step",
            )));
        }
  ```
  Then the slot check. The literal already copies the three fields (T1).
- New, beside `State::record_branch` (`:6868`):
  ```rust
    /// MOD-139 D4: [`WriteStore::insert_run_plan`] in Postgres's order (blueprint §1): the shape,
    /// the run, the fence, the planner step, the id, then the revision.
    fn insert_run_plan(
        &mut self,
        fence: StepFence,
        plan: NewRunPlan,
        now: DateTime<Utc>,
    ) -> Result<RunPlan> {
        if let Some(refusal) = run_plan_refusal(&plan) {
            return Err(StoreError::Constraint(refusal));
        }
        if !self.runs.contains_key(&plan.run_id) {
            return Err(StoreError::NotFound { entity: "run", id: plan.run_id.to_string() });
        }
        if self.lease_owners.get(&plan.run_id).copied() != fence.owner() {
            return Err(StoreError::Fenced { step: plan.planner_step_id });
        }
        if self.steps.get(&plan.planner_step_id).is_none_or(|step| step.run_id != plan.run_id) {
            return Err(StoreError::Constraint(planner_not_in_run(plan.planner_step_id, plan.run_id)));
        }
        if self.run_plans.iter().any(|row| row.id == plan.id) {
            return Err(StoreError::Constraint(already_exists("run_plan", plan.id)));
        }
        if self.run_plans.iter().any(|row| row.run_id == plan.run_id && row.revision == plan.revision) {
            return Err(StoreError::Constraint(plan_revision_taken(plan.run_id, plan.revision)));
        }
        let row = RunPlan { id: plan.id, run_id: plan.run_id, revision: plan.revision,
            planner_step_id: plan.planner_step_id, scope: plan.scope, status: PlanStatus::Proposed,
            base_position: plan.base_position, base_refs: None, body: plan.body, digest: plan.digest,
            created_at: now, updated_at: now };
        self.run_plans.push(row.clone());
        Ok(row)
    }

    /// MOD-139 D4: the run's plans in `revision` order.
    fn run_plans(&self, run: RunId) -> Vec<RunPlan> {
        let mut rows: Vec<RunPlan> =
            self.run_plans.iter().filter(|row| row.run_id == run).cloned().collect();
        rows.sort_by_key(|row| row.revision);
        rows
    }
  ```
  Before writing the lease line, check how `fence_holds` (`mem.rs`, used at `:6886`) reads
  `lease_owners`, and reuse that reading if it differs (for example, a run whose lease has expired).
  The rule is "`run.lease_owner IS NOT DISTINCT FROM fence.owner()`", the same as Postgres.
- `MemStore` (`ReadStore` `:9550`, `WriteStore` `:9706`): replace the T1 stubs. Mirror
  `record_branch`'s wrapper at `:10384` and its clock (`self.now()`, truncated as the other writers'
  `now` is).

### T2.3 Reach and delete

- `project_reach` (`:5446`): in the literal, `run_plans: rows(self.run_plans.iter().filter(|row|
  runs.contains(&row.run_id)).count())` replaces T1's `0`. Its doc gains "MOD-139's `run_plans` from the
  same pass (plan D5)".
- `remove_project` (`:5764`): beside `self.runs.retain` (`:5782`) add
  `self.run_plans.retain(|row| !gone.runs.contains(&row.run_id));` with
  `// MOD-139 plan D5: a plan goes with its run (and its planner step) — \`ON DELETE CASCADE\`.`
- `delete_project_leaves_no_row_in_any_map` (`:13315`): stage one `run_plan` on a `htui` run (via
  `state.run_plans.push` in its `store.write` block, as it stages relay rows), and assert
  `state.run_plans.is_empty()` after the delete.

### T2.4 Counts

- `crates/htui-core/tests/mem_store.rs:37`: `248` → `257`. Append to the message: "…, and MOD-139 M1's
  nine for divided fan-out's records (plan D1, D4-D6)".
- `crates/htui-store/tests/pg_conformance.rs:53`: `EXPECTED_CASES = 257`, with a doc line "and MOD-139
  M1's nine divided fan-out record cases (plan D1, D4-D6) make it 257". Extend the message the same way.

### T2.5 Commits

1. `test(store): MOD-139 M1 T2, nine run-plan and step-role conformance cases (red on MemStore)`.
2. `feat(store): MOD-139 M1 T2, MemStore step roles, run plans, reach and delete`.

### T2.6 Validate (P-10: the feature is required)

```bash
cargo test -p htui-core --features test-support --test mem_store -- --test-threads=1
cargo test -p htui-core --features test-support --lib conformance::
cargo test -p htui-core --features test-support --lib store::mem::tests::delete_project_leaves_no_row_in_any_map
cargo clippy -p htui-core --all-targets --all-features -- -D warnings
```

`conformance::tests::run_case_accepts_every_name_in_cases` and
`every_cross_referenced_test_name_exists` must pass (P-9). `pg_conformance` turns green only at T4,
once T3's PgStore bodies land.

---

## T3 — PgStore, the mirror and the migration tests (parallel with T2)

Files: `crates/htui-store/src/pg/read.rs`, `pg/write.rs`, `src/cache/read.rs`, `src/cache/refresh.rs`,
`src/cache/mod.rs`, `.sqlx/*`, `tests/migrations.rs`, `tests/pg_criteria.rs` and `tests/cache.rs`.

### T3.1 `pg/read.rs`: `run_plans` (replace the T1 stub)

```rust
    /// MOD-139 D4: the run's revisions, `revision` order.
    async fn run_plans(&self, run: RunId) -> Result<Vec<RunPlan>> {
        sqlx::query_as!(
            RunPlan,
            r#"
            SELECT id              AS "id: RunPlanId",
                   run_id          AS "run_id: RunId",
                   revision,
                   planner_step_id AS "planner_step_id: StepId",
                   scope           AS "scope: PlanScope",
                   status          AS "status: PlanStatus",
                   base_position,
                   base_refs,
                   body,
                   digest,
                   created_at,
                   updated_at
              FROM run_plan
             WHERE run_id = $1
             ORDER BY revision
            "#,
            run.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }
```

### T3.2 `pg/write.rs`: `insert_run_plan`, its mapper, and the reach

Put the helper in a new `// MOD-139 helpers (plan D4)` block after the MOD-163 helpers (`:712-`):

```rust
/// MOD-139 D4: `insert_run_plan`'s two `23505`s in `MemStore`'s sentences: `run_plan_pkey` is the
/// id, `uq_run_plan_revision` the revision; anything else through [`map_sqlx`].
fn run_plan_insert_refused(err: sqlx::Error, plan: &NewRunPlan) -> StoreError {
    match &err {
        sqlx::Error::Database(db) if db.constraint() == Some("run_plan_pkey") => {
            StoreError::Constraint(already_exists("run_plan", plan.id))
        }
        sqlx::Error::Database(db) if db.constraint() == Some("uq_run_plan_revision") => {
            StoreError::Constraint(plan_revision_taken(plan.run_id, plan.revision))
        }
        _ => map_sqlx(err),
    }
}
```

The verb, replacing the stub after `record_branch`:

```rust
    /// MOD-139 D4 (blueprint §1, P-8): one transaction. The planner step is read `FOR KEY SHARE`
    /// and then the run `FOR SHARE`, the step → run order of MOD-77 (`step_fence`), so an adoption
    /// cannot commit between the fence check and the insert, and a park holding the step cannot
    /// deadlock it. The decisions are made in MemStore's order on what was read; the two `23505`s
    /// map through `run_plan_insert_refused`.
    async fn insert_run_plan(&self, fence: StepFence, plan: NewRunPlan) -> Result<RunPlan> {
        if let Some(refusal) = run_plan_refusal(&plan) {
            return Err(StoreError::Constraint(refusal));
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let planner_run = sqlx::query_scalar!(
            r#"SELECT run_id AS "run_id: RunId" FROM run_step WHERE id = $1 FOR KEY SHARE"#,
            plan.planner_step_id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        let owner = sqlx::query_scalar!(
            r#"SELECT lease_owner AS "lease_owner?" FROM run WHERE id = $1 FOR SHARE"#,
            plan.run_id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound { entity: "run", id: plan.run_id.to_string() })?;
        if owner != fence.owner() {
            return Err(StoreError::Fenced { step: plan.planner_step_id });
        }
        if planner_run != Some(plan.run_id) {
            return Err(StoreError::Constraint(planner_not_in_run(plan.planner_step_id, plan.run_id)));
        }
        let row = sqlx::query_as!(
            RunPlan,
            r#"
            INSERT INTO run_plan (id, run_id, revision, planner_step_id, scope, base_position,
                                  body, digest)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING …the twelve columns as in §T3.1…
            "#,
            plan.id.as_uuid(),
            plan.run_id.as_uuid(),
            plan.revision,
            plan.planner_step_id.as_uuid(),
            plan.scope.as_str(),
            plan.base_position,
            &plan.body,
            plan.digest,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| run_plan_insert_refused(err, &plan))?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(row)
    }
```

An early `return` drops `tx`, and sqlx rolls it back. Clone `plan.digest` or bind `&plan.digest` so
`plan` stays borrowable for the mapper. If the future grows the worker stack, box the rare branch as
`fenced_or_missing`'s note asks (`:347-354`).

`project_reach` (`:612`): append to the SELECT, after `"memory_revisions!"`:

```sql
               (SELECT count(*) FROM run_plan
                 WHERE run_id IN (SELECT id FROM r))                          AS "run_plans!"
```

The map (`:678-709`) takes `run_plans: rows(row.run_plans)`. The doc (`:589-611`) gains: "`run_plan`
hangs off `r` (MOD-139 D5); its planner step is in `s` as well (`insert_run_plan` refuses a
step of another run), so its two cascade paths take the same rows." Imports in both files: `NewRunPlan,
PlanScope, PlanStatus, RunPlan, RunPlanId`, `plan_revision_taken`, `planner_not_in_run`,
`run_plan_refusal`.

### T3.3 The mirror

- `cache/refresh.rs`: `const RUN_PLAN: &str = "run_plan";` after `RUN_STEP_TREE` (`:206`). In
  `run_pass`'s list (`:254-270`), put `RUN_PLAN` after `RUN_STEP_TREE` (after `run_step`, D2). Add an
  explicit arm after `RUN_STEP_TREE`'s (`:294`): `RUN_PLAN => refresh_run_plan(pool, sqlite, project,
  since, hw).await?,`. No `_` arm (`:286-290`). After `refresh_run_step_tree`:
  ```rust
  const RUN_PLAN_COLUMNS: &[&str] = &[
      "id", "run_id", "revision", "planner_step_id", "scope", "status", "base_position",
      "base_refs", "body", "digest", "created_at", "updated_at",
  ];

  /// MOD-139 D2: `run_plan` on its own `updated_at` cursor, which a status move bumps
  /// (`trg_run_plan_updated_at`), so an admission reaches the mirror on the next pass.
  async fn refresh_run_plan(
      pool: &PgPool,
      sqlite: &SqlitePool,
      project: ProjectId,
      since: DateTime<Utc>,
      hw: i64,
  ) -> Result<Batch> {
      let rows = sqlx::query!(
          "SELECT id, run_id, revision, planner_step_id, scope, status, base_position, \
                  base_refs, body, digest, created_at, updated_at \
             FROM run_plan \
            WHERE run_id IN (SELECT id FROM run WHERE project_id = $1) AND updated_at > $2 \
            ORDER BY updated_at",
          project.as_uuid(),
          since,
      )
      .fetch_all(pool)
      .await
      .map_err(map_sqlx)?;

      let sql = upsert_sql(RUN_PLAN, RUN_PLAN_COLUMNS, 1);
      let mut tx = sqlite.begin().await.map_err(map_sqlx)?;
      for row in &rows {
          sqlx::query(AssertSqlSafe(Arc::clone(&sql)))
              .bind(row.id.to_string())
              .bind(row.run_id.to_string())
              .bind(i64::from(row.revision))
              .bind(row.planner_step_id.to_string())
              .bind(&row.scope)
              .bind(&row.status)
              .bind(i64::from(row.base_position))
              .bind(opt_json_text(row.base_refs.as_ref())?)
              .bind(json_text(&row.body)?)
              .bind(&row.digest)
              .bind(ts_bind(row.created_at))
              .bind(ts_bind(row.updated_at))
              .execute(&mut *tx)
              .await
              .map_err(map_sqlx)?;
      }
      let high_water = rows.last().map_or(hw, |row| ts_bind(row.updated_at));
      finish(tx, project, RUN_PLAN, high_water).await?;
      Ok(Batch { rows: rows.len() as u64, tombstones: 0 })
  }
  ```
- `cache/mod.rs` (P-5b): `MIRRORED_TABLES: [&str; 22]`, with `"run_plan"` after `"run_step_tree"`.
  The doc becomes "The twenty-two mirrored tables …", plus a paragraph: "`run_plan` joined in MOD-139 M1
  (plan D2): an offline reader answers `ReadStore::run_plans` off it (MOD-142's parts tree); it rides
  its own `updated_at`, which a status move bumps".
- `cache/read.rs`: after `run_step_of` (`:345`):
  ```rust
  /// Every `run_plan` column, in [`run_plan_of`]'s names.
  const RUN_PLAN_SELECT: &str = "id, run_id, revision, planner_step_id, scope, status, \
       base_position, base_refs, body, digest, created_at, updated_at";

  /// One `run_plan` row, every column of [`RUN_PLAN_SELECT`].
  fn run_plan_of(row: &SqliteRow) -> Result<RunPlan> {
      Ok(RunPlan {
          id: uuid_col::<RunPlanId>("run_plan.id", &text(row, "id")?)?,
          run_id: uuid_col("run_plan.run_id", &text(row, "run_id")?)?,
          revision: get(row, "revision")?,
          planner_step_id: uuid_col("run_plan.planner_step_id", &text(row, "planner_step_id")?)?,
          scope: get::<PlanScope>(row, "scope")?,
          status: get::<PlanStatus>(row, "status")?,
          base_position: get(row, "base_position")?,
          base_refs: opt_json_col("run_plan.base_refs", opt_text(row, "base_refs")?.as_deref())?,
          body: json_col("run_plan.body", &text(row, "body")?)?,
          digest: text(row, "digest")?,
          created_at: ts_col("run_plan.created_at", get(row, "created_at")?)?,
          updated_at: ts_col("run_plan.updated_at", get(row, "updated_at")?)?,
      })
  }
  ```
  `CacheStore::run_plans` replaces the stub, in `run_steps`' shape (`:993-1003`):
  `SELECT {RUN_PLAN_SELECT} FROM run_plan WHERE run_id = ? ORDER BY revision`, bound
  `run.to_string()`, then `rows.iter().map(run_plan_of).collect()`.

### T3.4 Migration tests (`tests/migrations.rs`)

- Comment pins. Paste these after `MOD163_TABLE_COMMENTS` (`:835-845`). They were generated from
  §T1.1's SQL and checked to reproduce it byte for byte:

```rust
/// The ten `COMMENT ON COLUMN` texts of `0028_divided_fanout.sql` (MOD-139 plan D1), verbatim and
/// in migration order, for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD139_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    (
        "run_step",
        "role",
        "MOD-139 D1 (ANA-33 7.4, 7.11): what the step is. step on every row written before 0028 \
         and on every compete candidate; planner, part and integrate rows belong to a divided \
         node; stage rows are htui's own pick, prepare and collect. Written once.",
    ),
    (
        "run_step",
        "parent_step_id",
        "MOD-139 D1 (ANA-33 7.4): the step this one was spawned under, a part's planner step; \
         NULL on every other row today. Written once.",
    ),
    (
        "run_step",
        "node_key",
        "MOD-139 D1 (ANA-33 7.4, 7.11): a part's <divided>/<part_key>, or a stage's pick, \
         prepare or collect; NULL on a node of the frozen snapshot, which position names. \
         Written once.",
    ),
    (
        "run_plan",
        "revision",
        "MOD-139 D1: 0 for default parts the engine admits itself, then one per planner \
         submission; unique per run.",
    ),
    (
        "run_plan",
        "planner_step_id",
        "MOD-139 D1: the step of the same run whose planner produced this revision; \
         insert_run_plan refuses a step of another run.",
    ),
    (
        "run_plan",
        "status",
        "MOD-139 D1: proposed when inserted; admitted, refused or abandoned once decided. Only \
         an admitted revision's parts run.",
    ),
    (
        "run_plan",
        "base_position",
        "MOD-139 D1 (ANA-33 7.2): the run_step.position of this revision's first planned node; \
         the i-th takes base_position + i, after every frozen node and every earlier revision.",
    ),
    (
        "run_plan",
        "base_refs",
        "MOD-139 D1 (ANA-33 7.4): the primary HEAD per repo at admission, which every part and \
         the integrate step branch from. NULL until admitted; an admitted revision has it.",
    ),
    (
        "run_plan",
        "digest",
        "MOD-139 D1 (ANA-33 7.2): sha256 over the canonical body; resume recomputes it to check \
         an admitted plan.",
    ),
    (
        "run_plan",
        "updated_at",
        "MOD-139 D1: an addition to ANA-33 7.2's columns. Moves with status (set_updated_at) \
         and is the mirror's cursor.",
    ),
];

/// The one `COMMENT ON TABLE` of `0028_divided_fanout.sql` (MOD-139 plan D1), verbatim.
const MOD139_TABLE_COMMENT: (&str, &str) = (
    "run_plan",
    "MOD-139 D1 (ANA-33 7.2): one revision of a run's plan, append-only in revisions. Inserted \
     as proposed; admission sets status and base_refs and writes the part rows in one \
     transaction (MOD-139 M2). Mirrored.",
);
```

  Chain them into `the_ana_column_comments_are_present_and_verbatim` (`:1063`). `MOD139_COLUMN_COMMENTS`
  goes after `MOD163_M2_COLUMN_COMMENTS` at all three column chains (`:1081`, `:1154`, `:1177`), and
  `MOD139_TABLE_COMMENT` goes into the table-comment array (`:1105-1109`). Add "MOD-139" to the two
  assertion messages and to the "nothing else" comment. That test asserts "exactly the listed
  commented columns, and no others" (P-12).
- `the_0028_step_role_checks_refuse_what_they_name` (`#[cfg(feature = "demo")]`, `demo_db`, the
  `the_branch_column_admits_an_object_of_booleans_and_null` shape). Raw SQL inserts on `ids::RUN_1`.
  §T1.2 rows 3 are accepted. Rows 5–8 are refused, each with its SQLSTATE (`23514`, or `23503` for the
  parent) **and** constraint name (`chk_run_step_stage`, `chk_run_step_part`, `chk_run_step_role`,
  `fk_run_step_parent_step`). The refused set includes **a stage at −1 with a NULL key** (P-2).
- `the_0028_run_plan_checks_refuse_what_they_name` (demo). Insert a planner step and one plan by raw
  SQL: the default `status = 'proposed'` and NULL `base_refs`. Each of §T1.2 rows 10–11 is refused with
  its name (`uq_run_plan_revision`, `23505`). Also refused: `status 'done'` → `chk_run_plan_status`.
  The admitted update moves `updated_at` (`trg_run_plan_updated_at`), and deleting the planner step
  removes the plan. `pg_indexes` lists `idx_run_step_parent_step`, `idx_run_plan_updated_at` and
  `idx_run_plan_planner_step`.
- `migration_0028_defaults_existing_steps_to_step` (`bare_db`, the `:6203` shape). Run
  `MIGRATOR.run_to(27)`. Plant `insert_user` → `insert_project` → `insert_box` →
  `insert_run(pool, project, "chat", "done", box, user)` (`:1580-1621`) and two `run_step` rows (SQL).
  Then `MIGRATOR.run`. Both rows read `role = 'step'` with NULL `parent_step_id`/`node_key`, and
  `information_schema.columns` shows `role` `NOT NULL` with default `'step'::text`.

### T3.5 `pg_criteria.rs` and `tests/cache.rs`

- `every_cascade_table_loses_exactly_what_the_report_names` (`:3195`). After the memory seed
  (`:3317`), seed one plan by SQL on the fixture `step` the case already holds (`:3201-3209`):
  `INSERT INTO run_plan (id, run_id, revision, planner_step_id, scope, base_position, body, digest)
  SELECT gen_random_uuid(), s.run_id, 0, s.id, 'parts', 0, '{}'::jsonb, 'sha256:measured' FROM
  run_step s WHERE s.id = $1`. Extend the final non-zero assertion (`:3439-3450`) with `run_plans`:
  `== (1, 1, 1, 1, 1, 1, 2, 1)`, "the nine seeded tables". The doc (`:3175-3194`) gains "MOD-139's
  `run_plan` is seeded by SQL like the first five".
- `tests/cache.rs`:
  - `:2297-2301`: `MIRRORED_TABLES.len()` is `22`, "MOD-139 adds run_plan".
  - `the_0010_run_plan_reaches_the_mirror` (beside T1's role test). First assert `RUN_1`'s
    `lease_owner IS NULL` by SQL, so a fixture change fails loudly. Create a planner step on `RUN_1`
    (`create_step`, role `Planner`, position 60), `insert_run_plan(StepFence::Unleased, …)` revision 0,
    then `run_pass`. `cache.run_plans(RUN_1) == db.store.run_plans(RUN_1) == [plan]`. Then
    `UPDATE run_plan SET status = 'admitted', base_refs = '{"r": "abc"}'` (raw SQL; the trigger moves
    `updated_at`) and run a second `run_pass`: the mirror reads `Admitted` with those refs.
    `cache.run_plans(RunId::new())` is empty.

### T3.6 `.sqlx` regeneration

New: `run_plans`, the planner-step lock, the run lock (it may reuse a byte-identical entry), the insert
and `refresh_run_plan`. Replaced: `project_reach`. Recipe (memory: the compose `htui` DB is empty, so
prepare needs a migrated scratch DB):

```bash
cd /home/mluigi/projects/htui/crates/htui-store
BASE="${HTUI_TEST_DATABASE_URL%/*}"
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE IF EXISTS mod139_prepare' -c 'CREATE DATABASE mod139_prepare'
DATABASE_URL="$BASE/mod139_prepare" sqlx migrate run --source migrations
DATABASE_URL="$BASE/mod139_prepare" cargo sqlx prepare -- --all-targets --all-features
DATABASE_URL="$BASE/mod139_prepare" cargo sqlx prepare --check -- --all-targets --all-features
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE mod139_prepare'
```

The same recipe serves T1 (four entries replaced). Keep `DATABASE_URL` exported to the migrated DB
while writing queries, because the macros check online.

### T3.7 Commits and validate

1. `feat(store): MOD-139 M1 T3, PgStore run_plans and insert_run_plan, run_plan in the delete reach`
   (with `.sqlx`).
2. `feat(store): MOD-139 M1 T3, run_plan mirrored on its own cursor`.
3. `test(store): MOD-139 M1 T3, 0028 CHECK probes, comment pins, measured cascade and mirror cases`.

```bash
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
cargo test -p htui-store --all-features --test migrations --test cache --test pg_criteria -- --test-threads=1
cargo clippy -p htui-store --all-targets --all-features -- -D warnings
```

---

## T4 — Integration gate (serial, after T2 and T3)

Merge T2 and T3. Their file sets are disjoint (§5), so the merge has no conflicts. Re-run
`cargo sqlx prepare --check`. Then run the plan's Validation block:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m1-gate.log
grep -c SIGABRT /tmp/mod139-m1-gate.log   # expect 0
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Confirm in the gate:
- `pg_store_conformance` prints the nine MOD-139 cases, and `EXPECTED_CASES == 257 == CASES.len()`.
- `every_cross_referenced_test_name_exists` passes (P-9).
- Linear and compete suites are green with no snapshot drift. M1 adds no UI, and `RunStep` JSON
  reaches no `.snap`.
- If MOD-97 merged `0028` first, renumber at collect: rename the file to `0029_…`, and move every 28
  pin to 29. That covers §T1.11's nine sites, the header's schema_version, the cache file's
  `schema_version` prose and `MOD139_*` docs that name the file. The cache file becomes `0011` if MOD-97
  took `0010`.
- Sandbox: the live-Qdrant suites time out here and are owed on the host. They are not a regression of
  this milestone.

---

## 5. Per-task file sets (C12 re-checked with this blueprint's additions)

| Task | Files |
|---|---|
| T1 (serial) | both migrations; `htui-core/src/model/{ids,run,mod}.rs`; `htui-core/src/store/{traits,mod,worker,graph_rules,mem,conformance}.rs`; `htui-core/src/fixtures.rs`; `htui-store/src/{pg/read,pg/write,cache/read,cache/refresh,writer,backend,worker}.rs`; `htui-agent/src/conformance.rs`, `htui-agent/tests/{recorder,relay}.rs`; `htui-orch/src/{graph,engine,gate,conformance,recover}.rs`, `htui-orch/tests/review_loop.rs`; `htui-mcp/tests/tools_backlog.rs`; `htui/src/{hierarchy,queue_overview,run_worker}.rs`, `htui/tests/{kinds,backlog,waiting}.rs`; `htui-store/tests/{migrations,connect,cache,pg_criteria}.rs`; `.sqlx` (four entries) |
| T2 | `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs` |
| T3 | `htui-store/src/pg/read.rs`, `pg/write.rs`, `cache/read.rs`, `cache/refresh.rs`, `cache/mod.rs`, `.sqlx/*`, `htui-store/tests/migrations.rs`, `tests/pg_criteria.rs`, `tests/cache.rs` |

**T2 ∩ T3 = ∅.** T1 touches files of both lanes (`mem.rs`, `conformance.rs`, `pg/*`, `cache/*`, the
three htui-store tests), but T1 is serial and merged before either lane starts. Nothing forces an
overlap. A signature change discovered in a lane goes back through the coordinator (plan Risks).
T2 adds no query, so it never touches `.sqlx`.

## 6. Risks and gotchas for implementers

- **Stack.** Every new `run_case` arm is `Box::pin`ned. If PgStore's `insert_run_plan` future shows
  up in a SIGABRT, box its rare branch (`fenced_or_missing`'s note).
- **`unused_imports` under the featureless clippy** after §T1.7. Remove `TemplateRole` and
  `reserved_phase_name` where the two sites were their only use.
- **The P-9 self-scan** in `conformance.rs`: no backticked constraint or index names of four or more
  underscores.
- **`query_as!` column order.** Keep every select's column order equal to the struct's field order
  (the three new columns after `branch`, before `updated_at`, as `f6da9680` placed `branch`).
  `pg/rows.rs:118-121` records that the house treats `query_as!` binding as positional.
- **Mirror arms.** Add `RUN_PLAN` to the list **and** to the match. The `other => unreachable!` arm
  panics at runtime, not at compile time.
- **`role` in SQLite** is `NOT NULL DEFAULT 'step'`. Applied over a populated 0009 file, it fills
  existing rows (`cache_0010_applies_over_a_populated_0009_mirror` pins it), and the schema bump then
  rebuilds.
- **Not re-litigated:** F-1 (stage key ↔ position binding) and F-2 (planner role check) are open for
  the coordinator, with no M1 change.
