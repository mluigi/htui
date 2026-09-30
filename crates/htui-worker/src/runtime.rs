//! The run runtime (MOD-4 milestone 6, plan D153; MOD-41 plan D6, D7): every command on a task of
//! its own, serialised per run, supervised, swept.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::AgentDriver;
use htui_agent::error::DriverError;
use htui_agent::registry::DriverFactory;
use htui_core::model::{
    AgentId, AgentSummary, BoxId, BoxProfile, Executor, ItemId, RepoId, Run, RunId, RunStatus,
    SnapshotCandidate, UserId,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{Result as StoreResult, StoreError};
use htui_orch::{
    Adopted, Clock, Command, CommandOutcome, DeadWalks, DriverFor, Engine, EngineError,
    EngineParts, FirstCandidate, GixIsolator, Isolator, IsolatorConfig, LeaseTimes, Next,
    OpeningPath, RepoCheckout, Rest, Resume, RunFence, SessionKey, ShellVerifier, SystemClock,
    Tails, UnblockCase, Verifier, cleanup_enabled,
};
use htui_store::{DATABASE_UNREACHABLE, identity};
use serde_json::Value;
use tokio::sync::{OwnedMutexGuard, mpsc, oneshot};
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

/// OQ-4: `c` on a run the box's worker is walking.
#[must_use]
pub fn worker_walks(run: RunId) -> String {
    format!(
        "the worker on this box is walking run {run}; cancelling a live run needs MOD-42's cancel command"
    )
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
}

/// Everything the runtime's tasks share. `P` is where the runtime's answers go (MOD-41 plan D7).
struct Shared<P: ReplySink> {
    /// I-1: which process this runtime is.
    role: Role,
    parts: Parts,
    drivers: Arc<DriverFactory>,
    clock: Arc<dyn Clock>,
    author: Option<Arc<dyn StepAuthor>>,
    owner: Uuid,
    dead_walks: Arc<DeadWalks>,
    publisher: Publisher<P>,
    events: mpsc::UnboundedSender<RunServed<P::Addr>>,
    tasks: StdMutex<Vec<Tracked>>,
    isolator_builds: AtomicUsize,
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

    /// D214 (review L5): one lazy pass per sweep tick — the finished task handles, the parent
    /// tokens of runs no task works on, and the run locks nobody holds or waits for — so an idle
    /// session keeps none of them for the life of the process.
    fn prune(&self) {
        self.prune_tasks();
        self.walks.prune();
        self.locks.prune();
    }

    /// The process's isolator and verifier (D156, D202). A `StartRun` re-reads the repo map and
    /// rebuilds the production isolator when it moved and no walk of this process is live, and is
    /// refused with [`REPOS_MOVED`] when one is (R-39).
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
            self.isolator_builds.fetch_add(1, Ordering::SeqCst);
        }
        if built.verifier.is_none() {
            let limits = command_limits(host, box_id)
                .await
                .map_err(|err| err.to_string())?;
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

/// A run's parent token and how many tasks work under it.
#[derive(Debug, Clone)]
struct Parent {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
}

/// One task's child token; dropping it is the task no longer working on the run.
#[derive(Debug)]
pub struct WalkToken {
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
        self.parents.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A child of `run`'s parent, minting the parent on first use; cancelled from birth once the
    /// runtime is closed.
    fn child(&self, run: RunId) -> WalkToken {
        let mut walks = self.lock();
        let parent = walks.entry(run).or_insert_with(|| Parent {
            token: self.root.child_token(),
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

/// The box row's `settings.command_limits`, else `{"verify": 1}` (D156): no row, no key, or a
/// stored value that does not parse (warned) all get the default.
///
/// D216 (review L7): a read that fails is not the default. `singletons` passes it up like the
/// reads beside it, so no verifier is cached from it and the next command reads again. The limits
/// are read once per process (per server): an edit to them reaches the next process (R-55).
///
/// # Errors
/// The store's own read failure.
async fn command_limits<H: htui_core::store::WorkerHost>(
    host: &H,
    box_id: BoxId,
) -> StoreResult<BTreeMap<String, u32>> {
    let default = || BTreeMap::from([("verify".to_owned(), 1)]);
    let Some(stored) = host
        .box_row(box_id)
        .await?
        .and_then(|row| row.settings.get("command_limits").cloned())
    else {
        return Ok(default());
    };
    Ok(serde_json::from_value(stored).unwrap_or_else(|err| {
        tracing::warn!(%box_id, %err, "box.settings.command_limits does not parse; verify runs one at a time");
        default()
    }))
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

/// Everything one task's engine borrows, owned (D156): the writer, the graph source, the two
/// singletons, the clock, the sink, the identities, and the agent registry read once per task.
struct Kit<H: htui_core::store::WorkerHost> {
    writer: H::Store,
    graphs: HostGraphs<H>,
    isolator: Arc<dyn Isolator>,
    verifier: Arc<dyn Verifier>,
    clock: Arc<dyn Clock>,
    sink: ProgressSink<H::Store>,
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
        let (isolator, verifier) = shared.singletons(host, &writer, start_run).await?;
        let sentence = |err: StoreError| err.to_string();
        let box_id = registered_box(host).await.map_err(sentence)?;
        let user = host.this_user().await.map_err(sentence)?;
        let app = host.app_settings().await.map_err(sentence)?;
        let box_profile = host
            .box_profile(box_id)
            .await
            .map_err(sentence)?
            .ok_or_else(|| "this box has no profile row".to_owned())?;
        let agents = host
            .agents()
            .await
            .map_err(sentence)?
            .into_iter()
            .map(|summary| (summary.agent.id, summary))
            .collect();
        Ok(Self {
            sink: ProgressSink {
                publisher: Arc::new(shared.publisher.clone()),
                writer: writer.clone(),
                author: shared.author.clone(),
            },
            writer,
            graphs: HostGraphs(host.clone()),
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
            scrubber: &self.scrubber,
            app: self.app.clone(),
            box_profile: self.box_profile.clone(),
            box_id: self.box_id,
            owner: self.owner,
            dead_walks: &self.dead_walks,
            user: self.user,
            tails: Tails::Walk,
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
                ended: AtomicU64::new(0),
                server: AtomicU64::new(0),
                sweep_every: AtomicU64::new(millis(lease_period(&BTreeMap::new()))),
                sweep_fixed: false,
                sweeping: AtomicBool::new(false),
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

    /// How many isolators this process has built (D156's test hook).
    #[must_use]
    pub fn isolator_builds(&self) -> usize {
        self.shared.isolator_builds.load(Ordering::SeqCst)
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
    /// aborted. The loop cancels the chats beside this, inside the same bounded quit.
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
                return;
            }
            for Tracked { tag, handle } in tasks {
                let abort = handle.abort_handle();
                if tokio::time::timeout_at(deadline, handle).await.is_err() {
                    abort.abort();
                    tracing::warn!(run = ?tag.run.get(), "a run task did not end within the grace window");
                }
            }
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
    if ctx.shared.dead_walks.runs().is_empty()
        && matches!(host.active_runs_on_box(box_id).await, Ok(0))
    {
        return;
    }
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

/// D158: an adopted run's walk, resumed on its own task under its lock.
async fn resumed<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId) {
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
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
            ctx.publish(Some(run), FrameKind::Rested(rest));
        }
        Some(Err(err)) => ctx.refuse(err.to_string()),
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
    let _ = ctx.tag.run.set(run);
    ctx.publish(Some(run), FrameKind::Started);
    // M5 D84: read now, so a refused claim joins the queue with no await after the refusal.
    let queued_at = htui_core::store::WorkerStore::run(&kit.writer, run)
        .await
        .ok()
        .flatten()
        .map_or_else(|| ctx.shared.clock.now(), |row| row.queued_at);

    let walk = ctx.shared.walks.child(run);
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
    let early = ctx.tag(run).await;
    if moves_the_run(&command) && !live.is_empty() {
        let refusal = match ctx.host.run_steps(run).await {
            Ok(steps) => chat_free(&steps, live).err().map(|err| err.to_string()),
            Err(err) => Some(err.to_string()),
        };
        if let Some(refusal) = refusal {
            return ctx.refuse(refusal);
        }
    }
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
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    // `project_id` never changes, so the row read before the lock stands in for a failed one.
    let row = ctx.tag_run(&kit.writer, run).await.or(early);
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
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
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
async fn cleanup<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId) {
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
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

    /// `command_limits`: the box row's `settings.command_limits`, else the default.
    ///
    /// # Errors
    /// The store's own read failure.
    pub async fn command_limits<H: htui_core::store::WorkerHost>(
        host: &H,
        box_id: BoxId,
    ) -> StoreResult<BTreeMap<String, u32>> {
        super::command_limits(host, box_id).await
    }

    /// The verdicts of an item the mirror does not hold, off the server: nothing is enabled.
    #[must_use]
    pub fn unreachable_actions(item: ItemId, key: String) -> ItemActions {
        crate::views::unreachable_actions(item, key)
    }
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
        NewRepo, NewRun, RepoId, RunId, RunMode, RunStatus, RunStep, SnapshotPhase,
        TIMESTAMPTZ_DIGITS, Transport,
    };
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use htui_orch::fake::{FakeIsolator, FakeVerifier};
    use htui_orch::{Clock, Isolator};
    use htui_store::Backend;
    use serde_json::json;
    use uuid::Uuid;

    use super::{Role, RunRuntime};
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
                name: "htui".to_owned(),
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
            None,
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
}
