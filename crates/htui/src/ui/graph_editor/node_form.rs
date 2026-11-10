//! MOD-141 D8: the node form - every M1 node field, the ordered candidate agents, and the M2
//! fields shown read-only. Key-agnostic: the editor maps its acts onto these methods.

use crossterm::event::KeyEvent;
use htui_core::model::{
    AgentId, AgentSummary, CommandQueue, Gate, GraphBody, Isolation, NodeBody, PersonaId,
    PhaseAgent, PhaseId, PlanScope, Split,
};
use ratatui::style::Style;
use ratatui::text::Line;

use super::{Lookups, step_name};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::label;
use crate::ui::{FieldOutcome, TextField, Theme};

/// The name left empty.
pub(crate) const NAME_REQUIRED: &str = "`name` is required";
/// A retry limit that is not a whole number of at least 0.
pub(crate) const RETRY_IS_A_NUMBER: &str = "`retry limit` is a whole number, 0 or more";
/// A token budget that is neither empty nor a whole number of at least 1.
pub(crate) const BUDGET_IS_A_NUMBER: &str =
    "`token budget` is a whole number of at least 1, or empty";
/// No template chosen while the scope has names to choose from.
pub(crate) const TEMPLATE_REQUIRED: &str = "`template` is required; choose one";
/// No template name to choose, and none chosen.
pub(crate) const NO_TEMPLATE: &str = "no template to choose; add one in Skills \u{203a} Templates";
/// The agents' read has not answered yet.
pub(crate) const AGENTS_NOT_READ: &str =
    "agents not read yet; the candidates can be edited once they are";
/// The agents' read answered with none.
pub(crate) const NO_AGENTS: &str = "no agents to choose from; add one in Settings \u{203a} Agents";
/// A candidate edit with no candidate row focused.
pub(crate) const NO_CANDIDATE: &str = "no candidate agent is selected";
/// What a `divide` node's split row adds.
pub(crate) const DIVIDE_NOTE: &str = "runs once MOD-139 lands";
/// The placeholder row's text while the node has no candidate, naming `add`, the key the editor
/// binds to adding one (D12, R-TUI-10: the key comes from the key file, never this module), or
/// `none` alone while nothing is bound to it.
pub(crate) fn no_candidates(add: Option<&str>) -> String {
    add.map_or_else(|| "none".to_owned(), |key| format!("none ({key} adds one)"))
}

/// The fixed rows before the candidates, in focus order.
const FIXED_ROWS: usize = 13;
/// Row indices of the fixed rows.
const NAME: usize = 0;
const TEMPLATE: usize = 1;
const VERSION: usize = 2;
const PERSONA: usize = 3;
const GATE: usize = 4;
const HARD_GATE: usize = 5;
const RETRY: usize = 6;
const ISOLATION: usize = 7;
const COMMANDS: usize = 8;
const VERIFY: usize = 9;
const BUDGET: usize = 10;
const INPUTS: usize = 11;
const OUTPUT: usize = 12;

/// Which node the form writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeTarget {
    /// A new node, added after `after` (D10), or into an empty graph.
    New {
        /// The selected node it follows.
        after: Option<PhaseId>,
    },
    /// The body's node with this id.
    Edit(PhaseId),
}

/// The focused row's kind, for the editor's key mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRow {
    /// A text row: it takes typed keys and pastes.
    Text,
    /// A choice row: it cycles.
    Choice,
    /// The hard-gate toggle: it cycles by flipping.
    Toggle,
    /// Candidate `n` (0-based): it cycles over the agent and model pairs.
    Candidate(usize),
    /// The placeholder row shown while there is no candidate.
    NoCandidates,
}

/// The open node form (D8).
pub(crate) struct NodeForm {
    /// The node it writes back to.
    target: NodeTarget,
    /// `name`.
    name: TextField,
    /// `retry_limit`, as typed.
    retry_limit: TextField,
    /// `verify_command`, as typed.
    verify_command: TextField,
    /// `token_budget`, as typed.
    token_budget: TextField,
    /// `input_kinds`, comma separated.
    input_kinds: TextField,
    /// `output_kind`.
    output_kind: TextField,
    /// `template_name`.
    template_name: String,
    /// `template_version`; `None` follows the latest.
    template_version: Option<i32>,
    /// `persona_id`.
    persona: Option<PersonaId>,
    /// `gate`.
    gate: Gate,
    /// `gate_hard`.
    gate_hard: bool,
    /// `isolation`; `None` is the project default.
    isolation: Option<Isolation>,
    /// `command_queue`.
    command_queue: CommandQueue,
    /// The candidate agents in priority order: agent and model.
    candidates: Vec<(AgentId, String)>,
    /// The focused row: [`FIXED_ROWS`] fixed rows, then the candidates or the placeholder.
    focus: usize,
}

/// The target, the focus and the candidate count, never what was typed (H-7).
impl core::fmt::Debug for NodeForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeForm")
            .field("target", &self.target)
            .field("focus", &self.focus)
            .field("candidates", &self.candidates.len())
            .finish_non_exhaustive()
    }
}

impl NodeForm {
    /// The form prefilled from `node` (D8), writing back to `target`.
    pub(crate) fn new(node: &NodeBody, target: NodeTarget) -> Self {
        let phase = &node.phase;
        Self {
            target,
            name: TextField::with_text(&phase.name),
            retry_limit: TextField::with_text(&phase.retry_limit.to_string()),
            verify_command: TextField::with_text(phase.verify_command.as_deref().unwrap_or("")),
            token_budget: TextField::with_text(
                &phase
                    .token_budget
                    .map_or_else(String::new, |budget| budget.to_string()),
            ),
            input_kinds: TextField::with_text(&phase.input_kinds.join(", ")),
            output_kind: TextField::with_text(&phase.output_kind),
            template_name: phase.template_name.clone(),
            template_version: phase.template_version,
            persona: phase.persona_id,
            gate: phase.gate,
            gate_hard: phase.gate_hard,
            isolation: phase.isolation,
            command_queue: phase.command_queue,
            candidates: node
                .agents
                .iter()
                .map(|agent| (agent.agent_id, agent.model.clone()))
                .collect(),
            focus: NAME,
        }
    }

    /// The node the form writes back to.
    pub(crate) const fn target(&self) -> NodeTarget {
        self.target
    }

    /// The focused row's kind.
    pub(crate) fn focused(&self) -> NodeRow {
        match self.focus {
            NAME | RETRY | VERIFY | BUDGET | INPUTS | OUTPUT => NodeRow::Text,
            TEMPLATE | VERSION | PERSONA | GATE | ISOLATION | COMMANDS => NodeRow::Choice,
            HARD_GATE => NodeRow::Toggle,
            _ if self.candidates.is_empty() => NodeRow::NoCandidates,
            at => NodeRow::Candidate(at - FIXED_ROWS),
        }
    }

    /// How many rows take the focus: the fixed rows, then the candidates or the placeholder.
    fn focusable(&self) -> usize {
        FIXED_ROWS + self.candidates.len().max(1)
    }

    /// Focus on the next row, wrapping. Read-only rows are never focused.
    pub(crate) fn next_field(&mut self) {
        self.focus = (self.focus + 1) % self.focusable();
    }

    /// Focus on the previous row, wrapping.
    pub(crate) fn prev_field(&mut self) {
        let rows = self.focusable();
        self.focus = (self.focus + rows - 1) % rows;
    }

    /// The focused text field, if a text row has the focus.
    fn text_mut(&mut self) -> Option<&mut TextField> {
        match self.focus {
            NAME => Some(&mut self.name),
            RETRY => Some(&mut self.retry_limit),
            VERIFY => Some(&mut self.verify_command),
            BUDGET => Some(&mut self.token_budget),
            INPUTS => Some(&mut self.input_kinds),
            OUTPUT => Some(&mut self.output_kind),
            _ => None,
        }
    }

    /// The focused text row takes the key; [`FieldOutcome::Pass`] off a text row.
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome {
        self.text_mut()
            .map_or(FieldOutcome::Pass, |field| field.on_key(key))
    }

    /// The focused text row takes the paste; any other row ignores it.
    pub(crate) fn on_paste(&mut self, text: &str) {
        if let Some(field) = self.text_mut() {
            // An unmasked field always takes a paste.
            field.on_paste(text);
        }
    }

    /// The focused choice, toggle or candidate row moves one option, wrapping. A template change
    /// resets the version to latest. A text row and the placeholder row do nothing.
    ///
    /// # Errors
    /// [`NO_TEMPLATE`] on the template row with no name to choose; [`AGENTS_NOT_READ`] or
    /// [`NO_AGENTS`] on a candidate row.
    pub(crate) fn cycle(&mut self, forward: bool, lookups: &Lookups) -> Result<(), String> {
        match self.focused() {
            NodeRow::Text | NodeRow::NoCandidates => {}
            NodeRow::Toggle => self.gate_hard = !self.gate_hard,
            NodeRow::Candidate(at) => {
                let pairs = pairs_of(lookups)?;
                if let Some(candidate) = self.candidates.get_mut(at) {
                    *candidate = step(&pairs, candidate, forward);
                }
            }
            NodeRow::Choice => match self.focus {
                TEMPLATE => {
                    let names = self.template_names(lookups);
                    if names.is_empty() {
                        return Err(NO_TEMPLATE.to_owned());
                    }
                    let name = step(&names, &self.template_name, forward);
                    if name != self.template_name {
                        self.template_name = name;
                        self.template_version = None;
                    }
                }
                VERSION => {
                    self.template_version =
                        step(&self.versions(lookups), &self.template_version, forward);
                }
                PERSONA => self.persona = step(&self.personas(lookups), &self.persona, forward),
                GATE => self.gate = step(Gate::ALL, &self.gate, forward),
                ISOLATION => {
                    let options: Vec<Option<Isolation>> = core::iter::once(None)
                        .chain(Isolation::ALL.iter().copied().map(Some))
                        .collect();
                    self.isolation = step(&options, &self.isolation, forward);
                }
                _ => self.command_queue = step(CommandQueue::ALL, &self.command_queue, forward),
            },
        }
        Ok(())
    }

    /// The template names to choose from: the scope's, sorted and unique, with the current name
    /// kept even when the scope lacks it.
    fn template_names(&self, lookups: &Lookups) -> Vec<String> {
        let mut names: Vec<String> = lookups
            .templates
            .iter()
            .flatten()
            .map(|template| template.name.clone())
            .collect();
        if !self.template_name.is_empty() {
            names.push(self.template_name.clone());
        }
        names.sort();
        names.dedup();
        names
    }

    /// The versions to choose from: latest, then every version of the chosen name, newest first,
    /// the current one kept even when the scope lacks it.
    fn versions(&self, lookups: &Lookups) -> Vec<Option<i32>> {
        let mut versions: Vec<i32> = lookups
            .templates
            .iter()
            .flatten()
            .filter(|template| template.name == self.template_name)
            .map(|template| template.version)
            .chain(self.template_version)
            .collect();
        versions.sort_unstable_by(|a, b| b.cmp(a));
        versions.dedup();
        core::iter::once(None)
            .chain(versions.into_iter().map(Some))
            .collect()
    }

    /// The personas to choose from: none, the registry in its order, then the current id when
    /// the registry lacks it.
    fn personas(&self, lookups: &Lookups) -> Vec<Option<PersonaId>> {
        let mut personas: Vec<Option<PersonaId>> = core::iter::once(None)
            .chain(lookups.personas.iter().map(|persona| Some(persona.id)))
            .collect();
        if !personas.contains(&self.persona) {
            personas.push(self.persona);
        }
        personas
    }

    /// Appends the first (agent, model) pair; focus moves to it.
    ///
    /// # Errors
    /// [`AGENTS_NOT_READ`] or [`NO_AGENTS`].
    pub(crate) fn candidate_add(&mut self, lookups: &Lookups) -> Result<(), String> {
        let first = pairs_of(lookups)?.swap_remove(0);
        self.candidates.push(first);
        self.focus = FIXED_ROWS + self.candidates.len() - 1;
        Ok(())
    }

    /// The focused candidate's index.
    fn candidate_at(&self) -> Result<usize, String> {
        match self.focused() {
            NodeRow::Candidate(at) => Ok(at),
            _ => Err(NO_CANDIDATE.to_owned()),
        }
    }

    /// Removes the focused candidate; focus stays on the row now there, or the last.
    ///
    /// # Errors
    /// [`NO_CANDIDATE`] off a candidate row.
    pub(crate) fn candidate_remove(&mut self) -> Result<(), String> {
        let at = self.candidate_at()?;
        self.candidates.remove(at);
        self.focus = FIXED_ROWS + at.min(self.candidates.len().saturating_sub(1));
        Ok(())
    }

    /// Moves the focused candidate one place up or down; focus follows it. At either end it
    /// stays.
    ///
    /// # Errors
    /// [`NO_CANDIDATE`] off a candidate row.
    pub(crate) fn candidate_move(&mut self, up: bool) -> Result<(), String> {
        let at = self.candidate_at()?;
        let to = if up {
            at.checked_sub(1)
        } else {
            Some(at + 1).filter(|to| *to < self.candidates.len())
        };
        if let Some(to) = to {
            self.candidates.swap(at, to);
            self.focus = FIXED_ROWS + to;
        }
        Ok(())
    }

    /// Parses and assembles the node. `base` supplies every column the form does not edit (id,
    /// graph id, position, fan-out, split, palette, plan scope, the divide columns, the parts and
    /// `updated_at`). `lookups` tells an unchosen template with names to choose
    /// ([`TEMPLATE_REQUIRED`]) from one with none ([`NO_TEMPLATE`]).
    ///
    /// # Errors
    /// The first refusal in row order.
    pub(crate) fn build(&self, base: &NodeBody, lookups: &Lookups) -> Result<NodeBody, String> {
        let name = text_of(&self.name);
        if name.is_empty() {
            return Err(NAME_REQUIRED.to_owned());
        }
        if self.template_name.is_empty() {
            return Err(if self.template_names(lookups).is_empty() {
                NO_TEMPLATE
            } else {
                TEMPLATE_REQUIRED
            }
            .to_owned());
        }
        let retry_limit = match text_of(&self.retry_limit).parse::<i32>() {
            Ok(limit) if limit >= 0 => limit,
            _ => return Err(RETRY_IS_A_NUMBER.to_owned()),
        };
        let verify = text_of(&self.verify_command);
        let budget = text_of(&self.token_budget);
        let token_budget = if budget.is_empty() {
            None
        } else {
            match budget.parse::<i32>() {
                Ok(budget) if budget >= 1 => Some(budget),
                _ => return Err(BUDGET_IS_A_NUMBER.to_owned()),
            }
        };
        let output = text_of(&self.output_kind);
        let mut node = base.clone();
        let phase = &mut node.phase;
        phase.name = name.to_owned();
        phase.template_name.clone_from(&self.template_name);
        phase.template_version = self.template_version;
        phase.persona_id = self.persona;
        phase.gate = self.gate;
        phase.gate_hard = self.gate_hard;
        phase.retry_limit = retry_limit;
        phase.isolation = self.isolation;
        phase.command_queue = self.command_queue;
        phase.verify_command = (!verify.is_empty()).then(|| verify.to_owned());
        phase.token_budget = token_budget;
        phase.input_kinds = input_kinds(text_of(&self.input_kinds));
        phase.output_kind = if output.is_empty() { name } else { output }.to_owned();
        node.agents = self
            .candidates
            .iter()
            .enumerate()
            .map(|(at, (agent_id, model))| PhaseAgent {
                phase_id: base.phase.id,
                position: i32::try_from(at).unwrap_or(i32::MAX),
                agent_id: *agent_id,
                model: model.clone(),
            })
            .collect();
        Ok(node)
    }

    /// The form's title: `step research`, `new step after research` or `new step`.
    pub(crate) fn title(&self, body: &GraphBody) -> String {
        match self.target {
            NodeTarget::Edit(id) => {
                let name = step_name(body, id).unwrap_or_else(|| text_of(&self.name));
                format!("step {name}")
            }
            NodeTarget::New { after } => after.and_then(|id| step_name(body, id)).map_or_else(
                || "new step".to_owned(),
                |name| format!("new step after {name}"),
            ),
        }
    }

    /// The form's rows `width` cells wide, the focused one being edited, then the read-only rows
    /// from `base`. `add` is the label of the key that adds a candidate, for the placeholder row.
    pub(crate) fn lines(
        &self,
        base: &NodeBody,
        lookups: &Lookups,
        add: Option<&str>,
        width: u16,
        accent: Style,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let template = if self.template_name.is_empty() {
            "none".to_owned()
        } else {
            self.template_name.clone()
        };
        let version = self
            .template_version
            .map_or_else(|| "latest".to_owned(), |version| format!("v{version}"));
        let persona = persona_word(self.persona, lookups);
        let isolation = self.isolation.map_or("project default", label::isolation);
        let candidates: Vec<(String, String)> = self
            .candidates
            .iter()
            .enumerate()
            .map(|(at, (agent, model))| {
                (
                    format!("agent {}", at + 1),
                    pair_word(*agent, model, lookups),
                )
            })
            .collect();
        let template_help = if lookups.templates.is_none() {
            "templates not read yet; the current name is kept"
        } else {
            "the prompt template this step runs"
        };
        let mut form = Form::new()
            .editing(true)
            .field(Field::new("name", Value::Text(&self.name)))
            .field(Field::new("template", Value::Choice(&template)).help(template_help))
            .field(
                Field::new("version", Value::Choice(&version))
                    .help("latest follows every new version"),
            )
            .field(Field::new("persona", Value::Choice(&persona)))
            .field(Field::new("gate", Value::Choice(label::gate(self.gate))))
            .field(Field::new("hard gate", Value::Toggle(self.gate_hard)))
            .field(Field::new("retry limit", Value::Text(&self.retry_limit)))
            .field(Field::new("isolation", Value::Choice(isolation)))
            .field(Field::new(
                "commands",
                Value::Choice(label::command_queue(self.command_queue)),
            ))
            .field(Field::new("verify", Value::Text(&self.verify_command)).help("empty for none"))
            .field(
                Field::new("token budget", Value::Text(&self.token_budget))
                    .help("empty for the project's budget"),
            )
            .field(
                Field::new("inputs", Value::Text(&self.input_kinds))
                    .help("document kinds, comma separated"),
            )
            .field(Field::new("output", Value::Text(&self.output_kind)).help("empty for the name"));
        if candidates.is_empty() {
            form = form.field(Field::new(
                "agents",
                Value::ReadOnly(no_candidates(add).into()),
            ));
        }
        for (label, value) in &candidates {
            form = form.field(
                Field::new(label, Value::Choice(value))
                    .help("agent and model, tried in this order"),
            );
        }
        let phase = &base.phase;
        let split = match phase.split {
            Split::None => "none".to_owned(),
            Split::Compete => "compete".to_owned(),
            Split::Divide => format!("divide \u{b7} {DIVIDE_NOTE}"),
        };
        let read_only =
            |label, text: String| Field::new(label, Value::ReadOnly(text.into())).style(theme.dim);
        form = form
            .field(read_only("split", split))
            .field(read_only("fan-out", phase.fan_out.to_string()))
            .field(read_only(
                "palette",
                if phase.palette { "yes" } else { "no" }.to_owned(),
            ))
            .field(read_only(
                "plan scope",
                phase.plan_scope.map_or("none", plan_scope).to_owned(),
            ));
        if phase.split == Split::Divide {
            form = form.field(read_only("divide", divide_words(base, lookups)));
        }
        form.field(read_only(
            "parts",
            label::plural(base.parts.len(), "part", "parts"),
        ))
        .field(read_only("deadline", "edited in M2".to_owned()))
        .focus(self.focus)
        .lines(width, accent, theme)
    }
}

/// What a text field holds, trimmed.
fn text_of(field: &TextField) -> &str {
    field.text().unwrap_or_default().trim()
}

/// `input_kinds` as `kinds.rs` reads it: split on `,`, each part trimmed, empty parts dropped,
/// order kept, no de-duplication.
fn input_kinds(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One step from `current` through `options`, wrapping; from a value the options lack, forward
/// goes to the first and back to the last.
fn step<T: Clone + PartialEq>(options: &[T], current: &T, forward: bool) -> T {
    let len = options.len();
    let next = match options.iter().position(|option| option == current) {
        Some(at) if forward => (at + 1) % len,
        Some(at) => (at + len - 1) % len,
        None if forward => 0,
        None => len.saturating_sub(1),
    };
    options
        .get(next)
        .cloned()
        .unwrap_or_else(|| current.clone())
}

/// The (agent, model) pairs to choose from.
fn pairs_of(lookups: &Lookups) -> Result<Vec<(AgentId, String)>, String> {
    let agents = lookups
        .agents
        .as_deref()
        .ok_or_else(|| AGENTS_NOT_READ.to_owned())?;
    let pairs = agent_pairs(agents);
    if pairs.is_empty() {
        return Err(NO_AGENTS.to_owned());
    }
    Ok(pairs)
}

/// Every (agent, model) pair, agents in reply order, models in `agent.models` order; an agent
/// with no models gives `(agent, default_model or "")`.
pub(crate) fn agent_pairs(agents: &[AgentSummary]) -> Vec<(AgentId, String)> {
    agents
        .iter()
        .flat_map(|summary| {
            let agent = &summary.agent;
            if agent.models.is_empty() {
                vec![(agent.id, agent.default_model.clone().unwrap_or_default())]
            } else {
                agent
                    .models
                    .iter()
                    .map(|model| (agent.id, model.clone()))
                    .collect()
            }
        })
        .collect()
}

/// An agent by name, or its id when the agents are unread or lack it.
fn agent_word(agent: AgentId, lookups: &Lookups) -> String {
    lookups
        .agents
        .iter()
        .flatten()
        .find(|summary| summary.agent.id == agent)
        .map_or_else(|| agent.to_string(), |summary| summary.agent.name.clone())
}

/// A candidate: `claude · opus`, or the agent alone when the model is empty.
fn pair_word(agent: AgentId, model: &str, lookups: &Lookups) -> String {
    let agent = agent_word(agent, lookups);
    if model.is_empty() {
        agent
    } else {
        format!("{agent} \u{b7} {model}")
    }
}

/// A persona by name, its id when the registry lacks it, `none` for none.
fn persona_word(persona: Option<PersonaId>, lookups: &Lookups) -> String {
    persona.map_or_else(
        || "none".to_owned(),
        |id| {
            lookups
                .personas
                .iter()
                .find(|persona| persona.id == id)
                .map_or_else(|| id.to_string(), |persona| persona.name.clone())
        },
    )
}

/// What a planner node may plan, in words.
const fn plan_scope(scope: PlanScope) -> &'static str {
    match scope {
        PlanScope::Parts => "its parts",
        PlanScope::Successors => "its successors",
        PlanScope::Graph => "the whole graph",
    }
}

/// A `divide` node's planner, integrate and part cap: `planner architect, claude · opus ·
/// integrate reviewer, agy · at most 4 parts`.
fn divide_words(node: &NodeBody, lookups: &Lookups) -> String {
    let phase = &node.phase;
    let who = |persona: Option<PersonaId>, agent: Option<AgentId>, model: &Option<String>| {
        let agent = agent.map_or_else(
            || "no agent".to_owned(),
            |agent| pair_word(agent, model.as_deref().unwrap_or(""), lookups),
        );
        format!("{}, {agent}", persona_word(persona, lookups))
    };
    let parts = phase.max_parts.map_or_else(
        || "no part cap".to_owned(),
        |max| format!("at most {max} parts"),
    );
    format!(
        "planner {} \u{b7} integrate {} \u{b7} {parts}",
        who(
            phase.planner_persona_id,
            phase.planner_agent_id,
            &phase.planner_model
        ),
        who(
            phase.integrate_persona_id,
            phase.integrate_agent_id,
            &phase.integrate_model
        ),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        GraphScope, PhasePart, ProjectId, PromptTemplate, PromptTemplateId, StepGraph, StepGraphId,
        UserId,
    };
    use htui_core::seed::{KINDS, phase_row};
    use ratatui::style::Color;

    use super::*;
    use crate::catalogue::PersonaSummary;

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// The candidate-add key's label as the compiled keys write it.
    const ADD: Option<&str> = Some("Alt+n");

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    /// The lines as text, trailing blanks dropped.
    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The analysis graph's seeded `research` node, no candidates; the same ids on every call.
    fn research() -> NodeBody {
        let id = PhaseId::from_uuid(uuid::Uuid::from_u128(0x0141_0001));
        let graph = StepGraphId::from_uuid(uuid::Uuid::from_u128(0x0141_0002));
        NodeBody {
            phase: phase_row(id, graph, 0, &KINDS[0].phases[0], now()),
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    fn summary(id: AgentId, name: &str, models: &[&str], default: Option<&str>) -> AgentSummary {
        let mut agent = htui_core::model::agent::seed_rows(now())
            .into_iter()
            .next()
            .expect("a seeded agent");
        agent.id = id;
        agent.name = name.to_owned();
        agent.models = models.iter().map(|model| (*model).to_owned()).collect();
        agent.default_model = default.map(str::to_owned);
        AgentSummary {
            agent,
            on_box: None,
            user_off: false,
        }
    }

    fn template(name: &str, version: i32) -> PromptTemplate {
        PromptTemplate {
            id: PromptTemplateId::new(),
            project_id: ProjectId::new(),
            name: name.to_owned(),
            version,
            body: String::new(),
            created_by: UserId::new(),
            created_at: now(),
            updated_at: now(),
        }
    }

    /// Two personas, two agents (claude with two models, agy with a default only) and the
    /// templates plan v1, research v1 and v2, verdict v1.
    fn lookups() -> Lookups {
        Lookups {
            personas: vec![
                PersonaSummary {
                    id: ids::PERSONA_REVIEWER,
                    name: "reviewer".to_owned(),
                },
                PersonaSummary {
                    id: ids::PERSONA_ARCHITECT,
                    name: "architect".to_owned(),
                },
            ],
            agents: Some(vec![
                summary(
                    ids::AGENT_CLAUDE,
                    "claude",
                    &["opus", "sonnet"],
                    Some("opus"),
                ),
                summary(ids::AGENT_AGY, "agy", &[], Some("gemini")),
            ]),
            templates: Some(vec![
                template("verdict", 1),
                template("research", 1),
                template("plan", 1),
                template("research", 2),
            ]),
        }
    }

    fn candidate(node: &NodeBody, position: i32, agent: AgentId, model: &str) -> PhaseAgent {
        PhaseAgent {
            phase_id: node.phase.id,
            position,
            agent_id: agent,
            model: model.to_owned(),
        }
    }

    /// `research` with persona `reviewer` and two candidates.
    fn research_bound() -> NodeBody {
        let mut node = research();
        node.phase.persona_id = Some(ids::PERSONA_REVIEWER);
        node.agents = vec![
            candidate(&node, 0, ids::AGENT_CLAUDE, "sonnet"),
            candidate(&node, 1, ids::AGENT_AGY, "gemini"),
        ];
        node
    }

    fn edit(node: &NodeBody) -> NodeForm {
        NodeForm::new(node, NodeTarget::Edit(node.phase.id))
    }

    fn focus(form: &mut NodeForm, row: usize) {
        form.focus = row;
    }

    /// Replaces what text row `row` holds by typing `text` into it.
    fn set_text(form: &mut NodeForm, row: usize, text: &str) {
        focus(form, row);
        for _ in 0..64 {
            let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
        for c in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
    }

    /// The node `form` builds over `research()`'s columns.
    fn built(form: &NodeForm) -> Result<NodeBody, String> {
        form.build(&research(), &lookups())
    }

    fn cycle_at(form: &mut NodeForm, row: usize, lookups: &Lookups) {
        focus(form, row);
        form.cycle(true, lookups).expect("a choice row cycles");
    }

    #[test]
    fn the_form_prefills_every_m1_field_from_the_node() {
        let mut node = research_bound();
        node.phase.gate = Gate::OnFailure;
        node.phase.gate_hard = true;
        node.phase.retry_limit = 3;
        node.phase.isolation = Some(Isolation::Copy);
        node.phase.command_queue = CommandQueue::Always;
        node.phase.verify_command = Some("cargo test".to_owned());
        node.phase.token_budget = Some(5000);
        node.phase.input_kinds = vec!["plan".to_owned(), "notes".to_owned()];
        node.phase.output_kind = "findings".to_owned();
        node.phase.template_version = Some(2);
        let form = edit(&node);
        assert_eq!(form.target(), NodeTarget::Edit(node.phase.id));
        assert_eq!(form.focused(), NodeRow::Text, "the name has the focus");
        assert_eq!(form.name.text(), Some("research"));
        assert_eq!(form.retry_limit.text(), Some("3"));
        assert_eq!(form.verify_command.text(), Some("cargo test"));
        assert_eq!(form.token_budget.text(), Some("5000"));
        assert_eq!(form.input_kinds.text(), Some("plan, notes"));
        assert_eq!(form.output_kind.text(), Some("findings"));
        assert_eq!(form.template_name, "research");
        assert_eq!(form.template_version, Some(2));
        assert_eq!(form.persona, Some(ids::PERSONA_REVIEWER));
        assert_eq!(form.gate, Gate::OnFailure);
        assert!(form.gate_hard);
        assert_eq!(form.isolation, Some(Isolation::Copy));
        assert_eq!(form.command_queue, CommandQueue::Always);
        assert_eq!(
            form.candidates,
            [
                (ids::AGENT_CLAUDE, "sonnet".to_owned()),
                (ids::AGENT_AGY, "gemini".to_owned()),
            ]
        );
        // Untouched, the form writes back exactly what it opened on.
        assert_eq!(form.build(&node, &lookups()), Ok(node));
    }

    #[test]
    fn build_writes_every_field_back_and_keeps_the_m2_columns_of_the_base() {
        let lookups = lookups();
        let mut form = edit(&research());
        set_text(&mut form, NAME, "  dig  ");
        cycle_at(&mut form, TEMPLATE, &lookups); // research → verdict
        cycle_at(&mut form, VERSION, &lookups); // latest → v1
        cycle_at(&mut form, PERSONA, &lookups); // none → reviewer
        cycle_at(&mut form, GATE, &lookups); // always → on failure
        cycle_at(&mut form, HARD_GATE, &lookups);
        set_text(&mut form, RETRY, "2");
        cycle_at(&mut form, ISOLATION, &lookups); // project default → worktree
        cycle_at(&mut form, COMMANDS, &lookups); // fan-out only → always
        set_text(&mut form, VERIFY, "make check");
        set_text(&mut form, BUDGET, "1200");
        set_text(&mut form, INPUTS, "plan");
        set_text(&mut form, OUTPUT, "dig");
        form.candidate_add(&lookups).expect("agents read");

        let mut base = research();
        base.phase.position = 4;
        base.phase.fan_out = 2;
        base.phase.split = Split::Divide;
        base.phase.palette = true;
        base.phase.plan_scope = Some(PlanScope::Parts);
        base.phase.planner_persona_id = Some(ids::PERSONA_ARCHITECT);
        base.phase.planner_agent_id = Some(ids::AGENT_CLAUDE);
        base.phase.planner_model = Some("opus".to_owned());
        base.phase.integrate_persona_id = Some(ids::PERSONA_REVIEWER);
        base.phase.integrate_agent_id = Some(ids::AGENT_AGY);
        base.phase.integrate_model = Some("gemini".to_owned());
        base.phase.max_parts = Some(3);
        base.parts = vec![part(&base, 0, "ui")];
        let node = form.build(&base, &lookups).expect("a valid form");

        let mut want = base.clone();
        want.phase.name = "dig".to_owned();
        want.phase.template_name = "verdict".to_owned();
        want.phase.template_version = Some(1);
        want.phase.persona_id = Some(ids::PERSONA_REVIEWER);
        want.phase.gate = Gate::OnFailure;
        want.phase.gate_hard = true;
        want.phase.retry_limit = 2;
        want.phase.isolation = Some(Isolation::Worktree);
        want.phase.command_queue = CommandQueue::Always;
        want.phase.verify_command = Some("make check".to_owned());
        want.phase.token_budget = Some(1200);
        want.phase.input_kinds = vec!["plan".to_owned()];
        want.phase.output_kind = "dig".to_owned();
        want.agents = vec![candidate(&base, 0, ids::AGENT_CLAUDE, "opus")];
        assert_eq!(node, want);
    }

    fn part(node: &NodeBody, position: i32, key: &str) -> PhasePart {
        PhasePart {
            phase_id: node.phase.id,
            position,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut form = edit(&research());
        set_text(&mut form, NAME, "   ");
        assert_eq!(built(&form), Err(NAME_REQUIRED.to_owned()));
    }

    #[test]
    fn retry_limit_is_a_whole_number_0_or_more() {
        for (typed, want) in [
            ("0", Ok(0)),
            (" 4 ", Ok(4)),
            ("", Err(RETRY_IS_A_NUMBER.to_owned())),
            ("-1", Err(RETRY_IS_A_NUMBER.to_owned())),
            ("two", Err(RETRY_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = edit(&research());
            set_text(&mut form, RETRY, typed);
            assert_eq!(
                built(&form).map(|node| node.phase.retry_limit),
                want,
                "{typed:?}"
            );
        }
    }

    #[test]
    fn token_budget_empty_is_none_and_below_1_is_refused() {
        for (typed, want) in [
            ("", Ok(None)),
            ("  ", Ok(None)),
            ("1", Ok(Some(1))),
            ("0", Err(BUDGET_IS_A_NUMBER.to_owned())),
            ("-5", Err(BUDGET_IS_A_NUMBER.to_owned())),
            ("lots", Err(BUDGET_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = edit(&research());
            set_text(&mut form, BUDGET, typed);
            assert_eq!(
                built(&form).map(|node| node.phase.token_budget),
                want,
                "{typed:?}"
            );
        }
    }

    #[test]
    fn verify_empty_is_none() {
        let mut form = edit(&research());
        set_text(&mut form, VERIFY, "   ");
        assert_eq!(built(&form).map(|node| node.phase.verify_command), Ok(None));
        set_text(&mut form, VERIFY, " cargo test ");
        assert_eq!(
            built(&form).map(|node| node.phase.verify_command),
            Ok(Some("cargo test".to_owned()))
        );
    }

    #[test]
    fn inputs_split_on_commas_trimmed_in_order() {
        let mut form = edit(&research());
        set_text(&mut form, INPUTS, " plan, ,review ,notes,plan,");
        assert_eq!(
            built(&form).map(|node| node.phase.input_kinds),
            Ok(vec![
                "plan".to_owned(),
                "review".to_owned(),
                "notes".to_owned(),
                "plan".to_owned(),
            ])
        );
        set_text(&mut form, INPUTS, "");
        assert_eq!(
            built(&form).map(|node| node.phase.input_kinds),
            Ok(Vec::new())
        );
    }

    #[test]
    fn output_empty_is_the_name() {
        let mut form = edit(&research());
        set_text(&mut form, NAME, " survey ");
        set_text(&mut form, OUTPUT, "  ");
        assert_eq!(
            built(&form).map(|node| node.phase.output_kind),
            Ok("survey".to_owned())
        );
    }

    #[test]
    fn template_cycles_through_the_scope_templates_and_keeps_an_unknown_current() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, TEMPLATE);
        assert_eq!(form.focused(), NodeRow::Choice);
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("names to choose");
            seen.push(form.template_name.clone());
        }
        assert_eq!(
            seen,
            ["verdict", "plan", "research"],
            "sorted, unique, wrapping"
        );
        form.cycle(false, &lookups).expect("names to choose");
        assert_eq!(form.template_name, "plan");

        // A name the scope lacks stays a choice while it is the current one.
        let mut node = research();
        node.phase.template_name = "legacy".to_owned();
        let mut form = edit(&node);
        focus(&mut form, TEMPLATE);
        assert!(
            text(&form.lines(&node, &lookups, ADD, 56, accent(), &Theme::default()))
                .contains("legacy")
        );
        form.cycle(false, &lookups).expect("names to choose");
        assert_eq!(
            form.template_name, "verdict",
            "legacy sorts first, back wraps"
        );
        let mut form = edit(&node);
        focus(&mut form, TEMPLATE);
        form.cycle(true, &lookups).expect("names to choose");
        assert_eq!(form.template_name, "plan");

        // Nothing to choose and nothing chosen.
        let mut node = research();
        node.phase.template_name = String::new();
        let empty = Lookups {
            templates: Some(Vec::new()),
            ..lookups.clone()
        };
        let mut form = edit(&node);
        focus(&mut form, TEMPLATE);
        assert_eq!(form.cycle(true, &empty), Err(NO_TEMPLATE.to_owned()));
        assert_eq!(
            form.cycle(true, &Lookups::default()),
            Err(NO_TEMPLATE.to_owned())
        );
        assert_eq!(form.build(&node, &empty), Err(NO_TEMPLATE.to_owned()));
        assert_eq!(
            form.build(&node, &Lookups::default()),
            Err(NO_TEMPLATE.to_owned())
        );
        // Names to choose, none chosen: choose one, not "add one".
        assert_eq!(built(&form), Err(TEMPLATE_REQUIRED.to_owned()));
    }

    #[test]
    fn version_resets_to_latest_when_the_template_changes() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, VERSION);
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("versions to choose");
            seen.push(form.template_version);
        }
        assert_eq!(
            seen,
            [Some(2), Some(1), None],
            "latest, then descending, wrapping"
        );
        form.cycle(false, &lookups).expect("versions to choose");
        assert_eq!(form.template_version, Some(1));

        cycle_at(&mut form, TEMPLATE, &lookups);
        assert_eq!(form.template_name, "verdict");
        assert_eq!(form.template_version, None, "a new name starts at latest");
    }

    #[test]
    fn persona_cycles_none_then_the_registry_and_shows_an_unknown_id() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, PERSONA);
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("a choice");
            seen.push(form.persona);
        }
        assert_eq!(
            seen,
            [
                Some(ids::PERSONA_REVIEWER),
                Some(ids::PERSONA_ARCHITECT),
                None
            ]
        );

        let unknown = PersonaId::new();
        let mut node = research();
        node.phase.persona_id = Some(unknown);
        let mut form = edit(&node);
        let shown = text(&form.lines(&node, &lookups, ADD, 80, accent(), &Theme::default()));
        assert!(shown.contains(&unknown.to_string()), "{shown}");
        focus(&mut form, PERSONA);
        form.cycle(true, &lookups).expect("a choice");
        assert_eq!(
            form.persona, None,
            "the unknown id sits last; forward wraps to none"
        );
    }

    #[test]
    fn gate_cycles_and_hard_gate_toggles() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, GATE);
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("a choice");
            seen.push(form.gate);
        }
        assert_eq!(seen, [Gate::OnFailure, Gate::Never, Gate::Always]);
        focus(&mut form, HARD_GATE);
        assert_eq!(form.focused(), NodeRow::Toggle);
        form.cycle(true, &lookups).expect("a toggle");
        assert!(form.gate_hard);
        form.cycle(false, &lookups).expect("a toggle");
        assert!(!form.gate_hard);
    }

    #[test]
    fn isolation_cycles_project_default_then_the_four() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, ISOLATION);
        let mut seen = Vec::new();
        for _ in 0..5 {
            form.cycle(true, &lookups).expect("a choice");
            seen.push(form.isolation);
        }
        assert_eq!(
            seen,
            [
                Some(Isolation::Worktree),
                Some(Isolation::Copy),
                Some(Isolation::SharedSerialized),
                Some(Isolation::Local),
                None,
            ]
        );
    }

    #[test]
    fn commands_cycles_the_three() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus(&mut form, COMMANDS);
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("a choice");
            seen.push(form.command_queue);
        }
        assert_eq!(
            seen,
            [
                CommandQueue::Always,
                CommandQueue::Off,
                CommandQueue::FanOutOnly
            ]
        );
    }

    #[test]
    fn candidates_add_remove_and_move_renumber_positions() {
        let lookups = lookups();
        let node = research();
        let mut form = edit(&node);
        assert_eq!(form.candidates.len(), 0);
        form.candidate_add(&lookups).expect("agents read");
        assert_eq!(form.focused(), NodeRow::Candidate(0));
        form.candidate_add(&lookups).expect("agents read");
        assert_eq!(
            form.focused(),
            NodeRow::Candidate(1),
            "focus moves to the new one"
        );
        form.cycle(false, &lookups).expect("pairs to choose"); // claude · opus → agy · gemini
        form.candidate_move(true).expect("a candidate row");
        assert_eq!(
            form.focused(),
            NodeRow::Candidate(0),
            "focus follows the move"
        );
        assert_eq!(
            built(&form).map(|node| node.agents),
            Ok(vec![
                candidate(&node, 0, ids::AGENT_AGY, "gemini"),
                candidate(&node, 1, ids::AGENT_CLAUDE, "opus"),
            ])
        );
        form.candidate_move(true).expect("the top stays put");
        assert_eq!(form.focused(), NodeRow::Candidate(0));
        form.candidate_remove().expect("a candidate row");
        assert_eq!(
            built(&form).map(|node| node.agents),
            Ok(vec![candidate(&node, 0, ids::AGENT_CLAUDE, "opus")])
        );
        form.candidate_remove().expect("a candidate row");
        assert_eq!(form.focused(), NodeRow::NoCandidates);
        assert_eq!(built(&form).map(|node| node.agents), Ok(Vec::new()));
    }

    #[test]
    fn a_candidate_cycles_through_every_agent_model_pair() {
        let lookups = lookups();
        let pairs = agent_pairs(lookups.agents.as_deref().expect("agents"));
        assert_eq!(
            pairs,
            [
                (ids::AGENT_CLAUDE, "opus".to_owned()),
                (ids::AGENT_CLAUDE, "sonnet".to_owned()),
                (ids::AGENT_AGY, "gemini".to_owned()),
            ]
        );
        let bare = summary(AgentId::new(), "bare", &[], None);
        assert_eq!(
            agent_pairs(core::slice::from_ref(&bare)),
            [(bare.agent.id, String::new())]
        );

        let mut form = edit(&research());
        form.candidate_add(&lookups).expect("agents read");
        let mut seen = Vec::new();
        for _ in 0..3 {
            form.cycle(true, &lookups).expect("pairs");
            seen.push(form.candidates[0].clone());
        }
        assert_eq!(seen, [pairs[1].clone(), pairs[2].clone(), pairs[0].clone()]);
        form.cycle(false, &lookups).expect("pairs");
        assert_eq!(form.candidates[0], pairs[2]);
    }

    #[test]
    fn candidates_need_the_agents_read_and_some_agent() {
        let unread = Lookups {
            agents: None,
            ..lookups()
        };
        let none = Lookups {
            agents: Some(Vec::new()),
            ..lookups()
        };
        let node = research_bound();
        let mut form = edit(&node);
        assert_eq!(form.candidate_add(&unread), Err(AGENTS_NOT_READ.to_owned()));
        assert_eq!(form.candidate_add(&none), Err(NO_AGENTS.to_owned()));
        focus(&mut form, FIXED_ROWS);
        assert_eq!(form.focused(), NodeRow::Candidate(0));
        assert_eq!(form.cycle(true, &unread), Err(AGENTS_NOT_READ.to_owned()));
        assert_eq!(form.cycle(true, &none), Err(NO_AGENTS.to_owned()));
        // The candidates the node had are kept as they are while the agents are unread.
        assert_eq!(
            form.build(&node, &lookups()).map(|built| built.agents),
            Ok(node.agents.clone())
        );

        focus(&mut form, NAME);
        assert_eq!(form.candidate_remove(), Err(NO_CANDIDATE.to_owned()));
        assert_eq!(form.candidate_move(false), Err(NO_CANDIDATE.to_owned()));
        let mut form = edit(&research());
        focus(&mut form, FIXED_ROWS);
        assert_eq!(form.focused(), NodeRow::NoCandidates);
        assert_eq!(form.candidate_remove(), Err(NO_CANDIDATE.to_owned()));
    }

    #[test]
    fn next_field_skips_the_read_only_rows() {
        use NodeRow::{Candidate, Choice, Text, Toggle};
        let mut form = edit(&research_bound());
        let mut seen = vec![form.focused()];
        for _ in 0..FIXED_ROWS + 2 {
            form.next_field();
            seen.push(form.focused());
        }
        assert_eq!(
            seen,
            [
                Text,
                Choice,
                Choice,
                Choice,
                Choice,
                Toggle,
                Text,
                Choice,
                Choice,
                Text,
                Text,
                Text,
                Text,
                Candidate(0),
                Candidate(1),
                Text,
            ],
            "after the last candidate comes the name again"
        );
        form.prev_field();
        assert_eq!(form.focused(), Candidate(1));

        let mut form = edit(&research());
        form.prev_field();
        assert_eq!(form.focused(), NodeRow::NoCandidates);
        form.next_field();
        assert_eq!(form.focused(), Text);
    }

    #[test]
    fn debug_never_prints_typed_text() {
        let mut form = edit(&research());
        set_text(&mut form, NAME, "hunter2");
        set_text(&mut form, VERIFY, "swordfish");
        let printed = format!("{form:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("swordfish"), "{printed}");
        assert!(printed.contains("NodeForm"), "{printed}");
    }

    /// A body of the `research` node and one after it.
    fn body(node: &NodeBody) -> GraphBody {
        GraphBody {
            graph: StepGraph {
                id: node.phase.graph_id,
                scope: GraphScope::Project(ProjectId::new()),
                name: "analysis".to_owned(),
                description: String::new(),
                is_override: false,
                revision: 1,
                created_at: now(),
                updated_at: now(),
            },
            nodes: vec![node.clone()],
            edges: Vec::new(),
        }
    }

    #[test]
    fn the_title_names_the_step() {
        let node = research();
        let graph = body(&node);
        assert_eq!(edit(&node).title(&graph), "step research");
        let after = NodeTarget::New {
            after: Some(node.phase.id),
        };
        assert_eq!(
            NodeForm::new(&node, after).title(&graph),
            "new step after research"
        );
        assert_eq!(
            NodeForm::new(&node, NodeTarget::New { after: None }).title(&graph),
            "new step"
        );
    }

    #[test]
    fn node_form_edit() {
        let node = research_bound();
        let form = edit(&node);
        let lines = form.lines(&node, &lookups(), ADD, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_edit", text(&lines));
    }

    #[test]
    fn node_form_new() {
        let mut node = research();
        node.phase.id = PhaseId::new();
        node.phase.name = String::new();
        node.phase.output_kind = String::new();
        node.phase.gate = Gate::Never;
        node.phase.retry_limit = 0;
        node.phase.input_kinds = Vec::new();
        let target = NodeTarget::New {
            after: Some(PhaseId::new()),
        };
        let form = NodeForm::new(&node, target);
        let lines = form.lines(&node, &lookups(), ADD, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_new", text(&lines));
    }

    #[test]
    fn node_form_divide() {
        let mut node = research_bound();
        node.phase.split = Split::Divide;
        node.phase.plan_scope = Some(PlanScope::Parts);
        node.phase.planner_persona_id = Some(ids::PERSONA_ARCHITECT);
        node.phase.planner_agent_id = Some(ids::AGENT_CLAUDE);
        node.phase.planner_model = Some("opus".to_owned());
        node.phase.integrate_persona_id = Some(ids::PERSONA_REVIEWER);
        node.phase.integrate_agent_id = Some(ids::AGENT_AGY);
        node.phase.integrate_model = None;
        node.phase.max_parts = Some(4);
        node.parts = vec![part(&node, 0, "ui"), part(&node, 1, "store")];
        let mut form = edit(&node);
        focus(&mut form, OUTPUT);
        let lines = form.lines(&node, &lookups(), ADD, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_divide", text(&lines));
    }
}
