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
//!
//! The **section half** is T3's and starts at "Settings > Connection: the section" below. It has
//! no worker at all: a [`SectionBench`] hands the section a [`ConnectionSnapshot`] built here and
//! reads back what the section emitted, and a [`Harness`] is used for the three cases that are
//! about the *shell* — the focus action and the no-DSN redirect — rather than about the section.
//! Every snapshot the section half builds is a literal, so no case in it can reach a keyring, a
//! mirror or a clock.
//!
//! A case that injects on `started.events_tx` sends a `(generation, ConnEvent)` pair, and
//! `htui_store::connect::LAUNCH_GENERATION` — zero — is the launch dial's shape. Such an injection
//! is delivered while no `SetDsn` has run and **dropped** after one, which is the point of the two
//! `a_launch_dial_*` cases below.
#![cfg(feature = "testkit")]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;

use chrono::{DateTime, TimeZone as _, Utc};
use htui::app::{Action, Handled, TabAction};
use htui::connection::{
    Attempt, AttemptOutcome, ConnectionSnapshot, DEMO_SESSION, MirrorInfo, NO_WORKER, READ_NAME,
    REQUEST_NAMES,
};
use htui::store_worker::{
    self, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, serve,
};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::BacklogTab;
use htui::ui::tabs::settings::{
    AgentsSection, ConnectionSection, HierarchySection, KindsSection, PromptSection, SectionId,
    SettingsSection, SettingsTab,
};
use htui_core::model::{BoxId, Scope};
use htui_core::store::MemStore;
use htui_store::testkit as common;
use htui_store::{
    Backend, CacheStore, ConnEvent, ConnectContext, Dsn, Identity, PgStore, Started, secret,
};
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
        since: Some(Utc::now()),
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
        since: Some(Utc::now()),
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
        since: Some(Utc::now()),
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
        since: Some(Utc::now()),
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
        since: Some(Utc::now()),
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
    // `project`, not `item`: `seed_mirror` writes the six unscoped tables an offline *write* path
    // needs, and the cursor-driven ones are not among them. It is one of the sixteen either way.
    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM project")
        .fetch_one(cache.pool())
        .await
        .expect("the mirror answers");
    assert!(projects > 0, "the fixture put rows in");

    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
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

    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM project")
        .fetch_one(cache.pool())
        .await
        .expect("the mirror answers");
    assert_eq!(projects, 0, "the mirrored tables are empty");
    let cursors: i64 = sqlx::query_scalar("SELECT count(*) FROM cache_cursor")
        .fetch_one(cache.pool())
        .await
        .expect("the mirror answers");
    assert_eq!(
        cursors, 0,
        "and so is the cursor the next pass restarts from"
    );

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
        .send((
            htui_store::connect::LAUNCH_GENERATION,
            ConnEvent::Failed("refused".to_owned()),
        ))
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
        since: Some(Utc::now()),
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

/// A store over a pool that has never dialled, standing in for the one a stale dial carries.
///
/// `PgStore::lazy` opens no socket, so an event built with it is the *shape* of an `Online` from a
/// server this session has left without needing a second server to leave.
fn unreachable_store() -> PgStore {
    let identity = Identity {
        box_id: BoxId::new(),
        hostname: "HTUI-TEST".to_owned(),
    };
    PgStore::lazy(
        "postgres://nobody:nothing@127.0.0.1:1/none",
        &identity,
        Duration::from_millis(250),
    )
    .expect("a lazy pool opens no socket")
}

/// Waits until the worker has taken every queued event off the connect channel.
///
/// The permit a `send` takes is returned when `recv` hands the message over, so
/// `capacity() == max_capacity()` is "the loop has consumed it" — an assertion about the channel
/// rather than a sleep long enough to be a guess. Every case that uses this holds its dial open on
/// a [`SilentServer`], so the injected event is the only one in flight.
async fn drained(events: &mpsc::Sender<(u64, ConnEvent)>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while events.capacity() < events.max_capacity() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the worker never took the injected event"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A worker over a fresh mirror with a dial that cannot answer, plus the connect channel.
///
/// The timeout is a minute and the DSN names a socket that accepts and says nothing, so the dial
/// `SetDsn` spawns at step (8) is still in flight for the whole of the case: anything the
/// assertions see came from the injected event and from nothing else.
async fn stalled_worker(
    root: &tempfile::TempDir,
    cache: &CacheStore,
    server: &SilentServer,
    database: &str,
) -> (Worker, mpsc::Sender<(u64, ConnEvent)>, String) {
    let mut started = Started::detached(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    });
    started.connect = Some(context(root, Duration::from_secs(60), false));
    let events = started.events_tx.clone();
    let mut worker = Worker::spawn(started);

    let dsn = server.dsn(database);
    let fingerprint = dsn.fingerprint();
    let after = connection(worker.ask(StoreRequest::SetDsn(dsn)).await);
    assert_eq!(
        after
            .mirror
            .as_ref()
            .expect("the new mirror is open")
            .db_fingerprint,
        fingerprint,
        "the swap happened, so the generation has moved"
    );
    assert_eq!(after.last_attempt, None, "a swap forgets the last dial");

    (worker, events, fingerprint)
}

/// The launch dial's `Online` is dropped after a `SetDsn` (review HIGH-1 gap (a)).
///
/// [`htui_store::connect::start`] spawns its own dial **before** the worker exists and sends the
/// outcome straight on `events_tx`; it never passes through `spawn_dial`, so no send-side check
/// can see it. It reports under [`htui_store::connect::LAUNCH_GENERATION`], and a `SetDsn` has
/// left that generation behind. Without the check at consumption, `go_online` installs the old
/// server's `PgStore` over the **new** mirror and the refresher writes one database's rows into
/// another's `cache.sqlite` (PRD `:373`).
///
/// This is also the shape of gap (b): an event that reached the queue without being filtered on
/// the way in — which is what a dial finishing in the instant before the `fetch_add` produces.
#[tokio::test]
async fn a_launch_dial_online_is_dropped_after_a_set_dsn() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-launch-online").await;
    let mut server = SilentServer::start().await;
    let (mut worker, events, fingerprint) =
        stalled_worker(&root, &cache, &server, "launch-online").await;

    events
        .send((
            htui_store::connect::LAUNCH_GENERATION,
            ConnEvent::Online(unreachable_store()),
        ))
        .await
        .expect("the worker is listening");
    drained(&events).await;

    let snapshot = worker.info().await;
    assert_eq!(
        snapshot.label, "connecting",
        "the old server's store was installed: {snapshot:?}"
    );
    assert_eq!(
        snapshot
            .mirror
            .as_ref()
            .expect("still mirrored")
            .db_fingerprint,
        fingerprint,
        "and it would have been read through the new DSN's mirror"
    );
    assert_eq!(
        snapshot.last_attempt, None,
        "a dropped event is not this server's attempt"
    );

    server.release();
    worker.shutdown().await;
    cache.close().await;
    drop(root);
}

/// The same shape carrying a pending-migration store: nothing is held aside for the prompt.
///
/// The count and the store belong to the **old** server's schema, so a `y` answered against them
/// would run this binary's migrations on a database the session has left. `held` is not readable
/// from a test, and `ApplyMigrations` is what reads it: with nothing held the loop's arm does not
/// fire and `try_serve` answers `applied: 0` (`store_worker.rs:989`).
#[tokio::test]
async fn a_launch_dial_migrations_pending_is_dropped_after_a_set_dsn() {
    let _keyring = common::mock_keyring().await;
    let (root, cache) = mirror("connection-launch-pending").await;
    let mut server = SilentServer::start().await;
    let (mut worker, events, _) = stalled_worker(&root, &cache, &server, "launch-pending").await;

    events
        .send((
            htui_store::connect::LAUNCH_GENERATION,
            ConnEvent::MigrationsPending(unreachable_store(), 3),
        ))
        .await
        .expect("the worker is listening");
    drained(&events).await;

    let StoreReply::StoreState {
        migrations_pending, ..
    } = worker.ask(StoreRequest::StoreState).await
    else {
        panic!("wrong reply variant")
    };
    assert_eq!(
        migrations_pending, None,
        "the old server's schema opened the prompt"
    );

    let StoreReply::MigrationsApplied { applied } = worker.ask(StoreRequest::ApplyMigrations).await
    else {
        panic!("wrong reply variant")
    };
    assert_eq!(applied, 0, "nothing was held aside to apply");
    assert_eq!(worker.info().await.last_attempt, None);

    server.release();
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
        since: Some(Utc::now()),
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
        since: Some(Utc::now()),
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

// -------------------------------------------------------------------------------------------
// Settings > Connection: the section (T3; D8, D15-D17, D19)
// -------------------------------------------------------------------------------------------

/// A password that appears in no constant this section renders, so `contains` finding it means
/// the section leaked it and nothing else.
const SECRET: &str = "s3cr3tword";

/// The DSN the editor cases type. Nineteen characters, so the masked count is assertable.
const TYPED: &str = "postgres://u:pw@h/d";

/// A fixed clock, so a snapshot of the Mirror row is the same on every run.
fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 15, hour, minute, 0)
        .single()
        .expect("a real instant")
}

/// The mirror the Mirror row reads, with a fingerprint long enough to be truncated to twelve.
fn mirror_info() -> MirrorInfo {
    MirrorInfo {
        db_fingerprint: "0f1e2d3c4b5a69788796a5b4c3d2e1f0".to_owned(),
        schema_version: 3,
        built_at: at(9, 30),
        last_full_refresh_at: Some(at(10, 15)),
    }
}

/// A box with a DSN in its keyring and a mirror behind it: the section's ordinary screen.
fn stored_snapshot() -> ConnectionSnapshot {
    ConnectionSnapshot {
        label: "offline \u{b7} 0s".to_owned(),
        dsn_stored: Some(true),
        dsn_summary: Some("postgres://htui@db.example:5432/htui \u{b7} sslmode=require".to_owned()),
        mirror: Some(mirror_info()),
        last_attempt: Some(Attempt {
            at: at(10, 20),
            outcome: AttemptOutcome::Failed("connection refused".to_owned()),
        }),
        offline: false,
    }
}

/// The box this milestone exists for: a keyring with nothing in it (D6, D8).
fn empty_snapshot() -> ConnectionSnapshot {
    ConnectionSnapshot {
        label: "offline \u{b7} 0s".to_owned(),
        dsn_stored: Some(false),
        dsn_summary: None,
        mirror: Some(mirror_info()),
        last_attempt: None,
        offline: false,
    }
}

/// `--demo`: no keyring was consulted and there is no mirror (D10).
fn demo_snapshot() -> ConnectionSnapshot {
    ConnectionSnapshot {
        label: "memory".to_owned(),
        dsn_stored: None,
        dsn_summary: None,
        mirror: None,
        last_attempt: None,
        offline: false,
    }
}

/// A bench and a section with `snapshot` already delivered as the read's reply.
async fn bench_with(snapshot: &ConnectionSnapshot) -> (SectionBench, ConnectionSection) {
    let bench = SectionBench::new().await;
    let mut section = ConnectionSection::new();
    feed(&bench, &mut section, snapshot);
    (bench, section)
}

/// Hands the section one read's reply and drops whatever it emitted.
fn feed(bench: &SectionBench, section: &mut ConnectionSection, snapshot: &ConnectionSnapshot) {
    bench.reply(section, &StoreReply::Connection(snapshot.clone()));
    let _ = bench.drained();
}

/// Types `text` one key at a time, as a paste arrives (ANA-10 §4.9 (4): a paste is a burst of
/// `Char` events, not a separate code path).
fn typed(bench: &SectionBench, section: &mut ConnectionSection, text: &str) {
    for c in text.chars() {
        assert_eq!(
            bench.key(section, &c.to_string()),
            Handled::Consumed,
            "`{c}` is a character while the field is open, not a section key"
        );
    }
}

/// The section's own frame, at the width every snapshot in this crate is pinned to.
fn frame_of(bench: &SectionBench, section: &ConnectionSection) -> String {
    bench.render_section(section, 100)
}

/// The hint line: the last row of the section's own layout.
fn hint_of(frame: &str) -> String {
    frame
        .lines()
        .nth(29)
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// The frame as one line, so an assertion about a sentence is not defeated by where it wrapped.
fn flattened(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every `Action::Store` the section emitted since the last drain.
fn requested(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// The five sections in the product's own registration order (D19).
fn sections() -> Vec<Box<dyn SettingsSection>> {
    vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
        Box::new(PromptSection::new()),
        Box::new(ConnectionSection::new()),
    ]
}

/// A two-tab shell over `backend`: Backlog on `1`, Settings on `2`.
///
/// Registered by hand rather than through `register_all` on purpose: `register_all` also names
/// the workspace switcher as the startup overlay, and a backend with no workspaces in it would
/// open the switcher over every frame these cases want to read. The product's own registration is
/// pinned separately, by `the_product_registers_connection_after_prompt`.
fn shell(backend: Backend) -> Harness {
    Harness::over_backend(backend)
        .with_tab(Box::new(BacklogTab::new()))
        .with_tab(Box::new(SettingsTab::with_sections(sections())))
}

// --- the masked field ------------------------------------------------------------------------

/// The hard rule (`R-TUI-8`, `R-SEC-2`): what is typed reaches neither the frame nor a `Debug`.
///
/// Both halves matter. The frame is what a shoulder or a screen recording sees; the `Debug` is
/// what one `tracing::debug!` of a section puts in the `--log` file, which is a *file*, and the
/// requirement's words are "not echoed, **not logged**, not written to any file".
#[tokio::test]
async fn the_field_renders_dots_and_a_count_never_the_text() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    // The empty snapshot opened the editor itself (D8); this is the screen the redirect lands on.
    assert!(section.captures_input(), "the field is open");
    typed(&bench, &mut section, TYPED);

    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("\u{2022}\u{2022}\u{2022}"),
        "one dot per character: {frame}"
    );
    assert!(frame.contains("(19)"), "and the count beside them: {frame}");
    assert!(
        !frame.contains("postgres://"),
        "nothing recoverable is drawn: {frame}"
    );
    assert!(!frame.contains("pw"), "least of all the password: {frame}");

    let printed = format!("{section:?}");
    assert!(
        !printed.contains("postgres://") && !printed.contains("pw"),
        "and no `Debug` of the section prints the buffer: {printed}"
    );
    assert!(
        printed.contains("len: 19"),
        "the length is legible, the text is not: {printed}"
    );
}

/// D3: there is no reveal toggle, and a section that held a secret has none to hold after `Esc`.
#[tokio::test]
async fn esc_disposes_the_field() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    typed(&bench, &mut section, TYPED);

    assert_eq!(bench.key(&mut section, "Esc"), Handled::Consumed);
    assert!(!section.captures_input(), "the editor is gone");
    assert!(
        requested(&bench).is_empty(),
        "and nothing was sent on the way out"
    );

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("(0)"),
        "the reopened field is empty, not the one that was abandoned: {frame}"
    );
}

/// The other disposal point (ANA-10 §4.9 (6)): entering another workspace.
#[tokio::test]
async fn a_scope_change_disposes_the_field() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    typed(&bench, &mut section, TYPED);

    section.on_scope_change(&Scope {
        workspace_id: htui_core::model::WorkspaceId::default(),
        project_ids: Vec::new(),
    });

    assert!(!section.captures_input(), "the editor went with the scope");
    assert!(
        !format!("{section:?}").contains("pw"),
        "and took the buffer with it"
    );
}

/// D17 / ANA-10 §4.9 (2): `l`, `h`, `[` and `]` are characters while the field is open.
///
/// This is the fix the hook shipped in milestone 3 was built for: the Settings tab consumes those
/// four for section cycling *before* a section sees them, unless the section says it is taking
/// typed text. A DSN containing an `l` is not exotic — `postgres://localhost/...` has three.
#[tokio::test]
async fn l_h_and_brackets_are_characters_while_editing() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    typed(&bench, &mut section, "postgres://");
    assert!(frame_of(&bench, &section).contains("(11)"));

    assert!(
        section.captures_input(),
        "which is what the tab checks before it takes `l`"
    );
    typed(&bench, &mut section, "lh[]");

    let frame = frame_of(&bench, &section);
    assert!(
        frame.contains("(15)"),
        "all four landed in the buffer: {frame}"
    );
}

// --- validation ------------------------------------------------------------------------------

/// D2, and the milestone's second hard rule: a refused DSN emits **no request at all**.
///
/// The parse runs in the section, so a string that is not a DSN never becomes one, never reaches
/// the keyring, never reaches sqlx's parser — which would log an unrecognised parameter's value —
/// and never reaches the worker's `Failed` path, which would put the request's name on the status
/// line for a mistake that is nobody's business but the typist's.
#[tokio::test]
async fn a_refused_dsn_emits_no_store_request() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    typed(&bench, &mut section, "nope");
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);

    assert!(
        bench.drained().is_empty(),
        "not one action, not even an error on the status line"
    );

    let frame = frame_of(&bench, &section);
    assert!(frame.contains("not a URL"), "the fixed sentence: {frame}");
    assert!(section.captures_input(), "and the field is still open");
    assert!(
        frame.contains("(0)"),
        "over an empty buffer, so the retype starts clean: {frame}"
    );
}

/// D2: five inputs, five fixed sentences, and not a fragment of any of them on screen.
#[tokio::test]
async fn each_refusal_is_one_of_five_sentences() {
    let cases = [
        ("nope", "not a URL"),
        ("postgres:///htui", "no host"),
        ("postgres://h/d?sslmode=maybe", "unsupported sslmode"),
        ("postgres://h:70000/d", "port out of range"),
        ("postgres://h/d?foo=bar", "unrecognised parameter"),
    ];

    for (input, sentence) in cases {
        let (bench, mut section) = bench_with(&empty_snapshot()).await;
        typed(&bench, &mut section, input);
        bench.key(&mut section, "Enter");

        assert!(
            bench.drained().is_empty(),
            "`{input}` sent something it should not have"
        );
        let frame = flattened(&frame_of(&bench, &section));
        assert!(frame.contains(sentence), "`{input}`: {frame}");
        for (_, other) in cases {
            assert!(
                other == sentence || !frame.contains(other),
                "`{input}` showed `{other}` as well as `{sentence}`"
            );
        }
        assert!(
            !frame.contains("70000") && !frame.contains("maybe") && !frame.contains("foo"),
            "`{input}` echoed part of what was typed: {frame}"
        );
    }
}

/// The accepting half: one `SetDsn` carrying the newtype, and the section says it is waiting.
#[tokio::test]
async fn a_valid_dsn_emits_set_dsn_and_marks_busy() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    typed(
        &bench,
        &mut section,
        &format!("postgres://htui:{SECRET}@db.example:5432/htui?sslmode=require"),
    );
    bench.key(&mut section, "Enter");

    let sent = requested(&bench);
    assert_eq!(sent.len(), 1, "exactly one request: {sent:?}");
    assert!(
        matches!(sent.first(), Some(StoreRequest::SetDsn(_))),
        "{sent:?}"
    );
    assert!(
        !format!("{sent:?}").contains(SECRET),
        "and the request prints redacted: {sent:?}"
    );

    assert!(!section.captures_input(), "the field closed behind it");
    let frame = frame_of(&bench, &section);
    assert!(
        hint_of(&frame).ends_with("set_dsn in flight"),
        "the hint says what is out: {frame}"
    );

    // A second write is refused until the first answers (the one-write-in-flight guard, D5).
    bench.key(&mut section, "e");
    assert!(!section.captures_input(), "{:?}", section);
    assert!(
        flattened(&frame_of(&bench, &section)).contains("set_dsn"),
        "and says why"
    );
}

/// An `Enter` over nothing is not a write: there is no DSN to store and the stored one stays.
#[tokio::test]
async fn an_empty_field_stores_nothing() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    bench.key(&mut section, "Enter");

    assert!(bench.drained().is_empty());
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("nothing typed; the stored DSN is unchanged"),
        "{frame}"
    );
}

// --- the rows --------------------------------------------------------------------------------

/// D16: what a box with a DSN shows, and D15's rule — the label is rendered, never parsed.
#[tokio::test]
async fn the_rows_show_the_snapshot_and_derive_nothing() {
    let (bench, section) = bench_with(&stored_snapshot()).await;
    let frame = flattened(&frame_of(&bench, &section));

    assert!(frame.contains("offline \u{b7} 0s"), "{frame}");
    assert!(
        frame.contains(
            "stored \u{2014} postgres://htui@db.example:5432/htui \u{b7} sslmode=require"
        ),
        "the DSN row is the summary and nothing else: {frame}"
    );
    assert!(
        frame.contains("last dial 10:20:00: failed: connection refused"),
        "the last dial is shown in the app, not only in a log: {frame}"
    );
    assert!(
        frame.contains("0f1e2d3c4b5a \u{b7} built 2026-09-15 09:30"),
        "the fingerprint is truncated to twelve: {frame}"
    );
    assert!(
        frame.contains("last full refresh 2026-09-15 10:15 \u{b7} schema 3"),
        "{frame}"
    );
    assert!(!frame.contains("0f1e2d3c4b5a6"), "and no further: {frame}");

    insta::assert_snapshot!("stored", frame_of(&bench, &section));
}

/// B-7: a DSN one `--set-dsn` wrote raw is reported as stored, without a summary it cannot build.
#[tokio::test]
async fn an_unreadable_stored_dsn_says_so_rather_than_guessing() {
    let mut snapshot = stored_snapshot();
    snapshot.dsn_summary = None;
    let (bench, section) = bench_with(&snapshot).await;

    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("stored \u{2014} not readable by this build; e replaces it"),
        "{frame}"
    );
}

/// D10: a `--demo` session has no keyring and no mirror, and the rows say so rather than lying.
#[tokio::test]
async fn a_demo_snapshot_never_opens_the_editor() {
    let (bench, section) = bench_with(&demo_snapshot()).await;

    assert!(
        !section.captures_input(),
        "nothing to type a DSN into: {section:?}"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("n/a in a demo session"), "{frame}");
    assert!(frame.contains("memory"), "{frame}");
    assert!(frame.contains("no dial yet this session"), "{frame}");
}

/// D8: the editor opens itself once for an empty keyring, and not again after the fourth-tick
/// re-read that lands a minute later.
#[tokio::test]
async fn an_empty_snapshot_opens_the_editor_once() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    assert!(section.captures_input(), "it opened itself");
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("no DSN is stored; type one and press Enter"),
        "{frame}"
    );
    assert!(frame.contains("not stored"), "and the row says so: {frame}");
    insta::assert_snapshot!("empty", frame_of(&bench, &section));

    bench.key(&mut section, "Esc");
    assert!(!section.captures_input());

    feed(&bench, &mut section, &empty_snapshot());
    assert!(
        !section.captures_input(),
        "a re-read does not reopen a field the user closed: {section:?}"
    );
}

/// The editor over a box that already has a DSN: `e` replaces rather than reveals.
#[tokio::test]
async fn e_opens_a_replacement_field_over_a_stored_dsn() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    typed(&bench, &mut section, TYPED);

    let frame = frame_of(&bench, &section);
    assert!(frame.contains("(19)"), "{frame}");
    assert!(
        frame.contains("Enter store"),
        "and the hint is the editor's: {frame}"
    );
    assert!(
        frame.contains("Enter replaces the stored DSN"),
        "the guide line is the one for a box that has a DSN, not the empty box's: {frame}"
    );
    assert!(
        !frame.contains("no DSN is stored"),
        "which would be the screen lying about the state it is reporting: {frame}"
    );
    insta::assert_snapshot!("editor", frame);
}

/// D16: `j`/`k` move within the four rows and stop at the ends.
#[tokio::test]
async fn j_and_k_move_between_the_four_rows() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    for _ in 0..8 {
        assert_eq!(bench.key(&mut section, "j"), Handled::Consumed);
    }
    // The cursor is a style, so the assertion is that `Enter` now means the Rebuild row (B-9).
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);
    assert!(
        flattened(&frame_of(&bench, &section)).contains("Rebuild the mirror?"),
        "eight `j` stopped on the last row rather than wrapping"
    );
}

// --- the two confirmations ---------------------------------------------------------------------

/// D16: `c` asks before it clears, and `n` is an answer.
#[tokio::test]
async fn c_needs_a_confirmation() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;

    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(
        bench.drained().is_empty(),
        "nothing is sent by the question"
    );
    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("Remove the DSN from the keyring?"),
        "{frame}"
    );
    assert!(
        frame.contains("the next launch starts offline"),
        "and says what it costs: {frame}"
    );
    assert!(section.captures_input(), "the question is modal");

    bench.key(&mut section, "n");
    assert!(!section.captures_input());
    assert!(bench.drained().is_empty(), "`n` sends nothing either");

    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let sent = requested(&bench);
    assert!(
        matches!(sent.as_slice(), [StoreRequest::ClearDsn]),
        "one clear, and only after the `y`: {sent:?}"
    );
}

/// There is nothing to clear on a box with no DSN, so `c` refuses instead of asking.
#[tokio::test]
async fn c_refuses_when_nothing_is_stored() {
    let (bench, mut section) = bench_with(&empty_snapshot()).await;
    bench.key(&mut section, "Esc");

    assert_eq!(bench.key(&mut section, "c"), Handled::Consumed);
    assert!(bench.drained().is_empty());
    assert!(!section.captures_input(), "no question was asked");
    assert!(
        flattened(&frame_of(&bench, &section)).contains("not stored"),
        "{section:?}"
    );
}

/// D14: the rebuild confirmation names **both** lists, and a second `y` is inert.
///
/// The copy is derived from `CacheStore::rebuild`'s own body, not from memory, and this is the
/// test that keeps it that way: if the action is ever extended to clear something else, one of
/// these two lists stops being true and this case is where it shows.
#[tokio::test]
async fn rebuild_needs_a_confirmation_and_names_both_lists() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    assert_eq!(bench.key(&mut section, "R"), Handled::Consumed);
    assert!(bench.drained().is_empty());

    let frame = flattened(&frame_of(&bench, &section));
    for survives in ["schema_version", "db_fingerprint", "built_at", "pending/"] {
        assert!(frame.contains(survives), "`{survives}` survives: {frame}");
    }
    for goes in ["16 mirrored tables", "cache_cursor", "last_full_refresh_at"] {
        assert!(frame.contains(goes), "`{goes}` goes: {frame}");
    }
    assert!(
        frame.contains("Survives:") && frame.contains("Goes:"),
        "and the two are told apart: {frame}"
    );
    insta::assert_snapshot!("confirm", frame_of(&bench, &section));

    bench.key(&mut section, "y");
    let sent = requested(&bench);
    assert!(
        matches!(sent.as_slice(), [StoreRequest::RebuildCache]),
        "{sent:?}"
    );
    assert!(
        flattened(&frame_of(&bench, &section)).contains("rebuilding"),
        "the question became a report"
    );

    bench.key(&mut section, "y");
    assert!(
        bench.drained().is_empty(),
        "a second `y` on a rebuild already out sends nothing"
    );
}

/// B-9: `Enter` on the Rebuild row is `R`, so the row is not a label with no key.
#[tokio::test]
async fn enter_on_the_rebuild_row_is_r() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    for _ in 0..3 {
        bench.key(&mut section, "j");
    }
    assert_eq!(bench.key(&mut section, "Enter"), Handled::Consumed);

    assert!(
        flattened(&frame_of(&bench, &section)).contains("Rebuild the mirror?"),
        "{section:?}"
    );
    assert!(bench.drained().is_empty());
}

/// A rebuild needs a mirror, and `--demo` has none.
#[tokio::test]
async fn rebuild_refuses_in_a_demo_session() {
    let (bench, mut section) = bench_with(&demo_snapshot()).await;
    assert_eq!(bench.key(&mut section, "R"), Handled::Consumed);

    assert!(bench.drained().is_empty());
    assert!(!section.captures_input(), "no question was asked");
    assert!(
        flattened(&frame_of(&bench, &section)).contains("n/a in a demo session"),
        "{section:?}"
    );
}

// --- replies ------------------------------------------------------------------------------------

/// D12: the `--offline` notice says what happened rather than leaving `offline \u{b7} 0s` to be
/// read as a failure.
#[tokio::test]
async fn the_offline_notice_after_set_dsn() {
    for offline in [false, true] {
        let (bench, mut section) = bench_with(&empty_snapshot()).await;
        typed(&bench, &mut section, "postgres://h:5432/d");
        bench.key(&mut section, "Enter");
        let _ = bench.drained();

        let mut answer = stored_snapshot();
        answer.offline = offline;
        feed(&bench, &mut section, &answer);

        let hint = hint_of(&frame_of(&bench, &section));
        if offline {
            assert_eq!(
                hint,
                "stored; this session was started with --offline, so it takes effect on the next launch",
                "the sentence wins the line when the keys do not fit beside it"
            );
        } else {
            assert!(hint.ends_with("\u{b7} stored"), "{hint}");
        }
    }
}

/// D13: the clear reply says what it did **and** what it deliberately did not do.
#[tokio::test]
async fn clear_reply_says_the_session_keeps_its_connection() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    let mut answer = stored_snapshot();
    answer.dsn_stored = Some(false);
    answer.dsn_summary = None;
    answer.label = "online".to_owned();
    feed(&bench, &mut section, &answer);

    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains(
            "the DSN is gone from the keyring \u{2014} this session keeps its current connection until you quit"
        ),
        "{frame}"
    );
    assert!(
        !section.captures_input(),
        "and the clear did not reopen the editor over its own report: {section:?}"
    );
}

/// D14's reply: the mirror is empty and the section says what refills it.
#[tokio::test]
async fn the_rebuild_reply_says_what_refills_the_mirror() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    bench.key(&mut section, "R");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    let mut answer = stored_snapshot();
    answer.mirror = Some(MirrorInfo {
        last_full_refresh_at: None,
        ..mirror_info()
    });
    feed(&bench, &mut section, &answer);

    let frame = flattened(&frame_of(&bench, &section));
    assert!(
        frame.contains("mirror rebuilt; the next refresh pass refills it"),
        "{frame}"
    );
    assert!(
        frame.contains("last full refresh never"),
        "and the row agrees with it: {frame}"
    );
    assert!(!section.captures_input(), "the question is answered");
}

/// A refused **read** leaves the section with no rows to trust, and says so instead of showing
/// rows nothing has confirmed since the outage started.
#[tokio::test]
async fn a_failed_read_blocks_e() {
    let bench = SectionBench::new().await;
    let mut section = ConnectionSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: READ_NAME,
            message: NO_WORKER.to_owned(),
        },
    );
    let _ = bench.drained();

    let frame = flattened(&frame_of(&bench, &section));
    assert!(frame.contains("connection info is unavailable"), "{frame}");
    assert!(frame.contains(NO_WORKER), "{frame}");

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    assert!(
        !section.captures_input(),
        "a field over rows nobody can see: {section:?}"
    );
}

/// A refused **write** is the seam's own sentence, verbatim, and the section is ready for the
/// next key.
#[tokio::test]
async fn a_failed_write_shows_the_seams_sentence() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    bench.key(&mut section, "R");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "rebuild_cache",
            message: DEMO_SESSION.to_owned(),
        },
    );

    let frame = frame_of(&bench, &section);
    assert!(flattened(&frame).contains(DEMO_SESSION), "{frame}");
    assert!(
        !hint_of(&frame).contains("in flight"),
        "and nothing is still out: {frame}"
    );
    assert!(
        !section.captures_input(),
        "the question it refused is closed: {section:?}"
    );
}

/// `r` is the way back from a lost reply, so it is allowed whatever else is going on (M3's rule).
#[tokio::test]
async fn r_re_reads_and_is_never_refused() {
    let (bench, mut section) = bench_with(&stored_snapshot()).await;
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = bench.drained();

    // `clear_dsn` is out, so `e` would be refused. `r` is deliberately not on that path.
    assert_eq!(bench.key(&mut section, "r"), Handled::Consumed);
    assert!(
        matches!(requested(&bench).as_slice(), [StoreRequest::ConnectionInfo]),
        "one read, with a write still out"
    );

    // A question, on the other hand, is modal over everything — `r` included. It is swallowed
    // rather than passed to the shell, so a `q` at a question cannot quit the application either.
    feed(&bench, &mut section, &stored_snapshot());
    bench.key(&mut section, "R");
    assert_eq!(
        bench.key(&mut section, "r"),
        Handled::Consumed,
        "swallowed, not offered to the shell"
    );
    assert!(
        bench.drained().is_empty(),
        "and nothing is asked behind a question on screen"
    );
}

/// The read every section names for itself (B-6), so the tab activating populates it.
#[tokio::test]
async fn the_section_asks_for_the_connection_and_nothing_else() {
    let section = ConnectionSection::new();
    let scope = Scope {
        workspace_id: htui_core::model::WorkspaceId::default(),
        project_ids: Vec::new(),
    };
    let wants = section.wants_requests(&scope);

    assert_eq!(wants.len(), 1, "{wants:?}");
    assert_eq!(wants[0].name(), READ_NAME);
    assert_eq!(section.id(), SectionId("connection"));
    assert_eq!(section.title(), "Connection");
}

// --- the shell: the focus action and the redirect ------------------------------------------------

/// D7: `FocusSection` reaches the section from a tab that is not even Settings.
#[tokio::test]
async fn focus_section_selects_from_any_tab() {
    let mut harness = shell(Backend::memory(MemStore::demo()));
    harness.settle().await;
    assert!(
        !harness.render().contains("Rebuild cache"),
        "the shell starts on Backlog"
    );

    harness.app().update(Action::Tab(TabAction::FocusSection(
        SettingsTab::ID,
        ConnectionSection::ID,
    )));
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("Rebuild cache"),
        "one action reached the tab and the section inside it: {frame}"
    );

    // An id nothing registers moves nothing: the shell must not die because a build dropped a
    // section, and it must not silently land somewhere else either.
    harness.app().update(Action::Tab(TabAction::FocusSection(
        SettingsTab::ID,
        SectionId("nothing-registers-this"),
    )));
    harness.settle().await;
    assert!(
        harness.render().contains("Rebuild cache"),
        "the active section did not move"
    );
}

/// D6: a box with an empty keyring lands on the field that fixes it — once per session.
#[tokio::test]
async fn the_redirect_fires_once() {
    let _keyring = common::mock_keyring().await;
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "connection-redirect",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");

    let mut harness = shell(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    });
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("no DSN is stored; type one and press Enter"),
        "the shell steered to the field without the user finding the tab: {frame}"
    );
    assert!(
        frame.contains("typed text is never shown"),
        "and it is open: {frame}"
    );

    // The user closes it and goes back to work.
    harness.key("Esc");
    harness.key("1");
    harness.settle().await;
    assert!(
        !harness.render().contains("Rebuild cache"),
        "back on Backlog"
    );

    // The shell re-reads under `Origin::App`, as the fourth tick does. It must not fight the user.
    harness
        .app()
        .update(Action::Store(StoreRequest::ConnectionInfo));
    harness.settle().await;
    assert!(
        !harness.render().contains("Rebuild cache"),
        "the redirect is once per session, not once per reply: {}",
        harness.render()
    );

    cache.close().await;
    drop(root);
}

/// D10: `--demo` answers `dsn_stored: None`, which is not "no DSN" and must not redirect.
#[tokio::test]
async fn a_demo_session_never_redirects() {
    let mut harness = shell(Backend::memory(MemStore::demo()));
    harness.settle().await;

    let frame = harness.render();
    assert!(
        !frame.contains("Rebuild cache") && !frame.contains("no DSN is stored"),
        "a demo session has nothing to fix and is left where it was: {frame}"
    );
}

/// D19: the product registers `Connection` **after** `Prompt`, so the strip reads
/// `Agents  Hierarchy  Kinds  Prompt  Connection` and four `l` reach it.
#[tokio::test]
async fn the_product_registers_connection_after_prompt() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    harness.settle().await;
    harness.key("3");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("Agents") && frame.contains("Prompt") && frame.contains("Connection"),
        "five sections in the strip: {frame}"
    );

    for _ in 0..4 {
        harness.key("l");
    }
    harness.settle().await;
    assert!(
        harness.render().contains("Rebuild cache"),
        "the fifth `l` would wrap; four reach the last section"
    );
}
