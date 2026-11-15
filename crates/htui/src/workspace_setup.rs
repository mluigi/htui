//! Creating and editing workspaces and projects from the Workspaces tab, and a workspace's
//! default target (MOD-116 M3, `R-TUI-14`, `R-ENT-2`): five requests, each served through the
//! writer as this user, with identity and every path resolved here and never on the render side.
//!
//! - D4 [`StoreRequest::NewWorkspace`]: the folder is prepared first (made when missing, used as
//!   it is when present, refused when it is a file or a git repository: `R-ID-4`, htui writes the
//!   workspace's files into it and never into a repository), then the workspace row, then a
//!   `workspace_box_path` row only when the folder is not the slug's default.
//! - D5 [`StoreRequest::EditWorkspace`]: the folder prepared the same way and **never moved**,
//!   then `update_workspace` under compare-and-set; the folder row follows an applied edit.
//! - D6 [`StoreRequest::NewProject`]: an existing checkout registered where it is, a git URL
//!   cloned into `<workspace folder>/projects/<slug>`, or a new repository there with one empty
//!   commit; then the project, linked last, its primary repo and this box's path. A failure after
//!   the project row deletes it and removes what this request put on disk.
//! - D7: the clone is bounded (`CLONE_TIMEOUT`) and the store worker answers `NewProject` from
//!   its own task; every other disk step is one stat or `mkdir` under `spawn_blocking`, so
//!   nothing here blocks the async task (`R-NF-3`).
//! - D8: git through `htui_orch::isolate::git` (`Cli::clone_repo`, `Cli::init_with_empty_commit`,
//!   `repo_facts`); a remote is stored with its userinfo dropped.
//! - D9 [`StoreRequest::TargetChoices`] and [`StoreRequest::SetWorkspaceTarget`]: the live
//!   targets, the planned kinds marked, and the scope row's target set or cleared under
//!   compare-and-set.
//!
//! A refusal the person can act on is `Ok(`[`StoreReply::SetupRefused`]`)` in a sentence (D13); a
//! store failure stays an `Err`, which the worker answers as `Failed` (and `Unreachable` drops the
//! backend). Offline there is no writer and every request is refused before any disk or store
//! work.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::model::{
    BoxId, NewProject, NewRepo, NewWorkspace, ProjectId, ProjectRef, RepoBoxPath, RepoId, TargetId,
    TargetKind, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspacePatch, WorkspaceSummary,
    project_folder, workspace_folder,
};
use htui_core::root_path::{RootRefusal, canonical_root};
use htui_core::store::{
    CasOutcome, ReadStore, Result, StoreError, WriteStore, project_slug_refusal,
};
use htui_orch::infer::{redact_remote, redact_urls};
use htui_orch::isolate::IsolateError;
use htui_orch::isolate::git::{Cli, repo_facts};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::hierarchy::{self, MirrorAfterDelete};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::workspaces;

/// The five requests this module serves, in [`StoreRequest::name`]'s order (D15).
pub const REQUEST_NAMES: [&str; 5] = [
    "new_workspace",
    "edit_workspace",
    "new_project",
    "target_choices",
    "set_workspace_target",
];

/// Where a new project's repository comes from (D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectSource {
    /// A checkout already on this box, registered where it is; nothing is written on disk.
    Existing {
        /// The folder as typed (absolute).
        path: String,
    },
    /// A git URL, cloned into `<workspace folder>/projects/<slug>`.
    Clone {
        /// The URL as typed.
        url: String,
    },
    /// A new repository at `<workspace folder>/projects/<slug>` with one empty commit on `main`.
    NewRepo,
}

/// What an [`StoreRequest::EditWorkspace`] did (D5).
#[derive(Debug, Clone, PartialEq)]
pub enum EditOutcome {
    /// The row was written, and the folder row where one was due.
    Applied {
        /// The workspace as the switcher lists it now.
        workspace: WorkspaceSummary,
        /// Its folder on this box, canonical.
        folder: String,
    },
    /// The token was spent; nothing was written. The row as it is now, for the form's new token.
    Stale(Box<Workspace>),
}

/// One target a workspace may run on by default (D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetChoice {
    /// The target.
    pub id: TargetId,
    /// Its name.
    pub name: String,
    /// Its kind.
    pub kind: TargetKind,
    /// `kind.owner()` when this build does not run the kind (D9).
    pub planned: Option<&'static str>,
}

/// What the default-target picker offers (D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetChoices {
    /// The workspace's scope row and its `updated_at`, the token of a write; `None` without one.
    pub scope_row: Option<(ProjectId, DateTime<Utc>)>,
    /// The target the scope row names; `None` when it inherits the Settings default.
    pub current: Option<TargetId>,
    /// The Settings default's name (no `app_setting` row is the seeded local target).
    pub default_name: String,
    /// Every target that is not retired, in the store's (name) order.
    pub targets: Vec<TargetChoice>,
}

/// What a [`StoreRequest::SetWorkspaceTarget`] did (D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetWrite {
    /// The scope row now names the target (or none).
    Applied,
    /// The token was spent; nothing was written.
    Stale,
}

/// D4-D9 with this user's home directory. `Unreachable(DATABASE_UNREACHABLE)` offline;
/// `Backend` for a request that is not this module's.
///
/// # Errors
/// [`StoreError::Unreachable`] offline, whatever the store reports (a taken workspace slug
/// included), and [`StoreError::Backend`] for a request that is not one of [`REQUEST_NAMES`]:
/// `try_serve` routes only these five here, so a caller that reached it anyway is told which
/// request it sent.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    serve_with_home(backend, request, dirs::home_dir().as_deref()).await
}

/// [`serve`] with the home directory a test chooses, so no test writes under the real
/// `~/workspaces`: a workspace with no folder row lives at `<home>/workspaces/<slug>`.
///
/// # Errors
/// As [`serve`].
pub async fn serve_with_home(
    backend: &Backend,
    request: &StoreRequest,
    home: Option<&Path>,
) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let setup = Setup {
        backend,
        writer: &writer,
        request,
        home,
    };
    match request {
        StoreRequest::NewWorkspace {
            slug,
            name,
            description,
            folder,
        } => setup.new_workspace(slug, name, description, folder).await,
        StoreRequest::EditWorkspace {
            id,
            expected,
            patch,
            folder,
        } => setup.edit_workspace(*id, *expected, patch, folder).await,
        StoreRequest::NewProject {
            workspace,
            slug,
            name,
            description,
            source,
        } => {
            setup
                .new_project(*workspace, slug, name, description, source)
                .await
        }
        StoreRequest::TargetChoices(workspace) => setup.target_choices(*workspace).await,
        StoreRequest::SetWorkspaceTarget {
            workspace,
            expected,
            target,
        } => setup.set_target(*workspace, *expected, *target).await,
        other => Err(StoreError::Backend(format!(
            "`{}` is not a workspace setup request",
            other.name()
        ))),
    }
}

/// One request's context: the backend, its writer, the request (for its name) and the home.
struct Setup<'a> {
    backend: &'a Backend,
    writer: &'a Writer,
    request: &'a StoreRequest,
    home: Option<&'a Path>,
}

/// A checkout ready to register (D6): where it is and what its primary repo row says.
struct Checkout {
    /// The canonical path stored as this box's `repo_box_path`.
    path: String,
    /// `repo.name`.
    name: String,
    /// `repo.default_branch`.
    branch: String,
    /// `repo.remote_url`, its userinfo dropped.
    remote: Option<String>,
    /// The destination this request made, removed again when the registration fails.
    made: Option<PathBuf>,
}

impl Setup<'_> {
    /// D13: a refusal the person can act on, under this request's name.
    fn refused(&self, reason: impl Into<String>) -> StoreReply {
        StoreReply::SetupRefused {
            request: self.request.name(),
            reason: reason.into(),
        }
    }

    /// This box's id, when the store has a row for it.
    async fn this_box(&self) -> Result<Option<BoxId>> {
        Ok(self.backend.box_info().await?.map(|info| info.box_id))
    }

    /// Whether `canonical` is `slug`'s default folder, both resolved: a default that does not
    /// resolve, or no home, is never it (D4 step 5).
    async fn is_default(&self, slug: &str, canonical: &Path) -> Result<bool> {
        let Some(default) = self
            .home
            .and_then(|home| workspace_folder(slug, None, home))
        else {
            return Ok(false);
        };
        let resolved = off_task(move || canonical_root(&default).ok()).await?;
        Ok(resolved.as_deref() == Some(canonical))
    }

    /// D4.
    async fn new_workspace(
        &self,
        slug: &str,
        name: &str,
        description: &str,
        folder: &str,
    ) -> Result<StoreReply> {
        let this_box = self.this_box().await?;
        let created_by = self.backend.this_user().await?;
        let (canonical, made) = match prepare(folder).await? {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(self.refused(reason)),
        };
        let stored = match utf8(folder, &canonical) {
            Ok(stored) => stored,
            Err(reason) => return Ok(self.refused(undo_folder(reason, made, &canonical).await)),
        };
        let row_box = if self.is_default(slug, &canonical).await? {
            None
        } else {
            match hierarchy::box_id(this_box) {
                Ok(box_id) => Some(box_id),
                Err(err) => {
                    let reason = undo_folder(err.to_string(), made, &canonical).await;
                    return Ok(self.refused(reason));
                }
            }
        };
        let created = self
            .writer
            .create_workspace(NewWorkspace {
                id: WorkspaceId::new(),
                slug: slug.to_owned(),
                name: name.to_owned(),
                description: description.to_owned(),
                created_by,
            })
            .await;
        let workspace = match created {
            Ok(workspace) => workspace,
            Err(err) => {
                // A store failure stays an `Err` (D13); only the folder it made goes with it.
                let _ = undo_folder(String::new(), made, &canonical).await;
                return Err(err);
            }
        };
        if let Some(box_id) = row_box {
            self.writer
                .upsert_workspace_box_path(&WorkspaceBoxPath {
                    workspace_id: workspace.id,
                    box_id,
                    folder_path: stored.clone(),
                    updated_at: Utc::now(),
                })
                .await?;
        }
        Ok(StoreReply::WorkspaceCreated {
            workspace: self.summary(workspace.id, this_box).await?,
            folder: stored,
            made,
        })
    }

    /// The switcher row of a workspace a write just touched.
    async fn summary(
        &self,
        workspace: WorkspaceId,
        this_box: Option<BoxId>,
    ) -> Result<WorkspaceSummary> {
        hierarchy::snapshot(self.writer, workspace, this_box)
            .await?
            .map(|tree| tree.summary())
            .ok_or_else(|| StoreError::NotFound {
                entity: "workspace",
                id: workspace.to_string(),
            })
    }

    /// D5: the folder is prepared (never moved), then the row under compare-and-set, then the
    /// folder row when one exists or the folder is not the new slug's default.
    async fn edit_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: &WorkspacePatch,
        folder: &str,
    ) -> Result<StoreReply> {
        let this_box = self.this_box().await?;
        let current = self
            .writer
            .workspace(id)
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "workspace",
                id: id.to_string(),
            })?;
        let has_row = self
            .writer
            .workspace_box_paths(id)
            .await?
            .iter()
            .any(|row| Some(row.box_id) == this_box);
        let (canonical, made) = match prepare(folder).await? {
            Ok(prepared) => prepared,
            Err(reason) => return Ok(self.refused(reason)),
        };
        let stored = match utf8(folder, &canonical) {
            Ok(stored) => stored,
            Err(reason) => return Ok(self.refused(undo_folder(reason, made, &canonical).await)),
        };
        // The box is refused before the write, against the slug the patch leaves.
        let slug = patch.slug.as_deref().unwrap_or(&current.slug);
        if this_box.is_none() && (has_row || !self.is_default(slug, &canonical).await?) {
            let reason = hierarchy::box_id(this_box)
                .err()
                .map(|err| err.to_string())
                .unwrap_or_default();
            return Ok(self.refused(undo_folder(reason, made, &canonical).await));
        }
        let outcome = match self
            .writer
            .update_workspace(id, expected, patch.clone())
            .await?
        {
            CasOutcome::Applied(workspace) => {
                if has_row || !self.is_default(&workspace.slug, &canonical).await? {
                    self.writer
                        .upsert_workspace_box_path(&WorkspaceBoxPath {
                            workspace_id: id,
                            box_id: hierarchy::box_id(this_box)?,
                            folder_path: stored.clone(),
                            updated_at: Utc::now(),
                        })
                        .await?;
                }
                EditOutcome::Applied {
                    workspace: self.summary(id, this_box).await?,
                    folder: stored,
                }
            }
            // The folder made for a stale edit stays: an empty directory, said in the plan's
            // risk table.
            CasOutcome::Stale(workspace) => EditOutcome::Stale(Box::new(workspace)),
        };
        Ok(StoreReply::WorkspaceEdited {
            workspace: id,
            outcome,
        })
    }

    /// D6: every refusal that needs no disk first, then the source's disk work, then the rows.
    async fn new_project(
        &self,
        workspace: WorkspaceId,
        slug: &str,
        name: &str,
        description: &str,
        source: &ProjectSource,
    ) -> Result<StoreReply> {
        let this_box = self.this_box().await?;
        let box_id = match hierarchy::box_id(this_box) {
            Ok(box_id) => box_id,
            Err(err) => return Ok(self.refused(err.to_string())),
        };
        let tree = hierarchy::snapshot(self.writer, workspace, this_box)
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "workspace",
                id: workspace.to_string(),
            })?;
        if let Some(reason) = project_slug_refusal(slug) {
            return Ok(self.refused(reason));
        }
        let taken = self
            .backend
            .workspaces()
            .await?
            .iter()
            .any(|summary| summary.projects.iter().any(|project| project.slug == slug));
        if taken {
            return Ok(self.refused(format!("project slug `{slug}` is taken")));
        }
        let Some(root) = hierarchy::workspace_root(&tree, self.home) else {
            return Ok(self.refused("this workspace has no folder on this box"));
        };
        let created_by = self.backend.this_user().await?;

        let checkout = match source {
            ProjectSource::Existing { path } => existing(path).await?,
            ProjectSource::Clone { url } => clone(&root, slug, url).await?,
            ProjectSource::NewRepo => new_repo(&root, slug).await?,
        };
        let checkout = match checkout {
            Ok(checkout) => checkout,
            Err(reason) => return Ok(self.refused(reason)),
        };

        let id = ProjectId::new();
        let new = NewProject {
            id,
            slug: slug.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            created_by,
        };
        match self.register(workspace, new, &checkout, box_id).await {
            Ok(project) => Ok(StoreReply::ProjectCreated {
                workspace,
                project,
                repo_path: checkout.path,
            }),
            Err(err) => {
                self.unregister(id).await;
                if let Some(dest) = &checkout.made {
                    let _ = remove_tree(dest.clone()).await;
                }
                Err(err)
            }
        }
    }

    /// The rows of D6: the project linked last, its primary repo and this box's path of it.
    async fn register(
        &self,
        workspace: WorkspaceId,
        new: NewProject,
        checkout: &Checkout,
        box_id: BoxId,
    ) -> Result<ProjectRef> {
        let project = hierarchy::create_linked_project(self.writer, workspace, new).await?;
        let repo = self
            .writer
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: project.id,
                name: checkout.name.clone(),
                remote_url: checkout.remote.clone(),
                default_branch: checkout.branch.clone(),
                is_primary: true,
            })
            .await?;
        self.writer
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: repo.id,
                box_id,
                local_path: checkout.path.clone(),
                updated_at: Utc::now(),
            })
            .await?;
        let position = self
            .writer
            .workspace_projects(workspace)
            .await?
            .iter()
            .find(|link| link.project_id == project.id)
            .map_or(0, |link| link.position);
        Ok(ProjectRef {
            project_id: project.id,
            slug: project.slug,
            name: project.name,
            position,
        })
    }

    /// Deletes the project a failed registration created, if it got that far, and rebuilds the
    /// mirror as `DeleteProject` does. Best-effort: the failure is the reply either way.
    async fn unregister(&self, id: ProjectId) {
        match ReadStore::project(self.writer, id).await {
            Ok(Some(_)) => {}
            Ok(None) => return,
            Err(err) => {
                tracing::warn!(%id, %err, "new_project: cannot tell whether the project was created");
                return;
            }
        }
        if let Err(err) = self.writer.delete_project(id).await {
            tracing::warn!(%id, %err, "new_project: the half-registered project stays");
            return;
        }
        if let MirrorAfterDelete::Failed(err) = hierarchy::rebuild_mirror(self.backend).await {
            tracing::warn!(%id, %err, "new_project: the mirror was not rebuilt");
        }
    }

    /// D9: the scope row's token and target, the Settings default's name, the live targets.
    async fn target_choices(&self, workspace: WorkspaceId) -> Result<StoreReply> {
        if self.writer.workspace(workspace).await?.is_none() {
            return Err(StoreError::NotFound {
                entity: "workspace",
                id: workspace.to_string(),
            });
        }
        let scope = match self.writer.scope_project(workspace).await? {
            Some(id) => ReadStore::project(self.writer, id).await?,
            None => None,
        };
        let targets = ReadStore::targets(self.writer)
            .await?
            .into_iter()
            .filter(|target| target.retired_at.is_none())
            .map(|target| TargetChoice {
                id: target.id,
                planned: if target.kind.runnable() {
                    None
                } else {
                    target.kind.owner()
                },
                name: target.name,
                kind: target.kind,
            })
            .collect();
        let choices = TargetChoices {
            scope_row: scope.as_ref().map(|row| (row.id, row.updated_at)),
            current: scope.and_then(|row| row.target_id),
            default_name: workspaces::settings_default_name(self.writer).await?,
            targets,
        };
        Ok(StoreReply::TargetChoices {
            workspace,
            choices: Box::new(choices),
        })
    }

    /// D9: the scope row's target, set or cleared under compare-and-set.
    async fn set_target(
        &self,
        workspace: WorkspaceId,
        expected: DateTime<Utc>,
        target: Option<TargetId>,
    ) -> Result<StoreReply> {
        let Some(row) = self.writer.scope_project(workspace).await? else {
            return Ok(self
                .refused("this workspace has no scope row, so its default target cannot be set"));
        };
        let outcome = match self
            .writer
            .set_project_target(row, expected, target)
            .await?
        {
            CasOutcome::Applied(_) => TargetWrite::Applied,
            CasOutcome::Stale(_) => TargetWrite::Stale,
        };
        Ok(StoreReply::WorkspaceTargetSet { workspace, outcome })
    }
}

/// `work` on a blocking thread (`R-NF-3`): every stat, `mkdir` and `gix` read here is one.
async fn off_task<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| StoreError::Backend(err.to_string()))
}

/// [`prepare_folder`] off the async task.
async fn prepare(folder: &str) -> Result<std::result::Result<(PathBuf, bool), String>> {
    let typed = PathBuf::from(folder);
    off_task(move || prepare_folder(&typed)).await
}

/// D4 step 2, sync (called under `spawn_blocking`): the canonical folder and whether it was made
/// now. Absolute, else refused; a missing path is created with its parents; an existing
/// directory is used as it is and nothing in it is touched; a non-directory is refused; a
/// directory that opens as a git repository is refused (`R-ID-4`). Every sentence names the path
/// as typed.
fn prepare_folder(path: &Path) -> std::result::Result<(PathBuf, bool), String> {
    if !path.is_absolute() {
        return Err(RootRefusal::Relative(path.to_path_buf()).to_string());
    }
    let made = match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path)
                .map_err(|err| format!("`{}` could not be created: {err}", path.display()))?;
            true
        }
        // Anything else (a file, a link, an unreadable parent) is `canonical_root`'s to name.
        _ => false,
    };
    let canonical = canonical_root(path).map_err(|refusal| refusal.to_string())?;
    match repo_facts(&canonical) {
        Ok(None) => Ok((canonical, made)),
        Ok(Some(_)) => Err(format!(
            "`{}` is a git repository; htui writes the workspace's files into this folder, and \
             never into a repository (R-ID-4)",
            path.display()
        )),
        Err(err) => Err(format!(
            "`{}` could not be read: {}",
            path.display(),
            isolate_reason(err)
        )),
    }
}

/// The canonical path as stored text, or the refusal `hierarchy::canonical` gives a path that
/// resolves to something that is not UTF-8 (it names the path as typed).
fn utf8(typed: &str, canonical: &Path) -> std::result::Result<String, String> {
    canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("`{typed}` resolves to a path that is not valid UTF-8"))
}

/// D4: removes the workspace folder this request made, only while it is still empty (the leaf
/// alone; parents it made stay), and answers `reason`, naming the folder when it could not be
/// removed.
async fn undo_folder(reason: String, made: bool, folder: &Path) -> String {
    if !made {
        return reason;
    }
    let leaf = folder.to_path_buf();
    let removed = off_task(move || {
        let empty = std::fs::read_dir(&leaf).is_ok_and(|mut entries| entries.next().is_none());
        if empty {
            std::fs::remove_dir(&leaf).map_err(|err| err.to_string())
        } else {
            Ok(())
        }
    })
    .await
    .unwrap_or_else(|err| Err(err.to_string()));
    match removed {
        Ok(()) => reason,
        Err(err) => {
            tracing::warn!(folder = %folder.display(), %err, "the folder it made stays");
            format!("{reason} and `{}` could not be removed", folder.display())
        }
    }
}

/// D6: removes a destination this request made (a clone or a new repository), whole. `Err` with
/// the sentence suffix when it could not be.
async fn remove_tree(dest: PathBuf) -> std::result::Result<(), String> {
    let shown = dest.display().to_string();
    let removed = off_task(move || match std::fs::remove_dir_all(&dest) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.to_string()),
        _ => Ok(()),
    })
    .await
    .unwrap_or_else(|err| Err(err.to_string()));
    removed.map_err(|err| {
        tracing::warn!(dest = %shown, %err, "the destination it made stays");
        format!(" and `{shown}` could not be removed")
    })
}

/// The sentence inside an [`IsolateError`], without the `isolation refused: ` / `git: ` its
/// `Display` adds.
fn isolate_reason(err: IsolateError) -> String {
    match err {
        IsolateError::Refused(reason) | IsolateError::Git(reason) => reason,
        other => other.to_string(),
    }
}

/// D6, an existing checkout: registered where it is; nothing is written on disk.
async fn existing(typed: &str) -> Result<std::result::Result<Checkout, String>> {
    let typed = typed.to_owned();
    off_task(move || {
        let canonical = canonical_root(Path::new(&typed)).map_err(|refusal| refusal.to_string())?;
        let facts = match repo_facts(&canonical) {
            Ok(Some(facts)) => facts,
            Ok(None) => {
                return Err(format!(
                    "`{typed}` is not a git repository; plain folders come with MOD-145"
                ));
            }
            Err(err) => {
                return Err(format!(
                    "`{typed}` could not be read: {}",
                    isolate_reason(err)
                ));
            }
        };
        if facts.bare {
            return Err(format!(
                "`{typed}` is a bare repository; register a checkout, not a bare repository"
            ));
        }
        // `gix::open` also opens a repository's `.git` directory: what is registered is the work
        // tree it belongs to, never the typed path.
        let work = match canonical.file_name() {
            Some(name) if name == ".git" => canonical
                .parent()
                .map_or_else(|| canonical.clone(), Path::to_path_buf),
            _ => canonical,
        };
        let name = work
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("`{typed}` has no folder name to call its repo"))?;
        Ok(Checkout {
            path: utf8(&typed, &work)?,
            name,
            branch: facts.branch.unwrap_or_else(|| MAIN.to_owned()),
            remote: facts.origin.as_deref().map(redact_remote),
            made: None,
        })
    })
    .await
}

/// The branch of a new repository, and of a checkout whose `HEAD` names none.
const MAIN: &str = "main";

/// D6: `<root>/projects/<slug>`, which must not exist yet; `words` end the refusal when it does.
async fn destination(
    root: &Path,
    slug: &str,
    words: &'static str,
) -> Result<std::result::Result<PathBuf, String>> {
    let Some(dest) = project_folder(root, slug) else {
        return Ok(Err(format!("project slug `{slug}` cannot name a folder")));
    };
    off_task(move || {
        if std::fs::symlink_metadata(&dest).is_ok() {
            Err(format!("`{}` already exists; {words}", dest.display()))
        } else {
            Ok(dest)
        }
    })
    .await
}

/// The `git` this box runs, located off the async task.
async fn git() -> Result<std::result::Result<Cli, String>> {
    Ok(off_task(Cli::locate)
        .await?
        .map_err(|err| format!("a git URL or a new repo needs git: {}", isolate_reason(err))))
}

/// The canonical text of a destination this request made, and its branch from `HEAD`.
async fn made_checkout(
    dest: PathBuf,
    name: &str,
    remote: Option<String>,
) -> Result<std::result::Result<Checkout, String>> {
    let name = name.to_owned();
    off_task(move || {
        let canonical = canonical_root(&dest).map_err(|refusal| refusal.to_string())?;
        // A clone can exit 0 with an unborn `HEAD`; its branch is still named.
        let branch = repo_facts(&canonical)
            .ok()
            .flatten()
            .and_then(|facts| facts.branch)
            .unwrap_or_else(|| MAIN.to_owned());
        Ok(Checkout {
            path: utf8(&dest.display().to_string(), &canonical)?,
            name,
            branch,
            remote,
            made: Some(dest),
        })
    })
    .await
}

/// D6, a git URL: cloned into `<root>/projects/<slug>`; a failure removes what git left.
async fn clone(
    root: &Path,
    slug: &str,
    url: &str,
) -> Result<std::result::Result<Checkout, String>> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(Err("a git URL is needed to clone".to_owned()));
    }
    let dest = match destination(root, slug, "nothing was cloned").await? {
        Ok(dest) => dest,
        Err(reason) => return Ok(Err(reason)),
    };
    let git = match git().await? {
        Ok(git) => git,
        Err(reason) => return Ok(Err(reason)),
    };
    let parent = dest.parent().map(Path::to_path_buf).unwrap_or_default();
    let made_parent = off_task({
        let parent = parent.clone();
        move || std::fs::create_dir_all(&parent)
    })
    .await?;
    if let Err(err) = made_parent {
        return Ok(Err(format!(
            "`{}` could not be created: {err}",
            parent.display()
        )));
    }
    if let Err(err) = git.clone_repo(url, &dest).await {
        let reason = match err {
            IsolateError::Refused(reason) => reason,
            other => format!("git clone: {}", redact_urls(&isolate_reason(other))),
        };
        let left = remove_tree(dest).await.err().unwrap_or_default();
        return Ok(Err(format!("{reason}{left}")));
    }
    made_checkout(dest, slug, Some(redact_remote(url))).await
}

/// D6, a new repository: `<root>/projects/<slug>` with one empty commit on `main`, nothing but
/// `.git` written (`R-ID-4`); a failure removes it.
async fn new_repo(root: &Path, slug: &str) -> Result<std::result::Result<Checkout, String>> {
    let dest = match destination(root, slug, "nothing was created").await? {
        Ok(dest) => dest,
        Err(reason) => return Ok(Err(reason)),
    };
    let git = match git().await? {
        Ok(git) => git,
        Err(reason) => return Ok(Err(reason)),
    };
    let made = off_task({
        let dest = dest.clone();
        move || std::fs::create_dir_all(&dest)
    })
    .await?;
    if let Err(err) = made {
        return Ok(Err(format!(
            "`{}` could not be created: {err}",
            dest.display()
        )));
    }
    if let Err(err) = git.init_with_empty_commit(&dest, MAIN).await {
        let left = remove_tree(dest).await.err().unwrap_or_default();
        return Ok(Err(format!("{}{left}", isolate_reason(err))));
    }
    made_checkout(dest, slug, None).await
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use chrono::{Duration, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        NewProject, NewTarget, ProjectId, TargetId, TargetKind, WorkspaceId, WorkspacePatch,
    };
    use htui_core::seed;
    use htui_core::store::{CasOutcome, MemStore, ReadStore as _, StoreError, WriteStore as _};
    use htui_orch::infer::redact_remote;
    use htui_orch::isolate::git::testkit::repo_with_one_commit;
    use htui_orch::isolate::git::{Cli, head, repo_facts};
    use htui_orch::skip_without_git;
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    use super::{EditOutcome, ProjectSource, TargetWrite, serve, serve_with_home};
    use crate::store_worker::{StoreReply, StoreRequest};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// `request` served with `home`, failing the case on an `Err`.
    async fn served(backend: &Backend, request: StoreRequest, home: Option<&Path>) -> StoreReply {
        serve_with_home(backend, &request, home)
            .await
            .unwrap_or_else(|err| panic!("{} is served: {err}", request.name()))
    }

    /// The reason of a `SetupRefused` for `name`, failing the case on anything else.
    fn refusal(reply: StoreReply, name: &str) -> String {
        match reply {
            StoreReply::SetupRefused { request, reason } if request == name => reason,
            other => panic!("expected `{name}` refused, got {other:?}"),
        }
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).expect("the path resolves")
    }

    fn text(path: &Path) -> String {
        path.display().to_string()
    }

    fn new_workspace(slug: &str, folder: &Path) -> StoreRequest {
        StoreRequest::NewWorkspace {
            slug: slug.to_owned(),
            name: "Mobile".to_owned(),
            description: "phones".to_owned(),
            folder: text(folder),
        }
    }

    /// A new workspace `mobile` at `folder`: its id, the reply's folder and `made`.
    async fn created(
        backend: &Backend,
        folder: &Path,
        home: Option<&Path>,
    ) -> (WorkspaceId, String, bool) {
        match served(backend, new_workspace("mobile", folder), home).await {
            StoreReply::WorkspaceCreated {
                workspace,
                folder,
                made,
            } => (workspace.workspace_id, folder, made),
            other => panic!("expected the workspace created, got {other:?}"),
        }
    }

    /// This box's folder rows of `workspace`, as paths.
    async fn rows(backend: &Backend, workspace: WorkspaceId) -> Vec<String> {
        backend
            .writer()
            .expect("a memory backend writes")
            .workspace_box_paths(workspace)
            .await
            .expect("a memory read")
            .into_iter()
            .map(|row| row.folder_path)
            .collect()
    }

    /// Whether any workspace has `slug`.
    async fn has_workspace(backend: &Backend, slug: &str) -> bool {
        backend
            .workspaces()
            .await
            .expect("a memory read")
            .iter()
            .any(|summary| summary.slug == slug)
    }

    /// The slugs of `workspace`'s projects, in position order.
    async fn project_slugs(backend: &Backend, workspace: WorkspaceId) -> Vec<String> {
        backend
            .workspaces()
            .await
            .expect("a memory read")
            .into_iter()
            .find(|summary| summary.workspace_id == workspace)
            .expect("the workspace is listed")
            .projects
            .into_iter()
            .map(|project| project.slug)
            .collect()
    }

    fn new_project(slug: &str, source: ProjectSource) -> StoreRequest {
        StoreRequest::NewProject {
            workspace: ids::WORKSPACE_PLATFORM,
            slug: slug.to_owned(),
            name: slug.to_uppercase(),
            description: String::new(),
            source,
        }
    }

    /// `git <args>` in `cwd`, failing the case on a non-zero exit; its stdout.
    async fn git_out(git: &Cli, cwd: &Path, args: &[&str]) -> String {
        let argv: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let exited = git.run("oracle", cwd, &argv, &[]).await.expect("git runs");
        assert!(exited.ok(), "git {args:?} failed: {}", exited.stderr);
        exited.stdout
    }

    /// A bare repository `<dir>/remote.git` whose `HEAD` names `main`, holding `source`'s `HEAD`.
    async fn bare_remote(git: &Cli, dir: &Path, source: &Path) -> PathBuf {
        let remote = dir.join("remote.git");
        git_out(
            git,
            dir,
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=main",
                &text(&remote),
            ],
        )
        .await;
        git_out(
            git,
            source,
            &[
                "push",
                "--quiet",
                "--",
                &text(&remote),
                "HEAD:refs/heads/main",
            ],
        )
        .await;
        remote
    }

    /// The created project's reply, failing the case on anything else.
    fn project_created(reply: StoreReply) -> (htui_core::model::ProjectRef, String) {
        match reply {
            StoreReply::ProjectCreated {
                workspace,
                project,
                repo_path,
            } => {
                assert_eq!(workspace, ids::WORKSPACE_PLATFORM);
                (project, repo_path)
            }
            other => panic!("expected the project created, got {other:?}"),
        }
    }

    /// The one repo of `project` and this box's paths of it.
    async fn the_repo(
        backend: &Backend,
        project: ProjectId,
    ) -> (htui_core::model::Repo, Vec<String>) {
        let writer = backend.writer().expect("a memory backend writes");
        let mut repos = writer.repos(project).await.expect("a memory read");
        assert_eq!(repos.len(), 1, "{repos:?}");
        let repo = repos.remove(0);
        let paths = writer
            .repo_box_paths(repo.id)
            .await
            .expect("a memory read")
            .into_iter()
            .map(|row| row.local_path)
            .collect();
        (repo, paths)
    }

    // ---- D4 ----

    #[tokio::test]
    async fn a_new_workspace_makes_its_default_folder_and_records_no_row() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let folder = home.path().join("workspaces").join("mobile");

        let reply = served(
            &backend,
            new_workspace("mobile", &folder),
            Some(home.path()),
        )
        .await;
        let StoreReply::WorkspaceCreated {
            workspace,
            folder: answered,
            made,
        } = reply
        else {
            panic!("expected the workspace created, got {reply:?}");
        };
        assert!(made);
        assert!(folder.is_dir());
        assert_eq!(answered, text(&canonical(&folder)));
        assert_eq!(workspace.slug, "mobile");
        assert!(workspace.projects.is_empty(), "{workspace:?}");
        assert_eq!(
            rows(&backend, workspace.workspace_id).await,
            Vec::<String>::new()
        );
        let scope = backend
            .writer()
            .expect("a memory backend writes")
            .scope_project(workspace.workspace_id)
            .await
            .expect("a memory read");
        assert!(scope.is_some(), "the workspace has its scope row");
    }

    #[tokio::test]
    async fn a_new_workspace_elsewhere_records_its_canonical_row() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let elsewhere = tempfile::tempdir().expect("a throwaway place");
        let folder = elsewhere.path().join("deep").join("mobile");

        let (workspace, answered, made) = created(&backend, &folder, Some(home.path())).await;
        assert!(made);
        let stored = text(&canonical(&folder));
        assert_eq!(answered, stored);
        assert_eq!(rows(&backend, workspace).await, [stored]);
    }

    #[tokio::test]
    async fn an_existing_folder_is_used_as_it_is() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let folder = tempfile::tempdir().expect("an existing folder");
        std::fs::write(folder.path().join("notes.txt"), "mine\n").expect("a file is written");

        let (workspace, answered, made) = created(&backend, folder.path(), Some(home.path())).await;
        assert!(!made);
        assert_eq!(
            std::fs::read_to_string(folder.path().join("notes.txt")).expect("the file stays"),
            "mine\n"
        );
        assert_eq!(rows(&backend, workspace).await, [answered]);
    }

    #[tokio::test]
    async fn a_folder_that_is_a_file_or_a_repository_is_refused_before_any_write() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let file = home.path().join("a-file");
        std::fs::write(&file, "x").expect("a file is written");
        let repo = home.path().join("a-repo");
        std::fs::create_dir(&repo).expect("the repo dir");
        repo_with_one_commit(&repo);

        let reason = refusal(
            served(&backend, new_workspace("mobile", &file), Some(home.path())).await,
            "new_workspace",
        );
        assert_eq!(reason, format!("`{}` is not a directory", file.display()));

        let reason = refusal(
            served(&backend, new_workspace("mobile", &repo), Some(home.path())).await,
            "new_workspace",
        );
        assert_eq!(
            reason,
            format!(
                "`{}` is a git repository; htui writes the workspace's files into this folder, \
                 and never into a repository (R-ID-4)",
                repo.display()
            )
        );
        assert!(!has_workspace(&backend, "mobile").await);
    }

    #[tokio::test]
    async fn a_relative_folder_is_refused() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let reason = refusal(
            served(
                &backend,
                new_workspace("mobile", Path::new("relative/mobile")),
                Some(home.path()),
            )
            .await,
            "new_workspace",
        );
        assert_eq!(reason, "`relative/mobile` is not an absolute path");
        assert!(!has_workspace(&backend, "mobile").await);
    }

    /// `graphics` is the demo's: the store refuses it, and only a folder this request made goes.
    #[tokio::test]
    async fn a_taken_slug_removes_only_the_folder_it_made() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let made = home.path().join("made").join("graphics");
        let taken = serve_with_home(
            &backend,
            &new_workspace("graphics", &made),
            Some(home.path()),
        )
        .await;
        assert!(matches!(taken, Err(StoreError::Constraint(_))), "{taken:?}");
        assert!(!made.exists(), "the folder it made is gone");
        assert!(home.path().join("made").is_dir(), "the leaf alone");

        let kept = home.path().join("kept");
        std::fs::create_dir(&kept).expect("an existing folder");
        let taken = serve_with_home(
            &backend,
            &new_workspace("graphics", &kept),
            Some(home.path()),
        )
        .await;
        assert!(matches!(taken, Err(StoreError::Constraint(_))), "{taken:?}");
        assert!(kept.is_dir(), "a folder it did not make stays");
    }

    // ---- D5 ----

    /// The token of `workspace`'s row.
    async fn token(backend: &Backend, workspace: WorkspaceId) -> chrono::DateTime<Utc> {
        backend
            .writer()
            .expect("a memory backend writes")
            .workspace(workspace)
            .await
            .expect("a memory read")
            .expect("the workspace exists")
            .updated_at
    }

    fn edit(
        id: WorkspaceId,
        expected: chrono::DateTime<Utc>,
        patch: WorkspacePatch,
        folder: &Path,
    ) -> StoreRequest {
        StoreRequest::EditWorkspace {
            id,
            expected,
            patch,
            folder: text(folder),
        }
    }

    #[tokio::test]
    async fn an_edit_renames_and_points_at_a_new_folder_without_moving_the_old() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let old = home.path().join("old");
        let (workspace, _, _) = created(&backend, &old, Some(home.path())).await;
        std::fs::write(old.join("keep.txt"), "kept\n").expect("a file in the old folder");
        let new = home.path().join("new");

        let patch = WorkspacePatch {
            name: Some("Mobile apps".to_owned()),
            ..WorkspacePatch::default()
        };
        let expected = token(&backend, workspace).await;
        let reply = served(
            &backend,
            edit(workspace, expected, patch, &new),
            Some(home.path()),
        )
        .await;
        let StoreReply::WorkspaceEdited {
            workspace: asked,
            outcome:
                EditOutcome::Applied {
                    workspace: summary,
                    folder,
                },
        } = reply
        else {
            panic!("expected the edit applied, got {reply:?}");
        };
        assert_eq!(asked, workspace);
        assert_eq!(summary.name, "Mobile apps");
        let stored = text(&canonical(&new));
        assert_eq!(folder, stored);
        assert!(
            old.join("keep.txt").is_file(),
            "the old folder is never moved"
        );
        assert_eq!(rows(&backend, workspace).await, [stored]);
    }

    #[tokio::test]
    async fn a_slug_change_with_no_row_records_the_old_folder() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let default = home.path().join("workspaces").join("mobile");
        let (workspace, _, _) = created(&backend, &default, Some(home.path())).await;
        assert!(rows(&backend, workspace).await.is_empty());

        let patch = WorkspacePatch {
            slug: Some("apps".to_owned()),
            ..WorkspacePatch::default()
        };
        let expected = token(&backend, workspace).await;
        let reply = served(
            &backend,
            edit(workspace, expected, patch, &default),
            Some(home.path()),
        )
        .await;
        assert!(
            matches!(
                &reply,
                StoreReply::WorkspaceEdited { outcome: EditOutcome::Applied { workspace, .. }, .. }
                    if workspace.slug == "apps"
            ),
            "{reply:?}"
        );
        assert_eq!(
            rows(&backend, workspace).await,
            [text(&canonical(&default))]
        );
        assert!(!home.path().join("workspaces").join("apps").exists());
    }

    #[tokio::test]
    async fn a_stale_edit_writes_neither_the_row_nor_the_folder_row() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let default = home.path().join("workspaces").join("mobile");
        let (workspace, _, _) = created(&backend, &default, Some(home.path())).await;
        let current = token(&backend, workspace).await;

        let patch = WorkspacePatch {
            name: Some("Renamed".to_owned()),
            ..WorkspacePatch::default()
        };
        let reply = served(
            &backend,
            edit(
                workspace,
                current - Duration::seconds(5),
                patch,
                &home.path().join("elsewhere"),
            ),
            Some(home.path()),
        )
        .await;
        let StoreReply::WorkspaceEdited {
            outcome: EditOutcome::Stale(row),
            ..
        } = reply
        else {
            panic!("expected a stale edit, got {reply:?}");
        };
        assert_eq!((row.name.as_str(), row.updated_at), ("Mobile", current));
        assert!(rows(&backend, workspace).await.is_empty());
        assert_eq!(token(&backend, workspace).await, current);
    }

    // ---- D6 ----

    #[tokio::test]
    async fn a_new_repo_project_is_registered_with_one_empty_commit() {
        let Some(git) = skip_without_git!() else {
            return;
        };
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");

        let reply = served(
            &backend,
            new_project("api", ProjectSource::NewRepo),
            Some(home.path()),
        )
        .await;
        let (project, repo_path) = project_created(reply);
        let dest = home
            .path()
            .join("workspaces")
            .join("platform")
            .join("projects")
            .join("api");
        assert_eq!(repo_path, text(&canonical(&dest)));
        assert_eq!(project.slug, "api");
        assert_eq!(
            project_slugs(&backend, ids::WORKSPACE_PLATFORM)
                .await
                .last(),
            Some(&"api".to_owned()),
            "linked at the end"
        );

        let (repo, paths) = the_repo(&backend, project.project_id).await;
        assert_eq!(
            (
                repo.name.as_str(),
                repo.default_branch.as_str(),
                repo.remote_url.as_deref()
            ),
            ("api", "main", None)
        );
        assert!(repo.is_primary);
        assert_eq!(paths, [repo_path]);

        head(&dest).expect("HEAD resolves");
        assert_eq!(git_out(&git, &dest, &["ls-tree", "-r", "HEAD"]).await, "");
        let names: Vec<String> = std::fs::read_dir(&dest)
            .expect("the repository reads")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, [".git"], "nothing but .git is written (R-ID-4)");
    }

    #[tokio::test]
    async fn a_git_url_project_clones_into_projects_and_keeps_the_remote_redacted() {
        let Some(git) = skip_without_git!() else {
            return;
        };
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let tmp = tempfile::tempdir().expect("a throwaway place");
        let source = tmp.path().join("source");
        std::fs::create_dir(&source).expect("the source dir");
        let commit = repo_with_one_commit(&source);
        let remote = bare_remote(&git, tmp.path(), &source).await;
        let url = format!("file://{}", remote.display());

        let reply = served(
            &backend,
            new_project(
                "api",
                ProjectSource::Clone {
                    url: format!("  {url} "),
                },
            ),
            Some(home.path()),
        )
        .await;
        let (project, repo_path) = project_created(reply);
        let dest = home
            .path()
            .join("workspaces")
            .join("platform")
            .join("projects")
            .join("api");
        assert_eq!(repo_path, text(&canonical(&dest)));
        assert_eq!(head(&dest).expect("HEAD reads"), commit);
        assert!(dest.join("f").is_file(), "the tree is checked out");

        let (repo, paths) = the_repo(&backend, project.project_id).await;
        assert_eq!(repo.name, "api");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.remote_url, Some(redact_remote(&url)));
        assert_eq!(paths, [repo_path]);
    }

    #[tokio::test]
    async fn an_existing_git_folder_registers_where_it_is_with_origin_redacted() {
        let Some(git) = skip_without_git!() else {
            return;
        };
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let tmp = tempfile::tempdir().expect("a throwaway place");
        let checkout = tmp.path().join("web");
        std::fs::create_dir(&checkout).expect("the checkout dir");
        repo_with_one_commit(&checkout);
        git_out(
            &git,
            &checkout,
            &["remote", "add", "origin", "https://u:t@host/x.git"],
        )
        .await;
        let status = git_out(&git, &checkout, &["status", "--porcelain"]).await;
        let branch = repo_facts(&checkout)
            .expect("the facts read")
            .expect("a repository")
            .branch
            .expect("HEAD names a branch");

        let reply = served(
            &backend,
            new_project(
                "web",
                ProjectSource::Existing {
                    path: text(&checkout),
                },
            ),
            Some(home.path()),
        )
        .await;
        let (project, repo_path) = project_created(reply);
        assert_eq!(repo_path, text(&canonical(&checkout)));
        let (repo, paths) = the_repo(&backend, project.project_id).await;
        assert_eq!(repo.name, "web");
        assert_eq!(repo.default_branch, branch);
        assert_eq!(repo.remote_url.as_deref(), Some("https://host/x.git"));
        assert_eq!(paths, [repo_path]);
        assert_eq!(
            git_out(&git, &checkout, &["status", "--porcelain"]).await,
            status
        );
        assert!(
            !home.path().join("workspaces").exists(),
            "nothing is written on disk"
        );
    }

    #[tokio::test]
    async fn a_plain_folder_is_refused_naming_mod_145() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let plain = tempfile::tempdir().expect("a plain folder");
        let before = project_slugs(&backend, ids::WORKSPACE_PLATFORM).await;

        let reason = refusal(
            served(
                &backend,
                new_project(
                    "web",
                    ProjectSource::Existing {
                        path: text(plain.path()),
                    },
                ),
                Some(home.path()),
            )
            .await,
            "new_project",
        );
        assert_eq!(
            reason,
            format!(
                "`{}` is not a git repository; plain folders come with MOD-145",
                plain.path().display()
            )
        );
        assert_eq!(
            project_slugs(&backend, ids::WORKSPACE_PLATFORM).await,
            before
        );
    }

    #[tokio::test]
    async fn an_existing_destination_is_refused_and_kept() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let dest = home
            .path()
            .join("workspaces")
            .join("platform")
            .join("projects")
            .join("api");
        std::fs::create_dir_all(&dest).expect("the destination exists");
        std::fs::write(dest.join("mine.txt"), "mine\n").expect("a file in it");

        for (source, words) in [
            (
                ProjectSource::Clone {
                    url: "file:///nowhere.git".to_owned(),
                },
                "nothing was cloned",
            ),
            (ProjectSource::NewRepo, "nothing was created"),
        ] {
            let reason = refusal(
                served(&backend, new_project("api", source), Some(home.path())).await,
                "new_project",
            );
            assert_eq!(
                reason,
                format!("`{}` already exists; {words}", dest.display())
            );
            assert!(dest.join("mine.txt").is_file(), "the destination is kept");
        }
        assert!(
            !project_slugs(&backend, ids::WORKSPACE_PLATFORM)
                .await
                .contains(&"api".to_owned())
        );
    }

    /// `htui` is the demo's: refused before the clone, so no destination is made.
    #[tokio::test]
    async fn a_taken_project_slug_is_refused_before_cloning() {
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let reason = refusal(
            served(
                &backend,
                new_project(
                    "htui",
                    ProjectSource::Clone {
                        url: "file:///nowhere.git".to_owned(),
                    },
                ),
                Some(home.path()),
            )
            .await,
            "new_project",
        );
        assert_eq!(reason, "project slug `htui` is taken");
        assert!(
            !home.path().join("workspaces").exists(),
            "no destination is made"
        );
    }

    #[tokio::test]
    async fn a_failed_clone_leaves_no_dest_and_no_project() {
        let Some(_git) = skip_without_git!() else {
            return;
        };
        let backend = demo();
        let home = tempfile::tempdir().expect("a throwaway home");
        let tmp = tempfile::tempdir().expect("a throwaway place");
        let url = text(&tmp.path().join("nothing.git"));

        let reason = refusal(
            served(
                &backend,
                new_project("api", ProjectSource::Clone { url }),
                Some(home.path()),
            )
            .await,
            "new_project",
        );
        assert!(reason.starts_with("git clone: "), "{reason}");
        let dest = home
            .path()
            .join("workspaces")
            .join("platform")
            .join("projects")
            .join("api");
        assert!(!dest.exists(), "nothing is left at {}", dest.display());
        assert!(
            !project_slugs(&backend, ids::WORKSPACE_PLATFORM)
                .await
                .contains(&"api".to_owned())
        );
    }

    /// An unlinked project already holds `api`: the pre-check (linked slugs) passes, the repo is
    /// made, and the store refuses the project; the new repository goes and the other project
    /// stays.
    #[tokio::test]
    async fn a_failed_registration_deletes_the_project_and_the_clone() {
        let Some(_git) = skip_without_git!() else {
            return;
        };
        let backend = demo();
        let writer = backend.writer().expect("a memory backend writes");
        let other = writer
            .create_project(NewProject {
                id: ProjectId::new(),
                slug: "api".to_owned(),
                name: "Someone else's".to_owned(),
                description: String::new(),
                created_by: ids::USER,
            })
            .await
            .expect("an unlinked project");
        let home = tempfile::tempdir().expect("a throwaway home");

        let reply = serve_with_home(
            &backend,
            &new_project("api", ProjectSource::NewRepo),
            Some(home.path()),
        )
        .await;
        assert!(matches!(reply, Err(StoreError::Constraint(_))), "{reply:?}");
        let dest = home
            .path()
            .join("workspaces")
            .join("platform")
            .join("projects")
            .join("api");
        assert!(!dest.exists(), "the new repository is removed");
        assert!(
            !project_slugs(&backend, ids::WORKSPACE_PLATFORM)
                .await
                .contains(&"api".to_owned())
        );
        assert_eq!(
            writer.project(other.id).await.expect("a memory read"),
            Some(other),
            "the other project stays"
        );
    }

    #[tokio::test]
    async fn a_workspace_without_a_folder_cannot_take_a_project() {
        let backend = demo();
        let reason = refusal(
            served(&backend, new_project("api", ProjectSource::NewRepo), None).await,
            "new_project",
        );
        assert_eq!(reason, "this workspace has no folder on this box");
    }

    // ---- D9 ----

    /// The demo's Platform target choices.
    async fn choices(backend: &Backend) -> super::TargetChoices {
        match served(
            backend,
            StoreRequest::TargetChoices(ids::WORKSPACE_PLATFORM),
            None,
        )
        .await
        {
            StoreReply::TargetChoices { workspace, choices } => {
                assert_eq!(workspace, ids::WORKSPACE_PLATFORM);
                *choices
            }
            other => panic!("expected the target choices, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn target_choices_list_live_targets_and_mark_planned_kinds() {
        let backend = demo();
        let writer = backend.writer().expect("a memory backend writes");
        let docker = writer
            .create_target(NewTarget {
                id: TargetId::new(),
                name: "builder".to_owned(),
                kind: TargetKind::Docker,
                locator: serde_json::json!({}),
            })
            .await
            .expect("a docker target");
        let old = writer
            .create_target(NewTarget {
                id: TargetId::new(),
                name: "old".to_owned(),
                kind: TargetKind::Docker,
                locator: serde_json::json!({}),
            })
            .await
            .expect("another target");
        let retired = writer
            .retire_target(old.id, old.updated_at)
            .await
            .expect("the target retires");
        assert!(matches!(retired, CasOutcome::Applied(_)), "{retired:?}");

        let choices = choices(&backend).await;
        assert_eq!(choices.default_name, "this box");
        assert_eq!(choices.current, None);
        let scope = writer
            .project(ids::PROJECT_SCOPE_PLATFORM)
            .await
            .expect("a memory read")
            .expect("Platform's scope row");
        assert_eq!(choices.scope_row, Some((scope.id, scope.updated_at)));
        assert_eq!(
            choices
                .targets
                .iter()
                .map(|choice| (choice.name.as_str(), choice.kind, choice.planned))
                .collect::<Vec<_>>(),
            [
                ("builder", TargetKind::Docker, Some("MOD-44")),
                (seed::LOCAL_TARGET_NAME, TargetKind::Local, None),
            ]
        );
        assert_eq!(choices.targets[0].id, docker.id);
    }

    /// The target the scope row of Platform names.
    async fn scope_target(backend: &Backend) -> Option<TargetId> {
        backend
            .writer()
            .expect("a memory backend writes")
            .project(ids::PROJECT_SCOPE_PLATFORM)
            .await
            .expect("a memory read")
            .expect("Platform's scope row")
            .target_id
    }

    fn set_target(expected: chrono::DateTime<Utc>, target: Option<TargetId>) -> StoreRequest {
        StoreRequest::SetWorkspaceTarget {
            workspace: ids::WORKSPACE_PLATFORM,
            expected,
            target,
        }
    }

    #[tokio::test]
    async fn the_workspace_target_is_set_and_cleared_on_the_scope_row() {
        let backend = demo();
        for target in [Some(seed::LOCAL_TARGET_ID), None] {
            let (_, token) = choices(&backend).await.scope_row.expect("a scope row");
            let reply = served(&backend, set_target(token, target), None).await;
            assert!(
                matches!(
                    reply,
                    StoreReply::WorkspaceTargetSet {
                        workspace: ids::WORKSPACE_PLATFORM,
                        outcome: TargetWrite::Applied,
                    }
                ),
                "{reply:?}"
            );
            assert_eq!(scope_target(&backend).await, target);
            assert_eq!(choices(&backend).await.current, target);
        }
    }

    #[tokio::test]
    async fn a_stale_target_write_is_refused() {
        let backend = demo();
        let (_, token) = choices(&backend).await.scope_row.expect("a scope row");
        let reply = served(
            &backend,
            set_target(token - Duration::seconds(5), Some(seed::LOCAL_TARGET_ID)),
            None,
        )
        .await;
        assert!(
            matches!(
                reply,
                StoreReply::WorkspaceTargetSet {
                    outcome: TargetWrite::Stale,
                    ..
                }
            ),
            "{reply:?}"
        );
        assert_eq!(scope_target(&backend).await, None);
    }

    // ---- offline, routing ----

    /// Offline there is no writer: every request is refused before any disk or store work, and
    /// the store worker answers it `Failed` under its own name.
    #[tokio::test]
    async fn offline_every_setup_request_is_refused_before_any_write() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "setup-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        let home = tempfile::tempdir().expect("a throwaway home");
        let folder = home.path().join("off").join("mobile");
        let requests = [
            new_workspace("mobile", &folder),
            edit(
                ids::WORKSPACE_PLATFORM,
                Utc::now(),
                WorkspacePatch::default(),
                &folder,
            ),
            new_project("api", ProjectSource::NewRepo),
            StoreRequest::TargetChoices(ids::WORKSPACE_PLATFORM),
            set_target(Utc::now(), None),
        ];
        assert_eq!(
            requests.iter().map(StoreRequest::name).collect::<Vec<_>>(),
            super::REQUEST_NAMES
        );
        for request in &requests {
            assert!(
                matches!(
                    serve_with_home(&backend, request, Some(home.path())).await,
                    Err(StoreError::Unreachable(message)) if message == DATABASE_UNREACHABLE
                ),
                "{}",
                request.name()
            );
            let reply = crate::store_worker::serve(&backend, request).await;
            assert!(
                matches!(
                    &reply,
                    StoreReply::Failed { request: name, message }
                        if *name == request.name() && message.ends_with(DATABASE_UNREACHABLE)
                ),
                "{reply:?}"
            );
        }
        assert!(!home.path().join("off").exists(), "no folder is made");
        assert!(
            !home.path().join("workspaces").exists(),
            "no folder is made"
        );
        cache.close().await;
    }

    /// A request that is not this module's is refused by name, not served.
    #[tokio::test]
    async fn another_request_is_refused_by_name() {
        let reply = serve(&demo(), &StoreRequest::Workspaces).await;
        assert!(
            matches!(&reply, Err(StoreError::Backend(message)) if message.contains("`workspaces`")),
            "{reply:?}"
        );
    }
}
