//! The catalogue behind `Settings > Kinds`: every project of the scope with its kinds, its graphs
//! and each graph's phases (MOD-15 milestone 4, D1/D3).
//!
//! One read per event and never one per keystroke (M3 D5's trade), one reply out, and every write
//! through milestone 1's compare-and-set seam. The section renders from the snapshot and never
//! patches a single row into it, so there is exactly one source of truth on the render side.
//!
//! Nothing here resolves an identity: no row this module writes carries a `created_by` or a box, so
//! unlike [`crate::hierarchy`] there is no `this_user` and no `box_info` call in the file.

use chrono::Utc;
use htui_core::model::{
    ItemKind, ItemKindId, NewItemKind, NewStepGraph, PhaseId, Project, Scope, StepGraph,
    StepGraphId, StepGraphPhase,
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
}

/// One project with everything the Kinds section shows of it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectCatalogue {
    /// The project row; `updated_at` is not a CAS token here — this section edits no project.
    pub project: Project,
    /// `item_kinds(project)`: by `position`, then `prefix`.
    pub kinds: Vec<ItemKind>,
    /// `step_graphs(project)`: by `name`.
    pub graphs: Vec<GraphEntry>,
}

/// One graph and its phases by `position`.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEntry {
    /// The graph row; `updated_at` is its CAS token.
    pub graph: StepGraph,
    /// Its phases, ordered by `position` (unique per graph).
    pub phases: Vec<StepGraphPhase>,
}

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
}

/// One read of the whole scope: every project of `scope.project_ids` that still names a row, with
/// its kinds, its graphs and each graph's phases.
///
/// N+1 reads on purpose (D3, M3 D5's trade, same words): they happen per event — activation, a
/// scope change, after a write — never per keystroke, and a joined reader would be a seam method
/// six implementors would owe for a read nothing else wants.
///
/// The bound is `ReadStore + WriteStore` rather than `ReadStore` alone because `item_kinds`,
/// `step_graphs` and `phases` live on [`WriteStore`] (which extends [`ReadStore`]); only
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
        for graph in store.step_graphs(*id).await? {
            let phases = store.phases(graph.id).await?;
            graphs.push(GraphEntry { graph, phases });
        }
        projects.push(ProjectCatalogue {
            project,
            kinds,
            graphs,
        });
    }
    Ok(CatalogueSnapshot { projects })
}

/// Serves one catalogue request, off the UI task.
///
/// `Err(StoreError::Unreachable)` on [`Backend::Offline`], whose [`writer`](Backend::writer) is
/// `None` — including for the read, so the section's `unavailable` path is the same sentence every
/// other request gets there.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline and
/// [`StoreError::Backend`] for a request that is not one of this module's nine.
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
                    project_id: *project,
                    name: name.clone(),
                    description: description.clone(),
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
        // `try_serve` routes exactly this module's nine variants here, so the last arm is
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

/// The nine request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a tenth
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 9] = [
    "catalogue",
    "create_kind",
    "update_kind",
    "delete_kind",
    "create_graph",
    "update_graph",
    "create_phase",
    "update_phase",
    "set_phase_budget",
];
