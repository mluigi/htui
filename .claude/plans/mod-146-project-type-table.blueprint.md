# Blueprint: MOD-146, project type table

**Status**: **proposed** (2026-10-21). Findings F-1 to F-8 (§0) are this blueprint's. **F-1 is a
Blocker that needs a maintainer nod** (its default is below). Everything else follows from the
plan's settled D1-D8 and the OQ defaults, which this blueprint does not reopen.

**Plan**: `.claude/plans/mod-146-project-type-table.plan.md` at `5d4ef2c8`, confirmed 2026-10-20.
**Design source**: `docs/ANA-35.md` §7.4 (`:904-930`) and §7.9 P3 (`:1092-1110`).

**Verified at**: HEAD `5d4ef2c8`, branch `hr/MOD-146`. Anchors were found through Gortex
(`search_text`, `read_file`, `get_symbol_source`) and re-read at their lines. **Line numbers are
pre-edit.** `graphify-out/` does not exist, so nothing here comes from it.
`crates/htui-store/.sqlx/` holds 403 files, `crates/htui/tests/snapshots/` holds 171.

**Scope**:
- **Order**: T1 → T2 → T3, serial, one tree (T4 is bookkeeping and not blueprinted).
- **Migrations**: `0020_project_type.sql` (Postgres), `cache_migrations/0007_project_type.sql`.
- **One new `WriteStore` method**: `project_types`, on all five implementors.
- **New modules**: `htui::ui::tabs::settings::project_types`; no new worker module (the read is
  one inline `try_serve` arm, the `AnswerPermission`/`FollowUp` precedent at
  `store_worker.rs:2317-2390`).
- **Request enums**: `StoreRequest` +1 (`ProjectTypes`), `StoreReply` +1 (`ProjectTypes`).
- **Pins that move**: store `CASES` 175 → 177 (two pins); `TABLES` 46 → 47; migration versions
  1..19 → 1..20; `.sqlx` 403 → 404 (5 replaced, 1 new); `Context::ALL` 48 → 49; `DECLARED` 82 → 83;
  Settings strip 10 sections / 87 columns → 11 / 102 (F-1); snapshots 171 → 173 (two new, none
  changed, F-2).

**House style (carried)**: `missing_docs` warns on every lib, so every new `pub` item has a doc.
Workspace lints are not edited. Implementers stage their own paths (never `-A`, never `stash`,
never `--amend`) and commit incrementally. Gates run on the real tree with `--test-threads=1`.

---

## 0. Findings the plan's fact-check missed

| # | Blocker? | Plan says | Tree at `5d4ef2c8` | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (the `htui` gate) | D8/OQ-4: appended after Queue as **Project types**. | `tests/settings.rs:1095` `the_section_strip_fits_the_frame` pins Σ(title + 2) ≤ 100 over the product's sections. Today 87; `"Project types"` adds 15 → **102**. `render_strip` (`settings/mod.rs:503-518`) is a plain unwrapped `Paragraph`, so at 100 columns (98 inside the border) the strip clips to `" Project ty"`. The test's own doc: "if an eleventh ever does not fit, the fix is the strip's, not a shorter title". | **Default (A):** the strip scrolls so that the active title always ends inside the area (§3.6). No committed snapshot moves: each draws a partial strip of ≤ 87 columns. **Alternative (B):** strip title `"Types"` (strip 94), heading stays `"Project types"`; D8's label changes and "Types" sits ambiguously next to "Kinds". Maintainer picks; the implementer builds A unless told B. |
| **F-2** | Non-blocker (plan risk row overstated) | T3: "every full-frame Settings strip snapshot gains ' Project types ', and the help box gains a context". | No committed `.snap` holds the product strip (`git grep -F 'Secrets  Queue' -- '*.snap'` is empty). Snapshots build a partial strip with `SettingsTab::with_sections`. No `.snap` pins the `?` box or `--print-keys`. | Expect **0** changed and **2** new snapshots (`project_types__browse`, `project_types__offline`). Still run the full `cargo insta test` (memory: grepping undercounts). |
| **F-3** | **Blocker** (T1's gate as written) | T1 Validate: `cargo clippy --workspace …`. | A new field on `Project` breaks the three `query_as!(Project …)` sites and the `cache/read.rs:907` literal. A new required trait method breaks `PgStore` and `Writer`. So `htui-store` (and `htui`, which depends on it) do not compile until T2. `htui-agent` and `htui-orch` depend on `htui-core` only (their `Cargo.toml`s), so they do compile. | T1's gate is crate-scoped (§1.8). T1's commit says the workspace is green again at T2's last commit. T1 and T2 run back to back in one lane. |
| **F-4** | Non-blocker (pins the plan's file table omits) | Files table, T1/T2/T3. | These also move: `htui-core/tests/mem_store.rs:36` (175); `htui-store/tests/pg_conformance.rs:39` `EXPECTED_CASES` (175, plus its doc and message); `tests/migrations.rs:87` the applied-version list and `TABLES.len()` 46; `the_ana_column_comments_are_present_and_verbatim` (three `.chain`s and the table-comment loop); `keys/stack.rs:226` `DECLARED` and `:423-424` (`len 82`, `[81]` = leave); `keys/load.rs:539-547` `TABLES`; `tests/keys_file.rs:244-252` (the same table list); `keys/views.rs` `VIEWS`, `VIEW_STACKS`, `DELEGATES`; `Context::is_view` (`catalogue.rs:343`, a `matches!` the compiler will **not** flag). | Each is listed at its task below. |
| **F-5** | Non-blocker (a vacuous test) | T2: "`tests/cache.rs` `projects_agree` carries `project_type`". | `projects_agree` (`cache.rs:306`) already compares whole `Project`s, so it needs no edit. Over the demo it only proves `software == software`, which a mirror that drops the column also passes, because of the column default. | New `the_0007_project_type_reaches_the_mirror` writes `research` by raw SQL and reads it off the mirror (§2.7). |
| **F-6** | Non-blocker (fidelity) | Plan: check `pg/demo.rs`. | `load_demo` (`demo.rs:115-131`) and `testkit::seed_mirror` (`testkit.rs:537-556`) both list the `project` columns explicitly. Without `type` they still work through the defaults, but a future fixture project with another type (MOD-148's per-type strides) would land as `software` without any error. | Both write `type` (§2.4). That replaces one more `.sqlx` file (demo). The demo's three projects are all `software` (ANA-35 §7.4: existing projects become `software`). |
| **F-7** | Non-blocker (a deviation from "verbatim") | D1: "ANA-35 §7.9's P3 DDL verbatim". | The sketch's two CHECKs and its FK are unnamed, so Postgres would name them `project_type_slug_check`, `project_type_check` and `project_type_fkey`. House style names every constraint (`0019`: `fk_run_workspace`, `chk_workspace_stop_mode`), and `assert_refused_by` matches on the name. `0012` gives every `updated_at` table its trigger. | Columns, types, defaults and the CHECK bodies are verbatim. The constraints are named `chk_project_type_slug`, `chk_project_type_seed` and `fk_project_type`. Add `trg_project_type_updated_at`. |
| **F-8** | Non-blocker (a scanner trap) | D7: conformance cases. | `every_cross_referenced_test_name_exists` (`conformance.rs:~21333`) requires every bare backticked snake_case name with four or more underscores in this module's docs to be defined in `conformance.rs` or `mem.rs`, and it **panics** on a `<file>.rs::<name>` whose file is not `conformance`, `mem` or `pg_criteria`. | Case docs name no `tests/migrations.rs` test. Write "the Postgres pin in `tests/migrations.rs`" without the test's name. |

---

## 1. T1: model, built-ins, `WriteStore::project_types`, MemStore, conformance

### 1.1 `crates/htui-core/src/model/hierarchy.rs`

Insert after `Project` (`:56-78`). The derives match `Project`'s: no `Eq`, because `Project` has
none. No `sqlx::FromRow` is needed. `query_as!` maps columns to fields itself. `persona_rows` goes
through `PersonaRow` only because of its `Json<…>` columns, and `ProjectType` has none (its JSONB
is a plain `Value`, as `Project.settings` is).

```rust
/// A row of `project_type` (MOD-146 D2, ANA-35 §7.4): what a project is, which seeds it once at
/// creation. Editing or adding a type never re-seeds an existing project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectType {
    /// `project_type.slug`, the key `project.type` references.
    pub slug: String,
    /// `project_type.name`, e.g. `Software`.
    pub name: String,
    /// `project_type.description`.
    pub description: String,
    /// `project_type.coding`: a software-like type, which ANA-39 and MOD-128 read.
    pub coding: bool,
    /// `project_type.builtin`: one of [`crate::seed::PROJECT_TYPES`].
    pub builtin: bool,
    /// `project_type.selectable`: offered for a new project; `false` for `workspace`.
    pub selectable: bool,
    /// `project_type.seed`: a user-defined type's recipe; `None` for a built-in, whose recipe code
    /// holds.
    pub seed: Option<Value>,
    /// `project_type.seed_version`.
    pub seed_version: i32,
    /// `project_type.created_by`; `None` for a built-in.
    pub created_by: Option<UserId>,
    /// `project_type.created_at`.
    pub created_at: DateTime<Utc>,
    /// `project_type.updated_at`.
    pub updated_at: DateTime<Utc>,
}
```

In `Project`, after `description` (`:65`):

```rust
    /// `project.type`: the slug of the [`ProjectType`] the project was created as (MOD-146 D2).
    /// `software` for every project until MOD-148's picker chooses one.
    pub project_type: String,
```

`model/mod.rs:121-124`: `pub use hierarchy::{ …, Project, ProjectPatch, ProjectRef, ProjectType,
Repo, … }`.

### 1.2 `crates/htui-core/src/seed.rs`

Extend the import (`:22-25`) with `ProjectType`. Append after `template_row`, before `mod tests`:

```rust
/// One built-in `project_type` row (MOD-146 D3): the five columns the six vary. Every built-in is
/// `builtin`, has no `seed` (code holds its recipe, ANA-35 §7.4), `seed_version` 1 and no author;
/// [`project_type_row`] writes those.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinProjectType {
    /// `project_type.slug`; passes `chk_project_type_slug`.
    pub slug: &'static str,
    /// `project_type.name`.
    pub name: &'static str,
    /// `project_type.description`.
    pub description: &'static str,
    /// `project_type.coding`.
    pub coding: bool,
    /// `project_type.selectable`.
    pub selectable: bool,
}

/// The type every project takes until MOD-148's picker chooses one (MOD-146 D4): the column
/// default of `project.type` in `0020_project_type.sql`.
pub const DEFAULT_PROJECT_TYPE: &str = "software";

/// ANA-35 §7.4's six built-in types, in `slug` byte order, which is the order
/// [`crate::store::WriteStore::project_types`] answers (MOD-146 D5). `0020_project_type.sql`
/// inserts the same six, and `htui-store`'s `tests/migrations.rs` pins the two equal.
pub const PROJECT_TYPES: [BuiltinProjectType; 6] = [
    BuiltinProjectType { slug: "docs", name: "Documents",
        description: "Documents outlined, drafted and reviewed.", coding: false, selectable: true },
    BuiltinProjectType { slug: "general", name: "General",
        description: "Tasks carried out and reviewed.", coding: false, selectable: true },
    BuiltinProjectType { slug: "ops", name: "Operations",
        description: "Operations planned, executed and verified; incidents investigated.",
        coding: false, selectable: true },
    BuiltinProjectType { slug: "research", name: "Research",
        description: "Questions answered and research concluded with a verdict.",
        coding: false, selectable: true },
    BuiltinProjectType { slug: "software", name: "Software",
        description: "Code in repositories: analyses, features, fixes.", coding: true,
        selectable: true },
    BuiltinProjectType { slug: "workspace", name: "Workspace",
        description: "A workspace's own items; not offered for new projects.", coding: false,
        selectable: false },
];

/// A built-in's `project_type` row. `at` fills both stamps: `MemStore` passes the epoch, and
/// Postgres's rows carry the migration's `now()`.
#[must_use]
pub fn project_type_row(builtin: &BuiltinProjectType, at: DateTime<Utc>) -> ProjectType {
    ProjectType {
        slug: builtin.slug.to_owned(),
        name: builtin.name.to_owned(),
        description: builtin.description.to_owned(),
        coding: builtin.coding,
        builtin: true,
        selectable: builtin.selectable,
        seed: None,
        seed_version: 1,
        created_by: None,
        created_at: at,
        updated_at: at,
    }
}
```

(Let `cargo fmt` lay out the literals.) **Unit tests** in `seed::tests`:
- `six_project_types_in_slug_byte_order_pass_the_check`: `len == 6`; `windows(2)` strictly
  increasing by bytes (so the slugs are unique, and the order is the read order); every slug is
  1-32 bytes, its first byte is `a..=z`, and the rest are `a..=z | 0..=9 | -` (a hand check, no
  `regex` dep); names and descriptions are non-blank.
- `software_is_the_one_coding_default_and_workspace_the_one_hidden`: the slugs with `coding` are
  `["software"]`; the slugs with `!selectable` are `["workspace"]`; `DEFAULT_PROJECT_TYPE` names a
  row that is `selectable` and `coding`.
- `project_type_row_carries_the_builtin_columns`: for `PROJECT_TYPES[0]` at `UNIX_EPOCH`,
  `builtin`, `seed == None`, `seed_version == 1`, `created_by == None`, both stamps `at`.

### 1.3 `crates/htui-core/src/store/traits.rs`

Add `ProjectType` to the model import (`:33-…`). After `delete_persona` (`:1102`):

```rust
    // project_type (MOD-146, plan D5)
    //
    // A global registry like `persona`, read here for the same reason: it is not mirrored, so the
    // read is online-only and an offline caller is refused. No writer ships with it (ANA-33's
    // editor owns user-defined types); both stores answer `seed::PROJECT_TYPES`' six built-ins.

    /// Every project type, ordered by `slug` bytes (`COLLATE "C"`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn project_types(&self) -> Result<Vec<ProjectType>>;
```

### 1.4 `crates/htui-core/src/store/mem.rs`

- In `State::create_project`'s literal (`:3031-3042`), after `description`:
  `project_type: seed::DEFAULT_PROJECT_TYPE.to_owned(),`. `update_project` clones the row, so the
  type survives an edit.
- In `impl WriteStore for MemStore`, after `delete_persona` (`:8752`), with `ProjectType` added to
  the model import:

```rust
    async fn project_types(&self) -> Result<Vec<ProjectType>> {
        Ok(seed::PROJECT_TYPES
            .iter()
            .map(|builtin| seed::project_type_row(builtin, DateTime::UNIX_EPOCH))
            .collect())
    }
```

The epoch keeps the rows identical across calls and handles. No state is read: no writer exists.

### 1.5 The other literals and delegates

- `fixtures.rs:733-747`: `project_type: crate::seed::DEFAULT_PROJECT_TYPE.to_owned(),`. All three
  demo projects are `software`.
- `secret.rs:1380` (test) and `htui-orch/src/secrets.rs:214` (test): the same field, through
  `htui_core::seed::DEFAULT_PROJECT_TYPE` in the orch crate.
- `htui-agent/src/conformance.rs` `UsageSpy`, after `delete_persona` (`:1060`), and
  `htui-agent/tests/recorder.rs` `SpyStore`, after `:787`:
  ```rust
      async fn project_types(&self) -> StoreResult<Vec<htui_core::model::ProjectType>> {
          self.inner.project_types().await
      }
  ```

### 1.6 Conformance (`crates/htui-core/src/store/conformance.rs`)

- `CASES` (`:61-237`): append after `"a_deleted_workspace_unscopes_its_runs_and_drops_its_stop"`
  (`:236`): `"project_types_are_the_six_builtins",` then `"a_new_project_is_software",`.
- `run_case`: two arms before `other =>` (`:639`), in the `{ f(store).await; }` style of `:636-638`.
- Imports: `use crate::seed::{DEFAULT_PROJECT_TYPE, PROJECT_TYPES};`. No model import is needed.
- The bodies go at the end of the case code, before `#[cfg(test)] mod tests` (`:21308`), under
  `// MOD-146: the project type registry (plan D3-D5, D7)`. Mind F-8 in their docs.

```rust
/// MOD-146 plan D3, D5: the registry is ANA-35 §7.4's six built-ins in slug byte order, each as
/// `seed::PROJECT_TYPES` spells it: `coding` on `software` alone, `selectable` false on
/// `workspace` alone, every row built in with no recipe, `seed_version` 1, no author and never
/// edited. The Postgres pin in `tests/migrations.rs` checks the migration's rows against the
/// same constant.
async fn project_types_are_the_six_builtins<S: WriteStore>(store: &S) {
    const CASE: &str = "project_types_are_the_six_builtins";
    let rows = store.project_types().await.expect(CASE);
    assert_eq!(
        rows.iter().map(|row| row.slug.as_str()).collect::<Vec<_>>(),
        ["docs", "general", "ops", "research", "software", "workspace"],
        "{CASE}: six rows in slug byte order"
    );
    for (row, builtin) in rows.iter().zip(PROJECT_TYPES.iter()) {
        assert_eq!(
            (row.slug.as_str(), row.name.as_str(), row.description.as_str(), row.coding,
             row.selectable),
            (builtin.slug, builtin.name, builtin.description, builtin.coding, builtin.selectable),
            "{CASE}: `{}` is the built-in as seeded", builtin.slug
        );
        assert!(row.builtin, "{CASE}: `{}` is built in", row.slug);
        assert_eq!((&row.seed, row.seed_version, row.created_by), (&None, 1, None),
            "{CASE}: `{}` carries no recipe, version 1, no author", row.slug);
        assert_eq!(row.created_at, row.updated_at, "{CASE}: `{}` was never edited", row.slug);
    }
    assert_eq!(
        rows.iter().filter(|row| row.coding).map(|row| row.slug.as_str()).collect::<Vec<_>>(),
        ["software"], "{CASE}: software is the one coding type");
    assert_eq!(
        rows.iter().filter(|row| !row.selectable).map(|row| row.slug.as_str())
            .collect::<Vec<_>>(),
        ["workspace"], "{CASE}: workspace is the one type a new project is not offered");
    assert_eq!(store.project_types().await.expect(CASE), rows, "{CASE}: a second read agrees");
}

/// MOD-146 plan D4: `NewProject` has no type, so a created project is `DEFAULT_PROJECT_TYPE`, it
/// reads back so, and an edit leaves it; the demo projects, which existed before the column, are
/// `software` too.
async fn a_new_project_is_software<S: WriteStore>(store: &S) {
    const CASE: &str = "a_new_project_is_software";
    for id in [ids::PROJECT_HTUI, ids::PROJECT_AGY, ids::PROJECT_VULKAN] {
        let project = store.project(id).await.expect(CASE).expect(CASE);
        assert_eq!(project.project_type, DEFAULT_PROJECT_TYPE, "{CASE}: demo project {id}");
    }
    let created = store.create_project(new_project("typed")).await.expect(CASE);
    assert_eq!(created.project_type, DEFAULT_PROJECT_TYPE, "{CASE}: the create answers it");
    assert_eq!(
        store.project(created.id).await.expect(CASE).map(|row| row.project_type),
        Some(DEFAULT_PROJECT_TYPE.to_owned()), "{CASE}: it reads back");
    let patch = ProjectPatch { name: Some("Typed".to_owned()), ..ProjectPatch::default() };
    let edited = applied(CASE, store
        .update_project(created.id, created.updated_at, patch).await.expect(CASE));
    assert_eq!(edited.project_type, DEFAULT_PROJECT_TYPE, "{CASE}: an edit leaves the type");
    let default = store.project_types().await.expect(CASE).into_iter()
        .find(|row| row.slug == DEFAULT_PROJECT_TYPE)
        .unwrap_or_else(|| panic!("{CASE}: the default names a row"));
    assert!(default.selectable && default.coding, "{CASE}: the default is selectable, coding");
}
```

(`applied` and `new_project` exist: `:2714`, and `applied` is used at `:3093`. The demo
project ids are `fixtures.rs:715-726`.)

**Count pins**:
- `htui-core/tests/mem_store.rs:36`: `175` → `177`; append to the message: `, and MOD-146's two
  for the project type registry (plan D7)`.
- `htui-store/tests/pg_conformance.rs:39`: `EXPECTED_CASES = 177`. Add a doc line,
  `/// and MOD-146's two project type cases (plan D7) make it 177.`, and the message `177 since …
  and MOD-146's two project type cases`. `pg_store_conformance` loops over `CASES`
  (`pg_conformance.rs:58`), so its Pg wiring needs **no other edit**.

### 1.7 Commit
`feat(core): MOD-146 T1 project types, built-ins and WriteStore::project_types`. Stage the
htui-core, htui-agent and htui-orch paths, plus `htui-store/tests/pg_conformance.rs`. The body says:
"htui-store and htui compile again at T2" (F-3).

### 1.8 T1 gate (F-3)
```bash
cargo test -p htui-core --all-features -- --test-threads=1
cargo clippy -p htui-core -p htui-agent -p htui-orch --all-targets --all-features -- -D warnings
cargo clippy -p htui-core -p htui-agent -p htui-orch -- -D warnings
```

---

## 2. T2: Postgres migration, PgStore, Writer, mirror

### 2.1 `crates/htui-store/migrations/0020_project_type.sql`

```sql
-- 0020_project_type.sql - MOD-146 (ANA-35 7.4, 7.9 P3; plan D1). Forward-only (R-STO-5).
--
-- project_type is the registry of project types ANA-35 7.4 settles on. A project's type is set at
-- creation and its seed is copied into the project once, so editing or adding a type never
-- re-seeds an existing project. The six built-in rows carry no recipe (seed NULL): code holds
-- theirs, so ANA-33 can change the graph format; a user-defined type (ANA-33, R-TUI-8) must
-- carry one (chk_project_type_seed). coding marks the software-like types ANA-39 and MOD-128
-- read; selectable is false for workspace, which only MOD-147's scope rows take. MOD-146 adds no
-- write path: the INSERT below is the table's only writer.
--
-- project.type references the registry. Every existing project takes the column default
-- 'software', and so does every project created until MOD-148's picker sets it. No index stands
-- behind fk_project_type: projects are few and no type is ever deleted. The mirror gains
-- project.type in cache_migrations/0007_project_type.sql; project_type itself is not mirrored,
-- like persona. schema_version becomes 20, so each box rebuilds its mirror once on first start. A
-- headless worker never migrates: migrate from a TUI first.

CREATE TABLE project_type (
    slug         TEXT        PRIMARY KEY
                             CONSTRAINT chk_project_type_slug CHECK (slug ~ '^[a-z][a-z0-9-]{0,31}$'),
    name         TEXT        NOT NULL,
    description  TEXT        NOT NULL DEFAULT '',
    coding       BOOLEAN     NOT NULL DEFAULT false,
    builtin      BOOLEAN     NOT NULL DEFAULT false,
    selectable   BOOLEAN     NOT NULL DEFAULT true,
    seed         JSONB,
    seed_version INTEGER     NOT NULL DEFAULT 1,
    created_by   UUID        REFERENCES app_user(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_project_type_seed CHECK (builtin OR seed IS NOT NULL)
);

-- 0001's set_updated_at, BEFORE UPDATE only (0012_persona.sql's shape).
CREATE TRIGGER trg_project_type_updated_at BEFORE UPDATE ON project_type
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The six built-ins, seed::PROJECT_TYPES verbatim (tests/migrations.rs pins the two equal).
INSERT INTO project_type (slug, name, description, coding, builtin, selectable) VALUES
    ('docs',      'Documents',  'Documents outlined, drafted and reviewed.',                           false, true, true),
    ('general',   'General',    'Tasks carried out and reviewed.',                                     false, true, true),
    ('ops',       'Operations', 'Operations planned, executed and verified; incidents investigated.', false, true, true),
    ('research',  'Research',   'Questions answered and research concluded with a verdict.',          false, true, true),
    ('software',  'Software',   'Code in repositories: analyses, features, fixes.',                   true,  true, true),
    ('workspace', 'Workspace',  'A workspace''s own items; not offered for new projects.',            false, true, false);

ALTER TABLE project
    ADD COLUMN type TEXT NOT NULL DEFAULT 'software'
        CONSTRAINT fk_project_type REFERENCES project_type(slug);

COMMENT ON TABLE project_type IS
    'MOD-146 D1 (ANA-35 7.4): the project type registry. A type seeds a project once, at '
    'creation; editing or adding a type never re-seeds one. Six built-in rows; not mirrored.';
COMMENT ON COLUMN project_type.slug IS
    'MOD-146 D1: the key project.type references; a letter, then up to 31 of a-z, 0-9 and '
    'hyphens (chk_project_type_slug).';
COMMENT ON COLUMN project_type.coding IS
    'MOD-146 D1: a software-like type (software alone among the built-ins); ANA-39 and MOD-128 '
    'read it.';
COMMENT ON COLUMN project_type.selectable IS
    'MOD-146 D1: false for workspace, which only scope rows take (MOD-147); a new-project picker '
    'offers the rest.';
COMMENT ON COLUMN project_type.seed IS
    'MOD-146 D1: the JSONB recipe of a user-defined type; NULL for a built-in, whose recipe code '
    'holds (chk_project_type_seed).';
COMMENT ON COLUMN project.type IS
    'MOD-146 D1: the project type slug, set at creation; every project before MOD-148 is '
    'software.';
```

### 2.2 `crates/htui-store/cache_migrations/0007_project_type.sql`

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-146 (plan D6): project.type, the project's type slug
-- (migrations/0020_project_type.sql). TEXT with the same default and no foreign key: project_type
-- itself is not mirrored, and CacheStore::project builds a whole Project from this column.
--
-- cache_meta.schema_version moves to 20 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE project ADD COLUMN type TEXT NOT NULL DEFAULT 'software';
```

### 2.3 The edited statements, verbatim

The override is the **unquoted plain rename** `type AS project_type` (precedent:
`pg/relay.rs:701-704`, `r.queued_at AS run_queued_at`). No type annotation is needed: `TEXT NOT
NULL` is `String`. `type` is a non-reserved keyword in both Postgres and SQLite, so it needs no
quoting as a column, and the alias also keeps it from becoming a field of `query!`'s anonymous row.

**`pg/read.rs:626-642` `PgStore::project`**:
```sql
            SELECT id              AS "id: ProjectId",
                   slug,
                   name,
                   description,
                   type            AS project_type,
                   secret_provider,
                   secret_scope,
                   settings,
                   created_by      AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM project WHERE id = $1
```
**`pg/write.rs:2422-2437` `create_project`**: the `INSERT` line is unchanged (the column default
writes `software`, D4). The `RETURNING` list gains `type            AS project_type,` after
`description,`.

**`pg/write.rs:2478-2497` `update_project`**: the `SET` list is unchanged (no write path). The
`RETURNING` list gains the same line after `description,`.

**`pg/write.rs` `impl WriteStore for PgStore`, after `delete_persona` (`:3816`)**, with
`ProjectType` added to the model import (`:23`):
```rust
    // project_type (MOD-146, plan D5)

    /// Every project type in `slug` byte order (`COLLATE "C"`).
    async fn project_types(&self) -> Result<Vec<ProjectType>> {
        sqlx::query_as!(
            ProjectType,
            r#"
            SELECT slug,
                   name,
                   description,
                   coding,
                   builtin,
                   selectable,
                   seed,
                   seed_version,
                   created_by   AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM project_type
             ORDER BY slug COLLATE "C"
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }
```
(`created_by` is nullable, so sqlx infers `Option<UserId>`; `seed` gives `Option<Value>`.)

**`writer.rs`, after `delete_persona` (`:839`)**:
```rust
    async fn project_types(&self) -> Result<Vec<ProjectType>> {
        match self {
            Self::Memory(store) => store.project_types().await,
            Self::Online(pg) => pg.project_types().await,
        }
    }
```

**`cache/refresh.rs:684-739`**: `PROJECT_COLUMNS` gains `"type",` **last** (after
`"updated_at"`). The `query!`:
```rust
        "SELECT id, slug, name, description, secret_provider, secret_scope, settings, created_by, \
                created_at, updated_at, type AS project_type \
           FROM project WHERE id = $1 AND updated_at > $2 ORDER BY updated_at",
```
and `.bind(&row.project_type)` after `.bind(ts_bind(row.updated_at))`. Bind order is column
order, and `upsert_sql` writes `type = excluded.type`.

**`cache/read.rs:897-919` `CacheStore::project`**:
```rust
            "SELECT id, slug, name, description, type, secret_provider, secret_scope, settings, \
             created_by, created_at, updated_at FROM project WHERE id = ?",
```
and in the literal, after `description`: `project_type: text(&row, "type")?,`.

### 2.4 Fixture writers (F-6)

**`pg/demo.rs:115-131` `load_demo`**:
```rust
                "INSERT INTO project (id, slug, name, description, secret_provider, secret_scope, \
                 settings, created_by, created_at, updated_at, type) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
```
with `row.project_type,` after `row.updated_at,`.

**`testkit.rs:537-556` `seed_mirror`** (unchecked SQLite, no `.sqlx`): the column list ends
`… created_at, updated_at, type) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, with
`.bind(&project.project_type)` last.

### 2.5 `.sqlx`
Prepare against a **migrated scratch DB** on `localhost:5439` (memory: the compose `htui` DB is
empty). Expect five files replaced (`project`, `create_project`, `update_project`,
`refresh_project`, the `load_demo` project insert) and one new (`project_types`): 403 → 404.
`git status crates/htui-store/.sqlx` shows 5 `D` and 6 `??`. Stage the deletions too.

### 2.6 `crates/htui-store/tests/migrations.rs`

- `TABLES` (`:30`): append `// 0020_project_type.sql (MOD-146)` and `"project_type",`. In the doc
  (`:25-29`), "The 47 tables …, then the one of `0020_project_type.sql` (MOD-146 plan D1)".
- `migrations_apply_on_a_clean_database` (`:87-140`): the version vec gains `20`; the message gains
  "and MOD-146's 0020_project_type.sql"; `TABLES.len(), 47`; and both messages name the table.
- The comment constants, after `MOD125_TABLE_COMMENT` (`:~597`). The texts are the SQL's, byte for
  byte:
```rust
/// The five `COMMENT ON COLUMN` texts of `0020_project_type.sql` (MOD-146 plan D1), verbatim and
/// in migration order, for [`ANA_COLUMN_COMMENTS`]'s reason.
const MOD146_COLUMN_COMMENTS: &[(&str, &str, &str)] = &[
    ("project_type", "slug",
     "MOD-146 D1: the key project.type references; a letter, then up to 31 of a-z, 0-9 and \
      hyphens (chk_project_type_slug)."),
    ("project_type", "coding",
     "MOD-146 D1: a software-like type (software alone among the built-ins); ANA-39 and \
      MOD-128 read it."),
    ("project_type", "selectable",
     "MOD-146 D1: false for workspace, which only scope rows take (MOD-147); a new-project \
      picker offers the rest."),
    ("project_type", "seed",
     "MOD-146 D1: the JSONB recipe of a user-defined type; NULL for a built-in, whose recipe \
      code holds (chk_project_type_seed)."),
    ("project", "type",
     "MOD-146 D1: the project type slug, set at creation; every project before MOD-148 is \
      software."),
];

/// The one `COMMENT ON TABLE` of `0020_project_type.sql` (MOD-146 plan D1), verbatim.
const MOD146_TABLE_COMMENT: (&str, &str) = (
    "project_type",
    "MOD-146 D1 (ANA-35 7.4): the project type registry. A type seeds a project once, at \
     creation; editing or adding a type never re-seeds one. Six built-in rows; not mirrored.",
);
```
- `the_ana_column_comments_are_present_and_verbatim` (`:869-970`): `.chain(MOD146_COLUMN_COMMENTS)`
  after each of the three `.chain(MOD125_COLUMN_COMMENTS)` (`:880`, `:941`, `:960`). The table loop
  becomes `[ANA_TABLE_COMMENT, MOD125_TABLE_COMMENT, MOD146_TABLE_COMMENT]`. The three messages add
  "or MOD-146". `project` is already a named table (through `project.settings`), so `project.type`
  **must** be in the list, or the "exactly … and no others" assertion fails.
- **New tests**, after the 0019 block (`:~1700`), mirroring `migration_0019_scopes_only_…`
  (`:1465`, `bare_db` + `MIGRATOR.run_to(N-1)`):
  1. `migration_0020_types_an_existing_project_software`: `bare_db`, `run_to(19)`, insert an
     `app_user`, `plant_project_kind(pool, user, "before")`, then `MIGRATOR.run`.
     `SELECT type FROM project WHERE id = $1` is `"software"`. `information_schema.columns` for
     `project.type` is `("text", "NO", "'software'::text")` (`data_type`, `is_nullable`,
     `column_default`).
  2. `migration_0020_seeds_the_six_builtin_types_verbatim`: `fresh_db`. `query_as` a 9-tuple,
     `SELECT slug, name, description, coding, builtin, selectable, seed IS NULL, seed_version,
     created_by IS NULL FROM project_type ORDER BY slug COLLATE "C"`, which equals
     `htui_core::seed::PROJECT_TYPES` mapped to `(slug, name, description, coding, true,
     selectable, true, 1, true)` (D3's pin).
  3. `migration_0020_refuses_an_unknown_type_a_bad_slug_and_a_recipeless_user_type`: `fresh_db`
     and `plant_project_kind`. `UPDATE project SET type = 'nope'` is refused with
     `assert_refused_by(err, "fk_project_type", …)` (if `map_sqlx` does not name 23503's
     constraint, assert SQLSTATE `23503` instead, the `:1610-1617` way).
     `INSERT INTO project_type (slug, name, builtin) VALUES ('Bad', 'Bad', true)` hits
     `chk_project_type_slug`, and so does `'9lives'`. `VALUES ('custom', 'Custom', false)`
     with no seed hits `chk_project_type_seed`. `INSERT … (slug, name, seed) VALUES ('custom',
     'Custom', '{}')` is admitted, and `UPDATE project SET type = 'custom'` then succeeds (the
     FK admits a known slug).

### 2.7 `crates/htui-store/tests/cache.rs` (F-5)

`the_0007_project_type_reaches_the_mirror`, after `the_0005_opening_reaches_the_mirror`
(`:1102-1136`), in its shape: `demo_db`, `open_cache`. Run
`sqlx::query("UPDATE project SET type = 'research' WHERE id = $1").bind(ids::PROJECT_AGY.as_uuid())`
(the project trigger bumps `updated_at`), then `run_pass(&db.pool, &cache, &all_projects(),
&settings(&db, 20))`. Assert `cache.project(ids::PROJECT_AGY)…project_type == "research"`, and that
it equals `db.store.project(ids::PROJECT_AGY)`. Then `teardown`. `projects_agree` is unchanged.

### 2.8 Commit and gate
One commit: `feat(store): MOD-146 T2 project_type migration, PgStore, Writer and mirror`
(migrations, src, tests, `.sqlx`). After it the workspace compiles again.
```bash
cargo test -p htui-store --all-features -- --test-threads=1
cargo sqlx prepare --check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
```

---

## 3. T3: Settings › Project types

### 3.1 `crates/htui/src/store_worker.rs`
- `StoreRequest`, after `ImportPersonas { … }` (`:450`):
  ```rust
      /// The project type registry in slug order (MOD-146 D8). Served in [`try_serve`] through
      /// the writer: `project_type` is not mirrored, so offline it is refused with
      /// `DATABASE_UNREACHABLE`. Answered with [`StoreReply::ProjectTypes`].
      ProjectTypes,
  ```
- `name()`, after `Self::ImportPersonas { .. } => "import_personas",` (`:1223`):
  `Self::ProjectTypes => "project_types",`
- `StoreReply`, after the persona replies (`:1612-…`):
  ```rust
      /// The project type registry in slug order: the answer to [`StoreRequest::ProjectTypes`]
      /// (MOD-146 D8). Read-only; nothing writes a type yet.
      ProjectTypes(Vec<ProjectType>),
  ```
  Add `ProjectType` to the model import.
- `try_serve`, after the persona arm (`:2217-2221`):
  ```rust
          // MOD-146 D8: the project type registry, through the writer. `project_type` is not
          // mirrored, so offline it is refused before any read, as `Personas` is; the `?` keeps
          // `go_offline` working for an `Unreachable` from the read.
          StoreRequest::ProjectTypes => {
              let writer = backend
                  .writer()
                  .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
              StoreReply::ProjectTypes(WriteStore::project_types(&writer).await?)
          }
  ```
- `name_arms_are_stable` (`:5130`): `assert_eq!(StoreRequest::ProjectTypes.name(),
  "project_types");`.
- Any other exhaustive `StoreReply`/`StoreRequest` match is the compiler's to find.

### 3.2 Keys
**`keys/catalogue.rs`**: the new context goes **after `SettingsQueue`** (the Settings block stays
contiguous, the MOD-12 precedent):
- the enum, after `:100`: `/// \`[settings.project_types]\`: the Project types section (no own
  action; list moves only).` `SettingsProjectTypes,`
- `ALL` after `:191`; `table()` after `:245` → `"settings.project_types"`; `heading()` after `:301`
  → `"Project types"`; `is_view()` after `:352` → `| Self::SettingsProjectTypes` (**manual**:
  `matches!` does not fail to compile).
- `context_position` (`:2335-2385`): `Context::SettingsProjectTypes => 19`, and every later arm +1
  (`Concepts => 20` … `Answer => 48`); `every_context_is_listed_once_in_all`: `ALL.len() == 49`.
- `context_tables_and_headings` (`:3195-3197`): add
  `(Context::SettingsProjectTypes, "settings.project_types", "Project types")` after the queue row,
  and in the view/shared block (`:3240-…`):
  `assert!(Context::SettingsProjectTypes.is_view() && !Context::SettingsProjectTypes.is_shared());`.
- No `CATALOGUE` row and no `VIEW_DEFAULTS` row; no `use Context::{…}` change (`:43-49` imports only
  contexts with rows).

**`keys/load.rs:539-547` `TABLES`** and **`tests/keys_file.rs:244-252`**: insert
`settings.project_types, ` after `settings.queue, ` in both strings (re-wrap the `\` lines).

**`keys/views.rs`**, after `QUEUE_BROWSE` (`:477-484`):
```rust
/// Settings > Project types, browsing (MOD-146 D8): read-only, so the list moves and the tab's
/// section cycling are all it offers.
pub const PROJECT_TYPES_BROWSE: Stack<'static> = Stack::new(&[
    Layer::view(Context::SettingsProjectTypes, &[]),
    SETTINGS,
    LIST,
    HISTORY,
    GLOBAL,
]);
```
In its tests: `VIEWS` gains `source!("ui/tabs/settings/project_types.rs")` after `queue.rs`
(`:1092`); `VIEW_STACKS` gains `PROJECT_TYPES_BROWSE` after `QUEUE_BROWSE` (`:1203`); `DELEGATES`
gains `("PROJECT_TYPES_BROWSE", &[SETTINGS_TAB_RS])` after the queue row (`:1269`; `NextSection`
and `PrevSection` are handled by `settings/mod.rs`). `the_view_stacks_resolve_as_designed`
(`:1336`) gains `resolve(PROJECT_TYPES_BROWSE, "j") == [Act::ListDown]`, `"k" == [Act::ListUp]`,
`"l" == [Act::NextSection]` and `"enter" == []`.

**`keys/stack.rs` `DECLARED`**, after `(\"in Settings > Queue\", views::QUEUE_BROWSE)` (`:271`):
`("in Settings > Project types", views::PROJECT_TYPES_BROWSE),`. In
`declared_starts_with_base_and_overlay_and_holds_every_view_stack` (`:413-428`): `len() == 83`, and
`DECLARED[82] == ("in the leave dialog", views::LEAVE)`.

The `j/k` acts are `Act::ListDown` / `Act::ListUp`, the ones Personas' browse view uses
(`personas.rs:488-497`) through the shared `LIST` layer. Nothing is hard-coded: a user may rebind
them under `[list]` or under `[settings.project_types]` (D10's per-view override).

### 3.3 `crates/htui/src/ui/tabs/settings/project_types.rs` (new)

```rust
//! The project type section of the Settings tab (MOD-146 D8): the `project_type` registry,
//! read-only. One line per type (name, slug, a `coding` mark, `not selectable` on `workspace`)
//! and the cursor row's description beneath it.
//!
//! A section like `Personas`: the registry is global, so the read is unscoped and a scope change
//! moves nothing. It holds no store handle (`R-NF-3`): it names [`StoreRequest::ProjectTypes`]
//! and renders what comes back. `project_type` is not mirrored, so offline the read is refused,
//! the section says "unavailable" and offers no key. No write and no `Mode`: user-defined types
//! and their editor are ANA-33's.

use crossterm::event::KeyEvent;
use htui_core::model::{ProjectType, Scope};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};

/// The Browse keys over a readable registry: `j/k select` with the default keys.
pub const HINT_BROWSE: HintSpec = &[Hint::Pair(Act::ListDown, Act::ListUp, "select")];
/// The Browse keys while the registry is unavailable: none (Personas' rule).
pub const HINT_UNAVAILABLE: HintSpec = &[];
/// The read's name, as `StoreRequest::name` spells it: a refused read leaves no list.
pub const READ_NAME: &str = "project_types";
/// Before the first read answered.
pub const NOT_READ: &str = "project types not read yet";
/// The opening of a refused read, drawn `{UNAVAILABLE}: {message}`.
pub const UNAVAILABLE: &str = "project types unavailable";
/// A readable, empty registry (a database short of `0020`'s rows).
pub const NO_TYPES: &str = "no project types";
/// The mark on a `coding` type's line.
pub const CODING: &str = "coding";
/// The mark on a type a new project is not offered.
pub const NOT_SELECTABLE: &str = "not selectable";
/// The separator of a line's segments.
const DOT: &str = " \u{b7} ";

/// `Settings > Project types` (MOD-146 D8).
#[derive(Debug, Default)]
pub struct ProjectTypesSection {
    /// The registry in slug order, as the last `ProjectTypes` answered.
    types: Vec<ProjectType>,
    /// Whether any read has answered: before it, the pane says [`NOT_READ`].
    read: bool,
    /// `Some(message)` after `Failed { request: READ_NAME }`.
    unavailable: Option<String>,
    /// Index into `types`; no wrap.
    cursor: usize,
}

impl ProjectTypesSection {
    /// Stable identity.
    pub const ID: SectionId = SectionId("project_types");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Puts the cursor back inside the list after a read replaced it.
    fn clamp(&mut self) { self.cursor = self.cursor.min(self.types.len().saturating_sub(1)); }

    /// The hint spec: no key while unavailable.
    fn hint_spec(&self) -> HintSpec {
        if self.unavailable.is_some() { HINT_UNAVAILABLE } else { HINT_BROWSE }
    }

    /// One line per type, the cursor row in `theme.selected` with its description wrapped
    /// under it in `theme.dim`, scrolled so the cursor row and its description stay on screen
    /// (`PersonasSection::list_lines`, `personas.rs:1212-1242`).
    fn lines(&self, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
        if let Some(why) = &self.unavailable {
            return wrapped(&format!("{UNAVAILABLE}: {why}"), usize::from(width).max(1))
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect();
        }
        if !self.read {
            return vec![Line::styled(NOT_READ, theme.dim)];
        }
        if self.types.is_empty() {
            return vec![Line::styled(NO_TYPES, theme.dim)];
        }
        let width = usize::from(width);
        let mut lines = Vec::new();
        let mut cursor_end = 0;
        for (index, row) in self.types.iter().enumerate() {
            if index == self.cursor {
                lines.push(Line::styled(row_line(row), theme.selected));
                for line in wrapped(&row.description, width.saturating_sub(4).max(1)) {
                    lines.push(Line::styled(format!("    {line}"), theme.dim));
                }
                cursor_end = lines.len();
            } else {
                lines.push(Line::styled(row_line(row), theme.base));
            }
        }
        let skip = cursor_end.saturating_sub(usize::from(height));
        lines.into_iter().skip(skip).collect()
    }
}

/// `Name · slug`, then `· coding` and `· not selectable` where they hold.
fn row_line(row: &ProjectType) -> String {
    let mut line = format!("{}{DOT}{}", row.name, row.slug);
    if row.coding { line.push_str(DOT); line.push_str(CODING); }
    if !row.selectable { line.push_str(DOT); line.push_str(NOT_SELECTABLE); }
    line
}

impl SettingsSection for ProjectTypesSection {
    fn id(&self) -> SectionId { Self::ID }
    fn title(&self) -> &str { "Project types" } // F-1 (B): "Types"
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `project_type` is global, so the read does not change with the workspace.
        vec![StoreRequest::ProjectTypes]
    }
    fn on_scope_change(&mut self, _scope: &Scope) {}
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::PROJECT_TYPES_BROWSE, chord) {
            match act {
                Act::ListDown => {
                    if self.cursor + 1 < self.types.len() { self.cursor += 1; }
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    self.cursor = self.cursor.saturating_sub(1);
                    return Handled::Consumed;
                }
                _ => continue, // a global act, or `settings.*`, which the tab took first
            }
        }
        Handled::Pass
    }
    fn key_stack(&self) -> Option<Stack<'static>> { Some(views::PROJECT_TYPES_BROWSE) }
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::ProjectTypes(rows) => {
                self.types.clone_from(rows);
                self.read = true;
                self.unavailable = None;
                self.clamp();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.types.clear();
                self.read = true;
                self.clamp();
                self.unavailable = Some(message.clone());
            }
            _ => {}
        }
    }
    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        super::key_line(keys.hint(views::PROJECT_TYPES_BROWSE, self.hint_spec()), theme)
    }
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        frame.render_widget(
            Paragraph::new(self.lines(area.width, area.height, ctx.theme)),
            area,
        );
    }
}
```

The section has no notice row and no help line: there is no write to report. The shell's hint row
carries the keys (MOD-67 M6 T2).

### 3.4 Registration
- `settings/mod.rs`: `pub mod project_types;` (alphabetical, after `prompt`), plus
  `pub use project_types::ProjectTypesSection;`; the module doc (`:3-8`) adds "MOD-146's project
  types (`Project types`)".
- `app/mod.rs:84-85`, after `QueueSection`:
  `// Last (MOD-146 D8): appending moves no existing section's line.` and
  `Box::new(ProjectTypesSection::new()),`; import it beside `QueueSection`.

### 3.5 Tests: `crates/htui/tests/project_types.rs` (new, `#![cfg(feature = "testkit")]`)
These mirror `tests/personas.rs`'s helpers (`bench_with`, `frame`, `shows`, `requests`,
`bench_with_keys`). Rows come from `MemStore::demo().project_types()`.
1. `the_registry_lists_one_line_per_type`: line 0 is `Documents · docs`, line 1
   `    Documents outlined, drafted and reviewed.`, line 2 `General · general`. The Software line is
   `Software · software · coding`, the Workspace line `Workspace · workspace · not selectable`.
   The hint contains `j/k select`. `insta::assert_snapshot!("browse", frame)`.
2. `j_and_k_move_the_cursor_and_stop_at_the_ends`: `k` at the top keeps Documents' description
   under line 0. Seven `j`s land on Workspace, whose description is the last line.
3. `wants_requests_is_the_unscoped_read`: `[StoreRequest::ProjectTypes]` for a demo scope, and
   `on_scope_change` leaves the frame unchanged.
4. `a_refused_read_says_unavailable_and_offers_no_keys`: the bench replies `Failed { request:
   "project_types", message: "x" }`. The frame shows `project types unavailable: x`, and
   `bench.key_hint` holds no `j/k`.
5. `offline_the_section_says_unavailable_and_offers_no_keys`: Personas' `:1361-1404` verbatim in
   shape (`mock_keyring`, a `tempdir` that outlives the harness, `Backend::Offline`,
   `with_store_state("offline · 0s", None)`). It shows `project types unavailable: store
   unreachable:` and `DATABASE_UNREACHABLE`, has no `j/k select`, `j`/`k` change nothing below the
   header, and ends with `insta::assert_snapshot!("offline", frame)`.
6. `the_product_registers_project_types_after_queue`: `Harness::demo()`, `register_all`, `"4"`,
   then ten `"l"`. The frame contains ` Queue  Project types ` (F-1 A: the strip has scrolled) and
   `Software · software · coding`, and the hint holds `j/k select`.
7. `the_read_is_named_project_types`: `StoreRequest::ProjectTypes.name() ==
   project_types::READ_NAME`.
8. `a_view_override_moves_the_cursor_on_its_new_chord_only`:
   `bench_with_keys("version = 2\n[settings.project_types]\ndown = [\"J\"]\n")`. `J` moves the
   cursor and `j` no longer does. This proves the context table exists and that no key is
   hard-coded.

### 3.6 F-1 (A): the strip scrolls
`settings/mod.rs:503-518` `render_strip`, after the spans:
```rust
    // MOD-146 F-1: a strip wider than its area scrolls so the active title ends inside it; one
    // that fits draws as before, from column 0.
    let mut end = 0usize;
    let mut active_end = 0usize;
    for (id, title) in registry.titles() {
        end += cells::cell_width(title) + 2;
        if Some(id) == active {
            active_end = end;
        }
    }
    let offset = u16::try_from(active_end.saturating_sub(usize::from(area.width))).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(Line::from(spans)).scroll((0, offset)), area);
```
- Unit test `a_strip_wider_than_its_area_scrolls_to_the_active_section` in `settings/mod.rs`
  tests: a `SettingsRegistry` with `ConnectionSection` and `QdrantSection` drawn by `render_strip`
  into `TestBackend::new(15, 1)` (the strip is 20 wide). With Connection active, the row starts
  ` Connection `; after `cycle_next`, it ends ` Qdrant `.
- `tests/settings.rs:1081-1112`: the eleven sections, `assert_eq!(sections.len(), 11)`, and the
  assertion becomes `assert_eq!(width, 102, …)`. Rewrite the doc: the eleven cost 102 of 100
  columns, so the strip scrolls (`render_strip`, MOD-146 F-1), and the unit test above pins the
  scroll. Rename it `the_section_strip_width_is_pinned`.
- (B instead: the title `"Types"`, the test stays a ≤ 100 pin at 94 columns and asserts 11 sections,
  test 6 looks for ` Queue  Types `, and §3.6's code is dropped.)

### 3.7 What else may move (find with the full run)
- **Offline full-shell tests**: every `register_all` shell over a non-`Memory` backend now also
  issues `ProjectTypes`. Offline, its `Failed` reaches the status line (`app/update.rs:407`), so
  a shell whose last flash was another section's may now show `project_types: store unreachable:
  …`. The files that combine `register_all` with `over_backend` are `tests/backlog.rs`,
  `connection.rs`, `keys.rs`, `personas.rs`, `secrets_settings.rs`, `settings.rs`, `skills_pg.rs`,
  `templates_pg.rs`, `src/testkit.rs` and `src/ui/header.rs`. Look for failures that name
  `project_types:`.
- **Snapshots**: none expected (F-2). `cargo insta test --workspace --all-features
  --test-runner nextest`, or the plain test run with `INSTA_UPDATE=no`, then `cargo insta
  pending-snapshots`. Accept only the two new files.
- The `?` box for the new view lists `Project types` only while that section is active. No pin
  shows it.

### 3.8 Commits and gate
1. `feat(tui): MOD-146 T3 the Settings strip scrolls to its active section`: §3.6 alone. It is
   green, and every existing snapshot is unchanged.
2. `feat(tui): MOD-146 T3 Settings > Project types`: §3.1-§3.5, the two new `.snap` files and the
   pin edits. One commit, because the stack must be named by a view
   (`every_view_stack_is_named_by_a_view`) and the section registered in the same tree. No
   `todo!()` anywhere in the section: `SettingsTab::on_reply` hands every reply to every section.

```bash
cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features   # review: 2 new, 0 changed
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1
cargo sqlx prepare --check --workspace
```
(Memory: `htui`'s `tests/*.rs` run 0 tests without `--features testkit` / `--all-features`; grep
the run for `SIGABRT`; re-run a `qdrant_live` timeout serially before calling it a regression.)

---

## 4. Plan corrections

1. **F-1**: the plan's D8 title overflows the pinned strip width (102 > 100). The plan's risk table
   misses it. A maintainer choice is needed (A: the strip scrolls, the default; B: the title
   `Types`).
2. **F-2**: "every full-frame Settings strip snapshot gains ' Project types ', and the help box gains
   a context" is false. No `.snap` draws the product strip, the `?` box or `--print-keys`. Expect
   0 changed and 2 new snapshots.
3. **F-3**: T1's `cargo clippy --workspace` cannot pass at T1's end. `htui-store`/`htui` compile
   only after T2.
4. **F-4**: the pins the files table omits: `mem_store.rs`, `pg_conformance.rs`, the
   `migrations.rs` version list and comment test, `keys/stack.rs` `DECLARED`, `keys/load.rs`
   `TABLES`, `tests/keys_file.rs`, `keys/views.rs`' three test lists, `Context::is_view`, and
   `tests/settings.rs`.
5. **F-5**: `projects_agree` needs no edit. It already compares whole rows, but only over the
   default, so a real mirror test is added.
6. **F-6**: `pg/demo.rs` `load_demo` and `testkit::seed_mirror` list `project` columns explicitly.
   Both gain `type`, and `.sqlx` replaces 5 files, not 4.
7. **F-7**: "P3 DDL verbatim" is kept for every column, type, default and CHECK body. The
   constraints are named and the `updated_at` trigger is added, both for house style.
8. **F-8**: case docs must not cite `tests/migrations.rs` test names (the cross-reference scanner).
9. Plan T2 "Mirror: `PgStore::personas` / `persona_rows`": `persona_rows` maps through
   `PersonaRow` only for its `Json<…>` columns. `project_types` reads straight into
   `ProjectType` with `query_as!` (no `FromRow`, no row struct).
