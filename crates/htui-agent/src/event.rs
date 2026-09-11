//! The driver event model of `docs/ANA-4.md` §4.1, and its total map onto
//! [`htui_core::model::EventKind`].
//!
//! [`DriverEvent`] is exactly `EventKind` minus the two kinds `htui` alone authors — `prompt` and
//! `follow_up` — so [`From<&DriverEvent>`] for `EventKind` is total in both directions: twelve
//! variants, twelve reachable kinds (plan MOD-2 D2, as amended by MOD-2 D93).
//!
//! `permission_answer` was a third `htui`-authored kind until D93. A transport whose own policy
//! settled a request has an answer to report and nobody to ask, so it reports one — and the row's
//! `session_event.role`, taken from [`crate::record::AnsweredBy`], is what says who chose. A
//! prompt and a follow-up stay `htui`'s alone for a reason no decision can change: they are what
//! `htui` sends, so there is nothing for a transport to observe.
//!
//! Every payload struct here serialises into `session_event.payload`. The keys of ANA-9 §4.3 are
//! the minimum contract: a driver may **add** keys, never rename a documented one, which is why
//! the §7 usage reconciliation fields live on [`UsageEvent`] next to §4.3's five.
//!
//! Three things that are *not* an event live here for the same reason the events do: they belong
//! to no single transport. [`Stamp`] is how a capture time is made, [`SESSION_STARTED`] is the
//! `other.update` a session banner carries and [`TRANSPORT_CLOSED`] is the `error.code` a child
//! that died mid-turn leaves behind. Each has more than one writer and more than one reader, and a
//! copy per transport would be a string — or a clock — that can drift.

use chrono::{DateTime, SubsecRound, Utc};
use htui_core::model::EventKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver::PermissionRequestId;
use crate::record::AnsweredBy;

// ---------------------------------------------------------------------------------------------
// Vocabularies
// ---------------------------------------------------------------------------------------------

wire_enum!(
    /// `tool_call.tool_kind` (§4.3, ANA-4 §3): the ACP ten-value vocabulary.
    ///
    /// Ten, not the nine of the published docs page: `switch_mode` is in the schema and omitted
    /// from the prose. [`ToolKind::Other`] is the [`Default`], and mapping an unknown wire value
    /// onto it is the transport mapper's job (§6.1), not serde's — a value that reaches this enum
    /// was written by `htui` itself.
    #[derive(Default)]
    ToolKind {
        /// Reads a file or resource.
        Read => "read",
        /// Edits or writes a file.
        Edit => "edit",
        /// Deletes a file or resource.
        Delete => "delete",
        /// Moves or renames a file.
        Move => "move",
        /// Searches the workspace.
        Search => "search",
        /// Runs a command.
        Execute => "execute",
        /// Reasons without an external effect.
        Think => "think",
        /// Fetches a remote resource.
        Fetch => "fetch",
        /// Switches the agent's operating mode.
        SwitchMode => "switch_mode",
        /// Anything else, including every kind a future protocol revision adds.
        #[default]
        Other => "other",
    }
);

wire_enum!(
    /// `tool_result.status` (ANA-9 §4.3). ACP has no `cancelled` or `rejected` on the wire; a
    /// denied or cancelled call is recorded as [`ToolResultStatus::Failed`] carrying a
    /// [`TerminalReason`] (ANA-4 §4.3 "Tool-call terminal states").
    ToolResultStatus {
        /// The call finished successfully.
        Completed => "completed",
        /// The call failed, was rejected, or was cancelled.
        Failed => "failed",
    }
);

wire_enum!(
    /// The added `terminal_reason` key on a synthesized `tool_result` row (ANA-4 §4.3).
    ///
    /// Present only when `htui` invented the row so that replay is total; a real failure reported
    /// by the agent leaves it `None`.
    TerminalReason {
        /// The user or a policy rule denied the permission request for this call.
        Rejected => "rejected",
        /// The session was cancelled while the call was still outstanding.
        Cancelled => "cancelled",
    }
);

wire_enum!(
    /// `done.stop_reason` (§6.1): the ACP `StopReason`, all five values.
    StopReason {
        /// The agent finished its turn.
        EndTurn => "end_turn",
        /// The model's output token budget was reached.
        MaxTokens => "max_tokens",
        /// The per-turn request budget was reached.
        MaxTurnRequests => "max_turn_requests",
        /// The agent refused to answer.
        Refusal => "refusal",
        /// The turn was cancelled.
        Cancelled => "cancelled",
    }
);

wire_enum!(
    /// `permission_request.options[].kind` (ANA-4 §3): a closed ACP enum.
    ///
    /// The kind is a UI hint; the `option_id` is what is sent back. The decoder accepts all four
    /// even though the `claude` adapter currently offers three (ANA-4 §4.3).
    PermissionOptionKind {
        /// Allow this call only.
        AllowOnce => "allow_once",
        /// Allow this call and remember the grant agent-side.
        AllowAlways => "allow_always",
        /// Reject this call only.
        RejectOnce => "reject_once",
        /// Reject this call and remember the refusal agent-side.
        RejectAlways => "reject_always",
    }
);

wire_enum!(
    /// `plan.entries[].status` (ANA-9 §4.3).
    PlanEntryStatus {
        /// Not started.
        Pending => "pending",
        /// Being worked on.
        InProgress => "in_progress",
        /// Finished.
        Completed => "completed",
    }
);

wire_enum!(
    /// `plan.entries[].priority` (ANA-9 §4.3).
    PlanEntryPriority {
        /// Highest.
        High => "high",
        /// Middle.
        Medium => "medium",
        /// Lowest.
        Low => "low",
    }
);

// ---------------------------------------------------------------------------------------------
// The wire strings and the clock every transport shares
// ---------------------------------------------------------------------------------------------

/// `other.update` of the session banner (`docs/ANA-4.md` §4.4 "Session load and resume",
/// `docs/ANA-2.md` §4.8).
///
/// The agent-side session id has no column in ANA-9, so resuming a step is a query for this row.
/// Here rather than in a transport module because **every** transport opens a session and every
/// reader — the chat tab's transcript, the replay, the conformance suite's
/// `session_banner_is_first_other_row` — matches this one string: a second definition beside a
/// second transport would be a string that can drift from the readers'.
pub const SESSION_STARTED: &str = "session_started";

/// `error.code` of the row written when the transport ends before the turn does.
///
/// Shared for [`SESSION_STARTED`]'s reason: a child that dies mid-turn is every transport's
/// failure mode, and the row a reader recognizes must say the same thing whichever one it was.
pub const TRANSPORT_CLOSED: &str = "transport_closed";

/// How [`DriverEnvelope::at`] is stamped.
///
/// A seam rather than a bare `Utc::now()`, because ANA-4 §11 criterion 2 compares replayed rows
/// byte for byte and the conformance suite compares `at`. Production stamps the wall clock; a test
/// stamps `epoch + n ms`, exactly as the fake does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// `Utc::now()` truncated to microseconds — `TIMESTAMPTZ`'s resolution, the rule the recorder
    /// already follows for the rows it authors itself.
    Wall,
    /// `epoch + n ms` for the *n*-th envelope of the session.
    Fixed {
        /// The session's zero point.
        epoch: DateTime<Utc>,
    },
}

impl Stamp {
    /// The capture time of the *n*-th envelope.
    #[must_use]
    pub fn at(self, n: u64) -> DateTime<Utc> {
        match self {
            Self::Wall => Utc::now().trunc_subsecs(6),
            Self::Fixed { epoch } => {
                epoch + chrono::TimeDelta::milliseconds(i64::try_from(n).unwrap_or(i64::MAX))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The envelope and the event
// ---------------------------------------------------------------------------------------------

/// One wire event plus its optional verbatim message (§4.1).
#[derive(Debug, Clone, PartialEq)]
pub struct DriverEnvelope {
    /// The decoded event.
    pub event: DriverEvent,
    /// The verbatim wire message, populated only when `SessionSpec.retain_raw` is set; the
    /// recorder writes it to `session_event.raw`.
    pub raw: Option<Value>,
    /// Capture time on the executing box; `session_event.at`. Informational — replay order is
    /// `seq`, never this.
    pub at: DateTime<Utc>,
}

/// Exactly [`EventKind`] minus the two kinds `htui` alone authors (`prompt`, `follow_up`), so
/// `From<&DriverEvent> for EventKind` is total (§4.1, plan D93).
///
/// Twelve variants. Adding a thirteenth means adding an `EventKind`, which is an ANA-9 §4.3 schema
/// change — a migration — not a driver change. That is the boundary, and it is why a recognizable
/// wire shape gets a variant of its own only where an existing `EventKind` is its destination;
/// everything else is an [`OtherEvent`], which is §6.2's extension point.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    /// A chunk of agent text; the recorder coalesces a contiguous run into one `assistant_text`.
    AssistantChunk(TextChunk),
    /// A chunk of agent reasoning; coalesced the same way into one `thought`.
    ThoughtChunk(TextChunk),
    /// The agent called a tool.
    ToolCall(ToolCallEvent),
    /// A tool call reached a terminal state.
    ToolResult(ToolResultEvent),
    /// The agent proposes a file edit.
    EditProposal(EditProposalEvent),
    /// The agent asks permission to proceed.
    PermissionRequest(PermissionRequestEvent),
    /// The agent published its complete plan.
    Plan(PlanEvent),
    /// A token / cost usage report.
    Usage(UsageEvent),
    /// Something went wrong.
    Error(ErrorEvent),
    /// End of turn. Exactly one per turn, before the next accepted follow-up.
    Done(DoneEvent),
    /// Any protocol update not mapped above, stored verbatim.
    Other(OtherEvent),
    /// A permission request the transport's **own policy** settled, reported rather than asked
    /// (plan D93). The only variant whose row is not the agent's.
    PermissionAnswer(PermissionAnswerEvent),
}

impl From<&DriverEvent> for EventKind {
    /// The total map of §4.1. Every arm is a distinct kind, and no arm is one of the two kinds
    /// `htui` alone authors — `crates/htui-agent/tests/driver_contract.rs` asserts both.
    fn from(event: &DriverEvent) -> Self {
        match event {
            DriverEvent::AssistantChunk(_) => Self::AssistantText,
            DriverEvent::ThoughtChunk(_) => Self::Thought,
            DriverEvent::ToolCall(_) => Self::ToolCall,
            DriverEvent::ToolResult(_) => Self::ToolResult,
            DriverEvent::EditProposal(_) => Self::EditProposal,
            DriverEvent::PermissionRequest(_) => Self::PermissionRequest,
            DriverEvent::Plan(_) => Self::Plan,
            DriverEvent::Usage(_) => Self::Usage,
            DriverEvent::Error(_) => Self::Error,
            DriverEvent::Done(_) => Self::Done,
            DriverEvent::Other(_) => Self::Other,
            DriverEvent::PermissionAnswer(_) => Self::PermissionAnswer,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Payloads
// ---------------------------------------------------------------------------------------------

/// A text delta with the grouping key ACP supplies and ANA-9 §4.3 does not carry (§4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextChunk {
    /// The delta itself. The recorder concatenates a contiguous run into one `text` key.
    pub text: String,
    /// The transport's own grouping key (`messageId` over ACP, `message.id` over stream-json).
    /// A change flushes the open run even when the variant did not change (§4.1 trigger 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// A location a tool call touched: `tool_call.locations[]` / `tool_result.locations[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLocation {
    /// Absolute or workspace-relative path, as the agent reported it.
    pub path: String,
    /// 1-based line, when the agent named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// `tool_call` (§6.1). The recorder also copies `tool_call_id` into the
/// `session_event.tool_call_id` column, which is what `idx_session_event_tool` joins on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// The transport's tool-call id; pairs this call with its result, proposals and permission
    /// events.
    pub tool_call_id: String,
    /// Human-readable title, as the agent phrased it.
    pub title: String,
    /// The ten-value ACP kind.
    pub tool_kind: ToolKind,
    /// The call's raw input, verbatim.
    pub input: Value,
    /// Paths the call names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<ToolLocation>,
}

/// `tool_result` (§6.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEvent {
    /// The call this result belongs to.
    pub tool_call_id: String,
    /// `completed` or `failed`; the two-value ANA-9 §4.3 vocabulary.
    pub status: ToolResultStatus,
    /// The tool's output, scrubbed by the recorder before it is persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Paths the result names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<ToolLocation>,
    /// Set only on a row `htui` synthesized because the protocol emits none (ANA-4 §4.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_reason: Option<TerminalReason>,
}

/// `edit_proposal` (§6.1): one row per `(tool_call_id, path)` per step (ANA-4 §4.3 dedup rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditProposalEvent {
    /// The enclosing tool call, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The file the edit targets.
    pub path: String,
    /// The **unified** diff. ACP carries old/new full text; the transport synthesizes this.
    pub diff: String,
    /// `true` for an allow, `false` for a reject, `None` while a permission request is parked.
    pub accepted: Option<bool>,
}

/// One option offered by a `permission_request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    /// The `optionId` sent back on selection.
    pub id: String,
    /// The label to render.
    pub label: String,
    /// A UI hint only; the id is what decides.
    pub kind: PermissionOptionKind,
}

/// `permission_request` (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequestEvent {
    /// Correlates the request with the `permission_answer` `htui` writes.
    pub request_id: PermissionRequestId,
    /// The tool call being gated, when the transport named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Every option, in the order the agent offered them.
    pub options: Vec<PermissionOption>,
}

/// `permission_answer` (ANA-9 §4.3), as a **transport** reports it (plan D85, D93).
///
/// The kind `htui` writes for an answer a human gave, reached from the other side: a transport
/// whose `agent.settings.permission` policy — or whose own permission mode — refused a call has
/// the same four facts to record and nobody to ask for them. What it does **not** carry is the
/// row's role: that is [`AnsweredBy::role`]'s, so a transport cannot claim a user chose.
///
/// [`AnsweredBy::role`]: crate::record::AnsweredBy::role
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAnswerEvent {
    /// The request this answers. A transport with no permission channel (§4.3) never announced
    /// one, so this is the id of the call that was refused — the same string as
    /// [`tool_call_id`](Self::tool_call_id), which is what makes the pair joinable either way.
    pub request_id: PermissionRequestId,
    /// The tool call the answer settled, when the transport named one. Copied by the recorder into
    /// the `session_event.tool_call_id` column that `idx_session_event_tool` joins on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// The option chosen. `None` when the answer chose none: a denial and a cancellation both
    /// settle a request without picking anything.
    ///
    /// Serialised even when it is `None`, unlike the field above: `permission_answer` payloads are
    /// read by key (`crates/htui/src/ui/tabs/chat/transcript.rs`), the ANA-9 §4.3 key set is what
    /// a reader expects to find, and the row `htui` writes for a human's answer carries an
    /// explicit `null` here too. An answer with no option is a fact, not a missing field.
    #[serde(default)]
    pub option_id: Option<String>,
    /// Who chose, and therefore which `session_event.role` the row is written under.
    pub by: AnsweredBy,
    /// `true` when one answer settled every outstanding request at once (§4.3's cancellation).
    pub cancelled: bool,
    /// `true` when the answer refused the call. An **added** key, not a renamed one: it is what
    /// tells a policy denial apart from an answer somebody actually gave (§6.2, plan D85).
    ///
    /// `#[serde(default)]`, and the default is the honest reading of a row written before this
    /// milestone (plan D94): those rows carry no `denied` key at all, because `htui` only ever
    /// wrote an answer a human gave or a cancellation, and neither is a refusal. Without it
    /// [`replay`](crate::replay) could not decode a single `permission_answer` row `htui` itself
    /// authored — the field is required, the recorder never wrote it — which is the blocker that
    /// kept the kind on replay's `verbatim` arm.
    #[serde(default)]
    pub denied: bool,
}

/// One entry of a `plan` update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    /// What the entry says.
    pub content: String,
    /// Where it stands.
    pub status: PlanEntryStatus,
    /// How urgent the agent thinks it is.
    pub priority: PlanEntryPriority,
}

/// `plan` (§6.1). Always the **complete** list: replace, never append.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanEvent {
    /// The whole plan as of this update.
    pub entries: Vec<PlanEntry>,
}

/// `usage` (ANA-9 §4.3 plus the ANA-4 §7 reconciliation).
///
/// The five §4.3 keys are kept and nullable rather than renamed, because ACP does not stably emit
/// per-turn token counts; the transports add the keys they can actually supply. `cost_micros` is
/// the **delta** since the previous `usage` row of this step, which is what makes `run_step.usage`
/// a plain sum.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageEvent {
    /// `usage.input_tokens`; `None` over ACP.
    pub input_tokens: Option<i64>,
    /// `usage.output_tokens`; `None` over ACP.
    pub output_tokens: Option<i64>,
    /// `usage.cache_read_tokens`; `None` over ACP.
    pub cache_read_tokens: Option<i64>,
    /// `usage.cache_write_tokens`; `None` over ACP and over `agy` stream-json.
    pub cache_write_tokens: Option<i64>,
    /// `usage.cost_micros`: the delta in USD micros since the previous `usage` row of this step.
    pub cost_micros: Option<i64>,
    /// Added key (§7): context-window occupancy, ACP `usage_update.used`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_used: Option<i64>,
    /// Added key (§7): context-window size, ACP `usage_update.size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<i64>,
    /// Added key (§7): the cumulative session cost in USD micros, from which `cost_micros` is the
    /// difference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_micros_total: Option<i64>,
    /// Added key (§7): the reported amount when the currency is not USD, in which case
    /// `cost_micros` is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_amount: Option<f64>,
    /// Added key (§7): the reported currency when it is not USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_currency: Option<String>,
    /// Added key (§7): which summing convention produced the token fields, from
    /// `agent.settings.usage.scope` (§5.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_scope: Option<String>,
    /// Added key (§7, plan D66): the transport's vendor rate-limit blob, **verbatim**. Over ACP it
    /// is `_meta["_claude/rateLimit"]` of the `usage_update`, present on some reports and not on
    /// others (`tests/acp_map.rs`: the first report of a turn carries none). Not one of the five
    /// summed keys — `UsageTotals::add_payload` reads five fixed names (`usage.rs:48-54`) — and
    /// normalized into `agent_box.quota` by `htui_core::model::quota::normalize`, never here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<Value>,
}

/// `error` (§6.1). Also the shape the recorder writes for its own failures, with role `htui`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// A short machine-readable code, e.g. `refusal`, `scrub_residue`, `cap_exceeded`.
    pub code: String,
    /// The human-readable message. Scrubbed like any other payload.
    pub message: String,
}

/// `done` (§6.1): the end of exactly one turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneEvent {
    /// Why the turn ended.
    pub stop_reason: StopReason,
}

/// `other` (§6.1): any protocol update not mapped above, stored verbatim.
///
/// This is what makes "every event" hold without a schema change per protocol revision — the
/// session banner, `available_commands_update`, and every update an adapter ships ahead of the
/// schema all land here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OtherEvent {
    /// The transport's own name for the update, e.g. `session_info_update`.
    pub update: String,
    /// The verbatim body.
    pub body: Value,
}
