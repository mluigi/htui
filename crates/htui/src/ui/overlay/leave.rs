//! The leave dialog (MOD-115 M2 D6, blueprint §9): what leaving htui stops, `Enter` (or a second
//! `ctrl-c`) leaves, `Esc` stays. It replaces the quit question, and `global.quit` always opens
//! it (`R-TUI-18`).
//!
//! The wording promises nothing about work surviving the exit (MD6, PRD Q3): leaving closes htui
//! and the in-process work stops with it (`RunRuntime::shutdown` aborts after a grace window).
//!
//! What is live (MOD-116 M2 D9, `R-TUI-18`): the open workspace's row from the same counts the
//! header reads (`TopBarState::waiting`), shown when it is live, an in-pane edit is open
//! (`Ctx::open_edit`) or its counts are unknown (before the first count lands the dialog cannot
//! know, so it says work may be running and shows `–` for both counts); then one row per other
//! workspace in the background or stopping, from the shell's run counts
//! (`TopBarState::workspace_runs`), in the order of the `Workspaces` reply the dialog asks for
//! when pushed. Another workspace's "waiting" counts its runs parked at a gate: the counts read
//! carries no question rows. Before the names land those rows fold into one.
//!
//! Offline (`TopBarState::offline`) the counts held are the last ones read (R1 F2): no other
//! workspace gets a row (one folded line says work may be running there) and `S` is not offered.
//!
//! `S` (`leave.stop`) stops them gracefully first: offered when the counts are known and current
//! and some workspace has a live run and no stop row, or a running run under one, it writes one
//! graceful `StopWorkspaces` for every live workspace without a stop row, then waits, reading the
//! counts once a second, until every waited workspace has no running run, or `htui stop`'s 20 s
//! wait has passed since `S` (wall time, the write's own wait included, R1 F7), and leaves. A
//! target whose stop row changed meanwhile is waited for (R1 F3); one whose write failed is named
//! and not (R1 F1), and when no target was written and nothing else is waited for, `S` is offered
//! again. `Enter` leaves at once and `Esc` stays in every stage; written stops stand. Leaving
//! still runs `RunRuntime::shutdown` (MOD-125 Q5), whose walks are by then stopped.
//!
//! Every answer resolves through [`Keys`] on [`views::LEAVE`], and the chips name the chords in
//! force. Shaped like the quit question it replaced: a centred, modal box sized to its text.
//! `register_all` registers it as `App::leave_overlay`.
//!
//! [`Keys`]: crate::keys::Keys

use crossterm::event::KeyEvent;
use htui_core::model::{
    ItemState, Scope, StopMode, WorkspaceId, WorkspaceLifecycle, WorkspaceRunCounts,
    WorkspaceSummary,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tokio::time::Instant;

use crate::app::{Action, Ctx, Handled, OverlayAction};
use crate::keys::{Act, KeyChord, Stack, views};
use crate::stop_report::{STOP_WAIT, StopTarget, StopWrite};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::chips::chip_line;
use crate::ui::kit::modal::{Modal, ModalSize};
use crate::ui::kit::status::mark;
use crate::ui::overlay::registry::{Overlay, OverlayId};

/// The box's title, which is also its question.
const TITLE: &str = "Leave htui?";

/// What the box says when something is live (D6).
const LIVE: &str = "Leaving closes htui. Work it is running stops with it:";

/// What the box says when nothing is live (D6).
const NOTHING_LIVE: &str = "Nothing is running. Leave htui?";

/// The open edit's row (D6): leaving drops the editor with `App`.
const OPEN_EDIT: &str = "\u{270e} an open edit (unsaved changes are lost)";

/// A count the dialog does not know yet: no `Waiting` reply has landed.
const UNKNOWN: &str = "\u{2013}";

/// The margin in front of each line of the box.
const LEAD: &str = " ";

/// The indent of a live row under the sentence.
const ROW: &str = "  ";

/// Between the two counts.
const SEP: &str = " \u{b7} ";

/// Between the counts and the workspace name.
const WORKSPACE_GAP: &str = "   ";

/// Between a stopping workspace's name and its `◌ stopping…` mark.
const STOPPING_GAP: &str = "  ";

/// The words after the chips.
const LEAVE_WORD: &str = "leave";
const STAY_WORD: &str = "stay";
const STOP_WORD: &str = "stop first";

/// The words after the chips while the stops settle (D9).
const LEAVE_NOW_WORD: &str = "leave now";
const STAY_STANDS_WORD: &str = "stay (the stops stand)";

/// The `S` line's second half (D9); its first names the chord and the wait.
const STOP_THEN: &str = "then leaves; each resumes later.";

/// Offline, in place of the other workspaces' rows (R1 F2): their counts are not current.
const OTHERS_UNKNOWN: &str = "work may be running in other workspaces";

/// Asks before `global.quit` closes htui, naming what stops with it.
#[derive(Debug, Default)]
pub struct LeaveDialog {
    /// Every workspace's name, in the `Workspaces` reply's order; `None` until it lands.
    names: Option<Vec<WorkspaceSummary>>,
    /// Where `S` has got to.
    stage: Stage,
}

/// Where the dialog is (D9): asking, or `S`'s stop written and settling.
#[derive(Debug, Default)]
enum Stage {
    /// The question, `S` offered when something can stop.
    #[default]
    Asking,
    /// `S` sent the stop request; nothing answered yet.
    Writing {
        /// The workspaces the dialog waits for once the stops are written.
        waited: Vec<WorkspaceId>,
        /// When `S` was answered: the bound runs from here (R1 F7).
        began: Instant,
    },
    /// D9: waiting for `waited` to settle.
    Settling {
        /// The workspaces whose running runs the dialog waits for.
        waited: Vec<WorkspaceId>,
        /// Targets whose stop row changed while asking, left as they are.
        changed: usize,
        /// R1 F1: targets whose write failed, each with its refusal; not waited for.
        failed: Vec<(WorkspaceId, String)>,
        /// When `S` was answered: the dialog leaves once `STOP_WAIT` has passed since (R1 F7).
        began: Instant,
        /// The waited rows' running runs, as last read.
        running: u32,
    },
    /// The write was refused (the sentence to show); `S` is offered again.
    Refused(String),
}

impl LeaveDialog {
    /// Identity of the dialog; Task 6's `register_all` registers the factory under it.
    pub const ID: OverlayId = OverlayId("leave");

    /// The dialog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The box's contents for its stage: the question while asking (or refused), the settling
    /// box once `S` has been answered.
    fn lines(&self, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        match &self.stage {
            Stage::Asking => self.question(ctx, None),
            Stage::Refused(message) => self.question(ctx, Some(message)),
            Stage::Writing { waited, .. } => self.settling_lines(ctx, waited.len(), None, 0, &[]),
            Stage::Settling {
                waited,
                changed,
                failed,
                began,
                running,
            } => self.settling_lines(
                ctx,
                waited.len(),
                Some((*running, seconds_left(*began))),
                *changed,
                failed,
            ),
        }
    }

    /// `workspace`'s name from the `Workspaces` reply, `a workspace` before it lands.
    fn name_of(&self, workspace: WorkspaceId) -> String {
        self.names
            .iter()
            .flatten()
            .find(|summary| summary.workspace_id == workspace)
            .map_or_else(|| "a workspace".to_owned(), |summary| summary.name.clone())
    }

    /// The question: the sentence, the live rows when something is live, the `S` line when it is
    /// offered, the refusal when the last stop was refused, a blank line, then the answers as key
    /// chips. Live means a run working or a row waiting in the open workspace, an open in-pane
    /// edit, counts not known yet (the dialog does not claim that nothing runs before it knows),
    /// or another workspace in the background or stopping. The chip labels come from `Keys`
    /// through [`views::LEAVE`], so a rebound answer shows its new chord.
    fn question(&self, ctx: &Ctx<'_>, refused: Option<&str>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let counts = ctx
            .top_bar
            .waiting
            .as_ref()
            .map(|view| (view.working, view.waiting()));
        let editing = ctx.open_edit().is_some();
        let open_live = editing || counts.is_none_or(|(working, waiting)| working + waiting > 0);
        let others = self.other_rows(ctx);
        let offered = offers_stop(ctx);

        let mut lines = Vec::new();
        if open_live || !others.is_empty() {
            lines.push(Line::styled(format!("{LEAD}{LIVE}"), theme.base));
            if open_live {
                lines.push(Self::counts_row(ctx, counts));
            }
            lines.extend(others);
            if editing {
                lines.push(Line::styled(format!("{LEAD}{ROW}{OPEN_EDIT}"), theme.base));
            }
        } else {
            lines.push(Line::styled(format!("{LEAD}{NOTHING_LIVE}"), theme.base));
        }
        let keys = ctx.keys();
        let stop_label = keys.label(views::LEAVE, Act::LeaveStop).unwrap_or_default();
        if offered {
            lines.push(Line::raw(""));
            let secs = STOP_WAIT.as_secs();
            lines.push(Line::styled(
                format!(
                    "{LEAD}{stop_label} stops them gracefully first, waits up to {secs} s for \
                     their runs to stop,"
                ),
                theme.base,
            ));
            lines.push(Line::styled(format!("{LEAD}{STOP_THEN}"), theme.base));
        }
        if let Some(sentence) = refused {
            lines.push(Line::raw(""));
            lines.push(Line::styled(format!("{LEAD}{sentence}"), theme.error));
        }
        lines.push(Line::raw(""));

        let mut answers = vec![
            (
                keys.label(views::LEAVE, Act::LeaveConfirm)
                    .unwrap_or_default(),
                LEAVE_WORD,
            ),
            (
                keys.label(views::LEAVE, Act::LeaveStay).unwrap_or_default(),
                STAY_WORD,
            ),
        ];
        if offered {
            answers.push((stop_label, STOP_WORD));
        }
        lines.push(Self::chips(ctx, &answers));
        lines
    }

    /// The settling box (D9): how many workspaces stop, how many runs are still running and the
    /// seconds left (once the write is answered), the targets left as they were, each target
    /// whose write failed (R1 F1), then the two answers that still hold.
    fn settling_lines(
        &self,
        ctx: &Ctx<'_>,
        stopping: usize,
        running: Option<(u32, u64)>,
        changed: usize,
        failed: &[(WorkspaceId, String)],
    ) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let workspaces = plural(stopping, "workspace");
        let mut lines = vec![Line::styled(
            format!("{LEAD}Stopping {stopping} {workspaces} gracefully, then leaving\u{2026}"),
            theme.base,
        )];
        if let Some((running, left)) = running {
            let runs = plural(usize::try_from(running).unwrap_or(usize::MAX), "run");
            lines.push(Line::styled(
                format!("{LEAD}{ROW}waiting for {running} running {runs} to stop ({left} s left)"),
                theme.base,
            ));
        }
        if changed > 0 {
            lines.push(Line::styled(
                format!("{LEAD}{ROW}{changed} changed meanwhile, left as they are"),
                theme.dim,
            ));
        }
        for (workspace, message) in failed {
            lines.push(Line::styled(
                format!(
                    "{LEAD}{ROW}could not stop {}: {message}",
                    self.name_of(*workspace)
                ),
                theme.error,
            ));
        }
        lines.push(Line::raw(""));
        let keys = ctx.keys();
        lines.push(Self::chips(
            ctx,
            &[
                (
                    keys.label(views::LEAVE, Act::LeaveConfirm)
                        .unwrap_or_default(),
                    LEAVE_NOW_WORD,
                ),
                (
                    keys.label(views::LEAVE, Act::LeaveStay).unwrap_or_default(),
                    STAY_STANDS_WORD,
                ),
            ],
        ));
        lines
    }

    /// The answers as key chips, after the box's margin.
    fn chips(ctx: &Ctx<'_>, answers: &[(String, &str)]) -> Line<'static> {
        let answers = chip_line(answers, ctx.theme);
        let mut chips = vec![Span::raw(LEAD)];
        chips.extend(answers.spans);
        Line::from(chips)
    }

    /// `  ◐ 3 working · ⚠ 1 waiting   Graphics`: the header's two counts for the open workspace,
    /// with the status glyphs, the waiting count in `warning` while someone waits, `–` for both
    /// while no count has landed, then the workspace name when there is one.
    fn counts_row(ctx: &Ctx<'_>, counts: Option<(usize, usize)>) -> Line<'static> {
        let theme = ctx.theme;
        let (working, waiting) = counts.map_or_else(
            || (UNKNOWN.to_owned(), UNKNOWN.to_owned()),
            |(working, waiting)| (working.to_string(), waiting.to_string()),
        );
        let waiting_style = match counts {
            Some((_, waiting)) if waiting > 0 => theme.warning,
            _ => theme.base,
        };
        let working_glyph = mark(ItemState::Running).glyph;
        let waiting_glyph = mark(ItemState::NeedsYou).glyph;
        let mut spans = vec![
            Span::styled(format!("{LEAD}{ROW}"), theme.base),
            Span::styled(format!("{working_glyph} {working} working"), theme.base),
            Span::styled(SEP, theme.base),
            Span::styled(format!("{waiting_glyph} {waiting} waiting"), waiting_style),
        ];
        let workspace = &ctx.top_bar.workspace;
        if !workspace.is_empty() {
            spans.push(Span::styled(
                format!("{WORKSPACE_GAP}{workspace}"),
                theme.base,
            ));
        }
        Line::from(spans)
    }

    /// One row per other workspace in the background or stopping, in the names' order; the ones
    /// without a name (before the names land, or created since) fold into one row. Offline the
    /// counts are the last ones read: when they show one, a single line says work may be running
    /// elsewhere (R1 F2).
    fn other_rows(&self, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let open = ctx.scope.workspace_id;
        let mut live: Vec<&WorkspaceRunCounts> = ctx
            .top_bar
            .workspace_runs
            .iter()
            .flatten()
            .filter(|row| row.workspace_id != open)
            .filter(|row| {
                matches!(
                    WorkspaceLifecycle::of(false, row),
                    WorkspaceLifecycle::Background { .. } | WorkspaceLifecycle::Stopping { .. }
                )
            })
            .collect();
        if ctx.top_bar.offline() {
            return if live.is_empty() {
                Vec::new()
            } else {
                vec![Line::styled(
                    format!("{LEAD}{ROW}{OTHERS_UNKNOWN}"),
                    ctx.theme.base,
                )]
            };
        }
        let mut lines = Vec::new();
        for summary in self.names.iter().flatten() {
            if let Some(at) = live
                .iter()
                .position(|row| row.workspace_id == summary.workspace_id)
            {
                lines.push(Self::other_row(ctx, live.remove(at), &summary.name));
            }
        }
        if !live.is_empty() {
            let running: u32 = live.iter().map(|row| row.running).sum();
            let n = live.len();
            let glyph = mark(ItemState::Running).glyph;
            lines.push(Line::styled(
                format!(
                    "{LEAD}{ROW}{glyph} {running} working in {n} other {}",
                    plural(n, "workspace")
                ),
                ctx.theme.base,
            ));
        }
        lines
    }

    /// `  ◐ 1 working · ⚠ 1 waiting · 1 queued   Platform  ◌ stopping…`: another workspace's
    /// running and parked runs (its "waiting" counts runs parked at a gate: the counts read
    /// carries no question rows), its queued ones when there are, its name, and the stopping
    /// mark while a stop is being applied.
    fn other_row(ctx: &Ctx<'_>, row: &WorkspaceRunCounts, name: &str) -> Line<'static> {
        let theme = ctx.theme;
        let working_glyph = mark(ItemState::Running).glyph;
        let waiting_glyph = mark(ItemState::NeedsYou).glyph;
        let waiting_style = if row.awaiting_approval > 0 {
            theme.warning
        } else {
            theme.base
        };
        let mut spans = vec![
            Span::styled(format!("{LEAD}{ROW}"), theme.base),
            Span::styled(
                format!("{working_glyph} {} working", row.running),
                theme.base,
            ),
            Span::styled(SEP, theme.base),
            Span::styled(
                format!("{waiting_glyph} {} waiting", row.awaiting_approval),
                waiting_style,
            ),
        ];
        if row.queued > 0 {
            spans.push(Span::styled(SEP, theme.base));
            spans.push(Span::styled(format!("{} queued", row.queued), theme.base));
        }
        spans.push(Span::styled(format!("{WORKSPACE_GAP}{name}"), theme.base));
        let lifecycle = WorkspaceLifecycle::of(false, row);
        if matches!(lifecycle, WorkspaceLifecycle::Stopping { .. }) {
            spans.push(Span::styled(
                format!("{STOPPING_GAP}{} stopping\u{2026}", lifecycle.glyph()),
                theme.dim,
            ));
        }
        Line::from(spans)
    }

    /// `S`: a graceful stop for every live workspace without a stop row, then settling; with
    /// none to write, settling at once.
    fn stop_first(&mut self, ctx: &Ctx<'_>) {
        let rows = ctx.top_bar.workspace_runs.as_deref().unwrap_or_default();
        let targets: Vec<StopTarget> = rows
            .iter()
            .filter(|row| row.live() > 0 && row.stop.is_none())
            .map(|row| StopTarget {
                workspace: row.workspace_id,
                decided_on: None,
            })
            .collect();
        let waited: Vec<WorkspaceId> = rows
            .iter()
            .filter(|row| {
                (row.live() > 0 && row.stop.is_none()) || (row.stop.is_some() && row.running > 0)
            })
            .map(|row| row.workspace_id)
            .collect();
        let began = Instant::now();
        if targets.is_empty() {
            let running = running_of(rows, &waited);
            self.stage = Stage::Settling {
                waited,
                changed: 0,
                failed: Vec::new(),
                began,
                running,
            };
            ctx.request(StoreRequest::WorkspaceRunCounts);
        } else {
            self.stage = Stage::Writing { waited, began };
            ctx.request(StoreRequest::StopWorkspaces {
                mode: StopMode::Graceful,
                targets,
            });
        }
    }
}

/// D9: `S` is offered when the counts are known and current (not offline, R1 F2) and some
/// workspace (the open one included) has a live run and no stop row, or a running run under a
/// stop row.
fn offers_stop(ctx: &Ctx<'_>) -> bool {
    !ctx.top_bar.offline()
        && ctx.top_bar.workspace_runs.as_ref().is_some_and(|rows| {
            rows.iter().any(|row| {
                (row.live() > 0 && row.stop.is_none()) || (row.stop.is_some() && row.running > 0)
            })
        })
}

/// The whole seconds left of `STOP_WAIT` since `began`, rounded up: `20` right after `S`.
fn seconds_left(began: Instant) -> u64 {
    let left = STOP_WAIT.saturating_sub(began.elapsed());
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// The running runs of the `waited` rows still under a stop (a row resumed meanwhile, or gone,
/// is not waited for).
fn running_of(rows: &[WorkspaceRunCounts], waited: &[WorkspaceId]) -> u32 {
    rows.iter()
        .filter(|row| waited.contains(&row.workspace_id) && row.stop.is_some())
        .map(|row| row.running)
        .sum()
}

/// `word`, with an `s` unless `n` is one.
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_owned()
    } else {
        format!("{word}s")
    }
}

impl Overlay for LeaveDialog {
    fn id(&self) -> OverlayId {
        Self::ID
    }

    fn title(&self) -> &str {
        TITLE
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(views::LEAVE)
    }

    /// The names of the other workspaces' rows (D9).
    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Workspaces]
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `Enter` and `ctrl-c` are `leave.confirm`; `Esc` and `n` are `leave.stay`; `S` is
        // `leave.stop`. The first two hold in every stage.
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::LEAVE, chord) {
            match act {
                Act::LeaveConfirm => {
                    // No `Close`: the loop stops and `App` drops, the editor with it.
                    ctx.emit(Action::Quit);
                    return Handled::Consumed;
                }
                Act::LeaveStay => {
                    // Written stops stand.
                    ctx.emit(Action::Overlay(OverlayAction::Close));
                    return Handled::Consumed;
                }
                Act::LeaveStop
                    if matches!(self.stage, Stage::Asking | Stage::Refused(_))
                        && offers_stop(ctx) =>
                {
                    self.stop_first(ctx);
                    return Handled::Consumed;
                }
                _ => continue, // `global.help`: the shell's; `S` when not offered
            }
        }
        // Everything else is swallowed by `is_modal`: `q` opens no second dialog.
        Handled::Pass
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Workspaces(list) => self.names = Some(list.clone()),
            StoreReply::StopsWritten { outcomes, .. } => {
                let Stage::Writing { waited, began } = &mut self.stage else {
                    return;
                };
                let began = *began;
                let mut waited = std::mem::take(waited);
                // R1 F3: a changed target has a stop row now (`decided_on` was none): it is
                // said and waited for like the written ones.
                let changed = outcomes
                    .iter()
                    .filter(|(_, outcome)| *outcome == StopWrite::Changed)
                    .count();
                // R1 F1: a failed one is named and not waited for.
                let failed: Vec<(WorkspaceId, String)> = outcomes
                    .iter()
                    .filter_map(|(workspace, outcome)| match outcome {
                        StopWrite::Failed(message) => Some((*workspace, message.clone())),
                        _ => None,
                    })
                    .collect();
                waited.retain(|workspace| failed.iter().all(|(gone, _)| gone != workspace));
                if waited.is_empty() && !failed.is_empty() {
                    let sentences: Vec<String> = failed
                        .iter()
                        .map(|(workspace, message)| {
                            format!("Could not stop {}: {message}", self.name_of(*workspace))
                        })
                        .collect();
                    self.stage = Stage::Refused(sentences.join("; "));
                    return;
                }
                let rows = ctx.top_bar.workspace_runs.as_deref().unwrap_or_default();
                // Before the read below lands, the rows the dialog was asked over.
                let running = rows
                    .iter()
                    .filter(|row| waited.contains(&row.workspace_id))
                    .map(|row| row.running)
                    .sum();
                self.stage = Stage::Settling {
                    waited,
                    changed,
                    failed,
                    began,
                    running,
                };
                ctx.request(StoreRequest::WorkspaceRunCounts);
            }
            StoreReply::Failed { request, message }
                if *request
                    == StoreRequest::StopWorkspaces {
                        mode: StopMode::Graceful,
                        targets: Vec::new(),
                    }
                    .name()
                    && matches!(self.stage, Stage::Writing { .. }) =>
            {
                self.stage = Stage::Refused(format!("Could not stop: {message}"));
            }
            StoreReply::WorkspaceRunCounts(rows) => {
                let Stage::Settling {
                    waited, running, ..
                } = &mut self.stage
                else {
                    return;
                };
                *running = running_of(rows, waited);
                let settled = waited.iter().all(|workspace| {
                    rows.iter()
                        .find(|row| row.workspace_id == *workspace)
                        .is_none_or(|row| row.running == 0 || row.stop.is_none())
                });
                if settled {
                    ctx.emit(Action::Quit);
                }
            }
            _ => {}
        }
    }

    /// While writing or settling: leave once `STOP_WAIT` has passed since `S` (wall time, so a
    /// refresh that did not come does not stretch it, R1 F7); otherwise, settling, read the
    /// counts again.
    fn refresh(&mut self, ctx: &mut Ctx<'_>) {
        let (Stage::Writing { began, .. } | Stage::Settling { began, .. }) = &self.stage else {
            return;
        };
        if began.elapsed() >= STOP_WAIT {
            ctx.emit(Action::Quit);
        } else if matches!(self.stage, Stage::Settling { .. }) {
            ctx.request(StoreRequest::WorkspaceRunCounts);
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let lines = self.lines(ctx);
        let widest = lines.iter().map(Line::width).max().unwrap_or(0);
        let content = Modal::new(self.title())
            .size(ModalSize::Fit {
                content_w: u16::try_from(widest).unwrap_or(u16::MAX),
                content_h: u16::try_from(lines.len()).unwrap_or(u16::MAX),
            })
            .render(frame, area, ctx.theme);
        frame.render_widget(Paragraph::new(lines), content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keys::{Keys, load_str};
    use crate::stop_report::{STOP_POLL, StopWrite};
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use htui_core::model::{
        ItemId, ProjectId, ProjectRef, StopMode, UserId, WorkspaceRunCounts, WorkspaceStop,
    };
    use htui_worker::{WaitingReason, WaitingRow, WaitingView};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// Everything a `Ctx` borrows; `keys` replaces the compiled defaults when set.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        theme: Theme,
        keys: Option<Keys>,
        open_edit: Option<String>,
        emit: Emit,
    }

    /// One waiting row; only the count matters here.
    fn row() -> WaitingRow {
        WaitingRow {
            item: ItemId::new(),
            item_key: "BL-1".to_owned(),
            project: ProjectId::new(),
            display_key: "BL-1".to_owned(),
            run: None,
            step: None,
            step_label: String::new(),
            reason: WaitingReason::Unblock,
            text: "blocked".to_owned(),
            since: chrono::DateTime::UNIX_EPOCH,
        }
    }

    impl Bench {
        /// Nothing live: the counts are in and both are zero, no edit is open.
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
                },
                projects: Vec::new(),
                top_bar: TopBarState {
                    workspace: "Graphics".to_owned(),
                    waiting: Some(WaitingView::default()),
                    ..TopBarState::default()
                },
                theme: Theme::default(),
                keys: None,
                open_edit: None,
                emit: Emit::default(),
            }
        }

        /// `working` runs and `waiting` rows in the open workspace.
        fn counting(mut self, working: usize, waiting: usize) -> Self {
            self.top_bar.waiting = Some(WaitingView {
                working,
                rows: (0..waiting).map(|_| row()).collect(),
                ..WaitingView::default()
            });
            self
        }

        /// No count has landed yet.
        fn uncounted(mut self) -> Self {
            self.top_bar.waiting = None;
            self
        }

        /// An in-pane edit is open.
        fn editing(mut self, label: &str) -> Self {
            self.open_edit = Some(label.to_owned());
            self
        }

        /// The bench with a loaded key file in force.
        fn with_keys(mut self, keys: Keys) -> Self {
            self.keys = Some(keys);
            self
        }

        fn with_theme(mut self, theme: Theme) -> Self {
            self.theme = theme;
            self
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                self.keys.as_ref().unwrap_or(Keys::compiled()),
                &self.theme,
                Origin::Overlay(LeaveDialog::ID),
                &self.emit,
            )
            .with_open_edit(self.open_edit.as_deref())
        }

        /// Sends one chord (a strict key-file spec) to the dialog.
        fn key(&self, dialog: &mut LeaveDialog, spec: &str) -> Handled {
            let chord = KeyChord::parse_strict(spec).expect("a valid spec");
            dialog.on_key(chord.to_event(), &mut self.ctx())
        }

        /// The store label reads offline: the counts held are the last ones read.
        fn offline(mut self) -> Self {
            self.top_bar.store = "offline \u{b7} 0s".to_owned();
            self
        }

        /// Every workspace's counts row (`TopBarState::workspace_runs`).
        fn runs(mut self, rows: Vec<WorkspaceRunCounts>) -> Self {
            self.top_bar.workspace_runs = Some(rows);
            self
        }

        /// Hands the dialog one reply addressed to it.
        fn reply(&self, dialog: &mut LeaveDialog, reply: StoreReply) {
            dialog.on_reply(&reply, &mut self.ctx());
        }

        /// The shell's once-a-second refresh of the top overlay.
        fn refresh(&self, dialog: &mut LeaveDialog) {
            dialog.refresh(&mut self.ctx());
        }

        /// A fresh dialog drawn over a blank 100×30 frame.
        fn terminal(&self) -> Terminal<TestBackend> {
            self.terminal_of(&LeaveDialog::new())
        }

        /// `dialog` drawn over a blank 100×30 frame.
        fn terminal_of(&self, dialog: &LeaveDialog) -> Terminal<TestBackend> {
            let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
            term.draw(|frame| dialog.render(frame, frame.area(), &self.ctx()))
                .expect("the overlay draws");
            term
        }

        /// A fresh dialog drawn over a blank 100×30 frame, one line per row.
        fn render(&self) -> String {
            self.render_of(&LeaveDialog::new())
        }

        /// `dialog` drawn over a blank 100×30 frame, one line per row.
        fn render_of(&self, dialog: &LeaveDialog) -> String {
            let term = self.terminal_of(dialog);
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

    const LIVE: &str = "Leaving closes htui. Work it is running stops with it:";
    const NOTHING: &str = "Nothing is running. Leave htui?";
    const EDIT: &str = "✎ an open edit (unsaved changes are lost)";

    #[test]
    fn it_is_a_modal_named_leave_htui_on_the_leave_stack() {
        let dialog = LeaveDialog::new();
        assert_eq!(dialog.id(), OverlayId("leave"));
        assert_eq!(dialog.title(), "Leave htui?");
        assert!(dialog.is_modal());
        let stack = dialog.key_stack().expect("the dialog names its stack");
        let keys = Keys::compiled();
        let enter = KeyChord::parse_strict("enter").expect("a valid spec");
        assert_eq!(
            keys.actions(stack, enter),
            keys.actions(views::LEAVE, enter)
        );
        assert_eq!(keys.actions(stack, enter), [Act::LeaveConfirm]);
    }

    #[test]
    fn with_work_running_the_box_names_the_counts_and_the_workspace() {
        let frame = Bench::new().counting(3, 1).render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("  ◐ 3 working · ⚠ 1 waiting   Graphics"),
            "{frame}"
        );
        assert!(!frame.contains(EDIT), "no edit is open: {frame}");
        assert!(!frame.contains(NOTHING), "{frame}");
        assert!(frame.contains("Enter  leave"), "{frame}");
        assert!(frame.contains("Esc  stay"), "{frame}");
    }

    #[test]
    fn an_open_edit_is_live_and_gets_its_row() {
        let frame = Bench::new().editing("BL-1 body").render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("  ◐ 0 working · ⚠ 0 waiting   Graphics"),
            "{frame}"
        );
        assert!(frame.contains(&format!("   {EDIT}")), "{frame}");
    }

    #[test]
    fn a_waiting_row_alone_is_live() {
        let frame = Bench::new().counting(0, 2).render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(frame.contains("◐ 0 working · ⚠ 2 waiting"), "{frame}");
    }

    #[test]
    fn with_nothing_live_the_box_asks_one_question() {
        let frame = Bench::new().render();
        assert!(frame.contains(NOTHING), "{frame}");
        assert!(!frame.contains(LIVE), "{frame}");
        assert!(!frame.contains("working"), "no counts row: {frame}");
        assert!(!frame.contains(EDIT), "{frame}");
        assert!(frame.contains("Enter  leave"), "the same chips: {frame}");
        assert!(frame.contains("Esc  stay"), "the same chips: {frame}");
    }

    #[test]
    fn before_the_first_count_the_box_assumes_work_and_shows_dashes() {
        let frame = Bench::new().uncounted().render();
        assert!(frame.contains(LIVE), "{frame}");
        assert!(
            frame.contains("◐ – working · ⚠ – waiting   Graphics"),
            "{frame}"
        );
    }

    #[test]
    fn without_a_workspace_name_the_counts_row_ends_at_the_counts() {
        let mut bench = Bench::new().counting(1, 0);
        bench.top_bar.workspace = String::new();
        let lines = LeaveDialog::new().lines(&bench.ctx());
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        assert!(
            text.iter()
                .any(|l| l.ends_with("◐ 1 working · ⚠ 0 waiting")),
            "{text:?}"
        );
    }

    #[test]
    fn enter_and_ctrl_c_leave() {
        let bench = Bench::new().counting(3, 1);
        let mut dialog = LeaveDialog::new();
        for spec in ["enter", "ctrl-c"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Consumed, "{spec}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Quit]),
                "{spec} leaves and does nothing else: {emitted:?}"
            );
        }
    }

    #[test]
    fn esc_and_n_stay() {
        let bench = Bench::new().counting(3, 1);
        let mut dialog = LeaveDialog::new();
        for spec in ["esc", "n"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Consumed, "{spec}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Overlay(OverlayAction::Close)]),
                "{spec} closes the dialog and nothing else: {emitted:?}"
            );
        }
    }

    #[test]
    fn every_other_key_is_left_to_the_modal_swallow() {
        let bench = Bench::new();
        let mut dialog = LeaveDialog::new();
        for spec in ["q", "y", "N", "space", "ctrl-y", "ctrl-q", "?"] {
            assert_eq!(bench.key(&mut dialog, spec), Handled::Pass, "{spec}");
            assert!(bench.emit.is_empty(), "{spec} emits nothing");
        }
    }

    #[test]
    fn the_chips_and_the_answer_follow_a_rebind() {
        let keys = load_str("[leave]\nconfirm = [\"f8\"]\n").expect("the file loads");
        let bench = Bench::new().with_keys(keys);

        let frame = bench.render();
        assert!(frame.contains("F8  leave"), "{frame}");
        assert!(!frame.contains("Enter  leave"), "{frame}");
        assert!(frame.contains("Esc  stay"), "{frame}");

        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "f8"), Handled::Consumed);
        assert!(matches!(bench.emit.take().as_slice(), [Action::Quit]));
        assert_eq!(
            bench.key(&mut dialog, "enter"),
            Handled::Pass,
            "enter is unbound"
        );
        assert!(bench.emit.is_empty());
    }

    #[test]
    fn the_waiting_count_is_a_warning_only_when_someone_waits() {
        let theme = Theme::default();
        let style_of = |bench: &Bench| {
            LeaveDialog::new()
                .lines(&bench.ctx())
                .iter()
                .flat_map(|line| line.spans.iter())
                .find(|span| span.content.contains("waiting"))
                .map(|span| span.style)
                .expect("a waiting span")
        };
        assert_eq!(style_of(&Bench::new().counting(0, 1)), theme.warning);
        assert_eq!(style_of(&Bench::new().counting(1, 0)), theme.base);
        assert_eq!(style_of(&Bench::new().uncounted()), theme.base);
        assert_ne!(theme.warning, theme.base);
    }

    #[test]
    fn under_monochrome_no_cell_names_a_colour() {
        for bench in [
            Bench::new(),
            Bench::new().counting(3, 1).editing("BL-1 body"),
            Bench::new().uncounted(),
        ] {
            let term = bench.with_theme(Theme::monochrome()).terminal();
            let buffer = term.backend().buffer();
            for cell in &buffer.content {
                assert_eq!(
                    (cell.fg, cell.bg),
                    (Color::Reset, Color::Reset),
                    "{:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn the_live_box() {
        insta::assert_snapshot!(Bench::new().counting(3, 1).render());
    }

    #[test]
    fn the_box_with_an_open_edit() {
        insta::assert_snapshot!(Bench::new().counting(3, 1).editing("BL-1 body").render());
    }

    #[test]
    fn the_box_with_nothing_live() {
        insta::assert_snapshot!(Bench::new().render());
    }

    #[test]
    fn the_box_before_the_first_count() {
        insta::assert_snapshot!(Bench::new().uncounted().render());
    }

    // MOD-116 M2 D9: every live workspace, and `S` stopping them gracefully first.

    /// A stop row in `mode` for `workspace`.
    fn stop_row(workspace: WorkspaceId, mode: StopMode) -> WorkspaceStop {
        WorkspaceStop {
            workspace_id: workspace,
            mode,
            requested_by: UserId::new(),
            requested_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    /// One workspace's counts row: `(queued, running, parked)` and a stop row in `stop`'s mode.
    fn counts(
        workspace: WorkspaceId,
        (queued, running, parked): (u32, u32, u32),
        stop: Option<StopMode>,
    ) -> WorkspaceRunCounts {
        WorkspaceRunCounts {
            workspace_id: workspace,
            queued,
            running,
            awaiting_approval: parked,
            stopped: 0,
            stop: stop.map(|mode| stop_row(workspace, mode)),
        }
    }

    /// The switcher row of `workspace`, named `name`.
    fn summary(workspace: WorkspaceId, name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: workspace,
            slug: name.to_lowercase(),
            name: name.to_owned(),
            projects: Vec::new(),
        }
    }

    /// D9's five workspaces: Graphics open (the bench's scope) with one run working; Platform in
    /// the background (one running, one parked, one queued); Ops stopping (one running, a stop
    /// row); Idle with no run; Held stopped (a stop row, nothing running, two queued).
    struct World {
        open: WorkspaceId,
        platform: WorkspaceId,
        ops: WorkspaceId,
        idle: WorkspaceId,
        held: WorkspaceId,
    }

    impl World {
        fn new() -> Self {
            Self {
                open: WorkspaceId::default(),
                platform: WorkspaceId::new(),
                ops: WorkspaceId::new(),
                idle: WorkspaceId::new(),
                held: WorkspaceId::new(),
            }
        }

        /// The `Workspaces` reply: the switcher's order.
        fn names(&self) -> Vec<WorkspaceSummary> {
            vec![
                summary(self.open, "Graphics"),
                summary(self.platform, "Platform"),
                summary(self.ops, "Ops"),
                summary(self.idle, "Idle"),
                summary(self.held, "Held"),
            ]
        }

        /// The counts rows, in an order that is not the names' (the rows follow the names).
        fn rows(&self) -> Vec<WorkspaceRunCounts> {
            vec![
                counts(self.held, (2, 0, 0), Some(StopMode::Graceful)),
                counts(self.ops, (0, 1, 0), Some(StopMode::Graceful)),
                counts(self.idle, (0, 0, 0), None),
                counts(self.platform, (1, 1, 1), None),
                counts(self.open, (0, 1, 0), None),
            ]
        }

        /// The bench over this world: the open workspace counts one run working.
        fn bench(&self) -> Bench {
            Bench::new().counting(1, 0).runs(self.rows())
        }
    }

    /// The frame's lines that count runs, in order.
    fn count_rows(frame: &str) -> Vec<String> {
        frame
            .lines()
            .filter(|line| line.contains(" working"))
            .map(ToOwned::to_owned)
            .collect()
    }

    /// The request in `emitted`, which holds that one action alone.
    fn only_request(emitted: &[Action]) -> &StoreRequest {
        match emitted {
            [Action::Store(request)] => request,
            other => panic!("one request, nothing else: {other:?}"),
        }
    }

    /// `S` pressed over `world`, its write answered `outcomes`, what that emitted taken.
    fn settling(
        world: &World,
        bench: &Bench,
        outcomes: Vec<(WorkspaceId, StopWrite)>,
    ) -> LeaveDialog {
        let mut dialog = LeaveDialog::new();
        bench.reply(&mut dialog, StoreReply::Workspaces(world.names()));
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let _ = bench.emit.take();
        bench.reply(
            &mut dialog,
            StoreReply::StopsWritten {
                mode: StopMode::Graceful,
                outcomes,
                lost: None,
            },
        );
        dialog
    }

    #[test]
    fn it_asks_for_the_workspace_names_when_pushed() {
        let bench = Bench::new();
        let requests = LeaveDialog::new().wants_requests(&bench.scope);
        assert!(
            matches!(requests.as_slice(), [StoreRequest::Workspaces]),
            "{requests:?}"
        );
    }

    #[test]
    fn every_live_workspace_has_a_row_the_open_one_first() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = LeaveDialog::new();
        bench.reply(&mut dialog, StoreReply::Workspaces(world.names()));
        let frame = bench.render_of(&dialog);
        let rows = count_rows(&frame);
        assert_eq!(rows.len(), 3, "{frame}");
        assert!(
            rows[0].contains("  ◐ 1 working · ⚠ 0 waiting   Graphics"),
            "{frame}"
        );
        assert!(
            rows[1].contains("  ◐ 1 working · ⚠ 1 waiting · 1 queued   Platform"),
            "{frame}"
        );
        assert!(
            rows[2].contains("  ◐ 1 working · ⚠ 0 waiting   Ops  ◌ stopping…"),
            "{frame}"
        );
        assert!(!frame.contains("Idle"), "nothing runs in Idle: {frame}");
        assert!(!frame.contains("Held"), "Held has settled: {frame}");
        assert!(frame.contains(LIVE), "the sentence is unchanged: {frame}");
        insta::assert_snapshot!("the_box_with_every_live_workspace", frame);
    }

    #[test]
    fn before_the_names_land_the_others_fold_into_one_row() {
        let world = World::new();
        let frame = world.bench().render();
        let rows = count_rows(&frame);
        assert_eq!(rows.len(), 2, "{frame}");
        assert!(rows[0].contains("Graphics"), "{frame}");
        assert!(
            rows[1].contains("  ◐ 2 working in 2 other workspaces"),
            "{frame}"
        );
        assert!(!frame.contains("Platform"), "{frame}");

        let one = Bench::new().runs(vec![
            counts(world.open, (0, 0, 0), None),
            counts(world.platform, (1, 1, 0), None),
        ]);
        let frame = one.render();
        assert!(
            frame.contains("◐ 1 working in 1 other workspace"),
            "{frame}"
        );
        assert!(
            !frame.contains("Graphics"),
            "the open workspace is idle, so no row: {frame}"
        );
        assert!(frame.contains(LIVE), "{frame}");
    }

    #[test]
    fn stop_first_is_offered_only_when_something_can_stop() {
        let world = World::new();
        const CHIP: &str = "S  stop first";
        const LINE: &str =
            "S stops them gracefully first, waits up to 20 s for their runs to stop,";
        const THEN: &str = "then leaves; each resumes later.";

        // No counts: nothing is known to stop.
        let unknown = Bench::new().counting(3, 1);
        let frame = unknown.render();
        assert!(!frame.contains(CHIP) && !frame.contains(LINE), "{frame}");
        let mut dialog = LeaveDialog::new();
        assert_eq!(unknown.key(&mut dialog, "S"), Handled::Pass);
        assert!(unknown.emit.is_empty());

        // Only stopped rows: nothing to stop.
        let settled = Bench::new().runs(vec![
            counts(world.open, (0, 0, 0), None),
            counts(world.held, (2, 0, 0), Some(StopMode::Graceful)),
        ]);
        let frame = settled.render();
        assert!(!frame.contains(CHIP) && !frame.contains(LINE), "{frame}");
        assert_eq!(settled.key(&mut dialog, "S"), Handled::Pass);
        assert!(settled.emit.is_empty());

        // A background row: offered.
        let background = Bench::new().runs(vec![
            counts(world.open, (0, 0, 0), None),
            counts(world.platform, (0, 1, 0), None),
        ]);
        let mut dialog = LeaveDialog::new();
        background.reply(
            &mut dialog,
            StoreReply::Workspaces(vec![
                summary(world.open, "Graphics"),
                summary(world.platform, "Platform"),
            ]),
        );
        let frame = background.render_of(&dialog);
        for needle in [CHIP, LINE, THEN, "Enter  leave", "Esc  stay"] {
            assert!(frame.contains(needle), "{needle}: {frame}");
        }
        insta::assert_snapshot!("the_box_offering_stop_first", frame);
    }

    #[test]
    fn s_writes_a_graceful_stop_for_every_unstopped_live_workspace() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let emitted = bench.emit.take();
        let StoreRequest::StopWorkspaces { mode, targets } = only_request(&emitted) else {
            panic!("a stop request: {emitted:?}");
        };
        assert_eq!(*mode, StopMode::Graceful);
        let mut written: Vec<WorkspaceId> = targets.iter().map(|t| t.workspace).collect();
        written.sort_by_key(ToString::to_string);
        let mut expected = vec![world.open, world.platform];
        expected.sort_by_key(ToString::to_string);
        assert_eq!(written, expected, "not Ops (stopping), Idle or Held");
        assert!(
            targets.iter().all(|target| target.decided_on.is_none()),
            "{targets:?}"
        );
        let frame = bench.render_of(&dialog);
        assert!(
            frame.contains("Stopping 3 workspaces gracefully, then leaving…"),
            "the two written and Ops, already stopping: {frame}"
        );
        assert_eq!(
            bench.key(&mut dialog, "S"),
            Handled::Pass,
            "one write at a time"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn with_nothing_to_write_s_goes_straight_to_settling() {
        let world = World::new();
        let bench = Bench::new().runs(vec![
            counts(world.open, (0, 0, 0), None),
            counts(world.ops, (0, 2, 0), Some(StopMode::Graceful)),
        ]);
        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let emitted = bench.emit.take();
        assert!(
            matches!(only_request(&emitted), StoreRequest::WorkspaceRunCounts),
            "{emitted:?}"
        );
        let frame = bench.render_of(&dialog);
        assert!(
            frame.contains("waiting for 2 running runs to stop (20 s left)"),
            "{frame}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn settling_leaves_once_every_waited_workspace_has_no_running_run() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = settling(
            &world,
            &bench,
            vec![
                (world.platform, StopWrite::Written),
                (world.open, StopWrite::Already(StopMode::Graceful)),
            ],
        );
        let emitted = bench.emit.take();
        assert!(
            matches!(only_request(&emitted), StoreRequest::WorkspaceRunCounts),
            "the dialog reads the counts itself, after the write: {emitted:?}"
        );

        // Platform and Ops still run one each; Graphics has settled.
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 0, 0), Some(StopMode::Graceful)),
                counts(world.platform, (1, 1, 1), Some(StopMode::Graceful)),
                counts(world.ops, (0, 1, 0), Some(StopMode::Graceful)),
            ]),
        );
        assert!(bench.emit.is_empty(), "two runs still run");
        let frame = bench.render_of(&dialog);
        for needle in [
            "Stopping 3 workspaces gracefully, then leaving…",
            "waiting for 2 running runs to stop (20 s left)",
            "Enter  leave now",
            "Esc  stay (the stops stand)",
        ] {
            assert!(frame.contains(needle), "{needle}: {frame}");
        }
        assert!(!frame.contains("stop first"), "{frame}");
        insta::assert_snapshot!("the_box_settling", frame);

        // Platform's run is done; Ops was resumed meanwhile, so it is not waited for.
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 0, 0), Some(StopMode::Graceful)),
                counts(world.platform, (1, 0, 1), Some(StopMode::Graceful)),
                counts(world.ops, (0, 1, 0), None),
            ]),
        );
        assert!(
            matches!(bench.emit.take().as_slice(), [Action::Quit]),
            "every waited workspace has settled"
        );
    }

    /// R1 F7: the bound is `STOP_WAIT` of wall time from `S`, read on each refresh: one a
    /// second reads the counts again until it has passed, then leaves.
    #[tokio::test(start_paused = true)]
    async fn settling_gives_up_after_the_bound() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = settling(&world, &bench, vec![(world.platform, StopWrite::Written)]);
        let _ = bench.emit.take();
        assert_eq!(STOP_WAIT.as_secs(), 20);
        for second in 1..=STOP_WAIT.as_secs() {
            tokio::time::advance(STOP_POLL).await;
            bench.refresh(&mut dialog);
            let emitted = bench.emit.take();
            if second < STOP_WAIT.as_secs() {
                assert!(
                    matches!(only_request(&emitted), StoreRequest::WorkspaceRunCounts),
                    "second {second} reads again: {emitted:?}"
                );
            } else {
                assert!(
                    matches!(emitted.as_slice(), [Action::Quit]),
                    "second {second} gives up and leaves: {emitted:?}"
                );
            }
            if second == 5 {
                let frame = bench.render_of(&dialog);
                assert!(frame.contains("(15 s left)"), "{frame}");
            }
        }
    }

    /// R1 F7: refreshes that did not come (an overlay on top is not refreshed) do not stretch the
    /// wait: the first refresh past the bound leaves.
    #[tokio::test(start_paused = true)]
    async fn a_late_refresh_past_the_bound_leaves_at_once() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = settling(&world, &bench, vec![(world.platform, StopWrite::Written)]);
        let _ = bench.emit.take();
        tokio::time::advance(STOP_WAIT + STOP_POLL).await;
        bench.refresh(&mut dialog);
        assert!(
            matches!(bench.emit.take().as_slice(), [Action::Quit]),
            "one refresh, after the bound"
        );
    }

    /// R1 F7: a write that is never answered is bound too, from `S`; until then a refresh reads
    /// nothing (there is nothing to wait for yet).
    #[tokio::test(start_paused = true)]
    async fn an_unanswered_write_leaves_after_the_bound() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let _ = bench.emit.take();
        tokio::time::advance(STOP_WAIT - STOP_POLL).await;
        bench.refresh(&mut dialog);
        assert!(bench.emit.is_empty(), "still writing, within the bound");
        tokio::time::advance(STOP_POLL).await;
        bench.refresh(&mut dialog);
        assert!(
            matches!(bench.emit.take().as_slice(), [Action::Quit]),
            "the bound has passed"
        );
    }

    /// R1 F7: the write's answer keeps the bound `S` started: what passed while writing counts.
    #[tokio::test(start_paused = true)]
    async fn the_time_spent_writing_counts_into_the_bound() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = LeaveDialog::new();
        bench.reply(&mut dialog, StoreReply::Workspaces(world.names()));
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let _ = bench.emit.take();
        tokio::time::advance(STOP_POLL * 5).await;
        bench.reply(
            &mut dialog,
            StoreReply::StopsWritten {
                mode: StopMode::Graceful,
                outcomes: vec![(world.platform, StopWrite::Written)],
                lost: None,
            },
        );
        let _ = bench.emit.take();
        let frame = bench.render_of(&dialog);
        assert!(frame.contains("(15 s left)"), "{frame}");
    }

    /// R1 F3: a target decided on no stop row that comes back `Changed` has a stop row now
    /// (someone else stopped it meanwhile): it is said, left as it is, and waited for.
    #[test]
    fn a_changed_target_is_left_as_it_is_said_and_waited_for() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = settling(
            &world,
            &bench,
            vec![
                (world.open, StopWrite::Written),
                (world.platform, StopWrite::Changed),
            ],
        );
        let _ = bench.emit.take();
        let frame = bench.render_of(&dialog);
        assert!(
            frame.contains("1 changed meanwhile, left as they are"),
            "{frame}"
        );
        assert!(
            frame.contains("Stopping 3 workspaces gracefully, then leaving\u{2026}"),
            "Graphics, Platform and Ops: {frame}"
        );
        // Platform still runs under someone else's stop: it is waited for.
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 0, 0), Some(StopMode::Graceful)),
                counts(world.platform, (0, 1, 0), Some(StopMode::Forced)),
                counts(world.ops, (0, 0, 0), Some(StopMode::Graceful)),
            ]),
        );
        assert!(bench.emit.is_empty(), "Platform still runs");
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 0, 0), Some(StopMode::Graceful)),
                counts(world.platform, (0, 0, 0), Some(StopMode::Forced)),
                counts(world.ops, (0, 0, 0), Some(StopMode::Graceful)),
            ]),
        );
        assert!(matches!(bench.emit.take().as_slice(), [Action::Quit]));
    }

    /// R1 F1: a target whose write failed is named with its refusal and not waited for; the
    /// written ones are.
    #[tokio::test(start_paused = true)]
    async fn a_failed_target_is_named_and_the_written_ones_are_waited_for() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = settling(
            &world,
            &bench,
            vec![
                (world.open, StopWrite::Written),
                (
                    world.platform,
                    StopWrite::Failed("workspace not found".to_owned()),
                ),
            ],
        );
        let emitted = bench.emit.take();
        assert!(
            matches!(only_request(&emitted), StoreRequest::WorkspaceRunCounts),
            "{emitted:?}"
        );
        let frame = bench.render_of(&dialog);
        assert!(
            frame.contains("could not stop Platform: workspace not found"),
            "{frame}"
        );
        assert!(
            frame.contains("Stopping 2 workspaces gracefully, then leaving\u{2026}"),
            "Graphics and Ops: {frame}"
        );
        insta::assert_snapshot!("the_box_settling_with_a_failed_target", frame);
        // Graphics still runs under its stop; Platform, not stopped, still runs too.
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 1, 0), Some(StopMode::Graceful)),
                counts(world.platform, (0, 1, 0), None),
                counts(world.ops, (0, 0, 0), Some(StopMode::Graceful)),
            ]),
        );
        assert!(bench.emit.is_empty(), "Graphics still runs");
        bench.reply(
            &mut dialog,
            StoreReply::WorkspaceRunCounts(vec![
                counts(world.open, (0, 0, 0), Some(StopMode::Graceful)),
                counts(world.platform, (0, 1, 0), None),
                counts(world.ops, (0, 0, 0), Some(StopMode::Graceful)),
            ]),
        );
        assert!(
            matches!(bench.emit.take().as_slice(), [Action::Quit]),
            "every written stop has settled"
        );
    }

    /// R1 F1: when every target failed and nothing else is waited for, the dialog says so and
    /// offers `S` again.
    #[test]
    fn when_every_target_failed_the_dialog_asks_again() {
        let world = World::new();
        let bench = Bench::new().runs(vec![
            counts(world.open, (0, 0, 0), None),
            counts(world.platform, (0, 1, 0), None),
        ]);
        let mut dialog = LeaveDialog::new();
        bench.reply(&mut dialog, StoreReply::Workspaces(world.names()));
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let _ = bench.emit.take();
        bench.reply(
            &mut dialog,
            StoreReply::StopsWritten {
                mode: StopMode::Graceful,
                outcomes: vec![(world.platform, StopWrite::Failed("no".to_owned()))],
                lost: None,
            },
        );
        assert!(bench.emit.is_empty(), "{:?}", bench.emit.take());
        let frame = bench.render_of(&dialog);
        assert!(frame.contains("Could not stop Platform: no"), "{frame}");
        assert!(frame.contains("S  stop first"), "{frame}");
    }

    /// R1 F2: offline the counts held are the last ones read: no other workspace's row (one
    /// folded line says work may be running there) and no `S`; back online both return.
    #[test]
    fn offline_no_other_workspace_row_and_no_stop_first() {
        let world = World::new();
        let mut dialog = LeaveDialog::new();
        let bench = world.bench().offline();
        bench.reply(&mut dialog, StoreReply::Workspaces(world.names()));
        let frame = bench.render_of(&dialog);
        assert!(!frame.contains("Platform"), "{frame}");
        assert!(!frame.contains("Ops"), "{frame}");
        assert!(
            frame.contains("work may be running in other workspaces"),
            "{frame}"
        );
        assert!(!frame.contains("stop first"), "{frame}");
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Pass);
        assert!(bench.emit.is_empty());

        let online = world.bench();
        let frame = online.render_of(&dialog);
        assert!(frame.contains("Platform"), "{frame}");
        assert!(frame.contains("S  stop first"), "{frame}");
        assert!(
            !frame.contains("work may be running in other workspaces"),
            "{frame}"
        );
    }

    /// R1 F2: offline with no other workspace live in the last counts, no folded line either.
    #[test]
    fn offline_with_nothing_live_elsewhere_there_is_no_folded_line() {
        let world = World::new();
        let bench = Bench::new()
            .runs(vec![counts(world.open, (0, 0, 0), None)])
            .offline();
        let frame = bench.render();
        assert!(frame.contains(NOTHING), "{frame}");
        assert!(!frame.contains("other workspaces"), "{frame}");
    }

    #[test]
    fn a_refused_stop_says_so_and_offers_s_again() {
        let world = World::new();
        let bench = world.bench();
        let mut dialog = LeaveDialog::new();
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let _ = bench.emit.take();
        bench.reply(
            &mut dialog,
            StoreReply::Failed {
                request: "stop_workspaces",
                message: "the database is unreachable".to_owned(),
            },
        );
        assert!(bench.emit.is_empty());
        let frame = bench.render_of(&dialog);
        assert!(
            frame.contains("Could not stop: the database is unreachable"),
            "{frame}"
        );
        assert!(frame.contains("S  stop first"), "{frame}");
        assert_eq!(bench.key(&mut dialog, "S"), Handled::Consumed);
        let emitted = bench.emit.take();
        assert!(
            matches!(only_request(&emitted), StoreRequest::StopWorkspaces { .. }),
            "{emitted:?}"
        );
    }

    #[test]
    fn enter_and_ctrl_c_leave_at_once_and_esc_stays_in_every_stage() {
        let world = World::new();
        let bench = world.bench();
        let asking = LeaveDialog::new();
        let mut writing = LeaveDialog::new();
        let _ = bench.key(&mut writing, "S");
        let mut refused = LeaveDialog::new();
        let _ = bench.key(&mut refused, "S");
        bench.reply(
            &mut refused,
            StoreReply::Failed {
                request: "stop_workspaces",
                message: "no".to_owned(),
            },
        );
        let settled = settling(&world, &bench, vec![(world.platform, StopWrite::Written)]);
        let _ = bench.emit.take();
        for (stage, mut dialog) in [
            ("asking", asking),
            ("writing", writing),
            ("refused", refused),
            ("settling", settled),
        ] {
            for spec in ["enter", "ctrl-c"] {
                assert_eq!(bench.key(&mut dialog, spec), Handled::Consumed, "{stage}");
                let emitted = bench.emit.take();
                assert!(
                    matches!(emitted.as_slice(), [Action::Quit]),
                    "{stage}: {spec} leaves at once: {emitted:?}"
                );
            }
            assert_eq!(bench.key(&mut dialog, "esc"), Handled::Consumed, "{stage}");
            let emitted = bench.emit.take();
            assert!(
                matches!(emitted.as_slice(), [Action::Overlay(OverlayAction::Close)]),
                "{stage}: esc stays: {emitted:?}"
            );
        }
    }
}
