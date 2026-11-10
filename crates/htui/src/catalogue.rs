//! The catalogue behind `Settings > Kinds`: every project of the scope with its kinds, its graphs
//! and each graph's whole body (MOD-15 milestone 4, D1/D3; MOD-137 D12).
//!
//! One read per event and never one per keystroke (M3 D5's trade), one reply out, and every write
//! through milestone 1's compare-and-set seam. The section renders from the snapshot and never
//! patches a single row into it, so there is exactly one source of truth on the render side.
//!
//! Nothing here resolves an identity: no row this module writes carries a `created_by` or a box, so
//! unlike [`crate::hierarchy`] there is no `this_user` and no `box_info` call in the file.

use chrono::Utc;
use htui_core::model::{
    GraphBody, GraphScope, ItemKind, ItemKindId, NewItemKind, NewStepGraph, PersonaId, PhaseId,
    Project, Scope, StepGraph, StepGraphId,
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

/// MOD-138 D12: how a list names a graph a kind or item may use: a project graph by its bare name,
/// a workspace or library graph as `name · workspace` / `name · library`, so a shared graph never
/// reads as one of the project's own.
#[must_use]
pub fn graph_label(name: &str, scope: GraphScope) -> String {
    match scope {
        GraphScope::Project(_) => name.to_owned(),
        GraphScope::Workspace(_) => format!("{name} \u{b7} workspace"),
        GraphScope::Library => format!("{name} \u{b7} library"),
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
/// and the kind draws as `graph missing`.
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
        });
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
/// returns the write's own outcome without a read, and this milestone adds none.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline and
/// [`StoreError::Backend`] for a request that is not one of this module's twelve.
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
            // is stored (`Applied`) or what it now must rebase onto (`Stale`).
            let catalogue = Box::new(snapshot(&writer, scope).await?);
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
        // `try_serve` routes exactly this module's twelve variants here, so the last arm is
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

/// The twelve request names, in [`StoreRequest`] order: MOD-15 M4's nine, MOD-137 D12's two, then
/// MOD-141 D13's one.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a
/// thirteenth request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 12] = [
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
];
