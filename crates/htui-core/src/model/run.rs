//! Runs, steps and their commits (`docs/ANA-9.md` §5.8).

use chrono::{DateTime, SubsecRound as _, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{AgentId, BoxId, ItemId, ProjectId, RepoId, RunId, StepId, UserId};

str_enum!(
    /// `run.kind` (§5.8).
    RunKind {
        /// A step-graph run against an item.
        Graph => "graph",
        /// A free-standing chat, `item_id` null.
        Chat => "chat",
    }
);

str_enum!(
    /// `run.mode` (§5.8).
    RunMode {
        /// Advanced by a human at each phase.
        Manual => "manual",
        /// Advanced by the orchestrator.
        Auto => "auto",
    }
);

str_enum!(
    /// `run.status` (§5.8).
    RunStatus {
        /// Waiting to start.
        Queued => "queued",
        /// Executing.
        Running => "running",
        /// Stopped at a gate.
        AwaitingApproval => "awaiting_approval",
        /// Finished successfully.
        Done => "done",
        /// Finished unsuccessfully.
        Failed => "failed",
        /// Stopped by a human.
        Cancelled => "cancelled",
    }
);

impl RunStatus {
    /// Whether the run counts towards the top bar's active-run indicator.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::AwaitingApproval)
    }
}

str_enum!(
    /// `run_step.status` (§5.8).
    StepStatus {
        /// Not started.
        Pending => "pending",
        /// Executing.
        Running => "running",
        /// Stopped at a gate.
        AwaitingApproval => "awaiting_approval",
        /// Finished successfully.
        Done => "done",
        /// Finished unsuccessfully.
        Failed => "failed",
        /// Stopped by a human.
        Cancelled => "cancelled",
        /// Replaced by a retry or by a fan-out winner.
        Superseded => "superseded",
    }
);

str_enum!(
    /// `run_step.gate_outcome` (§5.8): how the human answered the gate.
    GateOutcome {
        /// Approved, the run continues.
        Approved => "approved",
        /// Rejected, the run stops.
        Rejected => "rejected",
        /// Rejected with a retry of the same phase.
        Retried => "retried",
        /// The phase was skipped.
        Skipped => "skipped",
    }
);

/// A row of `run` (§5.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// `run.id`.
    pub id: RunId,
    /// `run.project_id`.
    pub project_id: ProjectId,
    /// `run.item_id`; `None` for a free-standing chat.
    pub item_id: Option<ItemId>,
    /// `run.kind`.
    pub kind: RunKind,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.status`.
    pub status: RunStatus,
    /// `run.target_box_id`: the box the run is for; version one executes only when it is local.
    pub target_box_id: BoxId,
    /// `run.executing_box_id`.
    pub executing_box_id: Option<BoxId>,
    /// `run.graph_snapshot` (`JSONB`): the graph, phases and agents as they were at start
    /// (`R-ORCH-11`).
    pub graph_snapshot: Option<Value>,
    /// `run.started_by`.
    pub started_by: UserId,
    /// `run.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `run.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `run.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
    /// `run.failure`.
    pub failure: Option<String>,
    /// `run.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `run_step` (§5.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStep {
    /// `run_step.id`.
    pub id: StepId,
    /// `run_step.run_id`.
    pub run_id: RunId,
    /// `run_step.position`: the phase index inside the snapshot.
    pub position: i32,
    /// `run_step.attempt`: retry / review loop counter.
    pub attempt: i32,
    /// `run_step.fanout_index`: `0..fan_out`.
    pub fanout_index: i32,
    /// `run_step.phase_name`; `chat` for a chat run.
    pub phase_name: String,
    /// `run_step.agent_id`.
    pub agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub model: Option<String>,
    /// `run_step.status`.
    pub status: StepStatus,
    /// `run_step.gate_outcome`.
    pub gate_outcome: Option<GateOutcome>,
    /// `run_step.gate_note`.
    pub gate_note: Option<String>,
    /// `run_step.selected`: the fan-out winner; `None` when `fan_out = 1`.
    pub selected: Option<bool>,
    /// `run_step.exit_code`.
    pub exit_code: Option<i32>,
    /// `run_step.prompt_digest`: sha256 of the `session_event` at `seq = 0`.
    pub prompt_digest: Option<String>,
    /// `run_step.trim_record` (`JSONB`): what was trimmed from the prompt and by how much
    /// (`R-PRM-3`).
    pub trim_record: Option<Value>,
    /// `run_step.usage` (`JSONB`): summed from the `usage` events.
    pub usage: Option<Value>,
    /// `run_step.isolation_path` on the executing box.
    pub isolation_path: Option<String>,
    /// `run_step.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `run_step.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
    /// `run_step.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Fractional-second digits Postgres `timestamptz` keeps: microseconds (§5.8).
const TIMESTAMPTZ_DIGITS: u16 = 6;

/// The two rows a free-standing chat needs before its first event can be recorded: one `run`
/// (`kind = 'chat'`, `item_id NULL`) and one `run_step` (`phase_name = 'chat'`, position 0)
/// (MOD-2 plan D4).
///
/// The ids are minted client-side, so both paths address the *same* two rows: written straight to
/// Postgres by [`WriteStore::start_chat_run`](crate::store::WriteStore::start_chat_run), or
/// buffered to `<cache_dir>/pending/<project_id>.<run_id>.jsonl` and uploaded later
/// (`docs/ANA-9.md` §4.3). Both inserts are `ON CONFLICT (id) DO NOTHING`, so however the two
/// interleave the database ends up with one `run` and one `run_step` for the chat, never a second
/// pair and never a duplicate-key error.
///
/// What converges is the row **count**, not every column. `DO NOTHING` means the path that lands
/// first owns the values, and the two paths do not write the same ones. `start_chat_run` writes
/// this spec's `agent_id` and `model`, `status = 'running'` on both rows (closed later by
/// `finish_chat_run`) and [`ChatRunSpec::started_at`] as every stamp. The upload
/// (`crates/htui-store/src/cache/pending.rs`) writes `agent_id` and `model` NULL - the pending line
/// format carries neither - `status = 'done'`, and stamps taken from the buffered events' `at`.
///
/// So an online-first chat keeps the spec's agent and model when the upload replays over it, while
/// an **offline-first** chat keeps a NULL `agent_id` even after a later online start. Carrying
/// `agent_id` and `model` in the pending format belongs to the offline session path, MOD-2
/// milestone 4 (plan D16); until it lands, that asymmetry is the guarantee. Both directions are
/// pinned in `crates/htui-store/tests/pg_criteria.rs`, by
/// `chat_run_rows_converge_with_the_offline_mint` and
/// `an_offline_first_chat_keeps_the_uploaded_columns`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRunSpec {
    /// `run.id`.
    pub run_id: RunId,
    /// `run_step.id` of the chat's only step.
    pub step_id: StepId,
    /// `run.project_id`.
    pub project_id: ProjectId,
    /// `run.target_box_id`; version one executes only when it is local, so it is also
    /// `run.executing_box_id`.
    pub target_box_id: BoxId,
    /// `run.started_by`.
    pub started_by: UserId,
    /// `run_step.agent_id`.
    pub agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub model: Option<String>,
    /// `run.queued_at`, `run.started_at` and `run_step.started_at`: one clock reading for the whole
    /// mint, so the two rows agree.
    ///
    /// Truncated to microseconds by [`ChatRunSpec::mint`], which is `timestamptz`'s resolution: an
    /// untruncated Windows clock reading would come back from Postgres different from the one held
    /// in memory, and the two backends would disagree about a column neither of them changed.
    pub started_at: DateTime<Utc>,
}

impl ChatRunSpec {
    /// Mints the ids and stamps the clock: the one place a chat's `run.id` and `run_step.id` come
    /// from, online or offline (plan D4).
    #[must_use]
    pub fn mint(
        project_id: ProjectId,
        target_box_id: BoxId,
        started_by: UserId,
        agent_id: Option<AgentId>,
        model: Option<String>,
    ) -> Self {
        Self {
            run_id: RunId::new(),
            step_id: StepId::new(),
            project_id,
            target_box_id,
            started_by,
            agent_id,
            model,
            started_at: Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS),
        }
    }
}

/// A row of `run_step_commit` (§5.8): the before/after commit of one repository for one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStepCommit {
    /// `run_step_commit.run_step_id`.
    pub run_step_id: StepId,
    /// `run_step_commit.repo_id`.
    pub repo_id: RepoId,
    /// `run_step_commit.before_hash`.
    pub before_hash: String,
    /// `run_step_commit.after_hash`.
    pub after_hash: Option<String>,
}

/// Step projection carried inside a [`RunSummary`]: what the Runs sub-tab lists per step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStepSummary {
    /// `run_step.id`.
    pub id: StepId,
    /// `run_step.position`.
    pub position: i32,
    /// `run_step.attempt`.
    pub attempt: i32,
    /// `run_step.fanout_index`.
    pub fanout_index: i32,
    /// `run_step.phase_name`.
    pub phase_name: String,
    /// `run_step.agent_id`.
    pub agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub model: Option<String>,
    /// `run_step.status`.
    pub status: StepStatus,
    /// `run_step.gate_outcome`.
    pub gate_outcome: Option<GateOutcome>,
    /// `run_step.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `run_step.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
    /// `run_step.trim_record.estimated_after`: what the assembler believed the prompt cost after
    /// trimming, or `None` on a step no assembler ever wrote (plan D106, `docs/ANA-5.md` §4.4).
    ///
    /// A projection of `trim_record` and not a column: the record itself is a whole JSON document
    /// the Runs pane has no room for, and `ReadStore` exposes neither it nor `prompt_digest`, so
    /// these two fields are the seam's only trace of a written prompt audit.
    pub prompt_tokens: Option<i32>,
    /// Whether any `trim_record.sections[].trimmed` is `true`: the `!` the Runs pane renders
    /// beside the token figure (plan D106).
    pub trimmed: bool,
}

/// Plan D106's derivation of [`RunStepSummary::prompt_tokens`] and [`RunStepSummary::trimmed`]
/// from `run_step.trim_record`, in one place so `MemStore` and the two SQL projections agree by
/// test rather than by luck.
///
/// A record with no `estimated_after`, a non-integer one, or one outside `i32` yields `None`
/// rather than a wrong number; `trimmed` is `false` unless `sections` is an array holding at least
/// one object whose `trimmed` is the JSON `true`. Both halves are deliberately total: `trim_record`
/// is an untyped `JSONB` column and a malformed document must render as "no figure", never panic a
/// list.
#[must_use]
pub fn prompt_summary(trim_record: Option<&Value>) -> (Option<i32>, bool) {
    let Some(record) = trim_record else {
        return (None, false);
    };
    let tokens = record
        .get("estimated_after")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let trimmed = record
        .get("sections")
        .and_then(Value::as_array)
        .is_some_and(|sections| {
            sections
                .iter()
                .any(|section| section.get("trimmed").and_then(Value::as_bool) == Some(true))
        });
    (tokens, trimmed)
}

/// Result row of [`crate::store::ReadStore::runs`]: a run with its steps, newest run first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// `run.id`.
    pub id: RunId,
    /// `run.item_id`.
    pub item_id: Option<ItemId>,
    /// `run.project_id`.
    pub project_id: ProjectId,
    /// `run.kind`.
    pub kind: RunKind,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.status`.
    pub status: RunStatus,
    /// `run.target_box_id`.
    pub target_box_id: BoxId,
    /// `run.executing_box_id`.
    pub executing_box_id: Option<BoxId>,
    /// `box.hostname` of the executing box, or of the target box when nothing is executing yet:
    /// the Runs table shows a name, not a UUID.
    pub box_hostname: String,
    /// `run.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `run.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `run.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
    /// `run.failure`.
    pub failure: Option<String>,
    /// The run's steps, ordered by `(position, attempt, fanout_index)`.
    pub steps: Vec<RunStepSummary>,
}

#[cfg(test)]
mod tests {
    use super::prompt_summary;
    use serde_json::json;

    /// Plan D106's two figures, and the four ways a record can decline to supply them.
    #[test]
    fn prompt_summary_reads_estimated_after_and_any_trimmed() {
        assert_eq!(
            prompt_summary(None),
            (None, false),
            "a step no assembler wrote has no figure and was not trimmed"
        );
        assert_eq!(
            prompt_summary(Some(&json!({
                "estimated_after": 34_000,
                "sections": [
                    { "name": "template", "trimmed": false },
                    { "name": "excerpts", "trimmed": true },
                ],
                "v": 1,
            }))),
            (Some(34_000), true),
            "one trimmed section is enough"
        );
        assert_eq!(
            prompt_summary(Some(&json!({ "estimated_after": 12, "sections": [] }))),
            (Some(12), false),
            "an empty section list is not a trim"
        );
        assert_eq!(
            prompt_summary(Some(&json!({ "sections": [{ "trimmed": true }] }))),
            (None, true),
            "the two facts are independent"
        );
        assert_eq!(
            prompt_summary(Some(&json!({ "estimated_after": "34000", "sections": {} }))),
            (None, false),
            "a malformed JSONB document renders as `no figure`, never a wrong one"
        );
        assert_eq!(
            prompt_summary(Some(&json!({ "estimated_after": 3_000_000_000_i64 }))),
            (None, false),
            "a value outside i32 is no figure rather than a truncated one"
        );
    }
}
