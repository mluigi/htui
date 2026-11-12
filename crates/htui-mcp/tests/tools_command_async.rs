//! MOD-152 T5: `command_start`, `command_wait` and `command_cancel` through `McpHost` (D7-D9),
//! with real `sh` children, in real time.
//!
//! Every case hosts `McpHost<Backend>` over a `MemStore::demo()`; the session's step is the
//! fixture's `R2/prd`, its box the demo box, its `cwd` a scratch directory. A command that must be
//! found again by `pgrep` sleeps a duration no other test uses.

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use htui_core::fixtures::ids;
use htui_core::model::{Claim, CommandRun, CommandRunStatus, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::scrub::{Scrubber, Unmasked};
use htui_core::store::{MemStore, StepFence, WriteStore};
use htui_mcp::protocol::CallResult;
use htui_mcp::{ENV_TOKEN, McpClient, McpHost};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use uuid::Uuid;

/// How long a case waits for anything asynchronous before it fails.
const PATIENCE: Duration = Duration::from_secs(30);

/// A session scope on the fixture's `R2/prd` step, running in `cwd`, with the queue exposed.
fn scope(cwd: &Path) -> ToolScope {
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
        command_queue: true,
        cwd: cwd.to_path_buf(),
        extra_dirs: Vec::new(),
        transport: Transport::Acp,
    }
}

/// A host over `store`.
fn host(store: &MemStore) -> McpHost<Backend> {
    McpHost::new(Backend::memory(store.clone())).expect("a host")
}

/// A live session and its initialised in-process client.
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

/// `tool` with `arguments`.
async fn call(client: &mut McpClient, tool: &str, arguments: Value) -> CallResult {
    client.call(tool, arguments).await.expect("the call")
}

/// Starts `command` as a `build` and answers its row's id.
async fn start(client: &mut McpClient, command: &str) -> String {
    let answer = ok(&call(
        client,
        "command_start",
        json!({"class": "build", "command": command}),
    )
    .await);
    assert_eq!(answer["status"], json!("queued"), "{answer}");
    answer["command_run_id"]
        .as_str()
        .expect("the row's id")
        .to_owned()
}

/// An RFC 3339 deadline `seconds` from now.
fn deadline(seconds: i64) -> String {
    (Utc::now() + TimeDelta::seconds(seconds)).to_rfc3339()
}

/// The `command_run` row `id` names.
async fn row(store: &MemStore, id: &str) -> CommandRun {
    store
        .command_runs(ids::STEP_R2_PRD)
        .await
        .expect("command runs")
        .into_iter()
        .find(|row| row.id.to_string() == id)
        .unwrap_or_else(|| panic!("command run {id} is stored"))
}

/// Polls until the row `id` is `status`, or fails after [`PATIENCE`].
async fn until_row(store: &MemStore, id: &str, status: CommandRunStatus) -> CommandRun {
    let started = Instant::now();
    loop {
        let stored = row(store, id).await;
        if stored.status == status {
            return stored;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "row {id} never became {status}: it is {}",
            stored.status
        );
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

/// Polls until a process matches `pattern`, or fails after five seconds.
async fn until_alive(pattern: &str) {
    let started = Instant::now();
    while !alive(pattern) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "`{pattern}` never started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls until no process matches `pattern`, or fails after `within`.
async fn until_gone(pattern: &str, within: Duration) {
    let started = Instant::now();
    while alive(pattern) {
        assert!(
            started.elapsed() < within,
            "`{pattern}` outlived its command"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// D7, D8, D3: a started command answers at once; a wait for a pattern answers the line it
/// matched while the command still runs; `command_output_read` reads the running command's
/// complete lines; a wait with no pattern answers once the row is terminal, with the tail.
#[tokio::test]
async fn start_wait_for_a_pattern_read_then_wait_for_the_end() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path())).await;

    let asked = Instant::now();
    let id = start(
        &mut client,
        "echo one; echo ready-5310; sleep 3; echo two; exit 4",
    )
    .await;
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "command_start answers before the command ends"
    );

    let seen = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30), "pattern": "^ready-\\d+$"}),
    )
    .await);
    assert_eq!(seen["matched"], json!(true), "{seen}");
    assert_eq!(seen["line_no"], json!(2));
    assert_eq!(seen["line"], json!("ready-5310"));
    assert_eq!(seen["next_line"], json!(2));
    assert_eq!(seen["status"], json!("running"));
    assert_eq!(seen["deadline_reached"], json!(false));

    let live = ok(&call(
        &mut client,
        "command_output_read",
        json!({"command_run_id": id}),
    )
    .await);
    assert_eq!(live["status"], json!("running"), "{live}");
    assert_eq!(live["text"], json!("one\nready-5310\n"));
    assert_eq!(live["total_lines"], json!(2));

    let again = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30), "pattern": "ready", "after_line": 2}),
    )
    .await);
    assert_eq!(
        again["matched"],
        json!(false),
        "no ready line after line 2: {again}"
    );
    assert_eq!(again["status"], json!("done"));
    assert_eq!(again["exit_code"], json!(4));

    let ended = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30)}),
    )
    .await);
    assert_eq!(ended["status"], json!("done"), "{ended}");
    assert_eq!(ended["exit_code"], json!(4));
    assert_eq!(ended["tail"], json!("one\nready-5310\ntwo\n"));
    assert_eq!(ended["total_lines"], json!(3));
    assert_eq!(ended["deadline_reached"], json!(false));

    let stored = ok(&call(
        &mut client,
        "command_output_read",
        json!({"command_run_id": id}),
    )
    .await);
    assert_eq!(stored["text"], json!("one\nready-5310\ntwo\n"));
    assert_eq!(row(&store, &id).await.exit_code, Some(4));
}

/// D8: a deadline that passes first answers what the command printed so far, and says so.
#[tokio::test]
async fn a_wait_that_reaches_its_deadline_says_so() {
    const PATTERN: &str = "sleep 30.5320";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let id = start(&mut client, &format!("echo first; {PATTERN}")).await;
    until_alive(PATTERN).await;

    let asked = Instant::now();
    let seen = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(1), "pattern": "never"}),
    )
    .await);
    assert!(asked.elapsed() < Duration::from_secs(5));
    assert_eq!(seen["deadline_reached"], json!(true), "{seen}");
    assert_eq!(seen["matched"], json!(false));
    assert_eq!(seen["status"], json!("running"));
    assert_eq!(seen["tail"], json!("first\n"));
    assert_eq!(seen["next_line"], json!(1));

    ok(&call(&mut client, "command_cancel", json!({"command_run_id": id})).await);
    until_gone(PATTERN, Duration::from_secs(5)).await;
}

/// D7: `command_cancel` kills a running command's process group and ends its row `cancelled`
/// with the reason; its answer is the settled status.
#[tokio::test]
async fn command_cancel_kills_a_running_child() {
    const PATTERN: &str = "sleep 30.5321";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let id = start(&mut client, &format!("echo started; {PATTERN}")).await;
    until_alive(PATTERN).await;
    // `pgrep -f` matches the shell's own command line at once: wait for what it printed.
    let seen = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30), "pattern": "^started$"}),
    )
    .await);
    assert_eq!(seen["matched"], json!(true), "{seen}");

    let answer = ok(&call(&mut client, "command_cancel", json!({"command_run_id": id})).await);
    assert_eq!(answer["status"], json!("cancelled"), "{answer}");
    assert_eq!(answer["command_run_id"], json!(id));
    until_gone(PATTERN, Duration::from_secs(5)).await;

    let stored = row(&store, &id).await;
    assert_eq!(stored.status, CommandRunStatus::Cancelled);
    let output = stored.output.unwrap_or_default();
    assert!(
        output.starts_with("[stopped: cancelled by command_cancel]\n"),
        "{output}"
    );
    assert!(output.contains("started\n"), "{output}");

    let again = ok(&call(&mut client, "command_cancel", json!({"command_run_id": id})).await);
    assert_eq!(
        again["status"],
        json!("cancelled"),
        "an ended row answers as it ended"
    );
}

/// D7: a row the session did not start is not the session's to cancel or to wait on live.
#[tokio::test]
async fn command_cancel_refuses_a_row_of_another_session() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_first_lease, mut first) = open(&host, scope(dir.path())).await;
    let (_second_lease, mut second) = open(&host, scope(dir.path())).await;
    let id = start(&mut first, "sleep 30.5322").await;

    let answer = call(&mut second, "command_cancel", json!({"command_run_id": id})).await;
    assert_eq!(refused(&answer), format!("not found: command_run {id}"));
    let unknown = Uuid::now_v7();
    let answer = call(
        &mut second,
        "command_cancel",
        json!({"command_run_id": unknown}),
    )
    .await;
    assert_eq!(
        refused(&answer),
        format!("not found: command_run {unknown}")
    );
    let answer = call(
        &mut second,
        "command_cancel",
        json!({"command_run_id": "nope"}),
    )
    .await;
    assert_eq!(
        refused(&answer),
        "refused: command_run_id `nope` is not an id"
    );

    ok(&call(&mut first, "command_cancel", json!({"command_run_id": id})).await);
    until_gone("sleep 30.5322", Duration::from_secs(5)).await;
}

/// D7: the end of the session kills a started command's child and ends its row `cancelled`.
#[tokio::test]
async fn the_end_of_the_session_kills_the_child_and_cancels_the_row() {
    const PATTERN: &str = "sleep 30.5323";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (lease, mut client) = open(&host, scope(dir.path())).await;
    let id = start(&mut client, &format!("echo started; {PATTERN}")).await;
    until_alive(PATTERN).await;

    drop(lease);
    until_gone(PATTERN, Duration::from_secs(5)).await;
    let stored = until_row(&store, &id, CommandRunStatus::Cancelled).await;
    let output = stored.output.unwrap_or_default();
    assert!(
        output.starts_with("[stopped: the session ended]\n"),
        "{output}"
    );
}

/// D7, MOD-78 D4: a lease lost while a started command runs is read on the next heartbeat; the
/// child is killed and the row ends `cancelled` with the lease's reason.
#[tokio::test]
async fn a_lost_lease_stops_a_started_child() {
    const PATTERN: &str = "sleep 30.5324";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let owner = Uuid::now_v7();
    assert_eq!(
        store
            .claim_run(
                ids::RUN_2,
                ids::BOX,
                owner,
                Utc::now(),
                TimeDelta::minutes(5)
            )
            .await
            .expect("the claim"),
        Claim::Admitted
    );
    let (_lease, mut client) = open(
        &host,
        ToolScope {
            fence: StepFence::Lease(owner),
            ..scope(dir.path())
        },
    )
    .await;
    let id = start(&mut client, &format!("echo started; {PATTERN}")).await;
    until_alive(PATTERN).await;

    assert!(
        store
            .release_lease(ids::RUN_2, owner)
            .await
            .expect("release")
    );
    assert!(
        store
            .take_lease(ids::RUN_2, ids::BOX, Uuid::now_v7(), TimeDelta::minutes(5))
            .await
            .expect("take")
    );
    until_gone(PATTERN, Duration::from_secs(25)).await;
    let stored = until_row(&store, &id, CommandRunStatus::Cancelled).await;
    let output = stored.output.unwrap_or_default();
    assert!(
        output.starts_with("[stopped: fenced: lease lost]\n"),
        "{output}"
    );

    let seen = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(10)}),
    )
    .await);
    assert_eq!(seen["status"], json!("cancelled"), "{seen}");
}

/// Refuses every text holding the fake key, and nothing else (the command line spells it in two
/// halves).
#[derive(Debug)]
struct RefusesTheKey;

impl Scrubber for RefusesTheKey {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        if value.to_string().contains("AKIA-HIDDEN") {
            return Err(Unmasked {
                path: String::new(),
                rule: "test_rule",
            });
        }
        Ok(())
    }
}

/// D3: a log the scrubber cannot mask is withheld by rule, live and stored; the key is in no
/// answer.
#[tokio::test]
async fn a_secret_in_the_log_is_withheld() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store).with_scrubber(Arc::new(RefusesTheKey));
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let id = start(
        &mut client,
        "echo before; printf 'AKIA%s\\n' -HIDDEN; echo after-5325; sleep 2",
    )
    .await;

    let live = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30), "pattern": "after-5325"}),
    )
    .await);
    let ended = ok(&call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(30)}),
    )
    .await);
    let read = ok(&call(
        &mut client,
        "command_output_read",
        json!({"command_run_id": id}),
    )
    .await);
    for answer in [&live, &ended, &read] {
        assert!(
            !answer.to_string().contains("AKIA-HIDDEN"),
            "the key leaked: {answer}"
        );
    }
    assert_eq!(ended["status"], json!("done"), "{ended}");
    assert_eq!(
        live["matched"],
        json!(false),
        "a withheld line matches nothing"
    );
    assert_eq!(
        ended["tail"],
        json!("[log withheld: it matched credential rule test_rule]\n")
    );
    assert_eq!(
        read["text"],
        json!("[log withheld: it matched credential rule test_rule]\n")
    );
}

/// D7: `command_run` and `command_start` share the box's class limit: a blocking call waits for
/// a started command of its class, and runs once that command is cancelled.
#[tokio::test]
async fn command_run_and_command_start_share_one_class_limit() {
    const PATTERN: &str = "sleep 30.5326";
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_first_lease, mut first) = open(&host, scope(dir.path())).await;
    let (_second_lease, mut second) = open(&host, scope(dir.path())).await;
    let id = start(&mut first, PATTERN).await;
    until_alive(PATTERN).await;

    let blocked = second.call(
        "command_run",
        json!({"class": "build", "command": "echo after"}),
    );
    let release = async {
        let started = Instant::now();
        loop {
            let rows = store
                .command_runs(ids::STEP_R2_PRD)
                .await
                .expect("command runs");
            if rows
                .iter()
                .any(|row| row.id.to_string() != id && row.status == CommandRunStatus::Queued)
            {
                break;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "the blocking call never queued"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let rows = store
            .command_runs(ids::STEP_R2_PRD)
            .await
            .expect("command runs");
        assert!(
            rows.iter()
                .any(|row| row.id.to_string() != id && row.status == CommandRunStatus::Queued),
            "the blocking call waits behind the started one"
        );
        ok(&call(&mut first, "command_cancel", json!({"command_run_id": id})).await)
    };
    let (blocked, cancelled) = tokio::join!(blocked, release);
    assert_eq!(cancelled["status"], json!("cancelled"));
    let blocked = ok(&blocked.expect("the call"));
    assert_eq!(blocked["output"], json!("after\n"));
    until_gone(PATTERN, Duration::from_secs(5)).await;
}

/// D7: at most four started commands are queued or running in one session.
#[tokio::test]
async fn a_fifth_started_command_is_refused() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let mut ids = Vec::new();
    for _ in 0..4 {
        ids.push(start(&mut client, "sleep 30.5327").await);
    }
    let answer = call(
        &mut client,
        "command_start",
        json!({"class": "build", "command": "echo five"}),
    )
    .await;
    assert_eq!(
        refused(&answer),
        "refused: 4 commands are already queued or running in this session"
    );
    for id in &ids {
        ok(&call(&mut client, "command_cancel", json!({"command_run_id": id})).await);
    }
    until_gone("sleep 30.5327", Duration::from_secs(5)).await;
    start(&mut client, "echo again").await;
}

/// D8: a deadline that is not an RFC 3339 time, and a pattern that does not compile, are refused.
#[tokio::test]
async fn bad_wait_arguments_are_refused() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = host(&store);
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let id = start(&mut client, "echo hi").await;
    let answer = call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": "tomorrow"}),
    )
    .await;
    assert_eq!(
        refused(&answer),
        "refused: deadline `tomorrow` is not an RFC 3339 time"
    );
    let answer = call(
        &mut client,
        "command_wait",
        json!({"command_run_id": id, "deadline": deadline(5), "pattern": "("}),
    )
    .await;
    assert!(
        refused(&answer).starts_with("refused: pattern: "),
        "{}",
        answer.text
    );
    let other = Uuid::now_v7();
    let answer = call(
        &mut client,
        "command_wait",
        json!({"command_run_id": other, "deadline": deadline(5)}),
    )
    .await;
    assert_eq!(refused(&answer), format!("not found: command_run {other}"));
}
