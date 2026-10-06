//! The degraded CLI transport: an agent that speaks only its own headless JSON stream, reaching
//! the same chat tab, recorder, store rows and replay as an ACP one (`docs/ANA-4.md` §4.3, §4.4,
//! §6.2, §7; MOD-2 milestone 8).
//!
//! The split mirrors `acp/`: the supervisor and the session task live in this file, and the
//! wire → [`DriverEvent`] mapping lives alone in [`claude`], which imports no process type and is
//! unit-testable from a single recorded line.
//!
//! **What differs from `acp/`, stated once.** There is no protocol layer at all: a line out is a
//! user message, a line in is a JSON value, and nothing negotiates. So there is no handshake beyond
//! the first `system/init`, no permission channel on the wire (§4.3 fixes
//! `DriverCaps { permission_requests: false, edit_proposals: false, plans: false }` for this
//! transport and, without a prompt port, [`AgentSession::answer_permission`] answers
//! [`DriverError::Unsupported`]), and a cancel is a **signal** rather than a notification — which is why it is the one sequence in this
//! file written from measurements instead of from a specification (plan D81, findings F-1..F-3).
//!
//! **MOD-11 D18, the one permission channel this transport has.** When `htui`'s MCP server hosts
//! `permission_prompt` for the session, `SessionSpec.prompt` carries a port: the CLI is started with
//! `--permission-prompt-tool` naming that tool, every call it gates arrives on the port as a
//! [`PromptRequest`], is announced as an ordinary `permission_request`, and is completed by
//! [`AgentSession::answer_permission`]. The capability triple stays all-false (the interlock reads the
//! row, not the session); the engine admits a CLI agent to a gated phase only when it hosts the tool.
//!
//! [`DriverEvent`]: crate::event::DriverEvent

pub mod claude;
mod mcp_file;

// MOD-79 (blueprint G-2): `open_session` is `pub` and takes one, so the type must be nameable
// outside the private module.
pub use mcp_file::McpConfigFile;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use htui_core::model::{Agent as AgentRow, AgentBox};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, McpServerSpec,
    PermissionAnswer, PermissionRequestId, SessionSpec, ToolExposure,
};
use crate::error::{DriverError, Result};
use crate::event::{
    DoneEvent, DriverEnvelope, DriverEvent, ErrorEvent, OtherEvent, PermissionOption,
    PermissionOptionKind, PermissionRequestEvent, SESSION_STARTED, Stamp, StopReason,
    TRANSPORT_CLOSED, TerminalReason, ToolResultEvent, ToolResultStatus,
};
use crate::launch::{
    AgentLaunch, AgentSettings, ChildGuard, ChildIo, CliSettings, ResolvedLaunch, StopSignal,
};
use crate::prompt_bridge::{PROMPT_TOOL, PromptPort, PromptRequest, PromptVerdict};
use crate::registry::TransportBuilder;

/// The adapter id this transport registers under (plan D12): `cli/<settings.cli.stream>`.
pub const ADAPTER_ID: &str = "cli/claude_stream_json";

/// The `settings.cli.stream` value that selects it — half of [`ADAPTER_ID`], and what a registry
/// row declares.
///
/// A **dialect** name, not an agent name (`R-AGT-5`): two rows may declare it, and nothing in this
/// module ever reads `agent.name` to decide anything.
pub const STREAM: &str = "claude_stream_json";

/// How long the first `system/init` may take before [`open_session`] gives up.
///
/// The CLI's login refusal prints to stderr and exits, which is EOF and is reported at once; this
/// bounds the other shape — an agent that hangs before saying anything — so a failed `ChatStart`
/// cannot hold the tab that issued it forever.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Depth of the session task's event channel; [`crate::acp::EVENTS_CAPACITY`]'s reason, and the
/// same number so the two transports back-pressure a slow consumer alike.
pub const EVENTS_CAPACITY: usize = 256;

/// The grace window a session gets when its **handle** is dropped rather than cancelled;
/// [`crate::acp::DROP_GRACE`]'s reason.
pub const DROP_GRACE: Duration = Duration::from_secs(1);

/// The most one stdout line may occupy before the reader forwards it unfinished.
///
/// One mebibyte, which is two orders of magnitude above the largest line any recorded transcript
/// holds (a `system/init` listing this box's tools and commands) and small enough that a child
/// writing without newlines cannot exhaust memory through it. See [`read_lines`].
const MAX_LINE_BYTES: u64 = 1024 * 1024;

/// `other.update` of a stdout line that is not JSON at all.
///
/// Not an error, and deliberately not a reason to stop reading (blueprint H-23): §6.2's rule for a
/// shape `htui` does not recognize is "stored verbatim", and a line a future release prints in
/// front of its stream — a warning, a progress bar, a crash trace — is exactly the thing a reader
/// of the transcript will want. No recorded transcript contains one (F-15), which is why this is
/// written from the rule rather than from a fixture.
pub const UNPARSED: &str = "<unparsed>";

// ---------------------------------------------------------------------------------------------
// The invocation (`docs/ANA-4.md` §4.4)
// ---------------------------------------------------------------------------------------------

/// The argv of `docs/ANA-4.md` §4.4, assembled from the row and the spec. Pure, and unit-tested as
/// a list rather than through a process.
///
/// `mcp_config` is the path of the [`McpConfigFile`] the caller wrote for `spec.mcp`; `argv` does not
/// read `spec.mcp` itself, so the code that owns the file is the code that decides the flag.
///
/// Order, and every position in it is a decision:
///
/// 1. the row's own resolved `args` first, so a registry row that wraps the CLI in something (`npx`,
///    a shim) keeps its own leading arguments where that something expects them;
/// 2. the fixed flags §4.4 verified: `-p` with **no positional prompt**, because the prompt travels
///    on stdin as the first user message and an argv is visible in `ps` to every account on the box
///    (blueprint P-2);
/// 3. the row's `--permission-mode`, omitted when the row names none rather than guessed at;
/// 4. the session id **or** the resume id, never both — `--session-id` mints (D84), `--resume`
///    continues, and the CLI refuses the pair (blueprint H-18);
/// 5. the spec's model and extra directories;
/// 6. the budget, **only above zero** — see below;
///
///    6¼. `htui`'s own MCP servers (MOD-11 D8, MOD-79): one `--mcp-config=<path>` argument when
///    `mcp_config` is `Some`, `=`-joined so the CLI's variadic parse cannot swallow the argument after
///    it. The path names the `0600` file [`McpConfigFile`] wrote [`mcp_config`]'s JSON into; the JSON
///    itself never reaches the argv, because its `env` carries `HTUI_MCP_TOKEN` and an argv is readable
///    by every account on the box (blueprint P-2's reason). No `--strict-mcp-config` — the operator's
///    own servers stay — and `--tools` is left as it is, because it never filters MCP tools;
///
///    6⅓. the pair `--permission-prompt-tool` [`PROMPT_TOOL`] when the spec carries a prompt port
///    (MOD-11 D18). A pair is safe here: the flag takes exactly one value;
///
///    6½. the step's narrowing (MOD-26 D11): `--tools=<allow>` when the allow-list is not empty,
///    then `--disallowedTools=<deny and the names deny_kinds inverts to>` when that is not empty,
///    each one `=`-joined argument; `--allowedTools` is never emitted (I-1);
///
///    6¾. a session with no tool at all ([`ToolExposure::no_tools`], MOD-55 review M1): `--tools=`
///    in place of the allow-list and `--strict-mcp-config`, while 3, 6¼ and 6⅓ emit nothing — no
///    permission mode, no server (the operator's own included), no prompt tool. [`refuse_widening`]
///    is what keeps 7 from undoing it;
/// 7. `settings.cli.extra_args` **last**, so an operator's repeated flag is the one the CLI keeps.
///
/// **The budget flag is omitted at zero, and that is a measurement, not a nicety** (plan F-10):
/// `--max-budget-usd 0` is refused before the CLI reads a byte of stdin — the process exits 1 with
/// no stdout at all — so passing it for an absent cap would turn "no cap" into "no turn".
#[must_use]
pub fn argv(
    row_args: &[String],
    cli: &CliSettings,
    spec: &SessionSpec,
    session_id: &str,
    mcp_config: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = row_args.to_vec();
    // `--verbose` is what makes `stream-json` emit every envelope rather than the terminal
    // `result` alone, and `--include-partial-messages` is what turns the `stream_event` channel on
    // — the deltas the chat tab renders as the reply arrives (§4.4).
    args.extend(
        [
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
        ]
        .map(ToOwned::to_owned),
    );

    // Scoped so the borrow ends before `extra_args` is appended: the flags below are all pairs,
    // and spelling `push` twice per pair is what a reader has to check for a transposition.
    {
        let mut push = |flag: &str, value: &str| {
            args.push(flag.to_owned());
            args.push(value.to_owned());
        };
        // MOD-55 review M1: a tool-less session has nothing a mode could approve, and the seeded
        // row's `acceptEdits` must not make help unusable, so the row's mode is dropped rather
        // than refused.
        if !cli.permission_mode.is_empty() && !spec.tools.no_tools {
            push("--permission-mode", &cli.permission_mode);
        }
        match spec.resume.as_ref() {
            Some(resume) => push("--resume", resume.as_str()),
            None => push("--session-id", session_id),
        }
        if let Some(model) = spec.model.as_deref() {
            push("--model", model);
        }
        for dir in &spec.extra_dirs {
            push("--add-dir", &dir.to_string_lossy());
        }
        if let Some(micros) = spec.budget_micros.filter(|micros| *micros > 0) {
            push("--max-budget-usd", &usd(micros));
        }
    }

    let no_tools = spec.tools.no_tools;
    // MOD-11 D8: after the last pair, before the narrowing, so `extra_args` still wins. MOD-79: the
    // file's path, never the JSON. Lossy as `--add-dir` above, and exact here: `McpConfigFile::write`
    // refuses a path that is not UTF-8.
    if let Some(config) = mcp_config.filter(|_| !no_tools) {
        args.push(format!("--mcp-config={}", config.to_string_lossy()));
    }
    // MOD-11 D18: every gated call asks `htui`'s prompt tool instead of the permission mode. A
    // tool-less session has no server to serve it, and no call to gate.
    if spec.prompt.is_some() && !no_tools {
        args.push("--permission-prompt-tool".to_owned());
        args.push(PROMPT_TOOL.to_owned());
    }

    // MOD-26 D11: the step's narrowing, each as one `=`-joined argument (the closure above pushes
    // pairs). `--tools` restricts the built-in set and is omitted when `allow` is empty —
    // `--tools=""` would disable every tool. `--allowedTools` is never emitted: it auto-approves
    // (I-1, probed).
    //
    // MOD-55 review M1: `no_tools` is that empty `--tools=` on purpose, and `--strict-mcp-config`
    // with no `--mcp-config` beside it loads no server at all. The deny list still travels.
    if no_tools {
        args.push("--tools=".to_owned());
        args.push("--strict-mcp-config".to_owned());
    } else if !spec.tools.allow.is_empty() {
        args.push(format!("--tools={}", spec.tools.allow.join(",")));
    }
    let denied = disallowed(&spec.tools);
    if !denied.is_empty() {
        args.push(format!("--disallowedTools={}", denied.join(",")));
    }

    args.extend(cli.extra_args.iter().cloned());
    args
}

/// The `extra_args` flags that would widen a tool-less session back: they come last on [`argv`],
/// so the CLI keeps them over `--tools=` and `--strict-mcp-config` (MOD-55 review M1).
const WIDENING_FLAGS: [&str; 6] = [
    "--allowedTools",
    "--allowed-tools",
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
    "--tools",
    "--mcp-config",
];

/// The `--permission-mode` values that approve without asking.
const WIDENING_MODES: [&str; 2] = ["bypassPermissions", "acceptEdits"];

/// Refuses a tool-less session ([`ToolExposure::no_tools`]) on a row whose `extra_args` would
/// widen it back; `Ok` for every other session, which this check is no business of.
///
/// Refused rather than stripped (MOD-55 review M1): `extra_args` are the operator's, passed
/// verbatim and last so their copy wins ([`argv`], 7), and silently dropping one would be a second
/// meaning nobody configured. The row's own `permission_mode` is the exception — [`argv`] drops it
/// — because the seeded `claude-cli` row carries `acceptEdits` and refusing it would refuse help
/// on every default install.
///
/// # Errors
/// [`DriverError::Transport`] naming the first widening flag. Never [`DriverError::Spawn`]:
/// nothing was spawned, and the chat runtime re-probes a row whose spawn failed.
pub fn refuse_widening(cli: &CliSettings, tools: &ToolExposure) -> Result<()> {
    if !tools.no_tools {
        return Ok(());
    }
    let mut args = cli.extra_args.iter().map(String::as_str).peekable();
    while let Some(arg) = args.next() {
        let (flag, joined) = arg
            .split_once('=')
            .map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let widens = if flag == "--permission-mode" {
            let value = joined.or_else(|| args.peek().copied());
            value.is_some_and(|value| WIDENING_MODES.contains(&value))
        } else {
            WIDENING_FLAGS.contains(&flag)
        };
        if widens {
            return Err(DriverError::Transport(format!(
                "a session with no tools refuses this row's `{arg}` extra argument: it would \
                 widen the session back"
            )));
        }
    }
    Ok(())
}

/// The MCP config JSON for `servers` (MOD-11 D8): the CLI's own
/// `{"mcpServers":{<name>:{"type":"stdio","command","args","env"}}}`, serialised compactly.
/// `None` for an empty slice, which is what keeps a session with no server on today's argv.
///
/// It is written to a file ([`McpConfigFile`], MOD-79) and never put on the argv: `env` carries the
/// session's `HTUI_MCP_TOKEN`.
///
/// Both maps are `BTreeMap`s, so the bytes are stable: servers by name, `env` by key.
#[must_use]
pub fn mcp_config(servers: &[McpServerSpec]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    let servers: BTreeMap<&str, Value> = servers
        .iter()
        .map(|server| {
            let entry = json!({
                "type": "stdio",
                "command": server.command,
                "args": server.args,
                "env": server.env,
            });
            (server.name.as_str(), entry)
        })
        .collect();
    Some(json!({ "mcpServers": servers }).to_string())
}

/// `deny`, then every name `deny_kinds` inverts to ([`claude::tool_names`]), then — when
/// `command_run` is exposed (MOD-11 D17, R-MCP-4) — `Bash(<prefix>:*)` per OQ-5 prefix
/// ([`HEAVY_COMMAND_PREFIXES`](htui_core::model::kind::HEAVY_COMMAND_PREFIXES)), first occurrence
/// kept.
fn disallowed(tools: &ToolExposure) -> Vec<String> {
    let inverted = tools
        .deny_kinds
        .iter()
        .flat_map(|kind| claude::tool_names(*kind).iter().copied())
        .map(str::to_owned);
    let heavy = htui_core::model::kind::HEAVY_COMMAND_PREFIXES
        .iter()
        .filter(|_| tools.command_run)
        .map(|prefix| format!("Bash({prefix}:*)"));
    let mut names: Vec<String> = Vec::new();
    for name in tools.deny.iter().cloned().chain(inverted).chain(heavy) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// USD micros as the decimal `--max-budget-usd` takes: integer arithmetic, six places, no float.
///
/// `300` → `"0.000300"`, `1_500_000` → `"1.500000"`. A float round-trip is what this exists to
/// avoid — the recorder's client-side cap and the CLI's server-side one must read **one** number
/// (D83, D90), and two caps that disagree in the sixth decimal place are worse than one.
///
/// A negative figure never reaches here: `ProjectCaps::from_settings` refuses it and [`argv`] gates
/// on `> 0` besides. The `debug_assert!` says so where it would be violated, and the clamp keeps
/// the release build producing a well-formed decimal rather than the `-0.-000300` the naive
/// arithmetic would emit.
#[must_use]
pub fn usd(micros: i64) -> String {
    debug_assert!(micros >= 0, "a per-run cap in micros is never negative");
    let micros = micros.max(0);
    format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000)
}

/// One `--input-format stream-json` user message, as the line it is written as.
///
/// **The measured shape, not the documented one.** Every one of the fourteen recorded probe
/// transcripts wrote exactly these three nested keys and nothing else, and the CLI accepted all of
/// them (`tests/fixtures/claude_stream_json_*.jsonl`, plan T55/T56). The vendor SDK additionally
/// carries a `session_id` on each line; it is left off here because the fixtures are this
/// milestone's evidence and none of them contains one — the id is already on the argv, which is
/// where `--session-id` put it and where `system/init` echoes it back from.
fn stdin_line(text: &str) -> String {
    let line = json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    });
    // `Value`'s `Display` is compact JSON and cannot fail, which `serde_json::to_string` can only
    // promise through a `Result` nobody here could act on.
    format!("{line}\n")
}

// ---------------------------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------------------------

/// Where a session's byte streams come from; `crate::acp`'s fork, for its reasons.
enum IoSource {
    /// Spawn a child at `start`: what the probe recorded when there is a usable recording,
    /// otherwise the row's tools resolved and substituted (D58, `crate::launch::launch_from`).
    Spawn {
        /// The row's `launch` document.
        launch: Box<AgentLaunch>,
        /// `agent_box.probe.resolved`, when the snapshot passed D58's three row-side rules.
        recorded: Option<ResolvedLaunch>,
    },
    /// A pre-built pair, taken once — a real process starts once too.
    ///
    /// Boxed for `acp::IoSource::Prepared`'s reason: a [`ChildIo`] carries a `Spawned`, which is
    /// materially larger on Windows, and an unboxed variant would make every session pay for the
    /// test-support one there.
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<Box<ChildIo>>>),
}

/// The driver for a `cli` row whose `settings.cli.stream` is [`STREAM`]: one per row, holding no
/// process.
pub struct CliDriver {
    name: String,
    /// `agent.models`, the banner's fallback when `system/init` names no model (D84).
    models: Vec<String>,
    /// `settings.cli` is what [`argv`] reads; `settings.usage.scope` is what every `usage` row the
    /// mapper writes is labelled with (§5.2, §7).
    settings: AgentSettings,
    caps: DriverCaps,
    io: IoSource,
    stamp: Stamp,
    /// `agent_box.version` — the probe's `claude --version` capture — the banner's `agent_version`
    /// fallback when `system/init` names no version.
    box_version: Option<String>,
}

impl core::fmt::Debug for CliDriver {
    /// `AcpDriver`'s fields, for its reason: a [`ResolvedLaunch`] carries an environment, so a log
    /// line says **which** of D58's two paths this driver is on and never what is on it.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CliDriver")
            .field("name", &self.name)
            .field("caps", &self.caps)
            .field("stamp", &self.stamp)
            .field(
                "recorded",
                &matches!(
                    self.io,
                    IoSource::Spawn {
                        recorded: Some(_),
                        ..
                    }
                ),
            )
            .finish()
    }
}

impl CliDriver {
    /// Builds a driver from a registry row.
    ///
    /// # Errors
    /// [`DriverError::Transport`] when `agent.launch` does not parse as the §5.1 document. An
    /// unreadable `agent.settings` is **not** fatal: it falls back to the documented defaults,
    /// exactly as `crate::registry` does, because the column is hand-editable and a session that
    /// cannot read it can still run — it simply passes no `--permission-mode`.
    pub fn from_row(agent: &AgentRow, caps: DriverCaps) -> Result<Self> {
        Self::from_row_with_probe(agent, None, caps)
    }

    /// [`from_row`](Self::from_row) plus this box's `agent_box` row (plan D58).
    ///
    /// `on_box` is read here, once, rather than at every `start`, and the transport check is
    /// applied on this side of `recorded_launch` for `AcpDriver::from_row_with_probe`'s reason:
    /// `agent.transport` is hand-editable, and a recording made for the *other* transport is not
    /// stale but wrong.
    ///
    /// # Errors
    /// As [`from_row`](Self::from_row). An unreadable `agent_box.probe` is never an error.
    pub fn from_row_with_probe(
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Self> {
        let launch: AgentLaunch = serde_json::from_value(agent.launch.clone())
            .map_err(|err| DriverError::Transport(format!("agent.launch does not parse: {err}")))?;
        let recorded = on_box
            .and_then(crate::probe::ProbeSnapshot::from_row)
            .filter(|snapshot| snapshot.transport == agent.transport)
            .and_then(|snapshot| snapshot.recorded_launch().cloned());
        Ok(Self {
            name: agent.name.clone(),
            models: agent.models.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Spawn {
                launch: Box::new(launch),
                recorded,
            },
            stamp: Stamp::Wall,
            box_version: on_box.and_then(|on_box| on_box.version.clone()),
        })
    }

    /// What this session would spawn, before spawning it: D58's rules for the spec's `cwd`, with
    /// the spec's own environment applied over the row's.
    ///
    /// `spec.env` is applied **last** and wins (`R-SEC-2`): the row's environment holds paths, the
    /// spec's holds what the secret provider produced for this run. The argv is **not** assembled
    /// here — [`argv`] needs the minted session id, which is `start`'s to make.
    ///
    /// # Errors
    /// `crate::launch::launch_from`'s, unchanged; [`DriverError::Transport`] for a prepared
    /// transport, which spawns nothing and so has no launch to describe.
    pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch> {
        let (launch, recorded) = match &self.io {
            IoSource::Spawn { launch, recorded } => (launch, recorded),
            #[cfg(feature = "test-support")]
            IoSource::Prepared(_) => {
                return Err(DriverError::Transport(
                    "a prepared transport spawns nothing".to_owned(),
                ));
            }
        };
        let mut resolved = crate::launch::launch_from(launch, recorded.as_ref(), &spec.cwd).await?;
        resolved.env.extend(spec.env.clone());
        Ok(resolved)
    }

    /// This row's `settings.cli`, or the documented defaults when the row carries no block.
    ///
    /// A `cli` row with no block cannot reach this driver — the registry derives the bare `cli`
    /// adapter id for it and answers [`DriverError::UnknownAdapter`] — but a driver built by hand
    /// can, and an empty block passes no `--permission-mode` and no `extra_args`, which is the same
    /// invocation minus the row's opinions.
    fn cli_settings(&self) -> CliSettings {
        self.settings.cli.clone().unwrap_or_default()
    }

    /// A driver over a prepared pair with a deterministic clock: the conformance harness.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn over(io: ChildIo, agent: &AgentRow, caps: DriverCaps, stamp: Stamp) -> Self {
        Self {
            name: agent.name.clone(),
            models: agent.models.clone(),
            settings: serde_json::from_value(agent.settings.clone()).unwrap_or_default(),
            caps,
            io: IoSource::Prepared(Mutex::new(Some(Box::new(io)))),
            stamp,
            box_version: None,
        }
    }

    /// The streams this session runs over, with the child started if there is one to start, and
    /// the MCP config file its argv names, which the caller must keep for the session's life
    /// (MOD-79 D3).
    async fn io(
        &self,
        spec: &SessionSpec,
        session_id: &str,
    ) -> Result<(ChildIo, Option<McpConfigFile>)> {
        self.io_in(spec, session_id, None).await
    }

    /// [`io`](Self::io) with the MCP config's base named: `None` is production's
    /// ([`crate::private_dir::base`]), `Some` a directory a test owns, so the failed-spawn arm's
    /// cleanup is observable without racing every other test on the shared base (MOD-79 review
    /// L1).
    async fn io_in(
        &self,
        spec: &SessionSpec,
        session_id: &str,
        mcp_base: Option<std::path::PathBuf>,
    ) -> Result<(ChildIo, Option<McpConfigFile>)> {
        match &self.io {
            IoSource::Spawn { .. } => {
                let mut resolved = self.launch_for(spec).await?;
                // MOD-79: written after the launch resolves (a failure there leaves nothing on disk)
                // and before the spawn; a failed spawn drops it here, with this frame (H-13).
                // On a blocking thread (review L3): the base's `is_dir`, the directory's create and
                // mode read-back and the file's write are all synchronous `std::fs`, which a
                // runtime worker must not wait on. A cancelled `io` still drops the guard: the task
                // runs to its end, and tokio drops an output nobody joins, which removes the file
                // and its directory. A `JoinError` is `launch::spawn`'s lookup's mapping.
                let servers = spec.mcp.clone();
                let config = crate::contained::spawn_blocking(move || {
                    let base = mcp_base.unwrap_or_else(crate::private_dir::base);
                    McpConfigFile::write_in(&base, &servers)
                })
                .await
                .map_err(|err| DriverError::Transport(format!("writing the MCP config: {err}")))?
                // The io error names the directory or file it tried (review L5).
                .map_err(|err| DriverError::Spawn(format!("cannot write the MCP config: {err}")))?;
                resolved.args = argv(
                    &resolved.args,
                    &self.cli_settings(),
                    spec,
                    session_id,
                    config.as_ref().map(McpConfigFile::path),
                );
                let spawned = crate::launch::spawn(&resolved, &spec.cwd).await?;
                Ok((ChildIo::from_spawned(spawned)?, config))
            }
            #[cfg(feature = "test-support")]
            IoSource::Prepared(slot) => slot
                .lock()
                .map_err(|_| {
                    DriverError::Transport("the prepared transport is poisoned".to_owned())
                })?
                .take()
                .map(|io| (*io, None))
                .ok_or_else(|| {
                    DriverError::Transport("this driver's transport was already used".to_owned())
                }),
        }
    }
}

impl AgentDriver for CliDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn caps(&self) -> DriverCaps {
        self.caps
    }

    // No `authenticate` override: the default body answers `DriverError::Unsupported`, which is
    // what `caps_from`'s `authenticate: false` for every `cli` row promises (plan MOD-21 D10). The
    // vendor CLI's own login is not `htui`'s to drive and a stream adapter has nowhere to put a
    // method list.
    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        Box::pin(async move {
            // **Minted before the spawn**, because the argv carries it (D84): `--session-id` is
            // what makes the id `htui`'s to choose rather than the agent's to report, which is what
            // `session_ref` promises a later step's `--resume`. A resuming session has an id
            // already and mints nothing — the two flags are exclusive (blueprint H-18) — so the
            // mint is skipped rather than made and thrown away.
            let session_id = spec.resume.as_ref().map_or_else(
                || Uuid::now_v7().to_string(),
                |resume| resume.as_str().to_owned(),
            );
            // MOD-55 review M1: before the spawn, so a refused row starts nothing.
            refuse_widening(&self.cli_settings(), &spec.tools)?;
            let (io, mcp_config) = self.io(&spec, &session_id).await?;
            let options = SessionOptions {
                agent_name: self.name.clone(),
                models: self.models.clone(),
                settings: self.settings.clone(),
                stamp: self.stamp,
                init_timeout: INIT_TIMEOUT,
                box_version: self.box_version.clone(),
                session_id,
            };
            let session = open_session(io, spec, prompt, options, mcp_config).await?;
            Ok(Box::new(session) as Box<dyn AgentSession>)
        })
    }
}

/// The [`TransportBuilder`] registered under [`ADAPTER_ID`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeStreamAdapter;

impl TransportBuilder for ClaudeStreamAdapter {
    fn build(
        &self,
        agent: &AgentRow,
        on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>> {
        Ok(Box::new(CliDriver::from_row_with_probe(
            agent, on_box, caps,
        )?))
    }
}

// ---------------------------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------------------------

/// What the handle asks the session task to do.
#[derive(Debug)]
pub enum Command {
    /// Start a new turn with this text.
    FollowUp(String),
    /// MOD-11 D18: complete the parked prompt `request_id` (a session with a prompt port only).
    Answer {
        /// The request the session announced.
        request_id: PermissionRequestId,
        /// What the user or the policy chose.
        answer: PermissionAnswer,
        /// `Ok` once the prompt tool has its verdict; `Transport` for an id that is not parked.
        done: oneshot::Sender<Result<()>>,
    },
    /// End the session. Acknowledged **after** the process tree is gone, so a caller that awaited
    /// `cancel` knows there is nothing left running (§11 criterion 11).
    Cancel {
        /// How long the drain may take before the tree is killed (plan D81).
        grace: Duration,
        /// Answered last.
        done: oneshot::Sender<()>,
    },
}

/// A live CLI session: channel endpoints and handle-side bookkeeping only.
pub struct CliSession {
    /// The id `htui` minted and passed as `--session-id` (D84).
    session_ref: AgentSessionRef,
    events: mpsc::Receiver<DriverEnvelope>,
    commands: mpsc::UnboundedSender<Command>,
    /// Envelopes drained while `cancel` or `answer_permission` waited for the task's reply; served
    /// before `events`.
    pending: VecDeque<DriverEnvelope>,
    /// `false` between a handed-out `done` and the next accepted follow-up.
    turn_open: bool,
    /// The task has ended: `next_event` answers `Ok(None)`, everything else
    /// [`DriverError::Closed`].
    ended: bool,
    /// MOD-11 D18: the spec carried a prompt port, so `answer_permission` has requests to answer.
    prompts: bool,
    task: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for CliSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CliSession")
            .field("session_ref", &self.session_ref)
            .field("pending", &self.pending.len())
            .field("turn_open", &self.turn_open)
            .field("ended", &self.ended)
            .field("prompts", &self.prompts)
            .finish()
    }
}

impl AgentSession for CliSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        Some(&self.session_ref)
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            // No parked check: a prompt (MOD-11 D18) is answered through the bridge whatever the
            // reader pulls meanwhile — the CLI itself waits on the tool, not on this handle.
            if let Some(envelope) = self.pending.pop_front() {
                self.note(&envelope);
                return Ok(Some(envelope));
            }
            if self.ended {
                return Ok(None);
            }
            match self.events.recv().await {
                Some(envelope) => {
                    self.note(&envelope);
                    Ok(Some(envelope))
                }
                None => {
                    self.ended = true;
                    Ok(None)
                }
            }
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            if text.is_empty() {
                return Err(DriverError::Transport(
                    "a follow-up must have text".to_owned(),
                ));
            }
            if self.turn_open {
                return Err(DriverError::Transport(
                    "a follow-up before the turn's done would interleave two turns".to_owned(),
                ));
            }
            self.send(Command::FollowUp(text))?;
            self.turn_open = true;
            Ok(())
        })
    }

    /// MOD-11 D18: with a prompt port, completes the parked prompt `request_id` and returns once the
    /// prompt tool has its verdict (`Transport("no parked request …")` for an id that is not
    /// parked).
    ///
    /// Without one there is no permission request to answer, and the honest error says so about
    /// the **operation** rather than about the id: [`DriverError::Unsupported`] and not
    /// `Transport("no parked request …")`, because the latter claims the id is unknown, when the
    /// truth is that this session has no such channel at all — which is what `Unsupported` was
    /// added for.
    ///
    /// [`DriverError::Closed`] is checked first because the trait's contract for every operation is
    /// "`Closed` once the session has ended": a session that is over should not be arguing about an
    /// operation it never had the chance to refuse.
    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            if !self.prompts {
                return Err(DriverError::Unsupported("answer_permission"));
            }
            let (done, mut answered) = oneshot::channel();
            self.send(Command::Answer {
                request_id,
                answer,
                done,
            })?;
            // Keep draining while the task gets to the command, as `cancel` does: a CLI that keeps
            // streaming while this call waits fills the event channel, and a task blocked in
            // `emit` never reads the answer — while the caller, waiting here, pulls nothing.
            // Drained envelopes are served by `next_event` ahead of the channel, in order.
            loop {
                tokio::select! {
                    // A task that ended before it read the command dropped `done`.
                    reply = &mut answered => return reply.map_err(|_| DriverError::Closed)?,
                    event = self.events.recv(), if !self.ended => match event {
                        Some(envelope) => self.pending.push_back(envelope),
                        // The task is gone, so `done` is dropped and the arm above resolves.
                        None => self.ended = true,
                    },
                }
            }
        })
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Ok(());
            }
            let (done, mut ack) = oneshot::channel();
            if self.send(Command::Cancel { grace, done }).is_err() {
                // `send` has already drained the task's last rows into `pending` and marked the
                // session ended. A cancel of a session that is already over is `Ok`, not an error
                // — there is nothing left to stop — but the rows it wrote on its way out are still
                // owed to the recorder.
                return Ok(());
            }
            // Keep draining while the task shuts down: the drained `result`, the synthesized
            // results and the `done` are ordinary events, and dropping them here would lose rows
            // the recorder still has to write.
            loop {
                tokio::select! {
                    _ = &mut ack => break,
                    event = self.events.recv() => match event {
                        Some(envelope) => self.pending.push_back(envelope),
                        None => {
                            self.ended = true;
                            break;
                        }
                    },
                }
            }
            // **Not** `ended = true`: some of the cancel's own rows may still be in the channel
            // when the acknowledgement wins the `select!` above, and the caller pulls them exactly
            // as it pulls any other event. The task acknowledges *after* it has killed the process
            // tree and returns immediately afterwards, so joining it here is what makes "cancel
            // returned" mean "the tree is gone" — which is what §11 criterion 11 measures.
            if let Some(task) = self.task.take()
                && let Err(err) = task.await
                && !err.is_cancelled()
            {
                tracing::warn!(%err, "the session task panicked on its way out");
            }
            Ok(())
        })
    }
}

impl CliSession {
    /// Handle-side bookkeeping, applied as an envelope is handed out rather than as it arrives:
    /// what the caller has *seen* is what decides whether a follow-up is legal.
    fn note(&mut self, envelope: &DriverEnvelope) {
        if matches!(envelope.event, DriverEvent::Done(_)) {
            self.turn_open = false;
        }
    }

    /// Sends a command, turning a dead task into [`DriverError::Closed`].
    ///
    /// **The rows the task already wrote are rescued before the session is marked ended**, because
    /// a dead task is usually one that has just finished saying something: the EOF path writes
    /// `error{transport_closed}`, a synthesized `failed` result for every open call, and a
    /// `done{cancelled}` — and *then* the task returns, which is what makes this send fail.
    /// [`Self::next_event`] honours `ended` before it looks at the channel, so marking it first
    /// would answer `Ok(None)` over a queue still holding the turn's last rows, and the recorder
    /// would never write them. Draining into `pending` costs nothing and keeps the transcript
    /// complete on exactly the path where it is hardest to reconstruct.
    fn send(&mut self, command: Command) -> Result<()> {
        if self.commands.send(command).is_err() {
            self.drain_into_pending();
            self.ended = true;
            return Err(DriverError::Closed);
        }
        Ok(())
    }

    /// Moves everything the task has already queued into `pending`, which `next_event` serves
    /// ahead of the channel and ahead of `ended`.
    fn drain_into_pending(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.pending.push_back(event);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------------------------

/// Everything the session task needs besides the streams.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// `agent.name`, for the banner and the logs. The stream carries no agent identity of its own.
    pub agent_name: String,
    /// `agent.models`, the banner's fallback when `system/init` names no model.
    pub models: Vec<String>,
    /// The parsed `agent.settings` (§5.2).
    pub settings: AgentSettings,
    /// How capture times are stamped.
    pub stamp: Stamp,
    /// How long the first `system/init` may take before [`open_session`] gives up.
    ///
    /// [`INIT_TIMEOUT`] in production. A field rather than the constant read at the point of use
    /// for `acp::SessionOptions::handshake_timeout`'s reason: the arm that matters is the one that
    /// has to kill the child it gave up on, and a minute-long test is a test nobody runs.
    pub init_timeout: Duration,
    /// `agent_box.version`, the banner's second source for `agent_version`.
    pub box_version: Option<String>,
    /// The id passed as `--session-id`, or the one `--resume` continues (D84).
    pub session_id: String,
}

/// Opens a session: spawns the task, waits for `system/init`, returns the handle.
///
/// # Errors
///
/// [`DriverError::Transport`] carrying whatever went wrong, with the child's captured stderr
/// appended when there was a child. Three failing exits, and every one of them returns with the
/// kill already sent — `acp::open_session`'s three, for its reasons, with its one asymmetry:
///
/// 1. **The agent never said anything.** The task is aborted and awaited, which resolves as soon as
///    the runtime has dropped the future; the `ChildGuard`'s `Drop` has then signalled the tree.
///    Signalled but not *reaped* by us, because a `Drop` cannot await.
/// 2. **The agent ended before `system/init`** — an unauthenticated CLI prints its refusal to
///    stderr and exits, which is EOF. Composed on the task's own timeline with the stderr tail
///    attached, so the tree is killed *and* reaped before this returns.
/// 3. **Nobody answered at all**, the sender dropped with the task. Awaited exactly as (2) is.
///
/// `mcp_config` is the file the argv names (MOD-79). It moves into the task and is removed when the
/// task ends, on all three failing exits as on the session's own end.
pub async fn open_session(
    io: ChildIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    mcp_config: Option<McpConfigFile>,
) -> Result<CliSession> {
    let (events_tx, events_rx) = mpsc::channel(EVENTS_CAPACITY);
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();

    let timeout = options.init_timeout;
    // Built here and not carried back from the task: over ACP the id is the *agent's* answer to
    // `session/new` and has to travel, but `--session-id` makes it `htui`'s own (D84) — so the task
    // answers only *whether* the stream opened, and a disagreement with what `system/init` echoes
    // is a `warn!` there rather than a value here (blueprint H-17).
    let session_ref = AgentSessionRef::new(options.session_id.clone());
    let prompts = spec.prompt.is_some();
    // MOD-79 D3: the config file is the task's, beside the `ChildGuard` `run_session` makes: every
    // exit drops it, the abort in exit (1) included, and a normal end drops it only after
    // `run_session`'s final kill. Captured here rather than passed in, so `run_session` keeps its
    // seven arguments (blueprint G-1, H-12); dropped by name, because `let _ =` would drop it at
    // once (H-11).
    let task = crate::contained::spawn(async move {
        run_session(io, spec, prompt, options, ready_tx, events_tx, commands_rx).await;
        drop(mcp_config);
    });

    match tokio::time::timeout(timeout, ready_rx).await {
        Err(_) => {
            // Exit (1). The aborted task drops its `ChildGuard`, whose `Drop` signals the kill, and
            // its MCP config file (MOD-79); awaiting the cancelled handle resolves as soon as the
            // runtime has dropped the future and is what makes this `Err` mean "the kill has been
            // sent" rather than "the kill will be sent shortly". The reap is tokio's orphan queue's,
            // as `acp::open_session` explains at length.
            task.abort();
            let _ = task.await;
            Err(DriverError::Transport(format!(
                "the agent did not send its `system/init` within {}s",
                timeout.as_secs()
            )))
        }
        Ok(Ok(Ok(()))) => Ok(CliSession {
            session_ref,
            events: events_rx,
            commands: commands_tx,
            pending: VecDeque::new(),
            // The first prompt has already gone out, so turn 0 is open before the caller has the
            // handle: a follow-up now would interleave two turns.
            turn_open: true,
            ended: false,
            prompts,
            task: Some(task),
        }),
        Ok(Ok(Err(err))) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(err)
        }
        Ok(Err(_)) => {
            let _ = tokio::time::timeout(timeout, task).await;
            Err(DriverError::Transport(format!(
                "the session task ended before `system/init` for session {}",
                session_ref.as_str()
            )))
        }
    }
}

/// The session task: owns the child, the stdout reader and the mapper.
async fn run_session(
    io: ChildIo,
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: oneshot::Sender<Result<()>>,
    events: mpsc::Sender<DriverEnvelope>,
    commands: mpsc::UnboundedReceiver<Command>,
) {
    let ChildIo {
        reader,
        writer,
        child,
    } = io;
    // **The child is owned here**, by the task that outlives the `start` which created it, and
    // through a `ChildGuard` rather than a bare `Spawned` for the one exit that runs no code of
    // ours: an **aborted** task drops the guard, whose `Drop` signals the kill, which is what
    // [`open_session`]'s timeout arm relies on. `acp::run_session`'s shape, copied.
    let child = Arc::new(Mutex::new(ChildGuard::new(child)));

    // **The stdout reader is its own task**, and that is not decoration. `read_until` is not
    // cancellation-safe: driven directly from the `select!` below it would lose a partly-read line
    // every time a command won the race, which on this wire means losing a turn's `result`. A
    // channel receive *is* cancellation-safe, so the line splitting happens over there and the
    // supervisor only ever selects over whole lines.
    let (lines_tx, lines_rx) = mpsc::channel(EVENTS_CAPACITY);
    let reading = crate::contained::spawn(read_lines(reader, lines_tx));

    // MOD-11 D18 (H-17): the port's receiver is taken once, here; a clone of the spec gets none.
    let mut prompts = Prompts::new(spec.prompt.as_ref().and_then(PromptPort::take));
    session_main(
        spec,
        prompt,
        options,
        ready,
        events,
        commands,
        lines_rx,
        writer,
        &child,
        &mut prompts,
    )
    .await;
    // Whatever is still asking gets an answer rather than the prompt tool's timeout.
    // Nothing is mapped after this point, so the denied ids need no marking.
    let _ = prompts.deny_all(ENDED);

    // A spawned child's stdout ends when the kill below closes it, but a prepared pair's writer is
    // held by whoever built it and may never close: the reader is aborted rather than left waiting
    // on a stream this session has stopped caring about.
    reading.abort();
    // Unconditional, and a no-op after the cancel path's own kill.
    kill(&child).await;
}

/// Splits the agent's stdout into lines and forwards them, whatever bytes it finds.
///
/// `read_until` + `from_utf8_lossy` and **not** `lines()`, which is the stderr reader's rule
/// (`launch.rs`) for a sharper reason here (blueprint H-23): `lines()` answers `Err(InvalidData)`
/// on a single byte that is not UTF-8 and then **ends**, so one stray byte anywhere in a transcript
/// would end the stream mid-turn and be recorded as a transport that closed. The `\r` a Windows
/// child writes before its `\n` is stripped for the same reason — a CRLF line that reached the JSON
/// parser with its carriage return still attached would parse, and then the *last* line of the
/// stream would not.
///
/// **The line is capped** (review gate, LOW). `read_until` grows its buffer until it finds the
/// delimiter, so a child that writes megabytes without a newline — a crash dump, a binary blob, a
/// subprocess whose output got redirected into ours — would grow this buffer without limit inside
/// a supervisor whose whole job is to survive the agent misbehaving. At the cap the partial line is
/// forwarded as it stands and the reader keeps going, so the transcript records what arrived (it
/// lands in `other` as unparsed, §6.2's rule) rather than either truncating in silence or holding
/// the whole thing in memory. No recorded transcript comes near it: the largest real line in the
/// fixtures is a `system/init` of a few kilobytes.
async fn read_lines(reader: Box<dyn AsyncRead + Send + Unpin>, lines: mpsc::Sender<String>) {
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_until(b'\n', &mut buffer)
            .await
        {
            // End of stream.
            Ok(0) => break,
            Ok(_) => {}
            // A real I/O failure on the pipe: there is nothing left to read from a descriptor that
            // answers an error, and the supervisor reads the close as an EOF either way.
            Err(_) => break,
        }
        if buffer.last() == Some(&b'\n') {
            buffer.pop();
            if buffer.last() == Some(&b'\r') {
                buffer.pop();
            }
        }
        let line = String::from_utf8_lossy(&buffer).into_owned();
        // A blank line is not an envelope and carries nothing; forwarding it would make every
        // drain and every end-of-turn check step over it.
        if line.trim().is_empty() {
            continue;
        }
        if lines.send(line).await.is_err() {
            break;
        }
    }
}

/// One stdout line, classified once so the supervisor's three readers agree about it.
enum Line {
    /// A JSON envelope, for the mapper.
    Json(Value),
    /// Anything else, kept verbatim under [`UNPARSED`].
    Text(String),
}

/// A line as one of the two shapes that can arrive.
fn classify(text: String) -> Line {
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => Line::Json(value),
        Err(_) => Line::Text(text),
    }
}

/// Whether a line is the `system/init` that opens the stream (§6.2).
fn is_init(line: &Value) -> bool {
    claude::kind_of(line) == ("system", Some("init"))
}

/// The task's own state machine: the prompt, the banner, then the turn loop.
#[expect(
    clippy::too_many_arguments,
    reason = "the task owns ten distinct things; bundling them into a struct renames the arity \
              without reducing it"
)]
async fn session_main(
    spec: SessionSpec,
    prompt: String,
    options: SessionOptions,
    ready: oneshot::Sender<Result<()>>,
    events: mpsc::Sender<DriverEnvelope>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut lines: mpsc::Receiver<String>,
    writer: Box<dyn AsyncWrite + Send + Unpin>,
    child: &Mutex<ChildGuard>,
    prompts: &mut Prompts,
) {
    let mut state = TaskState::new(options.stamp, spec.retain_raw, options.settings.usage.scope);
    let session_ref = AgentSessionRef::new(options.session_id.clone());
    // An `Option` because a cancel **takes** it: closing stdin is the documented end of input for
    // `--input-format stream-json`, and the only way to close it is to drop it (plan D81 step 1).
    let mut writer = Some(writer);

    // 1. The prompt goes out first, on stdin, never on the argv (blueprint P-2). The turn is
    //    already open at this point in the sense that matters — a failure here still owes the
    //    caller an answer, and it gets one through `ready` rather than through a `done`, because
    //    the handle does not exist yet.
    if let Some(sink) = writer.as_mut()
        && let Err(err) = write_line(sink, &stdin_line(&prompt)).await
    {
        let _ = ready.send(Err(with_stderr(&err.to_string(), child)));
        kill(child).await;
        return;
    }

    // 2. Wait for `system/init`, buffering everything that arrives first.
    //
    //    **The buffer is required, and F-9 is why.** Without `--bare` — which D92 dropped from the
    //    seed because it refuses to read an OAuth login at all — three `system/hook_started` and
    //    three `system/hook_response` envelopes arrive *before* `init`, and in the recorded run
    //    before the first stdin line had even been written. Mapping them as they arrived would put
    //    an `other` row in front of the banner, and "the session banner is the step's first `other`
    //    row" is a conformance case (§4.4, D84). So they wait here and are released in arrival
    //    order behind the banner.
    let mut pre_init: Vec<Line> = Vec::new();
    let init = loop {
        match lines.recv().await {
            Some(text) => match classify(text) {
                Line::Json(value) if is_init(&value) => break value,
                line => pre_init.push(line),
            },
            // The stream ended without an `init`. This is where an unauthenticated box's refusal
            // surfaces: the CLI prints it to stderr and exits, so the stderr tail is the whole of
            // what the user needs to read.
            None => {
                let _ = ready.send(Err(with_stderr(
                    "the agent ended before sending `system/init`",
                    child,
                )));
                kill(child).await;
                return;
            }
        }
    };

    // 3. The banner, first row of the step (D84).
    if let Some(reported) = init.get("session_id").and_then(Value::as_str)
        && reported != session_ref.as_str()
    {
        // Blueprint H-17: the id `htui` minted is the one `--resume` will take, so the banner and
        // `session_ref` carry ours whatever the CLI echoes. A disagreement is worth a log line and
        // is not worth failing a session over.
        tracing::warn!(
            minted = %session_ref,
            reported = %reported,
            "the CLI reported a session id other than the one it was given"
        );
    }
    let banner = DriverEvent::Other(OtherEvent {
        update: SESSION_STARTED.to_owned(),
        body: json!({
            "session_id": session_ref.as_str(),
            // The stream negotiates nothing, so there is no version to report — and reporting a
            // `1` copied from ACP would be a claim about a handshake that never happened.
            "protocol_version": Value::Null,
            // The row's name: the stream carries no agent identity of its own.
            "agent_name": options.agent_name,
            "agent_version": agent_version(&init, options.box_version.as_deref()),
            "models": banner_models(&init, &options.models),
        }),
    });
    let raw = state.retain_raw.then(|| init.clone());
    if !emit(&mut state, &events, banner, raw).await {
        kill(child).await;
        return;
    }
    for line in pre_init {
        if !on_line(&mut state, &events, line).await {
            kill(child).await;
            return;
        }
    }

    // 4. The handle may exist now: everything above is what `start` promised to have done.
    if ready.send(Ok(())).is_err() {
        kill(child).await;
        return;
    }

    // 5. The turn loop. Every arm is a cancellation-safe channel receive.
    loop {
        let step = tokio::select! {
            line = lines.recv() => Step::Line(line),
            command = commands.recv() => Step::Command(command),
            request = prompts.next() => Step::Prompt(request),
        };
        match step {
            Step::Line(Some(text)) => {
                if !on_line(&mut state, &events, classify(text)).await {
                    break;
                }
            }
            // EOF. **Between turns this is a clean end** — F-1 measured it: the CLI runs a turn to
            // completion and exits 0 after its stdin closes, so a stream that stops when nothing is
            // open is a session that finished. **Mid-turn it is the milestone-3 defect class**
            // (`682a423`, "a stream ending before its `done` was recorded as a finished turn"), and
            // it is closed the same way `acp`'s `Step::Closed` closes it: an `error` naming the
            // transport, every open call given its synthesized `failed`, and exactly one
            // `done { cancelled }` — never a silent `Ok(None)`.
            Step::Line(None) => {
                if state.turn_open {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: stderr_tail(child),
                    });
                    emit(&mut state, &events, event, None).await;
                    close_turn(&mut state, &events, StopReason::Cancelled).await;
                }
                break;
            }
            Step::Command(Some(Command::FollowUp(text))) => {
                // Open the turn **before** the write: the handle already counts it as open, and a
                // failure has to be able to close it (`close_turn` is a no-op on a closed turn).
                state.turn_open = true;
                let sent = match writer.as_mut() {
                    Some(sink) => write_line(sink, &stdin_line(&text)).await,
                    // Only a cancel takes the writer, and a cancel returns from this loop — so this
                    // is unreachable, and a named error rather than a panic if it stops being.
                    None => Err(DriverError::Closed),
                };
                if let Err(err) = sent {
                    let event = DriverEvent::Error(ErrorEvent {
                        code: TRANSPORT_CLOSED.to_owned(),
                        message: err.to_string(),
                    });
                    emit(&mut state, &events, event, None).await;
                    close_turn(&mut state, &events, StopReason::Cancelled).await;
                    break;
                }
            }
            // MOD-11 D18: a gated call, announced as an ordinary request.
            Step::Prompt(Some(request)) => {
                // The prompt may have overtaken its `tool_use` line: the call goes first, so the
                // request's consumers can see what it gates (adversarial review T9-ADV-ORDER-1).
                if let Some(call) = state.mapper.prompted_call(&request.call)
                    && !emit(&mut state, &events, call, None).await
                {
                    break;
                }
                let event = DriverEvent::PermissionRequest(prompts.park(request));
                if !emit(&mut state, &events, event, None).await {
                    break;
                }
            }
            // Every asking end is gone; nothing more can arrive.
            Step::Prompt(None) => prompts.closed(),
            Step::Command(Some(Command::Answer {
                request_id,
                answer,
                done,
            })) => {
                let answered = prompts.answer(&request_id, answer);
                if answered.is_ok() {
                    // D18 dedup: the result's `permission_denials[]` repeats a denied call; the
                    // answer already settled it.
                    state.mapper.mark_answered(request_id.as_str());
                }
                let _ = done.send(answered);
            }
            Step::Command(Some(Command::Cancel { grace, done })) => {
                // The CLI is blocked on the prompt tool: answer it before the interrupt.
                for request_id in prompts.deny_all(CANCELLED) {
                    // D18 dedup, as in the `Answer` arm: the interrupt's `result` repeats the
                    // denied call in `permission_denials[]`.
                    state.mapper.mark_answered(request_id.as_str());
                }
                cancel_session(&mut state, &events, &mut writer, &mut lines, child, grace).await;
                kill(child).await;
                // Last, so a caller that awaited `cancel` knows the tree is gone (criterion 11).
                let _ = done.send(());
                return;
            }
            // The handle is gone: nobody is reading, so end the session rather than leave a child
            // running for an audience that left.
            Step::Command(None) => {
                for request_id in prompts.deny_all(ENDED) {
                    state.mapper.mark_answered(request_id.as_str());
                }
                cancel_session(
                    &mut state,
                    &events,
                    &mut writer,
                    &mut lines,
                    child,
                    DROP_GRACE,
                )
                .await;
                break;
            }
        }
    }

    kill(child).await;
}

/// One iteration's cause.
enum Step {
    /// A stdout line, or the end of them.
    Line(Option<String>),
    /// A command from the handle, or the handle's disappearance.
    Command(Option<Command>),
    /// A prompt from `htui`'s `permission_prompt` tool, or the end of them (MOD-11 D18).
    Prompt(Option<PromptRequest>),
}

/// The option id a prompt's allow carries.
const ALLOW_OPTION: &str = "allow";

/// The option id a prompt's reject carries.
const REJECT_OPTION: &str = "reject";

/// What the agent reads when the person driving the session rejected the call.
const DENIED: &str = "denied in htui";

/// What the agent reads when the session was cancelled with the call still asking.
const CANCELLED: &str = "the session was cancelled";

/// What the agent reads when the session ended with the call still asking.
const ENDED: &str = "the session ended";

/// MOD-11 D18: the session end of the permission bridge — the port's receiver, and every prompt
/// announced and not yet answered.
struct Prompts {
    /// `None` without a port, or once every asking end is gone.
    rx: Option<mpsc::Receiver<PromptRequest>>,
    /// Announced prompts, by request id, each waiting for its verdict.
    pending: HashMap<PermissionRequestId, oneshot::Sender<PromptVerdict>>,
    /// Prompts received so far: the `prompt-<n>` of one that names no tool-use id.
    received: u64,
}

impl Prompts {
    fn new(rx: Option<mpsc::Receiver<PromptRequest>>) -> Self {
        Self {
            rx,
            pending: HashMap::new(),
            received: 0,
        }
    }

    /// The next prompt; never resolves without a port, so the turn loop's arm simply idles.
    async fn next(&mut self) -> Option<PromptRequest> {
        match self.rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => std::future::pending().await,
        }
    }

    /// The asking ends are all gone: stop polling the receiver.
    fn closed(&mut self) {
        self.rx = None;
    }

    /// Parks `request` and answers the event that announces it.
    fn park(&mut self, request: PromptRequest) -> PermissionRequestEvent {
        let n = self.received;
        self.received += 1;
        let PromptRequest { call, answer } = request;
        let request_id = PermissionRequestId::new(
            call.tool_use_id
                .clone()
                .unwrap_or_else(|| format!("prompt-{n}")),
        );
        self.pending.insert(request_id.clone(), answer);
        PermissionRequestEvent {
            request_id,
            tool_call_id: call.tool_use_id,
            options: vec![
                PermissionOption {
                    id: ALLOW_OPTION.to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                },
                PermissionOption {
                    id: REJECT_OPTION.to_owned(),
                    label: "Reject".to_owned(),
                    kind: PermissionOptionKind::RejectOnce,
                },
            ],
        }
    }

    /// Completes the parked prompt `request_id` with `answer`.
    ///
    /// # Errors
    /// [`DriverError::Transport`] for an id that is not parked, or an option this transport never
    /// offered (the prompt stays parked).
    fn answer(&mut self, request_id: &PermissionRequestId, answer: PermissionAnswer) -> Result<()> {
        let verdict = match &answer {
            PermissionAnswer::Selected(option) if option == ALLOW_OPTION => PromptVerdict::Allow,
            PermissionAnswer::Selected(option) if option == REJECT_OPTION => PromptVerdict::Deny {
                message: DENIED.to_owned(),
            },
            PermissionAnswer::Selected(option) => {
                return Err(DriverError::Transport(format!(
                    "permission request `{request_id}` offers no option `{option}`"
                )));
            }
            PermissionAnswer::Cancelled => PromptVerdict::Deny {
                message: CANCELLED.to_owned(),
            },
        };
        let sender = self.pending.remove(request_id).ok_or_else(|| {
            DriverError::Transport(format!("no parked permission request `{request_id}`"))
        })?;
        // The asker may have gone (the CLI's tool timeout); the answer is still the session's.
        let _ = sender.send(verdict);
        Ok(())
    }

    /// Denies every parked prompt and every one still queued, with `message`, and closes the port.
    ///
    /// Answers the ids of the **parked** ones: each was announced, so its consumer records its
    /// `cancelled` answer itself (the relay's `answer_cancelled`, chat's `Cancel` arm), and the
    /// session marks it answered exactly as the `Answer` arm does (D18 dedup). A queued one was
    /// never announced; a `permission_denials[]` entry is the only row it can get.
    fn deny_all(&mut self, message: &str) -> Vec<PermissionRequestId> {
        let deny = || PromptVerdict::Deny {
            message: message.to_owned(),
        };
        let mut denied = Vec::with_capacity(self.pending.len());
        for (request_id, sender) in self.pending.drain() {
            let _ = sender.send(deny());
            denied.push(request_id);
        }
        if let Some(mut rx) = self.rx.take() {
            rx.close();
            while let Ok(request) = rx.try_recv() {
                let _ = request.answer.send(deny());
            }
        }
        denied
    }
}

/// The cancel sequence of plan D81, in the order **F-1, F-2 and F-3 measured** rather than the one
/// `docs/ANA-4.md` §4.4 hypothesised.
///
/// 1. **Close stdin.** The documented end of input for `--input-format stream-json` — and, F-1:
///    *not* a cancel. The CLI runs the open turn to completion and exits 0. So this step is what
///    stops the *next* turn, not this one.
/// 2. **`SIGINT` to the group.** F-2: this is the step that buys a terminal envelope. The CLI
///    answers with a real `result` — an error-shaped one (`subtype: "error_during_execution"`,
///    `is_error: true`) carrying `terminal_reason: "aborted_streaming"`, exit 0 — which the mapper
///    turns into `done { cancelled }` by reading that key rather than by remembering that *we* sent
///    the signal. On Windows there are no signals and `Spawned::signal` says so; that is a step
///    this platform does not have, not a failure, so the sequence goes on to the grace and the
///    kill, which is exactly what the Windows cancel has always been.
/// 3. **Drain for the grace window.** Whatever arrives is emitted in order, and a `done` that
///    arrives is the turn's **real** ending. This ordering is the other half of the milestone-3
///    defect: nothing is synthesized until the read side is exhausted or the deadline passes, so a
///    `result` still in the pipe can never be overtaken by a `done` `htui` invented.
/// 4. **Close the turn if it is still open**, and only then. Cancelling between turns ends the
///    session rather than a turn, so this is a no-op there — and a no-op after a drained `result`,
///    which is what keeps the cancel from writing a second `done`.
///
/// An EOF during the drain writes **no** `error { transport_closed }`: the cancel is the cause of
/// this stream ending, and the `done` below already says so. F-3 is the shape that reaches here —
/// SIGTERM (or a kill) leaves exit 143 and no `result` at all.
async fn cancel_session(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    writer: &mut Option<Box<dyn AsyncWrite + Send + Unpin>>,
    lines: &mut mpsc::Receiver<String>,
    child: &Mutex<ChildGuard>,
    grace: Duration,
) {
    if let Some(mut sink) = writer.take() {
        let _ = sink.shutdown().await;
    }
    interrupt(child);

    if state.turn_open && grace > Duration::ZERO {
        let deadline = tokio::time::sleep(grace);
        tokio::pin!(deadline);
        loop {
            let line = tokio::select! {
                () = &mut deadline => break,
                line = lines.recv() => line,
            };
            let Some(text) = line else { break };
            if !on_line(state, events, classify(text)).await {
                break;
            }
            // The drained `result` closed the turn: its `done` is the real one and there is nothing
            // left to wait for.
            if !state.turn_open {
                break;
            }
        }
    }

    close_turn(state, events, StopReason::Cancelled).await;
}

/// Asks the child's process **group** to stop, and treats a platform that cannot as a step it does
/// not have.
///
/// The group and not the pid: `process-wrap` makes the child a group leader at spawn, so this is a
/// `killpg` that reaches the helpers a CLI agent spawned for itself. A supervisor that signalled
/// only the pid it can name would leave the rest of the tree holding the pipe.
///
/// No lock is held across an await because nothing here awaits: `Spawned::signal` sends and
/// returns, and what the child does with the request is the child's business.
fn interrupt(child: &Mutex<ChildGuard>) {
    let mut guard = child.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(spawned) = guard.child_mut() else {
        return;
    };
    if let Err(err) = spawned.signal(StopSignal::Interrupt) {
        // Windows has no SIGINT and says so, and a group that has already exited answers `ESRCH`.
        // Neither is a reason to stop cancelling: the grace window and the tree kill follow either
        // way, and that pair *is* the Windows cancel.
        tracing::debug!(%err, "the interrupt step of the cancel was unavailable; going on");
    }
}

/// Task-side state, one per session.
struct TaskState {
    stamp: Stamp,
    n: u64,
    retain_raw: bool,
    mapper: claude::Mapper,
    /// Calls announced and not yet settled: what a cancel or an EOF owes a synthesized result.
    open_calls: Vec<String>,
    settled_calls: BTreeSet<String>,
    turn_open: bool,
}

impl TaskState {
    fn new(stamp: Stamp, retain_raw: bool, scope: crate::launch::UsageScope) -> Self {
        Self {
            stamp,
            n: 0,
            retain_raw,
            mapper: claude::Mapper::new(scope),
            open_calls: Vec::new(),
            settled_calls: BTreeSet::new(),
            // The first prompt goes out before this state is used for anything: turn 0 is open.
            turn_open: true,
        }
    }

    /// Wraps an event in its envelope, stamping the capture time.
    ///
    /// With `retain_raw` set, a row that has **no** wire line still carries a `raw` naming what
    /// produced it — the banner, the synthesized results of a cancel, and every `error` `htui`
    /// authored itself are rows a replay has to explain (§11 criterion 4). `acp::TaskState`'s rule,
    /// and the same synthesized shape so the two transports replay alike.
    fn envelope(&mut self, event: DriverEvent, raw: Option<Value>) -> DriverEnvelope {
        let at = self.stamp.at(self.n);
        self.n += 1;
        let raw = if self.retain_raw {
            Some(raw.unwrap_or_else(|| {
                json!({
                    "htui_synthesized": htui_core::model::EventKind::from(&event).as_str(),
                    "n": self.n - 1,
                })
            }))
        } else {
            None
        };
        DriverEnvelope { event, raw, at }
    }

    /// Notes what an outgoing event means for the call bookkeeping of §4.3.
    fn note(&mut self, event: &DriverEvent) {
        match event {
            DriverEvent::ToolCall(call) => {
                if !self.settled_calls.contains(&call.tool_call_id) {
                    self.open_calls.push(call.tool_call_id.clone());
                }
            }
            DriverEvent::ToolResult(result) => {
                self.settled_calls.insert(result.tool_call_id.clone());
                self.open_calls.retain(|id| id != &result.tool_call_id);
            }
            _ => {}
        }
    }
}

/// Sends one event, returning `false` once the consumer is gone.
///
/// `acp::emit`'s rules minus the one that cannot apply: this transport reports no `edit_proposal`
/// (§4.3 fixes `edit_proposals: false`), so there is no `accepted` to fill in.
async fn emit(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    event: DriverEvent,
    raw: Option<Value>,
) -> bool {
    // A `tool_result` for a call a cancel already settled is the stream's own late report: `htui`
    // wrote the synthesized row, and a second one would be two results for one call (§4.3
    // "Tool-call terminal states").
    if let DriverEvent::ToolResult(result) = &event
        && state.settled_calls.contains(&result.tool_call_id)
    {
        return true;
    }
    state.note(&event);
    let envelope = state.envelope(event, raw);
    events.send(envelope).await.is_ok()
}

/// Maps one classified line and emits whatever it produced.
async fn on_line(state: &mut TaskState, events: &mpsc::Sender<DriverEnvelope>, line: Line) -> bool {
    match line {
        Line::Json(value) => {
            // Cloned only when the project asked for it: `retain_raw` off means a transport does
            // not even allocate the verbatim value (`SessionSpec::retain_raw`).
            let raw = state.retain_raw.then(|| value.clone());
            for event in state.mapper.map(&value) {
                let ends_the_turn = matches!(event, DriverEvent::Done(_));
                if !emit(state, events, event, raw.clone()).await {
                    return false;
                }
                // The stream said the turn is over, so nothing after this may synthesize a second
                // `done` for it — not the EOF arm, and not a cancel's.
                if ends_the_turn {
                    state.turn_open = false;
                }
            }
            true
        }
        Line::Text(text) => {
            let event = DriverEvent::Other(OtherEvent {
                update: UNPARSED.to_owned(),
                body: json!({ "line": text }),
            });
            emit(state, events, event, None).await
        }
    }
}

/// Closes the open turn: every call still open gets its synthesized result, then exactly one
/// `done`.
///
/// A no-op when the turn is already closed, which is what makes it safe to call from the EOF arm,
/// the failed-write arm and the cancel alike — and what keeps a cancel that drained a real `result`
/// from writing a second ending for one turn.
async fn close_turn(
    state: &mut TaskState,
    events: &mpsc::Sender<DriverEnvelope>,
    stop_reason: StopReason,
) -> bool {
    if !state.turn_open {
        return true;
    }
    if stop_reason == StopReason::Cancelled {
        let open: Vec<String> = state.open_calls.clone();
        for tool_call_id in open {
            let event = DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id,
                status: ToolResultStatus::Failed,
                output: None,
                locations: Vec::new(),
                terminal_reason: Some(TerminalReason::Cancelled),
            });
            if !emit(state, events, event, None).await {
                return false;
            }
        }
    }
    state.turn_open = false;
    emit(
        state,
        events,
        DriverEvent::Done(DoneEvent { stop_reason }),
        None,
    )
    .await
}

/// Writes one NDJSON line to the child's stdin and flushes it.
///
/// The flush matters: the CLI reads a line at a time, and a message sitting in a buffer is a turn
/// that never starts. A pathological prompt is bounded by this `await` on the task's own timeline
/// and never on the UI's (`R-NF-3`), and the stdout reader keeps draining throughout, so the child
/// cannot deadlock on its own output while this waits (blueprint H-14).
///
/// # Errors
/// [`DriverError::Transport`] naming the I/O failure.
async fn write_line(writer: &mut (dyn AsyncWrite + Send + Unpin), line: &str) -> Result<()> {
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|err| DriverError::Transport(format!("writing to the agent's stdin: {err}")))?;
    writer
        .flush()
        .await
        .map_err(|err| DriverError::Transport(format!("flushing the agent's stdin: {err}")))
}

/// The banner's `agent_version`, from the three sources in the order D84 ranks them.
///
/// `system/init.claude_code_version` is the key the recorded transcripts actually carry — there is
/// no `version` key on that envelope, which is the sort of thing only a fixture can settle. Then
/// the probe's own `claude --version` capture, then the empty string, which is `acp`'s fallback and
/// is a banner that says "unknown" rather than one that is missing a documented key.
fn agent_version(init: &Value, box_version: Option<&str>) -> String {
    init.get("claude_code_version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| box_version.map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The banner's `models`: what the stream says it is running, else what the row offers.
///
/// D84 said "the configured row"; `system/init.model` is the model this process actually selected,
/// and the stream's own answer is the more honest one — recorded as a clarification of D84 rather
/// than a reversal, with the row as the fallback it always was.
fn banner_models(init: &Value, row_models: &[String]) -> Vec<String> {
    init.get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .map(|model| vec![model.to_owned()])
        .unwrap_or_else(|| row_models.to_vec())
}

/// Kills the process tree and reaps it, if there is one and nobody has yet.
///
/// `ChildGuard::kill_and_reap` takes `&mut self` and awaits, so the guard is **swapped out** for an
/// empty one under the lock and awaited outside it: no lock is ever held across an `.await`, and
/// the emptied guard left behind makes a second call — and the `Drop` that eventually runs — a
/// no-op rather than a second signal at a pid the operating system may already have reissued.
async fn kill(child: &Mutex<ChildGuard>) {
    let mut taken = std::mem::replace(
        &mut *child.lock().unwrap_or_else(PoisonError::into_inner),
        ChildGuard::new(None),
    );
    taken.kill_and_reap().await;
}

/// What the child last wrote to stderr, for an error message.
fn stderr_tail(child: &Mutex<ChildGuard>) -> String {
    child
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stderr_tail()
        .join("\n")
}

/// A failure with the child's captured stderr appended when there is any.
///
/// The CLI says why it will not run on stderr and nowhere else — "Not logged in · Please run
/// /login" is the whole of what an unauthenticated box gets — so a start failure that dropped it
/// would be the difference between a box you can fix and one that looks broken.
fn with_stderr(message: &str, child: &Mutex<ChildGuard>) -> DriverError {
    let stderr = stderr_tail(child);
    if stderr.is_empty() {
        DriverError::Transport(message.to_owned())
    } else {
        DriverError::Transport(format!("{message}\n{stderr}"))
    }
}

#[cfg(test)]
mod tests {
    use htui_core::model::{AgentId, Billing, StepId, Transport};

    use super::*;
    use crate::driver::PermissionPolicy;

    /// MOD-79 review L1: `io`'s failed-spawn arm drops the config it wrote before the spawn (H-13).
    ///
    /// A unit test, through [`CliDriver::io_in`], because that is the seam that reaches the spawn
    /// path with a base the test owns: the production base (`$XDG_RUNTIME_DIR` or the temp
    /// directory) is shared with every other test in the process, so "no `htui-cli-*` left behind"
    /// could only be asserted there by racing them. The command does not exist, so
    /// `launch::spawn` fails at its lookup, which is after the write; the error being that lookup's
    /// is what shows the config was reached.
    #[tokio::test]
    async fn a_failed_spawn_leaves_no_mcp_config_behind() {
        let tmp = tempfile::tempdir().expect("a scratch directory");
        let base = tmp.path().join("base");
        std::fs::create_dir(&base).expect("the stand-in base");
        let now = chrono::Utc::now();
        let row = AgentRow {
            id: AgentId::new(),
            name: "missing-cli".to_owned(),
            transport: Transport::Cli,
            launch: json!({
                "command": tmp.path().join("no-such-cli").to_string_lossy(),
                "args": [],
                "env": {},
                "discovery": { "handshake": false, "tools": {} },
            }),
            models: Vec::new(),
            default_model: None,
            billing: Billing::Subscription,
            enabled: true,
            settings: json!({ "cli": { "stream": STREAM } }),
            created_at: now,
            updated_at: now,
        };
        let driver = CliDriver::from_row(&row, DriverCaps::default()).expect("the row parses");
        let spec = SessionSpec {
            agent_id: AgentId::new(),
            step_id: StepId::new(),
            cwd: tmp.path().to_path_buf(),
            extra_dirs: Vec::new(),
            env: BTreeMap::new(),
            model: None,
            tools: ToolExposure::default(),
            mcp: vec![McpServerSpec {
                name: "htui".to_owned(),
                command: "/abs/htui".to_owned(),
                args: vec!["mcp".to_owned()],
                env: BTreeMap::from([("HTUI_MCP_TOKEN".to_owned(), "token-value".to_owned())]),
            }],
            permission: PermissionPolicy::default(),
            retain_raw: false,
            resume: None,
            budget_micros: None,
            prompt: None,
        };

        let Err(err) = driver.io_in(&spec, "session-id", Some(base.clone())).await else {
            panic!("a command that does not exist does not spawn");
        };
        match &err {
            DriverError::Spawn(message) => assert!(
                message.contains("is not executable"),
                "the spawn's lookup failed, after the config was written: {message}"
            ),
            other => panic!("expected the spawn's error, got {other:?}"),
        }
        let left: Vec<_> = std::fs::read_dir(&base)
            .expect("the base is readable")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }
}
