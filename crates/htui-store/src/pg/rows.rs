//! Row structs for the three Postgres reads that cannot decode straight into a model type
//! (blueprint C.6).
//!
//! Everything else - [`Item`](htui_core::model::Item),
//! [`ItemSummary`](htui_core::model::ItemSummary),
//! [`DocumentHead`](htui_core::model::DocumentHead), [`Note`](htui_core::model::Note),
//! [`SessionEvent`](htui_core::model::SessionEvent),
//! [`ItemRevision`](htui_core::model::ItemRevision),
//! [`ProjectRef`](htui_core::model::ProjectRef), [`BoxInfo`](htui_core::model::BoxInfo) - is
//! produced by `query_as!` with column overrides directly into the model type, which is why this
//! module is this short.
//!
//! The three that remain are the ones whose result shape is not a table row:
//!
//! - [`RunRow`] carries the joined `box.hostname` and leaves `steps` to a second statement, because
//!   [`RunSummary`](htui_core::model::RunSummary) is nested and a run with no steps must still
//!   appear.
//! - [`StepRow`] carries `run_id` so the caller can group the second statement's rows.
//! - [`LinkNodeRow`] carries the recursive CTE's `depth` as the `INTEGER` Postgres produces;
//!   [`LinkNode`](htui_core::model::LinkNode) declares it as a `u8`.
//!
//! Field order is load-bearing: `query_as!` binds result columns to fields positionally, so the
//! `SELECT` list in `read.rs` is written in the order declared here.

use chrono::{DateTime, Utc};
use htui_core::model::{
    AgentId, BoxId, GateOutcome, ItemId, LinkNode, ProjectId, RunId, RunKind, RunMode, RunStatus,
    RunStepSummary, RunSummary, Status, StepId, StepStatus,
};

/// One `run` row with the joined `box.hostname`: every [`RunSummary`] field except `steps`.
#[derive(Debug, Clone)]
pub(crate) struct RunRow {
    /// `run.id`.
    pub(crate) id: RunId,
    /// `run.item_id`; `None` for a free-standing chat.
    pub(crate) item_id: Option<ItemId>,
    /// `run.project_id`.
    pub(crate) project_id: ProjectId,
    /// `run.kind`.
    pub(crate) kind: RunKind,
    /// `run.mode`.
    pub(crate) mode: RunMode,
    /// `run.status`.
    pub(crate) status: RunStatus,
    /// `run.target_box_id`.
    pub(crate) target_box_id: BoxId,
    /// `run.executing_box_id`.
    pub(crate) executing_box_id: Option<BoxId>,
    /// `box.hostname` of the executing box, else of the target box; `''` when neither has a row.
    pub(crate) box_hostname: String,
    /// `run.queued_at`.
    pub(crate) queued_at: DateTime<Utc>,
    /// `run.started_at`.
    pub(crate) started_at: Option<DateTime<Utc>>,
    /// `run.finished_at`.
    pub(crate) finished_at: Option<DateTime<Utc>>,
    /// `run.failure`.
    pub(crate) failure: Option<String>,
}

impl RunRow {
    /// This run with its steps attached, in the order the second statement returned them.
    pub(crate) fn into_summary(self, steps: Vec<RunStepSummary>) -> RunSummary {
        RunSummary {
            id: self.id,
            item_id: self.item_id,
            project_id: self.project_id,
            kind: self.kind,
            mode: self.mode,
            status: self.status,
            target_box_id: self.target_box_id,
            executing_box_id: self.executing_box_id,
            box_hostname: self.box_hostname,
            queued_at: self.queued_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            failure: self.failure,
            steps,
        }
    }
}

/// One `run_step` projected for [`RunSummary::steps`], plus the `run_id` it is grouped by.
#[derive(Debug, Clone)]
pub(crate) struct StepRow {
    /// `run_step.run_id`: the grouping key, not a [`RunStepSummary`] field.
    pub(crate) run_id: RunId,
    /// `run_step.id`.
    pub(crate) id: StepId,
    /// `run_step.position`.
    pub(crate) position: i32,
    /// `run_step.attempt`.
    pub(crate) attempt: i32,
    /// `run_step.fanout_index`.
    pub(crate) fanout_index: i32,
    /// `run_step.phase_name`.
    pub(crate) phase_name: String,
    /// `run_step.agent_id`.
    pub(crate) agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub(crate) model: Option<String>,
    /// `run_step.status`.
    pub(crate) status: StepStatus,
    /// `run_step.gate_outcome`.
    pub(crate) gate_outcome: Option<GateOutcome>,
    /// `run_step.started_at`.
    pub(crate) started_at: Option<DateTime<Utc>>,
    /// `run_step.finished_at`.
    pub(crate) finished_at: Option<DateTime<Utc>>,
    /// `run_step.trim_record->>'estimated_after'`, the statement's own projection (plan D106).
    ///
    /// Appended, never inserted: `query_as!` binds a struct's fields **positionally**, so a new
    /// field in the middle would silently re-map every column after it.
    pub(crate) prompt_tokens: Option<i32>,
    /// Whether any `run_step.trim_record->'sections'` entry has `trimmed: true` (plan D106).
    pub(crate) trimmed: bool,
}

impl StepRow {
    /// This row as the summary the Runs sub-tab lists, dropping the grouping key.
    pub(crate) fn into_summary(self) -> RunStepSummary {
        RunStepSummary {
            id: self.id,
            position: self.position,
            attempt: self.attempt,
            fanout_index: self.fanout_index,
            phase_name: self.phase_name,
            agent_id: self.agent_id,
            model: self.model,
            status: self.status,
            gate_outcome: self.gate_outcome,
            started_at: self.started_at,
            finished_at: self.finished_at,
            prompt_tokens: self.prompt_tokens,
            trimmed: self.trimmed,
        }
    }
}

/// One node of the `links` traversal, with `project.slug` joined and the CTE's `INTEGER` depth.
#[derive(Debug, Clone)]
pub(crate) struct LinkNodeRow {
    /// `item.id`.
    pub(crate) item_id: ItemId,
    /// `item.project_id`.
    pub(crate) project_id: ProjectId,
    /// `project.slug`, so the graph view can label a cross-project node.
    pub(crate) project_slug: String,
    /// `item.key`.
    pub(crate) key: String,
    /// `item.title`.
    pub(crate) title: String,
    /// `item.status`.
    pub(crate) status: Status,
    /// Hops from the root, as `MIN(depth)` over the recursive walk.
    pub(crate) depth: i32,
}

impl LinkNodeRow {
    /// This row as a [`LinkNode`], narrowing `depth` to the `u8` the caller asked for.
    ///
    /// The walk never goes past the `hops: u8` bound, so the cast cannot lose information; it is
    /// saturating rather than a `try_into().expect(...)` so a future widening of `hops` degrades
    /// into a clamped depth instead of a panic in a read path.
    pub(crate) fn into_node(self) -> LinkNode {
        LinkNode {
            item_id: self.item_id,
            project_id: self.project_id,
            project_slug: self.project_slug,
            key: self.key,
            title: self.title,
            status: self.status,
            depth: u8::try_from(self.depth).unwrap_or(u8::MAX),
        }
    }
}
