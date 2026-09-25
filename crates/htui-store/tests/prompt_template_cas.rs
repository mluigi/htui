//! MOD-9 D3: two saves at one head write one row, on a real server.
//!
//! The conformance cases show the compare-and-set one call at a time, which is all `MemStore` can
//! show: its write lock serialises every append. On Postgres the two saves are two sessions, and
//! the one-statement `INSERT … SELECT … WHERE head IS NOT DISTINCT FROM $6 ON CONFLICT DO NOTHING`
//! is what has to decide between them (blueprint D16, D31): the second blocks on the unique index,
//! then inserts nothing, and the head read after it answers `Stale` with the winner's row.
//!
//! With `HTUI_TEST_DATABASE_URL` unset the case prints `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

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

    let (a, b) = tokio::join!(
        db.store.append_prompt_template(save(A), Some(1)),
        db.store.append_prompt_template(save(B), Some(1)),
    );
    let (a, b) = (a.expect("save A"), b.expect("save B"));

    let (winner, loser) = match (a, b) {
        (CasOutcome::Applied(winner), CasOutcome::Stale(loser))
        | (CasOutcome::Stale(loser), CasOutcome::Applied(winner)) => (winner, loser),
        other => panic!("exactly one save applies and one is stale, got {other:?}"),
    };
    assert_eq!(winner.version, 2, "the winner appends v2");
    assert_eq!(
        loser, winner,
        "the loser's Stale row is the winner's v2, as the head read found it"
    );
    assert!(
        winner.body == A || winner.body == B,
        "the winner carries one of the two bodies"
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
    assert_eq!(
        implement[1].1, winner.body,
        "v2's body is the Applied save's"
    );

    db.drop_db().await;
}
