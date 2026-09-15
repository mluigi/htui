//! The Runs sub-tab: the item's runs, newest first, each with its steps under it.
//!
//! The steps are here because they are the way into a replay (MOD-2 D39): `J` / `K` move a cursor
//! over the flattened `(run, step)` list and `Enter` emits [`Action::Replay`] for the step under
//! it. The pane does not read the step's rows itself — it holds no store handle (`R-NF-3`) — and
//! it does not know which tab will show them either; naming the step is its whole part.

use htui_core::model::{ItemId, RunStatus, RunStepSummary, RunSummary, StepId, StepStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Cell, Row, Table};

use crate::app::{Action, Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crossterm::event::{KeyCode, KeyEvent};

/// Two lines per row: `kind` over `mode`, `started` over `finished`.
///
/// Six columns do not fit the 43 columns a detail pane has at 100x30; folding the two pairs that
/// belong together is what keeps every field of the blueprint's list visible instead of clipped.
const ROW: u16 = 2;

/// What replaces a timestamp a run has not reached yet.
const PENDING: &str = "\u{2014}";

/// Marks the step the cursor is on. Unselected step rows carry a space of the same width, so the
/// columns do not shift as the cursor moves.
const CURSOR: &str = "\u{25b8}";

/// Marks a step whose prompt was trimmed (D106). One character, because it shares the phase cell
/// with the token figure; the record that says *what* was trimmed is the `Prompt` sub-tab's.
const TRIMMED: &str = "!";

/// Width of the phase column, which D106's second line shares with the phase name (hazard H-26).
///
/// Named because it is the bound [`indicator`] has to fit in: a fifth column does not exist at 43
/// columns, so a figure too wide for this cell clips instead of wrapping.
const PHASE_WIDTH: u16 = 12;

/// The `Runs` reply as a table of kind, mode, status, box, started and finished, with one line
/// per step under each run.
#[derive(Debug, Default)]
pub struct RunsTab {
    /// The runs of the selected item, newest first.
    runs: Vec<RunSummary>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// First visible run.
    scroll: Scroll,
    /// Index into the flattened `(run, step)` list: runs in reply order, steps in each run's own
    /// order (`RunSummary.steps` is already sorted by `(position, attempt, fanout_index)`).
    ///
    /// `None` while no run has a step, which is also every state before the first `Runs` reply.
    selected: Option<usize>,
}

impl RunsTab {
    /// Identity of the Runs sub-tab.
    pub const ID: DetailId = DetailId("runs");

    /// A sub-tab with no runs yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The step under the cursor, or `None` when this item's runs have no steps.
    ///
    /// The one question the modules above this one ask the pane, and what its `Enter` replays.
    #[must_use]
    pub fn selected_step(&self) -> Option<StepId> {
        let at = self.selected?;
        self.steps().nth(at).map(|(_, step)| step)
    }

    /// The flattened `(run index, step id)` list the cursor indexes into.
    fn steps(&self) -> impl Iterator<Item = (usize, StepId)> + '_ {
        self.runs
            .iter()
            .enumerate()
            .flat_map(|(at, run)| run.steps.iter().map(move |step| (at, step.id)))
    }

    /// How many steps there are to move over.
    fn step_count(&self) -> usize {
        self.runs.iter().map(|run| run.steps.len()).sum()
    }

    /// Moves the step cursor, clamped to both ends. A pane with no step keeps `None`.
    fn move_cursor(&mut self, delta: isize) {
        let Some(last) = self.step_count().checked_sub(1) else {
            self.selected = None;
            return;
        };
        let current = self.selected.unwrap_or(0);
        self.selected = Some(current.saturating_add_signed(delta).min(last));
    }

    /// Puts the cursor on the first step, or takes it away when there is none.
    fn reselect(&mut self) {
        self.selected = (self.step_count() > 0).then_some(0);
    }

    /// First run to draw: the scrolled-to one, except that the cursor is never scrolled off the
    /// top - the offset is derived here rather than being a second thing `J` has to keep right.
    fn first_visible(&self) -> usize {
        let cursor_run = self
            .selected
            .and_then(|at| self.steps().nth(at))
            .map_or(usize::MAX, |(run, _)| run);
        self.scroll.skip().min(cursor_run)
    }
}

/// The style a run status renders with. `Theme::status_style` is about items, not runs.
fn run_style(theme: &Theme, status: RunStatus) -> Style {
    match status {
        RunStatus::Queued => theme.base,
        RunStatus::Running => Style::new().fg(Color::Cyan),
        RunStatus::AwaitingApproval => Style::new().fg(Color::Yellow),
        RunStatus::Done => theme.dim,
        RunStatus::Failed | RunStatus::Cancelled => theme.error,
    }
}

/// The style a step status renders with: the run palette, plus the two states only a step has.
fn step_style(theme: &Theme, status: StepStatus) -> Style {
    match status {
        StepStatus::Pending | StepStatus::Superseded => theme.dim,
        StepStatus::Running => Style::new().fg(Color::Cyan),
        StepStatus::AwaitingApproval => Style::new().fg(Color::Yellow),
        StepStatus::Done => theme.dim,
        StepStatus::Failed | StepStatus::Cancelled => theme.error,
    }
}

/// D106's token figure in the seven characters B.17's second line has room for.
///
/// Under a thousand is exact (`~812`), thousands are whole and **rounded** (`~36k`), and a million
/// or more takes one decimal (`~1.2M`). The `~` is not decoration: every number in a `trim_record`
/// is an estimate by a named estimator, and a step row has no room to say which one.
///
/// Total over the whole of `i32`, including the negatives a malformed `JSONB` document can put in
/// `estimated_after`: a list must render a nonsense figure, not panic on it.
fn figure(tokens: i32) -> String {
    let tokens = i64::from(tokens);
    let thousands = (tokens + 500 * tokens.signum()) / 1_000;
    if tokens.abs() < 1_000 {
        format!("~{tokens}")
    } else if thousands.abs() < 1_000 {
        format!("~{thousands}k")
    } else {
        // Rounded tenths of a million, so `999 500` reads `~1.0M` rather than `~1000k`.
        let tenths = (tokens + 50_000 * tokens.signum()) / 100_000;
        format!("~{}.{}M", tenths / 10, (tenths % 10).abs())
    }
}

/// The second line of a step row, or `None` for a step that stays one line (B.17).
///
/// The two `RunStepSummary` fields are independent — a record can carry a trim and no readable
/// `estimated_after` — so the marker renders alone rather than suppressing itself for want of a
/// number it never needed.
fn indicator(step: &RunStepSummary) -> Option<String> {
    match (step.prompt_tokens, step.trimmed) {
        (None, false) => None,
        (None, true) => Some(TRIMMED.to_owned()),
        (Some(tokens), false) => Some(figure(tokens)),
        (Some(tokens), true) => Some(format!("{} {TRIMMED}", figure(tokens))),
    }
}

impl DetailTab for RunsTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Runs"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.runs.clear();
        self.scroll.reset();
        self.selected = None;
    }

    /// `J` / `K` move the step cursor, `Enter` replays the step under it, and everything else is
    /// still the shared scroll.
    ///
    /// The cursor takes `J` / `K` off [`Scroll::on_key`] on this pane: the steps are the rows a
    /// reader moves between, and `PageDown` / `PageUp` remain the way to walk a long run list.
    /// `Enter` with no step under the cursor passes, so the Backlog tab's keymap row is what
    /// answers it — the pane never consumes a key it cannot act on.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('J') => self.move_cursor(1),
            KeyCode::Char('K') => self.move_cursor(-1),
            KeyCode::Enter => {
                let Some(step_id) = self.selected_step() else {
                    return Handled::Pass;
                };
                ctx.emit(Action::Replay { step_id });
            }
            _ => return self.scroll.on_key(key, self.runs.len()),
        }
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Runs(runs) = reply {
            self.runs = runs.clone();
            self.scroll.reset();
            self.reselect();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if self.runs.is_empty() {
            message(frame, area, "No runs for this item.", ctx.theme);
            return;
        }

        let cursor = self.selected_step();
        let mut rows: Vec<Row<'_>> = Vec::new();
        for run in self.runs.iter().skip(self.first_visible()) {
            let started = run
                .started_at
                .map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string());
            let finished = run
                .finished_at
                .map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string());
            rows.push(
                Row::new(vec![
                    Cell::from(Text::from(vec![
                        Line::styled(run.kind.as_str(), ctx.theme.base),
                        Line::styled(run.mode.as_str(), ctx.theme.dim),
                    ])),
                    Cell::from(Line::styled(
                        run.status.as_str(),
                        run_style(ctx.theme, run.status),
                    )),
                    Cell::from(Line::styled(run.box_hostname.clone(), ctx.theme.base)),
                    Cell::from(Text::from(vec![
                        Line::styled(started, ctx.theme.base),
                        Line::styled(finished, ctx.theme.dim),
                    ])),
                ])
                .height(ROW),
            );

            for step in &run.steps {
                let on_cursor = cursor == Some(step.id);
                let mark = if on_cursor { CURSOR } else { " " };
                let label = if on_cursor {
                    ctx.theme.accent
                } else {
                    ctx.theme.dim
                };
                let started = step
                    .started_at
                    .map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string());
                // D106's two indicators fold into the phase cell rather than taking a fifth
                // column: four columns and their spacing already fill the 41 of a 43-wide pane.
                let indicator = indicator(step);
                let phase = match &indicator {
                    Some(line) => Cell::from(Text::from(vec![
                        Line::styled(step.phase_name.clone(), label),
                        Line::styled(line.clone(), ctx.theme.dim),
                    ])),
                    None => Cell::from(Line::styled(step.phase_name.clone(), label)),
                };
                let row = Row::new(vec![
                    Cell::from(Line::styled(format!("{mark} {}", step.position), label)),
                    Cell::from(Line::styled(
                        step.status.as_str(),
                        step_style(ctx.theme, step.status),
                    )),
                    phase,
                    Cell::from(Line::styled(started, ctx.theme.dim)),
                ]);
                rows.push(if indicator.is_some() {
                    row.height(ROW)
                } else {
                    row
                });
            }
        }

        let header = Row::new(vec![
            Cell::from(Text::from(vec![Line::raw("kind"), Line::raw("mode")])),
            Cell::from("status"),
            Cell::from("box"),
            Cell::from(Text::from(vec![
                Line::raw("started"),
                Line::raw("finished"),
            ])),
        ])
        .height(ROW)
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(PHASE_WIDTH),
                Constraint::Length(11),
            ],
        )
        .header(header)
        .column_spacing(1);
        frame.render_widget(table, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crossterm::event::KeyModifiers;
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, Scope, WorkspaceId};
    use htui_core::store::{MemStore, ReadStore};

    /// Everything a `Ctx` borrows, kept alive for the length of a test.
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Shell {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
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
                Origin::Tab(crate::ui::tabs::BacklogTab::ID),
                &self.emit,
            )
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A Runs pane holding the fixture's `FEAT-1` runs: one run, four steps.
    async fn pane(shell: &Shell) -> RunsTab {
        let runs = MemStore::demo()
            .runs(ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails");
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        pane
    }

    #[tokio::test]
    async fn a_runs_reply_puts_the_cursor_on_the_first_step() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        assert_eq!(pane.step_count(), 4, "the FEAT-1 run has four steps");
        assert_eq!(pane.selected_step(), Some(ids::STEP_PRD));
    }

    #[tokio::test]
    async fn the_step_cursor_moves_and_clamps_at_both_ends() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;

        assert_eq!(
            pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(pane.selected_step(), Some(ids::STEP_PLAN));

        for _ in 0..8 {
            pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        }
        assert_eq!(
            pane.selected_step(),
            Some(ids::STEP_REVIEW),
            "the cursor stops on the last step rather than running off it"
        );

        for _ in 0..8 {
            pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        }
        assert_eq!(
            pane.selected_step(),
            Some(ids::STEP_PRD),
            "and stops on the first going back"
        );
    }

    #[tokio::test]
    async fn enter_emits_a_replay_of_the_step_under_the_cursor() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());

        assert_eq!(
            pane.on_key(key(KeyCode::Enter), &mut shell.ctx()),
            Handled::Consumed
        );
        let emitted = shell.emit.take();
        assert_eq!(emitted.len(), 1);
        assert!(
            matches!(&emitted[0], Action::Replay { step_id } if *step_id == ids::STEP_PLAN),
            "the pane names the step and nothing else: {:?}",
            emitted[0]
        );
    }

    /// A pane with nothing to replay consumes neither `Enter` nor a cursor it does not have, so
    /// the Backlog tab's keymap row is what answers the key.
    #[tokio::test]
    async fn a_pane_with_no_step_passes_enter_on() {
        let shell = Shell::new();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_ANA_2));

        assert_eq!(pane.selected_step(), None);
        assert_eq!(
            pane.on_key(key(KeyCode::Enter), &mut shell.ctx()),
            Handled::Pass
        );
        assert!(shell.emit.is_empty());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(pane.selected_step(), None, "nothing to move over");
    }

    /// The width a detail pane has at the 100x30 the snapshots render at: hazard H-26 is that the
    /// second line clips, and a narrower or wider test area would not see it.
    const PANE_WIDTH: u16 = 43;

    /// The pane drawn at [`PANE_WIDTH`], one `String` per row, trailing blanks trimmed.
    fn lines(pane: &RunsTab, shell: &Shell) -> Vec<String> {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(PANE_WIDTH, 16))
            .expect("the test backend is constructible");
        term.draw(|frame| pane.render(frame, frame.area(), &shell.ctx()))
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

    /// D106: the fixture's `implement` step is the one with a `trim_record`, so it is the one step
    /// that grows a second line, and the line sits under its phase name.
    #[tokio::test]
    async fn a_step_with_a_trim_record_takes_two_lines() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let lines = lines(&pane, &shell);
        let at = lines
            .iter()
            .position(|line| line.contains("implement"))
            .expect("the `implement` step is listed");
        let second = lines
            .get(at + 1)
            .expect("the `implement` row has a second line");
        assert!(
            second.contains("~36k"),
            "the figure sits in the phase cell under the phase name, not on the row above: \
             {second:?}"
        );
        assert_eq!(
            second.find("~36k"),
            lines[at].find("implement"),
            "and in the same column, because it is the same cell"
        );
    }

    /// The `!` is `trimmed`, the figure is `estimated_after`, and neither clips at [`PANE_WIDTH`]
    /// (hazard H-26).
    #[tokio::test]
    async fn the_figure_is_thousands_with_a_bang_when_trimmed() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        assert!(
            lines(&pane, &shell)
                .iter()
                .any(|line| line.contains("~36k !")),
            "the demo record is 35 988 tokens with four dropped sections"
        );

        assert_eq!(figure(812), "~812", "under a thousand is exact");
        assert_eq!(figure(999), "~999");
        assert_eq!(figure(1_000), "~1k");
        assert_eq!(
            figure(35_988),
            "~36k",
            "thousands round rather than truncate"
        );
        assert_eq!(figure(35_499), "~35k");
        assert_eq!(figure(999_499), "~999k");
        assert_eq!(figure(999_500), "~1.0M", "and never render as `~1000k`");
        assert_eq!(figure(1_234_567), "~1.2M");
        assert_eq!(figure(0), "~0");
        assert_eq!(
            figure(-7),
            "~-7",
            "a malformed record renders, it does not panic"
        );
        assert_eq!(figure(35_988), "~36k");

        // Hazard H-26 over the whole domain, not just the plausible part of it: `estimated_after`
        // is an untyped `JSONB` number and the cell it lands in does not grow.
        for tokens in [0, 999, 1_000, 999_500, 35_988, i32::MAX, i32::MIN] {
            let step = RunStepSummary {
                prompt_tokens: Some(tokens),
                trimmed: true,
                ..MemStore::demo()
                    .runs(ids::HTUI_FEAT_1)
                    .await
                    .expect("the memory store never fails")[0]
                    .steps[0]
                    .clone()
            };
            let rendered = indicator(&step).expect("a step with a figure has a second line");
            assert!(
                u16::try_from(rendered.chars().count()).expect("a short string") <= PHASE_WIDTH,
                "`{rendered}` does not fit the phase cell"
            );
        }
    }

    /// The other three fixture steps have no record, so the pane is one line per step for them —
    /// which is what keeps `backlog__detail_runs.snap` one line longer rather than four.
    #[tokio::test]
    async fn a_step_without_one_stays_one_line() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let lines = lines(&pane, &shell);
        for phase in ["prd", "plan", "review"] {
            let at = lines
                .iter()
                .position(|line| line.contains(phase))
                .unwrap_or_else(|| panic!("the `{phase}` step is listed"));
            let next = lines.get(at + 1).map_or("", String::as_str);
            assert!(
                !next.contains('~') && !next.contains('!'),
                "`{phase}` has no record and so no second line, but got {next:?}"
            );
        }
        assert_eq!(
            lines.iter().filter(|line| line.contains('~')).count(),
            1,
            "exactly one step row carries an indicator"
        );
    }

    /// A new item's reply re-seats the cursor instead of leaving it on an index of the old one.
    #[tokio::test]
    async fn a_new_item_takes_the_cursor_with_it() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(pane.selected_step(), Some(ids::STEP_PLAN));

        pane.on_item_change(Some(ids::HTUI_ANA_2));
        assert_eq!(pane.selected_step(), None);

        pane.on_reply(&StoreReply::Runs(Vec::new()), &mut shell.ctx());
        assert_eq!(
            pane.selected_step(),
            None,
            "an item with no run has no step"
        );
    }
}
