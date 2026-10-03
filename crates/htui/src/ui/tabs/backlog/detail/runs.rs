//! The Runs sub-tab: the item's runs, newest first, each with its steps under it.
//!
//! The steps are here because they are the way into a replay (MOD-2 D39): `J` / `K` move a cursor
//! over the flattened `(run, step)` list and `Enter` emits [`Action::Replay`] for the step under
//! it. The pane does not read the step's rows itself — it holds no store handle (`R-NF-3`) — and
//! it does not know which tab will show them either; naming the step is its whole part.
//!
//! Every step is two lines at the pane's 43 columns (MOD-4 plan D169, blueprint D197): slot,
//! status, phase, usage and duration, then the gate, the prompt figure and the agent/model.
//!
//! The pane is also where a run is driven (MOD-4 plan D166-D168). Each action key reads its
//! verdict from the `RunActions` answer, which the worker computes with the engine's own guards
//! (blueprint D182): an enabled key sends its `Orch` request, a refused one puts the guard's
//! sentence on the status line and sends nothing.
//!
//! | Key | On | Sends |
//! |---|---|---|
//! | `a` / `x` | step | approve (`AnswerGate`) / reject with a typed note |
//! | `r` | step | `RetryStep` |
//! | `p` | step | promote to chat (`Action::Promote`, served for the Chat tab) |
//! | `s` | step | `SelectFanout` with the step as the winner |
//! | `A` | step | `AcceptArtifact` |
//! | `o` | step | opens the step's output document read-only, over the pane (D173) |
//! | `c` | run | `CancelRun`, after a `y` |
//! | `T` | run | a retry of the run's cleanup |
//! | `u` / `R` | item | `Unblock` / `StartRun` |
//! | `C` | item | close-out: the counts, a picked resolution, a `y`, the key typed back (D167) |
//! | `1`-`9` | step with a pending request | `AnswerPermission` with that option (MOD-42 D14) |
//! | `v` | pane | the list or the flow view of the cursor's run (MOD-28 D1, D8) |
//! | `+` / `-` / `=` | flow | zoom in, zoom out, fit the run (MOD-28 D8) |
//!
//! MOD-42 plan D14: every `Runs` reply also asks for the item's `RelayView`. A step whose session
//! parked a stage-3 permission request takes two more lines, the scrubbed summary and the strip,
//! and while the cursor is on it every digit is the pane's; anywhere else a digit passes on to the
//! global tab select. A run with a pending cancel says `cancel requested` under its grid.
//!
//! MOD-28: `v` draws the run under the cursor as a flow (`execution_graph.rs`). The flow's selected
//! node *is* the cursor, so every key above acts the same in both views (ANA-12 invariant 2).
//! `PageUp`/`PageDown` do nothing there.
//!
//! At the close-out's counts `←`/`→` (and `h`/`l`) pick the resolution among the legal ones for
//! the item's status (`Resolution::closes_from`), starting on `Resolution::default_for` (MOD-39
//! plan P13).

mod execution_graph;

use core::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use htui_agent::event::{PermissionOption, PermissionOptionKind};
use htui_core::model::{
    Document, DocumentId, ItemId, PermissionId, RelayOption, RelayOptionKind, RelayView,
    Resolution, RunId, RunMode, RunStatus, RunStepSummary, RunSummary, Status, StepId,
    StepPermission, StepStatus, ToolCallCount, UsageTotals,
};
use htui_orch::closeout::Preview;
use htui_orch::{Command, CommandOutcome, GateAnswer};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use serde_json::Value;

use self::execution_graph::ExecutionGraph;
use crate::app::{Action, Ctx, Handled};
use crate::run_worker::{Enabled, FrameKind, ItemActions, ORCH_NAMES, OrchReply, OrchRequest};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crate::ui::tabs::chat::permission::PermissionStrip;
use crate::ui::text_field::{FieldOutcome, TextField};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What an action key says while the verdicts it reads have not arrived.
pub const NOT_LOADED: &str = "the run actions have not loaded yet";

/// What a step key says with no step under the cursor.
pub const NO_STEP: &str = "no step is under the cursor";

/// What a run key says with no run under the cursor.
pub const NO_RUN: &str = "no run is under the cursor";

/// What `Enter` on an empty rejection note says: ANA-2's `reject with note` requires one.
pub const NOTE_NEEDED: &str = "a rejection needs a note";

/// What the typed close-out stage says to a key that is not the item's (D167).
pub const NOT_THE_KEY: &str = "that is not the item's key";

/// `Orch(CloseOutPreview)`'s and `Orch(Command(CloseOut))`'s names (blueprint D209), which a
/// refusal is answered under.
const CLOSE_OUT_PREVIEW: &str = "close_out_preview";
/// See [`CLOSE_OUT_PREVIEW`].
const CLOSE_OUT: &str = "close_out";

/// What `o` says when the step's document is not in this box's store (D173).
pub const NOT_ON_BOX: &str = "the document is not on this box";

/// `StoreRequest::Document`'s name, which a refused read is answered under.
const DOCUMENT: &str = "document";

/// `StoreRequest::AnswerPermission`'s name, which a refused answer is answered under (MOD-42
/// blueprint B-13).
const ANSWER_PERMISSION: &str = "answer_permission";

/// The line under a run with a pending cancel (MOD-42 plan D14). The pane's own label: the
/// runtime's sentence is longer than the pane is wide.
const CANCEL_REQUESTED_LINE: &str = "cancel requested";

/// The line under a run a command is queued behind (R-51).
const WAITING_LINE: &str = "waiting for the walk";

/// MOD-28 D5: what the flow view says under a run with no step yet.
const NO_STEPS_YET: &str = "No steps yet.";

/// What a relayed request with no summary asks for (MOD-42 blueprint B-14: the transport named no
/// call, or the scrubber refused it).
const NO_SUMMARY: &str = "a permission";

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
    /// Index into [`RunsTab::entries`]: runs in reply order, steps in each run's own order
    /// (`RunSummary.steps` is already sorted by `(position, attempt, fanout_index)`).
    ///
    /// `None` while there is no run, which is also every state before the first `Runs` reply.
    selected: Option<usize>,
    /// Every action's verdict for [`RunsTab::item`] (blueprint D182); `None` until the first
    /// `RunActions` reply for it.
    actions: Option<ItemActions>,
    /// The item this pane's `RunStream` subscription is for (blueprint D199). Left alone by an
    /// item change: the next `Runs` reply subscribes again.
    subscribed: Option<ItemId>,
    /// What the pane is doing: browsing, or waiting on typed text or an answer.
    mode: Mode,
    /// MOD-42 plan D14: the last `RelayView` for [`RunsTab::item`]; `None` until the first one.
    relay: Option<RelayView>,
    /// The request this pane last sent an `AnswerPermission` for, until a refusal frees it.
    ///
    /// A second answer to it would supersede the first in the app's staleness gate
    /// (`App::dispatch`), dropping the first's `PermissionAnswered` and leaving only the second's
    /// `already answered` refusal on the status line; and a relay view read before the answer
    /// landed may still list it as pending. Ids are never reused, so a stale entry matches nothing.
    answering: Option<PermissionId>,
    /// R-51: the runs a command is queued behind a live walk of, from a `Waiting` frame until the
    /// next frame that invalidates.
    waiting: BTreeSet<RunId>,
    /// MOD-28 D14: list or flow. Starts as list, and an item change leaves it alone.
    view: View,
    /// MOD-28 D13: the flow, behind a `RefCell` because `DetailTab::render` takes `&self` and
    /// rataflow draws through `impl Widget for &mut Flow` only.
    graph: RefCell<ExecutionGraph>,
    /// MOD-72 D7: the last `ToolCalls` reply for [`RunsTab::item`], by step. An item change
    /// clears it; only the flow view asks for it (plan D4), and a toggle back keeps it (E14).
    tool_calls: BTreeMap<StepId, Vec<ToolCallCount>>,
}

/// One place the cursor can be (blueprint D198): a step, or the header of a run with no step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A step; `run` indexes [`RunsTab::runs`].
    Step {
        /// Its run.
        run: usize,
        /// The step.
        step: StepId,
    },
    /// A run with no step yet.
    Run {
        /// The run.
        run: usize,
    },
}

/// An [`Entry`] by identity rather than position, so a re-read can find it again (D198).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKey {
    /// A step.
    Step(StepId),
    /// A run with no step.
    Run(RunId),
}

/// MOD-28 D1, D14: which view of the runs the pane draws. Per pane, not per item, never persisted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// The run list (MOD-4 D197).
    #[default]
    List,
    /// The cursor's run as a `rataflow` graph (MOD-28).
    Flow,
}

/// The pane's modes. Anything but [`Mode::Browse`] captures the keyboard (blueprint D201).
#[derive(Debug, Default)]
enum Mode {
    /// The run list, the cursor and the action keys.
    #[default]
    Browse,
    /// `x`: the note a rejection needs, typed.
    RejectNote {
        /// The step's run.
        run: RunId,
        /// The parked step.
        step: StepId,
        /// The note.
        field: TextField,
    },
    /// `c`: `y` cancels the run, `n` or `Esc` does not.
    ConfirmCancel {
        /// The run to cancel.
        run: RunId,
    },
    /// `C`: the two-stage close-out confirmation (MOD-4 plan D167).
    CloseOut(CloseOutStage),
    /// The read-only body view of a step's output document (MOD-4 plan D173).
    Artifact {
        /// The document asked for.
        id: DocumentId,
        /// Its row, once the `Document` reply is in.
        doc: Option<Box<Document>>,
        /// First visible body row, counted after wrapping.
        scroll: Scroll,
        /// The width the body last wrapped at, which is what a scroll key clamps against: only
        /// `render` knows it. `0` until the first draw, read as [`PANE`].
        width: Cell<u16>,
    },
}

/// Where a close-out confirmation is (MOD-15's slug confirmation, `settings/hierarchy.rs`).
#[derive(Debug)]
enum CloseOutStage {
    /// `CloseOutPreview` is on its way.
    Counting,
    /// The counts are shown; `←`/`→` pick the resolution in `Preview::resolution` (MOD-39 plan
    /// P13) and `y` goes on to the typed stage, which sends it.
    Warn(Preview),
    /// The item key has to be typed back.
    Typed {
        /// The counts, and the key to match.
        preview: Preview,
        /// What was typed.
        field: TextField,
    },
    /// `CloseOut` is on its way; every key waits for its answer.
    InFlight,
}

impl RunsTab {
    /// Identity of the Runs sub-tab.
    pub const ID: DetailId = DetailId("runs");

    /// A sub-tab with no runs yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The step under the cursor, or `None` when the cursor is on no step.
    ///
    /// The one question the modules above this one ask the pane, and what its `Enter` replays.
    #[must_use]
    pub fn selected_step(&self) -> Option<StepId> {
        match self.entry()? {
            Entry::Step { step, .. } => Some(step),
            Entry::Run { .. } => None,
        }
    }

    /// Every cursor entry: each run's steps, or the run itself when it has none (D198).
    fn entries(&self) -> Vec<Entry> {
        self.runs
            .iter()
            .enumerate()
            .flat_map(|(run, summary)| {
                if summary.steps.is_empty() {
                    vec![Entry::Run { run }]
                } else {
                    summary
                        .steps
                        .iter()
                        .map(|step| Entry::Step { run, step: step.id })
                        .collect()
                }
            })
            .collect()
    }

    /// The entry under the cursor.
    fn entry(&self) -> Option<Entry> {
        self.entries().get(self.selected?).copied()
    }

    /// The run of the entry under the cursor.
    fn entry_run(&self) -> Option<&RunSummary> {
        let (Entry::Step { run, .. } | Entry::Run { run }) = self.entry()?;
        self.runs.get(run)
    }

    /// The step under the cursor, with its run.
    fn entry_step(&self) -> Option<(RunId, &RunStepSummary)> {
        let Entry::Step { run, step } = self.entry()? else {
            return None;
        };
        let run = self.runs.get(run)?;
        let step = run.steps.iter().find(|summary| summary.id == step)?;
        Some((run.id, step))
    }

    /// Moves the cursor, clamped to both ends. A pane with no entry keeps `None`.
    fn move_cursor(&mut self, delta: isize) {
        let Some(last) = self.entries().len().checked_sub(1) else {
            self.selected = None;
            return;
        };
        let current = self.selected.unwrap_or(0);
        self.selected = Some(current.saturating_add_signed(delta).min(last));
    }

    /// MOD-28 D7, D12: rebuilds the flow from the cursor's run, the cursor's step selected. Only
    /// while the flow is shown: the list never pays for it, and `v` syncs on the way in.
    ///
    /// `get_mut`, not `borrow_mut`: `render_flow` holds the only runtime borrow (review N1).
    fn sync_graph(&mut self, theme: &Theme) {
        if self.view != View::Flow {
            return;
        }
        let step = self.selected_step();
        let run = match self.entry() {
            Some(Entry::Step { run, .. } | Entry::Run { run }) => self.runs.get(run),
            None => None,
        };
        self.graph
            .get_mut()
            .sync(run, step, &self.tool_calls, theme);
    }

    /// First run to draw: the scrolled-to one, except that the cursor is never scrolled off the
    /// top - the offset is derived here rather than being a second thing `J` has to keep right.
    fn first_visible(&self) -> usize {
        let cursor_run = self
            .entry_run()
            .and_then(|run| self.runs.iter().position(|summary| summary.id == run.id))
            .unwrap_or(usize::MAX);
        self.scroll.skip().min(cursor_run)
    }

    /// What an entry is across a re-read: its run's index moves, its ids do not (D198).
    fn entry_key(&self, entry: Entry) -> Option<EntryKey> {
        match entry {
            Entry::Step { step, .. } => Some(EntryKey::Step(step)),
            Entry::Run { run } => self.runs.get(run).map(|run| EntryKey::Run(run.id)),
        }
    }

    /// A `Runs` reply: the rows, the cursor kept on its entry (D198), the subscription for this
    /// item when it has none yet (D199), and the verdicts over the new rows.
    fn on_runs(&mut self, runs: &[RunSummary], ctx: &Ctx<'_>) {
        let kept = self.entry().and_then(|entry| self.entry_key(entry));
        self.runs = runs.to_vec();
        let entries = self.entries();
        self.selected = kept
            .and_then(|kept| {
                entries
                    .iter()
                    .position(|entry| self.entry_key(*entry) == Some(kept))
            })
            .or_else(|| (!entries.is_empty()).then_some(0));
        self.sync_graph(ctx.theme);
        let Some(item) = self.item else {
            return;
        };
        if self.subscribed != Some(item) {
            ctx.request(StoreRequest::RunStream { item });
            self.subscribed = Some(item);
        }
        ctx.request(StoreRequest::RunActions(item));
        ctx.request(StoreRequest::RelayView { item });
    }

    /// MOD-42 plan D14: the pending request on `step`, as the last `RelayView` had it.
    fn pending_on(&self, step: StepId) -> Option<&StepPermission> {
        self.relay
            .as_ref()?
            .permissions
            .iter()
            .find(|pending| pending.run_step_id == step)
    }

    /// The pending request on the step under the cursor, which the digits answer.
    fn pending_under_cursor(&self) -> Option<&StepPermission> {
        self.pending_on(self.selected_step()?)
    }

    /// Whether the last `RelayView` lists a pending cancel of `run`.
    fn cancel_requested(&self, run: RunId) -> bool {
        self.relay
            .as_ref()
            .is_some_and(|relay| relay.cancels.contains(&run))
    }

    /// Asks for this item's runs again.
    fn re_read(&self, ctx: &Ctx<'_>) {
        if let Some(item) = self.item {
            ctx.request(StoreRequest::Runs(item));
        }
    }

    /// The close-out's transitions on an `Orch` answer (D167): the counts move `Counting` to
    /// `Warn`, and `ClosedOut` ends an in-flight close-out.
    fn on_orch(&mut self, reply: &OrchReply) {
        match (reply, &self.mode) {
            (OrchReply::CloseOutPreview(preview), Mode::CloseOut(CloseOutStage::Counting)) => {
                self.mode = Mode::CloseOut(CloseOutStage::Warn((**preview).clone()));
            }
            (OrchReply::Done(outcome), Mode::CloseOut(CloseOutStage::InFlight))
                if matches!(**outcome, CommandOutcome::ClosedOut { .. }) =>
            {
                self.mode = Mode::Browse;
            }
            _ => {}
        }
    }

    /// A key while the pane captures (blueprint D201): a `CONTROL` chord passes, so `ctrl-c`
    /// still quits, and everything else is the mode's, answered or swallowed.
    fn modal_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::Pass;
        }
        self.mode = match core::mem::take(&mut self.mode) {
            Mode::Browse => Mode::Browse,
            Mode::RejectNote { run, step, field } => reject_key(run, step, field, key, ctx),
            Mode::ConfirmCancel { run } => match key.code {
                KeyCode::Char('y') => {
                    ctx.request(command(Command::CancelRun { run }));
                    Mode::Browse
                }
                KeyCode::Char('n') | KeyCode::Esc => Mode::Browse,
                _ => Mode::ConfirmCancel { run },
            },
            Mode::CloseOut(stage) => match self.item {
                Some(item) => close_out_key(item, stage, key, ctx),
                None => Mode::Browse,
            },
            Mode::Artifact {
                id,
                doc,
                mut scroll,
                width,
            } => {
                let code = match key.code {
                    KeyCode::Char('j') => KeyCode::Char('J'),
                    KeyCode::Char('k') => KeyCode::Char('K'),
                    other => other,
                };
                if code == KeyCode::Esc {
                    Mode::Browse
                } else {
                    let len = doc
                        .as_ref()
                        .map_or(0, |doc| artifact_rows(doc, width.get()).len());
                    let _ = scroll.on_key(KeyEvent::new(code, KeyModifiers::NONE), len);
                    Mode::Artifact {
                        id,
                        doc,
                        scroll,
                        width,
                    }
                }
            }
        };
        Handled::Consumed
    }

    /// A `Document` reply: the artifact view's body, or back to the list when the box does not
    /// hold it (D173).
    fn on_document(&mut self, doc: Option<&Document>, ctx: &Ctx<'_>) {
        let Mode::Artifact { id, doc: slot, .. } = &mut self.mode else {
            return;
        };
        match doc {
            Some(doc) if doc.id == *id => *slot = Some(Box::new(doc.clone())),
            Some(_) => {}
            None => {
                ctx.emit(Action::Error(NOT_ON_BOX.to_owned()));
                self.mode = Mode::Browse;
            }
        }
    }

    /// One action key in [`Mode::Browse`] (MOD-4 plan D168, blueprint §9.3).
    ///
    /// Each key reads its verdict first: none yet says so, a refusal puts the guard's sentence on
    /// the status line, and only an `Ok` sends. Every request goes through `ctx.request`, so it is
    /// stamped with the Backlog tab's origin and its reply comes back here.
    fn action(&mut self, key: char, ctx: &mut Ctx<'_>) -> Handled {
        let Some(item) = self.item else {
            return Handled::Pass;
        };
        let Some(actions) = &self.actions else {
            ctx.emit(Action::Error(NOT_LOADED.to_owned()));
            return Handled::Consumed;
        };
        match key {
            'u' => {
                if allowed(&actions.unblock, ctx) {
                    ctx.request(command(Command::Unblock { item }));
                }
            }
            'R' => {
                if allowed(&actions.run, ctx) {
                    ctx.request(command(Command::StartRun {
                        item,
                        mode: RunMode::Manual,
                        repo_scope: None,
                    }));
                }
            }
            'C' => {
                if allowed(&actions.close_out, ctx) {
                    ctx.request(StoreRequest::Orch(OrchRequest::CloseOutPreview { item }));
                    self.mode = Mode::CloseOut(CloseOutStage::Counting);
                }
            }
            'c' | 'T' => {
                let Some(run) = self.entry_run().map(|run| run.id) else {
                    ctx.emit(Action::Error(NO_RUN.to_owned()));
                    return Handled::Consumed;
                };
                let verdict = actions.runs.get(&run).map_or_else(
                    || Err(NOT_LOADED.to_owned()),
                    |verdicts| {
                        if key == 'c' {
                            verdicts.cancel.clone()
                        } else {
                            verdicts.cleanup.clone()
                        }
                    },
                );
                if allowed(&verdict, ctx) {
                    if key == 'c' {
                        self.mode = Mode::ConfirmCancel { run };
                    } else {
                        ctx.request(StoreRequest::Orch(OrchRequest::Cleanup { run }));
                    }
                }
            }
            'a' | 'x' | 'r' | 'p' | 'o' | 's' | 'A' => {
                let Some((run, step)) = self.entry_step() else {
                    ctx.emit(Action::Error(NO_STEP.to_owned()));
                    return Handled::Consumed;
                };
                let Some(verdicts) = actions.steps.get(&step.id) else {
                    ctx.emit(Action::Error(NOT_LOADED.to_owned()));
                    return Handled::Consumed;
                };
                let (id, position, attempt) = (step.id, step.position, step.attempt);
                match key {
                    'a' if allowed(&verdicts.approve, ctx) => {
                        ctx.request(command(Command::AnswerGate {
                            run,
                            step: id,
                            answer: GateAnswer::Approved,
                        }));
                    }
                    'x' if allowed(&verdicts.reject, ctx) => {
                        self.mode = Mode::RejectNote {
                            run,
                            step: id,
                            field: TextField::new(),
                        };
                    }
                    'r' if allowed(&verdicts.retry, ctx) => {
                        ctx.request(command(Command::RetryStep { run, step: id }));
                    }
                    'p' if allowed(&verdicts.promote, ctx) => {
                        ctx.emit(Action::Promote { run, step: id });
                    }
                    's' if allowed(&verdicts.select, ctx) => {
                        ctx.request(command(Command::SelectFanout {
                            run,
                            position,
                            attempt,
                            winner: id,
                        }));
                    }
                    'A' if allowed(&verdicts.accept, ctx) => {
                        // D185: only the worker knows whether a chat is live on the step.
                        ctx.request(command(Command::AcceptArtifact {
                            run,
                            step: id,
                            chat_live: false,
                        }));
                    }
                    'o' => match &verdicts.open {
                        Ok(document) => {
                            ctx.request(StoreRequest::Document(*document));
                            self.mode = Mode::Artifact {
                                id: *document,
                                doc: None,
                                scroll: Scroll::default(),
                                width: Cell::new(0),
                            };
                        }
                        Err(sentence) => ctx.emit(Action::Error(sentence.clone())),
                    },
                    _ => {}
                }
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }
}

/// Whether a frame of the pane's item means its rows changed: every kind but the subscription's
/// acknowledgement (D172) and a command's wait (R-51). Written out, so a new kind has to be placed
/// deliberately.
const fn invalidates(kind: &FrameKind) -> bool {
    match kind {
        FrameKind::Subscribed | FrameKind::Waiting => false,
        FrameKind::Started
        | FrameKind::SessionDone { .. }
        | FrameKind::Rested(_)
        | FrameKind::Changed
        | FrameKind::Adopted
        | FrameKind::Error(_) => true,
    }
}

/// The artifact view's rows at `width` columns (`0` reads as [`PANE`]): `<kind> v<version> ·
/// <title>`, then the body line by line, each wrapped by `cells::wrap`. The `bool` marks the
/// title's rows.
///
/// Wrapped here rather than by `Paragraph::wrap`, so the scroll clamps against the rows the reader
/// sees: a document of long paragraphs has many more of those than it has lines.
fn artifact_rows(doc: &Document, width: u16) -> Vec<(bool, String)> {
    let width = if width == 0 { PANE } else { usize::from(width) };
    let title = format!("{} v{} · {}", doc.kind, doc.version, doc.title);
    let mut rows: Vec<(bool, String)> = cells::wrap(&title, width)
        .into_iter()
        .map(|row| (true, row))
        .collect();
    for line in doc.body.lines() {
        rows.extend(cells::wrap(line, width).into_iter().map(|row| (false, row)));
    }
    rows
}

/// Draws the artifact view (D173): the document read-only over the pane, `Esc` to leave.
///
/// Records the width it wrapped at in `width`, which is what the next scroll key clamps against.
fn render_artifact(
    frame: &mut Frame<'_>,
    area: Rect,
    doc: Option<&Document>,
    scroll: Scroll,
    width: &Cell<u16>,
    theme: &Theme,
) {
    let [body, hint] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let Some(doc) = doc else {
        message(frame, body, "Opening the document\u{2026}", theme);
        return;
    };
    width.set(body.width);
    let rows = artifact_rows(doc, body.width);
    // A pane that widened since the last key has fewer rows than the offset assumed.
    let last = u16::try_from(rows.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let lines: Vec<Line<'static>> = rows
        .into_iter()
        .map(|(title, row)| Line::styled(row, if title { theme.title } else { theme.base }))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll.offset().min(last), 0)),
        body,
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "read-only · j/k scroll · Esc close",
            theme.dim,
        )),
        hint,
    );
}

/// A key in [`Mode::RejectNote`]: `Enter` with a note rejects, `Enter` without one says a note is
/// needed, `Esc` goes back to the list and everything else is typed.
fn reject_key(
    run: RunId,
    step: StepId,
    mut field: TextField,
    key: KeyEvent,
    ctx: &Ctx<'_>,
) -> Mode {
    match field.on_key(key) {
        FieldOutcome::Submit => {
            let note = field.text().unwrap_or_default().trim().to_owned();
            if note.is_empty() {
                ctx.emit(Action::Error(NOTE_NEEDED.to_owned()));
                Mode::RejectNote { run, step, field }
            } else {
                ctx.request(command(Command::AnswerGate {
                    run,
                    step,
                    answer: GateAnswer::Rejected { note },
                }));
                Mode::Browse
            }
        }
        FieldOutcome::Cancel => Mode::Browse,
        FieldOutcome::Consumed | FieldOutcome::Pass => Mode::RejectNote { run, step, field },
    }
}

/// ANA-11 §4.2 through `Resolution::closes_from`: `Resolution::ALL` in declaration order,
/// filtered (MOD-39 plan P13). The store stays the authority.
fn resolutions(status: Status) -> Vec<Resolution> {
    Resolution::ALL
        .iter()
        .copied()
        .filter(|resolution| resolution.closes_from(status))
        .collect()
}

/// The next (`forward`) or previous legal resolution after `current`, wrapping; the first legal
/// one when `current` is not legal. `current` itself when nothing closes `status`, which the
/// preview's guard has already refused.
fn cycle(status: Status, current: Resolution, forward: bool) -> Resolution {
    let legal = resolutions(status);
    let Some(at) = legal.iter().position(|&resolution| resolution == current) else {
        return legal.first().copied().unwrap_or(current);
    };
    let next = if forward {
        (at + 1) % legal.len()
    } else {
        (at + legal.len() - 1) % legal.len()
    };
    legal[next]
}

/// A key in [`Mode::CloseOut`], stage by stage (D167): the mode it leaves the pane in.
///
/// Counting and warning leave on `n`/`Esc`; the warning's `←`/`→` (and `h`/`l`) pick the
/// resolution (MOD-39 plan P13); the typed stage leaves on `Esc` only, because `n` is a letter
/// there; in flight, every key waits for the answer.
fn close_out_key(item: ItemId, stage: CloseOutStage, key: KeyEvent, ctx: &Ctx<'_>) -> Mode {
    let stage = match stage {
        CloseOutStage::Counting => match key.code {
            KeyCode::Char('n') | KeyCode::Esc => return Mode::Browse,
            _ => CloseOutStage::Counting,
        },
        CloseOutStage::Warn(preview) => match key.code {
            KeyCode::Char('y') => CloseOutStage::Typed {
                preview,
                field: TextField::new(),
            },
            KeyCode::Char('n') | KeyCode::Esc => return Mode::Browse,
            KeyCode::Right | KeyCode::Char('l') => {
                let mut preview = preview;
                preview.resolution = cycle(preview.status, preview.resolution, true);
                CloseOutStage::Warn(preview)
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let mut preview = preview;
                preview.resolution = cycle(preview.status, preview.resolution, false);
                CloseOutStage::Warn(preview)
            }
            _ => CloseOutStage::Warn(preview),
        },
        CloseOutStage::Typed { preview, mut field } => match field.on_key(key) {
            FieldOutcome::Submit if field.text() == Some(preview.key.as_str()) => {
                ctx.request(command(Command::CloseOut {
                    item,
                    resolution: preview.resolution,
                }));
                CloseOutStage::InFlight
            }
            FieldOutcome::Submit => {
                field.clear();
                ctx.emit(Action::Error(NOT_THE_KEY.to_owned()));
                CloseOutStage::Typed { preview, field }
            }
            FieldOutcome::Cancel => return Mode::Browse,
            FieldOutcome::Consumed | FieldOutcome::Pass => CloseOutStage::Typed { preview, field },
        },
        CloseOutStage::InFlight => CloseOutStage::InFlight,
    };
    Mode::CloseOut(stage)
}

/// A verdict's answer: `true` when the action may go, else the guard's sentence is on the status
/// line and nothing is sent.
fn allowed(verdict: &Enabled, ctx: &Ctx<'_>) -> bool {
    match verdict {
        Ok(()) => true,
        Err(sentence) => {
            ctx.emit(Action::Error(sentence.clone()));
            false
        }
    }
}

/// An `Orch` command, as the store request that carries it.
fn command(command: Command) -> StoreRequest {
    StoreRequest::Orch(OrchRequest::Command(command))
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

/// A blank cell of `width` columns.
fn blank(width: usize) -> String {
    " ".repeat(width)
}

/// Four cells on [`RUN_GRID`], single-spaced.
fn run_grid(values: [(&str, Style); 4]) -> Line<'static> {
    let mut spans = Vec::with_capacity(7);
    for (at, ((text, style), width)) in values.into_iter().zip(RUN_GRID).enumerate() {
        if at > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(cells::fit(text, width), style));
    }
    Line::from(spans)
}

/// The run grid's two header lines, `kind status box started` over `mode … finished`.
fn header_lines(theme: &Theme) -> [Line<'static>; 2] {
    let title = theme.title;
    [
        run_grid([
            ("kind", title),
            ("status", title),
            ("box", title),
            ("started", title),
        ]),
        run_grid([
            ("mode", title),
            ("", title),
            ("", title),
            ("finished", title),
        ]),
    ]
}

/// `run.status` in the run grid's nine columns: `awaiting_approval` is the one that does not fit,
/// and it reads `awaiting`, as a step's does (D197).
const fn run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::AwaitingApproval => "awaiting",
        other => other.as_str(),
    }
}

/// `run_step.status` in the step grid's ten columns (D197).
const fn step_status(status: StepStatus) -> &'static str {
    match status {
        StepStatus::AwaitingApproval => "awaiting",
        other => other.as_str(),
    }
}

/// A timestamp in the pane's [`STAMP`] format, or [`PENDING`].
fn stamp(at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    at.map_or_else(|| PENDING.to_owned(), |at| at.format(STAMP).to_string())
}

/// One run's lines: `kind status box started`, `mode … finished`, `cancel requested` while a
/// cancel of it is pending (MOD-42 plan D14), and the failure when there is one, fitted to the
/// pane.
fn run_lines(run: &RunSummary, cancel_requested: bool, theme: &Theme) -> Vec<Line<'static>> {
    let started = stamp(run.started_at);
    let finished = stamp(run.finished_at);
    let mut lines = vec![
        run_grid([
            (run.kind.as_str(), theme.base),
            (run_status(run.status), run_style(theme, run.status)),
            (&run.box_hostname, theme.base),
            (&started, theme.base),
        ]),
        run_grid([
            (run.mode.as_str(), theme.dim),
            ("", theme.dim),
            ("", theme.dim),
            (&finished, theme.dim),
        ]),
    ];
    if cancel_requested {
        lines.push(Line::from(Span::styled(
            cells::fit(CANCEL_REQUESTED_LINE, PANE),
            theme.accent,
        )));
    }
    if let Some(failure) = &run.failure {
        lines.push(Line::from(Span::styled(
            cells::fit(failure, PANE),
            theme.error,
        )));
    }
    lines
}

/// The slot a step sits in: `p.a`, plus `/i` in a fan-out slot and `/j` for its judge.
fn slot(step: &RunStepSummary, siblings: &[RunStepSummary]) -> String {
    let fanned = siblings.iter().any(|sibling| {
        sibling.position == step.position
            && sibling.attempt == step.attempt
            && sibling.fanout_index != 0
    });
    let at = format!("{}.{}", step.position, step.attempt);
    match (fanned, step.fanout_index) {
        (false, _) => at,
        (true, -1) => format!("{at}/j"),
        (true, index) => format!("{at}/{index}"),
    }
}

/// The gate cell: the outcome or `—`, then `*` for a promoted step and `✓` for a selected one.
fn gate(step: &RunStepSummary) -> String {
    let mut gate = step
        .gate_outcome
        .map_or(PENDING, |outcome| outcome.as_str())
        .to_owned();
    if step.promoted_at.is_some() {
        gate.push('*');
    }
    if step.selected == Some(true) {
        gate.push('\u{2713}');
    }
    gate
}

/// Line 2's tail: D106's indicator, then `agent/model`.
fn tail(step: &RunStepSummary) -> String {
    let who = format!(
        "{}/{}",
        step.agent_name.as_deref().unwrap_or(PENDING),
        step.model.as_deref().unwrap_or(PENDING)
    );
    match indicator(step) {
        Some(figure) => format!("{figure} {who}"),
        None => who,
    }
}

/// One step's two lines (D197). `siblings` are the steps of its run: a fan-out slot is known by a
/// sibling at the same `(position, attempt)` with a non-zero `fanout_index`.
fn step_lines(
    step: &RunStepSummary,
    siblings: &[RunStepSummary],
    on_cursor: bool,
    theme: &Theme,
) -> [Line<'static>; 2] {
    let label = if on_cursor { theme.accent } else { theme.dim };
    let mark = if on_cursor { CURSOR } else { " " };
    let first = Line::from(vec![
        Span::styled(mark, label),
        Span::raw(" "),
        Span::styled(cells::fit(&slot(step, siblings), SLOT_WIDTH), label),
        Span::raw(" "),
        Span::styled(
            cells::fit(step_status(step.status), STATUS_WIDTH),
            step_style(theme, step.status),
        ),
        Span::raw(" "),
        Span::styled(cells::fit(&step.phase_name, PHASE_WIDTH), label),
        Span::raw(" "),
        Span::styled(
            cells::pad_left(&usage_cell(step.usage.as_ref()), USAGE_WIDTH),
            theme.dim,
        ),
        Span::raw(" "),
        Span::styled(
            cells::pad_left(&duration_cell(step), DURATION_WIDTH),
            theme.dim,
        ),
    ]);
    let second = Line::from(vec![
        Span::raw(blank(INDENT)),
        Span::styled(cells::fit(&gate(step), GATE_WIDTH), theme.dim),
        Span::raw(" "),
        Span::styled(cells::fit(&tail(step), TAIL_WIDTH), theme.dim),
    ]);
    [first, second]
}

/// A parked step's reason on a third line (R-3): `Some` only for an awaiting step with a
/// non-empty `gate_note`, indented to the status column and fitted to the pane.
fn note_line(step: &RunStepSummary, theme: &Theme) -> Option<Line<'static>> {
    if step.status != StepStatus::AwaitingApproval {
        return None;
    }
    let note = step.gate_note.as_deref().filter(|note| !note.is_empty())?;
    Some(Line::from(vec![
        Span::raw(blank(INDENT)),
        Span::styled(cells::fit(note, PANE - INDENT), theme.accent),
    ]))
}

/// A relayed request's options as the strip takes them (MOD-42 blueprint F-19: the orphan rule
/// forbids a `From` between core's and the agent crate's types).
fn strip_options(options: &[RelayOption]) -> Vec<PermissionOption> {
    options
        .iter()
        .map(|option| PermissionOption {
            id: option.id.clone(),
            label: option.label.clone(),
            kind: match option.kind {
                RelayOptionKind::AllowOnce => PermissionOptionKind::AllowOnce,
                RelayOptionKind::AllowAlways => PermissionOptionKind::AllowAlways,
                RelayOptionKind::RejectOnce => PermissionOptionKind::RejectOnce,
                RelayOptionKind::RejectAlways => PermissionOptionKind::RejectAlways,
            },
        })
        .collect()
}

/// MOD-42 plan D14: the two lines under a step with a pending request, each exactly [`PANE`]
/// wide: `asks: <summary>` from the status column, then the answer strip.
fn permission_lines(pending: &StepPermission, theme: &Theme) -> [Line<'static>; 2] {
    let summary = pending.summary.as_deref().unwrap_or(NO_SUMMARY);
    let asks = Line::from(vec![
        Span::raw(blank(INDENT)),
        Span::styled(
            cells::fit(&format!("asks: {summary}"), PANE - INDENT),
            theme.accent,
        ),
    ]);
    let mut spans = cells::clip_spans(
        &PermissionStrip::line(&strip_options(&pending.options), theme).spans,
        PANE,
    );
    let used: usize = spans.iter().map(|span| cell_width(&span.content)).sum();
    spans.push(Span::raw(blank(PANE.saturating_sub(used))));
    [asks, Line::from(spans)]
}

/// D170's usage cell: dollars when the usage document carries a cost, else tokens, else `—`. At
/// most [`USAGE_WIDTH`] characters over the whole of `i64`.
///
/// Computed in `i128`, so rounding `i64::MAX` cannot overflow.
fn usage_cell(usage: Option<&Value>) -> String {
    let Some(totals) =
        usage.and_then(|usage| serde_json::from_value::<UsageTotals>(usage.clone()).ok())
    else {
        return PENDING.to_owned();
    };
    let cost = i128::from(totals.cost_micros.unwrap_or(0));
    if cost > 0 {
        let cents = (cost + 5_000) / 10_000;
        if cents < 10_000 {
            return format!("${}.{:02}", cents / 100, cents % 100);
        }
        let dollars = (cost + 500_000) / 1_000_000;
        return if dollars < 100_000 {
            format!("${dollars}")
        } else {
            ">$99k".to_owned()
        };
    }
    let tokens = i128::from(totals.input_tokens.unwrap_or(0))
        + i128::from(totals.output_tokens.unwrap_or(0));
    if tokens <= 0 {
        return PENDING.to_owned();
    }
    if tokens < 1_000 {
        return tokens.to_string();
    }
    let thousands = (tokens + 500) / 1_000;
    if thousands < 1_000 {
        return format!("{thousands}k");
    }
    // Tenths of a million below ten million, so `999 500` reads `1.0M` rather than `1000k`.
    let tenths = (tokens + 50_000) / 100_000;
    if tenths < 100 {
        return format!("{}.{}M", tenths / 10, tenths % 10);
    }
    let millions = (tokens + 500_000) / 1_000_000;
    if millions < 1_000 {
        format!("{millions}M")
    } else {
        ">999M".to_owned()
    }
}

/// D170's duration cell: `finished_at - started_at`, `…` while running, `—` before starting. At
/// most [`DURATION_WIDTH`] characters.
fn duration_cell(step: &RunStepSummary) -> String {
    let Some(started) = step.started_at else {
        return PENDING.to_owned();
    };
    let Some(finished) = step.finished_at else {
        return RUNNING.to_owned();
    };
    let seconds = (finished - started).num_seconds().max(0);
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m", seconds / 60);
    }
    let hours = seconds / 3_600;
    if hours < 100 {
        format!("{hours}h{:02}", (seconds % 3_600) / 60)
    } else {
        ">99h".to_owned()
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
        self.actions = None;
        self.relay = None;
        self.tool_calls.clear();
        self.answering = None;
        self.mode = Mode::Browse;
    }

    /// `J` / `K` move the cursor, `Enter` replays the step under it, the action keys act, and
    /// everything else is still the shared scroll.
    ///
    /// The cursor takes `J` / `K` off [`Scroll::on_key`] on this pane: the steps are the rows a
    /// reader moves between, and `PageDown` / `PageUp` remain the way to walk a long run list.
    /// `Enter` with no step under the cursor passes, so the Backlog tab's keymap row is what
    /// answers it — the pane never consumes a key it cannot act on.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !matches!(self.mode, Mode::Browse) {
            return self.modal_key(key, ctx);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        // MOD-42 D14: while the step under the cursor has a pending request, every digit is this
        // pane's, as the chat's parked strip takes them (`chat/mod.rs`); otherwise digits pass to
        // the global tab select (`keymap.rs`).
        if let KeyCode::Char(digit @ '1'..='9') = key.code
            && let Some(pending) = self.pending_under_cursor()
        {
            // One answer per request: a digit while it is in flight is swallowed.
            if self.answering != Some(pending.id)
                && let Some(option) =
                    PermissionStrip::pick_from(&strip_options(&pending.options), digit)
            {
                let permission = pending.id;
                ctx.request(StoreRequest::AnswerPermission {
                    permission,
                    option_id: option.id,
                });
                self.answering = Some(permission);
            }
            return Handled::Consumed;
        }
        match key.code {
            KeyCode::Char('J') => {
                self.move_cursor(1);
                self.sync_graph(ctx.theme);
            }
            KeyCode::Char('K') => {
                self.move_cursor(-1);
                self.sync_graph(ctx.theme);
            }
            KeyCode::Enter => {
                let Some(step_id) = self.selected_step() else {
                    return Handled::Pass;
                };
                ctx.emit(Action::Replay { step_id });
            }
            KeyCode::Char('v') => {
                self.view = match self.view {
                    View::List => View::Flow,
                    View::Flow => View::List,
                };
                self.sync_graph(ctx.theme);
            }
            KeyCode::Char('+') if self.view == View::Flow => self.graph.get_mut().zoom_in(),
            KeyCode::Char('-') if self.view == View::Flow => self.graph.get_mut().zoom_out(),
            KeyCode::Char('=') if self.view == View::Flow => self.graph.get_mut().fit(),
            // MOD-28 D8: the list's page keys do nothing in the flow, and leave its scroll alone.
            KeyCode::PageUp | KeyCode::PageDown if self.view == View::Flow => {}
            KeyCode::Char(
                key @ ('a' | 'x' | 'r' | 'p' | 'c' | 'o' | 's' | 'u' | 'A' | 'R' | 'C' | 'T'),
            ) => return self.action(key, ctx),
            _ => return self.scroll.on_key(key, self.runs.len()),
        }
        Handled::Consumed
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// MOD-22 review M-1: a bracketed paste into the rejection note or the typed-back key. The
    /// `y`/`n` questions are not fields: a paste there is dropped, so its `y` confirms nothing.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::RejectNote { field, .. } | Mode::CloseOut(CloseOutStage::Typed { field, .. }) => {
                field.on_paste(text);
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    /// MOD-41 plan D16: an item is selected and one of its runs is `queued`, `running` or
    /// `awaiting_approval`, as the last `Runs` reply had it.
    fn has_active_run(&self) -> bool {
        self.item.is_some() && self.runs.iter().any(|run| run.status.is_active())
    }

    /// The pane's replies (blueprint §9.5): its rows, its verdicts, the stream's invalidations,
    /// the answers to what it sent, and the document it opened.
    ///
    /// Every `Orch` answer and every refusal of an orchestrator request re-reads the runs, so a
    /// compare-and-set miss renders the rows as they are (D171); so does every frame of the item's
    /// stream but the acknowledgement (D172).
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Runs(runs) => self.on_runs(runs, ctx),
            StoreReply::RunActions(actions) if Some(actions.item) == self.item => {
                self.actions = Some((**actions).clone());
            }
            StoreReply::RunStream(frame)
                if Some(frame.item) == self.item && matches!(frame.kind, FrameKind::Waiting) =>
            {
                self.waiting.extend(frame.run);
            }
            StoreReply::RunStream(frame)
                if Some(frame.item) == self.item && invalidates(&frame.kind) =>
            {
                // Only a frame that ends the wait clears it: the walk's own `Changed` and
                // `SessionDone` land while the command is still queued behind it (R-51).
                if matches!(
                    frame.kind,
                    FrameKind::Rested(_) | FrameKind::Error(_) | FrameKind::Adopted
                ) {
                    match frame.run {
                        Some(run) => {
                            self.waiting.remove(&run);
                        }
                        None => self.waiting.clear(),
                    }
                }
                if let FrameKind::Error(sentence) = &frame.kind {
                    ctx.emit(Action::Error(sentence.clone()));
                }
                self.re_read(ctx);
            }
            StoreReply::Orch(reply) => {
                self.re_read(ctx);
                self.on_orch(reply);
            }
            StoreReply::Failed { request, .. } if ORCH_NAMES.contains(request) => {
                self.re_read(ctx);
                let ends = match &self.mode {
                    Mode::CloseOut(CloseOutStage::Counting) => *request == CLOSE_OUT_PREVIEW,
                    Mode::CloseOut(CloseOutStage::InFlight) => *request == CLOSE_OUT,
                    _ => false,
                };
                if ends {
                    // The refusal is on the status line already (`app/update.rs`).
                    self.mode = Mode::Browse;
                }
            }
            StoreReply::Failed { request, .. }
                if *request == DOCUMENT && matches!(self.mode, Mode::Artifact { .. }) =>
            {
                self.mode = Mode::Browse;
            }
            StoreReply::Document(doc) => self.on_document(doc.as_ref().as_ref(), ctx),
            // MOD-42 D14, B-13: the relay view of this item; an answer, applied or refused,
            // re-reads the runs (a refusal's sentence is on the status line already), and the
            // re-read brings a fresh view.
            StoreReply::RelayView { item, view } if Some(*item) == self.item => {
                self.relay = Some((**view).clone());
            }
            StoreReply::PermissionAnswered { .. } => self.re_read(ctx),
            StoreReply::Failed { request, .. } if *request == ANSWER_PERMISSION => {
                // Refused, so the request may be answered again.
                self.answering = None;
                self.re_read(ctx);
            }
            _ => {}
        }
    }

    /// The footer a modal asks in is drawn whatever the list holds, so the close-out of an item
    /// with no run is on screen too: `C` needs no run (`close_out_enabled`).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if let Mode::Artifact {
            doc, scroll, width, ..
        } = &self.mode
        {
            render_artifact(frame, area, doc.as_deref(), *scroll, width, ctx.theme);
            return;
        }

        let footer = self.footer(area.width, ctx.theme);
        let height = footer
            .iter()
            .map(|line| line.width().max(1).div_ceil(usize::from(area.width.max(1))))
            .sum::<usize>();
        let [list, prompt] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::try_from(height).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(footer).wrap(Wrap { trim: false }), prompt);
        if self.runs.is_empty() {
            message(frame, list, "No runs for this item.", ctx.theme);
            return;
        }
        if self.view == View::Flow {
            self.render_flow(frame, list, ctx.theme);
            return;
        }

        // The header stays on top, and the runs under it scroll so the cursor's lines are always
        // in the rows the footer leaves: D197 made every step two lines, and one long run can be
        // more than a pane.
        let [head, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(list);
        frame.render_widget(Paragraph::new(Vec::from(header_lines(ctx.theme))), head);
        let (lines, cursor_end) = self.list_lines(ctx.theme);
        let skip = cursor_end.saturating_sub(usize::from(body.height));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(skip).unwrap_or(u16::MAX), 0)),
            body,
        );
    }
}

impl RunsTab {
    /// MOD-28 D11: the flow branch of `render`. The head is what the list says about the cursor's
    /// run and step, and the rest is the canvas, or "No steps yet." under a run with none.
    fn render_flow(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let Some(run) = self.entry_run() else {
            return;
        };
        let lines = self.flow_head(run, theme);
        let [head, canvas] = Layout::vertical([
            Constraint::Length(u16::try_from(lines.len()).unwrap_or(u16::MAX)),
            Constraint::Min(0),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(lines), head);
        if run.steps.is_empty() {
            message(frame, canvas, NO_STEPS_YET, theme);
            return;
        }
        self.graph.borrow_mut().render(frame, canvas);
    }

    /// MOD-28 review L3, H1: the flow's head, the lines the list draws for `run` and the cursor's
    /// step that a node has no room for: the run header (failure, pending cancel, a queued
    /// command), the step's parked reason, and the pending request the digits answer, so a digit
    /// in the flow never answers a request the pane does not show.
    fn flow_head(&self, run: &RunSummary, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = run_lines(run, self.cancel_requested(run.id), theme);
        if self.waiting.contains(&run.id) {
            lines.push(Line::styled(cells::fit(WAITING_LINE, PANE), theme.accent));
        }
        if let Some((_, step)) = self.entry_step() {
            lines.extend(note_line(step, theme));
            if let Some(pending) = self.pending_on(step.id) {
                lines.extend(permission_lines(pending, theme));
            }
        }
        lines
    }

    /// The runs from the first visible one, each with its steps, and the line index just past the
    /// cursor's entry (0 with no cursor): what the list has to show to keep the cursor in view.
    fn list_lines(&self, theme: &Theme) -> (Vec<Line<'static>>, usize) {
        let cursor = self.entry();
        let step_cursor = self.selected_step();
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut cursor_end = 0;
        for (at, run) in self.runs.iter().enumerate().skip(self.first_visible()) {
            let mut header = run_lines(run, self.cancel_requested(run.id), theme);
            if self.waiting.contains(&run.id) {
                header.push(Line::styled(cells::fit(WAITING_LINE, PANE), theme.accent));
            }
            if cursor == Some(Entry::Run { run: at }) {
                // A run with no step is its own entry (D198); the run grid has no cursor column,
                // so its kind cell takes the accent instead.
                if let Some(kind) = header.first_mut().and_then(|line| line.spans.first_mut()) {
                    kind.style = theme.accent;
                }
                cursor_end = lines.len() + header.len();
            }
            lines.extend(header);
            for step in &run.steps {
                let on_cursor = step_cursor == Some(step.id);
                lines.extend(step_lines(step, &run.steps, on_cursor, theme));
                lines.extend(note_line(step, theme));
                // MOD-42 D14: the request's two lines belong to the step, so the cursor's end
                // counts them and the scroll keeps them in view.
                if let Some(pending) = self.pending_on(step.id) {
                    lines.extend(permission_lines(pending, theme));
                }
                if on_cursor {
                    cursor_end = lines.len();
                }
            }
        }
        (lines, cursor_end)
    }

    /// What a capturing mode asks, drawn under the run list; nothing while browsing.
    fn footer(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let hint = |text: &str| Line::styled(text.to_owned(), theme.dim);
        match &self.mode {
            Mode::Browse | Mode::Artifact { .. } => Vec::new(),
            Mode::RejectNote { field, .. } => vec![
                Line::styled("reject with a note:", theme.title),
                field.line(width, true, theme),
                hint("Enter reject · Esc cancel"),
            ],
            Mode::ConfirmCancel { .. } => vec![
                Line::styled("cancel this run?", theme.title),
                hint("y cancel it · n keep it"),
            ],
            Mode::CloseOut(CloseOutStage::Counting) => {
                vec![hint("counting what the close-out writes\u{2026}")]
            }
            Mode::CloseOut(CloseOutStage::Warn(preview)) => vec![
                Line::styled(
                    format!(
                        "close {} as {} · {} runs · {} commit rows · summary v{}",
                        preview.key,
                        preview.resolution,
                        preview.runs,
                        preview.rows,
                        preview.version
                    ),
                    theme.title,
                ),
                hint("←/→ resolution · y continue · n cancel"),
            ],
            Mode::CloseOut(CloseOutStage::Typed { preview, field }) => {
                let prompt = format!("type {} to close it: ", preview.key);
                let room =
                    width.saturating_sub(u16::try_from(cell_width(&prompt)).unwrap_or(u16::MAX));
                let mut line = Line::from(Span::styled(prompt, theme.title));
                line.spans.extend(field.line(room, true, theme).spans);
                vec![line, hint("Enter close · Esc cancel")]
            }
            Mode::CloseOut(CloseOutStage::InFlight) => vec![hint("closing\u{2026}")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::run_worker::{RunActions, RunFrame, StepActions};
    use crate::store_worker::Origin;
    use chrono::TimeDelta;
    use crossterm::event::KeyModifiers;
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{
        GateOutcome, PermissionStatus, ProjectRef, RelaySessionId, Scope, WorkspaceId,
    };
    use htui_core::store::{MemStore, ReadStore};
    use htui_orch::Rest;
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

    /// A Runs pane holding the fixture's `FEAT-1` runs: one run, four steps. What the reply made
    /// it ask for (its subscription and its verdicts) is drained.
    async fn pane(shell: &Shell) -> RunsTab {
        let runs = feat_1_runs().await;
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        let _ = shell.emit.take();
        pane
    }

    #[tokio::test]
    async fn a_runs_reply_puts_the_cursor_on_the_first_step() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        assert_eq!(pane.entries().len(), 4, "the FEAT-1 run has four steps");
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
                cell_width(&rendered) <= PHASE_WIDTH,
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
            let gate = next
                .chars()
                .skip(INDENT)
                .take(GATE_WIDTH)
                .collect::<String>();
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

    /// R-3: a parked step's reason takes a third line under its two, indented to the status
    /// column; a step without a note keeps two.
    #[tokio::test]
    async fn a_parked_step_shows_its_reason_on_a_third_line() {
        const NOTE: &str = "judge failed: orderings disagree";
        let shell = Shell::new();
        let mut runs = feat_1_runs().await;
        runs[0].steps[2].status = StepStatus::AwaitingApproval;
        runs[0].steps[2].gate_note = Some(NOTE.to_owned());
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        let _ = shell.emit.take();
        let lines = lines(&pane, &shell);
        let at = lines
            .iter()
            .position(|line| line.contains("implement"))
            .expect("the `implement` step is listed");
        let third = lines.get(at + 2).expect("the parked step has a third line");
        assert!(
            third.starts_with(&" ".repeat(INDENT)) && third.contains(NOTE),
            "the reason sits on the third line, indented: {third:?}"
        );
        assert!(
            !lines.get(at + 3).is_some_and(|line| line.contains(NOTE)),
            "once"
        );
        assert_eq!(
            lines.iter().filter(|line| !line.is_empty()).count(),
            2 + 2 + 4 * 2 + 1,
            "only the parked step with a note grew"
        );
    }

    /// R-3: the reason line is the pane's 43 columns whatever the note holds, cut with
    /// `cells::ELLIPSIS`.
    #[tokio::test]
    async fn a_reason_line_is_forty_three_columns_whatever_the_note() {
        let theme = Theme::default();
        let mut step = feat_1_runs().await[0].steps[0].clone();
        step.status = StepStatus::AwaitingApproval;
        let note = format!("first\nsecond\tthird {}", "x".repeat(300));
        step.gate_note = Some(note);
        let line = note_line(&step, &theme).expect("a parked step with a note has a line");
        assert_eq!(line.width(), PANE);
        assert!(text(&line).ends_with(cells::ELLIPSIS), "{:?}", text(&line));
        assert!(text(&line).chars().all(|c| !c.is_control()));
        step.gate_note = Some(String::new());
        assert!(note_line(&step, &theme).is_none(), "an empty note is none");
        step.gate_note = Some("a note".to_owned());
        step.status = StepStatus::Running;
        assert!(note_line(&step, &theme).is_none(), "only a parked step");
    }

    /// The text of a line, spans joined.
    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
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
                    cells::fit(&step.phase_name, PHASE_WIDTH).trim(),
                    "the phase starts at column 19"
                );
                assert_eq!(cell(31, 37), usage_cell(step.usage.as_ref()), "usage at 31");
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
                text(&first)
                    .chars()
                    .skip(2)
                    .take(SLOT_WIDTH)
                    .collect::<String>()
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
        assert_eq!(
            run_lines(&run, false, &theme).len(),
            2,
            "no failure, two lines"
        );

        let failed = RunSummary {
            status: RunStatus::Failed,
            failure: Some(format!(
                "stage 3 refused the prompt:\n{}",
                "a sentence far too long for the pane ".repeat(3)
            )),
            ..run
        };
        let lines = run_lines(&failed, false, &theme);
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
        let lines = run_lines(&run, false, &theme);
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
        let lines = run_lines(&parked, false, &theme);
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
                    cell_width(&cell) <= USAGE_WIDTH,
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
            gate_note: None,
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
        // Up to about 25 000 years: `chrono`'s own range ends not far past that.
        seconds.extend((0..26).map(|exponent| 3_i64.pow(exponent) / 7));
        for seconds in seconds {
            let cell = duration_cell(&span(seconds));
            assert!(
                cell_width(&cell) <= DURATION_WIDTH,
                "`{cell}` for {seconds}s is wider than {DURATION_WIDTH}"
            );
        }
    }

    /// MOD-60 D1: the phase is fitted in cells, so a CJK phase name leaves the first line
    /// exactly the pane wide and the usage figure in its column at cell 31.
    #[tokio::test]
    async fn a_wide_phase_name_keeps_the_usage_column() {
        let theme = Theme::default();
        let step = RunStepSummary {
            phase_name: "\u{5b9f}\u{88c5}".repeat(6),
            ..feat_1_runs().await[0].steps[0].clone()
        };
        let [first, _] = step_lines(&step, std::slice::from_ref(&step), false, &theme);
        let widths: Vec<usize> = first
            .spans
            .iter()
            .map(|span| cell_width(&span.content))
            .collect();
        assert_eq!(widths.iter().sum::<usize>(), PANE, "{first:?}");
        assert_eq!(widths[..8].iter().sum::<usize>(), 31, "{first:?}");
        assert_eq!(
            first.spans[8].content.trim_start(),
            usage_cell(step.usage.as_ref())
        );
    }

    /// MOD-60 D1: the summary and the strip are cut in cells, so CJK in either keeps both
    /// lines exactly the pane wide.
    #[test]
    fn a_wide_permission_summary_is_forty_three_cells() {
        let theme = Theme::default();
        let summary = "\u{6f22}".repeat(40);
        let options = vec![
            option(
                "allow",
                &"\u{8a31}\u{53ef}".repeat(10),
                RelayOptionKind::AllowOnce,
            ),
            option("reject", "Reject once", RelayOptionKind::RejectOnce),
        ];
        let pending = pending_on(ids::RUN_1, ids::STEP_PRD, Some(&summary), options);
        for line in permission_lines(&pending, &theme) {
            let used: usize = line
                .spans
                .iter()
                .map(|span| cell_width(&span.content))
                .sum();
            assert_eq!(used, PANE, "{line:?}");
        }
    }

    /// MOD-60 D4: an artifact's body wraps by cells, so no row of a CJK document is wider
    /// than the pane and its last character is still on a row.
    #[test]
    fn a_wide_document_wraps_by_cells() {
        let body = format!("{}\u{7d42}", "\u{6f22}".repeat(49));
        let rows = artifact_rows(&document(DocumentId::new(), &body), 20);
        for (_, row) in &rows {
            assert!(cell_width(row) <= 20, "{row:?} against 20");
        }
        let kept: String = rows
            .iter()
            .filter(|(title, _)| !title)
            .map(|(_, row)| row.as_str())
            .collect();
        assert_eq!(kept, body);
    }

    // -----------------------------------------------------------------------------------------
    // The action keys (MOD-4 plan D168, blueprint §9.3): criterion 21's reachability.
    // -----------------------------------------------------------------------------------------

    /// The sentence a hand-built refusal carries, so a test can tell which verdict a key read.
    fn refused(verdict: &str) -> String {
        format!("{verdict} is refused")
    }

    /// Hand-built verdicts over `runs` (blueprint §9.6): every one `Ok`, or every one refused.
    fn verdicts(
        item: ItemId,
        runs: &[RunSummary],
        allow: bool,
        document: DocumentId,
    ) -> ItemActions {
        let verdict = |name: &str| if allow { Ok(()) } else { Err(refused(name)) };
        ItemActions {
            item,
            key: "FEAT-1".to_owned(),
            run: verdict("run"),
            unblock: verdict("unblock"),
            close_out: verdict("close_out"),
            runs: runs
                .iter()
                .map(|run| {
                    (
                        run.id,
                        RunActions {
                            cancel: verdict("cancel"),
                            cleanup: verdict("cleanup"),
                        },
                    )
                })
                .collect(),
            steps: runs
                .iter()
                .flat_map(|run| &run.steps)
                .map(|step| {
                    (
                        step.id,
                        StepActions {
                            approve: verdict("approve"),
                            reject: verdict("reject"),
                            retry: verdict("retry"),
                            promote: verdict("promote"),
                            accept: verdict("accept"),
                            select: verdict("select"),
                            open: if allow {
                                Ok(document)
                            } else {
                                Err(refused("open"))
                            },
                        },
                    )
                })
                .collect(),
        }
    }

    /// A `FEAT-1` pane holding hand-built verdicts, its emit queue drained, and the document its
    /// `open` verdicts name.
    async fn driven(shell: &Shell, allow: bool) -> (RunsTab, DocumentId) {
        let mut pane = pane(shell).await;
        let document = DocumentId::new();
        let actions = verdicts(ids::HTUI_FEAT_1, &pane.runs, allow, document);
        pane.on_reply(&StoreReply::RunActions(Box::new(actions)), &mut shell.ctx());
        let _ = shell.emit.take();
        (pane, document)
    }

    /// The command an emitted action sends, when it sends one.
    fn sent_command(action: &Action) -> Option<&Command> {
        match action {
            Action::Store(StoreRequest::Orch(OrchRequest::Command(command))) => Some(command),
            _ => None,
        }
    }

    /// `code` on a pane whose verdicts all refuse: the verdict's sentence and nothing else.
    async fn assert_refused(code: KeyEvent, verdict: &str) {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, false).await;
        assert_eq!(pane.on_key(code, &mut shell.ctx()), Handled::Consumed);
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if *sentence == refused(verdict)),
            "{code:?} is refused with `{verdict}`'s sentence and sends nothing: {emitted:?}"
        );
        assert!(!pane.captures_input(), "a refused key opens nothing");
    }

    /// `code` on a pane whose verdicts all allow, the cursor `down` entries down: the one action it
    /// emitted.
    async fn allowed_emit(code: KeyEvent, down: usize) -> (RunsTab, Action, DocumentId) {
        let shell = Shell::new();
        let (mut pane, document) = driven(&shell, true).await;
        for _ in 0..down {
            pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        }
        assert_eq!(pane.on_key(code, &mut shell.ctx()), Handled::Consumed);
        let mut emitted = shell.emit.take();
        assert_eq!(emitted.len(), 1, "one action: {emitted:?}");
        (pane, emitted.remove(0), document)
    }

    fn shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    #[tokio::test]
    async fn a_approves_a_parked_step() {
        let (pane, action, _) = allowed_emit(key(KeyCode::Char('a')), 1).await;
        assert_eq!(
            sent_command(&action),
            Some(&Command::AnswerGate {
                run: pane.runs[0].id,
                step: ids::STEP_PLAN,
                answer: GateAnswer::Approved,
            })
        );
        assert_refused(key(KeyCode::Char('a')), "approve").await;
    }

    #[tokio::test]
    async fn r_retries() {
        let (pane, action, _) = allowed_emit(key(KeyCode::Char('r')), 0).await;
        assert_eq!(
            sent_command(&action),
            Some(&Command::RetryStep {
                run: pane.runs[0].id,
                step: ids::STEP_PRD,
            })
        );
        assert_refused(key(KeyCode::Char('r')), "retry").await;
    }

    /// `p` names the step and nothing else: the shell asks for the promotion from the Chat tab
    /// (D165), so its replies land where the chat opens.
    #[tokio::test]
    async fn p_emits_promote() {
        let (pane, action, _) = allowed_emit(key(KeyCode::Char('p')), 2).await;
        assert!(
            matches!(action, Action::Promote { run, step }
                if run == pane.runs[0].id && step == pane.runs[0].steps[2].id),
            "{action:?}"
        );
        assert_refused(key(KeyCode::Char('p')), "promote").await;
    }

    #[tokio::test]
    async fn s_selects_a_candidate() {
        let (pane, action, _) = allowed_emit(key(KeyCode::Char('s')), 3).await;
        let step = &pane.runs[0].steps[3];
        assert_eq!(
            sent_command(&action),
            Some(&Command::SelectFanout {
                run: pane.runs[0].id,
                position: step.position,
                attempt: step.attempt,
                winner: step.id,
            })
        );
        assert_refused(key(KeyCode::Char('s')), "select").await;
    }

    #[tokio::test]
    async fn u_unblocks() {
        let (_, action, _) = allowed_emit(key(KeyCode::Char('u')), 0).await;
        assert_eq!(
            sent_command(&action),
            Some(&Command::Unblock {
                item: ids::HTUI_FEAT_1
            })
        );
        assert_refused(key(KeyCode::Char('u')), "unblock").await;
    }

    /// `chat_live` is sent `false`: only the worker knows, and it overwrites it (D185).
    #[tokio::test]
    async fn shift_a_accepts_the_artifact() {
        let (pane, action, _) = allowed_emit(shift('A'), 1).await;
        assert_eq!(
            sent_command(&action),
            Some(&Command::AcceptArtifact {
                run: pane.runs[0].id,
                step: ids::STEP_PLAN,
                chat_live: false,
            })
        );
        assert_refused(shift('A'), "accept").await;
    }

    #[tokio::test]
    async fn shift_r_starts_a_run() {
        let (_, action, _) = allowed_emit(shift('R'), 0).await;
        assert_eq!(
            sent_command(&action),
            Some(&Command::StartRun {
                item: ids::HTUI_FEAT_1,
                mode: RunMode::Manual,
                repo_scope: None,
            })
        );
        assert_refused(shift('R'), "run").await;
    }

    /// R-25: the run under the cursor, whichever of its steps the cursor is on.
    #[tokio::test]
    async fn t_retries_the_cleanup() {
        let (pane, action, _) = allowed_emit(shift('T'), 2).await;
        assert!(
            matches!(&action, Action::Store(StoreRequest::Orch(OrchRequest::Cleanup { run }))
                if *run == pane.runs[0].id),
            "{action:?}"
        );
        assert_refused(shift('T'), "cleanup").await;
    }

    /// A document row for the artifact view.
    fn document(id: DocumentId, body: &str) -> Document {
        Document {
            id,
            item_id: ids::HTUI_FEAT_1,
            kind: "plan".to_owned(),
            version: 2,
            title: "Plan: TUI scaffold".to_owned(),
            body: body.to_owned(),
            produced_by_step_id: Some(ids::STEP_PLAN),
            created_by: ids::USER,
            created_at: demo_at(1, 0),
        }
    }

    /// D173: `o` reads the document and shows it over the pane, read-only: every action key is
    /// swallowed, `j` scrolls and `Esc` goes back to the list.
    #[tokio::test]
    async fn o_opens_the_artifact_read_only() {
        let shell = Shell::new();
        let (mut pane, id) = driven(&shell, true).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(
            pane.on_key(key(KeyCode::Char('o')), &mut shell.ctx()),
            Handled::Consumed
        );
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Store(StoreRequest::Document(asked))] if *asked == id),
            "`o` reads the verdict's document: {emitted:?}"
        );
        assert!(pane.captures_input(), "the view takes the keyboard");

        let body = (1..=40)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        pane.on_reply(
            &StoreReply::Document(Box::new(Some(document(id, &body)))),
            &mut shell.ctx(),
        );
        let drawn = lines(&pane, &shell);
        assert_eq!(drawn[0], "plan v2 · Plan: TUI scaffold");
        assert_eq!(drawn[1], "line 1");

        for pressed in ['a', 'x', 'q', 'R', 'C', '1'] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(pressed)), &mut shell.ctx()),
                Handled::Consumed,
                "`{pressed}` is swallowed"
            );
        }
        assert!(shell.emit.is_empty(), "read-only: nothing was sent");
        pane.on_key(key(KeyCode::Char('j')), &mut shell.ctx());
        assert_eq!(lines(&pane, &shell)[0], "line 1", "`j` scrolls the body");
        assert_eq!(
            pane.on_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &mut shell.ctx()
            ),
            Handled::Pass,
            "a `CONTROL` chord still reaches the shell"
        );

        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(!pane.captures_input(), "`Esc` closes the view");
        assert!(lines(&pane, &shell)[0].starts_with("kind"));

        pane.on_key(key(KeyCode::Char('o')), &mut shell.ctx());
        let _ = shell.emit.take();
        pane.on_reply(&StoreReply::Document(Box::new(None)), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NOT_ON_BOX),
            "{emitted:?}"
        );
        assert!(
            !pane.captures_input(),
            "a document this box lacks closes the view"
        );

        assert_refused(key(KeyCode::Char('o')), "open").await;
    }

    /// Before the verdicts arrive a key says so and sends nothing; a step key needs a step.
    #[tokio::test]
    async fn an_action_key_needs_its_verdicts_and_its_entry() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let _ = shell.emit.take();
        pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NOT_LOADED),
            "{emitted:?}"
        );

        // A run with no step yet is an entry of its own (D198): run keys act on it, step keys
        // say there is no step.
        let mut run = feat_1_runs().await.remove(0);
        run.steps.clear();
        pane.on_reply(&StoreReply::Runs(vec![run.clone()]), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::RunActions(Box::new(verdicts(
                ids::HTUI_FEAT_1,
                std::slice::from_ref(&run),
                true,
                DocumentId::new(),
            ))),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();
        assert_eq!(pane.selected_step(), None);
        pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NO_STEP),
            "{emitted:?}"
        );
        pane.on_key(shift('T'), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Store(StoreRequest::Orch(OrchRequest::Cleanup { run: id }))] if *id == run.id),
            "{emitted:?}"
        );

        // With no item there is nothing to act on, and the key is not this pane's.
        pane.on_item_change(None);
        assert_eq!(
            pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx()),
            Handled::Pass
        );
    }

    /// M4 OQ-4's human path: a slot parked for selection lists its candidates and the judge, and
    /// `s` on a candidate names it the winner.
    #[tokio::test]
    async fn a_parked_fanout_shows_its_candidates_and_s_picks_one() {
        let shell = Shell::new();
        let mut run = feat_1_runs().await.remove(0);
        run.status = RunStatus::AwaitingApproval;
        let base = run.steps[1].clone();
        run.steps = [0, 1, -1]
            .into_iter()
            .map(|fanout_index| RunStepSummary {
                id: StepId::new(),
                position: 1,
                attempt: 1,
                fanout_index,
                status: StepStatus::AwaitingApproval,
                ..base.clone()
            })
            .collect();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(vec![run.clone()]), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::RunActions(Box::new(verdicts(
                ids::HTUI_FEAT_1,
                std::slice::from_ref(&run),
                true,
                DocumentId::new(),
            ))),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();

        let drawn = lines(&pane, &shell);
        for slot in ["1.1/0", "1.1/1", "1.1/j"] {
            assert!(
                drawn.iter().any(|line| line.contains(slot)),
                "`{slot}` is listed: {drawn:#?}"
            );
        }

        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('s')), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::SelectFanout {
                run: run.id,
                position: 1,
                attempt: 1,
                winner: run.steps[1].id,
            }]
        );
    }

    // -----------------------------------------------------------------------------------------
    // The modes (MOD-4 plan D167, D168, blueprint D201).
    // -----------------------------------------------------------------------------------------

    /// Types `text` one key at a time, asserting the pane takes every one of them.
    fn type_text(pane: &mut RunsTab, shell: &Shell, text: &str) {
        for c in text.chars() {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(c)), &mut shell.ctx()),
                Handled::Consumed,
                "`{c}` is typed, not navigated"
            );
        }
    }

    /// The footer the pane draws under its list, as text.
    fn footer(pane: &RunsTab) -> Vec<String> {
        pane.footer(43, &Theme::default())
            .iter()
            .map(|line| text(line).trim_end().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn x_asks_for_a_note_then_rejects() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(
            pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(shell.emit.is_empty(), "`x` only opens the note");
        assert!(pane.captures_input());

        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NOTE_NEEDED),
            "{emitted:?}"
        );
        assert!(pane.captures_input(), "the note stays open");

        type_text(&mut pane, &shell, "  needs work: check jkl  ");
        assert_eq!(
            footer(&pane)[1],
            "  needs work: check jkl",
            "the note is on screen"
        );
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::AnswerGate {
                run: pane.runs[0].id,
                step: ids::STEP_PLAN,
                answer: GateAnswer::Rejected {
                    note: "needs work: check jkl".to_owned()
                },
            }],
            "the trimmed note: {emitted:?}"
        );
        assert!(!pane.captures_input(), "and back to the list");

        pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx());
        type_text(&mut pane, &shell, "never mind");
        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(!pane.captures_input(), "`Esc` drops the note");
        assert!(shell.emit.is_empty(), "and sends nothing");

        assert_refused(key(KeyCode::Char('x')), "reject").await;
    }

    #[tokio::test]
    async fn c_asks_then_cancels() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        let run = pane.runs[0].id;
        pane.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        assert!(shell.emit.is_empty(), "`c` only asks");
        assert_eq!(
            footer(&pane),
            ["cancel this run?", "y cancel it · n keep it"]
        );

        for swallowed in ['q', 'j', 'x', '1'] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(swallowed)), &mut shell.ctx()),
                Handled::Consumed
            );
        }
        assert!(
            shell.emit.is_empty() && pane.captures_input(),
            "anything else is swallowed"
        );

        pane.on_key(key(KeyCode::Char('n')), &mut shell.ctx());
        assert!(
            !pane.captures_input() && shell.emit.is_empty(),
            "`n` keeps the run"
        );

        pane.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::CancelRun { run }]
        );
        assert!(!pane.captures_input());

        assert_refused(key(KeyCode::Char('c')), "cancel").await;
    }

    /// The first confirmation's figures for `FEAT-1`.
    fn preview() -> Preview {
        Preview {
            key: "FEAT-1".to_owned(),
            title: "TUI scaffold".to_owned(),
            status: Status::Done,
            resolution: Resolution::Done,
            runs: 1,
            rows: 3,
            version: 2,
        }
    }

    /// `C` on an allowed pane, then [`preview`]'s answer: the pane at the warning.
    async fn warned(shell: &Shell) -> RunsTab {
        warned_with(shell, preview()).await
    }

    /// `C` on an allowed pane, then `preview` as the answer: the pane at the warning.
    async fn warned_with(shell: &Shell, preview: Preview) -> RunsTab {
        let (mut pane, _) = driven(shell, true).await;
        pane.on_key(shift('C'), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(),
                [Action::Store(StoreRequest::Orch(OrchRequest::CloseOutPreview { item }))]
                    if *item == ids::HTUI_FEAT_1),
            "`C` asks for the counts first: {emitted:?}"
        );
        assert!(pane.captures_input(), "and waits for them");
        assert_eq!(
            footer(&pane),
            ["counting what the close-out writes\u{2026}"]
        );
        pane.on_reply(
            &StoreReply::Orch(OrchReply::CloseOutPreview(Box::new(preview))),
            &mut shell.ctx(),
        );
        assert!(
            is_one_re_read(&requests(shell.emit.take()), ids::HTUI_FEAT_1),
            "an `Orch` answer re-reads the runs (D171)"
        );
        pane
    }

    #[tokio::test]
    async fn shift_c_counts_then_asks_for_the_key() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        assert_eq!(
            footer(&pane),
            [
                "close FEAT-1 as done · 1 runs · 3 commit rows · summary v2",
                "←/→ resolution · y continue · n cancel"
            ]
        );
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        assert_eq!(
            footer(&pane),
            ["type FEAT-1 to close it:", "Enter close · Esc cancel"]
        );
        assert!(
            shell.emit.is_empty(),
            "nothing is written before the key is typed"
        );

        // `Esc`/`n` leave at the first two stages.
        let mut pane = warned(&shell).await;
        pane.on_key(key(KeyCode::Char('n')), &mut shell.ctx());
        assert!(!pane.captures_input());
        pane.on_key(shift('C'), &mut shell.ctx());
        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(!pane.captures_input(), "`Esc` while counting leaves too");
        let _ = shell.emit.take();

        // A refused preview (a run went live meanwhile) closes the counting stage.
        pane.on_key(shift('C'), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::Failed {
                request: CLOSE_OUT_PREVIEW,
                message: "run is live".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(!pane.captures_input());

        assert_refused(shift('C'), "close_out").await;
    }

    /// MOD-38 PRD D2: the first confirmation names the resolution the item closes as, and the
    /// typed key sends that resolution: a `failed` item closes as `withdrawn` (plan D6).
    #[tokio::test]
    async fn the_close_out_warn_names_the_resolution() {
        let shell = Shell::new();
        let failed = Preview {
            status: Status::Failed,
            resolution: Resolution::Withdrawn,
            ..preview()
        };
        let mut pane = warned_with(&shell, failed).await;
        assert_eq!(
            footer(&pane)[0],
            "close FEAT-1 as withdrawn · 1 runs · 3 commit rows · summary v2"
        );

        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        type_text(&mut pane, &shell, "FEAT-1");
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::CloseOut {
                item: ids::HTUI_FEAT_1,
                resolution: Resolution::Withdrawn,
            }]
        );
    }

    /// The resolution the warning stage has picked.
    fn picked(pane: &RunsTab) -> Resolution {
        match &pane.mode {
            Mode::CloseOut(CloseOutStage::Warn(preview)) => preview.resolution,
            other => panic!("not at the warning: {other:?}"),
        }
    }

    /// `presses` of `code` at the warning, each consumed: the resolutions picked, in order.
    fn walk(pane: &mut RunsTab, shell: &Shell, code: KeyCode, presses: usize) -> Vec<Resolution> {
        (0..presses)
            .map(|_| {
                assert_eq!(pane.on_key(key(code), &mut shell.ctx()), Handled::Consumed);
                picked(pane)
            })
            .collect()
    }

    /// MOD-39 plan P13: `→` on a `done` item walks every resolution in declaration order and
    /// wraps; nothing is sent while picking.
    #[tokio::test]
    async fn right_on_a_done_preview_walks_all_six_resolutions() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        assert_eq!(picked(&pane), Resolution::Done);
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Right, 6),
            [
                Resolution::Concluded,
                Resolution::Rejected,
                Resolution::Withdrawn,
                Resolution::Superseded,
                Resolution::Duplicate,
                Resolution::Done,
            ]
        );
        assert!(shell.emit.is_empty(), "picking sends nothing");
        assert!(pane.captures_input(), "and stays at the warning");
    }

    /// MOD-39 plan P13: `←` and `h` walk back, wrapping from the first resolution to the last.
    #[tokio::test]
    async fn left_walks_back_and_wraps() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Left, 1),
            [Resolution::Duplicate]
        );
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Char('h'), 1),
            [Resolution::Superseded]
        );
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Char('l'), 2),
            [Resolution::Duplicate, Resolution::Done]
        );
    }

    /// ANA-11 §4.2: a `blocked` item cannot close as `done` or `concluded`, so the picker offers
    /// the four others, starting on `withdrawn`.
    #[tokio::test]
    async fn a_blocked_preview_offers_the_four_non_success_resolutions() {
        let shell = Shell::new();
        let blocked = Preview {
            status: Status::Blocked,
            resolution: Resolution::Withdrawn,
            ..preview()
        };
        let mut pane = warned_with(&shell, blocked).await;
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Char('l'), 4),
            [
                Resolution::Superseded,
                Resolution::Duplicate,
                Resolution::Rejected,
                Resolution::Withdrawn,
            ]
        );
        assert_eq!(
            resolutions(Status::Blocked),
            [
                Resolution::Rejected,
                Resolution::Withdrawn,
                Resolution::Superseded,
                Resolution::Duplicate,
            ]
        );
    }

    /// MOD-39 plan P13: an `open` item is closable now; its picker starts on `withdrawn` and
    /// offers the same four as `blocked`. A resolution that is not legal steps to the first legal
    /// one.
    #[tokio::test]
    async fn an_open_preview_starts_on_withdrawn() {
        assert_eq!(
            Resolution::default_for(Status::Open),
            Some(Resolution::Withdrawn)
        );
        assert_eq!(resolutions(Status::Open), resolutions(Status::Blocked));
        assert!(resolutions(Status::Queued).is_empty());
        assert_eq!(
            cycle(Status::Open, Resolution::Done, true),
            Resolution::Rejected
        );
        assert_eq!(
            cycle(Status::Open, Resolution::Done, false),
            Resolution::Rejected
        );

        let shell = Shell::new();
        let open = Preview {
            status: Status::Open,
            resolution: Resolution::Withdrawn,
            ..preview()
        };
        let mut pane = warned_with(&shell, open).await;
        assert_eq!(picked(&pane), Resolution::Withdrawn);
        assert_eq!(
            walk(&mut pane, &shell, KeyCode::Left, 4),
            [
                Resolution::Rejected,
                Resolution::Duplicate,
                Resolution::Superseded,
                Resolution::Withdrawn,
            ]
        );
    }

    /// MOD-39 plan P13: the typed key sends the resolution picked, not the one the preview
    /// started on.
    #[tokio::test]
    async fn the_chosen_resolution_is_what_close_out_sends() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        walk(&mut pane, &shell, KeyCode::Char('l'), 2);
        assert_eq!(picked(&pane), Resolution::Rejected);

        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        type_text(&mut pane, &shell, "FEAT-1");
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::CloseOut {
                item: ids::HTUI_FEAT_1,
                resolution: Resolution::Rejected,
            }]
        );
    }

    /// MOD-39 plan P13: the warning names the picker in its hint and the picked resolution in its
    /// title line.
    #[tokio::test]
    async fn the_warn_footer_names_the_picker() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        assert!(
            footer(&pane)[1].contains("←/→ resolution"),
            "{:?}",
            footer(&pane)
        );
        walk(&mut pane, &shell, KeyCode::Right, 2);
        assert_eq!(
            footer(&pane),
            [
                "close FEAT-1 as rejected · 1 runs · 3 commit rows · summary v2",
                "←/→ resolution · y continue · n cancel"
            ]
        );
    }

    /// D167: the typed stage wants the item's key exactly; a wrong key clears the field, `n` is a
    /// letter there, and once sent every key waits for the answer.
    #[tokio::test]
    async fn the_close_out_key_must_match_and_other_keys_are_swallowed() {
        let shell = Shell::new();
        let mut pane = warned(&shell).await;
        for swallowed in ['x', 'q', 'j', 'G', '3'] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(swallowed)), &mut shell.ctx()),
                Handled::Consumed
            );
        }
        assert!(
            footer(&pane)[0].starts_with("close FEAT-1"),
            "still warning"
        );
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());

        type_text(&mut pane, &shell, "FEAT-2n");
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NOT_THE_KEY),
            "{emitted:?}"
        );
        assert_eq!(
            footer(&pane)[0],
            "type FEAT-1 to close it:",
            "the field is cleared"
        );

        type_text(&mut pane, &shell, "FEAT-1");
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert_eq!(
            emitted.iter().filter_map(sent_command).collect::<Vec<_>>(),
            [&Command::CloseOut {
                item: ids::HTUI_FEAT_1,
                resolution: Resolution::Done,
            }]
        );
        for swallowed in [KeyCode::Esc, KeyCode::Char('y'), KeyCode::Enter] {
            assert_eq!(
                pane.on_key(key(swallowed), &mut shell.ctx()),
                Handled::Consumed
            );
        }
        assert!(shell.emit.is_empty() && pane.captures_input(), "in flight");

        pane.on_reply(
            &StoreReply::Orch(OrchReply::Done(Box::new(CommandOutcome::ClosedOut {
                item: ids::HTUI_FEAT_1,
                summary: DocumentId::new(),
                version: 2,
            }))),
            &mut shell.ctx(),
        );
        assert!(!pane.captures_input(), "the answer ends the close-out");

        // A refused close-out ends it too; the sentence is the status line's.
        let mut pane = warned(&shell).await;
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        type_text(&mut pane, &shell, "FEAT-1");
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::Failed {
                request: CLOSE_OUT,
                message: "refused".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(!pane.captures_input());

        // `Esc` leaves the typed stage.
        let mut pane = warned(&shell).await;
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert!(!pane.captures_input());
    }

    /// D201: the pane captures exactly while it is not browsing, and a `CONTROL` chord always
    /// passes so `ctrl-c` quits from any mode.
    #[tokio::test]
    async fn captures_input_follows_the_mode() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!pane.captures_input(), "browsing");
        assert_eq!(pane.on_key(ctrl_c, &mut shell.ctx()), Handled::Pass);

        for (opener, name) in [
            (key(KeyCode::Char('x')), "a note"),
            (key(KeyCode::Char('c')), "a y/n"),
            (shift('C'), "a close-out"),
            (key(KeyCode::Char('o')), "a document"),
        ] {
            pane.on_key(opener, &mut shell.ctx());
            assert!(pane.captures_input(), "{name} captures");
            assert_eq!(
                pane.on_key(ctrl_c, &mut shell.ctx()),
                Handled::Pass,
                "{name} lets `ctrl-c` through"
            );
            pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
            assert!(!pane.captures_input(), "{name} ends on `Esc`");
        }

        pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx());
        pane.on_item_change(Some(ids::HTUI_ANA_2));
        assert!(!pane.captures_input(), "a new item drops whatever was open");
    }

    // -----------------------------------------------------------------------------------------
    // The subscription and the re-reads (MOD-4 plan D171, D172, blueprint D198, D199).
    // -----------------------------------------------------------------------------------------

    /// The store requests an emit queue held, in order.
    fn requests(emitted: Vec<Action>) -> Vec<StoreRequest> {
        emitted
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// Whether `requests` is exactly one `Runs` read of `item`.
    fn is_one_re_read(requests: &[StoreRequest], item: ItemId) -> bool {
        matches!(requests, [StoreRequest::Runs(asked)] if *asked == item)
    }

    /// D199: the pane subscribes for its own item on the first `Runs` reply, and asks for the
    /// verdicts on every one.
    #[tokio::test]
    async fn the_first_runs_reply_subscribes_once_and_asks_for_the_actions() {
        let shell = Shell::new();
        let runs = feat_1_runs().await;
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(runs.clone()), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [
                StoreRequest::RunStream { item: streamed },
                StoreRequest::RunActions(asked),
                StoreRequest::RelayView { item: relayed },
            ] if *streamed == ids::HTUI_FEAT_1
                && *asked == ids::HTUI_FEAT_1
                && *relayed == ids::HTUI_FEAT_1),
            "MOD-42 D14: the relay view rides along with the verdicts: {sent:?}"
        );

        pane.on_reply(&StoreReply::Runs(runs.clone()), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [
                StoreRequest::RunActions(asked),
                StoreRequest::RelayView { item: relayed },
            ] if *asked == ids::HTUI_FEAT_1 && *relayed == ids::HTUI_FEAT_1),
            "one subscription per item: {sent:?}"
        );

        // An item with no run is subscribed for too: the reply is empty, the item is the pane's.
        pane.on_item_change(Some(ids::HTUI_ANA_2));
        pane.on_reply(&StoreReply::Runs(Vec::new()), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [
                StoreRequest::RunStream { item: streamed },
                StoreRequest::RunActions(asked),
                StoreRequest::RelayView { item: relayed },
            ] if *streamed == ids::HTUI_ANA_2
                && *asked == ids::HTUI_ANA_2
                && *relayed == ids::HTUI_ANA_2),
            "{sent:?}"
        );

        // Verdicts for another item are not this pane's.
        let other = verdicts(ids::HTUI_FEAT_1, &runs, true, DocumentId::new());
        pane.on_reply(&StoreReply::RunActions(Box::new(other)), &mut shell.ctx());
        pane.on_key(shift('R'), &mut shell.ctx());
        let emitted = shell.emit.take();
        assert!(
            matches!(emitted.as_slice(), [Action::Error(sentence)] if sentence == NOT_LOADED),
            "{emitted:?}"
        );
    }

    /// D171: an `Orch` answer, and a refusal of any orchestrator request, re-read the runs so the
    /// pane shows the rows as they are after a compare-and-set miss.
    #[tokio::test]
    async fn a_refusal_re_reads_the_runs() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_reply(
            &StoreReply::Failed {
                request: "retry_step",
                message: "step is `superseded`".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(is_one_re_read(
            &requests(shell.emit.take()),
            ids::HTUI_FEAT_1
        ));

        for name in ORCH_NAMES {
            pane.on_reply(
                &StoreReply::Failed {
                    request: name,
                    message: String::new(),
                },
                &mut shell.ctx(),
            );
            assert!(
                is_one_re_read(&requests(shell.emit.take()), ids::HTUI_FEAT_1),
                "`{name}`"
            );
        }

        pane.on_reply(
            &StoreReply::Orch(OrchReply::CleanedUp {
                run: pane.runs[0].id,
            }),
            &mut shell.ctx(),
        );
        assert!(is_one_re_read(
            &requests(shell.emit.take()),
            ids::HTUI_FEAT_1
        ));

        pane.on_reply(
            &StoreReply::Failed {
                request: "notes",
                message: "not an orchestrator request".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(
            shell.emit.is_empty(),
            "another read's refusal is not the pane's"
        );
    }

    /// A frame of `item` with this kind.
    fn frame(item: ItemId, kind: FrameKind) -> StoreReply {
        StoreReply::RunStream(RunFrame {
            item,
            run: None,
            kind,
        })
    }

    /// D172: every frame but the acknowledgement is an invalidation, all six kinds of it.
    #[tokio::test]
    async fn a_run_stream_frame_re_reads_the_runs() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        let kinds = [
            FrameKind::Started,
            FrameKind::SessionDone {
                step: ids::STEP_PLAN,
            },
            FrameKind::Rested(Rest {
                run: RunStatus::AwaitingApproval,
                position: Some(1),
                failure: None,
            }),
            FrameKind::Changed,
            FrameKind::Adopted,
            FrameKind::Error("the walk failed".to_owned()),
        ];
        for kind in kinds {
            let name = format!("{kind:?}");
            pane.on_reply(&frame(ids::HTUI_FEAT_1, kind), &mut shell.ctx());
            assert!(
                is_one_re_read(&requests(shell.emit.take()), ids::HTUI_FEAT_1),
                "{name} re-reads"
            );
        }
        pane.on_reply(
            &frame(ids::HTUI_ANA_2, FrameKind::Changed),
            &mut shell.ctx(),
        );
        assert!(
            shell.emit.is_empty(),
            "another item's frame is not this pane's"
        );
    }

    /// R-41: the sentence of a dropped failure rides the stream's `Error` frame, which keeps the
    /// subscription's `seq` and so survives the staleness check an `Orch` `Failed` does not.
    #[tokio::test]
    async fn a_run_stream_error_frame_reaches_the_status_line() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_reply(
            &frame(
                ids::HTUI_FEAT_1,
                FrameKind::Error("the walk failed".to_owned()),
            ),
            &mut shell.ctx(),
        );
        let emitted = shell.emit.take();
        assert!(
            emitted
                .iter()
                .any(|action| matches!(action, Action::Error(s) if s == "the walk failed")),
            "{emitted:?}"
        );
        assert!(is_one_re_read(&requests(emitted), ids::HTUI_FEAT_1));
        pane.on_reply(
            &frame(ids::HTUI_ANA_2, FrameKind::Error("not mine".to_owned())),
            &mut shell.ctx(),
        );
        assert!(
            shell.emit.is_empty(),
            "another item's error is not reported"
        );
    }

    #[tokio::test]
    async fn the_subscription_ack_does_not_re_read() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_reply(
            &StoreReply::RunStream(RunFrame::subscribed(ids::HTUI_FEAT_1)),
            &mut shell.ctx(),
        );
        assert!(shell.emit.is_empty(), "the acknowledgement changes nothing");
    }

    /// D198: a re-read of the same item keeps the cursor on the step it was on, even when a new
    /// run lands above it; a step that is gone gives way to the first entry.
    #[tokio::test]
    async fn a_re_read_keeps_the_cursor_on_its_step() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        let implement = pane.runs[0].steps[2].id;
        assert_eq!(pane.selected_step(), Some(implement));

        let mut newer = feat_1_runs().await.remove(0);
        newer.id = RunId::new();
        newer.steps.clear();
        let mut runs = vec![newer];
        runs.extend(feat_1_runs().await);
        pane.on_reply(&StoreReply::Runs(runs.clone()), &mut shell.ctx());
        assert_eq!(
            pane.selected_step(),
            Some(implement),
            "the cursor follows its step, not its index"
        );

        // The stepless run is an entry of its own, and it is kept too.
        pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        assert_eq!(pane.entry_run().map(|run| run.id), Some(runs[0].id));
        pane.on_reply(&StoreReply::Runs(runs.clone()), &mut shell.ctx());
        assert_eq!(pane.entry_run().map(|run| run.id), Some(runs[0].id));
        assert_eq!(pane.selected_step(), None);

        runs.remove(0);
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        assert_eq!(
            pane.selected_step(),
            Some(ids::STEP_PRD),
            "an entry that is gone gives way to the first one"
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

    // -----------------------------------------------------------------------------------------
    // MOD-28: the flow view (plan D5, D7, D8, D11, D14).
    // -----------------------------------------------------------------------------------------

    /// `FEAT-1`'s run with a fresh run id and fresh step ids, a second run to cross into.
    async fn copied_run() -> RunSummary {
        let mut run = feat_1_runs().await.remove(0);
        run.id = RunId::new();
        for step in &mut run.steps {
            step.id = StepId::new();
        }
        run
    }

    /// `FEAT-1`'s run with a fresh id and no step, as a just-queued run reads.
    async fn stepless_run() -> RunSummary {
        let mut run = feat_1_runs().await.remove(0);
        run.id = RunId::new();
        run.steps.clear();
        run
    }

    #[tokio::test]
    async fn v_toggles_the_flow_view_and_is_typed_in_a_modal() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        assert_eq!(pane.view, View::List, "the pane starts on the list");
        assert_eq!(
            pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(pane.view, View::Flow);
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        assert_eq!(pane.view, View::List);

        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx());
        assert_eq!(
            pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(pane.view, View::List, "`v` in the note is a letter");
        assert_eq!(footer(&pane)[1], "v", "and it is typed");
    }

    #[tokio::test]
    async fn j_and_k_in_flow_move_the_same_cursor_across_runs() {
        let shell = Shell::new();
        let first = feat_1_runs().await.remove(0);
        let second = copied_run().await;
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(
            &StoreReply::Runs(vec![first.clone(), second.clone()]),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        assert_eq!(pane.graph.borrow().shown_run(), Some(first.id));
        assert_eq!(
            pane.graph.borrow().selected(),
            Some(first.steps[0].id.to_string())
        );

        for _ in 0..4 {
            pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        }
        assert_eq!(pane.selected_step(), Some(second.steps[0].id));
        assert_eq!(pane.graph.borrow().shown_run(), Some(second.id));
        assert_eq!(
            pane.graph.borrow().selected(),
            Some(second.steps[0].id.to_string())
        );

        pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        assert_eq!(pane.selected_step(), Some(first.steps[3].id));
        assert_eq!(pane.graph.borrow().shown_run(), Some(first.id));
        assert_eq!(
            pane.graph.borrow().selected(),
            Some(first.steps[3].id.to_string())
        );
    }

    /// ANA-12 invariant 2: the flow's selected node is the list's cursor, so an action key sends
    /// exactly what it sends in the list.
    #[tokio::test]
    async fn action_keys_in_flow_send_what_the_list_sends() {
        for (code, down) in [
            (KeyCode::Char('a'), 1),
            (KeyCode::Char('s'), 3),
            (KeyCode::Enter, 1),
            (KeyCode::Char('r'), 0),
        ] {
            let (_, listed, _) = allowed_emit(key(code), down).await;

            let shell = Shell::new();
            let (mut pane, _) = driven(&shell, true).await;
            pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
            assert!(
                matches!(requests(shell.emit.take()).as_slice(),
                    [StoreRequest::ToolCalls { item }] if *item == ids::HTUI_FEAT_1),
                "`v` asks only for the chips"
            );
            for _ in 0..down {
                pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
            }
            assert_eq!(pane.on_key(key(code), &mut shell.ctx()), Handled::Consumed);
            let flowed = shell.emit.take();
            assert_eq!(flowed.len(), 1, "{code:?} sends one action: {flowed:?}");
            assert_eq!(
                format!("{:?}", flowed[0]),
                format!("{listed:?}"),
                "{code:?} {down} down"
            );
        }
    }

    #[tokio::test]
    async fn plus_minus_and_equals_pass_in_the_list_view() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        for c in ['+', '-', '='] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(c)), &mut shell.ctx()),
                Handled::Pass,
                "`{c}` is not the list's"
            );
        }

        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        assert_eq!(
            pane.on_key(key(KeyCode::Char('+')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(pane.graph.borrow().zoom() > 1.0);
        for c in ['-', '='] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(c)), &mut shell.ctx()),
                Handled::Consumed,
                "`{c}` is the flow's"
            );
        }
    }

    #[tokio::test]
    async fn page_keys_are_a_no_op_in_flow() {
        let shell = Shell::new();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let mut runs = vec![stepless_run().await];
        runs.extend(feat_1_runs().await);
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let scroll = pane.scroll;
        for code in [KeyCode::PageDown, KeyCode::PageUp] {
            assert_eq!(pane.on_key(key(code), &mut shell.ctx()), Handled::Consumed);
            assert_eq!(
                pane.scroll, scroll,
                "{code:?} leaves the list's scroll alone"
            );
        }
    }

    #[tokio::test]
    async fn a_runs_re_read_keeps_the_cursor_node_selected() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        let implement = pane.runs[0].steps[2].id;

        let mut runs = vec![stepless_run().await];
        runs.extend(feat_1_runs().await);
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        assert_eq!(pane.selected_step(), Some(implement));
        assert_eq!(pane.graph.borrow().selected(), Some(implement.to_string()));
    }

    #[tokio::test]
    async fn the_flow_view_draws_the_run_line_then_the_canvas() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let lines = lines(&pane, &shell);
        assert!(lines[0].starts_with("graph"), "{lines:#?}");
        assert!(
            lines.iter().any(|line| line.contains("0.1 done")),
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("kind")),
            "no list header: {lines:#?}"
        );
    }

    #[tokio::test]
    async fn a_run_with_no_steps_says_so_in_flow() {
        let shell = Shell::new();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        let mut runs = vec![stepless_run().await];
        runs.extend(feat_1_runs().await);
        pane.on_reply(&StoreReply::Runs(runs), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let lines = lines(&pane, &shell);
        // Rows 0 and 1 are the run header (review L3).
        assert_eq!(lines[2], NO_STEPS_YET, "{lines:#?}");
    }

    /// MOD-28 review L3: the flow's head is the list's whole run header (the failure, a pending
    /// cancel, a queued command), and the cursor step's parked reason under it.
    #[tokio::test]
    async fn the_flow_view_shows_the_run_s_failure_and_the_cursor_step_s_note() {
        let shell = Shell::new();
        let mut run = feat_1_runs().await.remove(0);
        run.status = RunStatus::Failed;
        run.failure = Some("the box went away".to_owned());
        run.steps[1].status = StepStatus::AwaitingApproval;
        run.steps[1].gate_note = Some("needs a second look".to_owned());
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(vec![run]), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        let lines = lines(&pane, &shell);
        assert!(lines[0].starts_with("graph"), "{lines:#?}");
        assert!(
            lines[1].starts_with("manual"),
            "the run's mode line: {lines:#?}"
        );
        assert!(
            lines[2].contains("the box went away"),
            "the failure: {lines:#?}"
        );
        assert!(
            lines[3].contains("needs a second look"),
            "the cursor step's note: {lines:#?}"
        );
    }

    /// MOD-28 review H1: a digit in the flow answers the request the flow draws, as in the list.
    #[tokio::test]
    async fn the_flow_view_draws_the_pending_request_a_digit_answers() {
        let shell = Shell::new();
        let (mut pane, pending) = asking(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let _ = shell.emit.take();
        let lines = lines(&pane, &shell);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("asks: execute: cargo test")),
            "the request's summary: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("Allow once")),
            "its strip: {lines:#?}"
        );

        assert_eq!(
            pane.on_key(key(KeyCode::Char('2')), &mut shell.ctx()),
            Handled::Consumed
        );
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [StoreRequest::AnswerPermission { permission, option_id }]
                if *permission == pending.id && option_id == "reject"),
            "{sent:?}"
        );
    }

    /// MOD-28 review L4: with no request pending, the flow keeps neither `Tab` nor a digit, so
    /// both reach the global keymap.
    #[tokio::test]
    async fn tab_and_a_digit_pass_in_flow_with_no_pending_request() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let _ = shell.emit.take();
        for code in [KeyCode::Tab, KeyCode::BackTab, KeyCode::Char('3')] {
            assert_eq!(
                pane.on_key(key(code), &mut shell.ctx()),
                Handled::Pass,
                "{code:?}"
            );
        }
        assert!(shell.emit.is_empty());
    }

    #[tokio::test]
    async fn the_flow_view_survives_an_item_change() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_item_change(Some(ids::HTUI_ANA_2));
        assert_eq!(pane.view, View::Flow);
    }

    // -----------------------------------------------------------------------------------------
    // MOD-72: the tool-call chips (plan D4, D7).
    // -----------------------------------------------------------------------------------------

    /// One `read` call on `step`.
    fn read_on(step: StepId) -> ToolCallCount {
        ToolCallCount {
            step,
            tool_kind: "read".to_owned(),
            calls: 1,
        }
    }

    /// D4: into the flow asks once; back to the list asks nothing.
    #[tokio::test]
    async fn v_into_the_flow_asks_for_the_tool_calls_once() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [StoreRequest::ToolCalls { item }]
                if *item == ids::HTUI_FEAT_1),
            "{sent:?}"
        );
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        assert!(shell.emit.take().is_empty(), "the list never asks");
    }

    /// D4, blueprint E15: in the flow, every `Runs` reply asks for the chips, last; the list's
    /// lists stay as `the_first_runs_reply_subscribes_once_and_asks_for_the_actions` pins them.
    #[tokio::test]
    async fn a_runs_reply_in_flow_asks_for_the_tool_calls_last() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        let _ = shell.emit.take();
        pane.on_reply(&StoreReply::Runs(feat_1_runs().await), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [
                StoreRequest::RunActions(asked),
                StoreRequest::RelayView { item: relayed },
                StoreRequest::ToolCalls { item: counted },
            ] if *asked == ids::HTUI_FEAT_1
                && *relayed == ids::HTUI_FEAT_1
                && *counted == ids::HTUI_FEAT_1),
            "{sent:?}"
        );
    }

    /// D7: a reply for this item lands in the map and the flow draws it.
    #[tokio::test]
    async fn a_tool_calls_reply_draws_the_chips_in_flow() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        let _ = shell.emit.take();
        pane.on_reply(
            &StoreReply::ToolCalls {
                item: ids::HTUI_FEAT_1,
                counts: vec![read_on(ids::STEP_PLAN)],
            },
            &mut shell.ctx(),
        );
        assert_eq!(pane.tool_calls[&ids::STEP_PLAN].len(), 1);
        let lines = lines(&pane, &shell);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\u{2692} read\u{d7}1")),
            "{lines:#?}"
        );
    }

    /// D4: another item's chips are not this pane's.
    #[tokio::test]
    async fn a_tool_calls_reply_for_another_item_is_dropped() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::ToolCalls {
                item: ids::HTUI_ANA_2,
                counts: vec![read_on(ids::STEP_PLAN)],
            },
            &mut shell.ctx(),
        );
        assert!(pane.tool_calls.is_empty());
    }

    /// D7: an item change clears the map.
    #[tokio::test]
    async fn an_item_change_forgets_the_tool_calls() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::ToolCalls {
                item: ids::HTUI_FEAT_1,
                counts: vec![read_on(ids::STEP_PLAN)],
            },
            &mut shell.ctx(),
        );
        assert!(!pane.tool_calls.is_empty(), "the reply applied");
        pane.on_item_change(Some(ids::HTUI_ANA_2));
        assert!(pane.tool_calls.is_empty());
    }

    // -----------------------------------------------------------------------------------------
    // What is on screen (verifier findings: the modal of an item with no run, the fold, the
    // wrapped document).
    // -----------------------------------------------------------------------------------------

    /// `C` needs no run: an item that is done, failed or blocked with none closes out, and its
    /// confirmation is drawn under the empty list rather than taking the keyboard unseen.
    #[tokio::test]
    async fn shift_c_on_an_item_with_no_run_shows_the_close_out() {
        let shell = Shell::new();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_ANA_2));
        pane.on_reply(&StoreReply::Runs(Vec::new()), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::RunActions(Box::new(verdicts(
                ids::HTUI_ANA_2,
                &[],
                true,
                DocumentId::new(),
            ))),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();

        pane.on_key(shift('C'), &mut shell.ctx());
        assert!(pane.captures_input(), "the close-out is open");
        let drawn = lines(&pane, &shell);
        assert_eq!(drawn[0], "No runs for this item.", "{drawn:#?}");
        assert!(
            drawn
                .iter()
                .any(|line| line == "counting what the close-out writes\u{2026}"),
            "the counting stage is on screen: {drawn:#?}"
        );

        pane.on_reply(
            &StoreReply::Orch(OrchReply::CloseOutPreview(Box::new(preview()))),
            &mut shell.ctx(),
        );
        let drawn = lines(&pane, &shell);
        assert!(
            drawn
                .iter()
                .any(|line| line.starts_with("close FEAT-1 as done \u{b7}")),
            "the warning is on screen: {drawn:#?}"
        );
        pane.on_key(key(KeyCode::Char('y')), &mut shell.ctx());
        let drawn = lines(&pane, &shell);
        assert!(
            drawn
                .iter()
                .any(|line| line.starts_with("type FEAT-1 to close it:")),
            "the typed stage is on screen: {drawn:#?}"
        );
    }

    /// A run with more steps than the pane has rows: the cursor is followed below the fold, the
    /// header stays on top, and a modal's footer does not cover the step it is about.
    #[tokio::test]
    async fn the_cursor_is_followed_below_the_fold() {
        let shell = Shell::new();
        let mut run = feat_1_runs().await.remove(0);
        let base = run.steps[0].clone();
        run.steps = (0..12)
            .map(|position| RunStepSummary {
                id: StepId::new(),
                position,
                attempt: 1,
                fanout_index: 0,
                ..base.clone()
            })
            .collect();
        let mut pane = RunsTab::new();
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(vec![run.clone()]), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::RunActions(Box::new(verdicts(
                ids::HTUI_FEAT_1,
                std::slice::from_ref(&run),
                true,
                DocumentId::new(),
            ))),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();

        /// The drawn row the cursor is on.
        fn cursor_row(drawn: &[String]) -> Option<&String> {
            drawn.iter().find(|line| line.starts_with(CURSOR))
        }

        for _ in 0..11 {
            pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        }
        assert_eq!(pane.selected_step(), Some(run.steps[11].id));
        let drawn = lines(&pane, &shell);
        assert!(drawn[0].starts_with("kind"), "the header stays: {drawn:#?}");
        assert!(
            cursor_row(&drawn).is_some_and(|line| line.contains("11.1")),
            "the last step is on screen with the cursor on it: {drawn:#?}"
        );

        pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx());
        let drawn = lines(&pane, &shell);
        assert!(
            drawn.iter().any(|line| line == "reject with a note:"),
            "{drawn:#?}"
        );
        assert!(
            cursor_row(&drawn).is_some_and(|line| line.contains("11.1")),
            "the note's footer does not cover the step it rejects: {drawn:#?}"
        );
        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());

        for _ in 0..11 {
            pane.on_key(key(KeyCode::Char('K')), &mut shell.ctx());
        }
        let drawn = lines(&pane, &shell);
        assert!(
            cursor_row(&drawn).is_some_and(|line| line.contains("0.1")),
            "and back up to the first: {drawn:#?}"
        );
    }

    /// D173: a document of long paragraphs wraps into more rows than it has lines, and its last
    /// row is still reachable.
    #[tokio::test]
    async fn a_wrapped_document_scrolls_to_its_last_row() {
        let shell = Shell::new();
        let (mut pane, id) = driven(&shell, true).await;
        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('o')), &mut shell.ctx());
        let body = (0..10)
            .map(|line| {
                (0..40)
                    .map(|word| format!("w{line}x{word}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        pane.on_reply(
            &StoreReply::Document(Box::new(Some(document(id, &body)))),
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();
        let _ = lines(&pane, &shell);

        for _ in 0..50 {
            pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        }
        let drawn = lines(&pane, &shell);
        assert!(
            drawn[0].contains("w9x39"),
            "`PageDown` stops with the last row on top: {drawn:#?}"
        );
        pane.on_key(key(KeyCode::Char('k')), &mut shell.ctx());
        let drawn = lines(&pane, &shell);
        assert!(
            drawn[1].contains("w9x39"),
            "and `k` walks back one row: {drawn:#?}"
        );
    }

    // -----------------------------------------------------------------------------------------
    // MOD-42 plan D14: the relayed permission requests and the pending cancels.
    // -----------------------------------------------------------------------------------------

    /// An option of a relayed request.
    fn option(id: &str, label: &str, kind: RelayOptionKind) -> RelayOption {
        RelayOption {
            id: id.to_owned(),
            label: label.to_owned(),
            kind,
        }
    }

    /// `[1] Allow once  [2] Reject once`.
    fn two_options() -> Vec<RelayOption> {
        vec![
            option("allow", "Allow once", RelayOptionKind::AllowOnce),
            option("reject", "Reject once", RelayOptionKind::RejectOnce),
        ]
    }

    /// A pending request on `step` of `run`.
    fn pending_on(
        run: RunId,
        step: StepId,
        summary: Option<&str>,
        options: Vec<RelayOption>,
    ) -> StepPermission {
        StepPermission {
            id: PermissionId::new(),
            run_id: run,
            run_step_id: step,
            session: RelaySessionId::new(),
            request_id: "req-1".to_owned(),
            tool_call_id: Some("call-1".to_owned()),
            summary: summary.map(str::to_owned),
            options,
            status: PermissionStatus::Pending,
            option_id: None,
            answered_by: None,
            answered_box: None,
            created_at: demo_at(1, 9),
            answered_at: None,
            resolved_at: None,
        }
    }

    /// A `RelayView` reply for `item`.
    fn relay(item: ItemId, permissions: Vec<StepPermission>, cancels: Vec<RunId>) -> StoreReply {
        StoreReply::RelayView {
            item,
            view: Box::new(RelayView {
                permissions,
                cancels,
            }),
        }
    }

    /// The `FEAT-1` pane with a two-option request pending on `prd`, the step under the cursor.
    async fn asking(shell: &Shell) -> (RunsTab, StepPermission) {
        let mut pane = pane(shell).await;
        let pending = pending_on(
            ids::RUN_1,
            ids::STEP_PRD,
            Some("execute: cargo test"),
            two_options(),
        );
        pane.on_reply(
            &relay(ids::HTUI_FEAT_1, vec![pending.clone()], Vec::new()),
            &mut shell.ctx(),
        );
        assert!(shell.emit.is_empty(), "a relay view asks for nothing");
        (pane, pending)
    }

    /// D14: a digit on a step with a pending request answers it with that option's id.
    #[tokio::test]
    async fn a_digit_answers_the_pending_request_under_the_cursor() {
        let shell = Shell::new();
        let (mut pane, pending) = asking(&shell).await;
        assert_eq!(pane.selected_step(), Some(ids::STEP_PRD));

        assert_eq!(
            pane.on_key(key(KeyCode::Char('2')), &mut shell.ctx()),
            Handled::Consumed
        );
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [StoreRequest::AnswerPermission { permission, option_id }]
                if *permission == pending.id && option_id == "reject"),
            "one answer, the second option's id: {sent:?}"
        );
    }

    /// D14: a second digit while the first answer is in flight sends nothing. Its
    /// `AnswerPermission` would supersede the first in the app's staleness gate
    /// (`App::dispatch`), so the first answer's `PermissionAnswered` would be dropped and the
    /// second's `already answered` refusal would be all the status line showed. The request stays
    /// taken while a relay view drawn before the answer still lists it; a refusal frees it.
    #[tokio::test]
    async fn a_second_digit_while_the_answer_is_in_flight_sends_nothing() {
        let shell = Shell::new();
        let (mut pane, pending) = asking(&shell).await;

        assert_eq!(
            pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(
            pane.on_key(key(KeyCode::Char('2')), &mut shell.ctx()),
            Handled::Consumed,
            "the step still shows its request, so the digit is still the pane's"
        );
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [StoreRequest::AnswerPermission { permission, option_id }]
                if *permission == pending.id && option_id == "allow"),
            "exactly one answer, the first digit's: {sent:?}"
        );

        // A view read before the answer landed still lists the request as pending.
        pane.on_reply(
            &relay(ids::HTUI_FEAT_1, vec![pending.clone()], Vec::new()),
            &mut shell.ctx(),
        );
        pane.on_reply(
            &StoreReply::PermissionAnswered {
                permission: pending.id,
            },
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();
        assert_eq!(
            pane.on_key(key(KeyCode::Char('2')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(
            shell.emit.is_empty(),
            "an answered request is never answered again: {:?}",
            shell.emit.take()
        );

        // A refused answer leaves the request open to another try.
        let (mut pane, pending) = asking(&shell).await;
        pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx());
        let _ = shell.emit.take();
        pane.on_reply(
            &StoreReply::Failed {
                request: "answer_permission",
                message: htui_core::model::EXECUTOR_GONE.to_owned(),
            },
            &mut shell.ctx(),
        );
        let _ = shell.emit.take();
        pane.on_key(key(KeyCode::Char('2')), &mut shell.ctx());
        let sent = requests(shell.emit.take());
        assert!(
            matches!(sent.as_slice(), [StoreRequest::AnswerPermission { permission, option_id }]
                if *permission == pending.id && option_id == "reject"),
            "after a refusal the next digit answers again: {sent:?}"
        );
    }

    /// D14, OQ-4: offline the relay read is an empty view (`try_serve`); the pane takes it
    /// quietly - no request, no strip, no `cancel requested`, and the digits stay tab select.
    #[tokio::test]
    async fn an_empty_relay_view_draws_nothing_and_digits_pass() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let before = lines(&pane, &shell);
        pane.on_reply(
            &relay(ids::HTUI_FEAT_1, Vec::new(), Vec::new()),
            &mut shell.ctx(),
        );
        assert!(shell.emit.is_empty(), "{:?}", shell.emit.take());
        assert_eq!(
            pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Pass
        );
        assert!(shell.emit.is_empty(), "{:?}", shell.emit.take());
        assert_eq!(
            lines(&pane, &shell),
            before,
            "the drawing is the one with no view"
        );
    }

    /// D14, `chat/mod.rs`: while a request is pending every digit is the pane's, so one past the
    /// offered options neither answers nor switches the tab.
    #[tokio::test]
    async fn a_digit_past_the_offered_options_is_consumed_and_sends_nothing() {
        let shell = Shell::new();
        let (mut pane, _) = asking(&shell).await;
        for digit in ['3', '5', '9'] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(digit)), &mut shell.ctx()),
                Handled::Consumed,
                "`{digit}`"
            );
        }
        assert!(shell.emit.is_empty(), "{:?}", shell.emit.take());
    }

    /// D14: with nothing pending under the cursor a digit passes on to the global tab select.
    #[tokio::test]
    async fn a_digit_with_no_pending_request_passes_to_tab_select() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Pass,
            "no relay view at all"
        );

        let elsewhere = pending_on(ids::RUN_1, ids::STEP_PLAN, None, two_options());
        pane.on_reply(
            &relay(ids::HTUI_FEAT_1, vec![elsewhere], Vec::new()),
            &mut shell.ctx(),
        );
        assert_eq!(pane.selected_step(), Some(ids::STEP_PRD));
        for digit in ['1', '2', '9'] {
            assert_eq!(
                pane.on_key(key(KeyCode::Char(digit)), &mut shell.ctx()),
                Handled::Pass,
                "a request on another step does not take `{digit}`"
            );
        }
        assert!(shell.emit.is_empty());
    }

    /// D14: a relay view answered for another item is not this pane's, and an item change drops
    /// the one it kept.
    #[tokio::test]
    async fn a_relay_view_for_another_item_is_dropped() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let pending = pending_on(ids::RUN_1, ids::STEP_PRD, None, two_options());
        pane.on_reply(
            &relay(ids::HTUI_ANA_2, vec![pending], Vec::new()),
            &mut shell.ctx(),
        );
        assert_eq!(
            pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Pass
        );
        assert!(shell.emit.is_empty());

        let (mut pane, _) = asking(&shell).await;
        pane.on_item_change(Some(ids::HTUI_FEAT_1));
        pane.on_reply(&StoreReply::Runs(feat_1_runs().await), &mut shell.ctx());
        let _ = shell.emit.take();
        assert_eq!(
            pane.on_key(key(KeyCode::Char('1')), &mut shell.ctx()),
            Handled::Pass,
            "an item change forgets the relay view until the next one arrives"
        );
    }

    /// D14: the summary and the strip are two more lines, each exactly the pane's 43 columns
    /// whatever they hold, drawn right under the step's own two.
    #[tokio::test]
    async fn a_step_with_a_pending_request_takes_two_more_lines_each_forty_three_wide() {
        let theme = Theme::default();
        let long = "a summary far too long for any pane ".repeat(6);
        let four = vec![
            option(
                "allow-once",
                "Allow this one call of a very long tool",
                RelayOptionKind::AllowOnce,
            ),
            option(
                "allow-always",
                "Allow every call like it from now on",
                RelayOptionKind::AllowAlways,
            ),
            option(
                "reject-once",
                "Reject this one call",
                RelayOptionKind::RejectOnce,
            ),
            option(
                "reject-always",
                "Reject every call like it",
                RelayOptionKind::RejectAlways,
            ),
        ];
        assert!(cell_width(&long) >= 200);
        for (summary, options) in [
            (Some("execute: cargo test"), two_options()),
            (Some(long.as_str()), two_options()),
            (None, two_options()),
            (Some("edit: src/lib.rs"), four),
        ] {
            let pending = pending_on(ids::RUN_1, ids::STEP_PRD, summary, options);
            let [asks, strip] = permission_lines(&pending, &theme);
            for line in [&asks, &strip] {
                assert_eq!(
                    line.width(),
                    PANE,
                    "{:?} is not {PANE} columns wide",
                    text(line)
                );
            }
            let asks = text(&asks);
            assert!(asks.starts_with(&blank(INDENT)), "{asks:?}");
            let shown = summary.unwrap_or("a permission");
            assert!(
                asks.trim_start()
                    .starts_with(&format!("asks: {}", &shown[..shown.len().min(10)])),
                "{asks:?}"
            );
            assert!(text(&strip).contains("[1] "), "{:?}", text(&strip));
        }

        let shell = Shell::new();
        let (pane, _) = asking(&shell).await;
        let drawn = lines(&pane, &shell);
        let at = drawn
            .iter()
            .position(|line| line.contains("prd"))
            .expect("the `prd` step is listed");
        assert!(
            drawn[at + 2].contains("asks: execute: cargo test"),
            "the summary is right under the step's second line: {drawn:#?}"
        );
        assert!(
            drawn[at + 3].contains("[1] Allow once  [2] Reject once"),
            "and the strip under it: {drawn:#?}"
        );
        assert!(
            drawn[at + 4].contains("plan"),
            "the next step follows: {drawn:#?}"
        );
        assert_eq!(
            drawn.iter().filter(|line| !line.is_empty()).count(),
            2 + 2 + 4 * 2 + 2,
            "the header, the run, four two-line steps and the request's two"
        );
    }

    /// D14: a run with a pending cancel says so under its grid, which does not move.
    #[tokio::test]
    async fn a_run_with_a_pending_cancel_says_cancel_requested() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let before = lines(&pane, &shell);
        pane.on_reply(
            &relay(ids::HTUI_FEAT_1, Vec::new(), vec![ids::RUN_1]),
            &mut shell.ctx(),
        );
        let after = lines(&pane, &shell);
        assert_eq!(
            after[..4],
            before[..4],
            "the header and the run grid are unchanged"
        );
        assert_eq!(
            after[4].trim(),
            "cancel requested",
            "one more line, under the grid: {after:#?}"
        );
        assert_eq!(after[5..], before[4..before.len() - 1], "{after:#?}");
    }

    /// R-51: a `Waiting` frame shows the run waiting without a re-read, and the next frame that
    /// invalidates clears it.
    #[tokio::test]
    async fn a_waiting_frame_shows_the_run_waiting_and_a_later_frame_clears_it() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        let before = lines(&pane, &shell);
        assert!(
            !before
                .iter()
                .any(|line| line.contains("waiting for the walk"))
        );
        pane.on_reply(
            &StoreReply::RunStream(RunFrame {
                item: ids::HTUI_FEAT_1,
                run: Some(ids::RUN_1),
                kind: FrameKind::Waiting,
            }),
            &mut shell.ctx(),
        );
        assert!(shell.emit.is_empty(), "a wait is not a re-read");
        let waiting = lines(&pane, &shell);
        assert!(
            waiting
                .iter()
                .any(|line| line.trim() == "waiting for the walk"),
            "{waiting:#?}"
        );
        assert!(waiting.iter().all(|line| cell_width(line) <= PANE));
        pane.on_reply(
            &StoreReply::RunStream(RunFrame {
                item: ids::HTUI_FEAT_1,
                run: Some(ids::RUN_1),
                kind: FrameKind::Rested(Rest {
                    run: RunStatus::AwaitingApproval,
                    position: Some(1),
                    failure: None,
                }),
            }),
            &mut shell.ctx(),
        );
        assert!(
            is_one_re_read(&requests(shell.emit.take()), ids::HTUI_FEAT_1),
            "the rest re-reads once"
        );
        let after = lines(&pane, &shell);
        assert!(
            !after
                .iter()
                .any(|line| line.contains("waiting for the walk"))
        );
    }

    /// R-51 (review F1): the walk's own frames (`Changed` at a step start, `SessionDone` after
    /// one) land while the command is still queued, so they keep the line; the rest ends it.
    #[tokio::test]
    async fn a_walks_own_frames_keep_the_waiting_line() {
        let shell = Shell::new();
        let (mut pane, _) = driven(&shell, true).await;
        let frame = |kind| {
            StoreReply::RunStream(RunFrame {
                item: ids::HTUI_FEAT_1,
                run: Some(ids::RUN_1),
                kind,
            })
        };
        let shown = |pane: &_| {
            lines(pane, &shell)
                .iter()
                .any(|line| line.trim() == "waiting for the walk")
        };
        pane.on_reply(&frame(FrameKind::Waiting), &mut shell.ctx());
        assert!(shown(&pane));
        for kind in [
            FrameKind::Changed,
            FrameKind::SessionDone {
                step: ids::STEP_PRD,
            },
            FrameKind::Started,
        ] {
            pane.on_reply(&frame(kind.clone()), &mut shell.ctx());
            let _ = shell.emit.take();
            assert!(shown(&pane), "{kind:?} keeps the line");
        }
        pane.on_reply(&frame(FrameKind::Adopted), &mut shell.ctx());
        let _ = shell.emit.take();
        assert!(!shown(&pane), "Adopted ends the wait");
        pane.on_reply(&frame(FrameKind::Waiting), &mut shell.ctx());
        pane.on_reply(&frame(FrameKind::Error("x".into())), &mut shell.ctx());
        let _ = shell.emit.take();
        assert!(!shown(&pane), "Error ends the wait");
    }

    /// D14, B-13: an answer, applied or refused, re-reads the runs; the refusal's sentence is on
    /// the status line already.
    #[tokio::test]
    async fn an_answer_reply_re_reads_the_runs() {
        let shell = Shell::new();
        let (mut pane, pending) = asking(&shell).await;
        pane.on_reply(
            &StoreReply::PermissionAnswered {
                permission: pending.id,
            },
            &mut shell.ctx(),
        );
        assert!(is_one_re_read(
            &requests(shell.emit.take()),
            ids::HTUI_FEAT_1
        ));

        pane.on_reply(
            &StoreReply::Failed {
                request: "answer_permission",
                message: htui_core::model::EXECUTOR_GONE.to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(is_one_re_read(
            &requests(shell.emit.take()),
            ids::HTUI_FEAT_1
        ));
    }
}
