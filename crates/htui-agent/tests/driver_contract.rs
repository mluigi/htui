//! The Task 1 contract of `docs/ANA-4.md` §4.1 (plan MOD-2 D2, D16).
//!
//! Four properties, none of which any later task may quietly drop:
//!
//! 1. `AgentDriver` and `AgentSession` are dyn-compatible, and a session driven from a spawned
//!    task is `Send` — the property MOD-4 needs to put one task on the runtime per session.
//! 2. `From<&DriverEvent> for EventKind` is **total** over the eleven `EventKind` values a driver
//!    can produce: the fourteen of `crates/htui-core/src/model/event.rs` minus the three `htui`
//!    authors itself (`prompt`, `follow_up`, `permission_answer`).
//! 3. `SessionSpec`'s hand-written `Debug` prints every environment value as `[REDACTED]`
//!    (ANA-4 §4.1's mechanical enforcement of invariant 4).
//! 4. `DriverCaps` is `Copy` and defaults to all-false, so a transport that forgets to answer a
//!    predicate advertises nothing rather than everything.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use chrono::Utc;
use htui_agent::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, McpServerSpec, PermissionAnswer,
    PermissionDefault, PermissionPolicy, PermissionRequestId, SessionSpec, ToolExposure,
};
use htui_agent::error::DriverError;
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionOption, PermissionOptionKind, PermissionRequestEvent, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, PlanEvent, StopReason, TerminalReason, TextChunk, ToolCallEvent, ToolKind,
    ToolLocation, ToolResultEvent, ToolResultStatus, UsageEvent,
};
use htui_core::model::{AgentId, EventKind, StepId};
use serde_json::json;

// ---------------------------------------------------------------------------------------------
// A stub transport. It exists only to be reached through `&dyn` / `Box<dyn>`; nothing about its
// behaviour is under test beyond "the trait pair can be implemented and spawned".
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
struct StubDriver;

impl AgentDriver for StubDriver {
    fn name(&self) -> &str {
        "stub"
    }

    fn caps(&self) -> DriverCaps {
        DriverCaps::default()
    }

    fn start<'a>(
        &'a self,
        _spec: SessionSpec,
        _prompt: String,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AgentSession>, DriverError>> + Send + 'a>> {
        Box::pin(async {
            let session = StubSession {
                left: 2,
                session_ref: AgentSessionRef("stub-1".to_owned()),
            };
            Ok(Box::new(session) as Box<dyn AgentSession>)
        })
    }
}

#[derive(Debug)]
struct StubSession {
    left: usize,
    session_ref: AgentSessionRef,
}

impl AgentSession for StubSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        Some(&self.session_ref)
    }

    fn next_event<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DriverEnvelope>, DriverError>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.left == 0 {
                return Ok(None);
            }
            self.left -= 1;
            Ok(Some(DriverEnvelope {
                event: DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
                raw: None,
                at: Utc::now(),
            }))
        })
    }

    fn send_follow_up<'a>(
        &'a mut self,
        _text: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn answer_permission<'a>(
        &'a mut self,
        _request_id: PermissionRequestId,
        _answer: PermissionAnswer,
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn cancel<'a>(
        &'a mut self,
        _grace: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

/// The dyn-compatibility assertion itself: this signature does not compile if either trait grows
/// an `async fn`, a generic method or a `Self: Sized` return.
fn takes_dyn(_driver: &dyn AgentDriver, _session: &mut dyn AgentSession) {}

fn spec() -> SessionSpec {
    SessionSpec {
        agent_id: AgentId::new(),
        step_id: StepId::new(),
        cwd: PathBuf::from("/work"),
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

// ---------------------------------------------------------------------------------------------
// 1. dyn-compatibility and `Send`
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn trait_pair_is_dyn_compatible_and_a_session_is_send() {
    let driver: Box<dyn AgentDriver> = Box::new(StubDriver);
    assert_eq!(driver.name(), "stub");

    let mut session = driver
        .start(spec(), "hello".to_owned())
        .await
        .expect("the stub starts");
    assert_eq!(
        session.session_ref().map(AgentSessionRef::as_str),
        Some("stub-1")
    );

    // `&dyn` on both traits.
    takes_dyn(driver.as_ref(), session.as_mut());

    // `Box<dyn AgentSession>` driven from a spawned task: this only compiles because every
    // returned future is `+ Send` and `AgentSession: Send`.
    let handle = tokio::spawn(async move {
        let mut seen = 0_usize;
        while let Some(envelope) = session.next_event().await.expect("the stub never fails") {
            assert!(envelope.raw.is_none());
            seen += 1;
        }
        seen
    });

    assert_eq!(handle.await.expect("the session task joins"), 2);
}

// ---------------------------------------------------------------------------------------------
// 2. `From<&DriverEvent> for EventKind` is total
// ---------------------------------------------------------------------------------------------

/// One value per `DriverEvent` variant, in the §4.1 declaration order. A twelfth variant added
/// without a line here fails to compile: the `match` below is exhaustive.
fn every_driver_event() -> Vec<DriverEvent> {
    let all = vec![
        DriverEvent::AssistantChunk(TextChunk {
            text: "hi".to_owned(),
            message_id: Some("m1".to_owned()),
        }),
        DriverEvent::ThoughtChunk(TextChunk {
            text: "hmm".to_owned(),
            message_id: None,
        }),
        DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call-1".to_owned(),
            title: "Read src/lib.rs".to_owned(),
            tool_kind: ToolKind::Read,
            input: json!({ "path": "src/lib.rs" }),
            locations: vec![ToolLocation {
                path: "src/lib.rs".to_owned(),
                line: Some(12),
            }],
        }),
        DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call-1".to_owned(),
            status: ToolResultStatus::Failed,
            output: None,
            locations: Vec::new(),
            terminal_reason: Some(TerminalReason::Rejected),
        }),
        DriverEvent::EditProposal(EditProposalEvent {
            tool_call_id: Some("call-2".to_owned()),
            path: "src/lib.rs".to_owned(),
            diff: "@@\n-a\n+b\n".to_owned(),
            accepted: None,
        }),
        DriverEvent::PermissionRequest(PermissionRequestEvent {
            request_id: PermissionRequestId("req-1".to_owned()),
            tool_call_id: Some("call-2".to_owned()),
            options: vec![PermissionOption {
                id: "opt-1".to_owned(),
                label: "Allow once".to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            }],
        }),
        DriverEvent::Plan(PlanEvent {
            entries: vec![PlanEntry {
                content: "write the test".to_owned(),
                status: PlanEntryStatus::InProgress,
                priority: PlanEntryPriority::High,
            }],
        }),
        DriverEvent::Usage(UsageEvent::default()),
        DriverEvent::Error(ErrorEvent {
            code: "refusal".to_owned(),
            message: "no".to_owned(),
        }),
        DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        }),
        DriverEvent::Other(OtherEvent {
            update: "session_info_update".to_owned(),
            body: json!({ "sessionId": "s1" }),
        }),
    ];

    // Exhaustiveness: a new variant breaks this match, not just the length assertion.
    for event in &all {
        match event {
            DriverEvent::AssistantChunk(_)
            | DriverEvent::ThoughtChunk(_)
            | DriverEvent::ToolCall(_)
            | DriverEvent::ToolResult(_)
            | DriverEvent::EditProposal(_)
            | DriverEvent::PermissionRequest(_)
            | DriverEvent::Plan(_)
            | DriverEvent::Usage(_)
            | DriverEvent::Error(_)
            | DriverEvent::Done(_)
            | DriverEvent::Other(_) => {}
        }
    }

    all
}

/// The three kinds `htui` writes itself; no driver may ever produce one (ANA-4 §6).
const HTUI_AUTHORED: [EventKind; 3] = [
    EventKind::Prompt,
    EventKind::FollowUp,
    EventKind::PermissionAnswer,
];

#[test]
fn driver_events_reach_every_event_kind_htui_does_not_author() {
    let events = every_driver_event();
    assert_eq!(
        events.len(),
        11,
        "ANA-4 §4.1 fixes eleven DriverEvent variants"
    );
    assert_eq!(
        EventKind::ALL.len(),
        14,
        "ANA-9 §4.3 fixes fourteen EventKind values"
    );

    let mut reached: Vec<EventKind> = Vec::new();
    for event in &events {
        let kind = EventKind::from(event);
        assert!(
            !HTUI_AUTHORED.contains(&kind),
            "{kind} is authored by htui, never mapped from a DriverEvent"
        );
        assert!(
            !reached.contains(&kind),
            "two DriverEvent variants map to {kind}"
        );
        reached.push(kind);
    }

    let expected: Vec<EventKind> = EventKind::ALL
        .iter()
        .copied()
        .filter(|kind| !HTUI_AUTHORED.contains(kind))
        .collect();
    assert_eq!(expected.len(), 11, "14 - 3 = 11");
    for kind in &expected {
        assert!(reached.contains(kind), "no DriverEvent maps to {kind}");
    }
    assert_eq!(reached.len(), expected.len(), "the map is a bijection");
}

// ---------------------------------------------------------------------------------------------
// 3. redacted `Debug`
// ---------------------------------------------------------------------------------------------

#[test]
fn session_spec_debug_redacts_every_environment_value() {
    let mut spec = spec();
    spec.env.insert("TOKEN".to_owned(), "s3cr3t".to_owned());
    spec.env
        .insert("OTHER".to_owned(), "also-s3cr3t".to_owned());
    spec.mcp.push(McpServerSpec {
        name: "htui".to_owned(),
        command: "htui-mcp".to_owned(),
        args: vec!["--stdio".to_owned()],
        env: BTreeMap::from([("MCP_TOKEN".to_owned(), "s3cr3t".to_owned())]),
    });

    let shown = format!("{spec:?}");

    assert!(shown.contains("TOKEN"), "keys stay visible: {shown}");
    assert!(shown.contains("[REDACTED]"), "values are masked: {shown}");
    assert!(
        !shown.contains("s3cr3t"),
        "no environment value survives Debug: {shown}"
    );
}

// ---------------------------------------------------------------------------------------------
// 4. `DriverCaps`
// ---------------------------------------------------------------------------------------------

#[test]
fn driver_caps_is_copy_and_defaults_to_all_false() {
    let caps = DriverCaps::default();
    let copied = caps; // `Copy`: `caps` is still usable below.
    assert_eq!(caps, copied);

    assert!(!caps.permission_requests);
    assert!(!caps.edit_proposals);
    assert!(!caps.plans);
    assert!(!caps.thoughts);
    assert!(!caps.follow_up_in_session);
    assert!(!caps.resume);
    assert!(!caps.usage);
}

// ---------------------------------------------------------------------------------------------
// The JSONB vocabularies: `as_str` and serde must never disagree (htui-core `str_enum!` rule).
// ---------------------------------------------------------------------------------------------

fn check_wire_enum<T>(all: &[T], texts: &[&str])
where
    T: Copy + core::fmt::Debug + PartialEq + core::fmt::Display + serde::Serialize,
{
    assert_eq!(all.len(), texts.len(), "variant count differs from ANA-4");
    for (variant, text) in all.iter().zip(texts) {
        assert_eq!(&variant.to_string(), text, "as_str / ANA-4 order mismatch");
        let json = serde_json::to_string(variant).expect("serialize");
        assert_eq!(json, format!("\"{text}\""), "serde rename follows as_str");
    }
}

#[test]
fn wire_enums_match_their_ana4_vocabularies() {
    check_wire_enum(
        ToolKind::ALL,
        &[
            "read",
            "edit",
            "delete",
            "move",
            "search",
            "execute",
            "think",
            "fetch",
            "switch_mode",
            "other",
        ],
    );
    check_wire_enum(ToolResultStatus::ALL, &["completed", "failed"]);
    check_wire_enum(TerminalReason::ALL, &["rejected", "cancelled"]);
    check_wire_enum(
        StopReason::ALL,
        &[
            "end_turn",
            "max_tokens",
            "max_turn_requests",
            "refusal",
            "cancelled",
        ],
    );
    check_wire_enum(
        PermissionOptionKind::ALL,
        &["allow_once", "allow_always", "reject_once", "reject_always"],
    );
    check_wire_enum(
        PlanEntryStatus::ALL,
        &["pending", "in_progress", "completed"],
    );
    check_wire_enum(PlanEntryPriority::ALL, &["high", "medium", "low"]);
    check_wire_enum(PermissionDefault::ALL, &["ask", "allow", "deny"]);

    assert_eq!(ToolKind::default(), ToolKind::Other, "ANA-4 §3");
}

#[test]
fn permission_policy_defaults_to_ask_and_round_trips() {
    let empty: PermissionPolicy = serde_json::from_str("{}").expect("every key is optional");
    assert_eq!(empty, PermissionPolicy::default());
    assert_eq!(empty.default, PermissionDefault::Ask, "§5.2 default = ask");
    assert!(empty.rules.is_empty());
    assert!(empty.remembered.is_empty());

    let seeded = json!({ "default": "ask", "rules": [], "remembered": [] });
    let parsed: PermissionPolicy = serde_json::from_value(seeded).expect("the seed row parses");
    assert_eq!(parsed, PermissionPolicy::default());
}
