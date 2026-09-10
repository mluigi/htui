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
#[cfg(unix)]
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::AcpDriver;
#[cfg(unix)]
use htui_agent::acp::{AcpIo, SessionOptions, Stamp, open_session};
#[cfg(unix)]
use htui_agent::driver::{AgentDriver, AgentSession};
use htui_agent::driver::{PermissionPolicy, SessionSpec, ToolExposure};
#[cfg(unix)]
use htui_agent::error::DriverError;
use htui_agent::launch::ResolvedLaunch;
#[cfg(unix)]
use htui_agent::launch::{AgentSettings, spawn};
use htui_agent::probe::{ProbeSnapshot, ProbeSource, ProbeStatus, agent_box_row};
#[cfg(unix)]
use htui_agent::registry::DriverFactory;
use htui_agent::registry::caps_for;
use htui_core::model::{Agent as AgentRow, AgentBox, AgentId, Billing, BoxId, StepId, Transport};
use serde_json::{Value, json};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
/// included, and `spec.env` is still applied last and still wins (`R-SEC-2`).
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
                ("ROW_VAR".to_owned(), "row".to_owned()),
                ("SECRET".to_owned(), "s".to_owned()),
                ("SHARED".to_owned(), "spec".to_owned()),
            ]),
            "the row's environment holds paths and the spec's holds secrets, and the spec wins \
             on a key both hold (`R-SEC-2`); status {status:?}"
        );
    }
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

    let driver = DriverFactory::with_acp()
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

/// Blueprint H-5: a `manual` row's hand-written `probe.resolved` is a human's note, not a launch.
///
/// The recorded command exists on disk here on purpose — the disk check passes, so `source` is the
/// only rule left that can refuse, and the case cannot pass for the wrong reason.
#[tokio::test]
async fn a_manual_snapshot_is_not_used_as_a_launch() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = resolvable_row(tmp.path());
    let recorded = ResolvedLaunch {
        command: plain_file(&tmp.path().join("bin/hand-written")),
        args: vec![MARKER.to_owned()],
        env: BTreeMap::new(),
    };
    let row_on_box = on_box(
        &agent,
        &snapshot(ProbeSource::Manual, ProbeStatus::Ready, recorded),
    );

    let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
        .expect("the row's launch parses");
    let launch = driver
        .launch_for(&spec(tmp.path().to_path_buf()))
        .await
        .expect("the row resolves for itself");

    assert_eq!(
        launch.command,
        entry_point(tmp.path()).to_string_lossy(),
        "a manual row is honoured through `agent.launch`, which is where the human wrote it"
    );
    assert!(
        !launch.args.contains(&MARKER.to_owned()),
        "the hand-written snapshot was not spawned: {:?}",
        launch.args
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
/// The recorded command exists on disk here, as in the `manual` case above, so the disk check
/// cannot be what refuses.
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
