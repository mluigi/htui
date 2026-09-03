//! The Runs sub-tab: the item's runs, newest first.

use htui_core::model::{ItemId, RunStatus, RunSummary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Cell, Row, Table};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crossterm::event::KeyEvent;

/// Two lines per row: `kind` over `mode`, `started` over `finished`.
///
/// Six columns do not fit the 43 columns a detail pane has at 100x30; folding the two pairs that
/// belong together is what keeps every field of the blueprint's list visible instead of clipped.
const ROW: u16 = 2;

/// What replaces a timestamp a run has not reached yet.
const PENDING: &str = "\u{2014}";

/// The `Runs` reply as a table of kind, mode, status, box, started and finished.
#[derive(Debug, Default)]
pub struct RunsTab {
    /// The runs of the selected item, newest first.
    runs: Vec<RunSummary>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// First visible run.
    scroll: Scroll,
}

impl RunsTab {
    /// Identity of the Runs sub-tab.
    pub const ID: DetailId = DetailId("runs");

    /// A sub-tab with no runs yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.runs.len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Runs(runs) = reply {
            self.runs = runs.clone();
            self.scroll.reset();
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

        let rows = self.runs.iter().skip(self.scroll.skip()).map(|run| {
            let started = run
                .started_at
                .map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string());
            let finished = run
                .finished_at
                .map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string());
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
            .height(ROW)
        });

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
                Constraint::Length(12),
                Constraint::Length(11),
            ],
        )
        .header(header)
        .column_spacing(1);
        frame.render_widget(table, area);
    }
}
