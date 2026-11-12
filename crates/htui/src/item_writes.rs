//! The Backlog's item writes (MOD-13 milestone 2): the read that opens the item form, a mint and an
//! edit of the seven `version`-covered spec columns (ANA-9 §4.2). The [`crate::requirements`]
//! shape: one module, one [`serve`], one or-arm in [`crate::store_worker`].
//!
//! **Offline is refused here, and that is the gate** (D2). All three requests take
//! [`Backend::writer`] before they read anything, and answer `Unreachable(DATABASE_UNREACHABLE)`
//! without one, the form's read included, so an offline box never opens a form. The Backlog has no
//! `writable` flag of its own; no view can get past a refusal the worker gives.
//!
//! **The form's read goes through the writer** (D3): `item_kinds`, `step_graphs` and `repos` are
//! `WriteStore` reads. [`ItemFormContext`] carries the project's kinds, its step graphs minus the
//! per-item override clones (A3), its repos, and for an edit the item read fresh, whose `version`
//! is the compare-and-set token the form saves at.
//!
//! **The validator is the authority here** (D4): both writes re-read the catalogue and run
//! [`item_spec::check_spec`] or [`item_spec::check_changes`], the functions the form called for its
//! early feedback, so the two give the same sentence. A refusal is a [`StoreError::Constraint`]
//! through [`SpecError`]'s `Display`. An edit with no change is refused before anything is written
//! (D5), because both stores bump `version` on an all-`None` patch.
//!
//! **A stale edit is never written over the head** (milestone 2 D6). [`UpdateOutcome::Diverged`]
//! answers [`StoreReply::ItemDiverged`] with both sides and the catalogue just read (milestone 3
//! D7). The token is the request's own and nothing here moves it. The request carries its revision
//! reason (milestone 3 D6): `divergence_resolution` once the form is rebased on a head.
//!
//! **A mint's `Failed` is hedged unless it is a refusal** (D11, as amended by the maintainer's
//! §10.1): a COMMIT whose answer was lost comes back as `Failed`, and a retry would mint a second
//! item. [`mint_refused`] tells the tab which `Failed`s were given before the insert.
//!
//! Redaction (A9, blueprint E6): `body` and `touched_paths` travel in
//! [`ItemSpec`](htui_core::model::ItemSpec) and [`SpecChanges`](htui_core::model::SpecChanges),
//! whose hand-written `Debug` prints their lengths only. No request carries a `NewItem` or an
//! `ItemPatch`, both of which derive `Debug` over the body. The replies' [`ItemFormContext`] and
//! [`ItemDivergence`] carry whole items, so their `Debug` is hand-written too (review L2).
//!
//! The worker fills the id ([`ItemId::new`]), the author and the box from [`Backend::this_user`]
//! and [`Backend::box_info`]; the render side never holds a `UserId` (`R-NF-3`).

use htui_core::model::item_spec::{self, NOTHING_TO_SAVE, SpecContext};
use htui_core::model::{
    Item, ItemId, ItemKind, ItemRevision, ProjectId, Repo, SpecError, StepGraph,
};
use htui_core::store::{ReadStore as _, Result, StoreError, UpdateOutcome, WriteStore as _};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

use crate::catalogue::WorkspaceNames;
use crate::store_worker::{StoreReply, StoreRequest};

/// The three requests, in [`StoreRequest::name`] order. One list, so a name changed in one place
/// and not the other fails `request_names_match_the_name_arms`.
pub const REQUEST_NAMES: [&str; 3] = ["item_form", "mint_item", "edit_item"];

/// The read that opens the form; its `Failed` opens nothing.
pub const FORM_NAME: &str = REQUEST_NAMES[0];

/// The mint: its `Failed` is hedged unless [`mint_refused`] says it is a refusal (D11, §10.1).
pub const MINT_NAME: &str = REQUEST_NAMES[1];

/// The edit.
pub const EDIT_NAME: &str = REQUEST_NAMES[2];

/// Whether `name` is one of this module's three requests.
#[must_use]
pub fn is_item_request(name: &str) -> bool {
    REQUEST_NAMES.contains(&name)
}

/// The answer to `StoreRequest::ItemForm` (D3): one project's catalogue, read through the writer.
///
/// `Debug` is hand-written (review L2): see the impl.
#[derive(Clone, PartialEq)]
pub struct ItemFormContext {
    /// The project the form writes in.
    pub project: ProjectId,
    /// `item_kinds(project)`, store order (position, then prefix).
    pub kinds: Vec<ItemKind>,
    /// `usable_graphs(project)` minus `is_override` (A3, MOD-138 D12): the project's graphs,
    /// then each workspace's that holds it, then the library's.
    pub graphs: Vec<StepGraph>,
    /// `repos(project)`, name and `is_primary` included.
    pub repos: Vec<Repo>,
    /// The item, fresh, for an edit (its `version` is the compare-and-set token); `None` for new.
    pub item: Option<Item>,
    /// The row of `item`'s own `step_graph_id` when `graphs` lacks it: its override (A3), or
    /// (MOD-138 D12) a workspace graph it kept after the project left that workspace, which the
    /// picker labels by scope rather than as an override. `None` otherwise, and for a new item.
    pub item_graph: Option<StepGraph>,
    /// MOD-141 M3 D2: the workspace of every workspace graph in `graphs` and `item_graph`, so
    /// [`label`](Self::label) names it. Filled for the replies that reach the UI (the form's
    /// read and a divergence); empty in the catalogue a mint or an edit validates against.
    pub workspaces: WorkspaceNames,
}

impl ItemFormContext {
    /// What the validator checks a spec against: this catalogue.
    #[must_use]
    pub fn spec_context(&self) -> SpecContext<'_> {
        SpecContext {
            kinds: &self.kinds,
            graphs: &self.graphs,
            repos: &self.repos,
        }
    }

    /// MOD-141 M3 D1: how the graph picker and the divergence view name `graph`, through
    /// [`workspaces`](Self::workspaces) ([`WorkspaceNames::label`]).
    #[must_use]
    pub fn label(&self, graph: &StepGraph) -> String {
        self.workspaces.label(&graph.name, graph.scope)
    }
}

/// Ids, the key, the version and lengths, never the body or the paths (review L2): the rule of
/// `StoreRequest`'s doc (E6), since this struct rides in a [`StoreReply`]. The catalogue prints
/// as counts.
impl core::fmt::Debug for ItemFormContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ItemFormContext")
            .field("project", &self.project)
            .field("kinds", &self.kinds.len())
            .field("graphs", &self.graphs.len())
            .field("repos", &self.repos.len())
            .field("item", &self.item.as_ref().map(ItemDigest))
            .field(
                "item_graph",
                &self.item_graph.as_ref().map(|graph| graph.id),
            )
            .field("workspaces", &self.workspaces.len())
            .finish()
    }
}

/// A stale edit (milestone 2 D6): both sides and the project's catalogue, for milestone 3's view.
///
/// `Debug` is hand-written (review L2): see the impl.
#[derive(Clone, PartialEq)]
pub struct ItemDivergence {
    /// The row as it is now.
    pub head: Item,
    /// The revision at the version the edit was made from: the spec's cross-check (ANA-9 §4.2
    /// step 3). The view renders the form's own opened item instead (milestone 3 D1).
    pub ancestor: ItemRevision,
    /// The catalogue the worker read to check the edit, with `item = Some(head)` (milestone 3
    /// D7): the rebased form opens on it, so a re-kinded or re-graphed head still has labels.
    pub context: ItemFormContext,
}

/// As [`ItemFormContext`]'s: ids, the key, the versions and lengths only (review L2).
impl core::fmt::Debug for ItemDivergence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ItemDivergence")
            .field("head", &ItemDigest(&self.head))
            .field("ancestor", &RevisionDigest(&self.ancestor))
            .field("context", &self.context)
            .finish()
    }
}

/// An [`Item`] as the reply payloads print it: ids, key, version and lengths (review L2).
struct ItemDigest<'a>(&'a Item);

impl core::fmt::Debug for ItemDigest<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let item = self.0;
        f.debug_struct("Item")
            .field("id", &item.id)
            .field("project_id", &item.project_id)
            .field("kind_id", &item.kind_id)
            .field("key", &item.key)
            .field("version", &item.version)
            .field("body_len", &item.body.len())
            .field("touched_paths", &item.touched_paths.len())
            .finish()
    }
}

/// An [`ItemRevision`] as [`ItemDivergence`] prints it: ids, version and the body's length.
struct RevisionDigest<'a>(&'a ItemRevision);

impl core::fmt::Debug for RevisionDigest<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let revision = self.0;
        f.debug_struct("ItemRevision")
            .field("item_id", &revision.item_id)
            .field("version", &revision.version)
            .field("body_len", &revision.body.len())
            .finish()
    }
}

/// What an applied item write did (self-naming, MOD-59).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemWrite {
    /// A new item landed under `key`.
    Minted {
        /// The minted key, e.g. `ANA-3`.
        key: String,
    },
    /// An edit landed; `version` is the new head's.
    Edited {
        /// The item's key.
        key: String,
        /// The version the edit landed as.
        version: i32,
    },
}

/// The refusal of an edit-form read whose item is in another project.
#[must_use]
pub fn item_not_in_project(key: &str) -> String {
    format!("{key} is not in the project the form was opened for")
}

/// MOD-59 re-review L2, adopted by the maintainer (§10.1): whether `message`, a mint's `Failed` as
/// the worker renders it, is a refusal [`serve`] gives before the insert, so nothing was written.
/// That is the offline refusal ([`DATABASE_UNREACHABLE`]) and every [`StoreError::Constraint`]:
/// a spec refusal, [`NOTHING_TO_SAVE`], a store rule. A store `Constraint` never follows a COMMIT.
/// Anything else, an `Unreachable` for another reason or a `Backend` error, may follow a COMMIT
/// whose answer was lost, and the tab keeps hedging it.
#[must_use]
pub fn mint_refused(message: &str) -> bool {
    // Rendered from the errors themselves, so the prefix cannot drift from `StoreError`'s
    // `#[error]` text.
    let constraint = StoreError::Constraint(String::new()).to_string();
    message == offline().to_string() || message.starts_with(&constraint)
}

/// Serves one item request, off the UI task.
///
/// `ItemForm` answers [`StoreReply::ItemForm`]; an applied mint or edit answers
/// [`StoreReply::ItemWritten`]; an edit that missed its version answers
/// [`StoreReply::ItemDiverged`] and wrote nothing.
///
/// # Errors
/// Offline: `Unreachable(DATABASE_UNREACHABLE)` for all three, before anything is read (D2).
/// `Constraint` for a spec refusal (`SpecError` through `Display`), `NOTHING_TO_SAVE`, or a
/// cross-project edit read. `NotFound` for an unknown item. `Backend` for a non-item request.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    match request {
        StoreRequest::ItemForm { project, item } => {
            let writer = write_access(backend)?;
            let mut context = catalogue(&writer, *project).await?;
            if let Some(id) = item {
                let row = existing(&writer, *id).await?;
                if row.project_id != *project {
                    return Err(StoreError::Constraint(item_not_in_project(&row.key)));
                }
                context.item_graph = item_graph(&writer, &context.graphs, &row).await?;
                context.item = Some(row);
            }
            named(&writer, &mut context).await?;
            Ok(StoreReply::ItemForm(Box::new(context)))
        }
        StoreRequest::MintItem { project, spec } => {
            let writer = write_access(backend)?;
            let context = catalogue(&writer, *project).await?;
            let spec = item_spec::check_spec(spec, &context.spec_context()).map_err(refused)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            let row = writer
                .mint_item(spec.into_new_item(ItemId::new(), *project, me, box_id))
                .await?;
            Ok(StoreReply::ItemWritten {
                item: row.id,
                outcome: ItemWrite::Minted { key: row.key },
            })
        }
        StoreRequest::EditItem {
            id,
            expected_version,
            changes,
            reason,
        } => {
            let writer = write_access(backend)?;
            if changes.is_empty() {
                return Err(StoreError::Constraint(NOTHING_TO_SAVE.to_owned()));
            }
            let project = existing(&writer, *id).await?.project_id;
            let mut context = catalogue(&writer, project).await?;
            let changes =
                item_spec::check_changes(changes, &context.spec_context()).map_err(refused)?;
            let me = backend.this_user().await?;
            let box_id = backend.box_info().await?.map(|row| row.box_id);
            // D6: the request's own token, never moved here.
            match writer
                .update_item(
                    *id,
                    *expected_version,
                    changes.into_patch(me, box_id, *reason),
                )
                .await?
            {
                UpdateOutcome::Updated(head) => Ok(StoreReply::ItemWritten {
                    item: head.id,
                    outcome: ItemWrite::Edited {
                        key: head.key,
                        version: head.version,
                    },
                }),
                UpdateOutcome::Diverged { head, ancestor } => {
                    // Milestone 3 D7: the catalogue just read, with the head as its item.
                    context.item_graph = item_graph(&writer, &context.graphs, &head).await?;
                    context.item = Some(head.clone());
                    named(&writer, &mut context).await?;
                    Ok(StoreReply::ItemDiverged(Box::new(ItemDivergence {
                        head,
                        ancestor,
                        context,
                    })))
                }
            }
        }
        other => Err(StoreError::Backend(format!(
            "not an item request: {}",
            other.name()
        ))),
    }
}

/// The writer, or the refusal every item request answers offline (D2).
fn write_access(backend: &Backend) -> Result<Writer> {
    backend.writer().ok_or_else(offline)
}

/// The refusal of every item request offline, before anything is read.
fn offline() -> StoreError {
    StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())
}

/// A validator refusal as the worker answers it: its sentence, a `Constraint`.
fn refused(err: SpecError) -> StoreError {
    StoreError::Constraint(err.to_string())
}

/// The item, or `NotFound { entity: "item" }`.
async fn existing(writer: &Writer, id: ItemId) -> Result<Item> {
    writer.item(id).await?.ok_or_else(|| StoreError::NotFound {
        entity: "item",
        id: id.to_string(),
    })
}

/// One project's catalogue, with no item: kinds, the non-override graphs it may use (A3, MOD-138
/// D12), repos.
async fn catalogue(writer: &Writer, project: ProjectId) -> Result<ItemFormContext> {
    let kinds = writer.item_kinds(project).await?;
    let graphs = writer
        .usable_graphs(project)
        .await?
        .into_iter()
        .filter(|graph| !graph.is_override)
        .collect();
    let repos = writer.repos(project).await?;
    Ok(ItemFormContext {
        project,
        kinds,
        graphs,
        repos,
        item: None,
        item_graph: None,
        workspaces: WorkspaceNames::default(),
    })
}

/// MOD-141 M3 D2: fills [`ItemFormContext::workspaces`] over its `graphs` and `item_graph`, one
/// `workspace` read per distinct workspace ([`WorkspaceNames::read`]). Called once `item_graph`
/// is set, and only for a context that reaches the UI.
async fn named(writer: &Writer, context: &mut ItemFormContext) -> Result<()> {
    context.workspaces =
        WorkspaceNames::read(writer, context.graphs.iter().chain(&context.item_graph)).await?;
    Ok(())
}

/// [`ItemFormContext::item_graph`]: `item`'s own graph row when `graphs` lacks it. A graph the
/// read no longer finds is `None`, and the picker falls back to the override label.
async fn item_graph(
    writer: &Writer,
    graphs: &[StepGraph],
    item: &Item,
) -> Result<Option<StepGraph>> {
    match item.step_graph_id {
        Some(id) if !graphs.iter().any(|graph| graph.id == id) => {
            Ok(writer.graph_body(id).await?.map(|body| body.graph))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EDIT_NAME, FORM_NAME, ItemDivergence, ItemFormContext, ItemWrite, MINT_NAME, REQUEST_NAMES,
        is_item_request, item_not_in_project, mint_refused, serve,
    };
    use crate::catalogue::{WorkspaceName, WorkspaceNames};
    use crate::store_worker::{self, StoreReply, StoreRequest};
    use htui_core::fixtures::ids;
    use htui_core::model::item_spec::NOTHING_TO_SAVE;
    use htui_core::model::{
        EditReason, GraphScope, Item, ItemId, ItemPatch, ItemRevision, ItemSpec, NewRepo,
        NewStepGraph, ProjectId, RepoId, SpecChanges, StepGraph, StepGraphId, WorkspaceProject,
    };
    use htui_core::store::{MemStore, ReadStore as _, StoreError, UpdateOutcome, WriteStore as _};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};

    /// The demo store and a backend over it; clones share state.
    fn demo() -> (MemStore, Backend) {
        let store = MemStore::demo();
        (store.clone(), Backend::memory(store))
    }

    /// A15: the demo has no repos. `htui` (primary) and `web` in `htui`.
    async fn with_repos(store: &MemStore) {
        for (name, is_primary) in [("htui", true), ("web", false)] {
            store
                .create_repo(NewRepo {
                    id: RepoId::new(),
                    project_id: ids::PROJECT_HTUI,
                    name: name.to_owned(),
                    remote_url: None,
                    default_branch: "main".to_owned(),
                    is_primary,
                })
                .await
                .expect("create a repo");
        }
    }

    /// An htui `ANA` spec every rule accepts.
    fn spec(title: &str) -> ItemSpec {
        ItemSpec {
            kind_id: ids::KIND_HTUI_ANA,
            title: title.to_owned(),
            body: "Body.".to_owned(),
            priority: 0,
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            step_graph_id: None,
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    async fn form(backend: &Backend, project: ProjectId, item: Option<ItemId>) -> ItemFormContext {
        match serve(backend, &StoreRequest::ItemForm { project, item }).await {
            Ok(StoreReply::ItemForm(context)) => *context,
            other => panic!("the form read answered {other:?}"),
        }
    }

    async fn row(store: &MemStore, id: ItemId) -> Item {
        store
            .item(id)
            .await
            .expect("read")
            .expect("the item exists")
    }

    fn mint(spec: ItemSpec) -> StoreRequest {
        StoreRequest::MintItem {
            project: ids::PROJECT_HTUI,
            spec,
        }
    }

    fn edit(expected_version: i32, changes: SpecChanges) -> StoreRequest {
        StoreRequest::EditItem {
            id: ids::HTUI_ANA_2,
            expected_version,
            changes,
            reason: EditReason::Edited,
        }
    }

    /// [`edit`] once the form is rebased on a divergence's head (milestone 3 D6).
    fn resolve(expected_version: i32, changes: SpecChanges) -> StoreRequest {
        StoreRequest::EditItem {
            id: ids::HTUI_ANA_2,
            expected_version,
            changes,
            reason: EditReason::DivergenceResolution,
        }
    }

    /// Someone else's title change, written straight to the store at `version`.
    fn theirs(title: &str) -> ItemPatch {
        ItemPatch {
            title: Some(title.to_owned()),
            body: None,
            kind_id: None,
            required_tags: None,
            priority: None,
            touched_paths: None,
            step_graph_id: None,
            author_id: ids::USER,
            box_id: None,
            reason: "elsewhere".to_owned(),
        }
    }

    fn retitle(title: &str) -> SpecChanges {
        SpecChanges {
            title: Some(title.to_owned()),
            ..SpecChanges::default()
        }
    }

    #[test]
    fn request_names_match_the_name_arms() {
        let samples = [
            StoreRequest::ItemForm {
                project: ids::PROJECT_HTUI,
                item: None,
            },
            mint(spec("New")),
            edit(1, retitle("New")),
        ];
        let names: Vec<&str> = samples.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!([FORM_NAME, MINT_NAME, EDIT_NAME], REQUEST_NAMES);
        for name in REQUEST_NAMES {
            assert!(is_item_request(name), "{name}");
        }
        assert!(!is_item_request("items") && !is_item_request("mint_requirement"));
    }

    /// D3: the project's kinds, graphs and repos, through the writer; no item for a new form.
    #[tokio::test]
    async fn the_new_form_read_carries_kinds_graphs_and_repos() {
        let (store, backend) = demo();
        with_repos(&store).await;

        let context = form(&backend, ids::PROJECT_HTUI, None).await;

        assert_eq!(context.project, ids::PROJECT_HTUI);
        assert_eq!(context.item, None);
        let kinds = store.item_kinds(ids::PROJECT_HTUI).await.expect("kinds");
        let graphs = store.step_graphs(ids::PROJECT_HTUI).await.expect("graphs");
        let repos = store.repos(ids::PROJECT_HTUI).await.expect("repos");
        assert!(!kinds.is_empty() && !graphs.is_empty() && repos.len() == 2);
        assert_eq!(
            context.kinds.iter().map(|k| k.id).collect::<Vec<_>>(),
            kinds.iter().map(|k| k.id).collect::<Vec<_>>()
        );
        assert_eq!(
            context.graphs.iter().map(|g| g.id).collect::<Vec<_>>(),
            graphs.iter().map(|g| g.id).collect::<Vec<_>>()
        );
        assert_eq!(
            context.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
            repos.iter().map(|r| r.id).collect::<Vec<_>>()
        );
        assert_eq!(context.spec_context().kinds, &kinds[..]);
    }

    /// A3: an override graph is a per-item clone, never a choice of the form.
    #[tokio::test]
    async fn the_form_read_drops_override_graphs() {
        let (store, backend) = demo();
        let clone = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Project(ids::PROJECT_HTUI),
                name: "ANA-2 override".to_owned(),
                description: String::new(),
                is_override: true,
            })
            .await
            .expect("create an override graph");

        let context = form(&backend, ids::PROJECT_HTUI, None).await;

        assert!(context.graphs.iter().all(|graph| graph.id != clone.id));
        assert!(context.graphs.iter().all(|graph| !graph.is_override));
        assert!(
            context
                .graphs
                .iter()
                .any(|graph| graph.id == ids::GRAPH_HTUI_FEAT),
            "the project's own graphs stay"
        );
    }

    /// MOD-138 D12: the form offers every graph the project may use, its own first, then a
    /// workspace's that holds it, then the library's; a mint naming the library graph lands.
    #[tokio::test]
    async fn the_form_offers_library_and_workspace_graphs() {
        let (store, backend) = demo();
        let shared = |scope, name: &str| NewStepGraph {
            id: StepGraphId::new(),
            scope,
            name: name.to_owned(),
            description: String::new(),
            is_override: false,
        };
        let library = store
            .create_step_graph(shared(GraphScope::Library, "libflow"))
            .await
            .expect("create a library graph");
        let workspace = store
            .create_step_graph(shared(
                GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
                "wsflow",
            ))
            .await
            .expect("create a workspace graph");
        let elsewhere = store
            .create_step_graph(shared(
                GraphScope::Workspace(ids::WORKSPACE_GRAPHICS),
                "otherws",
            ))
            .await
            .expect("create another workspace's graph");

        let context = form(&backend, ids::PROJECT_HTUI, None).await;

        let mut expected: Vec<StepGraphId> = store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect("graphs")
            .iter()
            .filter(|graph| !graph.is_override)
            .map(|graph| graph.id)
            .collect();
        expected.extend([workspace.id, library.id]);
        assert_eq!(
            context.graphs.iter().map(|g| g.id).collect::<Vec<_>>(),
            expected,
            "the project's graphs, then the workspace's, then the library's"
        );
        assert!(context.graphs.iter().all(|g| g.id != elsewhere.id));

        let answer = serve(
            &backend,
            &mint(ItemSpec {
                step_graph_id: Some(library.id),
                ..spec("On the library graph")
            }),
        )
        .await
        .expect("the mint applies");
        let StoreReply::ItemWritten { item, .. } = answer else {
            panic!("a mint answers ItemWritten, not {answer:?}")
        };
        assert_eq!(row(&store, item).await.step_graph_id, Some(library.id));
    }

    /// `htui` joins Graphics too, and each workspace gets a graph named `flow`: the two `flow`s,
    /// Platform's then Graphics'.
    async fn two_flows(store: &MemStore) -> (StepGraph, StepGraph) {
        store
            .upsert_workspace_project(&WorkspaceProject {
                workspace_id: ids::WORKSPACE_GRAPHICS,
                project_id: ids::PROJECT_HTUI,
                position: 9,
            })
            .await
            .expect("htui joins graphics");
        let mut flows = Vec::new();
        for workspace in [ids::WORKSPACE_PLATFORM, ids::WORKSPACE_GRAPHICS] {
            flows.push(
                store
                    .create_step_graph(NewStepGraph {
                        id: StepGraphId::new(),
                        scope: GraphScope::Workspace(workspace),
                        name: "flow".to_owned(),
                        description: String::new(),
                        is_override: false,
                    })
                    .await
                    .expect("a workspace graph"),
            );
        }
        let graphics = flows.pop().expect("two flows");
        let platform = flows.pop().expect("two flows");
        (platform, graphics)
    }

    /// MOD-141 M3 D2: the form's read names the workspace of each graph it lists, so two
    /// workspaces' `flow`s read apart.
    #[tokio::test]
    async fn the_form_names_each_workspace_of_its_graphs() {
        let (store, backend) = demo();
        let (platform, graphics) = two_flows(&store).await;

        let context = form(&backend, ids::PROJECT_HTUI, None).await;

        assert_eq!(context.workspaces.len(), 2);
        assert_eq!(context.label(&platform), "flow \u{b7} workspace Platform");
        assert_eq!(context.label(&graphics), "flow \u{b7} workspace Graphics");
        let own = &context.graphs[0];
        assert_eq!(context.label(own), own.name);
    }

    /// D2: a workspace graph the item kept after its project left that workspace is named too,
    /// though `graphs` no longer lists it.
    #[tokio::test]
    async fn a_kept_item_graph_is_named_too() {
        let (store, backend) = demo();
        let kept = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Workspace(ids::WORKSPACE_PLATFORM),
                name: "wsflow".to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("a workspace graph");
        store
            .update_item(
                ids::HTUI_ANA_2,
                1,
                ItemPatch {
                    step_graph_id: Some(Some(kept.id)),
                    ..theirs("Theirs")
                },
            )
            .await
            .expect("the item takes it");
        store
            .remove_workspace_project(ids::WORKSPACE_PLATFORM, ids::PROJECT_HTUI)
            .await
            .expect("htui leaves platform");

        let context = form(&backend, ids::PROJECT_HTUI, Some(ids::HTUI_ANA_2)).await;

        let item_graph = context.item_graph.as_ref().expect("the kept graph");
        assert_eq!(item_graph.id, kept.id);
        assert_eq!(context.workspaces.len(), 1);
        assert_eq!(
            context.label(item_graph),
            "wsflow \u{b7} workspace Platform"
        );
    }

    /// D2: a divergence's catalogue names its workspaces as the form's read does.
    #[tokio::test]
    async fn a_divergence_context_names_its_workspaces() {
        let (store, backend) = demo();
        let (platform, graphics) = two_flows(&store).await;
        store
            .update_item(ids::HTUI_ANA_2, 1, theirs("Theirs"))
            .await
            .expect("their edit");

        let answer = serve(&backend, &edit(1, retitle("Mine")))
            .await
            .expect("a stale edit is an answer");

        let StoreReply::ItemDiverged(divergence) = answer else {
            panic!("a stale edit diverges, not {answer:?}")
        };
        let context = &divergence.context;
        assert_eq!(context.workspaces.len(), 2);
        assert_eq!(context.label(&platform), "flow \u{b7} workspace Platform");
        assert_eq!(context.label(&graphics), "flow \u{b7} workspace Graphics");
    }

    /// D3: an edit form carries the item fresh; its version is the token.
    #[tokio::test]
    async fn the_edit_form_read_carries_the_item_at_its_version() {
        let (store, backend) = demo();

        let context = form(&backend, ids::PROJECT_HTUI, Some(ids::HTUI_ANA_2)).await;

        let item = context.item.expect("the edit form carries the item");
        assert_eq!(item.id, ids::HTUI_ANA_2);
        assert_eq!(item.version, 1);
        assert_eq!(item, row(&store, ids::HTUI_ANA_2).await);
    }

    #[tokio::test]
    async fn an_edit_form_read_outside_its_project_is_refused() {
        let (_, backend) = demo();

        let answer = serve(
            &backend,
            &StoreRequest::ItemForm {
                project: ids::PROJECT_AGY,
                item: Some(ids::HTUI_ANA_2),
            },
        )
        .await;

        assert_eq!(
            answer.err(),
            Some(StoreError::Constraint(item_not_in_project("ANA-2")))
        );
        let unknown = ItemId::new();
        assert_eq!(
            serve(
                &backend,
                &StoreRequest::ItemForm {
                    project: ids::PROJECT_HTUI,
                    item: Some(unknown),
                },
            )
            .await
            .err(),
            Some(StoreError::NotFound {
                entity: "item",
                id: unknown.to_string(),
            })
        );
    }

    /// E9: the revision itself is pinned by conformance `mint_writes_revision_v1`.
    #[tokio::test]
    async fn a_mint_lands_the_next_key_at_version_one_by_this_user() {
        let (store, backend) = demo();
        let before = store.item_count();

        let answer = serve(&backend, &mint(spec("  A new one  ")))
            .await
            .expect("the mint applies");

        let StoreReply::ItemWritten { item, outcome } = answer else {
            panic!("a mint answers ItemWritten, not {answer:?}")
        };
        assert_eq!(
            outcome,
            ItemWrite::Minted {
                key: "ANA-3".to_owned()
            }
        );
        let minted = row(&store, item).await;
        assert_eq!(minted.key, "ANA-3");
        assert_eq!(minted.version, 1);
        assert_eq!(minted.created_by, ids::USER);
        assert_eq!(minted.title, "A new one");
        assert_eq!(minted.project_id, ids::PROJECT_HTUI);
        assert_eq!(store.item_count(), before + 1);
    }

    /// E9: `reason = edited` is pinned through the ancestor of a later, deliberately stale edit.
    #[tokio::test]
    async fn an_edit_lands_the_next_version_with_reason_edited() {
        let (store, backend) = demo();

        let answer = serve(&backend, &edit(1, retitle("Mine")))
            .await
            .expect("the edit applies");

        assert!(
            matches!(
                &answer,
                StoreReply::ItemWritten {
                    item,
                    outcome: ItemWrite::Edited { key, version: 2 },
                } if *item == ids::HTUI_ANA_2 && key == "ANA-2"
            ),
            "{answer:?}"
        );
        assert_eq!(row(&store, ids::HTUI_ANA_2).await.title, "Mine");

        let theirs = ItemPatch {
            title: Some("Theirs".to_owned()),
            body: None,
            kind_id: None,
            required_tags: None,
            priority: None,
            touched_paths: None,
            step_graph_id: None,
            author_id: ids::USER,
            box_id: None,
            reason: "elsewhere".to_owned(),
        };
        let moved = store
            .update_item(ids::HTUI_ANA_2, 2, theirs)
            .await
            .expect("their edit");
        assert!(matches!(moved, UpdateOutcome::Updated(ref head) if head.version == 3));

        let stale = serve(&backend, &edit(2, retitle("Mine again")))
            .await
            .expect("a stale edit is an answer");
        let StoreReply::ItemDiverged(divergence) = stale else {
            panic!("a stale edit diverges, not {stale:?}")
        };
        assert_eq!(divergence.ancestor.version, 2);
        assert_eq!(divergence.ancestor.reason, "edited");
        assert_eq!(divergence.ancestor.author_id, ids::USER);
        assert_eq!(divergence.ancestor.title, "Mine");
    }

    /// D6: both sides come back, and the head is not touched.
    #[tokio::test]
    async fn a_stale_edit_diverges_and_writes_nothing() {
        let (store, backend) = demo();
        serve(&backend, &edit(1, retitle("First")))
            .await
            .expect("the first edit applies");
        let head = row(&store, ids::HTUI_ANA_2).await;

        let answer = serve(&backend, &edit(1, retitle("Second")))
            .await
            .expect("a stale edit is an answer");

        let StoreReply::ItemDiverged(divergence) = answer else {
            panic!("a stale edit diverges, not {answer:?}")
        };
        let ItemDivergence {
            head: answered,
            ancestor,
            ..
        } = *divergence;
        assert_eq!(answered, head);
        assert_eq!(ancestor.version, 1);
        let after = row(&store, ids::HTUI_ANA_2).await;
        assert_eq!(after.title, "First");
        assert_eq!(after.version, 2);
        assert_eq!(after, head);
    }

    /// Milestone 3 D6: a resolution lands with its own reason, pinned through the ancestor of a
    /// later, deliberately stale edit.
    #[tokio::test]
    async fn a_resolution_lands_with_reason_divergence_resolution() {
        let (store, backend) = demo();
        let moved = store
            .update_item(ids::HTUI_ANA_2, 1, theirs("Theirs"))
            .await
            .expect("their edit");
        assert!(matches!(moved, UpdateOutcome::Updated(ref head) if head.version == 2));
        let stale = serve(&backend, &edit(1, retitle("Mine")))
            .await
            .expect("a stale edit is an answer");
        assert!(matches!(stale, StoreReply::ItemDiverged(_)), "{stale:?}");

        let answer = serve(&backend, &resolve(2, retitle("Resolved")))
            .await
            .expect("the resolution applies");
        assert!(
            matches!(
                &answer,
                StoreReply::ItemWritten {
                    outcome: ItemWrite::Edited { version: 3, .. },
                    ..
                }
            ),
            "{answer:?}"
        );

        let moved = store
            .update_item(ids::HTUI_ANA_2, 3, theirs("Theirs again"))
            .await
            .expect("their second edit");
        assert!(matches!(moved, UpdateOutcome::Updated(ref head) if head.version == 4));
        let stale = serve(&backend, &edit(3, retitle("Stale")))
            .await
            .expect("a stale edit is an answer");
        let StoreReply::ItemDiverged(divergence) = stale else {
            panic!("a stale edit diverges, not {stale:?}")
        };
        assert_eq!(divergence.ancestor.version, 3);
        assert_eq!(divergence.ancestor.reason, "divergence_resolution");
        assert_eq!(divergence.ancestor.title, "Resolved");
        assert_eq!(divergence.ancestor.author_id, ids::USER);
    }

    /// Milestone 3 D7: the divergence carries the catalogue the worker read for this edit, fresh,
    /// with the head as its item.
    #[tokio::test]
    async fn a_stale_edit_carries_the_fresh_catalogue_and_the_head() {
        let (store, backend) = demo();
        serve(&backend, &edit(1, retitle("First")))
            .await
            .expect("the first edit applies");
        let graph = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                scope: GraphScope::Project(ids::PROJECT_HTUI),
                name: "after the form".to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("create a graph");

        let answer = serve(&backend, &edit(1, retitle("Second")))
            .await
            .expect("a stale edit is an answer");

        let StoreReply::ItemDiverged(d) = answer else {
            panic!("a stale edit diverges, not {answer:?}")
        };
        assert_eq!(d.context.project, ids::PROJECT_HTUI);
        assert_eq!(d.context.item.as_ref(), Some(&d.head));
        assert!(d.context.graphs.iter().any(|g| g.id == graph.id));
        assert!(d.context.graphs.iter().all(|g| !g.is_override));
        let kinds = store.item_kinds(ids::PROJECT_HTUI).await.expect("kinds");
        assert_eq!(
            d.context.kinds.iter().map(|k| k.id).collect::<Vec<_>>(),
            kinds.iter().map(|k| k.id).collect::<Vec<_>>()
        );
    }

    /// D5: both stores would bump `version` on an all-`None` patch.
    #[tokio::test]
    async fn an_empty_edit_is_refused_and_writes_nothing() {
        let (store, backend) = demo();

        let answer = store_worker::serve(&backend, &edit(1, SpecChanges::default())).await;

        match answer {
            StoreReply::Failed { request, message } => {
                assert_eq!(request, EDIT_NAME);
                assert!(message.ends_with(NOTHING_TO_SAVE), "{message}");
            }
            other => panic!("an empty edit is refused, not {other:?}"),
        }
        assert_eq!(row(&store, ids::HTUI_ANA_2).await.version, 1);
    }

    #[tokio::test]
    async fn a_mint_canonicalises_its_paths() {
        let (store, backend) = demo();
        with_repos(&store).await;

        let answer = serve(
            &backend,
            &mint(ItemSpec {
                touched_paths: strings(&[" web:src/** ", "src/**", "web:src/**"]),
                ..spec("Paths")
            }),
        )
        .await
        .expect("the mint applies");

        let StoreReply::ItemWritten { item, .. } = answer else {
            panic!("a mint answers ItemWritten, not {answer:?}")
        };
        assert_eq!(
            row(&store, item).await.touched_paths,
            strings(&["web:src/**", "src/**"])
        );
    }

    /// D4 through `store_worker::serve`, so the or-arm is covered: each refusal is a `Failed`
    /// naming its request and the entry, and nothing is written.
    #[tokio::test]
    async fn every_spec_refusal_fails_naming_the_entry_and_writes_nothing() {
        let (store, backend) = demo();
        with_repos(&store).await;
        let count = store.item_count();
        let cases = [
            (mint(spec("   ")), "title"),
            (
                mint(ItemSpec {
                    required_tags: strings(&["Rust"]),
                    ..spec("Tags")
                }),
                "`Rust`",
            ),
            (
                mint(ItemSpec {
                    kind_id: ids::KIND_AGY_FEAT,
                    ..spec("Kind")
                }),
                "kind",
            ),
            (
                mint(ItemSpec {
                    step_graph_id: Some(ids::GRAPH_AGY_FEAT),
                    ..spec("Graph")
                }),
                "step graph",
            ),
            (
                mint(ItemSpec {
                    touched_paths: strings(&["nope:src"]),
                    ..spec("Repo")
                }),
                "`nope`",
            ),
            (
                mint(ItemSpec {
                    touched_paths: strings(&["web: src"]),
                    ..spec("Space")
                }),
                "`web: src`",
            ),
            (
                edit(
                    1,
                    SpecChanges {
                        touched_paths: Some(strings(&["nope:src"])),
                        ..SpecChanges::default()
                    },
                ),
                "`nope`",
            ),
        ];
        for (request, named) in &cases {
            match store_worker::serve(&backend, request).await {
                StoreReply::Failed {
                    request: name,
                    message,
                } => {
                    assert_eq!(name, request.name());
                    assert!(message.contains(named), "{named}: {message}");
                    assert!(message.starts_with("constraint violated: "), "{message}");
                }
                other => panic!("{named}: refused, not {other:?}"),
            }
            assert_eq!(store.item_count(), count, "{named}");
            assert_eq!(row(&store, ids::HTUI_ANA_2).await.version, 1, "{named}");
        }
    }

    /// D2: the writer is taken before anything is read, so an offline box never opens a form.
    #[tokio::test]
    async fn offline_every_item_request_is_refused_before_anything_is_read() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "item-writes-offline", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline { cache, since: None };
        let requests = [
            StoreRequest::ItemForm {
                project: ids::PROJECT_HTUI,
                item: Some(ids::HTUI_ANA_2),
            },
            mint(spec("New")),
            edit(1, retitle("New")),
        ];
        for request in &requests {
            assert_eq!(
                serve(&backend, request).await.err(),
                Some(StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned())),
                "{}",
                request.name()
            );
            match store_worker::serve(&backend, request).await {
                StoreReply::Failed {
                    request: name,
                    message,
                } => {
                    assert_eq!(name, request.name());
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("an offline {} is refused, not {other:?}", request.name()),
            }
        }
    }

    /// A9, E6: `StoreRequest` derives `Debug`; the body and the paths print as lengths.
    #[test]
    fn mint_and_edit_debug_print_no_body_and_no_paths() {
        let secret = ItemSpec {
            body: "SECRET-BODY".to_owned(),
            touched_paths: strings(&["secret/dir/**"]),
            ..spec("Visible title")
        };
        for request in [mint(secret.clone()), edit(1, SpecChanges::from(secret))] {
            for printed in [format!("{request:?}"), format!("{request:#?}")] {
                assert!(!printed.contains("SECRET-BODY"), "{printed}");
                assert!(!printed.contains("secret/dir"), "{printed}");
                assert!(printed.contains("Visible title"), "{printed}");
            }
        }
    }

    /// Review L2: the form's read and a divergence carry whole items; their replies print ids,
    /// keys, versions and lengths, never a body or a path.
    #[tokio::test]
    async fn form_and_divergence_debug_print_no_body_and_no_paths() {
        let (store, _) = demo();
        let mut item = store
            .item(ids::HTUI_ANA_2)
            .await
            .expect("read")
            .expect("the item");
        item.body = "SECRET-BODY".to_owned();
        item.touched_paths = strings(&["secret/dir/**"]);
        let ancestor = ItemRevision {
            item_id: item.id,
            version: 1,
            title: item.title.clone(),
            body: "SECRET-ANCESTOR".to_owned(),
            required_tags: Vec::new(),
            author_id: ids::USER,
            box_id: None,
            reason: "created".to_owned(),
            created_at: item.created_at,
        };
        let context = ItemFormContext {
            project: item.project_id,
            kinds: Vec::new(),
            graphs: Vec::new(),
            repos: Vec::new(),
            item: Some(item.clone()),
            item_graph: None,
            workspaces: WorkspaceNames::new([WorkspaceName {
                id: ids::WORKSPACE_PLATFORM,
                name: "SECRET-WORKSPACE".to_owned(),
                slug: "secret-slug".to_owned(),
            }]),
        };
        let divergence = ItemDivergence {
            head: item.clone(),
            ancestor,
            context: context.clone(),
        };
        for reply in [
            StoreReply::ItemForm(Box::new(context)),
            StoreReply::ItemDiverged(Box::new(divergence)),
        ] {
            let diverged = matches!(reply, StoreReply::ItemDiverged(_));
            for printed in [format!("{reply:?}"), format!("{reply:#?}")] {
                for secret in [
                    "SECRET-BODY",
                    "SECRET-ANCESTOR",
                    "secret/dir",
                    "SECRET-WORKSPACE",
                    "secret-slug",
                ] {
                    assert!(!printed.contains(secret), "{secret}: {printed}");
                }
                assert!(printed.contains(&item.key), "{printed}");
                // MOD-141 M3 D2: the workspace names print as a count.
                assert!(printed.contains("workspaces: 1"), "{printed}");
                if diverged {
                    // Milestone 3 D7: the catalogue prints, as counts.
                    assert!(printed.contains("kinds"), "{printed}");
                }
            }
        }
    }

    #[tokio::test]
    async fn a_request_that_is_not_an_item_one_is_named() {
        let (_, backend) = demo();

        assert_eq!(
            serve(&backend, &StoreRequest::BoxInfo).await.err(),
            Some(StoreError::Backend(
                "not an item request: box_info".to_owned()
            ))
        );
    }

    /// §10.1 (MOD-59 re-review L2): a refusal given before the insert, as the worker renders it
    /// into `Failed`, is told from a store failure that may follow a lost COMMIT.
    #[tokio::test]
    async fn mint_refused_tells_a_refusal_from_a_store_failure() {
        let (_, backend) = demo();
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "item-writes-mint-refused", 1)
            .await
            .expect("open a throwaway mirror");
        let offline = Backend::Offline { cache, since: None };
        let refusals = [
            ("a spec refusal", &backend, mint(spec("  "))),
            ("nothing to save", &backend, edit(1, SpecChanges::default())),
            ("offline", &offline, mint(spec("New"))),
        ];
        for (why, backend, request) in refusals {
            let StoreReply::Failed { message, .. } = store_worker::serve(backend, &request).await
            else {
                panic!("{why}: the request is refused")
            };
            assert!(mint_refused(&message), "{why}: {message}");
        }

        for failure in [
            StoreError::Backend("connection reset by peer".to_owned()),
            StoreError::Unreachable("connection reset".to_owned()),
            StoreError::NotFound {
                entity: "item",
                id: "x".to_owned(),
            },
        ] {
            assert!(!mint_refused(&failure.to_string()), "{failure}");
        }
    }
}
