//! The permission and control relay's rows and outcomes (MOD-42 plan D1-D5, D12, D13;
//! `0011_permission_relay.sql`) and MOD-70's follow-ups (plan D1-D5, D12;
//! `0016_follow_up.sql`). None of the three tables is mirrored (MOD-42 OQ-4, MOD-70 D1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::model::ids::{BoxId, PermissionId, RelaySessionId, RunCommandId, RunId, StepId, UserId};
use crate::scrub::{MinimalScrubber, Scrubber, Unmasked};

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
    /// `run_command.kind` (MOD-42 plan D1; MOD-70 plan D1).
    RunCommandKind {
        /// Cancel the run (MOD-42 D12).
        Cancel => "cancel",
        /// One user follow-up for a running engine step's live session (MOD-70).
        FollowUp => "follow_up",
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
    /// MOD-70 D5: the newest follow-up of each step of the item's non-terminal runs (B-13),
    /// `(issued_at, id)` order. Never the text (OQ-6).
    pub follow_ups: Vec<FollowUpView>,
}

// -- MOD-70: follow-ups for engine steps (plan D1-D5, D12) -------------------------------------

/// MOD-70 D2: a follow-up's text, checked when built: not empty after trimming, and nothing a
/// pattern-only scrubber refuses. Stored and sent as typed (PRD Q3). `Debug` prints the length
/// only; there is no `Display` and no serde (I-5).
#[derive(Clone, PartialEq, Eq)]
pub struct FollowUpText(String);

impl FollowUpText {
    /// Checks `text` and keeps it as typed.
    ///
    /// # Errors
    /// [`FollowUpTextError::Empty`] for an empty or whitespace-only text;
    /// [`FollowUpTextError::Residue`] when `MinimalScrubber::new(Vec::<String>::new())` refuses
    /// `{"text": text}` (the payload shape `record_follow_up` scrubs).
    pub fn new(text: String) -> Result<Self, FollowUpTextError> {
        if text.trim().is_empty() {
            return Err(FollowUpTextError::Empty);
        }
        let mut payload = json!({ "text": text });
        MinimalScrubber::new(Vec::<String>::new())
            .scrub(&mut payload)
            .map_err(FollowUpTextError::Residue)?;
        Ok(Self(text))
    }

    /// The text as typed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The text as typed, moved out.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl core::fmt::Debug for FollowUpText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FollowUpText")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Why a typed follow-up was not sent (D2, D12; nothing was written).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FollowUpTextError {
    /// Empty or whitespace-only.
    #[error("{}", FOLLOW_UP_EMPTY)]
    Empty,
    /// A pattern-only scrubber refused the text. Carries the pointer and the rule, never the text
    /// (`scrub.rs`'s [`Unmasked`]).
    #[error("not sent: the text looks like it holds a credential ({})", .0.rule)]
    Residue(Unmasked),
}

/// Arguments of `WriteStore::request_follow_up` (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFollowUp {
    /// Minted by the sender (`RunCommandId::new()`).
    pub id: RunCommandId,
    /// The running engine step whose live session takes it.
    pub run_step_id: StepId,
    /// The checked text.
    pub text: FollowUpText,
    /// The sending user.
    pub issued_by: UserId,
    /// The sending box.
    pub issued_box: BoxId,
}

/// `WriteStore::request_follow_up`'s outcome (D3; a refusal writes nothing, MOD-42 OQ-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpRequest {
    /// A new `pending` row.
    Queued(RunCommandId),
    /// Nothing was written; why.
    Refused(FollowUpRefusal),
}

/// Why an enqueue wrote nothing, in D3's classification order. `Display` is D12's sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FollowUpRefusal {
    /// The step is a chat run's.
    #[error("{}", FOLLOW_UP_CHAT_RUN)]
    ChatRun,
    /// The step is a judge (`fanout_index < 0`).
    #[error("{}", FOLLOW_UP_JUDGE)]
    Judge,
    /// The step is not `running`.
    #[error("{}", FOLLOW_UP_NOT_RUNNING)]
    NotRunning,
    /// The run has a pending cancel.
    #[error("{}", FOLLOW_UP_CANCELLING)]
    Cancelling,
    /// The step already has a pending follow-up.
    #[error("{}", FOLLOW_UP_ALREADY_QUEUED)]
    AlreadyQueued,
    /// The run's lease is not live under the window's owner (B-4).
    #[error("{}", FOLLOW_UP_EXECUTOR_GONE)]
    ExecutorGone,
    /// The step has no window yet.
    #[error("{}", FOLLOW_UP_NOT_STARTED)]
    NotStarted,
    /// The step's window is closed.
    #[error("{}", FOLLOW_UP_SESSION_ENDED)]
    SessionEnded,
}

/// The step's pending follow-up, as the executor reads it (`next_follow_up`, D4). `Debug` prints
/// the id and the text's length only.
#[derive(Clone, PartialEq, Eq)]
pub struct QueuedFollowUp {
    /// `run_command.id`.
    pub id: RunCommandId,
    /// `run_command.text`, as typed.
    pub text: String,
}

impl core::fmt::Debug for QueuedFollowUp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueuedFollowUp")
            .field("id", &self.id)
            .field("len", &self.text.len())
            .finish()
    }
}

/// What `settle_follow_up` moves a pending row to (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowUpSettle {
    /// Taken for the next turn: `applied`, `resolution` NULL.
    Applied,
    /// `refused`, with this sentence as `resolution` (`executor_scrub_refusal`).
    Refused(String),
}

/// `settle_follow_up`'s outcome (D4, B-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    /// This call moved it; `text` is now NULL.
    Settled,
    /// Not pending any more (a cancel, a newer window or a close refused it).
    NotPending,
    /// Pending, but `owner` is not the run's lease owner.
    Fenced,
}

/// What the Runs pane shows for one step's newest follow-up (D5, D14). No text (OQ-6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUpView {
    /// `run_command.id`.
    pub id: RunCommandId,
    /// `run_command.run_id`.
    pub run_id: RunId,
    /// `run_command.run_step_id`.
    pub run_step_id: StepId,
    /// `run_command.status`.
    pub status: RunCommandStatus,
    /// `run_command.resolution`: the refusal's sentence, or `None`.
    pub resolution: Option<String>,
    /// `run_command.issued_at`, the store's clock.
    pub issued_at: DateTime<Utc>,
    /// `run_command.resolved_at`.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// A row of `follow_up_window` (D1, OQ-2): one per step whose engine session takes follow-ups.
/// MemStore's state and its test-support reader (B-1); no trait method returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpWindow {
    /// `follow_up_window.run_step_id` (the key).
    pub run_step_id: StepId,
    /// `follow_up_window.run_id`.
    pub run_id: RunId,
    /// `follow_up_window.session`: the session that opened it.
    pub session: RelaySessionId,
    /// `follow_up_window.owner`: the lease owner that opened it (B-4).
    pub owner: Uuid,
    /// `follow_up_window.opened_at`, the store's clock.
    pub opened_at: DateTime<Utc>,
    /// `follow_up_window.closed_at`: set when the session ends.
    pub closed_at: Option<DateTime<Utc>>,
}

/// [`FollowUpRefusal::NotRunning`].
pub const FOLLOW_UP_NOT_RUNNING: &str =
    "only a running step takes a follow-up; p promotes a parked or failed step to a chat";
/// [`FollowUpRefusal::Judge`].
pub const FOLLOW_UP_JUDGE: &str = "a judge session takes no follow-up";
/// [`FollowUpRefusal::ChatRun`].
pub const FOLLOW_UP_CHAT_RUN: &str = "a chat takes follow-ups in its own view";
/// [`FollowUpRefusal::AlreadyQueued`].
pub const FOLLOW_UP_ALREADY_QUEUED: &str = "a follow-up is already queued";
/// [`FollowUpRefusal::SessionEnded`], and the resolution of a row its window's close (or a newer
/// window) refused.
pub const FOLLOW_UP_SESSION_ENDED: &str = "the step finished its session; promote it to continue";
/// [`FollowUpRefusal::NotStarted`].
pub const FOLLOW_UP_NOT_STARTED: &str = "the step's session has not started yet";
/// [`FollowUpRefusal::Cancelling`].
pub const FOLLOW_UP_CANCELLING: &str = "the run is being cancelled";
/// The resolution of a pending follow-up a run's cancel refused (D5, B-14).
pub const FOLLOW_UP_RUN_CANCELLED: &str = "the run was cancelled";
/// [`FollowUpRefusal::ExecutorGone`].
pub const FOLLOW_UP_EXECUTOR_GONE: &str = "the process walking the step no longer holds the run";
/// The resolution of a pending follow-up whose session was cancelled before it was sent (D6).
pub const FOLLOW_UP_SESSION_CANCELLED: &str =
    "the step's session was cancelled before the follow-up was sent";
/// [`FollowUpTextError::Empty`].
pub const FOLLOW_UP_EMPTY: &str = "a follow-up needs text";
/// The Runs pane's label for a pending follow-up (PRD Q7, D12, B-21).
pub const FOLLOW_UP_QUEUED: &str = "queued \u{2014} sent when the current turn ends";
/// The Runs pane's label for an applied follow-up (D12, B-21).
pub const FOLLOW_UP_SENT: &str = "follow-up sent";
/// The Runs pane's label for a refused follow-up (D12, B-21).
pub const FOLLOW_UP_REFUSED: &str = "follow-up refused";

/// D12: the executor's scrubber refused the text (D6 step 4).
#[must_use]
pub fn executor_scrub_refusal(rule: &str) -> String {
    format!("the executing box's scrubber refused the text ({rule})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_text_refuses_empty_and_whitespace_only_text() {
        for text in ["", " ", "\t\n  "] {
            let err = FollowUpText::new(text.to_owned()).expect_err("empty");
            assert_eq!(err, FollowUpTextError::Empty);
            assert_eq!(err.to_string(), FOLLOW_UP_EMPTY);
        }
    }

    #[test]
    fn follow_up_text_refuses_a_credential_and_names_only_its_rule() {
        let err = FollowUpText::new("use sk-ant-api03-aaaaaaaaaaaaaaaaaaaa now".to_owned())
            .expect_err("a credential");
        assert!(
            matches!(err, FollowUpTextError::Residue(ref u) if u.rule == "anthropic_api_key"),
            "{err:?}"
        );
        let display = err.to_string();
        let debug = format!("{err:?}");
        assert!(display.contains("anthropic_api_key"), "{display}");
        assert!(!display.contains("sk-ant"), "{display}");
        assert!(!debug.contains("sk-ant"), "{debug}");
    }

    #[test]
    fn follow_up_text_keeps_prose_as_typed() {
        let typed = "  use the smaller fixture\t";
        let text = FollowUpText::new(typed.to_owned()).expect("prose");
        assert_eq!(text.as_str(), typed);
        assert_eq!(text.into_string().as_bytes(), typed.as_bytes());
    }

    #[test]
    fn follow_up_texts_debug_prints_its_length_only() {
        let text = FollowUpText::new("rename the helper".to_owned()).expect("prose");
        let debug = format!("{text:?}");
        assert_eq!(debug, "FollowUpText { len: 17 }");
        assert!(!debug.contains("rename"), "{debug}");
    }

    #[test]
    fn a_queued_follow_ups_debug_prints_its_length_only() {
        let id = RunCommandId::new();
        let queued = QueuedFollowUp {
            id,
            text: "rename the helper".to_owned(),
        };
        let debug = format!("{queued:?}");
        assert_eq!(debug, format!("QueuedFollowUp {{ id: {id:?}, len: 17 }}"));
        assert!(!debug.contains("rename"), "{debug}");
    }
}
