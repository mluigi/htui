//! MOD-45 T4 against live Postgres (blueprint §5.7, plan D311): the production `PgVerifier`.
//!
//! The verifier connects with a "local" config root, as `htui provision` does with this machine's.
//! The "remote" box registers through `worker_cmd::connect` with a root of its own, as the new
//! `htui worker` on the provisioned host would. Every root is a temporary directory under
//! `CARGO_TARGET_TMPDIR`, never `~/.config/htui`.
//!
//! Each case prints `testkit::SKIP` and returns with `HTUI_TEST_DATABASE_URL` unset, and panics
//! instead when `CI` is set, like every other Postgres-backed suite. Each ends with
//! `db.drop_db()`.

#![cfg(feature = "testkit")]

use std::time::Duration;

use htui::provision::verify::{PgVerifier, Poll, Verifier as _};
use htui::worker_cmd;
use htui_core::model::{BoxId, BoxRow, Executor};
use htui_core::store::WriteStore as _;
use htui_store::pg::PoolSize;
use htui_store::{PgStore, testkit};

/// The poll for a box that should be seen.
const SEEN: Poll = Poll {
    interval: Duration::from_millis(20),
    deadline: Duration::from_secs(10),
};

/// The poll for a box that should not be.
const NOT_SEEN: Poll = Poll {
    interval: Duration::from_millis(20),
    deadline: Duration::from_millis(300),
};

/// A throwaway config root under `CARGO_TARGET_TMPDIR`.
fn temp_root(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("a config root under CARGO_TARGET_TMPDIR")
}

/// The "remote" worker's connect: registers a box of its own under `root`.
async fn register_remote(url: &str, root: &tempfile::TempDir) -> PgStore {
    worker_cmd::connect(url, root.path(), PoolSize::clamped(2))
        .await
        .expect("the remote worker connects")
}

/// `id`'s row, as `boxes()` lists it.
async fn row_of(store: &PgStore, id: BoxId) -> BoxRow {
    store
        .boxes()
        .await
        .expect("list the boxes")
        .into_iter()
        .find(|record| record.row.id == id)
        .expect("the box is listed")
        .row
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_registered_after_the_baseline_is_seen() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let local = temp_root("provision-local-");
    let remote_root = temp_root("provision-remote-");
    let verifier = PgVerifier::new(local.path().to_path_buf());
    let baseline = verifier.baseline(&db.url).await.expect("a baseline");

    let remote = register_remote(&db.url, &remote_root).await;
    let id = remote.this_box();
    assert!(!baseline.contains_key(&id), "the remote box is new");
    assert_eq!(verifier.box_seen(id, &baseline, SEEN).await, Ok(()));

    drop(remote);
    drop(verifier);
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_box_in_the_baseline_is_not_seen() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let local = temp_root("provision-local-");
    let remote_root = temp_root("provision-remote-");
    let remote = register_remote(&db.url, &remote_root).await;
    let id = remote.this_box();

    let verifier = PgVerifier::new(local.path().to_path_buf());
    let baseline = verifier.baseline(&db.url).await.expect("a baseline");
    assert!(
        baseline.contains_key(&id),
        "the remote box is in the baseline"
    );
    let answer = verifier.box_seen(id, &baseline, NOT_SEEN).await;
    let reason = answer.expect_err("an unchanged box is not seen");
    assert!(reason.contains("did not check in"), "{reason}");

    drop(remote);
    drop(verifier);
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_after_the_baseline_is_seen() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let local = temp_root("provision-local-");
    let remote_root = temp_root("provision-remote-");
    let remote = register_remote(&db.url, &remote_root).await;
    let id = remote.this_box();

    let verifier = PgVerifier::new(local.path().to_path_buf());
    let baseline = verifier.baseline(&db.url).await.expect("a baseline");
    assert!(
        baseline.contains_key(&id),
        "the remote box is in the baseline"
    );
    assert!(
        db.store.touch_box(id).await.expect("touch the box"),
        "the touch found the row"
    );
    assert_eq!(verifier.box_seen(id, &baseline, SEEN).await, Ok(()));

    drop(remote);
    drop(verifier);
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn set_executor_writes_worker_and_keeps_every_other_key() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    let local = temp_root("provision-local-");
    let remote_root = temp_root("provision-remote-");
    let remote = register_remote(&db.url, &remote_root).await;
    let id = remote.this_box();
    sqlx::query(
        r#"UPDATE box SET settings = settings || '{"max_concurrent_items": 3}' WHERE id = $1"#,
    )
    .bind(id.as_uuid())
    .execute(&db.pool)
    .await
    .expect("seed another settings key");

    let verifier = PgVerifier::new(local.path().to_path_buf());
    verifier.baseline(&db.url).await.expect("a baseline");
    let before = row_of(&db.store, id).await;
    assert!(
        !matches!(Executor::of(&before.settings), Executor::Worker),
        "the remote box does not start as a worker box"
    );
    assert_eq!(
        verifier.set_executor(id).await,
        Ok(true),
        "the first call writes"
    );

    let after = row_of(&db.store, id).await;
    assert!(
        matches!(Executor::of(&after.settings), Executor::Worker),
        "executor is worker: {}",
        after.settings
    );
    assert_eq!(after.settings["max_concurrent_items"], serde_json::json!(3));
    assert!(
        after.edit_version > before.edit_version,
        "the write bumped edit_version"
    );

    // E-23: already `worker`, so nothing is written again.
    assert_eq!(
        verifier.set_executor(id).await,
        Ok(false),
        "the second call finds worker"
    );
    let again = row_of(&db.store, id).await;
    assert_eq!(again.edit_version, after.edit_version);
    assert_eq!(again.settings, after.settings);

    drop(remote);
    drop(verifier);
    db.drop_db().await;
}
