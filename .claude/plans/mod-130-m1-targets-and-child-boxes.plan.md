# Plan: MOD-130 M1 — Targets and child boxes exist

**Source PRD**: `.claude/prds/mod-130-execution-targets.prd.md`
**Selected Milestone**: 1 — Targets and child boxes exist
**Complexity**: Large
**Status**: complete (`9ea41c6b`..`f8c3d2b3`, 2026-11-15); review fix round R1 applied; sandbox gate green.
**Routing**: PRD path, ultracode for implement (C2, C4) (`/handoff-run`, 2026-11-14). The plan runs
without a confirm gate (handoff-run step 3).

**Blueprint**: `.claude/plans/mod-130-m1-targets-and-child-boxes.blueprint.md`: 12 architect deltas (B1-B12), all
accepted 2026-11-14 and folded into the task briefs (`mod-130-m1-targets-and-child-boxes.briefs/T1..T5.md`), where they
supersede the task text. T6 landed in `6ed37436`.

## Summary

After M1 the store knows what an execution target is and where each level of the cascade points:

- **`target` table.** It holds one seeded `local` row ("this box"). Targets of every other kind can be
  created and edited, and a `docker` target can be bound to a child box of a machine.
- **Box kinds.** `box` gains a kind (`machine | container`) and a parent.
- **Cascade levels.** A project (and, through its scope row, a workspace), a workflow and an item each
  name a target or inherit. The Settings default is one `app_setting` row.
- **Retire and delete.** A target in use cannot be retired or deleted.
- **Cache.** The mirror carries the targets and the level columns.

Nothing reads the cascade yet: the resolver, `enqueue` and the queue are M2 and M3, and the TUI is M4.
`run.target_id` and `run.target_source` exist as columns only, and M2 gives them a model field and a
writer.

## Shared decisions

Every task's brief carries this section.

### Decisions from the PRD gate (2026-11-14) that apply to M1

The gate decided D1..D18, which are OQ1..OQ18 of the PRD. M1 implements these:

| D | What M1 does with it |
|---|---|
| D1 | All five kinds are stored. Every kind but `local` derives `planned` in this build. |
| D2 | One seeded `local` target with `box_id NULL`, meaning the box that starts the run. It cannot be created, edited, retired or deleted. |
| D3 | `NULL` means inherit at every level. The Settings default is the `app_setting` row `default_target`, holding a target id; an absent row means the local target. |
| D4 | `status` is derived at read time and never stored. Its inputs are the kind's support in this build, the bound box and its host, and the stored `probed_at` and `probe_error`. |
| D5 | The workspace level is the workspace's scope `project` row (`project.target_id`). There is no `workspace` column. |
| D7 | `run.target_id` and `run.target_source` are added as columns now. M2 adds the model fields and writes them. |
| D11 | The item's target is a spec field, under `version` and the divergence merge like `step_graph_id`. Setting a workflow's target bumps `step_graph.revision`. |
| D12 | `box.kind` is `machine | container`. |
| D13 | `os_family` keeps its three values. Only a `container` box may hold `NULL`. |
| D14 | Retire and delete are refused while a level or the Settings default names the target, and the refusal lists them. Delete is also refused while a run references the target. Retire hides a target; nothing deletes one with history. |
| D15 | `target` is mirrored in the cache. |
| D18 | The `R-NF-2` sentence and its amendment-trail entry. |

D6, D8, D9, D10, D16 and D17 belong to M2 to M4.

### Decisions this plan takes

- **P1 — Migration `0037_execution_targets.sql`, cache migration `0013_execution_targets.sql`.**
- **P2 — The seed.**
  - `crates/htui-core/src/seed.rs` holds two constants:
    - `pub const LOCAL_TARGET_ID: TargetId = TargetId::from_uuid(Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001));`
    - `pub const LOCAL_TARGET_NAME: &str = "this box";`
  - `0037` inserts that row. MemStore seeds the same row when its state is built.
  - A migrations test pins the migration's row equal to the constants, as `0021`'s project types are
    pinned to `seed::PROJECT_TYPES`.
- **P3 — The `os_family` CHECK.** A plain `os_family IN (…)` passes on `NULL`, so the CHECK is written
  `CASE WHEN os_family IS NULL THEN kind = 'container' ELSE os_family IN ('windows','linux','macos') END`.
- **P4 — Who binds a box.** Only `attach_child_box` sets `target.box_id` in M1 (a `docker` target only).
  `NewTarget` has no box. MOD-131 adds the ssh binder.
- **P5 — Child box values.** A child box is inserted with:
  - the parent's `user_id` and the caller's `hostname`;
  - `os_family NULL`, `os_version ''`, `arch ''` and `htui_version ''`, so MOD-44's first probe always
    runs;
  - empty tags and `edit_version 0`.
  `BoxRow.os_family` becomes `Option<OsFamily>`, and so does `BoxProfile.os_family`. The render omits
  the OS line on `None`. `BoxInfo` (this box, always a machine) keeps `OsFamily`, and its Pg read uses
  the `"os_family!: OsFamily"` override.
- **P6 — Compare-and-set tokens.**
  - `target`, `project` and `step_graph` use `updated_at`, as `update_project` and `update_step_graph`
    do.
  - The Settings default uses the row's `updated_at`, with `None` meaning "I expect no row", exactly as
    `set_box_probe_spec` does.
  - The item uses `version` (D11).
  - Each setter is its own method, so `ProjectPatch` and `StepGraphPatch` do not change.
- **P7 — The cache mirror.**
  - Only this box is mirrored (`refresh_box` deletes the others), and this box is a machine. The mirror
    `box` table therefore stays unchanged.
  - The mirror gains a `target` table (a full replace, like `replace_workspace`), `project.target_id`,
    `item.target_id`, `run.target_id` and `run.target_source`.
  - `step_graph` is not mirrored.
- **P8 — Run history on MemStore waits for M2.** M1's `Run` has no `target_id`, so only Pg can see run
  history. Pg's `delete_target` refuses on run history now, and a `pg_criteria` test proves it. Mem's
  branch and its conformance coverage land with M2's `Run.target_id`.
- **P9 — Refusal sentences** are `#[must_use] pub fn … -> String` in `crates/htui-core/src/store/traits.rs`,
  beside `link_outside_reach`. They are returned as `StoreError::Constraint(sentence)`. A missing row is
  `StoreError::NotFound { entity: "target" | "box" | …, id }`. Neither store ever surfaces a raw
  SQLSTATE: `ON DELETE RESTRICT` is `23503` on PG 16 and `23001` on PG 18, so every refusal is checked in
  code before the statement runs. The sentences:
  - `target_built_in()`: "the local target is built in; it cannot be created, edited, retired or deleted"
  - `target_name_taken(name: &str)`: "a target named {name} exists"
  - `target_name_refused(name: &str)`: "a target name is 1-64 characters with no control character; {name:?} is not"
  - `target_locator_refused(why: &str)`: "a target locator holds names and addresses, never secrets: {why}"
  - `target_retired(name: &str)`: "target {name} is retired"
  - `target_in_use(name: &str, users: &[String])`: "target {name} is used by {users joined with ", "}; set them to inherit or another target first"
  - `target_has_runs(name: &str, runs: u64)`: "target {name} has {runs} run(s); retire it instead"
  - `target_not_docker(name: &str, kind: TargetKind)`: "only a docker target takes a container box; {name} is {kind}"
  - `target_bound_elsewhere(name: &str, other: BoxId)`: "target {name} is bound to box {other}"
  - `child_box_needs_a_machine(parent: BoxId)`: "box {parent} is a container; a child box needs a machine parent"
  - `box_not_a_child_of(child: BoxId, parent: BoxId)`: "box {child} exists and is not a container of box {parent}"

  A "user" (in `target_in_use`) is labelled `project <slug>`, or `workspace <slug without @>` for a
  scope row; `workflow <name>`; `item <key>`; or `Settings default`. The labels are sorted by bytes.
- **P10 — Read order.** `targets()` orders by `name` byte order: Pg `ORDER BY name COLLATE "C", id`,
  Mem a byte sort. Retired targets are included; pickers filter them later.
- **P11 — No `StoreRequest`, no TUI surface (D16).** The new methods sit on the store traits and are
  exercised by conformance cases only. The one TUI edit is the item spec field (T3), because the item
  form and the divergence view compile against `ItemSpec` and `SpecField`.

### Conventions

- **Conformance pattern.** Each case is `async fn <name><S: WriteStore>(store: &S)` and starts with
  `const CASE: &str = "<name>";` (`crates/htui-core/src/store/conformance.rs`, for example the case at
  `:36594`). It is registered twice: as a name in `CASES` (`:120`), and as the arm
  `"<name>" => { boxed(<name>, store).await; }` in `run_case_inline` (`:503`, see `:1496-1497`).
  Compare-and-set outcomes use the `applied`/`stale` helpers (`:5016`, `:5024`).
- **The case count is pinned twice and moves with every task that adds cases:**
  `crates/htui-store/tests/it/pg_conformance.rs:70` `const EXPECTED_CASES: usize = 324;` (with the doc
  text at `:69` and `:77`), and `crates/htui-core/tests/it/mem_store.rs:50-51`. The tasks run in series,
  so each one moves the count from the previous task's value. The planned counts are:
  - T1: 325
  - T2: 329
  - T3: 330
  - T4: 335
- **Every new or changed `query!`/`query_as!` needs `.sqlx`.** In the hr sandbox, the test Postgres is
  at `localhost:5439`, user `postgres`, trust auth, and there is no docker (`docs/hr-sandbox.md:227-242`):
  ```
  psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
  cd crates/htui-store
  export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations
  cargo sqlx prepare -- --all-targets --all-features     # keep both flags, or test-only queries are deleted
  cargo sqlx prepare --check -- --all-targets --all-features
  ```
  A "potentially unused queries" warning from `--check` is expected. Byte-identical SQL reuses an entry.
- **Tests.** Integration tests are modules of each crate's single `it` binary (`tests/it/<name>.rs`).
  Run one module as `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p <crate> --all-features -E
  'binary(it) and test(/^<module>::/)'`. `HTUI_TEST_DATABASE_URL` is already exported in the sandbox
  (`postgres://postgres@localhost:5439/postgres`). `testkit::fresh_db` copies a migrated template, so a
  migration test starts from `common::bare_db()` and `MIGRATOR.run_to(n, &db.pool)`. TDD: write each
  task's tests first and see them fail.
- **Lints.** `unsafe_code = "forbid"`, the workspace lints and the MSRV are unchanged. Every new pub item
  has a doc line, and docs cite `MOD-130 M1` plus the D or P number.
- **Commit incrementally**, one commit per green step, message `feat(store): MOD-130 M1 T<n> …`, ending
  with the attribution lines. Never stash; never push.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Typed `app_setting` key outside `SettingKey` | `crates/htui-core/src/model/box_.rs:152-156` `BOX_PROBE_SPEC_KEY`; trait `box_probe_spec`/`set_box_probe_spec`; case `box_probe_spec_is_cas_on_updated_at` (`conformance.rs:14509`, registered `:227-228`, `:779-784`) | `expected: None` = "no row expected"; `CasOutcome<Option<StoredSetting>>` |
| Compare-and-set on `updated_at` | `update_project` (`traits.rs:903`, `mem.rs:3552`, `pg/write.rs:4231`) | `CasOutcome::{Applied, Stale(current)}` (`traits.rs:4484`) |
| Graph revision bump | `State::bump_graph` (`mem.rs:4024`); Pg `bump_graph` (`pg/write.rs:2146`); `update_step_graph` sets `revision = revision + 1` (`pg/write.rs:4885`) | every graph write bumps once |
| Item spec field | `step_graph_id` in `ItemSpec` (`item_spec.rs:83`), `SpecChanges` (`:103`), `check_changes` (`:277-293`), `between` (`:462`), `into_patch` (`:487`), `SpecField::Graph` (`item_merge.rs:26`, `ALL` `:35`), `divergence.rs:437` | one field across spec, changes, patch, merge, label |
| Seeded rows pinned to code | `0021_project_type.sql:35-42` ↔ `seed.rs:478` `PROJECT_TYPES`, pinned at `tests/it/migrations.rs:3549` | migration INSERT, seed constant, equality test |
| Migration test | `migration_0034_creates_item_artifact_link` (about `tests/it/migrations.rs:7861`) | `bare_db` → `run_to(36)` → plant → `run_to(37)` → `pg_get_constraintdef` asserts |
| Column and table comments | `MOD139_*` consts (`migrations.rs:950`); the chain in `the_ana_column_comments_are_present_and_verbatim` (`:1246-1261`, `:1325-1340`, `:1350-1365`), table comments `:1283-1290` | verbatim pin, set equality |
| Cache full replace | `WORKSPACE_COLUMNS` (`cache/refresh.rs:528`), `replace_workspace` (`:538`), `run_pass` unscoped block (`:246-250`) | DELETE all, upsert all |
| Cache column add | `cache_migrations/0006_run_workspace.sql` (`ALTER TABLE run ADD COLUMN workspace_id TEXT;`); `PROJECT_COLUMNS` (`:694`), `ITEM_COLUMNS` (`:860`), `RUN_COLUMNS` (`:1191`) | ALTER ADD, append to the column const |
| Cache migration test | `cache_0011_applies_over_a_populated_0010_mirror` (`migrations.rs:7612-7700`) | `CACHE_MIGRATOR.run_to(12)`, plant, `.run()`, `pragma_table_info` |

## Files to Change

| File | Task |
|---|---|
| `crates/htui-store/migrations/0037_execution_targets.sql` (CREATE) | T1 |
| `crates/htui-core/src/model/target.rs` (CREATE), `model/mod.rs`, `model/ids.rs`, `model/box_.rs`, `seed.rs` | T1 |
| `crates/htui-core/src/store/traits.rs`, `store/mem.rs`, `store/conformance.rs`, `fixtures.rs`, `prompt/render.rs`, `prompt/fixtures.rs` | T1 (and T2-T4 for traits, mem, conformance) |
| `crates/htui-store/src/pg/{read,write,mod}.rs`, `src/writer.rs`, `src/backend.rs`, `src/cache/read.rs` (stub) | T1 (and T2-T4 for pg and writer) |
| `crates/htui-store/tests/it/{migrations,connect,pg_conformance}.rs`, `crates/htui-core/tests/it/mem_store.rs` | T1 (count pins also T2-T4) |
| `crates/htui-agent/src/conformance.rs` (UsageSpy), `crates/htui-agent/tests/it/recorder.rs` (SpyStore) | T1-T4 |
| `crates/htui/src/provision/verify.rs`, `crates/htui/src/ui/tabs/settings/boxes.rs` (BoxRow literals) | T1 |
| `crates/htui-core/src/model/{item,item_spec,item_merge}.rs`, `crates/htui/src/item_writes.rs`, `crates/htui/src/ui/tabs/backlog/{item_form,divergence}.rs` and every `Item`/`ItemSpec`/`ItemPatch`/`NewItem` literal | T3 |
| `crates/htui-core/src/model/{hierarchy,kind}.rs` and every `Project`/`StepGraph` literal; `crates/htui-store/tests/it/pg_criteria.rs` | T4 |
| `crates/htui-store/cache_migrations/0013_execution_targets.sql` (CREATE), `src/cache/{mod,refresh,read}.rs`, `tests/it/cache.rs`, `tests/it/migrations.rs`, `crates/htui/tests/it/snapshots/it__connection__confirm.snap` | T5 |
| `docs/REQUIREMENTS.md` | T6 |

## Tasks

The tasks form one serial lane, T1 → T2 → T3 → T4 → T5, because they all share `traits.rs`, `mem.rs`,
`pg/write.rs`, `conformance.rs`, the spies and `.sqlx`. T6 is independent: its file set is
`{docs/REQUIREMENTS.md}`, which no other task touches. **T1 is `verify: foundation`.**

### Task 1: The schema, the target model and the box kinds (`verify: foundation`)

**Depends on:** nothing. **Files:** the T1 rows of Files to Change.

**Migration `crates/htui-store/migrations/0037_execution_targets.sql`.** It opens with a header comment
in the house style (see `0026`/`0036`): forward-only (`R-STO-5`), a headless worker never migrates, and
the mirror is rebuilt once through the schema version. The DDL:

```sql
CREATE TABLE target (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL CONSTRAINT uq_target_name UNIQUE,
    kind        TEXT NOT NULL CONSTRAINT chk_target_kind
                CHECK (kind IN ('local','docker','ssh','ssh_docker','enrolled')),
    locator     JSONB NOT NULL DEFAULT '{}'
                CONSTRAINT chk_target_locator_object CHECK (jsonb_typeof(locator) = 'object'),
    box_id      UUID CONSTRAINT fk_target_box REFERENCES box(id) ON DELETE RESTRICT,
    probed_at   TIMESTAMPTZ,
    probe_error TEXT,
    retired_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_target_local_shape CHECK (kind <> 'local'
        OR (box_id IS NULL AND locator = '{}'::jsonb AND retired_at IS NULL))
);
CREATE UNIQUE INDEX uq_target_one_local ON target (kind) WHERE kind = 'local';
CREATE TRIGGER trg_target_updated_at BEFORE UPDATE ON target
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
INSERT INTO target (id, name, kind)
    VALUES ('00000000-0000-4000-8000-000000000001', 'this box', 'local');

ALTER TABLE box ADD COLUMN kind TEXT NOT NULL DEFAULT 'machine'
    CONSTRAINT chk_box_kind CHECK (kind IN ('machine','container'));
ALTER TABLE box ADD COLUMN parent_box_id UUID
    CONSTRAINT fk_box_parent REFERENCES box(id) ON DELETE RESTRICT;
ALTER TABLE box ADD CONSTRAINT chk_box_kind_parent
    CHECK ((kind = 'container') = (parent_box_id IS NOT NULL));
ALTER TABLE box ADD CONSTRAINT chk_box_parent_not_self CHECK (parent_box_id <> id);
ALTER TABLE box DROP CONSTRAINT box_os_family_check;
ALTER TABLE box ALTER COLUMN os_family DROP NOT NULL;
ALTER TABLE box ADD CONSTRAINT chk_box_os_family CHECK (CASE WHEN os_family IS NULL
    THEN kind = 'container' ELSE os_family IN ('windows','linux','macos') END);
CREATE INDEX idx_box_parent ON box (parent_box_id) WHERE parent_box_id IS NOT NULL;

ALTER TABLE project    ADD COLUMN target_id UUID CONSTRAINT fk_project_target    REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE step_graph ADD COLUMN target_id UUID CONSTRAINT fk_step_graph_target REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE item       ADD COLUMN target_id UUID CONSTRAINT fk_item_target       REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE run        ADD COLUMN target_id UUID CONSTRAINT fk_run_target        REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE run ADD COLUMN target_source TEXT CONSTRAINT chk_run_target_source
    CHECK (target_source IN ('item','workflow','project','workspace','settings','default'));
ALTER TABLE run ADD CONSTRAINT chk_run_target_pair CHECK ((target_id IS NULL) = (target_source IS NULL));
CREATE INDEX idx_project_target    ON project    (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_step_graph_target ON step_graph (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_item_target       ON item       (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_run_target        ON run        (target_id) WHERE target_id IS NOT NULL;
```

Comments to add:
- `COMMENT ON TABLE target` (ANA-42 §7.1).
- `COMMENT ON COLUMN` for `target.kind`, `target.locator` ("names and addresses only, never values"),
  `target.box_id`, `target.probe_error`, `target.retired_at`, `box.kind`, `box.parent_box_id`,
  `project.target_id` ("NULL = inherit; on a scope row, the workspace level (D5)"), `step_graph.target_id`,
  `item.target_id`, `run.target_id` and `run.target_source`.
- Do not comment `box.os_family`.

**Model, `crates/htui-core/src/model/target.rs` (new, re-exported from `model/mod.rs`):**

```rust
str_enum!(/// `target.kind` (D1). TargetKind { Local => "local", Docker => "docker", Ssh => "ssh",
    SshDocker => "ssh_docker", Enrolled => "enrolled" });
str_enum!(/// D4: derived, never stored. TargetStatus { Ready => "ready", Unprepared => "unprepared",
    Unreachable => "unreachable", Planned => "planned" });
pub const RUNNABLE_KINDS: &[TargetKind] = &[TargetKind::Local];   // MOD-44, MOD-131, MOD-47 extend it
impl TargetKind {
    pub fn runnable(self) -> bool;                    // RUNNABLE_KINDS.contains(&self)
    pub fn owner(self) -> Option<&'static str>;       // Docker→"MOD-44", Ssh|SshDocker→"MOD-131", Enrolled→"MOD-47", Local→None
}
pub struct Target { pub id: TargetId, pub name: String, pub kind: TargetKind, pub locator: Value,
    pub box_id: Option<BoxId>, pub probed_at: Option<DateTime<Utc>>, pub probe_error: Option<String>,
    pub retired_at: Option<DateTime<Utc>>, pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc> }
impl Target {
    /// D4, with `bound` = the row of `box_id` and `host` = its parent (docker) or itself (ssh_docker).
    pub fn status(&self, bound: Option<&BoxRow>, host: Option<&BoxRow>) -> TargetStatus;
    pub fn status_with(&self, runnable: &[TargetKind], bound: Option<&BoxRow>, host: Option<&BoxRow>) -> TargetStatus;
}
pub fn target_name_refusal(name: &str) -> Option<String>;   // P9 sentence; trims, 1..=64 chars, no control char
pub fn locator_refusal(locator: &Value) -> Option<String>;   // not an object, or any key (any depth, ASCII case-insensitive)
    // in {password, passwd, secret, token, api_key, apikey, private_key, credential, credentials}
pub const DEFAULT_TARGET_KEY: &str = "default_target";       // D3
```

`status_with` rule order:
1. Not in `runnable` → `Planned`.
2. `Local` → `Ready`.
3. `bound` is `None` → `Unprepared`.
4. `probe_error` is set → `Unreachable`.
5. `probed_at` is `None` → `Unprepared`.
6. A `Docker` or `SshDocker` target whose `host` lacks `docker` in `probed_tags ∪ declared_tags` →
   `Unreachable`.
7. Otherwise `Ready`.

**Other model changes:**
- `model/ids.rs`: add `TargetId` after `ArtifactLinkId` (`:137`): "`target.id` (MOD-130 M1)". Add it to the
  `model/mod.rs:138` re-export.
- `model/box_.rs`:
  - `str_enum!(BoxKind { Machine => "machine", Container => "container" })`.
  - `BoxRow` gains `pub kind: BoxKind` and `pub parent_box_id: Option<BoxId>`, and `os_family` becomes
    `Option<OsFamily>`.
  - `BoxProfile.os_family` becomes `Option<OsFamily>`, and `render.rs`'s `box_profile` (`:416`) omits
    the OS line on `None`.
- `seed.rs`: the P2 constants and `pub fn local_target(at: DateTime<Utc>) -> Target`.

**Readers.** Add to `ReadStore` (`traits.rs`):

```rust
async fn targets(&self) -> Result<Vec<Target>>;            // P10 order, retired included
async fn target(&self, id: TargetId) -> Result<Target>;    // NotFound { entity: "target" }
```

Implement them in:
- `MemStore`: a new `State.targets: BTreeMap<TargetId, Target>`, seeded with `seed::local_target`.
- `PgStore` (`pg/read.rs`).
- `Writer` (`writer.rs:133`) and `Backend` (`backend.rs:1026`).
- `CacheStore` (`cache/read.rs:435`): a temporary `Ok(vec![seed::local_target(epoch)])` and NotFound
  otherwise, marked `// MOD-130 M1 T5 replaces`.
- `UsageSpy` and `SpyStore` (forwarders).

Every Pg `query_as!` that reads a full box row adds `kind` and `parent_box_id` and reads `os_family` as
`Option`: `pg/read.rs:2024`, `:2294`; `pg/write.rs:3593`; the `boxes()` mapping at `pg/write.rs:3517`.
`BoxInfo`'s query uses `"os_family!: OsFamily"`. Each `BoxRow {` literal gains
`kind: BoxKind::Machine, parent_box_id: None, os_family: Some(…)`: `fixtures.rs:508`,
`conformance.rs:14036`, `:14145`, `pg/write.rs:3517`, `htui/src/provision/verify.rs:188`,
`htui/src/ui/tabs/settings/boxes.rs:1481`, and the tests in `box_.rs` (`:445`, `:489`, `:503`, `:573`).

**Tests first:**
- **In-file unit tests in `model/target.rs`:**
  - `every_kind_but_local_is_planned_in_this_build`.
  - `status_follows_the_d4_order`, which drives `status_with(&[all kinds], …)` through rules 1-7.
  - `locator_refusal_names_a_secret_key_at_any_depth`.
  - `a_name_is_trimmed_and_bounded`.
  - `kind_and_status_match_the_check_list`, which pins the `str_enum` spellings against the DDL lists.
- **In-file test in `model/box_.rs`:** `box_kind_matches_the_check_list`.
- **In-file test in `prompt/render.rs`:** `a_box_without_an_os_family_omits_the_os_line`. Every existing
  render and digest test stays unedited.
- **Conformance case `the_local_target_is_seeded`:**
  - `targets()` is exactly `[local]` with id `LOCAL_TARGET_ID`, name "this box", kind `local`, locator
    `{}`, no box and not retired.
  - Its `status(None, None)` is `Ready`.
  - `target(TargetId::new())` is NotFound `target`.
- **In `tests/it/migrations.rs`:**
  - `migration_0037_adds_targets_and_box_kinds`:
    1. Use `bare_db`, `run_to(36)`, and plant a user, a box (`insert_box` `:1932`), a project, an item, a
       graph and a run (`insert_run` `:1946`).
    2. Run `run_to(37)`.
    3. The old rows read `box.kind = 'machine'`, `parent_box_id NULL` and every `target_id NULL`.
    4. The local row exists.
    5. These are refused, each with `23514` (check_violation):
       - a second `local`;
       - a machine with `os_family NULL`;
       - a container without a parent;
       - a machine with a parent;
       - a locator `'[]'`;
       - `target_source` without `target_id`;
       - `target_source = 'elsewhere'`.
    6. A container with `os_family NULL` and a parent is accepted.
    7. The constraint texts are pinned with `pg_get_constraintdef`.
    8. Updating a target bumps `updated_at`.
  - `migration_0037_seeds_the_code_seed`: the row equals `seed::LOCAL_TARGET_ID` and
    `seed::LOCAL_TARGET_NAME`.

**Pins moved:**
- `tests/it/migrations.rs`:
  - the applied vec at `:128-152` gains `37`, and the message gains "MOD-130's
    0037_execution_targets.sql";
  - `TABLES` (`:37`) gains `"target"` after `"activity_event"`, under a comment line `// 0037_execution_targets.sql (MOD-130 M1)`;
  - the doc at `:25-36` names it;
  - the length goes from 57 to 58 at `:167`, with its message;
  - a new `MOD130_COLUMN_COMMENTS` is chained at `:1246-1261`, `:1325-1340` and `:1350-1365`;
  - a new `MOD130_TABLE_COMMENT` is added to the table-comment check (`:1283-1290`).
- `Pending(36)` → `Pending(37)` at `connect.rs:140`, `:241` and `migrations.rs:4270`, `:4371`.
- `MigrationsPending(36)` → `MigrationsPending(37)` at `migrations.rs:4380`, `:4403`.
- (fact-check) `connect.rs:154-155` `ConnEvent::MigrationsPending(_, pending) => assert_eq!(pending, 36, …)` → 37, and
  `migrations.rs:4554-4555` the `_sqlx_migrations` row count 36 → 37. Also every "thirty-six" in those messages
  (`connect.rs:141`, `:156-157`, `:242`; `migrations.rs:4271`, `:4372`, `:4381`, `:4404`, `:4556`) becomes "thirty-seven".
- (fact-check) `str_enum!` is textually scoped: declare `mod target;` in `model/mod.rs` *below* the macro at `:25`.
- `EXPECTED_CASES` goes from 324 to 325 (`pg_conformance.rs:69-77`) and `mem_store.rs:50-51` from 324 to 325.
- `.sqlx` is regenerated.

**Check:**

```
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(migrations|connect|pg_conformance|box_identity)::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-agent --all-features -E 'binary(it) and test(/^recorder::/)'
cargo check -p htui -p htui-orch -p htui-worker -p htui-mcp --all-features --tests
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
```

### Task 2: Targets are created, edited and bound to a child box

**Depends on:** T1. **Files:** `traits.rs`, `mem.rs`, `conformance.rs`, `pg/write.rs`, `writer.rs`,
`htui-agent/src/conformance.rs`, `htui-agent/tests/it/recorder.rs`, `pg_conformance.rs`,
`mem_store.rs`, `.sqlx`.

**Model (`model/target.rs`):**

```rust
pub struct NewTarget { pub id: TargetId, pub name: String, pub kind: TargetKind, pub locator: Value }   // P4: no box
#[derive(Default)] pub struct TargetEdit { pub name: Option<String>, pub locator: Option<Value> }       // kind and box never edited
pub struct NewChildBox { pub id: BoxId, pub parent: BoxId, pub hostname: String, pub target: TargetId }
pub struct AttachedBox { pub child: BoxRow, pub target: Target }
```

**`WriteStore` methods**, implemented on Mem, Pg, Writer, UsageSpy and SpyStore. They are not on
`WorkerStore`:

```rust
async fn create_target(&self, new: NewTarget) -> Result<Target>;
async fn edit_target(&self, id: TargetId, expected: DateTime<Utc>, edit: TargetEdit) -> Result<CasOutcome<Target>>;
async fn attach_child_box(&self, new: NewChildBox) -> Result<AttachedBox>;
```

**`create_target` refusals, in order:**
1. `kind == Local` → `target_built_in()`.
2. `target_name_refusal`.
3. `locator_refusal` → `target_locator_refused`.
4. Name taken → `target_name_taken`. Check this before the INSERT; Pg must never surface `23505`.

`create_target` is idempotent on `id` when every field is equal: it returns the stored row, as
`create_project` does.

**`edit_target` refusals, in order:**
1. NotFound `target`.
2. Local → `target_built_in()`.
3. The name and locator refusals as above.
4. `Stale(current)` when `expected` is not the row's `updated_at`.
5. A name taken by another target → `target_name_taken`.

A retired target can still be edited.

**`attach_child_box` refusals, in order:**
1. NotFound `target`.
2. Retired → `target_retired`.
3. `kind != Docker` → `target_not_docker`.
4. NotFound `box` for the parent.
5. The parent is a container → `child_box_needs_a_machine`.
6. `id` exists and is not a container of `parent` → `box_not_a_child_of`.
7. `target.box_id` is `Some(other)` with `other != id` → `target_bound_elsewhere`.

Then it inserts the child (P5) unless it exists, and sets `target.box_id = id` unless already set. A
repeat returns the same `AttachedBox` and writes nothing: the target's `updated_at` does not move. On Pg
it is one transaction and takes `SELECT … FROM target WHERE id = $1 FOR UPDATE` before checking.

**Tests first.** Four conformance cases, registered in `CASES` and `run_case_inline`:
- `create_target_round_trips_every_kind_but_local`:
  - Create docker, ssh, ssh_docker and enrolled targets.
  - `targets()` lists local plus the four in name byte order.
  - Each status is `Planned` (D1).
  - `target(id)` equals what `create_target` returned.
  - A repeat create is idempotent.
- `create_target_refusals`: each P9 sentence, compared exactly with `constraint_exactly`
  (`conformance.rs:23301`, signature `(case, outcome: Result<T, StoreError>, sentence, what)`), in the order above, and nothing written.
- `edit_target_is_cas_on_updated_at`:
  - Rename and change the locator → Applied, and `updated_at` moves.
  - A spent token → `Stale(current)`.
  - Editing the local target → `target_built_in`.
  - A name clash → `target_name_taken`.
- `attach_child_box_creates_reuses_and_refuses`:
  - The child has `kind container`, the parent, `os_family None`, `htui_version ""` and the parent's
    user, and the target's `box_id` is set.
  - A repeat makes no write (`updated_at` unchanged).
  - Each refusal 1, 3, 4, 5, 6 and 7 fires. Refusal 2 is covered in T4, after retire exists.
  - `boxes()` now lists the child.

**Pins moved:** the case count goes from 325 to 329 in both files. `.sqlx`.

**Check:**

```
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(pg_conformance|box_identity)::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-agent --all-features -E 'binary(it) and test(/^recorder::/)'
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
```

### Task 3: The item's target is a spec field (D11)

**Depends on:** T2. **Files:**
- `crates/htui-core/src/model/{item,item_spec,item_merge}.rs`, `fixtures.rs`, `store/mem.rs`,
  `store/conformance.rs`;
- `crates/htui-store/src/pg/{read,write}.rs`, `src/cache/read.rs` (literal only), `.sqlx`;
- `crates/htui/src/item_writes.rs`, `crates/htui/src/ui/tabs/backlog/{item_form,divergence}.rs`;
- every other `Item {`, `ItemSpec {`, `ItemPatch {` and `NewItem {` literal the compiler names (about
  40, 64, 88 and 45 sites across `htui-core`, `htui-orch`, `htui-store`, `htui-worker`, `htui-mcp` and
  `htui`);
- the count pins.

**Types:**
- `Item` (`item.rs:157`) gains `pub target_id: Option<TargetId>` after `step_graph_id`.
- `NewItem` (`:294`) gains `pub target_id: Option<TargetId>`.
- `ItemPatch` (`:322`) gains `pub target_id: Option<Option<TargetId>>`, with the same serde shape as
  `step_graph_id` (`:337`).
- `ItemSpec` (`item_spec.rs:69`) gains `pub target_id: Option<TargetId>`.
- `SpecChanges` gains `pub target_id: Option<Option<TargetId>>`, and its manual `Debug` (`:117`, `:133`)
  names it.
- `ItemSpec::of`, `into_new_item` (`:425`), `into_patch` (`:487`) and `SpecChanges::between` (`:462`)
  carry it.
- `check_spec` and `check_changes` need no target rule: whether the target exists is the store's check.
- `SpecField` (`item_merge.rs:16`) gains `Target`, after `Graph`. `ALL` becomes `[Self; 8]`, the label is
  "target", and `state`, `resolve` and `moved` handle it like `Graph`.
- `divergence.rs` `Labels::value` (`:430`) gets `SpecField::Target => target_text(spec.target_id)`:
  "inherit" for `None`, otherwise the first 8 hex digits of the id. A comment says MOD-119 replaces this
  with the name.
- `item_form.rs` `parsed` (`:883`) carries `target_id: base.and_then(|b| b.target_id)`, so an edit keeps
  the stored target. The form offers no target widget (D16).

**Store.** `create_item` and `update_item` (Mem `mem.rs:11227`, Pg `pg/write.rs:2766`) write
`target_id`, refusing first:
1. a `Some(id)` that is not a target → NotFound `target`;
2. a retired target → `target_retired`.

Setting `None` is always allowed. The item's `version` moves as for any spec field. Every Pg `query_as!`
that reads an item adds `target_id`: `pg/read.rs:191`, `:1510`; `pg/write.rs:2684`, `:2773`. The cache's
`Item {` literal sets `target_id: None` with `// MOD-130 M1 T5 reads the mirror`.

**Tests first:**
- **In-file `item_spec.rs` tests:**
  - extend `into_patch_and_into_new_item_carry_every_field` and
    `between_names_only_the_fields_that_differ`;
  - add `a_target_change_is_a_spec_change`.
- **In-file `item_merge.rs` tests:**
  - `a_target_conflict_resolves_like_the_graph`;
  - `changed_lists_fields_in_form_order` with `Target` after `Graph`.
- **In-file `divergence.rs` test:** `a_target_row_reads_inherit_or_the_short_id`.
- **In-file `item_form.rs` test:** `an_edit_keeps_the_items_target`, where an item with a target is
  edited on its title and the `EditItem` changes hold no `target_id`.
- **Conformance case `item_target_is_a_spec_field`:**
  - create with a target → read back;
  - `update_item` to `Some(other)` and then to `None`, each moving `version`;
  - a spent version → the existing stale outcome;
  - an unknown target → NotFound `target`.

**Pins moved:**
- The case count goes from 329 to 330.
- `.sqlx`.
- No snapshot should move, because the fixtures' items carry `target_id: None`. Run
  `cargo insta test -p htui --all-features --check`; a moved snapshot is a bug.

**Check:**

```
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'test(/divergence|item_form|item_writes/) or (binary(it) and test(/^item_writes_pg::/))'
cargo check -p htui-orch -p htui-worker -p htui-mcp -p htui-agent --all-features --tests
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
```

### Task 4: Project, workspace, workflow and Settings levels; retire and delete

**Depends on:** T3. **Files:**
- `crates/htui-core/src/model/{hierarchy,kind,target}.rs`, `store/{traits,mem,conformance}.rs`,
  `fixtures.rs`;
- `crates/htui-store/src/pg/{read,write}.rs`, `src/writer.rs`, `src/cache/read.rs` (literal only),
  `tests/it/pg_criteria.rs`, `.sqlx`;
- the spies;
- every `Project {` literal (8 sites, one of which, `engine.rs:25797`, uses `..project`) and `StepGraph {` literal
  (20 sites incl. `Ok(Self {` at `pg/read.rs:4056`; 3 use `..` so 17 need the field) the compiler names;
- the count pins.

**Types:**
- `Project` (`hierarchy.rs:59`) gains `pub target_id: Option<TargetId>`. On a scope row this is the
  workspace level (D5).
- `StepGraph` (`kind.rs:448`) gains `pub target_id: Option<TargetId>`.
- `ProjectPatch` and `StepGraphPatch` are unchanged (P6).

**`WriteStore` methods:**

```rust
async fn set_project_target(&self, project: ProjectId, expected: DateTime<Utc>, target: Option<TargetId>) -> Result<CasOutcome<Project>>;
async fn set_graph_target(&self, graph: StepGraphId, expected: DateTime<Utc>, target: Option<TargetId>) -> Result<CasOutcome<StepGraph>>;   // bumps revision (D11)
async fn default_target(&self) -> Result<Option<StoredSetting>>;                     // app_setting DEFAULT_TARGET_KEY; value = JSON string of the id
async fn set_default_target(&self, target: Option<TargetId>, expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Option<StoredSetting>>>;   // None deletes the row (D3)
async fn retire_target(&self, id: TargetId, expected: DateTime<Utc>) -> Result<CasOutcome<Target>>;
async fn delete_target(&self, id: TargetId) -> Result<()>;
```

**Refusals shared by the three setters.** For a `Some(t)`:
1. NotFound `target`;
2. retired → `target_retired`.

Before those, the setter's own NotFound: `project`, `step_graph`, or none for the default. After them,
`Stale(current)`. Setting a project's target does not move `revision`. `set_graph_target` uses
`bump_graph` on both stores and is added to `every_graph_write_bumps_revision` (`conformance.rs:28480`).

**`retire_target` refusals, in order:**
1. NotFound.
2. Local → `target_built_in`.
3. In use → `target_in_use(name, users)`, with P9 labels from `project.target_id` (scope rows labelled
   workspace), `step_graph.target_id`, `item.target_id` and the default row.
4. `Stale`.

On success it sets `retired_at`. It is idempotent on an already retired target: Applied, unchanged.

**`delete_target` refusals, in order:**
1. NotFound.
2. Local.
3. In use, as for retire.
4. Pg only (P8): a run references the target → `target_has_runs(name, n)`.

It deletes otherwise.

Reads: every Pg `query_as!` reading a project or graph row adds `target_id` (`pg/read.rs:729`, `:3291`,
`:3329`, `:3664`; `pg/write.rs:4189`, `:4268`, `:4836`, `:4880`; `StepGraphRow` `pg/read.rs:4044`). The
cache's `Project {` literal sets `target_id: None` with `// MOD-130 M1 T5`.

**Tests first.** Five conformance cases:
- `project_and_scope_row_targets_are_cas`:
  - Set, clear and stale on a project and on a workspace's scope row.
  - The scope row's value reads back on `project(scope_id)`.
  - `revision` of the project's graphs does not move.
- `graph_target_bumps_revision`:
  - `set_graph_target` bumps `revision` by exactly 1 and moves `updated_at`.
  - A spent token gives `Stale` with the revision unchanged.
- `default_target_is_cas_on_updated_at`:
  - Absent at first.
  - Set with `expected: None` → Applied; the value is the id string.
  - A second `expected: None` → Stale.
  - Clear with the row's token → the row is gone.
- `retire_and_delete_refuse_a_target_in_use`:
  - One docker target is named by a project, a scope row, a graph, an item and the default.
  - Retire and delete each give exactly the sorted sentence.
  - Clear every level → retire Applied, and `targets()` still lists it with `retired_at` set.
  - Delete a second, unused target → it is gone.
  - Local → `target_built_in` for both.
- `a_retired_target_is_refused_everywhere`: after a retire, each of `set_project_target`,
  `set_graph_target`, `set_default_target`, `update_item` and `attach_child_box` gives `target_retired`
  and writes nothing.

`pg_criteria.rs` gets `delete_target_refuses_one_with_run_history`: plant a run with `target_id` and
`target_source = 'item'` through SQL, then `delete_target` gives `target_has_runs(name, 1)` and the row
stays.

**Pins moved:**
- The case count goes from 330 to 335.
- `every_graph_write_bumps_revision` gains the new writer.
- `.sqlx`.
- `pg_criteria`'s cascade `TABLES` and `DeleteReach` do not move: no table loses rows on a project
  delete (prove it by running that test).

**Check:**

```
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(pg_conformance|pg_criteria)::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-agent --all-features -E 'binary(it) and test(/^recorder::/)'
cargo check -p htui -p htui-orch -p htui-worker -p htui-mcp --all-features --tests
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
```

### Task 5: The cache mirrors targets and the level columns (D15)

**Depends on:** T4. **Files:** `crates/htui-store/cache_migrations/0013_execution_targets.sql` (new),
`crates/htui-store/src/cache/{mod,refresh,read}.rs`, `crates/htui-store/tests/it/{cache,migrations}.rs`,
`crates/htui/tests/it/snapshots/it__connection__confirm.snap`.

**Cache migration (P7).** The header follows `0012`'s, stating that `schema_version` moves to 37
through `PgStore::schema_version()` and forces a full rebuild, so there is no backfill.

```sql
CREATE TABLE target (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, locator TEXT NOT NULL,
    box_id TEXT, probed_at TEXT, probe_error TEXT, retired_at TEXT, created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL);
ALTER TABLE project ADD COLUMN target_id TEXT;
ALTER TABLE item    ADD COLUMN target_id TEXT;
ALTER TABLE run     ADD COLUMN target_id TEXT;
ALTER TABLE run     ADD COLUMN target_source TEXT;
```

The column types follow the mirror's conventions for UUID, JSONB and timestamp columns. Read
`0001_mirror.sql`'s `box` and `workspace` and copy their spelling.

**Code:**
- `MIRRORED_TABLES` (`cache/mod.rs:51`) becomes `[&str; 24]`, with `"target"` after `"box"`. Its doc
  gains a "`target` joined in MOD-130 M1 (D15)" paragraph.
- `refresh.rs`:
  - `TARGET_COLUMNS` and `replace_target`, modelled on `WORKSPACE_COLUMNS`/`replace_workspace`
    (`:528`, `:538`), called in `run_pass`'s unscoped block (`:246-250`) after `replace_workspace`;
  - a `TARGET` const beside `:197-213`;
  - `"target_id"` appended to `PROJECT_COLUMNS` (`:694`) and `ITEM_COLUMNS` (`:860`);
  - `"target_id", "target_source"` appended to `RUN_COLUMNS` (`:1191`). M2 reads them.
- `cache/read.rs`:
  - `targets()` and `target()` read the mirror, replacing T1's stub;
  - the `Project {` and `Item {` literals read `target_id`, replacing T3's and T4's `None`.

**Tests first:**
- In `tests/it/cache.rs`:
  - `a_refresh_mirrors_targets_and_the_level_columns`: create a docker target, set it on a project, an
    item and a scope row, refresh, and the offline reads equal the online ones.
  - `a_retired_target_stays_in_the_mirror`.
  - `a_deleted_target_leaves_the_mirror`.
  - Move `MIRRORED_TABLES.len()` from 23 to 24 at `cache.rs:3150`.
- In `tests/it/migrations.rs`: `cache_0013_applies_over_a_populated_0012_mirror`, after `cache_0011_…`
  (`:7612`). It uses `CACHE_MIGRATOR.run_to(12)`, plants a project, an item and a run, `.run()`, then
  asserts the new columns through `pragma_table_info` and the `target` table.

**Pins moved:**
- `it__connection__confirm.snap:11`: "the 23 mirrored tables" becomes "the 24 mirrored tables". Re-run
  the test and review the wrap with `cargo insta review`.
- `crates/htui/src/ui/tabs/settings/connection.rs:221` and `:1168` read `TABLES.len()` and need no edit.
  Their tests run in the check below.

**Check:**

```
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^(cache|migrations)::/)'
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'test(/connection/)'
cargo insta test -p htui --all-features --check
```

### Task 6: The `R-NF-2` amendment (D18)

**Depends on:** nothing. **Files:** `docs/REQUIREMENTS.md` only.

**Edit** `R-NF-2` (`docs/REQUIREMENTS.md:892-902`). Keep every existing sentence. Directly after "except
the code-context tools in coding sessions (R-LATER-7)." insert:

> `dockerd` and `sshd` are opt-in per execution target (R-ORCH-15): only a docker or ssh target needs
> them, gated by the box probe's `docker` tag and a successful target probe; a project that never adds
> such a target needs neither.

Then end the amendment trail with "…, and on 2026-10-20 (one Qdrant for the whole database), and on
2026-11-14 (ANA-42, §8 Q4: `dockerd` and `sshd` opt-in per target)." Keep the existing "and on
2026-10-20" wording grammatical. No other requirement changes.

**Check:** `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation (main thread, once for the lane)

```bash
bash scripts/gate.sh
cargo clippy --workspace -- -D warnings          # featureless; --all-features hides dead code
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

The live-Qdrant suites and the host Postgres 16 run are owed on the host (sandbox).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The `Item`/`ItemSpec`/`ItemPatch`/`NewItem` ripple (about 240 literals) hides a site that drops the target silently | High | The compiler names every literal. No `..Default` is added where there was none. `an_edit_keeps_the_items_target` and `into_patch_and_into_new_item_carry_every_field` catch a dropped carry |
| Nullable `os_family` changes `sqlx`'s inferred type in every box query | High | P5: `Option` in `BoxRow`; the `!` override only for `BoxInfo`; `cargo sqlx prepare --check` |
| A plain `IN` CHECK lets a machine store `NULL` | Medium | P3's `CASE` form, and the migration test refuses it |
| Migration `0037` or cache `0013` collides with a parallel branch at collect | Medium | Renumber at collect, and re-pin `Pending(N)`, the applied vec, `TABLES`, the comment chains and `MIRRORED_TABLES` |
| Pg raises `23503`/`23001` instead of a sentence | Medium | P9: every RESTRICT is pre-checked in code; the tests compare sentences, never SQLSTATEs |
| The divergence view shows a short id instead of a name | Low | Accepted for M1 (D16); MOD-119 owns the label |

## Claims to verify

1. The latest Pg migration is `crates/htui-store/migrations/0036_activity_event.sql`; there is no `0037_*`.
2. The latest cache migration is `crates/htui-store/cache_migrations/0012_requirement_keys.sql`; there is no `0013_*`.
3. `0001_init.sql:57` declares `os_family TEXT NOT NULL CHECK (os_family IN ('windows','linux','macos'))` as a column CHECK, which Postgres names `box_os_family_check`.
4. No later migration alters `box.os_family` or its CHECK; only `0005_box_identity.sql:15-20` alters `box`.
5. In Postgres, a CHECK whose expression evaluates to NULL passes, so `os_family IN (…)` admits a NULL `os_family`.
6. `set_updated_at()` exists and is the trigger function used for every table with `updated_at` (`0001_init.sql:575-581`).
7. `CREATE UNIQUE INDEX … WHERE kind = 'local'` makes a second `local` row fail with `23505`, and a CHECK violation is `23514`.
8. `TargetId::from_uuid` and `Uuid::from_u128` are both `const fn`, so `LOCAL_TARGET_ID` can be a `const` (`model/ids.rs:36-39`).
9. `ArtifactLinkId` is the last entry of `id_newtype!` at `model/ids.rs:136-137` and is re-exported at `model/mod.rs:138`.
10. `str_enum!` declares `OsFamily` at `model/box_.rs:13-22` and is usable from a new `model/target.rs`.
11. `BoxRow` is at `model/box_.rs:28` with 20 fields; its literals outside the file are `fixtures.rs:508`, `conformance.rs:14036`, `:14145`, `pg/write.rs:3517`, `htui/src/provision/verify.rs:188` and `htui/src/ui/tabs/settings/boxes.rs:1481`.
12. `BoxInfo.os_family` (`box_.rs:307`) and `BoxProfile.os_family` (`box_.rs:370`) are `OsFamily`; `BoxProfile::project` copies `row.os_family` (`:413`).
13. `prompt/render.rs:416` `box_profile` renders the OS family, and its test `the_box_profile_omits_what_the_row_does_not_have_and_never_a_path` is at `:1577`.
14. `NewBoxRow::insert` (`pg/mod.rs:827-857`) inserts `os_version ''` and `htui_version = HTUI_VERSION`, and `register_box` is an inherent `PgStore` method (`pg/mod.rs:634`), not a trait method.
15. No `delete_box` writer exists; `DELETE FROM box` appears only in tests, `testkit.rs:332` and `cache/refresh.rs:680`.
16. `refresh_box` mirrors only this box: `WHERE id = $1`, then `DELETE FROM box WHERE id <> ?` (`cache/refresh.rs:632`, `:675-681`).
17. The mirror's `box.os_family` is `TEXT NOT NULL` (`cache_migrations/0001_mirror.sql:37-45`).
18. The mirror has no `step_graph` table; `project` is at `0001_mirror.sql:57`, `item` at `:75` and `run` at `:103`.
19. `MIRRORED_TABLES` is `[&str; 23]` at `cache/mod.rs:51`; its length is pinned at `tests/it/cache.rs:3150`; `it__connection__confirm.snap:11` reads "the 23 mirrored tables".
20. `WORKSPACE_COLUMNS` (`refresh.rs:528`), `replace_workspace` (`:538`), the `run_pass` unscoped block (`:246-250`), `PROJECT_COLUMNS` (`:694`), `ITEM_COLUMNS` (`:860`) and `RUN_COLUMNS` (`:1191`, 19 columns ending `"workspace_id"`).
21. `cache_migrations/0006_run_workspace.sql` adds a column with `ALTER TABLE run ADD COLUMN workspace_id TEXT;`.
22. `cache_0011_applies_over_a_populated_0010_mirror` is at `tests/it/migrations.rs:7612-7700`, and there is no `cache_0012` test.
23. `tests/it/migrations.rs` pins the applied vec `1..=36` at `:128-152`, declares `TABLES` at `:37` and asserts 57 at `:167`.
24. `Pending(36)` is pinned at `connect.rs:140`, `:241` and `migrations.rs:4270`, `:4371`; `MigrationsPending(36)` at `migrations.rs:4380`, `:4403`; `htui/src/state_cmd.rs:1593`'s `MigrationsPending(3)` is unrelated.
25. The column-comment lists are chained at `migrations.rs:1246-1261`, `:1325-1340` and `:1350-1365`, and table comments are checked at about `:1283-1290`, so a commented `box`/`run` column without a pin fails "and no others".
26. `migration_0034_creates_item_artifact_link` (about `migrations.rs:7861`) is the `bare_db` → `run_to` pattern; `insert_box` is at `:1932` and `insert_run` at `:1956`.
27. `0021_project_type.sql:35-42` seeds rows that `seed.rs:478` `PROJECT_TYPES` mirrors, pinned equal at `migrations.rs:3549`.
28. `CASES` is at `conformance.rs:120` and `run_case_inline` at `:503`; an entry is a name plus an arm, as at `:460` and `:1496-1497`.
29. `EXPECTED_CASES` is 324 at `pg_conformance.rs:70` (doc `:69`, `:77`), and `mem_store.rs:50-51` asserts 324.
30. `applied`/`stale` helpers are at `conformance.rs:5016` and `:5024`, and `constraint_exactly` at `:21459`.
31. `CasOutcome` is at `traits.rs:4484` with `Applied(T)` and `Stale(T)`.
32. `StoreError` (`store/error.rs:10-51`) has `NotFound { entity, id }` and `Constraint(String)` and no conflict variant.
33. `BOX_PROBE_SPEC_KEY` is at `box_.rs:156`, with typed `box_probe_spec`/`set_box_probe_spec` and the case `box_probe_spec_is_cas_on_updated_at` at `conformance.rs:14509` (registered `:227-228`, `:779-784`).
34. `update_project(id, expected: DateTime<Utc>, patch) -> Result<CasOutcome<Project>>` is at `traits.rs:903`.
35. `update_step_graph` is at `traits.rs:1034`, and its Pg UPDATE sets `revision = revision + 1` at `pg/write.rs:4885`.
36. `State::bump_graph` is at `mem.rs:4024`, Pg `bump_graph` at `pg/write.rs:2146`, and `every_graph_write_bumps_revision` at `conformance.rs:28480-28614`.
37. `update_item(id, expected_version: i32, patch: ItemPatch)` is at `traits.rs:420` (Mem `mem.rs:11227`, Pg `pg/write.rs:2766`).
38. `Item` is at `item.rs:157` (`step_graph_id` `:183`), `NewItem` at `:294` and `ItemPatch` at `:322` (`step_graph_id: Option<Option<StepGraphId>>` `:337`).
39. `ItemSpec` is at `item_spec.rs:69` (`step_graph_id` `:83`); `SpecChanges` is at `:89` with a manual `Debug` at `:117`/`:133`; `between` is at `:462`; `into_patch` at `:487`.
40. `SpecField` is at `item_merge.rs:16`, with seven variants and `ALL: [Self; 7]` at `:35`; `Graph` is handled at `:142`, `:175` and `:233`.
41. `divergence.rs:430-441` `Labels::value` matches every `SpecField` exhaustively.
42. `item_form.rs` `parsed` (about `:883`) builds the `ItemSpec` from `base = self.context.item.as_ref().map(ItemSpec::of)`.
43. `item_revision` (`0001_init.sql:340-351`) does not store `step_graph_id`, so it needs no `target_id`.
44. `Project` is at `hierarchy.rs:59`, with 20 literal sites outside the file; `StepGraph` is at `kind.rs:448` with `revision: i64` at `:462` and 23 literal sites.
45. `Project` is read by `query_as!` at `pg/read.rs:729` and `pg/write.rs:4189`, `:4268`; `StepGraphRow` at `pg/read.rs:4044` is read at `pg/read.rs:3291`, `:3329`, `:3664` and `pg/write.rs:4836`, `:4880`; `Item` at `pg/read.rs:191`, `:1510` and `pg/write.rs:2684`, `:2773`; `BoxRow` at `pg/read.rs:2024`, `:2294` and `pg/write.rs:3593`.
46. The `ReadStore` impls are MemStore, PgStore, Writer, CacheStore, Backend, UsageSpy and SpyStore; the `WriteStore` impls are MemStore, PgStore, Writer, UsageSpy and SpyStore (Backend has none).
47. A workspace's scope row is a `project` row with `owner_workspace_id` set and slug `@<workspace slug>` (`0026_workspace_scope.sql:53-59`).
48. `pg_criteria.rs` `every_cascade_table_loses_exactly_what_the_report_names` (`const TABLES: [&str; 33]`, about `:3447-3484`) lists tables that lose rows on a project delete, and a `target` table referenced *from* `project` adds none.
49. On Postgres 16 an `ON DELETE RESTRICT` violation is `23503`, and on 18 it is `23001` (AGENTS.md).
50. `docs/REQUIREMENTS.md:892-902` holds `R-NF-2` with the sentence "except the code-context tools in coding sessions (R-LATER-7)" and an amendment trail ending "and on 2026-10-20 (one Qdrant for the whole database)."
51. `docs/hr-sandbox.md:227-242` is the `cargo sqlx prepare` recipe against `localhost:5439`, and `:197` exports `HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres`.
52. The `box` mirror refresh, `boxes()` and `box_info` keep working when `box` gains columns with defaults, because `pg/demo.rs:67`, `testkit.rs:694` and `pg/mod.rs:839` name their INSERT columns.

## Verified claims (fact-check, 2026-11-14)

Three read-only checkers, claims 1-18 / 19-35 / 36-52; Postgres behaviour probed on the sandbox's PG 18.7.

| claim | verdict | evidence |
|---|---|---|
| 1-2 | TRUE | `ls` of `migrations/` ends `0036_activity_event.sql`, `cache_migrations/` ends `0012_requirement_keys.sql` |
| 3 | TRUE | `0001_init.sql:57`; probe: `pg_constraint` lists `box_os_family_check` (PG 18 also names `box_os_family_not_null`) |
| 4 | TRUE | only `0005:15-20` alters `box`; `0032:42-44` adds a `BEFORE DELETE ON box` trigger (not an ALTER) |
| 5 | TRUE | probe: nullable column with `CHECK (x IN (…))` accepts NULL; the real column is also NOT NULL, which T1 drops |
| 6 | TRUE | function `0001:21-25`; loop `:574-580`; later tables add the trigger explicitly |
| 7 | TRUE | probe: partial unique index → `23505`; CHECK → `23514` |
| 8 | TRUE | `ids.rs:37` `const fn from_uuid`; uuid 1.26.0 `builder.rs:206` `const fn from_u128` |
| 9 | TRUE | `ids.rs:137`, `model/mod.rs:138` |
| 10 | TRUE (note) | `box_.rs:13-23`; macro at `model/mod.rs:25` is textually scoped, so `mod target;` goes below it (amended into T1) |
| 11 | TRUE | 20 fields; six outside literals as listed; four in-file test literals `:445`, `:489`, `:503`, `:573` |
| 12-13 | TRUE | `box_.rs:308`, `:371`, `:413`; `render.rs:405`/`:416`, test fn `:1574` |
| 14-18 | TRUE | `pg/mod.rs:634`, `:836-857`; no `delete_box`; `refresh.rs:632`/`:680`; `0001_mirror.sql:39`, `:57`, `:75`, `:103` |
| 19 | TRUE | `cache/mod.rs:51`; `cache.rs:3148-3150`; snapshot is in the `htui` crate: `crates/htui/tests/it/snapshots/it__connection__confirm.snap:11` |
| 20-23 | TRUE | `refresh.rs` lines match (`run_pass` block `:247-251`); `0006:10`; no `cache_0012` test; `migrations.rs:37`, `:167-168` |
| 24 | FALSE → amended | six pins right; two more: `connect.rs:154-155` (`pending, 36`) and `migrations.rs:4554-4555` (`_sqlx_migrations` = 36), plus the "thirty-six" texts; amended into T1 |
| 25 | TRUE | `:1246-1261`, `:1325-1340`, `:1350-1365`; table comments `:1284-1294` |
| 26 | TRUE (lines) | fn `:7862`; `insert_box` `:1929`; `insert_run` `:1946` (amended) |
| 27-29 | TRUE | `0021:35-42`, `seed.rs:478`, `migrations.rs:3549`; `CASES` counted at exactly 324 |
| 30 | PARTIAL → amended | `constraint_exactly` is at `conformance.rs:23301` (amended in T2) |
| 31-35 | TRUE | `traits.rs:4484`, `error.rs:10-51` (no conflict variant), `box_.rs:156`, `traits.rs:903`, `:1034`, `write.rs:4885` |
| 36-43 | TRUE | sites as cited; `between` defined `:450`, `into_patch` `:474`; `Labels::value` exhaustive; `item_revision` has no graph column |
| 44 | FALSE → amended | `Project`: 8 literal sites (not 20); `StepGraph`: 20 sites, 17 need the field (not 23); amended in T4 |
| 45-51 | TRUE | `query_as!` lists complete; impl lists as cited; `0026:53-60`; `pg_criteria.rs:3448-3482` unchanged by `project → target`; `AGENTS.md:79`; `REQUIREMENTS.md:892-903`; `hr-sandbox.md:227-241` |
| 52 | PARTIAL | conclusion holds, reason corrected: readers name their columns (`cache/refresh.rs:641-644`, `pg/read.rs:2026`/`:2296`, `pg/write.rs:3604` RETURNING), so defaulted new `box` columns break none |

Task independence: T1-T5 share the store traits, both stores, conformance, spies and `.sqlx`, so they run serially in one lane;
T6 touches only `docs/REQUIREMENTS.md`, which no other task lists (intersection empty).

## Acceptance

- [ ] T1-T6 complete, tests written before code, each task's check green
- [ ] Validation passes in the sandbox; the host Postgres/Qdrant run noted as owed
- [ ] PRD M1 row → complete; M2 planned next
