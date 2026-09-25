//! The launch recipe: `agent.launch` / `agent.settings` as Rust types, `${tool}` resolution, and
//! the supervised spawn (`docs/ANA-4.md` §5.1, §5.2, §4.6; plan MOD-2 D11).
//!
//! An `agent` row is **box-independent**: it says `${node}`, not `C:/Program Files/nodejs/node.exe`.
//! [`resolve`] turns a row plus a per-box [`ToolMap`] into a [`ResolvedLaunch`], which is what a
//! transport actually spawns. The map is an *input* here — filling it from `agent_box.probe.tools`
//! is milestone 5's probe, and nothing in this module touches `PATH` looking for agents.
//!
//! The one thing this module does consult `PATH` for is the resolved command itself, through the
//! `which` crate rather than `Command::new`: `std::process::Command` does not read `PATHEXT`, so
//! handing it a Windows `.cmd` shim yields `os error 193` (ANA-4 §4.6, "Windows shim handling, as
//! a rule").
//!
//! What a spawn hands back is a [`Spawned`], and [`ChildIo::from_spawned`] is the one place that
//! turns it into the reader/writer pair a protocol runs over. Both live here rather than in a
//! transport module because both are transport-neutral: which protocol speaks over the pair is the
//! caller's business, and a second transport must not have to import the first one to spawn.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use agent_client_protocol::AcpAgentConfig;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::warn;

use crate::driver::{PermissionPolicy, RedactedEnv};
use crate::error::{DriverError, Result};

/// How many stderr lines a [`Spawned`] keeps. Enough to explain a failed handshake, bounded so a
/// chatty agent cannot grow the buffer without limit.
const STDERR_TAIL_LINES: usize = 64;

/// How much stdout [`Spawned::read_stdout_to_end`] will read before it stops and kills the child.
///
/// The reader is for one-shot children — `--version`, `npm root -g` — whose *entire* useful output
/// is one line a regex matches. A tool that streams instead would otherwise be bounded only by the
/// probe's 15 s timeout times its throughput, which is hundreds of megabytes buffered for a value
/// that was on the first line. 64 KiB is thousands of banner lines.
pub const STDOUT_LIMIT: usize = 64 * 1024;

/// `CREATE_NO_WINDOW`: a TUI must not flash a console window when it spawns an agent
/// (ANA-4 §4.6). Named here rather than imported so the constant reads the same on every platform.
///
/// `pub(crate)` since MOD-21: the login flow's URL opener spawns a second Windows process
/// (`auth::browser`, plan D17) and has the same rule to obey, and a second literal `0x0800_0000`
/// somewhere else in the crate is a constant that can drift from this one.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Per-box tool paths, keyed by the `${name}` a launch row writes.
///
/// Milestone 5's probe fills this from `agent_box.probe.tools`; until then a caller supplies it
/// (a test, or a hand-written box profile).
pub type ToolMap = BTreeMap<String, String>;

// ---------------------------------------------------------------------------------------------
// `agent.launch` (§5.1)
// ---------------------------------------------------------------------------------------------

/// `agent.launch` (§5.1): the box-independent recipe for starting one agent.
///
/// `{command, args, env}` rather than `{argv, env}` because that is what `std::process::Command`
/// wants and what the SDK's own [`AcpAgentConfig`] holds. Secrets never live here — `env` carries
/// `${tool}` placeholders and literal configuration, and [`SessionSpec::env`] carries resolved
/// secrets (`R-SEC-2`).
///
/// [`SessionSpec::env`]: crate::driver::SessionSpec::env
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunch {
    /// The executable, possibly a `${tool}` placeholder.
    pub command: String,
    /// Arguments, each possibly containing `${tool}` placeholders.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment for the child. Values may contain placeholders; redacted by `Debug` anyway,
    /// because a hand-written row is not prevented from holding something sensitive.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// How to find the tools the placeholders name. Absent means `command` and `args` are literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Discovery>,
}

impl core::fmt::Debug for AgentLaunch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentLaunch")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &RedactedEnv(&self.env))
            .field("discovery", &self.discovery)
            .finish()
    }
}

/// `agent.launch.discovery` (§5.1): the per-agent probe recipe milestone 5 executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    /// One probe per `${name}` the row uses.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolProbe>,
    /// Whether to run the tier-2 `initialize` handshake after the cheap `--version` tier.
    #[serde(default = "default_true")]
    pub handshake: bool,
    /// Where a credential the agent's **own** auth flow leaves on this box may be found (plan D59).
    /// Names and paths only, never values; a row without it keeps milestone 5's rule (a non-empty
    /// `authMethods` is `unauthenticated`, full stop).
    ///
    /// `skip_serializing_if` as well as `default`, so a row that declares no block re-serialises
    /// without a `"credential": null` key: a registry document written before milestone 6 round
    /// trips byte for byte through this type (blueprint P-10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialProbe>,
    /// Where this row's adapter comes from when the box does not have it (plan MOD-20 D12,
    /// `R-AGT-10`). Data, never code: the installer reads the registry entry `id` names and
    /// writes where `discovery.tools[tool]`'s glob will find it. A row without it — every
    /// `NodePackage`-served adapter — cannot be installed from the app, and `Settings > i` says so.
    ///
    /// `skip_serializing_if` for the same round-trip rule as [`credential`](Self::credential).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<Install>,
}

/// `agent.launch.discovery.install` (plan MOD-20 D12): the declared source of this row's adapter.
///
/// Three coordinates and no fourth: which registry, which entry in it, and which of the row's own
/// tools the installed file has to satisfy. Nothing here is a URL or a file name, because a row
/// that spelled either would be describing one box's disk and one vendor's CDN — the two things
/// `R-AGT-5` and D5 keep out of the document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    /// Which registry the [`id`](Self::id) is looked up in.
    pub source: InstallSource,
    /// The entry id in that registry — never an agent name, never a URL.
    pub id: String,
    /// The `${tool}` placeholder the installed `cmd` fills: the key into
    /// [`Discovery::tools`] whose glob must resolve the file the installer wrote.
    ///
    /// That agreement is what the pipeline checks after it promotes, so a token typo is a refusal
    /// with the tree rolled back rather than a row that reads `missing` beside a full install.
    pub tool: String,
}

/// Whether an `agent.launch` document declares `discovery.install` (MOD-20 D12). One rule for the
/// Settings section, the install pre-flight and the box probe's report (MOD-7 D13). A document
/// that does not parse declares nothing.
#[must_use]
pub fn declares_install(launch: &serde_json::Value) -> bool {
    serde_json::from_value::<AgentLaunch>(launch.clone())
        .ok()
        .and_then(|launch| launch.discovery)
        .is_some_and(|discovery| discovery.install.is_some())
}

wire_enum!(
    /// `install.source` (plan MOD-20 D12): one value today.
    ///
    /// A closed vocabulary rather than a string, so the day a second source arrives it is a
    /// variant every `match` is forced to answer for, not a `_ =>` arm that silently declines.
    InstallSource {
        /// The ACP registry, `registry/v1/latest/registry.json`.
        AcpRegistry => "acp_registry",
    }
);

/// `agent.launch.discovery.credential` (plan D59): how to tell an *installed* agent from an
/// installed-and-logged-in one, declared by the row rather than decided by the probe.
///
/// The Antigravity ACP server advertises its four auth methods whether or not this box has logged
/// in (ANA-4 §4.5), so "a non-empty `authMethods` means unauthenticated" reads an authenticated
/// box as unusable and the orchestrator skips it. The fix has to be data: `R-AGT-5` costs a new
/// agent one registry row and at most one adapter, and a `match agent.name` in the probe would be
/// exactly the second hard-coded agent that rule exists to forbid.
///
/// `htui` never handles the credential itself (`R-SEC-2`, D63): the vendor's own login writes it,
/// the probe answers *whether* one of these candidates is there, and [`SessionSpec::env`] — not
/// this — is where a resolved secret would ever live.
///
/// [`SessionSpec::env`]: crate::driver::SessionSpec::env
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CredentialProbe {
    /// Environment variables whose presence — set and non-empty — counts, as the shell names them.
    ///
    /// An exported-but-empty variable does not count: that is how a shell profile *unsets* a key,
    /// and a probe that read it as a credential would report a box ready that cannot authenticate.
    pub env: Vec<String>,
    /// File candidates in the glob tier's expander grammar (`probe::expand`): `%VAR%` anywhere, a
    /// leading `~`, and an unset variable skips the candidate rather than failing the probe — which
    /// is what lets one row carry a Windows `%VAR%` path and a unix `~` one side by side.
    ///
    /// In order; the first that exists wins. Checked **before** [`env`](Self::env) (plan D63: on a
    /// subscription box the token file is the tier that fires, and it is the more specific fact).
    pub files: Vec<String>,
}

/// How to find one tool (§5.1). The `kind` tag is the JSON discriminant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolProbe {
    /// Look for any of `names` on `PATH`.
    Path {
        /// Candidate file names, in preference order.
        #[serde(default)]
        names: Vec<String>,
        /// How to ask the tool for its version. Absent means presence is the whole probe.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<VersionProbe>,
    },
    /// Resolve an npm package's entry point, never the shim on `PATH` (ANA-4 §4.6: the `PATH`
    /// entry is a shell script whose `.cmd` sibling `CreateProcess` refuses).
    NodePackage {
        /// The package name.
        package: String,
        /// The entry point inside the package, e.g. `dist/index.js`.
        entry: String,
        /// The version the fallback pins.
        pinned: String,
        /// What to run when the package is not installed locally.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<FallbackCommand>,
    },
    /// Glob the filesystem, optionally per platform, for a tool that is not on `PATH`.
    Glob {
        /// Platform-independent patterns.
        #[serde(default)]
        patterns: Vec<String>,
        /// Patterns and extra arguments per `<os>-<arch>` (the ACP registry's own vocabulary:
        /// `darwin-aarch64`, `linux-x86_64`, `linux-aarch64`, `windows-x86_64`,
        /// `windows-aarch64`).
        #[serde(default)]
        platform: BTreeMap<String, PlatformGlob>,
    },
}

/// The cheap version tier of a probe (§4.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionProbe {
    /// Arguments that make the tool print its version.
    #[serde(default)]
    pub args: Vec<String>,
    /// The regex whose first capture group is the version. Held as text: milestone 5 compiles it,
    /// and a row that fails to compile must still deserialise so the Settings tab can show it.
    pub pattern: String,
    /// The lowest acceptable version, when one is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
}

/// What to run when a [`ToolProbe::NodePackage`] is not installed locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackCommand {
    /// The executable, possibly a `${tool}` placeholder of its own.
    pub command: String,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

/// One platform's entry in a [`ToolProbe::Glob`] (§5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlatformGlob {
    /// Patterns to search on this platform.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Arguments **appended** to `agent.launch.args` when this platform matches. This is the
    /// mechanism that carries the ACP registry's Linux-only `--uid=` without giving `agy` a second
    /// registry row.
    #[serde(default)]
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// `agent.settings` (§5.2)
// ---------------------------------------------------------------------------------------------

/// `agent.settings` (§5.2). Every key is optional with a documented default, because the column is
/// `JSONB NOT NULL DEFAULT '{}'` and a row written by hand in the Settings tab must stay valid.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// ACP client behaviour. Present even for a `cli` agent, because the degraded path can be
    /// promoted without rewriting the row.
    pub acp: AcpSettings,
    /// The CLI transport's settings, when the row has a CLI path at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<CliSettings>,
    /// The permission policy (§4.3).
    pub permission: PermissionPolicy,
    /// Where remaining allowance is read from (`R-AGT-7`).
    pub quota: QuotaSettings,
    /// What the `usage` payload's numbers mean on the CLI path.
    pub usage: UsageSettings,
}

/// `agent.settings.acp` (§5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcpSettings {
    /// The ACP protocol version to negotiate. `1` today.
    pub protocol_version: u16,
    /// What `htui` tells the agent it can do.
    pub client_capabilities: ClientCapabilities,
    /// The `configId` used for model selection; `None` means discover it at `session/new`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_config_id: Option<String>,
    /// Which session operations the agent supports.
    pub session: SessionSettings,
}

impl Default for AcpSettings {
    fn default() -> Self {
        Self {
            protocol_version: 1,
            client_capabilities: ClientCapabilities::default(),
            model_config_id: None,
            session: SessionSettings::default(),
        }
    }
}

/// `agent.settings.acp.client_capabilities` (§5.2).
///
/// `terminal` and `elicitation` are false until MOD-11 and a UI for elicitation exist: advertising
/// a capability `htui` cannot serve turns a working session into a hung one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientCapabilities {
    /// The agent may ask `htui` to read files.
    pub fs_read: bool,
    /// The agent may ask `htui` to write files.
    pub fs_write: bool,
    /// The agent may ask `htui` for a terminal.
    pub terminal: bool,
    /// The agent may ask `htui` structured questions.
    pub elicitation: bool,
}

impl Default for ClientCapabilities {
    fn default() -> Self {
        Self {
            fs_read: true,
            fs_write: true,
            terminal: false,
            elicitation: false,
        }
    }
}

/// `agent.settings.acp.session` (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// `session/load` is supported.
    pub load: bool,
    /// `session/resume` is supported.
    pub resume: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            load: true,
            resume: true,
        }
    }
}

/// `agent.settings.cli` (§5.2): the degraded, stream-parsing path.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CliSettings {
    /// Which stream dialect the adapter parses, e.g. `claude_stream_json`.
    ///
    /// A `String` rather than a closed enum on purpose: this value is half of the adapter id the
    /// registry keys on (plan D12, `cli/<stream>`), so a row naming a dialect this build has no
    /// adapter for must still *parse* — it fails as
    /// [`DriverError::UnknownAdapter`], which names the missing adapter, rather than as a serde
    /// error that names nothing useful.
    pub stream: String,
    /// The agent CLI's own permission mode flag, passed through verbatim.
    pub permission_mode: String,
    /// Extra arguments appended to every CLI invocation.
    pub extra_args: Vec<String>,
}

/// `agent.settings.quota` (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QuotaSettings {
    /// Where remaining allowance is read from.
    pub source: QuotaSource,
}

// `QuotaSource` was a `wire_enum!` here until MOD-2 milestone 7 (plan D66, blueprint P-6). It
// moved to `htui_core::model::quota` because `normalize` — which turns a transport's raw blob into
// the `agent_box.quota` document of `docs/ANA-4.md` §7 — selects on it, and `htui-core` cannot
// name a type of this crate. The re-export keeps `htui_agent::launch::QuotaSource` resolving, so
// the driver-side spelling is the same type and no caller moved.
pub use htui_core::model::QuotaSource;

/// `agent.settings.usage` (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSettings {
    /// What the `usage` payload's numbers mean.
    pub scope: UsageScope,
}

wire_enum!(
    /// `agent.settings.usage.scope` (§5.2). Describes the CLI path only: over ACP the stable
    /// report is context occupancy plus cost, so the four token fields are null regardless (§7).
    #[derive(Default)]
    UsageScope {
        /// Per-model token counts.
        #[default]
        ModelUsage => "model_usage",
        /// Session context occupancy.
        SessionContext => "session_context",
    }
);

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------------------------

/// An [`AgentLaunch`] with every `${tool}` placeholder replaced: what a transport spawns.
///
/// Serialisable because it is, by definition, "what `AgentDriver::start` actually spawns" — which
/// is what `agent_box.probe.resolved` records (ANA-4 §4.6, plan D45) and what milestone 6 reads
/// back rather than resolving a second time.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLaunch {
    /// The executable, as a concrete path or a name `which` can find.
    pub command: String,
    /// Arguments, fully substituted.
    pub args: Vec<String>,
    /// Environment for the child, fully substituted. Redacted by `Debug`.
    pub env: BTreeMap<String, String>,
}

impl core::fmt::Debug for ResolvedLaunch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResolvedLaunch")
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &RedactedEnv(&self.env))
            .finish()
    }
}

impl ResolvedLaunch {
    /// Builds the SDK's launch configuration.
    ///
    /// Through the builder, not by deserialising the row into it: `AcpAgentConfig`'s fields are
    /// private, and the row carries `discovery` and unresolved placeholders the SDK type has no
    /// place for. ANA-4 §5.1's "deserializes into the SDK type with no adapter layer" is one
    /// sentence looser than the crate (plan X6); the *shape* is field-for-field as the ANA says.
    #[must_use]
    pub fn to_acp_config(&self) -> AcpAgentConfig {
        AcpAgentConfig::new(&self.command)
            .args(self.args.clone())
            .envs(self.env.clone())
    }
}

/// Replaces every `${name}` in `launch` from `tools`.
///
/// Substitution reaches the command, every argument and every environment **value** — an agent
/// whose token is `${claude}` is as unlaunchable as one whose binary is.
///
/// # Errors
/// [`DriverError::Unresolved`] naming the first placeholder with no entry in `tools`. A row with
/// no placeholders resolves against an empty map.
pub fn resolve(launch: &AgentLaunch, tools: &ToolMap) -> Result<ResolvedLaunch> {
    Ok(ResolvedLaunch {
        command: substitute(&launch.command, tools)?,
        args: launch
            .args
            .iter()
            .map(|arg| substitute(arg, tools))
            .collect::<Result<Vec<_>>>()?,
        env: launch
            .env
            .iter()
            .map(|(key, value)| Ok((key.clone(), substitute(value, tools)?)))
            .collect::<Result<BTreeMap<_, _>>>()?,
    })
}

/// What a spawn in `cwd` would launch, before launching it: D58's rules, in one body.
///
/// The recorded launch when the probe left one **and** its `command` is still a file on disk;
/// otherwise the row's tools are resolved now ([`crate::tools::resolve`] → [`resolve`]), which is
/// the pre-milestone-6 path and carries no platform `args`. The disk check is not belt and braces:
/// ANA-4 §4.6 records that `agy` self-updates in place, so a recorded path can name a
/// version-numbered directory that is gone, and a chat must degrade *into* resolution rather than
/// fail the request. A *file* and not merely an entry, because the same self-update can leave a
/// directory where the binary used to be, and everything that is not spawnable belongs on the same
/// side of this branch ([`crate::probe::is_file`]).
///
/// It runs on the caller's task, which is a session's own, for the same reason
/// [`crate::tools::resolve`] does: this is filesystem I/O, and `AgentRuntime`'s worker loop must not
/// do any (`R-NF-3`). The command is checked exactly as recorded, so a bare name — an `HTUI_TOOL_*`
/// override the probe took on trust — is resolved against this process's own directory and almost
/// always falls back. That costs nothing: the fallback's first tier is that same override, so both
/// paths answer with the same string (blueprint H-3).
///
/// **Here rather than in a transport** (blueprint A row 3): the ACP driver and the CLI supervisor
/// both owe D58's four rules, and two copies would be two places for the rules to rot — a second
/// transport that applied only three of them would spawn something the probe never measured. The
/// environment is the row's, exactly as it resolved; a caller with more to add — a session's
/// `spec.env`, a login's browser policy — adds it to the value it is handed.
///
/// # Errors
/// [`DriverError::Unresolved`] naming the first tool that resolves nowhere or the first placeholder
/// with no entry; [`DriverError::Transport`] when the resolution machinery itself failed.
pub(crate) async fn launch_from(
    launch: &AgentLaunch,
    recorded: Option<&ResolvedLaunch>,
    cwd: &Path,
) -> Result<ResolvedLaunch> {
    match recorded {
        Some(recorded) if crate::probe::is_file(Path::new(&recorded.command)).await => {
            Ok(recorded.clone())
        }
        Some(recorded) => {
            warn!(
                command = %recorded.command,
                "the probe's recorded command is gone or is not a file; resolving again"
            );
            resolve_now(launch, cwd).await
        }
        None => resolve_now(launch, cwd).await,
    }
}

/// `tools::resolve` then [`resolve`]: the pre-milestone-6 path, and D58's fallback.
///
/// Its own function rather than two lines inside [`launch_from`] because it is reached from three
/// conditions there — no recording, a stale recording, and a snapshot D58 refuses — and a reader
/// should be able to see that all three land on the same code.
async fn resolve_now(launch: &AgentLaunch, cwd: &Path) -> Result<ResolvedLaunch> {
    let tools = crate::tools::resolve(launch.discovery.as_ref(), cwd).await?;
    resolve(launch, &tools)
}

/// Replaces every `${name}` in one string.
///
/// Hand written rather than a templating crate for the same reason ANA-5 gives for the prompt
/// templates: the vocabulary is closed and an unknown name must be an error with the name in it,
/// which is exactly what a general-purpose engine turns into a generic parse failure.
fn substitute(input: &str, tools: &ToolMap) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            // An unterminated `${` is literal text, not a placeholder: a row is allowed to contain
            // a stray brace, and refusing to launch over one would be worse than passing it on.
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let name = &after[..end];
        let value = tools
            .get(name)
            .ok_or_else(|| DriverError::Unresolved(name.to_owned()))?;
        out.push_str(value);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Spawn
// ---------------------------------------------------------------------------------------------

/// The child's stderr as [`spawn`] keeps it: a bounded tail and, when a caller asked for one, a
/// live tap.
///
/// One struct behind one lock rather than a deque and an `Option<Sender>` behind two (plan MOD-21
/// D14, blueprint P-9). Two locks would be two orders to take them in, and the *point* of the tap
/// is that installing it and appending to the tail are one atomic step: a replay that ran outside
/// the reader's lock would either miss a line written between the copy and the install, or deliver
/// it twice.
#[derive(Debug, Default)]
struct StderrTail {
    /// The last [`STDERR_TAIL_LINES`] lines, oldest first.
    lines: VecDeque<String>,
    /// Where each line also goes, while a caller holds a tap.
    tap: Option<mpsc::UnboundedSender<String>>,
    /// Whether the reader task has seen end-of-stream. A tap installed after that has a tail to
    /// replay but nothing left to stream, so it is handed the replay and then closed rather than
    /// left waiting on a sender no task still holds.
    ended: bool,
}

/// What [`Spawned::signal`] may ask a child, short of killing it.
///
/// Two values, not an `i32`: these are the only two a cancel sequence has any use for, and a
/// number would put "which signals may `htui` send a process it supervises" in the caller's hands
/// instead of here. Both numbers are fixed by POSIX and identical on every unix `htui` builds for,
/// which is why naming them here costs no platform dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSignal {
    /// `SIGINT` (2): what a terminal sends on Ctrl-C. The request a cancel makes first, because it
    /// is the one a CLI agent is written to expect (`docs/ANA-4.md` §4.4, plan D81).
    Interrupt,
    /// `SIGTERM` (15): the conventional "stop now, tidily". Sent when an interrupt was ignored and
    /// before a kill takes the choice away.
    Terminate,
}

impl StopSignal {
    /// The signal number, as `killpg` takes it.
    #[must_use]
    pub const fn number(self) -> i32 {
        match self {
            Self::Interrupt => 2,
            Self::Terminate => 15,
        }
    }
}

impl core::fmt::Display for StopSignal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        })
    }
}

/// A running agent process and the handles a transport talks to it through.
#[derive(Debug)]
pub struct Spawned {
    child: Box<dyn process_wrap::tokio::ChildWrapper>,
    stdin: Option<Compat<ChildStdin>>,
    stdout: Option<Compat<ChildStdout>>,
    stderr_tail: Arc<Mutex<StderrTail>>,
    /// Whether the child was assigned to a Windows job object.
    ///
    /// Always `false` on unix, where the child is a process-group leader instead. On Windows a
    /// `false` here means the assignment was **refused** and the process tree is supervised only
    /// as far as the direct child — the degraded path, logged at `warn`.
    pub job_object: bool,
}

impl Spawned {
    /// Takes the child's stdin, for the transport to write frames into.
    pub fn take_stdin(&mut self) -> Option<Compat<ChildStdin>> {
        self.stdin.take()
    }

    /// Takes the child's stdout, for the transport to read frames from.
    pub fn take_stdout(&mut self) -> Option<Compat<ChildStdout>> {
        self.stdout.take()
    }

    /// The child's operating-system process id, while it is running.
    ///
    /// The one identifier a caller can check a kill against that nothing else can accidentally
    /// answer to: a `pgrep` pattern matches any command line that merely *contains* the text,
    /// including the shell that launched the check.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// The last stderr lines the child wrote, oldest first.
    ///
    /// Bounded to the last 64 lines; this is what milestone 5's probe records as
    /// `probe.stderr_tail` when a handshake fails.
    #[must_use]
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.lines.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Every stderr line from now on — and first, **replayed under the same lock the reader
    /// holds**, every line already in the tail, so tapping after [`spawn`] loses and duplicates
    /// nothing (plan MOD-21 D14: a login URL can be printed during `initialize`, before the caller
    /// that wants it has finished wiring itself up). Lines only, lossy UTF-8, exactly as the tail.
    ///
    /// **One tap per child.** A second call replaces the first, whose receiver then ends; the
    /// replacement replays the tail like any other late tap. A receiver dropped by its caller
    /// uninstalls the tap on the next line, so a chatty child stops paying for it.
    ///
    /// The tail is untouched by any of this: it keeps its 64-line bound and its
    /// contents whatever the tap holds, because [`stderr_tail`](Self::stderr_tail) is what a failed
    /// handshake is explained with and a tap must not be draining it.
    ///
    /// The receiver ends when the child's stderr does. That is what lets a `select!` over it retire
    /// the arm on `None` rather than poll a stream that will never speak again.
    pub fn tap_stderr(&mut self) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        // A poisoned lock leaves the tap uninstalled and `tx` dropped here, so the caller's
        // receiver ends at once: the same degradation `stderr_tail` answers with an empty tail.
        if let Ok(mut tail) = self.stderr_tail.lock() {
            for line in &tail.lines {
                // `rx` is alive in this frame, so the send cannot fail.
                let _ = tx.send(line.clone());
            }
            if !tail.ended {
                tail.tap = Some(tx);
            }
        }
        rx
    }

    /// Reads the child's stdout as UTF-8, lossily, to end-of-stream **or [`STDOUT_LIMIT`]**.
    ///
    /// For a one-shot child (`--version`, `npm root -g`). A streaming transport takes the handle
    /// with [`take_stdout`](Self::take_stdout) instead.
    ///
    /// A child still writing at the cap is **killed** and what was read is answered: a tool that
    /// streams on `--version` has already said whatever its version pattern was going to match, and
    /// leaving it running would keep the pipe — and the caller — alive for nothing. Hitting the cap
    /// is therefore an answer, not an error; the `warn!` names the command.
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the read fails, or when stdout was already taken.
    pub async fn read_stdout_to_end(&mut self) -> Result<String> {
        let mut stdout = self
            .stdout
            .take()
            .ok_or_else(|| DriverError::Transport("stdout was already taken".to_owned()))?
            .into_inner();
        let mut buffer = Vec::new();
        // One byte past the cap, so "the child had more to say" is distinguishable from "the child
        // said exactly this much and stopped".
        (&mut stdout)
            .take(STDOUT_LIMIT as u64 + 1)
            .read_to_end(&mut buffer)
            .await
            .map_err(|error| DriverError::Transport(format!("reading stdout: {error}")))?;
        if buffer.len() > STDOUT_LIMIT {
            buffer.truncate(STDOUT_LIMIT);
            warn!(
                pid = ?self.pid(),
                limit = STDOUT_LIMIT,
                "a one-shot child wrote past the stdout cap; killing it and answering what it said"
            );
            if let Err(error) = self.kill_tree().await {
                warn!(%error, "the child that outran the stdout cap did not die cleanly");
            }
        }
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }

    /// Waits for the child to exit.
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the wait itself fails.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        self.child
            .wait()
            .await
            .map_err(|error| DriverError::Transport(format!("waiting for the agent: {error}")))
    }

    /// Signals the child's process group **without** killing it: the polite half of
    /// [`kill_tree`](Self::kill_tree).
    ///
    /// The two existing exits from a session are both `SIGKILL` — `kill_tree` and
    /// [`start_kill`](Self::start_kill) — and a transport whose only cancel is a kill cannot let an
    /// agent finish the message that ends its turn. A cancel sequence therefore asks first
    /// ([`StopSignal::Interrupt`]), waits out its grace window, and kills only what is still there
    /// (`docs/ANA-4.md` §4.4, plan D81). What the child does with the request is the child's
    /// business: it may exit with a status of its own, it may ignore the signal entirely, and this
    /// method neither waits for it nor claims it worked.
    ///
    /// The **group**, not the process: `process-wrap` puts the child in its own group at spawn, so
    /// this is a `killpg` and reaches a helper the agent spawned exactly as `kill_tree` does. A
    /// supervisor that signalled only the pid it can name would leave the rest of the tree running
    /// with nobody holding it.
    ///
    /// **On Windows there are no signals**, and this answers [`DriverError::Transport`] saying so
    /// rather than silently doing nothing: a caller that believed a request had been delivered
    /// would spend its grace window waiting for an exit nobody asked for. The Windows cancel is
    /// therefore stdin's close, then the grace, then the job object — which is what
    /// [`kill_tree`](Self::kill_tree) already is. (`unsafe_code = "forbid"`, so the signal goes
    /// through `process-wrap`'s own safe wrapper and there is no `libc::kill` here.)
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the signal is refused — including a group that has already
    /// exited, which is not an error a caller has to treat as one — and on every non-unix platform.
    pub fn signal(&self, signal: StopSignal) -> Result<()> {
        #[cfg(unix)]
        {
            self.child.signal(signal.number()).map_err(|error| {
                DriverError::Transport(format!("signalling the agent with {signal}: {error}"))
            })
        }
        #[cfg(not(unix))]
        {
            Err(DriverError::Transport(format!(
                "this platform has no {signal}; a cancel here closes stdin and then kills the tree"
            )))
        }
    }

    /// Kills the whole process tree: the job object on Windows, the process group on unix.
    ///
    /// This is what [`AgentSession::cancel`] falls back to after its grace window
    /// (milestone 3 wires it up).
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the kill fails.
    ///
    /// [`AgentSession::cancel`]: crate::driver::AgentSession::cancel
    pub async fn kill_tree(&mut self) -> Result<()> {
        Box::into_pin(self.child.kill())
            .await
            .map_err(|error| DriverError::Transport(format!("killing the agent: {error}")))
    }

    /// Signals the tree without waiting for it: the **synchronous** half of
    /// [`kill_tree`](Self::kill_tree), for a `Drop`.
    ///
    /// `kill_tree` is `start_kill` followed by `wait`, and a `Drop` cannot await the second half.
    /// Blocking on it there is not an option either — a `Drop` running inside the runtime would be
    /// blocking a worker thread — so the child is signalled and left for the reaper.
    ///
    /// **Two callers reach it the same way**: through a `ChildGuard`'s `Drop`, each with a
    /// `kill_tree` on every path it can await instead. The probe's handshake (`acp::handshake`) is
    /// what makes an **aborted** probe task leave no live agent behind (the milestone-3 CRITICAL,
    /// `682a423`); since milestone 6 (plan D61) [`acp::open_session`]'s session task is the other,
    /// and the abort there is the handshake timeout giving up on an adapter that never answered.
    /// The two have the same shape because the problem does: a future that may be dropped
    /// mid-await, owning a process no other frame can name.
    ///
    /// [`acp::open_session`]: crate::acp::open_session
    ///
    /// # Errors
    /// [`DriverError::Transport`] when the signal is refused.
    pub fn start_kill(&mut self) -> Result<()> {
        self.child
            .start_kill()
            .map_err(|error| DriverError::Transport(format!("killing the agent: {error}")))
    }
}

// ---------------------------------------------------------------------------------------------
// The byte streams a session runs over
// ---------------------------------------------------------------------------------------------

/// The byte streams a session runs over, plus the child that owns them when there is one.
///
/// Held as `tokio` traits and adapted to `futures::io` exactly once, where the transport is built:
/// everything else in this crate speaks `tokio`.
///
/// Beside [`Spawned`] rather than inside a transport module because it is the seam between the two:
/// a spawn produces one of these, and *which* protocol then runs over the pair is the caller's
/// business. A second transport that had to import the first one's type to say "a reader, a writer
/// and maybe a child" would be depending on the first transport for nothing.
pub struct ChildIo {
    /// The agent's stdout, as this client reads it.
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    /// The agent's stdin, as this client writes it.
    pub writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    /// `Some` when the transport spawned the process; `None` for an in-process pair.
    pub child: Option<Spawned>,
}

impl ChildIo {
    /// The streams of a child this crate spawned, with the child carried along.
    ///
    /// Factored out of the ACP driver's own spawn so the probe's tier 2 reaches the agent exactly
    /// the way a session does — one place decides what "piped stdio" means, and a probe that
    /// resolved a launch differently from the chat would be measuring the wrong box.
    ///
    /// # Errors
    /// [`DriverError::Spawn`] when stdin or stdout was not piped (already taken, or a spawn that
    /// did not request them).
    pub fn from_spawned(mut spawned: Spawned) -> Result<Self> {
        let writer = spawned
            .take_stdin()
            .ok_or_else(|| DriverError::Spawn("the agent's stdin was not piped".to_owned()))?;
        let reader = spawned
            .take_stdout()
            .ok_or_else(|| DriverError::Spawn("the agent's stdout was not piped".to_owned()))?;
        Ok(Self {
            reader: Box::new(reader.into_inner()),
            writer: Box::new(writer.into_inner()),
            child: Some(spawned),
        })
    }
}

impl core::fmt::Debug for ChildIo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChildIo")
            .field("child", &self.child.is_some())
            .finish()
    }
}

// ---------------------------------------------------------------------------------------------
// The child guard
// ---------------------------------------------------------------------------------------------

/// A [`Spawned`] owned by the frame or task that started it, killed by its `Drop`.
///
/// The guard, not `Spawned`, is what makes a dropped or aborted future leave no running process.
/// `Spawned` deliberately has no killing `Drop` of its own, and that is the policy rather than an
/// omission: a process type that signalled itself whenever it went out of scope could not be moved
/// into a task, parked in a `Mutex` or returned from a builder without each of those moves needing
/// a `mem::forget` to mean what it says. The guard carries the policy, so each caller states once
/// where the kill belongs.
///
/// Three holders, all of them futures that can be dropped before they finish. [`acp::handshake`]
/// holds one because `connect_with` runs the connection actors and the foreground future under a
/// `select` and an actor that fails first **drops** the foreground future (the milestone-3
/// CRITICAL, `682a423`). The probe's one-shot children (`--version`, `npm root -g`) hold one
/// because `AgentRuntime::shutdown` and the background limit both `abort()` the probe task, and an
/// aborted task drops its future mid-await. Since milestone 6 (plan D61) [`open_session`]'s session
/// task holds one too: it hands its child to a task that outlives the call which started it, so no
/// caller's frame can kill it, and the handshake timeout that abandons that task leaves the kill to
/// the guard the abort drops. What differs between the three is only *where* the guard lives.
///
/// [`open_session`]: crate::acp::open_session
/// [`acp::handshake`]: crate::acp::handshake()
pub(crate) struct ChildGuard {
    child: Option<Spawned>,
}

impl ChildGuard {
    /// A guard over `child`. `None` is a guard over nothing — a [`ChildIo`] built from an
    /// in-process duplex has no process to kill.
    pub(crate) fn new(child: Option<Spawned>) -> Self {
        Self { child }
    }

    /// The child, while the guard still holds one.
    pub(crate) fn child_mut(&mut self) -> Option<&mut Spawned> {
        self.child.as_mut()
    }

    /// What the child last wrote to stderr, for an error message.
    pub(crate) fn stderr_tail(&self) -> Vec<String> {
        self.child
            .as_ref()
            .map(Spawned::stderr_tail)
            .unwrap_or_default()
    }

    /// Kills the tree **and reaps it**, so a caller that got an answer knows there is nothing left
    /// running. A second call is a no-op, and so is the `Drop` that follows it.
    pub(crate) async fn kill_and_reap(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Err(err) = child.kill_tree().await {
            warn!(%err, "the guarded process tree did not die cleanly");
        }
        if let Err(err) = child.wait().await {
            warn!(%err, "the guarded process could not be reaped");
        }
    }

    /// Releases a child that has **already exited and been reaped**, so the `Drop` below does not
    /// signal it.
    ///
    /// On unix `start_kill` is a `killpg` at the group, which answers `ESRCH` once the group is
    /// gone: without this, every successful one-shot child would log a spurious warning about
    /// "surviving" its guard.
    pub(crate) fn release(&mut self) {
        self.child = None;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        // Signal only: a `Drop` cannot await the reap, and blocking for it here would block a
        // runtime worker. What that leaves is a *signalled* process, not a running one, and not a
        // zombie for the life of the program either: dropping the `tokio::process::Child` inside
        // hands it to tokio's global orphan queue (1.53's `process/unix/reap.rs` `Reaper::drop`,
        // or `pidfd_reaper.rs` on the pidfd path), which the process driver drains on every park
        // once `SIGCHLD` has arrived (`runtime/process.rs:33`). Windows has no zombie state at
        // all. `htui` therefore owes this no reaper task of its own.
        if let Err(err) = child.start_kill() {
            warn!(%err, "the guarded process survived its guard");
        }
    }
}

/// Starts the resolved command under process supervision, with piped stdio.
///
/// The command is resolved through `which` first: `std::process::Command` does not consult
/// `PATHEXT`, so a Windows `.cmd` shim handed straight to `CreateProcess` fails with `os error
/// 193` (ANA-4 §4.6). On Windows the child gets `CREATE_NO_WINDOW` and a job object; on unix it
/// becomes a process-group leader, so a kill reaches the agent's own children too.
///
/// A refused job-object assignment is a **downgrade, not a failure**: the child still runs, and
/// [`Spawned::job_object`] says so.
///
/// **Async, and the `which` lookup runs on a blocking thread.** Resolving a command walks `PATH`
/// with a `stat` per candidate — and on Windows a `PATHEXT` product per candidate — which is
/// filesystem I/O, not arithmetic. Milestone 3 calls this from inside `AgentDriver::start`, so it
/// would sit on a runtime worker; `crates/htui-store/src/cache/pending.rs` already sets this
/// crate family's precedent of pushing blocking file I/O through `spawn_blocking`, and doing it
/// *inside* this function makes the type system enforce it rather than the caller's memory.
///
/// The rest of the work stays on the caller's thread on purpose: `tokio::process::Command::spawn`
/// is non-blocking, and the stderr reader is a `tokio::spawn`ed task, so this function must be
/// called from within a runtime.
///
/// # Errors
/// [`DriverError::Spawn`] when the command cannot be found or the operating system refuses the
/// spawn; [`DriverError::Transport`] when the blocking lookup task itself fails to run.
pub async fn spawn(launch: &ResolvedLaunch, cwd: &Path) -> Result<Spawned> {
    let command = launch.command.clone();
    let program = tokio::task::spawn_blocking(move || which::which(&command))
        .await
        .map_err(|error| DriverError::Transport(format!("resolving the command: {error}")))?
        .map_err(|error| {
            DriverError::Spawn(format!("`{}` is not executable: {error}", launch.command))
        })?;

    let (mut child, job_object) = spawn_supervised(&program, launch, cwd)?;

    let stdin = child
        .stdin()
        .take()
        .map(TokioAsyncWriteCompatExt::compat_write);
    let stdout = child.stdout().take().map(TokioAsyncReadCompatExt::compat);
    let stderr_tail = Arc::new(Mutex::new(StderrTail {
        lines: VecDeque::with_capacity(STDERR_TAIL_LINES),
        tap: None,
        ended: false,
    }));

    if let Some(stderr) = child.stderr().take() {
        let tail = Arc::clone(&stderr_tail);
        tokio::spawn(async move {
            // `read_until` and `from_utf8_lossy`, not `lines()`: that iterator answers
            // `Err(InvalidData)` on a single byte that is not UTF-8 and **ends** — which would end
            // this reader, close the tap and freeze the tail on the first stray byte an adapter
            // wrote, and then stop draining the pipe at all, so a child that filled it would block
            // on its own stderr. A login is where that costs the most: the line after the stray
            // byte is often the one carrying the URL. Lossy is what this file's `tap_stderr` doc
            // has promised all along.
            let mut reader = BufReader::new(stderr);
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer).await {
                    // End of stream.
                    Ok(0) => break,
                    Ok(_) => {}
                    // A real I/O failure on the pipe, which `lines()` also stopped at: there is
                    // nothing left to read from a descriptor that answers an error.
                    Err(_) => break,
                }
                // The separators `lines()` strips, stripped the same way: a `\n`, and the `\r`
                // before it that a Windows child writes.
                if buffer.last() == Some(&b'\n') {
                    buffer.pop();
                    if buffer.last() == Some(&b'\r') {
                        buffer.pop();
                    }
                }
                let line = String::from_utf8_lossy(&buffer).into_owned();
                let Ok(mut tail) = tail.lock() else { return };
                if tail.lines.len() == STDERR_TAIL_LINES {
                    tail.lines.pop_front();
                }
                tail.lines.push_back(line.clone());
                // The tap is taken out and put back rather than borrowed: a send whose receiver is
                // gone drops the sender here, so the next line costs nothing.
                if let Some(tap) = tail.tap.take()
                    && tap.send(line).is_ok()
                {
                    tail.tap = Some(tap);
                }
            }
            // End of stream: close the tap so a caller waiting on it is told, rather than left
            // holding a receiver whose sender no task will ever write to again.
            if let Ok(mut tail) = tail.lock() {
                tail.ended = true;
                tail.tap = None;
            }
        });
    }

    Ok(Spawned {
        child,
        stdin,
        stdout,
        stderr_tail,
        job_object,
    })
}

/// Builds the wrapped command and spawns it, returning the child and whether a job object was
/// assigned. Split out so the platform difference is one function, not a `cfg` in the middle of
/// [`spawn`].
fn spawn_supervised(
    program: &Path,
    launch: &ResolvedLaunch,
    cwd: &Path,
) -> Result<(Box<dyn process_wrap::tokio::ChildWrapper>, bool)> {
    use process_wrap::tokio::CommandWrap;

    let build = || {
        let mut command = tokio::process::Command::new(program);
        command
            .args(&launch.args)
            .envs(&launch.env)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        CommandWrap::from(command)
    };

    #[cfg(unix)]
    {
        let mut wrapped = build();
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        let child = wrapped
            .spawn()
            .map_err(|error| DriverError::Spawn(format!("{}: {error}", program.display())))?;
        // Job objects are a Windows mechanism; on unix the process group is the equivalent, and
        // reporting `true` here would make the field mean two different things per platform.
        Ok((child, false))
    }

    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::PROCESS_CREATION_FLAGS;

        let mut wrapped = build();
        wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)));
        wrapped.wrap(JobObject);
        match wrapped.spawn() {
            Ok(child) => Ok((child, true)),
            Err(error) => {
                // A refused job object is a degraded supervision mode, not a dead session: retry
                // without it and record that the process tree is unsupervised beyond the child.
                warn!(
                    program = %program.display(),
                    %error,
                    "job object assignment refused; spawning without it"
                );
                let mut wrapped = build();
                wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)));
                let child = wrapped.spawn().map_err(|error| {
                    DriverError::Spawn(format!("{}: {error}", program.display()))
                })?;
                Ok((child, false))
            }
        }
    }
}
