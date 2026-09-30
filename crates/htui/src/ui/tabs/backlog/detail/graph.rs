//! The Graph sub-tab: the selected item's link neighbourhood as a navigable tree (MOD-14).
//!
//! The Backlog reads the traversal once per selection at [`GraphTab::MAX_HOPS`], and the view
//! depth (`+` / `-`) filters it here, so a depth change costs no read (plan D2). The rows form a
//! `cargo tree`-style tree: one row per edge, each item expanded once under the first parent that
//! reaches it, and every other edge a `(*)` leaf (D3), its arrow read from the row it hangs under
//! (D4). `Enter` re-roots by emitting `Action::Reveal` for the item under the cursor (D6), so the
//! sub-tab never names the list it re-roots.

use core::cmp::Ordering;

use htui_core::model::{ItemId, LinkEdge, LinkGraph, LinkKind, LinkNode, ProjectId, ProjectRef};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, PAGE, message};
use crate::ui::tabs::backlog::list;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// An edge that leaves the parent row's item (D4).
const OUT: &str = "\u{2192}";

/// An edge that arrives at the parent row's item (D4).
const IN: &str = "\u{2190}";

/// Marks the row under the cursor (the `runs.rs` / `requirements.rs` glyph).
const CURSOR: &str = "\u{25b8}";

/// Default view depth (D2).
const DEFAULT_DEPTH: u8 = 2;

/// Columns one tree level indents by.
const INDENT: usize = 2;

/// Width the kind column is padded to: `blocked_by` / `supersedes`, the old table's `Length(10)`.
const KIND_WIDTH: usize = 10;

/// Narrowest title worth drawing: below it a clipped title is an ellipsis and a letter or two.
const TITLE_MIN: usize = 8;

/// Marks a non-tree edge (D3).
const REPEAT: &str = " (*)";

/// `StoreRequest::Links`' name: what a refused `Links` read is answered `Failed` under.
const LINKS: &str = "links";

/// Which way an edge points relative to the row it hangs under (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrow {
    /// `parent --kind--> row`.
    Out,
    /// `row --kind--> parent`.
    In,
}

impl Arrow {
    /// The glyph drawn for it.
    const fn glyph(self) -> &'static str {
        match self {
            Self::Out => OUT,
            Self::In => IN,
        }
    }
}

/// How a non-root row is reached from the row it hangs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Link {
    /// `→` / `←` relative to the parent row's item.
    arrow: Arrow,
    /// The edge's kind.
    kind: LinkKind,
    /// Not a tree edge: a `(*)` leaf whose item is expanded under its own tree edge.
    repeat: bool,
}

/// One line of the tree, borrowed from the `LinkGraph` it was built from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphRow<'a> {
    /// `0` for the root, one more than the row it hangs under otherwise.
    level: u8,
    /// `None` on the root row.
    link: Option<Link>,
    /// The item this row names.
    node: &'a LinkNode,
}

impl GraphRow<'_> {
    /// Whether this row is a `(*)` leaf rather than its item's own row.
    fn is_repeat(&self) -> bool {
        self.link.is_some_and(|link| link.repeat)
    }
}

/// The selected item's link neighbourhood as a navigable tree (MOD-14).
#[derive(Debug)]
pub struct GraphTab {
    /// The `Links` reply for the selected item, fetched at `MAX_HOPS`.
    graph: Option<LinkGraph>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// Why the `Links` read for the selected item was refused, until another answer arrives.
    failed: Option<String>,
    /// Index into the visible rows.
    cursor: usize,
    /// View depth, `1..=MAX_HOPS`. A preference: survives `on_item_change` (D2).
    depth: u8,
}

impl GraphTab {
    /// Identity of the Graph sub-tab.
    pub const ID: DetailId = DetailId("graph");

    /// How far the Backlog's `Links` read reaches, and the deepest view `+` reaches (D2).
    pub const MAX_HOPS: u8 = 3;

    /// A sub-tab with no graph yet, at the default view depth.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            graph: None,
            item: None,
            failed: None,
            cursor: 0,
            depth: DEFAULT_DEPTH,
        }
    }

    /// The rows at the current view depth; empty while no graph is loaded.
    fn visible(&self) -> Vec<GraphRow<'_>> {
        self.graph
            .as_ref()
            .map(|graph| rows(graph, self.depth))
            .unwrap_or_default()
    }

    /// Moves the cursor `delta` rows, clamped to the rows (0 when there are none).
    fn move_cursor(&mut self, delta: isize) {
        let last = self.visible().len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    /// Changes the view depth, keeping the cursor on its item's own row when it is still shown.
    ///
    /// The row index moves with the depth (preorder changes), so the cursor follows the item, not
    /// the index; an item the new depth hides sends it back to the root. A depth the clamp left
    /// where it was changes nothing, so the cursor stays even on a `(*)` row.
    fn set_depth(&mut self, depth: u8) {
        if depth == self.depth {
            return;
        }
        let item = self.visible().get(self.cursor).map(|row| row.node.item_id);
        self.depth = depth;
        self.cursor = item
            .and_then(|id| {
                self.visible()
                    .iter()
                    .position(|row| !row.is_repeat() && row.node.item_id == id)
            })
            .unwrap_or(0);
    }

    /// `Enter`: re-root on the item under the cursor, or say why not (D6, D7). Nothing on the
    /// root row, or with no graph loaded.
    fn re_root(&self, ctx: &Ctx<'_>) {
        let rows = self.visible();
        let Some(row) = rows.get(self.cursor) else {
            return;
        };
        if row.link.is_none() {
            return;
        }
        let node = row.node;
        if in_workspace(node, ctx.projects) {
            ctx.emit(Action::Reveal(RevealTarget::Item {
                id: node.item_id,
                key: node.key.clone(),
            }));
        } else {
            // D7: revealing it would only re-read the list to answer `not_in_this_backlog`.
            ctx.emit(Action::Error(format!(
                "{}:{} is outside this workspace",
                node.project_slug, node.key
            )));
        }
    }
}

impl Default for GraphTab {
    fn default() -> Self {
        Self::new()
    }
}

/// `(depth, key bytes, project slug bytes, item id)`: the order every visible node is ranked in.
///
/// Bytes, not the backend's order: `MemStore` answers in discovery order and Postgres in its
/// collation (`UpstreamEntry::sort_canonical`'s reasoning).
fn node_order(a: &LinkNode, b: &LinkNode) -> Ordering {
    a.depth
        .cmp(&b.depth)
        .then_with(|| a.key.as_bytes().cmp(b.key.as_bytes()))
        .then_with(|| a.project_slug.as_bytes().cmp(b.project_slug.as_bytes()))
        .then_with(|| a.item_id.cmp(&b.item_id))
}

/// The tree of `graph` cut at view depth `depth` (D2, D3, D4), in preorder, the root first.
///
/// Nodes are ranked by [`node_order`] and edges by their endpoints' ranks, then their kind. A
/// node's tree parent is its first-ranked neighbour one level up, joined by the first edge between
/// the two; every other edge hangs, as a `(*)` leaf, under its shallower endpoint (the first-ranked
/// one on a tie). A node with no neighbour one level up (a malformed reply) is dropped with its
/// edges. The walk terminates because a tree parent is strictly shallower than its child.
fn rows(graph: &LinkGraph, depth: u8) -> Vec<GraphRow<'_>> {
    let Some(root) = graph.node(graph.root) else {
        return Vec::new();
    };
    let mut nodes: Vec<&LinkNode> = graph
        .nodes
        .iter()
        .filter(|node| node.item_id == root.item_id || (node.depth > 0 && node.depth <= depth))
        .collect();
    nodes.sort_by(|a, b| {
        // The root ranks first whatever depth the reply gave it.
        (a.item_id != root.item_id)
            .cmp(&(b.item_id != root.item_id))
            .then_with(|| node_order(a, b))
    });
    nodes.dedup_by_key(|node| node.item_id);
    let rank = |id: ItemId| nodes.iter().position(|node| node.item_id == id);

    let mut edges: Vec<(usize, usize, &LinkEdge)> = graph
        .edges
        .iter()
        .filter_map(|edge| Some((rank(edge.from_item_id)?, rank(edge.to_item_id)?, edge)))
        .collect();
    edges.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| a.2.kind.as_str().cmp(b.2.kind.as_str()))
    });

    // Tree edges, by child rank: `(parent rank, edge index)`. Ranks ascend with depth, so every
    // candidate parent has been decided (kept or dropped) before its children are looked at.
    let mut parent: Vec<Option<(usize, usize)>> = vec![None; nodes.len()];
    let mut kept = vec![false; nodes.len()];
    kept[0] = true;
    for child in 1..nodes.len() {
        let wanted = nodes[child].depth.checked_sub(1);
        parent[child] = edges
            .iter()
            .enumerate()
            .filter_map(|(at, &(from, to, _))| {
                let other = match (from == child, to == child) {
                    (true, false) => to,
                    (false, true) => from,
                    _ => return None,
                };
                (kept[other] && Some(nodes[other].depth) == wanted).then_some((other, at))
            })
            .min();
        kept[child] = parent[child].is_some();
    }

    // What hangs under each node: its tree children and the non-tree edges it owns.
    let mut under: Vec<Vec<(usize, Link)>> = vec![Vec::new(); nodes.len()];
    for (at, &(from, to, edge)) in edges.iter().enumerate() {
        if !kept[from] || !kept[to] {
            continue;
        }
        let tree = |child: usize| parent[child].is_some_and(|(_, tree_edge)| tree_edge == at);
        let (owner, other, repeat) = if from != to && tree(to) {
            (from, to, false)
        } else if from != to && tree(from) {
            (to, from, false)
        } else {
            (from.min(to), from.max(to), true)
        };
        // A self-loop has `owner == other` and reads outward.
        let arrow = if nodes[owner].item_id == edge.from_item_id {
            Arrow::Out
        } else {
            Arrow::In
        };
        under[owner].push((
            other,
            Link {
                arrow,
                kind: edge.kind,
                repeat,
            },
        ));
    }
    for children in &mut under {
        children.sort_by(|(a, a_link), (b, b_link)| {
            let (a, b) = (nodes[*a], nodes[*b]);
            a.key
                .as_bytes()
                .cmp(b.key.as_bytes())
                .then_with(|| a.project_slug.as_bytes().cmp(b.project_slug.as_bytes()))
                .then_with(|| a.item_id.cmp(&b.item_id))
                .then_with(|| a_link.kind.as_str().cmp(b_link.kind.as_str()))
                .then_with(|| (a_link.arrow == Arrow::In).cmp(&(b_link.arrow == Arrow::In)))
        });
    }

    let mut out = vec![GraphRow {
        level: 0,
        link: None,
        node: nodes[0],
    }];
    walk(0, 1, &nodes, &under, &mut out);
    out
}

/// Preorder: the rows under `at`, each at `level`, a tree child followed by its own subtree.
fn walk<'a>(
    at: usize,
    level: u8,
    nodes: &[&'a LinkNode],
    under: &[Vec<(usize, Link)>],
    out: &mut Vec<GraphRow<'a>>,
) {
    for &(other, link) in &under[at] {
        out.push(GraphRow {
            level,
            link: Some(link),
            node: nodes[other],
        });
        if !link.repeat {
            walk(other, level.saturating_add(1), nodes, under, out);
        }
    }
}

/// Whether `node`'s project is one of the workspace's (D7).
fn in_workspace(node: &LinkNode, projects: &[ProjectRef]) -> bool {
    projects
        .iter()
        .any(|project| project.project_id == node.project_id)
}

/// One row as a line at most `width` columns wide (§1.5 of the blueprint).
fn line(
    row: &GraphRow<'_>,
    on_cursor: bool,
    root_project: ProjectId,
    in_workspace: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let node = row.node;
    // D7: a node outside the workspace is drawn, but dim, so it does not read as reachable.
    let (label_style, status_style, title_style) = if in_workspace {
        (theme.accent, theme.status_style(node.status), theme.base)
    } else {
        (theme.dim, theme.dim, theme.dim)
    };
    let status = node.status.as_str();
    let mut spans = vec![
        Span::styled(if on_cursor { CURSOR } else { " " }, theme.accent),
        Span::raw(" "),
    ];

    let Some(link) = row.link else {
        spans.push(Span::styled(node.key.clone(), label_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(status, status_style));
        push_title(&mut spans, &node.title, width, title_style);
        return Line::from(spans);
    };

    let indent = (INDENT * usize::from(row.level.saturating_sub(1)))
        .min(INDENT * usize::from(GraphTab::MAX_HOPS));
    let kind = link.kind.as_str();
    let key = node.key.as_str();
    let slug = (node.project_id != root_project).then_some(node.project_slug.as_str());
    let label = slug.map_or_else(|| key.to_owned(), |slug| format!("{slug}:{key}"));
    let repeat = if link.repeat { REPEAT } else { "" };
    // Everything but the kind and the label: cursor, space, indent, arrow, three separators,
    // the status and the repeat mark.
    let fixed = 2 + indent + 1 + 3 + status.len() + repeat.chars().count();

    let padded = format!("{kind:<KIND_WIDTH$}");
    let (kind, label) = if fixed + padded.len() + label.chars().count() <= width {
        (padded, label)
    } else if fixed + kind.len() + label.chars().count() <= width {
        (kind.to_owned(), label)
    } else {
        // The slug gives way before the key does: the key is what names the item.
        let label_room = width.saturating_sub(fixed + kind.len());
        let label = match slug {
            Some(slug) => match label_room.checked_sub(key.chars().count() + 1) {
                Some(slug_room) if slug_room >= 2 => {
                    format!("{}:{key}", clip(slug, slug_room))
                }
                // Too little for a readable slug: the key alone, clipped if it must be.
                _ => clip(key, label_room),
            },
            None => clip(key, label_room),
        };
        (kind.to_owned(), label)
    };

    spans.push(Span::raw(" ".repeat(indent)));
    spans.push(Span::styled(link.arrow.glyph(), theme.dim));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(kind, theme.base));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(label, label_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(status, status_style));
    if link.repeat {
        // The item's own row carries its title.
        spans.push(Span::styled(REPEAT, theme.dim));
    } else {
        push_title(&mut spans, &node.title, width, title_style);
    }
    Line::from(spans)
}

/// `text` in at most `room` columns, `…`-clipped: [`list::clip`], except that no room is no text
/// (`list::clip` answers a lone `…` there, one column over).
fn clip(text: &str, room: usize) -> String {
    if room == 0 {
        String::new()
    } else {
        list::clip(text, room)
    }
}

/// Appends `' ' + title`, clipped to what is left of `width`, when at least [`TITLE_MIN`]
/// columns are left for it: titles are the first thing a narrow pane drops.
fn push_title(spans: &mut Vec<Span<'static>>, title: &str, width: usize, style: Style) {
    let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    let room = width.saturating_sub(used + 1);
    if room >= TITLE_MIN {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(list::clip(title, room), style));
    }
}

impl DetailTab for GraphTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Graph"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.graph = None;
        self.failed = None;
        self.cursor = 0;
    }

    /// `J` / `K` and `PageDown` / `PageUp` move the cursor, `+` / `-` change the view depth,
    /// `Enter` re-roots (D5, D6).
    ///
    /// Matched on the code alone: a terminal sends `J`, `K` and `+` with `SHIFT`. Unlike the Runs
    /// pane, `Enter` is consumed even when there is nothing to re-root: here it means "re-root",
    /// and the Backlog's `replay step` miss would be the wrong answer from this pane.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        match key.code {
            KeyCode::Char('J') => self.move_cursor(1),
            KeyCode::Char('K') => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(PAGE),
            KeyCode::PageUp => self.move_cursor(-PAGE),
            KeyCode::Char('+') => self.set_depth(self.depth.saturating_add(1).min(Self::MAX_HOPS)),
            KeyCode::Char('-') => self.set_depth(self.depth.saturating_sub(1).max(1)),
            KeyCode::Enter => self.re_root(ctx),
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// The `Links` reply for the selected item, or its refusal (L8). One for another root is
    /// dropped: the shell's staleness index already does that, this is the cheap guard on top.
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Links(graph) if Some(graph.root) == self.item => {
                self.graph = Some(graph.clone());
                self.failed = None;
                self.cursor = 0;
            }
            // The refusal is on the status line already (`app/update.rs`); the pane says why it
            // is empty rather than claiming the item has no links.
            StoreReply::Failed { request, message } if *request == LINKS => {
                self.graph = None;
                self.failed = Some(message.clone());
                self.cursor = 0;
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if self.graph.is_none() {
            let text = match self.failed.as_deref() {
                Some(why) => format!("Links unavailable: {why}"),
                None => "Loading links\u{2026}".to_owned(),
            };
            message(frame, area, &text, ctx.theme);
            return;
        }
        let rows = self.visible();
        let Some(root) = rows.first().filter(|_| rows.len() > 1) else {
            message(frame, area, "No links for this item.", ctx.theme);
            return;
        };

        let [tree, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let cursor = self.cursor.min(rows.len() - 1);
        let width = usize::from(tree.width);
        let mut lines: Vec<Line<'static>> = rows
            .iter()
            .enumerate()
            .map(|(at, row)| {
                line(
                    row,
                    at == cursor,
                    root.node.project_id,
                    in_workspace(row.node, ctx.projects),
                    width,
                    ctx.theme,
                )
            })
            .collect();
        let offset = list::window(cursor, lines.len(), usize::from(tree.height));
        // No `Wrap`: a wrapped row would break the row-to-cursor mapping.
        frame.render_widget(
            Paragraph::new(lines.split_off(offset.min(lines.len()))),
            tree,
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!(
                    "depth {}/{} \u{b7} +/- depth \u{b7} Enter re-root",
                    self.depth,
                    Self::MAX_HOPS
                ),
                ctx.theme.dim,
            )),
            footer,
        );
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::{Scope, Status};
    use htui_core::store::{MemStore, ReadStore as _};
    use uuid::Uuid;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crate::ui::tabs::BacklogTab;

    /// The detail pane's inner width at `DEFAULT_SIZE`, pinned by the Backlog tab's
    /// `the_detail_strip_fits_the_detail_pane`.
    const PANE: usize = 43;

    /// Everything a `Ctx` borrows, kept alive for the length of a test (the Runs pane's `Shell`).
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Shell {
        /// Scoped to the demo's `platform` workspace: htui and agy, not vulkan-tutorials.
        async fn platform() -> Self {
            let store = MemStore::demo();
            let workspace = store
                .workspaces()
                .await
                .expect("the memory store never fails")
                .into_iter()
                .find(|workspace| workspace.slug == "platform")
                .expect("the demo holds `platform`");
            let scope = Scope::from_workspace(&workspace);
            let projects = store.projects(&scope).await.expect("the projects");
            Self {
                scope,
                projects,
                top_bar: TopBarState::default(),
                keymap: Keymap::new(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keymap,
                &self.theme,
                Origin::Tab(BacklogTab::ID),
                &self.emit,
            )
        }
    }

    /// The demo's traversal from `root`, at the depth the Backlog reads it.
    async fn demo(root: ItemId) -> LinkGraph {
        MemStore::demo()
            .links(root, GraphTab::MAX_HOPS)
            .await
            .expect("the memory store never fails")
    }

    /// A tree as text: `{indent}{arrow} {kind} {slug:}KEY{ (*)}`, the root as its bare key.
    fn shape(rows: &[GraphRow<'_>]) -> Vec<String> {
        let root = rows.first().map(|row| row.node.project_id);
        rows.iter()
            .map(|row| {
                let Some(link) = row.link else {
                    return row.node.key.clone();
                };
                let slug = if Some(row.node.project_id) == root {
                    String::new()
                } else {
                    format!("{}:", row.node.project_slug)
                };
                format!(
                    "{}{} {} {slug}{}{}",
                    " ".repeat(2 * usize::from(row.level - 1)),
                    link.arrow.glyph(),
                    link.kind.as_str(),
                    row.node.key,
                    if link.repeat { REPEAT } else { "" },
                )
            })
            .collect()
    }

    /// A key event with no modifier, as the harness sends it.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// A pane showing `graph` for its root.
    fn pane(graph: LinkGraph) -> GraphTab {
        let mut tab = GraphTab::new();
        tab.on_item_change(Some(graph.root));
        tab.graph = Some(graph);
        tab
    }

    /// The line's text, spans joined.
    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn node(n: u128, key: &str, depth: u8) -> LinkNode {
        LinkNode {
            item_id: ItemId::from_uuid(Uuid::from_u128(n)),
            project_id: ProjectId::from_uuid(Uuid::from_u128(100)),
            project_slug: "p".to_owned(),
            key: key.to_owned(),
            title: format!("{key} title"),
            status: Status::Open,
            depth,
        }
    }

    fn edge(from: &LinkNode, kind: LinkKind, to: &LinkNode) -> LinkEdge {
        LinkEdge {
            from_item_id: from.item_id,
            to_item_id: to.item_id,
            kind,
        }
    }

    #[tokio::test]
    async fn feat_1_at_depth_2_is_a_tree_of_every_live_edge_once() {
        let graph = demo(ids::HTUI_FEAT_1).await;
        let rows = rows(&graph, 2);
        assert_eq!(
            shape(&rows),
            [
                "FEAT-1",
                "→ origin ANA-1",
                "  ← blocked_by agy:ANA-1",
                "  ← origin FEAT-3 (*)",
                "  ← blocked_by TOOL-1",
                "← blocked_by FEAT-2",
                "  ← relates agy:FEAT-1",
                "← relates FEAT-3",
            ]
        );
        assert_eq!(rows.len(), 1 + 7);
    }

    #[tokio::test]
    async fn the_triangle_yields_one_repeat_row_and_terminates() {
        let graph = demo(ids::HTUI_FEAT_1).await;
        let rows = rows(&graph, 3);
        let feat_3_repeats = rows
            .iter()
            .filter(|row| row.is_repeat() && row.node.item_id == ids::HTUI_FEAT_3)
            .count();
        assert_eq!(feat_3_repeats, 1, "{:#?}", shape(&rows));
        for node in &graph.nodes {
            let own = rows
                .iter()
                .filter(|row| !row.is_repeat() && row.node.item_id == node.item_id)
                .count();
            assert_eq!(own, 1, "{} is expanded once: {:#?}", node.key, shape(&rows));
        }
        assert_eq!(rows.len(), 1 + graph.edges.len());
        assert_eq!(graph.edges.len(), 9);
    }

    #[tokio::test]
    async fn a_tombstoned_edge_is_not_drawn() {
        let graph = demo(ids::HTUI_FEAT_1).await;
        for depth in 1..=GraphTab::MAX_HOPS {
            let rows = rows(&graph, depth);
            assert!(
                !rows
                    .iter()
                    .any(|row| row.level == 1 && row.node.item_id == ids::HTUI_TOOL_1),
                "`TOOL-1 relates FEAT-1` is tombstoned: {:#?}",
                shape(&rows)
            );
            let own: Vec<String> = shape(&rows)
                .into_iter()
                .zip(&rows)
                .filter(|(_, row)| !row.is_repeat() && row.node.item_id == ids::HTUI_TOOL_1)
                .map(|(line, _)| line)
                .collect();
            if depth >= 2 {
                assert_eq!(own, ["  ← blocked_by TOOL-1"], "under ANA-1");
            } else {
                assert!(own.is_empty(), "TOOL-1 is two hops away");
            }
        }
    }

    #[tokio::test]
    async fn depth_1_keeps_the_one_hop_arrows() {
        let graph = demo(ids::HTUI_FEAT_1).await;
        let rows = rows(&graph, 1);
        let level_1: Vec<(Arrow, LinkKind, &str)> = rows
            .iter()
            .filter(|row| row.level == 1)
            .map(|row| {
                let link = row.link.expect("a level-1 row has a link");
                (link.arrow, link.kind, row.node.key.as_str())
            })
            .collect();
        assert_eq!(
            level_1,
            [
                (Arrow::Out, LinkKind::Origin, "ANA-1"),
                (Arrow::In, LinkKind::BlockedBy, "FEAT-2"),
                (Arrow::In, LinkKind::Relates, "FEAT-3"),
            ]
        );
        // Plan deviation D-2: both ends of `FEAT-3 origin ANA-1` are one hop out, so D2 shows it.
        assert_eq!(
            shape(&rows),
            [
                "FEAT-1",
                "→ origin ANA-1",
                "  ← origin FEAT-3 (*)",
                "← blocked_by FEAT-2",
                "← relates FEAT-3",
            ]
        );
    }

    #[tokio::test]
    async fn backend_order_does_not_change_the_tree() {
        for root in [ids::HTUI_FEAT_1, ids::AGY_FIX_1, ids::HTUI_ANA_1] {
            let graph = demo(root).await;
            let mut reversed = graph.clone();
            reversed.nodes.reverse();
            reversed.edges.reverse();
            for depth in 1..=GraphTab::MAX_HOPS {
                assert_eq!(
                    shape(&rows(&graph, depth)),
                    shape(&rows(&reversed, depth)),
                    "depth {depth}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_cross_project_parent_tie_is_broken_by_key() {
        let graph = demo(ids::AGY_FIX_1).await;
        assert_eq!(
            shape(&rows(&graph, 2)),
            [
                "FIX-1",
                "→ blocked_by ANA-1",
                "  → blocked_by htui:ANA-1",
                "→ origin vulkan-tutorials:FEAT-1",
                "→ origin htui:TOOL-1",
                "  → blocked_by htui:ANA-1 (*)",
            ]
        );
    }

    #[tokio::test]
    async fn the_other_roots_the_integration_tests_use() {
        assert_eq!(
            shape(&rows(&demo(ids::HTUI_ANA_1).await, 2)),
            [
                "ANA-1",
                "← blocked_by agy:ANA-1",
                "  ← blocked_by agy:FIX-1",
                "← origin FEAT-1",
                "  ← blocked_by FEAT-2",
                "  ← relates FEAT-3 (*)",
                "← origin FEAT-3",
                "← blocked_by TOOL-1",
                "  ← origin agy:FIX-1 (*)",
            ]
        );
        assert_eq!(
            shape(&rows(&demo(ids::HTUI_FEAT_2).await, 2)),
            [
                "FEAT-2",
                "← relates agy:FEAT-1",
                "→ blocked_by FEAT-1",
                "  → origin ANA-1",
                "    ← origin FEAT-3 (*)",
                "  ← relates FEAT-3",
            ]
        );
        assert_eq!(
            shape(&rows(&demo(ids::HTUI_FEAT_1).await, 3)),
            [
                "FEAT-1",
                "→ origin ANA-1",
                "  ← blocked_by agy:ANA-1",
                "    ← blocked_by agy:FIX-1",
                "  ← origin FEAT-3 (*)",
                "  ← blocked_by TOOL-1",
                "    ← origin agy:FIX-1 (*)",
                "← blocked_by FEAT-2",
                "  ← relates agy:FEAT-1",
                "← relates FEAT-3",
            ]
        );
    }

    #[test]
    fn parallel_edges_and_a_self_loop_each_get_one_row() {
        let r = node(1, "R", 0);
        let x = node(2, "X", 1);
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Relates, &r),
                edge(&r, LinkKind::Relates, &x),
                edge(&r, LinkKind::BlockedBy, &x),
            ],
            nodes: vec![x, r],
        };
        let rows = rows(&graph, 3);
        assert_eq!(
            shape(&rows),
            ["R", "→ relates R (*)", "→ blocked_by X", "→ relates X (*)",]
        );
        assert_eq!(rows.len(), 1 + 3);
    }

    #[tokio::test]
    async fn plus_and_minus_clamp_and_survive_an_item_change() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        assert_eq!(tab.depth, DEFAULT_DEPTH);
        assert_eq!(DEFAULT_DEPTH, 2);
        for _ in 0..2 {
            assert_eq!(
                tab.on_key(key(KeyCode::Char('-')), &mut ctx),
                Handled::Consumed
            );
        }
        assert_eq!(tab.depth, 1, "`-` stops at 1");
        for _ in 0..3 {
            assert_eq!(
                tab.on_key(key(KeyCode::Char('+')), &mut ctx),
                Handled::Consumed
            );
        }
        assert_eq!(tab.depth, GraphTab::MAX_HOPS, "`+` stops at MAX_HOPS");
        tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        tab.on_item_change(Some(ids::HTUI_FEAT_2));
        assert_eq!(tab.depth, GraphTab::MAX_HOPS, "the depth is a preference");
        assert_eq!(tab.cursor, 0);
        assert!(tab.graph.is_none());
        assert!(shell.emit.is_empty());
    }

    #[tokio::test]
    async fn j_k_and_paging_clamp_to_the_rows() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        assert_eq!(
            tab.on_key(key(KeyCode::Char('K')), &mut ctx),
            Handled::Consumed
        );
        assert_eq!(tab.cursor, 0);
        for _ in 0..20 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        assert_eq!(tab.cursor, 7);
        assert_eq!(
            tab.on_key(key(KeyCode::PageUp), &mut ctx),
            Handled::Consumed
        );
        assert_eq!(tab.cursor, 0);
        assert_eq!(
            tab.on_key(key(KeyCode::PageDown), &mut ctx),
            Handled::Consumed
        );
        assert_eq!(tab.cursor, 7);

        // A terminal sends the capital with SHIFT; the harness sends it bare. Both move.
        tab.on_key(
            KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
            &mut ctx,
        );
        assert_eq!(tab.cursor, 6);

        let mut empty = GraphTab::new();
        empty.on_item_change(Some(ids::HTUI_FEAT_1));
        assert_eq!(
            empty.on_key(key(KeyCode::Char('J')), &mut ctx),
            Handled::Consumed
        );
        assert_eq!(empty.cursor, 0);
    }

    #[tokio::test]
    async fn a_depth_change_keeps_the_cursor_on_its_item() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        for _ in 0..6 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        assert_eq!(tab.visible()[tab.cursor].node.item_id, ids::AGY_FEAT_1);
        assert_eq!(tab.cursor, 6);
        tab.on_key(key(KeyCode::Char('+')), &mut ctx);
        assert_eq!(tab.cursor, 8);
        assert_eq!(tab.visible()[tab.cursor].node.item_id, ids::AGY_FEAT_1);
        tab.on_key(key(KeyCode::Char('-')), &mut ctx);
        tab.on_key(key(KeyCode::Char('-')), &mut ctx);
        assert_eq!(tab.cursor, 0, "agy FEAT-1 is two hops out");
    }

    #[tokio::test]
    async fn enter_on_a_node_in_the_workspace_reveals_it() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        let emitted = shell.emit.take();
        assert_eq!(emitted.len(), 1, "{emitted:?}");
        let Action::Reveal(target) = &emitted[0] else {
            panic!("a reveal: {emitted:?}");
        };
        assert_eq!(
            *target,
            RevealTarget::Item {
                id: ids::HTUI_ANA_1,
                key: "ANA-1".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn enter_on_the_root_does_nothing() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        assert!(shell.emit.is_empty());
    }

    #[tokio::test]
    async fn enter_outside_the_workspace_says_so() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::AGY_FIX_1).await);
        for _ in 0..3 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        let emitted = shell.emit.take();
        assert_eq!(emitted.len(), 1, "{emitted:?}");
        assert!(
            matches!(
                &emitted[0],
                Action::Error(text) if text == "vulkan-tutorials:FEAT-1 is outside this workspace"
            ),
            "{emitted:?}"
        );
    }

    #[tokio::test]
    async fn enter_with_no_graph_is_consumed() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = GraphTab::new();
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        tab.on_item_change(Some(ids::HTUI_FEAT_1));
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        assert!(shell.emit.is_empty());
    }

    #[tokio::test]
    async fn a_reply_for_another_root_is_ignored() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = GraphTab::new();
        tab.on_item_change(Some(ids::HTUI_FEAT_1));
        tab.on_reply(&StoreReply::Links(demo(ids::HTUI_FEAT_2).await), &mut ctx);
        assert!(tab.graph.is_none(), "FEAT-2's graph is not FEAT-1's");
        tab.on_reply(&StoreReply::Links(demo(ids::HTUI_FEAT_1).await), &mut ctx);
        assert_eq!(tab.visible().len(), 8);
    }

    #[tokio::test]
    async fn other_keys_pass() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        for event in [
            key(KeyCode::Char('j')),
            key(KeyCode::Char('x')),
            key(KeyCode::Char('q')),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('J'), KeyModifiers::ALT),
        ] {
            assert_eq!(tab.on_key(event, &mut ctx), Handled::Pass, "{event:?}");
        }
        assert_eq!(tab.cursor, 0);
        assert!(shell.emit.is_empty());
    }

    #[tokio::test]
    async fn every_line_fits_the_pane() {
        let shell = Shell::platform().await;
        for root in [ids::HTUI_FEAT_1, ids::AGY_FIX_1] {
            let graph = demo(root).await;
            for depth in 1..=GraphTab::MAX_HOPS {
                let rows = rows(&graph, depth);
                let root_project = rows[0].node.project_id;
                for (at, row) in rows.iter().enumerate() {
                    let drawn = line(
                        row,
                        at == 0,
                        root_project,
                        in_workspace(row.node, &shell.projects),
                        PANE,
                        &shell.theme,
                    );
                    assert!(
                        drawn.width() <= PANE,
                        "{:?} is {} columns",
                        text(&drawn),
                        drawn.width()
                    );
                }
            }
        }

        let graph = demo(ids::AGY_FIX_1).await;
        let rows = rows(&graph, 2);
        let drawn: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(at, row)| {
                text(&line(
                    row,
                    at == 0,
                    rows[0].node.project_id,
                    in_workspace(row.node, &shell.projects),
                    PANE,
                    &shell.theme,
                ))
            })
            .collect();
        assert_eq!(
            drawn,
            [
                "▸ FIX-1 open Session leak on cancel",
                "  → blocked_by ANA-1 done Prompt assembly …",
                "    → blocked_by htui:ANA-1 done Data mode…",
                "  → origin vulkan-tutor…:FEAT-1 in_progress",
                "  → origin htui:TOOL-1 awaiting_approval",
                "    → blocked_by htui:ANA-1 done (*)",
            ]
        );
    }

    #[tokio::test]
    async fn feat_1_draws_the_worked_rows() {
        let shell = Shell::platform().await;
        let graph = demo(ids::HTUI_FEAT_1).await;
        let rows = rows(&graph, 2);
        let drawn: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(at, row)| {
                text(&line(
                    row,
                    at == 0,
                    rows[0].node.project_id,
                    true,
                    PANE,
                    &shell.theme,
                ))
            })
            .collect();
        assert_eq!(
            drawn,
            [
                "▸ FEAT-1 in_progress TUI scaffold",
                "  → origin     ANA-1 done Data model, box …",
                "    ← blocked_by agy:ANA-1 done Prompt ass…",
                "    ← origin     FEAT-3 queued (*)",
                "    ← blocked_by TOOL-1 awaiting_approval",
                "  ← blocked_by FEAT-2 blocked Agent driver…",
                "    ← relates    agy:FEAT-1 open ACP trans…",
                "  ← relates    FEAT-3 queued Postgres stor…",
            ]
        );
    }

    /// The pane drawn into a `width` x `height` area, one `String` per row, trailing blanks
    /// trimmed (the Runs pane's `lines`).
    fn draw(tab: &GraphTab, shell: &Shell, width: u16, height: u16) -> Vec<String> {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
            .expect("the test backend is constructible");
        term.draw(|frame| tab.render(frame, frame.area(), &shell.ctx()))
            .expect("the pane draws");
        let buffer = term.backend().buffer();
        (buffer.area.top()..buffer.area.bottom())
            .map(|y| {
                let row: String = (buffer.area.left()..buffer.area.right())
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// M1: a depth `+` / `-` cannot change is a no-op, so the cursor stays on a `(*)` row rather
    /// than jumping to its item's own row.
    #[tokio::test]
    async fn a_clamped_depth_change_leaves_the_cursor_where_it_is() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        tab.on_key(key(KeyCode::Char('+')), &mut ctx);
        assert_eq!(tab.depth, GraphTab::MAX_HOPS);
        for _ in 0..4 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        let on_repeat = |tab: &GraphTab| {
            let row = tab.visible()[tab.cursor];
            row.is_repeat() && row.node.item_id == ids::HTUI_FEAT_3
        };
        assert!(on_repeat(&tab), "{:#?}", shape(&tab.visible()));
        tab.on_key(key(KeyCode::Char('+')), &mut ctx);
        assert_eq!(tab.cursor, 4, "`+` at MAX_HOPS moves nothing");

        tab.on_key(key(KeyCode::Char('-')), &mut ctx);
        tab.on_key(key(KeyCode::Char('-')), &mut ctx);
        assert_eq!(tab.depth, 1);
        // Depth 1 is `FEAT-1`, `ANA-1`, `FEAT-3 (*)`, `FEAT-2`, `FEAT-3`: back up onto the repeat.
        tab.on_key(key(KeyCode::Char('K')), &mut ctx);
        tab.on_key(key(KeyCode::Char('K')), &mut ctx);
        assert_eq!(tab.cursor, 2);
        assert!(on_repeat(&tab), "{:#?}", shape(&tab.visible()));
        tab.on_key(key(KeyCode::Char('-')), &mut ctx);
        assert_eq!(tab.cursor, 2, "`-` at 1 moves nothing");
    }

    /// L5: when the slug cannot keep two cells the key is what is clipped, never the slug; and a
    /// label with no room left is empty rather than a one-cell-over ellipsis.
    #[tokio::test]
    async fn the_narrowest_label_keeps_the_key() {
        let shell = Shell::platform().await;
        let graph = demo(ids::AGY_FIX_1).await;
        let rows = rows(&graph, 2);
        let vulkan = rows
            .iter()
            .find(|row| row.node.item_id == ids::VULKAN_FEAT_1)
            .expect("vulkan FEAT-1 is one hop from agy FIX-1");
        let at = |width| {
            let drawn = line(
                vulkan,
                false,
                rows[0].node.project_id,
                false,
                width,
                &shell.theme,
            );
            assert!(drawn.width() <= width, "{:?} at {width}", text(&drawn));
            text(&drawn)
        };
        // 17 fixed cells and `origin` leave 8 for the label: one for the slug, so it goes whole.
        assert_eq!(at(31), "  → origin FEAT-1 in_progress");
        assert_eq!(at(25), "  → origin F… in_progress");
        assert_eq!(at(23), "  → origin  in_progress");

        // A label with no slug is clipped too, not left to the pane edge.
        let graph = demo(ids::HTUI_FEAT_1).await;
        let feat_1 = super::rows(&graph, 2);
        let drawn = line(
            &feat_1[1],
            false,
            feat_1[0].node.project_id,
            true,
            18,
            &shell.theme,
        );
        assert_eq!(text(&drawn), "  → origin A… done");
    }

    /// L8: no graph yet is a pending read, not an item without links.
    #[tokio::test]
    async fn the_pane_says_loading_until_the_links_arrive() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = GraphTab::new();
        tab.on_item_change(Some(ids::HTUI_FEAT_1));
        assert_eq!(draw(&tab, &shell, 43, 6)[0], "Loading links\u{2026}");
        tab.on_reply(&StoreReply::Links(demo(ids::HTUI_FEAT_1).await), &mut ctx);
        assert_eq!(
            draw(&tab, &shell, 43, 6)[0],
            "▸ FEAT-1 in_progress TUI scaffold"
        );
    }

    /// L8: a refused `Links` read says so, and the next selection is pending again.
    #[tokio::test]
    async fn a_failed_links_read_says_so() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = GraphTab::new();
        tab.on_item_change(Some(ids::HTUI_FEAT_1));
        tab.on_reply(
            &StoreReply::Failed {
                request: "documents",
                message: "not ours".to_owned(),
            },
            &mut ctx,
        );
        assert_eq!(draw(&tab, &shell, 43, 6)[0], "Loading links\u{2026}");
        tab.on_reply(
            &StoreReply::Failed {
                request: LINKS,
                message: "connection refused".to_owned(),
            },
            &mut ctx,
        );
        assert_eq!(
            draw(&tab, &shell, 43, 6)[0],
            "Links unavailable: connection refused"
        );
        tab.on_item_change(Some(ids::HTUI_FEAT_2));
        assert_eq!(draw(&tab, &shell, 43, 6)[0], "Loading links\u{2026}");
        tab.on_reply(&StoreReply::Links(demo(ids::HTUI_FEAT_2).await), &mut ctx);
        assert_eq!(
            tab.visible().len(),
            6,
            "a later answer replaces the refusal"
        );
    }

    #[test]
    fn links_is_the_links_request_name() {
        let request = crate::store_worker::StoreRequest::Links {
            id: ids::HTUI_FEAT_1,
            hops: GraphTab::MAX_HOPS,
        };
        assert_eq!(request.name(), LINKS);
    }

    #[tokio::test]
    async fn an_out_of_workspace_node_is_dim() {
        let shell = Shell::platform().await;
        let graph = demo(ids::AGY_FIX_1).await;
        let rows = rows(&graph, 2);
        let vulkan = rows
            .iter()
            .find(|row| row.node.item_id == ids::VULKAN_FEAT_1)
            .expect("vulkan FEAT-1 is one hop from agy FIX-1");
        assert!(!in_workspace(vulkan.node, &shell.projects));
        let drawn = line(
            vulkan,
            false,
            rows[0].node.project_id,
            false,
            PANE,
            &shell.theme,
        );
        let label = drawn
            .spans
            .iter()
            .find(|span| span.content.contains("FEAT-1"))
            .expect("the label span");
        assert_eq!(label.style, shell.theme.dim);
    }
}
