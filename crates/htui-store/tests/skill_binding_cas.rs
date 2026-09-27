//! MOD-9 D76, D78, R-32: two writers at one attachment key, and an unbind under a spent token,
//! on a real server.
//!
//! The conformance cases show each compare-and-set one call at a time, which is all `MemStore`
//! can show: its write lock serialises every write. On Postgres the two writers are two sessions,
//! and the one-statement `INSERT … SELECT … WHERE <the row's `updated_at`> IS NOT DISTINCT FROM
//! $10 ON CONFLICT (skill_id, project_id, phase_id) DO UPDATE … WHERE skill_binding.updated_at =
//! $10` is what has to decide between them (blueprint §2.5): the second blocks on the
//! `UNIQUE NULLS NOT DISTINCT` index, the `DO UPDATE`'s own `WHERE` then matches no row, and the
//! re-read after it answers `Stale` with the winner's row.
//!
//! The loser that blocks is rare enough that the winner is a transaction held open by hand, as
//! `prompt_template_cas.rs` does: it writes the row and does not commit, the loser's write runs
//! into its index entry and waits, and only the commit lets it finish. The second case is R-32 —
//! an unbind that loses the race must answer `Stale` and leave the winner's row standing, which is
//! the whole reason `remove_skill_binding` is a compare-and-set and not a bare delete.
//!
//! Every statement here is `sqlx::query`, unchecked, so the file adds nothing to `.sqlx` (D31).
//! With `HTUI_TEST_DATABASE_URL` unset each case prints `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use std::time::{Duration, Instant};

use htui_store::testkit as common;

use chrono::{DateTime, Utc};
use htui_core::fixtures::ids;
use htui_core::model::{Activation, NewSkillBinding, ProjectId, SkillBindingId};
use htui_core::store::{CasOutcome, WriteStore as _};

/// An attachment of the fixture's `tests` skill at one level, with a fresh id.
fn binding(project: Option<ProjectId>, position: i32) -> NewSkillBinding {
    NewSkillBinding {
        id: SkillBindingId::new(),
        skill_id: ids::SKILL_TESTS,
        project_id: project,
        phase_id: None,
        pinned_version: None,
        position,
        activation: Activation::Always,
        globs: Vec::new(),
        languages: Vec::new(),
    }
}

/// How long a case waits for the loser's statement to reach its `Lock` wait before giving up.
const PATIENCE: Duration = Duration::from_secs(10);

/// Waits until some backend of this database is waiting on a lock, and returns.
///
/// `pg_stat_activity` is the only evidence available from outside the loser's session, and it is
/// the evidence that matters: without it a test that passed could have passed because the loser
/// simply finished first.
async fn wait_on_a_lock(pool: &sqlx::PgPool) {
    let started = Instant::now();
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("read pg_stat_activity");
        if waiting > 0 {
            return;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "the loser's write never waited on the winner's row"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Asserts the loser is still blocked, which is what makes the commit below the thing that
/// releases it rather than a race the test won by luck.
async fn assert_still_blocked<T>(handle: &mut tokio::task::JoinHandle<T>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut *handle)
            .await
            .is_err(),
        "the loser's write answered while the winner's row was uncommitted"
    );
}

/// MOD-9 D76: the same `(skill, NULL, NULL)` written twice is one row. The winner's insert is
/// uncommitted, so the loser's guard reads no row and its insert aims at the same unique index
/// entry; the index blocks it, the `DO UPDATE`'s `WHERE updated_at = NULL` matches nothing, and
/// the loser is `Stale` with the winner's row rather than a second attachment.
#[tokio::test(flavor = "multi_thread")]
async fn two_bindings_at_one_key_write_one_row() {
    let Some(db) = common::demo_db().await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let winner_id = SkillBindingId::new();
    let mut winner = db
        .pool
        .begin()
        .await
        .expect("open the winner's transaction");
    sqlx::query(
        "INSERT INTO skill_binding (id, skill_id, project_id, phase_id, position) \
         VALUES ($1, $2, NULL, NULL, 4)",
    )
    .bind(winner_id.as_uuid())
    .bind(ids::SKILL_TESTS.as_uuid())
    .execute(&mut *winner)
    .await
    .expect("the winner's uncommitted global attachment");

    // The loser: a create (`expected: None`) of the same key, which cannot see the uncommitted
    // row and so believes the key is free.
    let store = db.store.clone();
    let mut loser =
        tokio::spawn(async move { store.set_skill_binding(binding(None, 9), None).await });

    wait_on_a_lock(&db.pool).await;
    assert_still_blocked(&mut loser).await;

    winner.commit().await.expect("commit the winner");
    let outcome = tokio::time::timeout(PATIENCE, loser)
        .await
        .expect("the loser answers once the winner commits")
        .expect("the loser's task")
        .expect("set_skill_binding");
    let CasOutcome::Stale(row) = outcome else {
        panic!("the write that lost the key is stale, got {outcome:?}");
    };
    assert_eq!(
        (row.id, row.project_id, row.phase_id, row.position),
        (winner_id, None, None, 4),
        "the Stale row is the winner's committed attachment"
    );

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM skill_binding WHERE skill_id = $1 AND project_id IS NULL AND phase_id IS NULL")
            .bind(ids::SKILL_TESTS.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("count the global attachments of the skill");
    assert_eq!(rows, 1, "UNIQUE NULLS NOT DISTINCT wrote one row, not two");

    db.drop_db().await;
}

/// R-32: a second unbind under a spent token deletes nothing. The winner updates the row in a
/// transaction it holds open, the `updated_at` trigger moves the token, and the loser's
/// `DELETE … WHERE id = $1 AND updated_at = $2` re-checks the row it waited for, finds the
/// winner's value, and answers `Stale` with the row as it now stands.
#[tokio::test(flavor = "multi_thread")]
async fn an_unbind_under_a_spent_token_keeps_the_row() {
    let Some(db) = common::demo_db().await else {
        return; // `common` printed the skip line already (plan D13).
    };
    let CasOutcome::Applied(row) = db
        .store
        .set_skill_binding(binding(Some(ids::PROJECT_AGY), 0), None)
        .await
        .expect("the attachment lands")
    else {
        panic!("a create under None is Applied");
    };
    let spent = row.updated_at;

    // The winner: a move of `position`, which the `updated_at` trigger turns into a new token.
    let mut winner = db
        .pool
        .begin()
        .await
        .expect("open the winner's transaction");
    sqlx::query("UPDATE skill_binding SET position = 6 WHERE id = $1")
        .bind(row.id.as_uuid())
        .execute(&mut *winner)
        .await
        .expect("the winner's uncommitted move");

    let store = db.store.clone();
    let id = row.id;
    let mut loser = tokio::spawn(async move { store.remove_skill_binding(id, spent).await });

    wait_on_a_lock(&db.pool).await;
    assert_still_blocked(&mut loser).await;

    winner.commit().await.expect("commit the winner");
    let outcome = tokio::time::timeout(PATIENCE, loser)
        .await
        .expect("the unbind answers once the winner commits")
        .expect("the loser's task")
        .expect("remove_skill_binding");
    let CasOutcome::Stale(found) = outcome else {
        panic!("an unbind under a spent token is stale, got {outcome:?}");
    };
    assert_eq!(
        (found.id, found.position),
        (row.id, 6),
        "the Stale row is the winner's committed attachment"
    );

    let (rows, position): (i64, i32) =
        sqlx::query_as("SELECT count(*), max(position) FROM skill_binding WHERE id = $1")
            .bind(row.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read the attachment back");
    assert_eq!(
        (rows, position),
        (1, 6),
        "the row is still there, and it is the winner's row, not the loser's delete"
    );

    // The token the winner left is the one a fresh unbind detaches, so the writer is not stuck
    // on a row no token can name.
    let (after, _position): (DateTime<Utc>, i32) =
        sqlx::query_as("SELECT updated_at, position FROM skill_binding WHERE id = $1")
            .bind(row.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read the attachment's token");
    assert!(
        after > spent,
        "the trigger moved the token ({spent} -> {after})"
    );
    let CasOutcome::Applied(detached) = db
        .store
        .remove_skill_binding(row.id, after)
        .await
        .expect("the unbind at the current token lands")
    else {
        panic!("an unbind at the current token is Applied");
    };
    assert_eq!(detached.id, row.id, "and it is the same row");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM skill_binding WHERE id = $1")
        .bind(row.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count the attachments");
    assert_eq!(
        rows, 0,
        "the row really goes once the token is the current one"
    );
}
