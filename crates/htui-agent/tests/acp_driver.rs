//! What the driver spawns (plan MOD-2 D58), and what [`open_session`] leaves behind when the
//! handshake does not finish (D61).
//!
//! Two questions in one file because they share a subject: the process an ACP session starts. D61
//! asks whether it is gone once the handshake has failed; D58 asks whether it was the right process
//! in the first place, argument for argument.
//!
//! **D61** — the three cases at the top start a real child and then make the handshake fail in the
//! three distinct ways it can: a transport that is open and silent (the `timeout` arm), one that
//! refuses `initialize` (the `Ok(Ok(Err))` arm, reported by the foreground future), and one that
//! completes `initialize` and refuses `session/new` (the same arm, but composed by the connection
//! future, because the SDK sends that request from an actor and an actor that fails drops the
//! foreground). Every one asserts the child is gone, because "the chat start failed" and "the
//! adapter is still on this box" are two different facts and until D61 only the first was ever
//! reported. Two of them assert it was *reaped* and the silent one only that it was signalled —
//! the asymmetry is blueprint H-1's and is deliberate, so each case says which it is asserting.
//!
//! The third case also asserts the *message*, which the other two do only in passing: on an
//! unauthenticated box the refusal is the vendor's own `Authentication required`, and reporting it
//! instead of "the session task ended before the handshake" is the entire user-visible difference
//! between a box you can fix and one that looks broken.
//!
//! **D58** — a glob tool's per-platform `args`, `agy`'s Linux-only `--uid=`, exist only on the
//! probe's path: a `ToolMap` value is one string and cannot carry them, so a chat resolving for
//! itself launches the adapter without its argument (blueprint H-3). The fix is that the driver
//! spawns what the probe recorded when the snapshot is usable. **Every row below declares a
//! discovery that cannot resolve on this box, or one that resolves to a file the case wrote
//! itself** — that is what makes a marker argument coming back out of `launch_for` evidence the
//! snapshot was read, rather than something resolution could have produced on its own.
//!
//! `cfg(unix)` per case rather than over the file: the D58 cases that stop at `launch_for` spawn
//! nothing and are portable, and MOD-16 should find them already running. What stays gated is what
//! needs a real child — `sleep`, `/proc`'s process state, and the `/bin/sh` script that reports its
//! own argv.
//!
//! Blueprint H-18 fixture rule: no case here touches a seed row. The D61 child is `sleep`, a system
//! binary with no `ETXTBSY` window and nothing to install, and it is deliberately *not* the
//! transport — a session's child and a session's byte streams are separable in [`AcpIo`], and
//! separating them here is what makes the assertion about the child alone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::AcpDriver;
use htui_agent::acp::Handshake;
use htui_agent::acp::isolation_meta;
#[cfg(unix)]
use htui_agent::acp::{
    AcpIo, SESSION_STARTED, SessionOptions, Stamp, TOOL_KIND_DENIED, open_session,
};
use htui_agent::driver::DriverFuture;
#[cfg(unix)]
use htui_agent::driver::McpServerSpec;
#[cfg(unix)]
use htui_agent::driver::{AgentDriver, AgentSession, AgentSessionRef};
use htui_agent::driver::{PermissionPolicy, SessionSpec, ToolExposure};
#[cfg(unix)]
use htui_agent::error::DriverError;
#[cfg(unix)]
use htui_agent::event::{
    DoneEvent, DriverEvent, EditProposalEvent, ErrorEvent, StopReason, ToolKind,
};
use htui_agent::launch::AcpSettings;
use htui_agent::launch::ResolvedLaunch;
use htui_agent::launch::SESSION_VAR;
#[cfg(unix)]
use htui_agent::launch::{AgentSettings, SessionSettings, spawn};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeSnapshot, ProbeSource, ProbeStatus, Tier2, agent_box_row,
    probe_snapshot,
};
#[cfg(unix)]
use htui_agent::registry::DriverFactory;
use htui_agent::registry::caps_for;
use htui_core::model::{Agent as AgentRow, AgentBox, AgentId, Billing, BoxId, StepId, Transport};
use serde_json::{Value, json};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::sync::oneshot;

/// The buffer each half of the in-process pipe gets, as `acp_conformance.rs` sizes it.
#[cfg(unix)]
const DUPLEX_BYTES: usize = 64 * 1024;

/// Short enough that a timeout case is a test rather than a coffee break, long enough that a
/// loaded box still gets the spawn and the first write done inside it.
#[cfg(unix)]
const SHORT_HANDSHAKE: Duration = Duration::from_millis(300);

/// The window a case that is **not** about the timeout gives the handshake.
///
/// A refusal case has to reach its assertion through the arm that reports the agent's answer, and
/// the timeout arm reports something else entirely: on a loaded box [`SHORT_HANDSHAKE`] would turn
/// a message regression into a timeout and hide which of the two broke. Ten seconds is never spent
/// — the transport is an in-process duplex and the answer is one write away — it is only the
/// distance between "the agent refused" and "nobody answered".
#[cfg(unix)]
const PATIENT_HANDSHAKE: Duration = Duration::from_secs(10);

/// A spec over `cwd` and nothing else: no case here reaches `session/new`, so every field past the
/// working directory is the default the contract test already pins.
fn spec(cwd: PathBuf) -> SessionSpec {
    SessionSpec {
        agent_id: AgentId::new(),
        step_id: StepId::new(),
        cwd,
        extra_dirs: Vec::new(),
        env: BTreeMap::new(),
        model: None,
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        retain_raw: false,
        resume: None,
        budget_micros: None,
        prompt: None,
    }
}

/// Options over a handshake window the case chooses, instead of the production minute.
#[cfg(unix)]
fn options(handshake_timeout: Duration) -> SessionOptions {
    SessionOptions {
        agent_name: "fixture".to_owned(),
        settings: AgentSettings::default(),
        stamp: Stamp::Wall,
        handshake_timeout,
    }
}

/// How long a signalled process is given to stop being one.
///
/// A `Drop` can only *send* the kill (`ChildGuard`'s own doc), and what the signal then costs is
/// the kernel's business: on this box the group takes single-digit milliseconds to leave `R`. The
/// window is three orders of magnitude larger than that and finite, so a regression is a named
/// failure in two seconds rather than a hung suite.
#[cfg(unix)]
const KILL_WINDOW: Duration = Duration::from_secs(2);

/// Fails unless `pid` is gone **or** reaped-pending within [`KILL_WINDOW`]: a killed child that
/// nobody has waited for is a zombie, which is dead by every measure this assertion is making
/// (blueprint H-1 — the timeout arm signals and does not reap).
///
/// Linux-only because `/proc` is; the pid is the one identifier a kill can be checked against that
/// nothing else can accidentally answer to (`launch.rs`'s `pid()` doc, and the `pgrep` warning in
/// `acp_live.rs`). Copied rather than shared, as `tests/probe.rs` and `tests/probe_live.rs` copy
/// their process helpers — a case's own assertion living in its own file is what lets one of them
/// change without the others being re-read.
#[cfg(unix)]
async fn assert_not_running(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + KILL_WINDOW;
        loop {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return;
            };
            // `pid (comm) state …`, and `comm` may hold spaces and parens: the state is the first
            // field after the **last** `)`.
            let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
            let state = after.trim().chars().next().unwrap_or('Z');
            if state == 'Z' || state == 'X' {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: pid {pid} is still running (state {state})"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

/// Fails unless `pid` has been **reaped** within [`KILL_WINDOW`]: no `/proc/{pid}` at all, which a
/// zombie still has.
///
/// The stronger half of [`assert_not_running`], and the two are not interchangeable.
/// `assert_not_running` accepts state `Z`, so it cannot tell a child that was *signalled* from one
/// that was signalled **and waited for** — and "reaped" is the whole claim of the exits that own an
/// awaitable path to `ChildGuard::kill_and_reap`. `tests/agy_live.rs` asserts the same way, on the
/// same directory, after its own kill.
///
/// Bounded rather than immediate even though `kill_and_reap` awaits the `wait`: the exits under
/// test reap on the *task's* timeline and `open_session` waits for that task, so the entry is
/// normally gone before this is called. The window is what turns a regression into a named failure
/// in two seconds instead of a flake.
#[cfg(unix)]
async fn assert_reaped(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + KILL_WINDOW;
        while Path::new(&format!("/proc/{pid}")).exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: pid {pid} was not reaped"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

/// An agent that answers the client's first request — `initialize` — with a JSON-RPC **error**,
/// then reads until the client goes away.
///
/// Raw newline-delimited JSON-RPC, importing no SDK type, for the reason `acp_conformance.rs`
/// gives: two ends sharing a library prove the library, not the protocol. It has to be a
/// well-formed error response rather than a line of junk — the client's decoder skips what it
/// cannot parse and keeps waiting, so junk is the *timeout* case above, not this one.
#[cfg(unix)]
async fn refuse_first_request(stream: tokio::io::DuplexStream) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let Ok(Some(line)) = lines.next_line().await else {
        return;
    };
    let request: Value = serde_json::from_str(&line).expect("the client speaks JSON-RPC");
    let response = json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "error": { "code": -32601, "message": "this agent refuses to initialize" },
    });
    let mut text = serde_json::to_string(&response).expect("the response serialises");
    text.push('\n');
    if writer.write_all(text.as_bytes()).await.is_err() {
        return;
    }
    let _ = writer.flush().await;
    while let Ok(Some(_)) = lines.next_line().await {}
}

/// The text the `session/new` fixture refuses with.
///
/// Deliberately unlike anything `htui` writes: every string in the driver's own handshake arms is
/// either a step name or a `DriverError` phrasing, so a message containing this came off the wire
/// and could not have been composed on this side of it. It is shaped like the refusal an
/// unauthenticated box actually gets, which is the whole point of the case.
#[cfg(unix)]
const VENDOR_REFUSAL: &str = "Authentication required. Please run `fixture login` first.";

/// An agent that **completes** `initialize` and then refuses everything after it.
///
/// The distinction from [`refuse_first_request`] is the finding this case pins: `initialize` is
/// sent from the foreground future, so its error comes back to the arm that reports it, while
/// `SessionBuilder::start_session` sends `session/new` from a task it spawns *on the connection*
/// (`session.rs:885-905`). A JSON-RPC error there fails a connection actor, and
/// `run_until_connection_close` does `background_result?` while still holding the foreground
/// (`jsonrpc.rs:3556-3560`) — so the foreground is dropped and its own `session/new` error arm
/// never runs. What the caller is owed is the agent's text, and this is the only shape of agent
/// that can prove it arrives.
///
/// Raw newline-delimited JSON-RPC for [`refuse_first_request`]'s reason. Only `protocolVersion` is
/// a required field of `InitializeResponse`; the rest default, and none of them are read here.
#[cfg(unix)]
async fn refuse_session_new(stream: tokio::io::DuplexStream) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // A notification carries no `id` and is owed no answer; answering one would be a response
        // to `null`, which the client's decoder correlates to nothing.
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let response = if request.get("method").and_then(Value::as_str) == Some("initialize") {
            json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": 1 } })
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": VENDOR_REFUSAL },
            })
        };
        let mut text = serde_json::to_string(&response).expect("the response serialises");
        text.push('\n');
        if writer.write_all(text.as_bytes()).await.is_err() {
            return;
        }
        if writer.flush().await.is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// D61: the handshake's three failing exits
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn open_session_times_out_and_kills_its_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let child = spawn(
        &ResolvedLaunch {
            command: "sleep".to_owned(),
            args: vec!["1000".to_owned()],
            env: BTreeMap::new(),
        },
        tmp.path(),
    )
    .await
    .expect("`sleep` is on this box");
    let pid = child.pid().expect("a freshly spawned child has a pid");

    // The child owns no stream here: the point is a transport that is **open and silent**, which
    // is what an adapter that accepts the connection and never answers `initialize` looks like.
    // The agent end is held for the test's life on purpose — dropping it is an EOF, and an EOF is
    // the connection-closed arm, not the timeout one.
    let (client_end, _agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: Some(child),
    };

    let opened = open_session(
        io,
        spec(tmp.path().to_path_buf()),
        "hi".to_owned(),
        options(SHORT_HANDSHAKE),
    )
    .await;
    let message = match &opened {
        Err(DriverError::Transport(message)) => message,
        Err(other) => panic!("expected a transport error, got {other:?}"),
        Ok(_) => panic!("a silent agent does not open a session"),
    };
    assert!(
        message.contains("did not complete its handshake"),
        "the timeout says which step gave up: {message}"
    );

    // `assert_not_running` and not `assert_reaped`: this is the one exit that runs no code of
    // ours, so the kill comes from a `Drop` that cannot await the `wait` (blueprint H-1). A zombie
    // is the honest expectation here, and pinning it is what keeps the asymmetry deliberate.
    assert_not_running(pid, "the session's child").await;
}

#[cfg(unix)]
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "test fakes run under no TUI panic hook"
)]
async fn a_failed_handshake_still_reaps_its_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let child = spawn(
        &ResolvedLaunch {
            command: "sleep".to_owned(),
            args: vec!["1000".to_owned()],
            env: BTreeMap::new(),
        },
        tmp.path(),
    )
    .await
    .expect("`sleep` is on this box");
    let pid = child.pid().expect("a freshly spawned child has a pid");

    // The other failing exit: the agent is answering, and its answer is "no". That reaches
    // `open_session` as `Ok(Ok(Err))`, which already waits for the task — this case is here so the
    // two arms are asserted side by side and the one that was already right cannot regress
    // unnoticed while the other is being changed.
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(refuse_first_request(agent_end));
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: Some(child),
    };

    let opened = open_session(
        io,
        spec(tmp.path().to_path_buf()),
        "hi".to_owned(),
        options(PATIENT_HANDSHAKE),
    )
    .await;
    let message = match &opened {
        Err(DriverError::Transport(message)) => message,
        Err(other) => panic!("expected a transport error, got {other:?}"),
        Ok(_) => panic!("a refused `initialize` does not open a session"),
    };
    assert!(
        message.contains("initialize failed"),
        "the error names the step that was refused: {message}"
    );

    // Reaped, not merely signalled: this exit reports from the foreground future, which has an
    // awaitable path to `kill_and_reap`, and `open_session` waits for the task before returning.
    // `assert_not_running` would accept a `Z` here and could not tell the two apart, which is the
    // claim the case's own name makes.
    assert_reaped(pid, "the session's child").await;
}

/// The third failing exit, and the one an unauthenticated box takes: the agent answers
/// `initialize` and then refuses `session/new`, and the **agent's own text** is what the caller
/// gets.
///
/// Before this case the driver reported `"the session task ended before the handshake"` — the
/// fallback for "nobody ever answered" — because the refusal drops the foreground future rather
/// than returning to it (see [`refuse_session_new`]), and the vendor's message died in a `warn!`
/// on the connection future. For every `agy` box that is installed but not logged in, that was the
/// whole of what the user was told.
///
/// Two assertions, because either alone would pass for the wrong reason: the text proves the
/// message came off the wire, and the step name proves the driver still says *which* handshake
/// failed rather than answering with a bare vendor string.
#[cfg(unix)]
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "test fakes run under no TUI panic hook"
)]
async fn a_refused_session_new_answers_with_the_agents_own_message() {
    let tmp = tempfile::tempdir().expect("temp box");
    let child = spawn(
        &ResolvedLaunch {
            command: "sleep".to_owned(),
            args: vec!["1000".to_owned()],
            env: BTreeMap::new(),
        },
        tmp.path(),
    )
    .await
    .expect("`sleep` is on this box");
    let pid = child.pid().expect("a freshly spawned child has a pid");

    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(refuse_session_new(agent_end));
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: Some(child),
    };

    let opened = open_session(
        io,
        spec(tmp.path().to_path_buf()),
        "hi".to_owned(),
        options(PATIENT_HANDSHAKE),
    )
    .await;
    let message = match &opened {
        Err(DriverError::Transport(message)) => message,
        Err(other) => panic!("expected a transport error, got {other:?}"),
        Ok(_) => panic!("a refused `session/new` does not open a session"),
    };
    assert!(
        message.contains(VENDOR_REFUSAL),
        "the agent's own refusal is what the caller is told: {message}"
    );
    assert!(
        message.contains("session/new failed"),
        "and it still names the step that was refused: {message}"
    );

    // The connection future owns the guard on this path and kills through it after it has
    // answered, so the exit is as complete as the one above.
    assert_reaped(pid, "the session's child").await;
}

// ---------------------------------------------------------------------------------------------
// MOD-26 D11 (B-12): a persona's denied kinds at `htui`'s own `fs/*` handlers
// ---------------------------------------------------------------------------------------------

/// The session id [`fs_agent`] mints.
#[cfg(unix)]
const FS_SESSION: &str = "fs-session";

/// The JSON-RPC id of [`fs_agent`]'s one request to the client.
#[cfg(unix)]
const FS_REQUEST_ID: i64 = 100;

/// How long a case waits for an event or an answer before calling the driver stuck. Never spent:
/// the transport is an in-process duplex.
#[cfg(unix)]
const EVENT_WINDOW: Duration = Duration::from_secs(10);

/// An agent that, on `session/prompt`, sends the client `request` (an `fs/*` call) as id
/// [`FS_REQUEST_ID`] with its `sessionId` filled in, forwards the client's response line to
/// `answered`, then ends the turn `end_turn`.
///
/// Raw newline-delimited JSON-RPC in [`refuse_session_new`]'s style: answers `initialize` and
/// `session/new`, ignores every notification.
#[cfg(unix)]
async fn fs_agent(
    stream: tokio::io::DuplexStream,
    mut request: Value,
    answered: oneshot::Sender<Value>,
) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut answered = Some(answered);
    let mut prompt: Option<Value> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        let reply = match message.get("method").and_then(Value::as_str) {
            // A response: the client's answer to the `fs/*` request.
            None if id == Some(json!(FS_REQUEST_ID)) => {
                if let Some(answered) = answered.take() {
                    let _ = answered.send(message);
                }
                prompt.take().map(|prompt| {
                    json!({ "jsonrpc": "2.0", "id": prompt, "result": { "stopReason": "end_turn" } })
                })
            }
            Some("initialize") => {
                Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": 1 } }))
            }
            Some("session/new") => {
                Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": FS_SESSION } }))
            }
            Some("session/prompt") => {
                prompt = id;
                request["jsonrpc"] = json!("2.0");
                request["id"] = json!(FS_REQUEST_ID);
                request["params"]["sessionId"] = json!(FS_SESSION);
                Some(request.clone())
            }
            _ => None,
        };
        let Some(reply) = reply else {
            continue;
        };
        let mut text = serde_json::to_string(&reply).expect("the reply serialises");
        text.push('\n');
        if writer.write_all(text.as_bytes()).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
}

/// What one [`fs_agent`] turn left behind: the session's events up to its `done`, and the
/// client's JSON-RPC response to the `fs/*` request.
#[cfg(unix)]
struct FsTurn {
    events: Vec<DriverEvent>,
    response: Value,
}

/// Opens a session over [`fs_agent`] in `cwd` with `deny_kinds` on the spec (the default
/// settings advertise fs read and write), pulls one turn, then ends the session and the agent.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fakes run under no TUI panic hook"
)]
async fn fs_turn(cwd: &Path, deny_kinds: Vec<ToolKind>, request: Value) -> FsTurn {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    let (answered, response) = oneshot::channel();
    let agent = tokio::spawn(fs_agent(agent_end, request, answered));
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: None,
    };
    let mut spec = spec(cwd.to_path_buf());
    spec.tools.deny_kinds = deny_kinds;

    let mut session = open_session(io, spec, "hi".to_owned(), options(PATIENT_HANDSHAKE))
        .await
        .expect("the fs agent opens a session");
    let mut events = Vec::new();
    loop {
        let next = tokio::time::timeout(EVENT_WINDOW, session.next_event())
            .await
            .expect("the turn ends within the window")
            .expect("the transport stays healthy");
        let Some(envelope) = next else {
            break;
        };
        let done = matches!(envelope.event, DriverEvent::Done(_));
        events.push(envelope.event);
        if done {
            break;
        }
    }
    let response = tokio::time::timeout(EVENT_WINDOW, response)
        .await
        .expect("the client answered within the window")
        .expect("the agent forwarded the client's answer");

    drop(session);
    agent.abort();
    let _ = agent.await;
    FsTurn { events, response }
}

/// An `fs/*` request with a path and, for a write, its content.
#[cfg(unix)]
fn fs_request(method: &str, path: &Path, content: Option<&str>) -> Value {
    let mut params = json!({ "path": path });
    if let Some(content) = content {
        params["content"] = json!(content);
    }
    json!({ "method": method, "params": params })
}

#[cfg(unix)]
fn denials(events: &[DriverEvent]) -> Vec<&ErrorEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Error(error) if error.code == TOOL_KIND_DENIED => Some(error),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
fn proposals(events: &[DriverEvent]) -> Vec<&EditProposalEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::EditProposal(proposal) => Some(proposal),
            _ => None,
        })
        .collect()
}

/// D11, B-12: under a denied `edit`, `fs/write_text_file` is refused before `guard`, the read of
/// the old text and the `EditProposal`: the file keeps its bytes, the agent gets `invalid_params`
/// and the step records `tool_kind_denied`.
#[cfg(unix)]
#[tokio::test]
async fn a_denied_write_is_refused_before_any_edit_proposal() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, "before\n").expect("the fixture file");

    let turn = fs_turn(
        tmp.path(),
        vec![ToolKind::Edit],
        fs_request("fs/write_text_file", &file, Some("after\n")),
    )
    .await;

    assert_eq!(
        turn.response["error"]["code"], -32602,
        "invalid_params: {}",
        turn.response
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("the file is still there"),
        "before\n",
        "nothing touched the file"
    );
    let denials = denials(&turn.events);
    assert_eq!(denials.len(), 1, "{:?}", turn.events);
    assert!(
        denials[0].message.contains("edit") && denials[0].message.contains("fs/write_text_file"),
        "{}",
        denials[0].message
    );
    assert!(
        proposals(&turn.events).is_empty(),
        "no proposal for an edit that never happens: {:?}",
        turn.events
    );
}

/// D11, B-12: under a denied `read`, `fs/read_text_file` is refused whatever the path.
#[cfg(unix)]
#[tokio::test]
async fn a_denied_read_is_refused() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, "secret plans\n").expect("the fixture file");

    let turn = fs_turn(
        tmp.path(),
        vec![ToolKind::Read],
        fs_request("fs/read_text_file", &file, None),
    )
    .await;

    assert_eq!(
        turn.response["error"]["code"], -32602,
        "invalid_params: {}",
        turn.response
    );
    assert!(
        !turn.response.to_string().contains("secret plans"),
        "the content never reaches the agent: {}",
        turn.response
    );
    let denials = denials(&turn.events);
    assert_eq!(denials.len(), 1, "{:?}", turn.events);
    assert!(
        denials[0].message.contains("read") && denials[0].message.contains("fs/read_text_file"),
        "{}",
        denials[0].message
    );
}

/// D11: a kind the persona does not deny is untouched — a write under a denied `read` lands and
/// is proposed accepted, as before MOD-26.
#[cfg(unix)]
#[tokio::test]
async fn an_undenied_write_still_lands() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, "before\n").expect("the fixture file");

    let turn = fs_turn(
        tmp.path(),
        vec![ToolKind::Read],
        fs_request("fs/write_text_file", &file, Some("after\n")),
    )
    .await;

    assert!(
        turn.response.get("error").is_none(),
        "the write is answered: {}",
        turn.response
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("the file is there"),
        "after\n"
    );
    assert!(denials(&turn.events).is_empty(), "{:?}", turn.events);
    let proposals = proposals(&turn.events);
    assert_eq!(proposals.len(), 1, "{:?}", turn.events);
    assert_eq!(proposals[0].accepted, Some(true));
}

// ---------------------------------------------------------------------------------------------
// D58: the driver spawns what the probe recorded
// ---------------------------------------------------------------------------------------------

/// The argument no resolution on this box can produce.
///
/// `tools::resolve` answers one string per tool and `launch::resolve` substitutes it into the row's
/// own `args`; neither can invent an argument the row does not hold. So a marker in what
/// `launch_for` returns came from `agent_box.probe.resolved` and from nowhere else.
const MARKER: &str = "--marker=htui-d58";

/// A registry row whose command is a placeholder, plus the `discovery.tools` document that must
/// answer for it.
///
/// The shape `tests/probe.rs`'s `synthetic_row` uses, copied rather than shared: an integration
/// test is its own crate, and this repo duplicates fixture helpers per file so one can change
/// without the others being re-read.
fn row(tools: Value) -> AgentRow {
    let now = Utc::now();
    AgentRow {
        id: AgentId::new(),
        name: "probed".to_owned(),
        transport: Transport::Acp,
        launch: json!({
            "command": "${tool}",
            "args": [],
            "env": {},
            "discovery": { "handshake": true, "tools": tools },
        }),
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// A row whose one tool is on no `PATH` anywhere: resolution answers `Unresolved("tool")`, so a
/// case that gets a launch back got it from the snapshot.
fn unresolvable_row() -> AgentRow {
    row(json!({ "tool": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] } }))
}

/// A row that *does* resolve, to a file this case wrote under `<cwd>/node_modules`.
///
/// The `node_package` tier is the deterministic one: no `PATH`, no `npm` and no glob walk of a box
/// whose contents nobody controls, so the fallback's answer is a string the case already knows.
fn resolvable_row(cwd: &Path) -> AgentRow {
    let entry = entry_point(cwd);
    std::fs::create_dir_all(entry.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&entry, "// entry").expect("write");
    row(json!({
        "tool": {
            "kind": "node_package",
            "package": "@htui/fake",
            "entry": "dist/index.js",
            "pinned": "0.0.0",
        },
    }))
}

/// Where [`resolvable_row`] resolves to.
fn entry_point(cwd: &Path) -> PathBuf {
    cwd.join("node_modules/@htui/fake/dist/index.js")
}

/// A snapshot carrying `resolved`, and defaults for everything D58 does not read.
fn snapshot(source: ProbeSource, status: ProbeStatus, resolved: ResolvedLaunch) -> ProbeSnapshot {
    ProbeSnapshot {
        transport: Transport::Acp,
        resolved: Some(resolved),
        tools: BTreeMap::new(),
        handshake: None,
        credential: None,
        status,
        stderr_tail: None,
        source,
        manual: BTreeMap::new(),
    }
}

/// This box's `agent_box` row for `agent`, through the same projection the probe writes.
fn on_box(agent: &AgentRow, snapshot: &ProbeSnapshot) -> AgentBox {
    agent_box_row(agent, BoxId::new(), snapshot, Utc::now())
}

/// A file that exists and is not an adapter: [`AcpDriver::launch_for`]'s fourth rule is a
/// `metadata` call, and a case about `source` or `args` must not fail for want of a `stat`.
fn plain_file(path: &Path) -> String {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, "not an adapter, just a path that exists").expect("write");
    path.to_string_lossy().into_owned()
}

/// [`plain_file`] with `contents` and the execute bit: the one case that reaches `launch::spawn`
/// needs a fixture the kernel will actually run.
#[cfg(unix)]
fn executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// D58 at the seam that decides it: a usable snapshot is what the session would spawn, arguments
/// included, and `spec.env` is still applied last and still wins (`R-SEC-2`). `HTUI_SESSION=1`
/// (MOD-91 D4) is inserted after the spec's env, so it is in the map too.
///
/// `ready` and `unauthenticated` are both asserted here rather than left to the pure case, because
/// blueprint H-4 is a *driver* claim — an unauthenticated box spawns the recorded launch and lets
/// the vendor's own auth error be the message, and that only happens if this seam agrees.
#[tokio::test]
async fn a_usable_snapshot_is_what_the_driver_would_spawn_marker_included() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = unresolvable_row();
    let recorded = ResolvedLaunch {
        command: plain_file(&tmp.path().join("bin/htui-fake-adapter")),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::from([
            ("ROW_VAR".to_owned(), "row".to_owned()),
            ("SHARED".to_owned(), "row".to_owned()),
        ]),
    };

    let mut spec = spec(tmp.path().to_path_buf());
    spec.env = BTreeMap::from([
        ("SHARED".to_owned(), "spec".to_owned()),
        ("SECRET".to_owned(), "s".to_owned()),
    ]);

    for status in [ProbeStatus::Ready, ProbeStatus::Unauthenticated] {
        let row_on_box = on_box(
            &agent,
            &snapshot(ProbeSource::Probe, status, recorded.clone()),
        );
        let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
            .expect("the row's launch parses");
        let launch = driver
            .launch_for(&spec)
            .await
            .unwrap_or_else(|err| panic!("a {status:?} snapshot is usable, got {err:?}"));

        assert_eq!(launch.command, recorded.command, "status {status:?}");
        assert_eq!(
            launch.args,
            vec![MARKER.to_owned()],
            "the per-platform args live in the snapshot and nowhere else (blueprint H-3); \
             status {status:?}"
        );
        assert_eq!(
            launch.env,
            BTreeMap::from([
                ("HTUI_SESSION".to_owned(), "1".to_owned()),
                ("ROW_VAR".to_owned(), "row".to_owned()),
                ("SECRET".to_owned(), "s".to_owned()),
                ("SHARED".to_owned(), "spec".to_owned()),
            ]),
            "the row's environment holds paths and the spec's holds secrets, and the spec wins \
             on a key both hold (`R-SEC-2`); status {status:?}"
        );
    }
}

/// MOD-91 D4: what an ACP session would spawn carries `HTUI_SESSION=1` whether or not the row
/// opts back in, and neither the recorded launch's env nor the spec's can set it to anything
/// else: it is inserted after both. The adapter passes its `process.env` to the CLI it spawns
/// (`acp-agent.js:5915-5925`), so this is what reaches Claude Code. A login's launch
/// ([`AcpDriver::launch_in`]) is not a session and stays unmarked (blueprint B3).
#[tokio::test]
async fn launch_for_marks_the_session_whatever_the_setting() {
    let tmp = tempfile::tempdir().expect("temp box");
    let command = plain_file(&tmp.path().join("bin/htui-fake-adapter"));
    for user_extensions in [false, true] {
        for (row_env, spec_env) in [(None, None), (Some("0"), None), (None, Some("0"))] {
            let mut agent = unresolvable_row();
            agent.settings["user_extensions"] = json!(user_extensions);
            let recorded = ResolvedLaunch {
                command: command.clone(),
                args: vec![MARKER.to_owned()],
                env: row_env
                    .map(|value| BTreeMap::from([(SESSION_VAR.to_owned(), value.to_owned())]))
                    .unwrap_or_default(),
            };
            let row_on_box = on_box(
                &agent,
                &snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded),
            );
            let driver =
                AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
                    .expect("the row's launch parses");
            let mut with = spec(tmp.path().to_path_buf());
            if let Some(value) = spec_env {
                with.env.insert(SESSION_VAR.to_owned(), value.to_owned());
            }

            let resolved = driver
                .launch_for(&with)
                .await
                .expect("a ready snapshot is usable");
            assert_eq!(
                resolved.env.get(SESSION_VAR).map(String::as_str),
                Some("1"),
                "user_extensions {user_extensions}, row {row_env:?}, spec {spec_env:?}"
            );

            if row_env.is_none() {
                let login = driver
                    .launch_in(tmp.path())
                    .await
                    .expect("a ready snapshot is usable");
                assert_eq!(
                    login.env.get(SESSION_VAR),
                    None,
                    "a login is not a session (user_extensions {user_extensions})"
                );
            }
        }
    }
    assert_eq!(SESSION_VAR, "HTUI_SESSION");
}

/// [`AgentDriver::start`], retrying the `ETXTBSY` window a fixture executable opens.
///
/// The harness runs these cases on threads of one process, so a `fork` in another case's spawn
/// inherits the write fd this one just closed and `execve` answers "Text file busy" until that
/// child execs. The window is short, so a bounded retry is the whole fix; `tests/probe.rs` carries
/// the same helper for the same reason. Anything else fails on the first attempt's error.
#[cfg(unix)]
async fn start_retrying_etxtbsy(
    driver: &dyn AgentDriver,
    spec: &SessionSpec,
) -> htui_agent::error::Result<Box<dyn AgentSession>> {
    for _ in 0..20u32 {
        match driver.start(spec.clone(), "hi".to_owned()).await {
            Err(DriverError::Spawn(message)) if message.contains("Text file busy") => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            other => return other,
        }
    }
    driver.start(spec.clone(), "hi".to_owned()).await
}

/// The same claim as the case above, but end to end and through the registry: `AcpAdapter::build`
/// no longer ignores `on_box`, so the argv the kernel actually saw carries the marker.
///
/// The fixture prints its own arguments and exits, which is not an ACP agent — the error is
/// expected and is *not* what is being asserted. What is asserted is the file it left behind.
#[cfg(unix)]
#[tokio::test]
async fn through_the_factory_the_spawned_argv_carries_the_marker() {
    let tmp = tempfile::tempdir().expect("temp box");
    let argv_file = tmp.path().join("argv");
    let command = tmp.path().join("bin/htui-fake-adapter");
    executable(
        &command,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HTUI_ARGV_FILE\"\nexit 0\n",
    );

    let agent = unresolvable_row();
    let recorded = ResolvedLaunch {
        command: command.to_string_lossy().into_owned(),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::from([(
            "HTUI_ARGV_FILE".to_owned(),
            argv_file.to_string_lossy().into_owned(),
        )]),
    };
    let row_on_box = on_box(
        &agent,
        &snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded),
    );

    let driver = DriverFactory::production()
        .driver_for(&agent, Some(&row_on_box))
        .expect("the `acp` adapter builds this row");
    let spec = spec(tmp.path().to_path_buf());
    match start_retrying_etxtbsy(driver.as_ref(), &spec).await {
        Err(DriverError::Transport(_)) => {}
        Err(other) => panic!("the fixture was spawned and then failed to speak ACP, got {other:?}"),
        Ok(_) => panic!("a script that prints its argv and exits does not open a session"),
    }

    assert_eq!(
        std::fs::read_to_string(&argv_file)
            .expect("the fixture recorded its argv")
            .trim(),
        MARKER,
        "the factory threaded `on_box` through to the process the kernel started"
    );
}

/// D58's fourth rule, which is the reason the check is I/O at all: ANA-4 §4.6 records that `agy`
/// self-updates in place, so a recorded path can name a version directory that is gone. A chat then
/// degrades into resolution rather than failing the request.
#[tokio::test]
async fn a_recorded_command_that_is_gone_falls_back_to_resolution() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let recorded = ResolvedLaunch {
        // Never created: a version-numbered directory that a self-update replaced.
        command: tmp
            .path()
            .join("gone/agy_acp_server.par")
            .to_string_lossy()
            .into_owned(),
        args: vec!["--uid=".to_owned()],
        env: BTreeMap::new(),
    };
    let row_on_box = on_box(
        &agent,
        &snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded),
    );

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the row resolves for itself when the recording is stale");

    assert_eq!(
        launch.command,
        entry_point(tmp.path()).to_string_lossy(),
        "resolution answered, not the recording"
    );
    assert!(
        launch.args.is_empty(),
        "the fallback is the pre-milestone-6 path and carries no platform args: {:?}",
        launch.args
    );
}

/// The same rule, for the shape "does it exist" cannot tell apart: a **directory** at the recorded
/// path.
///
/// The self-update ANA-4 §4.6 describes replaces version-numbered directories, so the path a
/// recording names comes back as a file, as nothing, or — while the two layouts disagree — as a
/// directory. Nothing is unusable: the recording is not a launch, and answering it would fail at
/// `execve` with a `Spawn` error the user reads as a broken chat and D60 answers with a whole
/// re-probe. Asking for a *file* instead of an entry costs the same `metadata` call and lands in
/// the fallback the rule already has.
#[tokio::test]
async fn a_recorded_command_that_is_now_a_directory_falls_back_to_resolution() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let directory = tmp.path().join("bin/agy_acp_server.par");
    std::fs::create_dir_all(&directory).expect("mkdir");
    let recorded = ResolvedLaunch {
        command: directory.to_string_lossy().into_owned(),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::new(),
    };
    let row_on_box = on_box(
        &agent,
        &snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded),
    );

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the row resolves for itself when the recording is not a file");

    assert_eq!(
        launch.command,
        entry_point(tmp.path()).to_string_lossy(),
        "a directory is not the adapter the probe recorded"
    );
    assert!(
        !launch.args.contains(&MARKER.to_owned()),
        "resolution answered, not the recording: {:?}",
        launch.args
    );
}

/// MOD-66 D6: a `manual` snapshot is the probe's own recording over a human's path, so it is
/// spawned exactly as a `probe` one is. Before MOD-66 this case pinned the refusal (MOD-2
/// blueprint H-5, now superseded).
///
/// The row resolves for itself, to a different file, so the case cannot pass for the wrong
/// reason: a launch carrying the marker came from the recording.
#[tokio::test]
async fn a_manual_snapshot_is_used_as_a_launch() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let recorded = ResolvedLaunch {
        command: plain_file(&tmp.path().join("bin/hand-picked")),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::new(),
    };
    let row_on_box = on_box(
        &agent,
        &snapshot(ProbeSource::Manual, ProbeStatus::Ready, recorded.clone()),
    );

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("a manual snapshot is usable");

    assert_eq!(
        launch.command, recorded.command,
        "MOD-66 D6: the recording made over the human's path is the launch"
    );
    assert!(
        launch.args.contains(&MARKER.to_owned()),
        "the manual snapshot was spawned as recorded: {:?}",
        launch.args
    );
}

/// The plan's acceptance criterion "a per-box manual path reaches a spawn", end to end at the
/// seam: the probe walks the manual tier, tier 2 answers, the row is projected, and the driver
/// spawns what the probe recorded. The row's one tool is on no `PATH`, so nothing but the manual
/// path can have produced the command.
#[tokio::test]
async fn a_manual_path_reaches_the_launch_through_the_probe() {
    /// A tier 2 that completes `initialize` with no auth methods, without a process.
    struct Answers;

    impl Tier2 for Answers {
        fn handshake<'a>(
            &'a self,
            _launch: &'a ResolvedLaunch,
            _settings: &'a AcpSettings,
            _env: &'a ProbeEnv,
        ) -> DriverFuture<'a, Handshake> {
            Box::pin(async move {
                Ok(Handshake {
                    at: Utc::now(),
                    protocol_version: 1,
                    agent_name: Some("manual-fixture".to_owned()),
                    agent_version: Some("0.0.0".to_owned()),
                    capabilities: json!({}),
                    auth_methods: vec![],
                })
            })
        }
    }

    let tmp = tempfile::tempdir().expect("temp box");
    let agent = unresolvable_row();
    let tool = plain_file(&tmp.path().join("opt/htui-fake-adapter"));
    let ctx = ProbeContext {
        env: ProbeEnv {
            cwd: tmp.path().to_path_buf(),
            platform: "linux-x86_64".to_owned(),
            home: None,
            vars: BTreeMap::from([(
                "PATH".to_owned(),
                tmp.path().join("bin").to_string_lossy().into_owned(),
            )]),
            versions: false,
            version_timeout: Duration::from_secs(5),
        },
        now: Utc::now(),
    };
    let manual = BTreeMap::from([("tool".to_owned(), tool.clone())]);

    let snapshot = probe_snapshot(&agent, &manual, &ctx, &Answers).await;
    assert_eq!(snapshot.status, ProbeStatus::Ready, "{snapshot:?}");
    assert_eq!(snapshot.source, ProbeSource::Manual);
    let row_on_box = agent_box_row(&agent, BoxId::new(), &snapshot, ctx.now);

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the manual recording is usable");
    assert_eq!(
        launch.command, tool,
        "the human's path reached the spawn through the probe's recording"
    );
}

/// A recording carries the transport it was made for, and this driver is the `acp` one.
///
/// `agent.transport` is a hand-editable column too, and flipping it is not a small edit: a `cli`
/// row's recorded launch is an argv for a *command-line* agent, and spawning it as an ACP adapter
/// would hand its stdin a JSON-RPC handshake it has no idea what to do with. The snapshot says
/// which transport it was probed for, so the two disagreeing is the one case where the recording
/// is not merely stale but wrong — and the row's own `launch` is what the edit was asking for.
///
/// The recorded command exists on disk here, so the disk check cannot be what refuses.
#[tokio::test]
async fn a_snapshot_probed_for_another_transport_is_not_used_as_a_launch() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let recorded = ResolvedLaunch {
        command: plain_file(&tmp.path().join("bin/htui-fake-adapter")),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::new(),
    };
    let stale_transport = ProbeSnapshot {
        transport: Transport::Cli,
        ..snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded)
    };
    let row_on_box = on_box(&agent, &stale_transport);

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the row resolves for itself");

    assert_eq!(
        launch.command,
        entry_point(tmp.path()).to_string_lossy(),
        "the edited row is honoured through `agent.launch`, which is where the edit landed"
    );
    assert!(
        !launch.args.contains(&MARKER.to_owned()),
        "a recording made for another transport was not spawned: {:?}",
        launch.args
    );
}

/// The column is hand-editable `JSONB`, so a document that does not parse must cost a chat nothing
/// more than the milestone-5 path — the same tolerance `ProbeSnapshot::from_row` already grants the
/// Settings tab.
#[tokio::test]
async fn a_probe_column_that_does_not_parse_resolves_as_before() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let recorded = ResolvedLaunch {
        command: plain_file(&tmp.path().join("bin/htui-fake-adapter")),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::new(),
    };
    let row_on_box = AgentBox {
        // Half a snapshot: `status` alone, with none of the keys the struct requires.
        probe: Some(json!({ "status": "ready" })),
        ..on_box(
            &agent,
            &snapshot(ProbeSource::Probe, ProbeStatus::Ready, recorded),
        )
    };

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("an unreadable snapshot is not a reason to refuse the row");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the row resolves for itself");

    assert_eq!(launch.command, entry_point(tmp.path()).to_string_lossy());
    assert!(
        !launch.args.contains(&MARKER.to_owned()),
        "nothing was read out of a document that does not parse: {:?}",
        launch.args
    );
}

// ---------------------------------------------------------------------------------------------
// MOD-37 M5 (R-48): the driver restores `spec.resume` - `session/resume`, else `session/load`
// with its replay discarded, else a refusal
// ---------------------------------------------------------------------------------------------

/// The session id [`restoring_agent`] mints on `session/new`. A banner carrying it means the
/// driver opened a fresh session instead of restoring the requested one.
#[cfg(unix)]
const FRESH_SESSION: &str = "fresh-session";

/// The agent-side id every restore case asks for.
#[cfg(unix)]
const PRIOR_SESSION: &str = "prior";

/// How [`restoring_agent`] answers one restore request.
#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
enum Answer {
    /// `result: {}`.
    Ok,
    /// `result: {}`, after the replay is streamed as `session/update` notifications: what an
    /// agent that sends updates ahead of its `session/resume` answer looks like (review N-1).
    /// `session/load` always streams its replay first.
    OkAfterUpdates,
    /// `error: { code: -32000, message: VENDOR_REFUSAL }`.
    Refuse,
}

/// Writes one JSON-RPC line and flushes it; `false` once the client has gone away.
#[cfg(unix)]
async fn write_line(
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    value: &Value,
) -> bool {
    let mut text = serde_json::to_string(value).expect("the line serialises");
    text.push('\n');
    writer.write_all(text.as_bytes()).await.is_ok() && writer.flush().await.is_ok()
}

/// The replay [`restoring_agent`] streams ahead of its `session/load` answer: one assistant chunk
/// and one completed tool call, the two kinds a duplicated history would show first.
#[cfg(unix)]
fn old_history() -> Vec<Value> {
    vec![
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "old reply" },
        }),
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "old-call",
            "title": "read old",
            "kind": "read",
            "status": "completed",
        }),
    ]
}

/// MOD-37 M5: an agent that advertises `caps` in its `initialize` answer, answers `session/new`
/// with [`FRESH_SESSION`], answers `session/resume` per `resume` and `session/load` per `load`
/// (writing `replay` as `session/update` notifications for the requested id first), and on
/// `session/prompt` sends one `agent_message_chunk` "fresh turn" and ends the turn `end_turn`.
/// Every request's method and params go to `seen`.
///
/// MOD-96 D4: `modes`, when `Some`, rides on the `session/new` answer and on every successful
/// restore answer as the session's mode state, and `session/set_mode` is answered per `set_mode`:
/// `result: {}`, or [`Answer::Refuse`]'s error (review R1 #3).
///
/// Raw newline-delimited JSON-RPC in [`refuse_session_new`]'s style. Notifications from the client
/// (`session/cancel`) are recorded and get no answer.
#[cfg(unix)]
async fn restoring_agent(
    stream: tokio::io::DuplexStream,
    caps: Value,
    resume: Answer,
    load: Answer,
    replay: Vec<Value>,
    seen: tokio::sync::mpsc::UnboundedSender<(String, Value)>,
    modes: Option<Value>,
    set_mode: Answer,
) {
    // `{}` plus the mode state when the case gives one: every successful opening answer's result.
    let opened = |mut result: Value| {
        if let Some(modes) = &modes {
            result["modes"] = modes.clone();
        }
        result
    };
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let _ = seen.send((method.to_owned(), params.clone()));
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let answered = |answer: Answer| match answer {
            Answer::Ok | Answer::OkAfterUpdates => {
                json!({ "jsonrpc": "2.0", "id": id, "result": opened(json!({})) })
            }
            Answer::Refuse => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": VENDOR_REFUSAL },
            }),
        };
        let reply = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocolVersion": 1, "agentCapabilities": caps },
            }),
            "session/new" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": opened(json!({ "sessionId": FRESH_SESSION })),
            }),
            "session/set_mode" if matches!(set_mode, Answer::Refuse) => answered(set_mode),
            "session/set_mode" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "session/resume" | "session/load" => {
                let resuming = method == "session/resume";
                if resuming && !matches!(resume, Answer::OkAfterUpdates) {
                    if !write_line(&mut writer, &answered(resume)).await {
                        return;
                    }
                    continue;
                }
                let session_id = params.get("sessionId").cloned().unwrap_or(Value::Null);
                for update in &replay {
                    let notification = json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": { "sessionId": session_id, "update": update },
                    });
                    if !write_line(&mut writer, &notification).await {
                        return;
                    }
                }
                answered(if resuming { resume } else { load })
            }
            "session/prompt" => {
                let session_id = params.get("sessionId").cloned().unwrap_or(Value::Null);
                let chunk = json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "fresh turn" },
                        },
                    },
                });
                if !write_line(&mut writer, &chunk).await {
                    return;
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } })
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" },
            }),
        };
        if !write_line(&mut writer, &reply).await {
            return;
        }
    }
}

/// `spec(cwd)` asking to restore `id`, with one extra directory so the request's
/// `additionalDirectories` is something to assert.
#[cfg(unix)]
fn spec_resuming(cwd: PathBuf, id: &str) -> SessionSpec {
    let extra = cwd.join("extra");
    SessionSpec {
        resume: Some(AgentSessionRef::new(id)),
        extra_dirs: vec![extra],
        ..spec(cwd)
    }
}

/// One restore attempt's outcome: what `open_session` answered, the pid of its `sleep` child
/// when it had one, the requests the agent saw, and the agent's task.
#[cfg(unix)]
struct Restore {
    opened: Result<htui_agent::acp::AcpSession, DriverError>,
    pid: Option<u32>,
    seen: tokio::sync::mpsc::UnboundedReceiver<(String, Value)>,
    agent: tokio::task::JoinHandle<()>,
}

#[cfg(unix)]
impl Restore {
    /// Every `(method, params)` the agent has seen so far. The agent records a request before it
    /// answers, and `open_session` returns only after its last answer, so nothing is still in
    /// flight when this is called after `open_session`.
    fn seen(&mut self) -> Vec<(String, Value)> {
        let mut seen = Vec::new();
        while let Ok(entry) = self.seen.try_recv() {
            seen.push(entry);
        }
        seen
    }
}

/// The methods in `seen`, in order.
#[cfg(unix)]
fn methods(seen: &[(String, Value)]) -> Vec<&str> {
    seen.iter().map(|(method, _)| method.as_str()).collect()
}

/// Opens a session resuming [`PRIOR_SESSION`] in `cwd` against [`restoring_agent`], with
/// `session` as `settings.acp.session` and, when `with_child`, a `sleep 1000` child whose pid the
/// case can assert on. Every other setting is the default, so the session is isolated (MOD-91 D3).
#[cfg(unix)]
async fn open_restoring(
    cwd: &Path,
    caps: Value,
    resume: Answer,
    load: Answer,
    session: SessionSettings,
    with_child: bool,
) -> Restore {
    let settings = AgentSettings {
        acp: AcpSettings {
            session,
            ..AcpSettings::default()
        },
        ..AgentSettings::default()
    };
    open_restoring_with(cwd, caps, resume, load, settings, with_child).await
}

/// [`open_restoring`] under the whole row settings `settings`: what MOD-91's `_meta` cases vary.
#[cfg(unix)]
async fn open_restoring_with(
    cwd: &Path,
    caps: Value,
    resume: Answer,
    load: Answer,
    settings: AgentSettings,
    with_child: bool,
) -> Restore {
    open_against(
        cwd,
        spec_resuming(cwd.to_path_buf(), PRIOR_SESSION),
        caps,
        resume,
        load,
        None,
        Answer::Ok,
        settings,
        with_child,
    )
    .await
}

/// Opens `spec` against [`restoring_agent`] reporting `modes` and answering `session/set_mode` per
/// `set_mode` (MOD-96 D4), under the row settings `settings` and, when `with_child`, with a `sleep 1000` child whose pid the case can assert on.
/// A `spec` with no `resume` opens with `session/new`.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fakes run under no TUI panic hook"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "each is one axis a case varies; a struct would rename the arity without reducing it"
)]
async fn open_against(
    cwd: &Path,
    spec: SessionSpec,
    caps: Value,
    resume: Answer,
    load: Answer,
    modes: Option<Value>,
    set_mode: Answer,
    settings: AgentSettings,
    with_child: bool,
) -> Restore {
    let child = if with_child {
        Some(
            spawn(
                &ResolvedLaunch {
                    command: "sleep".to_owned(),
                    args: vec!["1000".to_owned()],
                    env: BTreeMap::new(),
                },
                cwd,
            )
            .await
            .expect("`sleep` is on this box"),
        )
    } else {
        None
    };
    let pid = child
        .as_ref()
        .map(|child| child.pid().expect("a freshly spawned child has a pid"));

    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    let (seen_tx, seen) = tokio::sync::mpsc::unbounded_channel();
    let agent = tokio::spawn(restoring_agent(
        agent_end,
        caps,
        resume,
        load,
        old_history(),
        seen_tx,
        modes,
        set_mode,
    ));
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child,
    };
    let mut options = options(PATIENT_HANDSHAKE);
    options.settings = settings;

    let opened = open_session(io, spec, "hi".to_owned(), options).await;
    Restore {
        opened,
        pid,
        seen,
        agent,
    }
}

/// Pulls events until the turn's `done`, inclusive.
#[cfg(unix)]
async fn events_to_done(session: &mut htui_agent::acp::AcpSession) -> Vec<DriverEvent> {
    let mut events = Vec::new();
    loop {
        let next = tokio::time::timeout(EVENT_WINDOW, session.next_event())
            .await
            .expect("the turn ends within the window")
            .expect("the transport stays healthy");
        let Some(envelope) = next else {
            break;
        };
        let done = matches!(envelope.event, DriverEvent::Done(_));
        events.push(envelope.event);
        if done {
            break;
        }
    }
    events
}

/// The banner's `session_id`, failing unless `event` is the banner.
#[cfg(unix)]
fn banner_session(event: &DriverEvent) -> &str {
    match event {
        DriverEvent::Other(other) if other.update == SESSION_STARTED => other
            .body
            .get("session_id")
            .and_then(Value::as_str)
            .expect("the banner names its session"),
        other => panic!("the first event is the banner, got {other:?}"),
    }
}

/// The transport error's text, failing on anything else.
#[cfg(unix)]
fn transport_message(opened: &Result<htui_agent::acp::AcpSession, DriverError>) -> &str {
    match opened {
        Err(DriverError::Transport(message)) => message,
        Err(other) => panic!("expected a transport error, got {other:?}"),
        Ok(_) => panic!("this restore must not open a session"),
    }
}

/// Fails unless `params` names the requested session, `cwd` and `cwd/extra`.
#[cfg(unix)]
fn assert_restore_params(params: &Value, cwd: &Path) {
    assert_eq!(params["sessionId"], json!(PRIOR_SESSION), "{params}");
    assert_eq!(params["cwd"], json!(cwd), "{params}");
    assert_eq!(
        params["additionalDirectories"],
        json!([cwd.join("extra")]),
        "{params}"
    );
}

/// The one event after the banner that opens the turn must be the new prompt's own chunk.
#[cfg(unix)]
fn assert_fresh_turn(after_banner: &[DriverEvent]) {
    match after_banner {
        [
            DriverEvent::AssistantChunk(chunk),
            DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            }),
        ] => assert_eq!(chunk.text, "fresh turn"),
        other => panic!("the fresh turn's chunk, then `done(end_turn)`, got {other:?}"),
    }
}

/// (a) `session/resume` advertised and allowed: it is sent with the id, the cwd and the extra
/// dirs, `session/new` never is, and the banner carries the restored id.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_sends_session_resume_and_never_session_new() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({ "sessionCapabilities": { "resume": {} }, "loadSession": true }),
        Answer::Ok,
        Answer::Ok,
        SessionSettings::default(),
        false,
    )
    .await;
    let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
    let mut session = opened.expect("a resumable session opens");
    let events = events_to_done(&mut session).await;
    let seen = restore.seen();

    let resumed: Vec<_> = seen
        .iter()
        .filter(|(method, _)| method == "session/resume")
        .collect();
    assert_eq!(
        resumed.len(),
        1,
        "one `session/resume`: {:?}",
        methods(&seen)
    );
    assert_restore_params(&resumed[0].1, tmp.path());
    assert!(
        !methods(&seen).contains(&"session/new") && !methods(&seen).contains(&"session/load"),
        "neither `session/new` nor `session/load` was sent: {:?}",
        methods(&seen)
    );
    assert_eq!(banner_session(&events[0]), PRIOR_SESSION);
    assert_fresh_turn(&events[1..]);

    drop(session);
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// (b) `loadSession` only: `session/load` is sent, and the history it replays ahead of its answer
/// never reaches `next_event`, so the first event after the banner is the new prompt's turn. A
/// replay streamed after the answer would show up here as an extra event (H-5).
#[cfg(unix)]
#[tokio::test]
async fn a_load_discards_the_replay_before_the_first_turn() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({ "loadSession": true }),
        Answer::Ok,
        Answer::Ok,
        SessionSettings::default(),
        false,
    )
    .await;
    let seen = restore.seen();
    let loaded: Vec<_> = seen
        .iter()
        .filter(|(method, _)| method == "session/load")
        .collect();
    assert_eq!(loaded.len(), 1, "one `session/load`: {:?}", methods(&seen));
    assert_restore_params(&loaded[0].1, tmp.path());
    assert!(
        !methods(&seen).contains(&"session/new"),
        "`session/new` was not sent: {:?}",
        methods(&seen)
    );

    let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
    let mut session = opened.expect("a loadable session opens");
    let events = events_to_done(&mut session).await;
    assert_eq!(banner_session(&events[0]), PRIOR_SESSION);
    assert!(
        !events.iter().any(|event| match event {
            DriverEvent::AssistantChunk(chunk) => chunk.text == "old reply",
            DriverEvent::ToolCall(call) => call.tool_call_id == "old-call",
            _ => false,
        }),
        "nothing from the replay reached the session: {events:?}"
    );
    assert_fresh_turn(&events[1..]);

    drop(session);
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// (c1) Neither request is advertised: `start` refuses, naming both missing capabilities, sends
/// no `session/new`, and reaps the child.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_the_agent_cannot_restore_is_refused_and_reaped() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({}),
        Answer::Ok,
        Answer::Ok,
        SessionSettings::default(),
        true,
    )
    .await;
    let message = transport_message(&restore.opened).to_owned();
    assert!(
        message.contains("cannot resume session `prior`"),
        "the refusal names the session: {message}"
    );
    assert!(
        message.contains("sessionCapabilities.resume") && message.contains("loadSession"),
        "and both capabilities the agent lacks: {message}"
    );
    let seen = restore.seen();
    assert!(
        !methods(&seen).contains(&"session/new"),
        "no fresh session stands in for the requested one: {:?}",
        methods(&seen)
    );
    assert_reaped(
        restore.pid.expect("the case spawned a child"),
        "the session's child",
    )
    .await;
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// (c2) Both requests advertised, both switched off in `settings.acp.session`: refused, naming
/// both settings, and no request of any kind is sent after `initialize`.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_the_settings_forbid_is_refused() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({ "sessionCapabilities": { "resume": {} }, "loadSession": true }),
        Answer::Ok,
        Answer::Ok,
        SessionSettings {
            load: false,
            resume: false,
        },
        false,
    )
    .await;
    let message = transport_message(&restore.opened).to_owned();
    assert!(
        message.contains("settings.acp.session.resume")
            && message.contains("settings.acp.session.load"),
        "the refusal names both settings: {message}"
    );
    let seen = restore.seen();
    assert_eq!(
        methods(&seen),
        vec!["initialize"],
        "no restore request and no `session/new` were sent"
    );
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// (d) A refused `session/load` answers with the agent's own message in the D61 shape, and the
/// child is reaped.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_session_load_answers_with_the_agents_own_message() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let restore = open_restoring(
        tmp.path(),
        json!({ "loadSession": true }),
        Answer::Ok,
        Answer::Refuse,
        SessionSettings::default(),
        true,
    )
    .await;
    let message = transport_message(&restore.opened).to_owned();
    assert!(
        message.contains("session/load failed"),
        "the error names the request that was refused: {message}"
    );
    assert!(
        message.contains(VENDOR_REFUSAL),
        "and carries the agent's own text: {message}"
    );
    assert_reaped(
        restore.pid.expect("the case spawned a child"),
        "the session's child",
    )
    .await;
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// (d') Review L-5: a refused `session/resume` answers with the agent's own message in the D61
/// shape, as (d) does for `session/load`, and the child is reaped.
#[cfg(unix)]
#[tokio::test]
async fn a_refused_session_resume_answers_with_the_agents_own_message() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({ "sessionCapabilities": { "resume": {} } }),
        Answer::Refuse,
        Answer::Ok,
        SessionSettings::default(),
        true,
    )
    .await;
    let message = transport_message(&restore.opened).to_owned();
    assert!(
        message.contains("session/resume failed"),
        "the error names the request that was refused: {message}"
    );
    assert!(
        message.contains(VENDOR_REFUSAL),
        "and carries the agent's own text: {message}"
    );
    let seen = restore.seen();
    assert!(
        !methods(&seen).contains(&"session/new") && !methods(&seen).contains(&"session/load"),
        "no other request stands in for it: {:?}",
        methods(&seen)
    );
    assert_reaped(
        restore.pid.expect("the case spawned a child"),
        "the session's child",
    )
    .await;
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// Review N-1: an agent that sends updates ahead of its `session/resume` answer has them
/// discarded, as a load's replay is: the first event after the banner is the new prompt's turn.
#[cfg(unix)]
#[tokio::test]
async fn updates_sent_before_a_resume_answer_never_reach_the_session() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let mut restore = open_restoring(
        tmp.path(),
        json!({ "sessionCapabilities": { "resume": {} } }),
        Answer::OkAfterUpdates,
        Answer::Ok,
        SessionSettings::default(),
        false,
    )
    .await;
    let seen = restore.seen();
    assert!(
        methods(&seen).contains(&"session/resume"),
        "the resume route was taken: {:?}",
        methods(&seen)
    );
    let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
    let mut session = opened.expect("a resumable session opens");
    let events = events_to_done(&mut session).await;
    assert_eq!(banner_session(&events[0]), PRIOR_SESSION);
    assert!(
        !events.iter().any(|event| match event {
            DriverEvent::AssistantChunk(chunk) => chunk.text == "old reply",
            DriverEvent::ToolCall(call) => call.tool_call_id == "old-call",
            _ => false,
        }),
        "nothing sent before the answer reached the session: {events:?}"
    );
    assert_fresh_turn(&events[1..]);

    drop(session);
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// Fails unless `pid` is already reaped: no `/proc/{pid}` at all, checked once, with no window.
///
/// [`assert_reaped`]'s window is for the exits that reap on the task's timeline; a restore that
/// failed reaps before it answers (review L-4), so by the time `start` returns there is nothing
/// left to wait for.
#[cfg(unix)]
fn assert_reaped_already(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "{what}: pid {pid} was still there when `start` returned"
    );
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

/// Review L-4: a restore that fails kills and reaps its child before it answers, so a caller
/// that falls back to a second adapter never runs it beside the first. Each way a restore
/// fails: no route, a refused `session/resume`, a refused `session/load`.
#[cfg(unix)]
#[tokio::test]
async fn a_failed_restore_has_reaped_its_child_when_start_returns() {
    let resume = json!({ "sessionCapabilities": { "resume": {} } });
    let load = json!({ "loadSession": true });
    for (what, caps, on_resume, on_load) in [
        ("no route", json!({}), Answer::Ok, Answer::Ok),
        ("a refused resume", resume, Answer::Refuse, Answer::Ok),
        ("a refused load", load, Answer::Ok, Answer::Refuse),
    ] {
        let tmp = tempfile::tempdir().expect("temp cwd");
        let restore = open_restoring(
            tmp.path(),
            caps,
            on_resume,
            on_load,
            SessionSettings::default(),
            true,
        )
        .await;
        assert!(restore.opened.is_err(), "{what}: the restore fails");
        assert_reaped_already(restore.pid.expect("the case spawned a child"), what);
        restore.agent.abort();
        let _ = restore.agent.await;
    }
}

/// (e) H-11: an ACP row resumes when either restore route is on, and hands off only with both off.
#[test]
fn caps_for_an_acp_row_resumes_when_load_or_resume_is_on() {
    for (resume, load, expected) in [
        (false, true, true),
        (true, false, true),
        (false, false, false),
    ] {
        let mut agent = row(json!({}));
        agent.settings = json!({ "acp": { "session": { "resume": resume, "load": load } } });
        assert_eq!(
            caps_for(&agent).resume,
            expected,
            "settings.acp.session {{ resume: {resume}, load: {load} }}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// MOD-11 D7: `session/new` carries `htui`'s MCP server
// ---------------------------------------------------------------------------------------------

/// An agent that answers `initialize` and `session/new`, forwards the `session/new` params to
/// `seen`, and ignores everything else (the prompt included: the case ends at the open).
///
/// [`fs_agent`]'s raw newline-delimited JSON-RPC, trimmed to the handshake. It reports no modes and
/// would leave a `session/set_mode` unanswered, so it is only ever opened with no pinned
/// `settings.acp.permission_mode` (MOD-96 D4); [`restoring_agent`] is the one that answers a pin.
#[cfg(unix)]
async fn mcp_agent(stream: tokio::io::DuplexStream, seen: oneshot::Sender<Value>) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut seen = Some(seen);
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        let reply = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => {
                json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": 1 } })
            }
            Some("session/new") => {
                if let Some(seen) = seen.take() {
                    let _ = seen.send(message.get("params").cloned().unwrap_or(Value::Null));
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": { "sessionId": "mcp-session" } })
            }
            _ => continue,
        };
        let mut text = serde_json::to_string(&reply).expect("the reply serialises");
        text.push('\n');
        if writer.write_all(text.as_bytes()).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
}

/// Opens a session over [`mcp_agent`] with `mcp` on the spec and answers the `session/new`
/// params the agent received. The row's settings are the default, so the session is isolated
/// (MOD-91 D3).
#[cfg(unix)]
async fn session_new_params(mcp: Vec<McpServerSpec>) -> Value {
    session_new_params_with(mcp, AgentSettings::default()).await
}

/// [`session_new_params`] under the row settings `settings`: what MOD-91's `_meta` cases vary.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fakes run under no TUI panic hook"
)]
async fn session_new_params_with(mcp: Vec<McpServerSpec>, settings: AgentSettings) -> Value {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    let (seen, params) = oneshot::channel();
    let agent = tokio::spawn(mcp_agent(agent_end, seen));
    let io = AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: None,
    };
    let mut spec = spec(tmp.path().to_path_buf());
    spec.mcp = mcp;

    let mut options = options(PATIENT_HANDSHAKE);
    options.settings = settings;

    let session = open_session(io, spec, "hi".to_owned(), options)
        .await
        .expect("the agent opens a session");
    let params = tokio::time::timeout(EVENT_WINDOW, params)
        .await
        .expect("session/new arrived within the window")
        .expect("the agent forwarded the params");

    drop(session);
    agent.abort();
    let _ = agent.await;
    params
}

/// D7: `htui`'s server travels as an ACP stdio entry — `name`, `command`, `args`, `env` as
/// `{name, value}` pairs in key order — and with **no `type` key**, because stdio is the
/// untagged variant every agent must support.
#[cfg(unix)]
#[tokio::test]
async fn session_new_carries_the_mcp_server_as_stdio() {
    let server = McpServerSpec {
        name: "htui".to_owned(),
        command: "/abs/htui".to_owned(),
        args: vec!["mcp".to_owned()],
        env: BTreeMap::from([
            ("HTUI_MCP_TOKEN".to_owned(), "token-value".to_owned()),
            (
                "HTUI_MCP_ADDR".to_owned(),
                "/run/htui-mcp-1-abcd/s".to_owned(),
            ),
        ]),
    };

    let params = session_new_params(vec![server]).await;

    assert_eq!(
        params["mcpServers"],
        json!([{
            "name": "htui",
            "command": "/abs/htui",
            "args": ["mcp"],
            "env": [
                { "name": "HTUI_MCP_ADDR", "value": "/run/htui-mcp-1-abcd/s" },
                { "name": "HTUI_MCP_TOKEN", "value": "token-value" },
            ],
        }]),
        "{params}"
    );
    assert!(
        params["mcpServers"][0].get("type").is_none(),
        "stdio is untagged on the wire: {params}"
    );
}

/// D7: a spec with no server sends `mcpServers: []`, as before MOD-11.
#[cfg(unix)]
#[tokio::test]
async fn session_new_without_mcp_sends_an_empty_list() {
    let params = session_new_params(Vec::new()).await;
    assert_eq!(params["mcpServers"], json!([]), "{params}");
}

// ---------------------------------------------------------------------------------------------
// MOD-91 D5: an isolated session's `_meta.claudeCode.options`
// ---------------------------------------------------------------------------------------------

/// MOD-91 B1: the one literal pin of the ACP lever, as T2 probed it (plan, "T2 — ACP", case (a)),
/// plus MOD-96 D4's `settingSources: []`, as MOD-96 T1 probed it.
/// Every other case compares against [`isolation_meta`] itself, so a change of lever touches the
/// function and this case, and nothing else.
#[test]
fn isolation_meta_is_the_probed_options() {
    assert_eq!(
        Value::Object(isolation_meta()),
        json!({
            "claudeCode": {
                "options": {
                    "settings": { "disableAllHooks": true },
                    "extraArgs": { "disable-slash-commands": "" },
                    "settingSources": [],
                },
            },
        }),
    );
}

/// MOD-91 D3/D5: a row that never mentions `user_extensions` is isolated, and its `session/new`
/// carries the lever as the whole `_meta`.
#[cfg(unix)]
#[tokio::test]
async fn session_new_carries_the_isolation_meta_by_default() {
    let params = session_new_params(Vec::new()).await;
    assert_eq!(params["_meta"], Value::Object(isolation_meta()), "{params}");
}

/// MOD-91 D3: a row that opts back in sends no `claudeCode` key at all, so the adapter loads the
/// user's extensions exactly as it did before MOD-91. Robust to `_meta` being absent or `null`.
#[cfg(unix)]
#[tokio::test]
async fn session_new_sends_no_claude_code_meta_when_opted_in() {
    let settings = AgentSettings {
        user_extensions: true,
        ..AgentSettings::default()
    };
    let params = session_new_params_with(Vec::new(), settings).await;
    assert!(
        params
            .get("_meta")
            .and_then(|meta| meta.get("claudeCode"))
            .is_none(),
        "an opted-in session/new carries no `claudeCode`: {params}"
    );
}

/// MOD-91 D5: a restored session carries the same `_meta` as `session/new`, because the adapter's
/// `getOrCreateSession` forwards `params._meta` on both routes (`acp-agent.js:5699-5720`, T2). A
/// restore that dropped it would reopen an isolated step with the user's extensions loaded. The
/// grid is both routes (`resume` advertised, then `loadSession` only) by both settings.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_and_a_load_carry_the_same_meta() {
    let routes = [
        (
            "session/resume",
            json!({ "sessionCapabilities": { "resume": {} } }),
        ),
        ("session/load", json!({ "loadSession": true })),
    ];
    for (method, caps) in routes {
        for user_extensions in [false, true] {
            let tmp = tempfile::tempdir().expect("temp cwd");
            let settings = AgentSettings {
                user_extensions,
                ..AgentSettings::default()
            };
            let mut restore = open_restoring_with(
                tmp.path(),
                caps.clone(),
                Answer::Ok,
                Answer::Ok,
                settings,
                false,
            )
            .await;
            let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
            let session = opened.unwrap_or_else(|err| {
                panic!("{method} restores (user_extensions {user_extensions}): {err:?}")
            });
            let seen = restore.seen();

            let restored: Vec<_> = seen.iter().filter(|(sent, _)| sent == method).collect();
            assert_eq!(
                restored.len(),
                1,
                "one `{method}` (user_extensions {user_extensions}): {:?}",
                methods(&seen)
            );
            let params = &restored[0].1;
            assert_restore_params(params, tmp.path());
            if user_extensions {
                assert!(
                    params
                        .get("_meta")
                        .and_then(|meta| meta.get("claudeCode"))
                        .is_none(),
                    "an opted-in `{method}` carries no `claudeCode`: {params}"
                );
            } else {
                assert_eq!(
                    params["_meta"],
                    Value::Object(isolation_meta()),
                    "an isolated `{method}` carries the lever: {params}"
                );
            }

            drop(session);
            restore.agent.abort();
            let _ = restore.agent.await;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// MOD-96 D4: `settings.acp.permission_mode` is pinned with `session/set_mode` on every route
// ---------------------------------------------------------------------------------------------

/// A session's mode state as `claude-agent-acp` reports it, currently in `current`. It offers
/// `default`, `acceptEdits` and `plan`.
#[cfg(unix)]
fn agent_modes(current: &str) -> Value {
    json!({
        "currentModeId": current,
        "availableModes": [
            { "id": "default", "name": "Default" },
            { "id": "acceptEdits", "name": "Accept Edits" },
            { "id": "plan", "name": "Plan" },
        ],
    })
}

/// The default row settings, pinning the ACP session mode `mode`.
#[cfg(unix)]
fn pinned(mode: &str) -> AgentSettings {
    AgentSettings {
        acp: AcpSettings {
            permission_mode: Some(mode.to_owned()),
            ..AcpSettings::default()
        },
        ..AgentSettings::default()
    }
}

/// The three opening requests a case walks.
#[cfg(unix)]
const OPENING_ROUTES: [&str; 3] = ["session/new", "session/resume", "session/load"];

/// The spec and the agent capabilities that make the driver open with `route` in `cwd`: a fresh
/// spec for `session/new`, else [`PRIOR_SESSION`] restored with only `route` advertised.
#[cfg(unix)]
fn opening(route: &str, cwd: &Path) -> (SessionSpec, Value) {
    match route {
        "session/new" => (spec(cwd.to_path_buf()), json!({})),
        "session/resume" => (
            spec_resuming(cwd.to_path_buf(), PRIOR_SESSION),
            json!({ "sessionCapabilities": { "resume": {} } }),
        ),
        "session/load" => (
            spec_resuming(cwd.to_path_buf(), PRIOR_SESSION),
            json!({ "loadSession": true }),
        ),
        other => panic!("no opening route `{other}`"),
    }
}

/// The `session/set_mode` requests in `seen`.
#[cfg(unix)]
fn set_modes(seen: &[(String, Value)]) -> Vec<&Value> {
    seen.iter()
        .filter(|(method, _)| method == "session/set_mode")
        .map(|(_, params)| params)
        .collect()
}

/// Fails unless `seen` holds exactly one `session/set_mode` to `default` for `session_id`, sent
/// after `opened_with` and before the first `session/prompt`.
#[cfg(unix)]
fn assert_pinned_after(seen: &[(String, Value)], opened_with: &str, session_id: &str) {
    let set = set_modes(seen);
    assert_eq!(set.len(), 1, "one `session/set_mode`: {:?}", methods(seen));
    assert_eq!(
        *set[0],
        json!({ "sessionId": session_id, "modeId": "default" }),
        "the pin names the session and the mode"
    );
    let order = methods(seen);
    let at = |method: &str| {
        order
            .iter()
            .position(|sent| *sent == method)
            .unwrap_or_else(|| panic!("`{method}` was sent: {order:?}"))
    };
    assert!(
        at(opened_with) < at("session/set_mode") && at("session/set_mode") < at("session/prompt"),
        "`{opened_with}`, then `session/set_mode`, then `session/prompt`: {order:?}"
    );
}

/// D4: `session/new` answered in `acceptEdits` (the user's `defaultMode`, which `_meta` cannot
/// override) is followed by one `session/set_mode` to the pinned `default`, before the first
/// prompt, and the session opens as usual.
#[cfg(unix)]
#[tokio::test]
async fn a_pinned_mode_is_set_after_session_new() {
    let tmp = tempfile::tempdir().expect("temp cwd");
    let (spec, caps) = opening("session/new", tmp.path());
    let mut restore = open_against(
        tmp.path(),
        spec,
        caps,
        Answer::Ok,
        Answer::Ok,
        Some(agent_modes("acceptEdits")),
        Answer::Ok,
        pinned("default"),
        false,
    )
    .await;
    let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
    let mut session = opened.expect("a pinned session opens");
    let events = events_to_done(&mut session).await;
    let seen = restore.seen();

    assert_pinned_after(&seen, "session/new", FRESH_SESSION);
    assert_eq!(banner_session(&events[0]), FRESH_SESSION);
    assert_fresh_turn(&events[1..]);

    drop(session);
    restore.agent.abort();
    let _ = restore.agent.await;
}

/// D4: a restored session reports its mode on the restore answer (schema `ResumeSessionResponse`
/// / `LoadSessionResponse.modes`), and is pinned exactly as a new one is, by its restored id. The
/// grid of [`a_resume_and_a_load_carry_the_same_meta`]: `resume` advertised, then `loadSession`.
#[cfg(unix)]
#[tokio::test]
async fn a_pinned_mode_is_set_after_a_resume_and_a_load() {
    for route in ["session/resume", "session/load"] {
        let tmp = tempfile::tempdir().expect("temp cwd");
        let (spec, caps) = opening(route, tmp.path());
        let mut restore = open_against(
            tmp.path(),
            spec,
            caps,
            Answer::Ok,
            Answer::Ok,
            Some(agent_modes("acceptEdits")),
            Answer::Ok,
            pinned("default"),
            false,
        )
        .await;
        let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
        let mut session = opened.unwrap_or_else(|err| panic!("{route} restores: {err:?}"));
        let events = events_to_done(&mut session).await;
        let seen = restore.seen();

        assert_pinned_after(&seen, route, PRIOR_SESSION);
        assert_eq!(banner_session(&events[0]), PRIOR_SESSION, "{route}");
        assert_fresh_turn(&events[1..]);

        drop(session);
        restore.agent.abort();
        let _ = restore.agent.await;
    }
}

/// D4: a mode that is already current is not set again, on every opening route (review R1 #9:
/// `session/load` included).
#[cfg(unix)]
#[tokio::test]
async fn a_mode_already_current_sends_no_set_mode() {
    for route in OPENING_ROUTES {
        let tmp = tempfile::tempdir().expect("temp cwd");
        let (spec, caps) = opening(route, tmp.path());
        let mut restore = open_against(
            tmp.path(),
            spec,
            caps,
            Answer::Ok,
            Answer::Ok,
            Some(agent_modes("default")),
            Answer::Ok,
            pinned("default"),
            false,
        )
        .await;
        let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
        let mut session = opened.unwrap_or_else(|err| panic!("{route} opens: {err:?}"));
        let events = events_to_done(&mut session).await;
        let seen = restore.seen();

        assert!(
            set_modes(&seen).is_empty(),
            "{route}: a current mode is not set again: {:?}",
            methods(&seen)
        );
        assert_fresh_turn(&events[1..]);

        drop(session);
        restore.agent.abort();
        let _ = restore.agent.await;
    }
}

/// D4: a row that pins nothing (`agy`'s, and every pre-MOD-96 row's) sends no `session/set_mode`,
/// whatever mode the agent reports, on every opening route (review R1 #9): today's behaviour.
#[cfg(unix)]
#[tokio::test]
async fn no_pinned_mode_sends_no_set_mode() {
    for route in OPENING_ROUTES {
        let tmp = tempfile::tempdir().expect("temp cwd");
        let (spec, caps) = opening(route, tmp.path());
        let mut restore = open_against(
            tmp.path(),
            spec,
            caps,
            Answer::Ok,
            Answer::Ok,
            Some(agent_modes("acceptEdits")),
            Answer::Ok,
            AgentSettings::default(),
            false,
        )
        .await;
        let opened = std::mem::replace(&mut restore.opened, Err(DriverError::Closed));
        let mut session = opened.unwrap_or_else(|err| panic!("{route} opens: {err:?}"));
        let events = events_to_done(&mut session).await;
        let seen = restore.seen();

        assert!(
            set_modes(&seen).is_empty(),
            "{route}: nothing pinned, nothing set: {:?}",
            methods(&seen)
        );
        assert_fresh_turn(&events[1..]);

        drop(session);
        restore.agent.abort();
        let _ = restore.agent.await;
    }
}

/// D4: an agent that reports no modes, or none with the pinned id, is refused: fail closed, since
/// an unknown starting mode is an unknown approval path. The refusal names the mode and the
/// setting, sends neither `session/set_mode` nor a prompt, and has reaped the child when `start`
/// returns (the owed path a failed restore takes, review L-4), on every opening route.
#[cfg(unix)]
#[tokio::test]
async fn an_agent_without_the_pinned_mode_is_refused() {
    let without_default = json!({
        "currentModeId": "acceptEdits",
        "availableModes": [
            { "id": "acceptEdits", "name": "Accept Edits" },
            { "id": "plan", "name": "Plan" },
        ],
    });
    for (world, modes) in [
        ("no modes", None),
        ("no `default` mode", Some(without_default)),
    ] {
        for route in OPENING_ROUTES {
            let what = format!("{world}, {route}");
            let tmp = tempfile::tempdir().expect("temp cwd");
            let (spec, caps) = opening(route, tmp.path());
            let mut restore = open_against(
                tmp.path(),
                spec,
                caps,
                Answer::Ok,
                Answer::Ok,
                modes.clone(),
                Answer::Ok,
                pinned("default"),
                true,
            )
            .await;

            let message = transport_message(&restore.opened);
            assert!(
                message.contains("`default`") && message.contains("settings.acp.permission_mode"),
                "{what}: the refusal names the mode and the setting: {message}"
            );
            let seen = restore.seen();
            assert!(
                !methods(&seen).contains(&"session/set_mode")
                    && !methods(&seen).contains(&"session/prompt"),
                "{what}: neither a pin nor a prompt was sent: {:?}",
                methods(&seen)
            );
            assert_reaped_already(restore.pid.expect("the case spawned a child"), &what);

            restore.agent.abort();
            let _ = restore.agent.await;
        }
    }
}

/// D4, review R1 #3: an agent that offers the pinned mode but answers `session/set_mode` with a
/// JSON-RPC error fails the start, on every opening route. The error names `session/set_mode` and
/// carries the agent's own text; the pin was sent once and no prompt followed it; and the child
/// is reaped by the time `start` returns (the owed path, review L-4).
#[cfg(unix)]
#[tokio::test]
async fn a_refused_set_mode_fails_the_start() {
    for route in OPENING_ROUTES {
        let tmp = tempfile::tempdir().expect("temp cwd");
        let (spec, caps) = opening(route, tmp.path());
        let mut restore = open_against(
            tmp.path(),
            spec,
            caps,
            Answer::Ok,
            Answer::Ok,
            Some(agent_modes("acceptEdits")),
            Answer::Refuse,
            pinned("default"),
            true,
        )
        .await;

        let message = transport_message(&restore.opened).to_owned();
        assert!(
            message.contains("session/set_mode"),
            "{route}: the error names the refused pin: {message}"
        );
        assert!(
            message.contains(VENDOR_REFUSAL),
            "{route}: and carries the agent's own text: {message}"
        );
        let seen = restore.seen();
        assert_eq!(
            set_modes(&seen).len(),
            1,
            "{route}: the pin was sent once: {:?}",
            methods(&seen)
        );
        assert!(
            !methods(&seen).contains(&"session/prompt"),
            "{route}: no prompt follows a refused pin: {:?}",
            methods(&seen)
        );
        assert_reaped_already(restore.pid.expect("the case spawned a child"), route);

        restore.agent.abort();
        let _ = restore.agent.await;
    }
}
