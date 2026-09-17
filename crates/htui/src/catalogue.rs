//! The catalogue behind `Settings > Kinds`: every project of the scope with its kinds, its graphs
//! and each graph's phases (MOD-15 milestone 4, D1/D3).
//!
//! One read per event and never one per keystroke (M3 D5's trade), one reply out, and every write
//! through milestone 1's compare-and-set seam. The section renders from the snapshot and never
//! patches a single row into it, so there is exactly one source of truth on the render side.
//!
//! Nothing here resolves an identity: no row this module writes carries a `created_by` or a box, so
//! unlike [`crate::hierarchy`] there is no `this_user` and no `box_info` call in the file.

use htui_core::model::{ItemKind, Project, Scope, StepGraph, StepGraphId, StepGraphPhase};
use htui_core::store::{ReadStore, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

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
