//! The store the application runs against (ANA-9 §6.1, plan D1, D12, blueprint C.16).
//!
//! [`Backend`] is an enum, not a `Box<dyn ReadStore>`: native `async fn` in traits is not object
//! safe, and a concrete type keeps the spawned worker's futures `Send`-inferable (MOD-1 plan D2).
//!
//! There is no `Backend::write*` method and no `impl WriteStore for Backend`. `WriteStore` is
//! implemented by [`PgStore`] alone and the only way to reach one is [`Backend::writable`], so a
//! write attempt against an offline store is a **compile error** rather than a runtime flag
//! (plan D1). That claim is documented here and deliberately not covered by a test: a compile-fail
//! test is not worth a `trybuild` dependency.

use chrono::{DateTime, Utc};
use htui_core::model::{
    BoxInfo, DocumentHead, Item, ItemFilter, ItemId, ItemSummary, LinkGraph, Note, ProjectRef,
    RunSummary, Scope, SessionEvent, StepId, WorkspaceSummary,
};
use htui_core::store::{MemStore, ReadStore, Result};

use crate::cache::CacheStore;
use crate::pg::PgStore;

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
    /// Postgres unreachable: reads come from the mirror, there is no write path at all.
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

    /// Whether write paths are reachable. [`Backend::Offline`] is the first backend to answer
    /// `false`.
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
