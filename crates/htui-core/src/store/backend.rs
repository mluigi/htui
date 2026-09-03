//! The concrete store the TUI holds.
//!
//! [`Backend`] is an enum, not a `Box<dyn ReadStore>`: native `async fn` in traits is not object
//! safe, and a concrete type keeps the spawned worker's futures `Send`-inferable (plan D2). MOD-6
//! adds `Online(PgStore, CacheStore)` and `Offline(CacheStore)`, at which point a write path is
//! unreachable in `Offline` at the type level rather than behind a runtime flag (§6.1).

use crate::model::{
    BoxInfo, DocumentHead, Item, ItemFilter, ItemId, ItemSummary, LinkGraph, Note, ProjectRef,
    RunSummary, Scope, SessionEvent, StepId, WorkspaceSummary,
};
use crate::store::error::Result;
use crate::store::mem::MemStore;
use crate::store::traits::ReadStore;

/// The store the application runs against.
#[derive(Debug, Clone)]
pub enum Backend {
    /// Everything in process memory (MOD-1, tests, `--demo`).
    Memory(MemStore),
    // MOD-6: Online(PgStore, CacheStore), Offline(CacheStore)  — §6.1
}

impl Backend {
    /// Wraps an in-memory store.
    #[must_use]
    pub fn memory(store: MemStore) -> Self {
        Self::Memory(store)
    }

    /// Store-state text for the top bar. MOD-6 returns `"online"` / `"offline · 3m"`.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Memory(_) => "memory".to_owned(),
        }
    }

    /// Whether write paths are reachable. `Offline` will be the first backend to answer `false`.
    #[must_use]
    pub fn is_writable(&self) -> bool {
        match self {
            Self::Memory(_) => true,
        }
    }

    /// Every workspace, ordered by name.
    ///
    /// Hierarchy reads are inherent methods rather than [`ReadStore`] methods: §6.1 is quoted
    /// verbatim (plan V2) and has no `workspaces()`, and guessing at MOD-6's trait shape now would
    /// be worse than a method on the concrete type the store worker is the only caller of
    /// (blueprint B.7).
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        match self {
            Self::Memory(store) => store.workspaces().await,
        }
    }

    /// This box's row, projected for the top bar.
    pub async fn box_info(&self) -> Result<Option<BoxInfo>> {
        match self {
            Self::Memory(store) => store.box_info().await,
        }
    }

    /// How many runs of the scope are active (`RunStatus::is_active`).
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize> {
        match self {
            Self::Memory(store) => store.active_runs(scope).await,
        }
    }

    /// The scope's projects, ordered by `workspace_project.position`.
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>> {
        match self {
            Self::Memory(store) => store.projects(scope).await,
        }
    }
}

impl ReadStore for Backend {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        match self {
            Self::Memory(store) => store.items(scope, filter).await,
        }
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        match self {
            Self::Memory(store) => store.item(id).await,
        }
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        match self {
            Self::Memory(store) => store.links(id, hops).await,
        }
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        match self {
            Self::Memory(store) => store.documents(id).await,
        }
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        match self {
            Self::Memory(store) => store.notes(id).await,
        }
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        match self {
            Self::Memory(store) => store.runs(id).await,
        }
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        match self {
            Self::Memory(store) => store.step_events(step).await,
        }
    }
}
