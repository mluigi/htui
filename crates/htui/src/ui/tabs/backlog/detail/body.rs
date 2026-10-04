//! The Body sub-tab: the item's head fields and its Markdown body.
//!
//! The body is hard-wrapped Markdown, so it is reflowed before it is wrapped (MOD-84):
//! `ui::markdown` classes its lines (fenced and indented code, blank lines, list items, headings,
//! rules, table rows, quotes and text), joins the soft-wrapped lines of a paragraph or list item,
//! and wraps the result at the pane's width with a list item's rows hanging under its text. Blank
//! lines, markers and structural lines are kept as written, a hard break keeps its row, and
//! inline code is drawn in the accent style without its backticks, except in code and table rows,
//! which keep theirs. Nothing else is rendered: no emphasis, links or heading styles. The scroll
//! clamps against the rows on screen at the last render's width, as the Notes pane does (MOD-13
//! review L1).

use std::borrow::Cow;
use std::cell::Cell;

use htui_core::model::{Item, ItemId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::markdown;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::KeyEvent;

/// How many lines the head block takes: title, key line, meta line, blank.
const HEAD: u16 = 4;

/// Title, key, kind, status, tags, priority, version, and the body reflowed and wrapped at the
/// pane's width (see the module doc).
#[derive(Debug, Default)]
pub struct BodyTab {
    /// The `Item` reply for the selected item.
    item: Option<Item>,
    /// Scroll of the body paragraph, not of the head block.
    scroll: Scroll,
    /// The body area's width at the last render, for the scroll clamp (MOD-84 D7); 0 before the
    /// first, which counts the body unwrapped. `Cell` because `render` is `&self`.
    drawn: Cell<u16>,
}

impl BodyTab {
    /// Identity of the Body sub-tab.
    pub const ID: DetailId = DetailId("body");

    /// A sub-tab with no item yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The body's rows as drawn at the last render's width: what the scroll clamps against
    /// (MOD-84 D7).
    fn len(&self) -> usize {
        self.item
            .as_ref()
            .map_or(0, |item| markdown::row_count(&item.body, self.drawn.get()))
    }
}

impl DetailTab for BodyTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Body"
    }

    fn on_item_change(&mut self, _item: Option<ItemId>) {
        self.item = None;
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Item(item) = reply {
            self.item = (**item).clone();
            self.scroll.reset();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let Some(item) = self.item.as_ref() else {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        };

        let tags: Cow<'_, str> = if item.required_tags.is_empty() {
            Cow::Borrowed("no tags")
        } else {
            Cow::Owned(item.required_tags.join(", "))
        };
        let head = Text::from(vec![
            Line::styled(item.title.as_str(), ctx.theme.title),
            Line::from(vec![
                Span::styled(item.key.as_str(), ctx.theme.key),
                Span::raw("  "),
                Span::styled(item.key_prefix.as_str(), ctx.theme.dim),
                Span::raw("  "),
                Span::styled(item.status.as_str(), ctx.theme.status_style(item.status)),
            ]),
            Line::styled(
                format!(
                    "{tags} \u{b7} priority {} \u{b7} version {}",
                    item.priority, item.version
                ),
                ctx.theme.dim,
            ),
            Line::raw(""),
        ]);

        let [head_area, body_area] =
            Layout::vertical([Constraint::Length(HEAD), Constraint::Min(0)]).areas(area);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), head_area);

        self.drawn.set(body_area.width);
        if item.body.is_empty() {
            message(frame, body_area, "No body for this item.", ctx.theme);
            return;
        }
        let lines = markdown::lines(&item.body, body_area.width, ctx.theme);
        // A pane that widened since the last key has fewer rows than the offset assumed.
        let last = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).scroll((self.scroll.offset().min(last), 0)),
            body_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use htui_core::fixtures::ids;
    use htui_core::store::{MemStore, ReadStore as _};

    use super::*;
    use crate::ui::tabs::backlog::detail::compose::bench::{Shell, drawn, key};

    /// A pane on FEAT-1 that has landed the item.
    async fn pane(shell: &Shell) -> BodyTab {
        let mut pane = BodyTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let item = MemStore::demo()
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("the item");
        pane.on_reply(&StoreReply::Item(Box::new(item)), &mut shell.ctx());
        pane
    }

    /// The pane drawn at `width` x `height`, one string per row, trailing spaces trimmed.
    fn rows(pane: &BodyTab, shell: &Shell, width: u16, height: u16) -> Vec<String> {
        drawn(width, height, |frame, area| {
            pane.render(frame, area, &shell.ctx());
        })
        .lines()
        .map(|row| row.trim_end().to_owned())
        .collect()
    }

    /// MOD-84: at the detail pane's width the body reads as paragraphs and list items, with no
    /// ragged row where the source broke its lines (A-2) and no backtick of inline code.
    #[tokio::test]
    async fn the_body_reads_as_paragraphs_at_the_detail_width() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let rows = rows(&pane, &shell, 43, 30);
        assert_eq!(
            rows[..4],
            [
                "TUI scaffold",
                "FEAT-1  FEAT  in_progress",
                "rust \u{b7} priority 2 \u{b7} version 1",
                "",
            ],
            "{rows:#?}"
        );
        assert_eq!(
            rows[4..],
            [
                "Stand up the terminal application: a",
                "workspace-scoped shell with a tab strip, a",
                "top bar and a backlog tab, all reading",
                "through the store seam.",
                "",
                "Shape",
                "- Two crates: htui-core holds the domain",
                "  model and the store traits, htui holds",
                "  the terminal application. Nothing in the",
                "  view layer holds a store handle.",
                "- A store worker task owns the backend. The",
                "  UI sends a request, the worker replies,",
                "  the event loop folds the reply into",
                "  state. Three select arms: terminal",
                "  events, store replies, a tick.",
                "- Tabs, detail sub-tabs and overlays are",
                "  trait objects in registries, so a later",
                "  module adds a screen by registering it",
                "  rather than by editing the loop.",
                "",
                "Scope",
                "- Backlog list grouped by project and key",
                "  prefix, with the five detail sub-tabs:",
                "  body, runs, graph, documents, notes.",
                "- A workspace switcher overlay, opened with",
                "  w.",
            ],
            "{rows:#?}"
        );
        assert!(
            rows.iter()
                .any(|row| row == "top bar and a backlog tab, all reading"),
            "{rows:#?}"
        );
        assert!(!rows.iter().any(|row| row.contains('`')), "{rows:#?}");
    }

    /// MOD-84 D7, as the Notes pane's review L1: the clamp counts the rows on screen at the last
    /// render's width, so `PageDown` reaches the body's last row in a narrow pane.
    #[tokio::test]
    async fn page_down_reaches_the_last_row_in_a_narrow_pane() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let text = rows(&pane, &shell, 20, 12).join("\n");
        assert!(!text.contains("run loop."), "{text}");
        for _ in 0..20 {
            let _ = pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        }
        let text = rows(&pane, &shell, 20, 12).join("\n");
        assert!(text.contains("run loop."), "{text}");
    }

    /// A pane that widened since the last key has fewer rows: the offset clamps to its last.
    #[tokio::test]
    async fn a_pane_that_widened_clamps_to_its_last_row() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let _ = rows(&pane, &shell, 20, 12);
        for _ in 0..20 {
            let _ = pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        }
        let rows = rows(&pane, &shell, 43, 30);
        assert_eq!(rows[4], "  loop.", "{rows:#?}");
    }

    /// Before the first render the scroll counts the body unwrapped, the old lower bound.
    #[tokio::test]
    async fn before_the_first_render_the_scroll_counts_unwrapped_rows() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        for _ in 0..5 {
            let _ = pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        }
        let rows = rows(&pane, &shell, 43, 30);
        assert_eq!(rows[4], "Scope", "{rows:#?}");
    }
}
