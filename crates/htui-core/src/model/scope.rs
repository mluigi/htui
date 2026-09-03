//! The workspace scope every read is issued against (plan D10, `R-ENT-2`).

use serde::{Deserialize, Serialize};

use crate::model::hierarchy::WorkspaceSummary;
use crate::model::ids::{ProjectId, WorkspaceId};

/// The set of projects a query may see: always one workspace, never a bare project list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// The workspace the TUI is inside.
    pub workspace_id: WorkspaceId,
    /// Its projects, ordered by `workspace_project.position`.
    pub project_ids: Vec<ProjectId>,
}

impl Scope {
    /// Builds the scope of a workspace, ordering the projects by
    /// `workspace_project.position` (ties keep the summary's own order).
    #[must_use]
    pub fn from_workspace(ws: &WorkspaceSummary) -> Self {
        let mut projects: Vec<_> = ws.projects.iter().collect();
        projects.sort_by_key(|p| p.position);
        Self {
            workspace_id: ws.workspace_id,
            project_ids: projects.iter().map(|p| p.project_id).collect(),
        }
    }

    /// Whether the project is inside this scope.
    #[must_use]
    pub fn contains(&self, p: ProjectId) -> bool {
        self.project_ids.contains(&p)
    }

    /// Whether the scope holds no project at all: an empty workspace reads as an empty backlog,
    /// not as "every project".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.project_ids.is_empty()
    }
}
