# Blueprint: MOD-15 milestone 1 — the seam can write the hierarchy

Elaborates `.claude/plans/mod-15-hierarchy-seam.plan.md` (D1–D12, twelve case names final). PRD
D1–D13 win on conflict. Tree at `3c1343c`. Fact-check F1–F4 honoured: T1/T2/T3 serial; the two
`u64` keys cap at `i64::MAX`; `DeleteReach` has `phase_agents`; the `COLLATE "C"` precedent is
`crates/htui-store/src/pg/read.rs:881` and `:903`.

## 0. Flags — read before building

The plan is buildable but five things in it are wrong or unbuildable as written. None re-opens
D1–D12; each is the smallest fix that keeps the PRD's rule.

| # | Where | Problem | Resolution used below |
|---|---|---|---|
| **A** | D7/D8 `SettingSpec.key` | The App key is `prompt_upstream_hops` (`settings.rs` `DEFAULTS.as_rows()`) but the Project key `resolve_hops` reads is `upstream_hops`. One `key` cannot serve both rungs; `set_setting(Project, UpstreamHops)` would write a key the reader ignores. | `SettingSpec.project_key: Option<&'static str>`; `Some("upstream_hops")` / `Some("token_budget")`; unit test `project_key_is_some_exactly_where_project_is_accepted`. |
| **B** | D8 "JSON is an integer"; case 10 "`prompt_reserve_fraction_bp = 5001`" | The stored value is `prompt_reserve_fraction`, a JSON **float** (`0.10`), read via `as_f64` and rounded to bp. There is no integer to validate and no `_bp` key to set. PRD D7 lists `kind` in the spec; the plan dropped it. | `SettingSpec.kind: SettingKind { Integer, Fraction }` (PRD wins). Fraction rule: finite, `>= 0.0`, `round(f*10_000) <= 5000`. Case 10 sends `json!(0.5001)`. |
| **C** | D8 Phase rung | `step_graph_phase.token_budget` is `INTEGER` (`Option<i32>`); `SPECS[TokenBudget].max = i64::MAX` would let `set_setting(Phase, ..)` accept a value the column cannot hold (`22003` on Pg, silent truncation risk on Mem). | `validate` narrows `max` to `i32::MAX` when `at == Rungs::PHASE`. |
| **D** | D8 `clear_setting` | On the App rung a clear is `DELETE`; there is no row left to supply `StoredSetting.updated_at`. Also `expected` for a clear can never sensibly be `None` (case 10 clears with the token). | `clear_setting(.., expected: DateTime<Utc>)` (not `Option`); App clear returns `Applied(StoredSetting { value: None, updated_at })` with the **deleted** row's token (`DELETE .. RETURNING updated_at`); doc says the next `set_setting(App)` passes `expected: None`. |
| **E** | T1 validate vs D12 twins | `every_cross_referenced_test_name_exists` (`conformance.rs:2119`) fails T1 if a case doc names a `pg_criteria.rs::` twin that only T2 writes. | T1 doc comments name only `mem.rs::` twins; T2 appends the two `pg_criteria.rs::` references when it adds those tests. Also: any backticked span with ≥ 4 underscores must be a fn in `conformance.rs` or `mem.rs` — write prose accordingly. |
| **F** | D4/`MemStore` | `State` has no `phase_agents` or `run_step_commits` map; Pg demo (`pg/demo.rs`) seeds neither table, nor `repo`, `repo_box_path`, `workspace_box_path`, `app_setting`. | Those two counts are `0` on both stores; case 5 asserts equality of the whole struct and non-zero only for the listed fields. Not a defect, but the doc on `DeleteReach` says it. |
| **G** | T1 `ranges_are_the_readers_clamps` | `HOPS_RANGE` and `MAX_RESERVE_BP` are private `const`s in `settings.rs`. | The test lives in `settings.rs`'s own `#[cfg(test)] mod`; the consts stay private. |
| **H** | T1 "three new `State` fields" | `State.app_settings: BTreeMap<String, Value>` (`mem.rs:80`) carries no `updated_at`, so `setting(App)`/CAS on App have no token on `MemStore`. | Retype to `BTreeMap<String, (Value, DateTime<Utc>)>`; `app_settings()` (`mem.rs:368`) projects `.1` away; `set_app_setting` (`:377`) stores `(value, Utc::now())` and its "**tests only**, the only writer" doc (`:79`, `:372-377`) is amended. Three new fields plus one retype. |
| **I** | Files table / T2 "22 writers + `delete_reach`" | 21 writers + `delete_reach` + 9 readers = 31; "22" counts `delete_reach` twice. | Arithmetic in §1. |
| **J** | D7 `not_above` | One-directional: lowering `excerpt_file_line_cap` below the current `excerpt_head_lines` is accepted, and afterwards the reader clamps `head_lines`. PRD metric "every accepted value survives its resolver" holds for the written value only. | Not changed (plan is authority); recorded so milestone 3's editor can show the consequence. A `not_below` twin is one field if wanted. |
| **K** | case 11 "byte-identical" | JSONB normalises key order and number text; across Pg, byte identity cannot be asserted. | Conformance case asserts per-key `Value` equality of every other key; the `mem.rs` twin asserts `to_string()` bytes. |

## 1. `WriteStore` — the 31 methods, in `traits.rs` order

File: `crates/htui-core/src/store/traits.rs`. Append after `set_step_prompt` (line ~293),
replacing the trailing `// links, notes, templates, box ...` comment. New imports at the top:

```rust
use crate::model::ids::{ItemKindId, PhaseId, ProjectId, RepoId, StepGraphId, WorkspaceId};
use crate::model::{
    ItemKind, ItemKindPatch, NewItemKind, NewProject, NewRepo, NewStepGraph, NewWorkspace,
    PhasePatch, Project, ProjectPatch, Repo, RepoBoxPath, RepoPatch, StepGraph, StepGraphPatch,
    StepGraphPhase, Workspace, WorkspaceBoxPath, WorkspacePatch, WorkspaceProject,
};
use crate::prompt::settings::{Rungs, SettingKey};
```

Count: writers — workspace 2, links/paths 3, project 2, repo 3, item_kind 2, graph/phase 4,
settings 2, deletes 3 = **21**; plus `delete_reach` = 22; readers — `workspace`,
`workspace_projects`, `workspace_box_paths`, `repos`, `repo_box_paths`, `item_kinds`,
`step_graphs`, `phases`, `setting` = **9**; total **31**.

```rust
    // ---- MOD-15 milestone 1: the hierarchy (plan D1-D12) -----------------------------------
    //
    // Every edit is a compare-and-set on `updated_at` (D3): the caller passes the token it edited
    // from, the trigger (`clock_timestamp()`) writes the next one, no statement here sets it.
    // `workspace_project` has no `updated_at` and the two path tables are per-box rows with one
    // writer each, so those three are plain upserts. Readers are here rather than on `ReadStore`
    // (D1) so the conformance suite can read back what it wrote on both stores.

    // workspace

    /// Inserts a workspace; the returned row carries the store's clock, not the caller's.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `slug` is taken or
    /// `created_by` names no user.
    async fn create_workspace(&self, new: NewWorkspace) -> Result<Workspace>;

    /// Edits `slug` / `name` / `description` when the row's `updated_at` still equals `expected`;
    /// `Stale` carries the row as it is now so the editor can reload (D3).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the new `slug`
    /// collides.
    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> Result<CasOutcome<Workspace>>;

    /// One workspace by id, `None` when there is no such row.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace(&self, id: WorkspaceId) -> Result<Option<Workspace>>;

    // workspace links and box paths

    /// Inserts or repositions a workspace to project link (PK `(workspace_id, project_id)`, no
    /// `updated_at`, so no CAS).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> Result<()>;

    /// Removes one link; the project survives (D4).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) (`entity: "workspace_project"`)
    /// when no such link exists.
    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> Result<()>;

    /// A workspace's links ordered by `position`, then `project_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace_projects(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceProject>>;

    /// Inserts or replaces this box's root path for a workspace (PK `(workspace_id, box_id)`); the
    /// trigger advances `updated_at` on replace.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> Result<()>;

    /// Every box's root path for a workspace, ordered by `box_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace_box_paths(&self, workspace: WorkspaceId) -> Result<Vec<WorkspaceBoxPath>>;

    // project

    /// Inserts a project with `settings = {}` and no secret provider (D9); milestone 2 seeds the
    /// graphs, kinds and templates inside this same transaction.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `slug` is taken or
    /// `created_by` names no user.
    async fn create_project(&self, new: NewProject) -> Result<Project>;

    /// Edits `slug` / `name` / `description` under CAS; never touches `settings` (that is
    /// [`set_setting`](Self::set_setting)'s).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `slug` collision.
    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> Result<CasOutcome<Project>>;

    // repo and repo box paths

    /// Inserts a repo. `is_primary: true` clears the project's current primary in the same
    /// transaction so `uq_repo_primary` never trips (D10).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `(project_id, name)`
    /// is taken or `project_id` names no row.
    async fn create_repo(&self, new: NewRepo) -> Result<Repo>;

    /// Edits under CAS; `is_primary: Some(true)` demotes the other primary (its `updated_at`
    /// advances too), `Some(false)` just unsets; `remote_url: Some(None)` clears the URL.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `name` collision.
    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> Result<CasOutcome<Repo>>;

    /// A project's repos ordered by `name` bytes (`COLLATE "C"`, as `prompt_templates`).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn repos(&self, project: ProjectId) -> Result<Vec<Repo>>;

    /// Inserts or replaces this box's checkout path for a repo (PK `(repo_id, box_id)`).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row.
    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> Result<()>;

    /// Every box's checkout path for a repo, ordered by `box_id` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn repo_box_paths(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>>;

    // item_kind

    /// Inserts a kind. The prefix is checked by [`ItemKind::prefix_is_valid`] before the statement
    /// and `default_graph_id` must belong to the same project (D11).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a bad prefix, a taken
    /// `(project_id, prefix)` or `(project_id, name)`, or a graph from another project.
    async fn create_item_kind(&self, new: NewItemKind) -> Result<ItemKind>;

    /// Edits under CAS with the same three rules as [`create_item_kind`](Self::create_item_kind).
    /// Renaming the prefix leaves existing keys (`ANA-2`) and counters alone; the next mint under
    /// the kind starts a counter for the new prefix (PRD D12).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) as for create.
    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> Result<CasOutcome<ItemKind>>;

    /// A project's kinds ordered by `position`, then `prefix` bytes (`position` is not unique).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>>;

    /// Deletes a kind nothing references (D6). `item_key_counter` is keyed by prefix and is never
    /// touched.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint)
    /// `"item_kind FEAT is held by 4 items"` while items reference it;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_item_kind(&self, id: ItemKindId) -> Result<()>;

    // step_graph and phase

    /// Inserts a graph (no phases; [`create_phase`](Self::create_phase) adds them).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `(project_id, name)`
    /// is taken or `project_id` names no row.
    async fn create_step_graph(&self, new: NewStepGraph) -> Result<StepGraph>;

    /// Edits `name` / `description` under CAS.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) on a `name` collision.
    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> Result<CasOutcome<StepGraph>>;

    /// A project's graphs ordered by `name` bytes.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn step_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>>;

    /// Inserts a whole phase row (D10); `phase.updated_at` is ignored and the store's clock is
    /// returned. `judge` and `handoff` are template roles, not phase names
    /// ([`TemplateRole::of_name`](crate::prompt::template::TemplateRole::of_name)).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a reserved name, a
    /// taken `(graph_id, position)` or `(graph_id, name)`, or a `graph_id` that names no row.
    async fn create_phase(&self, phase: &StepGraphPhase) -> Result<StepGraphPhase>;

    /// Edits the five columns of [`PhasePatch`] under CAS; `token_budget` is
    /// [`set_setting`](Self::set_setting)'s on the `Phase` rung and is not here (D8).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a reserved name or a
    /// `(graph_id, position)` / `(graph_id, name)` collision.
    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> Result<CasOutcome<StepGraphPhase>>;

    /// A graph's phases ordered by `position` (unique per graph).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn phases(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>>;

    // settings (D7, D8)

    /// Writes one setting on one rung after [`validate`](crate::prompt::settings::validate):
    /// rung, kind, range, `not_above` against the same rung's current peer (or its default).
    /// `App` is an `app_setting` row (`expected: None` = "I expect no row", the insert after a
    /// clear; `Stale` if one exists); `Project` merges one key into `project.settings` under CAS
    /// on `project.updated_at`; `Phase` writes `step_graph_phase.token_budget` under CAS on the
    /// phase's `updated_at`. `Stale` carries the setting as stored now.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with the key, value and
    /// rule for every validation refusal, and for `expected: None` on `Project` / `Phase`;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown project or
    /// phase.
    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<StoredSetting>>;

    /// Removes one setting from one rung under CAS: `DELETE` on `app_setting` (the returned
    /// `updated_at` is the deleted row's; the next `set_setting` passes `expected: None`),
    /// `settings - key` on the project, `NULL` on the phase. `Applied` always carries
    /// `value: None`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the key is not
    /// accepted on the rung; [`StoreError::NotFound`](crate::store::StoreError::NotFound) when
    /// the row (or, on `App`, the setting) does not exist.
    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<StoredSetting>>;

    /// One setting on one rung with its CAS token. `None` only when the rung's row is absent
    /// (`App`: no such `app_setting`; `Project` / `Phase`: no such id); a present project or
    /// phase without the key answers `Some` with `value: None`.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when the key is not
    /// accepted on the rung.
    async fn setting(&self, rung: SettingRung, key: SettingKey) -> Result<Option<StoredSetting>>;

    // deletes (D4)

    /// What a delete would remove, counted without removing; `None` when the target does not
    /// exist. The same counting code feeds [`delete_workspace`](Self::delete_workspace) and
    /// [`delete_project`](Self::delete_project), so the report equals the act (PRD D13).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn delete_reach(&self, target: DeleteTarget) -> Result<Option<DeleteReach>>;

    /// Removes a workspace, its links and its box paths; projects survive
    /// (`0001_init.sql:162,174`).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_workspace(&self, id: WorkspaceId) -> Result<DeleteReach>;

    /// Removes a project and everything PRD D13 lists, in one transaction, and returns the counts
    /// it took. The mirror rebuild is the caller's (D5).
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown id.
    async fn delete_project(&self, id: ProjectId) -> Result<DeleteReach>;
}
```

Also in `traits.rs`, beside `not_a_terminal_status` (~line 315), four text helpers so both stores
emit one sentence per rule (D11 "checked in htui-core once"):

```rust
/// D11: the `item_kind.prefix` CHECK, in words.
#[must_use]
pub fn invalid_prefix(prefix: &str) -> String {
    format!("item_kind.prefix `{prefix}` is not `^[A-Z][A-Z0-9]{{1,15}}$`")
}
/// D11: `default_graph_id` must be one of the kind's own project's graphs.
#[must_use]
pub fn graph_not_in_project(graph: StepGraphId, project: ProjectId) -> String {
    format!("step_graph {graph} is not in project {project}")
}
/// D11: `judge` / `handoff` are template roles (ANA-5 §4.6), not phase names.
#[must_use]
pub fn reserved_phase_name(name: &str) -> String {
    format!("`{name}` is a reserved template name, not a phase name")
}
/// D6: the holder count, in the sentence the case asserts on.
#[must_use]
pub fn item_kind_is_held(prefix: &str, items: u64) -> String {
    format!("item_kind {prefix} is held by {items} items")
}
```

`crates/htui-core/src/store/mod.rs` re-export line becomes
`pub use traits::{CasOutcome, DeleteReach, DeleteTarget, MAX_UPSTREAM_HOPS, ReadStore, SettingRung, StoredSetting, UpdateOutcome, WriteStore, chat_step_status, graph_not_in_project, invalid_prefix, item_kind_is_held, not_a_terminal_status, reserved_phase_name};`

## 2. New seam types (`traits.rs`, immediately after `UpdateOutcome`, line ~333)

Derives match the neighbour: `UpdateOutcome` is `#[derive(Debug, Clone, PartialEq)]`; id newtypes
are `Copy + Eq + Hash`; `serde_json::Value` is `Eq`.

```rust
/// Result of a compare-and-set edit (D3). `Applied` carries the row the trigger stamped;
/// `Stale` carries the row as it is now, because the token the caller edited from no longer
/// matches, so the editor can reload and retry (PRD D8).
#[derive(Debug, Clone, PartialEq)]
pub enum CasOutcome<T> {
    Applied(T),
    Stale(T),
}

impl<T> CasOutcome<T> {
    /// The row either way; the conformance cases read back through this.
    pub fn into_inner(self) -> T {
        match self {
            Self::Applied(row) | Self::Stale(row) => row,
        }
    }
}

/// What [`WriteStore::delete_reach`] counts for (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Workspace(WorkspaceId),
    Project(ProjectId),
}

/// Rows a delete removes, per table, in `0001_init.sql`'s cascade order (PRD D13; F3 added
/// `phase_agents`). A workspace delete fills `workspace_links` and `workspace_box_paths` only.
/// `phase_agents` and `run_step_commits` are `0` on `MemStore`, which holds neither table, and
/// `0` on the demo database, which seeds neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteReach {
    pub workspace_links: u64,
    pub workspace_box_paths: u64,
    pub items: u64,
    pub item_key_counters: u64,
    pub item_kinds: u64,
    pub step_graphs: u64,
    pub phases: u64,
    pub phase_agents: u64,
    pub prompt_templates: u64,
    pub repos: u64,
    pub repo_box_paths: u64,
    pub skill_bindings: u64,
    pub runs: u64,
    pub run_steps: u64,
    pub session_events: u64,
    pub run_step_commits: u64,
    pub notes: u64,
    pub revisions: u64,
    pub links: u64,
    pub documents: u64,
}

/// Where a setting lives (D8): an `app_setting` row, one key of `project.settings`, or
/// `step_graph_phase.token_budget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRung {
    App,
    Project(ProjectId),
    Phase(PhaseId),
}

impl SettingRung {
    /// The flag [`validate`](crate::prompt::settings::validate) checks against
    /// `SettingSpec::rungs`.
    #[must_use]
    pub const fn flag(self) -> Rungs {
        match self {
            Self::App => Rungs::APP,
            Self::Project(_) => Rungs::PROJECT,
            Self::Phase(_) => Rungs::PHASE,
        }
    }
}

/// One setting as stored, with the token a later `set_setting` / `clear_setting` must present.
/// `value: None` is "the rung's row exists but holds no value for this key".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSetting {
    pub value: Option<Value>,
    pub updated_at: DateTime<Utc>,
}
```

## 3. Request and patch structs (`model/`)

Mirror `NewItem` / `ItemPatch` in `crates/htui-core/src/model/item.rs`:
`#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`, patches add `Default`; nullable
columns use `Option<Option<T>>` as `ItemPatch::step_graph_id` does; doc line "Arguments of
[`crate::store::WriteStore::create_x`]" / "Edit passed to
[`crate::store::WriteStore::update_x`]".

`crates/htui-core/src/model/hierarchy.rs` (after `Workspace`, `Project`, `Repo` respectively):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewWorkspace { pub id: WorkspaceId, pub slug: String, pub name: String, pub description: String, pub created_by: UserId }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePatch { pub slug: Option<String>, pub name: Option<String>, pub description: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewProject { pub id: ProjectId, pub slug: String, pub name: String, pub description: String, pub created_by: UserId }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectPatch { pub slug: Option<String>, pub name: Option<String>, pub description: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRepo { pub id: RepoId, pub project_id: ProjectId, pub name: String, pub remote_url: Option<String>, pub default_branch: String, pub is_primary: bool }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoPatch { pub name: Option<String>, pub remote_url: Option<Option<String>>, pub default_branch: Option<String>, pub is_primary: Option<bool> }
```

`crates/htui-core/src/model/kind.rs` (after `ItemKind`, `StepGraph`, `StepGraphPhase`):

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewItemKind { pub id: ItemKindId, pub project_id: ProjectId, pub prefix: String, pub name: String, pub description: String, pub default_graph_id: StepGraphId, pub position: i32 }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemKindPatch { pub prefix: Option<String>, pub name: Option<String>, pub description: Option<String>, pub default_graph_id: Option<StepGraphId>, pub position: Option<i32> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewStepGraph { pub id: StepGraphId, pub project_id: ProjectId, pub name: String, pub description: String }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepGraphPatch { pub name: Option<String>, pub description: Option<String> }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhasePatch { pub name: Option<String>, pub position: Option<i32>, pub template_name: Option<String>, pub gate_hard: Option<bool>, pub input_kinds: Option<Vec<String>> }

impl ItemKind {
    /// The `item_kind.prefix` CHECK `^[A-Z][A-Z0-9]{1,15}$` (`0001_init.sql:282-290`) without a
    /// regex crate: 2..=16 bytes, first `A-Z`, the rest `A-Z0-9`.
    #[must_use]
    pub fn prefix_is_valid(prefix: &str) -> bool {
        let bytes = prefix.as_bytes();
        (2..=16).contains(&bytes.len())
            && bytes[0].is_ascii_uppercase()
            && bytes[1..].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    }
}
```

`crates/htui-core/src/model/mod.rs`: extend the two `pub use` lines with `NewProject, NewRepo,
NewWorkspace, ProjectPatch, RepoPatch, WorkspacePatch` and `ItemKindPatch, NewItemKind,
NewStepGraph, PhasePatch, StepGraphPatch`.

Unit test (first red test of step 2), `kind.rs` `#[cfg(test)] mod tests`:
`prefix_is_valid_mirrors_the_check` — accepts `ANA`, `A1`, `A` + 15 × `9`; refuses `feat`, `1A`,
`A`, 17 chars, `AN-A`, `ÄNA`.

## 4. Settings registry (`crates/htui-core/src/prompt/settings.rs`)

Placed after `Defaults` / `DEFAULTS`, before the `positive_*` helpers. Discriminants are **key byte
order** so `SettingKey::ALL` reproduces `as_rows()`'s order (pinned by `as_rows_is_key_byte_order`)
and `SPECS[key as usize]` is the spec.

```rust
/// The ten `app_setting` keys migration `0002` seeds (ANA-5 §4.4), declared in key byte order:
/// the discriminant indexes [`SPECS`] and [`Self::ALL`] is [`Defaults::as_rows`]'s order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingKey {
    ExcerptFileLineCap = 0,
    ExcerptHeadLines = 1,
    ExcerptMaxFileBytes = 2,
    ExcerptMaxFiles = 3,
    ExcerptMaxScanFiles = 4,
    ExcerptProviderDeadlineMs = 5,
    MaxSkillTokens = 6,
    PromptReserveFraction = 7,
    UpstreamHops = 8,
    TokenBudget = 9,
}

impl SettingKey {
    pub const ALL: [Self; 10] = [
        Self::ExcerptFileLineCap, Self::ExcerptHeadLines, Self::ExcerptMaxFileBytes,
        Self::ExcerptMaxFiles, Self::ExcerptMaxScanFiles, Self::ExcerptProviderDeadlineMs,
        Self::MaxSkillTokens, Self::PromptReserveFraction, Self::UpstreamHops, Self::TokenBudget,
    ];
    /// The row's spec.
    #[must_use]
    pub const fn spec(self) -> &'static SettingSpec { &SPECS[self as usize] }
    /// The `app_setting.key` column value.
    #[must_use]
    pub const fn key(self) -> &'static str { self.spec().key }
    /// The reverse of [`Self::key`].
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> { Self::ALL.into_iter().find(|k| k.key() == key) }
}

impl core::fmt::Display for SettingKey { /* writes self.key() */ }

/// Which rungs accept a key (D7): a three-flag bitset, no `bitflags` dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rungs(u8);

impl Rungs {
    pub const APP: Self = Self(1);
    pub const PROJECT: Self = Self(2);
    pub const PHASE: Self = Self(4);
    #[must_use]
    pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
    #[must_use]
    pub const fn or(self, other: Self) -> Self { Self(self.0 | other.0) }
}

impl core::ops::BitOr for Rungs { type Output = Self; fn bitor(self, rhs: Self) -> Self { self.or(rhs) } }
impl core::fmt::Display for Rungs { /* "app", "project", "phase", joined by '|' */ }

/// How the JSON value is read (PRD D7's `kind`): every key but one is an integer;
/// `prompt_reserve_fraction` is a float the reader rounds to basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Integer,
    Fraction,
}

/// One row of the registry (D7). `min`/`max` are the reader's own clamps, in `unit`; for
/// [`SettingKind::Fraction`] they are basis points of the float.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingSpec {
    pub key: &'static str,
    /// The name under which `project.settings` holds it; `Some` iff `rungs` has `PROJECT`.
    pub project_key: Option<&'static str>,
    pub kind: SettingKind,
    pub min: i64,
    pub max: i64,
    pub rungs: Rungs,
    pub not_above: Option<SettingKey>,
    pub unit: &'static str,
    pub doc: &'static str,
}

const U32_MAX: i64 = u32::MAX as i64;

pub const SPECS: [SettingSpec; 10] = [
    SettingSpec { key: "excerpt_file_line_cap", project_key: None, kind: SettingKind::Integer, min: 1, max: U32_MAX, rungs: Rungs::APP, not_above: None, unit: "lines", doc: "Lines of one file the excerpt scanner reads." },
    SettingSpec { key: "excerpt_head_lines", project_key: None, kind: SettingKind::Integer, min: 1, max: U32_MAX, rungs: Rungs::APP, not_above: Some(SettingKey::ExcerptFileLineCap), unit: "lines", doc: "Lines quoted from the top of a file; the reader clamps it to excerpt_file_line_cap." },
    SettingSpec { key: "excerpt_max_file_bytes", project_key: None, kind: SettingKind::Integer, min: 1, max: i64::MAX, rungs: Rungs::APP, not_above: None, unit: "bytes", doc: "Largest file the scanner opens (read as i64, F2)." },
    SettingSpec { key: "excerpt_max_files", project_key: None, kind: SettingKind::Integer, min: 1, max: U32_MAX, rungs: Rungs::APP, not_above: None, unit: "files", doc: "Files quoted per prompt." },
    SettingSpec { key: "excerpt_max_scan_files", project_key: None, kind: SettingKind::Integer, min: 1, max: U32_MAX, rungs: Rungs::APP, not_above: None, unit: "files", doc: "Files the scanner visits before it stops." },
    SettingSpec { key: "excerpt_provider_deadline_ms", project_key: None, kind: SettingKind::Integer, min: 1, max: i64::MAX, rungs: Rungs::APP, not_above: None, unit: "ms", doc: "Wall clock the excerpt provider gets (read as i64, F2)." },
    SettingSpec { key: "max_skill_tokens", project_key: None, kind: SettingKind::Integer, min: 1, max: i64::MAX, rungs: Rungs::APP, not_above: None, unit: "tokens", doc: "Tokens the skill section may take." },
    SettingSpec { key: "prompt_reserve_fraction", project_key: None, kind: SettingKind::Fraction, min: 0, max: 5_000, rungs: Rungs::APP, not_above: None, unit: "bp", doc: "Share of the budget held back for the answer, 0.0 to 0.5." },
    SettingSpec { key: "prompt_upstream_hops", project_key: Some("upstream_hops"), kind: SettingKind::Integer, min: 1, max: 2, rungs: Rungs::APP.or(Rungs::PROJECT), not_above: None, unit: "hops", doc: "Upstream items followed when quoting context." },
    SettingSpec { key: "token_budget", project_key: Some("token_budget"), kind: SettingKind::Integer, min: 1, max: i64::MAX, rungs: Rungs::APP.or(Rungs::PROJECT).or(Rungs::PHASE), not_above: None, unit: "tokens", doc: "Prompt budget; the phase column is INTEGER, so that rung caps at i32::MAX." },
];

/// Refuses what the reader would clamp or ignore (D7, D8), in order: rung, kind, range,
/// `not_above`. `peer` is the same rung's current value of `not_above`'s key, or `None` for
/// its default. The `String` is the whole refusal; the store wraps it in `StoreError::Constraint`.
///
/// # Errors
/// The sentence to show, one of:
/// - "`{key}` is not accepted on the {rung} rung"
/// - "`{key}` must be a JSON integer, got {value}" / "`{key}` must be a finite JSON number, got {value}"
/// - "`{key}` = {n} is outside {min}..={max} {unit}"
/// - "`{key}` = {n} is above `{other}` = {m}; the reader would clamp it"
pub fn validate(key: SettingKey, at: Rungs, value: &Value, peer: Option<&Value>) -> Result<(), String>
```

Body notes: Integer → `value.as_i64()`; `max` becomes `spec.max.min(i64::from(i32::MAX))` when
`at == Rungs::PHASE` (flag C). Fraction → `value.as_f64().filter(|f| f.is_finite())`, refuse
`f < 0.0`, `n = (f * 10_000.0).round() as i64`, range on `n` (range text quotes `value` and `n` bp).
`not_above` → `m = peer.and_then(Value::as_i64).unwrap_or_else(|| DEFAULTS.integer(other))`.

`Defaults` gains `pub fn value_of(&self, key: SettingKey) -> Value` (fraction emitted as the same
`f64` expression `as_rows` uses today, so the `0002` pin stays byte-for-byte) and
`fn integer(&self, key: SettingKey) -> i64` (the nine integer keys; `PromptReserveFraction` answers
bp). `as_rows()` becomes `SettingKey::ALL` mapped through `(k.key(), self.value_of(k))`, same return
type. Nothing else in the file moves; `HOPS_RANGE`, `MAX_RESERVE_BP` stay private.

`crates/htui-core/src/prompt/mod.rs`: `pub use settings::{Budget, BudgetSource, DEFAULTS, Defaults,
Rungs, SPECS, SettingKey, SettingKind, SettingSpec, validate};`

Unit tests (step 3's red tests, `settings.rs` test module):
`every_default_validates_under_its_own_spec` (each key, `Rungs::APP`, `DEFAULTS.value_of(k)`,
`peer: None`); `ranges_are_the_readers_clamps` (`UpstreamHops` min/max == `HOPS_RANGE`;
`PromptReserveFraction.max == i64::from(MAX_RESERVE_BP)`; four `u32` keys `max == U32_MAX`; two
`u64` keys and `MaxSkillTokens`/`TokenBudget` `max == i64::MAX`; `ExcerptHeadLines.not_above ==
Some(ExcerptFileLineCap)`); `project_key_is_some_exactly_where_project_is_accepted`;
`specs_are_indexed_by_discriminant` (`SPECS[k as usize].key == k.key()` and keys strictly ascending
as bytes); `an_accepted_value_survives_its_resolver` (hops 1 and 2 through `resolve_hops`, fraction
0.25 → 2500 through `resolve_reserve_bp`, head_lines == cap through `resolve_excerpt_caps`);
`phase_rung_caps_token_budget_at_i32` (`i64::from(i32::MAX) + 1` refused on PHASE, accepted on APP).
Existing `the_defaults_are_migration_0002s_ten_rows_verbatim` and `as_rows_is_key_byte_order`
untouched.

## 5. `MemStore` `State` deltas (`crates/htui-core/src/store/mem.rs`)

| Field | Type | Position | `from_demo` |
|---|---|---|---|
| `workspace_box_paths` | `Vec<WorkspaceBoxPath>` | after `workspace_projects` (line ~58) | `Vec::new()` |
| `repos` | `HashMap<RepoId, Repo>` | after `projects` (line ~59) | `HashMap::new()` |
| `repo_box_paths` | `Vec<RepoBoxPath>` | after `repos` | `Vec::new()` |
| `app_settings` (retype, flag H) | `BTreeMap<String, (Value, DateTime<Utc>)>` | unchanged (line 80) | `BTreeMap::new()` |

`#[expect(dead_code, ..)]` comes off `graphs` (line 61) and `phases` (line 64): `step_graphs()` /
`phases()` / `delete_project` read them. `app_settings()` (`:368`) maps `(k, (v, _))` → `(k, v)`;
`set_app_setting` (`:377`) stores `(value, Utc::now())`, its doc drops "the only writer".
`DemoData` unchanged (no repos/paths in the fixture).

Impl shape: each of the 31 arms is `let now = Utc::now(); self.write(|state| state.<verb>(.., now))`
or `self.read(|state| ..)`; the rules live in `impl State` so `delete_reach` and `delete_project`
share one `fn project_reach(&self, id) -> Option<(DeleteReach, ProjectReach)>` (the id sets below)
and cannot disagree. CAS on Mem: compare `row.updated_at == expected`, on match apply and set
`row.updated_at = now`. Note K applies: `Utc::now()` is ns-resolution on Linux, so two lock
acquisitions never share a token; if a case ever reports `Applied` on a spent token, add a monotonic
tick to `State` rather than sleeping in the case.

Settings on Mem: App → the tuple map (`expected: None` ⇒ `Stale` if present; `Some(t)` ⇒ `NotFound`
if absent, `Stale` if `t != stored.1`); Project → `Value::Object` insert/remove of
`spec.project_key` on `project.settings` (if the blob is not an object, `Constraint`), bump
`project.updated_at`; Phase → `phase.token_budget = Some(i32)` / `None`, bump `phase.updated_at`.

## 6. `MemStore::delete_project(p)` cascade — explicit `State` field order

Collect first, under one `write`: `items_gone` = `items` with `project_id == p`; `runs_gone` =
`runs` with `project_id == p || item_id ∈ items_gone`; `steps_gone` = `steps` with `run_id ∈
runs_gone`; `graphs_gone` = `graphs` with `project_id == p`; `phases_gone` = `phases` with `graph_id
∈ graphs_gone`; `repos_gone` = `repos` with `project_id == p`. Then remove and count, in this order
(`DeleteReach` field named):

1. `events` where `run_step_id ∈ steps_gone` → `session_events`
2. `steps` ∈ `steps_gone` → `run_steps`
3. `runs` ∈ `runs_gone` → `runs`
4. `documents` where `item_id ∈ items_gone` → `documents`
5. `notes` where `item_id ∈ items_gone` → `notes`
6. `revisions` where `key.0 ∈ items_gone` → `revisions`
7. `links` where `from ∈ items_gone || to ∈ items_gone` (tombstones included; this is what takes
   `AGY_FEAT_1`'s cross-project link) → `links`
8. `items` ∈ `items_gone` → `items`
9. `item_key_counter` where `key.0 == p` → `item_key_counters`
10. `skill_bindings` where `project_id == p` → `skill_bindings`
11. `kinds` where `project_id == p` → `item_kinds`
12. `phases` ∈ `phases_gone` → `phases`
13. `graphs` ∈ `graphs_gone` → `step_graphs`
14. `templates` where `project_id == p` → `prompt_templates`
15. `repo_box_paths` where `repo_id ∈ repos_gone` → `repo_box_paths`
16. `repos` ∈ `repos_gone` → `repos`
17. `workspace_projects` where `project_id == p` → `workspace_links`
18. `projects.remove(&p)`

`phase_agents`, `run_step_commits`, `workspace_box_paths` = 0 for a project target. Untouched:
`users`, `boxes`, `this_box`, `workspaces`, `workspace_box_paths`, `skills`, `skill_versions`,
`box_tools`, `app_settings`, `agents`, `agent_boxes`. `delete_workspace(w)`: `workspace_projects`
where `workspace_id == w` → `workspace_links`; `workspace_box_paths` where `workspace_id == w` →
`workspace_box_paths`; `workspaces.remove(&w)`; all else 0.

Fixture expectations for case 5 on `PROJECT_HTUI`: `item_kinds 5`, `phases 15`, `step_graphs 5`,
`prompt_templates 10` exact; `workspace_links 1`; `skill_bindings 3`; `items 8`; `repos`,
`repo_box_paths`, `phase_agents`, `run_step_commits`, `workspace_box_paths` 0 on both stores
(flag F).

## 7. Build order inside T1 (file by file, compile status after each)

| Step | File | Work | First red test lives in | `cargo check -p htui-core` | `.. --all-features` | `-p htui-store` |
|---|---|---|---|---|---|---|
| 1 | `store/conformance.rs`, `tests/mem_store.rs` | twelve case fns, twelve `CASES` entries, twelve `run_case` arms, count pin 23 → 35 | `conformance.rs` (the cases) | green | **red** (unresolved names) | green |
| 2 | `model/hierarchy.rs`, `model/kind.rs`, `model/mod.rs` | request/patch structs, `prefix_is_valid`, re-exports | `kind.rs::prefix_is_valid_mirrors_the_check` | green | red | green |
| 3 | `prompt/settings.rs`, `prompt/mod.rs` | `SettingKey`, `Rungs`, `SettingKind`, `SettingSpec`, `SPECS`, `validate`, `Defaults::value_of`, `as_rows` over `ALL` | `settings.rs` test module (the six tests in §4) | green | red | green |
| 4 | `store/traits.rs`, `store/mod.rs` | seam types, 31 methods, four text helpers, re-exports | none (types step) | **red** (`MemStore` lacks 31 impls) | red | **red — stays red until T2 and T3; expected, F1 serial** |
| 5 | `store/mem.rs` | `State` deltas (§5), `app_settings` retype, dead_code off, 31 impls, cascade (§6) | the twelve cases now run: red on behaviour, then green | green | green | red (expected) |
| 6 | `store/mem.rs` tests module | `set_setting_project_rung_leaves_unknown_keys_byte_identical`, `delete_project_leaves_no_row_in_any_map` | `mem.rs` | green | green | red (expected) |

T1's gate is crate-scoped: `cargo test -p htui-core --all-features` and
`cargo clippy -p htui-core --all-features`. The workspace gate returns green at the end of T3.
T2 adds `pg/write.rs` (21 writers + `delete_reach`, `.sqlx` regenerated and committed) and
`pg/read.rs` (9 readers, `ORDER BY name COLLATE "C"`), plus the two `pg_criteria.rs` twins and
their doc references (flag E); T3 adds
`fn hierarchy_needs_the_server() -> StoreError { StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()) }`
beside `item_writes_need_the_server` (`writer.rs:402`), 31 refusing arms on `BufferedWriter` and 31
delegating arms on `Writer`.

## 8. Conformance skeleton (`crates/htui-core/src/store/conformance.rs`)

Fixture constants verified present in `crates/htui-core/src/fixtures.rs` `pub mod ids`:
`PROJECT_HTUI`, `PROJECT_AGY`, `KIND_HTUI_ANA`, `HTUI_ANA_2`, `AGY_FEAT_1`, `PHASE_HTUI_IMPLEMENT`,
`GRAPH_HTUI_FEAT` (also used: `USER`, `BOX`, `WORKSPACE_PLATFORM`, `WORKSPACE_GRAPHICS`,
`PROJECT_VULKAN`, `HTUI_FEAT_2`).

Imports grow: `use crate::model::{.., NewItemKind, NewProject, NewRepo, NewStepGraph, NewWorkspace,
ItemKindPatch, PhasePatch, ProjectPatch, RepoPatch, StepGraphPatch, WorkspacePatch,
WorkspaceProject, WorkspaceBoxPath, RepoBoxPath, StepGraphPhase, ..}; use
crate::store::traits::{CasOutcome, DeleteReach, DeleteTarget, ReadStore, SettingRung,
UpdateOutcome, WriteStore}; use crate::prompt::settings::SettingKey;`.

Registration:

```rust
pub const CASES: &[&str] = &[
    // ... the 23 existing names ...
    "workspace_round_trip_and_cas",
    "workspace_links_and_box_paths_upsert",
    "workspace_delete_reports_its_reach",
    "project_create_update_cas",
    "project_delete_takes_everything_and_says_so",
    "repo_round_trip_and_primary_flag",
    "item_kind_round_trip_and_prefix_rules",
    "item_kind_delete_refused_while_referenced",
    "step_graph_and_phase_round_trip",
    "settings_app_rung_validates_and_cas",
    "settings_project_rung_merges_keys",
    "settings_phase_rung_writes_token_budget_only",
];

// in run_case's match, one arm per name, same shape as the existing arms:
        "workspace_round_trip_and_cas" => workspace_round_trip_and_cas(store).await,
        // ...
        "settings_phase_rung_writes_token_budget_only" => {
            settings_phase_rung_writes_token_budget_only(store).await;
        }
```

Helpers (private, beside `new_item` / `title_patch`):

```rust
fn applied<T: core::fmt::Debug>(case: &str, outcome: CasOutcome<T>) -> T {
    match outcome {
        CasOutcome::Applied(row) => row,
        CasOutcome::Stale(row) => panic!("{case}: expected Applied, got Stale({row:?})"),
    }
}
fn stale<T: core::fmt::Debug>(case: &str, outcome: CasOutcome<T>) -> T { /* mirror */ }
fn new_workspace(slug: &str) -> NewWorkspace { /* id fresh, created_by: ids::USER */ }
fn new_project(slug: &str) -> NewProject { /* same */ }
```

Case shape (case 1 in full as the template; the case name prefixes every assertion message,
`mem.rs::` twins named in the doc only where they exist — flag E):

```rust
/// D3 on the smallest table: a create, a slug collision, a read-back, one `Applied` and one
/// `Stale` carrying the current row. `updated_at` advanced by the store, never by the case.
async fn workspace_round_trip_and_cas<S: WriteStore>(store: &S) {
    const CASE: &str = "workspace_round_trip_and_cas";
    let created = store.create_workspace(new_workspace("ops")).await.expect(CASE);
    assert!(
        matches!(store.create_workspace(new_workspace("ops")).await, Err(StoreError::Constraint(_))),
        "{CASE}: duplicate slug must be Constraint"
    );
    assert_eq!(store.workspace(created.id).await.expect(CASE).as_ref(), Some(&created), "{CASE}: read-back");
    let patch = WorkspacePatch { name: Some("Operations".into()), ..WorkspacePatch::default() };
    let edited = applied(CASE, store.update_workspace(created.id, created.updated_at, patch.clone()).await.expect(CASE));
    assert!(edited.updated_at > created.updated_at, "{CASE}: the trigger advances the token");
    let now = stale(CASE, store.update_workspace(created.id, created.updated_at, patch).await.expect(CASE));
    assert_eq!(now, edited, "{CASE}: Stale carries the current row");
    assert!(
        matches!(store.update_workspace(WorkspaceId::new_v4(), edited.updated_at, WorkspacePatch::default()).await, Err(StoreError::NotFound { entity: "workspace", .. })),
        "{CASE}: unknown id is NotFound"
    );
}
```

Per-case notes beyond the plan's text: case 3 deletes a fresh workspace linked to `PROJECT_VULKAN`
(not `WORKSPACE_PLATFORM`, which case 5 relies on) and checks `project(PROJECT_VULKAN)` is still
`Some`. Case 5 builds `expected: DeleteReach` from `delete_reach(Project(PROJECT_HTUI))`, asserts
`delete_project` returns the identical struct, then the field expectations of §6, then `items(AGY)`
unchanged and no link on `AGY_FEAT_1` points at an `HTUI_*` item. Case 6 asserts the demoted repo's
`updated_at` advanced. Case 7 mints under `KIND_HTUI_ANA` after the rename and asserts key `ANL-1`.
Case 8 asserts the `Constraint` text contains `"held by 2 items"` (HTUI has 2 ANA items). Case 9's
duplicate-position and duplicate-name phases target `GRAPH_HTUI_FEAT` (positions 0–3 taken; use a
fresh graph for the happy path). Case 10 reads `setting(App, TokenBudget)` first, uses the token if
`Some`, sends `json!(0.5001)` for `PromptReserveFraction` (flag B), and sends
`json!(i64::from(u32::MAX) + 1)` for `ExcerptMaxFiles` as a range refusal that is not a
clamp-to-zero on either store. Case 11 snapshots `project(PROJECT_HTUI).settings` as a `Map`, writes
`UpstreamHops = 2`, asserts every key other than `upstream_hops` equal by `Value` (flag K), then
`clear` leaves the map equal to the snapshot. Case 12 sets `TokenBudget = 90_000`, reads it in
`phases(GRAPH_HTUI_FEAT)` for `PHASE_HTUI_IMPLEMENT`, clears, asserts `token_budget == None` and
`setting(Phase(..))` is `Some(StoredSetting { value: None, .. })`; also
`set_setting(Phase, UpstreamHops, ..)` → `Constraint` and `expected: None` on Phase → `Constraint`.

`every_cross_referenced_test_name_exists` and `run_case_accepts_every_name_in_cases` stay untouched
and must be green at the end of step 6.
