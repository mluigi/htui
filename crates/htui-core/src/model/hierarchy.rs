//! Workspaces, projects, repositories and their per-box paths (`docs/ANA-9.md` §5.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{BoxId, ProjectId, RepoId, UserId, WorkspaceId};

/// A row of `workspace` (§5.3): the scope the TUI is always inside (plan D10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// `workspace.id`.
    pub id: WorkspaceId,
    /// `workspace.slug`, unique.
    pub slug: String,
    /// `workspace.name`.
    pub name: String,
    /// `workspace.description`.
    pub description: String,
    /// `workspace.created_by`.
    pub created_by: UserId,
    /// `workspace.created_at`.
    pub created_at: DateTime<Utc>,
    /// `workspace.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::create_workspace`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewWorkspace {
    /// `workspace.id`, minted client-side as a UUIDv7.
    pub id: WorkspaceId,
    /// `workspace.slug`, unique across the database.
    pub slug: String,
    /// `workspace.name`.
    pub name: String,
    /// `workspace.description`.
    pub description: String,
    /// `workspace.created_by`.
    pub created_by: UserId,
}

/// Edit passed to [`crate::store::WriteStore::update_workspace`]; `None` leaves the column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePatch {
    /// `workspace.slug`.
    pub slug: Option<String>,
    /// `workspace.name`.
    pub name: Option<String>,
    /// `workspace.description`.
    pub description: Option<String>,
}

/// A row of `project` (§5.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// `project.id`.
    pub id: ProjectId,
    /// `project.slug`, unique.
    pub slug: String,
    /// `project.name`.
    pub name: String,
    /// `project.description`.
    pub description: String,
    /// `project.secret_provider`, e.g. `infisical`.
    pub secret_provider: Option<String>,
    /// `project.secret_scope`: provider-specific project/environment reference.
    pub secret_scope: Option<String>,
    /// `project.settings` (`JSONB`): token budget, retention, cached transcript steps, ...
    pub settings: Value,
    /// `project.created_by`.
    pub created_by: UserId,
    /// `project.created_at`.
    pub created_at: DateTime<Utc>,
    /// `project.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::create_project`].
///
/// No `settings` and no secret columns (plan D9): the project lands with `settings = {}` and the
/// key-level writer [`crate::store::WriteStore::set_setting`] fills it one key at a time, because a
/// whole-document write would erase MOD-4's and MOD-12's keys without a word (PRD risk table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewProject {
    /// `project.id`, minted client-side as a UUIDv7.
    pub id: ProjectId,
    /// `project.slug`, unique across the database.
    pub slug: String,
    /// `project.name`.
    pub name: String,
    /// `project.description`.
    pub description: String,
    /// `project.created_by`.
    pub created_by: UserId,
}

/// Edit passed to [`crate::store::WriteStore::update_project`]; `None` leaves the column.
///
/// `settings` is deliberately absent, for the reason [`NewProject`] gives.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectPatch {
    /// `project.slug`.
    pub slug: Option<String>,
    /// `project.name`.
    pub name: Option<String>,
    /// `project.description`.
    pub description: Option<String>,
}

/// A row of `workspace_project` (§5.3): a project's membership and order inside a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceProject {
    /// `workspace_project.workspace_id`.
    pub workspace_id: WorkspaceId,
    /// `workspace_project.project_id`.
    pub project_id: ProjectId,
    /// `workspace_project.position`.
    pub position: i32,
}

/// A row of `repo` (§5.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    /// `repo.id`.
    pub id: RepoId,
    /// `repo.project_id`.
    pub project_id: ProjectId,
    /// `repo.name`, unique within the project.
    pub name: String,
    /// `repo.remote_url`.
    pub remote_url: Option<String>,
    /// `repo.default_branch`.
    pub default_branch: String,
    /// `repo.is_primary`: at most one primary repository per project.
    pub is_primary: bool,
    /// `repo.created_at`.
    pub created_at: DateTime<Utc>,
    /// `repo.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::create_repo`].
///
/// `is_primary: true` is honoured rather than refused: the store clears the project's current
/// primary in the same transaction, so `uq_repo_primary` (`0001_init.sql:185-200`) is never tripped
/// by a create (plan D10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRepo {
    /// `repo.id`, minted client-side as a UUIDv7.
    pub id: RepoId,
    /// `repo.project_id`.
    pub project_id: ProjectId,
    /// `repo.name`, unique within the project.
    pub name: String,
    /// `repo.remote_url`.
    pub remote_url: Option<String>,
    /// `repo.default_branch`.
    pub default_branch: String,
    /// `repo.is_primary`.
    pub is_primary: bool,
}

/// Edit passed to [`crate::store::WriteStore::update_repo`]; `None` leaves the column.
///
/// `remote_url` is doubly wrapped as [`crate::model::ItemPatch::step_graph_id`] is: `None` leaves
/// the stored URL, `Some(None)` clears it, `Some(Some(url))` replaces it. A single `Option` could
/// not express "clear" at all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoPatch {
    /// `repo.name`.
    pub name: Option<String>,
    /// `repo.remote_url`; `Some(None)` clears it.
    pub remote_url: Option<Option<String>>,
    /// `repo.default_branch`.
    pub default_branch: Option<String>,
    /// `repo.is_primary`; `Some(true)` demotes the project's current primary in the same
    /// transaction, `Some(false)` only unsets this row.
    pub is_primary: Option<bool>,
}

/// A row of `repo_box_path` (§5.3): where a repository lives on one box (`R-BOX-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoBoxPath {
    /// `repo_box_path.repo_id`.
    pub repo_id: RepoId,
    /// `repo_box_path.box_id`.
    pub box_id: BoxId,
    /// `repo_box_path.local_path`.
    pub local_path: String,
    /// `repo_box_path.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `workspace_box_path` (§5.3): where a workspace is rooted on one box (`R-BOX-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceBoxPath {
    /// `workspace_box_path.workspace_id`.
    pub workspace_id: WorkspaceId,
    /// `workspace_box_path.box_id`.
    pub box_id: BoxId,
    /// `workspace_box_path.root_path`.
    pub root_path: String,
    /// `workspace_box_path.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Join of `workspace_project` and `project`, ordered by position. Not a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectRef {
    /// `project.id`.
    pub project_id: ProjectId,
    /// `project.slug`.
    pub slug: String,
    /// `project.name`.
    pub name: String,
    /// `workspace_project.position`.
    pub position: i32,
}

/// Switcher row: a workspace with its projects. `projects` is ordered by position and is what a
/// [`crate::model::Scope`] is built from (plan D10). Not a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    /// `workspace.id`.
    pub workspace_id: WorkspaceId,
    /// `workspace.slug`.
    pub slug: String,
    /// `workspace.name`.
    pub name: String,
    /// The workspace's projects, ordered by `workspace_project.position`.
    pub projects: Vec<ProjectRef>,
}
