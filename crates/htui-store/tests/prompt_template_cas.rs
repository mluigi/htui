//! MOD-9 D3: two saves at one head write one row, on a real server.
//!
//! The conformance cases show the compare-and-set one call at a time, which is all `MemStore` can
//! show: its write lock serialises every append. On Postgres the two saves are two sessions, and
//! the one-statement `INSERT … SELECT … WHERE head IS NOT DISTINCT FROM $6 ON CONFLICT DO NOTHING`
//! is what has to decide between them (blueprint D16, D31): the second blocks on the unique index,
//! then inserts nothing, and the head read after it answers `Stale` with the winner's row.
//!
//! Two saves started together rarely overlap, so the winner here is a transaction held open by
//! hand: it inserts v2 and does not commit, the save at head v1 runs into its index entry and
//! waits, and only the commit lets it finish. The insert is `sqlx::query`, unchecked, so the file
//! adds nothing to `.sqlx` (D31).
//!
//! With `HTUI_TEST_DATABASE_URL` unset the case prints `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use std::time::{Duration, Instant};

use htui_store::testkit as common;

use htui_core::fixtures::ids;
use htui_core::model::{NewPromptTemplate, PromptTemplateId};
use htui_core::store::{CasOutcome, WriteStore as _};

/// A save of `implement` in the fixture's htui project, with a fresh id.
fn save(body: &str) -> NewPromptTemplate {
    NewPromptTemplate {
        id: PromptTemplateId::new(),
        project_id: ids::PROJECT_HTUI,
        name: "implement".to_owned(),
        body: body.to_owned(),
        created_by: ids::USER,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_saves_at_one_head_write_one_row() {
    let Some(db) = common::demo_db().await else {
        return; // `common` printed the skip line already (plan D13).
    };
    const A: &str = "A {{item}}\n";
    const B: &str = "B {{item}}\n";

    // The winner: v2 inserted on a second connection, not committed yet.
    let winner_id = PromptTemplateId::new();
    let mut winner = db
        .pool
        .begin()
        .await
        .expect("open the winner's transaction");
    sqlx::query(
        "INSERT INTO prompt_template (id, project_id, name, version, body, created_by) \
         VALUES ($1, $2, 'implement', 2, $3, $4)",
    )
    .bind(winner_id.as_uuid())
    .bind(ids::PROJECT_HTUI.as_uuid())
    .bind(A)
    .bind(ids::USER.as_uuid())
    .execute(&mut *winner)
    .await
    .expect("the winner's uncommitted v2");

    // The loser: a save at head v1, which cannot see the uncommitted row and so aims at v2 too.
    let store = db.store.clone();
    let mut loser =
        tokio::spawn(async move { store.append_prompt_template(save(B), Some(1)).await });

    // It blocks on the unique index: a backend of this database waits on a lock, and the save has
    // not answered.
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
            "the save never waited on the winner's row"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut loser)
            .await
            .is_err(),
        "the save answered while the winner's v2 was uncommitted"
    );

    winner.commit().await.expect("commit the winner");
    let outcome = tokio::time::timeout(Duration::from_secs(10), loser)
        .await
        .expect("the save answers once the winner commits")
        .expect("the save's task")
        .expect("save B");
    let CasOutcome::Stale(head) = outcome else {
        panic!("the save that lost is stale, got {outcome:?}");
    };
    assert_eq!(
        (head.id, head.version, head.body.as_str()),
        (winner_id, 2, A),
        "the Stale row is the winner's committed v2"
    );

    let implement: Vec<(i32, String)> = db
        .store
        .prompt_templates(ids::PROJECT_HTUI)
        .await
        .expect("read the project's templates")
        .into_iter()
        .filter(|row| row.name == "implement")
        .map(|row| (row.version, row.body))
        .collect();
    assert_eq!(
        implement
            .iter()
            .map(|(version, _)| *version)
            .collect::<Vec<_>>(),
        [1, 2],
        "one row was written: v1 and v2, nothing else"
    );
    assert_eq!(implement[1].1, A, "v2's body is the winner's");

    db.drop_db().await;
}
