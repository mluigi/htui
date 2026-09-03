//! Session events: the ordered, scrubbed replay log of a run step (`docs/ANA-9.md` §4.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::StepId;

str_enum!(
    /// `session_event.kind` (§4.3). The payload contract per kind is the table in §4.3: the
    /// driver may add keys, never rename the documented ones.
    EventKind {
        /// The assembled initial prompt; always `seq = 0`, `turn = 0`.
        Prompt => "prompt",
        /// A user follow-up.
        FollowUp => "follow_up",
        /// Agent text, contiguous chunks already coalesced by the driver.
        AssistantText => "assistant_text",
        /// Agent reasoning, coalesced the same way.
        Thought => "thought",
        /// A tool call.
        ToolCall => "tool_call",
        /// The result of a tool call.
        ToolResult => "tool_result",
        /// A proposed edit.
        EditProposal => "edit_proposal",
        /// A permission request from the agent.
        PermissionRequest => "permission_request",
        /// The answer to a permission request.
        PermissionAnswer => "permission_answer",
        /// A plan update.
        Plan => "plan",
        /// A token/cost usage report.
        Usage => "usage",
        /// An error.
        Error => "error",
        /// End of turn.
        Done => "done",
        /// Any protocol update not mapped above, stored verbatim.
        Other => "other",
    }
);

str_enum!(
    /// `session_event.role` (§4.3).
    EventRole {
        /// Written by the user.
        User => "user",
        /// Written by the agent.
        Agent => "agent",
        /// Written by `htui` itself.
        Htui => "htui",
    }
);

/// A row of `session_event` (§4.3). `seq` is the total replay order per step; `at` is
/// informational and ordering never uses it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// `session_event.run_step_id`.
    pub run_step_id: StepId,
    /// `session_event.seq`: 0-based, assigned by `htui` at capture.
    pub seq: i32,
    /// `session_event.turn`: increments on each prompt or follow-up.
    pub turn: i32,
    /// `session_event.kind`.
    pub kind: EventKind,
    /// `session_event.role`.
    pub role: EventRole,
    /// `session_event.tool_call_id`: pairs a call with its result, proposal or permission events.
    pub tool_call_id: Option<String>,
    /// `session_event.payload` (`JSONB`): the per-kind contract of §4.3.
    pub payload: Value,
    /// `session_event.raw` (`JSONB`): the wire message, kept only when the project opts in.
    pub raw: Option<Value>,
    /// `session_event.at`: capture time on the executing box.
    pub at: DateTime<Utc>,
}
