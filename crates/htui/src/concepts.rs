//! `--index-items` and `--search-items`: the concepts index from the command line (MOD-34 D3,
//! `R-STO-8`).
//!
//! Both run before any terminal work, like `--set-dsn`, print to the shell and exit. Neither
//! touches the TUI's start path: a missing Qdrant URL, a Qdrant that does not answer or a
//! Postgres that does not answer is one line on stderr and a non-zero exit, and nothing else is
//! affected (`docs/ANA-19.md` §2 invariant 3). The automatic sync belongs to the headless worker
//! (MOD-41) and the agent-facing tool to the MCP server (MOD-11).
//!
//! `--index-items` connects headless (`PgStore::connect_headless`, MOD-40 plan D8): a pending
//! schema or a build below the database's target version is refused, never migrated.
//! `--search-items` reads only (MOD-92): `PgStore::connect_read_only` and
//! `vector::ExistingCollection` (the collection before the model), with the same refusals, and
//! answers through `state_cmd` with exit 3 when Postgres or Qdrant does not answer, 2 on a refusal
//! and 1 on a failure, as lines or (`--json`) one document.
//!
//! Both work per workspace (MOD-159 M1): each workspace has its own Qdrant collection
//! (`htui_ws_<uuid>`) and there is one global one (`htui_global`). `--index-items` indexes a
//! collection only under its `indexer_lease` row, and says which box holds one it cannot take;
//! `--search-items` reads each workspace's collection and the global one, and names a workspace
//! whose collection is missing or refused.
//!
//! The background sync (MOD-41 plan D19, MOD-158 D7, MOD-159 M1 plan D7) lives here too:
//! [`IndexJob`] runs [`index_job`] in `htui worker` and in the TUI while it is online. Every beat
//! (the run lease's refresh) it resolves the Qdrant URL, takes or renews the lease of every
//! collection, and reconciles each one it holds at start and then every [`sync_interval`].
//! Between reconciles (MOD-159 M2) it indexes, every [`INCREMENTAL_CYCLE`], only the rows written
//! since its last pass of that collection, pulled back by [`overlap_window`].
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use chrono::{DateTime, TimeDelta, Utc};
use htui_core::model::{
    BoxId, IndexChanges, IndexCursor, IndexedCollection, IndexerLease, LeaseTake, LeaseTurn,
    ProjectId, RequirementState, Resolution, Scope,
};
use htui_core::store::{
    IndexChangesStore, IndexerLeaseStore, MemStore, ReadStore, StoreError, take_or_request_pass,
};
use htui_orch::recover::LeaseTimes;
use htui_store::embed::DenseEmbedder;
use htui_store::embed::RtenEmbedder;
use htui_store::model;
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::secret::QdrantEnv;
use htui_store::vector::{
    self, EMBEDDER_MISMATCH, ExistingCollection, Hit, LEGACY_COLLECTION, PointType, QdrantStore,
    SearchQuery, VectorStore, is_embedder_mismatch, is_qdrant_refused, is_query_embed_failure,
};
use htui_store::vector_sync::{DeleteGuard, Indexer, SyncReport, workspace_search_scope};
use htui_store::{HeadlessError, Identity, PgStore, identity, secret};
use serde_json::Value;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::concepts_worker::{self, KeySource, QdrantIndex, merge_hits};
use crate::state_cmd::{
    self, Answer, ErrorCode, Failure, IndexAnswer, Kind, Outcome, SearchAnswer, StateExit,
};

/// Options of `--search-items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The query text.
    pub query: String,
    /// Restrict to one project, by slug.
    pub project: Option<String>,
    /// Decisions only (ANA-11 §4.2): items closed as [`DECISION_RESOLUTIONS`], and their documents.
    pub decisions: bool,
    /// Most hits printed.
    pub limit: u64,
    /// `--cwd`: only the project whose repository, registered on this box, holds this path (MOD-92).
    pub cwd: Option<PathBuf>,
    /// `--json`: one versioned document instead of lines (MOD-92).
    pub json: bool,
}

/// Hits `--search-items` prints when `--limit` is not given.
pub const DEFAULT_LIMIT: u64 = 10;

/// The resolutions `--decisions` keeps (MOD-50 D223, maintainer): an item closed as withdrawn
/// ("dropped without a decision"), duplicate or superseded records no decision in force, and a
/// done item that was never closed out has none yet. Requirements carry no resolution, so they are
/// left out too.
pub const DECISION_RESOLUTIONS: [Resolution; 3] = [
    Resolution::Done,
    Resolution::Concluded,
    Resolution::Rejected,
];

/// The search both front ends send (MOD-64 D239): `text` in `projects`, narrowed to decisions (items
/// closed as one of [`DECISION_RESOLUTIONS`], and their documents) when `decisions` is set, at most
/// `limit` hits. No type or status filter: `--search-items` has none either.
#[must_use]
pub fn query(text: &str, projects: Vec<ProjectId>, decisions: bool, limit: u64) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        projects,
        types: Vec::new(),
        statuses: Vec::new(),
        resolutions: if decisions {
            DECISION_RESOLUTIONS.to_vec()
        } else {
            Vec::new()
        },
        limit,
    }
}

/// The sentence a search or an index run refuses with when no Qdrant URL is set (MOD-158 D3: no
/// `HTUI_QDRANT_URL` override and no `qdrant_url` row).
pub(crate) const NO_QDRANT_URL: &str =
    "no Qdrant URL is set; set one in Settings > Qdrant or with `htui --set-qdrant-url`";

/// The sentence every verb that needs the keyring's DSN refuses with when none is stored.
pub(crate) const NO_DSN: &str = "no Postgres DSN is stored; run `htui --set-dsn` first";

/// `--index-items`' connect: headless, because it writes the index by design. `--search-items`
/// does not come here (MOD-92: it reads only). MOD-158 D8's order: the DSN, the connect, the
/// Qdrant URL (the override or the row) and key. The model and Qdrant come after, per collection
/// and only for one whose lease the run takes ([`CliIndex`], MOD-159 M1 plan D12).
async fn open() -> anyhow::Result<(PgStore, QdrantSettings)> {
    let Some(dsn) = secret::get_dsn()? else {
        bail!("{NO_DSN}");
    };
    let identity = identity::load_or_mint(&identity::config_root()?)?;
    let pg = PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT, PoolSize::TUI)
        .await
        .map_err(headless_refusal)?;
    let settings = concepts_worker::resolve_settings(
        &QdrantEnv::from_process(),
        Some(&pg),
        &KeySource::Interactive,
        secret::get_qdrant_api_key,
    )
    .await
    .map_err(|why| anyhow::anyhow!("{why}"))?;
    Ok((pg, settings))
}

/// `--index-items`' [`JobIndex`] (MOD-159 M1 plan D12): the model loaded when the first collection
/// is opened, so a run that takes no lease loads none, and each collection connected, and so
/// created, with it.
#[derive(Default)]
struct CliIndex {
    /// The model, once a collection needed it.
    embedder: tokio::sync::OnceCell<RtenEmbedder>,
}

impl JobIndex for CliIndex {
    type Store = QdrantStore<RtenEmbedder>;

    async fn open(
        &self,
        settings: QdrantSettings,
        collection: &IndexedCollection,
    ) -> Result<Arc<Self::Store>, String> {
        let embedder = self
            .embedder
            .get_or_try_init(load_embedder)
            .await
            .map_err(|err| format!("{err:#}"))?;
        QdrantStore::connect(&settings, embedder.clone(), collection)
            .await
            .map(Arc::new)
            .map_err(|err| format!("{:#}", qdrant_refusal(err)))
    }

    fn forget(&self, _: Option<&Arc<Self::Store>>) {}

    fn forget_synced(&self) {}

    async fn drop_legacy(&self, settings: &QdrantSettings) -> Result<bool, String> {
        drop_legacy(settings).await
    }
}

/// MOD-159 M1 plan D10: drops the one shared collection every htui wrote before each workspace
/// had its own ([`LEGACY_COLLECTION`]); `true` when it existed.
async fn drop_legacy(settings: &QdrantSettings) -> Result<bool, String> {
    vector::drop_collection_named(settings, LEGACY_COLLECTION)
        .await
        .map_err(|err| format!("{:#}", qdrant_refusal(err)))
}

/// The embedding model, downloaded first if this box has none. Said on stderr only when a download
/// happens: these commands' log goes nowhere without `--log`, and 133 MB is otherwise a silent
/// pause (review L4); stdout stays the command's.
async fn load_embedder() -> anyhow::Result<RtenEmbedder> {
    let files = model::ensure_model_noting(|line| eprintln!("{line}"))
        .await
        .context("cannot load the embedding model")?;
    tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
        .await
        .context("the embedding model's loader stopped")?
        .context("cannot load the embedding model")
}

/// A failed `QdrantStore::connect` as the error a user reads, at all three sites that connect
/// (review L3): "cannot reach Qdrant" above everything but an embedder mismatch and a refused API
/// key (review R1 M2), which Qdrant answered and which name their own remedy.
pub(crate) fn qdrant_refusal(err: StoreError) -> anyhow::Error {
    if is_embedder_mismatch(&err) || is_qdrant_refused(&err) {
        anyhow::Error::new(err)
    } else {
        anyhow::Error::new(err).context("cannot reach Qdrant")
    }
}

/// A headless connect's refusal as the line a user reads (MOD-40 plan D8's sentences, byte for
/// byte). `--index-items` exits 1 with it, `htui worker` exits 2 (MOD-41 plan D14).
pub fn headless_refusal(err: HeadlessError) -> anyhow::Error {
    match err {
        HeadlessError::MigrationsPending(n) => {
            anyhow::anyhow!("{n} schema migration(s) are pending; start `htui` once to apply them")
        }
        HeadlessError::Store(err) => {
            let context = store_context(&err);
            anyhow::Error::new(err).context(context)
        }
        below @ HeadlessError::BelowTarget { .. } => below.into(),
    }
}

/// The line above a headless connect's store error: only an unreachable server is "cannot
/// reach"; a newer or drifted schema, a partial version or a malformed target was reached and
/// refused this build (MOD-40 plan D8).
fn store_context(err: &StoreError) -> &'static str {
    match err {
        StoreError::Unreachable(_) => "cannot reach Postgres",
        _ => "Postgres refused this htui",
    }
}

/// The scopes `--search-items`, `--index-items` and the worker's index job cover (MOD-147 M4 D7).
///
/// With no `project`, every workspace's scope widened through [`workspace_search_scope`]: its
/// scope row first, then its members by position, so the workspace's own `WS` items and
/// documents are indexed and found. A workspace with neither members nor a scope row is left out.
/// `@<workspace-slug>` narrows to that workspace's scope row alone; any other slug narrows to
/// the member projects of that slug. A slug that matches nothing is an error, naming a workspace
/// for an `@` slug and a project otherwise, which `search_projects` answers as `unknown_project`.
pub(crate) async fn scopes(pg: &PgStore, project: Option<&str>) -> anyhow::Result<Vec<Scope>> {
    Ok(scopes_named(pg, project)
        .await?
        .into_iter()
        .map(|(_, scope)| scope)
        .collect())
}

/// [`scopes`], each with its workspace's slug: one collection per workspace (MOD-159 M1 plan D1),
/// so `--index-items` and `--search-items` name the workspace a collection belongs to.
pub(crate) async fn scopes_named(
    pg: &PgStore,
    project: Option<&str>,
) -> anyhow::Result<Vec<(String, Scope)>> {
    let mut scopes = Vec::new();
    for ws in pg.workspaces().await? {
        let scope = match project {
            None => workspace_search_scope(pg, &Scope::from_workspace(&ws)).await,
            Some(slug) => match slug.strip_prefix('@') {
                Some(workspace) if workspace == ws.slug => Scope {
                    workspace_id: ws.workspace_id,
                    project_ids: pg
                        .scope_project(ws.workspace_id)
                        .await?
                        .into_iter()
                        .collect(),
                },
                Some(_) => continue,
                None => {
                    let mut scope = Scope::from_workspace(&ws);
                    let wanted: Vec<ProjectId> = ws
                        .projects
                        .iter()
                        .filter(|p| p.slug == slug)
                        .map(|p| p.project_id)
                        .collect();
                    scope.project_ids.retain(|id| wanted.contains(id));
                    scope
                }
            },
        };
        if !scope.project_ids.is_empty() {
            scopes.push((ws.slug.clone(), scope));
        }
    }
    if let Some(slug) = project
        && scopes.is_empty()
    {
        // MOD-147 M4 R1 L3: an `@` slug names a workspace; the code stays `unknown_project`.
        match slug.strip_prefix('@') {
            Some(workspace) => bail!("no workspace with slug `{workspace}`"),
            None => bail!("no project with slug `{slug}`"),
        }
    }
    Ok(scopes)
}

/// `--index-items`: brings each workspace's collection in step with its projects (or one
/// project's), under that collection's lease, then reports (MOD-159 M1 plan D12). A workspace
/// whose lease another box holds is skipped: the run asks the holder for a pass (MOD-159 M3 D4)
/// and says so in one line on stderr naming its box; the run still exits 0. Every lease the run
/// took is let go at the end.
///
/// # Errors
///
/// Missing settings, an unreachable Postgres or Qdrant, a model that cannot load.
pub async fn index_items(project: Option<&str>) -> anyhow::Result<()> {
    let (pg, settings) = open().await?;
    let named = scopes_named(&pg, project).await?;
    let holder = Holder {
        box_id: pg.this_box(),
        // D5: this run's own token, so nothing it takes is ever another process's `Ours`.
        owner: Uuid::now_v7(),
        ttl: LeaseTimes::from_app(&pg.app_settings().await?).ttl,
    };
    let report = index_under_leases(
        &pg,
        &CliIndex::default(),
        &settings,
        &named,
        holder,
        &mut |line| eprintln!("{line}"),
    )
    .await;
    if let Err(err) = pg.release_indexer_leases(holder.owner).await {
        tracing::warn!(%err, "the index run's leases were not let go; they lapse");
    }
    eprintln!("{}", report_line(&report?));
    Ok(())
}

/// What an index run whose lease another box took over mid-sync says (plan D6): it stopped there,
/// nothing deleted. `--index-items` names the workspace before it; the overlay says it alone.
pub(crate) const TAKEN_OVER: &str = "another box took the indexing over; stopped here";

/// Who takes a lease, and for how long: the box, the process's token (plan D5), the TTL.
#[derive(Debug, Clone, Copy)]
pub struct Holder {
    /// The box the lease names.
    pub box_id: BoxId,
    /// The token the lease is taken, renewed and released with.
    pub owner: Uuid,
    /// The TTL each take and renewal stamps.
    pub ttl: TimeDelta,
}

/// The sentence for a workspace whose collection another box indexes (plan D12), when no pass
/// request could be recorded ([`LeaseTurn::Held`]).
fn held_note(slug: &str, lease: &IndexerLease) -> String {
    format!(
        "workspace {slug}: indexing runs on {}; not indexed here",
        holder_name(lease)
    )
}

/// The sentence for a workspace whose collection another box indexes and was asked to reindex
/// (MOD-159 M3 D6): [`held_note`]'s box, through [`holder_name`].
fn requested_note(slug: &str, lease: &IndexerLease) -> String {
    format!(
        "workspace {slug}: indexing runs on {}; asked it to reindex",
        holder_name(lease)
    )
}

/// The box a lease names, as a person reads it: its hostname, or the last eight hex digits of its
/// id when its row is gone, stripped of control characters. The CLI's note and the overlay's
/// `held_line` both say it (MOD-159 M1 R2: one rule, written once).
pub(crate) fn holder_name(lease: &IndexerLease) -> String {
    let name = lease.box_name.clone().unwrap_or_else(|| {
        lease.box_id.map_or_else(
            || "another box".to_owned(),
            |id| {
                let text = id.as_uuid().simple().to_string();
                text[text.len().saturating_sub(8)..].to_owned()
            },
        )
    });
    name.chars().filter(|c| !c.is_control()).collect()
}

/// `--index-items`' body over `store` and `index` (plan D12, D10), the test seam. Per workspace in
/// `scopes`, its collection's lease, through [`take_or_request_pass`] (MOD-159 M3 D4): held live
/// by another owner, the run asks the holder for a pass, `note` says so (or, when nothing could
/// be recorded, that the holder indexes it) and the workspace is skipped; otherwise the
/// collection is opened and synced, the lease renewed before each delete batch. Then the global
/// collection's lease (taken, never requested: it has nothing to sync): when taken, the
/// collection is opened (created) and the legacy collection dropped. Leases are not let go here:
/// the caller does.
///
/// # Errors
///
/// The first store, lease or index failure; later workspaces are not synced.
pub async fn index_under_leases<S, I>(
    store: &S,
    index: &I,
    settings: &QdrantSettings,
    scopes: &[(String, Scope)],
    holder: Holder,
    note: &mut impl FnMut(String),
) -> anyhow::Result<SyncReport>
where
    S: ReadStore + IndexerLeaseStore,
    I: JobIndex,
{
    let mut report = SyncReport::default();
    for (slug, scope) in scopes {
        let collection = IndexedCollection::Workspace(scope.workspace_id);
        let turn =
            take_or_request_pass(store, &collection, holder.box_id, holder.owner, holder.ttl)
                .await?;
        match turn {
            LeaseTurn::Ours => {}
            LeaseTurn::Requested(lease) => {
                note(requested_note(slug, &lease));
                continue;
            }
            LeaseTurn::Held(lease) => {
                note(held_note(slug, &lease));
                continue;
            }
        }
        let vectors = index
            .open(settings.clone(), &collection)
            .await
            .map_err(anyhow::Error::msg)?;
        let guard = Renewing {
            leases: store,
            collection,
            holder,
        };
        let one = Indexer::sync(store, scope, &*vectors, &guard).await?;
        if one.abandoned {
            note(format!("workspace {slug}: {TAKEN_OVER}"));
        }
        report += one;
    }
    let global = IndexedCollection::Global;
    let take = store
        .take_indexer_lease(&global, holder.box_id, holder.owner, holder.ttl)
        .await?;
    if !matches!(take, LeaseTake::Held(_)) {
        index
            .open(settings.clone(), &global)
            .await
            .map_err(anyhow::Error::msg)?;
        if index
            .drop_legacy(settings)
            .await
            .map_err(anyhow::Error::msg)?
        {
            note(format!(
                "dropped the old shared collection `{LEGACY_COLLECTION}`"
            ));
        }
    }
    Ok(report)
}

/// D6's guard under a lease: renews `collection`'s lease for `holder` before each delete batch;
/// a renewal that touches nothing (another owner took it) stops the sync.
struct Renewing<'a, L> {
    /// Where the lease lives.
    leases: &'a L,
    /// The collection whose lease is renewed.
    collection: IndexedCollection,
    /// The token and TTL.
    holder: Holder,
}

impl<L: IndexerLeaseStore> DeleteGuard for Renewing<'_, L> {
    async fn before_delete(&self) -> Result<bool, StoreError> {
        self.leases
            .renew_indexer_lease(&self.collection, self.holder.owner, self.holder.ttl)
            .await
    }
}

/// The `app_setting` key of the worker's index interval (MOD-41 plan D19): a positive JSON
/// integer of minutes.
pub const SYNC_MINUTES_KEY: &str = "concepts_sync_minutes";

/// The interval, in minutes, when [`SYNC_MINUTES_KEY`] is absent, non-positive or not an integer.
pub const DEFAULT_SYNC_MINUTES: u64 = 15;

/// The worker's sleep between two syncs, from [`SYNC_MINUTES_KEY`] in `app`, re-read each cycle
/// (the rule of `htui-orch`'s lease times): absent, `null`, a string, a bool, a fraction, zero
/// and a negative all mean [`DEFAULT_SYNC_MINUTES`].
#[must_use]
pub fn sync_interval(app: &BTreeMap<String, Value>) -> Duration {
    let minutes = app
        .get(SYNC_MINUTES_KEY)
        .and_then(Value::as_u64)
        .filter(|minutes| *minutes > 0)
        .unwrap_or(DEFAULT_SYNC_MINUTES);
    Duration::from_secs(minutes.saturating_mul(60))
}

/// How often the index job indexes what was written since its last pass of a held collection,
/// between two full reconciles (MOD-159 M2 plan D6, ANA-37 Q4). A constant, not a setting; revisit
/// with MOD-46 (NOTIFY).
pub const INCREMENTAL_CYCLE: Duration = Duration::from_secs(60);

/// The `app_setting` key of the overlap an incremental pass reads back behind its cursor (MOD-159
/// M2 plan D5): the cache refresh's own, a positive JSON integer of seconds.
pub const OVERLAP_KEY: &str = "cache_overlap_seconds";

/// The overlap, in seconds, when [`OVERLAP_KEY`] is absent, non-positive or not an integer.
pub const DEFAULT_OVERLAP_SECONDS: u64 = 300;

/// How far behind its cursor an incremental pass reads, from [`OVERLAP_KEY`] in `app`, re-read
/// each beat (the rule of `htui_store::connect`'s refresh settings): absent, `null`, a string, a
/// bool, a fraction, zero and a negative all mean [`DEFAULT_OVERLAP_SECONDS`]. A row committed
/// later than its stamp by less than this, or a document whose writer's clock lags the store's by
/// less, is still read; anything later is the full reconcile's.
#[must_use]
pub fn overlap_window(app: &BTreeMap<String, Value>) -> Duration {
    let seconds = app
        .get(OVERLAP_KEY)
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_OVERLAP_SECONDS);
    Duration::from_secs(seconds)
}

/// What the worker's index job reads besides [`ReadStore`] and the change read of its incremental
/// passes ([`IndexChangesStore`], MOD-159 M2 plan D10): both are inherent on the stores, not on a
/// trait (MOD-41 plan D19).
#[allow(async_fn_in_trait, reason = "no dyn IndexSource is formed")]
pub trait IndexSource: ReadStore + IndexChangesStore {
    /// Every workspace's scope, its scope row first and then its members (MOD-147 M4 D7), a
    /// workspace with neither left out.
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>>;
    /// The `app_setting` map, [`sync_interval`]'s and the lease times' input.
    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError>;
    /// The box a lease taken through this store names (MOD-159 M1 plan D5): Postgres's registered
    /// box, the memory store's own; `None` when there is none.
    async fn index_box(&self) -> Option<BoxId>;
}

impl IndexSource for PgStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        scopes(self, None).await
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }

    async fn index_box(&self) -> Option<BoxId> {
        Some(self.this_box())
    }
}

impl IndexSource for MemStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        let mut scopes = Vec::new();
        for ws in self.workspaces().await? {
            let scope = workspace_search_scope(self, &Scope::from_workspace(&ws)).await;
            if !scope.project_ids.is_empty() {
                scopes.push(scope);
            }
        }
        Ok(scopes)
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }

    async fn index_box(&self) -> Option<BoxId> {
        self.box_info().await.ok().flatten().map(|info| info.box_id)
    }
}

/// What the index job does to Qdrant besides syncing (MOD-159 M1 plan D7): open a collection's
/// store (creating the collection, so loading the model), forget a connection that failed, and drop
/// the legacy collection. [`QdrantIndex`] in the worker and the TUI, `CliIndex` for
/// `--index-items`, a fake in tests.
#[allow(async_fn_in_trait, reason = "no dyn JobIndex is formed")]
pub trait JobIndex {
    /// One collection's store.
    type Store: VectorStore;
    /// `collection` at `settings`, created when there is none.
    ///
    /// # Errors
    ///
    /// The sentence a user reads: an unreachable or refusing Qdrant, a model that cannot load.
    async fn open(
        &self,
        settings: QdrantSettings,
        collection: &IndexedCollection,
    ) -> Result<Arc<Self::Store>, String>;
    /// `Some(store)` after a sync on `store` failed, so the next one reconnects; `None` when the
    /// settings stopped resolving, so no connection, and no key it was made with, outlives them
    /// (MOD-158 R1, B-L1).
    fn forget(&self, store: Option<&Arc<Self::Store>>);
    /// The key stopped resolving while the URL still does (MOD-159 M1 R2): the connections a sync
    /// made, and the key they hold, go; a search's read-only ones stay, since each search resolves
    /// its settings, key included, itself.
    fn forget_synced(&self);
    /// Drops the legacy collection (plan D10); `true` when it existed.
    ///
    /// # Errors
    ///
    /// The sentence a user reads.
    async fn drop_legacy(&self, settings: &QdrantSettings) -> Result<bool, String>;
}

impl JobIndex for QdrantIndex {
    type Store = QdrantStore<RtenEmbedder>;

    async fn open(
        &self,
        settings: QdrantSettings,
        collection: &IndexedCollection,
    ) -> Result<Arc<Self::Store>, String> {
        self.connect(settings, collection).await
    }

    fn forget(&self, store: Option<&Arc<Self::Store>>) {
        Self::forget(self, store);
    }

    fn forget_synced(&self) {
        Self::forget_synced(self);
    }

    async fn drop_legacy(&self, settings: &QdrantSettings) -> Result<bool, String> {
        drop_legacy(settings).await
    }
}

/// One collection the index job holds (MOD-159 M2 plan D8).
#[derive(Debug, Clone, Copy)]
struct Held {
    /// When its last full reconcile started (M1's value).
    reconciled: tokio::time::Instant,
    /// When its last pass, full or incremental, started.
    passed: tokio::time::Instant,
    /// Where its last completed pass began, on the store's clock: `None` until a full reconcile
    /// of its workspace completes (not abandoned, no error), and always for the global collection.
    cursor: Option<IndexCursor>,
}

impl Held {
    /// When its next incremental pass is due, from now: `None` without a cursor.
    fn incremental_in(&self) -> Option<Duration> {
        self.cursor?;
        Some(INCREMENTAL_CYCLE.saturating_sub(self.passed.elapsed()))
    }
}

/// What the index job remembers from one beat to the next (MOD-159 M1 plan D7, M2 plan D8).
#[derive(Debug, Default)]
pub struct JobState {
    /// Each collection this job holds, when its last reconcile and pass started, and its cursor.
    /// Forgotten when another owner holds the lease, when a sync was abandoned, when the settings
    /// stop resolving, when the key cannot be read and when the URL changes, so the next take
    /// reconciles at once (MOD-159 M2 D1: the cursor goes with it).
    held: HashMap<IndexedCollection, Held>,
    /// The settings, key included, the last due full reconcile resolved (MOD-159 M2 plan D8): an
    /// incremental pass opens with them, so a 60 s cycle never reads the keyring. Cleared when the
    /// settings stop resolving or the key cannot be read; replaced by each key read.
    keyed: Option<QdrantSettings>,
    /// The collections whose failed incremental pass was logged at `warn`, each once until a pass
    /// of it succeeds (MOD-159 M2 plan D9).
    incremental_failed: HashSet<IndexedCollection>,
    /// The collections whose embedder mismatch was logged at `error` (review L3), each once
    /// until it opens again (MOD-159 M1 R2).
    mismatch: HashSet<IndexedCollection>,
    /// The legacy collection was dropped, or found absent, by this process (plan D10).
    legacy_checked: bool,
    /// "No Qdrant URL" was logged since the URL was last set.
    no_url_said: bool,
    /// The sentence of the last key failure logged, until the key reads again (MOD-159 M1 R2).
    key_said: Option<String>,
    /// When the key last could not be read, until it reads again: the job then holds no lease,
    /// and takes none until [`sync_interval`] has passed since (MOD-159 M1 R2 review).
    key_failed_at: Option<tokio::time::Instant>,
}

impl JobState {
    /// The collections this job holds, in name order.
    #[must_use]
    pub fn held(&self) -> Vec<IndexedCollection> {
        let mut held: Vec<_> = self.held.keys().copied().collect();
        held.sort_by_key(IndexedCollection::name);
        held
    }
}

/// One beat of the index job (MOD-159 M1 plan D7); answers how long to sleep before the next: the
/// lease times' refresh, read now (D4), or less when a held collection's incremental pass is due
/// sooner (MOD-159 M2 D9; that term floored at a second).
///
/// The URL first: none resolves, and nothing is taken, opened or loaded (no URL is one `info`
/// line until a URL is set). Then, for every workspace of [`IndexSource::index_scopes`] and the
/// global collection, in that order: its lease is taken (or renewed) for `owner`; one another
/// owner holds is forgotten. A held workspace collection's pass request is claimed right after
/// its take (MOD-159 M3 D5), before the pass reads anything, so a request made during the pass
/// stays for the next beat; a claimed request reconciles the collection in full this beat. A held
/// collection whose last reconcile is older than [`sync_interval`], that this job never
/// reconciled, or that was requested, is opened and synced over its workspace's scope, the lease
/// renewed before each delete batch; an abandoned sync forgets the collection. A requested
/// reconcile that fails has consumed its request.
/// The key is read at the first due collection only (MOD-159 M1 R2), and while one collection
/// reconciles every lease the job holds is renewed each refresh. A key that cannot be read lets
/// every lease the job holds go, so a box with a key takes the indexing over, and the job takes
/// no lease, and reads no key, until [`sync_interval`] has passed since.
/// Between reconciles (MOD-159 M2 D9), a workspace collection whose last reconcile completed is
/// passed over every [`INCREMENTAL_CYCLE`]: the rows its scope's projects had written since its
/// cursor less [`overlap_window`] are rebuilt with [`Indexer::sync_changes`] under the lease,
/// opened with the key the last reconcile read; a pass that finds nothing opens nothing. Its
/// cursor is the store's clock read before the pass's read, kept in this process only, so a
/// restart, a lost lease or a changed URL (which drops every cursor) reconciles in full first.
/// The global collection has nothing to sync yet: it is opened (created), and its holder drops the
/// legacy collection once per process (D10). Failures are log lines; the next beat or interval is
/// the retry.
pub async fn index_beat<S, I>(
    source: &S,
    settings: &concepts_worker::QdrantSource,
    index: &I,
    owner: Uuid,
    state: &mut JobState,
) -> Duration
where
    S: IndexSource + IndexerLeaseStore,
    I: JobIndex,
{
    let app = match source.index_settings().await {
        Ok(app) => app,
        Err(err) => {
            tracing::warn!(%err, "concepts index settings unread; the defaults apply");
            BTreeMap::new()
        }
    };
    let times = LeaseTimes::from_app(&app);
    let every = sync_interval(&app);
    let overlap = overlap_window(&app);
    // MOD-159 M1 R2: the URL alone; the key is read only when a collection is due (below).
    let url = match settings.url().await {
        Ok(url) => {
            state.no_url_said = false;
            url
        }
        Err(why) => {
            index.forget(None);
            state.held.clear();
            state.keyed = None;
            match why {
                concepts_worker::Unresolved::NoUrl => {
                    if !std::mem::replace(&mut state.no_url_said, true) {
                        tracing::info!("concepts index not synced: {NO_QDRANT_URL}");
                    }
                }
                why => {
                    tracing::warn!(err = %why.sentence(), "concepts index not opened; next beat");
                }
            }
            return times.refresh;
        }
    };
    // MOD-159 M2 D9: another Qdrant may hold none of what the last one did, so every held
    // collection reconciles in full against it now, and no pass opens with the old settings.
    if state.keyed.as_ref().is_some_and(|keyed| keyed.url != url) {
        state.keyed = None;
        state.held.clear();
    }
    // MOD-159 M1 R2 review: a box that cannot read the key leaves the indexing to one that can,
    // and does not ask the keyring again before the interval.
    if state
        .key_failed_at
        .is_some_and(|failed| failed.elapsed() < every)
    {
        return times.refresh;
    }
    let Some(box_id) = source.index_box().await else {
        tracing::warn!("concepts index not synced: this box has no identity; next beat");
        return times.refresh;
    };
    let scopes = match source.index_scopes().await {
        Ok(scopes) => scopes,
        Err(err) => {
            tracing::warn!(err = %format!("{err:#}"), "concepts index scopes failed; next beat");
            return times.refresh;
        }
    };
    let mut collections: Vec<(IndexedCollection, Option<Scope>)> = scopes
        .into_iter()
        .map(|scope| {
            (
                IndexedCollection::Workspace(scope.workspace_id),
                Some(scope),
            )
        })
        .collect();
    collections.push((IndexedCollection::Global, None));
    state
        .held
        .retain(|held, _| collections.iter().any(|(listed, _)| listed == held));
    let holder = Holder {
        box_id,
        owner,
        ttl: times.ttl,
    };
    // The key is read at the first due full reconcile, once for this beat (MOD-159 M1 R2).
    let mut key_read = false;
    for (collection, scope) in collections {
        match source
            .take_indexer_lease(&collection, box_id, owner, times.ttl)
            .await
        {
            Ok(LeaseTake::Taken | LeaseTake::Ours) => {}
            Ok(LeaseTake::Held(_)) => {
                state.held.remove(&collection);
                continue;
            }
            Err(err) => {
                tracing::warn!(%err, %collection, "concepts index lease not taken; next beat");
                // No pass without the lease: the next waits a cycle, not a second.
                if let Some(held) = state.held.get_mut(&collection) {
                    held.passed = tokio::time::Instant::now();
                }
                continue;
            }
        }
        // MOD-159 M3 D5: the claim comes before the pass reads anything, so a request made
        // during it re-stamps the cleared column and is claimed at the next beat.
        let requested = scope.is_some() && claim_pass(source, &collection, owner).await;
        let now = tokio::time::Instant::now();
        let last = state.held.get(&collection).copied();
        if let Some(held) = last.filter(|held| !requested && held.reconciled.elapsed() < every) {
            // MOD-159 M2 D9: between reconciles, the rows written since the last pass.
            let (Some(cursor), Some(scope)) = (held.cursor, scope.as_ref()) else {
                continue;
            };
            if held.passed.elapsed() < INCREMENTAL_CYCLE {
                continue;
            }
            if let Some(held) = state.held.get_mut(&collection) {
                held.passed = now;
            }
            let pass = match changes_since(source, scope, cursor, overlap).await {
                Ok(pass) => pass,
                Err(err) => {
                    pass_failed(state, collection, &err);
                    continue;
                }
            };
            if pass.changes.is_empty() {
                // Nothing written: nothing opened, no key, no model, no Qdrant call.
                pass_done(state, collection, pass.as_of);
                continue;
            }
            if state.keyed.is_none() {
                key_read = true;
                if !with_key(settings, index, &url, state).await {
                    release_held(source, owner, state).await;
                    state.key_failed_at = Some(tokio::time::Instant::now());
                    break;
                }
            }
            let leases: Vec<IndexedCollection> = state.held.keys().copied().collect();
            let work = incremental(source, index, collection, scope, pass, holder, state);
            let ((), lost) = while_renewing(source, &leases, holder, times.refresh, work).await;
            for collection in lost {
                state.held.remove(&collection);
            }
            continue;
        }
        // MOD-159 M2 D9: the cursor a completed reconcile sets is the store's clock before it
        // read anything, so a row written during the reconcile is the next pass's.
        let as_of = if scope.is_some() {
            match source.index_clock().await {
                Ok(as_of) => Some(as_of),
                Err(err) => {
                    tracing::warn!(%err, %collection, "concepts index clock unread; next beat");
                    state.held.remove(&collection);
                    continue;
                }
            }
        } else {
            None
        };
        state.held.insert(
            collection,
            Held {
                reconciled: now,
                passed: now,
                cursor: last.and_then(|held| held.cursor),
            },
        );
        if !key_read {
            key_read = true;
            if !with_key(settings, index, &url, state).await {
                // MOD-159 M1 R2 review: a held lease this box cannot sync under would keep a box
                // with a key from taking it over for as long as this one stays online.
                release_held(source, owner, state).await;
                state.key_failed_at = Some(tokio::time::Instant::now());
                break;
            }
        }
        let Some(resolved) = state.keyed.clone() else {
            continue;
        };
        // MOD-159 M1 R2: a reconcile may outlast the TTL (a first build), so every lease this
        // job holds, this collection's included, is renewed every refresh while it runs.
        let leases: Vec<IndexedCollection> = state.held.keys().copied().collect();
        let work = reconcile(source, index, &resolved, collection, scope, holder, state);
        let (completed, lost) = while_renewing(source, &leases, holder, times.refresh, work).await;
        if completed && let Some(as_of) = as_of {
            pass_done(state, collection, as_of);
        }
        for collection in lost {
            state.held.remove(&collection);
        }
    }
    // MOD-159 M2 D9: wake for the earliest pass due before the next lease beat; only that term
    // is floored, since the refresh may itself be under a second (a short TTL) and must not grow.
    state
        .held
        .values()
        .filter_map(Held::incremental_in)
        .min()
        .map_or(times.refresh, |due| {
            times.refresh.min(due.max(Duration::from_secs(1)))
        })
}

/// Claims `collection`'s pass request for `owner` (MOD-159 M3 D5): `true` is one `info` line and
/// a full reconcile this beat. A claim that fails is a `warn` line and no request.
async fn claim_pass<L: IndexerLeaseStore>(
    leases: &L,
    collection: &IndexedCollection,
    owner: Uuid,
) -> bool {
    match leases.claim_indexer_pass(collection, owner).await {
        Ok(true) => {
            tracing::info!(%collection, "concepts index pass requested; reconciling this beat");
            true
        }
        Ok(false) => false,
        Err(err) => {
            tracing::warn!(%err, %collection, "concepts index pass request unread; next beat");
            false
        }
    }
}

/// `url` with the key [`concepts_worker::QdrantSource::api_key`] reads now (MOD-159 M1 R2: only
/// when a collection is due, since an interactive key may prompt a locked keyring). A key that
/// cannot be read is `false`, said at `warn` once until its sentence changes, and drops only the
/// connections a sync made ([`JobIndex::forget_synced`]). The settings are kept in
/// [`JobState::keyed`] (MOD-159 M2 D8), the incremental passes' until the next due reconcile.
async fn with_key<I: JobIndex>(
    settings: &concepts_worker::QdrantSource,
    index: &I,
    url: &htui_core::qdrant::QdrantUrl,
    state: &mut JobState,
) -> bool {
    match settings.api_key().await {
        Ok(api_key) => {
            state.key_said = None;
            state.key_failed_at = None;
            state.keyed = Some(QdrantSettings {
                url: url.clone(),
                api_key,
            });
            true
        }
        Err(why) => {
            index.forget_synced();
            state.keyed = None;
            let sentence = why.sentence();
            if state.key_said.as_deref() != Some(sentence.as_str()) {
                tracing::warn!(err = %sentence, "concepts index not synced: no key; next interval");
                state.key_said = Some(sentence);
            }
            false
        }
    }
}

/// Lets go of every lease `state` holds for `owner` (MOD-159 M1 R2 review: the key cannot be
/// read), each on its own, so a lease `owner` lost meanwhile stays its new holder's. A release
/// that fails is a log line: that lease lapses within its TTL.
async fn release_held<L: IndexerLeaseStore>(leases: &L, owner: Uuid, state: &mut JobState) {
    for collection in std::mem::take(&mut state.held).into_keys() {
        if let Err(err) = leases.release_indexer_lease(&collection, owner).await {
            tracing::warn!(%err, %collection, "concepts index lease not released; it lapses");
        }
    }
}

/// `work`, with `collections`' leases renewed for `holder` every `every` while it runs (MOD-159
/// M1 R2). Answers what `work` answered, and the collections whose renewal touched nothing
/// (another owner took the lease): the caller stops holding them. A renewal that fails is a log
/// line; the next beat retries.
async fn while_renewing<L: IndexerLeaseStore, T>(
    leases: &L,
    collections: &[IndexedCollection],
    holder: Holder,
    every: Duration,
    work: impl Future<Output = T>,
) -> (T, Vec<IndexedCollection>) {
    let every = every.max(Duration::from_secs(1));
    let mut work = std::pin::pin!(work);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut lost = Vec::new();
    loop {
        tokio::select! {
            out = &mut work => return (out, lost),
            _ = ticker.tick() => {
                for collection in collections {
                    if lost.contains(collection) {
                        continue;
                    }
                    match leases
                        .renew_indexer_lease(collection, holder.owner, holder.ttl)
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::info!(%collection, "concepts index lease taken over; let go");
                            lost.push(*collection);
                        }
                        Err(err) => {
                            tracing::warn!(%err, %collection, "concepts index lease not renewed");
                        }
                    }
                }
            }
        }
    }
}

/// [`index_beat`]'s work on one held, due collection. Answers whether a workspace's sync
/// completed, not abandoned (MOD-159 M2 C43): only then does its cursor move. A failed open, a
/// failed sync, an abandon and the global collection (nothing to sync) answer `false`.
async fn reconcile<S, I>(
    source: &S,
    index: &I,
    resolved: &QdrantSettings,
    collection: IndexedCollection,
    scope: Option<Scope>,
    holder: Holder,
    state: &mut JobState,
) -> bool
where
    S: IndexSource + IndexerLeaseStore,
    I: JobIndex,
{
    let store = match index.open(resolved.clone(), &collection).await {
        Ok(store) => {
            state.mismatch.remove(&collection);
            store
        }
        Err(err) => {
            match open_failure_level(names_a_mismatch(&err), &mut state.mismatch, collection) {
                tracing::Level::ERROR => tracing::error!(
                    %err,
                    %collection,
                    "concepts index refused by Qdrant; no sync until the collection is rebuilt \
                     (checked again every interval, logged once)"
                ),
                tracing::Level::DEBUG => {
                    tracing::debug!(%err, %collection, "concepts index still refused");
                }
                _ => tracing::warn!(%err, %collection, "concepts index not opened; next interval"),
            }
            return false;
        }
    };
    let Some(scope) = scope else {
        // D10: the global collection's holder drops the legacy one, once per process.
        if !state.legacy_checked {
            match index.drop_legacy(resolved).await {
                Ok(dropped) => {
                    state.legacy_checked = true;
                    if dropped {
                        tracing::info!(
                            "dropped the old shared concepts collection `{LEGACY_COLLECTION}`"
                        );
                    }
                }
                Err(err) => tracing::warn!(%err, "the old concepts collection was not dropped"),
            }
        }
        return false;
    };
    let guard = Renewing {
        leases: source,
        collection,
        holder,
    };
    match Indexer::sync(source, &scope, &*store, &guard).await {
        Ok(report) if report.abandoned => {
            state.held.remove(&collection);
            tracing::info!(
                %collection,
                "concepts index lease taken over during the sync; stopped, nothing deleted"
            );
            false
        }
        Ok(report) => {
            tracing::info!(?report, %collection, "concepts index synced");
            true
        }
        Err(err) => {
            index.forget(Some(&store));
            tracing::warn!(%err, %collection, "concepts index sync failed; next interval");
            false
        }
    }
}

/// What one incremental pass reads before it writes (MOD-159 M2 plan D9).
struct Pass {
    /// The store's clock before the change read: the collection's next cursor.
    as_of: DateTime<Utc>,
    /// The rows written since the cursor, less the overlap.
    changes: IndexChanges,
}

/// The rows of `scope` written since `cursor`, pulled back by `overlap` (MOD-159 M2 plan D5), and
/// the store's clock read before them.
async fn changes_since<S: IndexChangesStore>(
    source: &S,
    scope: &Scope,
    cursor: IndexCursor,
    overlap: Duration,
) -> Result<Pass, StoreError> {
    let as_of = source.index_clock().await?;
    let back = TimeDelta::from_std(overlap).unwrap_or(TimeDelta::MAX);
    let since = cursor
        .rows
        .checked_sub_signed(back)
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let changes = source.index_changes(&scope.project_ids, since).await?;
    Ok(Pass { as_of, changes })
}

/// [`index_beat`]'s incremental pass of one held collection with changes (MOD-159 M2 plan D9):
/// opened with [`JobState::keyed`], then [`Indexer::sync_changes`] under the lease. Success moves
/// the cursor to `pass.as_of`; an abandon forgets the collection, as a reconcile's does; a failed
/// open or sync keeps the cursor, so the next cycle reads the same window again, and is a `warn`
/// line once per collection, `debug` after, until a pass succeeds.
async fn incremental<S, I>(
    source: &S,
    index: &I,
    collection: IndexedCollection,
    scope: &Scope,
    pass: Pass,
    holder: Holder,
    state: &mut JobState,
) where
    S: IndexSource + IndexerLeaseStore,
    I: JobIndex,
{
    let Some(resolved) = state.keyed.clone() else {
        return;
    };
    let store = match index.open(resolved, &collection).await {
        Ok(store) => {
            state.mismatch.remove(&collection);
            store
        }
        Err(err) => {
            pass_failed(state, collection, &err);
            return;
        }
    };
    let guard = Renewing {
        leases: source,
        collection,
        holder,
    };
    match Indexer::sync_changes(source, scope, &pass.changes, &*store, &guard).await {
        Ok(report) if report.abandoned => {
            state.held.remove(&collection);
            tracing::info!(
                %collection,
                "concepts index lease taken over during a pass; stopped, nothing deleted"
            );
        }
        Ok(report) => {
            tracing::debug!(?report, %collection, "concepts index pass done");
            pass_done(state, collection, pass.as_of);
        }
        Err(err) => {
            index.forget(Some(&store));
            pass_failed(state, collection, &err);
        }
    }
}

/// A completed pass of `collection`, full or incremental, that began at `as_of`: its cursor
/// moves there, and a failed pass logged before may be logged again.
fn pass_done(state: &mut JobState, collection: IndexedCollection, as_of: DateTime<Utc>) {
    if let Some(held) = state.held.get_mut(&collection) {
        held.cursor = Some(IndexCursor::at(as_of));
    }
    state.incremental_failed.remove(&collection);
}

/// A failed incremental pass of `collection`: `warn` the first time, `debug` until a pass
/// succeeds. Its cursor stays where it was.
fn pass_failed(state: &mut JobState, collection: IndexedCollection, err: &dyn std::fmt::Display) {
    if state.incremental_failed.insert(collection) {
        tracing::warn!(%err, %collection, "concepts index pass failed; same window next cycle");
    } else {
        tracing::debug!(%err, %collection, "concepts index pass still failing");
    }
}

/// Whether a failed open's text (`QdrantIndex::connect`'s `{:#}` of [`qdrant_refusal`]) is
/// [`is_embedder_mismatch`]'s refusal: the store error's display wraps its message, so the
/// prefix is looked for inside it.
fn names_a_mismatch(err: &str) -> bool {
    err.contains(&format!("{EMBEDDER_MISMATCH}: "))
}

/// The index job: [`index_beat`] at start, then again after each beat, forever. It never touches
/// the run runtime; [`IndexJob::stop`] ends it.
pub async fn index_job<S, I>(
    source: &S,
    settings: &concepts_worker::QdrantSource,
    index: &I,
    owner: Uuid,
) -> !
where
    S: IndexSource + IndexerLeaseStore,
    I: JobIndex,
{
    let mut state = JobState::default();
    loop {
        let pause = index_beat(source, settings, index, owner, &mut state).await;
        tokio::time::sleep(pause).await;
    }
}

/// How long `htui worker`'s [`IndexJob::stop`] waits for its leases to be let go before it gives
/// up on them (they lapse on their own after the TTL). The TUI waits less (MOD-159 M1 R2): its
/// whole store-loop shutdown has `crate::SHUTDOWN`.
pub const WORKER_RELEASE_WAIT: Duration = Duration::from_secs(5);

/// A running [`index_job`] (MOD-159 M1 plan D7, D8): `htui worker`'s, and the TUI's while it is
/// online. [`Self::stop`] ends it and lets go of every lease it holds.
#[derive(Debug)]
pub struct IndexJob<S: IndexerLeaseStore> {
    /// The job's task.
    task: tokio::task::JoinHandle<()>,
    /// Where its leases live.
    leases: S,
    /// Its token (plan D5).
    owner: Uuid,
}

impl IndexJob<PgStore> {
    /// The job over `pg` and the process's shared index (MOD-11 H-26: the worker's index job and
    /// `search_concepts` load one model), with a token of its own (plan D5). Always spawned,
    /// whatever the setting says: each beat resolves the URL again, and the key when a collection
    /// is due (MOD-158 D7, MOD-159 M1 R2).
    #[must_use]
    pub fn spawn(pg: PgStore) -> Self {
        Self::spawn_over(pg, concepts_worker::shared_qdrant(), Uuid::now_v7(), None)
    }

    /// The job over `pg` and `index`, taking leases as `owner`; its first beat waits for `after`
    /// (the previous jobs' [`Self::stop`]s, so their release cannot free what this one takes).
    pub(crate) fn spawn_over(
        pg: PgStore,
        index: Arc<QdrantIndex>,
        owner: Uuid,
        after: Option<futures::future::BoxFuture<'static, ()>>,
    ) -> Self {
        let leases = pg.clone();
        let task = tokio::spawn(async move {
            if let Some(after) = after {
                after.await;
            }
            let settings = Arc::clone(index.source());
            match index_job(&pg, &settings, &*index, owner).await {}
        });
        Self {
            task,
            leases,
            owner,
        }
    }
}

impl<S: IndexerLeaseStore> IndexJob<S> {
    /// Whether the task has ended (it ends only when stopped, or on a panic).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Ends the job, then lets go of every lease its token holds (best effort, logged, waited for
    /// at most `wait`: [`WORKER_RELEASE_WAIT`] in `htui worker`, less in the TUI): another process
    /// takes them over on its next beat rather than after the TTL. A sync stopped half-way is
    /// safe: the next holder's reconcile finishes it. A job that had panicked is said at `error`.
    pub async fn stop(self, wait: Duration) {
        self.task.abort();
        if let Err(err) = self.task.await
            && err.is_panic()
        {
            tracing::error!(%err, "the concepts index job had panicked; it indexed nothing since");
        }
        match tokio::time::timeout(wait, self.leases.release_indexer_leases(self.owner)).await {
            Ok(Ok(freed)) => tracing::debug!(freed, "concepts index leases let go"),
            Ok(Err(err)) => tracing::warn!(%err, "concepts index leases not let go; they lapse"),
            Err(_) => tracing::warn!("concepts index leases not let go in time; they lapse"),
        }
    }
}

/// The level [`reconcile`] logs one failed open at: an embedder mismatch is `ERROR` the first time
/// and `DEBUG` after, since it says the same until someone rebuilds the collection; anything else
/// is `WARN` every time (review L3).
/// The once is per collection (MOD-159 M1 R2): `reported` holds the collections whose mismatch
/// was said.
fn open_failure_level(
    mismatch: bool,
    reported: &mut HashSet<IndexedCollection>,
    collection: IndexedCollection,
) -> tracing::Level {
    if !mismatch {
        tracing::Level::WARN
    } else if reported.insert(collection) {
        tracing::Level::ERROR
    } else {
        tracing::Level::DEBUG
    }
}

/// `work` on a named thread of its own, awaited. Not `spawn_blocking`: the runtime waits for its
/// blocking tasks when it is dropped, and a keyring prompt nobody answers or a model load
/// must not hold `htui worker`'s exit (`worker_cmd::read_dsn_apart`'s reason). The thread is
/// left behind on an abort and dies with the process. `None` when the thread cannot be spawned
/// or ends without answering. `concepts_worker`'s model load runs here too.
pub(crate) async fn apart<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (answer, answered) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || drop(answer.send(work())))
        .ok()?;
    answered.await.ok()
}

/// `--search-items`: the best hits, as lines or (`--json`) one document (MOD-92). Read-only:
/// `PgStore::connect_read_only` and `QdrantStore`'s read-only open; a missing collection is an
/// empty index, never created. The embedding model may still be downloaded into the user cache
/// (plan D8), and only when there is a collection to search; its notes go to stderr.
///
/// # Errors
/// [`StateExit`]: 3 when Postgres or Qdrant does not answer, 2 on a refusal (no DSN, schema, no
/// URL or an unusable one, an unreadable key, scope, embedder mismatch, a refused API key), 1 on a
/// model or read failure.
pub async fn search_items(options: &SearchOptions) -> Result<(), StateExit> {
    let answer = match search_inputs(options) {
        Err(failure) => Answer {
            kind: Kind::Search,
            outcome: Outcome::Failed(failure),
        },
        Ok((dsn, identity)) => {
            let env = QdrantEnv::from_process();
            let ctx = SearchContext {
                env: &env,
                keyring_key: secret::get_qdrant_api_key,
                prefix: "",
                dsn: &dsn,
                identity: identity.as_ref(),
                connect_timeout: CONNECT_TIMEOUT,
            };
            search_answer_with(&ctx, options, load_embedder).await
        }
    };
    state_cmd::emit(&mut std::io::stdout().lock(), &answer, options.json)
}

/// What [`search_answer_with`] needs that [`search_items`] reads from the process, the keyring
/// and `box.toml`: the test seam (review R1 M4), as `state_cmd::Context` is for `status` and
/// `waiting`. The Qdrant URL is not here: it is the `qdrant_url` row, read after the connect
/// (MOD-158 D8), unless `env` overrides it.
#[derive(Clone)]
pub struct SearchContext<'a> {
    /// `HTUI_QDRANT_URL` and `HTUI_QDRANT_API_KEY` (a test's literal, never the process's).
    pub env: &'a QdrantEnv,
    /// The keyring read of the Qdrant API key: `secret::get_qdrant_api_key`, or a test's closure.
    pub keyring_key: concepts_worker::KeyringRead,
    /// Put before every collection name searched (MOD-159 M1 plan D9: `htui_ws_<uuid>` per
    /// workspace, and `htui_global`): empty in production, a test's throwaway prefix.
    pub prefix: &'a str,
    /// The Postgres DSN.
    pub dsn: &'a str,
    /// `box.toml`'s identity; read only for `--cwd`, so `None` otherwise.
    pub identity: Option<&'a Identity>,
    /// How long the Postgres connect may take.
    pub connect_timeout: Duration,
}

impl core::fmt::Debug for SearchContext<'_> {
    /// Everything but the DSN and the API key, which are secrets.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SearchContext")
            .field(
                "qdrant_url_override",
                &concepts_worker::shown_env_url(self.env),
            )
            .field("prefix", &self.prefix)
            .field("dsn", &"<redacted>")
            .field("identity", &self.identity)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

/// The reads before the connect: the DSN, then `box.toml` (for `--cwd` only). The Qdrant URL is
/// read after the connect (MOD-158 D8: it is a row in the database).
fn search_inputs(
    options: &SearchOptions,
) -> Result<(Zeroizing<String>, Option<Identity>), Failure> {
    let dsn = state_cmd::read_dsn()?;
    let identity = match options.cwd {
        Some(_) => state_cmd::read_identity()?,
        None => None,
    };
    Ok((dsn, identity))
}

/// Why no Qdrant settings resolved, as the search's [`Failure`]: `no_qdrant_url`,
/// `qdrant_url_unusable`, the row read's own failure, or `keyring_unreadable` for the key. The
/// message says the search is unavailable first (MOD-160 D8); the code and the exit are the cause's.
fn unresolved_failure(why: concepts_worker::Unresolved) -> Failure {
    use concepts_worker::Unresolved;
    unavailable_failure(match why {
        Unresolved::NoUrl => Failure::refused(ErrorCode::NoQdrantUrl, NO_QDRANT_URL),
        Unresolved::Unusable(sentence) => Failure::refused(ErrorCode::QdrantUrlUnusable, sentence),
        Unresolved::Read(err) => Failure::from_read(&err),
        Unresolved::Key(err) => Failure::refused(ErrorCode::KeyringUnreadable, err.to_string()),
        // A search reads through the store it connected, so it never lacks one.
        Unresolved::Offline => Failure::from_read(&StoreError::Unreachable(
            concepts_worker::URL_OFFLINE.to_owned(),
        )),
    })
}

/// MOD-160 D8: `failure`, its message now [`htui_core::qdrant::unavailable`]'s: "knowledge search
/// unavailable: " and then today's sentence. `store` and `code` (so the exit) are unchanged.
fn unavailable_failure(mut failure: Failure) -> Failure {
    failure.error.message = htui_core::qdrant::unavailable(&failure.error.message);
    failure
}

/// `--search-items` over `ctx`, with the model from `load` (review R1 M4). Never fails: a failure
/// is an [`Answer`] too. In MOD-158 D8's order: the read-only connect, the Qdrant URL (the override
/// or the row) and key, the scope (the pool is closed once it is read, review R1 L9), then the
/// index (`search_index`: Qdrant first, the model only when there is a collection to search).
pub async fn search_answer_with<E, F, Fut>(
    ctx: &SearchContext<'_>,
    options: &SearchOptions,
    load: F,
) -> Answer
where
    E: DenseEmbedder + Clone,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let outcome = match search_online(ctx, options, load).await {
        Ok(search) => Outcome::Search(search),
        Err(failure) => Outcome::Failed(failure),
    };
    Answer {
        kind: Kind::Search,
        outcome,
    }
}

/// [`search_answer_with`]'s reads.
async fn search_online<E, F, Fut>(
    ctx: &SearchContext<'_>,
    options: &SearchOptions,
    load: F,
) -> Result<SearchAnswer, Failure>
where
    E: DenseEmbedder + Clone,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let pg = PgStore::connect_read_only(
        ctx.dsn,
        ctx.identity.map(|identity| identity.box_id),
        ctx.connect_timeout,
    )
    .await
    .map_err(Failure::from_connect)?;
    let settings = match concepts_worker::resolve_settings(
        ctx.env,
        Some(&pg),
        &KeySource::Interactive,
        ctx.keyring_key,
    )
    .await
    {
        Ok(settings) => settings,
        Err(why) => {
            pg.close().await;
            return Err(unresolved_failure(why));
        }
    };
    let targets = search_projects(&pg, options, ctx.identity).await;
    pg.close().await;
    let (index, unavailable, cause) =
        search_index(&settings, ctx.prefix, &targets?, options, load).await;
    Ok(SearchAnswer {
        query: options.query.clone(),
        decisions: options.decisions,
        index,
        unavailable,
        cause,
    })
}

/// One workspace a search reads (MOD-159 M1 plan D9): its slug, for the `unavailable` list, and
/// its scope: the workspace, whose collection is searched, and the projects searched there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchTarget {
    /// The workspace's slug.
    pub slug: String,
    /// The workspace and the projects searched in its collection.
    pub scope: Scope,
}

/// The workspaces a search reads, each with its projects (MOD-92 D6, MOD-159 M1 plan D9): `--cwd`'s
/// project in every workspace that holds it, `--project`'s, or every workspace's, in scope order.
async fn search_projects(
    pg: &PgStore,
    options: &SearchOptions,
    identity: Option<&Identity>,
) -> Result<Vec<SearchTarget>, Failure> {
    if let Some(cwd) = options.cwd.as_deref() {
        let workspaces = pg
            .workspaces()
            .await
            .map_err(|err| Failure::from_read(&err))?;
        let project = state_cmd::cwd_project(pg, &workspaces, cwd, identity).await?;
        return Ok(workspaces
            .iter()
            .filter(|ws| ws.projects.iter().any(|p| p.project_id == project))
            .map(|ws| SearchTarget {
                slug: ws.slug.clone(),
                scope: Scope {
                    workspace_id: ws.workspace_id,
                    project_ids: vec![project],
                },
            })
            .collect());
    }
    let scopes = scopes_named(pg, options.project.as_deref())
        .await
        .map_err(|err| match err.downcast_ref::<StoreError>() {
            Some(err) => Failure::from_read(err),
            None => Failure::refused(ErrorCode::UnknownProject, err.to_string()),
        })?;
    Ok(scopes
        .into_iter()
        .map(|(slug, scope)| SearchTarget { slug, scope })
        .collect())
}

/// One collection's part of a search: its hits, no such collection, or why it failed.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    /// The collection answered.
    Hits(Vec<Hit>),
    /// There is no such collection.
    Missing,
    /// The collection, the model or the search failed: the answer that says how.
    Failed(IndexAnswer),
}

/// D9: the collections' parts as one answer, the workspaces whose collection is missing or
/// failed (the global one, `None`, is never named), and the first failure's sentence when the
/// answer is hits anyway (MOD-159 M1 R2: the stderr note says it). A failure and no workspace
/// collection that answered: the first failure, as a search of one collection answered; the
/// global collection, empty on most boxes, never stands in for a workspace (R2). Otherwise any
/// collection that answered: `Hits`, merged ([`merge_hits`]), and the workspaces named. Every
/// collection missing: `Empty`.
fn combine(
    parts: Vec<(Option<String>, Part)>,
    limit: u64,
) -> (IndexAnswer, Vec<String>, Option<String>) {
    let mut lists = Vec::new();
    let mut unavailable = Vec::new();
    let mut failure = None;
    let mut a_workspace_answered = false;
    for (slug, part) in parts {
        match part {
            Part::Hits(hits) => {
                a_workspace_answered |= slug.is_some();
                lists.push(hits);
                continue;
            }
            Part::Missing => {}
            Part::Failed(answer) => {
                failure.get_or_insert(answer);
            }
        }
        if let Some(slug) = slug
            && !unavailable.contains(&slug)
        {
            unavailable.push(slug);
        }
    }
    match failure {
        Some(failure) if !a_workspace_answered => (failure, Vec::new(), None),
        failure if !lists.is_empty() => (
            IndexAnswer::Hits(merge_hits(lists, limit)),
            unavailable,
            failure.as_ref().and_then(failure_sentence),
        ),
        _ => (IndexAnswer::Empty, Vec::new(), None),
    }
}

/// A failed answer's sentence; `None` for hits and the empty index.
fn failure_sentence(answer: &IndexAnswer) -> Option<String> {
    match answer {
        IndexAnswer::Offline(sentence)
        | IndexAnswer::Refused(sentence)
        | IndexAnswer::AccessRefused(sentence)
        | IndexAnswer::Failed(sentence) => Some(sentence.clone()),
        IndexAnswer::Hits(_) | IndexAnswer::Empty => None,
    }
}

/// The collections, then the model, then the searches (MOD-92 G8, review R1 M3, MOD-159 M1 plan
/// D9): each target's workspace collection over that target's projects, and the global one over
/// all of them. A Qdrant that does not answer or refuses is answered at the first collection,
/// and a missing collection is never created; neither loads the model. The model loads once,
/// only when a collection exists and there is something to search (a project, a non-zero limit);
/// the embedder check runs on it before each collection is searched.
async fn search_index<E, F, Fut>(
    settings: &QdrantSettings,
    prefix: &str,
    targets: &[SearchTarget],
    options: &SearchOptions,
    load: F,
) -> (IndexAnswer, Vec<String>, Option<String>)
where
    E: DenseEmbedder + Clone,
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<E>>,
{
    let mut every = Vec::new();
    for target in targets {
        for project in &target.scope.project_ids {
            if !every.contains(project) {
                every.push(*project);
            }
        }
    }
    let wanted = targets
        .iter()
        .map(|target| {
            (
                Some(target.slug.clone()),
                IndexedCollection::Workspace(target.scope.workspace_id),
                target.scope.project_ids.clone(),
            )
        })
        .chain(std::iter::once((None, IndexedCollection::Global, every)));
    let mut parts = Vec::new();
    let mut found = Vec::new();
    for (slug, collection, projects) in wanted {
        let name = format!("{prefix}{collection}");
        match ExistingCollection::find_in(settings, &name).await {
            Ok(Some(existing)) => {
                let query = query(&options.query, projects, options.decisions, options.limit);
                found.push((parts.len(), existing, query));
                parts.push((slug, Part::Hits(Vec::new())));
            }
            Ok(None) => parts.push((slug, Part::Missing)),
            Err(err) => return (index_refusal(err), Vec::new(), None),
        }
    }
    found.retain(|(_, _, query)| !query.projects.is_empty() && query.limit > 0);
    if !found.is_empty() {
        match load().await {
            Ok(embedder) => {
                for (at, existing, query) in found {
                    parts[at].1 = match existing.open(embedder.clone()).await {
                        Ok(store) => match store.search(&query).await {
                            Ok(hits) => Part::Hits(hits),
                            Err(err) => Part::Failed(index_refusal(err)),
                        },
                        Err(err) => Part::Failed(index_refusal(err)),
                    };
                }
            }
            Err(err) => {
                let failed =
                    IndexAnswer::Failed(htui_core::qdrant::unavailable(&format!("{err:#}")));
                for (at, _, _) in found {
                    parts[at].1 = Part::Failed(failed.clone());
                }
            }
        }
    }
    combine(parts, options.limit)
}

/// A failed open or search (review R1 M2): a query the model failed on is the model's failure
/// (`model_unavailable`); a refused API key and an embedder mismatch are refusals
/// (`qdrant_refused`, `embedder_mismatch`); anything else "cannot reach Qdrant"
/// (`qdrant_refusal`'s sentences). Every sentence says the search is unavailable first (MOD-160
/// D8).
fn index_refusal(err: StoreError) -> IndexAnswer {
    use htui_core::qdrant::unavailable;
    if is_query_embed_failure(&err) {
        return IndexAnswer::Failed(unavailable(&format!(
            "{:#}",
            anyhow::Error::new(err).context("the embedding model failed on the query")
        )));
    }
    let (mismatch, refused) = (is_embedder_mismatch(&err), is_qdrant_refused(&err));
    let sentence = unavailable(&format!("{:#}", qdrant_refusal(err)));
    if refused {
        IndexAnswer::AccessRefused(sentence)
    } else if mismatch {
        IndexAnswer::Refused(sentence)
    } else {
        IndexAnswer::Offline(sentence)
    }
}

/// One result line: key, where it matched, score and snippet. Where it matched names a closed
/// item's resolution and a withdrawn requirement (MOD-50 D227): `--decisions` lists `rejected`
/// next to `done`, and a rejected decision must not read as an adopted one.
#[must_use]
pub fn format_hit(hit: &Hit) -> String {
    let place = match (&hit.point_type, &hit.document) {
        (PointType::Document, Some((_, kind))) => format!("{kind} document"),
        (PointType::Requirement, _) => "requirement".to_owned(),
        _ => "item".to_owned(),
    };
    let place = match (hit.resolution, hit.state) {
        (Some(resolution), _) => format!("{place} ({resolution})"),
        (None, Some(RequirementState::Withdrawn)) => format!("{place} (withdrawn)"),
        _ => place,
    };
    // The snippet is already stripped of control characters (`vector::snippet`); the key and the
    // document kind are stripped here, so nothing stored can drive the terminal it is printed on.
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!(
        "{:<10} {:<26} {:.3}  {}",
        clean(&hit.key),
        clean(&place),
        hit.score,
        hit.snippet
    )
}

/// What one index run did, as one line (MOD-64 D237, D239): `htui --index-items` prints it on
/// stderr and the search overlay under its hits.
#[must_use]
pub fn report_line(report: &SyncReport) -> String {
    format!(
        "indexed: {} item(s) rebuilt, {} unchanged; {} requirement(s) rebuilt, {} unchanged; \
         {} point(s) written, {} removed",
        report.items_rebuilt,
        report.items_unchanged,
        report.requirements_rebuilt,
        report.requirements_unchanged,
        report.points_upserted,
        report.points_deleted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::model::{DocumentId, ItemId, RequirementId};
    use htui_store::vector::Owner;
    use uuid::Uuid;

    fn hit(point_type: PointType, document: Option<&str>) -> Hit {
        Hit {
            point_type,
            owner: Owner::Item(ItemId(Uuid::nil())),
            key: "ANA-11".into(),
            document: document.map(|kind| (DocumentId(Uuid::nil()), kind.to_owned())),
            resolution: None,
            state: None,
            score: 0.5,
            snippet: "a decision is a closed item".into(),
        }
    }

    /// MOD-158 T3: `HTUI_QDRANT_URL` is unvalidated, so `Debug` prints it only once it parses;
    /// a refused override may hold a password and is never echoed (nor the key, nor the DSN).
    #[test]
    fn a_search_context_debug_never_prints_an_unusable_override() {
        let shown = |url: &str| {
            let env = QdrantEnv::new(Some(url.to_owned()), Some("KEY-SENTINEL".to_owned()));
            let ctx = SearchContext {
                env: &env,
                keyring_key: || Ok(None),
                prefix: "c",
                dsn: "postgres://u:DSN-SENTINEL@db/htui",
                identity: None,
                connect_timeout: Duration::from_secs(1),
            };
            format!("{ctx:?}")
        };
        let refused = shown("http://user:SENTINEL@env:6334");
        assert!(!refused.contains("SENTINEL"), "{refused}");
        assert!(refused.contains("<not a usable URL>"), "{refused}");
        let shown = shown(" http://env:6334 ");
        assert!(shown.contains("http://env:6334"), "{shown}");
        assert!(!shown.contains("SENTINEL"), "{shown}");
    }

    #[test]
    fn only_an_unreachable_server_is_cannot_reach() {
        use htui_core::store::StoreError;
        assert_eq!(
            store_context(&StoreError::Unreachable("refused".into())),
            "cannot reach Postgres"
        );
        assert_eq!(
            store_context(&StoreError::Backend("the schema is newer".into())),
            "Postgres refused this htui"
        );
    }

    /// Review L3: Qdrant answered and refused, so the line says that, not "cannot reach".
    #[test]
    fn an_embedder_mismatch_is_not_cannot_reach_qdrant() {
        use htui_store::vector::EMBEDDER_MISMATCH;
        let mismatch = StoreError::Backend(format!(
            "{EMBEDDER_MISMATCH}: collection `c` holds vectors from hash/384; delete collection \
             `c` and run `htui --index-items` to rebuild it"
        ));
        let text = format!("{:#}", qdrant_refusal(mismatch));
        assert!(text.contains(EMBEDDER_MISMATCH), "{text}");
        assert!(!text.contains("cannot reach"), "{text}");
        assert!(
            names_a_mismatch(&text),
            "the job's open reads it as one: {text}"
        );
        let down = StoreError::Backend("qdrant: collection exists: transport error".into());
        let text = format!("{:#}", qdrant_refusal(down));
        assert!(!names_a_mismatch(&text), "{text}");
        assert!(text.starts_with("cannot reach Qdrant: "), "{text}");
        assert!(
            text.ends_with("qdrant: collection exists: transport error"),
            "{text}"
        );
    }

    /// Review L3: the index job says a mismatch once, at `error`, and a Qdrant that does not
    /// answer every cycle, at `warn`.
    #[test]
    fn a_mismatch_is_an_error_once_then_debug_and_anything_else_a_warning() {
        use tracing::Level;
        let c = IndexedCollection::Global;
        let mut reported = HashSet::new();
        assert_eq!(open_failure_level(false, &mut reported, c), Level::WARN);
        assert_eq!(open_failure_level(true, &mut reported, c), Level::ERROR);
        assert_eq!(open_failure_level(true, &mut reported, c), Level::DEBUG);
        assert_eq!(open_failure_level(false, &mut reported, c), Level::WARN);
        assert_eq!(open_failure_level(true, &mut reported, c), Level::DEBUG);
    }

    #[test]
    fn hits_print_key_place_score_and_snippet() {
        let line = format_hit(&hit(PointType::Document, Some("summary")));
        assert!(line.starts_with("ANA-11"));
        assert!(line.contains("summary document"));
        assert!(line.contains("0.500"));
        assert!(line.ends_with("a decision is a closed item"));
    }

    #[test]
    fn an_open_item_prints_as_before() {
        let line = format_hit(&hit(PointType::Item, None));
        assert!(line.contains("item "), "{line}");
        assert!(!line.contains('('), "{line}");
    }

    #[test]
    fn a_closed_item_and_its_documents_name_the_resolution() {
        let rejected = Hit {
            resolution: Some(Resolution::Rejected),
            ..hit(PointType::Item, None)
        };
        assert!(format_hit(&rejected).contains("item (rejected)"));
        let done_doc = Hit {
            resolution: Some(Resolution::Done),
            ..hit(PointType::Document, Some("summary"))
        };
        assert!(format_hit(&done_doc).contains("summary document (done)"));
    }

    #[test]
    fn requirements_print_as_such_and_say_when_withdrawn() {
        let active = Hit {
            owner: Owner::Requirement(RequirementId(Uuid::nil())),
            key: "R-STO-8".into(),
            state: Some(RequirementState::Active),
            ..hit(PointType::Requirement, None)
        };
        let line = format_hit(&active);
        assert!(line.starts_with("R-STO-8"), "{line}");
        assert!(line.contains("requirement "), "{line}");
        assert!(!line.contains('('), "{line}");
        let withdrawn = Hit {
            state: Some(RequirementState::Withdrawn),
            ..active
        };
        assert!(format_hit(&withdrawn).contains("requirement (withdrawn)"));
    }

    #[test]
    fn decisions_are_done_concluded_and_rejected() {
        assert_eq!(
            DECISION_RESOLUTIONS,
            [
                Resolution::Done,
                Resolution::Concluded,
                Resolution::Rejected
            ]
        );
    }

    #[test]
    fn query_narrows_to_decisions_only_when_asked() {
        let projects = vec![ProjectId(Uuid::from_u128(2)), ProjectId(Uuid::from_u128(1))];
        for decisions in [false, true] {
            let q = query("closed items", projects.clone(), decisions, 7);
            assert_eq!(q.text, "closed items");
            assert_eq!(q.projects, projects);
            assert_eq!(q.limit, 7);
            assert!(q.types.is_empty() && q.statuses.is_empty(), "{q:?}");
            let expected = if decisions {
                DECISION_RESOLUTIONS.to_vec()
            } else {
                Vec::new()
            };
            assert_eq!(q.resolutions, expected, "decisions = {decisions}");
        }
    }

    /// MOD-92 D6: without `--project` or `--cwd`, `--search-items` still searches every
    /// workspace's projects, in workspace order, each workspace's by position; `--project`
    /// narrows to one. Through the real scope code over the demo database (review R1 M4).
    /// MOD-147 M4 D7: each workspace's scope row goes first, before its members; `--project
    /// @<workspace>` answers that scope row alone, and an unknown `@` slug is `unknown_project`.
    #[tokio::test]
    async fn search_projects_default_to_every_workspace_and_project_narrows() {
        let Some(db) = htui_store::testkit::demo_db().await else {
            return;
        };
        let pg = PgStore::connect_read_only(&db.url, None, CONNECT_TIMEOUT)
            .await
            .expect("connect read-only");
        let options = SearchOptions {
            query: "closed items".into(),
            project: None,
            decisions: false,
            limit: DEFAULT_LIMIT,
            cwd: None,
            json: true,
        };
        let workspaces = pg.workspaces().await.expect("workspaces");
        assert!(workspaces.len() >= 2, "{workspaces:?}");
        let mut expected = Vec::new();
        for ws in &workspaces {
            let row = pg
                .scope_project(ws.workspace_id)
                .await
                .expect("the scope-row lookup")
                .expect("every demo workspace has a scope row");
            let mut projects = vec![row];
            projects.extend(Scope::from_workspace(ws).project_ids);
            // MOD-159 M1 D9: one target per workspace, each searched in its own collection.
            expected.push(SearchTarget {
                slug: ws.slug.clone(),
                scope: Scope {
                    workspace_id: ws.workspace_id,
                    project_ids: projects,
                },
            });
        }
        let every = search_projects(&pg, &options, None)
            .await
            .expect("the default scope");
        assert_eq!(every, expected);
        let one = SearchOptions {
            project: Some("htui".into()),
            ..options.clone()
        };
        let projects = |targets: Vec<SearchTarget>| -> Vec<ProjectId> {
            targets
                .into_iter()
                .flat_map(|target| target.scope.project_ids)
                .collect()
        };
        assert_eq!(
            projects(search_projects(&pg, &one, None).await.expect("htui")),
            [htui_core::fixtures::ids::PROJECT_HTUI]
        );
        let platform = SearchOptions {
            project: Some("@platform".into()),
            ..options.clone()
        };
        let at_platform = search_projects(&pg, &platform, None)
            .await
            .expect("@platform");
        assert_eq!(
            at_platform
                .iter()
                .map(|target| target.slug.as_str())
                .collect::<Vec<_>>(),
            ["platform"]
        );
        assert_eq!(
            projects(at_platform),
            [htui_core::fixtures::ids::PROJECT_SCOPE_PLATFORM]
        );
        let nope = SearchOptions {
            project: Some("@nope".into()),
            ..options.clone()
        };
        let refused = search_projects(&pg, &nope, None)
            .await
            .expect_err("an unknown workspace slug");
        assert_eq!(refused.error.code, ErrorCode::UnknownProject, "{refused:?}");
        // MOD-147 M4 R1 L3: an `@` slug names a workspace, so the sentence does too.
        assert_eq!(
            refused.error.message, "no workspace with slug `nope`",
            "{refused:?}"
        );
        pg.close().await;
        db.drop_db().await;
    }

    /// MOD-147 M4 D7: a workspace with no member projects is still scoped when it has a scope
    /// row, so its own `WS` items and documents are indexed and searched.
    #[tokio::test]
    async fn scopes_keep_a_workspace_whose_only_project_is_its_scope_row() {
        use htui_core::model::{NewWorkspace, WorkspaceId};
        use htui_core::store::WriteStore;
        let Some(db) = htui_store::testkit::demo_db().await else {
            return;
        };
        let lone = db
            .store
            .create_workspace(NewWorkspace {
                id: WorkspaceId::new(),
                slug: "lone".into(),
                name: "Lone".into(),
                description: String::new(),
                created_by: htui_core::fixtures::ids::USER,
            })
            .await
            .expect("create a workspace without members");
        let row = db
            .store
            .scope_project(lone.id)
            .await
            .expect("the scope-row lookup")
            .expect("create_workspace writes the scope row");
        let every = scopes(&db.store, None).await.expect("every scope");
        assert!(
            every.contains(&Scope {
                workspace_id: lone.id,
                project_ids: vec![row],
            }),
            "{every:?}"
        );
        assert_eq!(
            scopes(&db.store, Some("@lone")).await.expect("@lone"),
            [Scope {
                workspace_id: lone.id,
                project_ids: vec![row],
            }]
        );
        db.drop_db().await;
    }

    /// MOD-159 M1 D9: `--cwd` searches its project in every workspace that holds it, each in
    /// that workspace's own collection.
    #[tokio::test]
    async fn a_cwd_search_reads_every_workspace_of_its_project() {
        use htui_core::fixtures::ids;
        use htui_core::model::{NewRepo, RepoBoxPath, RepoId, WorkspaceProject};
        use htui_core::store::WriteStore;
        let Some(db) = htui_store::testkit::demo_db().await else {
            return;
        };
        db.store
            .upsert_workspace_project(&WorkspaceProject {
                workspace_id: ids::WORKSPACE_GRAPHICS,
                project_id: ids::PROJECT_HTUI,
                position: 9,
            })
            .await
            .expect("htui joins graphics too");
        let checkout = tempfile::tempdir().expect("a checkout");
        let repo = db
            .store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "d9-cwd".into(),
                remote_url: None,
                default_branch: "main".into(),
                is_primary: false,
            })
            .await
            .expect("a repo");
        db.store
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: repo.id,
                box_id: ids::BOX,
                local_path: std::fs::canonicalize(checkout.path())
                    .expect("canonical")
                    .display()
                    .to_string(),
                updated_at: Utc::now(),
            })
            .await
            .expect("this box's checkout");
        let pg = PgStore::connect_read_only(&db.url, Some(ids::BOX), CONNECT_TIMEOUT)
            .await
            .expect("connect read-only");
        let identity = Identity {
            box_id: ids::BOX,
            hostname: "DESKTOP-HTUI".into(),
        };
        let options = SearchOptions {
            query: "closed items".into(),
            project: None,
            decisions: false,
            limit: DEFAULT_LIMIT,
            cwd: Some(checkout.path().to_path_buf()),
            json: true,
        };
        let targets = search_projects(&pg, &options, Some(&identity))
            .await
            .expect("the cwd's project");
        let mut holding: Vec<_> = pg
            .workspaces()
            .await
            .expect("workspaces")
            .into_iter()
            .filter(|ws| {
                ws.projects
                    .iter()
                    .any(|p| p.project_id == ids::PROJECT_HTUI)
            })
            .map(|ws| SearchTarget {
                slug: ws.slug,
                scope: Scope {
                    workspace_id: ws.workspace_id,
                    project_ids: vec![ids::PROJECT_HTUI],
                },
            })
            .collect();
        assert!(holding.len() >= 2, "{holding:?}");
        assert_eq!(targets, std::mem::take(&mut holding));
        pg.close().await;
        db.drop_db().await;
    }

    /// One hit of `key` at `score`.
    fn scored(key: &str, score: f32) -> Hit {
        Hit {
            key: key.into(),
            snippet: key.into(),
            score,
            ..hit(PointType::Item, None)
        }
    }

    /// MOD-159 M1 D9: hits from some collections are the answer, merged best first; the
    /// workspaces whose collection is missing or refused are named, the global one never is.
    #[test]
    fn a_search_names_the_workspaces_whose_collection_it_could_not_read() {
        let (answer, unavailable, _) = combine(
            vec![
                (Some("htui".into()), Part::Hits(vec![scored("A-1", 0.4)])),
                (Some("platform".into()), Part::Missing),
                (
                    Some("lone".into()),
                    Part::Failed(IndexAnswer::Refused("mismatch".into())),
                ),
                (None, Part::Hits(vec![scored("GLOBAL-22", 0.9)])),
            ],
            10,
        );
        let IndexAnswer::Hits(hits) = answer else {
            panic!("hits from the collections that answered: {answer:?}");
        };
        assert_eq!(
            hits.iter().map(|hit| hit.key.as_str()).collect::<Vec<_>>(),
            ["GLOBAL-22", "A-1"]
        );
        assert_eq!(unavailable, ["platform", "lone"]);
        let (answer, unavailable, _) = combine(
            vec![
                (Some("htui".into()), Part::Hits(vec![scored("A-1", 0.4)])),
                (None, Part::Missing),
            ],
            10,
        );
        assert!(matches!(answer, IndexAnswer::Hits(_)), "{answer:?}");
        assert!(unavailable.is_empty(), "the global one is never named");
    }

    /// MOD-159 M1 R2: the global collection, empty on most boxes, always answers, so it never
    /// counts as a workspace that answered: a failed workspace beside it is the failure, its exit
    /// as before. One workspace answering beside a failed one is hits, the failed one named.
    #[test]
    fn the_global_collection_never_hides_a_failed_workspace() {
        let mismatch = IndexAnswer::Refused(htui_core::qdrant::unavailable(
            "embedder mismatch: collection `htui_ws_1` holds vectors from hash/384",
        ));
        let (answer, unavailable, _) = combine(
            vec![
                (Some("lone".into()), Part::Failed(mismatch.clone())),
                (None, Part::Hits(Vec::new())),
            ],
            10,
        );
        assert_eq!(answer, mismatch);
        assert!(unavailable.is_empty(), "{unavailable:?}");
        let (answer, unavailable, _) = combine(
            vec![
                (Some("lone".into()), Part::Failed(mismatch.clone())),
                (None, Part::Hits(vec![scored("GLOBAL-1", 0.9)])),
            ],
            10,
        );
        assert_eq!(
            answer, mismatch,
            "global hits do not answer for a workspace"
        );
        assert!(unavailable.is_empty(), "{unavailable:?}");
        let (answer, unavailable, cause) = combine(
            vec![
                (Some("htui".into()), Part::Hits(vec![scored("A-1", 0.4)])),
                (Some("lone".into()), Part::Failed(mismatch.clone())),
                (None, Part::Hits(Vec::new())),
            ],
            10,
        );
        assert!(matches!(answer, IndexAnswer::Hits(_)), "{answer:?}");
        assert_eq!(unavailable, ["lone"]);
        assert_eq!(
            cause.as_deref(),
            failure_sentence(&mismatch).as_deref(),
            "the stderr note carries why"
        );
        assert!(
            cause.is_some_and(|cause| cause.contains("embedder mismatch")),
            "the failure's own sentence"
        );
    }

    /// D9: every collection missing is the empty index, as one missing collection was; no hits and
    /// a failure is that failure, its exit unchanged.
    #[test]
    fn a_search_with_no_collection_is_empty_and_one_with_only_failures_fails() {
        let (answer, unavailable, _) = combine(
            vec![(Some("htui".into()), Part::Missing), (None, Part::Missing)],
            10,
        );
        assert_eq!(answer, IndexAnswer::Empty);
        assert!(unavailable.is_empty(), "{unavailable:?}");
        let (answer, unavailable, _) = combine(
            vec![
                (Some("htui".into()), Part::Missing),
                (
                    Some("platform".into()),
                    Part::Failed(IndexAnswer::Failed("no model".into())),
                ),
                (None, Part::Missing),
            ],
            10,
        );
        assert_eq!(answer, IndexAnswer::Failed("no model".into()));
        assert!(unavailable.is_empty(), "{unavailable:?}");
    }

    /// Review R1 M2: a refused API key is `qdrant_refused` (not "cannot reach"), a query the model
    /// fails on is the model's failure, a mismatch is a refusal, and the rest is offline.
    #[test]
    fn index_refusals_by_cause() {
        use htui_store::vector::{EMBEDDER_MISMATCH, QDRANT_REFUSED, QUERY_EMBED_FAILED};
        let refused = index_refusal(StoreError::Backend(format!(
            "{QDRANT_REFUSED}: collection exists: Invalid api-key; check the Qdrant API key in \
             Settings > Qdrant"
        )));
        let IndexAnswer::AccessRefused(sentence) = &refused else {
            panic!("{refused:?}");
        };
        assert!(!sentence.contains("cannot reach"), "{sentence}");
        assert!(sentence.contains("API key"), "{sentence}");
        assert!(matches!(
            index_refusal(StoreError::Backend(format!(
                "{QUERY_EMBED_FAILED}: backend error: the session failed"
            ))),
            IndexAnswer::Failed(_)
        ));
        assert!(matches!(
            index_refusal(StoreError::Backend(format!(
                "{EMBEDDER_MISMATCH}: collection `c` holds vectors from hash/384"
            ))),
            IndexAnswer::Refused(_)
        ));
        let IndexAnswer::Offline(sentence) = index_refusal(StoreError::Backend(
            "qdrant: collection exists: transport error".into(),
        )) else {
            panic!("a Qdrant that does not answer is offline");
        };
        assert!(
            sentence.starts_with("knowledge search unavailable: cannot reach Qdrant: "),
            "{sentence}"
        );
    }

    /// MOD-160 D8: every index refusal a search answers says the sentence first, then today's
    /// cause after a colon; the kind (and so the code and the exit) is unchanged.
    #[test]
    fn index_refusal_says_knowledge_search_unavailable_first() {
        use htui_store::vector::{EMBEDDER_MISMATCH, QDRANT_REFUSED, QUERY_EMBED_FAILED};
        const SENTENCE: &str = "knowledge search unavailable: ";
        let refused = index_refusal(StoreError::Backend(format!(
            "{QDRANT_REFUSED}: collection exists: Invalid api-key"
        )));
        let IndexAnswer::AccessRefused(message) = &refused else {
            panic!("{refused:?}");
        };
        assert!(message.starts_with(SENTENCE), "{message}");
        assert!(message.contains(QDRANT_REFUSED), "{message}");

        let failed = index_refusal(StoreError::Backend(format!(
            "{QUERY_EMBED_FAILED}: backend error: the session failed"
        )));
        let IndexAnswer::Failed(message) = &failed else {
            panic!("{failed:?}");
        };
        assert!(
            message.starts_with(&format!(
                "{SENTENCE}the embedding model failed on the query: "
            )),
            "{message}"
        );

        let mismatch = index_refusal(StoreError::Backend(format!(
            "{EMBEDDER_MISMATCH}: collection `c` holds vectors from hash/384"
        )));
        let IndexAnswer::Refused(message) = &mismatch else {
            panic!("{mismatch:?}");
        };
        assert!(message.starts_with(SENTENCE), "{message}");
        assert!(message.contains(EMBEDDER_MISMATCH), "{message}");

        let offline = index_refusal(StoreError::Backend(
            "qdrant: collection exists: transport error".into(),
        ));
        let IndexAnswer::Offline(message) = &offline else {
            panic!("{offline:?}");
        };
        assert_eq!(
            message,
            "knowledge search unavailable: cannot reach Qdrant: store backend error: qdrant: \
             collection exists: transport error"
        );
    }

    /// MOD-160 D8: a search whose Qdrant settings do not resolve keeps its code, its `store` and
    /// so its exit, and says the sentence first, then today's cause.
    #[test]
    fn unresolved_failures_keep_their_code_and_say_unavailable() {
        use crate::state_cmd::StoreState;
        use concepts_worker::Unresolved;
        use htui_store::secret::QdrantKeyError;
        let cases = [
            (
                Unresolved::NoUrl,
                StoreState::Refused,
                ErrorCode::NoQdrantUrl,
                NO_QDRANT_URL.to_owned(),
            ),
            (
                Unresolved::Unusable("HTUI_QDRANT_URL is not a URL".into()),
                StoreState::Refused,
                ErrorCode::QdrantUrlUnusable,
                "HTUI_QDRANT_URL is not a URL".to_owned(),
            ),
            (
                Unresolved::Read(StoreError::Unreachable("refused".into())),
                StoreState::Offline,
                ErrorCode::Unreachable,
                "cannot reach Postgres: ".to_owned(),
            ),
            (
                Unresolved::Read(StoreError::Backend("the row is odd".into())),
                StoreState::Error,
                ErrorCode::ReadFailed,
                String::new(),
            ),
            (
                Unresolved::Key(QdrantKeyError::Keyring("locked".into())),
                StoreState::Refused,
                ErrorCode::KeyringUnreadable,
                "the Qdrant API key could not be read from the OS keyring: locked".to_owned(),
            ),
            (
                Unresolved::Offline,
                StoreState::Offline,
                ErrorCode::Unreachable,
                "cannot reach Postgres: ".to_owned(),
            ),
        ];
        for (why, store, code, cause) in cases {
            let shown = format!("{why:?}");
            let failure = unresolved_failure(why);
            assert_eq!(failure.store, store, "{shown}");
            assert_eq!(failure.error.code, code, "{shown}");
            let message = &failure.error.message;
            assert!(
                message.starts_with(&format!("knowledge search unavailable: {cause}")),
                "{shown}: {message}"
            );
            assert_eq!(
                message.matches("knowledge search unavailable").count(),
                1,
                "{message}"
            );
        }
    }

    /// MOD-160 D8: a Qdrant that does not answer is `offline` (exit 3), says the sentence first,
    /// and loads no model.
    #[tokio::test]
    async fn search_index_against_a_closed_port_is_offline_unavailable_and_loads_nothing() {
        use htui_store::embed::HashEmbedder;
        let settings = QdrantSettings::new(
            htui_core::qdrant::QdrantUrl::parse("http://127.0.0.1:1").expect("a valid URL"),
            None,
        );
        let targets = [SearchTarget {
            slug: "platform".into(),
            scope: Scope {
                workspace_id: htui_core::model::WorkspaceId::new(),
                project_ids: vec![ProjectId(Uuid::nil())],
            },
        }];
        let options = SearchOptions {
            query: "closed items".into(),
            project: None,
            decisions: false,
            limit: 5,
            cwd: None,
            json: true,
        };
        let load = || -> std::future::Ready<anyhow::Result<HashEmbedder>> {
            panic!("no model for a Qdrant that does not answer")
        };
        let (answer, unavailable, _) = search_index(&settings, "c", &targets, &options, load).await;
        assert!(
            unavailable.is_empty(),
            "a failure names nobody: {unavailable:?}"
        );
        let IndexAnswer::Offline(message) = &answer else {
            panic!("a closed port is offline: {answer:?}");
        };
        assert!(
            message.starts_with("knowledge search unavailable: cannot reach Qdrant: "),
            "{message}"
        );
    }

    /// `--index-items` and the worker say the same: a refused key is not "cannot reach".
    #[test]
    fn a_refused_api_key_is_not_cannot_reach_qdrant() {
        use htui_store::vector::QDRANT_REFUSED;
        let err = StoreError::Backend(format!("{QDRANT_REFUSED}: collection exists: denied"));
        let text = format!("{:#}", qdrant_refusal(err));
        assert!(text.contains(QDRANT_REFUSED), "{text}");
        assert!(!text.contains("cannot reach"), "{text}");
    }

    #[test]
    fn report_line_is_index_items_wording() {
        let report = SyncReport {
            items_rebuilt: 1,
            items_unchanged: 2,
            requirements_rebuilt: 3,
            requirements_unchanged: 4,
            points_upserted: 5,
            points_deleted: 6,
            abandoned: false,
        };
        assert_eq!(
            report_line(&report),
            "indexed: 1 item(s) rebuilt, 2 unchanged; 3 requirement(s) rebuilt, 4 unchanged; \
             5 point(s) written, 6 removed"
        );
    }

    /// The search runtime shares one loaded model between its tasks (MOD-64 D238), the index job
    /// keeps one across retries, and both move it across tasks; `htui-store`'s own tests build
    /// without `local-embed`, so the check lives here (D258).
    #[test]
    fn rten_embedder_is_clone_send_and_sync() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        shared::<RtenEmbedder>();
    }

    /// The index job (MOD-41 plan D19, MOD-159 M1 plan D7) and `--index-items`' body (D12) over
    /// `MemStore` and one `MemVectorStore` per collection, under paused time.
    mod index_job {
        use super::super::*;
        use chrono::{DateTime, SubsecRound as _, Utc};
        use htui_core::clock::Clock;
        use htui_core::fixtures::ids;
        use htui_core::model::document::GOALS_KIND;
        use htui_core::model::{
            DocumentId, IndexCursor, ItemId, NewItem, NewOwnerDocument, RequirementPatch,
            TIMESTAMPTZ_DIGITS,
        };
        use htui_core::store::WriteStore as _;
        use htui_store::vector::{ConceptPoint, IndexedPoint, MemVectorStore, Subject};
        use serde_json::json;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use uuid::Uuid;

        const MINUTE: Duration = Duration::from_secs(60);
        /// The default beat: the run lease's refresh, a third of its 120 s TTL.
        const BEAT: Duration = Duration::from_secs(40);

        const PLATFORM: IndexedCollection = IndexedCollection::Workspace(ids::WORKSPACE_PLATFORM);
        const GRAPHICS: IndexedCollection = IndexedCollection::Workspace(ids::WORKSPACE_GRAPHICS);

        /// Every collection of the demo, in name order.
        fn every_collection() -> Vec<IndexedCollection> {
            let mut every = vec![PLATFORM, GRAPHICS, IndexedCollection::Global];
            every.sort_by_key(IndexedCollection::name);
            every
        }

        /// Wall time that moves with tokio's paused clock, so leases lapse when a test advances.
        #[derive(Debug)]
        struct TokioClock {
            base: DateTime<Utc>,
            start: tokio::time::Instant,
        }

        impl Clock for TokioClock {
            fn now(&self) -> DateTime<Utc> {
                let elapsed = TimeDelta::from_std(self.start.elapsed()).expect("a test is short");
                (self.base + elapsed).trunc_subsecs(TIMESTAMPTZ_DIGITS)
            }
        }

        /// The demo, its clock tokio's.
        fn demo() -> MemStore {
            MemStore::demo().with_clock(Arc::new(TokioClock {
                base: Utc::now(),
                start: tokio::time::Instant::now(),
            }))
        }

        /// Hands `collection`'s lease from `from` to `to` (both on the demo's box) the first time
        /// the store it is armed on is read: another box taking it over in the middle of a sync.
        struct Takeover {
            leases: MemStore,
            collection: IndexedCollection,
            from: Uuid,
            to: Uuid,
        }

        /// `MemVectorStore`, counting the syncs that reach it (one `indexed` read of the htui
        /// project per whole sync of Platform's collection) and every `indexed` read, failing the
        /// first `fail` upserts, and running an armed [`Takeover`] at its next read.
        #[derive(Default)]
        struct Counting {
            inner: MemVectorStore,
            syncs: AtomicUsize,
            reads: AtomicUsize,
            /// `indexed` reads of the agy project: a full sync of Platform reads every project,
            /// an incremental pass (MOD-159 M2) only a changed one, and no test changes agy.
            agy: AtomicUsize,
            fail: AtomicUsize,
            takeover: Mutex<Option<Takeover>>,
            /// How long its next read of the htui project sleeps (a sync longer than the TTL).
            pause: Mutex<Option<Duration>>,
        }

        impl Counting {
            fn syncs(&self) -> usize {
                self.syncs.load(Ordering::SeqCst)
            }

            fn reads(&self) -> usize {
                self.reads.load(Ordering::SeqCst)
            }

            /// The full syncs of Platform's collection that reached this store (see `agy`).
            fn full_syncs(&self) -> usize {
                self.agy.load(Ordering::SeqCst)
            }
        }

        impl VectorStore for Counting {
            async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError> {
                let failing = self
                    .fail
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                if failing {
                    return Err(StoreError::Backend(
                        "qdrant: upsert: planted failure".into(),
                    ));
                }
                self.inner.upsert(points).await
            }

            async fn delete(&self, ids: Vec<Uuid>) -> Result<(), StoreError> {
                self.inner.delete(ids).await
            }

            async fn indexed(&self, project: ProjectId) -> Result<Vec<IndexedPoint>, StoreError> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                if project == ids::PROJECT_AGY {
                    self.agy.fetch_add(1, Ordering::SeqCst);
                }
                if project == ids::PROJECT_HTUI {
                    self.syncs.fetch_add(1, Ordering::SeqCst);
                    let pause = self.pause.lock().expect("not poisoned").take();
                    if let Some(pause) = pause {
                        tokio::time::sleep(pause).await;
                    }
                }
                let armed = self.takeover.lock().expect("not poisoned").take();
                if let Some(t) = armed {
                    let lapsed = t
                        .leases
                        .renew_indexer_lease(&t.collection, t.from, TimeDelta::zero())
                        .await
                        .expect("the memory store never fails");
                    assert!(lapsed, "the victim held it");
                    let taken = t
                        .leases
                        .take_indexer_lease(&t.collection, ids::BOX, t.to, TimeDelta::seconds(120))
                        .await
                        .expect("the memory store never fails");
                    assert_eq!(taken, LeaseTake::Taken);
                }
                self.inner.indexed(project).await
            }

            async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError> {
                self.inner.search(query).await
            }
        }

        /// The job's index: one [`Counting`] per collection, every open recorded with its URL,
        /// the first `fail_opens` opens failing, every `forget` recorded (`true` for a store whose
        /// sync failed, `false` for "drop every connection"), and the legacy drops counted.
        #[derive(Default)]
        struct Fakes {
            stores: Mutex<HashMap<IndexedCollection, Arc<Counting>>>,
            opens: Mutex<Vec<(String, IndexedCollection)>>,
            fail_opens: AtomicUsize,
            fail_upserts: usize,
            forgotten: Mutex<Vec<bool>>,
            synced: AtomicUsize,
            legacy: AtomicUsize,
        }

        impl Fakes {
            /// Fakes whose stores each fail their first upsert.
            fn syncing_badly_once() -> Self {
                Self {
                    fail_upserts: 1,
                    ..Self::default()
                }
            }

            fn store(&self, collection: IndexedCollection) -> Arc<Counting> {
                let mut stores = self.stores.lock().expect("not poisoned");
                Arc::clone(stores.entry(collection).or_insert_with(|| {
                    Arc::new(Counting {
                        fail: AtomicUsize::new(self.fail_upserts),
                        ..Counting::default()
                    })
                }))
            }

            /// The collections opened, in order, repeats kept.
            fn opened(&self) -> Vec<IndexedCollection> {
                let opens = self.opens.lock().expect("not poisoned");
                opens.iter().map(|(_, collection)| *collection).collect()
            }

            /// The collections opened at least once, in name order.
            fn opened_once(&self) -> Vec<IndexedCollection> {
                let mut opened = self.opened();
                opened.sort_by_key(IndexedCollection::name);
                opened.dedup();
                opened
            }

            fn urls(&self) -> Vec<String> {
                let opens = self.opens.lock().expect("not poisoned");
                opens.iter().map(|(url, _)| url.clone()).collect()
            }

            fn forgotten(&self) -> Vec<bool> {
                self.forgotten.lock().expect("not poisoned").clone()
            }

            fn legacy_drops(&self) -> usize {
                self.legacy.load(Ordering::SeqCst)
            }

            /// How often the job's own connections were dropped (an unreadable key).
            fn synced_forgets(&self) -> usize {
                self.synced.load(Ordering::SeqCst)
            }
        }

        impl JobIndex for Fakes {
            type Store = Counting;

            async fn open(
                &self,
                settings: QdrantSettings,
                collection: &IndexedCollection,
            ) -> Result<Arc<Counting>, String> {
                self.opens
                    .lock()
                    .expect("not poisoned")
                    .push((settings.url.to_string(), *collection));
                let failing = self
                    .fail_opens
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_ok();
                if failing {
                    return Err("cannot reach Qdrant: planted failure".to_owned());
                }
                Ok(self.store(*collection))
            }

            fn forget(&self, store: Option<&Arc<Counting>>) {
                self.forgotten
                    .lock()
                    .expect("not poisoned")
                    .push(store.is_some());
            }

            fn forget_synced(&self) {
                self.synced.fetch_add(1, Ordering::SeqCst);
            }

            async fn drop_legacy(&self, _: &QdrantSettings) -> Result<bool, String> {
                Ok(self.legacy.fetch_add(1, Ordering::SeqCst) == 0)
            }
        }

        /// A pool that has never connected: the job's start must not need it.
        fn unreachable_store() -> PgStore {
            let identity = Identity {
                box_id: BoxId::new(),
                hostname: "HTUI-TEST".to_owned(),
            };
            PgStore::lazy(
                "postgres://nobody:nothing@127.0.0.1:1/none",
                &identity,
                Duration::from_millis(250),
            )
            .expect("a lazy pool opens no socket")
        }

        /// The job's settings source over `store`, no override, a keyring holding no key.
        fn settings_over(store: &MemStore) -> concepts_worker::QdrantSource {
            let source =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(|| Ok(None));
            source.publish(Some(Arc::new(store.clone())));
            source
        }

        /// Sets `store`'s `qdrant_url` row to `text`, over whatever it holds.
        async fn set_row(store: &MemStore, text: &str) {
            let token = store
                .qdrant_url()
                .await
                .expect("the memory store never fails")
                .map(|row| row.token);
            store
                .set_qdrant_url(
                    &htui_core::qdrant::QdrantUrl::parse(text).expect("a valid URL"),
                    token,
                )
                .await
                .expect("the memory store never fails");
        }

        /// The demo, its URL row set: a store the job indexes from.
        async fn indexable() -> MemStore {
            let read = demo();
            set_row(&read, "http://qdrant:6334").await;
            read
        }

        /// `collection`'s lease as people see it.
        async fn lease(read: &MemStore, collection: &IndexedCollection) -> Option<IndexerLease> {
            read.indexer_lease(collection)
                .await
                .expect("the memory store never fails")
        }

        /// `index_job(read, …)` as `owner`, spawned over `index`.
        fn spawn_job(read: &MemStore, index: &Arc<Fakes>, owner: Uuid) -> IndexJob<MemStore> {
            let (store, index) = (read.clone(), Arc::clone(index));
            let task = tokio::spawn(async move {
                let settings = settings_over(&store);
                match index_job(&store, &settings, &*index, owner).await {}
            });
            IndexJob {
                task,
                leases: read.clone(),
                owner,
            }
        }

        /// D7: two jobs over one store, each with its own token: the first to beat takes every
        /// collection and indexes it, the other opens nothing. Once the holder stops, its leases
        /// are let go and the other takes every collection over on its next beat.
        #[tokio::test(start_paused = true)]
        async fn one_of_two_jobs_indexes_each_collection_and_the_other_takes_over_after_a_stop() {
            let read = indexable().await;
            let (first, second) = (Arc::new(Fakes::default()), Arc::new(Fakes::default()));
            let first_job = spawn_job(&read, &first, Uuid::now_v7());
            tokio::time::sleep(Duration::from_secs(1)).await;
            let second_job = spawn_job(&read, &second, Uuid::now_v7());
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert_eq!(first.opened_once(), every_collection());
            assert!(
                !first.store(PLATFORM).inner.points().is_empty(),
                "the holder indexed Platform"
            );
            assert!(
                !first.store(GRAPHICS).inner.points().is_empty(),
                "and Graphics"
            );
            tokio::time::sleep(2 * BEAT).await;
            assert!(second.opened().is_empty(), "the other job opened nothing");
            for collection in every_collection() {
                assert!(
                    lease(&read, &collection).await.is_some_and(|l| l.live),
                    "{collection} is held"
                );
            }

            first_job.stop(WORKER_RELEASE_WAIT).await;
            for collection in every_collection() {
                assert!(
                    !lease(&read, &collection).await.is_some_and(|l| l.live),
                    "{collection} was let go at the stop"
                );
            }
            tokio::time::sleep(BEAT).await;
            assert_eq!(
                second.opened_once(),
                every_collection(),
                "the other job took every collection over on its next beat"
            );
            assert!(!second.store(PLATFORM).inner.points().is_empty());
            assert!(!second_job.is_finished(), "it runs until it is stopped");
            second_job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// D6, D7: a holder whose lease another box takes during its sync deletes nothing (the
        /// renewal before the delete batch touches no row), stops holding the collection, and does
        /// not sync it again while the other box holds it.
        #[tokio::test(start_paused = true)]
        async fn a_holder_whose_lease_is_taken_over_mid_sync_deletes_nothing_and_stops_holding() {
            let read = indexable().await;
            let settings = settings_over(&read);
            let index = Fakes::default();
            let (victim, thief) = (Uuid::now_v7(), Uuid::now_v7());
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, victim, &mut state).await;
            assert_eq!(state.held(), every_collection());
            let platform = index.store(PLATFORM);
            // A point of an item that is gone: the next reconcile's delete batch holds it.
            let mut ghost = platform
                .inner
                .points()
                .into_iter()
                .find(|p| matches!(p.subject, Subject::Item { .. }))
                .expect("an item point");
            if let Subject::Item { id, .. } = &mut ghost.subject {
                *id = ItemId::new();
            }
            platform.upsert(vec![ghost]).await.expect("plant the ghost");
            let before = platform.inner.points();
            *platform.takeover.lock().expect("not poisoned") = Some(Takeover {
                leases: read.clone(),
                collection: PLATFORM,
                from: victim,
                to: thief,
            });

            tokio::time::advance(15 * MINUTE).await;
            index_beat(&read, &settings, &index, victim, &mut state).await;
            assert_eq!(platform.inner.points(), before, "nothing was deleted");
            assert!(
                !state.held().contains(&PLATFORM),
                "the victim no longer holds Platform: {:?}",
                state.held()
            );
            assert!(state.held().contains(&GRAPHICS), "{:?}", state.held());
            let syncs = platform.syncs();
            tokio::time::advance(15 * MINUTE).await;
            // The thief's lease lapsed meanwhile (nobody renews it here): renewed, it stays live.
            read.renew_indexer_lease(&PLATFORM, thief, TimeDelta::seconds(120))
                .await
                .expect("the memory store never fails");
            index_beat(&read, &settings, &index, victim, &mut state).await;
            assert_eq!(platform.syncs(), syncs, "no sync under another's lease");
            assert!(!state.held().contains(&PLATFORM));
        }

        /// D7: no URL takes no lease, opens nothing and loads no model; and settings that stop
        /// resolving drop the cached connections (MOD-158 R1, B-L1).
        #[tokio::test(start_paused = true)]
        async fn a_job_with_no_url_never_takes_a_lease_and_loads_no_model() {
            let read = demo();
            let settings = settings_over(&read);
            let index = Fakes::default();
            let mut state = JobState::default();
            for _ in 0..3 {
                let pause = index_beat(&read, &settings, &index, Uuid::now_v7(), &mut state).await;
                assert_eq!(pause, BEAT, "the beat is the lease refresh");
            }
            assert!(index.opened().is_empty(), "never opened without a URL");
            assert!(state.held().is_empty());
            for collection in every_collection() {
                assert_eq!(lease(&read, &collection).await, None, "{collection}");
            }
            assert_eq!(index.forgotten(), [false, false, false]);
        }

        /// D7: the lease is renewed every beat, and each collection reconciled at most once per
        /// `concepts_sync_minutes` (15 by default, read every beat).
        #[tokio::test(start_paused = true)]
        async fn the_job_syncs_a_collection_once_per_interval_while_renewing_every_beat() {
            let read = indexable().await;
            let index = Arc::new(Fakes::default());
            let job = spawn_job(&read, &index, Uuid::now_v7());
            tokio::time::sleep(Duration::from_secs(1)).await;
            let platform = index.store(PLATFORM);
            assert_eq!(platform.syncs(), 1, "one sync at start");
            let mut expiry = lease(&read, &PLATFORM).await.and_then(|l| l.expires_at);
            // Beats at 40 s, 80 s, ..., 880 s: each renews, none syncs.
            for beat in 1..=22 {
                tokio::time::sleep(BEAT).await;
                let now = lease(&read, &PLATFORM).await.and_then(|l| l.expires_at);
                assert!(now > expiry, "beat {beat} renewed the lease");
                expiry = now;
                assert_eq!(platform.syncs(), 1, "beat {beat}: not before 15 minutes");
            }
            tokio::time::sleep(BEAT).await;
            assert_eq!(
                platform.syncs(),
                2,
                "the second sync, at the first beat after 15 minutes (920 s)"
            );

            read.set_app_setting(SYNC_MINUTES_KEY, json!(1));
            tokio::time::sleep(2 * BEAT).await;
            assert_eq!(
                platform.syncs(),
                3,
                "the new minute applies from the next beat"
            );
            job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// D10: the holder of the global collection creates it and drops the legacy collection,
        /// once per process; a workspace collection's holder never does.
        #[tokio::test(start_paused = true)]
        async fn the_global_holder_drops_the_legacy_collection_once() {
            let read = indexable().await;
            read.set_app_setting(SYNC_MINUTES_KEY, json!(1));
            let index = Arc::new(Fakes::default());
            let job = spawn_job(&read, &index, Uuid::now_v7());
            tokio::time::sleep(10 * MINUTE).await;
            let global = index
                .opened()
                .into_iter()
                .filter(|c| *c == IndexedCollection::Global)
                .count();
            assert!(
                global > 1,
                "the global collection is opened every interval: {global}"
            );
            assert_eq!(
                index.legacy_drops(),
                1,
                "the legacy collection is dropped once"
            );
            job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// MOD-158 R1 (B-L1): a sync that fails forgets the connection it used, so the next
        /// reconnects; a sync that succeeds forgets nothing. The failure waits for the interval.
        #[tokio::test(start_paused = true)]
        async fn a_failed_sync_forgets_its_connection_and_retries_next_interval() {
            let read = indexable().await;
            let settings = settings_over(&read);
            let index = Fakes::syncing_badly_once();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(index.store(PLATFORM).inner.points().is_empty(), "failed");
            assert_eq!(index.forgotten(), [true, true], "each failed sync's store");
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(
                index.store(PLATFORM).inner.points().is_empty(),
                "no retry before the interval"
            );
            tokio::time::advance(15 * MINUTE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(!index.store(PLATFORM).inner.points().is_empty(), "retried");
            assert_eq!(
                index.forgotten(),
                [true, true],
                "a good sync forgets nothing"
            );
        }

        /// MOD-158 D7: a URL set while the job runs is used by its next beat, no restart; a
        /// changed URL is what the next interval opens.
        #[tokio::test(start_paused = true)]
        async fn a_url_set_or_changed_after_the_job_starts_is_used_without_a_restart() {
            let read = demo();
            let index = Arc::new(Fakes::default());
            let job = spawn_job(&read, &index, Uuid::now_v7());
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert!(index.opened().is_empty(), "no URL at start");
            set_row(&read, "http://first:6334").await;
            tokio::time::sleep(BEAT).await;
            assert_eq!(index.opened_once(), every_collection());
            assert!(
                index.urls().iter().all(|url| url == "http://first:6334"),
                "{:?}",
                index.urls()
            );
            set_row(&read, "http://second:6334").await;
            // Opened at the 40 s beat, so due again at the 960 s one.
            tokio::time::sleep(15 * MINUTE + BEAT).await;
            assert!(
                index.urls().iter().any(|url| url == "http://second:6334"),
                "{:?}",
                index.urls()
            );
            job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// MOD-158 D7: a failed open is a log line; the collection is opened again at the next
        /// interval.
        #[tokio::test(start_paused = true)]
        async fn a_failed_open_is_retried_next_interval() {
            let read = indexable().await;
            let settings = settings_over(&read);
            let index = Fakes {
                fail_opens: AtomicUsize::new(3),
                ..Fakes::default()
            };
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(index.opened().len(), 3, "each collection tried once");
            assert_eq!(index.store(PLATFORM).reads(), 0, "nothing synced");
            tokio::time::advance(15 * MINUTE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(index.opened().len(), 6, "opened again");
            assert_eq!(index.store(PLATFORM).syncs(), 1);
        }

        /// MOD-159 M1 R2: a reconcile longer than the TTL (a first build) does not let the job's
        /// other leases lapse, nor the one it syncs under: they are renewed every refresh while it
        /// runs, so another owner's take finds each one held.
        #[tokio::test(start_paused = true)]
        async fn a_long_reconcile_keeps_every_held_lease_live() {
            let read = indexable().await;
            read.set_app_setting(SYNC_MINUTES_KEY, json!(1));
            let settings = settings_over(&read);
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, owner, &mut state).await;
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            tokio::time::advance(BEAT).await;
            // Every collection is due again; Platform's sync now takes five minutes.
            *index.store(PLATFORM).pause.lock().expect("not poisoned") = Some(5 * MINUTE);
            let other = Uuid::now_v7();
            let probe = async {
                tokio::time::sleep(4 * MINUTE).await;
                assert_eq!(index.store(PLATFORM).syncs(), 2, "still in the long sync");
                for collection in every_collection() {
                    let take = read
                        .take_indexer_lease(&collection, ids::BOX, other, TimeDelta::seconds(120))
                        .await
                        .expect("the memory store never fails");
                    assert!(
                        matches!(take, LeaseTake::Held(_)),
                        "{collection} is still held by the job: {take:?}"
                    );
                }
            };
            tokio::join!(
                index_beat(&read, &settings, &index, owner, &mut state),
                probe
            );
            assert_eq!(state.held(), every_collection());
        }

        /// MOD-159 M1 R2: a beat resolves the URL alone; the key (which may prompt a locked
        /// keyring) is read only when a held collection is due, once for that beat.
        #[tokio::test(start_paused = true)]
        async fn beats_with_nothing_due_read_no_key() {
            static READS: AtomicUsize = AtomicUsize::new(0);
            fn ring() -> htui_core::store::Result<Option<String>> {
                READS.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            }
            let read = indexable().await;
            let settings =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(ring);
            settings.publish(Some(Arc::new(read.clone())));
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                READS.load(Ordering::SeqCst),
                1,
                "one read for the first beat"
            );
            assert_eq!(index.opened_once(), every_collection());
            for _ in 0..5 {
                tokio::time::advance(BEAT).await;
                index_beat(&read, &settings, &index, owner, &mut state).await;
            }
            assert_eq!(
                READS.load(Ordering::SeqCst),
                1,
                "nothing due, no key read; the leases were still renewed"
            );
            assert_eq!(state.held(), every_collection());
            tokio::time::advance(15 * MINUTE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(READS.load(Ordering::SeqCst), 2, "due again, read again");
        }

        /// MOD-159 M1 R2: a key that cannot be read opens nothing and lets every lease go, so a box
        /// with a key takes the indexing over; it drops the job's own connections only (never the
        /// search's), and the job takes nothing, and reads no key, until the interval has passed.
        #[tokio::test(start_paused = true)]
        async fn an_unreadable_key_lets_the_leases_go_and_drops_only_the_jobs_connections() {
            fn locked() -> htui_core::store::Result<Option<String>> {
                Err(StoreError::Backend("the Secret Service is locked".into()))
            }
            let read = indexable().await;
            let settings =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(locked);
            settings.publish(Some(Arc::new(read.clone())));
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            let other = Uuid::now_v7();
            let every_take_is = |want: fn(&LeaseTake) -> bool, why: &'static str| {
                let read = read.clone();
                async move {
                    for collection in every_collection() {
                        let take = read
                            .take_indexer_lease(
                                &collection,
                                ids::BOX,
                                other,
                                TimeDelta::seconds(120),
                            )
                            .await
                            .expect("the memory store never fails");
                        assert!(want(&take), "{collection}: {why}: {take:?}");
                    }
                }
            };
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(index.opened().is_empty(), "nothing opened without the key");
            assert_eq!(state.held(), Vec::new(), "the leases are let go");
            assert_eq!(index.forgotten(), Vec::<bool>::new(), "no forget-all");
            assert_eq!(index.synced_forgets(), 1, "the job's connections, once");
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                index.synced_forgets(),
                1,
                "within the interval, no key read"
            );
            assert_eq!(state.held(), Vec::new(), "nor any lease taken");
            every_take_is(
                |take| matches!(take, LeaseTake::Taken),
                "another box takes the indexing over",
            )
            .await;
            read.release_indexer_leases(other)
                .await
                .expect("the memory store never fails");
            tokio::time::advance(15 * MINUTE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                index.synced_forgets(),
                2,
                "the interval passed: the key is read again"
            );
            assert_eq!(state.held(), Vec::new(), "still unreadable: let go again");
            every_take_is(|take| matches!(take, LeaseTake::Taken), "let go again").await;
        }

        /// MOD-159 M1 R2: each collection's embedder mismatch is an error once, on its own.
        #[test]
        fn each_collections_mismatch_is_an_error_once() {
            use tracing::Level;
            let mut reported = HashSet::new();
            assert_eq!(
                open_failure_level(true, &mut reported, PLATFORM),
                Level::ERROR
            );
            assert_eq!(
                open_failure_level(true, &mut reported, GRAPHICS),
                Level::ERROR,
                "another collection's mismatch is said too"
            );
            assert_eq!(
                open_failure_level(true, &mut reported, PLATFORM),
                Level::DEBUG
            );
            assert_eq!(
                open_failure_level(false, &mut reported, GRAPHICS),
                Level::WARN
            );
        }

        /// The interval's rule: positive whole minutes, else 15.
        #[test]
        fn the_interval_reads_concepts_sync_minutes_with_a_15_minute_default() {
            let default = DEFAULT_SYNC_MINUTES * MINUTE.as_secs();
            assert_eq!(default, 15 * 60);
            let with = |value: Value| BTreeMap::from([(SYNC_MINUTES_KEY.to_owned(), value)]);
            assert_eq!(sync_interval(&BTreeMap::new()).as_secs(), default, "absent");
            for value in [
                json!(0),
                json!(-3),
                json!("5"),
                json!(2.5),
                json!(null),
                json!(true),
            ] {
                assert_eq!(
                    sync_interval(&with(value.clone())).as_secs(),
                    default,
                    "{value}"
                );
            }
            assert_eq!(sync_interval(&with(json!(5))), 5 * MINUTE);
            assert_eq!(
                sync_interval(&with(json!(u64::MAX))),
                Duration::from_secs(u64::MAX),
                "a huge value saturates rather than overflowing"
            );
        }

        /// MOD-159 M2 D5: the overlap's rule, the cache refresh's: positive whole seconds, else
        /// 300.
        #[test]
        fn the_overlap_reads_cache_overlap_seconds_with_a_300_second_default() {
            let default = Duration::from_secs(DEFAULT_OVERLAP_SECONDS);
            assert_eq!(default, 5 * MINUTE);
            let with = |value: Value| BTreeMap::from([(OVERLAP_KEY.to_owned(), value)]);
            assert_eq!(overlap_window(&BTreeMap::new()), default, "absent");
            for value in [
                json!(0),
                json!(-3),
                json!("5"),
                json!(2.5),
                json!(null),
                json!(true),
            ] {
                assert_eq!(overlap_window(&with(value.clone())), default, "{value}");
            }
            assert_eq!(overlap_window(&with(json!(90))), Duration::from_secs(90));
        }

        /// A new htui item, stamped by `read`'s clock.
        async fn mint_item(read: &MemStore, title: &str) -> ItemId {
            read.mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: title.into(),
                body: "a body about the incremental cycle".into(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                target_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the memory store never fails")
            .id
        }

        /// A new htui owner document of `kind`, stamped from the store's clock (C34: the writer
        /// stamps a document, and a wall-clock stamp falls behind `TokioClock`).
        async fn write_goals(read: &MemStore, body: &str) -> DocumentId {
            let created_at = read
                .index_clock()
                .await
                .expect("the memory store never fails");
            read.write_owner_document(NewOwnerDocument {
                id: DocumentId::new(),
                project_id: ids::PROJECT_HTUI,
                kind: GOALS_KIND.into(),
                title: "Goals".into(),
                body: body.into(),
                created_by: ids::USER,
                created_at,
            })
            .await
            .expect("the memory store never fails")
            .id
        }

        fn has_item(store: &Counting, id: ItemId) -> bool {
            store
                .inner
                .points()
                .iter()
                .any(|p| p.subject.owner().item() == Some(id))
        }

        fn has_owner_document(store: &Counting, id: DocumentId) -> bool {
            store
                .inner
                .points()
                .iter()
                .any(|p| matches!(p.subject, Subject::OwnerDocument { id: of } if of == id))
        }

        /// `collection`'s cursor in `state`.
        fn cursor_of(state: &JobState, collection: IndexedCollection) -> Option<IndexCursor> {
            state.held.get(&collection).and_then(|held| held.cursor)
        }

        /// One job over `read` and a fresh index, its first beat (every collection reconciled)
        /// done.
        async fn reconciled(
            read: &MemStore,
        ) -> (concepts_worker::QdrantSource, Fakes, Uuid, JobState) {
            let settings = settings_over(read);
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                index.store(PLATFORM).full_syncs(),
                1,
                "the reconcile at the take"
            );
            assert!(
                cursor_of(&state, PLATFORM).is_some(),
                "a completed reconcile sets the cursor"
            );
            (settings, index, owner, state)
        }

        /// MOD-159 M2 D9: an item filed after the reconcile is indexed by the next incremental
        /// pass, with no second full sync.
        #[tokio::test(start_paused = true)]
        async fn an_item_filed_after_the_reconcile_is_indexed_within_one_cycle() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            let htui_reads = platform.syncs();
            tokio::time::advance(Duration::from_secs(1)).await;
            let item = mint_item(&read, "Filed after the reconcile").await;
            tokio::time::advance(INCREMENTAL_CYCLE - Duration::from_secs(1)).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(has_item(&platform, item), "indexed within one cycle");
            assert_eq!(platform.full_syncs(), 1, "no second full sync");
            // `syncs` counts every read of htui's points: the pass read them once.
            assert_eq!(platform.syncs(), htui_reads + 1);
            assert!(
                index
                    .store(GRAPHICS)
                    .inner
                    .points()
                    .iter()
                    .all(|p| p.subject.owner().item() != Some(item)),
                "only the item's collection"
            );
        }

        /// MOD-159 M2 D9: an amended requirement and a new owner document are each indexed by
        /// the next incremental pass.
        #[tokio::test(start_paused = true)]
        async fn an_amended_requirement_and_a_new_owner_document_are_indexed_within_one_cycle() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            tokio::time::advance(Duration::from_secs(1)).await;
            let row = read
                .requirement(ids::REQ_STO_1)
                .await
                .expect("the memory store never fails")
                .expect("a demo requirement");
            read.amend_requirement(
                row.id,
                row.version,
                RequirementPatch {
                    body: Some("Search finds requirements within a minute.".into()),
                    rationale: None,
                    priority: None,
                    author_id: ids::USER,
                    box_id: Some(ids::BOX),
                    reason: "amended".into(),
                },
                ids::HTUI_FEAT_3,
            )
            .await
            .expect("the memory store never fails");
            let goals = write_goals(&read, "## Aim\nindex within a minute").await;
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            let requirement = platform
                .inner
                .points()
                .into_iter()
                .find(|p| p.subject.owner().requirement() == Some(row.id))
                .expect("the requirement's point");
            assert!(
                requirement.text.contains("within a minute"),
                "{}",
                requirement.text
            );
            assert!(has_owner_document(&platform, goals), "the owner document");
            assert_eq!(platform.full_syncs(), 1, "no second full sync");
        }

        /// MOD-159 M2 D9: a pass that finds nothing written opens nothing, loads nothing and
        /// reads no key; its cursor still moves.
        #[tokio::test(start_paused = true)]
        async fn a_cycle_with_nothing_written_opens_nothing_and_reads_no_key() {
            static READS: AtomicUsize = AtomicUsize::new(0);
            fn ring() -> htui_core::store::Result<Option<String>> {
                READS.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            }
            let read = indexable().await;
            let settings =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(ring);
            settings.publish(Some(Arc::new(read.clone())));
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(READS.load(Ordering::SeqCst), 1, "the reconcile's read");
            let opened = index.opened();
            let reads = index.store(PLATFORM).reads();
            let first = cursor_of(&state, PLATFORM).expect("reconciled");
            for _ in 0..5 {
                tokio::time::advance(INCREMENTAL_CYCLE).await;
                index_beat(&read, &settings, &index, owner, &mut state).await;
            }
            assert_eq!(READS.load(Ordering::SeqCst), 1, "no key read");
            assert_eq!(index.opened(), opened, "nothing opened");
            assert_eq!(index.store(PLATFORM).reads(), reads, "no Qdrant call");
            let last = cursor_of(&state, PLATFORM).expect("still held");
            assert!(
                last.rows > first.rows,
                "the cursor moved: {first:?} -> {last:?}"
            );
        }

        /// MOD-159 M2 D5: a write committed late, stamped inside the overlap behind the cursor,
        /// is still read by the next pass.
        #[tokio::test(start_paused = true)]
        async fn a_write_stamped_inside_the_overlap_is_still_indexed() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let cursor = cursor_of(&state, PLATFORM).expect("reconciled");
            let late = read.handle_at(cursor.rows - TimeDelta::seconds(100));
            let item = mint_item(&late, "Committed late").await;
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(has_item(&index.store(PLATFORM), item));
        }

        /// MOD-159 M2 D5: a write stamped further behind than the overlap is the full
        /// reconcile's.
        #[tokio::test(start_paused = true)]
        async fn a_write_stamped_before_the_overlap_waits_for_the_reconcile() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let cursor = cursor_of(&state, PLATFORM).expect("reconciled");
            let late = read.handle_at(cursor.rows - TimeDelta::seconds(400));
            let item = mint_item(&late, "Committed very late").await;
            let platform = index.store(PLATFORM);
            for _ in 0..14 {
                tokio::time::advance(INCREMENTAL_CYCLE).await;
                index_beat(&read, &settings, &index, owner, &mut state).await;
                assert!(!has_item(&platform, item), "not the passes'");
            }
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(platform.full_syncs(), 2, "the reconcile at 15 minutes");
            assert!(has_item(&platform, item), "found by the reconcile");
        }

        /// MOD-159 M2 D7, D9: an incremental pass whose lease another box takes before its delete
        /// batch deletes nothing and stops holding the collection.
        #[tokio::test(start_paused = true)]
        async fn an_incremental_pass_whose_lease_was_taken_over_deletes_nothing_and_stops_holding()
        {
            let read = indexable().await;
            let first = write_goals(&read, "## Aim\nthe first goals").await;
            tokio::time::advance(Duration::from_secs(1)).await;
            let (settings, index, victim, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            assert!(has_owner_document(&platform, first));
            tokio::time::advance(Duration::from_secs(1)).await;
            // Supersedes the first: its points are the pass's delete batch.
            let second = write_goals(&read, "## Aim\nthe second goals").await;
            let thief = Uuid::now_v7();
            *platform.takeover.lock().expect("not poisoned") = Some(Takeover {
                leases: read.clone(),
                collection: PLATFORM,
                from: victim,
                to: thief,
            });
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, victim, &mut state).await;
            assert!(
                platform.takeover.lock().expect("not poisoned").is_none(),
                "the pass read Platform's points"
            );
            assert!(has_owner_document(&platform, first), "nothing was deleted");
            assert!(
                has_owner_document(&platform, second),
                "the upsert came first"
            );
            assert!(!state.held().contains(&PLATFORM), "{:?}", state.held());
            assert!(state.held().contains(&GRAPHICS), "{:?}", state.held());
            let take = read
                .take_indexer_lease(&PLATFORM, ids::BOX, victim, TimeDelta::seconds(120))
                .await
                .expect("the memory store never fails");
            assert!(
                matches!(take, LeaseTake::Held(_)),
                "the thief holds it: {take:?}"
            );
        }

        /// MOD-159 M2 D9: a pass that fails forgets its connection and keeps its cursor, so the
        /// next cycle reads the same window again.
        #[tokio::test(start_paused = true)]
        async fn a_failed_incremental_pass_keeps_its_window_and_retries_next_cycle() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            let cursor = cursor_of(&state, PLATFORM);
            tokio::time::advance(Duration::from_secs(1)).await;
            let item = mint_item(&read, "Filed before a failure").await;
            platform.fail.store(1, Ordering::SeqCst);
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(!has_item(&platform, item), "the upsert failed");
            assert_eq!(index.forgotten(), [true], "the failed pass's store");
            assert_eq!(cursor_of(&state, PLATFORM), cursor, "the window is kept");
            tokio::time::advance(INCREMENTAL_CYCLE).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(has_item(&platform, item), "retried");
            assert!(
                cursor_of(&state, PLATFORM).map(|c| c.rows) > cursor.map(|c| c.rows),
                "and moved on"
            );
            assert_eq!(platform.full_syncs(), 1, "no full sync for it");
        }

        /// MOD-159 M2 D1: the cursor is the process's; a job that takes a collection over
        /// reconciles it in full first.
        #[tokio::test(start_paused = true)]
        async fn a_taken_over_collection_starts_with_a_full_reconcile() {
            let read = indexable().await;
            let (first, second) = (Arc::new(Fakes::default()), Arc::new(Fakes::default()));
            let first_job = spawn_job(&read, &first, Uuid::now_v7());
            tokio::time::sleep(INCREMENTAL_CYCLE + Duration::from_secs(1)).await;
            assert_eq!(first.store(PLATFORM).full_syncs(), 1);
            first_job.stop(WORKER_RELEASE_WAIT).await;
            let second_job = spawn_job(&read, &second, Uuid::now_v7());
            tokio::time::sleep(Duration::from_secs(1)).await;
            let platform = second.store(PLATFORM);
            assert_eq!(
                platform.full_syncs(),
                1,
                "the new holder reconciled in full"
            );
            assert!(!platform.inner.points().is_empty());
            second_job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// MOD-159 M2 D9: a changed URL drops every cursor, so every held collection is
        /// reconciled in full against the new Qdrant at once, not at the next interval.
        #[tokio::test(start_paused = true)]
        async fn a_changed_url_reconciles_every_held_collection_at_once() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            set_row(&read, "http://elsewhere:6334").await;
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            let elsewhere: Vec<IndexedCollection> = {
                let opens = index.opens.lock().expect("not poisoned");
                let mut opened: Vec<_> = opens
                    .iter()
                    .filter(|(url, _)| url == "http://elsewhere:6334")
                    .map(|(_, collection)| *collection)
                    .collect();
                opened.sort_by_key(IndexedCollection::name);
                opened
            };
            assert_eq!(elsewhere, every_collection());
            assert_eq!(index.store(PLATFORM).full_syncs(), 2, "reconciled in full");
            assert_eq!(state.held(), every_collection());
        }

        /// MOD-159 M2 D9: the job wakes for a pass between lease beats, so with the default 40 s
        /// beat a collection's pass runs every 60 s, not every 80 s.
        #[tokio::test(start_paused = true)]
        async fn the_job_wakes_for_the_cycle_between_lease_beats() {
            let read = indexable().await;
            let settings = settings_over(&read);
            let index = Fakes::default();
            let owner = Uuid::now_v7();
            let mut state = JobState::default();
            let pause = index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(pause, BEAT, "the lease beat comes first");
            tokio::time::advance(pause).await;
            let pause = index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                pause,
                INCREMENTAL_CYCLE - BEAT,
                "woken for the pass at 60 s"
            );
            let item = mint_item(&read, "Filed at 40 s").await;
            tokio::time::advance(pause).await;
            let pause = index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(has_item(&index.store(PLATFORM), item), "indexed at 60 s");
            assert_eq!(pause, BEAT);
        }

        /// D12: `--index-items` syncs each workspace whose lease it takes, asks the box that
        /// indexes one it cannot take for a pass (MOD-159 M3 D4), says so and skips it, then
        /// creates the global collection and drops the legacy one.
        #[tokio::test(start_paused = true)]
        async fn index_items_skips_a_workspace_another_box_indexes_and_says_so() {
            let read = demo();
            let index = Fakes::default();
            let settings = QdrantSettings::new(
                htui_core::qdrant::QdrantUrl::parse("http://qdrant:6334").expect("a valid URL"),
                None,
            );
            let other = Uuid::now_v7();
            read.take_indexer_lease(&GRAPHICS, ids::BOX, other, TimeDelta::seconds(120))
                .await
                .expect("the memory store never fails");
            let mut named = Vec::new();
            for ws in read.workspaces().await.expect("workspaces") {
                let scope = workspace_search_scope(&read, &Scope::from_workspace(&ws)).await;
                named.push((ws.slug, scope));
            }
            let holder = Holder {
                box_id: ids::BOX,
                owner: Uuid::now_v7(),
                ttl: TimeDelta::seconds(120),
            };
            let mut notes = Vec::new();
            let report =
                index_under_leases(&read, &index, &settings, &named, holder, &mut |line| {
                    notes.push(line)
                })
                .await
                .expect("the run");
            assert!(report.items_rebuilt > 0, "{report:?}");
            assert_eq!(index.opened(), [PLATFORM, IndexedCollection::Global]);
            assert!(index.store(GRAPHICS).inner.points().is_empty(), "skipped");
            let graphics = named
                .iter()
                .find(|(_, scope)| scope.workspace_id == ids::WORKSPACE_GRAPHICS)
                .map(|(slug, _)| slug.clone())
                .expect("the demo's Graphics workspace");
            assert_eq!(
                notes,
                [
                    format!(
                        "workspace {graphics}: indexing runs on DESKTOP-HTUI; asked it to reindex"
                    ),
                    format!("dropped the old shared collection `{LEGACY_COLLECTION}`"),
                ]
            );
            assert_eq!(
                read.release_indexer_leases(holder.owner)
                    .await
                    .expect("the memory store never fails"),
                2,
                "the run held Platform and the global collection"
            );
            assert!(
                read.claim_indexer_pass(&GRAPHICS, other)
                    .await
                    .expect("the memory store never fails"),
                "the run asked Graphics' holder for a pass"
            );
        }

        /// D12: a holder whose box row is gone is named by the last eight hex digits of its id.
        #[test]
        fn a_held_note_names_the_box_by_hostname_or_short_id() {
            let lease = IndexerLease {
                collection: PLATFORM.name(),
                box_id: Some(BoxId::from_uuid(
                    Uuid::parse_str("0192f3a4-5b6c-7d8e-9f01-23456789abcd").expect("uuid"),
                )),
                box_name: None,
                expires_at: None,
                live: true,
            };
            assert_eq!(
                held_note("platform", &lease),
                "workspace platform: indexing runs on 6789abcd; not indexed here"
            );
            let named = IndexerLease {
                box_name: Some("hal\u{1b}".into()),
                ..lease
            };
            assert_eq!(
                held_note("platform", &named),
                "workspace platform: indexing runs on hal; not indexed here"
            );
        }

        /// MOD-159 M3 D6: `requested_note` names the holder as `held_note` does, and says the
        /// holder was asked to reindex.
        #[test]
        fn a_requested_note_names_the_box_by_hostname_or_short_id() {
            let lease = IndexerLease {
                collection: PLATFORM.name(),
                box_id: Some(BoxId::from_uuid(
                    Uuid::parse_str("0192f3a4-5b6c-7d8e-9f01-23456789abcd").expect("uuid"),
                )),
                box_name: None,
                expires_at: None,
                live: true,
            };
            assert_eq!(
                requested_note("platform", &lease),
                "workspace platform: indexing runs on 6789abcd; asked it to reindex"
            );
            let named = IndexerLease {
                box_name: Some("hal\u{1b}".into()),
                ..lease
            };
            assert_eq!(
                requested_note("platform", &named),
                "workspace platform: indexing runs on hal; asked it to reindex"
            );
        }

        /// MOD-159 M3 D5: a pass request on a collection the job holds is claimed at the next
        /// beat, which reconciles it in full though the interval has not passed; the request is
        /// gone after, and a beat with none reconciles nothing.
        #[tokio::test(start_paused = true)]
        async fn a_pass_request_reconciles_a_held_collection_at_the_next_beat() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            tokio::time::advance(BEAT).await;
            let holder = read
                .request_indexer_pass(&PLATFORM)
                .await
                .expect("the memory store never fails");
            assert!(holder.is_some(), "the job holds Platform live");
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(platform.full_syncs(), 2, "the requested reconcile");
            assert!(
                !read
                    .claim_indexer_pass(&PLATFORM, owner)
                    .await
                    .expect("the memory store never fails"),
                "the beat claimed the request"
            );
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(platform.full_syncs(), 2, "no request, no reconcile");
        }

        /// MOD-159 M3 D3: the claim comes before the pass, so a request made while a requested
        /// reconcile runs stays for the next beat, which reconciles again.
        #[tokio::test(start_paused = true)]
        async fn a_request_made_during_a_requested_reconcile_is_served_by_the_next_beat() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            tokio::time::advance(BEAT).await;
            read.request_indexer_pass(&PLATFORM)
                .await
                .expect("the memory store never fails")
                .expect("the job holds Platform live");
            *platform.pause.lock().expect("not poisoned") = Some(MINUTE);
            let again = tokio::spawn({
                let read = read.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    read.request_indexer_pass(&PLATFORM)
                        .await
                        .expect("the memory store never fails")
                }
            });
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(
                again.await.expect("the request ran").is_some(),
                "made while the reconcile ran"
            );
            assert_eq!(platform.full_syncs(), 2, "the first requested reconcile");
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(platform.full_syncs(), 3, "the request made during it");
        }

        /// MOD-159 M3 D3: a requested reconcile that fails has consumed its request: the next
        /// beat runs no full sync (the interval and the next request are the retries).
        #[tokio::test(start_paused = true)]
        async fn a_failed_requested_reconcile_consumes_its_request() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            let platform = index.store(PLATFORM);
            tokio::time::advance(BEAT).await;
            let item = mint_item(&read, "Filed before a failed reconcile").await;
            platform.fail.store(1, Ordering::SeqCst);
            // `syncs` counts reads of htui's points: a full sync reads them before it upserts
            // (and fails), and an incremental pass is not due within a beat.
            let htui_reads = platform.syncs();
            read.request_indexer_pass(&PLATFORM)
                .await
                .expect("the memory store never fails")
                .expect("the job holds Platform live");
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(
                platform.syncs(),
                htui_reads + 1,
                "the requested reconcile ran"
            );
            assert!(!has_item(&platform, item), "and failed");
            assert!(
                !read
                    .claim_indexer_pass(&PLATFORM, owner)
                    .await
                    .expect("the memory store never fails"),
                "the request was consumed"
            );
            tokio::time::advance(BEAT).await;
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert_eq!(platform.syncs(), htui_reads + 1, "not retried");
            assert_eq!(
                platform.full_syncs(),
                1,
                "no full sync reached agy since the first"
            );
        }

        /// MOD-159 M3 D5: the global collection has no scope to reconcile, so a beat never claims
        /// its pass request, even one the job holds: the request stays pending.
        #[tokio::test(start_paused = true)]
        async fn a_beat_never_claims_the_global_collections_request() {
            let read = indexable().await;
            let (settings, index, owner, mut state) = reconciled(&read).await;
            tokio::time::advance(BEAT).await;
            read.request_indexer_pass(&IndexedCollection::Global)
                .await
                .expect("the memory store never fails")
                .expect("the job holds the global collection live");
            index_beat(&read, &settings, &index, owner, &mut state).await;
            assert!(
                read.claim_indexer_pass(&IndexedCollection::Global, owner)
                    .await
                    .expect("the memory store never fails"),
                "the request is still pending"
            );
        }

        /// MOD-158 D7: the job starts whatever the setting says, and runs until it is stopped.
        /// MOD-158 R1 (B-L8): its first beat really ran: the published row, which says there is
        /// no URL, was read through the index's settings source.
        #[tokio::test]
        async fn the_job_starts_whatever_the_setting_says() {
            /// A row that is not there, counting its reads.
            #[derive(Default)]
            struct NoRow(AtomicUsize);
            impl concepts_worker::UrlRow for NoRow {
                fn qdrant_url(
                    &self,
                ) -> futures::future::BoxFuture<
                    '_,
                    Result<Option<htui_core::qdrant::StoredQdrantUrl>, StoreError>,
                > {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok(None) })
                }
            }

            let source =
                concepts_worker::QdrantSource::new(QdrantEnv::default(), KeySource::Interactive)
                    .with_keyring(|| Ok(None));
            let row = Arc::new(NoRow::default());
            source.publish(Some(Arc::clone(&row) as Arc<dyn concepts_worker::UrlRow>));
            let index = Arc::new(
                QdrantIndex::with_loader(|| Box::pin(std::future::pending()))
                    .with_source(Arc::new(source)),
            );
            let job = IndexJob::spawn_over(unreachable_store(), index, Uuid::now_v7(), None);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while row.0.load(Ordering::SeqCst) == 0 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the job's first beat never read the setting"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(!job.is_finished(), "the job runs until it is stopped");
            job.stop(WORKER_RELEASE_WAIT).await;
        }

        /// MOD-11 H-26: the index job and every search take the model from one load slot on the
        /// index `search_concepts` searches, so a worker that indexes and serves the tool loads the
        /// 133 MB model once, not twice. The second taker waits on the slot directly (MOD-160
        /// P-1): a search asks Qdrant before the model, and this one's Qdrant is a closed port.
        #[tokio::test]
        async fn the_index_job_and_the_search_share_one_model_load() {
            let loads = Arc::new(AtomicUsize::new(0));
            let settings = concepts_worker::QdrantSource::new(
                QdrantEnv::new(Some("http://127.0.0.1:1".to_owned()), None),
                KeySource::Interactive,
            )
            .with_keyring(|| Ok(None));
            let index = Arc::new(
                QdrantIndex::with_loader({
                    let loads = Arc::clone(&loads);
                    move || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        // A load still in flight: everyone who wants the model waits on this one.
                        Box::pin(std::future::pending())
                    }
                })
                .with_source(Arc::new(settings)),
            );

            let read = MemStore::demo();
            let job = tokio::spawn({
                let index = Arc::clone(&index);
                async move {
                    let settings = Arc::clone(index.source());
                    match index_job(&read, &settings, &*index, Uuid::now_v7()).await {}
                }
            });
            tokio::time::timeout(Duration::from_secs(10), async {
                while loads.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the job loads its model through the index");

            let model = index.embedder();
            assert!(
                tokio::time::timeout(Duration::from_millis(500), model)
                    .await
                    .is_err(),
                "a search's model waits on the job's load"
            );
            assert_eq!(loads.load(Ordering::SeqCst), 1, "one model for both");
            job.abort();
        }
    }
}
