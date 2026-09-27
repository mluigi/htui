# Blueprint: MOD-9 milestone 3 — skills are editable and bindable

> **Source of truth:** `.claude/plans/mod-9-skills-editable.plan.md`, CONFIRMED by the maintainer
> 2026-09-27 with two overrides (OQ-14 hand-written matcher, OQ-15 changed paths in scope). Where
> this blueprint and the plan disagree, the plan wins unless §11 says the tree made it impossible.
> The two prior MOD-9 blueprints (`mod-9-templates-editable.blueprint.md`,
> `mod-9-skills-reach-the-run.blueprint.md`) are the format model.
>
> **Numbering.** The plan owns D70–D89 and R-25–R35. This blueprint owns **D90–D106** and **H-1…H-33**.
>
> **Tree state when written:** branch `mod-9-m3`, HEAD `df99f62` (plan doc; source identical to the
> base `68c058f` the plan cited). `.sqlx` = 268 files. Six `templates__*.snap`.

---

## 0. Findings — what the plan got wrong, and what this blueprint changes

Thirteen findings came out of reading the seams. Six change a task's file list, so they are stated
here and reflected in the task sections rather than applied silently.

| # | Finding | Severity | Decision |
|---|---|---|---|
| **F-1** | **T1 cannot compile as scoped.** D78 makes `set_skill_binding` refuse a glob `glob::compile` rejects, but `prompt/glob.rs` is a T2 file and T1 lands first. | **Blocker** | **D95** — `prompt/glob.rs`, its `mod` line and its full dialect table land as **commit 1 of T1**. It needs only `RepoPath` (`excerpt.rs:102`) and `BoundSkill` (`skill.rs:161`), both of which exist today. |
| **F-2** | **One request per project is wrong.** `store_worker.rs`'s own `Catalogue(Scope)` doc says one request per event and **never one per project**: the staleness index (`App::latest`, keyed on `(origin, discriminant)`) keeps only the newest of a variant, so N `Skills(project_i)` requests leave N−1 undrawn. | Major | **D92** — the read is one request carrying the scope: `StoreRequest::Skills(Scope)`, answered by a `SkillsSnapshot` that holds the library and every attachment of every project in the scope. The per-phase read the clone needs is a second, different method (D98). |
| **F-3** | **The name is taken.** `pg/rows.rs:257` already declares `pub(crate) struct SkillBindingRow`, imported by `pg/read.rs` and `pg/write.rs`. A second `SkillBindingRow` in `model` is an E0252 in both. | Major | **D93** — the model's row type is **`SkillAttachmentRow`**. The store's private DTO keeps its name and gains the same job; `bound_skills` keeps using it. |
| **F-4** | D77 and T1's file list put `NewSkill` / `NewSkillVersion` / `NewSkillBinding` in `model/kind.rs`. | Minor | **D90** — they live in **`model/skill.rs`**, with the model they construct. `kind.rs` holds `NewPromptTemplate` because it holds `PromptTemplate`. |
| **F-5** | D73 types `skill_matches` as a `BTreeMap` but hands `select` an `Option<&BTreeMap>`; and `PromptSpec` derives no `Default`, so every literal is a hard error on a new field. | Major | **D97** — the spec field is `pub skill_matches: Option<BTreeMap<SkillId, String>>` and it gains `pub skill_files: Vec<RepoPath>`. Without the `Option`, "no file set resolved" and "the set ran and matched nothing" collapse into one record and `no_path` becomes unreachable. Every `PromptSpec` literal in the tree grows both fields: `prompt::fixtures`, `phase_spec`, `preview::build`, the judge builder. |
| **F-6** | D71 does not say whether `{…}` may sit inside a component. | Minor | **D96** — an alternation is a **whole component**: `src/{a,b}/x.rs` compiles, `pre{a,b}fix.rs` is refused. The compiled form is then one level deep and the matcher is a small recursive function. |
| **F-7** | D85's copy needs phase-level attachment **rows**, and `override_graph<S: WriteStore, G: GraphSource>` has no read: `bound_skills` returns collapsed `BoundSkill`s, which carry no `languages`. | Major | **D98** — a new `GraphSource::phase_attachments(project, phase) -> Result<Vec<SkillBinding>>`, with four delegations: `graph.rs::TestSource`, `fake.rs::MemStore`, `fake.rs::FakeGraphSource`, and **`run_worker.rs::BackendGraphs`**. T5's file list gains `crates/htui/src/run_worker.rs`. |
| **F-8** | `.sqlx` bookkeeping across three tasks. | Minor | **D100** — T1 prepares against a database migrated to `0007`; T2 adds a comment-only `0008`, so nothing re-prepares and `--check` still passes; T5 re-prepares because `create_step_graph`'s text changes (one file **replaced**, net still 268). |
| **F-9** | D78 says the writer is the only place a bad pattern is refused; T4's test says the view refuses "before it is sent". | Minor | **D99** — both hold because the view's notice and the writer's `Constraint` render **the same `GlobError::Display` string**, and the view pre-flights with the same `glob::compile`. |
| **F-10** | T4's language-map test says "`rust` → its two patterns"; ANA-22 §9's `rust` is one. | Minor | **D100** — the test keeps its *shape* using `shell` (`.sh`, `.bash`, `.zsh`); `rust` stays `["**/*.rs"]`. |
| **F-11** | Every conformance case runs against a store seeded with `tests` and `rust-style`. A case that upserts either name moves the demo row and breaks `mem.rs:5966-6010` and `pg_criteria.rs:1806`. | Major, hazard | **D102** — every new case mints its own skill name (`writer-case-skill`, `version-case-skill`, …). A test that asserts on a demo row is a bug. |
| **F-12** | `wants_requests` grows from one request to two. | Minor | **D92** — safe, because the staleness index keys on the variant's discriminant, so `Templates` and `Skills` do not supersede each other. Stated so nobody "fixes" it. |
| **F-13** | The global keymap is `q`, `Tab`, `Shift+Tab`, `1`–`9`, `?`, overlay `Esc`. `w` is documented as T6's but unbound. | Minor | **D106** — every T3/T4 browse key is free; `w` stays free and unused. |

## 0a. Settled before implementation, no further gate

- The four `WriteStore` methods, their `CasOutcome<T>` returns and the token-first error order
  (D76, D78) are the plan's and are not reopened here.
- The matcher is hand-written (D70, D71). There is **no dependency change in this milestone**:
  `git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` must be empty at the end of T5.
- The dialect is normative (D71, D103): `?` one non-`/` char; `*` never crosses `/`; `**` only as a
  whole component; `{a,b}` only as a whole component and never nested; `[abc]`, `[!abc]`, `[a-z]`;
  `\` escapes; leading `!`, trailing `/`, absolute paths and NUL are refused; case-sensitive.
- `migrations/0008_skill_match.sql` changes no column: it re-issues one comment and moves the
  `migrations.rs:81` and `:208-216` pins.

---

## 1. Build order and gate environment

| Task | Lands | Commits | Gate after |
|---|---|---|---|
| **T1** | `glob.rs` first, then the four writers, then the worker seam | 5–7 | `htui-core`, `htui-store` (Postgres), `htui` compile, `sqlx prepare --check` |
| **T2** | matcher wiring, the record, the isolator, `0008`, engine and preview | 5–6 | `htui-core`, `htui-orch`, `htui` (Postgres) |
| **T3** | the Skills view | 3–4 | `htui` (Postgres) + `insta` review |
| **T4** | the attachments matrix and the language map | 3–4 | `htui` (Postgres) + `insta` review |
| **T5** | the override clone | 2–3 | `htui-core`, `htui-store` (Postgres), `htui-orch` |

Serial, not parallel: the fact-check demoted the T3/T4 wave because both need a `mod` line in
`skills/mod.rs` and a `mod matrix;` cannot commit before `matrix.rs` exists (R-30).

**Gate environment.** `USERNAME=htui-ci` and `HTUI_TEST_DATABASE_URL` for `htui-store` and `htui`;
`cargo test -p <crate> --all-features -- --test-threads=2`; the workspace gate runs single-threaded
for anything touching the keyring fake. `ORT_LIB_LOCATION` points at the npm ONNX Runtime copy
because `parcel.pyke.io` is still denied (R-34).

---

## 2. Task 1 — the glob matcher, the writers, the worker seam

### 2.1 `crates/htui-core/src/prompt/glob.rs` (new — commit 1, D95)

```rust
//! Path globs for skill activation (ANA-22 §6 item 7; plan D70, D71). Hand-written, deliberately:
//! `globset` matches `*` across `/` by default and its `backslash_escape` default differs per
//! platform, so both would have to be configured away to get the dialect below.
//!
//! The dialect, over repo-relative `/`-separated paths, is normative: `?` is one non-`/` character,
//! `*` is any run of non-`/` characters and never crosses `/`, `**` matches zero or more whole
//! components and is legal only as a whole component, `{a,b}` alternates and is legal only as a
//! whole component, `[abc]` / `[!abc]` / `[a-z]` are character classes, `\` escapes the next
//! metacharacter, and a leading `!`, a trailing `/`, an absolute path and a NUL are refused.
//! Matching is case-sensitive. One implementation of "compiles" serves both the writer and the
//! matcher, so a stored glob is always a compiling one (plan D78, D99).

/// A compiled glob: segments separated by `/`, each one of [`Segment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The `<repo>:` qualifier, when the pattern names one. `None` matches in any repo (D71).
    pub repo: Option<String>,
    /// The path part, `/`-separated.
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// `*` anywhere inside a component.
    Parts(Vec<Tok>),
    /// A `**` whole component: zero or more components.
    AnyComponents,
    /// `{a,b}` over whole components (D96: never nested).
    Alt(Vec<Vec<Segment>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Char(char),
    /// `?`
    Any,
    /// `*`
    Star,
    Class { negated: bool, ranges: Vec<(char, char)> },
}
```

`pub fn compile(pattern: &str) -> Result<Pattern, GlobError>`, `pub fn matches(pattern: &Pattern, path: &str) -> bool`,
`pub fn first_match(globs: &[String], files: &[RepoPath]) -> Option<RepoPath>` and
`pub fn matched_skills(skills: &[BoundSkill], files: &[RepoPath]) -> BTreeMap<SkillId, String>`
(the value rendered `<repo>:<path>`; the first match in file order wins).

**`GlobError` (D103)** — one variant per refusal, each carrying a byte position, and
`pub fn at(&self) -> Option<usize>` beside the `error_at` helper in `templates.rs:290`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GlobError {
    #[error("a glob must not be empty")]
    Empty,
    #[error("a leading `!` is not negation in htui globs, at byte {at}")]
    Negation { at: usize },
    #[error("a trailing `/` matches directories, which a file set never holds, at byte {at}")]
    TrailingSlash { at: usize },
    #[error("a glob must be repo-relative, at byte {at}")]
    Absolute { at: usize },
    #[error("`**` is only legal as a whole component, at byte {at}")]
    MisplacedGlobstar { at: usize },
    #[error("`{{a,{{b,c}}}}` nests, which this dialect does not allow, at byte {at}")]
    NestedAlternation { at: usize },
    #[error("`{{a,}}` has an empty alternative, at byte {at}")]
    EmptyAlternative { at: usize },
    #[error("`{` without `}`, at byte {at}")]
    UnterminatedBrace { at: usize },
    #[error("`[` without `]`, at byte {at}")]
    UnterminatedClass { at: usize },
    #[error("`[]` matches nothing, at byte {at}")]
    EmptyClass { at: usize },
    #[error("`\\` at the end of a glob escapes nothing, at byte {at}")]
    DanglingEscape { at: usize },
    #[error("a glob must not contain a NUL character, at byte {at}")]
    Nul { at: usize },
    #[error("`{repo}:` names no repository, at byte {at}")]
    BadQualifier { at: usize },
}
```

The module's tests are the plan's T2 table, re-homed here because the table is now the
specification, not a cross-implementation check: `**/*.rs` matches `crates/x.rs`; `*.rs` does **not**
match `crates/x.rs`; `src/*.rs` does not match `crates/x.rs`; `{a,b}/x.rs` matches `a/x.rs`;
`[abc].rs` matches `b.rs` and not `d.rs`; `[!abc].rs` matches `d.rs`; `htui:**/*.rs` matches repo
`htui` only; a bare glob matches every repo; `src/` does not match `src2/x.rs`; `src/**` matches
`src`, `src/a.rs` and `src/a/b.rs`; an empty `globs` list matches nothing; and one test per
`GlobError` variant asserting the byte position.

### 2.2 `model/skill.rs` — the three `New*` structs (D90, D91)

```rust
/// Arguments of [`crate::store::WriteStore::upsert_skill`]. The id is minted client-side, like
/// `NewItemKind` and `NewPromptTemplate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSkill {
    /// `skill.id`, a UUIDv7.
    pub id: SkillId,
    /// `skill.name`; the Agent Skills rule is checked by the writer (D77, OQ-20).
    pub name: String,
    /// `skill.description`.
    pub description: String,
    /// `skill.created_by`; the worker fills it from `this_user` (`R-NF-3`).
    pub created_by: UserId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSkillVersion {
    /// `skill_version.skill_id`.
    pub skill_id: SkillId,
    /// `skill_version.body`.
    pub body: String,
    /// `skill_version.source`: import provenance. `{}` for a body typed in the TUI.
    pub source: serde_json::Value,
    /// `skill_version.created_by`.
    pub created_by: UserId,
}

/// Arguments of [`crate::store::WriteStore::set_skill_binding`]. The key is
/// `(skill_id, project_id, phase_id)`; a global attachment is `(skill, None, None)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSkillBinding {
    /// `skill_binding.id`, minted client-side: the upsert's `ON CONFLICT … DO UPDATE` needs a row id
    /// for the insert case and keeps it for the update case.
    pub id: SkillBindingId,
    /// `skill_binding.skill_id`.
    pub skill_id: SkillId,
    /// `skill_binding.project_id`; `None` is the global level (ANA-22 §6 item 2).
    pub project_id: Option<ProjectId>,
    /// `skill_binding.phase_id`; `Some` requires a project.
    pub phase_id: Option<PhaseId>,
    /// `skill_binding.pinned_version`; `None` follows latest.
    pub pinned_version: Option<i32>,
    /// `skill_binding.position`.
    pub position: i32,
    /// `skill_binding.activation`.
    pub activation: Activation,
    /// `skill_binding.globs`: typed globs unioned with the named languages' expansions (D83).
    pub globs: Vec<String>,
    /// `skill_binding.languages`, as authored; display only.
    pub languages: Vec<String>,
}
```

Plus `impl Skill { pub fn name_is_valid(name: &str) -> bool }` beside
`PromptTemplate::name_is_valid` (`kind.rs:359`) — `[a-z0-9-]`, length 1–64, no leading, trailing or
doubled hyphen.

### 2.3 `store/traits.rs` — four methods, four helpers (D76, D77)

After the `append_prompt_template` section at `:722-745`:

```rust
    // skill, skill_version, skill_binding (MOD-9 milestone 3, plan D76-D78)

    /// Creates the skill, or updates its description, iff its `updated_at` is `expected` (`None`:
    /// no row with that name). The name is the library key and never moves; `skill` is global and
    /// `skill_binding` is what scopes a skill.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "skill" }` when
    /// `expected` is `Some` and the name has no row; [`StoreError::Constraint`] for a name the
    /// Agent Skills rule refuses, a NUL, or a `created_by` that names no row. Nothing is written.
    async fn upsert_skill(&self, new: NewSkill, expected: Option<DateTime<Utc>>)
        -> Result<CasOutcome<Skill>>;

    /// Appends version `expected + 1` of the skill iff `expected` is its head (`None`: no version
    /// yet), as one compare-and-set. `skill_version` is append-only and carries no `updated_at`
    /// (`0001_init.sql:569-571`), so the head version is the only token it has.
    async fn add_skill_version(&self, new: NewSkillVersion, expected: Option<i32>)
        -> Result<CasOutcome<SkillVersion>>;

    /// Inserts the attachment, or updates it, iff its `updated_at` is `expected` (`None`: no row at
    /// that `(skill, project, phase)`). Order, the same on every store: the token first, so a spent
    /// token answers `Stale` even for bad input; then the checks Postgres also carries —
    /// `skill_binding_phase_needs_project` and `skill_binding_glob_needs_globs`; then the two rules
    /// it does not — a repo-qualified glob on a **global** row (ANA-22 §6 item 6) and a pin the
    /// skill has no version for; then every glob compiles (D95, D99); then the keys.
    async fn set_skill_binding(&self, new: NewSkillBinding, expected: Option<DateTime<Utc>>)
        -> Result<CasOutcome<SkillBinding>>;

    /// Deletes the attachment iff its `updated_at` is `expected`. A spent token answers `Stale` and
    /// the row survives; `R-SKL-3`'s "unbind" is a delete, not an `off` row that reads as attached.
    async fn remove_skill_binding(&self, id: SkillBindingId, expected: DateTime<Utc>)
        -> Result<CasOutcome<SkillBinding>>;
```

Helpers beside `invalid_template_name` (`:1464`) and `prompt_template_refusal` (`:1482`):
`invalid_skill_name(name)`, `skill_key(name)`, `skill_binding_key(skill, project, phase)`, and

```rust
/// MOD-9 D77: why a skill may not be saved, or `None` when it may. The name rule first, then a NUL
/// in the name or the description (which Postgres `text` cannot hold, `22021`).
#[must_use]
pub fn skill_refusal(name: &str, description: &str) -> Option<String>;

/// MOD-9 D78: why an attachment may not be written, or `None`. Every sentence here is one both
/// stores give, and the glob sentence is `glob::compile`'s own `Display` (D99).
#[must_use]
pub fn skill_binding_refusal(new: &NewSkillBinding, versions: &[SkillVersion]) -> Option<String>;
```

The module doc at `traits.rs:26` loses `skill_version` and `skill_binding` from its "stay inherent"
sentence and gains the reason: the **reads** stay inherent (the tables are not mirrored), the
**writers** are on the trait because they must be reachable through `Writer`.

### 2.4 `store/mem.rs` — four `State` methods (D79)

Over `skills: HashMap<SkillId, Skill>`, `skill_versions: Vec<SkillVersion>`,
`skill_bindings: Vec<SkillBinding>` — the collections keep their shapes (`mem.rs:114-122`), because
`skill_version`'s key is `(skill_id, version)` and `skill_binding`'s is `(skill_id, project_id,
phase_id)` **with NULLs**, so both stay `Vec` and are filtered.

Each method takes `(… , now: DateTime<Utc>)`, and each reads the token first:
`append_prompt_template` at `mem.rs:2714-2768` is the model, and its doc line is the rule —
*"token, then input, then keys"*. `State::require_user` (`:1862`) and `references_no_row` (`:1512`)
are the key checks; `already_exists` (`:1517`) guards a reused id. `project_reach` (`:3136`) and
`delete_project` (`:3317`, which keeps globals by retaining `row.project_id != Some(id)`) are
unchanged.

`remove_skill_binding` returns `Err(StoreError::NotFound { entity: "skill_binding", id })` when no
row matches the id at all, `Ok(Stale(current))` when the row exists and the token is spent, and
`Ok(Applied(removed))` otherwise.

### 2.5 `pg/write.rs` — the four statements (D78, D100)

Model: `append_prompt_template` (`write.rs:2439-2502`) and `cas_miss` (`:78-89`).

```sql
-- upsert_skill
INSERT INTO skill (id, name, description, created_by, created_at, updated_at)
SELECT $1, $2, $3, $4, now(), now()
 WHERE (SELECT updated_at FROM skill WHERE name = $2) IS NOT DISTINCT FROM $5::timestamptz
ON CONFLICT (name) DO UPDATE
   SET description = $3, updated_at = now()
 WHERE skill.updated_at = $5::timestamptz
RETURNING id, name, description, created_by, created_at, updated_at
```

```sql
-- add_skill_version
INSERT INTO skill_version (skill_id, version, body, source, created_by, created_at)
SELECT $1, COALESCE($4::int, 0) + 1, $2, $3, $5, now()
 WHERE (SELECT max(version) FROM skill_version WHERE skill_id = $1) IS NOT DISTINCT FROM $4::int
ON CONFLICT (skill_id, version) DO NOTHING
RETURNING skill_id, version, body, source, created_by, created_at
```

```sql
-- set_skill_binding
INSERT INTO skill_binding (id, skill_id, project_id, phase_id, pinned_version, position,
                           activation, globs, languages, updated_at)
SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, now()
ON CONFLICT (skill_id, project_id, phase_id) DO UPDATE
   SET pinned_version = $5, position = $6, activation = $7, globs = $8, languages = $9,
       updated_at = now()
 WHERE skill_binding.updated_at = $10::timestamptz
RETURNING id, skill_id, project_id, phase_id, pinned_version, position, activation, globs,
          languages, updated_at
```

```sql
-- remove_skill_binding
DELETE FROM skill_binding WHERE id = $1 AND updated_at = $2::timestamptz
RETURNING id, skill_id, project_id, phase_id, pinned_version, position, activation, globs,
          languages, updated_at
```

`ON CONFLICT (skill_id, project_id, phase_id)` infers the `UNIQUE NULLS NOT DISTINCT` index
(D78's confirmed caveat: order-insensitive, exact column set, non-partial). `TEXT[]` binds as a
`Vec<String>`; the re-read (for `cas_miss`) takes `&row[..]` the way the demo loader does.

Re-reads, new and inherent on `PgStore` (`pg/read.rs`, beside `prompt_template` at `:1673`):
`skill_by_name(name)`, `skill_versions_of(skill)`, `skill_binding_at(skill, project, phase)`,
`skill_binding_by_id(id)` — each `query_as!` with the house alias style
(`AS "id: htui_core::model::SkillBindingId"`, `AS "activation: Activation"`).

### 2.6 The rest of T1

- **`writer.rs:705-714`** — four delegating arms, in trait order.
- **`htui-agent/src/conformance.rs:936-942`** and **`htui-agent/tests/recorder.rs:631-637`** — four
  pure delegating arms each. No counter: only `run_step.usage` and `agent_box.quota` are spied, and
  no new case needs to observe a call (D86's rule).
- **`backend.rs`** — four inherent read dispatchers beside `bound_skills` (`:366`), each refusing
  offline with `prompt_offline()`; the `:336` comment's list of unmirrored tables is unchanged,
  because nothing about the mirroring moved.
- **`conformance.rs`** — four cases with fresh skill names (D102):
  `skill_upsert_creates_then_edits_under_the_updated_at_token`,
  `skill_version_append_is_a_cas_on_the_head` (including `Some(0)` → `NotFound`, the F-B
  regression the template case pins), `skill_binding_upsert_replaces_its_own_row_and_a_spent_token_is_stale`,
  `skill_binding_refuses_what_its_checks_refuse` (every refusal, then a good save at the same token
  that proves none of them wrote). Registered in `CASES` and in `run_case`; `applied`/`stale`
  (`:1949`/`:1957`) throughout, never `.unwrap()`.
- **`crates/htui-store/tests/skill_attachments.rs`** — the four new cases over planted rows, plus
  the glob-compile refusal. Plant helpers stay unchecked `sqlx::query`, so the file still adds
  nothing to `.sqlx` (`:10-11`).
- **`crates/htui-store/tests/skill_binding_cas.rs`** (new) — the concurrent CAS, modelled on
  `prompt_template_cas.rs`: a transaction held open by hand (`:44-47`), the second writer waits on
  the index and inserts nothing, and the re-read answers `Stale` with the winner's row. Unchecked
  SQL throughout, so `.sqlx` is untouched by the test.
- **`pg_conformance.rs:19`** — `EXPECTED_CASES: usize = 81`.

### 2.7 `crates/htui/src/skills.rs` (new) — the worker seam (D81, D92, D101)

Mirrors `crates/htui/src/templates.rs` whole. `pub const REQUEST_NAMES: [&str; 4] =
["skills", "save_skill", "set_skill_binding", "remove_skill_binding"];` with `pub const READ_NAME`
and the three write names; `SkillBody(String)` with a `Debug` that prints `len`; and

```rust
/// Everything the Skills view renders, read for every project in the scope at once (D92: the
/// staleness index keeps only the newest request of a variant, so one request per project would
/// leave all but one undrawn).
#[derive(Debug, Clone, PartialEq)]
pub struct SkillsSnapshot {
    /// The global library, name-ordered by bytes.
    pub skills: Vec<SkillSummary>,
    /// Every attachment of every project in the scope, plus the global ones.
    pub attachments: Vec<SkillAttachmentRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillSummary {
    pub id: SkillId,
    pub name: String,
    pub description: String,
    /// Every version, ascending; the last is the head.
    pub versions: Vec<SkillVersion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillAttachmentRow {
    pub id: SkillBindingId,
    pub skill_id: SkillId,
    pub skill_name: String,
    pub project_id: Option<ProjectId>,
    pub phase_id: Option<PhaseId>,
    pub phase_name: Option<String>,
    pub pinned_version: Option<i32>,
    pub position: i32,
    pub activation: Activation,
    pub globs: Vec<String>,
    pub languages: Vec<String>,
    pub updated_at: DateTime<Utc>,
}
```

`serve` matches four variants. `Skills(scope)` goes through `Backend` (the reads are inherent and
refuse offline with their own sentence); the three writes take `backend.writer()` or
`Unreachable(DATABASE_UNREACHABLE)`, fill `created_by` from `this_user()`, call the writer, and
**re-read the whole scope** rather than handing the view the one row the outcome carries. `SaveSkill`
carries **two** tokens (D101) and the worker short-circuits on the first `Stale`:

```rust
StoreRequest::SaveSkill {
    scope: Scope,
    skill: Option<SkillId>,          // None: a new name
    name: String,
    description: String,
    body: SkillBody,
    expected: Option<DateTime<Utc>>, // the row's updated_at the editor opened on
    expected_version: Option<i32>,   // the head version the editor opened on
} => { /* upsert_skill, then add_skill_version; the first Stale answers SkillsStale */ }
```

`StoreRequest` 69 → 73, `StoreReply` 40 → 42 (`Skills(Box<SkillsSnapshot>)`,
`SkillsStale(Box<SkillsSnapshot>)`), one new **or-ed** `try_serve` arm beside `:1126` (a guard arm
does not count towards exhaustivity — MOD-15 M3 plan F-12), four `StoreRequest::name` arms, and
`pub mod skills;` at `crates/htui/src/lib.rs:28`.

The module ships `request_names_match_the_name_arms` (`templates.rs:425-436` is the model), a test
that a `SaveSkill` `Debug` prints the body length, and the two offline tests `templates.rs` has.

### 2.8 Hazards — T1

| # | Hazard | Evidence | What to do |
|---|---|---|---|
| H-1 | A conformance case that upserts `tests` or `rust-style` moves a demo row and breaks `mem.rs:5966-6010` and `pg_criteria.rs:1806` | `fixtures::skill_bindings()` `:574-608` | fresh names in every case (D102) |
| H-2 | A second `SkillBindingRow` in `model` is an E0252 against `pg/rows.rs:257` | F-3 | name it `SkillAttachmentRow` (D93) |
| H-3 | `skill_version` has no `updated_at` trigger, so its only token is the head version; a direct `add_skill_version` with a stale `expected` writes a version whose body nobody edited | `0001_init.sql:569-571` | D101; case 2 pins it |
| H-4 | `Some(0)` on a skill with no version must be `NotFound`, not an insert of v1 | the template case's F-B regression | copy the case's loop |
| H-5 | The refusal order is the load-bearing invariant: token first, then input, then keys | `mem.rs:2714-2768`, case 3's tail | every store, every method |
| H-6 | `sqlx prepare` against the compose `htui` database produces an empty, wrong `.sqlx` | memory: the compose DB is empty; the prepare DSN is not the test DSN | a scratch DB migrated from zero to `0007` |
| H-7 | A `TEXT[]` re-read binds as `&row[..]`, not `Vec<String>` | `pg/demo.rs` precedent | copy the binding |
| H-8 | `.sqlx` count must be recorded, not assumed | D100 | the count goes in the T1 commit message |

## 3. Task 2 — F2's file set, the matcher, the record

### 3.1 `ExcerptSet.listed` (D72) and `PromptSpec` (D97)

`ExcerptSet` (`:1463`) gains `pub listed: Vec<RepoPath>`, filled in `select` from the `listing` local
(`:1085`) **after** the `skip_by_path` and `is_repo_relative` filters and **before**
`fill_lexical_heads` takes `&mut listing`. Three construction sites grow the field: `select` itself,
`no_excerpts` (`engine.rs:5822-5835`, field by field), and every literal in the excerpt tests.

`PromptSpec` (`:71-123`) gains two fields next to `skills`:

```rust
    /// The step's file set for glob activation: the walk's listing unioned with the previous
    /// attempt's changed paths (D87, D89). Empty when no root resolved.
    pub skill_files: Vec<RepoPath>,
    /// Each candidate's matched path, `<repo>:<path>`, when its globs matched (D97). `None` means
    /// no file set resolved, which is what `no_path` records; `Some` and empty means the set ran
    /// and matched nothing, which is `no_match`.
    pub skill_matches: Option<BTreeMap<SkillId, String>>,
```

**Every `PromptSpec` literal in the tree grows both** — `prompt::fixtures`, `phase_spec`
(`engine.rs:4992-5116`), the judge builder, `preview::build`. The struct derives no `Default`, so
this is a compile error at each site, which is the intent.

### 3.2 `Isolator::changed_paths` (D89)

```rust
    /// The repo-relative files the step's commits changed, as `git diff --name-only` reports them
    /// (MOD-9 D89). `Ok(vec![])` when a repository has no before/after pair, and the caller treats
    /// a failure as "no changed paths" with a note — the walk still runs.
    fn changed_paths<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Vec<RepoPath>>;
```

`GixIsolator` implements it beside `diff_of` (`:1287-1351`), sharing a new `range_of` helper with
it, and calls `Cli::changed_paths` (`git.rs`, beside `Cli::diff` at `:815-841`):

```
diff --no-color --no-ext-diff --no-textconv --name-only -z <before> <after> --
```

`-z` makes the output NUL-separated, so `core.quotePath` quoting never has to be unescaped — which
is the whole reason to use `--name-only` rather than parsing a diff (D89). `FakeIsolator` queues an
answer beside its `diff` queue and logs the call.

`Engine::with_excerpts` (`:4936-4980`) calls it once, **only when some candidate is
`Activation::Glob`** (R-35), unions the result into `spec.skill_files`, and hands the same vector to
`ExcerptRequest.changed_paths` — which `excerpts_for` today hard-codes to `Vec::new()` under a
`D122` comment (`htui-agent/src/excerpt.rs:975-977`). That comment is replaced, and `TIER2_PREV_DIFF`
(`excerpt.rs:755-761`) becomes live in production for the first time.

### 3.3 `model::skill::select` and the record (D74)

`ChoiceReason` gains `Matched` and `NoMatch`; `SkillChoice` gains `pub matched: Option<String>`, so
its key set is eight (`skill.rs:777-786` moves, with a message naming the reason) and the pinned
`reason` spellings at `:795-800` gain the two. The signature becomes:

```rust
pub fn select(
    candidates: Vec<BoundSkill>,
    placed: bool,
    matches: Option<&BTreeMap<SkillId, String>>,
) -> (Vec<BoundSkill>, Vec<SkillChoice>)
```

`Activation::Glob` resolves to `Matched` when `matches` holds the skill, `NoMatch` when it does not,
and `NoPath` when `matches` is `None`. `is_active` widens to `Always | Matched`. The three pinned
tests move with new cases (`select_applies_its_rules_in_order` gains the `Glob` case; two new tests
for `matched` and `no_match`).

`TrimRecord` (`:181-206`) does not change shape: `skill_choices` was already a top-level key and
`v` is already `2`. **`prompt_digest.rs`'s thirteen top-level keys and `v == 2` do not move** — the
`matched` key is inside a choice, not at the record's top level. The golden `IMPL_TRIM_RECORD`
(`fixtures.rs:1631-1665`) gains `"matched": null` in each choice.

### 3.4 `migrations/0008_skill_match.sql` (D75)

Comment-only. `COMMENT ON COLUMN run_step.trim_record IS '…reason always, matched, no_match, off,
no_path, missing_version or not_placed (ANA-22 6 item 8; MOD-9 D74)…'` and nothing else. The pins
that move: `migrations.rs:81` (`vec![1, 2, 3, 4, 5, 6, 7]` → `… 8`, message updated),
`migrations.rs:877` (`Pending(7)` → `Pending(8)`), `migrations.rs:878` ("seven embedded migrations"
→ "eight", also `tests/connect.rs:103`, `:119`, `:205`), and **`migrations.rs:208-216`**, the pinned
comment text, byte for byte. `TABLES` stays 39; the commented-column total stays 34.

A migration test reads `0008` with `include_str!` and asserts each of the seven reason words appears
in the pinned text — the same guard style as D58's literal check.

### 3.5 The two callers (D73)

`with_excerpts` fills `skill_files` and `skill_matches` after the walk; `preview::build` does the
same from its own walk (`preview.rs:308`). `SKILLS_NOTE` (`:85-87`) flips its sentence, and
`STAND_INS` (`:61-70`) keeps its eight entries — the test
`the_stand_in_list_is_the_one_the_notes_are_built_from` requires every stand-in to start
`"preview: "`.

### 3.6 Hazards — T2

| # | Hazard | Evidence | What to do |
|---|---|---|---|
| H-9 | `fill_lexical_heads` takes `&mut listing`; the new field must be captured first | `excerpt.rs:1085`, `:1403-1455` | clone or drain into `listed` before that call |
| H-10 | A `PromptSpec` literal somewhere deep in a test grows the two fields and is missed because it is behind a helper | the struct has no `Default` | the compile error is the gate; do not add a `Default` to make it quiet |
| H-11 | `with_excerpts` re-reads the previous step's trees and commits that `forwarded` (`:5163-5212`) already read | both read `step_trees`/`step_commits` | share the read, or accept the second read and say so in the commit |
| H-12 | R-35's "skip the call unless a candidate is `Glob`" keeps `TIER2_PREV_DIFF` dead on steps with no glob candidate | `excerpt.rs:755-761` | the tier-2 test must plant a glob attachment, or it proves nothing |
| H-13 | `IMPL_TRIM_RECORD` moves but `prompt_digest.rs`'s thirteen keys must not | D74, fact-check row | a diff of those two files is the proof |
| H-14 | The `migrations.rs:208-216` literal is a byte-for-byte pin; a paraphrase fails the suite for a reason that reads like a typo | `:417-501` | copy the new text out of `0008` and compare |

## 4. Task 3 — the Skills view (D82, D84, D106)

`crates/htui/src/ui/tabs/skills/library.rs`, declared `mod library;` at `skills/mod.rs:8`, with
`skills/mod.rs` gaining the per-view `captures_input()` guard (today's is at `:86`, single-view) and
`wants_requests` sending **two** requests (`:75-77`, safe per F-12).

**State**, mirroring `TemplatesView`'s thirteen fields: `snapshot: Option<SkillsSnapshot>`,
`unavailable: Option<String>`, `cursor: usize`, `shown: Option<i32>`, `base: Option<DiffBase>`,
`pane: Pane`, `scroll: Scroll`, `pane_rows: Cell<usize>`, `mode: Mode`, `busy: Option<&'static str>`,
`notice: Option<Notice>`, `external: Option<Pending>`, `page: Cell<u16>`.

`Mode::{Browse, Naming { name: TextField, description: TextField }, Editing(Editor)}` — the new
skill form has **two** fields, because `Skill.name` and `Skill.description` are separate columns and
a name is the library key. `Editor` mirrors the template one with `skill: Option<SkillId>`, `token:
Option<DateTime<Utc>>`, `token_version: Option<i32>`, `from: Option<i32>` (D101's two tokens).

**Keymap** (`on_browse_key`): `j`/`k` move, `,`/`.` step the shown version, `b` base, `d` diff, `e`
edit, `E` `$EDITOR`, `n` new, `r` reload, `J`/`K`/`PgUp`/`PgDn` scroll, `t` toggles the matrix pane.
All free per F-13; `w` deliberately unused.

**Layout**: the templates view's two-pane shape with `LIST_WIDTH`, the right pane carrying the
version browser, the diff (`diff::unified` + `diff::lines`) and the **token estimate** (D84) —
`TokenEstimator::DEFAULT.estimate(&version.body)` at render time, with the estimator id in the pane
title. It is `Copy`, pure, and touches no store, so `R-NF-3` holds.

**Requests**: `Skills(scope)` on activation and on `r`; `SaveSkill` on `Ctrl+S` from the editor and
`E`-then-`Ctrl+S` from Browse; `SaveSkill` from the naming form once both fields submit. Replies
`Skills` and `SkillsStale`, with `TemplatesStale`'s shape: the draft stays, the token moves to the
head the maintainer has been told about, and the notice says so in the templates view's sentence.

**Snapshots** (all new, all `skills__*`, none touching `templates__*`): `skills__browse`,
`skills__diff_two_versions`, `skills__new_name_form`, `skills__changed_elsewhere`.

**Hazards — T3**

| # | Hazard | Evidence | What to do |
|---|---|---|---|
| H-15 | The tab's frame is one widget: moving the switch line or a hint string moves all six `templates__*.snap` files | the six exist; `tests/templates.rs:905` pins the strip text | the templates snapshots must be byte-identical after T3 and T4 |
| H-16 | `wants_requests` growing to two variants looks like a staleness bug | F-12 | leave it; say why in the commit |
| H-17 | The two-token save can land a version whose description is stale | D101 | the worker short-circuits on the first `Stale`; the view's notice names which token moved |
| H-18 | A `SaveSkill` that carries a body in a plain `String` would print it in `StoreRequest`'s `Debug` | `TemplateBody`'s rule | `SkillBody` newtype, `Debug` = length |

## 5. Task 4 — the attachments matrix and the language map (D82, D83, D92, D99)

`crates/htui/src/ui/tabs/skills/matrix.rs`, declared by T4 itself once the file exists, plus
`crates/htui-core/src/model/language.rs` (`pub mod language;` at `model/mod.rs:88`).

```rust
/// A language's globs, expanded at save (ANA-22 §6 item 5, §9's list; plan D83). Data, not code,
/// like MOD-7's probe spec: a later change here never changes a saved attachment, because the
/// expansion is frozen into `skill_binding.globs` when the maintainer saves.
pub const LANGUAGE_GLOBS: &[(&str, &[&str])] = &[
    ("c", &["**/*.c", "**/*.h"]),
    ("cpp", &["**/*.cc", "**/*.cpp", "**/*.h", "**/*.hpp"]),
    ("csharp", &["**/*.cs"]),
    ("go", &["**/*.go"]),
    ("java", &["**/*.java"]),
    ("javascript", &["**/*.js", "**/*.jsx", "**/*.mjs"]),
    ("markdown", &["**/*.md"]),
    ("python", &["**/*.py"]),
    ("rust", &["**/*.rs"]),
    ("shell", &["**/*.sh", "**/*.bash", "**/*.zsh"]),
    ("sql", &["**/*.sql"]),
    ("toml", &["**/*.toml"]),
    ("typescript", &["**/*.ts", "**/*.tsx"]),
    ("yaml", &["**/*.yaml", "**/*.yml"]),
];

/// The globs a save stores: what the maintainer typed, then every named language's patterns in
/// table order, de-duplicated (D83).
#[must_use]
pub fn effective_globs(typed: &[String], languages: &[String]) -> Vec<String>;
```

A module test asserts every pattern in the table **compiles** under `glob::compile` — the map is
only useful if the matcher accepts it, and that check is free.

**The matrix** is a `format!`-packed row per skill in a bordered `Block` (the templates list pattern,
`templates.rs:876-949`) with three level columns, the global row above the projects and the phases
(ANA-22 §8), and the existing right-hand pane carrying the selected row's detail. It is **not** a
`ratatui::widgets::Table`: three files use that widget and all three have a row cursor and none a cell
cursor (D82's amended evidence).

The activation form refuses in the view with the **same sentence the store will use** (D99): it
pre-flights with `glob::compile` and renders `GlobError::Display`, and the writer's `Constraint`
carries the identical string, so the two can never disagree about why a glob was refused.

**Hazards — T4**

| # | Hazard | Evidence | What to do |
|---|---|---|---|
| H-19 | A `mod matrix;` committed before `matrix.rs` exists does not compile | the reason T3/T4 are serial (R-30) | T4 declares it, in its own first commit |
| H-20 | The view's pre-flight sentence and the writer's `Constraint` drifting apart | D99 | one `GlobError::Display`; a conformance case asserts the exact substring |
| H-21 | The language map's patterns not compiling is silent until a maintainer picks the language | D83 | the map's own test compiles every pattern |
| H-22 | `LANGUAGE_GLOBS` gaining an entry silently changes no stored attachment — which is the point, but a test could assume otherwise | D83 | the test asserts the *expansion*, not the stored row |
| H-23 | An unbind that writes `activation = 'off'` instead of deleting leaves a row that reads as attached | OQ-19 | `remove_skill_binding`, with its own conformance case |

## 6. Task 5 — the override clone (D85, D98, D100)

`NewStepGraph` (`kind.rs:152-161`) gains `pub is_override: bool`. All nine construction sites, and
what each sets:

| Site | Sets |
|---|---|
| `crates/htui-orch/src/graph.rs:410` (`override_graph`) | `true` |
| `crates/htui/src/catalogue.rs:190` (`StoreRequest::CreateGraph`) | `false` |
| `crates/htui-core/src/store/conformance.rs:3254`, `:3264` | `false` |
| `crates/htui-orch/src/conformance.rs:824`, `:884` | `false` |
| `crates/htui-orch/src/engine.rs:6152` (`Harness::repoint`) | `false` |
| `crates/htui-orch/tests/fixtures.rs:54` | `false` |
| `crates/htui-orch/tests/gix_isolator.rs:338` | `false` |

`MemStore::State::create_step_graph` (`:2511`, `is_override: false` at `:2539`) and
`PgStore::create_step_graph` (`write.rs:2203`) insert it; the latter's `.sqlx` file is **replaced**
and re-prepared, net 268 (D100).

`override_graph` copies each phase's **phase-level** attachments onto the cloned phase with
`set_skill_binding` — new `SkillBindingId`, same `skill_id`, `pinned_version`, `position`,
`activation`, `globs`, `languages`, and the cloned phase's id. It reads them through the new
`GraphSource::phase_attachments` (D98), whose four delegations are `graph.rs::TestSource`,
`fake.rs::MemStore`, `fake.rs::FakeGraphSource` and **`crates/htui/src/run_worker.rs::BackendGraphs`**
— a file the plan's T5 list does not name.

`OVERRIDE_SKILLS_NOTE` (`engine.rs:88-89`) becomes:

```
skills: an override graph's phase attachments were copied from `{graph}` when it was made and do not follow later changes there
```

Tests: `override_clone_copies_phase_attachments` (replacing `override_clone_leaves_bindings_alone`,
keeping that test's own rationale as its own assertion — the clone still copies **no** project-level
rows), `an_override_graph_is_marked_as_one`, `create_step_graph_honours_its_new_is_override_field`,
and `demo_graphs_are_not_overrides` (the demo loader's `step_graph` INSERT names six columns, so its
graphs read `false`).

**Hazards — T5**

| # | Hazard | Evidence | What to do |
|---|---|---|---|
| H-24 | Adding a `Default` for `NewStepGraph` would silently fill the field and hide a missed site | the struct has no `Default` today | no `Default` |
| H-25 | The clone loop is a partial write: a failure part-way leaves some phases copied | `override_graph` already has that shape for phases | same shape, same note; the item is repointed last |
| H-26 | `GraphSource` gains a method, so all four implementors and every fake must grow an arm | D98 | the four sites are named above; `fake.rs`'s delegation test gains a fifth read |
| H-27 | The note's `{graph}` interpolation needs the source graph's name, which `override_graph` has and the engine does not | the note is pushed in `phase_skills` | the note names the item's own graph, or the constant gains a second form; decide at implementation and say which |

## 7. Cross-task contracts

- **`BoundSkill` is not extended.** T2 adds the matched path to `SkillChoice`, not to `BoundSkill`;
  the matcher's input is `globs` plus the file set, both of which exist today.
- **`SkillAttachmentRow` (model) and `SkillBindingRow` (`pg/rows.rs`) are different types** (D93).
  The former is what the snapshot and the UI see; the latter is the store's private join result.
- **`ExcerptSet.listed` is the single source of the file set** (D72, D87). T2's union happens in
  `with_excerpts` and in `preview::build` — nowhere else, and never in the assembler, which stays
  I/O-free (ANA-5's invariant).
- **The `GlobError::Display` string is a contract** between the view, the writer and the conformance
  case (D99).
- **Nothing is mirrored.** The skill tables stay out of `MIRRORED_TABLES` (D86); `READ_CASES` stays
  14 and `CacheStore` is untouched by all five tasks.

## 8. Count pins

| Pin | Before | After |
|---|---|---|
| `WriteStore` methods | 79 | **83** |
| `CASES` / `EXPECTED_CASES` | 77 / 77 | **81 / 81** |
| `StoreRequest` variants | 69 | **73** |
| `StoreReply` variants | 40 | **42** |
| `.sqlx` files | 268 | 268 + N in T1, 268 after T2, **268** after T5 (one replaced) |
| applied migrations (`migrations.rs:81`) | `vec![1..7]` | `vec![1..8]` |
| `TABLES` | 39 | 39 |
| commented columns | 34 | 34 |
| `PromptSpec` fields | 24 | **26** |
| `SkillChoice` keys | 7 | **8** |
| trim-record top-level keys | 13 | **13** (must not move) |
| dependencies added | — | **none** |

## 9. Merge order and the workspace gate

T1 → T2 → T3 → T4 → T5, each merged into `mod-9-m3` with its own commits, then the workspace gate:
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-features --all-targets -- -D warnings`,
`USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features
--no-fail-fast -- --test-threads=2`, `cargo sqlx prepare --check`, and
`git diff --exit-code Cargo.lock Cargo.toml crates/*/Cargo.toml` (empty, by D70).

## 10. Risks carried into implementation

R-25 (a matcher bug silently matching nothing — mitigated by the dialect table and by the writer's
compile refusal), R-26 (`listed` rides on every prompt), R-27 (a glob skill fires on a mixed repo
for a docs-only item — the recorded `matched` path is the audit), R-31 (the demo/fixture asymmetry),
R-32 (an unbind deleting a moved row — the `updated_at` token), R-35 (the second `git diff`), R-34
(`ORT_LIB_LOCATION`). The plan's table is the record; nothing here supersedes it.

## 11. Departures from the plan

1. **T1 gains `crates/htui-core/src/prompt/glob.rs` and its tests** (F-1/D95). Without it T1 cannot
   compile, because D78 makes the writer refuse a glob it cannot compile.
2. **T5 gains `crates/htui/src/run_worker.rs` and a new `GraphSource` method** (F-7/D98).
3. **T1's `New*` structs live in `model/skill.rs`, not `model/kind.rs`** (F-4/D90).
4. **The read request is one per scope, not one per project** (F-2/D92).
5. **The model's row type is `SkillAttachmentRow`** (F-3/D93).
6. **`skill_matches` is an `Option<BTreeMap, …>` on the spec** (F-5/D97).
7. **An alternation is a whole component** (F-6/D96) — a narrowing the plan left open.

## 12. Decisions

| # | Decision |
|---|---|
| D90 | `NewSkill`, `NewSkillVersion`, `NewSkillBinding` live in `model/skill.rs` |
| D91 | All three carry a client-minted id (the `NewItemKind` / `NewPromptTemplate` rule) |
| D92 | One `Skills(Scope)` request for the whole scope; `wants_requests` sends two variants |
| D93 | The model's row type is `SkillAttachmentRow`; `pg/rows.rs`'s `SkillBindingRow` is untouched |
| D94 | The snapshot carries phases by name (`phase_name`) so the matrix needs no second read |
| D95 | `prompt/glob.rs` is T1's first commit, not T2's |
| D96 | An alternation is a whole component; `pre{a,b}fix.rs` is refused |
| D97 | `PromptSpec` gains `skill_files: Vec<RepoPath>` and `skill_matches: Option<BTreeMap<SkillId, String>>`; every literal grows both |
| D98 | `GraphSource::phase_attachments(project, phase) -> Result<Vec<SkillBinding>>`, four delegations including `run_worker.rs` |
| D99 | The view's pre-flight and the writer's `Constraint` share one `GlobError::Display` |
| D100 | `.sqlx`: prepared in T1 at `0007`, untouched by T2's comment-only `0008`, re-prepared in T5 (one file replaced) |
| D101 | `SaveSkill` carries both CAS tokens; the worker short-circuits on the first `Stale` |
| D102 | Every new conformance case mints fresh skill names |
| D103 | `GlobError`: one variant per refusal, each with a byte position, plus `at()` |
| D104 | `LANGUAGE_GLOBS` seeds the fourteen names ANA-22 §9 lists; the union is typed globs first, then each language in table order |
| D105 | `Cli::changed_paths` is `git diff --no-color --no-ext-diff --no-textconv --name-only -z`, sharing a `range_of` with `diff_of` |
| D106 | The new views' hints mirror `templates.rs`'s; `w` stays unbound and unused |
