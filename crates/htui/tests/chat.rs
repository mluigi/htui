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
#![cfg(feature = "testkit")]

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
        // `register_all` names the Chat tab as the replay tab (D39); a hand-registered shell has
        // to say so itself, and the replay cases below go through the same `Action::Replay` the
        // Runs pane emits rather than reaching into the tab.
        .with_replay_tab(ChatTab::ID)
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

// -------------------------------------------------------------------------------------------
// The staleness re-probe (MOD-2 milestone 5, D55): a chat on a stale `agent_box` row refreshes
// it in the background and is never blocked, delayed or failed by it.
// -------------------------------------------------------------------------------------------

/// The scripted row's `agent_box`, as the store holds it now.
async fn on_box(store: &MemStore) -> Option<AgentBox> {
    store
        .agents()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|summary| summary.agent.name == "scripted")
        .expect("the scripted row is registered")
        .on_box
}

/// The scripted row's id.
async fn scripted_id(store: &MemStore) -> AgentId {
    store
        .agents()
        .await
        .expect("the memory store never fails")
        .into_iter()
        .find(|summary| summary.agent.name == "scripted")
        .expect("the scripted row is registered")
        .agent
        .id
}

/// An `agent_box` row for the scripted agent, last probed at `probed_at`.
fn probed(agent_id: AgentId, probed_at: chrono::DateTime<chrono::Utc>) -> AgentBox {
    AgentBox {
        agent_id,
        box_id: htui_core::fixtures::ids::BOX,
        enabled: true,
        version: Some("0.48.0".to_owned()),
        path: None,
        probed_at: Some(probed_at),
        quota: None,
        quota_at: None,
        updated_at: probed_at,
        probe: Some(json!({ "status": "ready", "source": "probe" })),
    }
}

/// D55: the row nobody has probed is refreshed behind the chat, and the probe's **failure** —
/// `command: "unused"` will never spawn — leaves the conversation untouched.
#[tokio::test]
async fn a_chat_on_an_unprobed_row_refreshes_agent_box_in_the_background() {
    let (mut harness, store) = harness(Script::one_turn(vec![chunk("It is empty."), done()])).await;
    assert!(
        on_box(&store).await.is_none(),
        "the fixture probes nothing; this is the row the chat makes stale"
    );

    compose(&mut harness, "what is in main.rs");
    harness.drive_to_end().await;

    let row = on_box(&store)
        .await
        .expect("the background re-probe wrote agent_box");
    assert_eq!(
        row.probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(serde_json::Value::as_str),
        Some("failed"),
        "the scripted launch spawns nothing, and the chat ran anyway: {:?}",
        row.probe
    );
    assert!(!row.enabled, "a launch that will not spawn is not enabled");
    let probed_at = row.probed_at.expect("a probe stamps the row it wrote");
    assert!(
        (chrono::Utc::now() - probed_at) < chrono::TimeDelta::minutes(1),
        "the stamp is this run's, not a fixture's: {probed_at}"
    );

    // And the chat itself is whole: the same log the un-probed run wrote.
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
            EventKind::AssistantText,
            EventKind::Done,
        ],
        "a failed re-probe never fails the chat"
    );
}

/// The TTL, through the whole shell: a row probed a minute ago is left exactly as it is, and the
/// same row aged past 24 hours is rewritten.
#[tokio::test]
async fn a_fresh_row_is_not_re_probed_and_a_stale_one_is() {
    let (mut shell, store) = harness(Script::one_turn(vec![chunk("hi"), done()])).await;
    let agent_id = scripted_id(&store).await;
    let fresh = chrono::Utc::now() - chrono::TimeDelta::minutes(1);
    store
        .upsert_agent_box(&probed(agent_id, fresh))
        .await
        .expect("the fresh row lands");

    compose(&mut shell, "one");
    shell.drive_to_end().await;

    let row = on_box(&store).await.expect("the row is still there");
    assert_eq!(
        row.probed_at,
        Some(fresh),
        "a row inside the TTL is not touched"
    );
    assert!(row.enabled, "and its verdict is not rewritten");

    // The same row, aged past the window.
    let (mut shell, store) = harness(Script::one_turn(vec![chunk("hi"), done()])).await;
    let agent_id = scripted_id(&store).await;
    let stale = chrono::Utc::now() - chrono::TimeDelta::hours(25);
    store
        .upsert_agent_box(&probed(agent_id, stale))
        .await
        .expect("the stale row lands");

    compose(&mut shell, "one");
    shell.drive_to_end().await;

    let row = on_box(&store).await.expect("the row is still there");
    assert!(
        row.probed_at.is_some_and(|at| at > stale),
        "a stale row is re-probed: {:?}",
        row.probed_at
    );
    assert_eq!(
        row.probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(serde_json::Value::as_str),
        Some("failed"),
        "and the fresh verdict replaces the stale one: {:?}",
        row.probe
    );
}

// -------------------------------------------------------------------------------------------
// Replay (MOD-2 milestone 4, T20): the same tab, the same transcript, over persisted rows.
// -------------------------------------------------------------------------------------------

/// [`stable`] plus the step id in the replay header, which is a fresh UUID whenever the step
/// under replay is one a chat in this test just minted.
fn stable_step() -> insta::Settings {
    let mut settings = stable();
    settings.add_filter(r"step …[0-9a-f]{8} ", "step …<step> ");
    settings
}

/// The transcript area of a rendered frame: everything between the tab's header line and its
/// hint line. Four lines of chrome above (top bar, tab strip, block border, header) and three
/// below (hint, block border, status line), all fixed by the layout — which is exactly why the
/// live and the replayed body can be compared line for line.
fn body_lines(render: &str) -> Vec<&str> {
    let lines: Vec<&str> = render.lines().collect();
    lines[4..lines.len() - 3].to_vec()
}

/// A shell over the demo fixture with a Chat tab and nothing running in it.
async fn replay_only() -> Harness {
    let mut harness = Harness::demo()
        .with_tab(Box::new(ChatTab::new()))
        .with_replay_tab(ChatTab::ID);
    harness.settle().await;
    harness
}

/// Opens a step's log in the Chat tab through the action the Runs pane emits (D39).
async fn replay(harness: &mut Harness, step_id: htui_core::model::StepId) {
    harness.app().update(htui::app::Action::Replay { step_id });
    harness.drive().await;
}

/// `R-HIS-2`: a step nobody is running renders through the transcript a live one renders
/// through — the prompt, the tool call and its result folded into one row, the plan, the usage
/// line and the end of the turn.
#[tokio::test]
async fn a_recorded_step_replays_through_the_live_transcript() {
    let mut harness = replay_only().await;
    replay(&mut harness, htui_core::fixtures::ids::STEP_PLAN).await;

    let rendered = harness.render();
    let id = htui_core::fixtures::ids::STEP_PLAN.to_string();
    let tail = &id[id.len() - 8..];
    assert!(
        rendered.contains(&format!("replay · step …{tail} · 7 rows · read-only")),
        "the header names which step is on screen and that it is over: {rendered}"
    );
    insta::assert_snapshot!("replay_fixture_step", rendered);
}

/// D38: a step this box has no rows for is *unknown*, not empty. Rendering it as an empty
/// conversation is the one reading `R-HIS-1` forbids.
#[tokio::test]
async fn a_step_that_is_not_on_this_box_says_so() {
    let mut harness = replay_only().await;
    replay(&mut harness, htui_core::fixtures::ids::STEP_PRD).await;
    insta::assert_snapshot!("replay_missing", harness.render());
}

/// `R-TUI-6`, the property the mode exists for: what a replay draws and what the live session
/// drew are the same lines, because they are the same renderer over the same events. Only the
/// header differs, which is what tells the reader they are looking at history.
#[tokio::test]
async fn a_replayed_turn_renders_the_lines_the_live_one_did() {
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
    let (mut harness, _) = harness(script).await;
    compose(&mut harness, "what is in main.rs");
    harness.drive().await;
    let live = harness.render();

    let step = harness.chat_steps()[0];
    replay(&mut harness, step).await;
    let replayed = harness.render();
    assert_eq!(
        body_lines(&replayed),
        body_lines(&live),
        "the replayed body is the live body; only the header moved"
    );
    stable_step().bind(|| insta::assert_snapshot!("replay_streamed_turn", replayed));

    // D40: leaving puts the live view back exactly as it was — replay never wrote to it.
    harness.key("esc");
    harness.drive().await;
    assert_eq!(harness.render(), live);
}

/// D40 through the shell: every key the live tab acts on is swallowed, so a finished step
/// cannot be prompted, answered or cancelled. The unit test beside `ChatTab` proves the tab
/// emits nothing; this one proves the screen agrees.
#[tokio::test]
async fn replay_takes_no_prompt_and_no_answer() {
    let (mut harness, _) = harness(Script::one_turn(vec![chunk("hello"), done()])).await;
    compose(&mut harness, "hi");
    harness.drive().await;
    let step = harness.chat_steps()[0];
    replay(&mut harness, step).await;
    let opened = harness.render();

    for pressed in ["i", "enter", "1", "9", "a"] {
        harness.key(pressed);
    }
    harness.drive().await;
    assert_eq!(
        harness.render(),
        opened,
        "no composer opened, no turn was sent, nothing moved"
    );
    assert_eq!(
        harness.chat_steps().len(),
        1,
        "and no second chat was started"
    );
}

/// D41: a request and its answer replay resolved, through the very same `resolve_permission`
/// the live path uses — including the answer the user typed and who typed it.
#[tokio::test]
async fn an_answered_permission_replays_resolved() {
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
    let (mut harness, _) = harness(script).await;
    compose(&mut harness, "clean the build");
    harness.drive().await;
    harness.key("2");
    harness.drive().await;

    let step = harness.chat_steps()[0];
    replay(&mut harness, step).await;
    let rendered = harness.render();
    assert!(
        rendered.contains("permission  reject (by user)"),
        "the log says who answered and with what: {rendered}"
    );
    stable_step().bind(|| insta::assert_snapshot!("replay_answered_permission", rendered));
}

/// D41's other half: a request nobody ever answered — a session that died with it outstanding —
/// is history, so it renders parked-looking and is **not** answerable. No strip is offered, and
/// the digit that would have answered it live does nothing.
#[tokio::test]
async fn an_unanswered_permission_replays_parked_but_answers_nothing() {
    use chrono::TimeZone as _;
    use htui_core::fixtures::ids;
    use htui_core::model::{EventRole, SessionEvent};

    let store = MemStore::demo();
    let row = |seq: i32, kind: EventKind, payload: serde_json::Value| SessionEvent {
        // A fixture step with no log of its own, so these rows are the whole conversation.
        run_step_id: ids::STEP_PRD,
        seq,
        turn: 0,
        kind,
        role: EventRole::Agent,
        tool_call_id: None,
        payload,
        raw: None,
        at: chrono::Utc
            .timestamp_opt(0, 0)
            .single()
            .expect("epoch is a time"),
    };
    store
        .append_events(&[
            row(0, EventKind::Prompt, json!({ "text": "clean the build" })),
            row(
                1,
                EventKind::PermissionRequest,
                json!({ "request_id": "req-1", "tool_call_id": "call-1",
                        "options": [{ "id": "allow", "label": "Allow once",
                                      "kind": "allow_once" }] }),
            ),
        ])
        .await
        .expect("the rows land on the fixture's step");

    let mut harness = Harness::over(store)
        .with_tab(Box::new(ChatTab::new()))
        .with_replay_tab(ChatTab::ID);
    harness.settle().await;
    replay(&mut harness, ids::STEP_PRD).await;
    let opened = harness.render();
    assert!(
        opened.contains("permission  waiting on you"),
        "the row says the agent asked and nobody answered: {opened}"
    );
    insta::assert_snapshot!("replay_parked_permission", opened);

    harness.key("1");
    harness.drive().await;
    assert_eq!(
        harness.render(),
        opened,
        "a finished step answers nothing, however parked its last row looks"
    );
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
