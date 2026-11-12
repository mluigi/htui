//! MOD-141 D8, M2 D7-D10: the node form - every M1 node field, the ordered candidate agents,
//! then the deadline, the palette flag, the split and the split's own rows (compete: fan-out and
//! judge; divide: planner, integrate, part cap, plan scope and the default parts). Key-agnostic:
//! the editor maps its acts onto these methods.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use htui_core::model::{
    AgentId, AgentSummary, CommandQueue, Gate, GraphBody, Isolation, NodeBody, PersonaId,
    PhaseAgent, PhaseId, PhasePart, PlanScope, Split,
};
use ratatui::style::Style;
use ratatui::text::Line;

use super::part_form::{PartForm, PartTarget};
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
/// A deadline that is neither empty nor a whole number of at least 1 (M2 D10).
pub(crate) const DEADLINE_IS_A_NUMBER: &str =
    "`deadline` is a whole number of seconds, at least 1, or empty";
/// A compete step's fan-out that is not a whole number of at least 2 (M2 D8).
pub(crate) const FAN_OUT_IS_A_NUMBER: &str = "`fan-out` is a whole number, 2 or more";
/// A divide step's part cap that is neither empty nor a whole number of at least 1 (M2 D9).
pub(crate) const MAX_PARTS_IS_A_NUMBER: &str =
    "`max parts` is a whole number of at least 1, or empty";
/// A part edit with no part row focused (M2 D14).
pub(crate) const NO_PART: &str = "no part is selected";
/// A new part asked for on a step that does not divide (M2 D9).
pub(crate) const PARTS_ON_DIVIDE_ONLY: &str = "only a divide step has parts; choose divide first";
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

/// The placeholder row's text while the node has no candidate, naming `add`, the key the editor
/// binds to adding one (D12, R-TUI-10: the key comes from the key file, never this module), or
/// `none` alone while nothing is bound to it.
pub(crate) fn no_candidates(add: Option<&str>) -> String {
    add.map_or_else(|| "none".to_owned(), |key| format!("none ({key} adds one)"))
}

/// The parts placeholder's text while a divide step has none: `none ({add} adds one)`, or
/// `none` alone while nothing is bound to adding one (R-TUI-10).
pub(crate) fn no_parts(add: Option<&str>) -> String {
    no_candidates(add)
}

/// The split row of a compete step while the compete switch is off (M2 D7): shown, never edited.
const COMPETE_LOCKED: &str =
    "compete \u{b7} read-only while compete is off (Settings \u{203a} Prompt)";
/// The split row's help with the compete switch on.
const SPLIT_HELP_ON: &str = "none, divide into parts, or compete: several agents, one winner";
/// The split row's help with the compete switch off.
const SPLIT_HELP_OFF: &str =
    "none or divide into parts; compete is off in Settings \u{203a} Prompt";

/// The fixed rows before the candidates, in focus order: M1's 13, which M2 never moves (B3).
const FIXED_ROWS: usize = 13;
/// The name row's index, where a form opens.
const NAME: usize = 0;
/// M1's fixed rows in focus order; a row's index here is its index in [`NodeForm::rows`].
const FIXED: [Row; FIXED_ROWS] = [
    Row::Name,
    Row::Template,
    Row::Version,
    Row::Persona,
    Row::Gate,
    Row::HardGate,
    Row::Retry,
    Row::Isolation,
    Row::Commands,
    Row::Verify,
    Row::Budget,
    Row::Inputs,
    Row::Output,
];

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
    /// A toggle (hard gate, palette): it cycles by flipping.
    Toggle,
    /// Candidate `n` (0-based): it cycles over the agent and model pairs.
    Candidate(usize),
    /// The placeholder row shown while there is no candidate.
    NoCandidates,
    /// Default part `n` (0-based) of a divide step: the part form opens it.
    Part(usize),
    /// The placeholder row shown while a divide step has no part.
    NoParts,
}

/// The key labels the form's own words name (R-TUI-10); the editor resolves them from its stack.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FormKeys<'a> {
    /// `graph_editor.candidate_add`'s label: the placeholder rows.
    pub(crate) add: Option<&'a str>,
    /// `graph_editor.open_part`'s label: a part row's help, `None` while nothing is bound to it.
    pub(crate) open: Option<&'a str>,
}

/// Which list the focus is on, for the four list acts (M2 D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListFocus {
    /// The candidate agents: any row but a part row.
    Candidates,
    /// The divide step's default parts: a part row or their placeholder.
    Parts,
}

/// What [`NodeForm::open_part`] opens (M2 D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartOpen {
    /// A blank part, appended on keep.
    New,
    /// The focused part.
    Focused,
}

/// Every row the form can show, in the order it shows them (B3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Name,
    Template,
    Version,
    Persona,
    Gate,
    HardGate,
    Retry,
    Isolation,
    Commands,
    Verify,
    Budget,
    Inputs,
    Output,
    Candidate(usize),
    NoCandidates,
    Deadline,
    Palette,
    Split,
    FanOut,
    Judge,
    PlannerPersona,
    PlannerAgent,
    IntegratePersona,
    IntegrateAgent,
    MaxParts,
    PlanScope,
    Part(usize),
    NoParts,
}

/// A seat: an agent and its model, `None` when the pair's model is empty.
pub(super) type Seat = (AgentId, Option<String>);

/// The open node form (D8, M2 D7-D10).
pub(crate) struct NodeForm {
    /// The node it writes back to.
    target: NodeTarget,
    /// The node's id: what its parts belong to.
    phase_id: PhaseId,
    /// The compete switch (M2 D5) as the form opened: whether the split offers compete.
    compete: bool,
    /// A compete step opened with the switch off: its split, fan-out and judge are shown and
    /// kept byte for byte, never edited (M2 D7).
    locked: bool,
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
    /// `deadline_seconds`, as typed; empty follows the project's.
    deadline: TextField,
    /// `palette`.
    palette: bool,
    /// `split`.
    split: Split,
    /// `fan_out`, as typed; read on a compete step only.
    fan_out: TextField,
    /// `judge_agent_id` and `judge_model`; `None` is the project default.
    judge: Option<Seat>,
    /// `planner_persona_id`.
    planner_persona: Option<PersonaId>,
    /// `planner_agent_id` and `planner_model`.
    planner: Option<Seat>,
    /// `integrate_persona_id`.
    integrate_persona: Option<PersonaId>,
    /// `integrate_agent_id` and `integrate_model`.
    integrate: Option<Seat>,
    /// `max_parts`, as typed; empty for no cap.
    max_parts: TextField,
    /// `plan_scope`.
    plan_scope: Option<PlanScope>,
    /// The default parts in position order.
    parts: Vec<PhasePart>,
    /// The focused row: an index into [`rows`](Self::rows).
    focus: usize,
}

/// The target, the focus, the counts, the split and the lock, never what was typed (H-7).
impl core::fmt::Debug for NodeForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NodeForm")
            .field("target", &self.target)
            .field("focus", &self.focus)
            .field("candidates", &self.candidates.len())
            .field("parts", &self.parts.len())
            .field("split", &self.split)
            .field("locked", &self.locked)
            .finish_non_exhaustive()
    }
}

impl NodeForm {
    /// The form prefilled from `node` (D8), writing back to `target`. `compete` is the compete
    /// switch (M2 D5): off, the split offers none and divide only, and a compete `node` keeps its
    /// split, fan-out and judge read-only.
    pub(crate) fn new(node: &NodeBody, target: NodeTarget, compete: bool) -> Self {
        let phase = &node.phase;
        let seat = |agent: Option<AgentId>, model: &Option<String>| {
            agent.map(|agent| (agent, model.clone()))
        };
        let mut parts = node.parts.clone();
        parts.sort_by_key(|part| part.position);
        Self {
            target,
            phase_id: phase.id,
            compete,
            locked: !compete && phase.split == Split::Compete,
            name: TextField::with_text(&phase.name),
            retry_limit: TextField::with_text(&phase.retry_limit.to_string()),
            verify_command: TextField::with_text(phase.verify_command.as_deref().unwrap_or("")),
            token_budget: TextField::with_text(&number_text(phase.token_budget)),
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
            deadline: TextField::with_text(&number_text(phase.deadline_seconds)),
            palette: phase.palette,
            split: phase.split,
            fan_out: TextField::with_text(&phase.fan_out.to_string()),
            judge: seat(phase.judge_agent_id, &phase.judge_model),
            planner_persona: phase.planner_persona_id,
            planner: seat(phase.planner_agent_id, &phase.planner_model),
            integrate_persona: phase.integrate_persona_id,
            integrate: seat(phase.integrate_agent_id, &phase.integrate_model),
            max_parts: TextField::with_text(&number_text(phase.max_parts)),
            plan_scope: phase.plan_scope,
            parts,
            focus: NAME,
        }
    }

    /// The node the form writes back to.
    pub(crate) const fn target(&self) -> NodeTarget {
        self.target
    }

    /// Every row drawn, in order, each with whether it takes the focus (B3): M1's fixed rows,
    /// the candidates or their placeholder, the deadline, the palette and the split, then the
    /// split's own rows. A locked compete step's split, fan-out and judge are drawn read-only,
    /// and so is a plan scope stored on a step that does not divide (M2 D9).
    fn shown(&self) -> Vec<(Row, bool)> {
        let mut rows: Vec<(Row, bool)> = FIXED.iter().map(|row| (*row, true)).collect();
        if self.candidates.is_empty() {
            rows.push((Row::NoCandidates, true));
        }
        rows.extend((0..self.candidates.len()).map(|at| (Row::Candidate(at), true)));
        rows.extend([
            (Row::Deadline, true),
            (Row::Palette, true),
            (Row::Split, !self.locked),
        ]);
        match self.split {
            Split::Compete => {
                rows.extend([(Row::FanOut, !self.locked), (Row::Judge, !self.locked)]);
            }
            Split::Divide => {
                rows.extend(
                    [
                        Row::PlannerPersona,
                        Row::PlannerAgent,
                        Row::IntegratePersona,
                        Row::IntegrateAgent,
                        Row::MaxParts,
                        Row::PlanScope,
                    ]
                    .map(|row| (row, true)),
                );
                if self.parts.is_empty() {
                    rows.push((Row::NoParts, true));
                }
                rows.extend((0..self.parts.len()).map(|at| (Row::Part(at), true)));
            }
            Split::None => {}
        }
        if self.split != Split::Divide && self.plan_scope.is_some() {
            rows.push((Row::PlanScope, false));
        }
        rows
    }

    /// The rows that take the focus, in order; [`focus`](Self::focus) indexes this list.
    fn rows(&self) -> Vec<Row> {
        self.shown()
            .into_iter()
            .filter_map(|(row, focusable)| focusable.then_some(row))
            .collect()
    }

    /// The focused row; the last one should the focus ever sit past the end.
    fn current(&self) -> Row {
        let rows = self.rows();
        rows.get(self.focus)
            .or_else(|| rows.last())
            .copied()
            .unwrap_or(Row::Name)
    }

    /// The focused row's kind.
    pub(crate) fn focused(&self) -> NodeRow {
        match self.current() {
            Row::Name
            | Row::Retry
            | Row::Verify
            | Row::Budget
            | Row::Inputs
            | Row::Output
            | Row::Deadline
            | Row::FanOut
            | Row::MaxParts => NodeRow::Text,
            Row::HardGate | Row::Palette => NodeRow::Toggle,
            Row::Candidate(at) => NodeRow::Candidate(at),
            Row::NoCandidates => NodeRow::NoCandidates,
            Row::Part(at) => NodeRow::Part(at),
            Row::NoParts => NodeRow::NoParts,
            Row::Template
            | Row::Version
            | Row::Persona
            | Row::Gate
            | Row::Isolation
            | Row::Commands
            | Row::Split
            | Row::Judge
            | Row::PlannerPersona
            | Row::PlannerAgent
            | Row::IntegratePersona
            | Row::IntegrateAgent
            | Row::PlanScope => NodeRow::Choice,
        }
    }

    /// Focus on the next row, wrapping. Read-only rows are never focused.
    pub(crate) fn next_field(&mut self) {
        self.focus = (self.focus + 1) % self.rows().len();
    }

    /// Focus on the previous row, wrapping.
    pub(crate) fn prev_field(&mut self) {
        let rows = self.rows().len();
        self.focus = (self.focus + rows - 1) % rows;
    }

    /// The focused text field, if a text row has the focus.
    fn text_mut(&mut self) -> Option<&mut TextField> {
        match self.current() {
            Row::Name => Some(&mut self.name),
            Row::Retry => Some(&mut self.retry_limit),
            Row::Verify => Some(&mut self.verify_command),
            Row::Budget => Some(&mut self.token_budget),
            Row::Inputs => Some(&mut self.input_kinds),
            Row::Output => Some(&mut self.output_kind),
            Row::Deadline => Some(&mut self.deadline),
            Row::FanOut => Some(&mut self.fan_out),
            Row::MaxParts => Some(&mut self.max_parts),
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
    /// resets the version to latest; a split change runs the M2 D7 transitions
    /// ([`set_split`](Self::set_split)). A text row and the placeholder row do nothing.
    ///
    /// # Errors
    /// [`NO_TEMPLATE`] on the template row with no name to choose; [`AGENTS_NOT_READ`] or
    /// [`NO_AGENTS`] on a candidate row; [`AGENTS_NOT_READ`] on the judge, planner agent and
    /// integrate agent rows.
    pub(crate) fn cycle(&mut self, forward: bool, lookups: &Lookups) -> Result<(), String> {
        match self.current() {
            Row::Name
            | Row::Retry
            | Row::Verify
            | Row::Budget
            | Row::Inputs
            | Row::Output
            | Row::Deadline
            | Row::FanOut
            | Row::MaxParts
            | Row::NoCandidates
            | Row::Part(_)
            | Row::NoParts => {}
            Row::HardGate => self.gate_hard = !self.gate_hard,
            Row::Palette => self.palette = !self.palette,
            Row::Candidate(at) => {
                let pairs = pairs_of(lookups)?;
                if let Some(candidate) = self.candidates.get_mut(at) {
                    *candidate = step(&pairs, candidate, forward);
                }
            }
            Row::Template => {
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
            Row::Version => {
                self.template_version =
                    step(&self.versions(lookups), &self.template_version, forward);
            }
            Row::Persona => {
                self.persona = step(
                    &persona_options(lookups, self.persona),
                    &self.persona,
                    forward,
                );
            }
            Row::Gate => self.gate = step(Gate::ALL, &self.gate, forward),
            Row::Isolation => {
                let options: Vec<Option<Isolation>> = core::iter::once(None)
                    .chain(Isolation::ALL.iter().copied().map(Some))
                    .collect();
                self.isolation = step(&options, &self.isolation, forward);
            }
            Row::Commands => {
                self.command_queue = step(CommandQueue::ALL, &self.command_queue, forward);
            }
            Row::Split => {
                let options: &[Split] = if self.compete {
                    &[Split::None, Split::Divide, Split::Compete]
                } else {
                    &[Split::None, Split::Divide]
                };
                let split = step(options, &self.split, forward);
                self.set_split(split);
            }
            Row::Judge => {
                self.judge = step(&seat_options(lookups, &self.judge)?, &self.judge, forward);
            }
            Row::PlannerAgent => {
                self.planner = step(
                    &seat_options(lookups, &self.planner)?,
                    &self.planner,
                    forward,
                );
            }
            Row::IntegrateAgent => {
                self.integrate = step(
                    &seat_options(lookups, &self.integrate)?,
                    &self.integrate,
                    forward,
                );
            }
            Row::PlannerPersona => {
                self.planner_persona = step(
                    &persona_options(lookups, self.planner_persona),
                    &self.planner_persona,
                    forward,
                );
            }
            Row::IntegratePersona => {
                self.integrate_persona = step(
                    &persona_options(lookups, self.integrate_persona),
                    &self.integrate_persona,
                    forward,
                );
            }
            Row::PlanScope => {
                let options = [
                    None,
                    Some(PlanScope::Parts),
                    Some(PlanScope::Successors),
                    Some(PlanScope::Graph),
                ];
                self.plan_scope = step(&options, &self.plan_scope, forward);
            }
        }
        Ok(())
    }

    /// M2 D7, moving the split from what it is to `split`: leaving divide clears the divide
    /// columns, the parts and the plan scope; entering compete sets the fan-out to 2; leaving
    /// compete sets it to 1 and clears the judge; entering divide clears the judge. Only rows
    /// after the split change, so the focus stays on it.
    fn set_split(&mut self, split: Split) {
        let from = self.split;
        if from == split {
            return;
        }
        if from == Split::Divide {
            self.planner_persona = None;
            self.planner = None;
            self.integrate_persona = None;
            self.integrate = None;
            self.max_parts = TextField::new();
            self.parts.clear();
            self.plan_scope = None;
        }
        if split == Split::Compete {
            self.fan_out = TextField::with_text("2");
        }
        if from == Split::Compete {
            self.fan_out = TextField::with_text("1");
            self.judge = None;
        }
        if split == Split::Divide {
            self.judge = None;
        }
        self.split = split;
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

    /// The row index of candidate `at` (or of the placeholder when there is none).
    const fn candidate_row(at: usize) -> usize {
        FIXED_ROWS + at
    }

    /// Appends the first (agent, model) pair; focus moves to it.
    ///
    /// # Errors
    /// [`AGENTS_NOT_READ`] or [`NO_AGENTS`].
    pub(crate) fn candidate_add(&mut self, lookups: &Lookups) -> Result<(), String> {
        let first = pairs_of(lookups)?.swap_remove(0);
        self.candidates.push(first);
        self.focus = Self::candidate_row(self.candidates.len() - 1);
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
        self.focus = Self::candidate_row(at.min(self.candidates.len().saturating_sub(1)));
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
            self.focus = Self::candidate_row(to);
        }
        Ok(())
    }

    /// Which list the four list acts work on (M2 D14): the parts on a part row or their
    /// placeholder, else the candidates.
    pub(crate) fn list_focus(&self) -> ListFocus {
        match self.current() {
            Row::Part(_) | Row::NoParts => ListFocus::Parts,
            _ => ListFocus::Candidates,
        }
    }

    /// Removes the focused list's focused entry: a part, else a candidate.
    ///
    /// # Errors
    /// [`NO_PART`] on the parts placeholder; [`NO_CANDIDATE`] off both lists.
    pub(crate) fn list_remove(&mut self) -> Result<(), String> {
        match self.list_focus() {
            ListFocus::Parts => self.part_remove(),
            ListFocus::Candidates => self.candidate_remove(),
        }
    }

    /// Moves the focused list's focused entry one place up or down: a part, else a candidate.
    ///
    /// # Errors
    /// [`NO_PART`] on the parts placeholder; [`NO_CANDIDATE`] off both lists.
    pub(crate) fn list_move(&mut self, up: bool) -> Result<(), String> {
        match self.list_focus() {
            ListFocus::Parts => self.part_move(up),
            ListFocus::Candidates => self.candidate_move(up),
        }
    }

    /// The focused part's index.
    fn part_at(&self) -> Result<usize, String> {
        match self.current() {
            Row::Part(at) => Ok(at),
            _ => Err(NO_PART.to_owned()),
        }
    }

    /// Focus on part `at`, or on the placeholder when there is no part; unchanged when the step
    /// shows no part rows.
    fn focus_part(&mut self, at: usize) {
        let row = if self.parts.is_empty() {
            Row::NoParts
        } else {
            Row::Part(at.min(self.parts.len() - 1))
        };
        if let Some(focus) = self.rows().iter().position(|each| *each == row) {
            self.focus = focus;
        }
    }

    /// Removes the focused part; focus stays on the row now there, or the last.
    ///
    /// # Errors
    /// [`NO_PART`] off a part row.
    pub(crate) fn part_remove(&mut self) -> Result<(), String> {
        let at = self.part_at()?;
        self.parts.remove(at);
        self.focus_part(at);
        Ok(())
    }

    /// Moves the focused part one place up or down; focus follows it. At either end it stays.
    ///
    /// # Errors
    /// [`NO_PART`] off a part row.
    pub(crate) fn part_move(&mut self, up: bool) -> Result<(), String> {
        let at = self.part_at()?;
        let to = if up {
            at.checked_sub(1)
        } else {
            Some(at + 1).filter(|to| *to < self.parts.len())
        };
        if let Some(to) = to {
            self.parts.swap(at, to);
            self.focus_part(to);
        }
        Ok(())
    }

    /// The part form (M2 D9): [`PartOpen::New`] a blank part, [`PartOpen::Focused`] the focused
    /// one ([`PartTarget::Edit`]). It offers the step form's own candidates as they are now,
    /// since admission refuses a part agent the step does not list.
    ///
    /// # Errors
    /// [`PARTS_ON_DIVIDE_ONLY`] for a new part off divide; [`NO_PART`] for the focused part off a
    /// part row.
    pub(crate) fn open_part(&self, open: PartOpen) -> Result<PartForm, String> {
        let candidates = self.candidates.clone();
        match open {
            PartOpen::New if self.split == Split::Divide => {
                Ok(PartForm::new(None, PartTarget::New, candidates))
            }
            PartOpen::New => Err(PARTS_ON_DIVIDE_ONLY.to_owned()),
            PartOpen::Focused => {
                let at = self.part_at()?;
                Ok(PartForm::new(
                    self.parts.get(at),
                    PartTarget::Edit(at),
                    candidates,
                ))
            }
        }
    }

    /// Keeps the part `form` built: [`PartTarget::New`] appends it, [`PartTarget::Edit`]
    /// replaces that part. Positions are renumbered `0..n` and the focus moves onto its row.
    ///
    /// # Errors
    /// What [`PartForm::build`] refuses (the key, against every other part's); [`NO_PART`] for an
    /// edit of a part the step no longer has.
    pub(crate) fn put_part(&mut self, form: &PartForm) -> Result<(), String> {
        let edited = match form.target() {
            PartTarget::New => None,
            PartTarget::Edit(at) if at < self.parts.len() => Some(at),
            PartTarget::Edit(_) => return Err(NO_PART.to_owned()),
        };
        let others: Vec<PhasePart> = self
            .parts
            .iter()
            .enumerate()
            .filter(|(at, _)| Some(*at) != edited)
            .map(|(_, part)| part.clone())
            .collect();
        let part = form.build(self.phase_id, &others)?;
        let at = match edited {
            Some(at) => {
                self.parts[at] = part;
                at
            }
            None => {
                self.parts.push(part);
                self.parts.len() - 1
            }
        };
        for (at, part) in self.parts.iter_mut().enumerate() {
            part.phase_id = self.phase_id;
            part.position = position(at);
        }
        self.focus_part(at);
        Ok(())
    }

    /// Parses and assembles the node. `base` supplies every column the form does not edit (id,
    /// graph id, position, `updated_at`), and a locked compete step's split, fan-out and judge,
    /// byte for byte (M2 D7). `lookups` tells an unchosen template with names to choose
    /// ([`TEMPLATE_REQUIRED`]) from one with none ([`NO_TEMPLATE`]).
    ///
    /// Off compete the fan-out is 1 and there is no judge; off divide there are no divide
    /// columns and no parts. The plan scope is written as the form holds it.
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
        let token_budget = at_least(text_of(&self.token_budget), 1, BUDGET_IS_A_NUMBER)?;
        let deadline_seconds = at_least(text_of(&self.deadline), 1, DEADLINE_IS_A_NUMBER)?;
        let compete = self.split == Split::Compete && !self.locked;
        let fan_out = if compete {
            match text_of(&self.fan_out).parse::<i32>() {
                Ok(fan_out) if fan_out >= 2 => fan_out,
                _ => return Err(FAN_OUT_IS_A_NUMBER.to_owned()),
            }
        } else {
            1
        };
        let divide = self.split == Split::Divide;
        let max_parts = if divide {
            at_least(text_of(&self.max_parts), 1, MAX_PARTS_IS_A_NUMBER)?
        } else {
            None
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
        phase.input_kinds = comma_list(text_of(&self.input_kinds));
        phase.output_kind = if output.is_empty() { name } else { output }.to_owned();
        phase.deadline_seconds = deadline_seconds;
        phase.palette = self.palette;
        if !self.locked {
            let judge = self.judge.clone().filter(|_| compete);
            phase.split = self.split;
            phase.fan_out = fan_out;
            phase.judge_agent_id = judge.as_ref().map(|(agent, _)| *agent);
            phase.judge_model = judge.and_then(|(_, model)| model);
        }
        let planner = self.planner.clone().filter(|_| divide);
        let integrate = self.integrate.clone().filter(|_| divide);
        phase.planner_persona_id = self.planner_persona.filter(|_| divide);
        phase.planner_agent_id = planner.as_ref().map(|(agent, _)| *agent);
        phase.planner_model = planner.and_then(|(_, model)| model);
        phase.integrate_persona_id = self.integrate_persona.filter(|_| divide);
        phase.integrate_agent_id = integrate.as_ref().map(|(agent, _)| *agent);
        phase.integrate_model = integrate.and_then(|(_, model)| model);
        phase.max_parts = max_parts;
        phase.plan_scope = self.plan_scope;
        node.agents = self
            .candidates
            .iter()
            .enumerate()
            .map(|(at, (agent_id, model))| PhaseAgent {
                phase_id: base.phase.id,
                position: position(at),
                agent_id: *agent_id,
                model: model.clone(),
            })
            .collect();
        node.parts = if divide {
            self.parts
                .iter()
                .enumerate()
                .map(|(at, part)| PhasePart {
                    phase_id: base.phase.id,
                    position: position(at),
                    ..part.clone()
                })
                .collect()
        } else {
            Vec::new()
        };
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

    /// The form's rows `width` cells wide, the focused one being edited; read-only rows dim.
    /// `base` is the node the form opened on: a locked compete step's split rows show its values,
    /// the ones [`build`](Self::build) keeps. `keys` names the keys the placeholder rows cite.
    pub(crate) fn lines(
        &self,
        base: &NodeBody,
        lookups: &Lookups,
        keys: FormKeys<'_>,
        width: u16,
        accent: Style,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let shown = self.shown();
        let views: Vec<View<'_>> = shown
            .iter()
            .map(|(row, focusable)| self.view(*row, *focusable, base, lookups, keys))
            .collect();
        let mut form = Form::new().editing(true);
        for view in &views {
            form = form.field(view.field(theme));
        }
        let current = self.current();
        let focus = shown
            .iter()
            .position(|(row, _)| *row == current)
            .unwrap_or(0);
        form.focus(focus).lines(width, accent, theme)
    }

    /// How `row` is drawn; `focusable` is `false` for a read-only row.
    fn view(
        &self,
        row: Row,
        focusable: bool,
        base: &NodeBody,
        lookups: &Lookups,
        keys: FormKeys<'_>,
    ) -> View<'_> {
        match row {
            Row::Name => View::text("name", &self.name),
            Row::Template => {
                let template = if self.template_name.is_empty() {
                    "none".to_owned()
                } else {
                    self.template_name.clone()
                };
                View::choice("template", template).help(if lookups.templates.is_none() {
                    "templates not read yet; the current name is kept"
                } else {
                    "the prompt template this step runs"
                })
            }
            Row::Version => View::choice(
                "version",
                self.template_version
                    .map_or_else(|| "latest".to_owned(), |version| format!("v{version}")),
            )
            .help("latest follows every new version"),
            Row::Persona => View::choice("persona", persona_word(self.persona, lookups)),
            Row::Gate => View::choice("gate", label::gate(self.gate)),
            Row::HardGate => View::toggle("hard gate", self.gate_hard),
            Row::Retry => View::text("retry limit", &self.retry_limit),
            Row::Isolation => View::choice(
                "isolation",
                self.isolation.map_or("project default", label::isolation),
            ),
            Row::Commands => View::choice("commands", label::command_queue(self.command_queue)),
            Row::Verify => View::text("verify", &self.verify_command).help("empty for none"),
            Row::Budget => View::text("token budget", &self.token_budget)
                .help("empty for the project's budget"),
            Row::Inputs => {
                View::text("inputs", &self.input_kinds).help("document kinds, comma separated")
            }
            Row::Output => View::text("output", &self.output_kind).help("empty for the name"),
            Row::Candidate(at) => {
                let (agent, model) = self.candidates.get(at).cloned().unwrap_or_default();
                View::choice(
                    format!("agent {}", at + 1),
                    pair_word(agent, &model, lookups),
                )
                .help("agent and model, tried in this order")
            }
            Row::NoCandidates => View::read_only("agents", no_candidates(keys.add)),
            Row::Deadline => View::text("deadline", &self.deadline)
                .help("seconds; empty for the project's deadline"),
            Row::Palette => {
                View::toggle("palette", self.palette).help("a planner places it; it has no links")
            }
            Row::Split if focusable => {
                View::choice("split", split_word(self.split)).help(if self.compete {
                    SPLIT_HELP_ON
                } else {
                    SPLIT_HELP_OFF
                })
            }
            Row::Split => View::read_only("split", COMPETE_LOCKED).dim(),
            Row::FanOut if focusable => {
                View::text("fan-out", &self.fan_out).help("agents that compete, 2 or more")
            }
            Row::FanOut => View::read_only("fan-out", base.phase.fan_out.to_string()).dim(),
            Row::Judge if focusable => {
                View::choice("judge", seat_word(&self.judge, "project default", lookups))
                    .help("picks the winner; project default asks the project's judge, else you")
            }
            Row::Judge => {
                let phase = &base.phase;
                let judge = phase
                    .judge_agent_id
                    .map(|agent| (agent, phase.judge_model.clone()));
                View::read_only("judge", seat_word(&judge, "project default", lookups)).dim()
            }
            Row::PlannerPersona => {
                View::choice("planner", persona_word(self.planner_persona, lookups))
                    .help("the persona that plans the parts")
            }
            Row::PlannerAgent => {
                View::choice("planner agent", seat_word(&self.planner, "none", lookups))
                    .help("an agent and model, or none")
            }
            Row::IntegratePersona => {
                View::choice("integrate", persona_word(self.integrate_persona, lookups))
                    .help("the persona that merges the parts")
            }
            Row::IntegrateAgent => View::choice(
                "integrate agent",
                seat_word(&self.integrate, "none", lookups),
            )
            .help("an agent and model, or none"),
            Row::MaxParts => {
                View::text("max parts", &self.max_parts).help("empty for no cap, else at least 1")
            }
            Row::PlanScope if focusable => {
                View::choice("plan scope", plan_scope_word(self.plan_scope))
                    .help("what the planner may plan")
            }
            Row::PlanScope => View::read_only("plan scope", plan_scope_word(self.plan_scope)).dim(),
            Row::Part(at) => {
                let words = self.parts.get(at).map_or_else(String::new, |part| {
                    if part.title.is_empty() {
                        part.key.clone()
                    } else {
                        format!("{} \u{b7} {}", part.key, part.title)
                    }
                });
                let view = View::read_only(format!("part {}", at + 1), words);
                match keys.open {
                    Some(open) => view.help(format!("{open} edits it")),
                    None => view,
                }
            }
            Row::NoParts => View::read_only("parts", no_parts(keys.add)),
        }
    }
}

/// One drawn row of the node form: label, value, help and whether it is read-only (dim).
#[derive(Debug)]
struct View<'f> {
    /// The label column's words.
    label: Cow<'static, str>,
    /// The value column.
    value: Shown<'f>,
    /// Shown under the row while it has the focus.
    help: Option<Cow<'static, str>>,
    /// A read-only row: `theme.dim`.
    dim: bool,
}

/// A [`View`]'s value, owned where the form computes it.
#[derive(Debug)]
enum Shown<'f> {
    /// A text field of the form.
    Text(&'f TextField),
    /// `● on` / `○ off`.
    Toggle(bool),
    /// `‹ word ›`.
    Choice(String),
    /// Plain words.
    ReadOnly(String),
}

impl<'f> View<'f> {
    /// A row with no help, not dim.
    fn new(label: impl Into<Cow<'static, str>>, value: Shown<'f>) -> Self {
        Self {
            label: label.into(),
            value,
            help: None,
            dim: false,
        }
    }

    /// A text row.
    fn text(label: &'static str, field: &'f TextField) -> Self {
        Self::new(label, Shown::Text(field))
    }

    /// A toggle row.
    fn toggle(label: &'static str, on: bool) -> Self {
        Self::new(label, Shown::Toggle(on))
    }

    /// A choice row showing `word`.
    fn choice(label: impl Into<Cow<'static, str>>, word: impl Into<String>) -> Self {
        Self::new(label, Shown::Choice(word.into()))
    }

    /// A row of plain words.
    fn read_only(label: impl Into<Cow<'static, str>>, words: impl Into<String>) -> Self {
        Self::new(label, Shown::ReadOnly(words.into()))
    }

    /// The help shown under the row while it has the focus.
    fn help(mut self, help: impl Into<Cow<'static, str>>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Drawn `theme.dim`: a read-only row.
    const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// The row as a kit form field.
    fn field(&self, theme: &Theme) -> Field<'_> {
        let value = match &self.value {
            Shown::Text(field) => Value::Text(field),
            Shown::Toggle(on) => Value::Toggle(*on),
            Shown::Choice(word) => Value::Choice(word),
            Shown::ReadOnly(words) => Value::ReadOnly(Cow::Borrowed(words)),
        };
        let field = Field::new(&self.label, value);
        let field = match &self.help {
            Some(help) => field.help(help.as_ref()),
            None => field,
        };
        if self.dim {
            field.style(theme.dim)
        } else {
            field
        }
    }
}

/// A split in words.
const fn split_word(split: Split) -> &'static str {
    match split {
        Split::None => "none",
        Split::Divide => "divide",
        Split::Compete => "compete",
    }
}

/// What a planner node may plan, in words; `none` for none.
const fn plan_scope_word(scope: Option<PlanScope>) -> &'static str {
    match scope {
        None => "none",
        Some(PlanScope::Parts) => "its parts",
        Some(PlanScope::Successors) => "its successors",
        Some(PlanScope::Graph) => "the whole graph",
    }
}

/// A seat in words: `claude · opus`, the agent alone with no model, `none` for none.
fn seat_word(seat: &Option<Seat>, none: &str, lookups: &Lookups) -> String {
    seat.as_ref().map_or_else(
        || none.to_owned(),
        |(agent, model)| pair_word(*agent, model.as_deref().unwrap_or(""), lookups),
    )
}

/// The seats to choose from: none, every (agent, model) pair (an empty model is `None`), then
/// `current` when the pairs lack it.
///
/// # Errors
/// [`AGENTS_NOT_READ`] while the agents' read has not answered.
fn seat_options(lookups: &Lookups, current: &Option<Seat>) -> Result<Vec<Option<Seat>>, String> {
    let agents = lookups
        .agents
        .as_deref()
        .ok_or_else(|| AGENTS_NOT_READ.to_owned())?;
    let mut options: Vec<Option<Seat>> = core::iter::once(None)
        .chain(
            agent_pairs(agents)
                .into_iter()
                .map(|(agent, model)| Some((agent, (!model.is_empty()).then_some(model)))),
        )
        .collect();
    if !options.contains(current) {
        options.push(current.clone());
    }
    Ok(options)
}

/// An optional whole number as a text row holds it: empty for `None`.
fn number_text(number: Option<i32>) -> String {
    number.map_or_else(String::new, |number| number.to_string())
}

/// `text` as an optional whole number of at least `min`: empty is `None`; anything else that is
/// not such a number is `refusal`.
fn at_least(text: &str, min: i32, refusal: &str) -> Result<Option<i32>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<i32>() {
        Ok(number) if number >= min => Ok(Some(number)),
        _ => Err(refusal.to_owned()),
    }
}

/// A list index as a stored `position`.
fn position(at: usize) -> i32 {
    i32::try_from(at).unwrap_or(i32::MAX)
}

/// What a text field holds, trimmed.
pub(super) fn text_of(field: &TextField) -> &str {
    field.text().unwrap_or_default().trim()
}

/// A comma-separated row as `kinds.rs` reads `input_kinds`: split on `,`, each part trimmed,
/// empty parts dropped, order kept, no de-duplication.
pub(super) fn comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One step from `current` through `options`, wrapping; from a value the options lack, forward
/// goes to the first and back to the last.
pub(super) fn step<T: Clone + PartialEq>(options: &[T], current: &T, forward: bool) -> T {
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
pub(super) fn pair_word(agent: AgentId, model: &str, lookups: &Lookups) -> String {
    let agent = agent_word(agent, lookups);
    if model.is_empty() {
        agent
    } else {
        format!("{agent} \u{b7} {model}")
    }
}

/// A persona by name, its id when the registry lacks it, `none` for none.
pub(super) fn persona_word(persona: Option<PersonaId>, lookups: &Lookups) -> String {
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

/// The personas to choose from: none, the registry in its order, then `current` when the
/// registry lacks it.
pub(super) fn persona_options(
    lookups: &Lookups,
    current: Option<PersonaId>,
) -> Vec<Option<PersonaId>> {
    let mut personas: Vec<Option<PersonaId>> = core::iter::once(None)
        .chain(lookups.personas.iter().map(|persona| Some(persona.id)))
        .collect();
    if !personas.contains(&current) {
        personas.push(current);
    }
    personas
}

#[cfg(test)]
pub(super) mod tests {
    use chrono::{DateTime, Utc};
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        GraphScope, ProjectId, PromptTemplate, PromptTemplateId, StepGraph, StepGraphId, UserId,
    };
    use htui_core::seed::{KINDS, phase_row};
    use ratatui::style::Color;

    use super::*;
    use crate::catalogue::PersonaSummary;

    // M1's fixed rows by index (B3): unchanged, so M1's drivers reach the same rows.
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

    pub(crate) fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// The candidate-add and part-open keys' labels as the compiled keys write them.
    const KEYS: FormKeys<'static> = FormKeys {
        add: Some("Alt+n"),
        open: Some("Alt+e"),
    };

    pub(crate) fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    /// The lines as text, trailing blanks dropped.
    pub(crate) fn text(lines: &[Line<'_>]) -> String {
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
    pub(crate) fn lookups() -> Lookups {
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

    fn part(node: &NodeBody, position: i32, key: &str, title: &str) -> PhasePart {
        PhasePart {
            phase_id: node.phase.id,
            position,
            key: key.to_owned(),
            title: title.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    /// `research_bound` as a store-valid divide step: planner `architect` on `claude · opus`,
    /// integrate `reviewer` on `agy` (no model), at most 4 parts, plan scope parts, the parts
    /// `ui · UI` and `store · Store`, a 600 s deadline.
    fn research_divide() -> NodeBody {
        let mut node = research_bound();
        let phase = &mut node.phase;
        phase.split = Split::Divide;
        phase.deadline_seconds = Some(600);
        phase.plan_scope = Some(PlanScope::Parts);
        phase.planner_persona_id = Some(ids::PERSONA_ARCHITECT);
        phase.planner_agent_id = Some(ids::AGENT_CLAUDE);
        phase.planner_model = Some("opus".to_owned());
        phase.integrate_persona_id = Some(ids::PERSONA_REVIEWER);
        phase.integrate_agent_id = Some(ids::AGENT_AGY);
        phase.integrate_model = None;
        phase.max_parts = Some(4);
        node.parts = vec![part(&node, 0, "ui", "UI"), part(&node, 1, "store", "Store")];
        node
    }

    /// `research_bound` as a compete step: fan-out 3, judged by `judge`.
    fn research_compete(judge: (AgentId, &str)) -> NodeBody {
        let mut node = research_bound();
        node.phase.split = Split::Compete;
        node.phase.fan_out = 3;
        node.phase.judge_agent_id = Some(judge.0);
        node.phase.judge_model = Some(judge.1.to_owned());
        node
    }

    /// The form on `node` with the compete switch `compete`.
    fn open(node: &NodeBody, compete: bool) -> NodeForm {
        NodeForm::new(node, NodeTarget::Edit(node.phase.id), compete)
    }

    /// The form on `node` with the compete switch off.
    fn edit(node: &NodeBody) -> NodeForm {
        open(node, false)
    }

    fn focus(form: &mut NodeForm, row: usize) {
        form.focus = row;
    }

    /// Focuses `row`, which must take the focus.
    fn focus_on(form: &mut NodeForm, row: Row) {
        form.focus = form
            .rows()
            .iter()
            .position(|each| *each == row)
            .unwrap_or_else(|| panic!("{row:?} takes the focus"));
    }

    /// Replaces what the focused text row holds by typing `text` into it.
    fn retype(form: &mut NodeForm, text: &str) {
        for _ in 0..64 {
            let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
        for c in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
    }

    /// Replaces what text row `row` holds by typing `text` into it.
    fn set_text(form: &mut NodeForm, row: usize, text: &str) {
        focus(form, row);
        retype(form, text);
    }

    /// Replaces what text row `row` holds by typing `text` into it.
    fn set_row_text(form: &mut NodeForm, row: Row, text: &str) {
        focus_on(form, row);
        retype(form, text);
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

        // M2: the deadline, the palette, the split and the split's columns prefill too.
        let mut node = research_divide();
        node.phase.palette = true;
        let form = edit(&node);
        assert_eq!(form.deadline.text(), Some("600"));
        assert!(form.palette);
        assert_eq!(form.split, Split::Divide);
        assert_eq!(form.fan_out.text(), Some("1"));
        assert_eq!(form.judge, None);
        assert_eq!(form.planner_persona, Some(ids::PERSONA_ARCHITECT));
        assert_eq!(
            form.planner,
            Some((ids::AGENT_CLAUDE, Some("opus".to_owned())))
        );
        assert_eq!(form.integrate_persona, Some(ids::PERSONA_REVIEWER));
        assert_eq!(form.integrate, Some((ids::AGENT_AGY, None)));
        assert_eq!(form.max_parts.text(), Some("4"));
        assert_eq!(form.plan_scope, Some(PlanScope::Parts));
        assert_eq!(form.parts, node.parts);
        let compete = research_compete((ids::AGENT_AGY, "gemini"));
        let form = open(&compete, true);
        assert_eq!(form.fan_out.text(), Some("3"));
        assert_eq!(
            form.judge,
            Some((ids::AGENT_AGY, Some("gemini".to_owned())))
        );
        assert_eq!(form.deadline.text(), Some(""), "no deadline: empty");

        // Untouched, a divide step with parts and a compete step with a judge build back as
        // they are, the switch on or off.
        for node in [research_divide(), compete] {
            for switch in [false, true] {
                assert_eq!(
                    open(&node, switch).build(&node, &lookups()),
                    Ok(node.clone()),
                    "{:?}, switch {switch}",
                    node.phase.split
                );
            }
        }
    }

    #[test]
    fn the_parts_prefill_in_position_order() {
        let mut node = research_divide();
        node.parts.reverse();
        let form = edit(&node);
        let keys: Vec<&str> = form.parts.iter().map(|part| part.key.as_str()).collect();
        assert_eq!(keys, ["ui", "store"]);
    }

    #[test]
    fn build_writes_every_m1_field_and_keeps_the_m2_fields_it_opened_with() {
        let lookups = lookups();
        let mut base = research();
        base.phase.position = 4;
        base.phase.split = Split::Divide;
        base.phase.plan_scope = Some(PlanScope::Parts);
        base.phase.planner_persona_id = Some(ids::PERSONA_ARCHITECT);
        base.phase.planner_agent_id = Some(ids::AGENT_CLAUDE);
        base.phase.planner_model = Some("opus".to_owned());
        base.phase.integrate_persona_id = Some(ids::PERSONA_REVIEWER);
        base.phase.integrate_agent_id = Some(ids::AGENT_AGY);
        base.phase.integrate_model = Some("gemini".to_owned());
        base.phase.max_parts = Some(3);
        base.phase.deadline_seconds = Some(600);
        base.parts = vec![part(&base, 0, "ui", "UI")];

        let mut form = edit(&base);
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
            text(&form.lines(&node, &lookups, KEYS, 56, accent(), &Theme::default()))
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
        let shown = text(&form.lines(&node, &lookups, KEYS, 80, accent(), &Theme::default()));
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
        for _ in 0..FIXED_ROWS + 5 {
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
                Toggle,
                Choice,
                Text,
            ],
            "after the candidates come deadline, palette and split, then the name again"
        );
        form.prev_field();
        assert_eq!(form.focused(), Choice);
        assert_eq!(form.current(), Row::Split);

        let mut form = edit(&research());
        form.prev_field();
        assert_eq!(form.current(), Row::Split);
        form.next_field();
        assert_eq!(form.focused(), Text);
        assert_eq!(form.current(), Row::Name);

        // A compete step with the switch off draws its split, fan-out and judge read-only: the
        // focus goes from the palette straight back to the name, and back again.
        let mut form = edit(&research_compete((ids::AGENT_AGY, "gemini")));
        for row in [Row::Split, Row::FanOut, Row::Judge] {
            assert!(form.shown().contains(&(row, false)), "{row:?} is drawn");
        }
        focus_on(&mut form, Row::Palette);
        form.next_field();
        assert_eq!(
            form.current(),
            Row::Name,
            "split, fan-out and judge skipped"
        );
        form.prev_field();
        assert_eq!(form.current(), Row::Palette);

        // A plan scope stored off divide is drawn read-only: the split is the last focus.
        let mut node = research();
        node.phase.plan_scope = Some(PlanScope::Successors);
        let mut form = edit(&node);
        assert!(form.shown().contains(&(Row::PlanScope, false)), "drawn");
        focus_on(&mut form, Row::Split);
        form.next_field();
        assert_eq!(form.current(), Row::Name, "the plan scope skipped");
        form.prev_field();
        assert_eq!(form.current(), Row::Split);
    }

    #[test]
    fn deadline_empty_is_none_and_below_1_is_refused() {
        for (typed, want) in [
            ("", Ok(None)),
            ("  ", Ok(None)),
            ("1", Ok(Some(1))),
            ("7200", Ok(Some(7200))),
            ("0", Err(DEADLINE_IS_A_NUMBER.to_owned())),
            ("-3", Err(DEADLINE_IS_A_NUMBER.to_owned())),
            ("soon", Err(DEADLINE_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = edit(&research());
            set_row_text(&mut form, Row::Deadline, typed);
            assert_eq!(form.focused(), NodeRow::Text);
            assert_eq!(
                built(&form).map(|node| node.phase.deadline_seconds),
                want,
                "{typed:?}"
            );
        }
    }

    #[test]
    fn palette_toggles() {
        let lookups = lookups();
        let mut form = edit(&research());
        focus_on(&mut form, Row::Palette);
        assert_eq!(form.focused(), NodeRow::Toggle);
        form.cycle(true, &lookups).expect("a toggle");
        assert_eq!(built(&form).map(|node| node.phase.palette), Ok(true));
        form.cycle(false, &lookups).expect("a toggle");
        assert_eq!(built(&form).map(|node| node.phase.palette), Ok(false));
    }

    #[test]
    fn split_offers_compete_only_with_the_switch_on() {
        let lookups = lookups();
        for (switch, want, help) in [
            (
                false,
                [Split::Divide, Split::None, Split::Divide],
                SPLIT_HELP_OFF,
            ),
            (
                true,
                [Split::Divide, Split::Compete, Split::None],
                SPLIT_HELP_ON,
            ),
        ] {
            let node = research();
            let mut form = open(&node, switch);
            focus_on(&mut form, Row::Split);
            assert_eq!(form.focused(), NodeRow::Choice);
            let shown = text(&form.lines(&node, &lookups, KEYS, 120, accent(), &Theme::default()));
            assert!(shown.contains(help), "switch {switch}: {shown}");
            let mut seen = Vec::new();
            for _ in 0..want.len() {
                form.cycle(true, &lookups).expect("a choice");
                seen.push(form.split);
            }
            assert_eq!(seen, want, "switch {switch}");
        }
    }

    #[test]
    fn leaving_divide_clears_the_divide_columns_parts_and_plan_scope() {
        let lookups = lookups();
        let node = research_divide();
        let mut form = edit(&node);
        focus_on(&mut form, Row::Split);
        form.cycle(false, &lookups).expect("a choice"); // divide → none
        assert_eq!(form.split, Split::None);
        assert_eq!(form.current(), Row::Split, "the focus stays on the split");
        assert_eq!(
            (
                form.planner_persona,
                &form.planner,
                form.integrate_persona,
                &form.integrate
            ),
            (None, &None, None, &None)
        );
        assert_eq!(form.max_parts.text(), Some(""));
        assert_eq!(form.plan_scope, None);
        assert!(form.parts.is_empty());

        let mut want = node.clone();
        let phase = &mut want.phase;
        phase.split = Split::None;
        phase.plan_scope = None;
        phase.planner_persona_id = None;
        phase.planner_agent_id = None;
        phase.planner_model = None;
        phase.integrate_persona_id = None;
        phase.integrate_agent_id = None;
        phase.integrate_model = None;
        phase.max_parts = None;
        want.parts = Vec::new();
        assert_eq!(form.build(&node, &lookups), Ok(want.clone()));

        // Back to divide: nothing comes back.
        form.cycle(true, &lookups).expect("a choice");
        assert_eq!(form.split, Split::Divide);
        want.phase.split = Split::Divide;
        assert_eq!(form.build(&node, &lookups), Ok(want));
    }

    #[test]
    fn entering_compete_sets_fan_out_2_and_leaving_it_sets_1_and_clears_the_judge() {
        let lookups = lookups();
        let node = research();
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Split);
        form.cycle(false, &lookups).expect("a choice"); // none → compete
        assert_eq!(form.split, Split::Compete);
        assert_eq!(form.fan_out.text(), Some("2"));
        let phase = |form: &NodeForm| {
            form.build(&node, &lookups).map(|node| {
                (
                    node.phase.split,
                    node.phase.fan_out,
                    node.phase.judge_agent_id,
                    node.phase.judge_model,
                )
            })
        };
        assert_eq!(phase(&form), Ok((Split::Compete, 2, None, None)));
        form.judge = Some((ids::AGENT_AGY, Some("gemini".to_owned())));
        assert_eq!(
            phase(&form),
            Ok((
                Split::Compete,
                2,
                Some(ids::AGENT_AGY),
                Some("gemini".to_owned())
            ))
        );

        form.cycle(true, &lookups).expect("a choice"); // compete → none
        assert_eq!(form.split, Split::None);
        assert_eq!(form.fan_out.text(), Some("1"));
        assert_eq!(form.judge, None);
        assert_eq!(phase(&form), Ok((Split::None, 1, None, None)));
    }

    #[test]
    fn entering_divide_clears_the_judge() {
        let lookups = lookups();
        // A split-none step with a judge: the store ties the judge to no split.
        let mut node = research();
        node.phase.judge_agent_id = Some(ids::AGENT_AGY);
        node.phase.judge_model = Some("gemini".to_owned());
        let mut form = edit(&node);
        assert_eq!(
            form.judge,
            Some((ids::AGENT_AGY, Some("gemini".to_owned())))
        );
        focus_on(&mut form, Row::Split);
        form.cycle(true, &lookups).expect("a choice"); // none → divide
        assert_eq!(form.split, Split::Divide);
        assert_eq!(form.judge, None);

        // From compete, the switch on: the judge goes and the fan-out is 1.
        let node = research_compete((ids::AGENT_AGY, "gemini"));
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Split);
        form.cycle(false, &lookups).expect("a choice"); // compete → divide
        assert_eq!(form.split, Split::Divide);
        assert_eq!(form.judge, None);
        assert_eq!(
            form.build(&node, &lookups).map(|node| (
                node.phase.split,
                node.phase.fan_out,
                node.phase.judge_agent_id,
                node.phase.judge_model
            )),
            Ok((Split::Divide, 1, None, None))
        );
    }

    #[test]
    fn a_compete_node_with_the_switch_off_is_read_only_and_kept_byte_for_byte() {
        let lookups = lookups();
        let base = research_compete((ids::AGENT_AGY, "gemini"));
        let mut form = edit(&base);
        let rows = form.rows();
        for row in [Row::Split, Row::FanOut, Row::Judge] {
            assert!(!rows.contains(&row), "{row:?} takes no focus");
            assert!(form.shown().contains(&(row, false)), "{row:?} is drawn");
        }
        let shown = text(&form.lines(&base, &lookups, KEYS, 120, accent(), &Theme::default()));
        assert!(shown.contains(COMPETE_LOCKED), "{shown}");
        assert!(shown.contains("agy \u{b7} gemini"), "{shown}");

        set_text(&mut form, NAME, "dig");
        let node = form.build(&base, &lookups).expect("a valid form");
        let mut want = base.clone();
        want.phase.name = "dig".to_owned();
        assert_eq!(node, want, "split, fan-out and judge as they were");

        // The switch on, the same step edits them.
        let rows = open(&base, true).rows();
        for row in [Row::Split, Row::FanOut, Row::Judge] {
            assert!(rows.contains(&row), "{row:?} takes the focus");
        }
    }

    #[test]
    fn fan_out_is_a_whole_number_2_or_more() {
        let node = research_compete((ids::AGENT_AGY, "gemini"));
        for (typed, want) in [
            ("2", Ok(2)),
            (" 5 ", Ok(5)),
            ("1", Err(FAN_OUT_IS_A_NUMBER.to_owned())),
            ("0", Err(FAN_OUT_IS_A_NUMBER.to_owned())),
            ("x", Err(FAN_OUT_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = open(&node, true);
            set_row_text(&mut form, Row::FanOut, typed);
            assert_eq!(form.focused(), NodeRow::Text);
            assert_eq!(
                form.build(&node, &lookups()).map(|node| node.phase.fan_out),
                want,
                "{typed:?}"
            );
        }
    }

    #[test]
    fn the_judge_cycles_project_default_then_every_pair_and_needs_the_agents_read() {
        let lookups = lookups();
        let mut node = research_bound();
        node.phase.split = Split::Compete;
        node.phase.fan_out = 2;
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Judge);
        assert_eq!(form.focused(), NodeRow::Choice);
        let mut seen = Vec::new();
        for _ in 0..4 {
            form.cycle(true, &lookups).expect("the agents read");
            seen.push(form.judge.clone());
        }
        assert_eq!(
            seen,
            [
                Some((ids::AGENT_CLAUDE, Some("opus".to_owned()))),
                Some((ids::AGENT_CLAUDE, Some("sonnet".to_owned()))),
                Some((ids::AGENT_AGY, Some("gemini".to_owned()))),
                None,
            ],
            "project default first, then every pair, wrapping"
        );
        form.cycle(false, &lookups).expect("the agents read");
        assert_eq!(
            form.build(&node, &lookups)
                .map(|node| (node.phase.judge_agent_id, node.phase.judge_model)),
            Ok((Some(ids::AGENT_AGY), Some("gemini".to_owned())))
        );

        // Unread agents: refused, the judge kept.
        let unread = Lookups {
            agents: None,
            ..lookups.clone()
        };
        assert_eq!(form.cycle(true, &unread), Err(AGENTS_NOT_READ.to_owned()));
        assert_eq!(
            form.judge,
            Some((ids::AGENT_AGY, Some("gemini".to_owned())))
        );

        // An agent with no model is a judge with no model.
        let bare = summary(AgentId::new(), "bare", &[], None);
        let only_bare = Lookups {
            agents: Some(vec![bare.clone()]),
            ..lookups.clone()
        };
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Judge);
        form.cycle(true, &only_bare).expect("the agents read");
        assert_eq!(form.judge, Some((bare.agent.id, None)));

        // A judge the agents lack is kept, shown by id, and sits last.
        let ghost = AgentId::new();
        node.phase.judge_agent_id = Some(ghost);
        node.phase.judge_model = Some("old".to_owned());
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Judge);
        let shown = text(&form.lines(&node, &lookups, KEYS, 120, accent(), &Theme::default()));
        assert!(shown.contains(&ghost.to_string()), "{shown}");
        form.cycle(true, &lookups).expect("the agents read");
        assert_eq!(
            form.judge, None,
            "forward from the last wraps to the default"
        );
    }

    #[test]
    fn divide_seats_cycle_personas_and_pairs() {
        let lookups = lookups();
        let node = research_divide();
        let mut form = edit(&node);
        for row in [
            Row::PlannerPersona,
            Row::PlannerAgent,
            Row::IntegratePersona,
            Row::IntegrateAgent,
        ] {
            focus_on(&mut form, row);
            assert_eq!(form.focused(), NodeRow::Choice, "{row:?}");
            form.cycle(true, &lookups).expect("a choice");
        }
        // architect → none; claude · opus → claude · sonnet; reviewer → architect; agy with no
        // model is none of the pairs, so it sits last and forward wraps to none.
        assert_eq!(form.planner_persona, None);
        assert_eq!(
            form.planner,
            Some((ids::AGENT_CLAUDE, Some("sonnet".to_owned())))
        );
        assert_eq!(form.integrate_persona, Some(ids::PERSONA_ARCHITECT));
        assert_eq!(form.integrate, None);
        let built = form.build(&node, &lookups).expect("a valid form");
        let phase = &built.phase;
        assert_eq!(
            (
                phase.planner_persona_id,
                phase.planner_agent_id,
                phase.planner_model.as_deref(),
                phase.integrate_persona_id,
                phase.integrate_agent_id,
                phase.integrate_model.as_deref(),
            ),
            (
                None,
                Some(ids::AGENT_CLAUDE),
                Some("sonnet"),
                Some(ids::PERSONA_ARCHITECT),
                None,
                None,
            )
        );
        form.cycle(true, &lookups).expect("a choice");
        assert_eq!(
            form.integrate,
            Some((ids::AGENT_CLAUDE, Some("opus".to_owned())))
        );

        let unread = Lookups {
            agents: None,
            ..lookups.clone()
        };
        for row in [Row::PlannerAgent, Row::IntegrateAgent] {
            focus_on(&mut form, row);
            assert_eq!(
                form.cycle(true, &unread),
                Err(AGENTS_NOT_READ.to_owned()),
                "{row:?}"
            );
        }
        focus_on(&mut form, Row::PlannerPersona);
        form.cycle(true, &unread).expect("personas need no read");
    }

    #[test]
    fn max_parts_empty_is_none_and_below_1_is_refused() {
        let node = research_divide();
        for (typed, want) in [
            ("", Ok(None)),
            ("  ", Ok(None)),
            ("1", Ok(Some(1))),
            ("4", Ok(Some(4))),
            ("0", Err(MAX_PARTS_IS_A_NUMBER.to_owned())),
            ("-2", Err(MAX_PARTS_IS_A_NUMBER.to_owned())),
            ("many", Err(MAX_PARTS_IS_A_NUMBER.to_owned())),
        ] {
            let mut form = edit(&node);
            set_row_text(&mut form, Row::MaxParts, typed);
            assert_eq!(form.focused(), NodeRow::Text);
            assert_eq!(
                form.build(&node, &lookups())
                    .map(|node| node.phase.max_parts),
                want,
                "{typed:?}"
            );
        }
    }

    #[test]
    fn plan_scope_cycles_none_parts_successors_graph() {
        let lookups = lookups();
        let mut node = research_divide();
        node.phase.plan_scope = None;
        let mut form = edit(&node);
        focus_on(&mut form, Row::PlanScope);
        assert_eq!(form.focused(), NodeRow::Choice);
        let mut seen = Vec::new();
        for _ in 0..4 {
            form.cycle(true, &lookups).expect("a choice");
            seen.push(form.plan_scope);
        }
        assert_eq!(
            seen,
            [
                Some(PlanScope::Parts),
                Some(PlanScope::Successors),
                Some(PlanScope::Graph),
                None,
            ]
        );
        form.cycle(false, &lookups).expect("a choice");
        assert_eq!(
            form.build(&node, &lookups)
                .map(|node| node.phase.plan_scope),
            Ok(Some(PlanScope::Graph))
        );
    }

    #[test]
    fn a_stored_plan_scope_off_divide_is_read_only_and_kept() {
        let lookups = lookups();
        let mut node = research();
        node.phase.plan_scope = Some(PlanScope::Successors);
        let mut form = edit(&node);
        assert!(!form.rows().contains(&Row::PlanScope), "no focus");
        assert!(form.shown().contains(&(Row::PlanScope, false)), "drawn");
        let shown = text(&form.lines(&node, &lookups, KEYS, 56, accent(), &Theme::default()));
        assert!(shown.contains("its successors"), "{shown}");
        set_text(&mut form, NAME, "dig");
        assert_eq!(
            form.build(&node, &lookups)
                .map(|node| node.phase.plan_scope),
            Ok(Some(PlanScope::Successors))
        );
        // A step with none draws no plan scope row off divide.
        assert!(
            !edit(&research())
                .shown()
                .iter()
                .any(|(row, _)| *row == Row::PlanScope)
        );
        // Divide makes it a choice again.
        focus_on(&mut form, Row::Split);
        form.cycle(true, &lookups).expect("a choice");
        assert!(form.rows().contains(&Row::PlanScope));
        assert_eq!(form.plan_scope, Some(PlanScope::Successors));
    }

    #[test]
    fn the_divide_rows_follow_the_split_and_the_parts_name_their_keys() {
        let node = research_divide();
        let form = edit(&node);
        let tail: Vec<Row> = form.rows().split_off(FIXED_ROWS + node.agents.len());
        assert_eq!(
            tail,
            [
                Row::Deadline,
                Row::Palette,
                Row::Split,
                Row::PlannerPersona,
                Row::PlannerAgent,
                Row::IntegratePersona,
                Row::IntegrateAgent,
                Row::MaxParts,
                Row::PlanScope,
                Row::Part(0),
                Row::Part(1),
            ]
        );
        let mut form = edit(&node);
        focus_on(&mut form, Row::Part(1));
        assert_eq!(form.focused(), NodeRow::Part(1));
        let shown = text(&form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default()));
        assert!(shown.contains("store \u{b7} Store"), "{shown}");
        assert!(shown.contains("Alt+e edits it"), "{shown}");
        let quiet = FormKeys {
            add: None,
            open: None,
        };
        let shown = text(&form.lines(&node, &lookups(), quiet, 56, accent(), &Theme::default()));
        assert!(
            !shown.contains("edits it"),
            "nothing bound, no help: {shown}"
        );

        // A part with no title is its key alone; none shows the placeholder.
        let mut bare = node.clone();
        bare.parts[0].title = String::new();
        let shown =
            text(&edit(&bare).lines(&bare, &lookups(), KEYS, 56, accent(), &Theme::default()));
        assert!(
            shown
                .lines()
                .any(|line| line.trim_start().starts_with("part 1") && line.ends_with(" ui")),
            "{shown}"
        );
        bare.parts.clear();
        let mut form = edit(&bare);
        focus_on(&mut form, Row::NoParts);
        assert_eq!(form.focused(), NodeRow::NoParts);
        let shown = text(&form.lines(&bare, &lookups(), KEYS, 56, accent(), &Theme::default()));
        assert!(shown.contains("none (Alt+n adds one)"), "{shown}");
        assert_eq!(no_parts(None), "none");

        // A compete step shows fan-out and judge; none shows neither.
        let compete = open(&research_compete((ids::AGENT_AGY, "gemini")), true);
        let tail: Vec<Row> = compete.rows().split_off(FIXED_ROWS + 2);
        assert_eq!(
            tail,
            [
                Row::Deadline,
                Row::Palette,
                Row::Split,
                Row::FanOut,
                Row::Judge
            ]
        );
    }

    /// The keys of `form`'s parts, in order.
    fn keys_of(form: &NodeForm) -> Vec<&str> {
        form.parts.iter().map(|part| part.key.as_str()).collect()
    }

    /// Types `text` into the focused row of `form` after clearing it.
    fn type_part(form: &mut PartForm, text: &str) {
        for _ in 0..64 {
            let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
        for c in text.chars() {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(form.on_text_key(key), FieldOutcome::Consumed);
        }
    }

    #[test]
    fn the_list_acts_follow_the_focus() {
        let node = research_divide();
        let mut form = edit(&node);

        // A part row: the parts.
        focus_on(&mut form, Row::Part(0));
        assert_eq!(form.list_focus(), ListFocus::Parts);
        form.list_move(true).expect("the top stays put");
        assert_eq!(form.current(), Row::Part(0));
        form.list_move(false).expect("a part row");
        assert_eq!(keys_of(&form), ["store", "ui"]);
        assert_eq!(form.current(), Row::Part(1), "focus follows the move");
        form.list_move(false).expect("the bottom stays put");
        assert_eq!(form.current(), Row::Part(1));
        form.list_remove().expect("a part row");
        assert_eq!(keys_of(&form), ["store"]);
        assert_eq!(form.current(), Row::Part(0), "the last row now");
        assert_eq!(form.candidates.len(), 2, "the candidates untouched");
        form.list_remove().expect("a part row");
        assert_eq!(form.current(), Row::NoParts);
        assert_eq!(form.list_focus(), ListFocus::Parts);
        assert_eq!(form.list_remove(), Err(NO_PART.to_owned()));
        assert_eq!(form.list_move(true), Err(NO_PART.to_owned()));
        assert_eq!(
            form.open_part(PartOpen::Focused).map(|part| part.target()),
            Err(NO_PART.to_owned())
        );
        assert_eq!(
            form.open_part(PartOpen::New).map(|part| part.target()),
            Ok(PartTarget::New),
            "the placeholder adds"
        );

        // A candidate row: the candidates.
        focus(&mut form, FIXED_ROWS + 1);
        assert_eq!(form.list_focus(), ListFocus::Candidates);
        form.list_move(true).expect("a candidate row");
        assert_eq!(
            form.candidates,
            [
                (ids::AGENT_AGY, "gemini".to_owned()),
                (ids::AGENT_CLAUDE, "sonnet".to_owned()),
            ]
        );
        form.list_remove().expect("a candidate row");
        assert_eq!(form.candidates.len(), 1);
        assert!(form.parts.is_empty(), "the parts untouched");

        // Any other row: M1's refusal.
        focus(&mut form, NAME);
        assert_eq!(form.list_focus(), ListFocus::Candidates);
        assert_eq!(form.list_remove(), Err(NO_CANDIDATE.to_owned()));
        assert_eq!(form.list_move(false), Err(NO_CANDIDATE.to_owned()));
        assert_eq!(
            form.open_part(PartOpen::Focused).map(|part| part.target()),
            Err(NO_PART.to_owned())
        );

        // Off divide there are no parts to add.
        let plain = edit(&research());
        assert_eq!(
            plain.open_part(PartOpen::New).map(|part| part.target()),
            Err(PARTS_ON_DIVIDE_ONLY.to_owned())
        );
        let mut form = edit(&node);
        focus_on(&mut form, Row::Part(1));
        assert_eq!(
            form.open_part(PartOpen::Focused).map(|part| part.target()),
            Ok(PartTarget::Edit(1))
        );
    }

    #[test]
    fn put_part_appends_replaces_and_renumbers() {
        let lookups = lookups();
        let node = research_divide();
        let mut form = edit(&node);

        // New: appended, the focus on it.
        let mut part = form.open_part(PartOpen::New).expect("a divide step");
        type_part(&mut part, "api");
        form.put_part(&part).expect("a free key");
        assert_eq!(keys_of(&form), ["ui", "store", "api"]);
        assert_eq!(form.current(), Row::Part(2));

        // Edit: replaced in place, its own key free to keep.
        focus_on(&mut form, Row::Part(0));
        let mut part = form.open_part(PartOpen::Focused).expect("a part row");
        type_part(&mut part, "web");
        form.put_part(&part).expect("a free key");
        assert_eq!(keys_of(&form), ["web", "store", "api"]);
        assert_eq!(form.current(), Row::Part(0));
        focus_on(&mut form, Row::Part(1));
        let part = form.open_part(PartOpen::Focused).expect("a part row");
        form.put_part(&part).expect("its own key");
        assert_eq!(keys_of(&form), ["web", "store", "api"]);

        // A taken key is refused and nothing moves.
        let mut part = form.open_part(PartOpen::New).expect("a divide step");
        type_part(&mut part, "store");
        assert_eq!(
            form.put_part(&part),
            Err(crate::ui::graph_editor::part_form::part_key_taken("store"))
        );
        assert_eq!(keys_of(&form), ["web", "store", "api"]);
        assert_eq!(form.current(), Row::Part(1));

        // Moved and built: positions 0..n on the node's id.
        form.part_move(true).expect("a part row");
        let built = form.build(&node, &lookups).expect("a valid form");
        let rows: Vec<(PhaseId, i32, &str)> = built
            .parts
            .iter()
            .map(|part| (part.phase_id, part.position, part.key.as_str()))
            .collect();
        let id = node.phase.id;
        assert_eq!(rows, [(id, 0, "store"), (id, 1, "web"), (id, 2, "api")]);

        // An edit of a part the step no longer has.
        focus_on(&mut form, Row::Part(2));
        let part = form.open_part(PartOpen::Focused).expect("a part row");
        form.part_remove().expect("a part row");
        assert_eq!(form.put_part(&part), Err(NO_PART.to_owned()));

        // A blank part's key is required.
        let part = form.open_part(PartOpen::New).expect("a divide step");
        assert_eq!(
            form.put_part(&part),
            Err(crate::ui::graph_editor::part_form::PART_KEY_REQUIRED.to_owned())
        );
    }

    #[test]
    fn the_part_form_offers_the_step_forms_candidates_as_they_are_now() {
        let lookups = lookups();
        let node = research_divide();
        let mut form = edit(&node);
        focus(&mut form, FIXED_ROWS + 1);
        form.candidate_remove().expect("a candidate row");
        let mut part = form.open_part(PartOpen::New).expect("a divide step");
        type_part(&mut part, "api");
        for _ in 0..5 {
            part.next_field();
        }
        part.cycle(true, &lookups);
        form.put_part(&part).expect("a free key");
        let built = form.build(&node, &lookups).expect("a valid form");
        let api = built.parts.last().expect("the new part");
        assert_eq!(
            (api.agent_id, api.model.as_deref()),
            (Some(ids::AGENT_CLAUDE), Some("sonnet")),
            "the one candidate left"
        );
    }

    #[test]
    fn debug_never_prints_typed_text() {
        let mut form = edit(&research());
        set_text(&mut form, NAME, "hunter2");
        set_text(&mut form, VERIFY, "swordfish");
        set_row_text(&mut form, Row::Deadline, "31337");
        let printed = format!("{form:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("swordfish"), "{printed}");
        assert!(!printed.contains("31337"), "{printed}");
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
            NodeForm::new(&node, after, false).title(&graph),
            "new step after research"
        );
        assert_eq!(
            NodeForm::new(&node, NodeTarget::New { after: None }, false).title(&graph),
            "new step"
        );
    }

    #[test]
    fn node_form_edit() {
        let node = research_bound();
        let form = edit(&node);
        let lines = form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default());
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
        let form = NodeForm::new(&node, target, false);
        let lines = form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_new", text(&lines));
    }

    #[test]
    fn node_form_divide() {
        let node = research_divide();
        let mut form = edit(&node);
        focus_on(&mut form, Row::Part(0));
        let lines = form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_divide", text(&lines));
    }

    #[test]
    fn node_form_compete_switch_on() {
        let node = research_compete((ids::AGENT_CLAUDE, "opus"));
        let mut form = open(&node, true);
        focus_on(&mut form, Row::Judge);
        let lines = form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_compete_switch_on", text(&lines));
    }

    #[test]
    fn node_form_compete_switch_off() {
        let node = research_compete((ids::AGENT_CLAUDE, "opus"));
        let form = edit(&node);
        let lines = form.lines(&node, &lookups(), KEYS, 56, accent(), &Theme::default());
        insta::assert_snapshot!("node_form_compete_switch_off", text(&lines));
    }
}
