//! MOD-24 D2 against live Postgres: a worker process killed mid-walk, and the one that follows.

#![cfg(target_os = "linux")]

use htui_core::fixtures::ids;
use htui_store::{Registration, testkit};

/// MOD-24 D2's helper: a second process reaches a `demo_db` database as the fixture box, the box
/// `db.store` stands for, and a later process from the same machine is that box again.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_process_connects_as_the_fixture_box() {
    let Some(db) = testkit::demo_db().await else {
        return;
    };
    for connect in ["first", "second"] {
        let store = testkit::fixture_box_store(&db.url).await;
        assert!(
            matches!(store.registration(), Some(Registration::Known { .. })),
            "the {connect} connect is the known fixture box: {:?}",
            store.registration()
        );
        assert_eq!(
            (store.this_box(), store.this_user()),
            (ids::BOX, ids::USER),
            "the {connect} connect"
        );
        store.pool().close().await;
    }
    db.drop_db().await;
}
