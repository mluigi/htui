//! `htui mcp` end to end (MOD-11 D6, blueprint §8.3, B-1, B-13): the real binary, started the
//! way an agent starts it — from the lease's spec, with a clean environment holding only `PATH`
//! and the two variables — relays its stdio to an `McpHost<Backend>` over
//! `Backend::memory(MemStore::demo())` through the host's Unix socket.
//!
//! Every child is `kill_on_drop`, so a failed assertion still reaps it.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use htui_core::fixtures::ids;
use htui_core::model::{RunId, StepId, Transport};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::{MemStore, StepFence};
use htui_mcp::channel::Refusal;
use htui_mcp::{ENV_ADDR, ENV_TOKEN, McpHost, Token};
use htui_orch::tools::{ToolHost, ToolLease, ToolScope};
use htui_store::Backend;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

const BUDGET: Duration = Duration::from_secs(20);
const HTUI: &str = env!("CARGO_BIN_EXE_htui");

fn host() -> McpHost<Backend> {
    McpHost::new(Backend::memory(MemStore::demo()))
        .expect("a host")
        .with_binary(PathBuf::from(HTUI))
}

fn scope() -> ToolScope {
    ToolScope {
        run_id: RunId::new(),
        step_id: StepId::new(),
        project_id: ids::PROJECT_HTUI,
        item_id: None,
        box_id: ids::BOX,
        user: ids::USER,
        fence: StepFence::Unleased,
        output_kind: None,
        hostname: HostnameLine::Omitted,
        command_queue: false,
        cwd: std::env::temp_dir(),
        transport: Transport::Acp,
    }
}

/// `htui mcp` with nothing in its environment but `PATH` and `env`.
fn relay<'a>(env: impl IntoIterator<Item = (&'a str, &'a str)>) -> Child {
    let mut command = Command::new(HTUI);
    command
        .arg("mcp")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.spawn().expect("htui mcp spawns")
}

/// The relay the lease describes, started exactly as the agent would start it.
fn relay_for(lease: &ToolLease) -> Child {
    assert_eq!(
        lease.spec.command, HTUI,
        "the host hands out its own binary"
    );
    assert_eq!(lease.spec.args, ["mcp"]);
    relay(
        lease
            .spec
            .env
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )
}

/// Closes stdin and waits for the child: its exit status, its whole stdout and its whole stderr.
async fn finish(mut child: Child) -> (ExitStatus, String, String) {
    drop(child.stdin.take());
    let output = tokio::time::timeout(BUDGET, child.wait_with_output())
        .await
        .expect("htui mcp exits in time")
        .expect("wait");
    (
        output.status,
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    )
}

/// One request in, its response out: the agent's lockstep (a request still in flight when stdin
/// closes is aborted, D2).
async fn exchange(
    child: &mut Child,
    stdout: &mut Lines<BufReader<ChildStdout>>,
    seen: &mut Vec<String>,
    request: Value,
) -> Value {
    let stdin = child.stdin.as_mut().expect("stdin is piped");
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .expect("write");
    stdin.flush().await.expect("flush");
    let line = tokio::time::timeout(BUDGET, stdout.next_line())
        .await
        .expect("an answer in time")
        .expect("read")
        .expect("a line before EOF");
    seen.push(line.clone());
    let answer: Value = serde_json::from_str(&line).expect("a JSON line");
    assert_eq!(answer["id"], request["id"], "{answer}");
    answer
}

/// A JSON-RPC 2.0 response: `jsonrpc`, an `id`, and exactly one of `result` / `error`.
fn assert_response(line: &str) {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
    let object = value.as_object().unwrap_or_else(|| panic!("{line}"));
    assert_eq!(object.get("jsonrpc"), Some(&json!("2.0")), "{line}");
    assert!(object.contains_key("id"), "{line}");
    assert!(
        object.contains_key("result") ^ object.contains_key("error"),
        "{line}"
    );
}

#[tokio::test]
async fn the_relay_serves_initialize_list_and_box_profile() {
    let host = host();
    let lease = host.open(scope()).expect("a lease");
    let mut child = relay_for(&lease);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let mut seen = Vec::new();

    let init = exchange(
        &mut child,
        &mut stdout,
        &mut seen,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                          "clientInfo": {"name": "mcp_stdio"}}}),
    )
    .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25", "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "htui", "{init}");

    // A notification is relayed and answered by nothing.
    let stdin = child.stdin.as_mut().expect("stdin is piped");
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .expect("write");

    let list = exchange(
        &mut child,
        &mut stdout,
        &mut seen,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("{list}"))
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"box_profile"), "{names:?}");

    let call = exchange(
        &mut child,
        &mut stdout,
        &mut seen,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "box_profile", "arguments": {}}}),
    )
    .await;
    assert_eq!(call["result"]["isError"], false, "{call}");
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{call}"));
    assert!(!text.is_empty(), "{call}");

    // The agent closing stdin ends the relay cleanly, and nothing but protocol reached stdout.
    let stdin = child.stdin.take().expect("stdin is piped");
    drop(stdin);
    while let Some(line) = tokio::time::timeout(BUDGET, stdout.next_line())
        .await
        .expect("EOF in time")
        .expect("read")
    {
        seen.push(line);
    }
    let (status, rest, stderr) = finish(child).await;
    assert_eq!(rest, "", "stdout was taken whole above");
    assert_eq!(seen.len(), 3, "three requests, three responses: {seen:?}");
    for line in &seen {
        assert_response(line);
    }
    assert_eq!(status.code(), Some(0), "{status}; stderr: {stderr}");
    assert_eq!(stderr, "", "a clean relay says nothing");
    drop(lease);
}

#[tokio::test]
async fn a_refused_token_exits_3_with_the_reason_on_stderr() {
    let host = host();
    let lease = host.open(scope()).expect("a lease");
    let stranger = Token::mint();
    let child = relay([
        (ENV_ADDR, lease.spec.env[ENV_ADDR].as_str()),
        (ENV_TOKEN, stranger.as_str()),
    ]);
    let (status, stdout, stderr) = finish(child).await;
    assert_eq!(status.code(), Some(3), "{status}; stderr: {stderr}");
    assert_eq!(stdout, "", "a refusal writes nothing on stdout");
    assert!(
        stderr.contains(&Refusal::UnknownToken.to_string()),
        "{stderr}"
    );
    assert!(
        !stderr.contains(stranger.as_str()),
        "stderr never names the token"
    );
    drop(lease);
}

#[tokio::test]
async fn missing_env_exits_2() {
    let token = Token::mint();
    for env in [
        vec![],
        vec![(ENV_TOKEN, token.as_str())],
        vec![(ENV_ADDR, "/nonexistent/htui-mcp/s")],
        vec![
            (ENV_ADDR, "/nonexistent/htui-mcp/s"),
            (ENV_TOKEN, "not-a-token"),
        ],
    ] {
        let child = relay(env.clone());
        let (status, stdout, stderr) = finish(child).await;
        assert_eq!(
            status.code(),
            Some(2),
            "{env:?}: {status}; stderr: {stderr}"
        );
        assert_eq!(stdout, "", "{env:?}");
        assert!(
            stderr.contains("never by hand") || stderr.contains(ENV_TOKEN),
            "{stderr}"
        );
        assert!(!stderr.contains("not-a-token"), "{stderr}");
    }
}

#[tokio::test]
async fn a_dead_host_exits_1() {
    let host = host();
    let lease = host.open(scope()).expect("a lease");
    let addr = lease.spec.env[ENV_ADDR].clone();
    let token = lease.spec.env[ENV_TOKEN].clone();
    host.close();
    assert!(
        !std::path::Path::new(&addr).exists(),
        "the socket is removed"
    );

    let child = relay([(ENV_ADDR, addr.as_str()), (ENV_TOKEN, token.as_str())]);
    let (status, stdout, stderr) = finish(child).await;
    assert_eq!(status.code(), Some(1), "{status}; stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("cannot reach the htui host"), "{stderr}");
    assert!(!stderr.contains(&token), "stderr never names the token");
    drop(lease);
}

/// A refused relay exits at once, with the agent still holding its stdin open: it never waits on
/// the agent to end (no orphan `htui mcp`).
#[tokio::test]
async fn a_refused_relay_exits_with_stdin_still_open() {
    let host = host();
    let lease = host.open(scope()).expect("a lease");
    let stranger = Token::mint();
    let mut child = relay([
        (ENV_ADDR, lease.spec.env[ENV_ADDR].as_str()),
        (ENV_TOKEN, stranger.as_str()),
    ]);
    // `Child::wait` closes a stdin it still holds: the agent's end is held here instead.
    let stdin = child.stdin.take().expect("stdin is piped");
    let status = tokio::time::timeout(BUDGET, child.wait())
        .await
        .expect("exits without stdin closing")
        .expect("wait");
    assert_eq!(status.code(), Some(3));
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("stdout is piped")
        .read_to_string(&mut stdout)
        .await
        .expect("read");
    assert_eq!(stdout, "");
    drop(stdin);
    drop(lease);
}

/// The host ending the session ends the relay at once, with the agent still holding its stdin
/// open: the exit is not held by the blocking read on stdin for the runtime's teardown grace
/// (`htui::SHUTDOWN`), so the agent sees its MCP server's stdout close promptly.
#[tokio::test]
async fn the_host_ending_the_session_ends_the_relay_with_stdin_still_open() {
    let host = host();
    let lease = host.open(scope()).expect("a lease");
    let mut child = relay_for(&lease);
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let mut seen = Vec::new();
    // One exchange: the relay is spliced, and its stdin read is in flight.
    exchange(
        &mut child,
        &mut stdout,
        &mut seen,
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
               "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                          "clientInfo": {"name": "mcp_stdio"}}}),
    )
    .await;

    // `Child::wait` closes a stdin it still holds: the agent's end is held here instead.
    let stdin = child.stdin.take().expect("stdin is piped");
    let started = std::time::Instant::now();
    host.close();
    let status = tokio::time::timeout(BUDGET, child.wait())
        .await
        .expect("exits without stdin closing")
        .expect("wait");
    let elapsed = started.elapsed();
    assert_eq!(status.code(), Some(0), "{status}");
    assert!(
        elapsed < htui::SHUTDOWN / 2,
        "the relay left {elapsed:?} after the host ended the session"
    );
    drop(stdin);
    drop(lease);
}
