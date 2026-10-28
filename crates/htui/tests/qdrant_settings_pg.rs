//! `Settings > Qdrant`, the worker half on **Postgres** (MOD-158 D9, D10): the database-wide
//! `qdrant_url` row read and written by compare-and-set, the old keyring URL used for every box or
//! discarded, and what an offline store still does.
//!
//! No harness: `qdrant_settings_info::serve_with` over `Backend::Online { pg, cache }` is what the
//! store loop runs for the seven requests, with a [`QdrantEnv`] literal so no case reads
//! `HTUI_QDRANT_*` from the process (blueprint P-13), and the fake keyring
//! (`testkit::mock_keyring`) so no case reaches the real one. The Online cases print
//! `testkit::SKIP` and return with `HTUI_TEST_DATABASE_URL` unset; the Offline ones need no server.
#![cfg(feature = "testkit")]

use htui::qdrant_settings_info::{
    NO_OLD_URL, OldUrl, QdrantSnapshot, QdrantState, URL_WRITE_OFFLINE, UrlRowState,
    old_url_unusable, serve_with,
};
use htui::secrets_settings::Redacted;
use htui::store_worker::{StoreReply, StoreRequest};
use htui_core::qdrant::{QdrantUrl, QdrantUrlError};
use htui_store::secret::QdrantEnv;
use htui_store::{Backend, CacheStore, PgStore, testkit};

/// The URL a case sets.
const URL: &str = "http://qdrant.example:6334";
/// A second URL, for an overwrite.
const OTHER: &str = "https://other.example:6334";
/// What this box's keyring holds from before (D10).
const OLD: &str = "http://old.example:6334";

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

    /// One request with no override in force.
    async fn serve(&self, request: StoreRequest) -> StoreReply {
        serve_with(&self.backend, &request, &QdrantEnv::default())
            .await
            .expect("the request is served")
    }

    /// Closes the mirror and drops the database. The case calls this on its last line.
    async fn finish(self) {
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

fn url(text: &str) -> QdrantUrl {
    QdrantUrl::parse(text).expect("a valid URL")
}

/// The snapshot of a `Qdrant` reply.
#[track_caller]
fn snapshot(reply: StoreReply) -> QdrantSnapshot {
    match reply {
        StoreReply::Qdrant(snapshot) => snapshot,
        other => panic!("expected a Qdrant snapshot, got {other:?}"),
    }
}

/// The snapshot of a `QdrantStale` reply.
#[track_caller]
fn stale(reply: StoreReply) -> QdrantSnapshot {
    match reply {
        StoreReply::QdrantStale(snapshot) => *snapshot,
        other => panic!("expected QdrantStale, got {other:?}"),
    }
}

/// The message of a `Failed` reply to `name`.
#[track_caller]
fn failed(reply: StoreReply, name: &str) -> String {
    match reply {
        StoreReply::Failed { request, message } => {
            assert_eq!(request, name);
            message
        }
        other => panic!("expected `{name}` refused, got {other:?}"),
    }
}

/// The `Set` row's URL and token.
#[track_caller]
fn set_row(snapshot: &QdrantSnapshot) -> (QdrantUrl, chrono::DateTime<chrono::Utc>) {
    match &snapshot.url {
        UrlRowState::Set { url, token } => (url.clone(), *token),
        other => panic!("expected a set row, got {other:?}"),
    }
}

/// D1, D9: no row reads `NotSet`; a set expecting none lands with its token; a clear by that token
/// removes it, for every box.
#[tokio::test]
async fn set_then_clear_the_url_row() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-set-clear").await else {
        return;
    };
    let read = snapshot(stack.serve(StoreRequest::QdrantInfo).await);
    assert_eq!(read.url, UrlRowState::NotSet);
    assert_eq!(read.key_state, QdrantState::NotStored);
    assert_eq!(read.old_url, None);
    assert_eq!(read.url_override, None);
    assert!(!read.key_override);

    let set = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            })
            .await,
    );
    let (landed, token) = set_row(&set);
    assert_eq!(landed, url(URL));
    let stored = stack
        .db
        .store
        .qdrant_url()
        .await
        .expect("the row reads")
        .expect("the row is there");
    assert_eq!(stored.token, token, "the snapshot carries the row's token");

    let cleared = snapshot(
        stack
            .serve(StoreRequest::ClearQdrantUrl { expected: token })
            .await,
    );
    assert_eq!(cleared.url, UrlRowState::NotSet);
    assert_eq!(
        stack.db.store.qdrant_url().await.expect("the row reads"),
        None
    );
    stack.finish().await;
}

/// R-3: a write with a spent token, or expecting no row over one, does not apply and answers the
/// row now under `QdrantStale`, whose token the next write presents.
#[tokio::test]
async fn a_spent_token_answers_qdrant_stale() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-stale").await else {
        return;
    };
    let first = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            })
            .await,
    );
    let (_, spent) = set_row(&first);
    let second = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(OTHER),
                expected: Some(spent),
            })
            .await,
    );
    let (now_url, now) = set_row(&second);
    assert_eq!(now_url, url(OTHER));
    assert_ne!(now, spent, "the overwrite moved the token");

    let missed = stale(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: Some(spent),
            })
            .await,
    );
    assert_eq!(set_row(&missed), (url(OTHER), now), "nothing was written");
    let missed = stale(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            })
            .await,
    );
    assert_eq!(
        set_row(&missed),
        (url(OTHER), now),
        "a row is already there"
    );
    let missed = stale(
        stack
            .serve(StoreRequest::ClearQdrantUrl { expected: spent })
            .await,
    );
    assert_eq!(set_row(&missed), (url(OTHER), now), "nothing was removed");

    // The fresh token works.
    let cleared = snapshot(
        stack
            .serve(StoreRequest::ClearQdrantUrl { expected: now })
            .await,
    );
    assert_eq!(cleared.url, UrlRowState::NotSet);
    stack.finish().await;
}

/// D10: with no row, the old keyring URL becomes the row for every box and the entry goes.
#[tokio::test]
async fn adopt_moves_the_keyring_url_to_the_row_and_clears_the_entry() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-adopt").await else {
        return;
    };
    htui_store::secret::set_qdrant_url(OLD).expect("the fake keyring stores it");
    let read = snapshot(stack.serve(StoreRequest::QdrantInfo).await);
    assert_eq!(read.old_url, Some(OldUrl::Usable(url(OLD))));
    assert_eq!(read.url, UrlRowState::NotSet);

    let adopted = snapshot(stack.serve(StoreRequest::AdoptOldQdrantUrl).await);
    assert_eq!(set_row(&adopted).0, url(OLD));
    assert_eq!(adopted.old_url, None, "the offer is gone");
    assert_eq!(
        testkit::fake_qdrant_dsn(),
        None,
        "the keyring entry is gone"
    );
    let stored = stack
        .db
        .store
        .qdrant_url()
        .await
        .expect("the row reads")
        .expect("the row is there");
    assert_eq!(stored.url, Ok(url(OLD)));

    // Nothing left to adopt.
    assert_eq!(
        failed(
            stack.serve(StoreRequest::AdoptOldQdrantUrl).await,
            "adopt_old_qdrant_url"
        ),
        NO_OLD_URL
    );
    stack.finish().await;
}

/// R-4: over a row already set (another box adopted first), the adopt is `QdrantStale`, the row is
/// unchanged and the keyring entry is kept, so the offer reads discard-only. An entry that fails
/// D2 is refused with its reason and kept too.
#[tokio::test]
async fn adopt_over_a_set_row_is_stale_and_keeps_the_entry() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-adopt-stale").await else {
        return;
    };
    let set = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            })
            .await,
    );
    let row = set_row(&set);
    htui_store::secret::set_qdrant_url(OLD).expect("the fake keyring stores it");

    let missed = stale(stack.serve(StoreRequest::AdoptOldQdrantUrl).await);
    assert_eq!(set_row(&missed), row, "the row is unchanged");
    assert_eq!(missed.old_url, Some(OldUrl::Usable(url(OLD))));
    assert_eq!(testkit::fake_qdrant_dsn().as_deref(), Some(OLD), "kept");

    htui_store::secret::set_qdrant_url("http://u:pw@old.example:6334").expect("stored");
    let read = snapshot(stack.serve(StoreRequest::QdrantInfo).await);
    assert_eq!(
        read.old_url,
        Some(OldUrl::Unusable(QdrantUrlError::Credentials))
    );
    assert_eq!(
        failed(
            stack.serve(StoreRequest::AdoptOldQdrantUrl).await,
            "adopt_old_qdrant_url"
        ),
        old_url_unusable(QdrantUrlError::Credentials)
    );
    assert!(testkit::fake_qdrant_dsn().is_some(), "kept for the discard");
    stack.finish().await;
}

/// D10: the discard removes the old keyring entry and nothing else: not the row, not the key.
#[tokio::test]
async fn discard_removes_only_the_entry() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-discard").await else {
        return;
    };
    let set = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            })
            .await,
    );
    let row = set_row(&set);
    htui_store::secret::set_qdrant_url(OLD).expect("stored");
    htui_store::secret::set_qdrant_api_key("qk-fixture").expect("stored");

    let discarded = snapshot(stack.serve(StoreRequest::DiscardOldQdrantUrl).await);
    assert_eq!(discarded.old_url, None);
    assert_eq!(testkit::fake_qdrant_dsn(), None);
    assert_eq!(set_row(&discarded), row, "the row stays");
    assert_eq!(discarded.key_state, QdrantState::Stored, "the key stays");
    stack.finish().await;
}

/// P-3: a row that does not parse reads `Unusable` with its token, never "not set", and can be
/// overwritten by that token, then cleared.
#[tokio::test]
async fn a_malformed_row_reads_unusable_with_its_token() {
    let _keyring = testkit::mock_keyring().await;
    let Some(stack) = Stack::new("qdrant-malformed").await else {
        return;
    };
    sqlx::query("INSERT INTO app_setting (key, value) VALUES ('qdrant_url', '42'::jsonb)")
        .execute(&stack.db.pool)
        .await
        .expect("plant a malformed qdrant_url row");
    let read = snapshot(stack.serve(StoreRequest::QdrantInfo).await);
    let token = match read.url {
        UrlRowState::Unusable { why, token } => {
            assert_eq!(why, QdrantUrlError::NotAString);
            token
        }
        other => panic!("expected an unusable row, got {other:?}"),
    };
    let fixed = snapshot(
        stack
            .serve(StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: Some(token),
            })
            .await,
    );
    let (_, token) = set_row(&fixed);
    let cleared = snapshot(
        stack
            .serve(StoreRequest::ClearQdrantUrl { expected: token })
            .await,
    );
    assert_eq!(cleared.url, UrlRowState::NotSet);
    stack.finish().await;
}

/// The offline backend this box falls back to: its mirror, no server.
async fn offline(root: &tempfile::TempDir) -> (CacheStore, Backend) {
    let cache = CacheStore::open(root.path(), "qdrant-offline", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    };
    (cache, backend)
}

/// D9: offline the URL row cannot be read, the URL writes and the adopt are refused by name, and
/// this box's key and old entry, which live in its keyring, are still stored and removed.
#[tokio::test]
async fn offline_refuses_url_writes_but_stores_the_key() {
    let _keyring = testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let (cache, backend) = offline(&root).await;
    let env = QdrantEnv::default();
    let serve = async |request: StoreRequest| {
        serve_with(&backend, &request, &env)
            .await
            .expect("the request is served")
    };

    let read = snapshot(serve(StoreRequest::QdrantInfo).await);
    assert_eq!(read.url, UrlRowState::Offline);
    for (request, name) in [
        (
            StoreRequest::SetQdrantUrl {
                url: url(URL),
                expected: None,
            },
            "set_qdrant_url",
        ),
        (
            StoreRequest::ClearQdrantUrl {
                expected: chrono::Utc::now(),
            },
            "clear_qdrant_url",
        ),
        (StoreRequest::AdoptOldQdrantUrl, "adopt_old_qdrant_url"),
    ] {
        assert_eq!(failed(serve(request).await, name), URL_WRITE_OFFLINE);
    }

    let stored = snapshot(
        serve(StoreRequest::SetQdrantApiKey(Redacted::new(
            "qk-fixture".to_owned(),
        )))
        .await,
    );
    assert_eq!(stored.key_state, QdrantState::Stored);
    assert_eq!(stored.url, UrlRowState::Offline);
    let cleared = snapshot(serve(StoreRequest::ClearQdrantApiKey).await);
    assert_eq!(cleared.key_state, QdrantState::NotStored);

    htui_store::secret::set_qdrant_url(OLD).expect("stored");
    let discarded = snapshot(serve(StoreRequest::DiscardOldQdrantUrl).await);
    assert_eq!(discarded.old_url, None);
    assert_eq!(testkit::fake_qdrant_dsn(), None);
    cache.close().await;
}

/// D3, D4: the overrides the process was handed are in the snapshot, parsed by D2: a usable one,
/// and one that is not, with its reason; the key only as "set", never its text.
#[tokio::test]
async fn the_overrides_in_force_are_in_the_snapshot() {
    let _keyring = testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let (cache, backend) = offline(&root).await;

    let env = QdrantEnv::new(
        Some("http://env.example:6334".to_owned()),
        Some("qk-env".to_owned()),
    );
    let read = snapshot(
        serve_with(&backend, &StoreRequest::QdrantInfo, &env)
            .await
            .expect("served"),
    );
    assert_eq!(read.url_override, Some(Ok(url("http://env.example:6334"))));
    assert!(read.key_override);
    assert!(!format!("{read:?}").contains("qk-env"), "{read:?}");

    let env = QdrantEnv::new(Some("env.example:6334".to_owned()), None);
    let read = snapshot(
        serve_with(&backend, &StoreRequest::QdrantInfo, &env)
            .await
            .expect("served"),
    );
    assert_eq!(read.url_override, Some(Err(QdrantUrlError::Scheme)));
    assert!(!read.key_override);
    cache.close().await;
}
