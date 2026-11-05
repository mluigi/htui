//! Logs tab (MOD-122 D4, `R-TUI-15`): htui's own log as it runs.
//!
//! The sources on the left (`all`, then each [`Source`] in the requirement's order, `other` only
//! while it holds a line), with counts of the lines the buffer still holds and of their warnings;
//! the selected source's lines on the right, one row each, coloured by level with the level word
//! always printed, so monochrome loses nothing.
//!
//! The tab reads a [`LogBuffer`] and never logs: [`Tab::on_activate`] and [`Tab::on_refresh`]
//! copy the lines newer than the last sequence it saw (D2), so the lines pane is current when the
//! tab comes to the front and moves once a second after, as the shell's refresh does. The lines pane follows the newest line until a step up pauses it; a paused view is
//! anchored on the sequence of its top line (blueprint B4), so lines dropping off the ring, a new
//! minimum level or another source never move what a paused reader sees. `logs.bottom` jumps to
//! the newest line and follows again, as `chat.bottom` does; `logs.follow` toggles following.

use core::cell::Cell;
use core::fmt;
use std::collections::VecDeque;

use crossterm::event::KeyEvent;
use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tracing::Level;

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, Stack, views};
use crate::logging::{Counts, LogBuffer, LogLine, Source, SourceCounts};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::chips::key_line;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::pane::{Focus, Pane};
use crate::ui::kit::scroll::{PAGE, Step, window};
use crate::ui::shell::{ListSize, PaneId, Panes, TabLayout, TabLoc};
use crate::ui::tabs::registry::{Tab, TabId};

/// The Logs tab's default layout (MOD-122 D4): a 30-cell sources list beside the lines.
pub const LOGS_LAYOUT: TabLayout = TabLayout {
    list: ListSize::Cells(30),
    ..TabLayout::DEFAULT
};

/// How a [`TabLoc`] cursor names the selected source: `source:all`, `source:store`, ….
const SOURCE_CURSOR: &str = "source";

/// The `all` row's name, beside the [`Source`] names.
const ALL: &str = "all";

/// The hint row the focused pane carries.
const HINT: HintSpec = &[
    Hint::One(Act::LogsLevel, "level"),
    Hint::One(Act::LogsFollow, "follow"),
    Hint::One(Act::LogsBottom, "newest"),
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
];

/// The empty buffer's next step (MOD-83: an empty state always names one). The blueprint's
/// "htui's own log appears here as it runs; …" is 76 cells and the lines pane holds 68 at 100
/// columns, so "as it runs" goes: 64 cells, drawn whole.
const NO_LINES_NEXT: &str =
    "htui's own log appears here; Settings \u{203a} Logging sets what is kept";

/// The level filter's next step when `logs.level` has no key.
const LOWER_THE_LEVEL: &str = "lower the minimum level";

/// The narrowest lines pane (inner cells) that keeps the source column under `all`.
const SOURCE_WIDTH: u16 = 48;

/// The narrowest lines pane (inner cells) that keeps the time column.
const TIME_WIDTH: u16 = 36;

/// Where the lines pane is (blueprint B4).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Scroll {
    /// Following: the last page, whatever arrives.
    #[default]
    Tail,
    /// Paused with this sequence's line (or the next one still shown) on top.
    At(u64),
}

/// The Logs tab (MOD-122 D4).
pub struct LogsTab {
    /// The process's log; the tab only copies from it.
    buffer: LogBuffer,
    /// The tab's copy of the lines the buffer retains, oldest first.
    lines: VecDeque<LogLine>,
    /// The sequence to ask the buffer from next.
    next: u64,
    /// The buffer's counts at the last pull (D2: of retained lines).
    counts: SourceCounts,
    /// The selected source; `None` is `all`.
    selected: Option<Source>,
    /// The least severe level shown: `TRACE` shows everything the global filter let through.
    min_level: Level,
    /// Following or paused.
    scroll: Scroll,
    /// The lines pane's rows as last drawn, so a step knows the page.
    height: Cell<usize>,
}

impl fmt::Debug for LogsTab {
    /// The state, never the lines (as [`LogBuffer`]'s own).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogsTab")
            .field("lines", &self.lines.len())
            .field("next", &self.next)
            .field("selected", &self.selected)
            .field("min_level", &self.min_level)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl LogsTab {
    /// The tab's id; the theme's `logs` accent is keyed by it.
    pub const ID: TabId = TabId("logs");

    /// The tab over `buffer`, with what it already holds.
    #[must_use]
    pub fn new(buffer: LogBuffer) -> Self {
        let mut tab = Self {
            buffer,
            lines: VecDeque::new(),
            next: 0,
            counts: SourceCounts::default(),
            selected: None,
            min_level: Level::TRACE,
            scroll: Scroll::Tail,
            height: Cell::new(0),
        };
        tab.pull();
        tab
    }

    /// Copies the lines newer than the last pull, drops the ones the ring dropped and adopts
    /// its counts, all from one read, so the counts match the lines (D2).
    fn pull(&mut self) {
        let since = self.buffer.since(self.next);
        self.lines.extend(since.lines);
        while self
            .lines
            .front()
            .is_some_and(|line| line.seq < since.oldest)
        {
            self.lines.pop_front();
        }
        self.next = since.next;
        self.counts = since.counts;
        self.keep_selection_listed();
    }

    /// D3: `other` is listed only while it holds a line, so a selected `other` that holds none
    /// hands the cursor to `all`.
    fn keep_selection_listed(&mut self) {
        if self.selected == Some(Source::Other) && self.counts.of(Source::Other).lines == 0 {
            self.selected = None;
        }
    }

    /// The sources list: `all`, then every source in the requirement's order, `other` only
    /// while it holds a line (D3).
    fn rows(&self) -> Vec<Option<Source>> {
        let mut rows = vec![None];
        rows.extend(
            Source::ALL
                .into_iter()
                .filter(|&source| source != Source::Other || self.counts.of(source).lines > 0)
                .map(Some),
        );
        rows
    }

    /// Whether `line` is from the selected source (or any, under `all`).
    fn is_selected_source(&self, line: &LogLine) -> bool {
        self.selected.is_none_or(|source| line.source == source)
    }

    /// The lines the lines pane shows: the selected source's at the minimum level or above
    /// (`tracing` orders `ERROR` < … < `TRACE`).
    fn visible(&self) -> Vec<&LogLine> {
        self.lines
            .iter()
            .filter(|line| self.is_selected_source(line) && line.level <= self.min_level)
            .collect()
    }

    /// The first row of `visible` shown in `height` rows: the last page while following, else
    /// the anchor's line (or the next one still held), never past the last page.
    fn top(&self, visible: &[&LogLine], height: usize) -> usize {
        let last_page = visible.len().saturating_sub(height);
        match self.scroll {
            Scroll::Tail => last_page,
            Scroll::At(seq) => visible
                .partition_point(|line| line.seq < seq)
                .min(last_page),
        }
    }

    /// Moves the lines pane `delta` rows (blueprint's state machine, after the chat
    /// transcript's): a step down that reaches the last page follows again; a step up pauses,
    /// unless everything fits one page, which keeps following.
    fn step_lines(&mut self, delta: isize) {
        let visible = self.visible();
        let height = self.height.get();
        let last_page = visible.len().saturating_sub(height);
        let anchor = |index: usize| {
            visible
                .get(index.min(last_page))
                .map_or(Scroll::Tail, |line| Scroll::At(line.seq))
        };
        let scroll = match self.scroll {
            Scroll::Tail if delta >= 0 || last_page == 0 => Scroll::Tail,
            Scroll::Tail => anchor(last_page.saturating_sub(delta.unsigned_abs())),
            Scroll::At(_) => {
                let to = self.top(&visible, height).saturating_add_signed(delta);
                if delta > 0 && to >= last_page {
                    Scroll::Tail
                } else {
                    anchor(to)
                }
            }
        };
        self.scroll = scroll;
    }

    /// A list motion the focused lines pane takes (`kit::scroll::Step::by_focus`).
    fn on_step(&mut self, step: Step) -> Handled {
        match step {
            Step::Down => self.step_lines(1),
            Step::Up => self.step_lines(-1),
            Step::Top => {
                if let Some(first) = self.visible().first() {
                    self.scroll = Scroll::At(first.seq);
                }
            }
            Step::Bottom => self.scroll = Scroll::Tail,
        }
        Handled::Consumed
    }

    /// `logs.follow`: following pauses where the view is; paused follows again.
    fn toggle_follow(&mut self) {
        self.scroll = match self.scroll {
            Scroll::Tail => {
                let visible = self.visible();
                let top = self.top(&visible, self.height.get());
                Scroll::At(visible.get(top).map_or(self.next, |line| line.seq))
            }
            Scroll::At(_) => Scroll::Tail,
        };
    }

    /// A list motion on the sources list; the follow state stays as it is.
    fn move_source(&mut self, act: Act) {
        let rows = self.rows();
        let cursor = rows
            .iter()
            .position(|row| *row == self.selected)
            .unwrap_or(0);
        let cursor = match act {
            Act::ListDown => (cursor + 1).min(rows.len().saturating_sub(1)),
            Act::ListUp => cursor.saturating_sub(1),
            _ => 0,
        };
        self.selected = rows.get(cursor).copied().flatten();
    }

    /// The sources list, the cursor on the selected row.
    fn render_sources(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        hints: Option<&Line<'static>>,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = Pane::inner(area);
        let rows = self.rows();
        let cursor = rows
            .iter()
            .position(|row| *row == self.selected)
            .unwrap_or(0);
        let list_rows: Vec<Row<'static>> = rows
            .iter()
            .map(|row| {
                let (name, counts) = match row {
                    None => (ALL, self.counts.all()),
                    Some(source) => (source.name(), self.counts.of(*source)),
                };
                Row {
                    line: source_line(name, counts, theme),
                    kind: RowKind::Item,
                }
            })
            .collect();
        let offset = window(cursor, rows.len(), usize::from(inner.height));
        list::render(
            frame,
            inner,
            &list_rows,
            Some(cursor),
            offset,
            ctx.accent(),
            theme,
        );
        let mut pane = Pane::new("Sources")
            .focus(focus)
            .accent(ctx.accent())
            .scrollbar(rows.len(), offset);
        if let Some(hints) = hints {
            pane = pane.hints(hints.clone());
        }
        pane.render(frame, area, theme);
    }

    /// The selected source's lines, or the empty state that says why there are none.
    fn render_lines(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        hints: Option<&Line<'static>>,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = Pane::inner(area);
        let height = usize::from(inner.height);
        self.height.set(height);
        let visible = self.visible();
        let top = self.top(&visible, height);
        let source = self.selected.map_or(ALL, Source::name);
        let level = self.min_level.as_str().to_ascii_lowercase();
        let state = match self.scroll {
            Scroll::Tail => "following",
            Scroll::At(_) => "paused",
        };
        if visible.is_empty() {
            let (what, next) = self.empty_state(source, &level, ctx);
            empty::render(frame, inner, &what, NextStep(&next), theme);
        } else {
            let rows: Vec<Line<'static>> = visible
                .iter()
                .skip(top)
                .take(height)
                .map(|line| self.line(line, inner.width, theme))
                .collect();
            frame.render_widget(Paragraph::new(rows), inner);
        }
        // D4: the border is the only place the state and the level show; a title too long for
        // it puts them first and leaves the source (the list's cursor names it) to be clipped.
        let mut title = format!("{source} \u{b7} min {level} \u{b7} {state}");
        if !Pane::title_fits(area, &title, &[], 0) {
            title = format!("{state} \u{b7} min {level} \u{b7} {source}");
        }
        let mut pane = Pane::new(&title)
            .focus(focus)
            .accent(ctx.accent())
            .scrollbar(visible.len(), top);
        if let Some(hints) = hints {
            pane = pane.hints(hints.clone());
        }
        pane.render(frame, area, theme);
    }

    /// Why the lines pane is empty, and the next step: nothing logged yet, nothing at this
    /// level, or nothing from this source.
    fn empty_state(&self, source: &str, level: &str, ctx: &Ctx<'_>) -> (String, String) {
        if self.lines.is_empty() {
            return ("No log lines yet.".to_owned(), NO_LINES_NEXT.to_owned());
        }
        if self.lines.iter().any(|line| self.is_selected_source(line)) {
            let next = ctx
                .keys()
                .label(views::LOGS_BROWSE, Act::LogsLevel)
                .map_or_else(
                    || LOWER_THE_LEVEL.to_owned(),
                    |key| format!("{key} lowers the minimum level"),
                );
            return (format!("No lines at {level} or above."), next);
        }
        (
            format!("No lines from {source} yet."),
            NO_LINES_NEXT.to_owned(),
        )
    }

    /// One line, one row (blueprint B6): time, level word, source under `all`, text, clipped
    /// to `width`. A deliberate deviation from B6's fixed shape: a pane narrower than
    /// [`SOURCE_WIDTH`] gives up the source column and one narrower than [`TIME_WIDTH`] the time
    /// too, so the text keeps room (at 60 columns the pane holds 28 cells, exactly the full
    /// prefix). The level word always stays (D4).
    fn line(&self, line: &LogLine, width: u16, theme: &Theme) -> Line<'static> {
        let style = level_style(line.level, theme);
        let mut spans = Vec::with_capacity(4);
        if width >= TIME_WIDTH {
            spans.push(Span::styled(
                format!("{} ", line.at.format("%H:%M:%S")),
                theme.dim,
            ));
        }
        spans.push(Span::styled(format!("{:<5} ", line.level.as_str()), style));
        if self.selected.is_none() && width >= SOURCE_WIDTH {
            spans.push(Span::styled(
                format!("{:<12} ", line.source.name()),
                theme.dim,
            ));
        }
        spans.push(Span::styled(line.text.clone(), style));
        cells::clip_line(Line::from(spans), width)
    }
}

/// A sources-list row: `name  lines · ⚠ warnings`, the warnings in the warning role while any.
fn source_line(name: &str, counts: Counts, theme: &Theme) -> Line<'static> {
    let warn = if counts.warnings > 0 {
        theme.warning
    } else {
        theme.dim
    };
    Line::from(vec![
        Span::styled(format!("{name:<12} {:>5} ", counts.lines), theme.base),
        Span::styled(format!("\u{b7} \u{26a0} {}", counts.warnings), warn),
    ])
}

/// A level's role (D4): error, warning, base, then dim for debug and trace.
fn level_style(level: Level, theme: &Theme) -> Style {
    match level {
        Level::ERROR => theme.error,
        Level::WARN => theme.warning,
        Level::INFO => theme.base,
        _ => theme.dim,
    }
}

/// `logs.level`'s cycle: trace → debug → info → warn → error → trace.
fn next_level(level: Level) -> Level {
    match level {
        Level::TRACE => Level::DEBUG,
        Level::DEBUG => Level::INFO,
        Level::INFO => Level::WARN,
        Level::WARN => Level::ERROR,
        _ => Level::TRACE,
    }
}

impl Tab for LogsTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Logs"
    }

    /// The log is the process's: nothing to read from the store.
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        Vec::new()
    }

    /// The log is the process's, not a workspace's: a scope change keeps it.
    fn on_scope_change(&mut self, _scope: &Scope) {}

    /// Every key resolves through [`views::LOGS_BROWSE`]: a list motion moves what the focused
    /// pane holds (the sources, or the lines), the pane scroll always moves the lines, and the
    /// tab's own verbs set the level and the follow state.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        let focus = ctx.focus();
        for act in ctx.keys().actions(views::LOGS_BROWSE, chord) {
            if Step::by_focus(act, focus, |step| self.on_step(step)) {
                return Handled::Consumed;
            }
            match act {
                Act::ListDown | Act::ListUp | Act::ListTop => self.move_source(act),
                Act::PaneScrollDown => self.step_lines(1),
                Act::PaneScrollUp => self.step_lines(-1),
                Act::PanePageDown => self.step_lines(PAGE),
                Act::PanePageUp => self.step_lines(-PAGE),
                Act::LogsLevel => self.min_level = next_level(self.min_level),
                Act::LogsFollow => self.toggle_follow(),
                Act::LogsBottom => self.scroll = Scroll::Tail,
                // A layout, history or global act: the shell's.
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// The tab asks the store nothing, so nothing comes back.
    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        key_line(ctx.keys().hint(views::LOGS_BROWSE, HINT), ctx.theme)
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let areas = ctx.split(area, LOGS_LAYOUT);
        let hints = key_line(ctx.keys().hint(views::LOGS_BROWSE, HINT), ctx.theme);
        if let Some(list) = areas.list.filter(|rect| !rect.is_empty()) {
            let focus = ctx.pane_focus(PaneId::List);
            self.render_sources(frame, list, focus, hints.as_ref(), ctx);
        }
        if !areas.detail.is_empty() {
            let focus = ctx.pane_focus(PaneId::Detail);
            self.render_lines(frame, areas.detail, focus, hints.as_ref(), ctx);
        }
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::LOGS_BROWSE)
    }

    /// MOD-122 D4: new lines, once a second while the tab is in front.
    fn on_refresh(&mut self, _ctx: &mut Ctx<'_>) {
        self.pull();
    }

    /// MOD-122 R1: new lines the moment the tab comes to the front, not a refresh later.
    fn on_activate(&mut self, _ctx: &mut Ctx<'_>) {
        self.pull();
    }

    fn panes(&self) -> Panes {
        Panes::ListDetail(LOGS_LAYOUT)
    }

    /// The selected source, `source:{name}`.
    fn location(&self) -> TabLoc {
        TabLoc {
            cursor: Some(format!(
                "{SOURCE_CURSOR}:{}",
                self.selected.map_or(ALL, Source::name)
            )),
            ..TabLoc::default()
        }
    }

    /// Back to a [`location`](Tab::location)'s source (an empty `other` lands on `all`, D3);
    /// the follow state stays as it is.
    fn restore(&mut self, loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
        let Some(name) = loc
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.strip_prefix(SOURCE_CURSOR))
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            return false;
        };
        let selected = if name == ALL {
            None
        } else {
            match Source::from_name(name) {
                Some(source) => Some(source),
                None => return false,
            }
        };
        self.selected = selected;
        self.keep_selection_listed();
        true
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};
    use crossterm::event::MouseEventKind;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Style};
    use tracing::Level;

    use super::*;
    use crate::app::Action;
    use crate::logging::CAPACITY;
    use crate::testkit::{Harness, SectionBench};

    /// A fixed wall-clock time, so no snapshot moves with the clock (plan risk "timestamps").
    fn at(second: usize) -> DateTime<Utc> {
        let second = u32::try_from(second % 60).expect("under sixty");
        Utc.with_ymd_and_hms(2026, 11, 5, 9, 30, second)
            .single()
            .expect("a valid time")
    }

    /// Store, agents, ui and third-party lines, one of every level.
    fn mixed() -> LogBuffer {
        let buffer = LogBuffer::new();
        buffer.push(
            at(1),
            Level::INFO,
            "htui::app::update",
            "shell started".into(),
        );
        buffer.push(
            at(2),
            Level::DEBUG,
            "htui_store::cache",
            "cache refreshed rows=3".into(),
        );
        buffer.push(
            at(3),
            Level::WARN,
            "htui_store::pg",
            "connection slow ms=812".into(),
        );
        buffer.push(
            at(4),
            Level::INFO,
            "htui_agent::acp",
            "session opened agent=\"claude\"".into(),
        );
        buffer.push(
            at(5),
            Level::ERROR,
            "htui_agent::acp",
            "adapter exited code=1".into(),
        );
        buffer.push(at(6), Level::TRACE, "htui::ui::tabs", "frame drawn".into());
        buffer.push(
            at(7),
            Level::WARN,
            "hyper::client",
            "connection reset".into(),
        );
        buffer
    }

    /// `count` ui lines at `INFO`, `line 0` first.
    fn many(count: usize) -> LogBuffer {
        let buffer = LogBuffer::new();
        push_lines(&buffer, 0..count);
        buffer
    }

    /// `line {n}` for every `n` in `range`, at `INFO` from `htui::app`.
    fn push_lines(buffer: &LogBuffer, range: std::ops::Range<usize>) {
        for n in range {
            buffer.push(at(n), Level::INFO, "htui::app", format!("line {n}"));
        }
    }

    /// The Logs tab alone over `buffer`, on the suite's 100x30 frame.
    async fn logs(buffer: &LogBuffer) -> Harness {
        logs_at(buffer, 100, 30).await
    }

    /// [`logs`] on a `width` x `height` frame.
    async fn logs_at(buffer: &LogBuffer, width: u16, height: u16) -> Harness {
        let mut harness = Harness::empty()
            .size(width, height)
            .with_tab(Box::new(LogsTab::new(buffer.clone())));
        harness.settle().await;
        harness
    }

    /// The shell's once-a-second refresh: four ticks.
    fn refresh(harness: &mut Harness) {
        for _ in 0..4 {
            harness.app().update(Action::Tick);
        }
    }

    /// Which of `line {n}` for `n` in `range` the frame shows.
    fn shown(frame: &str, range: std::ops::Range<usize>) -> Vec<usize> {
        range
            .filter(|n| frame.contains(&format!("line {n} ")))
            .collect()
    }

    /// The style of the first cell of `word`'s first occurrence in `buffer`.
    fn style_of(buffer: &Buffer, word: &str) -> Style {
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            let row: Vec<&str> = (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let text = row.concat();
            if let Some(byte) = text.find(word) {
                let column = text[..byte].chars().count();
                let x = area.left() + u16::try_from(column).expect("fits");
                return buffer[(x, y)].style();
            }
        }
        panic!("`{word}` is not on screen");
    }

    /// A source's list row, as the tab writes it.
    fn source_row(name: &str, lines: usize, warnings: usize) -> String {
        format!("{name:<12} {lines:>5} \u{b7} \u{26a0} {warnings}")
    }

    #[tokio::test]
    async fn an_empty_buffer_says_where_lines_come_from() {
        let mut harness = logs(&LogBuffer::new()).await;
        let frame = harness.render();
        assert!(frame.contains("No log lines yet."), "{frame}");
        assert!(
            frame.contains(
                "htui's own log appears here; Settings \u{203a} Logging sets what is kept"
            ),
            "the whole next step fits the lines pane at 100 columns: {frame}"
        );
        assert!(frame.contains(&source_row("all", 0, 0)), "{frame}");
        insta::assert_snapshot!("logs_empty", frame);
    }

    #[tokio::test]
    async fn the_sources_list_counts_lines_and_warnings() {
        let mut harness = logs(&mixed()).await;
        let frame = harness.render();
        for row in [
            source_row("all", 7, 3),
            source_row("ui", 2, 0),
            source_row("store", 2, 1),
            source_row("agents", 2, 1),
            source_row("other", 1, 1),
        ] {
            assert!(frame.contains(&row), "`{row}` in {frame}");
        }
        let all = frame.find(&source_row("all", 7, 3));
        let ui = frame.find(&source_row("ui", 2, 0));
        assert!(all < ui, "all first");
        insta::assert_snapshot!("logs_sources", frame);
    }

    #[test]
    fn other_is_listed_only_with_lines() {
        let buffer = LogBuffer::new();
        buffer.push(at(1), Level::INFO, "htui::app", "one".into());
        let mut tab = LogsTab::new(buffer.clone());
        let mut expected: Vec<Option<Source>> = vec![None];
        expected.extend(
            Source::ALL
                .into_iter()
                .filter(|source| *source != Source::Other)
                .map(Some),
        );
        assert_eq!(tab.rows(), expected, "no third-party line: no `other`");
        buffer.push(at(2), Level::INFO, "hyper::client", "two".into());
        tab.pull();
        assert_eq!(tab.rows().last(), Some(&Some(Source::Other)));
    }

    /// D3: `other` is listed only while it holds a line, so a selected `other` whose lines
    /// leave the ring hands the cursor to `all`.
    #[test]
    fn a_selected_other_that_empties_hands_the_cursor_to_all() {
        let buffer = LogBuffer::new();
        buffer.push(at(1), Level::WARN, "hyper::client", "reset".into());
        let mut tab = LogsTab::new(buffer.clone());
        tab.move_source(Act::ListUp);
        while tab.selected != Some(Source::Other) {
            tab.move_source(Act::ListDown);
        }
        push_lines(&buffer, 0..CAPACITY);
        tab.pull();
        assert_eq!(tab.selected, None, "the cursor is on `all`");
        assert!(!tab.rows().contains(&Some(Source::Other)), "no `other` row");
    }

    /// D3: a location on `other` while it holds no line lands on `all`, the row it had.
    #[tokio::test]
    async fn restoring_an_empty_other_lands_on_all() {
        let bench = SectionBench::new().await;
        let mut tab = LogsTab::new(many(3));
        tab.selected = Some(Source::Store);
        let other = TabLoc {
            cursor: Some("source:other".to_owned()),
            ..TabLoc::default()
        };
        assert!(tab.restore(&other, &mut bench.ctx()));
        assert_eq!(tab.selected, None);
        assert!(!tab.rows().contains(&Some(Source::Other)));
    }

    #[tokio::test]
    async fn each_level_takes_its_style_and_its_word() {
        let mut harness = logs(&mixed()).await;
        // The lines pane holds the keys, so its hints show in the snapshot. The shell moves
        // focus between the panes it last drew, so a frame comes first.
        harness.render();
        harness.key("tab");
        let theme = Theme::default();
        let buffer = harness.render_buffer();
        // An unset foreground draws as the terminal's own.
        let fg = |style: Style| style.fg.unwrap_or(Color::Reset);
        for (word, role) in [
            ("ERROR", theme.error),
            ("adapter exited", theme.error),
            ("WARN ", theme.warning),
            ("connection slow", theme.warning),
            ("INFO ", theme.base),
            ("shell started", theme.base),
            ("DEBUG", theme.dim),
            ("TRACE", theme.dim),
            ("09:30:05", theme.dim),
        ] {
            assert_eq!(fg(style_of(&buffer, word)), fg(role), "{word}");
        }
        assert_ne!(fg(theme.base), fg(theme.dim), "info reads apart from debug");
        insta::assert_snapshot!("logs_levels", harness.render());
    }

    #[tokio::test]
    async fn under_monochrome_the_level_word_is_the_signal() {
        let mut harness = logs(&mixed()).await;
        harness.app().theme = Theme::monochrome();
        let buffer = harness.render_buffer();
        for word in ["ERROR", "WARN ", "INFO ", "DEBUG", "TRACE"] {
            let style = style_of(&buffer, word);
            assert!(
                matches!(style.fg, None | Some(Color::Reset)),
                "{word}: {style:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_minimum_level_hides_lower_lines_and_shows_on_the_border() {
        let mut harness = logs(&mixed()).await;
        assert!(harness.render().contains("all \u{b7} min trace"));
        for _ in 0..3 {
            harness.key("v");
        }
        let frame = harness.render();
        assert!(
            frame.contains("all \u{b7} min warn \u{b7} following"),
            "{frame}"
        );
        assert!(frame.contains("connection slow"), "{frame}");
        assert!(frame.contains("adapter exited"), "{frame}");
        assert!(!frame.contains("shell started"), "{frame}");
        assert!(!frame.contains("cache refreshed"), "{frame}");
        insta::assert_snapshot!("logs_min_warn", frame);
    }

    #[tokio::test]
    async fn the_border_says_following_or_paused() {
        let mut harness = logs(&mixed()).await;
        assert!(harness.render().contains("following"));
        harness.key("F");
        let frame = harness.render();
        assert!(
            frame.contains("all \u{b7} min trace \u{b7} paused"),
            "{frame}"
        );
        insta::assert_snapshot!("logs_paused", frame);
    }

    #[tokio::test]
    async fn a_narrow_terminal_keeps_both_panes_readable() {
        let mut harness = logs_at(&mixed(), 60, 20).await;
        let frame = harness.render();
        assert!(frame.contains("Sources"), "{frame}");
        assert!(frame.contains("ERROR"), "{frame}");
        insta::assert_snapshot!("logs_narrow", frame);
    }

    /// D4: the border is the only place the follow state and the level show, so a narrow
    /// pane gives up the source name (the list's cursor names it) before either.
    #[tokio::test]
    async fn a_narrow_border_keeps_the_state_and_the_level() {
        let mut harness = logs_at(&mixed(), 60, 20).await;
        for _ in 0..3 {
            harness.key("j");
        }
        let frame = harness.render();
        let top = frame.lines().nth(1).expect("the pane borders");
        assert!(top.contains("following"), "{top}");
        assert!(top.contains("min trace"), "{top}");
        harness.key("F");
        let frame = harness.render();
        let top = frame.lines().nth(1).expect("the pane borders");
        assert!(top.contains("paused"), "{top}");
        assert!(top.contains("min trace"), "{top}");
        assert!(
            top.contains("\u{b7} orc"),
            "the source keeps what room is left: {top}"
        );
    }

    #[tokio::test]
    async fn v_cycles_the_minimum_level_through_every_level() {
        let mut harness = logs(&mixed()).await;
        for level in ["debug", "info", "warn", "error", "trace"] {
            harness.key("v");
            let frame = harness.render();
            assert!(frame.contains(&format!("min {level} ")), "{level}: {frame}");
        }
    }

    #[tokio::test]
    async fn a_step_up_in_the_lines_pauses_follow() {
        let buffer = many(60);
        let mut harness = logs(&buffer).await;
        assert_eq!(shown(&harness.render(), 59..60), [59], "the newest line");
        harness.key("tab");
        harness.key("k");
        let frame = harness.render();
        assert!(frame.contains("paused"), "{frame}");
        assert!(shown(&frame, 59..60).is_empty(), "one line up: {frame}");

        let mut harness = logs(&buffer).await;
        harness.render();
        harness.mouse(MouseEventKind::ScrollUp, 60, 10);
        let frame = harness.render();
        assert!(frame.contains("paused"), "the wheel too: {frame}");
    }

    #[tokio::test]
    async fn capital_g_resumes_follow_at_the_newest_line() {
        let mut harness = logs(&many(60)).await;
        harness.render();
        harness.key("tab");
        for _ in 0..5 {
            harness.key("k");
        }
        assert!(shown(&harness.render(), 59..60).is_empty());
        harness.key("G");
        let frame = harness.render();
        assert!(frame.contains("following"), "{frame}");
        assert_eq!(shown(&frame, 59..60), [59], "{frame}");
    }

    #[tokio::test]
    async fn capital_f_toggles_follow() {
        let mut harness = logs(&mixed()).await;
        harness.key("F");
        assert!(harness.render().contains("paused"));
        harness.key("F");
        assert!(harness.render().contains("following"));
    }

    #[tokio::test]
    async fn new_lines_arrive_on_refresh_and_stay_in_view_only_while_following() {
        let buffer = many(60);
        let mut harness = logs(&buffer).await;
        harness.render();
        push_lines(&buffer, 60..61);
        assert!(
            shown(&harness.render(), 60..61).is_empty(),
            "not before refresh"
        );
        refresh(&mut harness);
        assert_eq!(shown(&harness.render(), 60..61), [60], "following: in view");

        harness.key("F");
        let paused = shown(&harness.render(), 0..61);
        push_lines(&buffer, 61..62);
        refresh(&mut harness);
        let frame = harness.render();
        assert_eq!(shown(&frame, 0..62), paused, "paused: the view stays put");
        assert!(frame.contains(&source_row("all", 62, 0)), "{frame}");
    }

    /// MOD-122 R1: switching to the tab reads the buffer at once, so it never shows lines up to
    /// a second old; the refresh carries on from there.
    #[tokio::test]
    async fn switching_to_the_tab_reads_the_buffer_at_once() {
        let mut harness = Harness::demo();
        crate::app::register_all(harness.app());
        harness.settle().await;
        let buffer = harness.app().logging.buffer().clone();
        push_lines(&buffer, 0..3);

        harness.key("6");
        let frame = harness.render();

        assert!(
            frame.contains("Sources"),
            "the Logs tab is in front: {frame}"
        );
        assert_eq!(shown(&frame, 0..3), [0, 1, 2], "{frame}");
    }

    #[tokio::test]
    async fn a_paused_view_keeps_its_line_when_old_lines_drop() {
        let buffer = many(CAPACITY);
        let mut harness = logs(&buffer).await;
        harness.render();
        harness.key("tab");
        for _ in 0..3 {
            harness.key("k");
        }
        let paused = shown(&harness.render(), CAPACITY - 40..CAPACITY);
        assert!(!paused.is_empty());
        push_lines(&buffer, CAPACITY..CAPACITY + 5);
        refresh(&mut harness);
        let frame = harness.render();
        assert_eq!(
            shown(&frame, CAPACITY - 40..CAPACITY + 5),
            paused,
            "{frame}"
        );
        assert!(frame.contains(&source_row("all", CAPACITY, 0)), "{frame}");
    }

    #[tokio::test]
    async fn a_source_change_keeps_the_follow_state() {
        let mut harness = logs(&mixed()).await;
        harness.key("j");
        assert!(
            harness
                .render()
                .contains("ui \u{b7} min trace \u{b7} following")
        );
        harness.key("F");
        harness.key("j");
        let frame = harness.render();
        assert!(
            frame.contains("store \u{b7} min trace \u{b7} paused"),
            "{frame}"
        );
    }

    #[tokio::test]
    async fn location_and_restore_return_to_the_source() {
        let bench = SectionBench::new().await;
        let mut tab = LogsTab::new(mixed());
        assert_eq!(tab.location().cursor.as_deref(), Some("source:all"));
        tab.selected = Some(Source::Store);
        let loc = tab.location();
        assert_eq!(loc.cursor.as_deref(), Some("source:store"));
        tab.selected = None;
        assert!(tab.restore(&loc, &mut bench.ctx()));
        assert_eq!(tab.selected, Some(Source::Store));
        let all = TabLoc {
            cursor: Some("source:all".to_owned()),
            ..TabLoc::default()
        };
        assert!(tab.restore(&all, &mut bench.ctx()));
        assert_eq!(tab.selected, None);
        let unknown = TabLoc {
            cursor: Some("source:nowhere".to_owned()),
            ..TabLoc::default()
        };
        assert!(!tab.restore(&unknown, &mut bench.ctx()));
        assert!(!tab.restore(&TabLoc::default(), &mut bench.ctx()));
        assert_eq!(tab.selected, None, "a refused restore moves nothing");
    }
}
