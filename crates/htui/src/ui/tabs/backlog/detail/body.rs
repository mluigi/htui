//! The Body sub-tab: the item's meta line and its Markdown body.
//!
//! MOD-115 M4 E4 (MOD-82): the key and the title are the detail pane's title (`KEY · title`, B1),
//! so the head is one meta line, `{status mark} · {tags} · priority {p} · version {v}`, then a
//! blank. The title keeps a line of its own above it only while the border cannot hold the whole
//! `KEY · title`, so a long title is never on screen cut alone (acceptance 7); the key prefix the
//! old key line showed is the key's own (`key = key_prefix || '-' || key_number`).
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

use htui_core::model::{Item, ItemId, ItemState, RunStatus, Status, latest_graph_run};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, KeyChord, views};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::cells::cell_width;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::pane::Pane;
use crate::ui::kit::scroll::{Scroll, Step};
use crate::ui::kit::status;
use crate::ui::kit::text::wrapped;
use crate::ui::markdown;
use crate::ui::tabs::backlog::detail;
use crate::ui::tabs::backlog::detail::{
    CHOOSE_UNBOUND, DetailId, DetailTab, Framing, Told, no_item,
};
use crossterm::event::KeyEvent;

/// The sub-tab's chip, also what the border's title room is measured against.
const TITLE: &str = "Body";

/// Between the meta line's parts.
const SEP: &str = " \u{b7} ";

/// What the empty body says.
const NO_BODY: &str = "No body for this item.";

/// State, tags, priority, version, and the body reflowed and wrapped at the pane's width (see
/// the module doc); the title too while the border cannot hold it. The state is the item's derived
/// [`ItemState`] mark (MOD-144), so the pane keeps the latest graph run of the `Runs` reply beside
/// the `Item` one.
#[derive(Debug, Default)]
pub struct BodyTab {
    /// The `Item` reply for the selected item.
    item: Option<Item>,
    /// MOD-144: the status of the selected item's latest graph run, read once from the `Runs`
    /// reply: `None` until one lands, `Some(None)` for an item with no such run.
    latest: Option<Option<RunStatus>>,
    /// Scroll of the body paragraph, not of the head block.
    scroll: Scroll,
    /// The body area's width at the last render, for the scroll clamp (MOD-84 D7); 0 before the
    /// first, which counts the body unwrapped. `Cell` because `render` is `&self`.
    drawn: Cell<u16>,
    /// How the detail pane and the list beside it are drawn this frame (`DetailTab::framing`,
    /// MOD-115 M4 RV2-L1; RV, repair 2): what the no-item and failed-read steps read.
    told: Told,
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

    /// The item's derived state: its stored status over its latest graph run. `None` for an `open`
    /// item until the `Runs` reply lands (or when the read failed): its runs decide between
    /// `open`, `running`, `needs you` and the rest, so the pane must not guess `open`. A `blocked`
    /// or `closed` item reads its status at once, whatever its runs.
    fn state(&self, item: &Item) -> Option<ItemState> {
        match (item.status, self.latest) {
            (Status::Open, None) => None,
            (status, latest) => Some(ItemState::of(status, latest.flatten())),
        }
    }
}

/// Whether the detail pane's top border holds `key · title` whole while Body is its active chip,
/// the pane's inner rect `inner` cells wide: the kit's own answer (MOD-115 M4 RV2-L7), asked with
/// the detail's chips ([`detail::CHIPS`]) and Body's place among them.
fn border_holds(key: &str, title: &str, inner: u16) -> bool {
    let area = Rect::new(0, 0, inner.saturating_add(2), 3);
    let active = detail::CHIPS
        .iter()
        .position(|chip| *chip == TITLE)
        .unwrap_or_default();
    Pane::title_fits(area, &format!("{key}{SEP}{title}"), &detail::CHIPS, active)
}

/// The meta line's rows at `width`: the state as its mark in its own style (`...` in dim while it
/// is not known, MOD-144), then the tags, the priority and the version, dim. One row when it fits (the detail pane at 100 columns); else the
/// parts fill rows greedily (MOD-115 M4 RV2-L4): ` · ` joins two parts on a row and never ends or
/// starts one, so at 80 columns `version 1` takes the second row alone, and a part wider than the
/// pane wraps inside itself (`kit::text::wrapped`) on rows of its own, so no part is lost.
fn meta_lines(
    item: &Item,
    state: Option<ItemState>,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let tags: Cow<'_, str> = if item.required_tags.is_empty() {
        Cow::Borrowed("no tags")
    } else {
        Cow::Owned(item.required_tags.join(", "))
    };
    // Not the `open` mark while the runs are unread: the state is not known yet (MOD-144).
    let (mark, mark_style) = match state {
        Some(state) => {
            let mark = status::mark(state);
            (
                format!("{} {}", mark.glyph, mark.word),
                theme.status_style(state),
            )
        }
        None => ("...".to_owned(), theme.dim),
    };
    let rest = format!(
        "{SEP}{tags}{SEP}priority {}{SEP}version {}",
        item.priority, item.version
    );
    if cell_width(&mark) + cell_width(&rest) <= usize::from(width) {
        let mut spans = match state {
            Some(state) => status::spans(state, theme),
            None => vec![Span::styled(mark, mark_style)],
        };
        spans.push(Span::styled(rest, theme.dim));
        return vec![Line::from(spans)];
    }
    let parts = [
        (mark, mark_style),
        (tags.into_owned(), theme.dim),
        (format!("priority {}", item.priority), theme.dim),
        (format!("version {}", item.version), theme.dim),
    ];
    let width = usize::from(width.max(1));
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    // The cells the last row holds; `None` while that row must not take another part (none yet,
    // or the last piece of a part wrapped inside itself, which a separator would read as a part).
    let mut used: Option<usize> = None;
    for (part, style) in parts {
        let cells = cell_width(&part);
        match used {
            Some(held) if held + cell_width(SEP) + cells <= width => {
                if let Some(row) = rows.last_mut() {
                    row.push(Span::styled(SEP, theme.dim));
                    row.push(Span::styled(part, style));
                }
                used = Some(held + cell_width(SEP) + cells);
            }
            _ if cells <= width => {
                rows.push(vec![Span::styled(part, style)]);
                used = Some(cells);
            }
            _ => {
                rows.extend(
                    wrapped(&part, width)
                        .into_iter()
                        .map(|piece| vec![Span::styled(piece, style)]),
                );
                used = None;
            }
        }
    }
    rows.into_iter().map(Line::from).collect()
}

/// The empty body's next step: the key that opens the item form (`common.edit` through
/// `BACKLOG_BODY`), or the form itself while nothing binds it.
fn writes_one(ctx: &Ctx<'_>) -> String {
    detail::writes_one(
        views::BACKLOG_BODY,
        Act::Edit,
        "the item form writes one",
        ctx.keys(),
    )
}

impl DetailTab for BodyTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        TITLE
    }

    fn on_item_change(&mut self, _item: Option<ItemId>) {
        self.item = None;
        self.latest = None;
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
            // The reply carries no item id; the shell's staleness gate keeps it this item's
            // (`App::on_reply`): every `Runs` read the Backlog tab sends is for its selection, and
            // a move sends a new one, so a late reply for the item left behind is dropped before
            // it reaches a sub-tab. An empty reply is an item with no run.
            StoreReply::Runs(runs) => {
                self.latest = Some(latest_graph_run(runs).map(|run| run.status));
            }
            _ => {}
        }
    }

    fn framing(&self, framing: Framing) {
        self.told.set(framing);
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let Some(item) = self.item.as_ref() else {
            no_item(
                frame,
                area,
                views::BACKLOG_BODY,
                CHOOSE_UNBOUND,
                self.told.list(),
                ctx,
            );
            return;
        };

        let mut head: Vec<Line<'static>> = Vec::new();
        if !border_holds(&item.key, &item.title, area.width) {
            head.extend(
                wrapped(&item.title, usize::from(area.width))
                    .into_iter()
                    .map(|row| Line::styled(row, ctx.theme.title)),
            );
        }
        head.extend(meta_lines(item, self.state(item), area.width, ctx.theme));
        head.push(Line::raw(""));
        let rows = u16::try_from(head.len()).unwrap_or(u16::MAX);

        let [head_area, body_area] =
            Layout::vertical([Constraint::Length(rows), Constraint::Min(0)]).areas(area);
        frame.render_widget(Paragraph::new(Text::from(head)), head_area);

        self.drawn.set(body_area.width);
        if item.body.is_empty() {
            let next = writes_one(ctx);
            empty::render(frame, body_area, NO_BODY, NextStep(&next), ctx.theme);
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

    /// The rows FEAT-1's head takes at the detail pane's width: the meta line and a blank.
    const HEAD: usize = 2;

    /// MOD-84: at the detail pane's width the body reads as paragraphs and list items, with no
    /// ragged row where the source broke its lines (A-2) and no backtick of inline code.
    #[tokio::test]
    async fn the_body_reads_as_paragraphs_at_the_detail_width() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let rows = rows(&pane, &shell, 43, 30);
        assert_eq!(
            rows[..2],
            [
                "\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
                ""
            ],
            "the head is the meta line alone (MOD-115 M4 E4): {rows:#?}"
        );
        assert_eq!(
            rows[2..],
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
                "- The top bar reads workspace, box, store",
                "  label and the active run count.",
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

    /// MOD-144 (PD16, R1 M3): the meta line shows the derived state. Until the `Runs` reply lands
    /// an `open` item shows a dim placeholder, never the `open` mark its runs may contradict; the
    /// reply's latest graph run decides it, an empty reply reading `open` (no run); a move to
    /// another item forgets it. A `blocked` item reads `blocked` at once.
    #[tokio::test]
    async fn the_meta_line_reads_the_latest_graph_run() {
        let shell = Shell::new();
        let store = MemStore::demo();
        let mut pane = BodyTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let item = store.item(ids::HTUI_FEAT_1).await.expect("the item");
        pane.on_reply(&StoreReply::Item(Box::new(item.clone())), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "... \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "no runs read yet: not `open`"
        );

        let runs = store.runs(ids::HTUI_FEAT_1).await.expect("FEAT-1's runs");
        pane.on_reply(&StoreReply::Runs(runs.clone()), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "the rework run RUN_4 is the latest"
        );

        pane.on_reply(&StoreReply::Runs(Vec::new()), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "\u{25cb} open \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "an empty reply is an item with no run"
        );
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "each reply replaces the last"
        );

        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Item(Box::new(item.clone())), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "... \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "a move forgets the runs"
        );

        let blocked = item.map(|item| Item {
            status: Status::Blocked,
            ..item
        });
        pane.on_reply(&StoreReply::Item(Box::new(blocked)), &mut shell.ctx());
        assert_eq!(
            rows(&pane, &shell, 43, 30)[0],
            "\u{2298} blocked \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
            "a blocked item needs no runs to read `blocked`"
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
        assert_eq!(rows[HEAD], "  loop.", "{rows:#?}");
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
        assert_eq!(rows[HEAD], "Scope", "{rows:#?}");
    }

    /// FEAT-1 with `edit` applied, landed in a fresh pane.
    async fn landed(shell: &Shell, edit: impl FnOnce(&mut Item)) -> BodyTab {
        let mut item = MemStore::demo()
            .item(ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails")
            .expect("the demo item");
        edit(&mut item);
        let mut pane = BodyTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Item(Box::new(Some(item))), &mut shell.ctx());
        let runs = MemStore::demo()
            .runs(ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails");
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        pane
    }

    /// E4: the status is its mark in the status role (`kit::status`), the rest of the meta line
    /// dim; no raw `in_progress` and no key line.
    #[tokio::test]
    async fn the_status_is_its_mark_in_the_status_role() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(43, 6)).expect("a terminal");
        terminal
            .draw(|frame| pane.render(frame, frame.area(), &shell.ctx()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        let status = shell.theme.status_style(ItemState::Running);
        assert_eq!(buffer[(0, 0)].symbol(), "\u{25d0}");
        assert_eq!(
            buffer[(0, 0)].fg,
            status.fg.unwrap_or_default(),
            "the glyph"
        );
        assert_eq!(buffer[(2, 0)].fg, status.fg.unwrap_or_default(), "the word");
        let dim = shell.theme.dim.fg.unwrap_or_default();
        assert_eq!(buffer[(12, 0)].fg, dim, "the tags");
        let text = rows(&pane, &shell, 43, 6).join("\n");
        assert!(
            !text.contains("in_progress") && !text.contains("FEAT  "),
            "{text}"
        );
    }

    /// The title line goes exactly when the border draws `KEY · title` whole, for every width
    /// and title length around the edge (`Pane::title_fits` is swept in `kit::pane` itself).
    #[test]
    fn border_holds_says_what_the_kit_border_draws() {
        for inner in 30u16..=60 {
            for length in 10..=50 {
                let title = "t".repeat(length);
                let whole = format!("FEAT-1 \u{b7} {title}");
                let drawn = drawn(inner + 2, 3, |frame, area| {
                    Pane::new(&whole)
                        .title_first()
                        .sub_tabs(&detail::CHIPS, 0)
                        .render(frame, area, &Theme::default());
                });
                let top = drawn.lines().next().unwrap_or_default().to_owned();
                assert_eq!(
                    border_holds("FEAT-1", &title, inner),
                    top.contains(&format!(" {whole} ")),
                    "{inner} cells, a {length}-cell title: {top}"
                );
            }
        }
    }

    /// Acceptance 7: a title the border cuts keeps its line, wrapped, above the meta line.
    #[tokio::test]
    async fn a_title_the_border_cuts_keeps_its_line() {
        let shell = Shell::new();
        let title = "A title far too long for the detail border to hold whole";
        let pane = landed(&shell, |item| title.clone_into(&mut item.title)).await;
        let rows = rows(&pane, &shell, 43, 30);
        assert_eq!(
            rows[..4],
            [
                "A title far too long for the detail border",
                "to hold whole",
                "\u{25d0} running \u{b7} rust \u{b7} priority 2 \u{b7} version 1",
                "",
            ],
            "{rows:#?}"
        );
        assert_eq!(rows[4], "Stand up the terminal application: a");
    }

    /// A meta line wider than the pane wraps by part rather than losing its tail (RV2-L4: a row
    /// ends on a part, never on its separator, and no part is split while it fits a row); so
    /// narrow a border cuts even `FEAT-1 · TUI scaffold`, so the title keeps its line above it.
    #[tokio::test]
    async fn a_narrow_pane_wraps_the_meta_line() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let rows = rows(&pane, &shell, 20, 12);
        assert_eq!(
            rows[..5],
            [
                "TUI scaffold",
                "\u{25d0} running \u{b7} rust",
                "priority 2",
                "version 1",
                "",
            ],
            "{rows:#?}"
        );
    }

    /// RV2-L4: at every width the meta line keeps all of its parts, in order, and no row starts
    /// or ends on the separator; a part wider than the pane wraps inside itself, whole.
    #[tokio::test]
    async fn the_meta_line_never_leaves_a_dangling_separator() {
        let shell = Shell::new();
        let pane = landed(&shell, |item| {
            item.required_tags = vec!["rust".to_owned(), "a-very-long-tag-name".to_owned()];
        })
        .await;
        let Some(item) = pane.item.as_ref() else {
            panic!("the item landed");
        };
        for width in 8..=60 {
            let lines = meta_lines(item, pane.state(item), width, &shell.theme);
            let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
            for row in &text {
                assert!(cell_width(row) <= usize::from(width), "{width}: {text:#?}");
                assert!(
                    !row.ends_with('\u{b7}') && !row.starts_with('\u{b7}'),
                    "{width}: {text:#?}"
                );
            }
            let joined: String = text.concat().split_whitespace().collect();
            for part in [
                "running",
                "rust,",
                "a-very-long-tag-name",
                "priority2",
                "version1",
            ] {
                assert!(joined.contains(part), "{width}: {part} in {text:#?}");
            }
        }
    }

    /// MOD-115 M4 RV2-M1: no item under the list's cursor is the kit's empty state, its next step
    /// the list's motion through `BACKLOG_BODY`, as every other sub-tab says it.
    #[test]
    fn no_item_is_an_empty_state_with_a_next_step() {
        let shell = Shell::new();
        let text = drawn(43, 6, |frame, area| {
            BodyTab::new().render(frame, area, &shell.ctx());
        });
        detail::assert_empty_state(&text, "No item selected.", "j/k chooses one");
    }

    /// No body: `kit::empty`, its next step the key the item form opens on, rebound or not.
    #[tokio::test]
    async fn an_empty_body_names_the_edit_key() {
        let shell = Shell::new();
        let pane = landed(&shell, |item| item.body.clear()).await;
        let text = rows(&pane, &shell, 43, 12).join("\n");
        assert!(text.contains("No body for this item."), "{text}");
        assert!(text.contains("e writes one"), "{text}");

        let keys = crate::keys::load_str("version = 1\n[common]\nedit = \"ctrl-y\"\n")
            .expect("the keys load");
        let scope = htui_core::model::Scope {
            workspace_id: htui_core::model::WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let top_bar = crate::app::TopBarState::default();
        let emit = crate::app::Emit::default();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keys,
            &shell.theme,
            crate::store_worker::Origin::Tab(crate::ui::tabs::BacklogTab::ID),
            &emit,
        );
        let text = drawn(43, 12, |frame, area| pane.render(frame, area, &ctx));
        assert!(text.contains("Ctrl+y writes one"), "{text}");
    }
}
