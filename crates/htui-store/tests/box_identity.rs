//! MOD-7 milestone 1, PRD D1-D5: a box is keyed on its `box.toml` id and checked by a keyed
//! machine fingerprint, never keyed on its hostname (ANA-16 C4).
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes. The fingerprints are built from synthetic identities: no case
//! asserts anything over this machine's own value.

use htui_store::testkit as common;

use htui_core::model::{BoxId, UserId};
use htui_store::identity::Identity;
use htui_store::{Fingerprint, Registration};
use sqlx::PgPool;

/// A synthetic machine identity, and the HMAC-SHA256 of `htui/box-fingerprint/v1` under it
/// (computed outside this crate).
const RAW: &str = "0123456789abcdef0123456789abcdef";
const RAW_FINGERPRINT: &str = "a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437dae364df";

/// The fingerprint of a synthetic identity.
fn fp(raw: &str) -> Fingerprint {
    Fingerprint::from_machine_identity(raw).expect("a synthetic identity has a fingerprint")
}

/// Machine one and machine two.
fn fp1() -> Fingerprint {
    fp("11111111111111111111111111111111")
}

fn fp2() -> Fingerprint {
    fp("22222222222222222222222222222222")
}

/// A hostname no other test registers, so a count by hostname sees only this test's rows.
fn unique_hostname(tag: &str) -> String {
    format!("HTUI-{tag}-{}", uuid::Uuid::now_v7().simple())
}

fn identity(box_id: BoxId, hostname: &str) -> Identity {
    Identity {
        box_id,
        hostname: hostname.to_owned(),
    }
}

/// How many `box` rows carry this id.
async fn rows_with_id(pool: &PgPool, id: BoxId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("count box rows by id")
}

/// How many `box` rows carry this hostname.
async fn rows_with_hostname(pool: &PgPool, hostname: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM box WHERE hostname = $1")
        .bind(hostname)
        .fetch_one(pool)
        .await
        .expect("count box rows by hostname")
}

/// `box.machine_fingerprint` of one row.
async fn stored_fingerprint(pool: &PgPool, id: BoxId) -> Option<String> {
    sqlx::query_scalar("SELECT machine_fingerprint FROM box WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read box.machine_fingerprint")
}

/// `hostname`, `machine_fingerprint` and `last_seen_at` of one row, as Postgres prints them.
async fn snapshot(pool: &PgPool, id: BoxId) -> (String, Option<String>, String) {
    sqlx::query_as(
        "SELECT hostname, machine_fingerprint, last_seen_at::text FROM box WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("read the box row")
}

/// The whole row as JSON text: equal before and after means nothing in it was written.
async fn whole_row(pool: &PgPool, id: BoxId) -> String {
    sqlx::query_scalar("SELECT row_to_json(box)::text FROM box WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read the box row as JSON")
}

/// The hazard ANA-16 C4 names: `box.toml` keeps its id across a rename, and the old
/// `UNIQUE (user_id, hostname)` upsert then inserted a second row under the same primary key.
#[tokio::test]
async fn a_hostname_change_keeps_the_box_id_and_registers() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let before = identity(id, &unique_hostname("A"));
    let after = identity(id, &unique_hostname("B"));
    let machine = fp1();

    let first = db
        .store
        .register_box(&before, Some(&machine))
        .await
        .expect("the first registration succeeds");
    let second = db
        .store
        .register_box(&after, Some(&machine))
        .await
        .expect("a renamed box registers under the id box.toml kept");

    assert_eq!(first, Registration::New, "a first registration inserts");
    assert_eq!(
        second,
        Registration::Known {
            renamed_from: Some(before.hostname.clone()),
        },
        "a rename is the same box, and names the old hostname"
    );
    assert_eq!(first.box_id(id), id, "a first registration keeps its id");
    assert_eq!(second.box_id(id), id, "a rename keeps the id");
    assert_eq!(rows_with_id(&db.pool, id).await, 1, "one row for the box");
    let (hostname, _, _) = snapshot(&db.pool, id).await;
    assert_eq!(hostname, after.hostname, "the row carries the new hostname");

    db.drop_db().await;
}

#[tokio::test]
async fn a_copied_box_toml_mints_a_new_box_and_leaves_the_old_row() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let original = identity(id, &unique_hostname("ORIGINAL"));
    let copy = identity(id, &unique_hostname("COPY"));
    let (one, two) = (fp1(), fp2());

    let first = db
        .store
        .register_box(&original, Some(&one))
        .await
        .expect("the original registers");
    assert_eq!(first, Registration::New);
    let before = snapshot(&db.pool, id).await;

    let second = db
        .store
        .register_box(&copy, Some(&two))
        .await
        .expect("the copy registers");
    let Registration::Copied { previous, minted } = second.clone() else {
        panic!("another machine presenting the id is a copy, got {second:?}");
    };
    assert_eq!(previous, id, "the copy names the row box.toml carried");
    assert_ne!(minted, previous, "the copy is a new box");
    assert_eq!(second.box_id(id), minted, "the box now has the minted id");

    assert_eq!(rows_with_id(&db.pool, id).await, 1, "the old row stays");
    assert_eq!(
        rows_with_id(&db.pool, minted).await,
        1,
        "the new row exists"
    );
    assert_eq!(
        snapshot(&db.pool, id).await,
        before,
        "the old row's hostname, fingerprint and last_seen_at are untouched"
    );
    let (hostname, fingerprint, _) = snapshot(&db.pool, minted).await;
    assert_eq!(
        hostname, copy.hostname,
        "the new row carries the copy's hostname"
    );
    assert_eq!(
        fingerprint,
        Some(two.as_hex()),
        "and the copy's machine fingerprint"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn a_matching_fingerprint_is_the_same_box() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let this = identity(id, &unique_hostname("SAME"));
    let machine = fp1();

    let first = db
        .store
        .register_box(&this, Some(&machine))
        .await
        .expect("the first registration");
    let second = db
        .store
        .register_box(&this, Some(&machine))
        .await
        .expect("a reconnect");

    assert_eq!(first, Registration::New);
    assert_eq!(second, Registration::Known { renamed_from: None });
    assert_eq!(rows_with_id(&db.pool, id).await, 1);
    assert_eq!(
        stored_fingerprint(&db.pool, id).await,
        Some(machine.as_hex())
    );

    db.drop_db().await;
}

#[tokio::test]
async fn no_fingerprint_registers_by_id_alone() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // A stored fingerprint and none presented: the id alone decides, and the column is kept.
    let id = BoxId::new();
    let this = identity(id, &unique_hostname("STORED"));
    let machine = fp1();
    db.store
        .register_box(&this, Some(&machine))
        .await
        .expect("register with a fingerprint");
    let again = db
        .store
        .register_box(&this, None)
        .await
        .expect("register without one");
    assert_eq!(again, Registration::Known { renamed_from: None });
    assert_eq!(
        stored_fingerprint(&db.pool, id).await,
        Some(machine.as_hex()),
        "a stored fingerprint is never cleared"
    );

    // Neither side has one.
    let other = BoxId::new();
    let bare = identity(other, &unique_hostname("BARE"));
    let first = db
        .store
        .register_box(&bare, None)
        .await
        .expect("register with none");
    let second = db
        .store
        .register_box(&bare, None)
        .await
        .expect("register with none again");
    assert_eq!(first, Registration::New);
    assert_eq!(second, Registration::Known { renamed_from: None });
    assert_eq!(rows_with_id(&db.pool, other).await, 1);
    assert_eq!(stored_fingerprint(&db.pool, other).await, None);

    db.drop_db().await;
}

#[tokio::test]
async fn a_first_fingerprint_is_recorded_on_a_row_that_had_none() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let hostname = unique_hostname("PLANTED");
    sqlx::query(
        "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
         VALUES ($1, $2, $3, 'linux', '', 'x86_64', '0.0.0')",
    )
    .bind(id.as_uuid())
    .bind(db.store.this_user().as_uuid())
    .bind(&hostname)
    .execute(&db.pool)
    .await
    .expect("plant a row with no fingerprint");
    assert_eq!(stored_fingerprint(&db.pool, id).await, None);

    let machine = fp1();
    let answer = db
        .store
        .register_box(&identity(id, &hostname), Some(&machine))
        .await
        .expect("register over the planted row");
    assert_eq!(answer, Registration::Known { renamed_from: None });
    assert_eq!(
        stored_fingerprint(&db.pool, id).await,
        Some(machine.as_hex()),
        "the first fingerprint a row sees is recorded"
    );

    db.drop_db().await;
}

#[tokio::test]
async fn another_id_on_the_same_hostname_is_another_box() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let hostname = unique_hostname("SHARED");
    let (one, two) = (BoxId::new(), BoxId::new());
    let first = db
        .store
        .register_box(&identity(one, &hostname), Some(&fp1()))
        .await
        .expect("the first id");
    let second = db
        .store
        .register_box(&identity(two, &hostname), Some(&fp2()))
        .await
        .expect("the second id");

    assert_eq!(first, Registration::New);
    assert_eq!(
        second,
        Registration::New,
        "a hostname is not a key: another id is another box"
    );
    assert_eq!(rows_with_hostname(&db.pool, &hostname).await, 2);

    db.drop_db().await;
}

#[tokio::test]
async fn two_first_registrations_of_one_id_both_succeed() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let machine = fp1();
    for round in 0..10 {
        let this = identity(BoxId::new(), &unique_hostname("RACE"));
        let (left, right) = tokio::join!(
            db.store.register_box(&this, Some(&machine)),
            db.store.register_box(&this, Some(&machine)),
        );
        let left = left.unwrap_or_else(|err| panic!("round {round}: the left one fails: {err}"));
        let right = right.unwrap_or_else(|err| panic!("round {round}: the right one fails: {err}"));

        let known = Registration::Known { renamed_from: None };
        assert!(
            (left == Registration::New && right == known)
                || (left == known && right == Registration::New),
            "round {round}: one inserts and the other finds it, got {left:?} and {right:?}"
        );
        assert_eq!(
            rows_with_id(&db.pool, this.box_id).await,
            1,
            "round {round}: one row"
        );
    }

    db.drop_db().await;
}

#[tokio::test]
async fn an_id_under_another_user_is_not_adopted() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let stranger = UserId::new();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
        .bind(stranger.as_uuid())
        .bind(format!("stranger-{}", uuid::Uuid::now_v7().simple()))
        .execute(&db.pool)
        .await
        .expect("plant another user");
    let id = BoxId::new();
    sqlx::query(
        "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
         VALUES ($1, $2, 'elsewhere', 'linux', '', 'x86_64', '0.0.0')",
    )
    .bind(id.as_uuid())
    .bind(stranger.as_uuid())
    .execute(&db.pool)
    .await
    .expect("plant a box under the other user");
    let before = whole_row(&db.pool, id).await;

    let answer = db
        .store
        .register_box(&identity(id, &unique_hostname("MINE")), Some(&fp1()))
        .await
        .expect("register the stranger's id");
    let Registration::Copied { previous, minted } = answer.clone() else {
        panic!("another user's id is never adopted, got {answer:?}");
    };
    assert_eq!(previous, id);
    assert_ne!(minted, id);
    assert_eq!(
        whole_row(&db.pool, id).await,
        before,
        "the stranger's row is untouched"
    );
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT user_id FROM box WHERE id = $1")
        .bind(minted.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read the minted row's owner");
    assert_eq!(owner, db.store.this_user().as_uuid(), "the new box is mine");

    db.drop_db().await;
}

#[tokio::test]
async fn the_stored_fingerprint_is_the_keyed_hash_never_the_raw_identity() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    db.store
        .register_box(&identity(id, &unique_hostname("KEYED")), Some(&fp(RAW)))
        .await
        .expect("register");

    assert_eq!(
        stored_fingerprint(&db.pool, id).await.as_deref(),
        Some(RAW_FINGERPRINT),
        "the column holds the HMAC"
    );
    let leaks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM box WHERE row_to_json(box)::text LIKE '%' || $1 || '%'",
    )
    .bind(RAW)
    .fetch_one(&db.pool)
    .await
    .expect("search every box row for the raw identity");
    assert_eq!(leaks, 0, "no row carries the raw identity");

    db.drop_db().await;
}

#[tokio::test]
async fn a_reconnect_leaves_htui_version_and_the_probe_columns_alone() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    let id = BoxId::new();
    let this = identity(id, &unique_hostname("PROBED"));
    let machine = fp1();
    db.store
        .register_box(&this, Some(&machine))
        .await
        .expect("register");
    sqlx::query(
        "UPDATE box SET htui_version = '0.0.0', last_probed_at = NULL, \
                        probe_spec_digest = repeat('a', 64) \
          WHERE id = $1",
    )
    .bind(id.as_uuid())
    .execute(&db.pool)
    .await
    .expect("stand in for an older probe");

    let answer = db
        .store
        .register_box(&this, Some(&machine))
        .await
        .expect("reconnect");
    assert_eq!(answer, Registration::Known { renamed_from: None });

    let (version, probed, digest): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT htui_version, last_probed_at::text, probe_spec_digest FROM box WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("read the probe columns");
    assert_eq!(
        version, "0.0.0",
        "only the probe writer rewrites htui_version"
    );
    assert_eq!(probed, None, "registration never stamps last_probed_at");
    assert_eq!(
        digest.as_deref(),
        Some("a".repeat(64).as_str()),
        "registration never writes probe_spec_digest"
    );

    db.drop_db().await;
}

/// MOD-7 D37 (blueprint; a deferred T2 finding): `boxes()` lists only this user's boxes, by
/// ascending id, each box's tools by name bytes (`COLLATE "C"`). The planted second box of this
/// user sorts **before** the registered one, so insertion order cannot pass; another user's box
/// and its tool never appear.
#[tokio::test]
async fn boxes_lists_only_this_user_s_boxes_in_id_order() {
    use htui_core::store::WriteStore as _;

    let Some(db) = common::fresh_db().await else {
        return;
    };

    let me = db.store.this_user();
    let registered = db.store.this_box();
    let second = BoxId::from_uuid(uuid::Uuid::from_u128(1));
    let foreign = BoxId::from_uuid(uuid::Uuid::from_u128(2));
    assert!(
        second < registered && foreign < registered,
        "the planted ids sort before the registered one"
    );
    let stranger = UserId::new();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
        .bind(stranger.as_uuid())
        .bind(format!("stranger-{}", uuid::Uuid::now_v7().simple()))
        .execute(&db.pool)
        .await
        .expect("plant another user");
    for (id, owner, hostname) in [
        (second, me, unique_hostname("SECOND")),
        (foreign, stranger, unique_hostname("ELSEWHERE")),
    ] {
        sqlx::query(
            "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version) \
             VALUES ($1, $2, $3, 'linux', '', 'x86_64', '0.0.0')",
        )
        .bind(id.as_uuid())
        .bind(owner.as_uuid())
        .bind(&hostname)
        .execute(&db.pool)
        .await
        .expect("plant a box");
    }
    for (box_id, name) in [
        (second, "awk"),
        (second, "Zig"),
        (second, "_x"),
        (registered, "git"),
        (foreign, "leak"),
    ] {
        sqlx::query("INSERT INTO box_tool (box_id, name, version, path) VALUES ($1, $2, '1', $3)")
            .bind(box_id.as_uuid())
            .bind(name)
            .bind(format!("/usr/bin/{name}"))
            .execute(&db.pool)
            .await
            .expect("plant a tool");
    }

    let records = db.store.boxes().await.expect("boxes answers");

    let listed: Vec<BoxId> = records.iter().map(|record| record.row.id).collect();
    assert_eq!(
        listed,
        [second, registered],
        "this user's two boxes, by ascending id"
    );
    assert!(
        records.iter().all(|record| record.row.user_id == me),
        "no other user's box is listed: {records:?}"
    );
    let tools: Vec<&str> = records[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(tools, ["Zig", "_x", "awk"], "tools by name bytes");
    let own: Vec<&str> = records[1]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(own, ["git"], "the registered box keeps only its own tool");

    db.drop_db().await;
}
