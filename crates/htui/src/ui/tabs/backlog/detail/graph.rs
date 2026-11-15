//! The Graph sub-tab: the selected item's link neighbourhood as a navigable tree (MOD-14).
//!
//! The Backlog reads the traversal once per selection at [`GraphTab::MAX_HOPS`], and the view
//! depth (`+` / `-`) filters it here, so a depth change costs no read (plan D2). The rows form a
//! `cargo tree`-style tree: one row per edge, each item expanded once under the first parent that
//! reaches it, and every other edge a `(*)` leaf (D3), its arrow read from the row it hangs under
//! (D4). `Enter` re-roots by emitting `Action::Reveal` for the item under the cursor (D6), so the
//! sub-tab never names the list it re-roots.
//!
//! MOD-115 M4: the rows are a `kit::list`, the cursor its gutter and band (MOD-85); an edge's kind
//! reads as `label::link_kind`'s words and a node's status as its `kit::status` mark (MOD-83).
//!
//! MOD-150 M3 D12: an item's linked documents are leaf rows under its own expanded row, after its
//! children: the kind, `[slug:]KEY doc_kind` (an owner document's `slug doc_kind`), the pinned
//! version beside the latest, then the title. `Enter` on one reveals the owning item.

use core::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use htui_core::model::{
    ItemId, ItemState, LinkGraph, LinkKind, LinkNode, LinkedDocument, ProjectId, ProjectRef,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width, graphemes};
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::label;
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::scroll::{PAGE, Step, window};
use crate::ui::kit::status;
use crate::ui::tabs::backlog::detail::{
    CHOOSE_UNBOUND, DetailId, DetailTab, Framing, Told, message, no_item, unavailable,
};
use crossterm::event::KeyEvent;

/// The footer's key half (MOD-67 M5): `+/- depth · Enter re-root` with the defaults, read
/// through `BACKLOG_GRAPH` so a rebound key shows its own label.
const GRAPH_HINT: HintSpec = &[
    Hint::Pair(Act::GraphDeeper, Act::GraphShallower, "depth"),
    Hint::One(Act::GraphReroot, "re-root"),
];

/// An edge that leaves the parent row's item (D4).
const OUT: &str = "\u{2192}";

/// An edge that arrives at the parent row's item (D4).
const IN: &str = "\u{2190}";

/// Default view depth (D2).
const DEFAULT_DEPTH: u8 = 2;

/// Columns one tree level indents by.
const INDENT: usize = 2;

/// What the pane says when the item has no link, and where links come from.
const NO_LINKS: (&str, NextStep<'static>) = (
    "No links for this item.",
    NextStep("a run step proposes the links shown here"),
);

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

/// What a [`TreeRow`] names, as an index into the `LinkGraph` it was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeTarget {
    /// Index into `LinkGraph::nodes`.
    Node(usize),
    /// Index into `LinkGraph::documents` (MOD-150 M3 D12).
    Document(usize),
}

/// One line of the tree, as an index into the `LinkGraph` it was built from.
///
/// Stored rather than borrowed, so the tree is built once per reply and per depth change and never
/// per key or per frame (M2); [`TreeRow::resolve`] turns it into a [`GraphRow`] to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TreeRow {
    /// `0` for the root, one more than the row it hangs under otherwise.
    level: u8,
    /// `None` on the root row.
    link: Option<Link>,
    /// The item or document this row names.
    target: TreeTarget,
}

impl TreeRow {
    /// The row read against `graph`, which must be the graph it was built from.
    fn resolve(self, graph: &LinkGraph) -> GraphRow<'_> {
        GraphRow {
            level: self.level,
            link: self.link,
            target: match self.target {
                TreeTarget::Node(at) => Target::Node(&graph.nodes[at]),
                TreeTarget::Document(at) => Target::Document(&graph.documents[at]),
            },
        }
    }
}

/// What a [`GraphRow`] names, borrowed from the `LinkGraph` it was built from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target<'a> {
    /// An item reached by the walk.
    Node(&'a LinkNode),
    /// A document an expanded item links (MOD-150 M3 D12): always a leaf.
    Document(&'a LinkedDocument),
}

/// One line of the tree, borrowed from the `LinkGraph` it was built from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GraphRow<'a> {
    /// `0` for the root, one more than the row it hangs under otherwise.
    level: u8,
    /// `None` on the root row.
    link: Option<Link>,
    /// The item or document this row names.
    target: Target<'a>,
}

impl<'a> GraphRow<'a> {
    /// Whether this row is a `(*)` leaf rather than its item's own row.
    fn is_repeat(&self) -> bool {
        self.link.is_some_and(|link| link.repeat)
    }

    /// The item this row names; `None` on a document's row.
    const fn node(&self) -> Option<&'a LinkNode> {
        match self.target {
            Target::Node(node) => Some(node),
            Target::Document(_) => None,
        }
    }

    /// The project of the item or document this row names: what dims it (D7 of MOD-147).
    const fn project_id(&self) -> ProjectId {
        match self.target {
            Target::Node(node) => node.project_id,
            Target::Document(document) => document.project_id,
        }
    }
}

/// The selected item's link neighbourhood as a navigable tree (MOD-14).
#[derive(Debug)]
pub struct GraphTab {
    /// The `Links` reply for the selected item, fetched at `MAX_HOPS`.
    graph: Option<LinkGraph>,
    /// `graph`'s tree at `depth`: rebuilt when either changes, empty while no graph is loaded.
    rows: Vec<TreeRow>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// Why the `Links` read for the selected item was refused, until another answer arrives.
    failed: Option<String>,
    /// Index into `rows`.
    cursor: usize,
    /// View depth, `1..=MAX_HOPS`. A preference: survives `on_item_change` (D2).
    depth: u8,
    /// How the detail pane and the list beside it are drawn this frame (`DetailTab::framing`,
    /// MOD-115 M4 RV2-L1; RV, repair 2): what the no-item and failed-read steps read.
    told: Told,
    /// MOD-147 M3 D3: the workspace's scope row the Backlog's last `Items` reply named, so a
    /// workspace item's node is in the workspace ([`workspace_projects`](Self::workspace_projects)).
    scope_row: Option<ProjectRef>,
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
            rows: Vec::new(),
            item: None,
            failed: None,
            cursor: 0,
            depth: DEFAULT_DEPTH,
            told: Told::new(),
            scope_row: None,
        }
    }

    /// The Backlog's groups (MOD-147 M3 D3): the scope row its last `Items` reply named, then
    /// `ctx.projects`. What `in_workspace` reads, so a workspace item's node is neither dimmed
    /// nor refused on `Enter`.
    /// Borrowed, never collected: it is read on every `Enter` and every frame.
    fn workspace_projects<'s>(
        &'s self,
        ctx: &'s Ctx<'_>,
    ) -> impl Iterator<Item = &'s ProjectRef> + Clone {
        self.scope_row.iter().chain(ctx.projects)
    }

    /// Rebuilds `rows` from `graph` at `depth`: the one place the tree is built.
    fn rebuild(&mut self) {
        self.rows = self
            .graph
            .as_ref()
            .map(|graph| tree(graph, self.depth))
            .unwrap_or_default();
    }

    /// Row `at`, read against the graph it was built from.
    fn row(&self, at: usize) -> Option<GraphRow<'_>> {
        let graph = self.graph.as_ref()?;
        self.rows.get(at).map(|row| row.resolve(graph))
    }

    /// The rows at the current view depth; empty while no graph is loaded.
    fn visible(&self) -> Vec<GraphRow<'_>> {
        self.graph.as_ref().map_or_else(Vec::new, |graph| {
            self.rows.iter().map(|row| row.resolve(graph)).collect()
        })
    }

    /// Moves the cursor `delta` rows, clamped to the rows (0 when there are none).
    fn move_cursor(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    /// Changes the view depth, keeping the cursor on its item's own row when it is still shown.
    ///
    /// The row index moves with the depth (preorder changes), so the cursor follows the item, not
    /// the index; an item the new depth hides sends it back to the root. A depth the clamp left
    /// where it was changes nothing, so the cursor stays even on a `(*)` row.
    ///
    /// A document's row keeps the cursor on that document while its item is still expanded.
    fn set_depth(&mut self, depth: u8) {
        if depth == self.depth {
            return;
        }
        let item = self
            .row(self.cursor)
            .and_then(|row| row.node())
            .map(|node| node.item_id);
        let at = self.rows.get(self.cursor).map(|row| row.target);
        self.depth = depth;
        self.rebuild();
        self.cursor = match (at, item) {
            (Some(TreeTarget::Document(_)), _) => {
                self.rows.iter().position(|row| Some(row.target) == at)
            }
            (_, Some(id)) => self.visible().iter().position(|row| {
                !row.is_repeat() && row.node().is_some_and(|node| node.item_id == id)
            }),
            _ => None,
        }
        .unwrap_or(0);
    }

    /// `Enter`: re-root on the item under the cursor, or say why not (D6, D7). Nothing on the
    /// root row, or with no graph loaded.
    fn re_root(&self, ctx: &Ctx<'_>) {
        let Some(row) = self.row(self.cursor) else {
            return;
        };
        if row.link.is_none() {
            return;
        }
        let (id, key, slug) = match row.target {
            Target::Node(node) => (node.item_id, &node.key, &node.project_slug),
            Target::Document(document) => {
                // MOD-150 M3 D12: an item's document reveals its owner; an owner document has
                // no item to reveal.
                let (Some(id), Some(key)) = (document.owner_item, &document.owner_key) else {
                    let owner = match document.project_slug.strip_prefix('@') {
                        Some(workspace) => format!("workspace {workspace}"),
                        None => format!("project {}", document.project_slug),
                    };
                    ctx.emit(Action::Error(format!(
                        "{} is {owner}'s own document; open it in Settings",
                        document_key(document),
                    )));
                    return;
                };
                (id, key, &document.project_slug)
            }
        };
        if in_workspace(row.project_id(), self.workspace_projects(ctx)) {
            ctx.emit(Action::Reveal(RevealTarget::Item {
                id,
                key: key.clone(),
            }));
        } else {
            // D7: revealing it would only re-read the list to answer `not_in_this_backlog`.
            ctx.emit(Action::Error(format!(
                "{slug}:{key} is outside this workspace"
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

/// An edge between two ranked nodes: `from` and `to` are ranks, `edge` indexes `LinkGraph::edges`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ranked {
    /// Rank of the edge's `from` end.
    from: usize,
    /// Rank of the edge's `to` end.
    to: usize,
    /// Index into `LinkGraph::edges`.
    edge: usize,
}

/// The tree of `graph` cut at view depth `depth` (D2, D3, D4), in preorder, the root first.
///
/// Each expanded item's documents follow its children, one level deeper (MOD-150 M3 D12); a
/// `(*)` row has none, so a document is drawn once. Nodes are ranked by [`rank_nodes`] and edges
/// by [`rank_edges`]; [`tree_parents`] picks each
/// node's tree edge and [`hang_edges`] hangs every other edge as a `(*)` leaf. A node with no
/// neighbour one level up is dropped with its edges. That is a malformed reply, or a legitimate
/// offline one: the cache walks links through items whose project it does not mirror but cannot
/// return those items, so a node reached only through one arrives with its depth and without the
/// neighbour one level up. The walk terminates because a tree parent is strictly shallower than
/// its child.
fn tree(graph: &LinkGraph, depth: u8) -> Vec<TreeRow> {
    let nodes = rank_nodes(graph, depth);
    let Some(&root) = nodes.first() else {
        return Vec::new();
    };
    let edges = rank_edges(graph, &nodes);
    let depths: Vec<u8> = nodes.iter().map(|&at| graph.nodes[at].depth).collect();
    let parent = tree_parents(&depths, &edges);
    let under = hang_edges(graph, &nodes, &edges, &parent);
    let documents = documents_by_item(graph);
    let mut out = vec![TreeRow {
        level: 0,
        link: None,
        target: TreeTarget::Node(root),
    }];
    let walk = Walk {
        graph,
        nodes: &nodes,
        under: &under,
        documents: &documents,
    };
    walk.rows(0, 1, &mut out);
    out
}

/// Each item's documents, as indices into `LinkGraph::documents`, in the order they are drawn.
/// That is the reply's order: every backend answers it sorted by
/// [`LinkGraph::sort_documents`], whose order within one item, `(kind, project slug, owner key,
/// doc kind, version)` by bytes, is the one drawn, so no re-sort here.
fn documents_by_item(graph: &LinkGraph) -> HashMap<ItemId, Vec<usize>> {
    let mut by_item: HashMap<ItemId, Vec<usize>> = HashMap::new();
    for (at, document) in graph.documents.iter().enumerate() {
        by_item.entry(document.from_item_id).or_default().push(at);
    }
    by_item
}

/// The nodes shown at view depth `depth`, as indices into `LinkGraph::nodes` in rank order: the
/// root first whatever depth the reply gave it, then [`node_order`]. Empty when the reply does not
/// hold its own root.
///
/// One rank per item: the first copy in rank order wins. A set rather than `dedup_by_key`, which
/// only drops *adjacent* duplicates, and two copies of an item at different depths are not.
fn rank_nodes(graph: &LinkGraph, depth: u8) -> Vec<usize> {
    let root = graph.root;
    if graph.node(root).is_none() {
        return Vec::new();
    }
    let mut ranked: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.item_id == root || (node.depth > 0 && node.depth <= depth))
        .map(|(at, _)| at)
        .collect();
    ranked.sort_by(|&a, &b| {
        let (a, b) = (&graph.nodes[a], &graph.nodes[b]);
        (a.item_id != root)
            .cmp(&(b.item_id != root))
            .then_with(|| node_order(a, b))
    });
    let mut seen = HashSet::with_capacity(ranked.len());
    ranked.retain(|&at| seen.insert(graph.nodes[at].item_id));
    ranked
}

/// The edges between two ranked nodes, ordered by their endpoints' ranks, then their kind.
fn rank_edges(graph: &LinkGraph, nodes: &[usize]) -> Vec<Ranked> {
    let rank: HashMap<ItemId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(rank, &at)| (graph.nodes[at].item_id, rank))
        .collect();
    let mut edges: Vec<Ranked> = graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(at, edge)| {
            Some(Ranked {
                from: *rank.get(&edge.from_item_id)?,
                to: *rank.get(&edge.to_item_id)?,
                edge: at,
            })
        })
        .collect();
    edges.sort_by(|a, b| {
        a.from.cmp(&b.from).then(a.to.cmp(&b.to)).then_with(|| {
            let (a, b) = (&graph.edges[a.edge], &graph.edges[b.edge]);
            a.kind.as_str().cmp(b.kind.as_str())
        })
    });
    edges
}

/// Each ranked node's tree edge, as `(parent rank, index into edges)`; `None` for the root and for
/// a node that is dropped.
///
/// A node's tree parent is its first-ranked neighbour one level up (`depths` are the nodes' reply
/// depths, by rank) that is itself kept, joined by the first edge between the two. Ranks ascend
/// with depth, so every candidate parent has been decided (kept or dropped) before its children
/// are looked at.
fn tree_parents(depths: &[u8], edges: &[Ranked]) -> Vec<Option<(usize, usize)>> {
    // Each node's neighbours, as `(other rank, index into edges)`. A self-loop is nobody's parent.
    let mut adjacent: Vec<Vec<(usize, usize)>> = vec![Vec::new(); depths.len()];
    for (at, edge) in edges.iter().enumerate() {
        if edge.from != edge.to {
            adjacent[edge.from].push((edge.to, at));
            adjacent[edge.to].push((edge.from, at));
        }
    }
    let mut parent: Vec<Option<(usize, usize)>> = vec![None; depths.len()];
    let mut kept = vec![false; depths.len()];
    if let Some(root) = kept.first_mut() {
        *root = true;
    }
    for child in 1..depths.len() {
        let wanted = depths[child].checked_sub(1);
        parent[child] = adjacent[child]
            .iter()
            .copied()
            .filter(|&(other, _)| kept[other] && Some(depths[other]) == wanted)
            .min();
        kept[child] = parent[child].is_some();
    }
    parent
}

/// What hangs under each ranked node, sorted: its tree children and the non-tree edges it owns.
///
/// A non-tree edge hangs, as a `(*)` leaf, under its shallower endpoint (the first-ranked one on a
/// tie); an edge with a dropped end is not drawn.
fn hang_edges(
    graph: &LinkGraph,
    nodes: &[usize],
    edges: &[Ranked],
    parent: &[Option<(usize, usize)>],
) -> Vec<Vec<(usize, Link)>> {
    let node = |rank: usize| &graph.nodes[nodes[rank]];
    let kept = |rank: usize| rank == 0 || parent[rank].is_some();
    let mut under: Vec<Vec<(usize, Link)>> = vec![Vec::new(); nodes.len()];
    for (at, &Ranked { from, to, edge }) in edges.iter().enumerate() {
        if !kept(from) || !kept(to) {
            continue;
        }
        let edge = &graph.edges[edge];
        let tree = |child: usize| parent[child].is_some_and(|(_, tree_edge)| tree_edge == at);
        let (owner, other, repeat) = if from != to && tree(to) {
            (from, to, false)
        } else if from != to && tree(from) {
            (to, from, false)
        } else {
            (from.min(to), from.max(to), true)
        };
        // A self-loop has `owner == other` and reads outward.
        let arrow = if node(owner).item_id == edge.from_item_id {
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
            let (a, b) = (node(*a), node(*b));
            a.key
                .as_bytes()
                .cmp(b.key.as_bytes())
                .then_with(|| a.project_slug.as_bytes().cmp(b.project_slug.as_bytes()))
                .then_with(|| a.item_id.cmp(&b.item_id))
                .then_with(|| a_link.kind.as_str().cmp(b_link.kind.as_str()))
                .then_with(|| (a_link.arrow == Arrow::In).cmp(&(b_link.arrow == Arrow::In)))
        });
    }
    under
}

/// What [`Walk::rows`] reads: the ranked nodes, what hangs under each and each item's documents.
struct Walk<'g> {
    /// The graph the tree is built from.
    graph: &'g LinkGraph,
    /// Ranked nodes, as indices into `LinkGraph::nodes`.
    nodes: &'g [usize],
    /// What hangs under each ranked node ([`hang_edges`]).
    under: &'g [Vec<(usize, Link)>],
    /// Each item's documents ([`documents_by_item`]).
    documents: &'g HashMap<ItemId, Vec<usize>>,
}

impl Walk<'_> {
    /// Preorder: the rows under rank `at`, each at `level`, a tree child followed by its own
    /// subtree, then `at`'s own documents (MOD-150 M3 D12).
    fn rows(&self, at: usize, level: u8, out: &mut Vec<TreeRow>) {
        for &(other, link) in &self.under[at] {
            out.push(TreeRow {
                level,
                link: Some(link),
                target: TreeTarget::Node(self.nodes[other]),
            });
            if !link.repeat {
                self.rows(other, level.saturating_add(1), out);
            }
        }
        let item = self.graph.nodes[self.nodes[at]].item_id;
        for &document in self.documents.get(&item).into_iter().flatten() {
            out.push(TreeRow {
                level,
                link: Some(Link {
                    arrow: Arrow::Out,
                    kind: self.graph.documents[document].kind.as_link_kind(),
                    repeat: false,
                }),
                target: TreeTarget::Document(document),
            });
        }
    }
}

/// Whether `project` is one of the workspace's (D7).
fn in_workspace<'p>(
    project: ProjectId,
    projects: impl IntoIterator<Item = &'p ProjectRef>,
) -> bool {
    projects
        .into_iter()
        .any(|member| member.project_id == project)
}

/// The cells the kind column is padded to: the widest of `label::link_kind`'s words (`blocked
/// by`, `supersedes`, `duplicates`, `implements`, ten cells each), so the column is as wide as
/// its longest word and no wider.
fn kind_width() -> usize {
    LinkKind::ALL
        .iter()
        .map(|kind| cell_width(label::link_kind(*kind)))
        .max()
        .unwrap_or(0)
}

/// A node's state as the pane shows it: its `kit::status` glyph and word, one span (MOD-144: the
/// derived [`ItemState`], not the stored status).
fn status_text(state: ItemState) -> String {
    let mark = status::mark(state);
    format!("{} {}", mark.glyph, mark.word)
}

/// Every row as a line at most `width` cells wide: the content of a `kit::list` row, which draws
/// the gutter (and the cursor's band) in the cell before it.
///
/// The kind column is padded to [`kind_width`] for every row or for none (L6): padded when every
/// row still fits with it, so a narrow pane drops the padding uniformly rather than row by row.
fn lines<'p>(
    rows: &[GraphRow<'_>],
    projects: impl IntoIterator<Item = &'p ProjectRef> + Clone,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(root) = rows.first() else {
        return Vec::new();
    };
    let root_project = root.project_id();
    let kind_width = kind_width();
    let pad_kind = rows.iter().all(|row| match row.link {
        None => true,
        Some(link) => {
            let kind = cell_width(label::link_kind(link.kind)).max(kind_width);
            fixed_cells(row, link) + kind + label_cells(row, root_project) <= width
        }
    });
    rows.iter()
        .map(|row| {
            line(
                row,
                root_project,
                in_workspace(row.project_id(), projects.clone()),
                pad_kind.then_some(kind_width),
                width,
                theme,
            )
        })
        .collect()
}

/// Columns a row at `level` indents by, capped at [`GraphTab::MAX_HOPS`] levels.
fn indent(level: u8) -> usize {
    (INDENT * usize::from(level.saturating_sub(1))).min(INDENT * usize::from(GraphTab::MAX_HOPS))
}

/// The slug a node is labelled with: its project's, unless that is the root's.
fn slug(node: &LinkNode, root_project: ProjectId) -> Option<&str> {
    (node.project_id != root_project).then_some(node.project_slug.as_str())
}

/// Cells of a non-root row before its label, but the kind: the space after the gutter, indent,
/// arrow and the separators either side of the kind. Everything the key never gives way to.
fn lead_cells(row: &GraphRow<'_>, link: Link) -> usize {
    1 + indent(row.level) + cell_width(link.arrow.glyph()) + 2
}

/// Cells of a non-root row but its kind, label and title: [`lead_cells`], the separator before
/// the status, the status and the repeat mark; a document's version text in the status's place.
fn fixed_cells(row: &GraphRow<'_>, link: Link) -> usize {
    let repeat = if link.repeat { cell_width(REPEAT) } else { 0 };
    let status = match row.target {
        Target::Node(node) => cell_width(&status_text(node.state())),
        Target::Document(document) => cell_width(&version_text(document)),
    };
    lead_cells(row, link) + 1 + status + repeat
}

/// Cells of the row's whole label: `[slug:]KEY`, or a document's `[slug:]KEY doc_kind`.
fn label_cells(row: &GraphRow<'_>, root_project: ProjectId) -> usize {
    let (slug, key) = match row.target {
        Target::Node(node) => (slug(node, root_project), cell_width(&node.key)),
        Target::Document(document) => (
            document_slug(document, root_project),
            cell_width(&document_key(document)),
        ),
    };
    slug.map_or(0, |slug| cell_width(slug) + 1) + key
}

/// A document's label but the slug (MOD-150 M3 D12): `KEY doc_kind` for an item's document,
/// `slug doc_kind` for an owner document, whose owner is its project.
fn document_key(document: &LinkedDocument) -> String {
    let owner = document
        .owner_key
        .as_deref()
        .unwrap_or(&document.project_slug);
    format!("{owner} {}", document.doc_kind)
}

/// The slug an item's document is labelled with: its project's, unless that is the root's. An
/// owner document's slug is its key already.
fn document_slug(document: &LinkedDocument, root_project: ProjectId) -> Option<&str> {
    (document.owner_key.is_some() && document.project_id != root_project)
        .then_some(document.project_slug.as_str())
}

/// The pinned version, and the latest beside it when that is newer: `v2 (latest v4)`.
fn version_text(document: &LinkedDocument) -> String {
    if document.version == document.latest_version {
        format!("v{}", document.version)
    } else {
        format!(
            "v{} (latest v{})",
            document.version, document.latest_version
        )
    }
}

/// One row as a line at most `width` cells wide (§1.5 of the blueprint), its kind padded to
/// `pad_kind` cells when the whole column is.
fn line(
    row: &GraphRow<'_>,
    root_project: ProjectId,
    in_workspace: bool,
    pad_kind: Option<usize>,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let node = match row.target {
        Target::Node(node) => node,
        Target::Document(document) => {
            return document_line(
                row,
                document,
                root_project,
                in_workspace,
                pad_kind,
                width,
                theme,
            );
        }
    };
    // D7: a node outside the workspace is drawn, but dim, so it does not read as reachable.
    let (label_style, status_style, title_style) = if in_workspace {
        (theme.key, theme.status_style(node.state()), theme.base)
    } else {
        (theme.dim, theme.dim, theme.dim)
    };
    let status = status_text(node.state());
    let mut spans = vec![Span::raw(" ")];

    let Some(link) = row.link else {
        // Clipped to its own width, so never cut, only flattened (D3): `cut` measures a control
        // at one cell that `ratatui` would draw as nothing.
        let key = cells::clip(&node.key, cell_width(&node.key));
        spans.push(Span::styled(key, label_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(status, status_style));
        push_title(&mut spans, &node.title, width, title_style);
        return Line::from(cut(spans, width));
    };

    let kind = label::link_kind(link.kind);
    let kind = match pad_kind {
        Some(cells) => format!("{kind:<cells$}"),
        None => kind.to_owned(),
    };
    // The label's room with the status kept, and the key's once the status gives way: the status
    // is cut at the pane edge by `cut`, the key only when nothing else is left (L5).
    let room = width.saturating_sub(fixed_cells(row, link) + cell_width(&kind));
    let bare_room = width.saturating_sub(lead_cells(row, link) + cell_width(&kind));
    let label = fit_label(&node.key, slug(node, root_project), room, bare_room);

    spans.push(Span::raw(" ".repeat(indent(row.level))));
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
    Line::from(cut(spans, width))
}

/// A document's row (MOD-150 M3 D12): the arrow, the kind padded as the items' are, the label,
/// the version text and the title. The title gives way first, then the slug, then the version
/// text at the pane edge, the label's key last, as on an item's row.
fn document_line(
    row: &GraphRow<'_>,
    document: &LinkedDocument,
    root_project: ProjectId,
    in_workspace: bool,
    pad_kind: Option<usize>,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let (label_style, version_style, title_style) = if in_workspace {
        (theme.key, theme.base, theme.base)
    } else {
        (theme.dim, theme.dim, theme.dim)
    };
    let link = row.link.unwrap_or(Link {
        arrow: Arrow::Out,
        kind: document.kind.as_link_kind(),
        repeat: false,
    });
    let kind = label::link_kind(link.kind);
    let kind = match pad_kind {
        Some(cells) => format!("{kind:<cells$}"),
        None => kind.to_owned(),
    };
    let room = width.saturating_sub(fixed_cells(row, link) + cell_width(&kind));
    let bare_room = width.saturating_sub(lead_cells(row, link) + cell_width(&kind));
    let label = fit_label(
        &document_key(document),
        document_slug(document, root_project),
        room,
        bare_room,
    );

    let mut spans = vec![
        Span::raw(" "),
        Span::raw(" ".repeat(indent(row.level))),
        Span::styled(link.arrow.glyph(), theme.dim),
        Span::raw(" "),
        Span::styled(kind, theme.base),
        Span::raw(" "),
        Span::styled(label, label_style),
        Span::raw(" "),
        Span::styled(version_text(document), version_style),
    ];
    push_title(&mut spans, &document.title, width, title_style);
    Line::from(cut(spans, width))
}

/// The `[slug:]KEY` label (§1.5 steps 3-4, L5): `room` is what the label has beside the whole
/// status, `bare_room` what the key alone has once the status gives way.
///
/// Whole when it fits `room`; then the slug clipped, while it keeps two cells; then the key alone,
/// in `bare_room`, clipped only if even that is too little. The degradation order is title, kind
/// padding, slug, status, key: the key is what names the item. Every part goes through
/// `cells::clip`, so a control character draws as one blank cell even when nothing is cut.
fn fit_label(key: &str, slug: Option<&str>, room: usize, bare_room: usize) -> String {
    let key_width = cell_width(key);
    if let Some(slug) = slug {
        if cell_width(slug) + 1 + key_width <= room {
            return cells::clip(&format!("{slug}:{key}"), room);
        }
        if let Some(slug_room) = room.checked_sub(key_width + 1)
            && slug_room >= 2
        {
            return format!(
                "{}:{}",
                cells::clip(slug, slug_room),
                cells::clip(key, key_width)
            );
        }
    }
    cells::clip(key, bare_room.max(room))
}

/// Appends `' ' + title`, clipped to what is left of `width`, when at least [`TITLE_MIN`]
/// cells are left for it: titles are the first thing a narrow pane drops.
fn push_title(spans: &mut Vec<Span<'static>>, title: &str, width: usize, style: Style) {
    let used: usize = spans.iter().map(|span| cell_width(&span.content)).sum();
    let room = width.saturating_sub(used + 1);
    if room >= TITLE_MIN {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(cells::clip(title, room), style));
    }
}

/// `spans` cut at `width` cells, as the pane edge would cut them: below the width of a row's
/// fixed parts nothing else gives, and a line must still never be wider than the pane.
fn cut(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut left = width;
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        let cells = cell_width(&span.content);
        if cells <= left {
            left -= cells;
            out.push(span);
            continue;
        }
        let mut kept = String::new();
        for grapheme in graphemes(&span.content) {
            let cells = cell_width(grapheme);
            if cells > left {
                break;
            }
            left -= cells;
            kept.push_str(grapheme);
        }
        if !kept.is_empty() {
            out.push(Span::styled(kept, span.style));
        }
        break;
    }
    out
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
        self.rows.clear();
        self.failed = None;
        self.cursor = 0;
    }

    fn key_stack(&self) -> Stack<'static> {
        views::BACKLOG_GRAPH
    }

    /// MOD-115 M2 §7: the list's motion moves the row cursor while the detail is focused, as
    /// its own `J`/`K` (`Act::GraphNextRow`/`GraphPrevRow`) do.
    fn step(&mut self, step: Step, _ctx: &Ctx<'_>) -> Handled {
        self.move_cursor(step.delta());
        Handled::Consumed
    }

    /// `J` / `K` and `PageDown` / `PageUp` move the cursor, `+` / `-` change the view depth,
    /// `Enter` re-roots (D5, D6).
    ///
    /// Resolved through `BACKLOG_GRAPH` (MOD-67 M5), whole chords, so a chord no act binds goes on
    /// to the tab. `Enter` is `backlog.graph.reroot`, which shadows the Backlog's replay miss
    /// (PA-2): unlike the Runs pane, it is consumed even when there is nothing to re-root, since
    /// here it means "re-root" and the miss would be the wrong answer from this pane.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(self.key_stack(), chord) {
            match act {
                Act::GraphNextRow => self.move_cursor(1),
                Act::GraphPrevRow => self.move_cursor(-1),
                Act::PanePageDown => self.move_cursor(PAGE),
                Act::PanePageUp => self.move_cursor(-PAGE),
                Act::GraphDeeper => {
                    self.set_depth(self.depth.saturating_add(1).min(Self::MAX_HOPS));
                }
                Act::GraphShallower => self.set_depth(self.depth.saturating_sub(1).max(1)),
                Act::GraphReroot => self.re_root(ctx),
                _ => continue, // list.*, pane.*_subtab, backlog.*, common.edit: the tab's
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// The `Links` reply for the selected item, or its refusal (L8). One for another root is
    /// dropped: the shell's staleness index already does that, this is the cheap guard on top.
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Links(graph) if Some(graph.root) == self.item => {
                self.graph = Some(graph.clone());
                self.rebuild();
                self.failed = None;
                self.cursor = 0;
            }
            // The refusal is on the status line already (`app/update.rs`); the pane says why it
            // is empty rather than claiming the item has no links.
            StoreReply::Failed { request, message } if *request == LINKS => {
                self.graph = None;
                self.rows.clear();
                self.failed = Some(message.clone());
                self.cursor = 0;
            }
            // MOD-147 M3 D3: the Backlog hands its `Items` reply on for the scope row it names.
            StoreReply::Items { scope_row, .. } => self.scope_row.clone_from(scope_row),
            _ => {}
        }
    }

    fn framing(&self, framing: Framing) {
        self.told.set(framing);
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            no_item(
                frame,
                area,
                self.key_stack(),
                CHOOSE_UNBOUND,
                self.told.list(),
                ctx,
            );
            return;
        }
        if self.graph.is_none() {
            match self.failed.as_deref() {
                Some(why) => unavailable(
                    frame,
                    area,
                    &format!("Links unavailable: {why}"),
                    self.key_stack(),
                    self.told.list(),
                    ctx,
                ),
                None => message(frame, area, "Loading links\u{2026}", ctx.theme),
            }
            return;
        }
        let rows = self.visible();
        if rows.len() <= 1 {
            let (what, next) = NO_LINKS;
            empty::render(frame, area, what, next, ctx.theme);
            return;
        }

        let [tree, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let cursor = self.cursor.min(rows.len() - 1);
        // The gutter takes the tree's first cell; the rows are fitted to the rest.
        let width = usize::from(tree.width.saturating_sub(1));
        let projects = self.workspace_projects(ctx);
        let rows: Vec<Row<'static>> = lines(&rows, projects, width, ctx.theme)
            .into_iter()
            .map(|line| Row {
                line,
                kind: RowKind::Item,
            })
            .collect();
        let offset = window(cursor, rows.len(), usize::from(tree.height));
        // One row per line, never wrapped: a wrapped row would break the row-to-cursor mapping.
        list::render(
            frame,
            tree,
            &rows,
            Some(cursor),
            offset,
            ctx.accent(),
            ctx.theme,
        );
        // The keys are the shell's hint row (MOD-67 M6 T2, `key_hint`); the depth stays here.
        let text = format!("depth {}/{}", self.depth, Self::MAX_HOPS);
        frame.render_widget(Paragraph::new(Line::styled(text, ctx.theme.dim)), footer);
    }

    /// `GRAPH_HINT` while the tree is drawn (MOD-67 M6 T2): the footer under it keeps only
    /// `depth d/3`. Nothing while there is no tree to move in.
    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        if self.item.is_none() || self.graph.is_none() || self.visible().len() <= 1 {
            return None;
        }
        let hint = keys.hint(views::BACKLOG_GRAPH, GRAPH_HINT);
        key_line(hint, theme)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{Scope, Status};
    use htui_core::store::{MemStore, ReadStore as _};
    use uuid::Uuid;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::store_worker::Origin;
    use crate::ui::cells::cell_width;
    use crate::ui::tabs::BacklogTab;
    use htui_core::model::{ArtifactLinkKind, DocumentId, LinkEdge};

    /// The detail pane's inner width at `DEFAULT_SIZE` (100x30): the 45-column detail pane less
    /// its border, as `backlog::tests::render`'s
    /// `the_detail_chips_window_around_the_active_one_at_100_columns` draws it.
    const PANE: usize = 43;

    /// What a row's content has of [`PANE`]: the `kit::list` gutter takes the first cell.
    const ROW: usize = PANE - 1;

    /// Everything a `Ctx` borrows, kept alive for the length of a test (the Runs pane's `Shell`).
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keys: Keys,
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
                keys: Keys::compiled().clone(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keys,
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

    /// The tree of `graph` at `depth`, read against it.
    fn rows(graph: &LinkGraph, depth: u8) -> Vec<GraphRow<'_>> {
        tree(graph, depth)
            .into_iter()
            .map(|row| row.resolve(graph))
            .collect()
    }

    impl<'a> GraphRow<'a> {
        /// The item this row names; a panic on a document's row.
        fn item(&self) -> &'a LinkNode {
            self.node().expect("an item's row")
        }
    }

    /// A tree as text: `{indent}{arrow} {kind} {slug:}KEY{ (*)}`, the root as its bare key; a
    /// document as `{indent}{arrow} {kind} {slug:}KEY doc_kind v{n}`.
    fn shape(rows: &[GraphRow<'_>]) -> Vec<String> {
        let root = rows.first().map(GraphRow::project_id);
        rows.iter()
            .map(|row| {
                let Some(link) = row.link else {
                    return row.item().key.clone();
                };
                let label = match row.target {
                    Target::Node(node) if Some(node.project_id) == root => node.key.clone(),
                    Target::Node(node) => format!("{}:{}", node.project_slug, node.key),
                    Target::Document(document) => {
                        let slug = root.and_then(|root| document_slug(document, root));
                        let slug = slug.map_or_else(String::new, |slug| format!("{slug}:"));
                        format!("{slug}{} v{}", document_key(document), document.version)
                    }
                };
                format!(
                    "{}{} {} {label}{}",
                    " ".repeat(2 * usize::from(row.level - 1)),
                    link.arrow.glyph(),
                    link.kind.as_str(),
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
        tab.rebuild();
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
            latest_run_status: None,
            latest_run_landing: None,
        }
    }

    fn edge(from: &LinkNode, kind: LinkKind, to: &LinkNode) -> LinkEdge {
        LinkEdge {
            from_item_id: from.item_id,
            to_item_id: to.item_id,
            kind,
        }
    }

    /// MOD-150 M3: `from --kind--> v{version}` of `doc_kind` in project `p` (as [`node`]'s),
    /// owned by item `owner` (its id `n + 1000`) or, with `None`, by the project; titled
    /// `{doc_kind} title`.
    fn document(
        n: u128,
        from: ItemId,
        kind: ArtifactLinkKind,
        owner: Option<&str>,
        doc_kind: &str,
        version: i32,
        latest_version: i32,
    ) -> LinkedDocument {
        LinkedDocument {
            from_item_id: from,
            kind,
            document_id: DocumentId::from_uuid(Uuid::from_u128(n)),
            project_id: ProjectId::from_uuid(Uuid::from_u128(100)),
            project_slug: "p".to_owned(),
            owner_item: owner.map(|_| ItemId::from_uuid(Uuid::from_u128(n + 1000))),
            owner_key: owner.map(str::to_owned),
            doc_kind: doc_kind.to_owned(),
            version,
            latest_version,
            title: format!("{doc_kind} title"),
        }
    }

    /// The lines of `graph` at `width`, as text.
    fn drawn(graph: &LinkGraph, shell: &Shell, width: usize) -> Vec<String> {
        lines(&rows(graph, 3), &shell.projects, width, &shell.theme)
            .iter()
            .map(text)
            .collect()
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
            .filter(|row| row.is_repeat() && row.item().item_id == ids::HTUI_FEAT_3)
            .count();
        assert_eq!(feat_3_repeats, 1, "{:#?}", shape(&rows));
        for node in &graph.nodes {
            let own = rows
                .iter()
                .filter(|row| !row.is_repeat() && row.item().item_id == node.item_id)
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
                    .any(|row| row.level == 1 && row.item().item_id == ids::HTUI_TOOL_1),
                "`TOOL-1 relates FEAT-1` is tombstoned: {:#?}",
                shape(&rows)
            );
            let own: Vec<String> = shape(&rows)
                .into_iter()
                .zip(&rows)
                .filter(|(_, row)| !row.is_repeat() && row.item().item_id == ids::HTUI_TOOL_1)
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
                (link.arrow, link.kind, row.item().key.as_str())
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
            documents: vec![],
        };
        let rows = rows(&graph, 3);
        assert_eq!(
            shape(&rows),
            ["R", "→ relates R (*)", "→ blocked_by X", "→ relates X (*)",]
        );
        assert_eq!(rows.len(), 1 + 3);
    }

    /// A ranked edge with no graph behind it, for `tree_parents`.
    fn ranked(from: usize, to: usize, edge: usize) -> Ranked {
        Ranked { from, to, edge }
    }

    #[test]
    fn a_tree_parent_is_the_first_ranked_kept_neighbour_one_level_up() {
        // 0 is the root; 1 and 2 are one hop out, 3 two hops out and adjacent to both.
        let depths = [0, 1, 1, 2];
        let edges = [
            ranked(0, 1, 0),
            ranked(0, 2, 1),
            ranked(1, 3, 2),
            ranked(3, 1, 3),
            ranked(2, 3, 4),
        ];
        assert_eq!(
            tree_parents(&depths, &edges),
            [None, Some((0, 0)), Some((0, 1)), Some((1, 2))],
            "3 hangs under 1, the first-ranked candidate, by the first edge between them"
        );

        // A same-level neighbour is not a parent, and a self-loop is nobody's.
        let depths = [0, 1, 1];
        let edges = [ranked(0, 0, 0), ranked(0, 2, 1), ranked(1, 2, 2)];
        assert_eq!(tree_parents(&depths, &edges), [None, None, Some((0, 1))]);

        // A node whose only neighbour one level up was dropped is dropped too.
        let depths = [0, 1, 2];
        let edges = [ranked(1, 2, 0)];
        assert_eq!(tree_parents(&depths, &edges), [None, None, None]);
    }

    #[test]
    fn a_duplicate_node_keeps_its_first_ranked_copy() {
        let r = node(1, "R", 0);
        let x = node(2, "X", 1);
        let y = node(3, "Y", 1);
        let mut x_again = x.clone();
        x_again.depth = 2;
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Relates, &x),
                edge(&r, LinkKind::Relates, &y),
            ],
            nodes: vec![x_again, y, r, x],
            documents: vec![],
        };
        assert_eq!(rank_nodes(&graph, 3), [2, 3, 1], "R, X at depth 1, Y");
        assert_eq!(shape(&rows(&graph, 3)), ["R", "→ relates X", "→ relates Y"]);
    }

    #[test]
    fn fit_label_gives_up_the_slug_before_the_key() {
        assert_eq!(fit_label("FEAT-1", None, 6, 6), "FEAT-1");
        assert_eq!(fit_label("FEAT-1", None, 4, 4), "FEA…");
        assert_eq!(fit_label("FEAT-1", None, 0, 0), "");
        assert_eq!(fit_label("FEAT-1", Some("agy"), 10, 10), "agy:FEAT-1");
        // Two columns of slug is the least worth drawing.
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 9, 9), "v…:FEAT-1");
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 8, 8), "FEAT-1");
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 3, 3), "FE…");
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 0, 0), "");
        // The status gives way before the key: the key takes the status's room, the slug does not.
        assert_eq!(fit_label("FEAT-1", None, 0, 6), "FEAT-1");
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 0, 13), "FEAT-1");
        assert_eq!(fit_label("FEAT-1", Some("vulkan"), 0, 4), "FEA…");
    }

    /// MOD-60 review L3: the label and the title are `cells::clip`'s, so a control character in a
    /// key, slug or title draws as one blank cell (D3) rather than as nothing, which would leave
    /// the row a cell short of what it was measured at.
    #[test]
    fn a_control_char_in_a_label_or_title_draws_as_a_blank_cell() {
        assert_eq!(fit_label("FEAT\u{1}1", None, 6, 6), "FEAT 1");
        assert_eq!(fit_label("FEAT-1", Some("a\u{1}y"), 10, 10), "a y:FEAT-1");
        assert_eq!(
            fit_label("FEAT-1", Some("v\u{1}lkan"), 10, 10),
            "v \u{2026}:FEAT-1"
        );
        assert_eq!(fit_label("F\u{7f}AT-1", None, 4, 4), "F A\u{2026}");
        let mut spans = vec![Span::raw("ab")];
        push_title(&mut spans, "a\r\nb title", 40, Style::new());
        assert_eq!(spans[2].content, "a b title");
    }

    /// MOD-60 review round 1: the root row's key is not a label (no `fit_label`), so it is
    /// flattened on its own; unflattened, `cut` measures its control at one cell that `ratatui`
    /// draws as nothing, and the status and title land a cell left of where they were measured.
    #[tokio::test]
    async fn a_control_char_in_the_root_key_draws_as_a_blank_cell() {
        let shell = Shell::platform().await;
        let graph = demo(ids::HTUI_FEAT_1).await;
        let root = rows(&graph, 2)[0];
        assert!(root.link.is_none(), "row 0 is the root");
        let mut node = root.item().clone();
        node.key = "FEAT\u{1}1".to_owned();
        let row = GraphRow {
            target: Target::Node(&node),
            ..root
        };
        let drawn = line(&row, node.project_id, true, None, ROW, &shell.theme);
        assert_eq!(text(&drawn), " FEAT 1 \u{25d0} running TUI scaffold");
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

    /// MOD-115 M2 §7: with the detail focused the list's motion moves the row cursor, a row as
    /// `J` and `K` do, or to either end.
    #[tokio::test]
    async fn a_step_moves_the_row_cursor_as_j_and_k_do() {
        use crate::ui::kit::scroll::Step;

        let shell = Shell::platform().await;
        let ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        assert_eq!(tab.step(Step::Down, &ctx), Handled::Consumed);
        assert_eq!(tab.cursor, 1, "a row, as `J`");
        assert_eq!(tab.step(Step::Bottom, &ctx), Handled::Consumed);
        assert_eq!(tab.cursor, 7, "the last row");
        assert_eq!(tab.step(Step::Up, &ctx), Handled::Consumed);
        assert_eq!(tab.cursor, 6, "a row up, as `K`");
        assert_eq!(tab.step(Step::Top, &ctx), Handled::Consumed);
        assert_eq!(tab.cursor, 0, "the root");
    }

    #[tokio::test]
    async fn a_depth_change_keeps_the_cursor_on_its_item() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        for _ in 0..6 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        assert_eq!(tab.visible()[tab.cursor].item().item_id, ids::AGY_FEAT_1);
        assert_eq!(tab.cursor, 6);
        tab.on_key(key(KeyCode::Char('+')), &mut ctx);
        assert_eq!(tab.cursor, 8);
        assert_eq!(tab.visible()[tab.cursor].item().item_id, ids::AGY_FEAT_1);
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
                for drawn in lines(&rows, &shell.projects, ROW, &shell.theme) {
                    assert!(
                        drawn.width() <= ROW,
                        "{:?} is {} columns",
                        text(&drawn),
                        drawn.width()
                    );
                }
            }
        }

        let graph = demo(ids::AGY_FIX_1).await;
        let drawn: Vec<String> = lines(&rows(&graph, 2), &shell.projects, ROW, &shell.theme)
            .iter()
            .map(text)
            .collect();
        assert_eq!(
            drawn,
            [
                " FIX-1 \u{25cb} open Session leak on cancel",
                " \u{2192} blocked by ANA-1 \u{25cf} done Prompt assembl\u{2026}",
                "   \u{2192} blocked by htui:ANA-1 \u{25cf} done Data mo\u{2026}",
                " \u{2192} origin vulkan-tutoria\u{2026}:FEAT-1 \u{25a0} stopped",
                " \u{2192} origin htui:TOOL-1 \u{26a0} needs you CI matr\u{2026}",
                "   \u{2192} blocked by htui:ANA-1 \u{25cf} done (*)",
            ]
        );
    }

    #[tokio::test]
    async fn feat_1_draws_the_worked_rows() {
        let shell = Shell::platform().await;
        let graph = demo(ids::HTUI_FEAT_1).await;
        let drawn: Vec<String> = lines(&rows(&graph, 2), &shell.projects, ROW, &shell.theme)
            .iter()
            .map(text)
            .collect();
        assert_eq!(
            drawn,
            [
                " FEAT-1 \u{25d0} running TUI scaffold",
                " \u{2192} origin     ANA-1 \u{25cf} done Data model, bo\u{2026}",
                "   \u{2190} blocked by agy:ANA-1 \u{25cf} done Prompt a\u{2026}",
                "   \u{2190} origin     FEAT-3 \u{25cc} queued (*)",
                "   \u{2190} blocked by TOOL-1 \u{26a0} needs you",
                " \u{2190} blocked by FEAT-2 \u{2298} blocked Agent driv\u{2026}",
                "   \u{2190} relates    agy:FEAT-1 \u{25cb} open ACP tra\u{2026}",
                " \u{2190} relates    FEAT-3 \u{25cc} queued Postgres st\u{2026}",
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
            row.is_repeat() && row.item().item_id == ids::HTUI_FEAT_3
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
        let fix_1 = rows(&graph, 2);
        let vulkan = fix_1
            .iter()
            .find(|row| row.item().item_id == ids::VULKAN_FEAT_1)
            .expect("vulkan FEAT-1 is one hop from agy FIX-1");
        let at = |width| {
            let drawn = line(
                vulkan,
                fix_1[0].item().project_id,
                false,
                None,
                width,
                &shell.theme,
            );
            assert!(drawn.width() <= width, "{:?} at {width}", text(&drawn));
            text(&drawn)
        };
        // 14 fixed cells and `origin` leave 8 of 28 for the label: one for the slug, so it goes
        // whole.
        assert_eq!(at(28), " \u{2192} origin FEAT-1 \u{25a0} stopped");
        // Then the status gives way, cut at the pane edge, and the key goes last.
        assert_eq!(at(24), " \u{2192} origin FEAT-1 \u{25a0} stopp");
        assert_eq!(at(22), " \u{2192} origin FEAT-1 \u{25a0} sto");
        assert_eq!(at(16), " \u{2192} origin FEAT-1");
        assert_eq!(at(15), " \u{2192} origin FEAT\u{2026}");

        // A label with no slug keeps its key over its status too.
        let graph = demo(ids::HTUI_FEAT_1).await;
        let feat_1 = rows(&graph, 2);
        let drawn = line(
            &feat_1[1],
            feat_1[0].item().project_id,
            true,
            None,
            17,
            &shell.theme,
        );
        assert_eq!(text(&drawn), " \u{2192} origin ANA-1 \u{25cf}");

        // An 80-column terminal's pane: the widest status gives way before the key does.
        let tool_1 = |width| text(&lines(&feat_1, &shell.projects, width, &shell.theme)[4]);
        assert_eq!(tool_1(30), "   \u{2190} blocked by TOOL-1 \u{26a0} needs");
        assert_eq!(tool_1(22), "   \u{2190} blocked by TOOL-1");
        assert_eq!(tool_1(21), "   \u{2190} blocked by TOOL\u{2026}");
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
            "\u{258c} FEAT-1 \u{25d0} running TUI scaffold"
        );
    }

    /// MOD-115 M4 RV2-M1: no item under the list's cursor is the kit's empty state, its next step
    /// the list's motion through `BACKLOG_GRAPH`.
    #[tokio::test]
    async fn no_item_is_an_empty_state_with_a_next_step() {
        let shell = Shell::platform().await;
        crate::ui::tabs::backlog::detail::assert_empty_state(
            &draw(&GraphTab::new(), &shell, 43, 6).join("\n"),
            "No item selected.",
            "j/k chooses one",
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
        // MOD-115 M4 RV2-M1: the failure is the kit's empty state, and its next step re-reads.
        crate::ui::tabs::backlog::detail::assert_empty_state(
            &draw(&tab, &shell, 43, 6).join("\n"),
            "Links unavailable: connection refused",
            "j/k away and back retries",
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

    /// L4: widths are terminal cells, not chars, so a wide title, slug or key never pushes a
    /// line past the pane.
    #[tokio::test]
    async fn a_wide_title_never_overflows_the_pane() {
        let shell = Shell::platform().await;
        let mut r = node(1, "R", 0);
        r.title = "图形视图".repeat(8);
        let mut x = node(2, "键-1", 1);
        x.title = "宽字符标题".repeat(8);
        x.project_id = ProjectId::from_uuid(Uuid::from_u128(101));
        x.project_slug = "项目".to_owned();
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![edge(&r, LinkKind::Relates, &x)],
            nodes: vec![r, x],
            documents: vec![],
        };
        let rows = rows(&graph, 3);
        for width in 0..=ROW {
            for drawn in lines(&rows, &shell.projects, width, &shell.theme) {
                assert!(
                    cell_width(&text(&drawn)) <= width,
                    "{:?} is {} cells at {width}",
                    text(&drawn),
                    cell_width(&text(&drawn))
                );
            }
        }
        let drawn: Vec<String> = lines(&rows, &shell.projects, ROW, &shell.theme)
            .iter()
            .map(text)
            .collect();
        assert_eq!(
            drawn,
            [
                " R \u{25cb} open 图形视图图形视图图形视图图形视\u{2026}",
                " \u{2192} relates    项目:键-1 \u{25cb} open 宽字符标题\u{2026}",
            ]
        );
    }

    /// L7: no width, down to nothing, panics or draws a line wider than the pane; MOD-150 M3 D12:
    /// nor does a document's row, an item's document from the root and an owner document of
    /// another project from the first node one hop out.
    #[tokio::test]
    async fn every_width_up_to_the_pane_fits_and_draws() {
        let shell = Shell::platform().await;
        for root in [ids::HTUI_FEAT_1, ids::AGY_FIX_1, ids::HTUI_ANA_1] {
            let mut graph = demo(root).await;
            let hop = graph
                .nodes
                .iter()
                .find(|node| node.depth == 1)
                .expect("every root has a neighbour")
                .item_id;
            let mut owner = document(91, hop, ArtifactLinkKind::Relates, None, "verdict", 1, 1);
            owner.project_id = ids::PROJECT_VULKAN;
            "vulkan-tutorials".clone_into(&mut owner.project_slug);
            graph.documents = vec![
                document(
                    90,
                    root,
                    ArtifactLinkKind::Implements,
                    Some("ANA-1"),
                    "research",
                    2,
                    3,
                ),
                owner,
            ];
            for depth in 1..=GraphTab::MAX_HOPS {
                let rows = rows(&graph, depth);
                for width in 0..=ROW {
                    let drawn = lines(&rows, &shell.projects, width, &shell.theme);
                    for (row, drawn) in rows.iter().zip(&drawn) {
                        let drawn = text(drawn);
                        assert!(cell_width(&drawn) <= width, "{drawn:?} at {width}");
                        // The key is the last thing to go: whole whenever the row's lead, its
                        // unpadded kind and the key itself fit.
                        let Some(link) = row.link else { continue };
                        let key = match row.target {
                            Target::Node(node) => node.key.clone(),
                            Target::Document(document) => document_key(document),
                        };
                        let key = key.as_str();
                        let needs = 1
                            + indent(row.level)
                            + 2
                            + cell_width(label::link_kind(link.kind))
                            + 1
                            + cell_width(key);
                        assert!(
                            width < needs || drawn.contains(key),
                            "{drawn:?} at {width} lost {key}"
                        );
                    }
                }
            }
            assert_eq!(
                rows(&graph, GraphTab::MAX_HOPS)
                    .iter()
                    .filter(|row| matches!(row.target, Target::Document(_)))
                    .count(),
                2,
                "both documents are drawn"
            );
            let mut tab = pane(graph);
            tab.depth = GraphTab::MAX_HOPS;
            tab.rebuild();
            tab.cursor = 1;
            for width in 0..=u16::try_from(PANE).expect("43 fits") {
                for height in [0, 1, 2, 5] {
                    for drawn in draw(&tab, &shell, width, height) {
                        assert!(cell_width(&drawn) <= usize::from(width), "{drawn:?}");
                    }
                }
            }
        }
    }

    /// L6: the kind column is padded for every row or for none, so it never goes ragged.
    #[tokio::test]
    async fn the_kind_column_is_padded_for_all_rows_or_none() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let x = node(2, "X", 1);
        let mut y = node(3, "Y", 1);
        y.project_id = ProjectId::from_uuid(Uuid::from_u128(101));
        y.project_slug = "long-project".to_owned();
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Relates, &x),
                edge(&r, LinkKind::Origin, &y),
            ],
            nodes: vec![r, x, y],
            documents: vec![],
        };
        let rows = rows(&graph, 3);
        let at = |width| -> Vec<String> {
            lines(&rows, &shell.projects, width, &shell.theme)
                .iter()
                .map(text)
                .collect()
        };
        // `long-project:Y` fits 35 columns padded, so both rows are.
        assert_eq!(
            at(35)[1..],
            [
                " \u{2192} relates    X \u{25cb} open X title",
                " \u{2192} origin     long-project:Y \u{25cb} open",
            ]
        );
        // At 34 it fits only unpadded, so neither row is, though `X` alone would be.
        assert_eq!(
            at(34)[1..],
            [
                " \u{2192} relates X \u{25cb} open X title",
                " \u{2192} origin long-project:Y \u{25cb} open",
            ]
        );
    }

    /// MOD-150 D1: the new kinds read as words in the kind column, `part of` padded to the ten
    /// cells `duplicates` and `implements` already fill, so the column keeps its width.
    #[tokio::test]
    async fn the_new_link_kinds_read_as_words_and_keep_the_column() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let p = node(2, "P", 1);
        let d = node(3, "D", 1);
        let i = node(4, "I", 1);
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Parent, &p),
                edge(&r, LinkKind::Duplicates, &d),
                edge(&r, LinkKind::Implements, &i),
            ],
            nodes: vec![r, p, d, i],
            documents: vec![],
        };
        assert_eq!(kind_width(), 10);
        let drawn: Vec<String> = lines(&rows(&graph, 3), &shell.projects, ROW, &shell.theme)
            .iter()
            .map(text)
            .collect();
        // Children sort by key bytes: D, I, P.
        assert_eq!(
            drawn[1..],
            [
                " \u{2192} duplicates D \u{25cb} open D title",
                " \u{2192} implements I \u{25cb} open I title",
                " \u{2192} part of    P \u{25cb} open P title",
            ]
        );
    }

    /// L7: a pane shorter than the tree scrolls it so the cursor row stays on screen, the footer
    /// kept below it (its keys the shell's hint row since MOD-67 M6 T2).
    #[tokio::test]
    async fn a_short_pane_scrolls_to_keep_the_cursor_visible() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        for _ in 0..4 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        // Three tree rows and the footer: the cursor sits mid-window.
        assert_eq!(
            draw(&tab, &shell, 43, 4),
            [
                "    \u{2190} origin     FEAT-3 \u{25cc} queued (*)",
                "\u{258c}   \u{2190} blocked by TOOL-1 \u{26a0} needs you",
                "  \u{2190} blocked by FEAT-2 \u{2298} blocked Agent driv\u{2026}",
                "depth 2/3",
            ]
        );
        for _ in 0..3 {
            tab.on_key(key(KeyCode::Char('J')), &mut ctx);
        }
        assert_eq!(
            draw(&tab, &shell, 43, 4),
            [
                "  \u{2190} blocked by FEAT-2 \u{2298} blocked Agent driv\u{2026}",
                "    \u{2190} relates    agy:FEAT-1 \u{25cb} open ACP tra\u{2026}",
                "\u{258c} \u{2190} relates    FEAT-3 \u{25cc} queued Postgres st\u{2026}",
                "depth 2/3",
            ],
            "the last row is the window's last"
        );
    }

    #[tokio::test]
    async fn an_out_of_workspace_node_is_dim() {
        let shell = Shell::platform().await;
        let graph = demo(ids::AGY_FIX_1).await;
        let rows = rows(&graph, 2);
        let vulkan = rows
            .iter()
            .find(|row| row.item().item_id == ids::VULKAN_FEAT_1)
            .expect("vulkan FEAT-1 is one hop from agy FIX-1");
        assert!(!in_workspace(vulkan.item().project_id, &shell.projects));
        let drawn = line(
            vulkan,
            rows[0].item().project_id,
            false,
            None,
            ROW,
            &shell.theme,
        );
        let label = drawn
            .spans
            .iter()
            .find(|span| span.content.contains("FEAT-1"))
            .expect("the label span");
        assert_eq!(label.style, shell.theme.dim);
    }

    /// FEAT-1's demo graph with its first neighbour (ANA-1) moved onto Platform's scope row as
    /// `WS-1`, and the scope row as the Backlog's `Items` reply names it (MOD-147 M3 D2).
    async fn with_a_workspace_node() -> (LinkGraph, StoreReply) {
        let mut graph = demo(ids::HTUI_FEAT_1).await;
        let node = graph
            .nodes
            .iter_mut()
            .find(|node| node.item_id == ids::HTUI_ANA_1)
            .expect("ANA-1 is one hop from FEAT-1");
        node.project_id = ids::PROJECT_SCOPE_PLATFORM;
        "@platform".clone_into(&mut node.project_slug);
        "WS-1".clone_into(&mut node.key);
        let reply = StoreReply::Items {
            items: Vec::new(),
            scope_row: Some(ProjectRef {
                project_id: ids::PROJECT_SCOPE_PLATFORM,
                slug: "@platform".to_owned(),
                name: "Platform".to_owned(),
                position: 0,
            }),
        };
        (graph, reply)
    }

    /// MOD-147 M3 D3: once the Backlog's `Items` reply names the scope row, a workspace item's
    /// node is in the workspace: drawn undimmed and re-rooted onto by `Enter`, not refused. Before
    /// it, the node reads as any other outside the members.
    #[tokio::test]
    async fn a_workspace_node_is_neither_dim_nor_refused() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let (graph, reply) = with_a_workspace_node().await;
        let mut tab = pane(graph);
        let label_style = |tab: &GraphTab| {
            let buffer = buffer(tab, &shell, 43, 10);
            (0..buffer.area.height)
                .find_map(|y| {
                    let row: String = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    let x = row.find("@platform:WS-1")?;
                    let x = u16::try_from(row[..x].chars().count()).expect("narrow");
                    let cell = &buffer[(x, y)];
                    Some((cell.fg, cell.modifier))
                })
                .expect("the WS-1 row is drawn")
        };
        let dim = (
            ratatui::style::Color::Indexed(244),
            ratatui::style::Modifier::empty(),
        );
        assert_eq!(label_style(&tab), dim, "members only: dim");
        tab.cursor = tab
            .visible()
            .iter()
            .position(|row| row.link.is_some() && row.item().item_id == ids::HTUI_ANA_1)
            .expect("WS-1 has a row of its own");
        tab.on_key(key(KeyCode::Enter), &mut ctx);
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Error(text)] if text == "@platform:WS-1 is outside this workspace"
            ),
            "members only: refused"
        );

        tab.on_reply(&reply, &mut ctx);
        assert_ne!(label_style(&tab), dim, "the scope row's: undimmed");
        assert!(
            label_style(&tab).1.contains(ratatui::style::Modifier::BOLD),
            "drawn as a key"
        );
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        match shell.emit.take().as_slice() {
            [Action::Reveal(RevealTarget::Item { id, key })] => {
                assert_eq!((*id, key.as_str()), (ids::HTUI_ANA_1, "WS-1"));
            }
            other => panic!("a reveal: {other:?}"),
        }
    }

    /// The pane drawn into a `width` x `height` test terminal, its buffer.
    fn buffer(tab: &GraphTab, shell: &Shell, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
            .expect("the test backend is constructible");
        term.draw(|frame| tab.render(frame, frame.area(), &shell.ctx()))
            .expect("the pane draws");
        term.backend().buffer().clone()
    }

    /// MOD-83, MOD-115 M4 D-1: an edge's kind reads as `label::link_kind`'s words and a node's
    /// status as its `kit::status` mark; no raw value reaches the pane.
    #[tokio::test]
    async fn kinds_and_statuses_read_as_words() {
        let shell = Shell::platform().await;
        let tab = pane(demo(ids::HTUI_FEAT_1).await);
        let rows = draw(&tab, &shell, 43, 10);
        assert_eq!(rows[0], "\u{258c} FEAT-1 \u{25d0} running TUI scaffold");
        assert!(
            rows.iter()
                .any(|row| row.starts_with("  \u{2190} blocked by FEAT-2 \u{2298} blocked ")),
            "{rows:#?}"
        );
        assert!(
            rows.contains(&"    \u{2190} blocked by TOOL-1 \u{26a0} needs you".to_owned()),
            "{rows:#?}"
        );
        let text = rows.join("\n");
        for raw in ["blocked_by", "in_progress", "awaiting"] {
            assert!(!text.contains(raw), "{raw}: {text}");
        }
    }

    /// MOD-85: the cursor row is the kit's: the gutter in the pane's accent, the row in the
    /// selection band to the pane's edge; legible under monochrome, where the band is reversed.
    #[tokio::test]
    async fn the_cursor_row_is_the_kit_band_behind_the_gutter() {
        let mut shell = Shell::platform().await;
        let mut tab = pane(demo(ids::HTUI_FEAT_1).await);
        tab.cursor = 1;
        let accent = shell.ctx().accent();
        let buf = buffer(&tab, &shell, 43, 10);
        assert_eq!(
            buf[(0, 0)].symbol(),
            " ",
            "the root row is not the cursor's"
        );
        assert_eq!(buf[(0, 1)].symbol(), "\u{258c}");
        assert_eq!(
            buf[(0, 1)].fg,
            accent.fg.unwrap_or_default(),
            "the gutter's accent"
        );
        let band = shell.theme.selected.bg.unwrap_or_default();
        for x in 1..43 {
            assert_eq!(buf[(x, 1)].bg, band, "column {x} of the cursor row");
        }
        assert_ne!(buf[(5, 0)].bg, band, "the root row is not banded");

        shell.theme = Theme::monochrome();
        let buf = buffer(&tab, &shell, 43, 10);
        for x in 1..43 {
            assert!(
                buf[(x, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED),
                "column {x} of the cursor row under monochrome"
            );
        }
        assert!(
            !buf[(5, 0)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
    }

    /// The kind column is as wide as the widest of `label::link_kind`'s words, and no raw kind is
    /// wider than its word's column (D-1).
    #[test]
    fn the_kind_column_is_the_widest_label() {
        assert_eq!(
            kind_width(),
            10,
            "`blocked by`, `supersedes`, `duplicates` and `implements`"
        );
        for kind in LinkKind::ALL {
            assert!(
                cell_width(label::link_kind(*kind)) <= kind_width(),
                "{kind:?}"
            );
        }
    }

    /// MOD-150 M3 D12: a document's row reads its kind, `KEY doc_kind` and the pinned version,
    /// the latest beside it only when that is newer, then its title.
    #[tokio::test]
    async fn a_document_link_reads_kind_owner_and_version() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let mut stale = document(
            10,
            r.item_id,
            ArtifactLinkKind::Implements,
            Some("ANA-1"),
            "verdict",
            2,
            4,
        );
        "Verdict: keep Postgres".clone_into(&mut stale.title);
        let latest = document(
            11,
            r.item_id,
            ArtifactLinkKind::Relates,
            Some("ANA-2"),
            "plan",
            3,
            3,
        );
        let mut graph = LinkGraph {
            root: r.item_id,
            nodes: vec![r],
            edges: vec![],
            documents: vec![latest, stale],
        };
        // As every backend answers it: the tab draws an item's documents in this order.
        graph.sort_documents();
        assert_eq!(
            drawn(&graph, &shell, 80)[1..],
            [
                " \u{2192} implements ANA-1 verdict v2 (latest v4) Verdict: keep Postgres",
                " \u{2192} relates    ANA-2 plan v3 plan title",
            ]
        );
    }

    /// MOD-150 M3 D12: an owner document is labelled by its project's slug, even the root's; an
    /// item's document of another project carries the slug as an item does.
    #[tokio::test]
    async fn an_owner_document_reads_its_slug() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let own = document(
            10,
            r.item_id,
            ArtifactLinkKind::Origin,
            None,
            "decisions",
            1,
            1,
        );
        let mut elsewhere = document(
            11,
            r.item_id,
            ArtifactLinkKind::Relates,
            Some("ANA-1"),
            "verdict",
            1,
            1,
        );
        elsewhere.project_id = ProjectId::from_uuid(Uuid::from_u128(101));
        "lib".clone_into(&mut elsewhere.project_slug);
        let mut lib = document(
            12,
            r.item_id,
            ArtifactLinkKind::Relates,
            None,
            "verdict",
            1,
            2,
        );
        lib.project_id = ProjectId::from_uuid(Uuid::from_u128(101));
        "lib".clone_into(&mut lib.project_slug);
        let mut graph = LinkGraph {
            root: r.item_id,
            nodes: vec![r],
            edges: vec![],
            documents: vec![lib, elsewhere, own],
        };
        graph.sort_documents();
        assert_eq!(
            drawn(&graph, &shell, 80)[1..],
            [
                " \u{2192} origin     p decisions v1 decisions title",
                " \u{2192} relates    lib verdict v1 (latest v2) verdict title",
                " \u{2192} relates    lib:ANA-1 verdict v1 verdict title",
            ]
        );
    }

    /// MOD-150 M3 D12: a document hangs under its item's own row, after its children, and never
    /// under a `(*)` repeat; a document of a node the walk did not keep is not drawn.
    #[test]
    fn a_document_under_a_repeated_node_is_not_drawn_twice() {
        let r = node(1, "R", 0);
        let x = node(2, "X", 1);
        let y = node(3, "Y", 1);
        let z = node(4, "Z", 2);
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![
                edge(&r, LinkKind::Relates, &x),
                edge(&r, LinkKind::Relates, &y),
                edge(&x, LinkKind::Relates, &y),
                edge(&x, LinkKind::Relates, &z),
            ],
            documents: vec![
                document(
                    10,
                    y.item_id,
                    ArtifactLinkKind::Implements,
                    Some("ANA-1"),
                    "verdict",
                    1,
                    1,
                ),
                document(
                    11,
                    x.item_id,
                    ArtifactLinkKind::Origin,
                    Some("ANA-2"),
                    "prd",
                    2,
                    2,
                ),
                document(
                    12,
                    r.item_id,
                    ArtifactLinkKind::Relates,
                    Some("ANA-3"),
                    "plan",
                    1,
                    1,
                ),
                document(
                    13,
                    ItemId::from_uuid(Uuid::from_u128(99)),
                    ArtifactLinkKind::Relates,
                    None,
                    "verdict",
                    1,
                    1,
                ),
            ],
            nodes: vec![r, x, y, z],
        };
        assert_eq!(
            shape(&rows(&graph, 3)),
            [
                "R",
                "\u{2192} relates X",
                "  \u{2192} relates Y (*)",
                "  \u{2192} relates Z",
                "  \u{2192} origin ANA-2 prd v2",
                "\u{2192} relates Y",
                "  \u{2192} implements ANA-1 verdict v1",
                "\u{2192} relates ANA-3 plan v1",
            ]
        );
        // At depth 1 Z is cut, and X's document still follows X's children.
        assert_eq!(
            shape(&rows(&graph, 1)),
            [
                "R",
                "\u{2192} relates X",
                "  \u{2192} relates Y (*)",
                "  \u{2192} origin ANA-2 prd v2",
                "\u{2192} relates Y",
                "  \u{2192} implements ANA-1 verdict v1",
                "\u{2192} relates ANA-3 plan v1",
            ]
        );
    }

    /// MOD-150 M3 D12: a document's kind is padded with the items' to the same ten cells, and a
    /// document too long to fit padded unpads every row (L6).
    #[tokio::test]
    async fn the_kind_column_keeps_its_width_with_documents() {
        let shell = Shell::platform().await;
        let r = node(1, "R", 0);
        let x = node(2, "X", 1);
        let graph = LinkGraph {
            root: r.item_id,
            edges: vec![edge(&r, LinkKind::Origin, &x)],
            documents: vec![document(
                10,
                r.item_id,
                ArtifactLinkKind::Relates,
                Some("ANA-1"),
                "verdict",
                1,
                1,
            )],
            nodes: vec![r, x],
        };
        assert_eq!(kind_width(), 10);
        assert_eq!(
            drawn(&graph, &shell, ROW)[1..],
            [
                " \u{2192} origin     X \u{25cb} open X title",
                " \u{2192} relates    ANA-1 verdict v1 verdict ti\u{2026}",
            ]
        );
        // 30 cells hold the padded document row, both titles dropped; 29 only the unpadded one.
        assert_eq!(
            drawn(&graph, &shell, 30)[1..],
            [
                " \u{2192} origin     X \u{25cb} open",
                " \u{2192} relates    ANA-1 verdict v1",
            ]
        );
        assert_eq!(
            drawn(&graph, &shell, 29)[1..],
            [
                " \u{2192} origin X \u{25cb} open X title",
                " \u{2192} relates ANA-1 verdict v1",
            ]
        );
    }

    /// FEAT-1's demo graph with `document` linked from its root, and the row it draws on.
    async fn feat_1_with(document: LinkedDocument) -> (GraphTab, usize) {
        let mut graph = demo(ids::HTUI_FEAT_1).await;
        graph.documents = vec![document];
        let tab = pane(graph);
        let at = tab
            .visible()
            .iter()
            .position(|row| matches!(row.target, Target::Document(_)))
            .expect("the document has a row");
        (tab, at)
    }

    /// MOD-150 M3 D12: `Enter` on an item's document reveals the item that owns it, and is
    /// refused, as an item's row is, when that item is outside the workspace.
    #[tokio::test]
    async fn enter_on_an_item_document_reveals_its_owner() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let mut ana_1 = document(
            10,
            ids::HTUI_FEAT_1,
            ArtifactLinkKind::Implements,
            Some("ANA-1"),
            "research",
            2,
            3,
        );
        ana_1.owner_item = Some(ids::HTUI_ANA_1);
        ana_1.project_id = ids::PROJECT_HTUI;
        "htui".clone_into(&mut ana_1.project_slug);
        let (mut tab, at) = feat_1_with(ana_1).await;
        assert_eq!(at, 8, "after every row the root's children draw");
        tab.cursor = at;
        assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
        match shell.emit.take().as_slice() {
            [Action::Reveal(RevealTarget::Item { id, key })] => {
                assert_eq!((*id, key.as_str()), (ids::HTUI_ANA_1, "ANA-1"));
            }
            other => panic!("a reveal: {other:?}"),
        }

        let mut vulkan = document(
            11,
            ids::HTUI_FEAT_1,
            ArtifactLinkKind::Relates,
            Some("FEAT-1"),
            "prd",
            1,
            1,
        );
        vulkan.owner_item = Some(ids::VULKAN_FEAT_1);
        vulkan.project_id = ids::PROJECT_VULKAN;
        "vulkan-tutorials".clone_into(&mut vulkan.project_slug);
        let (mut tab, at) = feat_1_with(vulkan).await;
        tab.cursor = at;
        tab.on_key(key(KeyCode::Enter), &mut ctx);
        assert!(
            matches!(
                shell.emit.take().as_slice(),
                [Action::Error(text)] if text == "vulkan-tutorials:FEAT-1 is outside this workspace"
            ),
            "outside the workspace: refused"
        );
        let label = lines(&tab.visible(), &shell.projects, ROW, &shell.theme)[at]
            .spans
            .iter()
            .find(|span| span.content.contains("FEAT-1 prd"))
            .expect("the label span")
            .style;
        assert_eq!(label, shell.theme.dim, "and dim");
    }

    /// MOD-150 M3 D12: an owner document has no item to reveal; `Enter` says where it opens,
    /// naming a scope row's owner as its workspace.
    #[tokio::test]
    async fn enter_on_an_owner_document_says_where_it_opens() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        for (project, slug, said) in [
            (
                ids::PROJECT_HTUI,
                "htui",
                "htui verdict is project htui's own document; open it in Settings",
            ),
            (
                ids::PROJECT_SCOPE_PLATFORM,
                "@platform",
                "@platform verdict is workspace platform's own document; open it in Settings",
            ),
        ] {
            let mut verdict = document(
                10,
                ids::HTUI_FEAT_1,
                ArtifactLinkKind::Relates,
                None,
                "verdict",
                1,
                1,
            );
            verdict.project_id = project;
            slug.clone_into(&mut verdict.project_slug);
            let (mut tab, at) = feat_1_with(verdict).await;
            tab.cursor = at;
            assert_eq!(tab.on_key(key(KeyCode::Enter), &mut ctx), Handled::Consumed);
            let emitted = shell.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Error(text)] if text == said),
                "no reveal: {emitted:?}"
            );
        }
    }

    /// MOD-150 M3 D12: a depth change keeps the cursor on a document whose item stays expanded.
    #[tokio::test]
    async fn a_depth_change_keeps_the_cursor_on_its_document() {
        let shell = Shell::platform().await;
        let mut ctx = shell.ctx();
        let (mut tab, at) = feat_1_with(document(
            10,
            ids::HTUI_FEAT_1,
            ArtifactLinkKind::Relates,
            None,
            "verdict",
            1,
            1,
        ))
        .await;
        tab.cursor = at;
        tab.on_key(key(KeyCode::Char('+')), &mut ctx);
        assert!(
            matches!(tab.visible()[tab.cursor].target, Target::Document(_)),
            "{:#?}",
            shape(&tab.visible())
        );
        assert_eq!(tab.cursor, tab.visible().len() - 1);
    }
}
