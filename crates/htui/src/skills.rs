//! The skill library behind the Skills tab's library and attachment panes (MOD-9 milestone 3,
//! D81): one snapshot read per scope and the four compare-and-set writes, the
//! [`crate::templates`] shape; and, since milestone 4, the `SKILL.md` import, whose walk and
//! writes are [`crate::skill_import`]'s.
//!
//! One read per event and never one per keystroke, one reply out. Every write re-reads the whole
//! snapshot rather than handing the view the row its outcome carries: the view renders from the
//! snapshot and never patches a row into it. A write that applied answers
//! [`StoreReply::SkillWritten`] naming what it wrote (MOD-59), so the view lands it on its own
//! reply and never by searching the snapshot; a re-read that fails after it still answers
//! `SkillWritten`, carrying the failure, because the write has landed. A write that missed its
//! token, or whose skill, project or phase is gone, answers [`StoreReply::SkillsStale`] naming
//! which write it was, so the view knows which draft keeps its text ([`crate::box_settings`]' rule
//! for a vanished box).
//!
//! The worker fills `created_by` from [`Backend::this_user`] and mints the new skill's id; the
//! render side never holds a `UserId` (`R-NF-3`). Nothing here reads the clock: the store stamps
//! every instant.

use chrono::{DateTime, Utc};
use htui_core::model::{
    Binding, BindingKey, BindingRef, EnvValue, ItemKind, McpServer, McpServerId, McpTransport,
    NewMcpServer, NewSkill, NewSkillVersion, ProjectId, RepoBoxPath, Scope, Skill, SkillId,
    SkillVersion, StepGraph, StepGraphPhase, WorkspaceId,
};
use htui_core::store::{BindingHome, CasOutcome, Result, StoreError, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY, Writer};

use crate::skill_import::{self, SkillImports};
use crate::store_worker::{StoreReply, StoreRequest, WriteOutcome};

/// The whole Skills view in one read (D81): the library, the global attachments, the scope
/// workspace's attachments (MOD-127 M1), and each scope project's attachments, item kinds, graphs
/// and repo names.
///
/// `PartialEq` only: [`StepGraph`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillsSnapshot {
    /// The library in `name` byte order, each skill with every version.
    pub skills: Vec<SkillEntry>,
    /// The MCP servers in `name` byte order (MOD-127 M2 D2). Their attachments are rows of
    /// [`global`](Self::global), [`workspaces`](Self::workspaces) and
    /// [`projects`](Self::projects) whose [`mcp_server_of`](Self::mcp_server_of) answers `Some`.
    pub mcp_servers: Vec<McpServer>,
    /// The global attachments (`skill_bindings(BindingHome::Global)`), in D91 order.
    pub global: Vec<Binding>,
    /// One entry per workspace the view attaches at: the scope's workspace
    /// (`skill_bindings(BindingHome::Workspace(w))`, D91 order).
    pub workspaces: Vec<WorkspaceSkills>,
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectSkills>,
}

/// One workspace's side of the attachment pane (MOD-127 M1 D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSkills {
    /// Which workspace.
    pub workspace: WorkspaceId,
    /// `workspace.name`, for the row's label; the id's first eight chars when the row is gone.
    pub name: String,
    /// Its attachments, in D91 order.
    pub bindings: Vec<Binding>,
}

/// One skill and its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    /// The `skill` row.
    pub skill: Skill,
    /// Every version, ascending (R-27: the library is small, so all of them travel).
    pub versions: Vec<SkillVersion>,
}

/// One scope project's side of the attachment pane.
///
/// `PartialEq` only: [`StepGraph`] and [`StepGraphPhase`] derive no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSkills {
    /// Which project.
    pub project: ProjectId,
    /// Its project, item-kind and phase attachments, `(skill_id, item_kind_id, phase_id)` byte
    /// order with the project row first (D91, MOD-127 D6).
    pub bindings: Vec<Binding>,
    /// Its item kinds, by `position` (the store's order): the rows an item-kind attachment sits
    /// under.
    pub item_kinds: Vec<ItemKind>,
    /// Its graphs in `name` byte order, **override graphs excluded** (a per-item clone is not
    /// where an attachment is authored), each with its phases by `position`.
    pub graphs: Vec<(StepGraph, Vec<StepGraphPhase>)>,
    /// Its repo names, byte order: D79's repo picker and the marks on a glob naming no repo.
    pub repos: Vec<String>,
    /// MOD-9 D126: its repo names with no `repo_box_path` row for this box, byte order; every
    /// repo when the box is unregistered.
    pub unrooted: Vec<String>,
}

impl SkillsSnapshot {
    /// One skill's entry; `None` when the library holds no such skill.
    #[must_use]
    pub fn entry(&self, skill: SkillId) -> Option<&SkillEntry> {
        self.skills.iter().find(|entry| entry.skill.id == skill)
    }

    /// The skill named `name`.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&SkillEntry> {
        self.skills.iter().find(|entry| entry.skill.name == name)
    }

    /// A skill's highest version: the compare-and-set token a save of that skill passes. `None`
    /// for an unknown skill and for one with no version row.
    #[must_use]
    pub fn head(&self, skill: SkillId) -> Option<&SkillVersion> {
        self.entry(skill)?
            .versions
            .iter()
            .max_by_key(|row| row.version)
    }

    /// One version of a skill.
    #[must_use]
    pub fn version(&self, skill: SkillId, version: i32) -> Option<&SkillVersion> {
        self.entry(skill)?
            .versions
            .iter()
            .find(|row| row.version == version)
    }

    /// A scope project's entry; `None` for a project outside the scope.
    #[must_use]
    pub fn project(&self, project: ProjectId) -> Option<&ProjectSkills> {
        self.projects.iter().find(|entry| entry.project == project)
    }

    /// A scope workspace's entry; `None` for a workspace the view does not attach at.
    #[must_use]
    pub fn workspace(&self, workspace: WorkspaceId) -> Option<&WorkspaceSkills> {
        self.workspaces
            .iter()
            .find(|entry| entry.workspace == workspace)
    }

    /// One MCP server; `None` when none has that id.
    #[must_use]
    pub fn mcp_server(&self, id: McpServerId) -> Option<&McpServer> {
        self.mcp_servers.iter().find(|server| server.id == id)
    }

    /// The MCP server named `name`.
    #[must_use]
    pub fn mcp_by_name(&self, name: &str) -> Option<&McpServer> {
        self.mcp_servers.iter().find(|server| server.name == name)
    }

    /// The server a stored attachment row attaches; `None` for a skill's row. A row carries the
    /// target's id only (`Binding::skill_id`), and the two id spaces are disjoint UUIDv7s, so a
    /// row is a server's exactly when its id is a server's here.
    #[must_use]
    pub fn mcp_server_of(&self, row: &Binding) -> Option<&McpServer> {
        self.mcp_server(McpServerId::from_uuid(row.skill_id.as_uuid()))
    }

    /// The key a stored row sits under, of either kind (`BindingKey::of` reads every row as a
    /// skill's).
    fn key_of(&self, row: &Binding) -> BindingKey {
        let mut key = BindingKey::of(row);
        if let Some(server) = self.mcp_server_of(row) {
            key.target = BindingRef::McpServer(server.id);
        }
        key
    }

    /// The row at `key`: from that workspace's `bindings` when `key.workspace` is set, from
    /// [`global`](Self::global) when `key.project` is `None`, otherwise from that project's
    /// `bindings`. `None` when no row sits there, which is also the `expected: None` a new
    /// attachment passes.
    #[must_use]
    pub fn binding(&self, key: BindingKey) -> Option<&Binding> {
        let rows = match (key.workspace, key.project) {
            (Some(workspace), _) => &self.workspace(workspace)?.bindings,
            (None, None) => &self.global,
            (None, Some(project)) => &self.project(project)?.bindings,
        };
        rows.iter().find(|row| self.key_of(row) == key)
    }
}

/// Which write a [`StoreReply::SkillsStale`] answers (D81), so the view knows which draft keeps
/// its text and which row to reload against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleWhat {
    /// [`StoreRequest::EditSkill`]: the row changed since the form opened, or is gone.
    Skill(SkillId),
    /// [`StoreRequest::SaveSkillVersion`]: the head moved, or the skill is gone.
    Version(SkillId),
    /// [`StoreRequest::SetSkillBinding`]: the row at the key changed, appeared or went, or its
    /// skill, workspace, project, item kind or phase is gone.
    Binding(BindingKey),
    /// [`StoreRequest::EditMcpServer`] or [`StoreRequest::DeleteMcpServer`]: the server changed
    /// since the form opened, or is gone.
    McpServer(McpServerId),
}

/// One read (D81, D97), through the writer: `skill_bindings(BindingHome::Global)`; the scope
/// workspace's `workspace(w)` and `skill_bindings(BindingHome::Workspace(w))`; then per id of
/// `scope.project_ids`, in that order, `skill_bindings(BindingHome::Project(p))`, `item_kinds(p)`,
/// `step_graphs(p)` without the override graphs, `phases(g)` per graph and `repos(p)`'s names;
/// then `skills()` and
/// `skill_versions(s)` per skill.
///
/// `box_paths` is this box's `repo_box_path` rows, read by the caller once per request through
/// the `Backend` (never once per repo), empty when the box is unregistered: each project's
/// [`unrooted`](ProjectSkills::unrooted) is its repos with no row there (MOD-9 D126, D131).
///
/// Attachments are read **before** versions (the milestone 2 review's rule): a write landing
/// between the two can then only add a version no attachment names yet, never leave a pin naming
/// a version the read missed. N+1 reads on purpose, per event and never per keystroke (R-27,
/// R-35).
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(
    writer: &Writer,
    scope: &Scope,
    box_paths: &[RepoBoxPath],
) -> Result<SkillsSnapshot> {
    let global = writer.skill_bindings(BindingHome::Global).await?;
    let workspace = scope.workspace_id;
    let name = writer.workspace(workspace).await?.map_or_else(
        || workspace.to_string().chars().take(8).collect(),
        |row| row.name,
    );
    let workspaces = vec![WorkspaceSkills {
        workspace,
        name,
        bindings: writer
            .skill_bindings(BindingHome::Workspace(workspace))
            .await?,
    }];
    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for project in &scope.project_ids {
        let bindings = writer
            .skill_bindings(BindingHome::Project(*project))
            .await?;
        let item_kinds = writer.item_kinds(*project).await?;
        let mut graphs = Vec::new();
        for graph in writer.step_graphs(*project).await? {
            if graph.is_override {
                continue;
            }
            let phases = writer.phases(graph.id).await?;
            graphs.push((graph, phases));
        }
        let rows = writer.repos(*project).await?;
        // MOD-9 D126: the repos a `glob` attachment could never fire in on this box.
        let mut unrooted: Vec<String> = rows
            .iter()
            .filter(|repo| !box_paths.iter().any(|path| path.repo_id == repo.id))
            .map(|repo| repo.name.clone())
            .collect();
        unrooted.sort();
        let repos = rows.into_iter().map(|repo| repo.name).collect();
        projects.push(ProjectSkills {
            project: *project,
            bindings,
            item_kinds,
            graphs,
            repos,
            unrooted,
        });
    }
    let mut skills = Vec::new();
    for skill in writer.skills().await? {
        let versions = writer.skill_versions(skill.id).await?;
        skills.push(SkillEntry { skill, versions });
    }
    // After the attachments, as the skills are: a server created in between is attached by no
    // row read above, never the other way round.
    let mcp_servers = writer.mcp_servers().await?;
    Ok(SkillsSnapshot {
        skills,
        mcp_servers,
        global,
        workspaces,
        projects,
    })
}

/// The six request names, in [`StoreRequest`] order.
///
/// The view's `Failed` match reads from here. [`StoreRequest::name`]'s arms spell the same six as
/// literals, and the `request_names_match_the_name_arms` test pins them to this list, so a name
/// changed in one place and not the other fails there.
pub const REQUEST_NAMES: [&str; 6] = [
    "skills",
    "create_skill",
    "edit_skill",
    "save_skill_version",
    "set_skill_binding",
    "import_skills",
];

/// The **read**'s name: a refused read leaves the view with no library, where a refused write
/// leaves the form or editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// The three MCP server request names, in [`StoreRequest`] order (MOD-127 M2 T7). Apart from
/// [`REQUEST_NAMES`], which stays the skills six; `request_names_match_the_name_arms`'s sibling
/// pins them to [`StoreRequest::name`].
pub const MCP_REQUEST_NAMES: [&str; 3] =
    ["create_mcp_server", "edit_mcp_server", "delete_mcp_server"];

/// What a new MCP server is made of: [`NewMcpServer`] without the two fields the worker fills,
/// the id it mints and `created_by` (`R-NF-3`). The store's own refusals word the bad shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerDraft {
    /// `mcp_server.name`.
    pub name: String,
    /// `mcp_server.description`.
    pub description: String,
    /// `mcp_server.transport`.
    pub transport: McpTransport,
    /// `mcp_server.command` (`Stdio`).
    pub command: String,
    /// `mcp_server.args` (`Stdio`).
    pub args: Vec<String>,
    /// `mcp_server.url` (`Http`).
    pub url: Option<String>,
    /// `mcp_server.headers`: secrets by name ([`EnvValue::Secret`]), never by value.
    pub headers: Vec<(String, EnvValue)>,
    /// `mcp_server.env`: secrets by name, never by value.
    pub env: Vec<(String, EnvValue)>,
    /// `mcp_server.allow`.
    pub allow: Vec<String>,
    /// `mcp_server.deny`.
    pub deny: Vec<String>,
}

/// What one MCP server write did, carried by [`StoreReply::McpServerWritten`] beside the snapshot
/// re-read. Ids, names and counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerWrite {
    /// [`StoreRequest::CreateMcpServer`]: the server landed.
    Created {
        /// The id the worker minted.
        server: McpServerId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::EditMcpServer`]: the patch applied.
    Edited {
        /// Which server.
        server: McpServerId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::DeleteMcpServer`]: the server and its attachments are gone.
    Deleted {
        /// Which server.
        server: McpServerId,
        /// How many attachment rows went with it.
        bindings: u64,
    },
}

impl McpServerWrite {
    /// The [`StoreRequest::name`] of the write this answers (what the view's `busy` holds).
    #[must_use]
    pub const fn request_name(&self) -> &'static str {
        match self {
            Self::Created { .. } => MCP_REQUEST_NAMES[0],
            Self::Edited { .. } => MCP_REQUEST_NAMES[1],
            Self::Deleted { .. } => MCP_REQUEST_NAMES[2],
        }
    }
}

/// What one skill write did (MOD-59 D2), carried by [`StoreReply::SkillWritten`] beside the
/// snapshot re-read. Ids, names, keys and tokens only: a body stays in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillWrite {
    /// [`StoreRequest::CreateSkill`]: the skill and its version 1 landed.
    Created {
        /// The id the worker minted.
        skill: SkillId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::SaveSkillVersion`]: a version was appended.
    Versioned {
        /// Which skill.
        skill: SkillId,
        /// The version the store appended.
        version: i32,
    },
    /// [`StoreRequest::EditSkill`]: the rename or re-describe applied.
    Edited {
        /// Which skill.
        skill: SkillId,
        /// The name, as stored.
        name: String,
    },
    /// [`StoreRequest::SetSkillBinding`] with an attach: the row at `key` is the one sent.
    Attached {
        /// The attachment's key.
        key: BindingKey,
        /// The landed row's `updated_at`: the token a kept edit saves over (MOD-59 DV-1), not the
        /// re-read's row, which another session may already have written over.
        updated_at: DateTime<Utc>,
    },
    /// [`StoreRequest::SetSkillBinding`] with a detach: no row at `key`.
    Detached {
        /// The attachment's key.
        key: BindingKey,
    },
}

impl SkillWrite {
    /// The [`StoreRequest::name`] of the write this answers: what the view's `busy` holds while
    /// it is in flight, so only that write lands on it (MOD-59 D4).
    #[must_use]
    pub const fn request_name(&self) -> &'static str {
        match self {
            Self::Created { .. } => REQUEST_NAMES[1],
            Self::Edited { .. } => REQUEST_NAMES[2],
            Self::Versioned { .. } => REQUEST_NAMES[3],
            Self::Attached { .. } | Self::Detached { .. } => REQUEST_NAMES[4],
        }
    }
}

/// Serves one skill request, off the UI task (D97).
///
/// The read answers [`StoreReply::Skills`]. A write answers [`StoreReply::SkillWritten`] when it
/// applied, with the [`SkillWrite`] the store's own return names, even when only the re-read
/// after it failed (MOD-59 D1, D5); and [`StoreReply::SkillsStale`] when its token was spent or
/// the row it names is gone: `NotFound` of the skill for `EditSkill`; of the skill or
/// `skill_version` for `SaveSkillVersion`; of the skill, workspace, project, `item_kind` or `step_graph_phase` for
/// `SetSkillBinding`. Every other refusal stays an error, so [`crate::store_worker::serve`]
/// answers `Failed` with the store's sentence.
///
/// # Errors
/// Whatever the seam reports; offline, [`StoreError::Unreachable`] with `PROMPT_ON_SERVER_ONLY`
/// for the read (skills are not mirrored, the Templates read's sentence) and
/// `DATABASE_UNREACHABLE` for the four writes and the import; [`StoreError::Backend`] for a
/// request that is not one of this module's six.
///
/// The import answers [`StoreReply::SkillImports`]: the report of every file and a fresh snapshot.
/// A file that lost its token is a row of the report while the rest of the batch lands, so an
/// import never answers `SkillsStale`.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::Skills(scope) => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))?;
            let box_paths = this_box_paths(backend).await?;
            Ok(StoreReply::Skills(Box::new(
                snapshot(&writer, scope, &box_paths).await?,
            )))
        }
        StoreRequest::CreateSkill {
            scope,
            name,
            description,
            body,
        } => {
            let writer = write_access(backend)?;
            let created_by = backend.this_user().await?;
            // A refusal (a bad name, a blank body, a taken name) stays an error: there is no token
            // to have spent, so it is `Failed` with the store's sentence, never `SkillsStale`.
            let (skill, _) = writer
                .create_skill(NewSkill {
                    id: SkillId::new(),
                    name: name.clone(),
                    description: description.clone(),
                    body: body.as_str().to_owned(),
                    source: empty_source(),
                    created_by,
                })
                .await?;
            let landed = WriteOutcome::Applied(SkillWrite::Created {
                skill: skill.id,
                name: skill.name,
            });
            answer(backend, &writer, scope, landed).await
        }
        StoreRequest::EditSkill {
            scope,
            skill,
            expected,
            patch,
        } => {
            let writer = write_access(backend)?;
            let landed = match writer.update_skill(*skill, *expected, patch.clone()).await {
                Ok(CasOutcome::Applied(row)) => WriteOutcome::Applied(SkillWrite::Edited {
                    skill: row.id,
                    name: row.name,
                }),
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "skill", ..
                }) => WriteOutcome::Stale(StaleWhat::Skill(*skill)),
                Err(other) => return Err(other),
            };
            answer(backend, &writer, scope, landed).await
        }
        StoreRequest::SaveSkillVersion {
            scope,
            skill,
            expected,
            body,
        } => {
            let writer = write_access(backend)?;
            let created_by = backend.this_user().await?;
            let new = NewSkillVersion {
                body: body.as_str().to_owned(),
                source: empty_source(),
                created_by,
            };
            let landed = match writer.add_skill_version(*skill, *expected, new).await {
                Ok(CasOutcome::Applied(row)) => WriteOutcome::Applied(SkillWrite::Versioned {
                    skill: *skill,
                    version: row.version,
                }),
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "skill" | "skill_version",
                    ..
                }) => WriteOutcome::Stale(StaleWhat::Version(*skill)),
                Err(other) => return Err(other),
            };
            answer(backend, &writer, scope, landed).await
        }
        StoreRequest::SetSkillBinding {
            scope,
            key,
            expected,
            change,
        } => {
            let writer = write_access(backend)?;
            let landed = match writer
                .set_skill_binding(*key, *expected, change.clone())
                .await
            {
                Ok(CasOutcome::Applied(Some(row))) => WriteOutcome::Applied(SkillWrite::Attached {
                    key: *key,
                    updated_at: row.updated_at,
                }),
                Ok(CasOutcome::Applied(None)) => {
                    WriteOutcome::Applied(SkillWrite::Detached { key: *key })
                }
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity:
                        "skill" | "mcp_server" | "project" | "step_graph_phase" | "workspace"
                        | "item_kind",
                    ..
                }) => WriteOutcome::Stale(StaleWhat::Binding(*key)),
                Err(other) => return Err(other),
            };
            answer(backend, &writer, scope, landed).await
        }
        StoreRequest::CreateMcpServer { scope, draft } => {
            let writer = write_access(backend)?;
            let created_by = backend.this_user().await?;
            // As a skill create: a refusal has no token to have spent, so it is `Failed`.
            let server = writer
                .create_mcp_server(NewMcpServer {
                    id: McpServerId::new(),
                    name: draft.name.clone(),
                    description: draft.description.clone(),
                    transport: draft.transport,
                    command: draft.command.clone(),
                    args: draft.args.clone(),
                    url: draft.url.clone(),
                    headers: draft.headers.clone(),
                    env: draft.env.clone(),
                    allow: draft.allow.clone(),
                    deny: draft.deny.clone(),
                    created_by,
                })
                .await?;
            let landed = WriteOutcome::Applied(McpServerWrite::Created {
                server: server.id,
                name: server.name,
            });
            answer_mcp(backend, &writer, scope, landed).await
        }
        StoreRequest::EditMcpServer {
            scope,
            id,
            expected,
            patch,
        } => {
            let writer = write_access(backend)?;
            let landed = match writer
                .update_mcp_server(*id, *expected, (**patch).clone())
                .await
            {
                Ok(CasOutcome::Applied(row)) => WriteOutcome::Applied(McpServerWrite::Edited {
                    server: row.id,
                    name: row.name,
                }),
                Ok(CasOutcome::Stale(_))
                | Err(StoreError::NotFound {
                    entity: "mcp_server",
                    ..
                }) => WriteOutcome::Stale(StaleWhat::McpServer(*id)),
                Err(other) => return Err(other),
            };
            answer_mcp(backend, &writer, scope, landed).await
        }
        StoreRequest::DeleteMcpServer { scope, id } => {
            let writer = write_access(backend)?;
            let landed = match writer.delete_mcp_server(*id).await {
                Ok(bindings) => WriteOutcome::Applied(McpServerWrite::Deleted {
                    server: *id,
                    bindings,
                }),
                Err(StoreError::NotFound {
                    entity: "mcp_server",
                    ..
                }) => WriteOutcome::Stale(StaleWhat::McpServer(*id)),
                Err(other) => return Err(other),
            };
            answer_mcp(backend, &writer, scope, landed).await
        }
        StoreRequest::ImportSkills { scope, paths } => {
            // The filesystem is read here and not in the view (`R-NF-3`): the walk is unbounded
            // work and the UI task must not carry it. The reply re-reads the whole snapshot, so the
            // report travels beside a library the view renders from rather than a row it patches.
            let report = skill_import::import(backend, paths).await?;
            let writer = write_access(backend)?;
            let box_paths = this_box_paths(backend).await?;
            Ok(StoreReply::SkillImports(Box::new(SkillImports {
                snapshot: snapshot(&writer, scope, &box_paths).await?,
                report,
            })))
        }
        // `try_serve` routes exactly this module's six variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a skills request: {}",
            other.name()
        ))),
    }
}

/// The writer, or the refusal every skill write answers offline.
fn write_access(backend: &Backend) -> Result<Writer> {
    backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))
}

/// This box's `repo_box_path` rows, or none when the box is unregistered (every repo then
/// unrooted). Read after the writer check, so an offline arm still refuses with its own sentence
/// first (the `hierarchy::serve` order; MOD-9 D131).
async fn this_box_paths(backend: &Backend) -> Result<Vec<RepoBoxPath>> {
    match backend.box_info().await? {
        Some(info) => backend.repo_paths(info.box_id).await,
        None => Ok(Vec::new()),
    }
}

/// `skill_version.source` for a version written from the view: `{}` (D77, D87).
fn empty_source() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// The re-read after a write (MOD-59 D1, D5): `SkillWritten` naming what `landed` wrote, or
/// `SkillsStale` naming the write that missed. A re-read that fails after an applied write still
/// answers `SkillWritten`, because the write landed; after a stale one it stays an error (D3).
async fn answer(
    backend: &Backend,
    writer: &Writer,
    scope: &Scope,
    landed: WriteOutcome<SkillWrite, StaleWhat>,
) -> Result<StoreReply> {
    let fresh = reread(backend, writer, scope).await;
    landed.answer(fresh, written, |snapshot, what| StoreReply::SkillsStale {
        snapshot: Box::new(snapshot),
        what,
    })
}

/// [`answer`] for an MCP server write: `McpServerWritten`, or `SkillsStale` naming the server.
async fn answer_mcp(
    backend: &Backend,
    writer: &Writer,
    scope: &Scope,
    landed: WriteOutcome<McpServerWrite, StaleWhat>,
) -> Result<StoreReply> {
    let fresh = reread(backend, writer, scope).await;
    landed.answer(
        fresh,
        |snapshot, outcome| StoreReply::McpServerWritten {
            snapshot: snapshot.map(Box::new).map_err(|err| err.to_string()),
            outcome,
        },
        |snapshot, what| StoreReply::SkillsStale {
            snapshot: Box::new(snapshot),
            what,
        },
    )
}

/// The snapshot after a write. Its callers are past `write_access`, so this box's paths are read
/// here (MOD-9 D131).
async fn reread(backend: &Backend, writer: &Writer, scope: &Scope) -> Result<SkillsSnapshot> {
    let box_paths = this_box_paths(backend).await?;
    snapshot(writer, scope, &box_paths).await
}

/// MOD-59 D5: the reply to a write that applied, whatever its re-read came to. A failed re-read
/// travels as its `StoreError` rendered through `Display`, the sentence `Failed` would carry.
/// An unreachable re-read is then no `Err` to the worker loop: it reaches neither its `go_offline`
/// nor the status line, and the refresher's health pass takes the backend offline instead.
fn written(reread: Result<SkillsSnapshot>, outcome: SkillWrite) -> StoreReply {
    StoreReply::SkillWritten {
        snapshot: reread.map(Box::new).map_err(|err| err.to_string()),
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MCP_REQUEST_NAMES, McpServerDraft, McpServerWrite, READ_NAME, REQUEST_NAMES, SkillWrite,
        SkillsSnapshot, StaleWhat, serve, written,
    };
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use crate::templates::TemplateBody;
    use chrono::{DateTime, Duration, Utc};
    use htui_core::fixtures::{self, ids};
    use htui_core::model::{
        Activation, Attachment, BindingChange, BindingKey, EnvValue, McpServerId, McpServerPatch,
        McpTransport, NewRepo, PhaseId, RepoBoxPath, RepoId, Scope, SkillId, SkillPatch, StepGraph,
        StepGraphId,
    };
    use htui_core::store::{MemStore, StoreError, WriteStore as _};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY};

    fn demo() -> Backend {
        Backend::memory(MemStore::demo())
    }

    /// The Platform workspace's scope: `htui` then `agy`, by `workspace_project.position`.
    async fn platform_scope(backend: &Backend) -> Scope {
        let StoreReply::Workspaces(workspaces) =
            store_worker::serve(backend, &StoreRequest::Workspaces).await
        else {
            panic!("workspaces answered with the wrong variant")
        };
        let platform = workspaces
            .iter()
            .find(|w| w.slug == "platform")
            .expect("the demo fixture holds the `platform` workspace");
        Scope::from_workspace(platform)
    }

    async fn read(backend: &Backend, scope: &Scope) -> SkillsSnapshot {
        match serve(backend, &StoreRequest::Skills(scope.clone())).await {
            Ok(StoreReply::Skills(snapshot)) => *snapshot,
            other => panic!("the read answered {other:?}"),
        }
    }

    /// The snapshot and outcome of a `SkillWritten` reply whose re-read succeeded, or a panic
    /// naming what came back instead (MOD-59 D1).
    async fn applied(backend: &Backend, request: &StoreRequest) -> (SkillsSnapshot, SkillWrite) {
        match serve(backend, request).await {
            Ok(StoreReply::SkillWritten {
                snapshot: Ok(snapshot),
                outcome,
            }) => (*snapshot, outcome),
            other => panic!("{} answered {other:?}", request.name()),
        }
    }

    /// The snapshot and `what` of a `SkillsStale` reply, or a panic naming what came back.
    async fn stale(backend: &Backend, request: &StoreRequest) -> (SkillsSnapshot, StaleWhat) {
        match serve(backend, request).await {
            Ok(StoreReply::SkillsStale { snapshot, what }) => (*snapshot, what),
            other => panic!("{} answered {other:?}", request.name()),
        }
    }

    fn always() -> Attachment {
        Attachment {
            pinned_version: None,
            position: 0,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
        }
    }

    fn import(scope: &Scope, path: &str) -> StoreRequest {
        StoreRequest::ImportSkills {
            scope: scope.clone(),
            paths: vec![path.to_owned()],
        }
    }

    fn create(scope: &Scope, name: &str, body: &str) -> StoreRequest {
        StoreRequest::CreateSkill {
            scope: scope.clone(),
            name: name.to_owned(),
            description: format!("{name}, briefly"),
            body: TemplateBody::new(body),
        }
    }

    fn edit(
        scope: &Scope,
        skill: SkillId,
        expected: DateTime<Utc>,
        description: &str,
    ) -> StoreRequest {
        StoreRequest::EditSkill {
            scope: scope.clone(),
            skill,
            expected,
            patch: SkillPatch {
                name: None,
                description: Some(description.to_owned()),
            },
        }
    }

    fn save(scope: &Scope, skill: SkillId, expected: i32, body: &str) -> StoreRequest {
        StoreRequest::SaveSkillVersion {
            scope: scope.clone(),
            skill,
            expected,
            body: TemplateBody::new(body),
        }
    }

    fn bind(
        scope: &Scope,
        key: BindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> StoreRequest {
        StoreRequest::SetSkillBinding {
            scope: scope.clone(),
            key,
            expected,
            change,
        }
    }

    /// The demo's phase-level attachment: `rust-style` on `htui`'s `implement` phase.
    fn demo_phase_key() -> BindingKey {
        BindingKey::phase(
            ids::SKILL_RUST_STYLE,
            ids::PROJECT_HTUI,
            ids::PHASE_HTUI_IMPLEMENT,
        )
    }

    /// F-K: the demo with one override graph on `htui` and repo `core`, so the snapshot has an
    /// override to leave out and a repo name to carry.
    #[tokio::test]
    async fn the_snapshot_carries_the_library_the_attachments_and_the_scope_s_graphs() {
        let mut data = fixtures::demo_data();
        data.graphs.push(StepGraph {
            id: StepGraphId::new(),
            scope: htui_core::model::GraphScope::Project(ids::PROJECT_HTUI),
            name: "FEAT-1-override".to_owned(),
            description: String::new(),
            is_override: true,
            revision: 0,
            target_id: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
        });
        let store = MemStore::from_demo(data);
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo takes a repo on `htui`");
        let backend = Backend::memory(store);
        let scope = platform_scope(&backend).await;
        assert_eq!(scope.project_ids, vec![ids::PROJECT_HTUI, ids::PROJECT_AGY]);

        let snapshot = read(&backend, &scope).await;

        let library: Vec<(&str, Vec<i32>)> = snapshot
            .skills
            .iter()
            .map(|entry| {
                (
                    entry.skill.name.as_str(),
                    entry.versions.iter().map(|row| row.version).collect(),
                )
            })
            .collect();
        assert_eq!(
            library,
            vec![("rust-style", vec![1, 2]), ("tests", vec![1])],
            "the library in name order, each with every version ascending"
        );
        assert!(
            snapshot.global.is_empty(),
            "the demo attaches nothing globally"
        );
        assert_eq!(
            snapshot
                .projects
                .iter()
                .map(|p| p.project)
                .collect::<Vec<_>>(),
            scope.project_ids,
            "one entry per scope project, in scope order"
        );

        let htui = snapshot
            .project(ids::PROJECT_HTUI)
            .expect("htui is in the scope");
        assert_eq!(
            htui.bindings.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![
                ids::BINDING_HTUI_RUST_STYLE,
                ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE,
                ids::BINDING_HTUI_TESTS,
            ],
            "the three demo rows, (skill_id, phase_id) order with the project row first (D91)"
        );
        let graphs: Vec<&str> = htui
            .graphs
            .iter()
            .map(|(graph, _)| graph.name.as_str())
            .collect();
        assert_eq!(
            graphs,
            ["analysis", "bug", "feature", "refactor", "tooling"],
            "name order, the override graph left out"
        );
        assert_eq!(
            htui.graphs
                .iter()
                .map(|(_, phases)| phases.len())
                .sum::<usize>(),
            15
        );
        for (graph, phases) in &htui.graphs {
            assert!(phases.iter().all(|phase| phase.graph_id == graph.id));
            assert!(
                phases.windows(2).all(|w| w[0].position < w[1].position),
                "{}'s phases by position",
                graph.name
            );
        }
        assert_eq!(htui.repos, ["core"]);

        let agy = snapshot
            .project(ids::PROJECT_AGY)
            .expect("agy is in the scope");
        assert!(agy.bindings.is_empty());
        assert!(agy.repos.is_empty());

        let rust_style = snapshot
            .by_name("rust-style")
            .expect("the demo holds rust-style");
        assert_eq!(rust_style.skill.id, ids::SKILL_RUST_STYLE);
        assert_eq!(snapshot.entry(ids::SKILL_RUST_STYLE), Some(rust_style));
        assert_eq!(
            snapshot.head(ids::SKILL_RUST_STYLE).map(|row| row.version),
            Some(2)
        );
        assert_eq!(
            snapshot
                .version(ids::SKILL_RUST_STYLE, 1)
                .map(|row| row.version),
            Some(1)
        );
        assert_eq!(snapshot.head(SkillId::new()), None);
        assert_eq!(
            snapshot.binding(demo_phase_key()).map(|row| row.id),
            Some(ids::BINDING_HTUI_IMPLEMENT_RUST_STYLE)
        );
        assert_eq!(
            snapshot.binding(BindingKey {
                project: None,
                phase: None,
                ..demo_phase_key()
            }),
            None,
            "no global row"
        );
        assert_eq!(snapshot.project(ids::PROJECT_VULKAN), None);
    }

    /// MOD-127 M1 T4: the snapshot holds the scope workspace's attachments and each project's
    /// item kinds, and `binding` finds a row at either level by its full key; a workspace
    /// attachment through the request answers `SkillWritten`, one onto a vanished workspace or
    /// item kind `SkillsStale`.
    #[tokio::test]
    async fn the_snapshot_carries_workspace_rows_and_item_kinds() {
        let store = MemStore::demo();
        let kinds = store
            .item_kinds(ids::PROJECT_HTUI)
            .await
            .expect("htui's kinds");
        let kind = kinds.first().expect("the demo gives htui a kind").clone();
        let backend = Backend::memory(store);
        let scope = platform_scope(&backend).await;

        let before = read(&backend, &scope).await;
        assert_eq!(
            before.workspaces.len(),
            1,
            "one entry, the scope's workspace"
        );
        assert_eq!(before.workspaces[0].workspace, scope.workspace_id);
        assert_eq!(before.workspaces[0].name, "Platform");
        assert!(before.workspaces[0].bindings.is_empty());
        let htui = before.project(ids::PROJECT_HTUI).expect("htui");
        assert_eq!(
            htui.item_kinds.iter().map(|k| k.id).collect::<Vec<_>>(),
            kinds.iter().map(|k| k.id).collect::<Vec<_>>(),
            "the project's kinds, the store's order"
        );

        let ws_key = BindingKey::workspace(ids::SKILL_TESTS, scope.workspace_id);
        let (after, wrote) = applied(
            &backend,
            &bind(&scope, ws_key, None, BindingChange::Attach(always())),
        )
        .await;
        assert!(matches!(wrote, SkillWrite::Attached { key, .. } if key == ws_key));
        assert_eq!(
            after.binding(ws_key).map(|row| row.workspace_id),
            Some(Some(scope.workspace_id)),
            "a workspace row is found by its key, in the workspace's entry"
        );
        assert!(after.global.is_empty(), "and is not a global row");

        let kind_key = BindingKey::item_kind(ids::SKILL_TESTS, ids::PROJECT_HTUI, kind.id);
        let (after, _) = applied(
            &backend,
            &bind(&scope, kind_key, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(
            after.binding(kind_key).map(|row| row.item_kind_id),
            Some(Some(kind.id)),
            "an item-kind row is found in its project's rows"
        );
        assert_eq!(
            after
                .binding(BindingKey::project(ids::SKILL_TESTS, ids::PROJECT_HTUI))
                .map(|row| (row.id, row.item_kind_id)),
            Some((ids::BINDING_HTUI_TESTS, None)),
            "the demo's project row is another key, untouched"
        );

        let gone = BindingKey::workspace(ids::SKILL_TESTS, htui_core::model::WorkspaceId::new());
        let (_, what) = stale(
            &backend,
            &bind(&scope, gone, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(what, StaleWhat::Binding(gone), "a vanished workspace");
        let gone = BindingKey::item_kind(
            ids::SKILL_TESTS,
            ids::PROJECT_HTUI,
            htui_core::model::ItemKindId::new(),
        );
        let (_, what) = stale(
            &backend,
            &bind(&scope, gone, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(what, StaleWhat::Binding(gone), "a vanished item kind");
    }

    /// MOD-9 D126: the snapshot names the repos this box has no checkout path for, so the
    /// attachments pane can warn on a `glob` row that could never fire here.
    #[tokio::test]
    async fn the_snapshot_lists_repos_with_no_path_on_this_box() {
        let store = MemStore::demo();
        let rooted = RepoId::new();
        for (id, name, is_primary) in [(rooted, "htui", true), (RepoId::new(), "web", false)] {
            store
                .create_repo(NewRepo {
                    id,
                    project_id: ids::PROJECT_HTUI,
                    name: name.to_owned(),
                    remote_url: None,
                    default_branch: "main".to_owned(),
                    is_primary,
                })
                .await
                .expect("the demo takes a repo on `htui`");
        }
        store
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: rooted,
                box_id: ids::BOX,
                local_path: "/src/htui".to_owned(),
                updated_at: DateTime::UNIX_EPOCH,
            })
            .await
            .expect("the path write");
        let backend = Backend::memory(store);
        let scope = platform_scope(&backend).await;

        let snapshot = read(&backend, &scope).await;

        let htui = snapshot
            .project(ids::PROJECT_HTUI)
            .expect("htui is in the scope");
        assert_eq!(htui.repos, ["htui", "web"]);
        assert_eq!(htui.unrooted, ["web"], "only `web` has no path on this box");
        let agy = snapshot
            .project(ids::PROJECT_AGY)
            .expect("agy is in the scope");
        assert!(agy.unrooted.is_empty(), "agy has no repo to be unrooted");
    }

    /// MOD-59 D1, D2: every write that applied answers `SkillWritten`, naming what the store
    /// stored, beside the snapshot re-read after it.
    #[tokio::test]
    async fn each_write_answers_skill_written_with_what_the_store_stored() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let (created, outcome) = applied(
            &backend,
            &create(&scope, "docs-style", "Write the doc first.\n"),
        )
        .await;
        let entry = created
            .by_name("docs-style")
            .expect("the created skill is in the fresh snapshot")
            .clone();
        assert_eq!(
            outcome,
            SkillWrite::Created {
                skill: entry.skill.id,
                name: "docs-style".to_owned(),
            },
            "the id the worker minted"
        );
        assert_eq!(
            entry
                .versions
                .iter()
                .map(|row| (row.version, row.body.as_str()))
                .collect::<Vec<_>>(),
            [(1, "Write the doc first.\n")]
        );
        assert_eq!(
            entry.skill.created_by,
            ids::USER,
            "the worker fills `created_by`"
        );
        assert_eq!(entry.versions[0].created_by, ids::USER);
        assert_eq!(entry.versions[0].source, serde_json::json!({}));
        let id = entry.skill.id;

        let (edited, outcome) = applied(
            &backend,
            &edit(&scope, id, entry.skill.updated_at, "Docs, then code."),
        )
        .await;
        assert_eq!(
            edited.entry(id).map(|e| e.skill.description.as_str()),
            Some("Docs, then code.")
        );
        assert_eq!(
            outcome,
            SkillWrite::Edited {
                skill: id,
                name: "docs-style".to_owned(),
            }
        );

        let (saved, outcome) = applied(
            &backend,
            &save(&scope, id, 1, "Write the doc first, always.\n"),
        )
        .await;
        let head = saved.head(id).expect("the skill has a head");
        assert_eq!(head.version, 2);
        assert_eq!(head.body, "Write the doc first, always.\n");
        assert_eq!(head.created_by, ids::USER);
        assert_eq!(
            outcome,
            SkillWrite::Versioned {
                skill: id,
                version: 2,
            }
        );

        let key = BindingKey::global(id);
        let (attached, outcome) = applied(
            &backend,
            &bind(&scope, key, None, BindingChange::Attach(always())),
        )
        .await;
        let row = attached
            .binding(key)
            .expect("the global attachment is in the fresh snapshot")
            .clone();
        assert_eq!(attached.global, std::slice::from_ref(&row));
        assert_eq!(
            outcome,
            SkillWrite::Attached {
                key,
                updated_at: row.updated_at,
            },
            "the landed row's token (MOD-59 DV-1)"
        );

        let (detached, outcome) = applied(
            &backend,
            &bind(&scope, key, Some(row.updated_at), BindingChange::Detach),
        )
        .await;
        assert!(detached.global.is_empty(), "the detach re-reads without it");
        assert_eq!(outcome, SkillWrite::Detached { key });
    }

    #[tokio::test]
    async fn a_spent_token_answers_skills_stale_with_what_went_stale() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;
        let rust_style = before
            .entry(ids::SKILL_RUST_STYLE)
            .expect("the demo holds rust-style")
            .skill
            .clone();

        let (fresh, what) = stale(
            &backend,
            &edit(
                &scope,
                rust_style.id,
                rust_style.updated_at - Duration::seconds(1),
                "never stored",
            ),
        )
        .await;
        assert_eq!(what, StaleWhat::Skill(rust_style.id));
        assert_eq!(fresh, before, "a stale edit writes nothing");

        let (fresh, what) =
            stale(&backend, &save(&scope, rust_style.id, 1, "never stored\n")).await;
        assert_eq!(what, StaleWhat::Version(rust_style.id), "the head is 2");
        assert_eq!(fresh, before);

        let key = demo_phase_key();
        let (fresh, what) = stale(
            &backend,
            &bind(&scope, key, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(
            what,
            StaleWhat::Binding(key),
            "a row already sits at the key"
        );
        assert_eq!(fresh, before);

        let gone = SkillId::new();
        let (fresh, what) = stale(&backend, &edit(&scope, gone, rust_style.updated_at, "x")).await;
        assert_eq!(
            what,
            StaleWhat::Skill(gone),
            "a skill gone is a miss like a spent token"
        );
        assert_eq!(fresh, before);

        let (fresh, what) = stale(&backend, &save(&scope, gone, 0, "never stored\n")).await;
        assert_eq!(what, StaleWhat::Version(gone));
        assert_eq!(fresh, before);

        let no_phase = BindingKey {
            phase: Some(PhaseId::new()),
            ..key
        };
        let (fresh, what) = stale(
            &backend,
            &bind(&scope, no_phase, None, BindingChange::Attach(always())),
        )
        .await;
        assert_eq!(what, StaleWhat::Binding(no_phase), "a phase gone is a miss");
        assert_eq!(fresh, before);
    }

    /// MOD-59 D5: a re-read that fails after a write applied still answers the write's own
    /// variant, the failure rendered as `Failed` would render it. `MemStore` cannot fail a read
    /// inside one `serve`, so the mapping is pinned here.
    #[test]
    fn a_reread_that_fails_after_an_applied_write_still_names_the_write() {
        let outcome = SkillWrite::Versioned {
            skill: ids::SKILL_TESTS,
            version: 2,
        };
        let reply = written(
            Err(StoreError::Unreachable("gone".to_owned())),
            outcome.clone(),
        );
        match reply {
            StoreReply::SkillWritten {
                snapshot: Err(message),
                outcome: named,
            } => {
                assert_eq!(message, "store unreachable: gone");
                assert_eq!(named, outcome);
            }
            other => panic!("a landed write answers `SkillWritten`, not {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_refused_write_answers_failed_with_the_store_sentence() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let before = read(&backend, &scope).await;

        let reply = store_worker::serve(&backend, &create(&scope, "Bad", "A body.\n")).await;

        let StoreReply::Failed { request, message } = reply else {
            panic!("a refused name answers `Failed`, got {reply:?}")
        };
        assert_eq!(request, "create_skill");
        assert!(
            message.contains("skill.name"),
            "the store's sentence reaches the view: {message}"
        );
        assert_eq!(read(&backend, &scope).await, before, "nothing is written");
    }

    async fn offline() -> (tempfile::TempDir, Backend) {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "skills-offline", 1)
            .await
            .expect("open a throwaway mirror");
        (root, Backend::Offline { cache, since: None })
    }

    fn htui_scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        }
    }

    /// Skills are not mirrored, so offline there is nothing to read: the Templates read's
    /// sentence, and `Failed` for `skills` through the worker.
    #[tokio::test]
    async fn an_offline_read_is_refused_with_the_server_only_sentence() {
        let (_root, backend) = offline().await;
        let request = StoreRequest::Skills(htui_scope());

        assert_eq!(
            serve(&backend, &request).await.err(),
            Some(StoreError::Unreachable(PROMPT_ON_SERVER_ONLY.to_owned()))
        );
        match store_worker::serve(&backend, &request).await {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, READ_NAME);
                assert!(message.contains(PROMPT_ON_SERVER_ONLY), "{message}");
            }
            other => panic!("an offline read is refused, not {other:?}"),
        }
    }

    /// Offline there is no writer, so each of the four writes and the import is refused before
    /// anything is sent, with the unreachable-database sentence, under its own name.
    #[tokio::test]
    async fn an_offline_write_is_refused_with_the_unreachable_sentence() {
        let (_root, backend) = offline().await;
        let scope = htui_scope();
        let writes = [
            create(&scope, "docs-style", "A body.\n"),
            edit(&scope, ids::SKILL_RUST_STYLE, DateTime::UNIX_EPOCH, "x"),
            save(&scope, ids::SKILL_RUST_STYLE, 2, "A body.\n"),
            bind(
                &scope,
                demo_phase_key(),
                Some(DateTime::UNIX_EPOCH),
                BindingChange::Detach,
            ),
            import(&scope, "/nowhere"),
        ];
        assert_eq!(writes.len(), REQUEST_NAMES.len() - 1, "every write is here");

        for (write, name) in writes.iter().zip(&REQUEST_NAMES[1..]) {
            assert_eq!(
                serve(&backend, write).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{name}"
            );
            match store_worker::serve(&backend, write).await {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, *name);
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {name} is refused, not {other:?}"),
            }
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let scope = htui_scope();
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
        let samples = [
            StoreRequest::Skills(scope.clone()),
            create(&scope, "docs-style", ""),
            edit(&scope, ids::SKILL_TESTS, DateTime::UNIX_EPOCH, ""),
            save(&scope, ids::SKILL_TESTS, 1, ""),
            bind(&scope, demo_phase_key(), None, BindingChange::Detach),
            import(&scope, ""),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);

        // MOD-59 D4: each outcome names the write it answers, the name the view's `busy` holds.
        let key = demo_phase_key();
        let outcomes = [
            (
                SkillWrite::Created {
                    skill: ids::SKILL_TESTS,
                    name: "docs-style".to_owned(),
                },
                &samples[1],
            ),
            (
                SkillWrite::Edited {
                    skill: ids::SKILL_TESTS,
                    name: "tests".to_owned(),
                },
                &samples[2],
            ),
            (
                SkillWrite::Versioned {
                    skill: ids::SKILL_TESTS,
                    version: 2,
                },
                &samples[3],
            ),
            (
                SkillWrite::Attached {
                    key,
                    updated_at: DateTime::UNIX_EPOCH,
                },
                &samples[4],
            ),
            (SkillWrite::Detached { key }, &samples[4]),
        ];
        for (outcome, request) in &outcomes {
            assert_eq!(outcome.request_name(), request.name(), "{outcome:?}");
        }
    }

    /// `StoreRequest` derives `Debug`; a skill body is user text, so a create and a save print
    /// its length only (`TemplateBody`'s rule).
    #[test]
    fn a_skill_body_request_debug_prints_its_length_not_its_text() {
        let scope = htui_scope();
        for request in [
            save(&scope, ids::SKILL_TESTS, 1, "secret rule"),
            create(&scope, "docs-style", "secret rule"),
        ] {
            let shown = format!("{request:?}");
            assert!(!shown.contains("secret"), "{shown}");
            assert!(shown.contains("len: 11"), "{shown}");
        }
    }

    /// `try_serve` routes the import here, and the reply is the report beside a fresh snapshot of
    /// the request's own scope, so the view can tell a crossed scope change as it does for
    /// `Skills`.
    #[tokio::test]
    async fn an_import_is_routed_here_and_answers_its_report_beside_a_snapshot() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let dir = tempfile::tempdir().expect("a temp dir");
        let nested = dir.path().join("docs-style");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(
            nested.join("SKILL.md"),
            "---\nname: docs-style\ndescription: Docs.\n---\n# Docs\n",
        )
        .expect("write");

        let reply =
            store_worker::serve(&backend, &import(&scope, &dir.path().display().to_string())).await;

        let StoreReply::SkillImports(imports) = reply else {
            panic!("`import_skills` answers `SkillImports`, not {reply:?}")
        };
        assert!(
            matches!(
                &imports.report[..],
                [crate::skill_import::ImportOutcome::Imported { name, version: 1, .. }]
                    if name == "docs-style"
            ),
            "{:?}",
            imports.report
        );
        assert!(
            imports.snapshot.by_name("docs-style").is_some(),
            "the snapshot is read after the writes"
        );
        assert_eq!(
            imports
                .snapshot
                .projects
                .iter()
                .map(|entry| entry.project)
                .collect::<Vec<_>>(),
            scope.project_ids,
            "and of the request's scope"
        );
    }

    fn mcp_draft(name: &str) -> McpServerDraft {
        McpServerDraft {
            name: name.to_owned(),
            description: format!("{name}, briefly"),
            transport: McpTransport::Stdio,
            command: "mcp-docs".to_owned(),
            args: vec!["--stdio".to_owned()],
            url: None,
            headers: Vec::new(),
            env: vec![("DOCS_TOKEN".to_owned(), EnvValue::Secret("docs".to_owned()))],
            allow: vec!["search".to_owned()],
            deny: Vec::new(),
        }
    }

    fn create_mcp(scope: &Scope, name: &str) -> StoreRequest {
        StoreRequest::CreateMcpServer {
            scope: scope.clone(),
            draft: Box::new(mcp_draft(name)),
        }
    }

    fn edit_mcp(
        scope: &Scope,
        id: McpServerId,
        expected: DateTime<Utc>,
        description: &str,
    ) -> StoreRequest {
        StoreRequest::EditMcpServer {
            scope: scope.clone(),
            id,
            expected,
            patch: Box::new(McpServerPatch {
                description: Some(description.to_owned()),
                ..McpServerPatch::default()
            }),
        }
    }

    fn delete_mcp(scope: &Scope, id: McpServerId) -> StoreRequest {
        StoreRequest::DeleteMcpServer {
            scope: scope.clone(),
            id,
        }
    }

    /// The snapshot and outcome of a `McpServerWritten` reply whose re-read succeeded.
    async fn applied_mcp(
        backend: &Backend,
        request: &StoreRequest,
    ) -> (SkillsSnapshot, McpServerWrite) {
        match serve(backend, request).await {
            Ok(StoreReply::McpServerWritten {
                snapshot: Ok(snapshot),
                outcome,
            }) => (*snapshot, outcome),
            other => panic!("{} answered {other:?}", request.name()),
        }
    }

    #[tokio::test]
    async fn create_edit_delete_an_mcp_server() {
        let backend = demo();
        let scope = platform_scope(&backend).await;

        let (snapshot, outcome) = applied_mcp(&backend, &create_mcp(&scope, "docs")).await;
        let McpServerWrite::Created { server, name } = outcome else {
            panic!("a create answers `Created`, not {outcome:?}")
        };
        assert_eq!(name, "docs");
        let row = snapshot.mcp_server(server).expect("the re-read holds it");
        assert_eq!(row.command, "mcp-docs");
        assert_eq!(
            row.created_by,
            backend.this_user().await.expect("this user"),
            "the worker fills `created_by`"
        );
        assert_eq!(snapshot.mcp_by_name("docs").map(|row| row.id), Some(server));

        let token = row.updated_at;
        let (snapshot, outcome) =
            applied_mcp(&backend, &edit_mcp(&scope, server, token, "Docs lookup")).await;
        assert_eq!(
            outcome,
            McpServerWrite::Edited {
                server,
                name: "docs".to_owned()
            }
        );
        let row = snapshot.mcp_server(server).expect("still there");
        assert_eq!(row.description, "Docs lookup");
        assert!(row.updated_at > token, "the store stamped the edit");

        let (snapshot, outcome) = applied_mcp(&backend, &delete_mcp(&scope, server)).await;
        assert_eq!(
            outcome,
            McpServerWrite::Deleted {
                server,
                bindings: 0
            }
        );
        assert!(snapshot.mcp_servers.is_empty());
    }

    /// A refusal that is no lost token (a bad name, a taken name) stays an error: `Failed`.
    #[tokio::test]
    async fn an_mcp_refusal_is_an_error_not_a_stale() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        applied_mcp(&backend, &create_mcp(&scope, "docs")).await;

        for bad in ["docs", "Docs", "htui"] {
            let err = serve(&backend, &create_mcp(&scope, bad))
                .await
                .expect_err("refused");
            assert!(matches!(err, StoreError::Constraint(_)), "{bad}: {err:?}");
        }
    }

    #[tokio::test]
    async fn a_stale_edit_answers_stale() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        let (snapshot, outcome) = applied_mcp(&backend, &create_mcp(&scope, "docs")).await;
        let McpServerWrite::Created { server, .. } = outcome else {
            panic!("{outcome:?}")
        };
        let token = snapshot.mcp_server(server).expect("created").updated_at;
        applied_mcp(&backend, &edit_mcp(&scope, server, token, "first")).await;

        // The spent token: stale, the snapshot is the fresh one, the other writer's text stands.
        let (snapshot, what) = stale(&backend, &edit_mcp(&scope, server, token, "second")).await;
        assert_eq!(what, StaleWhat::McpServer(server));
        assert_eq!(
            snapshot.mcp_server(server).expect("kept").description,
            "first"
        );

        // A server that is gone: edit and delete both answer stale for it.
        let gone = McpServerId::new();
        for request in [edit_mcp(&scope, gone, token, "x"), delete_mcp(&scope, gone)] {
            let (_, what) = stale(&backend, &request).await;
            assert_eq!(what, StaleWhat::McpServer(gone), "{}", request.name());
        }
    }

    #[tokio::test]
    async fn the_snapshot_lists_servers_and_their_bindings() {
        let backend = demo();
        let scope = platform_scope(&backend).await;
        assert!(
            read(&backend, &scope).await.mcp_servers.is_empty(),
            "the demo holds no server"
        );
        let (_, outcome) = applied_mcp(&backend, &create_mcp(&scope, "docs")).await;
        let McpServerWrite::Created { server, .. } = outcome else {
            panic!("{outcome:?}")
        };
        applied_mcp(&backend, &create_mcp(&scope, "build")).await;

        let key = BindingKey::global(server);
        let (snapshot, _) = applied(
            &backend,
            &bind(&scope, key, None, BindingChange::Attach(always())),
        )
        .await;

        let names: Vec<&str> = snapshot
            .mcp_servers
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(names, ["build", "docs"], "name byte order");
        let row = snapshot.binding(key).expect("the global binding is listed");
        assert_eq!(row.activation, Activation::Always);
        assert!(
            snapshot.mcp_server_of(row).is_some_and(|s| s.id == server),
            "the row says which server it attaches"
        );
        assert!(
            snapshot
                .binding(BindingKey::global(ids::SKILL_TESTS))
                .is_none(),
            "a skill key does not find the server's row"
        );
        assert_eq!(snapshot.global.len(), 1);

        // Deleting the server takes its bindings with it, and says how many.
        let (snapshot, outcome) = applied_mcp(&backend, &delete_mcp(&scope, server)).await;
        assert_eq!(
            outcome,
            McpServerWrite::Deleted {
                server,
                bindings: 1
            }
        );
        assert!(snapshot.global.is_empty());
    }

    #[tokio::test]
    async fn offline_mcp_writes_are_refused() {
        let (_dir, backend) = offline().await;
        let scope = htui_scope();
        let id = McpServerId::new();
        for write in [
            create_mcp(&scope, "docs"),
            edit_mcp(&scope, id, DateTime::UNIX_EPOCH, "x"),
            delete_mcp(&scope, id),
        ] {
            assert_eq!(
                serve(&backend, &write).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{}",
                write.name()
            );
        }
    }

    #[test]
    fn mcp_request_names_match_the_name_arms() {
        let scope = htui_scope();
        let id = McpServerId::new();
        let samples = [
            create_mcp(&scope, "docs"),
            edit_mcp(&scope, id, DateTime::UNIX_EPOCH, ""),
            delete_mcp(&scope, id),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, MCP_REQUEST_NAMES);
        let outcomes = [
            McpServerWrite::Created {
                server: id,
                name: String::new(),
            },
            McpServerWrite::Edited {
                server: id,
                name: String::new(),
            },
            McpServerWrite::Deleted {
                server: id,
                bindings: 0,
            },
        ];
        for (outcome, request) in outcomes.iter().zip(&samples) {
            assert_eq!(outcome.request_name(), request.name(), "{outcome:?}");
        }
    }

    /// A request that is not one of the six is refused by name rather than served.
    #[tokio::test]
    async fn a_foreign_request_is_refused_by_name() {
        let backend = demo();
        assert_eq!(
            serve(&backend, &StoreRequest::Workspaces).await.err(),
            Some(StoreError::Backend(
                "not a skills request: workspaces".to_owned()
            ))
        );
    }
}
