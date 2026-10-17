//! rataflow graphs: an item's neighbourhood (Graph), a run's steps (Runs), and requirements.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use rataflow::{Edge, EdgeStyle, FitViewOptions, Flow, Node, NodeContent, NodeRenderContext, StepEdge, Sugiyama, Theme};

use crate::data::{Item, Requirement, Status, Workflow};
use crate::theme::{self, Palette};

pub type Graph = Flow<StatusNode, StepEdge>;

const NODE_W: f64 = 30.0;
const NODE_H: f64 = 4.0;

/// How relationship kinds are shown: a legend box, a label on each link, both, or neither.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkLabels {
    Legend,
    OnLinks,
    Both,
    Off,
}

impl LinkLabels {
    pub fn next(self) -> Self {
        match self {
            LinkLabels::Legend => LinkLabels::OnLinks,
            LinkLabels::OnLinks => LinkLabels::Both,
            LinkLabels::Both => LinkLabels::Off,
            LinkLabels::Off => LinkLabels::Legend,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            LinkLabels::Legend => "legend",
            LinkLabels::OnLinks => "labels on links",
            LinkLabels::Both => "legend + labels",
            LinkLabels::Off => "no labels",
        }
    }
    pub fn legend(self) -> bool {
        matches!(self, LinkLabels::Legend | LinkLabels::Both)
    }
    fn labels(self) -> bool {
        matches!(self, LinkLabels::OnLinks | LinkLabels::Both)
    }
}

/// Graph view options, shared by every graph.
#[derive(Clone, Copy, Debug)]
pub struct GraphOpts {
    pub labels: LinkLabels,
    pub minimap: bool,
    pub horizontal: bool,
}

impl GraphOpts {
    /// Part of a graph's cache key: options that change how it is built.
    pub fn id(&self) -> String {
        format!("{}{}", u8::from(self.labels.labels()), u8::from(self.horizontal))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Item,
    Requirement,
    Step,
}

/// One box: status glyph and name on top, a detail line under it.
#[derive(Debug, Clone)]
pub struct StatusNode {
    head: String,
    head_color: Color,
    sub: String,
    status: Status,
    accent: Color,
    /// The thing the graph is about: drawn with a heavier border.
    root: bool,
    shape: Shape,
}

impl NodeContent for StatusNode {
    fn render(&self, ctx: &NodeRenderContext, buf: &mut Buffer) {
        let border = if ctx.selected {
            Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::status_color(self.status))
        };
        let kind = match self.shape {
            _ if self.root || ctx.selected => BorderType::Thick,
            Shape::Requirement => BorderType::LightDoubleDashed,
            _ => BorderType::Rounded,
        };
        let block = Block::bordered().border_type(kind).border_style(border);
        let inner = block.inner(ctx.area);
        block.render(ctx.area, buf);
        if inner.height == 0 {
            return;
        }
        let dimmed = matches!(self.status, Status::Closed | Status::Queued);
        let glyph = match self.shape {
            Shape::Requirement => Span::styled("◇", Style::new().fg(theme::status_color(self.status))),
            _ => theme::glyph(self.status),
        };
        let head = Line::from(vec![glyph, Span::raw(" "), Span::styled(self.head.clone(), Style::new().fg(self.head_color).add_modifier(Modifier::BOLD))]);
        head.render(Rect { height: 1, ..inner }, buf);
        if inner.height > 1 {
            let style = if dimmed { theme::dim() } else { theme::text() };
            Line::from(Span::styled(self.sub.clone(), style)).render(Rect { y: inner.y + 1, height: 1, ..inner }, buf);
        }
    }
}

/// A relationship kind: its colour, whether its line is dotted, and what it means.
/// Every arrow points down the page: from what comes first to what follows.
pub fn rel_style(kind: &str) -> (Color, bool, &'static str) {
    match kind {
        "refines" => (theme::status_color(Status::AwaitingApproval), false, "parent → child requirement"),
        "covered by" => (theme::status_color(Status::AwaitingApproval), true, "requirement → item covering it"),
        "blocks" => (theme::TEXT, false, "must be done first"),
        "follow-up" => (theme::kind_color("MOD"), false, "item → what came out of it"),
        "relates to" => (theme::kind_color("FEAT"), true, "related, no order"),
        "fans out" => (theme::kind_color("ANA"), false, "parallel agents, one part each"),
        "merges" => (theme::kind_color("ANA"), true, "their results combined"),
        "changes requested" => (theme::status_color(Status::Failed), false, "review sends it back"),
        "approved" => (theme::status_color(Status::Done), false, "review passes it on"),
        _ => (theme::DIM, false, "next step"),
    }
}

/// The legend for the kinds a graph uses.
pub fn legend_lines(kinds: &[&str]) -> Vec<Line<'static>> {
    let w = kinds.iter().map(|k| k.chars().count()).max().unwrap_or(0) + 1;
    kinds
        .iter()
        .map(|k| {
            let (color, dotted, meaning) = rel_style(k);
            Line::from(vec![
                Span::styled(if dotted { " ┄┄▶ " } else { " ──▶ " }, Style::new().fg(color)),
                Span::styled(format!("{k:<w$}"), Style::new().fg(color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{meaning} "), theme::dim()),
            ])
        })
        .collect()
}

fn edge(id: usize, source: &str, target: &str, kind: &str, opts: GraphOpts, animated: bool) -> Edge<StepEdge> {
    let (color, dotted, _) = rel_style(kind);
    let base = if dotted { EdgeStyle::dotted() } else { EdgeStyle::default() };
    let style = base.with_stroke_style(Style::new().fg(color)).with_label_style(Style::new().fg(color).add_modifier(Modifier::ITALIC));
    let mut e = Edge::new(format!("e{id}"), source, target).with_content(StepEdge::default().with_style(style)).with_animated(animated);
    if opts.labels.labels() && kind != "next" {
        e = e.with_label(kind);
    }
    e
}

fn flow_theme(pal: Palette) -> Theme {
    let mut p = Theme::Dark.palette();
    p.canvas_bg = Color::Reset;
    p.surface = Color::Reset;
    p.muted = theme::DIM;
    p.subtle = theme::FAINT;
    p.accent = pal.accent;
    p.text = theme::TEXT;
    Theme::Custom(p)
}

fn finish(nodes: Vec<Node<StatusNode>>, edges: Vec<Edge<StepEdge>>, select: &str, pal: Palette, opts: GraphOpts) -> Graph {
    // What to centre on: the node that matters and its direct neighbours.
    let mut focus = vec![select.to_string()];
    for e in &edges {
        if e.source == select {
            focus.push(e.target.clone());
        } else if e.target == select {
            focus.push(e.source.clone());
        }
    }
    let links: Vec<(String, String)> = edges.iter().map(|e| (e.source.clone(), e.target.clone())).collect();
    // Nodes can be dragged to tidy the picture; links can never be made, moved or deleted here.
    let nodes = nodes.into_iter().map(|n| n.with_draggable(true).with_connectable(false).with_deletable(false)).collect();
    let mut flow = Graph::with_graph(nodes, edges)
        .expect("mock graph is valid")
        .with_theme(flow_theme(pal))
        .with_edges_reconnectable(false);
    let layout = if opts.horizontal { Sugiyama::horizontal() } else { Sugiyama::vertical() };
    let spacing = if opts.labels.labels() { 4.0 } else { 2.0 };
    flow.apply_layout(layout.with_node_spacing(2.0).with_rank_spacing(spacing).with_margin(1.0));
    spread_components(&mut flow, &links, opts.horizontal);
    flow.select_node(select);
    // Zoom 1, centred on that neighbourhood: fitting everything shrinks nodes past their text.
    flow.request_fit_view_with_options(FitViewOptions::default().with_zoom_range(1.0, 1.0).with_nodes(focus));
    flow
}

fn item_node(item: Option<&Item>, key: &str, root: bool, pal: Palette) -> Node<StatusNode> {
    let (status, title) = item.map_or((Status::Open, "(another workspace)"), |i| (i.status, i.title));
    let content = StatusNode {
        head: key.to_string(),
        head_color: pal.key(key).fg.unwrap_or(theme::TEXT),
        sub: title.to_string(),
        status,
        accent: pal.accent,
        root,
        shape: Shape::Item,
    };
    Node::new(key, (0.0, 0.0), (NODE_W, NODE_H), content)
}

fn req_node(r: &Requirement, root: bool, pal: Palette) -> Node<StatusNode> {
    let content = StatusNode {
        head: r.id.to_string(),
        head_color: theme::status_color(Status::AwaitingApproval),
        sub: r.title.to_string(),
        status: r.status,
        accent: pal.accent,
        root,
        shape: Shape::Requirement,
    };
    Node::new(format!("req:{}", r.id), (0.0, 0.0), (NODE_W, NODE_H), content)
}

/// The item; what blocks it (transitively, above) and what it blocks (below); its other
/// relationships; and the requirements it covers. Returns the graph and the kinds it uses.
pub fn item_graph(items: &[Item], reqs: &[Requirement], root: &Item, pal: Palette, opts: GraphOpts) -> (Graph, Vec<&'static str>) {
    let mut keys: Vec<&str> = vec![root.key];
    let mut links: Vec<(String, String, &'static str)> = Vec::new();
    let mut stack = vec![root.key];
    while let Some(key) = stack.pop() {
        let Some(item) = items.iter().find(|i| i.key == key) else { continue };
        for dep in item.deps {
            links.push((dep.to_string(), key.to_string(), "blocks"));
            if !keys.contains(dep) {
                keys.push(dep);
                stack.push(dep);
            }
        }
    }
    for item in items.iter().filter(|i| i.deps.contains(&root.key)) {
        links.push((root.key.to_string(), item.key.to_string(), "blocks"));
        if !keys.contains(&item.key) {
            keys.push(item.key);
        }
    }
    for (kind, other) in root.rels {
        // "X follows up Y": Y came first, so the arrow runs Y → X.
        match *kind {
            "follows up" => links.push((other.to_string(), root.key.to_string(), "follow-up")),
            k => links.push((root.key.to_string(), other.to_string(), if k == "relates to" { "relates to" } else { "follow-up" })),
        }
        if !keys.contains(other) {
            keys.push(other);
        }
    }
    for item in items.iter().filter(|i| i.rels.iter().any(|(_, t)| *t == root.key)) {
        let kind: &str = item.rels.iter().find(|(_, t)| *t == root.key).map_or("relates to", |(k, _)| k);
        match kind {
            "follows up" => links.push((root.key.to_string(), item.key.to_string(), "follow-up")),
            _ => links.push((item.key.to_string(), root.key.to_string(), "relates to")),
        }
        if !keys.contains(&item.key) {
            keys.push(item.key);
        }
    }
    let mut nodes: Vec<Node<StatusNode>> = keys.iter().map(|k| item_node(items.iter().find(|i| i.key == *k), k, *k == root.key, pal)).collect();
    // The requirements it covers sit on top, with the requirements they refine above them.
    let mut req_ids: Vec<&str> = Vec::new();
    for r in reqs.iter().filter(|r| r.items.contains(&root.key)) {
        links.push((format!("req:{}", r.id), root.key.to_string(), "covered by"));
        let mut cur = Some(r);
        while let Some(r) = cur {
            if req_ids.contains(&r.id) {
                break;
            }
            req_ids.push(r.id);
            nodes.push(req_node(r, false, pal));
            cur = r.parent.and_then(|p| reqs.iter().find(|x| x.id == p && x.ws == r.ws));
            if let Some(parent) = cur {
                links.push((format!("req:{}", parent.id), format!("req:{}", r.id), "refines"));
            }
        }
    }
    let mut kinds: Vec<&'static str> = Vec::new();
    for (_, _, k) in &links {
        if !kinds.contains(k) {
            kinds.push(k);
        }
    }
    let edges = links.iter().enumerate().map(|(i, (s, t, k))| edge(i, s, t, k, opts, false)).collect();
    (finish(nodes, edges, root.key, pal, opts), kinds)
}

/// One step of a run as the graph and Enter see it.
pub struct Step {
    pub id: &'static str,
    pub name: String,
    pub agent: &'static str,
    pub status: Status,
}

/// A run: htui prepares the target, a fast model picks the workflow, then the workflow's steps.
pub fn run_steps(item: &Item, wf: &Workflow, target: &str) -> Vec<Step> {
    // Every run starts with the same two stages, then the workflow's own.
    let mut all: Vec<(&'static str, String, &'static str, usize)> = vec![
        ("prepare", format!("prepare · {target}"), "htui", 0),
        ("pick", "pick workflow".to_string(), "fast model", 1),
    ];
    all.extend(wf.steps.iter().map(|(id, name, agent, stage)| (*id, name.to_string(), *agent, stage + 2)));
    let stages = all.iter().map(|s| s.3).max().unwrap_or(0) + 1;
    // How many stages are finished; the next one is the current stage (none when not started).
    let (done, current) = match item.status {
        Status::Open | Status::Queued => (0, None),
        Status::Blocked => (2, Some(Status::Blocked)),
        Status::InProgress => ((stages * 5 / 8).max(2), Some(Status::InProgress)),
        Status::AwaitingApproval => ((stages * 6 / 8).max(2), Some(Status::AwaitingApproval)),
        Status::Failed => (stages - 1, Some(Status::Failed)),
        Status::Done | Status::Closed => (stages, None),
    };
    all.into_iter()
        .map(|(id, name, agent, stage)| {
            let status = match current {
                _ if stage < done => Status::Done,
                Some(cur) if stage == done => cur,
                _ => Status::Queued,
            };
            Step { id, name, agent, status }
        })
        .collect()
}

pub fn run_graph(item: &Item, wf: &Workflow, target: &str, pal: Palette, opts: GraphOpts) -> Graph {
    let steps = run_steps(item, wf, target);
    let status = |id: &str| steps.iter().find(|s| s.id == id).map_or(Status::Queued, |s| s.status);
    let nodes = steps
        .iter()
        .map(|s| {
            let sub = match s.status {
                Status::Done => format!("{} · 4m", s.agent),
                Status::Queued => "not started".to_string(),
                st => format!("{} · {}", s.agent, st.label()),
            };
            let content = StatusNode { head: s.name.clone(), head_color: theme::TEXT, sub, status: s.status, accent: pal.accent, root: false, shape: Shape::Step };
            Node::new(s.id, (0.0, 0.0), (NODE_W - 4.0, NODE_H), content)
        })
        .collect();
    let first = wf.steps.first().map_or("gate", |s| s.0);
    let mut pairs = vec![("prepare", "pick", "next"), ("pick", first, "next")];
    pairs.extend(wf.links.iter().copied());
    // The links into a step that is running now move.
    let edges = pairs
        .iter()
        .enumerate()
        .map(|(i, (s, t, k))| edge(i, s, t, k, opts, status(t) == Status::InProgress && status(s) == Status::Done))
        .collect();
    let select = steps.iter().find(|s| s.status != Status::Done).or(steps.last()).map_or("prepare", |s| s.id);
    finish(nodes, edges, select, pal, opts)
}

/// The link kinds a workflow's run graph uses, for its legend.
pub fn run_kinds(wf: &Workflow) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    for (_, _, k) in wf.links {
        if *k != "next" && !kinds.contains(k) {
            kinds.push(*k);
        }
    }
    kinds
}

/// Requirements in scope, parent above child, and the items covering each.
pub fn req_graph(items: &[Item], reqs: &[&Requirement], pal: Palette, opts: GraphOpts) -> (Graph, Vec<&'static str>) {
    let mut nodes: Vec<Node<StatusNode>> = reqs.iter().map(|r| req_node(r, false, pal)).collect();
    let mut links: Vec<(String, String, &'static str)> = Vec::new();
    let mut item_keys: Vec<&str> = Vec::new();
    for r in reqs {
        if let Some(parent) = r.parent
            && reqs.iter().any(|x| x.id == parent)
        {
            links.push((format!("req:{parent}"), format!("req:{}", r.id), "refines"));
        }
        for key in r.items {
            links.push((format!("req:{}", r.id), key.to_string(), "covered by"));
            if !item_keys.contains(key) {
                item_keys.push(key);
            }
        }
    }
    nodes.extend(item_keys.iter().map(|k| item_node(items.iter().find(|i| i.key == *k), k, false, pal)));
    let mut kinds: Vec<&'static str> = Vec::new();
    for (_, _, k) in &links {
        if !kinds.contains(k) {
            kinds.push(k);
        }
    }
    let select = reqs.first().map_or(String::new(), |r| format!("req:{}", r.id));
    let edges = links.iter().enumerate().map(|(i, (s, t, k))| edge(i, s, t, k, opts, false)).collect();
    (finish(nodes, edges, &select, pal, opts), kinds)
}

/// rataflow lays each connected component out from the same origin, so separate clusters land on
/// top of each other. Put them side by side (or one under another, flowing left to right).
fn spread_components(flow: &mut Graph, links: &[(String, String)], horizontal: bool) {
    let nodes: Vec<(String, f64, f64, f64, f64)> =
        flow.nodes().map(|n| (n.id.clone(), n.position.x, n.position.y, n.width, n.height)).collect();
    // Union-find over the links.
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    fn root(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let index = |id: &str| nodes.iter().position(|n| n.0 == id);
    for (a, b) in links {
        if let (Some(a), Some(b)) = (index(a), index(b)) {
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            parent[ra] = rb;
        }
    }
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    for i in 0..nodes.len() {
        let r = root(&mut parent, i);
        match seen.iter().position(|s| *s == r) {
            Some(c) => comps[c].push(i),
            None => {
                seen.push(r);
                comps.push(vec![i]);
            }
        }
    }
    if comps.len() < 2 {
        return;
    }
    let mut cursor = 1.0;
    let mut moved = Vec::new();
    for comp in comps {
        // Along the axis siblings spread on: x top to bottom, y left to right.
        let start = comp.iter().map(|i| if horizontal { nodes[*i].2 } else { nodes[*i].1 }).fold(f64::MAX, f64::min);
        let end = comp
            .iter()
            .map(|i| if horizontal { nodes[*i].2 + nodes[*i].4 } else { nodes[*i].1 + nodes[*i].3 })
            .fold(f64::MIN, f64::max);
        for i in comp {
            let (id, x, y, ..) = &nodes[i];
            let pos = if horizontal { (*x, y - start + cursor) } else { (x - start + cursor, *y) };
            moved.push((id.clone(), pos));
        }
        cursor += end - start + 4.0;
    }
    flow.set_node_positions(moved);
}

/// `f`: everything in view, but never larger than life.
pub fn fit_all(flow: &mut Graph) {
    flow.request_fit_view_with_options(FitViewOptions::default().with_max_zoom(1.0).with_padding(1.0));
}
