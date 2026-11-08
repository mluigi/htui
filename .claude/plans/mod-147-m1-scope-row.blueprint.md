# Blueprint: MOD-147 M1 — scope row and owner documents in the store

**Plan**: `.claude/plans/mod-147-m1-scope-row.plan.md` (D1-D9 confirmed). **Design**: `docs/ANA-35.md` §7.5, §7.9 P4.
Every line ref below was read on branch `hr/MOD-147` (HEAD `d2ccf464`). The 0026 DDL and its refusals were
dry-run on the sandbox PostgreSQL 16.15 inside a rolled-back transaction over 0001-0025; cache 0009 was
dry-run on an in-memory SQLite built from cache 0001-0008 with rows in `project` and `document`.

## Plan deviations (facts the plan did not have)

| # | Fact | Evidence | What changes |
|---|---|---|---|
| V1 | Two more `ReadStore`+`WriteStore` implementors outside every task's file set | `crates/htui-agent/src/conformance.rs:645,768` (`UsageSpy`), `crates/htui-agent/tests/recorder.rs:332,456` (`SpyStore`) | **T2** adds the three delegations to both (htui-agent depends on htui-core only, so T2 keeps it green) |
| V2 | Two more implementors in htui-store the plan's T3 list omits, plus the mirror seeder | `crates/htui-store/src/backend.rs:896` (`impl ReadStore for Backend`), `crates/htui-store/src/writer.rs:131,351` (`Writer`), `crates/htui-store/src/testkit.rs:537-553` (`seed_mirror` project insert) | **T3** adds `scope_project`/`owner_documents` to `Backend`, all three to `Writer`, `owner_workspace_id` to `seed_mirror` |
| V3 | `scope_row_is_in_no_listing` cannot call `workspaces()` / `projects(scope)`: both are inherent, the suite is `S: WriteStore` | `mem.rs:494,526`; `pg/read.rs:1466,1583`; `cache/read.rs:1465,1570`; `backend.rs:241,280`; `conformance.rs:341` | The case asserts `workspace_projects` + `items`; the two inherent listers get one test per store: `mem.rs` unit test (T2), `pg_criteria.rs` (T3), `tests/cache.rs` (T3) |
| V4 | Dropping `document_item_id_kind_version_key` removes the **only** index leading with `item_id` (live: `document` has `document_pkey` and that unique, nothing else) | dry run on 0001-0025; `0001_init.sql:399` | 0026 adds `idx_document_item (item_id, kind, version)`. The plan's `document(project_id)` index is **dropped**: `uq_document_owner_kind_version` already leads with `project_id` |
| V5 | `document(id)` reads `WHERE id = $1`; with `"item_id!"` an owner-document id would fail to decode instead of answering `None` | `pg/read.rs:520-533`, `cache/read.rs:829-832` | Both gain `AND item_id IS NOT NULL`; pinned by `item_documents_exclude_owner_documents` |
| V6 | The demo loader inserts documents without `project_id` → NOT NULL violation after 0026 | `pg/demo.rs:679-697` | T3: `project_id` from the item in the same statement (§T3.9) |
| V7 | Delete reach counts documents through items only; owner documents would cascade uncounted (breaks PRD D13 "the report equals the act") | `pg/write.rs:581-582`; `mem.rs:5493-5498`; `mem.rs:5645` | Pg counts `WHERE project_id = $1`; Mem counts and deletes `owner_documents` too; pinned in `owner_document_versions_per_owner_and_kind` |
| V8 | `pg/write.rs:494` is `workspace_reach`'s `count(*)` of `workspace_box_path`; it names no column | `pg/write.rs:494` | No change there. Real `root_path` sites: `pg/read.rs:2875`, `pg/write.rs:3109-3114`, `conformance.rs:3391,3406,3433`, `pg_criteria.rs:2828`, `.sqlx` |
| V9 | T3 ∥ T4 is file-parallel only: `htui` cannot compile until htui-store's `Backend`/`Writer` implement the new trait methods | `backend.rs:896`, `writer.rs:131,351` | T4 may be written beside T3 but is **verified after T3 lands** |
| V10 | Existing infer tests call `serve` (`tests/hierarchy.rs:973-981`); with D8's default, `serve` would read the real `~/workspaces/graphics` | `htui/src/hierarchy.rs:235,560-580` | `serve_with_home(backend, request, home)`; `serve` passes `dirs::home_dir()`; tests pass `None` or a tempdir |
| V11 | D6's advice "close or move them first" cannot be followed: there is no item delete (`traits.rs:939` deletes kinds only), no item move (`update_kind_keeps_key_and_project`), documents are append-only, and a closed item still counts. A workspace whose scope row ever owned work becomes undeletable | `traits.rs:939, 1851`; `conformance.rs:87` (`update_kind_keeps_key_and_project`), `:90` (`no_delete_path`) | D6 kept as decided. Sentence below drops "close" (see §T2.6); **maintainer to confirm wording** |
| V12 | `delete_project` on a scope row is unspecified; today it would succeed and leave a workspace without one until the next bootstrap | `traits.rs:1426`, `pg/write.rs:9961`, `mem.rs:5609` | Not changed in M1 (no caller can reach it: the scope row is in no tree). Flagged as an open question |

## Maintainer amendments (2026-11-08, after V11/V12) — these SUPERSEDE every D6 detail below

**A1 (replaces D6).** Deleting a workspace **cascades its scope row with counts**, like `delete_project`:
- `delete_reach(DeleteTarget::Workspace(ws))` = today's `workspace_links` + `workspace_box_paths` **plus the
  scope row's full project reach** (every `DeleteReach` field the scope row's `project_reach` fills: items,
  counters, kinds, graphs, phases, templates, documents incl. owner documents (V7), runs, steps, events, notes,
  revisions, links, memory…), field-wise summed. The report equals the act (PRD D13). Both stores.
- `delete_workspace` is **refused only while a scope-row run is live** (`queued`, `running`,
  `awaiting_approval`), with the sentence `workspace_has_live_run(workspace_slug, n)` → "workspace `shop` has
  1 live run; stop it before deleting the workspace" (`n` singular/plural). Items, documents and finished runs
  no longer refuse; they cascade and are counted.
- So **`workspace_owns_work` is not written**; T2.3 adds `workspace_has_live_run` instead (unit test
  `workspace_has_live_run_counts_runs`). T2.5 MemStore `delete_workspace`: held-graph check, then the live-run
  refusal, then `reach = workspace part + project_reach(scope)`, then `delete_project(scope)` internals (bypassing
  A2's refusal) and the existing removals; returns the summed reach. T3.6 Postgres: the count query becomes
  `SELECT count(*) FROM run r JOIN project p ON p.id = r.project_id WHERE p.owner_workspace_id = $1 AND r.status
  IN ('queued','running','awaiting_approval')`; `workspace_reach` (`pg/write.rs:~480-510`) adds the scope row's
  `project_reach` summed in; the `DELETE FROM workspace` cascade is unchanged.
- Conformance case 12 becomes **`workspace_delete_refuses_while_a_scope_row_run_is_live`** (ws `busy`: mint a
  `WS` item, `create_run` on it → `Err(Constraint(workspace_has_live_run("busy", 1)))`, nothing removed; after the
  run finishes/cancels the delete succeeds). Case 13 becomes **`workspace_delete_cascades_its_scope_row_with_counts`**
  (ws with one `WS` item and one owner document: `reach = delete_reach(Workspace)` has `items 1`, `documents ≥ 1`,
  `item_kinds 1`, `step_graphs 1`, `phases 2`, `prompt_templates 10`; `delete_workspace(..) == reach`; then
  `project(scope)` `None`, the item gone, `owner_documents(scope)` empty, `scope_project(ws.id)` `None`).
- T4: the TUI's workspace-delete confirmation renders the reach it gets; check that the workspace-delete dialog
  prints the non-zero project fields (it may print only links/box paths today) and, if so, extend it to print
  the same lines the project dialog prints. Snapshot moves only where a fixture workspace's scope row adds lines;
  review each.

**A2 (answers V12).** `delete_project` (and `delete_reach(Project(scope))`'s caller path) **refuses a scope
row**: `Constraint(scope_row_is_deleted_with_its_workspace(slug))` → "project `@shop` is workspace `shop`'s own
scope row; delete the workspace instead". T2.3 adds the sentence fn; MemStore and PgStore check before any
write (Pg: `SELECT slug FROM project WHERE id = $1 AND owner_workspace_id IS NOT NULL` at the top of
`delete_project_once`); new conformance case **`scope_row_cannot_be_deleted_alone`** (CASES total 247, pins
233 → **247**). `delete_reach(Project(scope))` itself still answers (read-only).

Blueprint choices (not in the plan, small, reversible): the scope row's `name` is the workspace's `name` and
follows a name edit as its slug does (D7's transaction already touches the row); `write_owner_document` locks
the project `FOR NO KEY UPDATE` (serialises owner-document writers without blocking concurrent `mint_item`'s
`FOR KEY SHARE`); `NewOwnerDocument` has no `produced_by_step_id` (ANA-35 §7.5: a person writes them).

---

## T1 — migration 0026 and cache 0009 (tests first)

Files: CREATE `crates/htui-store/migrations/0026_workspace_scope.sql`,
`crates/htui-store/cache_migrations/0009_workspace_scope.sql`; UPDATE `crates/htui-store/tests/migrations.rs`.

### T1.1 Tests first (`tests/migrations.rs`, raw SQL only, no new Rust API)

Pattern `migration_0021_types_an_existing_project_software` (`:2218`) and
`migration_0024_refuses_two_live_graph_runs_naming_the_items` (`:5895`); helpers `plant_project_kind`, `plant_item`.

1. `migration_0026_backfills_document_project_and_keeps_item` — `bare_db`, `run_to(25)`, user, workspace (raw),
   `plant_project_kind`, `plant_item`, two documents (`plan` v1, v2); `MIGRATOR.run`. Assert both rows have
   `project_id = <project>` and their `item_id`; `information_schema.columns`: `document.project_id` `NO`,
   `document.item_id` `YES`, `project.owner_workspace_id` `YES`, `workspace_box_path.folder_path` present and
   `root_path` absent; `count(*) FROM project WHERE owner_workspace_id IS NOT NULL` = 0 (scope rows are the
   bootstrap's, T3).
2. `migration_0026_checks_refuse_a_half_scope_row` — after `run`, each refused with `23514`/`23505`/`23503` and
   `db_error.constraint()` equal to: user project slug `@x` → `chk_project_scope_slug`; slug `x` type
   `workspace` no owner → `chk_project_scope_row`; a valid scope row (`@shop`, `workspace`, owner) lands, a
   second for the same workspace → `uq_project_owner_workspace`; a document whose `(item_id, project_id)`
   names another project → `fk_document_item_project`.
3. `migration_0026_versions_owner_documents_per_owner_and_kind` — owner document (`item_id NULL`, `goals`, v1)
   lands; a second v1 → `23505 uq_document_owner_kind_version` (NULLS NOT DISTINCT); v2 lands; an item document
   `goals` v1 of the same project lands (different `item_id`).
4. `migration_0026_refuses_a_reserved_slug_naming_the_projects` — `run_to(25)`, plant projects `@legacy` and a
   normal one; `run_to(26)` errs; text contains `MOD-147` and `@legacy` and not the normal slug;
   `_sqlx_migrations` count 25; `document.project_id` column absent.
5. `migration_0026_deleting_a_workspace_cascades_its_scope_row` — after `run`: scope row + `step_graph` +
   `step_graph_phase` + `item_kind` (default graph) + `prompt_template` + owner document on it; `DELETE FROM
   workspace`; all five tables empty for it (the `item_kind.default_graph_id` NO ACTION FK does not trip —
   verified).
6. `cache_0009_applies_over_a_populated_0008_mirror` — **no Postgres, never skips**: `tempfile::tempdir()`,
   `SqlitePool` on `<dir>/cache.sqlite?mode=rwc`, `htui_store::CACHE_MIGRATOR.run_to(8, &pool)`, insert one
   `project` and one old-shape `document` row, `CACHE_MIGRATOR.run(&pool)` Ok; `pragma_table_info('document')`
   has `project_id` notnull 1 and `item_id` notnull 0; `project` has `owner_workspace_id`; `document` count 0
   (dropped: the version bump rebuilds anyway), `project` count 1; indexes `idx_cache_document_item`,
   `idx_cache_document_project`, `idx_cache_project_owner` exist (`sqlite_master`).
7. Repin `migrations_apply_on_a_clean_database` (`:112-130`): `1..=26`, message gains "and MOD-147's
   0026_workspace_scope.sql". `TABLES` stays 51 (no table added).

### T1.2 `migrations/0026_workspace_scope.sql` (full text; dry-run verified)

```sql
-- 0026_workspace_scope.sql - MOD-147 milestone 1 (ANA-35 7.5, 7.9 P4, 7.10 row 4; plan D2, D3, D8,
-- D9). Forward-only (R-STO-5).
--
-- Every workspace gets one hidden scope row in project (owner_workspace_id set, type 'workspace', slug
-- '@<workspace slug>'), created by create_workspace in its own transaction and, for workspaces that
-- exist before this migration, by the bootstrap (PgStore::seed_if_empty_as), not here: the seed is
-- code (htui_core::seed). The row never joins workspace_project. Three facts are tied by two CHECKs:
-- owner_workspace_id set <=> type 'workspace' <=> slug starting with '@'.
--
-- Documents gain project_id on every row and a nullable item_id (ANA-35 D2): an item document names
-- both (fk_document_item_project keeps the copy honest), a project's or a workspace's names only the
-- owner. Versions are per (owner, item, kind): uq_document_owner_kind_version, NULLS NOT DISTINCT
-- (PostgreSQL 15+). The old (item_id, kind, version) unique was the only index leading with item_id,
-- so idx_document_item replaces it; the new unique leads with project_id and serves owner reads.
-- uq_document_id_project is the target MOD-114's attachments reference.
--
-- workspace_box_path.root_path becomes folder_path; an absent row means ~/workspaces/<slug>
-- (htui_core::model::workspace_folder). No COMMENT ON (the column-comment pins stay), no new table
-- (TABLES stays 51). The mirror gains project.owner_workspace_id and the new document shape in
-- cache_migrations/0009_workspace_scope.sql; schema_version becomes 26, so each box rebuilds its mirror
-- once. A headless worker never migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. Pre-check: '@' slugs and the 'workspace' type are reserved for scope rows
-- --------------------------------------------------------------------------------------------

DO $$
DECLARE
    offenders TEXT;
BEGIN
    SELECT string_agg(p.slug, ', ' ORDER BY p.slug)
      INTO offenders
      FROM project p
     WHERE p.slug LIKE '@%' OR p.type = 'workspace';
    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION 'MOD-147 (0026_workspace_scope): slugs starting with @ and the type workspace are reserved for workspace scope rows, and these projects use them: %. Rename them and change their type with the previous htui, then start this one again.', offenders
            USING ERRCODE = 'check_violation';
    END IF;
END
$$;

-- --------------------------------------------------------------------------------------------
-- 2. project: the scope row
-- --------------------------------------------------------------------------------------------

ALTER TABLE project
    ADD COLUMN owner_workspace_id UUID
        CONSTRAINT uq_project_owner_workspace UNIQUE
        CONSTRAINT fk_project_owner_workspace REFERENCES workspace(id) ON DELETE CASCADE;
ALTER TABLE project ADD CONSTRAINT chk_project_scope_row
    CHECK ((owner_workspace_id IS NULL) = (type <> 'workspace'));
ALTER TABLE project ADD CONSTRAINT chk_project_scope_slug
    CHECK ((owner_workspace_id IS NULL) = (slug NOT LIKE '@%'));

-- --------------------------------------------------------------------------------------------
-- 3. document: project_id on every row, item_id nullable
-- --------------------------------------------------------------------------------------------

ALTER TABLE item ADD CONSTRAINT uq_item_id_project UNIQUE (id, project_id);

ALTER TABLE document
    ADD COLUMN project_id UUID
        CONSTRAINT fk_document_project REFERENCES project(id) ON DELETE CASCADE;
UPDATE document d SET project_id = i.project_id FROM item i WHERE i.id = d.item_id;
ALTER TABLE document ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE document ALTER COLUMN item_id DROP NOT NULL;
ALTER TABLE document ADD CONSTRAINT fk_document_item_project
    FOREIGN KEY (item_id, project_id) REFERENCES item(id, project_id) ON DELETE CASCADE;
ALTER TABLE document ADD CONSTRAINT uq_document_id_project UNIQUE (id, project_id);
ALTER TABLE document DROP CONSTRAINT document_item_id_kind_version_key;     -- 0001's unnamed UNIQUE
ALTER TABLE document ADD CONSTRAINT uq_document_owner_kind_version
    UNIQUE NULLS NOT DISTINCT (project_id, item_id, kind, version);
CREATE INDEX idx_document_item ON document (item_id, kind, version);

-- --------------------------------------------------------------------------------------------
-- 4. workspace_box_path: the folder
-- --------------------------------------------------------------------------------------------

ALTER TABLE workspace_box_path RENAME COLUMN root_path TO folder_path;
```

Names verified live: `document_item_id_kind_version_key` (u), `document_item_id_fkey` (kept: harmless with a
nullable column), `project_slug_key`. Explicit `CONSTRAINT` names on every new object, as 0021 does.

### T1.3 `cache_migrations/0009_workspace_scope.sql` (dry-run verified on a populated 0008 file)

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-147 milestone 1 (migrations/0026_workspace_scope.sql): project.owner_workspace_id
-- (the scope row, looked up by ReadStore::scope_project) and document's new shape, project_id on every
-- row with a nullable item_id. document is dropped and recreated: SQLite cannot drop NOT NULL in
-- place. CACHE_MIGRATOR runs before the schema_version check (cache/mod.rs:138), so this must apply on
-- a populated 0008 file; the rows it drops come back on the full rebuild schema_version 26 forces.
-- workspace_box_path is not mirrored, so its rename has no mirror side.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE project ADD COLUMN owner_workspace_id TEXT;
CREATE INDEX idx_cache_project_owner ON project(owner_workspace_id);

DROP TABLE document;
CREATE TABLE document (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, item_id TEXT, kind TEXT NOT NULL,
    version INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL, produced_by_step_id TEXT,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_document_item    ON document(item_id, kind, version);
CREATE INDEX idx_cache_document_project ON document(project_id, kind, version);
```

Every mirror read and the refresher name columns (`refresh.rs:1047-1057`, `cache/read.rs:593,831,846`), so
column order is free. `MIRRORED_TABLES` (`cache/mod.rs`) is unchanged.

**Build order**: 1-6 (red: 0026/0009 absent) → the two SQL files → 7 → `cargo test -p htui-store --all-features
--test migrations -- --test-threads=1`. T1 adds no Rust API, so it verifies on the pre-T2 tree.

---

## T2 — core model, seed, traits, MemStore, conformance (tests first)

Files: the plan's T2 set **plus** `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` (V1).

### T2.1 Model

`crates/htui-core/src/model/hierarchy.rs`:

```rust
use std::path::{Path, PathBuf};

pub struct Project {
    // ... after `project_type`:
    /// `project.owner_workspace_id` (MOD-147 D9): `Some` only on a workspace's scope row, whose
    /// `project_type` is `workspace` and whose slug is `@<workspace slug>` (`chk_project_scope_row`,
    /// `chk_project_scope_slug`). A scope row joins no workspace and is in no project list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_workspace_id: Option<WorkspaceId>,
    // ...
}

pub struct WorkspaceBoxPath {
    pub workspace_id: WorkspaceId,
    pub box_id: BoxId,
    /// `workspace_box_path.folder_path` (MOD-147 D8): the workspace's folder on this box.
    pub folder_path: String,          // was `root_path`
    pub updated_at: DateTime<Utc>,
}

/// The directory under the home directory that holds the default workspace folders (ANA-35 §7.5).
pub const WORKSPACES_DIR: &str = "workspaces";

/// MOD-147 D8: workspace `slug`'s folder on one box — `row.folder_path` when the box has a
/// `workspace_box_path` row, else `<home>/workspaces/<slug>`. Pure: no I/O, no canonicalisation;
/// the caller resolves `home` (htui reads `dirs::home_dir()`, as `settings/logging.rs:244` does) and
/// canonicalises the answer.
#[must_use]
pub fn workspace_folder(slug: &str, row: Option<&WorkspaceBoxPath>, home: &Path) -> PathBuf {
    match row {
        Some(row) => PathBuf::from(&row.folder_path),
        None => home.join(WORKSPACES_DIR).join(slug),
    }
}
```

htui-core has no `dirs` dependency and needs none. Unit test `workspace_folder_defaults_under_home`.

`crates/htui-core/src/model/document.rs` (imports `ProjectId`):

```rust
/// A `document` row a project — or a workspace's scope row — owns: `item_id IS NULL` (MOD-147 D2,
/// ANA-35 §7.5). Body included; versions are per `(project, kind)`. [`Document`] stays item-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerDocument {
    pub id: DocumentId,
    pub project_id: ProjectId,
    pub kind: String,
    pub version: i32,
    pub title: String,
    pub body: String,
    /// `document.produced_by_step_id`; `None` for every row M1 writes.
    pub produced_by_step_id: Option<StepId>,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::write_owner_document`]: everything but `version`, which the
/// store allocates under the owner's row lock, and `produced_by_step_id`, which is always `NULL` (a
/// person writes an owner document, ANA-35 §7.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewOwnerDocument {
    pub id: DocumentId,
    pub project_id: ProjectId,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
}
```

`model/mod.rs:120` → `pub use document::{Document, DocumentHead, NewDocument, NewOwnerDocument, OwnerDocument,
ResolvedInput};` `:123-127` gains `WORKSPACES_DIR, workspace_folder`.

`Project {` literals gaining `owner_workspace_id: None`: `fixtures.rs:753` (and the new scope rows, Some),
`secret.rs:1412`, `mem.rs:3157` (create_project), `htui-orch/src/secrets.rs:219`; `cache/read.rs:956` is T3's.

### T2.2 Seed (`crates/htui-core/src/seed.rs`)

```rust
/// MOD-147 D5: the one kind a workspace's scope row is seeded with — plan (soft gate) then review (hard
/// gate), reusing the `plan` and `review` templates (`prompt/defaults.rs:221,223`) until MOD-148 adds the
/// workspace `plan` body. Not in [`KINDS`]: a user project never gets it. Its default edges are one
/// `next` (plan → review) and one `changes_requested` (review → plan, `loop_limit` 1: no `implement`, so
/// the rule falls back to the previous phase).
pub const WORKSPACE_KIND: KindSeed = KindSeed {
    prefix: "WS",
    name: "workspace",
    description: "Work that spans the workspace's projects",
    phases: &[
        PhaseSeed { name: "plan", input_kinds: &[], gate_hard: false },
        PhaseSeed { name: "review", input_kinds: &["plan"], gate_hard: true },
    ],
};

/// `project.type` of every scope row: MOD-146's built-in `workspace` (`selectable: false`).
pub const SCOPE_ROW_TYPE: &str = "workspace";

/// MOD-147 D3: `@<workspace slug>`, the scope row's slug. `@` is reserved: no user slug starts with it.
#[must_use]
pub fn scope_row_slug(workspace_slug: &str) -> String { format!("@{workspace_slug}") }

/// MOD-147 D3: whether `slug` has the scope-row form.
#[must_use]
pub fn is_scope_row_slug(slug: &str) -> bool { slug.starts_with('@') }
```

Seed tests: `the_workspace_kind_is_plan_then_a_hard_review` (prefix passes `ItemKind::prefix_is_valid`, phases,
gates, `body_of` for both, `default_edges` → `(next, loops) == (1, 1)`, loop `review → plan` limit 1);
`scope_row_slug_is_at_slug`. `gate_hard_is_exactly_the_three_prd_flags` keeps iterating `KINDS` only.

### T2.3 Refusal sentences (`crates/htui-core/src/store/traits.rs`, beside `workspace_graph_is_held` `:3253`)

```rust
/// MOD-147 D3: a user project's slug may not start with `@`.
#[must_use]
pub fn project_slug_is_reserved(slug: &str) -> String {
    format!("project.slug `{slug}` starts with `@`, which is reserved for a workspace's scope row")
}

/// MOD-147 D3: [`project_slug_is_reserved`] when `slug` starts with `@`, else `None`. Both stores judge it
/// before any read, on create and on a slug patch, so they refuse in the same words.
#[must_use]
pub fn project_slug_refusal(slug: &str) -> Option<String> {
    crate::seed::is_scope_row_slug(slug).then(|| project_slug_is_reserved(slug))
}

/// MOD-147 D3: a slug patch on a scope row (MemStore's words; Postgres's `chk_project_scope_slug` is the
/// backstop and answers its own `Constraint`).
#[must_use]
pub fn scope_row_slug_is_fixed(slug: &str) -> String {
    format!("project `{slug}` is a workspace's scope row; its slug follows the workspace's, so rename the workspace")
}

/// MOD-147 D4: `upsert_workspace_project` on a scope row.
#[must_use]
pub fn scope_row_joins_no_workspace(slug: &str) -> String {
    format!("project `{slug}` is a workspace's own scope row and joins no workspace")
}

/// MOD-147 D6: `delete_workspace` while the scope row owns work. Non-zero parts only, `a, b and c`;
/// `1 item` / `3 items`, `1 document` / `2 documents`, `1 run` / `2 runs`. Example:
/// "workspace `shop` owns 3 items and 1 document; its work would be deleted with it, so it is kept".
/// (V11: the plan's "close or move them first" names no path that exists; maintainer to confirm.)
#[must_use]
pub fn workspace_owns_work(workspace: &str, items: u64, documents: u64, runs: u64) -> String
```

Unit test `workspace_owns_work_names_only_what_it_owns` (1 part, 2 parts, 3 parts, singular/plural).

### T2.4 Trait methods

`ReadStore` (append after `memory_revisions`, `traits.rs:347`, under `// ---- MOD-147 M1 ----`):

```rust
/// The workspace's scope row (MOD-147 D1): the one `project` with `owner_workspace_id = workspace`.
/// `None` for an unknown workspace, and on the mirror for one whose scope row is not mirrored yet
/// (the store worker adds it to the refresher's projects, T4). Mirrored, so every backend answers.
///
/// # Errors
/// The backend's own failures only.
async fn scope_project(&self, workspace: WorkspaceId) -> Result<Option<ProjectId>>;

/// The project's owner documents (`item_id IS NULL`, MOD-147 D2), every version, bodies included, in
/// `(kind bytes, version)` order; empty for an unknown project. Item documents never appear here, and
/// owner documents never appear in [`documents`](Self::documents), [`document`](Self::document),
/// [`documents_of_kinds`](Self::documents_of_kinds) or [`resolve_inputs`](Self::resolve_inputs).
///
/// # Errors
/// The backend's own failures only.
async fn owner_documents(&self, project: ProjectId) -> Result<Vec<OwnerDocument>>;
```

`WriteStore` (after `write_document`, `traits.rs:1851`):

```rust
/// Inserts an owner document at `max(version) + 1` for `(project, kind)` among the project's owner
/// documents, under the project's row lock (MOD-147 D2), so two writers cannot allocate one version.
/// An item document of the same kind does not move the count, nor this one the item's.
///
/// # Errors
/// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "project" }`;
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a duplicate id (any document)
/// or an unknown `created_by`.
async fn write_owner_document(&self, new: NewOwnerDocument) -> Result<OwnerDocument>;
```

Doc edits, same file: `create_workspace` (`:747`) "inserts the workspace **and its scope row** (`@<slug>`,
type `workspace`, the `WS` seed: one kind, one graph, two phases, two edges, ten templates) in one
transaction"; `update_workspace` (`:754`) "a `slug`/`name` edit renames the scope row in the same
transaction"; `upsert_workspace_project` (`:776`) adds `Constraint(`[`scope_row_joins_no_workspace`]`)`;
`create_project`/`update_project` (`:817-838`) add [`project_slug_refusal`] first; `delete_workspace`
(`:1410`) "its scope row and the scope row's seed go with it; refused with [`workspace_owns_work`] while the
scope row owns an item, an owner document or a run (MOD-147 D6)".

Implementors (exhaustive; `impl ReadStore for` / `impl WriteStore for` across the workspace):

| Crate | Impl | Task | Body |
|---|---|---|---|
| htui-core | `MemStore` (`mem.rs:9326`, `:9472`) | T2 | §T2.5 |
| htui-agent | `UsageSpy` (`src/conformance.rs:645`, `:768`) | **T2** | `self.inner.<m>(..).await` |
| htui-agent | `SpyStore` (`tests/recorder.rs:332`, `:456`) | **T2** | `self.inner.<m>(..).await` |
| htui-store | `PgStore` (`pg/read.rs:82`, `pg/write.rs:1677`) | T3 | §T3 |
| htui-store | `CacheStore` (`cache/read.rs:396`, read only) | T3 | §T3 |
| htui-store | `Backend` (`backend.rs:896`, read only) | **T3** | three-arm dispatch like `documents` (`:921`) |
| htui-store | `Writer` (`writer.rs:131`, `:351`) | **T3** | two-arm delegation |

No `todo!()` anywhere: T2 leaves htui-store and htui not compiling until T3/T4 (the `WorkspaceBoxPath` rename
alone breaks them), which the plan's T1 ∥ T2 → T3 ∥ T4 order already assumes.

### T2.5 MemStore (`crates/htui-core/src/store/mem.rs`)

- `State` (`:286`) gains `owner_documents: Vec<OwnerDocument>`; `from_demo` (`:436`) `owner_documents: Vec::new()`.
- Extract `fn seed_project(&mut self, project_id: ProjectId, kinds: &[seed::KindSeed], created_by: UserId,
  now: DateTime<Utc>)` from `create_project` `:3173-3209` (graphs, phases, edges, kinds by enumerate position,
  then all ten `DEFAULT_TEMPLATES` regardless of `kinds`). `create_project` calls it with `&seed::KINDS`.
- `create_workspace` (`:2953`): validate (user, id, slug, and `seed::scope_row_slug(&new.slug)` not taken by a
  project), then insert the workspace, then the scope row `Project { id: ProjectId::new(), slug:
  scope_row_slug(..), name: new.name.clone(), description: String::new(), project_type: SCOPE_ROW_TYPE,
  owner_workspace_id: Some(row.id), secret_provider: None, secret_scope: None, settings: {}, created_by, now, now }`,
  then `self.seed_project(scope.id, std::slice::from_ref(&seed::WORKSPACE_KIND), created_by, now)`.
- `update_workspace` (`:2981`): after the applied edit, when `patch.slug` or `patch.name` was `Some`, the row
  with `owner_workspace_id == Some(id)` gets `slug = scope_row_slug(&row.slug)`, `name = row.name`,
  `updated_at = now` (Postgres's trigger moves it too). `Stale` touches nothing.
- `upsert_workspace_project` (`:3028`): first, if `link.project_id` names a scope row →
  `Constraint(scope_row_joins_no_workspace(&slug))` (Postgres's pre-read order, §T3.4); then the two existence checks.
- `create_project` (`:3141`): `project_slug_refusal(&new.slug)` first, before `require_user`.
- `update_project` (`:3215`): `patch.slug.as_deref().and_then(project_slug_refusal)` first (before NotFound);
  after Stale, a `Some(slug)` patch on a scope row → `Constraint(scope_row_slug_is_fixed(&current.slug))`.
- `write_owner_document` (new, beside `write_document` `:7224`): project exists else
  `NotFound { entity: "project" }`; id unused in `documents` **and** `owner_documents` else
  `Constraint(already_exists("document", id))`; `require_user(created_by, "document.created_by")`; version =
  max over `owner_documents` with same `(project_id, kind)` + 1; push; return.
- `delete_workspace` (`:5555`): after the held-graph check, `if let Some(scope) = scope row of id` → count
  `items` (`project_id == scope`), `owner_documents` (`project_id == scope`), `runs` (`project_id == scope`);
  any non-zero → `Constraint(workspace_owns_work(&workspace.slug, ..))`, nothing removed. Else compute `reach`
  as today, then `self.delete_project(scope, now)?` (its reach discarded: the scope seed is not in a
  workspace's `DeleteReach`, as on Postgres), then the existing removals.
- `project_reach` (`:5493`): `documents` adds `owner_documents` with `project_id == id`; `delete_project`
  (`:5645`) adds `self.owner_documents.retain(|row| row.project_id != id)`.
- `ReadStore`: `scope_project` = first project with `owner_workspace_id == Some(workspace)`;
  `owner_documents` = filter by `project_id`, sort by `(kind.as_bytes(), version)`. `WriteStore`:
  `write_owner_document` = `self.write(|state| state.write_owner_document(new))`.
- `#[cfg(test)]` test `workspaces_and_projects_never_list_a_scope_row` (V3): `MemStore::demo()`;
  `workspaces()` Platform/Graphics `projects` exclude `ids::PROJECT_SCOPE_*`; `projects(&Scope { workspace_id:
  PLATFORM, project_ids: vec![HTUI, AGY, PROJECT_SCOPE_PLATFORM] })` answers HTUI, AGY only.

### T2.6 Conformance cases (`crates/htui-core/src/store/conformance.rs`)

Append to `CASES` after `"record_branch_refuses_a_stale_fence"` (`:324`) under
`// MOD-147 M1: the workspace scope row and owner documents (plan D1-D9)`, in this order; one boxed arm each
before `other =>` (`:1032`). Helpers: `new_workspace` (`:3127`), `new_project`, `new_item` (`:1145`),
`applied`/`stale`, `edge_shape`, a new `new_owner_document(project, kind) -> NewOwnerDocument`.

1. `create_workspace_creates_its_scope_row` — `ws = create_workspace(new_workspace("scoped"))`;
   `scope = scope_project(ws.id)` is `Some`; `project(scope)` has slug `@scoped`, type `workspace`,
   `owner_workspace_id == Some(ws.id)`, name `ws.name`, settings `{}`, no secret, `created_by == ws.created_by`.
   A second `create_workspace(new_workspace("scoped"))` → `Constraint(_)`; `scope_project(ws.id)` unchanged.
2. `scope_row_is_in_no_listing` — `workspace_projects(PLATFORM)` ids == `[HTUI, AGY]`; fresh ws →
   `workspace_projects` empty; `items(&platform_scope [HTUI, AGY])` holds no item of `PROJECT_SCOPE_PLATFORM`
   after one `WS` mint there. (Inherent listers: V3.)
3. `scope_project_names_the_scope_row` — fixture: PLATFORM → `Some(PROJECT_SCOPE_PLATFORM)`, GRAPHICS →
   `Some(PROJECT_SCOPE_GRAPHICS)`, `WorkspaceId::new()` → `None`; `project(PROJECT_SCOPE_PLATFORM)
   .owner_workspace_id == Some(PLATFORM)`; `project(HTUI).owner_workspace_id == None`.
4. `scope_row_cannot_join_a_workspace` — `upsert_workspace_project(GRAPHICS, PROJECT_SCOPE_PLATFORM, 9)` →
   `Err(Constraint(scope_row_joins_no_workspace("@platform")))` exactly; `workspace_projects(GRAPHICS)` == `[VULKAN]`.
5. `user_slug_cannot_start_with_at` — `create_project(new_project("@ops"))` →
   `Constraint(project_slug_is_reserved("@ops"))`; `update_project(HTUI, token, slug "@htui")` → same words, row
   unchanged; `update_project(PROJECT_SCOPE_PLATFORM, token, slug "platform2")` → `Constraint(_)`; a name-only
   patch on the scope row applies.
6. `scope_row_is_seeded_with_the_ws_kind` — fresh ws; `item_kinds(scope)` → `[("WS","workspace",0)]`;
   `step_graphs(scope)` → one `workspace` graph, `GraphScope::Project(scope)`; `phases` →
   `[("plan",[],false),("review",["plan"],true)]`; `graph_edges` shape == `seed::default_edges(&phases)` (2 edges);
   `delete_reach(Project(scope))` → `(item_kinds, step_graphs, phases, prompt_templates, items, workspace_links)
   == (1, 1, 2, 10, 0, 0)`.
7. `scope_row_mints_ws_keys` — `mint_item(new_item(PROJECT_SCOPE_PLATFORM, KIND_SCOPE_PLATFORM_WS, ..))` twice →
   keys `WS-1`, `WS-2`, `project_id` the scope row; `items(Scope{PLATFORM, [PROJECT_SCOPE_PLATFORM]})` lists both;
   a `FEAT` mint in HTUI still answers `FEAT-4` (per-owner counter).
8. `owner_document_versions_per_owner_and_kind` — reach0 = `delete_reach(Project(HTUI)).documents`; owner docs
   HTUI `overview` ×2 → v1, v2; HTUI `goals` → v1; `PROJECT_SCOPE_PLATFORM` `goals` → v1; unknown project →
   `NotFound { entity: "project" }`; reused id → `Constraint(_)`; `delete_reach(Project(HTUI)).documents == reach0 + 3`.
9. `owner_documents_exclude_item_documents` — write HTUI `b` then `a`; `owner_documents(HTUI)` == exactly those
   two, `a` first (byte order), no `DOC_*` fixture row; `owner_documents(ProjectId::new())` empty.
10. `item_documents_exclude_owner_documents` — owner doc HTUI kind `plan`; `documents(HTUI_FEAT_1)` unchanged;
    `documents_of_kinds(HTUI_FEAT_1, ["plan"])` is still `DOC_FEAT_1_PLAN_V2`; `document(owner.id)` is `None` (V5);
    `write_document(new_document(HTUI_FEAT_1, "plan", None)).version == 3`.
11. `workspace_rename_renames_its_scope_row` — ws `before`; `update_workspace(slug "after", name "After")`
    applied → `project(scope)` slug `@after`, name `After`, `updated_at` moved; a stale retry changes nothing;
    `scope_project(ws.id)` is the same id.
12. `workspace_delete_refuses_while_its_scope_row_owns_work` — ws `busy`: mint a `WS` item →
    `delete_workspace` = `Err(Constraint(workspace_owns_work("busy", 1, 0, 0)))`, workspace and scope row still
    there; ws `goals`: one owner doc only → `workspace_owns_work("goals", 0, 1, 0)`.
13. `workspace_delete_removes_an_empty_scope_row` — fresh ws; `reach = delete_reach(Workspace)`;
    `delete_workspace(..) == reach` (seed rows uncounted on both stores); then `project(scope)` `None`,
    `item_kinds(scope)` and `step_graphs(scope)` empty, `scope_project(ws.id)` `None`.

Pins: `crates/htui-core/tests/mem_store.rs:37` 233 → **246** (message gains "and MOD-147 M1's thirteen for the
workspace scope row and owner documents (plan D1-D9)"); `htui-store/tests/pg_conformance.rs:51` 233 → 246 is T3's.

### T2.7 Fixture (`crates/htui-core/src/fixtures.rs`)

Ids (new `demo_ids!` block after `:167`), all in their own block `SCOPE_BLOCK: u8 = 200`:

| Const | `(class, n)` |
|---|---|
| `PROJECT_SCOPE_PLATFORM: ProjectId` | `(PROJECT, 200)` |
| `PROJECT_SCOPE_GRAPHICS: ProjectId` | `(PROJECT, 201)` |
| `KIND_SCOPE_PLATFORM_WS` / `KIND_SCOPE_GRAPHICS_WS: ItemKindId` | `(ITEM_KIND, 200 / 201)` |
| `GRAPH_SCOPE_PLATFORM_WS` / `GRAPH_SCOPE_GRAPHICS_WS: StepGraphId` | `(STEP_GRAPH, 200 / 201)` |

```rust
/// `(workspace, scope row, workspace slug, workspace name)` of the two demo scope rows (MOD-147), in
/// `demo_uuid` order.
const SCOPE_SPECS: [(WorkspaceId, ProjectId, &str, &str); 2] = [
    (ids::WORKSPACE_PLATFORM, ids::PROJECT_SCOPE_PLATFORM, "platform", "Platform"),
    (ids::WORKSPACE_GRAPHICS, ids::PROJECT_SCOPE_GRAPHICS, "graphics", "Graphics"),
];
/// First `n` of the scope rows' id block: above every per-project stride (max today 44, phases).
const SCOPE_BLOCK: u8 = 200;
```

`projects()` (`:750`) appends, per `SCOPE_SPECS` entry, `Project { slug: seed::scope_row_slug(slug), name,
description: "", project_type: SCOPE_ROW_TYPE, owner_workspace_id: Some(ws), settings: json!({}), secret None,
created_by: ids::USER, epoch(), epoch() }`.

`catalogue()` (`:796-871`), after the project loop (keep `edge_n` local to the project loop; scope rows use
their own counter):

```rust
// MOD-147: each workspace's scope row, seeded as `create_workspace` seeds one (the `WS` kind, its graph,
// two phases, two default edges, the ten templates), in its own id block so no per-project stride reaches it.
let ws_phases = seed::WORKSPACE_KIND.phases.len() as u8;               // 2
let mut scope_edge_n = SCOPE_BLOCK;
for (scope_index, (_, project_id, _, _)) in SCOPE_SPECS.iter().enumerate() {
    let scope_index = scope_index as u8;
    let n = SCOPE_BLOCK + scope_index;                                  // graph, kind: 200, 201
    // graph_row(STEP_GRAPH n), kind_row(ITEM_KIND n, position 0)
    // phases: PHASE SCOPE_BLOCK + scope_index * ws_phases + position  // 200..=203
    // edges:  EDGE scope_edge_n, scope_edge_n += 1                    // 200..=203
    // templates: PROMPT_TEMPLATE SCOPE_BLOCK + scope_index * templates_per_project + i  // 200..=219
}
```

Highest `n` 219 < 255. Documentation of `catalogue()` (`:782-795`) gains the block.

Moved counts, derived (2 scope rows × the `WS` seed: 1 kind, 1 graph, 2 phases, 2 edges, 10 templates):

| Site | Today | New | Derivation |
|---|---|---|---|
| `fixtures.rs:2501` `data.graphs.len()` | 15 | **17** | 3×5 + 2×1 |
| `fixtures.rs:2502` `data.phases.len()` | 45 | **49** | 3×15 + 2×2 |
| `fixtures.rs:2504-2506` `data.templates.len()` | 30 | **50** | (3+2)×10 |
| `fixtures.rs:2610` distinct template ids | 30 | **50** | message "three projects and two scope rows × ten templates" |
| `fixtures.rs:2491` loop over `PROJECT_SPECS` (5 kinds each) | — | unchanged; add: each `SCOPE_SPECS` row carries exactly one kind, `WS` | |
| `fixtures.rs:2516` (bodies, per `PROJECT_SPECS`) | — | unchanged; extend the per-owner loop to `SCOPE_SPECS` (ten each, in order) | |
| `data.kinds.len()` (implicit) | 15 | 17 | |
| `data.edges.len()` (`migrations.rs:4940-4944`, T3) | 42 | **46** | 3×14 + 2×2 |
| `data.projects.len()` (`migrations.rs:4949`, delta) | 3 | 5 | computed, no literal |

`htui/tests/kinds.rs:892-894` (14, per project) and `conformance.rs:25299` (fresh project) do not move.

**Build order (T2)**: seed + traits sentences unit tests (red) → model types/fields → seed consts → trait
methods + doc edits → htui-agent delegations → conformance cases 1-13 + pin (red on Mem) → fixture ids,
`SCOPE_SPECS`, `projects()`, `catalogue()` + moved counts → MemStore (`seed_project` extraction first, then
create/update workspace, refusals, owner documents, delete cascade, reach) → `mem.rs` listing test →
`cargo test -p htui-core --all-features -- --test-threads=1` and `cargo check -p htui-agent -p htui-orch
--all-features --tests`.

---

## T3 — htui-store (Postgres and cache)

Files: the plan's T3 set **plus** `src/backend.rs`, `src/writer.rs`, `src/testkit.rs` (V2). `cache/mod.rs` needs
no change.

1. **`seed_project`** (`pg/write.rs:9588`) → `async fn seed_project(tx: &mut PgConnection, project_id:
   ProjectId, created_by: UserId, kinds: &[seed::KindSeed]) -> Result<()>`; loop `kinds.iter().enumerate()`;
   templates loop unchanged. `create_project` (`:3177`) passes `&seed::KINDS`. Statement text unchanged → `.sqlx`
   entries reused.
2. **Scope-row insert** (new, `pg/write.rs`, `pub(crate)` for the bootstrap):
   ```rust
   /// MOD-147: the workspace's scope row and its `WS` seed, on `conn`'s transaction. `Ok(None)` when the
   /// workspace already has one (`ON CONFLICT (owner_workspace_id) DO NOTHING`), so the bootstrap is idempotent.
   pub(crate) async fn insert_scope_row(conn: &mut PgConnection, workspace: WorkspaceId,
       workspace_slug: &str, name: &str, created_by: UserId) -> Result<Option<ProjectId>>
   ```
   `INSERT INTO project (id, slug, name, description, created_by, type, owner_workspace_id) VALUES ($1, $2, $3,
   '', $4, $5, $6) ON CONFLICT (owner_workspace_id) DO NOTHING RETURNING id AS "id: ProjectId"` with
   `ProjectId::new()`, `seed::scope_row_slug(workspace_slug)`, `seed::SCOPE_ROW_TYPE`; on `Some(id)` →
   `seed_project(conn, id, created_by, std::slice::from_ref(&seed::WORKSPACE_KIND))`.
3. **`create_workspace`** (`:2953`) → `pool.begin()`, the same `INSERT … RETURNING` on `&mut *tx` (text
   unchanged), `insert_scope_row(&mut tx, row.id, &row.slug, &row.name, row.created_by)`, commit.
   **`update_workspace`** (`:2990`) → tx; same `UPDATE … RETURNING` on the tx; on `Some(row)` and (`patch.slug`
   or `patch.name`) `UPDATE project SET slug = $2, name = $3 WHERE owner_workspace_id = $1`; commit; `None` →
   drop tx, `cas_miss` as today.
4. **`upsert_workspace_project`** (`:3046`) → pre-read `SELECT slug FROM project WHERE id = $1 AND
   owner_workspace_id IS NOT NULL` (race-free: `owner_workspace_id` is never updated); `Some(slug)` →
   `Constraint(scope_row_joins_no_workspace(&slug))`; else the upsert as today.
5. **`create_project`/`update_project`** (`:3147`, `:3198`) → `project_slug_refusal` first (before the tx / the
   statement); both `RETURNING` lists gain `owner_workspace_id AS "owner_workspace_id: WorkspaceId"`; a scope
   row slug patch hits `chk_project_scope_slug` (`23514` → `Constraint`).
6. **`delete_workspace_once`** (`:9898`) → after the held-graph refusal, on the same repeatable-read tx:
   ```sql
   SELECT w.slug AS "slug!",
          (SELECT count(*) FROM item i     WHERE i.project_id = p.id)                      AS "items!",
          (SELECT count(*) FROM document d WHERE d.project_id = p.id AND d.item_id IS NULL) AS "documents!",
          (SELECT count(*) FROM run r      WHERE r.project_id = p.id)                      AS "runs!"
     FROM project p JOIN workspace w ON w.id = p.owner_workspace_id
    WHERE p.owner_workspace_id = $1
   ```
   any non-zero → `Constraint(workspace_owns_work(..))`; the `DELETE FROM workspace` then cascades the scope row
   (verified). A row committed after the count raises `40001` on the cascade → retried → refused.
7. **Documents** — `insert_document` (`:395`): column list `(id, project_id, item_id, …)`, select list
   `$1, (SELECT project_id FROM item WHERE id = $2), $2, …`, `RETURNING item_id AS "item_id!: ItemId"`.
   `pg/read.rs:309` and `:556` → `"item_id!: ItemId"`; `:525` → `"item_id!: ItemId"` **and** `:533` `WHERE id = $1
   AND item_id IS NOT NULL` (V5). `project_reach` `:581-582` → `WHERE project_id = $1` (V7).
   **`write_owner_document`**: tx; `SELECT 1 FROM project WHERE id = $1 FOR NO KEY UPDATE` → `NotFound { entity:
   "project" }`; `INSERT INTO document (id, project_id, item_id, kind, version, title, body, produced_by_step_id,
   created_by, created_at) SELECT $1, $2, NULL, $3, COALESCE(MAX(version), 0) + 1, $4, $5, NULL, $6, $7 FROM
   document WHERE project_id = $2 AND item_id IS NULL AND kind = $3 RETURNING id AS "id: DocumentId",
   project_id AS "project_id: ProjectId", kind, version, title, body, produced_by_step_id AS
   "produced_by_step_id: StepId", created_by AS "created_by: UserId", created_at`; commit.
   **`owner_documents`**: same columns `FROM document WHERE project_id = $1 AND item_id IS NULL ORDER BY kind
   COLLATE "C", version`. **`scope_project`**: `SELECT id AS "id: ProjectId" FROM project WHERE
   owner_workspace_id = $1`.
8. **`project()`** (`pg/read.rs:685-698`) gains `owner_workspace_id AS "owner_workspace_id: WorkspaceId"`;
   `workspace_box_path_rows` (`:2875`) and `upsert_workspace_box_path` (`pg/write.rs:3109-3114`) → `folder_path`.
9. **Demo loader** (`pg/demo.rs`): project insert (`:114-130`) gains `owner_workspace_id` (`$12`,
   `row.owner_workspace_id.map(WorkspaceId::as_uuid)`); document insert (`:679-697`) → `INSERT INTO document (id,
   project_id, item_id, kind, version, title, body, produced_by_step_id, created_by, created_at) VALUES ($1,
   (SELECT project_id FROM item WHERE id = $2), $2, $3, $4, $5, $6, $7, $8, $9)` (V6). Scope rows, their
   kinds, graphs, phases, edges and templates arrive through `data.*` with no other loader change.
10. **Bootstrap** (`pg/mod.rs:462`, inside the `app_user`-locked tx, after personas): `SELECT w.id AS "id:
    WorkspaceId", w.slug, w.name, w.created_by AS "created_by: UserId" FROM workspace w WHERE NOT EXISTS (SELECT 1
    FROM project p WHERE p.owner_workspace_id = w.id) ORDER BY w.id`, then `write::insert_scope_row` each.
11. **Cache** — `refresh.rs`: `PROJECT_COLUMNS` (`:684`) + `"owner_workspace_id"`, select + bind
    `row.owner_workspace_id.map(|id| id.to_string())`; `DOCUMENT_COLUMNS` (`:1047`) + `"project_id"`;
    `refresh_document` (`:1067-1092`) `SELECT id, project_id, item_id, …  WHERE project_id = $1 AND created_at >
    $2`, bind `row.project_id.to_string()`, `row.item_id.map(|id| id.to_string())`. `cache/read.rs`: `project()`
    (`:946-968`) selects and maps `owner_workspace_id` (`opt_uuid_col`); `document()` (`:831`) `AND item_id IS
    NOT NULL`; new `scope_project` (`SELECT id FROM project WHERE owner_workspace_id = ?`) and `owner_documents`
    (`… WHERE project_id = ? AND item_id IS NULL ORDER BY kind, version`, BINARY collation = bytes).
    `testkit.rs:537-553` binds `owner_workspace_id`.
12. **Backend / Writer** delegations (V2).
13. **Tests**: `pg_conformance.rs:50-51` → 246; `pg_criteria.rs:2828` rename; new
    `pg_criteria.rs::workspaces_and_projects_never_list_a_scope_row` (V3) and
    `owner_document_versions_do_not_collide_under_contention` (the `:5098` pattern on `goals`);
    `migrations.rs::bootstrap_gives_every_existing_workspace_one_scope_row` (`bare_db`, `run_to(25)`, two
    workspaces raw, `MIGRATOR.run`, a connected store's `seed_if_empty_as` twice (the `:4640-4707` connect
    pattern) → exactly one scope row each, `@slug`, one `WS` kind, ten templates); `migrations.rs:4940-4944` →
    46 "fourteen per project, two per scope row"; `tests/cache.rs:1682` + `project_id` (`ids::PROJECT_HTUI`),
    `:1753` + `project_id`; new `a_scope_row_mirrors_its_kinds_items_and_owner_documents` (mint `WS-1` and an owner
    doc on `PROJECT_SCOPE_PLATFORM`, `run_pass` with `all_projects()` + the scope row; cache `scope_project`,
    `project().owner_workspace_id`, `owner_documents` == Postgres's, `items` of the scope row, `document(owner)`
    `None`, `workspaces()` Platform excludes it). Then `cargo sqlx prepare` against a migrated scratch DB.

**Build order (T3)**: `pg_conformance` pin and the new Pg/cache/migration tests (red) → Pg reads/writes 1-10 →
cache 11 → delegations 12 → `cargo sqlx prepare` → `cargo test -p htui-store --all-features -- --test-threads=1`.

---

## T4 — htui (compiles only after T3, V9)

1. **`store_worker.rs:3036-3038`**: a loop-local `let mut scope_rows: HashMap<WorkspaceId, ProjectId> =
   HashMap::new();` beside `projects`; on `StoreRequest::Items { scope, .. }`: `let mut ids =
   scope.project_ids.clone();` then the memoised row — `match scope_rows.get(&scope.workspace_id)`, else
   `backend.scope_project(scope.workspace_id).await` (`Ok(Some)` cached, `Ok(None)` not cached — the mirror may
   not hold it yet, `Err` → `tracing::warn!` and skip) — pushed onto `ids`; `projects.send_replace(ids)`.
2. **`hierarchy.rs`**: `HierarchySnapshot.root_path` → `folder_path: Option<WorkspaceBoxPath>` (this box's row,
   as today); `snapshot` (`:187-217`) and `SetWorkspaceRoot` (`:281-289`) use `folder_path`.
   ```rust
   /// MOD-147 D8: this box's folder for the tree's workspace — the row's `folder_path`, else
   /// `<home>/workspaces/<slug>`; `None` only with no row and no home.
   #[must_use]
   pub fn workspace_root(tree: &HierarchySnapshot, home: Option<&Path>) -> Option<PathBuf>

   /// [`serve`] with the home directory a test chooses (V10); `serve` passes `dirs::home_dir()`.
   pub async fn serve_with_home(backend: &Backend, request: &StoreRequest, home: Option<&Path>) -> Result<StoreReply>
   ```
   `infer(backend, writer, ws, this_box, home)` (`:560`): `workspace_root(&tree, home)`; `None` → `root: None` as
   today; a row whose path no longer resolves → `Constraint` as today (`:580`); the **default** folder not
   resolving → `root: None`, nothing walked (an absent default is not an error).
   `workspace_of` (`:537`): membership first as today; on no match, `backend.project(project).await?
   .and_then(|row| row.owner_workspace_id)`; still `NotFound { entity: "workspace_project" }` otherwise.
3. **`ui/tabs/settings/hierarchy.rs`** `:456-459`, `:820-823`, `:1303`, `:1325-1327`: field renames only
   (`snapshot.folder_path`, `path.folder_path`); the visible "root on this box: … / unset" text stays (M3's).
4. **`tests/hierarchy.rs`**: renames `:72`, `:359-360`, `:1147`, `:2042-2045`, `:2963`; the `infer` helper
   (`:973`) calls `serve_with_home(.., None)`; new `infer_defaults_to_the_workspace_folder_under_home` (tempdir
   home, `<home>/workspaces/graphics/<repo>` checkout matching a Graphics repo, no row → report `root` is that
   folder canonical and the repo inferred), `infer_without_a_row_or_default_folder_walks_nothing`,
   `a_scope_row_resolves_its_workspace` (`SetProjectSecretScope { id: ids::PROJECT_SCOPE_GRAPHICS, .. }` answers
   `SecretScopeWritten` with Graphics' tree), and `the_tree_never_lists_a_scope_row` (Hierarchy(PLATFORM)
   projects == HTUI, AGY).

**Build order (T4)**: tests (red) → hierarchy.rs rename + `workspace_root`/`serve_with_home`/`infer`/`workspace_of`
→ settings rename → store_worker → `cargo test -p htui --all-features --test hierarchy -- --test-threads=1`, then a
full insta run (no snapshot should move).

---

## Compile and merge order

T1 (raw SQL only) and T2 run in parallel; T1 verifies on the pre-T2 tree. After T2: htui-core, htui-agent,
htui-orch compile; htui-store and everything above it do not (`WorkspaceBoxPath.folder_path`, `Project` field,
three trait methods). T3 restores htui-store (PgStore, CacheStore, Backend, Writer, demo, testkit, `.sqlx`);
only then does htui (and htui-worker, which depends on htui-store) compile, so T4 is written beside T3 and
verified after it. Final gate: `cargo fmt --all --check`, both clippy runs, `bash scripts/gate.sh`.

## Open for the maintainer

- V11: D6 makes a workspace that ever owned scope-row work undeletable; confirm the refusal sentence
  ("…its work would be deleted with it, so it is kept") or name the escape (e.g. a future item delete/move).
- V12: should `delete_project` refuse a scope row ("delete the workspace instead")? Unreachable in M1's UI.
- ANA-35 §7.5 reserves `summary` for item close-out; `write_owner_document` does not refuse it in M1.
