//! MOD-9 milestone 3 (plan D75, D78): two skill writes racing on one token write one row, on a
//! real server.
//!
//! The conformance cases show each compare-and-set one call at a time, which is all `MemStore`
//! can show: its write lock serialises every call. On Postgres the two writes are two sessions,
//! and a single statement has to decide between them (blueprint D92, §0.3's probes):
//!
//! - `add_skill_version` is `INSERT … SELECT … WHERE (SELECT COALESCE(max(version), 0) …) =
//!   $expected ON CONFLICT (skill_id, version) DO NOTHING`. The loser cannot see the winner's
//!   uncommitted version, so it aims at the same number, blocks on the primary key, then inserts
//!   nothing; the head read after it answers `Stale` with the winner's version.
//! - `set_skill_binding` with `expected: None` is `INSERT … ON CONFLICT (skill_id, project_id,
//!   phase_id) DO NOTHING`, which infers the `UNIQUE NULLS NOT DISTINCT` key, so two global
//!   attaches of one skill collide too: the second blocks on the first's uncommitted row (probed:
//!   2.0 s against a 3 s hold), inserts nothing, and the key re-read answers `Stale` with the
//!   winner's row.
//!
//! Two writes started together rarely overlap, so the winner here is a transaction held open by
//! hand, as in `prompt_template_cas.rs`. The inserts are `sqlx::query`, unchecked, so the file adds
//! nothing to `.sqlx`.
//!
//! With `HTUI_TEST_DATABASE_URL` unset each case prints `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use std::time::{Duration, Instant};

use htui_store::testkit as common;

use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, Attachment, BindingChange, NewSkillVersion, SkillBindingId, SkillBindingKey,
};
use htui_core::store::{CasOutcome, WriteStore as _};
use serde_json::json;

/// An append authored by the fixture user, with a `{}` source.
fn version(body: &str) -> NewSkillVersion {
    NewSkillVersion {
        body: body.to_owned(),
        source: json!({}),
        created_by: ids::USER,
    }
}

/// Waits until a backend of this database waits on a lock, then checks the loser has not
/// answered: the proof that it is blocked on the winner's row and not merely slow.
async fn wait_for_the_lock<T>(db: &common::TestDb, loser: &mut tokio::task::JoinHandle<T>) {
    let started = Instant::now();
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("read pg_stat_activity");
        if waiting > 0 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the write never waited on the winner's row"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut *loser)
            .await
            .is_err(),
        "the write answered while the winner's row was uncommitted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_appends_at_one_head_write_one_version() {
    let Some(db) = common::demo_db().await else {
        return; // `common` printed the skip line already (plan D13).
    };

    // The winner: v3 of `rust-style` inserted on a second connection, not committed yet.
    let mut winner = db
        .pool
        .begin()
        .await
        .expect("open the winner's transaction");
    sqlx::query(
        "INSERT INTO skill_version (skill_id, version, body, source, created_by) \
         VALUES ($1, 3, 'A', '{}'::jsonb, $2)",
    )
    .bind(ids::SKILL_RUST_STYLE.as_uuid())
    .bind(ids::USER.as_uuid())
    .execute(&mut *winner)
    .await
    .expect("the winner's uncommitted v3");

    // The loser: an append at head v2, which cannot see the uncommitted row and so aims at v3 too.
    let store = db.store.clone();
    let mut loser = tokio::spawn(async move {
        store
            .add_skill_version(ids::SKILL_RUST_STYLE, 2, version("B"))
            .await
    });
    wait_for_the_lock(&db, &mut loser).await;

    winner.commit().await.expect("commit the winner");
    let outcome = tokio::time::timeout(Duration::from_secs(10), loser)
        .await
        .expect("the append answers once the winner commits")
        .expect("the append's task")
        .expect("append B");
    let CasOutcome::Stale(head) = outcome else {
        panic!("the append that lost is stale, got {outcome:?}");
    };
    assert_eq!(
        (head.version, head.body.as_str()),
        (3, "A"),
        "the Stale row is the winner's committed v3"
    );

    let versions = db
        .store
        .skill_versions(ids::SKILL_RUST_STYLE)
        .await
        .expect("read rust-style's versions");
    assert_eq!(
        versions.iter().map(|row| row.version).collect::<Vec<_>>(),
        [1, 2, 3],
        "one version was written: v3, nothing after it"
    );
    assert_eq!(versions[2].body, "A", "v3's body is the winner's");

    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_attaches_of_one_key_write_one_row() {
    let Some(db) = common::demo_db().await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let key = SkillBindingKey {
        skill: ids::SKILL_TESTS,
        project: None,
        phase: None,
    };

    // The winner: a global attachment of `tests`, not committed yet.
    let winner_id = SkillBindingId::new();
    let mut winner = db
        .pool
        .begin()
        .await
        .expect("open the winner's transaction");
    sqlx::query(
        "INSERT INTO skill_binding (id, skill_id, project_id, phase_id, position) \
         VALUES ($1, $2, NULL, NULL, 7)",
    )
    .bind(winner_id.as_uuid())
    .bind(ids::SKILL_TESTS.as_uuid())
    .execute(&mut *winner)
    .await
    .expect("the winner's uncommitted row");

    // The loser: an attach under "I expect no row", which cannot see the uncommitted row, so its
    // key read passes and its insert runs into the winner's index entry.
    let store = db.store.clone();
    let mut loser = tokio::spawn(async move {
        store
            .set_skill_binding(
                key,
                None,
                BindingChange::Attach(Attachment {
                    pinned_version: None,
                    position: 9,
                    activation: Activation::Always,
                    globs: Vec::new(),
                    languages: Vec::new(),
                }),
            )
            .await
    });
    wait_for_the_lock(&db, &mut loser).await;

    winner.commit().await.expect("commit the winner");
    let outcome = tokio::time::timeout(Duration::from_secs(10), loser)
        .await
        .expect("the attach answers once the winner commits")
        .expect("the attach's task")
        .expect("attach at 9");
    let CasOutcome::Stale(Some(row)) = outcome else {
        panic!("the attach that lost is stale with the winner's row, got {outcome:?}");
    };
    assert_eq!(
        (row.id, row.position),
        (winner_id, 7),
        "the Stale row is the winner's committed row"
    );

    let global = db
        .store
        .skill_bindings(None)
        .await
        .expect("read the global attachments");
    assert_eq!(global, [row], "one row was written: the winner's");

    db.drop_db().await;
}
