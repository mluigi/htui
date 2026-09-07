//! The Chat tab, driven end to end through the chat seam (`R-TUI-6`, MOD-2 milestone 3).
//!
//! Every test here runs a **real** chat: the tab issues `ChatStart`, the harness serves it through
//! an `AgentRuntime`, a scripted driver plays a turn, the recorder writes the rows and the frames
//! come back through the reply channel. Nothing is faked at the tab's boundary, which is why the
//! store assertions below are meaningful — the same rows a live `claude` session would write are
//! the ones these produce.
//!
//! The transport is milestone 2's fake, reached through the same registry the ACP one uses, so no
//! snapshot here depends on a process being installed.

use std::sync::Arc;
use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::ChatTab;
use htui_agent::conformance::{Script, ScriptEvent};
use htui_agent::driver::{DriverCaps, PermissionRequestId};
use htui_agent::event::{
    DoneEvent, DriverEvent, EditProposalEvent, PermissionOption, PermissionOptionKind,
    PermissionRequestEvent, StopReason, TextChunk, ToolCallEvent, ToolKind, ToolResultEvent,
    ToolResultStatus,
};
use htui_agent::fake::FakeAdapter;
use htui_agent::registry::DriverFactory;
use htui_core::model::{Agent, AgentBox, AgentId, Billing, EventKind, Transport};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
use serde_json::json;

/// The fake mints a fresh session id per session, so a snapshot that showed it would differ on
/// every run. The id itself is asserted where it matters (the store's `session_started` row);
/// here it is substituted so the *layout* is what the snapshot pins.
fn stable() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(r"fake-[0-9a-f-]{36}", "fake-<session>");
    settings
}

/// A registry row the factory reaches by **row data**: `cli` with stream `fake` (plan D12).
fn scripted_row(id: AgentId, caps_via_transport: Transport) -> Agent {
    Agent {
        id,
        name: "scripted".to_owned(),
        transport: caps_via_transport,
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

/// A harness whose Chat tab talks to a scripted transport, and the store behind it.
///
/// The row is `acp`, so `caps_for` gives it the full profile and the tab renders no capability
/// banner: these tests are about what a *capable* session looks like. The degraded profile has its
/// own test below, over a `cli` row.
async fn harness(script: Script) -> (Harness, MemStore) {
    harness_with(script, Transport::Acp).await
}

/// [`harness`] over a chosen transport, so a test can ask for the degraded capability profile.
async fn harness_with(script: Script, transport: Transport) -> (Harness, MemStore) {
    let store = MemStore::demo();
    // The fixture's own agents are `acp` rows this build has no adapter for; disabling them leaves
    // the tab exactly one agent to talk to, which is what makes the header deterministic and the
    // choice deliberate rather than alphabetical.
    for summary in store.agents().await.expect("the fixture's agents") {
        let mut row = summary.agent;
        row.enabled = false;
        store.upsert_agent(&row).await.expect("the row is disabled");
    }
    store
        .upsert_agent(&scripted_row(AgentId::new(), transport))
        .await
        .expect("the scripted row lands");

    let adapter = Arc::new(FakeAdapter::new());
    adapter.load(script);
    let mut factory = DriverFactory::new();
    // The fake stands in for whichever transport the row names, and the factory reaches it by row
    // data alone — `acp`, or `cli` plus `settings.cli.stream` — with no entry for the agent's
    // *name*, which is the `R-AGT-5` shape.
    factory.register("acp", Box::new(SharedAdapter(Arc::clone(&adapter))));
    factory.register("cli/fake", Box::new(SharedAdapter(Arc::clone(&adapter))));

    let mut harness = Harness::over(store.clone())
        .with_tab(Box::new(ChatTab::new()))
        .with_agent_runtime(AgentRuntime::new(factory).with_grace(Duration::from_millis(0)));
    harness.drive().await;
    (harness, store)
}

/// Types `text` into the composer and submits it.
///
/// A space is `"space"`, because `KeyChord::parse` trims its input and a bare `" "` is not a
/// chord — the composer sees the same `KeyCode::Char(' ')` either way.
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

fn chunk(text: &str) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
        text: text.to_owned(),
        message_id: Some("m1".to_owned()),
    }))
}

fn done() -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
        stop_reason: StopReason::EndTurn,
    }))
}

#[tokio::test]
async fn an_unstarted_chat_names_its_agent_and_asks_for_a_prompt() {
    let (mut harness, _) = harness(Script::default()).await;
    stable().bind(|| insta::assert_snapshot!("chat_empty", harness.render()));
}

#[tokio::test]
async fn a_streamed_turn_renders_text_a_thought_and_a_folded_tool_call() {
    let script = Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::ThoughtChunk(TextChunk {
            text: "I should read the file first".to_owned(),
            message_id: Some("t1".to_owned()),
        })),
        chunk("Reading it now. "),
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
        chunk("It is empty."),
        done(),
    ]);
    let (mut harness, store) = harness(script).await;

    compose(&mut harness, "what is in main.rs");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_streamed_turn", harness.render()));

    // The rows behind the screen: the same log a live session writes.
    let step = harness.chat_steps()[0];
    let kinds: Vec<EventKind> = store
        .step_events(step)
        .await
        .expect("the log reads")
        .expect("the chat step has a log")
        .iter()
        .map(|row| row.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::Prompt,
            EventKind::Other,
            EventKind::Thought,
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::ToolResult,
            EventKind::AssistantText,
            EventKind::Done,
        ],
        "the prompt, the banner, and then what the agent did"
    );
}

#[tokio::test]
async fn an_edit_proposal_renders_as_a_diff() {
    let script = Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::EditProposal(EditProposalEvent {
            tool_call_id: Some("call-1".to_owned()),
            path: "src/lib.rs".to_owned(),
            diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old line\n+new line\n"
                .to_owned(),
            accepted: Some(true),
        })),
        done(),
    ]);
    let (mut harness, _) = harness(script).await;
    compose(&mut harness, "fix it");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_edit_proposal", harness.render()));
}

#[tokio::test]
async fn a_parked_permission_is_answered_with_a_digit_and_recorded_as_the_users() {
    let script = Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "rm -rf build".to_owned(),
            tool_kind: ToolKind::Execute,
            input: json!({ "command": "rm -rf build" }),
            locations: Vec::new(),
        })),
        ScriptEvent::ParkPermission(PermissionRequestEvent {
            request_id: PermissionRequestId::new("req-1"),
            tool_call_id: Some("call-1".to_owned()),
            options: vec![
                PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow once".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                },
                PermissionOption {
                    id: "reject".to_owned(),
                    label: "Reject".to_owned(),
                    kind: PermissionOptionKind::RejectOnce,
                },
            ],
        }),
        done(),
    ]);
    let (mut harness, store) = harness(script).await;
    compose(&mut harness, "clean the build");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_permission_inline", harness.render()));

    // `2` is the reject option, and the tab consumes the digit rather than letting the global
    // `1`..`9` tab bindings see it.
    harness.key("2");
    harness.drive().await;
    assert_eq!(
        harness.app().tabs.active_id().map(|id| id.0),
        Some("chat"),
        "a digit answering a permission request must not switch tabs"
    );
    stable().bind(|| insta::assert_snapshot!("chat_permission_answered", harness.render()));

    let step = harness.chat_steps()[0];
    let log = store
        .step_events(step)
        .await
        .expect("the log reads")
        .expect("a log");
    let answer = log
        .iter()
        .find(|row| row.kind == EventKind::PermissionAnswer)
        .expect("the answer is recorded");
    assert_eq!(
        answer.payload.get("by").and_then(serde_json::Value::as_str),
        Some("user"),
        "a human typed it, so ANA-9 §4.3's `by` says user, not policy"
    );
    assert_eq!(
        answer
            .payload
            .get("option_id")
            .and_then(serde_json::Value::as_str),
        Some("reject")
    );
    assert!(
        log.iter().any(|row| row.kind == EventKind::ToolResult
            && row
                .payload
                .get("terminal_reason")
                .and_then(serde_json::Value::as_str)
                == Some("rejected")),
        "a rejected call still gets a result, or a replay would spin forever (§4.3)"
    );
}

#[tokio::test]
async fn a_follow_up_opens_a_second_turn() {
    let script = Script::turns(vec![
        vec![chunk("first answer"), done()],
        vec![chunk("second answer"), done()],
    ]);
    let (mut harness, store) = harness(script).await;
    compose(&mut harness, "one");
    harness.drive().await;
    compose(&mut harness, "two");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_follow_up", harness.render()));

    let step = harness.chat_steps()[0];
    let log = store
        .step_events(step)
        .await
        .expect("the log reads")
        .expect("a log");
    let turns: Vec<i32> = log.iter().map(|row| row.turn).collect();
    assert_eq!(
        turns.iter().max(),
        Some(&1),
        "the follow-up opens turn 1 (`docs/ANA-4.md` §4.1)"
    );
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::FollowUp)
            .count(),
        1
    );
}

#[tokio::test]
async fn esc_esc_ends_the_chat_and_closes_its_run() {
    let (mut harness, store) =
        harness(Script::one_turn(vec![chunk("done thinking"), done()])).await;
    compose(&mut harness, "hello");
    harness.drive().await;

    // One `Esc` arms, the second ends: an `Esc` that only leaves the composer cannot end a chat.
    harness.key("esc");
    harness.key("esc");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_ended", harness.render()));

    let scope = htui_core::model::Scope {
        workspace_id: htui_core::fixtures::ids::WORKSPACE_PLATFORM,
        project_ids: vec![htui_core::fixtures::ids::PROJECT_HTUI],
    };
    assert_eq!(
        store.active_runs(&scope).await.expect("count"),
        1,
        "the chat's run is closed; only the fixture's own queued run is still active"
    );
}

/// `DriverCaps` is authoritative, not `agent.transport`: the tab names what this session cannot
/// do without knowing what a CLI session is (`docs/ANA-4.md` §4.3).
#[tokio::test]
async fn a_degraded_transport_banners_what_it_cannot_do() {
    let (mut harness, _) = harness_with(
        Script::one_turn(vec![chunk("no tools here"), done()]),
        Transport::Cli,
    )
    .await;
    compose(&mut harness, "hello");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_capability_banner", harness.render()));
}

#[tokio::test]
async fn a_harness_without_a_runtime_renders_the_refusal() {
    let store = MemStore::demo();
    store
        .upsert_agent(&scripted_row(AgentId::new(), Transport::Cli))
        .await
        .expect("the row lands");
    let mut harness = Harness::over(store).with_tab(Box::new(ChatTab::new()));
    harness.drive().await;
    compose(&mut harness, "anyone there");
    harness.drive().await;
    stable().bind(|| insta::assert_snapshot!("chat_refused", harness.render()));
}
