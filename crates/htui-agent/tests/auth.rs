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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::{AcpIo, WireFlow, run_auth};
use htui_agent::auth::{
    AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome,
    BrowserPolicy, OpenerCommand, first_url, open_url,
};
use htui_agent::error::DriverError;
use htui_agent::launch::{AcpSettings, ResolvedLaunch};
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

/// An [`AcpIo`] whose far end answers `initialize` with `methods` and then **ends** the moment the
/// returned sender is fired: the adapter dying while the human is still reading the method list.
///
/// On cue rather than straight after the answer, because the two orderings are different cases:
/// this one is only about the flow that got its list and was waiting for a choice, and a fixture
/// that closed at once would race the client's own dispatch of the response it had just written.
fn io_that_ends_on_cue(methods: Value) -> (AcpIo, oneshot::Sender<()>) {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    let (close_tx, close_rx) = oneshot::channel();
    tokio::spawn(async move {
        let (agent_reader, mut agent_writer) = tokio::io::split(agent_end);
        let mut lines = BufReader::new(agent_reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(request) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if request.get("method").and_then(Value::as_str) != Some("initialize") {
                continue;
            }
            let answer = json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned().unwrap_or(Value::Null),
                "result": init_result(methods.clone(), false),
            });
            let mut text = serde_json::to_string(&answer).expect("the response serialises");
            text.push('\n');
            let _ = agent_writer.write_all(text.as_bytes()).await;
            let _ = agent_writer.flush().await;
            break;
        }
        // The cue, and then end of file: dropping both halves is what the client reads as a dead
        // adapter.
        let _ = close_rx.await;
    });
    (
        AcpIo {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
        },
        close_tx,
    )
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
/// Production's two policies: [`AUTH_IDLE_CAP`] and [`BrowserPolicy::Neutralised`]. A case that is
/// about one of them says so by calling [`auth_flow_with`] instead, which is what keeps "the
/// default is what ships" readable in every case that is about something else.
fn auth_flow(
    cwd: &Path,
) -> (
    AuthFlow,
    mpsc::UnboundedReceiver<AuthEvent>,
    oneshot::Sender<AuthChoice>,
    CancellationToken,
) {
    auth_flow_with(cwd, AUTH_IDLE_CAP, BrowserPolicy::Neutralised)
}

/// [`auth_flow`] with the two policies named: the idle cap in milliseconds, and whether the
/// adapter's own browser opener is neutralised.
fn auth_flow_with(
    cwd: &Path,
    idle: Duration,
    browser: BrowserPolicy,
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
            idle,
            browser,
        },
        events_rx,
        choice_tx,
        cancel,
    )
}

/// Every event a finished flow queued, in order.
///
/// Awaited rather than drained with `try_recv`: once [`AuthFlow::events`] has been dropped by both
/// the operation and the wire — which is what a returned flow means — `recv` answers `None` at the
/// end of the queue and cannot answer it early.
async fn drained(events: &mut mpsc::UnboundedReceiver<AuthEvent>) -> Vec<AuthEvent> {
    let mut all = Vec::new();
    while let Some(event) = events.recv().await {
        all.push(event);
    }
    all
}

/// A made-up authorisation link with the punctuation a real one carries (`R-AGT-5`).
const FIXTURE_LINK: &str = "https://h.invalid/login?a=1&b=%2F";

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

/// An adapter that dies while the chooser is open is left to the **caller's** clock (D13).
///
/// Measured rather than assumed, and it is the answer to review L-2, which expected this to be
/// reported as "the agent ended before answering …". It is not reported at all: end of file with no
/// request outstanding is not an answer, and this SDK's `connect_with` does not give up on a
/// foreground future that is parked on a human rather than on a request. So the outer frame's
/// "sender dropped unused" arm is never reached from here, the flow simply waits — and what ends it
/// is the pane's cancel or `authenticate`'s own idle cap, which is the second half of the pair D13
/// describes: the two defences really are one defence.
///
/// The moment a choice *is* made the ordinary path takes over: the request goes out on a closed
/// transport, the SDK marks the error, and `Wire::Ended(Stage::Authenticate)` names the call
/// nobody answered.
#[tokio::test]
async fn an_adapter_that_dies_during_the_chooser_is_left_to_the_callers_clock() {
    let (io, close) = io_that_ends_on_cue(json!([{ "id": "m-one", "name": "One" }]));
    let (flow, mut events, _choice_tx, cancel) = wire_flow(PATIENCE);
    let settings = settings();
    let running = run_auth(io, &settings, flow);
    tokio::pin!(running);

    // The list is in the caller's hands, so the flow is provably past `initialize` and parked on a
    // choice when the adapter goes away.
    let event = tokio::select! {
        outcome = &mut running => panic!("the flow ended before it offered anything: {outcome:?}"),
        event = events.recv() => event,
    };
    let (methods, _, _) = methods_of(event);
    assert_eq!(methods, vec![info("m-one", "One", None)]);

    close.send(()).expect("the fixture is waiting on its cue");
    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut running)
            .await
            .is_err(),
        "end of file with nothing outstanding neither answers the flow nor fails it"
    );

    // And the caller's own token is what gets it out, exactly as it does for an adapter that is
    // still alive and silent.
    cancel.cancel();
    let outcome = running
        .await
        .expect("a cancel is an outcome, not a failure");
    assert_eq!(outcome, AuthOutcome::Cancelled);
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
// The URL scan (plan D15)
// ---------------------------------------------------------------------------------------------

#[test]
fn first_url_finds_the_scheme_anywhere_in_the_line() {
    assert_eq!(
        first_url("open the following link to log in: https://h.invalid/o?a=1&b=%2F#frag")
            .as_deref(),
        Some("https://h.invalid/o?a=1&b=%2F#frag"),
        "the scheme is the only thing every agent's sentence shares, and the query is the link"
    );
    assert_eq!(
        first_url("http://h.invalid/p is where to go").as_deref(),
        Some("http://h.invalid/p")
    );
    assert_eq!(
        first_url("https://h.invalid/one, or else http://h.invalid/two").as_deref(),
        Some("https://h.invalid/one"),
        "`https://` never contains a `http://` prefix, so the smaller index is simply the first"
    );
}

#[test]
fn first_url_stops_at_whitespace_and_brackets_and_strips_trailing_punctuation() {
    for (line, expected) in [
        ("<https://h.invalid/p>.", "https://h.invalid/p"),
        ("(https://h.invalid/p),", "https://h.invalid/p"),
        ("see \"https://h.invalid/p\" now", "https://h.invalid/p"),
        ("`https://h.invalid/p`", "https://h.invalid/p"),
        ("https://h.invalid/p\tand then", "https://h.invalid/p"),
        ("https://h.invalid/p;", "https://h.invalid/p"),
        ("https://h.invalid/p]}", "https://h.invalid/p"),
    ] {
        assert_eq!(
            first_url(line).as_deref(),
            Some(expected),
            "a link a human wrapped or ended a sentence with is still the link: {line}"
        );
    }
}

#[test]
fn first_url_ignores_non_http_schemes() {
    for line in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ftp://h.invalid/p",
        "mailto:nobody@h.invalid",
    ] {
        assert_eq!(
            first_url(line),
            None,
            "the scan admits the two schemes the opener will accept, and no others: {line}"
        );
    }
}

#[test]
fn first_url_is_none_for_a_line_without_one() {
    assert_eq!(first_url(""), None);
    assert_eq!(first_url("waiting for the browser to come back"), None);
    assert_eq!(
        first_url("nothing here but a word: shttp"),
        None,
        "a miss costs the `o` key and nothing else — the line itself is shown either way"
    );
}

// ---------------------------------------------------------------------------------------------
// The browser policy (plan D16)
// ---------------------------------------------------------------------------------------------

/// A launch that spawns nothing, carrying `pairs` as its environment.
fn launch_with_env(pairs: &[(&str, &str)]) -> ResolvedLaunch {
    ResolvedLaunch {
        command: "/nonexistent/htui-fixture-adapter".to_owned(),
        args: Vec::new(),
        env: pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// D16's whole reason: a chain-style opener stops at a candidate that **exists and exits 0**, and
/// falls through to the next one — a terminal browser — at anything else.
///
/// So the assertion is not "the variable is set" but "the value names something this box can run
/// and that succeeds": a nonexistent command would be worse than no policy at all.
#[cfg(unix)]
#[tokio::test]
async fn neutralised_sets_browser_to_an_existing_executable_that_exits_zero() {
    let mut launch = launch_with_env(&[]);
    BrowserPolicy::Neutralised.apply(&mut launch);

    let value = launch
        .env
        .get("BROWSER")
        .expect("the policy inserts exactly one variable")
        .clone();
    assert!(
        Path::new(&value).is_absolute(),
        "resolved, never spelled: macOS keeps its copy somewhere this one is not ({value})"
    );
    assert!(
        Path::new(&value).is_file(),
        "an opener that cannot run this would fall through to the next candidate: {value}"
    );
    let status = tokio::process::Command::new(&value)
        .status()
        .await
        .expect("the neutraliser runs");
    assert!(
        status.success(),
        "and the next candidate is the hijack: {value}"
    );
}

/// Unsetting `DISPLAY` would make the hijack *more* likely, not less: an opener with no display
/// goes looking for a terminal browser. The policy is one variable, and this says which.
#[test]
fn neutralised_leaves_display_alone() {
    let mut launch = launch_with_env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-1")]);
    BrowserPolicy::Neutralised.apply(&mut launch);

    assert_eq!(launch.env.get("DISPLAY").map(String::as_str), Some(":0"));
    assert_eq!(
        launch.env.get("WAYLAND_DISPLAY").map(String::as_str),
        Some("wayland-1")
    );
    assert_eq!(
        launch.env.keys().collect::<Vec<_>>(),
        vec!["BROWSER", "DISPLAY", "WAYLAND_DISPLAY"],
        "exactly one variable is inserted and nothing else is touched"
    );

    let mut inherited = launch_with_env(&[("DISPLAY", ":0")]);
    BrowserPolicy::Inherit.apply(&mut inherited);
    assert_eq!(
        inherited.env.keys().collect::<Vec<_>>(),
        vec!["DISPLAY"],
        "the control writes nothing at all"
    );
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
    /// the pid, the argv, the environment dump and the call log go; `FIXTURE_INIT` is the
    /// `initialize` result on one line; `FIXTURE_KEY` unset makes `authenticate` refuse;
    /// `FIXTURE_HOLD` makes it never answer; `FIXTURE_DIE` makes the process write to stderr and
    /// exit instead of answering; `FIXTURE_URL` is a link printed to stderr before the answer,
    /// `FIXTURE_TWICE` prints the same link a second time, `FIXTURE_TICKS` prints that many lines
    /// a tenth of a second apart, and `FIXTURE_HIJACK` is the observed hijack — the adapter's own
    /// browser opener, which runs `$BROWSER` when it has one and writes alt-screen sequences to
    /// **stdout** when it does not. Those sequences carry no trailing newline, because a terminal
    /// browser's do not and because that is what makes them fatal: they are glued to the front of
    /// the agent's own next line.
    ///
    /// It writes its own `$@` and its own `env` because a file is the only channel a case has for
    /// "what did the kernel actually start this with": argv answers which launch was spawned, and
    /// the environment dump answers which variables reached the child.
    ///
    /// The id is echoed back **as it arrived**, quotes and all: this SDK sends a UUID *string* as
    /// its JSON-RPC id, and a fixture that re-quoted it — or that assumed a number — would answer
    /// with a line the client's decoder skips, which is a handshake timeout wearing a costume.
    const AGENT_SH: &str = r#"
echo $$ > "$FIXTURE_DIR/pid"
printf '%s\n' "$@" > "$FIXTURE_DIR/argv"
env > "$FIXTURE_DIR/env"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\|[0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      echo initialize >> "$FIXTURE_DIR/calls"
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$FIXTURE_INIT" ;;
    *'"method":"authenticate"'*)
      echo authenticate >> "$FIXTURE_DIR/calls"
      if [ -n "$FIXTURE_URL" ]; then echo "open the following link to log in: $FIXTURE_URL" >&2; fi
      if [ -n "$FIXTURE_TWICE" ]; then echo "still waiting; the link again: $FIXTURE_URL" >&2; fi
      if [ -n "$FIXTURE_HIJACK" ]; then
        if [ -n "$BROWSER" ]; then
          "$BROWSER" "$FIXTURE_URL"
        else
          printf '\033[?1049h\033[1;24r opening %s' "$FIXTURE_URL"
        fi
      fi
      if [ -n "$FIXTURE_TICKS" ]; then
        tick=0
        while [ "$tick" -lt "$FIXTURE_TICKS" ]; do
          sleep 0.1
          tick=$((tick + 1))
          echo "still waiting for the browser, $tick" >&2
        done
      fi
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
        agent_box_row(agent, BoxId::new(), &snapshot, Utc::now())
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

    // -----------------------------------------------------------------------------------------
    // The hand-off: stderr, the link, the browser and the clock (plan D13, D15, D16)
    // -----------------------------------------------------------------------------------------

    /// The one method every case below offers.
    fn one_method() -> Value {
        json!([{ "id": "m-one", "name": "One" }])
    }

    /// A driver over a row that *is* the fixture launch, with `extra` in its environment.
    fn fixture_driver(dir: &Path, extra: &[(&str, &str)]) -> (Agent, AcpDriver) {
        let agent = row_over(&fixture_launch(dir, one_method(), extra));
        let driver = AcpDriver::from_row(&agent, caps_for(&agent))
            .expect("the row's launch document parses");
        (agent, driver)
    }

    /// The pid the fixture wrote for itself.
    fn fixture_pid(dir: &Path) -> u32 {
        std::fs::read_to_string(dir.join("pid"))
            .expect("the fixture recorded its pid")
            .trim()
            .parse()
            .expect("a pid is a number")
    }

    /// The value the fixture's own environment carried for `name`, if any.
    fn fixture_env(dir: &Path, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        std::fs::read_to_string(dir.join("env"))
            .expect("the fixture dumped its environment")
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(ToOwned::to_owned))
    }

    /// Only the [`AuthEvent::Url`]s of a finished flow.
    fn urls_of(events: &[AuthEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                AuthEvent::Url(url) => Some(url.as_str()),
                _ => None,
            })
            .collect()
    }

    /// How many [`AuthEvent::Line`]s a finished flow carried.
    fn lines_of(events: &[AuthEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, AuthEvent::Line(_)))
            .count()
    }

    /// D15's deduplication, over a real adapter that says the same thing twice: two lines, one
    /// link.
    ///
    /// An adapter that reprints its link while it waits is the normal case, not a strange one —
    /// and a pane that grew a second "open this" row every time would be unreadable by the time
    /// the human came back.
    #[tokio::test]
    async fn a_url_is_reported_once_per_flow() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) = fixture_driver(
            tmp.path(),
            &[
                ("FIXTURE_KEY", "set"),
                ("FIXTURE_URL", FIXTURE_LINK),
                ("FIXTURE_TWICE", "1"),
            ],
        );

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
        let events = drained(&mut events).await;
        assert_eq!(
            lines_of(&events),
            2,
            "every line the adapter wrote is shown, verbatim: {events:?}"
        );
        assert_eq!(
            urls_of(&events),
            vec![FIXTURE_LINK],
            "the same link twice is one link: {events:?}"
        );
    }

    /// H-11: the variable reaches the login's own child and nothing else.
    ///
    /// A policy written into the row, or into what the probe records, would follow the agent into
    /// every chat it ever runs — and `probe.resolved` is a document the next spawn reads back.
    #[tokio::test]
    async fn the_policy_touches_the_auth_launch_only() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) = fixture_driver(tmp.path(), &[("FIXTURE_KEY", "set")]);

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
        methods_of(events.recv().await);

        let value = fixture_env(tmp.path(), "BROWSER")
            .expect("the login's own child got the variable the policy inserts");
        assert!(
            Path::new(&value).is_file(),
            "and it names something that exists: {value}"
        );

        let resolved = driver
            .launch_in(tmp.path())
            .await
            .expect("the row resolves");
        assert!(
            !resolved.env.contains_key("BROWSER"),
            "resolution is what a chat spawns and what a probe records; the policy is not in it: \
             {:?}",
            resolved.env.keys().collect::<Vec<_>>()
        );
    }

    /// The PRD's live finding as a pair, and the reason D16 exists.
    ///
    /// The fixture is the hijack: on `authenticate` it hands the link to `$BROWSER` when it has
    /// one, and writes alt-screen sequences to **stdout** — the JSON-RPC channel — when it does
    /// not. With the policy the stream survives and the link still reaches the pane.
    ///
    /// The launch is the control's, byte for byte, empty `BROWSER` included: the policy overwrites
    /// that value, so the only thing this case has that the next one does not is the policy, and a
    /// `BROWSER` a developer happens to have exported cannot stand in for it.
    ///
    /// The idle cap is [`PATIENCE`] rather than the production ten minutes for the reason that
    /// constant exists: a login this fixture answers in milliseconds does not reach the clock, and
    /// bounding it here is what makes a policy that stopped working a named failure in two seconds
    /// instead of a suite that hangs for the length of the real cap.
    #[tokio::test]
    async fn with_the_policy_the_protocol_stream_survives_the_agents_browser() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) = fixture_driver(
            tmp.path(),
            &[
                ("FIXTURE_KEY", "set"),
                ("FIXTURE_URL", FIXTURE_LINK),
                ("FIXTURE_HIJACK", "1"),
                ("BROWSER", ""),
            ],
        );

        let (flow, mut events, choice_tx, _cancel) =
            auth_flow_with(tmp.path(), PATIENCE, BrowserPolicy::Neutralised);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = driver
            .authenticate(flow)
            .await
            .expect("the adapter's opener wrote nothing into the protocol channel");

        assert_eq!(
            outcome,
            AuthOutcome::Completed {
                call: AuthCall::Authenticate("m-one".to_owned())
            }
        );
        let events = drained(&mut events).await;
        assert_eq!(
            urls_of(&events),
            vec![FIXTURE_LINK],
            "and the user still gets the link to open themselves: {events:?}"
        );
    }

    /// The control, and the case that would have caught the live hijack.
    ///
    /// The same fixture, the same launch, one value removed. `BROWSER` is written **empty** rather
    /// than merely left out because a child inherits this process's environment too, and a
    /// developer with a browser configured would otherwise be running it: empty is what the
    /// adapter reads as "no opener configured", which is the state the live box was in.
    ///
    /// **What the corruption costs, measured rather than assumed.** The plan expected a decode
    /// error — `Err(Transport)`. This SDK does not fail on one: an undecodable stdout line is
    /// *skipped*, which is the same tolerance `AGENT_SH`'s doc records for a response whose id it
    /// re-quoted. So escape sequences that arrive on a line of their own cost nothing at all, and
    /// the sequences that cost everything are the ones a terminal browser actually writes — no
    /// trailing newline, so the agent's own answer is glued to the end of them and skipped with
    /// them. The login then waits for an answer that was already thrown away, which is exactly the
    /// abandoned flow D13's clock exists for: the two defences are one defence, and this is the
    /// case that says so.
    #[tokio::test]
    async fn without_the_policy_the_same_agent_corrupts_the_stream() {
        let cap = Duration::from_millis(400);
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) = fixture_driver(
            tmp.path(),
            &[
                ("FIXTURE_KEY", "set"),
                ("FIXTURE_URL", FIXTURE_LINK),
                ("FIXTURE_HIJACK", "1"),
                ("BROWSER", ""),
            ],
        );

        let (flow, _events, choice_tx, _cancel) =
            auth_flow_with(tmp.path(), cap, BrowserPolicy::Inherit);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");

        match driver.authenticate(flow).await {
            Ok(AuthOutcome::Idle { after }) => assert_eq!(after, cap),
            other => panic!(
                "without the policy the adapter's own browser writes into the JSON-RPC channel \
                 and the answer goes with it, so the flow cannot end any way but abandoned — but \
                 this one answered {other:?}"
            ),
        }
        assert_not_running(fixture_pid(tmp.path()), "the child of a hijacked flow").await;
    }

    /// D13: silence, not elapsed time — and the child does not outlive the clock.
    #[tokio::test]
    async fn an_idle_flow_is_killed_after_the_cap_and_reported_idle() {
        let cap = Duration::from_millis(200);
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) =
            fixture_driver(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]);

        let (flow, mut events, choice_tx, _cancel) =
            auth_flow_with(tmp.path(), cap, BrowserPolicy::Neutralised);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = driver
            .authenticate(flow)
            .await
            .expect("giving up on a human is an outcome, not a failure");

        assert_eq!(
            outcome,
            AuthOutcome::Idle { after: cap },
            "a flow nobody is watching says so, rather than reporting the cancel it was killed by"
        );
        methods_of(events.recv().await);
        assert_not_running(fixture_pid(tmp.path()), "the child of an idle flow").await;
    }

    /// The other half of D13: a line is a sign of life, so an adapter that keeps talking outlives
    /// a cap shorter than the login it is running.
    ///
    /// Six lines a tenth of a second apart under a cap four times the gap: the claim is that each
    /// line restarts the clock, not that the margin is tight, and a cap measured from the spawn
    /// would have killed this flow at 400 ms.
    #[tokio::test]
    async fn a_stderr_line_resets_the_idle_clock() {
        let cap = Duration::from_millis(400);
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let (_agent, driver) = fixture_driver(
            tmp.path(),
            &[("FIXTURE_KEY", "set"), ("FIXTURE_TICKS", "6")],
        );

        let (flow, mut events, choice_tx, _cancel) =
            auth_flow_with(tmp.path(), cap, BrowserPolicy::Neutralised);
        choice_tx
            .send(AuthChoice::Method("m-one".to_owned()))
            .expect("the flow holds the receiver");
        let outcome = driver.authenticate(flow).await.expect("the flow ran");

        assert_eq!(
            outcome,
            AuthOutcome::Completed {
                call: AuthCall::Authenticate("m-one".to_owned())
            },
            "an adapter that is still writing is still working"
        );
        let events = drained(&mut events).await;
        assert!(
            lines_of(&events) >= 6,
            "every one of those lines is what kept the flow alive: {events:?}"
        );
    }

    // -----------------------------------------------------------------------------------------
    // The opener (plan D17)
    // -----------------------------------------------------------------------------------------

    /// A `sh` script that records what it was handed and what its three streams are, then outlives
    /// its caller by half a minute.
    ///
    /// The directory is baked into the text because [`open_url`] hands the opener a URL and
    /// nothing else — no environment, no arguments — which is the whole of D17's contract.
    ///
    /// The three streams are read into shell variables **before** anything is redirected: `sh`
    /// applies a command's redirection to its own descriptors first, so a `readlink` written
    /// straight into a file reports that file as its stdout and answers a question nobody asked.
    fn recorder(dir: &Path) -> PathBuf {
        let path = dir.join("opener.sh");
        let dir = dir.display();
        executable(
            &path,
            &format!(
                "#!/bin/sh\n\
                 if [ -t 0 ] || [ -t 1 ] || [ -t 2 ]; then terminal=tty; else terminal=notty; fi\n\
                 zero=$(readlink /proc/$$/fd/0)\n\
                 one=$(readlink /proc/$$/fd/1)\n\
                 two=$(readlink /proc/$$/fd/2)\n\
                 printf '%s\\n%s\\n%s\\n%s\\n' \"$terminal\" \"$zero\" \"$one\" \"$two\" \
                 >> \"{dir}/stdio\"\n\
                 printf '%s\\n' \"$1\" >> \"{dir}/opened\"\n\
                 sleep 30\n"
            ),
        );
        path
    }

    /// [`open_url`], retrying the `ETXTBSY` window [`executable`] describes, and answering how
    /// long the attempt that got through took.
    async fn opened(
        url: &str,
        opener: &OpenerCommand,
    ) -> (htui_agent::error::Result<()>, Duration) {
        for _ in 0..20u32 {
            let started = std::time::Instant::now();
            match open_url(url, opener).await {
                Err(DriverError::Spawn(message)) if message.contains("Text file busy") => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                other => return (other, started.elapsed()),
            }
        }
        panic!("the opener never got past the `ETXTBSY` window");
    }

    /// The lines the recorder wrote to `name`.
    fn recorded(dir: &Path, name: &str) -> Vec<String> {
        std::fs::read_to_string(dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// D17 closes the one door the scan could otherwise open, and closes it **before** a process
    /// exists: `file:` and `javascript:` are refused, not spawned and then regretted.
    #[tokio::test]
    async fn open_url_refuses_a_non_http_scheme_before_spawning() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let opener = OpenerCommand::Custom(recorder(tmp.path()));

        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://h.invalid/p",
            "h.invalid/no-scheme-at-all",
        ] {
            match open_url(url, &opener).await {
                Err(DriverError::Transport(message)) => assert!(
                    message.contains("http"),
                    "the refusal says which schemes are opened: {message}"
                ),
                other => panic!("`{url}` is not a link `htui` opens, but it answered {other:?}"),
            }
        }
        assert!(
            !tmp.path().join("opened").exists(),
            "the refusal happened before any process did"
        );
    }

    /// H-3 and H-4 together: the opener gets the link, gets no terminal, and is never waited on.
    ///
    /// The recorder sleeps for thirty seconds. A call that returns in milliseconds is the whole
    /// claim: `htui` neither blocks on the browser nor owns the tree it starts.
    #[tokio::test]
    async fn open_url_spawns_with_null_stdio_and_does_not_wait() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let opener = OpenerCommand::Custom(recorder(tmp.path()));

        let (result, elapsed) = opened(FIXTURE_LINK, &opener).await;
        result.expect("the opener spawned");
        assert!(
            elapsed < Duration::from_millis(500),
            "the opener sleeps for thirty seconds; the call may not: {elapsed:?}"
        );

        until("the opener to record what it was handed", || {
            !recorded(tmp.path(), "opened").is_empty()
        })
        .await;
        assert_eq!(
            recorded(tmp.path(), "opened"),
            vec![FIXTURE_LINK.to_owned()],
            "the URL travels as the opener's one argument, unquoted and unmangled"
        );

        let stdio = recorded(tmp.path(), "stdio");
        assert_eq!(
            stdio.first().map(String::as_str),
            Some("notty"),
            "an opener with a terminal is an opener that can seize `htui`'s screen: {stdio:?}"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            stdio[1..],
            ["/dev/null", "/dev/null", "/dev/null"],
            "all three streams, not just stdout"
        );
    }
}
