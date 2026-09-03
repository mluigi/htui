//! The Graph sub-tab: the item's links at one hop, as a flat table.
//!
//! MOD-14 replaces the table with a navigable traversal; re-rooting is already expressible as
//! `ctx.request(Links { id: other, hops: n })`, so nothing outside this file has to change for it
//! (blueprint §F).

use htui_core::model::{ItemId, LinkEdge, LinkGraph, LinkNode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::KeyEvent;

/// An edge that leaves the selected item.
const OUT: &str = "\u{2192}";

/// An edge that arrives at the selected item.
const IN: &str = "\u{2190}";

/// The one-hop neighbourhood of the selected item: direction, kind, and the other item.
#[derive(Debug, Default)]
pub struct GraphTab {
    /// The `Links` reply for the selected item.
    graph: Option<LinkGraph>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// First visible edge.
    scroll: Scroll,
}

impl GraphTab {
    /// Identity of the Graph sub-tab.
    pub const ID: DetailId = DetailId("graph");

    /// A sub-tab with no graph yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The edges incident to the root, with the arrow that shows which way they point.
    ///
    /// A traversal also returns edges *between* neighbours; they have no "other item" relative to
    /// the root, so a one-hop table drops them.
    fn edges(&self) -> Vec<(&'static str, &LinkEdge, &LinkNode)> {
        let Some(graph) = self.graph.as_ref() else {
            return Vec::new();
        };
        graph
            .edges
            .iter()
            .filter_map(|edge| {
                let (direction, other) = if edge.from_item_id == graph.root {
                    (OUT, edge.to_item_id)
                } else if edge.to_item_id == graph.root {
                    (IN, edge.from_item_id)
                } else {
                    return None;
                };
                Some((direction, edge, graph.node(other)?))
            })
            .collect()
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
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.edges().len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Links(graph) = reply {
            self.graph = Some(graph.clone());
            self.scroll.reset();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        let edges = self.edges();
        if edges.is_empty() {
            message(frame, area, "No links at one hop.", ctx.theme);
            return;
        }

        let rows = edges
            .into_iter()
            .skip(self.scroll.skip())
            .map(|(direction, edge, node)| {
                Row::new(vec![
                    Cell::from(Line::styled(direction, ctx.theme.dim)),
                    Cell::from(Line::styled(edge.kind.as_str(), ctx.theme.base)),
                    Cell::from(Line::styled(node.key.clone(), ctx.theme.accent)),
                    Cell::from(Line::styled(
                        node.status.as_str(),
                        ctx.theme.status_style(node.status),
                    )),
                ])
            })
            .collect::<Vec<_>>();

        let header = Row::new(vec![
            Cell::from(""),
            Cell::from("link"),
            Cell::from("item"),
            Cell::from("status"),
        ])
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Length(17),
            ],
        )
        .header(header)
        .column_spacing(1);
        frame.render_widget(table, area);
    }
}
