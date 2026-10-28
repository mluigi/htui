//! MOD-158 T5 (D8): `htui --set-qdrant-url`, `--clear-qdrant-url`, `--set-qdrant-key` and
//! `--clear-qdrant-key`, through `qdrant_cmd`'s cores.
//!
//! The URL cases run over `testkit::demo_db()` through `set_url_on` / `clear_url_on` (the connect
//! itself is `concepts::open`'s, pinned elsewhere); each returns early, printing `testkit::SKIP`,
//! with `HTUI_TEST_DATABASE_URL` unset. The key cases and the refusals before any connect run over
//! the fake keyring and never skip. No case reads `HTUI_QDRANT_*` from the process (blueprint
//! P-13): the override is passed in as a literal. Run with `--test-threads=1`: the fake keyring is
//! process-wide.
#![cfg(feature = "testkit")]

use std::io::Read as _;

use htui::qdrant_cmd;
use htui::state_cmd::StateExit;
use htui_core::qdrant::{QDRANT_URL_KEY, QdrantUrl, QdrantUrlError};
use htui_store::{secret, testkit};
use serde_json::Value;

/// `concepts::NO_DSN`, the sentence every verb that needs the keyring's DSN refuses with.
const NO_DSN: &str = "no Postgres DSN is stored; run `htui --set-dsn` first";

/// The `qdrant_url` row's value, or `None` when there is no row.
async fn row(db: &testkit::TestDb) -> Option<Value> {
    sqlx::query_scalar::<_, Value>("SELECT value FROM app_setting WHERE key = $1")
        .bind(QDRANT_URL_KEY)
        .fetch_optional(&db.pool)
        .await
        .expect("read the qdrant_url row")
}

/// The row's compare-and-set token, through D1's read.
async fn token(db: &testkit::TestDb) -> chrono::DateTime<chrono::Utc> {
    db.store
        .qdrant_url()
        .await
        .expect("read")
        .expect("a row")
        .token
}

/// A refusal's sentence; panics on any other ending.
fn refused(ended: Result<String, StateExit>) -> String {
    match ended {
        Err(StateExit::Refused(sentence)) => sentence,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn set_stores_the_row_as_a_json_string() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let line = qdrant_cmd::set_url_on(&db.store, "  HTTP://qdrant.lan:6334  ", None)
        .await
        .expect("set");
    assert_eq!(
        line,
        "Qdrant URL set for every box: http://qdrant.lan:6334."
    );
    assert_eq!(
        row(&db).await,
        Some(Value::String("http://qdrant.lan:6334".into())),
        "trimmed, scheme lowercased, stored as a JSON string"
    );
    let stored = db.store.qdrant_url().await.expect("read").expect("a row");
    assert_eq!(
        stored.usable().expect("usable"),
        &QdrantUrl::parse("http://qdrant.lan:6334").expect("parse")
    );
    db.drop_db().await;
}

#[tokio::test]
async fn a_url_with_credentials_is_refused_and_writes_nothing() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    for (text, why) in [
        (
            "http://admin:SENTINEL-PW@qdrant.lan:6334",
            QdrantUrlError::Credentials,
        ),
        (
            "http://SENTINEL-PW@qdrant.lan:6334",
            QdrantUrlError::Credentials,
        ),
        (
            "http://qdrant.lan:6334/?api-key=SENTINEL-PW",
            QdrantUrlError::Query,
        ),
        (
            "http://qdrant.lan:6334/#SENTINEL-PW",
            QdrantUrlError::Fragment,
        ),
        ("qdrant.lan:6334", QdrantUrlError::Scheme),
        ("   ", QdrantUrlError::Empty),
    ] {
        let sentence = refused(qdrant_cmd::set_url_on(&db.store, text, None).await);
        assert_eq!(
            sentence,
            format!("not setting the Qdrant URL: {why}"),
            "{text}"
        );
        assert!(!sentence.contains("SENTINEL"), "the refusal echoes the URL");
    }
    assert_eq!(row(&db).await, None, "nothing was written");
    db.drop_db().await;
}

#[tokio::test]
async fn set_over_a_set_row_updates_it_by_its_token() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    qdrant_cmd::set_url_on(&db.store, "http://qdrant-a.lan:6334", None)
        .await
        .expect("first set");
    let first = token(&db).await;
    let line = qdrant_cmd::set_url_on(&db.store, "https://qdrant-b.lan", None)
        .await
        .expect("second set");
    assert_eq!(line, "Qdrant URL set for every box: https://qdrant-b.lan.");
    assert_eq!(
        row(&db).await,
        Some(Value::String("https://qdrant-b.lan".into()))
    );
    assert!(token(&db).await > first, "the update moved the token");
    db.drop_db().await;
}

#[tokio::test]
async fn set_over_a_malformed_row_fixes_it() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    sqlx::query("INSERT INTO app_setting (key, value) VALUES ($1, $2)")
        .bind(QDRANT_URL_KEY)
        .bind(serde_json::json!(42))
        .execute(&db.pool)
        .await
        .expect("plant a malformed row");
    let stored = db.store.qdrant_url().await.expect("read").expect("a row");
    assert_eq!(stored.url, Err(QdrantUrlError::NotAString));
    qdrant_cmd::set_url_on(&db.store, "http://qdrant.lan:6334", None)
        .await
        .expect("set over it");
    assert_eq!(
        row(&db).await,
        Some(Value::String("http://qdrant.lan:6334".into()))
    );
    db.drop_db().await;
}

#[tokio::test]
async fn clear_removes_the_row_and_twice_is_nothing_to_clear() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    qdrant_cmd::set_url_on(&db.store, "http://qdrant.lan:6334", None)
        .await
        .expect("set");
    let line = qdrant_cmd::clear_url_on(&db.store).await.expect("clear");
    assert_eq!(line, "Qdrant URL cleared for every box.");
    assert_eq!(row(&db).await, None);
    let line = qdrant_cmd::clear_url_on(&db.store)
        .await
        .expect("a second clear is no failure");
    assert_eq!(line, "no Qdrant URL is set; nothing was cleared");
    db.drop_db().await;
}

#[tokio::test]
async fn clear_removes_a_malformed_row() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    sqlx::query("INSERT INTO app_setting (key, value) VALUES ($1, $2)")
        .bind(QDRANT_URL_KEY)
        .bind(serde_json::json!("ftp://qdrant.lan"))
        .execute(&db.pool)
        .await
        .expect("plant a malformed row");
    let line = qdrant_cmd::clear_url_on(&db.store).await.expect("clear");
    assert_eq!(line, "Qdrant URL cleared for every box.");
    assert_eq!(row(&db).await, None);
    db.drop_db().await;
}

#[tokio::test]
async fn an_env_override_is_named_on_set() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let line = qdrant_cmd::set_url_on(
        &db.store,
        "http://qdrant.lan:6334",
        Some("http://localhost:6334"),
    )
    .await
    .expect("set");
    assert_eq!(
        line,
        "Qdrant URL set for every box: http://qdrant.lan:6334. HTUI_QDRANT_URL overrides it in \
         processes that have it."
    );
    assert_eq!(
        row(&db).await,
        Some(Value::String("http://qdrant.lan:6334".into())),
        "the row holds the URL given, never the override"
    );
    db.drop_db().await;
}

/// D8: the URL flags refuse before Postgres is contacted: an invalid URL before the keyring is
/// read, then no DSN. Needs no database: never skips.
#[tokio::test]
async fn the_url_flags_refuse_an_invalid_url_then_no_dsn_before_any_connect() {
    let _keyring = testkit::mock_keyring().await;
    let err = qdrant_cmd::set_url("http://u:SENTINEL-PW@qdrant.lan")
        .await
        .expect_err("refused");
    assert_eq!(err.code(), 2);
    assert_eq!(
        err.to_string(),
        format!(
            "not setting the Qdrant URL: {}",
            QdrantUrlError::Credentials
        )
    );
    for err in [
        qdrant_cmd::set_url("http://qdrant.lan:6334")
            .await
            .expect_err("no DSN"),
        qdrant_cmd::clear_url().await.expect_err("no DSN"),
    ] {
        assert_eq!(err.code(), 2, "{err}");
        assert_eq!(err.to_string(), NO_DSN);
    }
}

/// D8: an unreadable keyring is a refusal naming it, for the URL flags' DSN read too.
#[tokio::test]
async fn the_url_flags_refuse_a_broken_keyring() {
    let _keyring = testkit::mock_keyring_broken().await;
    for err in [
        qdrant_cmd::set_url("http://qdrant.lan:6334")
            .await
            .expect_err("refused"),
        qdrant_cmd::clear_url().await.expect_err("refused"),
    ] {
        assert_eq!(err.code(), 2, "{err}");
        assert!(err.to_string().contains(testkit::BROKEN_KEYRING), "{err}");
    }
}

#[tokio::test]
async fn set_key_stores_one_stdin_line() {
    let _keyring = testkit::mock_keyring().await;
    let mut input = std::io::Cursor::new("  qk-SENTINEL-1  \nnext\n");
    let line = qdrant_cmd::set_key(&mut input).expect("stored");
    assert_eq!(line, "Qdrant API key stored in this box's OS keyring.");
    assert!(!line.contains("SENTINEL"));
    assert_eq!(
        secret::get_qdrant_api_key().expect("read").as_deref(),
        Some("qk-SENTINEL-1"),
        "one line, trimmed"
    );
    let mut rest = String::new();
    input.read_to_string(&mut rest).expect("read");
    assert_eq!(rest, "next\n", "the rest of stdin is left unread");
    assert_eq!(
        secret::get_dsn().expect("read"),
        None,
        "the DSN entry is untouched"
    );
}

#[tokio::test]
async fn a_blank_key_line_is_refused_and_stores_nothing() {
    let _keyring = testkit::mock_keyring().await;
    secret::set_qdrant_api_key("qk-old").expect("seed");
    for input in ["\n", "   \n", ""] {
        let sentence = refused(qdrant_cmd::set_key(&mut std::io::Cursor::new(input)));
        assert_eq!(sentence, "no Qdrant API key on stdin; nothing was stored");
    }
    assert_eq!(
        secret::get_qdrant_api_key().expect("read").as_deref(),
        Some("qk-old"),
        "the stored key is kept"
    );
}

#[tokio::test]
async fn clear_key_removes_it() {
    let _keyring = testkit::mock_keyring().await;
    secret::set_qdrant_api_key("qk-old").expect("seed");
    let line = qdrant_cmd::clear_key().expect("removed");
    assert_eq!(line, "Qdrant API key removed from this box's OS keyring.");
    assert_eq!(secret::get_qdrant_api_key().expect("read"), None);
    qdrant_cmd::clear_key().expect("a missing key is no failure");
}

#[tokio::test]
async fn a_broken_keyring_is_a_refusal_naming_it() {
    let _keyring = testkit::mock_keyring_broken().await;
    let stored = refused(qdrant_cmd::set_key(&mut std::io::Cursor::new(
        "qk-SENTINEL-2\n",
    )));
    assert!(
        stored.starts_with("the Qdrant API key could not be stored in the OS keyring: "),
        "{stored}"
    );
    assert!(stored.contains(testkit::BROKEN_KEYRING), "{stored}");
    assert!(!stored.contains("SENTINEL"), "the refusal echoes the key");
    let removed = refused(qdrant_cmd::clear_key());
    assert!(
        removed.starts_with("the Qdrant API key could not be removed from the OS keyring: "),
        "{removed}"
    );
    assert!(removed.contains(testkit::BROKEN_KEYRING), "{removed}");
}
