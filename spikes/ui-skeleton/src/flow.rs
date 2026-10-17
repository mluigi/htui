//! rataflow graphs: an item's neighbourhood (Graph), a run's steps (Runs), and requirements.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use rataflow::{Edge, EdgeStyle, FitViewOptions, Flow, Handle, HandlePosition, Node, NodeContent, NodeRenderContext, StepEdge, Sugiyama, Theme};

use crate::data::{self, Item, Requirement, Status, Workflow};
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
        "changes requested" => (theme::status_color(Status::Failed), false, "loop: review sends it back"),
        "approved" => (theme::status_color(Status::Done), false, "review passes it on"),
        "fails" => (theme::status_color(Status::Failed), true, "loop: a failed check sends it back"),
        "retries" => (theme::status_color(Status::Blocked), true, "loop: the same step again, up to a cap"),
        "asks you" => (theme::status_color(Status::AwaitingApproval), false, "waits for your go-ahead"),
        "if" => (theme::kind_color("FEAT"), true, "only when a condition holds"),
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
    // Rank spacing 2, never 1: with a one-row gap the arrow vanishes whenever the view's pan lands
    // on a whole row (rataflow 0.1 rounds the short edge away).
    let spacing = if opts.labels.labels() { 4.0 } else { 2.0 };
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
    pub id: String,
    pub name: String,
    pub agent: String,
    pub status: Status,
}

/// How a run carries a template out: the steps in order, a loop's body run twice (the review asked
/// for changes once), a fan-out as one node per part. Returns (id, name, agent, stage) and links.
pub fn actuate(wf: &Workflow) -> (Vec<(String, String, String, usize)>, Vec<(String, String, &'static str)>) {
    let pos = |id: &str| wf.steps.iter().position(|s| s.id == id);
    let fwd = |a: usize, b: usize| {
        wf.links.iter().find(|l| !data::is_loop(l.kind) && l.from == wf.steps[a].id && l.to == wf.steps[b].id).map_or("next", |l| l.kind)
    };
    // The loop the mock run goes round once: the first "changes requested" back to an earlier step.
    let main_loop = wf.links.iter().filter(|l| l.kind == "changes requested").find_map(|l| {
        let (src, dst) = (pos(&l.from)?, pos(&l.to)?);
        (dst <= src).then_some((src, dst))
    });
    let mut groups: Vec<(Vec<(String, String, String)>, &'static str)> = Vec::new();
    let mut push = |j: usize, round: Option<usize>, kind: &'static str| {
        let s = &wf.steps[j];
        let (sid, sname) = match round {
            Some(r) => (format!("{}-{r}", s.id), format!("{} · round {r}", s.name)),
            None => (s.id.clone(), s.name.clone()),
        };
        let nodes = if s.parts.is_empty() {
            vec![(sid, sname, s.agent.clone())]
        } else {
            s.parts.iter().enumerate().map(|(k, (part, agent))| (format!("{sid}-p{k}"), format!("{sname} · {part}"), agent.clone())).collect()
        };
        groups.push((nodes, kind));
    };
    let mut prev: Option<usize> = None;
    let mut i = 0;
    while i < wf.steps.len() {
        if let Some((src, dst)) = main_loop.filter(|(_, dst)| *dst == i) {
            for round in 1..=2 {
                for j in dst..=src {
                    let kind = if round == 2 && j == dst { "changes requested" } else { prev.map_or("next", |p| fwd(p, j)) };
                    push(j, Some(round), kind);
                    prev = Some(j);
                }
            }
            i = src + 1;
        } else {
            push(i, None, prev.map_or("next", |p| fwd(p, i)));
            prev = Some(i);
            i += 1;
        }
    }
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    for (g, (group, kind)) in groups.iter().enumerate() {
        nodes.extend(group.iter().map(|(id, name, agent)| (id.clone(), name.clone(), agent.clone(), g)));
        if g == 0 {
            continue;
        }
        let before = &groups[g - 1].0;
        for (a, (from, ..)) in before.iter().enumerate() {
            for (b, (to, ..)) in group.iter().enumerate() {
                let kind = match (before.len(), group.len()) {
                    (1, n) if n > 1 => "fans out",
                    (n, 1) if n > 1 => "merges",
                    (n, m) if n > 1 && m > 1 && a != b => continue,
                    _ => *kind,
                };
                links.push((from.clone(), to.clone(), kind));
            }
        }
    }
    (nodes, links)
}

/// A run: htui prepares the target, a fast model picks the workflow, the workflow's steps, and on a
/// sandboxed or remote target a last step that waits for you to collect the branch.
pub fn run_steps(item: &Item, wf: &Workflow, target: &data::Target) -> Vec<Step> {
    let (body, _) = actuate(wf);
    let mut all: Vec<(String, String, String, usize)> =
        vec![("prepare".into(), format!("prepare · {}", target.name), "htui".into(), 0), ("pick".into(), "pick workflow".into(), "fast model".into(), 1)];
    all.extend(body.into_iter().map(|(id, name, agent, stage)| (id, name, agent, stage + 2)));
    if target.kind != "local" {
        let last = all.iter().map(|s| s.3).max().unwrap_or(0) + 1;
        all.push(("collect".into(), "collect the branch".into(), "you".into(), last));
    }
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

pub fn run_graph(item: &Item, wf: &Workflow, target: &data::Target, pal: Palette, opts: GraphOpts) -> Graph {
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
            Node::new(s.id.as_str(), (0.0, 0.0), (NODE_W - 4.0, NODE_H), content)
        })
        .collect();
    let (body, mut pairs) = actuate(wf);
    let firsts: Vec<String> = body.iter().filter(|s| s.3 == 0).map(|s| s.0.clone()).collect();
    pairs.insert(0, ("prepare".into(), "pick".into(), "next"));
    for (k, f) in firsts.iter().enumerate() {
        pairs.insert(1 + k, ("pick".into(), f.clone(), if firsts.len() > 1 { "fans out" } else { "next" }));
    }
    if let Some(collect) = steps.iter().find(|s| s.id == "collect") {
        let last_stage = body.iter().map(|s| s.3).max().unwrap_or(0);
        for s in body.iter().filter(|s| s.3 == last_stage) {
            pairs.push((s.0.clone(), collect.id.clone(), "asks you"));
        }
    }
    // The links into a step that is running now move.
    let edges = pairs
        .iter()
        .enumerate()
        .map(|(i, (s, t, k))| edge(i, s, t, k, opts, status(t) == Status::InProgress && status(s) == Status::Done))
        .collect();
    let select = steps.iter().find(|s| s.status != Status::Done).or(steps.last()).map_or("prepare".to_string(), |s| s.id.clone());
    finish(nodes, edges, &select, pal, opts)
}

/// The link kinds a run of this workflow shows, for its legend.
pub fn run_kinds(wf: &Workflow) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    for (_, _, k) in actuate(wf).1 {
        if k != "next" && !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    kinds
}

/// The template itself: one node per step (a fan-out shows its parts), every link, loops pointing
/// back up. Laid out without the loops, which are added afterwards so they never bend the ranks.
pub fn workflow_graph(wf: &Workflow, selected: Option<&str>, pal: Palette, opts: GraphOpts) -> (Graph, Vec<&'static str>) {
    let nodes: Vec<Node<StatusNode>> = wf
        .steps
        .iter()
        .map(|s| {
            let (head, sub) = if s.parts.is_empty() {
                let who = match (s.persona.as_str(), s.agent.as_str()) {
                    ("", a) => a.to_string(),
                    (p, "persona's default") => p.to_string(),
                    (p, a) => format!("{p} · {a}"),
                };
                (s.name.clone(), who)
            } else {
                (format!("{} ×{}", s.name, s.parts.len()), s.parts.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", "))
            };
            let content = StatusNode { head, head_color: theme::TEXT, sub, status: Status::Open, accent: pal.accent, root: false, shape: Shape::Step };
            // Loops leave and enter by the sides, so they never hide under the forward chain:
            // review loops on the right, failure loops on the left.
            let side = |pos: HandlePosition, id: &str, source: bool, offset: f64| {
                let h = if source { Handle::source(pos) } else { Handle::target(pos) };
                h.with_id(id).with_offset(offset).with_hidden(true)
            };
            Node::new(s.id.as_str(), (0.0, 0.0), (NODE_W, NODE_H), content).with_handles(vec![
                Handle::source(HandlePosition::Bottom).with_id("bottom"),
                Handle::target(HandlePosition::Top).with_id("top"),
                side(HandlePosition::Right, "r-out", true, 0.75),
                side(HandlePosition::Right, "r-in", false, 0.25),
                side(HandlePosition::Left, "l-out", true, 0.75),
                side(HandlePosition::Left, "l-in", false, 0.25),
            ])
        })
        .collect();
    let mut forward = Vec::new();
    let mut loops = Vec::new();
    for (i, l) in wf.links.iter().enumerate() {
        let e = edge(i, &l.from, &l.to, l.kind, opts, false);
        if data::is_loop(l.kind) {
            let (out, inn) = if l.kind == "changes requested" { ("r-out", "r-in") } else { ("l-out", "l-in") };
            loops.push(e.with_source_handle(Some(out.into())).with_target_handle(Some(inn.into())));
        } else {
            forward.push(e.with_source_handle(Some("bottom".into())).with_target_handle(Some("top".into())));
        }
    }
    let mut kinds: Vec<&'static str> = Vec::new();
    for l in &wf.links {
        if !kinds.contains(&l.kind) {
            kinds.push(l.kind);
        }
    }
    let first = selected.map(str::to_string).or_else(|| wf.steps.first().map(|s| s.id.clone())).unwrap_or_default();
    let mut flow = finish(nodes, forward, &first, pal, opts);
    for e in loops {
        let _ = flow.add_edge(e);
    }
    // In the editor a link can be drawn by dragging from a step's ● to another step. Dropping
    // anywhere on the step counts: rataflow snaps to the nearest handle within this radius (its
    // default needs the drop right on a handle).
    for s in &wf.steps {
        flow.set_node_connectable(&s.id, true);
    }
    flow.connection_radius = NODE_W / 2.0 + 2.0;

    (flow, kinds)
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
