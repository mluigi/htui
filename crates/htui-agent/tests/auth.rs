//! A login, both halves (plan MOD-21 D12, T2; D9, T3): `initialize`, the live method list, one
//! `authenticate` or `logout`, the child killed on every exit — and, from T3, the operation
//! `AcpDriver::authenticate` wraps around all of it, which is where the launch a flow spawns is
//! decided.
//!
//! Two fixtures, because the file asks two different questions. The **scripted duplex agent** is
//! raw newline-delimited JSON-RPC with no SDK type on the agent side — the reason
//! `tests/acp_conformance.rs` gives, that two ends sharing a library prove the library and not the
//! protocol — and it answers every question about *what went on the wire*: which methods were
//! offered, which one was sent, what a JSON-RPC error becomes. The **`sh` fixture** is a real
//! process with a real pid, and it answers the only question a duplex cannot: whether anything is
//! still running once the flow has returned. Every kill claim in this file is a `/proc` read
//! against a pid, never a `pgrep` pattern.
//!
//! `R-AGT-5`: every method id, variable name and sentence here is made up. Nothing in this file is
//! a real agent's vocabulary, and nothing in `src/` may become one.
//!
//! Helpers are local rather than shared, as `tests/probe.rs` and `tests/acp_driver.rs` keep theirs:
//! a case's own assertion living in its own file is what lets one of them change without the others
//! being re-read.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::{AcpIo, WireFlow, run_auth};
use htui_agent::auth::{
    AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome,
    BrowserPolicy,
};
use htui_agent::error::DriverError;
use htui_agent::launch::AcpSettings;
use htui_core::model::{Agent, AgentId, Billing, Transport};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// The scripted duplex agent
// ---------------------------------------------------------------------------------------------

/// The buffer each half of the in-process pipe gets, as `tests/probe.rs` sizes it.
const DUPLEX_BYTES: usize = 64 * 1024;

/// How long a case waits for something another task has to do before it calls it a failure.
///
/// Bounded rather than unbounded so a regression is a named failure in two seconds instead of a
/// hung suite; three orders of magnitude above what any of these round trips costs on this box.
const PATIENCE: Duration = Duration::from_secs(2);

/// What the scripted agent does with a call it recognises.
#[derive(Debug, Clone)]
enum Answer {
    /// `{"result":{}}` — the call succeeded.
    Ok,
    /// A JSON-RPC error object, in the agent's own words (plan D5).
    Error {
        /// `error.code`.
        code: i64,
        /// `error.message`, the sentence the user is owed.
        message: &'static str,
    },
    /// Read on without answering: the request stays outstanding until the client goes away.
    Never,
}

/// What the scripted agent answers, and where it records what it was asked.
#[derive(Debug, Clone)]
struct Script {
    /// The `initialize` result, verbatim.
    initialize: Value,
    /// The answer to `authenticate`.
    authenticate: Answer,
    /// The answer to `logout`.
    logout: Answer,
    /// Every request the agent saw, in order — what makes "never sent" an assertion.
    log: Arc<Mutex<Vec<Value>>>,
}

/// A fresh request log.
fn new_log() -> Arc<Mutex<Vec<Value>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// The methods the agent was asked for, in order.
fn calls(log: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
    log.lock()
        .expect("the request log")
        .iter()
        .filter_map(|request| request.get("method")?.as_str().map(ToOwned::to_owned))
        .collect()
}

/// The first logged request for `method`, once there is one.
fn call_for(log: &Arc<Mutex<Vec<Value>>>, method: &str) -> Option<Value> {
    log.lock()
        .expect("the request log")
        .iter()
        .find(|request| request.get("method").and_then(Value::as_str) == Some(method))
        .cloned()
}

/// [`scripted_initialize`](../probe.rs) generalised: raw JSON-RPC lines, no SDK type on the agent
/// side, one answer per method name and a log of everything it was asked.
///
/// An unknown method is answered `-32601` rather than ignored, because a client that hangs and a
/// client that is refused are different bugs and only one of them is this file's subject.
async fn scripted_agent(stream: DuplexStream, script: Script) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if request.get("method").is_none() {
            continue;
        }
        script
            .log
            .lock()
            .expect("the request log")
            .push(request.clone());
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let answer = match method {
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": script.initialize.clone(),
            })),
            "authenticate" => answered(&script.authenticate, &id),
            "logout" => answered(&script.logout, &id),
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" },
            })),
        };
        let Some(answer) = answer else {
            continue;
        };
        let mut text = serde_json::to_string(&answer).expect("the response serialises");
        text.push('\n');
        if writer.write_all(text.as_bytes()).await.is_err() {
            return;
        }
        let _ = writer.flush().await;
    }
}

/// One [`Answer`] as a JSON-RPC response, or `None` for [`Answer::Never`].
fn answered(answer: &Answer, id: &Value) -> Option<Value> {
    match answer {
        Answer::Ok => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        Answer::Error { code, message } => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        })),
        Answer::Never => None,
    }
}

/// An `initialize` result carrying `methods` and, when `logout`, the capability that advertises a
/// logout verb.
fn init_result(methods: Value, logout: bool) -> Value {
    let auth = if logout {
        json!({ "logout": {} })
    } else {
        json!({})
    };
    json!({
        "protocolVersion": 1,
        "agentInfo": { "name": "fixture-agent", "version": "0.0.0" },
        "agentCapabilities": { "loadSession": false, "auth": auth },
        "authMethods": methods,
    })
}

/// An [`AcpIo`] over an in-process duplex whose far end is `script`. No child, so no kill to make.
fn duplex_io(script: Script) -> AcpIo {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(scripted_agent(agent_end, script));
    AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: None,
    }
}

/// An [`AcpIo`] whose far end reads and never answers anything: the handshake-timeout case.
fn silent_io() -> AcpIo {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(async move {
        let (agent_reader, _agent_writer) = tokio::io::split(agent_end);
        let mut lines = BufReader::new(agent_reader).lines();
        while let Ok(Some(_)) = lines.next_line().await {}
    });
    AcpIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: None,
    }
}

/// A [`WireFlow`] and the three ends its caller keeps: the events, the choice, the token.
fn wire_flow(
    handshake_timeout: Duration,
) -> (
    WireFlow,
    mpsc::UnboundedReceiver<AuthEvent>,
    oneshot::Sender<AuthChoice>,
    CancellationToken,
) {
    let (events, events_rx) = mpsc::unbounded_channel();
    let (choice_tx, choice) = oneshot::channel();
    let cancel = CancellationToken::new();
    (
        WireFlow {
            events,
            choice,
            cancel: cancel.clone(),
            handshake_timeout,
        },
        events_rx,
        choice_tx,
        cancel,
    )
}

/// The row's defaults: nothing here reads a setting but `client_capabilities`, and D21 says that
/// one is the default.
fn settings() -> AcpSettings {
    AcpSettings::default()
}

/// The `Methods` event, or a failure that says which event arrived instead.
fn methods_of(event: Option<AuthEvent>) -> (Vec<AuthMethodInfo>, bool, Vec<AuthMethodInfo>) {
    match event {
        Some(AuthEvent::Methods {
            methods,
            logout,
            hidden,
        }) => (methods, logout, hidden),
        other => panic!("the first event of a flow is its method list, not {other:?}"),
    }
}

/// One advertised method, as the flow reports it.
fn info(id: &str, name: &str, description: Option<&str>) -> AuthMethodInfo {
    AuthMethodInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.map(ToOwned::to_owned),
    }
}

/// A registry row over `launch`, with everything the login path does not read left at a default.
///
/// The name is the fixture's own and means nothing to the code under test: `caps_for` reads
/// `transport` and `settings`, and the login path reads `launch` (`R-AGT-5`).
fn agent_row(launch: Value) -> Agent {
    let now = Utc::now();
    Agent {
        id: AgentId::new(),
        name: "fixture-agent".to_owned(),
        transport: Transport::Acp,
        launch,
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// An [`AuthFlow`] for `cwd` and the three ends its caller keeps, as [`wire_flow`] does for the
/// wire half.
///
/// `idle` is [`AUTH_IDLE_CAP`] because nothing reads it yet — the clock is T5's — and stating the
/// production value here is what makes the day it starts being read a visible change.
fn auth_flow(
    cwd: &Path,
) -> (
    AuthFlow,
    mpsc::UnboundedReceiver<AuthEvent>,
    oneshot::Sender<AuthChoice>,
    CancellationToken,
) {
    let (events, events_rx) = mpsc::unbounded_channel();
    let (choice_tx, choice) = oneshot::channel();
    let cancel = CancellationToken::new();
    (
        AuthFlow {
            cwd: cwd.to_path_buf(),
            events,
            choice,
            cancel: cancel.clone(),
            idle: AUTH_IDLE_CAP,
            browser: BrowserPolicy::Neutralised,
        },
        events_rx,
        choice_tx,
        cancel,
    )
}

/// Polls `ready` until it answers, or fails after [`PATIENCE`].
async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting: {what}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------------------------
// The method list
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn methods_carry_the_agents_own_name_and_description_in_order() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(
            json!([
                { "id": "m-one", "name": "One", "description": "the first way in" },
                { "id": "m-two", "name": "Two" }
            ]),
            false,
        ),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, mut events, choice_tx, _cancel) = wire_flow(PATIENCE);
    // The list is this case's subject; how the flow then ends is `the_choice_sender_dropped…`'s.
    drop(choice_tx);
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    assert_eq!(outcome, AuthOutcome::Declined);
    let (methods, logout, hidden) = methods_of(events.recv().await);
    assert_eq!(
        methods,
        vec![
            info("m-one", "One", Some("the first way in")),
            info("m-two", "Two", None)
        ],
        "the chooser shows the agent's own words, in the agent's own order"
    );
    assert!(!logout, "this agent advertised no logout capability");
    assert!(hidden.is_empty());
}

#[tokio::test]
async fn a_terminal_typed_method_is_hidden_and_named_but_never_sent() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(
            json!([
                { "id": "m-one", "name": "One" },
                { "type": "terminal", "id": "m-tty", "name": "Terminal", "description": "needs a tty" }
            ]),
            false,
        ),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, mut events, choice_tx, _cancel) = wire_flow(PATIENCE);
    // Chosen anyway: the spec forbids passing a terminal method to `authenticate` (D4, D21), so
    // the wire has to refuse it rather than trust the chooser to.
    choice_tx
        .send(AuthChoice::Method("m-tty".to_owned()))
        .expect("the flow holds the receiver");
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    let (methods, _logout, hidden) = methods_of(events.recv().await);
    assert_eq!(methods, vec![info("m-one", "One", None)]);
    assert_eq!(
        hidden,
        vec![info("m-tty", "Terminal", Some("needs a tty"))],
        "a terminal method is named so the chooser can say why it is missing"
    );
    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned()],
        "`htui` advertises no terminal capability, so a terminal method must never reach the wire"
    );
    match outcome {
        AuthOutcome::Refused { call, message } => {
            assert_eq!(call, AuthCall::Authenticate("m-tty".to_owned()));
            assert!(message.contains("m-tty"), "{message}");
            assert!(message.contains("terminal"), "{message}");
        }
        other => panic!("a terminal choice is refused without a round trip, not {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_method_type_is_offered_as_an_agent_method() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(
            json!([{ "type": "future-kind", "id": "m-new", "name": "New", "description": "a kind this build has never heard of" }]),
            false,
        ),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, mut events, choice_tx, _cancel) = wire_flow(PATIENCE);
    choice_tx
        .send(AuthChoice::Method("m-new".to_owned()))
        .expect("the flow holds the receiver");
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    let (methods, _logout, hidden) = methods_of(events.recv().await);
    // Documented, not desired: the schema's `Agent` arm is `#[serde(untagged)]`, so it swallows
    // every unrecognised `type` before this crate ever sees one. This case says out loud what the
    // wire does today, and goes red the day the SDK starts distinguishing a new kind.
    assert_eq!(
        methods,
        vec![info(
            "m-new",
            "New",
            Some("a kind this build has never heard of")
        )],
        "an unknown `type` reaches this crate as an agent method, so it is offered as one"
    );
    assert!(hidden.is_empty());
    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned(), "authenticate".to_owned()]
    );
    assert_eq!(
        outcome,
        AuthOutcome::Completed {
            call: AuthCall::Authenticate("m-new".to_owned())
        }
    );
}

#[tokio::test]
async fn logout_is_offered_exactly_when_the_agent_advertises_it() {
    for advertised in [false, true] {
        let log = new_log();
        let io = duplex_io(Script {
            initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), advertised),
            authenticate: Answer::Ok,
            logout: Answer::Ok,
            log: Arc::clone(&log),
        });

        let (flow, mut events, choice_tx, _cancel) = wire_flow(PATIENCE);
        drop(choice_tx);
        run_auth(io, &settings(), flow).await.expect("the flow ran");

        let (_methods, logout, _hidden) = methods_of(events.recv().await);
        assert_eq!(
            logout, advertised,
            "`agentCapabilities.auth.logout` is the one thing that says a logout verb exists"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The call
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn choosing_a_method_sends_authenticate_with_that_id_and_completes() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(
            json!([{ "id": "m-one", "name": "One" }, { "id": "m-two", "name": "Two" }]),
            false,
        ),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
    choice_tx
        .send(AuthChoice::Method("m-two".to_owned()))
        .expect("the flow holds the receiver");
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned(), "authenticate".to_owned()],
        "one spawn, one handshake, one call"
    );
    let request = call_for(&log, "authenticate").expect("the agent was asked to authenticate");
    assert_eq!(
        request.pointer("/params/methodId").and_then(Value::as_str),
        Some("m-two"),
        "the chosen id travels as `params.methodId`: {request}"
    );
    assert_eq!(
        outcome,
        AuthOutcome::Completed {
            call: AuthCall::Authenticate("m-two".to_owned())
        },
        "the call returned; whether the box is now usable is the probe's verdict, not this one"
    );
}

#[tokio::test]
async fn a_json_rpc_error_is_refused_in_the_agents_own_words() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), false),
        authenticate: Answer::Error {
            code: -32602,
            message: "The FIXTURE_KEY environment variable must be set in the environment this \
                      server is launched from.",
        },
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
    choice_tx
        .send(AuthChoice::Method("m-one".to_owned()))
        .expect("the flow holds the receiver");
    let outcome = run_auth(io, &settings(), flow)
        .await
        .expect("a refusal is an answer, not a transport failure (D5)");

    match outcome {
        AuthOutcome::Refused { call, message } => {
            assert_eq!(call, AuthCall::Authenticate("m-one".to_owned()));
            assert!(
                message.contains("FIXTURE_KEY environment variable"),
                "the refusal names what the user has to do next, verbatim: {message}"
            );
        }
        other => panic!("a JSON-RPC error to the call is `Refused`, not {other:?}"),
    }
}

#[tokio::test]
async fn choosing_logout_sends_logout() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), true),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
    choice_tx
        .send(AuthChoice::Logout)
        .expect("the flow holds the receiver");
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned(), "logout".to_owned()]
    );
    assert_eq!(
        outcome,
        AuthOutcome::Completed {
            call: AuthCall::Logout
        }
    );
}

// ---------------------------------------------------------------------------------------------
// The two exits `handshake()` does not have
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn cancel_before_the_choice_ends_the_connection_and_answers_cancelled() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), false),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, mut events, _choice_tx, cancel) = wire_flow(PATIENCE);
    let settings = settings();
    let running = run_auth(io, &settings, flow);
    tokio::pin!(running);

    let event = tokio::select! {
        outcome = &mut running => panic!("the flow ended before it offered anything: {outcome:?}"),
        event = events.recv() => event,
    };
    methods_of(event);
    cancel.cancel();
    let outcome = running
        .await
        .expect("a cancel is an outcome, not a failure");

    assert_eq!(outcome, AuthOutcome::Cancelled);
    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned()],
        "nobody chose, so nothing was called"
    );
}

#[tokio::test]
async fn cancel_during_authenticate_answers_cancelled() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), false),
        // The agent that takes the request and never answers: a human sitting in a browser.
        authenticate: Answer::Never,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, mut events, choice_tx, cancel) = wire_flow(PATIENCE);
    let settings = settings();
    let running = run_auth(io, &settings, flow);
    tokio::pin!(running);

    let event = tokio::select! {
        outcome = &mut running => panic!("the flow ended before it offered anything: {outcome:?}"),
        event = events.recv() => event,
    };
    methods_of(event);
    choice_tx
        .send(AuthChoice::Method("m-one".to_owned()))
        .expect("the flow holds the receiver");
    until("the agent to be asked to authenticate", || {
        calls(&log).contains(&"authenticate".to_owned())
    })
    .await;

    cancel.cancel();
    let outcome = running
        .await
        .expect("a cancel is an outcome, not a failure");
    assert_eq!(
        outcome,
        AuthOutcome::Cancelled,
        "the token has to reach the kill while the foreground future is parked on a request the \
         agent will never answer"
    );
}

#[tokio::test]
async fn the_choice_sender_dropped_is_declined() {
    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), false),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });

    let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
    drop(choice_tx);
    let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

    assert_eq!(
        outcome,
        AuthOutcome::Declined,
        "a caller that went away without choosing declined; it did not cancel and did not fail"
    );
    assert_eq!(calls(&log), vec!["initialize".to_owned()]);
}

#[tokio::test]
async fn initialize_is_still_bounded_by_the_handshake_timeout() {
    let (flow, _events, _choice_tx, _cancel) = wire_flow(Duration::from_millis(100));
    let error = run_auth(silent_io(), &settings(), flow)
        .await
        .expect_err("an agent that never answers `initialize` is a transport failure");

    match error {
        DriverError::Transport(message) => assert!(
            message.contains("handshake"),
            "the one message whose job is to say how long the agent had: {message}"
        ),
        other => panic!("a handshake timeout is a transport error, not {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The operation: `AcpDriver::authenticate`, off any session
// ---------------------------------------------------------------------------------------------

/// The driver's operation over a transport that was handed to it (blueprint P-1).
///
/// `IoSource::Prepared` is private to `acp/` and `launch_for` refuses it by name, so without
/// `auth_source` this case could not exist and `AcpDriver::authenticate` would be reachable only
/// through a process. The row's `command` is a path that could never spawn: reaching the scripted
/// agent at all is what says nothing was launched.
#[cfg(feature = "test-support")]
#[tokio::test]
async fn a_prepared_transport_runs_the_flow_without_a_process() {
    use htui_agent::acp::{AcpDriver, Stamp};
    use htui_agent::driver::AgentDriver;
    use htui_agent::registry::caps_for;

    let log = new_log();
    let io = duplex_io(Script {
        initialize: init_result(json!([{ "id": "m-one", "name": "One" }]), false),
        authenticate: Answer::Ok,
        logout: Answer::Ok,
        log: Arc::clone(&log),
    });
    let agent = agent_row(json!({
        "command": "/nonexistent/htui-fixture-adapter",
        "args": [],
        "env": {},
    }));
    let driver = AcpDriver::over(io, &agent, caps_for(&agent), Stamp::Wall);

    let tmp = tempfile::tempdir().expect("a throwaway directory");
    let (flow, mut events, choice_tx, _cancel) = auth_flow(tmp.path());
    choice_tx
        .send(AuthChoice::Method("m-one".to_owned()))
        .expect("the flow holds the receiver");
    let outcome = driver.authenticate(flow).await.expect("the flow ran");

    assert_eq!(
        outcome,
        AuthOutcome::Completed {
            call: AuthCall::Authenticate("m-one".to_owned())
        },
        "a prepared transport is driven by the same operation a spawned one is"
    );
    assert_eq!(
        calls(&log),
        vec!["initialize".to_owned(), "authenticate".to_owned()],
        "the flow reached the prepared pair rather than trying to resolve the row's command"
    );
    methods_of(events.recv().await);
}

// ---------------------------------------------------------------------------------------------
// A real process, and what is left of it
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
mod process {
    use htui_agent::acp::AcpDriver;
    use htui_agent::driver::AgentDriver;
    use htui_agent::launch::{ResolvedLaunch, Spawned};
    use htui_agent::probe::{ProbeSnapshot, ProbeSource, ProbeStatus, agent_box_row};
    use htui_agent::registry::caps_for;
    use htui_core::model::{AgentBox, BoxId};

    use super::*;

    /// How long a signalled process is given to stop being one, as `tests/acp_driver.rs` sizes it.
    ///
    /// A `Drop` can only *send* the kill, and what the signal then costs is the kernel's business.
    const KILL_WINDOW: Duration = Duration::from_secs(2);

    /// The scripted agent again, as a shell script: same protocol, a real pid.
    ///
    /// Behaviour by environment, so one script serves every process case. `FIXTURE_DIR` is where
    /// the pid, the argv and the call log go; `FIXTURE_INIT` is the `initialize` result on one
    /// line; `FIXTURE_KEY` unset makes `authenticate` refuse; `FIXTURE_HOLD` makes it never
    /// answer; `FIXTURE_DIE` makes the process write to stderr and exit instead of answering.
    ///
    /// It writes its own `$@` because at T3 stderr is not yet an event stream (that is T5's tap),
    /// so a file is the only channel a case has for "what argv did the kernel actually start".
    ///
    /// The id is echoed back **as it arrived**, quotes and all: this SDK sends a UUID *string* as
    /// its JSON-RPC id, and a fixture that re-quoted it — or that assumed a number — would answer
    /// with a line the client's decoder skips, which is a handshake timeout wearing a costume.
    const AGENT_SH: &str = r#"
echo $$ > "$FIXTURE_DIR/pid"
printf '%s\n' "$@" > "$FIXTURE_DIR/argv"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\|[0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      echo initialize >> "$FIXTURE_DIR/calls"
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$FIXTURE_INIT" ;;
    *'"method":"authenticate"'*)
      echo authenticate >> "$FIXTURE_DIR/calls"
      if [ -n "$FIXTURE_DIE" ]; then echo boom >&2; exit 3; fi
      if [ -n "$FIXTURE_HOLD" ]; then sleep 3600; fi
      if [ -z "$FIXTURE_KEY" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"the FIXTURE_KEY variable must be set where this server is launched from"}}\n' "$id"
      else
        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      fi ;;
  esac
done
"#;

    /// Writes `contents` at `path` and makes it executable (`tests/probe.rs`'s helper).
    ///
    /// The script is run as `/bin/sh <path>` rather than executed directly, for the `ETXTBSY`
    /// reason that helper's doc gives: a `fork` in another test's spawn inherits every fd open at
    /// that instant, and a child holding a write fd makes `execve` refuse until it execs or exits.
    /// `sh` only *reads* the file, so the window never opens.
    fn executable(path: &Path, contents: &str) {
        std::fs::write(path, contents).expect("write");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    /// The fixture's launch: `/bin/sh <script>`, with the answers in its environment.
    fn fixture_launch(dir: &Path, methods: Value, extra: &[(&str, &str)]) -> ResolvedLaunch {
        let script = dir.join("agent.sh");
        executable(&script, AGENT_SH);
        let mut env = std::collections::BTreeMap::new();
        env.insert("FIXTURE_DIR".to_owned(), dir.to_string_lossy().into_owned());
        env.insert(
            "FIXTURE_INIT".to_owned(),
            serde_json::to_string(&init_result(methods, false)).expect("one line of JSON"),
        );
        for (name, value) in extra {
            env.insert((*name).to_owned(), (*value).to_owned());
        }
        ResolvedLaunch {
            command: "/bin/sh".to_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            env,
        }
    }

    /// [`htui_agent::launch::spawn`], with the streams taken and the pid read **first**: `AcpIo`
    /// takes the `Spawned` by value, so a pid asked for afterwards has no child to ask.
    async fn spawned_io(launch: &ResolvedLaunch, cwd: &Path) -> (AcpIo, u32) {
        let spawned: Spawned = htui_agent::launch::spawn(launch, cwd)
            .await
            .expect("the fixture spawns");
        let pid = spawned.pid().expect("a running child has a pid");
        (AcpIo::from_spawned(spawned).expect("piped stdio"), pid)
    }

    /// The methods the fixture recorded, in order.
    fn fixture_calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The arguments the kernel actually started the fixture with, as it recorded them.
    ///
    /// Empty lines are dropped: a script started with no arguments still writes the one newline
    /// `printf '%s\n' "$@"` produces for an empty list.
    fn fixture_argv(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("argv"))
            .unwrap_or_default()
            .lines()
            .filter(|argument| !argument.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    /// An argument no resolution on this box can invent.
    ///
    /// `tools::resolve` answers one string per tool and `launch::resolve` substitutes it into the
    /// row's own `args`; neither can add an argument the document does not already hold. So the
    /// marker that comes back out of `argv` names which of the two launches was spawned, and
    /// nothing else could have put it there (`tests/acp_driver.rs`'s D58 rule, at the login seam).
    const RECORDED_ARG: &str = "--fixture-recorded=1";

    /// The same trick for the other side of the branch: the row's own document.
    const ROW_ARG: &str = "--fixture-row=1";

    /// [`fixture_launch`] with one more argument after the script path.
    fn fixture_launch_marked(
        dir: &Path,
        methods: Value,
        extra: &[(&str, &str)],
        marker: &str,
    ) -> ResolvedLaunch {
        let mut launch = fixture_launch(dir, methods, extra);
        launch.args.push(marker.to_owned());
        launch
    }

    /// A registry row whose `launch` document **is** this resolved launch, literally.
    ///
    /// No `discovery`, so `tools::resolve` has nothing to find and `launch::resolve` substitutes
    /// nothing: what the row says is what a resolution answers, which is what makes the fallback
    /// case's argv predictable.
    fn row_over(launch: &ResolvedLaunch) -> Agent {
        agent_row(json!({
            "command": launch.command,
            "args": launch.args,
            "env": launch.env,
        }))
    }

    /// This box's `agent_box` row recording `resolved`, through the projection the probe writes.
    ///
    /// `unauthenticated` because that is the status a box a login is *for* has, and because
    /// `ProbeSnapshot::recorded_launch` accepts it: D58's rule is that the status certifies the
    /// recording is the right binary, and "nobody has logged in" says nothing against it.
    fn recorded_on_box(agent: &Agent, resolved: ResolvedLaunch) -> AgentBox {
        let snapshot = ProbeSnapshot {
            transport: Transport::Acp,
            resolved: Some(resolved),
            tools: std::collections::BTreeMap::new(),
            handshake: None,
            credential: None,
            status: ProbeStatus::Unauthenticated,
            stderr_tail: None,
            source: ProbeSource::Probe,
        };
        agent_box_row(agent, BoxId::new(), None, &snapshot, Utc::now())
    }

    /// Fails unless `pid` is gone **or** reaped-pending within [`KILL_WINDOW`]: a killed child
    /// nobody has waited for is a zombie, which is dead by every measure this asserts.
    ///
    /// Linux-only because `/proc` is; the pid is the one identifier a kill can be checked against
    /// that nothing else can accidentally answer to. Copied from `tests/acp_driver.rs:128-190`
    /// rather than shared, as that file's own doc explains.
    async fn assert_not_running(pid: u32, what: &str) {
        #[cfg(target_os = "linux")]
        {
            let deadline = std::time::Instant::now() + KILL_WINDOW;
            loop {
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                    return;
                };
                // `pid (comm) state …`, and `comm` may hold spaces and parens: the state is the
                // first field after the **last** `)`.
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

    /// Fails unless `pid` has been **reaped** within [`KILL_WINDOW`]: no `/proc/{pid}` at all,
    /// which a zombie still has.
    ///
    /// The stronger half of [`assert_not_running`], and the two are not interchangeable: every
    /// exit of the flow that *returns* has an awaitable path to `ChildGuard::kill_and_reap`, and
    /// "reaped" is that path's whole claim.
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

    #[tokio::test]
    async fn an_agent_that_dies_after_initialize_is_a_transport_error_with_its_stderr() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let launch = fixture_launch(
            tmp.path(),
            json!([{ "id": "m-one", "name": "One" }]),
            &[("FIXTURE_DIE", "1")],
        );
        let (io, pid) = spawned_io(&launch, tmp.path()).await;

        let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let error = run_auth(io, &settings(), flow)
            .await
            .expect_err("an agent that dies mid-call answered nothing at all");

        match error {
            DriverError::Transport(message) => assert!(
                message.contains("boom"),
                "the child's own last words are what say why it died: {message}"
            ),
            other => panic!("a dead transport is a transport error, not {other:?}"),
        }
        assert_not_running(pid, "an agent that exited on its own").await;
    }

    #[tokio::test]
    async fn no_child_survives_a_cancelled_flow() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let launch = fixture_launch(
            tmp.path(),
            json!([{ "id": "m-one", "name": "One" }]),
            &[("FIXTURE_HOLD", "1"), ("FIXTURE_KEY", "set")],
        );
        let (io, pid) = spawned_io(&launch, tmp.path()).await;

        let (flow, mut events, choice_tx, cancel) = wire_flow(PATIENCE);
        let settings = settings();
        let running = run_auth(io, &settings, flow);
        tokio::pin!(running);
        let event = tokio::select! {
            outcome = &mut running => panic!("the flow ended before it offered anything: {outcome:?}"),
            event = events.recv() => event,
        };
        methods_of(event);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        until("the fixture to be asked to authenticate", || {
            fixture_calls(tmp.path()).contains(&"authenticate".to_owned())
        })
        .await;

        cancel.cancel();
        let outcome = running
            .await
            .expect("a cancel is an outcome, not a failure");

        assert_eq!(outcome, AuthOutcome::Cancelled);
        assert_reaped(pid, "the child of a cancelled flow").await;
    }

    #[tokio::test]
    async fn no_child_survives_a_refusal() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        // No `FIXTURE_KEY`: the fixture refuses `authenticate` the way a real adapter refuses a
        // login it has no credential for.
        let launch = fixture_launch(tmp.path(), json!([{ "id": "m-one", "name": "One" }]), &[]);
        let (io, pid) = spawned_io(&launch, tmp.path()).await;

        let (flow, _events, choice_tx, _cancel) = wire_flow(PATIENCE);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = run_auth(io, &settings(), flow).await.expect("the flow ran");

        assert!(
            matches!(outcome, AuthOutcome::Refused { .. }),
            "expected a refusal, got {outcome:?}"
        );
        assert_reaped(pid, "the child of a refused flow").await;
    }

    #[tokio::test]
    async fn no_child_survives_a_dropped_flow_future() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let launch = fixture_launch(
            tmp.path(),
            json!([{ "id": "m-one", "name": "One" }]),
            &[("FIXTURE_HOLD", "1"), ("FIXTURE_KEY", "set")],
        );
        let (io, pid) = spawned_io(&launch, tmp.path()).await;

        let (flow, mut events, choice_tx, _cancel) = wire_flow(PATIENCE);
        let settings = settings();
        // `Box::pin`, not `tokio::pin!`: that macro shadows the binding with a `Pin<&mut _>`, and
        // dropping a pointer to the future is not dropping the future — which is the one thing
        // this case is about.
        let mut running = Box::pin(run_auth(io, &settings, flow));
        let event = tokio::select! {
            outcome = &mut running => panic!("the flow ended before it offered anything: {outcome:?}"),
            event = events.recv() => event,
        };
        methods_of(event);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        until("the fixture to be asked to authenticate", || {
            fixture_calls(tmp.path()).contains(&"authenticate".to_owned())
        })
        .await;

        // The one exit that does not return: nobody is left to await the reap, so the guard's
        // `Drop` signals the group and tokio's orphan reaper does the rest. Signalled, not
        // reaped, is therefore the claim.
        drop(running);
        assert_not_running(pid, "the child of a dropped flow future").await;
    }

    // -----------------------------------------------------------------------------------------
    // The operation over a row: which launch a login spawns
    // -----------------------------------------------------------------------------------------

    /// D58 at the login seam: a usable recording is what a flow spawns, per-platform arguments
    /// and all.
    ///
    /// A login is the one operation that runs on a box the probe has just called
    /// `unauthenticated`, so the recording it is holding is exactly the one this path has to
    /// honour — resolving again there would launch the adapter without the arguments a glob tool
    /// declared, which a `ToolMap` value cannot carry (blueprint H-3).
    #[tokio::test]
    async fn the_acp_driver_resolves_the_recorded_launch_for_the_flow() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let methods = json!([{ "id": "m-one", "name": "One" }]);
        let key = [("FIXTURE_KEY", "set")];
        let recorded = fixture_launch_marked(tmp.path(), methods.clone(), &key, RECORDED_ARG);
        let agent = row_over(&fixture_launch_marked(tmp.path(), methods, &key, ROW_ARG));
        let row_on_box = recorded_on_box(&agent, recorded);
        let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
            .expect("the row's launch document parses");

        let (flow, mut events, choice_tx, _cancel) = auth_flow(tmp.path());
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = driver.authenticate(flow).await.expect("the flow ran");

        assert_eq!(
            outcome,
            AuthOutcome::Completed {
                call: AuthCall::Authenticate("m-one".to_owned())
            }
        );
        assert_eq!(
            fixture_argv(tmp.path()),
            vec![RECORDED_ARG.to_owned()],
            "the login spawned what the probe recorded, not what the row resolves to today"
        );
        methods_of(events.recv().await);
    }

    /// The fourth of D58's rules, at the login seam: a recording whose command is gone degrades
    /// into resolution rather than failing the flow.
    #[tokio::test]
    async fn a_stale_recording_falls_back_to_resolution() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let methods = json!([{ "id": "m-one", "name": "One" }]);
        let key = [("FIXTURE_KEY", "set")];
        let mut recorded = fixture_launch_marked(tmp.path(), methods.clone(), &key, RECORDED_ARG);
        // Never created: the version-numbered directory an adapter's self-update replaced.
        recorded.command = tmp
            .path()
            .join("gone/fixture-adapter")
            .to_string_lossy()
            .into_owned();
        let agent = row_over(&fixture_launch_marked(tmp.path(), methods, &key, ROW_ARG));
        let row_on_box = recorded_on_box(&agent, recorded);
        let driver = AcpDriver::from_row_with_probe(&agent, Some(&row_on_box), caps_for(&agent))
            .expect("the row's launch document parses");

        let (flow, mut events, choice_tx, _cancel) = auth_flow(tmp.path());
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = driver.authenticate(flow).await.expect("the flow ran");

        assert_eq!(
            outcome,
            AuthOutcome::Completed {
                call: AuthCall::Authenticate("m-one".to_owned())
            }
        );
        assert_eq!(
            fixture_argv(tmp.path()),
            vec![ROW_ARG.to_owned()],
            "a recording that names nothing on disk is not a launch; the row's own document is"
        );
        methods_of(events.recv().await);
    }

    /// A login that cannot start is a spawn failure naming the command, not a silent nothing.
    ///
    /// The command is what the user has to fix, and it is the one thing the message can carry that
    /// they can act on — the row is theirs to edit and the adapter is theirs to install.
    #[tokio::test]
    async fn a_spawn_failure_is_a_spawn_error_naming_the_command() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let command = tmp
            .path()
            .join("never-installed/fixture-adapter")
            .to_string_lossy()
            .into_owned();
        let agent = agent_row(json!({ "command": command, "args": [], "env": {} }));
        let driver = AcpDriver::from_row(&agent, caps_for(&agent))
            .expect("the row's launch document parses");

        let (flow, _events, _choice_tx, _cancel) = auth_flow(tmp.path());
        match driver.authenticate(flow).await {
            Err(DriverError::Spawn(message)) => assert!(
                message.contains(&command),
                "the failure names the command that could not be started: {message}"
            ),
            other => panic!("a command that is not on this box is a spawn error, not {other:?}"),
        }
    }

    /// D5, and the whole of it: the environment a login runs in is the row's, resolved once.
    ///
    /// The fixture refuses `authenticate` unless `FIXTURE_KEY` is in its environment, which is the
    /// shape of the live `-32602` this feature exists to make legible. A row that carries the
    /// variable completes and one that does not is refused **in the agent's own words** — so the
    /// answer to "how does a user supply it" stays "the row's `launch.env`", and this path grows no
    /// second mechanism for putting a variable in front of an adapter.
    #[tokio::test]
    async fn the_flow_reads_a_variable_already_in_the_spawn_environment() {
        for carried in [true, false] {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let extra: &[(&str, &str)] = if carried {
                &[("FIXTURE_KEY", "set")]
            } else {
                &[]
            };
            let agent = row_over(&fixture_launch(
                tmp.path(),
                json!([{ "id": "m-one", "name": "One" }]),
                extra,
            ));
            let driver = AcpDriver::from_row(&agent, caps_for(&agent))
                .expect("the row's launch document parses");

            let (flow, mut events, choice_tx, _cancel) = auth_flow(tmp.path());
            choice_tx
                .send(AuthChoice::Method("m-one".to_owned()))
                .expect("the flow holds the receiver");
            let outcome = driver
                .authenticate(flow)
                .await
                .expect("a refusal is an answer, not a transport failure (D5)");

            match (carried, outcome) {
                (true, AuthOutcome::Completed { call }) => {
                    assert_eq!(call, AuthCall::Authenticate("m-one".to_owned()));
                }
                (false, AuthOutcome::Refused { call, message }) => {
                    assert_eq!(call, AuthCall::Authenticate("m-one".to_owned()));
                    assert!(
                        message.contains("FIXTURE_KEY"),
                        "the refusal names the variable the row is missing: {message}"
                    );
                }
                (carried, other) => panic!(
                    "a row that {} the variable does not end {other:?}",
                    if carried { "carries" } else { "omits" }
                ),
            }
            methods_of(events.recv().await);
        }
    }
}
