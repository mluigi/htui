//! Startup and reconnect over a real server (plan D10, blueprint C.15).
//!
//! Every case creates and drops its own database and opens its mirror in a throwaway directory, so
//! nothing under `%APPDATA%\htui` (or `~/.config/htui`) is ever touched. With
//! `HTUI_TEST_DATABASE_URL` unset each case prints `common::SKIP` and passes (plan D13); the two
//! that need no server never skip.

mod common;

use std::time::Duration;

use htui_store::connect::{ConnEvent, StartOptions, Started, refresh_settings, start};
use htui_store::{Backend, MigrationState};

/// How long a `ConnEvent` may take. The connect pool's own acquire timeout is ten seconds, so this
/// is "the attempt answered at all", not a performance assertion.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

/// The next `ConnEvent`, or a panic naming what was being waited for.
async fn next_event(started: &mut Started, what: &str) -> ConnEvent {
    match tokio::time::timeout(EVENT_TIMEOUT, started.events.recv()).await {
        Ok(Some(event)) => event,
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
        config_root: root.path().to_owned(),
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
        MigrationState::Pending(1),
        "the harness left the schema unapplied"
    );
    let root = tempfile::tempdir().expect("temp root");

    let mut started = start(StartOptions {
        dsn: Some(db.url.clone()),
        offline: false,
        config_root: root.path().to_owned(),
    })
    .await
    .expect("start");

    match next_event(&mut started, "a bare database").await {
        ConnEvent::MigrationsPending(_, pending) => assert_eq!(
            pending, 1,
            "one embedded migration is waiting (R-STO-5: nothing is applied unasked)"
        ),
        other => panic!("expected MigrationsPending, got {other:?}"),
    }

    close(&started).await;
    db.drop_db().await;
}

#[tokio::test]
async fn an_unreachable_server_is_a_failed_event_and_the_shell_still_starts() {
    let root = tempfile::tempdir().expect("temp root");
    // Port 1 on the loopback: refused immediately, no DNS, no server needed.
    let mut started = start(StartOptions {
        dsn: Some("postgres://nobody:nothing@127.0.0.1:1/none".to_owned()),
        offline: false,
        config_root: root.path().to_owned(),
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
        config_root: root.path().to_owned(),
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
        config_root: root.path().to_owned(),
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
    assert!(expected.join("pending").is_dir());

    close(&started).await;
}
