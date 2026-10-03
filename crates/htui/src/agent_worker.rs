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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use htui_agent::acp::SESSION_STARTED;
use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, DeliverError, DeliverLimits, ListenerReply,
    NO_LOOPBACK_REDIRECT, RedirectUrl,
};
use htui_agent::auth::{
    AUTH_IDLE_CAP, AuthChoice, AuthEvent, AuthFlow, AuthOutcome, BrowserPolicy, OpenerCommand,
    open_url,
};
use htui_agent::box_probe;
use htui_agent::box_probe::hardware::{HardwareSource, SystemHardware};
use htui_agent::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, PermissionAnswer, PermissionPolicy,
    PermissionRequestId, SessionSpec,
};
use htui_agent::error::DriverError;
use htui_agent::event::{DriverEnvelope, DriverEvent, OtherEvent, StopReason, ToolCallEvent};
use htui_agent::install::{
    InstallConfig, InstallError, InstallJob, InstallOutcome, InstallPlan, InstallProgress,
    Installer, PlanError, install, plan as plan_install,
};
use htui_agent::launch::{AgentLaunch, AgentSettings};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeStatus, SpawnTier2, agent_box_row,
    probe_agent, probe_snapshot,
};
use htui_agent::record::{
    AnsweredBy, CapBreach, QuotaLatch, Recorder, RunCap, enforce_breach as enforce_cap_breach,
};
use htui_agent::registry::{DriverFactory, caps_for};
use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, ChatRunSpec, ItemId, PER_TOKEN_CAP_BATCH, ProjectCaps,
    ProjectId, QuotaSource, RunStatus, Scope, SessionEvent, StepId, StepOpening, Transport,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{ReadStore as _, StoreError, WriteStore};
use htui_orch::OpeningPath;
use htui_store::{Backend, Writer};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::agent_settings::{AgentWrite, LITERAL_LAUNCH, SET_TOOL_PATHS, parse_tool_path};
use crate::store_worker::{
    AuthFrame, ChatFrame, InstallFrame, Origin, ReplyEnvelope, RequestEnvelope, Seq, StoreReply,
    StoreRequest, UNSOLICITED,
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

/// What `claim_is_free`, `probe` and `ProbeBox` refuse with while a box probe holds the slot
/// (MOD-7 blueprint D27).
pub const BOX_PROBE_RUNNING: &str =
    "a box probe is running on this box; try again once it has finished";

/// What `claim_is_free` refuses with while a probe or a chat's staleness re-probe writes this
/// box. Verb-neutral, because every claim-taking request reads it: an install, a login, a
/// `ProbeBox` and a `SetToolPaths` (MOD-66 review L4).
const BOX_WRITE_RUNNING: &str =
    "a probe or a re-probe is already writing this box; try again once it has finished";

/// What one box probe did (MOD-7 D13, blueprint D25): the only thing the box probe task ever
/// sends.
///
/// A failure is a field here rather than a [`StoreReply::Failed`], because the registration probe
/// answers at [`UNSOLICITED`] and the shell surfaces a `Failed` only below its freshness gate,
/// which drops that address (blueprint F-D). `observe_reply` sits above the gate and renders
/// [`status_line`](Self::status_line) whatever the address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxProbeReport {
    /// `box_tool` rows written.
    pub tools: usize,
    /// `box.probed_tags` written.
    pub probed_tags: Vec<String>,
    /// Enabled agents whose fresh row is `missing` and whose launch declares `discovery.install`.
    pub installable: Vec<String>,
    /// The agent half's failure, when it had one.
    pub agents_failed: Option<String>,
    /// `EffectiveSpec::error`: why a stored `box_probe_spec` was ignored.
    pub spec_error: Option<String>,
    /// The box half's failure (no box, a read or the write); nothing was written.
    pub box_failed: Option<String>,
    /// This box was left alone, already probed by this `htui` under this spec, and the report
    /// exists only to say [`spec_error`](Self::spec_error): every other field is empty.
    pub unchanged: bool,
}

impl BoxProbeReport {
    /// The status-line sentence (blueprint D25): the head, then the install offer, the agent
    /// half's failure and the ignored spec, each only when there is one. An
    /// [`unchanged`](Self::unchanged) report probed nothing, so it says only that and the
    /// ignored spec.
    #[must_use]
    pub fn status_line(&self) -> String {
        use core::fmt::Write as _;

        if self.unchanged {
            return match &self.spec_error {
                Some(error) => format!("box probe unchanged · {error}"),
                None => "box probe unchanged".to_owned(),
            };
        }
        let mut line = match &self.box_failed {
            Some(message) => format!("box probe failed: {message}"),
            None if self.probed_tags.is_empty() => {
                format!("box probed: {} tools · no tags", self.tools)
            }
            None => format!(
                "box probed: {} tools · tags {}",
                self.tools,
                self.probed_tags.join(", ")
            ),
        };
        if !self.installable.is_empty() {
            let verb = if self.installable.len() == 1 {
                "is"
            } else {
                "are"
            };
            let _ = write!(
                line,
                " · {} {verb} missing and can be installed: Settings > Agents, i",
                self.installable.join(", ")
            );
        }
        if let Some(message) = &self.agents_failed {
            let _ = write!(line, " · agent probe failed: {message}");
        }
        if let Some(error) = &self.spec_error {
            let _ = write!(line, " · {error}");
        }
        line
    }
}

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
    /// Where the chat's frames go, for [`StoreRequest::ChatFollow`] and a refused promotion to
    /// move (blueprint D185). The address alone, not a sender: a runtime holding a reply sender
    /// per chat would keep the reply channel open after every session had ended.
    stream: Stream,
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

/// What a request into a login is told when no login is held at all (`auth_command`,
/// `auth_cancel`).
///
/// Public for [`LOGIN_ENDED`]'s reason (review L-3): the settings section reads a refused
/// `auth_deliver` carrying either as "the flow is gone", and a sentence the compiler links is not
/// one either side can reword alone. The tests still pin the literal text.
pub const NO_LOGIN_RUNNING: &str = "no login is running";

/// What every request a finished login can no longer serve is told (MOD-22 D287): a command
/// `auth_command` could not send, one still queued when the flow ended, and a delivery the flow's
/// end overtook.
///
/// Public for [`AUTH_ALREADY_CHOSEN`]'s reason: the settings section matches on this exact value
/// (a refused `auth_deliver` carrying it means the flow is gone, not that one delivery was
/// answered no), and a sentence the compiler links is not one either side can reword alone.
pub const LOGIN_ENDED: &str = "this login has ended";

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
    /// Relay a pasted redirect to the flow's own loopback listener (MOD-22 D269).
    ///
    /// Validated again here against the flow's own record of the advertised redirect, never the
    /// pane's. The address is a credential for the length of the one `GET` (the credential rule
    /// of `htui_agent::auth::loopback`), and [`RedirectUrl`]'s `Debug` keeps this enum's derive
    /// safe.
    Deliver {
        /// The pasted address.
        url: RedirectUrl,
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
    /// chat command, the probe task for [`StoreRequest::ProbeAgents`], the tool-paths task for
    /// [`StoreRequest::SetToolPaths`] (MOD-66 D7).
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
    /// A [`StoreRequest::ChatFollow`] served before the bind opened its chat (T7): the Chat tab
    /// follows when its promotion answers, which can be before `bind_promoted` has run. One slot,
    /// so memory is bounded; `bind_promoted` takes it on every path, so none outlives a promotion.
    pending_follow: Option<(StepId, ReplyAddr)>,
    started: Vec<StepId>,
    grace: Duration,
    /// Tasks this runtime spawned that answer a request of their own: today the probe's (MOD-2
    /// D53), a chat's staleness re-probe's (plan D55), the prompt preview's (plan D102) and a
    /// `SetToolPaths` write's (MOD-66 D8, tagged [`Writes::ToolPaths`]). Swept
    /// when finished, awaited by [`finish_background`](Self::finish_background), aborted by
    /// [`shutdown`](Self::shutdown).
    ///
    /// The runtime owns the handle for the same reason it owns a chat's: a bare `tokio::spawn`
    /// inside the worker loop would leave a probe with a 60-second handshake running after the UI
    /// is gone, with nobody able to name it.
    ///
    /// One collection of two kinds of task: some **write `agent_box`**, some only read. The split
    /// is [`Background`]'s tag, stated once at each push site, and
    /// [`claim_is_free`](Self::claim_is_free) consults the writing half — a preview holds no claim,
    /// so holding `j` in the Backlog detail no longer refuses `i` in Settings (MOD-31 D5).
    /// [`background_len`](Self::background_len) counts them all; the half is
    /// [`writing_background_len`](Self::writing_background_len).
    background: Vec<Background>,
    /// The rows a `ChatStart` re-probe is running for, so two overlapping chats do not each start
    /// one for the same `(agent_id, box_id)` (blueprint H-9).
    ///
    /// On the runtime because that is the only thing both triggers outlive: the staleness re-probe
    /// is a task this struct owns, the D60 one runs inside a chat task, and neither can see the
    /// other from where it lives.
    reprobe_claims: ReprobeClaims,
    /// The in-flight [`StoreRequest::PromptPreview`] task per asking [`Origin`], so the next one
    /// from that origin can cancel it (review finding M3).
    ///
    /// Keyed on the origin rather than held one deep, because two origins can each be waiting on a
    /// preview of their own and neither supersedes the other. An entry names a task that is also in
    /// [`background`](Self::background) — this map owns the *right to cancel*, not the task.
    ///
    /// Without it, holding `j` across thirty rows spawned thirty tasks, each doing eight store
    /// reads against an eight-connection pool and assembling to the token budget. The shell's
    /// staleness index dropped twenty-nine of the replies, but only *after* their reads were paid
    /// for, and the worker's own `Item`/`Runs` reads queued behind them on `acquire`. Nothing was
    /// ever blocked (`R-NF-3` held throughout); the UI simply waited on Postgres for work whose
    /// answer was already known to be unwanted.
    previews: HashMap<Origin, AbortHandle>,
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
    /// What a delivered paste may wait for (MOD-22 D267, D278).
    ///
    /// [`DeliverLimits::default`] in production; a login case injects milliseconds through
    /// [`with_deliver_limits`](Self::with_deliver_limits).
    deliver_limits: DeliverLimits,
    /// The box probe running right now, if any (MOD-7 D11, blueprint D27).
    ///
    /// One deep, and it holds the same claim as the install, the login and the agent probe: it
    /// ends by probing every agent row on this box, which writes `agent_box`. Swept when finished,
    /// awaited by [`finish_background`](Self::finish_background), aborted by
    /// [`shutdown`](Self::shutdown); its version children die with their `ChildGuard`s.
    box_probe: Option<JoinHandle<()>>,
    /// Whether [`on_online`](Self::on_online) probes at all (MOD-7 D11). Only the binary's entry
    /// point opts in, through [`with_registration_probe`](Self::with_registration_probe): a test
    /// or harness that drives an `Online` swap never probes the maintainer's box by accident.
    registration_probe: bool,
    /// The env every box probe and `ProbeAgents` resolve through when a test injected one
    /// (blueprint D35); `None` is [`ProbeEnv::host`] over the working directory.
    probe_env: Option<ProbeEnv>,
    /// The hardware seam the box probe reads when a test injected one; `None` is
    /// [`SystemHardware::host`].
    hardware: Option<Arc<dyn HardwareSource>>,
}

impl core::fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("adapters", &self.factory.adapter_ids())
            .field("live", &self.live.len())
            .field("box_probe", &self.box_probe.is_some())
            .finish()
    }
}

/// One task this runtime owns in [`background`](AgentRuntime::background), and what it writes (MOD-31 D1).
///
/// The collection mixes two kinds of work and the guard consults only one of them: a probe and a
/// chat's staleness re-probe both end by writing `agent_box`, and a prompt preview **reaches no
/// write method** (plan D102) — it reads a dozen tables, walks the filesystem and records nothing.
/// So a preview held in the Backlog detail used to refuse an install, a login, a `ProbeBox` and a
/// connect's registration probe, with a sentence about a probe that was not running.
///
/// The tag is a promise and the constructor is where it is made. There is no bool field and no
/// other way in, so a fifth push site has to name what its task does rather than pass a guess —
/// and a reviewer greps for `Background::reading` and checks each against the task it names.
struct Background {
    /// The task. Reached only through [`task`](Self::task) and [`into_task`](Self::into_task), so
    /// nothing outside this module can take the handle and drop the tag.
    task: JoinHandle<()>,
    /// What the task writes, decided once, at the push site that knows the spawned future.
    writes: Writes,
}

/// What a background task writes, and therefore whether it holds the install claim (MOD-31 D1).
///
/// An enum rather than a bool for [`LivePhase`](LivePhase)'s reason: the name is the claim, and
/// `reads: true` at a push site would be a maintainer's guess with nothing to grep for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Writes {
    /// The task ends by writing `agent_box` for at least one row.
    AgentBox,
    /// The task reaches no write method: it reads, and answers on the reply channel.
    Nothing,
    /// A `SetToolPaths` task for this agent (MOD-66 D8): it writes `agent_box`, so it holds the
    /// claim exactly as [`Writes::AgentBox`] does, **and** `ProbeAgents` refuses beside it. The
    /// agent is named so the refusal can say whose write to wait for.
    ToolPaths(AgentId),
}

impl Background {
    /// A task that **writes `agent_box`** for at least one row, so
    /// [`claim_is_free`](AgentRuntime::claim_is_free) must see it.
    ///
    /// The promise: a task pushed here is one an install's re-probe would race on the same row
    /// (hazard H-10, MOD-21 D19), and the guard is entitled to refuse the install beside it. A task
    /// that writes nothing, or that writes some other table, does not belong here.
    fn writing(task: JoinHandle<()>) -> Self {
        Self {
            task,
            writes: Writes::AgentBox,
        }
    }

    /// A task that **reaches no write method** (plan D102's "the preview writes nothing").
    ///
    /// The promise: the task records nothing, so it holds no claim and leaves no row for an
    /// install to race. Promoting the preview to [`writing`](Self::writing) the day `run_preview`
    /// grows a write is one word at one call site — which is the whole reason the tag lives here
    /// and not in the guard.
    fn reading(task: JoinHandle<()>) -> Self {
        Self {
            task,
            writes: Writes::Nothing,
        }
    }

    /// A `SetToolPaths` task (MOD-66 D8, B4). In `background`, not in a slot of its own, so the
    /// sweep, [`finish_background`](AgentRuntime::finish_background),
    /// [`shutdown`](AgentRuntime::shutdown) and `writing_background_len` all cover it unchanged.
    fn writing_tool_paths(task: JoinHandle<()>, agent_id: AgentId) -> Self {
        Self {
            task,
            writes: Writes::ToolPaths(agent_id),
        }
    }

    /// The agent of a running `SetToolPaths`, or `None` for every other task.
    fn tool_paths(&self) -> Option<AgentId> {
        match self.writes {
            Writes::ToolPaths(agent_id) => Some(agent_id),
            Writes::AgentBox | Writes::Nothing => None,
        }
    }

    /// The task, borrowed: for the sweep, which looks at every entry and keeps most of them.
    fn task(&self) -> &JoinHandle<()> {
        &self.task
    }

    /// The task, by move: for the wait and the abort, which consume the entry.
    fn into_task(self) -> JoinHandle<()> {
        self.task
    }

    /// Whether this task writes `agent_box`, which is the one question
    /// [`claim_is_free`](AgentRuntime::claim_is_free) asks of the collection.
    fn writes_agent_box(&self) -> bool {
        matches!(self.writes, Writes::AgentBox | Writes::ToolPaths(_))
    }
}

/// What `claim_is_free` and `ProbeAgents` refuse with while a `SetToolPaths` write runs (MOD-66
/// B5): its own sentence, naming the write and the row, where the generic
/// [`BOX_WRITE_RUNNING`] names a probe (F-7).
fn tool_paths_running(agent_id: AgentId) -> String {
    format!("a tool-paths write is running for agent {agent_id}; try again once it has finished")
}

impl AgentRuntime {
    /// A runtime over a transport registry.
    #[must_use]
    pub fn new(factory: DriverFactory) -> Self {
        Self {
            factory,
            live: HashMap::new(),
            pending_follow: None,
            started: Vec::new(),
            grace: CANCEL_GRACE,
            background: Vec::new(),
            reprobe_claims: ReprobeClaims::default(),
            previews: HashMap::new(),
            installer: None,
            install: None,
            auth: None,
            opener: OpenerCommand::Platform,
            deliver_limits: DeliverLimits::default(),
            box_probe: None,
            registration_probe: false,
            probe_env: None,
            hardware: None,
        }
    }

    /// Opts in to the registration probe (MOD-7 D11): only the binary's entry point does.
    #[must_use]
    pub fn with_registration_probe(mut self) -> Self {
        self.registration_probe = true;
        self
    }

    /// The env and hardware every box probe and `ProbeAgents` use (MOD-7 D11, blueprint D35).
    ///
    /// The injected seam H-7 needs: a test hands in a fake `PATH` and fixed facts, so no case
    /// spawns a real host tool or reads this box's hardware.
    #[must_use]
    pub fn with_probe_env(mut self, env: ProbeEnv, hardware: Arc<dyn HardwareSource>) -> Self {
        self.probe_env = Some(env);
        self.hardware = Some(hardware);
        self
    }

    /// Whether a box probe is still running.
    #[must_use]
    pub fn box_probe_running(&self) -> bool {
        self.box_probe
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    /// Called by the store loop right after each `go_online` (MOD-7 D11, blueprint D26). Awaits
    /// nothing: at most it spawns the registration probe into the box probe slot, whose task
    /// decides whether this box needs one.
    ///
    /// It checks no backend kind, and neither call site guards it: the store loop's
    /// `ConnEvent::Online` arm and its `ApplyMigrations` arm each call it right after a
    /// `go_online`, and `--demo` and the harness run on [`Backend::Memory`], which never reaches
    /// one. Opting in is the other half: only the binary turns
    /// [`with_registration_probe`](Self::with_registration_probe) on. A claim held elsewhere (a
    /// box probe, a login, an install or a background probe) skips the probe until the next swap
    /// (R-12); nothing was recorded, so the next launch retries.
    pub fn on_online(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>) {
        if !self.registration_probe {
            return;
        }
        self.sweep_finished();
        if let Err(err) = self.claim_is_free() {
            tracing::info!(reason = %err, "the registration probe waits for the next connect");
            return;
        }
        let Some(writer) = backend.writer() else {
            return;
        };
        let (env, hardware) = match self.probe_env() {
            Ok(found) => found,
            Err(err) => {
                tracing::warn!(%err, "the registration probe has no env to probe through");
                return;
            }
        };
        let frames = Frames::new(
            replies.clone(),
            ReplyAddr {
                seq: UNSOLICITED,
                origin: Origin::App,
            },
        );
        let answer = frames.answer(registration_probe_failed);
        self.box_probe = Some(tokio::spawn(answering(
            "box probe",
            run_box_probe(BoxProbeArgs {
                backend: backend.clone(),
                writer,
                env,
                hardware,
                decide: true,
                frames,
            }),
            Some(answer),
        )));
    }

    /// The production runtime: every transport this build ships, which is whatever
    /// [`DriverFactory::production`] registers — the ACP one and, since milestone 8, the headless
    /// CLI stream. The list lives there and is named nowhere here, so a third transport reaches
    /// the worker without touching this file (`R-AGT-5`).
    ///
    /// It carries an installer, and carrying one costs nothing until the user presses `i`: the
    /// value is where the registry is and where a tree may be written, and the HTTP clients it
    /// implies are built inside the install task (blueprint P-13).
    #[must_use]
    pub fn production() -> Self {
        Self::new(DriverFactory::production()).with_installer(InstallConfig::default())
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

    /// The deadlines a delivered paste runs under (MOD-22 D278).
    ///
    /// Production leaves it at [`DeliverLimits::default`]. The seam a case needs to reach
    /// `DeliverError::Timeout` without waiting fifteen seconds.
    #[must_use]
    pub fn with_deliver_limits(mut self, limits: DeliverLimits) -> Self {
        self.deliver_limits = limits;
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

    /// How many of those background tasks write `agent_box` — the set `claim_is_free` consults.
    ///
    /// The narrow half of [`background_len`](Self::background_len), and the one a test that says
    /// "a preview holds no claim" needs: a total cannot say whether the entry inside it is one the
    /// guard would refuse on (MOD-31 D4).
    ///
    /// The predicate is named through a closure rather than as `Background::writes_agent_box`,
    /// because `filter` hands it `&&Background` and a method on `&self` is a `fn(&Background)`:
    /// the same path spelled as a function item is a trait-bound error, not a clippy lint.
    /// [`claim_is_free`](Self::claim_is_free) asks the same question of the same collection and
    /// spells it `any(Background::writes_agent_box)` instead, because `any` hands it
    /// `&Background` and so takes the function item directly — `any` takes `FnMut(Self::Item)`
    /// where `filter` takes `FnMut(&Self::Item)`. Neither spelling is the other's mistake.
    #[must_use]
    pub fn writing_background_len(&self) -> usize {
        self.background
            .iter()
            .filter(|entry| entry.writes_agent_box())
            .count()
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
        // Every preview named here is one of the tasks about to be awaited, so the right to cancel
        // it dies with the handle (review finding M3).
        self.previews.clear();
        for entry in std::mem::take(&mut self.background) {
            let handle = entry.into_task();
            let abort = handle.abort_handle();
            if tokio::time::timeout(limit, handle).await.is_err() {
                abort.abort();
                tracing::warn!(?limit, "a background task did not finish and was aborted");
            }
        }
        // MOD-7 D27: the box probe answers from a task of its own too, and a harness frame taken
        // after this call has its report in it.
        if let Some(task) = self.box_probe.take() {
            let abort = task.abort_handle();
            if tokio::time::timeout(limit, task).await.is_err() {
                abort.abort();
                tracing::warn!(?limit, "a box probe did not finish and was aborted");
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

    /// Blueprint D206: the steps whose chat task is still running — its command channel is open.
    ///
    /// Not [`caps`](Self::caps): an ended chat keeps its entry until the next
    /// [`serve`](Self::serve) sweeps it (F-H), and a promotion or an accept judged against that
    /// entry would be refused over a chat that is already over. The receiver lives in
    /// [`run_chat`], so a closed channel is exactly a session task that has returned.
    #[must_use]
    pub fn live_steps(&self) -> Vec<StepId> {
        let mut steps: Vec<StepId> = self
            .live
            .iter()
            .filter(|(_, chat)| !chat.commands.is_closed())
            .map(|(step, _)| *step)
            .collect();
        steps.sort_unstable();
        steps
    }

    /// MOD-4 plan D165: `RunServed::Attach`'s other half, binding a chat to a promoted graph step.
    ///
    /// `start`'s checks minus the mint, plus the tail: the writer (refused off the
    /// server), the box, the opening agent's row (enabled), `driver_for`, its settings, the
    /// project's caps and the quota latch, then the step's log read **through the writer**
    /// (blueprint H-9), whose absence is "the step's log is not on this box". The spec is the
    /// promoted step's: its id, `cwd` and `extra_dirs` from the opening, and `resume` on
    /// [`OpeningPath::Resume`], whose handoff the session falls back to (MOD-37 M5). No re-probe
    /// rides on it: the walk that ran the step already started this agent here.
    ///
    /// Answers [`Served::Start`], whose session answers `addr` with `ChatAccepted` and every frame
    /// after it, or a `Failed` for `promote_step`. While a chat is live it answers
    /// [`Served::Deferred`], having refused at `addr` itself and moved the live chat's stream there
    /// (blueprint D185).
    pub async fn attach_promoted(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        promoted: crate::run_worker::Promoted,
    ) -> Served {
        match self.bind_promoted(backend, replies, addr, promoted).await {
            Ok(served) => served,
            Err(err) => Served::Reply(failed(PROMOTE_STEP, &err)),
        }
    }

    /// [`attach_promoted`](Self::attach_promoted)'s body, with `?` for the store's refusals.
    async fn bind_promoted(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        promoted: crate::run_worker::Promoted,
    ) -> Result<Served, StoreError> {
        // A follow served ahead of this bind (T7) is this promotion's only if it names its step;
        // taken on every path, so a refusal leaves no stale slot.
        let follow = self
            .pending_follow
            .take()
            .filter(|(step, _)| *step == promoted.step)
            .map(|(_, follow)| follow);
        // A chat that ended on this step before must not make the new one a second entry.
        self.live.retain(|_, chat| !chat.commands.is_closed());
        // Blueprint D185, for the race the run runtime's guard cannot see: two promotions served
        // before either was bound both found no chat live, so both passed `chat_open`. A chat this
        // runtime already holds is the only one — a second session on the same step would be two
        // agents writing one log, and on another step a chat the tab cannot hold — so this one is
        // refused with `ChatLive(None)`'s sentence.
        //
        // The tab reset its view when it read this promotion's `Promoted`, and this address is now
        // the newest `Orch` one from it, which leaves the live chat's own address stale. So the
        // live chat's stream moves here and its acceptance is sent again, after the refusal: the
        // tab is left driving the one session rather than beside a chat it can neither see nor end.
        if let Some(chat) = self.live.values().find(|chat| !chat.commands.is_closed()) {
            chat.stream.hand_over(
                replies,
                addr,
                StoreReply::Failed {
                    request: PROMOTE_STEP,
                    message: htui_orch::EngineError::ChatLive { step: None }.to_string(),
                },
            );
            return Ok(Served::Deferred);
        }
        let crate::run_worker::Promoted {
            step: step_id,
            project: project_id,
            opening,
            ..
        } = promoted;

        let writer = backend
            .writer()
            .ok_or_else(|| StoreError::Unreachable(htui_store::DATABASE_UNREACHABLE.to_owned()))?;
        let box_id = registered_box(backend).await?;
        let summary = backend
            .agents()
            .await?
            .into_iter()
            .find(|summary| summary.agent.id == opening.agent_id)
            .ok_or_else(|| StoreError::NotFound {
                entity: "agent",
                id: opening.agent_id.to_string(),
            })?;
        if !summary.agent.enabled {
            return Err(StoreError::Constraint(format!(
                "agent `{}` is disabled",
                summary.agent.name
            )));
        }
        // MOD-23 review L-3: a promotion is a chat on this box, so the per-box switch gates it the
        // way it gates `ChatStart`. Offline, `CacheStore::agents` answers `user_off: false`
        // because `agent_box` is not mirrored — but an offline backend has no writer and was
        // refused above.
        refuse_switched_off(&summary)?;
        let driver = self
            .factory
            .driver_for(&summary.agent, summary.on_box.as_ref())
            .map_err(|err| StoreError::Backend(err.to_string()))?;
        let settings: AgentSettings =
            serde_json::from_value(summary.agent.settings.clone()).unwrap_or_default();
        let project_caps =
            project_caps_for(project_id, backend.project_settings(project_id).await?)?;
        let quota_latch = quota_latch_for(&summary.agent, box_id, settings.quota.source);
        let Some(tail) = writer.step_events(step_id).await? else {
            return Ok(Served::Reply(StoreReply::Failed {
                request: PROMOTE_STEP,
                message: "the step's log is not on this box".to_owned(),
            }));
        };

        let (resume, opening_text, fallback) = match opening.path {
            // Review M-1: a resume whose handoff could not be built has no fallback.
            OpeningPath::Resume {
                session_ref,
                text,
                fallback,
            } => (
                Some(session_ref.clone()),
                text,
                fallback.map(|handoff| ResumeFallback {
                    session_ref,
                    handoff: handoff.text,
                }),
            ),
            OpeningPath::Handoff { text, .. } => (None, text, None),
        };
        let spec = SessionSpec {
            agent_id: opening.agent_id,
            step_id,
            cwd: opening.cwd,
            extra_dirs: opening.extra_dirs,
            env: BTreeMap::new(),
            model: opening
                .model
                .or_else(|| summary.agent.default_model.clone()),
            tools: htui_agent::driver::ToolExposure::default(),
            mcp: Vec::new(),
            permission: settings.permission.clone(),
            retain_raw: std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1"),
            resume,
            budget_micros: project_caps.run_micros,
        };

        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let caps = driver.caps();
        // The stream opens at the follow's address when one arrived first: the promotion's own
        // is an `Orch` one, which any later `Orch` request from the tab supersedes.
        let frames = Frames::new(replies.clone(), follow.unwrap_or(addr));
        self.live.insert(
            step_id,
            LiveChat {
                commands: commands_tx,
                caps,
                task: None,
                stream: frames.stream.clone(),
            },
        );
        self.started.push(step_id);

        let answer = frames.answer(chat_failed);
        let args = ChatArgs {
            driver,
            writer,
            binding: ChatBinding::Promoted {
                step_id,
                tail,
                fallback,
            },
            spec,
            prompt: opening_text,
            policy: settings.permission,
            caps,
            commands: commands_rx,
            frames,
            grace: self.grace,
            reprobe: None,
            project_caps,
            quota_latch,
        };
        Ok(Served::Start {
            step_id,
            task: Box::pin(answering("chat", run_chat(args), Some(answer))),
        })
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
        self.sweep_finished();

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
            StoreRequest::ChatFollow { step_id } => {
                self.follow(*step_id, addr);
                Served::Deferred
            }
            StoreRequest::ProbeAgents => match self.probe(backend, replies, addr).await {
                Ok(served) => served,
                Err(err) => Served::Reply(failed("probe_agents", &err)),
            },
            StoreRequest::ProbeBox => match self.probe_box(backend, replies, addr).await {
                Ok(served) => served,
                Err(err) => Served::Reply(failed("probe_box", &err)),
            },
            StoreRequest::SetToolPaths { agent_id, paths } => {
                match self
                    .set_tool_paths(backend, replies, addr, *agent_id, paths)
                    .await
                {
                    Ok(served) => served,
                    Err(err) => Served::Reply(failed(SET_TOOL_PATHS, &err)),
                }
            }
            StoreRequest::PromptPreview {
                item,
                template_name,
                scope,
            } => self.preview(
                backend,
                replies,
                addr,
                *item,
                template_name.clone(),
                scope.clone(),
            ),
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
            // A wiped copy: the envelope's own is wiped when the loop drops the envelope.
            StoreRequest::AuthDeliver { url } => self.auth_command(
                "auth_deliver",
                AuthCommand::Deliver {
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

    /// Forgets every task that has finished, so a finished one holds no claim (blueprint D27).
    ///
    /// Run before [`serve`](Self::serve) looks anything up and before
    /// [`on_online`](Self::on_online) asks for the claim.
    fn sweep_finished(&mut self) {
        // A finished chat leaves its entry behind; sweep before anything looks one up, so a second
        // chat on a finished step is a start and not a "no live chat".
        self.live.retain(|_, chat| !chat.commands.is_closed());
        // The same sweep for the tasks that answer their own request: a probe that has answered is
        // not a probe still running, and `background_len` is what a test reads.
        self.background.retain(|entry| !entry.task().is_finished());
        // And for the right to cancel a preview, which must not outlive the task it names: an
        // origin whose preview has answered has nothing left to supersede (review finding M3).
        self.previews.retain(|_, preview| !preview.is_finished());
        // And for the install claim, which is one deep: an install that has answered must not go
        // on refusing the next `i` (blueprint H-10).
        self.install.take_if(|live| live.task.is_finished());
        // And for the login claim, which is one deep for the same reason. Dropping the value here
        // is also what releases its `ReprobeClaim`: a finished flow must not go on excluding the
        // chat re-probe of its own row (blueprint H-25).
        self.auth.take_if(|live| live.task.is_finished());
        // And for the box probe, one deep too (MOD-7 D27): a probe that has answered must not go on
        // refusing an install, a login or the next registration probe.
        self.box_probe.take_if(|task| task.is_finished());
    }

    /// The env and hardware a probe resolves through: the injected ones when a test gave them
    /// (blueprint D35), else [`ProbeEnv::host`] over the working directory and
    /// [`SystemHardware::host`].
    fn probe_env(&self) -> Result<(ProbeEnv, Arc<dyn HardwareSource>), StoreError> {
        if let (Some(env), Some(hardware)) = (&self.probe_env, &self.hardware) {
            return Ok((env.clone(), Arc::clone(hardware)));
        }
        let cwd = std::env::current_dir().map_err(|err| {
            StoreError::Backend(format!("this process has no working directory: {err}"))
        })?;
        Ok((ProbeEnv::host(cwd), Arc::new(SystemHardware::host())))
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
        self.previews.clear();
        for entry in std::mem::take(&mut self.background) {
            entry.into_task().abort();
        }
        // MOD-7 D27: the box probe is aborted like the agent probe; each version child dies with
        // its `ChildGuard`, and a probe cut short recorded nothing, so the next launch retries.
        if let Some(task) = self.box_probe.take() {
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
        // MOD-66 D8: a tool-paths write is probing and writing one row, and this probe writes
        // every row. It is the one background writer consulted here: `r` beside a chat's
        // staleness re-probe stays allowed, as before.
        if let Some(agent_id) = self.background.iter().find_map(Background::tool_paths) {
            return Err(StoreError::Backend(tool_paths_running(agent_id)));
        }
        // MOD-7 D27: a box probe is already probing every row on this box.
        if self.box_probe_running() {
            return Err(StoreError::Backend(BOX_PROBE_RUNNING.to_owned()));
        }
        // An offline backend hands out no writer (MOD-25), so a probe off the server is refused
        // with plan D52's sentence. It is checked here rather than discovered on the write,
        // because by then the spawns have happened.
        let writer = backend.writer().ok_or_else(|| {
            StoreError::Unreachable(htui_store::REGISTRY_ON_SERVER_ONLY.to_owned())
        })?;
        let box_id = backend
            .box_info()
            .await?
            .ok_or_else(|| StoreError::NotFound {
                entity: "box",
                id: "this box is not registered".to_owned(),
            })?
            .box_id;
        let agents = backend.agents().await?;
        // Blueprint D35: the injected env when a test gave one, else this process's own.
        let (env, _) = self.probe_env()?;

        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(probe_agents_failed);
        self.background
            .push(Background::writing(tokio::spawn(answering(
                "agent probe",
                run_probe(ProbeArgs {
                    writer,
                    box_id,
                    agents,
                    env,
                    frames,
                }),
                Some(answer),
            ))));
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::SetToolPaths`] path (MOD-66 D7–D9). Every refusal but one comes
    /// **before anything is spawned**, in `auth_start`'s order (B6): writer (offline:
    /// `REGISTRY_ON_SERVER_ONLY`), registered box, the box claim, this row's re-probe claim, the
    /// row exists, it is `enabled`, its `launch` parses and declares a tool ([`LITERAL_LAUNCH`],
    /// the form's sentence), every key is a tool its `discovery.tools` declares, and every value
    /// passes [`parse_tool_path`]. A store refusal is
    /// an `Err`. A refused field is `Ok(Served::Reply(Failed))` carrying its own sentence, never
    /// behind a `StoreError` prefix (MOD-23 D250's precedent).
    ///
    /// Awaited on the loop's arm: `box_info()` and `agents()`, the same class of cost the probe's
    /// and the login's arms already pay (`R-NF-3`). The one refusal that is not here is `is_file`:
    /// it is filesystem I/O that a hung mount can stall, so [`run_tool_paths`] makes it, with the
    /// claims already held, and answers the same `Failed` (review L1). The probe runs in that
    /// task too, which holds the row's re-probe claim to its end (D8, H-10).
    async fn set_tool_paths(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        agent_id: AgentId,
        paths: &BTreeMap<String, String>,
    ) -> Result<Served, StoreError> {
        let writer = recording_writer(backend)?;
        let box_id = registered_box(backend).await?;
        self.claim_is_free()?;
        // D8: `claim_is_free` cannot see a chat's D60 re-probe, which runs inside the chat task.
        // The row's claim can, and it is moved into the task so it is held until the write lands.
        let claim = self
            .reprobe_claims
            .claim((agent_id, box_id))
            .ok_or_else(|| {
                StoreError::Backend(format!(
                    "a re-probe is running for agent {agent_id}; try again in a moment"
                ))
            })?;
        let agent = row_for(backend, agent_id).await?.agent;
        if !agent.enabled {
            return refuse_tool_paths(format!(
                "`{}` is disabled in the registry; nothing to set a path for",
                agent.name
            ));
        }
        // No serde text: it can quote an `env` value (`R-SEC-2`, the `checked_launch` rule).
        let Ok(launch) = AgentLaunch::deserialize(&agent.launch) else {
            return refuse_tool_paths(format!(
                "`{}`'s launch does not parse; nothing declares a tool",
                agent.name
            ));
        };
        let declared = launch
            .discovery
            .map(|discovery| discovery.tools)
            .unwrap_or_default();
        // D10: no `${tool}` to give a path, so not even an empty map is written (review N4).
        if declared.is_empty() {
            return refuse_tool_paths(LITERAL_LAUNCH.to_owned());
        }
        // `BTreeMap` order, so the first refusal is the same one every time.
        let mut checked = BTreeMap::new();
        for (tool, text) in paths {
            if !declared.contains_key(tool) {
                return refuse_tool_paths(format!(
                    "`{tool}` is not a tool `{}` declares",
                    agent.name
                ));
            }
            let path = match parse_tool_path(tool, text) {
                Ok(path) => path,
                Err(sentence) => return refuse_tool_paths(sentence),
            };
            checked.insert(tool.clone(), path);
        }
        // Blueprint D35: the injected env when a test gave one, else this process's own.
        let (env, _) = self.probe_env()?;

        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(tool_paths_failed);
        self.background.push(Background::writing_tool_paths(
            tokio::spawn(answering(
                "tool paths",
                run_tool_paths(ToolPathsArgs {
                    backend: backend.clone(),
                    writer,
                    box_id,
                    agent,
                    paths: checked,
                    env,
                    frames,
                    claim,
                }),
                Some(answer),
            )),
            agent_id,
        ));
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::ProbeBox`] path (MOD-7 D11): the registration probe without the
    /// "needs a probe" decision.
    ///
    /// Every refusal is here, **before anything is spawned**, and answered as a `Failed` at the
    /// requester's own address (blueprint D25): no writer (offline), no box row, a claim held, no
    /// env. From the spawn on, the task answers once with [`StoreReply::BoxProbed`].
    async fn probe_box(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
    ) -> Result<Served, StoreError> {
        // Since MOD-25 an offline backend answers `None` here: the registry is on the server.
        let writer = backend.writer().ok_or_else(|| {
            StoreError::Unreachable(htui_store::REGISTRY_ON_SERVER_ONLY.to_owned())
        })?;
        registered_box(backend).await?;
        self.claim_is_free()?;
        let (env, hardware) = self.probe_env()?;

        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(box_probe_failed);
        self.box_probe = Some(tokio::spawn(answering(
            "box probe",
            run_box_probe(BoxProbeArgs {
                backend: backend.clone(),
                writer,
                env,
                hardware,
                decide: false,
                frames,
            }),
            Some(answer),
        )));
        Ok(Served::Deferred)
    }

    /// The [`StoreRequest::PromptPreview`] path (MOD-2 D102, D103, D109).
    ///
    /// The probe's shape with one refusal instead of four, and **before anything is spawned**: an
    /// offline backend has no `prompt_template` row to render — `prompt_template`, `skill*` and
    /// `box_tool` are all outside the mirror — so the task could only fail on its first read
    /// (blueprint H-16). Plan D109 makes that the product's direction rather than a milestone
    /// expedient: `htui` is an online-only program, and the sentence names the real reason.
    ///
    /// **One preview per origin runs at a time** (review finding M3). A selection change issues six
    /// reads and this is the expensive one, so holding `j` used to leave a task per row alive to the
    /// end — eight store reads and an assembly each, against an eight-connection pool — for replies
    /// the shell's staleness index had already decided to drop. The newest request from an origin
    /// aborts that origin's previous one instead. Keyed per origin because two origins can each be
    /// waiting on a preview and neither supersedes the other.
    ///
    /// Not `async` and not fallible: the two things it does are a `match` and a `tokio::spawn`, so
    /// the worker's `select!` arm returns having awaited nothing at all (`R-NF-3`). The clone is a
    /// snapshot of the backend, not the backend: it cannot perform the swap the worker owns
    /// (blueprint E-10), and one that outlives a swap fails on its own arm and exits (H-17).
    fn preview(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        item: ItemId,
        template_name: Option<String>,
        scope: Scope,
    ) -> Served {
        if matches!(backend, Backend::Offline { .. }) {
            return Served::Reply(failed(
                crate::store_worker::PROMPT_PREVIEW,
                &StoreError::Unreachable(crate::preview::offline_refusal().to_owned()),
            ));
        }
        let origin = addr.origin.clone();
        let answer = Answer::at(replies.clone(), addr.clone(), preview_failed);
        let task = tokio::spawn(answering(
            "prompt preview",
            crate::preview::run_preview(
                backend.clone(),
                item,
                template_name,
                scope,
                replies.clone(),
                addr,
            ),
            Some(answer),
        ));
        // The previous preview for this origin, if it is still running, is work whose answer the
        // staleness index is already committed to dropping (review finding M3). Aborting is safe at
        // any await point the task is parked on: `run_preview` only reads — plan D102's "the preview
        // writes nothing" — so there is no half-finished row to leave behind, and the reply it would
        // have sent is one nobody would have rendered.
        if let Some(superseded) = self.previews.insert(origin, task.abort_handle()) {
            superseded.abort();
        }
        self.background.push(Background::reading(task));
        Served::Deferred
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
        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(install_failed);
        let task = tokio::spawn(answering(
            "install plan",
            run_plan(
                PlanArgs {
                    config,
                    agent,
                    cwd,
                    frames,
                },
                cancel.clone(),
            ),
            Some(answer),
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
        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(install_failed);
        let task = tokio::spawn(answering(
            "install",
            run_install(InstallArgs {
                config,
                plan,
                writer,
                box_id,
                agent: summary.agent,
                existing: summary.on_box,
                cwd,
                cancel: cancel.clone(),
                frames,
            }),
            Some(answer),
        ));
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
        let frames = Frames::new(replies.clone(), addr);
        let answer = frames.answer(auth_failed);
        let task = tokio::spawn(answering(
            "login",
            run_auth(AuthArgs {
                driver,
                agent: summary.agent,
                existing: summary.on_box,
                box_id,
                writer,
                cwd,
                cancel: cancel.clone(),
                commands: commands_rx,
                opener: self.opener.clone(),
                deliver_limits: self.deliver_limits,
                frames,
            }),
            Some(answer),
        ));
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
                message: NO_LOGIN_RUNNING.to_owned(),
            });
        };
        if live.commands.send(command).is_err() {
            self.auth.take_if(|live| live.task.is_finished());
            return Served::Reply(StoreReply::Failed {
                request,
                message: LOGIN_ENDED.to_owned(),
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
                message: NO_LOGIN_RUNNING.to_owned(),
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

    /// `Ok` when no box probe, install, probe, login or tool-paths write holds the claim (hazard
    /// H-10, MOD-21 D19, MOD-7 D27, MOD-66 D8).
    fn claim_is_free(&self) -> Result<(), StoreError> {
        // MOD-7 D27, the fourth holder: a box probe ends by probing every agent row on this box,
        // so it writes `agent_box` for every row, exactly as `ProbeAgents` does.
        if self.box_probe_running() {
            return Err(StoreError::Backend(BOX_PROBE_RUNNING.to_owned()));
        }
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
        // A probe writes `agent_box` for every enabled row, and an install's re-probe writes one of
        // them — and so does a chat's staleness re-probe, which is why the sentence names both.
        // Run together, they race on the same row and the last write wins, so the claim covers a
        // writing background task while it is in flight.
        //
        // A prompt preview is in the same collection and holds **no** claim: `run_preview` reaches
        // no write method (plan D102), so there is no row for an install to race. Before the split
        // this arm tested the whole collection, and holding `j` in the Backlog detail refused `i`
        // here — plus a login, a `ProbeBox` and a connect's registration probe — with a sentence
        // about a probe that was not running (MOD-31 D5).
        //
        // The Settings section's own `probing` flag is not enough either way: **any**
        // `StoreReply::Agents` clears it (`ui/tabs/settings/agents.rs`, module doc), and
        // `wants_requests` re-issues `Agents` on every activation, so `r` → switch tab → back → `i`
        // reaches here with `run_probe` still running.
        //
        // MOD-66 D8, the fifth holder: a `SetToolPaths` task writes its row and is a writing
        // background entry, so the check below would refuse beside it too, but with a sentence
        // about a probe and an install. It names itself first (B5).
        if let Some(agent_id) = self.background.iter().find_map(Background::tool_paths) {
            return Err(StoreError::Backend(tool_paths_running(agent_id)));
        }
        if self.background.iter().any(Background::writes_agent_box) {
            return Err(StoreError::Backend(BOX_WRITE_RUNNING.to_owned()));
        }
        Ok(())
    }

    /// [`StoreRequest::ChatFollow`]: the live chat on `step_id` streams at `addr` from now on
    /// (blueprint D185).
    ///
    /// Nothing is answered: the stream is the answer, and a chat that is already over has sent its
    /// last frame to the address it had. A follow for a step with no entry yet is kept for the
    /// bind that is about to open it (T7), in the runtime's one slot.
    fn follow(&mut self, step_id: StepId, addr: ReplyAddr) {
        match self.live.get(&step_id) {
            Some(chat) if !chat.commands.is_closed() => chat.stream.follow(addr),
            Some(_) => {}
            None => self.pending_follow = Some((step_id, addr)),
        }
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
        // `htui` is online-only since MOD-25: an offline backend hands out no writer, and a chat
        // it cannot record is refused here. This is the one place the unreachable-database
        // sentence is answered — `R-STO-4`'s "No item creation, no runs", rendered by the status
        // line as `chat_start: <sentence>` and by the Chat body.
        // The milestone-4 behaviour it replaced (`Writer::Buffered` into `<cache_dir>/pending/`,
        // plan D34) has since been removed.
        let writer = backend
            .writer()
            .ok_or_else(|| StoreError::Unreachable(htui_store::DATABASE_UNREACHABLE.to_owned()))?;
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
        // MOD-23 review L-3: the per-box switch gates a chat started on this box. Offline,
        // `CacheStore::agents` answers `user_off: false` because `agent_box` is not mirrored — but
        // an offline backend has no writer and was refused above.
        refuse_switched_off(&summary)?;
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
        // absent *row* is [`project_caps_for`]'s question.
        let project_caps =
            project_caps_for(project_id, backend.project_settings(project_id).await?)?;
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
        let quota_latch = quota_latch_for(&summary.agent, box_id, settings.quota.source);

        let chat = ChatRunSpec::mint(project_id, box_id, user, Some(agent_id), model.clone());
        writer.start_chat_run(&chat).await?;
        #[cfg(test)]
        tests::minted(&chat);

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
            env: BTreeMap::new(),
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
        let frames = Frames::new(replies.clone(), addr);
        self.live.insert(
            chat.step_id,
            LiveChat {
                commands: commands_tx,
                caps,
                task: None,
                stream: frames.stream.clone(),
            },
        );
        self.started.push(chat.step_id);

        // Plan D55, and everything about it is in what this is *not*: the chat is already
        // started, the future below is already decided, and the re-probe shares no channel with
        // either. It refreshes the row for the next chat; this one proceeds on what
        // `tools::resolve` gave it, because coupling a chat's start to a 60-second handshake
        // timeout would make a stale row a minute of waiting.
        //
        // Two conditions, each for its own reason. `acp`, because tier 2 *is* `initialize` and a
        // `cli` row has none (milestone 8's problem). And stale, or there is nothing to learn.
        //
        // The first is a property of the *row* and holds for D60's trigger too, so it is what
        // builds the arguments; staleness is the third trigger's own condition and is applied to
        // the spawn alone.
        let reprobe = (summary.agent.transport == Transport::Acp).then(|| ReprobeArgs {
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
        //
        // MOD-7 blueprint D27: nor while a box probe runs, which ends by re-probing every agent
        // row on this box anyway; a staleness re-probe beside it would race it for this row.
        let stale = needs_reprobe(&summary.agent, summary.on_box.as_ref(), Utc::now())
            && !self.box_probe_running();
        let reprobe = match (stale, reprobe) {
            (true, Some(args)) => {
                self.background
                    .push(Background::writing(tokio::spawn(answering(
                        "re-probe",
                        run_reprobe(args),
                        None,
                    ))));
                None
            }
            (_, held) => held,
        };

        let step_id = chat.step_id;
        // MOD-24 D4: copies of the pair and the writer, because the panicked task is dropped with
        // the originals before its answer is sent. Review L3: one closed flag between the two, so
        // a panic after the session's own close never closes the run a second time.
        let closed = Arc::new(AtomicBool::new(false));
        let answer =
            frames
                .answer(chat_failed)
                .closing(writer.clone(), chat.clone(), Arc::clone(&closed));
        let args = ChatArgs {
            driver,
            writer,
            binding: ChatBinding::Fresh(chat, closed),
            spec,
            prompt,
            policy: settings.permission,
            caps,
            commands: commands_rx,
            frames,
            grace: self.grace,
            reprobe,
            project_caps,
            quota_latch,
        };
        Ok(Served::Start {
            step_id,
            task: Box::pin(answering("chat", run_chat(args), Some(answer))),
        })
    }
}

/// Blueprint D205: what a chat session records against.
#[derive(Debug)]
enum ChatBinding {
    /// MOD-2's own chat: a `run(kind='chat')` minted at start, closed at the end. The flag is
    /// raised once that close has landed, and shared with the task's panic answer (MOD-24 review
    /// L3), which then leaves the run alone.
    Fresh(ChatRunSpec, Arc<AtomicBool>),
    /// MOD-4 plan D165: a promoted graph step. No run is minted and none is closed: the step's
    /// status is the engine's (`awaiting_approval`, promoted) and stays so when the session ends.
    Promoted {
        /// The promoted step, which keeps its id.
        step_id: StepId,
        /// The step's persisted rows, read through the writer (blueprint H-9): where the
        /// continuing recorder starts (plan D164).
        tail: Vec<SessionEvent>,
        /// MOD-37 M5: what the chat opens with if resuming the step's session fails. `Some`
        /// exactly when the opening is [`OpeningPath::Resume`] and the engine built its handoff
        /// (review M-1); `None` on a resume makes a failed start fail the chat, as before M5.
        fallback: Option<ResumeFallback>,
    },
}

/// MOD-37 M5: what a promoted chat opens with when resuming the step's own session fails: the
/// session it tried, and the handoff prompt the engine built for this promotion.
#[derive(Debug, Clone)]
struct ResumeFallback {
    session_ref: AgentSessionRef,
    handoff: String,
}

impl ChatBinding {
    /// The step every row of the session is recorded against.
    const fn step_id(&self) -> StepId {
        match self {
            Self::Fresh(chat, _) => chat.step_id,
            Self::Promoted { step_id, .. } => *step_id,
        }
    }

    /// The request a failed start is answered as: the one that opened the session.
    const fn request(&self) -> &'static str {
        match self {
            Self::Fresh(..) => "chat_start",
            Self::Promoted { .. } => PROMOTE_STEP,
        }
    }

    /// Closes what the session opened when it ends with `status`. A promoted step's session
    /// opened nothing: `finish_chat_run` is never called for a graph step. A fresh chat's close
    /// that landed raises its closed flag (MOD-24 review L3); one that failed leaves it down, so a
    /// later panic's answer still tries.
    async fn close(&self, writer: &Writer, status: RunStatus) {
        match self {
            Self::Fresh(chat, closed) => {
                if close_run(writer, chat, status).await {
                    closed.store(true, Ordering::Release);
                }
            }
            Self::Promoted { .. } => {}
        }
    }
}

/// [`StoreRequest::name`] of a promotion (blueprint D209): what a promoted chat's refusals are
/// answered as, since the promotion is the request that opened it.
const PROMOTE_STEP: &str = crate::run_worker::ORCH_NAMES[5];

/// Everything one chat session needs.
pub struct ChatArgs {
    driver: Box<dyn AgentDriver>,
    writer: Writer,
    /// What the session records against (blueprint D205).
    binding: ChatBinding,
    spec: SessionSpec,
    prompt: String,
    policy: PermissionPolicy,
    caps: DriverCaps,
    commands: mpsc::UnboundedReceiver<ChatCommand>,
    frames: Frames,
    grace: Duration,
    /// Plan D60: `Some` when a spawn failure should refresh this box's row for this agent. `None`
    /// for a `cli` row (tier 2 is `initialize`, which a `cli` row has none of), and for a chat
    /// whose staleness re-probe is already running — a second one would race it for the same row.
    reprobe: Option<ReprobeArgs>,
    /// Plan D70: `project.settings`'s two token caps, read at `ChatStart`.
    ///
    /// Not named `caps`: that field above is [`DriverCaps`], which is what the *transport* can do.
    /// These are what the **project** allows it to spend, and two fields called `caps` in one
    /// struct would be one bug away from each other.
    project_caps: ProjectCaps,
    /// Plan D66-D68: the `agent_box` row this chat latches its allowance into
    /// ([`quota_latch_for`]). Always one: `Backend::writer` answers `None` offline, so a chat that
    /// gets this far has a store with an `agent_box` table. `Recorder`'s own latch stays an
    /// `Option` for the recorders that have none.
    quota_latch: QuotaLatch,
}

impl core::fmt::Debug for ChatArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChatArgs")
            .field("driver", &self.driver.name())
            .field("step", &self.binding.step_id())
            .field("spec", &self.spec)
            .field("project_caps", &self.project_caps)
            .finish()
    }
}

/// The caps a chat enforces, out of `project.settings` (plan D70) — and what an **absent** row
/// means.
///
/// A `None` is a chat whose project is not in the database the chat is about to write to: the id
/// came from the tab's own scope, read from that same database, so the row went away under the
/// chat and refusing is the honest answer. Before MOD-25 an offline chat read the row from the
/// mirror and took an unmirrored project as `{}` (review M-3); a chat now starts only online.
///
/// A document that does not parse refuses too — that is `start_chat`'s own comment.
fn project_caps_for(
    project_id: ProjectId,
    settings: Option<Value>,
) -> Result<ProjectCaps, StoreError> {
    let settings = match settings {
        Some(settings) => settings,
        None => {
            return Err(StoreError::NotFound {
                entity: "project",
                id: project_id.to_string(),
            });
        }
    };
    ProjectCaps::from_settings(&settings).map_err(|err| StoreError::Constraint(err.to_string()))
}

/// The `agent_box` row a chat latches its allowance into (plan D66-D68).
///
/// The decision is made **here**, at chat start, rather than discovered on the first `usage` row.
/// Every chat gets one: `Backend::writer` answers `None` offline (MOD-25), so a chat that gets this
/// far has a store with an `agent_box` table. The `None` an offline chat once got (the mirror has
/// none, plan D52) left with the offline path (CLEAN-7).
///
/// `source` is `agent.settings.quota.source` and `billing` is `agent.billing`, both read off the
/// row. Nothing here looks at `agent.name` (`R-AGT-5`) — the name is logged, and a log line is not
/// a dispatch.
fn quota_latch_for(agent: &Agent, box_id: BoxId, source: QuotaSource) -> QuotaLatch {
    QuotaLatch {
        agent_id: agent.id,
        box_id,
        source,
        billing: agent.billing,
    }
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
    env: ProbeEnv,
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
        agents,
        env,
        frames,
    } = args;
    let reply = match probe_agents_on(&writer, box_id, agents, &env).await {
        Ok(agents) => StoreReply::Agents(agents),
        Err(err) => StoreReply::Failed {
            request: "probe_agents",
            message: err.to_string(),
        },
    };
    frames.reply(&frames.addr(), reply);
}

/// The per-box switch at a chat's start (MOD-23 review L-3): a row the human switched off on this
/// box (`agent_box.user_off`) is refused whatever the probe's verdict, by `ChatStart` and by a
/// promotion alike, with this one sentence.
///
/// Read off the [`AgentSummary`](htui_core::model::AgentSummary) both paths already hold, so it
/// costs no read. A row with no `agent_box` row is `user_off: false` and passes. Reads the switch,
/// never the name (`R-AGT-5`).
///
/// What a refusal leaves differs by path (MOD-23 re-review Low-3). For `ChatStart` it refuses
/// before this runtime writes any row. For a promotion it does **not**: the engine has already
/// written the promotion by the time `bind_promoted` runs, so a refused promotion leaves the step
/// `awaiting_approval` with `promoted_at` set and no chat, exactly as the `agent is disabled`
/// refusal beside it does. Promoting again once the switch is on opens the chat.
fn refuse_switched_off(summary: &htui_core::model::AgentSummary) -> Result<(), StoreError> {
    if summary.user_off {
        return Err(StoreError::Constraint(switched_off(&summary.agent.name)));
    }
    Ok(())
}

/// The one sentence for an agent switched off on this box (MOD-23 review L-3, re-review Low-1):
/// [`refuse_switched_off`]'s, and `htui_worker`'s `Kit::driver`'s for a step admitted before the
/// switch. The name is only quoted, never branched on (`R-AGT-5`).
pub(crate) fn switched_off(name: &str) -> String {
    htui_worker::switched_off(name)
}

/// [`run_probe`]'s loop, shared with the box probe (MOD-7 D11): every enabled row of `agents`
/// probed on `box_id` through `env`, one at a time, each fresh row written through `writer`.
///
/// The rows come back as this probe left them: `on_box` replaced by what was written, with
/// `enabled` as the store kept it under the per-box switch (MOD-23 D243), and left as read for a
/// disabled row or a hand-written one the probe kept (plan D51). The first write that fails ends
/// the walk with its error.
async fn probe_agents_on(
    writer: &Writer,
    box_id: BoxId,
    mut agents: Vec<htui_core::model::AgentSummary>,
    env: &ProbeEnv,
) -> Result<Vec<htui_core::model::AgentSummary>, StoreError> {
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
            ProbeOutcome::Row(mut row) => {
                writer.upsert_agent_box(&row).await?;
                // MOD-23 D243: the store kept `enabled AND NOT user_off`; the reply says the same.
                //
                // Review L-1: `user_off` is the start-of-walk read's, so a switch flipped while
                // this walk runs shows on the next read, not in this reply. The store row is right
                // regardless (`upsert_agent_box` applies the rule against the row it writes), and
                // the section's `on this box` cell checks `user_off` before `enabled`.
                row.enabled &= !summary.user_off;
                summary.on_box = Some(row);
            }
            // Plan D51: a hand-written row the probe could not confirm is left exactly as it is,
            // `probed_at` included.
            ProbeOutcome::Kept { reason } => {
                tracing::info!(agent = %summary.agent.name, reason, "the probe left a row alone");
            }
        }
    }

    Ok(agents)
}

/// Everything the box probe task owns (MOD-7 D11, blueprint D26).
///
/// The reads go through a [`Backend`] clone — `box_info`, `app_settings` and `agents` are not
/// `WriteStore` methods — and the writes through the [`Writer`] taken at spawn, so a swap the
/// loop makes meanwhile cannot redirect them.
struct BoxProbeArgs {
    backend: Backend,
    writer: Writer,
    env: ProbeEnv,
    hardware: Arc<dyn HardwareSource>,
    /// The registration probe's "needs a probe" decision (plan D5, D18); `ProbeBox` skips it.
    decide: bool,
    frames: Frames,
}

impl core::fmt::Debug for BoxProbeArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BoxProbeArgs")
            .field("writer", &self.writer.label())
            .field("decide", &self.decide)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

/// One box probe, then one probe of this box's agents (MOD-7 D11–D13, blueprint D25).
///
/// One [`EffectiveSpec`](box_probe::spec::EffectiveSpec) is computed from the stored
/// `box_probe_spec` and used for both the decision and the probe, so the digest compared and the
/// digest recorded cannot differ. With `decide`, a box probed by this `htui` under this spec is
/// left alone and a reconnect costs three reads; **nothing is sent** unless the stored spec was
/// ignored, which one [`unchanged`](BoxProbeReport::unchanged) report names at every reconnect
/// (plan D17). Otherwise exactly one [`StoreReply::BoxProbed`] goes back, failures included —
/// never a `Failed`, which the shell would drop at [`UNSOLICITED`] (blueprint F-D).
async fn run_box_probe(args: BoxProbeArgs) {
    let BoxProbeArgs {
        backend,
        writer,
        env,
        hardware,
        decide,
        frames,
    } = args;
    let mut report = BoxProbeReport::default();
    let send = |report: BoxProbeReport| {
        frames.reply(&frames.addr(), StoreReply::BoxProbed(report));
    };

    let box_id = match backend.box_info().await {
        Ok(Some(info)) => info.box_id,
        Ok(None) => {
            report.box_failed = Some("this box is not registered".to_owned());
            return send(report);
        }
        Err(err) => {
            report.box_failed = Some(err.to_string());
            return send(report);
        }
    };
    let settings = match backend.app_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            report.box_failed = Some(err.to_string());
            return send(report);
        }
    };
    let effective = box_probe::spec::effective(
        box_probe::spec::seed(),
        settings.get(box_probe::spec::SETTING_KEY),
    );
    report.spec_error.clone_from(&effective.error);

    if decide {
        match writer.boxes().await {
            Ok(records) => match records.iter().find(|record| record.row.id == box_id) {
                Some(record) if record.needs_probe(htui_store::HTUI_VERSION, &effective.digest) => {
                }
                // Probed already, but an ignored overlay is still worth saying: the maintainer
                // who stored it would otherwise never hear that it did not merge.
                Some(_) => {
                    if effective.error.is_some() {
                        report.unchanged = true;
                        send(report);
                    }
                    return;
                }
                None => {
                    tracing::warn!(%box_id, "this box is not among the user's boxes; no probe");
                    return;
                }
            },
            Err(err) => {
                tracing::warn!(%err, "the registration probe could not read the boxes");
                return;
            }
        }
    }

    let probe = box_probe::probe_box(
        box_id,
        &env,
        hardware.as_ref(),
        &effective,
        htui_store::HTUI_VERSION,
        Utc::now(),
    )
    .await;
    if let Err(err) = writer.record_box_probe(&probe).await {
        report.box_failed = Some(err.to_string());
        return send(report);
    }
    report.tools = probe.tools.len();
    report.probed_tags = probe.probed_tags;

    match backend.agents().await {
        Ok(agents) => match probe_agents_on(&writer, box_id, agents, &env).await {
            Ok(agents) => report.installable = installable(&agents),
            Err(err) => report.agents_failed = Some(err.to_string()),
        },
        Err(err) => report.agents_failed = Some(err.to_string()),
    }
    send(report);
}

/// The enabled agents a probe just found `missing` whose launch declares where to install them
/// from (PRD D7): named on the status line, never installed.
fn installable(agents: &[htui_core::model::AgentSummary]) -> Vec<String> {
    agents
        .iter()
        .filter(|summary| summary.agent.enabled)
        .filter(|summary| {
            summary
                .on_box
                .as_ref()
                .and_then(ProbeSnapshot::from_row)
                .is_some_and(|snapshot| matches!(snapshot.status, ProbeStatus::Missing))
        })
        .filter(|summary| htui_agent::launch::declares_install(&summary.agent.launch))
        .map(|summary| summary.agent.name.clone())
        .collect()
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

/// A `SetToolPaths` field refusal (MOD-66 D9): `Failed` with its own sentence, answered on the
/// arm.
fn refuse_tool_paths(message: String) -> Result<Served, StoreError> {
    Ok(Served::Reply(StoreReply::Failed {
        request: SET_TOOL_PATHS,
        message,
    }))
}

/// Everything a `SetToolPaths` task owns (MOD-66 D7). Reads go through a [`Backend`] clone
/// (`BoxProbeArgs`'s precedent: the reply carries the registry re-read), and the write through
/// the [`Writer`] taken at spawn. `claim` is the row's re-probe claim, **held to the end** and
/// released by its `Drop`, aborted or not (D8, H-10).
struct ToolPathsArgs {
    backend: Backend,
    writer: Writer,
    box_id: BoxId,
    agent: Agent,
    paths: BTreeMap<String, String>,
    env: ProbeEnv,
    frames: Frames,
    claim: ReprobeClaim,
}

impl core::fmt::Debug for ToolPathsArgs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolPathsArgs")
            .field("writer", &self.writer.label())
            .field("agent", &self.agent.name)
            .field("paths", &self.paths.len())
            .finish_non_exhaustive()
    }
}

/// First D9's `is_file` check of every path: a path that is not a file is the task's one answer,
/// `Failed` with the tool-naming sentence the arm's refusals use, and nothing is written.
///
/// Then one probe of one row over the requested map, written whatever it found (MOD-66 D9):
/// [`probe_snapshot`], never `probe_agent`, so D51's `Kept` cannot swallow the edit (B1). Exactly
/// one reply at the request's address: `AgentWritten { agents, ToolPaths { id, name, status } }`,
/// or `Failed { "set_tool_paths" }` when the write fails. A re-read that fails after an applied
/// write also answers `Failed` (`agent_settings::serve`'s known residue, B15).
///
/// `upsert_agent_box` applies `enabled AND NOT user_off`, so a row switched off on this box stays
/// off with no code here (MOD-23 D243).
async fn run_tool_paths(args: ToolPathsArgs) {
    let ToolPathsArgs {
        backend,
        writer,
        box_id,
        agent,
        paths,
        env,
        frames,
        claim,
    } = args;
    // D9's last check, here rather than on the arm: `metadata` on a hung mount would stall every
    // tab's store requests (review L1). `BTreeMap` order, so the first refusal is the same one
    // every time.
    for (tool, path) in &paths {
        if !htui_agent::probe::is_file(std::path::Path::new(path)).await {
            drop(claim);
            frames.reply(
                &frames.addr(),
                StoreReply::Failed {
                    request: SET_TOOL_PATHS,
                    message: format!("`{tool}`: `{path}` is not a file on this box"),
                },
            );
            return;
        }
    }
    let ctx = ProbeContext {
        env,
        now: Utc::now(),
    };
    let snapshot = probe_snapshot(&agent, &paths, &ctx, &SpawnTier2::default()).await;
    let row = agent_box_row(&agent, box_id, &snapshot, ctx.now);
    let reply = match writer.upsert_agent_box(&row).await {
        Err(err) => failed(SET_TOOL_PATHS, &err),
        Ok(_) => match backend.agents().await {
            Ok(agents) => {
                let name = agents
                    .iter()
                    .find(|summary| summary.agent.id == agent.id)
                    .map_or_else(|| agent.name.clone(), |summary| summary.agent.name.clone());
                StoreReply::AgentWritten {
                    agents,
                    outcome: AgentWrite::ToolPaths {
                        id: agent.id,
                        name,
                        status: snapshot.status,
                    },
                }
            }
            Err(err) => failed(SET_TOOL_PATHS, &err),
        },
    };
    // Before the answer: a request the reply prompts finds the row's claim free.
    drop(claim);
    frames.reply(&frames.addr(), reply);
}

/// The writer of a backend that can hold an `agent_box` row, or the refusal that names why not.
///
/// An offline backend hands out no writer (MOD-25), which this answers with
/// `REGISTRY_ON_SERVER_ONLY` (plan D52): the whole point of asking here is to hear that sentence
/// before the work rather than after it.
fn recording_writer(backend: &Backend) -> Result<Writer, StoreError> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(htui_store::REGISTRY_ON_SERVER_ONLY.to_owned()))?;
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
    // MOD-7 blueprint F-E: the one reading the Settings section and the box probe share.
    if htui_agent::launch::declares_install(&agent.launch) {
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
    let addr = frames.addr();

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
    let addr = frames.addr();

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
    deliver_limits: DeliverLimits,
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
/// **A pasted redirect is delivered from here** (MOD-22 D269). It is validated against the flow's
/// own record of the advertised redirect, never the pane's, and its one `GET` runs beside the
/// flow, so the wire, stderr and `x` are served while it is in flight. Every `AuthDeliver` is
/// answered once, before the flow's own last frame. The address is a credential for the length
/// of that `GET`: the credential rule of `htui_agent::auth::loopback` binds everything here, and
/// nothing on this path is logged.
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
        deliver_limits,
        frames,
    } = args;
    // The `AuthStart`'s address until the choice arrives, and the `AuthChoose`'s afterwards.
    let mut addr = frames.addr();

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
    // The loopback redirect the newest link advertised, if any, and the one delivery allowed in
    // flight (MOD-22 D269).
    let mut advertised: Option<Advertised> = None;
    let mut delivering: Option<Delivering> = None;
    let outcome = loop {
        tokio::select! {
            // Biased towards the flow: the moment it has answered there is nothing left to
            // forward that the drain below will not pick up.
            biased;
            outcome = &mut running => break outcome,
            event = events_rx.recv(), if listening => match event {
                Some(event) => {
                    // Read before `auth_frame` consumes it. Newest wins; a link that advertises
                    // no loopback redirect leaves the previous one standing.
                    if let AuthEvent::Url(link) = &event
                        && let Some(found) = Advertised::from_auth_url(link)
                    {
                        advertised = Some(found);
                    }
                    frames.reply(&addr, StoreReply::Auth(auth_frame(event)));
                }
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
                Some(AuthCommand::Deliver { url, reply }) => {
                    let refusal = accept_delivery(
                        url,
                        &reply,
                        advertised.as_ref(),
                        &mut delivering,
                        deliver_limits,
                    );
                    if let Some(message) = refusal {
                        frames.reply(
                            &reply,
                            StoreReply::Failed {
                                request: "auth_deliver",
                                message,
                            },
                        );
                    }
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
            Some((reply, answer)) = settle(&mut delivering), if delivering.is_some() => {
                frames.reply(&reply, delivered(answer));
            }
        }
    };

    // Closed **before** the drain, the re-probe and the row write, which are seconds a caller could
    // otherwise spend a request into: from here on `auth_command` fails to send and refuses the
    // request itself, which is the answer it already has words for.
    commands.close();
    // A delivery still in flight is answered before the queue, the re-probe and the last frame
    // (MOD-22 D269(d), D277), so its answer never trails the login's result (R-10). A flow that
    // ended — `x`, a shutdown, the idle clock, a declined chooser — drops it at once: by now the
    // child is reaped (`acp::auth::run`), so its own listener's socket has already ended, and a
    // listener that is not the child's would otherwise hold `Cancelled` for the whole response
    // deadline. Any other end waits for it, and `x` still cuts that wait short.
    if let Some(pending) = delivering.take() {
        let ended = cancel.is_cancelled()
            || matches!(
                outcome,
                Ok(AuthOutcome::Cancelled | AuthOutcome::Declined | AuthOutcome::Idle { .. })
            );
        let answer = if ended {
            ended_reply("auth_deliver")
        } else {
            tokio::select! {
                biased;
                () = cancel.cancelled() => ended_reply("auth_deliver"),
                answer = pending.answer => delivered(answer),
            }
        };
        frames.reply(&pending.reply, answer);
    }
    // What was already in the queue when the loop broke is answered at its own address rather than
    // dropped with the task: a `Served::Deferred` the pane never hears back from is the shape
    // MOD-20's review rejected, and this flow has nothing left to do for any of them.
    while let Ok(command) = commands.try_recv() {
        let (request, reply) = match command {
            AuthCommand::Choose { reply, .. } => ("auth_choose", reply),
            AuthCommand::Open { reply, .. } => ("auth_open", reply),
            AuthCommand::Deliver { reply, .. } => ("auth_deliver", reply),
        };
        frames.reply(&reply, ended_reply(request));
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

/// One `AuthCommand::Deliver` inside the running login (MOD-22 D269): the paste checked against
/// the newest advertised redirect and, when it passes and no delivery is in flight, its `GET` put
/// in `slot`. `Some` is the refusal the request is owed; `None` means the slot's answer is.
///
/// The `GET` is not awaited here, as `Open` is: a response deadline of seconds would stop the wire
/// and `x` for all of them. The pasted text ends in this function; the delivery holds only what
/// its `GET` needs.
fn accept_delivery(
    url: RedirectUrl,
    reply: &ReplyAddr,
    advertised: Option<&Advertised>,
    slot: &mut Option<Delivering>,
    limits: DeliverLimits,
) -> Option<String> {
    let Some(found) = advertised else {
        return Some(NO_LOOPBACK_REDIRECT.to_owned());
    };
    if slot.is_some() {
        return Some(DELIVERY_IN_FLIGHT.to_owned());
    }
    match loopback::validate(&url, found) {
        Ok(delivery) => {
            *slot = Some(Delivering {
                reply: reply.clone(),
                answer: Box::pin(loopback::deliver(delivery, limits)),
            });
            None
        }
        Err(err) => Some(err.to_string()),
    }
}

/// A delivery in flight (MOD-22 D269): who asked, and the one `GET`.
struct Delivering {
    reply: ReplyAddr,
    answer: BoxFuture<'static, Result<ListenerReply, DeliverError>>,
}

impl core::fmt::Debug for Delivering {
    /// The address only: the future holds the request line, which carries the pasted code.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Delivering")
            .field("reply", &self.reply)
            .finish_non_exhaustive()
    }
}

/// The in-flight delivery's answer, or never.
///
/// `tokio::select!` evaluates a branch's expression even while its precondition is false, so this
/// is an `async fn` that touches the slot only when polled. It takes the slot **after** the answer,
/// never before: when another arm wins, this future is dropped, and the delivery must still be
/// there. `None` only if the slot emptied under the await, which nothing can do while this future
/// holds it; the arm's pattern then disables itself instead of the worker panicking (review L-4).
async fn settle(
    slot: &mut Option<Delivering>,
) -> Option<(ReplyAddr, Result<ListenerReply, DeliverError>)> {
    let Some(delivering) = slot.as_mut() else {
        return std::future::pending().await;
    };
    let answer = delivering.answer.as_mut().await;
    slot.take().map(|delivering| (delivering.reply, answer))
}

/// A delivery's answer as the reply its request is owed (D268): what the listener said, or why
/// nothing usable came back. Neither carries a byte of the pasted address.
fn delivered(answer: Result<ListenerReply, DeliverError>) -> StoreReply {
    match answer {
        Ok(reply) => StoreReply::Auth(AuthFrame::Delivered(reply)),
        Err(err) => StoreReply::Failed {
            request: "auth_deliver",
            message: err.to_string(),
        },
    }
}

/// The refusal of a request this login can no longer serve ([`LOGIN_ENDED`]).
fn ended_reply(request: &'static str) -> StoreReply {
    StoreReply::Failed {
        request,
        message: LOGIN_ENDED.to_owned(),
    }
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

/// Where a task's frames go: the address of the request that opened it, until a chat's is moved.
///
/// A chat is the one task whose address can move (MOD-4 plan D165, blueprint D185). A promoted
/// chat opens at an `Orch` request's address, and the shell's staleness index is keyed by request
/// kind (`App::is_fresh`), so any later `Orch` request from the Chat tab — a second promotion the
/// run runtime refuses included — would supersede it, and every frame after it would be dropped
/// as stale while the store went on recording. [`StoreRequest::ChatFollow`] moves the stream to a
/// chat request's address that nothing else from the tab supersedes, and a promotion refused at
/// the bind moves it to that promotion's (`AgentRuntime::attach_promoted`).
///
/// Shared, because the mover is the runtime and the sender is the session task. One lock around
/// both the address and the send, so no frame is sent to an address a move has already left, and
/// the acceptance a move re-sends reaches the new address before any frame that follows it.
#[derive(Debug, Clone)]
struct Stream(Arc<Mutex<StreamState>>);

/// [`Stream`]'s state.
#[derive(Debug)]
struct StreamState {
    /// Where the next frame goes.
    addr: ReplyAddr,
    /// The chat's `ChatAccepted`, once sent: what a move re-sends to an asker that has not seen it.
    accepted: Option<StoreReply>,
}

impl Stream {
    fn lock(&self) -> std::sync::MutexGuard<'_, StreamState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// [`StoreRequest::ChatFollow`]: every frame from now on goes to `addr`.
    fn follow(&self, addr: ReplyAddr) {
        self.lock().addr = addr;
    }

    /// A refused promotion (blueprint D185): `refusal` answers `addr`, then the stream moves there
    /// and the acceptance, once there is one, is sent again, because the asker's view of the chat
    /// was reset since it was first sent. One lock for all three, so no frame lands between them.
    fn hand_over(
        &self,
        tx: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
        refusal: StoreReply,
    ) {
        let send = |reply: StoreReply| {
            // A UI that has gone away is not an error, as in `Frames::send`.
            let _ = tx.send(ReplyEnvelope {
                seq: addr.seq,
                origin: addr.origin.clone(),
                reply,
            });
        };
        let mut state = self.lock();
        send(refusal);
        if let Some(accepted) = state.accepted.clone() {
            send(accepted);
        }
        state.addr = addr.clone();
    }
}

/// The reply-channel side of one chat: one address, one sender, one place frames are shaped.
struct Frames {
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    stream: Stream,
}

impl Frames {
    /// Frames answering at `addr`, the address of the request that opened the task.
    fn new(tx: mpsc::UnboundedSender<ReplyEnvelope>, addr: ReplyAddr) -> Self {
        Self {
            tx,
            stream: Stream(Arc::new(Mutex::new(StreamState {
                addr,
                accepted: None,
            }))),
        }
    }

    /// Where the stream goes now.
    fn addr(&self) -> ReplyAddr {
        self.stream.lock().addr.clone()
    }

    /// Sends `reply` at the stream's address, under the lock a move takes.
    fn to_stream(&self, reply: StoreReply) {
        let state = self.stream.lock();
        self.send(state.addr.clone(), reply);
    }

    /// The chat's acceptance, at the stream's address, kept for a move to re-send.
    fn accept(&self, reply: StoreReply) {
        let mut state = self.stream.lock();
        state.accepted = Some(reply.clone());
        self.send(state.addr.clone(), reply);
    }

    /// One recorded, scrubbed envelope.
    fn event(&self, envelope: DriverEnvelope) {
        self.to_stream(StoreReply::Chat(ChatFrame::Event(Box::new(envelope))));
    }

    /// One of the three rows `htui` authors itself, shaped as an `other` event for transport only.
    ///
    /// The recorder has already written the real row with its real kind; this is the copy the tab
    /// renders, and shaping it as `other` is what keeps [`ChatFrame`] one type instead of four.
    fn local(&self, update: &str, body: Value, at: DateTime<Utc>) {
        self.event(DriverEnvelope {
            event: DriverEvent::Other(OtherEvent {
                update: update.to_owned(),
                body,
            }),
            raw: None,
            at,
        });
    }

    /// The session is over.
    fn ended(&self, stop_reason: StopReason) {
        self.to_stream(StoreReply::Chat(ChatFrame::Ended { stop_reason }));
    }

    /// The session died.
    fn failed(&self, message: String) {
        self.to_stream(StoreReply::Chat(ChatFrame::Failed { message }));
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

/// The replies a task owes its request if it panics: the ones that end the request (MOD-53).
///
/// A plain `fn` so every spawn site names its own, and so the answer carries no state of the task
/// it outlives; a fresh chat's answer carries copies of the run's ids and a writer (MOD-24 D4).
/// More than one because a chat's stream ends the way its own failures end it: `Failed`, then
/// `Ended`.
type LastWord = fn(String) -> Vec<StoreReply>;

/// Where a panicked task's [`LastWord`] goes: the task's reply channel and its stream (MOD-53).
///
/// The [`Stream`] rather than a copied [`ReplyAddr`], so a chat that has moved to a later request's
/// address is answered where its frames were going, not where they started.
struct Answer {
    tx: mpsc::UnboundedSender<ReplyEnvelope>,
    stream: Stream,
    last_word: LastWord,
    /// MOD-24 D4: a fresh chat's `run(kind='chat')` pair, closed `failed` before the last word is
    /// sent, so a tab that re-reads runs on `Ended` finds it closed. `None` for every other task,
    /// and for a promoted step's chat, which opened no run (MOD-4 D165). The flag is the
    /// binding's (review L3): raised, the session closed the run itself and it is left alone.
    closes: Option<(Writer, ChatRunSpec, Arc<AtomicBool>)>,
}

/// MOD-24 review L2: how long a panicked chat's answer waits for its run's close before its last
/// word goes out anyway. A stalled store must not leave the tab's pending start set for ever.
const PANICKED_CHAT_CLOSE: Duration = Duration::from_secs(5);

impl Answer {
    /// An answer at a fixed `addr`, for a task that sends through something other than [`Frames`].
    fn at(tx: mpsc::UnboundedSender<ReplyEnvelope>, addr: ReplyAddr, last_word: LastWord) -> Self {
        Frames::new(tx, addr).answer(last_word)
    }

    /// MOD-24 D4: this answer closes `chat`'s run first, unless `closed` says the session already
    /// did (review L3). Copies, not the task's state: the task is dropped before the answer is
    /// sent.
    fn closing(mut self, writer: Writer, chat: ChatRunSpec, closed: Arc<AtomicBool>) -> Self {
        self.closes = Some((writer, chat, closed));
        self
    }

    /// Closes the run this answer owns, if any and if the session has not, within
    /// [`PANICKED_CHAT_CLOSE`]; then sends the last word for `message` at the stream's address as
    /// it is now.
    async fn send(self, message: String) {
        // Awaited before the frames go out: a tab that re-reads runs on `Ended` must find the run
        // closed. Bounded, so a stalled store delays the last word but never withholds it (review
        // L2). `close_run` logs its own failure.
        if let Some((writer, chat, closed)) = &self.closes {
            if closed.load(Ordering::Acquire) {
                tracing::debug!(
                    run = %chat.run_id,
                    "a chat panicked after its run was closed; the close stands"
                );
            } else if tokio::time::timeout(
                PANICKED_CHAT_CLOSE,
                close_run(writer, chat, RunStatus::Failed),
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    run = %chat.run_id,
                    limit_secs = PANICKED_CHAT_CLOSE.as_secs(),
                    "closing a panicked chat's run timed out; its last word goes out regardless"
                );
            }
        }
        let addr = self.stream.lock().addr.clone();
        for reply in (self.last_word)(message) {
            // A UI that has gone away is not an error, as in `Frames::send`.
            let _ = self.tx.send(ReplyEnvelope {
                seq: addr.seq,
                origin: addr.origin.clone(),
                reply,
            });
        }
    }
}

impl Frames {
    /// What a panic in the task that owns these frames answers with, at their stream (MOD-53).
    fn answer(&self, last_word: LastWord) -> Answer {
        Answer {
            tx: self.tx.clone(),
            stream: self.stream.clone(),
            last_word,
            closes: None,
        }
    }
}

/// A panicked agent probe's last word: the `probe_agents` failure that clears the Agents
/// section's `probing` (MOD-53).
fn probe_agents_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Failed {
        request: "probe_agents",
        message,
    }]
}

/// A panicked `SetToolPaths`'s last word: the failure that clears the Agents section's `busy`
/// (MOD-53's shape, MOD-66 D10).
fn tool_paths_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Failed {
        request: SET_TOOL_PATHS,
        message,
    }]
}

/// A panicked `ProbeBox`'s last word: the `probe_box` failure its refusals already use, which
/// clears the Boxes section's `probing` and puts the sentence on its notice line (MOD-53).
fn box_probe_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Failed {
        request: "probe_box",
        message,
    }]
}

/// A panicked registration probe's last word: a report with `box_failed` set. It answers at
/// `UNSOLICITED`, where the freshness gate drops a `Failed` unread; a `BoxProbed` is rendered above
/// the gate (`App::observe_reply`).
fn registration_probe_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::BoxProbed(BoxProbeReport {
        box_failed: Some(message),
        ..BoxProbeReport::default()
    })]
}

/// A panicked prompt preview's last word.
fn preview_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Failed {
        request: crate::store_worker::PROMPT_PREVIEW,
        message,
    }]
}

/// A panicked plan's or install's last word: the terminal frame that closes the section's pane.
fn install_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Install(InstallFrame::Failed {
        message,
        manual: None,
    })]
}

/// A panicked login's last word: the terminal frame that returns the section to idle. Sent at the
/// `AuthStart`'s address: `run_auth` moves its stream to an `AuthChoose`'s in a local, not in
/// `Frames`, and the start's `seq` stays fresh for the whole flow because a choose is another
/// request kind.
fn auth_failed(message: String) -> Vec<StoreReply> {
    vec![StoreReply::Auth(AuthFrame::Failed { message })]
}

/// A panicked chat's last words, as its transport failures end it: `Failed`, which clears a
/// pending start, then `Ended`, which tells an accepted session it is over so the tab stops
/// sending to a chat the runtime has already swept. `Cancelled`, because the last turn was cut.
/// Sent after the chat's run is closed `failed` (MOD-24 D4).
fn chat_failed(message: String) -> Vec<StoreReply> {
    vec![
        StoreReply::Chat(ChatFrame::Failed { message }),
        StoreReply::Chat(ChatFrame::Ended {
            stop_reason: StopReason::Cancelled,
        }),
    ]
}

/// `task`, polled so that a panic inside it ends the request instead of the terminal (MOD-53).
///
/// Every task the runtime spawns goes through here. A panic used to be dropped twice over: tokio
/// kept it in a `JoinHandle` that [`sweep_finished`](AgentRuntime::sweep_finished) forgets
/// unread, so the flag the UI raised for the request (`probing`, a running install or login, a
/// chat's pending start) stayed set for the rest of the session; and the process hook
/// (`terminal::install_panic_hook`) ran first and gave the terminal back under a live event loop,
/// because nothing marked the panic as one the process survives.
///
/// So each poll runs inside [`contain`](htui_agent::excerpt::contain) and `catch_unwind`. On an
/// unwind the task is dropped, which runs whatever guards it still held (a `ChildGuard` kills its
/// child, a `ReprobeClaim` releases its row), the panic is logged, a fresh chat's run is closed
/// unless its session already closed it (MOD-24 D4, review L3), and `answer`, when the task owes
/// one, is sent. The reply goes out as soon as that close has answered, or after
/// [`PANICKED_CHAT_CLOSE`] at the latest (review L2), and not at the next sweep, because the
/// sweep only runs when another request arrives. Cancelling is untouched: an aborted task is
/// dropped at an await and never reaches the catch, so a superseded preview or a shutdown still
/// answers nothing.
///
/// A panic on a thread the task starts (a blocking thread, a spawned task, a provider thread) is
/// not caught here; it comes back to the task as an error, as it always did. It does not reach the
/// terminal either: every blocking thread and task `htui_agent` starts opens its own window
/// through [`htui_agent::contained`] (MOD-65), and a provider thread opens one in `run_providers`.
async fn answering<F>(name: &'static str, task: F, answer: Option<Answer>)
where
    F: Future<Output = ()>,
{
    let mut task = Box::pin(task);
    let caught = std::future::poll_fn(|cx| {
        htui_agent::excerpt::contain(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.as_mut().poll(cx)))
        })
        .map_or_else(
            |payload| std::task::Poll::Ready(Err(payload)),
            |poll| poll.map(Ok),
        )
    })
    .await;
    // Before the answer: whatever the task still held is let go of first, so a request the answer
    // prompts finds the child reaped and the claim free.
    drop(task);
    let Err(payload) = caught else {
        return;
    };
    let message = format!("the {name} task panicked: {}", panic_text(payload.as_ref()));
    tracing::error!(task = name, %message, "a runtime task panicked; its request is answered as failed");
    if let Some(answer) = answer {
        answer.send(message).await;
    }
}

/// A panic payload as text: the `&str` or `String` `panic!` builds, else a placeholder.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a non-text payload")
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
        binding,
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
    let step_id = binding.step_id();

    // MOD-37 M5 (blueprint D5, H-6): one recorder, built before the first start, so a
    // `resume_failed` row and the handoff's `follow_up` after it come from the same continuing
    // recorder: a second one over the stale tail would reuse its `seq`. A start that fails drops a
    // recorder that wrote nothing (`Recorder` has no `Drop` side effect).
    let (ui_tx, mut ui_rx) = mpsc::channel(UI_FRAMES);
    let retain_raw = std::env::var(KEEP_RAW_ENV).is_ok_and(|value| value == "1");
    let mut recorder = match &binding {
        ChatBinding::Fresh(..) => {
            Recorder::new(&writer, &scrubber, step_id, retain_raw, Some(ui_tx))
        }
        // Plan D164: the step's log goes on past its last row, at its next turn, with its
        // pre-promotion spend in the running total and its prompt digest left alone.
        ChatBinding::Promoted { tail, .. } => {
            Recorder::continuing(&writer, &scrubber, step_id, retain_raw, Some(ui_tx), tail)
        }
    };
    // Two opt-in builders rather than two more `new` parameters, because most recorders in this
    // tree have neither (plan D66-D68, D70). The grace the cap's cancel takes is this runtime's
    // own `CANCEL_GRACE`, riding on `RunCap` so `htui_agent::record::pump` keeps its signature.
    recorder = recorder.with_quota_latch(quota_latch);
    if let Some(micros) = project_caps.run_micros {
        recorder = recorder.with_run_cap(RunCap { micros, grace });
    }

    // The opening is a resume exactly when the spec resumes; a fallback is optional (review M-1).
    let resuming = spec.resume.is_some();
    let first = driver.start(spec.clone(), prompt.clone()).await;
    let fallback = match &binding {
        ChatBinding::Promoted { fallback, .. } => fallback.clone(),
        ChatBinding::Fresh(..) => None,
    };
    // MOD-37 M5: a promoted resume that fails is reported, then the chat opens with the handoff
    // in the same bind. `Ok` carries the session, the text recorded as its opening, how it opened
    // and the notice owed the tab; `Err` the refusal and that notice.
    let started = match (first, fallback) {
        (Ok(session), _) => {
            let opening = if resuming {
                StepOpening::Resumed
            } else {
                StepOpening::Handoff
            };
            Ok((session, prompt, opening, None))
        }
        (Err(err), Some(fallback)) if falls_back(&err) => {
            let at = Utc::now();
            let notice = resume_failed_notice(&fallback.session_ref, &err.to_string());
            // The column first, so the Runs pane is truthful even if the row write fails.
            record_opening(&writer, step_id, StepOpening::ResumeFailed).await;
            // The row, at the step's current turn, before a second session can write. The tab is
            // sent what was written, scrubbed (review L-1); a row that could not be written has
            // no frame either, so the live view and a replay agree.
            let envelope = match recorder.record_notice(&notice, at).await {
                Ok(written) => Some(written),
                Err(record_err) => {
                    tracing::error!(%record_err, "the resume_failed row could not be written");
                    None
                }
            };
            // The second start: no resume, the handoff text.
            let handoff_spec = SessionSpec {
                resume: None,
                ..spec
            };
            match driver.start(handoff_spec, fallback.handoff.clone()).await {
                Ok(session) => Ok((
                    session,
                    fallback.handoff,
                    StepOpening::ResumeFailed,
                    envelope,
                )),
                Err(err) => Err((err, envelope)),
            }
        }
        (Err(err), _) => Err((err, None)),
    };

    let (mut session, opening_text, opening, notice) = match started {
        Ok(started) => started,
        Err((err, notice)) => {
            // MOD-37 M5 (H-7): the report reaches the tab before the refusal.
            if let Some(notice) = notice {
                frames.event(notice);
            }
            let message = err.to_string();
            frames.to_stream(StoreReply::Failed {
                request: binding.request(),
                message: message.clone(),
            });
            binding.close(&writer, RunStatus::Failed).await;
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

    frames.accept(StoreReply::ChatAccepted {
        step_id,
        session_ref: session.session_ref().cloned(),
        caps,
    });
    // MOD-37 M5 (H-8): a promoted chat's opening, written only once its start succeeded.
    // `resume_failed` was written before the fallback start; a fresh chat records none.
    if matches!(binding, ChatBinding::Promoted { .. }) && opening != StepOpening::ResumeFailed {
        record_opening(&writer, step_id, opening).await;
    }
    // H-7: after the acceptance, before the opening.
    if let Some(envelope) = notice {
        frames.event(envelope);
    }

    let now = Utc::now();
    match &binding {
        ChatBinding::Fresh(..) => {
            if let Err(err) = recorder
                .record_prompt(&opening_text, prompt_sections(), now)
                .await
            {
                tracing::error!(%err, "the prompt row could not be written");
            }
            frames.local("prompt", json!({ "text": opening_text }), now);
        }
        // ANA-5 criterion 18: the opening — the handoff prompt, or the resume sentence — is the
        // step's next `follow_up`, never a second `prompt` (which would rewrite the digest).
        ChatBinding::Promoted { .. } => {
            if let Err(err) = recorder.record_follow_up(&opening_text, now).await {
                tracing::error!(%err, "the opening's follow-up row could not be written");
            }
            frames.event(follow_up_frame(&opening_text, now));
        }
    }

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
    binding.close(&writer, status).await;
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
        event: DriverEvent::Other(OtherEvent {
            update: "permission_answer".to_owned(),
            // The **same** key set the recorder writes, `denied` included (D94). A live frame and
            // the row it replays as are read by one function
            // (`chat/transcript.rs::resolve_permission`), so a key present in one and absent in the
            // other is a transcript that renders differently depending on whether you are watching
            // it or reopening it — which is the asymmetry D94 was raised to close, in its other
            // direction.
            //
            // The expression is `record_permission_answer`'s own: `htui` authors exactly two
            // answers, a picked option and a cancellation, so neither is ever a refusal. It is
            // written out rather than defaulted so that the day a third kind of answer exists, this
            // reads as a claim to re-check instead of as a `false` nobody chose.
            body: json!({
                "request_id": request_id.as_str(),
                "option_id": option_id,
                "by": by.as_str(),
                "cancelled": cancelled,
                "denied": option_id.is_none() && !cancelled,
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

/// MOD-37 M5: whether a failed resume is worth a handoff start (review L-2): only when it failed on
/// the wire (`Transport`, `Closed`), which is what an agent that lost or refused the session
/// answers. Not when the adapter cannot run at all: a missing or unspawnable command (`Spawn`), an
/// unresolved launch placeholder (`Unresolved`) or no transport (`UnknownAdapter`) would fail the
/// handoff start the same way (blueprint A-2, H-18). Not on a transport that cannot restore
/// (`Unsupported`), a cancel (`Cancelled`), or the driver's own write or scrub (`Store`, `Scrub`),
/// which are not the resume's to fall back from. Exhaustive, with no wildcard: a new variant is a
/// decision, not a default.
const fn falls_back(err: &DriverError) -> bool {
    match err {
        DriverError::Transport(_) | DriverError::Closed => true,
        DriverError::Unresolved(_)
        | DriverError::UnknownAdapter(_)
        | DriverError::Spawn(_)
        | DriverError::Unsupported(_)
        | DriverError::Cancelled
        | DriverError::Store(_)
        | DriverError::Scrub(_) => false,
    }
}

/// MOD-37 M5: the `resume_failed` notice: the session tried, why it failed, and what the chat
/// opens with instead.
fn resume_failed_notice(session_ref: &AgentSessionRef, reason: &str) -> OtherEvent {
    OtherEvent {
        update: htui_agent::event::RESUME_FAILED.to_owned(),
        body: json!({
            "session_id": session_ref.as_str(),
            "reason": reason,
            "note": htui_orch::promote::CONTEXT_NOT_CARRIED,
        }),
    }
}

/// MOD-37 M5: `run_step.opening`, written through the bind's writer. A failed write is logged and
/// never fails the chat: the column is a label, and the session is what the user is waiting on.
async fn record_opening(writer: &Writer, step: StepId, opening: StepOpening) {
    if let Err(err) = writer.record_opening(step, opening).await {
        tracing::warn!(%err, %step, %opening, "the chat's opening could not be recorded");
    }
}

/// The frame a follow-up produces, shaped as the `other` row the tab renders.
fn follow_up_frame(text: &str, at: DateTime<Utc>) -> DriverEnvelope {
    DriverEnvelope {
        event: DriverEvent::Other(OtherEvent {
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

/// Closes the chat's `run` / `run_step` pair, so it stops counting as an active run. Whether the
/// close landed; a failure is logged here.
async fn close_run(writer: &Writer, chat: &ChatRunSpec, status: RunStatus) -> bool {
    match writer
        .finish_chat_run(chat.run_id, chat.step_id, status, Utc::now())
        .await
    {
        Ok(()) => true,
        Err(err) => {
            tracing::error!(%err, "the chat run could not be closed");
            false
        }
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
    use htui_core::fixtures::{edit_agent, ids};
    use htui_core::model::{
        Agent, AgentId, EventKind, EventRole, RunId, Scope, StepStatus, Transport,
    };
    use htui_core::store::MemStore;
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
            env: BTreeMap::new(),
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
            .upsert_agent(&fake_row(agent_id), None)
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

    /// Where [`FailingStarts`] writes down every `(spec, prompt)` it was started with.
    type StartLog = Arc<Mutex<Vec<(SessionSpec, String)>>>;

    /// The errors [`FailingStarts`] fails its next starts with, front first.
    type Failures = Arc<Mutex<std::collections::VecDeque<DriverError>>>;

    /// MOD-37 M5: the fake driver, failing its first starts with `failures` (front first) and
    /// writing down every `(spec, prompt)` it was started with.
    #[derive(Debug)]
    struct FailingStarts {
        inner: Box<dyn AgentDriver>,
        starts: StartLog,
        failures: Failures,
    }

    impl AgentDriver for FailingStarts {
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
            // Both locks released here, as in `SpecSpy::start`: the future below is `Send`.
            self.starts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((spec.clone(), prompt.clone()));
            let failure = self
                .failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front();
            if let Some(err) = failure {
                return Box::pin(async move { Err(err) });
            }
            self.inner.start(spec, prompt)
        }
    }

    /// [`FakeBuilder`]'s twin for [`FailingStarts`]: one queue of failures and one start log
    /// shared by every driver the factory builds.
    #[derive(Debug)]
    struct FailingBuilder {
        adapter: Arc<FakeAdapter>,
        starts: StartLog,
        failures: Failures,
    }

    impl htui_agent::registry::TransportBuilder for FailingBuilder {
        fn build(
            &self,
            agent: &Agent,
            on_box: Option<&AgentBox>,
            caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            Ok(Box::new(FailingStarts {
                inner: self.adapter.build(agent, on_box, caps)?,
                starts: Arc::clone(&self.starts),
                failures: Arc::clone(&self.failures),
            }))
        }
    }

    /// MOD-37 M5: [`fixture_with_spec_spy`]'s shape over [`FailingStarts`], whose first starts
    /// fail with `failures`.
    async fn fixture_with_failing_starts(
        script: Script,
        failures: Vec<DriverError>,
    ) -> (MemStore, Backend, AgentRuntime, AgentId, StartLog) {
        let store = MemStore::from_demo(htui_core::fixtures::demo_data());
        let agent_id = AgentId::new();
        store
            .upsert_agent(&fake_row(agent_id), None)
            .await
            .expect("the fake row lands");

        let adapter = Arc::new(FakeAdapter::new());
        adapter.load(script);
        let starts: StartLog = Arc::new(Mutex::new(Vec::new()));
        let mut factory = DriverFactory::new();
        factory.register(
            "cli/fake",
            Box::new(FailingBuilder {
                adapter,
                starts: Arc::clone(&starts),
                failures: Arc::new(Mutex::new(failures.into())),
            }),
        );

        let backend = Backend::memory(store.clone());
        let runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));
        (store, backend, runtime, agent_id, starts)
    }

    /// The starts [`FailingStarts`] saw, in order.
    fn starts_of(starts: &StartLog) -> Vec<(SessionSpec, String)> {
        starts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    // A promoted graph step (MOD-4 plan D164, D165, blueprint D205, D206)
    // -----------------------------------------------------------------------------------------

    /// The fixture's `plan` step of `RUN_1`, promoted: the one step with a cached log, so the
    /// continuing recorder has a tail to start past.
    fn promoted(agent_id: AgentId, path: OpeningPath) -> crate::run_worker::Promoted {
        crate::run_worker::Promoted {
            run: ids::RUN_1,
            step: ids::STEP_PLAN,
            project: ids::PROJECT_HTUI,
            opening: htui_orch::Opening {
                agent_id,
                agent_name: "fake".to_owned(),
                model: None,
                phase: "plan".to_owned(),
                cwd: std::env::temp_dir(),
                extra_dirs: vec![std::env::temp_dir().join("second-tree")],
                path,
            },
        }
    }

    /// The promotion's address: the Chat tab's `PromoteStep`, at seq 7.
    fn promote_addr() -> ReplyAddr {
        ReplyAddr {
            seq: 7,
            origin: Origin::Tab(crate::ui::tabs::TabId("chat")),
        }
    }

    /// Attaches `promoted`, queues the user's `Esc Esc` before the session is polled, and drives
    /// it to its end: [`run`]'s shape for a promotion.
    async fn attach_and_end(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        promoted: crate::run_worker::Promoted,
        between: impl AsyncFnOnce(StepId),
    ) -> Vec<ReplyEnvelope> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let Served::Start { step_id, task } = runtime
            .attach_promoted(backend, &tx, promote_addr(), promoted)
            .await
        else {
            panic!("a promotion over a registered row opens a session")
        };
        between(step_id).await;
        let cancel = runtime
            .serve(
                backend,
                &tx,
                &envelope(8, StoreRequest::ChatCancel { step_id }),
            )
            .await;
        assert!(matches!(cancel, Served::Deferred), "{cancel:?}");
        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }
        replies
    }

    /// Blueprint D192, R-48: a CLI step with a `session_started` banner is resumed — the spec the
    /// driver starts carries the banner's session ref, the step's own id and the step's trees —
    /// and the chat records against that step, past its log.
    #[tokio::test]
    async fn attach_promoted_resumes_a_cli_step_with_its_banner() {
        let (store, backend, mut runtime, agent_id, spec) =
            fixture_with_spec_spy(Script::one_turn(vec![ends(StopReason::EndTurn)]), None).await;
        let tail = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("the fixture's step has a log");
        let promotion = promoted(
            agent_id,
            OpeningPath::Resume {
                session_ref: AgentSessionRef::new("banner-1"),
                text: htui_orch::promote::RESUME_OPENING.to_owned(),
                fallback: Some(htui_orch::HandoffText {
                    text: "the handoff".to_owned(),
                    digest: "d".to_owned(),
                }),
            },
        );

        let replies = attach_and_end(&mut runtime, &backend, promotion, async |_| {}).await;

        let seen = spec
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("the driver was started");
        assert_eq!(seen.resume, Some(AgentSessionRef::new("banner-1")));
        assert_eq!(seen.step_id, ids::STEP_PLAN, "the step keeps its id");
        assert_eq!(seen.cwd, std::env::temp_dir());
        assert_eq!(
            seen.extra_dirs,
            vec![std::env::temp_dir().join("second-tree")]
        );
        assert!(
            matches!(replies[0].reply, StoreReply::ChatAccepted { step_id, .. } if step_id == ids::STEP_PLAN)
                && replies[0].seq == 7,
            "the promotion's own address hears the acceptance: {:?}",
            replies[0]
        );

        let log = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("a log");
        let follow_up = log
            .iter()
            .find(|row| row.kind == EventKind::FollowUp)
            .expect("the opening is recorded");
        let last = tail.iter().map(|row| row.seq).max().expect("a tail");
        assert_eq!(
            follow_up.seq,
            last + 1,
            "the log continues past its last row"
        );
        assert_eq!(follow_up.turn, 1, "the opening opens the step's next turn");
        assert_eq!(
            follow_up.payload.get("text").and_then(Value::as_str),
            Some(htui_orch::promote::RESUME_OPENING)
        );
        assert_eq!(
            opening_of(&store, ids::STEP_PLAN).await,
            Some(StepOpening::Resumed),
            "a resume that started is recorded as one (MOD-37 M5)"
        );
        assert!(
            !log.iter().any(is_resume_failed),
            "and no `resume_failed` row is written: {log:?}"
        );
    }

    /// MOD-37 M5: `run_step.opening` of the fixture's step, read through the item's run summary,
    /// the read the Runs pane makes.
    async fn opening_of(store: &MemStore, step: StepId) -> Option<StepOpening> {
        let item = store
            .run(ids::RUN_1)
            .await
            .expect("the read answers")
            .expect("the fixture's run")
            .item_id
            .expect("the fixture's run is an item's");
        store
            .runs(item)
            .await
            .expect("the read answers")
            .into_iter()
            .find(|run| run.id == ids::RUN_1)
            .expect("the fixture's run is the item's")
            .steps
            .into_iter()
            .find(|summary| summary.id == step)
            .expect("the fixture's step is the run's")
            .opening
    }

    /// Whether `row` is `htui`'s `resume_failed` notice.
    fn is_resume_failed(row: &SessionEvent) -> bool {
        row.kind == EventKind::Other
            && row.payload.get("update").and_then(Value::as_str)
                == Some(htui_agent::event::RESUME_FAILED)
    }

    /// Whether `reply` is the tab's copy of the `resume_failed` notice.
    fn is_resume_failed_frame(reply: &ReplyEnvelope) -> bool {
        matches!(
            &reply.reply,
            StoreReply::Chat(ChatFrame::Event(envelope))
                if matches!(&envelope.event, DriverEvent::Other(other)
                    if other.update == htui_agent::event::RESUME_FAILED)
        )
    }

    /// Whether `reply` is the tab's copy of a `follow_up` row.
    fn is_follow_up_frame(reply: &ReplyEnvelope) -> bool {
        matches!(
            &reply.reply,
            StoreReply::Chat(ChatFrame::Event(envelope))
                if matches!(&envelope.event, DriverEvent::Other(other) if other.update == "follow_up")
        )
    }

    /// MOD-37 M5: the opening a resume would try, with its handoff fallback.
    fn resume_path() -> OpeningPath {
        OpeningPath::Resume {
            session_ref: AgentSessionRef::new("banner-1"),
            text: htui_orch::promote::RESUME_OPENING.to_owned(),
            fallback: Some(htui_orch::HandoffText {
                text: "HANDOFF TEXT".to_owned(),
                digest: "d".to_owned(),
            }),
        }
    }

    /// MOD-37 review M-1: a resume whose handoff the engine could not build.
    fn resume_path_without_fallback() -> OpeningPath {
        OpeningPath::Resume {
            session_ref: AgentSessionRef::new("banner-1"),
            text: htui_orch::promote::RESUME_OPENING.to_owned(),
            fallback: None,
        }
    }

    /// Attaches `promoted` and drives its task to the end without a cancel: for a chat that fails
    /// to start, which [`attach_and_end`]'s `ChatCancel` would find already gone.
    async fn attach_and_await(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        promoted: crate::run_worker::Promoted,
    ) -> Vec<ReplyEnvelope> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let Served::Start { task, .. } = runtime
            .attach_promoted(backend, &tx, promote_addr(), promoted)
            .await
        else {
            panic!("a promotion over a registered row opens a session")
        };
        task.await;
        drop(tx);
        let mut replies = Vec::new();
        while let Some(reply) = rx.recv().await {
            replies.push(reply);
        }
        replies
    }

    /// MOD-37 M5 (a): a handoff opening starts once, with no resume, and records `handoff`.
    #[tokio::test]
    async fn a_handoff_promotion_records_handoff() {
        let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            Vec::new(),
        )
        .await;
        let promotion = promoted(
            agent_id,
            OpeningPath::Handoff {
                text: "pick up where the step stopped".to_owned(),
                digest: "d-handoff".to_owned(),
            },
        );

        let replies = attach_and_end(&mut runtime, &backend, promotion, async |_| {}).await;

        let starts = starts_of(&starts);
        assert_eq!(starts.len(), 1, "one start: {starts:?}");
        assert_eq!(starts[0].0.resume, None, "a handoff resumes nothing");
        assert!(
            !replies.iter().any(is_resume_failed_frame),
            "no notice: {replies:?}"
        );
        assert_eq!(
            opening_of(&store, ids::STEP_PLAN).await,
            Some(StepOpening::Handoff)
        );
    }

    /// MOD-37 M5 (c): a resume that fails is reported, then the chat opens with the handoff in
    /// the same bind. The notice is `htui`'s `other` row in the step's current turn, the handoff
    /// is the next turn's `follow_up`, and the tab hears the notice after the acceptance and
    /// before the opening.
    #[tokio::test]
    async fn a_failed_resume_reports_then_opens_the_handoff() {
        let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            vec![DriverError::Transport(
                "session/load failed: no such session".to_owned(),
            )],
        )
        .await;
        let tail = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("the fixture's step has a log");
        let last_seq = tail.iter().map(|row| row.seq).max().expect("a tail");
        let last_turn = tail.iter().map(|row| row.turn).max().expect("a tail");

        let replies = attach_and_end(
            &mut runtime,
            &backend,
            promoted(agent_id, resume_path()),
            async |_| {},
        )
        .await;

        let starts = starts_of(&starts);
        assert_eq!(starts.len(), 2, "the resume, then the handoff: {starts:?}");
        let (first, first_prompt) = &starts[0];
        let (second, second_prompt) = &starts[1];
        assert_eq!(first.resume, Some(AgentSessionRef::new("banner-1")));
        assert_eq!(first_prompt, htui_orch::promote::RESUME_OPENING);
        assert_eq!(second.resume, None, "the fallback resumes nothing");
        assert_eq!(second_prompt, "HANDOFF TEXT");
        assert_eq!(second.step_id, first.step_id, "the same step");
        assert_eq!(second.cwd, first.cwd, "in the same tree");
        assert_eq!(second.extra_dirs, first.extra_dirs);

        let log = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("a log");
        let past: Vec<&SessionEvent> = log.iter().filter(|row| row.seq > last_seq).collect();
        assert!(past.len() >= 2, "the notice and the opening: {past:?}");
        let notice = past[0];
        assert!(
            is_resume_failed(notice),
            "the notice comes first: {notice:?}"
        );
        assert_eq!(
            (notice.seq, notice.turn),
            (last_seq + 1, last_turn),
            "in the step's current turn"
        );
        assert_eq!(notice.role, EventRole::Htui, "`htui` authors the notice");
        let body = notice.payload.get("body").expect("a body");
        assert_eq!(body.get("session_id"), Some(&json!("banner-1")));
        assert!(
            body.get("reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| reason.contains("no such session")),
            "the reason quotes the failure: {body}"
        );
        assert_eq!(
            body.get("note").and_then(Value::as_str),
            Some(htui_orch::promote::CONTEXT_NOT_CARRIED)
        );
        let follow_up = past[1];
        assert_eq!(follow_up.kind, EventKind::FollowUp);
        assert_eq!(
            (follow_up.seq, follow_up.turn),
            (last_seq + 2, last_turn + 1),
            "the handoff opens the next turn"
        );
        assert_eq!(
            follow_up.payload.get("text").and_then(Value::as_str),
            Some("HANDOFF TEXT")
        );
        assert_eq!(
            opening_of(&store, ids::STEP_PLAN).await,
            Some(StepOpening::ResumeFailed)
        );

        let accepted = replies
            .iter()
            .position(|reply| {
                matches!(reply.reply, StoreReply::ChatAccepted { .. }) && reply.seq == 7
            })
            .unwrap_or_else(|| panic!("the chat was accepted: {replies:?}"));
        let reported = replies
            .iter()
            .position(is_resume_failed_frame)
            .unwrap_or_else(|| panic!("the tab hears the notice: {replies:?}"));
        let opened = replies
            .iter()
            .position(is_follow_up_frame)
            .unwrap_or_else(|| panic!("the tab hears the opening: {replies:?}"));
        assert!(
            accepted < reported && reported < opened,
            "acceptance, notice, opening: {replies:?}"
        );
        assert!(
            !replies
                .iter()
                .any(|reply| matches!(reply.reply, StoreReply::Failed { .. })),
            "nothing failed: {replies:?}"
        );
    }

    /// MOD-37 M5 (d): a fallback whose own start fails fails the chat as before, after the notice.
    /// The step keeps `resume_failed` and its notice row, and has no opening.
    #[tokio::test]
    async fn a_failed_resume_whose_handoff_fails_too_fails_the_chat() {
        let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            vec![
                DriverError::Transport("a".to_owned()),
                DriverError::Transport("second refusal".to_owned()),
            ],
        )
        .await;
        let tail_len = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("the fixture's step has a log")
            .len();

        let replies =
            attach_and_await(&mut runtime, &backend, promoted(agent_id, resume_path())).await;

        assert_eq!(starts_of(&starts).len(), 2, "the resume, then the handoff");
        let reported = replies
            .iter()
            .position(is_resume_failed_frame)
            .unwrap_or_else(|| panic!("the tab hears the notice: {replies:?}"));
        let refused = replies
            .iter()
            .position(|reply| {
                matches!(&reply.reply, StoreReply::Failed { request, message }
                    if *request == PROMOTE_STEP && message.contains("second refusal"))
            })
            .unwrap_or_else(|| panic!("the promotion is refused: {replies:?}"));
        let ended = replies
            .iter()
            .position(|reply| matches!(reply.reply, StoreReply::Chat(ChatFrame::Failed { .. })))
            .unwrap_or_else(|| panic!("the chat failed: {replies:?}"));
        assert!(
            reported < refused && refused < ended,
            "notice, refusal, failure: {replies:?}"
        );

        let log = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("a log");
        assert_eq!(
            log.len(),
            tail_len + 1,
            "the notice and nothing else: {log:?}"
        );
        assert!(log.iter().any(is_resume_failed));
        assert!(
            !log.iter().any(|row| row.kind == EventKind::FollowUp),
            "no opening was recorded"
        );
        assert_eq!(
            opening_of(&store, ids::STEP_PLAN).await,
            Some(StepOpening::ResumeFailed)
        );
    }

    /// MOD-37 review L-1: the tab's `resume_failed` frame is the row the recorder wrote, scrubbed,
    /// never the raw notice. The failure reason quotes a secret from the spec's env (the one the
    /// chat's scrubber masks); neither the stored row nor the frame carries it. Driven through
    /// [`run_chat`] itself, because a promotion's spec carries no env until MOD-10.
    #[tokio::test]
    async fn the_resume_failed_frame_is_the_scrubbed_row() {
        const SECRET: &str = "hunter2-env-secret-value";
        let (store, backend, runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            vec![
                DriverError::Transport(format!("session/load failed: token {SECRET} rejected")),
                DriverError::Transport("second refusal".to_owned()),
            ],
        )
        .await;
        let agent = fake_row(agent_id);
        let driver = runtime
            .factory
            .driver_for(&agent, None)
            .expect("the fake row builds a driver");
        let writer = backend.writer().expect("a memory backend writes");
        let tail = writer
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("the fixture's step has a log");
        let tail_len = tail.len();
        let box_id = registered_box(&backend).await.expect("the box registers");
        let settings = AgentSettings::default();
        let session_ref = AgentSessionRef::new("banner-1");
        let spec = SessionSpec {
            agent_id,
            step_id: ids::STEP_PLAN,
            cwd: std::env::temp_dir(),
            extra_dirs: Vec::new(),
            env: BTreeMap::from([("API_TOKEN".to_owned(), SECRET.to_owned())]),
            model: None,
            tools: htui_agent::driver::ToolExposure::default(),
            mcp: Vec::new(),
            permission: settings.permission.clone(),
            retain_raw: false,
            resume: Some(session_ref.clone()),
            budget_micros: None,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_commands_tx, commands) = mpsc::unbounded_channel();
        let caps = driver.caps();
        run_chat(ChatArgs {
            driver,
            writer,
            binding: ChatBinding::Promoted {
                step_id: ids::STEP_PLAN,
                tail,
                fallback: Some(ResumeFallback {
                    session_ref,
                    handoff: "HANDOFF TEXT".to_owned(),
                }),
            },
            spec,
            prompt: htui_orch::promote::RESUME_OPENING.to_owned(),
            policy: settings.permission,
            caps,
            commands,
            frames: Frames::new(tx, promote_addr()),
            grace: Duration::from_millis(0),
            reprobe: None,
            project_caps: project_caps_for(
                ids::PROJECT_HTUI,
                backend
                    .project_settings(ids::PROJECT_HTUI)
                    .await
                    .expect("the settings read"),
            )
            .expect("the demo project's caps"),
            quota_latch: quota_latch_for(&agent, box_id, settings.quota.source),
        })
        .await;
        let mut replies = Vec::new();
        while let Ok(reply) = rx.try_recv() {
            replies.push(reply);
        }

        assert_eq!(starts_of(&starts).len(), 2, "the resume, then the handoff");
        let log = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("a log");
        assert_eq!(log.len(), tail_len + 1, "the notice: {log:?}");
        let row = log
            .iter()
            .find(|row| is_resume_failed(row))
            .expect("the notice row");
        assert!(
            !row.payload.to_string().contains(SECRET),
            "the row is scrubbed: {row:?}"
        );
        let frame = replies
            .iter()
            .find(|reply| is_resume_failed_frame(reply))
            .unwrap_or_else(|| panic!("the tab hears the notice: {replies:?}"));
        assert!(
            !format!("{frame:?}").contains(SECRET),
            "the frame is scrubbed: {frame:?}"
        );
        let StoreReply::Chat(ChatFrame::Event(envelope)) = &frame.reply else {
            panic!("an event frame: {frame:?}");
        };
        assert_eq!(
            htui_agent::replay::envelope_from_row(row)
                .expect("the notice replays")
                .event,
            envelope.event,
            "the frame is the row the tab would replay"
        );
    }

    /// MOD-37 review L-2: the two wire failures, `Transport` and `Closed`, fall back: two starts,
    /// the notice row and frame, and `resume_failed`.
    #[tokio::test]
    async fn a_resume_that_fails_on_the_wire_falls_back() {
        for failure in [
            DriverError::Transport("session/load failed: gone".to_owned()),
            DriverError::Closed,
        ] {
            let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
                Script::one_turn(vec![ends(StopReason::EndTurn)]),
                vec![failure.clone()],
            )
            .await;

            let replies = attach_and_end(
                &mut runtime,
                &backend,
                promoted(agent_id, resume_path()),
                async |_| {},
            )
            .await;

            assert_eq!(starts_of(&starts).len(), 2, "{failure:?}: two starts");
            assert!(
                replies.iter().any(is_resume_failed_frame),
                "{failure:?}: the notice: {replies:?}"
            );
            let log = store
                .step_events(ids::STEP_PLAN)
                .await
                .expect("the log reads")
                .expect("a log");
            assert!(log.iter().any(is_resume_failed), "{failure:?}: the row");
            assert_eq!(
                opening_of(&store, ids::STEP_PLAN).await,
                Some(StepOpening::ResumeFailed),
                "{failure:?}"
            );
        }
    }

    /// MOD-37 review M-1: a resume with no fallback that starts is still recorded as `resumed`:
    /// the label is the opening's, not the fallback's.
    #[tokio::test]
    async fn a_resume_with_no_fallback_that_starts_records_resumed() {
        let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            Vec::new(),
        )
        .await;

        let replies = attach_and_end(
            &mut runtime,
            &backend,
            promoted(agent_id, resume_path_without_fallback()),
            async |_| {},
        )
        .await;

        let starts = starts_of(&starts);
        assert_eq!(starts.len(), 1, "one start: {starts:?}");
        assert_eq!(starts[0].0.resume, Some(AgentSessionRef::new("banner-1")));
        assert!(
            !replies.iter().any(is_resume_failed_frame),
            "no notice: {replies:?}"
        );
        assert_eq!(
            opening_of(&store, ids::STEP_PLAN).await,
            Some(StepOpening::Resumed)
        );
    }

    /// MOD-37 review M-1: a resume with no fallback whose start fails fails the chat as before M5:
    /// one start, the refusal, no `resume_failed` row (there is no handoff to label) and no
    /// opening, since nothing opened (H-8).
    #[tokio::test]
    async fn a_failed_resume_with_no_fallback_fails_the_chat() {
        let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
            Script::one_turn(vec![ends(StopReason::EndTurn)]),
            vec![DriverError::Transport(
                "session/resume failed: no such session".to_owned(),
            )],
        )
        .await;
        let tail_len = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("the fixture's step has a log")
            .len();

        let replies = attach_and_await(
            &mut runtime,
            &backend,
            promoted(agent_id, resume_path_without_fallback()),
        )
        .await;

        assert_eq!(starts_of(&starts).len(), 1, "one start");
        assert!(
            replies.iter().any(|reply| matches!(
                &reply.reply,
                StoreReply::Failed { request, message }
                    if *request == PROMOTE_STEP && message.contains("no such session")
            )),
            "the promotion is refused: {replies:?}"
        );
        assert!(
            !replies.iter().any(is_resume_failed_frame),
            "no notice: {replies:?}"
        );
        let log = store
            .step_events(ids::STEP_PLAN)
            .await
            .expect("the log reads")
            .expect("a log");
        assert_eq!(log.len(), tail_len, "nothing is written: {log:?}");
        assert_eq!(opening_of(&store, ids::STEP_PLAN).await, None, "no opening");
    }

    /// MOD-37 M5 (e, A-2, A-5), review L-2: only a wire failure falls back. A resume whose adapter
    /// cannot run at all, that the transport cannot do, that was cancelled, or whose own write or
    /// scrub failed does not: the handoff would fail the same way, or the failure is not the
    /// resume's. One start, the failure as before, no notice, no opening.
    #[tokio::test]
    async fn a_resume_that_cannot_spawn_does_not_fall_back() {
        for failure in [
            DriverError::Spawn("gone".to_owned()),
            DriverError::Unresolved("node".to_owned()),
            DriverError::UnknownAdapter("nope".to_owned()),
            DriverError::Unsupported("resume"),
            DriverError::Cancelled,
            DriverError::Store(StoreError::Backend("the write failed".to_owned())),
            DriverError::Scrub(htui_core::scrub::Unmasked {
                path: "/payload".to_owned(),
                rule: "anthropic_api_key",
            }),
        ] {
            let (store, backend, mut runtime, agent_id, starts) = fixture_with_failing_starts(
                Script::one_turn(vec![ends(StopReason::EndTurn)]),
                vec![failure.clone()],
            )
            .await;

            // Bounded: a fallback that wrongly starts opens a chat nobody ends.
            let replies = tokio::time::timeout(
                Duration::from_secs(30),
                attach_and_await(&mut runtime, &backend, promoted(agent_id, resume_path())),
            )
            .await
            .unwrap_or_else(|_| panic!("{failure:?}: a second session opened and never ended"));

            assert_eq!(starts_of(&starts).len(), 1, "{failure:?}: one start");
            assert!(
                replies.iter().any(|reply| matches!(
                    &reply.reply,
                    StoreReply::Failed { request, .. } if *request == PROMOTE_STEP
                )),
                "{failure:?}: the promotion is refused: {replies:?}"
            );
            assert!(
                !replies.iter().any(is_resume_failed_frame),
                "{failure:?}: no notice: {replies:?}"
            );
            let log = store
                .step_events(ids::STEP_PLAN)
                .await
                .expect("the log reads")
                .expect("a log");
            assert!(
                !log.iter().any(is_resume_failed),
                "{failure:?}: no notice row"
            );
            assert_eq!(
                opening_of(&store, ids::STEP_PLAN).await,
                None,
                "{failure:?}: no opening"
            );
        }
    }

    /// Blueprint D205: a promoted session neither mints a `run(kind='chat')` nor closes one — the
    /// step and its run are the engine's, and stay as the promotion left them.
    #[tokio::test]
    async fn a_promoted_chat_never_closes_a_run() {
        let (store, backend, mut runtime, agent_id) =
            fixture(Script::one_turn(vec![ends(StopReason::EndTurn)])).await;
        let run_before = store
            .run(ids::RUN_1)
            .await
            .expect("the read answers")
            .expect("the fixture's run");
        let step_of = async |store: &MemStore| {
            store
                .run_steps(ids::RUN_1)
                .await
                .expect("the read answers")
                .into_iter()
                .find(|step| step.id == ids::STEP_PLAN)
                .expect("the fixture's step")
        };
        let step_before = step_of(&store).await;
        let active = store.active_runs(&scope()).await.expect("count");

        let promotion = promoted(
            agent_id,
            OpeningPath::Handoff {
                text: "pick up where the step stopped".to_owned(),
                digest: "d-handoff".to_owned(),
            },
        );
        let replies = attach_and_end(&mut runtime, &backend, promotion, async |_| {
            assert_eq!(
                store.active_runs(&scope()).await.expect("count"),
                active,
                "no chat run is minted for a promoted step"
            );
        })
        .await;
        assert!(
            matches!(
                replies.last().map(|reply| &reply.reply),
                Some(StoreReply::Chat(ChatFrame::Ended { .. }))
            ),
            "the session ended: {replies:?}"
        );

        let run_after = store
            .run(ids::RUN_1)
            .await
            .expect("the read answers")
            .expect("the fixture's run");
        assert_eq!(run_after.status, run_before.status, "the run is untouched");
        assert_eq!(run_after.finished_at, run_before.finished_at);
        let step_after = step_of(&store).await;
        assert_eq!(step_after.status, step_before.status, "so is the step");
        assert_eq!(
            step_after.prompt_digest, step_before.prompt_digest,
            "and the step's prompt digest is the original prompt's"
        );
        assert_eq!(store.active_runs(&scope()).await.expect("count"), active);
    }

    /// Blueprint D206 (F-H): a chat whose task has ended is not live, even while the runtime still
    /// holds its entry — `caps` answers until the next `serve` sweeps it, `live_steps` does not.
    #[tokio::test]
    async fn live_steps_drops_an_ended_chat() {
        let (_store, backend, mut runtime, agent_id) =
            fixture(Script::one_turn(vec![ends(StopReason::EndTurn)])).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let Served::Start { step_id, task } = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await
        else {
            panic!("a chat start opens a session")
        };
        assert_eq!(
            runtime.live_steps(),
            vec![step_id],
            "a started chat is live"
        );

        let cancel = runtime
            .serve(
                &backend,
                &tx,
                &envelope(8, StoreRequest::ChatCancel { step_id }),
            )
            .await;
        assert!(matches!(cancel, Served::Deferred), "{cancel:?}");
        task.await;

        assert!(
            runtime.caps(step_id).is_some(),
            "the ended chat's entry is still held until the next serve"
        );
        assert!(
            runtime.live_steps().is_empty(),
            "but its command channel is closed, so it is not live"
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

    /// Since MOD-25 an offline backend is refused for *being* offline, and the refusal is one
    /// fixed sentence: `htui` is online-only, so a box whose Postgres is unreachable browses its
    /// read-only cache and starts no run (`R-STO-4`).
    ///
    /// Three facts, and the sentence is asserted by **equality**, not `contains`: it is the
    /// contract the status line and the Chat body both render, so a reword is a deliberate change
    /// and not a silent one. The refusal is answered under the `chat_start` request name, and
    /// nothing was started — no step, so no driver was spawned to write into a buffer nobody
    /// reads.
    #[tokio::test]
    async fn an_offline_backend_refuses_a_chat_with_the_unreachable_warning() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "chat-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(
            backend.writer().is_none(),
            "since MOD-25 the offline write path is a refusal, not the buffer"
        );

        let mut runtime = AgentRuntime::new(DriverFactory::new());
        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(1, start(AgentId::new(), "hi")))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "chat_start");
                // Equality, not `contains`: the sentence is the contract. `message` is the
                // error's `Display`, so it carries `StoreError::Unreachable`'s own
                // `store unreachable: ` prefix (`htui-core/src/store/error.rs:33`) — the
                // assertion pins the whole rendered line, and the constant word for word
                // inside it.
                assert_eq!(
                    message,
                    format!("store unreachable: {}", htui_store::DATABASE_UNREACHABLE),
                    "the sentence is the contract, word for word"
                );
            }
            other => panic!("an offline chat must be refused: {other:?}"),
        }
        assert!(
            runtime.steps().is_empty(),
            "a refused chat starts no step: {:?}",
            runtime.steps()
        );
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
            .upsert_agent(&acp_fake_row(agent_id), None)
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

    /// Blueprint D27: while a box probe runs, a chat start on a stale row spawns no staleness
    /// re-probe. The box probe ends by probing every agent row on this box, so a second writer
    /// of the same `agent_box` row would only race it.
    #[tokio::test]
    async fn a_chat_start_spawns_no_staleness_reprobe_while_a_box_probe_runs() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(agent_id), None)
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
        runtime.box_probe = Some(tokio::spawn(std::future::pending()));
        assert!(runtime.box_probe_running());

        let (tx, _rx) = mpsc::unbounded_channel();
        let served = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hello")))
            .await;
        assert!(
            matches!(served, Served::Start { .. }),
            "the chat still starts: {served:?}"
        );
        assert_eq!(
            runtime.background_len(),
            0,
            "the box probe re-probes every row anyway"
        );
        runtime.box_probe.take().expect("still held").abort();
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
            .upsert_agent(&acp_fake_row(agent_id), None)
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
            .upsert_agent(&acp_fake_row(acp_agent), None)
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
            .upsert_agent(&unspawnable_row(agent_id), None)
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
        store
            .upsert_agent(&agent, None)
            .await
            .expect("the acp row lands");
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
            .upsert_agent(&unspawnable_row(agent_id), None)
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
        make_unresolvable(&store).await;
        store
    }

    /// Rewrites every registry row of `store` to [`unresolvable_registry`]'s launch (H-7).
    pub(crate) async fn make_unresolvable(store: &MemStore) {
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
            edit_agent(store, &agent).await.expect("the row updates");
        }
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
                    "the offline backend's own sentence, not a second one: {message}"
                );
            }
            other => panic!("an offline backend refuses the probe: {other:?}"),
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

    /// MOD-23 D243: a row the human switched off on this box stays off in the probe's own reply,
    /// as the store keeps it (`enabled AND NOT user_off`), though the probe finds it `ready`. The
    /// row is a `cli` row whose `launch` is a literal command with no `discovery`, so it probes
    /// `ready` and nothing is spawned: a `cli` row has no handshake. (Blueprint F-10 said `acp`,
    /// but an `acp` row with no `discovery` does run tier 2: `probe.rs`'s `is_none_or`.) The box
    /// probe shares `probe_agents_on`, so this covers its reply too.
    #[tokio::test]
    async fn a_probe_reply_keeps_a_switched_off_row_off() {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let command = tmp.path().join("agent-bin");
        std::fs::write(&command, "").expect("the literal command exists");
        let store = MemStore::demo();
        let now = Utc::now();
        let agent = Agent {
            id: AgentId::new(),
            name: "agent-literal".to_owned(),
            transport: Transport::Cli,
            launch: json!({ "command": command.to_string_lossy(), "args": [], "env": {} }),
            models: Vec::new(),
            default_model: None,
            billing: htui_core::model::Billing::Subscription,
            enabled: true,
            settings: json!({}),
            created_at: now,
            updated_at: now,
        };
        store
            .upsert_agent(&agent, None)
            .await
            .expect("the new row lands");
        store
            .set_agent_box_enabled(agent.id, ids::BOX, false)
            .await
            .expect("the switch lands");
        let rows: Vec<_> = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .filter(|summary| summary.agent.id == agent.id)
            .collect();
        assert!(rows[0].user_off, "the read carries the switch");

        let replied = probe_agents_on(
            &Writer::Memory(store.clone()),
            ids::BOX,
            rows,
            &ProbeEnv::host(tmp.path().to_path_buf()).without_versions(),
        )
        .await
        .expect("the memory store never fails");

        let on_box = replied[0].on_box.as_ref().expect("the probe wrote a row");
        assert_eq!(
            on_box
                .probe
                .as_ref()
                .and_then(|probe| probe.get("status"))
                .and_then(Value::as_str),
            Some("ready"),
            "a literal command resolves without a spawn"
        );
        assert!(
            !on_box.enabled,
            "the reply says what the store kept: a switched-off row stays off"
        );
        let stored = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent.id)
            .expect("the row is still listed");
        assert!(stored.user_off);
        assert!(
            !stored.on_box.expect("the probe's row").enabled,
            "the store kept the veto"
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
    async fn a_plan_is_refused_before_any_request_on_an_offline_backend() {
        let root = tempfile::tempdir().expect("a throwaway config root");
        let cache = htui_store::CacheStore::open(root.path(), "install-offline", 1)
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
                    "the offline backend's own sentence, not a second one: {message}"
                );
            }
            other => panic!("an offline backend refuses the plan: {other:?}"),
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
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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
                    message.ends_with(BOX_WRITE_RUNNING),
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

    /// MOD-31 D5, the bug in one test: a prompt preview reaches no write method (plan D102), so it
    /// holds no claim and a live one does not refuse an install.
    ///
    /// Hold `j` in the Backlog detail — a task that reads a dozen tables, walks the filesystem and
    /// records nothing — and press `i` in Settings. Before the split this came back *"a probe is
    /// already running on this box"* while the only thing alive was the preview, and no probe had
    /// been asked for.
    ///
    /// The two `serve` calls are adjacent on purpose: `serve` sweeps finished tasks as its first
    /// statement, so an `await` of my own between the preview and the install could sweep the very
    /// entry whose presence makes this case mean anything.
    #[tokio::test]
    async fn a_running_preview_does_not_refuse_an_install() {
        // `unresolvable_registry` for the same reason the sibling above uses it, and **no**
        // `fixture.route("/registry.json", …)`: this case never wants a *long* registry read, only
        // a short one. Unrouted, the responder answers 404 in milliseconds, so the plan task ends
        // long before the teardown rather than being waited out by it.
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true), None)
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime = installing_runtime(&fixture, &tmp.path().join("agents"));
        let (tx, _rx) = mpsc::unbounded_channel();

        // A preview first: deferred, owned by the runtime, and holding no claim. `HTUI_FEAT_1` is a
        // real demo item, so the task does real work and is still unfinished when the next request
        // sweeps — which is what keeps the entry in the collection the guard consults.
        let previewed = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::PromptPreview {
                        item: ids::HTUI_FEAT_1,
                        template_name: None,
                        scope: scope(),
                    },
                ),
            )
            .await;
        assert!(
            matches!(previewed, Served::Deferred),
            "the preview is deferred to the runtime's own task (`R-NF-3`): {previewed:?}"
        );
        assert_eq!(
            runtime.background_len(),
            1,
            "the runtime owns one task, which is the fact `R-NF-3` asks for"
        );
        assert_eq!(
            runtime.writing_background_len(),
            0,
            "and that task writes no `agent_box` row, so it is not in the guard's set (MOD-31 D4)"
        );

        let planned = runtime
            .serve(
                &backend,
                &tx,
                &envelope(2, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        // `Deferred`, and **not** "the install succeeded". This case pins the decision `serve`
        // returned, not the plan's outcome: `install_plan` gets past the guard, spawns `run_plan`,
        // and the registry is unrouted, so the pre-flight fails for the fixture's own reasons —
        // asynchronously, inside the spawned task, after `install_plan` has already returned
        // `Deferred`. Asserting on that failure would be asserting on the fixture, and there is no
        // way to await the reply here without giving `run_plan` the one thing this case must not
        // grant it: an `await` of the test's own between the preview and the install, which
        // `sweep_finished` would use to drop the very entry whose presence makes the case mean
        // anything.
        //
        // The positive form is what makes this bite, and the old `if let` shape is what hid it.
        // `serve` converts **every** `Err` out of `install_plan` into
        // `Served::Reply(StoreReply::Failed)`, so a refusal is never `Deferred`: an
        // `if let … = &planned` was skipped outright whenever the answer was not a `Failed`, and
        // matched vacuously when it *was* one for any other reason — an
        // `installing_runtime` that stopped attaching its installer fails at `install_config`,
        // which is checked before `claim_is_free`, and the whole claim went untested in green.
        // Naming the guard's two sentences instead did not close that hole either: it only asked
        // that a refusal be *some other* refusal, so the pre-fix guard's own wording was never
        // what a passing assert had to exclude. Here every precondition — `install_config`,
        // `recording_writer`, `registered_box`, `claim_is_free`, `row_for`, `declares_a_source` —
        // has to have returned `Ok` and the spawn has to have happened, so a runtime that refused
        // for any reason at all, the pre-fix guard's included, goes red right here.
        assert!(
            matches!(planned, Served::Deferred),
            "a preview writes no `agent_box` row, so it holds no claim (MOD-31 D5): {planned:?}"
        );
        runtime.shutdown(Duration::ZERO).await;
    }

    /// MOD-31 D5 and D6, the other half of the claim: a chat's staleness re-probe **does** write
    /// `agent_box`, so it holds the claim exactly as a probe does, and the refusal names both
    /// kinds of task that do.
    ///
    /// OQ-3's conservative answer, pinned. Narrowing the guard to the writing half would have been
    /// wrong for a re-probe — the hazard is the same one a probe carries, on the same row — so this
    /// case is here to stop a future reader "simplifying" the re-probe's push site to `reading`.
    ///
    /// The staging is `a_chat_start_on_a_stale_acp_row_re_probes_in_the_background` **plus** an
    /// installer, because the two cannot be combined from either fixture alone:
    /// `installing_runtime`'s `DriverFactory::new()` registers no `acp` transport, so a `ChatStart`
    /// on an `acp` row is refused by `driver_for` before the re-probe is ever considered; and a
    /// runtime with no installer answers "this runtime has no installer" at
    /// [`install_config`](Self::install_config), which is checked before the claim.
    #[tokio::test]
    async fn a_running_chat_reprobe_still_refuses_an_install() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&acp_fake_row(agent_id), None)
            .await
            .expect("the acp row lands");
        let backend = Backend::memory(store);
        let fixture = Fixture::start().await;
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let mut runtime =
            AgentRuntime::new(acp_factory(Script::one_turn(vec![ScriptEvent::Emit(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
            )])))
            .with_grace(Duration::from_millis(0))
            .with_installer(InstallConfig::new(
                fixture.base(),
                Some(tmp.path().join("agents")),
            ));
        let (tx, _rx) = mpsc::unbounded_channel();

        let started = runtime
            .serve(&backend, &tx, &envelope(1, start(agent_id, "hello")))
            .await;
        assert!(
            matches!(started, Served::Start { .. }),
            "the chat starts on what resolution already gave it: {started:?}"
        );
        assert_eq!(
            runtime.writing_background_len(),
            1,
            "an unprobed `acp` row is re-probed beside the chat, and the re-probe writes `agent_box`"
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
                    message.ends_with(BOX_WRITE_RUNNING),
                    "the refusal names the re-probe as well as the probe (MOD-31 D6): {message}"
                );
            }
            other => panic!("a re-probe and an install race on `agent_box`: {other:?}"),
        }
        // The refusal is at the claim, which is before `row_for`, so `acp_fake_row`'s lack of a
        // `discovery.install` never comes into it, and nothing was spawned behind it: no plan, no
        // registry read, no `/registry.json` route. The chat and the re-probe go with `shutdown`.
        runtime.shutdown(Duration::ZERO).await;
    }

    /// MOD-31 D5, the caller the HANDOFF does not mention: [`StoreRequest::ProbeBox`] is the path
    /// an `Online` swap's registration probe goes through, and `on_online` skips it **silently** —
    /// it only logs `tracing::info!` and records nothing, so the box stays unprobed until the next
    /// swap. The install case above at least tells the user something; this one says nothing at
    /// all, which is why it needs its own test rather than riding along on that one.
    ///
    /// Its staging is *not* the install case's: [`probe_box`](Self::probe_box) never calls
    /// `install_config`, so it needs no `Fixture` and no installer. Routing a 30-second
    /// `/registry.json` here would buy nothing — this path spawns a **box** probe, not a plan — and
    /// `finish_background` would then wait out the delay.
    #[tokio::test]
    async fn a_running_preview_does_not_refuse_a_box_probe() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store);
        let mut runtime = box_runtime(tmp.path());
        let (tx, _rx) = mpsc::unbounded_channel();

        let previewed = runtime
            .serve(
                &backend,
                &tx,
                &envelope(
                    1,
                    StoreRequest::PromptPreview {
                        item: ids::HTUI_FEAT_1,
                        template_name: None,
                        scope: scope(),
                    },
                ),
            )
            .await;
        assert!(
            matches!(previewed, Served::Deferred),
            "the preview is deferred to the runtime's own task (`R-NF-3`): {previewed:?}"
        );
        assert_eq!(
            runtime.background_len(),
            1,
            "the preview is in the collection, so the zero below is about which entry it is"
        );
        assert_eq!(
            runtime.writing_background_len(),
            0,
            "and it holds no claim (MOD-31 D4), so it is not in the guard's set"
        );

        let probed = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeBox))
            .await;
        // The same rule, and for the same reasons as the install case: the *decision* is pinned
        // rather than the probe's outcome, and the positive form is what closes the hole an `if
        // let` leaves. See that case's comment for why — `serve` turns every `Err` out of
        // `probe_box` into `Served::Reply(Failed)`, so `Deferred` can only mean that
        // `registered_box` and `claim_is_free` both let it through, and the pre-fix guard's
        // refusal is then not one of the ways this assert can pass.
        assert!(
            matches!(probed, Served::Deferred),
            "a preview writes no `agent_box` row, so it holds no claim (MOD-31 D5): {probed:?}"
        );
        // A `ProbeBox` that got past the guard really does spawn a box probe, so this case has to
        // finish it rather than leave it behind a drop. `never_probed` is the unresolvable
        // registry, so it resolves in milliseconds and spawns no adapter anywhere near the suite.
        runtime.finish_background(Duration::from_secs(10)).await;
    }

    /// A row whose `discovery` declares no source is refused **by the row**, with the pre-flight's
    /// own sentence and without a request: the answer is in the document the worker already holds.
    #[tokio::test]
    async fn a_plan_for_a_row_without_install_is_refused_by_name() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "undeclared", false), None)
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
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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
        store.upsert_agent(&row, None).await.expect("the row lands");

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
        use htui_agent::auth::loopback::PasteError;
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

        /// The authorization code a pasted redirect carries (MOD-22): no frame, no `Debug` and
        /// no log line may ever carry it (the credential rule of `htui_agent::auth::loopback`).
        pub(crate) const CODE: &str = "CODE-SENTINEL-4f1c";

        /// The `state` a loopback link carries and a pasted redirect echoes. Unlike [`CODE`] it
        /// is on screen already, as part of the link.
        pub(crate) const STATE: &str = "STATE-SENTINEL-9a2e";

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
        /// printed to stderr before the answer; `FIXTURE_WAIT_FOR` is a path `authenticate` waits to
        /// exist, every 50 ms, before answering (MOD-22: the case's listener creates it);
        /// `FIXTURE_CRED` is written into the credential file on success and removed on `logout`.
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
      if [ -n "$FIXTURE_WAIT_FOR" ]; then while [ ! -e "$FIXTURE_WAIT_FOR" ]; do sleep 0.05; done; fi
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
                tools: BTreeMap::new(),
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
                manual: BTreeMap::new(),
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
                .upsert_agent(&login_row(agent_id, Transport::Acp, dir, extra), None)
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
            AgentRuntime::new(DriverFactory::production())
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
        async fn a_start_is_refused_before_any_spawn_on_an_offline_backend() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let cache = htui_store::CacheStore::open(tmp.path(), "login-offline", 1)
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
                        "the offline backend's own sentence, not a second one: {message}"
                    );
                }
                other => panic!("an offline backend refuses the login: {other:?}"),
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
                .upsert_agent(
                    &login_row(
                        agent_id,
                        Transport::Cli,
                        tmp.path(),
                        &[("FIXTURE_KEY", "set")],
                    ),
                    None,
                )
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
                .upsert_agent(
                    &login_row(
                        agent_id,
                        Transport::Acp,
                        tmp.path(),
                        &[("FIXTURE_KEY", "set")],
                    ),
                    None,
                )
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
                .upsert_agent(&install_row(install_id, "demo", true), None)
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
                .upsert_agent(&install_row(install_id, "demo", true), None)
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

        /// MOD-66 D8: a login writes its row at the end of the flow, so a tool-paths write for
        /// any row is refused while one runs.
        #[tokio::test]
        async fn set_tool_paths_while_a_login_runs_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, agent_id) =
                login_store(tmp.path(), &[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;

            let message = refused(set_paths(&mut runtime, &backend, &tx, 2, agent_id, &[]).await);
            assert!(
                message.contains("a login is already running"),
                "the refusal says what holds the claim: {message}"
            );
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
                deliver_limits: DeliverLimits::default(),
                frames: Frames::new(
                    tx,
                    ReplyAddr {
                        seq: 2,
                        origin: origin.clone(),
                    },
                ),
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

        // -------------------------------------------------------------------------------------
        // MOD-22: a pasted redirect, delivered from inside the live flow (D269, D277, D286)
        //
        // No assertion message here prints what the listener received: that alone carries the
        // code sentinel. Replies are printed freely, since the rule is that none can carry it.
        // -------------------------------------------------------------------------------------

        /// The listener's page, as a browser would have been shown it.
        const SIGNED_IN: &str = "<title>signed in</title>";

        /// A patience for a refusal that must *not* happen: nothing connects within it.
        const QUIET: Duration = Duration::from_millis(200);

        /// A loopback listener on a free port, and the port.
        async fn listener() -> (u16, tokio::net::TcpListener) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a free loopback port");
            let port = listener.local_addr().expect("a bound address").port();
            (port, listener)
        }

        /// The link a loopback adapter prints: a vendor URL whose `redirect_uri` is this box's
        /// `127.0.0.1:<port>/`, carrying [`STATE`].
        fn loopback_link(port: u16) -> String {
            format!(
                "https://h.invalid/login?redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2F&state={STATE}"
            )
        }

        /// The address a browser would fail to open: `code` is [`CODE`], `state` is `state`.
        fn pasted(port: u16, state: &str) -> RedirectUrl {
            RedirectUrl::new(format!(
                "http://127.0.0.1:{port}/?code={CODE}&state={state}"
            ))
        }

        /// The `DeliverLimits` a case with a silent listener would otherwise wait fifteen seconds
        /// on.
        fn quick_limits() -> DeliverLimits {
            DeliverLimits {
                connect: Duration::from_millis(500),
                response: Duration::from_millis(200),
            }
        }

        /// Reads one request head (through the blank line) off `stream`.
        async fn read_head(stream: &mut tokio::net::TcpStream) -> String {
            use tokio::io::AsyncReadExt as _;
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut buf)
                    .await
                    .expect("the request is readable");
                if read == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..read]);
            }
            String::from_utf8_lossy(&head).into_owned()
        }

        /// A listener that reads one request, answers `200` with [`SIGNED_IN`] and closes, and
        /// only **then** lets the adapter answer `authenticate` (it creates `marker`, which the
        /// fixture's `FIXTURE_WAIT_FOR` polls for) once `release` fires. Yields the head it read.
        fn answering_listener(
            listener: tokio::net::TcpListener,
            marker: PathBuf,
            release: oneshot::Receiver<()>,
        ) -> JoinHandle<String> {
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                let (mut stream, _) = listener.accept().await.expect("the delivery connects");
                let head = read_head(&mut stream).await;
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{SIGNED_IN}",
                    SIGNED_IN.len()
                );
                stream
                    .write_all(answer.as_bytes())
                    .await
                    .expect("the answer is written");
                stream.shutdown().await.expect("the answer is closed");
                drop(stream);
                let _ = release.await;
                std::fs::write(&marker, b"").expect("the marker lands");
                head
            })
        }

        /// A listener that accepts and says nothing, holding every connection until aborted, and
        /// signals each accept on the receiver it returns (review L-9).
        fn silent_listener(
            listener: tokio::net::TcpListener,
        ) -> (JoinHandle<()>, mpsc::UnboundedReceiver<()>) {
            let (accepted_tx, accepted) = mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                let mut held = Vec::new();
                loop {
                    let (stream, _) = listener.accept().await.expect("a delivery connects");
                    held.push(stream);
                    let _ = accepted_tx.send(());
                }
            });
            (task, accepted)
        }

        /// Whether `listener` is connected to within [`QUIET`].
        async fn accepts_within_quiet(listener: &tokio::net::TcpListener) -> bool {
            tokio::time::timeout(QUIET, listener.accept()).await.is_ok()
        }

        /// Chooses [`METHOD`] at seq 2 and waits for the link the adapter prints.
        async fn choose_and_await_link(
            runtime: &mut AgentRuntime,
            backend: &Backend,
            tx: &mpsc::UnboundedSender<ReplyEnvelope>,
            rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
        ) -> String {
            let served = runtime
                .serve(
                    backend,
                    tx,
                    &envelope(
                        2,
                        StoreRequest::AuthChoose {
                            choice: AuthChoice::Method(METHOD.to_owned()),
                        },
                    ),
                )
                .await;
            assert!(matches!(served, Served::Deferred), "{served:?}");
            loop {
                match next_frame(rx).await {
                    (seq, AuthFrame::Url(url)) => {
                        assert_eq!(seq, 2, "the link answers the choice");
                        return url;
                    }
                    (_, other) => assert!(
                        matches!(other, AuthFrame::Line(_)),
                        "a flow says nothing but lines before its link: {other:?}"
                    ),
                }
            }
        }

        /// `AuthDeliver` at `seq`, which the live flow takes (`Served::Deferred`).
        async fn deliver_at(
            runtime: &mut AgentRuntime,
            backend: &Backend,
            tx: &mpsc::UnboundedSender<ReplyEnvelope>,
            seq: Seq,
            url: RedirectUrl,
        ) {
            let served = runtime
                .serve(
                    backend,
                    tx,
                    &envelope(seq, StoreRequest::AuthDeliver { url }),
                )
                .await;
            assert!(
                matches!(served, Served::Deferred),
                "a delivery goes into the running flow: {served:?}"
            );
        }

        /// `AuthCancel` at `seq`, answered `Cancelling` at once.
        async fn cancel_at(
            runtime: &mut AgentRuntime,
            backend: &Backend,
            tx: &mpsc::UnboundedSender<ReplyEnvelope>,
            seq: Seq,
        ) {
            let served = runtime
                .serve(backend, tx, &envelope(seq, StoreRequest::AuthCancel))
                .await;
            assert!(
                matches!(
                    served,
                    Served::Reply(StoreReply::Auth(AuthFrame::Cancelling))
                ),
                "a cancel is answered at once: {served:?}"
            );
        }

        /// Every reply up to and including the flow's terminal frame, `Failed`s included
        /// ([`next_frame`] panics on those).
        async fn replies_until_terminal(
            rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
        ) -> Vec<(Seq, StoreReply)> {
            let mut replies = Vec::new();
            loop {
                let reply = next_reply(rx).await;
                let last = matches!(&reply.reply, StoreReply::Auth(frame) if is_terminal(frame));
                replies.push((reply.seq, reply.reply));
                if last {
                    return replies;
                }
            }
        }

        /// Whether `reply` is a refusal of `auth_deliver` that says exactly `sentence`.
        fn refused_with(reply: &StoreReply, sentence: &str) -> bool {
            matches!(
                reply,
                StoreReply::Failed { request, message }
                    if *request == "auth_deliver" && message == sentence
            )
        }

        /// The next reply must be the refusal of the deliver at `seq`, in `sentence`.
        async fn expect_refusal(
            rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
            seq: Seq,
            sentence: &str,
        ) {
            let reply = next_reply(rx).await;
            assert_eq!(reply.seq, seq, "a deliver is answered at its own address");
            assert!(
                refused_with(&reply.reply, sentence),
                "expected `{sentence}`, got {:?}",
                reply.reply
            );
        }

        /// D269 end to end: the paste reaches the port the link advertised, as one `GET` of the
        /// pasted path and query, and the listener's answer reaches the pane **before** the
        /// login's own result.
        #[tokio::test]
        async fn a_pasted_redirect_reaches_the_advertised_port_and_the_login_completes() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let link = loopback_link(port);
            let marker = tmp.path().join("delivered");
            let marker_text = marker.to_string_lossy().into_owned();
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_CRED", CREDENTIAL),
                    ("FIXTURE_URL", &link),
                    ("FIXTURE_WAIT_FOR", &marker_text),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (release, released) = oneshot::channel();
            let _ = release.send(());
            let head = answering_listener(bound, marker, released);

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let url = choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            assert_eq!(url, link);
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            let replies = replies_until_terminal(&mut rx).await;

            let head = tokio::time::timeout(PATIENCE, head)
                .await
                .expect("the delivery reached the listener")
                .expect("the listener ran");
            let request_line = format!("GET /?code={CODE}&state={STATE} HTTP/1.1\r\n");
            assert!(
                head.starts_with(&request_line),
                "one GET of the pasted path and query, to the advertised port"
            );
            let delivered = replies.iter().position(|(seq, reply)| {
                *seq == 3
                    && matches!(
                        reply,
                        StoreReply::Auth(AuthFrame::Delivered(said))
                            if said.status == 200 && said.said.as_deref() == Some("signed in")
                    )
            });
            let done = replies.iter().position(|(seq, reply)| {
                *seq == 2
                    && matches!(
                        reply,
                        StoreReply::Auth(AuthFrame::Done {
                            status: ProbeStatus::Ready,
                            ..
                        })
                    )
            });
            assert!(
                delivered.is_some() && done.is_some() && delivered < done,
                "`Delivered` at the deliver's seq, then `Done {{ ready }}` at the choice's: \
                 {replies:?}"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// D265 in the worker: the port is checked against the flow's own record, and a paste
        /// for another one connects nowhere.
        #[tokio::test]
        async fn a_paste_for_another_port_is_refused_and_nothing_connects() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (advertised, on_advertised) = listener().await;
            let (other, on_other) = listener().await;
            let link = loopback_link(advertised);
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", &link),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(other, STATE)).await;
            expect_refusal(
                &mut rx,
                3,
                &format!(
                    "the address is for port {other}; this login is listening on {advertised}"
                ),
            )
            .await;
            assert!(
                !accepts_within_quiet(&on_advertised).await,
                "the advertised port heard nothing"
            );
            assert!(
                !accepts_within_quiet(&on_other).await,
                "and neither did the pasted one"
            );

            cancel_at(&mut runtime, &backend, &tx, 4).await;
            let replies = replies_until_terminal(&mut rx).await;
            assert!(
                matches!(
                    replies.last(),
                    Some((2, StoreReply::Auth(AuthFrame::Cancelled)))
                ),
                "the login is still the pane's to cancel: {replies:?}"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// D247's rule (MOD-23): the worker's check is the authority, whatever the pane let
        /// through.
        #[tokio::test]
        async fn a_stale_state_is_refused_by_the_worker_even_if_the_pane_let_it_through() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let link = loopback_link(port);
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", &link),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, "OTHER-STATE")).await;
            expect_refusal(&mut rx, 3, &PasteError::StaleState.to_string()).await;
            assert!(!accepts_within_quiet(&bound).await, "nothing connected");

            cancel_at(&mut runtime, &backend, &tx, 4).await;
            replies_until_terminal(&mut rx).await;
            runtime.finish_background(PATIENCE).await;
        }

        /// A link with no loopback `redirect_uri` (MOD-21's own) gives the worker nothing to
        /// deliver to.
        #[tokio::test]
        async fn a_deliver_before_any_loopback_redirect_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
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
            assert_eq!(
                choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await,
                LINK
            );
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            expect_refusal(&mut rx, 3, NO_LOOPBACK_REDIRECT).await;
            assert!(!accepts_within_quiet(&bound).await, "nothing connected");

            cancel_at(&mut runtime, &backend, &tx, 4).await;
            replies_until_terminal(&mut rx).await;
            runtime.finish_background(PATIENCE).await;
        }

        /// `auth_command`'s first refusal covers the new request too.
        #[tokio::test]
        async fn a_deliver_with_no_login_running_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (store, _agent_id) = login_store(tmp.path(), &[("FIXTURE_KEY", "set")]).await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, _rx) = mpsc::unbounded_channel();

            match runtime
                .serve(
                    &backend,
                    &tx,
                    &envelope(
                        1,
                        StoreRequest::AuthDeliver {
                            url: pasted(9, STATE),
                        },
                    ),
                )
                .await
            {
                Served::Reply(reply) => assert!(
                    matches!(
                        &reply,
                        StoreReply::Failed { request, message }
                            if *request == "auth_deliver" && message == "no login is running"
                    ),
                    "refused by name: {reply:?}"
                ),
                other => panic!("a deliver with no flow is refused, not served: {other:?}"),
            }
            assert_eq!(fixture_pid(tmp.path()), None, "and nothing ran");
        }

        /// One delivery at a time: a second is refused by name, and the first is still answered
        /// exactly once when the login ends under it.
        #[tokio::test]
        async fn a_second_deliver_while_one_is_in_flight_is_refused() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let (silent, _accepted) = silent_listener(bound);
            let link = loopback_link(port);
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", &link),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            // The default limits (F-5): the first delivery outlives everything below.
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            deliver_at(&mut runtime, &backend, &tx, 4, pasted(port, STATE)).await;
            expect_refusal(&mut rx, 4, DELIVERY_IN_FLIGHT).await;

            cancel_at(&mut runtime, &backend, &tx, 5).await;
            let replies = replies_until_terminal(&mut rx).await;
            let first = replies
                .iter()
                .position(|(seq, reply)| *seq == 3 && refused_with(reply, LOGIN_ENDED));
            assert!(
                first.is_some() && first < Some(replies.len() - 1),
                "the first delivery is answered `{LOGIN_ENDED}` before the stream ends: \
                 {replies:?}"
            );
            assert!(
                matches!(
                    replies.last(),
                    Some((2, StoreReply::Auth(AuthFrame::Cancelled)))
                ),
                "{replies:?}"
            );

            silent.abort();
            runtime.finish_background(PATIENCE).await;
        }

        /// D269(c)/D277: the delivery runs beside the flow, so `x` is served while it is in
        /// flight, and a cancelled login does not wait out the response deadline.
        #[tokio::test]
        async fn cancel_is_served_while_a_delivery_is_in_flight() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let (silent, mut accepted) = silent_listener(bound);
            let link = loopback_link(port);
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", &link),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            let pid = fixture_pid(tmp.path()).expect("the fixture wrote its pid when it started");
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            // Review L-9: the listener has accepted the delivery's connection, so the `x` below
            // lands on a delivery that is provably in flight rather than one not yet started.
            tokio::time::timeout(Duration::from_secs(5), accepted.recv())
                .await
                .expect("the delivery connects well inside its deadlines")
                .expect("the listener is still accepting");

            cancel_at(&mut runtime, &backend, &tx, 4).await;
            let replies =
                tokio::time::timeout(Duration::from_secs(5), replies_until_terminal(&mut rx))
                    .await
                    .expect("the login ends well inside the 15 s response deadline");
            let answers: Vec<_> = replies.iter().filter(|(seq, _)| *seq == 3).collect();
            assert!(
                answers.len() == 1 && refused_with(&answers[0].1, LOGIN_ENDED),
                "the deliver is answered exactly once, `{LOGIN_ENDED}`: {replies:?}"
            );
            assert!(
                matches!(
                    replies.last(),
                    Some((2, StoreReply::Auth(AuthFrame::Cancelled)))
                ),
                "{replies:?}"
            );
            assert_not_running(pid, "a login cancelled mid-delivery").await;

            silent.abort();
            runtime.finish_background(PATIENCE).await;
        }

        /// D286: the one reason `with_deliver_limits` exists. A listener that never answers
        /// times the delivery out, and the login is still there to finish or cancel.
        #[tokio::test]
        async fn a_silent_listener_times_the_delivery_out_and_the_login_keeps_running() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let (silent, _accepted) = silent_listener(bound);
            let link = loopback_link(port);
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_HOLD", "1"),
                    ("FIXTURE_URL", &link),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path()).with_deliver_limits(quick_limits());
            let (tx, mut rx) = mpsc::unbounded_channel();

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            let reply = next_reply(&mut rx).await;
            assert_eq!(reply.seq, 3, "answered at the deliver's own address");
            assert!(
                matches!(
                    &reply.reply,
                    StoreReply::Failed { request, message }
                        if *request == "auth_deliver" && message.contains("did not answer within")
                ),
                "a silent listener times out: {:?}",
                reply.reply
            );
            assert!(runtime.auth_running(), "and the login keeps running");

            cancel_at(&mut runtime, &backend, &tx, 4).await;
            let replies = replies_until_terminal(&mut rx).await;
            assert!(
                matches!(
                    replies.last(),
                    Some((2, StoreReply::Auth(AuthFrame::Cancelled)))
                ),
                "{replies:?}"
            );

            silent.abort();
            runtime.finish_background(PATIENCE).await;
        }

        /// R-10's ordering (D286): a listener that lets the adapter finish and closes without a
        /// word is still answered for, and **before** the login's own result.
        #[tokio::test]
        async fn a_delivery_the_listener_drops_is_answered_before_the_logins_result() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let link = loopback_link(port);
            let marker = tmp.path().join("delivered");
            let marker_text = marker.to_string_lossy().into_owned();
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_CRED", CREDENTIAL),
                    ("FIXTURE_URL", &link),
                    ("FIXTURE_WAIT_FOR", &marker_text),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();
            let dropping = tokio::spawn(async move {
                let (mut stream, _) = bound.accept().await.expect("the delivery connects");
                read_head(&mut stream).await;
                std::fs::write(&marker, b"").expect("the marker lands");
                drop(stream);
            });

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            let replies = replies_until_terminal(&mut rx).await;
            tokio::time::timeout(PATIENCE, dropping)
                .await
                .expect("the delivery reached the listener")
                .expect("the listener ran");

            let closed = DeliverError::ClosedWithoutAnswer {
                target: format!("127.0.0.1:{port}"),
            }
            .to_string();
            let answered = replies
                .iter()
                .position(|(seq, reply)| *seq == 3 && refused_with(reply, &closed));
            let done = replies.iter().position(|(seq, reply)| {
                *seq == 2 && matches!(reply, StoreReply::Auth(AuthFrame::Done { .. }))
            });
            assert!(
                answered.is_some() && done.is_some() && answered < done,
                "the dropped delivery is answered before `Done`: {replies:?}"
            );

            runtime.finish_background(PATIENCE).await;
        }

        /// Review L-1 for the new command: a `Deliver` still queued when the flow ends is refused
        /// at its own address rather than dropped.
        #[tokio::test]
        async fn a_deliver_queued_when_the_flow_ends_is_refused_rather_than_dropped() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let backend = Backend::memory(MemStore::demo());
            let writer = recording_writer(&backend).expect("a memory backend hands out a writer");
            let origin = Origin::Tab(crate::ui::tabs::TabId("settings"));
            let (commands_tx, commands_rx) = mpsc::unbounded_channel();
            let (tx, mut rx) = mpsc::unbounded_channel();

            // Queued before the flow is polled even once, and never read by the loop.
            commands_tx
                .send(AuthCommand::Deliver {
                    url: pasted(9, STATE),
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
                deliver_limits: DeliverLimits::default(),
                frames: Frames::new(
                    tx,
                    ReplyAddr {
                        seq: 2,
                        origin: origin.clone(),
                    },
                ),
            })
            .await;

            let mut replies = Vec::new();
            while let Some(reply) = rx.recv().await {
                replies.push((reply.seq, reply.reply));
            }
            assert!(
                replies
                    .iter()
                    .any(|(seq, reply)| *seq == 3 && refused_with(reply, LOGIN_ENDED)),
                "the queued deliver is answered at its own address: {replies:?}"
            );
            assert!(
                matches!(
                    replies.last(),
                    Some((2, StoreReply::Auth(AuthFrame::Failed { .. })))
                ),
                "and the stream still ends with the flow's own last frame: {replies:?}"
            );
        }

        /// The credential rule (D273) through the worker: the pasted code is on no frame and in
        /// no `Debug` — the runtime's, the command's or the request's. The state is allowed: the
        /// link on screen already shows it.
        #[tokio::test]
        async fn no_frame_and_no_debug_carries_the_pasted_code() {
            let tmp = tempfile::tempdir().expect("a throwaway directory");
            let (port, bound) = listener().await;
            let link = loopback_link(port);
            let marker = tmp.path().join("delivered");
            let marker_text = marker.to_string_lossy().into_owned();
            let (store, agent_id) = login_store(
                tmp.path(),
                &[
                    ("FIXTURE_KEY", "set"),
                    ("FIXTURE_CRED", CREDENTIAL),
                    ("FIXTURE_URL", &link),
                    ("FIXTURE_WAIT_FOR", &marker_text),
                ],
            )
            .await;
            let backend = Backend::memory(store);
            let mut runtime = login_runtime(tmp.path());
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (release, released) = oneshot::channel();
            let head = answering_listener(bound, marker, released);

            let request = StoreRequest::AuthDeliver {
                url: pasted(port, STATE),
            };
            let command = AuthCommand::Deliver {
                url: pasted(port, STATE),
                reply: ReplyAddr {
                    seq: 3,
                    origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
                },
            };
            let mut printed = vec![format!("{request:?}"), format!("{command:?}")];

            start_login(&mut runtime, &backend, &tx, &mut rx, 1, agent_id).await;
            choose_and_await_link(&mut runtime, &backend, &tx, &mut rx).await;
            deliver_at(&mut runtime, &backend, &tx, 3, pasted(port, STATE)).await;
            // The listener has answered; the adapter is still waiting on the marker.
            let delivered = next_reply(&mut rx).await;
            assert_eq!(delivered.seq, 3);
            assert!(
                matches!(delivered.reply, StoreReply::Auth(AuthFrame::Delivered(_))),
                "the listener's answer is relayed"
            );
            printed.push(format!("{:?}", delivered.reply));
            printed.push(format!("{runtime:?}"));
            printed.push(format!("{:?}", runtime.auth));
            let _ = release.send(());

            for (_, reply) in replies_until_terminal(&mut rx).await {
                printed.push(format!("{reply:?}"));
            }
            assert!(
                tokio::time::timeout(PATIENCE, head)
                    .await
                    .expect("the delivery reached the listener")
                    .expect("the listener ran")
                    .contains(CODE),
                "and the code really did travel, to the listener alone"
            );
            for text in &printed {
                assert!(
                    !text.contains(CODE),
                    "a frame or a `Debug` carried the pasted code"
                );
            }

            runtime.finish_background(PATIENCE).await;
        }
    }

    // -----------------------------------------------------------------------------------------
    // MOD-7: the box probe (blueprint §6.5, §6.6; H-7, F-T)
    //
    // Every case runs over a fake `PATH` (`fake_env`) and fixed hardware (`fake_hardware`), and
    // every registry row is rewritten to the unresolvable launch first: nothing here spawns a
    // real host tool or agent, and nothing reads this box's hardware.
    // -----------------------------------------------------------------------------------------

    /// A box made of directories under `tmp`: `cwd` is `tmp`, `tmp/bin` is the whole `PATH`, and
    /// `home` is `None`, so no `~` pattern reaches the maintainer's home (blueprint F-T).
    pub(crate) fn fake_env(tmp: &std::path::Path) -> ProbeEnv {
        std::fs::create_dir_all(tmp.join("bin")).expect("the fixture bin");
        let mut vars = BTreeMap::new();
        vars.insert(
            "PATH".to_owned(),
            tmp.join("bin").to_string_lossy().into_owned(),
        );
        ProbeEnv {
            cwd: tmp.to_path_buf(),
            platform: htui_agent::probe::platform_key(),
            home: None,
            vars,
            versions: true,
            version_timeout: Duration::from_secs(5),
        }
    }

    /// Writes an executable `sh` script `name` into `tmp/bin`.
    ///
    /// Every script of a case is written before that case's first probe: a `fork` elsewhere while
    /// a write handle is open makes `execve` answer `ETXTBSY` (the gate runs `--test-threads=1`).
    #[cfg(unix)]
    pub(crate) fn script(tmp: &std::path::Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(tmp.join("bin")).expect("the fixture bin");
        let path = tmp.join("bin").join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    /// The two seeded tools every probing case finds: `cargo` (the `rust` tag) and `git`.
    #[cfg(unix)]
    fn standard_tools(tmp: &std::path::Path) {
        script(tmp, "git", "echo 'git version 2.43.0'");
        script(tmp, "cargo", "echo 'cargo 1.80.0 (376290515 2024-07-16)'");
    }

    /// Fixed facts: an AMD display device, so the seed names the GPU `amd` and derives `gpu`.
    pub(crate) fn fake_hardware() -> Arc<dyn HardwareSource> {
        Arc::new(box_probe::hardware::FixedHardware(
            box_probe::hardware::Hardware {
                os_version: "Test OS 1".to_owned(),
                cpu: "Test CPU".to_owned(),
                ram_mb: Some(2048),
                display_vendors: vec!["0x1002".to_owned()],
            },
        ))
    }

    /// The runtime every registration case drives: opted in, over the fake env and hardware.
    fn box_runtime(tmp: &std::path::Path) -> AgentRuntime {
        AgentRuntime::new(DriverFactory::production())
            .with_registration_probe()
            .with_probe_env(fake_env(tmp), fake_hardware())
    }

    /// The demo world with this box never probed, and every registry row unresolvable (H-7).
    pub(crate) async fn never_probed() -> MemStore {
        let mut data = htui_core::fixtures::demo_data();
        let this_box = data.this_box;
        for row in &mut data.boxes {
            if Some(row.id) == this_box {
                row.last_probed_at = None;
            }
        }
        let store = MemStore::from_demo(data);
        make_unresolvable(&store).await;
        store
    }

    /// This box's record, as `boxes()` answers it.
    async fn this_box_record(store: &MemStore) -> htui_core::model::BoxRecord {
        store
            .boxes()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|record| record.row.id == ids::BOX)
            .expect("this box is listed")
    }

    /// One `Online` swap as the store loop makes it: `on_online`, then the box task to its end.
    async fn swap(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        tx: &mpsc::UnboundedSender<ReplyEnvelope>,
    ) {
        runtime.on_online(backend, tx);
        runtime.finish_background(Duration::from_secs(10)).await;
    }

    /// Every reply sent so far.
    fn sent(rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>) -> Vec<ReplyEnvelope> {
        let mut replies = Vec::new();
        while let Ok(reply) = rx.try_recv() {
            replies.push(reply);
        }
        replies
    }

    /// The one `BoxProbed` among `replies`, at `UNSOLICITED` and `Origin::App`.
    fn the_report(replies: &[ReplyEnvelope]) -> BoxProbeReport {
        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        assert_eq!(replies[0].seq, UNSOLICITED);
        assert_eq!(replies[0].origin, Origin::App);
        let StoreReply::BoxProbed(report) = &replies[0].reply else {
            panic!("the box task answers BoxProbed: {:?}", replies[0].reply)
        };
        report.clone()
    }

    /// The seed's digest.
    #[cfg(unix)]
    fn seed_digest() -> String {
        box_probe::spec::digest(box_probe::spec::seed())
    }

    /// A probe recorded as `record` shows it, to plant in another store.
    #[cfg(unix)]
    fn as_probe(record: &htui_core::model::BoxRecord) -> htui_core::model::BoxProbe {
        htui_core::model::BoxProbe {
            box_id: record.row.id,
            os_version: record.row.os_version.clone(),
            cpu: record.row.cpu.clone(),
            ram_mb: record.row.ram_mb,
            gpu_present: record.row.gpu_present,
            gpu_vendor: record.row.gpu_vendor.clone(),
            tools: record
                .tools
                .iter()
                .map(|tool| htui_core::model::ProbedTool {
                    name: tool.name.clone(),
                    version: tool.version.clone(),
                    path: tool.path.clone(),
                })
                .collect(),
            probed_tags: record.row.probed_tags.clone(),
            htui_version: record.row.htui_version.clone(),
            spec_digest: record
                .probe_spec_digest
                .clone()
                .expect("the record was probed"),
            probed_at: record.row.last_probed_at.expect("the record was probed"),
        }
    }

    /// A stored overlay adding `terraform`, whose script `tool_scripts` writes.
    #[cfg(unix)]
    fn terraform_spec() -> Value {
        json!({
            "tools": {
                "terraform": {
                    "kind": "path",
                    "names": ["terraform"],
                    "version": { "args": ["version"], "pattern": "^Terraform v(\\S+)" }
                }
            }
        })
    }

    /// Plan D11: the first `Online` swap probes a box that was never probed, then its agents.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_online_swap_probes_a_box_that_was_never_probed() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        let record = this_box_record(&store).await;
        assert_eq!(record.row.os_version, "Test OS 1");
        assert_eq!(record.row.cpu, "Test CPU");
        assert_eq!(record.row.ram_mb, Some(2048));
        assert!(record.row.gpu_present);
        assert_eq!(record.row.gpu_vendor.as_deref(), Some("amd"));
        assert_eq!(record.row.htui_version, htui_store::HTUI_VERSION);
        assert!(record.row.last_probed_at.is_some());
        assert_eq!(record.probe_spec_digest, Some(seed_digest()));
        let tools: Vec<(&str, &str)> = record
            .tools
            .iter()
            .map(|tool| (tool.name.as_str(), tool.version.as_str()))
            .collect();
        assert_eq!(tools, [("cargo", "1.80.0"), ("git", "2.43.0")]);
        assert_eq!(record.row.probed_tags, ["gpu", "rust"]);

        let agents = store.agents().await.expect("the memory store never fails");
        assert!(agents.iter().any(|summary| summary.agent.enabled));
        for summary in agents.iter().filter(|summary| summary.agent.enabled) {
            let status = summary
                .on_box
                .as_ref()
                .and_then(ProbeSnapshot::from_row)
                .map(|snapshot| snapshot.status);
            assert!(
                matches!(status, Some(ProbeStatus::Missing)),
                "{} was probed on this box: {status:?}",
                summary.agent.name
            );
        }

        let report = the_report(&sent(&mut rx));
        assert_eq!(report.tools, 2);
        assert_eq!(report.probed_tags, ["gpu", "rust"]);
        assert_eq!(report.box_failed, None);
        assert_eq!(report.agents_failed, None);
        assert_eq!(report.spec_error, None);
    }

    /// Plan D5, D18: a reconnect at the same version and spec costs reads, not a probe.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_online_swap_skips_a_box_probed_at_this_version_and_spec() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;
        the_report(&sent(&mut rx));
        let first = this_box_record(&store).await;

        swap(&mut runtime, &backend, &tx).await;
        let replies = sent(&mut rx);
        assert!(replies.is_empty(), "no probe, no reply: {replies:?}");
        let second = this_box_record(&store).await;
        assert_eq!(second.row.last_probed_at, first.row.last_probed_at);
        assert_eq!(second.row.updated_at, first.row.updated_at);
    }

    /// Plan D5: a box probed by another `htui` is probed again.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_online_swap_reprobes_after_a_version_change() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        let store = never_probed().await;
        store
            .record_box_probe(&htui_core::model::BoxProbe {
                box_id: ids::BOX,
                os_version: "Old OS".to_owned(),
                cpu: "Old CPU".to_owned(),
                ram_mb: None,
                gpu_present: false,
                gpu_vendor: None,
                tools: Vec::new(),
                probed_tags: Vec::new(),
                htui_version: "0.0.0".to_owned(),
                spec_digest: seed_digest(),
                probed_at: Utc::now() - chrono::TimeDelta::hours(1),
            })
            .await
            .expect("the old probe lands");
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        the_report(&sent(&mut rx));
        let record = this_box_record(&store).await;
        assert_eq!(record.row.htui_version, htui_store::HTUI_VERSION);
        assert_eq!(record.row.os_version, "Test OS 1");
        assert_eq!(record.tools.len(), 2);
    }

    /// Blueprint D25: `ProbeBox` is refused before anything spawns, offline and unregistered.
    #[tokio::test]
    async fn probe_box_is_refused_offline_before_spawning_anything() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "box-probe-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let tmp = tempfile::tempdir().expect("temp box");
        let mut runtime = box_runtime(tmp.path());
        let (tx, _rx) = mpsc::unbounded_channel();

        let served = runtime
            .serve(&backend, &tx, &envelope(1, StoreRequest::ProbeBox))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_box");
                assert!(
                    message.contains(htui_store::REGISTRY_ON_SERVER_ONLY),
                    "the offline sentence: {message}"
                );
            }
            other => panic!("an offline backend refuses the box probe: {other:?}"),
        }
        assert!(!runtime.box_probe_running());
        assert_eq!(runtime.background_len(), 0);
        cache.close().await;

        let backend = Backend::memory(MemStore::new());
        let served = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeBox))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_box");
                assert!(message.contains("not registered"), "{message}");
            }
            other => panic!("an unregistered box refuses the box probe: {other:?}"),
        }
        assert!(!runtime.box_probe_running());
        assert_eq!(runtime.background_len(), 0);
    }

    /// Blueprint D25: the `ProbeBox` task answers once, at the request's own address.
    #[tokio::test]
    async fn the_probe_box_task_answers_once_at_the_request_s_address() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = RequestEnvelope {
            seq: 7,
            origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
            request: StoreRequest::ProbeBox,
        };

        let served = runtime.serve(&backend, &tx, &request).await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert!(runtime.box_probe_running());
        runtime.finish_background(Duration::from_secs(10)).await;

        let replies = sent(&mut rx);
        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        assert_eq!(replies[0].seq, 7);
        assert!(
            matches!(&replies[0].origin, Origin::Tab(id) if id.0 == "settings"),
            "{:?}",
            replies[0].origin
        );
        let StoreReply::BoxProbed(report) = &replies[0].reply else {
            panic!("the box task answers BoxProbed: {:?}", replies[0].reply)
        };
        assert_eq!(report.box_failed, None);
        assert!(this_box_record(&store).await.row.last_probed_at.is_some());
    }

    /// PRD D7: a `missing` agent that declares an install source is named, never installed.
    #[tokio::test]
    async fn a_missing_agent_with_an_install_source_is_named_and_not_installed() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let agent = Agent {
            name: "installable".to_owned(),
            transport: Transport::Acp,
            launch: json!({
                "command": "${gone}",
                "args": [],
                "env": {},
                "discovery": {
                    "tools": {
                        "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                    },
                    "handshake": true,
                    "install": { "source": "acp_registry", "id": INSTALL_ID, "tool": "gone" }
                }
            }),
            settings: json!({}),
            ..fake_row(AgentId::new())
        };
        store
            .upsert_agent(&agent, None)
            .await
            .expect("the row lands");
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        let replies = sent(&mut rx);
        assert!(
            replies
                .iter()
                .all(|reply| !matches!(reply.reply, StoreReply::Install(_))),
            "no install frame: {replies:?}"
        );
        let report = the_report(&replies);
        assert_eq!(report.installable, ["installable"]);
        assert!(
            report
                .status_line()
                .contains("installable is missing and can be installed: Settings > Agents, i"),
            "the install offer: {}",
            report.status_line()
        );
        assert!(!runtime.install_running());
    }

    /// Blueprint D25: the status line is the head, then the install offer, the agent half's
    /// failure and the ignored spec, in that order and each only when there is one.
    #[test]
    fn the_status_line_reads_each_part_in_order() {
        let probed = BoxProbeReport {
            tools: 2,
            probed_tags: vec!["gpu".to_owned(), "rust".to_owned()],
            ..BoxProbeReport::default()
        };
        assert_eq!(probed.status_line(), "box probed: 2 tools · tags gpu, rust");

        assert_eq!(
            BoxProbeReport::default().status_line(),
            "box probed: 0 tools · no tags"
        );

        let failed = BoxProbeReport {
            box_failed: Some("x".to_owned()),
            ..BoxProbeReport::default()
        };
        assert_eq!(failed.status_line(), "box probe failed: x");

        let one = BoxProbeReport {
            installable: vec!["a".to_owned()],
            ..BoxProbeReport::default()
        };
        assert_eq!(
            one.status_line(),
            "box probed: 0 tools · no tags · a is missing and can be installed: Settings > Agents, i"
        );

        let two = BoxProbeReport {
            installable: vec!["a".to_owned(), "b".to_owned()],
            ..BoxProbeReport::default()
        };
        assert_eq!(
            two.status_line(),
            "box probed: 0 tools · no tags · a, b are missing and can be installed: \
             Settings > Agents, i"
        );

        let all = BoxProbeReport {
            installable: vec!["a".to_owned()],
            agents_failed: Some("y".to_owned()),
            spec_error: Some("z".to_owned()),
            ..probed
        };
        assert_eq!(
            all.status_line(),
            "box probed: 2 tools · tags gpu, rust · a is missing and can be installed: \
             Settings > Agents, i · agent probe failed: y · z"
        );

        let unchanged = BoxProbeReport {
            unchanged: true,
            ..all
        };
        assert_eq!(unchanged.status_line(), "box probe unchanged · z");
    }

    /// Blueprint D27: an install is refused while the registration probe holds the claim.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_install_is_refused_while_the_registration_probe_runs() {
        let tmp = tempfile::tempdir().expect("temp box");
        script(
            tmp.path(),
            "cmake",
            "/bin/sleep 2; echo 'cmake version 3.28.0'",
        );
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path()).with_installer(InstallConfig::new(
            "http://127.0.0.1:1".to_owned(),
            Some(tmp.path().join("agents")),
        ));
        let (tx, _rx) = mpsc::unbounded_channel();

        runtime.on_online(&backend, &tx);
        assert!(runtime.box_probe_running());
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
                assert!(message.contains(BOX_PROBE_RUNNING), "{message}");
            }
            other => panic!("the install is refused while the box probe runs: {other:?}"),
        }
        assert!(!runtime.install_running());
        runtime.finish_background(Duration::from_secs(10)).await;
    }

    /// Blueprint D27: `ProbeAgents` is refused while the registration probe holds the claim.
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_agents_is_refused_while_the_registration_probe_runs() {
        let tmp = tempfile::tempdir().expect("temp box");
        script(
            tmp.path(),
            "cmake",
            "/bin/sleep 2; echo 'cmake version 3.28.0'",
        );
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, _rx) = mpsc::unbounded_channel();

        runtime.on_online(&backend, &tx);
        assert!(runtime.box_probe_running());
        let served = runtime
            .serve(&backend, &tx, &envelope(1, StoreRequest::ProbeAgents))
            .await;
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(message.contains(BOX_PROBE_RUNNING), "{message}");
            }
            other => panic!("the agent probe is refused while the box probe runs: {other:?}"),
        }
        assert_eq!(runtime.background_len(), 0);
        runtime.finish_background(Duration::from_secs(10)).await;
    }

    /// MOD-2 D51: a hand-written `agent_box` row survives the registration probe byte for byte.
    #[tokio::test]
    async fn a_manual_agent_row_survives_the_registration_probe() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let agent_id = store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.enabled)
            .expect("an enabled demo row")
            .agent
            .id;
        let old = Utc::now() - chrono::TimeDelta::days(3);
        store
            .upsert_agent_box(&AgentBox {
                agent_id,
                box_id: ids::BOX,
                enabled: true,
                version: Some("hand-written".to_owned()),
                path: Some("/opt/agent".to_owned()),
                probed_at: Some(old),
                quota: None,
                quota_at: None,
                updated_at: old,
                probe: Some(json!({
                    "transport": "acp",
                    "resolved": null,
                    "tools": {},
                    "handshake": null,
                    "status": "ready",
                    "stderr_tail": null,
                    "source": "manual",
                })),
            })
            .await
            .expect("the manual row lands");
        let on_box = |agents: Vec<htui_core::model::AgentSummary>| {
            agents
                .into_iter()
                .find(|summary| summary.agent.id == agent_id)
                .and_then(|summary| summary.on_box)
                .expect("the manual row is there")
        };
        let before = on_box(store.agents().await.expect("the memory store never fails"));
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        the_report(&sent(&mut rx));
        let after = on_box(store.agents().await.expect("the memory store never fails"));
        assert_eq!(
            after, before,
            "the probe found nothing, so it wrote nothing"
        );
    }

    /// Blueprint D27: a finished background task does not hold the claim — `sweep_finished` clears
    /// it before `on_online` asks. The entry below is tagged `writing` on purpose, so the sweep is
    /// the only thing that frees the claim here (MOD-31 D2); a `reading` entry would be free
    /// already, and the sweep would be untested.
    #[tokio::test]
    async fn a_finished_background_task_does_not_stop_the_registration_probe() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, _rx) = mpsc::unbounded_channel();

        runtime
            .background
            .push(Background::writing(tokio::spawn(async {})));
        while !runtime
            .background
            .iter()
            .all(|entry| entry.task().is_finished())
        {
            tokio::task::yield_now().await;
        }
        runtime.on_online(&backend, &tx);
        assert!(runtime.box_probe_running());
        assert_eq!(runtime.background_len(), 0, "the finished task was swept");
        runtime.finish_background(Duration::from_secs(10)).await;
        assert!(this_box_record(&store).await.row.last_probed_at.is_some());
    }

    /// R-12: a claim held elsewhere skips the registration probe until the next swap, and says
    /// nothing: no box probe starts and no reply is sent.
    #[tokio::test]
    async fn a_held_claim_skips_the_registration_probe_without_a_reply() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        runtime.background.push(Background::writing(tokio::spawn(
            std::future::pending::<()>(),
        )));
        runtime.on_online(&backend, &tx);

        assert!(!runtime.box_probe_running());
        assert_eq!(runtime.background_len(), 1, "the held task is still there");
        assert!(sent(&mut rx).is_empty(), "a skipped probe says nothing");
        for entry in std::mem::take(&mut runtime.background) {
            entry.into_task().abort();
        }
        assert_eq!(this_box_record(&store).await.row.last_probed_at, None);
    }

    /// Plan D11 and the `connection.rs` shape: a runtime that did not opt in never probes.
    #[tokio::test]
    async fn production_runtime_does_not_auto_probe_without_opt_in() {
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::production();
        let (tx, mut rx) = mpsc::unbounded_channel();

        runtime.on_online(&backend, &tx);

        assert!(!runtime.box_probe_running());
        assert_eq!(runtime.background_len(), 0);
        runtime.finish_background(Duration::from_secs(1)).await;
        assert!(sent(&mut rx).is_empty());
        assert_eq!(this_box_record(&store).await.row.last_probed_at, None);
    }

    /// Blueprint F-E: the install pre-flight and the Settings section ask one function.
    #[test]
    fn the_install_pre_flight_and_the_section_share_declares_install() {
        let declared = install_row(AgentId::new(), "declared", true).launch;
        let undeclared = install_row(AgentId::new(), "undeclared", false).launch;
        for (launch, expected) in [
            (declared, true),
            (undeclared, false),
            (json!("this is not a launch document"), false),
        ] {
            let agent = Agent {
                launch,
                ..fake_row(AgentId::new())
            };
            assert_eq!(
                declares_a_source(&agent).is_ok(),
                htui_agent::launch::declares_install(&agent.launch)
            );
            assert_eq!(declares_a_source(&agent).is_ok(), expected);
        }
    }

    /// Plan D18: a stored spec that adds a tool changes the digest, so the next swap probes it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stored_spec_adds_a_tool_and_reprobes_at_the_next_swap() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        script(tmp.path(), "terraform", "echo 'Terraform v1.9.0'");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;
        the_report(&sent(&mut rx));
        let first = this_box_record(&store).await;
        assert!(first.tools.iter().all(|tool| tool.name != "terraform"));

        store.set_app_setting(box_probe::spec::SETTING_KEY, terraform_spec());
        swap(&mut runtime, &backend, &tx).await;

        let report = the_report(&sent(&mut rx));
        assert_eq!(report.tools, 3);
        let second = this_box_record(&store).await;
        let terraform = second
            .tools
            .iter()
            .find(|tool| tool.name == "terraform")
            .expect("the overlay's tool was probed");
        assert_eq!(terraform.version, "1.9.0");
        assert_ne!(second.probe_spec_digest, first.probe_spec_digest);
        let overlay = terraform_spec();
        assert_eq!(
            second.probe_spec_digest,
            Some(box_probe::spec::effective(box_probe::spec::seed(), Some(&overlay)).digest)
        );
    }

    /// Plan D18: a stored spec that has not changed does not probe again.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unchanged_spec_does_not_reprobe() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        script(tmp.path(), "terraform", "echo 'Terraform v1.9.0'");
        let store = never_probed().await;
        store.set_app_setting(box_probe::spec::SETTING_KEY, terraform_spec());
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;
        the_report(&sent(&mut rx));
        let first = this_box_record(&store).await;
        swap(&mut runtime, &backend, &tx).await;

        assert!(sent(&mut rx).is_empty(), "one probe for one spec");
        let second = this_box_record(&store).await;
        assert_eq!(second.row.last_probed_at, first.row.last_probed_at);
        assert_eq!(second.probe_spec_digest, first.probe_spec_digest);
    }

    /// Plan D18: removing the stored spec takes the digest back to the seed's, which probes.
    ///
    /// `MemStore` has no way to remove an `app_setting` row, and `mem.rs` is not T4's to change,
    /// so the "after" store is rebuilt from the "before" store's recorded probe with no setting
    /// (blueprint §6.6).
    #[cfg(unix)]
    #[tokio::test]
    async fn removing_the_stored_spec_reprobes_with_the_seed() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        script(tmp.path(), "terraform", "echo 'Terraform v1.9.0'");
        let before = never_probed().await;
        before.set_app_setting(box_probe::spec::SETTING_KEY, terraform_spec());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        swap(&mut runtime, &Backend::memory(before.clone()), &tx).await;
        the_report(&sent(&mut rx));
        let recorded = this_box_record(&before).await;
        assert_ne!(recorded.probe_spec_digest, Some(seed_digest()));

        let after = never_probed().await;
        after
            .record_box_probe(&as_probe(&recorded))
            .await
            .expect("the recorded probe lands");
        swap(&mut runtime, &Backend::memory(after.clone()), &tx).await;

        let report = the_report(&sent(&mut rx));
        assert_eq!(report.tools, 2);
        let record = this_box_record(&after).await;
        assert_eq!(record.probe_spec_digest, Some(seed_digest()));
        assert!(record.tools.iter().all(|tool| tool.name != "terraform"));
    }

    /// Plan D17: an overlay that does not merge is ignored, the seed probed, and the report says
    /// why.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_invalid_stored_spec_probes_the_seed_and_the_report_says_so() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        let store = never_probed().await;
        store.set_app_setting(box_probe::spec::SETTING_KEY, json!(42));
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        let report = the_report(&sent(&mut rx));
        let error = report
            .spec_error
            .clone()
            .expect("the ignored overlay is named");
        assert!(error.starts_with(box_probe::spec::SPEC_IGNORED), "{error}");
        assert!(report.status_line().contains(&error));
        let record = this_box_record(&store).await;
        assert_eq!(record.probe_spec_digest, Some(seed_digest()));
        assert_eq!(record.tools.len(), 2);
        assert_eq!(report.tools, 2);
    }

    /// Plan D17: an ignored overlay is named at every swap, even one that leaves the box alone.
    ///
    /// `json!(42)` does not merge, so the effective spec is the seed's and its digest matches the
    /// first probe's: the box is not probed again, and exactly one `unchanged` report says why
    /// the stored spec was ignored.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_invalid_stored_spec_is_named_on_a_swap_that_does_not_probe() {
        let tmp = tempfile::tempdir().expect("temp box");
        standard_tools(tmp.path());
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = box_runtime(tmp.path());
        let (tx, mut rx) = mpsc::unbounded_channel();
        swap(&mut runtime, &backend, &tx).await;
        assert_eq!(the_report(&sent(&mut rx)).spec_error, None);
        let first = this_box_record(&store).await;

        store.set_app_setting(box_probe::spec::SETTING_KEY, json!(42));
        swap(&mut runtime, &backend, &tx).await;

        let report = the_report(&sent(&mut rx));
        assert!(report.unchanged, "the box was not probed again: {report:?}");
        let error = report
            .spec_error
            .clone()
            .expect("the ignored overlay is named");
        assert!(error.starts_with(box_probe::spec::SPEC_IGNORED), "{error}");
        assert_eq!(
            report,
            BoxProbeReport {
                unchanged: true,
                spec_error: Some(error.clone()),
                ..BoxProbeReport::default()
            }
        );
        assert_eq!(
            report.status_line(),
            format!("box probe unchanged · {error}")
        );
        let second = this_box_record(&store).await;
        assert_eq!(second.row.last_probed_at, first.row.last_probed_at);
        assert_eq!(second.probe_spec_digest, Some(seed_digest()));
    }

    /// Blueprint D28: the version a probe records is the binary's own.
    #[test]
    fn htui_version_is_the_binary_s_version() {
        assert_eq!(htui_store::HTUI_VERSION, env!("CARGO_PKG_VERSION"));
    }

    // -----------------------------------------------------------------------------------------
    // MOD-53: a task that panics still answers its request, and leaves the terminal alone
    // -----------------------------------------------------------------------------------------

    /// Hardware whose read panics: the box probe's own seam, used to put a panic inside the task.
    #[derive(Debug)]
    struct PanickingHardware;

    impl HardwareSource for PanickingHardware {
        fn read<'a>(&'a self, _env: &'a ProbeEnv) -> box_probe::hardware::HardwareFuture<'a> {
            Box::pin(async { panic!("the hardware read blew up") })
        }
    }

    /// A transport whose sessions panic as they start.
    #[derive(Debug)]
    struct PanickingDriver;

    impl AgentDriver for PanickingDriver {
        fn name(&self) -> &str {
            "panicking-fixture"
        }

        fn caps(&self) -> DriverCaps {
            DriverCaps::default()
        }

        fn start<'a>(
            &'a self,
            _spec: SessionSpec,
            _prompt: String,
        ) -> htui_agent::driver::DriverFuture<'a, Box<dyn AgentSession>> {
            Box::pin(async { panic!("the adapter blew up on start") })
        }
    }

    #[derive(Debug)]
    struct PanickingBuilder;

    impl htui_agent::registry::TransportBuilder for PanickingBuilder {
        fn build(
            &self,
            _agent: &Agent,
            _on_box: Option<&AgentBox>,
            _caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            Ok(Box::new(PanickingDriver))
        }
    }

    fn settings_addr(seq: Seq) -> ReplyAddr {
        ReplyAddr {
            seq,
            origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
        }
    }

    /// The wrapper's whole contract: a panic becomes the task's last word, at the stream's address
    /// as it is when the panic lands, and the terminal is not given back on the way.
    #[tokio::test]
    async fn a_panicking_task_answers_with_its_last_word_at_the_stream_s_address() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let frames = Frames::new(tx, settings_addr(3));
        let answer = frames.answer(probe_agents_failed);
        let contained = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&contained);

        answering(
            "agent probe",
            async move {
                // The stream moves, as a chat's does when a later request adopts it.
                frames.stream.lock().addr = settings_addr(9);
                *seen.lock().expect("unpoisoned") = Some(!crate::terminal::restores_the_terminal());
                tokio::task::yield_now().await;
                panic!("probe went sideways");
            },
            Some(answer),
        )
        .await;

        let replies = sent(&mut rx);
        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        assert_eq!(replies[0].seq, 9, "at the moved address");
        match &replies[0].reply {
            StoreReply::Failed { request, message } => {
                assert_eq!(*request, "probe_agents");
                assert_eq!(
                    message,
                    "the agent probe task panicked: probe went sideways"
                );
            }
            other => panic!("the last word: {other:?}"),
        }
        assert_eq!(
            *contained.lock().expect("unpoisoned"),
            Some(true),
            "the hook would have left the terminal alone"
        );
        assert!(
            crate::terminal::restores_the_terminal(),
            "and the window closed again"
        );
    }

    /// A task that ends normally owes no last word, and a `String` payload reads as itself.
    #[tokio::test]
    async fn only_a_panic_sends_the_last_word() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let answer = Answer::at(tx.clone(), settings_addr(1), install_failed);
        answering("install", async {}, Some(answer)).await;
        assert!(sent(&mut rx).is_empty(), "a clean exit adds nothing");

        let answer = Answer::at(tx, settings_addr(2), install_failed);
        let code = 7;
        answering(
            "install",
            async move { panic!("exit code {code}") },
            Some(answer),
        )
        .await;
        let replies = sent(&mut rx);
        assert!(
            matches!(
                &replies[..],
                [ReplyEnvelope { seq: 2, reply: StoreReply::Install(InstallFrame::Failed { message, manual: None }), .. }]
                    if message == "the install task panicked: exit code 7"
            ),
            "{replies:?}"
        );
    }

    /// The case the item was found on: a box probe that panics clears the section's `probing`,
    /// because the `probe_box` failure arrives, and the slot is free for the next `p`.
    #[tokio::test]
    async fn a_box_probe_that_panics_answers_probe_box_and_frees_the_slot() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = never_probed().await;
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(DriverFactory::production())
            .with_probe_env(fake_env(tmp.path()), Arc::new(PanickingHardware));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = RequestEnvelope {
            seq: 7,
            origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
            request: StoreRequest::ProbeBox,
        };

        let served = runtime.serve(&backend, &tx, &request).await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        let first = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("the panicked probe answers")
            .expect("the channel is open");
        // Not `finish_background`, which takes the slot itself: the task has to finish on its
        // own for the sweep inside the next `serve` to release it.
        for _ in 0..1_000 {
            if !runtime.box_probe_running() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!runtime.box_probe_running(), "the panicked task finished");

        let mut replies = vec![first];
        replies.extend(sent(&mut rx));
        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        assert_eq!(replies[0].seq, 7);
        match &replies[0].reply {
            StoreReply::Failed { request, message } => {
                assert_eq!(*request, StoreRequest::ProbeBox.name());
                assert!(
                    message.contains("the hardware read blew up"),
                    "the panic is named: {message}"
                );
            }
            other => panic!("a panicked box probe answers probe_box: {other:?}"),
        }

        let again = RequestEnvelope { seq: 8, ..request };
        let served = runtime.serve(&backend, &tx, &again).await;
        assert!(
            matches!(served, Served::Deferred),
            "the slot was released, so a second probe starts: {served:?}"
        );
        runtime.finish_background(Duration::from_secs(10)).await;
    }

    thread_local! {
        /// MOD-24 review L1: the run of every chat `serve` minted on this thread, by its step.
        /// No store read goes from a step to its run, and a case only learns the step
        /// (`Served::Start { step_id }`).
        static MINTED: std::cell::RefCell<HashMap<StepId, RunId>> =
            std::cell::RefCell::default();
    }

    /// Records `chat`'s pair for [`run_of`]; called by `serve` in test builds only.
    pub(crate) fn minted(chat: &ChatRunSpec) {
        MINTED.with(|minted| minted.borrow_mut().insert(chat.step_id, chat.run_id));
    }

    /// The chat run `serve` minted for `step` on this thread.
    fn run_of(step: StepId) -> RunId {
        MINTED
            .with(|minted| minted.borrow().get(&step).copied())
            .expect("serve minted this step's run on this thread")
    }

    /// MOD-24 review L1: `task` spawned, its stream received up to and including its `Failed`
    /// frame, and at that moment the chat's run already closed: `run` `failed` with `finished_at`
    /// set, and `step` with it. Then the task's end and the rest of its stream; every reply.
    async fn closed_before_failed(
        store: &MemStore,
        rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
        task: ChatTask,
        step: StepId,
    ) -> Vec<ReplyEnvelope> {
        let run = run_of(step);
        let handle = tokio::spawn(task);
        let mut replies = Vec::new();
        loop {
            let reply = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("the stream's `Failed` arrives")
                .expect("the stream stays open until its last word");
            let failed = matches!(reply.reply, StoreReply::Chat(ChatFrame::Failed { .. }));
            replies.push(reply);
            if failed {
                break;
            }
        }
        let row = store
            .run(run)
            .await
            .expect("the read answers")
            .expect("the chat's run");
        assert_eq!(
            row.status,
            RunStatus::Failed,
            "the run was closed before the `Failed` frame went out"
        );
        assert!(row.finished_at.is_some(), "and its `finished_at` is set");
        let steps = store.run_steps(run).await.expect("the read answers");
        let chat_step = steps
            .iter()
            .find(|row| row.id == step)
            .expect("the run's step is the chat's");
        assert_eq!(
            chat_step.status,
            StepStatus::Failed,
            "the step closed with it"
        );
        handle.await.expect("the wrapper caught the panic");
        replies.extend(sent(rx));
        replies
    }

    /// A chat whose session panics ends its stream with a `Failed` frame, which is what clears the
    /// chat tab's pending start, and closes the run it opened first (MOD-24 D4).
    #[tokio::test]
    async fn a_chat_that_panics_ends_its_stream_with_failed_then_ended() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&fake_row(agent_id), None)
            .await
            .expect("the fake row lands");
        let mut factory = DriverFactory::new();
        factory.register("cli/fake", Box::new(PanickingBuilder));
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let before = store.active_runs(&scope()).await.expect("count");

        let Served::Start { step_id, task } = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hi")))
            .await
        else {
            panic!("a chat start opens a session")
        };
        let replies = closed_before_failed(&store, &mut rx, task, step_id).await;

        let [.., failed, ended] = &replies[..] else {
            panic!("the stream ends with two frames: {replies:?}")
        };
        assert_eq!((failed.seq, ended.seq), (7, 7));
        assert!(
            matches!(
                &failed.reply,
                StoreReply::Chat(ChatFrame::Failed { message })
                    if message.contains("the adapter blew up on start")
            ),
            "{replies:?}"
        );
        // And then `Ended`, as a transport failure ends it: an accepted session in the tab would
        // otherwise go on sending to a chat the runtime has already swept.
        assert!(
            matches!(
                &ended.reply,
                StoreReply::Chat(ChatFrame::Ended {
                    stop_reason: StopReason::Cancelled
                })
            ),
            "{replies:?}"
        );
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "the panicked chat's run is closed"
        );
    }

    /// A transport whose sessions start cleanly and panic on their first pull: a panic **mid-turn**,
    /// past `run_chat`'s own close of a failed start, so only the wrapper is left to close the run
    /// (MOD-24 D4).
    #[derive(Debug)]
    struct PanicsMidTurn;

    impl AgentDriver for PanicsMidTurn {
        fn name(&self) -> &str {
            "panics-mid-turn-fixture"
        }

        fn caps(&self) -> DriverCaps {
            DriverCaps::default()
        }

        fn start<'a>(
            &'a self,
            _spec: SessionSpec,
            _prompt: String,
        ) -> htui_agent::driver::DriverFuture<'a, Box<dyn AgentSession>> {
            Box::pin(async { Ok(Box::new(PanicsOnPull) as Box<dyn AgentSession>) })
        }
    }

    /// [`PanicsMidTurn`]'s session: accepted, then a panic on the turn's first `next_event`.
    #[derive(Debug)]
    struct PanicsOnPull;

    impl AgentSession for PanicsOnPull {
        fn session_ref(&self) -> Option<&AgentSessionRef> {
            None
        }

        fn next_event<'a>(
            &'a mut self,
        ) -> htui_agent::driver::DriverFuture<'a, Option<DriverEnvelope>> {
            Box::pin(async { panic!("the adapter blew up mid-turn") })
        }

        fn send_follow_up<'a>(
            &'a mut self,
            _text: String,
        ) -> htui_agent::driver::DriverFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn answer_permission<'a>(
            &'a mut self,
            _request_id: PermissionRequestId,
            _answer: PermissionAnswer,
        ) -> htui_agent::driver::DriverFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn cancel<'a>(&'a mut self, _grace: Duration) -> htui_agent::driver::DriverFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Debug)]
    struct PanicsMidTurnBuilder;

    impl htui_agent::registry::TransportBuilder for PanicsMidTurnBuilder {
        fn build(
            &self,
            _agent: &Agent,
            _on_box: Option<&AgentBox>,
            _caps: DriverCaps,
        ) -> Result<Box<dyn AgentDriver>, DriverError> {
            Ok(Box::new(PanicsMidTurn))
        }
    }

    /// The demo store plus the fake row, and a runtime whose `cli/fake` sessions panic mid-turn.
    async fn panics_mid_turn() -> (MemStore, Backend, AgentRuntime, AgentId) {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&fake_row(agent_id), None)
            .await
            .expect("the fake row lands");
        let mut factory = DriverFactory::new();
        factory.register("cli/fake", Box::new(PanicsMidTurnBuilder));
        let backend = Backend::memory(store.clone());
        let runtime = AgentRuntime::new(factory).with_grace(Duration::from_millis(0));
        (store, backend, runtime, agent_id)
    }

    /// The stream's last two frames: a `Failed` naming `panic`, then `Ended { Cancelled }`, both
    /// at seq 7.
    fn ends_failed_then_ended(replies: &[ReplyEnvelope], panic: &str) {
        let [.., failed, ended] = replies else {
            panic!("the stream ends with two frames: {replies:?}")
        };
        assert_eq!((failed.seq, ended.seq), (7, 7), "{replies:?}");
        assert!(
            matches!(
                &failed.reply,
                StoreReply::Chat(ChatFrame::Failed { message }) if message.contains(panic)
            ),
            "{replies:?}"
        );
        assert!(
            matches!(
                &ended.reply,
                StoreReply::Chat(ChatFrame::Ended {
                    stop_reason: StopReason::Cancelled
                })
            ),
            "{replies:?}"
        );
    }

    /// MOD-24 D4: a fresh chat whose session panics mid-turn closes its `run(kind='chat')` pair
    /// before its stream ends, so a tab that re-reads runs on `Ended` finds no chat still running.
    #[tokio::test]
    async fn a_chat_that_panics_mid_turn_closes_its_run_then_ends_its_stream() {
        let (store, backend, mut runtime, agent_id) = panics_mid_turn().await;
        let before = store.active_runs(&scope()).await.expect("count");
        let (tx, mut rx) = mpsc::unbounded_channel();

        let Served::Start { step_id, task } = runtime
            .serve(&backend, &tx, &envelope(7, start(agent_id, "hi")))
            .await
        else {
            panic!("a chat start opens a session")
        };
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before + 1,
            "the start opened the chat's run"
        );
        let replies = closed_before_failed(&store, &mut rx, task, step_id).await;

        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.reply, StoreReply::ChatAccepted { .. })),
            "the session was accepted before it panicked: {replies:?}"
        );
        ends_failed_then_ended(&replies, "the adapter blew up mid-turn");
        assert_eq!(
            store.active_runs(&scope()).await.expect("count"),
            before,
            "the panicked chat's run is closed"
        );
    }

    /// MOD-24 review L3 (blueprint H-19): a chat task that panics **after** its session closed its
    /// run normally (`run_chat`'s last `binding.close`) leaves the run as that close left it. The
    /// binding and the answer share one closed flag, so the answer's own close is skipped and
    /// `done` is never rewritten as `failed`; the last word still goes out. No transport can panic
    /// past that close today (only `frames.ended` follows it), so the case drives the two halves
    /// directly: the binding's close, then the answer a panic would send.
    #[tokio::test]
    async fn a_panic_after_a_normal_close_leaves_the_chat_run_as_closed() {
        let store = MemStore::demo();
        let backend = Backend::memory(store.clone());
        let writer = backend
            .writer()
            .expect("a memory backend hands out a writer");
        let chat = ChatRunSpec::mint(ids::PROJECT_HTUI, ids::BOX, ids::USER, None, None);
        writer
            .start_chat_run(&chat)
            .await
            .expect("the chat's run lands");
        let closed = Arc::new(AtomicBool::new(false));
        let binding = ChatBinding::Fresh(chat.clone(), Arc::clone(&closed));
        binding.close(&writer, RunStatus::Done).await;
        let (tx, mut rx) = mpsc::unbounded_channel();

        Frames::new(tx, promote_addr())
            .answer(chat_failed)
            .closing(writer, chat.clone(), closed)
            .send("the chat task panicked: past its close".to_owned())
            .await;

        let row = store
            .run(chat.run_id)
            .await
            .expect("the read answers")
            .expect("the chat's run");
        assert_eq!(row.status, RunStatus::Done, "the normal close stands");
        let steps = store
            .run_steps(chat.run_id)
            .await
            .expect("the read answers");
        assert_eq!(
            steps.iter().map(|step| step.status).collect::<Vec<_>>(),
            [StepStatus::Done],
            "and so does its step's"
        );
        ends_failed_then_ended(&sent(&mut rx), "past its close");
    }

    /// MOD-24 D4 leaves MOD-4 D165 alone: a promoted step's chat opened no run, so its panic closes
    /// nothing; the step and its run stay as the promotion left them.
    #[tokio::test]
    async fn a_promoted_chat_that_panics_closes_nothing() {
        let (store, backend, mut runtime, agent_id) = panics_mid_turn().await;
        let run_before = store
            .run(ids::RUN_1)
            .await
            .expect("the read answers")
            .expect("the fixture's run");
        let step_of = async |store: &MemStore| {
            store
                .run_steps(ids::RUN_1)
                .await
                .expect("the read answers")
                .into_iter()
                .find(|step| step.id == ids::STEP_PLAN)
                .expect("the fixture's step")
        };
        let step_before = step_of(&store).await;
        let active = store.active_runs(&scope()).await.expect("count");
        let (tx, mut rx) = mpsc::unbounded_channel();

        let promotion = promoted(
            agent_id,
            OpeningPath::Handoff {
                text: "pick up where the step stopped".to_owned(),
                digest: "d-handoff".to_owned(),
            },
        );
        let Served::Start { task, .. } = runtime
            .attach_promoted(&backend, &tx, promote_addr(), promotion)
            .await
        else {
            panic!("a promotion over a registered row opens a session")
        };
        task.await;

        ends_failed_then_ended(&sent(&mut rx), "the adapter blew up mid-turn");
        let run_after = store
            .run(ids::RUN_1)
            .await
            .expect("the read answers")
            .expect("the fixture's run");
        assert_eq!(run_after.status, run_before.status, "the run is untouched");
        assert_eq!(run_after.finished_at, run_before.finished_at);
        assert_eq!(
            step_of(&store).await.status,
            step_before.status,
            "so is the step"
        );
        assert_eq!(store.active_runs(&scope()).await.expect("count"), active);
    }

    /// The registration probe answers at `UNSOLICITED`, where only a `BoxProbed` is rendered, so a
    /// panic there is a report with `box_failed` set rather than a `Failed` the gate would drop.
    #[tokio::test]
    async fn a_registration_probe_that_panics_reports_box_failed() {
        let tmp = tempfile::tempdir().expect("temp box");
        let backend = Backend::memory(never_probed().await);
        let mut runtime = AgentRuntime::new(DriverFactory::production())
            .with_registration_probe()
            .with_probe_env(fake_env(tmp.path()), Arc::new(PanickingHardware));
        let (tx, mut rx) = mpsc::unbounded_channel();

        swap(&mut runtime, &backend, &tx).await;

        let report = the_report(&sent(&mut rx));
        assert!(
            report
                .box_failed
                .as_deref()
                .is_some_and(|message| message.contains("the hardware read blew up")),
            "{report:?}"
        );
        assert!(
            report.status_line().contains("the hardware read blew up"),
            "the status line says so: {}",
            report.status_line()
        );
    }

    // -----------------------------------------------------------------------------------------
    // MOD-66: SetToolPaths (blueprint §4.5; D7–D9, B1, B4–B6, H-10, H-11)
    //
    // Every case runs over `fake_env`, whose `PATH` holds neither tool the fixture row declares,
    // over a registry whose other rows resolve nowhere (H-7), and the fixture row says
    // `handshake: false`: a row that resolves is `ready` and **nothing is spawned**.
    // -----------------------------------------------------------------------------------------

    /// The fixture row's name.
    const PATHS_ROW: &str = "paths-fixture";

    /// An `acp` row declaring two tools that resolve nowhere except through a manual path.
    fn paths_row(id: AgentId) -> Agent {
        Agent {
            name: PATHS_ROW.to_owned(),
            transport: Transport::Acp,
            launch: json!({
                "command": "${first}",
                "args": ["${second}"],
                "env": {},
                "discovery": {
                    "tools": {
                        "first": { "kind": "path", "names": ["htui-no-such-binary-66a"] },
                        "second": { "kind": "path", "names": ["htui-no-such-binary-66b"] }
                    },
                    "handshake": false
                }
            }),
            settings: json!({}),
            ..fake_row(id)
        }
    }

    /// The unresolvable demo registry plus [`paths_row`], and a runtime over the fake env.
    async fn paths_fixture(tmp: &std::path::Path) -> (MemStore, Backend, AgentRuntime, AgentId) {
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        store
            .upsert_agent(&paths_row(agent_id), None)
            .await
            .expect("the row lands");
        let backend = Backend::memory(store.clone());
        let runtime =
            AgentRuntime::new(DriverFactory::new()).with_probe_env(fake_env(tmp), fake_hardware());
        (store, backend, runtime, agent_id)
    }

    /// Writes a plain file at `path`: a manual path only has to be a file, and nothing runs it.
    fn plain_file(path: &std::path::Path) -> String {
        std::fs::write(path, "not a real tool\n").expect("write the file");
        path.to_string_lossy().into_owned()
    }

    /// `pairs` as the request's map.
    fn tool_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(tool, path)| ((*tool).to_owned(), (*path).to_owned()))
            .collect()
    }

    /// `SetToolPaths { agent_id, pairs }` at `seq`.
    async fn set_paths(
        runtime: &mut AgentRuntime,
        backend: &Backend,
        tx: &mpsc::UnboundedSender<ReplyEnvelope>,
        seq: Seq,
        agent_id: AgentId,
        pairs: &[(&str, &str)],
    ) -> Served {
        runtime
            .serve(
                backend,
                tx,
                &envelope(
                    seq,
                    StoreRequest::SetToolPaths {
                        agent_id,
                        paths: tool_map(pairs),
                    },
                ),
            )
            .await
    }

    /// The message of the `Failed { "set_tool_paths" }` a refusal answers on the arm.
    fn refused(served: Served) -> String {
        match served {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, SET_TOOL_PATHS);
                message
            }
            other => panic!("a refused SetToolPaths answers Failed on the arm: {other:?}"),
        }
    }

    /// Waits for every background task, then the one reply they sent.
    async fn one_reply(
        runtime: &mut AgentRuntime,
        rx: &mut mpsc::UnboundedReceiver<ReplyEnvelope>,
    ) -> ReplyEnvelope {
        runtime.finish_background(Duration::from_secs(5)).await;
        let mut replies = Vec::new();
        while let Ok(reply) = rx.try_recv() {
            replies.push(reply);
        }
        assert_eq!(replies.len(), 1, "exactly one reply: {replies:?}");
        replies.remove(0)
    }

    /// The `ToolPaths` outcome of `reply`.
    fn tool_paths_outcome(reply: ReplyEnvelope) -> AgentWrite {
        match reply.reply {
            StoreReply::AgentWritten { outcome, .. } => outcome,
            other => panic!("a SetToolPaths that ran answers AgentWritten: {other:?}"),
        }
    }

    /// `agent_id`'s registry row and this box's `agent_box` for it, as stored now.
    async fn summary_of(store: &MemStore, agent_id: AgentId) -> htui_core::model::AgentSummary {
        store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == agent_id)
            .expect("the row is listed")
    }

    /// The stored snapshot of `agent_id` on this box.
    async fn stored_snapshot(store: &MemStore, agent_id: AgentId) -> (AgentBox, ProbeSnapshot) {
        let on_box = summary_of(store, agent_id)
            .await
            .on_box
            .expect("an agent_box row was written");
        let snapshot = ProbeSnapshot::from_row(&on_box).expect("the stored probe parses");
        (on_box, snapshot)
    }

    /// A stored `source: manual` row over `manual`, probed long ago (`demo_at(0, 0)`).
    async fn stage_manual_row(
        store: &MemStore,
        agent_id: AgentId,
        manual: BTreeMap<String, String>,
    ) {
        let snapshot = ProbeSnapshot {
            transport: Transport::Acp,
            resolved: None,
            tools: BTreeMap::new(),
            handshake: None,
            credential: None,
            status: ProbeStatus::Ready,
            stderr_tail: None,
            source: htui_agent::probe::ProbeSource::Manual,
            manual,
        };
        let row = agent_box_row(
            &paths_row(agent_id),
            ids::BOX,
            &snapshot,
            htui_core::fixtures::demo_at(0, 0),
        );
        store
            .upsert_agent_box(&row)
            .await
            .expect("the stored row lands");
    }

    /// Takes every staged background task out and aborts it.
    fn abort_background(runtime: &mut AgentRuntime) {
        for entry in runtime.background.drain(..) {
            entry.into_task().abort();
        }
    }

    /// D9's first check: with no writable registry the request is refused before any spawn.
    #[tokio::test]
    async fn set_tool_paths_offline_refuses_before_spawning_anything() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = htui_store::CacheStore::open(root.path(), "paths-test", 1)
            .await
            .expect("mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        let mut runtime = AgentRuntime::new(DriverFactory::new())
            .with_probe_env(fake_env(root.path()), fake_hardware());
        let (tx, _rx) = mpsc::unbounded_channel();

        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, AgentId::new(), &[]).await);
        assert!(
            message.contains(htui_store::REGISTRY_ON_SERVER_ONLY),
            "the offline backend's own sentence: {message}"
        );
        assert_eq!(runtime.background_len(), 0, "nothing was spawned");
        cache.close().await;
    }

    /// D9: a box that is not registered has no `agent_box` primary key to write against.
    #[tokio::test]
    async fn set_tool_paths_on_an_unregistered_box_is_refused() {
        let tmp = tempfile::tempdir().expect("temp box");
        let backend = Backend::memory(MemStore::new());
        let mut runtime = AgentRuntime::new(DriverFactory::new())
            .with_probe_env(fake_env(tmp.path()), fake_hardware());
        let (tx, _rx) = mpsc::unbounded_channel();

        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, AgentId::new(), &[]).await);
        assert!(message.contains("not registered"), "{message}");
        assert_eq!(runtime.background_len(), 0);
    }

    /// D9: the row must exist and be enabled in the registry.
    #[tokio::test]
    async fn set_tool_paths_refuses_an_unknown_agent_and_a_disabled_one() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let (tx, _rx) = mpsc::unbounded_channel();

        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, AgentId::new(), &[]).await);
        assert!(message.contains("not found"), "{message}");
        assert_eq!(runtime.background_len(), 0);

        // An update carries the stored `updated_at` as its token (MOD-23's CAS).
        let mut disabled = summary_of(&store, agent_id).await.agent;
        let token = disabled.updated_at;
        disabled.enabled = false;
        assert!(
            matches!(
                store.upsert_agent(&disabled, Some(token)).await,
                Ok(htui_core::store::CasOutcome::Applied(_))
            ),
            "the row is switched off in the registry"
        );
        let message = refused(set_paths(&mut runtime, &backend, &tx, 2, agent_id, &[]).await);
        assert_eq!(
            message,
            "`paths-fixture` is disabled in the registry; nothing to set a path for"
        );
        assert_eq!(runtime.background_len(), 0);
    }

    /// D9: every key is a tool the row's `discovery.tools` declares.
    #[tokio::test]
    async fn set_tool_paths_refuses_a_tool_the_row_does_not_declare() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (_store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let file = plain_file(&tmp.path().join("third-tool"));
        let (tx, _rx) = mpsc::unbounded_channel();

        let message = refused(
            set_paths(
                &mut runtime,
                &backend,
                &tx,
                1,
                agent_id,
                &[("third", &file)],
            )
            .await,
        );
        assert_eq!(message, "`third` is not a tool `paths-fixture` declares");
        assert_eq!(runtime.background_len(), 0);
    }

    /// D10 on the worker's side (review N4): a row whose launch declares no tool, with no
    /// `discovery` or an empty `tools`, is refused with the form's own sentence, even for an
    /// empty map, which would otherwise be written.
    #[tokio::test]
    async fn set_tool_paths_refuses_a_literal_launch_with_the_forms_sentence() {
        let tmp = tempfile::tempdir().expect("temp box");
        let store = unresolvable_registry().await;
        let no_discovery = AgentId::new();
        let no_tools = AgentId::new();
        for (id, name, launch) in [
            (
                no_discovery,
                "no-discovery",
                json!({ "command": "/bin/true", "args": [], "env": {} }),
            ),
            (
                no_tools,
                "no-tools",
                json!({
                    "command": "/bin/true",
                    "args": [],
                    "env": {},
                    "discovery": { "tools": {}, "handshake": false }
                }),
            ),
        ] {
            store
                .upsert_agent(
                    &Agent {
                        name: name.to_owned(),
                        launch,
                        ..paths_row(id)
                    },
                    None,
                )
                .await
                .expect("the row lands");
        }
        let backend = Backend::memory(store.clone());
        let mut runtime = AgentRuntime::new(DriverFactory::new())
            .with_probe_env(fake_env(tmp.path()), fake_hardware());
        let (tx, _rx) = mpsc::unbounded_channel();

        for (seq, agent_id) in [(1, no_discovery), (2, no_tools)] {
            let message = refused(set_paths(&mut runtime, &backend, &tx, seq, agent_id, &[]).await);
            assert_eq!(message, "this row's launch is literal; e edits its command");
            assert_eq!(runtime.background_len(), 0, "nothing was spawned");
            assert!(summary_of(&store, agent_id).await.on_box.is_none());
        }
    }

    /// D9, `R-SEC-2` (review L3): a `launch` that does not parse is refused by name, and the
    /// sentence carries no serde text, which would quote the document, `env` included.
    #[tokio::test]
    async fn set_tool_paths_refuses_an_unparsable_launch_without_quoting_it() {
        const SENTINEL: &str = "SECRET-SENTINEL";
        let tmp = tempfile::tempdir().expect("temp box");
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        // `args` as a string: serde's own text quotes the offending value.
        let launch = json!({
            "command": "${first}",
            "args": SENTINEL,
            "env": { "K": SENTINEL },
            "discovery": { "tools": {}, "handshake": false }
        });
        let serde_text = serde_json::from_value::<AgentLaunch>(launch.clone())
            .expect_err("the launch is malformed")
            .to_string();
        assert!(
            serde_text.contains(SENTINEL),
            "the case is only worth having if serde would leak: {serde_text}"
        );
        store
            .upsert_agent(
                &Agent {
                    launch,
                    ..paths_row(agent_id)
                },
                None,
            )
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let mut runtime = AgentRuntime::new(DriverFactory::new())
            .with_probe_env(fake_env(tmp.path()), fake_hardware());
        let (tx, _rx) = mpsc::unbounded_channel();

        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, agent_id, &[]).await);
        assert_eq!(
            message,
            "`paths-fixture`'s launch does not parse; nothing declares a tool"
        );
        assert!(!message.contains(SENTINEL), "{message}");
        assert_eq!(runtime.background_len(), 0);
    }

    /// D9: every value is absolute (`parse_tool_path`) and a file on this box (`is_file`), and
    /// the refusal names the tool. The shape is checked on the arm; `is_file` is I/O and runs in
    /// the task, so a hung mount cannot stall the store loop (review L1). Its refusal is the
    /// task's one answer, nothing is written, and the row's claim is free after.
    #[tokio::test]
    async fn set_tool_paths_refuses_a_relative_path_a_directory_and_a_missing_file_naming_the_tool()
    {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let (tx, mut rx) = mpsc::unbounded_channel();

        let message = refused(
            set_paths(
                &mut runtime,
                &backend,
                &tx,
                1,
                agent_id,
                &[("first", "bin/x")],
            )
            .await,
        );
        assert_eq!(message, "`first`: the path must be absolute");
        assert_eq!(runtime.background_len(), 0, "nothing was spawned");

        let dir = tmp.path().join("a-directory");
        std::fs::create_dir_all(&dir).expect("the directory");
        let dir = dir.to_string_lossy().into_owned();
        let absent = tmp.path().join("absent").to_string_lossy().into_owned();
        for (seq, path) in [(2, &dir), (3, &absent)] {
            let served = set_paths(
                &mut runtime,
                &backend,
                &tx,
                seq,
                agent_id,
                &[("first", path)],
            )
            .await;
            assert!(
                matches!(served, Served::Deferred),
                "`is_file` is the task's check: {served:?}"
            );
            let reply = one_reply(&mut runtime, &mut rx).await;
            assert_eq!(reply.seq, seq);
            match reply.reply {
                StoreReply::Failed { request, message } => {
                    assert_eq!(request, SET_TOOL_PATHS);
                    assert_eq!(
                        message,
                        format!("`first`: `{path}` is not a file on this box")
                    );
                }
                other => panic!("a path that is not a file answers Failed: {other:?}"),
            }
            assert!(
                runtime.reprobe_claims.claim((agent_id, ids::BOX)).is_some(),
                "the refusal released the row's claim"
            );
        }
        assert!(
            summary_of(&store, agent_id).await.on_box.is_none(),
            "a refused map writes nothing"
        );
    }

    /// D8: a writing background task, a box probe and another tool-paths write each hold the box.
    #[tokio::test]
    async fn set_tool_paths_is_refused_while_a_writer_holds_the_box() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (_store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let (tx, _rx) = mpsc::unbounded_channel();

        runtime
            .background
            .push(Background::writing(tokio::spawn(std::future::pending())));
        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, agent_id, &[]).await);
        assert!(
            message.ends_with(BOX_WRITE_RUNNING),
            "verb-neutral, not \"install\" (review L4): {message}"
        );
        assert_eq!(runtime.background_len(), 1, "the refusal spawned nothing");
        abort_background(&mut runtime);

        runtime.box_probe = Some(tokio::spawn(std::future::pending()));
        let message = refused(set_paths(&mut runtime, &backend, &tx, 2, agent_id, &[]).await);
        assert!(message.contains(BOX_PROBE_RUNNING), "{message}");
        assert_eq!(runtime.background_len(), 0);
        runtime.box_probe.take().expect("still staged").abort();

        let other = AgentId::new();
        runtime.background.push(Background::writing_tool_paths(
            tokio::spawn(std::future::pending()),
            other,
        ));
        let message = refused(set_paths(&mut runtime, &backend, &tx, 3, agent_id, &[]).await);
        assert!(
            message.contains(&tool_paths_running(other)),
            "B5: its own sentence, not the install one: {message}"
        );
        assert_eq!(runtime.background_len(), 1);
        abort_background(&mut runtime);
    }

    /// D8: an install writes this row at its end, so a tool-paths write may not run beside it.
    #[tokio::test]
    async fn set_tool_paths_while_an_install_runs_is_refused() {
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&install_row(agent_id, "demo", true), None)
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

        let planning = runtime
            .serve(
                &backend,
                &tx,
                &envelope(1, StoreRequest::InstallPlan { agent_id }),
            )
            .await;
        assert!(matches!(planning, Served::Deferred), "{planning:?}");
        let message = refused(set_paths(&mut runtime, &backend, &tx, 2, agent_id, &[]).await);
        assert!(
            message.contains("an install is already running"),
            "the refusal names what holds the box: {message}"
        );
        assert_eq!(runtime.background_len(), 0, "and it spawned nothing");
        runtime.shutdown(Duration::ZERO).await;
    }

    /// D8: `claim_is_free` cannot see a D60 re-probe (it runs inside a chat task), so the row's
    /// re-probe claim is taken too, and a held one refuses.
    #[tokio::test]
    async fn set_tool_paths_is_refused_while_a_re_probe_holds_the_row() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (_store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let (tx, _rx) = mpsc::unbounded_channel();

        let _held = runtime
            .reprobe_claims
            .claim((agent_id, ids::BOX))
            .expect("the row is free");
        let message = refused(set_paths(&mut runtime, &backend, &tx, 1, agent_id, &[]).await);
        assert!(
            message.contains(&format!("a re-probe is running for agent {agent_id}")),
            "{message}"
        );
        assert_eq!(runtime.background_len(), 0);
    }

    /// D8, H-11: `ProbeAgents` writes every row, so it refuses while a tool-paths write runs.
    #[tokio::test]
    async fn probe_agents_is_refused_while_a_tool_paths_write_runs() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (_store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let (tx, _rx) = mpsc::unbounded_channel();

        runtime.background.push(Background::writing_tool_paths(
            tokio::spawn(std::future::pending()),
            agent_id,
        ));
        match runtime
            .serve(&backend, &tx, &envelope(1, StoreRequest::ProbeAgents))
            .await
        {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(message.contains(&tool_paths_running(agent_id)), "{message}");
            }
            other => panic!("a probe may not run beside a tool-paths write: {other:?}"),
        }
        assert_eq!(runtime.background_len(), 1, "nothing was added");
        abort_background(&mut runtime);
    }

    /// A row like [`paths_row`] whose one tool, `hold`, is the script `/bin/sh` runs, with
    /// `handshake: true`: a `SetToolPaths` over it is in flight until the script exits.
    #[cfg(unix)]
    fn holding_row(id: AgentId) -> Agent {
        Agent {
            launch: json!({
                "command": "/bin/sh",
                "args": ["${hold}"],
                "env": {},
                "discovery": {
                    "tools": {
                        "hold": { "kind": "path", "names": ["htui-no-such-binary-66h"] }
                    },
                    "handshake": true
                }
            }),
            ..paths_row(id)
        }
    }

    /// A `SetToolPaths` the real handler accepted over [`holding_row`], still in flight.
    #[cfg(unix)]
    struct InFlight {
        backend: Backend,
        runtime: AgentRuntime,
        agent_id: AgentId,
        tx: mpsc::UnboundedSender<ReplyEnvelope>,
        rx: mpsc::UnboundedReceiver<ReplyEnvelope>,
        /// The manual path the write was sent with.
        script: String,
        /// The file whose existence lets the script exit.
        release: std::path::PathBuf,
    }

    /// Sends `SetToolPaths` (seq 1) over [`holding_row`], whose script marks `started`, never
    /// answers `initialize`, and exits once `release` exists. Returns once the script runs, so
    /// the task is inside its handshake.
    ///
    /// The script is read by `/bin/sh` rather than executed (the `ETXTBSY` rule of
    /// `login_row`'s helper), and it names `/bin/sleep` because `fake_env`'s `PATH` holds nothing.
    #[cfg(unix)]
    async fn in_flight_tool_paths(tmp: &std::path::Path) -> InFlight {
        let store = unresolvable_registry().await;
        let agent_id = AgentId::new();
        store
            .upsert_agent(&holding_row(agent_id), None)
            .await
            .expect("the row lands");
        let backend = Backend::memory(store);
        let mut runtime =
            AgentRuntime::new(DriverFactory::new()).with_probe_env(fake_env(tmp), fake_hardware());
        let started = tmp.join("started");
        let release = tmp.join("release");
        let script = tmp.join("hold.sh");
        std::fs::write(
            &script,
            format!(
                ": > '{}'\nwhile [ ! -e '{}' ]; do /bin/sleep 0.05; done\n",
                started.display(),
                release.display()
            ),
        )
        .expect("write the script");
        let script = script.to_string_lossy().into_owned();
        let (tx, rx) = mpsc::unbounded_channel();

        let served = set_paths(
            &mut runtime,
            &backend,
            &tx,
            1,
            agent_id,
            &[("hold", &script)],
        )
        .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !started.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the task never reached its handshake"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        InFlight {
            backend,
            runtime,
            agent_id,
            tx,
            rx,
            script,
            release,
        }
    }

    #[cfg(unix)]
    impl InFlight {
        /// Lets the script exit and returns the write's one answer.
        async fn finish(&mut self) -> ReplyEnvelope {
            std::fs::write(&self.release, "").expect("release the script");
            one_reply(&mut self.runtime, &mut self.rx).await
        }
    }

    /// H-10 (review M1): the row's re-probe claim is moved into the task, so a staleness re-probe
    /// of the row is refused for as long as the write is in flight, and is free once it answered.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_tool_paths_write_holds_the_reprobe_claim_while_in_flight() {
        let tmp = tempfile::tempdir().expect("temp box");
        let mut flight = in_flight_tool_paths(tmp.path()).await;
        let agent_id = flight.agent_id;

        assert!(
            flight
                .runtime
                .reprobe_claims
                .claim((agent_id, ids::BOX))
                .is_none(),
            "a staleness re-probe of this row waits for the write"
        );
        assert!(
            flight
                .runtime
                .reprobe_claims
                .claim((AgentId::new(), ids::BOX))
                .is_some(),
            "another row on this box is not excluded"
        );

        let reply = flight.finish().await;
        assert!(
            matches!(
                tool_paths_outcome(reply),
                AgentWrite::ToolPaths {
                    status: ProbeStatus::Failed,
                    ..
                }
            ),
            "a handshake the script never answered"
        );
        assert!(
            flight
                .runtime
                .reprobe_claims
                .claim((agent_id, ids::BOX))
                .is_some(),
            "and the claim goes with the task"
        );
    }

    /// H-11, B5 (review M2): the real handler tags its task `Writes::ToolPaths`, so a
    /// `ProbeAgents` and a second `SetToolPaths` beside it are refused with B5's sentence.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_running_tool_paths_write_refuses_a_probe_and_a_second_write_by_name() {
        let tmp = tempfile::tempdir().expect("temp box");
        let mut flight = in_flight_tool_paths(tmp.path()).await;
        let agent_id = flight.agent_id;

        match flight
            .runtime
            .serve(
                &flight.backend,
                &flight.tx,
                &envelope(2, StoreRequest::ProbeAgents),
            )
            .await
        {
            Served::Reply(StoreReply::Failed { request, message }) => {
                assert_eq!(request, "probe_agents");
                assert!(
                    message.ends_with(&tool_paths_running(agent_id)),
                    "{message}"
                );
            }
            other => panic!("a probe may not run beside a tool-paths write: {other:?}"),
        }
        let script = flight.script.clone();
        let message = refused(
            set_paths(
                &mut flight.runtime,
                &flight.backend,
                &flight.tx,
                3,
                agent_id,
                &[("hold", &script)],
            )
            .await,
        );
        assert!(
            message.ends_with(&tool_paths_running(agent_id)),
            "B5: its own sentence: {message}"
        );
        assert_eq!(flight.runtime.background_len(), 1, "nothing was added");

        let reply = flight.finish().await;
        assert_eq!(reply.seq, 1, "the one answer is the first write's");
    }

    /// D7, D9: a valid map is probed over, written as a `manual` snapshot carrying the map, and
    /// answered once at the request's address. The row's re-probe claim is released after (H-10).
    #[tokio::test]
    async fn a_valid_map_writes_a_manual_snapshot_and_answers_tool_paths() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let first = plain_file(&tmp.path().join("first-tool"));
        let second = plain_file(&tmp.path().join("second-tool"));
        let map = tool_map(&[("first", &first), ("second", &second)]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let request = RequestEnvelope {
            seq: 7,
            origin: Origin::Tab(crate::ui::tabs::TabId("settings")),
            request: StoreRequest::SetToolPaths {
                agent_id,
                paths: map.clone(),
            },
        };

        let served = runtime.serve(&backend, &tx, &request).await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert_eq!(runtime.writing_background_len(), 1, "a writing task (B4)");

        let reply = one_reply(&mut runtime, &mut rx).await;
        assert_eq!(reply.seq, 7, "the request's own address");
        assert_eq!(reply.origin, request.origin);
        match reply.reply {
            StoreReply::AgentWritten { agents, outcome } => {
                assert_eq!(
                    outcome,
                    AgentWrite::ToolPaths {
                        id: agent_id,
                        name: PATHS_ROW.to_owned(),
                        status: ProbeStatus::Ready,
                    }
                );
                assert!(
                    agents
                        .iter()
                        .any(|summary| summary.agent.id == agent_id && summary.on_box.is_some()),
                    "the reply carries the registry re-read"
                );
            }
            other => panic!("a SetToolPaths that ran answers AgentWritten: {other:?}"),
        }

        let (on_box, snapshot) = stored_snapshot(&store, agent_id).await;
        assert!(on_box.enabled, "a ready row is enabled");
        assert!(on_box.probed_at.is_some());
        assert_eq!(snapshot.source, htui_agent::probe::ProbeSource::Manual);
        assert_eq!(snapshot.manual, map);
        let resolved = snapshot.resolved.expect("the launch resolved");
        assert_eq!(resolved.command, first);
        assert_eq!(resolved.args, vec![second]);
        assert!(
            runtime.reprobe_claims.claim((agent_id, ids::BOX)).is_some(),
            "the task released the row's re-probe claim"
        );
    }

    /// Plan amendment 7: a stored manual row plus a report that stays incomplete is **written**
    /// (`probe_agent` would have kept it, D51; B1 bypasses that).
    #[tokio::test]
    async fn a_stored_manual_row_with_an_incomplete_report_is_still_written() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let first = plain_file(&tmp.path().join("first-tool"));
        let second = plain_file(&tmp.path().join("second-tool"));
        stage_manual_row(
            &store,
            agent_id,
            tool_map(&[("first", &first), ("second", &second)]),
        )
        .await;
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = set_paths(
            &mut runtime,
            &backend,
            &tx,
            1,
            agent_id,
            &[("first", &first)],
        )
        .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert_eq!(
            tool_paths_outcome(one_reply(&mut runtime, &mut rx).await),
            AgentWrite::ToolPaths {
                id: agent_id,
                name: PATHS_ROW.to_owned(),
                status: ProbeStatus::Missing,
            }
        );

        let (on_box, snapshot) = stored_snapshot(&store, agent_id).await;
        assert!(
            on_box
                .probed_at
                .is_some_and(|at| at > htui_core::fixtures::demo_at(0, 0)),
            "the row was written, not kept: {:?}",
            on_box.probed_at
        );
        assert_eq!(snapshot.status, ProbeStatus::Missing);
        assert_eq!(snapshot.manual, tool_map(&[("first", &first)]));
        assert_eq!(
            snapshot.source,
            htui_agent::probe::ProbeSource::Manual,
            "`first` was decided by a manual path (D4)"
        );
    }

    /// D9 / B1: a row with no `agent_box` row yet is written, with no synthesized `existing`.
    #[tokio::test]
    async fn a_row_with_no_agent_box_row_is_written() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        assert!(
            summary_of(&store, agent_id).await.on_box.is_none(),
            "the fixture starts with no agent_box row"
        );
        let first = plain_file(&tmp.path().join("first-tool"));
        let second = plain_file(&tmp.path().join("second-tool"));
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = set_paths(
            &mut runtime,
            &backend,
            &tx,
            1,
            agent_id,
            &[("first", &first), ("second", &second)],
        )
        .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert_eq!(
            tool_paths_outcome(one_reply(&mut runtime, &mut rx).await),
            AgentWrite::ToolPaths {
                id: agent_id,
                name: PATHS_ROW.to_owned(),
                status: ProbeStatus::Ready,
            }
        );
        let (_on_box, snapshot) = stored_snapshot(&store, agent_id).await;
        assert_eq!(snapshot.status, ProbeStatus::Ready);
    }

    /// D9: an empty map clears every manual path, and the row is written even when `missing`.
    #[tokio::test]
    async fn an_empty_map_clears_to_a_probe_snapshot_even_when_missing() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let first = plain_file(&tmp.path().join("first-tool"));
        stage_manual_row(&store, agent_id, tool_map(&[("first", &first)])).await;
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = set_paths(&mut runtime, &backend, &tx, 1, agent_id, &[]).await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert_eq!(
            tool_paths_outcome(one_reply(&mut runtime, &mut rx).await),
            AgentWrite::ToolPaths {
                id: agent_id,
                name: PATHS_ROW.to_owned(),
                status: ProbeStatus::Missing,
            }
        );

        let (on_box, snapshot) = stored_snapshot(&store, agent_id).await;
        assert_eq!(snapshot.source, htui_agent::probe::ProbeSource::Probe);
        assert!(
            on_box
                .probe
                .as_ref()
                .expect("a probe document")
                .get("manual")
                .is_none(),
            "an empty map adds no key"
        );
        assert!(
            on_box
                .probed_at
                .is_some_and(|at| at > htui_core::fixtures::demo_at(0, 0)),
            "written, not kept"
        );
    }

    /// MOD-23 D243: a row switched off on this box stays off whatever the probe says.
    #[tokio::test]
    async fn a_switched_off_row_stays_off_under_set_tool_paths() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        store
            .set_agent_box_enabled(agent_id, ids::BOX, false)
            .await
            .expect("the switch lands");
        let first = plain_file(&tmp.path().join("first-tool"));
        let second = plain_file(&tmp.path().join("second-tool"));
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = set_paths(
            &mut runtime,
            &backend,
            &tx,
            1,
            agent_id,
            &[("first", &first), ("second", &second)],
        )
        .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        assert_eq!(
            tool_paths_outcome(one_reply(&mut runtime, &mut rx).await),
            AgentWrite::ToolPaths {
                id: agent_id,
                name: PATHS_ROW.to_owned(),
                status: ProbeStatus::Ready,
            }
        );
        let summary = summary_of(&store, agent_id).await;
        assert!(summary.user_off, "the switch is untouched");
        assert!(
            !summary.on_box.expect("the written row").enabled,
            "the store kept the veto"
        );
    }

    /// D4: every probe path carries the map, so a `ProbeAgents` after the write keeps it.
    #[tokio::test]
    async fn a_probe_after_set_tool_paths_keeps_the_map() {
        let tmp = tempfile::tempdir().expect("temp box");
        let (store, backend, mut runtime, agent_id) = paths_fixture(tmp.path()).await;
        let first = plain_file(&tmp.path().join("first-tool"));
        let second = plain_file(&tmp.path().join("second-tool"));
        let map = tool_map(&[("first", &first), ("second", &second)]);
        let (tx, mut rx) = mpsc::unbounded_channel();

        let served = set_paths(
            &mut runtime,
            &backend,
            &tx,
            1,
            agent_id,
            &[("first", &first), ("second", &second)],
        )
        .await;
        assert!(matches!(served, Served::Deferred), "{served:?}");
        let _ = one_reply(&mut runtime, &mut rx).await;

        let probing = runtime
            .serve(&backend, &tx, &envelope(2, StoreRequest::ProbeAgents))
            .await;
        assert!(matches!(probing, Served::Deferred), "{probing:?}");
        runtime.finish_background(Duration::from_secs(10)).await;

        let (on_box, snapshot) = stored_snapshot(&store, agent_id).await;
        assert_eq!(snapshot.source, htui_agent::probe::ProbeSource::Manual);
        assert_eq!(snapshot.manual, map);
        assert_eq!(snapshot.status, ProbeStatus::Ready);
        assert!(on_box.enabled);
    }
}
