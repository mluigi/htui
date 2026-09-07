//! `docs/ANA-4.md` §6.1 as code: one `session/update` in, zero or more [`DriverEvent`]s out.
//!
//! The input is the **raw** `update` object of a `session/update` notification, never an SDK enum.
//! `SessionUpdate` is a `#[serde(tag = "sessionUpdate")]` `#[non_exhaustive]` enum with no
//! catch-all variant, so a typed decode of an update the schema does not know is a serde error —
//! and the `claude` adapter ships five kinds the schema does not know (`subagent_spawned`,
//! `subagent_state_update`, `async_task_spawned`, `async_task_progress`,
//! `async_task_state_update`, §6.1's last paragraph). §6.1's wildcard row exists precisely so
//! those are *stored*, not dropped, so this module reads JSON and never gives the schema a chance
//! to refuse one.
//!
//! Everything here is pure: JSON in, events out, one field of carried state (the previous
//! cumulative cost, which is what makes `usage.cost_micros` a delta and `run_step.usage` a plain
//! sum, §7).

use serde_json::{Map, Value};

use crate::acp::fs::unified_diff;
use crate::event::{
    DriverEvent, EditProposalEvent, PlanEntry, PlanEntryPriority, PlanEntryStatus, PlanEvent,
    StopReason, TextChunk, ToolCallEvent, ToolKind, ToolLocation, ToolResultEvent,
    ToolResultStatus, UsageEvent,
};

/// The `sessionUpdate` tag reported for an update object that carries none.
const MISSING_KIND: &str = "<missing>";

/// One micro-dollar, as the multiplier from the `cost.amount` ACP reports in whole currency units.
const MICROS_PER_UNIT: f64 = 1_000_000.0;

/// Per-session mapping state.
///
/// Only the cumulative cost: ACP reports `cost` as the session total (§3), while
/// `session_event.payload.cost_micros` is documented as a delta, and `run_step.usage` is their sum
/// (`crate::event::UsageEvent`).
#[derive(Debug, Default)]
pub struct Mapper {
    /// The last `cost_micros_total` observed, if any.
    last_cost_micros_total: Option<i64>,
}

impl Mapper {
    /// A mapper for one session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps one `update` object.
    ///
    /// Zero events for `user_message_chunk` (§6.1's last row: it appears only during
    /// `session/load` replay, where it would duplicate a `prompt`/`follow_up` row `htui` already
    /// wrote). Two for a `tool_call` that carries a diff content block, or a `tool_call_update`
    /// that both carries one and reaches a terminal status — the proposal is emitted **before**
    /// the result, so a reader sees the edit and then how the call ended.
    pub fn map(&mut self, update: &Value) -> Vec<DriverEvent> {
        let kind = update_kind(update);
        match kind {
            "agent_message_chunk" => vec![chunk(update, kind, DriverEvent::AssistantChunk)],
            "agent_thought_chunk" => vec![chunk(update, kind, DriverEvent::ThoughtChunk)],
            "tool_call" => tool_call(update),
            "tool_call_update" => tool_call_update(update),
            "plan" => vec![DriverEvent::Plan(plan(update))],
            "usage_update" => vec![DriverEvent::Usage(self.usage(update))],
            // §6.1's last row: dropped, never manufactured into a duplicate.
            "user_message_chunk" => Vec::new(),
            // Everything else — the four named session updates of §6.1's `other` row, every
            // `unstable_*` variant, every kind an adapter ships ahead of the schema, and a typo.
            _ => vec![other(kind, body_of(update))],
        }
    }

    /// `usage_update` → `usage`, with the cost delta of §7.
    fn usage(&mut self, update: &Value) -> UsageEvent {
        let mut event = UsageEvent {
            context_used: int_at(update, "used"),
            context_size: int_at(update, "size"),
            ..UsageEvent::default()
        };
        let cost = update.get("cost");
        let amount = cost
            .and_then(|cost| cost.get("amount"))
            .and_then(Value::as_f64);
        let currency = cost
            .and_then(|cost| cost.get("currency"))
            .and_then(Value::as_str);
        match (amount, currency) {
            (Some(amount), Some("USD") | None) => {
                let total = (amount * MICROS_PER_UNIT).round() as i64;
                event.cost_micros = Some(total - self.last_cost_micros_total.unwrap_or(0));
                event.cost_micros_total = Some(total);
                self.last_cost_micros_total = Some(total);
            }
            // A currency `htui` cannot sum into `run_step.usage`: the numbers are reported as they
            // arrived and `cost_micros` stays `None` rather than pretending the rate is one.
            (Some(amount), Some(currency)) => {
                event.cost_amount = Some(amount);
                event.cost_currency = Some(currency.to_owned());
            }
            _ => {}
        }
        event
    }
}

/// The `sessionUpdate` tag of an update object, or `"<missing>"`.
///
/// The tag is also the `other.update` text, which is why a missing one is a *name* rather than an
/// error: an update `htui` cannot classify is still an event that happened.
#[must_use]
pub fn update_kind(update: &Value) -> &str {
    update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or(MISSING_KIND)
}

/// The update object minus its `sessionUpdate` key: §6.1's "verbatim body".
#[must_use]
pub fn body_of(update: &Value) -> Value {
    match update {
        Value::Object(fields) => {
            let mut body = fields.clone();
            body.remove("sessionUpdate");
            Value::Object(body)
        }
        other => other.clone(),
    }
}

/// A wire `kind` string → the ten-value [`ToolKind`], unknown → [`ToolKind::Other`].
///
/// The mapper owns this fallback, not serde: `ToolKind`'s own `Other` is documented as the value a
/// *transport mapper* produces for an unknown wire string (`crate::event`).
#[must_use]
pub fn tool_kind(kind: Option<&str>) -> ToolKind {
    kind.and_then(|kind| ToolKind::ALL.iter().copied().find(|k| k.as_str() == kind))
        .unwrap_or(ToolKind::Other)
}

/// A wire `status` string → `Some` for the two terminal statuses, `None` for every other.
///
/// ACP's `ToolCallStatus` is `pending | in_progress | completed | failed` (§3) while ANA-9 §4.3's
/// `tool_result.status` vocabulary is `completed | failed`: only a terminal status produces a row.
#[must_use]
pub fn terminal_status(status: Option<&str>) -> Option<ToolResultStatus> {
    match status {
        Some("completed") => Some(ToolResultStatus::Completed),
        Some("failed") => Some(ToolResultStatus::Failed),
        _ => None,
    }
}

/// The `{ type: "diff", path, oldText, newText }` entries of a `content[]` array as proposals.
///
/// ACP carries old and new full text, never a diff (§3), so the unified text ANA-9 §4.3 asks for
/// is synthesized here — the same function the `fs/write_text_file` interception uses, so both
/// sources of an `edit_proposal` produce the same shape.
#[must_use]
pub fn diffs_of(tool_call_id: &str, content: Option<&Value>) -> Vec<EditProposalEvent> {
    let Some(entries) = content.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("diff"))
        .filter_map(|entry| {
            let path = entry.get("path").and_then(Value::as_str)?;
            let old = entry
                .get("oldText")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let new = entry
                .get("newText")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(EditProposalEvent {
                tool_call_id: Some(tool_call_id.to_owned()),
                path: path.to_owned(),
                diff: unified_diff(path, old, new),
                // Filled from the paired permission answer, or defaulted by the recorder for a
                // write that reached the filesystem with no request attached (§4.3).
                accepted: None,
            })
        })
        .collect()
}

/// `tool_result.output`: the `content[]` text blocks joined by newlines, else `rawOutput`.
#[must_use]
pub fn output_of(content: Option<&Value>, raw_output: Option<&Value>) -> Option<Value> {
    if let Some(entries) = content.and_then(Value::as_array) {
        let text: Vec<&str> = entries
            .iter()
            .filter_map(|entry| match entry.get("type").and_then(Value::as_str) {
                Some("content") => entry
                    .get("content")
                    .and_then(|content| content.get("text"))
                    .and_then(Value::as_str),
                Some("text") => entry.get("text").and_then(Value::as_str),
                _ => None,
            })
            .collect();
        if !text.is_empty() {
            return Some(Value::String(text.join("\n")));
        }
    }
    raw_output.cloned()
}

/// `locations[]` → [`ToolLocation`]s, skipping entries that name no path.
#[must_use]
pub fn locations_of(locations: Option<&Value>) -> Vec<ToolLocation> {
    let Some(entries) = locations.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            Some(ToolLocation {
                path: entry.get("path").and_then(Value::as_str)?.to_owned(),
                line: entry
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
            })
        })
        .collect()
}

/// A wire `stopReason` → [`StopReason`]; an unknown one is `end_turn` plus a `warn!`.
///
/// `StopReason` is `#[non_exhaustive]` on the wire, and a turn that ended for a reason this build
/// does not know still ended: refusing to map it would leave the turn open forever.
#[must_use]
pub fn stop_reason(text: &str) -> StopReason {
    StopReason::ALL
        .iter()
        .copied()
        .find(|reason| reason.as_str() == text)
        .unwrap_or_else(|| {
            tracing::warn!(
                stop_reason = text,
                "unknown stop reason; recorded as end_turn"
            );
            StopReason::EndTurn
        })
}

/// `agent_message_chunk` / `agent_thought_chunk` → a text chunk, or `other` when the chunk is not
/// text.
///
/// ANA-9 §4.3's `assistant_text` / `thought` payloads have a `text` column and nothing else, so an
/// image or resource chunk has nowhere to go and lands verbatim in `other` rather than being
/// flattened into an empty string.
fn chunk(update: &Value, kind: &str, wrap: fn(TextChunk) -> DriverEvent) -> DriverEvent {
    let content = update.get("content");
    let text = content
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str);
    match text {
        Some(text) => wrap(TextChunk {
            text: text.to_owned(),
            message_id: update
                .get("messageId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }),
        None => other(kind, body_of(update)),
    }
}

/// `tool_call` → the call, its diffs, and a result when the call arrives already terminal.
fn tool_call(update: &Value) -> Vec<DriverEvent> {
    let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
        return vec![other(update_kind(update), body_of(update))];
    };
    let mut events = vec![DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: id.to_owned(),
        title: update
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        tool_kind: tool_kind(update.get("kind").and_then(Value::as_str)),
        input: update.get("rawInput").cloned().unwrap_or(Value::Null),
        locations: locations_of(update.get("locations")),
    })];
    events.extend(
        diffs_of(id, update.get("content"))
            .into_iter()
            .map(DriverEvent::EditProposal),
    );
    if let Some(status) = terminal_status(update.get("status").and_then(Value::as_str)) {
        events.push(DriverEvent::ToolResult(result_of(update, id, status)));
    }
    events
}

/// `tool_call_update` → its diffs, then a result when the status is terminal.
///
/// A title-, kind- or locations-only update produces nothing: ANA-9 §4.3 has no "tool call
/// updated" kind, and inventing one would put rows in the log that no replay can render.
fn tool_call_update(update: &Value) -> Vec<DriverEvent> {
    let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
        return vec![other(update_kind(update), body_of(update))];
    };
    let mut events: Vec<DriverEvent> = diffs_of(id, update.get("content"))
        .into_iter()
        .map(DriverEvent::EditProposal)
        .collect();
    if let Some(status) = terminal_status(update.get("status").and_then(Value::as_str)) {
        events.push(DriverEvent::ToolResult(result_of(update, id, status)));
    }
    events
}

/// The `tool_result` row a terminal `tool_call` / `tool_call_update` produces.
fn result_of(update: &Value, id: &str, status: ToolResultStatus) -> ToolResultEvent {
    ToolResultEvent {
        tool_call_id: id.to_owned(),
        status,
        output: output_of(update.get("content"), update.get("rawOutput")),
        locations: locations_of(update.get("locations")),
        // Only a row `htui` synthesized carries one (§4.3 "Tool-call terminal states"); this row
        // is the agent's own report.
        terminal_reason: None,
    }
}

/// `plan` → the complete entry list; an entry whose status or priority is unknown is dropped.
///
/// A `plan` update is always the whole plan (§6.1: "replace, never append"), so a dropped entry is
/// visibly missing from the next render rather than silently stale — and the `warn!` says which.
fn plan(update: &Value) -> PlanEvent {
    let entries = update
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let content = entry.get("content").and_then(Value::as_str)?;
                    let status = wire_value(PlanEntryStatus::ALL, entry.get("status"));
                    let priority = wire_value(PlanEntryPriority::ALL, entry.get("priority"));
                    match (status, priority) {
                        (Some(status), Some(priority)) => Some(PlanEntry {
                            content: content.to_owned(),
                            status,
                            priority,
                        }),
                        _ => {
                            tracing::warn!(
                                entry = content,
                                "plan entry has an unknown status or priority; dropped"
                            );
                            None
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    PlanEvent { entries }
}

/// The member of a closed wire vocabulary a JSON string names.
fn wire_value<T: Copy + WireText>(all: &'static [T], value: Option<&Value>) -> Option<T> {
    let text = value?.as_str()?;
    all.iter()
        .copied()
        .find(|member| member.wire_text() == text)
}

/// The `as_str` of a `wire_enum!` vocabulary, as a trait so one lookup serves every one of them.
trait WireText {
    /// The JSON text of this value.
    fn wire_text(self) -> &'static str;
}

impl WireText for PlanEntryStatus {
    fn wire_text(self) -> &'static str {
        self.as_str()
    }
}

impl WireText for PlanEntryPriority {
    fn wire_text(self) -> &'static str {
        self.as_str()
    }
}

/// The `other` row of §6.1: the transport's own name for the update plus the verbatim body.
fn other(update: &str, body: Value) -> DriverEvent {
    DriverEvent::Other(crate::event::OtherEvent {
        update: update.to_owned(),
        body,
    })
}

/// An integer field of an update object.
fn int_at(update: &Value, key: &str) -> Option<i64> {
    update.get(key).and_then(Value::as_i64)
}

/// An object body for a synthesized `other` row.
#[must_use]
pub fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    let mut map = Map::new();
    for (key, value) in fields {
        map.insert(key.to_owned(), value);
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map_one(update: Value) -> Vec<DriverEvent> {
        Mapper::new().map(&update)
    }

    #[test]
    fn an_assistant_chunk_carries_its_text_and_grouping_key() {
        let events = map_one(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" },
            "messageId": "msg_1"
        }));
        assert_eq!(
            events,
            vec![DriverEvent::AssistantChunk(TextChunk {
                text: "hello".to_owned(),
                message_id: Some("msg_1".to_owned()),
            })]
        );
    }

    #[test]
    fn a_thought_chunk_maps_the_same_way_and_may_have_no_message_id() {
        let events = map_one(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": { "type": "text", "text": "thinking" }
        }));
        assert_eq!(
            events,
            vec![DriverEvent::ThoughtChunk(TextChunk {
                text: "thinking".to_owned(),
                message_id: None,
            })]
        );
    }

    /// ANA-9 §4.3's `assistant_text` payload has a `text` column and nothing else.
    #[test]
    fn a_non_text_chunk_lands_in_other_rather_than_becoming_empty_text() {
        let events = map_one(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "image", "data": "…", "mimeType": "image/png" }
        }));
        match &events[..] {
            [DriverEvent::Other(row)] => {
                assert_eq!(row.update, "agent_message_chunk");
                assert_eq!(row.body["content"]["type"], "image");
                assert!(
                    row.body.get("sessionUpdate").is_none(),
                    "the tag is the name"
                );
            }
            other => panic!("expected one `other` row: {other:?}"),
        }
    }

    #[test]
    fn a_tool_call_carries_its_id_kind_input_and_locations() {
        let events = map_one(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "Read src/main.rs",
            "kind": "read",
            "status": "pending",
            "rawInput": { "path": "src/main.rs" },
            "locations": [ { "path": "src/main.rs", "line": 12 }, { "line": 3 } ]
        }));
        assert_eq!(
            events,
            vec![DriverEvent::ToolCall(ToolCallEvent {
                tool_call_id: "call_1".to_owned(),
                title: "Read src/main.rs".to_owned(),
                tool_kind: ToolKind::Read,
                input: json!({ "path": "src/main.rs" }),
                locations: vec![ToolLocation {
                    path: "src/main.rs".to_owned(),
                    line: Some(12),
                }],
            })],
            "a location without a path is skipped, and a pending call has no result"
        );
    }

    #[test]
    fn every_acp_tool_kind_decodes_including_switch_mode() {
        for kind in ToolKind::ALL {
            assert_eq!(tool_kind(Some(kind.as_str())), *kind);
        }
        assert_eq!(tool_kind(Some("switch_mode")), ToolKind::SwitchMode);
        assert_eq!(tool_kind(Some("teleport")), ToolKind::Other);
        assert_eq!(tool_kind(None), ToolKind::Other);
    }

    #[test]
    fn only_a_terminal_tool_call_update_produces_a_result() {
        assert_eq!(
            terminal_status(Some("completed")),
            Some(ToolResultStatus::Completed)
        );
        assert_eq!(
            terminal_status(Some("failed")),
            Some(ToolResultStatus::Failed)
        );
        assert_eq!(terminal_status(Some("in_progress")), None);
        assert_eq!(terminal_status(Some("pending")), None);
        assert_eq!(terminal_status(None), None);

        let running = map_one(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "in_progress",
            "title": "still going"
        }));
        assert!(running.is_empty(), "no ANA-9 kind for a mid-flight update");

        let done = map_one(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "completed",
            "content": [ { "type": "content", "content": { "type": "text", "text": "ok" } } ]
        }));
        assert_eq!(
            done,
            vec![DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id: "call_1".to_owned(),
                status: ToolResultStatus::Completed,
                output: Some(json!("ok")),
                locations: Vec::new(),
                terminal_reason: None,
            })]
        );
    }

    #[test]
    fn a_diff_content_block_becomes_an_edit_proposal_before_the_result() {
        let events = map_one(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_2",
            "status": "completed",
            "content": [
                { "type": "diff", "path": "src/a.rs", "oldText": "one\n", "newText": "two\n" }
            ]
        }));
        match &events[..] {
            [
                DriverEvent::EditProposal(edit),
                DriverEvent::ToolResult(result),
            ] => {
                assert_eq!(edit.tool_call_id.as_deref(), Some("call_2"));
                assert_eq!(edit.path, "src/a.rs");
                assert!(edit.diff.contains("-one"), "{}", edit.diff);
                assert!(edit.diff.contains("+two"), "{}", edit.diff);
                assert_eq!(edit.accepted, None, "no answer is paired with it yet");
                assert_eq!(result.status, ToolResultStatus::Completed);
            }
            other => panic!("expected a proposal then a result: {other:?}"),
        }
    }

    /// A new file: `oldText` is null on the wire (§3), so the diff is a run of `+` lines.
    #[test]
    fn a_new_file_diff_block_has_no_old_text() {
        let edits = diffs_of(
            "call_3",
            Some(&json!([
                { "type": "diff", "path": "new.rs", "oldText": null, "newText": "fresh\n" }
            ])),
        );
        assert_eq!(edits.len(), 1);
        assert!(edits[0].diff.contains("+fresh"), "{}", edits[0].diff);
        assert!(!edits[0].diff.contains("-fresh"), "{}", edits[0].diff);
    }

    #[test]
    fn a_plan_is_the_complete_entry_list_and_drops_what_it_cannot_read() {
        let events = map_one(json!({
            "sessionUpdate": "plan",
            "entries": [
                { "content": "first", "status": "in_progress", "priority": "high" },
                { "content": "second", "status": "pending", "priority": "low" },
                { "content": "broken", "status": "sideways", "priority": "high" }
            ]
        }));
        assert_eq!(
            events,
            vec![DriverEvent::Plan(PlanEvent {
                entries: vec![
                    PlanEntry {
                        content: "first".to_owned(),
                        status: PlanEntryStatus::InProgress,
                        priority: PlanEntryPriority::High,
                    },
                    PlanEntry {
                        content: "second".to_owned(),
                        status: PlanEntryStatus::Pending,
                        priority: PlanEntryPriority::Low,
                    },
                ],
            })]
        );
    }

    /// ACP reports the session's cumulative cost; the row carries the delta (§7).
    #[test]
    fn usage_costs_are_deltas_of_the_cumulative_total() {
        let mut mapper = Mapper::new();
        let update = |amount: f64, used: i64| {
            json!({
                "sessionUpdate": "usage_update",
                "used": used,
                "size": 200_000,
                "cost": { "amount": amount, "currency": "USD" }
            })
        };

        let first = mapper.map(&update(0.000_100, 1_000));
        let DriverEvent::Usage(first) = &first[0] else {
            panic!("expected usage: {first:?}")
        };
        assert_eq!(first.cost_micros, Some(100));
        assert_eq!(first.cost_micros_total, Some(100));
        assert_eq!(first.context_used, Some(1_000));
        assert_eq!(first.context_size, Some(200_000));
        assert_eq!(first.input_tokens, None, "ACP reports no token counts (§7)");

        let second = mapper.map(&update(0.000_350, 2_000));
        let DriverEvent::Usage(second) = &second[0] else {
            panic!("expected usage")
        };
        assert_eq!(second.cost_micros, Some(250), "delta, not the total");
        assert_eq!(second.cost_micros_total, Some(350));

        let third = mapper.map(&update(0.000_351, 2_100));
        let DriverEvent::Usage(third) = &third[0] else {
            panic!("expected usage")
        };
        assert_eq!(third.cost_micros, Some(1));
        assert_eq!(third.cost_micros_total, Some(351));
    }

    #[test]
    fn a_non_usd_cost_is_reported_as_it_arrived_and_sums_nothing() {
        let events = map_one(json!({
            "sessionUpdate": "usage_update",
            "used": 10,
            "size": 100,
            "cost": { "amount": 1.5, "currency": "EUR" }
        }));
        let DriverEvent::Usage(usage) = &events[0] else {
            panic!("expected usage")
        };
        assert_eq!(usage.cost_micros, None);
        assert_eq!(usage.cost_amount, Some(1.5));
        assert_eq!(usage.cost_currency.as_deref(), Some("EUR"));
    }

    #[test]
    fn a_user_message_chunk_is_dropped() {
        assert!(
            map_one(json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": "replayed prompt" }
            }))
            .is_empty(),
            "replay must not manufacture a second prompt row"
        );
    }

    #[test]
    fn the_four_named_session_updates_land_in_other_verbatim() {
        for kind in [
            "available_commands_update",
            "current_mode_update",
            "config_option_update",
            "session_info_update",
        ] {
            let events = map_one(json!({ "sessionUpdate": kind, "payload": { "n": 1 } }));
            match &events[..] {
                [DriverEvent::Other(row)] => {
                    assert_eq!(row.update, kind);
                    assert_eq!(row.body, json!({ "payload": { "n": 1 } }));
                }
                other => panic!("expected one `other` row for {kind}: {other:?}"),
            }
        }
    }

    /// The reason this module reads JSON instead of the SDK enum: the adapter ships kinds the
    /// schema does not know, and they must be stored rather than refused.
    #[test]
    fn an_update_the_schema_never_heard_of_is_stored_in_other() {
        let events = map_one(json!({
            "sessionUpdate": "subagent_spawned",
            "subagentId": "sub_1",
            "prompt": "go"
        }));
        match &events[..] {
            [DriverEvent::Other(row)] => {
                assert_eq!(row.update, "subagent_spawned");
                assert_eq!(row.body["subagentId"], "sub_1");
            }
            other => panic!("expected one `other` row: {other:?}"),
        }
    }

    #[test]
    fn an_update_with_no_tag_is_named_rather_than_lost() {
        let events = map_one(json!({ "whatever": true }));
        match &events[..] {
            [DriverEvent::Other(row)] => assert_eq!(row.update, "<missing>"),
            other => panic!("expected one `other` row: {other:?}"),
        }
    }

    #[test]
    fn stop_reasons_map_one_to_one_and_an_unknown_one_ends_the_turn() {
        for reason in StopReason::ALL {
            assert_eq!(stop_reason(reason.as_str()), *reason);
        }
        assert_eq!(stop_reason("exploded"), StopReason::EndTurn);
    }

    #[test]
    fn output_prefers_the_content_text_over_the_raw_output() {
        let content = json!([
            { "type": "content", "content": { "type": "text", "text": "one" } },
            { "type": "content", "content": { "type": "text", "text": "two" } }
        ]);
        assert_eq!(
            output_of(Some(&content), Some(&json!({ "ignored": true }))),
            Some(json!("one\ntwo"))
        );
        assert_eq!(
            output_of(None, Some(&json!({ "exit": 0 }))),
            Some(json!({ "exit": 0 })),
            "no content leaves rawOutput"
        );
        assert_eq!(output_of(None, None), None);
    }
}
