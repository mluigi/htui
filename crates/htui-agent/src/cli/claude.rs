//! `docs/ANA-4.md` §6.2 as code: one `claude -p --output-format stream-json` line in, zero or more
//! [`DriverEvent`]s out (plan T51; blueprint B.4).
//!
//! JSON is read by key and never through a typed decode, for `acp/map.rs:1-14`'s reason carried one
//! step further: the CLI stream has **no schema at all**. There is no negotiated version to branch
//! on and no published envelope list, so a `claude` release may add a kind between two Tuesdays —
//! and one did before this file existed. The nine live probes that produced this module's fixtures
//! recorded **six `system` subtypes §6.2 never names** (`status`, `thinking_tokens`, and three
//! `task_*` from the subagent run), on the first run, without warning. The wildcard arm is
//! load-bearing.
//!
//! Carried state, in one sentence: the id of the message currently streaming, which messages
//! already streamed their text as deltas, the previous `result`'s five cumulative totals, and a
//! rate-limit blob waiting for the turn's `usage` row.
//!
//! # What the probes changed here
//!
//! Four of the plan's "Probe findings" are decisions this file makes differently from the way the
//! plan first wrote them, each because a fixture said so:
//!
//! - **F-4.** `modelUsage[*]` token counts are **cumulative across the session**, while
//!   `result.usage` is per-turn — the inversion of what the names suggest. Every figure this module
//!   reports is a *delta* against the previous `result`, which is what makes `run_step.usage` a
//!   plain sum (criterion 7).
//! - **F-7.** A turn's thinking block and its reply share one `message.id`, so the chunk grouping
//!   key is `(message.id, content-block index)`. `message.id` alone would fold a thought into a
//!   reply.
//! - **F-8.** `result.subtype` is a label, not a verdict: an unauthenticated run reports
//!   `subtype: "success"` with `is_error: true`. The outcome is read from `is_error` and
//!   `terminal_reason`.
//! - **F-6.** A `thinking` block's text is the empty string beside a signature — thinking is
//!   signalled and not disclosed. The event is still emitted, and the signature never becomes its
//!   text.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::driver::PermissionRequestId;
use crate::event::{
    DoneEvent, DriverEvent, ErrorEvent, OtherEvent, PermissionAnswerEvent, StopReason, TextChunk,
    ToolCallEvent, ToolKind, ToolLocation, ToolResultEvent, ToolResultStatus, UsageEvent,
};
use crate::launch::UsageScope;
use crate::record::AnsweredBy;

/// The `type` reported for a line that carries none.
const MISSING_KIND: &str = "<missing>";

/// One micro-dollar, as the multiplier from the whole US dollars `total_cost_usd` reports.
const MICROS_PER_UNIT: f64 = 1_000_000.0;

/// The terminal reason a turn stopped because something cancelled it mid-stream.
///
/// Measured, not assumed (plan F-2): a SIGINT after the stdin close brings a real terminal
/// envelope, so the driver gets a real `done` — but that envelope is error-shaped
/// (`subtype: "error_during_execution"`, `is_error: true`) and this key is the only thing on the
/// line that tells a cancellation from a failure. Reading it, rather than remembering that *we*
/// sent the signal, is also what makes a turn cancelled from somewhere else report honestly.
const ABORTED: &str = "aborted_streaming";

/// Per-session mapping state.
#[derive(Debug)]
pub struct Mapper {
    /// `agent.settings.usage.scope`: what the token sum means, written onto every `usage` row.
    scope: UsageScope,
    /// The `message.id` of the message currently streaming, from `stream_event/message_start`.
    message: Option<String>,
    /// Chunk keys whose text already arrived as deltas, so the assembled envelope is not read
    /// twice (blueprint H-8). Keyed the way the chunks are: `(message.id, block index)`.
    streamed: BTreeSet<String>,
    /// The previous `result`'s five cumulative figures, which is what makes every reported figure
    /// a delta (F-4).
    last: Totals,
    /// The last `rate_limit_event` blob, held for the turn's single `usage` row.
    ///
    /// It cannot travel as a `usage` row of its own: that would be a usage report arriving
    /// mid-turn on the one transport whose `DriverCaps.usage_mid_turn` is `false` (D91).
    pending_quota: Option<Value>,
    /// `tool_use_id`s already answered by a `permission_answer` row.
    ///
    /// The CLI reports a refusal **twice** — once as `system/permission_denied` when it happens,
    /// and again in the terminal `result`'s `permission_denials[]` (measured, plan F-12b). Both are
    /// the same refusal of the same call, so the second is suppressed: two rows would make the
    /// transcript claim a tool was refused twice, and `idx_session_event_tool` joins on the id that
    /// both carry.
    answered: BTreeSet<String>,
}

/// The five cumulative figures a `result` carries.
#[derive(Debug, Default, Clone, Copy)]
struct Totals {
    cost_micros: i64,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

impl Mapper {
    /// A mapper for one session.
    #[must_use]
    pub fn new(scope: UsageScope) -> Self {
        Self {
            scope,
            message: None,
            streamed: BTreeSet::new(),
            last: Totals::default(),
            pending_quota: None,
            answered: BTreeSet::new(),
        }
    }

    /// Maps one stdout line.
    ///
    /// `system/init` maps to `other` here even though the supervisor intercepts that kind before
    /// `map` and emits the session banner instead: the arm exists so a fixture replay shows the
    /// line where it arrived, which is what makes the transcripts a regression net for the whole
    /// stream rather than for the part `htui` happens to route through this function.
    pub fn map(&mut self, line: &Value) -> Vec<DriverEvent> {
        let (kind, subtype) = kind_of(line);
        match (kind, subtype) {
            ("stream_event", _) => self.stream_event(line),
            ("assistant", _) => self.assistant(line),
            ("user", _) => tool_results(line),
            ("rate_limit_event", _) => {
                // Verbatim and unparsed, then held: normalizing the vendor's shape is
                // `htui_core::model::quota`'s job, selected by the row's declared source rather
                // than by the agent's name (D66). The `is_object` filter is the only judgement
                // made here, for `acp/map.rs:116-120`'s reason — a scalar under that key is not
                // the document §7 describes, and storing one would hand the normalizer a shape it
                // would have to reject anyway.
                self.pending_quota = line
                    .get("rate_limit_info")
                    .filter(|blob| blob.is_object())
                    .cloned();
                vec![other(line)]
            }
            // §6.2's row, as written: a retry the CLI announces is a transport-level error the
            // chat tab should show. Not observed in any recorded run (blueprint H-20).
            ("system", Some("api_retry")) => vec![DriverEvent::Error(ErrorEvent {
                code: "api_retry".to_owned(),
                message: text_at(line, "message").unwrap_or_default(),
            })],
            // **The denial, where it happened.** D85 named two sources and this is the live one:
            // the CLI announces the refusal mid-turn, in order, before whatever the agent does
            // next. The terminal `result` repeats it in `permission_denials[]` (see
            // [`Mapper::result`]), so the same refusal arrives twice and the second one is
            // suppressed by [`Mapper::answered`] — one refusal is one row, and the one that is
            // kept is the one in the position it actually occupied. A transcript that reported
            // three denials at the end of a turn instead of at the three moments they happened
            // would be a worse account of the same turn.
            ("system", Some("permission_denied")) => self
                .denial(line.get("tool_use_id").and_then(Value::as_str))
                .into_iter()
                .collect(),
            ("result", _) => self.result(line),
            // `system/init`, the two hook kinds, `status`, `thinking_tokens`, the three `task_*`,
            // `compact_boundary`, plugins, and every kind a future release ships ahead of this
            // file.
            _ => vec![other(line)],
        }
    }

    /// `stream_event`: the partial-message channel `--include-partial-messages` turns on.
    ///
    /// Only three of its nine observed shapes produce rows. The rest — `message_start`,
    /// `message_stop`, `message_delta`, `content_block_start`, `content_block_stop` — are
    /// bookkeeping, and `signature_delta` is a cryptographic block signature rather than words
    /// (F-6): appending it to a thought would put an opaque kilobyte in the transcript where a
    /// thought should be.
    fn stream_event(&mut self, line: &Value) -> Vec<DriverEvent> {
        let event = line.get("event").unwrap_or(&Value::Null);
        let event_type = event.get("type").and_then(Value::as_str);
        match event_type {
            Some("message_start") => {
                self.message = event
                    .get("message")
                    .and_then(|message| message.get("id"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                Vec::new()
            }
            Some("content_block_delta") => {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                let index = event.get("index").and_then(Value::as_u64);
                let key = self.chunk_key(index);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        self.streamed.insert(key.clone().unwrap_or_default());
                        vec![DriverEvent::AssistantChunk(TextChunk {
                            text: text_at(delta, "text").unwrap_or_default(),
                            message_id: key,
                        })]
                    }
                    Some("thinking_delta") => {
                        self.streamed.insert(key.clone().unwrap_or_default());
                        vec![DriverEvent::ThoughtChunk(TextChunk {
                            text: text_at(delta, "thinking").unwrap_or_default(),
                            message_id: key,
                        })]
                    }
                    // `signature_delta` (F-6) and `input_json_delta`: the whole `tool_use` arrives
                    // on the `assistant` envelope, so the partial JSON has nowhere useful to go.
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// `assistant`: the assembled message, one event per content block.
    ///
    /// Text and thinking are emitted **only** when they did not already arrive as deltas
    /// (blueprint H-8) — `--include-partial-messages` sends both, and mapping both would double
    /// every reply in the transcript.
    fn assistant(&mut self, line: &Value) -> Vec<DriverEvent> {
        let message = line.get("message").unwrap_or(&Value::Null);
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let blocks = message
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                let key = chunk_key(id.as_deref(), u64::try_from(index).ok());
                let already = self.streamed.contains(key.as_deref().unwrap_or_default());
                match block.get("type").and_then(Value::as_str) {
                    Some("text") if !already => Some(DriverEvent::AssistantChunk(TextChunk {
                        text: text_at(block, "text").unwrap_or_default(),
                        message_id: key,
                    })),
                    Some("thinking") if !already => Some(DriverEvent::ThoughtChunk(TextChunk {
                        // F-6: empty on every recorded run, and left empty. The `signature`
                        // beside it is not the thought.
                        text: text_at(block, "thinking").unwrap_or_default(),
                        message_id: key,
                    })),
                    Some("tool_use") => {
                        let input = block.get("input").cloned().unwrap_or(Value::Null);
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        Some(DriverEvent::ToolCall(ToolCallEvent {
                            tool_call_id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            title: name.to_owned(),
                            tool_kind: tool_kind(name),
                            locations: locations_of(&input),
                            input,
                        }))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// The terminal `result`: the turn's denials, its cost, its failure if it had one, and its end.
    ///
    /// In that order, and the order is a contract. `Done` is last so that a per-run cap breach
    /// detected on the `usage` row finds the `done` **behind** it and can withhold it (milestone
    /// 7's D69, blueprint H-4); the reverse would let a turn close before its own cost was known.
    fn result(&mut self, line: &Value) -> Vec<DriverEvent> {
        let mut events = self.denials_of(line);
        let terminal = line.get("terminal_reason").and_then(Value::as_str);
        let cancelled = terminal == Some(ABORTED);

        // A cancelled turn reports `total_cost_usd: 0` and an empty `modelUsage` (F-2), which is
        // an absence of measurement rather than a measurement of zero — so it gets no `usage` row
        // at all. Writing one would put a zero in `run_step.usage` that nothing spent.
        if !cancelled {
            events.push(DriverEvent::Usage(self.usage(line)));
        }

        // F-8: the verdict is `is_error`, never `subtype`. An unauthenticated run reports
        // `subtype: "success"` with `is_error: true` and the reason in `result` — the one place
        // it exists.
        if line.get("is_error").and_then(Value::as_bool) == Some(true) && !cancelled {
            events.push(DriverEvent::Error(ErrorEvent {
                code: terminal
                    .or_else(|| line.get("subtype").and_then(Value::as_str))
                    .unwrap_or("error")
                    .to_owned(),
                message: error_message(line),
            }));
        }

        events.push(DriverEvent::Done(DoneEvent {
            stop_reason: stop_reason(terminal, line.get("subtype").and_then(Value::as_str)),
        }));
        events
    }

    /// `result` → `usage`, every figure a delta of the session's cumulative report (D86, F-4).
    fn usage(&mut self, line: &Value) -> UsageEvent {
        let models = line.get("modelUsage").and_then(Value::as_object);
        let sums = models.map(|models| Totals {
            cost_micros: 0,
            input: sum_over(models, "inputTokens"),
            output: sum_over(models, "outputTokens"),
            cache_read: sum_over(models, "cacheReadInputTokens"),
            cache_write: sum_over(models, "cacheCreationInputTokens"),
        });

        let mut event = UsageEvent {
            usage_scope: Some(self.scope.as_str().to_owned()),
            // The stream carries no occupancy figure: `modelUsage[*].contextWindow` is a model's
            // window, not this turn's use of it, and reporting the one as the other would fill the
            // Settings cell with a constant.
            context_used: None,
            context_size: None,
            // USD only; there is no other currency on this wire to disambiguate.
            cost_amount: None,
            cost_currency: None,
            // The last `rate_limit_event` of the turn rides the turn's one `usage` row, which is
            // where `Recorder::latch_quota` reads it.
            quota: self.pending_quota.take(),
            ..UsageEvent::default()
        };

        if let Some(total_usd) = line.get("total_cost_usd").and_then(Value::as_f64) {
            let total = saturating_micros(total_usd);
            event.cost_micros = Some(total.saturating_sub(self.last.cost_micros));
            event.cost_micros_total = Some(total);
            self.last.cost_micros = total;
        }

        // An absent or empty `modelUsage` answers `null` rather than `0` — a harness that sends
        // none yields the five-null shape `usage_deltas_sum_to_step_usage` expects, and a real
        // turn that reported none did not report a zero.
        if let Some(sums) = sums.filter(|_| models.is_some_and(|models| !models.is_empty())) {
            event.input_tokens = Some(sums.input.saturating_sub(self.last.input));
            event.output_tokens = Some(sums.output.saturating_sub(self.last.output));
            event.cache_read_tokens = Some(sums.cache_read.saturating_sub(self.last.cache_read));
            event.cache_write_tokens = Some(sums.cache_write.saturating_sub(self.last.cache_write));
            self.last.input = sums.input;
            self.last.output = sums.output;
            self.last.cache_read = sums.cache_read;
            self.last.cache_write = sums.cache_write;
        }
        event
    }

    /// The chunk grouping key: `<message.id>#<block index>` (F-7).
    ///
    /// Not `message.id` alone. A turn's thinking block and its reply arrive under **one**
    /// `message.id`, so that key would make the recorder read them as one contiguous run and fold
    /// a thought into an answer — losing the thought as a kind and corrupting the reply as text.
    /// The composite still starts with the message id, so a message change still flushes the open
    /// run (§4.1 trigger 2).
    fn chunk_key(&self, index: Option<u64>) -> Option<String> {
        chunk_key(self.message.as_deref(), index)
    }
}

/// The `(type, subtype)` of a line, `("<missing>", None)` when there is no `type`.
#[must_use]
pub fn kind_of(line: &Value) -> (&str, Option<&str>) {
    (
        line.get("type")
            .and_then(Value::as_str)
            .unwrap_or(MISSING_KIND),
        line.get("subtype").and_then(Value::as_str),
    )
}

/// The `other.update` text: `type`, or `type/subtype` when there is one.
#[must_use]
pub fn update_name(line: &Value) -> String {
    match kind_of(line) {
        (kind, Some(subtype)) => format!("{kind}/{subtype}"),
        (kind, None) => kind.to_owned(),
    }
}

/// The line minus `type` and `subtype`: §6.2's verbatim body.
#[must_use]
pub fn body_of(line: &Value) -> Value {
    match line {
        Value::Object(fields) => {
            let mut body = fields.clone();
            body.remove("type");
            body.remove("subtype");
            Value::Object(body)
        }
        other => other.clone(),
    }
}

/// §6.2's tool-name → kind table.
///
/// The names are the **dialect's** tool vocabulary — a wire fact of the stream this file parses,
/// exactly like a `sessionUpdate` tag — and not an agent's name, which is what keeps `R-AGT-5`
/// intact: a second CLI dialect declares its own `settings.cli.stream` and gets its own module.
///
/// `TodoWrite` is deliberately `Other` rather than something plan-shaped: §6.2 marks `plan` as not
/// produced by this transport, and mapping the tool that would be its source would contradict the
/// `DriverCaps` the chat tab draws its banner from.
#[must_use]
pub fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" | "NotebookRead" => ToolKind::Read,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => ToolKind::Edit,
        "Bash" | "BashOutput" | "KillShell" => ToolKind::Execute,
        "Glob" | "Grep" => ToolKind::Search,
        "WebFetch" | "WebSearch" => ToolKind::Fetch,
        _ => ToolKind::Other,
    }
}

/// `terminal_reason` (and `subtype` behind it) → the turn's [`StopReason`].
///
/// Every row here was measured rather than guessed (F-2, F-8, F-10): `completed`,
/// `aborted_streaming`, `api_error` and `budget_exhausted` are the four the probes produced, and
/// the fall-through is `end_turn` for `acp/map.rs:270-283`'s reason — a turn that ended for a
/// reason this build does not know still ended, and refusing to map it would leave it open forever.
#[must_use]
pub fn stop_reason(terminal: Option<&str>, subtype: Option<&str>) -> StopReason {
    match (terminal, subtype) {
        (Some(ABORTED), _) => StopReason::Cancelled,
        (_, Some("error_max_turns")) => StopReason::MaxTurnRequests,
        // `budget_exhausted` and `api_error` are both `end_turn` with an `error` row beside them
        // saying why: the `stop_reason` vocabulary is ACP's five and has no arm for either, and
        // the reason is already carried where a reader will look for it.
        _ => StopReason::EndTurn,
    }
}

/// `input.file_path | path | notebook_path` → one location; nothing else on this wire is a path.
///
/// No line is ever reported: the stream names none, and defaulting to 1 would claim the top of the
/// file as the subject of a call that touched the middle of it.
#[must_use]
pub fn locations_of(input: &Value) -> Vec<ToolLocation> {
    ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(|path| {
            vec![ToolLocation {
                path: path.to_owned(),
                line: None,
            }]
        })
        .unwrap_or_default()
}

/// A `tool_result` block's `content`: a string as is, an array's text blocks joined by newlines.
#[must_use]
pub fn output_of(content: &Value) -> Option<Value> {
    match content {
        Value::String(_) => Some(content.clone()),
        Value::Array(entries) => {
            let text: Vec<&str> = entries
                .iter()
                .filter_map(|entry| entry.get("text").and_then(Value::as_str))
                .collect();
            if text.is_empty() {
                // A structured result with no text block at all — a `tool_reference` list, say —
                // is kept whole rather than flattened to nothing.
                Some(content.clone())
            } else {
                Some(Value::String(text.join("\n")))
            }
        }
        Value::Null => None,
        other => Some(other.clone()),
    }
}

impl Mapper {
    /// `result.permission_denials[]` → the refusals **not already reported** as they happened.
    ///
    /// The terminal `result` repeats every refusal of the turn, and the live
    /// `system/permission_denied` envelope already produced a row for each at the moment it
    /// occurred, so in practice this arm is usually empty. It is not redundant: it is the only
    /// source for a refusal whose live envelope this build did not recognize, or that a future
    /// release stops emitting, and an end-of-turn row is a great deal better than none.
    fn denials_of(&mut self, result: &Value) -> Vec<DriverEvent> {
        let Some(entries) = result.get("permission_denials").and_then(Value::as_array) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| self.denial(entry.get("tool_use_id").and_then(Value::as_str)))
            .collect()
    }

    /// One refusal as a `permission_answer`, or nothing when this call was already answered.
    ///
    /// `by: Policy` — the transport's own `--permission-mode` refused, with no request `htui` could
    /// have answered (§4.3) — which is what makes the recorder stamp the row `role: htui` and keeps
    /// it distinguishable from an answer a human gave.
    ///
    /// The reason the CLI gives (`decision_reason`: "no approval surface in this session;
    /// permission request denied automatically") is deliberately **not** squeezed into the typed
    /// row: `PermissionAnswerEvent` has no free-text field, inventing one for a single dialect's
    /// prose would be the agent-name coupling `R-AGT-5` forbids in another costume, and the whole
    /// line survives as the row's `raw` under `retain_raw` where a reader can find it.
    fn denial(&mut self, tool_use_id: Option<&str>) -> Option<DriverEvent> {
        let id = tool_use_id?;
        if !self.answered.insert(id.to_owned()) {
            return None;
        }
        Some(DriverEvent::PermissionAnswer(PermissionAnswerEvent {
            // This transport announces no request, so the refused call's own id is what makes the
            // pair joinable (`PermissionAnswerEvent::request_id`).
            request_id: PermissionRequestId::new(id),
            tool_call_id: Some(id.to_owned()),
            option_id: None,
            by: AnsweredBy::Policy,
            // One call was refused, not every outstanding request settled at once.
            cancelled: false,
            denied: true,
        }))
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// The whole line as an `other` row: §6.2's wildcard, and the extension point that makes "every
/// event" hold without a schema change per release.
fn other(line: &Value) -> DriverEvent {
    DriverEvent::Other(OtherEvent {
        update: update_name(line),
        body: body_of(line),
    })
}

/// A `user` line's `tool_result` blocks; a `user` line without them maps to nothing.
///
/// §6.2 rows 1–2: the `prompt` and `follow_up` rows are `htui`'s own, written by the recorder from
/// what it sent. Reading them back off the wire would duplicate them — the same rule `acp/map.rs`
/// applies to `user_message_chunk`. It is also what keeps the CLI's `[Request interrupted by user]`
/// notice out of the transcript as agent speech: it arrives as a `user` message, and a `user`
/// message is not the agent talking.
fn tool_results(line: &Value) -> Vec<DriverEvent> {
    let Some(blocks) = line
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| {
            let id = block.get("tool_use_id").and_then(Value::as_str)?;
            Some(DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id: id.to_owned(),
                status: if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                    ToolResultStatus::Failed
                } else {
                    ToolResultStatus::Completed
                },
                output: block.get("content").and_then(output_of),
                locations: Vec::new(),
                // A failure the agent itself reported is not a row `htui` synthesized, and the key
                // exists to tell those apart (`event.rs:262-264`).
                terminal_reason: None,
            }))
        })
        .collect()
}

/// The failing turn's message: `result.error` when there is one, else the `result` text itself.
///
/// The unauthenticated run put "Not logged in · Please run /login" in `result` and nothing in
/// `error` (F-8), so a reader of `error` alone would report a failure with no reason.
fn error_message(line: &Value) -> String {
    if let Some(message) = text_at(line, "error") {
        return message;
    }
    if let Some(errors) = line.get("errors").and_then(Value::as_array)
        && !errors.is_empty()
    {
        let joined: Vec<&str> = errors.iter().filter_map(Value::as_str).collect();
        if !joined.is_empty() {
            return joined.join("\n");
        }
    }
    text_at(line, "result").unwrap_or_default()
}

/// A string field, skipping an empty one so a caller can fall through to the next source.
fn text_at(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

/// One token key summed across `modelUsage`'s per-model objects.
fn sum_over(models: &Map<String, Value>, key: &str) -> i64 {
    models
        .values()
        .filter_map(|model| model.get(key).and_then(Value::as_i64))
        .fold(0i64, i64::saturating_add)
}

/// `<message.id>#<index>`, or `None` when the line named no message.
fn chunk_key(message: Option<&str>, index: Option<u64>) -> Option<String> {
    let message = message?;
    Some(match index {
        Some(index) => format!("{message}#{index}"),
        None => message.to_owned(),
    })
}

/// US dollars as micros, clamping rather than wrapping or panicking.
///
/// Saturating throughout for `acp/map.rs:120-136`'s reason: the numbers come off a wire `htui`
/// does not control, and the test profile's overflow checks would turn an absurd pair into a panic.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the cast saturates at the i64 bounds, which is the clamp this function is for"
)]
fn saturating_micros(amount: f64) -> i64 {
    let micros = (amount * MICROS_PER_UNIT).round();
    if micros.is_nan() {
        return 0;
    }
    micros as i64
}
