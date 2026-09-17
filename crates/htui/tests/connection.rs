//! `Settings > Connection`, from the worker side out (MOD-15 milestone 6, D18).
//!
//! The worker half is this file's first half and T2's: it drives `htui::store_worker::serve`
//! directly where a request needs no loop, and a **spawned worker** over two channels where it
//! does — the three writers rewire `backend`, `refresher`, `held` and `reconnect`, which only the
//! loop owns. The section half is T3's and follows below.
//!
//! **Every case that can reach the keyring takes `common::mock_keyring()` as its first
//! statement.** The guard installs a process-wide fake slot and holds a lock for as long as it
//! lives, so a case here can store a DSN and read it back without the developer's OS store ever
//! being opened (blueprint flag L). The two cases that deliberately take no guard are the ones
//! asserting that `Backend::Memory` never consults a keyring at all (D10): if they were wrong,
//! they would be wrong against the real one.
//!
//! Four cases are Postgres-gated and return without asserting when `HTUI_TEST_DATABASE_URL` is
//! unset, as every other Postgres-backed suite in this crate does. One of them,
//! `set_dsn_goes_online_without_a_restart`, is the milestone's whole point: a box that started
//! with an empty keyring reaches `online` **in the same process**.
#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;

use htui::connection::{
    Attempt, AttemptOutcome, ConnectionSnapshot, DEMO_SESSION, NO_WORKER, READ_NAME, REQUEST_NAMES,
};
use htui::store_worker::{
    self, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, serve,
};
use htui::ui::tabs::settings::SettingsTab;
use htui_core::store::MemStore;
use htui_store::testkit as common;
use htui_store::{Backend, CacheStore, ConnEvent, ConnectContext, Dsn, PgStore, Started, secret};
use tokio::sync::mpsc;

/// A DSN that parses and names a port nothing listens on, for the cases about *storing* rather
/// than about connecting. `htui` is the user so a summary assertion has something to find.
const DEAD_DSN: &str = "postgres://htui:s3cret@127.0.0.1:1/htui?sslmode=disable";

/// The demo world behind a memory backend: the `--demo` session D10 is written for.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The connection snapshot a served reply carries, or a panic naming what came back instead.
#[track_caller]
fn connection(reply: StoreReply) -> ConnectionSnapshot {
    match reply {
        StoreReply::Connection(snapshot) => snapshot,
        other => panic!("expected a connection snapshot: {other:?}"),
    }
}

/// The `Failed` reply's `(request, message)`, or a panic naming what came back instead.
#[track_caller]
fn refusal(reply: StoreReply) -> (&'static str, String) {
    match reply {
        StoreReply::Failed { request, message } => (request, message),
        other => panic!("expected a refusal: {other:?}"),
    }
}

/// One of each of the four, in [`REQUEST_NAMES`] order.
fn connection_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::ConnectionInfo,
        StoreRequest::SetDsn(Dsn::parse(DEAD_DSN).expect("a parseable DSN")),
        StoreRequest::ClearDsn,
        StoreRequest::RebuildCache,
    ]
}

/// A spawned store worker and the two channels a test talks to it over.
///
/// The `Harness` cannot stand in for this: it serves inline through `store_worker::serve`, which
/// is `try_serve` and therefore the half of the seam that has no loop state to rewire (flag K).
struct Worker {
    /// Requests in. Dropped by [`Worker::shutdown`], which is what ends the loop.
    requests: mpsc::UnboundedSender<RequestEnvelope>,
    /// Replies out, in whatever order the loop produced them.
    replies: mpsc::UnboundedReceiver<ReplyEnvelope>,
    /// The loop's own handle, awaited by [`Worker::shutdown`].
    task: tokio::task::JoinHandle<()>,
    /// The next sequence number; a reply whose `seq` is not the one asked for is ignored, so an
    /// unsolicited answer cannot be mistaken for this request's.
    seq: u64,
}

impl Worker {
    /// Spawns the real loop over `started`.
    fn spawn(started: Started) -> Self {
        let (requests, request_rx) = mpsc::unbounded_channel();
        let (reply_tx, replies) = mpsc::unbounded_channel();
        let task = store_worker::spawn(started, request_rx, reply_tx);
        Self {
            requests,
            replies,
            task,
            seq: 1,
        }
    }

    /// Sends one request and waits for the reply addressed to it.
    ///
    /// # Panics
    ///
    /// If the worker is gone before it answers.
    async fn ask(&mut self, request: StoreRequest) -> StoreReply {
        let seq = self.seq;
        self.seq += 1;
        self.requests
            .send(RequestEnvelope {
                seq,
                origin: Origin::Tab(SettingsTab::ID),
                request,
            })
            .expect("the worker is alive");
        loop {
            let envelope = self.replies.recv().await.expect("the worker answers");
            if envelope.seq == seq {
                return envelope.reply;
            }
        }
    }

    /// One `ConnectionInfo`, the read every case checks its writes against.
    async fn info(&mut self) -> ConnectionSnapshot {
        connection(self.ask(StoreRequest::ConnectionInfo).await)
    }

    /// Re-reads until `wanted` answers `true`, or gives up after `within`.
    ///
    /// Polling rather than sleeping once: a dial resolves when the server answers, and a fixed
    /// sleep would either be flaky or be the slowest machine's worst case on every run.
    async fn poll_until(
        &mut self,
        within: Duration,
        wanted: impl Fn(&ConnectionSnapshot) -> bool,
    ) -> ConnectionSnapshot {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let snapshot = self.info().await;
            if wanted(&snapshot) {
                return snapshot;
            }
            if tokio::time::Instant::now() >= deadline {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Closes the request channel and awaits the loop, so a mirror it holds is released before a
    /// `TempDir` tries to delete it.
    async fn shutdown(self) {
        drop(self.requests);
        let _ = self.task.await;
    }
}

/// A loopback socket that completes a TCP connect and then says nothing.
///
/// A dial against it is provably still in flight — sqlx has sent its `SSLRequest` and is waiting
/// for the one byte back — until [`SilentServer::release`] drops the accepted stream, at which
/// point the dial ends in a `ConnEvent::Failed`. That is what makes the generation test (ruling
/// O-1) about ordering rather than about timing.
struct SilentServer {
    /// Where the DSN points.
    addr: SocketAddr,
    /// Fired by [`SilentServer::release`]; the accept task ends and its held streams drop.
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

impl SilentServer {
    /// Binds an ephemeral loopback port and starts accepting.
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let (release, wait) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut held = Vec::new();
            let accepting = async {
                while let Ok((stream, _)) = listener.accept().await {
                    held.push(stream);
                }
            };
            tokio::select! {
                () = accepting => {}
                _ = wait => {}
            }
            drop(held);
        });
        Self {
            addr,
            release: Some(release),
        }
    }

    /// A DSN naming this socket, with `database` so two of them are told apart.
    fn dsn(&self, database: &str) -> Dsn {
        Dsn::parse(&format!(
            "postgres://htui@{}:{}/{database}",
            self.addr.ip(),
            self.addr.port()
        ))
        .expect("a loopback DSN parses")
    }

    /// Lets every dial against this socket end.
    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

/// A throwaway config root with an opened mirror under `fingerprint`.
async fn mirror(fingerprint: &str) -> (tempfile::TempDir, CacheStore) {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), fingerprint, PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    (root, cache)
}

/// The context a worker needs to apply a DSN, rooted at a throwaway directory.
fn context(root: &tempfile::TempDir, connect_timeout: Duration, offline: bool) -> ConnectContext {
    ConnectContext {
        config_root: root.path().to_owned(),
        connect_timeout,
        offline,
    }
}

// ---------------------------------------------------------------------------------------------
// Names and the two refusals
// ---------------------------------------------------------------------------------------------

/// `REQUEST_NAMES` is what `StoreRequest::name` answers for the four, in variant order, and no two
/// of them share a name — the `Failed` routing the section does is by name alone (D4).
#[test]
fn connection_names_are_stable() {
    let names: Vec<&'static str> = connection_requests()
        .iter()
        .map(StoreRequest::name)
        .collect();

    assert_eq!(names, REQUEST_NAMES);
    assert_eq!(READ_NAME, "connection_info");

    let unique: BTreeSet<&'static str> = names.iter().copied().collect();
    assert_eq!(unique.len(), REQUEST_NAMES.len(), "four distinct names");
}

/// `try_serve` has no loop to rewire, so it refuses all three writers by name (D9).
///
/// The sentence is not the demo one: a build with no worker and a demo session are two different
/// situations, and the section tells them apart by text (B-4).
#[tokio::test]
async fn try_serve_refuses_every_writer_by_name() {
    let backend = demo();
    for (request, name) in connection_requests().into_iter().zip(REQUEST_NAMES).skip(1) {
        let (failed, message) = refusal(serve(&backend, &request).await);
        assert_eq!(failed, name);
        assert_eq!(message, NO_WORKER);
    }
}

/// `Backend::Memory` answers `dsn_stored: None` and never opens a keyring (D10).
///
/// **No `mock_keyring` guard on purpose**: this is the case that would reach the developer's own
/// credential store if the arm were wrong, and `Harness::settle` serves `ConnectionInfo` through
/// exactly this path on every `--demo` snapshot in the suite.
#[tokio::test]
async fn memory_answers_none_for_dsn_stored_and_never_opens_the_keyring() {
    let snapshot = connection(serve(&demo(), &StoreRequest::ConnectionInfo).await);

    assert_eq!(snapshot.label, "memory");
    assert_eq!(snapshot.dsn_stored, None, "no keyring story in this build");
    assert_eq!(snapshot.dsn_summary, None);
    assert_eq!(snapshot.mirror, None, "a memory backend has no mirror");
    assert_eq!(snapshot.last_attempt, None);
    assert!(!snapshot.offline);
}

/// The worker refuses all three writers on a demo session, with the demo sentence (D10, B-4).
#[tokio::test]
async fn the_worker_refuses_the_writers_on_memory() {
    let mut worker = Worker::spawn(Started::detached(demo()));

    for (request, name) in connection_requests().into_iter().zip(REQUEST_NAMES).skip(1) {
        let (failed, message) = refusal(worker.ask(request).await);
        assert_eq!(failed, name);
        assert_eq!(
            message, DEMO_SESSION,
            "a demo session, not a missing worker"
        );
    }

    worker.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// The read over a real mirror
// ---------------------------------------------------------------------------------------------

/// A stored DSN is reported by its redacted summary and by nothing else (D1, R-TUI-8).
#[tokio::test]
async fn an_offline_backend_reports_a_stored_dsn_by_summary_only() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-read").await;
    secret::set_dsn(DEAD_DSN).expect("the fake keyring accepts a write");

    let mut worker = Worker::spawn(Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    }));
    let snapshot = worker.info().await;

    assert_eq!(snapshot.dsn_stored, Some(true));
    assert_eq!(
        snapshot.dsn_summary,
        Some(Dsn::parse(DEAD_DSN).expect("parses").summary()),
        "the summary is the newtype's, not a second rendering"
    );
    let mirror = snapshot
        .mirror
        .as_ref()
        .expect("an offline backend mirrors");
    assert_eq!(mirror.db_fingerprint, "connection-read");

    let printed = format!("{snapshot:?}");
    assert!(
        !printed.contains("s3cret"),
        "no Debug of the snapshot carries the password: {printed}"
    );

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// A DSN `--set-dsn` stored raw before this milestone reads as stored, without a summary (B-7).
#[tokio::test]
async fn an_unparseable_stored_dsn_reports_stored_without_a_summary() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-unparseable").await;
    // An unrecognised query key: sqlx still connects with it, so `connect::start` keeps working,
    // but `Dsn::parse` refuses it and the section must say so rather than invent a summary.
    secret::set_dsn("postgres://htui@127.0.0.1:1/htui?foo=bar").expect("the fake keyring writes");

    let mut worker = Worker::spawn(Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    }));
    let snapshot = worker.info().await;

    assert_eq!(snapshot.dsn_stored, Some(true));
    assert_eq!(snapshot.dsn_summary, None, "it fails closed, not open");

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

// ---------------------------------------------------------------------------------------------
// The three writers
// ---------------------------------------------------------------------------------------------

/// A worker with no connect context has nothing to open a mirror under, so `SetDsn` refuses and
/// leaves every piece of loop state where it was (D11's "any failure before (6)").
#[tokio::test]
async fn set_dsn_without_a_context_fails_and_changes_nothing() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-no-context").await;

    // `detached` is `connect: None`, which is what `--demo` and every harness carries.
    let mut worker = Worker::spawn(Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    }));
    let (failed, message) = refusal(
        worker
            .ask(StoreRequest::SetDsn(Dsn::parse(DEAD_DSN).expect("parses")))
            .await,
    );

    assert_eq!(failed, "set_dsn");
    assert_eq!(message, NO_WORKER);

    let snapshot = worker.info().await;
    assert_eq!(snapshot.dsn_stored, Some(false), "nothing was stored");
    assert_eq!(
        snapshot
            .mirror
            .as_ref()
            .expect("the mirror is untouched")
            .db_fingerprint,
        "connection-no-context"
    );
    assert_eq!(common::fake_dsn(), None, "the keyring was never written");

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// `ClearDsn` empties the keyring, disarms the dial and says nothing about the live connection,
/// which it deliberately does not tear down (D13).
#[tokio::test]
async fn clear_dsn_empties_the_keyring_and_keeps_the_backend() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-clear").await;
    secret::set_dsn(DEAD_DSN).expect("the fake keyring writes");

    let mut worker = Worker::spawn(Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    }));
    let before = worker.info().await;
    assert_eq!(before.dsn_stored, Some(true));

    let after = connection(worker.ask(StoreRequest::ClearDsn).await);
    assert_eq!(after.dsn_stored, Some(false));
    assert_eq!(after.dsn_summary, None);
    assert_eq!(common::fake_dsn(), None, "the keyring entry is gone");
    assert_eq!(
        after.label, before.label,
        "the session keeps its backend until it quits"
    );

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// `--offline` stores and re-opens but never dials (D12).
///
/// Not Postgres-gated, unlike the plan's sketch: the point of the case is that **nothing dialled**,
/// and a live server would only make that harder to see.
#[tokio::test]
async fn set_dsn_under_offline_stores_but_does_not_dial() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-offline").await;
    let dsn = Dsn::parse(DEAD_DSN).expect("parses");
    let fingerprint = dsn.fingerprint();

    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    });
    started.connect = Some(context(&root, Duration::from_millis(250), true));
    let mut worker = Worker::spawn(started);

    let snapshot = connection(worker.ask(StoreRequest::SetDsn(dsn)).await);
    assert_eq!(snapshot.dsn_stored, Some(true));
    assert!(snapshot.offline, "the snapshot carries the session's flag");
    assert!(
        snapshot.label.starts_with("offline · "),
        "`--offline` never reads `connecting`: {}",
        snapshot.label
    );
    assert_eq!(
        snapshot
            .mirror
            .as_ref()
            .expect("the new mirror is open")
            .db_fingerprint,
        fingerprint,
        "the mirror was re-opened under the new DSN even though nothing dials"
    );

    // Well past the dead DSN's connect timeout: had a dial been spawned, its `Failed` would be
    // the last attempt by now.
    tokio::time::sleep(Duration::from_millis(750)).await;
    let later = worker.info().await;
    assert!(
        later.label.starts_with("offline · "),
        "still offline: {}",
        later.label
    );
    assert_eq!(later.last_attempt, None, "no dial was ever spawned");

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// `RebuildCache` empties the sixteen mirrored tables and the cursor, and keeps the file's
/// identity (D14, `cache/mod.rs:166-171`).
#[tokio::test]
async fn rebuild_cache_empties_the_mirrored_tables_and_keeps_the_meta() {
    let (root, cache) = mirror("connection-rebuild").await;
    common::seed_mirror(&cache, &htui_core::fixtures::demo_data())
        .await
        .expect("the mirror is seeded");
    let before = cache.meta().await.expect("a seeded mirror has meta");
    let items: i64 = sqlx::query_scalar("SELECT count(*) FROM item")
        .fetch_one(cache.pool())
        .await
        .expect("the mirror answers");
    assert!(items > 0, "the fixture put rows in");

    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    });
    started.connect = Some(context(&root, Duration::from_millis(250), false));
    let mut worker = Worker::spawn(started);

    let snapshot = connection(worker.ask(StoreRequest::RebuildCache).await);
    let after = snapshot.mirror.as_ref().expect("the mirror survives");

    assert_eq!(after.db_fingerprint, before.db_fingerprint);
    assert_eq!(after.schema_version, before.schema_version);
    assert_eq!(after.built_at, before.built_at, "the file is not recreated");
    assert_eq!(
        after.last_full_refresh_at, None,
        "nothing has been fully pulled into this content yet"
    );

    let items: i64 = sqlx::query_scalar("SELECT count(*) FROM item")
        .fetch_one(cache.pool())
        .await
        .expect("the mirror answers");
    assert_eq!(items, 0, "the mirrored tables are empty");

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// A dial's outcome is remembered and reported, which is what the Status row shows (B-5).
#[tokio::test]
async fn a_failed_dial_is_the_last_attempt() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-attempt").await;

    let started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: None,
    });
    let events = started.events_tx.clone();
    let mut worker = Worker::spawn(started);

    assert_eq!(worker.info().await.last_attempt, None, "nothing yet");

    events
        .send(ConnEvent::Failed("refused".to_owned()))
        .await
        .expect("the worker is listening");

    let snapshot = worker
        .poll_until(Duration::from_secs(5), |snapshot| {
            snapshot.last_attempt.is_some()
        })
        .await;
    let Some(Attempt { outcome, .. }) = snapshot.last_attempt else {
        panic!("the dial's outcome is remembered: {snapshot:?}");
    };
    assert_eq!(outcome, AttemptOutcome::Failed("refused".to_owned()));

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

// ---------------------------------------------------------------------------------------------
// The generation guard (ruling O-1, hazard H-6)
// ---------------------------------------------------------------------------------------------

/// A dial spawned before a `SetDsn` is discarded when it lands after the swap (O-1, H-6).
///
/// Without the guard the old server's `ConnEvent` installs its `PgStore` over the **new** mirror,
/// which is PRD `:373`'s named risk: this database's reads answered from another database's cache.
/// The evidence is `last_attempt`, which `SetDsn` clears: if the stale dial's event were
/// delivered, it would be `Some` again. The second half of the case is the control — the dial
/// spawned by the *last* `SetDsn` still carries the current generation, so its event **is**
/// delivered, which is what makes the first half about ordering rather than about a guard that
/// swallows everything.
#[tokio::test]
async fn a_dial_in_flight_across_a_set_dsn_is_discarded() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-generation").await;
    let mut stale = SilentServer::start().await;
    let mut current = SilentServer::start().await;

    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    });
    // A minute: neither dial may end on a timeout, only when this test releases its socket.
    started.connect = Some(context(&root, Duration::from_secs(60), false));
    let mut worker = Worker::spawn(started);

    // Generation 0's dial: in flight against a socket that accepts and says nothing.
    let first = connection(worker.ask(StoreRequest::SetDsn(stale.dsn("first"))).await);
    assert_eq!(first.label, "connecting", "the swap dialled at once");

    // Generation 1: the stale dial is now answering for a server this session has left.
    let second = connection(
        worker
            .ask(StoreRequest::SetDsn(current.dsn("second")))
            .await,
    );
    assert_eq!(second.last_attempt, None, "a swap forgets the last dial");

    stale.release();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        worker.info().await.last_attempt,
        None,
        "the stale dial's event was dropped, not installed"
    );

    current.release();
    let live = worker
        .poll_until(Duration::from_secs(5), |snapshot| {
            snapshot.last_attempt.is_some()
        })
        .await;
    assert!(
        matches!(
            live.last_attempt,
            Some(Attempt {
                outcome: AttemptOutcome::Failed(_),
                ..
            })
        ),
        "the current generation's dial is still delivered: {live:?}"
    );

    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

// ---------------------------------------------------------------------------------------------
// Postgres-gated: the transition this milestone exists for
// ---------------------------------------------------------------------------------------------

/// **The headline.** A box that started with an empty keyring reaches `online` without a restart
/// (PRD D9, D11), and the mirror it reads through is the new DSN's.
#[tokio::test]
async fn set_dsn_goes_online_without_a_restart() {
    let Some(db) = common::fresh_db().await else {
        println!("{}", common::SKIP);
        return;
    };
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror(htui_store::connect::NO_DSN_FINGERPRINT).await;
    let dsn = Dsn::parse(&db.url).expect("the test DSN parses");
    let fingerprint = dsn.fingerprint();

    // Exactly the shape `connect::start` hands back for an empty keyring: offline, no reconnect.
    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    });
    started.connect = Some(context(&root, Duration::from_secs(10), false));
    let mut worker = Worker::spawn(started);

    assert_eq!(
        worker.info().await.dsn_stored,
        Some(false),
        "an empty keyring is where this box starts"
    );

    let stored = connection(worker.ask(StoreRequest::SetDsn(dsn)).await);
    assert_eq!(stored.dsn_stored, Some(true));
    assert_eq!(stored.label, "connecting", "the dial is already in flight");
    assert_eq!(
        stored
            .mirror
            .as_ref()
            .expect("the new mirror is open")
            .db_fingerprint,
        fingerprint,
        "the mirror was re-opened under the new DSN before the swap (D11 step 3)"
    );

    let online = worker
        .poll_until(Duration::from_secs(30), |snapshot| {
            snapshot.label == "online"
        })
        .await;
    assert_eq!(
        online.label, "online",
        "the same process reached the server: {online:?}"
    );
    assert_eq!(
        online
            .mirror
            .as_ref()
            .expect("still mirrored")
            .db_fingerprint,
        fingerprint
    );
    assert!(
        root.path().join("cache").join(&fingerprint).is_dir(),
        "the live mirror is `<root>/cache/<db_fingerprint(dsn)>`"
    );
    assert_eq!(
        common::fake_dsn().as_deref(),
        Some(db.url.as_str()),
        "and the next launch inherits it"
    );

    worker.shutdown().await;
    cache.close().await;
    drop(root);
    db.drop_db().await;
}

/// A second `SetDsn` swaps the mirror again and leaves the first database's file on disk (D11).
#[tokio::test]
async fn a_second_set_dsn_swaps_the_mirror_and_leaves_the_old_file() {
    let Some(first) = common::fresh_db().await else {
        println!("{}", common::SKIP);
        return;
    };
    let Some(second) = common::fresh_db().await else {
        println!("{}", common::SKIP);
        first.drop_db().await;
        return;
    };
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror(htui_store::connect::NO_DSN_FINGERPRINT).await;
    let one = Dsn::parse(&first.url).expect("parses");
    let two = Dsn::parse(&second.url).expect("parses");
    let (one_print, two_print) = (one.fingerprint(), two.fingerprint());
    assert_ne!(one_print, two_print, "two databases, two mirrors");

    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(chrono::Utc::now()),
    });
    started.connect = Some(context(&root, Duration::from_secs(10), false));
    let mut worker = Worker::spawn(started);

    connection(worker.ask(StoreRequest::SetDsn(one)).await);
    let after = connection(worker.ask(StoreRequest::SetDsn(two)).await);

    assert_eq!(
        after.mirror.as_ref().expect("mirrored").db_fingerprint,
        two_print,
        "the second database's mirror is the one being read"
    );
    assert!(
        root.path()
            .join("cache")
            .join(&one_print)
            .join("cache.sqlite")
            .is_file(),
        "the first mirror's file is left where it was, not deleted"
    );

    worker.shutdown().await;
    cache.close().await;
    drop(root);
    first.drop_db().await;
    second.drop_db().await;
}
