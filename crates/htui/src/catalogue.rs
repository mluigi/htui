//! The catalogue behind `Settings > Kinds`: every project of the scope with its kinds, its graphs
//! and each graph's whole body (MOD-15 milestone 4, D1/D3; MOD-137 D12).
//!
//! One read per event and never one per keystroke (M3 D5's trade), one reply out, and every write
//! through milestone 1's compare-and-set seam. The section renders from the snapshot and never
//! patches a single row into it, so there is exactly one source of truth on the render side.
//!
//! Nothing here resolves an identity: no row this module writes carries a `created_by` or a box, so
//! unlike [`crate::hierarchy`] there is no `this_user` and no `box_info` call in the file.

use std::collections::BTreeSet;

use chrono::Utc;
use htui_core::model::{
    GraphBody, GraphScope, ItemKind, ItemKindId, NewItemKind, NewStepGraph, PersonaId, PhaseId,
    Project, Scope, StepGraph, StepGraphId, Workspace, WorkspaceId,
};
use htui_core::prompt::SettingKey;
use htui_core::seed;
use htui_core::store::{CasOutcome, ReadStore, Result, SettingRung, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::Value;

use crate::hierarchy::MirrorAfterDelete;
use crate::store_worker::{StoreReply, StoreRequest};

/// One read of the scope: one entry per `scope.project_ids`, in scope order.
///
/// A project id that names no row is **skipped** rather than failing the read, exactly as a widowed
/// `workspace_project` link is in [`crate::hierarchy::snapshot`]: on Postgres the scope's ids come
/// from links that cascade with the project, so a missing project is a torn read rather than a
/// state to render (D3).
///
/// `Eq` is deliberately absent: `Project`, `ItemKind`, `StepGraph` and `StepGraphPhase` all derive
/// `PartialEq` only, and adding a derive to `htui-core` for this crate's convenience is not this
/// milestone's to do.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSnapshot {
    /// The scope's projects, in scope order.
    pub projects: Vec<ProjectCatalogue>,
    /// The global persona registry by name (MOD-26 M2 D23): the phase editor's `persona` field
    /// resolves against it and a bound phase's line names its persona from it.
    pub personas: Vec<PersonaSummary>,
}

/// One persona as `Settings > Kinds` names it (MOD-26 M2 D23): id and name, nothing the phase
/// editor does not print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaSummary {
    /// `persona.id`.
    pub id: PersonaId,
    /// `persona.name`.
    pub name: String,
}

/// MOD-141 M2 D6: what the graph editor reads beside the body, from one `app_setting` read: the
/// run limits resolve freezes and the compete switch. The answer to
/// [`StoreRequest::GraphEditorSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorSettings {
    /// [`htui_orch::graph::run_bounds`] over the `app_setting` rows.
    pub bounds: htui_orch::graph::RunBounds,
    /// [`htui_core::prompt::settings::resolve_compete_fan_out`]: whether the compete split is
    /// offered.
    pub compete: bool,
}

/// One project with everything the Kinds section shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectCatalogue {
    /// The project row; `updated_at` is not a CAS token here — this section edits no project.
    pub project: Project,
    /// `item_kinds(project)`: by `position`, then `prefix`.
    pub kinds: Vec<ItemKind>,
    /// The project's own graphs (`usable_graphs(project)`'s project rows): by `name`.
    pub graphs: Vec<GraphEntry>,
    /// MOD-138 D12: the workspace and library rows of `usable_graphs(project)`, in its order
    /// (each workspace's that holds the project, then the library's, by `name`). A kind may name
    /// one; the section edits none of them, so they carry no body.
    pub shared: Vec<StepGraph>,
    /// MOD-138 D12: the row of each graph a kind of the project still names that neither
    /// [`graphs`](Self::graphs) nor [`shared`](Self::shared) lists — a workspace graph kept after
    /// the project left that workspace. The kind still resolves, so its line names the graph with
    /// its scope rather than `graph missing`; no kind may newly take one (the store refuses it), so
    /// [`graph_name`](Self::graph_name) finds these and a typed name does not. In kind order.
    pub kept: Vec<StepGraph>,
    /// MOD-141 M3 D1/D2: the workspace of every [`shared`](Self::shared) and
    /// [`kept`](Self::kept) workspace graph, so [`label`](Self::label) names it. The same for
    /// every project of one snapshot (R1 L2): the scope's workspaces, read once, so the slug rule
    /// is decided over the whole scope and every project labels a graph alike.
    pub workspaces: WorkspaceNames,
}

/// One graph as the catalogue holds it: its whole [`GraphBody`] from one
/// [`WriteStore::graph_body`] read — the row, each node (phase by `position`, its candidates and
/// its parts) and the edges by (from position, to position, kind bytes).
///
/// The whole body and not a summary of it, because the editor's one write
/// ([`StoreRequest::ReplaceGraph`]) replaces candidates, parts and edges wholesale: a body built
/// from less than this would wipe what it left out (MOD-137 R1b, review C-M2). `graph.updated_at`
/// is the CAS token of [`StoreRequest::UpdateGraph`] and `graph.revision` that of
/// [`StoreRequest::ReplaceGraph`] (MOD-137 D5).
pub type GraphEntry = GraphBody;

impl ProjectCatalogue {
    /// The entry for one of the project's graphs, or `None` when the id names none of them.
    ///
    /// Linear over five graphs in the seeded shape: a kind whose `default_graph_id` resolves to
    /// nothing is a row the section draws as `graph missing` (D4), so the lookup has to be able to
    /// fail rather than index.
    #[must_use]
    pub fn graph(&self, id: StepGraphId) -> Option<&GraphEntry> {
        self.graphs.iter().find(|entry| entry.graph.id == id)
    }

    /// MOD-138 D12: the name and scope of any graph a kind of this project may name, its own
    /// ([`Self::graph`]) or a [`shared`](Self::shared) one, or of one a kind
    /// [`kept`](Self::kept); `None` when the id names none of them.
    #[must_use]
    pub fn graph_name(&self, id: StepGraphId) -> Option<(&str, GraphScope)> {
        self.graph(id)
            .map(|entry| &entry.graph)
            .or_else(|| {
                self.shared
                    .iter()
                    .chain(&self.kept)
                    .find(|graph| graph.id == id)
            })
            .map(|graph| (graph.name.as_str(), graph.scope))
    }
}

impl ProjectCatalogue {
    /// MOD-141 M3 D1: how every list of this catalogue names `graph`, through
    /// [`workspaces`](Self::workspaces) ([`WorkspaceNames::label`]).
    #[must_use]
    pub fn label(&self, graph: &StepGraph) -> String {
        self.workspaces.label(&graph.name, graph.scope)
    }

    /// [`label`](Self::label) of the graph `id` names, found as [`graph_name`](Self::graph_name)
    /// finds it; `None` when the id names none of them.
    #[must_use]
    pub fn label_of(&self, id: StepGraphId) -> Option<String> {
        self.graph_name(id)
            .map(|(name, scope)| self.workspaces.label(name, scope))
    }
}

/// MOD-141 M3 D1: one workspace as a graph label names it: the id, the name, and the slug that
/// stands in for the name when another listed workspace shares it or spells it as its slug
/// (`workspace.slug` is unique, `workspace.name` is not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceName {
    /// `workspace.id`.
    pub id: WorkspaceId,
    /// `workspace.name`.
    pub name: String,
    /// `workspace.slug`.
    pub slug: String,
}

/// MOD-141 M3 D1: the workspaces of the graphs one list shows, sorted by id with one row per id,
/// and the one labeller every list shares (MOD-138 D12): see [`label`](Self::label).
///
/// The default names no workspace, so every workspace graph reads `name · workspace`, the label
/// before M3.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceNames {
    /// Sorted by id, one row per id.
    rows: Vec<WorkspaceName>,
}

impl WorkspaceNames {
    /// The rows, sorted by id; a repeated id keeps its first row.
    #[must_use]
    pub fn new(rows: impl IntoIterator<Item = WorkspaceName>) -> Self {
        let mut rows: Vec<WorkspaceName> = rows.into_iter().collect();
        rows.sort_by_key(|row| row.id);
        rows.dedup_by_key(|row| row.id);
        Self { rows }
    }

    /// One [`WriteStore::workspace`] read per distinct workspace among `graphs`' scopes (D2: per
    /// event, never per keystroke). A workspace the read no longer finds is left out, a torn read
    /// like a missing project, and its graphs keep the bare `name · workspace`.
    ///
    /// Never fails (MOD-141 M3 R1 L3): a read that errs leaves that workspace out too, logged at
    /// `warn`, so a failed name read costs a label its workspace word and never the reply the
    /// names ride in (the item form, a divergence, the catalogue).
    pub async fn read<'a, S: WriteStore + ?Sized>(
        store: &S,
        graphs: impl IntoIterator<Item = &'a StepGraph>,
    ) -> Self {
        // Collected before the first await, so no borrow of `graphs` is held across one.
        let ids = Self::ids(graphs);
        let mut reads = Vec::with_capacity(ids.len());
        for id in ids {
            reads.push((id, store.workspace(id).await));
        }
        Self::from_reads(reads)
    }

    /// The distinct workspaces among `graphs`' scopes, in id order: what [`read`](Self::read)
    /// reads, one read each.
    fn ids<'a>(graphs: impl IntoIterator<Item = &'a StepGraph>) -> BTreeSet<WorkspaceId> {
        graphs
            .into_iter()
            .filter_map(|graph| graph.scope.workspace_id())
            .collect()
    }

    /// The names [`read`](Self::read)'s answers give, one `(workspace, answer)` each in id order:
    /// a found row is named, a missing one left out, and a failed one left out and logged (R1
    /// L3). The seam its tests fail a read through.
    fn from_reads(
        reads: impl IntoIterator<Item = (WorkspaceId, Result<Option<Workspace>>)>,
    ) -> Self {
        let mut rows = Vec::new();
        for (id, read) in reads {
            match read {
                Ok(Some(workspace)) => rows.push(WorkspaceName {
                    id,
                    name: workspace.name,
                    slug: workspace.slug,
                }),
                Ok(None) => {}
                Err(err) => tracing::warn!(
                    %err,
                    workspace = %id,
                    "a workspace name read failed; its graphs read `name \u{b7} workspace`"
                ),
            }
        }
        Self { rows }
    }

    /// How a list names a graph a kind or item may use (MOD-138 D12, MOD-141 M3 M-1): a project
    /// graph by its bare name; a library graph as `name · library`; a workspace graph as
    /// `name · workspace <word>` ([`word`](Self::word): its name, or its slug when the name is
    /// another listed row's name or slug); and `name · workspace` for a workspace no read found.
    /// A shared graph so never reads as one of the project's own, and two read workspaces'
    /// same-named graphs never read alike.
    #[must_use]
    pub fn label(&self, name: &str, scope: GraphScope) -> String {
        match scope {
            GraphScope::Project(_) => name.to_owned(),
            GraphScope::Library => format!("{name} \u{b7} library"),
            GraphScope::Workspace(id) => match self.row(id) {
                None => format!("{name} \u{b7} workspace"),
                Some(row) => format!("{name} \u{b7} workspace {}", self.word(row)),
            },
        }
    }

    /// MOD-141 M3 R1 M1: every spelling a typed label may name a workspace graph by, whatever
    /// [`label`](Self::label) shows today: `name · workspace <workspace name>` and
    /// `name · workspace <slug>`. Which one a list shows depends on what else is listed, so a
    /// label typed or prefilled under one list stays valid under the next. None for another scope
    /// or for a workspace no read found.
    #[must_use]
    pub fn spellings(&self, name: &str, scope: GraphScope) -> Vec<String> {
        let Some(row) = scope.workspace_id().and_then(|id| self.row(id)) else {
            return Vec::new();
        };
        [&row.name, &row.slug]
            .into_iter()
            .map(|word| format!("{name} \u{b7} workspace {word}"))
            .collect()
    }

    /// The row of workspace `id`, when a read found it.
    fn row(&self, id: WorkspaceId) -> Option<&WorkspaceName> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The word a label names `row` by (D1, R1 L5): its name, unless another listed row has that
    /// name or spells it as its slug; then its slug, which is unique. So no two read rows ever
    /// read alike: a name word matches no other row's name or slug, and two slugs never match.
    fn word<'a>(&self, row: &'a WorkspaceName) -> &'a str {
        let taken = self
            .rows
            .iter()
            .any(|other| other.id != row.id && (other.name == row.name || other.slug == row.name));
        if taken { &row.slug } else { &row.name }
    }

    /// How many workspaces are named.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no workspace is named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// One read of the whole scope: every project of `scope.project_ids` that still names a row, with
/// its kinds, its graphs and each graph's whole body, then the persona registry by name (MOD-26 M2
/// D23) — global rather than per project, so one `personas()` read whatever the scope holds.
///
/// N+1 reads on purpose (D3, M3 D5's trade, same words): they happen per event — activation, a
/// scope change, after a write — never per keystroke, and a joined reader would be a seam method
/// six implementors would owe for a read nothing else wants. The graphs are one `usable_graphs`
/// read (MOD-138 D12): its project rows each get one `graph_body` read (MOD-137 R1b, review
/// C-L5), not a `phases` and a `graph_edges` read, and its workspace and library rows go to
/// [`ProjectCatalogue::shared`] as they are; a graph `usable_graphs` named and `graph_body` no
/// longer finds is skipped, a torn read like a missing project. A kind's graph neither list holds
/// gets one more `graph_body` read for [`ProjectCatalogue::kept`]; one no read finds is left out,
/// and the kind draws as `graph missing`. Then one `workspace` read per distinct workspace among
/// every project's `shared` and `kept` (MOD-141 M3 D2, R1 L2, [`WorkspaceNames::read`]) names
/// each workspace graph's workspace, the same names for every project; a failed name read leaves
/// that workspace unnamed rather than failing the snapshot (R1 L3).
///
/// The bound is `ReadStore + WriteStore` rather than `ReadStore` alone because `item_kinds`,
/// `usable_graphs` and `graph_body` live on [`WriteStore`] (which extends [`ReadStore`]); only
/// `project` is on the read half. Same shape as [`crate::hierarchy::snapshot`]'s bound.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot<S: ReadStore + WriteStore + ?Sized>(
    store: &S,
    scope: &Scope,
) -> Result<CatalogueSnapshot> {
    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for id in &scope.project_ids {
        let Some(project) = store.project(*id).await? else {
            continue;
        };
        let kinds = store.item_kinds(*id).await?;
        let mut graphs = Vec::new();
        let mut shared = Vec::new();
        for graph in store.usable_graphs(*id).await? {
            if graph.scope != GraphScope::Project(*id) {
                shared.push(graph);
            } else if let Some(body) = store.graph_body(graph.id).await? {
                graphs.push(body);
            }
        }
        let mut kept: Vec<StepGraph> = Vec::new();
        for kind in &kinds {
            let id = kind.default_graph_id;
            let listed = graphs.iter().any(|entry| entry.graph.id == id)
                || shared.iter().chain(&kept).any(|graph| graph.id == id);
            if !listed && let Some(body) = store.graph_body(id).await? {
                kept.push(body.graph);
            }
        }
        projects.push(ProjectCatalogue {
            project,
            kinds,
            graphs,
            shared,
            kept,
            workspaces: WorkspaceNames::default(),
        });
    }
    // MOD-141 M3 R1 L2: one read per distinct workspace across the whole scope, and the slug rule
    // decided over all of them, so a graph two projects share reads alike on both lines.
    // Collected into a `Vec` first: a `flat_map` closure held in this future trips the worker
    // task's `Send` check (a higher-ranked lifetime the compiler cannot prove general).
    let graphs: Vec<&StepGraph> = projects
        .iter()
        .flat_map(|project| project.shared.iter().chain(&project.kept))
        .collect();
    let workspaces = WorkspaceNames::read(store, graphs).await;
    for project in &mut projects {
        project.workspaces = workspaces.clone();
    }
    let personas = store
        .personas()
        .await?
        .into_iter()
        .map(|row| PersonaSummary {
            id: row.id,
            name: row.name,
        })
        .collect();
    Ok(CatalogueSnapshot { projects, personas })
}

/// Serves one catalogue request, off the UI task.
///
/// `Err(StoreError::Unreachable)` on [`Backend::Offline`], whose [`writer`](Backend::writer) is
/// `None` — including for the read, so the section's `unavailable` path is the same sentence every
/// other request gets there.
///
/// Known residue, the same one [`crate::hierarchy::serve`] carries and milestone 3 recorded: every
/// write arm answers `Failed` if the re-read *after* an applied write fails, so the section is told
/// nothing happened when the row has in fact changed — separating the two needs a seam method that
/// returns the write's own outcome without a read, and this milestone adds none. `replace_graph`
/// is exempt (MOD-141 R1 L1): its outcome travels in its own reply, so a failed re-read after it
/// answers [`StoreReply::GraphReplaced`] with `catalogue: None` rather than `Failed`.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline and
/// [`StoreError::Backend`] for a request that is not one of this module's thirteen.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;

    match request {
        StoreRequest::Catalogue(scope) => reread(&writer, scope).await,
        StoreRequest::CreateKind {
            scope,
            project,
            prefix,
            name,
            description,
            graph,
            position,
        } => {
            writer
                .create_item_kind(NewItemKind {
                    id: ItemKindId::new(),
                    project_id: *project,
                    prefix: prefix.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    default_graph_id: *graph,
                    position: *position,
                })
                .await?;
            reread(&writer, scope).await
        }
        StoreRequest::UpdateKind {
            scope,
            id,
            expected,
            patch,
        } => {
            let outcome = writer
                .update_item_kind(*id, *expected, patch.clone())
                .await?;
            cas(&writer, scope, &outcome).await
        }
        StoreRequest::DeleteKind { scope, id } => {
            writer.delete_item_kind(*id).await?;
            // `item_kind` is a mirrored table whose deletes have no propagation of their own — the
            // refresh pass rides `updated_at` and only `item_link` is tombstoned — so without this
            // the Backlog keeps offering a kind that is gone (D12). The row is gone before this
            // line, so a failed rebuild is reported *inside* a successful reply rather than turning
            // a delete that happened into a `Failed` that claims nothing did.
            let mirror = match backend.cache() {
                Some(cache) => match cache.rebuild().await {
                    Ok(()) => MirrorAfterDelete::Rebuilt,
                    Err(err) => MirrorAfterDelete::Failed(err.to_string()),
                },
                None => MirrorAfterDelete::NoMirror,
            };
            Ok(StoreReply::KindDeleted {
                mirror,
                catalogue: Box::new(snapshot(&writer, scope).await?),
            })
        }
        StoreRequest::CreateGraph {
            scope,
            project,
            name,
            description,
        } => {
            writer
                .create_step_graph(NewStepGraph {
                    id: StepGraphId::new(),
                    scope: GraphScope::Project(*project),
                    name: name.clone(),
                    description: description.clone(),
                    is_override: false,
                })
                .await?;
            reread(&writer, scope).await
        }
        StoreRequest::UpdateGraph {
            scope,
            id,
            expected,
            patch,
        } => {
            let outcome = writer
                .update_step_graph(*id, *expected, patch.clone())
                .await?;
            cas(&writer, scope, &outcome).await
        }
        StoreRequest::CreatePhase {
            scope,
            graph,
            name,
            position,
            template_name,
            gate_hard,
            input_kinds,
        } => {
            // D9/F-6: `PhaseSeed`'s `name` and `input_kinds` are `&'static`, so typed text cannot
            // travel through them. The seed carries `gate_hard` alone and the four text columns are
            // written over the row `phase_row` returned — the eight frozen ones (`fan_out`, `gate`,
            // `retry_limit`, `isolation`, `command_queue`, `verify_command`, `template_version`,
            // `token_budget`) are never named here, so the app is not a second source of ANA-2's
            // defaults. `create_phase` ignores the row's `updated_at` and returns the store's own
            // clock, so the `now` below is not this crate setting the column by hand.
            let mut row = seed::phase_row(
                PhaseId::new(),
                *graph,
                *position,
                &seed::PhaseSeed {
                    name: "",
                    input_kinds: &[],
                    gate_hard: *gate_hard,
                },
                Utc::now(),
            );
            row.name = name.clone();
            row.output_kind = name.clone();
            row.template_name = template_name.clone();
            row.input_kinds = input_kinds.clone();
            writer.create_phase(&row).await?;
            reread(&writer, scope).await
        }
        StoreRequest::UpdatePhase {
            scope,
            id,
            expected,
            patch,
        } => {
            let outcome = writer.update_phase(*id, *expected, patch.clone()).await?;
            cas(&writer, scope, &outcome).await
        }
        // `set` and `clear` are separate operations because clearing lets the compiled default
        // answer rather than the editor guessing at a constant (PRD D7, D16). `expected` is always
        // `Some` on the set: the Phase rung refuses `None` with a constraint, not a CAS miss (B-2,
        // H-4), and the editor always has the row's token.
        StoreRequest::SetPhaseBudget {
            scope,
            phase,
            expected,
            budget,
        } => {
            let outcome = match budget {
                Some(number) => {
                    writer
                        .set_setting(
                            SettingRung::Phase(*phase),
                            SettingKey::TokenBudget,
                            Value::from(*number),
                            Some(*expected),
                        )
                        .await?
                }
                None => {
                    writer
                        .clear_setting(
                            SettingRung::Phase(*phase),
                            SettingKey::TokenBudget,
                            *expected,
                        )
                        .await?
                }
            };
            cas(&writer, scope, &outcome).await
        }
        // MOD-137 D12: the editor's one write, CAS on `step_graph.revision`. A refusal is the
        // seam's `Constraint`, which the worker turns into `Failed { request: "replace_graph" }`.
        StoreRequest::ReplaceGraph {
            scope,
            graph,
            expected_revision,
            body,
        } => {
            let outcome = writer
                .replace_graph(*graph, *expected_revision, (**body).clone())
                .await?;
            // MOD-141 D13: the body travels back with the scope re-read, so the editor holds what
            // is stored (`Applied`) or what it now must rebase onto (`Stale`). R1 L1: the write
            // has happened by now, so a failed re-read is `None`, never a `Failed` that says it
            // did not.
            let catalogue = snapshot(&writer, scope).await.ok().map(Box::new);
            let outcome = match outcome {
                CasOutcome::Applied(body) => CasOutcome::Applied(Box::new(body)),
                CasOutcome::Stale(body) => CasOutcome::Stale(Box::new(body)),
            };
            Ok(StoreReply::GraphReplaced { outcome, catalogue })
        }
        // MOD-137 D11/D12: the phase goes with its candidates, parts, edges and phase bindings.
        StoreRequest::DeletePhase { scope, phase } => {
            writer.delete_phase(*phase).await?;
            reread(&writer, scope).await
        }
        // MOD-141 D13: any scope's graph by id, for the editor's open-by-id (the Toolbox hook).
        StoreRequest::GraphBody(graph) => Ok(StoreReply::GraphBody(
            writer.graph_body(*graph).await?.map(Box::new),
        )),
        // MOD-141 M2 D6: one `app_setting` read, the editor's limits and switch. Offline it never
        // gets here: `writer()` is `None` and the request answers `Unreachable` like the rest.
        StoreRequest::GraphEditorSettings => {
            let app = backend.app_settings().await?;
            Ok(StoreReply::GraphEditorSettings(Box::new(EditorSettings {
                bounds: htui_orch::graph::run_bounds(&app),
                compete: htui_core::prompt::settings::resolve_compete_fan_out(&app),
            })))
        }
        // `try_serve` routes exactly this module's thirteen variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a catalogue request: {}",
            other.name()
        ))),
    }
}

/// The catalogue as it is now, for a read or for a write that applied.
async fn reread(writer: &Writer, scope: &Scope) -> Result<StoreReply> {
    Ok(StoreReply::Catalogue(Box::new(
        snapshot(writer, scope).await?,
    )))
}

/// A compare-and-set outcome as a reply: `Applied` answers the fresh catalogue, `Stale` answers the
/// same catalogue under [`StoreReply::CatalogueStale`] so the editor reloads and retries by hand
/// (D8).
///
/// The worker re-reads rather than handing the section the single row `Stale` carries: the section
/// renders a tree, and a row patched in locally would be a second source of truth (D3).
async fn cas<T>(writer: &Writer, scope: &Scope, outcome: &CasOutcome<T>) -> Result<StoreReply> {
    let fresh = Box::new(snapshot(writer, scope).await?);
    Ok(match outcome {
        CasOutcome::Applied(_) => StoreReply::Catalogue(fresh),
        CasOutcome::Stale(_) => StoreReply::CatalogueStale(fresh),
    })
}

/// The thirteen request names, in [`StoreRequest`] order: MOD-15 M4's nine, MOD-137 D12's two,
/// then MOD-141 D13's one and M2 D6's one.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a
/// fourteenth request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 13] = [
    "catalogue",
    "create_kind",
    "update_kind",
    "delete_kind",
    "create_graph",
    "update_graph",
    "create_phase",
    "update_phase",
    "set_phase_budget",
    "replace_graph",
    "delete_phase",
    "graph_body",
    "graph_editor_settings",
];

#[cfg(test)]
mod tests {
    use super::{WorkspaceName, WorkspaceNames, snapshot};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        GraphScope, ItemKindId, NewItemKind, NewStepGraph, Scope, StepGraph, StepGraphId,
        WorkspaceId, WorkspaceProject,
    };
    use htui_core::store::{MemStore, WriteStore as _};

    fn row(id: WorkspaceId, name: &str, slug: &str) -> WorkspaceName {
        WorkspaceName {
            id,
            name: name.to_owned(),
            slug: slug.to_owned(),
        }
    }

    /// The demo's two workspaces, as a read of both would name them.
    fn demo_names() -> WorkspaceNames {
        WorkspaceNames::new([
            row(ids::WORKSPACE_GRAPHICS, "Graphics", "graphics"),
            row(ids::WORKSPACE_PLATFORM, "Platform", "platform"),
        ])
    }

    async fn graph(store: &MemStore, scope: GraphScope, name: &str) -> StepGraph {
        store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope,
                name: name.to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("create a graph")
    }

    /// MOD-141 M3 D1 (M-1): a project graph is its bare name, a library graph says so, and a
    /// workspace graph always names its workspace.
    #[test]
    fn workspace_names_label_each_scope() {
        let names = demo_names();
        assert_eq!(names.len(), 2);
        assert!(!names.is_empty());
        assert_eq!(
            names.label("flow", GraphScope::Project(ids::PROJECT_HTUI)),
            "flow"
        );
        assert_eq!(
            names.label("flow", GraphScope::Library),
            "flow \u{b7} library"
        );
        assert_eq!(
            names.label("flow", GraphScope::Workspace(ids::WORKSPACE_PLATFORM)),
            "flow \u{b7} workspace Platform"
        );
        assert_eq!(
            names.label("flow", GraphScope::Workspace(ids::WORKSPACE_GRAPHICS)),
            "flow \u{b7} workspace Graphics"
        );
    }

    /// D1: a workspace name is not unique and its slug is, so two listed workspaces of one name
    /// read by slug, and a third of another name keeps its name.
    #[test]
    fn same_named_workspaces_read_by_slug() {
        let other = WorkspaceId::new();
        let names = WorkspaceNames::new([
            row(ids::WORKSPACE_PLATFORM, "Shared", "platform"),
            row(ids::WORKSPACE_GRAPHICS, "Shared", "graphics"),
            row(other, "Other", "other"),
        ]);
        assert_eq!(
            names.label("flow", GraphScope::Workspace(ids::WORKSPACE_PLATFORM)),
            "flow \u{b7} workspace platform"
        );
        assert_eq!(
            names.label("flow", GraphScope::Workspace(ids::WORKSPACE_GRAPHICS)),
            "flow \u{b7} workspace graphics"
        );
        assert_eq!(
            names.label("flow", GraphScope::Workspace(other)),
            "flow \u{b7} workspace Other"
        );
    }

    /// D1: a workspace no read found keeps today's bare word, and the default names nothing.
    #[test]
    fn an_unread_workspace_keeps_the_bare_word() {
        let unread = GraphScope::Workspace(WorkspaceId::new());
        assert_eq!(demo_names().label("flow", unread), "flow \u{b7} workspace");
        let none = WorkspaceNames::default();
        assert!(none.is_empty());
        assert_eq!(
            none.label("flow", GraphScope::Workspace(ids::WORKSPACE_PLATFORM)),
            "flow \u{b7} workspace"
        );
        assert_eq!(
            none.label("flow", GraphScope::Library),
            "flow \u{b7} library"
        );
    }

    /// D2: the snapshot reads the workspace of every shared graph and of every kept one, and its
    /// projects label through them.
    #[tokio::test]
    async fn the_snapshot_names_the_workspaces_of_shared_and_kept_graphs() {
        let store = MemStore::demo();
        store
            .upsert_workspace_project(&WorkspaceProject {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_id: ids::PROJECT_VULKAN,
                position: 9,
            })
            .await
            .expect("vulkan joins platform");
        let shared = graph(
            &store,
            GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
            "flow",
        )
        .await;
        let kept = graph(
            &store,
            GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
            "kept",
        )
        .await;
        store
            .create_item_kind(NewItemKind {
                id: ItemKindId::new(),
                project_id: ids::PROJECT_VULKAN,
                prefix: "DOC".to_owned(),
                name: "docs".to_owned(),
                description: String::new(),
                default_graph_id: kept.id,
                position: 99,
            })
            .await
            .expect("a kind on the graphics graph");
        store
            .remove_workspace_project(ids::WORKSPACE_GRAPHICS, ids::PROJECT_VULKAN)
            .await
            .expect("vulkan leaves graphics");

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_VULKAN],
        };
        let read = snapshot(&store, &scope).await.expect("the catalogue");
        let project = &read.projects[0];

        assert!(project.shared.iter().any(|graph| graph.id == shared.id));
        assert_eq!(
            project
                .kept
                .iter()
                .map(|graph| graph.id)
                .collect::<Vec<_>>(),
            vec![kept.id]
        );
        assert_eq!(project.workspaces, demo_names());
        assert_eq!(project.label(&shared), "flow \u{b7} workspace Platform");
        assert_eq!(
            project.label_of(kept.id).as_deref(),
            Some("kept \u{b7} workspace Graphics")
        );
        let own = &project.graphs[0].graph;
        assert_eq!(project.label_of(own.id).as_deref(), Some(own.name.as_str()));
        assert_eq!(project.label_of(StepGraphId::new()), None);
    }

    /// MOD-141 M3 R1 L5: a name another row spells as its slug reads by slug too, so no two read
    /// workspaces ever label alike: `x`'s name is the first `Shared`'s slug, and `z`'s name is
    /// then `x`'s slug.
    #[test]
    fn a_name_another_row_uses_as_its_slug_reads_by_slug() {
        let ids: Vec<WorkspaceId> = (0..4).map(|_| WorkspaceId::new()).collect();
        let names = WorkspaceNames::new([
            row(ids[0], "Shared", "x"),
            row(ids[1], "Shared", "y"),
            row(ids[2], "x", "z"),
            row(ids[3], "z", "w"),
        ]);
        let labels: Vec<String> = ids
            .iter()
            .map(|&id| names.label("flow", GraphScope::Workspace(id)))
            .collect();
        assert_eq!(
            labels,
            [
                "flow \u{b7} workspace x",
                "flow \u{b7} workspace y",
                "flow \u{b7} workspace z",
                "flow \u{b7} workspace w",
            ]
        );
    }

    /// R1 M1: every spelling a typed label may name a workspace graph by, whatever `label` shows:
    /// by workspace name and by slug; none for another scope or an unread workspace.
    #[test]
    fn spellings_name_a_workspace_graph_both_ways() {
        let names = WorkspaceNames::new([
            row(ids::WORKSPACE_PLATFORM, "Shared", "platform"),
            row(ids::WORKSPACE_GRAPHICS, "Shared", "graphics"),
        ]);
        assert_eq!(
            names.spellings("flow", GraphScope::Workspace(ids::WORKSPACE_PLATFORM)),
            [
                "flow \u{b7} workspace Shared",
                "flow \u{b7} workspace platform"
            ]
        );
        assert!(names.spellings("flow", GraphScope::Library).is_empty());
        assert!(
            names
                .spellings("flow", GraphScope::Workspace(WorkspaceId::new()))
                .is_empty()
        );
    }

    /// R1 L2 and L6: the scope's workspaces are read once, and the slug rule decided over the
    /// whole scope, so a graph two projects share reads alike on both. Graphics is renamed
    /// `Platform` and holds a graph only Vulkan sees, so the two `Platform`s read by slug on
    /// `htui`'s line as well as on Vulkan's: the slug path through `snapshot`.
    #[tokio::test]
    async fn every_project_of_the_scope_labels_a_graph_alike() {
        use htui_core::model::WorkspacePatch;
        let store = MemStore::demo();
        store
            .upsert_workspace_project(&WorkspaceProject {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_id: ids::PROJECT_VULKAN,
                position: 9,
            })
            .await
            .expect("vulkan joins platform");
        let graphics = store
            .workspace(ids::WORKSPACE_GRAPHICS)
            .await
            .expect("a memory read")
            .expect("graphics");
        store
            .update_workspace(
                graphics.id,
                graphics.updated_at,
                WorkspacePatch {
                    name: Some("Platform".to_owned()),
                    ..WorkspacePatch::default()
                },
            )
            .await
            .expect("graphics is renamed");
        let flow = graph(
            &store,
            GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
            "flow",
        )
        .await;
        let other = graph(
            &store,
            GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
            "other",
        )
        .await;
        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_VULKAN],
        };

        let read = snapshot(&store, &scope).await.expect("the catalogue");

        assert_eq!(read.projects.len(), 2);
        for project in &read.projects {
            assert_eq!(
                project.label_of(flow.id).as_deref(),
                Some("flow \u{b7} workspace platform"),
                "{}",
                project.project.slug
            );
            assert_eq!(project.workspaces, read.projects[0].workspaces);
        }
        assert_eq!(
            read.projects[1].label(&other),
            "other \u{b7} workspace graphics"
        );
    }

    /// R1 L3: a workspace whose read fails is left unread, logged, and its graphs keep the bare
    /// `name · workspace`; the rest are named. One read per distinct workspace, however many of
    /// its graphs are listed.
    #[tokio::test]
    async fn a_failed_workspace_read_leaves_that_workspace_unread() {
        use htui_core::store::StoreError;
        let store = MemStore::demo();
        let platform = store
            .workspace(ids::WORKSPACE_PLATFORM)
            .await
            .expect("a memory read")
            .expect("platform");
        let graphs = [
            graph(
                &store,
                GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
                "one",
            )
            .await,
            graph(
                &store,
                GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
                "two",
            )
            .await,
            graph(
                &store,
                GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
                "three",
            )
            .await,
            graph(&store, GraphScope::Library, "four").await,
        ];
        let asked = WorkspaceNames::ids(&graphs);
        let mut expected = vec![ids::WORKSPACE_PLATFORM, ids::WORKSPACE_GRAPHICS];
        expected.sort();
        assert_eq!(
            asked.iter().copied().collect::<Vec<_>>(),
            expected,
            "one read per workspace"
        );

        let names = WorkspaceNames::from_reads(asked.into_iter().map(|id| {
            let read = if id == ids::WORKSPACE_GRAPHICS {
                Err(StoreError::Backend("down".to_owned()))
            } else {
                Ok(Some(platform.clone()))
            };
            (id, read)
        }));

        assert_eq!(names.len(), 1);
        assert_eq!(
            names.label("one", graphs[0].scope),
            "one \u{b7} workspace Platform"
        );
        assert_eq!(
            names.label("three", graphs[2].scope),
            "three \u{b7} workspace"
        );
    }
}
