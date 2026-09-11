//! Live chat sessions, owned by the store worker's task (MOD-2 plan D27, D28).
//!
//! One [`AgentRuntime`] lives inside the store worker loop, because a chat needs two things only
//! that loop has: the [`Backend`] (for the writer, the box, the user and the registry row) and the
//! reply channel every view is answered through. The chat tab holds neither — it asks for a chat
//! with [`StoreRequest::ChatStart`] and is answered
//! in `on_reply`, exactly as it is for a list of items (`R-NF-3`).
//!
//! **The stream is one request, many replies.** Every frame a session produces is sent as a
//! [`ReplyEnvelope`] carrying the `ChatStart` request's own `seq` and origin, so `App::is_fresh`
//! passes all of them for as long as that chat is the tab's newest `ChatStart`
//! (`docs/ANA-4.md` §8: the stream gets its own discriminant and nothing else uses it).
//!
//! **`run_chat` is a plain `async fn`, not a spawned task.** Production spawns it; the test
//! harness awaits it inline, which is what keeps chat-tab snapshots byte-stable with no sleeps —
//! the same trade `Harness::settle` makes for store requests.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::acp::SESSION_STARTED;
use htui_agent::auth::{
    AUTH_IDLE_CAP, AuthChoice, AuthEvent, AuthFlow, AuthOutcome, BrowserPolicy, OpenerCommand,
    open_url,
};
use htui_agent::driver::{
    AgentDriver, AgentSession, DriverCaps, PermissionAnswer, PermissionPolicy, PermissionRequestId,
    SessionSpec,
};
use htui_agent::error::DriverError;
use htui_agent::event::{DriverEnvelope, DriverEvent, StopReason, ToolCallEvent};
use htui_agent::install::{
    InstallConfig, InstallError, InstallJob, InstallOutcome, InstallPlan, InstallProgress,
    Installer, PlanError, install, plan as plan_install,
};
use htui_agent::launch::{AgentLaunch, AgentSettings};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeStatus, SpawnTier2, probe_agent,
};
use htui_agent::record::{
    AnsweredBy, CapBreach, QuotaLatch, Recorder, RunCap, enforce_breach as enforce_cap_breach,
};
use htui_agent::registry::{DriverFactory, caps_for};
use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, ChatRunSpec, PER_TOKEN_CAP_BATCH, ProjectCaps, ProjectId,
    QuotaSource, RunStatus, StepId, Transport,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{StoreError, WriteStore};
use htui_store::{Backend, Writer};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::store_worker::{
    AuthFrame, ChatFrame, InstallFrame, Origin, ReplyEnvelope, RequestEnvelope, Seq, StoreReply,
    StoreRequest,
};

/// How long a cancelled session may take the graceful path before its tree is killed.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// Depth of the recorder's render channel.
///
/// Drained by the same task after every `record`, so a full channel is structurally impossible
/// here; the bound exists so a bug cannot buffer a turn without limit.
pub const UI_FRAMES: usize = 256;

/// How old an `agent_box` row may be before a chat starting on it triggers a re-probe.
///
/// `docs/ANA-4.md`:791-793 splits the probe's triggers three ways, and this is the lazy one:
/// "before the first session of the day", because `agy` self-updates in place and a row written
/// yesterday may describe a binary that no longer exists. 24 hours (MOD-2 plan D55).
pub const PROBE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// `HTUI_KEEP_RAW_EVENTS=1` keeps the verbatim wire message on every row (plan D29).
///
/// `project.settings.keep_raw_events` is the real source and has no reader until MOD-15; this is
/// the stand-in, and the default is `false`.
pub const KEEP_RAW_ENV: &str = "HTUI_KEEP_RAW_EVENTS";

/// Where a reply goes: the request that asked, by `seq` and origin.
#[derive(Debug, Clone)]
pub struct ReplyAddr {
    /// The `seq` of the request being answered.
    pub seq: Seq,
    /// Who asked.
    pub origin: Origin,
}

/// What the worker asks a live chat to do. Each carries the address of the request that asked, so
/// exactly one reply goes back for it.
#[derive(Debug)]
pub enum ChatCommand {
    /// A follow-up turn.
    Send {
        /// The user's text.
        text: String,
        /// Who to answer.
        reply: ReplyAddr,
    },
    /// An answer to a parked permission request.
    Answer {
        /// Which request.
        request_id: PermissionRequestId,
        /// The chosen option, or a cancellation.
        answer: PermissionAnswer,
        /// Who to answer.
        reply: ReplyAddr,
    },
    /// End the session.
    Cancel {
        /// Who to answer; `None` when the runtime is shutting every chat down.
        reply: Option<ReplyAddr>,
    },
}

/// One live chat, as the worker sees it.
pub struct LiveChat {
    /// The command channel into [`run_chat`].
    commands: mpsc::UnboundedSender<ChatCommand>,
    /// What the driver can do, for the tab's capability banner.
    caps: DriverCaps,
    /// The spawned task, when production spawned one.
    task: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for LiveChat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveChat")
            .field("caps", &self.caps)
            .field("closed", &self.commands.is_closed())
            .field("spawned", &self.task.is_some())
            .finish()
    }
}

/// Which half of an install is running (MOD-20 D18).
///
/// One claim covers both, because they are one action from the user's side and because two of
/// them for the same registry id would race each other on `.staging/` (blueprint H-10). The
/// distinction is kept anyway: what a stuck runtime is doing is the first question asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivePhase {
    /// A pre-flight is reading the registry.
    Planning,
    /// A confirmed plan is being fetched, unpacked and probed.
    Installing,
}

/// The one install this runtime allows at a time (MOD-20 D18).
///
/// The token, not the handle, is how this is stopped: `abort()` drops a future at its next await
/// and can neither sweep the staging entry nor send the frame the section is waiting on — and it
/// does not stop the blocking thread an unpack runs on at all (blueprint P-3, hazard H-8).
/// `AgentRuntime::shutdown` therefore cancels **before** it aborts.
pub struct LiveInstall {
    /// Which registry row this install is for.
    agent_id: AgentId,
    /// Planning or installing; both hold the claim.
    phase: LivePhase,
    /// Tripped by [`StoreRequest::InstallCancel`] and by shutdown.
    cancel: CancellationToken,
    /// The task that answers the request.
    task: JoinHandle<()>,
}

impl core::fmt::Debug for LiveInstall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveInstall")
            .field("agent_id", &self.agent_id)
            .field("phase", &self.phase)
            .field("finished", &self.task.is_finished())
            .finish()
    }
}

/// The one refusal of an `auth_choose` that leaves the login **running** (review L-4).
///
/// `run_auth` takes the choice sender exactly once, so a second choice is refused rather than
/// dropped — the pane hears back for every request it spends. But the other two refusals of that
/// request ("no login is running", "this login has ended") mean the flow is *gone* and this one
/// means it is very much there, and a section that treated them alike would clear its pane and
/// leave a live adapter with no `x` to cancel it.
///
/// A constant rather than a literal at each end because both ends are in this crate: the settings
/// section matches on this exact value, and a sentence the compiler links is not a sentence one
/// side can reword without the other.
pub const AUTH_ALREADY_CHOSEN: &str = "a method was already chosen";

/// What the worker asks a live login to do (MOD-21 D18).
///
/// The [`ChatCommand`] shape, and for the same reason: there are two things a running flow can be
/// told, each answered at the address of the request that asked. A `oneshot` on the runtime would
/// carry one of them, and the opener has to run somewhere the runtime can name — `background`
/// would trip `AgentRuntime::claim_is_free`'s "a probe is running", and a bare `tokio::spawn`
/// would be a task nobody owns.
#[derive(Debug)]
pub enum AuthCommand {
    /// The user picked from the live method list.
    Choose {
        /// The method, or a logout.
        choice: AuthChoice,
        /// Who to answer, and whose `seq` every later frame of the flow carries.
        reply: ReplyAddr,
    },
    /// Open the link the pane is showing.
    Open {
        /// The link, as the adapter printed it.
        url: String,
        /// Who to answer, once.
        reply: ReplyAddr,
    },
}

/// The one login this runtime allows at a time (MOD-21 D18, D19).
///
/// Cancelled through the token rather than aborted, for [`LiveInstall`]'s reason: the task owes
/// its stream a last frame, and it owns the child that has to be killed and reaped before it can
/// send one.
pub struct LiveAuth {
    /// Which registry row this login is for.
    agent_id: AgentId,
    /// Tripped by [`StoreRequest::AuthCancel`], by shutdown, and (as its child, inside
    /// `htui-agent`) by the idle clock.
    cancel: CancellationToken,
    /// Into [`run_auth`]. Closed means the flow has ended.
    commands: mpsc::UnboundedSender<AuthCommand>,
    /// The task that answers the request.
    task: JoinHandle<()>,
    /// MOD-21 D19: the row's `(agent_id, box_id)` re-probe claim, held for the flow's **whole**
    /// life so a chat started during the browser round trip cannot re-probe the row under it and
    /// record `unauthenticated` seconds before the flow's own probe records `ready` (blueprint
    /// H-16). Released by `Drop` when the runtime lets go of this value.
    _claim: ReprobeClaim,
}

impl core::fmt::Debug for LiveAuth {
    /// Ids and finishedness, never a line, a link or an environment (`R-SEC-2`, blueprint H-5).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveAuth")
            .field("agent_id", &self.agent_id)
            .field("finished", &self.task.is_finished())
            .field("closed", &self.commands.is_closed())
            .finish()
    }
}

/// A chat's session future: production spawns it, the harness polls it inline (plan D30).
pub type ChatTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// What [`AgentRuntime::serve`] decided about one request.
pub enum Served {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// A task the runtime owns answers this request itself, exactly once — the session task for a
    /// chat command, the probe task for [`StoreRequest::ProbeAgents`].
    Deferred,
    /// A chat is starting; the caller spawns (or polls) the future and attaches the handle.
    Start {
        /// The step the chat records against.
        step_id: StepId,
        /// The session future.
        task: ChatTask,
    },
}

impl core::fmt::Debug for Served {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Reply(reply) => f.debug_tuple("Reply").field(reply).finish(),
            Self::Deferred => f.write_str("Deferred"),
            Self::Start { step_id, .. } => {
                f.debug_struct("Start").field("step_id", step_id).finish()
            }
        }
    }
}

/// Every live chat this process owns.
pub struct AgentRuntime {
    factory: DriverFactory,
    live: HashMap<StepId, LiveChat>,
    started: Vec<StepId>,
    grace: Duration,
    /// Tasks this runtime spawned that answer a request of their own: today the probe's (MOD-2
    /// D53). Swept when finished, awaited by [`finish_background`](Self::finish_background),
    /// aborted by [`shutdown`](Self::shutdown).
    ///
    /// The runtime owns the handle for the same reason it owns a chat's: a bare `tokio::spawn`
    /// inside the worker loop would leave a probe with a 60-second handshake running after the UI
    /// is gone, with nobody able to name it.
    background: Vec<JoinHandle<()>>,
    /// The rows a `ChatStart` re-probe is running for, so two overlapping chats do not each start
    /// one for the same `(agent_id, box_id)` (blueprint H-9).
    ///
    /// On the runtime because that is the only thing both triggers outlive: the staleness re-probe
    /// is a task this struct owns, the D60 one runs inside a chat task, and neither can see the
    /// other from where it lives.
    reprobe_claims: ReprobeClaims,
    /// Where an install would read the registry from and write its tree to (MOD-20 D18).
    ///
    /// `None` until [`with_installer`](Self::with_installer) or [`production`](Self::production):
    /// a runtime built by [`new`](Self::new) refuses `i` by saying it has no installer, so no test
    /// can reach the real registry by forgetting to inject one.
    ///
    /// The **config**, not an [`Installer`]: the clients are built inside the task that uses them
    /// (blueprint P-13), which is what keeps a client that cannot be built from panicking on the
    /// loop and keeps `production()` from opening a socket nobody asked for.
    installer: Option<InstallConfig>,
    /// The install running right now, if any.
    install: Option<LiveInstall>,
    /// The login running right now, if any (MOD-21 D18).
    ///
    /// One deep, and it shares [`claim_is_free`](Self::claim_is_free) with the install and the
    /// probe: all three end by writing `agent_box`, and two of them at once are a last-write-wins
    /// on one row (MOD-21 D19).
    auth: Option<LiveAuth>,
    /// What [`StoreRequest::AuthOpen`] spawns (MOD-21 D17, blueprint P-5).
    ///
    /// [`OpenerCommand::Platform`] in production. The injected seam a login case needs: `set_var`
    /// is forbidden here, so a test that must not launch the maintainer's browser says so as data.
    opener: OpenerCommand,
}

impl core::fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("adapters", &self.factory.adapter_ids())
            .field("live", &self.live.len())
            .finish()
    }
}

impl AgentRuntime {
    /// A runtime over a transport registry.
    #[must_use]
    pub fn new(factory: DriverFactory) -> Self {
        Self {
            factory,
            live: HashMap::new(),
            started: Vec::new(),
            grace: CANCEL_GRACE,
            background: Vec::new(),
            reprobe_claims: ReprobeClaims::default(),
            installer: None,
            install: None,
            auth: None,
            opener: OpenerCommand::Platform,
        }
    }

    /// The production runtime: the ACP transport and nothing else (milestone 8 adds the CLI one).
    ///
    /// It carries an installer, and carrying one costs nothing until the user presses `i`: the
    /// value is where the registry is and where a tree may be written, and the HTTP clients it
    /// implies are built inside the install task (blueprint P-13).
    #[must_use]
    pub fn production() -> Self {
        Self::new(DriverFactory::with_acp()).with_installer(InstallConfig::default())
    }

    /// A runtime that installs from this registry, into this root (MOD-20 D18).
    ///
    /// The injected seam the item is built on: `set_var` is forbidden here, so a test that must
    /// keep off the real registry and the maintainer's real install root says so as data.
    #[must_use]
    pub fn with_installer(mut self, config: InstallConfig) -> Self {
        self.installer = Some(config);
        self
    }

    /// Whether an install — a pre-flight or a confirmed one — is running.
    ///
    /// One claim for both, because they are one action and two of them for the same registry id
    /// would race on `.staging/` (blueprint H-10).
    #[must_use]
    pub fn install_running(&self) -> bool {
        self.install
            .as_ref()
            .is_some_and(|live| !live.task.is_finished())
    }

    /// A runtime that opens links with this command (MOD-21 D17, blueprint P-5).
    ///
    /// Production leaves it at [`OpenerCommand::Platform`]. A test injects a script that records
    /// what it was handed, which is the only way `o` is provable without a browser.
    #[must_use]
    pub fn with_opener(mut self, opener: OpenerCommand) -> Self {
        self.opener = opener;
        self
    }

    /// Whether a login is running.
    ///
    /// The same shape as [`install_running`](Self::install_running) and for the same reason: a
    /// flow that has answered still occupies the slot until the next [`serve`](Self::serve)
    /// sweeps it (blueprint H-25), and what a case asks about is the *task*.
    #[must_use]
    pub fn auth_running(&self) -> bool {
        self.auth
            .as_ref()
            .is_some_and(|live| !live.task.is_finished())
    }

    /// A runtime whose cancels use this grace window. Tests use zero.
    #[must_use]
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Every step this runtime has started, oldest first.
    #[must_use]
    pub fn steps(&self) -> Vec<StepId> {
        self.started.clone()
    }

    /// How many background tasks this runtime still owns.
    ///
    /// The one fact a test needs to prove D52: a refused probe spawned **nothing**, rather than
    /// spawning and then discarding.
    #[must_use]
    pub fn background_len(&self) -> usize {
        self.background.len()
    }

    /// Awaits every background task, each under `limit`; one past it is aborted and named.
    ///
    /// The deterministic end the test harness needs: a probe answers through the reply channel
    /// from its own task, so a harness that rendered before this returned would photograph a probe
    /// that had not finished. A live install is one of those tasks and is awaited here too
    /// (blueprint P-2) — it is *not* in `background`, because the runtime has to be able to name
    /// it and cancel it by itself.
    ///
    /// The token is tripped before the abort for the reason
    /// [`shutdown`](Self::shutdown) does it: an abort cannot stop a blocking unpack.
    pub async fn finish_background(&mut self, limit: Duration) {
        for handle in std::mem::take(&mut self.background) {
            let abort = handle.abort_handle();
            if tokio::time::timeout(limit, handle).await.is_err() {
                abort.abort();
                tracing::warn!(?limit, "a background task did not finish and was aborted");
            }
        }
        if let Some(LiveInstall { cancel, task, .. }) = self.install.take() {
            let abort = task.abort_handle();
            if tokio::time::timeout(limit, task).await.is_err() {
                cancel.cancel();
                abort.abort();
                tracing::warn!(?limit, "an install did not finish and was aborted");
            }
        }
        // The identical arm for a login (blueprint P-2): awaited first, cancelled and aborted only
        // when it overruns. A flow parked on a human therefore costs a caller the whole `limit`,
        // which is why a harness case watching a live login drives and sleeps rather than calling
        // this.
        if let Some(LiveAuth { cancel, task, .. }) = self.auth.take() {
            let abort = task.abort_handle();
            if tokio::time::timeout(limit, task).await.is_err() {
                cancel.cancel();
                abort.abort();
                tracing::warn!(?limit, "a login did not finish and was aborted");
            }
        }
    }

    /// Records the task handle of a chat the caller spawned.
    pub fn attach(&mut self, step_id: StepId, task: JoinHandle<()>) {
        if let Some(chat) = self.live.get_mut(&step_id) {
            chat.task = Some(task);
        }
    }

    /// What a tab may ask about a chat that is running.
    #[must_use]
    pub fn caps(&self, step_id: StepId) -> Option<DriverCaps> {
        self.live.get(&step_id).map(|chat| chat.caps)
    }

    /// Serves one request the agent runtime owns: the four chat variants, `ProbeAgents`, and
    /// MOD-20's three install variants.
    ///
    /// # Panics
    ///
    /// Never: a request this runtime does not own is answered with a `Failed` naming it rather
    /// than by panicking, because the worker's match is the only caller and a widened enum should
    /// not become a crash.
    pub async fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
    ) -> Served {
        // A finished chat leaves its entry behind; sweep before anything looks one up, so a second
        // chat on a finished step is a start and not a "no live chat".
        self.live.retain(|_, chat| !chat.commands.is_closed());
        // The same sweep for the tasks that answer their own request: a probe that has answered is
        // not a probe still running, and `background_len` is what a test reads.
        self.background.retain(|task| !task.is_finished());
        // And for the install claim, which is one deep: an install that has answered must not go
        // on refusing the next `i` (blueprint H-10).
        self.install.take_if(|live| live.task.is_finished());
        // And for the login claim, which is one deep for the same reason. Dropping the value here
        // is also what releases its `ReprobeClaim`: a finished flow must not go on excluding the
        // chat re-probe of its own row (blueprint H-25).
        self.auth.take_if(|live| live.task.is_finished());

        let addr = ReplyAddr {
            seq: envelope.seq,
            origin: envelope.origin.clone(),
        };
        match &envelope.request {
            StoreRequest::ChatStart {
                project_id,
                agent_id,
                model,
                prompt,
            } => {
                match self
                    .start(
                        backend,
                        replies,
                        addr,
                        *project_id,
                        *agent_id,
                        model.clone(),
                        prompt.clone(),
                    )
                    .await
                {
                    Ok(started) => started,
                    Err(err) => Served::Reply(failed("chat_start", &err)),
                }
            }
            StoreRequest::ChatSend { step_id, text } => self.command(
                *step_id,
                "chat_send",
                ChatCommand::Send {
                    text: text.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::ChatAnswer {
                step_id,
                request_id,
                answer,
            } => self.command(
                *step_id,
                "chat_answer",
                ChatCommand::Answer {
                    request_id: request_id.clone(),
                    answer: answer.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::ChatCancel { step_id } => self.command(
                *step_id,
                "chat_cancel",
                ChatCommand::Cancel { reply: Some(addr) },
            ),
            StoreRequest::ProbeAgents => match self.probe(backend, replies, addr).await {
                Ok(served) => served,
                Err(err) => Served::Reply(failed("probe_agents", &err)),
            },
            StoreRequest::InstallPlan { agent_id } => {
                match self.install_plan(backend, replies, addr, *agent_id).await {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed("install_plan", &err)),
                }
            }
            StoreRequest::InstallConfirm { plan } => {
                match self
                    .install_confirm(backend, replies, addr, plan.clone())
                    .await
                {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed("install_confirm", &err)),
                }
            }
            StoreRequest::InstallCancel => self.install_cancel(),
            StoreRequest::AuthStart { agent_id } => {
                match self.auth_start(backend, replies, addr, *agent_id).await {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed("auth_start", &err)),
                }
            }
            StoreRequest::AuthChoose { choice } => self.auth_command(
                "auth_choose",
                AuthCommand::Choose {
                    choice: choice.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::AuthOpen { url } => self.auth_command(
                "auth_open",
                AuthCommand::Open {
                    url: url.clone(),
                    reply: addr,
                },
            ),
            StoreRequest::AuthCancel => self.auth_cancel(),
            other => Served::Reply(StoreReply::Failed {
                request: other.name(),
                message: "not a chat request".to_owned(),
            }),
        }
    }

    /// Cancels every live chat and waits for it, then gives up on stragglers.
    ///
    /// Called when the UI is gone. Without it the runtime drops every session task at its first
    /// await and the agent processes are orphaned (`docs/ANA-4.md` §11 criterion 11).
    ///
    /// Background tasks are aborted rather than awaited, after the chats are down: a probe's child
    /// dies with the guard the handshake holds it in, and reaping it at process exit buys nothing
    /// but a wait on a handshake nobody will read.
    ///
    /// **The install is cancelled before it is aborted**, and it is the one place shutdown does
    /// more than `abort()` (blueprint P-3). `JoinHandle::abort` drops a future at its next await
    /// and does nothing at all to the `spawn_blocking` thread an unpack runs on, so a shutdown
    /// mid-unpack would leave a thread writing into `.staging/` after the runtime is gone. The
    /// token is what that thread checks between entries (hazard H-8); the abort is what stops the
    /// async half from waiting on it.
    pub async fn shutdown(&mut self, grace: Duration) {
        for (step, chat) in self.live.drain() {
            let _ = chat.commands.send(ChatCommand::Cancel { reply: None });
            let Some(task) = chat.task else { continue };
            if tokio::time::timeout(grace * 2, task).await.is_err() {
                tracing::warn!(%step, "a chat did not end within the grace window");
            }
        }
        if let Some(live) = self.install.take() {
            // Cancelled, then given the same bounded window a chat gets, and only then aborted.
            // The task's own cleanup is what removes a partial `.staging/` entry (blueprint H-6),
            // and `abort()` cannot run it — nor can it stop the `spawn_blocking` unpack at all
            // (P-3), so an immediate abort leaves residue for the next install's hourly sweep.
            live.cancel.cancel();
            if tokio::time::timeout(grace * 2, live.task).await.is_err() {
                tracing::warn!(
                    agent = %live.agent_id,
                    "an install did not end within the grace window; its staging entry waits for the next sweep"
                );
            }
        }
        if let Some(live) = self.auth.take() {
            // The same arm, for the same reason and one more: the flow's child is an adapter with
            // an open loopback listener waiting for an OAuth redirect, and only the task's own
            // exit runs `ChildGuard::kill_and_reap` (MOD-21 D12). The token is what gets it there.
            live.cancel.cancel();
            if tokio::time::timeout(grace * 2, live.task).await.is_err() {
                tracing::warn!(
                    agent = %live.agent_id,
                    "a login did not end within the grace window"
                );
            }
        }
        for task in std::mem::take(&mut self.background) {
            task.abort();
        }
    }

    /// The [`StoreRequest::ProbeAgents`] path (MOD-2 D52, D53).
    ///
    /// In this order, and **before anything is spawned**: the writer, because probing costs
    /// process spawns and a probe with nowhere to write its result would pay them to throw the
    /// answer away; then the box, because `agent_box` has no primary key without one; then the
    /// registry and the working directory the probe resolves relative to. Only then does the task
    /// start, and from that point on it answers the request itself.
    async fn probe(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
    ) -> Result<Served, StoreError> {
        // MOD-21 D19, the third holder of one claim: a login ends by re-probing its row, and a
        // probe writes one for every row — including that one. The sentence names the row so the
        // status line says what to wait for.
        if let Some(live) = &self.auth {
            return Err(StoreError::Backend(format!(
                "a login is running for agent {}; probe once it has finished",
                live.agent_id
            )));
        }
        // The other half of `claim_is_free`'s rule: an install's re-probe writes `agent_box` for
        // its row, and a probe writes one for every row. Whichever started first, running them
        // together races on the same row, so each refuses while the other holds the box.
        if let Some(live) = &self.install {
            return Err(StoreError::Backend(format!(
                "an install is running for agent {}; probe once it has finished",
                live.agent_id
            )));
        }
        let writer = backend.writer().ok_or_else(|| {
            StoreError::Unreachable("this backend hands out no writer".to_owned())
        })?;
        // `Writer::Buffered` refuses `upsert_agent_box` with this same sentence (plan D52). It is
        // checked here rather than discovered on the write, because by then the spawns have
        // happened.
        if matches!(writer, Writer::Buffered(_)) {
            return Err(StoreError::Unreachable(
                htui_store::REGISTRY_ON_SERVER_ONLY.to_owned(),
            ));
        }
        let box_id = backend
            .box_info()
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "box",
                id: "this box is not registered".to_owned(),
            })?
            .box_id;
        let agents = backend.agents().await?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        self.background.push(tokio::spawn(run_probe(ProbeArgs {
            writer,
            box_id,
            agents,
            cwd,
            frames: Frames {
                tx: replies.clone(),
                addr,
            },
        })));
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::InstallPlan`] path (MOD-20 D13, D18).
    ///
    /// Every refusal is here, **before anything is spawned**, in the probe's own order and for the
    /// probe's own reasons: no installer, because a runtime that was never given one must not
    /// reach the network by accident; no writer, or one that refuses `agent_box`, because a plan
    /// is the last free moment to notice that the install it leads to could never be recorded — a
    /// registry read spent on it would be spent for nothing; no box row, because `agent_box` has
    /// no primary key without one; an install already running, because two of them for one id
    /// race on `.staging/` (hazard H-10); no such row; and a row that declares no source, which is
    /// a fact about a document the loop is already holding.
    ///
    /// What is **awaited** here is the whole of `R-NF-3` for this request: `box_info()` and
    /// `agents()`, the same two the probe awaits on this arm. The registry read, the `HEAD` and
    /// even the construction of the HTTP client happen in the task (blueprint P-13, hazard H-9).
    async fn install_plan(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        agent_id: AgentId,
    ) -> Result<Served, StoreError> {
        let config = self.install_config()?;
        // Taken and dropped: planning writes nothing, but the install it exists to authorise
        // does, and refusing costs nothing only while nothing has been fetched.
        drop(recording_writer(backend)?);
        registered_box(backend).await?;
        self.claim_is_free()?;
        let agent = row_for(backend, agent_id).await?.agent;
        declares_a_source(&agent)?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_plan(
            PlanArgs {
                config,
                agent,
                cwd,
                frames: Frames {
                    tx: replies.clone(),
                    addr,
                },
            },
            cancel.clone(),
        ));
        self.install = Some(LiveInstall {
            agent_id,
            phase: LivePhase::Planning,
            cancel,
            task,
        });
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::InstallConfirm`] path (MOD-20 D12, D18).
    ///
    /// [`install_plan`](Self::install_plan)'s refusals minus the last: the plan in hand is the
    /// evidence that a source was declared, and the pipeline re-checks every path in it at run
    /// time anyway (hazard H-16). The [`Writer`] is **taken** here rather than checked, because
    /// the task is what writes the row the probe produces and the loop replaces its own `Backend`
    /// wholesale whenever the server comes or goes.
    async fn install_confirm(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        plan: Box<InstallPlan>,
    ) -> Result<Served, StoreError> {
        let config = self.install_config()?;
        let writer = recording_writer(backend)?;
        let box_id = registered_box(backend).await?;
        self.claim_is_free()?;
        let summary = row_for(backend, plan.agent_id).await?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        let agent_id = summary.agent.id;
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_install(InstallArgs {
            config,
            plan,
            writer,
            box_id,
            agent: summary.agent,
            existing: summary.on_box,
            cwd,
            cancel: cancel.clone(),
            frames: Frames {
                tx: replies.clone(),
                addr,
            },
        }));
        self.install = Some(LiveInstall {
            agent_id,
            phase: LivePhase::Installing,
            cancel,
            task,
        });
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::InstallCancel`] path (MOD-20 D18).
    ///
    /// Answered at once, and the claim is **not** released here: the task is still running, still
    /// owns the staging entry it has to sweep, and still owes its stream a last frame. It ends
    /// itself, and the sweep at the top of [`serve`](Self::serve) is what forgets it.
    fn install_cancel(&mut self) -> Served {
        let Some(live) = self.install.as_ref() else {
            return Served::Reply(StoreReply::Failed {
                request: "install_cancel",
                message: "no install is running".to_owned(),
            });
        };
        live.cancel.cancel();
        Served::Reply(StoreReply::Install(InstallFrame::Cancelling))
    }

    /// The [`StoreRequest::AuthStart`] path (MOD-21 D18, D19).
    ///
    /// Every refusal is here, **before anything is spawned**, in the probe's and the install's own
    /// order and for their own reasons. No writer, or one that refuses `agent_box`: the flow ends
    /// by re-probing the row, and a login whose verdict could never be recorded would cost a
    /// process spawn and a human's minutes to throw the answer away. No box row: `agent_box` has
    /// no primary key without one. The claim held: a login, an install and a probe all write that
    /// row (D19). The row's own re-probe claim held: a chat's staleness probe is already asking
    /// the same question of the same box. No such row. Then two facts read off documents the loop
    /// is already holding — the transport has no `authenticate` call at all (D10), or the stored
    /// snapshot says the agent advertises no method to choose from.
    ///
    /// The [`Writer`] is **taken** rather than checked, as [`install_confirm`](Self::install_confirm)
    /// takes it: the task is what writes the row the re-probe produces, and the loop replaces its
    /// own `Backend` wholesale whenever the server comes or goes.
    ///
    /// What is **awaited** here is the whole of `R-NF-3` for this request: `box_info()` and
    /// `agents()`, the same two the probe and the pre-flight await on their arms. The spawn, the
    /// handshake, the human and the re-probe all happen in the task.
    async fn auth_start(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        agent_id: AgentId,
    ) -> Result<Served, StoreError> {
        let writer = recording_writer(backend)?;
        let box_id = registered_box(backend).await?;
        self.claim_is_free()?;
        let claim = self
            .reprobe_claims
            .claim((agent_id, box_id))
            .ok_or_else(|| {
                StoreError::Backend(format!(
                    "a re-probe is running for agent {agent_id}; try again in a moment"
                ))
            })?;
        let summary = row_for(backend, agent_id).await?;
        if !caps_for(&summary.agent).authenticate {
            return Err(StoreError::Backend(
                DriverError::Unsupported("authenticate").to_string(),
            ));
        }
        // The snapshot decides whether a login is *offered*; the chooser is fed by the live
        // `initialize` answer the flow itself gets (MOD-21 D7). A row nobody has probed and a row
        // whose agent demands nothing are the same refusal: there is no method to choose.
        let advertises = summary
            .on_box
            .as_ref()
            .and_then(ProbeSnapshot::from_row)
            .and_then(|snapshot| snapshot.handshake)
            .is_some_and(|handshake| !handshake.auth_methods.is_empty());
        if !advertises {
            return Err(StoreError::Backend(format!(
                "`{}` advertises no authentication methods",
                summary.agent.name
            )));
        }
        let driver = self
            .factory
            .driver_for(&summary.agent, summary.on_box.as_ref())
            .map_err(|err| StoreError::Backend(err.to_string()))?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        let cancel = CancellationToken::new();
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_auth(AuthArgs {
            driver,
            agent: summary.agent,
            existing: summary.on_box,
            box_id,
            writer,
            cwd,
            cancel: cancel.clone(),
            commands: commands_rx,
            opener: self.opener.clone(),
            frames: Frames {
                tx: replies.clone(),
                addr,
            },
        }));
        self.auth = Some(LiveAuth {
            agent_id,
            cancel,
            commands: commands_tx,
            task,
            _claim: claim,
        });
        Ok(Served::Deferred)
    }

    /// Forwards a command to the live login (MOD-21 D18).
    ///
    /// [`command`](Self::command)'s shape, and its two refusals: nothing running, and a channel
    /// the task has closed on its way out or already dropped. Both are refusals *of the request*
    /// ([`StoreReply::Failed`]), never frames of a stream that has ended — the section keeps its
    /// pane on the second one for `auth_open` and clears it on the flow's own terminal frame
    /// (blueprint H-22).
    ///
    /// A closed channel is **not** on its own a finished task (review L-1: [`run_auth`] closes it
    /// before its re-probe), so the value is let go of only once the task is, exactly as the sweep
    /// at the top of [`serve`](Self::serve) does it. Dropping it any earlier would release the
    /// re-probe claim while the flow's own probe is still writing the row (blueprint H-16).
    fn auth_command(&mut self, request: &'static str, command: AuthCommand) -> Served {
        let Some(live) = self.auth.as_ref() else {
            return Served::Reply(StoreReply::Failed {
                request,
                message: "no login is running".to_owned(),
            });
        };
        if live.commands.send(command).is_err() {
            self.auth.take_if(|live| live.task.is_finished());
            return Served::Reply(StoreReply::Failed {
                request,
                message: "this login has ended".to_owned(),
            });
        }
        Served::Deferred
    }

    /// The [`StoreRequest::AuthCancel`] path (MOD-21 D3, D18).
    ///
    /// [`install_cancel`](Self::install_cancel)'s shape: answered at once, and the claim is **not**
    /// released here. The task still owns a child with an open loopback listener that it has to
    /// kill and reap, and still owes its stream a last frame; it ends itself, and the sweep at the
    /// top of [`serve`](Self::serve) is what forgets it.
    fn auth_cancel(&mut self) -> Served {
        let Some(live) = self.auth.as_ref() else {
            return Served::Reply(StoreReply::Failed {
                request: "auth_cancel",
                message: "no login is running".to_owned(),
            });
        };
        live.cancel.cancel();
        Served::Reply(StoreReply::Auth(AuthFrame::Cancelling))
    }

    /// Where this runtime would install from, or the refusal that says it cannot.
    fn install_config(&self) -> Result<InstallConfig, StoreError> {
        self.installer
            .clone()
            .ok_or_else(|| StoreError::Backend("this runtime has no installer".to_owned()))
    }

    /// `Ok` when no install, probe or login holds the claim (hazard H-10, MOD-21 D19).
    fn claim_is_free(&self) -> Result<(), StoreError> {
        // MOD-21 D19: one claim, three holders. A login writes `agent_box` for its row at the end
        // of the flow, exactly as an install does, so the two exclude each other in both
        // directions and a second login is refused by the same line.
        if let Some(live) = &self.auth {
            return Err(StoreError::Backend(format!(
                "a login is already running for agent {}",
                live.agent_id
            )));
        }
        if let Some(live) = &self.install {
            return Err(StoreError::Backend(format!(
                "an install is already running for agent {}",
                live.agent_id
            )));
        }
        // A probe writes `agent_box` for every row, and an install's re-probe writes one of them.
        // Run together, they race on the same row and the last write wins — so the claim covers a
        // probe in flight too. The Settings section's own `probing` flag is not enough: **any**
        // `StoreReply::Agents` clears it (`ui/tabs/settings/agents.rs`, module doc), and
        // `wants_requests` re-issues `Agents` on every activation, so `r` → switch tab → back → `i`
        // reaches here with `run_probe` still running.
        if !self.background.is_empty() {
            return Err(StoreError::Backend(
                "a probe is already running on this box; install once it has finished".to_owned(),
            ));
        }
        Ok(())
    }

    /// Forwards a command to a live chat.
    fn command(&mut self, step_id: StepId, request: &'static str, command: ChatCommand) -> Served {
        let Some(chat) = self.live.get(&step_id) else {
            return Served::Reply(StoreReply::Failed {
                request,
                message: format!("no live chat for step {step_id}"),
            });
        };
        if chat.commands.send(command).is_err() {
            self.live.remove(&step_id);
            return Served::Reply(StoreReply::Failed {
                request,
                message: "this chat has ended".to_owned(),
            });
        }
        Served::Deferred
    }

    /// The `ChatStart` path: identity, registry row, driver, the two rows, the session future.
    #[expect(
        clippy::too_many_arguments,
        reason = "the request's own fields plus the three the worker supplies; a struct would \
                  rename the arity"
    )]
    async fn start(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        project_id: ProjectId,
        agent_id: AgentId,
        model: Option<String>,
        prompt: String,
    ) -> Result<Served, StoreError> {
        // Every backend hands out a writer since milestone 4 — the offline one is
        // `Writer::Buffered`, which records to `<cache_dir>/pending/` and is uploaded on the next
        // connection (plan D34). The `ok_or_else` stays because the signature is still `Option`: a
        // later backend that genuinely cannot record must refuse a chat rather than run one into
        // memory nobody will ever read.
        let writer = backend.writer().ok_or_else(|| {
            StoreError::Unreachable("this backend hands out no writer".to_owned())
        })?;
        let writer_label = writer.label();
        let box_id = backend
            .box_info()
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "box",
                id: "this box is not registered".to_owned(),
            })?
            .box_id;
        let user = backend.this_user().await?;
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;

        let summary = backend
            .agents()
            .await?
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "agent",
                id: agent_id.to_string(),
            })?;
        if !summary.agent.enabled {
            return Err(StoreError::Constraint(format!(
                "agent `{}` is disabled",
                summary.agent.name
            )));
        }
        let driver = self
            .factory
            .driver_for(&summary.agent, summary.on_box.as_ref())
            .map_err(|err| StoreError::Backend(err.to_string()))?;
        // The registry's own rule: settings that do not parse are settings that are not set
        // (`htui_agent::registry`), because the column is `JSONB NOT NULL DEFAULT '{}'` and
        // hand-editable.
        let settings: AgentSettings =
            serde_json::from_value(summary.agent.settings.clone()).unwrap_or_default();
        let model = model.or_else(|| summary.agent.default_model.clone());

        // Plan D70: the caps are the **project's**, read once here — beside the three registry
        // reads above, on the worker's task and not the UI's (`R-NF-3`) — and carried into the
        // chat's own task on `ChatArgs`. `run_chat` holds a `Writer`, not the `Backend`, so this
        // is the last place that can ask.
        //
        // A cap that does not parse **refuses the chat**, which is the opposite of `AgentSettings`
        // above and deliberately so: settings that do not parse are settings that are not set, but
        // a cap an operator wrote and `htui` ignored is the risk table's "wrong by a factor of a
        // million" pointing the other way — a run that was supposed to be bounded and was not. An
        // absent *row* is [`project_caps_for`]'s question, and its answer differs offline.
        let project_caps = project_caps_for(
            &writer,
            project_id,
            backend.project_settings(project_id).await?,
        )?;
        if project_caps.batch_micros.is_some() {
            // Plan D71: read and reported, never compared. A batch spans runs MOD-4 does not yet
            // create, so enforcing it here would mean inventing the batch identity
            // (`docs/ANA-4.md`:1283 assigns it to MOD-12).
            tracing::info!(
                project = %project_id,
                key = PER_TOKEN_CAP_BATCH,
                "a batch cap is set; a chat does not enforce it (ANA-4 §9: MOD-12 does)"
            );
        }
        let quota_latch = quota_latch_for(&writer, &summary.agent, box_id, settings.quota.source);

        let chat = ChatRunSpec::mint(project_id, box_id, user, Some(agent_id), model.clone());
        writer.start_chat_run(&chat).await?;

        let spec = SessionSpec {
            agent_id,
            step_id: chat.step_id,
            // The chat's working directory is this process's own until MOD-13 and MOD-7 give a
            // project a repo path per box (`docs/ANA-2.md` §4.7); the header shows the project's
            // name, never a path it does not have.
            //
            // A failure here **refuses the chat**: the path guard admits a file only under an
            // absolute session directory, and `"."` as a fallback would be a session with no scope
            // at all rather than a session scoped to somewhere unexpected.
            cwd,
            extra_dirs: Vec::new(),
            // MOD-10 fills this from the secret provider; until then a session carries none, and
            // the scrubber below therefore masks the credential prefixes only.
            env: std::collections::BTreeMap::new(),
            model: model.clone(),
            tools: htui_agent::driver::ToolExposure::default(),
            mcp: Vec::new(),
            permission: settings.permission.clone(),
            retain_raw: std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1"),
            resume: None,
            // Plan D83/D90: the per-run cap the recorder enforces client-side, handed to the
            // transport as well so one that has a server-side budget knob bounds the same run by
            // the same number. `project_caps` is read once, above, and both readers take it from
            // there — two reads of the setting would be two chances to convert it differently.
            budget_micros: project_caps.run_micros,
        };

        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let caps = driver.caps();
        self.live.insert(
            chat.step_id,
            LiveChat {
                commands: commands_tx,
                caps,
                task: None,
            },
        );
        self.started.push(chat.step_id);

        // Plan D55, and everything about it is in what this is *not*: the chat is already
        // started, the future below is already decided, and the re-probe shares no channel with
        // either. It refreshes the row for the next chat; this one proceeds on what
        // `tools::resolve` gave it, because coupling a chat's start to a 60-second handshake
        // timeout would make a stale row a minute of waiting.
        //
        // Three conditions, each for its own reason. `acp`, because tier 2 *is* `initialize` and
        // a `cli` row has none (milestone 8's problem). Not `Writer::Buffered`, because it
        // refuses `upsert_agent_box` (plan D52) and a probe with nowhere to write its answer
        // would spawn an adapter to throw it away. And stale, or there is nothing to learn.
        //
        // The first two are properties of the *row and the writer* and hold for D60's trigger
        // too, so they are what builds the arguments; staleness is the third trigger's own
        // condition and is applied to the spawn alone.
        let reprobe = (summary.agent.transport == Transport::Acp
            && !matches!(writer, Writer::Buffered(_)))
        .then(|| ReprobeArgs {
            writer: writer.clone(),
            box_id,
            agent: summary.agent.clone(),
            existing: summary.on_box.clone(),
            cwd: spec.cwd.clone(),
            claims: self.reprobe_claims.clone(),
        });
        // A chat whose row is already being re-probed carries none: two re-probes for one
        // `(agent_id, box_id)` would race each other for the same row (blueprint H-9). The
        // arguments **move** into whichever trigger gets them — the staleness one consumes them
        // here, and D60's arm below is the only other place they can go, so neither path clones
        // what the other threw away.
        let stale = needs_reprobe(&summary.agent, summary.on_box.as_ref(), Utc::now());
        let reprobe = match (stale, reprobe) {
            (true, Some(args)) => {
                self.background.push(tokio::spawn(run_reprobe(args)));
                None
            }
            (_, held) => held,
        };

        let step_id = chat.step_id;
        let args = ChatArgs {
            driver,
            writer,
            writer_label,
            chat,
            spec,
            prompt,
            policy: settings.permission,
            caps,
            commands: commands_rx,
            frames: Frames {
                tx: replies.clone(),
                addr,
            },
            grace: self.grace,
            reprobe,
            project_caps,
            quota_latch,
        };
        Ok(Served::Start {
            step_id,
            task: Box::pin(run_chat(args)),
        })
    }
}

/// Everything one chat session needs.
pub struct ChatArgs {
    driver: Box<dyn AgentDriver>,
    writer: Writer,
    /// [`Writer::label`], taken before the writer moves: what the tab's header says about where
    /// this conversation is being kept (plan D42).
    writer_label: &'static str,
    chat: ChatRunSpec,
    spec: SessionSpec,
    prompt: String,
    policy: PermissionPolicy,
    caps: DriverCaps,
    commands: mpsc::UnboundedReceiver<ChatCommand>,
    frames: Frames,
    grace: Duration,
    /// Plan D60: `Some` when a spawn failure should refresh this box's row for this agent. `None`
    /// for a `cli` row (tier 2 is `initialize`, which a `cli` row has none of), for a buffered
    /// writer (`upsert_agent_box` is refused, plan D52), and for a chat whose staleness re-probe
    /// is already running — a second one would race it for the same row.
    reprobe: Option<ReprobeArgs>,
    /// Plan D70: `project.settings`'s two token caps, read at `ChatStart`.
    ///
    /// Not named `caps`: that field above is [`DriverCaps`], which is what the *transport* can do.
    /// These are what the **project** allows it to spend, and two fields called `caps` in one
    /// struct would be one bug away from each other.
    project_caps: ProjectCaps,
    /// Plan D66-D68: the `agent_box` row this chat latches its allowance into, or `None` with the
    /// reason already logged by [`quota_latch_for`].
    quota_latch: Option<QuotaLatch>,
}

impl core::fmt::Debug for ChatArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChatArgs")
            .field("driver", &self.driver.name())
            .field("step", &self.chat.step_id)
            .field("spec", &self.spec)
            .field("project_caps", &self.project_caps)
            // Whether this chat latches, not which row it names: `Recorder`'s own `Debug` makes
            // the same choice for the same reason.
            .field("quota_latch", &self.quota_latch.is_some())
            .finish()
    }
}

/// The caps a chat enforces, out of `project.settings` (plan D70) — and what an **absent** row
/// means, which is not the same question online and offline (review M-3).
///
/// Online a `None` is a chat whose project is not in the database the chat is about to write to:
/// the id came from the tab's own scope, read from that same database, so the row went away under
/// the chat and refusing is the honest answer.
///
/// Offline the row comes from the **mirror**, and a mirror is a snapshot. A project created on the
/// server since the last sync has no row here, and such a chat used to start perfectly well —
/// [`Writer::Buffered`]'s `start_chat_run` reads no project at all, so nothing but this read
/// refuses it. Turning that into a refusal would be a regression bought for nothing, so an
/// unmirrored project is read as `{}`: unbounded, with a log line naming the situation. A cap the
/// mirror *does* hold is enforced exactly as online, which is the property plan D70 chose this
/// column for.
///
/// A document that does not parse refuses either way — that is `start_chat`'s own comment, and
/// this function is where the two answers are told apart rather than folded into one `ok_or`.
fn project_caps_for(
    writer: &Writer,
    project_id: ProjectId,
    settings: Option<Value>,
) -> Result<ProjectCaps, StoreError> {
    let settings = match (settings, writer) {
        (Some(settings), _) => settings,
        (None, Writer::Buffered(_)) => {
            tracing::info!(
                project = %project_id,
                "unmirrored project: no cap is enforced offline (plan D70)"
            );
            json!({})
        }
        (None, _) => {
            return Err(StoreError::NotFound {
                entity: "project",
                id: project_id.to_string(),
            });
        }
    };
    ProjectCaps::from_settings(&settings).map_err(|err| StoreError::Constraint(err.to_string()))
}

/// The `agent_box` row a chat latches its allowance into, or `None` with the reason logged
/// (plan D66-D68).
///
/// The decision is made **here**, at chat start, rather than discovered on the first `usage` row:
/// a [`Writer::Buffered`] refuses every registry write with
/// [`REGISTRY_ON_SERVER_ONLY`](htui_store::REGISTRY_ON_SERVER_ONLY) (`recording_writer`, plan D52),
/// because the offline mirror has no `agent_box` table at all — deliberately. An offline chat
/// therefore leaves the last server-side value standing and buffers the `usage` rows that
/// re-derive it after upload, which is what `R-HIS-1` actually asks for; failing or retrying a
/// turn over an advisory allowance figure would trade the requirement for the courtesy.
///
/// `source` is `agent.settings.quota.source` and `billing` is `agent.billing`, both read off the
/// row. Nothing here looks at `agent.name` (`R-AGT-5`) — the name is logged, and a log line is not
/// a dispatch.
fn quota_latch_for(
    writer: &Writer,
    agent: &Agent,
    box_id: BoxId,
    source: QuotaSource,
) -> Option<QuotaLatch> {
    if matches!(writer, Writer::Buffered(_)) {
        tracing::info!(
            agent = %agent.name,
            reason = htui_store::REGISTRY_ON_SERVER_ONLY,
            "offline: quota is not latched, and the usage rows are (plan D68)"
        );
        return None;
    }
    Some(QuotaLatch {
        agent_id: agent.id,
        box_id,
        source,
        billing: agent.billing,
    })
}

/// Everything the probe task owns (MOD-2 D53).
///
/// A [`Writer`] and not a [`Backend`]: the loop owns the one `Backend` and replaces it wholesale
/// when the server comes or goes, so a task holding a copy would keep writing to a server the loop
/// has already declared gone. `Writer` is the owned handle built for exactly this.
struct ProbeArgs {
    writer: Writer,
    box_id: BoxId,
    agents: Vec<htui_core::model::AgentSummary>,
    cwd: std::path::PathBuf,
    frames: Frames,
}

impl core::fmt::Debug for ProbeArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProbeArgs")
            .field("writer", &self.writer.label())
            .field("box_id", &self.box_id)
            .field("agents", &self.agents.len())
            .finish()
    }
}

/// One probe of every enabled registry row on this box, start to finish.
///
/// **One agent at a time**: two adapters spawning at once on a laptop buys nothing and blurs whose
/// stderr belongs to whom. A disabled registry row is skipped and its `on_box` left as it was
/// read — the probe answers what the box can run, and a row the user has turned off is not a
/// question about the box.
///
/// The reply is assembled from what this task itself wrote rather than re-read from the store, so
/// it states exactly what this probe did. Exactly one goes back, at the request's own address: the
/// row writes that fail end the run with a `Failed` at that same address, which is what clears the
/// section's in-flight state.
async fn run_probe(args: ProbeArgs) {
    let ProbeArgs {
        writer,
        box_id,
        mut agents,
        cwd,
        frames,
    } = args;
    let env = ProbeEnv::host(cwd);
    let tier2 = SpawnTier2::default();

    for summary in &mut agents {
        if !summary.agent.enabled {
            continue;
        }
        let ctx = ProbeContext {
            env: env.clone(),
            now: Utc::now(),
        };
        match probe_agent(
            &summary.agent,
            box_id,
            summary.on_box.as_ref(),
            &ctx,
            &tier2,
        )
        .await
        {
            ProbeOutcome::Row(row) => {
                if let Err(err) = writer.upsert_agent_box(&row).await {
                    frames.reply(
                        &frames.addr,
                        StoreReply::Failed {
                            request: "probe_agents",
                            message: err.to_string(),
                        },
                    );
                    return;
                }
                summary.on_box = Some(row);
            }
            // Plan D51: a hand-written row the probe could not confirm is left exactly as it is,
            // `probed_at` included.
            ProbeOutcome::Kept { reason } => {
                tracing::info!(agent = %summary.agent.name, reason, "the probe left a row alone");
            }
        }
    }

    frames.reply(&frames.addr, StoreReply::Agents(agents));
}

/// Whether this box's row for an agent is worth re-probing (plan D55).
///
/// Four ways to be stale and one way to be fresh: no row at all, a row no probe ever stamped, a
/// stamp further back than [`PROBE_TTL`], or a registry row **edited since** the probe read it. A
/// stamp in the *future* — a clock that stepped backwards under an NTP correction or a suspended
/// laptop — is not stale: `to_std` refuses a negative age, and treating one as "very old" would
/// re-probe on every chat until the clock caught up.
///
/// The fourth is age's blind spot and D58 is what opened it. Since the driver spawns
/// `probe.resolved` whenever its rules pass, the recording is no longer a hint the chat may
/// improve on — it *is* the launch. A hand-edited `agent.launch` (different args, a new
/// `HTUI_TOOL_*`, another binary entirely) would then be ignored for up to a day while every chat
/// kept spawning what was recorded from the row as it used to be. The store stamps that edit in
/// `agent.updated_at`, and a probe never writes `agents`, so the comparison has no way to see its
/// own work: the two stamps meeting is the steady state, and only `agents` moving past
/// `agent_box` is an edit. Nothing is compared *inside* the two documents — a byte-for-byte
/// launch comparison would re-probe on a whitespace change and still miss a `PATH` that moved.
///
/// A re-probe is cheap and asynchronous (D55 spawns it beside the chat, never in front of it), so
/// the failure this errs towards is one extra tier-2 handshake, not a delayed conversation.
#[must_use]
pub fn needs_reprobe(agent: &Agent, on_box: Option<&AgentBox>, now: DateTime<Utc>) -> bool {
    let Some(probed_at) = on_box.and_then(|row| row.probed_at) else {
        return true;
    };
    if agent.updated_at > probed_at {
        return true;
    }
    now.signed_duration_since(probed_at)
        .to_std()
        .is_ok_and(|age| age > PROBE_TTL)
}

/// Which `(agent_id, box_id)` rows a re-probe is running for right now (blueprint H-9).
///
/// **One per [`AgentRuntime`], not one per chat**, which is the whole of the widening. The
/// per-chat guard — `ChatArgs.reprobe = None` when the staleness trigger already fired — stops one
/// chat asking twice, and two chats can still overlap: a second `ChatStart` arriving while the
/// first chat's re-probe is in flight sees a `probed_at` that has not moved yet, calls the row
/// fresh, spawns the same recording, fails the same way, and starts a second `probe_agent` for the
/// same row. Two tier-2 children and a last-write-wins on one `agent_box` row, bounded only by how
/// fast the user can click.
///
/// Benign — both writes carry the same verdict — but two adapters spawned to answer one question
/// is a cost with no reader, and the exclusion is cheaper than the second handshake. Held by value
/// so both triggers get the same set: the staleness one spawns into the runtime's `background`,
/// the D60 one runs inside a chat task that outlives the arm that built it.
#[derive(Clone, Default)]
struct ReprobeClaims(Arc<Mutex<HashSet<(AgentId, BoxId)>>>);

impl ReprobeClaims {
    /// Claims `key` for the caller, or `None` when a re-probe already holds it.
    fn claim(&self, key: (AgentId, BoxId)) -> Option<ReprobeClaim> {
        let claimed = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
        claimed.then(|| ReprobeClaim {
            claims: self.clone(),
            key,
        })
    }
}

/// A held [`ReprobeClaims`] entry, released by its `Drop`.
///
/// RAII rather than a release call at the end of `run_reprobe`, because the re-probe has an exit
/// that runs none of its own code: `AgentRuntime::shutdown` aborts the background tasks, and an
/// aborted task drops its future mid-await. A claim released only on the normal path would leak on
/// that one, and the leak would outlive the process's next chat rather than the process.
struct ReprobeClaim {
    claims: ReprobeClaims,
    key: (AgentId, BoxId),
}

impl Drop for ReprobeClaim {
    fn drop(&mut self) {
        self.claims
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.key);
    }
}

/// What a re-probe needs, whichever trigger asks for it.
///
/// Two triggers, one argument set: the staleness check at `AgentRuntime::start` (plan D55) and a
/// spawn failure inside [`run_chat`] (plan D60). They differ in *when* they fire and in nothing
/// else, so the struct is what keeps them from drifting into two slightly different re-probes.
///
/// A [`Writer`] and not a [`Backend`], for `ProbeArgs`'s reason: the loop replaces its `Backend`
/// wholesale when the server comes or goes, and this outlives the arm that built it.
/// The `Agent` and the `Option<AgentBox>` are cloned once per ACP chat start, whether or not a
/// re-probe ever runs: both triggers need them and neither knows at build time which will fire.
/// Two rows with a `JSONB` column each, on the arm that is already building a `SessionSpec` and a
/// driver — the loop is not what makes a chat start feel slow, and a lazier shape would mean
/// keeping the `Backend` alive to re-read the rows later, which is what `ProbeArgs`'s doc explains
/// this type exists to avoid.
#[derive(Clone)]
struct ReprobeArgs {
    writer: Writer,
    box_id: BoxId,
    agent: Agent,
    existing: Option<AgentBox>,
    cwd: std::path::PathBuf,
    /// Whose turn it is to probe this row (blueprint H-9): the runtime's set, not this chat's.
    claims: ReprobeClaims,
}

impl core::fmt::Debug for ReprobeArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReprobeArgs")
            .field("writer", &self.writer.label())
            .field("agent", &self.agent.name)
            .field("existing", &self.existing.is_some())
            .finish()
    }
}

/// The `ChatStart` re-probe: tier 2 over one row, answering nobody (plan D55, plan D60).
///
/// **Tier 2 only.** Resolution is unavoidable — tier 2 has nothing to spawn without it — but the
/// `--version` children are skipped ([`ProbeEnv::without_versions`]), because what a lazy
/// re-probe is checking is whether the launch still *runs*, and three extra processes per chat to
/// re-read version strings that only the Settings tab renders is a cost with no reader.
///
/// Nothing here can fail the chat: it holds no chat channel, sends no reply, and a write that
/// fails is a `warn!` and nothing else. A `failed` verdict writes `enabled = false` and the
/// running conversation is untouched — the row it wrote is for the *next* chat.
///
/// **Not a task of its own.** The staleness trigger spawns it into the runtime's `background`
/// because the chat it belongs to is still going; the D60 trigger awaits it inline, because by
/// then the chat is over and its task has nothing left to do (blueprint P-7).
///
/// **One at a time per row** ([`ReprobeClaims`]). Both triggers arrive here, so this is the one
/// place that can see the overlap two chats create, and refusing is right for either of them: the
/// re-probe already in flight is asking the same question of the same box and will write the same
/// answer.
async fn run_reprobe(args: ReprobeArgs) {
    let ReprobeArgs {
        writer,
        box_id,
        agent,
        existing,
        cwd,
        claims,
    } = args;
    // Held for the whole probe and dropped with this future, aborted or not.
    let Some(_claim) = claims.claim((agent.id, box_id)) else {
        tracing::debug!(
            agent = %agent.name,
            "a re-probe for this row is already running; leaving it to that one"
        );
        return;
    };
    let ctx = ProbeContext {
        env: ProbeEnv::host(cwd).without_versions(),
        now: Utc::now(),
    };
    match probe_agent(
        &agent,
        box_id,
        existing.as_ref(),
        &ctx,
        &SpawnTier2::default(),
    )
    .await
    {
        ProbeOutcome::Row(row) => {
            if let Err(err) = writer.upsert_agent_box(&row).await {
                tracing::warn!(
                    agent = %agent.name,
                    %err,
                    "a stale agent_box row could not be refreshed"
                );
            }
        }
        // Plan D51 again, from the other trigger: a hand-written row the probe could not confirm
        // is left exactly as it is. `debug!`, not `info!` — nobody asked for this probe.
        ProbeOutcome::Kept { reason } => {
            tracing::debug!(agent = %agent.name, reason, "the re-probe left a row alone");
        }
    }
}

/// The writer of a backend that can hold an `agent_box` row, or the refusal that names why not.
///
/// `Writer::Buffered` refuses `upsert_agent_box` with `REGISTRY_ON_SERVER_ONLY` (plan D52), and
/// the whole point of asking here is to hear that sentence before the work rather than after it.
fn recording_writer(backend: &Backend) -> Result<Writer, StoreError> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable("this backend hands out no writer".to_owned()))?;
    if matches!(writer, Writer::Buffered(_)) {
        return Err(StoreError::Unreachable(
            htui_store::REGISTRY_ON_SERVER_ONLY.to_owned(),
        ));
    }
    Ok(writer)
}

/// This box's id, or the refusal that says it has never been registered.
async fn registered_box(backend: &Backend) -> Result<BoxId, StoreError> {
    Ok(backend
        .box_info()
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "box",
            id: "this box is not registered".to_owned(),
        })?
        .box_id)
}

/// One registry row with this box's `agent_box` for it, or a `NotFound` naming the id.
async fn row_for(
    backend: &Backend,
    agent_id: AgentId,
) -> Result<htui_core::model::AgentSummary, StoreError> {
    backend
        .agents()
        .await?
        .into_iter()
        .find(|summary| summary.agent.id == agent_id)
        .ok_or_else(|| StoreError::NotFound {
            entity: "agent",
            id: agent_id.to_string(),
        })
}

/// `Ok` when the row says where its adapter comes from (MOD-20 D12).
///
/// Read off the document the loop already holds, so `Settings > i` on a `NodePackage`-served row
/// costs no request at all. A `launch` that does not parse declares nothing, source included —
/// the pre-flight reaches the same conclusion from the same document, and this is its sentence.
fn declares_a_source(agent: &Agent) -> Result<(), StoreError> {
    let declared = serde_json::from_value::<AgentLaunch>(agent.launch.clone())
        .ok()
        .and_then(|launch| launch.discovery)
        .and_then(|discovery| discovery.install);
    if declared.is_some() {
        return Ok(());
    }
    Err(StoreError::Backend(
        PlanError::NoSource {
            agent: agent.name.clone(),
        }
        .to_string(),
    ))
}

/// Everything the pre-flight task owns (MOD-20 D18).
///
/// The [`InstallConfig`] rather than an [`Installer`]: the clients are built inside
/// [`run_plan`], so a client that cannot be built is a frame at this request's address rather
/// than a panic on the worker loop (blueprint P-13).
struct PlanArgs {
    config: InstallConfig,
    agent: Agent,
    cwd: std::path::PathBuf,
    frames: Frames,
}

impl core::fmt::Debug for PlanArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlanArgs")
            .field("registry_base", &self.config.registry_base)
            .field("agent", &self.agent.name)
            .finish_non_exhaustive()
    }
}

/// Everything the install task owns (MOD-20 D18).
///
/// A [`Writer`] and not a [`Backend`], for [`ProbeArgs`]'s reason: the loop replaces its one
/// `Backend` wholesale when the server comes or goes, and this outlives the arm that built it.
struct InstallArgs {
    config: InstallConfig,
    plan: Box<InstallPlan>,
    writer: Writer,
    box_id: BoxId,
    agent: Agent,
    existing: Option<AgentBox>,
    cwd: std::path::PathBuf,
    cancel: CancellationToken,
    frames: Frames,
}

impl core::fmt::Debug for InstallArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InstallArgs")
            .field("writer", &self.writer.label())
            .field("agent", &self.agent.name)
            .field("registry_id", &self.plan.registry_id)
            .field("version", &self.plan.version)
            .finish_non_exhaustive()
    }
}

/// The pre-flight, in its own task: one registry read, one `HEAD`, no archive byte (MOD-20 D13).
///
/// A plain `async fn` over an owned argument struct, like [`run_chat`]: production spawns it and a
/// harness can poll it, and neither has to know which.
///
/// Three shapes of answer, because the section renders three things. A plan is a consent pane. A
/// network failure is the by-hand steps, which is the one failure the user can route around
/// (MOD-20 D20). Everything else is a sentence on the status line, addressed as an ordinary
/// request failure — a refusal is not an install stream that has begun.
async fn run_plan(args: PlanArgs, cancel: CancellationToken) {
    let PlanArgs {
        config,
        agent,
        cwd,
        frames,
    } = args;
    let addr = frames.addr.clone();

    // Here and not on the loop: `Client::build` can fail, and a failure inside a `select!` arm
    // would be a panic that takes the worker with it (blueprint P-13).
    let installer = match Installer::new(config.clone()) {
        Ok(installer) => installer,
        Err(err) => {
            frames.reply(
                &addr,
                StoreReply::Install(InstallFrame::Failed {
                    message: err.to_string(),
                    manual: None,
                }),
            );
            return;
        }
    };
    let env = config.apply_to(ProbeEnv::host(cwd));

    let planned = tokio::select! {
        // The pre-flight has no cancellation of its own — it is one `GET` and one `HEAD`, both
        // already bounded by the registry timeout — so `x` during it is served here, by dropping
        // the future.
        () = cancel.cancelled() => {
            frames.reply(&addr, StoreReply::Install(InstallFrame::Cancelled));
            return;
        }
        planned = plan_install(&installer, &agent, &env, Utc::now()) => planned,
    };

    let reply = match planned {
        Ok(plan) => StoreReply::Install(InstallFrame::Plan(Box::new(plan))),
        Err(PlanError::Network { message, manual }) => StoreReply::Install(InstallFrame::Failed {
            message,
            manual: Some(manual),
        }),
        Err(err) => StoreReply::Failed {
            request: "install_plan",
            message: err.to_string(),
        },
    };
    frames.reply(&addr, reply);
}

/// The confirmed install, in its own task: fetch, verify, unpack, promote, re-probe (MOD-20 D16).
///
/// Every frame goes to the `InstallConfirm`'s own address, which is what makes one request many
/// replies (plan D18) and what `App::is_fresh` passes until the section confirms another install.
///
/// **The row is written before the terminal frame.** The section issues a
/// [`StoreRequest::Agents`] the moment it sees [`InstallFrame::Done`], and a read that overtook
/// the write would show the box as it was before the install — the one ordering this task is
/// responsible for. A write that fails is a failure *of the install*: the tree is on disk and
/// nothing records it, which is exactly what the user needs to be told.
///
/// The status is never this task's. `R-AGT-6`: whatever the pipeline achieved, what the row says
/// is what `probe_agent` decided, and the outcome carries it through unread.
async fn run_install(args: InstallArgs) {
    let InstallArgs {
        config,
        plan,
        writer,
        box_id,
        agent,
        existing,
        cwd,
        cancel,
        frames,
    } = args;
    let addr = frames.addr.clone();

    let installer = match Installer::new(config.clone()) {
        Ok(installer) => installer,
        Err(err) => {
            frames.reply(
                &addr,
                StoreReply::Install(InstallFrame::Failed {
                    message: err.to_string(),
                    manual: None,
                }),
            );
            return;
        }
    };
    let ctx = ProbeContext {
        env: config.apply_to(ProbeEnv::host(cwd)),
        now: Utc::now(),
    };
    let tier2 = SpawnTier2::default();
    let job = InstallJob {
        plan: &plan,
        agent: &agent,
        box_id,
        existing: existing.as_ref(),
        ctx: &ctx,
        tier2: &tier2,
    };
    let mut progress = |frame: InstallProgress| {
        frames.reply(
            &addr,
            StoreReply::Install(InstallFrame::Progress {
                phase: frame.phase,
                done: frame.done,
                total: frame.total,
            }),
        );
    };

    let reply = match install(&installer, job, &mut progress, &cancel).await {
        Ok(outcome) => {
            // Plan D51 travels through untouched: a `Kept` is a hand-written row the probe could
            // not confirm, and writing anything for it would be the installer overruling the user.
            let row = match &outcome {
                InstallOutcome::Installed { row, .. } => Some(row.clone()),
                InstallOutcome::Failed {
                    probe: ProbeOutcome::Row(row),
                    ..
                } => Some(row.clone()),
                InstallOutcome::Failed { .. } => None,
            };
            match row {
                Some(row) => match writer.upsert_agent_box(&row).await {
                    Ok(()) => StoreReply::Install(InstallFrame::Done(Box::new(outcome))),
                    Err(err) => StoreReply::Install(InstallFrame::Failed {
                        message: err.to_string(),
                        manual: None,
                    }),
                },
                None => StoreReply::Install(InstallFrame::Done(Box::new(outcome))),
            }
        }
        Err(InstallError::Cancelled) => StoreReply::Install(InstallFrame::Cancelled),
        Err(InstallError::Network { message, manual }) => {
            StoreReply::Install(InstallFrame::Failed {
                message,
                manual: Some(manual),
            })
        }
        Err(err) => StoreReply::Install(InstallFrame::Failed {
            message: err.to_string(),
            manual: None,
        }),
    };
    frames.reply(&addr, reply);
}

/// Everything the login task owns (MOD-21 D18).
///
/// A [`Writer`] and not a [`Backend`], for [`ProbeArgs`]'s reason: the loop replaces its one
/// `Backend` wholesale when the server comes or goes, and this outlives the arm that built it by
/// however long a human takes in a browser — the longest-lived of the three.
struct AuthArgs {
    driver: Box<dyn AgentDriver>,
    agent: Agent,
    existing: Option<AgentBox>,
    box_id: BoxId,
    writer: Writer,
    cwd: std::path::PathBuf,
    cancel: CancellationToken,
    commands: mpsc::UnboundedReceiver<AuthCommand>,
    opener: OpenerCommand,
    frames: Frames,
}

impl core::fmt::Debug for AuthArgs {
    /// The writer's label and the row's name; never the launch environment (`R-SEC-2`).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthArgs")
            .field("writer", &self.writer.label())
            .field("agent", &self.agent.name)
            .finish_non_exhaustive()
    }
}

/// One login, in its own task: the method list, the user's choice, the stream, and the re-probe
/// that decides what any of it meant (MOD-21 D18, D6).
///
/// **Two addresses, one stream.** Frames before the choice answer the `AuthStart`; from the
/// choice onwards they answer the `AuthChoose`, because `App::is_fresh` keys on the request kind
/// and a stale start must not out-rank the answer the user just gave (blueprint P-3: a local
/// `addr`, since `Frames` owns its own).
///
/// **The probe is the sole authority** (`R-AGT-6`, D6). On [`AuthOutcome::Completed`] — and only
/// there — the row is re-probed through `probe_agent` and **written before** the terminal frame,
/// exactly as [`run_install`] documents: the section issues a [`StoreRequest::Agents`] the moment
/// it sees [`AuthFrame::Done`], and a read that overtook the write would show the box as it was
/// before the login. Tier 2 only ([`ProbeEnv::without_versions`]) because a login changes no
/// version string. Every other outcome writes **nothing at all**, `probed_at` included: a refusal,
/// a cancel and an abandoned flow each leave `agent_box` exactly as they found it.
///
/// **The command channel is the caller's last word.** A `None` from it cancels the flow (review
/// M-1): the only thing that closes it is the runtime letting go of this login, and a login nobody
/// can choose for, open a link for or cancel is one that has to end itself. In the other direction,
/// the channel is closed and drained the instant the loop breaks (review L-1), so a command that
/// arrives during the re-probe is refused rather than carried down with the task.
///
/// Nothing here touches `agent`. A login is a fact about a box (`R-AGT-9`).
async fn run_auth(args: AuthArgs) {
    let AuthArgs {
        driver,
        agent,
        existing,
        box_id,
        writer,
        cwd,
        cancel,
        mut commands,
        opener,
        frames,
    } = args;
    // The `AuthStart`'s address until the choice arrives, and the `AuthChoose`'s afterwards.
    let mut addr = frames.addr.clone();

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (choice_tx, choice_rx) = oneshot::channel();
    // Taken by the first choice; a second one is refused rather than dropped, so the pane hears
    // back exactly once for every request it spent.
    let mut choice_tx = Some(choice_tx);

    let running = driver.authenticate(AuthFlow {
        cwd: cwd.clone(),
        events: events_tx,
        choice: choice_rx,
        // Cloned, not moved: the arm below needs a way to end a flow no caller can reach any more.
        cancel: cancel.clone(),
        idle: AUTH_IDLE_CAP,
        browser: BrowserPolicy::Neutralised,
    });
    tokio::pin!(running);

    let mut listening = true;
    let mut serving = true;
    let outcome = loop {
        tokio::select! {
            // Biased towards the flow: the moment it has answered there is nothing left to
            // forward that the drain below will not pick up.
            biased;
            outcome = &mut running => break outcome,
            event = events_rx.recv(), if listening => match event {
                Some(event) => frames.reply(&addr, StoreReply::Auth(auth_frame(event))),
                None => listening = false,
            },
            command = commands.recv(), if serving => match command {
                Some(AuthCommand::Choose { choice, reply }) => match choice_tx.take() {
                    Some(sender) => {
                        addr = reply;
                        let _ = sender.send(choice);
                    }
                    None => frames.reply(
                        &reply,
                        StoreReply::Failed {
                            request: "auth_choose",
                            message: AUTH_ALREADY_CHOSEN.to_owned(),
                        },
                    ),
                },
                Some(AuthCommand::Open { url, reply }) => {
                    // The opener is spawned and never waited on, so this arm costs the flow one
                    // `fork`/`exec` and not a browser's lifetime (MOD-21 D17).
                    let answer = match open_url(&url, &opener).await {
                        Ok(()) => StoreReply::Auth(AuthFrame::Opened),
                        Err(err) => StoreReply::Failed {
                            request: "auth_open",
                            message: err.to_string(),
                        },
                    };
                    frames.reply(&reply, answer);
                }
                // The runtime let go of this login **without** cancelling it: a panic on the worker
                // loop, or any drop of `AgentRuntime` that never reached `shutdown`, closes this
                // channel and drops the token clone rather than tripping it. A closed channel means
                // no caller can ever reach this flow again — its `AuthCancel` has nowhere to go —
                // so cancelling is the only correct reading. Without it the child, and the OAuth
                // loopback listener it is holding open, would wait on a human nobody can answer for
                // until the idle cap; and since every stderr line restarts that clock
                // (`htui-agent`'s `auth/run.rs`), an adapter that prints anything periodically
                // would never reach it at all.
                None => {
                    serving = false;
                    cancel.cancel();
                }
            },
        }
    };

    // Closed **before** the drain, the re-probe and the row write, which are seconds a caller could
    // otherwise spend a request into: from here on `auth_command` fails to send and refuses the
    // request itself, which is the answer it already has words for.
    commands.close();
    // What was already in the queue when the loop broke is answered at its own address rather than
    // dropped with the task: a `Served::Deferred` the pane never hears back from is the shape
    // MOD-20's review rejected, and this flow has nothing left to do for any of them.
    while let Ok(command) = commands.try_recv() {
        let (request, reply) = match command {
            AuthCommand::Choose { reply, .. } => ("auth_choose", reply),
            AuthCommand::Open { reply, .. } => ("auth_open", reply),
        };
        frames.reply(
            &reply,
            StoreReply::Failed {
                request,
                message: "this login has ended".to_owned(),
            },
        );
    }

    // The events sender lived inside the future that has just returned, so this drains what it
    // wrote on its way out and then ends — the last stderr line an adapter prints is often the
    // one that says why.
    while let Some(event) = events_rx.recv().await {
        frames.reply(&addr, StoreReply::Auth(auth_frame(event)));
    }

    let frame = match outcome {
        Ok(AuthOutcome::Completed { call }) => {
            let ctx = ProbeContext {
                env: ProbeEnv::host(cwd).without_versions(),
                now: Utc::now(),
            };
            match probe_agent(
                &agent,
                box_id,
                existing.as_ref(),
                &ctx,
                &SpawnTier2::default(),
            )
            .await
            {
                ProbeOutcome::Row(row) => match writer.upsert_agent_box(&row).await {
                    Ok(()) => AuthFrame::Done {
                        call,
                        // Read back off the row this task just wrote, never decided here.
                        status: ProbeSnapshot::from_row(&row)
                            .map_or(ProbeStatus::Failed, |snapshot| snapshot.status),
                    },
                    Err(err) => AuthFrame::Failed {
                        message: err.to_string(),
                    },
                },
                // Plan D51: a hand-written row the probe could not confirm is left exactly as it
                // is, and what the box says about itself is still what it said before.
                ProbeOutcome::Kept { reason } => {
                    tracing::info!(agent = %agent.name, reason, "the login left a row alone");
                    AuthFrame::Done {
                        call,
                        status: existing
                            .as_ref()
                            .and_then(ProbeSnapshot::from_row)
                            .map_or(ProbeStatus::Unauthenticated, |snapshot| snapshot.status),
                    }
                }
            }
        }
        Ok(AuthOutcome::Refused { message, .. }) => AuthFrame::Refused { message },
        // Blueprint H-23: a `Declined` is this task's own exit — the sender dropped by a shutdown
        // mid-chooser — and reads to a user as exactly what a cancel does.
        Ok(AuthOutcome::Cancelled | AuthOutcome::Declined) => AuthFrame::Cancelled,
        Ok(AuthOutcome::Idle { after }) => AuthFrame::Idle { after },
        Err(err) => AuthFrame::Failed {
            message: err.to_string(),
        },
    };
    frames.reply(&addr, StoreReply::Auth(frame));
}

/// One flow event as the frame the section renders. A total map, and deliberately dull: the two
/// enums are the same vocabulary either side of the crate boundary.
fn auth_frame(event: AuthEvent) -> AuthFrame {
    match event {
        AuthEvent::Methods {
            methods,
            logout,
            hidden,
        } => AuthFrame::Methods {
            methods,
            logout,
            hidden,
        },
        AuthEvent::Line(line) => AuthFrame::Line(line),
        AuthEvent::Url(url) => AuthFrame::Url(url),
    }
}

/// The reply-channel side of one chat: one address, one sender, one place frames are shaped.
struct Frames {
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    addr: ReplyAddr,
}

impl Frames {
    /// One recorded, scrubbed envelope.
    fn event(&self, envelope: DriverEnvelope) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Event(Box::new(envelope))),
        );
    }

    /// One of the three rows `htui` authors itself, shaped as an `other` event for transport only.
    ///
    /// The recorder has already written the real row with its real kind; this is the copy the tab
    /// renders, and shaping it as `other` is what keeps [`ChatFrame`] one type instead of four.
    fn local(&self, update: &str, body: Value, at: DateTime<Utc>) {
        self.event(DriverEnvelope {
            event: DriverEvent::Other(htui_agent::event::OtherEvent {
                update: update.to_owned(),
                body,
            }),
            raw: None,
            at,
        });
    }

    /// The session is over.
    fn ended(&self, stop_reason: StopReason) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Ended { stop_reason }),
        );
    }

    /// The session died.
    fn failed(&self, message: String) {
        self.send(
            self.addr.clone(),
            StoreReply::Chat(ChatFrame::Failed { message }),
        );
    }

    /// Answers one request by its own address.
    fn reply(&self, addr: &ReplyAddr, reply: StoreReply) {
        self.send(addr.clone(), reply);
    }

    fn send(&self, addr: ReplyAddr, reply: StoreReply) {
        // A UI that has gone away is not an error: the rows still matter, the frames do not.
        let _ = self.tx.send(ReplyEnvelope {
            seq: addr.seq,
            origin: addr.origin,
            reply,
        });
    }
}

/// How one turn ended.
enum TurnEnd {
    /// The agent finished it.
    Done(StopReason),
    /// The user cancelled it, and the session is over.
    Cancelled,
    /// The per-run token cap was reached: the session is cancelled and the run **fails**
    /// (`docs/ANA-4.md` §7 `:1143-1150`, §11 criterion 8).
    ///
    /// Distinct from [`TurnEnd::Cancelled`] because the two close the run differently — a user's
    /// cancel is `RunStatus::Cancelled` and a breached cap is `Failed`, which is §7's own word for
    /// it ("marks the step failed"). The rows are already written by then: the `error` row the cap
    /// authored is what the chat tab shows (plan D73).
    CapExceeded,
}

/// One chat session, start to finish.
///
/// Owns the driver, the writer, the recorder and the session handle; answers the `ChatStart`
/// request once the handshake is through (a spawn plus `initialize` plus `session/new` takes
/// seconds, and the store worker must not be blocked behind them).
pub async fn run_chat(args: ChatArgs) {
    let ChatArgs {
        driver,
        writer,
        writer_label,
        chat,
        spec,
        prompt,
        policy,
        caps,
        mut commands,
        frames,
        grace,
        reprobe,
        project_caps,
        quota_latch,
    } = args;

    let scrubber = MinimalScrubber::new(spec.env.values().cloned());
    let step_id = chat.step_id;
    let start_addr = frames.addr.clone();

    let mut session = match driver.start(spec, prompt.clone()).await {
        Ok(session) => session,
        Err(err) => {
            let message = err.to_string();
            frames.reply(
                &start_addr,
                StoreReply::Failed {
                    request: "chat_start",
                    message: message.clone(),
                },
            );
            close_run(&writer, &chat, RunStatus::Failed).await;
            frames.failed(message);
            // Plan D60. A failure to *spawn* is a fact about this box's row, not about the
            // conversation: the adapter it names is gone, moved by a self-update, or no longer
            // executable, and the row still says otherwise. It is refreshed here, in what is left
            // of this task's life — `run_chat` is spawned in production and awaited inline by the
            // harness, so this is off the worker's `select!` arm either way (`R-NF-3`) and
            // deterministic in a test. The command receiver goes first, so a command that arrives
            // meanwhile is refused as "this chat has ended" rather than queued for nobody.
            //
            // `Spawn` alone (blueprint H-8): `Unresolved` names the tool in its own message and
            // `Transport` is about the wire, not the row. And no transport fallback of any kind —
            // a CLI agent is its own registry row, never a degraded mode of an ACP one.
            if let (DriverError::Spawn(_), Some(reprobe)) = (&err, reprobe) {
                drop(commands);
                run_reprobe(reprobe).await;
            }
            return;
        }
    };

    frames.reply(
        &start_addr,
        StoreReply::ChatAccepted {
            step_id,
            session_ref: session.session_ref().cloned(),
            caps,
            writer_label,
        },
    );

    let (ui_tx, mut ui_rx) = mpsc::channel(UI_FRAMES);
    let mut recorder = Recorder::new(
        &writer,
        &scrubber,
        step_id,
        std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1"),
        Some(ui_tx),
    );
    // Two opt-in builders rather than two more `new` parameters, because most recorders in this
    // tree have neither (plan D66-D68, D70). The grace the cap's cancel takes is this runtime's
    // own `CANCEL_GRACE`, riding on `RunCap` so `htui_agent::record::pump` keeps its signature.
    if let Some(latch) = quota_latch {
        recorder = recorder.with_quota_latch(latch);
    }
    if let Some(micros) = project_caps.run_micros {
        recorder = recorder.with_run_cap(RunCap { micros, grace });
    }

    let now = Utc::now();
    if let Err(err) = recorder
        .record_prompt(&prompt, prompt_sections(), now)
        .await
    {
        tracing::error!(%err, "the prompt row could not be written");
    }
    frames.local("prompt", json!({ "text": prompt }), now);

    // Every call this session has seen, so a permission request can be evaluated against the tool
    // call it gates (`htui_agent::permission`).
    let mut calls: HashMap<String, ToolCallEvent> = HashMap::new();
    let mut status = RunStatus::Done;
    let mut last_stop = StopReason::EndTurn;

    loop {
        match run_turn(
            session.as_mut(),
            &mut recorder,
            &mut ui_rx,
            &mut commands,
            &policy,
            &mut calls,
            &frames,
            grace,
        )
        .await
        {
            Ok(TurnEnd::Done(stop)) => last_stop = stop,
            Ok(TurnEnd::Cancelled) => {
                status = RunStatus::Cancelled;
                last_stop = StopReason::Cancelled;
                break;
            }
            // The cap: the session is already cancelled and its two closing rows are already
            // written, so all that is left is how the *run* closes. `Failed` is §7's own word
            // ("marks the step failed"), and no `frames.failed` goes with it — plan D73 rules that
            // the `error` row the cap authored is the visibility, and it reached the tab through
            // the recorder's channel a moment ago.
            Ok(TurnEnd::CapExceeded) => {
                status = RunStatus::Failed;
                last_stop = StopReason::Cancelled;
                break;
            }
            Err(err) => {
                tracing::warn!(%err, "the chat session ended with a transport error");
                status = RunStatus::Failed;
                // The stop reason follows the **log**, not the error (review L-1). A turn that
                // failed *after* the cap fired has `error{cap_exceeded}` and `done{cancelled}` as
                // its last two rows — the H-2 path writes them from `finish` even when the flush
                // that was supposed to fails — so `Ended{EndTurn}` here would leave the tab's
                // closing frame contradicting the row underneath it. Every other failure keeps the
                // last turn's own reason, which is what it always was.
                if recorder.cap_breach().is_some() {
                    last_stop = StopReason::Cancelled;
                }
                frames.failed(err.to_string());
                break;
            }
        }

        // Between turns the session idles on the user, not on the wire: a closed transport is
        // noticed by the next command rather than here (a `next_event` here would end every chat
        // the moment its first turn closed).
        match commands.recv().await {
            Some(ChatCommand::Send { text, reply }) => {
                if let Err(err) = session.send_follow_up(text.clone()).await {
                    frames.reply(
                        &reply,
                        StoreReply::Failed {
                            request: "chat_send",
                            message: err.to_string(),
                        },
                    );
                    status = RunStatus::Failed;
                    frames.failed(err.to_string());
                    break;
                }
                let at = Utc::now();
                if let Err(err) = recorder.record_follow_up(&text, at).await {
                    tracing::error!(%err, "the follow-up row could not be written");
                }
                // **Once**, at the request's own address: the frame passes `App::is_fresh` from
                // either address, so sending it to the stream as well would render the same
                // follow-up twice. Every request is answered exactly once, including the ones that
                // succeed (`ChatCommand`'s contract).
                frames.reply(
                    &reply,
                    StoreReply::Chat(ChatFrame::Event(Box::new(follow_up_frame(&text, at)))),
                );
            }
            Some(ChatCommand::Answer { reply, .. }) => {
                frames.reply(
                    &reply,
                    StoreReply::Failed {
                        request: "chat_answer",
                        message: "no permission request is waiting".to_owned(),
                    },
                );
            }
            // Ending a chat **between** turns is not a cancellation: nothing was cut, the user is
            // simply finished, and the run closes `done` with the last turn's own stop reason
            // (a turn cut mid-flight is the `TurnEnd::Cancelled` arm above, which closes
            // `cancelled`).
            Some(ChatCommand::Cancel { reply }) => {
                let _ = session.cancel(grace).await;
                drain(session.as_mut(), &mut recorder, &mut ui_rx, &frames).await;
                if let Some(reply) = reply {
                    frames.reply(
                        &reply,
                        StoreReply::Chat(ChatFrame::Ended {
                            stop_reason: last_stop,
                        }),
                    );
                }
                break;
            }
            // The runtime is gone: end the session rather than leave a child running. Same rule —
            // no turn was open, so the run is done rather than cancelled.
            None => {
                let _ = session.cancel(grace).await;
                drain(session.as_mut(), &mut recorder, &mut ui_rx, &frames).await;
                break;
            }
        }
    }

    if let Err(err) = recorder.finish().await {
        tracing::error!(%err, "the recorder did not close cleanly");
        status = RunStatus::Failed;
    }
    close_run(&writer, &chat, status).await;
    frames.ended(last_stop);
}

/// Drives one turn: pulls events, records them, and serves commands while a request is parked.
///
/// This is [`htui_agent::record::pump`]'s shape with one difference the plan's D28 did not have:
/// `pump` cannot cross a parked permission request — `next_event` refuses while one is
/// outstanding, on every transport — so the pull and the command channel have to be served by the
/// same loop.
#[expect(
    clippy::too_many_arguments,
    reason = "one turn's collaborators; a struct would rename the arity without reducing it"
)]
async fn run_turn(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    commands: &mut mpsc::UnboundedReceiver<ChatCommand>,
    policy: &PermissionPolicy,
    calls: &mut HashMap<String, ToolCallEvent>,
    frames: &Frames,
    grace: Duration,
) -> Result<TurnEnd, DriverError> {
    let mut parked: Option<PermissionRequestId> = None;

    loop {
        if parked.is_some() {
            // Nothing may be pulled until the agent has its answer.
            match commands.recv().await {
                Some(ChatCommand::Answer {
                    request_id,
                    answer,
                    reply,
                }) => {
                    // A second tap on the same digit, or an answer that raced the frame saying it
                    // was already answered, is a stale request — **not** a reason to end the
                    // conversation. It is refused at its own address and the turn goes on.
                    if parked.as_ref() != Some(&request_id) {
                        frames.reply(
                            &reply,
                            StoreReply::Failed {
                                request: "chat_answer",
                                message: format!("no permission request `{request_id}` is waiting"),
                            },
                        );
                        continue;
                    }
                    session
                        .answer_permission(request_id.clone(), answer.clone())
                        .await?;
                    // **One** frame, addressed to the request that caused it: it passes
                    // `App::is_fresh` either way, and a second copy at the stream's address would
                    // render the same answer twice.
                    record_answer(
                        recorder,
                        &request_id,
                        &answer,
                        AnsweredBy::User,
                        frames,
                        Some(&reply),
                    )
                    .await;
                    parked = None;
                }
                Some(ChatCommand::Cancel { reply }) => {
                    session.cancel(grace).await?;
                    if let Some(request_id) = parked.take() {
                        record_answer(
                            recorder,
                            &request_id,
                            &PermissionAnswer::Cancelled,
                            AnsweredBy::Policy,
                            frames,
                            None,
                        )
                        .await;
                    }
                    drain(session, recorder, ui, frames).await;
                    if let Some(reply) = reply {
                        frames.reply(
                            &reply,
                            StoreReply::Chat(ChatFrame::Ended {
                                stop_reason: StopReason::Cancelled,
                            }),
                        );
                    }
                    return Ok(TurnEnd::Cancelled);
                }
                Some(ChatCommand::Send { reply, .. }) => {
                    frames.reply(
                        &reply,
                        StoreReply::Failed {
                            request: "chat_send",
                            message: "answer the permission request first".to_owned(),
                        },
                    );
                }
                None => return Ok(TurnEnd::Cancelled),
            }
            continue;
        }

        let Some(envelope) = session.next_event().await? else {
            // The stream ended without a `done`. That is a transport that died mid-turn, not a
            // turn that finished: reporting it as `EndTurn` would close the run `done`, leave no
            // `done` row in the log, and tell the tab nothing.
            return Err(DriverError::Closed);
        };
        let event = envelope.event.clone();
        // Plan D69 as amended (blueprint P-1): the recorder **detects** the per-run cap and this
        // loop — the one that holds the session in production — performs the cancel, through the
        // same `enforce_breach` `htui_agent::record::pump` calls. `pump` cannot be reused here
        // (`next_event` refuses while a permission request is parked, which is this function's
        // whole reason for existing), so the sequence is shared and the loops are not.
        if let Some(breach) = record(recorder, envelope, ui, frames).await {
            enforce_cap_breach(session, recorder, breach).await?;
            // The two closing rows the sequence just wrote, as frames: `enforce_breach` records
            // through the recorder's channel like everything else, and nothing else drains it.
            forward(ui, frames);
            return Ok(TurnEnd::CapExceeded);
        }

        match event {
            DriverEvent::ToolCall(call) => {
                calls.insert(call.tool_call_id.clone(), call);
            }
            DriverEvent::PermissionRequest(request) => {
                let call = request.tool_call_id.as_ref().and_then(|id| calls.get(id));
                // Stages 1 and 2 of ANA-4 §4.3 decide here, where the recorder is; stage 3 parks
                // the request and the user decides.
                match htui_agent::permission::evaluate(policy, call, &request.options) {
                    Some(answered) => {
                        let answer = PermissionAnswer::Selected(answered.option_id.clone());
                        session
                            .answer_permission(request.request_id.clone(), answer.clone())
                            .await?;
                        tracing::info!(
                            stage = ?answered.stage,
                            reason = %answered.reason,
                            "a permission request was answered by policy"
                        );
                        record_answer(
                            recorder,
                            &request.request_id,
                            &answer,
                            AnsweredBy::Policy,
                            frames,
                            None,
                        )
                        .await;
                    }
                    None => parked = Some(request.request_id.clone()),
                }
            }
            DriverEvent::Done(done) => return Ok(TurnEnd::Done(done.stop_reason)),
            _ => {}
        }
    }
}

/// Records one envelope, forwards the scrubbed copy the recorder made of it, and hands back the
/// per-run cap verdict.
///
/// The frame comes from the recorder's own channel, never from the envelope this function was
/// handed: what reaches the screen must be masked exactly as what reached the store (`R-SEC-3`).
///
/// A recording failure is logged and **not** returned, as it always was: the turn goes on and
/// `finish` reports it, and there is no verdict to answer because the call that failed produced
/// none.
///
/// That last clause is the whole of it, and it used to read "a row the store never took spent
/// nothing" — which was wrong on one path (review M-1). A `usage` row is summed and compared
/// **before** the flush that persists it, so a store that refuses that flush leaves a session
/// whose spend is counted and whose cap is spent. The recorder therefore hands the verdict back on
/// that path rather than the error (`Recorder::record_unreadable`), and this arm's `None` means
/// only what it says: nothing to cancel over.
async fn record(
    recorder: &mut Recorder<'_, Writer>,
    envelope: DriverEnvelope,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    frames: &Frames,
) -> Option<CapBreach> {
    let breach = match recorder.record(envelope).await {
        Ok(breach) => breach,
        Err(err) => {
            tracing::error!(%err, "an event could not be recorded");
            None
        }
    };
    forward(ui, frames);
    breach
}

/// Drains the recorder's render channel into the reply stream.
///
/// Extracted from [`record`] because the cap's closing sequence writes two rows without going
/// through it, and their frames have to reach the tab the same way every other row's does.
fn forward(ui: &mut mpsc::Receiver<DriverEnvelope>, frames: &Frames) {
    while let Ok(frame) = ui.try_recv() {
        frames.event(frame);
    }
}

/// Pulls what is left of a cancelled session into the log.
async fn drain(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    frames: &Frames,
) {
    while let Ok(Some(envelope)) = session.next_event().await {
        let done = matches!(envelope.event, DriverEvent::Done(_));
        // The cap verdict is discarded here on purpose: this session is already being cancelled,
        // and a second cancel over a row pulled *by* the first would be a cancel of a cancel.
        record(recorder, envelope, ui, frames).await;
        if done {
            break;
        }
    }
}

/// Writes the `permission_answer` row and its frame.
async fn record_answer(
    recorder: &mut Recorder<'_, Writer>,
    request_id: &PermissionRequestId,
    answer: &PermissionAnswer,
    by: AnsweredBy,
    frames: &Frames,
    // Where the frame goes: the request that asked, when a user's answer caused it, else the
    // stream's own address (a policy answer nobody asked for).
    to: Option<&ReplyAddr>,
) {
    let (option_id, cancelled) = match answer {
        PermissionAnswer::Selected(option_id) => (Some(option_id.clone()), false),
        PermissionAnswer::Cancelled => (None, true),
    };
    let at = Utc::now();
    if let Err(err) = recorder
        .record_permission_answer(request_id, option_id.as_deref(), by, cancelled, at)
        .await
    {
        tracing::error!(%err, "the permission answer row could not be written");
    }
    let frame = DriverEnvelope {
        event: DriverEvent::Other(htui_agent::event::OtherEvent {
            update: "permission_answer".to_owned(),
            body: json!({
                "request_id": request_id.as_str(),
                "option_id": option_id,
                "by": by.as_str(),
                "cancelled": cancelled,
            }),
        }),
        raw: None,
        at,
    };
    match to {
        Some(addr) => frames.reply(addr, StoreReply::Chat(ChatFrame::Event(Box::new(frame)))),
        None => frames.event(frame),
    }
}

/// The frame a follow-up produces, shaped as the `other` row the tab renders.
fn follow_up_frame(text: &str, at: DateTime<Utc>) -> DriverEnvelope {
    DriverEnvelope {
        event: DriverEvent::Other(htui_agent::event::OtherEvent {
            update: "follow_up".to_owned(),
            body: json!({ "text": text }),
        }),
        raw: None,
        at,
    }
}

/// The `sections[]` of the `prompt` row.
///
/// A chat's prompt is what the user typed, so it is one section. ANA-5's assembler fills this with
/// the real section list in milestone 9, and the row's shape does not change when it does.
fn prompt_sections() -> Value {
    json!([{ "name": "chat", "tokens": Value::Null, "trimmed": false }])
}

/// Closes the chat's `run` / `run_step` pair, so it stops counting as an active run.
async fn close_run(writer: &Writer, chat: &ChatRunSpec, status: RunStatus) {
    if let Err(err) = writer
        .finish_chat_run(chat.run_id, chat.step_id, status, Utc::now())
        .await
    {
        tracing::error!(%err, "the chat run could not be closed");
    }
}

/// The `session_started` banner body, for a caller that wants the agent-side id out of a frame.
#[must_use]
pub fn session_ref_of(envelope: &DriverEnvelope) -> Option<&str> {
    match &envelope.event {
        DriverEvent::Other(other) if other.update == SESSION_STARTED => {
            other.body.get("session_id").and_then(Value::as_str)
        }
        _ => None,
    }
}

/// Renders a store error into the reply the asking view receives.
fn failed(request: &'static str, err: &StoreError) -> StoreReply {
    StoreReply::Failed {
        request,
        message: err.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use htui_agent::INSTALL_ROOT_VAR;
    use htui_agent::conformance::{Script, ScriptEvent};
    use htui_agent::event::{
        DoneEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent, TextChunk,
        ToolKind,
    };
    use htui_agent::fake::FakeAdapter;
    use htui_core::fixtures::ids;
    use htui_core::model::{Agent, AgentId, EventKind, EventRole, Scope, Transport};
    use htui_core::store::MemStore;
    use htui_core::store::ReadStore as _;
    use std::sync::Arc;

    // -----------------------------------------------------------------------------------------
    // MOD-20: the install fixtures
    //
    // The fixture server, the archive builder and the row below are this file's own, per the
    // repo's per-file test-helper rule. Every one of them is deliberately anonymous: `R-AGT-5`
    // says the installer knows no agent's name, and a worker test that spelled one would be
    // asserting the seeds rather than the plumbing.
    // -----------------------------------------------------------------------------------------

    /// The registry entry id these cases install. Not an agent, not a vendor: a made-up id, which
    /// is the whole point — the worker never learns what it is installing.
    pub(crate) const INSTALL_ID: &str = "demo-acp";

    /// The version those cases install.
    pub(crate) const INSTALL_VERSION: &str = "1.0.0";

    /// The `discovery.tools` key whose glob must resolve what the install writes, and the file
    /// inside the archive that it resolves to.
    pub(crate) const INSTALL_TOOL: &str = "demo_server";

    /// One scripted answer: what the fixture responder sends for one path.
    #[derive(Debug, Clone, Default)]
    struct Route {
        status: u16,
        body: Vec<u8>,
        /// How long to sit on the request before answering at all — the stall a loop-freedom or a
        /// shutdown case needs, so the install is provably still running when it is asserted on.
        delay: Duration,
        /// When non-zero, the body is sent in two halves with this pause between them.
        ///
        /// It is what puts a cancellation inside the download's `select!` rather than at the check
        /// that guards the next step: without a gap between chunks a body this small arrives whole
        /// before any token could be tripped.
        chunk_delay: Duration,
    }

    /// A loopback HTTP/1.1 responder with a scripted route table and a request recorder.
    ///
    /// This file's own, deliberately: the repo keeps test helpers per file, and the one in
    /// `htui-agent`'s `tests/install.rs` answers a different set of questions (conditional
    /// requests, `HEAD` sizes, digests) that no worker case asks.
    #[derive(Debug, Clone)]
    struct Fixture {
        addr: std::net::SocketAddr,
        log: Arc<Mutex<Vec<String>>>,
        routes: Arc<Mutex<HashMap<String, Route>>>,
    }

    impl Fixture {
        /// Binds an ephemeral port and starts accepting.
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("an ephemeral loopback port");
            let addr = listener.local_addr().expect("the bound address");
            let fixture = Self {
                addr,
                log: Arc::new(Mutex::new(Vec::new())),
                routes: Arc::new(Mutex::new(HashMap::new())),
            };
            let serving = fixture.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let serving = serving.clone();
                    tokio::spawn(async move { serving.answer(stream).await });
                }
            });
            fixture
        }

        /// `http://127.0.0.1:<port>`, the value `InstallConfig::registry_base` takes.
        fn base(&self) -> String {
            format!("http://{}", self.addr)
        }

        /// The absolute URL of one path on this server.
        fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.addr)
        }

        /// Scripts one path.
        fn route(&self, path: &str, route: Route) {
            self.routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path.to_owned(), route);
        }

        /// Every `"<METHOD> <PATH>"` the responder has seen, in order.
        fn lines(&self) -> Vec<String> {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        /// Reads one request, records it, and writes the scripted answer.
        async fn answer(self, mut stream: tokio::net::TcpStream) {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                match stream.read(&mut byte).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => head.push(byte[0]),
                }
                if head.ends_with(b"\r\n\r\n") || head.len() > 16 * 1024 {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let mut parts = head.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let target = parts.next().unwrap_or_default();
            let path = target.split(['?', '#']).next().unwrap_or(target).to_owned();
            // The guard is dropped before the first `.await` below, on purpose: this file lives by
            // the rule the worker does — no lock is ever held across a suspension point.
            let route = {
                let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
                log.push(format!("{method} {path}"));
                drop(log);
                self.routes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&path)
                    .cloned()
            };
            let Some(route) = route else {
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                    .await;
                return;
            };
            if !route.delay.is_zero() {
                tokio::time::sleep(route.delay).await;
            }
            let head = format!(
                "HTTP/1.1 {} OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                route.status,
                route.body.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            if method == "HEAD" {
                let _ = stream.flush().await;
                return;
            }
            if route.chunk_delay.is_zero() {
                let _ = stream.write_all(&route.body).await;
            } else {
                let (first, second) = route.body.split_at(route.body.len() / 2);
                let _ = stream.write_all(first).await;
                let _ = stream.flush().await;
                tokio::time::sleep(route.chunk_delay).await;
                let _ = stream.write_all(second).await;
            }
            let _ = stream.flush().await;
        }
    }

    /// A plan for [`INSTALL_ID`] under `root`, fetched from `archive_url`.
    ///
    /// Hand-built rather than produced by `plan()`: T7 is about what the worker does with a plan,
    /// and a pre-flight in front of every case would put a registry read on the path of tests
    /// whose subject is the *confirm*. It is exactly what `InstallConfirm` carries — a value the
    /// section round-trips — so building one is not a shortcut around anything.
    pub(crate) fn demo_plan(
        agent_id: AgentId,
        root: std::path::PathBuf,
        archive_url: String,
    ) -> InstallPlan {
        InstallPlan {
            agent_id,
            agent_name: "demo".to_owned(),
            tool: INSTALL_TOOL.to_owned(),
            registry_id: INSTALL_ID.to_owned(),
            registry_name: "Demo".to_owned(),
            version: INSTALL_VERSION.to_owned(),
            platform: htui_agent::probe::platform_key(),
            archive_url,
            format: htui_agent::ArchiveFormat::Zip,
            content_length: None,
            sha256: None,
            cmd: format!("./{INSTALL_TOOL}"),
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
            license: None,
            license_url: None,
            install_dir: root.join(INSTALL_ID).join(INSTALL_VERSION),
            root,
            existing_versions: Vec::new(),
            available_bytes: None,
            need_bytes: None,
            args_differ: false,
            consent: None,
            recorded: None,
            registry_cached_age_secs: None,
            planned_at: Utc::now(),
        }
    }

    /// A registry row whose glob looks under the install root, declaring an install or not.
    ///
    /// `acp`, because only an `acp` row has a tier-2 handshake for the re-probe to run, and the
    /// re-probe's verdict is what decides the outcome the confirm case reads. The pattern is the
    /// seeds' own shape — `%HTUI_AGENTS_ROOT%/<id>/*/<leaf>` — so the post-promote agreement the
    /// pipeline checks is a real one and not a tautology.
    pub(crate) fn install_row(id: AgentId, name: &str, declared: bool) -> Agent {
        let pattern = format!("%{INSTALL_ROOT_VAR}%/{INSTALL_ID}/*/{INSTALL_TOOL}");
        let mut discovery = json!({
            "tools": { "demo_server": { "kind": "glob", "patterns": [pattern] } },
            "handshake": true,
        });
        if declared {
            discovery["install"] =
                json!({ "source": "acp_registry", "id": INSTALL_ID, "tool": INSTALL_TOOL });
        }
        Agent {
            name: name.to_owned(),
            transport: Transport::Acp,
            launch: json!({
                "command": "${demo_server}",
                "args": [],
                "env": {},
                "discovery": discovery,
            }),
            settings: json!({}),
            ..fake_row(id)
        }
    }

    /// A one-entry `.zip` holding `body` at [`INSTALL_TOOL`], **stored** rather than deflated.
    ///
    /// Written by hand so this crate needs no archive dependency for one fixture: `stored` is
    /// method 0, which every zip reader supports, and the only arithmetic is the CRC the reader
    /// checks. The entry declares no unix mode, which is deliberate — it is the archive shape
    /// hazard H-5 is about, and the promoted file is executable only because the installer made it
    /// so.
    fn stored_zip(body: &[u8]) -> Vec<u8> {
        /// The CRC-32 (IEEE) of `bytes`, which is the one number a zip reader recomputes.
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFF_u32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    let carry = crc & 1;
                    crc >>= 1;
                    if carry == 1 {
                        crc ^= 0xEDB8_8320;
                    }
                }
            }
            !crc
        }

        let name = INSTALL_TOOL.as_bytes();
        let crc = crc32(body);
        let size = u32::try_from(body.len()).expect("the fixture archive is tiny");
        let name_len = u16::try_from(name.len()).expect("the entry name is short");
        let mut zip = Vec::new();

        // The local file header, then the bytes themselves.
        zip.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        zip.extend_from_slice(&0_u16.to_le_bytes()); // flags
        zip.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
        zip.extend_from_slice(&0_u16.to_le_bytes()); // modification time
        zip.extend_from_slice(&0x21_u16.to_le_bytes()); // modification date: 1980-01-01
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes()); // compressed
        zip.extend_from_slice(&size.to_le_bytes()); // uncompressed
        zip.extend_from_slice(&name_len.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(name);
        zip.extend_from_slice(body);

        // The central directory, which is what a reader opens the archive by.
        let directory_at = u32::try_from(zip.len()).expect("the fixture archive is tiny");
        zip.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&20_u16.to_le_bytes()); // version made by
        zip.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        zip.extend_from_slice(&0_u16.to_le_bytes()); // flags
        zip.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
        zip.extend_from_slice(&0_u16.to_le_bytes()); // modification time
        zip.extend_from_slice(&0x21_u16.to_le_bytes()); // modification date
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&name_len.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes()); // extra field length
        zip.extend_from_slice(&0_u16.to_le_bytes()); // comment length
        zip.extend_from_slice(&0_u16.to_le_bytes()); // disk number
        zip.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
        zip.extend_from_slice(&0_u32.to_le_bytes()); // external attributes: no unix mode
        zip.extend_from_slice(&0_u32.to_le_bytes()); // offset of the local header
        zip.extend_from_slice(name);

        // The end-of-central-directory record.
        let directory_len =
            u32::try_from(zip.len()).expect("the fixture archive is tiny") - directory_at;
        zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes()); // this disk
        zip.extend_from_slice(&0_u16.to_le_bytes()); // the disk the directory starts on
        zip.extend_from_slice(&1_u16.to_le_bytes()); // entries on this disk
        zip.extend_from_slice(&1_u16.to_le_bytes()); // entries in total
        zip.extend_from_slice(&directory_len.to_le_bytes());
        zip.extend_from_slice(&directory_at.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes()); // comment length
        zip
    }

    /// A registry row for the fake transport: `cli`, stream `fake`, so the factory reaches the
    /// adapter by **row data** and not by name (`R-AGT-5`, plan D12).
    fn fake_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Cli,
            billing: htui_core::model::Billing::PerToken,
            models: Vec::new(),
            default_model: None,
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({ "cli": { "stream": "fake", "permission_mode": "ask",
                                       "extra_args": [] } }),
            enabled: true,
            created_at: htui_core::fixtures::demo_at(0, 0),
            updated_at: htui_core::fixtures::demo_at(0, 0),
        }
    }

    /// A store holding the demo fixture plus the fake row, and the runtime that can drive it.
    async fn fixture(script: Script) -> (MemStore, Backend, AgentRuntime, AgentId) {
        fixture_with_project_settings(script, None).await
    }

    /// [`fixture`], with `PROJECT_HTUI`'s `settings` column replaced.
    ///
    /// The caps a chat enforces are that column's (MOD-2 plan D70), so the cases below need a demo
    /// fixture whose project carries one. `DemoData` is public and `MemStore::from_demo` takes it,
    /// which is why this needs no store method and no migration: the document is edited before the
    /// store is built, exactly as an operator would edit the row.
    async fn fixture_with_project_settings(
        script: Script,
        settings: Option<Value>,
    ) -> (MemStore, Backend, AgentRuntime, AgentId) {
        let (store, backend, runtime, agent_id, _spec) =
            fixture_with_spec_spy(script, settings).await;
        (store, backend, runtime, agent_id)
    }

    /// [`fixture_with_project_settings`], plus the slot the driver records its [`SessionSpec`] in.
    ///
    /// What a chat puts *on* the spec is production wiring no transport-side assertion can see —
    /// the fake reads the fields it needs and drops the rest — so the one case that asks whether a
    /// figure reached the transport at all asks the spy instead.
    async fn fixture_with_spec_spy(
        script: Script,
        settings: Option<Value>,
    ) -> (MemStore, Backend, AgentRuntime, AgentId, SpecSlot) {
        let mut data = htui_core::fixtures::demo_data();
        if let Some(settings) = settings {
            for project in &mut data.projects {
                if project.id == ids::PROJECT_HTUI {
                    project.settings = settings.clone();
                }
            }
        }
        let store = MemStore::from_demo(data);
        let agent_id = AgentId::new();
        store
            .upsert_agent(&fake_row(agent_id))
            .await
            .expect("the fake row lands");

        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let spec: SpecSlot = Arc::new(Mutex::new(None));
        let mut factory = DriverFactory::new();
        factory.register(
            "cli/fake",
            Box::new(FakeBuilder(Arc::clone(&adapter), Arc::clone(&spec))),
        );

        let backend = Backend::memory(store.clone());
        let runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));
        (store, backend, runtime, agent_id, spec)
    }

    /// Where [`SpecSpy`] leaves the last spec a session was started with.
    type SpecSlot = Arc<Mutex<Option<SessionSpec>>>;

    /// Lets one `FakeAdapter` be shared between the test and the factory.
    #[derive(Debug)]
    struct FakeBuilder(Arc<FakeAdapter>, SpecSlot);

    impl htui_agent::registry::TransportBuilder for FakeBuilder {
        fn build(
            &self,
            agent: &Agent,
            on_box: Option<&AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            Ok(Box::new(SpecSpy {
                inner: self.0.build(agent, on_box, caps)?,
                seen: Arc::clone(&self.1),
            }))
        }
    }

    /// The fake driver with one addition: it writes down the [`SessionSpec`] it was started with.
    ///
    /// A wrapper rather than a field on the fake, because the fake is a *transport* under test in
    /// two crates and this is a question about the worker that starts one.
    #[derive(Debug)]
    struct SpecSpy {
        inner: Box<dyn AgentDriver>,
        seen: SpecSlot,
    }

    impl AgentDriver for SpecSpy {
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
        ) -> htui_agent::driver::DriverFuture<'a, Box<dyn AgentSession>> {
            // Recorded before the delegate runs, and the lock released here: the future below is
            // `Send`, and a guard held across it would not compile.
            if let Ok(mut slot) = self.seen.lock() {
                *slot = Some(spec.clone());
            }
            self.inner.start(spec, prompt)
        }
    }

    fn envelope(seq: Seq, request: StoreRequest) -> RequestEnvelope {
        RequestEnvelope {
            seq,
            origin: Origin::Tab(crate::ui::tabs::TabId("chat")),
            request,
        }
    }

    fn start(agent_id: AgentId, prompt: &str) -> StoreRequest {
        StoreRequest::ChatStart {
            project_id: ids::PROJECT_HTUI,
            agent_id,
            model: None,
            prompt: prompt.to_owned(),
        }
    }

    fn scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_PLATFORM,
            project_ids: vec![ids::PROJECT_HTUI],
        }
    }

    /// Drives a `ChatStart` to completion inline and returns every reply it produced.
    ///
    /// The cancel is queued **before** the future is polled, because a chat does not end by
    /// itself: after its turn it waits on the user, exactly as it does in the running binary. The
    /// command channel is unbounded, so the session plays its whole turn and then finds the
    /// waiting `Cancel` — which is the same sequence as a user pressing `Esc Esc`.
    async fn run(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        request: StoreRequest,
    ) -> (StepId, Vec<ReplyEnvelope>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let envelope = envelope(7, request);
        let Served::Start { step_id, task } = runtime.serve(backend, &tx, &envelope).await else {
            panic!("a chat start opens a session")
        };
        let cancel_envelope = RequestEnvelope {
            seq: 8,
            origin: envelope.origin.clone(),
            request: StoreRequest::ChatCancel { step_id },
        };
        let cancel = runtime.serve(backend, &tx, &cancel_envelope).await;
        assert!(
            matches!(cancel, Served::Deferred),
            "the session answers its own cancel: {cancel:?}"
        );
        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }
        (step_id, replies)
    }

    #[tokio::test]
    async fn a_scripted_chat_records_its_turn_and_closes_its_run() {
        let script = Script::one_turn(vec![
            ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
                text: "hello".to_owned(),
                message_id: Some("m1".to_owned()),
            })),
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ]);
        let (store, backend, mut runtime, agent_id) = fixture(script).await;
        let before = store.active_runs(&scope()).await.expect("count");

        let (step_id, replies) = run(&mut runtime, &backend, start(agent_id, "say hello")).await;

        // Every *stream* frame answers the `ChatStart` request, so `App::is_fresh` passes all of
        // them; the `ChatCancel` request gets its own answer at its own `seq`, which is what
        // "exactly one reply per request" means for a request that is not the stream's.
        assert!(
            replies
                .iter()
                .filter(|reply| matches!(reply.reply, StoreReply::Chat(ChatFrame::Event(_))))
                .all(|reply| reply.seq == 7),
            "every stream frame carries the ChatStart seq: {replies:?}"
        );
        assert_eq!(
            replies.iter().filter(|reply| reply.seq == 8).count(),
            1,
            "the cancel is answered exactly once"
        );
        assert!(
            matches!(replies[0].reply, StoreReply::ChatAccepted { .. }),
            "the first reply is the acceptance: {:?}",
            replies[0].reply
        );
        assert!(
            matches!(
                replies.last().map(|reply| &reply.reply),
                Some(StoreReply::Chat(ChatFrame::Ended { .. }))
            ),
            "the last reply ends the stream: {:?}",
            replies.last()
        );

        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("the chat step has a log");
        let kinds: Vec<EventKind> = log.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::Prompt,
                EventKind::Other,
                EventKind::AssistantText,
                EventKind::Done
            ],
            "the prompt, the session banner, the coalesced text and the done"
        );
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "a finished chat stops counting as an active run"
        );
    }

    // -----------------------------------------------------------------------------------------
    // The per-run token cap (`docs/ANA-4.md` §7 `:1143-1150`, §11 criterion 8, plan D69-D71)
    // -----------------------------------------------------------------------------------------

    /// One `usage` report costing `cost` USD micros, with a context reading beside it.
    fn usage(cost: i64) -> ScriptEvent {
        ScriptEvent::Emit(DriverEvent::Usage(htui_agent::event::UsageEvent {
            cost_micros: Some(cost),
            context_used: Some(1_000),
            context_size: Some(200_000),
            ..htui_agent::event::UsageEvent::default()
        }))
    }

    /// The turn's `done`.
    fn ends(stop_reason: StopReason) -> ScriptEvent {
        ScriptEvent::Emit(DriverEvent::Done(DoneEvent { stop_reason }))
    }

    /// The events a reply stream carried, as `DriverEvent`s.
    fn stream_events(replies: &[ReplyEnvelope]) -> Vec<DriverEvent> {
        replies
            .iter()
            .filter_map(|reply| match &reply.reply {
                StoreReply::Chat(ChatFrame::Event(envelope)) => Some(envelope.event.clone()),
                _ => None,
            })
            .collect()
    }

    /// Plan D90: the per-run cap the recorder enforces client-side is also handed to the
    /// transport, on the spec, so a transport with a server-side knob of its own (`--max-budget-usd`,
    /// D83) bounds the same run by the same number.
    ///
    /// The point is that there is **one** figure. Two reads of `project.settings.per_token_cap_run`
    /// would be two chances to convert dollars to micros differently, and the disagreement would
    /// surface as a run that stopped at a figure no setting names.
    #[tokio::test]
    async fn the_projects_run_cap_reaches_the_transport_on_the_spec() {
        let capped = Script::one_turn(vec![ends(StopReason::EndTurn)]);
        let (_store, backend, mut runtime, agent_id, spec) =
            fixture_with_spec_spy(capped, Some(json!({ "per_token_cap_run": 300 }))).await;
        run(&mut runtime, &backend, start(agent_id, "spend a little")).await;
        let started = spec
            .lock()
            .expect("the spy's slot is not poisoned")
            .clone()
            .expect("the chat started a session");
        assert_eq!(
            started.budget_micros,
            Some(300),
            "the same micros `RunCap` is built from, in the same units (D70)"
        );

        // And a project that sets no cap says so, rather than defaulting to a number: a transport
        // that received `Some(0)` would refuse every turn.
        let uncapped = Script::one_turn(vec![ends(StopReason::EndTurn)]);
        let (_store, backend, mut runtime, agent_id, spec) =
            fixture_with_spec_spy(uncapped, None).await;
        run(&mut runtime, &backend, start(agent_id, "spend a little")).await;
        assert_eq!(
            spec.lock()
                .expect("the spy's slot is not poisoned")
                .clone()
                .expect("the chat started a session")
                .budget_micros,
            None
        );
    }

    /// The whole cap, end to end through the production path: `ChatStart` reads
    /// `project.settings.per_token_cap_run`, `run_turn` answers the recorder's verdict with the
    /// shared `enforce_breach`, and the run closes failed with the two rows criterion 8 names.
    ///
    /// **This is the case blueprint P-1 exists for.** `htui_agent::record::pump` is what the
    /// conformance suites drive and production never calls it, so a cap wired into `pump` alone
    /// would pass every suite in this repo and enforce nothing in the binary. Here the loop is
    /// `run_turn`, the one that also serves the command channel, and it reaches the same sequence.
    #[tokio::test]
    async fn a_chat_over_its_run_cap_is_cancelled_and_its_run_fails() {
        // 100 then 250 micros: the running spend is 350 against a cap of 300, crossed by the
        // second report.
        //
        // The turn ends in the transport's own `done { end_turn }`, not in `ExpectCancel` (T53).
        // The fixture's row is a **CLI** row, and since plan D91 the fake plays the profile its row
        // declares (`fake.rs` rule 6): `usage_mid_turn` is `false` there, so the two reports are
        // held and leave as one just ahead of the `done`, which is the shape `docs/ANA-4.md` §7
        // measured on the real dialect. A turn whose cost only exists at its end cannot be stopped
        // before it ends, so the marker has nothing left to guard — and the guard it used to give
        // is not lost, it is stronger: if the cap were never enforced this turn would end
        // `end_turn` and the match below would fail on the stop reason instead of hanging.
        let script = Script::one_turn(vec![
            usage(100),
            usage(250),
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ]);
        let (store, backend, mut runtime, agent_id) =
            fixture_with_project_settings(script, Some(json!({ "per_token_cap_run": 300 }))).await;
        let before = store.active_runs(&scope()).await.expect("count");

        let (step_id, replies) = run(&mut runtime, &backend, start(agent_id, "spend it")).await;

        let events = stream_events(&replies);
        let closing: Vec<&DriverEvent> = events
            .iter()
            .filter(|event| matches!(event, DriverEvent::Error(_) | DriverEvent::Done(_)))
            .collect();
        match closing.as_slice() {
            [DriverEvent::Error(error), DriverEvent::Done(done)] => {
                assert_eq!(error.code, "cap_exceeded");
                assert!(
                    error.message.contains("per_token_cap_run"),
                    "the frame names the setting an operator would change: {}",
                    error.message
                );
                assert_eq!(done.stop_reason, StopReason::Cancelled);
            }
            other => panic!("the tab is told what happened, in order: {other:?}"),
        }
        assert!(
            matches!(
                replies.last().map(|reply| &reply.reply),
                Some(StoreReply::Chat(ChatFrame::Ended {
                    stop_reason: StopReason::Cancelled
                }))
            ),
            "and the stream ends cancelled: {:?}",
            replies.last()
        );

        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("the chat step has a log");
        assert_eq!(
            log.iter()
                .rev()
                .take(2)
                .map(|row| row.kind)
                .collect::<Vec<_>>(),
            vec![EventKind::Done, EventKind::Error],
            "criterion 8: `error{{cap_exceeded}}` then `done{{cancelled}}` are the step's last two \
             rows — read backwards here, so the message says which end it read from"
        );
        assert_eq!(log[log.len() - 2].payload["code"], json!("cap_exceeded"));
        assert_eq!(
            log[log.len() - 2].role,
            EventRole::Htui,
            "`htui` authored it, not the agent"
        );
        assert_eq!(
            log.last().expect("a last row").payload["stop_reason"],
            json!("cancelled")
        );
        assert_eq!(
            log.iter()
                .filter(|row| row.kind == EventKind::Usage)
                .count(),
            1,
            "the turn reported its cost once, and nothing after that report was pulled — the \
             transport's own `done {{end_turn}}` is withheld rather than recorded beside the cap's \
             (plan D91, milestone 7 H-4): {:?}",
            log.iter().map(|row| row.kind).collect::<Vec<_>>()
        );
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "the run is closed, not left running"
        );
    }

    /// A cap key an operator wrote and `htui` could not read **refuses the chat**, at the
    /// `ChatStart` request's own address, naming the key.
    ///
    /// The opposite of `AgentSettings`, where settings that do not parse are settings that are not
    /// set (`htui_agent::registry`), and deliberately so: an unreadable *cap* silently treated as
    /// absent is the plan's "wrong by a factor of a million" risk pointing the other way — a run
    /// that was supposed to be bounded and quietly was not. Refusing costs one visible error and
    /// nothing else, because the read happens before the run row is minted.
    #[tokio::test]
    async fn a_negative_run_cap_refuses_the_chat_start() {
        let script = Script::one_turn(vec![ends(StopReason::EndTurn)]);
        let (store, backend, mut runtime, agent_id) =
            fixture_with_project_settings(script, Some(json!({ "per_token_cap_run": -1 }))).await;
        let before = store.active_runs(&scope()).await.expect("count");
        let (tx, _rx) = mpsc::unbounded_channel();

        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(agent_id, "hi")))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "chat_start");
                assert!(
                    message.contains("per_token_cap_run") && message.contains("micros"),
                    "the refusal names the key and its unit: {message}"
                );
            }
            other => panic!("a cap that does not parse refuses the chat: {other:?}"),
        }
        assert!(
            runtime.live.is_empty(),
            "no session was opened, so nothing is waiting for a command"
        );
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "and no run row was minted for a chat that never started"
        );
    }

    /// Plan D71: `per_token_cap_batch` is **read** — a document carrying it parses, so the chat is
    /// not refused — and **not enforced**, because a batch spans runs MOD-4 does not yet create
    /// (`docs/ANA-4.md`:1283 assigns it to MOD-12).
    ///
    /// A cap of one micro is below the report's own cost, so a build that enforced the batch key
    /// here would cancel this turn instead of finishing it.
    #[tokio::test]
    async fn a_batch_cap_is_read_and_not_enforced() {
        let script = Script::one_turn(vec![usage(100), ends(StopReason::EndTurn)]);
        let (store, backend, mut runtime, agent_id) =
            fixture_with_project_settings(script, Some(json!({ "per_token_cap_batch": 1 }))).await;

        let (step_id, replies) = run(&mut runtime, &backend, start(agent_id, "spend it")).await;

        assert!(
            matches!(replies[0].reply, StoreReply::ChatAccepted { .. }),
            "the chat is accepted: a batch cap is readable, so it is not a bad document"
        );
        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("the chat step has a log");
        assert!(
            !log.iter().any(|row| row.kind == EventKind::Error),
            "and nothing enforced it: {:?}",
            log.iter().map(|row| row.kind).collect::<Vec<_>>()
        );
        assert_eq!(
            log.last().expect("a last row").payload["stop_reason"],
            json!("end_turn"),
            "the turn ended on its own terms"
        );
    }

    /// Review M-3: an offline chat whose project is **not in the mirror** still starts, unbounded;
    /// the same absent row online refuses the chat.
    ///
    /// `project_caps_for` is called directly for `a_buffered_writer_gets_no_latch`'s reason — the
    /// thing under test is a decision, not a turn. The offline half cannot be driven through the
    /// shell at all: an offline chat takes its project from `projects(scope)`, which joins the
    /// mirrored `project` row it is about to be missing, so the situation this guards against
    /// arrives from a caller that names a project id some other way (MOD-4's orchestrator is the
    /// next one) rather than from the Chat tab's own composer.
    ///
    /// The third assertion is the one that keeps the degradation honest: a cap the mirror *does*
    /// hold is enforced offline exactly as online, which is why plan D70 put the cap in a mirrored
    /// column instead of an env knob.
    #[tokio::test]
    async fn an_unmirrored_project_is_unbounded_offline_and_refused_online() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "caps-test", 1)
            .await
            .expect("mirror");
        let buffered = Writer::Buffered(htui_store::BufferedWriter::new(cache.clone()));

        assert_eq!(
            project_caps_for(&buffered, ids::PROJECT_HTUI, None)
                .expect("an unmirrored project does not refuse an offline chat"),
            ProjectCaps::default(),
            "no row, no cap: `{{}}` is unbounded, and the chat records to the buffer as it always \
             did"
        );
        let refused = project_caps_for(&Writer::Memory(MemStore::demo()), ids::PROJECT_HTUI, None);
        match refused {
            Err(StoreError::NotFound { entity, id }) => {
                assert_eq!(entity, "project");
                assert_eq!(
                    id,
                    ids::PROJECT_HTUI.to_string(),
                    "the refusal names the row"
                );
            }
            other => panic!("online, a project the store does not hold refuses: {other:?}"),
        }
        assert_eq!(
            project_caps_for(
                &buffered,
                ids::PROJECT_HTUI,
                Some(json!({ "per_token_cap_run": 300 })),
            )
            .expect("a mirrored document parses offline")
            .run_micros,
            Some(300),
            "a cap the mirror holds is enforced offline, which is why D70 chose this column"
        );
        assert!(
            project_caps_for(
                &buffered,
                ids::PROJECT_HTUI,
                Some(json!({ "per_token_cap_run": -1 }))
            )
            .is_err(),
            "and a document that does not parse still refuses, offline included"
        );
        cache.close().await;
    }

    /// Plan D66-D68: an offline chat latches no allowance and says why, decided at chat start
    /// rather than discovered on the first `usage` row.
    ///
    /// `quota_latch_for` is called directly because the thing under test is a decision, not a
    /// turn: a `Writer::Buffered` refuses every registry write, so the latch has nowhere to go and
    /// the *usage rows* are what carry the figure until they are uploaded.
    #[tokio::test]
    async fn a_buffered_writer_gets_no_latch() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "latch-test", 1)
            .await
            .expect("mirror");
        let agent = fake_row(AgentId::new());

        assert_eq!(
            quota_latch_for(
                &Writer::Buffered(htui_store::BufferedWriter::new(cache.clone())),
                &agent,
                ids::BOX,
                QuotaSource::AcpMetaRateLimit,
            ),
            None,
            "the offline mirror has no `agent_box` table at all, deliberately (plan D68)"
        );
        let online = quota_latch_for(
            &Writer::Memory(MemStore::demo()),
            &agent,
            ids::BOX,
            QuotaSource::AcpMetaRateLimit,
        )
        .expect("a writer that can reach the registry latches");
        assert_eq!(online.agent_id, agent.id, "the row the chat runs on");
        assert_eq!(online.box_id, ids::BOX);
        assert_eq!(
            online.source,
            QuotaSource::AcpMetaRateLimit,
            "the source is the row's declaration, passed through (`R-AGT-5`)"
        );
        assert_eq!(online.billing, agent.billing);
        cache.close().await;
    }

    /// The latch, wired: a chat that reports a cost leaves `agent_box.quota` on the row it ran on.
    ///
    /// T39 proved the recorder latches; this proves the **worker hands it a latch**, which is a
    /// separate claim and the one a user notices — without it the Settings quota column would read
    /// `—` forever with every test still green. `fake_row` declares no quota source, so the
    /// document is spend alone, which is the seeded live-ACP row's shape (plan D65) and `agy`'s.
    #[tokio::test]
    async fn a_chat_latches_the_quota_of_the_row_it_runs_on() {
        let script = Script::one_turn(vec![usage(100), ends(StopReason::EndTurn)]);
        let (store, backend, mut runtime, agent_id) = fixture(script).await;
        // The row a latch writes into: two columns of an **existing** row, so an unprobed agent
        // has nothing to latch into (plan D67) and the chat carries on regardless.
        store
            .upsert_agent_box(&probed_row(agent_id, Some(Utc::now())))
            .await
            .expect("the probed row lands");

        run(&mut runtime, &backend, start(agent_id, "spend it")).await;

        let on_box = store
            .agents()
            .await
            .expect("the registry reads")
            .into_iter()
            .find(|row| row.agent.id == agent_id)
            .expect("the row is registered")
            .on_box
            .expect("this box has an agent_box row");
        let quota = on_box
            .quota
            .expect("the chat latched an allowance document");
        assert_eq!(
            quota["spend"]["session_micros"],
            json!(100),
            "the running spend of the session that just ended: {quota}"
        );
        assert_eq!(quota["source"], json!("none"), "as the row declares");
        assert_eq!(
            quota["billing"],
            json!("per_token"),
            "and the billing the row declares, so a reader knows what `spend` means"
        );
        assert_eq!(
            on_box
                .quota_at
                .map(|at| serde_json::to_value(at).expect("a timestamp serialises")),
            Some(quota["observed_at"].clone()),
            "`agent_box.quota_at` mirrors the document's `observed_at` (ANA-4 §7)"
        );
        assert_eq!(
            on_box.probe,
            probed_row(agent_id, None).probe,
            "and the probe snapshot beside it is untouched (plan D67)"
        );
    }

    #[tokio::test]
    async fn a_command_for_an_unknown_step_is_refused() {
        let (_, backend, mut runtime, _) = fixture(Script::default()).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::ChatSend {
                        step_id: StepId::new(),
                        text: "hi".to_owned(),
                    },
                ),
            )
            .await;
        assert!(
            matches!(
                served,
                Served::Reply(StoreReply::Failed {
                    request: "chat_send",
                    ..
                })
            ),
            "{served:?}"
        );
    }

    /// An offline backend is no longer refused for *being* offline (milestone 4, D34): it hands
    /// out `Writer::Buffered` and a chat records to `<cache_dir>/pending/`. What still refuses it
    /// is a mirror that has nothing in it — a box that has never synced cannot name
    /// `run.target_box_id`, and the refusal says which row is missing rather than "offline".
    #[tokio::test]
    async fn an_offline_backend_with_an_empty_mirror_refuses_and_names_what_is_missing() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "chat-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(
            backend.writer().is_some(),
            "the offline write path is the buffer, not a refusal"
        );

        let mut runtime = AgentRuntime::new(DriverFactory::new());
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(AgentId::new(), "hi")))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "chat_start");
                assert!(
                    message.contains("box") && message.contains("not registered"),
                    "the refusal names the row this box has never synced: {message}"
                );
            }
            other => panic!("a chat with no box row must be refused: {other:?}"),
        }
        cache.close().await;
    }

    /// An `acp` registry row the fake transport answers for, whose `launch` names a command that
    /// does not exist.
    ///
    /// The re-probe cases need a row that is **`acp`** (so tier 2 is in scope) and whose launch
    /// resolves without ever reaching a real binary: `command: "unused"` has no `${placeholder}`,
    /// so tier 1 is trivially complete, and `launch::spawn`'s lookup then fails — `failed`, with
    /// no process anywhere near this test (blueprint H-7).
    fn acp_fake_row(id: AgentId) -> Agent {
        Agent {
            // A name of its own: `agent.name` is unique, and one test registers this row beside
            // `fake_row`'s.
            name: "scripted-acp".to_owned(),
            transport: Transport::Acp,
            ..fake_row(id)
        }
    }

    /// A factory whose `acp` transport is the scripted fake.
    fn acp_factory(script: Script) -> DriverFactory {
        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let mut factory = DriverFactory::new();
        factory.register(
            "acp",
            Box::new(FakeBuilder(Arc::clone(&adapter), SpecSlot::default())),
        );
        factory
    }

    /// An `agent_box` row for `agent_id` last probed at `probed_at`.
    fn probed_row(agent_id: AgentId, probed_at: Option<DateTime<Utc>>) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.48.0".to_owned()),
            path: None,
            probed_at,
            quota: None,
            quota_at: None,
            updated_at: probed_at.unwrap_or_else(Utc::now),
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        }
    }

    /// A registry row last edited at `updated_at`.
    ///
    /// The staleness cases pin `agent.updated_at` rather than taking [`fake_row`]'s demo timestamp
    /// as given: that constant is fixed in the calendar and these cases are relative to `now`, so
    /// a row whose edit time is stated in the case is the only one that cannot start reading as
    /// hand-edited on some future afternoon.
    fn edited_row(agent_id: AgentId, updated_at: DateTime<Utc>) -> Agent {
        Agent {
            updated_at,
            ..fake_row(agent_id)
        }
    }

    /// Plan D55: a row nobody has probed, or one probed longer ago than [`PROBE_TTL`], is stale.
    #[test]
    fn needs_reprobe_is_absent_or_older_than_the_ttl() {
        let now = Utc::now();
        let agent_id = AgentId::new();
        // Edited a month ago: older than every `probed_at` below, so age is the only thing any of
        // these assertions is measuring.
        let agent = edited_row(agent_id, now - chrono::TimeDelta::days(30));
        assert!(
            needs_reprobe(&agent, None, now),
            "an unprobed box has nothing to go on"
        );
        assert!(
            needs_reprobe(&agent, Some(&probed_row(agent_id, None)), now),
            "a row with no `probed_at` is a row no probe ever wrote"
        );
        assert!(
            needs_reprobe(
                &agent,
                Some(&probed_row(
                    agent_id,
                    Some(now - chrono::TimeDelta::hours(25))
                )),
                now
            ),
            "25 hours is past the 24-hour window"
        );
        assert!(
            !needs_reprobe(
                &agent,
                Some(&probed_row(
                    agent_id,
                    Some(now - chrono::TimeDelta::hours(23))
                )),
                now
            ),
            "23 hours is inside it: `agy` self-updates in place, not every hour"
        );
        // A clock that went backwards (an NTP step, a suspended laptop) must not read as stale
        // for the next 24 hours' worth of drift.
        assert!(
            !needs_reprobe(
                &agent,
                Some(&probed_row(
                    agent_id,
                    Some(now + chrono::TimeDelta::hours(1))
                )),
                now
            ),
            "a future `probed_at` is not stale"
        );
    }

    /// Blueprint H-9 widened from one chat to the runtime: a re-probe for a
    /// `(agent_id, box_id)` excludes every other re-probe for that pair, whichever trigger asked
    /// for it, and stops excluding them the moment it is done.
    ///
    /// The unit under test is the claim itself rather than a pair of overlapping chats, and
    /// deliberately: `run_reprobe` hard-wires [`SpawnTier2`], so "how many tier-2 handshakes ran"
    /// is not observable from the store — both re-probes of an overlap write the same verdict for
    /// the same row, which is exactly why the race was benign enough to survive review. What is
    /// observable is the exclusion, and the wiring that uses it is one `let`-`else`.
    #[test]
    fn a_reprobe_claim_excludes_the_same_row_until_it_is_dropped() {
        let claims = ReprobeClaims::default();
        let agent_id = AgentId::new();
        let key = (agent_id, ids::BOX);

        let held = claims.claim(key).expect("nobody is probing this row");
        assert!(
            claims.claim(key).is_none(),
            "a second re-probe for one row is the race H-9 is about"
        );
        assert!(
            claims.claim((AgentId::new(), ids::BOX)).is_some(),
            "another agent on this box is a different row and is not excluded"
        );
        assert!(
            claims.claim((agent_id, BoxId::new())).is_some(),
            "the same agent on another box is a different row too"
        );

        drop(held);
        assert!(
            claims.claim(key).is_some(),
            "the claim is released when the re-probe ends — including when its task is aborted, \
             which is the only way `AgentRuntime::shutdown` ends one"
        );
    }

    /// The other way to be stale, and the one age alone cannot see: the recording is older than
    /// the row it was made from.
    ///
    /// D58 spawns `probe.resolved` whenever its rules pass, so a hand-edited `agent.launch` — new
    /// args, a new `HTUI_TOOL_*`, a different binary — would be ignored for up to
    /// [`PROBE_TTL`] while every chat kept spawning the recording made from the *old* row. The
    /// edit is the event; `agent.updated_at` is where the store records it.
    #[test]
    fn needs_reprobe_when_the_row_was_edited_after_it_was_probed() {
        let now = Utc::now();
        let agent_id = AgentId::new();
        let probed_at = now - chrono::TimeDelta::hours(2);
        let on_box = probed_row(agent_id, Some(probed_at));

        assert!(
            needs_reprobe(
                &edited_row(agent_id, probed_at + chrono::TimeDelta::minutes(1)),
                Some(&on_box),
                now
            ),
            "a row edited after the probe read it is stale however recently it was probed"
        );
        assert!(
            !needs_reprobe(
                &edited_row(agent_id, probed_at - chrono::TimeDelta::minutes(1)),
                Some(&on_box),
                now
            ),
            "a row the probe read *after* its last edit is what the recording was made from"
        );
        // The probe writes `agent_box`, never `agents`, so a re-probe cannot move `updated_at` and
        // the two stamps meeting exactly is the ordinary steady state, not an edit.
        assert!(
            !needs_reprobe(&edited_row(agent_id, probed_at), Some(&on_box), now),
            "the same instant is not an edit the probe missed"
        );
    }

    /// Plan D55, the whole shape in one: a stale `acp` row starts its chat **and** gets a tier-2
    /// re-probe in the background, and the re-probe's failure never reaches the chat.
    #[tokio::test]
    async fn a_chat_start_on_a_stale_acp_row_re_probes_in_the_background() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(agent_id))
            .await
            .expect("the acp row lands");
        let backend = Backend::memory(store.clone());
        let mut runtime =
            AgentRuntime::new(acp_factory(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )])))
            .with_grace(Duration::from_millis(0));

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await;
        assert!(
            matches!(served, Served::Start { .. }),
            "the chat starts on what resolution already gave it: {served:?}"
        );
        assert_eq!(
            runtime.background_len(),
            1,
            "an unprobed row is re-probed beside the chat, never in front of it"
        );

        runtime.finish_background(Duration::from_secs(5)).await;
        let on_box = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .expect("the row is still there")
            .on_box
            .expect("the re-probe wrote agent_box");
        assert_eq!(
            on_box
                .probe
                .as_ref()
                .and_then(|probe| probe.get("status"))
                .and_then(Value::as_str),
            Some("failed"),
            "`command: unused` spawns nothing: {:?}",
            on_box.probe
        );
        assert!(
            !on_box.enabled,
            "a launch that will not spawn is not enabled"
        );
        assert!(on_box.probed_at.is_some());
    }

    /// A re-probe mid-chat leaves the latched quota standing, and since MOD-2 plan D74 it is the
    /// **store** that guarantees that rather than the probe.
    ///
    /// `probe::agent_box_row` projects `quota: None, quota_at: None` and
    /// `WriteStore::upsert_agent_box` writes neither column, so the row the re-probe hands back
    /// cannot discard a latch however stale its own copy is. It used to carry both forward, which
    /// was the lost update D74 removes. `existing` is still load-bearing for the other reason:
    /// `probe_agent` reads its `probe.source` for the manual-entry rule.
    ///
    /// Without this case, a backend that replaced the row wholesale would wipe a box's quota on
    /// every stale-row chat start and no test would notice.
    #[tokio::test]
    async fn a_stale_row_keeps_the_quota_the_probe_does_not_own() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(agent_id))
            .await
            .expect("the acp row lands");
        let quota_at = Utc::now() - chrono::TimeDelta::hours(30);
        let stale = probed_row(agent_id, Some(Utc::now() - chrono::TimeDelta::hours(25)));
        store
            .upsert_agent_box(&stale)
            .await
            .expect("the stale agent_box lands");
        // Through the narrow setter, because since D74 that is the only way the two columns are
        // ever written — an `upsert_agent_box` carrying a `quota` stores `None`.
        store
            .set_agent_box_quota(agent_id, stale.box_id, json!({ "remaining": 1 }), quota_at)
            .await
            .expect("the latch lands on the stale row");
        let backend = Backend::memory(store.clone());
        let mut runtime =
            AgentRuntime::new(acp_factory(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )])))
            .with_grace(Duration::from_millis(0));

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(9, start(agent_id, "hello")))
            .await;
        assert!(matches!(served, Served::Start { .. }), "{served:?}");
        assert_eq!(runtime.background_len(), 1, "a 25 h old row is stale");
        runtime.finish_background(Duration::from_secs(5)).await;

        let on_box = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .expect("the row is still there")
            .on_box
            .expect("the re-probe wrote agent_box");
        assert_eq!(
            on_box.quota,
            Some(json!({ "remaining": 1 })),
            "the upsert cannot write the two columns, so the latched value stands (D74)"
        );
        assert_eq!(on_box.quota_at, Some(quota_at), "and its timestamp with it");
        assert_ne!(
            on_box.probed_at, stale.probed_at,
            "the row was re-probed, so this assertion is over a row the probe actually rewrote"
        );
    }

    /// The three rows that are **not** re-probed: a `cli` one (no `initialize` to complete), a
    /// fresh one, and one whose writer cannot hold the answer.
    #[tokio::test]
    async fn a_cli_row_and_a_fresh_row_are_not_re_probed() {
        let (store, backend, mut runtime, cli_agent) =
            fixture(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )]))
            .await;
        let (tx, _rx) = mpsc::unbounded_channel();

        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(cli_agent, "hello")))
            .await;
        assert!(matches!(served, Served::Start { .. }), "{served:?}");
        assert_eq!(
            runtime.background_len(),
            0,
            "a `cli` row has no handshake to re-run (plan D55 is tier 2 only)"
        );

        // The same box, an `acp` row, probed a minute ago: inside the TTL. The runtime is a
        // second one because `fixture`'s factory knows only `cli/fake`, and this half has to
        // reach the re-probe decision rather than be refused before it.
        let acp_agent = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(acp_agent))
            .await
            .expect("the acp row lands");
        store
            .upsert_agent_box(&probed_row(
                acp_agent,
                Some(Utc::now() - chrono::TimeDelta::minutes(1)),
            ))
            .await
            .expect("the fresh agent_box lands");
        let mut runtime =
            AgentRuntime::new(acp_factory(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )])))
            .with_grace(Duration::from_millis(0));
        let served = runtime
            .serve(&backend, &tx, &envelope(2, start(acp_agent, "hello")))
            .await;
        assert!(
            matches!(served, Served::Start { .. }),
            "the chat starts, which is what puts the TTL on the path: {served:?}"
        );
        assert_eq!(
            runtime.background_len(),
            0,
            "a row probed a minute ago is not re-probed"
        );
    }

    /// Plan D52 for the chat path: a `Writer::Buffered` refuses `upsert_agent_box`, so a re-probe
    /// against one would spawn an adapter to throw its answer away.
    #[tokio::test]
    async fn a_buffered_writer_never_re_probes() {
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = htui_store::CacheStore::open(root.path(), "reprobe-test", 1)
            .await
            .expect("a fresh mirror");
        let agent_id = AgentId::new();
        let mut demo = htui_core::fixtures::demo_data();
        for user in &mut demo.users {
            user.name = htui_store::identity::os_user_name();
        }
        demo.agents = vec![acp_fake_row(agent_id)];
        htui_store::testkit::seed_mirror(&cache, &demo)
            .await
            .expect("the mirror is seeded");

        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let mut runtime =
            AgentRuntime::new(acp_factory(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )])))
            .with_grace(Duration::from_millis(0));
        let (tx, _rx) = mpsc::unbounded_channel();

        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(agent_id, "hello")))
            .await;
        assert!(
            matches!(served, Served::Start { .. }),
            "an offline chat still starts: {served:?}"
        );
        assert_eq!(
            runtime.background_len(),
            0,
            "probing costs process spawns; a writer that refuses the row is refused first"
        );
        drop(served);
        runtime.shutdown(Duration::ZERO).await;
        cache.close().await;
    }

    /// An `acp` registry row whose command exists nowhere.
    ///
    /// No `discovery`, so tier 1 has no placeholder to resolve and nothing is searched for: the
    /// first thing that can fail is `launch::spawn`'s own lookup, which is [`DriverError::Spawn`]
    /// — the one error D60 acts on. Nothing is started anywhere near this test (blueprint H-7).
    fn unspawnable_row(id: AgentId) -> Agent {
        Agent {
            name: "unspawnable-acp".to_owned(),
            transport: Transport::Acp,
            launch: json!({ "command": "/nonexistent/htui-d60/agent" }),
            settings: json!({}),
            ..fake_row(id)
        }
    }

    /// This box's `agent_box` row for `agent_id`, read back through the store.
    async fn stored_box(store: &MemStore, agent_id: AgentId) -> AgentBox {
        store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .expect("the registry row is still there")
            .on_box
            .expect("this box has a row for that agent")
    }

    /// `probe.status`, the one key every re-probe case reads.
    fn probe_status(row: &AgentBox) -> Option<&str> {
        row.probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(Value::as_str)
    }

    /// Plan D60: a chat that cannot **spawn** its adapter refreshes this box's row for that agent,
    /// in the failed chat's own task.
    ///
    /// The pre-inserted row is deliberately *fresh*, which takes the staleness trigger (D55) off
    /// the path and leaves D60 as the only re-probe that can run. `background_len() == 0` with the
    /// row untouched **before the task is polled** is `R-NF-3` in the runtime's own terms: the
    /// worker's `select!` arm returned having spawned nothing and written nothing.
    #[tokio::test]
    async fn a_spawn_failure_reports_the_adapters_message_and_reprobes_the_row_off_the_arm() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&unspawnable_row(agent_id))
            .await
            .expect("the acp row lands");
        let fresh = probed_row(agent_id, Some(Utc::now()));
        store
            .upsert_agent_box(&fresh)
            .await
            .expect("the fresh agent_box lands");
        let backend = Backend::memory(store.clone());
        // The real ACP builder, because the fact under test is what `launch::spawn` does with a
        // command that is not there; a scripted adapter cannot fail to spawn.
        let mut runtime = AgentRuntime::production().with_grace(Duration::from_millis(0));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await;
        let Served::Start { task, .. } = served else {
            panic!("a chat start opens a session: {served:?}")
        };
        assert_eq!(
            runtime.background_len(),
            0,
            "a fresh row is not stale, so nothing was spawned on the arm"
        );
        let before = stored_box(&store, agent_id).await;
        assert_eq!(
            probe_status(&before),
            Some("ready"),
            "nothing has run yet: {:?}",
            before.probe
        );
        assert_eq!(
            before.probed_at, fresh.probed_at,
            "and the arm wrote nothing"
        );

        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }

        let message = replies
            .iter()
            .find_map(|reply| match &reply.reply {
                StoreReply::Failed {
                    request: "chat_start",
                    message,
                } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the request that asked is answered: {replies:?}"));
        assert!(
            message.contains("is not executable"),
            "the adapter's own message, not a rewrite of it: {message}"
        );
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.reply, StoreReply::Chat(ChatFrame::Failed { .. }))),
            "the stream is failed as well as the request: {replies:?}"
        );

        let after = stored_box(&store, agent_id).await;
        assert_eq!(
            probe_status(&after),
            Some("failed"),
            "the spawn failure is a fact about the row: {:?}",
            after.probe
        );
        assert!(
            after
                .probe
                .as_ref()
                .and_then(|probe| probe.pointer("/stderr_tail/0"))
                .and_then(Value::as_str)
                .is_some_and(|line| line.contains("not executable")),
            "the re-probe recorded why: {:?}",
            after.probe
        );
        assert!(
            !after.enabled,
            "a launch that will not spawn is not enabled"
        );
        assert_ne!(
            after.probed_at, before.probed_at,
            "the row was re-probed, not merely rewritten"
        );
    }

    /// Blueprint H-8: D60 names `Spawn` and nothing else.
    ///
    /// A placeholder that resolves nowhere is [`DriverError::Unresolved`], which the 24-hour
    /// staleness path already covers and whose message already names the tool. Re-probing here
    /// would spawn an adapter to re-learn what the error just said.
    #[tokio::test]
    async fn an_unresolved_placeholder_does_not_reprobe() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        let mut agent = unspawnable_row(agent_id);
        agent.launch = json!({
            "command": "${gone}",
            "args": [],
            "env": {},
            "discovery": {
                "tools": {
                    "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                },
                "handshake": true
            }
        });
        store.upsert_agent(&agent).await.expect("the acp row lands");
        let fresh = probed_row(agent_id, Some(Utc::now()));
        store
            .upsert_agent_box(&fresh)
            .await
            .expect("the fresh agent_box lands");
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::production().with_grace(Duration::from_millis(0));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await;
        let Served::Start { task, .. } = served else {
            panic!("a chat start opens a session: {served:?}")
        };
        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }

        let message = replies
            .iter()
            .find_map(|reply| match &reply.reply {
                StoreReply::Failed {
                    request: "chat_start",
                    message,
                } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the request that asked is answered: {replies:?}"));
        assert!(
            message.contains("gone"),
            "the message names the placeholder: {message}"
        );

        let after = stored_box(&store, agent_id).await;
        assert_eq!(
            probe_status(&after),
            Some("ready"),
            "an `Unresolved` chat start leaves the row alone: {:?}",
            after.probe
        );
        assert_eq!(after.probed_at, fresh.probed_at);
        assert_eq!(runtime.background_len(), 0);
    }

    /// Blueprint H-9: a stale row and a spawn failure on the same chat are **one** re-probe.
    ///
    /// `ChatArgs.reprobe` is `None` once the staleness path has spawned one, so the two triggers
    /// cannot race each other for the same primary key. `MemStore` exposes no write counter, so
    /// the count is read off the row itself: `upsert_agent_box` stamps `updated_at` per write, and
    /// a second re-probe would necessarily move it.
    #[tokio::test]
    async fn a_stale_row_reprobes_once_not_twice() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&unspawnable_row(agent_id))
            .await
            .expect("the acp row lands");
        store
            .upsert_agent_box(&probed_row(
                agent_id,
                Some(Utc::now() - chrono::TimeDelta::hours(25)),
            ))
            .await
            .expect("the stale agent_box lands");
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::production().with_grace(Duration::from_millis(0));

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await;
        let Served::Start { task, .. } = served else {
            panic!("a chat start opens a session: {served:?}")
        };
        assert_eq!(runtime.background_len(), 1, "a 25 h old row is stale");
        runtime.finish_background(Duration::from_secs(5)).await;
        let after_staleness = stored_box(&store, agent_id).await;
        assert_eq!(
            probe_status(&after_staleness),
            Some("failed"),
            "the staleness re-probe is the one that wrote: {:?}",
            after_staleness.probe
        );

        task.await;
        let after_chat = stored_box(&store, agent_id).await;
        assert_eq!(
            after_chat.updated_at, after_staleness.updated_at,
            "the spawn failure found a re-probe already running and started no second one"
        );
        assert_eq!(after_chat.probed_at, after_staleness.probed_at);
    }

    /// A registry whose every row resolves nowhere — the probe fixture rule (blueprint H-7).
    ///
    /// This box has `node`, `claude` and the ACP adapter installed, so a test that probed an
    /// unmodified seed row would spawn the real adapter inside `cargo test` and wait up to
    /// `HANDSHAKE_TIMEOUT` on it. Every probe test outside `htui-agent`'s `probe_live.rs` runs
    /// against this registry instead: the `discovery` names one tool that cannot exist, so tier 1
    /// reports `missing` and **nothing is spawned**.
    pub(crate) async fn unresolvable_registry() -> MemStore {
        let store = MemStore::demo();
        for summary in store.agents().await.expect("the memory store never fails") {
            let mut agent = summary.agent;
            agent.launch = json!({
                "command": "${gone}",
                "args": [],
                "env": {},
                "discovery": {
                    "tools": {
                        "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                    },
                    "handshake": true
                }
            });
            store.upsert_agent(&agent).await.expect("the row updates");
        }
        store
    }

    /// Plan D52: with no writable registry the request is refused **before** anything is spawned.
    #[tokio::test]
    async fn an_offline_backend_refuses_the_probe_before_spawning_anything() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "probe-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let mut runtime = AgentRuntime::new(DriverFactory::new());
        let (tx, _rx) = mpsc::unbounded_channel();

        let served = runtime
            .serve(&backend, &tx, &envelope(1, StoreRequest::ProbeAgents))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(
                    message.contains(htui_store::REGISTRY_ON_SERVER_ONLY),
                    "the buffered writer's own sentence, not a second one: {message}"
                );
            }
            other => panic!("a buffered writer refuses the probe: {other:?}"),
        }
        assert_eq!(
            runtime.background_len(),
            0,
            "probing costs process spawns; a probe with nowhere to write is refused before any"
        );
        cache.close().await;

        // The other refusal, for the same reason: a box that is not registered has no `agent_box`
        // primary key to write against.
        let backend = Backend::memory(MemStore::new());
        let served = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeAgents))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(message.contains("not registered"), "{message}");
            }
            other => panic!("an unregistered box refuses the probe: {other:?}"),
        }
        assert_eq!(runtime.background_len(), 0);
    }

    /// Plan D53: the task the runtime owns answers the request itself, exactly once, at the
    /// request's own address, and with the reply the Settings section already renders.
    #[tokio::test]
    async fn the_probe_task_answers_once_at_the_requests_address_with_agents() {
        let store = unresolvable_registry().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(DriverFactory::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = RequestEnvelope {
            seq: 7,
            origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
            request: StoreRequest::ProbeAgents,
        };

        let served = runtime.serve(&backend, &tx, &request).await;
        assert!(
            matches!(served, Served::Deferred),
            "the probe is deferred to the runtime's own task: {served:?}"
        );
        assert_eq!(runtime.background_len(), 1);

        runtime.finish_background(Duration::from_secs(5)).await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }

        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        assert_eq!(replies[0].seq, 7);
        assert!(
            matches!(&replies[0].origin, Origin::Tab(id) if id.0 == "settings"),
            "{:?}",
            replies[0].origin
        );
        let StoreReply::Agents(rows) = &replies[0].reply else {
            panic!(
                "the probe answers with the registry: {:?}",
                replies[0].reply
            )
        };
        // Three since MOD-2 D79 added the `cli` row: the demo fixture derives from `seed_rows`,
        // so a registry addition lands here without this test being about the registry.
        assert_eq!(rows.len(), 3);
        for row in rows {
            let on_box = row
                .on_box
                .as_ref()
                .expect("every enabled row was probed and written");
            assert_eq!(
                on_box
                    .probe
                    .as_ref()
                    .and_then(|probe| probe.get("status"))
                    .and_then(Value::as_str),
                Some("missing"),
                "a tool that resolves nowhere is `missing`: {:?}",
                on_box.probe
            );
            assert!(
                !on_box.enabled,
                "a missing agent is not enabled on this box"
            );
        }

        let stored = store.agents().await.expect("the memory store never fails");
        assert_eq!(
            stored
                .iter()
                .filter(|summary| summary.on_box.is_some())
                .count(),
            3,
            "the reply states what the task itself wrote"
        );
    }

    /// A runtime that installs from `fixture` into `root`, and knows no transport at all.
    ///
    /// `DriverFactory::new()`: an install never builds a driver, and a runtime that could would
    /// let a case pass for the wrong reason.
    fn installing_runtime(fixture: &Fixture, root: &std::path::Path) -> AgentRuntime {
        AgentRuntime::new(DriverFactory::new())
            .with_installer(InstallConfig::new(fixture.base(), Some(root.to_path_buf())))
    }

    /// Plan D18's first refusal, and the one that has to come before the network: a writer that
    /// cannot hold the row refuses the **plan**, so no registry read is ever spent on an install
    /// whose result could not be written.
    #[tokio::test]
    async fn a_plan_is_refused_before_any_request_on_a_buffered_writer() {
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = htui_store::CacheStore::open(root.path(), "install-buffered", 1)
            .await
            .expect("a fresh mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let fixture = Fixture::start().await;
        let agents = root.path().join("agents");
        let mut runtime = installing_runtime(&fixture, &agents);

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::InstallPlan {
                        agent_id: AgentId::new(),
                    },
                ),
            )
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "install_plan");
                assert!(
                    message.contains(htui_store::REGISTRY_ON_SERVER_ONLY),
                    "the buffered writer's own sentence, not a second one: {message}"
                );
            }
            other => panic!("a buffered writer refuses the plan: {other:?}"),
        }
        assert!(
            !runtime.install_running(),
            "the refusal is before the claim, so a later plan is not locked out"
        );
        assert_eq!(
            runtime.background_len(),
            0,
            "and before anything is spawned"
        );
        assert!(
            fixture.lines().is_empty(),
            "and before a single request: {:?}",
            fixture.lines()
        );
        assert!(
            !agents.exists(),
            "and before the install root is so much as created"
        );

        cache.close().await;
    }

    /// Hazard H-10: the claim covers planning **and** installing, so a second `i` while a
    /// pre-flight is still reading the registry is refused rather than racing it.
    #[tokio::test]
    async fn a_second_plan_while_one_runs_is_refused() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        // The registry read never answers inside this test's lifetime, so the first plan is
        // provably still running when the second one arrives.
        fixture.route(
            "/registry.json",
            Route {
                status: 200,
                delay: Duration::from_secs(30),
                ..Route::default()
            },
        );
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));

        let (tx, _rx) = mpsc::unbounded_channel();
        let first = runtime
            .serve(
                &backend,
                &tx,
                &envelope(1, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        assert!(
            matches!(first, Served::Deferred),
            "the pre-flight answers from its own task: {first:?}"
        );
        assert!(
            runtime.install_running(),
            "planning holds the claim, not only installing"
        );

        let second = runtime
            .serve(
                &backend,
                &tx,
                &envelope(2, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        match second {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "install_plan");
                assert!(
                    message.contains("already running"),
                    "the refusal says what is holding the claim: {message}"
                );
            }
            other => panic!("two installs of one id would race on `.staging/`: {other:?}"),
        }

        runtime.shutdown(Duration::ZERO).await;
        assert!(
            !runtime.install_running(),
            "and the claim goes with the runtime"
        );
    }

    /// A probe and an install both write `agent_box`, so neither may run while the other does
    /// (review finding, MOD-20 T7). The Settings section's `probing` flag cannot be the guard:
    /// **any** `StoreReply::Agents` clears it and `wants_requests` re-issues `Agents` on every
    /// activation, so `r` → switch tab → back → `i` arrives here with the probe still running.
    #[tokio::test]
    async fn a_probe_and_an_install_never_write_the_same_row_at_once() {
        // `unresolvable_registry` and not `MemStore::demo`: this test lets a probe actually run,
        // and the demo rows resolve real adapters — the probe would spawn a handshake child per
        // row and wait on it. Every row here resolves nowhere, so the probe is quick and answers
        // the same whatever is installed on the box running the suite.
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        fixture.route(
            "/registry.json",
            Route {
                status: 200,
                delay: Duration::from_secs(30),
                ..Route::default()
            },
        );
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));
        let (tx, _rx) = mpsc::unbounded_channel();

        // A probe first: it lands in `background`, where the install claim must see it.
        let probing = runtime
            .serve(&backend, &tx, &envelope(1, StoreRequest::ProbeAgents))
            .await;
        assert!(
            matches!(probing, Served::Deferred),
            "the probe answers from its own task: {probing:?}"
        );
        match runtime
            .serve(
                &backend,
                &tx,
                &envelope(2, StoreRequest::InstallPlan { agent_id }),
            )
            .await
        {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "install_plan");
                assert!(
                    message.contains("probe is already running"),
                    "the refusal names what holds the box: {message}"
                );
            }
            other => panic!("an install beside a probe races on `agent_box`: {other:?}"),
        }
        assert_eq!(
            runtime.background_len(),
            1,
            "and the refusal spawned nothing of its own"
        );
        runtime.shutdown(Duration::ZERO).await;

        // Then the other way round: an install in flight refuses a probe.
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));
        let planning = runtime
            .serve(
                &backend,
                &tx,
                &envelope(3, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        assert!(matches!(planning, Served::Deferred), "{planning:?}");
        match runtime
            .serve(&backend, &tx, &envelope(4, StoreRequest::ProbeAgents))
            .await
        {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(
                    message.contains("install is running"),
                    "the refusal names what holds the box: {message}"
                );
            }
            other => panic!("a probe beside an install races on `agent_box`: {other:?}"),
        }
        runtime.shutdown(Duration::ZERO).await;
    }

    /// A row whose `discovery` declares no source is refused **by the row**, with the pre-flight's
    /// own sentence and without a request: the answer is in the document the worker already holds.
    #[tokio::test]
    async fn a_plan_for_a_row_without_install_is_refused_by_name() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "undeclared", false))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(1, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "install_plan");
                assert!(
                    message.contains("nothing declares how to install")
                        && message.contains("undeclared"),
                    "the refusal names the row, so the section can render it as it stands: \
                     {message}"
                );
            }
            other => panic!("a row with no declared source cannot be installed: {other:?}"),
        }
        assert!(!runtime.install_running());
        assert!(
            fixture.lines().is_empty(),
            "and nothing was asked of the registry to find that out: {:?}",
            fixture.lines()
        );
    }

    /// Plan D18's stream contract: one request, many replies, every one of them at the confirm's
    /// own `seq` — and the row written **before** the terminal frame.
    ///
    /// The archive's one entry is not an executable in any format, so the re-probe's spawn fails
    /// the moment the tree is promoted. That is the point rather than a shortcut: the pipeline
    /// runs end to end, the *probe* decides what the box can run (`R-AGT-6`), and the outcome the
    /// worker writes is a row that says so.
    #[tokio::test]
    async fn confirm_streams_frames_at_the_confirm_seq() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store.clone());
        let fixture = Fixture::start().await;
        fixture.route(
            "/archive.zip",
            Route {
                status: 200,
                body: stored_zip(b"htui install fixture, not an executable\n"),
                ..Route::default()
            },
        );
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let root = tmp.path().join("agents");
        let mut runtime = installing_runtime(&fixture, &root);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let plan = demo_plan(agent_id, root.clone(), fixture.url("/archive.zip"));
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::InstallConfirm {
                        plan: Box::new(plan),
                    },
                ),
            )
            .await;
        assert!(
            matches!(served, Served::Deferred),
            "the install answers from its own task: {served:?}"
        );

        let mut frames = Vec::new();
        let outcome = loop {
            let reply = tokio::time::timeout(Duration::from_secs(60), rx.recv())
                .await
                .expect("the install answers inside a minute")
                .expect("the reply channel is open");
            assert_eq!(
                reply.seq, 1,
                "every frame of a confirm carries the confirm's own seq: {:?}",
                reply.reply
            );
            assert!(
                matches!(&reply.origin, Origin::Tab(id) if id.0 == "chat"),
                "and its origin: {:?}",
                reply.origin
            );
            match reply.reply {
                StoreReply::Install(InstallFrame::Done(outcome)) => break outcome,
                StoreReply::Install(frame) => frames.push(frame),
                other => panic!("an install answers with install frames: {other:?}"),
            }
        };
        assert!(
            frames
                .iter()
                .any(|frame| matches!(frame, InstallFrame::Progress { .. })),
            "the stream says where it got to: {frames:?}"
        );

        // Read the instant the terminal frame is in hand and before the task is joined: this is
        // exactly what the section does when it issues `StoreRequest::Agents` on `Done`.
        let on_box = stored_box(&store, agent_id).await;
        assert!(
            on_box.probed_at.is_some(),
            "the row the install wrote is the probe's own: {on_box:?}"
        );
        assert!(
            matches!(*outcome, InstallOutcome::Failed { .. }),
            "a tree the probe cannot handshake is a failed install, however well it downloaded: \
             {outcome:?}"
        );
        assert!(
            root.join(INSTALL_ID).join(INSTALL_VERSION).is_dir(),
            "and D16(c) leaves the tree where it is, for the user to look at"
        );

        runtime.finish_background(Duration::from_secs(5)).await;
        assert!(!runtime.install_running(), "the claim ends with the task");
    }

    /// Plan D18's cancellation: cooperative, through the token. The request is acknowledged at
    /// once and the running stream ends itself.
    ///
    /// The acknowledgement is a `Served::Reply`, which the worker loop addresses to the
    /// `InstallCancel`'s own `seq`; the `Cancelled` frame below is the install's, at the
    /// confirm's. Two requests, two addresses, one token.
    #[tokio::test]
    async fn cancel_answers_cancelling_then_the_stream_ends_cancelled() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store.clone());
        let fixture = Fixture::start().await;
        // Half the body, then a pause: the download is inside its `select!` when the token trips,
        // which is the arm hazard H-6 is written about.
        fixture.route(
            "/archive.zip",
            Route {
                status: 200,
                body: stored_zip(b"htui install fixture, not an executable\n"),
                chunk_delay: Duration::from_secs(30),
                ..Route::default()
            },
        );
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let root = tmp.path().join("agents");
        let mut runtime = installing_runtime(&fixture, &root);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let plan = demo_plan(agent_id, root.clone(), fixture.url("/archive.zip"));
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::InstallConfirm {
                        plan: Box::new(plan),
                    },
                ),
            )
            .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");

        let cancelled = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::InstallCancel))
            .await;
        assert!(
            matches!(
                cancelled,
                Served::Reply(StoreReply::Install(InstallFrame::Cancelling))
            ),
            "a cancel is answered at once, not when the install notices: {cancelled:?}"
        );

        let last = loop {
            let reply = tokio::time::timeout(Duration::from_secs(30), rx.recv())
                .await
                .expect("the cancelled install ends inside 30 s")
                .expect("the reply channel is open");
            assert_eq!(reply.seq, 1, "the stream keeps its own address: {reply:?}");
            if !matches!(
                reply.reply,
                StoreReply::Install(InstallFrame::Progress { .. })
            ) {
                break reply.reply;
            }
        };
        assert!(
            matches!(last, StoreReply::Install(InstallFrame::Cancelled)),
            "the task ends its own stream: {last:?}"
        );
        assert!(
            !root.join(INSTALL_ID).join(INSTALL_VERSION).exists(),
            "and a cancelled install leaves the box nothing to resolve"
        );
        let summary = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .expect("the registry row is still there");
        assert!(
            summary.on_box.is_none(),
            "and it writes no row: there is nothing to describe"
        );

        runtime.finish_background(Duration::from_secs(5)).await;
    }

    /// Blueprint P-3: `shutdown` trips the token **before** it aborts, because `abort()` cannot
    /// stop the blocking thread an unpack runs on.
    ///
    /// What is observable from here is the half that matters to the next process: after shutdown
    /// the runtime holds no install, nothing was promoted, and no frame was ever sent for a
    /// request nobody is left to read.
    #[tokio::test]
    async fn shutdown_aborts_a_running_install_and_leaves_nothing_resolvable() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true))
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        fixture.route(
            "/archive.zip",
            Route {
                status: 200,
                delay: Duration::from_secs(30),
                ..Route::default()
            },
        );
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let root = tmp.path().join("agents");
        let mut runtime = installing_runtime(&fixture, &root);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let plan = demo_plan(agent_id, root.clone(), fixture.url("/archive.zip"));
        let served = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    7,
                    StoreRequest::InstallConfirm {
                        plan: Box::new(plan),
                    },
                ),
            )
            .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert!(runtime.install_running());

        runtime.shutdown(Duration::ZERO).await;
        assert!(
            !runtime.install_running(),
            "the runtime lets go of the install it just aborted"
        );
        assert!(
            !root.join(INSTALL_ID).exists(),
            "and nothing under the root resolves: {}",
            root.display()
        );

        drop(tx);
        let mut answered = Vec::new();
        while let Some(reply) = rx.recv().await {
            answered.push(reply);
        }
        assert!(
            !answered
                .iter()
                .any(|reply| matches!(reply.reply, StoreReply::Install(InstallFrame::Done(_)))),
            "an aborted install finishes nothing: {answered:?}"
        );
    }

    /// A policy rule answers stages 1–2 of ANA-4 §4.3 without ever reaching the user.
    #[tokio::test]
    async fn a_policy_rule_answers_a_permission_request_and_records_it() {
        let script = Script::one_turn(vec![
            ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
                tool_call_id: "call-1".to_owned(),
                title: "Read".to_owned(),
                tool_kind: ToolKind::Read,
                input: json!({ "path": "src/main.rs" }),
                locations: Vec::new(),
            })),
            ScriptEvent::ParkPermission(PermissionRequestEvent {
                request_id: PermissionRequestId::new("req-1"),
                tool_call_id: Some("call-1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            }),
            ScriptEvent::Emit(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
        ]);
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        let mut row = fake_row(agent_id);
        // Every read is allowed, by rule.
        row.settings = json!({
            "cli": { "stream": "fake", "permission_mode": "ask", "extra_args": [] },
            "permission": {
                "default": "ask",
                "rules": [ { "match": { "tool_kind": "read" }, "answer": "allow_once",
                             "reason": "reads are safe" } ],
                "remembered": []
            }
        });
        store.upsert_agent(&row).await.expect("the row lands");

        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let mut factory = DriverFactory::new();
        factory.register(
            "cli/fake",
            Box::new(FakeBuilder(Arc::clone(&adapter), SpecSlot::default())),
        );
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));

        let (step_id, _replies) = run(&mut runtime, &backend, start(agent_id, "read it")).await;

        let log = store
            .step_events(step_id)
            .await
            .expect("the log reads")
            .expect("a log");
        let answer = log
            .iter()
            .find(|row| row.kind == EventKind::PermissionAnswer)
            .expect("the rule answered the request without asking the user");
        assert_eq!(
            answer.payload.get("by").and_then(Value::as_str),
            Some("policy"),
            "a rule's answer is recorded as policy, not as the user's (ANA-9 §4.3)"
        );
        assert_eq!(
            answer.payload.get("option_id").and_then(Value::as_str),
            Some("allow")
        );
    }

    // -----------------------------------------------------------------------------------------
    // MOD-21: the login fixtures and cases (T6)
    //
    // A real child, because the subject is a runtime that spawns one: the fixture is a `sh`
    // script that speaks just enough JSON-RPC to answer `initialize`, `authenticate` and
    // `logout`, and writes its own pid where a case can read it. Every id, link and "credential"
    // below is made up — `R-AGT-5` says this crate knows no agent's name, no method id and no
    // vendor host, and a worker case that spelled one would be asserting the seeds.
    // -----------------------------------------------------------------------------------------

    #[cfg(unix)]
    pub(crate) mod auth {
        use super::*;
        use htui_agent::acp::Handshake;
        use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo, OpenerCommand};
        use htui_agent::probe::{CredentialTier, ProbeSource};
        use std::path::{Path, PathBuf};

        /// The one method the fixture advertises. A made-up id: the chooser is fed by the agent's
        /// own `initialize`, so a case only ever needs *an* id, never a real one.
        pub(crate) const METHOD: &str = "m-one";

        /// What a "credential" is here: a sentinel string the fixture writes into the file its
        /// row's `discovery.credential.files` names. No frame may ever carry it (`R-SEC-2`).
        pub(crate) const CREDENTIAL: &str = "SECRET-SENTINEL";

        /// The link the fixture prints to its own stderr, carrying a second sentinel in its query.
        ///
        /// The URL itself is a frame the user is meant to see; what the case pins is that the
        /// *credential* never becomes one.
        pub(crate) const LINK: &str = "https://h.invalid/login?state=SENTINEL-TOKEN-VALUE";

        /// How long a case waits on a child before calling the flow stuck rather than slow.
        const PATIENCE: Duration = Duration::from_secs(30);

        /// How long a signalled process is given to stop being one (`tests/acp_driver.rs`'s size).
        const KILL_WINDOW: Duration = Duration::from_secs(5);

        /// The scripted agent, as a shell script: the same protocol as a duplex fixture, a real
        /// pid, and a real environment.
        ///
        /// Behaviour by environment, so one script serves every case. `FIXTURE_DIR` is where the
        /// pid and the credential go; `FIXTURE_INIT` is the `initialize` result on one line;
        /// `FIXTURE_KEY` unset makes `authenticate` refuse the way a real adapter refuses a login
        /// it has no variable for; `FIXTURE_HOLD` makes it never answer; `FIXTURE_URL` is a link
        /// printed to stderr before the answer; `FIXTURE_CRED` is written into the credential file
        /// on success and removed on `logout`.
        ///
        /// The id is echoed back **as it arrived**, quotes and all: this SDK sends a UUID *string*
        /// as its JSON-RPC id, and a fixture that assumed a number would answer with a line the
        /// client's decoder skips — a handshake timeout wearing a costume.
        const AGENT_SH: &str = r#"
echo $$ > "$FIXTURE_DIR/pid"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\|[0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$FIXTURE_INIT" ;;
    *'"method":"authenticate"'*)
      if [ -n "$FIXTURE_URL" ]; then echo "open the following link to log in: $FIXTURE_URL" >&2; fi
      if [ -n "$FIXTURE_HOLD" ]; then sleep 3600; fi
      if [ -z "$FIXTURE_KEY" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"the FIXTURE_KEY variable must be set where this server is launched from"}}\n' "$id"
      else
        if [ -n "$FIXTURE_CRED" ]; then printf '%s\n' "$FIXTURE_CRED" > "$FIXTURE_DIR/credential"; fi
        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      fi ;;
    *'"method":"logout"'*)
      rm -f "$FIXTURE_DIR/credential"
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

        /// Writes `contents` at `path` and makes it executable (`tests/probe.rs`'s helper).
        ///
        /// The agent script is run as `/bin/sh <path>` rather than executed, for that helper's
        /// `ETXTBSY` reason: a `fork` in another test's spawn inherits every fd open at that
        /// instant, and a child holding a write fd makes `execve` refuse until it execs. `sh` only
        /// *reads* the file, so the window never opens.
        fn executable(path: &Path, contents: &str) {
            std::fs::write(path, contents).expect("write");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }

        /// The `initialize` result the fixture answers with, on one line so the script's `case`
        /// globs cannot trip on it.
        fn init_result() -> String {
            serde_json::to_string(&json!({
                "protocolVersion": 1,
                "agentInfo": { "name": "login-fixture", "version": "0.0.0" },
                "agentCapabilities": { "loadSession": false, "auth": { "logout": {} } },
                "authMethods": [
                    { "id": METHOD, "name": "One", "description": "the fixture's only method" }
                ],
            }))
            .expect("one line of JSON")
        }

        /// A registry row whose adapter **is** the fixture script.
        ///
        /// `command: "/bin/sh"` with the script as its argument, and a `discovery` that names no
        /// tool: tier 1 has nothing to resolve, so the row reaches tier 2 (`probe.rs:1444-1448`)
        /// and the re-probe the flow ends with is a real handshake. `credential.files` names the
        /// file the fixture writes, which is what turns a completed call into `ready`.
        pub(crate) fn login_row(
            id: AgentId,
            transport: Transport,
            dir: &Path,
            extra: &[(&str, &str)],
        ) -> Agent {
            let script = dir.join("agent.sh");
            executable(&script, AGENT_SH);
            let mut env = serde_json::Map::new();
            env.insert(
                "FIXTURE_DIR".to_owned(),
                json!(dir.to_string_lossy().into_owned()),
            );
            env.insert("FIXTURE_INIT".to_owned(), json!(init_result()));
            for (name, value) in extra {
                env.insert((*name).to_owned(), json!(*value));
            }
            Agent {
                name: "login-fixture".to_owned(),
                transport,
                launch: json!({
                    "command": "/bin/sh",
                    "args": [script.to_string_lossy().into_owned()],
                    "env": Value::Object(env),
                    "discovery": {
                        "tools": {},
                        "handshake": true,
                        "credential": {
                            "env": [],
                            "files": [dir.join("credential").to_string_lossy().into_owned()],
                        },
                    },
                }),
                settings: json!({}),
                ..fake_row(id)
            }
        }

        /// This box's `agent_box` for `agent_id`: probed, `unauthenticated`, advertising
        /// `auth_methods`.
        ///
        /// The row a login is *for*. `resolved: None`, so the driver resolves the row's own
        /// document rather than a recording, and nothing here is what the flow's own re-probe
        /// will write.
        pub(crate) fn probed_login_box(agent_id: AgentId, auth_methods: Vec<String>) -> AgentBox {
            let now = Utc::now();
            let snapshot = ProbeSnapshot {
                transport: Transport::Acp,
                resolved: None,
                tools: std::collections::BTreeMap::new(),
                handshake: Some(Handshake {
                    at: now,
                    protocol_version: 1,
                    agent_name: Some("login-fixture".to_owned()),
                    agent_version: Some("0.0.0".to_owned()),
                    capabilities: json!({}),
                    auth_methods,
                }),
                credential: Some(CredentialTier::Absent),
                status: ProbeStatus::Unauthenticated,
                stderr_tail: None,
                source: ProbeSource::Probe,
            };
            AgentBox {
                agent_id,
                box_id: ids::BOX,
                enabled: true,
                version: Some("0.0.0".to_owned()),
                path: None,
                probed_at: Some(now),
                quota: None,
                quota_at: None,
                updated_at: now,
                probe: Some(snapshot.to_value()),
            }
        }

        /// The demo store plus one `acp` login row and the `unauthenticated` box row for it.
        pub(crate) async fn login_store(dir: &Path, extra: &[(&str, &str)]) -> (MemStore, AgentId) {
            let store = MemStore::demo();
            let agent_id = AgentId::new();
            store
                .upsert_agent(&login_row(agent_id, Transport::Acp, dir, extra))
                .await
                .expect("the login row lands");
            store
                .upsert_agent_box(&probed_login_box(agent_id, vec![METHOD.to_owned()]))
                .await
                .expect("the box row lands");
            (store, agent_id)
        }

        /// The opener a case injects (blueprint P-5): a script that records the URL it was handed.
        ///
        /// It exits at once rather than sleeping — what "the opener is not waited on" costs is
        /// `htui-agent`'s case to make, and a lingering child here would outlive the tempdir.
        pub(crate) fn recorder(dir: &Path) -> PathBuf {
            let path = dir.join("opener.sh");
            executable(
                &path,
                &format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"{}/opened\"\n",
                    dir.display()
                ),
            );
            path
        }

        /// A runtime over the **real** ACP transport, with the recording opener and no grace.
        ///
        /// The real transport because the subject is a login that spawns a process; a scripted
        /// adapter would answer `Unsupported` and prove nothing.
        pub(crate) fn login_runtime(dir: &Path) -> AgentRuntime {
            AgentRuntime::new(DriverFactory::with_acp())
                .with_grace(Duration::ZERO)
                .with_opener(OpenerCommand::Custom(recorder(dir)))
        }

        /// The fixture's own pid, or `None` when it never ran at all.
        fn fixture_pid(dir: &Path) -> Option<u32> {
            std::fs::read_to_string(dir.join("pid"))
                .ok()
                .and_then(|text| text.trim().parse().ok())
        }

        /// The URLs the injected opener recorded, in order.
        fn opened(dir: &Path) -> Vec<String> {
            std::fs::read_to_string(dir.join("opened"))
                .unwrap_or_default()
                .lines()
                .map(ToOwned::to_owned)
                .collect()
        }

        /// Fails unless `pid` is gone — or reaped-pending — within [`KILL_WINDOW`].
        ///
        /// Linux-only because `/proc` is; copied from `htui-agent`'s `tests/auth.rs` rather than
        /// shared, per the repo's per-file helper rule.
        async fn assert_not_running(pid: u32, what: &str) {
            #[cfg(target_os = "linux")]
            {
                let deadline = std::time::Instant::now() + KILL_WINDOW;
                loop {
                    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                        return;
                    };
                    let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
                    let state = after.trim().chars().next().unwrap_or('Z');
                    if state == 'Z' || state == 'X' {
                        return;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{what}: pid {pid} is still running (state {state})"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (pid, what);
            }
        }

        /// The next reply, or a failure that says the login stopped talking.
        async fn next_reply(rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>) -> ReplyEnvelope {
            tokio::time::timeout(PATIENCE, rx.recv())
                .await
                .expect("a login answers inside the patience window")
                .expect("the reply channel is open")
        }

        /// The next login frame and the `seq` it answered.
        async fn next_frame(rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>) -> (Seq, AuthFrame) {
            let reply = next_reply(rx).await;
            match reply.reply {
                StoreReply::Auth(frame) => (reply.seq, frame),
                other => panic!("a login answers with login frames: {other:?}"),
            }
        }

        /// Whether this frame ends the stream.
        fn is_terminal(frame: &AuthFrame) -> bool {
            matches!(
                frame,
                AuthFrame::Done { .. }
                    | AuthFrame::Refused { .. }
                    | AuthFrame::Cancelled
                    | AuthFrame::Idle { .. }
                    | AuthFrame::Failed { .. }
            )
        }

        /// `AuthStart` at `seq`, and the method list the agent's own `initialize` advertised.
        async fn start_login(
            runtime: &mut AgentRuntime,
            backend: &Backend,
            tx: &mpsc::UnboundedSender<ReplyEnvelope>,
            rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
            seq: Seq,
            agent_id: AgentId,
        ) -> Vec<AuthMethodInfo> {
            let served = runtime
                .serve(
                    backend,
                    tx,
                    &envelope(seq, StoreRequest::AuthStart { agent_id }),
                )
                .await;
            assert!(
                matches!(served, Served::Deferred),
                "a login answers from its own task: {served:?}"
            );
            let (at, frame) = next_frame(rx).await;
            assert_eq!(at, seq, "the method list answers the request that asked");
            match frame {
                AuthFrame::Methods { methods, .. } => methods,
                other => panic!("the first frame of a login is its method list: {other:?}"),
            }
        }

        /// Sends `choice` at `seq` and collects every frame up to and including the terminal one.
        async fn choose_and_collect(
            runtime: &mut AgentRuntime,
            backend: &Backend,
            tx: &mpsc::UnboundedSender<ReplyEnvelope>,
            rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
            seq: Seq,
            choice: AuthChoice,
        ) -> Vec<(Seq, AuthFrame)> {
            let served = runtime
                .serve(
                    backend,
                    tx,
                    &envelope(seq, StoreRequest::AuthChoose { choice }),
                )
                .await;
            assert!(
                matches!(served, Served::Deferred),
                "the choice goes into the running flow: {served:?}"
            );
            let mut frames = Vec::new();
            loop {
                let (at, frame) = next_frame(rx).await;
                let last = is_terminal(&frame);
                frames.push((at, frame));
                if last {
                    return frames;
                }
            }
        }

        /// The terminal frame of a collected stream.
        fn terminal(frames: &[(Seq, AuthFrame)]) -> &AuthFrame {
            &frames.last().expect("a stream has a terminal frame").1
        }

        // -------------------------------------------------------------------------------------
        // Refusals, every one of them before a process exists
        // -------------------------------------------------------------------------------------

        /// D18's first refusal, in the probe's own order: a writer that cannot hold the row
        /// refuses the login **before** an adapter is spawned to produce one.
        #[tokio::test]
        async fn a_start_is_refused_before_any_spawn_on_a_buffered_writer() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let cache = htui_store::CacheStore::open(tmp.path(), "login-buffered", 1)
                .await
                .expect("a fresh mirror");
            let backend = Backend::Offline {
                cache: cache.clone(),
                since: None,
            };
            let mut runtime = login_runtime(tmp.path());
            let (tx, _rx) = mpsc::unbounded_channel();

            let served = runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        1,
                        StoreRequest::AuthStart {
                            agent_id: AgentId::new(),
                        },
                    ),
                )
                .await;
            match served {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "auth_start");
                    assert!(
                        message.contains(htui_store::REGISTRY_ON_SERVER_ONLY),
                        "the buffered writer's own sentence, not a second one: {message}"
                    );
                }
                other => panic!("a buffered writer refuses the login: {other:?}"),
            }
            assert!(!runtime.auth_running(), "and holds no claim afterwards");
            assert_eq!(runtime.background_len(), 0);
            assert_eq!(
                fixture_pid(tmp.path()),
                None,
                "the fixture writes its pid the instant it starts; it never started"
            );

            cache.close().await;
        }

        /// D10's predicate, read off the row before a request is spent: a `cli` row has no
        /// `authenticate` call and the refusal is the seam's own sentence.
        #[tokio::test]
        async fn a_start_on_a_cli_row_is_refused_by_the_predicate() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let store = MemStore::demo();
            let agent_id = AgentId::new();
            store
                .upsert_agent(&login_row(
                    agent_id,
                    Transport::Cli,
                    tmp.path(),
                    &[("FIXTURE_KEY", "set")],
                ))
                .await
                .expect("the row lands");
            store
                .upsert_agent_box(&probed_login_box(agent_id, vec![METHOD.to_owned()]))
                .await
                .expect("the box row lands");
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, _rx) = mpsc::unbounded_channel();

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(1, StoreRequest::AuthStart { agent_id }),
                )
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "auth_start");
                    assert!(
                        message.contains("has no `authenticate` operation"),
                        "the seam's own refusal, not a second wording: {message}"
                    );
                }
                other => panic!("a transport with no login verb refuses: {other:?}"),
            }
            assert_eq!(fixture_pid(tmp.path()), None, "and nothing ran to find out");
        }

        /// D18's last refusal: a box whose stored snapshot advertises no method has nothing to
        /// choose from, and the sentence names the row.
        #[tokio::test]
        async fn a_start_on_a_row_with_no_auth_methods_is_refused_by_name() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let store = MemStore::demo();
            let agent_id = AgentId::new();
            store
                .upsert_agent(&login_row(
                    agent_id,
                    Transport::Acp,
                    tmp.path(),
                    &[("FIXTURE_KEY", "set")],
                ))
                .await
                .expect("the row lands");
            store
                .upsert_agent_box(&probed_login_box(agent_id, Vec::new()))
                .await
                .expect("the box row lands");
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, _rx) = mpsc::unbounded_channel();

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(1, StoreRequest::AuthStart { agent_id }),
                )
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "auth_start");
                    assert!(
                        message.contains("login-fixture")
                            && message.contains("advertises no authentication methods"),
                        "the refusal names the row it is about: {message}"
                    );
                }
                other => panic!("a row with no advertised method refuses: {other:?}"),
            }
            assert_eq!(fixture_pid(tmp.path()), None, "and nothing ran to find out");
        }

        /// D19, side one: a login and an install both write `agent_box`, so a login refuses while
        /// an install holds the claim.
        #[tokio::test]
        async fn a_start_while_an_install_runs_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(tmp.path(), &[("FIXTURE_KEY", "set")]).await;
            let install_id = AgentId::new();
            store
                .upsert_agent(&install_row(install_id, "demo", true))
                .await
                .expect("the install row lands");
            let backend = Backend::memory(store);
            let fixture = Fixture::start().await;
            // The registry read never answers inside this test's lifetime, so the install is
            // provably still holding the claim when the login arrives.
            fixture.route(
                "/registry.json",
                Route {
                    status: 200,
                    delay: Duration::from_secs(30),
                    ..Route::default()
                },
            );
            let mut runtime = login_runtime(tmp.path()).with_installer(InstallConfig::new(
                fixture.base(),
                Some(tmp.path().join("agents")),
            ));
            let (tx, _rx) = mpsc::unbounded_channel();

            let planning = runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        1,
                        StoreRequest::InstallPlan {
                            agent_id: install_id,
                        },
                    ),
                )
                .await;
            assert!(matches!(planning, Served::Deferred), "{planning:?}");

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(2, StoreRequest::AuthStart { agent_id }),
                )
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "auth_start");
                    assert!(
                        message.contains("an install is already running"),
                        "the refusal says what holds the claim: {message}"
                    );
                }
                other => panic!("two writers of one row would race: {other:?}"),
            }
            assert!(!runtime.auth_running());
            assert_eq!(fixture_pid(tmp.path()), None, "and nothing was spawned");

            runtime.shutdown(Duration::ZERO).await;
        }

        /// D19, side two: an install refuses while a login holds the claim.
        #[tokio::test]
        async fn an_install_plan_while_a_login_runs_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let install_id = AgentId::new();
            store
                .upsert_agent(&install_row(install_id, "demo", true))
                .await
                .expect("the install row lands");
            let backend = Backend::memory(store);
            let fixture = Fixture::start().await;
            let mut runtime = login_runtime(tmp.path()).with_installer(InstallConfig::new(
                fixture.base(),
                Some(tmp.path().join("agents")),
            ));
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        2,
                        StoreRequest::InstallPlan {
                            agent_id: install_id,
                        },
                    ),
                )
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "install_plan");
                    assert!(
                        message.contains("a login is already running"),
                        "the refusal says what holds the claim: {message}"
                    );
                }
                other => panic!("an install may not run under a login: {other:?}"),
            }
            assert!(
                fixture.lines().is_empty(),
                "and nothing was asked of the registry: {:?}",
                fixture.lines()
            );

            runtime.shutdown(Duration::from_millis(500)).await;
        }

        /// D19, side three: a probe writes every row, so it refuses while a login holds one.
        #[tokio::test]
        async fn a_probe_while_a_login_runs_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;

            match runtime
                .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeAgents))
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "probe_agents");
                    assert!(
                        message.contains("a login is running"),
                        "the refusal says what to wait for: {message}"
                    );
                }
                other => panic!("a probe may not run under a login: {other:?}"),
            }
            assert_eq!(runtime.background_len(), 0, "and it spawned nothing");

            runtime.shutdown(Duration::from_millis(500)).await;
        }

        /// One login at a time: the claim is one deep, like the install's.
        #[tokio::test]
        async fn a_second_start_while_one_runs_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(2, StoreRequest::AuthStart { agent_id }),
                )
                .await
            {
                Served::Reply(StoreReply::Failed { request, message }) => {
                    assert_eq!(request, "auth_start");
                    assert!(
                        message.contains("a login is already running"),
                        "the refusal names what holds the claim: {message}"
                    );
                }
                other => panic!("two logins would be two children: {other:?}"),
            }

            runtime.shutdown(Duration::from_millis(500)).await;
        }

        /// A choice with nothing to choose for is a refusal of the *request*, not a frame of a
        /// stream that does not exist (blueprint H-22).
        #[tokio::test]
        async fn a_choose_with_no_flow_pending_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, _agent_id) = login_store(tmp.path(), &[("FIXTURE_KEY", "set")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, _rx) = mpsc::unbounded_channel();

            for (request, name) in [
                (
                    StoreRequest::AuthChoose {
                        choice: AuthChoice::Method(METHOD.to_owned()),
                    },
                    "auth_choose",
                ),
                (
                    StoreRequest::AuthOpen {
                        url: LINK.to_owned(),
                    },
                    "auth_open",
                ),
                (StoreRequest::AuthCancel, "auth_cancel"),
            ] {
                match runtime.serve(&backend, &tx, &envelope(1, request)).await {
                    Served::Reply(StoreReply::Failed { request, message }) => {
                        assert_eq!(request, name);
                        assert_eq!(message, "no login is running");
                    }
                    other => panic!("`{name}` with no flow is refused by name: {other:?}"),
                }
            }
            assert_eq!(fixture_pid(tmp.path()), None, "and nothing ran");
        }

        // -------------------------------------------------------------------------------------
        // The stream
        // -------------------------------------------------------------------------------------

        /// D18's address switch: the method list answers the `AuthStart`, and every frame after
        /// the choice answers the `AuthChoose` (`App::is_fresh` keys on request kind).
        #[tokio::test]
        async fn methods_arrive_at_the_start_seq_and_the_rest_at_the_choose_seq() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_URL", LINK)]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            let methods = start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            assert_eq!(
                methods,
                vec![AuthMethodInfo {
                    id: METHOD.to_owned(),
                    name: "One".to_owned(),
                    description: Some("the fixture's only method".to_owned()),
                }],
                "the chooser is fed by the agent's own `initialize`"
            );

            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;
            assert!(
                frames.iter().all(|(seq, _)| *seq == 2),
                "every frame after the choice carries the choice's own seq: {frames:?}"
            );
            assert!(
                frames.iter().any(|(_, frame)| matches!(
                    frame,
                    AuthFrame::Url(url) if url == LINK
                )),
                "the link the adapter printed reaches the pane: {frames:?}"
            );
            assert!(
                matches!(terminal(&frames), AuthFrame::Done { .. }),
                "and the stream ends with the probe's verdict: {frames:?}"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// D6 and `R-AGT-6`: the probe is the sole authority, and the row is written **before**
        /// the terminal frame — the section reads the registry the instant it sees `Done`.
        #[tokio::test]
        async fn done_carries_the_probes_status_and_the_row_was_written_first() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(
                tmp.path(),
                &[("FIXTURE_KEY", "set"), ("FIXTURE_CRED", CREDENTIAL)],
            )
            .await;
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;

            match terminal(&frames) {
                AuthFrame::Done { call, status } => {
                    assert_eq!(*call, AuthCall::Authenticate(METHOD.to_owned()));
                    assert_eq!(
                        *status,
                        ProbeStatus::Ready,
                        "the credential the agent left is what the probe found: {frames:?}"
                    );
                }
                other => panic!("a completed call ends with the probe's verdict: {other:?}"),
            }

            // Read the instant the terminal frame is in hand: this is what the section does.
            let on_box = stored_box(&store, agent_id).await;
            assert_eq!(
                probe_status(&on_box),
                Some("ready"),
                "the row was written before `Done`: {:?}",
                on_box.probe
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// The PRD's own metric: a call that succeeded and left nothing behind is still
        /// `unauthenticated`, because the flow never decides the status.
        #[tokio::test]
        async fn success_into_a_box_with_no_credential_still_reads_unauthenticated() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(tmp.path(), &[("FIXTURE_KEY", "set")]).await;
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;

            match terminal(&frames) {
                AuthFrame::Done { status, .. } => assert_eq!(
                    *status,
                    ProbeStatus::Unauthenticated,
                    "the agent said yes and the box says otherwise: {frames:?}"
                ),
                other => panic!("the call returned, so the stream ends `Done`: {other:?}"),
            }
            assert_eq!(
                probe_status(&stored_box(&store, agent_id).await),
                Some("unauthenticated")
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// Hazard H-17 from the other side: `logout` is the same spawn and the same stream, and
        /// the probe reads the box the vendor's own call left behind.
        #[tokio::test]
        async fn logout_reprobes_and_reads_unauthenticated() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(tmp.path(), &[("FIXTURE_KEY", "set")]).await;
            std::fs::write(tmp.path().join("credential"), CREDENTIAL).expect("a logged-in box");
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let frames =
                choose_and_collect(&mut runtime, &backend, &tx, &mut rx, 2, AuthChoice::Logout)
                    .await;

            match terminal(&frames) {
                AuthFrame::Done { call, status } => {
                    assert_eq!(*call, AuthCall::Logout);
                    assert_eq!(*status, ProbeStatus::Unauthenticated);
                }
                other => panic!("a logout ends `Done` like any other call: {other:?}"),
            }
            assert!(
                !tmp.path().join("credential").exists(),
                "the agent removed its own credential; `htui` never touched it"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// D6: a `Refused` writes **nothing**. The row before and after is the same row.
        #[tokio::test]
        async fn a_refusal_writes_nothing_and_keeps_probed_at() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            // No `FIXTURE_KEY`: the fixture refuses `authenticate` the way an adapter refuses a
            // login it has no variable for, in its own words.
            let (store, agent_id) = login_store(tmp.path(), &[]).await;
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            let before = stored_box(&store, agent_id).await;
            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;

            match terminal(&frames) {
                AuthFrame::Refused { message } => assert!(
                    message.contains("FIXTURE_KEY"),
                    "the agent's own sentence, verbatim: {message}"
                ),
                other => panic!("a JSON-RPC error to the call is a refusal: {other:?}"),
            }

            let after = stored_box(&store, agent_id).await;
            assert_eq!(
                serde_json::to_value(&before).expect("a row serialises"),
                serde_json::to_value(&after).expect("a row serialises"),
                "a refusal re-probes nothing, so `probed_at` never moves"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// `R-AGT-9`: a login is a fact about a **box**. Nothing here writes `agent`.
        #[tokio::test]
        async fn the_agent_row_is_byte_identical_before_and_after_a_login() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(
                tmp.path(),
                &[("FIXTURE_KEY", "set"), ("FIXTURE_CRED", CREDENTIAL)],
            )
            .await;
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            let row_of = async |store: &MemStore| -> Vec<u8> {
                let summary = store
                    .agents()
                    .await
                    .expect("the memory store never fails")
                    .into_iter()
                    .find(|summary| summary.agent.id == agent_id)
                    .expect("the registry row is there");
                serde_json::to_vec(&summary.agent).expect("a row serialises")
            };
            let before = row_of(&store).await;

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;
            assert!(
                matches!(terminal(&frames), AuthFrame::Done { .. }),
                "the login ran to its end: {frames:?}"
            );

            assert_eq!(
                before,
                row_of(&store).await,
                "the outcome reaches `agent_box` through the probe and `agent` not at all"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// `R-SEC-2` / `R-ID-7`: no frame, and no `Debug` of the runtime's own state, may carry a
        /// credential value.
        ///
        /// The URL is allowed — the user is looking at it — and the sentinel *inside* the token
        /// file is what is asserted absent.
        #[tokio::test]
        async fn no_frame_carries_anything_but_ids_text_and_status() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_URL", LINK),
                    ("FIXTURE_CRED", CREDENTIAL),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let live = format!("{:?}", runtime.auth);
            assert!(
                !live.contains(CREDENTIAL) && !live.contains("SENTINEL-TOKEN-VALUE"),
                "a live login prints ids and finishedness, never a line or a link: {live}"
            );

            let frames = choose_and_collect(
                &mut runtime,
                &backend,
                &tx,
                &mut rx,
                2,
                AuthChoice::Method(METHOD.to_owned()),
            )
            .await;
            for (_, frame) in &frames {
                let rendered = format!("{frame:?}");
                assert!(
                    !rendered.contains(CREDENTIAL),
                    "a frame may carry ids, text and a status, never a credential: {rendered}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(tmp.path().join("credential"))
                    .expect("the fixture wrote its credential")
                    .trim(),
                CREDENTIAL,
                "and the value the assertion is about really was on this box"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// D17 through the runtime: `o` is forwarded into the live flow and answered at **its**
        /// own `seq`, so the pane's link stays on screen.
        #[tokio::test]
        async fn open_is_forwarded_to_the_live_flow_and_answered_at_its_own_seq() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", LINK),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let served = runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        2,
                        StoreRequest::AuthChoose {
                            choice: AuthChoice::Method(METHOD.to_owned()),
                        },
                    ),
                )
                .await;
            assert!(matches!(served, Served::Deferred), "{served:?}");

            // The link arrives on the adapter's stderr; `o` is only offered once it has.
            let url = loop {
                match next_frame(&mut rx).await {
                    (seq, AuthFrame::Url(url)) => {
                        assert_eq!(seq, 2, "the link answers the choice, not the start");
                        break url;
                    }
                    (_, other) => assert!(
                        matches!(other, AuthFrame::Line(_)),
                        "a held flow says nothing but lines until it is opened: {other:?}"
                    ),
                }
            };
            assert_eq!(url, LINK);

            let served = runtime
                .serve(&backend, &tx, &envelope(3, StoreRequest::AuthOpen { url }))
                .await;
            assert!(
                matches!(served, Served::Deferred),
                "the opener runs in the flow's own task: {served:?}"
            );
            let (seq, frame) = next_frame(&mut rx).await;
            assert_eq!(seq, 3, "`o` is answered at its own address");
            assert!(
                matches!(frame, AuthFrame::Opened),
                "the opener was spawned: {frame:?}"
            );

            // The recorder exits at once; give it the moment the kernel needs.
            for _ in 0..200u32 {
                if !opened(tmp.path()).is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(
                opened(tmp.path()),
                vec![LINK.to_owned()],
                "the URL travels as the opener's one argument, unmangled"
            );

            runtime.shutdown(Duration::from_millis(500)).await;
        }

        // -------------------------------------------------------------------------------------
        // Ends
        // -------------------------------------------------------------------------------------

        /// D18's cancellation: acknowledged at its own `seq`, the stream ends itself at the
        /// choice's, and the child is gone.
        #[tokio::test]
        async fn cancel_answers_cancelling_then_the_stream_ends_cancelled_and_no_child_survives() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store.clone());
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            let before = stored_box(&store, agent_id).await;
            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let pid = fixture_pid(tmp.path()).expect("the fixture wrote its pid when it started");

            let served = runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        2,
                        StoreRequest::AuthChoose {
                            choice: AuthChoice::Method(METHOD.to_owned()),
                        },
                    ),
                )
                .await;
            assert!(matches!(served, Served::Deferred), "{served:?}");

            let cancelling = runtime
                .serve(&backend, &tx, &envelope(3, StoreRequest::AuthCancel))
                .await;
            assert!(
                matches!(
                    cancelling,
                    Served::Reply(StoreReply::Auth(AuthFrame::Cancelling))
                ),
                "a cancel is answered at once, not when the flow notices: {cancelling:?}"
            );

            let last = loop {
                let (seq, frame) = next_frame(&mut rx).await;
                if is_terminal(&frame) {
                    break (seq, frame);
                }
            };
            assert_eq!(last.0, 2, "the stream keeps the choice's address");
            assert!(
                matches!(last.1, AuthFrame::Cancelled),
                "the task ends its own stream: {:?}",
                last.1
            );
            assert_not_running(pid, "a cancelled login").await;
            assert_eq!(
                serde_json::to_value(&before).expect("a row serialises"),
                serde_json::to_value(&stored_box(&store, agent_id).await)
                    .expect("a row serialises"),
                "and a cancelled login leaves `agent_box` exactly as it found it"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// The shutdown path: the token first, the handle after, and no child left behind.
        #[tokio::test]
        async fn shutdown_cancels_then_aborts_a_running_login_and_no_child_survives() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let pid = fixture_pid(tmp.path()).expect("the fixture wrote its pid when it started");
            assert!(runtime.auth_running());

            runtime.shutdown(Duration::from_millis(500)).await;
            assert!(
                !runtime.auth_running(),
                "the runtime lets go of the login it just ended"
            );
            assert_not_running(pid, "a login the runtime shut down").await;
        }

        /// D19's second claim (hazard H-16): a login holds the row's re-probe claim for its whole
        /// life, so a chat started during the browser round trip cannot re-probe under it.
        #[tokio::test]
        async fn a_flow_holds_the_reprobe_claim_for_its_row() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            assert!(
                runtime.reprobe_claims.claim((agent_id, ids::BOX)).is_none(),
                "a staleness re-probe of this row waits for the login"
            );
            assert!(
                runtime
                    .reprobe_claims
                    .claim((AgentId::new(), ids::BOX))
                    .is_some(),
                "another row on this box is not excluded"
            );

            runtime.shutdown(Duration::from_millis(500)).await;
            assert!(
                runtime.reprobe_claims.claim((agent_id, ids::BOX)).is_some(),
                "and the claim goes with the flow"
            );
        }

        /// Review M-1: a runtime that lets go of a login **without** cancelling it.
        ///
        /// A panic on the worker loop, or any drop of [`AgentRuntime`] that never reaches
        /// `shutdown`, drops the [`LiveAuth`] — which closes the command channel and *drops* the
        /// token clone rather than tripping it. From that moment nobody can choose, open or cancel
        /// this flow: its `AuthCancel` has no runtime to reach. Bounded only by the idle clock it
        /// would keep an adapter — and its OAuth loopback listener — alive for the whole cap, and
        /// an adapter that prints any periodic line resets that clock forever (`auth/run.rs`).
        ///
        /// So a closed command channel is read as what it is: the last caller is gone.
        #[tokio::test]
        async fn a_login_whose_runtime_let_go_of_it_cancels_itself_rather_than_waiting() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let pid = fixture_pid(tmp.path()).expect("the fixture wrote its pid when it started");

            // The drop a panicking loop leaves behind: the task handle is detached rather than
            // aborted, the sender goes, and the token is dropped **uncancelled**.
            drop(runtime.auth.take().expect("the login is live"));

            let last = loop {
                let (_, frame) = next_frame(&mut rx).await;
                if is_terminal(&frame) {
                    break frame;
                }
            };
            assert!(
                matches!(last, AuthFrame::Cancelled),
                "a flow no caller can reach again ends itself: {last:?}"
            );
            assert_not_running(pid, "a login whose runtime let go of it").await;
        }

        /// A driver whose `authenticate` is the seam's default: it refuses, and it refuses on the
        /// **first poll**.
        ///
        /// That is what makes review L-1 a deterministic case rather than a race: the loop's
        /// `select!` is `biased` towards the flow, so a command queued before the task starts is
        /// still in the channel when the loop breaks, and whether it is ever answered is then a
        /// question about the drain and not about scheduling.
        #[derive(Debug)]
        struct RefusingDriver;

        impl AgentDriver for RefusingDriver {
            fn name(&self) -> &str {
                "refusing-fixture"
            }

            fn caps(&self) -> DriverCaps {
                DriverCaps::default()
            }

            fn start<'a>(
                &'a self,
                _spec: SessionSpec,
                _prompt: String,
            ) -> htui_agent::driver::DriverFuture<'a, Box<dyn AgentSession>> {
                Box::pin(async { Err(DriverError::Unsupported("start")) })
            }
        }

        /// Review L-1: a command deferred into the window between the loop and the last frame is
        /// answered at its own address, not dropped.
        ///
        /// `auth_command` sees a live receiver for as long as the task holds one — through the
        /// event drain, the re-probe and the row write — and answers [`Served::Deferred`]. A
        /// command that then goes down with the task is the "deferred with no frame" shape MOD-20's
        /// own review rejected: the pane spent a request and hears nothing back, ever.
        #[tokio::test]
        async fn a_command_still_queued_when_the_flow_ends_is_refused_rather_than_dropped() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let backend = Backend::memory(MemStore::demo());
            let writer = recording_writer(&backend).expect("a memory backend hands out a writer");
            let origin = Origin::Tab(crate::ui::tabs::TabId("settings"));
            let (commands_tx, commands_rx) = mpsc::unbounded_channel();
            let (tx, mut rx) = mpsc::unbounded_channel();

            // Queued before the flow is polled even once, and never read by the loop.
            commands_tx
                .send(AuthCommand::Open {
                    url: LINK.to_owned(),
                    reply: ReplyAddr {
                        seq: 3,
                        origin: origin.clone(),
                    },
                })
                .expect("the task holds the receiver");

            run_auth(AuthArgs {
                driver: Box::new(RefusingDriver),
                agent: login_row(AgentId::new(), Transport::Acp, tmp.path(), &[]),
                existing: None,
                box_id: ids::BOX,
                writer,
                cwd: tmp.path().to_path_buf(),
                cancel: CancellationToken::new(),
                commands: commands_rx,
                opener: OpenerCommand::Custom(recorder(tmp.path())),
                frames: Frames {
                    tx,
                    addr: ReplyAddr {
                        seq: 2,
                        origin: origin.clone(),
                    },
                },
            })
            .await;

            let mut replies = Vec::new();
            while let Some(reply) = rx.recv().await {
                replies.push((reply.seq, reply.reply));
            }
            assert!(
                replies.iter().any(|(seq, reply)| *seq == 3
                    && matches!(
                        reply,
                        StoreReply::Failed { request, message }
                            if *request == "auth_open" && message == "this login has ended"
                    )),
                "the queued command is answered at its own address: {replies:?}"
            );
            assert!(
                replies.iter().any(|(seq, reply)| *seq == 2
                    && matches!(reply, StoreReply::Auth(AuthFrame::Failed { .. }))),
                "and the stream still ends with the flow's own last frame: {replies:?}"
            );
            assert!(
                opened(tmp.path()).is_empty(),
                "a login that has ended opens nothing: {:?}",
                opened(tmp.path())
            );
        }
    }
}
