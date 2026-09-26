//! The hierarchy the Settings tab edits: one worker-assembled snapshot per read, thirteen served
//! requests, identity filled here and never on the render side (MOD-15 milestone 3, D5/D6/D10).
//!
//! A section names a read and is handed rows (`R-NF-3`): nothing below this module is reachable
//! from `ui/`, and the two identity columns a write needs — `workspace.created_by` and the box a
//! path belongs to — are resolved from [`Backend`] here, so no view ever holds a `UserId` or a
//! `BoxId`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use htui_core::model::{
    BoxId, NewProject, NewRepo, NewWorkspace, Project, ProjectId, ProjectRef, Repo, RepoBoxPath,
    RepoId, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspaceProject, WorkspaceSummary,
};
use htui_core::root_path::canonical_root;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, Result, StoreError, WriteStore,
};
use htui_orch::infer::{Checkout, Choice, MatchedBy};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::store_worker::{StoreReply, StoreRequest};

/// A workspace, its projects in position order, their repos by name, and this box's paths.
///
/// One read answers the whole tree (D5): the section re-renders from this and never patches its
/// rows from a single returned row, so there is exactly one source of truth on the render side.
///
/// It carries **no `BoxId`**: the filtering D5 asks for is done by [`snapshot`] against the
/// `this_box` it was handed, and the rows that survive it already name the box they belong to. A
/// field here would be the one thing that made blueprint §15's "no view holds a `UserId` or a
/// `BoxId`" false by ownership rather than by convention.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchySnapshot {
    /// The workspace row itself; `updated_at` is the CAS token an editor opens on.
    pub workspace: Workspace,
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
/// [`StoreReply::Failed`]: the rows are already gone, and
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

/// What one `InferRepoPaths` did (MOD-7 milestone 4, plan D116). Carries no URL and no id of a
/// box or a user: repo names, canonical paths and outcomes only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferReport {
    /// This box's workspace root, canonical; `None` when the workspace has no root here, in which
    /// case nothing was walked and `repos` is empty.
    pub root: Option<String>,
    /// The walk hit `htui_orch::infer::MAX_DIRS`; every repo without a row is `ScanTruncated`.
    pub truncated: bool,
    /// One per repo of the workspace, in the tree's order (projects by position, repos by name).
    pub repos: Vec<RepoInference>,
}

/// One repo's line of an [`InferReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInference {
    /// The repo.
    pub repo: RepoId,
    /// `repo.name`, what the notice prints.
    pub name: String,
    /// What happened.
    pub outcome: InferOutcome,
}

/// Why a repo did or did not get a row (plan D116).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferOutcome {
    /// A row existed before the pass, or a manual write landed first (the insert answered `false`).
    AlreadySet,
    /// A row was written.
    Inferred {
        /// The canonical path stored.
        path: String,
        /// Which rung chose it.
        by: MatchedBy,
    },
    /// No checkout matched.
    NoMatch,
    /// Several did; nothing was written.
    Ambiguous {
        /// How many.
        candidates: usize,
    },
    /// The chosen path was refused by `canonical_root`, or the store refused the row. The
    /// sentence names the path as found, never a link's target.
    Refused(String),
    /// The scan was cut short, so nothing was inferred.
    ScanTruncated,
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
/// the hierarchy variants below here, so it is unreachable from the shell, and a caller that
/// reached it anyway is better told which request it sent than killed.
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
            // The box first: it is an `Option` already in hand, and a box with no row is refused
            // whatever the path says — paying `canonical`'s `spawn_blocking` stat to answer with
            // the *path's* refusal would both cost a blocking call and name the wrong problem.
            let box_id = box_id(this_box)?;
            let root_path = canonical(path).await?;
            writer
                .upsert_workspace_box_path(&WorkspaceBoxPath {
                    workspace_id: *id,
                    box_id,
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
            // One past the highest, not the count: a middle project that was deleted leaves
            // `[0, 2]`, where `links.len()` is `2` and `workspace_project` has no unique on
            // `position` to catch the collision (`0001_init.sql:161-166`).
            let position = links
                .iter()
                .map(|link| link.position)
                .max()
                .map_or(0, |highest| highest.saturating_add(1));
            writer
                .upsert_workspace_project(&WorkspaceProject {
                    workspace_id: *workspace,
                    project_id: project.id,
                    position,
                })
                .await?;
            reread(&writer, *workspace, this_box).await
        }
        StoreRequest::UpdateProject {
            id,
            expected,
            patch,
        } => {
            // Before the write: a project linked to no workspace has no tree for the reply to
            // re-read, and resolving it afterwards would answer `Failed` for a write that applied.
            // It doubles as the check that the project is linked at all.
            let ws = workspace_of(backend, *id).await?;
            let outcome = writer.update_project(*id, *expected, patch.clone()).await?;
            cas(&writer, ws, this_box, &outcome).await
        }
        StoreRequest::CreateRepo {
            project,
            name,
            remote_url,
            default_branch,
            is_primary,
        } => {
            let ws = workspace_of(backend, *project).await?;
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
            reread(&writer, ws, this_box).await
        }
        StoreRequest::UpdateRepo {
            project,
            id,
            expected,
            patch,
        } => {
            let ws = workspace_of(backend, *project).await?;
            let outcome = writer.update_repo(*id, *expected, patch.clone()).await?;
            cas(&writer, ws, this_box, &outcome).await
        }
        StoreRequest::SetRepoPath {
            project,
            repo,
            path,
        } => {
            // Everything that can refuse without touching the disk, first; then the stat; then the
            // write. See `SetWorkspaceRoot` and `UpdateProject` above for the two halves of this.
            let box_id = box_id(this_box)?;
            let ws = workspace_of(backend, *project).await?;
            let local_path = canonical(path).await?;
            writer
                .upsert_repo_box_path(&RepoBoxPath {
                    repo_id: *repo,
                    box_id,
                    local_path,
                    updated_at: Utc::now(),
                })
                .await?;
            reread(&writer, ws, this_box).await
        }
        StoreRequest::DeleteReach(target) => Ok(StoreReply::DeleteReach(
            writer.delete_reach(*target).await?.map(Box::new),
        )),
        StoreRequest::DeleteWorkspace(id) => Ok(StoreReply::Deleted {
            target: DeleteTarget::Workspace(*id),
            reach: Box::new(writer.delete_workspace(*id).await?),
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
                reach: Box::new(reach),
                mirror,
            })
        }
        StoreRequest::InferRepoPaths(ws) => infer(backend, &writer, *ws, this_box).await,
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
///
/// A canonical path that is not UTF-8 is a refusal of the same shape rather than a
/// `to_string_lossy` that stores a string naming nothing on disk. What was typed is a `String` and
/// so always UTF-8; only what a link resolves to can get here, and the sentence names the path as
/// typed — never the target (`R-BOX-4`).
async fn canonical(path: &str) -> Result<String> {
    let typed = path.to_owned();
    let resolved = tokio::task::spawn_blocking({
        let typed = typed.clone();
        move || canonical_root(Path::new(&typed))
    })
    .await
    .map_err(|err| StoreError::Backend(err.to_string()))?
    .map_err(|refusal| StoreError::Constraint(refusal.to_string()))?;
    resolved.into_os_string().into_string().map_err(|_| {
        StoreError::Constraint(format!(
            "`{typed}` resolves to a path that is not valid UTF-8"
        ))
    })
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

/// `InferRepoPaths` (MOD-7 milestone 4, PRD D5, plan D114, D116, D124): this box's filesystem,
/// this box's rows, this workspace only.
///
/// The box first, then the tree, then this box's root, canonicalised (a legacy link row walks its
/// target). With no root, the answer is `root: None` and nothing is walked. With every repo
/// already set, nothing is walked either. Otherwise one `find_checkouts` under `spawn_blocking`,
/// then per repo without a row, in tree order: `choose` against the paths already held on this
/// box (each row as stored and as it resolves), `canonical` of the chosen path, and `infer_repo_box_path`. A path this pass writes is held
/// for every later repo. A truncated scan infers nothing. The reply is the re-read tree and the
/// report.
async fn infer(
    backend: &Backend,
    writer: &Writer,
    ws: WorkspaceId,
    this_box: Option<BoxId>,
) -> Result<StoreReply> {
    let box_id = box_id(this_box)?;
    let tree = snapshot(writer, ws, this_box)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "workspace",
            id: ws.to_string(),
        })?;
    let Some(root_row) = &tree.root_path else {
        return Ok(StoreReply::RepoPathsInferred {
            tree: Box::new(tree),
            report: InferReport {
                root: None,
                truncated: false,
                repos: Vec::new(),
            },
        });
    };
    // A root that no longer resolves is a `Constraint`, which the status line reads as
    // `infer_repo_paths: constraint violated: …`.
    let root = canonical(&root_row.root_path).await?;

    let entries: Vec<&RepoEntry> = tree
        .projects
        .iter()
        .flat_map(|entry| entry.repos.iter())
        .collect();
    // Walk only when some repo has no row here: a workspace whose repos are all set answers
    // without touching the disk.
    let mut walk = if entries.iter().all(|entry| entry.local_path.is_some()) {
        None
    } else {
        // Every repo's row on this box, not only this workspace's: a checkout another repo already
        // owns is never a candidate.
        let rows = backend.repo_paths(box_id).await?;
        let (scan, held) = tokio::task::spawn_blocking({
            let root = PathBuf::from(&root);
            move || {
                // Each row as stored and as it resolves: a legacy row stored as a link (before
                // F-102) names a checkout the walk yields by its target, and must hold it too.
                let mut held = BTreeSet::new();
                for row in rows {
                    let raw = PathBuf::from(row.local_path);
                    if let Ok(resolved) = canonical_root(&raw) {
                        held.insert(resolved);
                    }
                    held.insert(raw);
                }
                (htui_orch::infer::find_checkouts(&root), held)
            }
        })
        .await
        .map_err(|err| StoreError::Backend(err.to_string()))?;
        Some((scan, held))
    };

    let truncated = walk.as_ref().is_some_and(|(scan, _)| scan.truncated);
    let mut repos = Vec::with_capacity(entries.len());
    for entry in entries {
        let outcome = match &mut walk {
            // D134: sequentially, in tree order, so a path written for one repo is held for the
            // next. A truncated scan writes nothing.
            Some((scan, held)) if entry.local_path.is_none() => {
                if scan.truncated {
                    InferOutcome::ScanTruncated
                } else {
                    infer_one(writer, &entry.repo, box_id, &scan.checkouts, held).await?
                }
            }
            // A row on this box before the pass; with no walk at all, every repo had one.
            _ => InferOutcome::AlreadySet,
        };
        repos.push(RepoInference {
            repo: entry.repo.id,
            name: entry.repo.name.clone(),
            outcome,
        });
    }

    let fresh = snapshot(writer, ws, this_box)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "workspace",
            id: ws.to_string(),
        })?;
    Ok(StoreReply::RepoPathsInferred {
        tree: Box::new(fresh),
        report: InferReport {
            root: Some(root),
            truncated,
            repos,
        },
    })
}

/// One repo without a row on this box: `choose`, `canonical` of the chosen path, then the
/// insert-if-absent write (plan D116). A refusal, by the guard or by the store's constraints, is
/// the repo's outcome rather than the request's; any other error fails the request.
async fn infer_one(
    writer: &Writer,
    repo: &Repo,
    box_id: BoxId,
    checkouts: &[Checkout],
    held: &mut BTreeSet<PathBuf>,
) -> Result<InferOutcome> {
    let (path, by) = match htui_orch::infer::choose(repo, checkouts, held) {
        Choice::NoMatch => return Ok(InferOutcome::NoMatch),
        Choice::Ambiguous { candidates } => return Ok(InferOutcome::Ambiguous { candidates }),
        Choice::Inferred { path, by } => (path, by),
    };
    let Some(found) = path.to_str() else {
        return Ok(InferOutcome::Refused(
            "the checkout's path is not valid UTF-8".to_owned(),
        ));
    };
    // The same guard `SetRepoPath` runs (F-102): the walk never follows a link, so this is the
    // path as found, and a refusal names it and never a target.
    let local_path = match canonical(found).await {
        Ok(local_path) => local_path,
        Err(StoreError::Constraint(message)) => return Ok(InferOutcome::Refused(message)),
        Err(err) => return Err(err),
    };
    let written = writer
        .infer_repo_box_path(&RepoBoxPath {
            repo_id: repo.id,
            box_id,
            local_path: local_path.clone(),
            // The store's trigger stamps the column, as for every other path write.
            updated_at: Utc::now(),
        })
        .await;
    match written {
        Ok(true) => {
            held.insert(path);
            held.insert(PathBuf::from(&local_path));
            Ok(InferOutcome::Inferred {
                path: local_path,
                by,
            })
        }
        // A row landed between the tree read and this insert, a manual write most likely: it wins.
        Ok(false) => Ok(InferOutcome::AlreadySet),
        Err(StoreError::Constraint(message)) => Ok(InferOutcome::Refused(message)),
        Err(err) => Err(err),
    }
}

/// The thirteen request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's
/// `Failed` match both read from here, so a
/// fourteenth request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 13] = [
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
    "infer_repo_paths",
];

/// Every count of a [`DeleteReach`] with the word the warning pane uses, in the struct's **field
/// order** (`htui-core`'s `store::traits`).
///
/// The destructuring carries no `..`, and the array's length is written out: a field added to
/// [`DeleteReach`] fails to compile here rather than being silently left out of a warning that
/// claims to list everything a delete removes (the zero-omission rule, PRD D13). Shared by
/// [`reach_parts`] and [`reach_totals`] so the two cannot disagree about what they counted.
fn labelled(reach: &DeleteReach) -> [(u64, &'static str); 28] {
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
        run_step_trees,
        command_runs,
        notes,
        revisions,
        links,
        documents,
        requirement_specs,
        requirement_areas,
        requirement_key_counters,
        requirements,
        requirement_revisions,
        item_requirements,
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
        (run_step_trees, "run step trees"),
        (command_runs, "command runs"),
        (notes, "notes"),
        (revisions, "revisions"),
        (links, "links"),
        (documents, "documents"),
        (requirement_specs, "requirement specs"),
        (requirement_areas, "requirement areas"),
        (requirement_key_counters, "requirement key counters"),
        (requirements, "requirements"),
        (requirement_revisions, "requirement revisions"),
        (item_requirements, "citations"),
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
    use super::{box_id, reach_parts, reach_totals};
    use htui_core::model::BoxId;
    use htui_core::store::{DeleteReach, StoreError};

    /// A path write on a box with no row is refused by name rather than stored under a nil id.
    /// There is no backend this is reachable from in a test — a `MemStore` either has a
    /// `this_box` or has no `box` row for a path to reference either way — so the refusal is
    /// pinned here, where the worker builds it.
    #[test]
    fn a_box_without_a_row_cannot_set_a_path() {
        assert_eq!(box_id(Some(BoxId::default())), Ok(BoxId::default()));
        assert_eq!(
            box_id(None),
            Err(StoreError::NotFound {
                entity: "box",
                id: "(this box)".to_owned(),
            })
        );
        assert_eq!(
            box_id(None).unwrap_err().to_string(),
            "box `(this box)` not found"
        );
    }

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

    /// MOD-38 (blueprint F1): the six requirement tables a project delete cascades are named in
    /// the warning, after `documents` and in field order, with `item_requirement` read as
    /// citations.
    #[test]
    fn reach_parts_names_the_requirement_tables() {
        let reach = DeleteReach {
            items: 12,
            documents: 2,
            requirement_specs: 1,
            requirement_areas: 2,
            requirement_key_counters: 2,
            requirements: 3,
            requirement_revisions: 4,
            item_requirements: 6,
            ..DeleteReach::default()
        };
        assert_eq!(
            reach_parts(&reach),
            vec![
                "12 items".to_owned(),
                "2 documents".to_owned(),
                "1 requirement specs".to_owned(),
                "2 requirement areas".to_owned(),
                "2 requirement key counters".to_owned(),
                "3 requirements".to_owned(),
                "4 requirement revisions".to_owned(),
                "6 citations".to_owned(),
            ]
        );
        assert_eq!(reach_totals(&reach), (32, 8));
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
