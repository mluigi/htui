//! `--index-items` and `--search-items`: the concepts index from the command line (MOD-34 D3,
//! `R-STO-8`).
//!
//! Both run before any terminal work, like `--set-dsn`, print to the shell and exit. Neither
//! touches the TUI's start path: a missing Qdrant URL, a Qdrant that does not answer or a
//! Postgres that does not answer is one line on stderr and a non-zero exit, and nothing else is
//! affected (`docs/ANA-19.md` §2 invariant 3). The automatic sync belongs to the headless worker
//! (MOD-41) and the agent-facing tool to the MCP server (MOD-11).
//!
//! Both connect headless (`PgStore::connect_headless`, MOD-40 plan D8): a pending schema or a
//! build below the database's target version is refused, never migrated.
//!
//! The worker's background sync (MOD-41 plan D19) lives here too: [`spawn_index_job`] reads the
//! keyring once at `htui worker` start and, when it holds a Qdrant URL, spawns [`index_loop`],
//! which re-syncs every project at start and then every [`sync_interval`].
use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context as _, bail};
use htui_core::model::{ProjectId, RequirementState, Resolution, Scope};
use htui_core::store::{MemStore, ReadStore, StoreError};
use htui_store::embed::FastEmbedder;
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, PointType, QdrantStore, SearchQuery, VectorStore};
use htui_store::vector_sync::{Indexer, SyncReport};
use htui_store::{HeadlessError, PgStore, identity, secret};
use serde_json::Value;

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

/// Builds the Qdrant settings from what the keyring holds.
///
/// # Errors
///
/// When no URL is stored, naming where to set one.
pub fn settings_from(
    url: Option<String>,
    api_key: Option<String>,
) -> anyhow::Result<QdrantSettings> {
    let Some(url) = url else {
        bail!("no Qdrant URL is stored; set one in Settings > Qdrant");
    };
    Ok(QdrantSettings::new(url, api_key)?)
}

async fn open() -> anyhow::Result<(PgStore, QdrantStore<FastEmbedder>)> {
    let settings = settings_from(secret::get_qdrant_url()?, secret::get_qdrant_api_key()?)?;
    let Some(dsn) = secret::get_dsn()? else {
        bail!("no Postgres DSN is stored; run `htui --set-dsn` first");
    };
    let identity = identity::load_or_mint(&identity::config_root()?)?;
    let pg = PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT, PoolSize::TUI)
        .await
        .map_err(headless_refusal)?;
    let embedder = FastEmbedder::new()?;
    let store = QdrantStore::connect(&settings, embedder)
        .await
        .context("cannot reach Qdrant")?;
    Ok((pg, store))
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

/// Every workspace's projects as scopes, narrowed to one project slug when given.
async fn scopes(pg: &PgStore, project: Option<&str>) -> anyhow::Result<Vec<Scope>> {
    let mut scopes = Vec::new();
    let mut found = project.is_none();
    for ws in pg.workspaces().await? {
        let mut scope = Scope::from_workspace(&ws);
        if let Some(slug) = project {
            let wanted: Vec<ProjectId> = ws
                .projects
                .iter()
                .filter(|p| p.slug == slug)
                .map(|p| p.project_id)
                .collect();
            scope.project_ids.retain(|id| wanted.contains(id));
        }
        found |= !scope.project_ids.is_empty();
        if !scope.project_ids.is_empty() {
            scopes.push(scope);
        }
    }
    if !found {
        bail!("no project with slug `{}`", project.unwrap_or_default());
    }
    Ok(scopes)
}

/// `--index-items`: brings the index in step with every project (or one), then reports.
///
/// # Errors
///
/// Missing settings, an unreachable Postgres or Qdrant, a model that cannot load.
pub async fn index_items(project: Option<&str>) -> anyhow::Result<()> {
    let (pg, store) = open().await?;
    let report = sync_all(&pg, &scopes(&pg, project).await?, &store).await?;
    eprintln!("{}", report_line(&report));
    Ok(())
}

/// The `app_setting` key of the worker's index interval (MOD-41 plan D19): a positive JSON
/// integer of minutes.
pub const SYNC_MINUTES_KEY: &str = "concepts_sync_minutes";

/// The interval, in minutes, when [`SYNC_MINUTES_KEY`] is absent, non-positive or not an integer.
pub const DEFAULT_SYNC_MINUTES: u64 = 15;

/// `index_items`' loop over given scopes (MOD-41 plan D19, fact-check F-C4): every scope in
/// order, the reports summed. Taking the scopes rather than a `PgStore` lets the worker job's
/// tests run over `MemStore`.
///
/// # Errors
///
/// The first scope's store or index error; later scopes are not synced.
pub async fn sync_all(
    read: &impl ReadStore,
    scopes: &[Scope],
    vectors: &impl VectorStore,
) -> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    for scope in scopes {
        report += Indexer::sync(read, scope, vectors).await?;
    }
    Ok(report)
}

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

/// What the worker's index job reads besides [`ReadStore`]: both are inherent on the stores, not
/// on a trait (MOD-41 plan D19).
#[allow(async_fn_in_trait, reason = "no dyn IndexSource is formed")]
pub trait IndexSource: ReadStore {
    /// Every workspace's projects as scopes, a workspace without projects left out.
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>>;
    /// The `app_setting` map, [`sync_interval`]'s input.
    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError>;
}

impl IndexSource for PgStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        scopes(self, None).await
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }
}

impl IndexSource for MemStore {
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>> {
        Ok(self
            .workspaces()
            .await?
            .iter()
            .map(Scope::from_workspace)
            .filter(|scope| !scope.project_ids.is_empty())
            .collect())
    }

    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError> {
        self.app_settings().await
    }
}

/// One cycle of the worker's index job: the scopes, [`sync_all`] over them, then the settings;
/// answers how long to sleep before the next. A failure is a `warn` and never an `Err`: the next
/// cycle is the retry (MOD-41 plan D19).
pub async fn index_cycle(source: &impl IndexSource, vectors: &impl VectorStore) -> Duration {
    match source.index_scopes().await {
        Ok(scopes) => match sync_all(source, &scopes, vectors).await {
            Ok(report) => tracing::info!(?report, "concepts index synced"),
            Err(err) => tracing::warn!(%err, "concepts index sync failed; next cycle"),
        },
        Err(err) => {
            tracing::warn!(err = %format!("{err:#}"), "concepts index scopes failed; next cycle")
        }
    }
    let app = match source.index_settings().await {
        Ok(app) => app,
        Err(err) => {
            tracing::warn!(%err, "concepts index interval unread; the default applies");
            BTreeMap::new()
        }
    };
    sync_interval(&app)
}

/// The worker's index job: [`index_cycle`] at start, then again after each interval, forever. It
/// never touches the run runtime, and the worker aborts it on shutdown (MOD-41 plan D19).
pub async fn index_loop(source: &impl IndexSource, vectors: &impl VectorStore) -> ! {
    loop {
        let pause = index_cycle(source, vectors).await;
        tokio::time::sleep(pause).await;
    }
}

/// The worker's index job over `pg`, when the keyring holds a Qdrant URL: read once, here, at
/// `htui worker` start (MOD-41 PRD D6, plan D19). `None`, with an `info` line saying why, when it
/// holds none or cannot be read, which on a keyring-less host is always.
pub async fn spawn_index_job(pg: PgStore) -> Option<tokio::task::JoinHandle<()>> {
    let settings = match apart("htui-index-keyring", keyring_settings).await {
        Some(Ok(settings)) => settings,
        Some(Err(why)) => {
            tracing::info!("no concepts index job: {why}");
            return None;
        }
        None => {
            tracing::info!("no concepts index job: the keyring read stopped");
            return None;
        }
    };
    Some(tokio::spawn(async move {
        let store = open_index(&pg, &settings).await;
        index_loop(&pg, &store).await;
    }))
}

/// The Qdrant settings the keyring holds, or why the worker has no index job (plan D19: `None`
/// and `Err` alike mean no job).
fn keyring_settings() -> Result<QdrantSettings, String> {
    let url = match secret::get_qdrant_url() {
        Ok(Some(url)) => url,
        Ok(None) => return Err("no Qdrant URL is stored in the keyring".to_owned()),
        Err(err) => return Err(format!("the keyring could not be read: {err}")),
    };
    let api_key = secret::get_qdrant_api_key()
        .map_err(|err| format!("the Qdrant API key could not be read: {err}"))?;
    QdrantSettings::new(url, api_key)
        .map_err(|err| format!("the stored Qdrant URL is not usable: {err}"))
}

/// The model on a thread of its own, then the collection; a failure is a `warn` and another
/// attempt after the interval, forever (plan D19). Rebuilds the model each attempt:
/// `QdrantStore::connect` takes it by value and `FastEmbedder` is not `Clone`.
async fn open_index(pg: &PgStore, settings: &QdrantSettings) -> QdrantStore<FastEmbedder> {
    loop {
        let opened = match apart("htui-index-model", FastEmbedder::new).await {
            Some(Ok(embedder)) => QdrantStore::connect(settings, embedder)
                .await
                .map_err(|err| format!("cannot reach Qdrant: {err}")),
            Some(Err(err)) => Err(err.to_string()),
            None => Err("the model loader stopped".to_owned()),
        };
        match opened {
            Ok(store) => return store,
            Err(err) => tracing::warn!(%err, "concepts index not opened; next cycle"),
        }
        let app = pg.index_settings().await.unwrap_or_default();
        tokio::time::sleep(sync_interval(&app)).await;
    }
}

/// `work` on a named thread of its own, awaited. Not `spawn_blocking`: the runtime waits for its
/// blocking tasks when it is dropped, and a keyring prompt nobody answers or a model download
/// must not hold `htui worker`'s exit (`worker_cmd::read_dsn_apart`'s reason). The thread is
/// left behind on an abort and dies with the process. `None` when the thread cannot be spawned
/// or ends without answering.
async fn apart<T: Send + 'static>(
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

/// `--search-items`: prints the best hits, one per line.
///
/// # Errors
///
/// As [`index_items`].
pub async fn search_items(options: &SearchOptions) -> anyhow::Result<()> {
    let (pg, store) = open().await?;
    let projects: Vec<ProjectId> = scopes(&pg, options.project.as_deref())
        .await?
        .into_iter()
        .flat_map(|s| s.project_ids)
        .collect();
    let query = query(&options.query, projects, options.decisions, options.limit);
    let hits = store.search(&query).await?;
    if hits.is_empty() {
        eprintln!("no matches (run `htui --index-items` if the index is empty)");
    }
    for hit in &hits {
        println!("{}", format_hit(hit));
    }
    Ok(())
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

    #[test]
    fn a_missing_url_names_where_to_set_it() {
        let err = settings_from(None, None).unwrap_err().to_string();
        assert!(err.contains("Settings > Qdrant"), "{err}");
    }

    #[test]
    fn a_stored_url_and_key_become_settings() {
        let s = settings_from(Some("http://localhost:6334".into()), Some("k".into())).unwrap();
        assert_eq!(s.url, "http://localhost:6334");
        assert!(s.api_key.is_some());
        assert!(settings_from(Some("localhost:6334".into()), None).is_err());
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

    #[test]
    fn report_line_is_index_items_wording() {
        let report = SyncReport {
            items_rebuilt: 1,
            items_unchanged: 2,
            requirements_rebuilt: 3,
            requirements_unchanged: 4,
            points_upserted: 5,
            points_deleted: 6,
        };
        assert_eq!(
            report_line(&report),
            "indexed: 1 item(s) rebuilt, 2 unchanged; 3 requirement(s) rebuilt, 4 unchanged; \
             5 point(s) written, 6 removed"
        );
    }

    /// The search runtime shares one loaded model between its tasks (MOD-64 D238); `htui-store`'s
    /// own tests build without `local-embed`, so the check lives here (D258).
    #[test]
    fn fast_embedder_is_clone() {
        fn clone_of<T: Clone>() {}
        clone_of::<FastEmbedder>();
    }

    /// The worker's index job (MOD-41 plan D19, blueprint §16) over `MemStore` and
    /// `MemVectorStore`, under paused time.
    mod index_job {
        use super::super::*;
        use htui_core::fixtures::ids;
        use htui_store::vector::{ConceptPoint, IndexedPoint, MemVectorStore};
        use serde_json::json;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use uuid::Uuid;

        const MINUTE: Duration = Duration::from_secs(60);

        /// `MemVectorStore`, counting the syncs that reach it (one `indexed` read of the htui
        /// project per whole sync) and every `indexed` read (a failed sync stops part-way, perhaps
        /// before the htui project), and failing the first `fail` upserts.
        #[derive(Debug, Default)]
        struct Counting {
            inner: MemVectorStore,
            syncs: AtomicUsize,
            reads: AtomicUsize,
            fail: AtomicUsize,
        }

        impl Counting {
            fn failing_once() -> Self {
                Self {
                    fail: AtomicUsize::new(1),
                    ..Self::default()
                }
            }

            fn syncs(&self) -> usize {
                self.syncs.load(Ordering::SeqCst)
            }

            fn reads(&self) -> usize {
                self.reads.load(Ordering::SeqCst)
            }
        }

        impl VectorStore for Counting {
            async fn upsert(&self, points: Vec<ConceptPoint>) -> Result<(), StoreError> {
                let failing = self
                    .fail
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
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
                if project == ids::PROJECT_HTUI {
                    self.syncs.fetch_add(1, Ordering::SeqCst);
                }
                self.inner.indexed(project).await
            }

            async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError> {
                self.inner.search(query).await
            }
        }

        /// Runs `checks` beside `index_loop(read, vectors)`; the loop never ends on its own, so
        /// `checks` ending first is also the proof it was still alive.
        async fn beside_the_loop(
            read: &MemStore,
            vectors: &Counting,
            checks: impl Future<Output = ()>,
        ) {
            tokio::select! {
                biased;
                () = checks => {}
                never = index_loop(read, vectors) => match never {},
            }
        }

        #[tokio::test(start_paused = true)]
        async fn the_index_job_syncs_at_start_and_every_interval() {
            let read = MemStore::demo();
            let vectors = Counting::default();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert_eq!(vectors.syncs(), 1, "one sync at start");
                assert!(
                    !vectors.inner.points().is_empty(),
                    "the start sync indexed the demo items"
                );
                tokio::time::sleep(15 * MINUTE - Duration::from_secs(2)).await;
                assert_eq!(vectors.syncs(), 1, "nothing before the default 15 minutes");
                tokio::time::sleep(Duration::from_secs(2)).await;
                assert_eq!(vectors.syncs(), 2, "the second sync after 15 minutes");
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(vectors.syncs(), 3, "the third after 30");
            })
            .await;
        }

        #[tokio::test(start_paused = true)]
        async fn a_failing_sync_is_logged_and_retried_next_cycle() {
            let read = MemStore::demo();
            let vectors = Counting::failing_once();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let failed = vectors.reads();
                assert!(failed > 0, "the start sync ran");
                assert!(
                    vectors.inner.points().is_empty(),
                    "and failed at its first upsert"
                );
                tokio::time::sleep(15 * MINUTE - Duration::from_secs(2)).await;
                assert_eq!(vectors.reads(), failed, "no retry before the next cycle");
                tokio::time::sleep(Duration::from_secs(2)).await;
                assert!(vectors.reads() > failed, "the loop outlived the failure");
                assert!(
                    vectors.syncs() >= 1,
                    "the next cycle synced the htui project"
                );
                assert!(
                    !vectors.inner.points().is_empty(),
                    "the next cycle indexed the demo items"
                );
            })
            .await;
        }

        #[tokio::test(start_paused = true)]
        async fn the_interval_reads_concepts_sync_minutes_with_a_15_minute_default() {
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

            let read = MemStore::demo();
            let vectors = Counting::default();
            beside_the_loop(&read, &vectors, async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                assert_eq!(vectors.syncs(), 1);
                read.set_app_setting(SYNC_MINUTES_KEY, json!(1));
                tokio::time::sleep(15 * MINUTE).await;
                assert_eq!(
                    vectors.syncs(),
                    2,
                    "the sleep in progress keeps its 15 minutes"
                );
                tokio::time::sleep(MINUTE).await;
                assert_eq!(vectors.syncs(), 3, "the next sleep is the new minute");
            })
            .await;
        }

        /// A pool that has never connected: `spawn_index_job` must not query it when there is
        /// no job to start.
        fn unreachable_store() -> PgStore {
            let identity = identity::Identity {
                box_id: htui_core::model::BoxId::new(),
                hostname: "HTUI-TEST".to_owned(),
            };
            PgStore::lazy(
                "postgres://nobody:nothing@127.0.0.1:1/none",
                &identity,
                Duration::from_millis(250),
            )
            .expect("a lazy pool opens no socket")
        }

        #[tokio::test]
        async fn no_qdrant_url_means_no_job() {
            let _keyring = htui_store::testkit::mock_keyring().await;
            assert!(
                spawn_index_job(unreachable_store()).await.is_none(),
                "an empty keyring starts no job"
            );
            secret::set_qdrant_api_key("a-key").expect("the fake keyring stores");
            assert!(
                spawn_index_job(unreachable_store()).await.is_none(),
                "an API key without a URL starts no job"
            );
        }

        #[tokio::test]
        async fn an_unreadable_keyring_means_no_job() {
            let _keyring = htui_store::testkit::mock_keyring_broken().await;
            assert!(spawn_index_job(unreachable_store()).await.is_none());
        }

        #[tokio::test]
        async fn a_stored_qdrant_url_starts_the_job() {
            let _keyring = htui_store::testkit::mock_keyring().await;
            // Nothing listens on port 1: the job starts, then warns every cycle, which is its
            // contract (plan D19); the case aborts it at once, before the model loads.
            secret::set_qdrant_url("http://127.0.0.1:1").expect("the fake keyring stores");
            let job = spawn_index_job(unreachable_store())
                .await
                .expect("a stored URL starts the job");
            assert!(!job.is_finished(), "the job runs until it is aborted");
            job.abort();
            assert!(job.await.expect_err("aborted").is_cancelled());
        }
    }
}
