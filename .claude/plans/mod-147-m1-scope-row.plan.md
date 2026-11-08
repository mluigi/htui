# Plan: MOD-147 M1 — scope row and owner documents in the store

**Source PRD**: `.claude/prds/mod-147-workspace-scope.prd.md`
**Selected Milestone**: 1 — Scope row and owner documents in the store
**Complexity**: Large (one Postgres migration, one cache migration, core model, MemStore, PgStore, cache, htui hierarchy)
**Routing**: PRD path (C2 + C4); ultracode accepted for implement and review.

## Summary

Every workspace gets one hidden row in `project` (`owner_workspace_id` set, type `workspace`, slug
`@<workspace-slug>`), created in the workspace's own transaction and by the bootstrap for existing
workspaces, seeded with the `WS` kind (plan → review). Documents gain `project_id` on every row and a
nullable `item_id`, so a project or the scope row can own a document. `workspace_box_path.root_path`
becomes `folder_path` with `~/workspaces/<slug>` when absent. The scope row never joins
`workspace_project`, so every existing lister (all of which read through membership, fact 16/17) already
hides it; a slug reservation and a membership refusal make that hold by construction.

## Decisions taken in this plan

| # | Decision | Why |
|---|---|---|
| D1 | `Scope` and `WorkspaceSummary` are **not** changed. A new read `scope_project(workspace) -> Option<ProjectId>` (ReadStore: Pg, Mem, cache) names a workspace's scope row; the store worker appends it to the project list it sends the cache refresher (`store_worker.rs:3037`) | `Scope` has ~101 struct-literal sites across six crates and `WorkspaceSummary` 13 (claims 1-2); `Scope.project_ids` feeds project pickers and group headers, so widening it would leak the scope row. M3's Backlog group reads `scope_project` the same way |
| D2 | Owner documents are a separate Rust type (`OwnerDocument`, `NewOwnerDocument`), written by `write_owner_document`, read by `owner_documents(project)`; `Document`/`DocumentHead` stay item-only | The DB follows ANA-35 D2 (one table, nullable `item_id`), but every existing `Document` reader means an item document. Keeps the 41 `NewDocument {` literals and every `doc.item_id` reader untouched; the four item-document queries gain a `"item_id!"` override (claim 13) |
| D3 | Slugs starting with `@` are reserved for scope rows: DB CHECKs tie `owner_workspace_id IS NOT NULL` ⇔ `type = 'workspace'` ⇔ `slug LIKE '@%'`; `create_project`/`update_project` refuse `@` with a readable Constraint error (Pg and Mem); the migration aborts with a named message if a user project already has an `@` slug | No slug validation exists anywhere today and `update_project` can change a slug (claim 14) |
| D4 | `upsert_workspace_project` (Pg `pg/write.rs:3046`, Mem `mem.rs:3028`) refuses a scope row | The scope row must never join membership; every lister relies on it |
| D5 | The WS kind's graph is project-scoped (owned by the scope row, `GraphScope::Project`), like every seeded graph; phases `plan` (soft gate) → `review` (**hard** gate), reusing the existing `plan` and `review` templates (`prompt/defaults.rs:221,223`); the scope row gets all ten `DEFAULT_TEMPLATES`. `seed_project` takes the kind list as a parameter (today it loops all of `seed::KINDS`, claim 5) | Project scope keeps `delete_workspace`'s held-graph check (graphs with `workspace_id`) untripped; templates reuse settles the PRD open question until MOD-148 adds the workspace `plan` body and narrows templates per type. **Maintainer to confirm the gates** |
| D6 | **Workspace delete cascades its scope row with counts** (amended by the maintainer 2026-11-08 after blueprint V11 showed the refusal could never be lifted: no item delete or move exists): `delete_reach(Workspace)` adds the scope row's full project reach, the person confirms, the scope row and everything it owns go with the workspace (`ON DELETE CASCADE` on Pg, by hand on Mem `mem.rs:5555`); refused only while a scope-row run is live. `delete_project` refuses a scope row ("delete the workspace instead", blueprint V12) | Same model as `delete_project` (preview with counts, then cascade); every FK to `project` cascades (claim 4) |
| D7 | `create_workspace` and `update_workspace` become transactions; a workspace slug rename renames its scope row (`@<new-slug>`) | Both are single pool statements today (claim 3) |
| D8 | `workspace_box_path` stays un-mirrored; `WorkspaceBoxPath.root_path` → `folder_path`; helper `workspace_folder(slug, row, home)` gives `~/workspaces/<slug>` when no row exists; `infer` anchors on the resolved folder | The table has never been mirrored. PRD M1 row amended: the cache mirrors the scope row and owner documents, not the folder |
| D9 | `Project` gains `owner_workspace_id: Option<WorkspaceId>` (5 literals, 3 `query_as!(Project…)`, claim 2); `workspace_of` (`htui/src/hierarchy.rs:537`) resolves a scope row through it | `workspace_of` matches membership today, so it answers NotFound for a scope row (claim 16); M3's secret-binding path needs it |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Migration + backfill test | `crates/htui-store/tests/migrations.rs:2218` (`migration_0021_types_an_existing_project_software`) | `common::bare_db()`, `MIGRATOR.run_to(25)`, plant rows, `MIGRATOR.run`, assert |
| Cache migration | `crates/htui-store/cache_migrations/0007_project_type.sql`; `cache/mod.rs:138-152` | migrations run on the old file first, then a version mismatch deletes and rebuilds it, so a `DROP TABLE document` + `CREATE` must also succeed on a populated old file |
| Project create + seed in one tx | `crates/htui-store/src/pg/write.rs:3147-3181` `create_project` → `seed_project` `:9588-9693` | `pool.begin()`, insert, seed, commit |
| Idempotent bootstrap top-up | `crates/htui-store/src/pg/mod.rs:462` `seed_if_empty_as` | single tx, `LOCK TABLE app_user`, conditional inserts |
| Seed constants | `crates/htui-core/src/seed.rs:56` `KINDS`, `kind_row`, `graph_row` (`GraphScope::Project`, `:197`) | `KindSeed { prefix, name, description, phases }` |
| Document version allocation | `crates/htui-store/src/pg/write.rs:395` `insert_document`, `:7768` `write_document` | owner row `FOR UPDATE`, `COALESCE(MAX(version),0)+1` |
| Delete with refusal | `crates/htui-store/src/pg/write.rs:9898` `delete_workspace_once` | repeatable read, held-graph refusal, retry on serialization failure |
| Conformance case | `crates/htui-core/src/store/conformance.rs:81` `CASES`, `run_case_inline` `:345` | one name in `CASES`, one boxed arm, pins `mem_store.rs:36` and `pg_conformance.rs:51` (233) |
| Errors | `crates/htui-core/src/store/traits.rs` (`invalid_prefix`, `graph_not_usable`) | `StoreError::Constraint` with a person-readable sentence |

## Files to Change (per task; independence is decided by these sets)

| Task | Files |
|---|---|
| T1 schema | CREATE `crates/htui-store/migrations/0026_workspace_scope.sql`, `crates/htui-store/cache_migrations/0009_workspace_scope.sql`; UPDATE `crates/htui-store/tests/migrations.rs` |
| T2 core | UPDATE `crates/htui-core/src/model/{hierarchy.rs,document.rs,mod.rs}`, `crates/htui-core/src/seed.rs`, `crates/htui-core/src/store/{traits.rs,mem.rs,conformance.rs}`, `crates/htui-core/src/fixtures.rs`, `crates/htui-core/src/secret.rs` (Project literal), `crates/htui-orch/src/secrets.rs` (Project literal), `crates/htui-core/tests/mem_store.rs` |
| T3 htui-store | UPDATE `crates/htui-store/src/pg/{write.rs,read.rs,mod.rs,demo.rs}`, `crates/htui-store/src/cache/{refresh.rs,read.rs,mod.rs}`, `crates/htui-store/.sqlx/*`, `crates/htui-store/tests/{pg_conformance.rs,pg_criteria.rs,cache.rs,migrations.rs}` |
| T4 htui | UPDATE `crates/htui/src/hierarchy.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs`, `crates/htui/tests/hierarchy.rs` |

Intersections: T1∩T3 = `tests/migrations.rs` (T3 repins the demo per-table counts at `:4895`); the Pg
and cache halves share `.sqlx` and `refresh_document` (claim 8), so they are one task; T2 changes the
traits every other task implements. T3∩T4 = ∅. **Order: T1 ∥ T2 → T3 ∥ T4.**

## Tasks

### T1: Migration 0026 and cache 0009 (tests first)
- **Action**: tests first in `migrations.rs`: `run_to(25)` → plant a workspace, a project with an item and two documents → `run` → assert `document.project_id` backfilled and `item_id` kept; the CHECKs refuse an `@x` user project, a `workspace`-typed row without `owner_workspace_id`, and a document whose `(item_id, project_id)` disagree; two owner documents of one kind get versions 1 and 2 under the new unique; a planted `@x` user project makes the migration fail with the named message; migration list 1..26; `TABLES` stays 51. Then the DDL (ANA-35 §7.9 P4, widened by D3): `owner_workspace_id UUID UNIQUE REFERENCES workspace ON DELETE CASCADE`, `chk_project_scope_row`, `chk_project_scope_slug`, `uq_item_id_project`, `document.project_id` backfill + NOT NULL, `item_id` nullable, `fk_document_item_project`, `uq_document_id_project`, drop `document_item_id_kind_version_key`, `uq_document_owner_kind_version UNIQUE NULLS NOT DISTINCT (project_id, item_id, kind, version)` (Postgres 16.15, claim 15), index `document(project_id)`, rename `root_path` → `folder_path`; a `DO $$ … RAISE EXCEPTION` precheck for `@` slugs first. Cache 0009: `project.owner_workspace_id`; `document` dropped and recreated with `project_id` and nullable `item_id`, indexes on `item_id` and `project_id`.
- **Validate**: `cargo test -p htui-store --all-features --test migrations -- --test-threads=1`.

### T2: Core model, seed, MemStore, conformance (tests first)
- **Action**: conformance cases first (names in `CASES`, arms in `run_case_inline`; MemStore now, PgStore in T3):
  `create_workspace_creates_its_scope_row`, `scope_row_is_in_no_listing` (`workspaces()`, `projects(scope)`, `workspace_projects`), `scope_project_names_the_scope_row`, `scope_row_cannot_join_a_workspace`, `user_slug_cannot_start_with_at` (create and update), `scope_row_is_seeded_with_the_ws_kind` (one kind `WS`, project-scoped graph plan → review, review hard), `scope_row_mints_ws_keys`, `owner_document_versions_per_owner_and_kind`, `owner_documents_exclude_item_documents`, `item_documents_exclude_owner_documents`, `workspace_rename_renames_its_scope_row`, `workspace_delete_refuses_while_its_scope_row_owns_work` (item, owner document), `workspace_delete_removes_an_empty_scope_row`. Then: `Project.owner_workspace_id`; `OwnerDocument`/`NewOwnerDocument`; `WorkspaceBoxPath.folder_path` and `workspace_folder`; `seed::WORKSPACE_KIND`, `seed::scope_row_slug`; trait methods `scope_project`, `write_owner_document`, `owner_documents`; MemStore implementation incl. the D6 hand cascade; fixtures: scope rows for the two demo workspaces in their own id block (`catalogue()` `:796-871`, `demo_uuid` ceiling 255) and the moved counts (`fixtures.rs:2491`, `:2516`, `:2598`) re-derived, not just repinned; `mem_store.rs` pin 233 → 233 + new.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`.

### T3: htui-store (Postgres and cache)
- **Action**: `create_workspace` in one transaction with the scope row and the WS seed (`seed_project` parameterised by kinds); bootstrap (`seed_if_empty_as`) creates missing scope rows idempotently; `create_project`/`update_project` refuse `@`; `upsert_workspace_project` refuses a scope row; `update_workspace` renames the scope row in its transaction; `delete_workspace_once` refuses per D6 (repeatable-read pattern kept); `insert_document` writes `project_id` from the item; `write_owner_document` locks the project row; `owner_documents`; `scope_project`; `"item_id!"` on `pg/read.rs:309, 525, 556` and `pg/write.rs:404`; `query_as!(Project…)` at `pg/read.rs:685`, `pg/write.rs:3151, 3208` return `owner_workspace_id`; `folder_path` SQL (`pg/read.rs:2866`, `pg/write.rs:3107`, `:494`); demo loader (`pg/demo.rs:99-384`) inserts scope rows matching the fixture; cache: project mirror carries `owner_workspace_id`, `refresh_document` filters `WHERE project_id = $1` and binds a nullable `item_id`, cache `scope_project`/`owner_documents`, `cache/read.rs:956` Project literal; tests: `tests/cache.rs:1682,1753` document inserts gain `project_id`, a cache test that a scope row's kinds, items and owner documents mirror, demo per-table counts (`migrations.rs:4895`, `:4943`); `cargo sqlx prepare` against a migrated scratch DB; `EXPECTED_CASES` pin.
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1` (qdrant_live owed on host); `cargo sqlx prepare --check`.

### T4: htui
- **Action**: `store_worker.rs:3037` sends `scope.project_ids` plus `scope_project(scope.workspace_id)` (memoised per workspace); `hierarchy.rs` `infer` and `SetWorkspaceRoot` use `folder_path` and `workspace_folder`; Settings hierarchy follows the rename (no visible text change until M3); `workspace_of` resolves a scope row via `owner_workspace_id`; tests in `crates/htui/tests/hierarchy.rs` (rename sites `:72, :359-360, :1147, :2042-2045, :2963`; infer defaulting to `~/workspaces/<slug>`).
- **Validate**: `cargo test -p htui --all-features --test hierarchy -- --test-threads=1`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
bash scripts/gate.sh                              # full gate; live-Qdrant suites owed on host
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Migration number collides at collect (MOD-139, MOD-143, MOD-159 owe one) | High | renumber at `scripts/hr collect`, repin `migrations.rs` |
| Fixture/demo id strides and whole-store counts move | Medium | scope rows in their own id block; the three fixture tests and the demo per-table counts re-derived (claims 11, 17) |
| Cache 0009 must apply to a populated old mirror before the rebuild | Medium | T1 test opens a cache file at 0008 with document rows, then migrates |
| `.sqlx` churn | High | one `cargo sqlx prepare` at the end of T3 |
| A scope-row run with no item (MOD-136 later) on workspace delete | Low | D6 refuses while any run exists; covered by the refusal case |
| Snapshot churn | Low | no visible text changes in M1; full insta run anyway |

## Acceptance
- [ ] All tasks complete, tests written first
- [ ] Every PRD M1 outcome observable in a conformance, migration or cache test
- [ ] Gate green except live-Qdrant suites (owed on host)
- [ ] Patterns mirrored, not reinvented

## Verified claims

Checked against the tree on 2026-11-08 (two read-only sweeps; `psql` against the sandbox database).

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `Scope` is built only through `from_workspace` | **FALSE** → D1 amended (no `Scope` field) | ~101 `Scope {` literals: htui-core 15, htui-orch 3, htui-worker 2, htui-store 11, htui/src 49, htui/tests 21 |
| 2 | `Project`/`WorkspaceSummary` literal churn is small | PARTLY → `WorkspaceSummary` left alone | `Project {` 5 sites + 3 `query_as!`; `WorkspaceSummary {` 13 sites |
| 3 | `update_workspace` can change the slug | TRUE | `pg/write.rs:2990` `COALESCE($3, slug)`, single statement, no tx; `mem.rs:2981`; `create_workspace` `:2953` also no tx → D7 |
| 4 | Every FK to `project(id)` cascades; `delete_project` exists | TRUE | `0001_init.sql:163,187,216,268,284,300,312,435,449`; `0006:35,48,72`; `0022:26`; `pg/write.rs:5904/9961`, `mem.rs:5609` |
| 5 | Seeded graphs are project-scoped | TRUE (seed needs a kinds parameter) | `seed.rs:197` `GraphScope::Project`; `pg/write.rs:9588` loops all `seed::KINDS`; `mem.rs:3172` |
| 6 | Cascading an empty scope row on workspace delete works | TRUE with notes | `pg/write.rs:9898` refuses only held workspace graphs; no-action FKs point at rows the same delete removes; Mem cascades by hand (`mem.rs:5555`); `DeleteReach` does not count scope-row rows |
| 7 | A cache migration may drop and recreate `document` | TRUE (must apply on a populated old file) | `cache/mod.rs:138` migrates first, `:141-152` rebuilds on version mismatch |
| 8 | Cache task does not share `.sqlx` with the Pg task | **FALSE** → T3 merges Pg and cache | `refresh.rs` holds 22 compile-time `sqlx::query!` reads incl. `refresh_document` `:1067`, binding `row.item_id` `:1085` |
| 9 | Cache refresher's project list comes from the store worker | TRUE | `store_worker.rs:3037` `projects.send_replace(scope.project_ids.clone())` → `connect.rs:306,387` → `refresh.rs:122` |
| 10 | Membership writer name | corrected | `upsert_workspace_project`, Pg `pg/write.rs:3046`, Mem `mem.rs:3028/9632`, caller `htui/src/hierarchy.rs:323` |
| 11 | Fixture strides tolerate scope rows | PARTLY → own id block | strides in `catalogue()` `fixtures.rs:796-871` per-project multiples; `demo_uuid(class, n: u8)` ceiling 255; `pg/demo.rs` inserts by SQL (`:99, :116, :229, :299, :384`) |
| 12 | `plan` and `review` templates exist | TRUE | `prompt/defaults.rs:221, 223` |
| 13 | Item-document queries needing `"item_id!"` | PARTLY → list corrected | `pg/read.rs:309` (DocumentHead), `:525`, `:556`, `pg/write.rs:404`; `pg/read.rs:858` already has it; `tests/cache.rs:1682,1753` insert without `project_id` |
| 14 | No slug validation; `update_project` changes slugs | TRUE | `pg/write.rs:3147, 3198`; `mem.rs:3141, 3203` |
| 15 | Postgres ≥ 15 for `NULLS NOT DISTINCT` | TRUE | sandbox server PostgreSQL 16.15 |
| 16 | Listers hide an unlinked scope row; `workspace_of` | TRUE / needs D9 | every lister reads through `workspace_project`; `workspace_of` (`hierarchy.rs:537`) matches membership → NotFound for a scope row |
| 17 | Whole-store counts that move | PARTLY → listed in T2/T3 | `fixtures.rs:2491, 2516, 2598` (`== 30`); `migrations.rs:4895, 4943`; `conformance.rs:25299` and `htui/tests/kinds.rs:196` per-project, unaffected |
| 18 | Task file sets: T1 ∥ T2, then T3 ∥ T4 | TRUE after amendment | intersections listed under Files to Change |
