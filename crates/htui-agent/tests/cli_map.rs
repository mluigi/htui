//! The recorded `claude` stream-json transcripts, replayed through the §6.2 mapper
//! (`docs/ANA-4.md` §8 test strategy 3; plan T51, blueprint F-T51).
//!
//! The fixtures are **real**, and they are the reason this file can assert what it does. They were
//! captured on 2026-09-10 by `tests/cli_live.rs`'s nine `#[ignore]`d probes driving `claude`
//! 2.1.267 against the subscription login on this box, and committed *before* their answers were
//! read (`9cc0ca0`, `2b7863b`) so that the cases rather than one session's notes were the durable
//! part. The answers are the plan's "Probe findings" table, F-1 to F-15, and this file is where
//! each one that concerns the mapper is pinned in code.
//!
//! Each line is `{"direction":"stdin"|"stdout"|"exit", …}`; only `stdout` lines are the agent's,
//! and only they are mapped. A `stdin` line is what `htui` wrote, which §6.2 rows 1–2 say the
//! mapper never turns into a row — the `prompt` and `follow_up` rows are the recorder's.
//!
//! This is the regression net for CLI drift, which is a sharper problem than ACP drift: there is
//! no schema, no negotiated version, and a `claude` release may add an envelope kind between two
//! Tuesdays. When it does, the snapshot shows the new kind landing in `other` rather than the
//! build breaking — and F-15 is the evidence that this is load-bearing rather than decorative:
//! **six** `system` subtypes arrived that §6.2 never names, on the very first recorded run.

use htui_agent::cli::claude::Mapper;
use htui_agent::event::{DriverEvent, StopReason, ToolKind, ToolResultStatus};
use htui_agent::launch::UsageScope;
use htui_core::model::UsageTotals;
use serde_json::{Value, json};

/// Every recorded line of a transcript, in order.
fn lines(name: &str) -> Vec<Value> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a fixture line is JSON"))
        .collect()
}

/// The agent's own lines: `stdout`, in order, parsed.
fn stdout(name: &str) -> Vec<Value> {
    lines(name)
        .into_iter()
        .filter(|line| line["direction"] == json!("stdout"))
        .map(|line| line["line"].clone())
        .collect()
}

/// A whole transcript mapped by one `Mapper`, as one session's worth of state.
fn mapped(name: &str) -> Vec<DriverEvent> {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);
    stdout(name)
        .iter()
        .flat_map(|line| mapper.map(line))
        .collect()
}

/// The `usage` events of a mapped transcript.
fn usage_events(events: &[DriverEvent]) -> Vec<&htui_agent::event::UsageEvent> {
    events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Usage(usage) => Some(usage),
            _ => None,
        })
        .collect()
}

/// The `done` events of a mapped transcript.
fn stop_reasons(events: &[DriverEvent]) -> Vec<StopReason> {
    events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Done(done) => Some(done.stop_reason),
            _ => None,
        })
        .collect()
}

/// The `other.update` names of a mapped transcript, in order.
fn other_updates(events: &[DriverEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Other(other) => Some(other.update.as_str()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The snapshots
// ---------------------------------------------------------------------------------------------

#[test]
fn a_recorded_two_turn_conversation_maps_to_the_rows_it_did_when_it_was_captured() {
    let events = mapped("claude_stream_json_turns.jsonl");
    // `Debug`, not JSON, for `acp_map.rs`'s reason: `DriverEvent` is deliberately not
    // `Serialize` — what reaches `session_event.payload` is the *payload* struct and the recorder
    // owns that step. The snapshot's job is to pin the decode, and the variant names are what a
    // drift diff should show.
    insta::assert_debug_snapshot!("cli_map__turns", events);
}

#[test]
fn a_recorded_tool_call_and_its_result_map_to_a_joined_pair() {
    let events = mapped("claude_stream_json_denied.jsonl");
    insta::assert_debug_snapshot!("cli_map__tool_call", events);
}

#[test]
fn a_recorded_thinking_turn_maps_to_the_rows_it_did_when_it_was_captured() {
    let events = mapped("claude_stream_json_thinking.jsonl");
    insta::assert_debug_snapshot!("cli_map__thinking", events);
}

#[test]
fn a_recorded_cancelled_turn_maps_to_the_rows_it_did_when_it_was_captured() {
    let events = mapped("claude_stream_json_sigint.jsonl");
    insta::assert_debug_snapshot!("cli_map__sigint", events);
}

// ---------------------------------------------------------------------------------------------
// F-4: `modelUsage` is cumulative, so every token figure is a delta
// ---------------------------------------------------------------------------------------------

/// Criterion 7 over the mapper alone, and E-2's answer pinned as arithmetic.
///
/// The `_turns` fixture is two turns in **one** process, which is the only shape that can tell a
/// cumulative counter from a per-turn one. It measured (F-4): `modelUsage` input 2 → 4, output
/// 4 → 8, cache-read 3809 → 21232, `total_cost_usd` 0.1381545 → 0.1480460, while `result.usage`
/// stayed 2/4 on both turns. So `modelUsage` is **cumulative** and `result.usage` is per-turn —
/// the inversion of what the field names suggest, and the reason the mapper reads the map rather
/// than the summary (D86) *and* differences it (E-2).
///
/// A mapper that summed `modelUsage` per `result` instead would put 4 input tokens on turn 2 where
/// 2 were spent, `run_step.usage` would read 6 for a conversation that cost 4, and **no test in
/// the tree would have failed** — which is exactly the silent criterion-7 breach E-2 was raised to
/// catch.
#[test]
fn usage_is_one_row_per_turn_with_every_figure_a_delta_of_the_cumulative_report() {
    let events = mapped("claude_stream_json_turns.jsonl");
    let usage = usage_events(&events);
    assert_eq!(
        usage.len(),
        2,
        "two turns, two `usage` rows: §6.2 puts the turn's whole cost on its terminal `result`, \
         which is also why `DriverCaps.usage_mid_turn` is false for this transport (D91)"
    );

    // Turn 1 is the whole session so far, so its delta is its total.
    assert_eq!(usage[0].input_tokens, Some(2));
    assert_eq!(usage[0].output_tokens, Some(4));
    assert_eq!(usage[0].cache_read_tokens, Some(3809));
    assert_eq!(usage[0].cost_micros, usage[0].cost_micros_total);

    // Turn 2's figures are the *difference*, not the cumulative report.
    assert_eq!(
        usage[1].input_tokens,
        Some(2),
        "F-4: `modelUsage.inputTokens` read 4 on turn 2 against 2 on turn 1; the row carries the \
         2 the turn spent"
    );
    assert_eq!(usage[1].output_tokens, Some(4));
    assert_eq!(
        usage[1].cache_read_tokens,
        Some(21232 - 3809),
        "the same rule over a figure large enough that a double count would be unmissable"
    );

    // Criterion 7, stated as the store will state it: the sum of the deltas is the last total.
    let last_total = usage[1]
        .cost_micros_total
        .expect("the terminal `result` reports `total_cost_usd`");
    let summed: i64 = usage
        .iter()
        .map(|usage| usage.cost_micros.unwrap_or_default())
        .sum();
    assert_eq!(
        summed, last_total,
        "criterion 7 on the mapper alone: `run_step.usage` is a plain sum of the step's `usage` \
         rows, so the deltas must reconstruct the cumulative figure exactly"
    );

    // And the same sum through the type the uploader actually uses.
    let mut totals = UsageTotals::default();
    for usage in &usage {
        totals.add_payload(&serde_json::to_value(usage).expect("a usage payload serialises"));
    }
    assert_eq!(totals.cost_micros, Some(last_total));
    assert_eq!(totals.input_tokens, Some(4), "2 + 2, the two turns' spend");
    assert_eq!(totals.output_tokens, Some(8));
    assert_eq!(totals.cache_read_tokens, Some(21232));
}

/// F-5, recorded as measured rather than as hoped.
///
/// D86 chose `modelUsage` over `result.usage` because the latter "counts only the top-level loop
/// and undercounts as soon as a subagent runs" (§7). The subagent probe confirms the direction and
/// **not** the drama: with one subagent spawned over three turns, `result.usage` read 6 in / 348
/// out and `modelUsage` read 8 in / 352 out. Two tokens and four. The choice stands on `modelUsage`
/// being a superset, which is a property, not on the size of a gap, which is a sample.
#[test]
fn a_subagent_turn_reports_the_model_usage_figure_not_the_result_summary() {
    let events = mapped("claude_stream_json_subagent.jsonl");
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 1, "one turn, one `usage` row");
    assert_eq!(
        (usage[0].input_tokens, usage[0].output_tokens),
        (Some(8), Some(352)),
        "the `modelUsage` figures; `result.usage` said 6 and 348 on the same line (F-5)"
    );
}

// ---------------------------------------------------------------------------------------------
// F-6, F-7: thinking
// ---------------------------------------------------------------------------------------------

/// F-6: §6.2's shape is right and its payload is empty.
///
/// The block arrives as `assistant.content[].type == "thinking"` exactly as §6.2 predicted, and
/// its `thinking` field is the empty string beside a `signature`. Thinking is **signalled and not
/// disclosed** on this box: a `thought` over this transport carries no prose.
///
/// The mapper still emits the event — the kind is real, the chat tab should show that the agent is
/// thinking, and inventing text for it would be worse than an empty one — and it never lets the
/// `signature_delta` become that text. A base64 block signature is not words, and concatenating it
/// into a `thought` row would put an opaque kilobyte in the transcript where a thought should be.
#[test]
fn a_thinking_block_is_a_thought_with_no_prose_and_never_its_signature() {
    let events = mapped("claude_stream_json_thinking.jsonl");
    let thoughts: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::ThoughtChunk(chunk) => Some(chunk.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !thoughts.is_empty(),
        "F-6: the turn thought — `system/thinking_tokens` counted 113 estimated tokens — so the \
         transcript must say so even though the words never arrived"
    );
    assert!(
        thoughts.iter().all(|text| text.is_empty()),
        "F-6: `thinking` was the empty string on every block and every delta; a non-empty thought \
         here means the mapper invented one, most likely out of the `signature`: {thoughts:?}"
    );
    let signatures = events.iter().any(|event| match event {
        DriverEvent::ThoughtChunk(chunk) => chunk.text.contains("CAIS"),
        _ => false,
    });
    assert!(!signatures, "a block signature is not a thought");
}

/// F-7: the coalescing key is `(message.id, block index)`, and `message.id` alone is a trap.
///
/// The thinking envelope and the reply envelope of one turn share **one** `message.id`
/// (`msg_011CevdLxgT3W8F3cwtmkydk` in the fixture). The plan said to coalesce `assistant_text` on
/// `message.id`; doing that would make the recorder treat the thought and the answer as one
/// contiguous run and fold them into a single `assistant_text` row — losing the thought as a kind
/// and corrupting the reply as text.
#[test]
fn a_thought_and_a_reply_sharing_one_message_id_stay_two_runs() {
    let ids: Vec<String> = stdout("claude_stream_json_thinking.jsonl")
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["id"].as_str().map(ToOwned::to_owned))
        .collect();
    assert_eq!(
        ids.len(),
        2,
        "the fixture's premise: two `assistant` envelopes, one thinking and one text"
    );
    assert_eq!(
        ids[0], ids[1],
        "F-7's premise: and they carry the *same* `message.id`, which is what makes it an unsafe \
         grouping key on its own"
    );

    let events = mapped("claude_stream_json_thinking.jsonl");
    let keys: Vec<(&str, Option<&str>)> = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::ThoughtChunk(chunk) => Some(("thought", chunk.message_id.as_deref())),
            DriverEvent::AssistantChunk(chunk) => Some(("text", chunk.message_id.as_deref())),
            _ => None,
        })
        .collect();
    let thought_key = keys
        .iter()
        .find(|(kind, _)| *kind == "thought")
        .and_then(|(_, key)| *key)
        .expect("the thought carries a grouping key");
    let text_key = keys
        .iter()
        .find(|(kind, _)| *kind == "text")
        .and_then(|(_, key)| *key)
        .expect("the reply carries a grouping key");
    assert_ne!(
        thought_key, text_key,
        "F-7: the two runs must be distinguishable by the key the recorder groups on, which is \
         why the block index is part of it — got {thought_key:?} for both"
    );
    assert!(
        thought_key.starts_with(&ids[0]) && text_key.starts_with(&ids[0]),
        "and both keys still name the message they belong to, so a message change still flushes \
         (§4.1 trigger 2): {thought_key:?} / {text_key:?}"
    );
}

/// H-8: text that arrived as deltas does not arrive again as a whole message.
///
/// `--include-partial-messages` sends both — every `text_delta`, and then the assembled `assistant`
/// envelope carrying the same string. Mapping both would double every reply in the transcript.
#[test]
fn text_streams_once_not_twice() {
    let events = mapped("claude_stream_json_thinking.jsonl");
    let streamed: String = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::AssistantChunk(chunk) => Some(chunk.text.as_str()),
            _ => None,
        })
        .collect();
    let whole = stdout("claude_stream_json_thinking.jsonl")
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| {
            line["message"]["content"][0]["text"]
                .as_str()
                .map(str::to_owned)
        })
        .next()
        .expect("the fixture's `assistant` text envelope");
    assert_eq!(
        streamed, whole,
        "H-8: the concatenated deltas equal the message exactly once — a mapper that emitted the \
         assembled envelope too would read it twice over"
    );
}

// ---------------------------------------------------------------------------------------------
// F-2, F-8, F-10: the terminal envelope
// ---------------------------------------------------------------------------------------------

/// F-2: a turn interrupted by SIGINT ends `cancelled`, keyed on the wire and not on memory.
///
/// The probe measured what §4.4 could only hypothesise: SIGINT after a stdin close **does** bring a
/// terminal `result`, so the driver gets a real `done` — but that envelope is error-shaped
/// (`subtype: "error_during_execution"`, `is_error: true`). `terminal_reason: "aborted_streaming"`
/// is what tells a cancellation apart from a failure, and it is a fact on the line rather than a
/// fact about the supervisor, which is why the mapper can read it at all. A mapper keyed on "did
/// *I* send the signal" would report a turn the user cancelled from another terminal as an error.
#[test]
fn an_aborted_stream_is_a_cancelled_done_and_not_an_error() {
    let events = mapped("claude_stream_json_sigint.jsonl");
    assert_eq!(
        stop_reasons(&events),
        vec![StopReason::Cancelled],
        "F-2: `terminal_reason: aborted_streaming` is the cancellation, whatever `subtype` says"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DriverEvent::Error(_))),
        "a cancellation the user asked for is not an error row: the turn ended the way it was told \
         to"
    );
    assert!(
        usage_events(&events).is_empty(),
        "F-2: the aborted `result` reported `total_cost_usd: 0` and an empty `modelUsage`, so \
         there is no spend to record; a zero-valued row would claim a measurement that was never \
         taken"
    );
}

/// F-8: `subtype: "success"` is a label, not a verdict.
///
/// The `--bare` probe's terminal envelope reads `subtype: "success"` **and** `is_error: true`,
/// `terminal_reason: "api_error"`, `result: "Not logged in · Please run /login"`. A mapper that
/// keyed the turn's outcome on `subtype` — which is what a reader of §6.2 would write — would
/// record an authentication failure as a clean turn, and the chat tab would show an empty reply
/// with no reason given.
#[test]
fn an_error_wearing_a_success_subtype_is_still_an_error() {
    let events = mapped("claude_stream_json_bare.jsonl");
    let errors: Vec<(&str, &str)> = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Error(error) => Some((error.code.as_str(), error.message.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "F-8: one `error` row, from `is_error: true` — never from `subtype`"
    );
    assert_eq!(errors[0].0, "api_error", "the code is the terminal reason");
    assert!(
        errors[0].1.contains("Not logged in"),
        "and the message is the one the CLI printed, which is the only place the reason exists: \
         {:?}",
        errors[0].1
    );
    assert_eq!(
        stop_reasons(&events),
        vec![StopReason::EndTurn],
        "the turn still ended, and exactly once: an error is a reason to stop, not a reason to \
         leave a turn open forever"
    );
}

/// F-10: a server-side budget breach is an error and a finished turn.
#[test]
fn a_budget_breach_is_an_error_row_and_a_done() {
    let events = mapped("claude_stream_json_budget.jsonl");
    let errors: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Error(error) => Some(error.message.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].contains("Reached maximum budget"),
        "the CLI's own words: {:?}",
        errors[0]
    );
    assert_eq!(stop_reasons(&events), vec![StopReason::EndTurn]);
}

/// The ordinary case, which is also the ordering `enforce_breach` depends on.
///
/// `Done` is emitted **after** `Usage`, so a per-run cap breach detected on the turn's `usage` row
/// finds the `done` behind it and can withhold it (milestone 7's D69/H-4). The reverse order would
/// let the turn close before its own cost was known.
#[test]
fn a_plain_turn_ends_with_usage_then_done() {
    let events = mapped("claude_stream_json_plain.jsonl");
    let tail: Vec<&str> = events
        .iter()
        .rev()
        .take(2)
        .map(|event| match event {
            DriverEvent::Usage(_) => "usage",
            DriverEvent::Done(_) => "done",
            DriverEvent::Error(_) => "error",
            _ => "other",
        })
        .collect();
    assert_eq!(tail, vec!["done", "usage"], "reversed: usage, then done");
    assert_eq!(stop_reasons(&events), vec![StopReason::EndTurn]);
}

// ---------------------------------------------------------------------------------------------
// Tool calls
// ---------------------------------------------------------------------------------------------

#[test]
fn a_tool_use_is_a_call_and_its_result_joins_on_the_tool_use_id() {
    let events = mapped("claude_stream_json_denied.jsonl");
    let call = events
        .iter()
        .find_map(|event| match event {
            DriverEvent::ToolCall(call) => Some(call),
            _ => None,
        })
        .expect("the fixture ran `Bash`");
    assert_eq!(call.title, "Bash");
    assert_eq!(
        call.tool_kind,
        ToolKind::Execute,
        "§6.2's table keys the kind on the dialect's tool vocabulary, which is a wire fact of the \
         stream this mapper parses and not an agent's name (`R-AGT-5`)"
    );
    assert_eq!(call.input["command"], json!("ls"));

    let result = events
        .iter()
        .find_map(|event| match event {
            DriverEvent::ToolResult(result) => Some(result),
            _ => None,
        })
        .expect("and reported its result");
    assert_eq!(
        result.tool_call_id, call.tool_call_id,
        "the join key §4.4 names: `tool_use_id`"
    );
    assert_eq!(result.status, ToolResultStatus::Completed);
    assert_eq!(
        result.terminal_reason, None,
        "a result the agent actually reported is not one `htui` synthesized (`event.rs:262-264`)"
    );
}

#[test]
fn the_tool_name_table_is_the_one_section_6_2_prints() {
    use htui_agent::cli::claude::tool_kind;
    assert_eq!(tool_kind("Read"), ToolKind::Read);
    assert_eq!(tool_kind("Edit"), ToolKind::Edit);
    assert_eq!(tool_kind("Write"), ToolKind::Edit);
    assert_eq!(tool_kind("MultiEdit"), ToolKind::Edit);
    assert_eq!(tool_kind("NotebookEdit"), ToolKind::Edit);
    assert_eq!(tool_kind("Bash"), ToolKind::Execute);
    assert_eq!(tool_kind("Glob"), ToolKind::Search);
    assert_eq!(tool_kind("Grep"), ToolKind::Search);
    assert_eq!(tool_kind("WebFetch"), ToolKind::Fetch);
    assert_eq!(tool_kind("WebSearch"), ToolKind::Fetch);
    assert_eq!(
        tool_kind("TodoWrite"),
        ToolKind::Other,
        "§6.2 marks `plan` as not produced by this transport, so the tool that would be its source \
         is deliberately not mapped to one"
    );
    assert_eq!(
        tool_kind("SomeToolShippedNextTuesday"),
        ToolKind::Other,
        "an unknown tool is a tool, not a parse failure"
    );
}

#[test]
fn a_tool_input_naming_a_path_carries_it_as_a_location() {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);
    let events = mapper.map(&json!({
        "type": "assistant",
        "message": { "id": "msg_1", "content": [ {
            "type": "tool_use",
            "id": "toolu_1",
            "name": "Read",
            "input": { "file_path": "/scratch/src/main.rs" }
        } ] }
    }));
    let DriverEvent::ToolCall(call) = &events[0] else {
        panic!("a tool call: {events:?}")
    };
    assert_eq!(call.locations.len(), 1);
    assert_eq!(call.locations[0].path, "/scratch/src/main.rs");
    assert_eq!(
        call.locations[0].line, None,
        "the stream names no line, and inventing 1 would claim the file's top was the subject"
    );
}

// ---------------------------------------------------------------------------------------------
// F-12: the denial, measured
// ---------------------------------------------------------------------------------------------

/// **D85, against a real refusal.**
///
/// This mapping shipped unproven and did not stay that way. Case 9 was written to provoke a denial
/// and failed to: it asked for `Bash(ls)`, and this box's settings carry `Bash(ls *)` in their
/// allow list, so the one command it chose was the one the box pre-approves — and
/// `permission_denials` was empty in all fourteen of the first transcripts (plan F-12).
///
/// Case 10 does not depend on the box. `--permission-prompts none` is documented as "anything that
/// would prompt is denied automatically", which makes the refusal a property of the invocation
/// rather than of somebody's allow list, and a `Write` under `--permission-mode default` is such a
/// thing. It produced `claude_stream_json_policy_denied.jsonl`, and the file it was refused was
/// never created.
#[test]
fn a_policy_denial_is_a_permission_answer_by_policy() {
    let events = mapped("claude_stream_json_policy_denied.jsonl");
    let answers: Vec<&htui_agent::event::PermissionAnswerEvent> = events
        .iter()
        .filter_map(|event| match event {
            DriverEvent::PermissionAnswer(answer) => Some(answer),
            _ => None,
        })
        .collect();
    assert_eq!(
        answers.len(),
        1,
        "one refusal, one row — and the CLI reported it **twice** (F-12b), live as \
         `system/permission_denied` and again in the terminal `result`. A mapper that took both \
         would make the transcript claim the tool was refused twice: {answers:?}"
    );
    let answer = answers[0];
    assert_eq!(
        answer.by,
        htui_agent::record::AnsweredBy::Policy,
        "D85: the transport's own policy answered, which is what makes the row `role: htui` rather \
         than `agent` and keeps it distinguishable from an answer a human gave"
    );
    assert!(answer.denied, "and it refused");
    assert!(
        !answer.cancelled,
        "one call was refused, not every outstanding request settled at once"
    );
    assert_eq!(answer.option_id, None, "a denial picks nothing");

    // The join key, against the call it actually settled — read out of the same transcript rather
    // than written into the test, which is what makes this a measurement.
    let call = events
        .iter()
        .find_map(|event| match event {
            DriverEvent::ToolCall(call) => Some(call),
            _ => None,
        })
        .expect("the turn reached for a tool");
    assert_eq!(call.title, "Write");
    assert_eq!(
        answer.tool_call_id.as_deref(),
        Some(call.tool_call_id.as_str()),
        "the answer names the call it refused"
    );
    assert_eq!(
        answer.request_id.as_str(),
        call.tool_call_id,
        "this transport announces no request, so the call's own id is what makes the pair joinable \
         (`PermissionAnswerEvent::request_id`)"
    );

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, DriverEvent::PermissionRequest(_))),
        "§4.3: this transport has no permission channel and must never claim to have asked"
    );
}

/// F-12b: the refusal is announced **where it happened**, not only at the end of the turn.
///
/// D85 named two sources and the live run produced both. The order is the point: a transcript that
/// reported a denial after the agent's closing remark would be a worse account of the same turn
/// than one that reports it before. The end-of-turn copy is suppressed, not preferred.
#[test]
fn the_denial_is_recorded_in_the_position_it_occurred() {
    let events = mapped("claude_stream_json_policy_denied.jsonl");
    let answer_at = events
        .iter()
        .position(|event| matches!(event, DriverEvent::PermissionAnswer(_)))
        .expect("the refusal is a row");
    let usage_at = events
        .iter()
        .position(|event| matches!(event, DriverEvent::Usage(_)))
        .expect("the turn reported its cost");
    assert!(
        answer_at < usage_at,
        "the answer precedes the terminal `result`'s own rows, which is only possible if it came \
         from the live `system/permission_denied` envelope rather than from \
         `result.permission_denials[]`"
    );
    assert!(
        !other_updates(&events).contains(&"system/permission_denied"),
        "and it is a typed row rather than an `other` one: D93's rule is that a recognizable shape \
         whose destination is an existing `EventKind` gets the typed variant"
    );
}

// ---------------------------------------------------------------------------------------------
// F-13, F-15: the quota blob, and everything §6.2 did not name
// ---------------------------------------------------------------------------------------------

/// F-13: the `rate_limit_event` blob is **held** and rides the turn's one `usage` row.
///
/// A `usage` row arriving mid-turn would contradict `DriverCaps.usage_mid_turn: false` for this
/// very transport (D91), so the blob cannot travel as one. It is stored verbatim on the terminal
/// `usage` row's `quota` key, which is where `Recorder::latch_quota` reads it — and the envelope is
/// *also* kept as an `other` row, so the transcript shows the moment it arrived.
#[test]
fn a_rate_limit_event_is_an_other_row_and_rides_the_turns_usage() {
    let events = mapped("claude_stream_json_plain.jsonl");
    assert!(
        other_updates(&events).contains(&"rate_limit_event"),
        "the envelope is in the transcript where it happened"
    );
    let usage = usage_events(&events);
    assert_eq!(usage.len(), 1);
    let quota = usage[0]
        .quota
        .as_ref()
        .expect("F-13: and its blob is on the turn's `usage` row, for the latch to read");
    assert_eq!(
        quota["status"],
        json!("allowed_warning"),
        "verbatim: normalizing the vendor's shape is `htui_core::model::quota`'s job, selected by \
         the row's declared source and never by the agent's name (D66)"
    );
    assert!(
        quota["unifiedWindows"]["seven_day"]["utilization"].is_number(),
        "and the windows survive, which is what makes `normalize` able to report one at all"
    );
}

/// A blob that is not an object is not the document §7 describes.
#[test]
fn a_rate_limit_event_that_is_not_an_object_is_not_carried() {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);
    let _ = mapper.map(&json!({ "type": "rate_limit_event", "rate_limit_info": "soon" }));
    let events = mapper.map(&json!({
        "type": "result", "subtype": "success", "is_error": false,
        "terminal_reason": "completed", "total_cost_usd": 1.0, "modelUsage": {}
    }));
    let usage = usage_events(&events);
    assert_eq!(
        usage[0].quota, None,
        "the `is_object` filter is the only judgement made here, and it is the same one \
         `acp/map.rs:116-120` makes: handing the normalizer a scalar would make it reject a shape \
         this mapper could have declined to store"
    );
}

/// F-15: six `system` subtypes §6.2 never names arrived on the first recorded run.
///
/// The wildcard arm is not a formality. `status`, `thinking_tokens` and the three `task_*` kinds
/// the subagent case produced are all real envelopes of a real release, and every one of them
/// lands in `other` **verbatim** — which is what makes "every event" hold without a schema change
/// per `claude` release (`docs/ANA-4.md` §6.2's extension point).
#[test]
fn every_envelope_section_6_2_does_not_name_lands_in_other_verbatim() {
    let events = mapped("claude_stream_json_subagent.jsonl");
    let updates = other_updates(&events);
    for expected in [
        "system/hook_started",
        "system/hook_response",
        "system/init",
        "system/thinking_tokens",
        "system/task_started",
        "system/task_updated",
        "system/task_notification",
        "rate_limit_event",
    ] {
        assert!(
            updates.contains(&expected),
            "{expected} is missing from {updates:?}; an envelope this mapper cannot classify is \
             still an event that happened"
        );
    }

    let body = events
        .iter()
        .find_map(|event| match event {
            DriverEvent::Other(other) if other.update == "system/task_started" => Some(&other.body),
            _ => None,
        })
        .expect("the subagent spawn");
    assert!(
        body.get("type").is_none() && body.get("subtype").is_none(),
        "the body is the line minus the two keys that became its name — §6.2's verbatim body, not \
         a re-encoding of it"
    );
    assert!(
        body.get("task_id").is_some() && body.get("subagent_type").is_some(),
        "and nothing else is dropped: {body}"
    );
}

/// An unparsed line is an event too.
#[test]
fn a_line_that_is_not_an_envelope_is_still_recorded() {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);
    let events = mapper.map(&json!({ "no_type_key": true }));
    let DriverEvent::Other(other) = &events[0] else {
        panic!("{events:?}")
    };
    assert_eq!(other.update, "<missing>");
    assert_eq!(
        other.body,
        json!({ "no_type_key": true }),
        "verbatim, because a line `htui` cannot classify is the one most worth keeping whole"
    );
}

/// §6.2 rows 1–2: the prompt and the follow-up are the recorder's rows, not the mapper's.
#[test]
fn a_user_line_with_no_tool_result_maps_to_nothing() {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);
    let events = mapper.map(&json!({
        "type": "user",
        "message": { "role": "user", "content": [ { "type": "text", "text": "hello" } ] }
    }));
    assert!(
        events.is_empty(),
        "`htui` wrote this line; reading it back as a row would duplicate the `prompt` row the \
         recorder already wrote — the same rule `acp/map.rs` applies to `user_message_chunk`: \
         {events:?}"
    );
}

/// The interruption message a SIGINT leaves behind is the CLI's own, and is kept.
#[test]
fn the_interruption_notice_is_kept_as_what_it_is() {
    let events = mapped("claude_stream_json_sigint.jsonl");
    assert!(
        !events.iter().any(|event| matches!(
            event,
            DriverEvent::AssistantChunk(chunk) if chunk.text.contains("[Request interrupted")
        )),
        "the CLI's `[Request interrupted by user]` arrives as a `user` message, and a `user` \
         message is not the agent speaking"
    );
}

// ---------------------------------------------------------------------------------------------
// A cumulative counter that goes backwards (review gate, HIGH)
// ---------------------------------------------------------------------------------------------

/// A `result` reporting **less** than the session total so far is not reporting this session's
/// total, and nothing about it may reach `run_step.usage`.
///
/// The shape is real and measured, not hypothetical: `_bare`'s terminal envelope is
/// `total_cost_usd: 0`, `modelUsage: {}`, `is_error: true`, `terminal_reason: "api_error"` — and an
/// `api_error` is **not** a cancellation, so F-2's guard does not cover it. In a multi-turn process
/// a costed turn followed by one of these produced `cost_micros = 0 − 168710` on the row **and**
/// reset the baseline to `0`, so the *next* turn re-reported the whole session total as its own
/// delta. `run_step.usage` went negative and then double-counted — criterion 7 broken in both
/// directions by one envelope — and the per-run cap reads the same figure.
///
/// Clamping the delta at zero would have been the wrong fix: it hides the negative row and leaves
/// the baseline reset, so the double count on the following turn survives. The rule is the honest
/// one — a cumulative figure cannot decrease, so an envelope where it did is not a measurement of
/// this session and is left out entirely, baseline included.
#[test]
fn a_result_reporting_less_than_the_session_total_is_not_a_measurement() {
    let mut mapper = Mapper::new(UsageScope::ModelUsage);

    let costed = json!({
        "type": "result", "subtype": "success", "is_error": false,
        "terminal_reason": "completed", "total_cost_usd": 0.168_710,
        "modelUsage": { "m": { "inputTokens": 2, "outputTokens": 4 } }
    });
    let first = mapper.map(&costed);
    let usage = usage_events(&first);
    assert_eq!(
        usage[0].cost_micros,
        Some(168_710),
        "the first turn's spend"
    );

    // The `_bare` shape: an error-shaped, **uncancelled** result reporting nothing.
    let api_error = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "terminal_reason": "api_error", "result": "Not logged in · Please run /login",
        "total_cost_usd": 0, "modelUsage": {}
    });
    let second = mapper.map(&api_error);
    assert!(
        usage_events(&second).is_empty(),
        "a turn that reported less than the session total reports nothing at all: {:?}",
        usage_events(&second)
    );
    assert!(
        second
            .iter()
            .any(|event| matches!(event, DriverEvent::Error(_))),
        "the failure is still an `error` row — what is dropped is the false measurement, not the \
         fact that the turn failed"
    );

    // The baseline must not have moved, or this turn pays for the first one twice.
    let third = mapper.map(&json!({
        "type": "result", "subtype": "success", "is_error": false,
        "terminal_reason": "completed", "total_cost_usd": 0.185_455,
        "modelUsage": { "m": { "inputTokens": 4, "outputTokens": 7 } }
    }));
    let usage = usage_events(&third);
    assert_eq!(
        usage[0].cost_micros,
        Some(185_455 - 168_710),
        "the delta is measured from the last **real** total, not from the zero the failed turn \
         reported"
    );
    assert_eq!(
        usage[0].input_tokens,
        Some(2),
        "and the token baseline survived the same way"
    );

    // Criterion 7 over the three turns: the deltas still reconstruct the last cumulative figure.
    let summed: i64 = [&first, &second, &third]
        .iter()
        .flat_map(|events| usage_events(events))
        .filter_map(|usage| usage.cost_micros)
        .sum();
    assert_eq!(
        summed, 185_455,
        "the sum of the deltas is the last total, exactly"
    );
}
