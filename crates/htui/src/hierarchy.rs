//! The hierarchy the Settings tab edits: one worker-assembled snapshot per read, twelve served
//! writes, identity filled here and never on the render side (MOD-15 milestone 3, D5/D6/D10).
//!
//! A section names a read and is handed rows (`R-NF-3`): nothing below this module is reachable
//! from `ui/`, and the two identity columns a write needs — `workspace.created_by` and the box a
//! path belongs to — are resolved from [`Backend`] here, so no view ever holds a `UserId` or a
//! `BoxId`.

use std::path::Path;

use chrono::Utc;
use htui_core::model::{
    BoxId, NewProject, NewRepo, NewWorkspace, Project, ProjectId, ProjectRef, Repo, RepoBoxPath,
    RepoId, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspaceProject, WorkspaceSummary,
};
use htui_core::root_path::canonical_root;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, Result, StoreError, WriteStore,
};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// A workspace, its projects in position order, their repos by name, and this box's paths.
///
/// One read answers the whole tree (D5): the section re-renders from this and never patches its
/// rows from a single returned row, so there is exactly one source of truth on the render side.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchySnapshot {
    /// The workspace row itself; `updated_at` is the CAS token an editor opens on.
    pub workspace: Workspace,
    /// This box, when one is registered — the id a path write is stored under.
    pub this_box: Option<BoxId>,
    /// This box's root path for the workspace, when a row exists. Other boxes' rows are dropped.
    pub root_path: Option<WorkspaceBoxPath>,
    /// The workspace's projects, ordered by `workspace_project.position`.
    pub projects: Vec<ProjectEntry>,
}

/// One project inside a workspace: its link row, the project itself and its repos.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEntry {
    /// The `workspace_project` row, which carries the position the tree is ordered by.
    pub link: WorkspaceProject,
    /// The project row; `updated_at` is its CAS token.
    pub project: Project,
    /// The project's repos, ordered by name.
    pub repos: Vec<RepoEntry>,
}

/// One repo and where it is checked out on this box.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    /// The repo row; `updated_at` is its CAS token.
    pub repo: Repo,
    /// This box's checkout path, when a row exists.
    pub local_path: Option<RepoBoxPath>,
}

/// What the worker did to the mirror after a delete (D10).
///
/// Never a reason to turn the reply into a
/// [`StoreReply::Failed`](crate::store_worker::StoreReply::Failed): the rows are already gone, and
/// a delete that happened must be reported as done whatever the mirror did afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorAfterDelete {
    /// `CacheStore::rebuild()` answered `Ok`.
    Rebuilt,
    /// There is no mirror on this backend (`Backend::Memory`).
    NoMirror,
    /// A workspace delete: `workspace` and `workspace_project` are full-table replaced on every
    /// refresh pass, so nothing stale can survive one.
    NotNeeded,
    /// The rebuild failed; the delete still happened.
    Failed(String),
}

impl HierarchySnapshot {
    /// The switcher row for this tree, so a section can emit `Action::SetScope` (D11).
    #[must_use]
    pub fn summary(&self) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: self.workspace.id,
            slug: self.workspace.slug.clone(),
            name: self.workspace.name.clone(),
            projects: self
                .projects
                .iter()
                .map(|entry| ProjectRef {
                    project_id: entry.project.id,
                    slug: entry.project.slug.clone(),
                    name: entry.project.name.clone(),
                    position: entry.link.position,
                })
                .collect(),
        }
    }
}

/// One read of the whole tree; `None` when the workspace does not exist.
///
/// N+1 reads on purpose (D5): they happen per event — activation, a scope change, after a write —
/// never per keystroke, and a joined reader would be a seam method six implementors would owe for
/// a read nothing else wants. A link whose project is gone is **skipped** rather than failing the
/// read: on Postgres the link cascades with the project, and `MemStore` keeps the two in step, so
/// a widowed link is a torn read rather than a state to render.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot<S: ReadStore + WriteStore + ?Sized>(
    store: &S,
    ws: WorkspaceId,
    this_box: Option<BoxId>,
) -> Result<Option<HierarchySnapshot>> {
    let Some(workspace) = store.workspace(ws).await? else {
        return Ok(None);
    };
    let root_path = store
        .workspace_box_paths(ws)
        .await?
        .into_iter()
        .find(|row| Some(row.box_id) == this_box);

    let mut projects = Vec::new();
    for link in store.workspace_projects(ws).await? {
        let Some(project) = store.project(link.project_id).await? else {
            continue;
        };
        let mut repos = Vec::new();
        for repo in store.repos(project.id).await? {
            let local_path = store
                .repo_box_paths(repo.id)
                .await?
                .into_iter()
                .find(|row| Some(row.box_id) == this_box);
            repos.push(RepoEntry { repo, local_path });
        }
        projects.push(ProjectEntry {
            link,
            project,
            repos,
        });
    }

    Ok(Some(HierarchySnapshot {
        workspace,
        this_box,
        root_path,
        projects,
    }))
}

/// Serves one hierarchy request, off the UI task and with identity filled in here (D6).
///
/// `Err(StoreError::Unreachable)` on [`Backend::Offline`], whose
/// [`writer`](Backend::writer) is `None`, so the store worker's caller keeps the offline
/// transition it already has for every other request.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline,
/// [`StoreError::NotFound`] when this box has no row and a path is being written, and
/// [`StoreError::Constraint`] carrying a [`RootRefusal`](htui_core::root_path::RootRefusal).
///
/// The last arm answers [`StoreError::Backend`] rather than panicking: `try_serve` routes exactly
/// the twelve variants below here, so it is unreachable from the shell, and a caller that reached
/// it anyway is better told which request it sent than killed.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let this_box = backend.box_info().await?.map(|info| info.box_id);

    match request {
        StoreRequest::Hierarchy(ws) => Ok(StoreReply::Hierarchy(
            snapshot(&writer, *ws, this_box).await?.map(Box::new),
        )),
        StoreRequest::CreateWorkspace {
            slug,
            name,
            description,
        } => {
            let created_by = backend.this_user().await?;
            let workspace = writer
                .create_workspace(NewWorkspace {
                    id: WorkspaceId::new(),
                    slug: slug.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    created_by,
                })
                .await?;
            reread(&writer, workspace.id, this_box).await
        }
        StoreRequest::UpdateWorkspace {
            id,
            expected,
            patch,
        } => {
            let outcome = writer
                .update_workspace(*id, *expected, patch.clone())
                .await?;
            cas(&writer, *id, this_box, &outcome).await
        }
        StoreRequest::SetWorkspaceRoot { id, path } => {
            let root_path = canonical(path).await?;
            writer
                .upsert_workspace_box_path(&WorkspaceBoxPath {
                    workspace_id: *id,
                    box_id: box_id(this_box)?,
                    root_path,
                    // The store's trigger stamps the column; nothing here sets `updated_at` by
                    // hand, and the value passed in is overwritten on both backends.
                    updated_at: Utc::now(),
                })
                .await?;
            reread(&writer, *id, this_box).await
        }
        StoreRequest::CreateProject {
            workspace,
            slug,
            name,
            description,
        } => {
            let created_by = backend.this_user().await?;
            let project = writer
                .create_project(NewProject {
                    id: ProjectId::new(),
                    slug: slug.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    created_by,
                })
                .await?;
            // Two seam calls rather than one transaction: `upsert_workspace_project` is the seam's
            // linker and this milestone adds no seam method, so a failure between them leaves an
            // unlinked project the editor's `Failed` reports and `r` does not show.
            let links = writer.workspace_projects(*workspace).await?;
            writer
                .upsert_workspace_project(&WorkspaceProject {
                    workspace_id: *workspace,
                    project_id: project.id,
                    position: i32::try_from(links.len()).unwrap_or(i32::MAX),
                })
                .await?;
            reread(&writer, *workspace, this_box).await
        }
        StoreRequest::UpdateProject {
            id,
            expected,
            patch,
        } => {
            let outcome = writer.update_project(*id, *expected, patch.clone()).await?;
            let ws = workspace_of(backend, *id).await?;
            cas(&writer, ws, this_box, &outcome).await
        }
        StoreRequest::CreateRepo {
            project,
            name,
            remote_url,
            default_branch,
            is_primary,
        } => {
            writer
                .create_repo(NewRepo {
                    id: RepoId::new(),
                    project_id: *project,
                    name: name.clone(),
                    remote_url: remote_url.clone(),
                    default_branch: default_branch.clone(),
                    is_primary: *is_primary,
                })
                .await?;
            let ws = workspace_of(backend, *project).await?;
            reread(&writer, ws, this_box).await
        }
        StoreRequest::UpdateRepo {
            project,
            id,
            expected,
            patch,
        } => {
            let outcome = writer.update_repo(*id, *expected, patch.clone()).await?;
            let ws = workspace_of(backend, *project).await?;
            cas(&writer, ws, this_box, &outcome).await
        }
        StoreRequest::SetRepoPath {
            project,
            repo,
            path,
        } => {
            let local_path = canonical(path).await?;
            writer
                .upsert_repo_box_path(&RepoBoxPath {
                    repo_id: *repo,
                    box_id: box_id(this_box)?,
                    local_path,
                    updated_at: Utc::now(),
                })
                .await?;
            let ws = workspace_of(backend, *project).await?;
            reread(&writer, ws, this_box).await
        }
        StoreRequest::DeleteReach(target) => {
            Ok(StoreReply::DeleteReach(writer.delete_reach(*target).await?))
        }
        StoreRequest::DeleteWorkspace(id) => Ok(StoreReply::Deleted {
            target: DeleteTarget::Workspace(*id),
            reach: writer.delete_workspace(*id).await?,
            // `workspace` and `workspace_project` are full-table replaced on every refresh pass,
            // so the mirror cannot serve a workspace this just removed (D10).
            mirror: MirrorAfterDelete::NotNeeded,
        }),
        StoreRequest::DeleteProject(id) => {
            let reach = writer.delete_project(*id).await?;
            // The rows are gone before this line, so a failed rebuild is reported *inside* a
            // successful reply rather than turning the delete into a `Failed` that claims nothing
            // happened.
            let mirror = match backend.cache() {
                Some(cache) => match cache.rebuild().await {
                    Ok(()) => MirrorAfterDelete::Rebuilt,
                    Err(err) => MirrorAfterDelete::Failed(err.to_string()),
                },
                None => MirrorAfterDelete::NoMirror,
            };
            Ok(StoreReply::Deleted {
                target: DeleteTarget::Project(*id),
                reach,
                mirror,
            })
        }
        other => Err(StoreError::Backend(format!(
            "not a hierarchy request: {}",
            other.name()
        ))),
    }
}

/// The tree as it is now, for a write that applied. A workspace that vanished between the write
/// and this read is a [`StoreError::NotFound`] rather than a `Hierarchy(None)`: the write just
/// succeeded against it.
async fn reread(writer: &Writer, ws: WorkspaceId, this_box: Option<BoxId>) -> Result<StoreReply> {
    let snapshot = snapshot(writer, ws, this_box)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "workspace",
            id: ws.to_string(),
        })?;
    Ok(StoreReply::Hierarchy(Some(Box::new(snapshot))))
}

/// A compare-and-set outcome as a reply: `Applied` answers the fresh tree, `Stale` answers the
/// same tree under [`StoreReply::HierarchyStale`] so the editor reloads and retries by hand (D7).
///
/// The worker re-reads rather than handing the section the single row `Stale` carries: the section
/// renders a tree, and a row patched in locally would be a second source of truth.
async fn cas<T>(
    writer: &Writer,
    ws: WorkspaceId,
    this_box: Option<BoxId>,
    outcome: &CasOutcome<T>,
) -> Result<StoreReply> {
    match outcome {
        CasOutcome::Applied(_) => reread(writer, ws, this_box).await,
        CasOutcome::Stale(_) => {
            let snapshot =
                snapshot(writer, ws, this_box)
                    .await?
                    .ok_or_else(|| StoreError::NotFound {
                        entity: "workspace",
                        id: ws.to_string(),
                    })?;
            Ok(StoreReply::HierarchyStale(Box::new(snapshot)))
        }
    }
}

/// This box's id, or the refusal a path write gets on a box with no row.
fn box_id(this_box: Option<BoxId>) -> Result<BoxId> {
    this_box.ok_or_else(|| StoreError::NotFound {
        entity: "box",
        id: "(this box)".to_owned(),
    })
}

/// The guard of D8, off the async task: `canonical_root` is `std::fs` and may block on a cold
/// mount, and this is the first `spawn_blocking` in `crates/htui/src`.
///
/// A refusal becomes a [`StoreError::Constraint`], so it reaches the status line as
/// ``set_repo_path: constraint violated: `/x` is a link to nothing``.
async fn canonical(path: &str) -> Result<String> {
    let typed = path.to_owned();
    tokio::task::spawn_blocking(move || canonical_root(Path::new(&typed)))
        .await
        .map_err(|err| StoreError::Backend(err.to_string()))?
        .map(|canonical| canonical.to_string_lossy().into_owned())
        .map_err(|refusal| StoreError::Constraint(refusal.to_string()))
}

/// Which workspace a project belongs to.
///
/// The seam has **no reverse reader** — no `workspace_of(project)` and no `repo(id)` — so this
/// resolves it from [`Backend::workspaces`], which already carries every workspace with its
/// `ProjectRef`s. A project linked into two workspaces resolves to the first by name; the section
/// always passes the scope's own workspace for the requests where that would be visible, so the
/// ambiguity only ever picks which tree a reply re-reads.
async fn workspace_of(backend: &Backend, project: ProjectId) -> Result<WorkspaceId> {
    backend
        .workspaces()
        .await?
        .into_iter()
        .find(|summary| summary.projects.iter().any(|row| row.project_id == project))
        .map(|summary| summary.workspace_id)
        .ok_or_else(|| StoreError::NotFound {
            entity: "workspace_project",
            id: project.to_string(),
        })
}

/// The twelve request names, in [`StoreRequest`](crate::store_worker::StoreRequest) order.
///
/// [`StoreRequest::name`](crate::store_worker::StoreRequest::name)'s arms and the section's
/// `Failed` match both read from here, so a
/// thirteenth request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 12] = [
    "hierarchy",
    "create_workspace",
    "update_workspace",
    "set_workspace_root",
    "create_project",
    "update_project",
    "create_repo",
    "update_repo",
    "set_repo_path",
    "delete_reach",
    "delete_workspace",
    "delete_project",
];

/// Every count of a [`DeleteReach`] with the word the warning pane uses, in the struct's **field
/// order** (`htui-core`'s `store::traits`).
///
/// The destructuring carries no `..`, and the array's length is written out: a field added to
/// [`DeleteReach`] fails to compile here rather than being silently left out of a warning that
/// claims to list everything a delete removes (the zero-omission rule, PRD D13). Shared by
/// [`reach_parts`] and [`reach_totals`] so the two cannot disagree about what they counted.
fn labelled(reach: &DeleteReach) -> [(u64, &'static str); 21] {
    let DeleteReach {
        workspace_links,
        workspace_box_paths,
        items,
        item_key_counters,
        item_kinds,
        step_graphs,
        phases,
        phase_agents,
        prompt_templates,
        repos,
        repo_box_paths,
        skill_bindings,
        runs,
        run_steps,
        session_events,
        run_step_commits,
        command_runs,
        notes,
        revisions,
        links,
        documents,
    } = *reach;
    [
        (workspace_links, "workspace links"),
        (workspace_box_paths, "box root paths"),
        (items, "items"),
        (item_key_counters, "key counters"),
        (item_kinds, "kinds"),
        (step_graphs, "graphs"),
        (phases, "phases"),
        (phase_agents, "phase agents"),
        (prompt_templates, "templates"),
        (repos, "repos"),
        (repo_box_paths, "repo paths"),
        (skill_bindings, "skill bindings"),
        (runs, "runs"),
        (run_steps, "run steps"),
        (session_events, "session events"),
        (run_step_commits, "run step commits"),
        (command_runs, "command runs"),
        (notes, "notes"),
        (revisions, "revisions"),
        (links, "links"),
        (documents, "documents"),
    ]
}

/// One `"{n} {label}"` per **non-zero** count, in [`DeleteReach`] field order.
///
/// A zero is omitted rather than printed: the pane lists what a delete takes, and "0 runs" is not
/// something being taken.
#[must_use]
pub fn reach_parts(reach: &DeleteReach) -> Vec<String> {
    labelled(reach)
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect()
}

/// `(rows, tables)`: the sum of every count and how many of them are non-zero.
#[must_use]
pub fn reach_totals(reach: &DeleteReach) -> (u64, usize) {
    let counts = labelled(reach);
    let rows = counts.iter().map(|(count, _)| *count).sum();
    let tables = counts.iter().filter(|(count, _)| *count > 0).count();
    (rows, tables)
}

#[cfg(test)]
mod tests {
    use super::{reach_parts, reach_totals};
    use htui_core::store::DeleteReach;

    /// Zeros are dropped and what is left reads in the struct's field order, not in the order the
    /// warning happens to mention them.
    #[test]
    fn reach_parts_omits_zeros_and_keeps_field_order() {
        let reach = DeleteReach {
            items: 12,
            phases: 15,
            documents: 2,
            ..DeleteReach::default()
        };
        assert_eq!(
            reach_parts(&reach),
            vec![
                "12 items".to_owned(),
                "15 phases".to_owned(),
                "2 documents".to_owned()
            ]
        );
        assert!(reach_parts(&DeleteReach::default()).is_empty());
    }

    #[test]
    fn reach_totals_counts_rows_and_tables() {
        let reach = DeleteReach {
            items: 12,
            phases: 15,
            documents: 2,
            ..DeleteReach::default()
        };
        assert_eq!(reach_totals(&reach), (29, 3));
        assert_eq!(reach_totals(&DeleteReach::default()), (0, 0));
    }
}
