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
//! - [`GraphEditor`] (this module): the modes, every key through the key catalogue (D12), the
//!   writes and the replies (D4), the mouse (D11) and the drawing (D2). Only this module names
//!   `Act`, `Keys`, `Stack`, `Ctx` or `StoreRequest`.

pub(crate) mod canvas;
pub(crate) mod edge_form;
pub(crate) mod history;
pub(crate) mod links;
pub(crate) mod model;
pub(crate) mod node_form;

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
use self::node_form::{NodeForm, NodeRow, NodeTarget};
use crate::app::{Ctx, Handled};
use crate::catalogue::PersonaSummary;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::TemplatesSnapshot;
use crate::ui::FieldOutcome;
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::text::{Tone, notice, wrapped};

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

/// The delete question (D10, C3): the editor cannot see the step's skill bindings, so it always
/// says what undo does not bring back. `answers` is the question's own keys.
fn delete_question(name: &str, answers: &str) -> String {
    format!(
        "delete step {name}? its links go with it, and what led into it now leads to what came \
         after; undo brings the step back but not its skill bindings, deadline or judge. {answers}"
    )
}

/// The line under the canvas while connecting: what the pick is for.
fn connecting(from: &str) -> String {
    format!("linking from {from}: pick the step it leads to")
}

/// The notice of a lookup read the store refused (`agents`, `templates`): the editor still works
/// without it.
fn unavailable(what: &str, message: &str) -> String {
    format!("{what} unavailable: {message}")
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

/// The step form's keys.
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
    /// `[StoreRequest::Agents, StoreRequest::Templates(scope)]`. Selects the first flow step
    /// (position order), else none.
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
        let form = NodeForm::new(node, NodeTarget::Edit(node.phase.id));
        self.mode = Mode::NodeForm(Box::new(NodeEditing {
            form,
            base: node.clone(),
        }));
        self.notice = None;
    }

    /// The reads the host re-sends on activation: `Agents`, `Templates(scope)`.
    #[must_use]
    pub fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Agents, StoreRequest::Templates(scope.clone())]
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
    /// the step form `GRAPH_EDITOR_NODE_FORM`, the link form `GRAPH_EDITOR_EDGE_FORM`, the links
    /// list `GRAPH_EDITOR_LINKS` and the delete question `GRAPH_EDITOR_CONFIRM`.
    #[must_use]
    pub const fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Canvas => views::GRAPH_EDITOR,
            Mode::Connecting { .. } => views::GRAPH_EDITOR_CONNECT,
            Mode::NodeForm(_) => views::GRAPH_EDITOR_NODE_FORM,
            Mode::EdgeForm(_) => views::GRAPH_EDITOR_EDGE_FORM,
            Mode::Links(_) => views::GRAPH_EDITOR_LINKS,
            Mode::ConfirmDelete { .. } => views::GRAPH_EDITOR_CONFIRM,
        }
    }

    /// The mode's keys through its stack, plus ` · saving…` while a write is in flight.
    #[must_use]
    pub fn hint_text(&self, keys: &Keys) -> String {
        let spec = match self.mode {
            Mode::Canvas => HINT_CANVAS,
            Mode::Connecting { .. } => HINT_CONNECT,
            Mode::NodeForm(_) => HINT_NODE_FORM,
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
            Mode::EdgeForm(form) => form.focused() != EdgeRow::Relation,
            _ => false,
        }
    }

    /// A bracketed paste into the focused text row. A form's other rows drop it; outside a form it
    /// is the host's.
    pub fn on_paste(&mut self, text: &str) -> Handled {
        match &mut self.mode {
            Mode::NodeForm(editing) => editing.form.on_paste(text),
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
        Handled::Consumed
    }

    /// The mouse went away mid-gesture.
    pub fn on_mouse_lost(&mut self) {
        self.canvas.get_mut().end_gesture();
    }

    /// Every reply the host receives; the editor takes its own: its write's answer, and the
    /// agents and templates it reads.
    pub fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
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
                if self.history.failed().is_some() {
                    self.after_save = None;
                    self.refuse(message);
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
            _ => {}
        }
    }

    /// Draws over the whole `area` (D2): the header, the canvas (and the open panel beside it),
    /// then the notice or the delete question.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let mut bottom = self.bottom_lines(area.width, ctx);
        bottom.truncate(NOTICE_ROWS);
        let [header, middle, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(u16::try_from(bottom.len()).unwrap_or(0)),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(Line::styled(self.header(ctx.keys()), theme.title)),
            header,
        );
        let panel = match &self.mode {
            Mode::NodeForm(_) | Mode::EdgeForm(_) | Mode::Links(_) => true,
            Mode::Canvas | Mode::Connecting { .. } | Mode::ConfirmDelete { .. } => false,
        };
        if panel {
            let [canvas, side] = Layout::horizontal([
                Constraint::Min(CANVAS_MIN),
                Constraint::Length(PANEL_WIDTH.min(middle.width / 2)),
            ])
            .areas(middle);
            self.draw_canvas(frame, canvas, ctx);
            frame.render_widget(Paragraph::new(self.panel_lines(side.width, ctx)), side);
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

    /// The panel beside the canvas: the open form's or the links list's title, then its rows.
    fn panel_lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let (title, rows) = match &self.mode {
            Mode::NodeForm(editing) => {
                let add = ctx
                    .keys()
                    .label(views::GRAPH_EDITOR_NODE_FORM, Act::GraphEditorCandidateAdd);
                (
                    editing.form.title(&self.body),
                    editing.form.lines(
                        &editing.base,
                        &self.lookups,
                        add.as_deref(),
                        width,
                        ctx.accent(),
                        theme,
                    ),
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

    /// Rebuilds the canvas from the body, the selection and the agents when any changed.
    fn sync_canvas(&self, ctx: &Ctx<'_>) {
        if self.resync.replace(false) {
            let agents = self.lookups.agents.as_deref().unwrap_or(&[]);
            self.canvas
                .borrow_mut()
                .sync(&self.body, self.selected, agents, ctx.theme);
        }
    }

    /// A form's focused text row takes `key` first; `None` when it passes it on (or no text row
    /// has the focus).
    fn on_text_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Option<Handled> {
        let outcome = match &mut self.mode {
            Mode::NodeForm(editing) if editing.form.focused() == NodeRow::Text => {
                editing.form.on_text_key(key)
            }
            Mode::EdgeForm(form) if form.focused() != EdgeRow::Relation => form.on_text_key(key),
            _ => return None,
        };
        match outcome {
            FieldOutcome::Consumed => {}
            FieldOutcome::Submit => self.save(ctx),
            FieldOutcome::Cancel => self.back_to_canvas(),
            FieldOutcome::Pass => return None,
        }
        Some(Handled::Consumed)
    }

    /// One act of the current mode's stack; `None` when this mode declines it.
    fn on_act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> Option<EditorKey> {
        let handled = match self.mode {
            Mode::Canvas => return self.on_canvas_act(act, ctx),
            Mode::Connecting { from } => self.on_connecting_act(from, act),
            Mode::NodeForm(_) => self.on_node_form_act(act, ctx),
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
                Ok(previous) => {
                    self.notice = None;
                    self.send(previous, ctx);
                }
                Err(refusal) => self.refuse(refusal.sentence()),
            },
            Act::GraphEditorRedo => match self.history.begin_redo(&self.body) {
                Ok(next) => {
                    self.notice = None;
                    self.send(next, ctx);
                }
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
            Act::GraphEditorCandidateAdd => form.candidate_add(&self.lookups),
            Act::GraphEditorCandidateRemove => form.candidate_remove(),
            Act::GraphEditorCandidateUp => form.candidate_move(true),
            Act::GraphEditorCandidateDown => form.candidate_move(false),
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
        EdgeId, NodeBody, ProjectRef, StepGraph, StepGraphEdge, StepGraphPhase,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{MemStore, WriteStore, loop_edge_runs_forward, reserved_phase_name};
    use htui_store::Backend;
    use ratatui::backend::TestBackend;
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use uuid::Uuid;

    use super::*;
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
            let area = Rect::new(0, 0, 100, 30);
            let mut terminal = Terminal::with_options(
                TestBackend::new(100, 30),
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
            catalogue: Box::new(crate::catalogue::CatalogueSnapshot {
                projects: Vec::new(),
                personas: Vec::new(),
            }),
        }
    }

    fn stale(body: GraphBody) -> StoreReply {
        StoreReply::GraphReplaced {
            outcome: CasOutcome::Stale(Box::new(body)),
            catalogue: Box::new(crate::catalogue::CatalogueSnapshot {
                projects: Vec::new(),
                personas: Vec::new(),
            }),
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

    #[tokio::test]
    async fn opening_asks_for_agents_and_templates() {
        let bench = Bench::new().await;
        let (editor, requests) =
            GraphEditor::open(flow(0, GraphScope::Library, 0), Vec::new(), &bench.scope);
        let reads = vec![
            StoreRequest::Agents,
            StoreRequest::Templates(bench.scope.clone()),
        ];
        let reads = format!("{reads:?}");
        assert_eq!(format!("{requests:?}"), reads);
        assert_eq!(format!("{:?}", editor.wants_requests(&bench.scope)), reads);
        assert_eq!(editor.graph(), graph_id(0));
        assert_eq!(editor.selected, Some(phase(1)), "the first flow step");
        assert_eq!(editor.stack(), views::GRAPH_EDITOR);
        assert!(!editor.busy());
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
                        what came after; undo brings the step back but not its skill bindings, \
                        deadline or judge. y delete \u{b7} n/Esc keep";
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
                    form: NodeForm::new(&node, NodeTarget::Edit(phase(1))),
                    base: node.clone(),
                })),
                views::GRAPH_EDITOR_NODE_FORM,
            ),
            (
                Mode::EdgeForm(Box::new(EdgeForm::new(phase(1), phase(3)))),
                views::GRAPH_EDITOR_EDGE_FORM,
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
