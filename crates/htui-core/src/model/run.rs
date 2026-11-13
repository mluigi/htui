//! Runs, steps and their commits (`docs/ANA-9.md` §5.8).

use chrono::{DateTime, SubsecRound as _, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{
    AgentId, BatchId, BoxId, CommandRunId, ItemId, PersonaId, ProjectId, RepoId, RunId, RunPlanId,
    StepGraphId, StepId, UserId, WorkspaceId,
};
use crate::model::kind::{
    CommandQueue, EdgeKind, EdgePredicate, Gate, GraphScope, Isolation, PlanScope, Split,
};

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
        /// Halted by a workspace stop (MOD-125 D1, `R-ORCH-16`): live and resumable. Holds no
        /// slot, still refuses an overlapping scope (it keeps its trees and branch), has no
        /// `finished_at`, and its item reads `stopped` (MOD-144).
        Stopped => "stopped",
        /// Finished successfully.
        Done => "done",
        /// Finished unsuccessfully.
        Failed => "failed",
        /// Stopped by a human.
        Cancelled => "cancelled",
    }
);

impl RunStatus {
    /// Whether the run is **live** (MOD-125 D2): `queued | running | awaiting_approval |
    /// stopped`. Every live run may still move; the complement is [`RunStatus::is_terminal`]. Not
    /// the top bar's count, which is [`RunStatus::on_top_bar`].
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::AwaitingApproval | Self::Stopped
        )
    }

    /// R-TUI-1's active-run count (MOD-125 D2): the live runs less `stopped`, so the top bar keeps
    /// counting "running and waiting". `active_runs` in every store filters by it.
    #[must_use]
    pub const fn on_top_bar(self) -> bool {
        self.is_active() && !matches!(self, Self::Stopped)
    }

    /// ANA-2 §4.3's `run` table (`docs/ANA-2.md:653-687`): whether `self -> to` is a sanctioned
    /// move. Every terminal status answers `false` for every `to`.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Queued => matches!(to, Self::Running | Self::Cancelled | Self::Failed),
            Self::Running => matches!(
                to,
                Self::AwaitingApproval
                    | Self::Done
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Stopped
            ),
            Self::AwaitingApproval => matches!(to, Self::Running | Self::Failed | Self::Cancelled),
            Self::Stopped => matches!(to, Self::Running | Self::Cancelled | Self::Failed),
            Self::Done | Self::Failed | Self::Cancelled => false,
        }
    }

    /// `done | failed | cancelled` — the complement of [`RunStatus::is_active`], and the set that
    /// reaches nothing in [`RunStatus::can_move_to`].
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !self.is_active()
    }
}

/// What [`latest_graph_run`] orders by; implemented by [`Run`] and [`RunSummary`].
pub trait RunFacts {
    /// `run.kind`.
    fn run_kind(&self) -> RunKind;
    /// `run.status`.
    fn run_status(&self) -> RunStatus;
    /// `(run.queued_at, run.id)`, the order every store's `ORDER BY` uses.
    fn run_order(&self) -> (DateTime<Utc>, RunId);
}

impl RunFacts for Run {
    fn run_kind(&self) -> RunKind {
        self.kind
    }
    fn run_status(&self) -> RunStatus {
        self.status
    }
    fn run_order(&self) -> (DateTime<Utc>, RunId) {
        (self.queued_at, self.id)
    }
}

impl RunFacts for RunSummary {
    fn run_kind(&self) -> RunKind {
        self.kind
    }
    fn run_status(&self) -> RunStatus {
        self.status
    }
    fn run_order(&self) -> (DateTime<Utc>, RunId) {
        (self.queued_at, self.id)
    }
}

/// MOD-144: the item's latest graph run, newest `kind = 'graph'` and not `cancelled` by
/// `(queued_at, id)` (ANA-35 §7.1). Every store's SQL repeats this rule; the caller passes the
/// runs of one item.
pub fn latest_graph_run<'a, R: RunFacts + 'a>(
    runs: impl IntoIterator<Item = &'a R>,
) -> Option<&'a R> {
    runs.into_iter()
        .filter(|run| run.run_kind() == RunKind::Graph && run.run_status() != RunStatus::Cancelled)
        .max_by_key(|run| run.run_order())
}

/// MOD-144 Δ2: the item's newest **live** graph run (`RunStatus::is_active`, `stopped`
/// included) by `(queued_at, id)`, the one `uq_run_item_live` allows and `create_run`'s
/// [`startable`](crate::store::startable) check names. The caller passes the runs of one item.
pub fn latest_live_graph_run<'a, R: RunFacts + 'a>(
    runs: impl IntoIterator<Item = &'a R>,
) -> Option<&'a R> {
    runs.into_iter()
        .filter(|run| run.run_kind() == RunKind::Graph && run.run_status().is_active())
        .max_by_key(|run| run.run_order())
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
        /// Interrupted by a workspace stop (MOD-125 D1). Resumes to `running` on its own row (M3):
        /// its agent session reopened, or, when that cannot be done, a fresh session on the same
        /// row and attempt opened with the handoff text (M3's fallback). Otherwise cancelled or
        /// failed with its run.
        Stopped => "stopped",
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

impl StepStatus {
    /// ANA-2 §4.3's `run_step` table (`docs/ANA-2.md:700-740`). `failed` is **not** terminal for
    /// a step: it can be promoted to `awaiting_approval` (§4.8) while its run is live, and, like
    /// every non-terminal status, can be cancelled with its run. The "while the run is
    /// non-terminal" condition on promotion belongs to
    /// [`WriteStore::promote_step`](crate::store::WriteStore::promote_step), not to this table.
    ///
    /// `awaiting_approval -> awaiting_approval` is §4.3's promotion row and the one self-move any
    /// of the three tables sanctions: a gated step promoted to chat keeps its status and gains
    /// `promoted_at`.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Pending => matches!(to, Self::Running | Self::Superseded | Self::Cancelled),
            Self::Running => matches!(
                to,
                Self::AwaitingApproval
                    | Self::Done
                    | Self::Failed
                    | Self::Cancelled
                    | Self::Stopped
            ),
            Self::Stopped => matches!(
                to,
                Self::Running | Self::Cancelled | Self::Failed | Self::Superseded
            ),
            Self::AwaitingApproval => matches!(
                to,
                Self::Done
                    | Self::Failed
                    | Self::Superseded
                    | Self::AwaitingApproval
                    | Self::Cancelled
            ),
            Self::Failed => matches!(to, Self::AwaitingApproval | Self::Cancelled),
            Self::Done => matches!(to, Self::Superseded),
            Self::Cancelled | Self::Superseded => false,
        }
    }

    /// `done | cancelled | superseded` (ANA-2 §4.3). `failed` is deliberately absent, and `done`
    /// is present even though it still reaches `superseded`.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Superseded)
    }
}

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

str_enum!(
    /// `run_step.verify_outcome` (ANA-2 §4.2): `unavailable` never fails a step.
    VerifyOutcome {
        /// The verify command exited 0.
        Pass => "pass",
        /// The verify command exited non-zero.
        Fail => "fail",
        /// No verify command, or it could not be run.
        Unavailable => "unavailable",
    }
);

str_enum!(
    /// `run_step.opening` (MOD-37 M5, R-48): how a promoted step's chat opened, or how a stopped
    /// step's session reopened when it resumed (MOD-125 M3). `NULL` on a step never bound to a
    /// promoted chat and never resumed through its saved session.
    StepOpening {
        /// The step's own agent session was restored.
        Resumed => "resumed",
        /// A fresh session opened with the handoff prompt.
        Handoff => "handoff",
        /// A requested resume failed; the session opened fresh, with the handoff prompt (a
        /// promoted chat) or the stop's fallback text on the same row (a resumed stopped step).
        ResumeFailed => "resume_failed",
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
    /// `run.workspace_id` (MOD-125 D3): the workspace a graph run was started from; `None` for a
    /// chat run, a run started without a scope, or one whose project was in no or several
    /// workspaces at 0019.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
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
    /// `run.repo_scope`: the repos this run may touch (ANA-2 §4.7); empty on every row older than
    /// `0003_orchestration.sql` and on a chat run.
    pub repo_scope: Vec<RepoId>,
    /// `run.lease_box_id` (ANA-2 §4.9).
    pub lease_box_id: Option<BoxId>,
    /// `run.lease_expires_at`; `None` = never claimed. `run.lease_owner` is deliberately **not**
    /// on this row: the mirror does not carry it and a reader has no use for another process's
    /// liveness token.
    pub lease_expires_at: Option<DateTime<Utc>>,
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
    /// `run_step.fanout_index`: `0..fan_out`; `-1` is the judge step of this position and attempt
    /// (ANA-2 §4.5).
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
    /// `run_step.verify_outcome` (ANA-2 §4.2).
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.verify_exit_code`: the verify command's own exit code, distinct from
    /// [`RunStep::exit_code`].
    pub verify_exit_code: Option<i32>,
    /// `run_step.promoted_at` (ANA-2 §4.8).
    pub promoted_at: Option<DateTime<Utc>>,
    /// `run_step.branch` (`JSONB`, MOD-138 M2 D5): `{"<to key>": bool}` per `if` edge of the
    /// step's node, written once by `record_branch`; `None` until decided and on every node with
    /// no `if` edge.
    #[serde(default)]
    pub branch: Option<Value>,
    /// `run_step.role` (MOD-139 D3, ANA-33 §7.4, §7.11): what the row is. [`StepRole::Step`] on
    /// every row written before `0029_divided_fanout.sql` and on every compete candidate and judge.
    /// Written once, by `create_step`.
    #[serde(default)]
    pub role: StepRole,
    /// `run_step.parent_step_id` (MOD-139 D3): the step this one was spawned under, a part's
    /// planner step; `None` on every other row today. Written once.
    #[serde(default)]
    pub parent_step_id: Option<StepId>,
    /// `run_step.node_key` (MOD-139 D3, ANA-33 §7.4, §7.11): a part's `<divided>/<part_key>`, or a
    /// stage's `pick` | `prepare` | `collect` ([`STAGE_KEYS`]); `None` on a node of the frozen
    /// snapshot, which `position` names. Written once.
    #[serde(default)]
    pub node_key: Option<String>,
    /// `run_step.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Fractional-second digits Postgres `timestamptz` keeps: microseconds (§5.8).
///
/// Public because [`ChatRunSpec::mint`] is no longer the only caller that has to respect it. Every
/// ANA-2 §8 writer takes its `started_at` / `finished_at` / `lease_expires_at` / `promoted_at` from
/// the caller (blueprint F-S) and hands the stored value back, so a clock reading finer than this
/// comes back from Postgres different from the one the caller still holds — while `MemStore` keeps
/// it whole, and the two backends disagree about a column neither of them changed.
pub const TIMESTAMPTZ_DIGITS: u16 = 6;

/// `run_step.phase_name` of a chat's only step (MOD-2 plan D4).
pub const CHAT_PHASE: &str = "chat";

/// `run_step.phase_name` of an editor's help turn (MOD-55 plan P5): still `run.kind = 'chat'`.
pub const EDIT_HELP_PHASE: &str = "edit_help";

/// `ChatRunSpec.phase_name`'s serde default (MOD-55 amendment A-3): a spec written before the
/// field existed is a chat, and `#[serde(default)]` alone would read it back as `""`.
fn chat_phase() -> String {
    CHAT_PHASE.to_owned()
}

/// The two rows a free-standing chat needs before its first event can be recorded: one `run`
/// (`kind = 'chat'`, `item_id NULL`) and one `run_step` (`phase_name` [`CHAT_PHASE`], or
/// [`EDIT_HELP_PHASE`] for a help turn, position 0) (MOD-2 plan D4).
///
/// The ids are minted client-side and both inserts of
/// [`WriteStore::start_chat_run`](crate::store::WriteStore::start_chat_run) are
/// `ON CONFLICT (id) DO NOTHING`, so a start retried after an answer that never arrived lands on
/// the pair the first attempt wrote: one `run` and one `run_step` for the chat, never a second pair
/// and never a duplicate-key error (`docs/ANA-9.md` §4.3). Before MOD-25 the same ids also named
/// the rows of a chat buffered offline under `<cache_dir>/pending/` and uploaded later; that path
/// has since been removed, and `start_chat_run` is the only writer of the pair.
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
    /// `run_step.phase_name`: [`CHAT_PHASE`], or [`EDIT_HELP_PHASE`] through
    /// [`ChatRunSpec::for_edit_help`] (MOD-55 plan P5). No CHECK constrains the column.
    #[serde(default = "chat_phase")]
    pub phase_name: String,
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
            phase_name: CHAT_PHASE.to_owned(),
        }
    }

    /// MOD-55 plan P5: the same pair, marked as an editor's help turn.
    #[must_use]
    pub fn for_edit_help(mut self) -> Self {
        self.phase_name = EDIT_HELP_PHASE.to_owned();
        self
    }
}

/// Arguments of [`crate::store::WriteStore::create_run`]: a `kind = 'graph'` run inserted at
/// `queued` (ANA-2 §4.3, §5.1); the item is not written (MOD-144: it reads `queued` from the run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRun {
    /// `run.id`, minted by the caller so a retry is idempotent.
    pub id: RunId,
    /// `run.project_id`.
    pub project_id: ProjectId,
    /// `run.item_id`: a graph run always has one.
    pub item_id: ItemId,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.target_box_id`.
    pub target_box_id: BoxId,
    /// `run.started_by`.
    pub started_by: UserId,
    /// `run.graph_snapshot`, serialised by the store (`R-ORCH-11`).
    pub graph_snapshot: GraphSnapshot,
    /// `run.repo_scope` (ANA-2 §4.7).
    ///
    /// An empty scope overlaps nothing, so the admission of
    /// [`claim_run`](crate::store::WriteStore::claim_run) never refuses it: a caller that resolves
    /// an item's `touched_paths` to repos must never queue an empty scope for an item that has a
    /// primary repo (§4.7's "an empty `touched_paths` overlaps the whole primary repo" is the
    /// resolver's rule, not the seam's).
    pub repo_scope: Vec<RepoId>,
    /// `run.queued_at`; the caller's clock, microsecond-truncated like [`ChatRunSpec::started_at`].
    pub queued_at: DateTime<Utc>,
    /// `run.batch_id` (MOD-12 D7): the queue batch an auto run was admitted under; `None` for a
    /// manual run. Not a [`Run`] field: only the runner and milestone 2's spend sum read it, through
    /// `batch_runs`.
    #[serde(default)]
    pub batch_id: Option<BatchId>,
    /// `run.workspace_id` (MOD-125 D3); `None` is never gated by a workspace stop.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
}

/// Arguments of [`crate::store::WriteStore::create_step`]: a `run_step` inserted at `pending`
/// with every settle column `NULL`. `UNIQUE (run_id, position, attempt, fanout_index)` is the
/// database's; a repeat is `Constraint`. `role`, `parent_step_id` and `node_key` must satisfy
/// [`run_step_refusal`](crate::store::run_step_refusal) (MOD-139 D4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRunStep {
    /// `run_step.id`.
    pub id: StepId,
    /// `run_step.run_id`.
    pub run_id: RunId,
    /// `run_step.position`.
    pub position: i32,
    /// `run_step.attempt`, 1-based.
    pub attempt: i32,
    /// `run_step.fanout_index`; `-1` for the judge.
    pub fanout_index: i32,
    /// `run_step.phase_name`.
    pub phase_name: String,
    /// `run_step.agent_id`.
    pub agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub model: Option<String>,
    /// `run_step.role` (MOD-139 D3); [`StepRole::Step`] at every engine site in M1 (D7).
    pub role: StepRole,
    /// `run_step.parent_step_id` (MOD-139 D3); a part's planner step, never the step itself.
    pub parent_step_id: Option<StepId>,
    /// `run_step.node_key` (MOD-139 D3); a part's `<divided>/<part_key>`, a stage's key.
    pub node_key: Option<String>,
}

/// What [`crate::store::WriteStore::finish_step`] writes: the settle columns and nothing about
/// `status`, which only the §4.3 law moves.
///
/// `usage` and `trim_record` `None` **leave** the column — the assembler and the usage summer
/// write them earlier — while every other field overwrites.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// `run_step.exit_code`.
    pub exit_code: Option<i32>,
    /// `run_step.usage`; `None` leaves the column.
    pub usage: Option<Value>,
    /// `run_step.trim_record`; `None` leaves the column.
    pub trim_record: Option<Value>,
    /// `run_step.verify_outcome`.
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.verify_exit_code`.
    pub verify_exit_code: Option<i32>,
    /// `run_step.finished_at`.
    pub finished_at: DateTime<Utc>,
}

/// A row of `run_plan` (MOD-139 D1, D3; ANA-33 §7.2): one revision of a run's plan. Append-only in
/// revisions; only `status` (and, at admission, `base_refs`) moves after the insert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunPlan {
    /// `run_plan.id`.
    pub id: RunPlanId,
    /// `run_plan.run_id`.
    pub run_id: RunId,
    /// `run_plan.revision`: 0 for default parts the engine admits itself, then one per planner
    /// submission; `UNIQUE (run_id, revision)`.
    pub revision: i32,
    /// `run_plan.planner_step_id`: a step of the same run.
    pub planner_step_id: StepId,
    /// `run_plan.scope` (`step_graph_phase.plan_scope`'s vocabulary).
    pub scope: PlanScope,
    /// `run_plan.status`.
    pub status: PlanStatus,
    /// `run_plan.base_position`: the `run_step.position` of the revision's first planned node.
    pub base_position: i32,
    /// `run_plan.base_refs` (`JSONB`): the primary HEAD per repo at admission; `None` until
    /// admitted, and always set on an admitted revision (`chk_run_plan_admitted_base_refs`).
    pub base_refs: Option<Value>,
    /// `run_plan.body` (`JSONB`): the plan itself (ANA-33 §7.6). Untyped in M1 (D3): MOD-140's
    /// validator owns its shape, and M2 types default parts.
    pub body: Value,
    /// `run_plan.digest`: sha256 over the canonical body, recomputed on resume (ANA-33 §7.2).
    pub digest: String,
    /// `run_plan.created_at`.
    pub created_at: DateTime<Utc>,
    /// `run_plan.updated_at` (an addition to §7.2, plan D1): moves with `status`; the mirror's
    /// cursor.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::insert_run_plan`] (MOD-139 D3, D4): a revision inserted
/// at [`PlanStatus::Proposed`] with no `base_refs`. Must pass
/// [`run_plan_refusal`](crate::store::run_plan_refusal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRunPlan {
    /// `run_plan.id`, client-minted.
    pub id: RunPlanId,
    /// `run_plan.run_id`.
    pub run_id: RunId,
    /// `run_plan.revision`, `>= 0`.
    pub revision: i32,
    /// `run_plan.planner_step_id`: a step of `run_id`.
    pub planner_step_id: StepId,
    /// `run_plan.scope`.
    pub scope: PlanScope,
    /// `run_plan.base_position`, `>= 0`.
    pub base_position: i32,
    /// `run_plan.body`.
    pub body: Value,
    /// `run_plan.digest`.
    pub digest: String,
}

/// MOD-139 M2 D9: which revision [`crate::store::WriteStore::admit_plan`] admits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanToAdmit {
    /// The engine's own revision (default parts), inserted straight at `admitted` with its
    /// `base_refs` in one statement (0029's admitted-needs-base-refs CHECK forbids two).
    New(NewRunPlan),
    /// A revision already written `proposed` (MOD-140's `plan_submit`; M2 reaches it only through
    /// a direct `insert_run_plan` in conformance), moved `proposed -> admitted`.
    Proposed(RunPlanId),
}

/// MOD-139 M2 D9 (blueprint P-1): one admission, decided and written in one transaction by
/// [`crate::store::WriteStore::admit_plan`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanAdmission {
    /// The run admitted into; a `New` plan names it too.
    pub run_id: RunId,
    /// The planner step: `running` before, `done` after. A `New` plan names it too, and every
    /// part's `parent_step_id` is it.
    pub planner_step_id: StepId,
    /// The revision admitted.
    pub plan: PlanToAdmit,
    /// The primary HEAD per repo (`Isolator::base`): a JSON object of repo id to commit string,
    /// possibly empty (an empty scope).
    pub base_refs: Value,
    /// Every part row, inserted `pending`; `role = part`, same run, parent = the planner.
    pub parts: Vec<NewRunStep>,
    /// The caller's instant: the planner's `finished_at` (`COALESCE`d).
    pub at: DateTime<Utc>,
}

/// A row of `run_step_tree` (ANA-2 §4.6): the isolation tree of one repo for one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStepTree {
    /// `run_step_tree.run_step_id`.
    pub run_step_id: StepId,
    /// `run_step_tree.repo_id`.
    pub repo_id: RepoId,
    /// `run_step_tree.mode`; the same `CHECK` list as `step_graph_phase.isolation`.
    pub mode: Isolation,
    /// `run_step_tree.path`: absolute, on the executing box.
    pub path: String,
    /// `run_step_tree.base_ref`.
    pub base_ref: String,
    /// `run_step_tree.dirty`.
    pub dirty: bool,
}

/// `run.graph_snapshot` (`R-ORCH-11`, ANA-2 §5.1): the graph as it was at queue time.
///
/// The only route to a graph while offline, since `step_graph` is not mirrored. Every optional
/// field carries `#[serde(default)]` so a snapshot written by a later builder still decodes here;
/// [`Run::graph_snapshot`] stays an untyped `Value`, because a reader that only lists runs never
/// decodes it, while [`NewRun`] carries the typed form and the store serialises it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// Schema version; a reader that meets an unknown `v` refuses rather than guesses.
    pub v: u32,
    /// The graph row this snapshot was taken from.
    pub graph: SnapshotGraph,
    /// MOD-138 D1: how the run's graph was chosen; `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SnapshotSource>,
    /// `sha256:…` digest of the topology. v1: `graph::topology` over the canonical `phases[]`;
    /// v2: `graph::topology_v2` over `phases[]` and `edges[]` (MOD-138 D3). The envelope (`v`,
    /// `graph`, `source`, `mode`, `settings`, `scope`, `personas`, `memory`) is outside both.
    pub topology: String,
    /// `run.mode`, repeated so the snapshot is self-contained.
    pub mode: RunMode,
    /// The phases in `position` order: the graph's nodes (ANA-33 §7.2's `nodes`; the JSON key
    /// stays `phases`, MOD-138 D1).
    pub phases: Vec<SnapshotPhase>,
    /// MOD-138 D2: the graph's edges by node key, in [`sort_snapshot_edges`] order. Empty in a
    /// v1 blob; [`GraphSnapshot::lift_v1`] fills them (D4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SnapshotEdge>,
    /// The project settings the resolution used.
    pub settings: SnapshotSettings,
    /// Plan D79: §4.7's resolved scope, written by `graph::resolve` at `StartRun`. **Not** hashed
    /// by `topology` (which reads `phases[]` alone) and [`GraphSnapshot::V`] is not bumped for it;
    /// `None` on every snapshot written before milestone 5, read conservatively by
    /// [`scope_of`](crate::model::overlap::scope_of).
    #[serde(default)]
    pub scope: Option<crate::model::overlap::RunScope>,
    /// MOD-26 D9: every persona a phase names, frozen at `StartRun`, one per name, ordered by
    /// name bytes. **Not** hashed by `topology` (editing a persona's body never parks a run,
    /// I-3) and [`GraphSnapshot::V`] is not bumped; absent from the JSON when empty (B-10).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personas: Vec<crate::model::persona::SnapshotPersona>,
    /// MOD-163 D11: the project's active memory entries at `StartRun`, selected under
    /// `MEMORY_TOKEN_CAP` and frozen with their text. **Not** hashed by `topology` and
    /// [`GraphSnapshot::V`] is not bumped; absent from the JSON when `None`; `None` on every
    /// snapshot written before MOD-163, which renders no memory section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<crate::model::memory::SnapshotMemory>,
}

impl GraphSnapshot {
    /// The `v` this crate writes (MOD-138 D1): nodes, edges, bounds, the graph's scope and
    /// revision, digested over nodes *and* edges.
    pub const V: u32 = 2;

    /// The oldest `v` this crate reads, lifted by [`Self::lift_v1`] (MOD-138 D4).
    pub const V1: u32 = 1;

    /// MOD-138 D4: a v1 snapshot's edges are `seed::default_edges`' rule over its own phases
    /// ([`default_snapshot_edges`]). Keeps `v == 1`, so resume can tell a v1 run from a v2 one;
    /// a no-op on a v2 snapshot, whose edges are whatever it froze.
    pub fn lift_v1(&mut self) {
        if self.v == Self::V1 && self.edges.is_empty() {
            self.edges = default_snapshot_edges(&self.phases);
        }
    }

    /// MOD-138 M2 D1: the edges every walker reads: the stored ones, or for a `v == V1` blob
    /// holding none (decoded by serde, not through `snapshot_of`, so never lifted) the lift,
    /// computed here. A v2 snapshot with no edges is **not** lifted: its nodes are roots.
    #[must_use]
    pub fn walk_edges(&self) -> std::borrow::Cow<'_, [SnapshotEdge]> {
        if self.v == Self::V1 && self.edges.is_empty() {
            std::borrow::Cow::Owned(default_snapshot_edges(&self.phases))
        } else {
            std::borrow::Cow::Borrowed(&self.edges)
        }
    }

    /// MOD-138 M2 D7: `node` reads its verdict iff it has an outgoing `changes_requested` edge
    /// (over [`Self::walk_edges`]), whatever it is called.
    #[must_use]
    pub fn reads_verdict(&self, node: &str) -> bool {
        self.walk_edges()
            .iter()
            .any(|edge| edge.from == node && edge.kind == EdgeKind::ChangesRequested)
    }

    /// MOD-144 (ANA-35 §7.1 `default_for`): the `output_kind` of the snapshot's last phase, read
    /// from the raw JSON so any `v` decodes; `None` without phases. A phase with no readable
    /// `position` or `output_kind` is skipped.
    ///
    /// MOD-138 M3 D9: "last" follows the graph. With an `edges` array it is the highest-`position`
    /// non-palette **sink**, a phase with no forward out-edge ([`forward_mask`] over the decoded
    /// edges, so a back edge does not count). Without edges (a v1 blob), or when no sink has a
    /// readable kind, it is the highest `position`, as before. An edge whose ends or kind do not
    /// decode is left out.
    ///
    /// [`forward_mask`]: crate::store::forward_mask
    #[must_use]
    pub fn last_output_kind(snapshot: &Value) -> Option<String> {
        let phases = snapshot.get("phases")?.as_array()?;
        let highest = |keep: &dyn Fn(usize) -> bool| {
            phases
                .iter()
                .enumerate()
                .filter(|(index, _)| keep(*index))
                .filter_map(|(_, phase)| {
                    Some((
                        phase.get("position")?.as_i64()?,
                        phase.get("output_kind")?.as_str()?,
                    ))
                })
                .max_by_key(|(position, _)| *position)
                .map(|(_, kind)| kind.to_owned())
        };
        if let Some(edges) = snapshot.get("edges").and_then(Value::as_array) {
            let sinks = raw_sinks(phases, edges);
            if let Some(kind) = highest(&|index| sinks[index]) {
                return Some(kind);
            }
        }
        highest(&|_| true)
    }

    /// MOD-26 D12, I-4: the frozen persona `phase` names. `Ok(None)` for a persona-less phase;
    /// `Err` is [`PersonaNotInSnapshot`](crate::model::persona::PersonaNotInSnapshot) when the
    /// snapshot does not carry the name.
    ///
    /// # Errors
    /// As above.
    pub fn persona_for(
        &self,
        phase: &SnapshotPhase,
    ) -> Result<
        Option<&crate::model::persona::SnapshotPersona>,
        crate::model::persona::PersonaNotInSnapshot,
    > {
        let Some(name) = phase.persona.as_deref() else {
            return Ok(None);
        };
        self.personas
            .iter()
            .find(|persona| persona.name == name)
            .map(Some)
            .ok_or_else(|| crate::model::persona::PersonaNotInSnapshot {
                persona: name.to_owned(),
            })
    }
}

/// MOD-138 M3 D9: per raw phase, whether it is a non-palette sink: no forward out-edge per
/// [`crate::store::forward_mask`] over the raw edges that decode (both keys name a phase, the
/// first holding it; the kind parses).
fn raw_sinks(phases: &[Value], edges: &[Value]) -> Vec<bool> {
    use crate::store::{WalkEdge, WalkNode, forward_mask};

    let names: Vec<Option<&str>> = phases
        .iter()
        .map(|phase| phase.get("name").and_then(Value::as_str))
        .collect();
    let nodes: Vec<WalkNode<'_>> = phases
        .iter()
        .zip(&names)
        .map(|(phase, name)| WalkNode {
            name: name.unwrap_or_default(),
            palette: phase
                .get("palette")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect();
    let index = |key: &str| names.iter().position(|name| *name == Some(key));
    let walk_edges: Vec<WalkEdge> = edges
        .iter()
        .filter_map(|edge| {
            Some(WalkEdge {
                from: index(edge.get("from")?.as_str()?)?,
                to: index(edge.get("to")?.as_str()?)?,
                kind: serde_json::from_value(edge.get("kind")?.clone()).ok()?,
            })
        })
        .collect();
    let mut sinks: Vec<bool> = nodes.iter().map(|node| !node.palette).collect();
    for (edge, forward) in walk_edges.iter().zip(forward_mask(&nodes, &walk_edges)) {
        if forward {
            sinks[edge.from] = false;
        }
    }
    sinks
}

/// The `step_graph` half of a [`GraphSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotGraph {
    /// `step_graph.id`.
    pub id: StepGraphId,
    /// `step_graph.name`.
    pub name: String,
    /// `step_graph.is_override` (ANA-2 §4.1).
    #[serde(default)]
    pub is_override: bool,
    /// MOD-138 D1: the graph's owner (`{"owner":"project","id":…}`, `workspace` or
    /// `{"owner":"library"}`); `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<GraphScope>,
    /// MOD-138 D1: `step_graph.revision` at resolve; `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
}

/// One resolved phase of a [`GraphSnapshot`] (ANA-2 §5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPhase {
    /// `step_graph_phase.position`.
    pub position: i32,
    /// `step_graph_phase.name`.
    pub name: String,
    /// `step_graph_phase.fan_out`.
    pub fan_out: i32,
    /// `step_graph_phase.gate`, as configured.
    pub gate: Gate,
    /// `gate` after the `R-ORCH-6` downgrade; equal to `gate` when nothing downgraded it.
    pub gate_effective: Gate,
    /// `step_graph_phase.gate_hard`.
    pub gate_hard: bool,
    /// `step_graph_phase.retry_limit`.
    pub retry_limit: i32,
    /// `step_graph_phase.input_kinds`.
    pub input_kinds: Vec<String>,
    /// `step_graph_phase.output_kind`.
    pub output_kind: String,
    /// Resolved: the phase's own, else [`SnapshotSettings::default_isolation`].
    pub isolation: Isolation,
    /// `step_graph_phase.command_queue`.
    pub command_queue: CommandQueue,
    /// `step_graph_phase.verify_command`.
    #[serde(default)]
    pub verify_command: Option<String>,
    /// `step_graph_phase.deadline_seconds`, resolved against the project rung.
    #[serde(default)]
    pub deadline_seconds: Option<u32>,
    /// The prompt template, resolved to a concrete version.
    pub template: SnapshotTemplate,
    /// `step_graph_phase.token_budget`.
    #[serde(default)]
    pub token_budget: Option<i32>,
    /// The `phase_agent` rows, in `position` order.
    pub candidates: Vec<SnapshotCandidate>,
    /// The fan-out judge, when the phase has one.
    #[serde(default)]
    pub judge: Option<SnapshotJudge>,
    /// MOD-26 D9: the name of the persona this phase runs under, frozen with its content in
    /// [`GraphSnapshot::personas`]. Hashed by `topology` when bound; absent from the JSON, and
    /// so from the digest, when `None` (I-7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    // MOD-138 D1: the four node fields come **after** `persona` and are skipped at their
    // defaults, so a linear phase serialises byte-identically to v1 and its v1 digest holds.
    /// MOD-138 D1: what the node is; absent from the JSON, and so from both digests, for `step`.
    #[serde(default, skip_serializing_if = "NodeRole::is_step")]
    pub role: NodeRole,
    /// `step_graph_phase.palette`; absent when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub palette: bool,
    /// `step_graph_phase.plan_scope`; absent when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_scope: Option<PlanScope>,
    /// MOD-138 D11: `Some` exactly on a node whose stored `split` is `divide`; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divide: Option<SnapshotDivide>,
}

impl SnapshotPhase {
    /// MOD-138 amendment A1 (F1): [`Split::Divide`] when the node carries a divide block, else
    /// what `fan_out` implies (`ck_phase_split_fan_out`). Derived, never serialised.
    #[must_use]
    pub const fn split(&self) -> Split {
        if self.divide.is_some() {
            Split::Divide
        } else {
            Split::for_fan_out(self.fan_out)
        }
    }
}

/// `skip_serializing_if` for a `bool` that is absent when `false`.
const fn is_false(value: &bool) -> bool {
    !*value
}

/// The prompt template of a [`SnapshotPhase`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotTemplate {
    /// `prompt_template.name`.
    pub name: String,
    /// Resolved: never `None` in a snapshot, unlike `step_graph_phase.template_version`.
    pub version: i32,
}

/// One candidate agent of a [`SnapshotPhase`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotCandidate {
    /// `phase_agent.agent_id`.
    pub agent_id: AgentId,
    /// `agent.name`, denormalised so an offline reader needs no registry.
    pub agent_name: String,
    /// `phase_agent.model`.
    pub model: String,
}

/// The fan-out judge of a [`SnapshotPhase`] (ANA-2 §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotJudge {
    /// `step_graph_phase.judge_agent_id`.
    pub agent_id: AgentId,
    /// `agent.name`.
    pub agent_name: String,
    /// `step_graph_phase.judge_model`.
    #[serde(default)]
    pub model: Option<String>,
    /// The `judge` prompt template, resolved to a concrete version. `graph::resolve` pins the
    /// latest `judge` template at run creation (plan D53); `None` in a snapshot written before
    /// milestone 4 means the latest at judge time. [`GraphSnapshot::V`] is not bumped for it.
    #[serde(default)]
    pub template: Option<SnapshotTemplate>,
}

/// The `project.settings` a [`GraphSnapshot`] resolved against (ANA-2 §5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSettings {
    /// `project.settings.default_isolation`.
    pub default_isolation: Isolation,
    /// `project.settings.per_token_cap_run`, micros.
    #[serde(default)]
    pub per_token_cap_run: Option<i64>,
    /// `project.settings.per_token_cap_batch`, micros.
    #[serde(default)]
    pub per_token_cap_batch: Option<i64>,
    /// `app_setting.max_fan_out`.
    pub max_fan_out: u32,
    /// `app_setting.max_agents_per_run`.
    pub max_agents_per_run: u32,
    /// MOD-138 D1, D13: `app_setting.max_parallel_sessions`, frozen for M3's walker. Always
    /// written; never hashed.
    #[serde(default = "SnapshotSettings::default_max_parallel_sessions")]
    pub max_parallel_sessions: u32,
    /// `app_setting.max_plan_revisions`, frozen for MOD-139 and MOD-140.
    #[serde(default = "SnapshotSettings::default_max_plan_revisions")]
    pub max_plan_revisions: u32,
    /// `app_setting.max_child_runs`, frozen for MOD-143.
    #[serde(default = "SnapshotSettings::default_max_child_runs")]
    pub max_child_runs: u32,
    /// `app_setting.runtime_changes`, frozen for MOD-140.
    #[serde(default)]
    pub runtime_changes: RuntimeChanges,
}

impl SnapshotSettings {
    /// `max_parallel_sessions` when `app_setting` names none (MOD-138 D13).
    pub const DEFAULT_MAX_PARALLEL_SESSIONS: u32 = 4;
    /// `max_plan_revisions` when `app_setting` names none (MOD-138 D13).
    pub const DEFAULT_MAX_PLAN_REVISIONS: u32 = 3;
    /// `max_child_runs` when `app_setting` names none (MOD-138 D13).
    pub const DEFAULT_MAX_CHILD_RUNS: u32 = 8;

    const fn default_max_parallel_sessions() -> u32 {
        Self::DEFAULT_MAX_PARALLEL_SESSIONS
    }

    const fn default_max_plan_revisions() -> u32 {
        Self::DEFAULT_MAX_PLAN_REVISIONS
    }

    const fn default_max_child_runs() -> u32 {
        Self::DEFAULT_MAX_CHILD_RUNS
    }
}

str_enum!(
    /// MOD-138 D1 (ANA-33 §7.2): what a frozen node is. `subgraph` is MOD-143's, added there.
    #[derive(Default)]
    NodeRole {
        /// An ordinary step; every v1 node.
        #[default]
        Step => "step",
        /// `<name>:plan` of an expanded `divide` node (D11).
        Planner => "planner",
        /// `<name>` of an expanded `divide` node (D11).
        Integrate => "integrate",
    }
);

impl NodeRole {
    /// D1's skip predicate: a `step` node serialises without `role`.
    #[must_use]
    pub const fn is_step(&self) -> bool {
        matches!(self, Self::Step)
    }
}

str_enum!(
    /// `run_step.role` (MOD-139 D3, ANA-33 §7.4, §7.11; `chk_run_step_role`): what a step **row**
    /// is. Not [`NodeRole`], the snapshot's three-value role of a frozen node: a node's role says
    /// what the walk will create, a row's role what was created. M1 writes only `step` (D7).
    #[derive(Default)]
    StepRole {
        /// An ordinary step: every row before `0029`, every compete candidate and its judge.
        #[default]
        Step => "step",
        /// A divided node's planner, `<name>:plan` ([`PLAN_SUFFIX`]); MOD-139 M2 writes it.
        Planner => "planner",
        /// One part of an admitted plan revision: `parent_step_id` is the planner step and
        /// `node_key` is `<divided>/<part_key>` (`chk_run_step_part`); MOD-139 M2 writes it.
        Part => "part",
        /// A divided node's integrate step; MOD-139 M2 writes it and M3 runs it: pre-merge,
        /// session, post-check, landing.
        Integrate => "integrate",
        /// A subgraph node's step (MOD-143).
        Subgraph => "subgraph",
        /// A read-only consult-mode subagent (MOD-27).
        Consult => "consult",
        /// One of htui's own stages at positions -3..-1, keyed by [`STAGE_KEYS`] (ANA-33 §7.11,
        /// `chk_run_step_stage`); MOD-126 and MOD-132 write them.
        Stage => "stage",
    }
);

/// The `node_key`s a [`StepRole::Stage`] row may carry, in reserved-position order: `pick` at -3,
/// `prepare` at -2, `collect` at -1 (ANA-33 §7.11). `chk_run_step_stage` lists the same three; the
/// CHECK does not bind a key to its position (blueprint F-1).
pub const STAGE_KEYS: [&str; 3] = ["pick", "prepare", "collect"];

/// The suffix of an expanded `divide` node's planner key, `<name>:plan` (MOD-138 D11). A phase may
/// not be named with it (MOD-139 D6, [`phase_name_refusal`](crate::store::phase_name_refusal)), so a
/// v2 snapshot's planner key never collides with a saved phase.
pub const PLAN_SUFFIX: &str = ":plan";

str_enum!(
    /// `run_plan.status` (MOD-139 D3, ANA-33 §7.2; `chk_run_plan_status`).
    PlanStatus {
        /// Written by [`insert_run_plan`](crate::store::WriteStore::insert_run_plan); not yet
        /// decided.
        Proposed => "proposed",
        /// Admitted: `base_refs` is set and its parts are rows (MOD-139 M2's admission).
        Admitted => "admitted",
        /// Refused at admission: the validator or a person said no.
        Refused => "refused",
        /// Left undecided once a later revision was admitted, or once the run ended.
        Abandoned => "abandoned",
    }
);

impl PlanStatus {
    /// MOD-139 M2 D9: `run_plan.status`'s law, read by both stores before a status write.
    /// `proposed -> admitted | refused | abandoned`; `admitted -> abandoned`; `refused` and
    /// `abandoned` are terminal. 0029 holds no trigger for it (C26), so the stores are the law.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Proposed => matches!(to, Self::Admitted | Self::Refused | Self::Abandoned),
            Self::Admitted => matches!(to, Self::Abandoned),
            Self::Refused | Self::Abandoned => false,
        }
    }
}

str_enum!(
    /// MOD-138 D1: how the run's graph was chosen. `pick` is MOD-126's, added there.
    SourceHow {
        /// `item.step_graph_id` named the graph (an override included).
        Pin => "pin",
        /// The item named none: its kind's `default_graph_id`.
        KindDefault => "kind_default",
    }
);

str_enum!(
    /// MOD-138 D1, D13 (ANA-33 §7.8): whether a run may change its own graph, frozen here and
    /// read by MOD-140.
    #[derive(Default)]
    RuntimeChanges {
        /// Never.
        Off => "off",
        /// A person approves each change.
        #[default]
        Ask => "ask",
        /// Applied without asking.
        Automatic => "automatic",
    }
);

/// [`GraphSnapshot::source`] (MOD-138 D1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSource {
    /// How the graph was chosen.
    pub how: SourceHow,
}

/// One edge of a v2 snapshot, by node key (MOD-138 D2): the phase `name` (`<name>:plan` for an
/// expanded planner), never a phase id or position. `predicate` stays MOD-137's opaque JSON,
/// serialised with sorted keys so neither digest depends on `serde_json`'s `preserve_order`
/// (F5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEdge {
    /// Key of the source node.
    pub from: String,
    /// Key of the target node.
    pub to: String,
    /// `step_graph_edge.kind`.
    pub kind: EdgeKind,
    /// `step_graph_edge.predicate`; `Some` iff `kind` is `if`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "sorted_predicate"
    )]
    pub predicate: Option<EdgePredicate>,
    /// `step_graph_edge.loop_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_limit: Option<i32>,
}

/// MOD-138 D1, D11: a `divide` node's frozen block. M1 never stored one (its resolve refused a
/// divide node after expanding it, D6(a)), which let MOD-139 shape it freely. From MOD-139 M3
/// resolve freezes and runs a default-parts node, so a run's snapshot holds this block and a
/// change to it is a compatibility question.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDivide {
    /// The `planner_*` columns.
    #[serde(default)]
    pub planner: SnapshotSeat,
    /// The `integrate_*` columns.
    #[serde(default)]
    pub integrate: SnapshotSeat,
    /// `step_graph_phase.max_parts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parts: Option<i32>,
    /// The `phase_part` rows, by `position` (D11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_parts: Vec<SnapshotPart>,
}

/// The planner or integrate columns of a divide row, as stored: ids, not frozen names (see
/// [`SnapshotDivide`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSeat {
    /// `*_agent_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// `*_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `*_persona_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<PersonaId>,
}

/// One `phase_part` row minus `phase_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPart {
    /// `phase_part.position`.
    pub position: i32,
    /// `phase_part.key`.
    pub key: String,
    /// `phase_part.title`.
    pub title: String,
    /// `phase_part.brief`.
    #[serde(default)]
    pub brief: String,
    /// `phase_part.owned_paths`.
    #[serde(default)]
    pub owned_paths: Vec<String>,
    /// `phase_part.acceptance`.
    #[serde(default)]
    pub acceptance: Option<String>,
    /// `phase_part.agent_id`.
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    /// `phase_part.model`.
    #[serde(default)]
    pub model: Option<String>,
    /// `phase_part.persona_id`.
    #[serde(default)]
    pub persona_id: Option<PersonaId>,
}

/// `serialize_with` for [`SnapshotEdge::predicate`] (F5): the JSON with object keys sorted by
/// bytes at every depth. Only reached on `Some`; the field is skipped on `None`.
fn sorted_predicate<S: serde::Serializer>(
    predicate: &Option<EdgePredicate>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match predicate {
        Some(EdgePredicate(value)) => SortedJson(value).serialize(serializer),
        None => serializer.serialize_none(),
    }
}

/// `Serialize` over a `&Value`: objects as maps in key-byte order, arrays element by element,
/// everything else as itself. Never re-enters `serde_json::Map`'s own order (finding 4).
#[derive(Debug)]
struct SortedJson<'a>(&'a Value);

impl Serialize for SortedJson<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap as _, SerializeSeq as _};
        match self.0 {
            Value::Object(map) => {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                let mut out = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    out.serialize_entry(key, &SortedJson(value))?;
                }
                out.end()
            }
            Value::Array(items) => {
                let mut out = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    out.serialize_element(&SortedJson(item))?;
                }
                out.end()
            }
            other => other.serialize(serializer),
        }
    }
}

/// F13's one edge order, as a sort key: (position of `from`, position of `to`, `kind` bytes,
/// `from` bytes, `to` bytes), positions looked up by key in `phases` and an unknown key at
/// `i32::MAX`. Public so `htui_orch::graph::topology_v2` sorts its borrowed view by the same
/// key and the two orders cannot drift.
#[must_use]
pub fn edge_sort_key<'a>(
    edge: &'a SnapshotEdge,
    phases: &[SnapshotPhase],
) -> (i32, i32, &'static str, &'a [u8], &'a [u8]) {
    let position = |key: &str| {
        phases
            .iter()
            .find(|phase| phase.name == key)
            .map_or(i32::MAX, |phase| phase.position)
    };
    (
        position(&edge.from),
        position(&edge.to),
        edge.kind.as_str(),
        edge.from.as_bytes(),
        edge.to.as_bytes(),
    )
}

/// F13: sorts `edges` into the one snapshot edge order ([`edge_sort_key`]), so resolve, the lift
/// and the v2 digest cannot make two equal graphs differ by order.
pub fn sort_snapshot_edges(edges: &mut [SnapshotEdge], phases: &[SnapshotPhase]) {
    edges.sort_by(|a, b| edge_sort_key(a, phases).cmp(&edge_sort_key(b, phases)));
}

/// MOD-138 D4: [`crate::seed::default_edge_plan`] over `(position, name, retry_limit)` of
/// `phases`, by key, in [`sort_snapshot_edges`] order. The v1 lift reads it. (M1's D6(c) check
/// in `htui_orch::graph::not_yet_runnable`, which refused non-default edges, is gone since
/// MOD-138 M2 D10: resolve runs authored edges.)
#[must_use]
pub fn default_snapshot_edges(phases: &[SnapshotPhase]) -> Vec<SnapshotEdge> {
    let projection: Vec<(i32, &str, i32)> = phases
        .iter()
        .map(|phase| (phase.position, phase.name.as_str(), phase.retry_limit))
        .collect();
    let mut edges: Vec<SnapshotEdge> = crate::seed::default_edge_plan(&projection)
        .into_iter()
        .map(|edge| SnapshotEdge {
            from: phases[edge.from].name.clone(),
            to: phases[edge.to].name.clone(),
            kind: edge.kind,
            predicate: None,
            loop_limit: edge.loop_limit,
        })
        .collect();
    sort_snapshot_edges(&mut edges, phases);
    edges
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

/// MOD-139 M3 D3: what `htui_orch::Isolator::premerge` did to integrate's tree, one entry
/// per tree row (a repo of the run's scope), in tree-row order. One htui-core renderer turns it into
/// integrate's protected `premerge` section and its item note (D4, D5); it is never stored
/// structurally (Confirm 1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremergeReport {
    /// One per integrate tree row; empty for a repo-less run (blueprint P-10).
    pub repos: Vec<RepoPremerge>,
}

/// MOD-139 M3 D3: one repo's pre-merge, every done part in plan order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoPremerge {
    /// `run_step_tree.repo_id`.
    pub repo_id: RepoId,
    /// The checkout's name (`RepoCheckout.name`); the fake writes the repo id.
    pub name: String,
    /// The tree's `HEAD` once the pre-merge stopped.
    pub head: String,
    /// One per done part, plan (position) order.
    pub parts: Vec<PartPremerge>,
}

/// MOD-139 M3 D3: one part in one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartPremerge {
    /// The part's step: the engine's key back to the row. Never rendered (blueprint P-9).
    pub step: StepId,
    /// `SnapshotPart.key`.
    pub key: String,
    /// The part's `after_hash` in this repo; `None` when it committed nothing here.
    pub tip: Option<String>,
    /// What the pre-merge did with it.
    pub merge: PartMerge,
}

/// MOD-139 M3 D3: the five outcomes of one part's pre-merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PartMerge {
    /// `merge --no-ff` made `commit` (subject `htui: premerge <part step>`).
    Merged {
        /// The merge commit.
        commit: String,
    },
    /// The tip was already an ancestor of the tree's `HEAD` (the single `local` part, or a part
    /// reached through another part's merge).
    Contained,
    /// No `after_hash` in this repo.
    NoCommits,
    /// The repo's first conflict; the merge was aborted. Paths in byte order, uncapped (the
    /// renderer caps, D5).
    Conflicted {
        /// `git::conflicted_paths`' answer.
        paths: Vec<String>,
    },
    /// A later part of a repo that already conflicted.
    NotAttempted,
}

str_enum!(
    /// `command_run.status` (`0001_init.sql:544`): the `R-MCP-3` queue's own vocabulary.
    ///
    /// MOD-4 milestone 3 writes two of the five. A verify command that ran to an exit code —
    /// whatever that code was — is `done`, because the queue's question is whether the command
    /// completed and `run_step.verify_outcome` is where pass and fail are told apart (ANA-2 §4.2).
    /// One that could not be run at all is `failed`, the row behind `unavailable`
    /// (`docs/ANA-2.md:515`). `queued`, `running` and `cancelled` belong to MOD-11's queue, which
    /// is the table's other writer.
    CommandRunStatus {
        /// Accepted, not yet started.
        Queued => "queued",
        /// Started and still executing.
        Running => "running",
        /// Ran to completion; `exit_code` says how it went.
        Done => "done",
        /// Could not be run, or died without an exit code.
        Failed => "failed",
        /// Stopped before it finished.
        Cancelled => "cancelled",
    }
);

/// A row of `command_run` (`0001_init.sql:537-551`), field for field (plan D31).
///
/// The durable home of one run of a step's `verify_command`: the command, where it ran, what it
/// printed and when. `run_step.verify_outcome` and `run_step.verify_exit_code` are the summary a
/// gate reads; this is the detail a human reads afterwards, and the two are written by the same
/// caller from the same result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRun {
    /// `command_run.id`, minted client-side like every other id (§3).
    pub id: CommandRunId,
    /// `command_run.run_step_id`.
    pub run_step_id: StepId,
    /// `command_run.box_id`: the box the command ran on.
    pub box_id: BoxId,
    /// `command_run.class`, `'verify'` for every row this milestone writes
    /// (`docs/ANA-2.md:501`). MOD-11's queue uses the rest of `box.settings.command_limits`.
    pub class: String,
    /// `command_run.command`, as it was handed to the shell.
    pub command: String,
    /// `command_run.cwd`: the tree the command ran in — the step's primary isolation path, or the
    /// session's own working directory when the step had no tree of its own.
    pub cwd: String,
    /// `command_run.status`.
    pub status: CommandRunStatus,
    /// `command_run.exit_code`; `None` when the command never produced one.
    pub exit_code: Option<i32>,
    /// `command_run.output`, already scrubbed and capped by the caller (`R-SEC-3`,
    /// `0001_init.sql:546`): a store neither redacts nor truncates it.
    pub output: Option<String>,
    /// `command_run.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `command_run.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `command_run.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
}

/// Arguments of [`WriteStore::record_command_run`](crate::store::WriteStore::record_command_run):
/// the same twelve fields as [`CommandRun`], every instant the caller's (blueprint F-S).
///
/// `NewX` / `X` is the shape the rest of the seam already has ([`NewRunStep`] / [`RunStep`]), and
/// the two are field-identical here for the same reason `NewRun` is not: the row has no column
/// the database fills in. `queued_at` carries a `DEFAULT now()` the seam never exercises, because
/// a writer that let the server stamp it would hand back a row the caller's own variable no longer
/// equals — the argument [`TIMESTAMPTZ_DIGITS`] makes for the rest of §8.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewCommandRun {
    /// `command_run.id`.
    pub id: CommandRunId,
    /// `command_run.run_step_id`; an unknown step is `NotFound`, not a constraint.
    pub run_step_id: StepId,
    /// `command_run.box_id`.
    pub box_id: BoxId,
    /// `command_run.class`.
    pub class: String,
    /// `command_run.command`.
    pub command: String,
    /// `command_run.cwd`.
    pub cwd: String,
    /// `command_run.status`.
    pub status: CommandRunStatus,
    /// `command_run.exit_code`.
    pub exit_code: Option<i32>,
    /// `command_run.output`, scrubbed before it gets here.
    pub output: Option<String>,
    /// `command_run.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `command_run.started_at`.
    pub started_at: Option<DateTime<Utc>>,
    /// `command_run.finished_at`.
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<NewCommandRun> for CommandRun {
    /// The stored row is the argument, because no column is the database's to fill in.
    ///
    /// This is what lets [`WriteStore::record_command_run`](crate::store::WriteStore::record_command_run)
    /// answer without a `RETURNING` clause and still promise that what comes back is what a later
    /// [`command_runs`](crate::store::WriteStore::command_runs) will read.
    fn from(new: NewCommandRun) -> Self {
        Self {
            id: new.id,
            run_step_id: new.run_step_id,
            box_id: new.box_id,
            class: new.class,
            command: new.command,
            cwd: new.cwd,
            status: new.status,
            exit_code: new.exit_code,
            output: new.output,
            queued_at: new.queued_at,
            started_at: new.started_at,
            finished_at: new.finished_at,
        }
    }
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
    /// `run_step.usage`, whole (ANA-2 §6.2).
    pub usage: Option<Value>,
    /// `run_step.selected`.
    pub selected: Option<bool>,
    /// `run_step.exit_code`.
    pub exit_code: Option<i32>,
    /// `run_step.verify_outcome`.
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.promoted_at`.
    pub promoted_at: Option<DateTime<Utc>>,
    /// `agent.name` of `agent_id`, denormalised for the Runs pane; `None` when `agent_id` is
    /// `None` or names no row.
    pub agent_name: Option<String>,
    /// `run_step.gate_note`.
    pub gate_note: Option<String>,
    /// `run_step.opening` (MOD-37 M5): how the step's promoted chat last opened; `None` when no
    /// chat was ever bound to it.
    pub opening: Option<StepOpening>,
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

/// One row of [`crate::store::ReadStore::tool_call_counts`] (MOD-72 plan D3): how many `tool_call`
/// events one step recorded of one `tool_kind`.
///
/// `tool_kind` is the wire string, not `htui_agent`'s `ToolKind`: no store crate depends on
/// `htui-agent`. A row whose payload has no JSON-string `tool_kind` counts as `"other"`, the
/// transport mappers' own fallback (`StepSummary::from_events` reads it the same way).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallCount {
    /// `session_event.run_step_id`.
    pub step: StepId,
    /// `session_event.payload ->> 'tool_kind'` when it is a string, else `"other"`.
    pub tool_kind: String,
    /// How many `tool_call` rows; never `0`.
    pub calls: u32,
}

impl ToolCallCount {
    /// Sorts by `(step, tool_kind bytes)`: every backend runs it after its own read, because
    /// Postgres would order text by collation and the mirror by bytes, the reason
    /// [`UpstreamEntry::sort_canonical`](crate::model::UpstreamEntry::sort_canonical) gives.
    pub fn sort_canonical(rows: &mut [Self]) {
        rows.sort_by(|a, b| {
            a.step
                .cmp(&b.step)
                .then_with(|| a.tool_kind.as_bytes().cmp(b.tool_kind.as_bytes()))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RunStatus, SnapshotJudge, SnapshotTemplate, StepId, StepStatus, ToolCallCount,
        prompt_summary,
    };
    use serde_json::json;
    use uuid::Uuid;

    /// Every row of ANA-2 §4.3's `run` transition table (`docs/ANA-2.md:653-687`), transcribed
    /// from the document rather than from [`RunStatus::can_move_to`].
    const RUN_SANCTIONED: &[(RunStatus, RunStatus)] = &[
        // `queued`
        (RunStatus::Queued, RunStatus::Running),
        (RunStatus::Queued, RunStatus::Cancelled),
        (RunStatus::Queued, RunStatus::Failed),
        // `running`
        (RunStatus::Running, RunStatus::AwaitingApproval),
        (RunStatus::Running, RunStatus::Done),
        (RunStatus::Running, RunStatus::Failed),
        (RunStatus::Running, RunStatus::Cancelled),
        // `awaiting_approval`
        (RunStatus::AwaitingApproval, RunStatus::Running),
        (RunStatus::AwaitingApproval, RunStatus::Failed),
        (RunStatus::AwaitingApproval, RunStatus::Cancelled),
        // MOD-125 M1's amendment rows (D1)
        (RunStatus::Running, RunStatus::Stopped),
        (RunStatus::Stopped, RunStatus::Running),
        (RunStatus::Stopped, RunStatus::Cancelled),
        (RunStatus::Stopped, RunStatus::Failed),
        // `done`, `failed`, `cancelled` are terminal.
    ];

    /// Every row of ANA-2 §4.3's `run_step` transition table (`docs/ANA-2.md:700-740`),
    /// transcribed from the document rather than from [`StepStatus::can_move_to`]. The
    /// `awaiting_approval -> awaiting_approval` self-move is §4.8's promotion row, and
    /// `failed -> awaiting_approval` is the same row from the escalation side.
    const STEP_SANCTIONED: &[(StepStatus, StepStatus)] = &[
        // `pending`
        (StepStatus::Pending, StepStatus::Running),
        (StepStatus::Pending, StepStatus::Superseded),
        (StepStatus::Pending, StepStatus::Cancelled),
        // `running`
        (StepStatus::Running, StepStatus::AwaitingApproval),
        (StepStatus::Running, StepStatus::Done),
        (StepStatus::Running, StepStatus::Failed),
        (StepStatus::Running, StepStatus::Cancelled),
        // `awaiting_approval`
        (StepStatus::AwaitingApproval, StepStatus::Done),
        (StepStatus::AwaitingApproval, StepStatus::Failed),
        (StepStatus::AwaitingApproval, StepStatus::Superseded),
        (StepStatus::AwaitingApproval, StepStatus::AwaitingApproval),
        (StepStatus::AwaitingApproval, StepStatus::Cancelled),
        // `failed` — terminal except for the promotion row, and cancellable with its run
        // (derived from row 724 + row 726's exception: ANA-2 §4.3's step table sanctions
        // `failed -> awaiting_approval` only through §4.8's promotion, blueprint C-4).
        (StepStatus::Failed, StepStatus::AwaitingApproval),
        (StepStatus::Failed, StepStatus::Cancelled),
        // `done`
        (StepStatus::Done, StepStatus::Superseded),
        // MOD-125 M1's amendment rows (D1)
        (StepStatus::Running, StepStatus::Stopped),
        (StepStatus::Stopped, StepStatus::Running),
        (StepStatus::Stopped, StepStatus::Cancelled),
        (StepStatus::Stopped, StepStatus::Failed),
        (StepStatus::Stopped, StepStatus::Superseded),
        // `cancelled` and `superseded` are terminal.
    ];

    #[test]
    fn the_run_status_table_sanctions_exactly_the_ana_2_pairs() {
        for &from in RunStatus::ALL {
            for &to in RunStatus::ALL {
                let sanctioned = RUN_SANCTIONED.contains(&(from, to));
                assert_eq!(
                    from.can_move_to(to),
                    sanctioned,
                    "run.status `{from}` -> `{to}`: the table says {sanctioned}"
                );
            }
        }
    }

    #[test]
    fn the_step_status_table_sanctions_exactly_the_ana_2_pairs() {
        for &from in StepStatus::ALL {
            for &to in StepStatus::ALL {
                let sanctioned = STEP_SANCTIONED.contains(&(from, to));
                assert_eq!(
                    from.can_move_to(to),
                    sanctioned,
                    "run_step.status `{from}` -> `{to}`: the table says {sanctioned}"
                );
            }
        }
    }

    /// A terminal run reaches nothing, which is the same set [`RunStatus::is_active`] excludes.
    #[test]
    fn a_terminal_run_status_is_one_that_reaches_nothing() {
        for &from in RunStatus::ALL {
            let reaches_nothing = RunStatus::ALL.iter().all(|&to| !from.can_move_to(to));
            assert_eq!(
                from.is_terminal(),
                reaches_nothing,
                "run.status `{from}`: is_terminal and reaching nothing are the same set"
            );
            assert_eq!(
                from.is_terminal(),
                !from.is_active(),
                "run.status `{from}`: is_terminal is the complement of is_active"
            );
        }
    }

    /// MOD-125 D2: `stopped` is live (it may still move) but off the top bar's count, which keeps
    /// counting "running and waiting". The slot half is pinned by the store conformance case
    /// `a_stopped_run_holds_no_slot_but_still_overlaps`.
    #[test]
    fn a_stopped_run_is_live_and_holds_no_slot() {
        assert!(RunStatus::Stopped.is_active(), "a stopped run is live");
        assert!(
            !RunStatus::Stopped.is_terminal(),
            "a stopped run is not terminal"
        );
        assert!(
            !RunStatus::Stopped.on_top_bar(),
            "a stopped run is off the top bar's count"
        );
        assert!(
            RunStatus::Running.on_top_bar(),
            "a running run is on the top bar's count"
        );
        assert!(
            !StepStatus::Stopped.is_terminal(),
            "a stopped step is not terminal"
        );
    }

    /// MOD-144 (ANA-35 §7.1 `default_for`): the highest `position` wins whatever the array
    /// order; a phase without `output_kind` is skipped; no phases → `None`.
    #[test]
    fn last_output_kind_reads_the_highest_position() {
        use super::GraphSnapshot;
        let snapshot = json!({
            "v": 1,
            "phases": [
                {"position": 2, "output_kind": "verdict"},
                {"position": 0, "output_kind": "plan"},
                {"position": 3},
                {"position": 1, "output_kind": "code"},
            ]
        });
        assert_eq!(
            GraphSnapshot::last_output_kind(&snapshot).as_deref(),
            Some("verdict")
        );
        assert_eq!(GraphSnapshot::last_output_kind(&json!(null)), None);
        assert_eq!(GraphSnapshot::last_output_kind(&json!({"v": 1})), None);
        assert_eq!(
            GraphSnapshot::last_output_kind(&json!({"phases": []})),
            None
        );
        assert_eq!(
            GraphSnapshot::last_output_kind(&json!({"phases": [{"position": 0}]})),
            None
        );
    }

    /// MOD-138 M3 D9: with an `edges` array, the highest-position non-palette sink (no forward
    /// out-edge, per `forward_mask`); a back edge does not make a node a source.
    #[test]
    fn last_output_kind_reads_the_graphs_sink() {
        use super::GraphSnapshot;
        let mut diamond = json!({
            "v": 2,
            "phases": [
                {"position": 0, "name": "a", "output_kind": "plan"},
                {"position": 1, "name": "d", "output_kind": "verdict"},
                {"position": 2, "name": "b", "output_kind": "code"},
                {"position": 3, "name": "c", "output_kind": "docs"},
                {"position": 4, "name": "p", "output_kind": "note", "palette": true},
            ],
            "edges": [
                {"from": "a", "to": "b", "kind": "next"},
                {"from": "a", "to": "c", "kind": "next"},
                {"from": "b", "to": "d", "kind": "next"},
                {"from": "c", "to": "d", "kind": "next"},
                {"from": "d", "to": "b", "kind": "changes_requested", "loop_limit": 2},
            ]
        });
        assert_eq!(
            GraphSnapshot::last_output_kind(&diamond).as_deref(),
            Some("verdict"),
            "the join is the one sink, below the arms' positions; the palette node is no sink"
        );
        diamond.as_object_mut().expect("an object").remove("edges");
        assert_eq!(
            GraphSnapshot::last_output_kind(&diamond).as_deref(),
            Some("note"),
            "without edges: the highest position, as before"
        );

        let branch = json!({
            "v": 2,
            "phases": [
                {"position": 0, "name": "a", "output_kind": "plan"},
                {"position": 1, "name": "b", "output_kind": "code"},
                {"position": 2, "name": "c", "output_kind": "docs"},
                {"position": 3, "name": "x", "output_kind": "notes"},
            ],
            "edges": [
                {"from": "a", "to": "b", "kind": "if"},
                {"from": "a", "to": "c", "kind": "if"},
                {"from": "c", "to": "a", "kind": "asks_you"},
                {"from": "x", "to": "b", "kind": "next"},
            ]
        });
        assert_eq!(
            GraphSnapshot::last_output_kind(&branch).as_deref(),
            Some("docs"),
            "two sinks, `b` and `c`: the higher; `c`'s back `asks_you` is not forward, and `x` is a source"
        );

        let unreadable = json!({
            "v": 2,
            "phases": [
                {"position": 0, "name": "a", "output_kind": "plan"},
                {"position": 1, "name": "b"},
            ],
            "edges": [{"from": "a", "to": "b", "kind": "next"}]
        });
        assert_eq!(
            GraphSnapshot::last_output_kind(&unreadable).as_deref(),
            Some("plan"),
            "no sink with a readable kind: the highest position that has one"
        );
    }

    /// MOD-144: the latest graph run skips `cancelled` and chat runs, orders by `queued_at`, and
    /// breaks a `queued_at` tie on the id, over [`Run`](super::Run) and
    /// [`RunSummary`](super::RunSummary) alike.
    #[test]
    fn latest_graph_run_skips_cancelled_and_chat_and_breaks_ties_on_id() {
        use super::{Run, RunId, RunKind, RunMode, RunSummary, latest_graph_run};
        use crate::model::ids::{BoxId, ProjectId, UserId};
        use chrono::{TimeZone as _, Utc};

        let at = |minute: u32| Utc.with_ymd_and_hms(2026, 10, 27, 9, minute, 0).unwrap();
        let id = |n: u128| RunId::from_uuid(Uuid::from_u128(n));
        let run = |n: u128, minute: u32, kind: RunKind, status: RunStatus| Run {
            id: id(n),
            project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
            item_id: None,
            workspace_id: None,
            kind,
            mode: RunMode::Manual,
            status,
            target_box_id: BoxId::from_uuid(Uuid::from_u128(1)),
            executing_box_id: None,
            graph_snapshot: None,
            started_by: UserId::from_uuid(Uuid::from_u128(1)),
            queued_at: at(minute),
            started_at: None,
            finished_at: None,
            failure: None,
            repo_scope: Vec::new(),
            lease_box_id: None,
            lease_expires_at: None,
            updated_at: at(minute),
        };
        let runs = vec![
            run(1, 1, RunKind::Graph, RunStatus::Done),
            run(2, 2, RunKind::Graph, RunStatus::Failed),
            run(3, 2, RunKind::Graph, RunStatus::Failed),
            run(4, 3, RunKind::Graph, RunStatus::Cancelled),
            run(5, 4, RunKind::Chat, RunStatus::Running),
        ];
        assert_eq!(latest_graph_run(&runs).map(|r| r.id), Some(id(3)));
        assert_eq!(latest_graph_run(&runs[..1]).map(|r| r.id), Some(id(1)));
        assert!(latest_graph_run(&runs[3..]).is_none());
        assert!(latest_graph_run(core::iter::empty::<&Run>()).is_none());

        let summaries: Vec<RunSummary> = runs
            .iter()
            .map(|r| RunSummary {
                id: r.id,
                item_id: r.item_id,
                project_id: r.project_id,
                kind: r.kind,
                mode: r.mode,
                status: r.status,
                target_box_id: r.target_box_id,
                executing_box_id: None,
                box_hostname: String::new(),
                queued_at: r.queued_at,
                started_at: None,
                finished_at: None,
                failure: None,
                steps: Vec::new(),
            })
            .collect();
        assert_eq!(latest_graph_run(&summaries).map(|r| r.id), Some(id(3)));
    }

    /// MOD-144 Δ2: the latest **live** graph run reads every live status (`stopped` included),
    /// skips finished and chat runs, and breaks a `queued_at` tie on the id.
    #[test]
    fn latest_live_graph_run_reads_only_live_graph_runs() {
        use super::{Run, RunId, RunKind, RunMode, latest_live_graph_run};
        use crate::model::ids::{BoxId, ProjectId, UserId};
        use chrono::{TimeZone as _, Utc};

        let at = |minute: u32| Utc.with_ymd_and_hms(2026, 10, 27, 9, minute, 0).unwrap();
        let id = |n: u128| RunId::from_uuid(Uuid::from_u128(n));
        let run = |n: u128, minute: u32, kind: RunKind, status: RunStatus| Run {
            id: id(n),
            project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
            item_id: None,
            workspace_id: None,
            kind,
            mode: RunMode::Manual,
            status,
            target_box_id: BoxId::from_uuid(Uuid::from_u128(1)),
            executing_box_id: None,
            graph_snapshot: None,
            started_by: UserId::from_uuid(Uuid::from_u128(1)),
            queued_at: at(minute),
            started_at: None,
            finished_at: None,
            failure: None,
            repo_scope: Vec::new(),
            lease_box_id: None,
            lease_expires_at: None,
            updated_at: at(minute),
        };
        for live in RunStatus::ALL.iter().copied().filter(|s| s.is_active()) {
            let runs = vec![
                run(1, 1, RunKind::Graph, live),
                run(2, 2, RunKind::Graph, RunStatus::Done),
                run(3, 3, RunKind::Chat, RunStatus::Running),
            ];
            assert_eq!(
                latest_live_graph_run(&runs).map(|r| (r.id, r.status)),
                Some((id(1), live)),
                "`{live}` is live"
            );
        }
        let tie = vec![
            run(1, 1, RunKind::Graph, RunStatus::Stopped),
            run(2, 1, RunKind::Graph, RunStatus::Queued),
        ];
        assert_eq!(latest_live_graph_run(&tie).map(|r| r.id), Some(id(2)));
        let finished = vec![
            run(1, 1, RunKind::Graph, RunStatus::Done),
            run(2, 2, RunKind::Graph, RunStatus::Failed),
            run(3, 3, RunKind::Graph, RunStatus::Cancelled),
        ];
        assert!(latest_live_graph_run(&finished).is_none());
    }

    /// `failed` is the one step status the two notions disagree about: it is not terminal,
    /// because §4.8 promotes it, and `done` is terminal even though it can be superseded.
    #[test]
    fn a_terminal_step_status_is_done_cancelled_or_superseded() {
        let terminal: Vec<StepStatus> = StepStatus::ALL
            .iter()
            .copied()
            .filter(|status| status.is_terminal())
            .collect();
        assert_eq!(
            terminal,
            vec![
                StepStatus::Done,
                StepStatus::Cancelled,
                StepStatus::Superseded
            ],
            "run_step.status: the terminal set of ANA-2 §4.3, `failed` deliberately absent"
        );
        assert!(
            !StepStatus::Failed.is_terminal(),
            "run_step.status `failed` is promotable, so it is not terminal"
        );
    }

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

    /// Plan D53: a snapshot written before milestone 4 has no `template` on its judge, and it
    /// still decodes, as `None`, which means "the latest `judge` template at judge time".
    /// `GraphSnapshot::V` is not bumped for this field.
    #[test]
    fn a_snapshot_judge_without_a_template_still_decodes() {
        let old: SnapshotJudge = serde_json::from_value(json!({
            "agent_id": "00000000-0000-0000-0000-000000000001",
            "agent_name": "claude",
            "model": null,
        }))
        .expect("a pre-M4 judge decodes");
        assert_eq!(old.template, None, "an absent template decodes as `None`");

        let pinned: SnapshotJudge = serde_json::from_value(json!({
            "agent_id": "00000000-0000-0000-0000-000000000001",
            "agent_name": "claude",
            "model": "opus",
            "template": { "name": "judge", "version": 3 },
        }))
        .expect("a pinned judge decodes");
        assert_eq!(
            pinned.template,
            Some(SnapshotTemplate {
                name: "judge".to_owned(),
                version: 3
            }),
            "a pinned template survives the round trip"
        );
        let again: SnapshotJudge =
            serde_json::from_value(serde_json::to_value(&pinned).expect("encodes"))
                .expect("decodes");
        assert_eq!(again, pinned, "encode then decode is the identity");
    }

    fn snapshot_phase(persona: Option<&str>) -> super::SnapshotPhase {
        use crate::model::kind::{CommandQueue, Gate, Isolation};
        super::SnapshotPhase {
            position: 0,
            name: "review".to_owned(),
            fan_out: 1,
            gate: Gate::Never,
            gate_effective: Gate::Never,
            gate_hard: false,
            retry_limit: 0,
            input_kinds: Vec::new(),
            output_kind: "review".to_owned(),
            isolation: Isolation::Local,
            command_queue: CommandQueue::Off,
            verify_command: None,
            deadline_seconds: None,
            template: SnapshotTemplate {
                name: "review".to_owned(),
                version: 1,
            },
            token_budget: None,
            candidates: Vec::new(),
            judge: None,
            persona: persona.map(str::to_owned),
            role: super::NodeRole::Step,
            palette: false,
            plan_scope: None,
            divide: None,
        }
    }

    fn graph_snapshot(
        phases: Vec<super::SnapshotPhase>,
        personas: Vec<crate::model::persona::SnapshotPersona>,
    ) -> super::GraphSnapshot {
        use crate::model::ids::StepGraphId;
        use crate::model::kind::Isolation;
        super::GraphSnapshot {
            v: super::GraphSnapshot::V,
            graph: super::SnapshotGraph {
                id: StepGraphId::new(),
                name: "feature".to_owned(),
                is_override: false,
                scope: None,
                revision: None,
            },
            source: None,
            topology: "sha256:00".to_owned(),
            mode: super::RunMode::Auto,
            phases,
            edges: Vec::new(),
            settings: super::SnapshotSettings {
                default_isolation: Isolation::Local,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 1,
                max_agents_per_run: 1,
                max_parallel_sessions: super::SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS,
                max_plan_revisions: super::SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS,
                max_child_runs: super::SnapshotSettings::DEFAULT_MAX_CHILD_RUNS,
                runtime_changes: super::RuntimeChanges::Ask,
            },
            scope: None,
            personas,
            memory: None,
        }
    }

    fn frozen(name: &str) -> crate::model::persona::SnapshotPersona {
        use crate::model::persona::{PersonaPermission, PersonaTools, SnapshotPersona};
        SnapshotPersona {
            name: name.to_owned(),
            digest: "sha256:00".to_owned(),
            body: format!("You are the {name}.\n"),
            tools: PersonaTools::default(),
            permission: PersonaPermission::default(),
        }
    }

    #[test]
    fn a_persona_less_phase_serialises_without_a_persona_key() {
        let phase = snapshot_phase(None);
        let text = serde_json::to_string(&phase).expect("encodes");
        assert!(!text.contains("\"persona\""), "{text}");

        let back: super::SnapshotPhase = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back.persona, None);
        assert_eq!(back, phase);
    }

    #[test]
    fn a_bound_phase_carries_its_persona_name() {
        let phase = snapshot_phase(Some("reviewer"));
        let text = serde_json::to_string(&phase).expect("encodes");
        assert!(text.ends_with(",\"persona\":\"reviewer\"}"), "{text}");

        let back: super::SnapshotPhase = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, phase);
    }

    /// MOD-163 D11: a pre-MOD-163 snapshot has no `memory` key and decodes as `None`; a `None`
    /// snapshot writes no key; a frozen memory round-trips and leaves `phases` and `edges` (the
    /// only inputs `topology_v2` hashes) as they were.
    #[test]
    fn a_snapshot_without_memory_decodes_as_none_and_writes_no_key() {
        use crate::model::memory::{MemoryKind, MemorySource, SnapshotMemory, SnapshotMemoryEntry};
        let snapshot = graph_snapshot(vec![snapshot_phase(None)], Vec::new());
        let value = serde_json::to_value(&snapshot).expect("encodes");
        assert!(value.get("memory").is_none(), "{value}");
        let back: super::GraphSnapshot = serde_json::from_value(value).expect("old JSON decodes");
        assert_eq!(back.memory, None);

        let mut with = snapshot.clone();
        with.memory = Some(SnapshotMemory {
            estimator: "chars-v2".to_owned(),
            total: 1,
            entries: vec![SnapshotMemoryEntry {
                id: crate::model::MemoryEntryId::from_uuid(Uuid::from_u128(1)),
                version: 1,
                kind: MemoryKind::Fact,
                subject: "Build cache".to_owned(),
                body: "Clear target/.".to_owned(),
                source: MemorySource::Person,
            }],
        });
        let value = serde_json::to_value(&with).expect("encodes");
        assert_eq!(value["memory"]["total"], 1, "{value}");
        let back: super::GraphSnapshot = serde_json::from_value(value).expect("decodes");
        assert_eq!(back, with);
        assert_eq!(
            (&back.phases, &back.edges),
            (&snapshot.phases, &snapshot.edges)
        );
    }

    #[test]
    fn a_snapshot_without_personas_omits_the_key() {
        let snapshot = graph_snapshot(vec![snapshot_phase(None)], Vec::new());
        let text = serde_json::to_string(&snapshot).expect("encodes");
        assert!(!text.contains("\"personas\""), "{text}");
        let back: super::GraphSnapshot = serde_json::from_str(&text).expect("old JSON decodes");
        assert!(back.personas.is_empty());
        assert_eq!(back, snapshot);

        let bound = graph_snapshot(
            vec![snapshot_phase(Some("reviewer"))],
            vec![frozen("reviewer")],
        );
        let text = serde_json::to_string(&bound).expect("encodes");
        assert!(
            text.contains("\"personas\":[{\"name\":\"reviewer\""),
            "{text}"
        );
        let back: super::GraphSnapshot = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, bound);
    }

    #[test]
    fn persona_for_finds_refuses_and_skips() {
        use crate::model::persona::PersonaNotInSnapshot;

        let carried = graph_snapshot(
            vec![snapshot_phase(None), snapshot_phase(Some("reviewer"))],
            vec![frozen("architect"), frozen("reviewer")],
        );
        assert_eq!(carried.persona_for(&carried.phases[0]), Ok(None));
        assert_eq!(
            carried.persona_for(&carried.phases[1]),
            Ok(Some(&carried.personas[1]))
        );

        let missing = graph_snapshot(
            vec![snapshot_phase(Some("reviewer"))],
            vec![frozen("architect")],
        );
        assert_eq!(
            missing.persona_for(&missing.phases[0]),
            Err(PersonaNotInSnapshot {
                persona: "reviewer".into()
            })
        );
    }

    /// MOD-72 plan D3: step first, then the kind by bytes, so `"Zed"` sorts before `"edit"`,
    /// which a collation would reverse.
    #[test]
    fn tool_call_counts_sort_by_step_then_kind_bytes() {
        let s1 = StepId::from_uuid(Uuid::from_u128(1));
        let s2 = StepId::from_uuid(Uuid::from_u128(2));
        let row = |step: StepId, tool_kind: &str| ToolCallCount {
            step,
            tool_kind: tool_kind.to_owned(),
            calls: 1,
        };
        let mut rows = vec![
            row(s2, "read"),
            row(s1, "read"),
            row(s1, "edit"),
            row(s1, "Zed"),
        ];

        ToolCallCount::sort_canonical(&mut rows);

        assert_eq!(
            rows,
            vec![
                row(s1, "Zed"),
                row(s1, "edit"),
                row(s1, "read"),
                row(s2, "read"),
            ]
        );
    }

    /// MOD-55 plan P5, amendment A-3: a spec written before `phase_name` existed reads back as a
    /// chat, not as `""` (which a bare `#[serde(default)]` on a `String` would give), and
    /// [`ChatRunSpec::for_edit_help`](super::ChatRunSpec::for_edit_help) changes nothing but the
    /// phase.
    #[test]
    fn a_spec_without_phase_name_deserialises_as_chat() {
        use super::{CHAT_PHASE, ChatRunSpec, EDIT_HELP_PHASE};
        use crate::fixtures::ids;

        let chat = ChatRunSpec::mint(
            ids::PROJECT_HTUI,
            ids::BOX,
            ids::USER,
            Some(ids::AGENT_CLAUDE),
            Some("sonnet".to_owned()),
        );
        assert_eq!(chat.phase_name, CHAT_PHASE, "mint marks a chat");

        let mut value = serde_json::to_value(&chat).expect("encodes");
        let removed = value
            .as_object_mut()
            .expect("a spec is an object")
            .remove("phase_name");
        assert_eq!(removed, Some(json!("chat")), "the key is written");
        let back: ChatRunSpec = serde_json::from_value(value).expect("old JSON decodes");
        assert_eq!(
            back.phase_name, "chat",
            "the serde default is the chat phase"
        );
        assert_eq!(back, chat);

        let help = chat.clone().for_edit_help();
        assert_eq!(help.phase_name, EDIT_HELP_PHASE);
        assert_eq!(
            ChatRunSpec {
                phase_name: CHAT_PHASE.to_owned(),
                ..help
            },
            chat,
            "for_edit_help keeps every other field"
        );
    }

    // ---- MOD-138 T1: snapshot v2 (D1, D2, D4) ----

    /// `snapshot_phase(None)` at `name`, `position` and `retry_limit`.
    fn phase_at(name: &str, position: i32, retry_limit: i32) -> super::SnapshotPhase {
        super::SnapshotPhase {
            name: name.to_owned(),
            position,
            retry_limit,
            ..snapshot_phase(None)
        }
    }

    fn edge(
        from: &str,
        to: &str,
        kind: crate::model::kind::EdgeKind,
        loop_limit: Option<i32>,
    ) -> super::SnapshotEdge {
        super::SnapshotEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            predicate: None,
            loop_limit,
        }
    }

    fn divide_block() -> super::SnapshotDivide {
        use crate::model::ids::{AgentId, PersonaId};
        super::SnapshotDivide {
            planner: super::SnapshotSeat {
                agent_id: Some(AgentId::from_uuid(Uuid::from_u128(7))),
                model: Some("opus".to_owned()),
                persona_id: None,
            },
            integrate: super::SnapshotSeat {
                agent_id: None,
                model: None,
                persona_id: Some(PersonaId::from_uuid(Uuid::from_u128(8))),
            },
            max_parts: Some(4),
            default_parts: vec![super::SnapshotPart {
                position: 0,
                key: "api".to_owned(),
                title: "The API".to_owned(),
                brief: "Build the API".to_owned(),
                owned_paths: vec!["src/api".to_owned()],
                acceptance: Some("its tests pass".to_owned()),
                agent_id: None,
                model: None,
                persona_id: None,
            }],
        }
    }

    /// D1 (plan risk 1): a linear phase carries none of the v2 keys, so it serialises exactly as
    /// v1 did and its v1 digest does not move. `split` is derived and never written (A1).
    #[test]
    fn a_step_phase_serialises_without_the_v2_keys() {
        let text = serde_json::to_string(&snapshot_phase(None)).expect("encodes");
        for key in [
            "\"role\"",
            "\"palette\"",
            "\"plan_scope\"",
            "\"divide\"",
            "\"split\"",
        ] {
            assert!(!text.contains(key), "{key} in {text}");
        }
        assert!(text.ends_with("\"judge\":null}"), "{text}");
    }

    #[test]
    fn a_planner_phase_carries_its_role_and_divide() {
        use crate::model::kind::PlanScope;
        let phase = super::SnapshotPhase {
            role: super::NodeRole::Planner,
            palette: true,
            plan_scope: Some(PlanScope::Parts),
            divide: Some(divide_block()),
            ..snapshot_phase(Some("planner"))
        };
        let value = serde_json::to_value(&phase).expect("encodes");
        assert_eq!(value["role"], json!("planner"));
        assert_eq!(value["palette"], json!(true));
        assert_eq!(value["plan_scope"], json!("parts"));
        assert_eq!(value["divide"]["max_parts"], json!(4));
        assert_eq!(value["divide"]["planner"]["model"], json!("opus"));
        assert_eq!(value["divide"]["default_parts"][0]["key"], json!("api"));
        assert!(value.get("split").is_none(), "split is never written (A1)");

        let back: super::SnapshotPhase = serde_json::from_value(value).expect("decodes");
        assert_eq!(back, phase, "encode then decode is the identity");
    }

    /// A1 (F1): `split()` is `divide` exactly when a divide block is frozen, else what `fan_out`
    /// implies (`ck_phase_split_fan_out`).
    #[test]
    fn split_is_divide_then_fan_out() {
        use crate::model::kind::Split;
        let divide = super::SnapshotPhase {
            divide: Some(divide_block()),
            ..snapshot_phase(None)
        };
        assert_eq!(divide.split(), Split::Divide);
        let compete = super::SnapshotPhase {
            fan_out: 2,
            ..snapshot_phase(None)
        };
        assert_eq!(compete.split(), Split::Compete);
        assert_eq!(snapshot_phase(None).split(), Split::None);
    }

    #[test]
    fn a_v2_snapshot_round_trips_edges_source_scope_revision_and_bounds() {
        use crate::model::kind::{EdgeKind, EdgePredicate, GraphScope};
        let mut snapshot = graph_snapshot(
            vec![phase_at("plan", 0, 1), phase_at("review", 1, 2)],
            Vec::new(),
        );
        assert_eq!(snapshot.v, 2, "the version this crate writes (D1)");
        snapshot.graph.scope = Some(GraphScope::Library);
        snapshot.graph.revision = Some(7);
        snapshot.source = Some(super::SnapshotSource {
            how: super::SourceHow::KindDefault,
        });
        snapshot.edges = vec![
            super::SnapshotEdge {
                predicate: Some(EdgePredicate(json!({ "verdict": "approve" }))),
                ..edge("plan", "review", EdgeKind::If, None)
            },
            edge("review", "plan", EdgeKind::ChangesRequested, Some(2)),
        ];
        snapshot.settings.max_parallel_sessions = 2;
        snapshot.settings.max_plan_revisions = 5;
        snapshot.settings.max_child_runs = 1;
        snapshot.settings.runtime_changes = super::RuntimeChanges::Off;

        let text = serde_json::to_string(&snapshot).expect("encodes");
        assert!(text.contains("\"edges\":[{\"from\":\"plan\""), "{text}");
        assert!(
            text.contains("\"source\":{\"how\":\"kind_default\"}"),
            "{text}"
        );
        assert!(
            text.contains("\"scope\":{\"owner\":\"library\"},\"revision\":7"),
            "{text}"
        );
        assert!(text.contains("\"max_parallel_sessions\":2"), "{text}");
        assert!(text.contains("\"runtime_changes\":\"off\""), "{text}");

        let back: super::GraphSnapshot = serde_json::from_str(&text).expect("decodes");
        assert_eq!(back, snapshot, "encode then decode is the identity");
    }

    /// D1: a blob written before MOD-138 (no `edges`, `source`, bounds or graph scope) decodes,
    /// every new field at its default.
    #[test]
    fn a_v1_blob_decodes_with_v2_defaults() {
        let v1 = json!({
            "v": 1,
            "graph": {
                "id": "01a06490-c789-7000-8000-000000000501",
                "name": "feature",
                "is_override": false,
            },
            "topology": "sha256:00",
            "mode": "auto",
            "phases": [serde_json::to_value(snapshot_phase(None)).expect("encodes")],
            "settings": {
                "default_isolation": "local",
                "per_token_cap_run": null,
                "per_token_cap_batch": null,
                "max_fan_out": 1,
                "max_agents_per_run": 1,
            },
            "scope": null,
        });
        let back: super::GraphSnapshot = serde_json::from_value(v1).expect("a v1 blob decodes");
        assert_eq!(back.v, super::GraphSnapshot::V1);
        assert!(back.edges.is_empty(), "the lift fills them, not serde");
        assert_eq!(back.source, None);
        assert_eq!((back.graph.scope, back.graph.revision), (None, None));
        assert_eq!(
            (
                back.settings.max_parallel_sessions,
                back.settings.max_plan_revisions,
                back.settings.max_child_runs,
                back.settings.runtime_changes,
            ),
            (4, 3, 8, super::RuntimeChanges::Ask),
            "the code defaults (D13)"
        );
        assert_eq!(
            (
                super::SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS,
                super::SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS,
                super::SnapshotSettings::DEFAULT_MAX_CHILD_RUNS,
            ),
            (4, 3, 8)
        );
        let phase = &back.phases[0];
        assert_eq!(phase.role, super::NodeRole::Step);
        assert!(!phase.palette);
        assert_eq!((phase.plan_scope, phase.divide.as_ref()), (None, None));
        assert_eq!(phase, &snapshot_phase(None));
    }

    /// F5: a predicate's bytes never depend on `serde_json`'s `preserve_order` (finding 4): its
    /// object keys sort by bytes at every depth, arrays included.
    #[test]
    fn a_predicate_serialises_with_sorted_keys() {
        use crate::model::kind::{EdgeKind, EdgePredicate};
        use serde_json::{Map, Value};
        let mut inner = Map::new();
        inner.insert("y".to_owned(), json!(2));
        inner.insert("b".to_owned(), json!(3));
        let mut listed = Map::new();
        listed.insert("q".to_owned(), json!(1));
        listed.insert("c".to_owned(), json!(null));
        let mut outer = Map::new();
        outer.insert("z".to_owned(), Value::Array(vec![Value::Object(listed)]));
        outer.insert("a".to_owned(), Value::Object(inner));
        let edge = super::SnapshotEdge {
            predicate: Some(EdgePredicate(Value::Object(outer))),
            ..edge("a", "b", EdgeKind::If, None)
        };
        assert_eq!(
            serde_json::to_string(&edge).expect("encodes"),
            r#"{"from":"a","to":"b","kind":"if","predicate":{"a":{"b":3,"y":2},"z":[{"c":null,"q":1}]}}"#
        );
        let bare = super::SnapshotEdge {
            predicate: None,
            ..edge.clone()
        };
        assert_eq!(
            serde_json::to_string(&bare).expect("encodes"),
            r#"{"from":"a","to":"b","kind":"if"}"#,
            "no predicate, no loop limit: neither key is written"
        );
        let back: super::SnapshotEdge =
            serde_json::from_str(&serde_json::to_string(&edge).expect("encodes")).expect("decodes");
        assert_eq!(back, edge);
    }

    /// F13: one edge order, (position of `from`, position of `to`, kind bytes), an unknown key
    /// last, ties by key bytes.
    #[test]
    fn edges_sort_by_position_then_kind() {
        use crate::model::kind::EdgeKind;
        let phases = vec![
            phase_at("implement", 1, 1),
            phase_at("plan", 0, 1),
            phase_at("review", 2, 1),
        ];
        let mut edges = vec![
            edge("review", "implement", EdgeKind::ChangesRequested, Some(1)),
            edge("ghost", "plan", EdgeKind::Next, None),
            edge("implement", "review", EdgeKind::Next, None),
            edge("plan", "implement", EdgeKind::Next, None),
            edge("plan", "review", EdgeKind::If, None),
            edge("plan", "implement", EdgeKind::AsksYou, None),
            edge("plan", "ghost", EdgeKind::Next, None),
        ];
        super::sort_snapshot_edges(&mut edges, &phases);
        let order: Vec<(&str, &str, &str)> = edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str(), edge.kind.as_str()))
            .collect();
        assert_eq!(
            order,
            [
                ("plan", "implement", "asks_you"),
                ("plan", "implement", "next"),
                ("plan", "review", "if"),
                ("plan", "ghost", "next"),
                ("implement", "review", "next"),
                ("review", "implement", "changes_requested"),
                ("ghost", "plan", "next"),
            ]
        );
    }

    /// D4's "one rule, pinned equal": over every seeded graph and the awkward shapes (gaps, a
    /// leading `review`, two `review`s, slice order unlike position order), the key-based edges
    /// are exactly `seed::default_edges` mapped to names.
    #[test]
    fn default_snapshot_edges_is_seed_default_edges() {
        use crate::model::ids::{EdgeId, PhaseId, StepGraphId};
        use crate::seed::{KINDS, PhaseSeed, default_edges, phase_row};

        type Shape = (String, String, &'static str, Option<i32>);
        let mut shapes: Vec<Vec<(&'static str, i32, i32)>> = KINDS
            .iter()
            .map(|kind| {
                kind.phases
                    .iter()
                    .zip(0..)
                    .map(|(phase, position)| (phase.name, position, 1))
                    .collect()
            })
            .collect();
        shapes.push(vec![("implement", 0, 3), ("docs", 5, 1), ("review", 9, 2)]);
        shapes.push(vec![("review", 0, 1), ("implement", 1, 2), ("check", 2, 1)]);
        shapes.push(vec![
            ("plan", 0, 2),
            ("review", 1, 1),
            ("implement", 2, 4),
            ("review", 3, 1),
        ]);
        shapes.push(vec![("draft", 4, 2), ("review", 2, 1), ("spec", 0, 5)]);
        for rows in shapes {
            let graph = StepGraphId::new();
            let stored: Vec<_> = rows
                .iter()
                .map(|&(name, position, retry_limit)| {
                    let seed = PhaseSeed {
                        name,
                        input_kinds: &[],
                        gate_hard: false,
                    };
                    let mut row =
                        phase_row(PhaseId::new(), graph, position, &seed, chrono::Utc::now());
                    row.retry_limit = retry_limit;
                    row
                })
                .collect();
            let name_of = |id| {
                stored
                    .iter()
                    .find(|phase| phase.id == id)
                    .map(|phase| phase.name.clone())
                    .expect("an endpoint of the graph")
            };
            let mut seeded: Vec<Shape> = default_edges(&stored, EdgeId::new)
                .iter()
                .map(|e| {
                    (
                        name_of(e.from),
                        name_of(e.to),
                        e.kind.as_str(),
                        e.loop_limit,
                    )
                })
                .collect();
            let phases: Vec<_> = rows
                .iter()
                .map(|&(name, position, retry_limit)| phase_at(name, position, retry_limit))
                .collect();
            let keyed = super::default_snapshot_edges(&phases);
            assert!(keyed.iter().all(|e| e.predicate.is_none()));
            let mut keyed: Vec<Shape> = keyed
                .iter()
                .map(|e| (e.from.clone(), e.to.clone(), e.kind.as_str(), e.loop_limit))
                .collect();
            seeded.sort();
            keyed.sort();
            assert_eq!(keyed, seeded, "{rows:?}");
        }
    }

    /// D4: the lift fills a v1 snapshot's edges and keeps `v == 1`; a v2 snapshot is untouched,
    /// edges or none.
    #[test]
    fn lift_v1_fills_default_edges_and_keeps_v1() {
        use crate::model::kind::EdgeKind;
        let phases = vec![
            phase_at("prd", 0, 1),
            phase_at("plan", 1, 1),
            phase_at("implement", 2, 3),
            phase_at("review", 3, 1),
        ];
        let mut v1 = super::GraphSnapshot {
            v: super::GraphSnapshot::V1,
            ..graph_snapshot(phases.clone(), Vec::new())
        };
        v1.lift_v1();
        assert_eq!(v1.v, 1, "resume tells a v1 run from a v2 one by `v`");
        assert_eq!(
            v1.edges,
            [
                edge("prd", "plan", EdgeKind::Next, None),
                edge("plan", "implement", EdgeKind::Next, None),
                edge("implement", "review", EdgeKind::Next, None),
                edge("review", "implement", EdgeKind::ChangesRequested, Some(3)),
            ]
        );
        let lifted = v1.clone();
        v1.lift_v1();
        assert_eq!(v1, lifted, "lifting twice changes nothing");

        let mut v2 = graph_snapshot(phases, Vec::new());
        let before = v2.clone();
        v2.lift_v1();
        assert_eq!(
            v2, before,
            "a v2 snapshot keeps the edges it froze, none included"
        );
    }

    // ---- MOD-138 M2 T1: walk_edges, reads_verdict (D1, D7) ----

    fn feature_phases() -> Vec<super::SnapshotPhase> {
        vec![
            phase_at("prd", 0, 1),
            phase_at("plan", 1, 1),
            phase_at("implement", 2, 3),
            phase_at("review", 3, 1),
        ]
    }

    /// D1: a raw v1 blob with no edges (as serde decodes one) walks its lift, computed on the
    /// spot; a lifted v1 and a v2 lend their own edges.
    #[test]
    fn walk_edges_lifts_a_raw_v1_blob_and_borrows_a_v2() {
        use std::borrow::Cow;
        let raw = super::GraphSnapshot {
            v: super::GraphSnapshot::V1,
            ..graph_snapshot(feature_phases(), Vec::new())
        };
        let mut lifted = raw.clone();
        lifted.lift_v1();
        let walked = raw.walk_edges();
        assert!(matches!(walked, Cow::Owned(_)), "the lift is computed");
        assert_eq!(&*walked, lifted.edges.as_slice(), "and equals lift_v1's");
        assert!(raw.edges.is_empty(), "walk_edges never writes the blob");
        assert!(matches!(lifted.walk_edges(), Cow::Borrowed(_)));

        let mut v2 = graph_snapshot(feature_phases(), Vec::new());
        v2.edges = vec![edge(
            "prd",
            "plan",
            crate::model::kind::EdgeKind::Next,
            None,
        )];
        let walked = v2.walk_edges();
        assert!(matches!(walked, Cow::Borrowed(_)));
        assert_eq!(&*walked, v2.edges.as_slice());
    }

    /// D1: a v2 snapshot with no edges is not lifted: its nodes are roots.
    #[test]
    fn a_v2_snapshot_with_no_edges_is_not_lifted() {
        let v2 = graph_snapshot(feature_phases(), Vec::new());
        assert_eq!(v2.v, super::GraphSnapshot::V);
        assert!(v2.walk_edges().is_empty());
        assert!(!v2.reads_verdict("review"));
    }

    /// D7: a node reads its verdict exactly when it has an outgoing `changes_requested` edge,
    /// whatever its name; a raw v1 blob answers through its lift.
    #[test]
    fn reads_verdict_is_having_a_changes_requested_edge() {
        use crate::model::kind::EdgeKind;
        let raw = super::GraphSnapshot {
            v: super::GraphSnapshot::V1,
            ..graph_snapshot(feature_phases(), Vec::new())
        };
        assert!(raw.reads_verdict("review"), "the lift's review loop");
        for node in ["prd", "plan", "implement", "absent"] {
            assert!(!raw.reads_verdict(node), "{node}");
        }

        let mut v2 = graph_snapshot(
            vec![
                phase_at("implement", 0, 1),
                phase_at("check", 1, 1),
                phase_at("review", 2, 1),
            ],
            Vec::new(),
        );
        v2.edges = vec![
            edge("implement", "check", EdgeKind::Next, None),
            edge("check", "review", EdgeKind::Next, None),
            edge("check", "implement", EdgeKind::ChangesRequested, Some(2)),
            edge("review", "implement", EdgeKind::Fails, Some(2)),
        ];
        assert!(v2.reads_verdict("check"), "not named review, still reads");
        assert!(
            !v2.reads_verdict("review"),
            "named review, a `fails` edge only: no verdict"
        );
        assert!(!v2.reads_verdict("implement"));
    }

    /// Q-D: a `review` first in its graph gets no loop edge from the lift, so reads no verdict.
    #[test]
    fn a_review_first_in_its_graph_reads_no_verdict() {
        let raw = super::GraphSnapshot {
            v: super::GraphSnapshot::V1,
            ..graph_snapshot(
                vec![phase_at("review", 0, 1), phase_at("implement", 1, 2)],
                Vec::new(),
            )
        };
        assert!(!raw.reads_verdict("review"));
    }

    /// D5: a step encoded before `run_step.branch` existed decodes with none; a decided one
    /// round-trips.
    #[test]
    fn run_step_branch_defaults_when_absent() {
        let mut step = crate::fixtures::demo_data()
            .steps
            .into_iter()
            .next()
            .expect("the fixture holds steps");
        step.branch = Some(json!({"deploy": true, "docs": false}));
        let mut value = serde_json::to_value(&step).expect("encodes");
        assert_eq!(value["branch"], json!({"deploy": true, "docs": false}));
        let back: super::RunStep = serde_json::from_value(value.clone()).expect("decodes");
        assert_eq!(back, step);

        value
            .as_object_mut()
            .expect("a step is an object")
            .remove("branch");
        let old: super::RunStep = serde_json::from_value(value).expect("old JSON decodes");
        assert_eq!(old.branch, None);
        assert_eq!(
            super::RunStep {
                branch: step.branch.clone(),
                ..old
            },
            step
        );
    }

    /// MOD-139 D3: a step encoded before `run_step.role`, `parent_step_id` and `node_key` existed
    /// decodes as an ordinary step with neither; a part round-trips.
    #[test]
    fn run_step_role_defaults_when_absent() {
        let mut step = crate::fixtures::demo_data()
            .steps
            .into_iter()
            .next()
            .expect("the fixture holds steps");
        step.role = super::StepRole::Part;
        step.parent_step_id = Some(StepId::new());
        step.node_key = Some("implement/api".to_owned());
        let mut value = serde_json::to_value(&step).expect("encodes");
        assert_eq!(value["role"], json!("part"));
        assert_eq!(value["node_key"], json!("implement/api"));
        let back: super::RunStep = serde_json::from_value(value.clone()).expect("decodes");
        assert_eq!(back, step);

        let object = value.as_object_mut().expect("a step is an object");
        for key in ["role", "parent_step_id", "node_key"] {
            object.remove(key);
        }
        let old: super::RunStep = serde_json::from_value(value).expect("old JSON decodes");
        assert_eq!(old.role, super::StepRole::Step);
        assert_eq!(old.parent_step_id, None);
        assert_eq!(old.node_key, None);
        assert_eq!(
            super::RunStep {
                role: step.role,
                parent_step_id: step.parent_step_id,
                node_key: step.node_key.clone(),
                ..old
            },
            step
        );
    }
}
