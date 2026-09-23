//! Startup and reconnect over a real server (plan D10, blueprint C.15).
//!
//! Every case creates and drops its own database and opens its mirror in a throwaway directory, so
//! nothing under `%APPDATA%\htui` (or `~/.config/htui`) is ever touched. With
//! `HTUI_TEST_DATABASE_URL` unset each case prints `common::SKIP` and passes (plan D13); the two
//! that need no server never skip.

use htui_store::testkit as common;

use std::time::Duration;

use htui_core::store::StoreError;
use htui_store::connect::{ConnEvent, StartOptions, Started, refresh_settings, start};
use htui_store::{Backend, MigrationState, map_sqlx};

/// How long a `ConnEvent` may take. The connect pool's own acquire timeout is ten seconds, so this
/// is "the attempt answered at all", not a performance assertion.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

/// The next `ConnEvent`, or a panic naming what was being waited for.
async fn next_event(started: &mut Started, what: &str) -> ConnEvent {
    match tokio::time::timeout(EVENT_TIMEOUT, started.events.recv()).await {
        // The generation the launch dial reports under; there is no worker here to move it.
        Ok(Some((_, event))) => event,
        Ok(None) => panic!("the connect task closed the channel without an event ({what})"),
        Err(_) => panic!("no ConnEvent within {EVENT_TIMEOUT:?} ({what})"),
    }
}

/// Closes the mirror so the throwaway root can be removed on Windows.
async fn close(started: &Started) {
    if let Some(cache) = started.backend.cache() {
        cache.close().await;
    }
}

#[tokio::test]
async fn start_opens_the_mirror_offline_and_reports_online_over_a_migrated_database() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let root = tempfile::tempdir().expect("temp root");

    let mut started = start(StartOptions {
        dsn: Some(db.url.clone()),
        offline: false,
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start");

    // ANA-9 4.4 step 1: the shell has a backend before the network is touched.
    assert!(
        matches!(started.backend, Backend::Offline { since: None, .. }),
        "start hands back an offline backend whose first attempt has not answered"
    );
    assert_eq!(started.backend.label(), "connecting");
    assert!(started.reconnect.is_some(), "a stored DSN arms the ticker");
    assert!(
        root.path().join("box.toml").exists(),
        "the identity is minted under the throwaway root"
    );

    match next_event(&mut started, "a migrated database").await {
        ConnEvent::Online(pg) => {
            assert_eq!(pg.identity().box_id, pg.this_box(), "the id is adopted");
            let settings = refresh_settings(&pg, started.settings).await;
            assert_eq!(
                settings.interval,
                Duration::from_secs(30),
                "cache_refresh_seconds is seeded at 30"
            );
            assert_eq!(
                settings.overlap,
                Duration::from_secs(300),
                "cache_overlap_seconds is seeded at 300"
            );
            assert_eq!(settings.this_box, pg.this_box());
            assert_eq!(settings.this_user, pg.this_user());
        }
        other => panic!("expected Online, got {other:?}"),
    }

    close(&started).await;
    db.drop_db().await;
}

#[tokio::test]
async fn start_reports_the_pending_count_over_a_bare_database() {
    let Some(db) = common::bare_db().await else {
        return;
    };
    assert_eq!(
        db.migrations_at_connect,
        MigrationState::Pending(4),
        "the harness left the schema unapplied"
    );
    let root = tempfile::tempdir().expect("temp root");

    let mut started = start(StartOptions {
        dsn: Some(db.url.clone()),
        offline: false,
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start");

    match next_event(&mut started, "a bare database").await {
        ConnEvent::MigrationsPending(_, pending) => assert_eq!(
            pending, 4,
            "all four embedded migrations are waiting (R-STO-5: nothing is applied unasked)"
        ),
        other => panic!("expected MigrationsPending, got {other:?}"),
    }

    close(&started).await;
    db.drop_db().await;
}

/// The server-side half of `error::is_unreachable`, which a unit test cannot reach.
///
/// A `PgStore` that is already connected loses its server mid-session. Nothing here restarts
/// Postgres: `pg_terminate_backend` on this pool's own backends is the same thing from the
/// client's side - the connection goes away under a query and the driver reports either a
/// SQLSTATE `57P01` (admin shutdown) or the socket error that follows it. Both are
/// [`StoreError::Unreachable`], which is what makes the store worker drop to the mirror.
#[tokio::test]
async fn a_terminated_backend_is_an_unreachable_error() {
    let Some(db) = common::fresh_db().await else {
        return;
    };

    // Every backend of this database except the one issuing the kill.
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid()",
    )
    .execute(&db.pool)
    .await
    .expect("terminate the other backends");
    db.pool.close().await;

    let err = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&db.pool)
        .await
        .expect_err("a closed pool cannot answer");
    assert!(
        htui_store::error::is_unreachable(&err),
        "a lost connection is unreachable, not a bad query: {err:?}"
    );
    assert!(
        matches!(map_sqlx(err), StoreError::Unreachable(_)),
        "and map_sqlx classifies it as such"
    );

    db.drop_db().await;
}

/// `Backend::went_offline`, the mid-session half of the `Online` / `Offline` pair.
///
/// It needs a real [`PgStore`] because `Backend::Online` holds one; the swap itself is pure.
#[tokio::test]
async fn an_online_backend_falls_back_onto_its_own_mirror() {
    let Some(db) = common::fresh_db().await else {
        return;
    };
    let root = tempfile::tempdir().expect("temp root");
    let cache = htui_store::CacheStore::open(root.path(), "went-offline", 1)
        .await
        .expect("open a throwaway mirror");

    let mut backend = Backend::Online {
        pg: db.store.clone(),
        cache: cache.clone(),
    };
    assert_eq!(backend.label(), "online");
    assert!(backend.is_writable());

    assert!(backend.went_offline(), "the server stopped answering");
    assert_eq!(backend.label(), "offline · 0s");
    assert!(!backend.is_writable(), "there is no write path any more");
    assert!(
        backend.writable().is_none(),
        "and no way to reach a PgStore"
    );
    assert!(
        backend.cache().is_some(),
        "the mirror moved across rather than being re-opened"
    );
    assert!(
        backend
            .workspaces()
            .await
            .expect("the mirror answers")
            .is_empty(),
        "reads now come from the file, and an unfilled mirror is empty rather than broken"
    );
    assert!(!backend.went_offline(), "a second call is a no-op");

    cache.close().await;
    db.drop_db().await;
}

#[tokio::test]
async fn an_unreachable_server_is_a_failed_event_and_the_shell_still_starts() {
    let root = tempfile::tempdir().expect("temp root");
    // Port 1 on the loopback: no DNS and no server needed. A second is plenty for a dial that
    // nothing is listening for, and it is what keeps this case off the ten-second default.
    let mut started = start(StartOptions {
        dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none".to_owned()),
        offline: false,
        connect_timeout: Duration::from_secs(1),
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start still opens the mirror");

    assert_eq!(started.backend.label(), "connecting");
    match next_event(&mut started, "an unreachable server").await {
        ConnEvent::Failed(why) => assert!(!why.is_empty(), "the failure carries a reason"),
        other => panic!("expected Failed, got {other:?}"),
    }

    close(&started).await;
}

#[tokio::test]
async fn offline_never_dials_and_starts_at_an_age() {
    let root = tempfile::tempdir().expect("temp root");
    let mut started = start(StartOptions {
        dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none".to_owned()),
        offline: true,
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start offline");

    assert!(
        matches!(started.backend, Backend::Offline { since: Some(_), .. }),
        "--offline is not `connecting`: no attempt is coming"
    );
    assert_eq!(started.backend.label(), "offline · 0s");
    assert!(started.reconnect.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(250), started.events.recv())
            .await
            .is_err(),
        "no attempt was spawned, so no event ever arrives"
    );

    close(&started).await;
}

#[tokio::test]
async fn the_mirror_directory_is_the_dsn_fingerprint() {
    let root = tempfile::tempdir().expect("temp root");
    let dsn = "postgres://nobody:nothing@127.0.0.1:1/none";
    let started = start(StartOptions {
        dsn: Some(dsn.to_owned()),
        offline: true,
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start offline");

    let expected = root
        .path()
        .join("cache")
        .join(htui_store::identity::db_fingerprint(dsn));
    assert_eq!(
        started
            .backend
            .cache()
            .expect("an offline backend owns the mirror")
            .dir(),
        expected,
        "the mirror lives under sha256(host:port/dbname), credentials excluded (4.4)"
    );
    assert!(expected.join("cache.sqlite").exists());

    close(&started).await;
}

/// A keyring that cannot be read starts the shell offline rather than refusing to launch.
///
/// The box this exists for has a session bus and no unlocked collection — a minimal window
/// manager, a container, most CI images — and `keyring` answers `NoStorageAccess` there, not
/// `NoEntry`. Before this fix `start` propagated it and the binary never drew a frame, which is
/// the exact opposite of the rule this module opens with: a first launch must still open offline.
///
/// `get_dsn` keeps reporting the failure as an error; what changed is that `start` treats it as
/// "no DSN this session" — an offline backend already at an age, with no reconnect armed and no
/// dial spawned — instead of an abort.
#[tokio::test]
async fn a_broken_keyring_starts_offline_instead_of_refusing_to_launch() {
    let _keyring = common::mock_keyring_broken().await;
    let root = tempfile::tempdir().expect("temp root");

    assert!(
        htui_store::secret::get_dsn().is_err(),
        "the seam still tells an unreadable keyring apart from an empty one"
    );

    let mut started = start(StartOptions::new(root.path().to_owned()))
        .await
        .expect("a keyring that cannot be read is not a reason to abort");

    assert!(
        matches!(started.backend, Backend::Offline { since: Some(_), .. }),
        "no dial is coming, so the top bar reads an age rather than `connecting`"
    );
    assert_eq!(started.backend.label(), "offline · 0s");
    assert!(
        started.reconnect.is_none(),
        "there is no DSN to re-dial with"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(250), started.events.recv())
            .await
            .is_err(),
        "nothing was spawned, so no event ever arrives"
    );
    assert_eq!(
        started
            .backend
            .cache()
            .expect("the mirror is opened anyway")
            .dir(),
        root.path().join("cache").join("offline"),
        "the no-DSN mirror, the same one an empty keyring gets"
    );

    close(&started).await;
}

/// An explicit DSN never consults the keyring, so a broken one cannot spoil `--set-dsn`'s session
/// or a test's own override.
#[tokio::test]
async fn a_broken_keyring_does_not_override_an_explicit_dsn() {
    let _keyring = common::mock_keyring_broken().await;
    let root = tempfile::tempdir().expect("temp root");

    let started = start(StartOptions {
        dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none".to_owned()),
        offline: true,
        ..StartOptions::new(root.path().to_owned())
    })
    .await
    .expect("start offline over an explicit DSN");

    assert_eq!(
        started
            .backend
            .cache()
            .expect("an offline backend owns the mirror")
            .dir(),
        root.path()
            .join("cache")
            .join(htui_store::identity::db_fingerprint(
                "postgres://nobody:nothing@127.0.0.1:1/none"
            )),
        "the explicit DSN's mirror, not the no-DSN one"
    );

    close(&started).await;
}
