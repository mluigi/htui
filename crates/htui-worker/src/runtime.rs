//! The run runtime (MOD-4 milestone 6, plan D153; MOD-41 plan D6, D7): every command on a task of
//! its own, serialised per run, supervised, swept.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::AgentSettings;
use htui_agent::driver::{AgentDriver, PermissionPolicy};
use htui_agent::error::DriverError;
use htui_agent::record::{Control, Signal};
use htui_agent::registry::DriverFactory;
use htui_core::model::{
    AgentId, AgentSummary, BoxId, BoxProfile, CancelRequest, Executor, ItemId, RepoId, Run,
    RunCommandId, RunCommandStatus, RunId, RunKind, RunStatus, SnapshotCandidate, UserId,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::secret::SecretSource;
use htui_core::store::{Result as StoreResult, StoreError};
use htui_orch::kill_point::{KillPoint, Site};
use htui_orch::tools::ToolHost;
use htui_orch::{
    Adopted, Clock, Command, CommandOutcome, DeadWalks, DriverFor, Engine, EngineError,
    EngineParts, FirstCandidate, GixIsolator, Isolator, IsolatorConfig, LeaseTimes, Next,
    OpeningPath, RepoCheckout, ResolveError, Rest, Resume, RunFence, RunSecrets, SessionKey,
    ShellVerifier, SystemClock, Tails, UnblockCase, Verifier, cleanup_enabled,
};
use htui_store::{DATABASE_UNREACHABLE, identity};
use serde_json::Value;
use tokio::sync::{Notify, OwnedMutexGuard, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::address::{
    ChatEnd, Promoted, Publish as _, Publisher, ReplySink, RunReply, RunRequest, RunServed,
};
use crate::graphs::HostGraphs;
use crate::views::{
    FrameKind, LiveChats, OrchReply, OrchRequest, ProgressSink, RefusedDriver, RunFrame,
    StepAuthor, Via, actions, chat_free, moves_the_run,
};

// ---------------------------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------------------------

/// Blueprint D202 (R-39): a `StartRun` found the repo map changed while a walk of this process
/// is live, so the isolator cannot be rebuilt under it.
pub const REPOS_MOVED: &str =
    "a repo was added or moved since the first run; wait for the live runs to rest (R-39)";

/// Which process a runtime is (MOD-41 I-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// The TUI's runtime.
    #[default]
    Tui,
    /// `htui worker`'s.
    Worker,
}

impl Role {
    /// I-1: whether this role claims, adopts and sweeps on a box whose executor is `executor`.
    /// [`Executor::Other`] is neither role's (fail closed, plan D9).
    #[must_use]
    pub const fn executes(self, executor: &Executor) -> bool {
        matches!(
            (self, executor),
            (Self::Tui, Executor::Tui) | (Self::Worker, Executor::Worker)
        )
    }

    /// Plan D12: the TUI hands a command's tail back on a `worker` box; everything else walks.
    #[must_use]
    pub const fn tails(self, executor: &Executor) -> Tails {
        if matches!((self, executor), (Self::Tui, Executor::Worker)) {
            Tails::HandBack
        } else {
            Tails::Walk
        }
    }
}

/// Plan D9: the refusal of `R` and the walking commands on a box whose executor this build does
/// not know.
#[must_use]
pub fn unknown_executor(executor: &Executor) -> String {
    format!(
        "box executor `{executor}` is not known to htui {}",
        htui_store::HTUI_VERSION
    )
}

/// MOD-42 plan D12 step 6: a cancel this process cannot apply now; the run's executor does.
pub const CANCEL_REQUESTED: &str = "cancel requested: the run's executor applies it";
/// The same, when a cancel of the run was already pending.
pub const CANCEL_ALREADY_REQUESTED: &str =
    "a cancel is already requested: the run's executor applies it";

/// MOD-42 OQ-3: `p` on a step the box's worker walks. Promotion hands a live session to this
/// TUI's chat, which cannot cross processes.
#[must_use]
pub fn promote_needs_the_walker(run: RunId) -> String {
    format!(
        "the worker on this box is walking run {run}; a live step is promoted only by the process that walks it"
    )
}

/// MOD-42 plan D11, B-19: the window a graceful preempt gives a session's cancel.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// OQ-6: the first delay before a run whose resume failed is resumed again.
const BACKOFF_FIRST: Duration = Duration::from_secs(5);
/// OQ-6: the delay stops doubling here.
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// OQ-6, blueprint B-10: when a run whose resume keeps failing may be resumed again, and the delay
/// that got it there.
#[derive(Debug, Clone, Copy)]
struct Backoff {
    due: tokio::time::Instant,
    delay: Duration,
}

/// OQ-6: a resume that failed to resolve the live graph for a reason other than the store
/// (`NoGraph`, `NoTemplate`, `NoAgentRow`, `EmptyScopeWithPrimary`, …; the refusals the resume
/// handles itself are `resume_window`'s) is retried later, not at every poll.
fn backs_off(err: &EngineError) -> bool {
    matches!(err, EngineError::Resolve(resolve) if !matches!(resolve, ResolveError::Store(_)))
}

/// The `copy_max_total_bytes` a box with no `app_setting` for it copies up to: 20 GiB.
const DEFAULT_COPY_MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024 * 1024;

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
    /// The [`Shared::server`] generation they belong to.
    server: u64,
    repos: Option<BTreeMap<RepoId, RepoCheckout>>,
    isolator: Option<Arc<dyn Isolator>>,
    verifier: Option<Arc<dyn Verifier>>,
    /// MOD-76 D4 (R-55): the stored `command_limits` [`Self::verifier`] was built from (`None`
    /// inside: no key).
    limits: Option<Box<StoredLimits>>,
}

/// The raw `command_limits` layers: the box row's value, then the app setting's.
type StoredLimits = (Option<Value>, Option<Value>);

/// Everything the runtime's tasks share. `P` is where the runtime's answers go (MOD-41 plan D7).
struct Shared<P: ReplySink> {
    /// I-1: which process this runtime is.
    role: Role,
    /// Plan D14, OQ-5: whether a sweep claims the box's queued rows; off only through
    /// [`RunRuntime::without_claim_scan`].
    claim_scan: bool,
    parts: Parts,
    drivers: Arc<DriverFactory>,
    clock: Arc<dyn Clock>,
    author: Option<Arc<dyn StepAuthor>>,
    /// MOD-11 D11: htui's MCP host every engine of this runtime opens its leases on; closed by
    /// [`RunRuntime::shutdown`] (B-19).
    tools: Option<Arc<dyn ToolHost>>,
    /// MOD-10 D15: the process's secret source, handed to every `Kit`'s `RunSecrets`; `None`
    /// refuses provider projects (`secrets_refused`) and leaves the rest untouched.
    secrets: Option<Arc<dyn SecretSource>>,
    owner: Uuid,
    dead_walks: Arc<DeadWalks>,
    publisher: Publisher<P>,
    events: mpsc::UnboundedSender<RunServed<P::Addr>>,
    tasks: StdMutex<Vec<Tracked>>,
    isolator_builds: AtomicUsize,
    /// MOD-76 review L-2: how many verifiers this process has built.
    verifier_builds: AtomicUsize,
    locks: RunLocks,
    walks: Walks,
    /// M5 D84: this process's runs a claim refused, by `queued_at`.
    queued: StdMutex<BTreeSet<(DateTime<Utc>, RunId)>>,
    /// How many tasks have ended: a refused `StartRun` compares it across its claim, so a walk
    /// that rested meanwhile and found the queue empty does not leave the run waiting (M5 D84).
    ended: AtomicU64,
    /// Which server the production parts are for; [`RunRuntime::forget_server`] moves it.
    server: AtomicU64,
    /// D190: the sweep period in milliseconds, the lease TTL until a test fixes it.
    sweep_every: AtomicU64,
    /// Whether [`RunRuntime::with_sweep_every`] fixed the period.
    sweep_fixed: bool,
    /// D190: one sweep at a time.
    sweeping: AtomicBool,
    /// I-1: the executor the last sweep read, so a change is logged once.
    last_executor: StdMutex<Option<Executor>>,
    /// OQ-6, blueprint B-10: the runs whose resume keeps failing (role [`Role::Worker`] only).
    backoff: StdMutex<HashMap<RunId, Backoff>>,
    /// MOD-42 plan D13: one command poll at a time.
    polling: AtomicBool,
    /// MOD-42 B-5: the run commands a task of this process is applying now, so the poll never
    /// applies a row the inline path (or an earlier tick) is still applying.
    applying: StdMutex<HashSet<RunCommandId>>,
    /// MOD-24 D3 (B5): the live chats the last command poll was handed; the sweep's own cancels
    /// honour them as the poll does (D212). Always empty for the worker.
    last_live: StdMutex<LiveChats>,
    /// MOD-24 D3: notified whenever an `Applying` guard drops, so the sweep can await a row the
    /// poll is applying.
    applied: Notify,
    /// MOD-42 plan D11, B-19: the grace a graceful preempt gives a session's cancel.
    cancel_grace: Duration,
}

/// One task of the runtime, with the run it works on once it knows it.
#[derive(Debug)]
struct Tracked {
    tag: Arc<Tag>,
    handle: JoinHandle<()>,
}

/// The run (and item) a task works on, set as soon as the task knows them.
#[derive(Debug, Default)]
pub(crate) struct Tag {
    pub(crate) run: OnceLock<RunId>,
    pub(crate) item: OnceLock<ItemId>,
}

impl<P: ReplySink> Shared<P> {
    /// I-1: the box's executor as a sweep read it, logged once per change: at `info` for the
    /// worker, whose whole job it decides (OQ-2), at `debug` for the TUI.
    fn note_executor(&self, executor: &Executor) {
        let mut last = self
            .last_executor
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.as_ref() == Some(executor) {
            return;
        }
        *last = Some(executor.clone());
        let executes = self.role.executes(executor);
        match self.role {
            Role::Worker => {
                tracing::info!(%executor, executes, "this box's executor; the worker executes only on `worker`");
            }
            Role::Tui => {
                tracing::debug!(%executor, executes, "this box's executor; the TUI executes only on `tui`");
            }
        }
    }

    /// OQ-6: `run`'s resume failed again; its next one waits, 5 s doubling to 5 min. Warned once
    /// per step up.
    fn step_backoff(&self, run: RunId) {
        let mut backoff = self.backoff.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = backoff.get(&run).map(|entry| entry.delay);
        let delay = previous.map_or(BACKOFF_FIRST, |delay| (delay * 2).min(BACKOFF_MAX));
        if previous != Some(delay) {
            tracing::warn!(
                %run,
                delay_secs = delay.as_secs(),
                "a run's resume keeps failing; the worker waits before resuming it again"
            );
        }
        backoff.insert(
            run,
            Backoff {
                due: tokio::time::Instant::now() + delay,
                delay,
            },
        );
    }

    /// OQ-6: `run`'s resume answered; its backoff is over.
    fn clear_backoff(&self, run: RunId) {
        self.backoff
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run);
    }

    /// OQ-6: when `run` may be resumed again, for the worker only; the TUI never backs off.
    fn backoff_due(&self, run: RunId) -> Option<tokio::time::Instant> {
        if self.role != Role::Worker {
            return None;
        }
        self.backoff
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&run)
            .map(|entry| entry.due)
    }

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

    /// Drops the handles of tasks that have ended: nothing reads a finished task's result.
    fn prune_tasks(&self) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|task| !task.handle.is_finished());
    }

    /// D214 (review L5): one lazy pass per sweep tick — the finished task handles, the stale
    /// backoff entries (MOD-41 review R-8), the parent tokens of runs no task works on, and the run
    /// locks nobody holds or waits for — so an idle session keeps none of them for the life of the
    /// process.
    fn prune(&self) {
        self.prune_tasks();
        self.prune_backoff();
        self.walks.prune();
        self.locks.prune();
    }

    /// MOD-41 review R-8: drops the backoff entry of every run no task of this process works on
    /// whose due is past by more than `max(BACKOFF_MAX, 2 x sweep period)`: a run that left the box
    /// (finished elsewhere, cancelled, its lease taken) is never resumed here again, so nothing
    /// would ever clear its entry. A run still failing is adopted within that grace and keeps its
    /// delay.
    fn prune_backoff(&self) {
        let every = Duration::from_millis(self.sweep_every.load(Ordering::SeqCst));
        let grace = BACKOFF_MAX.max(every.saturating_mul(2));
        let now = tokio::time::Instant::now();
        self.backoff
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|run, entry| {
                now.saturating_duration_since(entry.due) <= grace || self.walks.is_live(*run)
            });
    }

    /// The process's isolator and verifier (D156, D202). A `StartRun`, and each sweep of the
    /// worker (MOD-41 review R-1), re-reads the repo map and rebuilds the production isolator and
    /// verifier when it moved and no walk of this process is live, and is refused with
    /// [`REPOS_MOVED`] when one is (R-39). Each such call also re-reads the box's `command_limits`
    /// and rebuilds the verifier when they changed and no walk is live; while one is, the cached
    /// verifier stays and nothing is refused (MOD-76 D4, R-55). A repo-map rebuild reads
    /// `copy_max_total_bytes` afresh.
    async fn singletons<H: htui_core::store::WorkerHost>(
        &self,
        host: &H,
        writer: &H::Store,
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
        // Parts built for a server the session has left are another server's repos and box.
        let server = self.server.load(Ordering::SeqCst);
        if built.server != server {
            *built = Built {
                server,
                ..Built::default()
            };
        }
        if !start_run && let (Some(isolator), Some(verifier)) = (&built.isolator, &built.verifier) {
            return Ok((Arc::clone(isolator), Arc::clone(verifier)));
        }
        let box_id = registered_box(host).await.map_err(|err| err.to_string())?;
        let repos = repo_map(host, writer, box_id)
            .await
            .map_err(|err| err.to_string())?;
        let rebuild = built.repos.as_ref() != Some(&repos);
        if rebuild {
            if built.isolator.is_some() && self.any_live() {
                return Err(REPOS_MOVED.to_owned());
            }
            let app = host.app_settings().await.map_err(|err| err.to_string())?;
            let scratch_root = match scratch_root {
                Some(root) => root.clone(),
                None => identity::config_root()
                    .map_err(|err| err.to_string())?
                    .join("trees"),
            };
            let config = IsolatorConfig {
                repos: repos.clone(),
                scratch_root,
                copy_exclude: Vec::new(),
                copy_max_total_bytes: app
                    .get("copy_max_total_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_COPY_MAX_TOTAL_BYTES),
                box_id,
            };
            // D213 (review L4): `GixIsolator::new` probes `git --version` for up to its bound and
            // creates the scratch root, so it runs on a blocking thread, never on a runtime worker
            // (`R-NF-3`). Nothing is assigned until it answers, so a task aborted meanwhile leaves
            // no half-built parts.
            let isolator = tokio::task::spawn_blocking(move || GixIsolator::new(config))
                .await
                .map_err(|err| format!("the isolator could not be built: {err}"))?
                .map_err(|err| err.to_string())?;
            built.isolator = Some(Arc::new(isolator));
            built.repos = Some(repos);
            // MOD-41 review R-1: the verifier is rebuilt at the same point, from the
            // `command_limits` read below (MOD-76 D4).
            built.verifier = None;
            self.isolator_builds.fetch_add(1, Ordering::SeqCst);
        }
        // MOD-76 D4 (R-55): the limits are read wherever the repo map is, so an edit to them alone
        // rebuilds the verifier. Never under a live walk: two verifiers are two `verify` semaphores
        // (verify.rs `ShellVerifier`), so the swap waits for the next call with none live.
        let stored = stored_limits(host, box_id)
            .await
            .map_err(|err| err.to_string())?;
        if built.limits.as_deref() != Some(&stored) && !self.any_live() {
            built.verifier = None;
        }
        if built.verifier.is_none() {
            let limits = parse_limits(box_id, &stored);
            built.verifier = Some(Arc::new(ShellVerifier::new(
                &limits,
                Arc::new(MinimalScrubber::new(std::iter::empty::<String>())),
                Arc::clone(&self.clock),
            )));
            built.limits = Some(Box::new(stored));
            self.verifier_builds.fetch_add(1, Ordering::SeqCst);
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
    ///
    /// MOD-42 plan D11: a graceful preempt's `Cancel` is a cancel here too. A task queued for the
    /// lock under the signalled parent would otherwise take it the moment the stopping walk
    /// rests, and walk a run its preemptor is waiting to own; it is refused instead, and its
    /// token's drop is what lets the preempt stop waiting.
    async fn lock_unless_cancelled(
        &self,
        run: RunId,
        walk: &WalkToken,
    ) -> Option<OwnedMutexGuard<()>> {
        self.lock_announcing(run, walk, || {}).await
    }

    /// [`Self::lock_unless_cancelled`], calling `waiting` once when the lock is held and the task
    /// has to queue (R-51). A cancelled task is refused first, as ever; a free lock is taken
    /// without announcing anything.
    async fn lock_announcing(
        &self,
        run: RunId,
        walk: &WalkToken,
        waiting: impl FnOnce(),
    ) -> Option<OwnedMutexGuard<()>> {
        let mut signal = walk.signal.clone();
        if walk.token.is_cancelled() || signal.borrow().is_cancel() {
            return None;
        }
        if let Some(guard) = self.locks.try_lock(run) {
            return Some(guard);
        }
        waiting();
        tokio::select! {
            biased;
            () = walk.token.cancelled() => None,
            () = cancel_signalled(&mut signal) => None,
            guard = self.locks.lock(run) => Some(guard),
        }
    }

    /// MOD-42 B-5: `id` claimed for this task until the guard drops; `None` while another task of
    /// this process applies it.
    ///
    /// The set's lock is released before any guard exists: a guard built under it (an eager
    /// `then_some`) and dropped on a refusal would lock the set again in its `Drop`, deadlocking
    /// the thread, and free the holder's id besides.
    fn applying(&self, id: RunCommandId) -> Option<Applying<'_>> {
        let inserted = self
            .applying
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id);
        inserted.then(|| Applying {
            set: &self.applying,
            done: &self.applied,
            id,
        })
    }

    /// MOD-42 B-5: whether a task of this process applies `id` now.
    fn is_applying(&self, id: RunCommandId) -> bool {
        self.applying
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&id)
    }

    /// MOD-24 D3: until no task of this process applies `id` (B-5's guard is free).
    async fn until_applied(&self, id: RunCommandId) {
        loop {
            let mut notified = std::pin::pin!(self.applied.notified());
            // Registered before the check, so a guard dropped in between is never missed.
            notified.as_mut().enable();
            if !self.is_applying(id) {
                return;
            }
            notified.await;
        }
    }
}

/// MOD-42 B-5: one run command a task is applying; dropping it frees the id for the next poll
/// and wakes whoever awaits it (MOD-24 D3).
struct Applying<'a> {
    set: &'a StdMutex<HashSet<RunCommandId>>,
    done: &'a Notify,
    id: RunCommandId,
}

impl Drop for Applying<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
        self.done.notify_waiters();
    }
}

/// Resolves once `signal` reads [`Signal::Cancel`]; pending for ever once its sender is gone (a
/// dropped parent is not a cancel: its token says so).
async fn cancel_signalled(signal: &mut watch::Receiver<Signal>) {
    if signal.wait_for(|signal| signal.is_cancel()).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// R-27: one async mutex per run, minted on first use. Held by every command, resume, claim and
/// sweep-driven recovery of that run for its whole duration (plan D157). An entry nobody holds or
/// waits for is pruned at the next sweep tick (D214), and minted again when next used.
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

    /// D214: drops every entry nobody holds or waits for. A guard and a waiter each own a clone
    /// of the entry, which [`Self::entry`] hands out under this same map lock, so a count of one
    /// that is also free means no task can be serialised by it: removing it keeps the exclusion.
    fn prune(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, lock| Arc::strong_count(lock) > 1 || lock.try_lock().is_err());
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
///
/// Every parent is a child of one root, which `close` cancels when the UI is gone: a parent minted
/// after that is born cancelled, so a task that reaches its run after the shutdown walks nothing.
#[derive(Debug, Clone, Default)]
struct Walks {
    parents: Arc<StdMutex<HashMap<RunId, Parent>>>,
    root: CancellationToken,
}

/// A run's parent token, how many tasks work under it, and its signal (MOD-42 plan D11).
#[derive(Debug, Clone)]
struct Parent {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
    /// D11: `Cancel` asks every walk of the run to stop gracefully before the token drops it.
    signal: Arc<watch::Sender<Signal>>,
    /// The channel's own receiver; the control lookup hands out clones (F-17).
    control: watch::Receiver<Signal>,
}

/// One task's child token; dropping it is the task no longer working on the run.
#[derive(Debug)]
pub struct WalkToken {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
    /// MOD-42 plan D11: the run's signal, which a task queued for the run's lock honours.
    signal: watch::Receiver<Signal>,
}

impl Drop for WalkToken {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Walks {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RunId, Parent>> {
        self.parents.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A child of `run`'s parent, minting the parent on first use; cancelled from birth once the
    /// runtime is closed.
    fn child(&self, run: RunId) -> WalkToken {
        let mut walks = self.lock();
        let parent = walks.entry(run).or_insert_with(|| {
            let (signal, control) = watch::channel(Signal::Run);
            Parent {
                token: self.root.child_token(),
                live: Arc::new(AtomicUsize::new(0)),
                signal: Arc::new(signal),
                control,
            }
        });
        parent.live.fetch_add(1, Ordering::SeqCst);
        WalkToken {
            token: parent.token.child_token(),
            live: Arc::clone(&parent.live),
            signal: parent.control.clone(),
        }
    }

    /// MOD-42 plan D10: the run's control: its live parent's receiver, else one no signal reaches.
    fn control(&self, run: RunId) -> Control {
        self.lock().get(&run).map_or_else(Control::never, |parent| {
            Control::new(parent.control.clone())
        })
    }

    /// MOD-42 plan D11 (R-38): `Cancel { grace }` to every walk of `run`; wait until none is live
    /// or `grace + 1 s`; then [`Self::preempt`] as before (a walk still live is dropped hard). The
    /// parent stays in the map while this waits, so a walk that looks its control up meanwhile
    /// reads the cancel, and a task queued for the run's lock is refused. Whether there was a
    /// parent.
    async fn preempt_gracefully(&self, run: RunId, grace: Duration) -> bool {
        let live = {
            let walks = self.lock();
            let Some(parent) = walks.get(&run) else {
                return false;
            };
            parent.signal.send_replace(Signal::Cancel { grace });
            Arc::clone(&parent.live)
        };
        let deadline = tokio::time::Instant::now() + grace + Duration::from_secs(1);
        while live.load(Ordering::SeqCst) > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.preempt(run)
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

    /// Every run's parent removed and cancelled; the root stays, so later tasks walk again.
    fn preempt_all(&self) {
        for (_, parent) in self.lock().drain() {
            parent.token.cancel();
        }
    }

    /// The UI is gone: the root is cancelled, and with it every run's parent, now and later.
    fn close(&self) {
        self.root.cancel();
        self.lock().clear();
    }

    /// Whether [`Self::close`] ran.
    fn closed(&self) -> bool {
        self.root.is_cancelled()
    }

    /// D214: drops the parent of every run no task works on. [`Self::child`] counts a task in under
    /// this same lock, so none is minted meanwhile, and [`Self::is_live`] and [`Self::any_live`]
    /// already read a parent with no task as absent; the next task of the run mints a fresh one,
    /// a child of the root like the one dropped.
    fn prune(&self) {
        self.lock()
            .retain(|_, parent| parent.live.load(Ordering::SeqCst) > 0);
    }
}

/// This box's id, or the refusal that says it has never been registered.
async fn registered_box<H: htui_core::store::WorkerHost>(host: &H) -> StoreResult<BoxId> {
    Ok(host
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
async fn repo_map<H: htui_core::store::WorkerHost>(
    host: &H,
    writer: &H::Store,
    box_id: BoxId,
) -> StoreResult<BTreeMap<RepoId, RepoCheckout>> {
    let paths: BTreeMap<RepoId, String> = host
        .repo_paths(box_id)
        .await?
        .into_iter()
        .map(|path| (path.repo_id, path.local_path))
        .collect();
    let mut projects = BTreeSet::new();
    for workspace in host.workspaces().await? {
        projects.extend(workspace.projects.iter().map(|project| project.project_id));
    }
    let mut repos = BTreeMap::new();
    for project in projects {
        for repo in htui_core::store::WorkerStore::repos(writer, project).await? {
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

/// MOD-11 D15 and MOD-76 B-1 (R-55): the two raw layers of the box's command limits, the box row's
/// `settings.command_limits` (`None` for no row or no key) and the `app_setting.command_limits`
/// value. [`Shared::singletons`] compares them raw, so a value that does not parse is parsed (and
/// warned) once per build, not once per sweep. The limits are read at every walking `StartRun`
/// and every worker sweep, and the verifier is rebuilt from an edit once no walk of the process is
/// live (MOD-76 D4).
///
/// D216 (review L7): a read that fails is not the default. `singletons` passes it up like the
/// reads beside it, so no verifier is cached from it and the next command reads again.
///
/// [`resolve_command_limits`]: htui_core::model::kind::resolve_command_limits
///
/// # Errors
/// The store's own read failures (the box row, then the app settings).
async fn stored_limits<H: htui_core::store::WorkerHost>(
    host: &H,
    box_id: BoxId,
) -> StoreResult<StoredLimits> {
    let stored = host
        .box_row(box_id)
        .await?
        .and_then(|row| row.settings.get("command_limits").cloned());
    let app = host.app_settings().await?.remove("command_limits");
    Ok((stored, app))
}

/// MOD-76 B-1 (R-55) over MOD-11 D15: `stored` resolved, the box's keys over the app setting's. A
/// box value that does not parse as a map of `u32` is warned.
fn parse_limits(box_id: BoxId, stored: &StoredLimits) -> BTreeMap<String, u32> {
    let (stored, app) = stored;
    if let Some(stored) = stored
        && let Err(err) = serde_json::from_value::<BTreeMap<String, u32>>(stored.clone())
    {
        tracing::warn!(%box_id, %err, "box.settings.command_limits does not parse; the app setting stands where it does not");
    }
    let app: BTreeMap<String, Value> = app
        .iter()
        .map(|value| ("command_limits".to_owned(), value.clone()))
        .collect();
    htui_core::model::kind::resolve_command_limits(stored.as_ref(), &app)
}

/// The engine every task builds, per step of work, over [`Kit`]'s parts: over the host's store
/// (MOD-41 plan D7).
type WorkerEngine<'a, H> = Engine<
    'a,
    <H as htui_core::store::WorkerHost>::Store,
    HostGraphs<H>,
    dyn Isolator,
    dyn Verifier,
    dyn Clock,
    FirstCandidate,
    ProgressSink<<H as htui_core::store::WorkerHost>::Store>,
>;

/// The one sentence for an agent switched off on this box (MOD-23 review L-3, re-review Low-1):
/// `Kit::driver`'s for a step admitted before the switch, and the agent worker's for a chat. The
/// name is only quoted, never branched on (`R-AGT-5`).
#[must_use]
pub fn switched_off(name: &str) -> String {
    format!("agent `{name}` is switched off on this box; Settings > Agents, t switches it on")
}

/// Everything one task's engine borrows, owned (D156): the writer, the graph source, the two
/// singletons, the clock, the sink, the identities, and the agent registry read once per task.
struct Kit<H: htui_core::store::WorkerHost> {
    writer: H::Store,
    graphs: HostGraphs<H>,
    isolator: Arc<dyn Isolator>,
    verifier: Arc<dyn Verifier>,
    clock: Arc<dyn Clock>,
    sink: ProgressSink<H::Store>,
    /// MOD-10 D11: this task's walk's secrets: the engine's scrubber and its env, one object.
    secrets: RunSecrets,
    app: BTreeMap<String, Value>,
    box_profile: BoxProfile,
    box_id: BoxId,
    user: UserId,
    owner: Uuid,
    dead_walks: Arc<DeadWalks>,
    agents: HashMap<AgentId, AgentSummary>,
    drivers: Arc<DriverFactory>,
    /// I-1: which process this is.
    role: Role,
    /// I-1: the box's `settings.executor`, read by [`Executor::of`] alone (plan D9).
    executor: Executor,
    /// Plan D12: [`Role::tails`] over [`Self::executor`].
    tails: Tails,
    /// MOD-42 plan D9: each agent's `agent.settings.permission`, parsed once per task as chat
    /// parses it (`agent_worker.rs:968-969`: a row that does not parse asks).
    policy: Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync>,
    /// MOD-42 plan D10: each run's control: its live `Walks` parent's, else never signalled.
    control: Box<dyn Fn(RunId) -> Control + Send + Sync>,
    /// MOD-11 D11: the runtime's tool host, handed to every engine.
    tools: Option<Arc<dyn ToolHost>>,
}

impl<H: htui_core::store::WorkerHost> Kit<H> {
    /// Reads the parts. `start_run` asks the singletons to re-check the repo map (D202).
    async fn read<P: ReplySink>(
        shared: &Shared<P>,
        host: &H,
        start_run: bool,
    ) -> Result<Self, String> {
        let writer = host
            .writer()
            .ok_or_else(|| DATABASE_UNREACHABLE.to_owned())?;
        let sentence = |err: StoreError| err.to_string();
        let box_id = registered_box(host).await.map_err(sentence)?;
        // I-1 (plan D12): per command and per sweep, the executor key alone (plan D9).
        let executor = host
            .box_row(box_id)
            .await
            .map_err(sentence)?
            .map(|row| Executor::of(&row.settings))
            .ok_or_else(|| "this box has no row".to_owned())?;
        let tails = shared.role.tails(&executor);
        // MOD-41 review R-6: a command whose tail is handed back walks nothing here, so it never
        // rebuilds the isolator (nor is refused with `REPOS_MOVED`).
        let (isolator, verifier) = shared
            .singletons(host, &writer, start_run && tails == Tails::Walk)
            .await?;
        let user = host.this_user().await.map_err(sentence)?;
        let app = host.app_settings().await.map_err(sentence)?;
        let box_profile = host
            .box_profile(box_id)
            .await
            .map_err(sentence)?
            .ok_or_else(|| "this box has no profile row".to_owned())?;
        let agents: HashMap<AgentId, AgentSummary> = host
            .agents()
            .await
            .map_err(sentence)?
            .into_iter()
            .map(|summary| (summary.agent.id, summary))
            .collect();
        let policy = policy_lookup(&agents);
        let walks = shared.walks.clone();
        Ok(Self {
            sink: ProgressSink {
                publisher: Arc::new(shared.publisher.clone()),
                writer: writer.clone(),
                author: shared.author.clone(),
                owner: shared.owner,
            },
            writer,
            graphs: HostGraphs(host.clone()),
            isolator,
            verifier,
            clock: Arc::clone(&shared.clock),
            // MOD-10 D11, D14: one per task, so one per walk. Building it does no I/O; it
            // resolves at the walk's first live path, so a task that walks nothing pays nothing.
            secrets: RunSecrets::new(shared.secrets.clone()),
            app,
            box_profile,
            box_id,
            user,
            owner: shared.owner,
            dead_walks: Arc::clone(&shared.dead_walks),
            agents,
            drivers: Arc::clone(&shared.drivers),
            role: shared.role,
            executor,
            tails,
            policy,
            control: Box::new(move |run| walks.control(run)),
            tools: shared.tools.clone(),
        })
    }

    /// Plan D9: the TUI refuses to start or walk on a box whose executor this build does not
    /// know. `None` for every other role and executor.
    fn walking_refusal(&self) -> Option<String> {
        (self.role == Role::Tui && matches!(self.executor, Executor::Other(_)))
            .then(|| unknown_executor(&self.executor))
    }

    /// [`Self::walking_refusal`] for `command` when it walks: `R`, `a`/`x`, `r`, `s` and `A`.
    /// `u` is refused in its resume case only ([`unblock`]); `c`, `p`, `C` and `T` walk nothing.
    fn refusal(&self, command: &Command) -> Option<String> {
        match command {
            Command::StartRun { .. }
            | Command::AnswerGate { .. }
            | Command::RetryStep { .. }
            | Command::SelectFanout { .. }
            | Command::AcceptArtifact { .. } => self.walking_refusal(),
            _ => None,
        }
    }

    /// The driver for one candidate: the registry row's, else the refusal (D156).
    ///
    /// MOD-23 re-review Low-1: a row switched off on this box is refused here too. Selection
    /// already skips it (its `agent_box.enabled` is false), but a step admitted before the switch —
    /// a pending fan-out member, or one a dead walk admitted and the sweep adopted — is driven from
    /// its own `agent_id`, so this is the one place every drive passes. The refusal fails the step
    /// and the run like any driver that will not start. Offline there is no writer and no walk.
    fn driver(&self, candidate: &SnapshotCandidate) -> Box<dyn AgentDriver> {
        let Some(summary) = self.agents.get(&candidate.agent_id) else {
            return Box::new(RefusedDriver(DriverError::Transport(format!(
                "agent {} is not in the registry",
                candidate.agent_id
            ))));
        };
        if summary.user_off {
            return Box::new(RefusedDriver(DriverError::Spawn(switched_off(
                &summary.agent.name,
            ))));
        }
        match self
            .drivers
            .driver_for(&summary.agent, summary.on_box.as_ref())
        {
            Ok(driver) => driver,
            Err(refusal) => Box::new(RefusedDriver(refusal)),
        }
    }

    /// One engine over these parts.
    fn engine<'a>(&'a self, driver: DriverFor<'a>) -> WorkerEngine<'a, H> {
        Engine::new(EngineParts {
            store: &self.writer,
            graphs: &self.graphs,
            isolator: &*self.isolator,
            verifier: &*self.verifier,
            clock: &*self.clock,
            selector: &FirstCandidate,
            sink: &self.sink,
            driver,
            policy: &*self.policy,
            control: &*self.control,
            scrubber: &self.secrets,
            secrets: &self.secrets,
            app: self.app.clone(),
            box_profile: self.box_profile.clone(),
            box_id: self.box_id,
            owner: self.owner,
            dead_walks: &self.dead_walks,
            user: self.user,
            tails: self.tails,
            tools: self.tools.clone(),
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
/// (D157, D187) — gracefully since MOD-42 (plan D11): the walk's control is signalled first and
/// its session cancelled with grace, and a cancel is a durable `run_command` row its executor
/// applies (D12), which [`Self::poll_commands_with`] picks up (D13).
///
/// MOD-41 plan D7: generic over its host `H` (every store read and write goes through it) and its
/// sink `P` (every answer and frame goes to it). The TUI's is `htui::run_worker::RunRuntime`: over
/// its `Backend` and the store loop's channel, driven through its `TuiRuns`.
pub struct RunRuntime<H, P>
where
    H: htui_core::store::WorkerHost,
    P: ReplySink,
{
    shared: Arc<Shared<P>>,
    events: Option<mpsc::UnboundedReceiver<RunServed<P::Addr>>>,
    host: PhantomData<fn() -> H>,
}

impl<H: htui_core::store::WorkerHost, P: ReplySink> core::fmt::Debug for RunRuntime<H, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RunRuntime")
            .field("parts", &self.shared.parts)
            .field("adapters", &self.shared.drivers.adapter_ids())
            .field("owner", &self.shared.owner)
            .field("tasks", &self.tasks_len())
            .finish_non_exhaustive()
    }
}

impl<H: htui_core::store::WorkerHost, P: ReplySink> RunRuntime<H, P> {
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
                role: Role::Tui,
                claim_scan: true,
                parts,
                drivers: Arc::new(drivers),
                clock: Arc::new(SystemClock),
                author: None,
                tools: None,
                secrets: None,
                owner: Uuid::now_v7(),
                dead_walks: Arc::new(DeadWalks::new()),
                publisher: Publisher::default(),
                events,
                tasks: StdMutex::default(),
                isolator_builds: AtomicUsize::new(0),
                verifier_builds: AtomicUsize::new(0),
                locks: RunLocks::default(),
                walks: Walks::default(),
                queued: StdMutex::default(),
                ended: AtomicU64::new(0),
                server: AtomicU64::new(0),
                sweep_every: AtomicU64::new(millis(lease_period(&BTreeMap::new()))),
                sweep_fixed: false,
                sweeping: AtomicBool::new(false),
                last_executor: StdMutex::default(),
                backoff: StdMutex::default(),
                polling: AtomicBool::new(false),
                applying: StdMutex::default(),
                last_live: StdMutex::default(),
                applied: Notify::new(),
                cancel_grace: CANCEL_GRACE,
            }),
            events: Some(receiver),
            host: PhantomData,
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
    ///
    /// Every tick first prunes what nothing uses any longer (D214): finished task handles, idle
    /// run parents and free run locks.
    ///
    /// MOD-41 blueprint B-3: named `sweep_with`, so it never shadows the TUI's `TuiRuns::sweep`.
    pub fn sweep_with(&mut self, host: &H, sink: &P) {
        self.shared.prune();
        if host.writer().is_none() || self.shared.walks.closed() {
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
        self.shared.publisher.wire(sink);
        let ctx = TaskCtx {
            shared: Arc::clone(&self.shared),
            host: host.clone(),
            sink: sink.clone(),
            addr: None,
            name: "sweep",
            tag: Arc::default(),
        };
        let shared = Arc::clone(&self.shared);
        let tag = Arc::clone(&ctx.tag);
        let handle = tokio::spawn(async move {
            /// Frees the one-sweep-at-a-time claim however the sweep ends.
            struct Swept<P: ReplySink>(Arc<Shared<P>>);
            impl<P: ReplySink> Drop for Swept<P> {
                fn drop(&mut self) {
                    self.0.sweeping.store(false, Ordering::SeqCst);
                }
            }
            let _swept = Swept(Arc::clone(&ctx.shared));
            sweep_once(ctx).await;
        });
        shared.track(tag, handle);
    }

    /// MOD-42 plan D13 (B-1, B-10): one command poll, the `sweep_with` shape: sync, one tracked
    /// task under an in-flight flag so ticks never overlap; nothing without a writer or after
    /// close. Each pending row this process may apply (`pending_commands(owner, box)`), not
    /// already being applied (B-5), runs as an internal cancel (D12 steps 3-5, no second row).
    /// `live` is the loop's [`LiveChats`]: a cancel a live chat refuses stays pending, silently.
    pub fn poll_commands_with(&mut self, host: &H, sink: &P, live: LiveChats) {
        // H-17: the worker runs no chats, so its loop hands an empty set (`worker.rs`), and the
        // sweep's cancels it stores below refuse nothing there.
        debug_assert!(
            self.shared.role != Role::Worker || live.is_empty(),
            "a worker runtime is polled with live chats"
        );
        // MOD-24 D3 (B5): every tick, even one the in-flight check skips, so the sweep's cancels
        // refuse what this poll would.
        *self
            .shared
            .last_live
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = live.clone();
        // Every tick, like a sweep's: the poll's own finished handles never pile up between the
        // sweeps (the TUI's sweep period is the lease TTL).
        self.shared.prune_tasks();
        if host.writer().is_none() || self.shared.walks.closed() {
            return;
        }
        if self
            .shared
            .polling
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.shared.publisher.wire(sink);
        let ctx = TaskCtx {
            shared: Arc::clone(&self.shared),
            host: host.clone(),
            sink: sink.clone(),
            addr: None,
            name: "cancel_run",
            tag: Arc::default(),
        };
        let shared = Arc::clone(&self.shared);
        let tag = Arc::clone(&ctx.tag);
        let handle = tokio::spawn(async move {
            /// Frees the one-poll-at-a-time claim however the poll ends.
            struct Polled<P: ReplySink>(Arc<Shared<P>>);
            impl<P: ReplySink> Drop for Polled<P> {
                fn drop(&mut self) {
                    self.0.polling.store(false, Ordering::SeqCst);
                }
            }
            let _polled = Polled(Arc::clone(&ctx.shared));
            poll_once(ctx, live).await;
        });
        shared.track(tag, handle);
    }

    /// B-19: the grace [`CANCEL_GRACE`] stands for, fixed by a test.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_cancel_grace(mut self, grace: Duration) -> Self {
        self.configure().cancel_grace = grace;
        self
    }

    /// The shared state, while nothing else holds it: configuration happens before the first
    /// request.
    fn configure(&mut self) -> &mut Shared<P> {
        Arc::get_mut(&mut self.shared).expect("a run runtime is configured before it serves")
    }

    /// MOD-41 I-1: a runtime that is `role`'s. A new runtime is [`Role::Tui`].
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_role(mut self, role: Role) -> Self {
        self.configure().role = role;
        self
    }

    /// A runtime whose sweeps never claim the box's queued rows (plan D14 and OQ-5 off); it still
    /// adopts. For `htui --demo` (MOD-41 finding T9-V1): the demo's seeded `queued` run is a
    /// showcase, and claiming it would walk it with the production parts.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn without_claim_scan(mut self) -> Self {
        self.configure().claim_scan = false;
        self
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

    /// MOD-11 D11: a runtime whose engines open one tool lease per session on `tools` (the
    /// worker's `McpHost<PgStore>`, the TUI's `McpHost<Backend>`). [`shutdown`](Self::shutdown)
    /// closes it.
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_tool_host(mut self, tools: Arc<dyn ToolHost>) -> Self {
        self.configure().tools = Some(tools);
        self
    }

    /// MOD-10 D15: a runtime whose walks resolve provider projects' secrets through `source`
    /// (`htui`'s `KeyringInfisical`, one per process, shared with the agent runtime).
    ///
    /// # Panics
    /// When called after the runtime has served.
    #[must_use]
    pub fn with_secret_source(mut self, source: Arc<dyn SecretSource>) -> Self {
        self.configure().secrets = Some(source);
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
    pub fn take_events(&mut self) -> mpsc::UnboundedReceiver<RunServed<P::Addr>> {
        self.events.take().unwrap_or_else(|| {
            let (events, receiver) = mpsc::unbounded_channel();
            self.configure().events = events;
            receiver
        })
    }

    /// The session moved to another server (`SetDsn`, MOD-15 D11 step 6).
    ///
    /// A walk belongs to the server it was claimed on: every walk of this process is preempted,
    /// giving its lease back to that server through `abandoned` (whose next sweep, in any
    /// process, adopts the run), so no walk keeps writing to a database the session has left or
    /// keeps its pool open. The production isolator and verifier, built from the old server's
    /// repo map and box, and the claim queue, the old server's runs, are forgotten: the new
    /// server's first command builds its own parts, with no `REPOS_MOVED` refusal on the way.
    pub fn forget_server(&mut self) {
        self.shared.server.fetch_add(1, Ordering::SeqCst);
        self.shared.walks.preempt_all();
        self.shared
            .queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// MOD-37 M4 D3 (R-46): every live walk of this process preempted at once, without
    /// forgetting the server: the isolator, the verifier and the claim queue stay (unlike
    /// [`Self::forget_server`]). The TUI's store loop calls it when its refresher notices the
    /// backend drop `Online → Offline`. Each walk ends through `walked`'s `None` → `abandoned` → a
    /// release that fails offline → the dead-walk set, and the next `Online` sweep adopts the run
    /// (D175, D190).
    pub fn preempt_walks(&self) {
        self.shared.walks.preempt_all();
    }

    /// How many isolators this process has built (D156's test hook).
    #[must_use]
    pub fn isolator_builds(&self) -> usize {
        self.shared.isolator_builds.load(Ordering::SeqCst)
    }

    /// How many verifiers this process has built (MOD-76 review L-2's test hook): one per build
    /// of the parts, and one more per `command_limits` edit applied (D4).
    #[must_use]
    pub fn verifier_builds(&self) -> usize {
        self.shared.verifier_builds.load(Ordering::SeqCst)
    }

    /// How many tasks this runtime still owns, finished ones included until the next `serve` or
    /// sweep tick prunes them (D214).
    #[must_use]
    pub fn tasks_len(&self) -> usize {
        self.shared
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Serves one `Orch`, `RunStream` or `RunActions` request (§8.5), answered at `addr`. Awaits
    /// nothing: every command, and every verdict read (D215), is a task that answers at `addr`.
    ///
    /// MOD-41 blueprint B-3: named `serve_request`, so it never shadows the TUI's `TuiRuns::serve`.
    pub async fn serve_request(
        &mut self,
        host: &H,
        sink: &P,
        addr: P::Addr,
        request: RunRequest,
        live: &LiveChats,
    ) -> RunServed<P::Addr> {
        self.shared.prune_tasks();
        self.shared.publisher.wire(sink);
        let name = request.name();
        match request {
            RunRequest::Stream { item } => {
                self.shared.publisher.subscribe(addr, item);
                RunServed::Reply(RunReply::Frame(RunFrame::subscribed(item)))
            }
            // D215: the verdicts read the item, its documents, its runs and each run's row and
            // steps, so they are a tracked task like a command and answer at the request's own
            // `seq`; `App::is_fresh` drops a reply a later one overtook.
            RunRequest::Actions(item) => {
                let (host, sink, live) = (host.clone(), sink.clone(), live.clone());
                let handle = tokio::spawn(async move {
                    let reply = match actions(&host, item, &live).await {
                        Ok(actions) => RunReply::Actions(Box::new(actions)),
                        Err(err) => RunReply::Failed {
                            request: name,
                            message: err.to_string(),
                        },
                    };
                    sink.send(&addr, reply);
                });
                self.shared.track(Arc::default(), handle);
                RunServed::Deferred
            }
            RunRequest::Orch(mut request) => {
                // D174: nothing is built and nothing is spawned without a server.
                if host.writer().is_none() {
                    return RunServed::Reply(RunReply::Failed {
                        request: name,
                        message: DATABASE_UNREACHABLE.to_owned(),
                    });
                }
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
                    host: host.clone(),
                    sink: sink.clone(),
                    addr: Some(addr),
                    name,
                    tag: Arc::default(),
                };
                spawn_task(ctx, request, live.clone());
                RunServed::Deferred
            }
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

    /// The UI is gone: the runtime closes, so no walk, task or sweep starts after this; every
    /// walk is cancelled — its lease given back through `abandoned` — and every task, including
    /// one spawned while this waits, is awaited within **one** shared window of `2 × grace`, then
    /// aborted. The loop cancels the chats beside this, inside the same bounded quit. Then the
    /// tool host is closed (MOD-11 B-19): its listener and socket go, and every session ends. A
    /// host shared with the chat runtime (the TUI) is handed in behind a view whose `close` is a
    /// no-op, and its owner closes it after both shutdowns (T6 ADV-2).
    pub async fn shutdown(&mut self, grace: Duration) {
        self.shared.walks.close();
        let deadline = tokio::time::Instant::now() + grace * 2;
        loop {
            let tasks = std::mem::take(
                &mut *self
                    .shared
                    .tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
            if tasks.is_empty() {
                break;
            }
            for Tracked { tag, handle } in tasks {
                let abort = handle.abort_handle();
                if tokio::time::timeout_at(deadline, handle).await.is_err() {
                    abort.abort();
                    tracing::warn!(run = ?tag.run.get(), "a run task did not end within the grace window");
                }
            }
        }
        if let Some(tools) = &self.shared.tools {
            tools.close();
        }
    }
}

/// One task's context: what it answers through and what it works on (MOD-41 plan D7: its host
/// and its sink).
#[derive(Clone)]
pub struct TaskCtx<H: htui_core::store::WorkerHost, P: ReplySink> {
    shared: Arc<Shared<P>>,
    host: H,
    sink: P,
    /// The request the task answers; `None` for a sweep's resume or a claim retry, which answer
    /// nobody and only publish.
    addr: Option<P::Addr>,
    name: &'static str,
    tag: Arc<Tag>,
}

/// MOD-41 blueprint F-12: hand-written, so no `H: Debug` or `P: Debug` is asked; `testing` makes
/// the type reachable, and `missing_debug_implementations` then asks for one.
impl<H: htui_core::store::WorkerHost, P: ReplySink> core::fmt::Debug for TaskCtx<H, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TaskCtx")
            .field("addr", &self.addr)
            .field("name", &self.name)
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

impl<H: htui_core::store::WorkerHost, P: ReplySink> TaskCtx<H, P> {
    /// The one answer to the request.
    fn answer(&self, reply: RunReply) {
        if let Some(addr) = &self.addr {
            self.sink.send(addr, reply);
        }
    }

    /// A context for a task of this runtime's own: it answers nobody.
    fn unaddressed(&self, name: &'static str) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            host: self.host.clone(),
            sink: self.sink.clone(),
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
        self.answer(RunReply::Failed {
            request: self.name,
            message,
        });
    }

    /// MOD-42 B-12 (D12 step 6): a cancel left for the run's executor. A `Changed` frame for the
    /// run, so every watching pane re-reads its pending cancels, and a `Failed` answer, so the
    /// sentence reaches the status line.
    fn requested(&self, already: bool) {
        self.publish(self.tag.run.get().copied(), FrameKind::Changed);
        let message = if already {
            CANCEL_ALREADY_REQUESTED
        } else {
            CANCEL_REQUESTED
        };
        self.answer(RunReply::Failed {
            request: self.name,
            message: message.to_owned(),
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

    /// A command's outcome: the answer, then a `Rested` frame when the walk rested, and a
    /// `Changed` one when nothing walked (D200).
    fn done(&self, outcome: CommandOutcome) {
        let kind = rest_of(&outcome).map_or(FrameKind::Changed, FrameKind::Rested);
        self.publish(self.tag.run.get().copied(), kind);
        self.answer(RunReply::Orch(OrchReply::Done(Box::new(outcome))));
    }

    /// Records the run (and, from its row, the item) the task works on.
    async fn tag_run(&self, writer: &H::Store, run: RunId) -> Option<Run> {
        let _ = self.tag.run.set(run);
        let row = htui_core::store::WorkerStore::run(writer, run)
            .await
            .ok()
            .flatten();
        self.tag_item(row.as_ref());
        row
    }

    /// [`Self::tag_run`] through the task's host, before the task waits for its lock or reads
    /// its parts, so a refusal that comes before either still reaches the item's subscribers
    /// (D200).
    async fn tag(&self, run: RunId) -> Option<Run> {
        let _ = self.tag.run.set(run);
        let row = self.host.run(run).await.ok().flatten();
        self.tag_item(row.as_ref());
        row
    }

    fn tag_item(&self, row: Option<&Run>) {
        if let Some(item) = row.and_then(|row| row.item_id) {
            let _ = self.tag.item.set(item);
        }
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

/// Spawns one request's task, supervised and tracked. `live` is the loop's [`LiveChats`] when the
/// request was served (D212).
fn spawn_task<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    request: OrchRequest,
    live: LiveChats,
) {
    let work = run_request(ctx.clone(), request, live);
    spawn_supervised(ctx, work);
}

/// D158: every task runs inside a supervisor. A task that panicked has its run marked dead — the
/// next sweep releases and adopts it (R-12) — and its request answered once with
/// [`WALK_PANICKED`]. Whatever the end, this process's refused claims are then retried when the
/// task's run no longer walks (M5 D84).
///
/// Only the supervisor is tracked, and dropping a `JoinHandle` detaches its task rather than
/// cancelling it: the supervisor therefore aborts the work when it is itself aborted, so
/// `settle` and `shutdown` stop the walk and not only the task watching it.
fn spawn_supervised<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    work: impl Future<Output = ()> + Send + 'static,
) {
    /// Aborts the work's task when the supervisor is dropped; a no-op once the work has ended.
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    // The UI is gone: nothing new starts, and the request, if any, is answered once.
    if ctx.shared.walks.closed() {
        return ctx.refuse(PREEMPTED.to_owned());
    }
    let shared = Arc::clone(&ctx.shared);
    let tag = Arc::clone(&ctx.tag);
    let handle = tokio::spawn(async move {
        let inner = tokio::spawn(work);
        let _abort = AbortOnDrop(inner.abort_handle());
        if let Err(err) = inner.await
            && err.is_panic()
        {
            if let Some(run) = ctx.tag.run.get() {
                ctx.shared.dead_walks.mark(*run);
                // MOD-41 review R-2 (OQ-6): a panic is a failed resume too, so the resume the next
                // sweep's adoption spawns waits out the run's backoff.
                if ctx.shared.role == Role::Worker {
                    ctx.shared.step_backoff(*run);
                }
            }
            tracing::error!(run = ?ctx.tag.run.get(), "a run task panicked; the next sweep adopts its run");
            ctx.refuse(WALK_PANICKED.to_owned());
        }
        // Before the queue is looked at, so a claim refused meanwhile sees the count move.
        ctx.shared.ended.fetch_add(1, Ordering::SeqCst);
        retry_claims(&ctx).await;
    });
    shared.track(tag, handle);
}

/// M5 D84: once a task's run no longer walks — parked, finished, or gone — every run a refused
/// claim left `queued` in this process is claimed again ([`claim_queued`]).
async fn retry_claims<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) {
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
    // Only a run read to be resting frees anything. A failed read proves nothing, and treating it
    // as a rest would have a queued run's own failed retry retrigger itself for a whole outage.
    match ctx.host.run(run).await {
        Ok(Some(Run {
            status: RunStatus::Running | RunStatus::Queued,
            ..
        }))
        | Err(_) => return,
        Ok(_) => {}
    }
    claim_queued(ctx).await;
}

/// How often [`claim_queued`] reads whether a claim it is waiting on has admitted its run.
const CLAIM_POLL: Duration = Duration::from_millis(20);

/// M5 D84, blueprint §8.6: every run a refused claim left `queued` in this process, claimed again
/// in `queued_at` order, each on a task of its own under its lock. A claim is decided — its task
/// ended, or its run read to be no longer `queued` — before the next one is tried, so an earlier
/// run is never overtaken for the scope a rested walk freed; the admitted run walks on its own
/// task. `Admitted` or a terminal run leaves the set; a second refusal puts it back.
async fn claim_queued<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) {
    let queued = std::mem::take(
        &mut *ctx
            .shared
            .queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    for (queued_at, run) in queued {
        let (decided, undecided) = oneshot::channel::<()>();
        let retry = ctx.unaddressed("claim_retry");
        spawn_supervised(retry.clone(), reclaim(retry, run, queued_at, decided));
        tokio::select! {
            _ = undecided => {}
            () = left_the_queue(&ctx.host, run) => {}
        }
    }
}

/// Returns once `run` is read to be anything but `queued`, or cannot be read.
async fn left_the_queue<H: htui_core::store::WorkerHost>(host: &H, run: RunId) {
    while let Ok(Some(Run {
        status: RunStatus::Queued,
        ..
    })) = host.run(run).await
    {
        tokio::time::sleep(CLAIM_POLL).await;
    }
}

/// One claim retry: the run's lock, then `claim` and its walk. Dropping `decided` — when the task
/// ends — tells [`claim_queued`] this claim no longer holds up the next.
async fn reclaim<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    run: RunId,
    queued_at: DateTime<Utc>,
    decided: oneshot::Sender<()>,
) {
    let _decided = decided;
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return;
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => {
            ctx.shared.queue(queued_at, run);
            return ctx.refuse(message);
        }
    };
    // Blueprint B-7 (F-17): the claim queue claims too, so it honours I-1. The run leaves the
    // queue; the executing process's claim scan finds it.
    if !kit.role.executes(&kit.executor) {
        tracing::debug!(%run, "not this process's to claim on this box (I-1)");
        return;
    }
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
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        Some(Err(EngineError::Cancelled { .. })) => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Err(err @ EngineError::ClaimRefused { .. })) => {
            ctx.shared.queue(queued_at, run);
            tracing::debug!(%run, %err, "a queued run's claim was refused again");
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// D158, D189: one sweep. I-1 (plan D12, D13): only the process whose role matches the box's
/// executor adopts or claims, so the executor is read first, at every sweep. The worker, which has
/// no `StartRun` of its own, then re-reads its repo map (MOD-41 review R-1, blueprint D202): a
/// repo or checkout change rebuilds the isolator and the verifier, with the limits read afresh,
/// when no walk of this process is live (a change to the limits alone rebuilds the verifier then
/// too, MOD-76 D4), and while one is ([`REPOS_MOVED`]) the sweep still adopts
/// but skips its claim scan this tick (a refused claim's retry, M5 D84, still runs when a walk
/// rests, with the parts the process has). Then the adoption
/// (nothing is built when there is nothing to adopt: no dead walk of this process and no run
/// holding a slot on this box), then the claim scan, always unless the runtime was built
/// [`RunRuntime::without_claim_scan`] (blueprint B-8: `queued` rows hold no slot, so the
/// adoption's short-circuit must not skip them).
async fn sweep_once<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>) {
    let host = &ctx.host;
    if !ctx.shared.sweep_fixed
        && let Ok(app) = host.app_settings().await
    {
        ctx.shared
            .sweep_every
            .store(millis(lease_period(&app)), Ordering::SeqCst);
    }
    let Ok(box_id) = registered_box(host).await else {
        return;
    };
    let executor = match host.box_row(box_id).await {
        Ok(Some(row)) => Executor::of(&row.settings),
        Ok(None) => return,
        Err(err) => {
            tracing::debug!(%err, "the sweep could not read this box's executor");
            return;
        }
    };
    ctx.shared.note_executor(&executor);
    if !ctx.shared.role.executes(&executor) {
        return;
    }
    // MOD-41 review R-1: the sweep task holds no `WalkToken`, so `any_live` reads real walks only.
    // Never through `reclaim`: it mints its run's token first, so every claim would be refused.
    let mut claims = ctx.shared.claim_scan;
    if ctx.shared.role == Role::Worker
        && let Some(writer) = host.writer()
    {
        match ctx.shared.singletons(host, &writer, true).await {
            Ok(_) => {}
            // D202 (R-39): the isolator is never swapped under a live walk, so the claim scan
            // waits for a sweep with no walk live. A claim this process had refused (M5 D84) is
            // still retried when one of its walks rests, with the parts it has.
            Err(message) if message == REPOS_MOVED => {
                tracing::debug!(%message, "the sweep claims nothing this tick");
                claims = false;
            }
            Err(message) => {
                tracing::debug!(%message, "the sweep could not refresh its parts");
            }
        }
    }
    if !ctx.shared.dead_walks.runs().is_empty()
        || !matches!(host.active_runs_on_box(box_id).await, Ok(0))
    {
        adopt(&ctx).await;
    }
    if claims {
        claim_scan(&ctx, box_id).await;
    }
}

/// D158, D189: the sweep's adoption: first the pending cancels of free runs (MOD-24 D3), then
/// every lapsed lease on the box adopted, and each run that owes a walk resumed on its own task.
/// A tick whose pending cancels could not be read adopts nothing.
async fn adopt<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) {
    // Before `Kit::read`: `cancel_run` builds its own kit, and a failed read here must not skip
    // the cancels. Their own read failing skips this tick's recovery instead: a cancel it could
    // not see may be pending, and the next tick tries again.
    if !cancels_first(ctx).await {
        tracing::debug!("the sweep skips its recovery this tick: the pending cancels went unread");
        return;
    }
    let host = &ctx.host;
    let kit = match Kit::read(&ctx.shared, host, false).await {
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
        let item = htui_core::store::WorkerStore::run(&kit.writer, run)
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

/// MOD-24 D3 (OQ-2): before the sweep recovers anything, every pending cancel this process may
/// apply (`pending_commands(owner, box)`, B-4) whose run is `running` and not walked here is
/// applied, through the poll's own `cancel_run`, and then awaited under B-5's guard: a row the
/// poll took first is waited for, never applied twice. A row the poll holds is awaited before the
/// walked-run skip (H-16): the poll's `cancel_run` mints a walk child on the run before it awaits
/// the run lock, so its run reads as walked here. Such a run is cancelled without recovery
/// (`cancel_leased` takes the lapsed lease) instead of being walked on, or finished by the
/// recovery itself, which used to lose the user's cancel to the crash.
///
/// Only a graph run (`kind == RunKind::Graph`) that reads `running` is applied: a chat run is
/// never adopted by the sweep (D3b), so its cancel has no recovery to beat and stays the poll's.
/// Left to the poll too: a run this process walks (a graceful preempt), and parked or terminal
/// runs (the sweep never adopts them, so nothing races them). A run whose lease is live elsewhere
/// is not in `pending_commands` at all. Failures are logged as the poll's are: the pending rows'
/// read at `warn`, a run's read and a refused cancel at `debug` (the row stays pending for the
/// poll, and recovery proceeds as before). The live chats are the last poll's (B5, D212), so a
/// cancel the poll would refuse is refused here too.
///
/// Answers whether the pending rows were read: when they were not (no writer, this box's row or
/// the rows themselves unreadable), a pending cancel may be waiting unseen, and the caller skips
/// this tick's recovery rather than recover a run whose cancel it could not see.
async fn cancels_first<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) -> bool {
    let Some(writer) = ctx.host.writer() else {
        return false;
    };
    let box_id = match registered_box(&ctx.host).await {
        Ok(box_id) => box_id,
        Err(err) => {
            tracing::debug!(%err, "the sweep could not read this box for its cancels");
            return false;
        }
    };
    let rows =
        match htui_core::store::WorkerStore::pending_commands(&writer, ctx.shared.owner, box_id)
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "the sweep could not read the pending run commands");
                return false;
            }
        };
    let live = ctx
        .shared
        .last_live
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for row in rows {
        // H-16: the poll's `cancel_run` takes B-5's guard and then a walk child on the run before
        // it awaits the run lock, so a row it holds reads as a walked run's below. Waited for
        // first: skipped, `sweep_fenced` could recover the run before the poll's cancel lands.
        if ctx.shared.is_applying(row.id) {
            ctx.shared.until_applied(row.id).await;
            continue;
        }
        if ctx.shared.walks.is_live(row.run_id) {
            continue;
        }
        let run = match htui_core::store::WorkerStore::run(&writer, row.run_id).await {
            Ok(Some(run)) => run,
            Ok(None) => continue,
            Err(err) => {
                tracing::debug!(
                    run = %row.run_id,
                    %err,
                    "the sweep could not read a pending cancel's run; the poll retries it"
                );
                continue;
            }
        };
        if run.kind != RunKind::Graph || run.status != RunStatus::Running {
            continue;
        }
        Box::pin(cancel_run(
            ctx.unaddressed("cancel_run"),
            row.run_id,
            &live,
            Some(row.id),
        ))
        .await;
        // B-5: if the poll took the row first, `cancel_run` returned at once; its application
        // ends here.
        ctx.shared.until_applied(row.id).await;
    }
    true
}

/// Plan D14, OQ-5: this box's queued rows join the claim queue in `(queued_at, id)` order and are
/// claimed through [`claim_queued`], each under its run lock, whichever process queued them. A
/// run this process is already working on (its own `StartRun` between `enqueue` and the lock, or
/// a claim retry) is skipped (blueprint B-9); what is left of that race is a wrong refusal
/// sentence on a TUI box, never a second claim. A row refused again (`SlotFull`, `Overlaps`) is
/// tried at every sweep (F-36, logged at `debug`).
async fn claim_scan<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: &TaskCtx<H, P>,
    box_id: BoxId,
) {
    let queued = match ctx.host.queued_runs_on_box(box_id).await {
        Ok(queued) => queued,
        Err(err) => {
            tracing::debug!(%err, "the claim scan could not list queued runs");
            return;
        }
    };
    let mut fed = false;
    for (run, queued_at) in queued {
        if ctx.shared.walks.is_live(run) {
            continue;
        }
        ctx.shared.queue(queued_at, run);
        fed = true;
    }
    if fed {
        claim_queued(ctx).await;
    }
}

/// D158: an adopted run's walk, resumed on its own task under its lock.
async fn resumed<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId) {
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
    // OQ-6, blueprint B-10: a run whose resume keeps failing is adopted as any other (a sweep
    // never re-adopts a lease this process owns, plan D88), and its resume waits here.
    if let Some(due) = ctx.shared.backoff_due(run) {
        tokio::select! {
            biased;
            () = walk.token.cancelled() => {
                // Shutdown while waiting: the adopted lease is this owner's; give it back.
                if let Some(store) = ctx.host.writer()
                    && let Err(err) =
                        htui_core::store::WorkerStore::release_lease(&store, run, ctx.shared.owner)
                            .await
                {
                    tracing::warn!(%run, %err, "releasing a backed-off run's lease failed; it lapses");
                }
                return;
            }
            () = tokio::time::sleep_until(due) => {}
        }
    }
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return;
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => {
            // The sweep made this process the run's lease owner, and plan D88 keeps every later
            // sweep of it off that lease: only the dead-walk pre-pass gives it back to adopt.
            ctx.shared.dead_walks.mark(run);
            return ctx.refuse(message);
        }
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    match walked(&walk, engine.resume(run)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(Resume::Walked(rest) | Resume::TopologyChanged { rest, .. })) => {
            ctx.shared.clear_backoff(run);
            ctx.publish(Some(run), FrameKind::Rested(rest));
        }
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        Some(Err(EngineError::Cancelled { .. })) => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Err(err)) => {
            if ctx.shared.role == Role::Worker && backs_off(&err) {
                ctx.shared.step_backoff(run);
            }
            ctx.refuse(err.to_string());
        }
    }
    drop(guard);
}

/// §8.6: one request, start to answer.
async fn run_request<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    request: OrchRequest,
    live: LiveChats,
) {
    match request {
        OrchRequest::Command(Command::StartRun {
            item,
            mode,
            repo_scope,
        }) => start_run(ctx, item, mode, repo_scope).await,
        OrchRequest::Command(Command::Unblock { item }) => unblock(ctx, item).await,
        OrchRequest::Command(command @ Command::CloseOut { item, .. }) => {
            let _ = ctx.tag.item.set(item);
            let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
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
        // MOD-42 B-20: every cancel of a live run is a durable command (D12).
        OrchRequest::Command(Command::CancelRun { run }) => {
            cancel_run(ctx, run, &live, None).await;
        }
        OrchRequest::Command(command) => {
            let (run, preempt) = match &command {
                Command::PromoteStep { run, .. } => (*run, Preempt::IfLive),
                Command::AnswerGate { run, .. }
                | Command::RetryStep { run, .. }
                | Command::SelectFanout { run, .. }
                | Command::AcceptArtifact { run, .. } => (*run, Preempt::Never),
                Command::StartRun { .. }
                | Command::Unblock { .. }
                | Command::CloseOut { .. }
                | Command::CancelRun { .. } => {
                    unreachable!("matched above")
                }
            };
            on_run(ctx, run, command, preempt, &live).await;
        }
        OrchRequest::CloseOutPreview { item } => {
            let _ = ctx.tag.item.set(item);
            let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
                Ok(kit) => kit,
                Err(message) => return ctx.refuse(message),
            };
            let driver =
                |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
            match kit.engine(&driver).close_out_preview(item).await {
                Ok(preview) => ctx.answer(RunReply::Orch(OrchReply::CloseOutPreview(Box::new(
                    preview,
                )))),
                Err(err) => ctx.answer(RunReply::Failed {
                    request: ctx.name,
                    message: err.to_string(),
                }),
            }
        }
        OrchRequest::Cleanup { run } => cleanup(ctx, run).await,
    }
}

/// Whether a command stops a live walk of its run before it waits for the lock (D157, D187).
/// MOD-42 plan D11: the stop is graceful ([`Walks::preempt_gracefully`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preempt {
    /// `PromoteStep`, and `CancelRun` of a queued run (B-20): only a walk of this process that
    /// is live.
    IfLive,
    /// Every other verb waits for the walk to rest.
    Never,
}

/// `StartRun` (D186): enqueue, then — under the run's lock and token — claim and walk.
async fn start_run<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    item: ItemId,
    mode: htui_core::model::RunMode,
    repo_scope: Option<Vec<RepoId>>,
) {
    let _ = ctx.tag.item.set(item);
    let kit = match Kit::read(&ctx.shared, &ctx.host, true).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    if let Some(refusal) = kit.walking_refusal() {
        return ctx.refuse(refusal);
    }
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    // The UI went while the parts were read: no run is enqueued for nobody to walk.
    if ctx.shared.walks.closed() {
        return ctx.refuse(PREEMPTED.to_owned());
    }
    let run = match engine.enqueue(item, mode, repo_scope).await {
        Ok(run) => run,
        Err(err) => return ctx.refuse(err.to_string()),
    };
    // Blueprint B-9: the run is this task's before anything awaits, so a claim scan that read it
    // meanwhile skips it.
    let walk = ctx.shared.walks.child(run);
    let _ = ctx.tag.run.set(run);
    ctx.publish(Some(run), FrameKind::Started);
    // Plan D12: on a `worker` box the TUI only enqueues; the box's worker claims the run.
    if kit.tails == Tails::HandBack {
        return ctx.done(CommandOutcome::Started {
            run,
            rest: Rest {
                run: RunStatus::Queued,
                position: None,
                failure: None,
            },
        });
    }
    // M5 D84: read now, so a refused claim joins the queue with no await after the refusal.
    let queued_at = htui_core::store::WorkerStore::run(&kit.writer, run)
        .await
        .ok()
        .flatten()
        .map_or_else(|| ctx.shared.clock.now(), |row| row.queued_at);

    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    let ended = ctx.shared.ended.load(Ordering::SeqCst);
    let mut missed = false;
    match walked(&walk, engine.claim(run)).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        Some(Err(EngineError::Cancelled { .. })) => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Err(err @ EngineError::ClaimRefused { .. })) => {
            ctx.shared.queue(queued_at, run);
            // A task that ended during the claim may have found the queue still empty and
            // retried nothing; this task retries in its place.
            missed = ctx.shared.ended.load(Ordering::SeqCst) != ended;
            ctx.refuse(err.to_string());
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
    if missed {
        claim_queued(&ctx).await;
    }
}

/// Every verb on one run: preempt as the verb says, lock, dispatch (§8.6).
///
/// D212 (review H3): a verb that would move the run under a live chat ([`moves_the_run`]) is
/// refused first, before it preempts or waits, with [`chat_free`]'s sentence — published for the
/// item, because the task is tagged (D200).
async fn on_run<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    run: RunId,
    command: Command,
    preempt: Preempt,
    live: &LiveChats,
) {
    let _answered = on_run_unless_claimed(&ctx, run, command, preempt, live, false).await;
}

/// MOD-42 review L-4: whether `err` says the run is no longer queued because a claim won the race
/// with a cancel that read it `queued` — a live lease elsewhere on this box, or a run executing
/// (or parked) rather than queued or terminal.
fn lost_to_a_claim(err: &EngineError) -> bool {
    match err {
        EngineError::LeaseHeld { .. } => true,
        EngineError::RunStatus { status, .. } => {
            *status != RunStatus::Queued && !status.is_terminal()
        }
        _ => false,
    }
}

/// [`on_run`]'s body. With `claimed_back`, an error that says a claim won the race with the
/// command ([`lost_to_a_claim`]) is not answered but handed back, the run's lock released, so a
/// cancel can take D12 step 2's durable path; every other ending is answered here.
async fn on_run_unless_claimed<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: &TaskCtx<H, P>,
    run: RunId,
    command: Command,
    preempt: Preempt,
    live: &LiveChats,
    claimed_back: bool,
) -> Option<EngineError> {
    let early = ctx.tag(run).await;
    if moves_the_run(&command) && !live.is_empty() {
        let refusal = match ctx.host.run_steps(run).await {
            Ok(steps) => chat_free(&steps, live).err().map(|err| err.to_string()),
            Err(err) => Some(err.to_string()),
        };
        if let Some(refusal) = refusal {
            ctx.refuse(refusal);
            return None;
        }
    }
    if preempt == Preempt::IfLive && ctx.shared.walks.is_live(run) {
        ctx.shared
            .walks
            .preempt_gracefully(run, ctx.shared.cancel_grace)
            .await;
    }
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx
        .shared
        .lock_announcing(run, &walk, || ctx.publish(Some(run), FrameKind::Waiting))
        .await
    else {
        ctx.refuse(PREEMPTED.to_owned());
        return None;
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => {
            ctx.refuse(message);
            return None;
        }
    };
    if let Some(refusal) = kit.refusal(&command) {
        ctx.refuse(refusal);
        return None;
    }
    // `project_id` never changes, so the row read before the lock stands in for a failed one.
    let row = ctx.tag_run(&kit.writer, run).await.or(early);
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let promoting = matches!(command, Command::PromoteStep { .. });
    match walked(&walk, engine.dispatch(command)).await {
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        None | Some(Err(EngineError::Cancelled { .. })) => {
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
            ctx.answer(RunReply::Orch(OrchReply::Promoted {
                step,
                run,
                phase: opening.phase.clone(),
                agent: opening.agent_name.clone(),
                model: opening.model.clone(),
                via,
            }));
            ctx.publish(Some(run), FrameKind::Rested(rest));
            // D181: the chat binding is the loop's; the runtime's event channel reaches it. The
            // run's project comes from the row read before the dispatch, or read again now: a
            // promotion whose chat cannot be bound is answered, never left waiting.
            let project = match row {
                Some(row) => Ok(row.project_id),
                None => htui_core::store::WorkerStore::run(&kit.writer, run)
                    .await
                    .and_then(|row| {
                        row.map(|row| row.project_id)
                            .ok_or_else(|| StoreError::NotFound {
                                entity: "run",
                                id: run.to_string(),
                            })
                    }),
            };
            if let Some(addr) = ctx.addr.clone() {
                match project {
                    Ok(project) => {
                        let _ = ctx.shared.events.send(RunServed::Attach {
                            addr,
                            promoted: Box::new(Promoted {
                                run,
                                step,
                                project,
                                opening: *opening,
                            }),
                            ended: ChatEnd {
                                publisher: Arc::new(ctx.shared.publisher.clone()),
                                tag: Arc::clone(&ctx.tag),
                            },
                        });
                    }
                    Err(err) => ctx.answer(RunReply::Failed {
                        request: ctx.name,
                        message: format!(
                            "the step was promoted, but its run could not be read to bind its chat: {err}"
                        ),
                    }),
                }
            }
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        // MOD-42 OQ-3: on a `worker` box a live lease elsewhere is the box's worker walking the
        // run, and its live session cannot be handed to this process's chat.
        Some(Err(EngineError::LeaseHeld { run: held }))
            if promoting && kit.tails == Tails::HandBack =>
        {
            ctx.refuse(promote_needs_the_walker(held));
        }
        Some(Err(err)) if claimed_back && lost_to_a_claim(&err) => {
            drop(guard);
            return Some(err);
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
    None
}

/// MOD-42 plan D12: every cancel of a live run is a durable command its executor applies.
///
/// D212's chat check first, as [`on_run`]'s. A `queued` or terminal run (read before anything is
/// written) takes today's path and writes no row (B-20). Otherwise the `run_command` row is
/// written first (D12 step 2), then: this process walks the run → a graceful preempt (D11) and
/// `cancel_leased` (step 3); the run executes on another box → left to that box, decided before
/// `cancel_leased` (step 4); else `cancel_leased`, whose `LeaseHeld` leaves the row to the lease's
/// holder (step 5). A row left pending is answered with [`CANCEL_REQUESTED`] (B-12).
///
/// `existing` is a polled row (D13): no second row is written for it, and a refusal it meets —
/// a live chat's (B-10), a transient failure's, or a terminal status's once the row is resolved —
/// is logged at `debug` and never published.
async fn cancel_run<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    run: RunId,
    live: &LiveChats,
    existing: Option<RunCommandId>,
) {
    let row = ctx.tag(run).await;
    let command = Command::CancelRun { run };
    // B-10 (F-12): a polled row answers nobody, and its row stays for the next poll, so a refusal
    // published from here would repeat an error frame every `COMMAND_POLL` while it lasts, or
    // follow a cancel another task already applied (B-5's race): it is logged instead.
    let refuse = move |ctx: &TaskCtx<H, P>, message: String| {
        if existing.is_some() {
            tracing::debug!(%run, %message, "a polled cancel was refused");
        } else {
            ctx.refuse(message);
        }
    };
    if moves_the_run(&command) && !live.is_empty() {
        let refusal = match ctx.host.run_steps(run).await {
            Ok(steps) => chat_free(&steps, live).err().map(|err| err.to_string()),
            Err(err) => Some(err.to_string()),
        };
        if let Some(refusal) = refusal {
            return refuse(&ctx, refusal);
        }
    }
    let Some(row) = row else {
        return refuse(
            &ctx,
            StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            }
            .to_string(),
        );
    };
    // B-20: a queued or terminal run takes today's path and writes no row. Plan D11 and D12 step
    // 1 (not B-20's `Preempt::Never`): a queued run's own start or claim task in this process is
    // still stopped first, as MOD-41's `Preempt::Always` did, now gracefully; with `Never` the
    // cancel would wait behind that task's whole walk.
    //
    // Review L-4: a claim by another process that lands between this snapshot and the queued CAS
    // leaves a run that is no longer queued, so D12 step 2's "otherwise" holds: the run is read
    // again (its `executing_box_id` decides step 4) and the cancel goes durable. A run that reads
    // queued or terminal again keeps today's rowless refusal.
    let row = if existing.is_none() && (row.status == RunStatus::Queued || row.status.is_terminal())
    {
        let queued = Box::pin(on_run_unless_claimed(
            &ctx,
            run,
            Command::CancelRun { run },
            Preempt::IfLive,
            live,
            true,
        ));
        let Some(lost) = queued.await else {
            return;
        };
        match ctx.tag(run).await {
            Some(row) if row.status != RunStatus::Queued && !row.status.is_terminal() => row,
            _ => return refuse(&ctx, lost.to_string()),
        }
    } else {
        row
    };
    let Some(writer) = ctx.host.writer() else {
        return refuse(&ctx, DATABASE_UNREACHABLE.to_owned());
    };
    let (box_id, user) = match (registered_box(&ctx.host).await, ctx.host.this_user().await) {
        (Ok(box_id), Ok(user)) => (box_id, user),
        (Err(err), _) | (_, Err(err)) => return refuse(&ctx, err.to_string()),
    };
    // D12 step 2: the row first.
    let (id, already) = match existing {
        Some(id) => (id, false),
        None => {
            match htui_core::store::WorkerStore::request_cancel(&writer, run, user, box_id).await {
                Ok(CancelRequest::Inserted(id)) => (id, false),
                Ok(CancelRequest::AlreadyPending(id)) => (id, true),
                Err(err) => return refuse(&ctx, err.to_string()),
            }
        }
    };
    // B-5: one task per row at a time.
    let Some(_applying) = ctx.shared.applying(id) else {
        return ctx.requested(true);
    };
    if ctx.shared.walks.is_live(run) {
        // D12 step 3: this process walks it.
        ctx.shared
            .walks
            .preempt_gracefully(run, ctx.shared.cancel_grace)
            .await;
    } else if row.executing_box_id != Some(box_id) {
        // D12 step 4, decided before `cancel_leased` (whose refusal there is `RunStatus`).
        return ctx.requested(already);
    }
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx
        .shared
        .lock_announcing(run, &walk, || ctx.publish(Some(run), FrameKind::Waiting))
        .await
    else {
        return refuse(&ctx, PREEMPTED.to_owned());
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => return refuse(&ctx, message),
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let resolve = async |to: RunCommandStatus, why: Option<String>| {
        match htui_core::store::WorkerStore::resolve_command(&kit.writer, id, to, why).await {
            Ok(true) => {}
            Ok(false) => tracing::debug!(%run, %id, "the cancel's row was resolved elsewhere"),
            Err(err) => tracing::warn!(%run, %id, %err, "the cancel's row was not resolved"),
        }
    };
    match walked(&walk, engine.dispatch(command)).await {
        None => {
            engine.abandoned(run).await;
            refuse(&ctx, PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => {
            resolve(RunCommandStatus::Applied, None).await;
            ctx.done(outcome);
        }
        // D12 step 5: a live lease elsewhere on this box (the worker): its poll applies it.
        Some(Err(EngineError::LeaseHeld { .. })) => ctx.requested(already),
        // D12 steps 3 and 5: already `cancelled` means the cancel's goal holds — most often
        // another process on this box applied this very row and is still in `cancel_leased`'s
        // cleanup, where B-4's terminal clause hands the pending row to this poll too. The row is
        // `applied` whichever process resolves it first; a TUI's own `c` still hears the status.
        Some(Err(
            err @ EngineError::RunStatus {
                status: RunStatus::Cancelled,
                ..
            },
        )) => {
            resolve(RunCommandStatus::Applied, None).await;
            refuse(&ctx, err.to_string());
        }
        // D13, B-4: otherwise already terminal → refused with the actual status.
        Some(Err(err @ EngineError::RunStatus { status, .. })) if status.is_terminal() => {
            let sentence = err.to_string();
            resolve(RunCommandStatus::Refused, Some(sentence.clone())).await;
            refuse(&ctx, sentence);
        }
        // Anything else is transient: the row stays pending and the next poll retries.
        Some(Err(err)) => refuse(&ctx, err.to_string()),
    }
    drop(guard);
}

/// MOD-42 plan D13: one command poll. Every pending row this process may apply
/// (`pending_commands(owner, box)`, B-4), unless a task of this process is applying it already
/// (B-5), runs as an internal cancel on a supervised task of its own.
async fn poll_once<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    live: LiveChats,
) {
    let Some(writer) = ctx.host.writer() else {
        return;
    };
    let box_id = match registered_box(&ctx.host).await {
        Ok(box_id) => box_id,
        Err(err) => {
            tracing::debug!(%err, "the command poll could not read this box");
            return;
        }
    };
    let rows =
        match htui_core::store::WorkerStore::pending_commands(&writer, ctx.shared.owner, box_id)
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "reading the pending run commands failed");
                return;
            }
        };
    for row in rows {
        if ctx.shared.is_applying(row.id) {
            continue;
        }
        // MOD-24 D1 (K5): this process will apply the row; nothing has applied it yet.
        htui_orch::kill_point::reached(KillPoint::CommandPicked, Site::NONE);
        let task = ctx.unaddressed("cancel_run");
        let live = live.clone();
        spawn_supervised(task.clone(), async move {
            cancel_run(task, row.run_id, &live, Some(row.id)).await;
        });
    }
}

/// `Unblock` (D161): a reopen needs no run; the other two cases lock the run they name and check
/// the case again under the lock (§8.6).
async fn unblock<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, item: ItemId) {
    let _ = ctx.tag.item.set(item);
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let case = match engine.unblock_case(item).await {
        Ok(case) => case,
        Err(err) => return ctx.refuse(err.to_string()),
    };
    // Plan D9: the resume case walks; the engine's `Tails` hands it back on a `worker` box.
    if matches!(case, UnblockCase::Resume(_))
        && let Some(refusal) = kit.walking_refusal()
    {
        return ctx.refuse(refusal);
    }
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
    let Some(guard) = ctx
        .shared
        .lock_announcing(run, &walk, || ctx.publish(Some(run), FrameKind::Waiting))
        .await
    else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    if engine.unblock_case(item).await.ok() != Some(case) {
        return ctx.refuse(UNBLOCK_MOVED.to_owned());
    }
    match walked(&walk, engine.dispatch(Command::Unblock { item })).await {
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        None | Some(Err(EngineError::Cancelled { .. })) => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => ctx.done(outcome),
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// D177 (R-25): a terminal run's cleanup, again, under the run's lock.
async fn cleanup<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId) {
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx
        .shared
        .lock_announcing(run, &walk, || ctx.publish(Some(run), FrameKind::Waiting))
        .await
    else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
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
        Ok(()) => {
            ctx.publish(Some(run), FrameKind::Changed);
            ctx.answer(RunReply::Orch(OrchReply::CleanedUp { run }));
        }
        Err(err) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}

/// MOD-41 plan D8: the run runtime's white-box surface, for the TUI's `run_worker` tests. Its cases
/// drive the runtime through the TUI's store loop, so they stay in `htui`, and reach what this
/// crate keeps private only through here: a [`TaskCtx`] constructor, the task functions they call
/// directly, and read accessors on the runtime's shared state.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod testing {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, PoisonError};

    use chrono::{DateTime, Utc};
    use htui_core::model::{BoxId, ItemId, RunId};
    use htui_core::store::Result as StoreResult;
    use tokio::sync::OwnedMutexGuard;

    use super::{ReplySink, RunRuntime, Shared};
    pub use super::{TaskCtx, WalkToken};
    use crate::views::ItemActions;

    /// A handle on a runtime's shared state (MOD-41 plan D8).
    pub struct Probe<P: ReplySink>(Arc<Shared<P>>);

    /// B-5's guard held by a case ([`Probe::hold_applying`]); dropping it frees the row and wakes
    /// whoever awaits it.
    #[must_use = "the guard frees the row when dropped"]
    pub struct Held<'a> {
        _guard: super::Applying<'a>,
    }

    impl core::fmt::Debug for Held<'_> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Held").finish_non_exhaustive()
        }
    }

    /// MOD-41 blueprint F-12: hand-written, so no `P: Debug` is asked of a sink.
    impl<P: ReplySink> core::fmt::Debug for Probe<P> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Probe")
                .field("owner", &self.0.owner)
                .finish_non_exhaustive()
        }
    }

    /// A probe on `runtime`'s shared state.
    #[must_use]
    pub fn probe<H: htui_core::store::WorkerHost, P: ReplySink>(
        runtime: &RunRuntime<H, P>,
    ) -> Probe<P> {
        Probe(Arc::clone(&runtime.shared))
    }

    impl<P: ReplySink> Probe<P> {
        /// This process's lease owner (MOD-42: a case hands it a run's lease).
        #[must_use]
        pub fn owner(&self) -> uuid::Uuid {
            self.0.owner
        }

        /// Whether a task of this process is applying the run command `id` (MOD-42 B-5).
        #[must_use]
        pub fn is_applying(&self, id: htui_core::model::RunCommandId) -> bool {
            self.0
                .applying
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&id)
        }

        /// B-5's guard on the run command `id`, as a task applying it holds it (MOD-24 H-16);
        /// `None` while a task of this process applies it already.
        #[must_use]
        pub fn hold_applying(&self, id: htui_core::model::RunCommandId) -> Option<Held<'_>> {
            self.0.applying(id).map(|guard| Held { _guard: guard })
        }

        /// `run`'s lock when nobody holds it.
        #[must_use]
        pub fn try_lock(&self, run: RunId) -> Option<OwnedMutexGuard<()>> {
            self.0.locks.try_lock(run)
        }

        /// Whether the run-lock map holds an entry for `run` (D214's pruning).
        #[must_use]
        pub fn has_lock_entry(&self, run: RunId) -> bool {
            self.0
                .locks
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(&run)
        }

        /// Whether `run` has a parent token (D187, D214).
        #[must_use]
        pub fn has_parent(&self, run: RunId) -> bool {
            self.0.walks.lock().contains_key(&run)
        }

        /// A task's child token on `run`: the run is live while it is held.
        #[must_use]
        pub fn walk_child(&self, run: RunId) -> WalkToken {
            self.0.walks.child(run)
        }

        /// How many tasks hold a child token on `run`'s parent; 0 with no parent.
        #[must_use]
        pub fn live_walks(&self, run: RunId) -> usize {
            self.0
                .walks
                .lock()
                .get(&run)
                .map_or(0, |parent| parent.live.load(Ordering::SeqCst))
        }

        /// Whether `run` is one of this process's dead walks (D158).
        #[must_use]
        pub fn is_dead_walk(&self, run: RunId) -> bool {
            self.0.dead_walks.contains(run)
        }

        /// `run` joins the claim queue at `queued_at` (M5 D84).
        pub fn queue(&self, queued_at: DateTime<Utc>, run: RunId) {
            self.0.queue(queued_at, run);
        }

        /// The claim queue, in `queued_at` order (M5 D84).
        #[must_use]
        pub fn queued(&self) -> Vec<(DateTime<Utc>, RunId)> {
            self.0
                .queued
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .copied()
                .collect()
        }

        /// Whether every tracked task has finished.
        #[must_use]
        pub fn all_tasks_finished(&self) -> bool {
            self.0
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .all(|task| task.handle.is_finished())
        }

        /// How many tracked tasks have not finished yet.
        #[must_use]
        pub fn unfinished_tasks(&self) -> usize {
            self.0
                .tasks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|task| !task.handle.is_finished())
                .count()
        }

        /// The sink frames go out on.
        pub fn wire(&self, sink: &P) {
            self.0.publisher.wire(sink);
        }

        /// `addr`'s subscriber now follows `item`, at `addr`.
        pub fn subscribe(&self, addr: P::Addr, item: ItemId) {
            self.0.publisher.subscribe(addr, item);
        }

        /// A context for a task of this runtime that answers nobody, over `host` and `sink`.
        #[must_use]
        pub fn task_ctx<H: htui_core::store::WorkerHost>(
            &self,
            host: H,
            sink: P,
            name: &'static str,
        ) -> TaskCtx<H, P> {
            TaskCtx {
                shared: Arc::clone(&self.0),
                host,
                sink,
                addr: None,
                name,
                tag: Arc::default(),
            }
        }
    }

    /// Tags `ctx` with the run it works on.
    pub fn set_task_run<H: htui_core::store::WorkerHost, P: ReplySink>(
        ctx: &TaskCtx<H, P>,
        run: RunId,
    ) {
        let _ = ctx.tag.run.set(run);
    }

    /// `TaskCtx::unaddressed`: a context for a task of the runtime's own.
    #[must_use]
    pub fn unaddressed<H: htui_core::store::WorkerHost, P: ReplySink>(
        ctx: &TaskCtx<H, P>,
        name: &'static str,
    ) -> TaskCtx<H, P> {
        ctx.unaddressed(name)
    }

    /// `spawn_supervised`: `work` on a supervised, tracked task.
    pub fn spawn_supervised<H: htui_core::store::WorkerHost, P: ReplySink>(
        ctx: TaskCtx<H, P>,
        work: impl Future<Output = ()> + Send + 'static,
    ) {
        super::spawn_supervised(ctx, work);
    }

    /// Whether a graceful preempt (MOD-42 plan D11) reached the task holding `walk`.
    #[must_use]
    pub fn signalled(walk: &WalkToken) -> bool {
        walk.signal.borrow().is_cancel()
    }

    /// `resumed`: an adopted run's walk, resumed under its lock.
    pub async fn resumed<H: htui_core::store::WorkerHost, P: ReplySink>(
        ctx: TaskCtx<H, P>,
        run: RunId,
    ) {
        super::resumed(ctx, run).await;
    }

    /// `retry_claims`: the claim retry once `ctx`'s run rests.
    pub async fn retry_claims<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) {
        super::retry_claims(ctx).await;
    }

    /// `command_limits`: the box row's `settings.command_limits` over the app setting (MOD-11
    /// D15).
    ///
    /// # Errors
    /// The store's own read failure.
    pub async fn command_limits<H: htui_core::store::WorkerHost>(
        host: &H,
        box_id: BoxId,
    ) -> StoreResult<BTreeMap<String, u32>> {
        Ok(super::parse_limits(
            box_id,
            &super::stored_limits(host, box_id).await?,
        ))
    }

    /// The verdicts of an item the mirror does not hold, off the server: nothing is enabled.
    #[must_use]
    pub fn unreachable_actions(item: ItemId, key: String) -> ItemActions {
        crate::views::unreachable_actions(item, key)
    }
}

/// MOD-42 plan D9: [`Kit`]'s policy lookup over `agents`, each agent's
/// `agent.settings.permission` parsed once per task as chat parses it
/// (`agent_worker.rs:968-969`): a row that does not parse, and an agent the task did not read,
/// ask.
fn policy_lookup(
    agents: &HashMap<AgentId, AgentSummary>,
) -> Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync> {
    let policies: HashMap<AgentId, PermissionPolicy> = agents
        .values()
        .map(|summary| {
            let settings: AgentSettings =
                serde_json::from_value(summary.agent.settings.clone()).unwrap_or_default();
            (summary.agent.id, settings.permission)
        })
        .collect();
    Box::new(move |agent| policies.get(&agent).cloned().unwrap_or_default())
}

/// MOD-41 plan D8: the parts only the library owns, over `Backend::memory(MemStore::demo())`. The
/// runtime is reached through its generic API alone, so no case imports `WorkerHost` beside the
/// `ReadStore` that `Backend` also implements (blueprint F-33).
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex, PoisonError};

    use htui_agent::registry::DriverFactory;
    use htui_core::fixtures::ids;
    use htui_core::model::ItemId;
    use htui_core::store::MemStore;
    use htui_store::Backend;

    use super::RunRuntime;
    use crate::address::Publish as _;
    use crate::{FrameKind, LiveChats, ReplySink, RunFrame, RunReply, RunRequest, RunServed};

    /// A sink that records every reply at its address. An address's tens digit is its subscriber,
    /// so `10` and `11` are one subscriber at two addresses.
    #[derive(Debug, Clone, Default)]
    struct Recording(Arc<StdMutex<Vec<(u64, RunReply)>>>);

    impl ReplySink for Recording {
        type Addr = u64;
        type Subscriber = u64;

        fn subscriber(addr: &u64) -> u64 {
            addr / 10
        }

        fn send(&self, to: &u64, reply: RunReply) {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((*to, reply));
        }
    }

    impl Recording {
        /// The addresses a frame of `item` reached since the last call, in order; any other reply
        /// fails the test.
        fn frames_of(&self, item: ItemId) -> Vec<u64> {
            let replies =
                std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
            replies
                .into_iter()
                .map(|(addr, reply)| match reply {
                    RunReply::Frame(frame) if frame.item == item => addr,
                    other => {
                        panic!("only frames of {item} were published, got {other:?} at {addr}")
                    }
                })
                .collect()
        }
    }

    fn runtime() -> RunRuntime<Backend, Recording> {
        RunRuntime::new(DriverFactory::new())
    }

    /// Subscribes `addr` to `item` through `serve_request`, which acknowledges it at once.
    async fn subscribe(
        runtime: &mut RunRuntime<Backend, Recording>,
        backend: &Backend,
        sink: &Recording,
        addr: u64,
        item: ItemId,
    ) {
        let served = runtime
            .serve_request(
                backend,
                sink,
                addr,
                RunRequest::Stream { item },
                &LiveChats::default(),
            )
            .await;
        assert!(
            matches!(&served, RunServed::Reply(RunReply::Frame(RunFrame { item: of, run: None, kind: FrameKind::Subscribed })) if *of == item),
            "{served:?}"
        );
    }

    fn changed(item: ItemId) -> RunFrame {
        RunFrame {
            item,
            run: None,
            kind: FrameKind::Changed,
        }
    }

    /// Plan D172, MOD-41 plan D7: a frame reaches every subscriber of its item, each at its own
    /// subscription's address, and no subscriber of another item.
    #[tokio::test]
    async fn publisher_sends_each_frame_to_its_items_subscribers_only() {
        let backend = Backend::memory(MemStore::demo());
        let sink = Recording::default();
        let mut runtime = runtime();
        subscribe(&mut runtime, &backend, &sink, 10, ids::HTUI_ANA_2).await;
        subscribe(&mut runtime, &backend, &sink, 20, ids::HTUI_ANA_2).await;
        subscribe(&mut runtime, &backend, &sink, 30, ids::HTUI_FEAT_1).await;
        assert!(
            sink.frames_of(ids::HTUI_ANA_2).is_empty(),
            "a subscription is answered, not sent"
        );

        runtime.shared.publisher.publish(&changed(ids::HTUI_ANA_2));
        let mut reached = sink.frames_of(ids::HTUI_ANA_2);
        reached.sort_unstable();
        assert_eq!(
            reached,
            [10, 20],
            "both subscribers of the item, and only they"
        );

        runtime.shared.publisher.publish(&changed(ids::HTUI_FEAT_1));
        assert_eq!(sink.frames_of(ids::HTUI_FEAT_1), [30]);
    }

    /// MOD-11 D15: with no box value the limits are `app_setting.command_limits`; a box's own
    /// `settings.command_limits` overlays them key by key; a box value that does not parse
    /// leaves the app's; with neither, nothing (every class then reads 1).
    #[tokio::test]
    async fn command_limits_falls_back_to_the_app_setting() {
        use std::collections::BTreeMap;

        use htui_core::model::BoxId;
        use serde_json::json;

        let with_box = |stored: Option<serde_json::Value>| {
            let mut data = htui_core::fixtures::demo_data();
            let row = data
                .boxes
                .iter_mut()
                .find(|row| row.id == ids::BOX)
                .expect("the demo box");
            match stored {
                Some(limits) => row.settings["command_limits"] = limits,
                None => row.settings = json!({}),
            }
            MemStore::from_demo(data)
        };
        let seeded = json!({"build": 1, "test": 4, "verify": 1});
        let app = BTreeMap::from([
            ("build".to_owned(), 1),
            ("test".to_owned(), 4),
            ("verify".to_owned(), 1),
        ]);

        let store = with_box(None);
        store.set_app_setting("command_limits", seeded.clone());
        let backend = Backend::memory(store);
        assert_eq!(
            super::testing::command_limits(&backend, BoxId::new())
                .await
                .expect("the read answers"),
            app,
            "no box row: the app setting"
        );
        assert_eq!(
            super::testing::command_limits(&backend, ids::BOX)
                .await
                .expect("the read answers"),
            app,
            "no box value: the app setting"
        );

        let store = with_box(Some(json!({"test": 2, "run": 3})));
        store.set_app_setting("command_limits", seeded.clone());
        assert_eq!(
            super::testing::command_limits(&Backend::memory(store), ids::BOX)
                .await
                .expect("the read answers"),
            BTreeMap::from([
                ("build".to_owned(), 1),
                ("run".to_owned(), 3),
                ("test".to_owned(), 2),
                ("verify".to_owned(), 1),
            ]),
            "the box overlays the app key by key"
        );

        let store = with_box(Some(json!("many")));
        store.set_app_setting("command_limits", seeded);
        assert_eq!(
            super::testing::command_limits(&Backend::memory(store), ids::BOX)
                .await
                .expect("the read answers"),
            app,
            "a box value that does not parse leaves the app's"
        );

        assert_eq!(
            super::testing::command_limits(&Backend::memory(with_box(None)), ids::BOX)
                .await
                .expect("the read answers"),
            BTreeMap::new(),
            "neither: nothing, and every class reads 1"
        );
    }

    /// Blueprint §0a point 3: a later subscription of the same subscriber replaces the earlier
    /// one, item and address both.
    #[tokio::test]
    async fn a_later_subscription_replaces_the_earlier_one() {
        let backend = Backend::memory(MemStore::demo());
        let sink = Recording::default();
        let mut runtime = runtime();
        subscribe(&mut runtime, &backend, &sink, 10, ids::HTUI_ANA_2).await;
        subscribe(&mut runtime, &backend, &sink, 20, ids::HTUI_ANA_2).await;
        subscribe(&mut runtime, &backend, &sink, 11, ids::HTUI_FEAT_1).await;

        runtime.shared.publisher.publish(&changed(ids::HTUI_ANA_2));
        assert_eq!(
            sink.frames_of(ids::HTUI_ANA_2),
            [20],
            "subscriber 1 left the item when it followed another"
        );
        runtime.shared.publisher.publish(&changed(ids::HTUI_FEAT_1));
        assert_eq!(
            sink.frames_of(ids::HTUI_FEAT_1),
            [11],
            "at its later subscription's address"
        );
    }
}

/// MOD-41 I-1, OQ-2, OQ-5, OQ-6: the role gate over `Backend::memory`, role [`Role::Worker`]. The
/// runtime is reached through its generic API alone (blueprint F-33).
#[cfg(test)]
mod role_gate {
    use std::sync::{Arc, Mutex as StdMutex, PoisonError};
    use std::time::Duration;

    use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::driver::{AgentDriver, DriverCaps};
    use htui_agent::error::DriverError;
    use htui_agent::event::{DoneEvent, DriverEvent, StopReason};
    use htui_agent::fake::FakeDriver;
    use htui_agent::registry::{DriverFactory, TransportBuilder};
    use htui_core::fixtures::{demo_at, demo_data, edit_agent, ids};
    use htui_core::model::{
        Agent, AgentBox, AgentId, Billing, BoxEdit, DocumentId, Executor, ItemId, NewDocument,
        NewRepo, NewRun, RepoBoxPath, RepoId, RunId, RunMode, RunStatus, RunStep, SnapshotPhase,
        TIMESTAMPTZ_DIGITS, Transport,
    };
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_orch::fake::{FakeIsolator, FakeVerifier};
    use htui_orch::{Clock, Isolator};
    use htui_store::Backend;
    use serde_json::json;
    use uuid::Uuid;

    use super::{BACKOFF_FIRST, BACKOFF_MAX, Role, RunRuntime};
    use crate::graphs::HostGraphs;
    use crate::{FrameKind, LiveChats, ReplySink, RunFrame, RunReply, RunRequest, StepAuthor};

    /// How long a case waits for the runtime to do what it asserts.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A sink that records every frame with the tokio instant it arrived at.
    #[derive(Debug, Clone)]
    struct Timed {
        start: tokio::time::Instant,
        frames: Arc<StdMutex<Vec<(Duration, RunFrame)>>>,
    }

    impl Timed {
        fn new() -> Self {
            Self {
                start: tokio::time::Instant::now(),
                frames: Arc::default(),
            }
        }

        /// When each `Error` frame of `item` arrived, from the sink's start.
        fn errors_of(&self, item: ItemId) -> Vec<Duration> {
            self.frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|(_, frame)| {
                    frame.item == item && matches!(frame.kind, FrameKind::Error(_))
                })
                .map(|(at, _)| *at)
                .collect()
        }
    }

    impl ReplySink for Timed {
        type Addr = u64;
        type Subscriber = u64;

        fn subscriber(addr: &u64) -> u64 {
            *addr
        }

        fn send(&self, _to: &u64, reply: RunReply) {
            if let RunReply::Frame(frame) = reply {
                self.frames
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((self.start.elapsed(), frame));
            }
        }
    }

    /// `base + (tokio now - start)`: a paused case moves the rows' instants and the store's lease
    /// comparisons with its own sleeps.
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
            let elapsed = TimeDelta::from_std(self.start.elapsed()).expect("a case is short");
            (self.base + elapsed).trunc_subsecs(TIMESTAMPTZ_DIGITS)
        }
    }

    /// One turn, then `done`, for every session of the scripted row.
    #[derive(Debug)]
    struct OneTurn;

    impl TransportBuilder for OneTurn {
        fn build(
            &self,
            agent: &Agent,
            _on_box: Option<&AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            let done = Script::one_turn(vec![ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            }))]);
            Ok(Box::new(FakeDriver::new(agent.name.clone(), caps, done)))
        }
    }

    /// One document of the phase's `output_kind` per step, so every step settles `done`.
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

    /// The demo with every fixture agent disabled, one scripted `acp` agent ready on the box, a
    /// primary repo every default scope resolves to, and the seeded queued `RUN_2` cancelled.
    async fn seeded(store: MemStore) -> MemStore {
        seeded_with_primary(store, "htui").await
    }

    /// [`seeded`], its primary repo named `primary` (MOD-10: a repo slug reaches the trim record).
    async fn seeded_with_primary(store: MemStore, primary: &str) -> MemStore {
        for summary in store.agents().await.expect("the fixture's agents") {
            let mut row = summary.agent;
            row.enabled = false;
            edit_agent(&store, &row).await.expect("the row is disabled");
        }
        let agent = AgentId::new();
        store
            .upsert_agent(
                &Agent {
                    id: agent,
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
                },
                None,
            )
            .await
            .expect("the scripted row lands");
        store
            .upsert_agent_box(&AgentBox {
                agent_id: agent,
                box_id: ids::BOX,
                enabled: true,
                version: Some("0.0.0-fake".to_owned()),
                path: None,
                probed_at: Some(demo_at(0, 0)),
                quota: None,
                quota_at: None,
                updated_at: demo_at(0, 0),
                probe: Some(json!({ "status": "ready", "source": "probe" })),
            })
            .await
            .expect("the agent_box row lands");
        store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: primary.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
        store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the seeded run is queued and cancellable");
        store
    }

    /// The box's executor set through its editor (MOD-41 plan D10).
    async fn set_executor(store: &MemStore, executor: Executor) {
        let row = store
            .box_row(ids::BOX)
            .await
            .expect("the read answers")
            .expect("the demo box");
        store
            .edit_box(
                ids::BOX,
                row.edit_version,
                BoxEdit {
                    executor: Some(executor),
                    ..BoxEdit::default()
                },
            )
            .await
            .expect("the edit answers");
    }

    /// A worker-role runtime over the fakes, the scripted row and the output author.
    fn worker_runtime() -> RunRuntime<Backend, Timed> {
        let mut factory = DriverFactory::new();
        factory.register("acp", Box::new(OneTurn));
        RunRuntime::with_parts(
            Arc::new(FakeIsolator::new()) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            factory,
        )
        .with_author(Arc::new(OutputAuthor))
        .with_role(Role::Worker)
    }

    /// A `queued` run of `item`, created by another process at `queued_at`.
    async fn queued(store: &MemStore, item: ItemId, queued_at: DateTime<Utc>) -> RunId {
        queued_over(store, item, queued_at, None).await
    }

    /// [`queued`] over `scope`, or the default scope when `None`.
    async fn queued_over(
        store: &MemStore,
        item: ItemId,
        queued_at: DateTime<Utc>,
        scope: Option<&[RepoId]>,
    ) -> RunId {
        let row = store
            .item(item)
            .await
            .expect("the read answers")
            .expect("the item");
        let app = store.app_settings().await.expect("the settings");
        let resolved = htui_orch::resolve(
            store,
            &HostGraphs(Backend::memory(store.clone())),
            &row,
            RunMode::Manual,
            &app,
            scope,
            ids::BOX,
        )
        .await
        .expect("the item resolves");
        let run = RunId::new();
        store
            .create_run(NewRun {
                id: run,
                project_id: row.project_id,
                item_id: row.id,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: resolved.snapshot,
                repo_scope: resolved.repo_scope,
                queued_at,
            })
            .await
            .expect("the run lands");
        run
    }

    /// A run of `item` another process claimed an hour ago under a lease that lapsed at once.
    async fn stranded(store: &MemStore, item: ItemId) -> RunId {
        let past = Utc::now() - TimeDelta::hours(1);
        let run = queued(store, item, past).await;
        let claim = store
            .claim_run(run, ids::BOX, Uuid::now_v7(), past, TimeDelta::zero())
            .await
            .expect("the claim answers");
        assert!(claim.is_admitted(), "{claim}");
        run
    }

    async fn status_of(store: &MemStore, run: RunId) -> RunStatus {
        store
            .run(run)
            .await
            .expect("the read answers")
            .expect("the run")
            .status
    }

    /// Polls until `run` reaches `status`.
    async fn rests_at(store: &MemStore, run: RunId, status: RunStatus) {
        tokio::time::timeout(PATIENCE, async {
            while status_of(store, run).await != status {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {run} did not reach `{status}` within {PATIENCE:?}"));
    }

    /// The idle assertions: the queued run is not claimed and the stranded one not adopted.
    async fn idles(store: MemStore) {
        let store = seeded(store).await;
        let waiting = queued(&store, ids::HTUI_CLEAN_1, Utc::now()).await;
        let lapsed = stranded(&store, ids::HTUI_ANA_2).await;
        let before = store.run(lapsed).await.expect("the read").expect("the run");
        let mut runtime = worker_runtime();
        let backend = Backend::memory(store.clone());

        runtime.sweep_with(&backend, &Timed::new());
        assert!(runtime.settle(PATIENCE).await.is_empty());

        let row = store
            .run(waiting)
            .await
            .expect("the read")
            .expect("the run");
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Queued, None),
            "I-1: not this role's box, so no claim"
        );
        let row = store.run(lapsed).await.expect("the read").expect("the run");
        assert_eq!(
            (row.status, row.lease_expires_at),
            (RunStatus::Running, before.lease_expires_at),
            "I-1: and no adoption"
        );
        assert!(
            store.run_steps(lapsed).await.expect("the read").is_empty(),
            "nothing walked"
        );
    }

    /// OQ-2: `htui worker` on a box whose executor is the default `tui` idles: it claims no
    /// queued row and adopts no lapsed lease.
    #[tokio::test]
    async fn a_worker_role_idles_on_a_tui_box() {
        idles(MemStore::demo()).await;
    }

    /// Plan D9: an executor this build does not know fails closed; the worker idles there too.
    #[tokio::test]
    async fn an_unknown_executor_idles_the_worker() {
        let mut data = demo_data();
        for row in &mut data.boxes {
            row.settings = json!({ "executor": "container", "max_concurrent_items": 2 });
        }
        idles(MemStore::from_demo(data)).await;
    }

    /// Plan D14, OQ-5: on a `worker` box the worker's sweep claims the box's queued rows in
    /// `(queued_at, id)` order, whoever queued them. Two runs scope the primary repo, so the
    /// earlier is claimed and walks to its gate, the later is refused on the overlap and waits,
    /// and it is claimed once the earlier one is gone.
    #[tokio::test]
    async fn a_worker_role_claims_queued_rows_in_order() {
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let now = Utc::now();
        // Created later, queued earlier: the order is `queued_at`'s, not the insert's.
        let later = queued(&store, ids::HTUI_CLEAN_1, now - TimeDelta::minutes(1)).await;
        let earlier = queued(&store, ids::HTUI_ANA_2, now - TimeDelta::minutes(2)).await;
        let mut runtime = worker_runtime();
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();

        runtime.sweep_with(&backend, &sink);
        rests_at(&store, earlier, RunStatus::AwaitingApproval).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(
            status_of(&store, later).await,
            RunStatus::Queued,
            "the parked earlier run still holds the overlapping scope"
        );

        store
            .finish_run(earlier, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the parked run is cancellable");
        runtime.sweep_with(&backend, &sink);
        rests_at(&store, later, RunStatus::AwaitingApproval).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
    }

    /// MOD-11 D11: a runtime built `with_tool_host` hands the host to every engine it builds:
    /// the claimed run's first session opened a lease scoped to its step, under the runtime's
    /// own fence, and gave it back when the session ended.
    #[tokio::test]
    async fn with_tool_host_reaches_the_engine() {
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let run = queued(&store, ids::HTUI_ANA_2, Utc::now()).await;
        let host = Arc::new(htui_orch::fake::FakeToolHost::default());
        let mut runtime = worker_runtime()
            .with_tool_host(Arc::clone(&host) as Arc<dyn htui_orch::tools::ToolHost>);
        let backend = Backend::memory(store.clone());

        runtime.sweep_with(&backend, &Timed::new());
        rests_at(&store, run, RunStatus::AwaitingApproval).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());

        let steps = store.run_steps(run).await.expect("the read");
        let opened = host.opened();
        assert!(!opened.is_empty(), "the walk's session opened a lease");
        assert_eq!(opened[0].run_id, run);
        assert_eq!(opened[0].step_id, steps[0].id);
        assert!(
            matches!(opened[0].fence, htui_core::store::StepFence::Lease(_)),
            "{:?}",
            opened[0].fence
        );
        assert_eq!(host.live(), 0, "the lease ended with the session");
    }

    /// MOD-11 B-19: `shutdown` closes the tool host once the tasks have ended.
    #[tokio::test]
    async fn shutdown_closes_the_tool_host() {
        let host = Arc::new(htui_orch::fake::FakeToolHost::default());
        let mut runtime = worker_runtime()
            .with_tool_host(Arc::clone(&host) as Arc<dyn htui_orch::tools::ToolHost>);
        assert!(!host.closed());
        runtime.shutdown(Duration::from_millis(10)).await;
        assert!(host.closed(), "the listener is the runtime's to close");
    }

    /// OQ-6, blueprint B-10: a stranded run whose live graph no longer resolves (`NoGraph`) fails
    /// its resume at every adoption. Polled every second for a minute, the worker resumes it at
    /// 0, 5, 15 and 35 s — the delay doubling from 5 s — not at every poll.
    #[tokio::test(start_paused = true)]
    async fn a_run_whose_resume_keeps_failing_backs_off() {
        let clock = Arc::new(TokioClock::new());
        let mut data = demo_data();
        data.graphs.retain(|graph| graph.id != ids::GRAPH_HTUI_FEAT);
        let store = MemStore::from_demo(data).with_clock(Arc::clone(&clock) as Arc<dyn Clock>);
        set_executor(&store, Executor::Worker).await;
        // `RUN_2` carries its snapshot from before the graph went: claimed an hour ago under a
        // lease that lapsed at once.
        let past = clock.now() - TimeDelta::hours(1);
        let claim = store
            .claim_run(
                ids::RUN_2,
                ids::BOX,
                Uuid::now_v7(),
                past,
                TimeDelta::zero(),
            )
            .await
            .expect("the claim answers");
        assert!(claim.is_admitted(), "{claim}");
        let mut runtime = RunRuntime::with_parts(
            Arc::new(FakeIsolator::new()) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            DriverFactory::new(),
        )
        .with_clock(clock)
        .with_role(Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        runtime
            .serve_request(
                &backend,
                &sink,
                1,
                RunRequest::Stream {
                    item: ids::HTUI_FEAT_3,
                },
                &LiveChats::default(),
            )
            .await;

        for _ in 0..60 {
            runtime.sweep_with(&backend, &sink);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let at: Vec<u64> = sink
            .errors_of(ids::HTUI_FEAT_3)
            .iter()
            .map(Duration::as_secs)
            .collect();
        assert_eq!(at, [0, 5, 15, 35], "4 resumes, not 60");
    }

    /// A transport whose every session panics as it is built: the walk task dies (D158).
    #[derive(Debug)]
    struct Panics;

    impl TransportBuilder for Panics {
        fn build(
            &self,
            _agent: &Agent,
            _on_box: Option<&AgentBox>,
            _caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            panic!("the scripted transport panics as its session is built");
        }
    }

    /// MOD-41 review R-2 (OQ-6): a resume whose walk panics steps its run's backoff like a resume
    /// that fails. The supervisor marks the run dead, so the next sweep gives its lease back and
    /// adopts it again; the resume that adoption spawns waits [`BACKOFF_FIRST`], not one sweep.
    #[tokio::test(start_paused = true)]
    async fn a_run_whose_resume_panics_backs_off() {
        let clock = Arc::new(TokioClock::new());
        let store = seeded(MemStore::demo().with_clock(Arc::clone(&clock) as Arc<dyn Clock>)).await;
        set_executor(&store, Executor::Worker).await;
        let run = stranded(&store, ids::HTUI_ANA_2).await;
        let mut factory = DriverFactory::new();
        factory.register("acp", Box::new(Panics));
        let mut runtime = RunRuntime::with_parts(
            Arc::new(FakeIsolator::new()) as Arc<dyn Isolator>,
            Arc::new(FakeVerifier::new()),
            factory,
        )
        .with_clock(clock)
        .with_author(Arc::new(OutputAuthor))
        .with_role(Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        runtime
            .serve_request(
                &backend,
                &sink,
                1,
                RunRequest::Stream {
                    item: ids::HTUI_ANA_2,
                },
                &LiveChats::default(),
            )
            .await;

        for _ in 0..20 {
            runtime.sweep_with(&backend, &sink);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let at: Vec<u64> = sink
            .errors_of(ids::HTUI_ANA_2)
            .iter()
            .map(Duration::as_secs)
            .collect();
        assert_eq!(
            at.get(..2),
            Some(&[0, BACKOFF_FIRST.as_secs()][..]),
            "run {run}: the resume after a panic waits {BACKOFF_FIRST:?}, not the next sweep: {at:?}"
        );
    }

    /// A throwaway directory, removed with everything under it when the case ends.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("htui-worker-case-{}", Uuid::now_v7()));
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A real repository `name` with one commit under `root`, a non-primary repo of the demo
    /// project whose checkout on this box is it.
    async fn checkout(store: &MemStore, root: &std::path::Path, name: &str) -> RepoId {
        let path = root.join(name);
        std::fs::create_dir_all(&path).expect("the repository directory");
        htui_orch::isolate::git::testkit::repo_with_one_commit(&path);
        let id = RepoId::new();
        store
            .create_repo(NewRepo {
                id,
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: false,
            })
            .await
            .expect("the demo project holds no repo of this name");
        store
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: id,
                box_id: ids::BOX,
                local_path: path.to_string_lossy().into_owned(),
                updated_at: Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS),
            })
            .await
            .expect("both ids name rows");
        id
    }

    /// Polls until `run` has left `queued`.
    async fn claimed(store: &MemStore, run: RunId) {
        tokio::time::timeout(PATIENCE, async {
            while status_of(store, run).await == RunStatus::Queued {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {run} was not claimed within {PATIENCE:?}"));
    }

    /// MOD-41 review R-1 (blueprint D202): the worker has no `StartRun` to re-read the repo map,
    /// so its sweep does, once per sweep while no walk is live. A checkout registered after the
    /// isolator was built reaches the next claim: the run scoped to it walks, and is not failed
    /// with "no checkout for this repo on this box".
    #[tokio::test]
    async fn a_worker_sweep_picks_up_a_new_checkout() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let alpha = checkout(&store, &scratch.0, "alpha").await;
        let first = queued_over(
            &store,
            ids::HTUI_ANA_2,
            Utc::now() - TimeDelta::minutes(2),
            Some(&[alpha]),
        )
        .await;
        let mut factory = DriverFactory::new();
        factory.register("acp", Box::new(OneTurn));
        let mut runtime: RunRuntime<Backend, Timed> = RunRuntime::new(factory)
            .with_author(Arc::new(OutputAuthor))
            .with_role(Role::Worker)
            .with_scratch_root(scratch.0.join("trees"));
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();

        runtime.sweep_with(&backend, &sink);
        rests_at(&store, first, RunStatus::AwaitingApproval).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(runtime.isolator_builds(), 1, "built over `alpha` alone");
        store
            .finish_run(first, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the parked run is cancellable");

        let beta = checkout(&store, &scratch.0, "beta").await;
        let second = queued_over(&store, ids::HTUI_CLEAN_1, Utc::now(), Some(&[beta])).await;
        runtime.sweep_with(&backend, &sink);
        claimed(&store, second).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        let row = store.run(second).await.expect("the read").expect("the run");
        assert_eq!(
            row.status,
            RunStatus::AwaitingApproval,
            "the run over the new checkout walks: {:?}",
            row.failure
        );
        assert_eq!(
            runtime.isolator_builds(),
            2,
            "the sweep rebuilt the isolator over `alpha` and `beta`"
        );
    }

    /// The process's cached parts, read without re-reading the repo map.
    async fn parts(
        runtime: &RunRuntime<Backend, Timed>,
        backend: &Backend,
    ) -> (Arc<dyn Isolator>, Arc<dyn htui_orch::Verifier>) {
        let writer = htui_core::store::WorkerHost::writer(backend).expect("the backend writes");
        runtime
            .shared
            .singletons(backend, &writer, false)
            .await
            .expect("the parts are built")
    }

    /// MOD-41 review R-1 (blueprint D202, R-39): while a walk of the worker is live, a sweep that
    /// finds the repo map moved swaps nothing and claims nothing; the first sweep with no walk
    /// live rebuilds the isolator and the verifier, and claims the run over the new checkout.
    #[tokio::test]
    async fn a_worker_sweep_under_a_live_walk_keeps_its_parts() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let alpha = checkout(&store, &scratch.0, "alpha").await;
        let first = queued_over(
            &store,
            ids::HTUI_ANA_2,
            Utc::now() - TimeDelta::minutes(2),
            Some(&[alpha]),
        )
        .await;
        let mut factory = DriverFactory::new();
        factory.register("acp", Box::new(OneTurn));
        let mut runtime: RunRuntime<Backend, Timed> = RunRuntime::new(factory)
            .with_author(Arc::new(OutputAuthor))
            .with_role(Role::Worker)
            .with_scratch_root(scratch.0.join("trees"));
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        runtime.sweep_with(&backend, &sink);
        rests_at(&store, first, RunStatus::AwaitingApproval).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        store
            .finish_run(first, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the parked run is cancellable");
        let (isolator, verifier) = parts(&runtime, &backend).await;

        let beta = checkout(&store, &scratch.0, "beta").await;
        let second = queued_over(&store, ids::HTUI_CLEAN_1, Utc::now(), Some(&[beta])).await;
        let walk = runtime.shared.walks.child(RunId::new());
        runtime.sweep_with(&backend, &sink);
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(
            status_of(&store, second).await,
            RunStatus::Queued,
            "no claim walks with the stale isolator"
        );
        assert_eq!(
            runtime.isolator_builds(),
            1,
            "nothing is swapped under a live walk"
        );
        let (still, still_verifier) = parts(&runtime, &backend).await;
        assert!(Arc::ptr_eq(&isolator, &still) && Arc::ptr_eq(&verifier, &still_verifier));

        drop(walk);
        runtime.sweep_with(&backend, &sink);
        claimed(&store, second).await;
        assert!(runtime.settle(PATIENCE).await.is_empty());
        assert_eq!(
            status_of(&store, second).await,
            RunStatus::AwaitingApproval,
            "the run over the new checkout walks once the walk rests"
        );
        assert_eq!(
            runtime.isolator_builds(),
            2,
            "the rest's sweep rebuilt the isolator"
        );
        let (_, rebuilt) = parts(&runtime, &backend).await;
        assert!(
            !Arc::ptr_eq(&verifier, &rebuilt),
            "the verifier is rebuilt with the isolator, so the limits are read afresh"
        );
    }

    /// MOD-76 D4 (R-55): a runtime of `role` with the `acp` driver and a scratch root.
    fn limits_runtime(scratch: &Scratch, role: Role) -> RunRuntime<Backend, Timed> {
        let mut factory = DriverFactory::new();
        factory.register("acp", Box::new(OneTurn));
        RunRuntime::new(factory)
            .with_author(Arc::new(OutputAuthor))
            .with_role(role)
            .with_scratch_root(scratch.0.join("trees"))
    }

    /// One sweep of `runtime`, settled.
    async fn tick(runtime: &mut RunRuntime<Backend, Timed>, backend: &Backend, sink: &Timed) {
        runtime.sweep_with(backend, sink);
        assert!(runtime.settle(PATIENCE).await.is_empty());
    }

    /// The demo box's `settings.command_limits` set to `value`, which it did not hold.
    async fn set_limits(store: &MemStore, value: serde_json::Value) {
        let row = store
            .box_row(ids::BOX)
            .await
            .expect("the read answers")
            .expect("the demo box");
        assert_ne!(
            row.settings.get("command_limits"),
            Some(&value),
            "the limits change"
        );
        assert!(store.set_box_setting(ids::BOX, "command_limits", value));
    }

    /// MOD-76 D4 (R-55): with no walk live, a change to the box's `command_limits` alone (the
    /// repo map unchanged) reaches the worker's next sweep: a new verifier, the same isolator.
    #[tokio::test]
    async fn a_worker_sweep_applies_a_limits_change_alone() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let mut runtime = limits_runtime(&scratch, Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        tick(&mut runtime, &backend, &sink).await;
        let (isolator, verifier) = parts(&runtime, &backend).await;

        set_limits(&store, json!({ "verify": 2 })).await;
        tick(&mut runtime, &backend, &sink).await;
        let (still, rebuilt) = parts(&runtime, &backend).await;
        assert!(
            !Arc::ptr_eq(&verifier, &rebuilt),
            "the verifier is rebuilt from the new limits"
        );
        assert!(Arc::ptr_eq(&isolator, &still), "the isolator stays");
        assert_eq!(runtime.isolator_builds(), 1, "the repo map did not move");
    }

    /// MOD-76 D4 (R-55): a sweep that finds the limits unchanged keeps the verifier.
    #[tokio::test]
    async fn a_worker_sweep_with_the_same_limits_keeps_its_verifier() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let mut runtime = limits_runtime(&scratch, Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        tick(&mut runtime, &backend, &sink).await;
        let (_, verifier) = parts(&runtime, &backend).await;

        tick(&mut runtime, &backend, &sink).await;
        let (_, still) = parts(&runtime, &backend).await;
        assert!(Arc::ptr_eq(&verifier, &still));
        assert_eq!(runtime.isolator_builds(), 1);
    }

    /// MOD-76 D4 (R-55): while a walk is live, a limits change keeps the cached verifier and
    /// refuses nothing (two verifiers would be two `verify` semaphores); the first sweep after
    /// the walk rests rebuilds it.
    #[tokio::test]
    async fn a_limits_change_under_a_live_walk_waits_for_the_rest() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let mut runtime = limits_runtime(&scratch, Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        tick(&mut runtime, &backend, &sink).await;
        let (isolator, verifier) = parts(&runtime, &backend).await;

        let walk = runtime.shared.walks.child(RunId::new());
        set_limits(&store, json!({ "verify": 2 })).await;
        tick(&mut runtime, &backend, &sink).await;
        let writer = htui_core::store::WorkerHost::writer(&backend).expect("the backend writes");
        let (still, kept) = runtime
            .shared
            .singletons(&backend, &writer, true)
            .await
            .expect("a limits change under a live walk refuses nothing");
        assert!(
            Arc::ptr_eq(&verifier, &kept),
            "no verifier is swapped under a live walk"
        );
        assert!(Arc::ptr_eq(&isolator, &still));
        assert_eq!(runtime.isolator_builds(), 1);

        drop(walk);
        tick(&mut runtime, &backend, &sink).await;
        let (_, rebuilt) = parts(&runtime, &backend).await;
        assert!(
            !Arc::ptr_eq(&verifier, &rebuilt),
            "the rest's sweep rebuilds the verifier from the new limits"
        );
        assert_eq!(runtime.isolator_builds(), 1);
    }

    /// MOD-76 D4 (R-55): the TUI, which has no sweep re-read, applies a limits change at its next
    /// walking `StartRun`; a call that walks nothing keeps the verifier.
    #[tokio::test]
    async fn a_walking_start_run_applies_a_limits_change() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        let runtime = limits_runtime(&scratch, Role::Tui);
        let backend = Backend::memory(store.clone());
        super::Kit::read(&runtime.shared, &backend, true)
            .await
            .expect("the kit reads");
        let (_, verifier) = parts(&runtime, &backend).await;

        set_limits(&store, json!({ "verify": 2 })).await;
        super::Kit::read(&runtime.shared, &backend, false)
            .await
            .expect("the kit reads");
        let (_, cached) = parts(&runtime, &backend).await;
        assert!(
            Arc::ptr_eq(&verifier, &cached),
            "a call that is no walking `StartRun` serves the cached parts"
        );
        super::Kit::read(&runtime.shared, &backend, true)
            .await
            .expect("the kit reads");
        let (_, rebuilt) = parts(&runtime, &backend).await;
        assert!(
            !Arc::ptr_eq(&verifier, &rebuilt),
            "the walking `StartRun` rebuilds the verifier from the new limits"
        );
        assert_eq!(runtime.isolator_builds(), 1);
    }

    /// MOD-76 B-1 (R-55): a stored value that does not parse is compared as stored, so it
    /// builds the verifier once (warned once) and the next sweep keeps it.
    #[tokio::test]
    async fn a_bad_stored_limits_value_is_parsed_once() {
        if let Err(skip) = htui_orch::isolate::git::testkit::usable_git() {
            println!("{skip}");
            return;
        }
        let scratch = Scratch::new();
        let store = seeded(MemStore::demo()).await;
        set_executor(&store, Executor::Worker).await;
        let mut runtime = limits_runtime(&scratch, Role::Worker);
        let backend = Backend::memory(store.clone());
        let sink = Timed::new();
        tick(&mut runtime, &backend, &sink).await;
        let (_, verifier) = parts(&runtime, &backend).await;

        set_limits(&store, json!("not a map")).await;
        tick(&mut runtime, &backend, &sink).await;
        let (_, bad) = parts(&runtime, &backend).await;
        assert!(
            !Arc::ptr_eq(&verifier, &bad),
            "the stored value changed, so the verifier is built from it"
        );
        tick(&mut runtime, &backend, &sink).await;
        let (_, still) = parts(&runtime, &backend).await;
        assert!(
            Arc::ptr_eq(&bad, &still),
            "the same stored value is not parsed and built again"
        );
    }

    /// The runs `shared` keeps a backoff entry for.
    fn backed_off(shared: &super::Shared<Timed>) -> std::collections::BTreeSet<RunId> {
        shared
            .backoff
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
    }

    /// MOD-41 review R-8: a sweep's prune drops a backoff entry whose due is past by more than
    /// the grace (`max(BACKOFF_MAX, 2 x sweep period)`) and whose run no task works on; a fresh
    /// entry, one past its due by less than the grace, and a stale one whose run is live, stay.
    #[tokio::test(start_paused = true)]
    async fn prune_drops_a_stale_backoff_entry() {
        let runtime = worker_runtime().with_sweep_every(Duration::from_secs(1));
        let shared = &runtime.shared;
        let (stale, within, live, fresh) = (RunId::new(), RunId::new(), RunId::new(), RunId::new());
        shared.step_backoff(stale);
        shared.step_backoff(live);
        let walk = shared.walks.child(live);
        tokio::time::advance(Duration::from_secs(2)).await;
        // Past its due by `BACKOFF_MAX - 1s` when the prune runs: inside the grace.
        shared.step_backoff(within);
        tokio::time::advance(BACKOFF_FIRST + BACKOFF_MAX - Duration::from_secs(1)).await;
        shared.step_backoff(fresh);

        shared.prune();
        assert_eq!(
            backed_off(shared),
            [within, live, fresh].into_iter().collect(),
            "only the entry past its due by more than the grace, of a run no task works on, goes"
        );
        drop(walk);
    }

    /// MOD-41 review R-8: when twice the sweep period exceeds [`BACKOFF_MAX`], that is the grace,
    /// so a run still failing keeps its delay across a slow sweep.
    #[tokio::test(start_paused = true)]
    async fn a_slow_sweep_widens_the_backoff_grace() {
        let every = BACKOFF_MAX;
        let runtime = worker_runtime().with_sweep_every(every);
        let shared = &runtime.shared;
        let (stale, within) = (RunId::new(), RunId::new());
        shared.step_backoff(stale);
        tokio::time::advance(Duration::from_secs(2)).await;
        shared.step_backoff(within);
        // `stale` is past its due by `2 x every + 1s`, `within` by `2 x every - 1s`: both beyond
        // `BACKOFF_MAX`.
        tokio::time::advance(every * 2 + BACKOFF_FIRST - Duration::from_secs(1)).await;

        shared.prune();
        assert_eq!(
            backed_off(shared),
            [within].into_iter().collect(),
            "the grace is twice the sweep period, not {BACKOFF_MAX:?}"
        );
    }

    /// MOD-10 M3 T5 (blueprint §D.5): the worker's `Kit` owns the walk's `RunSecrets`, lent to
    /// the engine as its scrubber and its secrets (D11), over the runtime's source (D15).
    mod run_secrets {
        use std::collections::{BTreeMap, BTreeSet};
        use std::sync::Arc;

        use chrono::{TimeDelta, Utc};
        use htui_agent::conformance::{Script, ScriptEvent};
        use htui_agent::driver::{AgentDriver, DriverCaps};
        use htui_agent::error::DriverError;
        use htui_agent::event::{DoneEvent, DriverEvent, StopReason, TextChunk};
        use htui_agent::fake::{FakeDriver, SpecSlot};
        use htui_agent::registry::{DriverFactory, TransportBuilder};
        use htui_core::fixtures::{demo_data, ids};
        use htui_core::model::{Agent, AgentBox, Executor, ItemPatch, Run, RunStatus, RunStep};
        use htui_core::scrub::MinimalScrubber;
        use htui_core::secret::fake::{FakeSecretProvider, FakeSecretSource};
        use htui_core::secret::{INFISICAL, NO_SECRET_SOURCE, SecretError, SecretSource};
        use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
        use htui_orch::fake::{FakeIsolator, FakeVerifier};
        use htui_orch::{Isolator, ShellVerifier, SystemClock, Verifier};
        use htui_store::Backend;

        use super::{
            OutputAuthor, PATIENCE, Role, RunRuntime, Scratch, Timed, queued, seeded,
            seeded_with_primary, set_executor,
        };

        const SCOPE: &str = r#"{"project_id":"p1","environment":"dev","path":"/"}"#;
        /// Long and not pattern-shaped, so a refusal can never stand in for a mask.
        const VALUE: &str = "zq7-resolved-value-0123456789";
        /// A key no environment of this process holds, so its absence from a printed environment
        /// is the injection's absence.
        const KEY: &str = "MOD10_T5_RESOLVED";

        /// Every session of the scripted row: one assistant chunk of `text`, then `done`; the
        /// spec each `start` was handed lands in `slot`.
        #[derive(Debug)]
        struct Echoes {
            text: String,
            slot: SpecSlot,
        }

        impl TransportBuilder for Echoes {
            fn build(
                &self,
                agent: &Agent,
                _on_box: Option<&AgentBox>,
                caps: DriverCaps,
            ) -> Result<Box<dyn AgentDriver>, DriverError> {
                let turn = Script::one_turn(vec![
                    ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
                        text: self.text.clone(),
                        message_id: None,
                    })),
                    ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                        stop_reason: StopReason::EndTurn,
                    })),
                ]);
                Ok(Box::new(
                    FakeDriver::new(agent.name.clone(), caps, turn)
                        .with_spec_slot(self.slot.clone()),
                ))
            }
        }

        /// A worker-role runtime over `isolator` and `verifier` whose agent echoes `text`.
        fn runtime_over(
            text: &str,
            slot: &SpecSlot,
            isolator: Arc<dyn Isolator>,
            verifier: Arc<dyn Verifier>,
        ) -> RunRuntime<Backend, Timed> {
            let mut factory = DriverFactory::new();
            factory.register(
                "acp",
                Box::new(Echoes {
                    text: text.to_owned(),
                    slot: slot.clone(),
                }),
            );
            RunRuntime::with_parts(isolator, verifier, factory)
                .with_author(Arc::new(OutputAuthor))
                .with_role(Role::Worker)
        }

        /// [`runtime_over`] the fake isolator and verifier.
        fn echoing(text: &str, slot: &SpecSlot) -> RunRuntime<Backend, Timed> {
            runtime_over(
                text,
                slot,
                Arc::new(FakeIsolator::new()),
                Arc::new(FakeVerifier::new()),
            )
        }

        /// A source over a provider resolving `pairs`, both kept for their counters.
        fn source(pairs: &[(&str, &str)]) -> (Arc<FakeSecretProvider>, Arc<FakeSecretSource>) {
            let provider = Arc::new(FakeSecretProvider::resolving(pairs));
            let source = Arc::new(FakeSecretSource::new(
                Arc::clone(&provider) as Arc<dyn htui_core::secret::SecretProvider>
            ));
            (provider, source)
        }

        fn dyn_source(source: &Arc<FakeSecretSource>) -> Arc<dyn SecretSource> {
            Arc::clone(source) as Arc<dyn SecretSource>
        }

        /// `store`, already [`seeded`], on a `worker` box; `provider` gives the demo project an
        /// Infisical scope.
        async fn on_a_worker_box(store: MemStore, provider: bool) -> MemStore {
            set_executor(&store, Executor::Worker).await;
            if provider {
                store.set_project_secret_columns(ids::PROJECT_HTUI, Some(INFISICAL), Some(SCOPE));
            }
            store
        }

        /// One sweep that claims a fresh queued run of `ANA-2` and walks it; every task settled.
        async fn walked(runtime: &mut RunRuntime<Backend, Timed>, store: &MemStore) -> Run {
            let run = queued(store, ids::HTUI_ANA_2, Utc::now()).await;
            runtime.sweep_with(&Backend::memory(store.clone()), &Timed::new());
            assert!(runtime.settle(PATIENCE).await.is_empty());
            store.run(run).await.expect("the read").expect("the run")
        }

        /// The run's first step.
        async fn first_step(store: &MemStore, run: &Run) -> RunStep {
            store
                .run_steps(run.id)
                .await
                .expect("the read")
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("the run walked a step: {:?}", run.failure))
        }

        /// The env the session was handed.
        fn env_of(slot: &SpecSlot) -> BTreeMap<String, String> {
            slot.get().expect("the session started").env
        }

        /// MOD-10 D15: a runtime built `with_secret_source` hands it to every walk's
        /// `RunSecrets`: the claimed run's session gets the resolved map.
        #[tokio::test]
        async fn with_secret_source_reaches_the_engine() {
            let store = on_a_worker_box(seeded(MemStore::demo()).await, true).await;
            let (provider, source) = source(&[("API_KEY", VALUE)]);
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot).with_secret_source(dyn_source(&source));

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::AwaitingApproval, "{:?}", run.failure);
            assert_eq!(
                env_of(&slot),
                BTreeMap::from([("API_KEY".to_owned(), VALUE.to_owned())])
            );
            assert_eq!((source.calls(), provider.resolves()), (1, 1));
        }

        /// MOD-61 (blueprint §D.5): a walked step whose agent echoes a resolved value, whose
        /// primary repo is named with it (the trim record) and whose item body holds it (the
        /// prompt) stores it only as `[REDACTED]`, read back from the store.
        #[tokio::test]
        async fn mod61_a_walked_step_stores_a_resolved_value_only_redacted() {
            let store = seeded_with_primary(MemStore::demo(), VALUE).await;
            let store = on_a_worker_box(store, true).await;
            let row = store
                .item(ids::HTUI_ANA_2)
                .await
                .expect("the read")
                .expect("the item");
            store
                .update_item(
                    ids::HTUI_ANA_2,
                    row.version,
                    ItemPatch {
                        body: Some(format!("the token is {VALUE}, keep it")),
                        author_id: row.created_by,
                        reason: "a test's edit".to_owned(),
                        ..ItemPatch::default()
                    },
                )
                .await
                .expect("the item's version is current");
            let (_, source) = source(&[("API_KEY", VALUE)]);
            let slot = SpecSlot::default();
            let mut runtime = echoing(&format!("the key is {VALUE}, see"), &slot)
                .with_secret_source(dyn_source(&source));

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::AwaitingApproval, "{:?}", run.failure);
            assert_eq!(
                env_of(&slot).get("API_KEY").map(String::as_str),
                Some(VALUE),
                "the session got the value, so the masks below are not vacuous"
            );
            let step = first_step(&store, &run).await;
            let trim = step
                .trim_record
                .as_ref()
                .expect("the step recorded its trim record")
                .to_string();
            assert!(!trim.contains(VALUE), "{trim}");
            assert!(
                trim.contains("[REDACTED]"),
                "the repo slug was masked: {trim}"
            );
            let events = store
                .step_events(step.id)
                .await
                .expect("the read")
                .expect("the step recorded its session");
            let rows: Vec<String> = events
                .iter()
                .map(|event| event.payload.to_string())
                .collect();
            assert!(rows.iter().all(|row| !row.contains(VALUE)), "{rows:?}");
            let prompt = events
                .iter()
                .find(|event| event.seq == 0)
                .expect("seq 0 is the prompt")
                .payload
                .to_string();
            assert!(
                prompt.contains("the token is [REDACTED], keep it"),
                "{prompt}"
            );
            assert!(
                rows.iter()
                    .any(|row| row.contains("the key is [REDACTED], see")),
                "the echo was recorded masked: {rows:?}"
            );
        }

        /// MOD-62 (D19): the step's verify command prints its environment through the real
        /// `ShellVerifier`; the stored `command_run.output` shows the environment and neither the
        /// resolved value nor its key, while the agent's session did hold both.
        #[tokio::test]
        async fn mod62_a_verify_command_printing_its_environment_shows_no_resolved_value() {
            let scratch = Scratch::new();
            let mut data = demo_data();
            // Names only on Unix, plus the one variable's own value: a bare `env` prints every
            // value of the test process, and a pattern-shaped one there (an exported
            // `ANTHROPIC_API_KEY`) makes the verifier withhold the whole output.
            let print_env = if cfg!(windows) {
                "set".to_owned()
            } else {
                format!("env | cut -d= -f1; echo \"resolved=${{{KEY}-unset}}\"")
            };
            for phase in &mut data.phases {
                phase.verify_command = Some(print_env.clone());
            }
            let store = on_a_worker_box(seeded(MemStore::from_demo(data)).await, true).await;
            // The fake roots each tree at `<root>/<repo id>` and creates nothing: the verify
            // command runs in the primary's, so it must exist.
            for repo in store.repos(ids::PROJECT_HTUI).await.expect("the read") {
                std::fs::create_dir_all(scratch.0.join(repo.id.to_string()))
                    .expect("the primary tree's directory");
            }
            let isolator = FakeIsolator::new();
            isolator.root_trees_at(&scratch.0);
            let verifier = ShellVerifier::new(
                &BTreeMap::new(),
                Arc::new(MinimalScrubber::new(std::iter::empty::<String>())),
                Arc::new(SystemClock),
            );
            let (_, source) = source(&[(KEY, VALUE)]);
            let slot = SpecSlot::default();
            let mut runtime = runtime_over("done", &slot, Arc::new(isolator), Arc::new(verifier))
                .with_secret_source(dyn_source(&source));

            let run = walked(&mut runtime, &store).await;

            assert_eq!(
                env_of(&slot).get(KEY).map(String::as_str),
                Some(VALUE),
                "the agent's session held the value, so the test is not vacuous"
            );
            let step = first_step(&store, &run).await;
            let rows = store.command_runs(step.id).await.expect("the read");
            assert_eq!(rows.len(), 1, "one verify run: {rows:?}");
            let output = rows[0].output.clone().unwrap_or_default();
            assert!(
                output.contains("PATH"),
                "the command printed the environment: {output}"
            );
            assert!(!output.contains(VALUE), "{output}");
            if !cfg!(windows) {
                assert!(
                    output.contains("resolved=unset"),
                    "the verify child does not inherit the resolved key: {output}"
                );
            }
            assert!(
                !output.contains(KEY),
                "the verifier's child never received the resolved map: {output}"
            );
        }

        /// MOD-10 D12: a runtime without a source walks a provider-less project as before: to
        /// its gate, with an empty env. One with a source never touches it for that project.
        #[tokio::test]
        async fn a_runtime_without_a_source_walks_a_provider_less_project_as_before() {
            let store = on_a_worker_box(seeded(MemStore::demo()).await, false).await;
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot);

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::AwaitingApproval, "{:?}", run.failure);
            assert!(env_of(&slot).is_empty());

            let store = on_a_worker_box(seeded(MemStore::demo()).await, false).await;
            let (provider, source) = source(&[("API_KEY", VALUE)]);
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot).with_secret_source(dyn_source(&source));

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::AwaitingApproval, "{:?}", run.failure);
            assert!(env_of(&slot).is_empty());
            assert_eq!((source.calls(), provider.resolves()), (0, 0));
        }

        /// MOD-10 D13: a provider project in a runtime with no source fails the run with the
        /// refusal before any agent starts.
        #[tokio::test]
        async fn a_provider_project_without_a_source_fails_secrets_refused() {
            let store = on_a_worker_box(seeded(MemStore::demo()).await, true).await;
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot);

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::Failed);
            assert_eq!(
                run.failure,
                Some(SecretError::Config(NO_SECRET_SOURCE.to_owned()).refusal())
            );
            assert!(slot.get().is_none(), "no session started");
        }

        /// R-SEC-2: the agent's env holds the resolved keys and nothing else: no htui variable,
        /// no DSN, no Qdrant or Infisical setting.
        #[tokio::test]
        async fn r_sec_2_the_agent_env_holds_only_resolved_keys() {
            let store = on_a_worker_box(seeded(MemStore::demo()).await, true).await;
            let pairs = [
                ("API_KEY", VALUE),
                ("OTHER_TOKEN", "zq7-other-value-9876543210"),
            ];
            let (_, source) = source(&pairs);
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot).with_secret_source(dyn_source(&source));

            let run = walked(&mut runtime, &store).await;

            assert_eq!(run.status, RunStatus::AwaitingApproval, "{:?}", run.failure);
            let env = env_of(&slot);
            let keys: BTreeSet<&str> = env.keys().map(String::as_str).collect();
            assert_eq!(keys, pairs.iter().map(|(key, _)| *key).collect());
            for key in keys {
                assert!(!key.starts_with("HTUI_"), "{key}");
                assert!(
                    !["DATABASE_URL", "HTUI_TEST_DATABASE_URL"].contains(&key),
                    "{key}"
                );
                assert!(
                    !key.contains("QDRANT") && !key.contains("INFISICAL"),
                    "{key}"
                );
            }
        }

        /// MOD-10 D14: each task's `Kit` is its walk's `RunSecrets`, so two claimed runs resolve
        /// twice.
        #[tokio::test]
        async fn each_task_resolves_in_its_own_kit() {
            let store = on_a_worker_box(seeded(MemStore::demo()).await, true).await;
            let (provider, source) = source(&[("API_KEY", VALUE)]);
            let slot = SpecSlot::default();
            let mut runtime = echoing("hello", &slot).with_secret_source(dyn_source(&source));
            let earlier = queued(&store, ids::HTUI_ANA_2, Utc::now() - TimeDelta::minutes(2)).await;
            let later = queued(
                &store,
                ids::HTUI_CLEAN_1,
                Utc::now() - TimeDelta::minutes(1),
            )
            .await;
            let backend = Backend::memory(store.clone());
            let sink = Timed::new();

            runtime.sweep_with(&backend, &sink);
            assert!(runtime.settle(PATIENCE).await.is_empty());
            let row = store
                .run(earlier)
                .await
                .expect("the read")
                .expect("the run");
            assert_eq!(row.status, RunStatus::AwaitingApproval, "{:?}", row.failure);
            store
                .finish_run(earlier, RunStatus::Cancelled, None, Utc::now())
                .await
                .expect("the parked run is cancellable");
            runtime.sweep_with(&backend, &sink);
            assert!(runtime.settle(PATIENCE).await.is_empty());
            let row = store.run(later).await.expect("the read").expect("the run");
            assert_eq!(row.status, RunStatus::AwaitingApproval, "{:?}", row.failure);

            assert_eq!((source.calls(), provider.resolves()), (2, 2));
        }
    }
}

/// MOD-42 plan D9: the production policy lookup `Kit::read` builds.
#[cfg(test)]
mod kit_policy {
    use std::collections::HashMap;

    use chrono::DateTime;
    use htui_agent::driver::{PermissionDefault, PermissionPolicy};
    use htui_core::model::agent::seed_rows;
    use htui_core::model::{AgentId, AgentSummary};
    use serde_json::{Value, json};

    /// One seeded agent per `settings` document, keyed by its id, in the given order.
    fn agents(settings: [Value; 2]) -> (Vec<AgentId>, HashMap<AgentId, AgentSummary>) {
        let now = DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp");
        let summaries: Vec<AgentSummary> = seed_rows(now)
            .into_iter()
            .zip(settings)
            .map(|(mut agent, settings)| {
                agent.settings = settings;
                AgentSummary {
                    agent,
                    on_box: None,
                    user_off: false,
                }
            })
            .collect();
        let ids = summaries.iter().map(|summary| summary.agent.id).collect();
        let map = summaries
            .into_iter()
            .map(|summary| (summary.agent.id, summary))
            .collect();
        (ids, map)
    }

    #[test]
    fn each_agent_gets_its_own_parsed_policy_and_anything_else_asks() {
        let (ids, agents) = agents([
            json!({ "permission": { "default": "allow" } }),
            // Not a policy: the whole row does not parse, and it asks (fail closed).
            json!({ "permission": 42 }),
        ]);
        let policy = super::policy_lookup(&agents);

        assert_eq!(policy(ids[0]).default, PermissionDefault::Allow);
        assert_eq!(policy(ids[1]), PermissionPolicy::default());
        assert_eq!(policy(ids[1]).default, PermissionDefault::Ask);
        assert_eq!(
            policy(AgentId::new()),
            PermissionPolicy::default(),
            "an agent the task did not read asks"
        );
    }
}
