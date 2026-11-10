//! MOD-141 D7, D11: a `GraphBody` drawn as a rataflow `Flow`. Forward edges (`forward_edges`) are
//! layered by longest path; back edges leave and enter by the sides (`changes_requested` and a
//! back `asks_you` on the right, `fails` on the left); palette nodes sit in a band below. The flow
//! keeps no selection of its own and never sees a key; the mouse reaches it only through
//! `Canvas::on_mouse`, and `FlowAction::Delete` is never routed to it.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use htui_core::model::{
    AgentSummary, EdgeId, EdgeKind, GraphBody, NodeBody, PhaseId, Split, StepGraphEdge, StepGraphId,
};
use rataflow::{
    ControlsAction, Direction, Edge, EdgeContent, EdgePathContext, EdgeRenderContext, EdgeStyle,
    Flow, FlowEvent, Handle, HandlePosition, Node, NodeContent, NodeRenderContext, Path, Position,
    StepEdge, Viewport,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget as _;
use serde_json::Value;

use super::forward_edges;
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::label;
use crate::ui::kit::pane::Pane;

/// D7: a node's width in cells at zoom 1: a border and 22 cells for the template, the gate and
/// the agents lines.
pub(crate) const NODE_W: f64 = 24.0;
/// D7: a node's height, a border, three lines and a border.
pub(crate) const NODE_H: f64 = 5.0;
/// D7: the columns between two nodes of one layer, room for a side loop between them.
pub(crate) const H_GAP: f64 = 6.0;
/// D7: the rows between two layers, which keep a label off the arrowhead (execution_graph R7).
pub(crate) const V_GAP: f64 = 3.0;
/// The x distance from one node of a row to the next.
const X_STEP: f64 = NODE_W + H_GAP;
/// The y distance from one layer to the next.
const Y_STEP: f64 = NODE_H + V_GAP;
/// The widest an edge label is drawn, in cells.
const LABEL_W: usize = 24;
/// The sign between a relation and its loop limit.
const TIMES: char = '\u{d7}';
/// D8: a divide node's badge. The node form says "runs once MOD-139 lands"; that is 32 cells,
/// and the node's interior is 22, so the canvas says it in 22.
const DIVIDE: &str = "divide \u{b7} needs MOD-139";

/// Layout of one body, rataflow-free (unit-tested alone).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// Every node's top-left corner in world units: flow layers top to bottom, left to right,
    /// then the palette band.
    pub(crate) positions: Vec<(PhaseId, (f64, f64))>,
    /// Flow layers (palette band not counted).
    pub(crate) layers: usize,
    /// The widest row in world units (0.0 for an empty body).
    pub(crate) width: f64,
}

/// The width in world units of a row of `n` nodes (`n >= 1`).
fn span(n: usize) -> f64 {
    let n = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    n.mul_add(NODE_W, (n - 1.0) * H_GAP)
}

/// `n` as world units.
fn units(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// D7: `body` laid out by longest path over its forward edges, deterministic and independent of
/// the order of `body.edges`. A body the store would refuse (a forward cycle) still lays out:
/// the steps the walk never reaches each take a layer of their own below the rest.
pub(crate) fn layout(body: &GraphBody) -> Layout {
    let mask = forward_edges(body);
    // Flow nodes in body order (position order), palette nodes apart.
    let flow: Vec<usize> = (0..body.nodes.len())
        .filter(|at| !body.nodes[*at].phase.palette)
        .collect();
    let palette: Vec<usize> = (0..body.nodes.len())
        .filter(|at| body.nodes[*at].phase.palette)
        .collect();
    let slot: HashMap<PhaseId, usize> = flow
        .iter()
        .enumerate()
        .map(|(slot, at)| (body.nodes[*at].phase.id, slot))
        .collect();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); flow.len()];
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); flow.len()];
    let mut indegree = vec![0_usize; flow.len()];
    // Which side a step's loops run on: a loop's channel runs past every node beside its ends.
    let mut right = vec![false; flow.len()];
    let mut left = vec![false; flow.len()];
    for (edge, forward) in body.edges.iter().zip(&mask) {
        let (Some(&from), Some(&to)) = (slot.get(&edge.from), slot.get(&edge.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        if !*forward {
            let side = match edge.kind {
                EdgeKind::ChangesRequested | EdgeKind::AsksYou => &mut right,
                EdgeKind::Fails => &mut left,
                EdgeKind::Next | EdgeKind::If => continue,
            };
            side[from] = true;
            side[to] = true;
            continue;
        }
        successors[from].push(to);
        predecessors[to].push(from);
        indegree[to] += 1;
    }
    // Kahn's algorithm, the longest path from a root.
    let mut layer = vec![0_usize; flow.len()];
    let mut reached = vec![false; flow.len()];
    let mut queue: std::collections::VecDeque<usize> = (0..flow.len())
        .filter(|slot| indegree[*slot] == 0)
        .collect();
    while let Some(from) = queue.pop_front() {
        reached[from] = true;
        for &to in &successors[from] {
            layer[to] = layer[to].max(layer[from] + 1);
            indegree[to] -= 1;
            if indegree[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    // Only a forward cycle leaves a step unreached: each takes its own layer below the rest.
    let mut next = (0..flow.len())
        .filter(|slot| reached[*slot])
        .map(|slot| layer[slot] + 1)
        .max()
        .unwrap_or(0);
    for slot in 0..flow.len() {
        if !reached[slot] {
            layer[slot] = next;
            next += 1;
        }
    }
    let layers = (0..flow.len())
        .map(|slot| layer[slot] + 1)
        .max()
        .unwrap_or(0);
    // Order within a layer, top-down: a step with loops only on the left first and one with
    // loops only on the right last, so no loop's channel runs through a sibling; then by the
    // leftmost forward predecessor above, then position.
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for slot in 0..flow.len() {
        rows[layer[slot]].push(slot);
    }
    let mut order = vec![usize::MAX; flow.len()];
    for row in &mut rows {
        let key = |slot: usize| {
            let above = predecessors[slot]
                .iter()
                .filter(|from| layer[**from] < layer[slot])
                .map(|from| order[*from])
                .min()
                .unwrap_or(usize::MAX);
            let side = match (left[slot], right[slot]) {
                (true, false) => 0,
                (false, true) => 2,
                _ => 1,
            };
            (side, above, body.nodes[flow[slot]].phase.position, slot)
        };
        row.sort_by_key(|slot| key(*slot));
        for (at, slot) in row.iter().enumerate() {
            order[*slot] = at;
        }
    }
    let width = rows
        .iter()
        .map(Vec::len)
        .chain(core::iter::once(palette.len()))
        .filter(|n| *n > 0)
        .map(span)
        .fold(0.0, f64::max);
    let mut positions = Vec::with_capacity(body.nodes.len());
    for (at, row) in rows.iter().enumerate() {
        // Centred on the widest row, in whole cells (execution_graph E7).
        let left = ((width - span(row.len())) / 2.0).floor();
        for (i, slot) in row.iter().enumerate() {
            let id = body.nodes[flow[*slot]].phase.id;
            positions.push((id, (units(i).mul_add(X_STEP, left), units(at) * Y_STEP)));
        }
    }
    // One empty band between the last layer and the palette, which has no edges.
    let band = if layers > 0 {
        units(layers + 1) * Y_STEP
    } else {
        0.0
    };
    for (i, at) in palette.iter().enumerate() {
        positions.push((body.nodes[*at].phase.id, (units(i) * X_STEP, band)));
    }
    Layout {
        positions,
        layers,
        width,
    }
}

/// D7: the handle ids an edge leaves and enters by, `(source, target)`. A forward edge runs
/// bottom to top; a back `changes_requested` or `asks_you` loops on the right, a back `fails` on
/// the left. A back `next` or `if` cannot exist (the walk refuses it); it is drawn as forward.
pub(crate) const fn edge_handles(kind: EdgeKind, forward: bool) -> (&'static str, &'static str) {
    match (kind, forward) {
        (EdgeKind::ChangesRequested | EdgeKind::AsksYou, false) => ("r-out", "r-in"),
        (EdgeKind::Fails, false) => ("l-out", "l-in"),
        _ => ("bottom", "top"),
    }
}

/// D7: the label an edge carries, in words and at most [`LABEL_W`] cells; `None` for `next`.
pub(crate) fn edge_label(edge: &StepGraphEdge) -> Option<String> {
    let limit = |word: &str| match edge.loop_limit {
        Some(n) => format!("{word} {TIMES}{n}"),
        None => word.to_owned(),
    };
    let label = match edge.kind {
        EdgeKind::Next => return None,
        EdgeKind::If => match &edge.predicate {
            Some(predicate) => format!("if {}", predicate_words(&predicate.0)),
            None => "if".to_owned(),
        },
        EdgeKind::AsksYou => limit("asks you"),
        EdgeKind::ChangesRequested => limit("changes requested"),
        EdgeKind::Fails => limit("fails"),
    };
    Some(cells::clip(&label, LABEL_W))
}

/// An `if` predicate in words (ANA-33 §7.3's closed set); anything outside it as compact JSON.
fn predicate_words(value: &Value) -> String {
    let one = value
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.iter().next());
    let globs = |word: &str, globs: &Value| {
        let list = globs.as_array()?;
        let first = list.first()?.as_str()?;
        Some(match list.len() - 1 {
            0 => format!("{word} {first}"),
            more => format!("{word} {first} +{more}"),
        })
    };
    let words = one.and_then(|(key, inner)| match (key.as_str(), inner) {
        ("verify", Value::String(want)) if want == "failed" || want == "passed" => {
            Some(format!("verify {want}"))
        }
        ("item_kind", Value::String(prefix)) => Some(format!("kind {prefix}")),
        ("item_tag", Value::String(tag)) => Some(format!("tag {tag}")),
        ("any_changed_path_matches", list) => globs("any path", list),
        ("all_changed_paths_match", list) => globs("all paths", list),
        ("not", inner) => Some(format!("not ({})", predicate_words(inner))),
        ("all", Value::Array(list)) => Some(format!("all of {}", list.len())),
        ("any", Value::Array(list)) => Some(format!("any of {}", list.len())),
        _ => None,
    });
    words.unwrap_or_else(|| value.to_string())
}

/// D7: the margin `ensure_node_visible` keeps (`rataflow` `state/viewport.rs:299`), reused as the
/// reset viewport's left floor and top offset (execution_graph E5).
const MARGIN: f64 = 1.0;
/// D11: a drop anywhere on a node snaps to its `top` handle (the spike's radius).
const CONNECTION_RADIUS: f64 = NODE_W / 2.0 + 2.0;

/// D7: one step's node. The flow draws no border (`rataflow` `content.rs:79`), so this does, as a
/// kit pane titled by the step's name.
#[derive(Debug, Clone, PartialEq)]
struct StepNode {
    /// The top border's title: the step's name.
    title: String,
    /// The template, the gate and the agents lines (`node_lines`).
    lines: [String; 3],
    /// A palette node, drawn with a dim border.
    palette: bool,
    /// The editor's theme, copied: `NodeContent::render` sees only rataflow's context.
    theme: Theme,
}

impl StepNode {
    /// The node of `node`, its candidates named from `agents`.
    fn new(node: &NodeBody, agents: &[AgentSummary], theme: &Theme) -> Self {
        Self {
            title: node.phase.name.clone(),
            lines: node_lines(node, agents),
            palette: node.phase.palette,
            theme: *theme,
        }
    }

    /// The border: `cursor` on the selected node, `dim` on a palette node, else `base`.
    const fn border(&self, selected: bool) -> Style {
        if selected {
            self.theme.cursor
        } else if self.palette {
            self.theme.dim
        } else {
            self.theme.base
        }
    }
}

impl NodeContent for StepNode {
    /// The border, a kit pane titled by the name, then up to three lines inside it, each clipped
    /// to the interior. A node with under two interior rows (zoom 0.5) is a bare box, as
    /// execution_graph's is (MOD-72 review L3).
    fn render(&self, ctx: &NodeRenderContext, buf: &mut Buffer) {
        let inner = Pane::new(&self.title)
            .border(self.border(ctx.selected))
            .render_into(buf, ctx.area, &self.theme);
        if inner.height < 2 {
            return;
        }
        let width = usize::from(inner.width);
        for (row, line) in (0u16..).zip(&self.lines) {
            if row >= inner.height {
                break;
            }
            buf.set_string(
                inner.x,
                inner.y + row,
                cells::clip(line, width),
                self.theme.base,
            );
        }
    }
}

/// D7: a node's three lines: the template (and its pinned version), the gate (hard, retries),
/// then the first of split, palette and the first candidate agent that applies.
fn node_lines(node: &NodeBody, agents: &[AgentSummary]) -> [String; 3] {
    let phase = &node.phase;
    let template = match phase.template_version {
        Some(version) => format!("template {} v{version}", phase.template_name),
        None => format!("template {}", phase.template_name),
    };
    let mut gate = format!("gate {}", label::gate(phase.gate));
    if phase.gate_hard {
        gate.push_str(" \u{b7} hard");
    }
    if phase.retry_limit > 0 {
        gate.push_str(&format!(" \u{b7} retries {}", phase.retry_limit));
    }
    let who = match phase.split {
        Split::Divide => DIVIDE.to_owned(),
        Split::Compete => format!("compete {TIMES}{}", phase.fan_out),
        Split::None if phase.palette => "palette".to_owned(),
        Split::None => candidate_line(node, agents),
    };
    [template, gate, who]
}

/// D7: the first candidate agent as `{name} · {model}`, ` +{k}` for the others; the name from
/// `agents`, else the first 8 characters of its id. `any agent` with no candidate.
fn candidate_line(node: &NodeBody, agents: &[AgentSummary]) -> String {
    let Some(first) = node.agents.iter().min_by_key(|agent| agent.position) else {
        return "any agent".to_owned();
    };
    let name = agents
        .iter()
        .find(|summary| summary.agent.id == first.agent_id)
        .map_or_else(
            || first.agent_id.to_string().chars().take(8).collect(),
            |summary| summary.agent.name.clone(),
        );
    let mut line = if first.model.is_empty() {
        name
    } else {
        format!("{name} \u{b7} {}", first.model)
    };
    if node.agents.len() > 1 {
        line.push_str(&format!(" +{}", node.agents.len() - 1));
    }
    line
}

/// D7, D11: the six handles of a node, baked into every builder because `set_nodes` resets
/// anything set after it (execution_graph R2). `top` takes a drop and `bottom` starts a drag;
/// the side handles only anchor loops. A palette node hides every handle.
///
/// `top` is hidden too while no connection is being drawn: rataflow draws a visible handle over
/// the border cell above the node's centre, where a name of 9 characters or more (`implement`,
/// `reproduce`) runs. [`Canvas::drop_targets`] shows it on every other flow node for as long as
/// a drag from a `bottom` handle lasts, since rataflow snaps a drop only to a visible handle.
fn handles(palette: bool) -> Vec<Handle> {
    let side = |handle: Handle, id: &str, offset: f64| {
        handle
            .with_id(id)
            .with_offset(offset)
            .with_hidden(true)
            .with_connectable(false)
    };
    vec![
        Handle::target(HandlePosition::Top)
            .with_id("top")
            .with_connectable_start(false)
            .with_hidden(true),
        Handle::source(HandlePosition::Bottom)
            .with_id("bottom")
            .with_connectable_end(false)
            .with_hidden(palette),
        side(Handle::source(HandlePosition::Right), "r-out", 0.75),
        side(Handle::target(HandlePosition::Right), "r-in", 0.25),
        side(Handle::source(HandlePosition::Left), "l-out", 0.75),
        side(Handle::target(HandlePosition::Left), "l-in", 0.25),
    ]
}

/// One step's node at `at`, `selected` baked in.
fn flow_node(
    node: &NodeBody,
    at: (f64, f64),
    selected: bool,
    agents: &[AgentSummary],
    theme: &Theme,
) -> Node<StepNode> {
    let palette = node.phase.palette;
    Node::new(
        node.phase.id.to_string(),
        at,
        (NODE_W, NODE_H),
        StepNode::new(node, agents, theme),
    )
    .with_selected(selected)
    .with_draggable(false)
    .with_deletable(false)
    .with_connectable(!palette)
    .with_handles(handles(palette))
}

/// D7: rataflow's `StepEdge`, but a loop's label sits on the loop's vertical run.
///
/// `StepEdge` puts the label of a side-to-side (right to right) path on its longest leg. When a
/// loop's ends are not stacked (the reference graph's review sits right of implement), that leg
/// is the horizontal one into the target's side, and a label wider than it runs under the target
/// node, which is drawn over it. The vertical run's midpoint lies in the gap between the layers.
#[derive(Debug, Default)]
struct StepLink(StepEdge);

impl EdgeContent for StepLink {
    fn compute_path(&self, ctx: &EdgePathContext) -> Path {
        let path = self.0.compute_path(ctx);
        if ctx.source_position != ctx.target_position || !ctx.source_position.is_horizontal() {
            return path;
        }
        let rise = |run: &[Position]| (run[0].y - run[1].y).abs();
        let run = path
            .points
            .windows(2)
            .filter(|run| (run[0].x - run[1].x).abs() < f64::EPSILON)
            .max_by(|a, b| rise(a).total_cmp(&rise(b)));
        match run {
            Some([from, to]) => {
                let at = Position::new(from.x, f64::midpoint(from.y, to.y));
                path.with_label_position(at)
            }
            _ => path,
        }
    }

    fn render(&self, ctx: &EdgeRenderContext, buf: &mut Buffer) {
        self.0.render(ctx, buf);
    }
}

/// One body edge, by its handles, in the editor's styles (execution_graph B-4).
fn flow_edge(edge: &StepGraphEdge, forward: bool, theme: &Theme) -> Edge<StepLink> {
    let content = StepLink(
        StepEdge::default().with_style(
            EdgeStyle::default()
                .with_stroke_style(theme.dim)
                .with_label_style(theme.base),
        ),
    );
    let (source, target) = edge_handles(edge.kind, forward);
    let drawn = Edge::new(
        edge.id.to_string(),
        edge.from.to_string(),
        edge.to.to_string(),
    )
    .with_content(content)
    .with_source_handle(Some(source.to_owned()))
    .with_target_handle(Some(target.to_owned()))
    .with_deletable(false)
    .with_selectable(true);
    match edge_label(edge) {
        Some(label) => drawn.with_label(label),
        None => drawn,
    }
}

/// Whether rataflow draws into `area` at all (`ui/canvas.rs:42`).
const fn drawable(area: Rect) -> bool {
    area.width >= 2 && area.height >= 2
}

/// What the next `render` does to the viewport before it draws (execution_graph E4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Reveal {
    /// Nothing.
    #[default]
    None,
    /// Keep the selected node on screen (`ensure_node_visible`).
    Cursor,
    /// A new graph: zoom 1, the graph centred, then the selected node kept on screen.
    Reset,
}

/// What a mouse event did on the canvas (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanvasEvent {
    /// Left click on a node.
    Select(PhaseId),
    /// Right click on a node: edit it.
    EditNode(PhaseId),
    /// Click on an edge: the links list on it.
    OpenLinks(EdgeId),
    /// A drag from a node's bottom handle dropped on another node.
    Connect {
        /// The step the drag started on.
        from: PhaseId,
        /// The step it was dropped on.
        to: PhaseId,
    },
}

/// The editor's event for one of rataflow's, if it is one the editor answers.
fn canvas_event(event: FlowEvent) -> Option<CanvasEvent> {
    match event {
        FlowEvent::NodeClicked { node_id } => node_id.parse().ok().map(CanvasEvent::Select),
        FlowEvent::NodeContextMenu { node_id } => node_id.parse().ok().map(CanvasEvent::EditNode),
        FlowEvent::EdgeClicked { edge_id } => edge_id.parse().ok().map(CanvasEvent::OpenLinks),
        FlowEvent::ConnectionCompleted(connection) => Some(CanvasEvent::Connect {
            from: connection.source.parse().ok()?,
            to: connection.target.parse().ok()?,
        }),
        _ => None,
    }
}

/// D7, D11: the canvas of one graph body, rebuilt from every sync.
#[derive(Debug)]
pub(crate) struct Canvas {
    /// The flow. Never fed a key; a mouse event reaches it only through [`Canvas::on_mouse`].
    flow: Flow<StepNode, StepLink>,
    /// The graph last synced, which tells a new graph apart.
    graph: Option<StepGraphId>,
    /// The editor's selected step, which the flow's selection is put back on after every event.
    selected: Option<PhaseId>,
    /// The widest row in world units, for the reset's centring.
    width: f64,
    /// The pending viewport move.
    reveal: Reveal,
    /// The size of the last drawable canvas; a different one reveals the selection.
    drawn: Option<(u16, u16)>,
    /// The terminal cell of a live pan's last pointer, `None` with no pan (MOD-74 D6).
    pan: Option<(u16, u16)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    /// An empty canvas. An edge is never reconnected, a press on empty canvas keeps the selection,
    /// and a drop anywhere on a node snaps to its `top` handle (D11).
    pub(crate) fn new() -> Self {
        Self {
            flow: Flow::new()
                .with_edges_reconnectable(false)
                .with_deselect_on_pane_click(false)
                .with_connection_radius(CONNECTION_RADIUS),
            graph: None,
            selected: None,
            width: 0.0,
            reveal: Reveal::None,
            drawn: None,
            pan: None,
        }
    }

    /// Rebuilds the flow from `body` with `selected` selected (baked into the node builder:
    /// `set_nodes` clears selection and handles, execution_graph R2). Same graph: the viewport
    /// shifts by the selected (else first common) node's move (execution_graph `anchor`), so the
    /// nodes already drawn stay still, and a selection change queues `Reveal::Cursor`. New graph
    /// id: `Reveal::Reset`. An edge naming a step the body lacks is not drawn; a body rataflow
    /// still refuses (a repeated id) leaves the canvas empty rather than panicking.
    pub(crate) fn sync(
        &mut self,
        body: &GraphBody,
        selected: Option<PhaseId>,
        agents: &[AgentSummary],
        theme: &Theme,
    ) {
        let layout = layout(body);
        let same = self.graph == Some(body.graph.id);
        let anchor = if same {
            self.anchor(&layout, selected)
        } else {
            None
        };
        let nodes = layout
            .positions
            .iter()
            .filter_map(|(id, at)| {
                let node = body.nodes.iter().find(|node| node.phase.id == *id)?;
                Some(flow_node(node, *at, selected == Some(*id), agents, theme))
            })
            .collect();
        let known: HashSet<PhaseId> = body.nodes.iter().map(|node| node.phase.id).collect();
        let edges = body
            .edges
            .iter()
            .zip(forward_edges(body))
            .filter(|(edge, _)| {
                edge.from != edge.to && known.contains(&edge.from) && known.contains(&edge.to)
            })
            .map(|(edge, forward)| flow_edge(edge, forward, theme))
            .collect();
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
        self.drop_targets();
        let next = if !same {
            Reveal::Reset
        } else if self.selected != selected {
            Reveal::Cursor
        } else {
            Reveal::None
        };
        self.graph = Some(body.graph.id);
        self.selected = selected;
        self.width = layout.width;
        self.reveal = self.reveal.max(next);
    }

    /// A node drawn before and after a re-layout, its old and new world corner: the selected
    /// one's when it was on the canvas, else the first such node (execution_graph review L2).
    fn anchor(
        &self,
        layout: &Layout,
        selected: Option<PhaseId>,
    ) -> Option<((f64, f64), (f64, f64))> {
        selected
            .into_iter()
            .chain(layout.positions.iter().map(|(id, _)| *id))
            .find_map(|id| {
                let key = id.to_string();
                let old = self.flow.node(&key)?.position;
                let (_, new) = layout.positions.iter().find(|(at, _)| *at == id)?;
                Some(((old.x, old.y), *new))
            })
    }

    /// An empty canvas that remembers no graph.
    fn clear(&mut self) {
        // Neither call can fail on an empty list: nothing duplicates and nothing dangles.
        let _ = self.flow.set_edges(Vec::new());
        let _ = self.flow.set_nodes(Vec::new());
        self.graph = None;
        self.selected = None;
        self.width = 0.0;
        self.reveal = Reveal::None;
    }

    /// D11: the `top` handle of every flow node but the source shown while a connection is being
    /// drawn, so rataflow can snap the drop to it, and hidden otherwise (see [`handles`]).
    fn drop_targets(&mut self) {
        let from = self
            .flow
            .edge_preview()
            .map(|preview| preview.from_node_id.clone());
        let nodes: Vec<String> = self
            .flow
            .nodes()
            .filter(|node| node.connectable)
            .map(|node| node.id.clone())
            .collect();
        for id in nodes {
            let shown = from.as_ref().is_some_and(|from| *from != id);
            let hidden = self
                .flow
                .node(&id)
                .and_then(|node| {
                    node.handles
                        .iter()
                        .find(|handle| handle.id.as_deref() == Some("top"))
                })
                .is_none_or(|handle| handle.hidden);
            if hidden == shown {
                self.flow.set_handle_hidden(&id, "top", !shown);
            }
        }
    }

    /// Puts the flow's selection back on the editor's (the flow keeps none of its own).
    fn reselect(&mut self) {
        match self.selected {
            Some(id) => self.flow.select_node(&id.to_string()),
            None => self.flow.clear_selection(),
        }
    }

    /// `select_node_in_direction`, then the flow's first selected node; `None` leaves the
    /// selection where it was. With nothing selected the first node is the answer.
    pub(crate) fn select_toward(&mut self, direction: Direction) -> Option<PhaseId> {
        self.flow.select_node_in_direction(direction);
        let now = self
            .flow
            .first_selected_node_id()
            .and_then(|id| id.parse::<PhaseId>().ok());
        match now {
            Some(id) if Some(id) != self.selected => {
                self.selected = Some(id);
                self.reveal = self.reveal.max(Reveal::Cursor);
                Some(id)
            }
            _ => {
                self.reselect();
                None
            }
        }
    }

    /// One zoom step (1.2) in, clamped to 2.0. The selection stays on screen.
    pub(crate) fn zoom_in(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomIn);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// One zoom step out, clamped to 0.5.
    pub(crate) fn zoom_out(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomOut);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// The whole graph fitted on the next render; a graph taller than the canvas at the 0.5
    /// floor does not fit whole, so the selection is kept on screen after it.
    pub(crate) fn fit(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::FitView);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// D11: one mouse event on the canvas, in terminal coordinates (rataflow maps them through
    /// the area the last `render` drew). Left and right press, drag and release and the wheel
    /// only (execution_graph :629); every other kind is dropped. A left press on empty canvas
    /// pans, the wheel zooms at the pointer, a click on a node selects it, a right click edits
    /// it, a press on an edge opens its links and a drag from a `bottom` handle dropped on
    /// another node connects them. After every event the flow's selection is put back on the
    /// editor's. Never applies a `FlowAction`, so nothing is ever removed.
    pub(crate) fn on_mouse(&mut self, mouse: MouseEvent) -> Option<CanvasEvent> {
        if !matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
                | MouseEventKind::Drag(MouseButton::Left | MouseButton::Right)
                | MouseEventKind::Up(MouseButton::Left | MouseButton::Right)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
        ) {
            return None;
        }
        let response = self.flow.handle_mouse_event(mouse);
        // MOD-74 D6, B-1: every left press assigns the pan; only an empty-canvas start is one.
        let at = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let pans = response
                    .events()
                    .iter()
                    .any(|event| matches!(event, FlowEvent::PaneClicked { .. }));
                self.pan = pans.then_some(at);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(pan) = self.pan.as_mut() {
                    *pan = at;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.pan = None,
            _ => {}
        }
        self.drop_targets();
        self.reselect();
        response.into_events().find_map(canvas_event)
    }

    /// Ends a live gesture without a click, a connection or a menu (MOD-74 D4): a **locked**
    /// left release resets rataflow's drag state and emits nothing, and a half-drawn connection
    /// preview is dropped.
    pub(crate) fn end_gesture(&mut self) {
        self.pan = None;
        if self.flow.is_dragging() {
            self.locked_left(MouseEventKind::Up(MouseButton::Left), (0, 0));
        }
        self.flow.clear_edge_preview();
        self.drop_targets();
        self.reselect();
    }

    /// One left-button event fed to rataflow with `locked` set (MOD-74 D4, D6): a press starts
    /// `Panning` at the pointer with no hit test, a release only ends the drag. The lock is put
    /// back as it was.
    fn locked_left(&mut self, kind: MouseEventKind, (column, row): (u16, u16)) {
        let locked = self.flow.locked;
        self.flow.locked = true;
        let _ = self.flow.handle_mouse_event(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
        self.flow.locked = locked;
    }

    /// execution_graph `render` (:703): draws `area`, revealing first when a reveal is pending.
    /// A canvas under 2x2 draws nothing, so a reveal stays pending for the next frame; an area
    /// of a new size reveals the selection; the reveal is measured on a scratch buffer; a reset
    /// is zoom 1, the widest row centred, a row of margin; a live pan re-anchors at its last
    /// pointer on what was just drawn (MOD-74 D6).
    pub(crate) fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if !drawable(area) {
            return;
        }
        let size = (area.width, area.height);
        if self.drawn != Some(size) {
            self.drawn = Some(size);
            self.reveal = self.reveal.max(Reveal::Cursor);
        }
        let reveal = core::mem::take(&mut self.reveal);
        if reveal != Reveal::None {
            // The canvas size is known only after a draw (`rataflow` `state/viewport.rs:288`).
            let mut scratch = Buffer::empty(area);
            (&mut self.flow).render(area, &mut scratch);
            if reveal == Reveal::Reset {
                let x = ((f64::from(area.width) - self.width) / 2.0)
                    .floor()
                    .max(MARGIN);
                self.flow.viewport = Viewport::new(x, MARGIN, 1.0);
            }
            if let Some(selected) = self.selected {
                self.flow.ensure_node_visible(&selected.to_string());
            }
        }
        frame.render_widget(&mut self.flow, area);
        if let Some(at) = self.pan
            && self.flow.is_dragging()
        {
            self.locked_left(MouseEventKind::Down(MouseButton::Left), at);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EdgeId, EdgePredicate, GraphScope, NodeBody, StepGraph, StepGraphId, StepGraphPhase,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{MemStore, WriteStore};
    use serde_json::json;

    use super::*;

    /// A fixed clock: nothing drawn reads it.
    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// A library graph row with id `id`.
    fn graph_row(id: StepGraphId) -> StepGraph {
        StepGraph {
            id,
            scope: GraphScope::Library,
            name: "hand-built".to_owned(),
            description: String::new(),
            is_override: false,
            revision: 0,
            created_at: now(),
            updated_at: now(),
        }
    }

    /// A seeded-default node `name` at `position`, its template named after it.
    fn node(graph: StepGraphId, position: i32, name: &str) -> NodeBody {
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
                ..phase_row(PhaseId::new(), graph, position, &seed, now())
            },
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// The id of the step named `name` in `body`.
    fn id_of(body: &GraphBody, name: &str) -> PhaseId {
        body.nodes
            .iter()
            .find(|node| node.phase.name == name)
            .map(|node| node.phase.id)
            .unwrap_or_else(|| panic!("no step {name}"))
    }

    /// An edge `from -> to` of `body`'s graph, a loop limit of 1 where the kind needs one.
    fn edge(body: &GraphBody, from: &str, to: &str, kind: EdgeKind) -> StepGraphEdge {
        StepGraphEdge {
            id: EdgeId::new(),
            graph_id: body.graph.id,
            from: id_of(body, from),
            to: id_of(body, to),
            kind,
            predicate: None,
            loop_limit: kind.needs_loop_limit().then_some(1),
        }
    }

    /// A body of `names` at positions 0.., no edges.
    fn body(names: &[&str]) -> GraphBody {
        let graph = StepGraphId::new();
        GraphBody {
            graph: graph_row(graph),
            nodes: (0..)
                .zip(names)
                .map(|(position, name)| node(graph, position, name))
                .collect(),
            edges: Vec::new(),
        }
    }

    /// `{"verify": "failed"}`.
    fn verify_failed() -> Option<EdgePredicate> {
        Some(EdgePredicate(json!({"verify": "failed"})))
    }

    /// The reference graph: `implement -next-> review`, `review -changes_requested(3)->
    /// implement`, `implement -if {"verify":"failed"}-> fix`.
    fn reference() -> GraphBody {
        let mut body = body(&["implement", "review", "fix"]);
        let mut back = edge(&body, "review", "implement", EdgeKind::ChangesRequested);
        back.loop_limit = Some(3);
        let mut branch = edge(&body, "implement", "fix", EdgeKind::If);
        branch.predicate = verify_failed();
        body.edges = vec![
            edge(&body, "implement", "review", EdgeKind::Next),
            back,
            branch,
        ];
        body
    }

    /// A seeded demo graph's body.
    async fn demo(graph: StepGraphId) -> GraphBody {
        MemStore::demo()
            .graph_body(graph)
            .await
            .expect("a memory read")
            .expect("the seeded graph")
    }

    /// The `(x, y)` of `name`'s node in `layout`.
    fn at(body: &GraphBody, layout: &Layout, name: &str) -> (f64, f64) {
        let id = id_of(body, name);
        layout
            .positions
            .iter()
            .find(|(at, _)| *at == id)
            .map(|(_, at)| *at)
            .unwrap_or_else(|| panic!("{name} is placed"))
    }

    /// The layer `name` sits in.
    fn layer_of(body: &GraphBody, layout: &Layout, name: &str) -> f64 {
        at(body, layout, name).1 / Y_STEP
    }

    #[tokio::test]
    async fn the_feature_seed_layers_one_step_per_layer() {
        let body = demo(ids::GRAPH_VULKAN_FEAT).await;
        let layout = layout(&body);
        assert_eq!(layout.layers, 4);
        for (layer, name) in [
            (0.0, "prd"),
            (1.0, "plan"),
            (2.0, "implement"),
            (3.0, "review"),
        ] {
            assert!(
                (layer_of(&body, &layout, name) - layer).abs() < f64::EPSILON,
                "{name}: {layout:?}"
            );
            assert!(at(&body, &layout, name).0.abs() < f64::EPSILON, "{name}");
        }
        assert!((layout.width - NODE_W).abs() < f64::EPSILON);
        assert!(
            body.edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::ChangesRequested),
            "the seed carries the review loop"
        );
    }

    #[test]
    fn back_edges_do_not_move_layers() {
        let mut body = body(&["plan", "build", "review"]);
        body.edges = vec![
            edge(&body, "plan", "build", EdgeKind::Next),
            edge(&body, "build", "review", EdgeKind::Next),
        ];
        let before = layout(&body);
        let mut back = edge(&body, "review", "plan", EdgeKind::AsksYou);
        back.loop_limit = None;
        body.edges.push(back);
        body.edges
            .push(edge(&body, "review", "build", EdgeKind::Fails));
        body.edges
            .push(edge(&body, "review", "build", EdgeKind::ChangesRequested));
        assert_eq!(layout(&body), before);
        assert_eq!(before.layers, 3);
    }

    #[test]
    fn an_if_branch_puts_both_targets_in_one_layer_centred() {
        let body = reference();
        let layout = layout(&body);
        assert_eq!(layout.layers, 2);
        // Review's loop runs on the right, so review sits right of fix.
        assert_eq!(at(&body, &layout, "fix"), (0.0, Y_STEP));
        assert_eq!(at(&body, &layout, "review"), (X_STEP, Y_STEP));
        assert!((layout.width - span(2)).abs() < f64::EPSILON);
        // Centred on the two below, in whole cells.
        assert_eq!(
            at(&body, &layout, "implement"),
            (((span(2) - NODE_W) / 2.0).floor(), 0.0)
        );
    }

    #[test]
    fn a_loop_end_sits_on_its_loop_s_side_of_its_layer() {
        let mut body = body(&["plan", "a", "b", "c"]);
        body.edges = vec![
            edge(&body, "plan", "a", EdgeKind::Next),
            edge(&body, "plan", "b", EdgeKind::Next),
            edge(&body, "plan", "c", EdgeKind::Next),
        ];
        let plain = layout(&body);
        let column =
            |body: &GraphBody, layout: &Layout, name: &str| at(body, layout, name).0 / X_STEP;
        assert_eq!(
            ["a", "b", "c"].map(|name| column(&body, &plain, name)),
            [0.0, 1.0, 2.0]
        );
        // `a` loops on the right, `c` on the left: they swap ends, `b` keeps the middle.
        body.edges
            .push(edge(&body, "a", "plan", EdgeKind::ChangesRequested));
        body.edges.push(edge(&body, "c", "plan", EdgeKind::Fails));
        let sided = layout(&body);
        assert_eq!(
            ["c", "b", "a"].map(|name| column(&body, &sided, name)),
            [0.0, 1.0, 2.0]
        );
    }

    #[test]
    fn palette_nodes_sit_in_their_own_band_below() {
        let mut body = body(&["plan", "triage", "build", "lint"]);
        body.nodes[1].phase.palette = true;
        body.nodes[3].phase.palette = true;
        body.edges = vec![edge(&body, "plan", "build", EdgeKind::Next)];
        let layout = layout(&body);
        assert_eq!(layout.layers, 2);
        assert_eq!(at(&body, &layout, "triage"), (0.0, 3.0 * Y_STEP));
        assert_eq!(at(&body, &layout, "lint"), (X_STEP, 3.0 * Y_STEP));
        // The palette row is the widest, so the flow is centred on it.
        assert!((layout.width - span(2)).abs() < f64::EPSILON);
        let left = ((span(2) - NODE_W) / 2.0).floor();
        assert_eq!(at(&body, &layout, "plan"), (left, 0.0));
        assert_eq!(at(&body, &layout, "build"), (left, Y_STEP));
        // Flow layers first, then the band.
        let order: Vec<PhaseId> = layout.positions.iter().map(|(id, _)| *id).collect();
        let names = ["plan", "build", "triage", "lint"].map(|name| id_of(&body, name));
        assert_eq!(order, names);

        let mut alone = self::body(&["triage"]);
        alone.nodes[0].phase.palette = true;
        let layout = super::layout(&alone);
        assert_eq!(layout.layers, 0);
        assert_eq!(
            layout.positions,
            vec![(alone.nodes[0].phase.id, (0.0, 0.0))]
        );
    }

    #[tokio::test]
    async fn layout_is_deterministic_and_ignores_edge_order() {
        for mut body in [demo(ids::GRAPH_VULKAN_FEAT).await, reference()] {
            let first = layout(&body);
            assert_eq!(layout(&body), first);
            body.edges.reverse();
            assert_eq!(layout(&body), first);
            body.edges.rotate_left(1);
            assert_eq!(layout(&body), first);
        }
    }

    #[test]
    fn a_forward_cycle_still_lays_out() {
        let mut body = body(&["root", "a", "b"]);
        body.edges = vec![
            edge(&body, "root", "a", EdgeKind::Next),
            edge(&body, "a", "b", EdgeKind::Next),
            edge(&body, "b", "a", EdgeKind::Next),
        ];
        let layout = layout(&body);
        assert_eq!(layout.positions.len(), 3);
        let mut ys: Vec<f64> = layout.positions.iter().map(|(_, (_, y))| *y).collect();
        ys.dedup();
        assert_eq!(ys.len(), 3, "no two steps overlap: {layout:?}");

        let mut empty = self::body(&[]);
        empty.edges.clear();
        assert_eq!(
            super::layout(&empty),
            Layout {
                positions: Vec::new(),
                layers: 0,
                width: 0.0,
            }
        );
    }

    #[test]
    fn edge_handles_by_kind_and_direction() {
        for kind in super::super::RELATIONS {
            assert_eq!(edge_handles(kind, true), ("bottom", "top"), "{kind:?}");
        }
        assert_eq!(
            edge_handles(EdgeKind::ChangesRequested, false),
            ("r-out", "r-in")
        );
        assert_eq!(edge_handles(EdgeKind::AsksYou, false), ("r-out", "r-in"));
        assert_eq!(edge_handles(EdgeKind::Fails, false), ("l-out", "l-in"));
        assert_eq!(edge_handles(EdgeKind::Next, false), ("bottom", "top"));
        assert_eq!(edge_handles(EdgeKind::If, false), ("bottom", "top"));
    }

    #[test]
    fn edge_labels_read_in_words() {
        let body = body(&["a", "b"]);
        let labelled = |kind: EdgeKind, limit: Option<i32>, predicate: Option<Value>| {
            let mut edge = edge(&body, "a", "b", kind);
            edge.loop_limit = limit;
            edge.predicate = predicate.map(EdgePredicate);
            edge_label(&edge)
        };
        assert_eq!(labelled(EdgeKind::Next, None, None), None);
        assert_eq!(
            labelled(EdgeKind::AsksYou, None, None).as_deref(),
            Some("asks you")
        );
        assert_eq!(
            labelled(EdgeKind::AsksYou, Some(2), None).as_deref(),
            Some("asks you \u{d7}2")
        );
        assert_eq!(
            labelled(EdgeKind::ChangesRequested, Some(3), None).as_deref(),
            Some("changes requested \u{d7}3")
        );
        assert_eq!(
            labelled(EdgeKind::Fails, Some(1), None).as_deref(),
            Some("fails \u{d7}1")
        );
        let words = |predicate: Value| labelled(EdgeKind::If, None, Some(predicate));
        for (predicate, want) in [
            (json!({"verify": "failed"}), "if verify failed"),
            (json!({"verify": "passed"}), "if verify passed"),
            (json!({"item_kind": "FIX"}), "if kind FIX"),
            (json!({"item_tag": "x"}), "if tag x"),
            (
                json!({"any_changed_path_matches": ["src/**"]}),
                "if any path src/**",
            ),
            (
                json!({"any_changed_path_matches": ["src/**", "a", "b"]}),
                "if any path src/** +2",
            ),
            (
                json!({"all_changed_paths_match": ["docs/*"]}),
                "if all paths docs/*",
            ),
            (
                json!({"all_changed_paths_match": ["docs/*", "x"]}),
                "if all paths docs/* +1",
            ),
            (
                json!({"not": {"verify": "passed"}}),
                "if not (verify passed)",
            ),
            (
                json!({"all": [{"verify": "passed"}, {"item_tag": "x"}]}),
                "if all of 2",
            ),
            (json!({"any": [{"item_kind": "FIX"}]}), "if any of 1"),
            (json!({"bogus": 1}), "if {\"bogus\":1}"),
        ] {
            assert_eq!(
                words(predicate.clone()).as_deref(),
                Some(want),
                "{predicate}"
            );
        }
        // Clipped to 24 cells with an ellipsis.
        let long =
            words(json!({"item_tag": "a-very-long-tag-name-indeed"})).expect("an if has a label");
        assert_eq!(cells::cell_width(&long), LABEL_W);
        assert!(long.ends_with('\u{2026}'), "{long}");
    }

    // --- the flow ---

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use htui_core::model::{AgentSummary, PhaseAgent, Split};
    use ratatui::buffer::Buffer;

    /// A canvas synced to `body`, `selected` selected, no agents.
    fn synced(body: &GraphBody, selected: Option<PhaseId>) -> Canvas {
        let mut canvas = Canvas::new();
        canvas.sync(body, selected, &[], &Theme::default());
        canvas
    }

    /// One frame of `canvas` at `width` x `height`, at the terminal's origin, so a buffer cell is
    /// the terminal cell rataflow maps.
    fn draw_at(canvas: &mut Canvas, width: u16, height: u16) -> Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
            .expect("the test backend is constructible");
        term.draw(|frame| canvas.render(frame, frame.area()))
            .expect("the canvas draws");
        term.backend().buffer().clone()
    }

    /// One frame at the snapshot size, 100x28.
    fn draw(canvas: &mut Canvas) -> Buffer {
        draw_at(canvas, 100, 28)
    }

    /// Each row as a `String`, trailing blanks trimmed (execution_graph `rows`).
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

    /// The `(x, y)` of the `╭` of the node titled `name` (one cell per char on these rows).
    fn corner_of(buf: &Buffer, name: &str) -> (u16, u16) {
        let needle = format!("\u{256d}\u{2500} {name} ");
        rows(buf)
            .iter()
            .enumerate()
            .find_map(|(y, row)| {
                let byte = row.find(&needle)?;
                let x = row[..byte].chars().count();
                Some((
                    u16::try_from(x).expect("the column fits"),
                    u16::try_from(y).expect("the row fits"),
                ))
            })
            .unwrap_or_else(|| panic!("{name} is drawn: {:#?}", rows(buf)))
    }

    /// `kind` at cell `(column, row)`, no modifier.
    fn mouse(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A press of `button` then its release at `at`: the release's answer (the press answers
    /// nothing).
    fn click_with(canvas: &mut Canvas, button: MouseButton, at: (u16, u16)) -> Option<CanvasEvent> {
        assert_eq!(
            canvas.on_mouse(mouse(MouseEventKind::Down(button), at)),
            None,
            "a press answers nothing yet"
        );
        canvas.on_mouse(mouse(MouseEventKind::Up(button), at))
    }

    /// The flow's selected node, as a step id.
    fn flow_selected(canvas: &Canvas) -> Option<PhaseId> {
        canvas
            .flow
            .first_selected_node_id()
            .and_then(|id| id.parse().ok())
    }

    /// How many nodes the flow has selected.
    fn selected_count(canvas: &Canvas) -> usize {
        canvas.flow.nodes().filter(|node| node.selected).count()
    }

    /// Every node's id and world corner, in flow order.
    type Corners = Vec<(String, (f64, f64))>;

    /// Every node's id and world corner, and every edge's id, in flow order.
    fn shape(canvas: &Canvas) -> (Corners, Vec<String>) {
        (
            canvas
                .flow
                .nodes()
                .map(|node| (node.id.clone(), (node.position.x, node.position.y)))
                .collect(),
            canvas
                .flow
                .edges()
                .iter()
                .map(|edge| edge.id.clone())
                .collect(),
        )
    }

    /// The flow edge of body edge `edge`.
    fn edge_id_between(body: &GraphBody, from: &str, to: &str) -> EdgeId {
        let (from, to) = (id_of(body, from), id_of(body, to));
        body.edges
            .iter()
            .find(|edge| edge.from == from && edge.to == to)
            .map(|edge| edge.id)
            .expect("the edge exists")
    }

    #[test]
    fn sync_bakes_selection_and_handles_and_keeps_the_viewport_on_a_re_sync() {
        let theme = Theme::default();
        let body = reference();
        let implement = id_of(&body, "implement");
        let mut canvas = synced(&body, Some(implement));
        draw(&mut canvas);
        canvas.zoom_in();
        draw(&mut canvas);
        let viewport = canvas.flow.viewport;
        assert!(canvas.flow.viewport.zoom > 1.0);

        // A rename re-syncs the same graph: the viewport, the selection and the handles stay.
        let mut renamed = body.clone();
        renamed.nodes[2].phase.name = "repair".to_owned();
        canvas.sync(&renamed, Some(implement), &[], &theme);
        assert_eq!(canvas.flow.viewport, viewport);
        assert_eq!(flow_selected(&canvas), Some(implement));
        assert_eq!(selected_count(&canvas), 1);
        for node in canvas.flow.nodes() {
            assert!(node.connectable && !node.draggable && !node.deletable);
            let shown: Vec<&str> = node
                .handles
                .iter()
                .filter(|handle| !handle.hidden)
                .filter_map(|handle| handle.id.as_deref())
                .collect();
            assert_eq!(shown, ["bottom"], "{}", node.id);
            assert_eq!(node.handles.len(), 6);
        }
        assert!(!canvas.flow.edges_reconnectable);
        assert!(canvas.flow.edges().iter().all(|edge| !edge.deletable));
        assert!(canvas.flow.edges().iter().all(|edge| edge.selectable));
        let back = canvas
            .flow
            .edge(&edge_id_between(&body, "review", "implement").to_string())
            .expect("the loop is drawn");
        assert_eq!(back.source_handle.as_deref(), Some("r-out"));
        assert_eq!(back.target_handle.as_deref(), Some("r-in"));
        draw(&mut canvas);
        assert_eq!(canvas.flow.viewport, viewport);

        // A palette node hides every handle and takes no connection.
        let mut palette = renamed.clone();
        palette.nodes[2].phase.palette = true;
        palette.edges.truncate(2);
        canvas.sync(&palette, Some(implement), &[], &theme);
        let node = canvas
            .flow
            .node(&palette.nodes[2].phase.id.to_string())
            .expect("the palette node is drawn");
        assert!(!node.connectable);
        assert!(node.handles.iter().all(|handle| handle.hidden));

        // A new graph resets the zoom.
        let other = self::body(&["solo"]);
        canvas.sync(&other, None, &[], &theme);
        draw(&mut canvas);
        assert!((canvas.flow.viewport.zoom - 1.0).abs() < f64::EPSILON);
        assert_eq!(canvas.graph, Some(other.graph.id));
    }

    #[tokio::test]
    async fn select_toward_moves_to_the_neighbour() {
        let body = demo(ids::GRAPH_VULKAN_FEAT).await;
        let (implement, review) = (id_of(&body, "implement"), id_of(&body, "review"));
        let mut canvas = synced(&body, Some(implement));
        draw(&mut canvas);
        assert_eq!(canvas.select_toward(Direction::Down), Some(review));
        assert_eq!(flow_selected(&canvas), Some(review));
        // Nothing below review: the selection stays.
        assert_eq!(canvas.select_toward(Direction::Down), None);
        assert_eq!(flow_selected(&canvas), Some(review));
        assert_eq!(canvas.select_toward(Direction::Up), Some(implement));
    }

    #[tokio::test]
    async fn zoom_and_fit_stay_in_range() {
        let body = demo(ids::GRAPH_VULKAN_FEAT).await;
        let mut canvas = synced(&body, Some(id_of(&body, "prd")));
        draw(&mut canvas);
        for _ in 0..10 {
            canvas.zoom_out();
        }
        draw(&mut canvas);
        assert!((canvas.flow.viewport.zoom - 0.5).abs() < 1e-9);
        for _ in 0..10 {
            canvas.zoom_in();
        }
        draw(&mut canvas);
        assert!((canvas.flow.viewport.zoom - 2.0).abs() < 1e-9);
        canvas.fit();
        draw(&mut canvas);
        let zoom = canvas.flow.viewport.zoom;
        assert!((0.5..=2.0).contains(&zoom), "{zoom}");
        assert!(zoom < 2.0, "fit zooms out to the four layers: {zoom}");
    }

    #[test]
    fn a_click_on_a_node_selects_it() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let (x, y) = corner_of(&buf, "review");
        assert_eq!(
            click_with(&mut canvas, MouseButton::Left, (x + 3, y + 2)),
            Some(CanvasEvent::Select(id_of(&body, "review")))
        );
        // The editor moves the selection; the canvas keeps its own until the next sync.
        assert_eq!(flow_selected(&canvas), Some(id_of(&body, "implement")));
        assert_eq!(selected_count(&canvas), 1);
    }

    #[test]
    fn a_right_click_on_a_node_edits_it() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let (x, y) = corner_of(&buf, "fix");
        assert_eq!(
            click_with(&mut canvas, MouseButton::Right, (x + 3, y + 2)),
            Some(CanvasEvent::EditNode(id_of(&body, "fix")))
        );
        assert_eq!(flow_selected(&canvas), Some(id_of(&body, "implement")));
    }

    #[test]
    fn a_click_on_an_edge_opens_its_links() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        // The loop leaves review's right side and climbs to implement's: a cell of its climb,
        // in the gap between the layers and clear of every node and handle.
        let (review_x, review_y) = corner_of(&buf, "review");
        let row = review_y - 2;
        let column = (review_x + 24..review_x + 30)
            .find(|column| buf[(*column, row)].symbol() != " ")
            .unwrap_or_else(|| panic!("the loop climbs right of review: {:#?}", rows(&buf)));
        // An edge answers on the press (rataflow's `EdgeClicked`); the release adds nothing.
        assert_eq!(
            canvas.on_mouse(mouse(
                MouseEventKind::Down(MouseButton::Left),
                (column, row)
            )),
            Some(CanvasEvent::OpenLinks(edge_id_between(
                &body,
                "review",
                "implement"
            )))
        );
        assert_eq!(
            canvas.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), (column, row))),
            None
        );
        assert_eq!(flow_selected(&canvas), Some(id_of(&body, "implement")));
        assert_eq!(selected_count(&canvas), 1);
        assert!(canvas.flow.edges().iter().all(|edge| !edge.selected));
    }

    #[test]
    fn a_drag_from_a_bottom_handle_onto_a_node_connects() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let (x, y) = corner_of(&buf, "review");
        let (fix_x, fix_y) = corner_of(&buf, "fix");
        let handle = (x + 12, y + 4);
        let drop = (fix_x + 12, fix_y + 2);
        for (kind, at) in [
            (MouseEventKind::Down(MouseButton::Left), handle),
            (MouseEventKind::Drag(MouseButton::Left), (x + 20, y + 6)),
            (MouseEventKind::Drag(MouseButton::Left), drop),
        ] {
            assert_eq!(canvas.on_mouse(mouse(kind, at)), None);
        }
        assert_eq!(
            canvas.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), drop)),
            Some(CanvasEvent::Connect {
                from: id_of(&body, "review"),
                to: id_of(&body, "fix"),
            })
        );
        assert_eq!(flow_selected(&canvas), Some(id_of(&body, "implement")));
        // The edge is the editor's to add: the flow did not grow one.
        assert_eq!(canvas.flow.edges().len(), body.edges.len());
    }

    #[test]
    fn drop_targets_show_only_while_a_connection_is_drawn() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let shown = |canvas: &Canvas| -> Vec<String> {
            canvas
                .flow
                .nodes()
                .filter(|node| {
                    node.handles
                        .iter()
                        .any(|handle| handle.id.as_deref() == Some("top") && !handle.hidden)
                })
                .map(|node| node.id.clone())
                .collect()
        };
        assert!(shown(&canvas).is_empty());
        let (x, y) = corner_of(&buf, "review");
        let _ = canvas.on_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            (x + 12, y + 4),
        ));
        let mut others: Vec<String> = ["implement", "fix"]
            .map(|name| id_of(&body, name).to_string())
            .to_vec();
        others.sort();
        let mut now = shown(&canvas);
        now.sort();
        assert_eq!(now, others);
        let _ = canvas.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), (99, 0)));
        assert!(shown(&canvas).is_empty());
    }

    #[test]
    fn no_mouse_event_removes_a_node_or_an_edge() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let before = shape(&canvas);
        let (x, y) = corner_of(&buf, "review");
        let (fix_x, fix_y) = corner_of(&buf, "fix");
        let left = MouseButton::Left;
        let right = MouseButton::Right;
        for (kind, at) in [
            // A node dragged away (no node moves), an edge pressed, a pan.
            (MouseEventKind::Down(left), (x + 3, y + 2)),
            (MouseEventKind::Drag(left), (x + 40, y + 5)),
            (MouseEventKind::Up(left), (x + 40, y + 5)),
            (MouseEventKind::Down(left), (1, 26)),
            (MouseEventKind::Drag(left), (5, 20)),
            (MouseEventKind::Up(left), (5, 20)),
            // A right-drag box over everything, the wheel both ways.
            (MouseEventKind::Down(right), (0, 0)),
            (MouseEventKind::Drag(right), (99, 27)),
            (MouseEventKind::Up(right), (99, 27)),
            (MouseEventKind::ScrollUp, (fix_x + 3, fix_y + 2)),
            (MouseEventKind::ScrollDown, (fix_x + 3, fix_y + 2)),
            // A connection dropped on nothing, then one dropped on its own node.
            (MouseEventKind::Down(left), (x + 12, y + 4)),
            (MouseEventKind::Drag(left), (99, 0)),
            (MouseEventKind::Up(left), (99, 0)),
            (MouseEventKind::Moved, (x + 3, y + 2)),
        ] {
            let _ = canvas.on_mouse(mouse(kind, at));
            draw(&mut canvas);
            let (nodes, edges) = shape(&canvas);
            assert_eq!(nodes.len(), before.0.len(), "{kind:?}");
            assert_eq!(edges, before.1, "{kind:?}");
        }
        let (nodes, _) = shape(&canvas);
        let ids: Vec<&String> = nodes.iter().map(|(id, _)| id).collect();
        let was: Vec<&String> = before.0.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, was);
        assert_eq!(nodes, before.0, "no node moved");
        assert_eq!(flow_selected(&canvas), Some(id_of(&body, "implement")));
        assert_eq!(selected_count(&canvas), 1);
    }

    #[test]
    fn ending_a_gesture_drops_a_half_drawn_connection() {
        let body = reference();
        let mut canvas = synced(&body, Some(id_of(&body, "implement")));
        let buf = draw(&mut canvas);
        let (x, y) = corner_of(&buf, "review");
        let _ = canvas.on_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            (x + 12, y + 4),
        ));
        let _ = canvas.on_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            (x + 40, y + 2),
        ));
        assert!(canvas.flow.is_dragging());
        canvas.end_gesture();
        assert!(!canvas.flow.is_dragging());
        assert!(canvas.flow.edge_preview().is_none());
        let (fix_x, fix_y) = corner_of(&buf, "fix");
        assert_eq!(
            canvas.on_mouse(mouse(
                MouseEventKind::Up(MouseButton::Left),
                (fix_x + 12, fix_y + 2)
            )),
            None,
            "the release after an ended gesture connects nothing"
        );
    }

    #[test]
    fn node_lines_read_template_gate_and_who() {
        let mut node = node(StepGraphId::new(), 0, "review");
        node.phase.template_version = Some(2);
        node.phase.gate_hard = true;
        node.phase.retry_limit = 3;
        let agent = htui_core::model::AgentId::new();
        node.agents = vec![
            PhaseAgent {
                phase_id: node.phase.id,
                position: 0,
                agent_id: agent,
                model: "opus".to_owned(),
            },
            PhaseAgent {
                phase_id: node.phase.id,
                position: 1,
                agent_id: htui_core::model::AgentId::new(),
                model: "sonnet".to_owned(),
            },
        ];
        let lines = node_lines(&node, &[]);
        assert_eq!(lines[0], "template review v2");
        assert_eq!(lines[1], "gate always \u{b7} hard \u{b7} retries 3");
        let short: String = agent.to_string().chars().take(8).collect();
        assert_eq!(lines[2], format!("{short} \u{b7} opus +1"));
        node.agents.clear();
        node.phase.retry_limit = 0;
        node.phase.gate_hard = false;
        node.phase.template_version = None;
        let lines = node_lines(&node, &[]);
        assert_eq!(lines[..2], ["template review", "gate always"]);
        assert_eq!(lines[2], "any agent");
        node.phase.palette = true;
        assert_eq!(node_lines(&node, &[])[2], "palette");
        node.phase.split = Split::Compete;
        node.phase.fan_out = 3;
        assert_eq!(node_lines(&node, &[])[2], "compete \u{d7}3");
        node.phase.split = Split::Divide;
        assert_eq!(node_lines(&node, &[])[2], "divide \u{b7} needs MOD-139");
        assert!(
            cells::cell_width(DIVIDE) <= 22,
            "the badge fits the interior"
        );
    }

    #[tokio::test]
    async fn node_lines_name_a_known_agent() {
        let agents = demo_agents().await;
        let known = agents.first().expect("the demo registry has agents");
        let mut node = node(StepGraphId::new(), 0, "review");
        node.agents = vec![PhaseAgent {
            phase_id: node.phase.id,
            position: 0,
            agent_id: known.agent.id,
            model: "opus".to_owned(),
        }];
        assert_eq!(
            node_lines(&node, &agents)[2],
            format!("{} \u{b7} opus", known.agent.name)
        );
    }

    // --- snapshots ---

    /// The demo registry, for the agents line.
    async fn demo_agents() -> Vec<AgentSummary> {
        MemStore::demo().agents().await.expect("a memory read")
    }

    /// `body` drawn into a `width` x `height` buffer, `selected` selected, as text.
    fn drawn(
        body: &GraphBody,
        selected: Option<&str>,
        agents: &[AgentSummary],
        (width, height): (u16, u16),
    ) -> String {
        let mut canvas = Canvas::new();
        let selected = selected.map(|name| id_of(body, name));
        canvas.sync(body, selected, agents, &Theme::default());
        rows(&draw_at(&mut canvas, width, height)).join("\n")
    }

    #[tokio::test]
    async fn snapshot_linear() {
        let body = demo(ids::GRAPH_VULKAN_ANA).await;
        insta::assert_snapshot!(
            "linear",
            drawn(&body, Some("research"), &demo_agents().await, (100, 28))
        );
    }

    #[tokio::test]
    async fn snapshot_review_loop() {
        let body = demo(ids::GRAPH_VULKAN_FEAT).await;
        insta::assert_snapshot!(
            "review_loop",
            drawn(&body, Some("implement"), &demo_agents().await, (100, 36))
        );
    }

    #[test]
    fn snapshot_if_branch() {
        let mut body = body(&["implement", "review", "fix"]);
        let mut branch = edge(&body, "implement", "fix", EdgeKind::If);
        branch.predicate = verify_failed();
        body.edges = vec![edge(&body, "implement", "review", EdgeKind::Next), branch];
        insta::assert_snapshot!("if_branch", drawn(&body, None, &[], (100, 28)));
    }

    #[tokio::test]
    async fn snapshot_palette() {
        let mut body = demo(ids::GRAPH_VULKAN_FIX).await;
        let last = body.nodes.iter().map(|node| node.phase.position).max();
        let mut triage = node(body.graph.id, last.unwrap_or(0) + 1, "triage");
        triage.phase.palette = true;
        body.nodes.push(triage);
        insta::assert_snapshot!(
            "palette",
            drawn(&body, None, &demo_agents().await, (100, 40))
        );
    }

    #[tokio::test]
    async fn snapshot_divide_badge() {
        let mut body = demo(ids::GRAPH_VULKAN_ANA).await;
        let verdict = id_of(&body, "verdict");
        for node in &mut body.nodes {
            if node.phase.id == verdict {
                node.phase.split = Split::Divide;
            }
        }
        insta::assert_snapshot!(
            "divide_badge",
            drawn(&body, None, &demo_agents().await, (100, 28))
        );
    }

    #[test]
    fn snapshot_reference() {
        insta::assert_snapshot!(
            "reference",
            drawn(&reference(), Some("review"), &[], (100, 28))
        );
    }
}
