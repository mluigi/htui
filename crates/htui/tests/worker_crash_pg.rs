//! MOD-24 D2 against live Postgres: a worker process killed mid-walk, and the one that follows.
//!
//! Each case re-executes **this test binary** as the worker: `std::env::current_exe()` with
//! `--exact crash_child --nocapture --test-threads=1` and the case's `HTUI_TEST_CRASH_*`
//! variables, which turn [`crash_child`] (a no-op without them) into `htui worker`'s loop
//! (`htui_worker::worker::run`) over the case's throwaway database, registered as the fixture box
//! `ids::BOX` (`testkit::fixture_box_store`). Its parts are the real `GixIsolator` over a
//! temporary git repository whose scratch root lies outside it, so the trees survive the kill as a
//! box's filesystem does; a fake verifier; and a scripted transport whose sessions commit one file
//! each, so a capture records an `after` commit (blueprint B8, A-3).
//!
//! The kill is a real `SIGKILL` (`Child::kill`), taken when the child has reached the point the
//! case names: the marker file `htui_orch::kill_point` writes (`documented`, `captured`,
//! `command_picked`), the marker a stalled session writes, or a `session_event` row in Postgres.
//! No destructor runs, no lease is given back, nothing shuts down. A second child (a new process,
//! so a new lease owner) then waits out the dead lease (`lease_ttl_seconds` is 2 for the case) and
//! recovers. Killing in process would not be a crash: an aborted loop task leaves its walks
//! heartbeating (plan fact 7).
//!
//! Every wait is a bounded poll of a marker or a row, never a sleep before a kill, and every poll
//! fails at once, with the child's log, when the child exited early. Children are killed and reaped
//! on every exit path ([`Reaped`]); a restarted child stops when its stdin closes.
//!
//! The file is ungated like `worker_pg.rs`: each case prints `testkit::SKIP` and returns with
//! `HTUI_TEST_DATABASE_URL` unset (and panics instead when `CI` is set).

#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use htui::run_worker::{LiveChats, OrchReply, OrchRequest, RunServed, StepAuthor, TuiRuns as _};
use htui::store_worker::{Origin, RequestEnvelope, StoreReply, StoreRequest};
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, PermissionOption, PermissionOptionKind,
    PermissionRequestEvent, StopReason, ToolCallEvent, ToolKind,
};
use htui_agent::fake::FakeDriver;
use htui_agent::registry::{DriverFactory, TransportBuilder};
use htui_core::fixtures::{edit_agent, ids};
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, BoxEdit, CancelRequest, DocumentId, EventKind, Executor,
    ItemId, NewDocument, NewRepo, RepoId, RunCommandStatus, RunId, RunMode, RunStatus, RunStep,
    SnapshotPhase, StepId, StepPermission, StepStatus, Transport,
};
use htui_core::store::{CasOutcome, ReadStore as _, WriteStore as _};
use htui_orch::fake::{FakeIsolator, FakeVerifier};
use htui_orch::kill_point::{MARK_VAR, MISCONFIGURED_EXIT, PARK_EXPIRED_EXIT, POINT_VAR};
use htui_orch::{
    Command, CommandOutcome, GixIsolator, Isolator, IsolatorConfig, RepoCheckout, Verifier,
};
use htui_store::{Backend, CacheStore, PgStore, Registration, testkit};
use htui_worker::worker::{self, WorkerConfig};
use htui_worker::{Role, RunRuntime, Unaddressed};
use serde_json::json;

/// How long a case waits for a marker, a row or an exit: a child's start, four git-backed phases
/// and a lapsed lease fit with room to spare.
const PATIENCE: Duration = Duration::from_secs(60);

/// `worker_pg.rs`'s fast loop: a 50 ms sweep, a short box beat, no walk grace.
const CONFIG: WorkerConfig = WorkerConfig {
    poll: Duration::from_millis(50),
    box_beat: Duration::from_secs(1),
    grace: Duration::ZERO,
};

/// `app_setting.lease_ttl_seconds` for the case, so a dead child's lease lapses in two seconds
/// (blueprint H-9: a live walk renews every TTL/3 and fences itself after two missed beats).
const TTL_SECONDS: i32 = 2;

/// The child entry's name; `--exact` matches a top-level fn's bare name only (blueprint H-14).
const CHILD: &str = "crash_child";
/// `TestDb.url`: the database the child connects to, and the child entry's switch.
const DSN_VAR: &str = "HTUI_TEST_CRASH_DSN";
/// The seeded primary repo's `RepoId`.
const REPO_VAR: &str = "HTUI_TEST_CRASH_REPO";
/// The temporary git repository: the repo's checkout on this box.
const CHECKOUT_VAR: &str = "HTUI_TEST_CRASH_CHECKOUT";
/// The isolator's scratch root, outside the checkout.
const SCRATCH_VAR: &str = "HTUI_TEST_CRASH_SCRATCH";
/// What the child's sessions play: [`Act::name`].
const SCRIPT_VAR: &str = "HTUI_TEST_CRASH_SCRIPT";
/// `RepoCheckout.name`: the directory a tree takes under a session's cwd (blueprint A-4).
const TREE: &str = "htui";
/// The file a stalled session leaves uncommitted in its tree (K1).
const STRAY: &str = "stray.txt";

// ---------------------------------------------------------------------------------------------
// The child's parts
// ---------------------------------------------------------------------------------------------

/// `worker_pg.rs`'s author: one document of the phase's `output_kind` per step; a `review` body
/// approves, so the ungated graph walks to `done`.
#[derive(Debug)]
struct OutputAuthor;

impl StepAuthor for OutputAuthor {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument> {
        let body = if phase.name == "review" {
            "---\nverdict: approve\n---\nfine"
        } else {
            "authored"
        };
        Some(NewDocument {
            id: DocumentId::new(),
            item_id: item,
            kind: phase.output_kind.clone(),
            title: format!("{} (attempt {})", phase.output_kind, step.attempt),
            body: body.to_owned(),
            produced_by_step_id: Some(step.id),
            created_by: ids::USER,
            created_at: Utc::now(),
        })
    }
}

/// What a child's sessions play. Every session but the first commits one file and ends its turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    /// The first session too.
    Walks,
    /// The first session writes [`STRAY`] into its tree, then the marker, then never answers (K1).
    Stall,
    /// The first session plays a gated `execute` call and parks on its permission request (K2,
    /// K5): `worker_pg.rs`'s `ParksOnce`.
    Park,
}

impl Act {
    const fn name(self) -> &'static str {
        match self {
            Self::Walks => "walks",
            Self::Stall => "stall",
            Self::Park => "park",
        }
    }

    fn parse(name: &str) -> Self {
        [Self::Walks, Self::Stall, Self::Park]
            .into_iter()
            .find(|act| act.name() == name)
            .unwrap_or_else(|| panic!("unknown script `{name}`"))
    }
}

/// What one session does before its first event.
#[derive(Debug, Clone)]
enum Play {
    /// Commits `step-<step id>.txt` into its tree, then plays its one turn.
    Commit,
    /// Writes [`STRAY`], then the marker at this path, then parks forever.
    Stray(PathBuf),
    /// Nothing: the script alone (the parked permission request).
    Script,
}

/// The `acp` transport of a child: [`Act`] decides the first session, every later one commits.
#[derive(Debug)]
struct Sessions {
    act: Act,
    mark: PathBuf,
    built: AtomicBool,
}

impl TransportBuilder for Sessions {
    fn build(
        &self,
        agent: &Agent,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, htui_agent::error::DriverError> {
        let first = !self.built.swap(true, Ordering::SeqCst);
        let (script, play) = match (self.act, first) {
            (Act::Stall, true) => (one_turn(), Play::Stray(self.mark.clone())),
            (Act::Park, true) => (parks_on_a_request(), Play::Script),
            _ => (one_turn(), Play::Commit),
        };
        Ok(Box::new(Driver {
            inner: FakeDriver::new(agent.name.clone(), caps, script),
            play,
        }))
    }
}

/// One turn that ends at once.
fn one_turn() -> Script {
    Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
        stop_reason: StopReason::EndTurn,
    }))])
}

/// `worker_pg.rs`'s parked session: a `ToolCall` (flushed, `record.rs:896-901`), then a park on
/// its permission request, then the end of the turn once answered.
fn parks_on_a_request() -> Script {
    Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "run the suite".to_owned(),
            tool_kind: ToolKind::Execute,
            input: json!({ "command": "cargo test" }),
            locations: Vec::new(),
        })),
        ScriptEvent::ParkPermission(PermissionRequestEvent {
            request_id: PermissionRequestId::new("request-1"),
            tool_call_id: Some("call-1".to_owned()),
            options: vec![
                PermissionOption {
                    id: "allow-once".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                },
                PermissionOption {
                    id: "reject-once".to_owned(),
                    label: "Reject".to_owned(),
                    kind: PermissionOptionKind::RejectOnce,
                },
            ],
        }),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ])
}

/// A [`FakeDriver`] whose session learns its tree and step from the [`SessionSpec`].
#[derive(Debug)]
struct Driver {
    inner: FakeDriver,
    play: Play,
}

impl AgentDriver for Driver {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn caps(&self) -> DriverCaps {
        self.inner.caps()
    }

    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        // Blueprint H-12: the cwd is the step directory, and the repo's tree is under it.
        let tree = spec.cwd.join(TREE);
        let step = spec.step_id;
        let play = self.play.clone();
        let inner = self.inner.start(spec, prompt);
        Box::pin(async move {
            let session = inner.await?;
            Ok(match play {
                Play::Script => session,
                play => Box::new(Session {
                    inner: session,
                    tree,
                    step,
                    play: Some(play),
                }) as Box<dyn AgentSession>,
            })
        })
    }
}

/// A session that does its [`Play`] before its first event.
#[derive(Debug)]
struct Session {
    inner: Box<dyn AgentSession>,
    tree: PathBuf,
    step: StepId,
    play: Option<Play>,
}

impl AgentSession for Session {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        self.inner.session_ref()
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            match self.play.take() {
                Some(Play::Commit) => commit_step(&self.tree, self.step),
                Some(Play::Stray(mark)) => {
                    assert_tree(&self.tree);
                    std::fs::write(self.tree.join(STRAY), "never committed\n")
                        .expect("write the stray file");
                    write_marker(&mark);
                    std::future::pending::<()>().await;
                }
                Some(Play::Script) | None => {}
            }
            self.inner.next_event().await
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        self.inner.send_follow_up(text)
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        self.inner.answer_permission(request_id, answer)
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        self.inner.cancel(grace)
    }
}

/// A changed layout fails here, loudly, instead of committing nowhere (blueprint H-12).
fn assert_tree(tree: &Path) {
    assert!(
        tree.join(".git").exists(),
        "a session's tree is `<cwd>/{TREE}` and is a git worktree: {}",
        tree.display()
    );
}

/// Blueprint B8: the step's one file, committed, so its capture records an `after` commit.
fn commit_step(tree: &Path, step: StepId) {
    assert_tree(tree);
    std::fs::write(tree.join(step_file(step)), format!("step {step}\n"))
        .expect("write the step's file");
    git(tree, &["add", "-A"]);
    git(
        tree,
        &[
            "-c",
            "user.name=htui-test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-q",
            "-m",
            &format!("step {step}"),
        ],
    );
}

/// `step-<step id>.txt`: the file [`commit_step`] commits.
fn step_file(step: StepId) -> String {
    format!("step-{step}.txt")
}

/// `git -C <dir> <args>`, which must succeed; its stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("git speaks UTF-8 here")
}

/// The kill point's own marker protocol: written whole to a sibling, synced, renamed into place.
fn write_marker(mark: &Path) {
    let temp = mark.with_extension("tmp");
    let mut file = File::create(&temp).expect("create the marker");
    file.write_all(b"stalled").expect("write the marker");
    file.sync_all().expect("sync the marker");
    drop(file);
    std::fs::rename(&temp, mark).expect("rename the marker into place");
}

/// A variable the parent sets for the child.
fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is set by the parent"))
}

/// Resolves when this process's stdin is at its end: the parent closed it, or died.
async fn stdin_closed() {
    let _ = tokio::task::spawn_blocking(|| {
        let mut stdin = std::io::stdin();
        let mut buffer = [0_u8; 64];
        while matches!(stdin.read(&mut buffer), Ok(n) if n > 0) {}
    })
    .await;
}

/// MOD-24 D2: the worker a case kills. Returns at once unless a case spawned this binary with
/// [`DSN_VAR`] set; then it is `htui worker`'s loop over the case's database as the fixture box,
/// with the real isolator, until its stdin closes (or a `SIGKILL`).
#[tokio::test(flavor = "multi_thread")]
async fn crash_child() {
    let Ok(dsn) = std::env::var(DSN_VAR) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,htui_orch=info,htui_worker=info")
        .with_writer(std::io::stderr)
        .try_init();
    let host = testkit::fixture_box_store(&dsn).await;
    let repo: RepoId = var(REPO_VAR).parse().expect("a RepoId");
    let isolator = GixIsolator::new(IsolatorConfig {
        repos: BTreeMap::from([(
            repo,
            RepoCheckout {
                name: TREE.to_owned(),
                local_path: var(CHECKOUT_VAR).into(),
                is_primary: true,
            },
        )]),
        scratch_root: var(SCRATCH_VAR).into(),
        copy_exclude: Vec::new(),
        copy_max_total_bytes: 1 << 30,
        box_id: ids::BOX,
    })
    .expect("the scratch root is outside the checkout");
    let mut factory = DriverFactory::new();
    factory.register(
        "acp",
        Box::new(Sessions {
            act: Act::parse(&var(SCRIPT_VAR)),
            mark: var(MARK_VAR).into(),
            built: AtomicBool::new(false),
        }),
    );
    let runtime: RunRuntime<PgStore, Unaddressed> = RunRuntime::with_parts(
        Arc::new(isolator) as Arc<dyn Isolator>,
        Arc::new(FakeVerifier::new()) as Arc<dyn Verifier>,
        factory,
    )
    .with_author(Arc::new(OutputAuthor))
    .with_role(Role::Worker);
    worker::run(host, runtime, CONFIG, stdin_closed()).await;
}

// ---------------------------------------------------------------------------------------------
// The parent: the database, the TUI that queues, the children
// ---------------------------------------------------------------------------------------------

/// `worker_pg.rs`'s seed: the demo's agents disabled, one scripted `acp` row ready on this box, a
/// primary repo for the demo project (returned), and `FEAT-3` freed of its seeded run.
async fn seed(store: &PgStore) -> RepoId {
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        edit_agent(store, &row).await.expect("the row is disabled");
    }
    let agent_id = AgentId::new();
    let at = Utc::now();
    store
        .upsert_agent(
            &Agent {
                id: agent_id,
                name: "scripted".to_owned(),
                transport: Transport::Acp,
                billing: Billing::Subscription,
                models: Vec::new(),
                default_model: Some("sonnet".to_owned()),
                launch: json!({ "command": "unused", "args": [] }),
                settings: json!({}),
                enabled: true,
                created_at: at,
                updated_at: at,
            },
            None,
        )
        .await
        .expect("the scripted row lands");
    store
        .upsert_agent_box(&AgentBox {
            agent_id,
            box_id: store.this_box(),
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("the agent_box row lands");
    let repo = RepoId::new();
    store
        .create_repo(NewRepo {
            id: repo,
            project_id: ids::PROJECT_HTUI,
            name: TREE.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("the demo project has no repo yet");
    // `FEAT-3` is seeded `queued` under a live `RUN_2`; cancelling it moves the item to `open`.
    store
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
        .await
        .expect("the seeded run is queued and cancellable");
    repo
}

/// The database, the mirror, and the TUI's run runtime over `Backend::Online`, which only queues
/// (`worker_pg.rs`'s `Stack`, trimmed).
struct Stack {
    db: testkit::TestDb,
    _root: tempfile::TempDir,
    cache: CacheStore,
    backend: Backend,
    tui: Option<htui::run_worker::RunRuntime>,
    seq: u64,
    repo: RepoId,
}

impl Stack {
    /// The stack over a seeded demo database, or `None` (after `testkit::SKIP`) without a server.
    async fn new() -> Option<Self> {
        let db = testkit::demo_db().await?;
        let repo = seed(&db.store).await;
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = CacheStore::open(root.path(), "worker-crash-pg", PgStore::schema_version())
            .await
            .expect("a fresh mirror");
        let backend = Backend::Online {
            pg: db.store.clone(),
            cache: cache.clone(),
        };
        let tui = RunRuntime::with_parts(
            Arc::new(FakeIsolator::new()) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()) as Arc<dyn Verifier>,
            DriverFactory::new(),
        );
        Some(Self {
            db,
            _root: root,
            cache,
            backend,
            tui: Some(tui),
            seq: 0,
            repo,
        })
    }

    /// One command from the TUI, its tasks settled, and its answer.
    async fn command(&mut self, command: Command) -> CommandOutcome {
        self.seq += 1;
        let seq = self.seq;
        let tui = self.tui.as_mut().expect("the TUI is still running");
        let (replies, mut answers) = tokio::sync::mpsc::unbounded_channel();
        let served = tui
            .serve(
                &self.backend,
                &replies,
                &RequestEnvelope {
                    seq,
                    origin: Origin::App,
                    request: StoreRequest::Orch(OrchRequest::Command(command)),
                },
                &LiveChats::default(),
            )
            .await;
        assert!(matches!(served, RunServed::Deferred), "{served:?}");
        assert!(tui.settle(PATIENCE).await.is_empty(), "no TUI task stuck");
        drop(replies);
        let mut reply = None;
        while let Ok(envelope) = answers.try_recv() {
            if envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_)) {
                reply = Some(envelope.reply);
            }
        }
        match reply.expect("the command was answered") {
            StoreReply::Orch(OrchReply::Done(outcome)) => *outcome,
            other => panic!("expected a command outcome, got {other:?}"),
        }
    }

    /// `R` on `item`: the run and where its first walk rested.
    async fn start(&mut self, item: ItemId) -> (RunId, htui_orch::Rest) {
        match self
            .command(Command::StartRun {
                item,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
        {
            CommandOutcome::Started { run, rest } => (run, rest),
            other => panic!("a start answers Started, got {other:?}"),
        }
    }

    /// The TUI exits, so the parent holds no runtime that could sweep (blueprint H-8).
    async fn exit_tui(&mut self) {
        let mut tui = self.tui.take().expect("the TUI is still running");
        tui.shutdown(Duration::ZERO).await;
    }

    /// The box's executor, written through its editor (MOD-41 plan D10).
    async fn set_executor(&self, executor: Executor) {
        let row = self
            .db
            .store
            .box_row(ids::BOX)
            .await
            .expect("the read answers")
            .expect("the demo box");
        let edited = self
            .db
            .store
            .edit_box(
                ids::BOX,
                row.edit_version,
                BoxEdit {
                    executor: Some(executor),
                    ..BoxEdit::default()
                },
            )
            .await
            .expect("the edit answers");
        assert!(
            matches!(edited, CasOutcome::Applied(_)),
            "the executor was written: {edited:?}"
        );
    }

    /// Every phase of `htui`'s default `FEAT` graph ungated, so a recovered `Finished` step lands
    /// `done` and the walk reaches `done` alone.
    async fn ungate_feat(&self) {
        sqlx::query(
            "UPDATE step_graph_phase SET gate = 'never', gate_hard = false WHERE graph_id = $1",
        )
        .bind(ids::GRAPH_HTUI_FEAT.as_uuid())
        .execute(&self.db.pool)
        .await
        .expect("ungate the FEAT graph");
    }

    /// Plan D2: a lease lapses [`TTL_SECONDS`] after its last renewal (seeded `120` by `0003`; no
    /// Postgres setter exists).
    async fn short_leases(&self) {
        let updated = sqlx::query(
            "UPDATE app_setting SET value = to_jsonb($1::integer) WHERE key = 'lease_ttl_seconds'",
        )
        .bind(TTL_SECONDS)
        .execute(&self.db.pool)
        .await
        .expect("shorten the lease");
        assert_eq!(updated.rows_affected(), 1, "the seeded setting exists");
    }

    async fn run_row(&self, run: RunId) -> htui_core::model::Run {
        self.db
            .store
            .run(run)
            .await
            .expect("the read answers")
            .expect("the run exists")
    }

    async fn steps(&self, run: RunId) -> Vec<RunStep> {
        self.db
            .store
            .run_steps(run)
            .await
            .expect("the read answers")
    }

    /// `run.lease_owner`, which the `Run` row deliberately does not carry.
    async fn lease_owner(&self, run: RunId) -> Option<uuid::Uuid> {
        sqlx::query_scalar::<_, Option<uuid::Uuid>>("SELECT lease_owner FROM run WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&self.db.pool)
            .await
            .expect("read run.lease_owner")
    }

    /// Closes the mirror and drops the database. Every case calls this on its last line.
    async fn finish(mut self) {
        if let Some(mut tui) = self.tui.take() {
            tui.shutdown(Duration::ZERO).await;
        }
        let Self { db, cache, .. } = self;
        cache.close().await;
        db.drop_db().await;
    }
}

/// A spawned child that is killed and reaped when the case ends, even by a panic
/// (`worker_pg.rs`'s guard, blueprint H-2).
struct Reaped {
    child: std::process::Child,
    log: PathBuf,
}

impl Reaped {
    /// The child's output so far.
    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_else(|err| format!("(no log: {err})"))
    }

    /// Panics, with the log, when the child already exited: it can reach nothing more.
    fn alive(&mut self, waiting_for: &str) {
        if let Some(status) = self.child.try_wait().expect("poll the child") {
            let why = match status.code() {
                Some(MISCONFIGURED_EXIT) => "the kill-point spec is malformed",
                Some(PARK_EXPIRED_EXIT) => "a parked kill point was never killed",
                Some(0) => "the child returned: did `--exact crash_child` match a test? (H-14)",
                _ => "the child failed",
            };
            panic!(
                "the child exited ({status}: {why}) while the case waited for {waiting_for}; its \
                 log:\n{}",
                self.log()
            );
        }
    }

    /// `SIGKILL`, reaped: the child's exit signal is 9.
    fn killed(mut self) {
        self.child.kill().expect("the child is still running");
        let status = self.child.wait().expect("reap the child");
        assert_eq!(
            status.signal(),
            Some(9),
            "the child was SIGKILLed, not ended otherwise ({status}); its log:\n{}",
            self.log()
        );
    }

    /// Closes the child's stdin and waits for its loop to shut down: exit code 0.
    async fn stopped(mut self) {
        drop(self.child.stdin.take());
        let status = self.exit_within(PATIENCE).await;
        assert_eq!(
            status.code(),
            Some(0),
            "the restarted worker shut down cleanly ({status}); its log:\n{}",
            self.log()
        );
    }

    async fn exit_within(&mut self, limit: Duration) -> ExitStatus {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll the child") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the child did not exit within {limit:?}; its log:\n{}",
                self.log()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One case: the stack, a queued `FEAT-3` run scoped to the temporary repository, and the
/// directory holding that repository, the scratch root, the marker and the children's logs.
struct Case {
    stack: Stack,
    dir: tempfile::TempDir,
    run: RunId,
    children: u32,
}

impl Case {
    /// The case, or `None` (after the skip sentence) without `git` or without a server.
    async fn new() -> Option<Self> {
        htui_orch::skip_without_git!()?;
        let mut stack = Stack::new().await?;
        stack.ungate_feat().await;
        stack.set_executor(Executor::Worker).await;
        stack.short_leases().await;
        let dir = tempfile::tempdir().expect("a throwaway case directory");
        htui_orch::isolate::git::testkit::repo_with_one_commit(&dir.path().join("repo"));
        let (run, rest) = stack.start(ids::HTUI_FEAT_3).await;
        assert_eq!(
            rest.run,
            RunStatus::Queued,
            "a worker box only queues (plan D12)"
        );
        // K3's precondition: an empty scope would make every capture vacuously complete.
        assert_eq!(
            stack.run_row(run).await.repo_scope,
            vec![stack.repo],
            "the run is scoped to the primary repo"
        );
        stack.exit_tui().await;
        Some(Self {
            stack,
            dir,
            run,
            children: 0,
        })
    }

    fn checkout(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    fn mark(&self) -> PathBuf {
        self.dir.path().join("killed")
    }

    /// The `n`th worker process: this binary's [`crash_child`] playing `act`, stopping at `point`
    /// when one is named. Never two at once (blueprint H-8).
    fn spawn(&mut self, act: Act, point: Option<&str>) -> Reaped {
        self.children += 1;
        let log = self.dir.path().join(format!("child-{}.log", self.children));
        let out = File::create(&log).expect("create the child's log");
        let err = out.try_clone().expect("share the log");
        let mut command = Process::new(std::env::current_exe().expect("this test binary"));
        command
            .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
            .env(DSN_VAR, &self.stack.db.url)
            .env(REPO_VAR, self.stack.repo.to_string())
            .env(CHECKOUT_VAR, self.checkout())
            .env(SCRATCH_VAR, self.dir.path().join("trees"))
            .env(SCRIPT_VAR, act.name())
            .env(MARK_VAR, self.mark())
            // H-13: reconcile's merge needs an identity, and CI may not have one.
            .env("GIT_AUTHOR_NAME", "htui-test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "htui-test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err);
        // H-3: a developer shell's spec must not park a child that names none.
        match point {
            Some(point) => command.env(POINT_VAR, point),
            None => command.env_remove(POINT_VAR),
        };
        Reaped {
            child: command.spawn().expect("spawn the child worker"),
            log,
        }
    }

    /// Polls until the marker exists (blueprint H-15).
    async fn marked(&self, child: &mut Reaped) {
        let deadline = Instant::now() + PATIENCE;
        while !self.mark().exists() {
            child.alive("its kill point");
            assert!(
                Instant::now() < deadline,
                "no marker within {PATIENCE:?}; the child's log:\n{}",
                child.log()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Polls until the run rests under `child`: not `queued` or `running`, and no step `running`;
    /// then until its lease is given back, which is its own write after the walk settled it.
    async fn rested(&self, child: &mut Reaped) -> htui_core::model::Run {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let row = self.stack.run_row(self.run).await;
            let walking = self
                .stack
                .steps(self.run)
                .await
                .iter()
                .any(|step| step.status == StepStatus::Running);
            if !matches!(row.status, RunStatus::Queued | RunStatus::Running)
                && !walking
                && self.stack.lease_owner(self.run).await.is_none()
            {
                return row;
            }
            child.alive("the run to rest");
            assert!(
                Instant::now() < deadline,
                "the run did not rest within {PATIENCE:?}: it is `{}`, steps {:?}; the child's \
                 log:\n{}",
                row.status,
                shape(&self.stack.steps(self.run).await),
                child.log()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn steps(&self) -> Vec<RunStep> {
        self.stack.steps(self.run).await
    }

    /// `item_note.body` of every note on `FEAT-3`.
    async fn notes(&self) -> Vec<String> {
        sqlx::query_scalar::<_, String>("SELECT body FROM item_note WHERE item_id = $1")
            .bind(ids::HTUI_FEAT_3.as_uuid())
            .fetch_all(&self.stack.db.pool)
            .await
            .expect("read item_note")
    }

    /// The steps of this run that produced a `FEAT-3` document of `kind`.
    async fn produced_by(&self, kind: &str) -> BTreeSet<StepId> {
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT d.produced_by_step_id FROM document d JOIN run_step s \
             ON s.id = d.produced_by_step_id WHERE d.item_id = $1 AND d.kind = $2 \
             AND s.run_id = $3",
        )
        .bind(ids::HTUI_FEAT_3.as_uuid())
        .bind(kind)
        .bind(self.run.as_uuid())
        .fetch_all(&self.stack.db.pool)
        .await
        .expect("read document")
        .into_iter()
        .map(StepId::from)
        .collect()
    }

    /// `run_command.status` of every command of the run, in issue order.
    async fn command_statuses(&self) -> Vec<RunCommandStatus> {
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM run_command WHERE run_id = $1 ORDER BY issued_at, id",
        )
        .bind(self.run.as_uuid())
        .fetch_all(&self.stack.db.pool)
        .await
        .expect("read run_command")
        .iter()
        .map(|status| status.parse().expect("a known command status"))
        .collect()
    }

    /// `(seq, kind)` of every `session_event` row of `step`.
    async fn events(&self, step: StepId) -> Vec<(i32, EventKind)> {
        self.stack
            .db
            .store
            .step_events(step)
            .await
            .expect("the log reads")
            .unwrap_or_default()
            .into_iter()
            .map(|row| (row.seq, row.kind))
            .collect()
    }

    /// The paths `HEAD` of the checkout holds: what the run landed.
    fn landed(&self) -> Vec<String> {
        git(&self.checkout(), &["ls-tree", "-r", "--name-only", "HEAD"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Plan T4's common outcome of K1-K3: `prd` attempt 1 failed `interrupted`, the item noted
    /// it, attempt 2 walked, the run `done` with every phase done, its tree landed. Attempt 2.
    async fn interrupted_then_retried(&self, row: &htui_core::model::Run) -> RunStep {
        let steps = self.steps().await;
        let first = prd(&steps, 1).expect("prd attempt 1");
        assert_eq!(
            (first.status, first.gate_note.as_deref()),
            (StepStatus::Failed, Some("interrupted")),
            "(a) the interrupted attempt failed `interrupted`: {:?}",
            shape(&steps)
        );
        let noted = "(`prd` attempt 1) did not finish";
        let notes = self.notes().await;
        assert!(
            notes
                .iter()
                .any(|note| note.contains(noted) && note.contains("retrying as attempt 2")),
            "(b) the item says attempt 1 did not finish and attempt 2 follows: {notes:?}"
        );
        let second = prd(&steps, 2).expect("(c) prd attempt 2 was admitted");
        assert_eq!(second.status, StepStatus::Done, "(c) {:?}", shape(&steps));
        assert_eq!(
            row.status,
            RunStatus::Done,
            "(d) the run walked on to the end"
        );
        let phases: BTreeSet<&str> = steps.iter().map(|step| step.phase_name.as_str()).collect();
        for phase in phases {
            assert!(
                steps
                    .iter()
                    .any(|step| step.phase_name == phase && step.status == StepStatus::Done),
                "(d) phase `{phase}` has a done step: {:?}",
                shape(&steps)
            );
        }
        // (e) is `rested`'s last condition: the lease was given back.
        let landed = self.landed();
        assert!(
            landed.contains(&step_file(second.id)),
            "(f) attempt 2's tree landed: {landed:?}"
        );
        second.clone()
    }

    /// Drops the database and the case directory. Every case calls this on its last line.
    async fn finish(self) {
        let Self { stack, dir, .. } = self;
        stack.finish().await;
        dir.close().expect("remove the case directory");
    }
}

/// The `prd` step of `attempt`.
fn prd(steps: &[RunStep], attempt: i32) -> Option<&RunStep> {
    steps
        .iter()
        .find(|step| step.phase_name == "prd" && step.attempt == attempt)
}

/// `(position, attempt, fanout_index, phase, status)` of every step, in that order.
fn shape(steps: &[RunStep]) -> Vec<(i32, i32, i32, String, StepStatus)> {
    let mut shape: Vec<_> = steps
        .iter()
        .map(|step| {
            (
                step.position,
                step.attempt,
                step.fanout_index,
                step.phase_name.clone(),
                step.status,
            )
        })
        .collect();
    shape.sort_by_key(|step| (step.0, step.1, step.2));
    shape
}

// ---------------------------------------------------------------------------------------------
// The helper
// ---------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------
// K4: after capture
// ---------------------------------------------------------------------------------------------

/// Plan T4 K4: a worker killed after `prd`'s `after` commits were recorded, before the step was
/// finished. Its document and capture both landed, so the next worker's sweep settles it `done`
/// without a session: no attempt 2, no new event, and the run walks on to `done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_killed_after_capture_settles_the_step_done() {
    let Some(mut case) = Case::new().await else {
        return;
    };
    let mut first = case.spawn(Act::Walks, Some("captured@prd#1"));
    case.marked(&mut first).await;
    first.killed();

    let steps = case.steps().await;
    let killed = prd(&steps, 1).expect("prd attempt 1").clone();
    assert_eq!(killed.status, StepStatus::Running, "killed mid-step");
    let commits = case
        .stack
        .db
        .store
        .step_commits(killed.id)
        .await
        .expect("the commits read");
    assert!(
        !commits.is_empty() && commits.iter().all(|row| row.after_hash.is_some()),
        "B8: the session committed, so the capture recorded an `after` commit: {commits:?}"
    );
    assert_eq!(
        case.produced_by("prd").await,
        BTreeSet::from([killed.id]),
        "the document precedes the capture"
    );
    let at_kill = case.events(killed.id).await;

    let mut second = case.spawn(Act::Walks, None);
    let row = case.rested(&mut second).await;
    let steps = case.steps().await;
    let settled = prd(&steps, 1).expect("prd attempt 1");
    assert_eq!(
        settled.status,
        StepStatus::Done,
        "recovery settled the captured step: {:?}",
        shape(&steps)
    );
    assert!(
        prd(&steps, 2).is_none(),
        "no attempt 2: {:?}",
        shape(&steps)
    );
    assert_eq!(
        case.events(killed.id).await,
        at_kill,
        "no new session ran for it"
    );
    let notes = case.notes().await;
    assert!(
        !notes
            .iter()
            .any(|note| note.contains("retrying as attempt 2")),
        "nothing was retried: {notes:?}"
    );
    assert_eq!(row.status, RunStatus::Done, "the run walked on to the end");
    let landed = case.landed();
    assert!(
        landed.contains(&step_file(killed.id)),
        "recovery's reconcile merged the killed step's commit: {landed:?}"
    );
    second.stopped().await;
    case.finish().await;
}

// ---------------------------------------------------------------------------------------------
// K1 to K3: interrupted, then retried
// ---------------------------------------------------------------------------------------------

/// The step tree of `step`: its one repo's worktree.
async fn tree_of(case: &Case, step: StepId) -> PathBuf {
    let trees = case
        .stack
        .db
        .store
        .step_trees(step)
        .await
        .expect("the trees read");
    assert_eq!(trees.len(), 1, "one repo, one tree: {trees:?}");
    PathBuf::from(&trees[0].path)
}

/// Plan T4 K1: a worker killed once its session started and before any driver event, after the
/// session wrote a file it never committed. The next worker's sweep resets the tree, fails attempt
/// 1 `interrupted`, notes the retry, and walks attempt 2 to `done`; the stray file is in no tree
/// the run lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_killed_before_any_event_retries_the_step() {
    let Some(mut case) = Case::new().await else {
        return;
    };
    let mut first = case.spawn(Act::Stall, None);
    case.marked(&mut first).await;
    first.killed();

    let steps = case.steps().await;
    let killed = prd(&steps, 1).expect("prd attempt 1").clone();
    assert_eq!(killed.status, StepStatus::Running, "killed mid-step");
    assert!(
        tree_of(&case, killed.id).await.join(STRAY).exists(),
        "the killed session left its stray file in its tree"
    );
    let prompt_only = vec![EventKind::Prompt];
    let kinds = |events: Vec<(i32, EventKind)>| -> Vec<EventKind> {
        events.into_iter().map(|(_, kind)| kind).collect()
    };
    assert_eq!(
        kinds(case.events(killed.id).await),
        prompt_only,
        "no driver event was flushed before the kill"
    );

    let mut second = case.spawn(Act::Walks, None);
    let row = case.rested(&mut second).await;
    let retried = case.interrupted_then_retried(&row).await;
    let landed = case.landed();
    assert!(
        !landed.iter().any(|path| path == STRAY),
        "the stray file did not land: {landed:?}"
    );
    assert!(
        !case.checkout().join(STRAY).exists(),
        "nor is it in the checkout's working tree"
    );
    assert!(
        !tree_of(&case, retried.id).await.join(STRAY).exists(),
        "attempt 2's tree never held it"
    );
    assert_eq!(
        kinds(case.events(killed.id).await),
        prompt_only,
        "recovery wrote nothing into attempt 1's log"
    );
    second.stopped().await;
    case.finish().await;
}

/// Plan T4 K2: a worker killed after its session's `ToolCall` was flushed, while it parks on a
/// permission request. As K1, and attempt 1 keeps its flushed rows (`R-HIS-1`).
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_killed_after_a_flush_keeps_its_rows_and_retries() {
    let Some(mut case) = Case::new().await else {
        return;
    };
    let mut first = case.spawn(Act::Park, None);
    let deadline = Instant::now() + PATIENCE;
    let killed = loop {
        let steps = case.steps().await;
        if let Some(step) = prd(&steps, 1) {
            let events = case.events(step.id).await;
            if events.iter().any(|(_, kind)| *kind != EventKind::Prompt) {
                break step.clone();
            }
        }
        first.alive("a flushed event");
        assert!(
            Instant::now() < deadline,
            "no event beyond the prompt within {PATIENCE:?}; the child's log:\n{}",
            first.log()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    first.killed();

    let at_kill = case.events(killed.id).await;
    assert!(
        at_kill.iter().any(|(_, kind)| *kind == EventKind::ToolCall),
        "the tool call was flushed: {at_kill:?}"
    );
    let mut second = case.spawn(Act::Walks, None);
    let row = case.rested(&mut second).await;
    case.interrupted_then_retried(&row).await;
    assert_eq!(
        case.events(killed.id).await,
        at_kill,
        "R-HIS-1: attempt 1 keeps exactly the rows it flushed"
    );
    second.stopped().await;
    case.finish().await;
}

/// Plan T4 K3: a worker killed after `prd`'s output document was written, before its trees were
/// captured. The document notwithstanding, the step is not finished: the sweep resets and retries
/// it as K1, and attempt 2 writes its own document.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_killed_after_the_document_retries_the_step() {
    let Some(mut case) = Case::new().await else {
        return;
    };
    let mut first = case.spawn(Act::Walks, Some("documented@prd#1"));
    case.marked(&mut first).await;
    first.killed();

    let steps = case.steps().await;
    let killed = prd(&steps, 1).expect("prd attempt 1").clone();
    assert_eq!(killed.status, StepStatus::Running, "killed mid-step");
    assert_eq!(
        case.produced_by("prd").await,
        BTreeSet::from([killed.id]),
        "attempt 1's document was written before the kill"
    );

    let mut second = case.spawn(Act::Walks, None);
    let row = case.rested(&mut second).await;
    let retried = case.interrupted_then_retried(&row).await;
    assert_eq!(
        case.produced_by("prd").await,
        BTreeSet::from([killed.id, retried.id]),
        "attempt 2 wrote its own document beside attempt 1's"
    );
    second.stopped().await;
    case.finish().await;
}

// ---------------------------------------------------------------------------------------------
// K5: a cancel picked, not applied
// ---------------------------------------------------------------------------------------------

/// A second store client registered as **another box** (`worker_pg.rs`), as a TUI elsewhere
/// holding the DSN; its `box.toml` lives in the returned directory, never the real home.
async fn another_box(db: &testkit::TestDb) -> (PgStore, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let identity = htui_store::identity::load_or_mint(root.path()).expect("mint box.toml");
    let client = PgStore::connect(&db.url, &identity)
        .await
        .expect("the second client connects")
        .store;
    assert_ne!(
        client.this_box(),
        ids::BOX,
        "the second client is another box"
    );
    (client, root)
}

/// The first pending request the other box's Runs pane shows for `FEAT-3`, while `child` lives.
async fn pending_request(client: &PgStore, child: &mut Reaped) -> StepPermission {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let view = client
            .relay_view(ids::HTUI_FEAT_3)
            .await
            .expect("the relay view reads");
        if let Some(row) = view.permissions.into_iter().next() {
            return row;
        }
        child.alive("the parked request");
        assert!(
            Instant::now() < deadline,
            "no request parked within {PATIENCE:?}; the child's log:\n{}",
            child.log()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Plan T4 K5: a client on another box requests a cancel while the worker's session is parked
/// (the lease is live, so the row waits for the executor); the worker's command poll picks the row
/// up and dies before applying it. The next worker applies it before it recovers anything (D3):
/// the run ends `cancelled` with its command `applied`, no step was retried, and it never reached
/// `done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_picked_but_not_applied_survives_the_kill() {
    let Some(mut case) = Case::new().await else {
        return;
    };
    let mut first = case.spawn(Act::Park, Some("command_picked"));
    let (client, _client_root) = another_box(&case.stack.db).await;
    let parked = pending_request(&client, &mut first).await;
    assert_eq!(parked.run_id, case.run);
    let requested = client
        .request_cancel(case.run, client.this_user(), client.this_box())
        .await
        .expect("the cancel is written");
    assert!(
        matches!(requested, CancelRequest::Inserted(_)),
        "queued for the executor, whose lease is live: {requested:?}"
    );
    case.marked(&mut first).await;
    first.killed();

    assert_eq!(
        case.command_statuses().await,
        [RunCommandStatus::Pending],
        "picked, not applied"
    );
    assert_eq!(
        case.stack.run_row(case.run).await.status,
        RunStatus::Running,
        "the kill left the run running"
    );

    let mut second = case.spawn(Act::Walks, None);
    let row = case.rested(&mut second).await;
    let steps = case.steps().await;
    assert_eq!(
        row.status,
        RunStatus::Cancelled,
        "the cancel survived the kill: {:?}",
        shape(&steps)
    );
    let deadline = Instant::now() + PATIENCE;
    while case.command_statuses().await != [RunCommandStatus::Applied] {
        second.alive("the command to be applied");
        assert!(
            Instant::now() < deadline,
            "the command was not applied: {:?}",
            case.command_statuses().await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        steps.iter().all(|step| step.attempt == 1)
            && !steps.iter().any(|step| step.status == StepStatus::Done),
        "no recovery ran, so nothing was retried or finished: {:?}",
        shape(&steps)
    );
    assert_eq!(
        prd(&steps, 1).map(|step| step.status),
        Some(StepStatus::Cancelled),
        "the parked step was cancelled: {:?}",
        shape(&steps)
    );
    drop(client);
    second.stopped().await;
    case.finish().await;
}
