//! The request and reply shapes the views speak, and every verdict for an item (MOD-4 D182,
//! D184), with the progress sink every engine of the runtime writes through (MOD-41 plan D6).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui_agent::driver::{AgentDriver, AgentSession, DriverCaps, DriverFuture, SessionSpec};
use htui_agent::error::DriverError;
use htui_agent::event::DoneEvent;
use htui_core::model::{
    DocumentHead, DocumentId, Item, ItemId, NewDocument, ProjectId, Run, RunId, RunStatus, RunStep,
    Scope, SnapshotPhase, StepId, StepStatus, WaitingCandidate, WaitingPermission,
};
use htui_core::store::{Result as StoreResult, StoreError};
use htui_orch::command::{answer_gate_enabled, cancel_enabled, select_enabled};
use htui_orch::status::{group_at, judge_at, resumable};
use htui_orch::{
    Command, CommandOutcome, Cursor, EngineError, GateAnswer, Rest, SessionKey, SessionSink,
    UnblockCase, accept_enabled, cleanup_enabled, close_out_enabled, cursor, phase_at,
    promote_enabled, retry_admitted, snapshot_of, start_enabled, unblock_enabled,
};
use htui_store::DATABASE_UNREACHABLE;

use crate::address::Publish;

/// Blueprint D209: the TUI's `StoreRequest::name` of each [`OrchRequest`], in [`OrchRequest`]'s
/// order — the nine commands of [`Command`], then the close-out preview and the cleanup retry. The status line reads `retry_step: …`, and the Runs
/// pane and the Chat tab match a `Failed` reply's `request` against this list.
pub const ORCH_NAMES: [&str; 11] = [
    "start_run",
    "answer_gate",
    "retry_step",
    "cancel_run",
    "select_fanout",
    "promote_step",
    "accept_artifact",
    "unblock",
    "close_out",
    "close_out_preview",
    "cleanup_run",
];

/// Plan D154: one orchestrator command or read.
#[derive(Debug, Clone)]
pub enum OrchRequest {
    /// One of ANA-2 §6.2's verbs, dispatched to the engine on a task of its own.
    Command(Command),
    /// Plan D167's first confirmation: what a close-out would write, read-only.
    CloseOutPreview {
        /// The item to close.
        item: ItemId,
    },
    /// Plan D177 (R-25): a manual retry of a terminal run's cleanup.
    Cleanup {
        /// The terminal run.
        run: RunId,
    },
}

impl OrchRequest {
    /// Blueprint D209: this request's entry of [`ORCH_NAMES`]. A `const fn`, because
    /// `StoreRequest::name` is one.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Command(command) => match command {
                Command::StartRun { .. } => ORCH_NAMES[0],
                Command::AnswerGate { .. } => ORCH_NAMES[1],
                Command::RetryStep { .. } => ORCH_NAMES[2],
                Command::CancelRun { .. } => ORCH_NAMES[3],
                Command::SelectFanout { .. } => ORCH_NAMES[4],
                Command::PromoteStep { .. } => ORCH_NAMES[5],
                Command::AcceptArtifact { .. } => ORCH_NAMES[6],
                Command::Unblock { .. } => ORCH_NAMES[7],
                Command::CloseOut { .. } => ORCH_NAMES[8],
            },
            Self::CloseOutPreview { .. } => ORCH_NAMES[9],
            Self::Cleanup { .. } => ORCH_NAMES[10],
        }
    }
}

/// The answer to one [`OrchRequest`], sent once at the request's own `seq` (R-41).
#[derive(Debug, Clone)]
pub enum OrchReply {
    /// A command's outcome.
    Done(Box<CommandOutcome>),
    /// To the Chat tab only (D165, D191): the promoted step and how its chat opens.
    Promoted {
        /// The promoted step, which keeps its id.
        step: StepId,
        /// Its run.
        run: RunId,
        /// `run_step.phase_name`.
        phase: String,
        /// `agent.name` of the step's agent.
        agent: String,
        /// `run_step.model`.
        model: Option<String>,
        /// Whether the chat resumes the step's own session or opens with the handoff prompt.
        via: Via,
    },
    /// [`OrchRequest::CloseOutPreview`]'s figures.
    CloseOutPreview(Box<htui_orch::closeout::Preview>),
    /// [`OrchRequest::Cleanup`] ran to its end.
    CleanedUp {
        /// The run cleaned up.
        run: RunId,
    },
}

/// How a promoted step's chat opens (blueprint D192).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The step's own agent session, resumed.
    Resumed,
    /// A fresh session opened with the `handoff` prompt.
    Handoff,
}

/// Plan D172: one frame of an item's run stream. Frames are invalidations: the pane re-reads the
/// item's runs on each.
#[derive(Debug, Clone)]
pub struct RunFrame {
    /// The item whose runs changed.
    pub item: ItemId,
    /// The run that changed, when one did.
    pub run: Option<RunId>,
    /// What happened.
    pub kind: FrameKind,
}

impl RunFrame {
    /// The acknowledgement a `RunStream` subscription is answered with at once (blueprint §0a
    /// point 3, D183).
    #[must_use]
    pub const fn subscribed(item: ItemId) -> Self {
        Self {
            item,
            run: None,
            kind: FrameKind::Subscribed,
        }
    }
}

/// What a [`RunFrame`] reports.
#[derive(Debug, Clone)]
pub enum FrameKind {
    /// The subscription is live.
    Subscribed,
    /// A run was created, `queued`.
    Started,
    /// A session of `step` ended (the progress sink's `after_done`).
    SessionDone {
        /// The step whose session ended.
        step: StepId,
    },
    /// A walk rested.
    Rested(Rest),
    /// A command changed the item's rows with no walk to rest: a reopen, a close-out, a cleanup
    /// (D200), or a step went live (R-40).
    Changed,
    /// A command is queued behind a live walk of the run (R-51). Nothing changed in the rows.
    Waiting,
    /// A sweep adopted the run; its walk resumes on a task of its own.
    Adopted,
    /// A command or walk failed, with the sentence.
    Error(String),
}

/// One action's enabling verdict: `Ok`, or the refusal's `Display` (D182, D184).
pub type Enabled = Result<(), String>;

/// Blueprint D182: every action's verdict for one item, from the engine's own guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemActions {
    /// The item.
    pub item: ItemId,
    /// `item.key`, empty when the item could not be read.
    pub key: String,
    /// `R`: start a run.
    pub run: Enabled,
    /// `u`: unblock.
    pub unblock: Enabled,
    /// Close-out.
    pub close_out: Enabled,
    /// Per run.
    pub runs: BTreeMap<RunId, RunActions>,
    /// Per step.
    pub steps: BTreeMap<StepId, StepActions>,
}

/// The run-level verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunActions {
    /// Cancel the run.
    pub cancel: Enabled,
    /// Retry its terminal cleanup (R-25).
    pub cleanup: Enabled,
}

/// The step-level verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepActions {
    /// Approve the parked gate.
    pub approve: Enabled,
    /// Reject it with a note.
    pub reject: Enabled,
    /// Retry the step.
    pub retry: Enabled,
    /// Promote it to a chat.
    pub promote: Enabled,
    /// Accept a promoted step's artefact.
    pub accept: Enabled,
    /// Select it as a fan-out winner.
    pub select: Enabled,
    /// The newest head of the phase's `output_kind` the step produced.
    pub open: Result<DocumentId, String>,
}

/// Blueprint D206: the steps a chat of this process is live on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveChats(BTreeSet<StepId>);

impl LiveChats {
    /// The set of these steps.
    pub fn of(steps: impl IntoIterator<Item = StepId>) -> Self {
        Self(steps.into_iter().collect())
    }

    /// Whether no chat is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a chat is live on `step`.
    #[must_use]
    pub fn contains(&self, step: StepId) -> bool {
        self.0.contains(&step)
    }
}

/// Blueprint D182, D184: every verdict for `item`, from the engine's own admission functions
/// over the rows the engine reads — the item, its runs with their steps, and its document heads.
///
/// Off the server (`writer()` is `None`) every command verdict is [`DATABASE_UNREACHABLE`]: the
/// rows still come from the mirror, but no command can run (D174). A snapshot that does not decode
/// refuses every step verdict of its run with that sentence.
///
/// MOD-41 plan D7: over any [`WorkerHost`](htui_core::store::WorkerHost); the TUI passes its
/// `Backend`.
///
/// # Errors
/// The store's own read failures, and [`StoreError::NotFound`] for an item a reachable server does
/// not hold.
pub async fn actions<H: htui_core::store::WorkerHost>(
    host: &H,
    item: ItemId,
    live: &LiveChats,
) -> StoreResult<ItemActions> {
    let online = host.writer().is_some();
    let row = host.item(item).await?;
    let row = match row {
        Some(row) => row,
        None if online => {
            return Err(StoreError::NotFound {
                entity: "item",
                id: item.to_string(),
            });
        }
        None => return Ok(unreachable_actions(item, String::new())),
    };
    let heads = host.documents(item).await?;
    let mut runs = Vec::new();
    for summary in host.runs(item).await? {
        let Some(run) = host.run(summary.id).await? else {
            continue;
        };
        let steps = host.run_steps(run.id).await?;
        runs.push((run, steps));
    }

    let mut actions = verdicts(&row, &runs, &heads, live);
    if !online {
        let offline = || Err(DATABASE_UNREACHABLE.to_owned());
        actions.run = offline();
        actions.unblock = offline();
        actions.close_out = offline();
        for verdict in actions.runs.values_mut() {
            *verdict = RunActions {
                cancel: offline(),
                cleanup: offline(),
            };
        }
        for verdict in actions.steps.values_mut() {
            verdict.approve = offline();
            verdict.reject = offline();
            verdict.retry = offline();
            verdict.promote = offline();
            verdict.accept = offline();
            verdict.select = offline();
        }
    }
    Ok(actions)
}

/// An item the mirror does not hold, off the server: nothing is enabled.
pub(crate) fn unreachable_actions(item: ItemId, key: String) -> ItemActions {
    let offline = || Err(DATABASE_UNREACHABLE.to_owned());
    ItemActions {
        item,
        key,
        run: offline(),
        unblock: offline(),
        close_out: offline(),
        runs: BTreeMap::new(),
        steps: BTreeMap::new(),
    }
}

/// The guards themselves, over rows already read.
fn verdicts(
    item: &Item,
    runs: &[(Run, Vec<RunStep>)],
    heads: &[DocumentHead],
    live: &LiveChats,
) -> ItemActions {
    let sentence = |err: EngineError| err.to_string();
    let mut actions = ItemActions {
        item: item.id,
        key: item.key.clone(),
        run: start_enabled(item).map_err(sentence),
        unblock: Ok(()),
        close_out: close_out_enabled(
            item,
            &runs.iter().map(|(run, _)| run.clone()).collect::<Vec<_>>(),
        )
        .map(|_| ())
        .map_err(sentence),
        runs: BTreeMap::new(),
        steps: BTreeMap::new(),
    };

    for (run, steps) in runs {
        // D212: while a step of this run is chatted with, the verbs that would move the run grey
        // with the refusal the worker answers them with, ahead of the engine's own guards.
        let chatting = chat_free(steps, live).map_err(sentence);
        actions.runs.insert(
            run.id,
            RunActions {
                cancel: chatting
                    .clone()
                    .and_then(|()| cancel_enabled(run).map_err(sentence)),
                cleanup: cleanup_enabled(run).map_err(sentence),
            },
        );
        let snapshot = match snapshot_of(run) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                let refusal = err.to_string();
                for step in steps {
                    actions.steps.insert(step.id, refused_step(&refusal));
                }
                continue;
            }
        };
        for step in steps {
            let phase = match phase_at(run.id, &snapshot, step.position) {
                Ok(phase) => phase,
                Err(err) => {
                    actions
                        .steps
                        .insert(step.id, refused_step(&err.to_string()));
                    continue;
                }
            };
            let open = newest_output(heads, &phase.output_kind, step.id);
            let has_output = open.is_ok();
            let select = chatting
                .clone()
                .and_then(|()| select_of(run, steps, step, &phase));
            actions.steps.insert(
                step.id,
                StepActions {
                    approve: chatting.clone().and_then(|()| {
                        answer_gate_enabled(step, &phase, has_output, &GateAnswer::Approved)
                            .map_err(sentence)
                    }),
                    reject: chatting.clone().and_then(|()| {
                        answer_gate_enabled(
                            step,
                            &phase,
                            has_output,
                            &GateAnswer::Rejected {
                                note: String::new(),
                            },
                        )
                        .map_err(sentence)
                    }),
                    retry: chatting.clone().and_then(|()| {
                        retry_admitted(run, item.status, steps, step, &phase).map_err(sentence)
                    }),
                    promote: promote_enabled(
                        run,
                        item.status,
                        steps,
                        step,
                        &phase,
                        !live.is_empty(),
                    )
                    .map_err(sentence),
                    accept: accept_enabled(steps, step, &phase, has_output, live.contains(step.id))
                        .map_err(sentence),
                    select,
                    open,
                },
            );
        }
    }
    actions.unblock = unblock_case(item, runs).map(drop);
    actions
}

/// `select` on `step` of `run`, ahead of the live-chat refusal: the one place [`verdicts`] and
/// [`selectable_slots`] read it (MOD-69 review M3).
fn select_of(run: &Run, steps: &[RunStep], step: &RunStep, phase: &SnapshotPhase) -> Enabled {
    if step.fanout_index >= 0 && phase.fan_out > 1 {
        select_enabled(
            run,
            &group_at(steps, step.position, step.attempt),
            step.id,
            step.position,
            step.attempt,
        )
        .map_err(|err| err.to_string())
    } else {
        Err(not_a_candidate(step.id))
    }
}

/// The `(position, attempt)` slots of `run` on whose candidates the Runs pane enables `select`
/// with no live chat: [`verdicts`]' `select` alone, without its clones of every run and its other
/// verdicts (MOD-69 review M3). A run whose snapshot does not decode has none, as every `select`
/// is refused there.
fn selectable_slots(run: &Run, steps: &[RunStep]) -> BTreeSet<(i32, i32)> {
    let Ok(snapshot) = snapshot_of(run) else {
        return BTreeSet::new();
    };
    steps
        .iter()
        .filter(|step| step.fanout_index >= 0)
        .filter(|step| {
            phase_at(run.id, &snapshot, step.position)
                .is_ok_and(|phase| select_of(run, steps, step, &phase).is_ok())
        })
        .map(|step| (step.position, step.attempt))
        .collect()
}

/// `u`'s case for `item` over its runs (MOD-69 blueprint E2): the one place both the Runs pane's
/// verdict ([`verdicts`], `.map(drop)`) and the waiting list read it, so the two cannot disagree on
/// whether, or how, an item is unblocked.
///
/// Every active run must decode its snapshot; the first that does not refuses with its sentence,
/// as [`verdicts`] always has. `runs` is in `ReadStore::runs` order (newest first), which
/// `unblock_enabled` is sensitive to (blueprint H-1).
fn unblock_case(item: &Item, runs: &[(Run, Vec<RunStep>)]) -> Result<UnblockCase, String> {
    let mut active = Vec::new();
    for (run, steps) in runs {
        if !run.status.is_active() {
            continue;
        }
        let snapshot = snapshot_of(run).map_err(|err| err.to_string())?;
        active.push((run.clone(), resumable(&cursor(&snapshot, steps), steps)));
    }
    unblock_enabled(item, &active).map_err(|err| err.to_string())
}

/// Why a row of the waiting-on-you list waits on a person (MOD-69 plan D3). Declaration order is
/// plan D9's last sort key, so `Ord` is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaitingReason {
    /// A step parked at `awaiting_approval`: approve or reject it.
    Gate,
    /// A parked fan-out slot whose judge failed: pick a winner.
    JudgeFailed,
    /// A parked fan-out slot with no judge verdict: pick a winner.
    Selection,
    /// `u` clears the item: reopen it, follow its parked run, or resume the run.
    Unblock,
    /// A run parked by an interrupt (the engine's `park_interrupted`: the run `awaiting_approval`
    /// over a `failed` step with no gate outcome and an `interrupted…` note), which no rule above
    /// lists: retry, promote or cancel it from the Runs pane (MOD-69 review H1).
    Interrupted,
    /// An open permission request of a live session.
    Permission,
}

impl WaitingReason {
    /// The overlay's reason column.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::JudgeFailed => "judge failed",
            Self::Selection => "selection",
            Self::Unblock => "unblock",
            Self::Interrupted => "interrupted",
            Self::Permission => "permission",
        }
    }
}

/// One row of the waiting-on-you list: ids, strings and counts only, so `TopBarState` keeps `Eq`
/// (MOD-69 plan D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingRow {
    /// The item, which `Enter` reveals.
    pub item: ItemId,
    /// `item.key`.
    pub item_key: String,
    /// The run the row is about; `None` for a Reopen row.
    pub run: Option<RunId>,
    /// The step `Enter` puts the cursor on; `None` focuses the run (or, with no run, nothing).
    pub step: Option<StepId>,
    /// The step as the Runs pane's slot column reads it, after the phase: `prd 0.1`,
    /// `research 0.1/1`, `research 0.1/j`; empty when `step` is `None`.
    pub step_label: String,
    /// Why it waits.
    pub reason: WaitingReason,
    /// The reason's text: a gate or judge note, the tool, or the Unblock case's sentence.
    pub text: String,
}

/// The waiting-on-you list and the top bar's two counts (MOD-69 plan D5, D6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitingView {
    /// Active runs in scope that own no row. Chat runs count too (review L6): `active_runs`
    /// counts every active run of the scope, and a chat run never owns a row.
    pub working: usize,
    /// Every row, in plan D9's order.
    pub rows: Vec<WaitingRow>,
    /// Whether the open permission requests were listed. `false` offline, where they are not
    /// mirrored (plan D4), and also online when the permission read failed with anything but
    /// `Unreachable` (review L4): [`Self::offline`] tells the two apart.
    pub permissions_known: bool,
    /// The reply was read from the offline mirror (the backend had no writer). Every row is then
    /// read-only (review L3): `Enter` still reveals it, and the Runs pane refuses the answer, as
    /// every write is refused offline. Set by the store worker; [`waiting`] leaves it `false`.
    pub offline: bool,
}

impl WaitingView {
    /// How many rows wait on a person: the top bar's second count.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.rows.len()
    }
}

/// A gate row's text when the step carries no note.
const GATE_TEXT: &str = "gate";
/// A promoted step's gate row text when it carries no note (MOD-69 blueprint E6).
const PROMOTED_TEXT: &str = "promoted to chat";
/// What a JudgeFailed row's text appends to the judge's note: the person's next move (review L5).
const JUDGE_HINT: &str = " - pick a candidate";
/// A Selection row's text.
const SELECTION_TEXT: &str = "awaits selection";
/// A Permission row's text when the request carries no summary.
const PERMISSION_TEXT: &str = "permission";
/// [`UnblockCase::Reopen`]'s row text.
const REOPEN_TEXT: &str = "blocked, no active run: u reopens it";
/// [`UnblockCase::FollowRun`]'s row text.
const FOLLOW_TEXT: &str = "blocked over a parked run: u follows it";
/// [`UnblockCase::Resume`]'s row text.
const RESUME_TEXT: &str = "parked by an interrupted command: u resumes it";
/// An Interrupted row's text when its rest step carries no note (MOD-69 review H1).
const INTERRUPTED_TEXT: &str = "interrupted";

/// The waiting-on-you list over one candidate read (MOD-69 plan D1-D5, D9), with the engine's own
/// guards: [`selectable_slots`] (`verdicts`' own `select` with no live chat; heads feed only
/// approve/accept/open, plan D3) decides a slot's `select`, one [`unblock_case`] per candidate the
/// item's `u`, step status a gate, and a parked run none of them lists is an Interrupted row
/// (review H1). `permissions` is `None` offline or when the permission read failed (review L4).
/// `active` is `Backend::active_runs` over the same scope.
#[must_use]
pub fn waiting(
    scope: &Scope,
    active: usize,
    candidates: &[WaitingCandidate],
    permissions: Option<&[WaitingPermission]>,
) -> WaitingView {
    let project_at = |project: ProjectId| {
        scope
            .project_ids
            .iter()
            .position(|id| *id == project)
            .unwrap_or(usize::MAX)
    };
    let mut keyed: Vec<(RowKey, WaitingRow)> = Vec::new();

    for candidate in candidates {
        let item = &candidate.item;
        let first = keyed.len();
        let row = |run: Option<&Run>,
                   step: Option<&RunStep>,
                   step_label: String,
                   reason: WaitingReason,
                   text: String| {
            let key = RowKey {
                project: project_at(item.project_id),
                key_prefix: item.key_prefix.as_str(),
                key_number: item.key_number,
                item: item.id,
                run_missing: run.is_none(),
                run: run.map(|run| (run.queued_at, run.id)),
                step_missing: step.is_none(),
                step: step.map(|step| (step.position, step.attempt, step.fanout_index)),
                reason,
            };
            let row = WaitingRow {
                item: item.id,
                item_key: item.key.clone(),
                run: run.map(|run| run.id),
                step: step.map(|step| step.id),
                step_label,
                reason,
                text,
            };
            (key, row)
        };
        for (run, steps) in &candidate.runs {
            // Step status decides a gate, not `approve`: a gate parked without its output greys
            // `approve` but still waits on a person (plan D3).
            let parked: Vec<&RunStep> = steps
                .iter()
                .filter(|step| step.status == StepStatus::AwaitingApproval)
                .collect();
            for step in &parked {
                let text = match &step.gate_note {
                    Some(note) if !note.is_empty() => note.clone(),
                    _ if step.promoted_at.is_some() => PROMOTED_TEXT.to_owned(),
                    _ => GATE_TEXT.to_owned(),
                };
                keyed.push(row(
                    Some(run),
                    Some(step),
                    label_of(step, steps),
                    WaitingReason::Gate,
                    text,
                ));
            }
            if run.status != RunStatus::AwaitingApproval || !parked.is_empty() {
                continue;
            }
            // A slot waits on a selection exactly when the pane enables `select` on one of its
            // candidates; a run whose snapshot does not decode has every `select` refused.
            // Review L2: read with no live chat, by design. A chat on the run greys the pane's
            // `select` only while it lasts, and the slot still waits on a person meanwhile.
            for (position, attempt) in selectable_slots(run, steps) {
                let failed_judge = judge_at(steps, position, attempt)
                    .filter(|judge| judge.status == StepStatus::Failed)
                    .and_then(|judge| {
                        judge
                            .gate_note
                            .as_ref()
                            .map(|note| (judge, format!("{note}{JUDGE_HINT}")))
                    });
                if let Some((judge, note)) = failed_judge {
                    keyed.push(row(
                        Some(run),
                        Some(judge),
                        label_of(judge, steps),
                        WaitingReason::JudgeFailed,
                        note,
                    ));
                } else if let Some(first) = group_at(steps, position, attempt).first() {
                    keyed.push(row(
                        Some(run),
                        Some(first),
                        label_of(first, steps),
                        WaitingReason::Selection,
                        SELECTION_TEXT.to_owned(),
                    ));
                }
            }
        }

        let run_of = |id: RunId| {
            candidate
                .runs
                .iter()
                .map(|(run, _)| run)
                .find(|run| run.id == id)
        };
        let unblock = match unblock_case(item, &candidate.runs) {
            Ok(UnblockCase::Reopen) => Some((None, REOPEN_TEXT)),
            Ok(UnblockCase::FollowRun(id)) => run_of(id).map(|run| (Some(run), FOLLOW_TEXT)),
            Ok(UnblockCase::Resume(id)) => run_of(id).map(|run| (Some(run), RESUME_TEXT)),
            Err(_) => None,
        };
        if let Some((run, text)) = unblock {
            keyed.push(row(
                run,
                None,
                String::new(),
                WaitingReason::Unblock,
                text.to_owned(),
            ));
        }

        // Review H1: a parked run no rule above lists (the engine's `park_interrupted`, whose
        // `failed` step has no gate outcome and which `u` refuses) still waits on a person, so it
        // is never counted working. The row sits on the cursor's rest step; with no rest step, or
        // a snapshot that does not decode, on the run alone.
        for (run, steps) in &candidate.runs {
            if run.status != RunStatus::AwaitingApproval
                || keyed[first..]
                    .iter()
                    .any(|(_, row)| row.run == Some(run.id))
            {
                continue;
            }
            let rest = snapshot_of(run)
                .ok()
                .and_then(|snapshot| match cursor(&snapshot, steps) {
                    Cursor::Rest { step, .. } => steps.iter().find(|row| row.id == step),
                    _ => None,
                });
            let text = rest
                .and_then(|step| step.gate_note.clone())
                .filter(|note| !note.is_empty())
                .unwrap_or_else(|| INTERRUPTED_TEXT.to_owned());
            keyed.push(row(
                Some(run),
                rest,
                rest.map_or_else(String::new, |step| label_of(step, steps)),
                WaitingReason::Interrupted,
                text,
            ));
        }
    }

    for p in permissions.unwrap_or_default() {
        let text = p
            .permission
            .summary
            .clone()
            .filter(|summary| !summary.is_empty())
            .unwrap_or_else(|| PERMISSION_TEXT.to_owned());
        let key = RowKey {
            project: project_at(p.project),
            key_prefix: p.key_prefix.as_str(),
            key_number: p.key_number,
            item: p.item,
            run_missing: false,
            run: Some((p.run_queued_at, p.permission.run_id)),
            step_missing: false,
            step: Some((p.step_position, p.step_attempt, p.step_fanout_index)),
            reason: WaitingReason::Permission,
        };
        let row = WaitingRow {
            item: p.item,
            item_key: p.item_key.clone(),
            run: Some(p.permission.run_id),
            step: Some(p.permission.run_step_id),
            step_label: slot_label(
                &p.phase_name,
                p.step_position,
                p.step_attempt,
                p.step_fanout_index,
                p.step_fanned,
            ),
            reason: WaitingReason::Permission,
            text,
        };
        keyed.push((key, row));
    }

    // The row's text is the last tie-break (two permissions on one step), read from the row
    // rather than cloned into the key.
    keyed.sort_by(|(a, a_row), (b, b_row)| a.cmp(b).then_with(|| a_row.text.cmp(&b_row.text)));
    let rows: Vec<WaitingRow> = keyed.into_iter().map(|(_, row)| row).collect();
    // Plan D5, blueprint H-19: a run owning several rows counts once; `saturating_sub` also
    // absorbs the race between `active_runs` and the candidate read.
    let owning = rows
        .iter()
        .filter_map(|row| row.run)
        .collect::<BTreeSet<_>>()
        .len();
    WaitingView {
        working: active.saturating_sub(owning),
        rows,
        permissions_known: permissions.is_some(),
        offline: false,
    }
}

/// A row's place in the list: plan D9 (project position, item key, run creation, step position),
/// then blueprint E8's ties — inside an item, rows with a run before the Reopen row; inside a run,
/// step rows before its Unblock row; then the reason (plan D3 order) and, outside the key, the
/// row's text. Borrowed from the candidate and permission rows, so building a key clones nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RowKey<'a> {
    /// The project's position in `scope.project_ids`; `usize::MAX` when absent.
    project: usize,
    /// Byte order (`str`'s `Ord`), so the order does not depend on a collation.
    key_prefix: &'a str,
    /// By number, so `FEAT-2` sorts before `FEAT-10`.
    key_number: i32,
    item: ItemId,
    /// `false` first: rows with a run before the item's Reopen row.
    run_missing: bool,
    /// `(queued_at, id)` ascending: plan D9's run creation.
    run: Option<(DateTime<Utc>, RunId)>,
    /// `false` first: a run's step rows before its Unblock row.
    step_missing: bool,
    /// `(position, attempt, fanout_index)`.
    step: Option<(i32, i32, i32)>,
    reason: WaitingReason,
}

/// `phase p.a`, then `/i` in a fan-out slot and `/j` for its judge (the Runs pane's slot column,
/// MOD-69 blueprint E7).
fn slot_label(phase: &str, position: i32, attempt: i32, fanout_index: i32, fanned: bool) -> String {
    let at = format!("{phase} {position}.{attempt}");
    match (fanned, fanout_index) {
        (false, _) => at,
        (true, -1) => format!("{at}/j"),
        (true, index) => format!("{at}/{index}"),
    }
}

/// [`slot_label`] of a step among its run's steps: a slot is fanned when a step at the same
/// `(position, attempt)` has a non-zero `fanout_index`.
fn label_of(step: &RunStep, steps: &[RunStep]) -> String {
    let fanned = steps.iter().any(|sibling| {
        sibling.position == step.position
            && sibling.attempt == step.attempt
            && sibling.fanout_index != 0
    });
    slot_label(
        &step.phase_name,
        step.position,
        step.attempt,
        step.fanout_index,
        fanned,
    )
}

/// Blueprint D212 (review H3): `Ok` unless a chat of this process is live on one of `steps` — the
/// steps of one run — and then `ChatLive(Some)` naming it (D185 allows one live chat per process,
/// so there is at most one).
///
/// The one admission for the verbs that would move a run under its chat ([`moves_the_run`]): the
/// worker refuses them with it inside their task, and [`verdicts`] greys the same keys with it
/// (D182, D184). Keyed on the run, not the named step: a group retry retires every member of the
/// slot and a selection resolves it, whichever step the chat is on.
///
/// # Errors
/// [`EngineError::ChatLive`] naming the step being chatted with.
pub(crate) fn chat_free(steps: &[RunStep], live: &LiveChats) -> Result<(), EngineError> {
    match steps.iter().find(|step| live.contains(step.id)) {
        Some(step) => Err(EngineError::ChatLive {
            step: Some(step.id),
        }),
        None => Ok(()),
    }
}

/// D212: the verbs [`chat_free`] admits — approve and reject (`AnswerGate`), retry, select and
/// cancel. A cancel is refused rather than ending the chat: the loop, not this runtime, owns it.
pub(crate) const fn moves_the_run(command: &Command) -> bool {
    matches!(
        command,
        Command::AnswerGate { .. }
            | Command::RetryStep { .. }
            | Command::SelectFanout { .. }
            | Command::CancelRun { .. }
    )
}

/// Every step verdict of a run whose snapshot (or phase) could not be read: that sentence.
fn refused_step(refusal: &str) -> StepActions {
    let refused = || Err(refusal.to_owned());
    StepActions {
        approve: refused(),
        reject: refused(),
        retry: refused(),
        promote: refused(),
        accept: refused(),
        select: refused(),
        open: Err(refusal.to_owned()),
    }
}

/// The step `step` is not a fan-out candidate, so it has nothing to select.
fn not_a_candidate(step: StepId) -> String {
    format!("step {step} is not a fan-out candidate")
}

/// The newest head of `kind` produced by `step` — the engine's `output_of` rule — or the sentence
/// that says there is none.
fn newest_output(heads: &[DocumentHead], kind: &str, step: StepId) -> Result<DocumentId, String> {
    heads
        .iter()
        .filter(|head| head.kind == kind && head.produced_by_step_id == Some(step))
        .max_by_key(|head| head.version)
        .map(|head| head.id)
        .ok_or_else(|| format!("step {step} produced no `{kind}` document"))
}

/// Blueprint D203: the producer of a step's output document, behind the progress sink. Production
/// has none — MOD-11's `document_write` writes documents (R-50) — and a test answers with one.
pub trait StepAuthor: Send + Sync + core::fmt::Debug {
    /// The document `step` produced, if any.
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument>;
}

/// D172, D203: `after_done` publishes `SessionDone` for the item and, in a test, writes the step's
/// output document. Production `author` is `None` (MOD-11 writes documents; R-50).
///
/// MOD-41 plan D7: `S` is the host's store ([`WorkerHost::Store`](htui_core::store::WorkerHost)).
#[derive(Debug, Clone)]
pub struct ProgressSink<S> {
    pub(crate) publisher: Arc<dyn Publish>,
    pub(crate) writer: S,
    pub(crate) author: Option<Arc<dyn StepAuthor>>,
}

impl<S: htui_core::store::WorkerStore> SessionSink for ProgressSink<S> {
    fn started(&self, item: ItemId, run: RunId, _step: StepId) {
        self.publisher.publish(&RunFrame {
            item,
            run: Some(run),
            kind: FrameKind::Changed,
        });
    }

    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        _key: &SessionKey<'_>,
        _done: &DoneEvent,
    ) -> Result<(), StoreError> {
        if let Some(author) = &self.author
            && let Some(document) = author.document(item, step, phase)
        {
            self.writer.write_document(document).await?;
        }
        self.publisher.publish(&RunFrame {
            item,
            run: Some(step.run_id),
            kind: FrameKind::SessionDone { step: step.id },
        });
        Ok(())
    }
}

/// A driver the registry refused: `start` answers that refusal, which the walk fails as a spawn
/// failure under every gate. `DriverError` is `Clone`.
#[derive(Debug)]
pub(crate) struct RefusedDriver(pub(crate) DriverError);

impl AgentDriver for RefusedDriver {
    fn name(&self) -> &str {
        "refused"
    }

    fn caps(&self) -> DriverCaps {
        DriverCaps::default()
    }

    fn start<'a>(
        &'a self,
        _spec: SessionSpec,
        _prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        let refusal = self.0.clone();
        Box::pin(async move { Err(refusal) })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use htui_core::fixtures::{demo_at, demo_data, ids};
    use htui_core::model::{
        GateOutcome, GraphSnapshot, Item, ItemId, PermissionId, PermissionStatus, ProjectId,
        RelaySessionId, Run, RunId, RunMode, RunStatus, RunStep, Scope, Status, StepId,
        StepPermission, StepStatus, WaitingCandidate, WaitingPermission,
    };
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_orch::conformance::Orchestrate as _;
    use htui_orch::fake::{FakeOrchestrator, ScriptedStep};
    use htui_orch::{Clock as _, Command, CommandOutcome, GateAnswer, UnblockCase};

    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{
        Enabled, FOLLOW_TEXT, GATE_TEXT, ItemActions, LiveChats, PERMISSION_TEXT, REOPEN_TEXT,
        RESUME_TEXT, SELECTION_TEXT, WaitingReason, WaitingRow, WaitingView, selectable_slots,
        unblock_case, verdicts, waiting,
    };
    use std::collections::BTreeSet;

    /// [`verdicts`]' `u` for `item`, over the rows [`super::actions`] reads.
    async fn unblock_verdict(store: &MemStore, item: ItemId) -> Enabled {
        let read = "MemStore never fails a read";
        let row = store.item(item).await.expect(read).expect("a seeded item");
        let heads = store.documents(item).await.expect(read);
        let mut runs = Vec::new();
        for summary in store.runs(item).await.expect(read) {
            let run = store
                .run(summary.id)
                .await
                .expect(read)
                .expect("a listed run");
            let steps = store.run_steps(run.id).await.expect(read);
            runs.push((run, steps));
        }
        verdicts(&row, &runs, &heads, &LiveChats::default()).unblock
    }

    /// `FEAT-3` freed of its seeded `RUN_2` and started: the walk parks at `prd`'s gate.
    async fn started(orch: &FakeOrchestrator) -> RunId {
        orch.store()
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock().now())
            .await
            .expect("the seeded run is queued and cancellable");
        let outcome = orch
            .dispatch(Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: None,
            })
            .await
            .expect("the walk starts");
        let CommandOutcome::Started { run, .. } = outcome else {
            panic!("`StartRun` answers `Started`, not {outcome:?}");
        };
        run
    }

    /// MOD-37 R-31 (review L2): `u` is enabled over a rejection a crash left parked (the run
    /// `awaiting_approval`, its latest step `failed` + `rejected`), which `verdicts` tells by
    /// `status::resumable` over the run's steps, while a real gate over the same run is refused.
    #[tokio::test]
    async fn unblock_is_enabled_over_a_crashed_rejection() {
        let orch = FakeOrchestrator::demo();
        let run = started(&orch).await;
        let steps = orch.store().run_steps(run).await.expect("MemStore reads");
        let [prd] = &steps[..] else {
            panic!("the walk parked `prd` alone: {steps:?}");
        };
        assert_eq!(prd.status, StepStatus::AwaitingApproval);
        assert!(
            unblock_verdict(orch.store(), ids::HTUI_FEAT_3)
                .await
                .is_err_and(|why| why.contains("parked at a gate")),
            "a real gate is answered, not unblocked"
        );

        assert!(
            orch.store()
                .answer_gate(
                    prd.id,
                    GateOutcome::Rejected,
                    Some("not like this".to_owned()),
                    orch.clock().now()
                )
                .await
                .expect("MemStore takes the answer"),
            "the rejection's first write landed and its unpark did not"
        );
        assert_eq!(
            orch.store()
                .run(run)
                .await
                .expect("read")
                .map(|row| row.status),
            Some(RunStatus::AwaitingApproval)
        );
        assert_eq!(
            unblock_verdict(orch.store(), ids::HTUI_FEAT_3).await,
            Ok(()),
            "a crashed rejection is resumable"
        );
    }

    /// MOD-37 milestone 2, maintainer-accepted (review L2): a review loop that escalated, once
    /// `u` has followed the run, rests on the rows a crashed rejection leaves, so `u` stays
    /// enabled (it resumes, re-runs the loop and escalates again) rather than naming the gate.
    #[tokio::test]
    async fn unblock_is_enabled_over_a_followed_escalation() {
        let orch = FakeOrchestrator::demo();
        orch.script("review", 1, ScriptedStep::review("approve", "first"));
        orch.script("review", 2, ScriptedStep::review("approve", "second"));
        let run = started(&orch).await;

        // Approve every parked step and reject each review; the second rejection escalates.
        let mut rejections = 0;
        while rejections < 2 {
            let steps = orch.store().run_steps(run).await.expect("MemStore reads");
            let step = steps
                .iter()
                .find(|step| step.status == StepStatus::AwaitingApproval)
                .expect("a parked step");
            let answer = if step.phase_name == "review" {
                rejections += 1;
                GateAnswer::Rejected {
                    note: "no tests".to_owned(),
                }
            } else {
                GateAnswer::Approved
            };
            orch.dispatch(Command::AnswerGate {
                run,
                step: step.id,
                answer,
            })
            .await
            .expect("the gate takes the answer");
        }
        let item = || async {
            orch.store()
                .item(ids::HTUI_FEAT_3)
                .await
                .expect("MemStore reads")
                .expect("seeded")
                .status
        };
        assert_eq!(item().await, Status::Blocked, "the loop escalated");
        assert_eq!(
            unblock_verdict(orch.store(), ids::HTUI_FEAT_3).await,
            Ok(()),
            "`FollowRun`"
        );

        let followed = orch
            .dispatch(Command::Unblock {
                item: ids::HTUI_FEAT_3,
            })
            .await
            .expect("a blocked item follows its parked run");
        assert!(
            matches!(
                followed,
                CommandOutcome::Unblocked {
                    case: UnblockCase::FollowRun(id),
                    rest: None,
                    ..
                } if id == run
            ),
            "{followed:?}"
        );
        assert_eq!(item().await, Status::AwaitingApproval);
        assert_eq!(
            unblock_verdict(orch.store(), ids::HTUI_FEAT_3).await,
            Ok(()),
            "the followed escalation reads as a crashed rejection and is resumed"
        );
    }

    // MOD-69 T2: the waiting-on-you classifier over hand-built rows, no store (blueprint §3.6).

    /// The demo `FEAT` graph's phases by position, as `RUN_1`'s snapshot holds them.
    const PHASES: [&str; 4] = ["prd", "plan", "implement", "review"];

    /// The demo `feature` snapshot (`RUN_1`'s) with phase 0's `fan_out` set: 1 = no slot, 2 = a slot.
    fn snapshot(fan_out: i32) -> Value {
        let mut graph: GraphSnapshot = serde_json::from_value(
            demo_data()
                .runs
                .into_iter()
                .find(|run| run.id == ids::RUN_1)
                .and_then(|run| run.graph_snapshot)
                .expect("RUN_1 carries a snapshot"),
        )
        .expect("the demo snapshot decodes");
        graph.phases[0].fan_out = fan_out;
        serde_json::to_value(graph).expect("it encodes")
    }

    /// The demo `FEAT-2` row as `prefix-number` of `project`, at `status`, with an id of its own.
    fn item(prefix: &str, number: i32, project: ProjectId, status: Status) -> Item {
        let mut item = demo_data()
            .items
            .into_iter()
            .find(|item| item.id == ids::HTUI_FEAT_2)
            .expect("the demo holds FEAT-2");
        item.id = ItemId::new();
        item.project_id = project;
        item.key_prefix = prefix.to_owned();
        item.key_number = number;
        item.key = format!("{prefix}-{number}");
        item.status = status;
        item
    }

    /// The demo `RUN_2` row as run `n` of `item`, queued at day 2, `hour`.
    fn run(n: u128, item: &Item, status: RunStatus, snapshot: Option<Value>, hour: i64) -> Run {
        let mut run = demo_data()
            .runs
            .into_iter()
            .find(|run| run.id == ids::RUN_2)
            .expect("the demo holds RUN_2");
        run.id = RunId::from_uuid(Uuid::from_u128(n));
        run.item_id = Some(item.id);
        run.project_id = item.project_id;
        run.status = status;
        run.graph_snapshot = snapshot;
        run.queued_at = demo_at(2, hour);
        run
    }

    /// A parked run over the `fan_out` snapshot.
    fn parked(n: u128, item: &Item, fan_out: i32, hour: i64) -> Run {
        run(
            n,
            item,
            RunStatus::AwaitingApproval,
            Some(snapshot(fan_out)),
            hour,
        )
    }

    /// The demo `STEP_R2_PRD` row as step `n` of `run` at `(position, attempt, fanout_index)`.
    fn step(
        n: u128,
        run: &Run,
        (position, attempt, fanout): (i32, i32, i32),
        status: StepStatus,
    ) -> RunStep {
        let mut step = demo_data()
            .steps
            .into_iter()
            .find(|step| step.id == ids::STEP_R2_PRD)
            .expect("the demo holds STEP_R2_PRD");
        step.id = StepId::from_uuid(Uuid::from_u128(n));
        step.run_id = run.id;
        step.position = position;
        step.attempt = attempt;
        step.fanout_index = fanout;
        step.phase_name = PHASES[usize::try_from(position).expect("a demo position")].to_owned();
        step.status = status;
        step
    }

    /// `step`, rejected at its gate: `failed` + `rejected`.
    fn rejected(mut step: RunStep) -> RunStep {
        step.gate_outcome = Some(GateOutcome::Rejected);
        step.gate_note = Some("not like this".to_owned());
        step
    }

    fn scope(projects: &[ProjectId]) -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: projects.to_vec(),
        }
    }

    /// An open permission request of `step`, created `created_minute` minutes into day 2.
    fn permission(
        item: &Item,
        run: &Run,
        step: &RunStep,
        summary: Option<&str>,
        created_minute: i64,
    ) -> WaitingPermission {
        WaitingPermission {
            item: item.id,
            project: item.project_id,
            item_key: item.key.clone(),
            key_prefix: item.key_prefix.clone(),
            key_number: item.key_number,
            run_queued_at: run.queued_at,
            step_position: step.position,
            step_attempt: step.attempt,
            step_fanout_index: step.fanout_index,
            step_fanned: step.fanout_index != 0,
            phase_name: step.phase_name.clone(),
            permission: StepPermission {
                id: PermissionId::new(),
                run_id: run.id,
                run_step_id: step.id,
                session: RelaySessionId::new(),
                request_id: format!("req-{created_minute}"),
                tool_call_id: None,
                summary: summary.map(str::to_owned),
                options: vec![],
                status: PermissionStatus::Pending,
                option_id: None,
                answered_by: None,
                answered_box: None,
                created_at: demo_at(2, 0) + Duration::minutes(created_minute),
                answered_at: None,
                resolved_at: None,
            },
        }
    }

    fn candidate(item: &Item, runs: Vec<(Run, Vec<RunStep>)>) -> WaitingCandidate {
        WaitingCandidate {
            item: item.clone(),
            runs,
        }
    }

    /// `verdicts` over the candidate as the list reads it: no heads, no live chat.
    fn verdict(candidate: &WaitingCandidate) -> ItemActions {
        verdicts(&candidate.item, &candidate.runs, &[], &LiveChats::default())
    }

    /// The list of one `htui` candidate, online with no permission, under two active runs.
    fn one(candidate: &WaitingCandidate) -> WaitingView {
        waiting(
            &scope(&[ids::PROJECT_HTUI]),
            2,
            std::slice::from_ref(candidate),
            Some(&[]),
        )
    }

    /// The row the list holds for `candidate` at `run`/`step`.
    fn row(
        candidate: &WaitingCandidate,
        run: Option<RunId>,
        step: Option<StepId>,
        step_label: &str,
        reason: WaitingReason,
        text: &str,
    ) -> WaitingRow {
        WaitingRow {
            item: candidate.item.id,
            item_key: candidate.item.key.clone(),
            run,
            step,
            step_label: step_label.to_owned(),
            reason,
            text: text.to_owned(),
        }
    }

    /// An escalated item: `blocked` over a parked run whose `plan` was rejected.
    fn escalation() -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::Blocked);
        let run = parked(1, &item, 1, 1);
        let steps = vec![
            step(1, &run, (0, 1, 0), StepStatus::Done),
            rejected(step(2, &run, (1, 1, 0), StepStatus::Failed)),
        ];
        candidate(&item, vec![(run, steps)])
    }

    /// A rejection a crash left parked: the item and run `awaiting_approval` over `failed` +
    /// `rejected`.
    fn crashed_rejection() -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::AwaitingApproval);
        let run = parked(1, &item, 1, 1);
        let steps = vec![rejected(step(1, &run, (0, 1, 0), StepStatus::Failed))];
        candidate(&item, vec![(run, steps)])
    }

    /// A `blocked` item with no run.
    fn reopen() -> WaitingCandidate {
        candidate(
            &item("FEAT", 1, ids::PROJECT_HTUI, Status::Blocked),
            Vec::new(),
        )
    }

    /// An item at `status` over a run parked at `prd`'s gate.
    fn over_a_gate(status: Status) -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, status);
        let run = parked(1, &item, 1, 1);
        let steps = vec![step(1, &run, (0, 1, 0), StepStatus::AwaitingApproval)];
        candidate(&item, vec![(run, steps)])
    }

    /// A parked fan-out slot at `prd`: two `done` candidates and, when given, a `failed` judge
    /// with that outcome and note.
    fn slot(judge: Option<(Option<GateOutcome>, &str)>) -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::AwaitingApproval);
        let run = parked(1, &item, 2, 1);
        let mut steps = vec![
            step(1, &run, (0, 1, 0), StepStatus::Done),
            step(2, &run, (0, 1, 1), StepStatus::Done),
        ];
        if let Some((outcome, note)) = judge {
            let mut judge = step(3, &run, (0, 1, -1), StepStatus::Failed);
            judge.gate_outcome = outcome;
            judge.gate_note = Some(note.to_owned());
            steps.push(judge);
        }
        candidate(&item, vec![(run, steps)])
    }

    /// A run the engine's `park_interrupted` left (engine.rs `settle_failed`/`reset_interrupted`):
    /// the item and run `awaiting_approval`, `prd` done at position 0, `plan` at position 1
    /// `failed` with no gate outcome and the note `note`.
    fn interrupted_park(note: Option<&str>) -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::AwaitingApproval);
        let run = parked(1, &item, 1, 1);
        let mut plan = step(2, &run, (1, 1, 0), StepStatus::Failed);
        plan.gate_note = note.map(str::to_owned);
        let steps = vec![step(1, &run, (0, 1, 0), StepStatus::Done), plan];
        candidate(&item, vec![(run, steps)])
    }

    /// A parked run whose snapshot does not decode, over `steps(run)`.
    fn undecodable(steps: impl FnOnce(&Run) -> Vec<RunStep>) -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::Blocked);
        let run = run(
            1,
            &item,
            RunStatus::AwaitingApproval,
            Some(json!({ "v": 999 })),
            1,
        );
        let steps = steps(&run);
        candidate(&item, vec![(run, steps)])
    }

    /// A decided slot at `prd` (winner and loser) under a parked run, with `plan`'s gate parked
    /// after it when `gate`.
    fn resolved_selection(gate: bool) -> WaitingCandidate {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::AwaitingApproval);
        let run = parked(1, &item, 2, 1);
        let mut winner = step(1, &run, (0, 1, 0), StepStatus::Done);
        winner.selected = Some(true);
        let mut loser = step(2, &run, (0, 1, 1), StepStatus::Superseded);
        loser.selected = Some(false);
        let mut steps = vec![winner, loser];
        if gate {
            steps.push(step(3, &run, (1, 1, 0), StepStatus::AwaitingApproval));
        }
        candidate(&item, vec![(run, steps)])
    }

    fn run_of(candidate: &WaitingCandidate) -> &Run {
        &candidate.runs[0].0
    }

    fn step_of(candidate: &WaitingCandidate, index: usize) -> &RunStep {
        &candidate.runs[0].1[index]
    }

    #[test]
    fn a_parked_gate_without_output_is_one_gate_row() {
        let gate = over_a_gate(Status::AwaitingApproval);
        let step = step_of(&gate, 0);

        let view = one(&gate);

        assert_eq!(
            view.rows,
            [row(
                &gate,
                Some(run_of(&gate).id),
                Some(step.id),
                "prd 0.1",
                WaitingReason::Gate,
                GATE_TEXT
            )]
        );
        assert_eq!(view.working, 1, "two active runs, one of them owns a row");
        assert!(
            verdict(&gate).steps[&step.id].approve.is_err(),
            "approve greys without the output, and the gate still waits on a person"
        );
    }

    #[test]
    fn a_judge_failed_slot_is_a_judge_row_not_a_selection_row() {
        let slot = slot(Some((Some(GateOutcome::Rejected), "judge: tie")));
        let judge = step_of(&slot, 2);

        assert_eq!(
            one(&slot).rows,
            [row(
                &slot,
                Some(run_of(&slot).id),
                Some(judge.id),
                "prd 0.1/j",
                WaitingReason::JudgeFailed,
                "judge: tie - pick a candidate"
            )]
        );
    }

    #[test]
    fn an_interrupted_judge_is_a_judge_row() {
        let slot = slot(Some((None, "interrupted")));
        let judge = step_of(&slot, 2);

        assert_eq!(
            one(&slot).rows,
            [row(
                &slot,
                Some(run_of(&slot).id),
                Some(judge.id),
                "prd 0.1/j",
                WaitingReason::JudgeFailed,
                "interrupted - pick a candidate"
            )]
        );
    }

    #[test]
    fn an_unjudged_parked_slot_is_a_selection_row_on_its_first_candidate() {
        let slot = slot(None);
        let first = step_of(&slot, 0);

        assert_eq!(
            one(&slot).rows,
            [row(
                &slot,
                Some(run_of(&slot).id),
                Some(first.id),
                "prd 0.1/0",
                WaitingReason::Selection,
                SELECTION_TEXT
            )]
        );
        let verdict = verdict(&slot);
        assert!(
            slot.runs[0]
                .1
                .iter()
                .any(|step| verdict.steps[&step.id].select.is_ok()),
            "the pane enables `select` on the slot"
        );
    }

    #[test]
    fn a_resolved_selection_is_no_row() {
        let resolved = resolved_selection(true);

        assert_eq!(
            one(&resolved).rows,
            [row(
                &resolved,
                Some(run_of(&resolved).id),
                Some(step_of(&resolved, 2).id),
                "plan 1.1",
                WaitingReason::Gate,
                GATE_TEXT
            )]
        );

        // The run still parked with no step parked, so the slot is read: decided, it is no
        // Selection row, and only the interrupted run's Resume row is left.
        let unparked = resolved_selection(false);
        assert_eq!(
            one(&unparked).rows,
            [row(
                &unparked,
                Some(run_of(&unparked).id),
                None,
                "",
                WaitingReason::Unblock,
                RESUME_TEXT
            )]
        );
    }

    #[test]
    fn an_escalation_is_one_follow_run_row_and_no_gate_row() {
        let escalation = escalation();

        assert_eq!(
            one(&escalation).rows,
            [row(
                &escalation,
                Some(run_of(&escalation).id),
                None,
                "",
                WaitingReason::Unblock,
                FOLLOW_TEXT
            )]
        );
        assert_eq!(verdict(&escalation).unblock, Ok(()));
    }

    #[test]
    fn a_person_blocked_item_over_a_parked_gate_is_a_gate_row_and_an_unblock_row() {
        let blocked = over_a_gate(Status::Blocked);
        let run = run_of(&blocked).id;

        assert_eq!(
            one(&blocked).rows,
            [
                row(
                    &blocked,
                    Some(run),
                    Some(step_of(&blocked, 0).id),
                    "prd 0.1",
                    WaitingReason::Gate,
                    GATE_TEXT
                ),
                row(
                    &blocked,
                    Some(run),
                    None,
                    "",
                    WaitingReason::Unblock,
                    FOLLOW_TEXT
                ),
            ]
        );
    }

    #[test]
    fn an_open_item_under_a_parked_gate_is_a_gate_row_only() {
        let open = over_a_gate(Status::Open);

        assert_eq!(
            one(&open).rows,
            [row(
                &open,
                Some(run_of(&open).id),
                Some(step_of(&open, 0).id),
                "prd 0.1",
                WaitingReason::Gate,
                GATE_TEXT
            )]
        );
        assert!(verdict(&open).unblock.is_err());
    }

    #[test]
    fn a_blocked_item_with_no_run_is_a_reopen_row() {
        let reopen = reopen();

        assert_eq!(
            one(&reopen).rows,
            [row(
                &reopen,
                None,
                None,
                "",
                WaitingReason::Unblock,
                REOPEN_TEXT
            )]
        );
    }

    #[test]
    fn a_crashed_rejection_is_a_resume_row() {
        let crashed = crashed_rejection();

        assert_eq!(
            one(&crashed).rows,
            [row(
                &crashed,
                Some(run_of(&crashed).id),
                None,
                "",
                WaitingReason::Unblock,
                RESUME_TEXT
            )]
        );
    }

    #[test]
    fn a_snapshot_that_does_not_decode_keeps_its_gate_row_and_loses_the_rest() {
        let undecodable =
            undecodable(|run| vec![step(1, run, (0, 1, 0), StepStatus::AwaitingApproval)]);

        assert_eq!(
            one(&undecodable).rows,
            [row(
                &undecodable,
                Some(run_of(&undecodable).id),
                Some(step_of(&undecodable, 0).id),
                "prd 0.1",
                WaitingReason::Gate,
                GATE_TEXT
            )]
        );
        assert!(verdict(&undecodable).unblock.is_err());
    }

    /// MOD-69 review H1: `park_interrupted` leaves the run `awaiting_approval` over a `failed`
    /// step with no gate outcome. No gate, slot or `u` lists it, so the fallback does, on the
    /// cursor's rest step with its note.
    #[test]
    fn an_interrupted_park_is_a_row() {
        let parked = interrupted_park(Some("interrupted"));
        let view = one(&parked);

        assert_eq!(
            view.rows,
            [row(
                &parked,
                Some(run_of(&parked).id),
                Some(step_of(&parked, 1).id),
                "plan 1.1",
                WaitingReason::Interrupted,
                "interrupted"
            )]
        );
        assert_eq!(view.working, 1, "the parked run is not working");
        assert!(
            verdict(&parked).unblock.is_err(),
            "`u` refuses it: the list's row is not an Unblock row"
        );

        let not_reset = interrupted_park(Some("interrupted, tree not reset"));
        assert_eq!(one(&not_reset).rows[0].text, "interrupted, tree not reset");
        let silent = interrupted_park(None);
        assert_eq!(one(&silent).rows[0].text, super::INTERRUPTED_TEXT);
        assert_eq!(WaitingReason::Interrupted.label(), "interrupted");
    }

    /// The fallback over a run whose snapshot does not decode names the run and no step.
    #[test]
    fn an_undecodable_park_without_a_gate_is_an_interrupted_run_row() {
        let undecodable = undecodable(|run| {
            vec![
                step(1, run, (0, 1, 0), StepStatus::Done),
                step(2, run, (1, 1, 0), StepStatus::Failed),
            ]
        });

        assert_eq!(
            one(&undecodable).rows,
            [row(
                &undecodable,
                Some(run_of(&undecodable).id),
                None,
                "",
                WaitingReason::Interrupted,
                super::INTERRUPTED_TEXT
            )]
        );
    }

    /// MOD-69 review H1's invariant over every fixture: an active run parked at
    /// `awaiting_approval` always owns at least one row, so the top bar never counts it working.
    #[test]
    fn every_parked_run_owns_a_row() {
        let mut promoted = over_a_gate(Status::AwaitingApproval);
        promoted.runs[0].1[0].promoted_at = Some(demo_at(2, 3));
        let fixtures = [
            escalation(),
            crashed_rejection(),
            reopen(),
            over_a_gate(Status::AwaitingApproval),
            over_a_gate(Status::Blocked),
            over_a_gate(Status::Open),
            over_a_gate(Status::InProgress),
            slot(None),
            slot(Some((Some(GateOutcome::Rejected), "judge: tie"))),
            slot(Some((None, "interrupted"))),
            resolved_selection(true),
            resolved_selection(false),
            undecodable(|run| vec![step(1, run, (0, 1, 0), StepStatus::AwaitingApproval)]),
            undecodable(|run| vec![step(1, run, (0, 1, 0), StepStatus::Failed)]),
            undecodable(|_| Vec::new()),
            promoted,
            interrupted_park(Some("interrupted")),
            interrupted_park(Some("interrupted, tree not reset")),
            interrupted_park(None),
        ];

        for candidate in &fixtures {
            assert_parked_runs_own_rows(std::slice::from_ref(candidate), &one(candidate));
        }
    }

    /// Review H1's invariant: every active run at `AwaitingApproval` among `candidates` owns at
    /// least one row of `view`. Called by every test that builds its candidates inline as well
    /// (review R2), so no fixture escapes it.
    fn assert_parked_runs_own_rows(candidates: &[WaitingCandidate], view: &WaitingView) {
        for candidate in candidates {
            for (run, _) in &candidate.runs {
                if run.status.is_active() && run.status == RunStatus::AwaitingApproval {
                    assert!(
                        view.rows.iter().any(|row| row.run == Some(run.id)),
                        "{candidate:?} -> {view:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_promoted_step_without_a_note_reads_promoted_to_chat() {
        let mut promoted = over_a_gate(Status::AwaitingApproval);
        promoted.runs[0].1[0].promoted_at = Some(demo_at(2, 3));

        assert_eq!(one(&promoted).rows[0].text, super::PROMOTED_TEXT);

        promoted.runs[0].1[0].gate_note = Some("look at the tests".to_owned());
        assert_eq!(one(&promoted).rows[0].text, "look at the tests");
    }

    #[test]
    fn permissions_are_rows_online_and_unknown_offline() {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::InProgress);
        let run = run(1, &item, RunStatus::Running, Some(snapshot(1)), 1);
        let live = step(1, &run, (0, 1, 0), StepStatus::Running);
        let perms = [
            permission(&item, &run, &live, None, 0),
            permission(&item, &run, &live, Some("edit: src/main.rs"), 1),
        ];
        let scope = scope(&[ids::PROJECT_HTUI]);

        let online = waiting(&scope, 1, &[], Some(&perms));
        let expected = |text: &str| WaitingRow {
            item: item.id,
            item_key: "FEAT-1".to_owned(),
            run: Some(run.id),
            step: Some(live.id),
            step_label: "prd 0.1".to_owned(),
            reason: WaitingReason::Permission,
            text: text.to_owned(),
        };
        assert_eq!(
            online.rows,
            [expected("edit: src/main.rs"), expected(PERMISSION_TEXT)]
        );
        assert!(online.permissions_known);
        assert_eq!(online.working, 0);

        let offline = waiting(&scope, 1, &[], None);
        assert!(offline.rows.is_empty());
        assert!(!offline.permissions_known);
        assert_eq!(offline.working, 1);
    }

    /// Review L1: the Runs pane labels candidate 0 of a fanned slot `/0`, and so does its
    /// permission row; an unfanned step stays bare.
    #[test]
    fn a_permission_on_candidate_zero_of_a_fanned_slot_reads_its_index() {
        let item = item("FEAT", 1, ids::PROJECT_HTUI, Status::InProgress);
        let run = run(1, &item, RunStatus::Running, Some(snapshot(2)), 1);
        let first = step(1, &run, (0, 1, 0), StepStatus::Running);
        let mut fanned = permission(&item, &run, &first, None, 0);
        fanned.step_fanned = true;
        let bare = permission(&item, &run, &first, Some("bare"), 1);
        let scope = scope(&[ids::PROJECT_HTUI]);

        let labels: Vec<String> = waiting(&scope, 1, &[], Some(&[fanned, bare]))
            .rows
            .into_iter()
            .map(|row| format!("{} {}", row.step_label, row.text))
            .collect();
        assert_eq!(labels, ["prd 0.1 bare", "prd 0.1/0 permission"]);
    }

    #[test]
    fn counts_split_working_from_waiting_and_never_count_a_run_twice() {
        let reopen = reopen();
        let blocked = over_a_gate(Status::Blocked);
        let busy = item("FIX", 1, ids::PROJECT_HTUI, Status::InProgress);
        let r2 = run(2, &busy, RunStatus::Running, Some(snapshot(1)), 2);
        let live = step(2, &r2, (0, 1, 0), StepStatus::Running);
        let perms = [permission(&busy, &r2, &live, Some("bash: ls"), 0)];
        let scope = scope(&[ids::PROJECT_HTUI]);
        let candidates = [reopen, blocked];

        let view = waiting(&scope, 5, &candidates, Some(&perms));
        assert_parked_runs_own_rows(&candidates, &view);
        assert_eq!(view.waiting(), 4, "Reopen, gate, FollowRun, permission");
        assert_eq!(view.working, 5 - 2, "R1 owns two rows and counts once");

        assert_eq!(
            waiting(&scope, 1, &candidates, Some(&perms)).working,
            0,
            "saturating"
        );
    }

    #[test]
    fn rows_sort_by_project_key_run_step_then_reason() {
        let agy = item("FEAT", 1, ids::PROJECT_AGY, Status::AwaitingApproval);
        let agy_run = parked(30, &agy, 1, 1);
        let agy_gate = step(30, &agy_run, (0, 1, 0), StepStatus::AwaitingApproval);

        let feat_10 = item("FEAT", 10, ids::PROJECT_HTUI, Status::Blocked);

        let feat_2 = item("FEAT", 2, ids::PROJECT_HTUI, Status::AwaitingApproval);
        // The older run takes the higher id, so a sort by run id alone fails.
        let old = parked(21, &feat_2, 1, 1);
        let old_steps = vec![
            step(20, &old, (0, 1, 0), StepStatus::Done),
            step(21, &old, (1, 1, 0), StepStatus::AwaitingApproval),
        ];
        let new = parked(20, &feat_2, 1, 2);
        let new_gate = step(22, &new, (0, 1, 0), StepStatus::AwaitingApproval);

        let ana = item("ANA", 1, ids::PROJECT_HTUI, Status::InProgress);
        let ana_run = run(10, &ana, RunStatus::Running, Some(snapshot(1)), 3);
        let ana_step = step(10, &ana_run, (0, 1, 0), StepStatus::Running);

        let candidates = [
            candidate(&agy, vec![(agy_run.clone(), vec![agy_gate])]),
            candidate(&feat_10, Vec::new()),
            candidate(
                &feat_2,
                vec![
                    (new.clone(), vec![new_gate.clone()]),
                    (old.clone(), old_steps),
                ],
            ),
        ];
        let perms = [
            permission(&feat_2, &new, &new_gate, Some("edit: a"), 0),
            permission(&ana, &ana_run, &ana_step, Some("edit: b"), 1),
        ];

        // `agy` first in the scope though its id sorts after `htui`'s: a sort by id fails.
        let view = waiting(
            &scope(&[ids::PROJECT_AGY, ids::PROJECT_HTUI]),
            4,
            &candidates,
            Some(&perms),
        );
        assert_parked_runs_own_rows(&candidates, &view);

        let keys: Vec<(ProjectId, &str, Option<RunId>, &str, WaitingReason)> = view
            .rows
            .iter()
            .map(|row| {
                let project = if row.item == agy.id {
                    ids::PROJECT_AGY
                } else {
                    ids::PROJECT_HTUI
                };
                (
                    project,
                    row.item_key.as_str(),
                    row.run,
                    row.step_label.as_str(),
                    row.reason,
                )
            })
            .collect();
        assert_eq!(
            keys,
            [
                (
                    ids::PROJECT_AGY,
                    "FEAT-1",
                    Some(agy_run.id),
                    "prd 0.1",
                    WaitingReason::Gate
                ),
                (
                    ids::PROJECT_HTUI,
                    "ANA-1",
                    Some(ana_run.id),
                    "prd 0.1",
                    WaitingReason::Permission
                ),
                (
                    ids::PROJECT_HTUI,
                    "FEAT-2",
                    Some(old.id),
                    "plan 1.1",
                    WaitingReason::Gate
                ),
                (
                    ids::PROJECT_HTUI,
                    "FEAT-2",
                    Some(new.id),
                    "prd 0.1",
                    WaitingReason::Gate
                ),
                (
                    ids::PROJECT_HTUI,
                    "FEAT-2",
                    Some(new.id),
                    "prd 0.1",
                    WaitingReason::Permission
                ),
                (
                    ids::PROJECT_HTUI,
                    "FEAT-10",
                    None,
                    "",
                    WaitingReason::Unblock
                ),
            ]
        );
    }

    /// Review M3: the list reads a slot's `select` through `selectable_slots`, which shares
    /// `verdicts`' select logic without its deep clones; both agree on every fixture.
    #[test]
    fn selectable_slots_are_verdicts_select() {
        let fixtures = [
            escalation(),
            crashed_rejection(),
            over_a_gate(Status::AwaitingApproval),
            slot(None),
            slot(Some((Some(GateOutcome::Rejected), "judge: tie"))),
            slot(Some((None, "interrupted"))),
            resolved_selection(true),
            resolved_selection(false),
            undecodable(|run| vec![step(1, run, (0, 1, 0), StepStatus::Done)]),
            interrupted_park(Some("interrupted")),
        ];
        for candidate in &fixtures {
            let verdict = verdict(candidate);
            for (run, steps) in &candidate.runs {
                let expected: BTreeSet<(i32, i32)> = steps
                    .iter()
                    .filter(|step| verdict.steps[&step.id].select.is_ok())
                    .map(|step| (step.position, step.attempt))
                    .collect();
                assert_eq!(selectable_slots(run, steps), expected, "{candidate:?}");
            }
        }
        assert_eq!(
            selectable_slots(run_of(&slot(None)), &slot(None).runs[0].1),
            BTreeSet::from([(0, 1)])
        );
    }

    #[test]
    fn unblock_case_is_verdicts_unblock_with_the_case_kept() {
        let escalation = escalation();
        let crashed = crashed_rejection();
        let reopen = reopen();
        let open = over_a_gate(Status::Open);

        for candidate in [&escalation, &crashed, &reopen, &open] {
            assert_eq!(
                unblock_case(&candidate.item, &candidate.runs).map(drop),
                verdict(candidate).unblock,
                "{candidate:?}"
            );
        }
        assert_eq!(
            unblock_case(&escalation.item, &escalation.runs),
            Ok(UnblockCase::FollowRun(run_of(&escalation).id))
        );
        assert_eq!(
            unblock_case(&crashed.item, &crashed.runs),
            Ok(UnblockCase::Resume(run_of(&crashed).id))
        );
        assert_eq!(
            unblock_case(&reopen.item, &reopen.runs),
            Ok(UnblockCase::Reopen)
        );
        assert!(unblock_case(&open.item, &open.runs).is_err());
    }
}
