//! MOD-11 T8: `command_run` through `McpHost` (blueprint §11, §2.10; plan D14-D16, OQ-3, OQ-7,
//! I-1, I-5, I-7, H-18).
//!
//! Every case hosts `McpHost<Backend>` over a `MemStore::demo()` (blueprint B-1). The session's
//! step is the fixture's `R2/prd`, its box the demo box, its `cwd` a scratch directory; the
//! commands are `sh` one-liners. A command that must be found again by `pgrep` sleeps a duration
//! no other test uses.

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use htui_core::clock::TestClock;
use htui_core::fixtures::ids;
use htui_core::model::{CommandRun, CommandRunId, CommandRunStatus, NewCommandRun, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
use htui_core::store::{MemStore, StepFence, WriteStore};
use htui_mcp::channel::{Address, Token};
use htui_mcp::protocol::CallResult;
use htui_mcp::{ENV_ADDR, ENV_TOKEN, McpClient, McpHost, RELAY_VERSION};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use uuid::Uuid;

/// The secret the masking case's scrubber knows.
const SECRET: &str = "fake-secret-9d8e7f";

/// How long a case waits for anything asynchronous before it fails.
const PATIENCE: Duration = Duration::from_secs(30);

/// A session scope on the fixture's `R2/prd` step, running in `cwd`, `command_run` per `exposed`.
fn scope(cwd: &Path, exposed: bool) -> ToolScope {
    ToolScope {
        run_id: ids::RUN_2,
        step_id: ids::STEP_R2_PRD,
        project_id: ids::PROJECT_HTUI,
        item_id: None,
        box_id: ids::BOX,
        user: ids::USER,
        fence: StepFence::Unleased,
        output_kind: None,
        hostname: HostnameLine::Omitted,
        command_queue: exposed,
        cwd: cwd.to_path_buf(),
        transport: Transport::Acp,
    }
}

/// A host over `store`.
fn host(store: &MemStore) -> McpHost<Backend> {
    McpHost::new(Backend::memory(store.clone())).expect("a host")
}

/// A live session on `scope` and its initialised in-process client.
async fn open(host: &McpHost<Backend>, scope: ToolScope) -> (ToolLease, McpClient) {
    let lease = host.open(scope).expect("a lease");
    let mut client = host
        .client(&lease.spec.env[ENV_TOKEN])
        .expect("a live session");
    client.initialize().await.expect("initialize");
    (lease, client)
}

/// The successful call's JSON.
fn ok(result: &CallResult) -> Value {
    assert!(!result.is_error, "{}", result.text);
    serde_json::from_str(&result.text).expect("the result is JSON")
}

/// The refused call's one-line reason.
fn refused(result: &CallResult) -> &str {
    assert!(result.is_error, "expected a refusal, got {}", result.text);
    &result.text
}

/// The `command_run` rows of the session's step.
async fn rows(store: &MemStore) -> Vec<CommandRun> {
    store
        .command_runs(ids::STEP_R2_PRD)
        .await
        .expect("command runs")
}

/// The row `id` names.
async fn row(store: &MemStore, id: &str) -> CommandRun {
    rows(store)
        .await
        .into_iter()
        .find(|row| row.id.to_string() == id)
        .unwrap_or_else(|| panic!("command run {id} is stored"))
}

/// Polls until a row of the step is `status`, or fails after [`PATIENCE`].
async fn until_status(store: &MemStore, status: CommandRunStatus) -> CommandRun {
    let started = Instant::now();
    loop {
        if let Some(row) = rows(store)
            .await
            .into_iter()
            .find(|row| row.status == status)
        {
            return row;
        }
        assert!(started.elapsed() < PATIENCE, "no row became {status}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether a process whose command line contains `pattern` is alive.
fn alive(pattern: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", pattern])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Polls until no process matches `pattern`, or fails after five seconds.
async fn until_gone(pattern: &str) {
    let started = Instant::now();
    while alive(pattern) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "`{pattern}` outlived its command"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A raw socket connection to the lease's session, past the handshake, initialised.
async fn connect(lease: &ToolLease) -> BufReader<UnixStream> {
    let addr = Address::new(lease.spec.env[ENV_ADDR].clone());
    let token = Token::parse(&lease.spec.env[ENV_TOKEN]).expect("a token");
    let stream = UnixStream::connect(addr.as_str()).await.expect("connect");
    let mut stream = BufReader::new(stream);
    send(
        &mut stream,
        json!({"token": token.as_str(), "version": RELAY_VERSION}),
    )
    .await;
    assert_eq!(
        recv(&mut stream).await,
        json!({"ok": true}),
        "the handshake"
    );
    send(
        &mut stream,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}
        }}),
    )
    .await;
    assert_eq!(recv(&mut stream).await["id"], json!(0));
    stream
}

async fn send(stream: &mut BufReader<UnixStream>, message: Value) {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.expect("write");
}

async fn recv(stream: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    let read = tokio::time::timeout(PATIENCE, stream.read_line(&mut line))
        .await
        .expect("a line in time")
        .expect("read");
    assert_ne!(read, 0, "the host closed the stream");
    serde_json::from_str(&line).expect("JSON")
}

/// A `queued` row of `class` on the session's step and box, as another session would enqueue.
fn queued(class: &str) -> NewCommandRun {
    NewCommandRun {
        id: CommandRunId::new(),
        run_step_id: ids::STEP_R2_PRD,
        box_id: ids::BOX,
        class: class.to_owned(),
        command: "make".to_owned(),
        cwd: "/srv".to_owned(),
        status: CommandRunStatus::Queued,
        exit_code: None,
        output: None,
        queued_at: Utc::now(),
        started_at: None,
        finished_at: None,
    }
}

/// A scrubber that cannot mask anything: every text is `Unmasked` (fail closed, I-5).
#[derive(Debug)]
struct Unmaskable;

impl Scrubber for Unmaskable {
    fn scrub(&self, _: &mut Value) -> Result<(), Unmasked> {
        Err(Unmasked {
            path: String::new(),
            rule: "test_rule",
        })
    }
}

// ---------------------------------------------------------------------------------------------

/// OQ-7, D14: the command is queued, admitted, run in the session's `cwd` through `sh`, finished
/// with its exit code and merged output, and answered with the row's id.
#[tokio::test]
async fn a_command_runs_after_admission_and_returns_its_tail() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    let answer = ok(&client
        .call(
            "command_run",
            json!({"class": "build", "command": "echo hello; echo oops >&2; pwd -P; exit 2"}),
        )
        .await
        .expect("the call"));
    assert_eq!(answer["exit_code"], json!(2), "{answer}");
    assert_eq!(
        answer["status"],
        json!("done"),
        "a command that exited is done"
    );
    assert_eq!(answer["truncated"], json!(false));
    let output = answer["output"].as_str().expect("output");
    assert!(
        output.contains("hello\n") && output.contains("oops\n"),
        "{output}"
    );
    let here = dir.path().canonicalize().expect("canonical");
    assert!(output.contains(here.to_str().expect("utf-8")), "{output}");

    let id = answer["command_run_id"].as_str().expect("the row's id");
    let stored = row(&store, id).await;
    assert_eq!(
        (
            stored.status,
            stored.exit_code,
            stored.class.as_str(),
            stored.cwd.as_str()
        ),
        (
            CommandRunStatus::Done,
            Some(2),
            "build",
            dir.path().to_str().expect("utf-8")
        )
    );
    assert_eq!(
        stored.output.as_deref(),
        Some(output),
        "the row holds the answer's output"
    );
    assert!(stored.started_at.is_some() && stored.finished_at.is_some());
    assert_eq!(stored.command, "echo hello; echo oops >&2; pwd -P; exit 2");
}

/// D14, D15: with no limit configured a class runs one at a time, so a second call waits for the
/// first; with `app_setting.command_limits.test = 2` (read per call) the two overlap.
#[tokio::test]
async fn the_class_limit_queues_the_second_call() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_first_lease, mut first) = open(&host, scope(dir.path(), true)).await;
    let (_second_lease, mut second) = open(&host, scope(dir.path(), true)).await;

    let slow = first.call(
        "command_run",
        json!({"class": "test", "command": "sleep 1.5; echo a"}),
    );
    let fast = async {
        until_status(&store, CommandRunStatus::Running).await;
        second
            .call("command_run", json!({"class": "test", "command": "echo b"}))
            .await
    };
    let (slow, fast) = tokio::join!(slow, fast);
    let (slow, fast) = (ok(&slow.expect("first")), ok(&fast.expect("second")));
    let slow = row(&store, slow["command_run_id"].as_str().expect("id")).await;
    let fast = row(&store, fast["command_run_id"].as_str().expect("id")).await;
    assert!(
        fast.started_at >= slow.finished_at,
        "one `test` at a time: the second started {:?}, the first finished {:?}",
        fast.started_at,
        slow.finished_at
    );

    store.set_app_setting("command_limits", json!({"test": 2}));
    let slow = first.call(
        "command_run",
        json!({"class": "test", "command": "sleep 1.5; echo a"}),
    );
    let fast = async {
        let started = Instant::now();
        while rows(&store)
            .await
            .iter()
            .all(|row| row.status != CommandRunStatus::Running)
        {
            assert!(started.elapsed() < PATIENCE);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        second
            .call("command_run", json!({"class": "test", "command": "echo b"}))
            .await
    };
    let (slow, fast) = tokio::join!(slow, fast);
    let (slow, fast) = (ok(&slow.expect("first")), ok(&fast.expect("second")));
    let slow = row(&store, slow["command_run_id"].as_str().expect("id")).await;
    let fast = row(&store, fast["command_run_id"].as_str().expect("id")).await;
    assert!(
        fast.finished_at < slow.finished_at,
        "two slots: the second ran beside the first"
    );
}

/// PRD OQ-6, I-1: `verify` is the orchestrator's, any other class than the three is refused, and
/// so is an argument the schema does not name; nothing is queued.
#[tokio::test]
async fn verify_and_unknown_classes_are_refused() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    let verify = client
        .call("command_run", json!({"class": "verify", "command": "true"}))
        .await
        .expect("the call");
    assert_eq!(refused(&verify), "refused: verify is the orchestrator's");
    let deploy = client
        .call("command_run", json!({"class": "deploy", "command": "true"}))
        .await
        .expect("the call");
    assert_eq!(
        refused(&deploy),
        "refused: class `deploy` is not build, test or run"
    );
    let run_id = client
        .call(
            "command_run",
            json!({"class": "run", "command": "true", "run_id": ids::RUN_2}),
        )
        .await
        .expect("the call");
    assert!(
        refused(&run_id).starts_with("invalid arguments: unknown field `run_id`"),
        "{}",
        run_id.text
    );
    let blank = client
        .call("command_run", json!({"class": "run", "command": "  "}))
        .await
        .expect("the call");
    assert_eq!(refused(&blank), "refused: the command is empty");
    assert!(rows(&store).await.is_empty(), "nothing was queued");
}

/// OQ-7: `cwd` is a relative path under the session's directory that exists, with no `..`; the
/// timeout is at most 1800 s and at least 1.
#[tokio::test]
async fn cwd_must_stay_inside_the_session_cwd() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    std::fs::create_dir(dir.path().join("sub")).expect("a subdirectory");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    for (cwd, reason) in [
        (
            "../elsewhere",
            "refused: cwd `../elsewhere` must be a relative path inside the session's directory, without `..`",
        ),
        (
            "sub/../../x",
            "refused: cwd `sub/../../x` must be a relative path inside the session's directory, without `..`",
        ),
        (
            "/tmp",
            "refused: cwd `/tmp` must be a relative path inside the session's directory, without `..`",
        ),
        (
            "missing",
            "refused: cwd `missing` is not a directory under the session's directory",
        ),
    ] {
        let answer = client
            .call(
                "command_run",
                json!({"class": "run", "command": "true", "cwd": cwd}),
            )
            .await
            .expect("the call");
        assert_eq!(refused(&answer), reason, "cwd {cwd}");
    }
    for timeout in [0, 1801] {
        let answer = client
            .call(
                "command_run",
                json!({"class": "run", "command": "true", "timeout_secs": timeout}),
            )
            .await
            .expect("the call");
        assert_eq!(
            refused(&answer),
            format!("refused: timeout_secs {timeout} is not 1..=1800"),
        );
    }
    assert!(rows(&store).await.is_empty(), "nothing was queued");

    let answer = ok(&client
        .call(
            "command_run",
            json!({"class": "run", "command": "pwd -P", "cwd": "sub"}),
        )
        .await
        .expect("the call"));
    let sub = dir.path().join("sub").canonicalize().expect("canonical");
    assert_eq!(
        answer["output"],
        json!(format!("{}\n", sub.display())),
        "it ran in the subdirectory"
    );
    let stored = row(&store, answer["command_run_id"].as_str().expect("id")).await;
    assert_eq!(
        stored.cwd,
        dir.path().join("sub").display().to_string(),
        "the row records the joined path"
    );
}

/// OQ-7: the timeout kills the shell's whole process group — a background `sleep` too — and the
/// row ends `failed` without an exit code.
#[tokio::test]
async fn a_timeout_kills_the_process_tree_and_fails() {
    const PATTERN: &str = "sleep 30.4217";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    let started = Instant::now();
    let answer = ok(&client
        .call(
            "command_run",
            json!({"class": "test", "command": format!("{PATTERN} & {PATTERN}"), "timeout_secs": 1}),
        )
        .await
        .expect("the call"));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "killed at the timeout"
    );
    assert_eq!(
        (&answer["status"], &answer["exit_code"]),
        (&json!("failed"), &Value::Null),
        "{answer}"
    );
    assert!(
        answer["output"]
            .as_str()
            .is_some_and(|output| output.starts_with("[killed: the 1 s timeout elapsed]")),
        "{answer}"
    );
    until_gone(PATTERN).await;
    let stored = row(&store, answer["command_run_id"].as_str().expect("id")).await;
    assert_eq!(
        (stored.status, stored.exit_code),
        (CommandRunStatus::Failed, None)
    );
}

/// I-5, OQ-7: the output is scrubbed before it is stored or answered, a cut tail opens with the
/// marker, the command line is scrubbed in the row too, and an output the scrubber cannot mask is
/// withheld by rule.
#[tokio::test]
async fn output_is_scrubbed_and_truncated_with_a_marker() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store).with_scrubber(Arc::new(MinimalScrubber::new([SECRET.to_owned()])));
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    let command =
        format!("head -c 100000 /dev/zero | tr '\\0' a; echo; echo the {SECRET} end # {SECRET}");
    let answer = ok(&client
        .call("command_run", json!({"class": "build", "command": command}))
        .await
        .expect("the call"));
    let output = answer["output"].as_str().expect("output");
    assert_eq!(answer["truncated"], json!(true));
    assert!(
        output.starts_with("[… earlier output truncated]\n"),
        "{}",
        &output[..80]
    );
    assert!(
        output.ends_with("the [REDACTED] end\n"),
        "{}",
        &output[output.len() - 40..]
    );
    assert!(!output.contains(SECRET));
    let stored = row(&store, answer["command_run_id"].as_str().expect("id")).await;
    assert_eq!(stored.output.as_deref(), Some(output));
    assert!(
        !stored.command.contains(SECRET) && stored.command.contains("[REDACTED]"),
        "{}",
        stored.command
    );

    let host = McpHost::new(Backend::memory(store.clone()))
        .expect("a host")
        .with_scrubber(Arc::new(Unmaskable));
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;
    let answer = client
        .call(
            "command_run",
            json!({"class": "build", "command": "echo hi"}),
        )
        .await
        .expect("the call");
    assert_eq!(
        refused(&answer),
        "refused: the text matched credential rule test_rule; nothing was written",
        "the command line itself cannot be stored"
    );
}

/// I-5: a command whose line is clean but whose output the scrubber cannot mask runs, and its
/// output is withheld by rule rather than stored or answered.
#[tokio::test]
async fn an_unmaskable_output_is_withheld_by_rule() {
    /// Masks nothing in a command line, refuses everything else.
    #[derive(Debug)]
    struct RefusesOutput;
    impl Scrubber for RefusesOutput {
        fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
            if value.as_str().is_some_and(|text| text.starts_with("echo ")) {
                return Ok(());
            }
            Err(Unmasked {
                path: String::new(),
                rule: "test_rule",
            })
        }
    }
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store).with_scrubber(Arc::new(RefusesOutput));
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;
    let answer = ok(&client
        .call(
            "command_run",
            json!({"class": "build", "command": "echo hi"}),
        )
        .await
        .expect("the call"));
    assert_eq!(
        answer["output"],
        json!("[output withheld: it matched credential rule test_rule]")
    );
    let stored = row(&store, answer["command_run_id"].as_str().expect("id")).await;
    assert_eq!(
        stored.output.as_deref(),
        Some("[output withheld: it matched credential rule test_rule]")
    );
}

/// H-18: `notifications/cancelled` aborts the call; its drop guard cancels the row and the child
/// dies with the task.
#[tokio::test]
async fn a_cancelled_call_cancels_its_row() {
    const PATTERN: &str = "sleep 30.4218";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let lease = host.open(scope(dir.path(), true)).expect("a lease");
    let mut stream = connect(&lease).await;

    send(
        &mut stream,
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
               "params": {"name": "command_run", "arguments": {"class": "run", "command": PATTERN}}}),
    )
    .await;
    until_status(&store, CommandRunStatus::Running).await;
    assert!(alive(PATTERN), "the command runs");
    send(
        &mut stream,
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}}),
    )
    .await;
    let cancelled = until_status(&store, CommandRunStatus::Cancelled).await;
    assert!(cancelled.finished_at.is_some());
    until_gone(PATTERN).await;

    // MOD-11 R1 L3: the cancelled call gave its session's slot back.
    let mut client = host
        .client(&lease.spec.env[ENV_TOKEN])
        .expect("a live session");
    client.initialize().await.expect("initialize");
    let again = client
        .call(
            "command_run",
            json!({"class": "run", "command": "echo again"}),
        )
        .await
        .expect("answered");
    assert_eq!(ok(&again)["output"], "again\n");
}

/// MOD-11 R1 L3: one `command_run` at a time per session, so one agent cannot fill the store's
/// connection pool with waiting calls. A second call while the first is queued or running is
/// refused at once and queues nothing; once the first has answered, a new one is accepted.
#[tokio::test]
async fn a_second_concurrent_call_in_one_session_is_refused() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (lease, mut first) = open(&host, scope(dir.path(), true)).await;
    let mut second = host
        .client(&lease.spec.env[ENV_TOKEN])
        .expect("a live session");
    second.initialize().await.expect("initialize");
    let before = rows(&store).await.len();

    let slow = first.call(
        "command_run",
        json!({"class": "run", "command": "sleep 1.2; echo a"}),
    );
    let busy = async {
        until_status(&store, CommandRunStatus::Running).await;
        let started = Instant::now();
        let answer = second
            .call("command_run", json!({"class": "run", "command": "echo b"}))
            .await;
        (answer, started.elapsed())
    };
    let (slow, (busy, waited)) = tokio::join!(slow, busy);
    assert_eq!(ok(&slow.expect("first"))["output"], "a\n");
    assert_eq!(
        refused(&busy.expect("answered")),
        "refused: a command_run is already queued or running in this session"
    );
    assert!(
        waited < Duration::from_secs(1),
        "refused at once: {waited:?}"
    );
    assert_eq!(
        rows(&store).await.len(),
        before + 1,
        "the refusal queued nothing"
    );

    let after = second
        .call("command_run", json!({"class": "run", "command": "echo c"}))
        .await
        .expect("answered");
    assert_eq!(ok(&after)["output"], "c\n");
}

/// OQ-3: a claim whose heartbeat went stale is reaped by another claim of its `(box, class)`; the
/// executor's next beat answers `false`, it kills its child, and the call answers the row as the
/// queue left it.
#[tokio::test]
async fn a_reaped_claim_stops_its_child() {
    const PATTERN: &str = "sleep 30.4219";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let clock = Arc::new(TestClock::at(Utc::now()));
    let store = MemStore::demo().with_clock(clock.clone());
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path(), true)).await;

    let call = client.call("command_run", json!({"class": "build", "command": PATTERN}));
    let reap = async {
        until_status(&store, CommandRunStatus::Running).await;
        clock.advance(TimeDelta::seconds(31));
        let other = store
            .enqueue_command(queued("build"))
            .await
            .expect("queued");
        let admitted = store
            .claim_command(other.id, Uuid::now_v7(), 1)
            .await
            .expect("a claim");
        assert!(
            admitted.is_some(),
            "the stale row is reaped and the slot is free"
        );
        Instant::now()
    };
    let (answer, reaped_at) = tokio::join!(call, reap);
    let answer = ok(&answer.expect("the call"));
    assert!(
        reaped_at.elapsed() < Duration::from_secs(20),
        "the next beat stopped the child"
    );
    assert_eq!(
        (&answer["status"], &answer["exit_code"]),
        (&json!("failed"), &Value::Null),
        "{answer}"
    );
    assert!(
        answer["output"]
            .as_str()
            .is_some_and(|output| output.contains("reaped")),
        "{answer}"
    );
    until_gone(PATTERN).await;
}

/// B-11: a call carrying `_meta.progressToken` hears `notifications/progress` while it waits for
/// a slot, then its answer once the slot frees.
#[tokio::test]
async fn progress_ticks_while_queued() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let holder = Uuid::now_v7();
    let held = store
        .enqueue_command(queued("build"))
        .await
        .expect("queued");
    assert!(
        store
            .claim_command(held.id, holder, 1)
            .await
            .expect("a claim")
            .is_some(),
        "another session holds the one build slot"
    );
    let lease = host.open(scope(dir.path(), true)).expect("a lease");
    let mut stream = connect(&lease).await;

    send(
        &mut stream,
        json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
               "params": {"name": "command_run", "arguments": {"class": "build", "command": "echo admitted"},
                          "_meta": {"progressToken": "queued-9"}}}),
    )
    .await;
    let tick = recv(&mut stream).await;
    assert_eq!(tick["method"], json!("notifications/progress"), "{tick}");
    assert_eq!(tick["params"]["progressToken"], json!("queued-9"));
    let second = recv(&mut stream).await;
    assert_eq!(
        second["method"],
        json!("notifications/progress"),
        "{second}"
    );
    assert!(
        second["params"]["progress"].as_u64() > tick["params"]["progress"].as_u64(),
        "progress grows: {tick} then {second}"
    );

    assert!(
        store
            .finish_command(held.id, holder, CommandRunStatus::Done, Some(0), None)
            .await
            .expect("finish"),
        "the slot frees"
    );
    let answer = loop {
        let message = recv(&mut stream).await;
        if message.get("id") == Some(&json!(9)) {
            break message;
        }
        assert_eq!(
            message["method"],
            json!("notifications/progress"),
            "{message}"
        );
    };
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .expect("a text result");
    let result: Value = serde_json::from_str(text).expect("JSON");
    assert_eq!(result["output"], json!("admitted\n"));
}

/// I-7, D16: a scope without the queue never lists `command_run` and refuses a call to it; with
/// the queue, an item-less session lists it after `box_profile`.
#[tokio::test]
async fn command_run_is_not_advertised_when_exposure_is_off() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_off, mut off) = open(&host, scope(dir.path(), false)).await;
    assert_eq!(off.tool_names().await.expect("tools/list"), ["box_profile"]);
    let answer = off
        .call("command_run", json!({"class": "run", "command": "true"}))
        .await
        .expect("the call");
    assert_eq!(refused(&answer), "unknown tool: command_run");
    assert!(rows(&store).await.is_empty());

    let (_on, mut on) = open(&host, scope(dir.path(), true)).await;
    assert_eq!(
        on.tool_names().await.expect("tools/list"),
        ["box_profile", "command_run"]
    );
}
