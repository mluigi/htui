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

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

use agent_client_protocol::AcpAgentConfig;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
// Only the Windows spawn path has a degraded mode to report; on unix the import would be unused.
#[cfg(windows)]
use tracing::warn;

use crate::driver::{PermissionPolicy, RedactedEnv};
use crate::error::{DriverError, Result};

/// How many stderr lines a [`Spawned`] keeps. Enough to explain a failed handshake, bounded so a
/// chatty agent cannot grow the buffer without limit.
const STDERR_TAIL_LINES: usize = 64;

/// `CREATE_NO_WINDOW`: a TUI must not flash a console window when it spawns an agent
/// (ANA-4 §4.6). Named here rather than imported so the constant reads the same on every platform.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

wire_enum!(
    /// `agent.settings.quota.source` (§5.2, §7).
    #[derive(Default)]
    QuotaSource {
        /// ACP's `_meta.rate_limit` on the session update.
        AcpMetaRateLimit => "acp_meta_rate_limit",
        /// A rate-limit event in the CLI's JSON stream.
        CliRateLimitEvent => "cli_rate_limit_event",
        /// The CLI's status line.
        CliStatusLine => "cli_status_line",
        /// No quota is reported. The default: a row that says nothing promises nothing.
        #[default]
        None => "none",
    }
);

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
#[derive(Clone, PartialEq, Eq)]
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

/// A running agent process and the handles a transport talks to it through.
#[derive(Debug)]
pub struct Spawned {
    child: Box<dyn process_wrap::tokio::ChildWrapper>,
    stdin: Option<Compat<ChildStdin>>,
    stdout: Option<Compat<ChildStdout>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
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
            .map(|tail| tail.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Reads the child's stdout to end-of-stream as UTF-8, lossily.
    ///
    /// For a one-shot child (`--version`, a probe). A streaming transport takes the handle with
    /// [`take_stdout`](Self::take_stdout) instead.
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
        stdout
            .read_to_end(&mut buffer)
            .await
            .map_err(|error| DriverError::Transport(format!("reading stdout: {error}")))?;
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
    let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));

    if let Some(stderr) = child.stderr().take() {
        let tail = Arc::clone(&stderr_tail);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(mut tail) = tail.lock() else { return };
                if tail.len() == STDERR_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
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
