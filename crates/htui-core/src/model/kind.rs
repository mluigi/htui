//! Item kinds, step graphs and prompt templates (`docs/ANA-9.md` §5.4, §5.5).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{
    AgentId, ItemKindId, PhaseId, ProjectId, PromptTemplateId, StepGraphId, UserId,
};

str_enum!(
    /// `step_graph_phase.gate` (§5.4): when the phase stops for a human.
    Gate {
        /// Always gate.
        Always => "always",
        /// Gate only when the phase failed.
        OnFailure => "on_failure",
        /// Never gate.
        Never => "never",
    }
);

str_enum!(
    /// `step_graph_phase.isolation` (§5.4): how a step gets its own view of the repository.
    Isolation {
        /// A git worktree per step.
        Worktree => "worktree",
        /// A copy of the working tree.
        Copy => "copy",
        /// The shared working tree, one step at a time.
        SharedSerialized => "shared_serialized",
        /// The working tree as it is, no isolation.
        Local => "local",
    }
);

str_enum!(
    /// `step_graph_phase.command_queue` (§5.4): when commands from this phase are queued
    /// (`R-MCP-3`).
    CommandQueue {
        /// Never queue.
        Off => "off",
        /// Queue only while the phase is fanned out.
        FanOutOnly => "fan_out_only",
        /// Always queue.
        Always => "always",
    }
);

/// A row of `item_kind` (§5.5): a per-project kind with its key prefix and default graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemKind {
    /// `item_kind.id`.
    pub id: ItemKindId,
    /// `item_kind.project_id`.
    pub project_id: ProjectId,
    /// `item_kind.prefix`, e.g. `MOD`; copied onto an item at mint time (§4.1).
    pub prefix: String,
    /// `item_kind.name`.
    pub name: String,
    /// `item_kind.description`.
    pub description: String,
    /// `item_kind.default_graph_id`.
    pub default_graph_id: StepGraphId,
    /// `item_kind.position`.
    pub position: i32,
    /// `item_kind.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `step_graph` (§5.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepGraph {
    /// `step_graph.id`.
    pub id: StepGraphId,
    /// `step_graph.project_id`.
    pub project_id: ProjectId,
    /// `step_graph.name`, unique within the project.
    pub name: String,
    /// `step_graph.description`.
    pub description: String,
    /// `step_graph.created_at`.
    pub created_at: DateTime<Utc>,
    /// `step_graph.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `step_graph_phase` (§5.4): one phase of a graph (`R-ORCH-1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepGraphPhase {
    /// `step_graph_phase.id`.
    pub id: PhaseId,
    /// `step_graph_phase.graph_id`.
    pub graph_id: StepGraphId,
    /// `step_graph_phase.position`.
    pub position: i32,
    /// `step_graph_phase.name`, e.g. `plan` or `implement`.
    pub name: String,
    /// `step_graph_phase.fan_out`.
    pub fan_out: i32,
    /// `step_graph_phase.gate`.
    pub gate: Gate,
    /// `step_graph_phase.gate_hard`.
    pub gate_hard: bool,
    /// `step_graph_phase.retry_limit`.
    pub retry_limit: i32,
    /// `step_graph_phase.input_kinds`.
    pub input_kinds: Vec<String>,
    /// `step_graph_phase.output_kind`: the `document.kind` this phase produces.
    pub output_kind: String,
    /// `step_graph_phase.isolation`; `None` means the project default.
    pub isolation: Option<Isolation>,
    /// `step_graph_phase.command_queue`.
    pub command_queue: CommandQueue,
    /// `step_graph_phase.verify_command`.
    pub verify_command: Option<String>,
    /// `step_graph_phase.template_name`.
    pub template_name: String,
    /// `step_graph_phase.template_version`; `None` follows the latest version.
    pub template_version: Option<i32>,
    /// `step_graph_phase.token_budget`; `None` falls back to `project.settings.token_budget`.
    pub token_budget: Option<i32>,
    /// `step_graph_phase.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `phase_agent` (§5.4): a candidate agent for a phase, in priority order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseAgent {
    /// `phase_agent.phase_id`.
    pub phase_id: PhaseId,
    /// `phase_agent.position`.
    pub position: i32,
    /// `phase_agent.agent_id`.
    pub agent_id: AgentId,
    /// `phase_agent.model`.
    pub model: String,
}

/// A row of `prompt_template` (§5.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// `prompt_template.id`.
    pub id: PromptTemplateId,
    /// `prompt_template.project_id`.
    pub project_id: ProjectId,
    /// `prompt_template.name`, normally a phase name.
    pub name: String,
    /// `prompt_template.version`.
    pub version: i32,
    /// `prompt_template.body`.
    pub body: String,
    /// `prompt_template.created_by`.
    pub created_by: UserId,
    /// `prompt_template.created_at`.
    pub created_at: DateTime<Utc>,
    /// `prompt_template.updated_at`.
    pub updated_at: DateTime<Utc>,
}
