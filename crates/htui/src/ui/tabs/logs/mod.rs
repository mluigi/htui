//! Logs tab (MOD-122 D4, `R-TUI-15`): htui's own log as it runs, and its headless workers'.
//!
//! The sources on the left (`all`, then each [`Source`] in the requirement's order, `other` only
//! while it holds a line, then one `worker · <box name>` row per headless worker box with stored
//! lines, M2 D10), with counts of the lines the tab still holds and of their warnings; the
//! selected source's lines on the right, one row each, coloured by level with the level word
//! always printed, so monochrome loses nothing. `all` keeps its M1 meaning, the process's lines
//! only: a worker's lines carry its own clock, and one merged timeline would imply an order the
//! clocks cannot give.
//!
//! The tab reads a [`LogBuffer`] and never logs: [`Tab::on_activate`] and [`Tab::on_refresh`]
//! copy the lines newer than the last sequence it saw (D2), so the lines pane is current when the
//! tab comes to the front and moves once a second after, as the shell's refresh does. The same
//! two hooks ask the store for the workers' lines ([`StoreRequest::WorkerLogs`]), one read at a
//! time; a refused read is absorbed, so a database that is down does not fill the status line
//! once a second, and the lines already read stay. The lines pane follows the newest line until a
//! step up pauses it; a paused view is anchored on the sequence of its top line (blueprint B4),
//! so lines dropping off the ring, a new minimum level or another source never move what a
//! paused reader sees. A worker line's sequence is its row id, another space than the process's:
//! crossing between a local and a worker source keeps the follow state, and a paused view lands
//! on the newest page of the other space (M2 D10). `logs.bottom` jumps to the newest line and
//! follows again, as `chat.bottom` does; `logs.follow` toggles following.

mod workers;

use core::cell::Cell;
use core::fmt;
use std::collections::VecDeque;

use crossterm::event::KeyEvent;
use htui_core::model::{BoxId, Scope};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tracing::Level;

use self::workers::{WorkerKey, Workers};
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

/// How a [`TabLoc`] cursor names a worker source after [`SOURCE_CURSOR`]:
/// `source:worker:<box id>`, or `source:worker:removed:<name>` for a deleted box. The local
/// `worker` source is `source:worker`, with no colon after it.
const WORKER_CURSOR: &str = "worker:";

/// The deleted box's half of a [`WORKER_CURSOR`].
const REMOVED_CURSOR: &str = "removed:";

/// The `all` row's name, beside the [`Source`] names.
const ALL: &str = "all";

/// [`StoreRequest::WorkerLogs`]'s name, whose refusal the tab absorbs.
const WORKER_LOGS: &str = "worker_logs";

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

/// An empty worker source's next step (M2 D10, D11): what a box ships is its own setting.
const NO_WORKER_LINES_NEXT: &str =
    "Settings \u{203a} Boxes \u{203a} Worker log level sets what it ships";

/// The level filter's next step when `logs.level` has no key.
const LOWER_THE_LEVEL: &str = "lower the minimum level";

/// The narrowest lines pane (inner cells) that keeps the source column under `all`.
const SOURCE_WIDTH: u16 = 48;

/// The narrowest lines pane (inner cells) that keeps the time column.
const TIME_WIDTH: u16 = 36;

/// A worker source with nothing to show: the lines the tab reads for it before any arrive.
static NO_LINES: VecDeque<LogLine> = VecDeque::new();

/// Where the lines pane is (blueprint B4).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Scroll {
    /// Following: the last page, whatever arrives.
    #[default]
    Tail,
    /// Paused with this sequence's line (or the next one still shown) on top.
    At(u64),
}

/// The selected source (M2 D10).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pick {
    /// The process's every line.
    All,
    /// One of the process's sources.
    Local(Source),
    /// One headless worker box's stored lines.
    Worker(WorkerKey),
}

impl Pick {
    /// Whether the pick reads a worker's lines, whose sequences are row ids.
    const fn is_worker(&self) -> bool {
        matches!(self, Self::Worker(_))
    }
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
    /// The headless workers' lines read so far (M2 D10).
    workers: Workers,
    /// Whether a [`StoreRequest::WorkerLogs`] is out: a second one would make the shell drop the
    /// first one's reply (blueprint F11), so none goes until it is answered.
    in_flight: bool,
    /// The selected source.
    selected: Pick,
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
            .field("in_flight", &self.in_flight)
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
            workers: Workers::default(),
            in_flight: false,
            selected: Pick::All,
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

    /// Asks the store for the workers' lines past the tab's cursor, unless a read is out
    /// (blueprint F11).
    fn request_workers(&mut self, ctx: &Ctx<'_>) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        ctx.request(StoreRequest::WorkerLogs {
            after: self.workers.after(),
        });
    }

    /// D3: `other` is listed only while it holds a line, so a selected `other` that holds none
    /// hands the cursor to `all`; so does a worker source that is no longer listed.
    fn keep_selection_listed(&mut self) {
        let listed = match &self.selected {
            Pick::All => true,
            Pick::Local(source) => {
                *source != Source::Other || self.counts.of(Source::Other).lines > 0
            }
            Pick::Worker(key) => self.workers.get(key).is_some(),
        };
        if !listed {
            self.select(Pick::All);
        }
    }

    /// Selects `pick`. Crossing between a local and a worker source keeps the follow state, but
    /// a paused anchor means nothing in the other sequence space, so the view lands on the
    /// newest page there, still paused (M2 D10).
    fn select(&mut self, pick: Pick) {
        let crossed = self.selected.is_worker() != pick.is_worker();
        self.selected = pick;
        if crossed && matches!(self.scroll, Scroll::At(_)) {
            let visible = self.visible();
            let top = visible.len().saturating_sub(self.height.get());
            let anchor = visible.get(top).map(|line| line.seq);
            self.scroll = Scroll::At(anchor.unwrap_or_else(|| self.space_next()));
        }
    }

    /// The sequence one past the newest line in the selected source's space.
    fn space_next(&self) -> u64 {
        if self.selected.is_worker() {
            self.workers.next()
        } else {
            self.next
        }
    }

    /// The sources list: `all`, then every source in the requirement's order, `other` only
    /// while it holds a line (D3), then the worker sources (M2 D10).
    fn rows(&self) -> Vec<Pick> {
        let mut rows = vec![Pick::All];
        rows.extend(
            Source::ALL
                .into_iter()
                .filter(|&source| source != Source::Other || self.counts.of(source).lines > 0)
                .map(Pick::Local),
        );
        rows.extend(self.workers.keys().into_iter().map(Pick::Worker));
        rows
    }

    /// The lines the selected source draws from: the process's, or one worker's.
    fn pool(&self) -> &VecDeque<LogLine> {
        match &self.selected {
            Pick::Worker(key) => self
                .workers
                .get(key)
                .map_or(&NO_LINES, |source| source.lines()),
            Pick::All | Pick::Local(_) => &self.lines,
        }
    }

    /// Whether `line` is from the selected source (or any, under `all` and a worker's).
    fn is_selected_source(&self, line: &LogLine) -> bool {
        match self.selected {
            Pick::Local(source) => line.source == source,
            Pick::All | Pick::Worker(_) => true,
        }
    }

    /// The selected source's name: `all`, a [`Source`]'s, or a worker's label.
    fn selected_name(&self) -> String {
        match &self.selected {
            Pick::All => ALL.to_owned(),
            Pick::Local(source) => source.name().to_owned(),
            Pick::Worker(key) => self.workers.label(key),
        }
    }

    /// The lines the lines pane shows: the selected source's at the minimum level or above
    /// (`tracing` orders `ERROR` < … < `TRACE`).
    fn visible(&self) -> Vec<&LogLine> {
        self.pool()
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
                Scroll::At(visible.get(top).map_or(self.space_next(), |line| line.seq))
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
        let pick = rows.into_iter().nth(cursor).unwrap_or(Pick::All);
        self.select(pick);
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
        // `list::render` keeps one cell for its cursor gutter.
        let width = usize::from(inner.width.saturating_sub(1));
        let list_rows: Vec<Row<'static>> = rows
            .iter()
            .map(|row| {
                let line = match row {
                    Pick::All => source_line(ALL, self.counts.all(), theme),
                    Pick::Local(source) => {
                        source_line(source.name(), self.counts.of(*source), theme)
                    }
                    Pick::Worker(key) => {
                        let counts = self
                            .workers
                            .get(key)
                            .map_or_else(Counts::default, |source| source.counts());
                        worker_line(&self.workers.label(key), counts, width, theme)
                    }
                };
                Row {
                    line,
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
        let source = self.selected_name();
        let level = self.min_level.as_str().to_ascii_lowercase();
        let state = match self.scroll {
            Scroll::Tail => "following",
            Scroll::At(_) => "paused",
        };
        if visible.is_empty() {
            let (what, next) = self.empty_state(&source, &level, ctx);
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
    /// level, or nothing from this source. A worker source with no line points at its box's
    /// shipping level (M2 D11).
    fn empty_state(&self, source: &str, level: &str, ctx: &Ctx<'_>) -> (String, String) {
        let worker = self.selected.is_worker();
        if worker && self.pool().is_empty() {
            return (
                format!("No lines from {source} yet."),
                NO_WORKER_LINES_NEXT.to_owned(),
            );
        }
        if !worker && self.lines.is_empty() {
            return ("No log lines yet.".to_owned(), NO_LINES_NEXT.to_owned());
        }
        if self.pool().iter().any(|line| self.is_selected_source(line)) {
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

    /// One line, one row (blueprint B6): time, level word, source under `all` (and under a
    /// worker, its target's source, M2 D10), text, clipped to `width`. A deliberate deviation
    /// from B6's fixed shape: a pane narrower than [`SOURCE_WIDTH`] gives up the source column
    /// and one narrower than [`TIME_WIDTH`] the time too, so the text keeps room (at 60 columns
    /// the pane holds 28 cells, exactly the full prefix). The level word always stays (D4).
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
        if !matches!(self.selected, Pick::Local(_)) && width >= SOURCE_WIDTH {
            spans.push(Span::styled(
                format!("{:<12} ", line.source.name()),
                theme.dim,
            ));
        }
        spans.push(Span::styled(line.text.clone(), style));
        cells::clip_line(Line::from(spans), width)
    }

    /// The selected source as a [`TabLoc`] cursor's tail: `all`, a [`Source`]'s name,
    /// `worker:<box id>` or `worker:removed:<name>`.
    fn cursor_name(&self) -> String {
        match &self.selected {
            Pick::All => ALL.to_owned(),
            Pick::Local(source) => source.name().to_owned(),
            Pick::Worker(WorkerKey::Box(id)) => format!("{WORKER_CURSOR}{id}"),
            Pick::Worker(WorkerKey::Removed(name)) => {
                format!("{WORKER_CURSOR}{REMOVED_CURSOR}{name}")
            }
        }
    }

    /// The pick a [`cursor_name`](Self::cursor_name) names, or `None` for no source at all.
    fn pick_named(name: &str) -> Option<Pick> {
        if name == ALL {
            return Some(Pick::All);
        }
        let Some(worker) = name.strip_prefix(WORKER_CURSOR) else {
            return Source::from_name(name).map(Pick::Local);
        };
        let key = match worker.strip_prefix(REMOVED_CURSOR) {
            Some(removed) => WorkerKey::Removed(removed.to_owned()),
            None => WorkerKey::Box(worker.parse::<BoxId>().ok()?),
        };
        Some(Pick::Worker(key))
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

/// A worker's sources-list row in `width` cells (M2 D10): its label, clipped so the counts
/// always show, then `lines · ⚠ warnings`, the warnings in the warning role while any.
fn worker_line(label: &str, counts: Counts, width: usize, theme: &Theme) -> Line<'static> {
    let warn = if counts.warnings > 0 {
        theme.warning
    } else {
        theme.dim
    };
    let tail = format!("\u{b7} \u{26a0} {}", counts.warnings);
    let lines = format!("{} ", counts.lines);
    let room = width.saturating_sub(cells::cell_width(&tail) + cells::cell_width(&lines) + 1);
    Line::from(vec![
        Span::styled(format!("{} {lines}", cells::fit(label, room)), theme.base),
        Span::styled(tail, warn),
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

    /// The log is the process's, and the workers' read goes out from [`Tab::on_activate`] under
    /// the in-flight flag (blueprint F11): nothing to ask here.
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

    /// The workers' lines merge into what the tab holds (an empty answer adds nothing); a
    /// refusal leaves them as they are. Either way the next read may go.
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::WorkerLogs(rows) => {
                self.in_flight = false;
                self.workers.merge(rows);
                self.keep_selection_listed();
            }
            StoreReply::Failed { request, .. } if *request == WORKER_LOGS => {
                self.in_flight = false;
            }
            _ => {}
        }
    }

    /// A refused worker read is absorbed (M2 D10): the database being down would otherwise put
    /// it on the status line once a second, and the lines already read stay on screen.
    fn absorbs_failure(&self, request: &str) -> bool {
        request == WORKER_LOGS
    }

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

    /// MOD-122 D4: new lines, once a second while the tab is in front; the workers' too (M2).
    fn on_refresh(&mut self, ctx: &mut Ctx<'_>) {
        self.pull();
        self.request_workers(ctx);
    }

    /// MOD-122 R1: new lines the moment the tab comes to the front, not a refresh later; the
    /// workers' read goes out at once too (M2 D10).
    fn on_activate(&mut self, ctx: &mut Ctx<'_>) {
        self.pull();
        self.request_workers(ctx);
    }

    fn panes(&self) -> Panes {
        Panes::ListDetail(LOGS_LAYOUT)
    }

    /// The selected source, `source:{name}`; a worker's is `source:worker:{box id}`.
    fn location(&self) -> TabLoc {
        TabLoc {
            cursor: Some(format!("{SOURCE_CURSOR}:{}", self.cursor_name())),
            ..TabLoc::default()
        }
    }

    /// Back to a [`location`](Tab::location)'s source (an empty `other`, or a worker box with no
    /// line, lands on `all`, D3, M2 D10); the follow state stays as it is.
    fn restore(&mut self, loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
        let Some(pick) = loc
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.strip_prefix(SOURCE_CURSOR))
            .and_then(|rest| rest.strip_prefix(':'))
            .and_then(Self::pick_named)
        else {
            return false;
        };
        self.select(pick);
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
        let mut expected: Vec<Pick> = vec![Pick::All];
        expected.extend(
            Source::ALL
                .into_iter()
                .filter(|source| *source != Source::Other)
                .map(Pick::Local),
        );
        assert_eq!(tab.rows(), expected, "no third-party line: no `other`");
        buffer.push(at(2), Level::INFO, "hyper::client", "two".into());
        tab.pull();
        assert_eq!(tab.rows().last(), Some(&Pick::Local(Source::Other)));
    }

    /// D3: `other` is listed only while it holds a line, so a selected `other` whose lines
    /// leave the ring hands the cursor to `all`.
    #[test]
    fn a_selected_other_that_empties_hands_the_cursor_to_all() {
        let buffer = LogBuffer::new();
        buffer.push(at(1), Level::WARN, "hyper::client", "reset".into());
        let mut tab = LogsTab::new(buffer.clone());
        tab.move_source(Act::ListUp);
        while tab.selected != Pick::Local(Source::Other) {
            tab.move_source(Act::ListDown);
        }
        push_lines(&buffer, 0..CAPACITY);
        tab.pull();
        assert_eq!(tab.selected, Pick::All, "the cursor is on `all`");
        assert!(
            !tab.rows().contains(&Pick::Local(Source::Other)),
            "no `other` row"
        );
    }

    /// D3: a location on `other` while it holds no line lands on `all`, the row it had.
    #[tokio::test]
    async fn restoring_an_empty_other_lands_on_all() {
        let bench = SectionBench::new().await;
        let mut tab = LogsTab::new(many(3));
        tab.selected = Pick::Local(Source::Store);
        let other = TabLoc {
            cursor: Some("source:other".to_owned()),
            ..TabLoc::default()
        };
        assert!(tab.restore(&other, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::All);
        assert!(!tab.rows().contains(&Pick::Local(Source::Other)));
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
        tab.selected = Pick::Local(Source::Store);
        let loc = tab.location();
        assert_eq!(loc.cursor.as_deref(), Some("source:store"));
        tab.selected = Pick::All;
        assert!(tab.restore(&loc, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::Local(Source::Store));
        let all = TabLoc {
            cursor: Some("source:all".to_owned()),
            ..TabLoc::default()
        };
        assert!(tab.restore(&all, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::All);
        let unknown = TabLoc {
            cursor: Some("source:nowhere".to_owned()),
            ..TabLoc::default()
        };
        assert!(!tab.restore(&unknown, &mut bench.ctx()));
        assert!(!tab.restore(&TabLoc::default(), &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::All, "a refused restore moves nothing");
    }

    // -- M2 D10: worker sources ---------------------------------------------------------------

    use htui_core::model::{WorkerLogLevel, WorkerLogLine};

    use super::workers::tests::{BUILD, GPU, row};
    use crate::store_worker::StoreRequest;

    /// `BUILD-01`'s five lines (a warning and an error among them) and `GPU-02`'s two.
    fn two_workers() -> Vec<WorkerLogLine> {
        let build = Some(BUILD);
        let gpu = Some(GPU);
        vec![
            row(
                1,
                build,
                "BUILD-01",
                WorkerLogLevel::Info,
                "htui_worker::walk",
                "walk started run=7",
            ),
            row(
                2,
                build,
                "BUILD-01",
                WorkerLogLevel::Debug,
                "htui_store::cache",
                "cache refreshed rows=3",
            ),
            row(
                3,
                build,
                "BUILD-01",
                WorkerLogLevel::Warn,
                "htui_store::pg",
                "slow statement ms=812",
            ),
            row(
                4,
                gpu,
                "GPU-02",
                WorkerLogLevel::Info,
                "htui_worker",
                "worker started",
            ),
            row(
                5,
                build,
                "BUILD-01",
                WorkerLogLevel::Error,
                "htui_agent::acp",
                "adapter exited code=1",
            ),
            row(
                6,
                build,
                "BUILD-01",
                WorkerLogLevel::Info,
                "hyper::client",
                "connection reused",
            ),
            row(
                7,
                gpu,
                "GPU-02",
                WorkerLogLevel::Warn,
                "htui_worker::walk",
                "walk retried",
            ),
        ]
    }

    /// `count` `info` lines from `BUILD-01`, ids from `first`, `wline {id}` each.
    fn build_lines(first: i64, count: i64) -> Vec<WorkerLogLine> {
        (first..first + count)
            .map(|id| {
                row(
                    id,
                    Some(BUILD),
                    "BUILD-01",
                    WorkerLogLevel::Info,
                    "htui_worker",
                    &format!("wline {id}"),
                )
            })
            .collect()
    }

    /// Which of `wline {id}` for `id` in `range` the frame shows.
    fn shown_worker(frame: &str, range: std::ops::Range<i64>) -> Vec<i64> {
        range
            .filter(|id| frame.contains(&format!("wline {id} ")))
            .collect()
    }

    /// The tab over `buffer` holding `rows` already read.
    fn with_workers(buffer: LogBuffer, rows: &[WorkerLogLine]) -> LogsTab {
        let mut tab = LogsTab::new(buffer);
        tab.workers.merge(rows);
        tab
    }

    /// `tab` alone on a `width` x `height` frame over an empty store, whose worker read adds
    /// nothing to what the tab holds.
    async fn tab_at(tab: LogsTab, width: u16, height: u16) -> Harness {
        let mut harness = Harness::empty().size(width, height).with_tab(Box::new(tab));
        harness.settle().await;
        harness
    }

    /// The row index of `BUILD-01` under `mixed()`: `all`, the eight local sources, then it.
    const BUILD_ROW: usize = 9;

    /// A worker's list row as the tab writes it at 100 columns (27 cells after the gutter).
    fn worker_row(label: &str, lines: usize, warnings: usize) -> String {
        let tail = format!("{lines} \u{b7} \u{26a0} {warnings}");
        let room = 27 - tail.chars().count() - 1;
        format!("{} {tail}", cells::fit(label, room))
    }

    #[tokio::test]
    async fn worker_boxes_are_listed_after_the_local_sources_with_their_counts() {
        let mut harness = tab_at(with_workers(mixed(), &two_workers()), 100, 30).await;
        let frame = harness.render();
        let build = worker_row("worker \u{b7} BUILD-01", 5, 2);
        let gpu = worker_row("worker \u{b7} GPU-02", 2, 1);
        for row in [
            &build,
            &gpu,
            &source_row("all", 7, 3),
            &source_row("other", 1, 1),
        ] {
            assert!(frame.contains(row.as_str()), "`{row}` in {frame}");
        }
        assert!(
            frame.find(&source_row("other", 1, 1)) < frame.find(&build),
            "the workers follow the local sources"
        );
        assert!(frame.find(&build) < frame.find(&gpu), "by name");
        insta::assert_snapshot!("logs_workers", frame);
    }

    /// D10: `all` keeps its M1 meaning, the process's lines only.
    #[tokio::test]
    async fn all_keeps_the_process_s_lines_only() {
        let mut harness = tab_at(with_workers(mixed(), &two_workers()), 100, 30).await;
        let frame = harness.render();
        assert!(frame.contains("shell started"), "{frame}");
        assert!(!frame.contains("walk started"), "{frame}");
    }

    #[tokio::test]
    async fn a_deleted_box_is_listed_as_removed() {
        let mut rows = two_workers();
        rows.push(row(
            8,
            None,
            "OLD-RIG",
            WorkerLogLevel::Error,
            "htui_worker",
            "walk failed",
        ));
        let mut harness = tab_at(with_workers(mixed(), &rows), 100, 30).await;
        for _ in 0..BUILD_ROW + 2 {
            harness.key("j");
        }
        let frame = harness.render();
        assert!(
            frame.contains("worker \u{b7} OLD-RIG (removed) \u{b7} min trace \u{b7} following"),
            "{frame}"
        );
        assert!(frame.contains("walk failed"), "{frame}");
        assert!(frame.contains(&worker_row("worker \u{b7} OLD-RIG (removed)", 1, 1)));
        insta::assert_snapshot!("logs_worker_removed", frame);
    }

    #[tokio::test]
    async fn a_worker_row_shows_its_lines_by_level_with_the_target_s_source() {
        let mut harness = tab_at(with_workers(mixed(), &two_workers()), 100, 30).await;
        for _ in 0..BUILD_ROW {
            harness.key("j");
        }
        harness.render();
        harness.key("tab");
        let frame = harness.render();
        assert!(
            frame.contains("worker \u{b7} BUILD-01 \u{b7} min trace \u{b7} following"),
            "{frame}"
        );
        assert!(!frame.contains("shell started"), "no process line: {frame}");
        assert!(
            !frame.contains("worker started"),
            "no other box's line: {frame}"
        );
        for line in [
            "10:15:01 INFO  worker       walk started run=7",
            "10:15:03 WARN  store        slow statement ms=812",
            "10:15:05 ERROR agents       adapter exited code=1",
            "10:15:06 INFO  other        connection reused",
        ] {
            assert!(frame.contains(line), "`{line}` in {frame}");
        }
        let theme = Theme::default();
        let buffer = harness.render_buffer();
        let fg = |style: Style| style.fg.unwrap_or(Color::Reset);
        for (word, role) in [
            ("adapter exited", theme.error),
            ("slow statement", theme.warning),
            ("walk started", theme.base),
            ("cache refreshed", theme.dim),
            ("10:15:01", theme.dim),
        ] {
            assert_eq!(fg(style_of(&buffer, word)), fg(role), "{word}");
        }
        insta::assert_snapshot!("logs_worker_lines", frame);
    }

    #[tokio::test]
    async fn a_narrow_terminal_keeps_a_worker_source_readable() {
        let mut harness = tab_at(with_workers(mixed(), &two_workers()), 60, 20).await;
        for _ in 0..BUILD_ROW {
            harness.key("j");
        }
        let frame = harness.render();
        assert!(frame.contains("ERROR adapter exited"), "{frame}");
        assert!(frame.contains("worker \u{b7} BUILD"), "{frame}");
        insta::assert_snapshot!("logs_worker_narrow", frame);
    }

    /// An empty worker source names the box's shipping level as the next step (D10, D11).
    #[tokio::test]
    async fn an_empty_worker_source_points_at_the_box_s_log_level() {
        let bench = SectionBench::new().await;
        let mut tab = LogsTab::new(mixed());
        tab.selected = Pick::Worker(WorkerKey::Box(GPU));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 12))
            .expect("a test terminal");
        let ctx = bench.ctx();
        term.draw(|frame| tab.render(frame, frame.area(), &ctx))
            .expect("a frame");
        let frame = text_of(term.backend().buffer());
        assert!(
            frame.contains("No lines from worker \u{b7} 00006b02 yet."),
            "{frame}"
        );
        assert!(
            frame.contains("Settings \u{203a} Boxes \u{203a} Worker log level sets what it ships"),
            "{frame}"
        );
        insta::assert_snapshot!("logs_worker_empty", frame);
    }

    /// A buffer as text, one line per row, trailing blanks trimmed.
    fn text_of(buffer: &Buffer) -> String {
        let area = buffer.area;
        (area.top()..area.bottom())
            .map(|y| {
                let row: String = (area.left()..area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn v_f_and_capital_g_work_on_a_worker_source() {
        let mut harness = tab_at(with_workers(mixed(), &two_workers()), 100, 30).await;
        for _ in 0..BUILD_ROW {
            harness.key("j");
        }
        for _ in 0..4 {
            harness.key("v");
        }
        let frame = harness.render();
        assert!(frame.contains("min error"), "{frame}");
        assert!(frame.contains("adapter exited"), "{frame}");
        assert!(!frame.contains("slow statement"), "{frame}");
        harness.key("tab");
        harness.key("F");
        assert!(
            harness
                .render()
                .contains("BUILD-01 \u{b7} min error \u{b7} paused")
        );
        harness.key("G");
        assert!(
            harness
                .render()
                .contains("BUILD-01 \u{b7} min error \u{b7} following")
        );
    }

    #[tokio::test]
    async fn a_step_up_in_a_worker_s_lines_pauses_and_g_resumes() {
        let tab = with_workers(LogBuffer::new(), &build_lines(1, 60));
        let mut harness = tab_at(tab, 100, 30).await;
        // No `other` line: `BUILD-01` is the row after `secrets`.
        for _ in 0..BUILD_ROW - 1 {
            harness.key("j");
        }
        assert_eq!(
            shown_worker(&harness.render(), 60..61),
            [60],
            "the newest line"
        );
        harness.key("tab");
        harness.key("k");
        let frame = harness.render();
        assert!(frame.contains("paused"), "{frame}");
        assert!(
            shown_worker(&frame, 60..61).is_empty(),
            "one line up: {frame}"
        );
        harness.key("G");
        let frame = harness.render();
        assert!(frame.contains("following"), "{frame}");
        assert_eq!(shown_worker(&frame, 60..61), [60], "{frame}");
    }

    /// D10: crossing between the process's sources and a worker's keeps the follow state; a
    /// paused anchor lands on the other space's newest page, still paused. Within one space the
    /// anchor holds, as M1's B4 says.
    #[test]
    fn crossing_local_and_worker_keeps_the_mode_and_lands_on_the_newest_page() {
        let mut rows = build_lines(1, 60);
        rows.extend((61..=70).map(|id| {
            row(
                id,
                Some(GPU),
                "GPU-02",
                WorkerLogLevel::Info,
                "htui_worker",
                &format!("gpu {id}"),
            )
        }));
        let mut tab = with_workers(many(60), &rows);
        tab.height.set(10);
        tab.scroll = Scroll::At(5);

        tab.select(Pick::Local(Source::Ui));
        assert_eq!(
            tab.scroll,
            Scroll::At(5),
            "within the local space the anchor holds"
        );

        tab.select(Pick::Worker(WorkerKey::Box(BUILD)));
        assert_eq!(tab.scroll, Scroll::At(51), "BUILD-01's newest page, paused");

        tab.scroll = Scroll::At(20);
        tab.select(Pick::Worker(WorkerKey::Box(GPU)));
        assert_eq!(tab.scroll, Scroll::At(20), "worker to worker: one id space");

        tab.select(Pick::All);
        assert_eq!(
            tab.scroll,
            Scroll::At(50),
            "the process's newest page, paused"
        );

        tab.scroll = Scroll::Tail;
        tab.select(Pick::Worker(WorkerKey::Box(BUILD)));
        assert_eq!(tab.scroll, Scroll::Tail, "following stays following");
    }

    #[tokio::test]
    async fn crossing_into_a_worker_while_paused_shows_its_newest_lines_paused() {
        let tab = with_workers(many(60), &build_lines(1, 60));
        let mut harness = tab_at(tab, 100, 30).await;
        harness.render();
        harness.key("F");
        harness.key("tab");
        for _ in 0..20 {
            harness.key("k");
        }
        harness.key("tab");
        for _ in 0..BUILD_ROW - 1 {
            harness.key("j");
        }
        let frame = harness.render();
        assert!(
            frame.contains("BUILD-01 \u{b7} min trace \u{b7} paused"),
            "{frame}"
        );
        assert_eq!(
            shown_worker(&frame, 60..61),
            [60],
            "the newest page: {frame}"
        );
    }

    /// Blueprint F11: one worker read at a time, so the shell never drops a reply as stale; the
    /// reply re-reads a margin and adds no line twice.
    #[tokio::test]
    async fn one_worker_read_is_out_at_a_time_and_a_reply_adds_no_line_twice() {
        let bench = SectionBench::new().await;
        let mut tab = LogsTab::new(LogBuffer::new());
        let reads = |actions: Vec<Action>| -> Vec<Option<i64>> {
            actions
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(StoreRequest::WorkerLogs { after }) => Some(after),
                    _ => None,
                })
                .collect()
        };
        tab.on_activate(&mut bench.ctx());
        tab.on_refresh(&mut bench.ctx());
        tab.on_refresh(&mut bench.ctx());
        assert_eq!(reads(bench.drained()), [None], "one read, the newest page");

        tab.on_reply(
            &StoreReply::WorkerLogs(build_lines(1, 600)),
            &mut bench.ctx(),
        );
        tab.on_refresh(&mut bench.ctx());
        assert_eq!(
            reads(bench.drained()),
            [Some(100)],
            "re-read 500 below the highest id"
        );

        tab.on_reply(
            &StoreReply::WorkerLogs(build_lines(101, 510)),
            &mut bench.ctx(),
        );
        let source = tab.workers.get(&WorkerKey::Box(BUILD)).expect("listed");
        assert_eq!(source.counts().lines, 610, "the overlap is not added twice");
        tab.on_refresh(&mut bench.ctx());
        assert_eq!(reads(bench.drained()), [Some(110)]);
    }

    /// D10: a refused read is absorbed and its lines stay; the next read may go.
    #[tokio::test]
    async fn a_failed_worker_read_keeps_the_lines_and_says_nothing() {
        let bench = SectionBench::new().await;
        let mut tab = with_workers(LogBuffer::new(), &two_workers());
        assert!(tab.absorbs_failure(StoreRequest::WorkerLogs { after: None }.name()));
        assert!(!tab.absorbs_failure("workspaces"), "only its own read");
        tab.on_refresh(&mut bench.ctx());
        let _ = bench.drained();
        tab.on_reply(
            &StoreReply::Failed {
                request: "worker_logs",
                message: "database unreachable".to_owned(),
            },
            &mut bench.ctx(),
        );
        assert!(bench.errors().is_empty(), "the tab writes no status line");
        assert!(
            tab.workers.get(&WorkerKey::Box(BUILD)).is_some(),
            "the lines stay"
        );
        tab.on_refresh(&mut bench.ctx());
        assert_eq!(bench.drained().len(), 1, "the next read goes");
    }

    /// D10 through the shell: a refused read over a database nobody answers stays off the
    /// status line, and the rows already read stay on screen.
    #[tokio::test]
    async fn a_refused_worker_read_stays_off_the_status_line() {
        let _keyring = htui_store::testkit::mock_keyring().await;
        let identity = htui_store::Identity {
            box_id: BoxId::new(),
            hostname: "HTUI-TEST".to_owned(),
        };
        let pg = htui_store::PgStore::lazy(
            "postgres://nobody:nothing@127.0.0.1:1/none",
            &identity,
            std::time::Duration::from_millis(250),
        )
        .expect("a lazy pool opens no socket");
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "logs-worker-refused", 1)
            .await
            .expect("open a throwaway mirror");
        let backend = htui_store::Backend::Online {
            pg,
            cache: cache.clone(),
        };
        let tab = with_workers(LogBuffer::new(), &two_workers());
        let mut harness = Harness::over_backend(backend).with_tab(Box::new(tab));
        harness.settle().await;
        harness.app().status = None;
        refresh(&mut harness);
        harness.settle().await;
        let status = harness.app().status.clone();
        assert!(
            status
                .as_deref()
                .is_none_or(|line| !line.contains("worker_logs")),
            "{status:?}"
        );
        for _ in 0..BUILD_ROW - 1 {
            harness.key("j");
        }
        let frame = harness.render();
        assert!(frame.contains("walk started run=7"), "{frame}");
        cache.close().await;
    }

    /// End to end: the workers' lines come from the store, and a refresh brings new ones.
    #[tokio::test]
    async fn worker_lines_arrive_from_the_store_and_on_refresh() {
        use htui_core::store::WorkerLogStore as _;

        let store = htui_core::store::MemStore::demo();
        let line = |text: &str| htui_core::model::NewWorkerLog {
            logged_at: at(1),
            level: WorkerLogLevel::Warn,
            target: "htui_worker".to_owned(),
            text: text.to_owned(),
        };
        store
            .append_worker_logs(htui_core::fixtures::ids::BOX, &[line("first ship")])
            .await
            .expect("the fixture box takes a line");
        let mut harness =
            Harness::over(store.clone()).with_tab(Box::new(LogsTab::new(LogBuffer::new())));
        harness.settle().await;
        let frame = harness.render();
        assert!(
            frame.contains(&worker_row("worker \u{b7} DESKTOP-HTUI", 1, 1)),
            "{frame}"
        );
        store
            .append_worker_logs(htui_core::fixtures::ids::BOX, &[line("second ship")])
            .await
            .expect("and another");
        refresh(&mut harness);
        harness.settle().await;
        let frame = harness.render();
        assert!(
            frame.contains(&worker_row("worker \u{b7} DESKTOP-HTUI", 2, 2)),
            "{frame}"
        );
    }

    #[tokio::test]
    async fn location_and_restore_return_to_a_worker_source() {
        let bench = SectionBench::new().await;
        let mut rows = two_workers();
        rows.push(row(
            8,
            None,
            "OLD-RIG",
            WorkerLogLevel::Error,
            "htui_worker",
            "gone",
        ));
        let mut tab = with_workers(mixed(), &rows);

        tab.selected = Pick::Worker(WorkerKey::Box(GPU));
        let gpu = tab.location();
        assert_eq!(gpu.cursor, Some(format!("source:worker:{GPU}")));
        tab.selected = Pick::Worker(WorkerKey::Removed("OLD-RIG".to_owned()));
        let removed = tab.location();
        assert_eq!(
            removed.cursor.as_deref(),
            Some("source:worker:removed:OLD-RIG")
        );
        tab.selected = Pick::Local(Source::Worker);
        let local = tab.location();
        assert_eq!(
            local.cursor.as_deref(),
            Some("source:worker"),
            "the local source"
        );

        tab.selected = Pick::All;
        assert!(tab.restore(&gpu, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::Worker(WorkerKey::Box(GPU)));
        assert!(tab.restore(&removed, &mut bench.ctx()));
        assert_eq!(
            tab.selected,
            Pick::Worker(WorkerKey::Removed("OLD-RIG".to_owned()))
        );
        assert!(tab.restore(&local, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::Local(Source::Worker));

        let silent = TabLoc {
            cursor: Some(format!("source:worker:{}", BoxId::new())),
            ..TabLoc::default()
        };
        assert!(tab.restore(&silent, &mut bench.ctx()));
        assert_eq!(
            tab.selected,
            Pick::All,
            "a box with no lines lands on `all`"
        );
        let garbled = TabLoc {
            cursor: Some("source:worker:not-a-box".to_owned()),
            ..TabLoc::default()
        };
        tab.selected = Pick::Local(Source::Store);
        assert!(!tab.restore(&garbled, &mut bench.ctx()));
        assert_eq!(tab.selected, Pick::Local(Source::Store), "nothing moves");
    }
}
