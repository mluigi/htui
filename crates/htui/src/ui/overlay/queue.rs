//! The queue overlay (MOD-12 M3 D8, `R-TUI-1`): this box's queue in order, each entry's state, and
//! the writes that steer it: reorder, pause/resume, dequeue. `Enter` reveals the row's run or item.
//!
//! It holds the last [`QueueOverview`] the store worker answered and re-reads it after every queue
//! write (saying the write's result on the status line, M3 review R1 N2), after a failed queue
//! write, and on the shell's refresh tick (M3 D9) while no read is in flight: every
//! `REFRESHES_PER_READ`th tick after a good read (M3 review R1 M2), every
//! `REFRESHES_PER_RETRY`th after a failed one.

use htui_core::model::{
    BatchClose, EntryState, ItemId, QueueMove, QueueOverview, QueueRow, Scope, format_usd,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction, RevealTarget};
use crate::store_worker::{QUEUE_REQUEST_NAMES, QueueWrite, StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crate::ui::tabs::backlog::queue_sentence;
use crossterm::event::{KeyCode, KeyEvent};

/// Marker in front of the row the cursor is on (the waiting list's): a snapshot records symbols
/// and not styles.
const CURSOR: &str = "> ";

/// The marker's width, in front of every other line, so the columns stay put.
const NO_CURSOR: &str = "  ";

/// Gap between two columns.
const GAP: &str = "  ";

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

/// The widest the title column grows, in cells; a longer title is cut with an ellipsis.
const TITLE_MAX: usize = 32;

/// The line under the list. `Esc` is the wildcard overlay binding; the rest are handled here.
const HINT: &str = "j/k move · J/K reorder · P pause/resume · Q dequeue · Enter open · Esc close";

/// What the list says before the first `QueueOverview` reply.
const READING: &str = "reading the queue";

/// What a reply with no entry says.
const EMPTY: &str = "the queue is empty";

/// In front of a failed read's message, on the row line.
const UNAVAILABLE: &str = "the queue is unavailable: ";

/// The memory backend's header tail: its runtime never admits (M1 review L3). The text of the
/// Backlog's own `DEMO_NOTHING_ADMITTED`, copied so the two views stay independent.
const DEMO_NOTHING_ADMITTED: &str = "demo: nothing is admitted";

/// [`StoreRequest::name`] of [`StoreRequest::QueueOverview`]: its `Failed` is shown, not re-read.
const OVERVIEW: &str = QUEUE_REQUEST_NAMES[5];

/// M3 review R1 M2: after a good read, `refresh` asks again on every third refresh tick (about
/// three seconds): the overview is about ten reads on the serial store worker, so every other
/// request would wait behind it each second the overlay is open.
const REFRESHES_PER_READ: u32 = 3;

/// After a failed read, `refresh` asks again on every tenth refresh tick (about ten seconds), not
/// every one: an unreachable store refuses each read, and `App::on_reply` would re-post the
/// refusal to the status line every second.
const REFRESHES_PER_RETRY: u32 = 10;

/// Lists this box's queue and steers it.
#[derive(Debug)]
pub struct QueueOverlay {
    /// The last `QueueOverview`; `None` before the first, and after a failed read.
    overview: Option<QueueOverview>,
    /// Each row's sentence, in row order, said once per reply that changed the overview rather
    /// than on every frame (M3 review R1 N1).
    sentences: Vec<String>,
    /// The column widths of those rows, measured with the sentences.
    columns: Columns,
    /// The last failed read's message, shown instead of the rows.
    failure: Option<String>,
    /// The highlighted row's index, the fallback when the anchor's row is gone.
    cursor: usize,
    /// The highlighted row's item, re-found on every reply (a move or a re-sort keeps it).
    anchor: Option<ItemId>,
    /// A `QueueOverview` is in flight: `refresh` asks for none. `true` from `new`, because the
    /// shell sends `wants_requests`' read as it pushes the overlay.
    in_flight: bool,
    /// Refresh ticks `refresh` still lets pass before it reads again: [`REFRESHES_PER_READ`] − 1
    /// after a good read, [`REFRESHES_PER_RETRY`] − 1 after a failed one.
    wait_in: u32,
    /// A refused queue write's status line (`"{request}: {message}"`) while its re-read is in
    /// flight: if that read fails too, its failure would replace the write's on the status line,
    /// so the overlay posts the write's again.
    write_failure: Option<String>,
}

impl Default for QueueOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl QueueOverlay {
    /// The factory's and `Act::Queue`'s id.
    pub const ID: OverlayId = OverlayId("queue");

    /// Empty, its first read in flight.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            overview: None,
            sentences: Vec::new(),
            columns: Columns::EMPTY,
            failure: None,
            cursor: 0,
            anchor: None,
            in_flight: true,
            wait_in: 0,
            write_failure: None,
        }
    }

    /// The rows on hand: none before the first reply or after a failed read.
    fn rows(&self) -> &[(QueueRow, EntryState)] {
        self.overview
            .as_ref()
            .map_or(&[][..], |overview| overview.rows.as_slice())
    }

    /// The highlighted row, if any.
    fn selected(&self) -> Option<&(QueueRow, EntryState)> {
        self.rows().get(self.cursor)
    }

    /// Puts the cursor on row `index`, index and item both.
    fn put(&mut self, index: usize) {
        self.cursor = index;
        self.anchor = self.rows().get(index).map(|(row, _)| row.entry.item_id);
    }

    /// A good read: the overview, each row's sentence and the column widths. A reply equal to the
    /// overview on hand (the common refresh) is neither cloned nor re-said (M3 review R1 N1).
    fn take(&mut self, overview: &QueueOverview) {
        if self.overview.as_ref() == Some(overview) {
            return;
        }
        self.sentences = overview
            .rows
            .iter()
            .map(|(_, state)| state.to_string())
            .collect();
        self.columns = Columns::of(&overview.rows, &self.sentences);
        self.overview = Some(overview.clone());
    }

    /// A failed read: no rows, so nothing to say of them.
    fn drop_rows(&mut self) {
        self.overview = None;
        self.sentences.clear();
        self.columns = Columns::EMPTY;
    }

    /// After a reply: the anchor's row, else the stored index clamped. An empty list keeps both,
    /// so the next rows fall back to where the cursor was.
    fn reanchor(&mut self) {
        let rows = self.rows();
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let at = self
            .anchor
            .and_then(|anchor| rows.iter().position(|(row, _)| row.entry.item_id == anchor))
            .unwrap_or_else(|| self.cursor.min(last));
        self.put(at);
    }

    /// Asks for the overview, marking it in flight.
    fn reread(&mut self, ctx: &Ctx<'_>) {
        self.in_flight = true;
        ctx.request(StoreRequest::QueueOverview);
    }

    /// `J`/`K`: move the highlighted entry one place; the anchor stays on its item.
    fn request_move(&self, to: QueueMove, ctx: &Ctx<'_>) {
        if let Some((row, _)) = self.selected() {
            ctx.request(StoreRequest::MoveQueueEntry {
                item: row.entry.item_id,
                to,
            });
        }
    }

    /// `Enter`: close, then reveal the highlighted row's run (and parked step) when its state
    /// carries one, else its item (the waiting list's order). Nothing on an empty list.
    fn enter(&self, ctx: &Ctx<'_>) {
        let Some((row, state)) = self.selected() else {
            return;
        };
        let (run, step) = state.reveal();
        let item = row.entry.item_id;
        let key = row.key.clone();
        let target = if run.is_some() {
            RevealTarget::Step {
                item,
                key,
                run,
                step,
            }
        } else {
            RevealTarget::Item { id: item, key }
        };
        ctx.emit(Action::Overlay(OverlayAction::Close));
        ctx.emit(Action::Reveal(target));
    }

    /// The box's contents: the header and the visible rows (or the reading, empty or unavailable
    /// line), a blank line, then the hint.
    ///
    /// `room` is the box's inner width and `text_width` the room the sentence column gets;
    /// `None` is the natural size, nothing clipped. The header, the unavailable line and the hint
    /// are clipped to `room` with an ellipsis too (the whole failure is on the status line as
    /// well).
    /// `visible` is how many rows fit: the window always holds the cursor.
    fn lines(
        &self,
        room: Option<usize>,
        text_width: Option<usize>,
        visible: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let clipped = |text: &str| {
            let text = match room {
                Some(room) => cells::clip(text, room.saturating_sub(cell_width(NO_CURSOR))),
                None => text.to_owned(),
            };
            format!("{NO_CURSOR}{text}")
        };
        let mut lines = match (&self.failure, &self.overview) {
            (Some(message), _) => vec![Line::styled(
                clipped(&format!("{UNAVAILABLE}{message}")),
                theme.dim,
            )],
            (None, None) => vec![Line::styled(format!("{NO_CURSOR}{READING}"), theme.dim)],
            (None, Some(overview)) => {
                let mut lines = vec![
                    Line::styled(clipped(&header(overview)), theme.base),
                    Line::raw(""),
                ];
                if overview.rows.is_empty() {
                    lines.push(Line::styled(format!("{NO_CURSOR}{EMPTY}"), theme.dim));
                } else {
                    let columns = &self.columns;
                    let text_width = text_width.unwrap_or(columns.text);
                    let at = self.cursor.min(overview.rows.len() - 1);
                    let first = if visible > 0 && at >= visible {
                        at + 1 - visible
                    } else {
                        0
                    };
                    lines.extend(
                        overview
                            .rows
                            .iter()
                            .enumerate()
                            .skip(first)
                            .take(visible)
                            .map(|(index, (row, state))| {
                                let sentence = self.sentences.get(index).map_or("", String::as_str);
                                row_line(
                                    row,
                                    state,
                                    sentence,
                                    index == at,
                                    columns,
                                    text_width,
                                    theme,
                                )
                            }),
                    );
                }
                lines
            }
        };
        lines.push(Line::raw(""));
        lines.push(Line::styled(clipped(HINT), theme.dim));
        lines
    }

    /// The lines around the rows: header and blank over them, blank and hint under them.
    const fn fixed(&self) -> usize {
        if self.failure.is_none() && self.overview.is_some() {
            4
        } else {
            2
        }
    }
}

/// The header (M3 D8): running or paused, then the batch's figures and the slots, or the demo's
/// note. The batch time is UTC, as the Connection section formats one, so it reads alike on every
/// host. A paused queue says why when a batch closed before (M3 review R1 L4): `P` paused it, or
/// the runner drained it. A stall and an emptied queue both close `drained`, so the drained
/// reason says only what both mean: nothing was admissible.
fn header(overview: &QueueOverview) -> String {
    let state = match (&overview.batch, overview.last_close) {
        (Some(_), _) => "running",
        (None, Some(BatchClose::Paused)) => "paused (by a user)",
        (None, Some(BatchClose::Drained)) => "paused (the last batch drained: nothing admissible)",
        (None, None) => "paused",
    };
    if overview.demo {
        return format!("queue: {state} · {DEMO_NOTHING_ADMITTED}");
    }
    let slots = format!("{}/{} slots", overview.slots_used, overview.slots_limit);
    match &overview.batch {
        Some(batch) => {
            let spent = batch.spent.map_or_else(
                || "no spend yet".to_owned(),
                |micros| format!("{} spent", format_usd(micros)),
            );
            format!(
                "queue: running · batch since {} · {spent} · {slots}",
                batch.opened_at.format("%H:%M")
            )
        }
        None => format!("queue: {state} · {slots}"),
    }
}

/// The widths of a row's columns, in cells.
#[derive(Debug)]
struct Columns {
    /// The item key: the widest.
    key: usize,
    /// The title: the widest, at most [`TITLE_MAX`].
    title: usize,
    /// The sentence: the widest.
    text: usize,
}

impl Columns {
    /// No rows.
    const EMPTY: Self = Self {
        key: 0,
        title: 0,
        text: 0,
    };

    /// The widths of `rows`, whose sentences are `sentences`.
    fn of(rows: &[(QueueRow, EntryState)], sentences: &[String]) -> Self {
        let widest = |cell: &dyn Fn(&(QueueRow, EntryState)) -> usize| {
            rows.iter().map(cell).max().unwrap_or(0)
        };
        Self {
            key: widest(&|(row, _)| cell_width(&row.key)),
            title: widest(&|(row, _)| cell_width(&row.title)).min(TITLE_MAX),
            text: sentences
                .iter()
                .map(|sentence| cell_width(sentence))
                .max()
                .unwrap_or(0),
        }
    }

    /// Everything in front of the sentence: the marker, two padded columns and their gaps.
    fn prefix(&self) -> usize {
        cell_width(CURSOR) + self.key + self.title + 2 * cell_width(GAP)
    }
}

/// One row: `{marker}{key}{GAP}{title}{GAP}{sentence}`, the key and title fit to their column and
/// `sentence` (`state`'s, said on reply) clipped to `text_width` with an ellipsis, never wrapped.
/// The selected row is in the accent style, an escalated one in the warning style.
fn row_line(
    row: &QueueRow,
    state: &EntryState,
    sentence: &str,
    selected: bool,
    columns: &Columns,
    text_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { CURSOR } else { NO_CURSOR };
    let style = if selected {
        theme.accent
    } else if state.is_escalated() {
        theme.warning
    } else {
        theme.base
    };
    Line::styled(
        format!(
            "{marker}{}{GAP}{}{GAP}{}",
            cells::fit(&row.key, columns.key),
            cells::fit(&row.title, columns.title),
            cells::clip(sentence, text_width),
        ),
        style,
    )
}

/// The widest of `lines`, in cells (the switcher's MOD-60 B3 measure).
fn widest(lines: &[Line<'_>]) -> usize {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| cell_width(&span.content))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

impl Overlay for QueueOverlay {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Queue"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // The queue is this box's, not the scope's: one read on open.
        vec![StoreRequest::QueueOverview]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let len = self.rows().len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(last) = len.checked_sub(1) {
                    self.put((self.cursor + 1).min(last));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(last) = len.checked_sub(1) {
                    self.put(self.cursor.min(last).saturating_sub(1));
                }
            }
            KeyCode::Char('J') => self.request_move(QueueMove::Down, ctx),
            KeyCode::Char('K') => self.request_move(QueueMove::Up, ctx),
            // M3 review R1 M1: the write names the state the box shows, so a queue that changed
            // since the last read (a stalled batch the runner closed, L4) refuses it rather than
            // flipping the other way.
            KeyCode::Char('P') => {
                if let Some(overview) = &self.overview {
                    ctx.request(match &overview.batch {
                        Some(batch) => StoreRequest::PauseQueue {
                            expect: Some(batch.id),
                        },
                        None => StoreRequest::ResumeQueue {
                            expect_paused: true,
                        },
                    });
                }
            }
            KeyCode::Char('Q') => {
                if let Some((row, _)) = self.selected() {
                    ctx.request(StoreRequest::DequeueItem {
                        item: row.entry.item_id,
                    });
                }
            }
            KeyCode::Enter => self.enter(ctx),
            // `Esc` falls through to the wildcard overlay binding; the rest (a lower-case `q`
            // included) is swallowed by `is_modal`, so nothing quits or reaches the tab below.
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::QueueOverview(overview) => {
                self.take(overview);
                self.failure = None;
                self.in_flight = false;
                self.wait_in = REFRESHES_PER_READ - 1;
                self.write_failure = None;
                self.reanchor();
            }
            StoreReply::QueueWritten { write, view } => {
                // M3 review R1 N2: the status line says what the write did, in the Backlog's
                // words, before the re-read redraws the box.
                let key = match write {
                    QueueWrite::Queued { item } | QueueWrite::Dequeued { item, .. } => self
                        .rows()
                        .iter()
                        .find(|(row, _)| row.entry.item_id == *item)
                        .map_or_else(|| item.to_string(), |(row, _)| row.key.clone()),
                    QueueWrite::Resumed { .. }
                    | QueueWrite::Paused { .. }
                    | QueueWrite::Moved { .. }
                    | QueueWrite::Stale { .. } => String::new(),
                };
                ctx.emit(Action::Error(queue_sentence(write, &key, view)));
                self.reread(ctx);
            }
            StoreReply::Failed { request, message } if *request == OVERVIEW => {
                // `App::on_reply` already put it on the status line; the box says it too.
                self.drop_rows();
                self.failure = Some(message.clone());
                self.in_flight = false;
                self.wait_in = REFRESHES_PER_RETRY - 1;
                if let Some(write) = self.write_failure.take() {
                    ctx.emit(Action::Error(write));
                }
            }
            StoreReply::Failed { request, message } if QUEUE_REQUEST_NAMES.contains(request) => {
                self.write_failure = Some(format!("{request}: {message}"));
                self.reread(ctx);
            }
            _ => {}
        }
    }

    fn refresh(&mut self, ctx: &mut Ctx<'_>) {
        if self.in_flight {
            return;
        }
        if self.wait_in > 0 {
            self.wait_in -= 1;
            return;
        }
        self.reread(ctx);
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The natural size: every row, every sentence whole.
        let natural = self.lines(None, None, usize::MAX, ctx.theme);
        let width = u16::try_from(widest(&natural))
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME)
            .min(area.width.saturating_sub(4));
        let height = u16::try_from(natural.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(area.height.saturating_sub(2));

        let room = usize::from(width.saturating_sub(CHROME));
        let text_width = room.saturating_sub(self.columns.prefix());
        let visible = usize::from(height.saturating_sub(2)).saturating_sub(self.fixed());
        let lines = self.lines(Some(room), Some(text_width), visible, ctx.theme);

        let box_area = centered(area, width, height);
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(Span::styled(format!(" {} ", self.title()), ctx.theme.title)),
            ),
            box_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::{Origin, QueueView};
    use chrono::{DateTime, TimeZone, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        BatchFigures, BatchId, BatchStop, CapError, Escalation, Hold, PER_TOKEN_CAP_BATCH,
        ProjectId, ProjectRef, QueueEntry, RunId, RunStatus, Status, StepId, Wait, WorkspaceId,
    };
    use htui_core::store::StoreError;
    use htui_store::DATABASE_UNREACHABLE;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// Everything a `Ctx` borrows; the overlay holds its own rows.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: vec![ids::PROJECT_HTUI],
                },
                projects: Vec::new(),
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
                Origin::Overlay(QueueOverlay::ID),
                &self.emit,
            )
        }

        fn code(&self, overlay: &mut QueueOverlay, code: KeyCode) -> Handled {
            overlay.on_key(KeyEvent::from(code), &mut self.ctx())
        }

        fn reply(&self, overlay: &mut QueueOverlay, reply: &StoreReply) {
            overlay.on_reply(reply, &mut self.ctx());
        }

        /// The overview `overview` answered.
        fn feed(&self, overlay: &mut QueueOverlay, overview: QueueOverview) {
            self.reply(overlay, &StoreReply::QueueOverview(Box::new(overview)));
        }

        /// What the overlay emitted since the last call.
        fn drained(&self) -> Vec<Action> {
            self.emit.take()
        }

        /// The overlay drawn over a blank `width`×`height` frame, one line per row.
        fn render_at(&self, overlay: &QueueOverlay, width: u16, height: u16) -> String {
            let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test backend");
            term.draw(|frame| overlay.render(frame, frame.area(), &self.ctx()))
                .expect("the overlay draws");
            let buffer = term.backend().buffer();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }

        fn render(&self, overlay: &QueueOverlay) -> String {
            self.render_at(overlay, 100, 30)
        }
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 7, hour, minute, 0)
            .single()
            .expect("a valid instant")
    }

    /// A queued open item keyed `key`.
    fn row(key: &str, title: &str) -> QueueRow {
        QueueRow {
            entry: QueueEntry {
                item_id: ItemId::new(),
                project_id: ProjectId::default(),
                box_id: ids::BOX,
                position: None,
                queued_at: at(9, 0),
                queued_by: ids::USER,
            },
            key: key.to_owned(),
            title: title.to_owned(),
            status: Status::Open,
            priority: 0,
            created_at: at(8, 0),
            latest_run: None,
            latest_note: None,
            open_blockers: Vec::new(),
        }
    }

    /// A running batch since 14:02 UTC that spent $1.20, one of two slots used.
    fn running_batch() -> BatchFigures {
        BatchFigures {
            id: BatchId::new(),
            opened_at: at(14, 2),
            spent: Some(1_200_000),
        }
    }

    fn overview(rows: Vec<(QueueRow, EntryState)>) -> QueueOverview {
        QueueOverview {
            box_id: ids::BOX,
            batch: Some(running_batch()),
            last_close: None,
            slots_used: 1,
            slots_limit: 2,
            rows,
            demo: false,
        }
    }

    /// Three `Next` rows, `A-1`, `B-2`, `C-3`.
    fn abc() -> Vec<(QueueRow, EntryState)> {
        ["A-1", "B-2", "C-3"]
            .into_iter()
            .map(|key| (row(key, "t"), EntryState::Next))
            .collect()
    }

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    /// Exactly one `Store(request)` matching `expected`.
    fn assert_one_request(emitted: &[Action], expected: impl Fn(&StoreRequest) -> bool) {
        match emitted {
            [Action::Store(request)] => assert!(expected(request), "{request:?}"),
            other => panic!("expected one store request: {other:?}"),
        }
    }

    /// One row per state the overlay can show, each with a title of its own.
    fn every_state() -> Vec<(QueueRow, EntryState)> {
        let (run, step) = (RunId::new(), StepId::new());
        let note = Some("pick a branch".to_owned());
        let states = [
            EntryState::Running {
                run,
                status: RunStatus::Running,
            },
            EntryState::Running {
                run,
                status: RunStatus::Queued,
            },
            EntryState::Elsewhere {
                run,
                hostname: Some("LAPTOP-B".to_owned()),
                status: RunStatus::Running,
            },
            EntryState::Elsewhere {
                run,
                hostname: None,
                status: RunStatus::Queued,
            },
            EntryState::Next,
            EntryState::Held(Hold::Budget(BatchStop::CapReached {
                spent: 600,
                cap: 500,
            })),
            EntryState::Held(Hold::Budget(BatchStop::Budget {
                remaining: 100,
                min: 200,
            })),
            EntryState::Held(Hold::BadCap(CapError::at_key(
                PER_TOKEN_CAP_BATCH,
                "\"x\"".to_owned(),
            ))),
            EntryState::Held(Hold::ProjectGone),
            EntryState::Waiting(Wait::BlockedBy(vec![
                "FEAT-1".to_owned(),
                "FEAT-2".to_owned(),
            ])),
            EntryState::Waiting(Wait::CancelledInBatch),
            EntryState::Waiting(Wait::Paused),
            EntryState::Waiting(Wait::NotReady(Status::InProgress)),
            EntryState::Escalated(Escalation::HardGateParked { run, step }),
            EntryState::Escalated(Escalation::JudgeUndecided {
                run,
                note: note.clone(),
            }),
            EntryState::Escalated(Escalation::ReviewLoopExhausted { run, note: None }),
            EntryState::Escalated(Escalation::Blocked {
                run: None,
                note: note.clone(),
            }),
            EntryState::Escalated(Escalation::Failed {
                run: Some(run),
                failure: Some("exit 1".to_owned()),
            }),
            EntryState::Escalated(Escalation::MissingTags(vec!["gpu".to_owned()])),
        ];
        states
            .into_iter()
            .enumerate()
            .map(|(n, state)| {
                (
                    row(&format!("QX-{}", n + 1), &format!("entry {}", n + 1)),
                    state,
                )
            })
            .collect()
    }

    #[test]
    fn before_the_first_reply_it_says_reading() {
        let bench = Bench::new();
        let overlay = QueueOverlay::new();
        let rendered = bench.render(&overlay);
        assert!(rendered.contains(READING), "{rendered}");
        insta::assert_snapshot!("reading", rendered);
    }

    #[test]
    fn every_state_reads_its_sentence() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(every_state()));
        let lines = overlay.lines(None, None, usize::MAX, &Theme::default());
        let texts = texts(&lines);
        for (row, state) in every_state() {
            let sentence = state.to_string();
            assert!(
                texts
                    .iter()
                    .any(|line| line.contains(&row.key) && line.ends_with(&sentence)),
                "{sentence}: {texts:?}"
            );
        }
        insta::assert_snapshot!("states", bench.render(&overlay));
    }

    /// M3 review R1 N1: each row's sentence is worked out once per reply that changed the queue,
    /// not on every frame; a reply equal to the one on hand is neither cloned nor re-said.
    #[test]
    fn a_reply_caches_each_rows_sentence_and_an_unchanged_one_is_kept() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let first = overview(every_state());
        bench.feed(&mut overlay, first.clone());
        let said: Vec<String> = first
            .rows
            .iter()
            .map(|(_, state)| state.to_string())
            .collect();
        assert_eq!(overlay.sentences, said);

        let (rows, sentences) = (overlay.rows().as_ptr(), overlay.sentences.as_ptr());
        bench.feed(&mut overlay, first);
        assert_eq!(
            (overlay.rows().as_ptr(), overlay.sentences.as_ptr()),
            (rows, sentences),
            "an unchanged reply keeps what is on hand"
        );

        bench.feed(&mut overlay, overview(abc()));
        assert_eq!(overlay.sentences, ["next to run"; 3]);
        assert!(bench.render(&overlay).contains("A-1  t  next to run"));
    }

    #[test]
    fn the_header_reads_a_running_batch() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(abc()));
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: running · batch since 14:02 · $1.20 spent · 1/2 slots"),
            "{rendered}"
        );
        insta::assert_snapshot!("header_running", rendered);

        let mut unspent = overview(abc());
        if let Some(batch) = unspent.batch.as_mut() {
            batch.spent = None;
        }
        bench.feed(&mut overlay, unspent);
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: running · batch since 14:02 · no spend yet · 1/2 slots"),
            "{rendered}"
        );
    }

    #[test]
    fn the_header_reads_paused() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(
            &mut overlay,
            QueueOverview {
                batch: None,
                slots_used: 0,
                ..overview(abc())
            },
        );
        let rendered = bench.render(&overlay);
        assert!(rendered.contains("queue: paused · 0/2 slots"), "{rendered}");
        insta::assert_snapshot!("header_paused", rendered);
    }

    /// M3 review R1 L4: a paused header says why: `P` paused it, or the runner closed the batch
    /// as drained (a stall and an emptied queue record the same reason, so it says only that
    /// nothing was admissible). Before any batch closed it says nothing more.
    #[test]
    fn the_header_says_why_the_queue_is_paused() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let paused = |last_close| QueueOverview {
            batch: None,
            last_close,
            slots_used: 0,
            ..overview(abc())
        };

        bench.feed(&mut overlay, paused(Some(BatchClose::Drained)));
        let rendered = bench.render(&overlay);
        assert!(
            rendered
                .contains("queue: paused (the last batch drained: nothing admissible) · 0/2 slots"),
            "{rendered}"
        );
        insta::assert_snapshot!("header_drained", rendered);

        bench.feed(&mut overlay, paused(Some(BatchClose::Paused)));
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: paused (by a user) · 0/2 slots"),
            "{rendered}"
        );

        bench.feed(&mut overlay, paused(None));
        let rendered = bench.render(&overlay);
        assert!(rendered.contains("queue: paused · 0/2 slots"), "{rendered}");

        bench.feed(
            &mut overlay,
            QueueOverview {
                demo: true,
                ..paused(Some(BatchClose::Paused))
            },
        );
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: paused (by a user) · demo: nothing is admitted"),
            "{rendered}"
        );
    }

    #[test]
    fn the_header_reads_demo() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(
            &mut overlay,
            QueueOverview {
                demo: true,
                ..overview(abc())
            },
        );
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: running · demo: nothing is admitted"),
            "{rendered}"
        );
        insta::assert_snapshot!("header_demo", rendered);

        bench.feed(
            &mut overlay,
            QueueOverview {
                demo: true,
                batch: None,
                ..overview(abc())
            },
        );
        let rendered = bench.render(&overlay);
        assert!(
            rendered.contains("queue: paused · demo: nothing is admitted"),
            "{rendered}"
        );
    }

    #[test]
    fn an_empty_queue_says_so() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(Vec::new()));
        let rendered = bench.render(&overlay);
        assert!(rendered.contains(EMPTY), "{rendered}");
    }

    #[test]
    fn a_failed_read_shows_the_unavailable_line() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let message = StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()).to_string();
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: OVERVIEW,
                message: message.clone(),
            },
        );
        assert!(bench.drained().is_empty(), "a failed read is not re-read");
        let natural = overlay.lines(None, None, usize::MAX, &Theme::default());
        assert_eq!(
            texts(&natural)[0],
            format!("{NO_CURSOR}{UNAVAILABLE}{message}")
        );
        let rendered = bench.render(&overlay);
        let line = rendered
            .lines()
            .find(|line| line.contains(UNAVAILABLE))
            .expect("the unavailable line is drawn");
        assert!(
            line.contains("the queue is unavailable: store unreachable: this box browses"),
            "{rendered}"
        );
        assert!(
            line.trim_end_matches('│').trim_end().ends_with('…'),
            "a message wider than the box is clipped: {rendered}"
        );
        insta::assert_snapshot!("offline", rendered);

        // Keys over the failure do nothing.
        for code in [
            KeyCode::Char('J'),
            KeyCode::Char('P'),
            KeyCode::Char('Q'),
            KeyCode::Enter,
        ] {
            assert_eq!(bench.code(&mut overlay, code), Handled::Consumed);
        }
        assert!(bench.drained().is_empty());

        // The next good read replaces it.
        bench.feed(&mut overlay, overview(abc()));
        assert!(!bench.render(&overlay).contains(UNAVAILABLE));
    }

    #[test]
    fn a_narrow_frame_clips_the_sentence_with_an_ellipsis() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let long = EntryState::Escalated(Escalation::JudgeUndecided {
            run: RunId::new(),
            note: Some("fan-out `p` attempt 2 awaits selection: 3 candidates".to_owned()),
        });
        bench.feed(
            &mut overlay,
            overview(vec![
                (row("FIX-1", "a short title"), EntryState::Next),
                (row("FIX-2", "the judge has to pick a branch"), long),
            ]),
        );
        let rendered = bench.render_at(&overlay, 60, 20);
        let clipped = rendered
            .lines()
            .find(|line| line.contains("FIX-2"))
            .expect("the long row is drawn");
        assert!(clipped.contains("judge undec"), "{rendered}");
        assert!(
            clipped.trim_end_matches('│').trim_end().ends_with('…'),
            "{rendered}"
        );
        let hint = rendered
            .lines()
            .find(|line| line.contains("j/k move"))
            .expect("the hint is drawn");
        assert!(
            hint.trim_end_matches('│').trim_end().ends_with('…'),
            "a hint wider than the box is cut with an ellipsis: {rendered}"
        );
        insta::assert_snapshot!("narrow", rendered);
    }

    #[test]
    fn the_row_window_keeps_the_cursor_drawn() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let rows = (1..=20)
            .map(|n| (row(&format!("W-{n:02}"), "t"), EntryState::Next))
            .collect();
        bench.feed(&mut overlay, overview(rows));

        // A 12-line frame holds a 10-line box: header, blank, four rows, blank, hint.
        let top = bench.render_at(&overlay, 100, 12);
        assert!(top.contains("> W-01"), "{top}");
        assert!(top.contains("W-04"), "{top}");
        assert!(!top.contains("W-05"), "{top}");

        for _ in 0..19 {
            bench.code(&mut overlay, KeyCode::Char('j'));
        }
        let bottom = bench.render_at(&overlay, 100, 12);
        assert!(
            bottom.contains("> W-20"),
            "the cursor row is drawn: {bottom}"
        );
        assert!(bottom.contains("W-17"), "{bottom}");
        assert!(!bottom.contains("W-16"), "{bottom}");
        assert!(
            !bottom.contains("W-01"),
            "the first row scrolled off: {bottom}"
        );
    }

    #[test]
    fn a_long_title_is_cut_at_its_column() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let title = "a title far longer than the thirty-two cells the column has";
        bench.feed(
            &mut overlay,
            overview(vec![(row("FIX-1", title), EntryState::Next)]),
        );
        let lines = overlay.lines(None, None, usize::MAX, &Theme::default());
        let row_text = &texts(&lines)[2];
        assert_eq!(
            row_text,
            &format!("> FIX-1  {}  next to run", cells::fit(title, TITLE_MAX)),
        );
        assert!(row_text.contains('…'));
    }

    #[test]
    fn escalated_rows_use_the_warning_style() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(
            &mut overlay,
            overview(vec![
                (row("SEL-1", "t"), EntryState::Next),
                (
                    row("ESC-2", "t"),
                    EntryState::Escalated(Escalation::MissingTags(vec!["gpu".to_owned()])),
                ),
                (row("CAL-3", "t"), EntryState::Next),
            ]),
        );
        let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
        term.draw(|frame| overlay.render(frame, frame.area(), &bench.ctx()))
            .expect("the overlay draws");
        let buffer = term.backend().buffer();
        let fg_of = |key: &str| {
            for y in 0..buffer.area.height {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect();
                if let Some(x) = line.find(key) {
                    let x = u16::try_from(line[..x].chars().count()).expect("narrow");
                    return buffer[(x, y)].fg;
                }
            }
            panic!("{key} is not drawn");
        };
        assert_eq!(
            fg_of("SEL-1"),
            Color::Cyan,
            "the selected row is the accent"
        );
        assert_eq!(
            fg_of("ESC-2"),
            Color::Yellow,
            "the escalated row is the warning"
        );
        assert_eq!(fg_of("CAL-3"), Color::Reset, "an ordinary row is the base");

        // Selected wins over escalated.
        bench.code(&mut overlay, KeyCode::Char('j'));
        let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
        term.draw(|frame| overlay.render(frame, frame.area(), &bench.ctx()))
            .expect("the overlay draws");
        let buffer = term.backend().buffer();
        let found = (0..buffer.area.height).any(|y| {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect();
            line.find("ESC-2").is_some_and(|x| {
                let x = u16::try_from(line[..x].chars().count()).expect("narrow");
                buffer[(x, y)].fg == Color::Cyan
            })
        });
        assert!(found, "the selected escalated row is the accent");
    }

    #[test]
    fn the_cursor_follows_its_item_across_a_reorder() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let rows = abc();
        let b = rows[1].0.entry.item_id;
        bench.feed(&mut overlay, overview(rows.clone()));
        assert_eq!(
            bench.code(&mut overlay, KeyCode::Char('j')),
            Handled::Consumed
        );
        assert_eq!((overlay.cursor, overlay.anchor), (1, Some(b)));

        let reordered = vec![rows[1].clone(), rows[0].clone(), rows[2].clone()];
        bench.feed(&mut overlay, overview(reordered));
        assert_eq!((overlay.cursor, overlay.anchor), (0, Some(b)));
        assert!(bench.render(&overlay).contains("> B-2 "));

        // Its row gone: the index clamped, the anchor on the row now there.
        bench.code(&mut overlay, KeyCode::Char('j'));
        bench.code(&mut overlay, KeyCode::Char('j'));
        let shrunk = vec![rows[0].clone()];
        bench.feed(&mut overlay, overview(shrunk));
        assert_eq!(
            (overlay.cursor, overlay.anchor),
            (0, Some(rows[0].0.entry.item_id))
        );

        // `k` saturates, `j` clamps.
        bench.code(&mut overlay, KeyCode::Up);
        bench.code(&mut overlay, KeyCode::Down);
        assert_eq!(overlay.cursor, 0);
    }

    #[test]
    fn capital_j_and_k_request_a_move_of_the_cursor_row() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let rows = abc();
        let b = rows[1].0.entry.item_id;
        bench.feed(&mut overlay, overview(rows));
        bench.code(&mut overlay, KeyCode::Char('j'));
        assert!(bench.drained().is_empty(), "the cursor asks for nothing");

        assert_eq!(
            bench.code(&mut overlay, KeyCode::Char('J')),
            Handled::Consumed
        );
        assert_one_request(
            &bench.drained(),
            |request| matches!(request, StoreRequest::MoveQueueEntry { item, to: QueueMove::Down } if *item == b),
        );
        assert_eq!(overlay.anchor, Some(b), "the anchor stays on the item");

        assert_eq!(
            bench.code(&mut overlay, KeyCode::Char('K')),
            Handled::Consumed
        );
        assert_one_request(
            &bench.drained(),
            |request| matches!(request, StoreRequest::MoveQueueEntry { item, to: QueueMove::Up } if *item == b),
        );
    }

    #[test]
    fn p_pauses_a_running_queue_and_resumes_a_paused_one() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        assert_eq!(
            bench.code(&mut overlay, KeyCode::Char('P')),
            Handled::Consumed
        );
        assert!(bench.drained().is_empty(), "nothing before the first reply");

        let running = overview(abc());
        let seen = running.batch.as_ref().map(|batch| batch.id);
        assert!(seen.is_some());
        bench.feed(&mut overlay, running);
        bench.code(&mut overlay, KeyCode::Char('P'));
        assert_one_request(
            &bench.drained(),
            |request| matches!(request, StoreRequest::PauseQueue { expect } if *expect == seen),
        );

        bench.feed(
            &mut overlay,
            QueueOverview {
                batch: None,
                ..overview(Vec::new())
            },
        );
        bench.code(&mut overlay, KeyCode::Char('P'));
        assert_one_request(&bench.drained(), |request| {
            matches!(
                request,
                StoreRequest::ResumeQueue {
                    expect_paused: true
                }
            )
        });
    }

    #[test]
    fn capital_q_dequeues_the_cursor_row() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let rows = abc();
        let c = rows[2].0.entry.item_id;
        bench.feed(&mut overlay, overview(rows));
        bench.code(&mut overlay, KeyCode::Char('j'));
        bench.code(&mut overlay, KeyCode::Char('j'));
        assert_eq!(
            bench.code(&mut overlay, KeyCode::Char('Q')),
            Handled::Consumed
        );
        assert_one_request(
            &bench.drained(),
            |request| matches!(request, StoreRequest::DequeueItem { item } if *item == c),
        );
    }

    #[test]
    fn keys_on_an_empty_or_unread_list_do_nothing() {
        let bench = Bench::new();
        for read in [false, true] {
            let mut overlay = QueueOverlay::new();
            if read {
                bench.feed(&mut overlay, overview(Vec::new()));
            }
            for code in [
                KeyCode::Char('j'),
                KeyCode::Char('k'),
                KeyCode::Char('J'),
                KeyCode::Char('K'),
                KeyCode::Char('Q'),
                KeyCode::Enter,
            ] {
                assert_eq!(
                    bench.code(&mut overlay, code),
                    Handled::Consumed,
                    "{code:?}"
                );
            }
            assert!(bench.drained().is_empty());
        }
    }

    #[test]
    fn enter_closes_then_reveals_the_run_or_the_item() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let (run, step) = (RunId::new(), StepId::new());
        let gate = row("GATE-1", "t");
        let next = row("NEXT-2", "t");
        bench.feed(
            &mut overlay,
            overview(vec![
                (
                    gate.clone(),
                    EntryState::Escalated(Escalation::HardGateParked { run, step }),
                ),
                (next.clone(), EntryState::Next),
            ]),
        );
        assert_eq!(bench.code(&mut overlay, KeyCode::Enter), Handled::Consumed);
        match bench.drained().as_slice() {
            [
                Action::Overlay(OverlayAction::Close),
                Action::Reveal(target),
            ] => assert_eq!(
                *target,
                RevealTarget::Step {
                    item: gate.entry.item_id,
                    key: "GATE-1".to_owned(),
                    run: Some(run),
                    step: Some(step),
                }
            ),
            other => panic!("expected Close then Reveal: {other:?}"),
        }

        bench.code(&mut overlay, KeyCode::Char('j'));
        bench.code(&mut overlay, KeyCode::Enter);
        match bench.drained().as_slice() {
            [
                Action::Overlay(OverlayAction::Close),
                Action::Reveal(target),
            ] => assert_eq!(
                *target,
                RevealTarget::Item {
                    id: next.entry.item_id,
                    key: "NEXT-2".to_owned(),
                }
            ),
            other => panic!("expected Close then Reveal: {other:?}"),
        }
    }

    #[test]
    fn a_queue_write_reply_re_reads_the_overview() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(abc()));
        assert!(bench.drained().is_empty());

        bench.reply(
            &mut overlay,
            &StoreReply::QueueWritten {
                write: QueueWrite::Moved { moved: true },
                view: QueueView {
                    entries: Vec::new(),
                    open_batch: None,
                    demo: false,
                },
            },
        );
        match bench.drained().as_slice() {
            [Action::Error(_), Action::Store(StoreRequest::QueueOverview)] => {}
            other => panic!("expected the write's sentence, then the re-read: {other:?}"),
        }

        bench.feed(&mut overlay, overview(abc()));
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: "pause_queue",
                message: "store unreachable".to_owned(),
            },
        );
        assert_one_request(&bench.drained(), |request| {
            matches!(request, StoreRequest::QueueOverview)
        });
        assert!(
            !bench.render(&overlay).contains(UNAVAILABLE),
            "a failed write keeps the rows"
        );

        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: "items",
                message: "boom".to_owned(),
            },
        );
        assert!(bench.drained().is_empty(), "not a queue request");
    }

    /// M3 review R1 N2: a write's reply puts the Backlog's sentence for it on the status line,
    /// then re-reads: a move at the end says so, a dequeue names its row's key, and a refused `P`
    /// (M1) says the queue changed.
    #[test]
    fn a_queue_write_says_what_it_did_then_re_reads() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let rows = abc();
        let b = rows[1].0.entry.item_id;
        let entries: Vec<ItemId> = rows.iter().map(|(row, _)| row.entry.item_id).collect();
        bench.feed(&mut overlay, overview(rows));
        let view = |entries: &[ItemId], open_batch| QueueView {
            entries: entries.to_vec(),
            open_batch,
            demo: false,
        };

        let cases = [
            (
                QueueWrite::Moved { moved: false },
                view(&entries, Some(BatchId::new())),
                "already at that end of the queue",
            ),
            (
                QueueWrite::Dequeued {
                    item: b,
                    was_queued: true,
                },
                view(&entries[..2], Some(BatchId::new())),
                "dequeued B-2 (2 in queue)",
            ),
            (
                QueueWrite::Stale { pause: true },
                view(&entries, None),
                "queue already paused: its batch closed before P reached it",
            ),
        ];
        for (write, view, sentence) in cases {
            bench.reply(&mut overlay, &StoreReply::QueueWritten { write, view });
            match bench.drained().as_slice() {
                [
                    Action::Error(said),
                    Action::Store(StoreRequest::QueueOverview),
                ] => assert_eq!(said, sentence),
                other => panic!("expected {sentence:?}, then the re-read: {other:?}"),
            }
        }
    }

    #[test]
    fn refresh_requests_only_when_nothing_is_in_flight() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        assert!(matches!(
            overlay.wants_requests(&bench.scope).as_slice(),
            [StoreRequest::QueueOverview]
        ));
        overlay.refresh(&mut bench.ctx());
        assert!(bench.drained().is_empty(), "the open read is in flight");

        bench.feed(&mut overlay, overview(abc()));
        for _ in 0..REFRESHES_PER_READ {
            overlay.refresh(&mut bench.ctx());
        }
        assert_one_request(&bench.drained(), |request| {
            matches!(request, StoreRequest::QueueOverview)
        });
        for _ in 0..REFRESHES_PER_READ {
            overlay.refresh(&mut bench.ctx());
        }
        assert!(bench.drained().is_empty(), "the tick's read is in flight");

        // A failed read clears it too, and is asked again after the back-off.
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: OVERVIEW,
                message: "boom".to_owned(),
            },
        );
        for _ in 0..REFRESHES_PER_RETRY {
            overlay.refresh(&mut bench.ctx());
        }
        assert_one_request(&bench.drained(), |request| {
            matches!(request, StoreRequest::QueueOverview)
        });
    }

    #[test]
    fn a_failed_read_is_asked_again_only_every_tenth_refresh() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        let failed = StoreReply::Failed {
            request: OVERVIEW,
            message: StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()).to_string(),
        };
        bench.reply(&mut overlay, &failed);
        assert_eq!(REFRESHES_PER_RETRY, 10);

        for round in 0..2 {
            for tick in 1..REFRESHES_PER_RETRY {
                overlay.refresh(&mut bench.ctx());
                assert!(
                    bench.drained().is_empty(),
                    "round {round}, refresh {tick}: an unreachable store is not asked every second"
                );
            }
            overlay.refresh(&mut bench.ctx());
            assert_one_request(&bench.drained(), |request| {
                matches!(request, StoreRequest::QueueOverview)
            });
            bench.reply(&mut overlay, &failed);
        }

        // A good read ends the back-off: the read cadence takes over.
        bench.feed(&mut overlay, overview(abc()));
        for _ in 0..REFRESHES_PER_READ {
            overlay.refresh(&mut bench.ctx());
        }
        assert_one_request(&bench.drained(), |request| {
            matches!(request, StoreRequest::QueueOverview)
        });
    }

    /// M3 review R1 M2: the overview costs the serial store worker about ten reads, so a good
    /// read is refreshed every third refresh tick (about three seconds), not every one; a write's
    /// re-read does not wait.
    #[test]
    fn a_good_read_is_refreshed_only_every_third_refresh() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        assert_eq!(REFRESHES_PER_READ, 3);
        for round in 0..2 {
            bench.feed(&mut overlay, overview(abc()));
            for tick in 1..REFRESHES_PER_READ {
                overlay.refresh(&mut bench.ctx());
                assert!(
                    bench.drained().is_empty(),
                    "round {round}, refresh {tick}: not every second"
                );
            }
            overlay.refresh(&mut bench.ctx());
            assert_one_request(&bench.drained(), |request| {
                matches!(request, StoreRequest::QueueOverview)
            });
        }

        // A write's reply re-reads at once, whatever the cadence.
        bench.feed(&mut overlay, overview(abc()));
        bench.reply(
            &mut overlay,
            &StoreReply::QueueWritten {
                write: QueueWrite::Moved { moved: true },
                view: QueueView {
                    entries: Vec::new(),
                    open_batch: None,
                    demo: false,
                },
            },
        );
        assert!(
            bench
                .drained()
                .iter()
                .any(|action| matches!(action, Action::Store(StoreRequest::QueueOverview))),
            "the write's re-read"
        );
    }

    #[test]
    fn a_refused_write_keeps_its_own_failure_on_the_status_line() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(abc()));
        let unreachable = StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()).to_string();

        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: "pause_queue",
                message: unreachable.clone(),
            },
        );
        assert_one_request(&bench.drained(), |request| {
            matches!(request, StoreRequest::QueueOverview)
        });

        // `App::on_reply` posts the read's failure over the write's; the overlay puts the write's
        // back, so the status line names what the user asked for.
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: OVERVIEW,
                message: unreachable.clone(),
            },
        );
        match bench.drained().as_slice() {
            [Action::Error(message)] => {
                assert_eq!(message, &format!("pause_queue: {unreachable}"));
            }
            other => panic!("expected the write's failure again: {other:?}"),
        }

        // Once said, it is not said again by a later failed read.
        for _ in 0..REFRESHES_PER_RETRY {
            overlay.refresh(&mut bench.ctx());
        }
        bench.drained();
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: OVERVIEW,
                message: unreachable,
            },
        );
        assert!(bench.drained().is_empty());
    }

    #[test]
    fn a_refused_write_followed_by_a_good_read_says_nothing_more() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(abc()));
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: "move_queue_entry",
                message: "not found: entry".to_owned(),
            },
        );
        bench.drained();
        bench.feed(&mut overlay, overview(abc()));
        assert!(bench.drained().is_empty());

        // A later failed read does not dig the old write failure back up.
        bench.reply(
            &mut overlay,
            &StoreReply::Failed {
                request: OVERVIEW,
                message: "boom".to_owned(),
            },
        );
        assert!(bench.drained().is_empty());
    }

    #[test]
    fn esc_and_lower_q_pass() {
        let bench = Bench::new();
        let mut overlay = QueueOverlay::new();
        bench.feed(&mut overlay, overview(abc()));
        assert_eq!(bench.code(&mut overlay, KeyCode::Esc), Handled::Pass);
        assert_eq!(bench.code(&mut overlay, KeyCode::Char('q')), Handled::Pass);
        assert!(bench.drained().is_empty());
        assert!(overlay.is_modal(), "a passed key is swallowed, never quits");
    }

    #[test]
    fn the_overview_request_name_is_the_one_the_worker_uses() {
        assert_eq!(StoreRequest::QueueOverview.name(), OVERVIEW);
    }
}
