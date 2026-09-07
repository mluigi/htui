//! An owned handle on a writable store (MOD-2 milestone 3, plan D26).
//!
//! [`Backend::writable`](crate::Backend::writable) hands out a `&PgStore`, which is the right
//! shape for a caller that writes and returns. A **recorder** is not that caller: it lives for a
//! whole chat session, inside a task the store worker spawned, and it is generic over
//! `S: WriteStore` — it needs something it can own.
//!
//! [`Writer`] is that something, and it changes no invariant:
//! [`Backend::writer`](crate::Backend::writer) answers `None` for
//! [`Backend::Offline`](crate::Backend::Offline) exactly as `writable` does, so an offline write
//! is still unreachable rather than merely refused at runtime. Both arms are cheap handles —
//! `PgStore` is a pool handle and `MemStore` is an `Arc` — so a `Writer` is a clone of a handle,
//! never a copy of a store.
//!
//! `MemStore` is reachable here and is not through `writable`, deliberately: `--demo` and every
//! chat-tab snapshot run against it, and a seam only the production backend can exercise is a seam
//! no test covers.

use chrono::{DateTime, Utc};
use htui_core::model::{
    Agent, AgentBox, ChatRunSpec, DocumentHead, Item, ItemFilter, ItemId, ItemPatch, ItemSummary,
    LinkGraph, NewItem, Note, RunId, RunStatus, RunSummary, Scope, SessionEvent, Status, StepId,
};
use htui_core::store::{MemStore, ReadStore, Result, UpdateOutcome, WriteStore};
use serde_json::Value;

use crate::pg::PgStore;

/// A writable store a caller can hold.
#[derive(Debug, Clone)]
pub enum Writer {
    /// In-process (`--demo`, tests).
    Memory(MemStore),
    /// Postgres.
    Online(PgStore),
}

impl Writer {
    /// The backend label this writer belongs to, for logs.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Online(_) => "online",
        }
    }
}

/// Plain delegation: a `Writer` decides *which* store, never *what* a read means.
impl ReadStore for Writer {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        match self {
            Self::Memory(store) => store.items(scope, filter).await,
            Self::Online(pg) => pg.items(scope, filter).await,
        }
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        match self {
            Self::Memory(store) => store.item(id).await,
            Self::Online(pg) => pg.item(id).await,
        }
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        match self {
            Self::Memory(store) => store.links(id, hops).await,
            Self::Online(pg) => pg.links(id, hops).await,
        }
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        match self {
            Self::Memory(store) => store.documents(id).await,
            Self::Online(pg) => pg.documents(id).await,
        }
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        match self {
            Self::Memory(store) => store.notes(id).await,
            Self::Online(pg) => pg.notes(id).await,
        }
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        match self {
            Self::Memory(store) => store.runs(id).await,
            Self::Online(pg) => pg.runs(id).await,
        }
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        match self {
            Self::Memory(store) => store.step_events(step).await,
            Self::Online(pg) => pg.step_events(step).await,
        }
    }
}

impl WriteStore for Writer {
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        match self {
            Self::Memory(store) => store.mint_item(new).await,
            Self::Online(pg) => pg.mint_item(new).await,
        }
    }

    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        match self {
            Self::Memory(store) => store.update_item(id, expected_version, patch).await,
            Self::Online(pg) => pg.update_item(id, expected_version, patch).await,
        }
    }

    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        match self {
            Self::Memory(store) => store.transition(id, from, to).await,
            Self::Online(pg) => pg.transition(id, from, to).await,
        }
    }

    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize> {
        match self {
            Self::Memory(store) => store.append_events(events).await,
            Self::Online(pg) => pg.append_events(events).await,
        }
    }

    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.set_step_usage(step, usage, prompt_digest).await,
            Self::Online(pg) => pg.set_step_usage(step, usage, prompt_digest).await,
        }
    }

    async fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_agent(agent).await,
            Self::Online(pg) => pg.upsert_agent(agent).await,
        }
    }

    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        match self {
            Self::Memory(store) => store.upsert_agent_box(row).await,
            Self::Online(pg) => pg.upsert_agent_box(row).await,
        }
    }

    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        match self {
            Self::Memory(store) => store.start_chat_run(chat).await,
            Self::Online(pg) => pg.start_chat_run(chat).await,
        }
    }

    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        match self {
            Self::Memory(store) => store.finish_chat_run(run, step, status, finished_at).await,
            Self::Online(pg) => pg.finish_chat_run(run, step, status, finished_at).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Backend;
    use htui_core::fixtures::ids;

    #[tokio::test]
    async fn a_memory_writer_round_trips_a_chat_run() {
        let store = MemStore::demo();
        let backend = Backend::memory(store.clone());
        let writer = backend.writer().expect("a memory backend is writable");
        assert_eq!(writer.label(), "memory");

        let scope = Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let before = store.active_runs(&scope).await.expect("count");

        let chat = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            None,
        );
        writer.start_chat_run(&chat).await.expect("the rows mint");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before + 1,
            "a live chat counts as an active run"
        );

        writer
            .finish_chat_run(
                chat.run_id,
                chat.step_id,
                RunStatus::Done,
                htui_core::fixtures::demo_at(0, 0),
            )
            .await
            .expect("the run closes");
        assert_eq!(
            store.active_runs(&scope).await.expect("count"),
            before,
            "a closed chat stops counting"
        );
    }

    #[tokio::test]
    async fn an_offline_backend_hands_out_no_writer_and_no_user() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = crate::cache::CacheStore::open(root.path(), "writer-test", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(backend.writer().is_none(), "no write path when offline");
        assert!(
            matches!(
                backend.this_user().await,
                Err(htui_core::store::StoreError::Unreachable(_))
            ),
            "and no author for a run row"
        );
        cache.close().await;
    }

    #[tokio::test]
    async fn a_memory_backend_answers_the_fixture_user() {
        let backend = Backend::memory(MemStore::demo());
        assert_eq!(
            backend.this_user().await.expect("the fixture has a user"),
            ids::USER
        );

        let empty = Backend::memory(MemStore::new());
        assert!(
            matches!(
                empty.this_user().await,
                Err(htui_core::store::StoreError::NotFound { .. })
            ),
            "an empty store has no user to start a run as"
        );
    }
}
