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

use htui_core::model::{Item, ItemId, ItemState, RunSummary, latest_graph_run};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, KeyChord};
use crate::store_worker::StoreReply;
use crate::ui::kit::status;
use crate::ui::markdown;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, Step, message};
use crossterm::event::KeyEvent;

/// How many lines the head block takes: title, key line, meta line, blank.
const HEAD: u16 = 4;

/// Title, key, kind, state, tags, priority, version, and the body reflowed and wrapped at the
/// pane's width (see the module doc). The state is the item's derived
/// [`ItemState`](htui_core::model::ItemState) mark (MOD-144), so the pane keeps the `Runs` reply
/// beside the `Item` one.
#[derive(Debug, Default)]
pub struct BodyTab {
    /// The selected item, from the last `on_item_change`.
    selected: Option<ItemId>,
    /// The `Item` reply for the selected item.
    item: Option<Item>,
    /// MOD-144: the selected item's runs (`Runs` reply), for its display state.
    runs: Option<Vec<RunSummary>>,
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

    /// The item's derived state: its stored status over its latest graph run, `open` until the
    /// `Runs` reply lands.
    fn state(&self, item: &Item) -> ItemState {
        let latest = self
            .runs
            .as_deref()
            .and_then(latest_graph_run)
            .map(|run| run.status);
        ItemState::of(item.status, latest)
    }
}

impl DetailTab for BodyTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Body"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.selected = item;
        self.item = None;
        self.runs = None;
        self.scroll.reset();
    }

    /// Whole chords through `BACKLOG_BODY` (the trait's default stack, MOD-67 M5 PA-1): the four
    /// pane-scroll acts scroll, so `alt-J` and `ctrl-PgDn` no longer do; everything else (the
    /// list, the sub-tabs, the Backlog's verbs, the shell's) is the tab's.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(self.key_stack(), chord) {
            match act {
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.len());
                }
                _ => {}
            }
        }
        Handled::Pass
    }

    /// MOD-115 M2 §7: the list's motion scrolls the body while the detail is focused.
    fn step(&mut self, step: Step, _ctx: &Ctx<'_>) -> Handled {
        self.scroll.follow(step, self.len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Item(item) => {
                self.item = (**item).clone();
                self.scroll.reset();
            }
            // A reply for another item (a late one after a move) is not this item's.
            StoreReply::Runs(runs) if runs.iter().all(|run| run.item_id == self.selected) => {
                self.runs = Some(runs.clone());
            }
            _ => {}
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
        let mut key_line = vec![
            Span::styled(item.key.as_str(), ctx.theme.key),
            Span::raw("  "),
            Span::styled(item.key_prefix.as_str(), ctx.theme.dim),
            Span::raw("  "),
        ];
        key_line.extend(status::spans(self.state(item), ctx.theme));
        let head = Text::from(vec![
            Line::styled(item.title.as_str(), ctx.theme.title),
            Line::from(key_line),
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

    /// A pane on FEAT-1 that has landed the item and its runs.
    async fn pane(shell: &Shell) -> BodyTab {
        let mut pane = BodyTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let store = MemStore::demo();
        let item = store.item(ids::HTUI_FEAT_1).await.expect("the item");
        let runs = store.runs(ids::HTUI_FEAT_1).await.expect("the runs");
        pane.on_reply(&StoreReply::Item(Box::new(item)), &mut shell.ctx());
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
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
                "FEAT-1  FEAT  \u{25d0} running",
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

    /// MOD-144 (PD16): the key line shows the derived state. Until the `Runs` reply lands it is
    /// the stored status over no run (`open`); another item's late `Runs` reply is not this
    /// item's; a move to another item forgets the runs.
    #[tokio::test]
    async fn the_key_line_reads_the_latest_graph_run() {
        let shell = Shell::new();
        let store = MemStore::demo();
        let mut pane = BodyTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let item = store.item(ids::HTUI_FEAT_1).await.expect("the item");
        pane.on_reply(&StoreReply::Item(Box::new(item.clone())), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[1],
            "FEAT-1  FEAT  \u{25cb} open"
        );

        let other = store.runs(ids::HTUI_CLEAN_1).await.expect("CLEAN-1's runs");
        assert!(!other.is_empty(), "CLEAN-1 has a failed run");
        pane.on_reply(&StoreReply::Runs(other), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[1],
            "FEAT-1  FEAT  \u{25cb} open",
            "CLEAN-1's runs are not FEAT-1's"
        );

        let runs = store.runs(ids::HTUI_FEAT_1).await.expect("FEAT-1's runs");
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[1],
            "FEAT-1  FEAT  \u{25d0} running",
            "the rework run RUN_4 is the latest"
        );

        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Item(Box::new(item)), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[1],
            "FEAT-1  FEAT  \u{25cb} open",
            "a move forgets the runs"
        );
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

    /// MOD-115 M2 §7: with the detail focused the list's motion scrolls the body, a row as `J`
    /// and `K` do, or to either end, clamped against the rows of the last render.
    #[tokio::test]
    async fn a_step_scrolls_the_body_as_its_scroll_keys_do() {
        let shell = Shell::new();
        let mut stepped = pane(&shell).await;
        let mut keyed = pane(&shell).await;
        let _ = rows(&stepped, &shell, 20, 12);
        let _ = rows(&keyed, &shell, 20, 12);
        assert_eq!(stepped.step(Step::Down, &shell.ctx()), Handled::Consumed);
        let _ = keyed.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(stepped.scroll, keyed.scroll, "a row, as `J`");
        assert_eq!(stepped.scroll.offset(), 1);

        let last = stepped.len() - 1;
        assert_eq!(stepped.step(Step::Bottom, &shell.ctx()), Handled::Consumed);
        assert_eq!(stepped.scroll.skip(), last, "the body's last row");
        assert_eq!(stepped.step(Step::Up, &shell.ctx()), Handled::Consumed);
        assert_eq!(stepped.scroll.skip(), last - 1, "a row up, as `K`");
        assert_eq!(stepped.step(Step::Top, &shell.ctx()), Handled::Consumed);
        assert_eq!(stepped.scroll.offset(), 0, "the top");
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
