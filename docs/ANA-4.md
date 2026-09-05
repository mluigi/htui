# ANA-4 - Agent protocol: ACP client and CLI fallback

> **Scope note:** Design authority for how `htui` drives a coding agent: the `AgentDriver` trait and
> its event model, the ACP client, the CLI stream adapter, permission and edit-proposal handling,
> the `agent.launch` and `agent.settings` JSONB shapes left open by `docs/ANA-9.md` §5.7,
> autodiscovery probes per agent, and quota capture. Governed by `.claude/rules/workflow-docs.md`,
> `CONCEPTS.md`, `docs/REQUIREMENTS.md` and `docs/ANA-9.md` (§4.3, §5.7, §5.8, §6.1, §9).
>
> **Requirements addressed:** `R-AGT-1..8`, `R-HIS-1..3`, `R-SEC-2..3`, `R-TUI-6`, `R-TUI-8`.
> Touched at their seams only, and settled elsewhere: `R-PRM-1..3` (ANA-5 supplies the prompt this
> document transports), `R-SEC-1` and `R-SEC-4` (ANA-7), `R-MCP-1` and `R-MCP-4` (MOD-11 supplies
> the server this document reserves a hook for), `R-ORCH-11` (the driver writes the per-step fields,
> the orchestrator owns the status machine).
>
> **Status (2026-09-05): concluded.** Implementation tracked as MOD-2 (agent driver + chat tab) in
> `HANDOFF.md`. One forward-only migration amendment to the ANA-9 schema is named in §9.

---

## 1. Context and problem statement

`docs/REQUIREMENTS.md` §5 fixes the contract: one `AgentDriver` trait with five operations and nine
event kinds (`R-AGT-1`), ACP as the primary transport with `htui` as the client (`R-AGT-2`), a CLI
stream adapter for agents without ACP (`R-AGT-3`), a Postgres registry seeded with `claude` and
`agy` (`R-AGT-4`), a new agent costing a registry row and at most one adapter (`R-AGT-5`),
autodiscovery probes per box (`R-AGT-6`), and quota tracking with a cancelling cap (`R-AGT-7..8`).
`docs/ANA-9.md` §4.3 fixes the row those events land in and hands three obligations to this
document: the chunk-coalescing rule, the `agent.launch` shape, and the `agent.settings` shape.
Until they are fixed, `agent.launch JSONB NOT NULL` has no default, so MOD-6 seeded zero agent rows
and the `agent` table is empty in Postgres today.

This document settles the six questions the `HANDOFF.md` ANA-4 item names, verbatim:

1. the `AgentDriver` trait and event model,
2. the ACP client design (`agent-client-protocol` crate or hand-rolled JSON-RPC),
3. permission and edit-proposal handling,
4. how `claude` is reached over ACP,
5. whether `agy` speaks ACP or needs the CLI adapter,
6. autodiscovery probes per agent.

Four premises that were current when the item was written are now false, and every one of them
changes an answer. They are recorded here so the next reader does not re-derive them:

| Stale premise | State on 2026-09-05 |
|---|---|
| The Rust SDK exposes `Client`/`Agent` traits and `ClientSideConnection` | Gone. `agent-client-protocol` 2.1.0 is a role/builder model: `Client` is a unit struct, `Client.builder()...connect_with(transport, main_fn)`. The old shape was the 0.x line. |
| The SDK needs `#[async_trait(?Send)]` and a `LocalSet` | False. 2.1.0's `src/` contains zero `?Send`, `LocalSet`, `spawn_local` or `Rc<`; handler futures are *required* to be `Send`. |
| `claude` speaks ACP behind a `--acp` flag (the demo fixture's `["claude","--acp"]`) | False. `claude --help` on 2.1.261 has no `acp` subcommand or flag. ACP requires the external Node adapter. |
| `agy` needs the CLI adapter (the demo fixture's `transport: cli`, `["agy","run"]`) | False twice. `agy` has no `run` subcommand, and Google ships a first-party ACP server, `agy_acp_server`, listed in the ACP registry. |

The demo fixture in `crates/htui-core/src/fixtures.rs` encodes the two false launch guesses; §5
replaces them.

---

## 2. Invariants

Restated from `CONCEPTS.md` and `docs/REQUIREMENTS.md`; each has a mechanical enforcement point in
this design.

1. **One driver trait, ACP first.** Every transport implements the same `AgentDriver` /
   `AgentSession` pair and emits the same `DriverEvent` enum (`R-AGT-1..3`). Enforced by the one
   conformance suite of §8 running over every transport plus a fake.
2. **A new agent is a registry row plus at most one adapter.** No orchestrator or prompt code
   changes (`R-AGT-5`). Enforced by putting every per-agent variation into `agent.launch` /
   `agent.settings` (§5) and by the transport dispatch being a two-arm match on
   `agent.transport`.
3. **No agent in a bookkeeping path.** The driver never writes a row except `session_event`,
   `run_step.usage`, `run_step.prompt_digest` and `agent_box`; every other table stays `htui`
   code's (ANA-9 §2.5). Enforced by the driver crate depending on `WriteStore`, never on `PgStore`.
4. **Secrets reach the agent only as subprocess environment.** `SessionSpec.env` is the single
   channel; `htui`'s own DSN is never in it, and no tool reads secrets (`R-SEC-2`). Enforced by a
   hand-written redacting `Debug` on the launch and spec types, and by the keyring DSN living in
   `htui-store`, a crate the driver does not depend on.
5. **Scrub before persist, fail closed.** Every event passes the ANA-7 scrubber before it reaches
   the store or the `pending/*.jsonl` buffer; an unmasked pattern fails the step and blocks the
   write (`R-SEC-3`). Enforced by the recorder of §4.1 owning the only two write paths.
6. **Ordering is `seq`, and `seq` has one writer.** One recorder task per `run_step` assigns `seq`
   and `turn`; `at` is informational (ANA-9 §4.3). Enforced by the recorder owning the counter and
   the store's `PRIMARY KEY (run_step_id, seq)` rejecting a duplicate.
7. **Coalescing is a pure function of the wire stream.** No wall-clock input decides what is
   persisted, so a recorded transcript replays byte-identically in tests (§4.1).

---

## 3. Protocol surface (ACP as read)

Read from `https://agentclientprotocol.com` (v1 pages) and from the vendored crates
`agent-client-protocol-schema` 1.7.0 and `agent-client-protocol` 2.1.0 on 2026-09-05.

**Version.** The protocol version is a single integer sent in `initialize`. `ProtocolVersion` is a
`u16` newtype with associated constants, not an enum: `V0` (pre-release), `V1`, and `V2` behind the
non-default cargo feature `unstable_protocol_v2`; `LATEST` is `V1` and is deliberately absent when
that feature is on. **`htui` sends `protocolVersion: 1`.** ACP v2 is a Draft redesign (prompt
returns on acceptance, stop reason moves to a `state_update`, `authenticate` becomes `auth/login`,
`session/load` gives way to `session/resume`) whose `SessionUpdate` carries *message patching*
semantics that `htui`'s append-only rows cannot express. v2 is out of scope for MOD-2.

**Transport and framing.** stdio is the only non-draft transport: the client launches the agent as a
subprocess, JSON-RPC 2.0 messages are UTF-8, newline-delimited, and MUST NOT contain embedded
newlines. There is no header-based framing. The agent MAY write logs to stderr and MUST NOT write
non-ACP bytes to stdout. A Streamable-HTTP/WebSocket RFD is in progress and is targeted as an
*additive* v1 feature, so a non-stdio transport may appear without a major bump.

**Methods, agent side (v1):** `initialize`, `authenticate`, `logout`, `session/new`,
`session/prompt`, `session/load`, `session/resume`, `session/close`, `session/list`,
`session/delete`, `session/set_mode`, `session/set_config_option`; notification `session/cancel`.
**Client side:** `session/request_permission` (baseline, no capability gate), `fs/read_text_file`,
`fs/write_text_file`, `terminal/create|output|wait_for_exit|kill|release`, `elicitation/create`;
notifications received `session/update`, `elicitation/complete`. Both sides: `$/cancel_request`.
Error codes: `-32700/-32600/-32601/-32602/-32603` plus `-32800` RequestCancelled, `-32000`
AuthRequired, `-32002` ResourceNotFound.

**Capabilities.** All omitted capabilities MUST be treated as unsupported. The client advertises
`fs.readTextFile`, `fs.writeTextFile`, `terminal`, `auth.terminal`, `elicitation` and
`session.configOptions.boolean`. The agent answers with `loadSession: bool`, `promptCapabilities`,
`mcpCapabilities`, `auth`, and `sessionCapabilities { delete, resume, close, list, fork,
additionalDirectories }` — note the mixed encoding: `loadSession` and the prompt/mcp entries are
booleans, while `sessionCapabilities.*` and `auth.logout` are empty **objects** (`{}` = supported).
A Rust decoder that types them all as `bool` fails on a real handshake.

**`session/update` variants, stable set (11):** `user_message_chunk`, `agent_message_chunk`,
`agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`,
`current_mode_update`, `config_option_update`, `session_info_update`, `usage_update`. Five more are
cfg-gated (`plan_update`, `plan_removed`, `notice`, `compaction_update`,
`compaction_summary_chunk`). The enum is `#[non_exhaustive]`, and real adapters ship update kinds
*ahead* of the schema, so the decoder needs a wildcard arm regardless of features.

**Tool calls.** `ToolKind` has ten values — `read, edit, delete, move, search, execute, think,
fetch, switch_mode, other` — with `other` as both the serde `other` catch-all and the `Default`;
the published docs page lists only nine and omits `switch_mode`. `ToolCallStatus` is
`pending|in_progress|completed|failed`; there is no `cancelled` on the wire, and a client is told to
mark unfinished calls cancelled locally when it sends `session/cancel`. `ToolCallContent` is a
tagged union of `content`, `diff { path, oldText: string|null, newText }` and
`terminal { terminalId }`. **Diffs are old/new full text, not unified diffs** — the client
synthesizes the unified text ANA-9 §4.3 asks for. `tool_call_update` is a *patch* keyed on
`toolCallId`: every field except the id is optional and only changed fields are sent.

**Permissions.** `session/request_permission { sessionId, toolCall, options[] }` →
`{ outcome }`. `PermissionOption` is `{ optionId, name, kind }` with kind in
`allow_once | allow_always | reject_once | reject_always` (a closed enum). The outcome is
`{"outcome":"cancelled"}` or `{"outcome":"selected","optionId":...}`. On cancellation the client
MUST answer every outstanding request with `cancelled`.

**Prompt turn.** `session/prompt { sessionId, prompt: ContentBlock[] }` → `{ stopReason }` with
`StopReason ∈ end_turn | max_tokens | max_turn_requests | refusal | cancelled`. `session/load`
replays the whole conversation as `session/update` notifications before responding, so the replay
decoder and the live decoder are the same code; `session/resume` restores context and returns with
no replay.

**Model selection is `session/set_config_option`, keyed on `configId`, not on `category`.** Config
options are `{ id, name, description?, category?, type: "select"|"boolean", currentValue,
options?[] }`; categories (`mode`, `model`, `model_config`, `thought_level`) are explicitly
UX-only and MUST NOT be required for correctness, so a client that looks up the model selector by
`category == "model"` is non-compliant. The response always carries the **complete** option list,
and the agent may push the same complete list as `config_option_update` (for example when it falls
back to another model under rate limiting). Session *modes* (`session/set_mode`) are a different
axis — operating mode, not model — still live in v1 and slated for removal in a later version, so a
client should tolerate a modes-only agent.

**Usage.** The stable mechanism is the `usage_update` variant of `session/update`, carrying
`{ used, size, cost?: { amount, currency } }` — context-window occupancy and cumulative session
cost, not per-turn token deltas — and the agent only **MAY** send it. Per-turn-shaped token
accounting exists as `PromptResponse.usage` behind `unstable_end_turn_token_usage`, is itself
documented as session-cumulative, and may be removed. **ACP carries no subscription allowance, plan
tier or quota-reset field anywhere, and no rate-limited error code.** §7 draws the consequence.

**Extension points.** `_meta` exists on every protocol type; custom methods must start with `_`;
implementations MUST NOT add fields at the root of a spec type. `mcpServers[]` on `session/new`
lets a client hand its own MCP server to the agent — the clean hook for `R-MCP-1`.

**What is unstable and stays off in MOD-2:** `unstable_protocol_v2`, `unstable_end_turn_token_usage`,
`unstable_plan_operations`, `unstable_session_notices`, `unstable_session_compaction`,
`unstable_llm_providers`, `unstable_mcp_over_acp`, `unstable_session_fork`,
`unstable_tool_call_name`.

---

## 4. Settled questions

### 4.1 `AgentDriver` trait and event model (`R-AGT-1`, `R-HIS-1`, `R-SEC-3`)

**Need.** One trait with the five operations `R-AGT-1` names — start a session (prompt, working
directory, environment, tool exposure), stream typed events, send a follow-up, answer a permission
request, cancel — usable as `dyn` from the orchestrator, `Send` enough for `tokio` rt-multi-thread,
and producing exactly the fourteen `EventKind` values of `crates/htui-core/src/model/event.rs`
under `unsafe_code = "forbid"`.

**Options.**

| Option | Verdict |
|---|---|
| `async fn` directly in the trait (AFIT) | Rejected. Not dyn compatible: a probe of `pub trait AgentDriver { async fn start(&self); }` plus `&dyn AgentDriver` fails with `error[E0038] ... because method start is async` on rustc 1.98.1. Return-type notation, which would let a `dyn`-free generic carry `Send`, is still nightly. |
| `#[async_trait]` macro crate | Rejected. It generates exactly the boxed-future desugaring below; the macro buys nothing but a dependency and worse rustdoc. |
| Eager-task returns (Zed's `AgentConnection` shape: every method returns an already-spawned handle) | Rejected. Dyn-compatible and `!Send`-friendly, but it forces a task per operation and hides cancellation; `htui` wants one task per *session*, not per call. |
| Push events into an `mpsc` from SDK callbacks; no session handle | Rejected. Callbacks that block the dispatch loop are a documented deadlock in the SDK, and a push-only design has no place to hold a permission responder. |
| Hand-written boxed futures (`Pin<Box<dyn Future<Output = ...> + Send + '_>>`) on two traits — a `Send + Sync` factory and a `Send` session — with a **pull** `next_event()` | **Adopted.** Dyn-compatible today, explicit about `Send`, one task per session, and the pull loop preserves ordering and backpressure without a `Stream` impl. |

**Adopted.**

```rust
// crates/htui-agent/src/driver.rs

/// Everything a session needs at start (`R-AGT-1`: prompt, cwd, environment, tool exposure, model).
pub struct SessionSpec {
    pub agent_id: AgentId,
    pub step_id: StepId,
    pub cwd: PathBuf,
    pub extra_dirs: Vec<PathBuf>,          // ACP additionalDirectories / claude --add-dir
    pub env: BTreeMap<String, String>,     // resolved secrets only (R-SEC-2); redacted Debug
    pub model: Option<String>,             // run_step.model; None = the agent's own default
    pub tools: ToolExposure,               // allow / deny lists + command_run exposure (R-MCP-3)
    pub mcp: Vec<McpServerSpec>,           // htui's own MCP server (R-MCP-1)
    pub permission: PermissionPolicy,      // agent.settings.permission, §4.3
    pub retain_raw: bool,                  // project.settings.keep_raw_events (ANA-9 §4.3)
    pub resume: Option<AgentSessionRef>,   // agent-side session id from a previous step
}

/// Capability predicates next to the operations they gate; the CLI transport answers `false` to
/// three of them, and the chat tab and orchestrator branch on that rather than on `Transport`.
#[derive(Debug, Clone, Copy)]
pub struct DriverCaps {
    pub permission_requests: bool,
    pub edit_proposals: bool,
    pub plans: bool,
    pub thoughts: bool,
    pub follow_up_in_session: bool,        // false = a follow-up respawns the process
    pub resume: bool,
    pub usage: bool,
}

/// One per `agent` row. Cheap, cloneable, holds no child process.
pub trait AgentDriver: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;
    fn caps(&self) -> DriverCaps;

    /// Starts the agent, negotiates, opens a session and sends the initial prompt.
    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn AgentSession>, DriverError>> + Send + 'a>>;
}

/// One per live session. Owns nothing but channel endpoints; the transport runs in its own task.
pub trait AgentSession: Send + std::fmt::Debug {
    /// The agent-side session id, for `session/load` / `--resume` on a later step.
    fn session_ref(&self) -> Option<&AgentSessionRef>;

    /// Ordered pull. `Ok(None)` means the transport closed with no further events.
    /// Exactly one `DriverEvent::Done` per turn precedes the next accepted follow-up.
    fn next_event<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DriverEnvelope>, DriverError>> + Send + 'a>>;

    fn send_follow_up<'a>(
        &'a mut self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>>;

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,          // Selected(option_id) | Cancelled
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>>;

    /// Graceful first: `session/cancel` (ACP) or SIGINT / stdin close (CLI), every outstanding
    /// permission request answered `cancelled`, then a process-tree kill after the grace window.
    fn cancel<'a>(
        &'a mut self,
        grace: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), DriverError>> + Send + 'a>>;
}

/// One wire event plus its optional verbatim message.
#[derive(Debug, Clone)]
pub struct DriverEnvelope {
    pub event: DriverEvent,
    pub raw: Option<serde_json::Value>,    // populated only when `SessionSpec.retain_raw`
    pub at: DateTime<Utc>,
}

/// Exactly `EventKind` minus the three kinds `htui` authors itself
/// (`prompt`, `follow_up`, `permission_answer`), so `From<&DriverEvent> for EventKind` is total.
#[derive(Debug, Clone)]
pub enum DriverEvent {
    AssistantChunk(TextChunk),             // -> assistant_text, after coalescing
    ThoughtChunk(TextChunk),               // -> thought, after coalescing
    ToolCall(ToolCallEvent),               // title, tool_kind, input, locations[]
    ToolResult(ToolResultEvent),           // status, output, locations[]
    EditProposal(EditProposalEvent),       // path, diff (unified), accepted
    PermissionRequest(PermissionRequestEvent),
    Plan(PlanEvent),                       // entries[] { content, status, priority }
    Usage(UsageEvent),
    Error(ErrorEvent),                     // code, message
    Done(DoneEvent),                       // stop_reason
    Other(OtherEvent),                     // update + verbatim body
}

/// A text delta with the grouping key ACP supplies and ANA-9 §4.3 does not carry.
#[derive(Debug, Clone)]
pub struct TextChunk {
    pub text: String,
    pub message_id: Option<String>,
}
```

`SessionSpec` and `AgentLaunch` (§5) both carry a hand-written `Debug` that prints every environment
value as `[REDACTED]`; that is the mechanical enforcement of invariant 4, and it is why
`missing_debug_implementations` is satisfied without deriving.

**Send bounds and dyn-compatibility.** `AgentDriver: Send + Sync`, `AgentSession: Send`, every
returned future `+ Send`. This is achievable because the chosen crate is `Send`-clean (§4.2) *and*
because the session handle is only channel endpoints: the transport's own types never cross the
trait boundary. That second property is what makes the `!Send` contingency cheap — see §4.2.

**Chunk coalescing (ANA-9 §4.3's obligation).** The driver yields one `AssistantChunk` /
`ThoughtChunk` per wire chunk; the **recorder** (`crates/htui-agent/src/record.rs`) merges a
contiguous run into one `assistant_text` or `thought` row. The chat tab receives the chunks, so live
rendering keeps its token-by-token look; the store receives one row per run, as §4.3 requires.
Flush triggers, in order of evaluation:

1. the next envelope is a different `DriverEvent` variant (kind change),
2. `TextChunk.message_id` differs from the open run's (ACP's own grouping key),
3. `Done` arrives (turn end),
4. the accumulated text reaches 16 KiB (a bound, so one pathological turn cannot buffer unboundedly),
5. the session ends or is cancelled.

There is deliberately **no idle-time flush**. A time-based trigger would make the persisted row set
a function of scheduling, which would break both replay equality and the snapshot tests of §8.

**Turn counting and `seq`.** The recorder owns both. `seq` is a 0-based counter incremented once per
persisted row; `turn` starts at 0 with the `prompt` row at `seq = 0` and increments on every
`follow_up`. Every event of a turn carries that turn's number, including the `done` that closes it.
Because exactly one recorder task exists per `run_step`, `seq` needs no coordination and the
`PRIMARY KEY (run_step_id, seq)` is a backstop, not the allocator.

**`prompt_digest`.** The recorder computes `sha256` (workspace dep `sha2 0.10`) over the assembled
prompt text once, writes it as the `digest` key of the `prompt` payload and as
`run_step.prompt_digest`, and never recomputes it. ANA-5 supplies the text and `sections[]`.

**Raw retention.** `SessionSpec.retain_raw` is read from `project.settings.keep_raw_events`. When
false the transports do not even allocate the `serde_json::Value`; when true each envelope carries
the verbatim wire message, which the recorder writes to `session_event.raw`.

**Persistence and the UI, in that order.** The recorder scrubs (`R-SEC-3`), persists, then
`try_send`s to a bounded UI channel, counting drops. It never blocks the read loop on the UI: a
paused or slow chat tab must not stall the agent, and a dropped *render* frame is recoverable from
the store while a dropped *row* is not.

**Store seam.** `WriteStore` gains, in `htui-core`:

```rust
async fn append_events(&self, events: &[SessionEvent]) -> Result<usize>;  // ON CONFLICT DO NOTHING
async fn set_step_usage(&self, step: StepId, usage: Value, prompt_digest: Option<String>) -> Result<()>;
async fn upsert_agent(&self, agent: &Agent) -> Result<()>;
async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()>;
```

Offline, the recorder writes the same rows as JSON lines to
`<cache_dir>/pending/<project_id>.<run_id>.jsonl`. That name matches
`crates/htui-store/src/cache/pending.rs`, **not** ANA-9 §4.3's `<run_id>.jsonl`: the upload half is
already implemented against the two-part name because `run.project_id`, `run.target_box_id` and
`run.started_by` are `NOT NULL` and the line format holds only `session_event` columns. The appender
is a new `pub async fn append_pending(dir, project, run, events)` next to `upload_pending`, so the
naming contract has exactly one owner.

**Retention (`R-HIS-3`).** The driver has no delete path at all: it appends, and the only removal is
ANA-9's retention sweep over `project.settings.retention_days`, which drops whole steps and never
single events. This is stated here because a driver that trimmed its own transcript to stay under a
budget would silently break `R-HIS-1` ("nothing about a run exists only on one box"); trimming
belongs to ANA-5's prompt budget, which trims *inputs*, never the recorded log.

### 4.2 ACP client design (`R-AGT-2`)

**Need.** A v1 ACP client over child-process stdio that is `Send`, runs under `tokio`
rt-multi-thread, can hold a permission responder across a UI round trip, and does not become the
largest thing in the workspace.

**Options.**

| Option | Verdict |
|---|---|
| `agent-client-protocol` 2.1.0 (`cargo info`: version 2.1.0, Apache-2.0, rust-version 1.88.0, repo `agentclientprotocol/rust-sdk`) | **Adopted**, pinned `= "2.1.0"`, no `unstable_*` features. |
| `agent-client-protocol-schema` 1.7.0 with `default-features = false` plus ~500 LOC of hand-rolled newline-JSON transport | Not adopted now; kept as the **named Plan B** with a trigger condition (below). |
| Fully hand-rolled, own types | Rejected. ~19 k LOC of v1 protocol types re-derived by hand on every protocol revision. |
| `agent-client-protocol-tokio` 0.11.1 | Rejected. Stale, pinned to `agent-client-protocol ^0.11.1`; its `AcpAgent`/`Stdio` types were absorbed into the core crate in 2.x. |
| `sacp` 11.0.0 | Rejected. Symposium's ACP *superset*, built on the 0.11-era schema, last updated 2026-03-16 — before ACP 1.0 and 2.0. |
| `boltz-acpx` 0.1.3 | Rejected. 78 downloads, three versions all published on one day in July 2026, no activity since. |

**Adopted design.**

*Version and features.* `agent-client-protocol = { version = "=2.1.0" }`, all `unstable_*` off,
`ProtocolVersion::V1` sent explicitly. The crate version is not the protocol version: 2.1.0 depends
on `agent-client-protocol-schema =1.7.0` and speaks wire protocol 1. Two majors landed in ten weeks
(1.0.0 on 2026-06-24, 2.0.0 on 2026-07-23, 2.1.0 on 2026-09-04), so a floating `"2"` would move
under the workspace; upgrades are scheduled work, not incidental.

*MSRV.* The crate declares `rust-version = "1.88.0"` and has done so since 1.1.0, so no older 2.x or
1.x pin escapes it; with `resolver = "3"` and the workspace's declared `rust-version = "1.85"`,
`cargo add` silently resolves 1.0.1 (June 2026, the dead API) instead of failing. **MOD-2 raises
`workspace.package.rust-version` to `1.88`.** This costs nothing: the declared 1.85 is already
fiction, because the locked `sqlx-core 0.9.0` declares `rust-version = "1.94.0"` and `ratatui
0.30.2` declares `1.88.0`. The honest floor for the workspace as it stands is 1.94; MOD-2 raises the
declaration to 1.88 for ACP and records the sqlx observation in the same commit rather than
pretending the two are unrelated.

*Runtime fit.* The crate has **no tokio dependency** (tokio and tokio-util appear only as
dev-dependencies). Its async stack is the smol family: `async-io 2.6`, `async-process 2.5`,
`blocking 1.7`, `futures 0.3`, `futures-concurrency 7.7`. `htui` therefore runs two reactors —
tokio's mio plus async-io's `polling` thread — and the `blocking` pool. That is workable rather than
free: a probe crate compiled `assert_send(fut)` and `tokio::spawn(fut)` over
`Client.builder()...connect_with(...)` with zero errors under
`#[tokio::main(flavor = "multi_thread", worker_threads = 4)]`, with an `Rc`-holding negative control
correctly failing the `Send` bound, and a runtime probe against a real child process returned a
clean ACP error rather than a missing-reactor panic. **No `LocalSet` is needed**, which the crate
source corroborates: zero `?Send`, `LocalSet`, `spawn_local` or `Rc<` in `src/`, `ConnectionTo<R>`
is `Clone + Send + Sync + 'static`, and `HandleDispatchFrom` *requires* handler futures to be `Send`.

*The bridge: one task per session, channels at the trait boundary.* The SDK's foreground future
borrows the connection (`connect_with(transport, main_fn)`), and `ActiveSession::read_update()` is a
pull API on a mutable borrow, so the connection cannot be handed around. MOD-2 therefore spawns one
`tokio::task` per session that owns the whole `connect_with` future, and `AgentSession` is a plain
handle over channels:

```
AgentSession (Send)                       session task (owns ConnectionTo<Agent> + ActiveSession)
  events:  mpsc::Receiver<DriverEnvelope>  <---- map(SessionUpdate) / MatchDispatch
  commands: mpsc::Sender<SessionCommand>   ----> FollowUp | AnswerPermission | Cancel | SetConfigOption
```

The task alternates between `session.read_update()` and `commands.recv()` in a `select!`. Permission
requests arrive as `RequestPermissionRequest` with a `Responder` that the task parks in a
`HashMap<PermissionRequestId, Responder>`; `answer_permission` resolves it. This is also the answer
to the `!Send` question in general: because only `DriverEnvelope` and `SessionCommand` cross the
boundary, a future transport that is `!Send` needs no trait change — it moves its task onto a
dedicated OS thread running `tokio::runtime::Builder::new_current_thread()` with
`LocalSet::block_on`, and the same two channels carry the traffic in FIFO order. One paragraph of
contingency, no design change.

*Deadlock rule, non-negotiable.* `ConnectionTo::send_request` returns a `SentRequest`, not a future,
with three consumption modes: `on_receiving_result` (callback), `block_task` (await) and `detach`.
The crate documents `block_task` as usable **only** outside the dispatch loop — in the `connect_with`
foreground future or a `ConnectionTo::spawn` task — and awaiting it inside a handler deadlocks.
MOD-2 calls `block_task` only from the session task's foreground future and never from a handler;
the same rule forbids awaiting a Postgres write inside a dispatch callback, which is why the
recorder is a separate consumer of the event channel rather than a handler body.

*Process supervision.* The SDK's own `AcpAgent` spawns the child, sets `CREATE_NO_WINDOW` on
Windows, and calls `process_group(0)` / `kill_process_group(SIGKILL)` **only under `#[cfg(unix)]`;**
the Windows path falls back to a bare `child.kill()`. Since ACP agents ship behind wrapper launchers
(`node`, `npx`, PyInstaller stubs), that orphans the real agent on Windows. MOD-2 therefore spawns
the child itself with `process-wrap` (`CreationFlags(CREATE_NO_WINDOW)` + `JobObject`, which gives
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` without `unsafe` in `htui`'s crates) and hands the SDK a
`ByteStreams` transport over the child's piped stdio — `ByteStreams` is public, is re-exported from
the crate root, implements `ConnectTo<R>`, and takes `futures::io::AsyncRead`/`AsyncWrite`, so
`tokio_util::compat` adapts the tokio child handles. `AcpAgent` remains the fallback path when the
job object cannot be created (`ERROR_ACCESS_DENIED` on some CI runners), where MOD-2 proceeds
without it, as Cargo does.

*Plan B trigger.* Drop to `agent-client-protocol-schema` with `default-features = false` and a
hand-rolled tokio transport if any of: the two-reactor arrangement shows measurable cost or a
shutdown-ordering fault in the TUI; the SDK's cadence forces more than one breaking migration per
quarter; or a future release re-introduces `!Send` requirements. The fallback is genuinely cheap
because framing is newline-delimited JSON (one `BufReader::lines()` loop plus `writeln!`) and the
schema crate is runtime-agnostic with no async dependency at all — but it does not escape the 1.88
MSRV, which the schema crate also declares, and it must branch on object-vs-array per line because a
line may hold a JSON-RPC batch.

### 4.3 Permission and edit-proposal handling (`R-AGT-2`, `R-SEC-3`, `R-TUI-6`, `R-MCP-4`)

**Need.** `R-AGT-2` requires that `htui` handle tool permissions and present edit proposals.
ANA-9 §4.3 fixes the two payloads: `permission_request { request_id, options[] { id, label, kind } }`
and `permission_answer { request_id, option_id, by ∈ user|policy }`, plus
`edit_proposal { path, diff (unified), accepted: bool|null }`.

**Options.**

| Option | Verdict |
|---|---|
| Do not advertise `fs.writeTextFile`; let the agent write through its own tools | Rejected. `htui` would see the edit only as a post-hoc tool call with no diff, which is exactly the CLI transport's deficiency; advertising it is what buys the proposal. |
| Advertise `fs/write_text_file` and **block** each write on a user decision | Rejected. It double-prompts (the adapter already asked via `session/request_permission` for the edit tool) and parks a protocol request on UI latency for every file. |
| Advertise `fs/write_text_file`, **intercept**: read the current text, synthesize the unified diff, persist `edit_proposal`, then apply | **Adopted.** One row per write with a real diff, no second prompt, and the gate stays where the protocol puts it. |
| Emulate "always allow" inside `htui` and stop forwarding `allow_always` | Rejected. The agent owns its own durable grant; emulating it would desynchronise the two. |
| Answer from a policy table before prompting, recording `by = "policy"` | **Adopted**, as the first stage of one pipeline. |

**Adopted design.**

*The permission pipeline.* Every `session/request_permission` runs through three stages:

1. **Policy.** `agent.settings.permission.rules[]` is evaluated in order against the tool call
   (`tool_kind`, tool name, first path/command argument). A match answers immediately and writes
   `permission_answer { request_id, option_id, by: "policy" }`. `R-MCP-4` is one seeded rule: when
   `command_run` is exposed for the phase, direct shell calls matching the command classes are
   rejected with a `reject_once` option so the agent is pushed back to the MCP tool.
2. **Remembered.** `agent.settings.permission.remembered[]` holds the `_always` choices the user has
   made in `htui`, keyed by `{ agent, tool_name | tool_kind, path_prefix | command_prefix }`. A hit
   answers with `by: "policy"` too — the distinction ANA-9 draws is "not typed by a human now", and
   the remembered rule records who added it and when.
3. **Ask.** Otherwise the chat tab renders the options inline (`R-TUI-6`), the responder stays
   parked, and the user's choice writes `by: "user"`. Selecting an `_always` option does two things:
   it forwards that option id to the agent (the agent's own durable grant, which `htui` does not
   emulate) **and** appends a `remembered` entry scoped to the agent.

The `permission_request` row carries the `toolCallId` in `session_event.tool_call_id`, so the
partial index `idx_session_event_tool` joins request, answer, call and result. The client's option
decoder accepts all four `PermissionOptionKind` values even though the claude adapter currently
offers only three; the kind is a UI hint, and the `optionId` is what is sent back.

*Cancellation.* On `cancel`, every parked responder is answered `{"outcome":"cancelled"}` before the
`session/cancel` notification is considered complete — the spec makes that a MUST — and each is
recorded as `permission_answer { option_id: null, by: "htui" }`... except that `by` is a closed
`user|policy` vocabulary in ANA-9 §4.3. MOD-2 therefore writes `by: "policy"` with
`option_id: null` and a `cancelled: true` key added by the driver (adding keys is permitted;
renaming is not).

*Edit proposals, two sources normalised into one kind.*

- `fs/write_text_file { sessionId, path, content }` — `htui` reads the current on-disk text (empty
  when absent, since the client MUST create the file), computes a unified diff with `similar 3.2.0`,
  writes `edit_proposal { path, diff, accepted }` keyed by the enclosing tool call, then performs the
  write and answers the request.
- `ToolCallContent::Diff { path, oldText (null for a new file), newText }` on a `tool_call` or
  `tool_call_update` — the same unified diff is synthesized from the pair without touching the disk.

Deduplication rule: a `(tool_call_id, path)` that already produced an `edit_proposal` row in this
step is *updated in the UI* and not written twice; the row's `accepted` is filled from the paired
permission answer when there is one (`true` for an allow, `false` for a reject) and defaults to
`true` for a write that reached the filesystem with no request attached. `accepted` stays `null`
only while a request is parked.

*Terminal-backed tool content.* `ToolCallContent::Terminal { terminalId }` has no ANA-9 kind. MOD-2
does **not** advertise the `terminal` capability in v1, so the case does not arise; if a future
agent requires it, the captured output lands in `tool_result.output` with a `terminal_id` key added
to `tool_call`. Recorded here so the gap is a decision, not an oversight.

*Tool-call terminal states.* ACP has no `cancelled` or `rejected` wire status, and ANA-9 §4.3's
`tool_result.status` vocabulary is `completed|failed`. A denied or cancelled call can have **no**
`tool_result` row at all, which a naive replay renderer would leave spinning. Rule for MOD-2: on a
reject and on cancel, the driver synthesizes a `tool_result { status: "failed", output: null }` row
carrying an added `terminal_reason: "rejected" | "cancelled"` key. Vocabulary unchanged, replay
total.

*What the CLI fallback can and cannot do here.* Nothing in the claude stream-json output is a
permission request, and nothing is an edit proposal: the CLI applies edits itself and surfaces them
only as `Edit`/`Write` tool calls after the fact. The one interception route is
`--permission-prompt-tool <mcp_tool>`, which points the CLI at an MCP tool that answers prompts —
`htui` already plans an MCP server (`R-MCP-1`), so MOD-11 can expose one extra tool and the CLI
transport gains real `permission_request` / `permission_answer` events at that point. Until then the
CLI transport reports `DriverCaps { permission_requests: false, edit_proposals: false, plans: false }`,
runs with the `--permission-mode` configured in `agent.settings.cli`, and synthesizes
`permission_answer { by: "policy" }` rows from `result.permission_denials[]`. The chat tab shows a
banner naming the missing capabilities rather than silently offering buttons that do nothing, and
`R-ORCH` gates that require an inline approval must not be scheduled onto a CLI-only agent.

### 4.4 Reaching `claude` over ACP (`R-AGT-2`, `R-AGT-3`, `R-AGT-4`)

**Need.** A concrete launch line, model selection, session resume and usage capture for `claude`
over ACP, plus the CLI mapping for boxes where the adapter is missing.

**Options.**

| Option | Verdict |
|---|---|
| `claude --acp` (the demo fixture's guess) | Rejected: no such flag. `claude --version` → `2.1.261 (Claude Code)`; `claude --help` is 304 lines with zero case-insensitive matches for `acp`, `agent-client` or `json-rpc`, and no `acp` subcommand. |
| `@zed-industries/claude-code-acp` | Rejected. Deprecated and frozen at 0.16.2 with the npm notice "This package has been renamed to @agentclientprotocol/claude-agent-acp." |
| `@agentclientprotocol/claude-agent-acp`, spawned **by name** (`claude-agent-acp`) | Rejected on Windows. `which claude-agent-acp` resolves to a 43-byte Bourne-Again shell script with a 28-byte `.cmd` sibling, and a Rust probe of `Command::new("claude-agent-acp")` returns `program not found`, while `Command::new("claude")`, `"agy"`, `"node"` and `"npx"` all succeed on the same box. |
| `@agentclientprotocol/claude-agent-acp`, spawned as `node <dist/index.js>` | **Adopted.** Verified: `node ".../claude-agent-acp/dist/index.js" --version` → `0.55.0`. No shim, no `PATHEXT`, no `cmd.exe` quoting, and the interpreter is pinned. |
| `npx -y @agentclientprotocol/claude-agent-acp@<pin>` | **Adopted as the fallback** when the package is not installed locally. `npx` is directly spawnable here (11.13.0), but the version must be pinned: the registry pins 0.75.0 while an unpinned `npx` follows `latest`. |

**Adopted design.**

*Adapter identity.* ACP registry id `claude-acp`, name "Claude Agent", version `0.75.0`,
distribution `{"npx": {"package": "@agentclientprotocol/claude-agent-acp@0.75.0"}}`, authors
Anthropic / Zed Industries / JetBrains. The installed copy on this box is 0.55.0, `engines.node >=
22` (local node is v24.18.0). The adapter wraps the Claude Agent SDK; it does not drive the `claude`
CLI unless told to, which is why the launch pins the CLI explicitly.

*Launch (resolved form on this box).*

```
command: <node>
args:    [ "<prefix>/@agentclientprotocol/claude-agent-acp/dist/index.js" ]
env:     { "CLAUDE_CODE_EXECUTABLE": "<resolved claude>" }
```

`CLAUDE_CODE_EXECUTABLE` is the first thing the adapter's CLI resolution consults; without it the
adapter falls back to a native binary shipped as an optional dependency of the Agent SDK, i.e. it
may drive a *different* `claude` build than the one autodiscovery recorded in `agent_box.version`.
`htui` always sets it. The adapter redirects `console.log`/`info`/`warn`/`debug` to stderr, so stdout
is pure protocol and stderr goes to the run log. Other env knobs the bundle reads and `htui` leaves
alone unless configured: `CLAUDE_CONFIG_DIR`, `ANTHROPIC_MODEL`.

*Handshake, as observed live against 0.55.0.* `protocolVersion: 1`; `agentCapabilities` with
`loadSession: true`, `sessionCapabilities { additionalDirectories, close, delete, fork, list, resume }`
(empty objects, not booleans), `promptCapabilities { image, embeddedContext }`,
`mcpCapabilities { http, sse }`, `auth { logout: {} }`; `agentInfo { name, title, version }`;
`authMethods: []` because this box holds a subscription login. An unauthenticated box lists methods
here, and the registry's daily probe matrix records `authMethods: ["terminal"]` for this adapter —
the `terminal` auth type means the client relaunches the configured invocation interactively and
then reconnects and re-initializes. MOD-2 surfaces that as a Settings action rather than attempting
it inline.

*Model selection.* `session/set_config_option { sessionId, configId, value }`, keyed on the option
`id`, never on `category`; the response is the complete option list, and `config_option_update` can
push a new complete list at any time (a model fallback under rate limiting is the documented
example, and `htui` surfaces it as an `other` row). The installed adapter bundle contains
`configOptions` (39 occurrences), `set_config_option` and a `model_config` identifier, consistent
with model choice being a runtime config option rather than a launch argument. **Unverified - MOD-2
must confirm** the exact option ids and value vocabulary from a live `session/new`, and must
tolerate an agent that offers no model option at all (`SessionSpec.model` is then advisory and the
step records the agent's own default).

*Session load and resume.* `htui` replays transcripts from its own store (`R-HIS-2`), so
`session/load` is not the replay path. It is the *continuation* path: when a later step resumes an
earlier one, the driver calls `load_session`/`resume_session` with the agent-side id. That id has no
column in ANA-9, and rather than migrate for it, MOD-2 records a session banner as the step's first
`other` row: `{ "update": "session_started", "session_id": ..., "protocol_version": 1,
"agent_name": ..., "agent_version": ..., "models": [...] }`. Resuming is a query for that row.
Because `session/load` replays the whole conversation as `session/update` notifications, the replay
decoder is the live decoder; `user_message_chunk` arrives during that replay and is dropped rather
than manufactured into a duplicate `follow_up` (§6).

*Usage capture.* `usage_update` carries `used`/`size` (context-window tokens) and
`cost { amount, currency }`; the adapter additionally forwards the whole Claude rate-limit blob
under `_meta["_claude/rateLimit"]`, which is the subscription quota source of §7. **Unverified -
MOD-2 must confirm** whether 0.75.0 still guards both emissions on a non-null last assistant usage —
0.55.0's bundle does, meaning a rate-limit event arriving before any assistant usage is dropped, so
`htui` must not assume a quota update every turn.

*The CLI fallback for `claude`.* Flags verified present in `claude --help` on 2.1.261:
`--print/-p`, `--output-format`, `--input-format`, `--include-partial-messages`, `--verbose`,
`--model`, `--permission-mode`, `--permission-prompts`, `--permission-prompt-tool`, `--allowedTools`,
`--disallowedTools`, `--add-dir`, `--mcp-config`, `--append-system-prompt`, `--session-id`,
`--resume`, `--continue`, `--fork-session`, `--forward-subagent-text`, `--max-budget-usd`,
`--json-schema`, `--no-session-persistence`, `--bare`. `--max-turns` is **hidden from `--help` but
documented and enforced** (a turn limit produces `subtype: "error_max_turns"`), and
`--permission-prompt-tool` likewise survives in help only as a cross-reference inside the
`--permission-prompts` description. The invocation:

```
claude -p <prompt> --output-format stream-json --input-format stream-json --verbose --bare \
       --model <model> --permission-mode <mode> --session-id <uuid> \
       [--add-dir <dir>]... [--max-budget-usd <cap>] [--allowedTools ...] [--disallowedTools ...]
```

`--input-format stream-json` is what makes follow-ups possible without respawning: `htui` writes one
NDJSON user message per line to stdin and closes stdin to end. `--bare` suppresses hook events and
is the documented recommendation for scripted callers. The envelope sequence is *not* "init is line
0": hook events precede `system/init` unless `--bare` is passed. `system/init` carries a
`capabilities: string[]` array intended for feature detection in preference to version parsing. The
join key from a tool result back to its call is `tool_use_id`, which becomes
`session_event.tool_call_id`; subagent messages carry `parent_tool_use_id` when
`--forward-subagent-text` is set. **Unverified - MOD-2 must confirm** the cancellation semantics
(SIGINT ends the turn cleanly while SIGTERM is reported to leave the turn unfinished with exit 143)
by probe, since the driver's `cancel` depends on it.

*Registry consequence.* `claude` is **one** row with `transport: 'acp'`. The CLI path is not a second
agent row but a documented degradation recorded in `agent.settings.cli`, selected per box by
autodiscovery when node ≥ 22 or the adapter is absent; `agent_box.probe.transport` records which one
that box will actually use. That keeps `R-AGT-5` honest — one row, one adapter — while `R-AGT-4`'s
`transport` column still names the primary transport.

### 4.5 `agy`: `acp` | `cli` | both (`R-AGT-4`)

**Need.** `R-AGT-4` states plainly that "support for `agy` over ACP is unverified and must be settled
by ANA-4". The demo fixture encodes the pessimistic guess.

**Options.**

| Option | Verdict |
|---|---|
| `agy` CLI with an ACP flag or subcommand | Rejected. `agy --help` on 1.1.26 lists subcommands `agent, agents, changelog, help, install, mcp, mic-serve, models, plugin, plugins, remote-control, update` — no `acp`, and no `run` either, so the fixture's `["agy","run"]` would fail immediately. A string sweep of the 189 MB Go binary finds no ACP wire method and no `agent-client-protocol`; its JSON-RPC symbols belong to the Go MCP SDK and an LSP client. |
| CLI stream adapter over `agy --output-format stream-json` | Rejected as the primary. It cannot carry a permission request or an edit proposal, and has no thought or plan channel — losing five of the fourteen kinds, including the two `R-AGT-2` names explicitly. |
| Google's first-party ACP server `agy_acp_server` | **Adopted.** `transport: 'acp'`. |
| Both, with runtime selection | Rejected for v1. Two adapters for one agent contradicts `R-AGT-5`'s "at most one"; the CLI contract is documented below as the contingency only. |

**Adopted design.**

*What was verified on this box.* `where agy` → `C:\Users\luigi\AppData\Local\agy\bin\agy.exe`;
`agy --version` → the bare string `1.1.26` (no product name, no `v`); the subcommand list above; and
`agy_acp_server.exe` (297 MB) plus `localharness_external.exe` (122 MB) already installed by
CLion 2026.2 at
`%LOCALAPPDATA%\JetBrains\CLion2026.2\acp-agents\antigravity-acp\1.0.0\`. A Rust spawn probe shows
`agy` is directly spawnable by name.

*What was verified on the web.* The ACP registry (`registry.json`, schema version 1.0.0, 39 agents,
re-fetched 2026-09-05) carries id `antigravity-acp`, name "Google Antigravity", version 1.1.1,
authors `["Google LLC"]`, license proprietary, distribution `binary` over five platforms —
`darwin-aarch64`, `linux-x86_64`, `linux-aarch64`, `windows-x86_64`, `windows-aarch64` (there is no
`darwin-x86_64`). The launched command is `./agy_acp_server.par` on darwin and linux and
`./agy_acp_server.exe` on Windows; **only the two Linux entries carry args, and that array is
exactly `["--uid="]`** — one token with an empty value, passed literally.

*What the handshake reports.* A live stdio `initialize` against the local binary returns
`protocolVersion: 1`; `agentCapabilities { loadSession: true, promptCapabilities { image, audio,
embeddedContext }, mcpCapabilities { http, sse }, sessionCapabilities { list: {}, resume: {} },
auth { logout: {} } }`; four `authMethods` (`oauth-personal`, `oauth-business`, `gemini-api-key`,
`agent-platform`); and `agentInfo { name: "antigravity-acp", title: "Google Antigravity",
version: "agy_acp_server_20260818_01_RC01" }`. Two operational facts fall out. First, the server
**echoes** whatever `protocolVersion` the client sends rather than negotiating down, so `htui` must
pin 1 and must not trust the echo as evidence of support. Second, the version strings disagree —
registry semver `1.1.1` versus the build tag in `agentInfo.version` — so autodiscovery records the
handshake value and treats the registry value as a download coordinate only.

*Configuration and auth are out of band.* The server resolves `$GEMINI_HOME` (default `~/.gemini`)
and reads its own `<GEMINI_HOME>/antigravity-acp/` directory — `settings.json`, `acp_token.json`,
`brain/`, `conversations/` — a sibling of, and disjoint from, the CLI's
`<GEMINI_HOME>/antigravity-cli/`. A session started over ACP is therefore not resumable through
`agy --conversation`, and vice versa. `htui` cannot log in non-interactively; a probe that gets a
valid `initialize` with a non-empty `authMethods` and no token is recorded as
*installed-but-unauthenticated* and the agent is left disabled on that box until the user
authenticates through the vendor's own flow.

*The exact adapter parse contract, if the CLI path is ever built.* Documented so a future MOD does
not re-derive it, and explicitly **not** implemented in v1. `agy --output-format stream-json` emits
NDJSON with the envelope `{"event": "<name>", "<name>": { ... }}`: exactly one `init`, any number of
`step_update`, exactly one terminal `result`. `init` carries `cwd`, `tools[]`, `permission_mode`
(`request-review` | `always-proceed`), `model?`, `agent?`. `step_update` carries
`conversation_id`, `step_index` (0-based), `state`, `step_type ∈ user_input | agent_response | tool
| checkpoint`, `tool_name?`, `text_delta?`, `duration_seconds?`, `usage?`, `tool_info?`,
`subagent_info?`. `state` is **not** just `ACTIVE|DONE`: a denied tool transitions to `ERROR`
carrying `tool_info.error = { type: "TOOL_ERROR", message: "permission check failed ..." }`, and the
terminal `result` then carries `denied_actions: [{ action, display_name }]` while still reporting
`status: "SUCCESS"` and exiting 0. So a driver must parse denial from the stream and must never
scrape stderr for it. `result` carries `conversation_id, status ∈ SUCCESS|ERROR|CANCELED|INTERRUPTED
|INVALID|WAITING|RUNNING, response, error?, duration_seconds, num_turns, structured_output?,
usage { input_tokens, output_tokens, thinking_tokens, cache_read_tokens, total_tokens }` — note no
`cache_write_tokens` and no cost field. Multi-turn is `--input-format stream-json` reading
`{"event":"user","message":{"content":"..."}}` per line, with `text` the only supported block type;
a `control_request`/`control_response` line produces an `ERROR` result and exit 2, and slash commands
fed this way are rejected the same way. Resume is `--continue`/`-c` or `--conversation <id>`.

*Risk carried forward.* Community adapters flag driving Antigravity from a non-Google client as a
Terms-of-Service violation for personal accounts, with API-key auth as the recommended mitigation.
The registry entry is Google-authored and Google-hosted, which argues the ACP path is sanctioned,
but the question is not resolved from a primary Google source. §10 carries it as a named risk, and
the Settings agent section states the account modes plainly rather than burying it.

### 4.6 Autodiscovery probes per agent (`R-AGT-6`)

**Need.** On box registration and on demand, probe `PATH` for known agent binaries **and ACP
adapters**, record version, mark enabled on that box, and allow manual entries. Two of the four
artifacts in scope are not on `PATH` at all, and one cannot be spawned by name on Windows.

**Options.**

| Option | Verdict |
|---|---|
| `PATH` scan for a name list only | Rejected alone. It finds `claude` and `agy` and misses both ACP servers, which is precisely the pair that decides transport. |
| Heuristic scan for executables ending in `-acp` / `_acp` (Martty's approach) | Rejected as primary, kept as a hint. It would find `claude-agent-acp` — the one artifact `htui` must *not* spawn by name — and miss `agy_acp_server.exe`, which is not on `PATH`. |
| Download every agent from the ACP registry and manage the install (Zed's approach) | Deferred. It fixes version skew and the not-on-`PATH` problem, but MOD-2 should not become a package manager; `agent.launch.discovery` reserves the shape and MOD-7 or a later MOD can implement it. |
| Two-tier probe: a cheap `--version` tier plus a definitive `initialize` handshake tier, both driven by a per-agent recipe in `agent.launch.discovery` | **Adopted.** |

**Adopted probe table.**

| Artifact | Names (per OS) | Version command | Expected output | Prerequisites |
|---|---|---|---|---|
| `claude` CLI | `claude` / `claude.exe` on `PATH` | `--version` | `^(\d+\.\d+\.\d+) \(Claude Code\)$` → `2.1.261 (Claude Code)`; keep the semver, drop the parenthetical | none; directly spawnable (native PE32+ here) |
| `node` | `node` / `node.exe` | `--version` | `^v(\d+\.\d+\.\d+)$` → `v24.18.0`; require major ≥ 22 | gates the claude ACP path |
| `claude-agent-acp` | **never by name.** Resolve `<npm-or-volta prefix>/@agentclientprotocol/claude-agent-acp/dist/index.js`; fall back to `npx -y @agentclientprotocol/claude-agent-acp@<pin>` | `node <entry> --version` | bare `^(\d+\.\d+\.\d+)$` → `0.55.0` (no product name) | `node ≥ 22`; on Windows the `PATH` entry is a shell script with a `.cmd` sibling and `Command::new` fails with `program not found` |
| `agy` CLI | `agy` / `agy.exe` (installed under `%LOCALAPPDATA%\agy\bin` here) | `--version` | bare `^(\d+\.\d+\.\d+)$` → `1.1.26`; undocumented, so tolerate an optional `v` and trailing text, and treat a parse failure as *present, version unknown* | none |
| `agy_acp_server` | not on `PATH`. Glob `%LOCALAPPDATA%\JetBrains\*\acp-agents\antigravity-acp\*\agy_acp_server.exe`; `./agy_acp_server.par` under an `htui`-managed unzip dir on darwin/linux; plus an explicitly configured path | handshake only (`--help` reports abseil flags, not a version) | `agentInfo.version` from `initialize`, e.g. `agy_acp_server_20260818_01_RC01` | keep the whole unzipped directory: `localharness_external.exe` ships beside it and may be required |
| any ACP agent (tier 2) | resolved `{command, args}` | spawn → `initialize { protocolVersion: 1 }` → read → close | `agentInfo{name,version}`, `agentCapabilities`, `authMethods`, negotiated `protocolVersion` | costs no tokens; the only proof the binary actually runs |

*What is written to `agent_box`.* `enabled`, `version` (handshake value when tier 2 ran, else the
`--version` value), `path` (the resolved command), `probed_at`, and a new `probe JSONB` column
(migration `0002`, §9) holding the full snapshot:

```json
{
  "transport": "acp",
  "resolved": { "command": "C:/…/node.exe",
                "args": ["C:/…/claude-agent-acp/dist/index.js"],
                "env": { "CLAUDE_CODE_EXECUTABLE": "C:/…/claude" } },
  "tools": { "node": "24.18.0", "claude": "2.1.261", "claude_agent_acp": "0.55.0" },
  "handshake": { "at": "2026-09-05T…Z", "protocol_version": 1,
                 "agent_name": "@agentclientprotocol/claude-agent-acp",
                 "agent_version": "0.55.0",
                 "capabilities": { "load_session": true, "session": ["resume","list","close"] },
                 "auth_methods": [] },
  "status": "ready",              // ready | unauthenticated | missing | failed
  "stderr_tail": null
}
```

`probe.resolved` is what `AgentDriver::start` actually spawns; `agent.launch` holds the
box-independent recipe with `${…}` placeholders (§5). That split is what lets one registry row serve
boxes with different install layouts without a per-box `agent` row.

*On-demand versus on-registration.* Tier 1 (`PATH` + `--version`) runs on box registration
(`MOD-7`'s hook), on `Settings > Refresh agents`, and whenever a session fails to spawn. Tier 2 (the
handshake) runs on registration, on demand, and lazily before the first session of the day when
`probed_at` is older than 24 h. Nothing probes on every run: `agy` self-updates in place via
`agy update`, so a recorded version can go stale without `htui` doing anything, and the failure mode
that matters (spawn fails) already triggers a re-probe. Manual entries are supported by writing
`agent_box.path` and `probe.resolved` by hand in the Settings tab and setting
`probe.status = "ready"`; a manual entry is never overwritten by a probe that finds nothing.

*Windows shim handling, as a rule.* Resolve with the `which` crate rather than trusting
`Command::new` (`std::process::Command` does not consult `PATHEXT`, and handing a `.cmd` to
`CreateProcess` yields `os error 193`); if the resolved file is not a native executable, resolve one
level deeper to the interpreter and its entry script and spawn that; never emit shell metacharacters
into a `.cmd` path; always set `CREATE_NO_WINDOW` (a TUI must not flash console windows) and always
assign the child to a job object.

---

## 5. Registry JSONB shapes

Both shapes are fixed here; `agent.launch` replaces ANA-9 §5.7's placeholder comment
`{argv: [...], env: {...}}`, and the DDL comment on `agent.name` ("`'claude','agy' seeded`") becomes
true for the first time when MOD-2 runs the seed.

### 5.1 `agent.launch` (`JSONB NOT NULL`)

```
{
  "command": string,                  // may be a "${tool}" placeholder
  "args":    [string],                // each element may contain "${tool}" placeholders
  "env":     { string: string },      // values may contain "${tool}" placeholders; secrets never here
  "discovery": {                      // optional; absent = command/args are literal
    "tools": { "<name>": <ToolProbe> },
    "handshake": bool                 // run the tier-2 initialize probe (default true for acp)
  }
}
```

`{command, args, env}` rather than `{argv, env}` for three reasons: it is what
`std::process::Command` wants; it is what the ACP registry, Zed and JetBrains all use for agent
definitions; and it is field-for-field the SDK's own `AcpAgentConfig { command: PathBuf, args:
Vec<String>, env: BTreeMap<String,String> }`, so a row deserializes into the SDK type with no
adapter layer. `${name}` placeholders are resolved from `agent_box.probe.tools`; a row with no
placeholders needs no probe.

`ToolProbe` is one of:

```
{ "kind": "path",  "names": [string], "version": { "args": [string], "pattern": regex, "min": semver? } }
{ "kind": "node_package", "package": string, "entry": string, "pinned": string,
  "fallback": { "command": string, "args": [string] } }
{ "kind": "glob",  "patterns": [string],
  "platform": { "<os>-<arch>": { "patterns": [string], "args": [string]? } } }
```

A `glob` probe's per-platform `args` are **appended** to `agent.launch.args` when that platform
matches — the mechanism that carries the ACP registry's Linux-only `["--uid="]` for
`agy_acp_server` without giving `agy` two registry rows. Platform ids follow the registry's own
vocabulary: `darwin-aarch64`, `linux-x86_64`, `linux-aarch64`, `windows-x86_64`, `windows-aarch64`.

### 5.2 `agent.settings` (`JSONB NOT NULL DEFAULT '{}'`)

```
{
  "acp": {
    "protocol_version": 1,
    "client_capabilities": { "fs_read": true, "fs_write": true, "terminal": false,
                             "elicitation": false },
    "model_config_id": string|null,       // the configId used for model selection; null = discover
    "session": { "load": true, "resume": true }
  },
  "cli": {                                 // present when transport = 'cli', or as the degraded path
    "stream": "claude_stream_json" | "agy_stream_json",
    "permission_mode": string,
    "extra_args": [string]
  },
  "permission": {
    "default": "ask" | "allow" | "deny",
    "rules": [ { "match": { "tool_kind": string?, "tool_name": string?,
                            "path_prefix": string?, "command_prefix": string? },
                 "answer": "allow_once"|"allow_always"|"reject_once"|"reject_always",
                 "reason": string } ],
    "remembered": [ { "match": { … }, "option_kind": string,
                      "added_at": timestamp, "added_by": "user" } ]
  },
  "quota": { "source": "acp_meta_rate_limit" | "cli_rate_limit_event" | "cli_status_line" | "none" },
  "usage": { "scope": "model_usage" | "session_context" }
}
```

Every key is optional with a documented default, because the column has `DEFAULT '{}'` and a row
written by hand in the Settings tab must remain valid. `usage.scope` describes the CLI path only:
over ACP the stable report is always context occupancy plus cost, so the four token fields of the
`usage` payload are null regardless of this key (§7).

### 5.3 Seed rows MOD-2 inserts

`claude` — one row, `transport: 'acp'`, CLI recorded as the degraded path:

```json
{
  "name": "claude",
  "transport": "acp",
  "billing": "subscription",
  "models": [],
  "default_model": null,
  "launch": {
    "command": "${node}",
    "args": ["${claude_agent_acp}"],
    "env": { "CLAUDE_CODE_EXECUTABLE": "${claude}" },
    "discovery": {
      "tools": {
        "node":   { "kind": "path", "names": ["node"],
                    "version": { "args": ["--version"], "pattern": "^v(\\d+\\.\\d+\\.\\d+)$",
                                 "min": "22.0.0" } },
        "claude": { "kind": "path", "names": ["claude"],
                    "version": { "args": ["--version"],
                                 "pattern": "^(\\d+\\.\\d+\\.\\d+) \\(Claude Code\\)$" } },
        "npx":    { "kind": "path", "names": ["npx"],
                    "version": { "args": ["--version"], "pattern": "^(\\d+\\.\\d+\\.\\d+)$" } },
        "claude_agent_acp": { "kind": "node_package",
                              "package": "@agentclientprotocol/claude-agent-acp",
                              "entry": "dist/index.js",
                              "pinned": "0.75.0",
                              "fallback": { "command": "${npx}",
                                            "args": ["-y",
                                              "@agentclientprotocol/claude-agent-acp@0.75.0"] } }
      },
      "handshake": true
    }
  },
  "settings": {
    "acp": { "protocol_version": 1,
             "client_capabilities": { "fs_read": true, "fs_write": true, "terminal": false,
                                      "elicitation": false },
             "model_config_id": null,
             "session": { "load": true, "resume": true } },
    "cli": { "stream": "claude_stream_json", "permission_mode": "acceptEdits",
             "extra_args": ["--bare"] },
    "permission": { "default": "ask", "rules": [], "remembered": [] },
    "quota": { "source": "acp_meta_rate_limit" },
    "usage": { "scope": "model_usage" }
  }
}
```

`agy` — one row, `transport: 'acp'`, no CLI block because the CLI path is not built:

```json
{
  "name": "agy",
  "transport": "acp",
  "billing": "subscription",
  "models": [],
  "default_model": null,
  "launch": {
    "command": "${agy_acp_server}",
    "args": [],
    "env": {},
    "discovery": {
      "tools": {
        "agy": { "kind": "path", "names": ["agy"],
                 "version": { "args": ["--version"], "pattern": "^v?(\\d+\\.\\d+\\.\\d+)" } },
        "agy_acp_server": {
          "kind": "glob",
          "patterns": [],
          "platform": {
            "windows-x86_64": { "patterns": [
              "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe",
              "%LOCALAPPDATA%/htui/agents/antigravity-acp/*/agy_acp_server.exe" ] },
            "windows-aarch64": { "patterns": [
              "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe" ] },
            "linux-x86_64":   { "patterns": ["~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"],
                                "args": ["--uid="] },
            "linux-aarch64":  { "patterns": ["~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"],
                                "args": ["--uid="] },
            "darwin-aarch64": { "patterns": ["~/Library/Application Support/htui/agents/antigravity-acp/*/agy_acp_server.par"] }
          }
        }
      },
      "handshake": true
    }
  },
  "settings": {
    "acp": { "protocol_version": 1,
             "client_capabilities": { "fs_read": true, "fs_write": true, "terminal": false,
                                      "elicitation": false },
             "model_config_id": null,
             "session": { "load": true, "resume": true } },
    "permission": { "default": "ask", "rules": [], "remembered": [] },
    "quota": { "source": "none" },
    "usage": { "scope": "session_context" }
  }
}
```

Three notes on the seed. **Models are empty and `default_model` is null on both rows.** ACP delivers
the model list as session config options at `session/new`, so seeding a guessed list would encode a
value that goes stale on every vendor release; the first successful handshake fills
`agent.models` from the option's `options[]`. **`billing` is `subscription` for both**: `claude` on
this box authenticates with a subscription login, and Antigravity is plan-quota based with an
opt-in credit overage (`useG1Credits`, "uses personal AI credits for model calls once plan quotas
are exhausted"). Whether those credits meter per token is not documented, so `per_token` would be a
guess; a box using API-key auth flips the row in Settings. **The demo fixture
(`crates/htui-core/src/fixtures.rs`) is corrected to match** — `claude` loses `["claude","--acp"]`,
`agy` moves from `cli`/`per_token`/`["agy","run"]` to `acp`/`subscription` — because the fixture is
what `MemStore::demo()` and the conformance suite read, and a fixture that cannot launch is a trap
for MOD-2's own tests.

---

## 6. Event mapping tables

Every `EventKind` in `crates/htui-core/src/model/event.rs` appears in every table, or is marked *not
produced by this transport*. `prompt`, `follow_up` and `permission_answer` are authored by `htui`
itself on every transport, so they are marked *htui-authored* rather than mapped.

### 6.1 ACP → `EventKind` (used by `claude` and by `agy`)

| ACP source | `EventKind` | role | payload keys and rule |
|---|---|---|---|
| — (ANA-5's assembled prompt) | `prompt` | htui | htui-authored: `text`, `digest`, `sections[]`; always `seq = 0`, `turn = 0` |
| — (chat tab input) | `follow_up` | user | htui-authored: `text`; increments `turn` |
| `agent_message_chunk` | `assistant_text` | agent | `text` coalesced per §4.1; flush on kind change, `messageId` change, `Done`, 16 KiB |
| `agent_thought_chunk` | `thought` | agent | same rule |
| `tool_call` | `tool_call` | agent | `title`, `tool_kind` (from `kind`, ten-value vocabulary incl. `switch_mode`), `input` (from `rawInput`), `locations[]`; `tool_call_id` = `toolCallId` |
| `tool_call_update` with `status ∈ completed|failed` | `tool_result` | agent | `status`, `output` (scrubbed, from `content[]`/`rawOutput`), `locations[]`; a rejected or cancelled call gets a synthesized `failed` row with an added `terminal_reason` |
| `tool_call` / `tool_call_update` `content[]` of `type: "diff"`, and client `fs/write_text_file` | `edit_proposal` | agent | `path`, `diff` (unified, synthesized from `oldText`/`newText` or from disk), `accepted`; deduped per `(tool_call_id, path)` |
| `session/request_permission` (a request, not an update) | `permission_request` | agent | `request_id`, `options[] { id, label, kind }`; `tool_call_id` from `toolCall.toolCallId` |
| the response `htui` sends | `permission_answer` | user or htui | htui-authored: `request_id`, `option_id`, `by`; cancellation writes `option_id: null` with an added `cancelled: true` |
| `plan` | `plan` | agent | `entries[] { content, status, priority }`; always the complete list — replace, never append |
| `usage_update` | `usage` | agent | see §7 for the field reconciliation |
| JSON-RPC error, `-32800`, `stopReason: "refusal"`, transport close | `error` | agent or htui | `code`, `message` |
| `session/prompt` response `stopReason` | `done` | agent | `stop_reason ∈ end_turn|max_tokens|max_turn_requests|refusal|cancelled` — five values, not three |
| `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update`, every unstable or unknown variant, and the session banner | `other` | agent | `update` plus the verbatim body |
| `user_message_chunk` | *not produced by this transport* | — | it appears only during `session/load` replay, where it would duplicate a `prompt`/`follow_up` row `htui` already stored; dropped |

The claude adapter also emits `subagent_spawned`, `subagent_state_update`, `async_task_spawned`,
`async_task_progress` and `async_task_state_update`, which are **not in the schema**. They land in
`other` unchanged, which is exactly why the decoder is written against a non-exhaustive enum with a
wildcard arm rather than a closed match.

### 6.2 `claude -p --output-format stream-json` → `EventKind`

| Stream source | `EventKind` | Rule |
|---|---|---|
| ANA-5's prompt (argv or the first stdin line) | `prompt` | htui-authored |
| a stdin NDJSON user message (`--input-format stream-json`) | `follow_up` | htui-authored |
| `assistant` → `message.content[].type == "text"`; deltas via `stream_event` `text_delta` | `assistant_text` | coalesced per §4.1, keyed on `message.id` |
| `assistant` → `content[].type == "thinking"` | `thought` | **Unverified - MOD-2 must confirm** the block and delta shape: thinking was off in the captured runs, though `usage.output_tokens_details.thinking_tokens` proves the channel exists |
| `assistant` → `content[].type == "tool_use"` | `tool_call` | `title` = `name`, `input` = `input`, `tool_call_id` = `id`; `tool_kind` derived from the tool name (`Read`→`read`, `Edit`/`Write`→`edit`, `Bash`→`execute`, …), `other` when unknown |
| `user` → `content[].type == "tool_result"` | `tool_result` | joined on `tool_use_id` |
| — | `edit_proposal` | *not produced by this transport*: the CLI applies edits itself and surfaces them only as post-hoc `Edit`/`Write` tool calls with no diff |
| — | `permission_request` | *not produced by this transport* in v1; available only out of band via `--permission-prompt-tool` pointed at `htui`'s MCP server (MOD-11) |
| `result.permission_denials[]`, `system/permission_denied` | `permission_answer` | synthesized with `by: "policy"` and an added `denied: true`; a real answer arrives only through the MCP route above |
| `TodoWrite` tool input | `plan` | *not produced by this transport* as a first-class event; the TodoWrite approximation is off by default and gated by `agent.settings.cli` |
| `result.usage`, `result.modelUsage[*]`, `result.total_cost_usd` | `usage` | one row at turn end; see §7 |
| `result.is_error`, `result.subtype`, `result.api_error_status`, `system/api_retry` | `error` | `code` = `subtype`, `message` = `result`/`error` |
| `result` | `done` | `stop_reason` = `stop_reason`, plus added `terminal_reason` and `num_turns` |
| `system/init`, `system/hook_started|hook_progress|hook_response`, `stream_event`, `rate_limit_event`, plugin events | `other` | stored verbatim; `rate_limit_event` additionally drives the quota latch of §7 |

### 6.3 `agy --output-format stream-json` → `EventKind` (contingency contract, not built in v1)

| Stream source | `EventKind` | Rule |
|---|---|---|
| prompt argv / first stdin line | `prompt` | htui-authored |
| stdin `{"event":"user",…}` | `follow_up` | htui-authored |
| `step_update` `step_type: "agent_response"` `text_delta` | `assistant_text` | coalesced per §4.1, keyed on `step_index` |
| — | `thought` | *not produced by this transport*: only `usage.thinking_tokens` is reported, never the text |
| `step_update` `step_type: "tool"` with `tool_info.name`/`parameters` | `tool_call` | `tool_call_id` synthesized as `<conversation_id>:<step_index>` |
| `step_update` `state: "DONE"` with `tool_info.output`, or `state: "ERROR"` with `tool_info.error` | `tool_result` | `status` = `completed` / `failed` |
| — | `edit_proposal` | *not produced by this transport*: writes appear as an ordinary `write_to_file` tool step, no diff |
| — | `permission_request` | *not produced by this transport*: headless mode has no interactive prompt |
| `result.denied_actions[]`, and the `ERROR` step above | `permission_answer` | synthesized with `by: "policy"`, added `denied: true` |
| — | `plan` | *not produced by this transport* |
| `result.usage` | `usage` | `cache_write_tokens` is always null; no cost field, so `cost_micros` is null |
| `result.status ∈ ERROR|INVALID`, `tool_info.error` | `error` | |
| `result` | `done` | `stop_reason` from `status` (`SUCCESS`→`end_turn`, `CANCELED`/`INTERRUPTED`→`cancelled`) |
| `init`, `step_update` `step_type: "checkpoint"` or `"user_input"`, `subagent_info` | `other` | stored verbatim |

Because `agy` is an ACP agent (§4.5), its production mapping is table 6.1; 6.3 exists so that a
future decision to add the CLI adapter starts from a written contract rather than a fresh
investigation.

---

## 7. Quota tracking (`R-AGT-7`, `R-AGT-8`)

**What each transport can report.**

| Transport | Subscription allowance / reset window | Per-run spend | Token counts |
|---|---|---|---|
| ACP, any agent | nothing in the protocol — no allowance, plan tier, reset field or rate-limited error code | `usage_update.cost { amount, currency }`, cumulative per session, and only **MAY** be sent | `used`/`size` are context-window occupancy, not billing tokens |
| ACP, `claude` adapter | `_meta["_claude/rateLimit"]` on `usage_update`: `status`, `rateLimitType`, `resetsAt`, and `unifiedWindows { five_hour, seven_day, seven_day_overage_included }` each `{ utilization, resetsAt }` | as above, from `total_cost_usd` | as above |
| ACP, `agy_acp_server` | nothing observed; `initialize` advertises no usage capability | **Unverified - MOD-2 must confirm** whether the server emits `usage_update` at all | — |
| `claude` CLI | the same rate-limit blob as an inline `rate_limit_event` message, emitted opportunistically mid-stream and absent from the terminal `result` and from `--output-format json` | `result.total_cost_usd` and per-model `modelUsage[*].costUSD` | `result.usage` (top-level loop only) and `modelUsage[*]` (subagents included) |
| `agy` CLI | `/usage` is an interactive TUI panel and slash commands are rejected under `--input-format stream-json`; a documented `statusLine` command hook does pipe a payload containing `quota { remaining_fraction, reset_time, reset_in_seconds }` and `plan_tier` — **Unverified - MOD-2 must confirm** | none in the stream | `result.usage`, no `cache_write_tokens` |

**`usage` payload reconciliation.** ANA-9 §4.3 fixes five keys (`input_tokens`, `output_tokens`,
`cache_read_tokens`, `cache_write_tokens`, `cost_micros`) that ACP does not stably emit. Rather than
rename anything, the driver keeps all five, treats the four token fields as nullable, and adds the
keys the transports do supply (adding keys is permitted by §4.3):

- ACP: the four token fields are `null`; `context_used` and `context_size` carry `used`/`size`;
  `cost_micros` carries the **delta** since the previous `usage` row of this step
  (`round(cost.amount × 1e6)` when `currency == "USD"`, else `null` with `cost_amount` and
  `cost_currency` set), and `cost_micros_total` carries the cumulative value. Storing the delta is
  what makes `run_step.usage` a plain sum.
- `claude` CLI: the four token fields are summed over `modelUsage[*]`
  (`inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`), not from
  `result.usage`, because `usage` counts only the top-level loop and undercounts as soon as a
  subagent runs, while `modelUsage` and `total_cost_usd` include subagents. `usage_scope:
  "model_usage"` records which convention was used.

**`run_step.usage` summing.** The recorder maintains a running total over the `usage` rows of the
step and writes it to `run_step.usage` at every flush and at step end, so a crashed step still has a
partial figure. Nothing renders it yet: `RunStepSummary` has no `usage` field, so the Runs tab
column MOD-4 wants needs a projection change as well as a render change.

**`agent_box.quota` shape.**

```json
{
  "source": "acp_meta_rate_limit",
  "billing": "subscription",
  "status": "allowed",
  "exhausted": false,
  "windows": [
    { "id": "five_hour", "utilization": 0.26, "resets_at": "2026-09-05T13:20:00Z" },
    { "id": "seven_day", "utilization": 0.57, "resets_at": "2026-09-08T04:00:00Z" }
  ],
  "spend": { "session_micros": 394692, "currency": "USD" },
  "observed_at": "2026-09-05T12:31:07Z"
}
```

`agent_box.quota_at` mirrors `observed_at`. `utilization` is the 0..1 fraction consumed and
`resets_at` is the epoch converted to a timestamp. A `per_token` agent has no `windows` and carries
`spend` plus the configured caps' state.

**Refresh trigger.** The value is *latched passively*, never polled: every `usage`-bearing message
seen during any run updates it, which is exactly `R-AGT-7`'s "refreshed per run" and costs nothing.
`Settings > Refresh agents` re-runs the probes but cannot refresh quota (a handshake reports none),
and this is stated in the UI rather than implied. There is no query path: `/usage` under `claude -p`
hangs and produces no output, and `agy`'s equivalent is TUI-only.

**`R-AGT-8` selection.** The orchestrator walks the phase's candidate agents in priority order and
skips one when: `quota.exhausted` is true; any window has `utilization >= 1.0`; `quota.status` is
not `allowed`; or the per-token cap is already reached for this run or batch. A **null** quota means
*unknown*, and unknown is treated as available — otherwise `agy`, which reports nothing, would never
be selected.

**Per-token cap enforcement point.** `project.settings.per_token_cap_run` and
`per_token_cap_batch` (columns that already exist). The recorder is the enforcement point because it
is the only place that sees every `usage` row: after each row it compares the running spend against
the caps and, on breach, calls `AgentSession::cancel()`, then writes
`error { code: "cap_exceeded", message }` and `done { stop_reason: "cancelled" }` and marks the step
failed. For `claude` the driver additionally passes `--max-budget-usd` (CLI) so a second, server-side
cap exists; both figures are documented client-side estimates that may differ from the actual bill,
so the cap is a guard rail and never a billing statement.

---

## 8. Crate and module layout for MOD-2

**A fourth workspace crate, `htui-agent`.** The trait, the event enum and both transports live
there; only the *persisted* types stay in `htui-core`. Rationale: `htui-core` is deliberately
dependency-light (no `sqlx`, no `tokio` process, no child-process code) and is the crate every other
one depends on; putting a JSON-RPC SDK, `process-wrap` and a diff library behind it would make the
domain crate the heaviest node in the graph. MOD-1's own plan already anticipated this ("MOD-2 adds
the driver crate the same way" as `htui-store`).

```
crates/htui-agent/
  Cargo.toml
  src/lib.rs            #![warn(missing_docs)]; re-exports driver, event, launch
  src/driver.rs         AgentDriver, AgentSession, SessionSpec, DriverCaps, DriverError
  src/event.rs          DriverEvent + payload structs, ToolKind, PermissionOption/Answer
  src/record.rs         Recorder: coalescing, seq/turn, prompt_digest, usage summing,
                        scrub hook, WriteStore append, pending/*.jsonl append
  src/launch.rs         AgentLaunch/AgentSettings serde types, ${…} resolution, redacted Debug,
                        spawn (process-wrap: CreationFlags + JobObject), stderr capture
  src/probe.rs          R-AGT-6 tiers, handshake probe, agent_box.probe snapshot
  src/acp/mod.rs        session task, capabilities, config options, permission responders
  src/acp/map.rs        SessionUpdate/Dispatch -> DriverEvent (table 6.1)
  src/cli/mod.rs        line-reader supervisor, stdin follow-ups, signal handling
  src/cli/claude.rs     stream-json envelopes -> DriverEvent (table 6.2)
  src/fake.rs           FakeDriver, feature `test-support`
  src/conformance.rs    CASES + run_case<D: AgentDriver> + run_all, feature `test-support`
  tests/fixtures/*.jsonl recorded transcripts
crates/htui-core/src/store/traits.rs   + append_events, set_step_usage, upsert_agent(_box)
crates/htui-store/src/cache/pending.rs + append_pending
crates/htui/src/agent_worker.rs        session tasks, one per chat/step
crates/htui/src/ui/tabs/chat/**        R-TUI-6
crates/htui/src/ui/tabs/settings/**    a SettingsRegistry mirroring DetailRegistry, so MOD-2's
                                       agent section and MOD-15's hierarchy section coexist
crates/htui/src/store_worker.rs        + StoreRequest::StepEvents / StoreReply::StepEvents,
                                       + StoreRequest::ChatStream / StoreReply::Chat, + name() arms
```

**No fourth `select!` arm in `event_loop.rs`.** The agent worker holds a clone of the existing
`mpsc::UnboundedSender<ReplyEnvelope>` and emits stream frames as replies stamped with the chat
tab's `Origin`. `App::latest` is keyed by `(Origin, Discriminant<StoreRequest>)` and `is_fresh` is a
stateless predicate, so many replies carrying one `seq` all pass — but a later request of the *same
discriminant* from the same origin would silently orphan the stream. That is why the stream gets its
own `ChatStream` request/reply discriminant pair that nothing else uses, and why the chat tab issues
exactly one per session. `event_loop.rs` is unchanged, as its module doc promises.

**New workspace dependencies, with the versions verified on 2026-09-05.**

| Crate | Version | MSRV | Why |
|---|---|---|---|
| `agent-client-protocol` | `= "2.1.0"`, no features | 1.88.0 | the ACP client (§4.2) |
| `tokio` | existing `1.53`, **add `process` and `io-util`** | — | `process` is genuinely absent from the workspace feature list; `io-util` currently arrives only because `sqlx-core`'s `runtime-tokio` unifies it in, which is an implicit cross-crate dependency worth making explicit |
| `tokio-util` | `0.7.19` (`compat`) | 1.71 | adapts tokio child stdio to the `futures` `AsyncRead`/`AsyncWrite` that `ByteStreams` wants |
| `process-wrap` | `10.0.0`, features `tokio1`, `creation-flags`, `job-object` | 1.87.0 | Windows job object + `CREATE_NO_WINDOW` without `unsafe` in `htui`'s own crates |
| `similar` | `3.2.0` | 1.85 | unified diff for `edit_proposal.diff` |
| `which` | `8.0.6` | 1.70 | `PATH`/`PATHEXT`-correct binary resolution for the probes |
| `sha2` | already present, `0.10` | — | `prompt_digest` |
| `futures` | already present, `0.3` | — | the SDK's stream traits |

Workspace `rust-version` moves `1.85` → `1.88` in the same change (§4.2).

**Test strategy.**

1. **`FakeDriver`** (feature `test-support`) yields a scripted `Vec<DriverEnvelope>` synchronously.
   It is what the chat tab's `insta` snapshots run against, driven by a `Harness::drive()` pump that
   mirrors `Harness::settle()`'s inline model — `settle()` answers requests through
   `store_worker::serve` rather than spawning the worker, which is what makes snapshots byte-stable
   with no sleeps, and a streaming driver must not break that property.
2. **A conformance suite copied from the store's**: `pub const CASES: &[&str]`,
   `run_case<D: AgentDriver>(name: &str, driver: &D)`, `run_all`. One list, run against the fake,
   the ACP transport (over an in-process transport pair) and the CLI transport. Cases include:
   coalescing across a `messageId` change; a tool call whose result is a rejection; a cancel that
   answers a parked permission request; `seq` gapless across a follow-up; `turn` increments; a
   usage delta summing to the cumulative total; `raw` present iff `retain_raw`.
3. **Recorded transcripts as fixtures.** `tests/fixtures/*.jsonl` hold real captures — an ACP
   `session/update` stream and a `claude` stream-json run — replayed through `acp/map.rs` and
   `cli/claude.rs` with `insta` snapshots of the resulting `Vec<SessionEvent>`. This is the
   regression net for adapter drift: when an adapter ships a new update kind, the snapshot shows it
   landing in `other` rather than the build breaking.
4. **`tokio` `test-util`** (already a dev-dependency) supplies `pause()`/`advance()` for the cancel
   grace window; nothing else in the driver is time-dependent, by §4.1's construction.
5. **A live smoke test, ignored by default** (`#[ignore]`), that runs the tier-2 handshake probe
   against whatever is installed on the box and asserts `protocolVersion == 1`. It is the only test
   that touches a real agent, and it burns no tokens.

---

## 9. Phasing and downstream impact

**MOD-2 build order.**

1. `htui-agent` crate skeleton: `driver.rs`, `event.rs`, `record.rs`, `fake.rs`, `conformance.rs`,
   plus the `WriteStore` additions and `append_pending`. Nothing spawns a process yet; the whole
   §4.1 contract is testable against the fake.
2. `launch.rs` and the JSONB serde types of §5, the seed rows, and the `fixtures.rs` corrections.
   MSRV bump and the new workspace dependencies land here.
3. `acp/` against recorded transcripts, then against `claude-agent-acp` live; the session banner and
   `session/set_config_option` model selection.
4. The chat tab (`R-TUI-6`), the `store_worker` variants, `StepEvents` replay (`R-HIS-2`), and the
   Settings agent section (`R-TUI-8`) on a new `SettingsRegistry`.
5. `probe.rs`, `agent_box` writes and migration `0002`.
6. Quota latching, `run_step.usage` summing and cap enforcement (`R-AGT-7..8`).
7. `cli/claude.rs` as the degraded path, with its capability banner.

**What ANA-5 must provide** (it gates step 1's `prompt` row): the assembled prompt text, the
`sections[] { name, tokens, trimmed }` array, the token budget accounting that produces
`run_step.trim_record`, and a stable serialization order so `prompt_digest` is reproducible for the
same inputs. The driver computes the digest; ANA-5 owns what is digested.

**What ANA-7 must provide** (it gates step 1's persistence path): a `Scrubber` with a
`scrub(&mut serde_json::Value) -> Result<(), Unmasked>` shape that the recorder calls on every
payload and on every `raw` blob before either write path, fail-closed per `R-SEC-3`; and the
environment map for `SessionSpec.env`, resolved per project at run start and never logged.

**Forward-only migration amendments to the ANA-9 schema.** One migration, `0002_agent_probe.sql`:

```sql
ALTER TABLE agent_box ADD COLUMN probe JSONB;
COMMENT ON COLUMN agent.name IS NULL;  -- and fix the stale inline comment in 0001: agents are
                                       -- seeded by MOD-2, not by 0001
```

Nothing else changes: `agent.launch` and `agent.settings` are already `JSONB` and needed only a
documented shape, `session_event` needed no new kind (the `other` catch-all earned its keep), and
`agent_box.quota` was already reserved for exactly the §7 blob.

**Other items this document shapes.** MOD-4 reads `DriverCaps` when matching a phase's gates to a
candidate agent, and consumes `run_step.usage`; MOD-7 calls `probe.rs` from box registration and
writes `agent_box`; MOD-10 supplies the scrubber and the environment; MOD-11 supplies the MCP server
whose permission-prompt tool is the CLI transport's only route to `permission_request`; MOD-12
enforces the batch cap using §7's accounting.

---

## 10. Risks

1. **SDK cadence.** Two majors in ten weeks, and 2.1.0 was a day old when it was evaluated.
   *Mitigation:* pin `= "2.1.0"`; keep every SDK type behind `AgentDriver`; keep the
   `SessionUpdate → DriverEvent` mapping in one module; treat upgrades as scheduled work with the
   fixture snapshots as the regression net.
2. **MSRV bump to 1.88.** *Mitigation:* it is a declaration change, not a build break — the
   toolchain here is 1.98.1, and `sqlx-core 0.9.0` (1.94.0) and `ratatui 0.30.2` (1.88.0) already
   demand more than the declared 1.85. Record the sqlx figure so the next MSRV question is not
   re-litigated.
3. **Two reactors in one process.** async-io's `polling` thread and async-process's driver thread
   run beside tokio's mio reactor for the life of the TUI. Compile and a short runtime probe pass;
   sustained load and clean shutdown were not tested. *Mitigation:* measure in MOD-2 step 3; the
   §4.2 Plan B (schema-only + ~500 LOC tokio transport) is the exit, and the trait boundary makes
   it a one-module change.
4. **Orphaned agent processes on Windows.** The SDK's process-group kill is `#[cfg(unix)]`, and
   agents ship behind wrapper launchers, so killing the immediate child can leave the real agent
   running. *Mitigation:* `process-wrap` job object with kill-on-close, `htui` spawning the child
   itself; proceed without the job object when assignment fails, as Cargo does.
5. **Adapters ship update kinds ahead of the schema.** *Mitigation:* non-exhaustive decode with a
   wildcard arm into `other`; a fixture snapshot proves an unknown kind is stored, not dropped.
6. **Adapter version skew.** The registry pins `claude-agent-acp` 0.75.0; this box has 0.55.0;
   an unpinned `npx` follows `latest`. *Mitigation:* pin the version in `agent.launch`, record
   `agentInfo.version` per box in `agent_box.probe`, and show both in the Settings tab.
7. **ACP v2.** Its `SessionUpdate` patches earlier messages by id, which an append-only
   `(run_step_id, seq)` table cannot express. *Mitigation:* pin `ProtocolVersion::V1`, no
   `unstable_*` features; a v2 move is an ANA amendment, not a driver patch.
8. **`agy` Terms of Service.** Driving Antigravity from a third-party client is flagged as a ToS
   violation for personal accounts by community adapters, while the ACP server itself is
   Google-authored and Google-hosted. *Mitigation:* state the account modes in the Settings agent
   section, prefer API-key or enterprise auth in documentation, and keep the row disabled on a box
   whose probe reports `unauthenticated`.
9. **The CLI fallback cannot gate.** No edit proposals, no in-stream permission requests, no plan.
   *Mitigation:* `DriverCaps` is authoritative, the chat tab banners the gap, and MOD-4 refuses to
   schedule a gated phase onto a CLI-only agent.
10. **No quota source for `agy`.** *Mitigation:* `agent_box.quota` stays null, `R-AGT-8` treats null
    as available, and per-token caps still bind because they are computed from `usage` rows rather
    than from the vendor.
11. **Handler deadlock.** `SentRequest::block_task()` inside a dispatch handler is a documented
    guaranteed deadlock, and so is a Postgres write in a handler body. *Mitigation:* the recorder is
    a separate consumer of the event channel; `block_task` is called only from the session task's
    foreground future; a code comment at each call site names this risk.
12. **Secrets in a launch environment.** *Mitigation:* hand-written redacting `Debug` on
    `AgentLaunch` and `SessionSpec`; the scrubber runs over every payload before persistence;
    `htui`'s DSN lives in a crate the driver does not depend on.
13. **Cost figures are estimates.** Both `total_cost_usd` and `modelUsage[*].costUSD` are documented
    as client-side estimates that can differ from the bill. *Mitigation:* caps are guard rails, the
    Settings tab labels the figure as an estimate, and nothing in `htui` bills from it.

---

## 11. Validation criteria (for MOD-2)

1. A `FakeDriver` and both real transports pass the same `CASES` list, and adding a transport adds no
   case.
2. Twenty `agent_message_chunk` values across two `messageId` groups, interleaved with one
   `tool_call`, persist as exactly three `assistant_text` rows and one `tool_call` row, in `seq`
   order, with no wall-clock dependency: the same fixture replayed twice yields identical rows.
3. `seq` is gapless `0..n` for a step with a prompt, two follow-ups and a cancel; `turn` is `0,1,2`;
   the `prompt` row is `seq = 0`, `turn = 0`, `role = htui`, and its `digest` equals
   `run_step.prompt_digest`.
4. With `project.settings.keep_raw_events = false` every persisted row has `raw IS NULL`; flipping it
   true and re-running the same fixture produces the same rows with `raw` populated.
5. A cancel answers every parked permission request with `{"outcome":"cancelled"}` before the
   session ends, writes a `permission_answer` row per parked request, and leaves no unfinished
   `tool_call` without a synthesized `tool_result`.
6. An `fs/write_text_file` for a new file writes one `edit_proposal` row whose `diff` applies cleanly
   to an empty file and produces the written content; a second write to the same path in the same
   tool call updates that row rather than adding a second.
7. `run_step.usage` equals the sum of the step's `usage` rows, and for an ACP session equals the last
   `cost_micros_total` observed.
8. A per-token cap breach cancels the session within one event of the breach and leaves
   `error{code:"cap_exceeded"}` followed by `done{stop_reason:"cancelled"}` as the last two rows.
9. A seeded `claude` row resolves on this box to a launch that starts the adapter and completes an
   `initialize` with `protocolVersion == 1`; the same row on a box without node resolves to
   `probe.status = "missing"` and leaves `agent_box.enabled = false`.
10. A seeded `agy` row resolves to `agy_acp_server` through the glob probe and completes
    `initialize`; on an unauthenticated box the probe records `unauthenticated` rather than `ready`.
11. Killing a session's process tree leaves no `node`, `claude` or `agy_acp_server` process behind on
    Windows (job object) and on Linux (process group).
12. A chat session started offline appends to `<cache_dir>/pending/<project_id>.<run_id>.jsonl` in
    `seq` order, and `upload_pending` lands it idempotently — the existing MOD-6 test still passes
    against rows the driver produced.
13. `cargo build` succeeds with `rust-version = "1.88"` declared, and `cargo tree -i tokio` shows the
    SDK contributing no tokio edge.
14. Every item below is closed by a probe or a live run before MOD-2 is marked done. These are the
    "**Unverified - MOD-2 must confirm**" lines from the body, collected:
    - the exact `configOption` ids and value vocabulary `claude-agent-acp` exposes for model
      selection, and behaviour when an agent offers none (§4.4);
    - whether `claude-agent-acp` 0.75.0 still drops a `rate_limit_event` that arrives before the
      first assistant usage (§4.4, §7);
    - `claude`'s cancellation semantics under SIGINT versus SIGTERM, and the resulting `done` row
      (§4.4);
    - the `assistant` content-block and `stream_event` delta shape for thinking blocks in the CLI
      stream (§6.2);
    - the exact `--permission-prompt-tool` MCP tool contract — tool name convention, request schema,
      allow/deny response shape, and whether it can carry a diff (§4.3, deferred to MOD-11);
    - whether `agy_acp_server` emits `usage_update`, and in what field (§7);
    - whether `agy_acp_server` issues `session/request_permission` in its `default` mode, and the
      option ids and kinds it offers (§4.5);
    - whether its file edits arrive as a standard `tool_call` with `kind: "edit"` and a `diff`
      content block, or in a vendor-specific shape (§4.5);
    - the model list and `configOptions` contents that `session/new` returns for `agy_acp_server`,
      which is what fills the empty `agent.models` seed (§5.3);
    - the darwin/linux `.par` launch mechanics and the meaning of the literal empty `--uid=`
      argument, and whether `localharness_external.exe` must remain beside the server (§4.6);
    - the `agy` CLI `statusLine` quota payload, if the CLI path is ever built (§7).

---

## 12. Sources

**Local repository** (branch `main`, HEAD `936e001`, read 2026-09-05):
`Cargo.toml`; `crates/htui-core/src/model/{agent.rs,event.rs,run.rs,ids.rs,mod.rs}`;
`crates/htui-core/src/store/{traits.rs,mem.rs,conformance.rs}`; `crates/htui-core/src/fixtures.rs`;
`crates/htui/src/{store_worker.rs,event_loop.rs,testkit.rs}`; `crates/htui/src/app/{state.rs,update.rs,mod.rs}`;
`crates/htui/src/ui/tabs/{settings.rs,registry.rs,backlog/detail/runs.rs}`;
`crates/htui-store/src/{secret.rs,identity.rs,connect.rs,backend.rs}`;
`crates/htui-store/src/cache/{mod.rs,pending.rs,refresh.rs}`;
`crates/htui-store/migrations/0001_init.sql`; `crates/htui-store/tests/migrations.rs`;
`docs/REQUIREMENTS.md` §5 (`R-AGT-1..8`), §7 (`R-HIS-1..3`), §9 (`R-SEC-1..4`), §10 (`R-MCP-1..4`),
§11 (`R-TUI-4/6/8`); `docs/ANA-9.md` §4.3, §5.7, §5.8, §5.10, §6.1, §6.3, §9; `CONCEPTS.md`;
`HANDOFF.md` (ANA-4, MOD-2).

**Local probes run on this box on 2026-09-05** (Windows 11, cargo 1.98.1, rustc 1.98.1):
`claude --version` → `2.1.261 (Claude Code)`; `claude --help` (304 lines, flag set of §4.4);
`node --version` → `v24.18.0`; `agy --version` → `1.1.26`; `agy --help` (flags and the twelve
subcommands, no `run`, no `acp`); `type -a claude` / `type -a claude-agent-acp`;
`file claude-agent-acp` → Bourne-Again shell script; `claude-agent-acp --version` → `0.55.0`;
`node ".../@agentclientprotocol/claude-agent-acp/dist/index.js" --version` → `0.55.0`;
a Rust spawn probe (`std::process::Command::new(name).arg("--version")`) → `claude`, `agy`, `node`,
`npx 11.13.0` succeed and `claude-agent-acp` fails with `program not found`;
a Rust dyn-compatibility probe (`pub trait AgentDriver { async fn start(&self); }` + `&dyn
AgentDriver`) → `error[E0038] ... because method start is async`;
`cargo info agent-client-protocol` → 2.1.0, Apache-2.0, rust-version 1.88.0, repo
`agentclientprotocol/rust-sdk`, features `unstable_*` all non-default;
`cargo info similar@3.2.0` (rust-version 1.85), `cargo info which@8.0.6` (1.70),
`cargo info process-wrap@10.0.0` (1.87.0, features `tokio1`/`creation-flags`/`job-object`),
`cargo info tokio-util@0.7.19` (1.71);
directory listing of `%LOCALAPPDATA%\JetBrains\CLion2026.2\acp-agents\antigravity-acp\1.0.0\`
(`agy_acp_server.exe` 297 MB, `localharness_external.exe` 122 MB).

**Vendored crate sources** (`~/.cargo/registry/src/index.crates.io-…`, read 2026-09-05):
`agent-client-protocol-2.1.0/src/{jsonrpc.rs,session.rs,session/v2.rs,role/acp.rs,acp_agent.rs,lib.rs}`
(`Client::builder`, `connect_with`, `on_receive_request`, `on_receive_notification`,
`SentRequest::{block_task,on_receiving_result,detach}` with its deadlock note, `ActiveSession::
{send_prompt,read_update}`, `SessionMessage::{SessionMessage,StopReason}`, `build_session`,
`load_session`, `resume_session`, `ByteStreams`, `AcpAgentConfig { command, args, env }`,
`CREATE_NO_WINDOW` at `acp_agent.rs:277-281` versus `process_group`/`kill_process_group` under
`#[cfg(unix)]` only, and zero `?Send`/`LocalSet`/`spawn_local`/`Rc<` in `src/`);
`agent-client-protocol-schema-1.7.0/src/v1/{client.rs,agent.rs,tool_call.rs}` (`SessionUpdate` 11
stable variants, `PermissionOptionKind` 4 values, `RequestPermissionOutcome`, `UsageUpdate { used,
size, cost }`, `ToolCallStatus` 4 values, `Diff { path, old_text: Option<String>, new_text }`,
`StopReason` 5 values) and `src/v2/client.rs` (the patching `SessionUpdate`);
`sqlx-core-0.9.0/Cargo.toml` (`rust-version = "1.94.0"`); `ratatui-0.30.2/Cargo.toml`
(`rust-version = "1.88.0"`).

**Installed adapter bundle** (`@agentclientprotocol/claude-agent-acp` 0.55.0, read 2026-09-05):
`package.json` (version 0.55.0), `dist/acp-agent.js` and `dist/settings.js`
(`CLAUDE_CODE_EXECUTABLE`, `CLAUDE_CONFIG_DIR`, `ANTHROPIC_MODEL`, `configOptions`,
`set_config_option`, `model_config`).

**Web** (fetched 2026-09-05 unless noted):
`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` (schema version 1.0.0, 39
agents; `claude-acp` 0.75.0 npx `@agentclientprotocol/claude-agent-acp@0.75.0`;
`antigravity-acp` 1.1.1, Google LLC, binary distribution over five platforms, `./agy_acp_server.exe`
on Windows and `./agy_acp_server.par` with `args: ["--uid="]` on Linux);
`https://raw.githubusercontent.com/agentclientprotocol/registry/main/.protocol-matrix/latest.json`
(daily capability probe matrix; `claude-acp` at `protocolVersion 1`, `authMethods ["terminal"]`,
`loadSession`/`sessionList`/`sessionResume` true, `setModel` false — a legacy `session/set_model`
probe, not evidence about `session/set_config_option`);
`https://agentclientprotocol.com/protocol/v1/tool-calls.md` (permission options and outcomes, the
`diff` content shape, tool-call statuses, the cancellation MUST);
`https://agentclientprotocol.com/protocol/v1/{overview,transports,initialization,authentication,session-setup,prompt-turn,content,agent-plan,file-system,terminals,cancellation,session-modes,session-config-options,slash-commands,extensibility,schema}.md`;
`https://agentclientprotocol.com/protocol/v2/overview.md` (Draft);
`https://agentclientprotocol.com/updates.md` (through 2026-07-22);
`https://crates.io/api/v1/crates/agent-client-protocol` (2.1.0, published 2026-09-04T16:34Z, 78
versions, `rust_version` 1.88.0 from 1.1.0 onward);
`https://registry.npmjs.org/@agentclientprotocol%2Fclaude-agent-acp` and
`https://registry.npmjs.org/@zed-industries%2Fclaude-code-acp` (the rename and the deprecation
string, old package frozen at 0.16.2);
`https://code.claude.com/docs/en/{headless,cli-reference}` (stream-json envelopes,
`--permission-prompts` v2.1.259+, `--max-turns` still documented and enforced, `--bare`, cost
estimates, `--forward-subagent-text` and `parent_tool_use_id`);
`https://antigravity.google/docs/cli/{headless,reference,commands/usage,credits}` and
`https://antigravity.google/docs/ide/extensions` (the `agy` stream-json contract, `denied_actions`,
`useG1Credits`, the `statusLine` quota payload, the four auth shapes);
`https://github.com/agentclientprotocol/rust-sdk` (`md/migration_v2.0.md`, `src/yopo/src/lib.rs`,
`examples/yolo_one_shot_client.rs`);
`https://github.com/zed-industries/zed` (`crates/acp_thread/src/{acp_thread.rs,connection.rs}`,
`crates/agent_servers/src/{acp.rs,agent_servers.rs}`, `crates/project/src/agent_server_store.rs`,
issue #55921 on `.cmd` quoting) — read as prior art; its permission-persistence and buffer-
interception behaviour is cited as design inspiration and was not independently re-verified here;
`https://github.com/google-antigravity/antigravity-cli/issues/31` (no `agy --acp` flag; open);
`https://github.com/github/copilot-cli/issues/845` (an ACP agent that auto-approves internally and
never calls `session/request_permission` — the reason `DriverCaps` exists);
`https://github.com/rust-lang/rust/issues/109417` and
`https://blog.rust-lang.org/2026/08/21/enabling-next-solver-on-nightly/` (return-type notation still
nightly); `https://github.com/rust-lang/rust/issues/37519` (`PATHEXT` and `Command::new`);
`https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects`;
`https://lib.rs/crates/process-wrap`.
