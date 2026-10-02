//! The request and reply shapes the views speak, and every verdict for an item (MOD-4 D182,
//! D184), with the progress sink every engine of the runtime writes through (MOD-41 plan D6).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use htui_agent::driver::{AgentDriver, AgentSession, DriverCaps, DriverFuture, SessionSpec};
use htui_agent::error::DriverError;
use htui_agent::event::DoneEvent;
use htui_core::model::{
    DocumentHead, DocumentId, Item, ItemId, NewDocument, Run, RunId, RunStep, SnapshotPhase, StepId,
};
use htui_core::store::{Result as StoreResult, StoreError};
use htui_orch::command::{answer_gate_enabled, cancel_enabled, select_enabled};
use htui_orch::status::group_at;
use htui_orch::{
    Command, CommandOutcome, Cursor, EngineError, GateAnswer, Rest, SessionKey, SessionSink,
    accept_enabled, cleanup_enabled, close_out_enabled, cursor, phase_at, promote_enabled,
    retry_admitted, snapshot_of, start_enabled, unblock_enabled,
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

    let mut active: Vec<(Run, Cursor)> = Vec::new();
    let mut unblock_refusal = None;
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
                if run.status.is_active() {
                    unblock_refusal.get_or_insert_with(|| refusal.clone());
                }
                for step in steps {
                    actions.steps.insert(step.id, refused_step(&refusal));
                }
                continue;
            }
        };
        if run.status.is_active() {
            active.push((run.clone(), cursor(&snapshot, steps)));
        }
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
            let select = chatting.clone().and_then(|()| {
                if step.fanout_index >= 0 && phase.fan_out > 1 {
                    select_enabled(
                        run,
                        &group_at(steps, step.position, step.attempt),
                        step.id,
                        step.position,
                        step.attempt,
                    )
                    .map_err(sentence)
                } else {
                    Err(not_a_candidate(step.id))
                }
            });
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
    actions.unblock = match unblock_refusal {
        Some(refusal) => Err(refusal),
        None => unblock_enabled(item, &active).map(drop).map_err(sentence),
    };
    actions
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
