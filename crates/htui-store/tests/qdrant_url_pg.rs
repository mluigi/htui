//! MOD-158 D1: the database-wide Qdrant URL, the `app_setting` row `qdrant_url`, read and written
//! by inherent compare-and-set methods on `MemStore` and `PgStore`. One list of cases runs on both.
//!
//! `pg_store_keeps_the_qdrant_url_row` creates and drops its own database; with
//! `HTUI_TEST_DATABASE_URL` unset it prints `common::SKIP` and passes.
//! `mem_store_keeps_the_qdrant_url_row` needs no server and never skips.

use htui_store::testkit as common;

use chrono::{DateTime, Utc};
use htui_core::qdrant::{QDRANT_URL_KEY, QdrantUrl, QdrantUrlError, StoredQdrantUrl};
use htui_core::store::{CasOutcome, MemStore, Result};
use htui_store::PgStore;
use serde_json::{Value, json};

/// The three inherent methods, and the two the cases need around them, over both stores.
trait UrlStore {
    async fn qdrant_url(&self) -> Result<Option<StoredQdrantUrl>>;
    async fn set_qdrant_url(
        &self,
        url: &QdrantUrl,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>>;
    async fn clear_qdrant_url(
        &self,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>>;
    /// The `qdrant_url` value as the settings chain reads it, if any.
    async fn stored_value(&self) -> Option<Value>;
    /// Writes `42` as the row's value, which no setter would accept.
    async fn plant_malformed(&self);
}

impl UrlStore for MemStore {
    async fn qdrant_url(&self) -> Result<Option<StoredQdrantUrl>> {
        MemStore::qdrant_url(self).await
    }
    async fn set_qdrant_url(
        &self,
        url: &QdrantUrl,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>> {
        MemStore::set_qdrant_url(self, url, expected).await
    }
    async fn clear_qdrant_url(
        &self,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>> {
        MemStore::clear_qdrant_url(self, expected).await
    }
    async fn stored_value(&self) -> Option<Value> {
        self.app_settings()
            .await
            .expect("app_settings")
            .remove(QDRANT_URL_KEY)
    }
    async fn plant_malformed(&self) {
        self.set_app_setting(QDRANT_URL_KEY, json!(42));
    }
}

impl UrlStore for PgStore {
    async fn qdrant_url(&self) -> Result<Option<StoredQdrantUrl>> {
        PgStore::qdrant_url(self).await
    }
    async fn set_qdrant_url(
        &self,
        url: &QdrantUrl,
        expected: Option<DateTime<Utc>>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>> {
        PgStore::set_qdrant_url(self, url, expected).await
    }
    async fn clear_qdrant_url(
        &self,
        expected: DateTime<Utc>,
    ) -> Result<CasOutcome<Option<StoredQdrantUrl>>> {
        PgStore::clear_qdrant_url(self, expected).await
    }
    async fn stored_value(&self) -> Option<Value> {
        self.app_settings()
            .await
            .expect("app_settings")
            .remove(QDRANT_URL_KEY)
    }
    async fn plant_malformed(&self) {
        // Unchecked on purpose: a `query!` here would need an offline entry of its own.
        sqlx::query("INSERT INTO app_setting (key, value) VALUES ('qdrant_url', '42'::jsonb)")
            .execute(self.pool())
            .await
            .expect("plant a malformed qdrant_url row");
    }
}

fn url(text: &str) -> QdrantUrl {
    QdrantUrl::parse(text).expect("a valid URL")
}

/// The applied row, or a panic naming what came back instead.
fn applied(outcome: CasOutcome<Option<StoredQdrantUrl>>, what: &str) -> Option<StoredQdrantUrl> {
    match outcome {
        CasOutcome::Applied(row) => row,
        CasOutcome::Stale(row) => panic!("{what}: expected Applied, got Stale({row:?})"),
    }
}

/// The current row a miss answers, or a panic naming what came back instead.
fn stale(outcome: CasOutcome<Option<StoredQdrantUrl>>, what: &str) -> Option<StoredQdrantUrl> {
    match outcome {
        CasOutcome::Stale(row) => row,
        CasOutcome::Applied(row) => panic!("{what}: expected Stale, got Applied({row:?})"),
    }
}

async fn cases(store: &impl UrlStore) {
    // 1. No row.
    assert_eq!(store.qdrant_url().await.expect("read"), None, "no row yet");

    // 2. Insert expecting none.
    let first = applied(
        store
            .set_qdrant_url(&url("http://first:6334"), None)
            .await
            .expect("insert"),
        "insert expecting none",
    )
    .expect("an applied set answers the row");
    assert_eq!(first.url, Ok(url("http://first:6334")));
    assert_eq!(
        store.qdrant_url().await.expect("read"),
        Some(first.clone()),
        "the read answers what the insert stored, token included"
    );
    assert_eq!(
        store.stored_value().await,
        Some(json!("http://first:6334")),
        "the value is a JSON string"
    );

    // 3. A second insert expecting none meets the first.
    let current = stale(
        store
            .set_qdrant_url(&url("http://second:6334"), None)
            .await
            .expect("second insert"),
        "a second insert expecting none",
    );
    assert_eq!(current, Some(first.clone()), "the row as it is");
    assert_eq!(
        store.stored_value().await,
        Some(json!("http://first:6334")),
        "unchanged"
    );

    // 4. Update by token.
    let second = applied(
        store
            .set_qdrant_url(&url("https://second/q"), Some(first.token))
            .await
            .expect("update"),
        "update by token",
    )
    .expect("an applied set answers the row");
    assert_eq!(second.url, Ok(url("https://second/q")));
    assert_ne!(second.token, first.token, "a write moves the token");

    // 5. Update with the spent token.
    let current = stale(
        store
            .set_qdrant_url(&url("http://third:6334"), Some(first.token))
            .await
            .expect("stale update"),
        "update with a spent token",
    );
    assert_eq!(current, Some(second.clone()));
    assert_eq!(store.stored_value().await, Some(json!("https://second/q")));

    // 6. Clear by token, then again with the same token.
    let cleared = applied(
        store.clear_qdrant_url(second.token).await.expect("clear"),
        "clear by token",
    );
    assert_eq!(cleared, None, "a clear leaves no row");
    assert_eq!(store.qdrant_url().await.expect("read"), None);
    assert_eq!(store.stored_value().await, None);
    let gone = stale(
        store
            .clear_qdrant_url(second.token)
            .await
            .expect("clear again"),
        "clear of a row that is gone",
    );
    assert_eq!(gone, None, "Stale(None): the row expected is gone");
    let gone = stale(
        store
            .set_qdrant_url(&url("http://x:1"), Some(second.token))
            .await
            .expect("update of a gone row"),
        "update of a row that is gone",
    );
    assert_eq!(gone, None);
    assert_eq!(store.stored_value().await, None, "nothing written");

    // 7. A malformed row reads as an error with a token, and an update by that token fixes it.
    store.plant_malformed().await;
    let malformed = store
        .qdrant_url()
        .await
        .expect("a malformed row still reads")
        .expect("the row is there");
    assert_eq!(malformed.url, Err(QdrantUrlError::NotAString));
    let err = malformed
        .usable()
        .expect_err("never \"no URL\"")
        .to_string();
    assert!(err.contains(QDRANT_URL_KEY), "{err}");
    let fixed = applied(
        store
            .set_qdrant_url(&url("http://fixed:6334"), Some(malformed.token))
            .await
            .expect("fix"),
        "update of a malformed row by its token",
    )
    .expect("the row");
    assert_eq!(fixed.url, Ok(url("http://fixed:6334")));
    assert_eq!(
        store.qdrant_url().await.expect("read"),
        Some(fixed),
        "fixed"
    );
}

#[tokio::test]
async fn mem_store_keeps_the_qdrant_url_row() {
    cases(&MemStore::default()).await;
}

#[tokio::test]
async fn pg_store_keeps_the_qdrant_url_row() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    cases(&db.store).await;
    db.drop_db().await;
}
