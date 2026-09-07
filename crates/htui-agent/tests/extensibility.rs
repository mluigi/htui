//! `R-AGT-5`, as a test rather than a claim (plan MOD-2 T9, milestone 2's acceptance test).
//!
//! An agent the codebase has never heard of — `zeta`, named in no Rust source, no seed and no
//! other test — is written into the store as a registry row, read back out, and driven through
//! every case of the shared conformance list. If any of that needed a code change, this file
//! would need one too, and it does not.
//!
//! The sweep in [`the_codebase_has_never_heard_of_zeta`] is what keeps the claim honest: it walks
//! the tree and fails if any source, seed or test other than this file mentions the name.
#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use htui_agent::conformance::{self, CaseHarness, Script, ScriptEvent};
use htui_agent::driver::{AgentDriver, PermissionPolicy, SessionSpec, ToolExposure};
use htui_agent::error::DriverError;
use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk, ToolKind};
use htui_agent::event::{ToolCallEvent, ToolResultEvent, ToolResultStatus};
use htui_agent::fake::FakeAdapter;
use htui_agent::launch::{AgentLaunch, ToolMap, resolve};
use htui_agent::record::{Recorder, pump};
use htui_agent::registry::DriverFactory;
use htui_core::model::{Agent, AgentId, ChatRunSpec, agent::seed_rows};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{MemStore, ReadStore, WriteStore};

/// The unknown agent's `agent` row, exactly as the Settings tab would write it: JSON, no Rust
/// constructor, no entry anywhere in the codebase.
const ZETA: &str = r#"{
  "name": "zeta",
  "transport": "cli",
  "billing": "per_token",
  "models": [],
  "default_model": null,
  "launch": {
    "command": "${zeta}",
    "args": ["--serve"],
    "env": { "ZETA_TOKEN": "${zeta_token}" },
    "discovery": {
      "tools": { "zeta": { "kind": "path", "names": ["zeta"] } },
      "handshake": false
    }
  },
  "settings": {
    "cli": { "stream": "fake", "permission_mode": "ask", "extra_args": [] }
  }
}"#;

/// The token the row's environment carries, and the string no persisted row may contain.
const ZETA_TOKEN: &str = "zeta-secret-9f8e7d";

/// Parses [`ZETA`] into an [`Agent`] the way a Settings-tab save would: minted id, `enabled`, the
/// caller's clock.
fn zeta_row() -> Agent {
    let mut row: serde_json::Value = serde_json::from_str(ZETA).expect("the zeta row parses");
    let object = row.as_object_mut().expect("an object");
    object.insert(
        "id".to_owned(),
        serde_json::to_value(AgentId::new()).expect("an id serialises"),
    );
    object.insert("enabled".to_owned(), serde_json::Value::Bool(true));
    let now = conformance::epoch();
    object.insert(
        "created_at".to_owned(),
        serde_json::to_value(now).expect("a timestamp serialises"),
    );
    object.insert(
        "updated_at".to_owned(),
        serde_json::to_value(now).expect("a timestamp serialises"),
    );
    serde_json::from_value(row).expect("the completed row is an Agent")
}

/// Every `.rs` under `crates/*/src` and `crates/*/tests`, plus every `crates/*/seeds/*.json`.
fn tree_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels above this crate")
        .join("crates");

    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // `target/` can appear per crate under some layouts and holds copies of sources.
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "rs" || ext == "json")
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn the_codebase_has_never_heard_of_zeta() {
    let this_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/extensibility.rs");
    let mut offenders = Vec::new();

    for path in tree_files() {
        if path == this_file {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.contains("zeta") {
            offenders.push(path);
        }
    }

    assert!(
        offenders.is_empty(),
        "`R-AGT-5` says a new agent needs a registry row and no source change. These files name \
         it, so the proof below would be proving the wrong thing: {offenders:?}"
    );
}

#[tokio::test]
async fn an_unknown_agent_reaches_a_driver_from_its_row_alone() {
    let store = MemStore::demo();
    store
        .upsert_agent(&zeta_row())
        .await
        .expect("the row saves");

    // Read **back** through the store: the driver is built from what was persisted, not from the
    // literal this test happens to hold.
    let summaries = store.agents().await.expect("the registry reads");
    let zeta = summaries
        .iter()
        .find(|summary| summary.agent.name == "zeta")
        .expect("zeta is in the registry");

    // A transport needs a wire, so the fake needs a script even when the case never pulls an
    // event: `build` refusing an empty slot is the behaviour a real adapter has when its process
    // cannot start, and silently substituting an empty script would hide it.
    let adapter = FakeAdapter::new();
    adapter.load(Script::one_turn(Vec::new()));
    let mut factory = DriverFactory::new();
    factory.register("cli/fake", Box::new(adapter));

    let driver = factory
        .driver_for(&zeta.agent, zeta.on_box.as_ref())
        .expect("a `cli/fake` adapter is registered");

    assert_eq!(driver.name(), "zeta");

    let caps = driver.caps();
    // ANA-4 §4.3, verbatim: the three a CLI transport cannot do.
    assert!(!caps.permission_requests);
    assert!(!caps.edit_proposals);
    assert!(!caps.plans);
    // §6.2 maps a stdin NDJSON user message to `follow_up`, so a CLI follow-up stays in session.
    assert!(caps.follow_up_in_session);
    assert!(caps.usage);
}

#[test]
fn the_factory_is_keyed_by_transport_not_by_agent() {
    let factory = DriverFactory::with_test_support();

    // One entry per adapter, zero per agent. A `"zeta"` builder would fail here.
    assert_eq!(factory.adapter_ids(), ["cli/fake"]);

    let mut nonesuch = zeta_row();
    nonesuch.settings["cli"]["stream"] = serde_json::Value::String("nonesuch".to_owned());
    match factory.driver_for(&nonesuch, None) {
        Err(DriverError::UnknownAdapter(id)) => assert_eq!(id, "cli/nonesuch"),
        other => panic!("an unregistered stream is `UnknownAdapter`, got {other:?}"),
    }

    // The seeded `claude` row goes through the same factory and is refused for the same reason:
    // milestone 2 registers no ACP adapter. The factory knows transports, never agents.
    let claude = seed_rows(conformance::epoch())
        .into_iter()
        .find(|agent| agent.name == "claude")
        .expect("claude is seeded");
    match factory.driver_for(&claude, None) {
        Err(DriverError::UnknownAdapter(id)) => assert_eq!(id, "acp"),
        other => panic!("ACP lands in milestone 3, got {other:?}"),
    }
}

#[test]
fn the_row_launches_from_a_probed_tool_map() {
    let zeta = zeta_row();
    let launch: AgentLaunch =
        serde_json::from_value(zeta.launch.clone()).expect("the launch row is an AgentLaunch");

    let tools: ToolMap = BTreeMap::from([
        ("zeta".to_owned(), "/opt/zeta/bin/zeta".to_owned()),
        ("zeta_token".to_owned(), ZETA_TOKEN.to_owned()),
    ]);
    let resolved = resolve(&launch, &tools).expect("both placeholders resolve");

    assert_eq!(resolved.command, "/opt/zeta/bin/zeta");
    assert_eq!(resolved.args, ["--serve"]);
    assert_eq!(
        resolved.env.get("ZETA_TOKEN").map(String::as_str),
        Some(ZETA_TOKEN)
    );
}

#[test]
fn every_seed_row_deserialises_into_the_launch_types() {
    for agent in seed_rows(conformance::epoch()) {
        let launch: AgentLaunch =
            serde_json::from_value(agent.launch.clone()).unwrap_or_else(|error| {
                panic!("{}'s launch row is an AgentLaunch: {error}", agent.name)
            });
        assert!(
            !launch.command.is_empty(),
            "{} declares a command",
            agent.name
        );
        let settings: htui_agent::launch::AgentSettings =
            serde_json::from_value(agent.settings.clone()).unwrap_or_else(|error| {
                panic!("{}'s settings row is AgentSettings: {error}", agent.name)
            });
        assert_eq!(settings.acp.protocol_version, 1, "{}", agent.name);
    }
}

/// The harness of the acceptance run: a script becomes a driver **through the factory**, from the
/// stored row, with no mention of the fake beyond the adapter id the row itself names.
#[derive(Debug)]
struct ZetaHarness {
    factory: DriverFactory,
    row: Agent,
    adapter: FakeAdapter,
}

impl CaseHarness for ZetaHarness {
    fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
        self.adapter.load(script);
        self.factory
            .driver_for(&self.row, None)
            .expect("the row names a registered adapter")
    }
}

#[tokio::test]
async fn the_unknown_agent_passes_every_conformance_case() {
    let store = MemStore::demo();
    store
        .upsert_agent(&zeta_row())
        .await
        .expect("the row saves");
    let row = store
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|summary| summary.agent.name == "zeta")
        .expect("zeta is in the registry")
        .agent;

    let adapter = FakeAdapter::new();
    let mut factory = DriverFactory::new();
    factory.register("cli/fake", Box::new(adapter.clone()));

    let harness = ZetaHarness {
        factory,
        row,
        adapter,
    };
    conformance::run_all(&harness, || async { MemStore::demo() }).await;
}

/// The sweep plan T9 item 6 promised, and the one this file was missing: the zeta row's **real**
/// resolved token goes through a whole session — factory, driver, recorder, scrubber, store — and
/// no persisted row may carry it.
///
/// The conformance run above proves masking generically, but against `conformance`'s own
/// `FAKE_TOKEN`, never against the value this row actually resolves. That gap is exactly where a
/// scrubber wired to the wrong map would survive a green suite.
#[tokio::test]
async fn the_resolved_token_reaches_no_persisted_row() {
    let store = MemStore::demo();
    let zeta = zeta_row();
    store.upsert_agent(&zeta).await.expect("the row saves");

    // The env the session runs with is the *resolved* launch env, not a literal: this is the
    // value a probed box would really hand the child process.
    let launch: AgentLaunch =
        serde_json::from_value(zeta.launch.clone()).expect("the launch row is an AgentLaunch");
    let tools: ToolMap = BTreeMap::from([
        ("zeta".to_owned(), "/opt/zeta/bin/zeta".to_owned()),
        ("zeta_token".to_owned(), ZETA_TOKEN.to_owned()),
    ]);
    let resolved = resolve(&launch, &tools).expect("both placeholders resolve");
    assert!(
        resolved.env.values().any(|value| value == ZETA_TOKEN),
        "the session env carries the resolved token, or this test proves nothing"
    );

    let chat = ChatRunSpec::mint(
        htui_core::fixtures::ids::PROJECT_HTUI,
        htui_core::fixtures::ids::BOX,
        htui_core::fixtures::ids::USER,
        Some(zeta.id),
        None,
    );
    store
        .start_chat_run(&chat)
        .await
        .expect("the chat rows mint");

    let adapter = FakeAdapter::new();
    // The agent echoes the token back in three shapes a real one plausibly would: assistant text,
    // a tool call's arguments, and a tool result's output.
    adapter.load(Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            message_id: Some("m-1".to_owned()),
            text: format!("I will use ZETA_TOKEN={ZETA_TOKEN} for this call."),
        })),
        ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "bash".to_owned(),
            tool_kind: ToolKind::Execute,
            input: serde_json::json!({ "command": format!("curl -H 'auth: {ZETA_TOKEN}'") }),
            locations: Vec::new(),
        })),
        ScriptEvent::Emit(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call-1".to_owned(),
            status: ToolResultStatus::Completed,
            output: Some(serde_json::json!({ "text": format!("echoed {ZETA_TOKEN}") })),
            locations: Vec::new(),
            terminal_reason: None,
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ]));
    let mut factory = DriverFactory::new();
    factory.register("cli/fake", Box::new(adapter));

    let stored = store
        .agents()
        .await
        .expect("the registry reads")
        .into_iter()
        .find(|summary| summary.agent.name == "zeta")
        .expect("zeta is in the registry")
        .agent;
    let driver = factory
        .driver_for(&stored, None)
        .expect("the row names a registered adapter");

    let spec = SessionSpec {
        agent_id: stored.id,
        step_id: chat.step_id,
        cwd: PathBuf::from("."),
        extra_dirs: Vec::new(),
        env: resolved.env.clone(),
        model: None,
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        // `raw` retained on purpose: an unscrubbed verbatim blob is the likeliest place for a
        // secret to survive, so the sweep is weaker without it.
        retain_raw: true,
        resume: None,
    };

    // The scrubber is built from the same resolved env the session runs with, which is the wiring
    // milestone 3 has to reproduce.
    let scrubber = MinimalScrubber::new(resolved.env.values().cloned());
    let mut session = driver
        .start(spec, "use the token".to_owned())
        .await
        .expect("the session starts");
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, true, None);
    recorder
        .record_prompt(
            &format!("use ZETA_TOKEN={ZETA_TOKEN}"),
            serde_json::json!({}),
            conformance::epoch(),
        )
        .await
        .expect("the prompt records");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("the turn runs to done");
    recorder.finish().await.expect("the session closes clean");

    let rows = store
        .step_events(chat.step_id)
        .await
        .expect("reading the log must not fail")
        .expect("the chat step has a log");
    assert!(
        rows.len() >= 4,
        "prompt, banner, text, call and result all persist: {}",
        rows.len()
    );

    let rendered = serde_json::to_string(&rows).expect("the rows serialise");
    assert!(
        !rendered.contains(ZETA_TOKEN),
        "the resolved token survived into a persisted row: {rendered}"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "it is masked in place, not silently dropped: {rendered}"
    );
}
