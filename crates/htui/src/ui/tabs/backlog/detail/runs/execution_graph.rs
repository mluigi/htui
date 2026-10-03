//! MOD-28: the Runs pane's flow view, the run under the cursor drawn as a `rataflow` graph.
//!
//! A pure projection of one `RunSummary` (ANA-12 invariant 1). [`project`] lays the steps out in
//! layers (plan D6), and `ExecutionGraph` owns the `Flow` that draws them. The flow keeps no
//! selection of its own: the cursor step is rebuilt selected on every sync (plan D7), and no key
//! ever reaches rataflow's own bindings (blueprint H-1).
//!
//! MOD-71: the mouse reaches the flow only through `ExecutionGraph::on_mouse`, from the Runs pane
//! while it browses the flow (D1, D5). A press on empty canvas or on an edge pans, the wheel
//! zooms at the pointer (D9), and a press on a node is a click on release that moves the cursor
//! (D6); no node ever moves (MOD-28 D9). A pan or a zoom survives a re-read: only a cursor
//! change, a resize or a new run moves the viewport (D7).

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use htui_core::model::{RunId, RunStepSummary, RunSummary, StepId, StepStatus, ToolCallCount};
use rataflow::{
    ControlsAction, Edge, EdgeStyle, Flow, FlowEvent, Handle, HandlePosition, Node, NodeContent,
    NodeRenderContext, StepEdge, Viewport,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget as _};

use crate::ui::Theme;
use crate::ui::cells;

/// Plan D10: a node's width in cells at zoom 1. Two candidates and the gap are 41 columns, which a
/// reveal's 1-cell margins keep inside the 43-column pane (fact-check R8; 21 cuts a border).
const NODE_W: f64 = 20.0;
/// Plan D10, MOD-72 D5: a node's height, a border, three lines (head, phase, chips) and a border.
const NODE_H: f64 = 5.0;
/// Plan D10: the columns between two nodes of one layer.
const H_GAP: f64 = 1.0;
/// Plan D10: the rows between two layers. Three keep a `retry` label off the arrowhead (R7).
const V_GAP: f64 = 3.0;
/// The x distance from one node of a layer to the next.
const X_STEP: f64 = NODE_W + H_GAP;
/// The y distance from one layer to the next.
const Y_STEP: f64 = NODE_H + V_GAP;
/// Plan D6: the label of an edge into the next attempt of the same position.
const RETRY: &str = "retry";
/// The margin `ensure_node_visible` keeps (`rataflow` `state/viewport.rs:299`), reused as the
/// reset viewport's left floor and top offset (blueprint E5).
const MARGIN: f64 = 1.0;
/// MOD-72 D6: the glyph that leads a node's chip line, once.
const TOOL: char = '\u{2692}';
/// MOD-72 D6: the sign between a chip's label and its count.
const TIMES: char = '\u{d7}';

/// One row of the flow (plan D6): the candidates of a `(position, attempt)`, or its judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layer {
    /// `run_step.position` of every step in it.
    pub(super) position: i32,
    /// `run_step.attempt` of every step in it.
    pub(super) attempt: i32,
    /// Whether this is the judge layer (`fanout_index < 0`) of its `(position, attempt)`.
    pub(super) judge: bool,
    /// Its steps, left to right: by `fanout_index`, then id.
    pub(super) steps: Vec<StepId>,
    /// The step marked `selected == Some(true)`, if any (`run.rs:739` is `Option<bool>`).
    pub(super) winner: Option<StepId>,
}

/// What [`project`] makes of a run.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Projection {
    /// The layers, top to bottom.
    pub(super) layers: Vec<Layer>,
    /// `(source, target, label)`, in layer order and then left to right.
    pub(super) edges: Vec<(StepId, StepId, Option<&'static str>)>,
    /// Every node's top-left corner in world units, in layer order and then left to right.
    pub(super) positions: Vec<(StepId, (f64, f64))>,
    /// The widest layer's width in world units, `0.0` for a run with no step (blueprint B-7).
    pub(super) width: f64,
}

/// Plan D6: a run's steps grouped into layers, each id once (blueprint B-5, H-4).
pub(super) fn layers(steps: &[RunStepSummary]) -> Vec<Layer> {
    /// The candidates and the judges of one `(position, attempt)`.
    type Group<'a> = (Vec<&'a RunStepSummary>, Vec<&'a RunStepSummary>);
    let mut seen = BTreeSet::new();
    let mut groups: BTreeMap<(i32, i32), Group<'_>> = BTreeMap::new();
    for step in steps {
        // Blueprint B-5: a reply is data, so a repeated id keeps its first step only.
        if !seen.insert(step.id) {
            continue;
        }
        let (candidates, judges) = groups.entry((step.position, step.attempt)).or_default();
        // `-1` is the judge (`engine.rs`); `< 0` keeps the split total over `i32` (E2).
        if step.fanout_index < 0 {
            judges.push(step);
        } else {
            candidates.push(step);
        }
    }
    let mut out = Vec::new();
    for ((position, attempt), (candidates, judges)) in groups {
        // The judge layer goes after its candidates, although `-1` sorts first (plan D6).
        for (judge, mut members) in [(false, candidates), (true, judges)] {
            if members.is_empty() {
                continue;
            }
            members.sort_by_key(|step| (step.fanout_index, step.id));
            out.push(Layer {
                position,
                attempt,
                judge,
                steps: members.iter().map(|step| step.id).collect(),
                winner: members
                    .iter()
                    .find(|step| step.selected == Some(true))
                    .map(|step| step.id),
            });
        }
    }
    out
}

/// The width in world units of a layer of `n` nodes (`n >= 1`).
fn span(n: usize) -> f64 {
    let n = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    n * NODE_W + (n - 1.0) * H_GAP
}

/// Plan D6: a run as layers, edges and positions. Rataflow-free, so T1 tests it alone.
pub(super) fn project(run: &RunSummary) -> Projection {
    let layers = layers(&run.steps);
    let width = layers
        .iter()
        .map(|layer| span(layer.steps.len()))
        .fold(0.0, f64::max);
    let mut positions = Vec::new();
    let mut y = 0.0;
    for layer in &layers {
        // Centred on the widest layer, in whole cells (blueprint E7).
        let mut x = ((width - span(layer.steps.len())) / 2.0).floor();
        for step in &layer.steps {
            positions.push((*step, (x, y)));
            x += X_STEP;
        }
        y += Y_STEP;
    }
    let mut edges = Vec::new();
    for pair in layers.windows(2) {
        let [from, to] = pair else { continue };
        let same_slot = (to.position, to.attempt) == (from.position, from.attempt);
        // The judge reads every candidate; past a judge or an `s`, only the winner feeds on.
        let sources = match from.winner {
            Some(winner) if !(to.judge && same_slot) => vec![winner],
            _ => from.steps.clone(),
        };
        let retry = to.position == from.position && to.attempt > from.attempt;
        // Review L5: one `retry` per target, on its first edge; a label on every edge into it
        // would draw the word once per source over the same rows.
        let mut labelled = BTreeSet::new();
        for source in &sources {
            for target in &to.steps {
                let label = (retry && labelled.insert(*target)).then_some(RETRY);
                edges.push((*source, *target, label));
            }
        }
    }
    Projection {
        layers,
        edges,
        positions,
        width,
    }
}

/// Plan D10: one step's node. The flow draws no border (`rataflow` `content.rs:79`), so this does.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StepNode {
    /// Line 1: the list's slot, the status, then `*`/`✓` (blueprint B-8).
    head: String,
    /// Line 2: the phase name.
    phase: String,
    /// Line 3 (MOD-72 D5): this step's tool-call rows, fitted to the interior at draw time
    /// (blueprint E10); none for a step that made no call.
    calls: Vec<ToolCallCount>,
    /// What colours the border.
    status: StepStatus,
    /// The pane's theme, copied: `NodeContent::render` sees only rataflow's context.
    theme: Theme,
}

impl StepNode {
    /// The node of `step`, one of `siblings` (its run's steps, for the slot's `/i` and `/j`),
    /// and `calls`, its rows of the last `ToolCalls` reply.
    pub(super) fn new(
        step: &RunStepSummary,
        siblings: &[RunStepSummary],
        calls: &[ToolCallCount],
        theme: &Theme,
    ) -> Self {
        // Blueprint B-8/E10: the marks in `gate()`'s order, `*` then `✓`.
        let mut marks = String::new();
        if step.promoted_at.is_some() {
            marks.push('*');
        }
        if step.selected == Some(true) {
            marks.push('\u{2713}');
        }
        let status = super::step_status(step.status);
        let slot = super::slot(step, siblings);
        let head = if marks.is_empty() {
            format!("{slot} {status}")
        } else {
            format!("{slot} {status} {marks}")
        };
        Self {
            head,
            phase: step.phase_name.clone(),
            calls: calls.to_vec(),
            status: step.status,
            theme: *theme,
        }
    }

    /// The border: `selected` on the cursor, else by status (plan D10).
    fn border(&self, selected: bool) -> Style {
        if selected {
            return self.theme.selected;
        }
        match self.status {
            StepStatus::Failed => self.theme.error,
            StepStatus::Running | StepStatus::AwaitingApproval => self.theme.accent,
            StepStatus::Superseded | StepStatus::Cancelled => self.theme.dim,
            StepStatus::Pending | StepStatus::Done => self.theme.base,
        }
    }

    /// The text: `dim` for a node whose border is `dim` (a loser, a retried attempt), `base`
    /// otherwise.
    fn text(&self) -> Style {
        match self.status {
            StepStatus::Superseded | StepStatus::Cancelled => self.theme.dim,
            StepStatus::Pending
            | StepStatus::Running
            | StepStatus::AwaitingApproval
            | StepStatus::Done
            | StepStatus::Failed => self.theme.base,
        }
    }
}

impl NodeContent for StepNode {
    /// The border, then the head and the phase inside it, then the chips, dim. `ctx.area` is the
    /// node's full size at the current zoom; a node with under two interior rows (zoom 0.5) is a
    /// bare box.
    fn render(&self, ctx: &NodeRenderContext, buf: &mut Buffer) {
        let block = Block::bordered().border_style(self.border(ctx.selected));
        let inner = block.inner(ctx.area);
        block.render(ctx.area, buf);
        // MOD-72 review L3: rataflow floors a node's top and bottom edges apart, so at zoom 0.5 a
        // 5-row node is 2 or 3 rows by pan offset. A 1-row interior would flicker a clipped head
        // while panning; it stays a bare box like the 0-row one (MOD-28's zoom 0.5).
        if inner.height < 2 {
            return;
        }
        let width = usize::from(inner.width);
        // Blueprint E10: fitted to the interior drawn, so `+N` is honest at every zoom.
        let chips = chips(&self.calls, width);
        let lines = [
            (self.head.as_str(), self.text()),
            (self.phase.as_str(), self.text()),
            (chips.as_str(), self.theme.dim), // MOD-72 D6: secondary to the head and phase
        ];
        for (row, (line, style)) in (0u16..).zip(lines) {
            if row >= inner.height {
                break; // a 2-row interior (between zooms) drops the chips line (D5)
            }
            buf.set_string(inner.x, inner.y + row, cells::clip(line, width), style);
        }
    }
}

/// MOD-72 D6: the short label of a `tool_kind` wire value (ACP's ten, `htui_agent::event::ToolKind`);
/// an unknown value is drawn as itself.
fn short_label(tool_kind: &str) -> &str {
    match tool_kind {
        "delete" => "del",
        "search" => "find",
        "execute" => "exec",
        "switch_mode" => "mode",
        // `read`, `edit`, `move`, `think`, `fetch`, `other`, and any value a later protocol adds.
        other => other,
    }
}

/// MOD-72 D6: a step's tool calls as one line of at most `width` cells: the tool glyph, then
/// `label×n` chips, larger counts first and ties by label, then `+N` for the `N` kinds that did
/// not fit. Empty when the step made no call. Never wider than `width` once `width` holds the
/// glyph and `+N`; `StepNode::render` clips anyway.
pub(super) fn chips(calls: &[ToolCallCount], width: usize) -> String {
    use core::fmt::Write as _;

    // D6: never a `×0`.
    let mut shown: Vec<(&str, u32)> = calls
        .iter()
        .filter(|call| call.calls > 0)
        .map(|call| (short_label(&call.tool_kind), call.calls))
        .collect();
    if shown.is_empty() {
        return String::new();
    }
    shown.sort_by(|(a_label, a_calls), (b_label, b_calls)| {
        b_calls
            .cmp(a_calls)
            .then_with(|| a_label.as_bytes().cmp(b_label.as_bytes()))
    });
    let mut line = String::from(TOOL);
    let mut used = cells::cell_width(&line);
    let total = shown.len();
    for (i, (label, count)) in shown.into_iter().enumerate() {
        let chip = format!(" {label}{TIMES}{count}");
        let left = total - i - 1;
        // Blueprint E12: room for the `+N` this chip's failure would leave, unless it is the last.
        // ` +` and `left`'s ASCII digits, counted without a `String` (MOD-72 review N6).
        let reserve = if left == 0 {
            0
        } else {
            2 + left.ilog10() as usize + 1
        };
        if used + cells::cell_width(&chip) + reserve > width {
            // E11: stop at the first chip that does not fit, so the shown ones are the largest;
            // the chip before this one reserved exactly this `+N`.
            let _ = write!(line, " +{}", total - i); // a `String` write cannot fail
            return line;
        }
        used += cells::cell_width(&chip);
        line.push_str(&chip);
    }
    line
}

/// MOD-72 D7: a `ToolCalls` reply's rows by step, each step's rows in reply order.
pub(super) fn by_step(counts: &[ToolCallCount]) -> BTreeMap<StepId, Vec<ToolCallCount>> {
    let mut map: BTreeMap<StepId, Vec<ToolCallCount>> = BTreeMap::new();
    for count in counts {
        map.entry(count.step).or_default().push(count.clone());
    }
    map
}

/// Plan D9: one source on the bottom and one target on the top, both hidden. Baked into every
/// builder, because `set_nodes` resets anything set after it (fact-check R2).
fn handles() -> Vec<Handle> {
    vec![
        Handle::source(HandlePosition::Bottom).with_hidden(true),
        Handle::target(HandlePosition::Top).with_hidden(true),
    ]
}

/// One projected edge, drawn in the pane's own styles (blueprint B-4).
fn edge(
    source: StepId,
    target: StepId,
    label: Option<&'static str>,
    theme: &Theme,
) -> Edge<StepEdge> {
    let content = StepEdge::default().with_style(
        EdgeStyle::default()
            .with_stroke_style(theme.dim)
            .with_label_style(theme.accent),
    );
    let edge = Edge::new(
        format!("{source}>{target}"),
        source.to_string(),
        target.to_string(),
    )
    .with_content(content)
    .with_deletable(false)
    .with_selectable(false);
    match label {
        Some(label) => edge.with_label(label),
        None => edge,
    }
}

/// Whether rataflow draws into `area` at all (`ui/canvas.rs:42`): what `render` needs before it
/// measures a reveal (review L1), and what the Runs pane needs before it hit-tests a press
/// against the area rataflow recorded on that draw (MOD-71 D5, blueprint B-7).
pub(super) const fn drawable(area: Rect) -> bool {
    area.width >= 2 && area.height >= 2
}

/// Blueprint E4: what the next `render` does to the viewport before it draws.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Reveal {
    /// Nothing.
    #[default]
    None,
    /// Keep the cursor node on screen (`ensure_node_visible`, plan D7).
    Cursor,
    /// A new run: zoom 1, the run centred, then the cursor kept on screen (plan D12, B-1).
    Reset,
}

/// Plan D7/D12/D13: the flow view of one run, rebuilt from every sync.
#[derive(Debug)]
pub(super) struct ExecutionGraph {
    /// The canvas. Never fed a key (blueprint H-1); a mouse event reaches it only through
    /// [`ExecutionGraph::on_mouse`] (MOD-71 D5).
    flow: Flow<StepNode, StepEdge>,
    /// The run last synced, which tells a new run apart (plan D12).
    run: Option<RunId>,
    /// The cursor step last synced, the node a reveal keeps on screen.
    cursor: Option<StepId>,
    /// The run's widest layer in world units (B-7), for the reset's centring.
    width: f64,
    /// The pending viewport move (B-3).
    reveal: Reveal,
    /// MOD-71 D7: the size of the pane the last drawable canvas sat in. A different one reveals
    /// the cursor; a pan or a zoom alone never does, and neither does a head line that comes or
    /// goes, which moves the canvas inside the same pane (review M1).
    drawn: Option<(u16, u16)>,
}

impl Default for ExecutionGraph {
    fn default() -> Self {
        Self {
            // Plan D9: an edge is never reconnected. MOD-71 D6: a press on empty canvas keeps
            // the cursor node selected; rataflow's default clears it (`state/mod.rs:509`).
            flow: Flow::new()
                .with_edges_reconnectable(false)
                .with_deselect_on_pane_click(false),
            run: None,
            cursor: None,
            width: 0.0,
            reveal: Reveal::None,
            drawn: None,
        }
    }
}

impl ExecutionGraph {
    /// Plan D7/D12: rebuilds the flow from `run`, `cursor` selected. `None` clears it.
    ///
    /// The selection and the hidden handles are baked into every node builder, because
    /// `set_nodes` clears both (fact-check R2). The viewport survives it, and a re-read of the same
    /// run keeps the canvas still: when a wider layer moves the centring, the viewport moves by
    /// the same amount, so the nodes already drawn stay where they were (review L2). Only a new
    /// run resets it (blueprint E4). MOD-71 D7: a same-run sync reveals the cursor only when the
    /// cursor step changed; the review-L2 anchor keeps the nodes still either way.
    ///
    /// `calls` is the pane's last `ToolCalls` reply by step (MOD-72 D7). A re-sync that only
    /// changes it keeps the viewport, because no position moves.
    pub(super) fn sync(
        &mut self,
        run: Option<&RunSummary>,
        cursor: Option<StepId>,
        calls: &BTreeMap<StepId, Vec<ToolCallCount>>,
        theme: &Theme,
    ) {
        let Some(run) = run else {
            self.clear();
            return;
        };
        let projection = project(run);
        let anchor = if self.run == Some(run.id) {
            self.anchor(&projection, cursor)
        } else {
            None
        };
        let nodes = projection
            .positions
            .iter()
            .filter_map(|(id, (x, y))| {
                let step = run.steps.iter().find(|step| step.id == *id)?;
                Some(
                    Node::new(
                        id.to_string(),
                        (*x, *y),
                        (NODE_W, NODE_H),
                        StepNode::new(
                            step,
                            &run.steps,
                            calls.get(id).map_or(&[][..], Vec::as_slice),
                            theme,
                        ),
                    )
                    .with_selected(Some(*id) == cursor)
                    .with_draggable(false)
                    .with_connectable(false)
                    .with_deletable(false)
                    .with_handles(handles()),
                )
            })
            .collect();
        let edges = projection
            .edges
            .iter()
            .map(|(source, target, label)| edge(*source, *target, *label, theme))
            .collect();
        // Blueprint B-5: a projection never repeats an id or loops, so neither call fails; if one
        // did, the canvas is drawn empty rather than the render path panicking.
        if self
            .flow
            .set_nodes(nodes)
            .and_then(|()| self.flow.set_edges(edges))
            .is_err()
        {
            self.clear();
            return;
        }
        if let Some(((old_x, old_y), (new_x, new_y))) = anchor {
            let zoom = self.flow.viewport.zoom;
            self.flow.viewport.x -= (new_x - old_x) * zoom;
            self.flow.viewport.y -= (new_y - old_y) * zoom;
        }
        // MOD-71 D7: a same-run re-read reveals the cursor only when the cursor step changed, so
        // the active-run poll and every `RunStream` re-read leave a pan or a wheel zoom where the
        // user put it. A new run still resets.
        let next = if self.run != Some(run.id) {
            Reveal::Reset
        } else if self.cursor != cursor {
            Reveal::Cursor
        } else {
            Reveal::None
        };
        self.run = Some(run.id);
        self.cursor = cursor;
        self.width = projection.width;
        self.reveal = self.reveal.max(next);
    }

    /// Review L2: a node drawn before and after a re-read, its old and new world corner: the
    /// cursor's when it was on the canvas, else the first such node.
    fn anchor(
        &self,
        projection: &Projection,
        cursor: Option<StepId>,
    ) -> Option<((f64, f64), (f64, f64))> {
        cursor
            .into_iter()
            .chain(projection.positions.iter().map(|(id, _)| *id))
            .find_map(|id| {
                let key = id.to_string();
                let old = self.flow.nodes().find(|node| node.id == key)?.position;
                let (_, new) = projection.positions.iter().find(|(at, _)| *at == id)?;
                Some(((old.x, old.y), *new))
            })
    }

    /// An empty canvas that remembers no run.
    fn clear(&mut self) {
        // Neither call can fail on an empty list: nothing duplicates and nothing dangles.
        let _ = self.flow.set_edges(Vec::new());
        let _ = self.flow.set_nodes(Vec::new());
        self.run = None;
        self.cursor = None;
        self.width = 0.0;
        self.reveal = Reveal::None;
    }

    /// Plan D8: `+`, one step (1.2) in, clamped to 2.0. The cursor stays on screen (E9).
    pub(super) fn zoom_in(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomIn);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// Plan D8: `-`, one step out, clamped to 0.5 (E9).
    pub(super) fn zoom_out(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomOut);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// Plan D8: `=`, the whole run fitted on the next render. A run taller than the canvas at the
    /// 0.5 zoom floor does not fit whole, so the cursor is kept on screen after it (review M1).
    pub(super) fn fit(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::FitView);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// MOD-71 D5, D6, D8: one mouse event on the canvas, in terminal coordinates — rataflow maps
    /// them through the area the last `render` drew (`ui/canvas.rs:37`). A left press on empty
    /// canvas or on an edge (never selectable, so never hit) pans; the wheel zooms at the pointer
    /// within 0.5–2.0 (D9); a left press on a node is a click when it is released, wherever that
    /// is. Every other kind is dropped (blueprint E2).
    ///
    /// The clicked step, if this event completed a click; the pane moves the cursor (D6). No
    /// reveal is queued (D7). After every event the flow's selection is put back on the cursor
    /// (blueprint E1): rataflow selects a pressed node at once (`state/mouse.rs:230-237`), and
    /// the flow keeps no selection of its own (MOD-28 D7).
    pub(super) fn on_mouse(&mut self, mouse: MouseEvent) -> Option<StepId> {
        if !matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left)
                | MouseEventKind::Up(MouseButton::Left)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ) {
            return None;
        }
        let response = self.flow.handle_mouse_event(mouse);
        match self.cursor {
            Some(cursor) => self.flow.select_node(&cursor.to_string()),
            None => self.flow.clear_selection(),
        }
        response.into_events().find_map(|event| match event {
            FlowEvent::NodeClicked { node_id } => node_id.parse().ok(),
            _ => None,
        })
    }

    /// MOD-74 D4: ends a live gesture without a click. A pan's drag state, or a node press's
    /// `AwaitingNodeClick`, gets a **locked** left release (`event_handlers.rs:496-509`): it
    /// resets the drag state and emits nothing, where an unlocked one would click the node.
    pub(super) fn end_gesture(&mut self) {}

    /// Plan D7, blueprint B-2/E6: draws `area`, revealing first when a reveal is pending.
    ///
    /// A canvas under 2x2 draws nothing (`rataflow` `ui/canvas.rs:42`), so a reveal measured
    /// against it would be wrong; it stays pending for the next frame (review L1). `pane` is the
    /// area the canvas was cut from: one whose size differs from the last drawable frame's reveals
    /// the cursor (MOD-71 D7). Not `area`'s size, which a head line coming or going changes on a
    /// re-read, and that would undo a pan (review M1).
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect, pane: Rect) {
        if !drawable(area) {
            return;
        }
        // MOD-71 D7, review M1: a pane of a new size reveals the cursor; the first drawable frame
        // counts, which a pending `Reset` covers anyway.
        let size = (pane.width, pane.height);
        if self.drawn != Some(size) {
            self.drawn = Some(size);
            self.reveal = self.reveal.max(Reveal::Cursor);
        }
        let reveal = core::mem::take(&mut self.reveal);
        if reveal != Reveal::None {
            // The canvas size is known only after a draw (`rataflow` `state/viewport.rs:288-291`),
            // so the first pass goes to a throwaway buffer: drawing twice into the frame would
            // leave the first pass's borders behind wherever the second does not overwrite them.
            let mut scratch = Buffer::empty(area);
            (&mut self.flow).render(area, &mut scratch);
            if reveal == Reveal::Reset {
                // Plan D12, blueprint B-1/E5: zoom 1, the widest layer centred, a row of margin.
                let x = ((f64::from(area.width) - self.width) / 2.0)
                    .floor()
                    .max(MARGIN);
                self.flow.viewport = Viewport::new(x, MARGIN, 1.0);
            }
            if let Some(cursor) = self.cursor {
                self.flow.ensure_node_visible(&cursor.to_string());
            }
        }
        frame.render_widget(&mut self.flow, area);
    }

    /// The canvas's pan and zoom, which the pane compares across a mouse event to tell one that
    /// changed the frame from one that did not (MOD-71 review L4).
    pub(super) const fn viewport(&self) -> Viewport {
        self.flow.viewport
    }

    /// The run on the canvas, for the tests.
    #[cfg(test)]
    pub(super) fn shown_run(&self) -> Option<RunId> {
        self.run
    }

    /// The id of the node the flow has selected, for the tests.
    #[cfg(test)]
    pub(super) fn selected(&self) -> Option<String> {
        self.flow.first_selected_node_id()
    }

    /// The canvas zoom, for the tests.
    #[cfg(test)]
    pub(super) fn zoom(&self) -> f64 {
        self.flow.viewport.zoom
    }

    /// Whether rataflow holds a drag, for the tests.
    #[cfg(test)]
    pub(super) fn is_dragging(&self) -> bool {
        self.flow.is_dragging()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    use htui_core::fixtures::demo_at;
    use htui_core::model::{BoxId, ProjectId, RunId, RunKind, RunMode, RunStatus, StepStatus};
    use ratatui::style::{Color, Modifier};
    use uuid::Uuid;

    use super::*;
    use crate::ui::cells::cell_width;

    /// A settled step at `(position, attempt, fanout_index)`, id `0x100 + n` (the `htui-orch`
    /// `closeout.rs` idiom).
    fn step(n: u128, position: i32, attempt: i32, fanout_index: i32) -> RunStepSummary {
        RunStepSummary {
            id: id(n),
            position,
            attempt,
            fanout_index,
            phase_name: "plan".to_owned(),
            agent_id: None,
            model: None,
            status: StepStatus::Done,
            gate_outcome: None,
            started_at: None,
            finished_at: None,
            prompt_tokens: None,
            trimmed: false,
            usage: None,
            selected: None,
            exit_code: None,
            verify_outcome: None,
            promoted_at: None,
            agent_name: None,
            gate_note: None,
        }
    }

    /// `step` with `selected: won`, `Superseded` when it lost and `Done` otherwise.
    fn candidate(
        n: u128,
        position: i32,
        attempt: i32,
        index: i32,
        won: Option<bool>,
    ) -> RunStepSummary {
        RunStepSummary {
            selected: won,
            status: if won == Some(false) {
                StepStatus::Superseded
            } else {
                StepStatus::Done
            },
            ..step(n, position, attempt, index)
        }
    }

    /// `step` with `status`.
    fn with_status(step: RunStepSummary, status: StepStatus) -> RunStepSummary {
        RunStepSummary { status, ..step }
    }

    /// A running graph run `0x900 + n` holding `steps` as given (order not normalised).
    fn run(n: u128, steps: Vec<RunStepSummary>) -> RunSummary {
        RunSummary {
            id: RunId::from_uuid(Uuid::from_u128(0x900 + n)),
            item_id: None,
            project_id: ProjectId::default(),
            kind: RunKind::Graph,
            mode: RunMode::Manual,
            status: RunStatus::Running,
            target_box_id: BoxId::default(),
            executing_box_id: None,
            box_hostname: "box".to_owned(),
            queued_at: demo_at(0, 0),
            started_at: None,
            finished_at: None,
            failure: None,
            steps,
        }
    }

    /// The id `step(n, ..)` carries.
    fn id(n: u128) -> StepId {
        StepId::from_uuid(Uuid::from_u128(0x100 + n))
    }

    /// The layers' step ids, top to bottom.
    fn ids_of(projection: &Projection) -> Vec<Vec<StepId>> {
        projection
            .layers
            .iter()
            .map(|layer| layer.steps.clone())
            .collect()
    }

    /// The top-left corner `project` gave `step`.
    fn at(projection: &Projection, step: StepId) -> (f64, f64) {
        projection
            .positions
            .iter()
            .find(|(id, _)| *id == step)
            .map(|(_, at)| *at)
            .expect("the step has a position")
    }

    #[test]
    fn a_linear_run_is_a_chain() {
        let projection = project(&run(
            1,
            vec![step(1, 0, 1, 0), step(2, 1, 1, 0), step(3, 2, 1, 0)],
        ));
        assert_eq!(ids_of(&projection), [vec![id(1)], vec![id(2)], vec![id(3)]]);
        assert_eq!(
            projection.edges,
            [(id(1), id(2), None), (id(2), id(3), None)]
        );
        assert_eq!(
            projection.positions,
            [
                (id(1), (0.0, 0.0)),
                (id(2), (0.0, 8.0)),
                (id(3), (0.0, 16.0))
            ]
        );
        assert!((projection.width - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_fanout_puts_its_candidates_in_one_layer_and_only_the_winner_feeds_the_next() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, Some(false)),
                candidate(2, 0, 1, 1, Some(true)),
                step(3, 1, 1, 0),
            ],
        ));
        assert_eq!(ids_of(&projection), [vec![id(1), id(2)], vec![id(3)]]);
        assert_eq!(projection.layers[0].winner, Some(id(2)));
        assert_eq!(projection.edges, [(id(2), id(3), None)]);
        assert_eq!(at(&projection, id(3)), (10.0, 8.0));
        assert!((projection.width - 41.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_judge_sorted_before_its_candidates_lands_in_the_layer_after_them() {
        let projection = project(&run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, None),
            ],
        ));
        assert_eq!(
            projection.layers,
            [
                Layer {
                    position: 0,
                    attempt: 1,
                    judge: false,
                    steps: vec![id(1), id(2)],
                    winner: None,
                },
                Layer {
                    position: 0,
                    attempt: 1,
                    judge: true,
                    steps: vec![id(3)],
                    winner: None,
                },
            ]
        );
    }

    #[test]
    fn every_candidate_feeds_the_judge() {
        let projection = project(&run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(4, 1, 1, 0),
            ],
        ));
        assert_eq!(
            projection.edges,
            [
                (id(1), id(3), None),
                (id(2), id(3), None),
                (id(3), id(4), None)
            ]
        );
    }

    #[test]
    fn a_retry_layer_is_fed_by_an_edge_labelled_retry() {
        let single = project(&run(
            1,
            vec![
                with_status(step(1, 1, 1, 0), StepStatus::Failed),
                with_status(step(2, 1, 2, 0), StepStatus::Running),
            ],
        ));
        assert_eq!(single.edges, [(id(1), id(2), Some(RETRY))]);

        let judged = project(&run(
            2,
            vec![
                step(3, 1, 1, -1),
                candidate(1, 1, 1, 0, None),
                candidate(2, 1, 1, 1, None),
                step(4, 1, 2, 0),
            ],
        ));
        assert_eq!(
            judged.edges,
            [
                (id(1), id(3), None),
                (id(2), id(3), None),
                (id(3), id(4), Some(RETRY))
            ]
        );
    }

    #[test]
    fn with_no_winner_yet_every_candidate_feeds_the_next_layer() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, Some(false)),
                step(3, 1, 1, 0),
            ],
        ));
        assert_eq!(
            projection.edges,
            [(id(1), id(3), None), (id(2), id(3), None)]
        );
    }

    #[test]
    fn an_empty_run_has_no_layers() {
        let projection = project(&run(1, Vec::new()));
        assert!(projection.layers.is_empty());
        assert!(projection.edges.is_empty());
        assert!(projection.positions.is_empty());
        assert!(projection.width.abs() < f64::EPSILON);
    }

    #[test]
    fn node_ids_are_step_ids_and_no_two_nodes_overlap() {
        let steps = vec![
            step(4, 0, 1, -1),
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, Some(true)),
            candidate(3, 0, 1, 2, Some(false)),
            step(7, 0, 2, -1),
            candidate(5, 0, 2, 0, None),
            candidate(6, 0, 2, 1, None),
            step(8, 1, 1, 0),
        ];
        let wanted: BTreeSet<StepId> = steps.iter().map(|step| step.id).collect();
        let projection = project(&run(1, steps));

        let placed: BTreeSet<StepId> = projection.positions.iter().map(|(id, _)| *id).collect();
        assert_eq!(placed, wanted);
        assert_eq!(projection.positions.len(), wanted.len());
        for (i, (_, (ax, ay))) in projection.positions.iter().enumerate() {
            for (_, (bx, by)) in &projection.positions[i + 1..] {
                let apart = ax + NODE_W <= *bx
                    || bx + NODE_W <= *ax
                    || ay + NODE_H <= *by
                    || by + NODE_H <= *ay;
                assert!(apart, "({ax}, {ay}) overlaps ({bx}, {by})");
            }
        }
        for (source, target, _) in &projection.edges {
            assert_ne!(source, target);
            assert!(placed.contains(source) && placed.contains(target));
        }
    }

    #[test]
    fn the_projection_does_not_depend_on_the_input_order() {
        let store = vec![
            step(3, 0, 1, -1),
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, None),
            step(4, 1, 1, 0),
        ];
        let judge_last = vec![
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, None),
            step(3, 0, 1, -1),
            step(4, 1, 1, 0),
        ];
        let reversed: Vec<_> = store.iter().rev().cloned().collect();
        let expected = project(&run(1, store));
        assert_eq!(project(&run(1, judge_last)), expected);
        assert_eq!(project(&run(1, reversed)), expected);
    }

    #[test]
    fn a_repeated_step_id_is_kept_once() {
        let projection = project(&run(1, vec![step(1, 0, 1, 0), step(1, 1, 1, 0)]));
        assert_eq!(projection.positions, [(id(1), (0.0, 0.0))]);
        assert_eq!(projection.layers.len(), 1);
        assert_eq!(projection.layers[0].position, 0);
        assert!(projection.edges.is_empty());
    }

    #[test]
    fn layers_are_centred_on_the_widest() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, None),
                candidate(3, 0, 1, 2, None),
                step(4, 1, 1, 0),
                candidate(5, 2, 1, 0, None),
                candidate(6, 2, 1, 1, None),
            ],
        ));
        assert!((projection.width - 62.0).abs() < f64::EPSILON);
        assert_eq!(at(&projection, id(1)), (0.0, 0.0));
        assert_eq!(at(&projection, id(3)), (42.0, 0.0));
        assert_eq!(at(&projection, id(4)), (21.0, 8.0));
        assert_eq!(at(&projection, id(5)), (10.0, 16.0));
        assert_eq!(at(&projection, id(6)), (31.0, 16.0));
    }

    // ---------------------------------------------------------------------------------------------
    // T2: the widget (plan D7, D9, D10, D12, D15).
    // ---------------------------------------------------------------------------------------------

    /// `step` with `phase` as its phase name.
    fn phased(step: RunStepSummary, phase: &str) -> RunStepSummary {
        RunStepSummary {
            phase_name: phase.to_owned(),
            ..step
        }
    }

    /// A fresh graph synced to `run`, the cursor on `cursor`.
    fn synced(run: &RunSummary, cursor: Option<StepId>) -> ExecutionGraph {
        let mut graph = ExecutionGraph::default();
        graph.sync(Some(run), cursor, &BTreeMap::new(), &Theme::default());
        graph
    }

    /// One frame of `graph` at 43x23 (the 100x30 frame's Runs canvas: 24 rows less the header).
    fn draw(graph: &mut ExecutionGraph) -> Buffer {
        draw_at(graph, 43, 23)
    }

    /// One frame of `graph` at `width` x `height`.
    fn draw_at(graph: &mut ExecutionGraph, width: u16, height: u16) -> Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
            .expect("the test backend is constructible");
        term.draw(|frame| graph.render(frame, frame.area(), frame.area()))
            .expect("the graph draws");
        term.backend().buffer().clone()
    }

    /// A linear run of `n` steps, one per position.
    fn linear(n: u128) -> RunSummary {
        let steps = (0..n)
            .map(|k| step(k, i32::try_from(k).expect("small"), 1, 0))
            .collect();
        run(1, steps)
    }

    /// Every `┌` in `buf` heads a node: the cell under it is `│` and the next one holds the
    /// slot's first digit. A corner left by an earlier pass heads nothing (blueprint B-2).
    fn assert_no_ghost_corner(buf: &Buffer) {
        for y in buf.area.top()..buf.area.bottom().saturating_sub(1) {
            for x in buf.area.left()..buf.area.right().saturating_sub(1) {
                if buf[(x, y)].symbol() == "\u{250c}" {
                    assert_eq!(buf[(x, y + 1)].symbol(), "\u{2502}", "({x}, {y})");
                    assert!(
                        buf[(x + 1, y + 1)]
                            .symbol()
                            .starts_with(|c: char| c.is_ascii_digit()),
                        "a ghost corner at ({x}, {y}): {:#?}",
                        rows(buf)
                    );
                }
            }
        }
    }

    /// Each row as a `String`, trailing blanks trimmed (as `runs.rs`'s `lines`).
    fn rows(buf: &Buffer) -> Vec<String> {
        (buf.area.top()..buf.area.bottom())
            .map(|y| {
                let row: String = (buf.area.left()..buf.area.right())
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// The `(x, y)` of the `┌` of the node whose text holds `needle` (an ASCII-only row).
    fn corner_of(buf: &Buffer, needle: &str) -> (u16, u16) {
        let rows = rows(buf);
        let (y, x) = rows
            .iter()
            .enumerate()
            .find_map(|(y, row)| {
                row.find(needle)
                    .map(|byte| (y, row[..byte].chars().count()))
            })
            .expect("the needle is drawn");
        let x = u16::try_from(x - 1).expect("the column fits");
        let y = u16::try_from(y).expect("the row fits");
        (0..y)
            .rev()
            .find(|row| buf[(x, *row)].symbol() == "\u{250c}")
            .map(|row| (x, row))
            .expect("the node has a top-left corner above its text")
    }

    #[test]
    fn the_cursor_node_is_drawn_selected() {
        let run = run(1, vec![step(1, 0, 1, 0), step(2, 1, 1, 0)]);
        let mut graph = synced(&run, Some(id(2)));
        let buf = draw(&mut graph);
        let cursor = corner_of(&buf, "1.1 done");
        let other = corner_of(&buf, "0.1 done");
        assert!(buf[cursor].modifier.contains(Modifier::REVERSED));
        assert!(!buf[other].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_failed_step_is_drawn_with_the_error_style() {
        let run = run(
            1,
            vec![
                step(1, 0, 1, 0),
                with_status(step(2, 1, 1, 0), StepStatus::Failed),
            ],
        );
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        assert_eq!(buf[corner_of(&buf, "1.1 failed")].fg, Color::Red);
    }

    #[test]
    fn the_winner_carries_a_check_and_the_loser_is_dim() {
        let run = run(
            1,
            vec![
                phased(candidate(1, 0, 1, 0, Some(true)), "research"),
                phased(candidate(2, 0, 1, 1, Some(false)), "research"),
            ],
        );
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        let rows = rows(&buf);
        assert!(
            rows.iter().any(|row| row.contains("0.1/0 done \u{2713}")),
            "{rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("0.1/1 superseded")),
            "{rows:#?}"
        );
        let (x, y) = corner_of(&buf, "0.1/1 superseded");
        assert_eq!(buf[(x, y)].fg, Color::DarkGray);
        assert_eq!(buf[(x + 1, y + 1)].fg, Color::DarkGray);
    }

    /// MOD-60 review L3: a node's text is `cells::clip`'s, flattened per cluster, so `"\r\n"` is
    /// one blank cell, not the two a per-`char` rule drew.
    #[test]
    fn a_crlf_in_a_phase_draws_as_one_blank_cell() {
        let run = run(1, vec![phased(step(1, 0, 1, 0), "x\r\nyz")]);
        let mut graph = synced(&run, Some(id(1)));
        let rows = rows(&draw(&mut graph));
        assert!(rows.iter().any(|row| row.contains("x yz")), "{rows:#?}");
    }

    #[test]
    fn a_promoted_winner_reads_star_then_check() {
        let promoted = RunStepSummary {
            selected: Some(true),
            promoted_at: Some(demo_at(0, 1)),
            ..step(1, 0, 1, 0)
        };
        let mut graph = synced(&run(1, vec![promoted]), Some(id(1)));
        let rows = rows(&draw(&mut graph));
        assert!(
            rows.iter().any(|row| row.contains("0.1 done *\u{2713}")),
            "{rows:#?}"
        );
    }

    #[test]
    fn text_is_clipped_by_display_width() {
        let wide = "\u{8abf}\u{67fb}\u{30d5}\u{30a7}\u{30fc}\u{30ba}\u{306e}\u{9577}\u{3044}\u{540d}\u{524d}\u{3067}\u{3059}";
        let cut = cells::clip(wide, 18);
        assert!(cell_width(&cut) <= 18, "{cut}");
        assert!(cut.ends_with(cells::ELLIPSIS), "{cut}");

        let run = run(1, vec![phased(step(1, 0, 1, 0), wide)]);
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "0.1 done");
        let right = (x..buf.area.right())
            .find(|column| buf[(*column, y)].symbol() == "\u{2510}")
            .expect("the node has a top-right corner");
        for row in y + 1..y + 4 {
            assert_eq!(buf[(right, row)].symbol(), "\u{2502}", "row {row}");
        }

        assert_eq!(cells::clip("ab", 5), "ab");
        assert_eq!(cells::clip("abcdef", 4), "abc\u{2026}");
        assert_eq!(
            cells::clip("\u{65e5}\u{672c}\u{8a9e}", 4),
            "\u{65e5}\u{2026}"
        );
        assert_eq!(cells::clip("a\nb", 3), "a b");
        assert_eq!(cells::clip("abc", 0), "");
    }

    #[test]
    fn a_re_sync_of_the_same_run_keeps_zoom_selection_and_hidden_handles() {
        let theme = Theme::default();
        let first = run(1, vec![step(1, 0, 1, 0), step(2, 1, 1, 0)]);
        let mut graph = synced(&first, Some(id(2)));
        draw(&mut graph);
        graph.zoom_in();
        draw(&mut graph);
        let viewport = graph.flow.viewport;
        assert!(graph.zoom() > 1.0);

        let mut changed = first.clone();
        changed.steps[1].status = StepStatus::Running;
        graph.sync(Some(&changed), Some(id(2)), &BTreeMap::new(), &theme);
        assert_eq!(graph.flow.viewport, viewport);
        assert_eq!(graph.selected(), Some(id(2).to_string()));
        assert!(
            graph
                .flow
                .nodes()
                .all(|node| node.handles.iter().all(|handle| handle.hidden))
        );
        draw(&mut graph);
        assert_eq!(graph.flow.viewport, viewport);

        let other = run(2, vec![step(3, 0, 1, 0)]);
        graph.sync(Some(&other), Some(id(3)), &BTreeMap::new(), &theme);
        draw(&mut graph);
        assert_eq!(graph.shown_run(), Some(other.id));
        assert!((graph.zoom() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_new_run_is_centred_at_zoom_one() {
        let mut one = synced(&run(1, vec![step(1, 0, 1, 0)]), Some(id(1)));
        let buf = draw(&mut one);
        assert_eq!(corner_of(&buf, "0.1 done"), (11, 1));

        let fanout = run(
            2,
            vec![
                phased(candidate(1, 0, 1, 0, Some(true)), "research"),
                phased(candidate(2, 0, 1, 1, Some(false)), "research"),
            ],
        );
        let mut graph = synced(&fanout, Some(id(1)));
        let buf = draw(&mut graph);
        assert_eq!(corner_of(&buf, "0.1/0 done"), (1, 1));
        assert_eq!(corner_of(&buf, "0.1/1 superseded"), (22, 1));
        assert_eq!(buf[(41, 1)].symbol(), "\u{2510}");
    }

    #[test]
    fn a_cursor_below_the_fold_is_visible_on_the_first_render() {
        let steps = (0..8)
            .map(|n| step(n, i32::try_from(n).expect("small"), 1, 0))
            .collect();
        let mut graph = synced(&run(1, steps), Some(id(7)));
        let rows = rows(&draw(&mut graph));
        assert!(rows.iter().any(|row| row.contains("7.1 done")), "{rows:#?}");
    }

    #[test]
    fn a_retry_edge_is_drawn_with_its_label() {
        let run = run(
            1,
            vec![
                with_status(step(1, 1, 1, 0), StepStatus::Failed),
                with_status(step(2, 1, 2, 0), StepStatus::Running),
            ],
        );
        let mut graph = synced(&run, Some(id(2)));
        let rows = rows(&draw(&mut graph));
        let label = rows
            .iter()
            .position(|row| row.contains(RETRY))
            .unwrap_or_else(|| panic!("no retry label: {rows:#?}"));
        assert!(
            rows[label + 1..].iter().any(|row| row.contains('\u{25bc}')),
            "{rows:#?}"
        );
    }

    #[test]
    fn two_renders_of_the_same_state_are_identical() {
        let run = run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(4, 1, 1, 0),
            ],
        );
        let mut graph = synced(&run, Some(id(4)));
        assert_eq!(draw(&mut graph), draw(&mut graph));
    }

    #[test]
    fn no_node_is_draggable_connectable_or_deletable() {
        let run = run(
            1,
            vec![
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(3, 1, 1, 0),
            ],
        );
        let graph = synced(&run, Some(id(1)));
        assert_eq!(graph.flow.nodes().count(), 3);
        assert!(
            graph
                .flow
                .nodes()
                .all(|node| !node.draggable && !node.connectable && !node.deletable)
        );
        assert!(!graph.flow.edges().is_empty());
        assert!(graph.flow.edges().iter().all(|edge| !edge.deletable));
        assert!(graph.flow.edges().iter().all(|edge| !edge.selectable));
        assert!(!graph.flow.edges_reconnectable);
    }

    #[test]
    fn zoom_is_clamped_and_fit_zooms_out_to_a_tall_run() {
        let steps = (0..8)
            .map(|n| step(n, i32::try_from(n).expect("small"), 1, 0))
            .collect();
        let mut graph = synced(&run(1, steps), Some(id(0)));
        draw(&mut graph);
        for _ in 0..10 {
            graph.zoom_out();
        }
        draw(&mut graph);
        assert!((graph.zoom() - 0.5).abs() < 1e-9, "{}", graph.zoom());
        for _ in 0..10 {
            graph.zoom_in();
        }
        draw(&mut graph);
        assert!((graph.zoom() - 2.0).abs() < 1e-9, "{}", graph.zoom());
        graph.fit();
        draw(&mut graph);
        assert!(graph.zoom() < 1.0, "{}", graph.zoom());
    }

    /// Review M1: `=` at the 0.5 floor cannot show a tall run whole, so it keeps the cursor on
    /// screen too.
    #[test]
    fn fit_keeps_the_cursor_node_on_screen() {
        let mut graph = synced(&linear(10), Some(id(9)));
        draw(&mut graph);
        graph.fit();
        let buf = draw(&mut graph);
        assert!((graph.zoom() - 0.5).abs() < 1e-9, "{}", graph.zoom());
        let selected = (buf.area.top()..buf.area.bottom())
            .flat_map(|y| (buf.area.left()..buf.area.right()).map(move |x| (x, y)))
            .any(|at| buf[at].modifier.contains(Modifier::REVERSED));
        assert!(selected, "the cursor's border is drawn: {:#?}", rows(&buf));
    }

    /// Review L1: a canvas too small to draw in keeps the reveal for the next frame.
    #[test]
    fn a_reveal_waits_for_a_canvas_big_enough_to_hold_it() {
        let mut graph = synced(&linear(8), Some(id(7)));
        draw_at(&mut graph, 1, 1);
        let rows = rows(&draw(&mut graph));
        assert!(rows.iter().any(|row| row.contains("7.1 done")), "{rows:#?}");
    }

    /// Review L2: a re-read whose new layer is wider than the run was moves the centring, and the
    /// viewport moves with it, so the nodes already on screen stay where they were.
    #[test]
    fn a_wider_re_read_of_the_same_run_keeps_the_nodes_still() {
        let before = run(1, vec![step(1, 0, 1, 0)]);
        let mut graph = synced(&before, Some(id(1)));
        let buf = draw(&mut graph);
        let column = corner_of(&buf, "0.1 done");

        let mut after = before.clone();
        after.steps.extend([
            candidate(2, 1, 1, 0, None),
            candidate(3, 1, 1, 1, None),
            candidate(4, 1, 1, 2, None),
        ]);
        graph.sync(
            Some(&after),
            Some(id(1)),
            &BTreeMap::new(),
            &Theme::default(),
        );
        let buf = draw(&mut graph);
        assert_eq!(corner_of(&buf, "0.1 done"), column, "{:#?}", rows(&buf));
    }

    /// Review L5: a retry into a fan-out fed by every candidate labels one edge per target.
    #[test]
    fn retry_is_labelled_once_per_target() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, None),
                candidate(3, 0, 2, 0, None),
                candidate(4, 0, 2, 1, None),
            ],
        ));
        assert_eq!(projection.edges.len(), 4);
        for target in [id(3), id(4)] {
            let labelled = projection
                .edges
                .iter()
                .filter(|(_, to, label)| *to == target && label.is_some())
                .count();
            assert_eq!(labelled, 1, "{target}: {:?}", projection.edges);
        }
    }

    /// Review N3: the reveal's scratch pass leaves nothing in the frame, on the first draw or the
    /// next one into the same terminal.
    #[test]
    fn the_reveal_leaves_no_ghost_corner() {
        let mut graph = synced(&linear(8), Some(id(7)));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(43, 23))
            .expect("the test backend is constructible");
        for _ in 0..2 {
            term.draw(|frame| graph.render(frame, frame.area(), frame.area()))
                .expect("the graph draws");
            assert_no_ghost_corner(term.backend().buffer());
        }
        graph.sync(
            Some(&linear(8)),
            Some(id(0)),
            &BTreeMap::new(),
            &Theme::default(),
        );
        term.draw(|frame| graph.render(frame, frame.area(), frame.area()))
            .expect("the graph draws");
        assert_no_ghost_corner(term.backend().buffer());
    }

    /// Review N4: a running and a parked step take the accent border.
    #[test]
    fn running_and_awaiting_steps_have_the_accent_border() {
        let run = run(
            1,
            vec![
                step(1, 0, 1, 0),
                with_status(step(2, 1, 1, 0), StepStatus::Running),
                with_status(step(3, 2, 1, 0), StepStatus::AwaitingApproval),
            ],
        );
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        assert_eq!(buf[corner_of(&buf, "1.1 running")].fg, Color::Cyan);
        assert_eq!(buf[corner_of(&buf, "2.1 awaiting")].fg, Color::Cyan);
    }

    #[test]
    fn a_sync_of_no_run_clears_the_canvas() {
        let mut graph = synced(&run(1, vec![step(1, 0, 1, 0)]), Some(id(1)));
        draw(&mut graph);
        graph.sync(None, None, &BTreeMap::new(), &Theme::default());
        assert_eq!(graph.flow.nodes().count(), 0);
        assert_eq!(graph.shown_run(), None);
        assert!(rows(&draw(&mut graph)).iter().all(String::is_empty));
    }

    // ---------------------------------------------------------------------------------------------
    // MOD-72 T3: the chip line (plan D5, D6, D7).
    // ---------------------------------------------------------------------------------------------

    /// `calls` calls of `tool_kind` on `step(1, ..)`.
    fn kind(tool_kind: &str, calls: u32) -> ToolCallCount {
        ToolCallCount {
            step: id(1),
            tool_kind: tool_kind.to_owned(),
            calls,
        }
    }

    /// D6's own example.
    #[test]
    fn chips_lead_with_the_tool_and_put_larger_counts_first() {
        let line = chips(&[kind("execute", 3), kind("read", 5)], 18);
        assert_eq!(line, "\u{2692} read\u{d7}5 exec\u{d7}3");
        assert_eq!(cell_width(&line), 15);
    }

    #[test]
    fn chips_break_a_tie_by_label() {
        assert_eq!(
            chips(&[kind("edit", 2), kind("delete", 2)], 18),
            "\u{2692} del\u{d7}2 edit\u{d7}2"
        );
    }

    #[test]
    fn every_known_kind_has_its_short_label() {
        let wire = [
            "read",
            "edit",
            "delete",
            "move",
            "search",
            "execute",
            "think",
            "fetch",
            "switch_mode",
            "other",
        ];
        let short: Vec<&str> = wire.iter().map(|kind| short_label(kind)).collect();
        assert_eq!(
            short,
            [
                "read", "edit", "del", "move", "find", "exec", "think", "fetch", "mode", "other"
            ]
        );
    }

    #[test]
    fn an_unknown_kind_is_drawn_as_itself() {
        assert_eq!(
            chips(&[kind("mcp_tool", 1)], 18),
            "\u{2692} mcp_tool\u{d7}1"
        );
    }

    #[test]
    fn chips_that_do_not_fit_become_plus_n() {
        let line = chips(
            &[
                kind("read", 5),
                kind("execute", 3),
                kind("edit", 2),
                kind("search", 1),
            ],
            18,
        );
        assert_eq!(line, "\u{2692} read\u{d7}5 exec\u{d7}3 +2");
        assert_eq!(cell_width(&line), 18);
    }

    /// Blueprint E12 (MOD-72 review M1): `exec×3` fits 16 alone (15 cells), but not with the
    /// ` +1` its successor would need, so it gives way to ` +2` rather than overflow to 18.
    #[test]
    fn a_chip_leaves_room_for_the_plus_n_after_it() {
        let line = chips(&[kind("read", 5), kind("execute", 3), kind("edit", 2)], 16);
        assert_eq!(line, "\u{2692} read\u{d7}5 +2");
        assert!(cell_width(&line) <= 16, "{line}");
    }

    #[test]
    fn the_last_chip_needs_no_room_for_plus_n() {
        let calls = [kind("read", 5), kind("delete", 1)];
        assert_eq!(chips(&calls, 14), "\u{2692} read\u{d7}5 del\u{d7}1");
        assert_eq!(chips(&calls, 13), "\u{2692} read\u{d7}5 +1");
    }

    /// Counting `char`s (6 of them) would wrongly fit the wide label at 7.
    #[test]
    fn chips_are_fitted_by_display_width() {
        let calls = [kind("\u{8abf}\u{67fb}", 2)];
        assert_eq!(chips(&calls, 7), "\u{2692} +1");
        assert_eq!(chips(&calls, 8), "\u{2692} \u{8abf}\u{67fb}\u{d7}2");
    }

    #[test]
    fn no_calls_is_no_line() {
        assert_eq!(chips(&[], 18), "");
        assert_eq!(chips(&[kind("read", 0)], 18), "");
    }

    #[test]
    fn by_step_groups_the_rows_of_a_reply() {
        let other = ToolCallCount {
            step: id(2),
            ..kind("read", 1)
        };
        let map = by_step(&[kind("read", 1), kind("edit", 2), other]);
        assert_eq!(map.len(), 2);
        assert_eq!(map[&id(1)].len(), 2);
        assert_eq!(map[&id(2)].len(), 1);
    }

    #[test]
    fn a_node_draws_its_chips_dim_on_the_third_line() {
        let run = run(1, vec![step(1, 0, 1, 0)]);
        let calls = BTreeMap::from([(id(1), vec![kind("read", 1)])]);
        let mut graph = ExecutionGraph::default();
        graph.sync(Some(&run), Some(id(1)), &calls, &Theme::default());
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "0.1 done");
        let rows = rows(&buf);
        assert!(
            rows[usize::from(y + 3)].contains("\u{2692} read\u{d7}1"),
            "{rows:#?}"
        );
        assert_eq!(buf[(x + 1, y + 3)].fg, Color::DarkGray);
        assert_eq!(buf[(x, y + 4)].symbol(), "\u{2514}", "{rows:#?}");
    }

    #[test]
    fn a_node_without_calls_draws_a_blank_third_line() {
        let mut graph = synced(&run(1, vec![step(1, 0, 1, 0)]), Some(id(1)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "0.1 done");
        for column in x + 1..x + 19 {
            assert_eq!(buf[(column, y + 3)].symbol(), " ", "column {column}");
        }
        assert_eq!(buf[(x, y + 3)].symbol(), "\u{2502}");
        assert_eq!(buf[(x, y + 4)].symbol(), "\u{2514}");
    }

    /// `step(1, …)` drawn alone into a `height`-row node, the way rataflow hands it a scratch
    /// buffer at local `(0, 0)`.
    fn render_node(height: u16) -> Buffer {
        let step = step(1, 0, 1, 0);
        let node = StepNode::new(
            &step,
            std::slice::from_ref(&step),
            &[kind("read", 1)],
            &Theme::default(),
        );
        let area = Rect::new(0, 0, 20, height);
        let mut buf = Buffer::empty(area);
        let ctx = NodeRenderContext {
            id: "1",
            area,
            selected: false,
            dragging: false,
            position_absolute: rataflow::Position::default(),
            theme: rataflow::Theme::default(),
            animation_phase: 0,
        };
        node.render(&ctx, &mut buf);
        buf
    }

    /// MOD-72 review L3: at zoom 0.5 rataflow floors the two edges apart, so a 5-row node is 2 or
    /// 3 rows by pan offset. The 3-row one, a 1-row interior, stays a bare box like the 2-row one,
    /// rather than flickering a clipped head while panning.
    #[test]
    fn a_one_row_interior_is_a_bare_box() {
        let buf = render_node(3);
        for column in 1..19 {
            assert_eq!(buf[(column, 1)].symbol(), " ", "column {column}");
        }
        assert_eq!(buf[(0, 1)].symbol(), "\u{2502}");
        let buf = render_node(4);
        assert!(rows(&buf)[1].contains("0.1 done"), "{:#?}", rows(&buf));
    }

    /// D7: new counts for the same run move no node, so the viewport stays (review L2).
    #[test]
    fn a_re_sync_with_new_counts_keeps_the_viewport() {
        let run = run(1, vec![step(1, 0, 1, 0), step(2, 1, 1, 0)]);
        let mut graph = synced(&run, Some(id(2)));
        draw(&mut graph);
        graph.zoom_in();
        draw(&mut graph);
        let viewport = graph.flow.viewport;

        let calls = BTreeMap::from([(
            id(2),
            vec![ToolCallCount {
                step: id(2),
                ..kind("read", 1)
            }],
        )]);
        graph.sync(Some(&run), Some(id(2)), &calls, &Theme::default());
        assert_eq!(graph.flow.viewport, viewport);
        let rows = rows(&draw(&mut graph));
        assert_eq!(graph.flow.viewport, viewport);
        assert!(
            rows.iter().any(|row| row.contains("\u{2692} read\u{d7}1")),
            "{rows:#?}"
        );
    }

    // ---------------------------------------------------------------------------------------------
    // MOD-71 T3: the mouse (plan D5-D9).
    // ---------------------------------------------------------------------------------------------

    /// A left press.
    const DOWN: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    /// A left drag.
    const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
    /// A left release.
    const UP: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

    /// `kind` at cell `(column, row)`, no modifier. `draw` is at `(0, 0)`, so a buffer cell is the
    /// terminal cell rataflow maps (blueprint H-8).
    fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A left press then its release at `at`: a click. The press answers `None`.
    fn click(graph: &mut ExecutionGraph, at: (u16, u16)) -> Option<StepId> {
        assert_eq!(
            graph.on_mouse(mouse(DOWN, at)),
            None,
            "a press clicks nothing yet"
        );
        graph.on_mouse(mouse(UP, at))
    }

    /// A left press at `from`, a drag to `to`, the release there: the release's answer.
    fn drag(graph: &mut ExecutionGraph, from: (u16, u16), to: (u16, u16)) -> Option<StepId> {
        assert_eq!(
            graph.on_mouse(mouse(DOWN, from)),
            None,
            "a press clicks nothing yet"
        );
        assert_eq!(
            graph.on_mouse(mouse(DRAG, to)),
            None,
            "a drag clicks nothing"
        );
        graph.on_mouse(mouse(UP, to))
    }

    /// Every node's id and world corner, in flow order.
    fn positions(graph: &ExecutionGraph) -> Vec<(String, (f64, f64))> {
        graph
            .flow
            .nodes()
            .map(|node| (node.id.clone(), (node.position.x, node.position.y)))
            .collect()
    }

    /// How many nodes the flow has selected.
    fn selected_count(graph: &ExecutionGraph) -> usize {
        graph.flow.nodes().filter(|node| node.selected).count()
    }

    /// Asserts `at` is blank in `buf`, so a press there lands on empty canvas.
    fn assert_blank(buf: &Buffer, at: (u16, u16)) {
        assert_eq!(
            buf[at].symbol(),
            " ",
            "{at:?} is not blank: {:#?}",
            rows(buf)
        );
    }

    /// The viewport's offset moved by `(dx, dy)` from `before`, its zoom unchanged.
    fn assert_panned(graph: &ExecutionGraph, before: Viewport, (dx, dy): (f64, f64)) {
        let now = graph.flow.viewport;
        assert!(
            (now.x - (before.x + dx)).abs() < f64::EPSILON,
            "{now:?} vs {before:?}"
        );
        assert!(
            (now.y - (before.y + dy)).abs() < f64::EPSILON,
            "{now:?} vs {before:?}"
        );
        assert!((now.zoom - before.zoom).abs() < f64::EPSILON);
    }

    /// D6: a click on a node's interior answers its step; the graph leaves the cursor to the pane.
    #[test]
    fn a_click_on_a_node_is_its_step() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "1.1 done");
        assert_eq!(click(&mut graph, (x + 2, y + 2)), Some(id(1)));
        assert_eq!(graph.selected(), Some(id(0).to_string()));
    }

    /// Blueprint E1, B-1: rataflow selects a pressed node at once; the graph puts the selection
    /// back on the cursor before the release.
    #[test]
    fn a_press_on_a_node_keeps_the_cursor_selected() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "1.1 done");
        assert_eq!(graph.on_mouse(mouse(DOWN, (x + 2, y + 2))), None);
        assert_eq!(graph.selected(), Some(id(0).to_string()));
        assert_eq!(selected_count(&graph), 1);
    }

    /// D5, MOD-28 D9: a drag on empty canvas moves the viewport by the drag and no node.
    #[test]
    fn a_drag_on_empty_canvas_pans_and_moves_no_node() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        assert_blank(&buf, (2, 12));
        let before = graph.flow.viewport;
        let nodes = positions(&graph);
        assert_eq!(drag(&mut graph, (2, 12), (5, 14)), None);
        assert_panned(&graph, before, (3.0, 2.0));
        assert_eq!(positions(&graph), nodes);
    }

    /// D6: a press on a node is a click wherever it is released (`AwaitingNodeClick` ignores the
    /// drag, `state/mouse.rs:819`), and nothing moves.
    #[test]
    fn a_node_press_dragged_away_is_still_a_click() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "1.1 done");
        let before = graph.flow.viewport;
        let nodes = positions(&graph);
        assert_eq!(
            drag(&mut graph, (x + 2, y + 2), (x + 7, y + 5)),
            Some(id(1))
        );
        assert_eq!(graph.flow.viewport, before);
        assert_eq!(positions(&graph), nodes);
    }

    /// D9: the wheel zooms by the keys' step, clamped to MOD-28's 0.5-2.0.
    #[test]
    fn the_wheel_zooms_at_the_pointer_within_the_flow_s_range() {
        let mut graph = synced(&linear(2), Some(id(0)));
        draw(&mut graph);
        for _ in 0..10 {
            assert_eq!(
                graph.on_mouse(mouse(MouseEventKind::ScrollUp, (21, 3))),
                None
            );
        }
        assert!((graph.zoom() - 2.0).abs() < 1e-9, "{}", graph.zoom());
        for _ in 0..10 {
            assert_eq!(
                graph.on_mouse(mouse(MouseEventKind::ScrollDown, (21, 3))),
                None
            );
        }
        assert!((graph.zoom() - 0.5).abs() < 1e-9, "{}", graph.zoom());
    }

    /// Blueprint E2, B-9: a right-drag over both nodes would be rataflow's box selection; it never
    /// reaches the flow.
    #[test]
    fn a_right_drag_selects_nothing() {
        let mut graph = synced(&linear(2), Some(id(0)));
        draw(&mut graph);
        for (kind, at) in [
            (MouseEventKind::Down(MouseButton::Right), (2, 12)),
            (MouseEventKind::Drag(MouseButton::Right), (40, 20)),
            (MouseEventKind::Up(MouseButton::Right), (40, 20)),
        ] {
            assert_eq!(graph.on_mouse(mouse(kind, at)), None);
        }
        assert_eq!(graph.selected(), Some(id(0).to_string()));
        assert_eq!(selected_count(&graph), 1);
    }

    /// D6, blueprint B-2: an edge is never selectable, so a press on one pans like the pane.
    #[test]
    fn an_edge_press_pans() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "0.1 done");
        let row = y + 6;
        let column = (buf.area.left()..buf.area.right())
            .find(|column| buf[(*column, row)].symbol() != " ")
            .expect("the edge is drawn between the two nodes");
        assert!(column > x, "the edge is right of the node's left border");
        let before = graph.flow.viewport;
        assert_eq!(drag(&mut graph, (column, row), (column + 3, row + 1)), None);
        assert_panned(&graph, before, (3.0, 1.0));
        assert_eq!(graph.selected(), Some(id(0).to_string()));
    }

    /// D6: a click on empty canvas keeps the cursor node selected (rataflow's default clears it).
    #[test]
    fn a_pane_click_keeps_the_cursor_node_selected() {
        let mut graph = synced(&linear(2), Some(id(1)));
        let buf = draw(&mut graph);
        assert_blank(&buf, (2, 12));
        assert_eq!(click(&mut graph, (2, 12)), None);
        assert_eq!(graph.selected(), Some(id(1).to_string()));
    }

    /// A three-step run, the cursor on the first, panned up until its node is off screen.
    fn panned() -> ExecutionGraph {
        let mut graph = synced(&linear(3), Some(id(0)));
        let buf = draw(&mut graph);
        assert_blank(&buf, (2, 20));
        assert_eq!(drag(&mut graph, (2, 20), (2, 2)), None);
        let rows = rows(&draw(&mut graph));
        assert!(
            !rows.iter().any(|row| row.contains("0.1 done")),
            "the pan took the cursor node off screen: {rows:#?}"
        );
        graph
    }

    /// D7: a same-run re-read with the cursor where it was leaves a pan where the user put it.
    #[test]
    fn a_re_read_after_a_pan_keeps_the_viewport() {
        let mut graph = panned();
        let before = graph.flow.viewport;
        let mut changed = linear(3);
        changed.steps[2].status = StepStatus::Running;
        graph.sync(
            Some(&changed),
            Some(id(0)),
            &BTreeMap::new(),
            &Theme::default(),
        );
        let rows = rows(&draw(&mut graph));
        assert_eq!(graph.flow.viewport, before);
        assert!(
            !rows.iter().any(|row| row.contains("0.1 done")),
            "{rows:#?}"
        );
    }

    /// D7: a cursor change still reveals the cursor.
    #[test]
    fn a_cursor_change_reveals_the_cursor() {
        let mut graph = panned();
        graph.sync(
            Some(&linear(3)),
            Some(id(1)),
            &BTreeMap::new(),
            &Theme::default(),
        );
        let rows = rows(&draw(&mut graph));
        assert!(rows.iter().any(|row| row.contains("1.1 done")), "{rows:#?}");
    }

    /// D7, blueprint E5: a canvas of a new size reveals the cursor.
    #[test]
    fn a_resize_reveals_the_cursor() {
        let mut graph = panned();
        let rows = rows(&draw_at(&mut graph, 43, 20));
        assert!(rows.iter().any(|row| row.contains("0.1 done")), "{rows:#?}");
    }

    // ---------------------------------------------------------------------------------------------
    // MOD-74 T2: ending a gesture (plan D4).
    // ---------------------------------------------------------------------------------------------

    /// D4: an ended pan forgets its anchor: a drag with no new press moves nothing, and the lock
    /// is put back.
    #[test]
    fn ending_a_pan_forgets_its_anchor() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        assert_blank(&buf, (2, 12));
        assert_eq!(graph.on_mouse(mouse(DOWN, (2, 12))), None);
        assert_eq!(graph.on_mouse(mouse(DRAG, (5, 14))), None);
        assert!(graph.is_dragging());
        graph.end_gesture();
        assert!(!graph.is_dragging());
        assert!(!graph.flow.locked);
        let before = graph.flow.viewport;
        assert_eq!(graph.on_mouse(mouse(DRAG, (9, 16))), None);
        assert_eq!(graph.flow.viewport, before);
    }

    /// D4, H-10: an ended node press clicks nothing, then or on a later release.
    #[test]
    fn ending_a_node_press_clicks_nothing() {
        let mut graph = synced(&linear(2), Some(id(0)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "1.1 done");
        let at = (x + 2, y + 2);
        assert_eq!(graph.on_mouse(mouse(DOWN, at)), None);
        graph.end_gesture();
        assert!(!graph.is_dragging());
        assert_eq!(graph.on_mouse(mouse(UP, at)), None, "no click");
        assert_eq!(graph.selected(), Some(id(0).to_string()));
        assert!(!graph.flow.locked);
    }

    /// D4: with no gesture live, ending one changes nothing.
    #[test]
    fn ending_no_gesture_changes_nothing() {
        let mut graph = synced(&linear(2), Some(id(0)));
        draw(&mut graph);
        let before = graph.flow.viewport;
        graph.end_gesture();
        assert_eq!(graph.flow.viewport, before);
        assert_eq!(graph.selected(), Some(id(0).to_string()));
        assert!(!graph.flow.locked);
        assert!(!graph.is_dragging());
    }
}
