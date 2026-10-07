//! Item kinds, step graphs and prompt templates (`docs/ANA-9.md` §5.4, §5.5).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{
    AgentId, ItemKindId, PersonaId, PhaseId, ProjectId, PromptTemplateId, StepGraphId, UserId,
};
use crate::model::persona::Persona;

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

impl CommandQueue {
    /// MOD-11 D16: whether a step of this phase gets `command_run` (and the prompt's command-queue
    /// section): `off` → never; `always` → always; `fan_out_only` → when the phase fans out to
    /// more than one agent or the item carries [`HEAVY_BUILD_TAG`] (ANA-5 `:337`, R-MCP-3).
    #[must_use]
    pub fn exposed(self, fan_out: i32, item_tags: &[String]) -> bool {
        match self {
            Self::Off => false,
            Self::Always => true,
            Self::FanOutOnly => fan_out > 1 || item_tags.iter().any(|tag| tag == HEAVY_BUILD_TAG),
        }
    }
}

/// MOD-11 D16's one resolver, as a function for callers holding the three values:
/// [`CommandQueue::exposed`].
#[must_use]
pub fn command_queue_exposed(mode: CommandQueue, fan_out: i32, item_tags: &[String]) -> bool {
    mode.exposed(fan_out, item_tags)
}

/// The item tag R-MCP-3 names: a `fan_out_only` phase queues the commands of an item carrying it
/// even when it does not fan out (MOD-11 D16).
pub const HEAVY_BUILD_TAG: &str = "heavy_build";

/// MOD-11 OQ-5: the shell prefixes refused once (`reject_once`) while `command_run` is exposed
/// (R-MCP-4), so an agent routes them through the queue. A prefix match only, ending at a word
/// (MOD-11 R1 L4: `make` never refuses `makepkg`): `cd x && cargo build` passes (R-6). The order
/// is a pin.
pub const HEAVY_COMMAND_PREFIXES: &[&str] = &[
    "cargo build",
    "cargo test",
    "cargo nextest",
    "cargo clippy",
    "cmake --build",
    "ctest",
    "make",
    "ninja",
    "msbuild",
    "dotnet build",
    "dotnet test",
    "npm test",
    "pnpm test",
    "go build",
    "go test",
];

/// MOD-11 D15: the class limits of a box. `app_setting.command_limits` (`0003_orchestration.sql`
/// seeds `{"build":1,"test":4,"verify":1}`) overlaid key by key with the box's own
/// `box.settings.command_limits`, which `box_settings` is (the value under that key, not the
/// whole settings object). A value that is not an object contributes nothing, and an entry
/// whose value is not a `u32` is skipped (the caller warns); a class missing from both is 1,
/// which [`command_limit`] answers.
#[must_use]
pub fn resolve_command_limits(
    box_settings: Option<&Value>,
    app: &BTreeMap<String, Value>,
) -> BTreeMap<String, u32> {
    let mut limits = BTreeMap::new();
    for layer in [app.get("command_limits"), box_settings]
        .into_iter()
        .flatten()
    {
        let Some(entries) = layer.as_object() else {
            continue;
        };
        for (class, value) in entries {
            if let Some(limit) = value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                limits.insert(class.clone(), limit);
            }
        }
    }
    limits
}

/// MOD-11 D15: the limit of `class` in resolved `limits`: a missing class is 1, and so is 0 (a
/// zero-slot class would admit nothing, ever: the `verify` reading of `ShellVerifier::new`).
#[must_use]
pub fn command_limit(limits: &BTreeMap<String, u32>, class: &str) -> u32 {
    limits.get(class).copied().unwrap_or(1).max(1)
}

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
    /// `step_graph.is_override` (ANA-2 §4.1): `true` only for `override_graph`'s per-item clone
    /// (MOD-9 D80); every other constructor passes `false`.
    pub is_override: bool,
}

/// Edit passed to [`crate::store::WriteStore::update_step_graph`]; `None` leaves the column.
///
/// `is_override` is not here: it is set once, at create ([`NewStepGraph::is_override`], MOD-9
/// D80), and never edited.
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
    /// `step_graph_phase.persona_id` (MOD-26 D5): the persona this phase runs under; `None` for
    /// none. Frozen by name and content into a run's snapshot at `StartRun` (D9).
    #[serde(default)]
    pub persona_id: Option<PersonaId>,
    /// `step_graph_phase.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// Edit passed to [`crate::store::WriteStore::update_phase`]; `None` leaves the column.
///
/// PRD D2's six editable columns minus `token_budget`, which the `Phase` rung of
/// [`crate::store::WriteStore::set_setting`] owns alone (plan D8): a column two writers could set
/// is the hole the rung design closes. `fan_out`, `isolation`, `command_queue`, `verify_command`
/// and `retry_limit` are MOD-4's and are rendered rather than edited, so they are not here either.
/// MOD-26 D5 adds the persona binding.
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
    /// `step_graph_phase.persona_id` (MOD-26 D5): `None` leaves the binding, `Some(None)` clears
    /// it, `Some(Some(id))` binds `id`, which must name a `persona` row (`references_no_row`).
    ///
    /// On the wire an absent key is `None` and `null` is `Some(None)`, so a clear survives a
    /// serde round trip (MOD-26 review L1).
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub persona: Option<Option<PersonaId>>,
}

/// Reads a present field as `Some(value)`, `null` included, for a double-option patch field whose
/// absent key falls to `#[serde(default)]`'s `None` (MOD-26 review L1).
pub(crate) fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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
    /// `phase_agent` rows in `position` order; empty for a phase that has none, as every phase of
    /// the demo fixture is.
    pub agents: Vec<PhaseAgent>,
    /// The `persona` row `phase.persona_id` names (MOD-26 D6); `None` when it names none.
    #[serde(default)]
    pub persona: Option<Persona>,
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

/// A `prompt_template` row to append (MOD-9 plan D1): everything but `version` and the two
/// instants, which the store assigns. `version` is the head's plus one, or 1 for a new name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPromptTemplate {
    /// `prompt_template.id`, minted client-side as a UUIDv7.
    pub id: PromptTemplateId,
    /// `prompt_template.project_id`.
    pub project_id: ProjectId,
    /// `prompt_template.name`; its role is `TemplateRole::of_name(name)`.
    pub name: String,
    /// `prompt_template.body`; the store refuses what `parse` refuses (plan D4).
    pub body: String,
    /// `prompt_template.created_by`.
    pub created_by: UserId,
}

impl PromptTemplate {
    /// Whether `name` may name a template (plan D4): non-empty, no leading or trailing whitespace,
    /// no `\n` or `\r`, and no U+0000, which Postgres `text` cannot hold. Here rather than in
    /// either store for `ItemKind::prefix_is_valid`'s reason: both stores refuse the same strings
    /// with the same sentence.
    #[must_use]
    pub fn name_is_valid(name: &str) -> bool {
        !name.is_empty() && name.trim() == name && !name.contains(['\n', '\r', '\0'])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::{
        CommandQueue, HEAVY_BUILD_TAG, HEAVY_COMMAND_PREFIXES, ItemKind, PhasePatch,
        PromptTemplate, command_limit, command_queue_exposed, resolve_command_limits,
    };
    use crate::model::ids::PersonaId;

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// MOD-11 D16: `off` never exposes `command_run`, whatever the fan-out or the tags.
    #[test]
    fn off_never_exposes() {
        for (fan_out, item_tags) in [(1, tags(&[])), (4, tags(&[HEAVY_BUILD_TAG]))] {
            assert!(!CommandQueue::Off.exposed(fan_out, &item_tags));
            assert!(!command_queue_exposed(
                CommandQueue::Off,
                fan_out,
                &item_tags
            ));
        }
    }

    /// MOD-11 D16: `always` always exposes it, a single-agent step without tags included.
    #[test]
    fn always_always_exposes() {
        for (fan_out, item_tags) in [(1, tags(&[])), (0, tags(&["rust"])), (3, tags(&[]))] {
            assert!(CommandQueue::Always.exposed(fan_out, &item_tags));
        }
    }

    /// MOD-11 D16, OQ-6: `fan_out_only` exposes it only to a fanned-out step (more than one
    /// agent) or on a `heavy_build` item; a tag that only resembles it does not count.
    #[test]
    fn fan_out_only_exposes_only_fanned_or_heavy() {
        let mode = CommandQueue::FanOutOnly;
        assert!(!mode.exposed(1, &tags(&[])), "one agent, no tag");
        assert!(
            !mode.exposed(0, &tags(&["rust", "heavy"])),
            "no heavy_build"
        );
        assert!(
            !mode.exposed(1, &tags(&["heavy_build_x", "HEAVY_BUILD"])),
            "an exact tag"
        );
        assert!(mode.exposed(2, &tags(&[])), "fanned out");
        assert!(mode.exposed(1, &tags(&["rust", "heavy_build"])), "heavy");
        assert_eq!(HEAVY_BUILD_TAG, "heavy_build");
    }

    /// MOD-11 D15: the box's `command_limits` overlays the app default key by key; a value of
    /// the wrong shape is skipped, an absent layer contributes nothing.
    #[test]
    fn limits_overlay_the_box_over_the_app_default() {
        let app = BTreeMap::from([(
            "command_limits".to_owned(),
            json!({"build": 1, "test": 4, "verify": 1}),
        )]);
        let seeded = BTreeMap::from([
            ("build".to_owned(), 1),
            ("test".to_owned(), 4),
            ("verify".to_owned(), 1),
        ]);
        assert_eq!(resolve_command_limits(None, &app), seeded, "no box value");
        assert_eq!(
            resolve_command_limits(Some(&json!({"test": 2, "run": 3})), &app),
            BTreeMap::from([
                ("build".to_owned(), 1),
                ("run".to_owned(), 3),
                ("test".to_owned(), 2),
                ("verify".to_owned(), 1),
            ]),
            "the box wins per key and adds its own classes"
        );
        assert_eq!(
            resolve_command_limits(
                Some(&json!({"build": "many", "test": -1, "verify": 2})),
                &app
            ),
            BTreeMap::from([
                ("build".to_owned(), 1),
                ("test".to_owned(), 4),
                ("verify".to_owned(), 2),
            ]),
            "an entry that is not a u32 is skipped, the app's value stands"
        );
        assert_eq!(
            resolve_command_limits(Some(&json!("many")), &app),
            seeded,
            "a box value that is not an object contributes nothing"
        );
        assert_eq!(
            resolve_command_limits(Some(&json!({"test": 3})), &BTreeMap::new()),
            BTreeMap::from([("test".to_owned(), 3)]),
            "no app default: the box alone"
        );
        assert_eq!(
            resolve_command_limits(
                None,
                &BTreeMap::from([("command_limits".to_owned(), Value::Null)])
            ),
            BTreeMap::new(),
            "neither layer: nothing"
        );
    }

    /// MOD-11 D15: a class missing from both layers is 1, and so is a stored 0.
    #[test]
    fn a_missing_or_zero_class_limit_is_one() {
        let limits = BTreeMap::from([("build".to_owned(), 0), ("test".to_owned(), 4)]);
        assert_eq!(command_limit(&limits, "test"), 4);
        assert_eq!(command_limit(&limits, "build"), 1, "zero reads as one");
        assert_eq!(command_limit(&limits, "run"), 1, "missing reads as one");
        assert_eq!(command_limit(&BTreeMap::new(), "verify"), 1);
    }

    /// MOD-11 OQ-5: the heavy-command list, in its order (a pin: the denials are spliced in it).
    #[test]
    fn the_heavy_prefixes_are_the_oq5_list() {
        assert_eq!(
            HEAVY_COMMAND_PREFIXES,
            [
                "cargo build",
                "cargo test",
                "cargo nextest",
                "cargo clippy",
                "cmake --build",
                "ctest",
                "make",
                "ninja",
                "msbuild",
                "dotnet build",
                "dotnet test",
                "npm test",
                "pnpm test",
                "go build",
                "go test",
            ]
        );
    }

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

    /// MOD-9 plan D4: the rule both stores apply before `parse`.
    #[test]
    fn template_names_are_trimmed_single_line_and_non_empty() {
        for good in ["implement", "judge", "a b", "é-1"] {
            assert!(
                PromptTemplate::name_is_valid(good),
                "`{good}` names a template"
            );
        }
        for bad in ["", " plan", "plan ", "a\nb", "a\rb", "\t", "a\0b"] {
            assert!(
                !PromptTemplate::name_is_valid(bad),
                "`{}` does not name a template",
                bad.escape_debug()
            );
        }
    }

    /// MOD-26 review L1: `PhasePatch.persona`'s three states survive a JSON round trip. An
    /// absent key leaves the binding (`None`), `null` clears it (`Some(None)`) and an id binds it
    /// (`Some(Some(id))`); a plain `Option<Option<_>>` reads `null` back as `None` and loses the
    /// clear.
    #[test]
    fn phase_patch_persona_round_trips_all_three_states() {
        let id = PersonaId::new();
        for (persona, json) in [
            (None, "{}".to_owned()),
            (Some(None), r#"{"persona":null}"#.to_owned()),
            (Some(Some(id)), format!(r#"{{"persona":"{id}"}}"#)),
        ] {
            let patch = PhasePatch {
                persona,
                ..PhasePatch::default()
            };
            let text = serde_json::to_string(&patch).expect("a patch serialises");
            let back: PhasePatch = serde_json::from_str(&text).expect("a patch deserialises");
            assert_eq!(back, patch, "`{text}` round-trips");
            let read: PhasePatch = serde_json::from_str(&json).expect("the literal deserialises");
            assert_eq!(read.persona, persona, "`{json}` reads as {persona:?}");
        }
    }
}
