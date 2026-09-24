//! The Runs sub-tab: the item's runs, newest first, each with its steps under it.
//!
//! The steps are here because they are the way into a replay (MOD-2 D39): `J` / `K` move a cursor
//! over the flattened `(run, step)` list and `Enter` emits [`Action::Replay`] for the step under
//! it. The pane does not read the step's rows itself — it holds no store handle (`R-NF-3`) — and
//! it does not know which tab will show them either; naming the step is its whole part.
//!
//! Every step is two lines at the pane's 43 columns (MOD-4 plan D169, blueprint D197): slot,
//! status, phase, usage and duration, then the gate, the prompt figure and the agent/model.

use htui_core::model::{
    ItemId, RunStatus, RunStepSummary, RunSummary, StepId, StepStatus, UsageTotals,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use serde_json::Value;

use crate::app::{Action, Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crossterm::event::{KeyCode, KeyEvent};

/// The width of a detail pane at 100x30, which every step line fills exactly (MOD-4 plan D169,
/// blueprint D197).
const PANE: usize = 43;

/// The run grid: `kind`, `status`, `box`, `started`, single-spaced (41 columns). Two lines per run:
/// `kind` over `mode`, `started` over `finished`, because six columns do not fit the 43 a detail
/// pane has at 100x30 and folding the pairs that belong together keeps every field visible.
const RUN_GRID: [usize; 4] = [6, 9, 12, 11];

/// The step grid's line 1 (D197): cursor, slot, status, phase, usage, duration, single-spaced.
/// `1 + 5 + 10 + 11 + 6 + 5` plus five spaces is exactly [`PANE`].
const SLOT_WIDTH: usize = 5;
/// See [`SLOT_WIDTH`].
const STATUS_WIDTH: usize = 10;
/// See [`SLOT_WIDTH`].
const USAGE_WIDTH: usize = 6;
/// See [`SLOT_WIDTH`].
const DURATION_WIDTH: usize = 5;

/// The step grid's line 2 (D197): an indent to the status column, the gate, then a tail that
/// starts in the phase column. `8 + 10 + 1 + 24` is exactly [`PANE`].
const INDENT: usize = 8;
/// See [`INDENT`].
const GATE_WIDTH: usize = 10;
/// See [`INDENT`].
const TAIL_WIDTH: usize = 24;

/// What replaces a timestamp a run has not reached yet, and any figure a step does not have.
const PENDING: &str = "\u{2014}";

/// The duration of a step that has not finished: no clock is read in `render` (D170).
const RUNNING: &str = "\u{2026}";

/// Marks a cut in [`fit`].
const CUT: char = '\u{2026}';

/// Marks the step the cursor is on. Unselected step rows carry a space of the same width, so the
/// columns do not shift as the cursor moves.
const CURSOR: &str = "\u{25b8}";

/// Marks a step whose prompt was trimmed (D106). One character, because it shares the tail with
/// the token figure; the record that says *what* was trimmed is the `Prompt` sub-tab's.
const TRIMMED: &str = "!";

/// Width of the phase column (D197), which is also where line 2's tail, D106's indicator first,
/// starts (hazard H-26).
///
/// Named because it is the bound [`indicator`] has to fit in: the widest figure, `~-2147.5M !`
/// over `i32::MIN`, is exactly this wide.
const PHASE_WIDTH: usize = 11;

/// The `Runs` reply as a table of kind, mode, status, box, started and finished, with two lines
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

/// D106's token figure, which leads line 2's tail.
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

/// D106's prompt indicator, or `None` for a step no assembler wrote a record for.
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

/// `text` in exactly `width` characters: space-padded, or cut with `…` as its last character.
///
/// Counted in `char`s, like the text field (`ui/text_field.rs`). A control character, a newline in
/// a failure sentence say, becomes a space: one line is one line.
fn fit(text: &str, width: usize) -> String {
    todo!("{text} {width}")
}

/// The run grid's two header lines, `kind status box started` over `mode … finished`.
fn header_lines(theme: &Theme) -> [Line<'static>; 2] {
    todo!("{theme:?}")
}

/// One run's lines: `kind status box started`, `mode … finished`, and the failure when there is
/// one, fitted to the pane.
fn run_lines(run: &RunSummary, theme: &Theme) -> Vec<Line<'static>> {
    todo!("{run:?} {theme:?}")
}

/// One step's two lines (D197). `siblings` are the steps of its run: a fan-out slot is known by a
/// sibling at the same `(position, attempt)` with a non-zero `fanout_index`.
fn step_lines(
    step: &RunStepSummary,
    siblings: &[RunStepSummary],
    on_cursor: bool,
    theme: &Theme,
) -> [Line<'static>; 2] {
    todo!("{step:?} {siblings:?} {on_cursor} {theme:?}")
}

/// D170's usage cell: dollars when the usage document carries a cost, else tokens, else `—`. At
/// most [`USAGE_WIDTH`] characters over the whole of `i64`.
fn usage_cell(usage: Option<&Value>) -> String {
    todo!("{usage:?}")
}

/// D170's duration cell: `finished_at - started_at`, `…` while running, `—` before starting. At
/// most [`DURATION_WIDTH`] characters.
fn duration_cell(step: &RunStepSummary) -> String {
    todo!("{step:?}")
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
        let mut lines: Vec<Line<'static>> = header_lines(ctx.theme).into();
        for run in self.runs.iter().skip(self.first_visible()) {
            lines.extend(run_lines(run, ctx.theme));
            for step in &run.steps {
                lines.extend(step_lines(
                    step,
                    &run.steps,
                    cursor == Some(step.id),
                    ctx.theme,
                ));
            }
        }
        frame.render_widget(Paragraph::new(lines), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use chrono::TimeDelta;
    use crossterm::event::KeyModifiers;
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{GateOutcome, ProjectRef, Scope, WorkspaceId};
    use htui_core::store::{MemStore, ReadStore};
    use serde_json::json;

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

    /// The fixture's `FEAT-1` runs: one run, four steps.
    async fn feat_1_runs() -> Vec<RunSummary> {
        MemStore::demo()
            .runs(ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails")
    }

    /// A Runs pane holding the fixture's `FEAT-1` runs: one run, four steps.
    async fn pane(shell: &Shell) -> RunsTab {
        let runs = feat_1_runs().await;
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

    /// The char column `needle` starts at in `line`: the grid is counted in characters, and the
    /// cursor, the gate's `—` and a cut's `…` are several bytes each.
    fn column(line: &str, needle: &str) -> Option<usize> {
        line.find(needle).map(|byte| line[..byte].chars().count())
    }

    /// D106: the fixture's `implement` step is the one with a `trim_record`, and its figure leads
    /// the tail of the step's second line, which starts in the phase column.
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
            "the figure sits on the step's second line, not on the row above: {second:?}"
        );
        assert_eq!(
            column(second, "~36k"),
            column(&lines[at], "implement"),
            "and in the same column: the tail starts where the phase does"
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
                ..feat_1_runs().await[0].steps[0].clone()
            };
            let rendered = indicator(&step).expect("a step with a figure has a second line");
            assert!(
                rendered.chars().count() <= PHASE_WIDTH,
                "`{rendered}` does not fit the phase cell"
            );
        }
    }

    /// Every step is two lines (D169), whether or not it has a prompt record: the second line
    /// always carries the gate and the agent/model, so the figure no longer decides the height.
    #[tokio::test]
    async fn every_step_takes_two_lines() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let lines = lines(&pane, &shell);
        for phase in ["prd", "plan", "implement", "review"] {
            let at = lines
                .iter()
                .position(|line| line.contains(phase))
                .unwrap_or_else(|| panic!("the `{phase}` step is listed"));
            let next = lines.get(at + 1).map_or("", String::as_str);
            let gate = next.chars().skip(INDENT).take(GATE_WIDTH).collect::<String>();
            assert!(
                next.chars().take(INDENT).all(|c| c == ' ') && !gate.trim().is_empty(),
                "`{phase}`'s second line starts with its gate at the status column: {next:?}"
            );
        }
        assert_eq!(
            lines.iter().filter(|line| line.contains('~')).count(),
            1,
            "exactly one step carries a prompt figure"
        );
        assert_eq!(
            lines.iter().filter(|line| !line.is_empty()).count(),
            2 + 2 + 4 * 2,
            "the header, the run and four two-line steps"
        );
    }

    /// The text of a line, spans joined.
    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|span| span.content.as_ref()).collect()
    }

    /// A 40-character agent name.
    const LONG_AGENT: &str = "an-agent-whose-name-is-forty-characters!";

    /// A 60-character model id.
    const LONG_MODEL: &str = "a-model-id-sixty-characters-long-which-no-column-could-hold-";

    /// Every fixture step, plus the synthetic extremes of blueprint §9.6.
    async fn extremes() -> Vec<RunStepSummary> {
        let mut steps = feat_1_runs().await[0].steps.clone();
        let base = steps[0].clone();
        steps.push(RunStepSummary {
            phase_name: "research:judge".to_owned(),
            position: 123,
            attempt: 45,
            fanout_index: -1,
            ..base.clone()
        });
        steps.push(RunStepSummary {
            status: StepStatus::AwaitingApproval,
            gate_outcome: Some(GateOutcome::Rejected),
            promoted_at: Some(demo_at(1, 0)),
            selected: Some(true),
            ..base.clone()
        });
        steps.push(RunStepSummary {
            prompt_tokens: Some(i32::MIN),
            trimmed: true,
            agent_name: Some(LONG_AGENT.to_owned()),
            model: Some(LONG_MODEL.to_owned()),
            ..base.clone()
        });
        steps.push(RunStepSummary {
            usage: Some(json!({ "cost_micros": i64::MAX })),
            started_at: Some(demo_at(0, 0)),
            finished_at: Some(demo_at(0, 0) + TimeDelta::hours(1_000)),
            ..base.clone()
        });
        steps.push(RunStepSummary {
            usage: Some(json!({ "input_tokens": i64::MAX, "output_tokens": i64::MAX })),
            status: StepStatus::Superseded,
            ..base
        });
        steps
    }

    /// D169, D197: every step line is exactly the pane's 43 columns, with each field in its own
    /// column: status at 8, phase at 19, usage at 31 and duration at 38.
    #[tokio::test]
    async fn every_step_row_fits_forty_three_columns() {
        let theme = Theme::default();
        let steps = extremes().await;
        for step in &steps {
            for on_cursor in [false, true] {
                let [first, second] = step_lines(step, &steps, on_cursor, &theme);
                for line in [&first, &second] {
                    assert_eq!(
                        line.width(),
                        PANE,
                        "{:?} is not {PANE} columns wide",
                        text(line)
                    );
                }
                let chars: Vec<char> = text(&first).chars().collect();
                for gap in [1, 7, 18, 30, 37] {
                    assert_eq!(
                        chars[gap], ' ',
                        "column {gap} separates two cells: {chars:?}"
                    );
                }
                let cell = |from: usize, to: usize| -> String {
                    chars[from..to].iter().collect::<String>().trim().to_owned()
                };
                let status = if step.status == StepStatus::AwaitingApproval {
                    "awaiting"
                } else {
                    step.status.as_str()
                };
                assert_eq!(cell(8, 18), status, "the status starts at column 8");
                assert_eq!(
                    cell(19, 30),
                    fit(&step.phase_name, PHASE_WIDTH).trim(),
                    "the phase starts at column 19"
                );
                assert_eq!(
                    cell(31, 37),
                    usage_cell(step.usage.as_ref()),
                    "usage at 31"
                );
                assert_eq!(cell(38, 43), duration_cell(step), "duration at 38");
                assert_eq!(chars[0], if on_cursor { '\u{25b8}' } else { ' ' });

                let tail: Vec<char> = text(&second).chars().collect();
                assert!(tail[..INDENT].iter().all(|c| *c == ' '), "{tail:?}");
                assert_eq!(tail[INDENT + GATE_WIDTH], ' ', "{tail:?}");
            }
        }

        let [judge, _] = step_lines(&steps[4], &steps, false, &theme);
        assert!(
            text(&judge).starts_with("  123.\u{2026}"),
            "a slot too wide for its cell is cut: {:?}",
            text(&judge)
        );
        let [_, gate] = step_lines(&steps[5], &steps, false, &theme);
        assert!(
            text(&gate).contains("rejected*\u{2713}"),
            "promoted and selected both mark the gate: {:?}",
            text(&gate)
        );
        let [_, long] = step_lines(&steps[6], &steps, false, &theme);
        assert!(
            text(&long).trim_end().ends_with('\u{2026}'),
            "a long agent/model is cut with `…`: {:?}",
            text(&long)
        );
        assert!(text(&long).contains("~-2147.5M !"), "{:?}", text(&long));
    }

    /// The slot names the fan-out candidate, and the judge by `j`.
    #[tokio::test]
    async fn a_fanout_slot_names_its_candidates() {
        let base = feat_1_runs().await[0].steps[0].clone();
        let steps: Vec<RunStepSummary> = [0, 1, -1]
            .into_iter()
            .map(|fanout_index| RunStepSummary {
                position: 2,
                attempt: 1,
                fanout_index,
                ..base.clone()
            })
            .collect();
        let theme = Theme::default();
        let slots: Vec<String> = steps
            .iter()
            .map(|step| {
                let [first, _] = step_lines(step, &steps, false, &theme);
                text(&first).chars().skip(2).take(SLOT_WIDTH).collect::<String>()
            })
            .collect();
        assert_eq!(slots, ["2.1/0", "2.1/1", "2.1/j"]);

        let [single, _] = step_lines(&base, std::slice::from_ref(&base), false, &theme);
        assert!(
            text(&single).starts_with("  0.1   "),
            "a step alone in its slot is `p.a`: {:?}",
            text(&single)
        );
    }

    /// D169: a run with a `failure` gains a third header line, fitted to the pane.
    #[tokio::test]
    async fn a_run_with_a_failure_gets_a_third_line_that_fits() {
        let theme = Theme::default();
        let run = feat_1_runs().await.remove(0);
        assert_eq!(run_lines(&run, &theme).len(), 2, "no failure, two lines");

        let failed = RunSummary {
            status: RunStatus::Failed,
            failure: Some(format!(
                "stage 3 refused the prompt:\n{}",
                "a sentence far too long for the pane ".repeat(3)
            )),
            ..run
        };
        let lines = run_lines(&failed, &theme);
        assert_eq!(lines.len(), 3);
        let third = text(&lines[2]);
        assert_eq!(lines[2].width(), PANE, "{third:?}");
        assert!(
            third.starts_with("stage 3 refused the prompt: a"),
            "{third:?}"
        );
        assert!(third.ends_with('\u{2026}'), "{third:?}");
    }

    /// The run grid is today's, byte for byte, and `awaiting_approval` reads `awaiting`.
    #[tokio::test]
    async fn the_run_lines_keep_the_run_grid() {
        let theme = Theme::default();
        let [kind, mode] = header_lines(&theme);
        assert_eq!(
            text(&kind).trim_end(),
            "kind   status    box          started"
        );
        assert_eq!(
            text(&mode).trim_end(),
            "mode                          finished"
        );

        let run = feat_1_runs().await.remove(0);
        let lines = run_lines(&run, &theme);
        assert_eq!(
            text(&lines[0]).trim_end(),
            "graph  done      DESKTOP-HTUI 09-02 08:00"
        );
        assert_eq!(
            text(&lines[1]).trim_end(),
            "manual                        09-02 12:00"
        );

        let parked = RunSummary {
            status: RunStatus::AwaitingApproval,
            finished_at: None,
            ..run
        };
        let lines = run_lines(&parked, &theme);
        assert!(text(&lines[0]).starts_with("graph  awaiting  DESKTOP-HTUI"));
        assert_eq!(
            text(&lines[1]).trim_end(),
            "manual                        \u{2014}"
        );
    }

    /// D170: no clock is read in `render`, so a step still running has no duration yet.
    #[tokio::test]
    async fn a_running_step_shows_no_duration() {
        let running = RunStepSummary {
            status: StepStatus::Running,
            finished_at: None,
            ..feat_1_runs().await[0].steps[0].clone()
        };
        assert_eq!(duration_cell(&running), "\u{2026}");
        let [first, _] = step_lines(
            &running,
            std::slice::from_ref(&running),
            false,
            &Theme::default(),
        );
        assert!(text(&first).ends_with("    \u{2026}"), "{:?}", text(&first));

        let unstarted = RunStepSummary {
            started_at: None,
            ..running
        };
        assert_eq!(duration_cell(&unstarted), PENDING);
    }

    /// D170 over the whole domain: the two cells never outgrow their width.
    #[test]
    fn usage_and_duration_cells_never_exceed_their_width() {
        assert_eq!(usage_cell(None), PENDING);
        assert_eq!(usage_cell(Some(&json!({}))), PENDING);
        assert_eq!(usage_cell(Some(&json!("not an object"))), PENDING);
        assert_eq!(
            usage_cell(Some(&json!({ "cost_micros": 420_000 }))),
            "$0.42"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "cost_micros": 12_340_000 }))),
            "$12.34"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "cost_micros": 123_000_000 }))),
            "$123"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "cost_micros": 100_000_000_000_i64 }))),
            ">$99k"
        );
        assert_eq!(
            usage_cell(Some(
                &json!({ "cost_micros": 0, "input_tokens": 700, "output_tokens": 112 })
            )),
            "812",
            "no cost falls back to tokens"
        );
        assert_eq!(usage_cell(Some(&json!({ "input_tokens": 12_000 }))), "12k");
        assert_eq!(
            usage_cell(Some(&json!({ "output_tokens": 1_234_567 }))),
            "1.2M"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "output_tokens": 999_500 }))),
            "1.0M"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "output_tokens": 45_000_000 }))),
            "45M"
        );
        assert_eq!(
            usage_cell(Some(&json!({ "output_tokens": 1_000_000_000 }))),
            ">999M"
        );
        assert_eq!(usage_cell(Some(&json!({ "input_tokens": -5 }))), PENDING);

        let mut figures = vec![i64::MIN, -1, 0, 1, i64::MAX];
        for exponent in 0..19 {
            let power = 10_i64.pow(exponent);
            figures.extend([power - 1, power, power + power / 2, 5 * power - 1]);
        }
        for figure in figures {
            for document in [
                json!({ "cost_micros": figure }),
                json!({ "input_tokens": figure, "output_tokens": figure }),
                json!({ "input_tokens": figure }),
            ] {
                let cell = usage_cell(Some(&document));
                assert!(
                    cell.chars().count() <= USAGE_WIDTH,
                    "`{cell}` from {document} is wider than {USAGE_WIDTH}"
                );
            }
        }

        let base = RunStepSummary {
            id: StepId::new(),
            position: 0,
            attempt: 1,
            fanout_index: 0,
            phase_name: "prd".to_owned(),
            agent_id: None,
            model: None,
            status: StepStatus::Done,
            gate_outcome: None,
            started_at: Some(demo_at(0, 0)),
            finished_at: None,
            prompt_tokens: None,
            trimmed: false,
            usage: None,
            selected: None,
            exit_code: None,
            verify_outcome: None,
            promoted_at: None,
            agent_name: None,
        };
        let span = |seconds: i64| RunStepSummary {
            finished_at: Some(demo_at(0, 0) + TimeDelta::seconds(seconds)),
            ..base.clone()
        };
        assert_eq!(duration_cell(&span(45)), "45s");
        assert_eq!(duration_cell(&span(12 * 60 + 59)), "12m");
        assert_eq!(duration_cell(&span(3_600 + 4 * 60)), "1h04");
        assert_eq!(duration_cell(&span(99 * 3_600 + 59 * 60 + 59)), "99h59");
        assert_eq!(duration_cell(&span(100 * 3_600)), ">99h");
        assert_eq!(
            duration_cell(&span(-30)),
            "0s",
            "a negative span is no time at all"
        );
        let mut seconds = vec![
            0,
            59,
            60,
            3_599,
            3_600,
            359_999,
            360_000,
            i64::from(i32::MAX),
        ];
        seconds.extend((0..40).map(|exponent| 3_i64.pow(exponent) / 7));
        for seconds in seconds {
            let cell = duration_cell(&span(seconds));
            assert!(
                cell.chars().count() <= DURATION_WIDTH,
                "`{cell}` for {seconds}s is wider than {DURATION_WIDTH}"
            );
        }
    }

    #[test]
    fn fit_pads_cuts_and_flattens() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcd", 4), "abcd");
        assert_eq!(fit("abcde", 4), "abc\u{2026}");
        assert_eq!(fit("a\nb", 3), "a b");
        assert_eq!(fit("\u{2014}", 2), "\u{2014} ", "counted in chars, not bytes");
        assert_eq!(fit("abc", 0), "");
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
