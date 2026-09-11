//! The store the application runs against (ANA-9 §6.1, plan D1, D12, blueprint C.16).
//!
//! [`Backend`] is an enum, not a `Box<dyn ReadStore>`: native `async fn` in traits is not object
//! safe, and a concrete type keeps the spawned worker's futures `Send`-inferable (MOD-1 plan D2).
//!
//! There is no `Backend::write*` method and no `impl WriteStore for Backend`. The two ways to
//! reach a `WriteStore` are [`Backend::writable`], which borrows a [`PgStore`] for the length of
//! one call and answers `None` unless the server is reachable, and [`Backend::writer`] (MOD-2
//! milestone 3), which hands out an owned [`Writer`] a spawned task can hold for a whole session.
//!
//! Since MOD-2 milestone 4 those two differ on [`Backend::Offline`] (plan D34): `writer()` answers
//! `Some(Writer::Buffered(..))` there, so there **is** an offline write path — and it reaches a
//! file under `<cache_dir>/pending/`, never the server. The invariant that survives is narrower
//! and still true: nothing writes to Postgres unless the backend is [`Backend::Online`], which is
//! what `writable()` still answers for. What has not changed at all is
//! [`Backend::is_writable`]: it is still `false` offline, because the store worker's re-dial
//! ticker keys on it and a `true` there would stop the process ever dialling Postgres again.
//! Both claims are documented here and deliberately not covered by a compile-fail test: they are
//! not worth a `trybuild` dependency.

use chrono::{DateTime, Utc};
use htui_core::model::{
    AgentSummary, BoxInfo, Document, DocumentHead, DocumentId, Item, ItemFilter, ItemId,
    ItemSummary, LinkGraph, Note, Project, ProjectId, ProjectRef, PromptScope, RunSummary, Scope,
    SessionEvent, StepId, UpstreamEntry, UserId, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore, Result, StoreError};
use serde_json::Value;

use crate::cache::CacheStore;
use crate::pg::PgStore;
use crate::writer::{BufferedWriter, Writer};

/// Seconds in a minute and minutes in an hour: the two thresholds of [`Backend::label`].
const MINUTE: i64 = 60;
/// Seconds in an hour.
const HOUR: i64 = 60 * MINUTE;

/// The store the application runs against (ANA-9 §6.1).
#[derive(Debug, Clone)]
pub enum Backend {
    /// Everything in process memory (`--demo`, tests).
    Memory(MemStore),
    /// Postgres reachable: reads go to Postgres, writes are available, the refresher is running.
    Online {
        /// The writable store.
        pg: PgStore,
        /// The mirror, kept warm by the refresher.
        cache: CacheStore,
    },
    /// Postgres unreachable: reads come from the mirror, and the only write path is the offline
    /// chat buffer under `<cache_dir>/pending/` (MOD-2 milestone 4, [`Backend::writer`]).
    Offline {
        /// The mirror.
        cache: CacheStore,
        /// When the process gave up. `None` means "the first attempt has not answered yet",
        /// which is what [`Backend::label`] renders as `connecting` (blueprint H.15).
        since: Option<DateTime<Utc>>,
    },
}

impl Backend {
    /// Wraps an in-memory store.
    #[must_use]
    pub const fn memory(store: MemStore) -> Self {
        Self::Memory(store)
    }

    /// Store-state text for the top bar (plan D11).
    ///
    /// - [`Backend::Memory`] → `memory`
    /// - [`Backend::Online`] → `online`
    /// - [`Backend::Offline`] with `since: None` → `connecting`
    /// - [`Backend::Offline`] with `since: Some(t)` → `offline · <age>`, where `<age>` is
    ///   `Utc::now() - t` truncated to `<n>s` under a minute, `<n>m` under an hour and `<n>h`
    ///   otherwise — so three minutes and forty seconds reads `offline · 3m`.
    ///
    /// A `since` in the future (a clock that moved backwards) reads `offline · 0s` rather than a
    /// negative age.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Memory(_) => "memory".to_owned(),
            Self::Online { .. } => "online".to_owned(),
            Self::Offline { since: None, .. } => "connecting".to_owned(),
            Self::Offline {
                since: Some(since), ..
            } => format!(
                "offline · {}",
                age(Utc::now().signed_duration_since(*since).num_seconds())
            ),
        }
    }

    /// Whether **the server** is reachable. [`Backend::Offline`] is the first backend to answer
    /// `false`.
    ///
    /// It stays `false` offline whatever else the mirror learns to answer: this is what the store
    /// worker's re-dial ticker keys on, so a `true` here would stop the process ever dialling
    /// Postgres again. "Can something be written at all" is [`Backend::writer`]'s question, and it
    /// is a different one.
    #[must_use]
    pub const fn is_writable(&self) -> bool {
        match self {
            Self::Memory(_) | Self::Online { .. } => true,
            Self::Offline { .. } => false,
        }
    }

    /// The writable store, or `None`. The **only** way a caller reaches `WriteStore` (plan D1).
    ///
    /// [`Backend::Memory`] answers `None` too: `MemStore` is a `WriteStore`, but it is not a
    /// [`PgStore`], and the write paths MOD-13 adds take the concrete type.
    #[must_use]
    pub const fn writable(&self) -> Option<&PgStore> {
        match self {
            Self::Online { pg, .. } => Some(pg),
            Self::Memory(_) | Self::Offline { .. } => None,
        }
    }

    /// An **owned** writable handle, or `None` (MOD-2 D26, D34).
    ///
    /// The counterpart of [`Backend::writable`] for a caller that outlives one call — the chat
    /// recorder, which is generic over `S: WriteStore` and lives inside a spawned session task.
    /// [`Backend::Memory`] answers `Some` here and `None` there, because a `MemStore` **is** a
    /// `WriteStore` even though it is not a [`PgStore`]. [`Backend::Offline`] answers `Some` here
    /// since milestone 4: a chat that cannot reach Postgres records into
    /// `<cache_dir>/pending/` through [`Writer::Buffered`] and the refresher uploads it on the
    /// next connection, which is the difference between a chat that is recorded and no chat at
    /// all. It still answers `None` to `writable()`, because that one is the server.
    ///
    /// Every call builds a **fresh** [`crate::BufferedWriter`]: its registration map belongs to
    /// one chat, and the runtime takes exactly one writer per chat.
    ///
    /// The return type stays `Option` even though no variant answers `None` today: `writable()`
    /// is the paired API and stays `Option`, and a fourth variant that cannot write should be a
    /// one-arm change here rather than a signature change at every call site.
    #[must_use]
    pub fn writer(&self) -> Option<Writer> {
        match self {
            Self::Memory(store) => Some(Writer::Memory(store.clone())),
            Self::Online { pg, .. } => Some(Writer::Online(pg.clone())),
            Self::Offline { cache, .. } => {
                Some(Writer::Buffered(BufferedWriter::new(cache.clone())))
            }
        }
    }

    /// Who this process is, for `run.started_by` (MOD-2 D29).
    ///
    /// The render side never learns a `UserId`: the chat tab asks for a chat and the worker fills
    /// in who started it, which is what keeps `R-NF-3` a fact about ownership rather than a habit.
    ///
    /// [`Backend::Offline`] resolves it from the mirror since MOD-2 milestone 4 (plan D33):
    /// [`CacheStore::this_user`] looks up the OS-derived name the online seed used, so an offline
    /// chat names the author the server already has instead of refusing outright.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] on a memory store with no `app_user` row — an empty store has no
    /// author to attribute a run to — and the same on a mirror that has never synced a row under
    /// this OS user's name.
    pub async fn this_user(&self) -> Result<UserId> {
        match self {
            Self::Memory(store) => store.this_user().ok_or_else(|| StoreError::NotFound {
                entity: "app_user",
                id: "(none loaded)".to_owned(),
            }),
            Self::Online { pg, .. } => Ok(pg.this_user()),
            Self::Offline { cache, .. } => cache.this_user().await,
        }
    }

    /// Records that a connection attempt answered and failed: `connecting` becomes `offline · 0s`.
    ///
    /// Idempotent — a second failure does not restart the clock, so the age keeps counting from
    /// the moment the process first gave up — and a no-op on every other variant. A method rather
    /// than a `match` inside the store worker (blueprint D.2) so the `since: None` invariant of
    /// [`Backend::label`] lives in one file.
    pub fn gave_up(&mut self) {
        if let Self::Offline {
            since: since @ None,
            ..
        } = self
        {
            *since = Some(Utc::now());
        }
    }

    /// Records that a server that *was* answering stopped: `online` becomes `offline · 0s`.
    ///
    /// The mid-session counterpart of [`Backend::gave_up`], which only ever turns the first
    /// `connecting` into an age. The mirror is moved across rather than re-opened - it is the file
    /// the refresher has been filling since the `Online` swap - and `since` starts now, because
    /// this is the moment the process lost the server.
    ///
    /// Answers whether the swap happened: `false` on [`Backend::Memory`] (no mirror to fall back
    /// to) and on [`Backend::Offline`] (already there, and its `since` must not be restarted), so
    /// the store worker can use it as the "did I just go offline" test. The caller is what stops
    /// the refresher: this type does not own it.
    pub fn went_offline(&mut self) -> bool {
        let Self::Online { cache, .. } = self else {
            return false;
        };
        *self = Self::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        true
    }

    /// The mirror, when there is one; `None` for [`Backend::Memory`].
    ///
    /// This is how the store worker moves the cache from an offline backend into an online one
    /// without re-opening the file (blueprint D.2).
    #[must_use]
    pub const fn cache(&self) -> Option<&CacheStore> {
        match self {
            Self::Online { cache, .. } | Self::Offline { cache, .. } => Some(cache),
            Self::Memory(_) => None,
        }
    }

    /// Every workspace, ordered by name.
    ///
    /// Hierarchy reads are inherent methods rather than [`ReadStore`] methods: ANA-9 §6.1 is
    /// quoted verbatim and has no `workspaces()` (MOD-1 blueprint B.7).
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports.
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        match self {
            Self::Memory(store) => store.workspaces().await,
            Self::Online { pg, .. } => pg.workspaces().await,
            Self::Offline { cache, .. } => cache.workspaces().await,
        }
    }

    /// This box's row, projected for the top bar.
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports.
    pub async fn box_info(&self) -> Result<Option<BoxInfo>> {
        match self {
            Self::Memory(store) => store.box_info().await,
            Self::Online { pg, .. } => pg.box_info().await,
            Self::Offline { cache, .. } => cache.box_info().await,
        }
    }

    /// How many runs of the scope are active (`RunStatus::is_active`).
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports.
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize> {
        match self {
            Self::Memory(store) => store.active_runs(scope).await,
            Self::Online { pg, .. } => pg.active_runs(scope).await,
            Self::Offline { cache, .. } => cache.active_runs(scope).await,
        }
    }

    /// The scope's projects, ordered by `workspace_project.position`.
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports.
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>> {
        match self {
            Self::Memory(store) => store.projects(scope).await,
            Self::Online { pg, .. } => pg.projects(scope).await,
            Self::Offline { cache, .. } => cache.projects(scope).await,
        }
    }

    /// The agent registry, ordered by `agent.name`, each row carrying this box's `agent_box`
    /// (MOD-2 plan D3, D14).
    ///
    /// This read used to refuse on [`Backend::Offline`], because neither table was mirrored. Since
    /// MOD-2 milestone 4 `agent` **is** (plan D31), so the arm answers from the mirror with every
    /// `on_box` set to `None`: an offline chat has to resolve a driver, and a refusal there is the
    /// difference between a chat that records to disk and no chat at all. `agent_box` stays
    /// server-side, so offline the Settings tab lists the rows as "not probed"
    /// ([`AgentSummary::on_box`]) where it used to show a failed read; its failure arm stays for a
    /// genuine server error, which this no longer is.
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports; offline, [`StoreError::Backend`] from the mirror decoder.
    pub async fn agents(&self) -> Result<Vec<AgentSummary>> {
        match self {
            Self::Memory(store) => store.agents().await,
            Self::Online { pg, .. } => pg.agents().await,
            Self::Offline { cache, .. } => cache.agents().await,
        }
    }

    /// `project.settings` of one project, or `None` when the arm's store holds no such row
    /// (MOD-2 plan D70).
    ///
    /// Every arm answers, including the offline one: `project.settings` is mirrored
    /// (`cache_migrations/0001_mirror.sql:57-61`), so the per-run token cap in that document is
    /// enforced by a chat that happened offline exactly as by one that did not. That is the whole
    /// reason the cap lives in this column and not in a new table or an env knob.
    ///
    /// # Errors
    ///
    /// Whatever the arm's store reports; offline, [`StoreError::Backend`] from the mirror decoder.
    pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
        match self {
            Self::Memory(store) => store.project_settings(project).await,
            Self::Online { pg, .. } => pg.project_settings(project).await,
            Self::Offline { cache, .. } => cache.project_settings(project).await,
        }
    }
}

/// The dispatch rule of plan D12: [`Backend::Online`] reads go to **Postgres**, never to the
/// mirror — live reads come from the source of truth while it is reachable (ANA-9 §4.4's last
/// paragraph) — and [`Backend::Offline`] reads come from the mirror.
impl ReadStore for Backend {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        match self {
            Self::Memory(store) => store.items(scope, filter).await,
            Self::Online { pg, .. } => pg.items(scope, filter).await,
            Self::Offline { cache, .. } => cache.items(scope, filter).await,
        }
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        match self {
            Self::Memory(store) => store.item(id).await,
            Self::Online { pg, .. } => pg.item(id).await,
            Self::Offline { cache, .. } => cache.item(id).await,
        }
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        match self {
            Self::Memory(store) => store.links(id, hops).await,
            Self::Online { pg, .. } => pg.links(id, hops).await,
            Self::Offline { cache, .. } => cache.links(id, hops).await,
        }
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        match self {
            Self::Memory(store) => store.documents(id).await,
            Self::Online { pg, .. } => pg.documents(id).await,
            Self::Offline { cache, .. } => cache.documents(id).await,
        }
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        match self {
            Self::Memory(store) => store.notes(id).await,
            Self::Online { pg, .. } => pg.notes(id).await,
            Self::Offline { cache, .. } => cache.notes(id).await,
        }
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        match self {
            Self::Memory(store) => store.runs(id).await,
            Self::Online { pg, .. } => pg.runs(id).await,
            Self::Offline { cache, .. } => cache.runs(id).await,
        }
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        match self {
            Self::Memory(store) => store.step_events(step).await,
            Self::Online { pg, .. } => pg.step_events(step).await,
            Self::Offline { cache, .. } => cache.step_events(step).await,
        }
    }

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        match self {
            Self::Memory(store) => store.document(id).await,
            Self::Online { pg, .. } => pg.document(id).await,
            Self::Offline { cache, .. } => cache.document(id).await,
        }
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        match self {
            Self::Memory(store) => store.documents_of_kinds(item, kinds).await,
            Self::Online { pg, .. } => pg.documents_of_kinds(item, kinds).await,
            Self::Offline { cache, .. } => cache.documents_of_kinds(item, kinds).await,
        }
    }

    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        match self {
            Self::Memory(store) => store.upstream_summaries(id, hops, scope).await,
            Self::Online { pg, .. } => pg.upstream_summaries(id, hops, scope).await,
            Self::Offline { cache, .. } => cache.upstream_summaries(id, hops, scope).await,
        }
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        match self {
            Self::Memory(store) => store.project(id).await,
            Self::Online { pg, .. } => pg.project(id).await,
            Self::Offline { cache, .. } => cache.project(id).await,
        }
    }
}

/// `<n>s` / `<n>m` / `<n>h`, truncating, with a negative age clamped to zero.
fn age(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < MINUTE {
        format!("{seconds}s")
    } else if seconds < HOUR {
        format!("{}m", seconds / MINUTE)
    } else {
        format!("{}h", seconds / HOUR)
    }
}

#[cfg(test)]
mod tests {
    use super::{Backend, age};
    use crate::cache::CacheStore;
    use crate::pg::PgStore;
    use chrono::{TimeDelta, Utc};
    use htui_core::store::MemStore;

    #[test]
    fn the_memory_backend_is_writable_and_has_no_mirror() {
        let mut backend = Backend::memory(MemStore::new());
        assert_eq!(backend.label(), "memory");
        assert!(backend.is_writable());
        assert!(backend.writable().is_none(), "MemStore is not a PgStore");
        assert!(backend.cache().is_none());
        assert!(
            !backend.went_offline(),
            "there is no mirror to fall back to"
        );
        assert_eq!(backend.label(), "memory");
    }

    #[test]
    fn the_age_units_truncate_at_a_minute_and_at_an_hour() {
        assert_eq!(
            age(-1),
            "0s",
            "a clock that moved backwards is not negative"
        );
        assert_eq!(age(0), "0s");
        assert_eq!(age(59), "59s");
        assert_eq!(age(60), "1m");
        assert_eq!(age(220), "3m", "3 min 40 s truncates to 3m");
        assert_eq!(age(3599), "59m");
        assert_eq!(age(3600), "1h");
        assert_eq!(age(86_400), "24h");
    }

    /// A mirror in a throwaway directory: no server, no `%APPDATA%`.
    async fn cache(root: &std::path::Path) -> CacheStore {
        CacheStore::open(root, "backend-test", PgStore::schema_version())
            .await
            .expect("open a throwaway mirror")
    }

    #[tokio::test]
    async fn an_offline_backend_reads_connecting_until_the_first_attempt_answers() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = cache(root.path()).await;

        let connecting = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert_eq!(connecting.label(), "connecting");
        assert!(!connecting.is_writable());
        assert!(
            connecting.writable().is_none(),
            "no write path when offline"
        );
        assert!(connecting.cache().is_some());

        let mut gave_up = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now() - TimeDelta::seconds(220)),
        };
        assert_eq!(gave_up.label(), "offline · 3m");
        assert!(
            !gave_up.went_offline(),
            "an offline backend is already there"
        );
        assert_eq!(
            gave_up.label(),
            "offline · 3m",
            "and its age is not restarted"
        );

        // An empty mirror answers rather than fails: offline reads go to the file (plan D12).
        assert!(
            connecting
                .workspaces()
                .await
                .expect("mirror read")
                .is_empty(),
            "a freshly built mirror is empty, not broken"
        );
        assert!(connecting.box_info().await.expect("mirror read").is_none());

        cache.close().await;
    }
}
