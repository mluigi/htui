//! MOD-122 milestone 2 (plan D1, D4, D6, D7; blueprint F2, F14): the Postgres-only half of the
//! worker log store. The rules both stores share run as `WORKER_LOG_CASES` in `pg_conformance.rs`;
//! what a generic case cannot plant (another user, a second box, a non-object blob, a deleted
//! box) is here, planted with raw SQL.
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes.

use htui_store::testkit as common;

use chrono::{TimeDelta, Utc};
use htui_core::fixtures::ids;
use htui_core::model::{BoxId, LogShipLevel, NewWorkerLog, UserId, WorkerLogLevel, WorkerLogLine};
use htui_core::store::traits::BOX_SETTINGS_NOT_AN_OBJECT;
use htui_core::store::{StoreError, WorkerLogStore as _};
use serde_json::{Value, json};
use sqlx::PgPool;

fn line(level: WorkerLogLevel, text: &str) -> NewWorkerLog {
    NewWorkerLog {
        logged_at: Utc::now(),
        level,
        target: "htui::worker".to_owned(),
        text: text.to_owned(),
    }
}

/// Plants a box owned by `user` under `hostname`.
async fn plant_box(pool: &PgPool, user: UserId, hostname: &str) -> BoxId {
    let id = BoxId::new();
    sqlx::query(
        "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
         VALUES ($1, $2, $3, 'linux', '', 'x86_64', '0.0.0')",
    )
    .bind(id.as_uuid())
    .bind(user.as_uuid())
    .bind(hostname)
    .execute(pool)
    .await
    .expect("plant a box");
    id
}

/// Plants an `app_user` that is not this store's.
async fn plant_stranger(pool: &PgPool) -> UserId {
    let stranger = UserId::new();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
        .bind(stranger.as_uuid())
        .bind(format!("stranger-{}", uuid::Uuid::now_v7().simple()))
        .execute(pool)
        .await
        .expect("plant another user");
    stranger
}

/// `(box_id, box_name, level, text)` of every row, any user's, in id order.
async fn every_row(pool: &PgPool) -> Vec<(Option<uuid::Uuid>, String, String, String)> {
    sqlx::query_as("SELECT box_id, box_name, level, text FROM worker_log ORDER BY id")
        .fetch_all(pool)
        .await
        .expect("read worker_log")
}

fn texts(lines: &[WorkerLogLine]) -> Vec<&str> {
    lines.iter().map(|line| line.text.as_str()).collect()
}

/// Plan D6: deleting a box deletes its non-error rows (the `BEFORE DELETE` trigger), and the
/// foreign key nulls `box_id` on its error rows, which keep `box_name`. The cap then skips them:
/// a removed box's errors stay until the age cut (blueprint F14). Another box's rows are untouched.
#[tokio::test]
async fn a_box_delete_keeps_only_its_error_rows() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let doomed = plant_box(&db.pool, db.store.this_user(), "DOOMED").await;
    db.store
        .append_worker_logs(
            doomed,
            &[
                line(WorkerLogLevel::Error, "e-1"),
                line(WorkerLogLevel::Warn, "w-1"),
                line(WorkerLogLevel::Info, "i-1"),
                line(WorkerLogLevel::Error, "e-2"),
                line(WorkerLogLevel::Debug, "d-1"),
            ],
        )
        .await
        .expect("the planted box takes the batch");
    db.store
        .append_worker_logs(ids::BOX, &[line(WorkerLogLevel::Info, "kept")])
        .await
        .expect("the fixture box takes a line");

    sqlx::query("DELETE FROM box WHERE id = $1")
        .bind(doomed.as_uuid())
        .execute(&db.pool)
        .await
        .expect("a box with log rows can be deleted");

    let fixture = Some(ids::BOX.as_uuid());
    assert_eq!(
        every_row(&db.pool).await,
        [
            (
                None,
                "DOOMED".to_owned(),
                "error".to_owned(),
                "e-1".to_owned()
            ),
            (
                None,
                "DOOMED".to_owned(),
                "error".to_owned(),
                "e-2".to_owned()
            ),
            (
                fixture,
                "DESKTOP-HTUI".to_owned(),
                "info".to_owned(),
                "kept".to_owned()
            ),
        ],
        "the deleted box's errors stay, attributable by name; its other rows go"
    );
    let read = db.store.worker_logs(None, 100).await.expect("a read");
    assert_eq!(texts(&read), ["e-1", "e-2", "kept"]);
    assert_eq!(read[0].box_id, None);

    let far_past = Utc::now() - TimeDelta::days(3650);
    assert_eq!(
        db.store
            .prune_worker_logs(far_past, 1)
            .await
            .expect("a sweep"),
        0,
        "the cap partitions by box and skips the removed box's rows"
    );
    assert_eq!(
        db.store
            .prune_worker_logs(Utc::now() + TimeDelta::minutes(1), 100)
            .await
            .expect("a sweep"),
        3,
        "the age cut reaches every row, the removed box's errors included"
    );

    db.drop_db().await;
}

/// `chk_worker_log_level` refuses a level outside the five.
#[tokio::test]
async fn the_check_refuses_an_unknown_level() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let refused = sqlx::query(
        "INSERT INTO worker_log (user_id, box_id, box_name, logged_at, level, target, text) \
         VALUES ($1, $2, 'x', now(), $3, 't', 'x')",
    )
    .bind(db.store.this_user().as_uuid())
    .bind(ids::BOX.as_uuid())
    .bind("fatal")
    .execute(&db.pool)
    .await
    .expect_err("`fatal` is not a level");
    let constraint = refused
        .as_database_error()
        .and_then(|error| error.constraint().map(str::to_owned));
    assert_eq!(
        constraint.as_deref(),
        Some("chk_worker_log_level"),
        "{refused}"
    );
    for level in WorkerLogLevel::ALL {
        sqlx::query(
            "INSERT INTO worker_log (user_id, box_id, box_name, logged_at, level, target, text) \
             VALUES ($1, $2, 'x', now(), $3, 't', 'x')",
        )
        .bind(db.store.this_user().as_uuid())
        .bind(ids::BOX.as_uuid())
        .bind(level.as_str())
        .execute(&db.pool)
        .await
        .unwrap_or_else(|error| panic!("`{level}` is a level: {error}"));
    }

    db.drop_db().await;
}

/// Plan D1 (blueprint F2): another user's box is `NotFound` on append and `false` on the level,
/// its blob untouched; a read returns only this user's rows. The memory half is `htui-core`'s
/// `store::mem` test `worker_log_another_user_s_box_is_refused_and_unread_in_memory`.
#[tokio::test]
async fn another_users_box_is_refused_and_unread() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let stranger = plant_stranger(&db.pool).await;
    let foreign = plant_box(&db.pool, stranger, "ELSEWHERE").await;

    let answer = db
        .store
        .append_worker_logs(foreign, &[line(WorkerLogLevel::Error, "theirs")])
        .await;
    assert!(
        matches!(answer, Err(StoreError::NotFound { entity: "box", .. })),
        "another user's box is NotFound, got {answer:?}"
    );
    assert!(
        !db.store
            .set_box_log_level(foreign, Some(LogShipLevel::Debug))
            .await
            .expect("the level is answered"),
        "another user's box is `false`"
    );
    let blob: Value = sqlx::query_scalar("SELECT settings FROM box WHERE id = $1")
        .bind(foreign.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read their blob");
    assert_eq!(blob, json!({}), "their blob is untouched");

    sqlx::query(
        "INSERT INTO worker_log (user_id, box_id, box_name, logged_at, level, target, text) \
         VALUES ($1, $2, 'ELSEWHERE', now(), 'error', 't', 'theirs')",
    )
    .bind(stranger.as_uuid())
    .bind(foreign.as_uuid())
    .execute(&db.pool)
    .await
    .expect("plant their row");
    db.store
        .append_worker_logs(ids::BOX, &[line(WorkerLogLevel::Info, "mine")])
        .await
        .expect("this user's box takes the line");
    for after in [None, Some(0)] {
        let read = db.store.worker_logs(after, 100).await.expect("a read");
        assert_eq!(texts(&read), ["mine"], "only this user's rows ({after:?})");
    }

    db.drop_db().await;
}

/// Plan D4 (blueprint F2): the cap is per box, so one box over it leaves another box's rows
/// alone, and each row carries its own box's hostname.
#[tokio::test]
async fn the_cap_leaves_another_box_alone() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let second = plant_box(&db.pool, db.store.this_user(), "SECOND").await;
    let four: Vec<_> = (1..=4)
        .map(|n| line(WorkerLogLevel::Info, &format!("a-{n}")))
        .collect();
    db.store
        .append_worker_logs(ids::BOX, &four)
        .await
        .expect("the fixture box takes four");
    db.store
        .append_worker_logs(
            second,
            &[
                line(WorkerLogLevel::Info, "b-1"),
                line(WorkerLogLevel::Info, "b-2"),
            ],
        )
        .await
        .expect("the second box takes two");
    let far_past = Utc::now() - TimeDelta::days(3650);
    assert_eq!(
        db.store
            .prune_worker_logs(far_past, 3)
            .await
            .expect("a sweep"),
        1,
        "only the fixture box is over the cap"
    );
    let read = db.store.worker_logs(None, 100).await.expect("a read");
    let rows: Vec<(&str, &str)> = read
        .iter()
        .map(|line| (line.box_name.as_str(), line.text.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            ("DESKTOP-HTUI", "a-2"),
            ("DESKTOP-HTUI", "a-3"),
            ("DESKTOP-HTUI", "a-4"),
            ("SECOND", "b-1"),
            ("SECOND", "b-2"),
        ]
    );

    db.drop_db().await;
}

/// Plan D7: a `box.settings` blob that is not an object takes no level, set or cleared:
/// `Constraint(BOX_SETTINGS_NOT_AN_OBJECT)`, and the row is untouched.
#[tokio::test]
async fn a_non_object_blob_takes_no_level() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    for blob in [json!([]), json!("x"), Value::Null] {
        sqlx::query("UPDATE box SET settings = $2 WHERE id = $1")
            .bind(ids::BOX.as_uuid())
            .bind(&blob)
            .execute(&db.pool)
            .await
            .expect("plant a non-object blob");
        let before = db.store.box_row(ids::BOX).await.expect("read the box");
        for level in [Some(LogShipLevel::Debug), None] {
            match db.store.set_box_log_level(ids::BOX, level).await {
                Err(StoreError::Constraint(said)) => {
                    assert_eq!(said, BOX_SETTINGS_NOT_AN_OBJECT, "{blob}");
                }
                other => panic!("{blob}: a non-object blob is a Constraint, got {other:?}"),
            }
        }
        assert_eq!(
            db.store.box_row(ids::BOX).await.expect("read the box"),
            before,
            "{blob}: the refusal wrote nothing"
        );
    }

    db.drop_db().await;
}

/// Plan D2: one statement writes at most 500 rows, so a larger batch loops; every row lands, in
/// order, under rising ids.
#[tokio::test]
async fn a_large_batch_is_written_whole_and_in_order() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let batch: Vec<_> = (0..1_201)
        .map(|n| line(WorkerLogLevel::Info, &format!("{n:04}")))
        .collect();
    assert_eq!(
        db.store
            .append_worker_logs(ids::BOX, &batch)
            .await
            .expect("the batch is written"),
        1_201
    );
    let read = db.store.worker_logs(None, 2_000).await.expect("a read");
    let want: Vec<String> = (0..1_201).map(|n| format!("{n:04}")).collect();
    assert_eq!(texts(&read), want);
    assert!(read.windows(2).all(|pair| pair[0].id < pair[1].id));

    db.drop_db().await;
}
