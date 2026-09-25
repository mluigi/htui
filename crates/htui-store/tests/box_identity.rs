//! MOD-7 milestone 1, PRD D1-D5: a box is keyed on its `box.toml` id and checked by a keyed
//! machine fingerprint, never keyed on its hostname (ANA-16 C4).
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes.

use htui_store::testkit as common;

use htui_core::model::BoxId;
use htui_store::identity::Identity;

/// A hostname no other test registers, so a count by hostname sees only this test's rows.
fn unique_hostname(tag: &str) -> String {
    format!("HTUI-{tag}-{}", uuid::Uuid::now_v7().simple())
}

/// How many `box` rows carry this id.
async fn rows_with_id(pool: &sqlx::PgPool, id: BoxId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("count box rows by id")
}

/// The hazard ANA-16 C4 names: `box.toml` keeps its id across a rename, and the old
/// `UNIQUE (user_id, hostname)` upsert then inserted a second row under the same primary key.
#[tokio::test]
async fn a_hostname_change_keeps_the_box_id_and_registers() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let before = Identity {
        box_id: id,
        hostname: unique_hostname("A"),
    };
    let after = Identity {
        box_id: id,
        hostname: unique_hostname("B"),
    };

    let first = db
        .store
        .register_box(&before)
        .await
        .expect("the first registration succeeds");
    let second = db
        .store
        .register_box(&after)
        .await
        .expect("a renamed box registers under the id box.toml kept");

    assert_eq!(first, id, "a first registration keeps its id");
    assert_eq!(second, id, "a rename keeps the id");
    assert_eq!(rows_with_id(&db.pool, id).await, 1, "one row for the box");
    let hostname: String = sqlx::query_scalar("SELECT hostname FROM box WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read the hostname");
    assert_eq!(hostname, after.hostname, "the row carries the new hostname");

    db.drop_db().await;
}
