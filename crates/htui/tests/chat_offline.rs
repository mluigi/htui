//! The offline chat, from the shell (MOD-25; previously `docs/ANA-4.md` §11 criterion 12, MOD-2
//! milestone 4 T21).
//!
//! **What this file proves now.** `htui` is online-only. A chat started on a box whose Postgres is
//! unreachable is **refused** with the one sentence `htui_store::DATABASE_UNREACHABLE` carries, no
//! step is minted, and nothing is written to `<cache_dir>/pending/`. That is
//! `an_offline_chat_is_refused_with_the_unreachable_warning`, driven against the production pieces
//! and nothing else — a `Backend::Offline` over a real mirror, the production `AgentRuntime`
//! driving a scripted transport through the same registry the ACP one uses. Its online negative,
//! `an_online_chat_header_says_nothing_about_a_buffer`, still runs: a chat over a store the
//! maintainer can read back says nothing about a buffer. The only thing the tests supply is the
//! mirror's contents (`testkit::seed_mirror`), because an offline box is by definition one that
//! has already synced.
//!
//! **What this file keeps, ignored.** Criterion 12 read: a chat started with the store unreachable
//! records its rows to `<cache_dir>/pending/`, and the next connection uploads them. It is
//! withdrawn with the mode it proved — see `docs/decisions/mod/mod-2.md`, "Known, accepted, and
//! handed on". The four cases that proved that mode are `#[ignore]`d rather than deleted, bodies
//! byte-for-byte: the two buffer-writing cases, the `HTUI_TEST_DATABASE_URL`-gated end-to-end
//! criterion-12 proof (`a_buffered_chat_lands_in_postgres_on_the_next_connection`, which prints
//! `testkit::SKIP` without a server, plan D13), and D33's `app_user` refusal, now unreachable from
//! the shell because `start()` refuses at the writer before it asks the mirror who this user is.
//! They still compile and are expected to fail if run with `--ignored`; that is what "disabled"
//! means. They are the evidence the reversal would need, and a later CLEAN item removes them.
//!
//! Only the **write** side is disabled. `upload_pending` still runs on every refresh pass, so a
//! buffer left by an earlier build still lands on the next connection.
#![cfg(feature = "testkit")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use htui::agent_worker::AgentRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::ChatTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::DriverCaps;
use htui_agent::event::{
    DoneEvent, DriverEvent, StopReason, TextChunk, ToolCallEvent, ToolKind, ToolResultEvent,
    ToolResultStatus, UsageEvent,
};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::model::{Agent, AgentBox, AgentId, Billing, Transport};
use htui_core::store::{MemStore, WriteStore as _};
use htui_store::{Backend, CacheStore, PgStore, testkit};
use serde_json::json;

/// A registry row the factory reaches by **row data**: `acp`, so the session is fully capable and
/// the tab draws no capability banner over the header this test is about (plan D12).
fn scripted_row(id: AgentId) -> Agent {
    Agent {
        id,
        name: "scripted".to_owned(),
        transport: Transport::Acp,
        billing: Billing::Subscription,
        models: Vec::new(),
        default_model: Some("sonnet".to_owned()),
        launch: json!({ "command": "unused", "args": [] }),
        settings: json!({ "cli": { "stream": "fake", "permission_mode": "ask",
                                   "extra_args": [] } }),
        enabled: true,
        created_at: htui_core::fixtures::demo_at(0, 0),
        updated_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// Lets the test keep the adapter it loaded a script into.
#[derive(Debug)]
struct SharedAdapter(Arc<FakeAdapter>);

impl htui_agent::registry::TransportBuilder for SharedAdapter {
    fn build(
        &self,
        agent: &Agent,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn htui_agent::driver::AgentDriver>, htui_agent::error::DriverError> {
        self.0.build(agent, on_box, caps)
    }
}

/// Types `text` into the composer and submits it, as `tests/chat.rs` does.
fn compose(harness: &mut Harness, text: &str) {
    harness.key("i");
    for ch in text.chars() {
        if ch == ' ' {
            harness.key("space");
        } else {
            harness.key(&ch.to_string());
        }
    }
    harness.key("enter");
}

/// A mirror holding the demo world, this OS user, and one registry row the fake can build.
///
/// The user's **name** is the one thing rewritten: `CacheStore::this_user` resolves the OS user
/// against the mirror by name (D33), which is what makes the run this test uploads carry the same
/// author an online one would. The `TempDir` is returned because dropping it deletes the mirror.
async fn offline_mirror(agent_id: AgentId) -> (tempfile::TempDir, CacheStore) {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "offline-chat", PgStore::schema_version())
        .await
        .expect("a fresh mirror");

    let mut demo = htui_core::fixtures::demo_data();
    for user in &mut demo.users {
        user.name = htui_store::identity::os_user_name();
    }
    // The fixture's own rows are `acp` agents this build has no adapter for. Replacing them leaves
    // the tab exactly one agent to talk to, which is what makes the header deterministic.
    demo.agents = vec![scripted_row(agent_id)];
    testkit::seed_mirror(&cache, &demo)
        .await
        .expect("the mirror is seeded");
    (root, cache)
}

/// The shell over an offline backend, with the Chat tab and a scripted transport behind it.
///
/// The frame is 140 columns wide rather than the harness default of 100: the D42 suffix is 43
/// characters on top of a header that already carries an agent, a model, a project name and a
/// 41-character session id, and a truncated header would hide the very words the snapshot exists
/// to pin.
///
/// The [`testkit::KeyringGuard`] comes back with the harness rather than being taken by each case:
/// since MOD-15 M6 `App::start` issues a `ConnectionInfo`, and over a non-`Memory` backend that
/// read reaches `secret::get_dsn`. Returned rather than installed and dropped here because the
/// guard has to outlive `settle()`, and returned rather than left to the caller because a case
/// added later would otherwise open the developer's own OS keyring and nothing would say so.
async fn offline_harness(cache: &CacheStore, script: Script) -> (testkit::KeyringGuard, Harness) {
    let keyring = testkit::mock_keyring().await;
    let adapter = Arc::new(FakeAdapter::new());
    adapter.load(script);
    let mut factory = DriverFactory::new();
    factory.register("acp", Box::new(SharedAdapter(Arc::clone(&adapter))));

    let harness = Harness::over_backend(Backend::Offline {
        cache: cache.clone(),
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(ChatTab::new()))
    .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::from_millis(0)))
    // `offline · 0s` would age between the render and the next tick; the top bar is not what this
    // suite is about, and the harness override is how every other suite pins it.
    .with_store_state("offline · 0s", None)
    .size(140, 30);
    (keyring, harness)
}

/// The turn every case plays: a thought, some text, a tool call and its result, a usage report and
/// the end of the turn — one row of every shape the buffer has to carry.
fn one_turn() -> Script {
    Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            text: "Reading it now. ".to_owned(),
            message_id: Some("m1".to_owned()),
        })),
        ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "src/main.rs".to_owned(),
            tool_kind: ToolKind::Read,
            input: json!({ "path": "src/main.rs" }),
            locations: Vec::new(),
        })),
        ScriptEvent::Emit(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call-1".to_owned(),
            status: ToolResultStatus::Completed,
            output: Some(json!("fn main() {}")),
            locations: Vec::new(),
            terminal_reason: None,
        })),
        ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
            text: "It is empty.".to_owned(),
            message_id: Some("m1".to_owned()),
        })),
        ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
            input_tokens: Some(120),
            output_tokens: Some(31),
            cost_micros: Some(940),
            ..UsageEvent::default()
        })),
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })),
    ])
}

/// MOD-25, from the shell: a box whose Postgres is unreachable **refuses** the chat instead of
/// buffering it. The refusal is the one sentence `htui_store::DATABASE_UNREACHABLE` carries, and
/// the proof that the buffer is disabled is negative on both sides — no step was minted, and
/// `<cache_dir>/pending/` is still empty, with neither an `.open` file nor a sealed one.
///
/// `Backend::writer()` answers `None` offline, so `AgentRuntime::start` refuses at the writer,
/// before it ever asks the mirror who this user is. That is why this case, not the D33 one below,
/// is what an offline chat does now.
///
/// Plain asserts and no snapshot: the sentence and the empty directory are the whole contract, and
/// a snapshot would only be one more file for the CLEAN item to delete.
#[tokio::test]
async fn an_offline_chat_is_refused_with_the_unreachable_warning() {
    let agent_id = AgentId::new();
    let (_root, cache) = offline_mirror(agent_id).await;
    let (_keyring, mut harness) = offline_harness(&cache, one_turn()).await;
    harness.drive().await;

    compose(&mut harness, "what is in main.rs");
    harness.drive().await;

    let rendered = harness.render();
    // `contains`, not equality: `StoreError::Unreachable` renders with a `store unreachable: `
    // prefix, so the sentence reaches the screen inside a longer line.
    assert!(
        rendered.contains(htui_store::DATABASE_UNREACHABLE),
        "the body states the refusal: {rendered}"
    );
    assert!(
        harness.chat_steps().is_empty(),
        "and no chat was started: {rendered}"
    );

    cache.close().await;
}

/// The negative of the header case: a chat that records into a store the maintainer can read back
/// says nothing about a buffer, so none of the eight `chat__*` snapshots moves.
#[tokio::test]
async fn an_online_chat_header_says_nothing_about_a_buffer() {
    let store = MemStore::demo();
    let agent_id = AgentId::new();
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        store.upsert_agent(&row).await.expect("the row is disabled");
    }
    store
        .upsert_agent(&scripted_row(agent_id))
        .await
        .expect("the scripted row lands");

    let adapter = Arc::new(FakeAdapter::new());
    adapter.load(one_turn());
    let mut factory = DriverFactory::new();
    factory.register("acp", Box::new(SharedAdapter(Arc::clone(&adapter))));
    let mut harness = Harness::over(store)
        .with_tab(Box::new(ChatTab::new()))
        .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::from_millis(0)))
        .size(140, 30);
    harness.drive().await;
    compose(&mut harness, "what is in main.rs");
    harness.drive().await;

    let rendered = harness.render();
    assert!(
        !rendered.contains("buffered"),
        "a memory chat is not buffered: {rendered}"
    );
    assert_eq!(
        harness.chat_steps().len(),
        1,
        "and it is a chat like any other"
    );
}
