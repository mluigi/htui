//! MOD-141 D10: the links list - the selected step's edges out and in, one row each, with a
//! cursor. Key-agnostic.

use htui_core::model::{EdgeId, EdgeKind, GraphBody, PhaseId, StepGraphEdge};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{relation_word, step_name};
use crate::ui::Theme;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::list::{GUTTER, ellipsize, selection_band};

/// One link of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkRow {
    /// The link.
    pub(crate) edge: EdgeId,
    /// Whether it leaves the list's step (`→`) rather than enters it (`←`).
    pub(crate) outgoing: bool,
    /// The row in words: `→ review · next`.
    pub(crate) text: String,
}

/// The links list of one step, with a cursor over its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Links {
    /// The step whose links it lists.
    node: PhaseId,
    /// Index into [`rows`](Self::rows).
    cursor: usize,
}

impl Links {
    /// The list of `node`, cursor on its first row.
    pub(crate) const fn new(node: PhaseId) -> Self {
        Self { node, cursor: 0 }
    }

    /// The list of `edge`'s source with the cursor on `edge` (a click on an edge); `None` when
    /// the body has no such edge.
    pub(crate) fn on_edge(body: &GraphBody, edge: EdgeId) -> Option<Self> {
        let from = body.edges.iter().find(|each| each.id == edge)?.from;
        let mut links = Self::new(from);
        links.cursor = links.rows(body).iter().position(|row| row.edge == edge)?;
        Some(links)
    }

    /// The step whose links it lists.
    pub(crate) const fn node(&self) -> PhaseId {
        self.node
    }

    /// Out-edges in body order, then in-edges in body order. Text: `→ {to} · {relation}` or
    /// `← {from} · {relation}`, then ` · {predicate as compact JSON}` on an `if` and
    /// ` · limit {n}` when the link has a loop limit.
    pub(crate) fn rows(&self, body: &GraphBody) -> Vec<LinkRow> {
        let out = body
            .edges
            .iter()
            .filter(|edge| edge.from == self.node)
            .map(|edge| row(body, edge, true));
        let into = body
            .edges
            .iter()
            .filter(|edge| edge.to == self.node)
            .map(|edge| row(body, edge, false));
        out.chain(into).collect()
    }

    /// The cursor one row down, stopping on the last.
    pub(crate) fn down(&mut self, body: &GraphBody) {
        if self.cursor + 1 < self.rows(body).len() {
            self.cursor += 1;
        }
    }

    /// The cursor one row up, stopping on the first.
    pub(crate) const fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// The cursor kept on a row after the body changed (a link removed).
    pub(crate) fn clamp(&mut self, body: &GraphBody) {
        self.cursor = self.cursor.min(self.rows(body).len().saturating_sub(1));
    }

    /// The link under the cursor; `None` when the step has none.
    pub(crate) fn selected(&self, body: &GraphBody) -> Option<EdgeId> {
        self.rows(body).get(self.cursor).map(|row| row.edge)
    }

    /// The list `width` cells wide: one row per link, the cursor's in the selection band behind
    /// the accent gutter; with no link, the empty state.
    pub(crate) fn lines(
        &self,
        body: &GraphBody,
        width: u16,
        accent: Style,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let rows = self.rows(body);
        if rows.is_empty() {
            return empty::lines(NO_LINKS, NextStep(CONNECT_NEXT), theme);
        }
        let content = width.saturating_sub(1);
        rows.into_iter()
            .enumerate()
            .map(|(at, row)| {
                let line = Line::from(row.text);
                if at == self.cursor {
                    let mut spans = vec![Span::styled(GUTTER, accent)];
                    spans.extend(selection_band(line, content, theme).spans);
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(" ")];
                    spans.extend(ellipsize(line, content).spans);
                    Line::from(spans)
                }
            })
            .collect()
    }
}

/// The empty state's words.
const NO_LINKS: &str = "no links";

/// The empty state's next step, key-agnostic.
const CONNECT_NEXT: &str = "connect this step to another to add one";

/// `edge` as a row of the list of its source (`outgoing`) or its target.
fn row(body: &GraphBody, edge: &StepGraphEdge, outgoing: bool) -> LinkRow {
    let (arrow, other) = if outgoing {
        ("\u{2192}", edge.to)
    } else {
        ("\u{2190}", edge.from)
    };
    let name = step_name(body, other).unwrap_or("a missing step");
    let mut text = format!("{arrow} {name} \u{b7} {}", relation_word(edge.kind));
    if edge.kind == EdgeKind::If
        && let Some(predicate) = &edge.predicate
    {
        text.push_str(&format!(" \u{b7} {}", predicate.0));
    }
    if let Some(limit) = edge.loop_limit {
        text.push_str(&format!(" \u{b7} limit {limit}"));
    }
    LinkRow {
        edge: edge.id,
        outgoing,
        text,
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::store::{MemStore, WriteStore};
    use ratatui::style::Color;

    use super::*;

    fn accent() -> Style {
        Style::new().fg(Color::Magenta)
    }

    /// The lines as text, trailing blanks dropped.
    fn text(lines: &[Line<'_>]) -> String {
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

    /// The seeded feature graph: prd → plan → implement → review, review ⇢ implement.
    async fn feature() -> GraphBody {
        MemStore::demo()
            .graph_body(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("a memory read")
            .expect("the feature graph")
    }

    fn step(body: &GraphBody, name: &str) -> PhaseId {
        body.nodes
            .iter()
            .find(|node| node.phase.name == name)
            .unwrap_or_else(|| panic!("no step {name}"))
            .phase
            .id
    }

    fn texts(links: &Links, body: &GraphBody) -> Vec<String> {
        links.rows(body).into_iter().map(|row| row.text).collect()
    }

    #[tokio::test]
    async fn rows_list_out_then_in_edges_in_body_order() {
        let mut body = feature().await;
        let review = step(&body, "review");
        let links = Links::new(review);
        assert_eq!(links.node(), review);
        assert_eq!(
            texts(&links, &body),
            [
                "\u{2192} implement \u{b7} changes requested \u{b7} limit 1",
                "\u{2190} implement \u{b7} next",
            ]
        );
        let rows = links.rows(&body);
        assert!(rows[0].outgoing);
        assert!(!rows[1].outgoing);

        // An `if` row carries its predicate as compact JSON.
        let prd = step(&body, "prd");
        let edge = body
            .edges
            .iter_mut()
            .find(|edge| edge.from == prd)
            .expect("prd → plan");
        edge.kind = EdgeKind::If;
        edge.predicate = Some(htui_core::model::EdgePredicate(
            serde_json::json!({"verify": "passed"}),
        ));
        assert_eq!(
            texts(&Links::new(prd), &body),
            ["\u{2192} plan \u{b7} if \u{b7} {\"verify\":\"passed\"}"]
        );
    }

    #[tokio::test]
    async fn the_cursor_moves_and_clamps_after_a_removal() {
        let mut body = feature().await;
        let implement = step(&body, "implement");
        let mut links = Links::new(implement);
        let rows = links.rows(&body);
        assert_eq!(rows.len(), 3, "→ review, ← plan, ← review");
        assert_eq!(links.selected(&body), Some(rows[0].edge));
        links.up();
        assert_eq!(
            links.selected(&body),
            Some(rows[0].edge),
            "stops on the first"
        );
        links.down(&body);
        links.down(&body);
        links.down(&body);
        assert_eq!(
            links.selected(&body),
            Some(rows[2].edge),
            "stops on the last"
        );

        // The last row's link goes: the cursor falls back onto the new last row.
        body.edges.retain(|edge| edge.id != rows[2].edge);
        links.clamp(&body);
        assert_eq!(links.selected(&body), Some(rows[1].edge));

        // Every link goes: nothing is selected, and the list says so.
        body.edges.clear();
        links.clamp(&body);
        assert_eq!(links.selected(&body), None);
        links.down(&body);
        assert_eq!(links.selected(&body), None);
        let lines = links.lines(&body, 56, accent(), &Theme::default());
        assert!(text(&lines).starts_with("no links"), "{}", text(&lines));
    }

    #[tokio::test]
    async fn on_edge_selects_that_edge() {
        let body = feature().await;
        let implement = step(&body, "implement");
        let review = step(&body, "review");
        let back = body
            .edges
            .iter()
            .find(|edge| edge.from == review && edge.to == implement)
            .expect("review ⇢ implement")
            .id;
        let links = Links::on_edge(&body, back).expect("a known edge");
        assert_eq!(links.node(), review, "the list of the edge's source");
        assert_eq!(links.selected(&body), Some(back));

        let forward = body
            .edges
            .iter()
            .find(|edge| edge.from == implement && edge.to == review)
            .expect("implement → review")
            .id;
        let links = Links::on_edge(&body, forward).expect("a known edge");
        assert_eq!(links.node(), implement);
        assert_eq!(links.selected(&body), Some(forward));

        assert_eq!(Links::on_edge(&body, EdgeId::new()), None);
    }

    #[tokio::test]
    async fn links_review() {
        let body = feature().await;
        let mut links = Links::new(step(&body, "review"));
        links.down(&body);
        let lines = links.lines(&body, 56, accent(), &Theme::default());
        insta::assert_snapshot!("links_review", text(&lines));
    }
}
