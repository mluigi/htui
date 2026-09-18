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

impl ItemKind {
    /// The `item_kind.prefix` CHECK `^[A-Z][A-Z0-9]{1,15}$` (`0001_init.sql:282-290`) without a
    /// regex crate: 2..=16 bytes, the first `A-Z`, the rest `A-Z0-9`.
    ///
    /// Here rather than in either store because both have to refuse the same strings with the same
    /// sentence (plan D11): on Postgres the column would refuse anyway, but with a constraint name
    /// rather than the rule, and `MemStore` has no column to refuse for it. Byte-wise and not
    /// char-wise on purpose — a multi-byte character can never be `A-Z0-9`, so a leading `Ä` fails
    /// on its first byte and the length test is the column's own, which counts bytes too.
    #[must_use]
    pub fn prefix_is_valid(prefix: &str) -> bool {
        let bytes = prefix.as_bytes();
        (2..=16).contains(&bytes.len())
            && bytes[0].is_ascii_uppercase()
            && bytes[1..]
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    }
}

/// Arguments of [`crate::store::WriteStore::create_item_kind`].
///
/// `default_graph_id` is `NOT NULL` (`0001_init.sql:288`) and must name a graph of `project_id`,
/// which is the seed order ANA-9 §5.10 fixes: graphs, then phases, then kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewItemKind {
    /// `item_kind.id`, minted client-side as a UUIDv7.
    pub id: ItemKindId,
    /// `item_kind.project_id`.
    pub project_id: ProjectId,
    /// `item_kind.prefix`; [`ItemKind::prefix_is_valid`] is checked before the statement.
    pub prefix: String,
    /// `item_kind.name`, unique within the project.
    pub name: String,
    /// `item_kind.description`.
    pub description: String,
    /// `item_kind.default_graph_id`, which must belong to `project_id`.
    pub default_graph_id: StepGraphId,
    /// `item_kind.position`.
    pub position: i32,
}

/// Edit passed to [`crate::store::WriteStore::update_item_kind`]; `None` leaves the column.
///
/// Renaming `prefix` leaves the keys already minted under the old one alone (PRD D12): the store
/// forbids rewriting `item.key_prefix`, so the rename is a change to what the *next* mint spells.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemKindPatch {
    /// `item_kind.prefix`.
    pub prefix: Option<String>,
    /// `item_kind.name`.
    pub name: Option<String>,
    /// `item_kind.description`.
    pub description: Option<String>,
    /// `item_kind.default_graph_id`.
    pub default_graph_id: Option<StepGraphId>,
    /// `item_kind.position`.
    pub position: Option<i32>,
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
    /// `step_graph.is_override` (ANA-2 §4.1): a per-item clone, hidden from the graph list.
    pub is_override: bool,
    /// `step_graph.created_at`.
    pub created_at: DateTime<Utc>,
    /// `step_graph.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::create_step_graph`]. The graph lands with no phases;
/// [`crate::store::WriteStore::create_phase`] adds them one row at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewStepGraph {
    /// `step_graph.id`, minted client-side as a UUIDv7.
    pub id: StepGraphId,
    /// `step_graph.project_id`.
    pub project_id: ProjectId,
    /// `step_graph.name`, unique within the project.
    pub name: String,
    /// `step_graph.description`.
    pub description: String,
}

/// Edit passed to [`crate::store::WriteStore::update_step_graph`]; `None` leaves the column.
///
/// `is_override` is not here: the column arrives with MOD-4's `0003` and seeded graphs take its
/// default (PRD scope), so there is nothing for this milestone to write.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepGraphPatch {
    /// `step_graph.name`.
    pub name: Option<String>,
    /// `step_graph.description`.
    pub description: Option<String>,
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

/// Edit passed to [`crate::store::WriteStore::update_phase`]; `None` leaves the column.
///
/// PRD D2's six editable columns minus `token_budget`, which the `Phase` rung of
/// [`crate::store::WriteStore::set_setting`] owns alone (plan D8): a column two writers could set
/// is the hole the rung design closes. `fan_out`, `isolation`, `command_queue`, `verify_command`
/// and `retry_limit` are MOD-4's and are rendered rather than edited, so they are not here either.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhasePatch {
    /// `step_graph_phase.name`; `judge` and `handoff` are refused (ANA-5 §4.6).
    pub name: Option<String>,
    /// `step_graph_phase.position`, unique within the graph.
    pub position: Option<i32>,
    /// `step_graph_phase.template_name`.
    pub template_name: Option<String>,
    /// `step_graph_phase.gate_hard`.
    pub gate_hard: Option<bool>,
    /// `step_graph_phase.input_kinds`, replaced whole.
    pub input_kinds: Option<Vec<String>>,
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

/// `project.settings` as ANA-2 §4.7 reads it.
///
/// Read-only (plan D11) and every field defaults, so `'{}'` decodes. Nothing re-serialises the
/// struct onto the row, which is what keeps unknown keys alive: MOD-15's key-level `set_setting`
/// is the column's only writer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// The isolation a phase that names none resolves to.
    pub default_isolation: Isolation,
    /// Per-step token budget.
    pub token_budget: Option<i32>,
    /// How long finished runs are kept.
    pub retention_days: Option<i32>,
    /// How many earlier steps' transcripts a prompt may carry.
    pub cached_transcript_steps: Option<i32>,
    /// Whether raw `session_event` rows survive retention.
    pub keep_raw_events: bool,
    /// `R-ORCH-8`'s per-run cap, micros.
    pub per_token_cap_run: Option<i64>,
    /// `R-ORCH-8`'s per-batch cap, micros.
    pub per_token_cap_batch: Option<i64>,
    /// The deadline a phase that names none resolves to.
    pub step_deadline_seconds: Option<u32>,
    /// The agent a phase with no candidate resolves to.
    pub default_agent_id: Option<AgentId>,
    /// The fan-out judge a phase with none resolves to (ANA-2 §4.5).
    pub judge_agent_id: Option<AgentId>,
    /// Globs excluded from a `copy` isolation tree (ANA-2 §4.6).
    pub copy_exclude: Vec<String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            default_isolation: Isolation::Worktree,
            token_budget: None,
            retention_days: None,
            cached_transcript_steps: None,
            keep_raw_events: false,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            step_deadline_seconds: None,
            default_agent_id: None,
            judge_agent_id: None,
            copy_exclude: Vec::new(),
        }
    }
}

/// What `Backend::resolve_graph` answers in one round trip (ANA-2 §8): the graph an item runs
/// under and its phases with their candidate agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedGraph {
    /// The `step_graph` row: the item's override, else its kind's default.
    pub graph: StepGraph,
    /// In `position` order.
    pub phases: Vec<ResolvedPhase>,
}

/// One phase of a [`ResolvedGraph`] with the candidates the snapshot builder picks from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPhase {
    /// The `step_graph_phase` row.
    pub phase: StepGraphPhase,
    /// `phase_agent` rows in `position` order; empty on `MemStore`, which holds no such table.
    pub agents: Vec<PhaseAgent>,
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

#[cfg(test)]
mod tests {
    use super::ItemKind;

    /// The CHECK, byte for byte: `^[A-Z][A-Z0-9]{1,15}$` (`0001_init.sql:282-290`).
    ///
    /// The store calls this before the statement so both backends refuse the same strings with the
    /// same sentence (plan D11), which makes this test — not a database — the thing that says the
    /// two agree with the column.
    #[test]
    fn prefix_is_valid_mirrors_the_check() {
        let sixteen = format!("A{}", "9".repeat(15));
        for good in ["ANA", "A1", "AB", sixteen.as_str()] {
            assert!(ItemKind::prefix_is_valid(good), "`{good}` is a prefix");
        }
        let seventeen = format!("A{}", "9".repeat(16));
        for bad in [
            "feat",
            "1A",
            "A",
            "",
            seventeen.as_str(),
            "AN-A",
            "AN A",
            "ÄNA",
            "A_B",
        ] {
            assert!(!ItemKind::prefix_is_valid(bad), "`{bad}` is not a prefix");
        }
    }
}
