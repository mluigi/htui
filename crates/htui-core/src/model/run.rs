//! Runs, steps and their commits (`docs/ANA-9.md` §5.8).

use chrono::{DateTime, SubsecRound as _, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{
    AgentId, BoxId, CommandRunId, ItemId, ProjectId, RepoId, RunId, StepGraphId, StepId, UserId,
};
use crate::model::kind::{CommandQueue, Gate, Isolation};

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

    /// ANA-2 §4.3's `run` table (`docs/ANA-2.md:600-616`): whether `self -> to` is a sanctioned
    /// move. Every terminal status answers `false` for every `to`.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Queued => matches!(to, Self::Running | Self::Cancelled | Self::Failed),
            Self::Running => matches!(
                to,
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Cancelled
            ),
            Self::AwaitingApproval => matches!(to, Self::Running | Self::Failed | Self::Cancelled),
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

impl StepStatus {
    /// ANA-2 §4.3's `run_step` table (`docs/ANA-2.md:630-654`). `failed` is **not** terminal for
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
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Cancelled
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
    /// `run_step.opening` (MOD-37 M5, R-48): how a promoted step's chat opened. `NULL` on a step
    /// never bound to a promoted chat.
    StepOpening {
        /// The step's own agent session was restored.
        Resumed => "resumed",
        /// A fresh session opened with the handoff prompt.
        Handoff => "handoff",
        /// A requested resume failed; the chat opened with the handoff prompt instead.
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

/// The two rows a free-standing chat needs before its first event can be recorded: one `run`
/// (`kind = 'chat'`, `item_id NULL`) and one `run_step` (`phase_name = 'chat'`, position 0)
/// (MOD-2 plan D4).
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

/// Arguments of [`crate::store::WriteStore::create_run`]: a `kind = 'graph'` run inserted at
/// `queued`, with the item moved to `queued` in the same transaction (ANA-2 §4.3, §5.1).
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
}

/// Arguments of [`crate::store::WriteStore::create_step`]: a `run_step` inserted at `pending`
/// with every settle column `NULL`. `UNIQUE (run_id, position, attempt, fanout_index)` is the
/// database's; a repeat is `Constraint`.
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
    /// `sha256:…` over the canonical `phases[]`; milestone 2 computes it, milestone 1 stores it.
    pub topology: String,
    /// `run.mode`, repeated so the snapshot is self-contained.
    pub mode: RunMode,
    /// The phases in `position` order.
    pub phases: Vec<SnapshotPhase>,
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
}

impl GraphSnapshot {
    /// The `v` this crate writes and the only one it reads.
    pub const V: u32 = 1;

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

    /// Every row of ANA-2 §4.3's `run` transition table (`docs/ANA-2.md:600-616`), transcribed
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
        // `done`, `failed`, `cancelled` are terminal.
    ];

    /// Every row of ANA-2 §4.3's `run_step` transition table (`docs/ANA-2.md:630-654`),
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
        // (derived from row 652 + row 654's exception: ANA-2 §4.3's step table sanctions
        // `failed -> awaiting_approval` only through §4.8's promotion, blueprint C-4).
        (StepStatus::Failed, StepStatus::AwaitingApproval),
        (StepStatus::Failed, StepStatus::Cancelled),
        // `done`
        (StepStatus::Done, StepStatus::Superseded),
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
            },
            topology: "sha256:00".to_owned(),
            mode: super::RunMode::Auto,
            phases,
            settings: super::SnapshotSettings {
                default_isolation: Isolation::Local,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 1,
                max_agents_per_run: 1,
            },
            scope: None,
            personas,
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
}
