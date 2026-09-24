//! The orchestrator, driven from the store worker loop (MOD-4 milestone 6, plan D153).
//!
//! `htui-orch` never names `htui-store` (ANA-2 invariant 10), so everything that joins the two
//! lives here:
//!
//! - [`BackendGraphs`], the graph source over a [`Backend`] (D155);
//! - the request and reply shapes the views speak — [`OrchRequest`], [`OrchReply`], [`RunFrame`],
//!   [`ItemActions`] — and [`actions`], every verdict from the engine's own admission functions
//!   (D182, D184);
//! - [`RunRuntime`], which lives in the store worker's `select!` beside `AgentRuntime` and runs
//!   every command on a task of its own that answers its request once, at the request's `seq`
//!   (`R-NF-3`, R-41). One [`RunLocks`] entry per run serialises a run's commands, walks and
//!   recoveries (R-27, D157); `CancelRun` and `PromoteStep` preempt a live walk through its
//!   cancellation token and give its lease and guards back (D187, D188); the sweep runs at start,
//!   at every `Online` and on a ticker, fenced by the same locks (D158, D189, D190); every task is
//!   supervised, so a panicked walk is adopted by the next sweep (R-12) and a refused claim is
//!   retried once a walk rests (M5 D84). Progress reaches the Runs pane as `RunStream` frames at
//!   each subscriber's own `seq` (D172, blueprint §0a point 3).
//!
//! Off the server every command is refused with MOD-25's sentence and nothing is spawned (D174).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::{AgentDriver, AgentSession, DriverCaps, DriverFuture, SessionSpec};
use htui_agent::error::DriverError;
use htui_agent::event::DoneEvent;
use htui_agent::registry::DriverFactory;
use htui_core::model::{
    Agent, AgentBox, AgentId, AgentSummary, BoxId, BoxProfile, DocumentHead, DocumentId, Item,
    ItemId, NewDocument, PhaseAgent, PhaseId, ProjectId, PromptTemplate, RepoId, ResolvedGraph,
    Run, RunId, RunStatus, RunStep, SnapshotCandidate, SnapshotPhase, StepId, UserId,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{ReadStore as _, Result as StoreResult, StoreError, WriteStore as _};
use htui_orch::command::{answer_gate_enabled, cancel_enabled, select_enabled};
use htui_orch::status::group_at;
use htui_orch::{
    Adopted, Clock, Command, CommandOutcome, Cursor, DeadWalks, DriverFor, Engine, EngineError,
    EngineParts, FirstCandidate, GateAnswer, GixIsolator, GraphSource, Isolator, IsolatorConfig,
    LeaseTimes, Next, Opening, OpeningPath, RepoCheckout, Rest, Resume, RunFence, SessionKey,
    SessionSink, ShellVerifier, SystemClock, UnblockCase, Verifier, accept_enabled,
    cleanup_enabled, close_out_enabled, cursor, phase_at, promote_enabled, retry_admitted,
    snapshot_of, start_enabled, unblock_enabled,
};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer, identity};
use serde_json::Value;
use tokio::sync::{OwnedMutexGuard, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent_worker::ReplyAddr;
use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, Seq, StoreReply, StoreRequest};

/// Blueprint D209: the [`StoreRequest::name`](crate::store_worker::StoreRequest::name) of each
/// [`OrchRequest`], in [`OrchRequest`]'s order — the nine commands of [`Command`], then the
/// close-out preview and the cleanup retry. The status line reads `retry_step: …`, and the Runs
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
/// # Errors
/// The store's own read failures, and [`StoreError::NotFound`] for an item a reachable server does
/// not hold.
pub async fn actions(
    backend: &Backend,
    item: ItemId,
    live: &LiveChats,
) -> StoreResult<ItemActions> {
    let online = backend.writer().is_some();
    let row = backend.item(item).await?;
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
    let heads = backend.documents(item).await?;
    let mut runs = Vec::new();
    for summary in backend.runs(item).await? {
        let Some(run) = backend.run(summary.id).await? else {
            continue;
        };
        let steps = backend.run_steps(run.id).await?;
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
fn unreachable_actions(item: ItemId, key: String) -> ItemActions {
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
        .map_err(sentence),
        runs: BTreeMap::new(),
        steps: BTreeMap::new(),
    };

    let mut active: Vec<(Run, Cursor)> = Vec::new();
    let mut unblock_refusal = None;
    for (run, steps) in runs {
        actions.runs.insert(
            run.id,
            RunActions {
                cancel: cancel_enabled(run).map_err(sentence),
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
            let select = if step.fanout_index >= 0 && phase.fan_out > 1 {
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
            };
            actions.steps.insert(
                step.id,
                StepActions {
                    approve: answer_gate_enabled(step, &phase, has_output, &GateAnswer::Approved)
                        .map_err(sentence),
                    reject: answer_gate_enabled(
                        step,
                        &phase,
                        has_output,
                        &GateAnswer::Rejected {
                            note: String::new(),
                        },
                    )
                    .map_err(sentence),
                    retry: retry_admitted(run, item.status, steps, step, &phase).map_err(sentence),
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

// ---------------------------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------------------------

/// Blueprint D202 (R-39): a `StartRun` found the repo map changed while a walk of this process
/// is live, so the isolator cannot be rebuilt under it.
pub const REPOS_MOVED: &str =
    "a repo was added or moved since the first run; wait for the live runs to rest (R-39)";

/// The `copy_max_total_bytes` a box with no `app_setting` for it copies up to: 20 GiB.
const DEFAULT_COPY_MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// What `RunRuntime::serve` (and the runtime's event channel) decided about one request.
#[derive(Debug)]
pub enum RunServed {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// A task this runtime owns answers the request, exactly once.
    Deferred,
    /// D165/D181: a promotion's engine writes are done; the loop hands `promoted` to
    /// `AgentRuntime::attach_promoted` (T7) and answers at `addr`.
    Attach {
        /// The promotion request's address.
        addr: ReplyAddr,
        /// What the chat binds to.
        promoted: Box<Promoted>,
    },
}

/// A promoted step, as the Chat tab's runtime binds to it (D165, D191).
#[derive(Debug, Clone)]
pub struct Promoted {
    /// The step's run.
    pub run: RunId,
    /// The promoted step.
    pub step: StepId,
    /// `run.project_id`.
    pub project: ProjectId,
    /// How its chat opens.
    pub opening: Opening,
}

/// Blueprint D203: the producer of a step's output document, behind the progress sink. Production
/// has none — MOD-11's `document_write` writes documents (R-50) — and a test answers with one.
pub trait StepAuthor: Send + Sync + core::fmt::Debug {
    /// The document `step` produced, if any.
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument>;
}

/// D172, D203: `after_done` publishes `SessionDone` for the item and, in a test, writes the step's
/// output document. Production `author` is `None` (MOD-11 writes documents; R-50).
#[derive(Debug, Clone)]
pub struct ProgressSink {
    publisher: Publisher,
    writer: Writer,
    author: Option<Arc<dyn StepAuthor>>,
}

impl SessionSink for ProgressSink {
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
struct RefusedDriver(DriverError);

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

/// Plan D172, blueprint §0a point 3: who is subscribed to which item's runs, and at which `seq`.
///
/// One subscription per origin: a later `RunStream` from the same origin replaces the earlier one,
/// whose `seq` `App::latest` already treats as stale. Every frame for `item` goes to each
/// subscriber of it at **its** subscription's `seq`, the only one `App::is_fresh` passes.
#[derive(Debug, Clone, Default)]
struct Publisher(Arc<StdMutex<Subscribers>>);

#[derive(Debug, Default)]
struct Subscribers {
    subs: HashMap<Origin, Subscription>,
    replies: Option<mpsc::UnboundedSender<ReplyEnvelope>>,
}

#[derive(Debug, Clone, Copy)]
struct Subscription {
    seq: Seq,
    item: ItemId,
}

impl Publisher {
    fn lock(&self) -> std::sync::MutexGuard<'_, Subscribers> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The channel frames go out on: the loop's reply sender.
    fn wire(&self, replies: &mpsc::UnboundedSender<ReplyEnvelope>) {
        let mut state = self.lock();
        if state
            .replies
            .as_ref()
            .is_none_or(mpsc::UnboundedSender::is_closed)
        {
            state.replies = Some(replies.clone());
        }
    }

    /// `origin` now follows `item`, at `seq`.
    fn subscribe(&self, origin: Origin, seq: Seq, item: ItemId) {
        self.lock().subs.insert(origin, Subscription { seq, item });
    }

    /// One frame to every subscriber of its item.
    fn publish(&self, frame: &RunFrame) {
        let state = self.lock();
        let Some(replies) = state.replies.as_ref() else {
            return;
        };
        for (origin, sub) in &state.subs {
            if sub.item != frame.item {
                continue;
            }
            let _ = replies.send(ReplyEnvelope {
                seq: sub.seq,
                origin: origin.clone(),
                reply: StoreReply::RunStream(frame.clone()),
            });
        }
    }
}

/// The isolator and verifier every engine of this process borrows (D156): injected, or built at
/// the first command from the box's repos.
enum Parts {
    Injected {
        isolator: Arc<dyn Isolator>,
        verifier: Arc<dyn Verifier>,
    },
    Production {
        scratch_root: Option<PathBuf>,
        built: tokio::sync::Mutex<Built>,
    },
}

impl core::fmt::Debug for Parts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Injected { isolator, .. } => f
                .debug_struct("Injected")
                .field("isolator", isolator)
                .finish_non_exhaustive(),
            Self::Production { scratch_root, .. } => f
                .debug_struct("Production")
                .field("scratch_root", scratch_root)
                .finish_non_exhaustive(),
        }
    }
}

/// What the production parts were built from, and the parts.
#[derive(Default)]
struct Built {
    repos: Option<BTreeMap<RepoId, RepoCheckout>>,
    isolator: Option<Arc<dyn Isolator>>,
    verifier: Option<Arc<dyn Verifier>>,
}

/// Everything the runtime's tasks share.
struct Shared {
    parts: Parts,
    drivers: Arc<DriverFactory>,
    clock: Arc<dyn Clock>,
    author: Option<Arc<dyn StepAuthor>>,
    owner: Uuid,
    dead_walks: Arc<DeadWalks>,
    publisher: Publisher,
    events: mpsc::UnboundedSender<RunServed>,
    tasks: StdMutex<Vec<Tracked>>,
    isolator_builds: AtomicUsize,
    locks: RunLocks,
    walks: Walks,
    /// M5 D84: this process's runs a claim refused, by `queued_at`.
    queued: StdMutex<BTreeSet<(DateTime<Utc>, RunId)>>,
    /// D190: the sweep period in milliseconds, the lease TTL until a test fixes it.
    sweep_every: AtomicU64,
    /// Whether [`RunRuntime::with_sweep_every`] fixed the period.
    sweep_fixed: bool,
    /// D190: one sweep at a time.
    sweeping: AtomicBool,
}

/// One task of the runtime, with the run it works on once it knows it.
#[derive(Debug)]
struct Tracked {
    tag: Arc<Tag>,
    handle: JoinHandle<()>,
}

/// The run (and item) a task works on, set as soon as the task knows them.
#[derive(Debug, Default)]
struct Tag {
    run: OnceLock<RunId>,
    item: OnceLock<ItemId>,
}

impl Shared {
    /// M5 D84: a run a claim refused waits in `queued_at` order.
    fn queue(&self, queued_at: DateTime<Utc>, run: RunId) {
        self.queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((queued_at, run));
    }

    fn track(&self, tag: Arc<Tag>, handle: JoinHandle<()>) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Tracked { tag, handle });
    }

    /// The process's isolator and verifier (D156, D202). A `StartRun` re-reads the repo map and
    /// rebuilds the production isolator when it moved and no walk of this process is live, and is
    /// refused with [`REPOS_MOVED`] when one is (R-39).
    async fn singletons(
        &self,
        backend: &Backend,
        writer: &Writer,
        start_run: bool,
    ) -> Result<(Arc<dyn Isolator>, Arc<dyn Verifier>), String> {
        let (scratch_root, built) = match &self.parts {
            Parts::Injected { isolator, verifier } => {
                return Ok((Arc::clone(isolator), Arc::clone(verifier)));
            }
            Parts::Production {
                scratch_root,
                built,
            } => (scratch_root, built),
        };
        let mut built = built.lock().await;
        if !start_run && let (Some(isolator), Some(verifier)) = (&built.isolator, &built.verifier) {
            return Ok((Arc::clone(isolator), Arc::clone(verifier)));
        }
        let box_id = registered_box(backend)
            .await
            .map_err(|err| err.to_string())?;
        let repos = repo_map(backend, writer, box_id)
            .await
            .map_err(|err| err.to_string())?;
        let rebuild = built.repos.as_ref() != Some(&repos);
        if rebuild {
            if built.isolator.is_some() && self.any_live() {
                return Err(REPOS_MOVED.to_owned());
            }
            let app = backend
                .app_settings()
                .await
                .map_err(|err| err.to_string())?;
            let scratch_root = match scratch_root {
                Some(root) => root.clone(),
                None => identity::config_root()
                    .map_err(|err| err.to_string())?
                    .join("trees"),
            };
            let isolator = GixIsolator::new(IsolatorConfig {
                repos: repos.clone(),
                scratch_root,
                copy_exclude: Vec::new(),
                copy_max_total_bytes: app
                    .get("copy_max_total_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_COPY_MAX_TOTAL_BYTES),
                box_id,
            })
            .map_err(|err| err.to_string())?;
            built.isolator = Some(Arc::new(isolator));
            built.repos = Some(repos);
            self.isolator_builds.fetch_add(1, Ordering::SeqCst);
        }
        if built.verifier.is_none() {
            let limits = command_limits(backend, box_id).await;
            built.verifier = Some(Arc::new(ShellVerifier::new(
                &limits,
                Arc::new(MinimalScrubber::new(std::iter::empty::<String>())),
                Arc::clone(&self.clock),
            )));
        }
        match (&built.isolator, &built.verifier) {
            (Some(isolator), Some(verifier)) => Ok((Arc::clone(isolator), Arc::clone(verifier))),
            _ => Err("the run runtime has no isolator".to_owned()),
        }
    }

    /// Whether a walk of this process is live.
    fn any_live(&self) -> bool {
        self.walks.any_live()
    }

    /// The run's lock, unless the task's token is cancelled first (H-6): a task cancelled while
    /// it waits walks nothing.
    async fn lock_unless_cancelled(
        &self,
        run: RunId,
        walk: &WalkToken,
    ) -> Option<OwnedMutexGuard<()>> {
        tokio::select! {
            biased;
            () = walk.token.cancelled() => None,
            guard = self.locks.lock(run) => Some(guard),
        }
    }
}

/// R-27: one async mutex per run, minted on first use. Held by every command, resume, claim and
/// sweep-driven recovery of that run for its whole duration (plan D157).
#[derive(Debug, Clone, Default)]
pub struct RunLocks(Arc<StdMutex<HashMap<RunId, Arc<tokio::sync::Mutex<()>>>>>);

impl RunLocks {
    fn entry(&self, run: RunId) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(run)
                .or_default(),
        )
    }

    /// Waits for `run`'s lock.
    pub async fn lock(&self, run: RunId) -> OwnedMutexGuard<()> {
        self.entry(run).lock_owned().await
    }

    /// `run`'s lock when nobody holds it.
    #[must_use]
    pub fn try_lock(&self, run: RunId) -> Option<OwnedMutexGuard<()>> {
        self.entry(run).try_lock_owned().ok()
    }
}

/// Blueprint D189: a sweep leaves a run a command of this process holds alone.
impl RunFence for RunLocks {
    type Guard = OwnedMutexGuard<()>;

    fn hold(&self, run: RunId) -> Option<Self::Guard> {
        self.try_lock(run)
    }
}

/// D187: one parent token per run; every task of the run works under a child. The std mutex is
/// never held across an `.await`.
#[derive(Debug, Clone, Default)]
struct Walks(Arc<StdMutex<HashMap<RunId, Parent>>>);

/// A run's parent token and how many tasks work under it.
#[derive(Debug, Clone)]
struct Parent {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
}

/// One task's child token; dropping it is the task no longer working on the run.
#[derive(Debug)]
struct WalkToken {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
}

impl Drop for WalkToken {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Walks {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RunId, Parent>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A child of `run`'s parent, minting the parent on first use.
    fn child(&self, run: RunId) -> WalkToken {
        let mut walks = self.lock();
        let parent = walks.entry(run).or_insert_with(|| Parent {
            token: CancellationToken::new(),
            live: Arc::new(AtomicUsize::new(0)),
        });
        parent.live.fetch_add(1, Ordering::SeqCst);
        WalkToken {
            token: parent.token.child_token(),
            live: Arc::clone(&parent.live),
        }
    }

    /// Whether a task of this process works on `run`.
    fn is_live(&self, run: RunId) -> bool {
        self.lock()
            .get(&run)
            .is_some_and(|parent| parent.live.load(Ordering::SeqCst) > 0)
    }

    /// Whether any task of this process works on any run.
    fn any_live(&self) -> bool {
        self.lock()
            .values()
            .any(|parent| parent.live.load(Ordering::SeqCst) > 0)
    }

    /// D187: removes and cancels `run`'s parent, so every task under it stops and the preempting
    /// task's own child comes from a fresh one. Whether there was one.
    fn preempt(&self, run: RunId) -> bool {
        let parent = self.lock().remove(&run);
        parent.is_some_and(|parent| {
            parent.token.cancel();
            true
        })
    }

    /// Every run's parent, cancelled: the UI is gone.
    fn cancel_all(&self) {
        for (_, parent) in self.lock().drain() {
            parent.token.cancel();
        }
    }
}

/// This box's id, or the refusal that says it has never been registered.
async fn registered_box(backend: &Backend) -> StoreResult<BoxId> {
    Ok(backend
        .box_info()
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "box",
            id: "this box is not registered".to_owned(),
        })?
        .box_id)
}

/// Blueprint D202: every repo of every project of every workspace, joined on this box's checkout
/// paths. A repo with no checkout here is not isolatable here and is left out.
async fn repo_map(
    backend: &Backend,
    writer: &Writer,
    box_id: BoxId,
) -> StoreResult<BTreeMap<RepoId, RepoCheckout>> {
    let paths: BTreeMap<RepoId, String> = backend
        .repo_paths(box_id)
        .await?
        .into_iter()
        .map(|path| (path.repo_id, path.local_path))
        .collect();
    let mut projects = BTreeSet::new();
    for workspace in backend.workspaces().await? {
        projects.extend(workspace.projects.iter().map(|project| project.project_id));
    }
    let mut repos = BTreeMap::new();
    for project in projects {
        for repo in writer.repos(project).await? {
            if let Some(path) = paths.get(&repo.id) {
                repos.insert(
                    repo.id,
                    RepoCheckout {
                        name: repo.name,
                        local_path: PathBuf::from(path),
                        is_primary: repo.is_primary,
                    },
                );
            }
        }
    }
    Ok(repos)
}

/// The box row's `settings.command_limits`, else `{"verify": 1}` (D156).
async fn command_limits(backend: &Backend, box_id: BoxId) -> BTreeMap<String, u32> {
    backend
        .box_row(box_id)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.settings.get("command_limits").cloned())
        .and_then(|limits| serde_json::from_value(limits).ok())
        .unwrap_or_else(|| BTreeMap::from([("verify".to_owned(), 1)]))
}

/// The engine every task builds, per step of work, over [`Kit`]'s parts.
type WorkerEngine<'a> = Engine<
    'a,
    Writer,
    BackendGraphs,
    dyn Isolator,
    dyn Verifier,
    dyn Clock,
    FirstCandidate,
    ProgressSink,
>;

/// Everything one task's engine borrows, owned (D156): the writer, the graph source, the two
/// singletons, the clock, the sink, the identities, and the agent registry read once per task.
struct Kit {
    writer: Writer,
    graphs: BackendGraphs,
    isolator: Arc<dyn Isolator>,
    verifier: Arc<dyn Verifier>,
    clock: Arc<dyn Clock>,
    sink: ProgressSink,
    scrubber: MinimalScrubber,
    app: BTreeMap<String, Value>,
    box_profile: BoxProfile,
    box_id: BoxId,
    user: UserId,
    owner: Uuid,
    dead_walks: Arc<DeadWalks>,
    agents: HashMap<AgentId, AgentSummary>,
    drivers: Arc<DriverFactory>,
}

impl Kit {
    /// Reads the parts. `start_run` asks the singletons to re-check the repo map (D202).
    async fn read(shared: &Shared, backend: &Backend, start_run: bool) -> Result<Self, String> {
        let writer = backend
            .writer()
            .ok_or_else(|| DATABASE_UNREACHABLE.to_owned())?;
        let (isolator, verifier) = shared.singletons(backend, &writer, start_run).await?;
        let sentence = |err: StoreError| err.to_string();
        let box_id = registered_box(backend).await.map_err(sentence)?;
        let user = backend.this_user().await.map_err(sentence)?;
        let app = backend.app_settings().await.map_err(sentence)?;
        let box_profile = backend
            .box_profile(box_id)
            .await
            .map_err(sentence)?
            .ok_or_else(|| "this box has no profile row".to_owned())?;
        let agents = backend
            .agents()
            .await
            .map_err(sentence)?
            .into_iter()
            .map(|summary| (summary.agent.id, summary))
            .collect();
        Ok(Self {
            sink: ProgressSink {
                publisher: shared.publisher.clone(),
                writer: writer.clone(),
                author: shared.author.clone(),
            },
            writer,
            graphs: BackendGraphs(backend.clone()),
            isolator,
            verifier,
            clock: Arc::clone(&shared.clock),
            scrubber: MinimalScrubber::new(std::iter::empty::<String>()),
            app,
            box_profile,
            box_id,
            user,
            owner: shared.owner,
            dead_walks: Arc::clone(&shared.dead_walks),
            agents,
            drivers: Arc::clone(&shared.drivers),
        })
    }

    /// The driver for one candidate: the registry row's, else the refusal (D156).
    fn driver(&self, candidate: &SnapshotCandidate) -> Box<dyn AgentDriver> {
        let Some(summary) = self.agents.get(&candidate.agent_id) else {
            return Box::new(RefusedDriver(DriverError::Transport(format!(
                "agent {} is not in the registry",
                candidate.agent_id
            ))));
        };
        match self
            .drivers
            .driver_for(&summary.agent, summary.on_box.as_ref())
        {
            Ok(driver) => driver,
            Err(refusal) => Box::new(RefusedDriver(refusal)),
        }
    }

    /// One engine over these parts.
    fn engine<'a>(&'a self, driver: DriverFor<'a>) -> WorkerEngine<'a> {
        Engine::new(EngineParts {
            store: &self.writer,
            graphs: &self.graphs,
            isolator: &*self.isolator,
            verifier: &*self.verifier,
            clock: &*self.clock,
            selector: &FirstCandidate,
            sink: &self.sink,
            driver,
            scrubber: &self.scrubber,
            app: self.app.clone(),
            box_profile: self.box_profile.clone(),
            box_id: self.box_id,
            owner: self.owner,
            dead_walks: &self.dead_walks,
            user: self.user,
        })
    }
}

/// What a command preempted by a later one (D187), or cancelled while it waited for the run's
/// lock (H-6), is answered with.
pub const PREEMPTED: &str = "the walk was stopped by a later command on its run";

/// Blueprint §8.6: an `Unblock` whose case moved while it waited for the run's lock.
pub const UNBLOCK_MOVED: &str = "the item changed; press `u` again";

/// D158: what a task that panicked is answered and published with.
pub const WALK_PANICKED: &str = "the walk task panicked; the next sweep adopts it";

/// A period as the millisecond count the runtime stores.
fn millis(every: Duration) -> u64 {
    u64::try_from(every.as_millis()).unwrap_or(u64::MAX).max(1)
}

/// The sweep period a runtime starts with: `LeaseTimes::from_app` over no settings, 120 s (D190).
fn lease_period(app: &BTreeMap<String, Value>) -> Duration {
    LeaseTimes::from_app(app)
        .ttl
        .to_std()
        .unwrap_or(Duration::from_secs(120))
}

/// The orchestrator's runtime, inside the store worker's loop beside `AgentRuntime` (plan D153).
///
/// Every `Orch` command runs on a task this runtime owns and answers its request once, at the
/// request's `seq` (`R-NF-3`, R-41); the loop never awaits a walk. A run's commands are serialised
/// by [`RunLocks`] (R-27), and `CancelRun` and `PromoteStep` preempt a live walk through its token
/// (D157, D187).
pub struct RunRuntime {
    shared: Arc<Shared>,
    events: Option<mpsc::UnboundedReceiver<RunServed>>,
}

impl core::fmt::Debug for RunRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RunRuntime")
            .field("parts", &self.shared.parts)
            .field("adapters", &self.shared.drivers.adapter_ids())
            .field("owner", &self.shared.owner)
            .field("tasks", &self.tasks_len())
            .finish_non_exhaustive()
    }
}

impl RunRuntime {
    /// A runtime over this transport registry, whose isolator and verifier are the production
    /// ones, built at the first command (D156).
    #[must_use]
    pub fn new(drivers: DriverFactory) -> Self {
        Self::assemble(
            Parts::Production {
                scratch_root: None,
                built: tokio::sync::Mutex::default(),
            },
            drivers,
        )
    }

    /// The production runtime: [`DriverFactory::production`] and the production parts.
    #[must_use]
    pub fn production() -> Self {
        Self::new(DriverFactory::production())
    }

    /// A runtime over injected parts, for tests.
    #[must_use]
    pub fn with_parts(
        isolator: Arc<dyn Isolator>,
        verifier: Arc<dyn Verifier>,
        drivers: DriverFactory,
    ) -> Self {
        Self::assemble(Parts::Injected { isolator, verifier }, drivers)
    }

    fn assemble(parts: Parts, drivers: DriverFactory) -> Self {
        let (events, receiver) = mpsc::unbounded_channel();
        Self {
            shared: Arc::new(Shared {
                parts,
                drivers: Arc::new(drivers),
                clock: Arc::new(SystemClock),
                author: None,
                owner: Uuid::now_v7(),
                dead_walks: Arc::new(DeadWalks::new()),
                publisher: Publisher::default(),
                events,
                tasks: StdMutex::default(),
                isolator_builds: AtomicUsize::new(0),
                locks: RunLocks::default(),
                walks: Walks::default(),
                queued: StdMutex::default(),
                sweep_every: AtomicU64::new(millis(lease_period(&BTreeMap::new()))),
                sweep_fixed: false,
                sweeping: AtomicBool::new(false),
            }),
            events: Some(receiver),
        }
    }

    /// D190: a runtime that sweeps every `every`, whatever `lease_ttl_seconds` says.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_sweep_every(mut self, every: Duration) -> Self {
        let shared = self.configure();
        shared.sweep_every = AtomicU64::new(millis(every));
        shared.sweep_fixed = true;
        self
    }

    /// D190: how often the loop's ticker asks for a sweep.
    #[must_use]
    pub fn sweep_every(&self) -> Duration {
        Duration::from_millis(self.shared.sweep_every.load(Ordering::SeqCst))
    }

    /// D158, D189, D190: one recovery sweep on a task of its own. Returns at once: the tick is
    /// skipped while a sweep is still running, and nothing happens without a server.
    pub fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>) {
        if backend.writer().is_none() {
            return;
        }
        if self
            .shared
            .sweeping
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.shared.publisher.wire(replies);
        let ctx = TaskCtx {
            shared: Arc::clone(&self.shared),
            backend: backend.clone(),
            replies: replies.clone(),
            addr: None,
            name: "sweep",
            tag: Arc::default(),
        };
        let shared = Arc::clone(&self.shared);
        let tag = Arc::clone(&ctx.tag);
        let handle = tokio::spawn(async move {
            /// Frees the one-sweep-at-a-time claim however the sweep ends.
            struct Swept(Arc<Shared>);
            impl Drop for Swept {
                fn drop(&mut self) {
                    self.0.sweeping.store(false, Ordering::SeqCst);
                }
            }
            let _swept = Swept(Arc::clone(&ctx.shared));
            sweep_once(ctx).await;
        });
        shared.track(tag, handle);
    }

    /// The shared state, while nothing else holds it: configuration happens before the first
    /// request.
    fn configure(&mut self) -> &mut Shared {
        Arc::get_mut(&mut self.shared).expect("a run runtime is configured before it serves")
    }

    /// A runtime whose engines read this clock; tests use a tokio-time one (H-2).
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.configure().clock = clock;
        self
    }

    /// D203: a runtime whose progress sink writes each step's output document through `author`.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_author(mut self, author: Arc<dyn StepAuthor>) -> Self {
        self.configure().author = Some(author);
        self
    }

    /// D202: the production isolator's scratch root, instead of `identity::config_root()/trees`.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_scratch_root(mut self, root: PathBuf) -> Self {
        if let Parts::Production { scratch_root, .. } = &mut self.configure().parts {
            *scratch_root = Some(root);
        }
        self
    }

    /// D181: the receiver of the runtime's events — today only `RunServed::Attach` — which the
    /// loop owns. Called once, before the loop; a second call hands out a fresh channel.
    pub fn take_events(&mut self) -> mpsc::UnboundedReceiver<RunServed> {
        self.events.take().unwrap_or_else(|| {
            let (events, receiver) = mpsc::unbounded_channel();
            self.configure().events = events;
            receiver
        })
    }

    /// How many isolators this process has built (D156's test hook).
    #[must_use]
    pub fn isolator_builds(&self) -> usize {
        self.shared.isolator_builds.load(Ordering::SeqCst)
    }

    /// How many tasks this runtime still owns, finished ones included until the next `serve`.
    #[must_use]
    pub fn tasks_len(&self) -> usize {
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Serves one `Orch`, `RunStream` or `RunActions` request (§8.5). Awaits nothing longer than
    /// the verdict reads: every command is a task.
    pub async fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
        live: &LiveChats,
    ) -> RunServed {
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|task| !task.handle.is_finished());
        self.shared.publisher.wire(replies);
        match &envelope.request {
            StoreRequest::RunStream { item } => {
                self.shared
                    .publisher
                    .subscribe(envelope.origin.clone(), envelope.seq, *item);
                RunServed::Reply(StoreReply::RunStream(RunFrame::subscribed(*item)))
            }
            StoreRequest::RunActions(item) => {
                RunServed::Reply(match actions(backend, *item, live).await {
                    Ok(actions) => StoreReply::RunActions(Box::new(actions)),
                    Err(err) => StoreReply::Failed {
                        request: envelope.request.name(),
                        message: err.to_string(),
                    },
                })
            }
            StoreRequest::Orch(request) => {
                let name = request.name();
                // D174: nothing is built and nothing is spawned without a server.
                if backend.writer().is_none() {
                    return RunServed::Reply(StoreReply::Failed {
                        request: name,
                        message: DATABASE_UNREACHABLE.to_owned(),
                    });
                }
                let mut request = request.clone();
                // D185: the facts only this process knows, whatever the view sent.
                if let OrchRequest::Command(command) = &mut request {
                    match command {
                        Command::PromoteStep { chat_open, .. } => *chat_open = !live.is_empty(),
                        Command::AcceptArtifact {
                            step, chat_live, ..
                        } => *chat_live = live.contains(*step),
                        _ => {}
                    }
                }
                let ctx = TaskCtx {
                    shared: Arc::clone(&self.shared),
                    backend: backend.clone(),
                    replies: replies.clone(),
                    addr: Some(ReplyAddr {
                        seq: envelope.seq,
                        origin: envelope.origin.clone(),
                    }),
                    name,
                    tag: Arc::default(),
                };
                spawn_task(ctx, request);
                RunServed::Deferred
            }
            other => RunServed::Reply(StoreReply::Failed {
                request: other.name(),
                message: "not an orchestrator request".to_owned(),
            }),
        }
    }

    /// Harness only: awaits every task, each under `limit`, and answers the runs whose task did
    /// not finish (they are aborted).
    pub async fn settle(&mut self, limit: Duration) -> Vec<RunId> {
        let mut stuck = Vec::new();
        loop {
            let tasks = std::mem::take(
                &mut *self
                    .shared
                    .tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
            if tasks.is_empty() {
                return stuck;
            }
            for Tracked { tag, handle } in tasks {
                let abort = handle.abort_handle();
                if tokio::time::timeout(limit, handle).await.is_err() {
                    abort.abort();
                    if let Some(run) = tag.run.get() {
                        stuck.push(*run);
                    }
                }
            }
        }
    }

    /// The UI is gone: every walk is cancelled — its lease given back through `abandoned` — and
    /// awaited for `2 × grace`, then aborted.
    pub async fn shutdown(&mut self, grace: Duration) {
        self.shared.walks.cancel_all();
        let tasks = std::mem::take(
            &mut *self
                .shared
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for Tracked { tag, handle } in tasks {
            let abort = handle.abort_handle();
            if tokio::time::timeout(grace * 2, handle).await.is_err() {
                abort.abort();
                tracing::warn!(run = ?tag.run.get(), "a run task did not end within the grace window");
            }
        }
    }
}

/// One task's context: what it answers through and what it works on.
#[derive(Clone)]
struct TaskCtx {
    shared: Arc<Shared>,
    backend: Backend,
    replies: mpsc::UnboundedSender<ReplyEnvelope>,
    /// The request the task answers; `None` for a sweep's resume or a claim retry, which answer
    /// nobody and only publish.
    addr: Option<ReplyAddr>,
    name: &'static str,
    tag: Arc<Tag>,
}

impl TaskCtx {
    /// The one answer to the request.
    fn answer(&self, reply: StoreReply) {
        if let Some(addr) = &self.addr {
            let _ = self.replies.send(ReplyEnvelope {
                seq: addr.seq,
                origin: addr.origin.clone(),
                reply,
            });
        }
    }

    /// A context for a task of this runtime's own: it answers nobody.
    fn unaddressed(&self, name: &'static str) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            backend: self.backend.clone(),
            replies: self.replies.clone(),
            addr: None,
            name,
            tag: Arc::default(),
        }
    }

    /// The request refused with `message`, and the refusal published for the item (D200).
    fn refuse(&self, message: String) {
        self.publish(
            self.tag.run.get().copied(),
            FrameKind::Error(message.clone()),
        );
        self.answer(StoreReply::Failed {
            request: self.name,
            message,
        });
    }

    /// A frame for the task's item, when it knows one.
    fn publish(&self, run: Option<RunId>, kind: FrameKind) {
        if let Some(item) = self.tag.item.get() {
            self.shared.publisher.publish(&RunFrame {
                item: *item,
                run,
                kind,
            });
        }
    }

    /// A command's outcome: the answer, then a `Rested` frame when the walk rested (D200).
    fn done(&self, outcome: CommandOutcome) {
        if let Some(rest) = rest_of(&outcome) {
            self.publish(self.tag.run.get().copied(), FrameKind::Rested(rest));
        }
        self.answer(StoreReply::Orch(OrchReply::Done(Box::new(outcome))));
    }

    /// Records the run (and, from its row, the item) the task works on.
    async fn tag_run(&self, writer: &Writer, run: RunId) -> Option<Run> {
        let _ = self.tag.run.set(run);
        let row = writer.run(run).await.ok().flatten();
        if let Some(item) = row.as_ref().and_then(|row| row.item_id) {
            let _ = self.tag.item.set(item);
        }
        row
    }
}

/// Where a command's walk rested, when it walked.
fn rest_of(outcome: &CommandOutcome) -> Option<Rest> {
    match outcome {
        CommandOutcome::Started { rest, .. }
        | CommandOutcome::Answered { rest }
        | CommandOutcome::Retried { rest, .. }
        | CommandOutcome::Selected { rest }
        | CommandOutcome::Cancelled { rest }
        | CommandOutcome::Promoted { rest, .. }
        | CommandOutcome::Accepted { rest } => Some(rest.clone()),
        CommandOutcome::Unblocked { rest, .. } => rest.clone(),
        CommandOutcome::ClosedOut { .. } => None,
    }
}

/// `work`, unless the task's token is cancelled first: the walk future is dropped then (M5 D86).
async fn walked<T>(walk: &WalkToken, work: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        out = work => Some(out),
        () = walk.token.cancelled() => None,
    }
}

/// Spawns one request's task, supervised and tracked.
fn spawn_task(ctx: TaskCtx, request: OrchRequest) {
    let work = run_request(ctx.clone(), request);
    spawn_supervised(ctx, work);
}

/// D158: every task runs inside a supervisor. A task that panicked has its run marked dead — the
/// next sweep releases and adopts it (R-12) — and its request answered once with
/// [`WALK_PANICKED`]. Whatever the end, this process's refused claims are then retried when the
/// task's run no longer walks (M5 D84).
fn spawn_supervised(ctx: TaskCtx, work: impl Future<Output = ()> + Send + 'static) {
    let shared = Arc::clone(&ctx.shared);
    let tag = Arc::clone(&ctx.tag);
    let handle = tokio::spawn(async move {
        let inner = tokio::spawn(work);
        if let Err(err) = inner.await
            && err.is_panic()
        {
            if let Some(run) = ctx.tag.run.get() {
                ctx.shared.dead_walks.mark(*run);
            }
            tracing::error!(run = ?ctx.tag.run.get(), "a run task panicked; the next sweep adopts its run");
            ctx.refuse(WALK_PANICKED.to_owned());
        }
        retry_claims(&ctx).await;
    });
    shared.track(tag, handle);
}

/// M5 D84: once a task's run no longer walks — parked, finished, or gone — every run a refused
/// claim left `queued` in this process is claimed again, in `queued_at` order, one task each,
/// under its lock. `Admitted` or a terminal run leaves the set; a second refusal puts it back.
async fn retry_claims(ctx: &TaskCtx) {
    let Some(run) = ctx.tag.run.get().copied() else {
        return;
    };
    if ctx
        .shared
        .queued
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty()
    {
        return;
    }
    let walks = matches!(
        ctx.backend.run(run).await,
        Ok(Some(Run {
            status: RunStatus::Running | RunStatus::Queued,
            ..
        }))
    );
    if walks {
        return;
    }
    let queued = std::mem::take(
        &mut *ctx
            .shared
            .queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    for (queued_at, run) in queued {
        let retry = ctx.unaddressed("claim_retry");
        spawn_supervised(retry.clone(), reclaim(retry, run, queued_at));
    }
}

/// One claim retry: the run's lock, then `claim` and its walk.
async fn reclaim(ctx: TaskCtx, run: RunId, queued_at: DateTime<Utc>) {
    let _ = ctx.tag.run.set(run);
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return;
    };
    let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
        Ok(kit) => kit,
        Err(message) => {
            ctx.shared.queue(queued_at, run);
            return ctx.refuse(message);
        }
    };
    if ctx.tag_run(&kit.writer, run).await.map(|row| row.status) != Some(RunStatus::Queued) {
        return;
    }
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    match walked(&walk, engine.claim(run)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        Some(Err(err @ EngineError::ClaimRefused { .. })) => {
            ctx.shared.queue(queued_at, run);
            tracing::debug!(%run, %err, "a queued run's claim was refused again");
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// D158, D189: one sweep. Nothing is built when there is nothing to adopt: no dead walk of this
/// process and no run holding a slot on this box.
async fn sweep_once(ctx: TaskCtx) {
    let backend = &ctx.backend;
    if !ctx.shared.sweep_fixed
        && let Ok(app) = backend.app_settings().await
    {
        ctx.shared
            .sweep_every
            .store(millis(lease_period(&app)), Ordering::SeqCst);
    }
    let Ok(box_id) = registered_box(backend).await else {
        return;
    };
    if ctx.shared.dead_walks.runs().is_empty()
        && matches!(backend.active_runs_on_box(box_id).await, Ok(0))
    {
        return;
    }
    let kit = match Kit::read(&ctx.shared, backend, false).await {
        Ok(kit) => kit,
        Err(message) => {
            tracing::warn!(%message, "the sweep could not build its engine");
            return;
        }
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let adopted = match engine.sweep_fenced(&ctx.shared.locks).await {
        Ok(adopted) => adopted,
        Err(err) => {
            tracing::warn!(%err, "the sweep failed");
            return;
        }
    };
    for Adopted { run, next } in adopted {
        let item = kit
            .writer
            .run(run)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.item_id);
        let frame = |kind: FrameKind| {
            if let Some(item) = item {
                ctx.shared.publisher.publish(&RunFrame {
                    item,
                    run: Some(run),
                    kind,
                });
            }
        };
        match next {
            Next::Walk => {
                frame(FrameKind::Adopted);
                let resume = ctx.unaddressed("resume");
                spawn_supervised(resume.clone(), resumed(resume, run));
            }
            Next::Parked(rest) | Next::Finished(rest) => frame(FrameKind::Rested(rest)),
            Next::Error(sentence) => frame(FrameKind::Error(sentence)),
        }
    }
}

/// D158: an adopted run's walk, resumed on its own task under its lock.
async fn resumed(ctx: TaskCtx, run: RunId) {
    let _ = ctx.tag.run.set(run);
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return;
    };
    let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    ctx.tag_run(&kit.writer, run).await;
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    match walked(&walk, engine.resume(run)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(Resume::Walked(rest) | Resume::TopologyChanged { rest, .. })) => {
            ctx.publish(Some(run), FrameKind::Rested(rest));
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// §8.6: one request, start to answer.
async fn run_request(ctx: TaskCtx, request: OrchRequest) {
    match request {
        OrchRequest::Command(Command::StartRun {
            item,
            mode,
            repo_scope,
        }) => start_run(ctx, item, mode, repo_scope).await,
        OrchRequest::Command(Command::Unblock { item }) => unblock(ctx, item).await,
        OrchRequest::Command(command @ Command::CloseOut { item }) => {
            let _ = ctx.tag.item.set(item);
            let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
                Ok(kit) => kit,
                Err(message) => return ctx.refuse(message),
            };
            let driver =
                |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
            match kit.engine(&driver).dispatch(command).await {
                Ok(outcome) => ctx.done(outcome),
                Err(err) => ctx.refuse(err.to_string()),
            }
        }
        OrchRequest::Command(command) => {
            let (run, preempt) = match &command {
                Command::CancelRun { run } => (*run, Preempt::Always),
                Command::PromoteStep { run, .. } => (*run, Preempt::IfLive),
                Command::AnswerGate { run, .. }
                | Command::RetryStep { run, .. }
                | Command::SelectFanout { run, .. }
                | Command::AcceptArtifact { run, .. } => (*run, Preempt::Never),
                Command::StartRun { .. } | Command::Unblock { .. } | Command::CloseOut { .. } => {
                    unreachable!("matched above")
                }
            };
            on_run(ctx, run, command, preempt).await;
        }
        OrchRequest::CloseOutPreview { item } => {
            let _ = ctx.tag.item.set(item);
            let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
                Ok(kit) => kit,
                Err(message) => return ctx.refuse(message),
            };
            let driver =
                |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
            match kit.engine(&driver).close_out_preview(item).await {
                Ok(preview) => ctx.answer(StoreReply::Orch(OrchReply::CloseOutPreview(Box::new(
                    preview,
                )))),
                Err(err) => ctx.answer(StoreReply::Failed {
                    request: ctx.name,
                    message: err.to_string(),
                }),
            }
        }
        OrchRequest::Cleanup { run } => cleanup(ctx, run).await,
    }
}

/// Whether a command stops a live walk of its run before it waits for the lock (D157, D187).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preempt {
    /// `CancelRun`.
    Always,
    /// `PromoteStep`: only a walk of this process that is live.
    IfLive,
    /// Every other verb waits for the walk to rest.
    Never,
}

/// `StartRun` (D186): enqueue, then — under the run's lock and token — claim and walk.
async fn start_run(
    ctx: TaskCtx,
    item: ItemId,
    mode: htui_core::model::RunMode,
    repo_scope: Option<Vec<RepoId>>,
) {
    let _ = ctx.tag.item.set(item);
    let kit = match Kit::read(&ctx.shared, &ctx.backend, true).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let run = match engine.enqueue(item, mode, repo_scope).await {
        Ok(run) => run,
        Err(err) => return ctx.refuse(err.to_string()),
    };
    let _ = ctx.tag.run.set(run);
    ctx.publish(Some(run), FrameKind::Started);

    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    match walked(&walk, engine.claim(run)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        Some(Err(err @ EngineError::ClaimRefused { .. })) => {
            if let Ok(Some(row)) = kit.writer.run(run).await {
                ctx.shared.queue(row.queued_at, run);
            }
            ctx.refuse(err.to_string());
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// Every verb on one run: preempt as the verb says, lock, dispatch (§8.6).
async fn on_run(ctx: TaskCtx, run: RunId, command: Command, preempt: Preempt) {
    let _ = ctx.tag.run.set(run);
    let stop = match preempt {
        Preempt::Always => true,
        Preempt::IfLive => ctx.shared.walks.is_live(run),
        Preempt::Never => false,
    };
    if stop {
        ctx.shared.walks.preempt(run);
    }
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let row = ctx.tag_run(&kit.writer, run).await;
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    match walked(&walk, engine.dispatch(command)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(CommandOutcome::Promoted {
            step,
            rest,
            opening,
        })) => {
            let via = match opening.path {
                OpeningPath::Resume { .. } => Via::Resumed,
                OpeningPath::Handoff { .. } => Via::Handoff,
            };
            ctx.answer(StoreReply::Orch(OrchReply::Promoted {
                step,
                run,
                phase: opening.phase.clone(),
                agent: opening.agent_name.clone(),
                model: opening.model.clone(),
                via,
            }));
            ctx.publish(Some(run), FrameKind::Rested(rest));
            // D181: the chat binding is the loop's; the runtime's event channel reaches it.
            if let (Some(row), Some(addr)) = (row, ctx.addr.clone()) {
                let _ = ctx.shared.events.send(RunServed::Attach {
                    addr,
                    promoted: Box::new(Promoted {
                        run,
                        step,
                        project: row.project_id,
                        opening: *opening,
                    }),
                });
            }
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// `Unblock` (D161): a reopen needs no run; the other two cases lock the run they name and check
/// the case again under the lock (§8.6).
async fn unblock(ctx: TaskCtx, item: ItemId) {
    let _ = ctx.tag.item.set(item);
    let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let case = match engine.unblock_case(item).await {
        Ok(case) => case,
        Err(err) => return ctx.refuse(err.to_string()),
    };
    let run = match case {
        UnblockCase::Reopen => {
            return match engine.dispatch(Command::Unblock { item }).await {
                Ok(outcome) => ctx.done(outcome),
                Err(err) => ctx.refuse(err.to_string()),
            };
        }
        UnblockCase::FollowRun(run) | UnblockCase::Resume(run) => run,
    };
    let _ = ctx.tag.run.set(run);
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    if engine.unblock_case(item).await.ok() != Some(case) {
        return ctx.refuse(UNBLOCK_MOVED.to_owned());
    }
    match walked(&walk, engine.dispatch(Command::Unblock { item })).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// D177 (R-25): a terminal run's cleanup, again, under the run's lock.
async fn cleanup(ctx: TaskCtx, run: RunId) {
    let _ = ctx.tag.run.set(run);
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    let kit = match Kit::read(&ctx.shared, &ctx.backend, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let Some(row) = ctx.tag_run(&kit.writer, run).await else {
        return ctx.refuse(
            StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            }
            .to_string(),
        );
    };
    if let Err(err) = cleanup_enabled(&row) {
        return ctx.refuse(err.to_string());
    }
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    match kit.engine(&driver).cleanup_run(run).await {
        Ok(()) => ctx.answer(StoreReply::Orch(OrchReply::CleanedUp { run })),
        Err(err) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// Plan D155: `GraphSource` is `htui-orch`'s and `Backend` is `htui-store`'s, so `impl GraphSource
/// for Backend` here is E0117 (proven: plan Verified claims). A local newtype is the answer, and it
/// keeps invariant 10 (the orchestrator never names `htui-store`).
///
/// Every read delegates to the `Backend`-inherent read of the same name; `agent` filters
/// [`Backend::agents`] exactly as the `MemStore` implementation in `htui-orch`'s fake does.
#[derive(Debug, Clone)]
pub struct BackendGraphs(pub Backend);

impl GraphSource for BackendGraphs {
    async fn resolve_graph(&self, item: ItemId) -> StoreResult<Option<ResolvedGraph>> {
        self.0.resolve_graph(item).await
    }

    async fn phase_agents(&self, phase: PhaseId) -> StoreResult<Vec<PhaseAgent>> {
        self.0.phase_agents(phase).await
    }

    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> StoreResult<Option<PromptTemplate>> {
        self.0.prompt_template(project, name, version).await
    }

    async fn agent(&self, id: AgentId) -> StoreResult<Option<Agent>> {
        Ok(self
            .0
            .agents()
            .await?
            .into_iter()
            .map(|summary| summary.agent)
            .find(|agent| agent.id == id))
    }

    async fn agent_boxes(&self, box_id: BoxId) -> StoreResult<Vec<AgentBox>> {
        self.0.agent_boxes(box_id).await
    }
}

/// The walk fixture is shared with `store_worker`'s promotion case (blueprint §8.10).
#[cfg(test)]
pub(crate) mod tests {
    use std::collections::{BTreeSet, VecDeque};
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::driver::{
        AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
        PermissionRequestId, SessionSpec,
    };
    use htui_agent::error::DriverError;
    use htui_agent::event::{DoneEvent, DriverEnvelope, DriverEvent, StopReason};
    use htui_agent::fake::FakeDriver;
    use htui_agent::registry::{DriverFactory, TransportBuilder};
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{
        Agent, AgentBox, AgentId, Billing, DocumentId, Item, ItemId, NewDocument, NewRepo, NewRun,
        RepoId, Run, RunId, RunMode, RunStatus, RunStep, SnapshotPhase, Status, StepId,
        TIMESTAMPTZ_DIGITS, Transport,
    };
    use htui_core::store::mem::MemFault;
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_orch::fake::{FakeIsolator, FakeVerifier};
    use htui_orch::{
        Clock, Command, CommandOutcome, EngineError, GateAnswer, GraphSource, Isolator,
    };
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore, Started};
    use serde_json::json;
    use tokio::sync::{Notify, mpsc};

    use super::{
        BackendGraphs, FrameKind, LiveChats, ORCH_NAMES, OrchReply, OrchRequest, PREEMPTED,
        RunRuntime, RunServed, StepAuthor, WALK_PANICKED,
    };
    use crate::agent_worker::AgentRuntime;
    use crate::store_worker::{
        self, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, spawn_with_runtimes,
    };
    use crate::ui::tabs::TabId;
    use uuid::Uuid;

    /// The scripted registry row every walk test runs on (blueprint F-O): an `acp` row, because
    /// the fixture graphs gate every phase and stage 1's inline-approval interlock skips a `cli`
    /// row at a gated phase; the factory reaches the fake by row data alone.
    fn scripted_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: Some("sonnet".to_owned()),
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({}),
            enabled: true,
            created_at: demo_at(0, 0),
            updated_at: demo_at(0, 0),
        }
    }

    /// The scripted agent's `agent_box` on the demo box, probed ready (rung 3, plan D62).
    fn ready_on_box(agent_id: AgentId, at: DateTime<Utc>) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        }
    }

    /// Blueprint F-O: the demo with every fixture agent disabled, one scripted agent and its
    /// `agent_box` on the demo box, so rung 3 of the candidate chain names exactly it.
    async fn seeded_store() -> (MemStore, AgentId) {
        let store = MemStore::demo();
        for summary in store.agents().await.expect("the fixture's agents") {
            let mut row = summary.agent;
            row.enabled = false;
            store.upsert_agent(&row).await.expect("the row is disabled");
        }
        let agent = AgentId::new();
        store
            .upsert_agent(&scripted_row(agent))
            .await
            .expect("the scripted row lands");
        store
            .upsert_agent_box(&ready_on_box(agent, demo_at(0, 0)))
            .await
            .expect("the agent_box row lands");
        // The demo project has no repo, and a promoted step chats in its own tree: the primary
        // repo is the scope a default `StartRun` resolves to.
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
        (store, agent)
    }

    // -----------------------------------------------------------------------------------------
    // The walk fixture (blueprint §8.9, F-O)
    // -----------------------------------------------------------------------------------------

    /// What one agent session does.
    #[derive(Debug, Clone)]
    enum Play {
        /// One turn, then `done`.
        Done,
        /// Waits on `release` before its first event; `reached` is raised when it starts waiting.
        Stall(Stall),
        /// The session's start panics.
        Panic,
    }

    /// A stalled session's three signals.
    #[derive(Debug, Clone, Default)]
    struct Stall {
        reached: Arc<Notify>,
        release: Arc<Notify>,
        dropped: Arc<AtomicBool>,
    }

    /// The sessions the fixture's builds play, in build order; [`Play::Done`] once it is empty.
    #[derive(Debug, Default)]
    struct Sessions(StdMutex<VecDeque<Play>>);

    impl Sessions {
        fn push(&self, play: Play) {
            self.0.lock().expect("the queue").push_back(play);
        }
    }

    /// The transport the scripted row reaches: one [`ScriptedDriver`] per session.
    #[derive(Debug)]
    struct Scripted(Arc<Sessions>);

    impl TransportBuilder for Scripted {
        fn build(
            &self,
            agent: &Agent,
            _on_box: Option<&AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            let play = self
                .0
                .0
                .lock()
                .expect("the queue")
                .pop_front()
                .unwrap_or(Play::Done);
            let done = Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            }))]);
            Ok(Box::new(ScriptedDriver {
                inner: FakeDriver::new(agent.name.clone(), caps, done),
                play,
            }))
        }
    }

    #[derive(Debug)]
    struct ScriptedDriver {
        inner: FakeDriver,
        play: Play,
    }

    impl AgentDriver for ScriptedDriver {
        fn name(&self) -> &str {
            self.inner.name()
        }

        fn caps(&self) -> DriverCaps {
            self.inner.caps()
        }

        fn start<'a>(
            &'a self,
            spec: SessionSpec,
            prompt: String,
        ) -> DriverFuture<'a, Box<dyn AgentSession>> {
            let inner = self.inner.start(spec, prompt);
            let play = self.play.clone();
            Box::pin(async move {
                let session = inner.await?;
                match play {
                    Play::Done => Ok(session),
                    Play::Stall(stall) => Ok(Box::new(Stalled {
                        inner: session,
                        stall,
                        released: false,
                    }) as Box<dyn AgentSession>),
                    Play::Panic => panic!("a scripted session panics"),
                }
            })
        }
    }

    /// A session that waits on its [`Stall`] before its first event, and says when it is dropped.
    #[derive(Debug)]
    struct Stalled {
        inner: Box<dyn AgentSession>,
        stall: Stall,
        released: bool,
    }

    impl Drop for Stalled {
        fn drop(&mut self) {
            self.stall.dropped.store(true, Ordering::SeqCst);
        }
    }

    impl AgentSession for Stalled {
        fn session_ref(&self) -> Option<&AgentSessionRef> {
            self.inner.session_ref()
        }

        fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
            Box::pin(async move {
                if !self.released {
                    self.stall.reached.notify_one();
                    self.stall.release.notified().await;
                    self.released = true;
                }
                self.inner.next_event().await
            })
        }

        fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
            self.inner.send_follow_up(text)
        }

        fn answer_permission<'a>(
            &'a mut self,
            request_id: PermissionRequestId,
            answer: PermissionAnswer,
        ) -> DriverFuture<'a, ()> {
            self.inner.answer_permission(request_id, answer)
        }

        fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
            self.inner.cancel(grace)
        }
    }

    /// Blueprint H-2: `base + (tokio now - start)`, so a `start_paused` test moves the lease
    /// fence with the heartbeat's own sleeps.
    #[derive(Debug)]
    struct TokioClock {
        base: DateTime<Utc>,
        start: tokio::time::Instant,
    }

    impl TokioClock {
        fn new() -> Self {
            Self {
                base: Utc::now(),
                start: tokio::time::Instant::now(),
            }
        }
    }

    impl Clock for TokioClock {
        fn now(&self) -> DateTime<Utc> {
            let elapsed = TimeDelta::from_std(self.start.elapsed()).expect("a test is short");
            (self.base + elapsed).trunc_subsecs(TIMESTAMPTZ_DIGITS)
        }
    }

    /// D203's author for tests: one document of the phase's `output_kind` per step.
    #[derive(Debug)]
    struct OutputAuthor;

    impl StepAuthor for OutputAuthor {
        fn document(
            &self,
            item: ItemId,
            step: &RunStep,
            phase: &SnapshotPhase,
        ) -> Option<NewDocument> {
            Some(NewDocument {
                id: DocumentId::new(),
                item_id: item,
                kind: phase.output_kind.clone(),
                title: format!("{} (attempt {})", phase.output_kind, step.attempt),
                body: "authored".to_owned(),
                produced_by_step_id: Some(step.id),
                created_by: ids::USER,
                created_at: Utc::now(),
            })
        }
    }

    /// The seeded store, the sessions its builds play, and the isolator its runs use.
    pub(crate) struct Fixture {
        pub(crate) store: MemStore,
        sessions: Arc<Sessions>,
        isolator: Arc<FakeIsolator>,
    }

    impl Fixture {
        pub(crate) async fn new() -> Self {
            let (store, _) = seeded_store().await;
            Self {
                store,
                sessions: Arc::default(),
                isolator: Arc::new(FakeIsolator::new()),
            }
        }

        /// The registry the scripted row reaches the fake through, by row data (`acp`).
        fn factory(&self) -> DriverFactory {
            let mut factory = DriverFactory::new();
            factory.register("acp", Box::new(Scripted(Arc::clone(&self.sessions))));
            factory
        }

        /// Blueprint §8.9's runtime: the fakes, a tokio-time clock and the output author.
        pub(crate) fn runtime(&self) -> RunRuntime {
            RunRuntime::with_parts(
                Arc::clone(&self.isolator) as Arc<dyn Isolator>,
                Arc::new(FakeVerifier::new()),
                self.factory(),
            )
            .with_clock(Arc::new(TokioClock::new()))
            .with_author(Arc::new(OutputAuthor))
        }

        pub(crate) async fn run(&self, id: RunId) -> Run {
            self.store
                .run(id)
                .await
                .expect("the read answers")
                .expect("the run exists")
        }

        async fn item(&self, id: ItemId) -> Item {
            self.store
                .item(id)
                .await
                .expect("the read answers")
                .expect("the item exists")
        }

        async fn steps(&self, run: RunId) -> Vec<RunStep> {
            self.store.run_steps(run).await.expect("the read answers")
        }
    }

    /// How long a test waits for any one reply before it calls the worker stuck.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A store worker over the fixture's store, with the run runtime under test.
    pub(crate) struct Worker {
        requests: mpsc::UnboundedSender<RequestEnvelope>,
        replies: mpsc::UnboundedReceiver<ReplyEnvelope>,
        seen: Vec<ReplyEnvelope>,
        seq: u64,
    }

    impl Worker {
        pub(crate) fn spawn(store: &MemStore, runtime: RunRuntime) -> Self {
            let (requests, requests_rx) = mpsc::unbounded_channel();
            let (replies_tx, replies) = mpsc::unbounded_channel();
            let _worker = spawn_with_runtimes(
                Started::detached(Backend::memory(store.clone())),
                requests_rx,
                replies_tx,
                AgentRuntime::new(DriverFactory::new()),
                runtime,
            );
            Self {
                requests,
                replies,
                seen: Vec::new(),
                seq: 0,
            }
        }

        /// Sends `request` from `origin` at the next `seq`.
        pub(crate) fn send(&mut self, origin: Origin, request: StoreRequest) -> u64 {
            self.seq += 1;
            self.send_at(origin, self.seq, request)
        }

        /// Sends `request` from `origin` at exactly `seq`.
        fn send_at(&mut self, origin: Origin, seq: u64, request: StoreRequest) -> u64 {
            self.seq = self.seq.max(seq);
            self.requests
                .send(RequestEnvelope {
                    seq,
                    origin,
                    request,
                })
                .expect("the worker is running");
            seq
        }

        /// The first non-stream reply at `seq`, waiting for it.
        pub(crate) async fn reply(&mut self, seq: u64) -> StoreReply {
            self.envelope(seq).await.reply
        }

        /// The first non-stream envelope at `seq`, waiting for it.
        pub(crate) async fn envelope(&mut self, seq: u64) -> ReplyEnvelope {
            self.envelope_within(seq, PATIENCE).await
        }

        /// [`Self::envelope`], waiting up to `patience` for each arrival.
        async fn envelope_within(&mut self, seq: u64, patience: Duration) -> ReplyEnvelope {
            loop {
                if let Some(at) = self.seen.iter().position(|envelope| {
                    envelope.seq == seq && !matches!(envelope.reply, StoreReply::RunStream(_))
                }) {
                    return self.seen.remove(at);
                }
                let envelope = tokio::time::timeout(patience, self.replies.recv())
                    .await
                    .unwrap_or_else(|_| panic!("no reply at seq {seq} within {patience:?}"))
                    .expect("the worker is running");
                self.seen.push(envelope);
            }
        }

        /// Whatever has arrived by now.
        fn drain(&mut self) {
            while let Ok(envelope) = self.replies.try_recv() {
                self.seen.push(envelope);
            }
        }
    }

    /// `StartRun` for `item`, manual, default scope.
    pub(crate) fn start_run(item: ItemId) -> StoreRequest {
        StoreRequest::Orch(OrchRequest::Command(Command::StartRun {
            item,
            mode: RunMode::Manual,
            repo_scope: None,
        }))
    }

    /// `future`, or a panic naming what did not happen.
    async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
        tokio::time::timeout(PATIENCE, future)
            .await
            .unwrap_or_else(|_| panic!("{what} did not happen within {PATIENCE:?}"))
    }

    /// The outcome of a `Done` reply, or a panic showing what came instead.
    fn outcome(reply: StoreReply) -> CommandOutcome {
        match reply {
            StoreReply::Orch(OrchReply::Done(outcome)) => *outcome,
            other => panic!("expected a command outcome, got {other:?}"),
        }
    }

    /// `R-NF-3`: a walk mid-session does not hold the loop — the `Workspaces` read asked after
    /// the `StartRun` is answered first, and the `StartRun` once its session ends.
    #[tokio::test]
    async fn an_orch_request_is_served_off_the_loop() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let workspaces = worker.send(Origin::App, StoreRequest::Workspaces);
        assert!(matches!(
            worker.reply(workspaces).await,
            StoreReply::Workspaces(_)
        ));
        worker.drain();
        assert!(
            !worker.seen.iter().any(|envelope| envelope.seq == start),
            "the walk is still in its session"
        );

        stall.release.notify_one();
        let CommandOutcome::Started { run, rest } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval, "`research` gates");
        assert_eq!(fixture.run(run).await.status, RunStatus::AwaitingApproval);
    }

    /// The one run of `item`, which a test has just started.
    pub(crate) async fn only_run(store: &MemStore, item: ItemId) -> RunId {
        let runs = store.runs(item).await.expect("the read answers");
        assert_eq!(runs.len(), 1, "one run of the item: {runs:?}");
        runs[0].id
    }

    /// The latest step at `position` of `run`.
    pub(crate) async fn step_at(fixture: &Fixture, run: RunId, position: i32) -> RunStep {
        fixture
            .steps(run)
            .await
            .into_iter()
            .filter(|step| step.position == position)
            .max_by_key(|step| step.attempt)
            .expect("a step at the position")
    }

    /// A parked run of `HTUI_ANA_2`: its `research` step awaits approval.
    pub(crate) async fn parked(fixture: &Fixture, worker: &mut Worker) -> (RunId, RunStep) {
        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        let CommandOutcome::Started { run, rest } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        assert_eq!(rest.run, RunStatus::AwaitingApproval);
        (run, step_at(fixture, run, 0).await)
    }

    /// R-27: a second command on a run waits for the first to finish its walk, then is judged
    /// against the rows that walk left.
    #[tokio::test]
    async fn two_commands_on_one_run_are_serialised() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, research) = parked(&fixture, &mut worker).await;

        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let retry = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::RetryStep {
                run,
                step: research.id,
            })),
        );
        within("attempt 2's session starting", stall.reached.notified()).await;
        let answer = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        worker.drain();
        assert!(
            !worker
                .seen
                .iter()
                .any(|envelope| envelope.seq == answer || envelope.seq == retry),
            "neither command has answered while attempt 2 walks: {:?}",
            worker.seen
        );

        stall.release.notify_one();
        assert!(matches!(
            outcome(worker.reply(retry).await),
            CommandOutcome::Retried { .. }
        ));
        let StoreReply::Failed { request, message } = worker.reply(answer).await else {
            panic!("the answer is refused");
        };
        assert_eq!(request, "answer_gate");
        assert!(
            message.contains(&format!("step {} is `superseded`", research.id)),
            "the gate is judged after the retry moved the step: {message}"
        );
    }

    /// D157, D187, D188: `CancelRun` stops a walk mid-session instead of waiting hours for it,
    /// and the dropped walk's lease and guards are given back.
    #[tokio::test]
    async fn cancel_preempts_a_live_walk() {
        let fixture = Fixture::new().await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        );

        assert!(matches!(
            outcome(worker.reply(cancel).await),
            CommandOutcome::Cancelled { .. }
        ));
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == PREEMPTED),
            "the preempted start is answered once, with the sentence"
        );
        assert!(
            stall.dropped.load(Ordering::SeqCst),
            "the session was dropped"
        );
        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Cancelled);
        assert!(
            row.lease_expires_at
                .is_some_and(|until| until <= Utc::now()),
            "the lease is given back: {:?}",
            row.lease_expires_at
        );
        assert_eq!(fixture.item(ids::HTUI_ANA_2).await.status, Status::Open);
        assert!(
            fixture.isolator.releases() >= 1,
            "the abandoned walk's guards were released"
        );
    }

    /// D182, D184: the verdicts are the engine's own guards over the rows.
    #[tokio::test]
    async fn run_actions_grey_by_the_engine_guards() {
        let fixture = Fixture::new().await;
        let runtime = RunRuntime::with_parts(
            Arc::clone(&fixture.isolator) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            fixture.factory(),
        )
        .with_clock(Arc::new(TokioClock::new()));
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let (_run, research) = parked(&fixture, &mut worker).await;

        let ask = worker.send(Origin::App, StoreRequest::RunActions(ids::HTUI_ANA_2));
        let StoreReply::RunActions(actions) = worker.reply(ask).await else {
            panic!("the verdicts");
        };
        assert_eq!(
            actions.steps[&research.id].approve,
            Err(EngineError::MissingOutputForApproval {
                step: research.id,
                kind: "research".to_owned(),
            }
            .to_string())
        );
        assert_eq!(actions.steps[&research.id].reject, Ok(()));

        let document = OutputAuthor
            .document(ids::HTUI_ANA_2, &research, &research_phase())
            .expect("the author writes one");
        fixture
            .store
            .write_document(document)
            .await
            .expect("the document lands");
        let ask = worker.send(Origin::App, StoreRequest::RunActions(ids::HTUI_ANA_2));
        let StoreReply::RunActions(actions) = worker.reply(ask).await else {
            panic!("the verdicts");
        };
        assert_eq!(actions.steps[&research.id].approve, Ok(()));
        assert!(actions.steps[&research.id].open.is_ok());
        assert_eq!(actions.steps[&research.id].promote, Ok(()));

        let live = super::actions(
            &Backend::memory(fixture.store.clone()),
            ids::HTUI_ANA_2,
            &LiveChats::of([StepId::new()]),
        )
        .await
        .expect("the verdicts");
        assert_eq!(
            live.steps[&research.id].promote,
            Err(EngineError::ChatLive { step: None }.to_string()),
            "a chat of this process is live elsewhere"
        );
    }

    /// The `research` phase as the ANA graph's snapshot carries it, for the author.
    fn research_phase() -> SnapshotPhase {
        SnapshotPhase {
            output_kind: "research".to_owned(),
            ..serde_json::from_value(json!({
                "position": 0, "name": "research", "fan_out": 1, "gate": "always",
                "gate_effective": "always", "gate_hard": false, "retry_limit": 1,
                "input_kinds": [], "output_kind": "research", "isolation": "worktree",
                "command_queue": "fan_out_only", "verify_command": null,
                "deadline_seconds": null, "template": { "name": "research", "version": 1 },
                "token_budget": null, "candidates": [], "judge": null
            }))
            .expect("a phase")
        }
    }

    /// D177 (R-25): a cleanup retry runs on a terminal run and is refused on a live one.
    #[tokio::test]
    async fn cleanup_retries_a_terminal_run() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (run, _) = parked(&fixture, &mut worker).await;

        let early = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Cleanup { run }),
        );
        assert_eq!(
            match worker.reply(early).await {
                StoreReply::Failed { message, .. } => message,
                other => panic!("a live run is refused: {other:?}"),
            },
            EngineError::NotTerminal {
                run,
                status: RunStatus::AwaitingApproval,
            }
            .to_string()
        );

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run })),
        );
        outcome(worker.reply(cancel).await);
        let before = fixture.isolator.cleanups();
        let again = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Cleanup { run }),
        );
        assert!(matches!(
            worker.reply(again).await,
            StoreReply::Orch(OrchReply::CleanedUp { run: cleaned }) if cleaned == run
        ));
        assert_eq!(fixture.isolator.cleanups(), before + 1);
    }

    /// D156: the production isolator is built at the first command and kept while the repo map
    /// does not move.
    #[tokio::test]
    async fn the_isolator_is_built_once_per_process() {
        let fixture = Fixture::new().await;
        let scratch = tempfile::tempdir().expect("a scratch root");
        let mut runtime = RunRuntime::new(fixture.factory())
            .with_clock(Arc::new(TokioClock::new()))
            .with_scratch_root(scratch.path().to_path_buf());
        let backend = Backend::memory(fixture.store.clone());
        let (replies, mut answers) = mpsc::unbounded_channel();
        assert_eq!(
            runtime.isolator_builds(),
            0,
            "nothing is built before a command"
        );

        for (seq, item) in [(1, ids::HTUI_ANA_2), (2, ids::AGY_FEAT_1)] {
            let served = runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request: start_run(item),
                    },
                    &LiveChats::default(),
                )
                .await;
            assert!(matches!(served, RunServed::Deferred));
            assert!(runtime.settle(PATIENCE).await.is_empty());
        }
        let mut answered = 0;
        while answers.try_recv().is_ok() {
            answered += 1;
        }
        assert!(answered >= 2, "both starts were answered");
        assert_eq!(runtime.isolator_builds(), 1);
    }

    /// A run of `item` another process claimed an hour ago and never renewed: `running`, its
    /// lease expired, its owner not this one. No step was created.
    async fn stranded(fixture: &Fixture, item: ItemId) -> RunId {
        let backend = Backend::memory(fixture.store.clone());
        let row = fixture.item(item).await;
        let app = fixture.store.app_settings().await.expect("the settings");
        let resolved = htui_orch::resolve(
            &fixture.store,
            &BackendGraphs(backend),
            &row,
            RunMode::Manual,
            &app,
            None,
            ids::BOX,
        )
        .await
        .expect("the item resolves");
        let past = Utc::now() - TimeDelta::hours(1);
        let run = RunId::new();
        fixture
            .store
            .create_run(NewRun {
                id: run,
                project_id: row.project_id,
                item_id: row.id,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: resolved.snapshot,
                repo_scope: resolved.repo_scope,
                queued_at: past,
            })
            .await
            .expect("the run lands");
        let claim = fixture
            .store
            .claim_run(
                run,
                ids::BOX,
                Uuid::now_v7(),
                past,
                past + TimeDelta::minutes(1),
            )
            .await
            .expect("the claim answers");
        assert!(claim.is_admitted(), "{claim}");
        run
    }

    /// Polls the store until `run` reaches `status`.
    async fn rests_at(fixture: &Fixture, run: RunId, status: RunStatus) {
        rests_within(fixture, run, status, PATIENCE).await;
    }

    /// [`rests_at`] with a patience of its own: a paused-clock case waits out sweep periods.
    async fn rests_within(fixture: &Fixture, run: RunId, status: RunStatus, patience: Duration) {
        tokio::time::timeout(patience, async {
            while fixture.run(run).await.status != status {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {run} did not reach `{status}` within {patience:?}"));
    }

    /// D158: the startup sweep adopts every stranded run and resumes each on its own task, so one
    /// run's session never waits on another's.
    #[tokio::test]
    async fn the_sweep_resumes_each_adopted_run_on_its_own_task() {
        let fixture = Fixture::new().await;
        let first = stranded(&fixture, ids::HTUI_ANA_2).await;
        let second = stranded(&fixture, ids::AGY_FEAT_1).await;
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let _worker = Worker::spawn(&fixture.store, fixture.runtime());

        within("one resumed session starting", stall.reached.notified()).await;
        within("the other run resting while the first stalls", async {
            loop {
                let rested = [first, second].len()
                    - [fixture.run(first).await, fixture.run(second).await]
                        .iter()
                        .filter(|row| row.status == RunStatus::Running)
                        .count();
                if rested == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        stall.release.notify_one();
        rests_at(&fixture, first, RunStatus::AwaitingApproval).await;
        rests_at(&fixture, second, RunStatus::AwaitingApproval).await;
    }

    /// D189: a sweep leaves a run a command of this process holds alone — its lease renewed to
    /// this owner by the adoption, and the run left to the holder.
    #[tokio::test]
    async fn a_sweep_skips_a_run_a_command_holds() {
        let fixture = Fixture::new().await;
        let run = stranded(&fixture, ids::HTUI_ANA_2).await;
        let mut runtime = fixture.runtime();
        let held = runtime.shared.locks.try_lock(run).expect("nobody holds it");
        let (replies, _answers) = mpsc::unbounded_channel();

        runtime.sweep(&Backend::memory(fixture.store.clone()), &replies);
        assert!(runtime.settle(PATIENCE).await.is_empty());

        let row = fixture.run(run).await;
        assert_eq!(row.status, RunStatus::Running, "not recovered");
        assert!(fixture.steps(run).await.is_empty(), "nothing walked");
        assert!(
            row.lease_expires_at.is_some_and(|until| until > Utc::now()),
            "the adoption leased it to this owner: {:?}",
            row.lease_expires_at
        );
        assert!(
            runtime.shared.dead_walks.contains(run),
            "and the next free sweep gives it back"
        );
        drop(held);
    }

    /// R-12, D158: a walk task that panics is marked dead, answered once with the sentence, and
    /// the next sweep releases and adopts its run, which a healthy session then rests.
    #[tokio::test]
    async fn a_panicked_walk_is_adopted_by_the_next_sweep() {
        let fixture = Fixture::new().await;
        fixture.sessions.push(Play::Panic);
        let mut worker = Worker::spawn(
            &fixture.store,
            fixture
                .runtime()
                .with_sweep_every(Duration::from_millis(100)),
        );

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        assert!(
            matches!(worker.reply(start).await, StoreReply::Failed { request: "start_run", ref message } if message == WALK_PANICKED),
            "the panicked task's request is answered"
        );
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        rests_at(&fixture, run, RunStatus::AwaitingApproval).await;
    }

    /// M5 D84: a run whose claim was refused waits, and is claimed again once a walk of this
    /// process rests.
    #[tokio::test]
    async fn a_refused_claim_is_retried_when_a_walk_rests() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let (first, _) = parked(&fixture, &mut worker).await;

        let start = worker.send(Origin::App, start_run(ids::HTUI_CLEAN_1));
        let StoreReply::Failed { message, .. } = worker.reply(start).await else {
            panic!("the second claim is refused");
        };
        assert!(
            message.contains("overlaps run"),
            "both runs scope the primary repo: {message}"
        );
        let second = only_run(&fixture.store, ids::HTUI_CLEAN_1).await;
        assert_eq!(fixture.run(second).await.status, RunStatus::Queued);

        let cancel = worker.send(
            Origin::App,
            StoreRequest::Orch(OrchRequest::Command(Command::CancelRun { run: first })),
        );
        outcome(worker.reply(cancel).await);
        rests_at(&fixture, second, RunStatus::AwaitingApproval).await;
    }

    /// The frames (not the acknowledgements) received so far, taken out of `seen`.
    fn frames(worker: &mut Worker) -> Vec<(Origin, u64, super::RunFrame)> {
        worker.drain();
        let (frames, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut worker.seen)
            .into_iter()
            .partition(|envelope| {
                matches!(&envelope.reply, StoreReply::RunStream(frame)
                    if !matches!(frame.kind, FrameKind::Subscribed))
            });
        worker.seen = rest;
        frames
            .into_iter()
            .map(|envelope| match envelope.reply {
                StoreReply::RunStream(frame) => (envelope.origin, envelope.seq, frame),
                _ => unreachable!("partitioned above"),
            })
            .collect()
    }

    /// Blueprint §0a point 3: every frame of an item goes to its subscribers at **their**
    /// subscription's `seq`, whoever asked for the command, and a re-subscription moves it.
    #[tokio::test]
    async fn run_stream_frames_carry_the_subscription_seq() {
        let fixture = Fixture::new().await;
        let mut worker = Worker::spawn(&fixture.store, fixture.runtime());
        let backlog = Origin::Tab(TabId("backlog"));
        let chat = Origin::Tab(TabId("chat"));
        worker.send_at(
            backlog.clone(),
            7,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        worker.send_at(
            chat.clone(),
            9,
            StoreRequest::RunStream {
                item: ids::HTUI_FEAT_1,
            },
        );

        let start = worker.send_at(Origin::App, 10, start_run(ids::HTUI_ANA_2));
        let CommandOutcome::Started { run, .. } = outcome(worker.reply(start).await) else {
            panic!("a start answers Started");
        };
        let acks: Vec<(Origin, u64)> = worker
            .seen
            .iter()
            .filter(|envelope| {
                matches!(&envelope.reply, StoreReply::RunStream(frame)
                if matches!(frame.kind, FrameKind::Subscribed))
            })
            .map(|envelope| (envelope.origin.clone(), envelope.seq))
            .collect();
        assert_eq!(acks, [(backlog.clone(), 7), (chat.clone(), 9)]);

        let first = frames(&mut worker);
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::Started)),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::SessionDone { .. })),
            "{first:?}"
        );
        assert!(
            first
                .iter()
                .any(|(_, _, frame)| matches!(frame.kind, FrameKind::Rested(_))),
            "{first:?}"
        );
        for (origin, seq, frame) in &first {
            assert_eq!((origin, *seq), (&backlog, 7), "{frame:?}");
            assert_eq!(frame.item, ids::HTUI_ANA_2);
        }

        worker.send_at(
            backlog.clone(),
            11,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );
        let research = step_at(&fixture, run, 0).await;
        let answer = worker.send_at(
            Origin::App,
            12,
            StoreRequest::Orch(OrchRequest::Command(Command::AnswerGate {
                run,
                step: research.id,
                answer: GateAnswer::Approved,
            })),
        );
        outcome(worker.reply(answer).await);
        let later = frames(&mut worker);
        assert!(!later.is_empty());
        for (origin, seq, frame) in &later {
            assert_eq!((origin, *seq), (&backlog, 11), "{frame:?}");
        }
    }

    /// D174, PRD `:197`: off the server every command is refused with MOD-25's one sentence and
    /// nothing is spawned, while the stream and the verdicts still answer from the mirror.
    #[tokio::test]
    async fn an_offline_backend_refuses_every_orch_request_with_one_sentence() {
        let root = tempfile::tempdir().expect("a throwaway mirror root");
        let cache = CacheStore::open(root.path(), "run-worker-offline", PgStore::schema_version())
            .await
            .expect("the mirror opens");
        htui_store::testkit::seed_mirror(&cache, &htui_core::fixtures::demo_data())
            .await
            .expect("the mirror is seeded");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: Some(Utc::now()),
        };
        let fixture = Fixture::new().await;
        let mut runtime = fixture.runtime();
        let (replies, _answers) = mpsc::unbounded_channel();
        let serve = async |runtime: &mut RunRuntime, seq: u64, request: StoreRequest| {
            runtime
                .serve(
                    &backend,
                    &replies,
                    &RequestEnvelope {
                        seq,
                        origin: Origin::App,
                        request,
                    },
                    &LiveChats::default(),
                )
                .await
        };

        for (seq, request) in (1..).zip(every_orch_request()) {
            let name = request.name();
            let served = serve(&mut runtime, seq, StoreRequest::Orch(request)).await;
            assert!(
                matches!(&served, RunServed::Reply(StoreReply::Failed { request, message })
                    if *request == name && message == DATABASE_UNREACHABLE),
                "{served:?}"
            );
        }
        assert_eq!(runtime.tasks_len(), 0, "nothing was spawned");
        assert_eq!(runtime.isolator_builds(), 0, "and nothing was built");

        let served = serve(
            &mut runtime,
            20,
            StoreRequest::RunStream {
                item: ids::HTUI_FEAT_1,
            },
        )
        .await;
        assert!(matches!(
            served,
            RunServed::Reply(StoreReply::RunStream(super::RunFrame {
                kind: FrameKind::Subscribed,
                ..
            }))
        ));

        let RunServed::Reply(StoreReply::RunActions(actions)) =
            serve(&mut runtime, 21, StoreRequest::RunActions(ids::HTUI_FEAT_1)).await
        else {
            panic!("the verdicts are read from the mirror");
        };
        let offline = Err(DATABASE_UNREACHABLE.to_owned());
        assert_eq!(actions.item, ids::HTUI_FEAT_1);
        assert_eq!(
            (&actions.run, &actions.unblock, &actions.close_out),
            (&offline, &offline, &offline)
        );
        for verdict in actions.runs.values() {
            assert_eq!((&verdict.cancel, &verdict.cleanup), (&offline, &offline));
        }
        for verdict in actions.steps.values() {
            for enabled in [
                &verdict.approve,
                &verdict.reject,
                &verdict.retry,
                &verdict.promote,
                &verdict.accept,
                &verdict.select,
            ] {
                assert_eq!(enabled, &offline);
            }
        }
        cache.close().await;
    }

    /// D175 (criterion 19 re-scoped): a store that stops answering lease refreshes fences the walk
    /// — its run joins the dead walks with a `LeaseLost` frame — and once the store answers again
    /// the next sweep adopts it and a second attempt rests it.
    #[tokio::test(start_paused = true)]
    async fn a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after() {
        let fixture = Fixture::new().await;
        fixture
            .store
            .set_app_setting("lease_ttl_seconds", json!(30));
        let stall = Stall::default();
        fixture.sessions.push(Play::Stall(stall.clone()));
        let runtime = fixture.runtime();
        let shared = Arc::clone(&runtime.shared);
        let mut worker = Worker::spawn(&fixture.store, runtime);
        let backlog = Origin::Tab(TabId("backlog"));
        worker.send_at(
            backlog,
            1,
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        );

        let start = worker.send(Origin::App, start_run(ids::HTUI_ANA_2));
        within("the session starting", stall.reached.notified()).await;
        let run = only_run(&fixture.store, ids::HTUI_ANA_2).await;
        fixture.store.set_fault(MemFault::RefreshLease, true);

        let StoreReply::Failed { message, .. } = worker
            .envelope_within(start, Duration::from_secs(600))
            .await
            .reply
        else {
            panic!("a fenced walk is refused");
        };
        fixture.store.set_fault(MemFault::RefreshLease, false);
        assert!(message.contains("lease"), "{message}");
        assert!(shared.dead_walks.contains(run), "the run is a dead walk");
        assert!(
            frames(&mut worker).iter().any(
                |(_, _, frame)| matches!(&frame.kind, FrameKind::Error(sentence) if *sentence == message)
            ),
            "the fence is published"
        );
        assert!(stall.dropped.load(Ordering::SeqCst), "the walk was dropped");

        rests_within(
            &fixture,
            run,
            RunStatus::AwaitingApproval,
            Duration::from_secs(600),
        )
        .await;
        assert_eq!(
            step_at(&fixture, run, 0).await.attempt,
            2,
            "the adopted run walked a second attempt"
        );
        assert!(!shared.dead_walks.contains(run));
    }

    /// Plan D155: the five trait reads are the inherent reads of the same name.
    #[tokio::test]
    async fn backend_graphs_delegates_each_read() {
        let (store, agent) = seeded_store().await;
        let backend = Backend::memory(store);
        let graphs = BackendGraphs(backend.clone());

        let resolved = GraphSource::resolve_graph(&graphs, ids::HTUI_ANA_2)
            .await
            .expect("the read answers");
        assert_eq!(
            resolved,
            backend
                .resolve_graph(ids::HTUI_ANA_2)
                .await
                .expect("the read answers")
        );
        let resolved = resolved.expect("the demo item has a graph");
        let phase = resolved.phases.first().expect("the graph has a phase");

        assert_eq!(
            GraphSource::phase_agents(&graphs, phase.phase.id)
                .await
                .expect("the read answers"),
            backend
                .phase_agents(phase.phase.id)
                .await
                .expect("the read answers")
        );
        assert_eq!(
            GraphSource::prompt_template(&graphs, ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers"),
            backend
                .prompt_template(ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers")
        );
        let row = GraphSource::agent(&graphs, agent)
            .await
            .expect("the read answers")
            .expect("the scripted row is registered");
        assert_eq!(row.name, "scripted");
        assert_eq!(
            GraphSource::agent(&graphs, AgentId::new())
                .await
                .expect("the read answers"),
            None,
            "an unknown id is no row"
        );
        let boxes = GraphSource::agent_boxes(&graphs, ids::BOX)
            .await
            .expect("the read answers");
        assert_eq!(
            boxes,
            backend
                .agent_boxes(ids::BOX)
                .await
                .expect("the read answers")
        );
        assert!(boxes.iter().any(|row| row.agent_id == agent));
    }

    /// One request per entry of `ORCH_NAMES`, in its order.
    fn every_orch_request() -> Vec<OrchRequest> {
        let (run, step) = (RunId::new(), StepId::new());
        vec![
            OrchRequest::Command(Command::StartRun {
                item: ids::HTUI_ANA_2,
                mode: RunMode::Manual,
                repo_scope: None,
            }),
            OrchRequest::Command(Command::AnswerGate {
                run,
                step,
                answer: GateAnswer::Approved,
            }),
            OrchRequest::Command(Command::RetryStep { run, step }),
            OrchRequest::Command(Command::CancelRun { run }),
            OrchRequest::Command(Command::SelectFanout {
                run,
                position: 0,
                attempt: 1,
                winner: step,
            }),
            OrchRequest::Command(Command::PromoteStep {
                run,
                step,
                chat_open: false,
            }),
            OrchRequest::Command(Command::AcceptArtifact {
                run,
                step,
                chat_live: false,
            }),
            OrchRequest::Command(Command::Unblock {
                item: ids::HTUI_ANA_2,
            }),
            OrchRequest::Command(Command::CloseOut {
                item: ids::HTUI_ANA_2,
            }),
            OrchRequest::CloseOutPreview {
                item: ids::HTUI_ANA_2,
            },
            OrchRequest::Cleanup { run },
        ]
    }

    /// Blueprint D209, §12: eleven distinct names, each the `name()` of its request, none shared
    /// with another `StoreRequest`.
    #[test]
    fn orch_names_are_eleven_distinct_request_names() {
        let requests = every_orch_request();
        assert_eq!(requests.len(), ORCH_NAMES.len());
        for (request, name) in requests.into_iter().zip(ORCH_NAMES) {
            assert_eq!(StoreRequest::Orch(request).name(), name);
        }
        let distinct: BTreeSet<&str> = ORCH_NAMES.into_iter().collect();
        assert_eq!(distinct.len(), 11);
        for other in [
            StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
            StoreRequest::Document(DocumentId::new()),
            StoreRequest::RunActions(ids::HTUI_ANA_2),
            StoreRequest::Runs(ids::HTUI_ANA_2),
        ] {
            assert!(!distinct.contains(other.name()), "{}", other.name());
        }
    }

    /// Blueprint D183: with no runtime, the stream is acknowledged, the verdicts are read and the
    /// document is served; only a command is refused, by name.
    #[tokio::test]
    async fn a_try_serve_without_a_runtime_answers_the_stream_and_the_actions() {
        let backend = Backend::memory(MemStore::demo());

        let StoreReply::RunStream(frame) = store_worker::serve(
            &backend,
            &StoreRequest::RunStream {
                item: ids::HTUI_ANA_2,
            },
        )
        .await
        else {
            panic!("a subscription is acknowledged");
        };
        assert_eq!(frame.item, ids::HTUI_ANA_2);
        assert!(matches!(frame.kind, FrameKind::Subscribed));

        let StoreReply::RunActions(actions) =
            store_worker::serve(&backend, &StoreRequest::RunActions(ids::HTUI_ANA_2)).await
        else {
            panic!("the verdicts are read");
        };
        assert_eq!(actions.item, ids::HTUI_ANA_2);
        assert_eq!(actions.run, Ok(()), "an open item may start a run");

        let heads = backend
            .documents(ids::HTUI_FEAT_1)
            .await
            .expect("the demo documents");
        let head = heads.first().expect("the demo item has a document");
        let StoreReply::Document(document) =
            store_worker::serve(&backend, &StoreRequest::Document(head.id)).await
        else {
            panic!("the document is read");
        };
        assert_eq!(document.expect("the row exists").id, head.id);

        for request in every_orch_request() {
            let name = request.name();
            let reply = store_worker::serve(&backend, &StoreRequest::Orch(request)).await;
            assert!(
                matches!(&reply, StoreReply::Failed { request, message }
                    if *request == name && message == store_worker::NO_RUN_RUNTIME),
                "{reply:?}"
            );
        }
    }
}
