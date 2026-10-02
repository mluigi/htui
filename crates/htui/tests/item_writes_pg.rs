//! MOD-13 milestone 2 T5: the Backlog's item writes land on **Postgres**.
//!
//! `item_writes`' own tests prove the worker over a `MemStore`, and `tests/backlog.rs` proves the
//! tab. What only this file can prove is that `PgStore` answers the same requests the same way: a
//! mint continues the kind's counter as this user, an edit at the head writes the next version,
//! an edit at an older one answers `ItemDiverged` and writes nothing, a spec refusal writes
//! nothing, and the form's catalogue read matches the memory store's (plan D3-D6, D10).
//!
//! No harness: `store_worker::serve` over `Backend::Online { pg, cache }` is what the worker task
//! runs. The case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and
//! panics instead when `CI` is set, like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use htui::item_writes::{ItemFormContext, ItemWrite};
use htui::store_worker::{self, StoreReply, StoreRequest};
use htui_core::fixtures::ids;
use htui_core::model::{EditReason, Item, ItemFilter, ItemId, ItemSpec, Scope, SpecChanges};
use htui_core::store::{MemStore, ReadStore as _};
use htui_store::{Backend, CacheStore, PgStore, testkit};

/// The database, the mirror (and the directory it lives in) and the backend over both.
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new(name: &str) -> Option<Self> {
        let db = testkit::demo_db().await?;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), name, PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        Some(Self {
            db,
            _root: root,
            cache,
            backend,
        })
    }

    /// The Platform workspace's scope: `htui` then `agy`.
    async fn platform(&self) -> Scope {
        let platform = self
            .db
            .store
            .workspaces()
            .await
            .expect("the server's workspaces")
            .into_iter()
            .find(|workspace| workspace.slug == "platform")
            .expect("the demo holds `platform`");
        Scope::from_workspace(&platform)
    }

    /// How many items the server holds in `scope`.
    async fn item_count(&self, scope: &Scope) -> usize {
        self.db
            .store
            .items(scope, &ItemFilter::default())
            .await
            .expect("the server's items")
            .len()
    }

    /// `id`'s head on the server.
    async fn head(&self, id: ItemId) -> Item {
        self.db
            .store
            .item(id)
            .await
            .expect("the server answers")
            .expect("the item exists")
    }

    /// Closes the mirror and drops the database. The case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// A new htui `ANA` item titled `title`, every other column at its default.
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

/// A title-only edit of `ANA-2` at `expected`.
fn retitle(title: &str, expected: i32) -> StoreRequest {
    StoreRequest::EditItem {
        id: ids::HTUI_ANA_2,
        expected_version: expected,
        changes: SpecChanges {
            title: Some(title.to_owned()),
            ..SpecChanges::default()
        },
        reason: EditReason::Edited,
    }
}

/// The write an item request answered with; panics on anything else.
fn written(reply: StoreReply) -> (ItemId, ItemWrite) {
    match reply {
        StoreReply::ItemWritten { item, outcome } => (item, outcome),
        other => panic!("the write answered {other:?}"),
    }
}

/// D1, D5, D6: a mint, an edit at the head and an edit at the old version, on Postgres.
#[tokio::test(flavor = "multi_thread")]
async fn mint_edit_and_a_stale_edit_on_postgres() {
    let Some(stack) = Stack::new("item-writes-pg-mint").await else {
        return;
    };
    let backend = &stack.backend;

    let mint = StoreRequest::MintItem {
        project: ids::PROJECT_HTUI,
        spec: spec("  Fresh item "),
    };
    let (minted, outcome) = written(store_worker::serve(backend, &mint).await);
    assert_eq!(
        outcome,
        ItemWrite::Minted {
            key: "ANA-3".to_owned()
        },
        "the mint continues htui's ANA counter"
    );
    let row = stack.head(minted).await;
    assert_eq!(row.key, "ANA-3");
    assert_eq!(row.title, "Fresh item", "the title lands trimmed");
    assert_eq!(row.version, 1);
    assert_eq!(
        row.created_by,
        stack.db.store.this_user(),
        "the worker fills `created_by`"
    );

    let (edited, outcome) = written(store_worker::serve(backend, &retitle("Mine", 1)).await);
    assert_eq!(edited, ids::HTUI_ANA_2);
    assert_eq!(
        outcome,
        ItemWrite::Edited {
            key: "ANA-2".to_owned(),
            version: 2
        }
    );

    let reply = store_worker::serve(backend, &retitle("Stale", 1)).await;
    let StoreReply::ItemDiverged(divergence) = reply else {
        panic!("an edit at the old version diverges: {reply:?}");
    };
    assert_eq!(divergence.head.version, 2, "the head the edit missed");
    assert_eq!(divergence.head.title, "Mine");
    assert_eq!(
        divergence.ancestor.version, 1,
        "the version it was made from"
    );
    let head = stack.head(ids::HTUI_ANA_2).await;
    assert_eq!(
        (head.title.as_str(), head.version),
        ("Mine", 2),
        "the stale edit wrote nothing"
    );

    stack.finish().await;
}

/// D4: a touched path naming no repo of the project is refused by name, and nothing is written.
#[tokio::test(flavor = "multi_thread")]
async fn a_spec_refusal_writes_nothing_on_postgres() {
    let Some(stack) = Stack::new("item-writes-pg-refusal").await else {
        return;
    };
    let scope = stack.platform().await;
    let before = stack.item_count(&scope).await;

    let mint = StoreRequest::MintItem {
        project: ids::PROJECT_HTUI,
        spec: ItemSpec {
            touched_paths: vec!["nope:src".to_owned()],
            ..spec("Fresh item")
        },
    };
    match store_worker::serve(&stack.backend, &mint).await {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, "mint_item");
            assert!(message.contains("`nope`"), "the entry is named: {message}");
        }
        other => panic!("a bad path is refused: {other:?}"),
    }
    assert_eq!(stack.item_count(&scope).await, before, "nothing was minted");

    stack.finish().await;
}

/// D3: the new form's catalogue on Postgres is the memory store's: the same kinds and graphs in
/// the same order, and no repos (the demo seeds none).
#[tokio::test(flavor = "multi_thread")]
async fn the_form_read_on_postgres_matches_memory() {
    let Some(stack) = Stack::new("item-writes-pg-form").await else {
        return;
    };
    let read = StoreRequest::ItemForm {
        project: ids::PROJECT_HTUI,
        item: None,
    };
    let pg = form(store_worker::serve(&stack.backend, &read).await);
    let memory = form(store_worker::serve(&Backend::memory(MemStore::demo()), &read).await);

    assert_eq!(pg.project, ids::PROJECT_HTUI);
    assert_eq!(pg.item, None);
    assert_eq!(
        pg.kinds.iter().map(|kind| kind.id).collect::<Vec<_>>(),
        memory.kinds.iter().map(|kind| kind.id).collect::<Vec<_>>(),
        "the same kinds"
    );
    assert_eq!(
        pg.graphs.iter().map(|graph| graph.id).collect::<Vec<_>>(),
        memory
            .graphs
            .iter()
            .map(|graph| graph.id)
            .collect::<Vec<_>>(),
        "the same graphs"
    );
    assert!(
        pg.repos.is_empty(),
        "the demo seeds no repo: {:?}",
        pg.repos
    );

    stack.finish().await;
}

/// The catalogue an `ItemForm` read answered with; panics on anything else.
fn form(reply: StoreReply) -> ItemFormContext {
    match reply {
        StoreReply::ItemForm(context) => *context,
        other => panic!("the form read answered {other:?}"),
    }
}
