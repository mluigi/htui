# Plan: MOD-149 — Workspace requirements and nested keys

**Source**: `HANDOFF.md:512` (from ANA-35 `docs/ANA-35.md` §7.7, §7.9 P6, §7.10 row 6; `R-ENT-14`, `R-ENT-15`)
**Complexity**: Medium-large (one migration, a store-API widening, both stores, the mirror)
**Routing**: plan path (`/handoff-run`, 2026-11-14); confirm waived. Builds on MOD-147's scope row
(`docs/decisions/mod/mod-147.md`).

## Summary

Requirements are keyed `R-<AREA>-<N>` by a generated column today (`0006_requirements.sql:76`), and
there is no refinement. After MOD-149:

- **Workspace requirements** (owned by a workspace's scope row) mint `W-<AREA>-<N>`; project ones keep
  `R-<AREA>-<N>`. The key is stored at mint, guarded by `UNIQUE (project_id, key)`.
- **A requirement may refine one other** (`refines_id`). Within one owner it nests: the child keeps the
  parent's area, stores `number` NULL and a `child_number` from a per-parent counter, and is keyed
  parent + `.n` (`R-TUI-2.1`, `W-ORCH-3.1`). A project requirement refining a workspace one of one of its
  workspaces is not nested: it takes its own top-level key. Anything else is refused.
- **Coverage derives from splits** (`R-ENT-15`): a requirement with active refiners is covered when all
  are, in progress when any is, open otherwise; a leaf derives from its own citations.
- The mirror carries the new columns; no new mirrored table.

Out of scope: the Workspaces-tab requirement UI, split rows, `↑ from` rows, the graph and the coverage
display (MOD-118, `HANDOFF.md:984`); `verifies` and the per-requirement coverage view (MOD-114 (2),
`HANDOFF.md:930`); agent-proposed splits as drafts (MOD-118's "agent proposals and edits as drafts a
person accepts"); new MCP tools (none exist for requirements today); W- key colouring in
`theme.rs:268` (MOD-118).

## Decisions

- **D1 — Migration `0034_requirement_keys.sql`** (final text; header comment in `0026`'s style):
  ```sql
  ALTER TABLE requirement ALTER COLUMN key DROP EXPRESSION;     -- values kept, stored from now on
  ALTER TABLE requirement ALTER COLUMN key SET NOT NULL;        -- not in the ANA sketch: see below
  UPDATE requirement r SET key = 'W' || substr(r.key, 2)        -- OQ2: scope-row rows become W-
    FROM project p WHERE p.id = r.project_id AND p.owner_workspace_id IS NOT NULL;
  ALTER TABLE requirement ADD COLUMN refines_id UUID
      CONSTRAINT fk_requirement_refines REFERENCES requirement(id) ON DELETE SET NULL;   -- OQ1
  ALTER TABLE requirement ADD COLUMN child_number INTEGER
      CONSTRAINT chk_requirement_child_number CHECK (child_number >= 1);
  ALTER TABLE requirement ALTER COLUMN number DROP NOT NULL;
  ALTER TABLE requirement ADD CONSTRAINT chk_requirement_number_or_child
      CHECK (num_nonnulls(number, child_number) = 1);
  ALTER TABLE requirement ADD CONSTRAINT chk_requirement_child_refines
      CHECK (child_number IS NULL OR refines_id IS NOT NULL);
  ALTER TABLE requirement ADD CONSTRAINT chk_requirement_key_shape
      CHECK (key ~ '^[RW]-[A-Z][A-Z0-9]*-[1-9][0-9]*(\.[1-9][0-9]*)*$');
  ALTER TABLE requirement ADD CONSTRAINT uq_requirement_key UNIQUE (project_id, key);
  ALTER TABLE requirement ADD CONSTRAINT uq_requirement_child UNIQUE (refines_id, child_number);
  -- UNIQUE (project_id, area_code, number) stays: NULL numbers are distinct, children pass it
  CREATE TABLE requirement_child_counter (
      parent_id  UUID PRIMARY KEY REFERENCES requirement(id) ON DELETE CASCADE,
      last_value INTEGER NOT NULL CHECK (last_value >= 0)
  );
  ```
  - **Deviation 1:** `key SET NOT NULL`. The generated column was declared without `NOT NULL`
    (`0006:76`), so after `DROP EXPRESSION` an insert that omits `key` (the demo loader today,
    `pg/demo.rs:604`) would store NULL, and the CHECK passes NULL.
  - **Deviation 2:** `ON DELETE SET NULL` on `refines_id` (OQ1). The ANA sketch has no action.
  - No `COMMENT ON`. One new table (`TABLES` 54 → 55). The `uq_requirement_child` index leads with
    `refines_id`, so it serves the FK and the refinements read.
  - The existing area-code CHECK `^[A-Z][A-Z0-9]{1,15}$` (`0006:49`) and `number >= 1` (`0006:75`)
    make every existing key match `chk_requirement_key_shape`. This settles ANA-35 §11's open point.
- **D2 — Key format.**
  - **Top level:** `{L}-{area_code}-{number}`. `L` is `W` when the owning `project` row has
    `owner_workspace_id` set (a scope row), else `R`.
  - **Nested:** `{parent.key}.{child_number}`, at any depth (`R-ENT-1.1.1`).
  - **Model helpers** in `model/requirement.rs`:
    - `Requirement::top_level_key(scope_row: bool, area_code: &str, number: i32) -> String`
    - `Requirement::child_key(parent_key: &str, child_number: i32) -> String`
    - `Requirement::key_is_valid(key: &str) -> bool`, a hand parser mirroring
      `chk_requirement_key_shape` (no regex dependency)
    - `Requirement::key_path(&self) -> Vec<i32>`, the digits after the last `-` split on `.`
    - `Requirement::list_cmp(&self, &Self) -> Ordering`
- **D3 — Model shape.**
  - **`Requirement`** (`model/requirement.rs:107`): `number: Option<i32>` (`None` on a nested child),
    plus `refines_id: Option<RequirementId>` and `child_number: Option<i32>`, placed after `key`.
  - **`NewRequirement`** (`:149`) gains `refines_id: Option<RequirementId>`. The trait signature
    `mint_requirement(area, new)` is unchanged, so no wrapper changes.
  - **Nesting needs the parent's area.** A nested child passes the parent's area as `area`. Any other
    area is refused (D4).
- **D4 — Mint rules and refusal order** (both stores, identical). Every refusal comes before any
  counter moves.
  1. Unknown area → `NotFound { entity: "requirement_area" }` (as today).
  2. With `refines_id = Some(p)`:
     - Unknown `p` → `NotFound { entity: "requirement" }`.
     - **Reach.** Let `C` be the area's project and `O` be `p`'s project.
       - `O == C`: nested.
       - `O != C`, `C` not a scope row, and `O` the scope row of a workspace with a
         `workspace_project (O.owner_workspace_id, C)` row: cross-owner.
       - Anything else → `Constraint(requirement_refines_outside_reach(&p.key))`. That covers a
         scope row refining another owner, a project refining a sibling project, and a project refining
         a workspace it is not in.
     - Nested and `area != p.area_id` → `Constraint(requirement_child_area(&p.key, &p.area_code))`.
     - `p` withdrawn → `Constraint(requirement_withdrawn(&p.key))` (existing fn, `traits.rs:3997`; see
       OQ4).
  3. Then the existing checks: duplicate id, `created_by`, `box_id`. Mem checks them before writing;
     Pg gets them from the INSERT, mapped as today.
  - **Nested writes:** `child_number` comes from the `requirement_child_counter` upsert on `p`;
    `number = NULL`; `area_id`/`area_code` are the parent's; `key = child_key(p.key, n)`.
  - **Top-level writes** (with or without a cross-owner `refines_id`): `number` comes from
    `requirement_key_counter` as today, and `key = top_level_key(...)`.
  - **New sentence fns** in `traits.rs`, beside `requirement_withdrawn`, each `#[must_use] pub fn … ->
    String`:
    - `requirement_child_area(parent_key, parent_area)` →
      `"a requirement nested under {parent_key} is minted in its area {parent_area}"`
    - `requirement_refines_outside_reach(parent_key)` →
      `"{parent_key} cannot be refined here: a project requirement refines its own project's or one of
      its workspaces', a workspace requirement only its own workspace's"`
- **D5 — Counter semantics.**
  - `requirement_child_counter` is one row per parent, and the first child is `.1`.
  - **Pg:** `INSERT … VALUES ($1, 1) ON CONFLICT (parent_id) DO UPDATE SET last_value =
    requirement_child_counter.last_value + 1 RETURNING last_value`.
  - **Mem:** `State.requirement_child_counter: HashMap<RequirementId, i32>`, same shape as
    `requirement_key_counter` (`mem.rs:384`). It is seeded empty at `mem.rs:524`, and dropped with its
    parent in `remove_project`.
  - Like the area counter, it is not mirrored and numbers are never reused.
  - **Pg mint** becomes one transaction, replacing the single CTE (`pg/write.rs:9802`):
    1. Area row joined to `project` for `owner_workspace_id IS NOT NULL`.
    2. With `refines_id`: the parent row, then the reach probe:
       `SELECT EXISTS (SELECT 1 FROM project t JOIN workspace_project wp ON wp.workspace_id =
       t.owner_workspace_id WHERE t.id = $parent_project AND wp.project_id = $child_project)`.
    3. The counter upsert.
    4. INSERT the requirement with an explicit `key`, `refines_id` and `child_number`, `RETURNING` every
       column.
    5. INSERT revision 1.
    6. Commit.
    A failure in steps 4-5 rolls the counter back, so no number is burned.
- **D6 — List order** (all three backends).
  - **Order:** `(area_code bytes, key_path, key bytes)`. `requirement_refinements` adds `id` last.
  - **Pg:** `ORDER BY r.area_code COLLATE "C", string_to_array(split_part(r.key, '-', 3),
    '.')::int[], r.key COLLATE "C"`. `int[]` order equals `Vec<i32>` order.
  - **Mem and cache:** sort in Rust with `Requirement::list_cmp`. SQLite has no arrays.
  - **Where it applies:**
    - `requirements` keeps that order.
    - `item_requirements` uses `(list order, kind)` in place of `(area_code, number, kind)`.
    - Mem's `item_citations` (`mem.rs:8772`) and the trait docs (`traits.rs:255, 283`) follow.
- **D7 — New read `ReadStore::requirement_refinements(&self, id: RequirementId) ->
  Result<Vec<Requirement>>`**:
  - **What it returns:** every requirement with `refines_id = id`, from any project, in D6 order;
    empty for an unknown id.
  - **On the mirror:** only rows of mirrored projects.
  - **Impls:** 7 (`scope_project`'s footprint, `traits.rs:370`): Mem, Pg, cache, `Backend`,
    `Writer`, `UsageSpy` (`htui-agent/src/conformance.rs`) and `SpyStore`
    (`htui-agent/tests/it/recorder.rs`).
- **D8 — Coverage.**
  - **Type** in `model/requirement.rs`: `pub enum CoverageState { Open, InProgress, Covered }`
    (`Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`, serde `snake_case`).
  - **`CoverageState::of_citations(rows: &[CoverageRow]) -> Self`** (leaf rule, OQ3):
    - `Covered` if some row has `kind == Addresses`, `!suspect`, `item.status == Closed` and
      `resolution` in {`Done`, `Concluded`}.
    - Otherwise `InProgress` if some row has `kind == Addresses`.
    - Otherwise `Open`.
  - **`CoverageState::of_splits(children: impl IntoIterator<Item = Self>) -> Option<Self>`**:
    - `None` when empty.
    - `Covered` when all are covered.
    - `InProgress` when any is `Covered` or `InProgress`.
    - `Open` otherwise.
  - **Store helper** `pub async fn requirement_coverage_state<S: ReadStore>(store: &S, id:
    RequirementId) -> Result<CoverageState>` in `traits.rs`:
    - A requirement's active refiners (`state == Active`) decide when there is at least one.
      Otherwise its own `requirement_coverage` rows decide.
    - It is applied recursively. The walk is iterative (no async recursion); refinement is acyclic by
      construction, since `refines_id` is set only at mint and only to an existing row.
  - No UI reads it in this item (MOD-118 and MOD-114 do).
- **D9 — Deleting.**
  - **Mem `remove_project`** (`mem.rs:6050-6075` pattern) emulates the FK:
    - Surviving requirements whose `refines_id` is gone get `refines_id = None, updated_at = now` (the
      SET NULL's UPDATE fires `trg_requirement_updated_at`).
    - `requirement_child_counter` entries of gone requirements are dropped.
  - **Effect:** a workspace delete leaves a member's cross-owner refinement as a plain top-level
    requirement with its key.
- **D10 — Mirror: `cache_migrations/0011_requirement_keys.sql`.**
  - `DROP TABLE requirement`, then recreate it with `number INTEGER` (nullable), `key TEXT NOT NULL`,
    `refines_id TEXT`, `child_number INTEGER`; `idx_cache_requirement_order` recreated, plus
    `idx_cache_requirement_refines ON requirement(refines_id)`. SQLite cannot drop NOT NULL in place;
    this is `0009_workspace_scope.sql`'s pattern.
  - The dropped rows return on the full rebuild that `schema_version` 34 forces
    (`PgStore::schema_version`, `pg/mod.rs:758`).
  - `MIRRORED_TABLES` stays 22.
- **D11 — Readers.**
  - **Search points** (`vector_sync.rs:314`) read `key` only: unchanged.
  - **TUI** keeps minting top-level only: `refines_id: None` at `htui/src/requirements.rs:567`.
  - **Bench** sorts with `list_cmp` (`htui-bench/src/lexical.rs:280`).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Mint (Mem) | `crates/htui-core/src/store/mem.rs:8918` `State::mint_requirement` | refusals first, counter `entry().or_insert(0)`, revision 1 pushed |
| Mint (Pg) | `crates/htui-store/src/pg/write.rs:9802` | counter upsert `ON CONFLICT … RETURNING`; NotFound when the area is absent |
| Reach (Mem/Pg) | `mem.rs:3941` `link_reaches`; `pg/write.rs:322` `link_reaches` | workspace membership + `scope_row_of` / `owner_workspace_id` |
| Transaction + refusals (Pg) | `pg/write.rs:1441` `revise_requirement` | `pool.begin()`, read, refuse, write, commit |
| SET NULL emulation (Mem) | `mem.rs:6050-6075` `remove_project` | `retain` then `for row in &mut …` clearing the FK with `updated_at = now` |
| New ReadStore method | `traits.rs:370` `scope_project` and its 7 impls (`mem.rs:10713`, `pg/read.rs:1191`, `cache/read.rs:1334`, `backend.rs:1217`, `writer.rs:350`, `htui-agent/src/conformance.rs:766`, `htui-agent/tests/it/recorder.rs:454`) | doc with `# Errors`; wrappers forward |
| Refusal sentences | `traits.rs:3997` `requirement_withdrawn`, `traits.rs:4105` `link_outside_reach` | `#[must_use] pub fn … -> String`, one doc line naming the rule |
| Conformance case | `crates/htui-core/src/store/conformance.rs:16605` `requirement_mint_is_per_area_and_never_reused`; helpers `new_requirement` `:16496`, `new_area` | `CASE` const, registered in `CASES` (`:113`, entry at `:174`) and `run_case` arm (`:598`) |
| Migration file | `crates/htui-store/migrations/0026_workspace_scope.sql` | header naming MOD/ANA/plan Ds, forward-only, mirror companion named |
| Migration test | `crates/htui-store/tests/it/migrations.rs:6540` `migration_0026_backfills_document_project_and_keeps_item` | `bare_db`, `run_to(N-1)`, plant raw SQL, `run`, assert, `drop_db` |
| Cache migration | `crates/htui-store/cache_migrations/0009_workspace_scope.sql` | DROP + CREATE with comment on the rebuild |
| Cache columns | `cache/read.rs:382` `REQUIREMENT_SELECT`, `:386` `requirement_of`; `cache/refresh.rs:1560` `REQUIREMENT_COLUMNS`, `:1579` `refresh_requirement` | one column list shared by every read |
| Cache test | `crates/htui-store/tests/it/cache.rs:1741` `the_0010_step_role_reaches_the_mirror` | write in Pg, `run_pass`, compare cache with Pg |
| Pg race test | `crates/htui-store/tests/it/pg_criteria.rs:235` | concurrent mints on one counter |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/requirement.rs` | UPDATE: fields, helpers, `CoverageState`, unit tests | T1 |
| `crates/htui-core/src/model/mod.rs` | UPDATE: re-export `CoverageState` (`:194`) | T1 |
| `crates/htui-core/src/store/traits.rs` | UPDATE: `requirement_refinements`, two sentence fns, `requirement_coverage_state`, docs | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE: mint rules, child counter, order, refinements read, delete | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE: 7 new cases + `CASES` + `run_case`; `new_requirement` gains `refines_id: None` | T1 |
| `crates/htui-core/src/fixtures.rs` | UPDATE: `Requirement` literal (`:2184`), counter test (`:2498`) | T1 |
| `crates/htui-core/tests/it/mem_store.rs` | UPDATE: `CASES` pin 312 → 319 (`:44`) | T1 |
| `crates/htui-agent/src/conformance.rs` | UPDATE: `UsageSpy::requirement_refinements` | T1 |
| `crates/htui-agent/tests/it/recorder.rs` | UPDATE: `SpyStore::requirement_refinements` | T1 |
| `crates/htui-store/migrations/0034_requirement_keys.sql` | CREATE | T2 |
| `crates/htui-store/cache_migrations/0011_requirement_keys.sql` | CREATE | T2 |
| `crates/htui-store/src/pg/write.rs` | UPDATE: mint (D5), `revise_requirement` columns | T2 |
| `crates/htui-store/src/pg/read.rs` | UPDATE: three reads' columns + order; `requirement_refinements` | T2 |
| `crates/htui-store/src/pg/demo.rs` | UPDATE: insert `key`, `refines_id`, `child_number` | T2 |
| `crates/htui-store/src/cache/read.rs` | UPDATE: columns, Rust sort, `requirement_refinements` | T2 |
| `crates/htui-store/src/cache/refresh.rs` | UPDATE: columns + binds | T2 |
| `crates/htui-store/src/backend.rs`, `src/writer.rs` | UPDATE: forward `requirement_refinements` | T2 |
| `crates/htui-store/.sqlx/*.json` | REGENERATE | T2 |
| `crates/htui-store/tests/it/migrations.rs` | UPDATE: `TABLES`, pins, two new tests | T2 |
| `crates/htui-store/tests/it/connect.rs` | UPDATE: `Pending(33)` pins (`:140, :156, :241`) | T2 |
| `crates/htui-store/tests/it/pg_conformance.rs` | UPDATE: `EXPECTED_CASES` 312 → 319 (`:68`) | T2 |
| `crates/htui-store/tests/it/pg_criteria.rs` | UPDATE: `refines_id: None` (`:235`); child-counter race | T2 |
| `crates/htui-store/tests/it/cache.rs` | UPDATE: new mirror test | T2 |
| `crates/htui-bench/src/lexical.rs`, `src/corpus.rs` | UPDATE: literals, sort, test reads | T3 |
| `crates/htui/src/requirements.rs` | UPDATE: `refines_id: None` (`:567`, `:1650`) | T3 |
| `crates/htui/src/ui/tabs/requirements/mod.rs` | UPDATE: test `row.number = 3` → `Some(3)` (`:2226`) | T3 |
| `crates/htui/src/hierarchy.rs` | UPDATE | `DeleteReach.requirement_child_counters` in the delete confirmation (T3, fact-check amendment) |

## Tasks

The tasks run serially: T1 → T2 → T3. T1 changes the model and a `ReadStore` method, so `htui-store`
does not compile until T2 is done, and T3's crates depend on `htui-store`. T1 is the foundation and gets a
verifier. The file sets are disjoint (Files table), but the build order rules out parallel runs.

### Task 1: Model, MemStore and the conformance cases — `verify: foundation`

**Files owned:** `crates/htui-core/src/model/requirement.rs`, `crates/htui-core/src/model/mod.rs`,
`crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
`crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/fixtures.rs`,
`crates/htui-core/tests/it/mem_store.rs`, `crates/htui-agent/src/conformance.rs`,
`crates/htui-agent/tests/it/recorder.rs`.

**Context:** read Decisions D2-D9. Demo fixture:
- **Projects:** `htui` and `agy` are members of workspace Platform; `vulkan-tutorials` is a member of
  Graphics (`fixtures.rs:728`).
- **Scope rows:** `ids::PROJECT_SCOPE_PLATFORM` and `ids::PROJECT_SCOPE_GRAPHICS` (`fixtures.rs:206`).
- **Requirements:** `htui` holds `R-ENT-1`, `R-ENT-2` and `R-STO-1`, with counters ENT 2 and STO 1
  (`fixtures.rs:2121`).
- **Items:** `HTUI_FIX_1` is closed as `done`; `HTUI_FEAT_1` is open.
- **Citations:** the store's `cite` checks no project or status (`mem.rs:9125`).

**Types and signatures:**
- `Requirement { …, number: Option<i32>, key: String, refines_id: Option<RequirementId>,
  child_number: Option<i32>, … }`; `NewRequirement { …, refines_id: Option<RequirementId> }`.
- The `impl Requirement` helpers from D2; `is_nested(&self) -> bool`.
- `CoverageState` and its two fns (D8).
- `ReadStore::requirement_refinements(&self, id: RequirementId) -> Result<Vec<Requirement>>`, with a
  doc and `# Errors`.
- `pub fn requirement_child_area(parent_key: &str, parent_area: &str) -> String`;
  `pub fn requirement_refines_outside_reach(parent_key: &str) -> String`.
- `pub async fn requirement_coverage_state<S: ReadStore>(store: &S, id: RequirementId) ->
  Result<CoverageState>`.
- **Mem:**
  - `State.requirement_child_counter`.
  - `State::mint_requirement` follows D4's order: key by owner, nested branch.
  - `requirement_rows` and `item_citations` sort by D6.
  - `refinement_rows(id)`.
  - `remove_project` follows D9.
  - The two `ReadStore` forwards for the agent spies.
- **Delete reach (fact-check amendment):** `DeleteReach` (`traits.rs:4469`) gains
  `requirement_child_counters: u64` ("`requirement_child_counter` rows of the project's requirements,
  which cascade from `requirement`"), placed after `requirement_key_counters`, and joins the exhaustive
  `Add` destructure (`traits.rs:4516`, `:4549`). Mem counts it beside `requirement_key_counters`
  (`mem.rs:5813`). The conformance reach case (`conformance.rs:~17600`) asserts it: plant one nested child
  in the project before the delete-reach read, so the count is 1. `htui-store` and `htui` construct and
  destructure it in T2/T3.

**Tests first:**
- **Unit tests** in `model/requirement.rs` (`#[cfg(test)]`):
  - `keys_follow_the_owner_and_nest`: `top_level_key(false, "ENT", 3) == "R-ENT-3"`,
    `(true, "ORCH", 1) == "W-ORCH-1"`, `child_key("W-ORCH-1", 2) == "W-ORCH-1.2"`.
  - `key_is_valid_mirrors_the_check`: accepts `R-ENT-1`, `W-A1-10.2.3`; rejects `X-ENT-1`, `R-ent-1`,
    `R-ENT-0`, `R-ENT-1.0`, `R-ENT-1.`, `R-ENT-01`.
  - `list_cmp_orders_children_under_their_parent`: `R-ENT-1 < R-ENT-1.1 < R-ENT-1.2 < R-ENT-2 <
    R-ENT-10`; `R-ENT-1.10 > R-ENT-1.9`.
  - `leaf_coverage_reads_addresses`, one assertion per D8 branch:
    - no row → Open
    - an open item's `addresses` → InProgress
    - a closed `done` item → Covered
    - a closed `rejected` item only → Open
    - a suspect closed `done` → InProgress
    - `reserves` only → Open
  - `split_coverage_is_all_any_or_open`: empty → None; all Covered → Covered; Covered + Open →
    InProgress; Open + Open → Open.
- **Conformance cases** in `conformance.rs`, each in `CASES` and `run_case`, run on Mem here and on Pg
  in T2:
  1. `requirement_mint_in_a_scope_row_keys_w`: an area `ORCH` in `PROJECT_SCOPE_PLATFORM`; the mint is
     `W-ORCH-1` with `number Some(1)` and no `refines_id`/`child_number`, and reads back equal. A
     second mint is `W-ORCH-2`.
  2. `requirement_nested_child_keys_parent_dot_n`:
     - Children of `R-ENT-1` in `AREA_ENT` are `R-ENT-1.1`, then `R-ENT-1.2` (`number None`,
       `child_number Some(n)`, `area_code "ENT"`, `refines_id Some(REQ_ENT_1)`).
     - A grandchild of `.1` is `R-ENT-1.1.1`.
     - A next top-level ENT mint is `R-ENT-3`: nesting leaves the area counter alone.
     - A child of `R-ENT-1` asked in `AREA_STO` → `requirement_child_area("R-ENT-1", "ENT")`, and the
       next child is still `.3`: no number burned.
     - Nested in a scope row: a child of `W-ORCH-1` is `W-ORCH-1.1`.
  3. `requirement_cross_owner_refinement_keeps_its_own_key`: `W-ORCH-1` in `@platform`; an `htui`
     mint in `AREA_ENT` refining it is `R-ENT-3` with `number Some(3)`, `child_number None` and
     `refines_id Some(w)`. An `agy` one (create an area there) is `R-<code>-1`.
  4. `requirement_refinement_outside_reach_is_refused`, each refused with the sentence and nothing
     written (`requirements` unchanged, next mint numbers unmoved):
     - `vulkan-tutorials` refining `W-ORCH-1` → outside reach.
     - `htui` refining an `agy` requirement → outside reach.
     - `@platform` refining `R-ENT-1` → outside reach.
     - `@graphics` refining `W-ORCH-1` → outside reach.
     - An unknown parent → `NotFound { entity: "requirement" }`.
     - A withdrawn parent → `requirement_withdrawn(key)`. Withdraw with `withdraw_requirement`, deciding
       item `HTUI_ANA_2`; use an `htui` parent.
  5. `requirement_refinements_list_every_owner`:
     - `requirement_refinements(w)` lists the nested `W-ORCH-1.1` and `htui`'s `R-ENT-3` in D6 order.
     - An unknown id → empty.
     - `requirements(htui)` lists `R-ENT-1, R-ENT-1.1, R-ENT-1.2, R-ENT-2, R-ENT-3, R-STO-1`.
  6. `requirement_coverage_derives_from_splits`: `W-ORCH-1` is refined by `htui`'s `R-ENT-3` and
     `R-STO-2`.
     - Uncited: `requirement_coverage_state(w)` is Open.
     - `cite(HTUI_FEAT_1, R-ENT-3, Addresses)`: InProgress.
     - `cite(HTUI_FIX_1, …)` on both: Covered.
     - Fixture leaf `R-STO-1` (FEAT-1 open, FIX-1 done): Covered.
  7. `workspace_delete_detaches_refining_requirements`: the setup of case 3, then
     `delete_workspace(WORKSPACE_PLATFORM)`. `htui`'s `R-ENT-3` survives with `refines_id None`, its
     key and number, and a later `updated_at`.
- **Fix the existing literals:**
  - `new_requirement` (`conformance.rs:16496`), `:16673`, `mem.rs:15134`, `:15222`: add
    `refines_id: None`.
  - Assertions on `.number` (`conformance.rs:16605` case, `fixtures.rs:2498`): use `Some(n)`.
- **Pin:** `mem_store.rs:44` 312 → 319, with the text naming MOD-149's seven cases.

**Check:**
```bash
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features -E 'kind(lib) and test(/requirement|fixtures::/)'
cargo build -p htui-agent --all-features --tests
```
`htui-store` and downstream do not build until T2. Do not run the workspace build or the gate.

### Task 2: Postgres, migration 0034, the mirror and `.sqlx`

**Files owned:**
- `crates/htui-store/migrations/0034_requirement_keys.sql` (new)
- `crates/htui-store/cache_migrations/0011_requirement_keys.sql` (new)
- `crates/htui-store/src/pg/write.rs`, `src/pg/read.rs`, `src/pg/demo.rs`
- `crates/htui-store/src/cache/read.rs`, `src/cache/refresh.rs`
- `crates/htui-store/src/backend.rs`, `src/writer.rs`
- `crates/htui-store/.sqlx/`
- `crates/htui-store/tests/it/migrations.rs`, `connect.rs`, `pg_conformance.rs`, `pg_criteria.rs`,
  `cache.rs`

**Context:** T1 landed the model, the trait method and the seven conformance cases. Here PgStore and
CacheStore must pass them and match MemStore's order and refusals exactly (D4, D6).

**Work:**
- **Migration:** D1's text verbatim, under a header in `0026`'s style that names both deviations and
  the mirror companion.
- **Pg mint:** the transaction of D5. Refusals in D4's order, before the counter upsert. The key is
  computed in Rust with T1's helpers from the area/parent rows read in the transaction.
- **Pg reads** (`pg/read.rs:994, :1050, :1251`) and `revise_requirement` (`pg/write.rs:1441`, head and
  `RETURNING`):
  - Select `refines_id AS "refines_id: RequirementId"` and `child_number`.
  - `number` becomes nullable. The three `"number!"` overrides (`pg/read.rs:1258`,
    `pg/write.rs:1543`, `:9834`) become `"number?"`, not plain: they come out of CTEs, where sqlx
    cannot infer nullability. The plain `number` selects (`read.rs:1017`, `:1058`, `write.rs:1455`)
    infer `Option<i32>` after the migration. The `"key!"` overrides stay.
- **Delete reach:** the project delete-reach query (`pg/write.rs:~882`, `:927`) gains
  `(SELECT count(*) FROM requirement_child_counter WHERE parent_id IN (SELECT id FROM rq)) AS
  "requirement_child_counters!"` and the field. `pg_criteria.rs:3418, :3451` destructure it.
  - Order per D6.
  - New `requirement_refinements` (D6/D7 order).
- **Demo loader** (`pg/demo.rs:602`): insert `key`, `refines_id` and `child_number`. The fixture has
  no refinements, so row order is unaffected.
- **Mirror:**
  - `0011` per D10.
  - `REQUIREMENT_SELECT`/`requirement_of` gain the two columns, with `number` optional.
  - `requirements`/`item_requirements` sort in Rust (`list_cmp`), and their SQL `ORDER BY` goes.
  - `requirement_refinements`.
  - `REQUIREMENT_COLUMNS` + binds in `refresh_requirement` (`refresh.rs:1560, :1579`).
- **Forwarding:** `Backend` (Memory/Online/Offline arms, as `backend.rs:1217`) and `Writer` (as
  `writer.rs:350`).
- **`.sqlx`:** regenerate with the recipe in `docs/hr-sandbox.md:227-237`:
  ```bash
  psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"   # once; drop/recreate if stale
  cd crates/htui-store && export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations
  cargo sqlx prepare -- --all-targets --all-features
  cargo sqlx prepare --check
  ```

**Tests first:**
- **`migrations.rs`:**
  - `migration_0034_stores_keys_and_letters_scope_rows`: `bare_db`, `run_to(33)`. Plant a user, a
    project (`plant_project_kind`), an area and `R-ENT-1` (raw INSERT without `key`), a workspace and
    its scope row (as `try_scope_row`, `:6476`) with an area and a requirement. Then `run_to(34)`.
    Assert:
    - the project row keeps `R-ENT-1`
    - the scope-row row reads `W-…`
    - `information_schema.columns` shows `key` `is_generated = 'NEVER'` and `is_nullable = 'NO'`,
      `number` `YES`, `refines_id`/`child_number` present
    - `requirement_child_counter` exists
  - `migration_0034_checks_tie_the_key_shape`: `fresh_db`. Raw inserts refused with these
    constraints:
    - `child_number` without `refines_id` → `23514` `chk_requirement_child_refines`
    - both `number` and `child_number` → `chk_requirement_number_or_child`
    - key `X-ENT-1` → `chk_requirement_key_shape`
    - a second `(project, key)` → `23505` `uq_requirement_key`
    - a second `(refines_id, child_number)` → `uq_requirement_child`
    - an insert without `key` → `23502`

    Then `DELETE FROM project` for a project holding a nested child succeeds (the SET NULL action
    meets no surviving child).
  - **Pins:**
    - `TABLES` gets `requirement_child_counter` under a `// 0034_requirement_keys.sql (MOD-149)`
      line; 54 → 55 (`:25, :35, :158, :174` texts).
    - The applied list adds `34` (`:125`) and its text.
    - `Pending(33)`/`MigrationsPending(33)` → 34 and "thirty-four … MOD-149's
      0034_requirement_keys.sql" at `:4256, :4357, :4366, :4389, :4542`.
- **`connect.rs:140, :156, :241`:** same pin moves.
- **`pg_conformance.rs:68`:** `EXPECTED_CASES` 312 → 319, both texts.
- **`pg_criteria.rs`:** add `refines_id: None` at `:235`; the `.number` collection at `:249` reads the
  now-optional field (collect `Option<i32>` or unwrap with a message). New `concurrent_children_take_distinct_numbers`:
  two `mint_requirement` calls with `refines_id = Some(REQ_ENT_1)` at once take `.1` and `.2`; neither
  gets `23505`.
- **`cache.rs`:** `the_0011_requirement_refinements_reach_the_mirror`. Mint `W-ORCH-1` in
  `PROJECT_SCOPE_PLATFORM`, a nested `W-ORCH-1.1` and `htui`'s cross-owner `R-ENT-3` in Pg. Then
  `run_pass` over `all_projects()` plus `PROJECT_SCOPE_PLATFORM`; `all_projects` (`:33`) lists no
  scope row. Assert on the cache:
  - `requirements(htui)` and `requirements(scope)` equal Pg's
  - `requirement_refinements(w)` equals Pg's
  - the nested row has `number None`

**Check:**
```bash
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(pg_conformance|migrations|connect|pg_criteria|cache)::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'kind(lib)'
cd crates/htui-store && cargo sqlx prepare --check
```

### Task 3: Downstream readers and the workspace build

**Files owned:** `crates/htui-bench/src/lexical.rs`, `crates/htui-bench/src/corpus.rs`,
`crates/htui/src/requirements.rs`, `crates/htui/src/ui/tabs/requirements/mod.rs`,
`crates/htui/src/hierarchy.rs`.

**Work:**
- **htui-bench:**
  - `lexical.rs:280` sorts requirements with `Requirement::list_cmp`.
  - The literals at `lexical.rs:356` and `corpus.rs:509` set `number: Some(..)`, `refines_id: None`,
    `child_number: None`.
  - Test reads at `corpus.rs:1113, :1455` compare `Some(n)`.
- **htui:**
  - `NewRequirement` literals at `requirements.rs:567, :1650` take `refines_id: None`.
  - The test at `ui/tabs/requirements/mod.rs:2226` sets `row.number = Some(3)`.
  - No behaviour change and no new UI (MOD-118 owns it).
  - `hierarchy.rs:863, :897` destructure `DeleteReach.requirement_child_counters` (T1) and list it in
    the delete confirmation beside the requirement key counters, in that block's wording;
    the test literal at `:1110` gains the field. If a delete-confirmation snapshot moves, accept it only
    when the sole change is the new line.
- **Other compile errors:** fix any further break `cargo build --workspace --all-features --tests`
  names, minimally, and list it in the report.

**Tests first:** none new. The existing `requirements` tests are the pin: `the_detail_of_r_ent_1_…`
(`requirements.rs:1472`) and `mint_answers_requirement_written_with_the_next_key` (`:1236`) must stay
green unchanged.

**Check:**
```bash
cargo build --workspace --all-features --tests
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'test(/requirements/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-bench --all-features -E 'test(/lexical::|corpus::/)'
cargo clippy --workspace -- -D warnings
cargo insta test -p htui --all-features   # expected: no snapshot moves (fixture unchanged)
```

## Validation

```bash
bash scripts/gate.sh                            # main thread, once
cargo clippy --workspace -- -D warnings         # featureless clippy
(cd crates/htui-store && cargo sqlx prepare --check)
cargo insta test --workspace --all-features     # full insta run; no snapshot should move
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Live Qdrant suites are owed on the host (`docs/hr-sandbox.md`); requirement points are unchanged
(D11).

## Verified claims

Checked 2026-11-14 on `hr/MOD-149` HEAD `e99acd97`. SQL probes ran on a scratch database (`mod149_probe`,
PG 18.7, migrations 0001-0033 each applied with `psql -1`, 54 base tables) with D1's DDL applied
verbatim; the database was dropped afterwards.

| Claim | Verdict | Evidence |
|---|---|---|
| `requirement.key` is `GENERATED … STORED` without `NOT NULL` | verified | `0006_requirements.sql:76` `key TEXT GENERATED ALWAYS AS ('R-' \|\| area_code \|\| '-' \|\| number::text) STORED`, no `NOT NULL` |
| `UNIQUE (project_id, area_code, number)` exists; `number NOT NULL CHECK (number >= 1)` | verified | `0006:75` `number INTEGER NOT NULL CHECK (number >= 1)`; `0006:85` `UNIQUE (project_id, area_code, number)`; probe: `requirement_project_id_area_code_number_key` still present after D1 |
| Area code CHECK is `^[A-Z][A-Z0-9]{1,15}$`, so existing keys match D1's shape CHECK | verified | `0006:49`. Probe (d): the shape regex accepts `R-ENT-1`, `R-NF-1`, `R-STO-12`, `W-A1-10.2.3` and every `"R-…` literal in `crates/**/*.rs` (ENT, STO, ORCH, NOPE, X, TUI, OURS, LATER, THEIRS, NF, ID, HIS); rejects `R-TUI-0`, `R-TUI-2.0`, `R-TUI-01`, `R-TUI-1.`, `X-ENT-1`, `R-ent-1`. `ADD CONSTRAINT chk_requirement_key_shape` validated the planted rows |
| No later migration touches `requirement` | verified | `git grep -n -E '\brequirement\b' migrations/` outside 0006: no hit; case-insensitive `requirement` hits only comments `0012:25`, `0022:3, :47, :97` |
| Next free migration is `0034`, cache `0011` | verified | `git ls-files` ends at `migrations/0033_memory_capture.sql`, `cache_migrations/0010_divided_fanout.sql` (no MOD-150 branch in this clone to check the collision risk) |
| Mirror `requirement.number` is `NOT NULL`; `key` a plain column | verified | `cache_migrations/0004_requirements.sql:30` `number INTEGER NOT NULL, key TEXT NOT NULL`; table `:28-33`, order index `:34` |
| `MIRRORED_TABLES` has 22 entries incl. `requirement`; the counters are not mirrored | verified | `cache/mod.rs:47` `[&str; 22]`, `"requirement"` at `:68`; `:43` "`requirement_revision` and `requirement_key_counter` are not mirrored"; `it__connection__confirm.snap` says "the 22 mirrored tables" |
| `schema_version` is the highest embedded migration | verified | `pg/mod.rs:758-759` `MIGRATOR.iter().map(\|m\| m.version).max()`; `cache/mod.rs:146` rebuilds when it differs |
| `TABLES` pin is 54 | amended | 54 holds (probe DB: 54 base tables), but the literals sit at `migrations.rs:25` (doc), `:159` (`54,`) and `:175` (text); `:35` is the `const TABLES`, `:158`/`:174` the `TABLES.len()` lines |
| Migration-count pins sit at `migrations.rs:125, :4256, :4357, :4366, :4389, :4542` and `connect.rs:140, :156, :241` | amended | migrations.rs exact (`:125` applied list ends `33`; `:4256`, `:4357` `Pending(33)`; `:4366`, `:4389` `MigrationsPending(33)`; `:4542` text). connect.rs: `:140` and `:241` `Pending(33)`, but the third is `:155` `pending, 33,` with its text on `:156-157`. No other `thirty-three`/`Pending(33)` pin in `crates/` |
| `CASES` is 312 (not 308) on this branch | verified | `conformance.rs:113-449` has 312 entries (counted); `pg_conformance.rs:68` `EXPECTED_CASES: usize = 312`; `mem_store.rs:44` `312` |
| `READ_CASES` 15, untouched here | verified | `conformance.rs:1504` list has 15 entries; `mem_store.rs:104` `15` (`:103` is the `.len()` line) |
| `mint_requirement(area, new)` has 7 impls/forwards; adding a field to `NewRequirement` needs no wrapper change | amended | 5 trait impls plus the trait declaration: `traits.rs:2245` (decl), `mem.rs:11629` (Mem), `pg/write.rs:9802` (Pg), `writer.rs:1590`, `htui-agent/src/conformance.rs:1536` (`UsageSpy`), `htui-agent/tests/it/recorder.rs:1262` (`SpyStore`); the 7th site is `mem.rs:8918` `State::mint_requirement`, not a forward. No `Backend` impl (`backend.rs:6`). Every wrapper forwards `new` whole, so the conclusion holds |
| `NewRequirement` literals: `conformance.rs:16497, :16673`, `mem.rs:15134, :15222`, `pg_criteria.rs:235`, `htui/src/requirements.rs:567, :1650` | amended | The list is complete (`git grep -E '\bNewRequirement \{'`), but `conformance.rs:16673` is `NewRequirement { created_by: UserId::default(), ..new_requirement("By nobody.") }`: struct-update syntax, needs no `refines_id`. Six sites need the field |
| `Requirement` literals: `fixtures.rs:2184`, `mem.rs:8946`, `pg/read.rs:1286`, `cache/read.rs:387`, `htui-bench/src/lexical.rs:356`, `corpus.rs:509` | verified | Complete per `git grep -E '(^\|[^A-Za-z_])Requirement \{'` (other hits are `Subject::`/`RevealTarget::`/`Owner::` variants). `query_as!(Requirement, …)` builds implicit literals too: `pg/read.rs` `requirements` (`:994`) and `requirement` (`:1050`), `pg/write.rs:1449` (revise head), the revise result (`:1543`), the mint (`:9807`) |
| `ReadStore` has 7 impls | verified | `mem.rs:10566`, `pg/read.rs:89`, `cache/read.rs:427`, `backend.rs:1001`, `writer.rs:131`, `htui-agent/src/conformance.rs:645`, `recorder.rs:332`; no other `impl … ReadStore for` in `crates/`. `scope_project` at `traits.rs:370`, `mem.rs:10713`, `pg/read.rs:1191`, `cache/read.rs:1334`, `backend.rs:1217` (Memory/Online/Offline), `writer.rs:350` (Memory/Online), agent `:766`, recorder `:454`: all match |
| `htui-agent` depends on `htui-core` only (not `htui-store`) | verified | `crates/htui-agent/Cargo.toml:18` `htui-core` is its only `htui*` dependency in any section; `htui-core` has none |
| `htui-bench` depends on `htui-store` | verified | `crates/htui-bench/Cargo.toml:16` (`features = ["local-embed"]`), dev-dep `:34` |
| Store `cite` checks neither project nor item status | verified | Mem `mem.rs:9125-9161`: `require_item`, requirement NotFound, withdrawn + `addresses`/`reserves`, step exists. Pg `pg/write.rs:9943-9985`: item exists, requirement `FOR SHARE`, withdrawn rule. Neither reads project or status |
| Demo: htui+agy in Platform, vulkan in Graphics; scope rows `PROJECT_SCOPE_PLATFORM/GRAPHICS` | verified | `fixtures.rs:728-746` `workspace_projects()` (Platform: HTUI pos 0, AGY pos 1; Graphics: VULKAN); `:206`, `:208` scope-row ids; `:767-770` `SCOPE_SPECS` |
| Demo `HTUI_FIX_1` closed `done`; counters ENT 2, STO 1 | verified | `HTUI_FIX_1 = (ITEM, 5)` (`:265`); spec n 5 `status: Status::Closed` `:1157`; `:1280` `resolution: (status == Closed).then_some(Resolution::Done)`; `:2122` `HashMap::from([(AREA_ENT, 2), (AREA_STO, 1)])`. `HTUI_FEAT_1` is open; `:2271-2272` FEAT-1 and FIX-1 cite R-STO-1 `Addresses` at v1 (R-STO-1 is v1, so not suspect) |
| `create_requirement_area` accepts a scope row | verified | `mem.rs:8872-8915` checks code shape, `projects.contains_key`, duplicate id, duplicate code only; Pg `pg/write.rs:9759` plain INSERT. A scope row is a `project` row (`chk_project_scope_row`) |
| Today "coverage" is the list of live citations (`CoverageRow`); no derived state exists | verified | `traits.rs:297` `requirement_coverage -> Vec<CoverageRow>`; `model/requirement.rs:250` `CoverageRow { item: ItemSummary, kind, resolution, requirement_version, suspect }` (`ItemSummary.status`, `model/item.rs:240`); `git grep -E '\bCoverageState\b\|\bCovered\b' crates/`: no hit |
| TUI reads coverage only as rows in the Requirements detail | verified | Only caller outside the store: `htui/src/requirements.rs:375`; rendered at `ui/tabs/requirements/detail.rs:92-96` |
| Search points read `key` only | verified | `vector_sync.rs:314-330` `requirement_point` reads `id, area_code, priority, state, version, key, project_id, updated_at, body, rationale`; no `number` |
| No MCP requirement tools exist | verified | `crates/htui-mcp/src`: `requirement` only in `search.rs:59-128` (kind/owner enums) and `tools/search.rs:68` (description) |
| Demo loader omits `key` in its INSERT | verified | `pg/demo.rs:604-606` column list `(id, project_id, area_id, area_code, number, body, rationale, priority, state, version, created_by, created_at, updated_at)`; the only raw `INSERT INTO requirement` outside migrations and the mint |
| Pg reads use `"key!"`, and `item_requirements`/revise use `"number!"` | amended | `"key!"`: `pg/read.rs:1018, :1059, :1259`, `pg/write.rs:1457, :1544, :9835`. `"number!"`: `pg/read.rs:1258` (`item_requirements`, `query!`), `pg/write.rs:1543` (revise result) and `:9834` (the mint, not revise). `requirements` (`read.rs:1017`), `requirement` (`:1058`) and the revise head (`write.rs:1455`) select plain `number` |
| Mem `remove_project` emulates `ON DELETE SET NULL` with `updated_at = now` | verified | `mem.rs:6050-6061`: `item_requirements.retain`, then `proposed_by_step_id = None; updated_at = now`. Workspace delete reaches it: `mem.rs:5936` `self.remove_project(scope, now)` |
| **Compile/runtime probe:** sqlx infers `number` as `Option<i32>` in `query_as!(Requirement)` after `DROP NOT NULL` | amended | Not compiled (needs T2 code). sqlx's rule: a direct column reference takes `attnotnull`, so plain `number` at `read.rs:1017, :1058`, `write.rs:1455` becomes `Option<i32>` (the probe showed `number is_nullable = YES`). The `"number!"` overrides at `read.rs:1258`, `write.rs:1543`, `:9834` force `i32` and will not type-check against `Option<i32>` (`number: row.number`, `read.rs:1291`): T2 must make them `"number?"`, not plain (CTE/`RETURNING`-through-CTE outputs have no table origin, so plain inference is unreliable) |
| **Compile/runtime probe:** `key` after `DROP EXPRESSION` + `SET NOT NULL` is inferred non-null | verified | Probe (a): `information_schema.columns` gives `key is_generated = NEVER, is_nullable = NO`, values kept (`R-TUI-2`, `R-A1-10`; the scope row's `R-ORCH-3` became `W-ORCH-3`). A direct `key` reference infers `String`; the existing `"key!"` overrides stay valid and are still needed on CTE outputs |
| **Runtime probe:** `ON DELETE SET NULL` on `refines_id` does not trip `chk_requirement_child_refines` when a project delete cascades parent and nested child together | verified | Probe (b): `DELETE FROM project WHERE slug='htui'` holding `R-TUI-2`, nested `R-TUI-2.1`, `R-TUI-2.1.1` and cross-owner `R-TUI-5` → `DELETE 1`, all four gone. Lone parent `DELETE FROM requirement WHERE key='R-TUI-2'` → `ERROR: 23514 … violates check constraint "chk_requirement_child_refines"` (raised from the RI `UPDATE ONLY "public"."requirement" SET "refines_id" = NULL`). Cross-owner: deleting `@platform` (and separately `DELETE FROM workspace`) → `W-ORCH-3`, `W-ORCH-3.1` gone, `R-TUI-5` kept with `number 5`, `refines_id NULL`, `updated_at` moved from 2020-01-01 to now (the SET NULL fires `trg_requirement_updated_at`). Also: no-`key` insert → `23502`; `child_number` without `refines_id` → `23514 chk_requirement_child_refines`; both numbers → `23514 chk_requirement_number_or_child`; `X-TUI-8` → `23514 chk_requirement_key_shape`; duplicate key → `23505 uq_requirement_key`; duplicate child → `23505 uq_requirement_child` |
| **Runtime probe:** `string_to_array(split_part(key,'-',3),'.')::int[]` order equals `Vec<i32>` order | verified | Probe (c): `R-TUI-1.9 {1,9}`, `R-TUI-1.10 {1,10}`, `R-TUI-2 {2}`, `R-TUI-2.1 {2,1}`, `R-TUI-2.2 {2,2}`, `R-TUI-2.10 {2,10}`, `R-TUI-10 {10}`: lexicographic, shorter prefix first, as `Vec<i32>`. D6's full `ORDER BY` over the table gave `R-A1-10, W-ORCH-3, W-ORCH-3.1, R-TUI-2, R-TUI-2.1, R-TUI-2.1.1, R-TUI-5` |
| **Compile probe:** `MemStore` init at `mem.rs:524` needs the new counter field | verified | `mem.rs:453-537` is an exhaustive `State { … }` literal (no `..`), `requirement_key_counter: data.requirement_key_counter` at `:524`; `State` derives `Default` (`:234`) but the literal does not use it |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| **Renumber at collect:** hr/MOD-150 M3 also took `0034` and cache `0011` | High | `scripts/hr collect` renumbers. The second lander moves: the file names, `schema_version`, the `TABLES` comment, the applied list and the ~9 `Pending(N)` pins (memory: a new migration moves ~7 count pins plus `TABLES`), the `the_0011_…` test name, and the header cross-refs. `.sqlx` hashes do not change (query text holds no number) |
| `connection__confirm` snapshot moves | Low | No new mirrored table here. If it moves, it is MOD-150 M3's table after the merge |
| Refusal order differs between stores | Medium | D4 is one order. Conformance case 4 runs each refusal with nothing else wrong |
| sqlx prepare against a stale scratch DB | Medium | Recreate `htui_sqlx` before `migrate run` (recipe); `prepare --check` in T2 and Validation |
| The mirror drops `requirement` rows before the rebuild | Low | `0009`'s precedent: `schema_version` 34 forces the full rebuild on open |
| A cross-owner refinement outlives a project leaving its workspace | Medium | Recorded, not refused: the rule binds at mint (OQ5) |
| Pg-only debug stack overflow in a new conformance arm | Low | `boxed(...)` arms as every case does (memory: orch/pg stack headroom) |
| The sandbox cannot run live Qdrant | High | Unaffected by this item; host gate owed |

## Open questions (answered by the maintainer, 2026-11-14: every recommended answer accepted)

1. **`refines_id` on delete.** ANA's sketch has no action, so deleting a workspace whose W-
   requirement is refined by a member's would fail with `23503`, and MOD-147's workspace delete cascades
   the scope row. **Recommended:** `ON DELETE SET NULL`. The member's requirement becomes a plain
   top-level one with its key (D1, D9, case 7). Nested children cannot be orphaned: their parent goes
   with the same project, and a lone parent delete trips `chk_requirement_child_refines`.
2. **Existing scope-row requirements.** Nothing refuses an area in a scope row today, so a scope row may
   hold `R-` keys. **Recommended:** 0034 rewrites them to `W-` (none are expected; the migration test
   plants one).
3. **Leaf coverage**, which R-ENT-15 does not define. **Recommended:** D8. Covered = a live,
   non-suspect `addresses` citation by an item closed `done` or `concluded`. In progress = any live
   `addresses`. Open otherwise. `reserves` does not count. MOD-114 adds `verifies`; it should count as
   covering once it exists.
4. **Refining a withdrawn requirement.** **Recommended:** refuse it with `requirement_withdrawn`, as a
   withdrawn requirement takes no new `addresses`.
5. **"In progress when any is".** **Recommended:** any refiner covered or in progress. Withdrawn refiners
   are ignored. A requirement whose refiners are all withdrawn falls back to its own citations. The
   rule applies to any requirement with active refiners (nested or cross-owner), recursively, because
   `W-ORCH-3 → W-ORCH-3.1 → R-API-4` needs it.
6. **Scope of "a workspace requirement may refine only a workspace one".** **Recommended:** only its own
   workspace's, which makes it nested. Workspaces do not contain workspaces, so another workspace's
   requirement is out of reach.
7. **Who owns agent-proposed splits as drafts** (R-ENT-15). **Recommended:** MOD-118, whose line
   already carries "agent proposals and edits as drafts a person accepts". It needs no store change
   beyond `refines_id`.

## Acceptance

- [ ] T1-T3 complete, tests written before code; conformance `CASES` 319 on Mem and Pg
- [ ] `0034` + cache `0011` applied; migration tests green; pins moved
- [ ] `.sqlx` regenerated, `prepare --check` clean
- [ ] Validation passes in the sandbox; host Postgres/Qdrant gate noted as owed
- [ ] Open questions answered (or the recommendations accepted) before the decision record
- [ ] Bookkeeping per `workflow-docs.md`: decision record `docs/decisions/mod/mod-149.md`, HANDOFF line,
      MOD-118 unblocked note
