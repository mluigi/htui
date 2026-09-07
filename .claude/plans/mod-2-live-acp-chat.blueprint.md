# Blueprint: MOD-2 milestone 3 — live `claude` over ACP and the chat tab

**Plan**: `.claude/plans/mod-2-live-acp-chat.plan.md` (D17–D31, T11–T14, V16–V32 binding).
**Design authority**: `docs/ANA-4.md` — where it and the plan disagree, ANA-4 wins and the
disagreement is named at the point it bites (marked **ANA-4 ≠ plan**). Facts about the tree and the
SDK carry `file:line`; `registry` = `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`,
`SDK` = `registry/agent-client-protocol-2.1.0/src`, `SCHEMA` = `registry/agent-client-protocol-schema-1.7.0/src/v1`.
Anything not verified in source is marked **UNVERIFIED — implementer must check** and is never
invented.

Conventions this milestone inherits unchanged: `#![warn(missing_docs)]` on every lib, `unsafe_code =
"forbid"`, one `thiserror` enum per crate, hand-written `Debug` for anything holding an env map
(`crates/htui-agent/src/driver.rs:293-306` `RedactedEnv`), no lock across an `.await`, `DriverFuture<'a, T>`
as the seam's only future shape (`driver.rs:36`), `wire_enum!` for closed JSON vocabularies
(`crates/htui-agent/src/lib.rs:26-64`).

---

## A. Module map (build order)

| # | File | Action | Task | Responsibility (one line) |
|---|---|---|---|---|
| 1 | `Cargo.toml` (workspace) | UPDATE | T11 | `tokio` workspace features gain `fs` (line 17-18 today: `rt-multi-thread, sync, macros, time, process, io-util`). |
| 2 | `crates/htui-agent/Cargo.toml` | UPDATE | T11 | `similar = { workspace = true }`; `tokio` features add `fs`. No `futures`: every byte-stream type is held as `tokio::io::{AsyncRead, AsyncWrite}` and adapted at the one `ByteStreams::new` call through `tokio_util::compat` (already a dep, `launch.rs:23`). |
| 3 | `crates/htui-agent/src/tools.rs` | CREATE | T11 | D25 `ToolMap` resolver: `Path` via `$HTUI_TOOL_<NAME>` then `which`; `NodePackage` via env, `<cwd>/node_modules`, `npm root -g`; `Glob` → `Unresolved`. |
| 4 | `crates/htui-agent/src/permission.rs` | CREATE | T11 | ANA-4 §4.3 stages 1–2 as a pure function over `PermissionPolicy` + the gated `ToolCallEvent` + the offered options. **File-set addition** (D17 lists no home for it; see H-5). |
| 5 | `crates/htui-agent/src/acp/map.rs` | CREATE | T11 | §6.1 as code: `Mapper::map(&Value) -> Vec<DriverEvent>` over the raw `session/update` params, wildcard into `Other`, `user_message_chunk` dropped, cost-delta state. Nothing else. |
| 6 | `crates/htui-agent/src/acp/fs.rs` | CREATE | T11 | Path guard (`cwd` + `extra_dirs`), current-text read, `similar` unified diff, the write. Pure async fs; no channel, no SDK type. |
| 7 | `crates/htui-agent/src/acp/client.rs` | CREATE | T11 | The client capability block, the `PermissionRequestEvent` decoder, the three `on_receive_request` handler bodies (forward-only), the `Inbound` enum. **No** `on_receive_notification` (H-1). |
| 8 | `crates/htui-agent/src/acp/mod.rs` | CREATE | T11 | `AcpDriver`, `AcpAdapter`, `AcpSession`, `AcpIo`, `Stamp`, `SessionCommand`, the session task (`run_session`) and the ordering rule doc. |
| 9 | `crates/htui-agent/src/registry.rs` | UPDATE | T11 | `DriverFactory::with_acp()` registering `"acp"` → `AcpAdapter`. |
| 10 | `crates/htui-agent/src/lib.rs` | UPDATE | T11 | `pub mod acp; pub mod permission; pub mod tools;` + re-exports. |
| 11 | `crates/htui-agent/src/conformance.rs` | UPDATE | T12 | Two scripts made transport-representable (F-3). **ANA-4 ≠ plan**: the plan's T12 file set is `tests/**` only; ANA-4 §7 and §4.3 make two cases unpassable on ACP as written (H-2). |
| 12 | `crates/htui-agent/tests/acp_conformance.rs` | CREATE | T12 | `AcpHarness` over `tokio::io::duplex` with the raw-JSON-RPC scripted agent; `CASES.len() == 13`; `run_all`. |
| 13 | `crates/htui-agent/tests/acp_map.rs`, `tests/fixtures/claude_acp_{handshake,turn}.jsonl`, `tests/snapshots/acp_map__*.snap` | CREATE | T12 | Recorded transcript → `Mapper` → `insta` (ANA-4 §8 strategy 3). |
| 14 | `crates/htui-agent/tests/acp_live.rs` | CREATE | T12 | `#[ignore]` handshake smoke over the seeded `claude` row. |
| 15 | `crates/htui-core/src/store/mem.rs` | UPDATE | T13 | `MemStore::this_user()` — the read `mem.rs:45-47` already reserves for MOD-2 (`#[expect(dead_code, reason = "loaded now, read by MOD-2 …")]`). **File-set addition** (H-5). |
| 16 | `crates/htui-store/src/writer.rs` | CREATE | T13 | D26 `Writer { Memory, Online }`, `impl ReadStore + WriteStore` by delegation. |
| 17 | `crates/htui-store/src/backend.rs`, `src/lib.rs` | UPDATE | T13 | `Backend::writer()`, `Backend::this_user()`, module-doc amendment (`backend.rs:6-10`), `pub use writer::Writer`. |
| 18 | `crates/htui-store/tests/pg_criteria.rs` | UPDATE | T13 | `writer()` / `this_user()` over the live server. |
| 19 | `crates/htui/Cargo.toml` | UPDATE | T13 | `htui-agent = { workspace = true }`; dev-dep `htui-agent = { workspace = true, features = ["test-support"] }`. |
| 20 | `crates/htui/src/store_worker.rs` | UPDATE | T13 | D27 four requests, `name()` arms, `ChatFrame`, `StoreReply::{Chat, ChatAccepted}`, the served-ahead arm, `spawn_with(.., AgentRuntime)`, shutdown on channel close. |
| 21 | `crates/htui/src/agent_worker.rs` | CREATE | T13 | D28 `AgentRuntime`, `LiveChat`, `ChatCommand`, `ReplyAddr`, `Served`, `run_chat`, `run_turn`. |
| 22 | `crates/htui/src/lib.rs` | UPDATE | T13 | `pub mod agent_worker;` and the quit order (`drop(app)` → await the worker with a timeout → `abort` as backstop; today `lib.rs:87` aborts immediately). |
| 23 | `crates/htui/src/ui/tabs/chat/transcript.rs` | CREATE | T14 | `TranscriptRow`, `Transcript` (frame → rows, in-tab chunk coalescing, thought folding, scroll). |
| 24 | `crates/htui/src/ui/tabs/chat/permission.rs` | CREATE | T14 | `PermissionStrip`: the parked request, numbered options, the `_always` hint. |
| 25 | `crates/htui/src/ui/tabs/chat/composer.rs` | CREATE | T14 | `Composer`: text buffer, compose mode, the hint line. |
| 26 | `crates/htui/src/ui/tabs/chat/mod.rs` | CREATE | T14 | `ChatTab`: state machine, header, caps banner, key handling, request/reply wiring. |
| 27 | `crates/htui/src/ui/tabs/mod.rs`, `src/app/mod.rs` | UPDATE | T14 | `pub mod chat; pub use chat::ChatTab;` and the fourth `register_tab` (`app/mod.rs:42-46`). |
| 28 | `crates/htui/src/testkit.rs` | UPDATE | T14 | `with_agent_runtime`, `drive`, `chat_steps`; a reply channel pair inside `Harness`. |
| 29 | `crates/htui/tests/chat.rs`, `tests/snapshots/chat__*.snap` | CREATE | T14 | The eight snapshots of T14 over `FakeAdapter`. |
| 30 | `crates/htui/tests/snapshots/{integration__demo_shell,shell__after_switch,shell__migration_prompt,shell__offline_label,shell__switcher_empty,shell__switcher_open}.snap` | RE-ACCEPT | T14 | The six snapshots that carry the full tab strip (` 1 Backlog  2 Skills  3 Settings`, six occurrences today) gain ` 4 Chat`. Expected, reviewed with `cargo insta review`; nothing else in them moves. |
| 31 | `README.md` | UPDATE | T14 | Chat tab, `HTUI_TOOL_<NAME>`, `HTUI_KEEP_RAW_EVENTS`, `HTUI_ACP_TRACE`, agent choice key. |

Optional, recommended (H-9): `crates/htui-agent/src/launch.rs:595-602` — `command.kill_on_drop(true)` before `CommandWrap::from(command)`. One line; not in any task's file set; the implementer adds it under T11 or leaves it as a named gap.

---

## B. Interfaces, exactly

### B.1 `crates/htui-agent/src/tools.rs`

```rust
//! D25: the smallest resolver that starts the seeded `claude` row. Stores nothing (probe = M5).
use std::path::Path;
use crate::launch::{Discovery, ToolMap, ToolProbe};
use crate::error::Result;

/// `HTUI_TOOL_<NAME>` with `NAME` = the `${name}` uppercased (`claude_agent_acp` → `HTUI_TOOL_CLAUDE_AGENT_ACP`).
pub fn env_override_key(name: &str) -> String;

/// Resolves every entry of `discovery.tools` (all of them, not only the ones referenced).
/// `None` discovery → empty map (a placeholder-free row resolves against it, `launch.rs:381-382`).
/// Order per tool: env override; `Path` → `which::which(names[i])` on `spawn_blocking`, first hit;
/// `NodePackage` → `<cwd>/node_modules/<package>/<entry>` if it exists, else `npm root -g` (via
/// `tokio::process::Command`, trimmed stdout) joined with `<package>/<entry>` if it exists;
/// `Glob` → `Err(DriverError::Unresolved(name))` whose message names milestone 5.
/// # Errors  `DriverError::Unresolved(name)` for the first tool that resolves nowhere;
/// `DriverError::Transport` when `spawn_blocking` or `npm` itself fails to run.
pub async fn resolve(discovery: Option<&Discovery>, cwd: &Path) -> Result<ToolMap>;
```

Unit tests (T11 list): `Path` through `which` (use `cargo`, resolved from `$CARGO` as `tests/launch.rs` does), `$HTUI_TOOL_NODE` overrides, `NodePackage` prefers a temp-dir `node_modules` entry over the global root (point `cwd` at a `tempfile::tempdir` — add `tempfile = "3"` as a dev-dependency of `htui-agent`; the workspace already carries it for `htui`/`htui-store`), `Glob` → `Unresolved` naming milestone 5.

### B.2 `crates/htui-agent/src/permission.rs`

```rust
use crate::driver::{PermissionDefault, PermissionMatch, PermissionPolicy};
use crate::event::{PermissionOption, PermissionOptionKind, ToolCallEvent};

/// Which stage answered (ANA-4 §4.3 stages 1 and 2; `Default` is `settings.permission.default`
/// when it is not `ask`). Recorded rows say `by: "policy"` for all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyStage { Rule, Remembered, Default }

/// A stage-1/2 answer: the option to send back and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyAnswer {
    pub option_id: String,
    pub kind: PermissionOptionKind,
    pub stage: PolicyStage,
    /// `rule.reason`, `"remembered"` or `"default"`.
    pub reason: String,
}

/// `None` means stage 3: ask the user. `call` is the `tool_call` event the caller saw for
/// `request.tool_call_id` (it may be absent; then only all-`None` matchers match).
#[must_use]
pub fn evaluate(policy: &PermissionPolicy, call: Option<&ToolCallEvent>, options: &[PermissionOption]) -> Option<PolicyAnswer>;

/// `tool_kind` ↔ `call.tool_kind.as_str()`; `tool_name` ↔ `call.title` (ACP has no tool name
/// without `unstable_tool_call_name`, `SCHEMA/tool_call.rs:37-41`); `path_prefix` ↔
/// `locations[0].path` else `input["path"]`/`input["file_path"]`; `command_prefix` ↔ `input["command"]`.
#[must_use]
pub fn matches(matcher: &PermissionMatch, call: Option<&ToolCallEvent>) -> bool;

/// The option a `kind` answers with: exact kind, else its `_once`/`_always` sibling, else `None`.
#[must_use]
pub fn option_for(kind: PermissionOptionKind, options: &[PermissionOption]) -> Option<&PermissionOption>;
```

`PermissionDefault::Ask` → `None`; `Allow` → `option_for(AllowOnce)`; `Deny` → `option_for(RejectOnce)`.

### B.3 `crates/htui-agent/src/acp/map.rs`

```rust
//! ANA-4 §6.1 as code. Input is the raw `params.update` object of one `session/update`
//! (`SCHEMA/client.rs:49-61`: `{ sessionId, update: { sessionUpdate: <tag>, ... }, _meta? }`),
//! never an SDK enum: `SessionUpdate` is `#[serde(tag = "sessionUpdate")] #[non_exhaustive]`
//! with no catch-all variant (`SCHEMA/client.rs:94-160`), so a typed decode of an update the
//! schema does not know is a serde error, and the claude adapter ships five such kinds
//! (`docs/ANA-4.md` §6.1 last paragraph). JSON in, `DriverEvent`s out, nothing escapes.
use serde_json::Value;
use crate::event::*;

/// Per-session mapping state: the previous cumulative cost, for the §7 delta rule.
#[derive(Debug, Default)]
pub struct Mapper { last_cost_micros_total: Option<i64> }

impl Mapper {
    #[must_use] pub fn new() -> Self;
    /// One `update` object → zero, one or two events (a `tool_call` with a `diff` content block is
    /// a `ToolCall` **and** an `EditProposal`). Empty for `user_message_chunk`.
    pub fn map(&mut self, update: &Value) -> Vec<DriverEvent>;
}

/// `sessionUpdate` of an update object, or `"<missing>"` when absent — the `other.update` text.
#[must_use] pub fn update_kind(update: &Value) -> &str;
/// The update object minus its `sessionUpdate` key: the `other.body` of §6.1 ("verbatim body").
#[must_use] pub fn body_of(update: &Value) -> Value;
/// `kind` string → the ten-value `ToolKind`, unknown → `Other` (`event.rs:26-29` says the mapper owns this).
#[must_use] pub fn tool_kind(kind: Option<&str>) -> ToolKind;
/// `status` string → `Some(Completed | Failed)` for the two terminal statuses, `None` otherwise.
#[must_use] pub fn terminal_status(status: Option<&str>) -> Option<ToolResultStatus>;
/// `content[]` `{type:"diff", path, oldText, newText}` entries → `EditProposalEvent`s with a
/// `similar` diff (`fs::unified_diff`), `accepted: None`.
#[must_use] pub fn diffs_of(tool_call_id: &str, content: Option<&Value>) -> Vec<EditProposalEvent>;
/// `content[]` text blocks joined by `\n`, else `rawOutput`, else `None` — `tool_result.output`.
#[must_use] pub fn output_of(content: Option<&Value>, raw_output: Option<&Value>) -> Option<Value>;
/// `locations[]` → `ToolLocation { path, line }`, skipping entries without a `path`.
#[must_use] pub fn locations_of(locations: Option<&Value>) -> Vec<ToolLocation>;
/// `stopReason` string → `StopReason`; unknown → `EndTurn` with a `warn!`.
#[must_use] pub fn stop_reason(text: &str) -> StopReason;
```

The table itself is section E.

### B.4 `crates/htui-agent/src/acp/fs.rs`

```rust
use std::path::{Path, PathBuf};

/// Why a path was refused (the recorded `error { code: "path_outside_session" }`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{path}` is outside the session directories")]
pub struct PathOutside { pub path: String }

/// Lexically normalises `path` (relative → joined onto `cwd`; `.`/`..` folded, no symlink
/// resolution) and admits it iff it starts with `cwd` or one of `extra_dirs`, themselves
/// canonicalised when they exist. Returns the normalised absolute path.
pub async fn guard(path: &Path, cwd: &Path, extra_dirs: &[PathBuf]) -> Result<PathBuf, PathOutside>;

/// Current text of `path`, `""` when it does not exist (ANA-4 §4.3: the client MUST create it).
/// # Errors  Any I/O error other than `NotFound`.
pub async fn read_current(path: &Path) -> std::io::Result<String>;

/// `similar::TextDiff::from_lines(old, new).unified_diff().context_radius(3).header("a/<path>", "b/<path>")`
/// rendered to a `String` (V31 compile-probed). Empty old + new text → a diff of pure `+` lines.
#[must_use] pub fn unified_diff(path: &str, old: &str, new: &str) -> String;

/// Creates parent directories, then writes `content` (truncating).
pub async fn write_text(path: &Path, content: &str) -> std::io::Result<()>;

/// `line`/`limit` of `fs/read_text_file` (`SCHEMA/client.rs:1219-1233`, both 1-based/optional).
#[must_use] pub fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String;
```

### B.5 `crates/htui-agent/src/acp/client.rs`

```rust
use agent_client_protocol::schema::v1::{
    ClientCapabilities as AcpClientCapabilities,         // SCHEMA/client.rs:1963
    FileSystemCapabilities,                              // SCHEMA/client.rs:2415 (new/read_text_file/write_text_file builders)
    Implementation,                                      // SCHEMA/agent.rs:222, new(name, version) at :252
    ReadTextFileRequest, ReadTextFileResponse,           // SCHEMA/client.rs:1219, :1293 — method "fs/read_text_file" (SDK/schema/enum_impls.rs:78)
    RequestPermissionRequest, RequestPermissionResponse, // SCHEMA/client.rs:863, :1006 — "session/request_permission" (enum_impls.rs:79)
    WriteTextFileRequest, WriteTextFileResponse,         // SCHEMA/client.rs:1120, :1175 — "fs/write_text_file" (enum_impls.rs:77)
    RequestId,                                           // registry/agent-client-protocol-schema-1.7.0/src/rpc.rs:31: Null | Number(i64) | Str(String)
};
use agent_client_protocol::Responder;                    // SDK/jsonrpc.rs:4465; respond :4668, respond_with_error :4678, id() :4598
use tokio::sync::mpsc;
use crate::driver::PermissionRequestId;
use crate::event::{PermissionOption, PermissionOptionKind, PermissionRequestEvent};
use crate::launch::ClientCapabilities;

/// What `htui` advertises in `initialize.clientCapabilities`: `fs.readTextFile = settings.fs_read`,
/// `fs.writeTextFile = settings.fs_write`, `terminal = false` **always** (ANA-4 §4.3 "MOD-2 does not
/// advertise the terminal capability in v1"; the row's `terminal` is MOD-11's), no `elicitation`.
#[must_use] pub fn client_capabilities(settings: &ClientCapabilities) -> AcpClientCapabilities;

/// `Implementation::new("htui", env!("CARGO_PKG_VERSION"))` for `initialize.clientInfo`.
#[must_use] pub fn client_info() -> Implementation;

/// The JSON-RPC id of the wire request, as text: `Number(n)` → `n.to_string()`, `Str(s)` → `s`,
/// `Null` → `"null"`. Matched on variants, not on `Display` (**UNVERIFIED — implementer must
/// check** the derived `Display` text of `Number`/`Str`; matching sidesteps it).
#[must_use] pub fn request_id(id: &RequestId) -> PermissionRequestId;

/// `optionId` → `id`, `name` → `label`, `kind` decoded from the wire string with all four values
/// accepted (`SCHEMA/client.rs:987-995`); an unknown string → `RejectOnce` plus a `warn!`
/// (the hint's safe direction). `tool_call_id` = `request.tool_call.tool_call_id`.
#[must_use] pub fn permission_event(request: &RequestPermissionRequest, id: PermissionRequestId) -> PermissionRequestEvent;

/// What a builder handler forwards to the session task. The handler does nothing else: no await
/// on a store, no `block_task`, no file I/O (ANA-4 §4.2 deadlock rule; D18).
#[derive(Debug)]
pub enum Inbound {
    Permission(RequestPermissionRequest, Responder<RequestPermissionResponse>),
    ReadFile(ReadTextFileRequest, Responder<ReadTextFileResponse>),
    WriteFile(WriteTextFileRequest, Responder<WriteTextFileResponse>),
}

/// The unbounded, never-blocking handler → task edge (D18).
pub type InboundTx = mpsc::UnboundedSender<Inbound>;
```

The three handler closures are written inline in `acp/mod.rs::run_session` (they must capture an
`InboundTx` clone each and are `AsyncFnMut(Req, Responder<Req::Response>, ConnectionTo<Agent>) ->
Result<(), Error> + Send`, `SDK/jsonrpc.rs:1494-1512`); `client.rs` owns the bodies as three free
functions `forward_permission`, `forward_read`, `forward_write`, each `tx.send(Inbound::…)` and
`Ok(())` (`()` is `IntoHandled` = claimed, per the example `SDK/examples/yolo_one_shot_client.rs:46-51`).

### B.6 `crates/htui-agent/src/acp/mod.rs`

```rust
//! Ordering rule (D19), stated once here and nowhere else:
//! update order within a turn is the SDK's — `ActiveSession::read_update` yields dispatches and
//! the stop reason from one ordered channel (`SDK/session.rs:1035-1062`, `send_ordered_request_to`
//! at :1037). A `permission_request` (and an `fs/*` interception) arrives on the handler channel
//! and is emitted when this task observes it; it is ordered only relative to events already
//! forwarded, and its correlation to a call is `tool_call_id`, never `seq` adjacency.
//! Builder handlers run **before** dynamic session handlers (`SDK/jsonrpc.rs:426-466`), so the
//! three `on_receive_request` handlers of `client.rs` claim their requests; `session/update` has
//! no builder handler and reaches `read_update()` untyped (`SDK/session.rs:1198-1238`).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use htui_core::model::{Agent, AgentBox};

pub mod client;
pub mod fs;
pub mod map;

/// `other.update` of the model-unavailable row (D23).
pub const MODEL_UNAVAILABLE: &str = "model_unavailable";
/// `other.update` of the transport-closed row (§6.1 "transport close → error"; the `done` that
/// follows is synthesized).
pub const TRANSPORT_CLOSED: &str = "transport_closed";
/// `error.code` of a refused `fs/*` path (D22).
pub const PATH_OUTSIDE_SESSION: &str = "path_outside_session";
/// Events channel depth (D18).
pub const EVENTS_CAPACITY: usize = 256;
/// The environment knob that tees every wire line to a file (plan open question 3, T12 fixtures).
pub const TRACE_ENV: &str = "HTUI_ACP_TRACE";

/// How `DriverEnvelope::at` is stamped: the wall clock in production, `epoch + n ms` for the
/// conformance suite, whose criterion 2 compares `at` byte for byte
/// (`crates/htui-agent/src/conformance.rs:323-342`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// `Utc::now().trunc_subsecs(6)` — `TIMESTAMPTZ` precision, the rule `record.rs:82-97` follows.
    Wall,
    /// `epoch + n ms` for the *n*-th envelope of the session (`fake.rs:28-32`'s rule).
    Fixed { epoch: DateTime<Utc> },
}
impl Stamp { #[must_use] pub fn at(self, n: u64) -> DateTime<Utc>; }

/// The byte streams a session runs over, plus the child that owns them when there is one.
/// tokio traits, boxed: adapted to `futures::io` exactly once, inside [`open_session`].
pub struct AcpIo {
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    pub writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    /// `Some` when [`AcpDriver::start`] spawned it; `None` for the in-process test pair.
    pub child: Option<crate::launch::Spawned>,
}
impl core::fmt::Debug for AcpIo { /* prints `child: bool` only */ }

/// Everything the session task needs besides the streams.
#[derive(Clone)]
pub struct SessionOptions {
    pub agent_name: String,
    pub settings: crate::launch::AgentSettings,
    pub stamp: Stamp,
}
impl core::fmt::Debug for SessionOptions {}   // plain derive is fine: no env map inside

/// The production driver for `transport = 'acp'` (D17/D18): one per row, holds no process.
pub struct AcpDriver {
    name: String,
    launch: crate::launch::AgentLaunch,
    settings: crate::launch::AgentSettings,
    caps: crate::driver::DriverCaps,
    io: IoSource,
    stamp: Stamp,
}
enum IoSource {
    /// `tools::resolve` → `launch::resolve` → `launch::spawn` at `start`.
    Spawn,
    /// A pre-built pair, taken once by `start` (a real process starts once too).
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<AcpIo>>),
}
impl core::fmt::Debug for AcpDriver {}   // hand-written: `launch`'s Debug already redacts env

impl AcpDriver {
    /// From a registry row: `agent.launch` and `agent.settings` parsed through serde.
    /// # Errors  `DriverError::Transport("agent.launch does not parse: …")`.
    pub fn from_row(agent: &Agent, caps: crate::driver::DriverCaps) -> crate::error::Result<Self>;
    /// A driver over an in-process transport with a deterministic clock — the T12 harness.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn over(io: AcpIo, agent: &Agent, caps: crate::driver::DriverCaps, stamp: Stamp) -> Self;
}

impl crate::driver::AgentDriver for AcpDriver {
    fn name(&self) -> &str;
    fn caps(&self) -> crate::driver::DriverCaps;
    /// `Spawn`: `tools::resolve(launch.discovery, &spec.cwd)` → `launch::resolve` → env = launch
    /// env ∪ `spec.env` (spec wins; `R-SEC-2`'s only entry point) → `launch::spawn(&resolved, &spec.cwd)`
    /// → `take_stdin().into_inner()` / `take_stdout().into_inner()` (`launch.rs:450-456`, `:483`)
    /// → [`open_session`]. `Prepared`: take the slot (second `start` → `Transport`).
    fn start<'a>(&'a self, spec: crate::driver::SessionSpec, prompt: String)
        -> crate::driver::DriverFuture<'a, Box<dyn crate::driver::AgentSession>>;
}

/// `TransportBuilder` for adapter id `"acp"` (D12/D17).
#[derive(Debug, Default, Clone, Copy)]
pub struct AcpAdapter;
impl crate::registry::TransportBuilder for AcpAdapter {
    fn build(&self, agent: &Agent, on_box: Option<&AgentBox>, caps: crate::driver::DriverCaps)
        -> crate::error::Result<Box<dyn crate::driver::AgentDriver>>;   // AcpDriver::from_row
}

/// What the handle sends the task (D18, with the cancel acknowledgement the plan implies).
#[derive(Debug)]
pub enum SessionCommand {
    FollowUp(String),
    AnswerPermission(crate::driver::PermissionRequestId, crate::driver::PermissionAnswer),
    /// Answered once the kill has happened (D20's last step), so `cancel()` returning means the
    /// tree is gone — what the live-run `pgrep` assertion measures.
    Cancel { grace: Duration, done: oneshot::Sender<()> },
}

/// The live session: channel endpoints and handle-side bookkeeping only (ANA-4 §4.1).
pub struct AcpSession {
    session_ref: crate::driver::AgentSessionRef,
    events: mpsc::Receiver<crate::event::DriverEnvelope>,
    commands: mpsc::UnboundedSender<SessionCommand>,
    /// Envelopes drained while `cancel()` waits for its ack (D-3), served before `events`.
    pending: VecDeque<crate::event::DriverEnvelope>,
    /// Requests handed out by `next_event` and not yet answered. Non-empty ⇒ `next_event` refuses
    /// (`conformance.rs:827-832` asserts `Err(Transport)` for every transport; `fake.rs:291-296`).
    parked: Vec<crate::driver::PermissionRequestId>,
    /// `false` between a handed-out `Done` and the next accepted follow-up (`fake.rs:25-26`).
    turn_open: bool,
    /// `events` closed: `next_event` → `Ok(None)`, the other four → `Closed`.
    ended: bool,
    task: Option<JoinHandle<()>>,
}
impl core::fmt::Debug for AcpSession {}   // counts, never contents

impl crate::driver::AgentSession for AcpSession {
    fn session_ref(&self) -> Option<&crate::driver::AgentSessionRef>;          // always Some
    fn next_event<'a>(&'a mut self) -> crate::driver::DriverFuture<'a, Option<crate::event::DriverEnvelope>>;
    fn send_follow_up<'a>(&'a mut self, text: String) -> crate::driver::DriverFuture<'a, ()>;
    fn answer_permission<'a>(&'a mut self, request_id: crate::driver::PermissionRequestId, answer: crate::driver::PermissionAnswer) -> crate::driver::DriverFuture<'a, ()>;
    fn cancel<'a>(&'a mut self, grace: Duration) -> crate::driver::DriverFuture<'a, ()>;
}
```

Handle rules, in `next_event` order: serve `pending` first; if `ended` → `Ok(None)`; if
`!parked.is_empty()` → `Err(Transport("permission request `<id>` is parked: answer or cancel it
before pulling again"))`; else `events.recv().await` → `None` ⇒ `ended = true`, `Ok(None)`; on an
envelope, bookkeeping **at hand-out**: `PermissionRequest` → push id to `parked`; `Done` →
`turn_open = false`. `send_follow_up`: `ended` → `Closed`; empty text → `Transport`; `turn_open` →
`Transport("a follow-up before the turn's done …")`; else `turn_open = true`, `commands.send(FollowUp)`
(send failure ⇒ `ended = true`, `Closed`). `answer_permission`: `ended` → `Closed`; id not in
`parked` → `Transport("no parked permission request …")`; else remove and send. `cancel`: `ended` →
`Ok(())`; else send `Cancel { grace, done }`, then `loop { select! { _ = &mut done_rx => break, ev =
events.recv() => match ev { Some(e) => pending.push_back(e), None => break } } }`, `parked.clear()`,
and if the task is gone (`done_rx` errored) ⇒ `ended = true`. After the ack the handle keeps
pulling `events` normally: the synthesized results and the `done { cancelled }` are ordinary events.

```rust
/// Opens the session: spawns the task, awaits its readiness, returns the handle.
/// # Errors  Whatever the handshake failed with, as `DriverError::Transport(String)` — with the
/// child's `stderr_tail()` appended when there is a child (`launch.rs:464-469`).
pub async fn open_session(io: AcpIo, spec: crate::driver::SessionSpec, prompt: String, options: SessionOptions)
    -> crate::error::Result<AcpSession>;

/// What the task reports once the first prompt is on the wire.
struct Ready { session_ref: crate::driver::AgentSessionRef }

/// The task body: owns `Spawned`, the `connect_with` future, the parked responders, the call
/// bookkeeping, the `Mapper` and the `Stamp` counter. Never awaits a store, never calls
/// `block_task` inside a handler (the handlers are `client.rs`'s three forwarders).
async fn run_session(
    io: AcpIo, spec: crate::driver::SessionSpec, prompt: String, options: SessionOptions,
    ready: oneshot::Sender<crate::error::Result<Ready>>,
    events: mpsc::Sender<crate::event::DriverEnvelope>,
    commands: mpsc::UnboundedReceiver<SessionCommand>,
);

/// Task-side state, one per session.
struct TaskState {
    stamp: Stamp, n: u64, retain_raw: bool,
    mapper: map::Mapper,
    /// `PermissionRequestId` → the wire responder plus the options offered (kind lookup for the
    /// rejection synthesis) and the gated call id.
    parked: HashMap<crate::driver::PermissionRequestId, ParkedRequest>,
    open_calls: Vec<String>,
    settled_calls: BTreeSet<String>,
    turn_open: bool,
}
struct ParkedRequest {
    responder: agent_client_protocol::Responder<agent_client_protocol::schema::v1::RequestPermissionResponse>,
    options: Vec<crate::event::PermissionOption>,
    tool_call_id: Option<String>,
}
```

Foreground closure (the only place `block_task` appears; each call site carries the comment
`// ANA-4 risk 11: block_task only here, never in a dispatch handler`):

1. `cx.send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(client::client_capabilities(&settings.acp.client_capabilities)).client_info(client::client_info())).block_task().await?`
   (`SCHEMA/agent.rs:55-96`; `ConnectionTo::send_request` `SDK/jsonrpc.rs:3950`; `block_task` `:5965`).
2. `cx.build_session_from(NewSessionRequest::new(&spec.cwd).additional_directories(spec.extra_dirs.clone())).block_task().start_session().await?`
   (`SDK/session.rs:73-79`, `:806`, `:871`; `SCHEMA/agent.rs:796-829`) → `ActiveSession<'static, Agent>`.
3. Model (D23): pick `config_id` = `settings.acp.model_config_id`, else the first
   `session.config_options()` (`SDK/session.rs:1009`) entry whose `kind` is `Select` and whose
   ungrouped/grouped `options[].value` contains `spec.model` (`SCHEMA/agent.rs:2102-2110`, `:2160-2240`,
   `:2328-2345`). If found: `cx.send_request(SetSessionConfigOptionRequest::new(session_id, config_id, SessionConfigOptionValue::value_id(model))).block_task().await`
   (`SCHEMA/agent.rs:2524-2540`, `:2445-2460`; method `session/set_config_option`, `SDK/schema/enum_impls.rs:32`);
   its `config_options` reply is not recorded (the banner already lists the models). If not found
   and `spec.model.is_some()`: emit `Other { update: MODEL_UNAVAILABLE, body: { "requested": model } }`
   **after** the banner. `spec.model == None` → nothing.
4. Banner (D24), the first envelope on `events`: `Other { update: "session_started", body: { session_id, protocol_version: 1, agent_name: init.agent_info.name (else options.agent_name), agent_version: init.agent_info.version (else ""), models: [values of the chosen option, or []] } }`;
   `raw` when `retain_raw` = `{ "initialize": <InitializeResponse as JSON>, "session/new": session.response() as JSON }` (`SDK/session.rs:1022`).
5. `ready.send(Ok(Ready { session_ref }))`; `session.send_prompt(prompt)?` (`SDK/session.rs:1035`).
6. Turn loop — `tokio::select!` over `session.read_update()`, `commands.recv()`, `inbound.recv()`,
   `cx.incoming_closed()` (`SDK/jsonrpc.rs:3623`), with `SessionMessage` matched as
   `SessionMessage(Dispatch::Notification(m)) if m.method() == "session/update"` →
   `mapper.map(&m.params()["update"])` (`SDK/jsonrpc.rs:5303-5311`; `SessionMessage` `#[non_exhaustive]`
   `SDK/session.rs:979-992`, so `_ => Other { update: "unknown_session_message" }`),
   `SessionMessage(Dispatch::Request(m, responder))` → `responder.respond_with_error(Error::method_not_found())` + `Other { update: "unhandled_request", body: { method } }`
   (defensive: builder handlers claim the three we serve), `SessionMessage(Dispatch::Response(..))` → ignore,
   `StopReason(r)` → close the turn (E-row `done`). The command and inbound arms are section C/D.

### B.7 `crates/htui-agent/src/registry.rs` and `src/lib.rs`

```rust
impl DriverFactory {
    /// The production factory: `"acp"` → `AcpAdapter`. Milestone 8 adds `cli/claude_stream_json`.
    #[must_use] pub fn with_acp() -> Self;
}
// lib.rs
pub mod acp; pub mod permission; pub mod tools;
pub use acp::{AcpAdapter, AcpDriver, AcpIo, AcpSession, SessionCommand, SessionOptions, Stamp};
pub use permission::{PolicyAnswer, PolicyStage, evaluate as evaluate_permission};
pub use tools::resolve as resolve_tools;
```

### B.8 `crates/htui-store/src/writer.rs`, `backend.rs`, `crates/htui-core/src/store/mem.rs`

```rust
// writer.rs
/// D26: an owned handle a recorder can hold across a session. `Offline` hands out none.
#[derive(Debug, Clone)]
pub enum Writer { Memory(MemStore), Online(PgStore) }
impl ReadStore for Writer  { /* 7 methods, `traits.rs:30-45`, plain delegation */ }
impl WriteStore for Writer { /* 9 methods, `traits.rs:50-146`, plain delegation */ }
impl Writer { /// `Backend::label()`'s vocabulary for logs.  #[must_use] pub const fn label(&self) -> &'static str; }

// backend.rs (module doc :6-10 amended: "the only ways to reach `WriteStore` are `writable()`
// for a borrowed `PgStore` and `writer()` for an owned handle; `Offline` answers `None` to both")
impl Backend {
    #[must_use] pub fn writer(&self) -> Option<Writer>;   // Memory → Some(Memory(store.clone())), Online → Some(Online(pg.clone())), Offline → None
    /// D29. `Online` → `pg.this_user()` (`crates/htui-store/src/pg/mod.rs:406`); `Memory` → `store.this_user()`,
    /// `None` → `StoreError::NotFound { entity: "app_user", id: "(none loaded)" }`; `Offline` → `StoreError::Unreachable("no user while offline")`.
    pub async fn this_user(&self) -> Result<UserId>;
}

// mem.rs (removes the `#[expect(dead_code)]` on `State.users`, :45-47)
impl MemStore {
    /// The earliest-created `app_user` row (`created_at`, then `id` as the tiebreak), if any.
    #[must_use] pub fn this_user(&self) -> Option<UserId>;
}
```

`PgStore` is `Clone` (`pg/mod.rs:53`), `MemStore` is `Clone` (`mem.rs:37`): a `Writer` is a handle.

### B.9 `crates/htui/src/store_worker.rs`

```rust
pub enum StoreRequest {
    /* existing */
    /// D27. `model` `None` = the agent's `default_model`, then the agent's own default (D23).
    ChatStart { project_id: ProjectId, agent_id: AgentId, model: Option<String>, prompt: String },
    ChatSend { step_id: StepId, text: String },
    ChatAnswer { step_id: StepId, request_id: PermissionRequestId, answer: PermissionAnswer },
    ChatCancel { step_id: StepId },
}
// name(): "chat_start" | "chat_send" | "chat_answer" | "chat_cancel"

/// One frame of a chat stream (D27). Every frame carries the `ChatStart` request's `seq`/origin.
#[derive(Debug, Clone)]
pub enum ChatFrame {
    /// A recorded, scrubbed envelope (the recorder's UI copy, `record.rs:833-846`), **or** one of
    /// the three htui-authored rows re-shaped as `DriverEvent::Other` for transport only:
    /// `{ update: "prompt", body: { text } }`, `{ update: "follow_up", body: { text } }`,
    /// `{ update: "permission_answer", body: { request_id, option_id, by, cancelled } }`.
    /// These three are never recorded as `other`; the recorder wrote the real row first.
    Event(Box<DriverEnvelope>),
    /// The session is over; `stop_reason` is `Cancelled` when a turn was cut, else the last turn's.
    Ended { stop_reason: StopReason },
    /// The session died before or after `ChatAccepted`; the run is closed `failed`.
    Failed { message: String },
}

pub enum StoreReply {
    /* existing */
    Chat(ChatFrame),
    /// Sent by the chat task once `AgentDriver::start` returned (deferred; see C-1 step 9).
    ChatAccepted { step_id: StepId, session_ref: AgentSessionRef, caps: DriverCaps },
}

/// [`spawn`] with the production runtime (`AgentRuntime::production()`); the harness builds its own.
pub fn spawn(started: Started, rx: mpsc::UnboundedReceiver<RequestEnvelope>, tx: mpsc::UnboundedSender<ReplyEnvelope>) -> JoinHandle<()>;
pub fn spawn_with(started: Started, rx: …, tx: …, runtime: AgentRuntime) -> JoinHandle<()>;
```

Served-ahead arm (before `try_serve`, mirroring `ApplyMigrations` at `store_worker.rs:308-330`):

```rust
StoreRequest::ChatStart { .. } | StoreRequest::ChatSend { .. }
| StoreRequest::ChatAnswer { .. } | StoreRequest::ChatCancel { .. } => {
    match runtime.serve(&backend, &tx, &envelope).await {
        Served::Reply(reply) => reply,                                   // falls through to the one send below
        Served::Deferred => continue,                                    // the task answers this seq
        Served::Start { step_id, task } => { runtime.attach(step_id, tokio::spawn(task)); continue }
    }
}
```

On `rx.recv() == None` (UI gone): `runtime.shutdown(SHUTDOWN_GRACE).await` before the refresher
abort (`store_worker.rs:393-395`).

### B.10 `crates/htui/src/agent_worker.rs`

```rust
/// D20's grace for a user cancel; the conformance suite uses 0.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Depth of the recorder's UI channel. Drained by the same task after every `record`, so a full
/// channel is structurally impossible here (one `record` emits at most one frame); the count is a
/// bound, not a budget.
pub const UI_FRAMES: usize = 256;
/// `HTUI_KEEP_RAW_EVENTS=1` (D29).
pub const KEEP_RAW_ENV: &str = "HTUI_KEEP_RAW_EVENTS";

/// Where a reply goes: the request's `seq` and origin (`store_worker.rs:155-162`).
#[derive(Debug, Clone)]
pub struct ReplyAddr { pub seq: Seq, pub origin: Origin }

/// What the worker sends a live chat (D28's `ChatCommand`); each carries its own reply address so
/// the task answers that request exactly once.
#[derive(Debug)]
pub enum ChatCommand {
    Send { text: String, reply: ReplyAddr },
    Answer { request_id: PermissionRequestId, answer: PermissionAnswer, reply: ReplyAddr },
    Cancel { reply: Option<ReplyAddr> },
}

/// One live chat as the worker sees it (D28 minus `session_ref`, which only the task learns).
pub struct LiveChat { pub commands: mpsc::UnboundedSender<ChatCommand>, pub caps: DriverCaps, pub task: Option<JoinHandle<()>> }
impl core::fmt::Debug for LiveChat {}

/// A chat's session future: production spawns it, the harness polls it inline (D30).
pub type ChatTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// What `serve` decided.
pub enum Served { Reply(StoreReply), Deferred, Start { step_id: StepId, task: ChatTask } }
impl core::fmt::Debug for Served {}

pub struct AgentRuntime { factory: DriverFactory, live: HashMap<StepId, LiveChat>, grace: Duration }
impl core::fmt::Debug for AgentRuntime {}
impl AgentRuntime {
    #[must_use] pub fn new(factory: DriverFactory) -> Self;
    #[must_use] pub fn production() -> Self;                 // DriverFactory::with_acp()
    #[must_use] pub fn with_grace(self, grace: Duration) -> Self;
    /// Every step this runtime has started and not yet swept (live or finished).
    #[must_use] pub fn steps(&self) -> Vec<StepId>;
    pub fn attach(&mut self, step_id: StepId, task: JoinHandle<()>);
    /// Serves one of the four chat requests. Sweeps finished chats first (`commands.is_closed()`).
    pub async fn serve(&mut self, backend: &Backend, tx: &mpsc::UnboundedSender<ReplyEnvelope>, envelope: &RequestEnvelope) -> Served;
    /// Cancels every live chat and awaits its task up to `2 × grace` each; then aborts stragglers.
    pub async fn shutdown(&mut self, grace: Duration);
}

/// Inputs of one chat task; built by `serve(ChatStart)`.
pub struct ChatArgs {
    pub driver: Box<dyn AgentDriver>, pub writer: Writer, pub chat: ChatRunSpec, pub spec: SessionSpec,
    pub prompt: String, pub policy: PermissionPolicy, pub caps: DriverCaps,
    pub commands: mpsc::UnboundedReceiver<ChatCommand>, pub replies: mpsc::UnboundedSender<ReplyEnvelope>,
    pub addr: ReplyAddr, pub grace: Duration,
}
impl core::fmt::Debug for ChatArgs {}   // hand-written: `spec` redacts env itself

/// D28's `run_chat`: start → accept → prompt row → turns → close the run → `Ended`/`Failed`.
pub async fn run_chat(args: ChatArgs);

/// How one turn ended.
enum TurnEnd { Done(StopReason), Cancelled }
/// Pumps one turn: mirrors `record::pump` (`record.rs:991-1008`) but stops pulling while a
/// permission request is parked and serves commands in between — `pump` cannot cross a parked
/// request on any transport (`conformance.rs:399-428` exists for the same reason).
async fn run_turn(session: &mut dyn AgentSession, recorder: &mut Recorder<'_, Writer>, ui: &mut mpsc::Receiver<DriverEnvelope>,
                  commands: &mut mpsc::UnboundedReceiver<ChatCommand>, policy: &PermissionPolicy,
                  calls: &mut HashMap<String, ToolCallEvent>, frames: &Frames, grace: Duration) -> Result<TurnEnd, DriverError>;

/// The reply-channel side of a chat: one address, one sender.
struct Frames { tx: mpsc::UnboundedSender<ReplyEnvelope>, addr: ReplyAddr }
impl Frames { fn event(&self, env: DriverEnvelope); fn local(&self, update: &str, body: Value, at: DateTime<Utc>); fn ended(&self, stop: StopReason); fn failed(&self, message: String); fn reply(&self, addr: &ReplyAddr, reply: StoreReply); }
```

`serve` per request (errors → `Served::Reply(Failed { request: name, message })`):

- `ChatStart`: `backend.writer()` (`None` → `Failed("chat needs a writable store")`, D28) →
  `backend.box_info().await?` (`BoxInfo.box_id`, `crates/htui-core/src/model/box_.rs:85-87`; `None` →
  `Failed("this box is not registered")`) → `backend.this_user().await?` → `backend.agents().await?`
  find `agent_id` with `enabled` → `driver = factory.driver_for(&agent, on_box)?` → `settings =
  serde_json::from_value::<AgentSettings>(agent.settings).unwrap_or_default()` (the `registry.rs:162-164`
  rule) → `ChatRunSpec::mint(project_id, box_id, user, Some(agent_id), model.or(agent.default_model))`
  (`crates/htui-core/src/model/run.rs:225-241`) → `writer.start_chat_run(&chat).await?` →
  `SessionSpec { agent_id, step_id: chat.step_id, cwd: std::env::current_dir()?, extra_dirs: [],
  env: BTreeMap::new(), model, tools: default, mcp: [], permission: settings.permission.clone(),
  retain_raw: env KEEP_RAW_ENV == "1", resume: None }` → channels → `live.insert(step_id, LiveChat
  { commands, caps: driver.caps(), task: None })` → `Served::Start { step_id, task: Box::pin(run_chat(args)) }`.
- `ChatSend`/`ChatAnswer`/`ChatCancel`: `live.get(&step_id)` (`None` → `Failed("no live chat for
  step …")`) → `commands.send(cmd with reply addr)` (`Err` → remove entry, `Failed("chat has
  ended")`) → `Served::Deferred`.

### B.11 `crates/htui/src/testkit.rs`

```rust
pub struct Harness {
    /* existing */
    /// The agent runtime the chat requests are served by, when a test installed one.
    runtime: Option<AgentRuntime>,
    /// Chat futures `drive()` polls inline (D30): production spawns, the harness awaits.
    chats: Vec<(StepId, ChatTask)>,
    /// The reply channel a runtime writes frames into; drained by `drive()`.
    replies: (mpsc::UnboundedSender<ReplyEnvelope>, mpsc::UnboundedReceiver<ReplyEnvelope>),
}
impl Harness {
    #[must_use] pub fn with_agent_runtime(mut self, runtime: AgentRuntime) -> Self;
    /// `settle()` plus the chat pump: serves queued store requests inline (chat requests through
    /// the runtime), polls every held chat future once with `futures::poll!`
    /// (`registry/futures-0.3.34/src/lib.rs:127`), delivers every reply frame through
    /// `App::update(Action::Reply)`, and repeats until a full round makes no progress. Panics
    /// after `SETTLE_ROUNDS` rounds like `settle()` (`testkit.rs:140-169`).
    pub async fn drive(&mut self);
    /// Steps of the chats this harness started, oldest first (for `step_events` assertions).
    #[must_use] pub fn chat_steps(&self) -> Vec<StepId>;
}
```

Without `with_agent_runtime`, `settle()`/`drive()` answer every chat request `Failed { request:
name, message: "no agent runtime in this harness" }` — the `chat_offline` fixture.

### B.12 `crates/htui/src/ui/tabs/chat/*`

```rust
// transcript.rs
pub enum TranscriptRow {
    Prompt { text: String }, FollowUp { text: String },
    Assistant { text: String, message_id: Option<String> }, Thought { text: String },
    ToolCall { id: String, title: String, kind: ToolKind, status: CallStatus },
    EditProposal { id: Option<String>, path: String, diff: String, accepted: Option<bool> },
    Permission { request_id: PermissionRequestId, tool_call_id: Option<String>, options: Vec<PermissionOption>, resolved: Option<Resolution> },
    Plan { entries: Vec<PlanEntry> }, Usage { line: String }, Error { code: String, message: String },
    Done { stop_reason: StopReason }, Other { update: String },
}
pub enum CallStatus { Running, Completed, Failed { reason: Option<TerminalReason> } }
pub struct Resolution { pub option_id: Option<String>, pub by: String, pub cancelled: bool }

#[derive(Debug, Default)]
pub struct Transcript { rows: Vec<TranscriptRow>, fold_thoughts: bool, scroll: usize }
impl Transcript {
    /// Applies one frame: chunks append to an open `Assistant`/`Thought` row with the same
    /// `message_id` (the tab's own coalescing; the store's is the recorder's); `ToolResult` folds
    /// into its `ToolCall` row; `EditProposal` replaces a row with the same `(id, path)`;
    /// `Other { update: "prompt" | "follow_up" | "permission_answer" }` become their own kinds,
    /// `permission_answer` resolving the matching `Permission` row; `session_started` is skipped
    /// here (the header shows it) and every other `Other` is a dim one-liner.
    pub fn apply(&mut self, envelope: &DriverEnvelope);
    pub fn parked(&self) -> Option<&TranscriptRow>;      // the newest unresolved Permission row
    pub fn on_key(&mut self, key: KeyEvent) -> Handled;  // j/k/g/G scroll, t toggles folding
    pub fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>>;   // `+`/`-` gutters: accent / error
}

// permission.rs
pub struct PermissionStrip;
impl PermissionStrip {
    /// `[1] Allow once  [2] Reject once …`, `_always` options suffixed `(agent remembers; htui does not yet)` (D21).
    pub fn render(frame: &mut Frame<'_>, area: Rect, row: &TranscriptRow, theme: &Theme);
    /// Digit → the option at that 1-based index, when the row offers one.
    #[must_use] pub fn pick(row: &TranscriptRow, digit: char) -> Option<PermissionOption>;
}

// composer.rs
#[derive(Debug, Default)]
pub struct Composer { text: String, active: bool }
impl Composer {
    pub fn on_key(&mut self, key: KeyEvent) -> ComposerOutcome;   // chars/Backspace edit; Enter → Submit(text); Esc → Leave
    pub fn render(frame: &mut Frame<'_>, area: Rect, composer: &Self, hint: &str, theme: &Theme);
}
pub enum ComposerOutcome { Consumed, Submit(String), Leave }

// mod.rs
#[derive(Debug)]
pub struct ChatTab {
    agents: Vec<AgentSummary>, agent_index: usize,
    session: Option<ChatSessionState>,     // None → `chat_empty`
    transcript: Transcript, composer: Composer,
    /// First `Esc` arms a cancel; a second within the same "armed" state cancels (T14 key table).
    cancel_armed: bool,
    /// The last `Failed { request: "chat_*" }` text, rendered as the one-line refusal.
    refusal: Option<String>,
}
#[derive(Debug, Clone)]
pub struct ChatSessionState { pub step_id: StepId, pub session_ref: Option<AgentSessionRef>, pub caps: DriverCaps, pub model: Option<String>, pub project: String, pub ended: Option<StopReason>, pub pending_start: bool }
impl ChatTab { pub const ID: TabId = TabId("chat"); #[must_use] pub fn new() -> Self; }
impl Tab for ChatTab {
    fn id(&self) -> TabId; fn title(&self) -> &str;                 // "Chat"
    fn wants_requests(&self, _: &Scope) -> Vec<StoreRequest>;       // [StoreRequest::Agents]
    fn on_scope_change(&mut self, _: &Scope);                       // keeps the session; refreshes nothing
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}
```

Key table (`on_key`, before the tab-scope and global tables, `state.rs:372-409`):

| state | key | effect |
|---|---|---|
| composing | any char / Backspace | edit, `Consumed` |
| composing | Enter | no session → `ctx.request(ChatStart { project_id: ctx.projects[0].project_id, agent_id, model: None, prompt })`; session → `ctx.request(ChatSend { step_id, text })`; `Consumed` |
| composing | Esc | leave compose, `Consumed` |
| parked permission | `1`..`9` | `ctx.request(ChatAnswer { step_id, request_id, answer: Selected(option.id) })`, `Consumed` (so `TabAction::Select`, `keymap.rs:218-225`, never sees the digit) |
| idle | `i`, Enter | enter compose, `Consumed` |
| idle | `a` (no session) | next enabled agent, `Consumed` |
| idle | Esc | first: `cancel_armed = true` (status hint); second: `ctx.request(ChatCancel { step_id })`, `Consumed`; any other key disarms |
| idle | `j` `k` `g` `G` `t` | transcript, `Consumed` |
| anything else | | `Pass` |

`on_reply`: `Agents(list)` → keep enabled agents, default index 0 (name order; `a` cycles);
`ChatAccepted { step_id, session_ref, caps }` → session state; `Chat(Event(env))` →
`transcript.apply`, `session_started` body → `session_ref`; `Chat(Ended { .. })` → `ended`;
`Chat(Failed { message })` / `Failed { request: "chat_*", message }` → `refusal`.

Render, top to bottom inside a `Block` titled ` Chat `: header line `agent · model · project ·
session <id|—>`; caps banner line only when `!caps.permission_requests || !caps.edit_proposals ||
!caps.plans` ("this agent cannot: permission requests, edit proposals, plans" — the missing ones);
transcript (`Constraint::Min(1)`); permission strip (1 line, only while parked); composer (1 line)
with hint `i compose · Esc Esc cancel · 1-9 answer · t thoughts` or `Enter send · Esc done`;
`refusal` replaces the transcript with one dim line. Styles come from `Theme` (`ui/theme.rs:11-24`)
only.

---

## C. Data flow

### C-1. Chat start → child → handshake → banner → first prompt → assistant chunk on screen

1. `ChatTab::on_key(Enter)` while composing with no session → `Composer::on_key` → `Submit(text)`
   → `ctx.request(StoreRequest::ChatStart { .. })` (`state.rs:110-112` pushes `Action::Store`).
2. `App::drain` → `App::dispatch(Origin::Tab(ChatTab::ID), ChatStart)` stamps `seq`, records
   `latest[(origin, discriminant(ChatStart))] = seq` (`state.rs:261-274`) and sends the
   `RequestEnvelope` to the worker.
3. `store_worker::spawn_with` loop, `rx.recv()` arm → the served-ahead arm →
   `AgentRuntime::serve(&backend, &tx, &envelope)`.
4. `serve`: `Backend::writer()` → `Writer::Online(pg)`; `Backend::box_info()`, `Backend::this_user()`,
   `Backend::agents()`; `DriverFactory::driver_for(&claude_row, on_box)` (`registry.rs:79-94`) →
   `AcpAdapter::build` → `AcpDriver::from_row`; `ChatRunSpec::mint`; `Writer::start_chat_run`;
   `SessionSpec`; `LiveChat` inserted; returns `Served::Start { step_id, task }`.
5. Worker: `tokio::spawn(task)`, `runtime.attach(step_id, handle)`, `continue` (no reply yet).
6. `run_chat` → `AgentDriver::start(spec, prompt)` → `AcpDriver::start`:
   `tools::resolve(Some(&launch.discovery), &cwd)` (`$HTUI_TOOL_NODE`/`which node`, `which claude`,
   `npm root -g` + `@agentclientprotocol/claude-agent-acp/dist/index.js`) → `launch::resolve`
   (`launch.rs:383`) → `launch::spawn(&resolved, &cwd)` (`launch.rs:543`: `which` on a blocking
   thread, `ProcessGroup::leader()` on unix `:606-615`, stderr tail task `:561-573`) →
   `Spawned::take_stdin().into_inner()`, `take_stdout().into_inner()` → `AcpIo` → `open_session`.
7. `open_session` → `tokio::spawn(run_session(..))` → awaits `ready`.
8. `run_session`: if `HTUI_ACP_TRACE` is set, wrap reader/writer in the line tee (D-6); build
   `ByteStreams::new(writer.compat_write(), reader.compat())` (`SDK/jsonrpc.rs:6391-6399`;
   `tokio_util::compat` as `launch.rs:23`); `Client.builder().name("htui")` (`SDK/role.rs:275`,
   `jsonrpc.rs:1209`) `.on_receive_request(forward_permission, on_receive_request!())` ×3
   (`jsonrpc.rs:1494`, macro `lib.rs:213`) `.connect_with(transport, async |cx: ConnectionTo<Agent>| { … })`
   (`jsonrpc.rs:1871-1880`). Inside: B.6 steps 1–5. The banner is `events.send(envelope).await`'d
   before `send_prompt`. `ready.send(Ok(Ready { session_ref }))`.
9. `open_session` returns `AcpSession`; `run_chat` sends `StoreReply::ChatAccepted { step_id,
   session_ref, caps }` stamped with `addr` (the ChatStart seq/origin) — the request's one reply.
10. `run_chat`: `Recorder::new(&writer, &scrubber, step, retain_raw, Some(ui_tx))`
    (`record.rs:311-317`); `recorder.record_prompt(&prompt, json!([]), now)` (`:370`); `Frames::local("prompt", { text }, now)`.
11. `run_turn`: `session.next_event()` → `AcpSession` pulls the banner envelope → `recorder.record`
    (`:507`; scrub → buffer → `send_ui` `:833-846` into `ui_tx`) → `run_turn` drains `ui_rx`
    (`try_recv` loop) → `Frames::event(env)` → `ReplyEnvelope { seq: addr.seq, origin: addr.origin,
    reply: Chat(Event(env)) }` on the unbounded reply channel.
12. Meanwhile the agent answers the prompt with `session/update` notifications. Transport →
    `Lines` → dispatch loop → builder handlers decline (`jsonrpc.rs:426-436`) → dynamic
    `ActiveSessionHandler` claims by `sessionId` (`session.rs:1198-1238`, `get_session_id`
    `jsonrpc.rs:5194-5203`) → `update_tx` → the task's `session.read_update()` →
    `SessionMessage::SessionMessage(Dispatch::Notification(m))` → `mapper.map(&m.params()["update"])`
    → `AssistantChunk { text, message_id }` → `stamp.at(n)`, `raw` iff `retain_raw` → `events.send().await`.
13. `run_turn` → `next_event` → `record` (coalesces into the open `assistant_text` row, frame out
    per chunk) → frame → reply channel.
14. `event_loop::run` `replies.recv()` arm (`event_loop.rs:41`) → `App::update(Action::Reply)` →
    `on_reply` (`update.rs:106-178`): `is_fresh(Tab(chat), addr.seq)` passes (V28) →
    `ChatTab::on_reply` → `Transcript::apply` appends the chunk to the open `Assistant` row →
    `dirty` → the next frame draws it.

### C-2. A permission request: wire → parked responder → options → key → wire answer → row

1. Agent sends `session/request_permission` (`SCHEMA/client.rs:863-871`: `sessionId`, `toolCall:
   ToolCallUpdate`, `options[] { optionId, name, kind }`).
2. Dispatch loop → builder handler `forward_permission(req, responder, _cx)` (typed
   `Responder<RequestPermissionResponse>`, `jsonrpc.rs:1497`) → `inbound.send(Inbound::Permission(req, responder))`
   (unbounded, sync) → `Ok(())`. Nothing else happens in the handler.
3. Task `select!` `inbound.recv()` arm → `client::request_id(responder.id())` → `PermissionRequestId`;
   `client::permission_event(&req, id)`; `parked.insert(id, ParkedRequest { responder, options,
   tool_call_id })`; `events.send(PermissionRequest(event)).await`.
4. `AcpSession::next_event` hands it out and pushes the id to `parked` (handle side).
5. `run_turn`: `permission::evaluate(policy, calls.get(tool_call_id), &options)`:
   - `Some(answer)` (stages 1–2): `session.answer_permission(id, Selected(answer.option_id))`;
     `recorder.record(env)` (the `permission_request` row); `recorder.record_permission_answer(&id,
     Some(&option_id), AnsweredBy::Policy, false, now)` (`record.rs:456-485`); `Frames::local("permission_answer", { request_id, option_id, by: "policy", cancelled: false }, now)`;
     not parked.
   - `None` (stage 3): `recorder.record(env)`; drain `ui_rx` → frame; `parked = Some((id, options, tool_call_id))`.
6. Tab: `Transcript::apply` adds a `Permission` row (unresolved) → `PermissionStrip` renders
   `[1] Allow  [2] Reject` (T14 `chat_permission_inline`).
7. User presses `2` → `ChatTab::on_key` → `PermissionStrip::pick(row, '2')` →
   `ctx.request(ChatAnswer { step_id, request_id, answer: Selected("reject-once") })`, `Consumed`.
8. Worker → `AgentRuntime::serve(ChatAnswer)` → `live[step].commands.send(ChatCommand::Answer { .., reply: addr })` → `Served::Deferred`.
9. `run_turn` (`parked.is_some()`, so only `commands.recv()` is awaited) → the id matches →
   `session.answer_permission(id, answer)` → `AcpSession` removes the id from `parked`, sends
   `SessionCommand::AnswerPermission` → task: `parked.remove(&id)` → the option's kind is looked
   up in `ParkedRequest.options`; `responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))))`
   (`SCHEMA/client.rs:1006-1020`, `:1050-1056`, `:1071-1080`; `respond` `jsonrpc.rs:4668`) → the
   response line goes out. If the kind is `RejectOnce | RejectAlways` and the request named a
   call: `settled_calls.insert(call)`, `open_calls.retain(..)`, `events.send(ToolResult { status:
   Failed, output: None, terminal_reason: Some(Rejected) })` (`fake.rs:262-275` rule 3).
10. `run_turn` → `recorder.record_permission_answer(&id, Some("reject-once"), AnsweredBy::User, false, now)`
    (role `user`, `record.rs:114-123`) → `Frames::local("permission_answer", ..)` →
    `Frames::reply(&addr, ChatAccepted { .. })` (the ChatAnswer's one reply) → `parked = None`; if
    the option was `_always`: `warn!(agent, tool_kind, "an _always grant was forwarded to the agent
    and not persisted (milestone 7)")` (D21).
11. Tab: the `permission_answer` frame resolves the `Permission` row (`resolved: Some(..)`), the
    strip disappears; the synthesized `tool_result` frame (step 9) folds the `ToolCall` row to
    `failed (rejected)`. The store holds `permission_request`, `permission_answer { by: "user" }`,
    `tool_result { terminal_reason: "rejected" }` — the test reads them via `Harness::chat_steps()` +
    `ReadStore::step_events`.

### C-3. `fs/write_text_file`: wire → `edit_proposal` row → diff on screen

1. Agent sends `fs/write_text_file { sessionId, path, content }` (`SCHEMA/client.rs:1120-1129`;
   only because `initialize.clientCapabilities.fs.writeTextFile` was `true`, B.5).
2. Builder handler `forward_write(req, responder, _cx)` → `inbound.send(Inbound::WriteFile(req, responder))` → `Ok(())`.
3. Task `inbound.recv()` arm → `fs::guard(&req.path, &spec.cwd, &spec.extra_dirs).await`:
   - `Err(PathOutside)` → `responder.respond_with_error(Error::invalid_params().data(json!({ "path": .. })))`
     (`SCHEMA/error.rs:94`, `.data` `:76`) and `events.send(Error { code: PATH_OUTSIDE_SESSION, message })`. Stop.
   - `Ok(path)` → `fs::read_current(&path).await` (`""` when absent) → `fs::unified_diff(path_str, &old, &req.content)`
     → `events.send(EditProposal { tool_call_id: None, path, diff, accepted: Some(true) }).await`
     (ANA-4 §4.3: "defaults to `true` for a write that reached the filesystem with no request
     attached"; the enclosing call is unknown to the client on this wire, hence `None`) →
     `fs::write_text(&path, &req.content).await` → `responder.respond(WriteTextFileResponse::new())`
     (`SCHEMA/client.rs:1175-1190`); a write error → `respond_with_error(Error::internal_error().data(msg))` and an `Error { code: "fs_write_failed" }` event.
4. `run_turn` → `record` → the recorder's edit-proposal dedup keys `(None, path)`
   (`record.rs:568-591`); a second write to the same path in the same buffer updates the row.
5. Frame → tab → `Transcript::apply` → `EditProposal` row; `Transcript::lines` renders the diff
   with `+`/`-` gutters (`accent`/`error`), header lines dim.

For the *other* source (`tool_call`/`tool_call_update` `content[].type == "diff"`) the mapper
(section E) produces the `EditProposal` with `tool_call_id: Some(id)` and `accepted: None`, and the
same dedup applies keyed `(Some(id), path)`.

---

## D. Concurrency and ownership

### D-1. Owners

| Thing | Owner | Lifetime |
|---|---|---|
| Child process (`Spawned`) | `run_session` task (moved in through `AcpIo.child`) | until `kill_tree()` at task exit; `wait()` afterwards to reap |
| `connect_with` future, `ConnectionTo<Agent>`, `ActiveSession<'static, Agent>` | `run_session` task, foreground closure | the closure's scope |
| Parked `Responder`s, call bookkeeping, `Mapper`, `Stamp` counter | `run_session` task (`TaskState`) | session |
| `AcpSession` handle | `run_chat` (`Box<dyn AgentSession>`) | until `run_chat` returns |
| `Recorder<'_, Writer>` and the `Writer` it borrows | `run_chat` stack frame | session; `finish()` before `finish_chat_run` |
| `MinimalScrubber` | `run_chat` stack frame (secrets = `spec.env` values) | session |
| `LiveChat { commands, caps, task }` | `AgentRuntime` inside the store worker loop | until swept (`commands.is_closed()`) or shutdown |
| `AgentRuntime` | the store worker task (production) / `Harness` (tests) | worker lifetime |
| Reply sender clone (`UnboundedSender<ReplyEnvelope>`) | one clone per chat, inside `Frames` | session |

The render side owns none of the above (`R-NF-3`): `ChatTab` holds `StepId`s and rows.

### D-2. Channels

| Channel | Type | Direction | Capacity | Full | Closed |
|---|---|---|---|---|---|
| `events` | `mpsc::Sender/Receiver<DriverEnvelope>` | task → handle | 256, `send().await` | the **task** waits (back-pressure; it stops reading the wire and the inbound channel) | task gone ⇒ handle `ended`, `next_event → Ok(None)` |
| `commands` | `mpsc::UnboundedSender/Receiver<SessionCommand>` | handle → task | unbounded | n/a — at most one command per user action; unbounded so a handle blocked in `events` back-pressure can never wait on the task that is waiting on it (**deviation from D18's `mpsc::Sender`**, H-3) | handle dropped ⇒ task treats it as `Cancel { grace: CANCEL_GRACE }` |
| `inbound` | `mpsc::UnboundedSender/Receiver<Inbound>` | builder handlers → task | unbounded | n/a — a handler must never block (`SDK/jsonrpc.rs:587-600`) | task gone ⇒ handler `send` fails ⇒ handler returns `Err(Error::internal_error())`, the SDK answers the request with that error (`:606-608`) |
| `ready` | `oneshot<Result<Ready, DriverError>>` | task → `open_session` | 1 | — | task died before readiness ⇒ `open_session` → `Transport("session task ended before the handshake")` |
| `done` (in `Cancel`) | `oneshot<()>` | task → handle | 1 | — | task died ⇒ `cancel()` marks `ended`, returns `Ok(())` |
| `ui` (recorder) | `mpsc::Sender/Receiver<DriverEnvelope>` | recorder → `run_turn` (same task) | 256, `try_send` | counted as dropped (`record.rs:837-845`); unreachable here because the producer's own loop drains it after every `record` | never (the receiver lives as long as the recorder) |
| `ChatCommand` | `mpsc::UnboundedSender/Receiver<ChatCommand>` | worker → `run_chat` | unbounded | n/a | worker/runtime dropped ⇒ `run_chat` cancels the session with `grace` and closes the run `Done` (no turn open) or `Cancelled` |
| replies | existing `UnboundedSender<ReplyEnvelope>` clone | `run_chat` → event loop | unbounded (MOD-1 D4) | n/a | UI gone ⇒ `send` fails ⇒ `run_chat` keeps recording (rows matter, frames do not) and ends at the next command-channel close |
| `read_update` | SDK-internal unbounded (`SDK/session.rs:967-968`) | dispatch loop → task | unbounded | n/a | — |

### D-3. Shutdown orders

**On cancel (user, `Esc Esc`)** — `ChatCancel` → worker → `ChatCommand::Cancel { reply }` →
`run_turn`/between-turns arm → `session.cancel(grace)` → handle sends `Cancel { grace, done }` and
drains `events` into `pending` until `done` → task: (1) every `parked` responder →
`respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))` (ANA-4 §4.3 MUST;
D20 first); (2) `cx.send_notification(CancelNotification::new(session_id))` (`SDK/jsonrpc.rs:4205`;
`SCHEMA/agent.rs:5173-5187`; method `session/cancel`, `SDK/schema/client_to_agent/notifications.rs:3`)
— only if a turn is open; (3) `select! { m = session.read_update() => until StopReason, _ =
tokio::time::sleep(grace) }` — a `StopReason` that arrives ends the turn normally with that
reason; the timeout synthesizes; (4) for every call in `open_calls` not in `settled_calls`:
`events.send(ToolResult { Failed, terminal_reason: Cancelled })`; then `events.send(Done { Cancelled })`
if the turn was open (if the agent's own `StopReason::Cancelled` arrived in (3), that is the `Done`;
never two); (5) `spawned.kill_tree().await` (`launch.rs:512-517`), then `spawned.wait().await` to
reap; (6) `done.send(())`; (7) return from the closure ⇒ `connect_with` returns ⇒ transport actors
end ⇒ task ends. `run_chat`: `recorder.record_permission_answer(id, None, Policy, true, now)` per
parked id (`conformance.rs:838-841` order: the answer row lands before the pump resumes), `local`
frames, pump `next_event` until `Done` (results, then done), `Frames::reply(addr, ChatAccepted)`,
`TurnEnd::Cancelled` → `recorder.finish()` → `writer.finish_chat_run(run, step, Cancelled, now)`
→ `Frames::ended(Cancelled)` → return; the runtime sweeps the entry on its next `serve`.
A cancel with **no** turn open skips (2)–(4): status `Done`, `Ended { stop_reason: last turn's }`.

**On turn end** — nothing shuts down. The task idles in `select!` over `read_update`, `commands`,
`inbound`, `incoming_closed`; `run_chat` idles on `commands.recv()` only (not `next_event`: on the
fake a closed turn answers `Ok(None)` at once, `fake.rs:297-299`, which would end every fake chat).

**On transport close (child died / EOF)** — task: `cx.incoming_closed()` resolves
(`SDK/jsonrpc.rs:3615-3625`; pending `block_task`s fail first, `:5455-5457`) → answer parked
responders `Cancelled` (they cannot reach the wire any more, but the map must empty) → if a turn
is open: synthesized results + `Error { code: TRANSPORT_CLOSED, message: stderr tail }` +
`Done { Cancelled }`; else `Error` only → `kill_tree`/`wait` → drop `events` tx → return.
Handle: `next_event` → `Ok(None)` after the queued events; `send_follow_up`/`answer_permission` →
`Closed`. `run_chat`: mid-turn, `run_turn` sees `Done` and returns normally, then the next `Send`
gets `Closed` → `Frames::reply(addr, Failed { request: "chat_send", message })` → status `Failed`,
`Frames::failed`, `finish_chat_run(Failed)`. `run_chat` does not learn of a close between turns
until the user acts (documented in `agent_worker.rs`).

**On app quit (`q`)** — `event_loop::run` returns (`event_loop.rs:44-46`) → `lib.rs::run`:
`term.restore()` → **`drop(app)`** (closes the request channel; today `lib.rs:87` aborts the worker
instead) → worker loop `rx.recv() == None` → `runtime.shutdown(CANCEL_GRACE)`: `Cancel { reply:
None }` to every live chat, `tokio::time::timeout(2 × grace, task)` each, `abort()` on timeout →
refresher abort → worker returns → `run()` awaits `tokio::time::timeout(Duration::from_secs(5),
worker)` then `worker.abort()` as the backstop → `main` returns. Without this order the tokio
runtime drops every task at the first await and the child is orphaned (H-9).

### D-4. Locks

None across an `.await`: `AcpDriver.io` (`Prepared`) is a `std::sync::Mutex` taken and released
before the future is built (the `fake.rs:130-136` pattern); the trace tee holds a `std::sync::Mutex<File>`
inside synchronous `poll_read`/`poll_write`; `MemStore`'s `RwLock` is the store's own (`mem.rs:3`).

---

## E. ANA-4 §6.1 as code (`acp/map.rs`)

`Mapper::map(update)` matches `update_kind(update)`; `body_of` = `update` minus `sessionUpdate`.
Wire keys are camelCase (`SCHEMA/client.rs:96` `rename_all = "camelCase"` on every struct).

| `sessionUpdate` (wire) | Emits | Field derivation |
|---|---|---|
| `agent_message_chunk` | `AssistantChunk(TextChunk)` | `text` = `content.text` when `content.type == "text"` (`SCHEMA/content.rs:38-44`, `:70-77`); any other `content.type` → `Other { update: "agent_message_chunk", body }` (an image/resource chunk has no `text` column); `message_id` = `messageId` as a string (`SCHEMA/client.rs:611-628`; `MessageId` is `#[serde(transparent)]`, `:670-673`) |
| `agent_thought_chunk` | `ThoughtChunk(TextChunk)` | same rule |
| `tool_call` | `ToolCall(ToolCallEvent)` [+ `EditProposal` per diff block] | `tool_call_id` = `toolCallId`; `title`; `tool_kind` = `tool_kind(kind)` (`SCHEMA/tool_call.rs:499-514`, ten values; `Other` is the serde default); `input` = `rawInput` else `Null`; `locations` = `locations_of(locations)` (`:765-770`); then `diffs_of(id, content)` for every `content[]` entry with `type == "diff"` (`:572-579`, `Diff { path, oldText: Option, newText }` `:700-710`) → `EditProposal { tool_call_id: Some(id), path, diff: unified_diff(path, oldText.unwrap_or(""), newText), accepted: None }`. A `tool_call` whose `status` is already terminal (`:540-547` `completed`/`failed`) additionally emits the `ToolResult` row below |
| `tool_call_update`, `status ∈ {completed, failed}` | `ToolResult(ToolResultEvent)` [+ `EditProposal` per diff block, emitted **before** the result] | `tool_call_id`; `status` = `terminal_status`; `output` = `output_of(content, rawOutput)`; `locations`; `terminal_reason: None`. Fields are flat on the update (`ToolCallUpdate { toolCallId, #[serde(flatten)] fields }`, `SCHEMA/tool_call.rs:220-224`, `:273-310`) |
| `tool_call_update`, any other/absent `status` | `EditProposal` per diff block, else nothing | a title/kind/locations-only update produces no row (ANA-9 has no "tool call updated" kind); the task's bookkeeping still notes the call as open |
| `plan` | `Plan(PlanEvent)` | `entries[] { content, status, priority }` (`SCHEMA/plan.rs:33-40`, `:455-460`); an entry whose status/priority string is unknown is dropped with a `warn!` (the fixture snapshot shows the drop) |
| `usage_update` | `Usage(UsageEvent)` | `context_used = used`, `context_size = size` (`SCHEMA/client.rs:504-513`); `cost` (`:564-569` `{ amount, currency }`): `currency == "USD"` → `total = round(amount × 1e6)`, `cost_micros_total = Some(total)`, `cost_micros = Some(total − last_cost_micros_total.unwrap_or(0))`, `last = Some(total)`; other currency → `cost_micros = None`, `cost_amount`, `cost_currency`; no `cost` → all cost keys `None`; the four token fields `None` (§7); `usage_scope: None`. `_meta["_claude/rateLimit"]` is left in `raw` only (M7 latches it) |
| `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update` | `Other { update, body }` | verbatim minus the tag (§6.1 row 14; D23 for `config_option_update`) |
| `user_message_chunk` | **nothing** (`Vec::new()`) | §6.1 last row: replay only; would duplicate a `prompt`/`follow_up` |
| anything else (`subagent_spawned`, `async_task_*`, `compaction_*`, a typo) | `Other { update, body }` | the wildcard; `update_kind` of a missing tag is `"<missing>"` |

Not updates, mapped by the task (`acp/mod.rs`), same table rows:

| Source | Emits | Derivation |
|---|---|---|
| `session/request_permission` (builder handler → `Inbound::Permission`) | `PermissionRequest(PermissionRequestEvent)` | `client::permission_event` (B.5): `request_id` = JSON-RPC id, `tool_call_id` = `toolCall.toolCallId`, `options[] { id: optionId, label: name, kind }` |
| the answer `htui` sends | — (recorded by `run_chat` via `Recorder::record_permission_answer`) | `option_id` + `by: user` / `by: policy`; cancel → `option_id: null, by: policy, cancelled: true` (ANA-4 §4.3 "Cancellation") |
| `fs/write_text_file` | `EditProposal { tool_call_id: None, accepted: Some(true) }` | C-3 |
| `fs/read_text_file` | nothing (served: `ReadTextFileResponse::new(slice_lines(text, line, limit))`) | a refused path → `Error { code: PATH_OUTSIDE_SESSION }` |
| `SessionMessage::StopReason(r)` | `Done { stop_reason }` [after synthesized `ToolResult`s when `r == Cancelled`] | `StopReason` maps one-to-one (`SCHEMA/agent.rs:3164-3171` ↔ `event.rs:81-95`); `r` unknown (`#[non_exhaustive]`) → `EndTurn` + `warn!` |
| JSON-RPC error on `session/prompt` (`block_task`/`on_receiving_result` `Err`) | `Error { code: error.code as text, message }` then `Done { Cancelled }` | `read_update` surfaces it as `Err` (`SDK/session.rs:1041-1051` maps the prompt result through `?`) |
| transport close | `Error { code: TRANSPORT_CLOSED, message: stderr tail }` [+ `Done { Cancelled }` if a turn is open] | D-3 |
| the handshake | `Other { update: "session_started", body }` first; `Other { update: MODEL_UNAVAILABLE }` when D23 finds no option | B.6 steps 3–4 |

Task-side terminal-state rules applied around the table (`fake.rs:11-26`, the five rules a harness
owes): a `ToolCall` id joins `open_calls` unless settled; a `ToolResult` for a settled id is
**dropped** (`conformance.rs:917-995` `rejected_tool_gets_failed_result`); a rejection answer
synthesizes `ToolResult { Failed, terminal_reason: Rejected }`; cancel synthesizes
`terminal_reason: Cancelled` for every open call; exactly one `Done` per turn.

`raw` (when `retain_raw`): notifications → `{ "method": "session/update", "params": <params> }`;
the `done` → `{ "method": "session/prompt", "result": { "stopReason": .. } }`; `permission_request`
→ `{ "method": "session/request_permission", "params": <request as JSON> }`; the banner → B.6
step 4. The SDK consumes the wire line (`SDK/jsonrpc.rs:6401-6418` `into_lines`), so `raw` is the
parsed message re-serialised, never the bytes; the bytes are what `HTUI_ACP_TRACE` keeps (H-10).

---

## F. Test seams

### F-1. The in-process scripted agent (`tests/acp_conformance.rs`)

```
AcpHarness { epoch: DateTime<Utc>, sessions: AtomicU64, row: Agent }   // row = seed_rows()[claude] re-stamped
impl CaseHarness for AcpHarness {
    fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
        let (client_end, agent_end) = tokio::io::duplex(64 * 1024);   // V31
        let (reader, writer) = tokio::io::split(client_end);
        tokio::spawn(scripted_agent(agent_end, script, format!("acp-{}", n)));   // inside #[tokio::test]
        Box::new(AcpDriver::over(AcpIo { reader: Box::new(reader), writer: Box::new(writer), child: None },
                                 &self.row, caps_for(&self.row), Stamp::Fixed { epoch: conformance::epoch() }))
    }
}
```

`scripted_agent` speaks raw newline-delimited JSON-RPC over `BufReader::lines()` /
`write_all` — **no SDK type on the agent side**, so the test proves the wire format (`serde_json`
only). Its state machine:

| Receives | Sends |
|---|---|
| `initialize` request | result `{ "protocolVersion": 1, "agentCapabilities": { "loadSession": false }, "authMethods": [], "agentInfo": { "name": "scripted", "version": "0.0.0-scripted" } }` |
| `session/new` request | result `{ "sessionId": "<id>", "configOptions": [ { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "sonnet", "options": [ { "value": "sonnet", "name": "Sonnet" } ] } ] }` (the D23 id path; `session_spec` sets `model: Some("sonnet")`, `conformance.rs:246`) |
| `session/set_config_option` request | result `{ "configOptions": [ …same… ] }` |
| `session/prompt` request | plays the next `Turn` (turn 0 on the first prompt; each later prompt is a follow-up) as `session/update` notifications, then answers the request with `{ "stopReason": <done's> }` |
| `session/cancel` notification | stops the turn; answers the open prompt `{ "stopReason": "cancelled" }` |
| a response to its own `session/request_permission` | continues the turn |
| EOF | exits |

Script → wire, one `ScriptEvent` at a time (the inverse of section E; anything unrepresentable
panics naming the event, so a future case that needs it fails loudly):

| `ScriptEvent` | wire (`params.update` unless stated) |
|---|---|
| `Emit(AssistantChunk { text, message_id })` | `{ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text" }, "messageId" }` |
| `Emit(ThoughtChunk ..)` | `agent_thought_chunk`, same |
| `Emit(ToolCall { id, title, kind, input, locations })` | `{ "sessionUpdate": "tool_call", "toolCallId", "title", "kind", "rawInput": input, "locations" }` |
| `Emit(ToolResult { id, status, output, .. })` | `{ "sessionUpdate": "tool_call_update", "toolCallId", "status", "rawOutput": output }` (the driver drops it when the call is settled) |
| `Emit(EditProposal { id, path, diff, accepted })` | `tool_call_update { toolCallId: id, content: [ { "type": "diff", "path", "oldText": null, "newText": diff } ] }` → the driver synthesizes `unified_diff(path, "", diff)`, which **contains** `diff` (F-3) |
| `Emit(Plan { entries })` | `{ "sessionUpdate": "plan", "entries" }` |
| `Emit(Usage { cost_micros: Some(d), context_used, context_size, .. })` | `{ "sessionUpdate": "usage_update", "used": context_used.unwrap_or(0), "size": context_size.unwrap_or(0), "cost": { "amount": cumulative/1e6, "currency": "USD" } }` — the agent keeps the cumulative sum, the driver re-derives the delta (E) |
| `Emit(Other { update, body })` | `{ "sessionUpdate": update, ..body }` (object bodies flattened; a non-object body → `{ "sessionUpdate", "body" }`) |
| `Emit(Done { stop_reason })` | the `session/prompt` result `{ "stopReason" }` |
| `ParkPermission(req)` | request `{ "id": req.request_id (a JSON string), "method": "session/request_permission", "params": { "sessionId", "toolCall": { "toolCallId": req.tool_call_id }, "options": [ { "optionId": o.id, "name": o.label, "kind": o.kind } ] } }`; blocks until the response line |
| `ExpectCancel` | stop emitting; wait for `session/cancel` |
| `Emit(PermissionRequest ..)`, `Emit(Error ..)` | panic: no ACP wire shape (no case uses them) |

Determinism: every `at` is `epoch + n ms` (`Stamp::Fixed`), every session id is minted from the
harness counter and substituted out by `without_identity` (`conformance.rs:330-342`), JSON-RPC ids
are the SDK's own counter per connection, and the scripted agent emits strictly in script order on
one stream. `raw_iff_retain` compares `raw` only for presence (`:774-788`) and the rest with `raw`
cleared (`:789-800`).

### F-2. `Harness::drive()` (T14) pumps a chat inline

`with_agent_runtime(AgentRuntime::new(factory))` where the test builds `factory` with its own
`FakeAdapter` registered under `cli/fake` (the `tests/extensibility.rs:151-158` pattern;
`with_test_support()`'s adapter is unreachable by design, `fake.rs:509-524`), upserts a `cli`
row with `settings.cli.stream = "fake"` into the `MemStore` it passes to `Harness::over` (the
`tests/settings.rs:62` pattern), and `adapter.load(script)` before the key that starts the chat.
`drive()` rounds: (a) `rx.try_recv()` every request — chat ones through `runtime.serve(&backend,
&replies.0, &envelope)`, `Start` pushes the future into `chats`, `Reply` goes straight to
`App::update`; others through `store_worker::serve` as `settle()` does (`testkit.rs:143-158`);
(b) `futures::poll!(task.as_mut())` for each held chat, dropping `Ready` ones; (c)
`replies.1.try_recv()` → `App::update(Action::Reply)`; (d) `pending` overlays as `settle()`; stop
when a round did (a)–(c) nothing. Every await inside a fake-backed `run_chat` resolves on its
first poll except `commands.recv()`, which is exactly "waiting for the user", so a `Pending` chat
after a quiet round is the parked state the snapshot wants. No task is spawned, no sleep exists.
`drive()` subsumes `settle()`; the existing tests keep calling `settle()`.

### F-3. Which existing tests and fixtures must not move, and which must

- `crates/htui-agent/tests/fake_conformance.rs` (`CASES.len() == 13`, `run_all` over the fake),
  `tests/recorder.rs`, `tests/extensibility.rs`, `tests/launch.rs`, `tests/driver_contract.rs`:
  unchanged and green after every T11/T12 commit. The `zeta` sweep (`extensibility.rs` item 2)
  walks `crates/*/src/**`, so no new file may mention that name.
- `conformance.rs` (T12): two scripts change, **assertions weaken only where ANA-4 says the
  transport owns the value** (**ANA-4 ≠ plan**, H-2):
  1. `usage_deltas_sum_to_step_usage` (`:1093-1099`, expected `:1132-1138`): the script's
     `UsageEvent { input_tokens, output_tokens, cost_micros }` becomes `{ cost_micros, context_used,
     context_size }` and the expected sum has the four token keys `null` and `cost_micros: 351`.
     ANA-4 §7: over ACP the four token fields are null; `UsageUpdate` has no token field
     (`SCHEMA/client.rs:504-513`). The recorder's own token summing stays covered by
     `tests/recorder.rs`.
  2. `edit_proposal_deduped_per_call_and_path` (`:1261-1266`): `assert_eq!(str_at(edits[0], "diff"), Some("@@ second"))`
     becomes `assert!(str_at(edits[0], "diff").is_some_and(|d| d.contains("@@ second")))` (and the
     same for the `src/b.rs` row if asserted). ANA-4 §4.3: the unified diff is synthesized from
     `oldText`/`newText`; no ACP shape carries a diff verbatim. The dedup assertion (`edits.len() == 2`,
     `accepted == Some(true)`) is untouched.
  The fake passes both unchanged in behaviour; `CASES` and its length do not change.
- `crates/htui/tests/{backlog,settings}.rs` snapshots: built with `Harness::…with_tab(one tab)`,
  strip ` 1 <Tab> ` only — untouched.
- The six full-strip snapshots (A-30) move by the ` 4 Chat ` token when `register_all` registers the
  tab; accepted deliberately in T14, reviewed line by line.
- `crates/htui/src/testkit.rs` in-module tests (`:237-350`) keep passing: `Harness::over` gains
  fields with `None`/empty defaults and `settle()` is unchanged.
- `crates/htui-store/tests/pg_conformance.rs` (`EXPECTED_CASES = 20`): untouched — no
  `WriteStore` method is added (`Writer` implements the existing nine).

### F-4. Fixture capture (`HTUI_ACP_TRACE`)

`run_session` wraps the streams in `TraceTee` when `HTUI_ACP_TRACE=<path>` is set: every newline-
terminated line is appended as `{ "dir": "agent" | "client", "line": <the raw JSON parsed as a
Value> }`. The T12 fixture is the `dir == "agent"` lines of one live prompt ("reply with the word
ok"), scrubbed by `MinimalScrubber` over the shell's environment before it is committed, with
`sessionId` values replaced by `"<session>"`. `tests/acp_map.rs` replays each fixture line's
`params.update` through `Mapper::map` and snapshots the `Vec<DriverEvent>` as JSON with
`insta::assert_json_snapshot!("acp_map__handshake" | "acp_map__turn")`; a third test appends
`{ "sessionUpdate": "subagent_spawned", "id": "x" }` and asserts the last event is
`Other { update: "subagent_spawned" }`.

---

## G. Build order and checkpoints (serial, TDD)

Every checkpoint: `cargo fmt --all -- --check` first. Postgres steps carry `USERNAME=htui-ci
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres`.

### T11 — the transport

| # | Step | Proof |
|---|---|---|
| 1 | Cargo: workspace `tokio` `fs`; `htui-agent` `similar`, `tokio` `fs`, dev `tempfile` | `cargo build -p htui-agent`; `cargo tree -i tokio -e normal --workspace` = milestone 2's list (criterion 13) |
| 2 | `tools.rs` tests first (B.1) → impl | `cargo test -p htui-agent --lib tools` |
| 3 | `permission.rs` tests first (rule order; remembered after rules; `default: ask` → `None`; `allow` → `allow_once`, falling back to `allow_always`; `path_prefix` from `locations` then `input.path`) → impl | `cargo test -p htui-agent --lib permission` |
| 4 | `acp/fs.rs` tests first (new-file diff applies to `""` and yields the content — assert by reparsing the hunk's `+` lines; a path outside `cwd`/`extra_dirs` is refused and nothing is written, `tempfile`; `slice_lines`) → impl | `cargo test -p htui-agent --lib acp::fs` |
| 5 | `acp/map.rs` tests first from `serde_json::json!` literals of the wire shapes: every §6.1 row of section E, `user_message_chunk` → empty, unknown tag → `Other` with verbatim body, ten `ToolKind` values incl. `switch_mode`, `tool_call_update` `completed`/`failed` → `ToolResult`, `in_progress` → none, a diff block → `EditProposal`, cost delta across three updates (100/350/351 µUSD → 100/250/1) → impl | `cargo test -p htui-agent --lib acp::map` |
| 6 | `acp/client.rs` tests first (capability block from `ClientCapabilities` omits `terminal` even when the row says `true`; `permission_event` decodes all four kinds; `request_id` for `Number`/`Str`) → impl | `cargo test -p htui-agent --lib acp::client` |
| 7 | `acp/mod.rs`: `Stamp`, `AcpIo`, `SessionCommand`, `AcpSession` handle rules with a unit test over a hand-fed `events`/`commands` pair (parked ⇒ `Err(Transport)`; follow-up before done ⇒ `Err(Transport)`; closed ⇒ `Ok(None)` then `Closed`) → `run_session` → `AcpDriver`, `AcpAdapter` | `cargo test -p htui-agent --lib acp`; `cargo clippy -p htui-agent --all-targets --all-features -- -D warnings` |
| 8 | `registry.rs` `with_acp()`, `lib.rs` exports; `extensibility.rs` still reports `adapter_ids() == ["cli/fake"]` for `with_test_support()` | `cargo test -p htui-agent --all-features`; `cargo doc -p htui-agent --no-deps` |

Commit: `feat(agent): ACP transport — session task, §6.1 mapper, fs interception, tool resolver`.

### T12 — conformance, fixtures, live smoke

| # | Step | Proof |
|---|---|---|
| 1 | `conformance.rs` F-3 amendments; the fake still passes | `cargo test -p htui-agent --features test-support --test fake_conformance` |
| 2 | `tests/acp_conformance.rs`: `CASES.len() == 13`, the scripted agent, `run_all(&AcpHarness, \|\| async { MemStore::demo() })` — expect red until each rule lands; run case by case with `run_case(name, ..)` | `cargo test -p htui-agent --features test-support --test acp_conformance` |
| 3 | `HTUI_ACP_TRACE` tee; capture one live prompt; scrub; commit the two fixtures; `tests/acp_map.rs` + snapshots | `cargo test -p htui-agent --features test-support --test acp_map`; `cargo insta review` |
| 4 | `tests/acp_live.rs` `#[ignore]`: `tools::resolve` on the seed row, `launch::resolve`, `launch::spawn`, one `initialize` through `Client.builder()…connect_with`, assert `protocolVersion == 1`, `agentInfo.version` non-empty, `authMethods` present; `kill_tree`; `pgrep -f claude-agent-acp` empty afterwards (Linux) | `cargo test -p htui-agent --features test-support --test acp_live -- --ignored --nocapture` |

Commit: `test(agent): ACP conformance over a duplex, transcript fixtures, live smoke`.

### T13 — writer, runtime, chat seam

| # | Step | Proof |
|---|---|---|
| 1 | `mem.rs` `this_user()` (+ unit test: demo → `ids::USER`, `MemStore::new()` → `None`) | `cargo test -p htui-core --all-features this_user` |
| 2 | `writer.rs` + `Backend::{writer, this_user}` tests first (`Memory`/`Online` `Some`, `Offline` `None`; `Writer::Memory` round-trips `start_chat_run` → `active_runs` → `finish_chat_run`; `this_user` on `Memory` = `ids::USER`, `Offline` = `Unreachable`) → impl; `pg_criteria.rs` arm | `cargo test -p htui-store --features demo`; with Postgres `cargo test -p htui-store --all-features` (no skip: quote the assertions) |
| 3 | `htui/Cargo.toml`; `store_worker.rs` variants, `name()` arms, `ChatFrame`, replies; unit tests: each chat request has a `name()` arm and is served ahead of `try_serve`; `ChatStart` against `Backend::Offline` → `Failed { request: "chat_start", .. }` | `cargo test -p htui store_worker` |
| 4 | `agent_worker.rs` tests first over `MemStore::demo()` + a `FakeAdapter` factory: `run_chat` writes `prompt`, the scripted events, `done`; `active_runs` returns to its previous count; the reply channel holds one `ChatAccepted`, one `Chat(Event)` per recorded envelope **plus** the `prompt` local frame, one `Chat(Ended)`, all with the `ChatStart` seq/origin; `ChatSend` for an unknown step → `Failed`; a mid-turn `ChatCancel` (script with `park` + `ExpectCancel`) → `Ended { Cancelled }`, `active_runs` back, `step_events` ends `done { cancelled }` → impl | `cargo test -p htui agent_worker` |
| 5 | `lib.rs` quit order; `spawn_with` | `cargo test -p htui`; `cargo clippy -p htui -p htui-store --all-targets --all-features -- -D warnings` |

Commit: `feat(store,tui): Writer, AgentRuntime and the four chat requests`.

### T14 — the chat tab

| # | Step | Proof |
|---|---|---|
| 1 | `testkit.rs` `with_agent_runtime` / `drive` / `chat_steps`; existing testkit tests green | `cargo test -p htui --features testkit testkit` |
| 2 | `tests/chat.rs` — the eight snapshots of the plan's T14 list, each `Harness::demo().with_tab(Box::new(ChatTab::new())).with_agent_runtime(..)`, keys via `harness.key(..)`, `drive().await`, `insta::assert_snapshot!("chat_<name>", harness.render())`; `chat_permission_inline` additionally reads `step_events(harness.chat_steps()[0])` from the `MemStore` clone it kept | red |
| 3 | `chat/transcript.rs`, `chat/permission.rs`, `chat/composer.rs`, `chat/mod.rs`; `tabs/mod.rs` export | `cargo test -p htui --features testkit --test chat`; `cargo insta review` for the eight new files |
| 4 | `app/mod.rs` fourth registration; re-accept the six strip snapshots (A-30), verifying each diff is the strip line only | `cargo test -p htui --features testkit`; `cargo insta review` |
| 5 | README | `cargo test --workspace --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo doc --workspace --no-deps` |
| 6 | Live run: `cargo run -p htui` against Postgres → `4` → `a` until `claude` → `i` → prompt → a tool call → a permission answered with a digit → `Esc Esc` → `pgrep -f claude-agent-acp` empty. `cargo run -p htui -- --demo` → `4` → the empty tab renders (H-8). | the milestone write-up quotes the transcript |

Commit: `feat(tui): the chat tab over the chat seam`.

---

## H. Risks

| # | Risk | Mitigation in this design | Anticipated by the plan? |
|---|---|---|---|
| H-1 | A builder `on_receive_notification::<SessionNotification>` would claim every `session/update` before the `ActiveSessionHandler` (`SDK/jsonrpc.rs:426-466`), starving `read_update()`; a typed decode also loses the claude adapter's off-schema kinds (`SessionUpdate` has no catch-all, `SCHEMA/client.rs:94-160`) | No notification handler on the builder; updates reach the task untyped through `read_update()` and `map.rs` reads JSON | **No** — D17 lists "one `on_receive_notification`"; dropped |
| H-2 | Two `CASES` cannot pass on ACP as written: token fields (ANA-4 §7, no token field in `UsageUpdate`) and the verbatim diff text (ANA-4 §4.3 synthesizes it) | F-3 amends the two scripts/assertions; `CASES` unchanged; ANA-4 wins | **No** — the plan claims "adding no case … pass unchanged" |
| H-3 | A bounded `commands` channel plus an awaited bounded `events` channel lets the handle and the task wait on each other (handle blocked sending a command while the task is blocked sending an event the handle is not draining) | `commands` unbounded (one message per user action); `cancel()` drains `events` into `pending` while it waits for its ack | **No** — D18 says `mpsc::Sender` |
| H-4 | `record::pump` cannot cross a parked permission request (`next_event` → `Err(Transport)` on every transport, `conformance.rs:827-832`) | `run_turn` (B.10) pulls only while nothing is parked and serves commands in between; `pump` stays untouched for the conformance suite | Partly — D28 said "pumps `record::pump` per turn" |
| H-5 | Stages 1–2 of §4.3 cannot "record through `Recorder::record_permission_answer` inside the task": the recorder lives in `run_chat`, not in the session task | `permission.rs` evaluator (pure) called by `run_chat` on every `PermissionRequest`, before it is parked; the wire answer and the row happen in the same place. File-set additions: `htui-agent/src/permission.rs` (T11), `htui-core/src/store/mem.rs` (T13, `this_user`) | **No** — D21 places the stages in the task |
| H-6 | The handshake (node start, `initialize`, `session/new`) can take seconds; doing it inside `AgentRuntime::serve` would stall every store request behind it | `ChatAccepted` is deferred: `run_chat` sends it after `start()`; `serve(ChatStart)` does only store writes and returns the future; `LiveChat` carries no `session_ref` | **No** — D27/D28 imply an immediate reply |
| H-7 | `at` must be deterministic under criterion 2 (`without_identity` keeps `at`, `conformance.rs:323-342`); a wall-clock stamp in the driver fails `coalesce_across_message_id` on ACP | `Stamp::Fixed` in `AcpDriver::over`; `Stamp::Wall` truncated to microseconds in production | No — nothing in the plan names the clock seam |
| H-8 | `--demo` cannot show a scripted chat: both demo agents are `acp`, and the fake adapter is `test-support`-only | T14's validate step is corrected to "the empty tab renders; a prompt runs the real driver" | **No** — T14 says "a scripted chat renders" |
| H-9 | On quit the tokio runtime drops the session task at its first await and orphans the child; `lib.rs:87` aborts the worker before it can cancel anything | D-3 quit order (`drop(app)` → `runtime.shutdown` → awaited worker with timeout); optional `kill_on_drop(true)` in `launch.rs:595-602` as the backstop (kills the leader only; the group kill is `kill_tree`) | Partly — criterion 11 is a live assertion, the quit path is not in any task |
| H-10 | `raw` is the parsed message re-serialised, not the wire bytes (the SDK consumes the line) | Documented on `DriverEnvelope.raw` usage in `acp/mod.rs`; the bytes are `HTUI_ACP_TRACE`'s | No |
| H-11 | The path guard is lexical: a symlink inside `cwd` pointing outside is admitted | Documented limitation; `canonicalize` of the deepest existing ancestor is a follow-up, not silent | No — D22 introduced the guard without stating its strength |
| H-12 | `tool_name` matching has nothing to match on ACP (no `name` without `unstable_tool_call_name`, `SCHEMA/tool_call.rs:37-41`) | `matches` compares `tool_name` against `title` and says so in its doc; M8's CLI path has real names | No |
| H-13 | `PermissionOptionKind` on the wire is `#[non_exhaustive]`; a fifth kind would not fit our four-value enum | Decoded from the JSON string; unknown → `RejectOnce` + `warn!` (the id is what is sent back, ANA-4 §4.3) | No |
| H-14 | `Backend::this_user()` on `Memory` over an empty store has no user | `NotFound { entity: "app_user" }` → `Failed { request: "chat_start" }`; `Harness::empty()` chats fail honestly | No |
| H-15 | Two `similar` majors in the graph (`2.7.0` via `insta`, `3.2.0` here) | Accepted (V30, X7); recorded at close-out | Yes (V30) |
| H-16 | The claude adapter emits a `usage_update` only after the first assistant usage (ANA-4 §4.4 "Unverified"); a rate-limit blob may never arrive in a short turn | Nothing in M3 depends on it (`_meta` stays in `raw`); M7 verifies | Yes (§11.14) |
| H-17 | `cwd` for a chat is the `htui` process directory: no project repo path exists before MOD-13/15 | Stated in `serve(ChatStart)`; the header shows the project name, not a path | No |
| H-18 | The between-turns idle cannot see a transport close until the next `ChatSend` (D-3) | Documented; the `Failed` reply on that send closes the run `failed` | No |
| H-19 | Windows job object, `PATHEXT`, `CREATE_NO_WINDOW` unverified on this box | Carried as D31; nothing here changes `launch.rs`'s Windows path | Yes (V32) |
