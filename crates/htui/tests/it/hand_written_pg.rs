//! MOD-13 milestone 5: the Backlog's hand-written notes and documents land on **Postgres**.
//!
//! `hand_written`'s own tests prove the worker over a `MemStore`, and `tests/backlog.rs` proves
//! the Notes and Docs panes. What only this file can prove is that `PgStore` answers the same
//! four requests the same way: a note lands trimmed, by hand, as this user on this box; a document
//! lands at the store's next version of its kind (D5: append-only, never a compare-and-set); the
//! `v` form's read prefills from the kind's latest version; and a refusal or an unknown item
//! writes nothing, with the sentence the memory store gives (plan D1-D5, D10).
//!
//! No harness: `store_worker::serve` over `Backend::Online { pg, cache }` is what the worker task
//! runs, compared with the same request over `Backend::memory(MemStore::demo())`. A reply's
//! `Debug` carries no body (D1), so it is the comparable outcome. The case prints
//! `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics instead when `CI`
//! is set, like every other Postgres-backed suite.
#![cfg(feature = "testkit")]

use htui::hand_written::HandText;
use htui::store_worker::{self, StoreReply, StoreRequest};
use htui_core::fixtures::ids;
use htui_core::model::ItemId;
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

    /// Closes the mirror and drops the database. The case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// The demo over the memory store: the reference every Postgres answer is compared with.
fn memory() -> Backend {
    Backend::memory(MemStore::demo())
}

/// A reply as comparable text. `Debug` holds no body (D1), so this is the outcome.
fn shape(reply: &StoreReply) -> String {
    format!("{reply:?}")
}

/// A note on `item`.
fn add_note(item: ItemId, body: &str) -> StoreRequest {
    StoreRequest::AddNote {
        item,
        body: HandText::new(body),
    }
}

/// A document of `kind` on `item`.
fn write_document(item: ItemId, kind: &str, title: &str, body: &str) -> StoreRequest {
    StoreRequest::WriteDocument {
        item,
        kind: kind.to_owned(),
        title: title.to_owned(),
        body: HandText::new(body),
    }
}

/// `request` served on both backends; panics unless the two answer alike. Returns the answer.
async fn alike(pg: &Backend, memory: &Backend, request: &StoreRequest) -> StoreReply {
    let on_pg = store_worker::serve(pg, request).await;
    let on_memory = store_worker::serve(memory, request).await;
    assert_eq!(
        shape(&on_pg),
        shape(&on_memory),
        "{request:?} answers alike"
    );
    on_pg
}

/// The version a `WriteDocument` landed at; panics on anything else.
fn landed(reply: &StoreReply) -> i32 {
    match reply {
        StoreReply::DocumentWritten { version, .. } => *version,
        other => panic!("the write answered {other:?}"),
    }
}

/// The message a request failed with; panics on anything else.
fn failed(reply: &StoreReply) -> &str {
    match reply {
        StoreReply::Failed { message, .. } => message,
        other => panic!("the request answered {other:?}"),
    }
}

/// D1, D4, D5: a note and three documents land on Postgres as on memory, by hand.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_and_a_document_land_on_postgres_as_on_memory() {
    let Some(stack) = Stack::new("hand-written-pg-land").await else {
        return;
    };
    let pg = &stack.backend;
    let memory = memory();

    let reply = alike(
        pg,
        &memory,
        &add_note(ids::HTUI_FEAT_1, "Written by hand.\n\n"),
    )
    .await;
    assert!(
        matches!(reply, StoreReply::NoteAdded { item } if item == ids::HTUI_FEAT_1),
        "{reply:?}"
    );
    let notes = stack
        .db
        .store
        .notes(ids::HTUI_FEAT_1)
        .await
        .expect("the server's notes");
    let note = notes.last().expect("the note landed");
    assert_eq!(
        note.body, "Written by hand.",
        "the body lands trimmed at the end"
    );
    assert_eq!(note.via_step_id, None, "by hand");
    assert_eq!(note.created_by, stack.db.store.this_user());
    assert_eq!(
        note.box_id,
        pg.box_info()
            .await
            .expect("the server's box")
            .map(|info| info.box_id),
        "the worker fills the box"
    );

    let plan = alike(
        pg,
        &memory,
        &write_document(ids::HTUI_FEAT_1, "plan", "Plan, by hand", "The plan."),
    )
    .await;
    assert_eq!(landed(&plan), 3, "the store's next version of `plan`");
    let review = alike(
        pg,
        &memory,
        &write_document(ids::HTUI_FEAT_1, "review", "Review", "The review."),
    )
    .await;
    assert_eq!(landed(&review), 1);
    let summary = alike(
        pg,
        &memory,
        &write_document(ids::HTUI_ANA_2, "summary", "Summary", "The summary."),
    )
    .await;
    assert_eq!(landed(&summary), 1);

    let head = stack
        .db
        .store
        .documents(ids::HTUI_FEAT_1)
        .await
        .expect("the server's documents")
        .into_iter()
        .find(|document| document.kind == "plan" && document.version == 3)
        .expect("plan v3 is listed");
    let v3 = stack
        .db
        .store
        .document(head.id)
        .await
        .expect("the server answers")
        .expect("plan v3");
    assert_eq!(v3.produced_by_step_id, None, "by hand");
    assert_eq!(v3.title, "Plan, by hand");
    assert_eq!(v3.body, "The plan.");

    stack.finish().await;
}

/// D9: the `v` form's read prefills from the version the next step reads on Postgres as on memory
/// (MOD-73 review M1: `ANA-1`'s research loser, v3, is never the base); the `a` read prefills
/// nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_document_form_on_postgres_matches_memory() {
    let Some(stack) = Stack::new("hand-written-pg-form").await else {
        return;
    };
    let pg = &stack.backend;
    let memory = memory();

    let base = |reply: StoreReply| match reply {
        StoreReply::DocumentForm(context) => {
            context.base.map(|base| (base.id, base.version, base.title))
        }
        other => panic!("the form read answered {other:?}"),
    };
    let request = StoreRequest::DocumentForm {
        item: ids::HTUI_FEAT_1,
        kind: Some("plan".to_owned()),
    };
    let on_pg = base(store_worker::serve(pg, &request).await);
    assert_eq!(on_pg, base(store_worker::serve(&memory, &request).await));
    assert_eq!(
        on_pg,
        Some((
            ids::DOC_FEAT_1_PLAN_V2,
            2,
            "Plan: TUI scaffold (revised)".to_owned()
        ))
    );

    let request = StoreRequest::DocumentForm {
        item: ids::HTUI_ANA_1,
        kind: Some("research".to_owned()),
    };
    let on_pg = base(store_worker::serve(pg, &request).await);
    assert_eq!(on_pg, base(store_worker::serve(&memory, &request).await));
    assert_eq!(
        on_pg.map(|(id, version, _)| (id, version)),
        Some((ids::DOC_ANA_1_RESEARCH_V2, 2)),
        "the selected output, not the loser's v3"
    );

    let request = StoreRequest::DocumentForm {
        item: ids::HTUI_FEAT_1,
        kind: None,
    };
    let on_pg = base(store_worker::serve(pg, &request).await);
    assert_eq!(on_pg, base(store_worker::serve(&memory, &request).await));
    assert_eq!(on_pg, None);

    stack.finish().await;
}

/// D3, D4, D10: a refusal and an unknown item answer `Failed` with the memory store's sentence,
/// and nothing lands on Postgres.
#[tokio::test(flavor = "multi_thread")]
async fn refusals_and_an_unknown_item_write_nothing_on_postgres() {
    let Some(stack) = Stack::new("hand-written-pg-refused").await else {
        return;
    };
    let pg = &stack.backend;
    let memory = memory();
    let store = &stack.db.store;
    let notes = store
        .notes(ids::HTUI_FEAT_1)
        .await
        .expect("the server's notes")
        .len();
    let documents = store
        .documents(ids::HTUI_FEAT_1)
        .await
        .expect("the server's documents")
        .len();

    let blank = alike(pg, &memory, &add_note(ids::HTUI_FEAT_1, "  \n\n")).await;
    assert!(failed(&blank).contains("note"), "{blank:?}");
    let kind = alike(
        pg,
        &memory,
        &write_document(ids::HTUI_FEAT_1, "pl\nan", "Title", "Body."),
    )
    .await;
    assert!(failed(&kind).contains("kind"), "{kind:?}");

    let unknown = ItemId::new();
    let not_found = format!("item `{unknown}` not found");
    for request in [
        StoreRequest::NoteForm { item: unknown },
        add_note(unknown, "A note."),
        StoreRequest::DocumentForm {
            item: unknown,
            kind: None,
        },
        write_document(unknown, "plan", "Title", "Body."),
    ] {
        let reply = alike(pg, &memory, &request).await;
        assert!(
            failed(&reply).contains(&not_found),
            "{request:?} answered {reply:?}"
        );
    }

    assert_eq!(
        store
            .notes(ids::HTUI_FEAT_1)
            .await
            .expect("the server's notes")
            .len(),
        notes,
        "no note landed"
    );
    assert_eq!(
        store
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("the server's documents")
            .len(),
        documents,
        "no document landed"
    );
    // Review L3: the counts above are FEAT-1's, which a write to `unknown` could never move; a
    // row that slipped through would be the unknown item's.
    assert!(
        store
            .notes(unknown)
            .await
            .expect("the server's notes")
            .is_empty(),
        "no note landed on the unknown item"
    );
    assert!(
        store
            .documents(unknown)
            .await
            .expect("the server's documents")
            .is_empty(),
        "no document landed on the unknown item"
    );

    stack.finish().await;
}
