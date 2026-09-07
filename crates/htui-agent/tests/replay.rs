//! Replay decode tests (plan MOD-2 T18, design row D37).
//!
//! The unit under test is [`htui_agent::replay`], the inverse of the recorder. Every case here is
//! stated over **persisted rows**: a script goes through a [`Recorder`] into a `MemStore`, the
//! rows come back through [`ReadStore::step_events`], and the decoder is asked to turn them into
//! the envelopes the live chat tab already renders. Asserting against a hand-built row set instead
//! would prove the decoder agrees with this file, not that it agrees with `record.rs` - and the
//! thing that will actually break is an encoder change nobody mirrored here.

use chrono::{DateTime, Utc};
use htui_agent::acp::SESSION_STARTED;
use htui_agent::acp::map::Mapper;
use htui_agent::driver::PermissionRequestId;
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, ErrorEvent, OtherEvent,
    PermissionOption, PermissionOptionKind, PermissionRequestEvent, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, PlanEvent, StopReason, TextChunk, ToolCallEvent, ToolKind, ToolLocation,
    ToolResultEvent, ToolResultStatus, UsageEvent,
};
use htui_agent::record::{AnsweredBy, Recorder};
use htui_agent::replay::{ReplayError, envelope_from_row, envelope_or_other, envelopes};
use htui_core::fixtures::ids;
use htui_core::model::{ChatRunSpec, EventKind, EventRole, SessionEvent, StepId};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{MemStore, ReadStore, WriteStore};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A fixed capture time, so a decoded envelope is a function of the script and of nothing else.
fn at() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the demo epoch is a valid instant")
}

/// An envelope with no `raw`.
fn env(event: DriverEvent) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: None,
        at: at(),
    }
}

/// The scrubber every case uses: nothing to mask, so a row is the script's own JSON.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new(Vec::<String>::new())
}

/// A fresh chat step in a fresh demo store.
async fn open_step() -> (MemStore, StepId) {
    let store = MemStore::demo();
    let spec = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    );
    store
        .start_chat_run(&spec)
        .await
        .expect("the chat run and step must mint");
    (store, spec.step_id)
}

/// The persisted log of a step, in `seq` order.
async fn rows(store: &MemStore, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("reading the log must not fail")
        .expect("the chat step has a log")
}

/// The eleven driver events the script records, in the order a real turn produces them, with the
/// `permission_answer` slot marked by `None` so the caller can interleave the `htui`-authored row.
fn driver_script() -> Vec<Option<DriverEnvelope>> {
    vec![
        Some(env(DriverEvent::Other(OtherEvent {
            update: SESSION_STARTED.to_owned(),
            body: json!({ "session_id": "sess_1", "agent": "claude", "version": "0.48.0" }),
        }))),
        Some(env(DriverEvent::AssistantChunk(TextChunk {
            text: "Reading the seam.".to_owned(),
            message_id: Some("msg_1".to_owned()),
        }))),
        Some(env(DriverEvent::ThoughtChunk(TextChunk {
            text: "The store trait is the only seam here.".to_owned(),
            message_id: Some("msg_1".to_owned()),
        }))),
        Some(env(DriverEvent::ToolCall(ToolCallEvent {
            tool_call_id: "call_1".to_owned(),
            title: "Read docs/ANA-9.md".to_owned(),
            tool_kind: ToolKind::Read,
            input: json!({ "path": "docs/ANA-9.md", "offset": 824 }),
            locations: vec![ToolLocation {
                path: "docs/ANA-9.md".to_owned(),
                line: Some(824),
            }],
        }))),
        Some(env(DriverEvent::PermissionRequest(
            PermissionRequestEvent {
                request_id: PermissionRequestId::new("req_1"),
                tool_call_id: Some("call_1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow once".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            },
        ))),
        // The `permission_answer` the user gives here; `record_permission_answer`, not `record`.
        None,
        Some(env(DriverEvent::EditProposal(EditProposalEvent {
            tool_call_id: Some("call_1".to_owned()),
            path: "crates/htui-agent/src/replay.rs".to_owned(),
            diff: "@@ -0,0 +1 @@\n+//! the inverse\n".to_owned(),
            accepted: Some(true),
        }))),
        Some(env(DriverEvent::ToolResult(ToolResultEvent {
            tool_call_id: "call_1".to_owned(),
            status: ToolResultStatus::Completed,
            output: Some(json!("pub trait ReadStore: Send + Sync { ... }")),
            locations: Vec::new(),
            terminal_reason: None,
        }))),
        Some(DriverEnvelope {
            event: DriverEvent::Plan(PlanEvent {
                entries: vec![PlanEntry {
                    content: "Write the decoder".to_owned(),
                    status: PlanEntryStatus::InProgress,
                    priority: PlanEntryPriority::High,
                }],
            }),
            // One row carries a verbatim wire message, so the decoder's `raw` is exercised.
            raw: Some(json!({ "jsonrpc": "2.0", "method": "session/update" })),
            at: at(),
        }),
        Some(env(DriverEvent::Usage(UsageEvent {
            input_tokens: Some(12_000),
            output_tokens: Some(2_400),
            ..UsageEvent::default()
        }))),
        Some(env(DriverEvent::Error(ErrorEvent {
            code: "refusal".to_owned(),
            message: "the agent declined".to_owned(),
        }))),
        Some(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        }))),
    ]
}

/// Records one row of every [`EventKind`]: the prompt, the eleven driver events (the
/// `session_started` banner among them) and the two other `htui`-authored rows.
async fn record_every_kind(store: &MemStore, step: StepId) {
    let scrubber = scrubber();
    let mut recorder = Recorder::new(store, &scrubber, step, true, None);
    recorder
        .record_prompt(
            "Plan the replay decoder.",
            json!([{ "name": "prd", "tokens": 800, "trimmed": false }]),
            at(),
        )
        .await
        .expect("the prompt must record");
    for slot in driver_script() {
        match slot {
            Some(envelope) => recorder.record(envelope).await.expect("a row must record"),
            None => recorder
                .record_permission_answer(
                    &PermissionRequestId::new("req_1"),
                    Some("allow"),
                    AnsweredBy::User,
                    false,
                    at(),
                )
                .await
                .expect("the answer must record"),
        }
    }
    recorder
        .record_follow_up("Now the tests.", at())
        .await
        .expect("the follow-up must record");
    recorder.finish().await.expect("nothing to fail on");
}

/// A hand-built row, for the cases about payloads no recorder wrote.
fn row(seq: i32, kind: EventKind, payload: Value) -> SessionEvent {
    SessionEvent {
        run_step_id: ids::STEP_PLAN,
        seq,
        turn: 0,
        kind,
        role: EventRole::Agent,
        tool_call_id: None,
        payload,
        raw: None,
        at: at(),
    }
}

/// The `update` of an envelope that decoded to `other`, or a panic naming what it decoded to.
fn other_of(envelope: &DriverEnvelope) -> &OtherEvent {
    match &envelope.event {
        DriverEvent::Other(other) => other,
        event => panic!("expected `other`, got {event:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The decode table (blueprint E)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_recorded_kind_decodes_to_the_event_that_produced_it() {
    let (store, step) = open_step().await;
    record_every_kind(&store, step).await;
    let rows = rows(&store, step).await;

    let kinds: Vec<EventKind> = rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds.len(),
        EventKind::ALL.len(),
        "the script must record one row of every kind: {kinds:?}"
    );
    for kind in EventKind::ALL {
        assert!(kinds.contains(kind), "the script never recorded {kind}");
    }

    for row in &rows {
        let decoded = envelope_from_row(row).unwrap_or_else(|error| {
            panic!("a recorded `{}` row must decode: {error}", row.kind);
        });
        assert_eq!(decoded.at, row.at, "`at` is the row's, seq {}", row.seq);
        assert_eq!(decoded.raw, row.raw, "`raw` is the row's, seq {}", row.seq);

        match row.kind {
            // The three kinds `htui` authors itself have no `DriverEvent` variant; they reach the
            // live transcript as `other` carrying the payload, and replay must be that shape.
            EventKind::Prompt | EventKind::FollowUp | EventKind::PermissionAnswer => {
                let other = other_of(&decoded);
                assert_eq!(other.update, row.kind.as_str());
                assert_eq!(other.body, row.payload);
            }
            _ => assert_eq!(
                EventKind::from(&decoded.event),
                row.kind,
                "seq {} decoded to the wrong variant",
                row.seq
            ),
        }
    }
}

#[tokio::test]
async fn the_session_banner_decodes_to_the_other_the_header_reads() {
    let (store, step) = open_step().await;
    record_every_kind(&store, step).await;
    let rows = rows(&store, step).await;
    let banner = rows
        .iter()
        .find(|row| row.kind == EventKind::Other)
        .expect("the script records the banner");

    let decoded = envelope_from_row(banner).expect("the banner decodes");
    let other = other_of(&decoded);
    assert_eq!(other.update, SESSION_STARTED);
    assert_eq!(other.body["session_id"], json!("sess_1"));
}

#[tokio::test]
async fn adjacent_text_rows_decode_one_row_at_a_time() {
    let (store, step) = open_step().await;
    let scrubber = scrubber();
    let mut recorder = Recorder::new(&store, &scrubber, step, false, None);
    // Two message groups: the recorder writes two rows, and the transcript must reopen two rows
    // rather than gluing them, which is what the `seq:N` stamp buys.
    for (text, group) in [("first.", "msg_1"), ("second.", "msg_2")] {
        recorder
            .record(env(DriverEvent::AssistantChunk(TextChunk {
                text: text.to_owned(),
                message_id: Some(group.to_owned()),
            })))
            .await
            .expect("a chunk must record");
    }
    recorder.finish().await.expect("nothing to fail on");

    let rows = rows(&store, step).await;
    assert_eq!(rows.len(), 2, "one row per group");
    let ids: Vec<Option<String>> = envelopes(&rows)
        .iter()
        .map(|envelope| match &envelope.event {
            DriverEvent::AssistantChunk(chunk) => chunk.message_id.clone(),
            event => panic!("expected a chunk, got {event:?}"),
        })
        .collect();
    assert_eq!(
        ids,
        vec![Some("seq:0".to_owned()), Some("seq:1".to_owned())],
        "each persisted row gets its own grouping key"
    );
}

// ---------------------------------------------------------------------------------------------
// The regression net
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn rows_re_recorded_from_their_envelopes_are_the_same_rows() {
    let (store, step) = open_step().await;
    record_every_kind(&store, step).await;
    let recorded = rows(&store, step).await;

    let (second, fresh) = open_step().await;
    let scrubber = scrubber();
    let mut recorder = Recorder::new(&second, &scrubber, fresh, true, None);
    for envelope in envelopes(&recorded) {
        let authored = match &envelope.event {
            DriverEvent::Other(other) => match other.update.as_str() {
                "prompt" | "follow_up" | "permission_answer" => Some(other.clone()),
                _ => None,
            },
            _ => None,
        };
        match authored {
            Some(other) => replay_authored(&mut recorder, &other, envelope.at).await,
            None => recorder.record(envelope).await.expect("a row must record"),
        }
    }
    recorder.finish().await.expect("nothing to fail on");

    let again = rows(&second, fresh).await;
    assert_eq!(again.len(), recorded.len(), "same row count");
    for (before, after) in recorded.iter().zip(&again) {
        let after = SessionEvent {
            run_step_id: before.run_step_id,
            ..after.clone()
        };
        assert_eq!(
            &after, before,
            "seq {} did not survive the round trip",
            before.seq
        );
    }
}

/// Re-records one of the three `htui`-authored rows through the call that wrote it.
async fn replay_authored<S: WriteStore>(
    recorder: &mut Recorder<'_, S>,
    other: &OtherEvent,
    at: DateTime<Utc>,
) {
    let text = other.body["text"].as_str().unwrap_or_default();
    match other.update.as_str() {
        "prompt" => recorder
            .record_prompt(text, other.body["sections"].clone(), at)
            .await
            .expect("the prompt must record"),
        "follow_up" => recorder
            .record_follow_up(text, at)
            .await
            .expect("the follow-up must record"),
        _ => {
            let by: AnsweredBy = serde_json::from_value(other.body["by"].clone())
                .expect("the answer names its author");
            recorder
                .record_permission_answer(
                    &PermissionRequestId::new(
                        other.body["request_id"].as_str().unwrap_or_default(),
                    ),
                    other.body["option_id"].as_str(),
                    by,
                    other.body["cancelled"].as_bool().unwrap_or_default(),
                    at,
                )
                .await
                .expect("the answer must record");
        }
    }
}

#[tokio::test]
async fn a_recorded_acp_turn_decodes_to_what_it_rendered() {
    let (store, step) = open_step().await;
    let scrubber = scrubber();
    let mut recorder = Recorder::new(&store, &scrubber, step, false, None);
    for event in mapped_fixture("claude_acp_turn.jsonl") {
        recorder
            .record(env(event))
            .await
            .expect("a row must record");
    }
    recorder.finish().await.expect("nothing to fail on");

    let decoded = envelopes(&rows(&store, step).await);
    // The same net `acp_map.rs` casts one layer up: adapter drift shows as a snapshot diff rather
    // than as a replay that quietly renders a dim `other` line.
    insta::assert_debug_snapshot!("acp_turn", decoded);
}

/// Every `session/update` of a recorded fixture, mapped in order (the `acp_map.rs` helper).
fn mapped_fixture(name: &str) -> Vec<DriverEvent> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    let mut mapper = Mapper::new();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("a fixture line is JSON"))
        .filter(|line| line["method"] == "session/update")
        .flat_map(|line| mapper.map(&line["params"]["update"]))
        .collect()
}

#[tokio::test]
async fn the_fixture_plan_step_decodes() {
    let store = MemStore::demo();
    let rows = store
        .step_events(ids::STEP_PLAN)
        .await
        .expect("reading the log must not fail")
        .expect("the demo plan step has a log");
    let decoded = envelopes(&rows);
    assert_eq!(decoded.len(), rows.len(), "one envelope per row");

    // The demo rows follow ANA-9 §4.3, whose key list omits the id: it is the column. Without the
    // column fill the `tool_call` would not decode at all.
    let call = decoded
        .iter()
        .find_map(|envelope| match &envelope.event {
            DriverEvent::ToolCall(call) => Some(call),
            _ => None,
        })
        .expect("the fixture step calls a tool");
    assert_eq!(call.tool_call_id, "call_1");

    insta::assert_debug_snapshot!("fixture_plan_step", decoded);
}

// ---------------------------------------------------------------------------------------------
// Degradation (D37: an unknown kind never costs the transcript)
// ---------------------------------------------------------------------------------------------

#[test]
fn an_undecodable_payload_is_strict_err_and_lossy_other() {
    let cases = [
        row(
            0,
            EventKind::Plan,
            json!({ "entries": [{ "content": "x", "status": "weird", "priority": "high" }] }),
        ),
        row(1, EventKind::Done, json!({ "stop_reason": "later" })),
        // A hand-written `other` row with no `update` is not a row `record.rs` could have written.
        row(2, EventKind::Other, json!({ "session_id": "sess_1" })),
        // Not an object at all.
        row(3, EventKind::Usage, json!("twelve thousand")),
    ];

    for case in &cases {
        let error = envelope_from_row(case)
            .expect_err("an undecodable payload is an error in the strict form");
        assert_eq!(
            error,
            ReplayError {
                kind: case.kind,
                seq: case.seq,
                reason: error.reason.clone(),
            }
        );
        assert!(
            !error.reason.is_empty(),
            "the error carries serde's reason, seq {}",
            case.seq
        );

        let lossy = envelope_or_other(case);
        let other = other_of(&lossy);
        assert_eq!(other.update, case.kind.as_str());
        assert_eq!(other.body, case.payload, "the payload is not thrown away");
        assert_eq!(lossy.at, case.at);
    }
}

#[test]
fn an_unknown_kind_never_costs_the_transcript() {
    // `EventKind` is closed, so an unmapped kind arrives as `other` - the same seam `map.rs` uses
    // for an update the adapter shipped ahead of the schema.
    let unknown = row(
        7,
        EventKind::Other,
        json!({ "update": "session_mode_update", "body": { "mode": "plan" } }),
    );
    let decoded = envelope_from_row(&unknown).expect("an `other` row always decodes");
    let other = other_of(&decoded);
    assert_eq!(other.update, "session_mode_update");
    assert_eq!(other.body, json!({ "mode": "plan" }));
}

#[test]
fn envelopes_sorts_by_seq() {
    let shuffled = [
        row(
            2,
            EventKind::Error,
            json!({ "code": "c", "message": "third" }),
        ),
        row(
            0,
            EventKind::Error,
            json!({ "code": "a", "message": "first" }),
        ),
        row(
            1,
            EventKind::Error,
            json!({ "code": "b", "message": "second" }),
        ),
    ];
    let messages: Vec<String> = envelopes(&shuffled)
        .iter()
        .map(|envelope| match &envelope.event {
            DriverEvent::Error(error) => error.message.clone(),
            event => panic!("expected an error, got {event:?}"),
        })
        .collect();
    assert_eq!(messages, vec!["first", "second", "third"]);
}
