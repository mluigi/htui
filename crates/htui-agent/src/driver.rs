//! The `AgentDriver` / `AgentSession` seam of `docs/ANA-4.md` §4.1 (plan MOD-2 D2).
//!
//! Two traits, not one: a `Send + Sync` **driver** built once per `agent` row that holds no child
//! process, and a `Send` **session** that owns nothing but channel endpoints. Every method returns
//! a hand-written `Pin<Box<dyn Future<Output = …> + Send + '_>>` rather than being an `async fn`,
//! because an `async fn` in a trait is not dyn-compatible on this toolchain (`E0038`) and MOD-4
//! holds a `Box<dyn AgentDriver>`. `async_trait` would generate exactly this desugaring and buy
//! nothing but a dependency, so it is not used.
//!
//! Events are **pulled** through [`AgentSession::next_event`]. A pull loop preserves ordering and
//! back-pressure without a `Stream` impl, and gives the session somewhere to park a permission
//! responder across a UI round trip.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{AgentId, StepId};
use serde::{Deserialize, Serialize};

use crate::error::DriverError;
use crate::event::{DriverEnvelope, PermissionOptionKind};

/// What a hand-written `Debug` prints in place of an environment value (ANA-4 §4.1, invariant 4).
const REDACTED: &str = "[REDACTED]";

/// ANA-4 §4.1's return type, spelled once.
///
/// Every operation of [`AgentDriver`] and [`AgentSession`] returns exactly
/// `Pin<Box<dyn Future<Output = Result<T, DriverError>> + Send + 'a>>`. The alias **is** that
/// type, so an implementor may write either form; it exists so the seam reads as five operations
/// rather than as five copies of one type.
pub type DriverFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DriverError>> + Send + 'a>>;

// ---------------------------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------------------------

/// The agent-side session id, for `session/load` / `--resume` on a later step.
///
/// Opaque: the transport mints it and `htui` only stores and replays it, so it is a `String` and
/// not a UUID newtype like the `htui_core::model::ids` family.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentSessionRef(
    /// The id as the agent reported it.
    pub String,
);

impl AgentSessionRef {
    /// Wraps an agent-supplied id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for AgentSessionRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Correlates a `permission_request` with the answer `htui` sends back.
///
/// Opaque for the same reason as [`AgentSessionRef`]: over ACP it is the JSON-RPC request id, over
/// a future CLI route it is whatever the MCP prompt tool supplies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionRequestId(
    /// The id as the transport reported it.
    pub String,
);

impl PermissionRequestId {
    /// Wraps a transport-supplied request id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for PermissionRequestId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The answer to a parked permission request (ANA-4 §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAnswer {
    /// The user or a policy rule chose this `PermissionOption::id`.
    Selected(String),
    /// The session was cancelled. The spec makes answering every outstanding request a MUST, and
    /// the recorded row is `permission_answer { option_id: null, by: "policy", cancelled: true }`.
    Cancelled,
}

// ---------------------------------------------------------------------------------------------
// Permission policy (`agent.settings.permission`, §5.2)
// ---------------------------------------------------------------------------------------------

wire_enum!(
    /// `agent.settings.permission.default` (§5.2): what happens to a request no rule matched.
    #[derive(Default)]
    PermissionDefault {
        /// Park the request and ask the user (`R-TUI-6`). The default.
        #[default]
        Ask => "ask",
        /// Answer with the first allow option.
        Allow => "allow",
        /// Answer with the first reject option.
        Deny => "deny",
    }
);

/// What a permission rule matches on (§5.2). Every field is optional; an all-`None` match matches
/// every request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionMatch {
    /// Matches `tool_call.tool_kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    /// Matches the tool's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Matches the first path argument by prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Matches the first command argument by prefix (`R-MCP-4`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_prefix: Option<String>,
}

/// One entry of `agent.settings.permission.rules[]` (§5.2), evaluated in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// The predicate. Named `match` on the wire, which is a Rust keyword.
    #[serde(rename = "match")]
    pub matcher: PermissionMatch,
    /// The option kind to answer with.
    pub answer: PermissionOptionKind,
    /// Why the rule exists; rendered next to the recorded answer.
    #[serde(default)]
    pub reason: String,
}

/// One entry of `agent.settings.permission.remembered[]` (§5.2): an `_always` choice the user made
/// in `htui`, scoped to this agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedPermission {
    /// The predicate. Named `match` on the wire.
    #[serde(rename = "match")]
    pub matcher: PermissionMatch,
    /// Which `_always` kind was chosen.
    pub option_kind: PermissionOptionKind,
    /// When the entry was added.
    pub added_at: DateTime<Utc>,
    /// Who added it; `user` today, because only a human can pick an `_always` option.
    #[serde(default)]
    pub added_by: String,
}

/// `agent.settings.permission` (§5.2), the policy stage of ANA-4 §4.3's three-stage pipeline.
///
/// Every key is optional with a documented default, because the column is `JSONB NOT NULL DEFAULT
/// '{}'` and a row written by hand in the Settings tab must stay valid.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionPolicy {
    /// What to do when no rule and no remembered entry matched. Defaults to
    /// [`PermissionDefault::Ask`].
    pub default: PermissionDefault,
    /// Stage 1: rules, in evaluation order.
    pub rules: Vec<PermissionRule>,
    /// Stage 2: the `_always` choices already made.
    pub remembered: Vec<RememberedPermission>,
}

// ---------------------------------------------------------------------------------------------
// Session inputs
// ---------------------------------------------------------------------------------------------

/// Which tools a session may use (`R-MCP-3`).
///
/// A plain record with no reader in milestones 1–2: MOD-11 fills it and the transports read it
/// (plan D16).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolExposure {
    /// Tool names the agent may call. Empty means "no allow-list": everything the agent offers.
    pub allow: Vec<String>,
    /// Tool names the agent may never call. Evaluated after `allow`.
    pub deny: Vec<String>,
    /// Whether `htui`'s own `command_run` MCP tool is exposed for this step (`R-MCP-4`).
    pub command_run: bool,
}

/// One MCP server handed to the agent at `session/new` (`R-MCP-1`).
///
/// A plain record with no reader in milestones 1–2: MOD-11 fills it (plan D16). `Debug` is hand
/// written for the same reason as [`SessionSpec`]'s.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerSpec {
    /// The name the agent sees.
    pub name: String,
    /// The server's executable.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its environment. Values may be secrets, so they never reach `Debug`.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl core::fmt::Debug for McpServerSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("McpServerSpec")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &RedactedEnv(&self.env))
            .finish()
    }
}

/// Everything a session needs at start (`R-AGT-1`: prompt, cwd, environment, tool exposure,
/// model).
///
/// `Debug` is hand written and prints every environment value as `[REDACTED]`. That is the
/// mechanical enforcement of ANA-4's invariant 4, and it is why
/// `missing_debug_implementations` is satisfied without deriving.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// `run_step.agent_id`: which registry row this session speaks to.
    pub agent_id: AgentId,
    /// `run_step.id`: the step every recorded event belongs to.
    pub step_id: StepId,
    /// The working directory the agent runs in.
    pub cwd: PathBuf,
    /// Extra readable roots: ACP `additionalDirectories`, `claude --add-dir`.
    pub extra_dirs: Vec<PathBuf>,
    /// Resolved secrets only (`R-SEC-2`). Redacted by `Debug`.
    pub env: BTreeMap<String, String>,
    /// `run_step.model`; `None` means the agent's own default.
    pub model: Option<String>,
    /// Allow / deny lists plus `command_run` exposure (`R-MCP-3`).
    pub tools: ToolExposure,
    /// `htui`'s own MCP servers (`R-MCP-1`).
    pub mcp: Vec<McpServerSpec>,
    /// `agent.settings.permission` (§4.3, §5.2).
    pub permission: PermissionPolicy,
    /// `project.settings.keep_raw_events` (ANA-9 §4.3). When false a transport does not even
    /// allocate the verbatim `serde_json::Value`.
    pub retain_raw: bool,
    /// The agent-side session id from a previous step, for `session/load` or `--resume`.
    pub resume: Option<AgentSessionRef>,
}

impl core::fmt::Debug for SessionSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionSpec")
            .field("agent_id", &self.agent_id)
            .field("step_id", &self.step_id)
            .field("cwd", &self.cwd)
            .field("extra_dirs", &self.extra_dirs)
            .field("env", &RedactedEnv(&self.env))
            .field("model", &self.model)
            .field("tools", &self.tools)
            .field("mcp", &self.mcp)
            .field("permission", &self.permission)
            .field("retain_raw", &self.retain_raw)
            .field("resume", &self.resume)
            .finish()
    }
}

/// Prints an environment map with every **value** replaced by `[REDACTED]`; keys stay visible so a
/// log still says which variables were set.
///
/// `pub(crate)` so [`crate::launch`]'s `AgentLaunch` and `ResolvedLaunch` redact the same way from
/// the same code: two copies of this would be two places for the invariant to rot.
pub(crate) struct RedactedEnv<'a>(pub(crate) &'a BTreeMap<String, String>);

impl core::fmt::Debug for RedactedEnv<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, REDACTED)))
            .finish()
    }
}

// ---------------------------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------------------------

/// Capability predicates next to the operations they gate (§4.1).
///
/// The CLI transport answers `false` to three of them, and the chat tab and the orchestrator
/// branch on **that** rather than on `agent.transport`. [`Default`] is all-false, so a transport
/// that forgets a predicate advertises nothing rather than everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DriverCaps {
    /// The transport can surface `session/request_permission` and accept an answer.
    pub permission_requests: bool,
    /// The transport can produce `edit_proposal` rows with a real diff.
    pub edit_proposals: bool,
    /// The transport reports plan updates.
    pub plans: bool,
    /// The transport reports agent reasoning as well as agent text.
    pub thoughts: bool,
    /// `false` means a follow-up respawns the process instead of reusing the session.
    pub follow_up_in_session: bool,
    /// The transport can resume a previous session from an [`AgentSessionRef`].
    pub resume: bool,
    /// The transport reports usage at all.
    pub usage: bool,
}

// ---------------------------------------------------------------------------------------------
// The seam
// ---------------------------------------------------------------------------------------------

/// One per `agent` row. Cheap, cloneable, holds no child process.
///
/// `Send + Sync` because MOD-4 keeps one behind a `Box<dyn AgentDriver>` shared across tasks;
/// `Debug` because `missing_debug_implementations` is a workspace lint and a `Box<dyn
/// AgentDriver>` field must not defeat it.
pub trait AgentDriver: Send + Sync + core::fmt::Debug {
    /// `agent.name`, for logs and for the chat tab's header.
    fn name(&self) -> &str;

    /// What this transport can actually do (§4.3).
    fn caps(&self) -> DriverCaps;

    /// Starts the agent, negotiates, opens a session and sends the initial prompt.
    ///
    /// # Errors
    /// [`DriverError::Spawn`] when the child process cannot start, [`DriverError::Transport`]
    /// when the handshake fails, [`DriverError::Unresolved`] when the launch row still holds an
    /// unresolved `${tool}` placeholder.
    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>>;
}

/// One per live session. Owns nothing but channel endpoints; the transport runs in its own task.
pub trait AgentSession: Send + core::fmt::Debug {
    /// The agent-side session id, for `session/load` / `--resume` on a later step.
    fn session_ref(&self) -> Option<&AgentSessionRef>;

    /// Ordered pull. `Ok(None)` means the transport closed with no further events.
    ///
    /// Exactly one [`crate::event::DriverEvent::Done`] per turn precedes the next accepted
    /// follow-up.
    ///
    /// # Errors
    /// [`DriverError::Transport`] on a decode or protocol failure.
    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>>;

    /// Sends a follow-up into the open session, starting a new turn.
    ///
    /// # Errors
    /// [`DriverError::Closed`] once the session has ended, [`DriverError::Transport`] otherwise.
    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()>;

    /// Answers a parked permission request.
    ///
    /// # Errors
    /// [`DriverError::Closed`] once the session has ended, [`DriverError::Transport`] when the
    /// request id is unknown to the transport.
    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()>;

    /// Graceful first: `session/cancel` (ACP) or SIGINT / stdin close (CLI), every outstanding
    /// permission request answered `cancelled`, then a process-tree kill after the grace window.
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the graceful path fails and the kill is what ended the
    /// session.
    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()>;
}
