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

/// The bound of the prompt's upstream walk (plan D95): the active workspace, or one project when
/// there is none (`R-PRM-1`'s "or current project when no workspace").
///
/// Not [`Scope`], and deliberately so. `Scope` is "always one workspace, never a bare project
/// list" and `from_workspace` is its only constructor, while `R-ENT-2` says there are no implicit
/// workspace rows — so the no-workspace case cannot be expressed there without inventing a
/// `WorkspaceId`. Twenty-plus sites across five crates construct or consume a `Scope` and every
/// one of them means "the active workspace"; widening that to serve one query is how a shipped
/// invariant erodes. The amended §7.3 walk takes `$workspace` and `$project` separately
/// (`docs/ANA-5.md` `:655-657`), so a type of its own costs nothing at the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptScope {
    /// The active workspace, or `None` when the walk is bounded by `project` alone.
    pub workspace: Option<WorkspaceId>,
    /// The project of the item being prompted for. Read by the scope CTE's `UNION SELECT
    /// $project WHERE $workspace IS NULL`, so it matters only in the no-workspace case.
    pub project: ProjectId,
}

impl PromptScope {
    /// The walk bounded by an active workspace, for an item in `project`.
    ///
    /// `project` need not be one of the scope's own projects: it is the prompted item's project,
    /// and the workspace is what decides which upstream items are in scope.
    #[must_use]
    pub const fn from_scope(scope: &Scope, project: ProjectId) -> Self {
        Self {
            workspace: Some(scope.workspace_id),
            project,
        }
    }

    /// The walk bounded by one project, no workspace active (`R-ENT-2`).
    #[must_use]
    pub const fn project_only(project: ProjectId) -> Self {
        Self {
            workspace: None,
            project,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::hierarchy::ProjectRef;
    use uuid::Uuid;

    fn workspace() -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(1)),
            slug: "core".to_owned(),
            name: "Core".to_owned(),
            projects: vec![ProjectRef {
                project_id: ProjectId::from_uuid(Uuid::from_u128(2)),
                slug: "htui".to_owned(),
                name: "htui".to_owned(),
                position: 0,
            }],
        }
    }

    /// The workspace half of the `$workspace`/`$project` pair the amended §7.3 walk takes: a
    /// workspace is active, so the scope CTE reads `workspace_project`.
    #[test]
    fn prompt_scope_from_scope_keeps_the_workspace() {
        let ws = workspace();
        let scope = Scope::from_workspace(&ws);
        let project = ProjectId::from_uuid(Uuid::from_u128(3));

        let prompt = PromptScope::from_scope(&scope, project);

        assert_eq!(prompt.workspace, Some(ws.workspace_id));
        assert_eq!(
            prompt.project, project,
            "the item's own project, which need not be in the workspace"
        );
    }

    /// `R-ENT-2`: there are no implicit workspace rows, so the no-workspace case carries `None`
    /// rather than a minted `WorkspaceId` — which is the whole of D95's argument for a type of its
    /// own rather than a widened [`Scope`].
    #[test]
    fn project_only_has_none() {
        let project = ProjectId::from_uuid(Uuid::from_u128(4));
        let prompt = PromptScope::project_only(project);

        assert_eq!(prompt.workspace, None);
        assert_eq!(prompt.project, project);
    }
}
