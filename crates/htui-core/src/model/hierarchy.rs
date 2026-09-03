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
