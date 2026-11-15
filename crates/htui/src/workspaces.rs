//! The Workspaces tab's overview (MOD-116 M1 D7, `R-TUI-14`): one worker-assembled read per
//! event, never per frame, as the Settings hierarchy's snapshot is.
//!
//! [`serve`] answers [`StoreRequest::WorkspaceOverview`] through the writer: the workspace's
//! tree ([`hierarchy::snapshot`]), this box's folder ([`hierarchy::workspace_root`]), the latest
//! `goals` owner document of its scope row, its run counts (the tally of its unfinished runs and
//! its stop row, as [`WorkspaceRunCounts`]), the default target its scope row names or inherits,
//! and per project the items by state, the running runs and the rows waiting on a person (the
//! rows the header's `⚠ n waiting` counts). Offline there is no writer and the read is refused
//! with `DATABASE_UNREACHABLE`: `workspace_stop` is not mirrored.

use std::path::{Path, PathBuf};

use htui_core::model::document::GOALS_KIND;
use htui_core::model::{
    ItemFilter, ItemState, ProjectId, RunStatus, Scope, WorkspaceId, WorkspaceRunCounts,
};
use htui_core::seed;
use htui_core::store::{ReadStore, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::hierarchy::{self, HierarchySnapshot};
use crate::store_worker::{StoreReply, StoreRequest, serve_waiting};

/// One workspace as the Workspaces tab's detail pane shows it (D7).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceOverview {
    /// The workspace row, this box's folder row, its projects with their repos, and its scope row.
    pub tree: HierarchySnapshot,
    /// This box's folder for the workspace ([`hierarchy::workspace_root`]); `None` with no row
    /// and no home.
    pub folder: Option<PathBuf>,
    /// The body of the highest-version `goals` owner document of the scope row; `None` without a
    /// scope row or a goals document.
    pub goals: Option<String>,
    /// The workspace's unfinished runs by status, and its stop row.
    pub counts: WorkspaceRunCounts,
    /// The target the workspace runs on by default.
    pub default_target: TargetLine,
    /// One per project, in the tree's order (`workspace_project.position`).
    pub projects: Vec<ProjectOverview>,
}

/// Where a workspace's runs go by default (D7): the target its scope row names, or the Settings
/// default it inherits (no `app_setting` row is the seeded local target, `this box`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetLine {
    /// The scope row names this target.
    Set(String),
    /// The scope row names none; the Settings default is this target.
    Inherits(String),
}

impl core::fmt::Display for TargetLine {
    /// `<name>`, or `inherits the Settings default (<name>)`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Set(name) => f.write_str(name),
            Self::Inherits(name) => write!(f, "inherits the Settings default ({name})"),
        }
    }
}

/// One project's counts in a [`WorkspaceOverview`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectOverview {
    /// The project.
    pub project: ProjectId,
    /// Its items by state, in [`ItemState::ALL`] order, non-zero only.
    pub items: Vec<(ItemState, usize)>,
    /// Its `running` runs.
    pub running: u32,
    /// The rows waiting on a person that belong to it.
    pub waiting: usize,
}

impl ProjectOverview {
    /// Every item the project holds, whatever its state.
    #[must_use]
    pub fn item_total(&self) -> usize {
        self.items.iter().map(|(_, count)| count).sum()
    }
}

/// Serves [`StoreRequest::WorkspaceOverview`] (D7) with this user's home directory.
///
/// # Errors
/// [`StoreError::Unreachable`] offline (`DATABASE_UNREACHABLE`), whatever the store reports, and
/// [`StoreError::Backend`] for a request that is not this module's: `try_serve` routes only the
/// overview here, so a caller that reached it anyway is told which request it sent.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    serve_with_home(backend, request, dirs::home_dir().as_deref()).await
}

/// [`serve`] with the home directory a test chooses, so no test reads the real
/// `~/workspaces`: the default folder of a workspace with no row is `<home>/workspaces/<slug>`.
///
/// # Errors
/// As [`serve`].
pub async fn serve_with_home(
    backend: &Backend,
    request: &StoreRequest,
    home: Option<&Path>,
) -> Result<StoreReply> {
    let StoreRequest::WorkspaceOverview(workspace) = request else {
        return Err(StoreError::Backend(format!(
            "`{}` is not a workspaces request",
            request.name()
        )));
    };
    let workspace = *workspace;
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let overview = overview(backend, &writer, workspace, home).await?;
    Ok(StoreReply::WorkspaceOverview {
        workspace,
        overview: overview.map(Box::new),
    })
}

/// The N+1 reads of one overview; `None` when the workspace does not exist.
async fn overview(
    backend: &Backend,
    writer: &Writer,
    workspace: WorkspaceId,
    home: Option<&Path>,
) -> Result<Option<WorkspaceOverview>> {
    let this_box = backend.box_info().await?.map(|info| info.box_id);
    let Some(tree) = hierarchy::snapshot(writer, workspace, this_box).await? else {
        return Ok(None);
    };
    let folder = hierarchy::workspace_root(&tree, home);
    let goals = match &tree.scope_row {
        Some(row) => latest_goals(writer, row.id).await?,
        None => None,
    };

    let runs = writer.workspace_live_runs(workspace).await?;
    let stop = writer.workspace_stop(workspace).await?;
    let counts = WorkspaceRunCounts::tally(workspace, runs.iter().map(|run| run.status), stop);

    let default_target = default_target(writer, &tree).await?;

    let scope = Scope {
        workspace_id: workspace,
        project_ids: tree.projects.iter().map(|entry| entry.project.id).collect(),
    };
    let waiting = if scope.is_empty() {
        Vec::new()
    } else {
        match serve_waiting(backend, &scope, async |writer, scope| {
            WriteStore::open_permissions(writer, scope).await
        })
        .await?
        {
            StoreReply::Waiting { view, .. } => view.rows,
            _ => Vec::new(),
        }
    };
    let items = if scope.is_empty() {
        Vec::new()
    } else {
        ReadStore::items(writer, &scope, &ItemFilter::default()).await?
    };

    let projects = scope
        .project_ids
        .iter()
        .map(|&project| ProjectOverview {
            project,
            items: ItemState::ALL
                .iter()
                .map(|&state| {
                    let count = items
                        .iter()
                        .filter(|item| item.project_id == project && item.state() == state)
                        .count();
                    (state, count)
                })
                .filter(|&(_, count)| count > 0)
                .collect(),
            running: u32::try_from(
                runs.iter()
                    .filter(|run| run.project_id == project && run.status == RunStatus::Running)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            waiting: waiting.iter().filter(|row| row.project == project).count(),
        })
        .collect();

    Ok(Some(WorkspaceOverview {
        tree,
        folder,
        goals,
        counts,
        default_target,
        projects,
    }))
}

/// The body of `owner`'s highest-version `goals` document, if it has one.
async fn latest_goals(writer: &Writer, owner: ProjectId) -> Result<Option<String>> {
    Ok(ReadStore::owner_documents(writer, owner)
        .await?
        .into_iter()
        .filter(|document| document.kind == GOALS_KIND)
        .max_by_key(|document| document.version)
        .map(|document| document.body))
}

/// The scope row's target by name, else the Settings default it inherits (D7). MOD-130's
/// resolver is not on the tree: this names the workspace level only.
async fn default_target(writer: &Writer, tree: &HierarchySnapshot) -> Result<TargetLine> {
    if let Some(target) = tree.scope_row.as_ref().and_then(|row| row.target_id) {
        return Ok(TargetLine::Set(
            ReadStore::target(writer, target).await?.name,
        ));
    }
    let name = match WriteStore::default_target(writer).await? {
        Some(default) => ReadStore::target(writer, default.target).await?.name,
        None => seed::LOCAL_TARGET_NAME.to_owned(),
    };
    Ok(TargetLine::Inherits(name))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{DocumentId, ItemState, NewOwnerDocument, Scope, WorkspaceSummary};
    use htui_core::seed;
    use htui_core::store::{CasOutcome, MemStore, ReadStore as _, StoreError, WriteStore as _};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    use super::{ProjectOverview, TargetLine, WorkspaceOverview, serve, serve_with_home};
    use crate::store_worker::{StoreReply, StoreRequest};

    /// The overview of `workspace` over `backend`, with `home` as the home directory.
    async fn overview_of(
        backend: &Backend,
        workspace: htui_core::model::WorkspaceId,
        home: &std::path::Path,
    ) -> Option<Box<WorkspaceOverview>> {
        let reply = serve_with_home(
            backend,
            &StoreRequest::WorkspaceOverview(workspace),
            Some(home),
        )
        .await
        .expect("the overview is served");
        let StoreReply::WorkspaceOverview {
            workspace: asked,
            overview,
        } = reply
        else {
            panic!("expected an overview, got {reply:?}");
        };
        assert_eq!(asked, workspace);
        overview
    }

    /// The demo's `workspace` summary.
    async fn summary(
        backend: &Backend,
        workspace: htui_core::model::WorkspaceId,
    ) -> WorkspaceSummary {
        backend
            .workspaces()
            .await
            .expect("a memory read")
            .into_iter()
            .find(|summary| summary.workspace_id == workspace)
            .expect("the demo holds it")
    }

    /// `project`'s line of `overview`.
    fn project(
        overview: &WorkspaceOverview,
        project: htui_core::model::ProjectId,
    ) -> &ProjectOverview {
        overview
            .projects
            .iter()
            .find(|line| line.project == project)
            .unwrap_or_else(|| panic!("a line for {project}: {:?}", overview.projects))
    }

    /// D7 over the demo's Platform: `htui` and `agy` in position order; `RUN_2` queued, `RUN_4`
    /// running and `RUN_5` parked, all on `htui`; no stop row; no folder row, so the folder is
    /// `<home>/workspaces/platform`; no goals; no target anywhere, so the seeded local target.
    #[tokio::test]
    async fn the_platform_overview_has_its_tree_counts_projects_and_target() {
        let backend = Backend::memory(MemStore::demo());
        let home = tempfile::tempdir().expect("a throwaway home");
        let overview = overview_of(&backend, ids::WORKSPACE_PLATFORM, home.path())
            .await
            .expect("Platform exists");

        assert_eq!(overview.tree.workspace.name, "Platform");
        assert_eq!(
            overview
                .projects
                .iter()
                .map(|line| line.project)
                .collect::<Vec<_>>(),
            [ids::PROJECT_HTUI, ids::PROJECT_AGY]
        );
        assert_eq!(
            overview.folder,
            Some(home.path().join("workspaces").join("platform"))
        );
        assert_eq!(overview.goals, None);
        let counts = &overview.counts;
        assert_eq!(
            (
                counts.queued,
                counts.running,
                counts.awaiting_approval,
                counts.stopped,
                counts.stop.clone()
            ),
            (1, 1, 1, 0, None)
        );
        assert_eq!(
            overview.default_target,
            TargetLine::Inherits(seed::LOCAL_TARGET_NAME.to_owned())
        );
        assert_eq!(
            overview.default_target.to_string(),
            "inherits the Settings default (this box)"
        );

        let htui = project(&overview, ids::PROJECT_HTUI);
        let agy = project(&overview, ids::PROJECT_AGY);
        assert_eq!((htui.running, agy.running), (1, 0));

        // The header's rows, grouped by project: the overview counts the same ones.
        let scope = Scope::from_workspace(&summary(&backend, ids::WORKSPACE_PLATFORM).await);
        let StoreReply::Waiting { view, .. } = crate::store_worker::serve(
            &backend,
            &StoreRequest::Waiting {
                scope: scope.clone(),
            },
        )
        .await
        else {
            panic!("the waiting list is served");
        };
        let waiting_on = |project| {
            view.rows
                .iter()
                .filter(|row| row.project == project)
                .count()
        };
        assert!(waiting_on(ids::PROJECT_HTUI) > 0, "{:?}", view.rows);
        assert_eq!(htui.waiting, waiting_on(ids::PROJECT_HTUI));
        assert_eq!(agy.waiting, waiting_on(ids::PROJECT_AGY));

        // Items by state, non-zero only, in `ItemState::ALL` order.
        let items = backend
            .items(&scope, &htui_core::model::ItemFilter::default())
            .await
            .expect("a memory read");
        for line in [htui, agy] {
            let expected: Vec<(ItemState, usize)> = ItemState::ALL
                .iter()
                .map(|&state| {
                    let count = items
                        .iter()
                        .filter(|item| item.project_id == line.project && item.state() == state)
                        .count();
                    (state, count)
                })
                .filter(|&(_, count)| count > 0)
                .collect();
            assert_eq!(line.items, expected, "{}", line.project);
            assert!(!line.items.is_empty(), "{}", line.project);
        }
        assert!(
            htui.items.contains(&(ItemState::Running, 1))
                && htui.items.contains(&(ItemState::NeedsYou, 1)),
            "{:?}",
            htui.items
        );
    }

    /// D7: the goals are the highest version's body, whatever order the store lists them in.
    #[tokio::test]
    async fn the_latest_goals_version_is_the_overview_s_goals() {
        let backend = Backend::memory(MemStore::demo());
        let writer = backend.writer().expect("a memory backend writes");
        for body in ["first goals", "second goals"] {
            writer
                .write_owner_document(NewOwnerDocument {
                    id: DocumentId::new(),
                    project_id: ids::PROJECT_SCOPE_PLATFORM,
                    kind: htui_core::model::document::GOALS_KIND.to_owned(),
                    title: "Goals".to_owned(),
                    body: body.to_owned(),
                    created_by: ids::USER,
                    created_at: Utc::now(),
                })
                .await
                .expect("a goals document is written");
        }
        let home = tempfile::tempdir().expect("a throwaway home");
        let overview = overview_of(&backend, ids::WORKSPACE_PLATFORM, home.path())
            .await
            .expect("Platform exists");
        assert_eq!(overview.goals.as_deref(), Some("second goals"));
    }

    /// D7: a target on the scope row is named, not inherited.
    #[tokio::test]
    async fn a_scope_row_target_is_named() {
        let backend = Backend::memory(MemStore::demo());
        let writer = backend.writer().expect("a memory backend writes");
        let row = writer
            .project(ids::PROJECT_SCOPE_PLATFORM)
            .await
            .expect("a memory read")
            .expect("Platform's scope row");
        let outcome = writer
            .set_project_target(row.id, row.updated_at, Some(seed::LOCAL_TARGET_ID))
            .await
            .expect("the target is set");
        assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
        let home = tempfile::tempdir().expect("a throwaway home");
        let overview = overview_of(&backend, ids::WORKSPACE_PLATFORM, home.path())
            .await
            .expect("Platform exists");
        assert_eq!(
            overview.default_target,
            TargetLine::Set(seed::LOCAL_TARGET_NAME.to_owned())
        );
        assert_eq!(overview.default_target.to_string(), "this box");
    }

    #[tokio::test]
    async fn an_unknown_workspace_has_no_overview() {
        let backend = Backend::memory(MemStore::demo());
        let home = tempfile::tempdir().expect("a throwaway home");
        assert_eq!(
            overview_of(&backend, htui_core::model::WorkspaceId::new(), home.path()).await,
            None
        );
    }

    /// D7: offline there is no writer, so the overview is refused before any read, named
    /// `workspace_overview`.
    #[tokio::test]
    async fn offline_the_overview_is_refused() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "overview-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        let request = StoreRequest::WorkspaceOverview(ids::WORKSPACE_PLATFORM);
        assert!(matches!(
            serve(&backend, &request).await,
            Err(StoreError::Unreachable(message)) if message == DATABASE_UNREACHABLE
        ));
        let reply = crate::store_worker::serve(&backend, &request).await;
        assert!(
            matches!(
                &reply,
                StoreReply::Failed {
                    request: "workspace_overview",
                    message,
                } if message.ends_with(DATABASE_UNREACHABLE)
            ),
            "{reply:?}"
        );
        cache.close().await;
    }

    /// A request that is not this module's is refused by name, not served.
    #[tokio::test]
    async fn another_request_is_refused_by_name() {
        let backend = Backend::memory(MemStore::demo());
        let reply = serve(&backend, &StoreRequest::Workspaces).await;
        assert!(
            matches!(&reply, Err(StoreError::Backend(message)) if message.contains("`workspaces`")),
            "{reply:?}"
        );
    }
}
