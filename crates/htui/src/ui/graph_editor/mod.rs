//! MOD-141: the one step-graph editor (PRD `mod-141-step-graph-editor`, M1 plan D1-D14).
//!
//! It edits one [`GraphBody`] at a time, of any scope (project, workspace, library): nodes and
//! typed edges drawn with rataflow, a node form, an edge form, a links list, the store's own graph
//! rules run before every write (D5), and in-memory undo and redo where every applied edit is one
//! `StoreRequest::ReplaceGraph` compare-and-set on `step_graph.revision` (D4, D6). Nothing in here
//! names `Settings > Kinds`: the host hands it a body, the persona registry and every reply.
//!
//! - `model`: pure body edits and the client-side validator (D4, D5, D10).
//! - `history`: undo, redo and the one write in flight (D4, D6).
//! - `canvas`: the rataflow projection, layout, selection and mouse (D7, D11).
//! - `node_form`, `edge_form`, `links`: the forms and the links list (D8, D9, D10).
//! - `notes`: what a run of one step would be refused with today, in resolve's own words (M2 D11).
//! - `part_form`: one default part of a divide step (M2 D9).
//! - [`GraphEditor`] (this module): the modes, every key through the key catalogue (D12), the
//!   writes and the replies (D4), the mouse (D11) and the drawing (D2). Only this module names
//!   `Act`, `Keys`, `Stack`, `Ctx` or `StoreRequest`.

pub(crate) mod canvas;
pub(crate) mod edge_form;
pub(crate) mod history;
pub(crate) mod links;
pub(crate) mod model;
pub(crate) mod node_form;
pub(crate) mod notes;
pub(crate) mod part_form;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use chrono::Utc;
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use htui_core::model::{
    AgentSummary, EdgeKind, EdgePredicate, GraphBody, GraphScope, PhaseId, PromptTemplate, Scope,
    StepGraphId,
};
use htui_core::store::{CasOutcome, WalkEdge, WalkNode, forward_mask};
use rataflow::Direction;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use self::canvas::{Canvas, CanvasEvent};
use self::edge_form::{EdgeForm, EdgeRow, EdgeTarget, cycle_kind};
use self::history::{History, Write};
use self::links::Links;
use self::node_form::{FormKeys, ListFocus, NodeForm, NodeRow, NodeTarget, PartOpen};
use self::part_form::{PartForm, PartRow};
use crate::app::{Ctx, Handled};
use crate::catalogue::{EditorSettings, PersonaSummary};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::TemplatesSnapshot;
use crate::ui::cells::cell_width;
use crate::ui::kit::form::FormLines;
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::{FieldOutcome, Theme};

/// What the host hands the editor beside the body (D1): the persona registry, always known (the
/// catalogue carries it), and the agents and the templates once their reads answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Lookups {
    /// The persona registry by name: the node form's persona choice and the validator's persona
    /// references (D5, D8).
    pub(crate) personas: Vec<PersonaSummary>,
    /// `StoreReply::Agents`, by `agent.name`; `None` until it arrived. While `None` the validator
    /// leaves the agent references to the store, and the candidate list cannot be edited.
    pub(crate) agents: Option<Vec<AgentSummary>>,
    /// The templates the graph's scope may use, every version; `None` until
    /// `StoreReply::Templates` arrived.
    pub(crate) templates: Option<Vec<PromptTemplate>>,
}

/// One link's editable part (D9): what the edge form yields and what `model` writes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdgeSpec {
    /// `step_graph_edge.kind`.
    pub(crate) kind: EdgeKind,
    /// `step_graph_edge.predicate`: `Some` exactly when `kind` is `if`.
    pub(crate) predicate: Option<EdgePredicate>,
    /// `step_graph_edge.loop_limit`: required on `changes_requested` and `fails`, optional on
    /// `asks_you`, `None` otherwise.
    pub(crate) loop_limit: Option<i32>,
}

/// The relations in picker order (D9).
pub(crate) const RELATIONS: [EdgeKind; 5] = [
    EdgeKind::Next,
    EdgeKind::If,
    EdgeKind::AsksYou,
    EdgeKind::ChangesRequested,
    EdgeKind::Fails,
];

/// A relation in words, never its wire name (MOD-83).
pub(crate) const fn relation_word(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Next => "next",
        EdgeKind::If => "if",
        EdgeKind::AsksYou => "asks you",
        EdgeKind::ChangesRequested => "changes requested",
        EdgeKind::Fails => "fails",
    }
}

/// Per edge of `body`, in `body.edges` order, whether it is forward: `forward_mask` over the body
/// exactly as `graph_body_walk_refusal` builds its walk, so the editor and the store cannot
/// disagree (D7). An edge naming a step the body lacks is `false`.
pub(crate) fn forward_edges(body: &GraphBody) -> Vec<bool> {
    let index: HashMap<PhaseId, usize> = body
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.phase.id, at))
        .collect();
    let nodes: Vec<WalkNode<'_>> = body
        .nodes
        .iter()
        .map(|node| WalkNode {
            name: &node.phase.name,
            palette: node.phase.palette,
        })
        .collect();
    // Out of range rather than skipped, so the mask stays aligned with `body.edges`.
    let at = |id: PhaseId| index.get(&id).copied().unwrap_or(usize::MAX);
    let edges: Vec<WalkEdge> = body
        .edges
        .iter()
        .map(|edge| WalkEdge {
            from: at(edge.from),
            to: at(edge.to),
            kind: edge.kind,
        })
        .collect();
    forward_mask(&nodes, &edges)
}

/// The name of step `id` of `body`, or `None` when the body has no such step.
pub(crate) fn step_name(body: &GraphBody, id: PhaseId) -> Option<&str> {
    body.nodes
        .iter()
        .find(|node| node.phase.id == id)
        .map(|node| node.phase.name.as_str())
}

/// D4: what a write that met another one's revision says; undo and redo are gone with it.
pub(crate) const STALE_RELOADED: &str = "changed elsewhere, reloaded; undo history cleared";
/// An edit, delete, connect or links key with no step selected.
pub(crate) const NO_STEP_SELECTED: &str = "no step is selected";
/// What a connect left with `close` says.
pub(crate) const NOT_CONNECTED: &str = "not connected";
/// What connecting from a palette step says: a palette step takes no link (D7).
pub(crate) const PALETTE_HAS_NO_LINKS: &str = "a palette step has no links";

/// The notice after an edit landed.
fn saved(label: &str) -> String {
    format!("saved: {label}")
}

/// The notice after an undo landed.
fn undid(label: &str) -> String {
    format!("undid {label}")
}

/// The notice after a redo landed.
fn redid(label: &str) -> String {
    format!("redid {label}")
}

/// An undo or redo the client's rules refused before it left (MOD-141 R1 M1): it is dropped
/// from the history.
fn cannot(write: &Write, refusal: &str) -> String {
    let (verb, label) = match write {
        Write::Undo(label) => ("undo", label),
        Write::Redo(label) => ("redo", label),
        Write::Edit(label) => ("save", label),
    };
    format!("can't {verb} {label}: {refusal}; dropped from history")
}

/// The delete question (D10, C3): the editor cannot see the step's skill bindings, so it always
/// says what undo does not bring back. Since M2 (M-3, C13) the body carries the deadline and the
/// judge, so undo brings those back. `answers` is the question's own keys.
fn delete_question(name: &str, answers: &str) -> String {
    format!(
        "delete step {name}? its links go with it, and what led into it now leads to what came \
         after; undo brings the step back but not its skill bindings. {answers}"
    )
}

/// The line under the canvas while connecting: what the pick is for.
fn connecting(from: &str) -> String {
    format!("linking from {from}: pick the step it leads to")
}

/// The notice of a lookup read the store refused (`agents`, `templates`, the run limits): the
/// editor still works without it.
fn unavailable(what: &str, message: &str) -> String {
    format!("{what} unavailable: {message}")
}

/// The header's second line before the settings read answered (M2 D6).
const LIMITS_NOT_READ: &str = "limits not read yet";

/// What ends a [`limits_row`] that had to drop part of the line.
const LIMITS_MORE: &str = " \u{b7} \u{2026}";

/// M2 D13: the run limits resolve freezes, read-only (M-1): exactly what a run would freeze.
fn limits_line(settings: Option<&EditorSettings>) -> String {
    settings.map_or_else(
        || LIMITS_NOT_READ.to_owned(),
        |settings| {
            let bounds = &settings.bounds;
            format!(
                "limits \u{b7} fan-out {} \u{b7} agents {} \u{b7} plan revisions {} \u{b7} parallel {} \
                 \u{b7} child runs {} \u{b7} run-time changes {}",
                bounds.max_fan_out,
                bounds.max_agents_per_run,
                bounds.max_plan_revisions,
                bounds.max_parallel_sessions,
                bounds.max_child_runs,
                bounds.runtime_changes.as_str(),
            )
        },
    )
}

/// [`limits_line`] in its one row, `width` cells wide: the editor clips it, it never wraps
/// (Open risk 5). What does not fit goes a whole `label value` at a time and ` · …` says so,
/// so no number is cut in half; a row too narrow for the first is `…`.
fn limits_row(settings: Option<&EditorSettings>, width: u16) -> String {
    let line = limits_line(settings);
    let width = usize::from(width);
    if cell_width(&line) <= width {
        return line;
    }
    let mut kept = String::new();
    for segment in line.split(" \u{b7} ") {
        let longer = if kept.is_empty() {
            segment.to_owned()
        } else {
            format!("{kept} \u{b7} {segment}")
        };
        if cell_width(&longer) + cell_width(LIMITS_MORE) > width {
            break;
        }
        kept = longer;
    }
    if kept.is_empty() {
        "\u{2026}".to_owned()
    } else {
        kept + LIMITS_MORE
    }
}

/// The step form's rows and its notes in `height` rows (M2 D11, D14), all of them when they fit.
/// Else the notes stay on screen under the rows, taking at most half of them (`…` ending the
/// notes when some were cut), and the rows above scroll the focused row into view.
fn fitted(
    form: FormLines,
    mut notes: Vec<Line<'static>>,
    height: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let height = usize::from(height);
    if form.lines.len() + notes.len() <= height {
        let mut lines = form.lines;
        lines.extend(notes);
        return lines;
    }
    let room = height / 2;
    if notes.len() > room {
        notes.truncate(room.saturating_sub(1));
        if room > 0 {
            notes.push(notice("  \u{2026}", Tone::Error, theme));
        }
    }
    let rows = height - notes.len();
    let offset = form.offset(u16::try_from(rows).unwrap_or(u16::MAX));
    let mut lines: Vec<Line<'static>> = form.lines.into_iter().skip(offset).take(rows).collect();
    lines.extend(notes);
    lines
}

/// The canvas's keys.
const HINT_CANVAS: HintSpec = &[
    Hint::Pair(Act::GraphEditorDown, Act::GraphEditorUp, "select"),
    Hint::One(Act::GraphEditorAdd, "add"),
    Hint::One(Act::GraphEditorEdit, "edit"),
    Hint::One(Act::GraphEditorDelete, "delete"),
    Hint::One(Act::GraphEditorConnect, "connect"),
    Hint::One(Act::GraphEditorOpenLinks, "links"),
    Hint::One(Act::GraphEditorUndo, "undo"),
    Hint::One(Act::GraphEditorRedo, "redo"),
    Hint::One(Act::GraphEditorFit, "fit"),
    Hint::One(Act::GraphEditorClose, "close"),
];

/// Connecting's keys.
const HINT_CONNECT: HintSpec = &[
    Hint::Pair(Act::GraphEditorDown, Act::GraphEditorUp, "pick"),
    Hint::One(Act::GraphEditorEdit, "connect here"),
    Hint::One(Act::GraphEditorClose, "cancel"),
];

/// The step form's keys while any row but a part row has the focus.
const HINT_NODE_FORM: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "value"),
    Hint::One(Act::GraphEditorCandidateAdd, "add agent"),
    Hint::One(Act::GraphEditorCandidateRemove, "remove agent"),
    Hint::Pair(
        Act::GraphEditorCandidateUp,
        Act::GraphEditorCandidateDown,
        "order",
    ),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The step form's keys while a part row has the focus (M2 D14): the list acts name parts.
const HINT_NODE_FORM_PARTS: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "value"),
    Hint::One(Act::GraphEditorCandidateAdd, "add part"),
    Hint::One(Act::GraphEditorOpenPart, "edit part"),
    Hint::One(Act::GraphEditorCandidateRemove, "remove part"),
    Hint::Pair(
        Act::GraphEditorCandidateUp,
        Act::GraphEditorCandidateDown,
        "order",
    ),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The part form's keys (M2 D9): `Enter` keeps the part in the step form, nothing is written.
const HINT_PART_FORM: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "value"),
    Hint::One(Act::FieldSubmit, "done"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The link form's keys.
const HINT_EDGE_FORM: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "relation"),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The links list's keys.
const HINT_LINKS: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "select"),
    Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "relation"),
    Hint::One(Act::LinksEdit, "edit"),
    Hint::One(Act::LinksDelete, "delete"),
    Hint::One(Act::LinksClose, "back"),
];

/// The delete question's keys, on the hint row and at the end of the question.
const HINT_CONFIRM: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::All(Act::ConfirmNo, "keep"),
];

/// The panel's widest: the forms and the links list are drawn at most this wide.
const PANEL_WIDTH: u16 = 56;

/// The canvas's narrowest beside a panel.
const CANVAS_MIN: u16 = 30;

/// The most rows the notice (or the delete question) takes under the canvas.
const NOTICE_ROWS: usize = 3;

/// What one key did: handled (or passed) inside the editor, or the editor asks its host to close
/// it (`graph_editor.close` with nothing in flight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKey {
    /// The editor's answer, as a section's.
    Handled(Handled),
    /// The host goes back to its own view.
    Close,
}

/// The step form and the node it was opened on (the columns the form does not edit).
struct NodeEditing {
    /// The form.
    form: NodeForm,
    /// The node as it was (an edit) or the draft (an add).
    base: htui_core::model::NodeBody,
}

/// The part form over the step form it came from (M2 D9).
struct PartEditing {
    /// The part form.
    form: PartForm,
    /// The step form it returns to, unchanged until the part is kept.
    node: Box<NodeEditing>,
}

/// What the editor is doing.
enum Mode {
    /// The canvas has the keys.
    Canvas,
    /// Picking a link's target from `from`.
    Connecting {
        /// The step the link leaves.
        from: PhaseId,
    },
    /// The step form is open.
    NodeForm(Box<NodeEditing>),
    /// The part form is open over the step form (M2 D9).
    PartForm(Box<PartEditing>),
    /// The link form is open.
    EdgeForm(Box<EdgeForm>),
    /// The links list of one step.
    Links(Links),
    /// The delete question about `node`.
    ConfirmDelete {
        /// The step to delete.
        node: PhaseId,
    },
}

impl Mode {
    /// The mode's name, for `Debug`: no typed text.
    const fn name(&self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Connecting { .. } => "connecting",
            Self::NodeForm(_) => "step form",
            Self::PartForm(_) => "part form",
            Self::EdgeForm(_) => "link form",
            Self::Links(_) => "links",
            Self::ConfirmDelete { .. } => "delete question",
        }
    }
}

/// One line under the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Something to read and move on.
    Info(String),
    /// A refusal, or a write that met another one's revision.
    Error(String),
}

/// MOD-141 D1: the step-graph editor on one graph of any scope. Holds no store handle
/// (`R-NF-3`): every write leaves through `Ctx::request`, every reply comes back through
/// `on_reply`.
pub struct GraphEditor {
    /// The body on screen: the store's, as the last reply left it.
    body: GraphBody,
    /// The selected step.
    selected: Option<PhaseId>,
    /// What the keys do.
    mode: Mode,
    /// Undo, redo and the write in flight (D4, D6).
    history: History,
    /// The personas, agents and templates (D1).
    lookups: Lookups,
    /// The run limits and the compete switch (M2 D6, C8); `None` until the settings read
    /// answered, which reads as the switch off and the limits not read.
    settings: Option<EditorSettings>,
    /// The canvas; rataflow draws through `&mut`, so `render(&self)` borrows it.
    canvas: RefCell<Canvas>,
    /// The body, the selection or the lookups changed since the canvas last synced.
    resync: Cell<bool>,
    /// Where the canvas was last drawn, for the mouse.
    canvas_area: Cell<Option<Rect>>,
    /// The step to select when the write in flight lands (an add's new step).
    after_save: Option<PhaseId>,
    /// The line under the canvas.
    notice: Option<Notice>,
}

/// The graph, its revision, the mode's name, the selection and whether a write is in flight;
/// never the step names or anything typed (H-7).
impl core::fmt::Debug for GraphEditor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GraphEditor")
            .field("graph", &self.body.graph.id)
            .field("revision", &self.body.graph.revision)
            .field("mode", &self.mode.name())
            .field("selected", &self.selected)
            .field("busy", &self.busy())
            .finish_non_exhaustive()
    }
}

impl GraphEditor {
    /// Opens on `body` with the host's persona registry, and answers the reads to send now:
    /// `[StoreRequest::Agents, StoreRequest::Templates(scope), StoreRequest::GraphEditorSettings]`.
    /// Selects the first flow step (position order), else none.
    #[must_use]
    pub fn open(
        body: GraphBody,
        personas: Vec<PersonaSummary>,
        scope: &Scope,
    ) -> (Self, Vec<StoreRequest>) {
        let selected = first_step(&body);
        let editor = Self {
            body,
            selected,
            mode: Mode::Canvas,
            history: History::default(),
            lookups: Lookups {
                personas,
                agents: None,
                templates: None,
            },
            settings: None,
            canvas: RefCell::new(Canvas::new()),
            resync: Cell::new(true),
            canvas_area: Cell::new(None),
            after_save: None,
            notice: None,
        };
        let requests = editor.wants_requests(scope);
        (editor, requests)
    }

    /// The graph on screen.
    #[must_use]
    pub const fn graph(&self) -> StepGraphId {
        self.body.graph.id
    }

    /// Selects step `node` (the host opens on a phase row); unknown ids are ignored.
    pub fn select(&mut self, node: PhaseId) {
        if step_name(&self.body, node).is_some() {
            self.selected = Some(node);
            self.resync.set(true);
        }
    }

    /// Opens the step form on the selected step (`e` on a phase row, D14); with none selected,
    /// says so. Refused while a write is in flight: its reply would close the form.
    pub fn open_node_form(&mut self) {
        if self.busy() {
            self.refuse(history::IN_FLIGHT);
            return;
        }
        let Some(node) = self
            .selected
            .and_then(|id| self.body.nodes.iter().find(|node| node.phase.id == id))
        else {
            self.refuse(NO_STEP_SELECTED);
            return;
        };
        let form = NodeForm::new(node, NodeTarget::Edit(node.phase.id), self.compete());
        self.mode = Mode::NodeForm(Box::new(NodeEditing {
            form,
            base: node.clone(),
        }));
        self.notice = None;
    }

    /// The reads the host re-sends on activation: `Agents`, `Templates(scope)` and (M2 D6) the
    /// run limits and the compete switch, `GraphEditorSettings`.
    #[must_use]
    pub fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![
            StoreRequest::Agents,
            StoreRequest::Templates(scope.clone()),
            StoreRequest::GraphEditorSettings,
        ]
    }

    /// The compete switch as last read (M2 D5): off until the settings read answers. A step form
    /// keeps the switch it opened with, but one opened before the first read takes that read.
    fn compete(&self) -> bool {
        self.settings.is_some_and(|settings| settings.compete)
    }

    /// The persona registry changed (every catalogue the host adopts).
    pub fn set_personas(&mut self, personas: &[PersonaSummary]) {
        personas.clone_into(&mut self.lookups.personas);
    }

    /// Whether a write is in flight.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.history.busy().is_some()
    }

    /// The current mode's stack: the canvas `GRAPH_EDITOR`, connecting `GRAPH_EDITOR_CONNECT`,
    /// the step form `GRAPH_EDITOR_NODE_FORM`, the part form `GRAPH_EDITOR_PART_FORM` (M2 D14),
    /// the link form `GRAPH_EDITOR_EDGE_FORM`, the links list `GRAPH_EDITOR_LINKS` and the delete
    /// question `GRAPH_EDITOR_CONFIRM`.
    #[must_use]
    pub const fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Canvas => views::GRAPH_EDITOR,
            Mode::Connecting { .. } => views::GRAPH_EDITOR_CONNECT,
            Mode::NodeForm(_) => views::GRAPH_EDITOR_NODE_FORM,
            Mode::PartForm(_) => views::GRAPH_EDITOR_PART_FORM,
            Mode::EdgeForm(_) => views::GRAPH_EDITOR_EDGE_FORM,
            Mode::Links(_) => views::GRAPH_EDITOR_LINKS,
            Mode::ConfirmDelete { .. } => views::GRAPH_EDITOR_CONFIRM,
        }
    }

    /// The mode's keys through its stack, plus ` · saving…` while a write is in flight. The step
    /// form's list acts name the focused list's noun (M2 D14).
    #[must_use]
    pub fn hint_text(&self, keys: &Keys) -> String {
        let spec = match &self.mode {
            Mode::Canvas => HINT_CANVAS,
            Mode::Connecting { .. } => HINT_CONNECT,
            Mode::NodeForm(editing) => match editing.form.list_focus() {
                ListFocus::Candidates => HINT_NODE_FORM,
                ListFocus::Parts => HINT_NODE_FORM_PARTS,
            },
            Mode::PartForm(_) => HINT_PART_FORM,
            Mode::EdgeForm(_) => HINT_EDGE_FORM,
            Mode::Links(_) => HINT_LINKS,
            Mode::ConfirmDelete { .. } => HINT_CONFIRM,
        };
        let hint = keys.hint(self.stack(), spec);
        if self.busy() {
            format!("{hint} \u{b7} saving\u{2026}")
        } else {
            hint
        }
    }

    /// Whether a paste lands: a form's focused text row.
    #[must_use]
    pub fn takes_paste(&self) -> bool {
        match &self.mode {
            Mode::NodeForm(editing) => editing.form.focused() == NodeRow::Text,
            Mode::PartForm(editing) => editing.form.focused() == PartRow::Text,
            Mode::EdgeForm(form) => form.focused() != EdgeRow::Relation,
            _ => false,
        }
    }

    /// A bracketed paste into the focused text row. A form's other rows drop it; outside a form it
    /// is the host's.
    pub fn on_paste(&mut self, text: &str) -> Handled {
        match &mut self.mode {
            Mode::NodeForm(editing) => editing.form.on_paste(text),
            Mode::PartForm(editing) => editing.form.on_paste(text),
            Mode::EdgeForm(form) => form.on_paste(text),
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// One key (the host hands every key while the editor is open). A form's focused text row
    /// takes it first; then the mode's acts through its stack; a chord nothing takes is swallowed
    /// unless the modal global layer lets it through (`ctrl-c`, `F1`).
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> EditorKey {
        self.sync_canvas(ctx);
        if let Some(handled) = self.on_text_key(key, ctx) {
            return EditorKey::Handled(handled);
        }
        let stack = self.stack();
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(stack, chord) {
            if let Some(answer) = self.on_act(act, ctx) {
                return answer;
            }
        }
        EditorKey::Handled(modal_rest(stack, chord))
    }

    /// One mouse event on the canvas (D11), in Canvas mode only: a press or the wheel off the
    /// canvas is `Pass`; a drag or a release is the canvas's wherever it lands, so a gesture that
    /// left the canvas still ends.
    pub fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !matches!(self.mode, Mode::Canvas) {
            return Handled::Pass;
        }
        let Some(area) = self.canvas_area.get() else {
            return Handled::Pass;
        };
        let inside = area.contains(Position::new(mouse.column, mouse.row));
        let gesture = matches!(mouse.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_));
        if !inside && !gesture {
            return Handled::Pass;
        }
        self.sync_canvas(ctx);
        let event = self.canvas.get_mut().on_mouse(mouse);
        match event {
            Some(CanvasEvent::Select(id)) => self.select(id),
            Some(CanvasEvent::EditNode(id)) => {
                self.select(id);
                self.open_node_form();
            }
            Some(CanvasEvent::OpenLinks(edge)) => {
                if let Some(links) = Links::on_edge(&self.body, edge) {
                    self.mode = Mode::Links(links);
                    self.notice = None;
                }
            }
            Some(CanvasEvent::Connect { from, to }) => {
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                } else {
                    self.mode = Mode::EdgeForm(Box::new(EdgeForm::new(from, to)));
                    self.notice = None;
                }
            }
            None => {}
        }
        // The links list (and the step form) open on a press: that gesture's drag and release
        // never reach the canvas once its mode is gone.
        self.end_gesture_off_canvas();
        Handled::Consumed
    }

    /// The mouse went away mid-gesture.
    pub fn on_mouse_lost(&mut self) {
        self.canvas.get_mut().end_gesture();
    }

    /// MOD-141 R1 L2: once the editor has left the canvas, a live gesture is ended. Outside
    /// [`Mode::Canvas`] no drag or release reaches the canvas, and the host keeps the mouse on
    /// while the editor is open, so nothing else would end it.
    fn end_gesture_off_canvas(&mut self) {
        if !matches!(self.mode, Mode::Canvas) {
            self.canvas.get_mut().end_gesture();
        }
    }

    /// Every reply the host receives; the editor takes its own: its write's answer, and the
    /// agents, templates and settings it reads.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::GraphReplaced { outcome, .. } => match outcome {
                CasOutcome::Applied(body) if body.graph.id == self.graph() && self.busy() => {
                    self.applied(body);
                }
                CasOutcome::Stale(current) if current.graph.id == self.graph() && self.busy() => {
                    self.stale(current);
                }
                // Another graph's, or nothing pending.
                CasOutcome::Applied(_) | CasOutcome::Stale(_) => {}
            },
            StoreReply::Failed { request, message } if *request == "replace_graph" => {
                let Some(write) = self.history.failed() else {
                    return;
                };
                self.after_save = None;
                self.refuse(message);
                // MOD-141 R1 M1: the entry stays (the refusal may pass), but the registry may have
                // lost what it names: the lookups are read again, so the next try is judged on
                // them and dropped before it leaves. The catalogue goes to the host, which hands
                // its personas on.
                if matches!(write, Write::Undo(_) | Write::Redo(_)) {
                    ctx.request(StoreRequest::Catalogue(ctx.scope.clone()));
                    ctx.request(StoreRequest::Agents);
                }
            }
            StoreReply::Agents(agents) => {
                self.lookups.agents = Some(agents.clone());
                self.resync.set(true);
            }
            StoreReply::Templates(snapshot) => {
                self.lookups.templates = Some(scope_templates(snapshot, self.body.graph.scope));
            }
            StoreReply::Failed { request, message } if *request == "agents" => {
                self.notice = Some(Notice::Info(unavailable("agents", message)));
            }
            StoreReply::Failed { request, message } if *request == "templates" => {
                self.notice = Some(Notice::Info(unavailable("templates", message)));
            }
            // M2 D6: the limits on the header, the switch for the next step form, and the
            // canvas's markers, which need `max_fan_out`.
            StoreReply::GraphEditorSettings(settings) => {
                // Open risk 4: a step form opened before the first read (Kinds' `e` opens it
                // before the reads leave) opened with the switch off; that read reaches it.
                if self.settings.is_none() {
                    match &mut self.mode {
                        Mode::NodeForm(editing) => editing.form.switch_read(settings.compete),
                        Mode::PartForm(editing) => {
                            editing.node.form.switch_read(settings.compete);
                        }
                        _ => {}
                    }
                }
                self.settings = Some(**settings);
                self.resync.set(true);
            }
            StoreReply::Failed { request, message } if *request == "graph_editor_settings" => {
                self.notice = Some(Notice::Info(unavailable("run limits", message)));
            }
            _ => {}
        }
    }

    /// Draws over the whole `area` (D2): the header and its limits line (M2 D13), the canvas
    /// (and the open panel beside it), then the notice or the delete question.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let mut bottom = self.bottom_lines(area.width, ctx);
        bottom.truncate(NOTICE_ROWS);
        let heading = vec![
            Line::styled(self.header(ctx.keys()), theme.title),
            Line::styled(limits_row(self.settings.as_ref(), area.width), theme.dim),
        ];
        let [header, middle, foot] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(u16::try_from(bottom.len()).unwrap_or(0)),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(heading), header);
        let panel = match &self.mode {
            Mode::NodeForm(_) | Mode::PartForm(_) | Mode::EdgeForm(_) | Mode::Links(_) => true,
            Mode::Canvas | Mode::Connecting { .. } | Mode::ConfirmDelete { .. } => false,
        };
        if panel {
            let [canvas, side] = Layout::horizontal([
                Constraint::Min(CANVAS_MIN),
                Constraint::Length(PANEL_WIDTH.min(middle.width / 2)),
            ])
            .areas(middle);
            self.draw_canvas(frame, canvas, ctx);
            frame.render_widget(
                Paragraph::new(self.panel_lines(side.width, side.height, ctx)),
                side,
            );
        } else {
            self.draw_canvas(frame, middle, ctx);
        }
        frame.render_widget(Paragraph::new(bottom), foot);
    }

    /// The header: `{name} · {scope} graph · revision {n}`, then what undo would take back.
    fn header(&self, keys: &Keys) -> String {
        let graph = &self.body.graph;
        let scope = match graph.scope {
            GraphScope::Project(_) => "project graph",
            GraphScope::Workspace(_) => "workspace graph",
            GraphScope::Library => "library graph",
        };
        let mut header = format!(
            "{} \u{b7} {scope} \u{b7} revision {}",
            graph.name, graph.revision
        );
        if let Some(label) = self.history.undo_label() {
            match keys.label(views::GRAPH_EDITOR, Act::GraphEditorUndo) {
                Some(key) => header.push_str(&format!(" \u{b7} {key} undoes: {label}")),
                None => header.push_str(&format!(" \u{b7} undo: {label}")),
            }
        }
        header
    }

    /// The panel beside the canvas, `width` x `height`: the open form's or the links list's title,
    /// then its rows; the step form's notes under its rows (M2 D11), the rows scrolled to the
    /// focused one when the step form is taller than the panel.
    fn panel_lines(&self, width: u16, height: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let (title, rows) = match &self.mode {
            Mode::NodeForm(editing) => {
                let label = |act| ctx.keys().label(views::GRAPH_EDITOR_NODE_FORM, act);
                let (add, open) = (
                    label(Act::GraphEditorCandidateAdd),
                    label(Act::GraphEditorOpenPart),
                );
                let form = editing.form.layout(
                    &editing.base,
                    &self.lookups,
                    FormKeys {
                        add: add.as_deref(),
                        open: open.as_deref(),
                    },
                    width,
                    ctx.accent(),
                    theme,
                );
                let notes = self.note_lines(editing, width, theme);
                // The title takes the panel's first row.
                let rows = fitted(form, notes, height.saturating_sub(1), theme);
                (editing.form.title(&self.body), rows)
            }
            Mode::PartForm(editing) => {
                let step =
                    step_name(&self.body, editing.node.base.phase.id).unwrap_or("the new step");
                (
                    editing.form.title(step),
                    editing
                        .form
                        .lines(&self.lookups, width, ctx.accent(), theme),
                )
            }
            Mode::EdgeForm(form) => (
                form.title(&self.body),
                form.lines(width, ctx.accent(), theme),
            ),
            Mode::Links(links) => {
                let name = step_name(&self.body, links.node()).unwrap_or("a missing step");
                (
                    format!("links of {name}"),
                    links.lines(&self.body, width, ctx.accent(), theme),
                )
            }
            Mode::Canvas | Mode::Connecting { .. } | Mode::ConfirmDelete { .. } => {
                return Vec::new();
            }
        };
        let mut lines = vec![Line::styled(title, theme.title)];
        lines.extend(rows);
        lines
    }

    /// M2 D11 (B5): what a run of the step as the form holds it would be refused with today,
    /// `width` cells wide, `! ` before each note's first line. While the form does not build, the
    /// notes are the node's as it opened.
    fn note_lines(&self, editing: &NodeEditing, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let node = editing
            .form
            .build(&editing.base, &self.lookups)
            .unwrap_or_else(|_| editing.base.clone());
        let width = usize::from(width.saturating_sub(2)).max(1);
        let mut lines = Vec::new();
        for note in notes::notes(&node, self.max_fan_out()) {
            for (at, line) in wrapped(&note, width).iter().enumerate() {
                let lead = if at == 0 { "! " } else { "  " };
                lines.push(notice(&format!("{lead}{line}"), Tone::Error, theme));
            }
        }
        lines
    }

    /// `max_fan_out` once the settings read answered (M2 D11, C9): the notes skip the checks that
    /// need it until then.
    fn max_fan_out(&self) -> Option<u32> {
        self.settings.map(|settings| settings.bounds.max_fan_out)
    }

    /// The rows under the canvas: the delete question, else the notice, wrapped to `width`.
    fn bottom_lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let (text, tone) = match (&self.mode, &self.notice) {
            (Mode::ConfirmDelete { node }, _) => {
                let name = step_name(&self.body, *node).unwrap_or("a missing step");
                let answers = ctx.keys().hint(views::GRAPH_EDITOR_CONFIRM, HINT_CONFIRM);
                (delete_question(name, &answers), Tone::Error)
            }
            (Mode::Connecting { from }, None) => {
                let name = step_name(&self.body, *from).unwrap_or("a missing step");
                (connecting(name), Tone::Info)
            }
            (_, Some(Notice::Error(text))) => (text.clone(), Tone::Error),
            (_, Some(Notice::Info(text))) => (text.clone(), Tone::Info),
            (_, None) => return Vec::new(),
        };
        wrapped(&text, usize::from(width).max(1))
            .iter()
            .map(|line| notice(line, tone, ctx.theme))
            .collect()
    }

    /// Draws the canvas into `area`, synced first when anything changed.
    fn draw_canvas(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        self.sync_canvas(ctx);
        self.canvas.borrow_mut().render(frame, area);
        self.canvas_area.set(Some(area));
    }

    /// Rebuilds the canvas from the body, the selection and the agents when any changed; a step
    /// a run would refuse today is marked (M2 D11, D12).
    fn sync_canvas(&self, ctx: &Ctx<'_>) {
        if self.resync.replace(false) {
            let agents = self.lookups.agents.as_deref().unwrap_or(&[]);
            let max_fan_out = self.max_fan_out();
            let warned = self
                .body
                .nodes
                .iter()
                .filter(|node| !notes::notes(node, max_fan_out).is_empty())
                .map(|node| node.phase.id)
                .collect();
            let mut canvas = self.canvas.borrow_mut();
            canvas.set_warned(warned);
            canvas.sync(&self.body, self.selected, agents, ctx.theme);
        }
    }

    /// A form's focused text row takes `key` first; `None` when it passes it on (or no text row
    /// has the focus).
    fn on_text_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Option<Handled> {
        let (outcome, part) = match &mut self.mode {
            Mode::NodeForm(editing) if editing.form.focused() == NodeRow::Text => {
                (editing.form.on_text_key(key), false)
            }
            Mode::PartForm(editing) if editing.form.focused() == PartRow::Text => {
                (editing.form.on_text_key(key), true)
            }
            Mode::EdgeForm(form) if form.focused() != EdgeRow::Relation => {
                (form.on_text_key(key), false)
            }
            _ => return None,
        };
        match outcome {
            FieldOutcome::Consumed => {}
            // The part form's `Enter` keeps the part and its `Esc` drops it: back to the step form.
            FieldOutcome::Submit if part => self.keep_part(),
            FieldOutcome::Cancel if part => self.back_to_step_form(),
            FieldOutcome::Submit => self.save(ctx),
            FieldOutcome::Cancel => self.back_to_canvas(),
            FieldOutcome::Pass => return None,
        }
        Some(Handled::Consumed)
    }

    /// One act of the current mode's stack; `None` when this mode declines it.
    fn on_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> Option<EditorKey> {
        let handled = match self.mode {
            Mode::Canvas => {
                let answer = self.on_canvas_act(act, ctx);
                self.end_gesture_off_canvas();
                return answer;
            }
            Mode::Connecting { from } => self.on_connecting_act(from, act),
            Mode::NodeForm(_) => self.on_node_form_act(act, ctx),
            Mode::PartForm(_) => self.on_part_form_act(act),
            Mode::EdgeForm(_) => self.on_edge_form_act(act, ctx),
            Mode::Links(_) => self.on_links_act(act, ctx),
            Mode::ConfirmDelete { node } => self.on_confirm_act(node, act, ctx),
        };
        handled.then_some(EditorKey::Handled(Handled::Consumed))
    }

    /// The canvas's acts.
    fn on_canvas_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> Option<EditorKey> {
        match act {
            Act::GraphEditorAdd => {
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                } else {
                    let base = model::draft_node(&self.body, self.selected, Utc::now());
                    let form = NodeForm::new(
                        &base,
                        NodeTarget::New {
                            after: self.selected,
                        },
                        self.compete(),
                    );
                    self.mode = Mode::NodeForm(Box::new(NodeEditing { form, base }));
                    self.notice = None;
                }
            }
            Act::GraphEditorEdit => self.open_node_form(),
            Act::GraphEditorDelete => {
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                } else if let Some(node) = self.selected_step() {
                    self.mode = Mode::ConfirmDelete { node };
                    self.notice = None;
                }
            }
            Act::GraphEditorConnect => {
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                } else if let Some(from) = self.selected_step() {
                    if self.is_palette(from) {
                        self.refuse(PALETTE_HAS_NO_LINKS);
                    } else {
                        self.mode = Mode::Connecting { from };
                        self.notice = None;
                    }
                }
            }
            Act::GraphEditorOpenLinks => {
                if let Some(node) = self.selected_step() {
                    self.mode = Mode::Links(Links::new(node));
                    self.notice = None;
                }
            }
            Act::GraphEditorUndo => match self.history.begin_undo(&self.body) {
                Ok(previous) => self.step_back_or_forth(previous, ctx),
                Err(refusal) => self.refuse(refusal.sentence()),
            },
            Act::GraphEditorRedo => match self.history.begin_redo(&self.body) {
                Ok(next) => self.step_back_or_forth(next, ctx),
                Err(refusal) => self.refuse(refusal.sentence()),
            },
            Act::GraphEditorClose => {
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                } else {
                    return Some(EditorKey::Close);
                }
            }
            _ => {
                if !self.on_view_act(act) {
                    return None;
                }
            }
        }
        Some(EditorKey::Handled(Handled::Consumed))
    }

    /// The selection and zoom acts the canvas and connecting share; `false` for any other.
    fn on_view_act(&mut self, act: Act) -> bool {
        let direction = match act {
            Act::GraphEditorLeft => Direction::Left,
            Act::GraphEditorRight => Direction::Right,
            Act::GraphEditorUp => Direction::Up,
            Act::GraphEditorDown => Direction::Down,
            Act::GraphEditorZoomIn => {
                self.canvas.get_mut().zoom_in();
                return true;
            }
            Act::GraphEditorZoomOut => {
                self.canvas.get_mut().zoom_out();
                return true;
            }
            Act::GraphEditorFit => {
                self.canvas.get_mut().fit();
                return true;
            }
            _ => return false,
        };
        if let Some(id) = self.canvas.get_mut().select_toward(direction) {
            self.selected = Some(id);
            self.resync.set(true);
        }
        true
    }

    /// Connecting's acts: the directions pick the target, `edit` opens the link form on it,
    /// `close` gives up.
    fn on_connecting_act(&mut self, from: PhaseId, act: Act) -> bool {
        match act {
            Act::GraphEditorEdit => match self.selected_step() {
                Some(to) if to == from => self.refuse(model::SELF_LINK),
                Some(to) => {
                    self.mode = Mode::EdgeForm(Box::new(EdgeForm::new(from, to)));
                    self.notice = None;
                }
                None => {}
            },
            Act::GraphEditorClose => {
                self.mode = Mode::Canvas;
                self.notice = Some(Notice::Info(NOT_CONNECTED.to_owned()));
            }
            _ => return self.on_view_act(act),
        }
        true
    }

    /// The step form's acts, once its focused text row passed the key on.
    fn on_node_form_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> bool {
        let Mode::NodeForm(editing) = &mut self.mode else {
            return false;
        };
        let form = &mut editing.form;
        let result = match act {
            Act::FormNextField => {
                form.next_field();
                Ok(())
            }
            Act::FormPrevField => {
                form.prev_field();
                Ok(())
            }
            Act::ChoicePrev | Act::ChoiceNext => form.cycle(act == Act::ChoiceNext, &self.lookups),
            // M2 D14: the four list acts work on the focused list, candidates or parts.
            Act::GraphEditorCandidateAdd => match form.list_focus() {
                ListFocus::Candidates => form.candidate_add(&self.lookups),
                ListFocus::Parts => return self.open_part(PartOpen::New),
            },
            Act::GraphEditorOpenPart => return self.open_part(PartOpen::Focused),
            Act::GraphEditorCandidateRemove => form.list_remove(),
            Act::GraphEditorCandidateUp => form.list_move(true),
            Act::GraphEditorCandidateDown => form.list_move(false),
            Act::FieldSubmit => {
                self.save(ctx);
                return true;
            }
            Act::FieldCancel => {
                self.back_to_canvas();
                return true;
            }
            _ => return false,
        };
        if let Err(refusal) = result {
            self.refuse(&refusal);
        }
        true
    }

    /// The step form's part form (M2 D9): [`PartOpen::New`] a blank part, [`PartOpen::Focused`]
    /// the focused one; the step form waits under it. What the step form refuses is said and it
    /// stays open.
    fn open_part(&mut self, open: PartOpen) -> bool {
        let Mode::NodeForm(editing) = &self.mode else {
            return false;
        };
        match editing.form.open_part(open) {
            Ok(form) => {
                self.mode = match core::mem::replace(&mut self.mode, Mode::Canvas) {
                    Mode::NodeForm(node) => Mode::PartForm(Box::new(PartEditing { form, node })),
                    other => other,
                };
                self.notice = None;
            }
            Err(refusal) => self.refuse(&refusal),
        }
        true
    }

    /// The part form's acts, once its focused text row passed the key on: `Enter` keeps the
    /// part, `Esc` drops the edit; neither writes.
    fn on_part_form_act(&mut self, act: Act) -> bool {
        let Mode::PartForm(editing) = &mut self.mode else {
            return false;
        };
        match act {
            Act::FormNextField => editing.form.next_field(),
            Act::FormPrevField => editing.form.prev_field(),
            Act::ChoicePrev | Act::ChoiceNext => {
                editing.form.cycle(act == Act::ChoiceNext, &self.lookups);
            }
            Act::FieldSubmit => self.keep_part(),
            Act::FieldCancel => self.back_to_step_form(),
            _ => return false,
        }
        true
    }

    /// The part form's `Enter`: the step form takes the part (nothing is written until the step
    /// is saved) and has the focus again. A refusal (a key that is empty or taken) is said and
    /// the part form stays.
    fn keep_part(&mut self) {
        let editing = match core::mem::replace(&mut self.mode, Mode::Canvas) {
            Mode::PartForm(editing) => editing,
            other => {
                self.mode = other;
                return;
            }
        };
        let PartEditing { form, mut node } = *editing;
        match node.form.put_part(&form) {
            Ok(()) => {
                self.mode = Mode::NodeForm(node);
                self.notice = None;
            }
            Err(refusal) => {
                self.mode = Mode::PartForm(Box::new(PartEditing { form, node }));
                self.refuse(&refusal);
            }
        }
    }

    /// The part form's `Esc`: back to the step form as it was, the notice cleared.
    fn back_to_step_form(&mut self) {
        self.mode = match core::mem::replace(&mut self.mode, Mode::Canvas) {
            Mode::PartForm(editing) => Mode::NodeForm(editing.node),
            other => other,
        };
        self.notice = None;
    }

    /// The link form's acts, once its focused text row passed the key on.
    fn on_edge_form_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> bool {
        let Mode::EdgeForm(form) = &mut self.mode else {
            return false;
        };
        match act {
            Act::FormNextField => form.next_field(),
            Act::FormPrevField => form.prev_field(),
            Act::ChoicePrev | Act::ChoiceNext => form.cycle(act == Act::ChoiceNext),
            Act::FieldSubmit => self.save(ctx),
            Act::FieldCancel => self.back_to_canvas(),
            _ => return false,
        }
        true
    }

    /// The links list's acts. Cycling a relation opens the link form on the link with the next
    /// relation chosen: nothing is sent until it is saved, one write per edit.
    fn on_links_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> bool {
        let busy = self.busy();
        let Mode::Links(links) = &mut self.mode else {
            return false;
        };
        let edge = links
            .selected(&self.body)
            .and_then(|id| self.body.edges.iter().find(|edge| edge.id == id));
        match act {
            Act::ListDown => links.down(&self.body),
            Act::ListUp => links.up(),
            // The reply to the write in flight would close the form these open.
            Act::ChoicePrev | Act::ChoiceNext | Act::LinksEdit if busy => {
                self.refuse(history::IN_FLIGHT);
            }
            Act::ChoicePrev | Act::ChoiceNext => {
                if let Some(edge) = edge {
                    let kind = cycle_kind(edge.kind, act == Act::ChoiceNext);
                    let form = EdgeForm::edit(edge).with_kind(kind);
                    self.mode = Mode::EdgeForm(Box::new(form));
                    self.notice = None;
                }
            }
            Act::LinksEdit => {
                if let Some(edge) = edge {
                    self.mode = Mode::EdgeForm(Box::new(EdgeForm::edit(edge)));
                    self.notice = None;
                }
            }
            Act::LinksDelete => {
                if let Some(edge) = edge {
                    let label = format!("removing the link {}", self.ends(edge.from, edge.to));
                    let id = edge.id;
                    if self.busy() {
                        self.refuse(history::IN_FLIGHT);
                    } else {
                        match model::remove_edge(&self.body, id) {
                            Ok(next) => {
                                self.write(label, next, ctx);
                            }
                            Err(refusal) => self.refuse(&refusal),
                        }
                    }
                }
            }
            Act::LinksClose => self.back_to_canvas(),
            _ => return false,
        }
        true
    }

    /// The delete question's answers: `yes` deletes in one write (the links rewired, D10), `no`
    /// keeps the step.
    fn on_confirm_act(&mut self, node: PhaseId, act: Act, ctx: &mut Ctx<'_>) -> bool {
        match act {
            Act::ConfirmYes => {
                self.mode = Mode::Canvas;
                if self.busy() {
                    self.refuse(history::IN_FLIGHT);
                    return true;
                }
                let name = step_name(&self.body, node).unwrap_or("a missing step");
                let label = format!("deleting {name}");
                match model::delete_node(&self.body, node) {
                    Ok(next) => {
                        self.write(label, next, ctx);
                    }
                    Err(refusal) => self.refuse(&refusal),
                }
            }
            Act::ConfirmNo => self.mode = Mode::Canvas,
            _ => return false,
        }
        true
    }

    /// `Enter` in a form: the form's body, the store's rules, then one write.
    fn save(&mut self, ctx: &mut Ctx<'_>) {
        if self.busy() {
            self.refuse(history::IN_FLIGHT);
            return;
        }
        let built = match &self.mode {
            Mode::NodeForm(editing) => self.node_write(editing),
            Mode::EdgeForm(form) => self.edge_write(form),
            _ => return,
        };
        match built {
            Ok((label, next, added)) => {
                if self.write(label, next, ctx) {
                    self.after_save = added;
                }
            }
            Err(refusal) => self.refuse(&refusal),
        }
    }

    /// The step form's write: its label, the next body and, for an add, the new step.
    fn node_write(
        &self,
        editing: &NodeEditing,
    ) -> Result<(String, GraphBody, Option<PhaseId>), String> {
        let node = editing.form.build(&editing.base, &self.lookups)?;
        match editing.form.target() {
            NodeTarget::New { after } => {
                let label = format!("adding {}", node.phase.name);
                let id = node.phase.id;
                Ok((label, model::add_after(&self.body, after, node)?, Some(id)))
            }
            NodeTarget::Edit(_) => {
                let label = format!("editing {}", editing.base.phase.name);
                Ok((label, model::replace_node(&self.body, node)?, None))
            }
        }
    }

    /// The link form's write: its label and the next body.
    fn edge_write(&self, form: &EdgeForm) -> Result<(String, GraphBody, Option<PhaseId>), String> {
        let spec = form.build()?;
        match form.target() {
            EdgeTarget::New { from, to } => {
                let label = format!("linking {}", self.ends(from, to));
                Ok((label, model::connect(&self.body, from, to, spec)?, None))
            }
            EdgeTarget::Edit(id) => {
                let edge = self
                    .body
                    .edges
                    .iter()
                    .find(|edge| edge.id == id)
                    .ok_or_else(|| model::NO_SUCH_LINK.to_owned())?;
                let label = format!("changing the link {}", self.ends(edge.from, edge.to));
                Ok((label, model::edit_edge(&self.body, id, spec)?, None))
            }
        }
    }

    /// One edit: the store's rules first (D5), so a body they refuse never leaves; then the
    /// history takes the write and it goes out. `false` when refused.
    fn write(&mut self, label: String, next: GraphBody, ctx: &Ctx<'_>) -> bool {
        if let Some(refusal) = model::validate(&next, &self.lookups) {
            self.refuse(&refusal);
            return false;
        }
        if let Err(refusal) = self.history.begin_edit(label, &self.body) {
            self.refuse(refusal.sentence());
            return false;
        }
        self.notice = None;
        self.send(next, ctx);
        true
    }

    /// An undo or redo the history began (MOD-141 R1 M1): the store's rules first, as for an edit
    /// (D5), since the registry may have lost what the entry names since it was pushed. A body
    /// they refuse never leaves: the entry is dropped from the history, not put back, and the
    /// notice says so.
    fn step_back_or_forth(&mut self, body: GraphBody, ctx: &Ctx<'_>) {
        if let Some(refusal) = model::validate(&body, &self.lookups) {
            if let Some(write) = self.history.dropped() {
                self.refuse(&cannot(&write, &refusal));
            }
            return;
        }
        self.notice = None;
        self.send(body, ctx);
    }

    /// The one request every write is (D4): the whole body, compare-and-set on the revision the
    /// editor shows. The editor keeps showing the current body until the reply.
    fn send(&self, next: GraphBody, ctx: &Ctx<'_>) {
        ctx.request(StoreRequest::ReplaceGraph {
            scope: ctx.scope.clone(),
            graph: self.body.graph.id,
            expected_revision: self.body.graph.revision,
            body: Box::new(next),
        });
    }

    /// `Applied`: the stored body is adopted, the history moves, the notice says what landed.
    fn applied(&mut self, body: &GraphBody) {
        self.body = body.clone();
        self.notice = self.history.applied().map(|write| {
            Notice::Info(match write {
                Write::Edit(label) => saved(&label),
                Write::Undo(label) => undid(&label),
                Write::Redo(label) => redid(&label),
            })
        });
        match &mut self.mode {
            // A links delete stays in the list.
            Mode::Links(links) if step_name(&self.body, links.node()).is_some() => {
                links.clamp(&self.body);
            }
            _ => self.mode = Mode::Canvas,
        }
        let added = self.after_save.take();
        self.reselect(added);
    }

    /// `Stale` (D4): the body as it is now replaces the shown one, undo and redo go.
    fn stale(&mut self, current: &GraphBody) {
        self.body = current.clone();
        self.history.stale();
        self.after_save = None;
        self.mode = Mode::Canvas;
        self.notice = Some(Notice::Error(STALE_RELOADED.to_owned()));
        self.reselect(None);
    }

    /// After the body changed: `preferred` when it is in the body, else the selection when it
    /// still is, else the first step.
    fn reselect(&mut self, preferred: Option<PhaseId>) {
        let present = |id: &PhaseId| step_name(&self.body, *id).is_some();
        self.selected = preferred
            .filter(present)
            .or_else(|| self.selected.filter(present))
            .or_else(|| first_step(&self.body));
        self.resync.set(true);
    }

    /// Back to the canvas, the notice cleared.
    fn back_to_canvas(&mut self) {
        self.mode = Mode::Canvas;
        self.notice = None;
    }

    /// The selected step, or the refusal saying there is none.
    fn selected_step(&mut self) -> Option<PhaseId> {
        if self.selected.is_none() {
            self.refuse(NO_STEP_SELECTED);
        }
        self.selected
    }

    /// Whether step `id` is a palette step.
    fn is_palette(&self, id: PhaseId) -> bool {
        self.body
            .nodes
            .iter()
            .any(|node| node.phase.id == id && node.phase.palette)
    }

    /// `{from} → {to}` by step name.
    fn ends(&self, from: PhaseId, to: PhaseId) -> String {
        let name = |id| step_name(&self.body, id).unwrap_or("a missing step");
        format!("{} \u{2192} {}", name(from), name(to))
    }

    /// A refusal on the notice row.
    fn refuse(&mut self, text: &str) {
        self.notice = Some(Notice::Error(text.to_owned()));
    }
}

/// The first flow step in position order, else none.
fn first_step(body: &GraphBody) -> Option<PhaseId> {
    body.nodes
        .iter()
        .filter(|node| !node.phase.palette)
        .min_by_key(|node| node.phase.position)
        .map(|node| node.phase.id)
}

/// The templates a graph of `scope` may use (T5's rule): a project graph its project's, every
/// version; a workspace or library graph every scope project's, one row per `(name, version)`.
fn scope_templates(snapshot: &TemplatesSnapshot, scope: GraphScope) -> Vec<PromptTemplate> {
    let rows = snapshot
        .projects
        .iter()
        .filter(|project| match scope {
            GraphScope::Project(id) => project.project_id == id,
            GraphScope::Workspace(_) | GraphScope::Library => true,
        })
        .flat_map(|project| project.templates.iter());
    let mut seen = HashSet::new();
    rows.filter(|row| seen.insert((row.name.clone(), row.version)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EdgeId, NodeBody, PersonaId, PhasePart, ProjectRef, Split, StepGraph, StepGraphEdge,
        StepGraphPhase,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{
        MemStore, WriteStore, loop_edge_runs_forward, references_no_row, reserved_phase_name,
    };
    use htui_store::Backend;
    use ratatui::backend::TestBackend;
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use uuid::Uuid;

    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton};

    use super::part_form::PartTarget;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keys::{KeyChord, load_str};
    use crate::store_worker::{self, Origin};
    use crate::ui::Theme;
    use crate::ui::tabs::settings::SettingsTab;

    /// Everything [`Ctx::new`] borrows, as `testkit::SectionBench` holds it, for the editor alone.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        theme: Theme,
        emit: Emit,
        keys: Keys,
    }

    impl Bench {
        /// The demo fixture's first workspace (`Graphics`), the compiled keys.
        async fn new() -> Self {
            let workspaces = MemStore::demo()
                .workspaces()
                .await
                .expect("the memory store never fails");
            Self {
                scope: Scope::from_workspace(workspaces.first().expect("a workspace")),
                projects: Vec::new(),
                top_bar: TopBarState::default(),
                theme: Theme::default(),
                emit: Emit::default(),
                keys: Keys::compiled().clone(),
            }
        }

        fn with_keys(mut self, keys: Keys) -> Self {
            self.keys = keys;
            self
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keys,
                &self.theme,
                Origin::Tab(SettingsTab::ID),
                &self.emit,
            )
        }

        /// One chord, written as `KeyChord::parse` reads it.
        fn key(&self, editor: &mut GraphEditor, chord: &str) -> EditorKey {
            let parsed = KeyChord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is no chord"));
            let mut ctx = self.ctx();
            editor.on_key(parsed.to_event(), &mut ctx)
        }

        /// Several chords in order.
        fn keys(&self, editor: &mut GraphEditor, chords: &[&str]) {
            for chord in chords {
                self.key(editor, chord);
            }
        }

        /// Every character of `text` as its own key.
        fn typed(&self, editor: &mut GraphEditor, text: &str) {
            for character in text.chars() {
                self.key(editor, &character.to_string());
            }
        }

        fn reply(&self, editor: &mut GraphEditor, reply: &StoreReply) {
            let mut ctx = self.ctx();
            editor.on_reply(reply, &mut ctx);
        }

        fn drained(&self) -> Vec<Action> {
            self.emit.take()
        }

        /// The store requests emitted since the last call.
        fn requests(&self) -> Vec<StoreRequest> {
            self.drained()
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(request) => Some(request),
                    _ => None,
                })
                .collect()
        }

        /// The one `ReplaceGraph` emitted since the last call: graph, revision, body.
        fn write(&self) -> (StepGraphId, i64, GraphBody, StoreRequest) {
            let mut requests = self.requests();
            assert_eq!(requests.len(), 1, "one write: {requests:?}");
            let request = requests.remove(0);
            match &request {
                StoreRequest::ReplaceGraph {
                    scope,
                    graph,
                    expected_revision,
                    body,
                } => {
                    assert_eq!(scope, &self.scope);
                    (
                        *graph,
                        *expected_revision,
                        (**body).clone(),
                        request.clone(),
                    )
                }
                other => panic!("not a write: {other:?}"),
            }
        }

        fn hint(&self, editor: &GraphEditor) -> String {
            editor.hint_text(&self.keys)
        }

        /// The editor drawn into 100x30, as text.
        fn render(&self, editor: &GraphEditor) -> String {
            self.render_in(editor, 100, 30)
        }

        /// The editor drawn into `width` x `height`, as text. The shell's 100x30 gives it 98x27
        /// (the Kinds pane's border, the top bar and the hint row).
        fn render_in(&self, editor: &GraphEditor, width: u16, height: u16) -> String {
            let area = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::with_options(
                TestBackend::new(width, height),
                TerminalOptions {
                    viewport: Viewport::Fixed(area),
                },
            )
            .expect("a test terminal");
            let ctx = self.ctx();
            terminal
                .draw(|frame| editor.render(frame, frame.area(), &ctx))
                .expect("the editor draws");
            let buffer = terminal.backend().buffer();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }

    /// Serves `request` on `backend` as the worker would.
    async fn served(backend: &Backend, request: &StoreRequest) -> StoreReply {
        store_worker::serve(backend, request).await
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    fn phase(n: u128) -> PhaseId {
        PhaseId::from_uuid(Uuid::from_u128(0x0141_0600 + n))
    }

    fn graph_id(n: u128) -> StepGraphId {
        StepGraphId::from_uuid(Uuid::from_u128(0x0141_0700 + n))
    }

    /// A step `name` with a template of its name.
    fn node(graph: StepGraphId, id: PhaseId, position: i32, name: &str) -> NodeBody {
        let seed = PhaseSeed {
            name: "step",
            input_kinds: &[],
            gate_hard: false,
        };
        NodeBody {
            phase: StepGraphPhase {
                name: name.to_owned(),
                template_name: name.to_owned(),
                output_kind: name.to_owned(),
                ..phase_row(id, graph, position, &seed, now())
            },
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    fn edge(graph: StepGraphId, n: u128, from: PhaseId, to: PhaseId) -> StepGraphEdge {
        StepGraphEdge {
            id: EdgeId::from_uuid(Uuid::from_u128(0x0141_0800 + n)),
            graph_id: graph,
            from,
            to,
            kind: EdgeKind::Next,
            predicate: None,
            loop_limit: None,
        }
    }

    /// `research -next-> plan -next-> review`, graph `n` of `scope` at `revision`.
    fn flow(n: u128, scope: GraphScope, revision: i64) -> GraphBody {
        let graph = graph_id(n);
        GraphBody {
            graph: StepGraph {
                id: graph,
                scope,
                name: "flow".to_owned(),
                description: String::new(),
                is_override: false,
                revision,
                created_at: now(),
                updated_at: now(),
            },
            nodes: vec![
                node(graph, phase(1), 0, "research"),
                node(graph, phase(2), 1, "plan"),
                node(graph, phase(3), 2, "review"),
            ],
            edges: vec![
                edge(graph, 1, phase(1), phase(2)),
                edge(graph, 2, phase(2), phase(3)),
            ],
        }
    }

    /// An editor on [`flow`] in the library at `revision`, its reads drained.
    fn library(bench: &Bench, revision: i64) -> GraphEditor {
        let (editor, _) = GraphEditor::open(
            flow(0, GraphScope::Library, revision),
            Vec::new(),
            &bench.scope,
        );
        editor
    }

    /// The names of `body`'s steps.
    fn names(body: &GraphBody) -> Vec<&str> {
        body.nodes
            .iter()
            .map(|node| node.phase.name.as_str())
            .collect()
    }

    fn notice_text(editor: &GraphEditor) -> Option<&str> {
        match &editor.notice {
            Some(Notice::Info(text) | Notice::Error(text)) => Some(text),
            None => None,
        }
    }

    fn applied(body: GraphBody) -> StoreReply {
        StoreReply::GraphReplaced {
            outcome: CasOutcome::Applied(Box::new(body)),
            catalogue: Some(Box::new(crate::catalogue::CatalogueSnapshot {
                projects: Vec::new(),
                personas: Vec::new(),
            })),
        }
    }

    fn stale(body: GraphBody) -> StoreReply {
        StoreReply::GraphReplaced {
            outcome: CasOutcome::Stale(Box::new(body)),
            catalogue: Some(Box::new(crate::catalogue::CatalogueSnapshot {
                projects: Vec::new(),
                personas: Vec::new(),
            })),
        }
    }

    /// `body` as the store answers a write of it: one revision on.
    fn stored(mut body: GraphBody) -> GraphBody {
        body.graph.revision += 1;
        body
    }

    /// The demo store's analysis graph of `vulkan` and the persona registry, from the store.
    async fn demo(backend: &Backend, bench: &Bench) -> (GraphBody, Vec<PersonaSummary>) {
        let StoreReply::GraphBody(Some(body)) =
            served(backend, &StoreRequest::GraphBody(ids::GRAPH_VULKAN_ANA)).await
        else {
            panic!("the analysis graph reads");
        };
        let StoreReply::Catalogue(snapshot) =
            served(backend, &StoreRequest::Catalogue(bench.scope.clone())).await
        else {
            panic!("the catalogue reads");
        };
        (*body, snapshot.personas.clone())
    }

    /// The settings read's answer (M2 D6): the code defaults, the compete switch as given.
    fn settings(compete: bool) -> StoreReply {
        StoreReply::GraphEditorSettings(Box::new(EditorSettings {
            bounds: htui_orch::graph::run_bounds(&std::collections::BTreeMap::new()),
            compete,
        }))
    }

    /// Line `n` of the editor drawn into 100x30.
    fn drawn_line(bench: &Bench, editor: &GraphEditor, n: usize) -> String {
        bench
            .render(editor)
            .lines()
            .nth(n)
            .unwrap_or_default()
            .to_owned()
    }

    #[tokio::test]
    async fn opening_asks_for_agents_templates_and_settings() {
        let bench = Bench::new().await;
        let (editor, requests) =
            GraphEditor::open(flow(0, GraphScope::Library, 0), Vec::new(), &bench.scope);
        let reads = vec![
            StoreRequest::Agents,
            StoreRequest::Templates(bench.scope.clone()),
            StoreRequest::GraphEditorSettings,
        ];
        let reads = format!("{reads:?}");
        assert_eq!(format!("{requests:?}"), reads);
        assert_eq!(format!("{:?}", editor.wants_requests(&bench.scope)), reads);
        assert_eq!(editor.graph(), graph_id(0));
        assert_eq!(editor.selected, Some(phase(1)), "the first flow step");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(!editor.busy());
    }

    /// M2 D13: the header's second line is the run limits resolve freezes, read-only; before the
    /// settings read answers it says so (D6).
    #[tokio::test]
    async fn the_header_shows_the_limits_once_read() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        assert_eq!(drawn_line(&bench, &editor, 1), LIMITS_NOT_READ);
        bench.reply(&mut editor, &settings(false));
        assert_eq!(
            drawn_line(&bench, &editor, 1),
            "limits \u{b7} fan-out 4 \u{b7} agents 8 \u{b7} plan revisions 3 \u{b7} parallel 4 \
             \u{b7} child runs 8 \u{b7} run-time changes ask"
        );
        assert!(
            drawn_line(&bench, &editor, 0).starts_with("flow \u{b7} library graph"),
            "the first line is the graph's"
        );
        // Narrower, it is clipped to its one row (Open risk 5), a whole `label value` at a time
        // and ` · …` saying so: no number is cut in half.
        let read = settings(false);
        let StoreReply::GraphEditorSettings(read) = &read else {
            unreachable!("a settings answer");
        };
        let whole = limits_line(Some(read));
        assert_eq!(limits_row(Some(read), 99), whole);
        assert_eq!(
            limits_row(Some(read), 98),
            "limits \u{b7} fan-out 4 \u{b7} agents 8 \u{b7} plan revisions 3 \u{b7} parallel 4 \
             \u{b7} child runs 8 \u{b7} \u{2026}"
        );
        assert_eq!(
            limits_row(Some(read), 30),
            "limits \u{b7} fan-out 4 \u{b7} \u{2026}"
        );
        assert_eq!(limits_row(None, 60), LIMITS_NOT_READ);
        assert_eq!(limits_row(None, 10), "\u{2026}");
        for width in 0..=110 {
            let row = limits_row(Some(read), width);
            assert!(
                cell_width(&row) <= usize::from(width.max(1)),
                "{width}: {row}"
            );
            let kept = row.trim_end_matches(" \u{b7} \u{2026}");
            assert!(
                row == whole || row == "\u{2026}" || whole.starts_with(&format!("{kept} \u{b7} ")),
                "{width}: {row}"
            );
        }
        // In the shell's 98 columns the header stays two rows: the third is the canvas's.
        let screen = bench.render_in(&editor, 98, 27);
        let rows: Vec<&str> = screen.lines().collect();
        assert_eq!(rows[1], limits_row(Some(read), 98));
        assert!(!screen.contains("run-time changes"), "{screen}");
    }

    /// M2 D6: a refused settings read is said like the other lookups', and the limits stay unread.
    #[tokio::test]
    async fn a_refused_settings_read_says_so() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.reply(
            &mut editor,
            &StoreReply::Failed {
                request: "graph_editor_settings",
                message: "refused".to_owned(),
            },
        );
        assert_eq!(
            notice_text(&editor),
            Some("run limits unavailable: refused")
        );
        assert_eq!(drawn_line(&bench, &editor, 1), LIMITS_NOT_READ);
        assert!(bench.requests().is_empty(), "nothing asked again");
    }

    /// The split the step form holds after `e`, the focus on the split row and `presses` times
    /// `l`; the form is cancelled after.
    fn split_after(bench: &Bench, editor: &mut GraphEditor, presses: usize) -> Split {
        bench.key(editor, "e");
        // M1's 13 rows, the candidates' placeholder, the deadline and the palette.
        bench.keys(editor, &["tab"; 16]);
        bench.keys(editor, &vec!["l"; presses]);
        let Mode::NodeForm(editing) = &editor.mode else {
            panic!("the step form is open");
        };
        let split = editing
            .form
            .build(&editing.base, &editor.lookups)
            .expect("the form builds")
            .phase
            .split;
        bench.key(editor, "esc");
        split
    }

    /// M2 D5, D7: compete is offered only once the settings read says the switch is on; until
    /// then it is off. A form already open keeps the switch it opened with.
    #[tokio::test]
    async fn compete_is_offered_once_the_switch_reads_on() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        assert_eq!(split_after(&bench, &mut editor, 1), Split::Divide);
        assert_eq!(
            split_after(&bench, &mut editor, 2),
            Split::None,
            "not read yet: off"
        );
        bench.reply(&mut editor, &settings(false));
        assert_eq!(split_after(&bench, &mut editor, 2), Split::None, "off");

        // A form open while the switch reads on keeps it off.
        bench.key(&mut editor, "e");
        bench.reply(&mut editor, &settings(true));
        bench.keys(&mut editor, &["tab"; 16]);
        bench.keys(&mut editor, &["l", "l"]);
        let Mode::NodeForm(editing) = &editor.mode else {
            panic!("the step form is open");
        };
        let built = editing.form.build(&editing.base, &editor.lookups);
        assert_eq!(built.map(|node| node.phase.split), Ok(Split::None));
        bench.key(&mut editor, "esc");

        assert_eq!(split_after(&bench, &mut editor, 2), Split::Compete, "on");
        assert_eq!(split_after(&bench, &mut editor, 3), Split::None);
    }

    #[tokio::test]
    async fn each_scope_saves_its_own_graph_and_revision() {
        let bench = Bench::new().await;
        for (n, scope, revision) in [
            (1, GraphScope::Project(ids::PROJECT_VULKAN), 4),
            (2, GraphScope::Workspace(ids::WORKSPACE_GRAPHICS), 7),
            (3, GraphScope::Library, 9),
        ] {
            let (mut editor, _) =
                GraphEditor::open(flow(n, scope, revision), Vec::new(), &bench.scope);
            bench.key(&mut editor, "e");
            bench.typed(&mut editor, "2");
            bench.key(&mut editor, "enter");
            let (graph, expected, body, _) = bench.write();
            assert_eq!(graph, graph_id(n), "{scope:?}");
            assert_eq!(expected, revision, "{scope:?}");
            assert_eq!(names(&body), ["research2", "plan", "review"], "{scope:?}");
            assert!(editor.busy());
        }
    }

    #[tokio::test]
    async fn a_refused_body_never_sends_a_request() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.key(&mut editor, "e");
        bench.keys(&mut editor, &["backspace"; 8]);
        bench.typed(&mut editor, "judge");
        bench.key(&mut editor, "enter");
        assert_eq!(
            notice_text(&editor),
            Some(reserved_phase_name("judge").as_str())
        );
        assert_eq!(
            editor.stack(),
            views::GRAPH_EDITOR_NODE_FORM,
            "the form stays open"
        );
        bench.key(&mut editor, "esc");

        // A `fails` link that runs forward: research -> plan.
        bench.keys(&mut editor, &["c", "j", "enter"]);
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_EDGE_FORM);
        bench.keys(&mut editor, &["l", "l", "l", "l", "tab"]);
        bench.typed(&mut editor, "1");
        bench.key(&mut editor, "enter");
        assert_eq!(
            notice_text(&editor),
            Some(loop_edge_runs_forward(EdgeKind::Fails, "research", "plan", graph_id(0)).as_str())
        );
        assert!(
            !bench
                .drained()
                .iter()
                .any(|action| matches!(action, Action::Store(_))),
            "nothing sent"
        );
        assert!(!editor.busy());
    }

    #[tokio::test]
    async fn a_second_edit_is_refused_while_one_is_in_flight() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let _ = bench.write();
        // Enter again re-sends nothing.
        bench.key(&mut editor, "enter");
        assert_eq!(notice_text(&editor), Some(history::IN_FLIGHT));
        bench.key(&mut editor, "esc");
        for chord in ["a", "x", "c", "u", "U"] {
            editor.notice = None;
            bench.key(&mut editor, chord);
            assert_eq!(notice_text(&editor), Some(history::IN_FLIGHT), "{chord}");
            assert_eq!(editor.stack(), views::GRAPH_EDITOR, "{chord}");
        }
        assert!(bench.requests().is_empty());
        assert!(bench.hint(&editor).ends_with(" \u{b7} saving\u{2026}"));
    }

    #[tokio::test]
    async fn no_form_opens_while_a_write_is_in_flight() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        // plan's links list, `plan -> review` removed: a write in flight, list stays.
        bench.keys(&mut editor, &["j", "i", "d"]);
        let (_, _, sent, _) = bench.write();
        assert!(editor.busy());
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_LINKS);
        for chord in ["e", "enter", "h", "l"] {
            editor.notice = None;
            bench.key(&mut editor, chord);
            assert_eq!(notice_text(&editor), Some(history::IN_FLIGHT), "{chord}");
            assert_eq!(editor.stack(), views::GRAPH_EDITOR_LINKS, "{chord}");
        }
        bench.key(&mut editor, "esc");
        editor.notice = None;
        bench.key(&mut editor, "e");
        assert_eq!(notice_text(&editor), Some(history::IN_FLIGHT));
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(bench.requests().is_empty());
        bench.reply(&mut editor, &applied(stored(sent)));
        assert_eq!(
            notice_text(&editor),
            Some("saved: removing the link plan \u{2192} review")
        );
    }

    #[tokio::test]
    async fn applied_adopts_the_body_and_the_header_names_the_undo() {
        let bench = Bench::new().await;
        let backend = Backend::memory(MemStore::demo());
        let (body, personas) = demo(&backend, &bench).await;
        let revision = body.graph.revision;
        let (mut editor, _) = GraphEditor::open(body, personas, &bench.scope);
        assert_eq!(
            step_name(&editor.body, editor.selected.expect("a step")),
            Some("research")
        );
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let (_, _, _, request) = bench.write();
        let reply = served(&backend, &request).await;
        bench.reply(&mut editor, &reply);
        assert!(!editor.busy());
        assert_eq!(editor.body.graph.revision, revision + 1);
        assert_eq!(names(&editor.body)[0], "research2");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert_eq!(notice_text(&editor), Some("saved: editing research"));
        let screen = bench.render(&editor);
        assert!(screen.contains("u undoes: editing research"), "{screen}");
        assert!(
            screen.contains(&format!("revision {}", revision + 1)),
            "{screen}"
        );
    }

    #[tokio::test]
    async fn undo_sends_the_previous_body_on_the_current_revision_and_redo_returns() {
        let bench = Bench::new().await;
        let backend = Backend::memory(MemStore::demo());
        let (body, personas) = demo(&backend, &bench).await;
        let revision = body.graph.revision;
        let (mut editor, _) = GraphEditor::open(body, personas, &bench.scope);
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let (_, _, _, request) = bench.write();
        bench.reply(&mut editor, &served(&backend, &request).await);

        bench.key(&mut editor, "u");
        let (_, expected, sent, request) = bench.write();
        assert_eq!(expected, revision + 1, "on the current revision");
        assert_eq!(names(&sent)[0], "research");
        bench.reply(&mut editor, &served(&backend, &request).await);
        assert_eq!(notice_text(&editor), Some("undid editing research"));
        assert_eq!(names(&editor.body)[0], "research");
        assert!(!bench.render(&editor).contains("undoes"));

        bench.key(&mut editor, "U");
        let (_, expected, sent, request) = bench.write();
        assert_eq!(expected, revision + 2);
        assert_eq!(names(&sent)[0], "research2");
        bench.reply(&mut editor, &served(&backend, &request).await);
        assert_eq!(notice_text(&editor), Some("redid editing research"));
        assert_eq!(editor.body.graph.revision, revision + 3);
        assert!(bench.render(&editor).contains("u undoes: editing research"));
    }

    #[tokio::test]
    async fn stale_adopts_the_current_body_and_clears_undo_and_redo() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 4);
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let (_, _, sent, _) = bench.write();
        bench.reply(&mut editor, &applied(stored(sent)));
        assert_eq!(editor.history.undo_label(), Some("editing research"));

        bench.keys(&mut editor, &["j", "e"]);
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let _ = bench.write();
        let mut current = flow(0, GraphScope::Library, 9);
        current.nodes[2].phase.name = "audit".to_owned();
        bench.reply(&mut editor, &stale(current.clone()));
        assert_eq!(editor.body, current);
        assert_eq!(
            editor.notice,
            Some(Notice::Error(STALE_RELOADED.to_owned()))
        );
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(!editor.busy());
        bench.key(&mut editor, "u");
        assert_eq!(notice_text(&editor), Some(history::NOTHING_TO_UNDO));
        bench.key(&mut editor, "U");
        assert_eq!(notice_text(&editor), Some(history::NOTHING_TO_REDO));
        assert!(bench.requests().is_empty());
    }

    /// The persona the R1 M1 tests reference and then remove from the registry.
    fn reviewer() -> PersonaSummary {
        PersonaSummary {
            id: PersonaId::from_uuid(Uuid::from_u128(0x0141_0900)),
            name: "reviewer".to_owned(),
        }
    }

    /// The sentence the client's rules give for `persona` gone from the registry.
    fn no_persona(persona: &PersonaSummary) -> String {
        references_no_row("step_graph_phase.persona_id", persona.id, "persona")
    }

    /// One `e`, `2`, `enter` rename of the selected step, answered `Applied` with the sent body
    /// one revision on, `persona` given to research when `Some`.
    fn renamed(bench: &Bench, editor: &mut GraphEditor, persona: Option<PersonaId>) {
        bench.key(editor, "e");
        bench.typed(editor, "2");
        bench.key(editor, "enter");
        let (_, _, sent, _) = bench.write();
        let mut answer = stored(sent);
        if persona.is_some() {
            answer.nodes[0].phase.persona_id = persona;
        }
        bench.reply(editor, &applied(answer));
    }

    /// MOD-141 R1 M1: an undo entry the client's rules now refuse (its persona was removed) never
    /// leaves: nothing is sent, the entry is dropped with a sentence, and the next undo reaches the
    /// older entry.
    #[tokio::test]
    async fn an_undo_the_rules_refuse_is_dropped_and_the_next_undo_reaches_the_older_entry() {
        let bench = Bench::new().await;
        let reviewer = reviewer();
        let (mut editor, _) = GraphEditor::open(
            flow(0, GraphScope::Library, 0),
            vec![reviewer.clone()],
            &bench.scope,
        );
        // Edit one: research renamed, and the store's answer has it played by the reviewer.
        renamed(&bench, &mut editor, Some(reviewer.id));
        // Edit two: plan renamed; its undo entry is the body naming the reviewer.
        bench.key(&mut editor, "j");
        renamed(&bench, &mut editor, None);
        assert_eq!(editor.history.undo_label(), Some("editing plan"));

        editor.set_personas(&[]);
        bench.key(&mut editor, "u");
        assert!(bench.requests().is_empty(), "a refused undo sends nothing");
        let dropped = format!(
            "can't undo editing plan: {}; dropped from history",
            no_persona(&reviewer)
        );
        assert_eq!(notice_text(&editor), Some(dropped.as_str()));
        assert!(!editor.busy());
        assert_eq!(editor.history.depths(), (1, 0), "dropped, not put back");

        bench.key(&mut editor, "u");
        let (_, _, sent, _) = bench.write();
        assert_eq!(names(&sent), ["research", "plan", "review"]);
        assert_eq!(
            editor.history.busy(),
            Some(&Write::Undo("editing research".to_owned()))
        );
    }

    /// MOD-141 R1 M1: a redo entry the client's rules now refuse is dropped the same way.
    #[tokio::test]
    async fn a_redo_the_rules_refuse_is_dropped() {
        let bench = Bench::new().await;
        let reviewer = reviewer();
        let (mut editor, _) = GraphEditor::open(
            flow(0, GraphScope::Library, 0),
            vec![reviewer.clone()],
            &bench.scope,
        );
        renamed(&bench, &mut editor, Some(reviewer.id));
        // Undo: the redo entry is the body naming the reviewer.
        bench.key(&mut editor, "u");
        let (_, _, sent, _) = bench.write();
        bench.reply(&mut editor, &applied(stored(sent)));
        assert_eq!(editor.history.depths(), (0, 1));

        editor.set_personas(&[]);
        bench.key(&mut editor, "U");
        assert!(bench.requests().is_empty(), "a refused redo sends nothing");
        let dropped = format!(
            "can't redo editing research: {}; dropped from history",
            no_persona(&reviewer)
        );
        assert_eq!(notice_text(&editor), Some(dropped.as_str()));
        assert!(!editor.busy());
        assert_eq!(editor.history.depths(), (0, 0));
        bench.key(&mut editor, "U");
        assert_eq!(notice_text(&editor), Some(history::NOTHING_TO_REDO));
    }

    /// MOD-141 R1 M1: the store refusing an undo keeps the entry (the refusal may pass) and asks
    /// for the lookups again, so the next undo is judged against the registry as it is now. An
    /// edit's refusal asks for nothing.
    #[tokio::test]
    async fn a_refused_undo_asks_for_the_lookups_again() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        renamed(&bench, &mut editor, None);
        bench.key(&mut editor, "u");
        let _ = bench.write();
        bench.reply(
            &mut editor,
            &StoreReply::Failed {
                request: "replace_graph",
                message: "refused".to_owned(),
            },
        );
        let asked = vec![
            StoreRequest::Catalogue(bench.scope.clone()),
            StoreRequest::Agents,
        ];
        assert_eq!(format!("{:?}", bench.requests()), format!("{asked:?}"));
        assert_eq!(editor.history.undo_label(), Some("editing research"));
        assert_eq!(notice_text(&editor), Some("refused"));

        // A redo refused asks the same.
        bench.key(&mut editor, "u");
        let (_, _, sent, _) = bench.write();
        bench.reply(&mut editor, &applied(stored(sent)));
        bench.key(&mut editor, "U");
        let _ = bench.write();
        bench.reply(
            &mut editor,
            &StoreReply::Failed {
                request: "replace_graph",
                message: "refused".to_owned(),
            },
        );
        assert_eq!(format!("{:?}", bench.requests()), format!("{asked:?}"));
        assert_eq!(editor.history.depths(), (0, 1));
    }

    /// MOD-141 R1 L2: a key that takes the editor off the canvas mid-gesture ends the gesture,
    /// since no drag or release reaches the canvas outside it and the host's mouse stays on.
    #[tokio::test]
    async fn a_key_that_leaves_the_canvas_ends_the_gesture() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        let _ = bench.render(&editor);
        let area = editor.canvas_area.get().expect("the canvas was drawn");
        // The canvas's bottom-right corner is empty: a press there pans.
        let (column, row) = (area.right() - 2, area.bottom() - 2);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        {
            let mut ctx = bench.ctx();
            let left = MouseButton::Left;
            editor.on_mouse(mouse(MouseEventKind::Down(left), column, row), &mut ctx);
            editor.on_mouse(
                mouse(MouseEventKind::Drag(left), column - 5, row - 3),
                &mut ctx,
            );
        }
        let gesture = |editor: &GraphEditor| format!("{:?}", editor.canvas.borrow());
        let live = gesture(&editor);
        assert!(!live.contains("drag_state: None"), "a pan is live: {live}");
        assert!(!live.contains("pan: None"), "a pan is live: {live}");

        bench.key(&mut editor, "a");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);
        let ended = gesture(&editor);
        assert!(ended.contains("drag_state: None"), "{ended}");
        assert!(ended.contains("edge_preview: None"), "{ended}");
        assert!(ended.contains("pan: None"), "{ended}");
    }

    #[tokio::test]
    async fn failed_keeps_the_body_and_the_form_with_the_refusal() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let _ = bench.write();
        let refusal = "step_graph_phase.name `research2` is taken";
        bench.reply(
            &mut editor,
            &StoreReply::Failed {
                request: "replace_graph",
                message: refusal.to_owned(),
            },
        );
        assert!(!editor.busy());
        assert_eq!(editor.body, flow(0, GraphScope::Library, 0));
        assert_eq!(editor.notice, Some(Notice::Error(refusal.to_owned())));
        assert_eq!(
            editor.stack(),
            views::GRAPH_EDITOR_NODE_FORM,
            "the form is kept"
        );
        // Enter retries the same text.
        bench.key(&mut editor, "enter");
        let (_, _, sent, _) = bench.write();
        assert_eq!(names(&sent)[0], "research2");
    }

    #[tokio::test]
    async fn close_is_refused_while_busy_and_answers_close_otherwise() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.keys(&mut editor, &["j", "x", "y"]);
        let _ = bench.write();
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert_eq!(
            bench.key(&mut editor, "esc"),
            EditorKey::Handled(Handled::Consumed)
        );
        assert_eq!(notice_text(&editor), Some(history::IN_FLIGHT));
        bench.reply(
            &mut editor,
            &StoreReply::Failed {
                request: "replace_graph",
                message: "refused".to_owned(),
            },
        );
        assert_eq!(bench.key(&mut editor, "esc"), EditorKey::Close);
        // `ctrl-c` is the shell's; `q` is nothing here.
        assert_eq!(
            bench.key(&mut editor, "ctrl-c"),
            EditorKey::Handled(Handled::Pass)
        );
        assert_eq!(
            bench.key(&mut editor, "q"),
            EditorKey::Handled(Handled::Consumed)
        );
    }

    #[tokio::test]
    async fn connect_picks_a_target_then_the_edge_form_then_one_write() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 3);
        bench.keys(&mut editor, &["j", "j", "c"]);
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_CONNECT);
        bench.key(&mut editor, "enter");
        assert_eq!(notice_text(&editor), Some(model::SELF_LINK));
        bench.keys(&mut editor, &["k", "k", "enter"]);
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_EDGE_FORM);
        // next -> if -> asks you -> changes requested; then its loop limit.
        bench.keys(&mut editor, &["l", "l", "l", "tab"]);
        bench.typed(&mut editor, "2");
        bench.key(&mut editor, "enter");
        let (_, expected, sent, _) = bench.write();
        assert_eq!(expected, 3);
        let added = sent.edges.last().expect("a link");
        assert_eq!((added.from, added.to), (phase(3), phase(1)));
        assert_eq!(added.kind, EdgeKind::ChangesRequested);
        assert_eq!(added.loop_limit, Some(2));
        assert_eq!(sent.edges.len(), 3);
        assert_eq!(
            editor.history.busy(),
            Some(&Write::Edit("linking review \u{2192} research".to_owned()))
        );

        // A connect given up says so.
        bench.reply(&mut editor, &applied(stored(sent)));
        bench.keys(&mut editor, &["c", "esc"]);
        assert_eq!(notice_text(&editor), Some(NOT_CONNECTED));
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(bench.requests().is_empty());
    }

    #[tokio::test]
    async fn delete_asks_then_rewires_in_one_write() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.keys(&mut editor, &["j", "x"]);
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_CONFIRM);
        assert!(bench.requests().is_empty(), "nothing before the answer");
        let question = "delete step plan? its links go with it, and what led into it now leads to \
                        what came after; undo brings the step back but not its skill bindings. y \
                        delete \u{b7} n/Esc keep";
        let ctx = bench.ctx();
        let shown: Vec<String> = editor
            .bottom_lines(1000, &ctx)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(shown, [question]);
        bench.key(&mut editor, "n");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(bench.requests().is_empty());

        bench.keys(&mut editor, &["x", "y"]);
        let (_, _, sent, _) = bench.write();
        assert_eq!(names(&sent), ["research", "review"]);
        assert_eq!(sent.edges.len(), 1);
        let rewired = &sent.edges[0];
        assert_eq!(
            (rewired.from, rewired.to, rewired.kind),
            (phase(1), phase(3), EdgeKind::Next)
        );
        assert_eq!(
            editor.history.busy(),
            Some(&Write::Edit("deleting plan".to_owned()))
        );
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        bench.reply(&mut editor, &applied(stored(sent)));
        assert_eq!(
            editor.selected,
            Some(phase(1)),
            "the deleted step's selection moves"
        );
    }

    #[tokio::test]
    async fn cycling_a_relation_in_the_links_list_opens_the_edge_form_unsaved() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.keys(&mut editor, &["j", "i"]);
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_LINKS);
        // plan's links: -> review, then <- research.
        bench.keys(&mut editor, &["j", "l"]);
        let Mode::EdgeForm(form) = &editor.mode else {
            panic!("the link form opens");
        };
        assert_eq!(
            form.target(),
            EdgeTarget::Edit(edge(graph_id(0), 1, phase(1), phase(2)).id)
        );
        assert_eq!(form.kind(), EdgeKind::If);
        assert!(
            bench.requests().is_empty(),
            "nothing sent until it is saved"
        );

        // A link removed from the list: one write, the list stays.
        bench.keys(&mut editor, &["esc", "i", "d"]);
        let (_, _, sent, _) = bench.write();
        assert_eq!(sent.edges.len(), 1);
        assert_eq!(
            editor.history.busy(),
            Some(&Write::Edit(
                "removing the link plan \u{2192} review".to_owned()
            ))
        );
        bench.reply(&mut editor, &applied(stored(sent)));
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_LINKS);
        assert_eq!(
            notice_text(&editor),
            Some("saved: removing the link plan \u{2192} review")
        );
    }

    #[tokio::test]
    async fn a_rebound_key_acts_and_the_hint_follows() {
        let keys = load_str("[graph_editor]\nadd = [\"A\"]\ncandidate_add = [\"alt-p\"]\n")
            .expect("the file loads");
        let bench = Bench::new().await.with_keys(keys);
        let mut editor = library(&bench, 0);
        let hint = bench.hint(&editor);
        assert!(hint.contains("A add"), "{hint}");
        assert!(!hint.contains("a add"), "{hint}");
        bench.key(&mut editor, "a");
        assert_eq!(
            editor.stack(),
            views::GRAPH_EDITOR,
            "`a` is no key here now"
        );
        bench.key(&mut editor, "A");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);
        // The placeholder names the bound key, never a literal one (R-TUI-10).
        let screen = bench.render(&editor);
        assert!(screen.contains("none (Alt+p adds one)"), "{screen}");
        assert!(bench.hint(&editor).contains("Alt+p add agent"));
    }

    #[tokio::test]
    async fn every_mode_names_its_stack() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        let body = editor.body.clone();
        let node = body.nodes[0].clone();
        for (mode, stack) in [
            (Mode::Canvas, views::GRAPH_EDITOR),
            (
                Mode::Connecting { from: phase(1) },
                views::GRAPH_EDITOR_CONNECT,
            ),
            (
                Mode::NodeForm(Box::new(NodeEditing {
                    form: NodeForm::new(&node, NodeTarget::Edit(phase(1)), false),
                    base: node.clone(),
                })),
                views::GRAPH_EDITOR_NODE_FORM,
            ),
            (
                Mode::EdgeForm(Box::new(EdgeForm::new(phase(1), phase(3)))),
                views::GRAPH_EDITOR_EDGE_FORM,
            ),
            (
                Mode::PartForm(Box::new(PartEditing {
                    form: PartForm::new(None, PartTarget::New, Vec::new()),
                    node: Box::new(NodeEditing {
                        form: NodeForm::new(&node, NodeTarget::Edit(phase(1)), false),
                        base: node.clone(),
                    }),
                })),
                views::GRAPH_EDITOR_PART_FORM,
            ),
            (Mode::Links(Links::new(phase(1))), views::GRAPH_EDITOR_LINKS),
            (
                Mode::ConfirmDelete { node: phase(1) },
                views::GRAPH_EDITOR_CONFIRM,
            ),
        ] {
            let name = mode.name();
            editor.mode = mode;
            assert_eq!(editor.stack(), stack, "{name}");
            assert!(!bench.hint(&editor).is_empty(), "{name}");
        }
    }

    #[tokio::test]
    async fn debug_never_prints_typed_text() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 0);
        bench.key(&mut editor, "e");
        bench.typed(&mut editor, "hunter");
        assert!(editor.takes_paste());
        assert_eq!(editor.on_paste("swordfish"), Handled::Consumed);
        let printed = format!("{editor:?}");
        assert!(!printed.contains("hunter"), "{printed}");
        assert!(!printed.contains("swordfish"), "{printed}");
        assert!(!printed.contains("research"), "{printed}");
        assert!(printed.contains("step form"), "{printed}");

        // M2 D9: the part form's typed text stays out too.
        let (mut editor, _) = GraphEditor::open(divided(&[]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW]);
        bench.key(&mut editor, "alt-n");
        bench.typed(&mut editor, "hunter");
        assert!(editor.takes_paste());
        assert_eq!(editor.on_paste("swordfish"), Handled::Consumed);
        let printed = format!("{editor:?}");
        assert!(!printed.contains("hunter"), "{printed}");
        assert!(!printed.contains("swordfish"), "{printed}");
        assert!(!printed.contains("research"), "{printed}");
        assert!(printed.contains("part form"), "{printed}");
    }

    /// The step form's row of a divide step's first part (or its placeholder) on [`divided`]:
    /// M1's 13 rows, the candidates' placeholder, the deadline, the palette, the split and the
    /// six divide rows.
    const FIRST_PART_ROW: usize = 23;

    /// [`flow`] with research a divide step holding a part per key, titled in capitals.
    fn divided(keys: &[&str]) -> GraphBody {
        let mut body = flow(0, GraphScope::Library, 0);
        let research = &mut body.nodes[0];
        research.phase.split = Split::Divide;
        research.parts = keys
            .iter()
            .enumerate()
            .map(|(at, key)| PhasePart {
                phase_id: phase(1),
                position: i32::try_from(at).expect("a few parts"),
                key: (*key).to_owned(),
                title: key.to_uppercase(),
                brief: String::new(),
                owned_paths: Vec::new(),
                acceptance: None,
                agent_id: None,
                model: None,
                persona_id: None,
            })
            .collect();
        body
    }

    /// The parts of the open step form, as it would save them: `(key, title)`.
    fn form_parts(editor: &GraphEditor) -> Vec<(String, String)> {
        let Mode::NodeForm(editing) = &editor.mode else {
            panic!("the step form is open");
        };
        editing
            .form
            .build(&editing.base, &editor.lookups)
            .expect("the form builds")
            .parts
            .into_iter()
            .map(|part| (part.key, part.title))
            .collect()
    }

    /// The notes under the open step form, each note's wrapped lines joined back.
    fn shown_notes(bench: &Bench, editor: &GraphEditor) -> Vec<String> {
        let ctx = bench.ctx();
        let mut shown: Vec<String> = Vec::new();
        for line in editor.panel_lines(PANEL_WIDTH, u16::MAX, &ctx) {
            let line = line.to_string();
            if let Some(first) = line.strip_prefix("! ") {
                shown.push(first.to_owned());
            } else if let (Some(rest), Some(note)) = (line.strip_prefix("  "), shown.last_mut()) {
                note.push(' ');
                note.push_str(rest);
            }
        }
        shown
    }

    /// M2 D14: on the parts list the add act opens a blank part form; `Enter` keeps the part in
    /// the step form (writing nothing), a taken key is refused with the part form kept, and the
    /// step form's save writes the part.
    #[tokio::test]
    async fn alt_n_on_the_parts_list_opens_a_new_part_form_and_enter_keeps_it() {
        let bench = Bench::new().await;
        let (mut editor, _) = GraphEditor::open(divided(&[]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW]);
        bench.key(&mut editor, "alt-n");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_PART_FORM);
        let screen = bench.render(&editor);
        assert!(screen.contains("new part of research"), "{screen}");
        bench.typed(&mut editor, "ui");
        bench.key(&mut editor, "tab");
        bench.typed(&mut editor, "Front");
        bench.key(&mut editor, "enter");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);
        assert_eq!(form_parts(&editor), [("ui".to_owned(), "Front".to_owned())]);
        assert!(bench.requests().is_empty(), "keeping a part writes nothing");

        // The focus is on the kept part: the add act opens another, which may not take its key.
        bench.key(&mut editor, "alt-n");
        bench.typed(&mut editor, "ui");
        bench.key(&mut editor, "enter");
        let taken = part_form::part_key_taken("ui");
        assert_eq!(notice_text(&editor), Some(taken.as_str()));
        assert_eq!(
            editor.stack(),
            views::GRAPH_EDITOR_PART_FORM,
            "the part form stays"
        );
        bench.key(&mut editor, "esc");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);
        assert_eq!(notice_text(&editor), None);

        bench.key(&mut editor, "enter");
        let (_, _, sent, _) = bench.write();
        let parts = &sent.nodes[0].parts;
        assert_eq!(parts.len(), 1);
        assert_eq!(
            (
                parts[0].key.as_str(),
                parts[0].title.as_str(),
                parts[0].position
            ),
            ("ui", "Front", 0)
        );
    }

    /// M2 D14: the open act edits the focused part (off a part row it says there is none); `Esc`
    /// leaves the part as it was, `Enter` keeps the edit, and the order acts move the part.
    #[tokio::test]
    async fn alt_e_edits_the_focused_part_and_esc_leaves_it_unchanged() {
        let bench = Bench::new().await;
        let (mut editor, _) =
            GraphEditor::open(divided(&["ui", "store"]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        bench.key(&mut editor, "alt-e");
        assert_eq!(notice_text(&editor), Some(node_form::NO_PART));
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);

        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW + 1]);
        bench.key(&mut editor, "alt-e");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_PART_FORM);
        assert_eq!(notice_text(&editor), None);
        let screen = bench.render(&editor);
        assert!(screen.contains("part store of research"), "{screen}");
        bench.typed(&mut editor, "x");
        bench.key(&mut editor, "esc");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_NODE_FORM);
        let unchanged = [
            ("ui".to_owned(), "UI".to_owned()),
            ("store".to_owned(), "STORE".to_owned()),
        ];
        assert_eq!(form_parts(&editor), unchanged);

        bench.key(&mut editor, "alt-e");
        bench.key(&mut editor, "tab");
        bench.typed(&mut editor, "d");
        bench.key(&mut editor, "enter");
        bench.key(&mut editor, "alt-up");
        assert_eq!(
            form_parts(&editor),
            [
                ("store".to_owned(), "STOREd".to_owned()),
                ("ui".to_owned(), "UI".to_owned()),
            ]
        );
        bench.key(&mut editor, "alt-d");
        assert_eq!(form_parts(&editor), [("ui".to_owned(), "UI".to_owned())]);
        assert!(
            bench.requests().is_empty(),
            "nothing is written until saved"
        );
    }

    /// M2 D14: the hint row names the focused list's noun, from the editor's own words.
    #[tokio::test]
    async fn the_hint_names_the_focused_lists_noun() {
        let bench = Bench::new().await;
        let (mut editor, _) = GraphEditor::open(divided(&["ui"]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        let hint = bench.hint(&editor);
        assert!(hint.contains("Alt+n add agent"), "{hint}");
        assert!(hint.contains("Alt+d remove agent"), "{hint}");
        assert!(!hint.contains("part"), "{hint}");

        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW]);
        let hint = bench.hint(&editor);
        for words in ["Alt+n add part", "Alt+e edit part", "Alt+d remove part"] {
            assert!(hint.contains(words), "{words}: {hint}");
        }
        assert!(!hint.contains("agent"), "{hint}");

        bench.key(&mut editor, "alt-e");
        let hint = bench.hint(&editor);
        assert!(hint.contains("done"), "{hint}");
        assert!(!hint.contains("part"), "{hint}");
    }

    /// M2 D11, D12 (B5): a step a run would refuse today is marked `!` on the canvas, and its
    /// form lists resolve's sentences under its rows, live as the form changes. A warning never
    /// refuses the save.
    #[tokio::test]
    async fn a_noted_step_is_marked_and_its_form_lists_the_notes() {
        let bench = Bench::new().await;
        let body = divided(&[]);
        let expected = notes::notes(&body.nodes[0], None);
        assert!(!expected.is_empty(), "a part-less divide step cannot run");
        let (mut editor, _) = GraphEditor::open(body, Vec::new(), &bench.scope);
        let screen = bench.render(&editor);
        assert!(screen.contains("! research"), "{screen}");
        assert!(!screen.contains("! plan"), "{screen}");

        bench.key(&mut editor, "e");
        assert_eq!(shown_notes(&bench, &editor), expected);
        // Divide to none: nothing to say; back to divide: the notes again.
        bench.keys(&mut editor, &["tab"; 16]);
        bench.key(&mut editor, "h");
        assert_eq!(shown_notes(&bench, &editor), Vec::<String>::new());
        bench.key(&mut editor, "l");
        assert_eq!(shown_notes(&bench, &editor), expected);

        bench.key(&mut editor, "enter");
        let (_, _, sent, _) = bench.write();
        assert_eq!(sent.nodes[0].phase.split, Split::Divide);
        assert!(sent.nodes[0].parts.is_empty());
    }

    /// M2 D11, D14: a divide step's form is taller than the panel in the shell's 98x27 (and in
    /// less), so the panel scrolls the focused row into view and keeps the notes under it.
    #[tokio::test]
    async fn a_tall_step_form_scrolls_to_its_focus_and_keeps_its_notes() {
        let bench = Bench::new().await;
        let note = "! phase `research` cannot run yet";
        for (width, height) in [(98, 27), (98, 18)] {
            let (mut editor, _) = GraphEditor::open(divided(&[]), Vec::new(), &bench.scope);
            bench.reply(&mut editor, &settings(false));
            bench.key(&mut editor, "e");
            let screen = bench.render_in(&editor, width, height);
            for shown in ["step research", "\u{258c}name", note, "MOD-140"] {
                assert!(
                    screen.contains(shown),
                    "{width}x{height} {shown}:\n{screen}"
                );
            }

            bench.keys(&mut editor, &["tab"; FIRST_PART_ROW]);
            let screen = bench.render_in(&editor, width, height);
            for shown in ["step research", "\u{258c}parts", note, "MOD-140"] {
                assert!(
                    screen.contains(shown),
                    "{width}x{height} {shown}:\n{screen}"
                );
            }
            assert!(!screen.contains("\u{258c}name"), "{screen}");
        }
    }

    /// M2 D11: notes taller than half the panel keep the rows' half, and `…` says some were cut.
    #[test]
    fn notes_past_half_the_panel_end_in_an_ellipsis() {
        let theme = Theme::default();
        let row = |n: usize| Line::raw(format!("row {n}"));
        let form = FormLines {
            lines: (0..10).map(row).collect(),
            focus: Some(9..10),
        };
        let notes: Vec<Line<'static>> = (0..6)
            .map(|n| notice(&format!("! note {n}"), Tone::Error, &theme))
            .collect();
        let shown: Vec<String> = fitted(form, notes, 8, &theme)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            shown,
            [
                "row 6",
                "row 7",
                "row 8",
                "row 9",
                "! note 0",
                "! note 1",
                "! note 2",
                "  \u{2026}"
            ]
        );
    }

    /// M2 D5 (Open risk 4): Kinds' `e` on a phase row opens the step form before the editor's
    /// reads leave, so before the switch is read. The first read reaches that form: on, the
    /// split offers compete and a compete step is no longer locked. A later read does not.
    #[tokio::test]
    async fn a_form_opened_before_the_first_settings_read_takes_its_switch() {
        let bench = Bench::new().await;
        // As `KindsSection::open_editor` opens it.
        let opened = |body: GraphBody| {
            let (mut editor, _) = GraphEditor::open(body, Vec::new(), &bench.scope);
            editor.select(phase(1));
            editor.open_node_form();
            editor
        };
        let built = |editor: &GraphEditor| {
            let Mode::NodeForm(editing) = &editor.mode else {
                panic!("the step form is open");
            };
            editing
                .form
                .build(&editing.base, &editor.lookups)
                .expect("the form builds")
        };

        let mut editor = opened(flow(0, GraphScope::Library, 0));
        bench.reply(&mut editor, &settings(true));
        bench.keys(&mut editor, &["tab"; 16]);
        bench.keys(&mut editor, &["l", "l"]);
        assert_eq!(built(&editor).phase.split, Split::Compete);

        let mut body = flow(0, GraphScope::Library, 0);
        body.nodes[0].phase.split = Split::Compete;
        body.nodes[0].phase.fan_out = 2;
        let locked = "read-only while";
        let mut editor = opened(body.clone());
        assert!(bench.render(&editor).contains(locked));
        bench.reply(&mut editor, &settings(true));
        let screen = bench.render(&editor);
        assert!(!screen.contains(locked), "{screen}");
        // The fan-out row (after the split) takes a key now.
        bench.keys(&mut editor, &["tab"; 17]);
        bench.keys(&mut editor, &["backspace"]);
        bench.typed(&mut editor, "3");
        assert_eq!(built(&editor).phase.fan_out, 3);

        // Read off first, then on: the form keeps the switch it had read.
        let mut editor = opened(body);
        bench.reply(&mut editor, &settings(false));
        bench.reply(&mut editor, &settings(true));
        assert!(bench.render(&editor).contains(locked));
    }

    /// M2 D14: a focused part row names the open act's key under it, from the editor's keys.
    #[tokio::test]
    async fn a_focused_part_names_the_key_that_edits_it() {
        let bench = Bench::new().await;
        let (mut editor, _) = GraphEditor::open(divided(&["ui"]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW]);
        let screen = bench.render(&editor);
        assert!(screen.contains("\u{258c}part 1"), "{screen}");
        assert!(screen.contains("Alt+e edits it"), "{screen}");

        let keys = load_str("[graph_editor]\nopen_part = [\"alt-o\"]\n").expect("the file loads");
        let rebound = Bench::new().await.with_keys(keys);
        let screen = rebound.render(&editor);
        assert!(screen.contains("Alt+o edits it"), "{screen}");
        assert!(!screen.contains("Alt+e"), "{screen}");
        assert!(rebound.hint(&editor).contains("Alt+o edit part"));
        rebound.key(&mut editor, "alt-o");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR_PART_FORM);
    }

    #[tokio::test]
    async fn editor_canvas() {
        let bench = Bench::new().await;
        let editor = library(&bench, 2);
        insta::assert_snapshot!(bench.render(&editor));
    }

    #[tokio::test]
    async fn editor_node_form() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 2);
        bench.keys(&mut editor, &["j", "e"]);
        insta::assert_snapshot!(bench.render(&editor));
    }

    #[tokio::test]
    async fn editor_links() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 2);
        bench.keys(&mut editor, &["j", "i"]);
        insta::assert_snapshot!(bench.render(&editor));
    }

    #[tokio::test]
    async fn editor_confirm() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 2);
        bench.keys(&mut editor, &["j", "x"]);
        insta::assert_snapshot!(bench.render(&editor));
    }

    #[tokio::test]
    async fn editor_connecting() {
        let bench = Bench::new().await;
        let mut editor = library(&bench, 2);
        bench.keys(&mut editor, &["c", "j", "j"]);
        insta::assert_snapshot!(bench.render(&editor));
    }

    /// M2 D9: the open act on a divide step's second part.
    #[tokio::test]
    async fn editor_part_form() {
        let bench = Bench::new().await;
        let (mut editor, _) =
            GraphEditor::open(divided(&["ui", "store"]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        bench.keys(&mut editor, &["tab"; FIRST_PART_ROW + 1]);
        bench.key(&mut editor, "alt-e");
        insta::assert_snapshot!(bench.render(&editor));
    }

    /// M2 D11, D12: a part-less divide step's form with its notes, its canvas node marked.
    #[tokio::test]
    async fn editor_notes() {
        let bench = Bench::new().await;
        let (mut editor, _) = GraphEditor::open(divided(&[]), Vec::new(), &bench.scope);
        bench.key(&mut editor, "e");
        insta::assert_snapshot!(bench.render(&editor));
    }

    /// The mask is the store's: `next` forward, `changes_requested` never (the seeded bug
    /// graph: reproduce -> fix -> review, review -changes_requested-> fix).
    #[tokio::test]
    async fn forward_edges_is_the_stores_mask() {
        let body = MemStore::demo()
            .graph_body(ids::GRAPH_VULKAN_FIX)
            .await
            .expect("a memory read")
            .expect("the bug graph");
        let mask = forward_edges(&body);
        for (edge, forward) in body.edges.iter().zip(&mask) {
            assert_eq!(*forward, edge.kind == EdgeKind::Next, "{:?}", edge.kind);
        }
    }
}
