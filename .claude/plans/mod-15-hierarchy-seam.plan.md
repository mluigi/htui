# Plan: MOD-15 milestone 1 — the seam can write the hierarchy

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 1 only. Design
authority: the PRD's D1–D13 (cited below as **PRD Dn**; this plan's own decisions are plain
**Dn**), `docs/ANA-9.md` §4.1/§5.10/§6.1, `docs/ANA-5.md` §4.4/§4.6, `docs/ANA-2.md` §4.7.
**Requirements**: `R-ENT-1..4`, `R-ENT-6`, `R-ENT-10`, `R-BOX-4`, `R-PRM-3`; `R-NF-3` by
construction (no UI in this milestone).
**Complexity**: Large (31 trait methods on four implementations, 12 conformance cases, one
registry).
**Routing**: routed as **PRD** by `/handoff-run MOD-15` (criteria C2, C3, C4 fired). Ultracode
recommended for the implement and review phases. Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`). Models per maintainer: plan and reviewer on Fable, every
implementer on Opus.

## Summary
Grow `WriteStore` (`crates/htui-core/src/store/traits.rs:135-295`) from eleven item/agent/run
methods to the hierarchy: create/update for `workspace`, `project`, `repo`, `item_kind`,
`step_graph`, `step_graph_phase`; upsert/remove for `workspace_project`; upsert for the two
per-box path tables; delete for `workspace` and `project` (cascade, with the reach counted before
and reported after, PRD D13) and for `item_kind` (refused while referenced, PRD D6); one
compare-and-set outcome type shared by every edit (`R-ENT-10`); and the two rung-aware settings
writers of PRD D7/D8 over a typed `SettingKey` registry that validates with the reader's own rules
and refuses rather than clamps. Readers for the six tables nothing reads today land on the same
trait so the conformance suite can read back what it wrote. `MemStore` is the reference
implementation, `PgStore` the product one, `Writer` delegates, `BufferedWriter` refuses with the
constant MOD-25 already coined. No UI, no seed, no migration, no `.sqlx` surprises: every new
Postgres query is prepared and committed in the same task that writes it.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **Every new method — writers and readers — goes on `WriteStore`**, not on `ReadStore` and not inherent. `ReadStore` stays ANA-9 §6.1 verbatim (the reason `mem.rs:195-199` gives for `workspaces()` being inherent); `WriteStore`'s own trailing comment `// links, notes, templates, box ...` (`traits.rs:294`) reserves it for growth. The single-row `item_kind(id)` and `app_settings()` inherent reads are left untouched. | PRD milestone 1 says "readable and writable through `WriteStore`". The conformance suite is generic over `S: WriteStore` (`conformance.rs:59-92`) and cannot call an inherent read, so a reader that is not on the trait cannot be asserted on both stores. Five of the six tables read here (`repo_box_path`, `workspace_box_path`, `step_graph`, `step_graph_phase`; `repo` is mirrored but has no mirror reader) are server-only, which is the same fact that keeps them off `ReadStore`: `CacheStore: ReadStore` could not answer them. |
| D2 | **`Writer` delegates; `BufferedWriter` refuses every new method with `StoreError::Unreachable(DATABASE_UNREACHABLE)`** (`writer.rs:449`, "this box browses its read-only cache and starts no run"). No real buffered implementation, no new constant. | `htui` is online-only since MOD-25 and CLEAN-2 deletes `BufferedWriter`; a real offline hierarchy write would be code written to be removed. `DATABASE_UNREACHABLE` is `R-STO-4`'s sentence — "offline read-only mode … no item creation" — and is the one refusal MOD-25 coined for exactly "a write arrived while offline". `REGISTRY_ON_SERVER_ONLY` (`:415`) and `PROMPT_ON_SERVER_ONLY` (`:433`) name other subsystems and would mislead. The same constant serves the new *reads* on `BufferedWriter`: the tables are not mirrored (`cache/mod.rs` `MIRRORED_TABLES`), so offline they are unreachable in the literal sense. |
| D3 | **Compare-and-set on every edit, with one outcome type**: `enum CasOutcome<T> { Applied(T), Stale(T) }` in `store/traits.rs` beside `UpdateOutcome` (`:322-332`). Every `update_*` and `set_setting`/`clear_setting` takes `expected: DateTime<Utc>` (the row's `updated_at` the caller edited from) and returns `CasOutcome<Row>`; `Stale` carries the row as it is now so the editor can reload and retry (PRD D8's accepted cost). `WHERE id = $1 AND updated_at = $2`; `rows_affected() == 0` then re-`SELECT` to tell `Stale` from `NotFound`. | PRD D8 (`updated_at` is the token; the trigger uses `clock_timestamp()`), PRD constraint "nothing here is last-writer-wins" (`R-ENT-10`). `UpdateOutcome` is typed to `Item`/`ItemRevision` and cannot be reused. `workspace_project` has no `updated_at` (`0001_init.sql:161`, not in the trigger loop) and the two path tables are per-box rows with one writer each, so those three are plain upserts, documented as such. |
| D4 | **Deletes report their reach, and the reach is read before the act by a separate method**: `delete_reach(DeleteTarget) -> Result<Option<DeleteReach>>` (`DeleteTarget::{Workspace(WorkspaceId), Project(ProjectId)}`) and `delete_workspace(id) -> Result<DeleteReach>`, `delete_project(id) -> Result<DeleteReach>`. `DeleteReach` is one struct of `u64` counts: `workspace_links, workspace_box_paths, items, item_key_counters, item_kinds, step_graphs, phases, phase_agents, prompt_templates, repos, repo_box_paths, skill_bindings, runs, run_steps, session_events, run_step_commits, notes, revisions, links, documents` (`phase_agents` added by the fact-check, F3: `phase_agent.phase_id` cascades from `step_graph_phase`, `0001_init.sql:255`). `PgStore` counts in one statement of scalar subqueries, deletes in a transaction, and returns the counts it took; `MemStore` removes from every map. The database's cascade does the deleting on Postgres; `MemStore` mirrors the cascade list of PRD D13. | PRD D13: counts shown before the act, and "the counts shown match what the cascade removes" is a success metric — so the conformance case asserts `delete_reach == delete_*`'s report. A workspace delete reaches only `workspace_project` and `workspace_box_path` (`0001_init.sql:162,174`); projects survive it. |
| D5 | **The mirror rebuild after a project delete belongs to the caller, not the seam.** `delete_project` returns; the store worker that owns `Backend::Online { pg, cache }` calls `CacheStore::rebuild()` (`cache/mod.rs:176-195`) afterwards. That is milestone 3's `StoreRequest` handler, with PRD D10's `Rebuild cache` action. | `PgStore` and `Writer::Online(PgStore)` hold no `CacheStore`; `MemStore` has no mirror at all; a `WriteStore` method that rebuilt a cache would need a handle only one of four implementors could have. PRD D13 already words it as "the delete path triggers the rebuild D10 builds" — D10 is the connection section, i.e. UI. Recorded here so milestone 3 does not re-decide it. |
| D6 | **`item_kind` delete refuses with `StoreError::Constraint` naming the holder**, checked by the seam before the statement: `Constraint("item_kind FEAT is held by 4 items")`. On Postgres the `item.kind_id` FK (`0001_init.sql:313`, no cascade) would refuse anyway with `23503`, but the text would name a constraint, not a count. `item_key_counter` is keyed by prefix, not kind, and is never touched. | PRD D6 ("names what holds it"). `Constraint` is the existing class for "the write violates a rule of the data" (`htui-store/src/error.rs`); no new variant. |
| D7 | **The settings registry is `SettingKey` (ten variants) plus a `const SPECS: [SettingSpec; 10]`** in `htui_core::prompt::settings`, indexed by `SettingKey as usize`. `SettingSpec { key: &'static str, min: i64, max: i64, rungs: Rungs, not_above: Option<SettingKey>, unit: &'static str, doc: &'static str }`. `Rungs` is a three-flag bitset (`APP \| PROJECT \| PHASE`): all ten accept `App`; `token_budget` and `upstream_hops` accept `Project` (ANA-5 §4.4's project keys); `token_budget` alone accepts `Phase` (the column). Ranges are lifted from the reader: `upstream_hops` `HOPS_RANGE` (`settings.rs:50`), `prompt_reserve_fraction_bp` `0..=MAX_RESERVE_BP` (`:53`), every `positive_*` key `1..=<its integer width's max>` — **except the two `u64` keys, whose max is `i64::MAX`, not `u64::MAX`** (fact-check F2: `resolve_excerpt_caps` reads `excerpt_max_file_bytes` and `excerpt_provider_deadline_ms` as `positive_i64(..).and_then(|v| u64::try_from(v).ok())`, so any value above `i64::MAX` is silently dropped by the reader, which is exactly what this decision exists to prevent) — and `excerpt_head_lines.not_above = Some(ExcerptFileLineCap)` because `resolve_excerpt_caps` (`:308`) clamps `head_lines` to `file_line_cap`. `DEFAULTS` stays the typed struct the resolvers read; `Defaults::as_rows()` (`:84`) is rewritten to iterate `SettingKey::ALL`, so the key list has one home, and a unit test asserts every default validates under its own spec. The `0002` pin tests on both sides stay byte-for-byte. | PRD D7. Validation "with the reader's own rules": the ranges are the clamps, quoted by constant where one exists. `not_above` is the one cross-key rule the reader applies; refusing it at write time is what "no value can be written that the read half would clamp" costs. The two MOD-6 cache keys (`cache_refresh_seconds`, `cache_overlap_seconds`, `pg/mod.rs:47-50`) are **not** in the registry: they are not among the ten the PRD exposes, their reader is `connect.rs:292-302`'s `> 0` rule, and the `0002` pin is asserted against exactly ten rows. Adding them is one variant each when MOD-12 or the connection section asks. |
| D8 | **Two writers and one reader over `SettingRung`**: `set_setting(rung, key, value: Value, expected: Option<DateTime<Utc>>) -> CasOutcome<StoredSetting>`, `clear_setting(rung, key, expected) -> CasOutcome<StoredSetting>`, `setting(rung, key) -> Option<StoredSetting>`; `enum SettingRung { App, Project(ProjectId), Phase(PhaseId) }`; `StoredSetting { value: Option<Value>, updated_at: DateTime<Utc> }`. `expected: None` on the `App` rung means "I expect no row" — the insert path after a `clear` — and is `Stale` if a row exists; `Some(ts)` is the CAS. `Project` and `Phase` rows always exist, so `None` there is `NotFound`-shaped misuse and refuses as `Constraint`. Per rung: `App` is `INSERT`/`UPDATE`/`DELETE` on `app_setting`; `Project` is `UPDATE project SET settings = settings \|\| jsonb_build_object($k, $v)` and `settings - $k` for clear (`serde_json::Map::insert`/`remove` on `MemStore`), CAS on `project.updated_at`; `Phase` is `UPDATE step_graph_phase SET token_budget = $v` / `NULL`, CAS on the phase's `updated_at`. Validation order: key accepted on rung, JSON is an integer, in `[min, max]`, `not_above` against the same rung's current value of the other key (or its default). Every refusal is `Constraint` with the key, the value and the rule in the text. | PRD D7 (typed key, refuse not clamp, `set`/`clear` distinct, key-level merge) and PRD D8 (CAS on `updated_at`). The PRD writes the phase id as `StepGraphPhaseId`; the newtype in the tree is `PhaseId` (`model/ids.rs:99`) and the plan uses the tree's name. `token_budget` on a phase has **one** writer: `PhasePatch` (D10) deliberately omits the column so the rung is not writable two ways. |
| D9 | **`create_project` lands in milestone 1, unseeded, structured for milestone 2 to seed inside the same transaction.** `PgStore::create_project` opens `pool.begin()`, inserts the row, commits; milestone 2 adds the 35 inserts between insert and commit, and `MemStore` the equivalent under one `write` closure. `NewProject { id, slug, name, description, created_by }`; `settings` is `{}` and `secret_provider`/`secret_scope` are `None` (MOD-10's). `ProjectPatch { slug, name, description }` never carries `settings`. | The PRD draws milestone 2 as "a created project is a working project": creating must exist first. A project that fails to seed must not exist, so the seed has to run inside the create's transaction — hence the shape now. `prompt_template` gets no writer here: it is milestone 2's seed row and MOD-9's editor. |
| D10 | **Patch structs carry `Option<T>` per editable column and nothing else.** `WorkspacePatch { slug, name, description }`; `ProjectPatch` (D9); `RepoPatch { name, remote_url, default_branch, is_primary }`; `ItemKindPatch { prefix, name, description, default_graph_id, position }`; `StepGraphPatch { name, description }`; `PhasePatch { name, position, template_name, gate_hard, input_kinds }` — PRD D2's six minus `token_budget` (D8). `create_phase(&StepGraphPhase)` takes the full row (every `NOT NULL` column has a meaning MOD-4 owns; the seeder and the fixture already build whole rows). `RepoPatch { is_primary: Some(true) }` clears the project's other primary in the same transaction so `uq_repo_primary` (`0001_init.sql:185-200`) is never tripped; `Some(false)` just unsets. | Same shape as `ItemPatch`. A patch that could carry a column two writers own is the hole D8 closes. |
| D11 | **Rules the schema cannot express are checked in `htui-core` once and called by both stores**: `ItemKind::prefix_is_valid` mirrors the CHECK `^[A-Z][A-Z0-9]{1,15}$` (`0001_init.sql:282-290`) without a regex crate (first byte `A-Z`, 2..=16 bytes, rest `A-Z0-9`); `default_graph_id` must belong to the kind's project (the FK is to `step_graph(id)` alone, `:288`) — `Constraint`, in the shape of `pg/write.rs`'s `kind_not_in_project()`; a phase named `judge` or `handoff` is refused via `TemplateRole::of_name(name) != TemplateRole::Phase` (`prompt/template.rs:49-57`). Slug/name uniqueness, `(graph, position)` and `(graph, name)` uniqueness are left to the database on Postgres (`23505` → `Constraint` through `map_sqlx`) and mirrored by a scan on `MemStore`. | The reserved-name refusal is `docs/ANA-5.md:1238`'s and is validation on write, which is this milestone's theme; a phase writer that accepted `judge` for one milestone would be a hole milestone 2 has to remember. Milestone 2's outcome row is unchanged — its seeder drives this check through `create_project`. |
| D12 | **Twelve conformance cases, one per entity group, `EXPECTED_CASES` 23 → 35; no `READ_CASES` change.** The Postgres runner creates and drops a database per case (`pg_conformance.rs:31-42`), so a case per *method* (31) is refused on cost and a case per entity group is the budget: each case exercises create, read-back, the constraint refusals, the CAS miss and `NotFound` for its group. Column-level facts §6.1 cannot read back stay per backend in `mem.rs` and `pg_criteria.rs`, named from the case's doc comment so `every_cross_referenced_test_name_exists` (`conformance.rs:2119`) checks them. | PRD Evidence: "conformance is the real cost of a seam method". `READ_CASES` is the mirror's suite and none of these tables has a mirror reader. |

### Blueprint amendments (2026-09-16)

`.claude/plans/mod-15-hierarchy-seam.blueprint.md` §0 found eleven issues while elaborating these
decisions. Five change what D7/D8 above say; the blueprint's version wins, and the reasons are
recorded there as flags A–K. The two that matter most, because they are factual errors in this
plan rather than refinements:

- **A — one `key` cannot serve two rungs.** The App key is `prompt_upstream_hops`; the key
  `resolve_hops` reads out of `project.settings` is `upstream_hops`. As written, D8's
  `set_setting(Project, UpstreamHops)` would have written a key the reader ignores — the exact
  silent-ineffectiveness PRD D7 exists to prevent. `SettingSpec` gains `project_key`.
- **B — `prompt_reserve_fraction` is a float, not an integer.** D8's "JSON is an integer" and case
  10's `prompt_reserve_fraction_bp = 5001` describe a key and a type that do not exist: the stored
  value is `0.10`, read through `as_f64` and rounded to basis points. PRD D7 listed `kind` in the
  spec and this plan dropped it; it comes back as `SettingKind { Integer, Fraction }`.

Also: **C** the `Phase` rung caps at `i32::MAX` (the column is `INTEGER`); **D** `clear_setting`
takes a non-optional `expected`; **H** `MemStore`'s `app_settings` map is retyped to carry a CAS
token; **I** the writer count is 21, not 22 (`delete_reach` was double-counted); **E** T1's case
docs must not name the `pg_criteria.rs` twins T2 has yet to write, or
`every_cross_referenced_test_name_exists` fails T1.

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Naming | `New*` / `*Patch` request types beside their row; `upsert_*` for PK-keyed rows; case names as sentences | `crates/htui-core/src/model/item.rs` (`NewItem`, `ItemPatch`); `traits.rs:135-295`; `conformance.rs:24-48` |
| Errors | `StoreError::{NotFound{entity,id}, Constraint(String)}`; `map_sqlx` (`23xxx` → `Constraint`); seam-side rule refusals as `Constraint` with a helper fn | `crates/htui-core/src/store/error.rs`; `crates/htui-store/src/error.rs:17-52`; `crates/htui-store/src/pg/write.rs` `kind_not_in_project()` |
| Refusals | One constant, reused by name, never a second sentence for the same fact | `crates/htui-store/src/writer.rs:415-419`, `:449` |
| Logging | None in the seam; the store never logs a write | `crates/htui-store/src/pg/write.rs` (no `tracing` calls) |
| Data access (Pg) | Single-statement `query!`/`query_as!` with `AS "col: Type"` overrides, `.map_err(map_sqlx)`, `rows_affected() == 0` → `NotFound`, `pool.begin()` only for multi-row writes, `RETURNING` sees the trigger's `updated_at`, **never `SET updated_at`** | `crates/htui-store/src/pg/write.rs:39-…`; `pg/read.rs:1088-1110` (`item_kind`) |
| Data access (Mem) | `self.read(\|state\| …)` / `self.write(\|state\| …)`, no `.await` under the lock, `State` maps keyed by id | `crates/htui-core/src/store/mem.rs:397-410`, `:1406-…` |
| Data access (JSONB) | Read the whole `settings` document, never a typed round-trip | `mem.rs:296-299` (`project_settings`), `pg/read.rs` `project_settings` |
| Tests | Case fn `async fn name<S: WriteStore>(store: &S)` with the case name in every message; `CASES` + `run_case` arm + count pins; per-backend twins named in the doc comment | `conformance.rs:59-92`, `:1534-1724`; `crates/htui-core/tests/mem_store.rs:36`; `crates/htui-store/tests/pg_conformance.rs:19`; `crates/htui-store/tests/pg_criteria.rs:1-60` |
| Settings reader | `DEFAULTS`/`as_rows()`, the `0002` pin from both sides | `crates/htui-core/src/prompt/settings.rs:36-90`, `the_defaults_are_migration_0002s_ten_rows_verbatim`; `crates/htui-store/tests/migrations.rs:268-290` |
| Worker seam | Not touched this milestone; `Writer` is the handle the store worker writes through and gains plain delegation arms only | `crates/htui-store/src/writer.rs:53-58`, `:547-…` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/hierarchy.rs` | edit | T1 | `NewWorkspace`, `WorkspacePatch`, `NewProject`, `ProjectPatch`, `NewRepo`, `RepoPatch` |
| `crates/htui-core/src/model/kind.rs` | edit | T1 | `NewItemKind`, `ItemKindPatch`, `NewStepGraph`, `StepGraphPatch`, `PhasePatch`, `ItemKind::prefix_is_valid` |
| `crates/htui-core/src/model/mod.rs` | edit | T1 | re-export the new types |
| `crates/htui-core/src/prompt/settings.rs` | edit | T1 | `SettingKey`, `SettingSpec`, `SPECS`, `Rungs`, `validate`; `as_rows()` over `SettingKey::ALL`; unit tests |
| `crates/htui-core/src/store/traits.rs` | edit | T1 | 31 `WriteStore` methods; `CasOutcome`, `DeleteTarget`, `DeleteReach`, `SettingRung`, `StoredSetting` |
| `crates/htui-core/src/store/mod.rs` | edit | T1 | re-export the new seam types |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | `State` gains `repos`, `repo_box_paths`, `workspace_box_paths`; drop the two `#[expect(dead_code)]` (`:61`, `:64`); 31 impls; unit twins |
| `crates/htui-core/src/store/conformance.rs` | edit | T1 | 12 cases, `CASES`, `run_case` arms, helpers |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | pin `CASES.len()` 23 → 35 |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | 22 writers + `delete_reach` |
| `crates/htui-store/src/pg/read.rs` | edit | T2 | 9 trait readers (beside the inherent ones) |
| `crates/htui-store/.sqlx/query-*.json` | add | T2 | offline data for every new query (`cargo sqlx prepare`) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 23 → 35 |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T2 | Postgres-only twins: CAS tokens advance by trigger, whole-row diff for `set_setting(Project)` |
| `crates/htui-store/src/writer.rs` | edit | T3 | `Writer` delegation arms; `BufferedWriter` refusals (D2) |
| `crates/htui-store/tests/writer_buffered.rs` | edit | T3 | every new method on `BufferedWriter` answers `Unreachable(DATABASE_UNREACHABLE)` |

Not touched, on purpose: `crates/htui-store/src/backend.rs` (no UI reads this milestone), `cache/**`
(D5), `migrations/**` (none), `pg/demo.rs`, `fixtures.rs`, anything under `crates/htui`.

## Tasks

**T1 → T2 → T3, fully serial. The parallel marking this plan first carried was stripped by the
fact-check (F1).** The file sets of T2 and T3 are disjoint, but disjoint files are not the test:
both tasks live in crate `htui-store`, and `WriteStore` gains 31 methods with no default bodies,
so `htui-store` does not compile until `PgStore` implements them. T3's own gate
(`cargo test -p htui-store --test writer_buffered`) therefore cannot go green before T2 has
landed. Running them concurrently would mean one agent watching the other's compile errors.

Consequence for the workspace gate: `cargo test --workspace` is red from the first commit of T1
until T3 lands, because `htui-core` compiles alone but `htui-store` does not. Each task's own
crate-scoped gate is the live signal in the meantime; the workspace gate is the acceptance gate.

Three serial tasks are not a workflow: **ultracode is not used for this milestone** despite the
routing recommendation, since there is nothing to fan out. The recommendation stands for later
milestones. TDD per task: tests first, red, then code.

Every implementer prompt carries: the PRD's D1–D13 win over this plan where they disagree;
graphify-first for codebase questions; `.sqlx` regenerated and committed with any query change;
nothing sets `updated_at` by hand; no new refusal constant; no new migration.

### Task 1: types, registry, trait, `MemStore`, conformance (serial)
- **Action**: write the 12 cases first (below), add their names to `CASES` and arms to
  `run_case`, bump `mem_store.rs:36` to 35; they fail to compile. Then: the request/patch types
  (D9, D10) and `ItemKind::prefix_is_valid` (D11) in `model/`; `CasOutcome`, `DeleteTarget`,
  `DeleteReach`, `SettingRung`, `StoredSetting` and the 31 trait methods with doc comments in
  `traits.rs` — each writer's doc names its refusals and its CAS token, each reader its order
  (`repos` by `name` bytes, `item_kinds` and `phases` by `position`, `step_graphs` by `name`
  bytes, `workspace_projects` by `position`); the registry in `prompt/settings.rs` (D7) with unit
  tests `every_default_validates_under_its_own_spec`, `ranges_are_the_readers_clamps` (asserts
  against `HOPS_RANGE`, `MAX_RESERVE_BP`), and the untouched `0002` pin; `MemStore`: three new
  `State` vectors (`repos: HashMap<RepoId, Repo>`, `repo_box_paths: Vec<RepoBoxPath>`,
  `workspace_box_paths: Vec<WorkspaceBoxPath>`, loaded empty by `from_demo`), remove the two
  `#[expect(dead_code)]`, 31 impls under `read`/`write` closures, cascade removal for
  `delete_project` over every `State` map PRD D13 lists. Per-backend twins in `mem.rs`:
  `set_setting_project_rung_leaves_unknown_keys_byte_identical` (seeds a blob with foreign keys
  through `write`, then asserts), `delete_project_leaves_no_row_in_any_map`.
  The 12 cases (names final; each asserts `NotFound` for an unknown id and `Stale` for a spent
  token where the group has a CAS):
  1. `workspace_round_trip_and_cas` — create; duplicate slug → `Constraint`; `workspace(id)`;
     update `Applied` then `Stale` on the old token carrying the current row.
  2. `workspace_links_and_box_paths_upsert` — `upsert_workspace_project` link + reposition;
     `workspace_projects` order; `remove_workspace_project`; `upsert_workspace_box_path` twice
     replaces; `workspace_box_paths`.
  3. `workspace_delete_reports_its_reach` — `delete_reach(Workspace)` equals
     `delete_workspace`'s report (links, box paths); `project(id)` still `Some`; second delete
     `NotFound`.
  4. `project_create_update_cas` — create with `settings == {}`; duplicate slug; update name
     `Applied`/`Stale`; `project(id)` read-back; `settings` untouched by `update_project`.
  5. `project_delete_takes_everything_and_says_so` — on `PROJECT_HTUI`: reach equals report;
     exact `5/15/5/10` for kinds/phases/graphs/templates, non-zero for items, runs, steps,
     events, documents, notes, revisions, links, workspace links; afterwards `project(id)` is
     `None`, the cross-project link from `AGY_FEAT_1` is gone, `PROJECT_AGY`'s items unchanged.
  6. `repo_round_trip_and_primary_flag` — two repos; `(project, name)` duplicate → `Constraint`;
     `repos` order; `is_primary: Some(true)` on the second clears the first (no `uq_repo_primary`
     trip); CAS; `upsert_repo_box_path` twice; `repo_box_paths`.
  7. `item_kind_round_trip_and_prefix_rules` — bad prefix (`feat`, `1A`, 17 chars) →
     `Constraint` on both stores; duplicate `(project, prefix)`; `default_graph_id` from another
     project → `Constraint`; rename `ANA` → `ANL`: `item(HTUI_ANA_2).key` still `ANA-2`, next
     `mint_item` under the kind is `ANL-1` (PRD D12); CAS; `item_kinds` order.
  8. `item_kind_delete_refused_while_referenced` — `KIND_HTUI_ANA` → `Constraint` whose text
     carries the item count; a fresh unreferenced kind deletes; deleting it again `NotFound`.
  9. `step_graph_and_phase_round_trip` — create graph; duplicate `(project, name)`; create
     phase; duplicate position and duplicate name → `Constraint`; `judge`/`handoff` →
     `Constraint`; `update_phase` of the five columns; CAS; `step_graphs`, `phases` order.
  10. `settings_app_rung_validates_and_cas` — read token via `setting(App, TokenBudget)` (row
      present on Postgres from `0002`, absent on `MemStore`; the case handles both by reading
      first); set `Applied`; `0`, a string, `upstream_hops = 3`, `prompt_reserve_fraction_bp =
      5001`, `excerpt_head_lines` above the current `excerpt_file_line_cap` → `Constraint`;
      stale token → `Stale`; `clear` → `setting` is `None`; set with `expected: None` →
      `Applied`; set with `expected: None` while the row exists → `Stale`.
  11. `settings_project_rung_merges_keys` — on `PROJECT_HTUI`: set `UpstreamHops`; every other
      key of `project(id).settings` byte-identical to before; `clear` removes only that key;
      `ExcerptMaxFiles` on `Project` → `Constraint`; CAS on `project.updated_at`.
  12. `settings_phase_rung_writes_token_budget_only` — on `PHASE_HTUI_IMPLEMENT`: set,
      `phases(GRAPH_HTUI_FEAT)` shows it, `setting(Phase)` reads it back; `clear` → `NULL`;
      `UpstreamHops` on `Phase` → `Constraint`; unknown phase → `NotFound`; CAS.
- **Mirror**: `conformance.rs:1534-1724` (`set_step_prompt_writes_digest_and_trim`) for case
  shape; `mem.rs:1406-…` for impl shape; `settings.rs:36-90` for the registry's neighbour.
- **Validate**: `cargo test -p htui-core --all-features`;
  `cargo clippy -p htui-core --all-targets --all-features -- -D warnings`; the `0002` pin still
  green; `every_cross_referenced_test_name_exists` green.

### Task 2: `PgStore` (after T1; T3 waits on this)
- **Action**: bump `EXPECTED_CASES` to 35 and run `pg_conformance` red. Then `pg/write.rs`:
  the 22 writers and `delete_reach` — `INSERT … RETURNING` into the model type; `UPDATE … WHERE
  id = $1 AND updated_at = $2 … RETURNING`, a follow-up `SELECT` on zero rows to split
  `Stale`/`NotFound`; `delete_project` as `begin` → count statement → `DELETE FROM project` →
  `commit`, returning the counts; `set_setting(Project)` as `settings || jsonb_build_object($2::text, $3::jsonb)`,
  `clear` as `settings - $2`; `App` rung `INSERT … ON CONFLICT DO NOTHING` (expected `None`),
  `UPDATE … WHERE key = $1 AND updated_at = $2`, `DELETE … WHERE key = $1 AND updated_at = $2`;
  `RepoPatch.is_primary = Some(true)` as `begin` → `UPDATE repo SET is_primary = false WHERE
  project_id = $p AND is_primary` → the row update → `commit`. `pg/read.rs`: the 9 readers,
  `ORDER BY name COLLATE "C"` where the order is bytes (precedent at `pg/read.rs:881`/`:903`;
  the plan's original `:920` citation was wrong, F4), `ORDER BY position`
  otherwise. Then, with `htui_sqlx` migrated (README `:478-497`):
  `cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features`,
  and commit the new `.sqlx/query-*.json`. `pg_criteria.rs` twins:
  `cas_tokens_advance_by_the_trigger_alone` (two writes, two distinct `updated_at`, neither
  equal to the value the client sent), `set_setting_project_rung_changes_only_settings_and_updated_at`
  (whole-row `to_jsonb` diff, the `set_step_prompt_writes_only_the_digest_and_the_record`
  shape).
- **Mirror**: `pg/write.rs:39-…` (`update_item` CAS, `kind_not_in_project`);
  `pg/read.rs:1088-1110`; `pg_criteria.rs:1-60`.
- **Validate**: `cargo sqlx prepare --check -- --all-targets --all-features` (from the crate);
  `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria`;
  `cargo clippy -p htui-store --all-targets --all-features -- -D warnings` with
  `SQLX_OFFLINE=true` (the default).

### Task 3: `Writer` and `BufferedWriter` (after T2 — F1)
- **Action**: in `tests/writer_buffered.rs`, one test that calls every new method on a
  `BufferedWriter` and asserts `Err(StoreError::Unreachable(s)) if s == DATABASE_UNREACHABLE`;
  red. Then `writer.rs`: 31 three-arm `match self` delegations on `Writer`, 31 one-line
  refusals on `BufferedWriter` through a private `fn hierarchy_needs_the_server() -> StoreError`
  that wraps `DATABASE_UNREACHABLE` (D2; the helper is the shape of
  `registry_writes_need_the_server`, not a new sentence).
- **Mirror**: `writer.rs:225-…` (`BufferedWriter`'s `upsert_agent` refusal), `:547-…`
  (`Writer`'s arms).
- **Validate**: `cargo test -p htui-store --all-features --test writer_buffered`;
  `cargo clippy -p htui-store --all-targets --all-features -- -D warnings`.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
cargo doc --workspace --no-deps
cargo run -p htui -- --demo                                  # unchanged UI; smoke only
```
`cargo sqlx prepare` (not `--check`) needs `htui_sqlx` migrated per README `:486-493`; the
Postgres in `compose.yaml` on port 5439 is the maintenance server the summary's DSN names.

## Verified claims (fact-check, 2026-09-16)
`tree` = grep/read of the repo on this date.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `WriteStore` has 11 methods and four implementors | **true** | `traits.rs:135-295`; impls at `pg/write.rs:39`, `mem.rs:1406`, `writer.rs:225` (Buffered), `:547` (Writer) |
| V2 | No reader exists for `repo`, `repo_box_path`, `workspace_box_path`; `graphs`/`phases` carry `#[expect(dead_code)]` | **true** | `mem.rs:61,64`; `State` fields `:45-104` hold no repo/path vectors; `pg/read.rs` method inventory |
| V3 | `app_setting` has no version column; trigger uses `clock_timestamp()`; `workspace_project` has no `updated_at` | **true** | `0001_init.sql:557`, `:161`; trigger loop excludes `workspace_project` |
| V4 | `PROJECT_HTUI` carries exactly 5 kinds, 5 graphs, 15 phases, 10 templates | **true** | `fixtures.rs:667-700`, `:719-815` (`TEMPLATE_NAMES` len 10, stride comment) |
| V5 | `DATABASE_UNREACHABLE`, `REGISTRY_ON_SERVER_ONLY`, `PROMPT_ON_SERVER_ONLY` texts | **true** | `writer.rs:449`, `:415`, `:433` |
| V6 | `item.kind_id` and `item_kind.default_graph_id` FKs have no cascade; `repo_box_path`, `step_graph_phase`, `workspace_project`, `workspace_box_path` cascade | **true** | `0001_init.sql:313`, `:288`; `:203`, `:230`, `:162`, `:174` |
| V7 | `PhaseId` is the phase newtype (PRD writes `StepGraphPhaseId`) | **true** | `model/ids.rs:99` |
| V8 | `TemplateRole::of_name` is the reserved-name check | **true** | `prompt/template.rs:49-57` |
| V9 | T2 ∩ T3 = ∅ | **true** | T2 = `pg/{write,read}.rs`, `.sqlx/`, `tests/{pg_conformance,pg_criteria}.rs`; T3 = `writer.rs`, `tests/writer_buffered.rs` |
| V10 | `Writer::Buffered` is no longer constructed by product code after MOD-25 | **true, with a nuance** | Constructed only at `agent_worker.rs:3727`, `:3787` (test module); *matched* in product at `:744`, `:1294`, `:1413`, `:1452`, `:1751` — those are CLEAN-2's listed guards. D2 unaffected |
| V11 | `skill_binding.project_id` and `workspace_project.project_id` cascade on project delete | **true, and the full list is now pinned** | `REFERENCES project(id) ON DELETE CASCADE` at `0001_init.sql:163` (workspace_project), `:187` (repo), `:216` (step_graph), `:268` (prompt_template), `:284` (item_kind), `:300` (item_key_counter), `:312` (item), `:435` (skill_binding), `:449` (run); transitively `repo_box_path`, `step_graph_phase`, `phase_agent`, `item_note`, `item_revision`, `item_link`, `document`, `run_step`, `session_event`, `run_step_commit` |
| V12 | Exact accept rule of the two `u64` keys | **FALSIFIED the plan's range rule — amended (F2)** | `settings.rs` `resolve_excerpt_caps`: both go through `positive_i64(..).and_then(\|v\| u64::try_from(v).ok())`, so the ceiling is `i64::MAX`. D7 amended in place |
| V13 | `MemStore` keys its counter by `(project, prefix)` so a renamed prefix mints from 1 | **true** | `mem.rs:86-87`: `item_key_counter: HashMap<(ProjectId, String), i32>`, doc "the highest number minted per `(project, prefix)`" |
| V14 | `PhaseId`, the two `#[expect(dead_code)]`, `HOPS_RANGE = (1, 2)`, `MAX_RESERVE_BP = 5_000`, `UpdateOutcome` at `traits.rs:322`, `EXPECTED_CASES = 23` at `pg_conformance.rs:19`, `every_cross_referenced_test_name_exists` at `conformance.rs:2119`, `impl WriteStore` at `mem.rs:1406` / `writer.rs:225` / `:547` | **all true** | re-grepped 2026-09-16; the `#[expect]` reason string literally names MOD-15 |
| V15 | `COLLATE "C"` precedent is at `pg/read.rs:920` | **FALSIFIED — amended (F4)** | Actual precedent `pg/read.rs:881` (doc) and `:903` (the `ORDER BY`) |
| V16 | `DeleteReach`'s field list is complete | **FALSIFIED — amended (F3)** | `phase_agent` cascades via `step_graph_phase` (`0001_init.sql:255`) and was missing; field added |
| V17 | T2 and T3 may run in parallel | **FALSIFIED — parallel marking stripped (F1)** | File sets are disjoint, but both are in crate `htui-store` and `WriteStore`'s 31 new methods have no default bodies, so the crate does not compile until `PgStore` implements them; T3's gate cannot pass before T2 |
| V18 | `pg_conformance` per-case cost | **not measured** | Twelve extra create/drop cycles; the risk table prices it as Low without a number |

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| 31 methods × 4 impls churns `writer.rs` for a `BufferedWriter` CLEAN-2 deletes | Certain | Low | D2: one-liners through one helper; CLEAN-2 removes them wholesale |
| `Stale` vs `NotFound` mis-split on Postgres (`rows_affected == 0` is both) | Medium | Medium | The follow-up `SELECT` in every CAS writer; every case asserts both outcomes |
| `set_setting` case diverges between stores because `MemStore` seeds no `app_setting` rows | High if the case assumes a row | Low | Case 10 reads the token first and handles `None`; `from_demo` is not changed (plan D101 of MOD-2 depends on it) |
| `delete_project` counts on Postgres drift from what the cascade takes (a table added by `0003`) | Low now, certain later | Medium | Case 5 asserts reach == report; `DeleteReach` is one struct so MOD-4 adds a field, not a method |
| Cross-key `not_above` reads the other key's *current* value and races a concurrent write of it | Low | Low | Both keys are `App` rung; the second write's CAS token was read after the first landed, and the rule is re-checked on every write |
| `.sqlx` left stale after a query edit breaks `SQLX_OFFLINE` builds for everyone | Medium | High | `cargo sqlx prepare --check` in T2's validate and in the Acceptance list |
| Twelve extra create/drop cycles slow `pg_conformance` | Certain | Low | Budgeted in D12; no per-method cases |
| The registry's ranges drift from the reader's clamps in a later edit | Medium | Medium | `ranges_are_the_readers_clamps` asserts against the reader's own constants, so the drift fails a unit test |

## Acceptance
- [ ] `WriteStore` carries the 31 methods of D1–D12 with doc comments naming refusal, order and CAS token; `ReadStore` unchanged
- [ ] `CASES.len() == 35`, `EXPECTED_CASES == 35`, `READ_CASES` unchanged at 6; all 35 pass on `MemStore` and, with the env var, on `PgStore`
- [ ] Every `update_*`, `set_setting`, `clear_setting` returns `CasOutcome` and no statement sets `updated_at` (grep `SET updated_at` in `pg/write.rs` is empty)
- [ ] `delete_reach` equals the delete's report on both stores (cases 3 and 5)
- [ ] `delete_item_kind` on a referenced kind is `Constraint` whose text carries the count (case 8)
- [ ] `SettingKey` has ten variants; `as_rows()` iterates it; both `0002` pin tests unchanged and green
- [ ] Out-of-range, wrong-type, wrong-rung and `not_above` writes are `Constraint`; nothing clamps (case 10)
- [ ] `project.settings` merge leaves every other key byte-identical (case 11 + `mem.rs` twin + `pg_criteria` twin)
- [ ] `BufferedWriter` answers every new method with `Unreachable(DATABASE_UNREACHABLE)`; no new constant in `writer.rs`
- [ ] `cargo sqlx prepare --check -- --all-targets --all-features` passes from `crates/htui-store`; new `.sqlx/query-*.json` committed
- [ ] No file under `migrations/`, `cache/`, `crates/htui/` changed; `unsafe_code = "forbid"`, MSRV 1.98, lint set untouched
- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo doc` clean on the workspace
