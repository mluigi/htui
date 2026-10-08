//! The waiting-on-you list (MOD-69 plan D6, D7; `R-TUI-11`): every row of the shell's last
//! `Waiting` reply, read straight from `ctx.top_bar.waiting`, so the list holds nothing but its
//! cursor and refreshes on every tick with no request of its own. `Enter` reveals the row's step;
//! the Runs pane stays the one place that answers (PRD scope).

use std::cell::Cell;

use htui_core::model::{ItemId, RunId, Scope, StepId};
use htui_worker::{WaitingReason, WaitingRow, WaitingView};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, Ctx, Handled, OverlayAction, RevealTarget};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::layout::centered;
use crate::ui::overlay::registry::{Overlay, OverlayId};
use crossterm::event::KeyEvent;

/// Marker in front of the row the cursor is on (the switcher's, `workspace_switcher.rs`): a
/// snapshot records symbols and not styles.
const CURSOR: &str = "> ";

/// The marker's width, in front of every other line, so the columns stay put.
const NO_CURSOR: &str = "  ";

/// Gap between two columns.
const GAP: &str = "  ";

/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;

/// The line under the list (MOD-67 D9): `list.down`/`list.up` and `waiting.open` are handled
/// here, `overlay.close` by the shell.
const HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::WaitingOpen, "open"),
    Hint::One(Act::OverlayClose, "close"),
];

/// What a reply with no row says.
const EMPTY: &str = "nothing is waiting on you";

/// What the list says before the first `Waiting` reply (blueprint A-6).
const READING: &str = "reading the store";

/// Offline, permission requests are not mirrored, so none can be listed (plan D4), and every row
/// is read-only: the Runs pane refuses the answer (review L3).
const OFFLINE: &str = "permissions unavailable (read-only offline)";

/// Online, the permission read failed with anything but `Unreachable` (review L4): none can be
/// listed, yet every row can still be answered, so the line claims neither offline nor read-only.
const UNREADABLE: &str = "permissions unavailable: the read failed";

/// The step column of a row with no step: the Runs pane's `PENDING` dash.
const NO_STEP: &str = "\u{2014}";

/// A row's identity across ticks (MOD-69 review M2): the list is re-read and re-sorted on every
/// tick, so an index alone would slide to another row when one sorts above it.
type RowId = (ItemId, Option<RunId>, Option<StepId>, WaitingReason);

/// `row`'s [`RowId`].
const fn id_of(row: &WaitingRow) -> RowId {
    (row.item, row.run, row.step, row.reason)
}

/// Lists what waits on a person and reveals the selected row's step.
#[derive(Debug, Default)]
pub struct WaitingList {
    /// The highlighted row's index at the last read or key; a key stores it unclamped, every read
    /// clamps it to the rows on hand (H-3) and writes back where it found the anchor, so it is the
    /// fallback when the anchor's row is gone (review R1).
    cursor: Cell<usize>,
    /// The highlighted row's identity, re-found on every read (review M2). The first read with
    /// rows sets it, so the row shown on open is anchored before any key (review R1); `None`
    /// only while no read has had a row. A `Cell` because a read (`render`) takes `&self`.
    anchor: Cell<Option<RowId>>,
}

impl WaitingList {
    /// Identity of the list: the factory and the global `Ctrl+W` binding are registered under it.
    pub const ID: OverlayId = OverlayId("waiting_list");

    /// A list with its cursor on the first row.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The highlighted row of `rows`: the anchor's row (of several sharing its identity, the one
    /// nearest the stored index), else the stored index clamped; `None` while there are none.
    /// The answer becomes the cursor, index and identity both (review R1), so whatever row a read
    /// highlights is the one the next re-sort keeps.
    fn at(&self, rows: &[WaitingRow]) -> Option<usize> {
        let last = rows.len().checked_sub(1)?;
        let cursor = self.cursor.get();
        let anchored = self.anchor.get().and_then(|anchor| {
            rows.iter()
                .enumerate()
                .filter(|(_, row)| id_of(row) == anchor)
                .map(|(index, _)| index)
                .min_by_key(|index| index.abs_diff(cursor))
        });
        let at = anchored.unwrap_or_else(|| cursor.min(last));
        self.put(rows, at);
        Some(at)
    }

    /// Puts the cursor on `rows[index]`, index and identity both.
    fn put(&self, rows: &[WaitingRow], index: usize) {
        self.cursor.set(index);
        self.anchor.set(rows.get(index).map(id_of));
    }

    /// `Enter`: close, then reveal the selected row's step, both applied by the same drain (the
    /// `ConceptsSearch` order). Nothing on an empty list.
    fn enter(&self, rows: &[WaitingRow], ctx: &Ctx<'_>) {
        let Some(row) = self.at(rows).and_then(|at| rows.get(at)) else {
            return;
        };
        ctx.emit(Action::Overlay(OverlayAction::Close));
        ctx.emit(Action::Reveal(RevealTarget::Step {
            item: row.item,
            key: row.item_key.clone(),
            run: row.run,
            step: row.step,
        }));
    }

    /// The box's contents: the visible rows (or the empty or reading line), the offline or
    /// unreadable line when permissions are unknown, a blank line, then `hint` (the rendered
    /// [`HINT`]).
    ///
    /// `text_width` is the room the text column gets; `None` is its widest text, unclipped.
    /// `visible` is how many rows fit: the window always holds the cursor.
    fn lines(
        &self,
        waiting: Option<&WaitingView>,
        text_width: Option<usize>,
        visible: usize,
        hint: &str,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = match waiting {
            None => vec![Line::styled(format!("{NO_CURSOR}{READING}"), theme.dim)],
            Some(view) if view.rows.is_empty() => {
                vec![Line::styled(format!("{NO_CURSOR}{EMPTY}"), theme.dim)]
            }
            Some(view) => {
                let columns = Columns::of(&view.rows);
                let text_width = text_width.unwrap_or(columns.text);
                let at = self.at(&view.rows).unwrap_or(0);
                let first = if visible > 0 && at >= visible {
                    at + 1 - visible
                } else {
                    0
                };
                view.rows
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(visible)
                    .map(|(index, row)| row_line(row, index == at, &columns, text_width, theme))
                    .collect()
            }
        };
        if let Some(view) = waiting.filter(|view| !view.permissions_known) {
            let why = if view.offline { OFFLINE } else { UNREADABLE };
            lines.push(Line::styled(format!("{NO_CURSOR}{why}"), theme.dim));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(format!("{NO_CURSOR}{hint}"), theme.dim));
        lines
    }
}

/// The widths of a row's columns, in cells: each the widest value in the list.
struct Columns {
    /// The item key.
    key: usize,
    /// The step label, or [`NO_STEP`].
    step: usize,
    /// The reason's label.
    label: usize,
    /// The reason's text.
    text: usize,
}

impl Columns {
    fn of(rows: &[WaitingRow]) -> Self {
        let widest = |cell: &dyn Fn(&WaitingRow) -> usize| rows.iter().map(cell).max().unwrap_or(0);
        Self {
            key: widest(&|row| cell_width(&row.item_key)),
            step: widest(&|row| cell_width(step_text(row))),
            label: widest(&|row| cell_width(row.reason.label())),
            text: widest(&|row| cell_width(&row.text)),
        }
    }

    /// Everything in front of the text: the marker, three padded columns and their gaps.
    fn prefix(&self) -> usize {
        cell_width(CURSOR) + self.key + self.step + self.label + 3 * cell_width(GAP)
    }
}

/// The step column of `row`: its label, or the dash when it has no step.
fn step_text(row: &WaitingRow) -> &str {
    if row.step_label.is_empty() {
        NO_STEP
    } else {
        &row.step_label
    }
}

/// One row: `{marker}{key}{GAP}{step}{GAP}{label}{GAP}{text}`, the first three padded to their
/// column and the text clipped to `text_width`, never wrapped.
fn row_line(
    row: &WaitingRow,
    selected: bool,
    columns: &Columns,
    text_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let marker = if selected { CURSOR } else { NO_CURSOR };
    let style = if selected { theme.accent } else { theme.base };
    Line::styled(
        format!(
            "{marker}{}{GAP}{}{GAP}{}{GAP}{}",
            cells::fit(&row.item_key, columns.key),
            cells::fit(step_text(row), columns.step),
            cells::fit(row.reason.label(), columns.label),
            cells::clip(&row.text, text_width),
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

impl Overlay for WaitingList {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Waiting on you"
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::WAITING_LIST)
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Plan D6, blueprint E13: nothing on open; the shell's tick re-reads the list.
        Vec::new()
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let rows = ctx
            .top_bar
            .waiting
            .as_ref()
            .map_or(&[][..], |view| view.rows.as_slice());
        let len = rows.len();
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::WAITING_LIST, chord) {
            match act {
                Act::ListDown => {
                    let next = self.at(rows).map_or(0, |at| (at + 1).min(len - 1));
                    self.put(rows, next);
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    let next = self.at(rows).map_or(0, |at| at.saturating_sub(1));
                    self.put(rows, next);
                    return Handled::Consumed;
                }
                Act::WaitingOpen => {
                    self.enter(rows, ctx);
                    return Handled::Consumed;
                }
                _ => continue, // `overlay.close` or `global.help`: the shell's
            }
        }
        // The shell's overlay step closes on `overlay.close`; the rest is swallowed by
        // `is_modal`, so a stray key never reaches the tab underneath.
        Handled::Pass
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        // The top bar already took the `Waiting` reply (`App::observe_reply`), whatever its
        // address; the list reads it from there.
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let waiting = ctx.top_bar.waiting.as_ref();
        let rows = waiting.map_or(&[][..], |view| view.rows.as_slice());
        let columns = Columns::of(rows);
        let fixed = 2 + usize::from(waiting.is_some_and(|view| !view.permissions_known));

        // The natural size: every row, every text whole.
        let hint = ctx.keys().hint(views::WAITING_LIST, HINT);
        let natural = self.lines(waiting, None, usize::MAX, &hint, ctx.theme);
        let width = u16::try_from(widest(&natural))
            .unwrap_or(u16::MAX)
            .saturating_add(CHROME)
            .min(area.width.saturating_sub(4));
        let height = u16::try_from(natural.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(area.height.saturating_sub(2));

        let room = usize::from(width.saturating_sub(CHROME));
        let text_width = room.saturating_sub(columns.prefix());
        let visible = usize::from(height.saturating_sub(2)).saturating_sub(fixed);
        let lines = self.lines(waiting, Some(text_width), visible, &hint, ctx.theme);

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
    use crate::app::{Action, Emit, OverlayAction, RevealTarget, TopBarState};
    use crate::keymap::Keymap;
    use crate::keys::Keys;
    use crate::store_worker::Origin;
    use crate::ui::cells::cell_width;
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemId, ProjectRef, RunId, StepId, WorkspaceId};
    use htui_worker::WaitingReason;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Everything a `Ctx` borrows; the list reads its rows from `top_bar.waiting`.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        fn over(waiting: Option<WaitingView>) -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: vec![ids::PROJECT_HTUI],
                },
                projects: Vec::new(),
                top_bar: TopBarState {
                    waiting,
                    ..TopBarState::default()
                },
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
                Origin::Overlay(WaitingList::ID),
                &self.emit,
            )
        }

        fn code(&self, list: &mut WaitingList, code: KeyCode) -> Handled {
            list.on_key(KeyEvent::from(code), &mut self.ctx())
        }

        /// The list drawn over a blank 100×30 frame, one line per row.
        fn render(&self, list: &WaitingList) -> String {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
            term.draw(|frame| list.render(frame, frame.area(), &self.ctx()))
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
    }

    fn gate(key: &str, item: ItemId) -> WaitingRow {
        WaitingRow {
            item,
            item_key: key.to_owned(),
            run: Some(RunId::new()),
            step: Some(StepId::new()),
            step_label: "prd 0.1".to_owned(),
            reason: WaitingReason::Gate,
            text: "gate".to_owned(),
        }
    }

    fn permission(key: &str, item: ItemId) -> WaitingRow {
        WaitingRow {
            reason: WaitingReason::Permission,
            text: "edit: src/lib.rs".to_owned(),
            ..gate(key, item)
        }
    }

    fn reopen(key: &str, item: ItemId) -> WaitingRow {
        WaitingRow {
            item,
            item_key: key.to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked, no active run: u reopens it".to_owned(),
        }
    }

    fn view(rows: Vec<WaitingRow>) -> WaitingView {
        WaitingView {
            working: 0,
            rows,
            permissions_known: true,
            offline: false,
        }
    }

    /// The hint rendered with the compiled keys.
    fn hint() -> String {
        Keys::compiled().hint(views::WAITING_LIST, HINT)
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

    /// `emitted` is exactly `Close`, then the reveal of `row`'s step (the `ConceptsSearch` order).
    fn assert_closes_then_reveals(emitted: &[Action], row: &WaitingRow) {
        let expected = RevealTarget::Step {
            item: row.item,
            key: row.item_key.clone(),
            run: row.run,
            step: row.step,
        };
        match emitted {
            [
                Action::Overlay(OverlayAction::Close),
                Action::Reveal(target),
            ] => {
                assert_eq!(*target, expected);
            }
            other => panic!("expected Close then Reveal: {other:?}"),
        }
    }

    #[test]
    fn the_cursor_clamps_when_rows_shrink() {
        let rows = vec![
            gate("ANA-2", ids::HTUI_ANA_2),
            permission("ANA-2", ids::HTUI_ANA_2),
            reopen("FEAT-2", ids::HTUI_FEAT_2),
        ];
        let mut bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        for _ in 0..5 {
            assert_eq!(bench.code(&mut list, KeyCode::Char('j')), Handled::Consumed);
        }
        assert_eq!(list.at(&rows), Some(2), "j stops at the last row");

        let shrunk = vec![rows[0].clone()];
        bench.top_bar.waiting = Some(view(shrunk.clone()));
        assert_eq!(
            list.at(&shrunk),
            Some(0),
            "the anchor is gone: the clamped index"
        );
        assert_eq!(bench.code(&mut list, KeyCode::Enter), Handled::Consumed);
        assert_closes_then_reveals(&bench.emit.take(), &rows[0]);
        assert_eq!(bench.code(&mut list, KeyCode::Char('k')), Handled::Consumed);
        assert_eq!(list.cursor.get(), 0, "k writes the clamped cursor back");
    }

    /// MOD-69 review M2: the list is re-sorted on every tick, so the cursor follows its row, not
    /// its index: a park that sorts above it leaves the cursor on the row it was on.
    #[test]
    fn the_cursor_follows_its_row_when_a_park_sorts_above_it() {
        let rows = vec![
            gate("ANA-2", ids::HTUI_ANA_2),
            reopen("FEAT-2", ids::HTUI_FEAT_2),
        ];
        let mut bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        bench.code(&mut list, KeyCode::Char('j'));

        let resorted = vec![
            gate("ANA-1", ItemId::new()),
            rows[0].clone(),
            rows[1].clone(),
        ];
        bench.top_bar.waiting = Some(view(resorted.clone()));
        assert_eq!(list.at(&resorted), Some(2));
        let rendered = bench.render(&list);
        assert!(rendered.contains("> FEAT-2 "), "{rendered}");
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &rows[1]);

        // `k` moves from the row the cursor is on, wherever it now sorts.
        bench.code(&mut list, KeyCode::Char('k'));
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &rows[0]);
    }

    /// MOD-69 review R1: the row highlighted on open is anchored by the first read, before any
    /// key, so a park that sorts above it leaves the highlight (and `Enter`) on the row shown.
    #[test]
    fn the_first_highlighted_row_is_anchored_before_any_key() {
        let rows = vec![reopen("FEAT-2", ids::HTUI_FEAT_2)];
        let mut bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        let opened = bench.render(&list);
        assert!(opened.contains("> FEAT-2 "), "{opened}");

        bench.top_bar.waiting = Some(view(vec![gate("ANA-2", ids::HTUI_ANA_2), rows[0].clone()]));
        let rendered = bench.render(&list);
        assert!(rendered.contains("> FEAT-2 "), "{rendered}");
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &rows[0]);
    }

    /// MOD-69 review R1: a re-found anchor writes its index back, so when its row then goes the
    /// fallback is the index it last sat at, not the one of the last key press.
    #[test]
    fn a_refound_anchor_writes_its_index_back() {
        let parked = gate("ANA-2", ids::HTUI_ANA_2);
        let blocked = reopen("FEAT-2", ids::HTUI_FEAT_2);
        let mut bench = Bench::over(Some(view(vec![parked.clone(), blocked.clone()])));
        let mut list = WaitingList::new();
        bench.render(&list);

        let above = [gate("ANA-1", ItemId::new()), gate("ANA-3", ItemId::new())];
        bench.top_bar.waiting = Some(view(vec![
            above[0].clone(),
            above[1].clone(),
            parked,
            blocked.clone(),
        ]));
        bench.render(&list);

        bench.top_bar.waiting = Some(view(vec![
            above[0].clone(),
            above[1].clone(),
            blocked.clone(),
        ]));
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &blocked);
    }

    /// Two permission requests of one step share an identity; the cursor still steps from one to
    /// the other, as the nearest match to its index wins.
    #[test]
    fn the_cursor_steps_between_rows_sharing_an_identity() {
        let first = permission("ANA-2", ids::HTUI_ANA_2);
        let second = WaitingRow {
            text: "bash: ls".to_owned(),
            ..first.clone()
        };
        let rows = vec![first, second.clone(), reopen("FEAT-2", ids::HTUI_FEAT_2)];
        let bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        bench.code(&mut list, KeyCode::Char('j'));
        assert_eq!(list.at(&rows), Some(1));
        bench.code(&mut list, KeyCode::Char('j'));
        assert_eq!(list.at(&rows), Some(2));
        bench.code(&mut list, KeyCode::Char('k'));
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &second);
    }

    #[test]
    fn enter_closes_then_reveals_the_rows_step() {
        let rows = vec![
            gate("ANA-2", ids::HTUI_ANA_2),
            reopen("FEAT-2", ids::HTUI_FEAT_2),
        ];
        let bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        bench.code(&mut list, KeyCode::Down);
        assert_eq!(bench.code(&mut list, KeyCode::Enter), Handled::Consumed);
        assert_closes_then_reveals(&bench.emit.take(), &rows[1]);
        bench.code(&mut list, KeyCode::Up);
        bench.code(&mut list, KeyCode::Enter);
        assert_closes_then_reveals(&bench.emit.take(), &rows[0]);
        assert_eq!(bench.code(&mut list, KeyCode::Esc), Handled::Pass);
    }

    /// MOD-67 M3 (D6): the list moves and opens through `list.down`/`list.up`/`waiting.open`,
    /// and chord equality includes modifiers, so `ctrl-j`, `alt-k` and `ctrl-enter` are not
    /// theirs: they pass and the modal overlay swallows them.
    #[test]
    fn a_chord_with_a_modifier_neither_moves_nor_opens() {
        let rows = vec![
            gate("ANA-2", ids::HTUI_ANA_2),
            reopen("FEAT-2", ids::HTUI_FEAT_2),
        ];
        let bench = Bench::over(Some(view(rows.clone())));
        let mut list = WaitingList::new();
        for (code, mods) in [
            (KeyCode::Char('j'), KeyModifiers::CONTROL),
            (KeyCode::Char('k'), KeyModifiers::ALT),
            (KeyCode::Enter, KeyModifiers::CONTROL),
        ] {
            let handled = list.on_key(KeyEvent::new(code, mods), &mut bench.ctx());
            assert_eq!(handled, Handled::Pass, "{code:?} {mods:?}");
        }
        assert!(bench.emit.is_empty(), "nothing opened");
        assert_eq!(list.at(&rows), Some(0), "the cursor did not move");
    }

    #[test]
    fn enter_on_an_empty_list_emits_nothing() {
        for waiting in [None, Some(view(Vec::new()))] {
            let bench = Bench::over(waiting);
            let mut list = WaitingList::new();
            assert_eq!(bench.code(&mut list, KeyCode::Char('j')), Handled::Consumed);
            assert_eq!(bench.code(&mut list, KeyCode::Enter), Handled::Consumed);
            assert!(bench.emit.is_empty());
        }
        let bench = Bench::over(Some(view(Vec::new())));
        assert!(bench.render(&WaitingList::new()).contains(EMPTY));
    }

    #[test]
    fn a_row_reads_key_step_reason_and_text() {
        let view = view(vec![
            gate("ANA-2", ids::HTUI_ANA_2),
            permission("ANA-2", ids::HTUI_ANA_2),
            reopen("FEAT-2", ids::HTUI_FEAT_2),
        ]);
        let list = WaitingList::new();
        let lines = list.lines(Some(&view), None, usize::MAX, &hint(), &Theme::default());
        assert_eq!(
            texts(&lines),
            vec![
                "> ANA-2   prd 0.1  gate        gate",
                "  ANA-2   prd 0.1  permission  edit: src/lib.rs",
                "  FEAT-2  \u{2014}        unblock     blocked, no active run: u reopens it",
                "",
                "  j/k move · Enter open · Esc close",
            ]
        );
    }

    #[test]
    fn before_the_first_reply_it_says_reading() {
        let list = WaitingList::new();
        let lines = list.lines(None, None, usize::MAX, &hint(), &Theme::default());
        assert_eq!(texts(&lines)[0], format!("{NO_CURSOR}{READING}"));
        let rendered = Bench::over(None).render(&list);
        assert!(rendered.contains(READING), "{rendered}");
        assert!(!rendered.contains(OFFLINE), "{rendered}");
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_cursor_drawn() {
        let rows: Vec<WaitingRow> = (1..=40)
            .map(|n| reopen(&format!("FEAT-{n}"), ItemId::new()))
            .collect();
        let bench = Bench::over(Some(view(rows)));
        let mut list = WaitingList::new();
        for _ in 0..34 {
            bench.code(&mut list, KeyCode::Char('j'));
        }
        let rendered = bench.render(&list);
        assert!(rendered.contains("> FEAT-35 "), "{rendered}");
        assert!(!rendered.contains("FEAT-1 "), "{rendered}");
        assert!(rendered.contains(&hint()), "{rendered}");
    }

    #[test]
    fn offline_it_says_permissions_are_unavailable() {
        let bench = Bench::over(Some(WaitingView {
            working: 1,
            rows: vec![
                gate("ANA-2", ids::HTUI_ANA_2),
                reopen("FEAT-2", ids::HTUI_FEAT_2),
            ],
            permissions_known: false,
            offline: true,
        }));
        insta::assert_snapshot!("offline", bench.render(&WaitingList::new()));
    }

    /// MOD-69 review R1 (L3 with L4): online, a failed permission read leaves the permissions
    /// unknown, but the rows can still be answered, so the line claims neither offline nor
    /// read-only.
    #[test]
    fn online_a_failed_permission_read_does_not_claim_offline() {
        let bench = Bench::over(Some(WaitingView {
            working: 1,
            rows: vec![gate("ANA-2", ids::HTUI_ANA_2)],
            permissions_known: false,
            offline: false,
        }));
        let rendered = bench.render(&WaitingList::new());
        assert!(rendered.contains(UNREADABLE), "{rendered}");
        assert!(!rendered.contains("offline"), "{rendered}");
        assert!(!rendered.contains("read-only"), "{rendered}");
    }
}
