//! MOD-152 T4: `command_output_read` through `McpHost`, and the log every `command_run` leaves
//! (D1-D5). Each case hosts `McpHost<Backend>` over a `MemStore::demo()`; the commands are `sh`
//! one-liners in a scratch directory.

#![cfg(unix)]

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use htui_core::fixtures::ids;
use htui_core::model::{
    COMMAND_LOG_CAP, CommandLog, CommandRunId, CommandRunStatus, NewCommandRun, Transport,
};
use htui_core::prompt::render::HostnameLine;
use htui_core::scrub::{Scrubber, Unmasked};
use htui_core::store::{MemStore, StepFence, WriteStore};
use htui_mcp::protocol::CallResult;
use htui_mcp::{ENV_TOKEN, McpClient, McpHost};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};

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

/// Runs `command` as a `build` and answers the call's JSON.
async fn run(client: &mut McpClient, command: &str) -> Value {
    ok(&client
        .call("command_run", json!({"class": "build", "command": command}))
        .await
        .expect("the call"))
}

/// A `done` row of `step` with no log, as another session left it.
fn row_of(step: htui_core::model::StepId) -> NewCommandRun {
    NewCommandRun {
        id: CommandRunId::new(),
        run_step_id: step,
        box_id: ids::BOX,
        class: "build".to_owned(),
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

/// `command_output_read` with `arguments`.
async fn read(client: &mut McpClient, arguments: Value) -> CallResult {
    client
        .call("command_output_read", arguments)
        .await
        .expect("the call")
}

/// A scrubber that masks nothing in a command line and refuses every other text.
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

/// D4: a `command_run` leaves its merged output as a log its own run can read, by default the
/// run's latest row.
#[tokio::test]
async fn a_command_run_leaves_a_log_the_run_can_read() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store.clone())).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let answer = run(&mut client, "echo a; echo b; echo c; exit 3").await;
    let id = answer["command_run_id"].as_str().expect("the row's id");

    let latest = ok(&read(&mut client, json!({})).await);
    assert_eq!(latest["command_run_id"], json!(id));
    assert_eq!(latest["status"], json!("done"));
    assert_eq!(latest["exit_code"], json!(3));
    assert_eq!(latest["text"], json!("a\nb\nc\n"));
    assert_eq!(latest["total_lines"], json!(3));
    assert_eq!(latest["log_truncated"], json!(false));
    assert_eq!(latest["first_line"], json!(1));
    assert_eq!(latest["last_line"], json!(3));
    assert_eq!(latest["capped"], json!(false));

    let named = ok(&read(&mut client, json!({"command_run_id": id})).await);
    assert_eq!(named, latest, "naming the row reads the same log");

    let stored = store
        .command_log_of_run(ids::RUN_2, None)
        .await
        .expect("the read")
        .expect("a row");
    let log = stored.log.expect("the runner wrote a log");
    assert_eq!(log.text, "a\nb\nc\n");
    assert!(!log.truncated);
    assert_eq!(log.total_bytes, 6);
}

/// D2: a log keeps the tail, opens with the truncation line, and starts on a line boundary.
#[tokio::test]
async fn a_long_log_keeps_a_capped_tail_on_a_line_boundary() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store.clone())).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    let answer = run(&mut client, "seq 1 500000").await;
    assert_eq!(answer["status"], json!("done"));

    let log = store
        .command_log_of_run(ids::RUN_2, None)
        .await
        .expect("the read")
        .and_then(|stored| stored.log)
        .expect("a log");
    assert!(log.truncated);
    assert!(
        log.text.len() <= COMMAND_LOG_CAP + 4096,
        "{}",
        log.text.len()
    );
    let mut lines = log.text.lines();
    let first: u64 = lines
        .nth(1)
        .and_then(|line| line.parse().ok())
        .expect("a whole number opens the tail");
    assert!(first > 1, "the head was dropped");
    // MOD-152 R1 F2: the truncation line says which lines it stands for.
    assert_eq!(
        log.text.lines().next(),
        Some(format!("[… earlier output truncated] (lines 1-{})", first - 1).as_str())
    );
    assert!(log.text.ends_with("500000\n"));
    assert!(log.total_bytes > COMMAND_LOG_CAP as u64);

    let tail = ok(&read(&mut client, json!({"tail_lines": 2})).await);
    assert_eq!(tail["text"], json!("499999\n500000\n"));
    assert_eq!(tail["log_truncated"], json!(true));
    // Lines are the output's own: line N of `seq` is N.
    assert_eq!(tail["first_line"], json!(499_999));
    assert_eq!(tail["total_lines"], json!(500_000));
    let range = ok(&read(
        &mut client,
        json!({"from_line": first, "to_line": first + 1}),
    )
    .await);
    assert_eq!(
        range["text"],
        json!(format!("{first}\n{}\n", first + 1)),
        "{range}"
    );
    let marker = ok(&read(&mut client, json!({"from_line": 1, "to_line": first})).await);
    assert_eq!(marker["first_line"], json!(first - 1), "{marker}");
    assert!(
        marker["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("[… earlier output truncated] (lines 1-")),
        "{marker}"
    );
}

/// A tail read answers the last lines, in order, with their numbers.
#[tokio::test]
async fn the_tail_is_the_default_and_counts_lines_from_the_end() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store)).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(&mut client, "seq 1 10").await;

    let tail = ok(&read(&mut client, json!({"tail_lines": 3})).await);
    assert_eq!(tail["text"], json!("8\n9\n10\n"));
    assert_eq!(tail["first_line"], json!(8));
    assert_eq!(tail["last_line"], json!(10));
    assert_eq!(tail["total_lines"], json!(10));

    let all = ok(&read(&mut client, json!({})).await);
    assert_eq!(
        all["first_line"],
        json!(1),
        "200 lines is more than there are"
    );
    assert_eq!(all["last_line"], json!(10));
}

/// A range read answers `from_line` to `to_line`, both included.
#[tokio::test]
async fn a_range_is_inclusive_and_clamped_to_the_log() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store)).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(&mut client, "seq 1 10").await;

    let mid = ok(&read(&mut client, json!({"from_line": 2, "to_line": 4})).await);
    assert_eq!(mid["text"], json!("2\n3\n4\n"));
    assert_eq!(
        (mid["first_line"].clone(), mid["last_line"].clone()),
        (json!(2), json!(4))
    );

    let past = ok(&read(&mut client, json!({"from_line": 9, "to_line": 99})).await);
    assert_eq!(past["text"], json!("9\n10\n"));
    assert_eq!(past["last_line"], json!(10));

    let beyond = ok(&read(&mut client, json!({"from_line": 50, "to_line": 60})).await);
    assert_eq!(beyond["text"], json!(""));
    assert_eq!(beyond["first_line"], Value::Null);
    assert_eq!(beyond["last_line"], Value::Null);

    assert_eq!(
        refused(&read(&mut client, json!({"from_line": 5, "to_line": 4})).await),
        "refused: from_line 5 is after to_line 4"
    );
    assert_eq!(
        refused(&read(&mut client, json!({"from_line": 0, "to_line": 4})).await),
        "refused: from_line starts at 1"
    );
}

/// A pattern read answers the matching lines with `context` lines around them, numbered
/// `N:` for a match and `N-` for context.
#[tokio::test]
async fn a_pattern_answers_matches_with_context() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store)).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(
        &mut client,
        "printf 'a\\nb\\nneedle\\nc\\nd\\ne\\nf\\ng\\nneedle two\\nh\\n'",
    )
    .await;

    let one = ok(&read(&mut client, json!({"pattern": "^needle$", "context": 1})).await);
    assert_eq!(one["text"], json!("2-b\n3:needle\n4-c\n"));
    assert_eq!(one["first_line"], json!(2));
    assert_eq!(one["last_line"], json!(4));

    let both = ok(&read(&mut client, json!({"pattern": "needle", "context": 1})).await);
    assert_eq!(
        both["text"],
        json!("2-b\n3:needle\n4-c\n--\n8-g\n9:needle two\n10-h\n"),
        "groups that do not touch are told apart"
    );

    let default_context = ok(&read(&mut client, json!({"pattern": "^f$"})).await);
    assert_eq!(
        default_context["text"],
        json!("5-d\n6-e\n7:f\n8-g\n9-needle two\n"),
        "two lines of context by default"
    );

    let none = ok(&read(&mut client, json!({"pattern": "absent"})).await);
    assert_eq!(none["text"], json!(""));

    let bad = read(&mut client, json!({"pattern": "("})).await;
    assert!(
        refused(&bad).starts_with("refused: pattern: "),
        "{}",
        bad.text
    );
    assert_eq!(
        refused(&read(&mut client, json!({"pattern": "a", "context": 21})).await),
        "refused: context 21 is not 0..=20"
    );
}

/// `max_bytes` bounds the answer and `capped` says it was cut.
#[tokio::test]
async fn max_bytes_caps_the_answer() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store)).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(&mut client, "seq 1 5000").await;

    let capped = ok(&read(&mut client, json!({"tail_lines": 5000, "max_bytes": 100})).await);
    let text = capped["text"].as_str().expect("text");
    assert!(text.len() <= 100, "{}", text.len());
    assert!(text.ends_with("5000\n"), "the tail is kept: {text:?}");
    assert_eq!(capped["capped"], json!(true));
    assert_eq!(capped["last_line"], json!(5000));

    let default = ok(&read(&mut client, json!({"tail_lines": 5000})).await);
    assert!(default["text"].as_str().expect("text").len() <= 16 * 1024);
    assert_eq!(default["capped"], json!(true), "16 KiB is the default cap");

    let range = ok(&read(
        &mut client,
        json!({"from_line": 1, "to_line": 5000, "max_bytes": 21}),
    )
    .await);
    assert_eq!(range["text"], json!("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"));
    assert_eq!(range["capped"], json!(true));
    assert_eq!(range["last_line"], json!(10), "a range keeps its head");

    assert_eq!(
        refused(&read(&mut client, json!({"max_bytes": 65537})).await),
        "refused: max_bytes 65537 is not 1..=65536"
    );
}

/// Exactly one mode, and the arguments of the others refused beside it.
#[tokio::test]
async fn the_modes_do_not_mix() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store)).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(&mut client, "echo hi").await;

    for arguments in [
        json!({"pattern": "h", "tail_lines": 3}),
        json!({"from_line": 1, "to_line": 2, "tail_lines": 3}),
        json!({"from_line": 1, "to_line": 2, "pattern": "h"}),
    ] {
        assert_eq!(
            refused(&read(&mut client, arguments).await),
            "refused: give one of pattern, from_line with to_line, or tail_lines"
        );
    }
    assert_eq!(
        refused(&read(&mut client, json!({"from_line": 1})).await),
        "refused: from_line and to_line go together"
    );
    assert_eq!(
        refused(&read(&mut client, json!({"context": 1})).await),
        "refused: context goes with pattern"
    );
    assert_eq!(
        refused(&read(&mut client, json!({"tail_lines": 0})).await),
        "refused: tail_lines must be 1 or more"
    );
    let unknown = read(&mut client, json!({"run_id": "x"})).await;
    assert!(
        refused(&unknown).starts_with("invalid arguments: unknown field `run_id`"),
        "{}",
        unknown.text
    );
}

/// D5: the read is scoped to the caller's run: another run's row is `not found`, and so is a
/// row of the run that has no log (yet).
#[tokio::test]
async fn only_the_callers_runs_logs_are_read() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store.clone())).expect("a host");
    let (_lease, mut client) = open(&host, scope(dir.path())).await;

    assert_eq!(
        refused(&read(&mut client, json!({})).await),
        "not found: command log",
        "the run has no command row"
    );

    let foreign = store
        .enqueue_command(row_of(ids::STEP_PRD))
        .await
        .expect("a row of RUN_1");
    store
        .put_command_log(CommandLog {
            command_run_id: foreign.id,
            text: "another run's output\n".to_owned(),
            truncated: false,
            total_bytes: 21,
            written_at: Utc::now(),
        })
        .await
        .expect("a log");
    assert_eq!(
        refused(&read(&mut client, json!({"command_run_id": foreign.id})).await),
        "not found: command log"
    );

    let bare = store
        .enqueue_command(row_of(ids::STEP_R2_PRD))
        .await
        .expect("a row of RUN_2");
    assert_eq!(
        refused(&read(&mut client, json!({"command_run_id": bare.id})).await),
        "not found: command log",
        "a row without a log"
    );
    assert_eq!(
        refused(&read(&mut client, json!({"command_run_id": CommandRunId::new()})).await),
        "not found: command log"
    );

    let own = store
        .enqueue_command(row_of(ids::STEP_R2_PRD))
        .await
        .expect("a row of RUN_2");
    store
        .put_command_log(CommandLog {
            command_run_id: own.id,
            text: "an earlier step's gate\n".to_owned(),
            truncated: false,
            total_bytes: 23,
            written_at: Utc::now(),
        })
        .await
        .expect("a log");
    let answer = ok(&read(&mut client, json!({"command_run_id": own.id})).await);
    assert_eq!(answer["text"], json!("an earlier step's gate\n"));
}

/// D3: a log that cannot be masked is withheld by rule, and so is the read of it.
#[tokio::test]
async fn an_unmaskable_log_is_withheld_by_rule() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let store = MemStore::demo();
    let host = McpHost::new(Backend::memory(store.clone()))
        .expect("a host")
        .with_scrubber(Arc::new(RefusesOutput));
    let (_lease, mut client) = open(&host, scope(dir.path())).await;
    run(&mut client, "echo hi").await;

    let log = store
        .command_log_of_run(ids::RUN_2, None)
        .await
        .expect("the read")
        .and_then(|stored| stored.log)
        .expect("a log");
    assert_eq!(
        log.text,
        "[log withheld: it matched credential rule test_rule]"
    );
    let answer = ok(&read(&mut client, json!({})).await);
    assert_eq!(
        answer["text"],
        json!("[log withheld: it matched credential rule test_rule]")
    );
}

/// `command_output_read` is advertised with `command_run` and not without it.
#[tokio::test]
async fn it_is_advertised_with_the_queue() {
    let dir = tempfile::tempdir().expect("a scratch cwd");
    let host = McpHost::new(Backend::memory(MemStore::demo())).expect("a host");
    for (exposed, expected) in [(true, true), (false, false)] {
        let (_lease, mut client) = open(
            &host,
            ToolScope {
                command_queue: exposed,
                ..scope(dir.path())
            },
        )
        .await;
        let listed = client
            .request("tools/list", json!({}))
            .await
            .expect("tools/list");
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(
            names.contains(&"command_output_read"),
            expected,
            "{names:?}"
        );
    }
}
