//! The permission and control relay's rows and outcomes (MOD-42 plan D1-D5, D12, D13;
//! `0011_permission_relay.sql`). Neither table is mirrored (plan OQ-4).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::ids::{BoxId, PermissionId, RelaySessionId, RunCommandId, RunId, StepId, UserId};

str_enum!(
    /// `step_permission.status` (MOD-42 plan D1), in `CHECK` order.
    PermissionStatus {
        /// Parked; any store client may answer it (D3).
        Pending => "pending",
        /// A client answered it; the executor has not applied it yet (D4).
        Answered => "answered",
        /// The executor applied the answer to the live session and recorded it (D4).
        Applied => "applied",
        /// Its session was cancelled gracefully while it was open (I-7, D5).
        Cancelled => "cancelled",
        /// Its session ended, or a newer session of the step opened a row (D5).
        Stale => "stale",
    }
);

str_enum!(
    /// `run_command.kind` (MOD-42 plan D1). A follow-up command adds a kind later (PRD Q9).
    RunCommandKind {
        /// Cancel the run (D12).
        Cancel => "cancel",
    }
);

str_enum!(
    /// `run_command.status` (MOD-42 plan D1), in `CHECK` order.
    RunCommandStatus {
        /// Written; not yet applied by the run's executor (OQ-2: no timeout).
        Pending => "pending",
        /// The executor applied it.
        Applied => "applied",
        /// The executor could not apply it; `resolution` says why (a run already terminal).
        Refused => "refused",
    }
);

str_enum!(
    /// A relayed option's kind: `htui_agent::event::PermissionOptionKind`'s four values, as text
    /// (htui-core cannot depend on htui-agent; plan D6-ids). A UI hint only.
    RelayOptionKind {
        /// Allow this call only.
        AllowOnce => "allow_once",
        /// Allow this call and let the agent remember it (D16: htui persists nothing).
        AllowAlways => "allow_always",
        /// Reject this call only.
        RejectOnce => "reject_once",
        /// Reject this call and let the agent remember it.
        RejectAlways => "reject_always",
    }
);

/// One option of a relayed request, as `step_permission.options[]` stores it (`{id,label,kind}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayOption {
    /// The agent's option id, sent back verbatim; never scrubbed.
    pub id: String,
    /// The label, scrubbed by the executor's recorder before it is stored (I-5).
    pub label: String,
    /// The kind.
    pub kind: RelayOptionKind,
}

/// A row of `step_permission`, without `owner` (blueprint B-8: a reader has no use for another
/// process's liveness token, as `Run` has no `lease_owner`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepPermission {
    /// `step_permission.id`.
    pub id: PermissionId,
    /// `step_permission.run_id`.
    pub run_id: RunId,
    /// `step_permission.run_step_id`.
    pub run_step_id: StepId,
    /// `step_permission.session`.
    pub session: RelaySessionId,
    /// `step_permission.request_id`: the transport's id, unique per session.
    pub request_id: String,
    /// `step_permission.tool_call_id`.
    pub tool_call_id: Option<String>,
    /// `step_permission.summary`: `"<tool_kind>: <title>"`, scrubbed; `None` when the transport
    /// named no call or the scrubber refused it (B-14).
    pub summary: Option<String>,
    /// `step_permission.options`, in the agent's order.
    pub options: Vec<RelayOption>,
    /// `step_permission.status`.
    pub status: PermissionStatus,
    /// `step_permission.option_id`: set by the answer.
    pub option_id: Option<String>,
    /// `step_permission.answered_by`.
    pub answered_by: Option<UserId>,
    /// `step_permission.answered_box`.
    pub answered_box: Option<BoxId>,
    /// `step_permission.created_at`, the store's clock (I-4).
    pub created_at: DateTime<Utc>,
    /// `step_permission.answered_at`.
    pub answered_at: Option<DateTime<Utc>>,
    /// `step_permission.resolved_at`: set by `applied`, `cancelled` and `stale`.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Arguments of `WriteStore::open_permission`: the row as the executor parks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPermission {
    /// Minted by the executor (`PermissionId::new()`).
    pub id: PermissionId,
    /// The run.
    pub run_id: RunId,
    /// The step; must be a step of `run_id`.
    pub run_step_id: StepId,
    /// The driven session.
    pub session: RelaySessionId,
    /// The transport's request id.
    pub request_id: String,
    /// The gated tool call, when the transport named one.
    pub tool_call_id: Option<String>,
    /// Scrubbed summary (B-14).
    pub summary: Option<String>,
    /// Scrubbed options.
    pub options: Vec<RelayOption>,
    /// The executor's lease owner at park time (I-2).
    pub owner: Uuid,
}

/// What an applied answer chose (`WriteStore::apply_permission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionChoice {
    /// The option id the answer named.
    pub option_id: String,
}

/// `WriteStore::answer_permission`'s outcome (D3, OQ-1: a loser is told, not persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerOutcome {
    /// This answer won the compare-and-set.
    Answered,
    /// Nothing was written; the request's actual state.
    Refused(AnswerRefusal),
}

/// Why an answer wrote nothing (D3). The `Display` text is what the status line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnswerRefusal {
    /// Another answer won first.
    #[error("{}", ALREADY_ANSWERED)]
    Answered,
    /// Another answer won and the executor applied it.
    #[error("{}", ALREADY_APPLIED)]
    Applied,
    /// The request's session was cancelled.
    #[error("{}", REQUEST_CANCELLED)]
    Cancelled,
    /// The request's session ended or was superseded.
    #[error("{}", REQUEST_STALE)]
    Stale,
    /// The row's owner no longer holds a live lease on the run (adopted, released or expired).
    #[error("{}", EXECUTOR_GONE)]
    ExecutorGone,
    /// The option id is not one the request offered.
    #[error("{}", NOT_OFFERED)]
    NotOffered,
}

/// [`AnswerRefusal::Answered`].
pub const ALREADY_ANSWERED: &str = "this permission request was already answered";
/// [`AnswerRefusal::Applied`].
pub const ALREADY_APPLIED: &str = "this permission request was already answered and applied";
/// [`AnswerRefusal::Cancelled`].
pub const REQUEST_CANCELLED: &str = "this permission request was cancelled with its session";
/// [`AnswerRefusal::Stale`].
pub const REQUEST_STALE: &str = "this permission request belongs to a session that has ended";
/// [`AnswerRefusal::ExecutorGone`].
pub const EXECUTOR_GONE: &str =
    "the process that asked no longer holds the run; its request cannot be answered";
/// [`AnswerRefusal::NotOffered`].
pub const NOT_OFFERED: &str = "that option was not offered by this permission request";

/// `WriteStore::request_cancel`'s outcome: the partial unique index admits one pending cancel per
/// run (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelRequest {
    /// A new row.
    Inserted(RunCommandId),
    /// A pending cancel already existed; nothing was written.
    AlreadyPending(RunCommandId),
}

/// A row of `run_command`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCommand {
    /// `run_command.id`.
    pub id: RunCommandId,
    /// `run_command.run_id`.
    pub run_id: RunId,
    /// `run_command.kind`.
    pub kind: RunCommandKind,
    /// `run_command.issued_by`.
    pub issued_by: UserId,
    /// `run_command.issued_box`.
    pub issued_box: BoxId,
    /// `run_command.status`.
    pub status: RunCommandStatus,
    /// `run_command.resolution`: the refusal's sentence, or `None`.
    pub resolution: Option<String>,
    /// `run_command.issued_at`, the store's clock.
    pub issued_at: DateTime<Utc>,
    /// `run_command.resolved_at`.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// What the Runs pane shows for one item (D14): its live pending requests and its runs with a
/// pending cancel. Empty offline (D14, OQ-4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RelayView {
    /// `pending` rows whose `owner` is the run's live lease owner, `(created_at, id)` order.
    pub permissions: Vec<StepPermission>,
    /// Non-terminal runs of the item with a `pending` cancel, ascending.
    pub cancels: Vec<RunId>,
}
