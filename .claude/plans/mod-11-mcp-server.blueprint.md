# Blueprint: MOD-11 — htui MCP server, T0–T10

**Status**: proposed (2026-10-03, code-architect). Implements `.claude/plans/mod-11-mcp-server.plan.md`
(CONFIRMED 2026-10-03, OQ-1…OQ-10 as recommended, fact-checked `wf_33a62d14-658`) under
`.claude/prds/mod-11-mcp-server.prd.md`. The plan's D1–D19, I-1…I-8, task order
`T0 → {T1 ∥ T2 ∥ T3} → {T4 ∥ T5} → T6 → {T7 ∥ T8} → T9 → T10`, file sets and "Verified claims" are
binding. Where this blueprint had to choose, the choice is a **B-n** driven by a finding **F-n**.
Anything that would move a confirmed decision is an **E-n** (§0b); there is one, and it changes nothing
unless the maintainer says so.

**Verified at**: `c4bf516c` (`hr/MOD-11`, sandbox). `git diff 95adf87f c4bf516c` touches only the plan
and the PRD, so every code anchor the plan cites still holds; every number below was re-read at HEAD.
Paths are relative to `crates/` unless they start with `docs/`, `.claude/` or name a root file. Gortex
answers symbol and file reads despite its "INACTIVE" banner; it has **no field-usage edges**, so field
searches were done with `grep -n '\bname\b'` (the hook blocks a bare word that matches a symbol).

**House style (MOD-41/MOD-42, unchanged)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`; every new
`pub` item is documented and `Debug`; **no default body on a store trait** (`WriteStore`,
`WorkerStore`, `RecorderStore`, `RelayStore`, `WorkerHost`); `max_width = 100`; every commit compiles;
red first, then green, committed incrementally (uncommitted work dies with the session; never stash on
a shared tree; never `git add -A`). **E0034 hygiene** (H-1): no module `use`s `WorkerStore`,
`RecorderStore`, `RelayStore` or `WorkerHost`; bounds name them by path; every forwarding body and
every call on a type implementing two families is UFCS
(`htui_core::store::WorkerStore::write_step_document(&store, …)`). The conformance doc rule
`every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs:1768`): a backticked
snake_case name with ≥ 4 underscores in that file's docs must be a fn there or in `mem.rs`; a Pg test is
spelled `pg_criteria.rs::name` **and must already exist**. **Never the word `zeta`** in an identifier
or fixture string (`htui-agent/tests/extensibility.rs` greps the workspace).

**Layout**: §0 findings · §0a decisions · §0b escalation · §1 build order and lanes · §2 shared shapes
(2.1–2.12) · §3–§13 one section per task T0…T10 (files, edits with anchors, tests written first,
commits, gate) · §14 hazards · §15 gate reference · §16 pins.

---

## 0. Findings

| # | Sev. | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | Major (T2, T4, T5) | T5: "against an `McpHost<MemStore>` listener"; D5 `McpHost<H: WorkerHost>`. | `WorkerHost` has exactly two implementors: `PgStore` and `Backend` (`htui-store/src/worker.rs:614`, `:694`). `MemStore` is not one, and `htui-core` cannot implement it for `MemStore` without T1's file (`htui-core/src/store/worker.rs`) while T2 runs in parallel. `htui-worker`'s and `htui`'s tests use `Backend::memory(MemStore)` (`htui-store/src/backend.rs:75`). | **B-1** |
| **F-2** | Major (T2, T6) | D5: "takes `host.writer()` **per call** (the TUI's backend swaps, `store_worker.rs:2315`)". | The TUI's `Backend` is a **value** the store loop reassigns or mutates in place at six sites (`store_worker.rs:2231`, `:2316`, `:2515`, `:2551`, `:2617`, and `*backend = Backend::Online` at `:2751`). A clone held by `McpHost` keeps the old `PgStore` pool alive and writes to the old server after a `SetDsn`; "per call" on a stale clone is still stale. Walks already "go back to the server [they were] claimed on" (`:2313-2314`). | **B-2** |
| **F-3** | Major (T2, T7) | D12: `ConceptSearch` "taking `SearchQuery` and returning `Vec<Hit>`"; D1 gives `htui-mcp` no `htui-store` dependency. | `SearchQuery` and `Hit` are `htui-store` types (`htui-store/src/vector.rs:310-348`); `PointType::parse` is private (`:95`). | **B-3** |
| **F-4** | Major (T1, T4) | `item_link`: "`to` resolved by key **within the scope's project**". | No by-key item read exists on `WriteStore` or `WorkerStore`; only `ReadStore::items(scope, filter)` (`traits.rs:91`), which `WorkerStore` does not forward and `WorkerHost::Store` does not have. `item.key` is generated, unique through `UNIQUE (project_id, key_prefix, key_number)` (`0001_init.sql:316`, `:330`). | **B-4** |
| **F-5** | Minor (T1, T4) | D13: `withdraw_link(fence, from, to, kind, run)`. | A fenced miss is `StoreError::Fenced { step: StepId }` (`traits.rs:2481-2491`); a `RunId` cannot name the step, and `StepFence` is a step's fence. | **B-5** |
| **F-6** | Major (T1) | D13: `propose_link` is a "`cite`-style upsert". | `cite`'s upsert overwrites `proposed_by_step_id` on a **live** row (`pg/write.rs:6229-6237`). Copied, an agent that re-proposes a live importer or human link becomes its proposer and may then withdraw it — defeating PRD OQ-4 "an agent may tombstone only a link its own run proposed". | **B-6** |
| **F-7** | Major (T6) | D19: a protected `<section name="output">`; "existing goldens do not move". | Sections are placeholder-driven (`prompt/mod.rs:760-800`); a new **template** placeholder would change every default body in `defaults.rs` and move every digest. The persona frame is the precedent for a section no template places: `Placeholder::Persona` is internal, not in `Placeholder::ALL` (`prompt/template.rs:117-121`, `:131-134`), rendered outside the spans (`prompt/mod.rs:517-520`, `:676-681`). | **B-7** |
| **F-8** | Major (T6) | T6 file set: `prompt/mod.rs`, `prompt/render.rs`, snapshots. | `PromptSpec` has **seven exhaustive literals**: `htui-core/src/prompt/fixtures.rs:161`, `:262`, `:367`, `:508`; `htui-orch/src/engine.rs:4815`, `:5560`; `htui/src/preview.rs:281`. `promote.rs:89` and `prompt/mod.rs:647`, `:1503`, `htui-agent/src/excerpt.rs:1147` use `..`. `fixtures.rs`, `template.rs` and `preview.rs` are outside T6's set. | **B-8** |
| **F-9** | Major (T6) | T6: "Engine tests: a fake session that calls `document_write` via its token". | `htui-mcp` depends on `htui-orch` (D1). `htui-orch`'s engine tests are unit tests (`engine.rs` `mod tests`); a dev-dependency back-edge to `htui-mcp` compiles `htui-orch` twice for them, so `McpHost` would implement a *different* `ToolHost`. | **B-9** |
| **F-10** | Major (T2) | D2: `initialize` answers the client's `protocolVersion` when it is `2025-06-18`, `2025-03-26` or `2024-11-05`, else the newest. | The fact-check's live probe of claude 2.1.287 (`/tmp/mcpprobe/srv.log`, transcribed in §2.7 because a sandbox restart wipes `/tmp`) sends `"protocolVersion":"2025-11-25"` and `tools/call` carries `"_meta":{"claudecode/toolUseId":…,"progressToken":2}`. | **B-10**, **B-11** |
| **F-11** | Minor (T2, T5) | D3: the handshake's `version` is "the htui build version"; "a version mismatch (child from a rebuilt binary) is refused". | No build id exists: every crate reads `env!("CARGO_PKG_VERSION")` = `0.1.0` (`htui-store/src/pg/mod.rs:57`, `htui-agent/src/acp/client.rs:43`); a rebuilt binary has the same version. | **B-12** |
| **F-12** | Major (T5) | D6: exit 0 / 2 / 3; T5 files `cli.rs`, `lib.rs`, `mcp_cmd.rs`, test, `htui/Cargo.toml`. | `main.rs:69-80` maps an exit code only through `WorkerExit`/`ProvisionExit` downcasts (else 1) and `reports_to_sentry` (`main.rs:91-96`) sends every code ≠ 2 to GlitchTip. Adding `htui-mcp` to `htui`'s dependencies also rewrites `Cargo.lock`. | **B-13** |
| **F-13** | Minor (T7) | D12: "`worker_cmd.rs` builds one for the worker"; T7 files omit it and name no test file. | `htui/src/worker_cmd.rs:96` is where the worker's runtime is built. T8 does not touch it. | **B-14** |
| **F-14** | Major (T8) | OQ-7: `command_run` through the platform shell with a timeout, output tail kept. | A timeout kill must reach the shell's children: `verify.rs`'s `spawn_supervised` (process group / job object, `:464-500`), `drain` (`:442`), `TailBuffer` (`isolate/git.rs:1153`, `CAPTURE_TAIL = 64 KiB` at `:63`) already do it, privately. `TailBuffer` keeps no "dropped" flag. | **B-15** |
| **F-15** | Minor (T8) | D14: `0014` adds `claimed_by`, `heartbeat_at`. | `CommandRun` has a `From<NewCommandRun>` (`model/run.rs:712`) and literals at `htui-core/src/store/conformance.rs:11035`, `htui-orch/src/recover.rs:901`; 11 `NewCommandRun` literals. `Run` omits `lease_owner` for the same reason (`model/run.rs:203-205`, MOD-42 B-8). | **B-16** |
| **F-16** | Minor (T6) | D10: "keep the lease alive until the session (and its `after_done`) completes". | `after_done` is the sink's write (`htui-worker/src/views.rs:535-552`, `engine.rs:5074-5077`) and never reaches MCP; a turn's tool traffic precedes its `done`, which is where `drive` returns. Returning the lease would change `drive_once`'s three call sites (`engine.rs:4158`, `:5057`, `:5793`). | **B-17** |
| **F-17** | Minor (T2, T6) | D4: `transport: TransportKind {Acp, Cli}`. | `htui_core::model::Transport { Acp => "acp", Cli => "cli" }` exists (`model/agent.rs:11-17`). `drive_once` holds no agent row; `self.parts.graphs.agent(id)` is what `walk_candidates` reads (`engine.rs:3218-3225`). | **B-18** |
| **F-18** | Minor (T2, T6) | D11: "the listener is closed in `RunRuntime::shutdown`". | `Shared.tools` is `Option<Arc<dyn ToolHost>>`; D4's trait has only `open`. | **B-19** |
| **F-19** | Major (T2) | D1: "Uses `contained::spawn*` where `htui-agent`'s clippy rules apply". | `htui-mcp`'s accept loop, connection tasks and `command_run` children run **inside the TUI process**, where MOD-65's panic hook restores the terminal for a panic outside the contain window (`htui-agent/src/contained.rs:1-30`). The root `clippy.toml` has no disallowed list; a crate-level one **replaces** the root (`htui-agent/clippy.toml:1-2`). | **B-20** |
| **F-20** | Minor (T2, T9) | D18: `SessionSpec` derives `PartialEq, Eq` and `Clone`; `PromptPort` id-compared. T9's files include only `tools/permission.rs` of `htui-mcp`. | `mpsc::Receiver` is not `Clone`; the port must survive `SessionSpec::clone`. Minting the pair happens in `McpHost::open` (`host.rs`, T2's file), not in the tool. | **B-21** |
| **F-21** | Note (T1) | D13: run → item lock order. | `step_fence` (`pg/write.rs:197-215`) answers `()` and reads only `lease_owner`; the own-item check needs `run.item_id` and `run.project_id` from the same `FOR SHARE OF r` read. `park_step` takes `FOR UPDATE OF s, r` then updates `item` (`:5485-5530`); `write_document` takes only the item (`:5312-5330`). | Helper `step_scope` (§2.4) |
| **F-22** | Note (T8) | D14: `claim_command` reaps `running` rows whose `heartbeat_at` is older than 3× the beat. | The conformance suite has no way to stage a stale Pg row through the API (Pg stamps with `clock_timestamp()`; `MemStore` with its clock). | Reaping pinned by a `mem.rs` unit test and `pg_criteria.rs` (raw-SQL backdate), not a conformance case |
| **F-23** | Note (T6) | D11: chat leases. | A chat's session lives in its own task; `ChatArgs` has two literals and one destructure (`htui/src/agent_worker.rs:1071`, `:2163`, `:3930`). `ProgressSink` has one literal (`htui-worker/src/runtime.rs:919`). | The lease rides in `ChatArgs` (§9) |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-1): `htui-mcp` tests run `McpHost<Backend>` over `Backend::memory(MemStore::demo())`;
  `htui-store` is a **dev-dependency** of `htui-mcp` (`features = ["demo", "test-support"]`). D1's
  runtime dependency list is unchanged. T5's integration test uses the same.
- **B-2** (F-2): `McpHost` keeps `host: StdMutex<H>` and `pub fn set_host(&self, host: H)`; every
  session captures `H::Store` once, at `open` (a walk's tool writes go to the server its `Kit` writes
  to). The TUI store loop calls `tools.set_host(backend.clone())` at the **top of every loop iteration**
  (one cheap clone of two `Arc` pools per request; robust to any future mutation site). `open` with no
  writer answers `ToolHostError::Offline`.
- **B-3** (F-3): `htui_mcp::search::{ConceptSearch, ConceptQuery, ConceptHit, ConceptType, OwnerKind}`
  are `htui-mcp`'s own; `htui/src/mcp_search.rs` maps `SearchQuery`/`Hit` by hand. `PointType::parse`
  stays private.
- **B-4** (F-4): T1 adds `WriteStore::item_by_key(project, key) -> Result<Option<ItemId>>` (a read on
  `WriteStore` by the `command_runs` precedent, `traits.rs:1451-1464`) with its `WorkerStore`
  forwarder and one conformance case; `propose_link` also checks `to`'s project inside the store.
- **B-5** (F-5): `withdraw_link(fence, WithdrawLink { from, to, kind, step })`; the run is the step's.
- **B-6** (F-6): `propose_link` revives a tombstone with the new proposer but **keeps the proposer of a
  live row**; `withdraw_link` therefore can only ever tombstone what this run proposed or revived.
- **B-7** (F-7): `Placeholder::Output` is internal (not in `ALL`, `from_token` never yields it),
  rendered **after** the last span, separated by `"\n\n"`; `SectionName::Output` (`"output"`) is
  protected. A spec with `document_tool: false` renders byte-identically.
- **B-8** (F-8): T6's file set gains `htui-core/src/prompt/template.rs`,
  `htui-core/src/prompt/fixtures.rs`, `htui/src/preview.rs` and `htui-core/tests/prompt_output.rs`
  (new). All seven literals get `document_tool: false` except the engine's two, which compute it.
- **B-9** (F-9): `htui_orch::fake::FakeToolHost` (feature `test-support`) implements `ToolHost`,
  records every `ToolScope` it opened and can be scripted to write the step's output document **through
  the scope's fence** when opened; engine tests use it. The production path (real `McpHost`, `client`,
  `document_write`, sink `author: None`) is pinned in `htui/src/run_worker.rs` tests (a new
  `Play::Tools` arm) and `htui/tests/runs_pg.rs`.
- **B-10** (F-10): the supported list is `["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]`;
  the newest is `2025-11-25`. Every method this server answers is unchanged across the four.
- **B-11** (F-10): a `tools/call` carrying `params._meta.progressToken` gets a
  `notifications/progress` every heartbeat while `command_run` waits or runs.
- **B-12** (F-11): the handshake's `version` is
  `htui_mcp::RELAY_VERSION = concat!(env!("CARGO_PKG_VERSION"), "+relay.1")`. It catches a release bump
  or a relay change; a rebuild-in-place is covered by `/proc/<pid>/exe` on Linux (D5, R-5).
- **B-13** (F-12): `htui::mcp_cmd::McpExit { MissingEnv (2), Refused (3), Failed (1) }`, downcast in
  `main.rs`; `reports_to_sentry` is false for every `McpExit`. T5's set gains `htui/src/main.rs` and
  `Cargo.lock`.
- **B-14** (F-13): T7's set gains `htui/src/worker_cmd.rs` and `crates/htui-mcp/tests/tools_search.rs`.
- **B-15** (F-14): T8 adds `pub async fn htui_orch::verify::run_shell(…) -> ShellRun` beside
  `ShellVerifier` (additive; `spawn_and_wait` is not refactored); T8's set gains
  `htui-orch/src/verify.rs`. The resolver, the heavy-command list and the limit overlay land in
  `htui-core/src/model/kind.rs` beside `CommandQueue` (`:40`) — the plan's "wherever the resolver
  lands"; `model/run.rs` stays untouched.
- **B-16** (F-15): `CommandRun`/`NewCommandRun` unchanged; liveness is store-internal (`MemStore`:
  `command_claims` map). `enqueue_command` refuses a row that is not `queued` or carries a start,
  finish, exit code or output.
- **B-17** (F-16): the `ToolLease` is a local of `drive_once`, declared **before** `session`, so the
  session (and its agent process) drops first; nothing is returned to the callers.
- **B-18** (F-17): `ToolScope.transport: htui_core::model::Transport`; `drive_once` reads
  `self.parts.graphs.agent(candidate.agent_id)` (no row → `Acp`, which advertises no prompt tool).
- **B-19** (F-18): `ToolHost::close(&self)` (required, no default): stops the listener, removes the
  socket file and directory, ends every session.
- **B-20** (F-19): `htui-mcp` spawns only through `htui_agent::contained::{spawn, spawn_in}` and ships
  `crates/htui-mcp/clippy.toml`, a copy of `htui-agent/clippy.toml` (msrv **and** the disallowed list).
- **B-21** (F-20): `PromptPort` holds `Arc<StdMutex<Option<mpsc::Receiver<PromptRequest>>>>` (take-once);
  T2's `McpHost::open` mints `(PromptPort, PromptAsk)` for a `Transport::Cli` scope and keeps the
  `PromptAsk` in the session. Nothing reads the port before T9.

### 0b. Escalation for the maintainer

- **E-1 (D8, I-6; no change made)**: `--mcp-config=<json>` puts `HTUI_MCP_TOKEN` in claude's argv,
  which any local user can read from `/proc/<pid>/cmdline` (or a process list on Windows). It grants
  nothing on its own: the socket lives in a `0700` directory (Unix) and the named pipe's default DACL
  gives other users read access only, so a stranger can neither connect nor write the handshake; the
  token dies with the session. The alternative the CLI also accepts (`--mcp-config <file>`, docs
  `cli-reference`) is a `0600` JSON file in the socket directory. Building as the plan says; switching
  later touches only `cli::argv` and `McpHost::open`.

---

## 1. Build order and lanes

```
Wave 0   T0  SessionSpec.prompt + prompt_bridge types                         (serial, first)
Wave 1   T1  store: fenced writes, item_by_key        ∥  T2  htui-mcp crate      ∥  T3  transports
Wave 2   T4  backlog write tools (after T1, T2)       ∥  T5  `htui mcp` relay (after T2)
Wave 3   T6  engine registration, hosts, output section (after T3, T4, T5)     (serial)
Wave 4   T7  search_concepts (after T6)               ∥  T8  command_run queue + exposure (after T6)
         T9  CLI permission_prompt (after T8)                                  (serial)
         T10 docs (after T9)
```

Lane rules (handoff-run step 3.5): a task edits only its file set (plan, amended by B-8, B-13, B-14,
B-15). Intersections after the amendments:

| Pair | Shared files | Verdict |
|---|---|---|
| T1 ∩ T2 | none (`htui-mcp` uses only existing `WorkerStore` methods in T2) | parallel |
| T1 ∩ T3 | none | parallel |
| T2 ∩ T3 | none | parallel |
| T4 ∩ T5 | none (`Cargo.lock` is T5's only; T4 adds no dependency) | parallel |
| T7 ∩ T8 | none (`worker_cmd.rs` is T7's, `verify.rs` and `model/kind.rs` T8's) | parallel |
| T8 ∩ T9 | `htui-agent/src/cli/mod.rs`, `tests/cli_driver.rs`, `htui-orch/src/engine.rs` | serial |

Postgres: T1 and T8 are the only lanes that regenerate `.sqlx` (never at the same time — they are in
different waves); every Postgres-touching command runs under `flock /tmp/mod11-pg.lock` (§15).

---

## 2. Shared code shapes

Names and signatures are binding; layout inside a function is not.

### 2.1 `htui-agent/src/prompt_bridge.rs` (new, T0 types; T9 behavior)

```rust
//! The CLI permission bridge (MOD-11 D18): the `permission_prompt` MCP tool asks, the CLI session
//! answers through the ordinary `DriverEvent::PermissionRequest` / `answer_permission` pair.

/// Depth of the request channel: one session asks at most one question per tool call.
pub const PROMPT_CAPACITY: usize = 16;

/// Mints a connected pair: the session's receiving end and the tool's asking end, one id.
#[must_use]
pub fn bridge() -> (PromptPort, PromptAsk);

/// The session side, carried in `SessionSpec.prompt`. `Clone` because `SessionSpec` is; the
/// receiver is taken once (`take`). Equality is by `id` (tokio's channels have no `PartialEq`).
#[derive(Clone)]
pub struct PromptPort {
    id: uuid::Uuid,
    rx: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<PromptRequest>>>>,
}
impl PromptPort {
    #[must_use] pub fn id(&self) -> uuid::Uuid;
    /// The receiver, the first time; `None` after (a cloned spec does not get a second stream).
    #[must_use] pub fn take(&self) -> Option<tokio::sync::mpsc::Receiver<PromptRequest>>;
}
impl PartialEq for PromptPort { /* self.id == other.id */ }
impl Eq for PromptPort {}
impl core::fmt::Debug for PromptPort { /* `PromptPort(<id>)` */ }

/// The tool side, kept by `McpHost`'s session for a `Transport::Cli` scope.
#[derive(Clone)]
pub struct PromptAsk { id: uuid::Uuid, tx: tokio::sync::mpsc::Sender<PromptRequest> }
impl PromptAsk {
    #[must_use] pub fn id(&self) -> uuid::Uuid;
    /// Sends one request and waits for its verdict.
    /// # Errors
    /// [`PromptClosed`] when the session dropped the port or the answer.
    pub async fn ask(&self, call: PromptCall) -> Result<PromptVerdict, PromptClosed>;
}
impl core::fmt::Debug for PromptAsk { /* `PromptAsk(<id>)` */ }

/// What the CLI's `--permission-prompt-tool` call carries (probe, §2.7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PromptCall {
    pub tool_name: String,
    pub input: serde_json::Value,
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// One request on the wire between the tool and the session.
#[derive(Debug)]
pub struct PromptRequest { pub call: PromptCall, pub answer: tokio::sync::oneshot::Sender<PromptVerdict> }

/// The answer the tool turns into the CLI's JSON (`{"behavior":"allow","updatedInput":…}` /
/// `{"behavior":"deny","message":…}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptVerdict { Allow, Deny { message: String } }

/// The session is gone: the tool answers `deny` with this sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the agent session ended before the permission was answered")]
pub struct PromptClosed;
```

`lib.rs` gains `pub mod prompt_bridge;` (alphabetical, after `pub mod probe;` at `:119`) and
`pub use prompt_bridge::{PromptAsk, PromptCall, PromptPort, PromptVerdict, bridge as prompt_bridge};`.
`uuid` is already a dependency of `htui-agent` (`Cargo.toml`, "MOD-2 milestone 8"), with `v7` only:
the port id is `Uuid::now_v7()` (an identity, not a secret).

`SessionSpec` (`htui-agent/src/driver.rs:254-283`) gains, after `budget_micros` (`:282`):

```rust
    /// MOD-11 D18: the CLI permission bridge, when `htui`'s MCP server hosts `permission_prompt`
    /// for this session. `None` everywhere else, and on every ACP session.
    pub prompt: Option<crate::prompt_bridge::PromptPort>,
```

and the hand-written `Debug` (`:285-301`) one line before `.finish()`:
`.field("prompt", &self.prompt.as_ref().map(PromptPort::id))`.

### 2.2 `htui-orch/src/tools.rs` (new, T2) — the seam (D4)

```rust
//! The tool-host seam (MOD-11 D4): the engine opens one lease per session; the host behind it
//! (`htui_mcp::McpHost`) speaks MCP. `None` in `EngineParts.tools` keeps every spec as before.

use std::path::PathBuf;
use htui_agent::driver::McpServerSpec;
use htui_agent::prompt_bridge::PromptPort;
use htui_core::model::{BoxId, ItemId, ProjectId, RunId, StepId, Transport, UserId};
use htui_core::prompt::render::HostnameLine;
use htui_core::store::StepFence;

/// Everything a session's tools are scoped to. Built by the engine or the chat runtime; never
/// from tool arguments (I-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolScope {
    pub run_id: RunId,
    pub step_id: StepId,
    pub project_id: ProjectId,
    /// `run.item_id`; `None` for a fresh chat (OQ-8).
    pub item_id: Option<ItemId>,
    pub box_id: BoxId,
    pub user: UserId,
    pub fence: StepFence,
    /// The phase's `output_kind` when non-empty; `None` withholds `document_write` (PRD OQ-5).
    pub output_kind: Option<String>,
    /// `Shown` or `Omitted`, from `settings::resolve_box_hostname` (PRD OQ-3).
    pub hostname: HostnameLine,
    /// D16's resolved exposure; T6 sets `false`, T8 the resolver.
    pub command_queue: bool,
    /// The session's working directory: `command_run`'s cwd root (OQ-7).
    pub cwd: PathBuf,
    /// B-18: the candidate's transport; `Cli` advertises `permission_prompt`.
    pub transport: Transport,
}

/// One registration. Dropping it unregisters the token; later calls answer `session ended`.
pub struct ToolLease {
    pub spec: McpServerSpec,
    /// B-21: `Some` for a `Transport::Cli` scope.
    pub prompt: Option<PromptPort>,
    on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
}
impl ToolLease {
    #[must_use]
    pub fn new(spec: McpServerSpec, prompt: Option<PromptPort>,
               on_drop: impl FnOnce() + Send + Sync + 'static) -> Self;
}
impl Drop for ToolLease { fn drop(&mut self) { if let Some(f) = self.on_drop.take() { f() } } }
impl core::fmt::Debug for ToolLease { /* spec (its Debug redacts env), prompt id; never the token */ }

/// Why a lease could not be opened. The engine fails the step as a driver error (D10).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolHostError {
    #[error("htui's tools cannot be hosted offline: no store to write to")]
    Offline,
    #[error("htui's MCP listener could not start: {0}")]
    Listener(String),
}

/// Object-safe, `Send + Sync`.
pub trait ToolHost: Send + Sync + core::fmt::Debug {
    /// # Errors
    /// [`ToolHostError`].
    fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError>;
    /// B-19: stops the listener, removes the socket and its directory, ends every session.
    fn close(&self);
}
```

`htui-orch/src/lib.rs` gains one line, `pub mod tools;`, after `pub mod status;` (`:44`). No re-export
(the paths read `htui_orch::tools::ToolHost`).

### 2.3 `WriteStore` additions (`htui-core/src/store/traits.rs`)

All on `WriteStore` (end of the trait, after `answer_permission`, `:1821-1829`, under a
`// -- MOD-11` banner) **and** forwarded by `WorkerStore` (`htui-core/src/store/worker.rs`, after
`resolve_command` at `:348-353`). Fenced methods take the fence first after `&self` (`pass_step`,
`:1494`), and the fenced-methods doc (`:2481-2491`) gains `write_step_document`, `add_step_note`,
`propose_link`, `withdraw_link`. No default bodies.

```rust
    // -- MOD-11: agent writes (plan D13, B-4..B-6) ------------------------------------------

    /// D13: [`write_document`](Self::write_document) for a step's own item under its fence. One
    /// transaction; the run's row `FOR SHARE` first, then the item `FOR UPDATE` (run → item, the
    /// `park_step` order). Several calls write several versions ("newest wins").
    /// # Errors
    /// `Constraint(document_needs_a_step())` when `produced_by_step_id` is `None` (before any read);
    /// `NotFound { entity: "run_step" }`; `Fenced { step }`; `Constraint(step_writes_own_item(..))`
    /// when the run's `item_id` is not `new.item_id`; then `write_document`'s own errors.
    async fn write_step_document(&self, fence: StepFence, new: NewDocument) -> Result<Document>;

    /// D13: [`add_note`](Self::add_note) with `via_step_id` required, on the step's own item,
    /// under its fence. Same order of refusals as `write_step_document`
    /// (`note_needs_a_step()` first).
    async fn add_step_note(&self, fence: StepFence, note: NewNote) -> Result<Note>;

    /// D13, B-6: upserts a live `item_link` proposed by `link.step`; revives a tombstone with the
    /// new proposer, keeps a live row's proposer. `updated_at` is the trigger's (Pg) / the clock's
    /// (Mem).
    /// # Errors
    /// `Constraint(self_link(..))` when `from == to` (before any read); `NotFound { run_step }`;
    /// `Fenced`; `Constraint(step_writes_own_item(..))` when `from` is not the run's item;
    /// `NotFound { entity: "item" }` for `to`; `Constraint(link_outside_project(..))` when `to` is in
    /// another project than the run.
    async fn propose_link(&self, fence: StepFence, link: ProposeLink) -> Result<ItemLink>;

    /// D13, B-5: tombstones the live link `(from, to, kind)` when its `proposed_by_step_id` is a step
    /// of `link.step`'s run. Answers the tombstoned row.
    /// # Errors
    /// `NotFound { run_step }`; `Fenced`; `Constraint(step_writes_own_item(..))`; then
    /// `NotFound { entity: "item_link", id: link_key(..) }` when no live row matches, else
    /// `Constraint(link_not_proposed_by_run(..))`.
    async fn withdraw_link(&self, fence: StepFence, link: WithdrawLink) -> Result<ItemLink>;

    /// B-4: the item of `project` whose `key` is `key`; `None` when there is none. A read on
    /// `WriteStore` by the `command_runs` precedent: `WorkerStore`'s reads come from here.
    /// # Errors
    /// The backend's own failures only.
    async fn item_by_key(&self, project: ProjectId, key: &str) -> Result<Option<ItemId>>;
```

T8 adds, under a second `// -- MOD-11 M4` banner (D14, B-16):

```rust
    /// D14: inserts a `queued` row. The row's own fields are the caller's (F-S).
    /// # Errors
    /// `Constraint(command_not_queued())` when `new.status != Queued` or any of `started_at`,
    /// `finished_at`, `exit_code`, `output` is set; then `record_command_run`'s errors.
    async fn enqueue_command(&self, new: NewCommandRun) -> Result<CommandRun>;

    /// D14: admits `id` when it is the oldest `queued` row of its `(box, class)` and fewer than
    /// `limit` rows of that pair are `running`, after reaping that pair's stale `running` rows
    /// (heartbeat older than [`COMMAND_STALE_AFTER`]: `failed`, `finished_at` now, `output` =
    /// [`reaped_note`]). `Ok(None)`: not admitted now (wait and ask again). `limit` 0 reads as 1.
    /// # Errors
    /// `NotFound { entity: "command_run" }`; `Constraint(command_not_claimable(status))` when the
    /// row is no longer `queued` (cancelled, reaped, or another claimant's).
    async fn claim_command(&self, id: CommandRunId, claimant: Uuid, limit: u32)
        -> Result<Option<CommandRun>>;

    /// D14: `heartbeat_at = now` while `id` is `running` under `claimant`. `Ok(false)`: it is not
    /// (reaped or cancelled) — the executor kills its child.
    async fn beat_command(&self, id: CommandRunId, claimant: Uuid) -> Result<bool>;

    /// D14: `running → status` (`done | failed | cancelled`) with `exit_code`, `output` (already
    /// scrubbed and capped) and `finished_at` now, while `claimant` holds it. `Ok(false)`: it does not.
    /// # Errors
    /// `Constraint(command_finish_status(status))` for `queued | running`.
    async fn finish_command(&self, id: CommandRunId, claimant: Uuid, status: CommandRunStatus,
        exit_code: Option<i32>, output: Option<String>) -> Result<bool>;

    /// D14: `queued | running → cancelled`, `finished_at` now. `Ok(false)`: already terminal.
    /// # Errors
    /// `NotFound { entity: "command_run" }`.
    async fn cancel_command(&self, id: CommandRunId) -> Result<bool>;
```

New refusal sentences (free fns beside `references_no_row`, house style; exact text is a pin):

```rust
pub fn document_needs_a_step() -> String  // "an agent document names the step that wrote it (produced_by_step_id)"
pub fn note_needs_a_step() -> String      // "an agent note names the step that wrote it (via_step_id)"
pub fn step_writes_own_item(step: StepId, item: ItemId) -> String
                                          // "step {step} may write only on its run's own item, not {item}"
pub fn self_link(item: ItemId) -> String  // "an item cannot link to itself ({item})"
pub fn link_outside_project(to: ItemId) -> String // "item {to} is outside the run's project"
pub fn link_key(from: ItemId, to: ItemId, kind: LinkKind) -> String // "{from}-{kind}->{to}"
pub fn link_not_proposed_by_run(key: &str) -> String // "link {key} was not proposed by this run"
// T8:
pub fn command_not_queued() -> String
pub fn command_not_claimable(status: CommandRunStatus) -> String // "command is {status}, not queued"
pub fn command_finish_status(status: CommandRunStatus) -> String
pub fn reaped_note() -> String            // "reaped: its host stopped heartbeating (MOD-11 OQ-3)"
pub const COMMAND_HEARTBEAT: std::time::Duration = Duration::from_secs(10);
pub const COMMAND_STALE_AFTER: chrono::TimeDelta = TimeDelta::seconds(30); // 3 × the beat (OQ-3)
```

`htui-core/src/model/link.rs` (T1) gains (paths `htui_core::model::link::…`; `model/mod.rs` is not in
T1's set, so no re-export):

```rust
/// Arguments of `WriteStore::propose_link` (MOD-11 D13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposeLink { pub from: ItemId, pub to: ItemId, pub kind: LinkKind, pub step: StepId }
/// Arguments of `WriteStore::withdraw_link` (MOD-11 D13, B-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawLink { pub from: ItemId, pub to: ItemId, pub kind: LinkKind, pub step: StepId }
```

Implementors (5 `WriteStore`, 3 `WorkerStore`): `MemStore` (`mem.rs:6570` + `State` methods),
`PgStore` (`pg/write.rs:783`), `Writer` (`htui-store/src/writer.rs:320`, `match self { Memory, Online }`
like `:1078-1083`), `UsageSpy` (`htui-agent/src/conformance.rs:746`, `self.inner.x(..).await`),
`SpyStore` (`htui-agent/tests/recorder.rs:434`, same); `WorkerStore for MemStore`
(`htui-core/src/store/worker.rs:485`, `WriteStore::x(self, ..)`), `for PgStore` and `for Writer`
(`htui-store/src/worker.rs:79`, `:370`, same UFCS).

### 2.4 PgStore SQL (T1, T8; bodies in `htui-store/src/pg/write.rs`)

Every new query is a `sqlx::query!`/`query_as!`/`query_scalar!` macro (offline `.sqlx`, H-4). Lock
order in every method: **run (`FOR SHARE OF r`) → item → link/command row**.

```rust
/// F-21: `step` exists and its run carries `fence`'s lease, read `FOR SHARE OF r` in the caller's
/// transaction; answers the run's id, item and project. `step_fence`'s order: `NotFound`, then
/// `Fenced`.
async fn step_scope(conn: &mut PgConnection, step: StepId, fence: StepFence) -> Result<StepScope>;
struct StepScope { run: RunId, item: Option<ItemId>, project: ProjectId }
```
```sql
SELECT r.id AS "run_id: RunId", r.item_id AS "item_id: ItemId", r.project_id AS "project_id: ProjectId",
       r.lease_owner AS "lease_owner?"
  FROM run_step s JOIN run r ON r.id = s.run_id
 WHERE s.id = $1
   FOR SHARE OF r
```

`write_step_document(fence, new)`: refuse `produced_by_step_id = None` → begin → `step_scope(step)` →
`scope.item != Some(new.item_id)` → `Constraint(step_writes_own_item)` →
`SELECT 1 FROM item WHERE id = $1 FOR UPDATE` (**byte-identical** to `write_document`'s, `:5316-5317`,
so its `.sqlx` entry is reused, H-4) → `insert_document(&mut tx, new)` (`:265`) → commit.

`add_step_note(fence, note)`: refuse `via_step_id = None` → begin → `step_scope` → own item → the
`add_note` INSERT (`:5897-5915`, byte-identical text, `.fetch_one(&mut *tx)`) → commit. No explicit item
lock: the FK check's `FOR KEY SHARE` on `item` comes after the run, which keeps the order.

`propose_link(fence, link)`: `from == to` → `Constraint(self_link)` → begin → `step_scope(link.step)` →
`scope.item != Some(link.from)` → own-item refusal →
```sql
SELECT project_id AS "project_id: ProjectId" FROM item WHERE id = $1
```
(none → `NotFound { entity: "item", id: to }`; `!= scope.project` → `Constraint(link_outside_project)`) →
```sql
INSERT INTO item_link (from_item_id, to_item_id, kind, proposed_by_step_id)
VALUES ($1, $2, $3, $4)
ON CONFLICT (from_item_id, to_item_id, kind) DO UPDATE
   SET proposed_by_step_id = CASE WHEN item_link.deleted_at IS NULL
                                  THEN item_link.proposed_by_step_id
                                  ELSE EXCLUDED.proposed_by_step_id END,
       deleted_at          = NULL
RETURNING from_item_id AS "from_item_id: ItemId", to_item_id AS "to_item_id: ItemId",
          kind AS "kind: LinkKind", proposed_by_step_id AS "proposed_by_step_id: StepId",
          created_at, updated_at, deleted_at
```
→ commit. (`updated_at` moves by the `BEFORE UPDATE` trigger, `0001_init.sql:576`; the mirror's cursor
picks it up, `cache/refresh.rs:931-975`.) Confirm `LinkKind` decodes through the `str_enum!` sqlx impl
the reads already use; else select `kind` as text and `LinkKind::from_str`.

`withdraw_link(fence, link)`: begin → `step_scope(link.step)` → own item →
```sql
UPDATE item_link SET deleted_at = clock_timestamp()
 WHERE from_item_id = $1 AND to_item_id = $2 AND kind = $3 AND deleted_at IS NULL
   AND proposed_by_step_id IN (SELECT id FROM run_step WHERE run_id = $4)
RETURNING …same columns…
```
zero rows → `SELECT 1 FROM item_link WHERE from_item_id = $1 AND to_item_id = $2 AND kind = $3 AND
deleted_at IS NULL` → found: `Constraint(link_not_proposed_by_run(link_key))`, else
`NotFound { entity: "item_link", id: link_key }` → commit.

`item_by_key(project, key)`: `SELECT id AS "id: ItemId" FROM item WHERE project_id = $1 AND key = $2`
(`fetch_optional`, pool).

T8 (`claim_command`, one transaction; `$box`, `$class` read from the row first):

```sql
-- 1. the row, locked; NotFound / not-queued refusals decided here
SELECT box_id AS "box_id: BoxId", class, status AS "status: CommandRunStatus"
  FROM command_run WHERE id = $1 FOR UPDATE;
-- 2. one claimant per (box, class) at a time, across processes
SELECT pg_advisory_xact_lock(hashtextextended($1::text || '/' || $2, 0));   -- $1 box, $2 class
-- 3. reap stale rows of the pair (OQ-3)
UPDATE command_run
   SET status = 'failed', finished_at = clock_timestamp(),
       output = COALESCE(output || E'\n', '') || $3                           -- reaped_note()
 WHERE box_id = $1 AND class = $2 AND status = 'running'
   AND COALESCE(heartbeat_at, started_at, queued_at) < clock_timestamp() - make_interval(secs => $4);
-- 4. admission
SELECT count(*) AS "running!" FROM command_run WHERE box_id = $1 AND class = $2 AND status = 'running';
SELECT id AS "id: CommandRunId" FROM command_run
 WHERE box_id = $1 AND class = $2 AND status = 'queued' ORDER BY queued_at, id LIMIT 1;
-- 5. admit when running < max(limit,1) and the oldest queued is $id
UPDATE command_run SET status = 'running', claimed_by = $2, started_at = clock_timestamp(),
       heartbeat_at = clock_timestamp()
 WHERE id = $1
RETURNING …the 12 CommandRun columns…
```
Lock order note: step 1 locks the row before the advisory lock; two claimants of **different** rows of
one pair then serialise on the advisory lock, and neither holds the other's row — no cycle. A claimant
never locks a `run` or `item` row.

`beat_command`: `UPDATE command_run SET heartbeat_at = clock_timestamp() WHERE id = $1 AND
claimed_by = $2 AND status = 'running'` (rows = 1). `finish_command`: same predicate,
`SET status = $3, exit_code = $4, output = $5, finished_at = clock_timestamp()`. `cancel_command`:
`UPDATE … SET status = 'cancelled', finished_at = clock_timestamp() WHERE id = $1 AND status IN
('queued','running')`, zero rows → `SELECT 1 … WHERE id = $1` tells `NotFound` from `Ok(false)`.
`enqueue_command`: the B-16 refusal, then the body of `record_command_run` (`:5232-5260`) verbatim
(byte-identical INSERT → reused `.sqlx` entry).

Migration `htui-store/migrations/0014_command_queue.sql` (T8), in full:

```sql
-- MOD-11 OQ-3 (plan D14): liveness for R-MCP-3's queue. A `running` row whose host crashed holds a
-- class slot until a claim of the same (box, class) sees its heartbeat older than three beats and
-- fails it. NULL on every row written before this migration and on every verify row (MOD-4 writes
-- only `done`/`failed`), so nothing existing is reaped.
ALTER TABLE command_run
    ADD COLUMN claimed_by   UUID,
    ADD COLUMN heartbeat_at TIMESTAMPTZ;
COMMENT ON COLUMN command_run.claimed_by   IS 'the claiming host''s owner id (MOD-11 OQ-3)';
COMMENT ON COLUMN command_run.heartbeat_at IS 'last beat of the claiming host; stale after 3 beats';
```

(`idx_command_run_queue (box_id, class, status, queued_at)`, `0001_init.sql:551`, already serves steps
3–4.) Check `migrations.rs`'s `the_ana_column_comments_are_present_and_verbatim` (`:547`) — if it
enumerates every column comment, add the two.

### 2.5 MemStore reference semantics (`htui-core/src/store/mem.rs`)

- `State` gains (T8) `command_claims: HashMap<CommandRunId, CommandClaim>` with
  `struct CommandClaim { claimant: Uuid, heartbeat_at: DateTime<Utc> }`; `State::delete_project`
  (`:4104`) prunes it with the step cascade, and `delete_project_leaves_no_row_in_any_map` gains the
  assert (H-21).
- A `State::step_scope(&self, step, fence) -> Result<(RunId, Option<ItemId>, ProjectId)>`:
  `require_step` (NotFound `run_step`), `fence_holds(&self.lease_owners, row, fence)` (`:1610`), then
  the run's `item_id`/`project_id`. Every new method decides all refusals before the first mutation.
- `write_step_document` → `step_scope` → own item → `self.write_document(new)` (`:5157`).
  `add_step_note` → same → `self.add_note(note)` (`:5448`).
- `propose_link(.., now)`: `links: Vec<ItemLink>` (`:214`); find `(from,to,kind)`; absent → push with
  `created_at = updated_at = now`, `deleted_at = None`, proposer `Some(step)`; tombstoned → revive,
  proposer = step, `updated_at = now`; live → `updated_at = now` only (B-6).
- `withdraw_link(.., now)`: live row whose proposer's `steps[p].run_id == scope.run` → `deleted_at =
  Some(now)`, `updated_at = now`.
- `item_by_key`: `items.values().find(|i| i.project_id == project && i.key == key)`.
- T8: `enqueue_command` (B-16 refusal → `record_command_run`); `claim_command(.., now)`: reap the pair's
  `running` rows whose claim's `heartbeat_at < now - COMMAND_STALE_AFTER` (`failed`, note,
  `finished_at = Some(now)`, claim removed), count, oldest `(queued_at, id)`, admit (`started_at =
  Some(now)`, claim inserted). `beat_command`/`finish_command` compare the claimant;
  `cancel_command` removes the claim. The `MemStore::write` closure is the advisory lock.

### 2.6 `htui-mcp` crate layout (T2 creates every file; later tasks own one file each)

```
crates/htui-mcp/
  Cargo.toml          lib; deps below
  clippy.toml         B-20: copy of htui-agent/clippy.toml (msrv + disallowed-methods), header updated
  src/lib.rs          crate doc; pub mod channel, host, protocol, search; mod tools; re-exports;
                      pub const RELAY_VERSION, ENV_ADDR = "HTUI_MCP_ADDR", ENV_TOKEN = "HTUI_MCP_TOKEN",
                      SERVER_NAME = "htui"
  src/protocol.rs     JSON-RPC 2.0 / MCP over NDJSON (D2, §2.7)
  src/channel.rs      Address, Listener (unix socket / named pipe), Token, handshake (both sides),
                      relay() for the child (D3, §2.8)
  src/host.rs         McpHost<H>, Session<S>, McpClient (D5, §2.9)
  src/search.rs       ConceptSearch + its plain types (D12, B-3)
  src/tools/mod.rs    ToolDef table of all eight, Ctx, ToolError, dispatch (§2.10) — owned by T2 forever
  src/tools/box_profile.rs   T2 (implemented)
  src/tools/document.rs      T4   (stub in T2: advertised = false)
  src/tools/note.rs          T4
  src/tools/status.rs        T4
  src/tools/link.rs          T4
  src/tools/search.rs        T7
  src/tools/command.rs       T8
  src/tools/permission.rs    T9
  tests/protocol.rs   T2 (transcripts, errors, versions)
  tests/channel.rs    T2 (unix round trip, refusals; cfg(unix))
  tests/transcripts/claude-2.1.287.ndjson   T2 (§2.7, verbatim)
  tests/tools_backlog.rs     T4
  tests/tools_search.rs      T7 (B-14)
  tests/tools_command.rs     T8
  tests/tools_permission.rs  T9
```

`Cargo.toml`:

```toml
[package]
name        = "htui-mcp"
version     = "0.1.0"
description = "htui's MCP server: the per-session tool host and the stdio relay's channel (MOD-11)"
edition.workspace = true  # rust-version, license, publish likewise

[dependencies]
htui-core  = { workspace = true }
htui-agent = { workspace = true }
htui-orch  = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
schemars   = { workspace = true }          # MOD-11 D1: promoted from the lock (1.2.2)
thiserror  = { workspace = true }
tracing    = { workspace = true }
chrono     = { workspace = true }
uuid       = { workspace = true, features = ["v4"] }   # D3 token; v7's `rng` already compiled
tokio      = { workspace = true, features = ["net", "io-util", "sync", "time", "process", "rt",
                                            "macros", "io-std"] }
process-wrap = { workspace = true }        # T8's executor via htui_orch::verify::run_shell (B-15);
                                           # drop if run_shell hides it completely

[target.'cfg(windows)'.dependencies]
# nothing yet: tokio::net::windows::named_pipe is enough

[dev-dependencies]
htui-core  = { workspace = true, features = ["test-support"] }
htui-agent = { workspace = true, features = ["test-support"] }
htui-orch  = { workspace = true, features = ["test-support"] }
htui-store = { workspace = true, features = ["demo", "test-support"] }   # B-1
tokio      = { workspace = true, features = ["macros", "rt", "rt-multi-thread", "test-util", "net"] }
tempfile   = "3"
insta      = { workspace = true }

[lints]
workspace = true
```

Root `Cargo.toml`: `members` gains `"crates/htui-mcp"`; `[workspace.dependencies]` gains
`htui-mcp = { path = "crates/htui-mcp" }` and, commented "MOD-11 D1: already compiled
(agent-client-protocol-schema); promoted, adds no crate", `schemars = "1.2"`. Confirm with
`cargo tree -i schemars@1.2.2` before and after that no new crate appears in `Cargo.lock` beyond
`htui-mcp` itself.

### 2.7 `protocol.rs` — JSON-RPC 2.0 / MCP over NDJSON (D2, B-10, B-11)

```rust
pub const SUPPORTED_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub const MAX_LINE_BYTES: usize = 1024 * 1024;           // mirrors htui-agent/src/cli/mod.rs:76

/// What the protocol loop asks of whoever owns the tools: `McpHost`'s session, or a test double.
pub trait Handler: Send + Sync + 'static {
    /// `tools/list`'s `tools` array, already filtered to the advertised set (I-7).
    fn tools(&self) -> Vec<ToolInfo>;
    /// One call. `progress` is `Some` when the client sent `_meta.progressToken` (B-11).
    fn call(&self, name: String, arguments: serde_json::Value, progress: Option<Progress>)
        -> Pin<Box<dyn Future<Output = Result<CallResult, CallRefused>> + Send>>;
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ToolInfo { pub name: &'static str, pub description: &'static str,
                      #[serde(rename = "inputSchema")] pub input_schema: serde_json::Value }
/// `{content:[{type:"text",text}], isError}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallResult { pub text: String, pub is_error: bool }
/// A call the protocol refuses rather than the tool: unknown or unadvertised name (-32602).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRefused(pub String);
/// Sends `notifications/progress {progressToken, progress}` on the connection's writer.
#[derive(Debug, Clone)] pub struct Progress { /* token: Value, tx: mpsc::Sender<Value> */ }
impl Progress { pub async fn tick(&self, progress: u64); }

/// Serves one connection until EOF; never panics on input. Requests are dispatched concurrently
/// (`contained::spawn_in` into a `JoinSet`, B-20); responses go through one writer task in the order
/// they complete. `notifications/cancelled {requestId}` aborts that request's task (its drop guards
/// run: `command_run` cancels its row). EOF aborts every in-flight task.
pub async fn serve<C>(conn: C, handler: Arc<dyn Handler>) -> std::io::Result<()>
where C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static;
```

Message shapes (server side; every response carries `"jsonrpc":"2.0"` and the request's `id`):

| In | Out |
|---|---|
| `initialize {protocolVersion, capabilities, clientInfo}` | `result {protocolVersion: <client's if in SUPPORTED_VERSIONS else "2025-11-25">, capabilities: {tools: {}}, serverInfo: {name: "htui", version: CARGO_PKG_VERSION}}` |
| `notifications/initialized` (no id) | nothing |
| `ping` | `result {}` |
| `tools/list` (any `cursor` ignored) | `result {tools: [ToolInfo…]}` (no `nextCursor`) |
| `tools/call {name, arguments?, _meta?}` | `result {content: [{type:"text", text}], isError}`; `arguments` absent → `{}` |
| `tools/call` unknown/unadvertised name | `error {code: -32602, message: "unknown tool: <name>"}` |
| `notifications/cancelled {requestId}` | nothing; aborts the call |
| any other notification | ignored |
| unknown method with an id | `error {code: -32601, message: "method not found: <m>"}` |
| a line that is not JSON | `error {id: null, code: -32700, message: "parse error"}` |
| JSON that is not a request object (array, no `method`, bad `jsonrpc`) | `error {id: <id or null>, code: -32600}` |
| a line over `MAX_LINE_BYTES` | the rest of the line discarded, `-32700 "line exceeds 1 MiB"`, loop continues |
| argument decode failure (serde) | `result {isError: true, text: "invalid arguments: <serde message>"}` (MCP convention, D2) |

Calls before `initialize` are served anyway (the CLI always initialises first; refusing buys nothing).

**Recorded transcript** (fact-check probe of claude 2.1.287, `/tmp/mcpprobe/srv.log`, copied here
verbatim because a sandbox restart wipes `/tmp`; T2 writes these four lines to
`tests/transcripts/claude-2.1.287.ndjson` and replays them against a `Handler` double):

```
{"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{"roots":{"listChanged":true},"elicitation":{"form":{},"url":{}}},"clientInfo":{"name":"claude-code","title":"Claude Code","version":"2.1.287","description":"Anthropic's agentic coding tool","websiteUrl":"https://claude.com/claude-code"}},"jsonrpc":"2.0","id":0}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"method":"tools/list","jsonrpc":"2.0","id":1}
{"method":"tools/call","params":{"name":"permission_prompt","arguments":{"tool_name":"mcp__htui__hello","input":{},"tool_use_id":"toolu_01HuMymhasmoPmGaLNFxJ4GV"},"_meta":{"claudecode/toolUseId":"toolu_01HuMymhasmoPmGaLNFxJ4GV","progressToken":2}},"jsonrpc":"2.0","id":2}
```

The probe's answers were `{"content":[{"type":"text","text":"{\"behavior\": \"allow\", \"updatedInput\": {}}"}]}`
(allow) and `{"behavior":"deny","message":"probe denies"}` (deny); a deny then appears in the CLI's
terminal `result.permission_denials[]` as `{"tool_name":"mcp__htui__hello","tool_use_id":"toolu_01Hu…",
"tool_input":{}}` (`/tmp/mcpprobe/out3.jsonl`) — the dedup D18 asks for (T9). The CLI's `system/init`
lists servers as `{"name":"htui","status":"connected","source":"dynamic"}` and never echoes `env`.

### 2.8 `channel.rs` — listener, address, handshake, relay (D3, OQ-1, B-12)

```rust
/// 32 bytes from two `Uuid::new_v4()` (getrandom-backed), lowercase hex, 64 chars. `Debug` prints
/// `Token(…)` — never the value (I-6).
#[derive(Clone, PartialEq, Eq, Hash)] pub struct Token(String);
impl Token { #[must_use] pub fn mint() -> Self; #[must_use] pub fn as_str(&self) -> &str;
             #[must_use] pub fn parse(s: &str) -> Option<Self>; /* 64 lowercase hex or None */ }

/// Where the child connects: a socket path (Unix) or a pipe name (Windows), rendered into
/// `HTUI_MCP_ADDR` as is.
#[derive(Debug, Clone, PartialEq, Eq)] pub struct Address(String);

/// One per process, created lazily by the first `open` (D3). Drop = `close`.
pub struct Listener { address: Address, dir: Option<PathBuf> /* unix */, stop: watch::Sender<bool> }
impl Listener {
    /// Binds and spawns the accept loop (`contained::spawn`). Each accepted stream gets its own task:
    /// read the handshake line (5 s budget, `HANDSHAKE_TIMEOUT`), resolve the token through `lookup`,
    /// answer, then `protocol::serve(stream, session)`.
    pub fn bind(lookup: Arc<dyn Fn(&HandshakeLine) -> Result<Arc<dyn Handler>, Refusal> + Send + Sync>)
        -> std::io::Result<Self>;
    #[must_use] pub fn address(&self) -> &Address;
    pub fn close(&mut self);   // stop the loop; unix: remove `<dir>/s` then `<dir>`, best effort
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandshakeLine { pub token: String, pub version: String }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandshakeReply { pub ok: bool, #[serde(default, skip_serializing_if = "Option::is_none")]
                            pub reason: Option<String> }
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("no live htui session has this token (it ended, or it never existed)")] UnknownToken,
    #[error("version mismatch: the host is htui {host}, this relay is {relay}; restart the agent")]
    Version { host: String, relay: String },
    #[error("malformed handshake")] Malformed,
}

/// The child side (D6): connect, send the line, read the reply, then splice
/// stdin → stream and stream → stdout until either side ends.
pub async fn relay<R, W>(addr: &Address, token: &Token, stdin: R, stdout: W) -> Result<(), RelayError>
where R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send;
#[derive(Debug, thiserror::Error)]
pub enum RelayError { #[error("cannot reach the htui host at {0}: {1}")] Connect(String, std::io::Error),
                      #[error("{0}")] Refused(Refusal), #[error("relay i/o: {0}")] Io(std::io::Error) }
```

Wire: child → host `{"token":"<64 hex>","version":"0.1.0+relay.1"}\n`; host → child `{"ok":true}\n`,
or `{"ok":false,"reason":"<Refusal Display>"}\n` and close. The reason never names the token. After
`ok`, the stream carries the MCP NDJSON unchanged in both directions; the child never parses it (I-2).
The version check compares the whole string (B-12).

**Unix**: base = `$XDG_RUNTIME_DIR` when set, absolute and a directory, else `std::env::temp_dir()`;
directory `htui-mcp-<pid>-<8 hex of a v4>` created with
`std::fs::DirBuilder::new().mode(0o700).create(..)` (`std::os::unix::fs::DirBuilderExt`; no `unsafe`);
then verified `metadata().permissions().mode() & 0o077 == 0` (refuse otherwise:
`ToolHostError::Listener`). Socket `<dir>/s` via `tokio::net::UnixListener::bind`. Keep the name short:
`sun_path` is 108 bytes on Linux, 104 on macOS (H-12). Cleanup on `close`/`Drop`; a crashed process
leaves the directory (no sweeper this item; noted in `docs/htui-mcp.md`).

**Windows**: name `\\.\pipe\htui-mcp-<v4 simple>`; first instance
`ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)`; the accept
loop follows tokio's documented pattern (create the next instance **before** handing the connected one
off). Default security descriptor (OQ-1: "current-user ACL by default"). The child opens with
`ClientOptions::new().open(&name)`, retrying `ERROR_PIPE_BUSY` (`os error 231`) every 50 ms for up to
5 s. Compiled with `cargo check -p htui-mcp --target x86_64-pc-windows-gnu` only (R-10).

### 2.9 `host.rs` — `McpHost<H>` (D5, B-2, B-19, B-21)

```rust
pub struct McpHost<H: htui_core::store::WorkerHost> { inner: Arc<Inner<H>> }
struct Inner<H: htui_core::store::WorkerHost> {
    host: StdMutex<H>,                                              // B-2
    sessions: StdMutex<HashMap<Token, Arc<Session<H::Store>>>>,
    listener: StdMutex<Option<Listener>>,                           // lazy (D3)
    search: Option<Arc<dyn ConceptSearch>>,                         // D12
    binary: PathBuf,                                                // absolute (ACP schema)
    scrubber: Arc<dyn htui_core::scrub::Scrubber>,                  // MinimalScrubber::new([]) default
    clock: Arc<dyn htui_core::clock::Clock>,                        // SystemClock default
}
pub(crate) struct Session<S> {
    pub(crate) scope: ToolScope,
    pub(crate) store: S,                                            // captured at open (B-2)
    pub(crate) ask: Option<PromptAsk>,                              // B-21
    pub(crate) ended: AtomicBool,                                   // I-6
    /* + Arc back-pointers the tools need: search, scrubber, clock, a host clone for reads */
}

impl<H: htui_core::store::WorkerHost> McpHost<H> {
    /// Resolves the binary once: Linux `/proc/<std::process::id()>/exe` (valid for the process's
    /// life even after the file is replaced, the `provision/mod.rs:104-110` precedent); elsewhere
    /// `std::env::current_exe()` (absolute, canonicalised).
    /// # Errors
    /// `ToolHostError::Listener` when no absolute binary path can be had.
    pub fn new(host: H) -> Result<Self, ToolHostError>;
    #[must_use] pub fn with_search(self, search: Arc<dyn ConceptSearch>) -> Self;   // T2 API, T7 uses
    #[must_use] pub fn with_binary(self, binary: PathBuf) -> Self;                   // tests (T5)
    #[must_use] pub fn with_scrubber(self, s: Arc<dyn Scrubber>) -> Self;
    #[must_use] pub fn with_clock(self, c: Arc<dyn Clock>) -> Self;
    pub fn set_host(&self, host: H);                                                 // B-2
    /// The listener's address once bound (tests).
    #[must_use] pub fn address(&self) -> Option<Address>;
    /// D5's in-process client: the same protocol over `tokio::io::duplex(64 KiB)`, no socket and no
    /// handshake.
    /// # Errors
    /// `Refusal::UnknownToken` when `token` names no live session.
    pub fn client(&self, token: &str) -> Result<McpClient, Refusal>;
}
impl<H: WorkerHost> Clone for McpHost<H> { /* Arc clone */ }
impl<H: WorkerHost> core::fmt::Debug for McpHost<H> { /* sessions count, address; no tokens */ }

impl<H: htui_core::store::WorkerHost> ToolHost for McpHost<H> {
    fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError> {
        // 1. store = self.host.lock().writer().ok_or(Offline)?        (B-2)
        // 2. listener: bind lazily (needs a tokio runtime: open is only called from async code)
        // 3. token = Token::mint(); (port, ask) = prompt_bridge() when scope.transport == Cli (B-21)
        // 4. sessions.insert(token, Arc::new(Session{..}))
        // 5. spec = McpServerSpec { name: "htui", command: binary, args: ["mcp"],
        //                           env: {HTUI_MCP_ADDR: address, HTUI_MCP_TOKEN: token} }
        // 6. ToolLease::new(spec, port, move || { remove token; session.ended = true })  (Weak<Inner>)
    }
    fn close(&self) { /* listener.close(); every session ended; map cleared */ }
}

/// The client half `client(token)` returns.
#[derive(Debug)] pub struct McpClient { /* write half, BufReader lines, next id */ }
impl McpClient {
    /// One raw request; answers the whole response object (`result` or `error`).
    pub async fn request(&mut self, method: &str, params: serde_json::Value)
        -> std::io::Result<serde_json::Value>;
    pub async fn initialize(&mut self) -> std::io::Result<serde_json::Value>;
    /// The advertised names, in table order.
    pub async fn tool_names(&mut self) -> std::io::Result<Vec<String>>;
    /// `tools/call`; `Err` only on transport failure; a refusal is `CallResult { is_error: true }`,
    /// an unknown tool `Ok(CallResult { is_error: true, text: "<-32602 message>" })`.
    pub async fn call(&mut self, tool: &str, arguments: serde_json::Value)
        -> std::io::Result<CallResult>;
}
```

Accept/serve tasks and `client`'s server task hold a `Weak<Inner>`; dropping the last `McpHost` ends
them. Every call checks `session.ended` first (I-6) and answers `isError "session ended"`.

### 2.10 `tools/mod.rs` — table, context, errors (fixed in T2)

```rust
pub(crate) struct ToolDef {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) schema: fn() -> serde_json::Value,          // schemars, `$schema` key removed
    pub(crate) advertised: fn(&ToolScope, &HostCaps) -> bool,
}
pub(crate) struct HostCaps { pub(crate) search: bool }
/// Table order is `tools/list` order and a pin.
pub(crate) const ALL: [&ToolDef; 8] = [&box_profile::DEF, &document::DEF, &note::DEF, &status::DEF,
                                       &link::DEF, &search::DEF, &command::DEF, &permission::DEF];
pub(crate) struct Ctx<'a, H: WorkerHost> { pub(crate) session: &'a Session<H::Store>,
                                           pub(crate) host: H, pub(crate) progress: Option<Progress> }
/// One-line reason; becomes `isError: true` (D2, "Tools").
#[derive(Debug, Clone, PartialEq, Eq)] pub(crate) struct ToolError(pub(crate) String);
pub(crate) type ToolResult = Result<serde_json::Value, ToolError>;   // Ok → text = compact JSON
pub(crate) async fn dispatch<H: WorkerHost>(name: &str, ctx: Ctx<'_, H>, args: Value) -> ToolResult {
    match name { "box_profile" => box_profile::call(ctx, args).await, "document_write" => …, /* all 8 */ }
}
pub(crate) fn args<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ToolError>;  // "invalid arguments: …"
pub(crate) fn scrubbed(s: &dyn Scrubber, text: String) -> Result<String, ToolError>;  // I-5, fail closed
pub(crate) fn store_error(err: StoreError) -> ToolError;
```

Every argument struct is `#[derive(Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]`, so a
`run_id` (or any other) argument is refused as `invalid arguments: unknown field …` (I-1). A stub file
is `pub(crate) const DEF: ToolDef = ToolDef { advertised: |_, _| false, .. }` plus
`pub(crate) async fn call<H: WorkerHost>(_: Ctx<'_, H>, _: Value) -> ToolResult { Err(ToolError(
"not available in this build".into())) }`; its owning task replaces both.

**Error mapping** (`store_error`, `scrubbed`, the session check):

| Cause | `text` |
|---|---|
| session ended (lease dropped / `close`) | `session ended` |
| `StoreError::Fenced { .. }` | `fenced: lease lost` |
| `StoreError::NotFound { entity, id }` | `not found: <entity> <id>` |
| `StoreError::Constraint(s)` | `refused: <s>` (the store's sentence, e.g. `link … was not proposed by this run` → tool rewords to `not yours`) |
| `StoreError::Unreachable`/`Backend` | `store unavailable: <msg>` |
| `Unmasked { rule, path }` | `refused: the text matched credential rule <rule>; nothing was written` (never the text) |
| a key not in the scope's project | `out of scope: <key> is not an item of this project` |
| `ConceptSearch` error | `search unavailable: <msg>` (R-STO-8) |

**Per-tool contracts** (advertised predicate · arguments · success JSON):

| Tool (file, task) | Advertised when | Arguments (schemars struct) | Success `text` (JSON) |
|---|---|---|---|
| `box_profile` (`box_profile.rs`, T2) | always | none: `struct BoxProfileArgs {}` | `{"profile": <BoxProfile, hostname key removed when scope.hostname = Omitted>, "text": <render::box_profile(&p, scope.hostname).content>}`; no profile row → `not found: box <id>` |
| `document_write` (`document.rs`, T4) | `item_id.is_some() && output_kind.is_some()` | `{title: Option<String>, body: String}` | `{"document_id": "<uuid>", "kind": "<kind>", "version": n}`; title default = the kind; title and body scrubbed |
| `note_add` (`note.rs`, T4) | `item_id.is_some()` | `{body: String}` | `{"note_id": "<uuid>"}`; body scrubbed then capped: > 16 KiB → `refused: note is N bytes, the limit is 16384` |
| `item_status` (`status.rs`, T4) | `item_id.is_some()` | `{status: Status, reason: String, resolution: Option<Resolution>}` (enums from `str_enum!` `ALL`; `#[schemars(with = "String")]` + `schemars(extend("enum" = …))` or a hand schema) | `{"note_id": "<uuid>"}`; body `status request: <status>[ (<resolution>)] — <reason>`; `resolution` with a status other than `closed` → `refused: a resolution goes with closed only`; `item.status` untouched (I-4) |
| `item_link` (`link.rs`, T4) | `item_id.is_some()` | `{op: "add"\|"remove", to: String (item key), kind: LinkKind}` | `{"from": "<key>", "to": "<key>", "kind": "...", "live": bool}`; `to` via `item_by_key(scope.project_id, to)` → `None` = out of scope |
| `search_concepts` (`search.rs`, T7) | `caps.search` | `{query: String, types: Option<Vec<String>>, statuses: Option<Vec<Status>>, limit: Option<u64> (1..=20, default 10)}` | `{"hits": [ConceptHit…]}` |
| `command_run` (`command.rs`, T8) | `scope.command_queue` | `{class: "build"\|"test"\|"run", command: String, cwd: Option<String>, timeout_secs: Option<u64>}` | `{"exit_code": n\|null, "status": "done"\|"failed"\|"cancelled", "output": "...", "truncated": bool, "command_run_id": "<uuid>"}` |
| `permission_prompt` (`permission.rs`, T9) | `scope.transport == Cli && session.ask.is_some()` | `PromptCall {tool_name, input, tool_use_id?}` (not `deny_unknown_fields`: the CLI may add keys) | the CLI contract: `{"behavior":"allow","updatedInput":<input>}` or `{"behavior":"deny","message":"…"}`, `isError: false` either way |

Descriptions are one sentence each and pinned by an `insta` snapshot of `tools/list` per scope shape
(fresh chat, promoted chat, phase step, CLI phase step).

### 2.11 `search.rs` (T2 trait and types; T7 the tool) — B-3

```rust
pub trait ConceptSearch: Send + Sync + core::fmt::Debug {
    fn search(&self, query: ConceptQuery)
        -> Pin<Box<dyn Future<Output = Result<Vec<ConceptHit>, String>> + Send + '_>>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptQuery { pub text: String, pub project: ProjectId, pub types: Vec<ConceptType>,
                          pub statuses: Vec<Status>, pub limit: u64 }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum ConceptType { Item, Document, Requirement }
impl ConceptType { #[must_use] pub fn parse(s: &str) -> Option<Self>; }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum OwnerKind { Item, Requirement }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ConceptHit {
    pub point_type: ConceptType, pub owner_kind: OwnerKind, pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub document_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub state: Option<String>,
    pub score: f32, pub snippet: String,
}
```

### 2.12 Engine and host shapes (T6, T8, T9)

- `EngineParts` (`htui-orch/src/engine.rs:422-472`) gains, after `tails` (`:471`):
  `/// MOD-11 D4: htui's MCP host; `None` keeps `mcp: Vec::new()` exactly.`
  `pub tools: Option<Arc<dyn crate::tools::ToolHost>>,`. Ten literals gain `tools: None` (plan D4
  list); `fake_parts` (`:6731`) takes `orch.tool_host()`.
- `drive_once` (`:5875-5945`), between the persona `match` (`:5900-5905`) and `driver` (`:5906`):
  ```rust
  // MOD-11 D10, B-17: declared before `session`, so the session drops first.
  let lease = match &self.parts.tools {
      Some(host) => Some(Box::pin(self.open_tools(host.as_ref(), run, step, phase, &candidate,
                                                  &project, &cwd)).await?),     // I-8: boxed
      None => None,
  };
  ```
  and in the spec: `mcp: lease.iter().map(|l| l.spec.clone()).collect(),`
  `prompt: lease.as_ref().and_then(|l| l.prompt.clone()),` (T0 wrote `prompt: None`).
  `open_tools` (new private `async fn`) builds the `ToolScope` (`hostname`:
  `if settings::resolve_box_hostname(Some(&project.settings)) { Shown } else { Omitted }`;
  `output_kind`: `Some(phase.output_kind.clone()).filter(|k| !k.is_empty())` and `None` when
  `run.item_id` is `None`; `fence: StepFence::Lease(self.parts.owner)`; `transport` B-18;
  `command_queue: false` in T6, the resolver in T8) and maps `ToolHostError` to
  `EngineError::Driver(DriverError::Spawn(err.to_string()))`.
- `PromptSpec` (`htui-core/src/prompt/mod.rs:74`) gains, after `command_queue` (`:122`):
  `/// MOD-11 D19: render the protected `output` trailer naming `document_write`.`
  `pub document_tool: bool,`. `phase_spec` (`engine.rs:5560`) and the judge spec (`:4815`) set
  `document_tool: self.parts.tools.is_some() && !phase.output_kind.is_empty()` (phase_spec always has
  an item; the judge's kind is `JUDGE_KIND`).
- D19 text (render.rs, pinned): `pub const OUTPUT_TEXT_PREFIX`… or one fn
  `pub fn output(kind: &str) -> Rendered` whose content is
  ``Write your `<kind>` document by calling the `document_write` tool of the `htui` MCP server; text left only in your reply is not recorded.``
  (`<kind>` substituted; scrubbed like any section).
- `SelectInput` (`htui-orch/src/select.rs:20-35`, T9) gains `pub inline_prompt: bool` ("MOD-11 D18:
  the engine hosts `permission_prompt`; a CLI agent may take a gated phase"). Rule 3 (`:166-171`)
  becomes `if !(caps.permission_requests || caps.edit_proposals
  || (input.inline_prompt && agent.transport == Transport::Cli))`. Fourteen literals
  (`select.rs` ×13, `engine.rs:3236`) gain the field; the engine's is `self.parts.tools.is_some()`.
- `htui-worker`: `Shared` (`runtime.rs:178`) gains `tools: Option<Arc<dyn htui_orch::tools::ToolHost>>`;
  `RunRuntime::with_tool_host(mut self, tools: Arc<dyn ToolHost>) -> Self` beside `with_author`
  (`:1316`, `configure()`'s panic-after-serve contract); `Kit` (`:848`) gains the same field, copied in
  `Kit::read`; `Kit::engine` sets `tools: self.tools.clone()`; `RunRuntime::shutdown` (`:1479`) calls
  `tools.close()` after the tasks end. `ProgressSink` (`views.rs:519-524`) gains `pub(crate) owner:
  Uuid` (literal at `runtime.rs:919`: `owner: shared.owner`) and `after_done` writes
  `self.writer.write_step_document(StepFence::Lease(self.owner), document)` (UFCS through
  `htui_core::store::WorkerStore`, H-1).
- `htui` chat (`agent_worker.rs`): `AgentRuntime` (`:417`) gains `tools: Option<Arc<dyn ToolHost>>` and
  `pub fn with_tool_host(mut self, tools: Arc<dyn ToolHost>) -> Self` beside
  `with_registration_probe` (`:652`); `ChatArgs` (`:2241`) gains `lease: Option<ToolLease>` (literals
  `:1071`, `:2163`; destructure `:3930` keeps it alive in the chat task until it returns).

---

Every task below: **tests first** (written and run red before the code), each commit compiles and
carries its tests, 2–5 commits per task, explicit-path staging, commit trailer
`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Gate helpers (`nopg`, `pg`, `lint`, `regen`,
`check`) are defined in §15.

## 3. T0 — `SessionSpec.prompt` and the bridge types (Wave 0, serial)

### 3.1 Files
`htui-agent/src/prompt_bridge.rs` (new), `htui-agent/src/lib.rs`, `htui-agent/src/driver.rs`, and the
ten literals: `htui-agent/src/conformance.rs:305`, `htui-agent/tests/relay.rs:196`,
`tests/cli_driver.rs:52`, `tests/acp_driver.rs:98`, `tests/driver_contract.rs:145`,
`tests/extensibility.rs:537`, `tests/agy_live.rs:799`, `htui-orch/src/engine.rs:5908`,
`htui/src/agent_worker.rs:1037`, `:2069`. Each literal gains `prompt: None,` after `budget_micros`.

### 3.2 Tests (written first)
`prompt_bridge.rs` `mod tests` (`#[tokio::test]`):
- `a_port_equals_its_clone_and_no_other_port` — `PartialEq` by id; two `bridge()` calls differ.
- `take_yields_the_receiver_once` — second `take()` (also through a clone) is `None` (B-21).
- `an_ask_reaches_the_taken_receiver_and_returns_its_verdict` — `Allow` and `Deny { message }`.
- `an_ask_after_the_port_is_dropped_is_closed` — `Err(PromptClosed)`, and when the request's
  `answer` sender is dropped unanswered.
- `debug_prints_the_id_and_nothing_else` — `PromptPort(<uuid>)`, `PromptAsk(<uuid>)`.
`tests/driver_contract.rs`:
- `session_spec_debug_names_the_prompt_port_by_id` — `spec.prompt = Some(port)` → the `Debug` string
  contains `prompt: Some(<id>)`; with `None`, `prompt: None`; env values still `[REDACTED]`.

### 3.3 Commits
1. `feat(mod-11): T0 prompt bridge types` — `prompt_bridge.rs`, `lib.rs`, its tests.
2. `feat(mod-11): T0 SessionSpec.prompt` — `driver.rs` field + `Debug`, the ten literals, the
   `driver_contract` test.

### 3.4 Gate (G-T0)
`cargo check --workspace --all-targets --all-features`; `nopg htui-agent`; `lint htui-agent`;
`cargo fmt --all -- --check`.

---

## 4. T1 — fenced agent writes in the store (Wave 1, the only Postgres lane)

### 4.1 Files (plan, unchanged; B-4 fits inside them)
`htui-core/src/store/{traits,mem,worker,conformance}.rs`, `htui-core/tests/mem_store.rs`,
`htui-core/src/model/link.rs`, `htui-store/src/pg/write.rs`, `htui-store/src/{writer,worker}.rs`,
`htui-store/tests/{pg_conformance,pg_criteria}.rs`, `htui-store/.sqlx/*`,
`htui-agent/src/conformance.rs` (`UsageSpy`), `htui-agent/tests/recorder.rs` (`SpyStore`).

### 4.2 Edits
- §2.3 methods and sentences in `traits.rs` (after `answer_permission`, `:1821-1829`); fenced-methods
  doc `:2481-2491` extended; `ProposeLink`/`WithdrawLink` in `model/link.rs` (after `ItemLink`, `:41`).
- `MemStore` per §2.5: `State` methods next to `add_note` (`:5448`) and `cite` (`:5982`); async
  wrappers next to `add_note` (`:7193`), each `let now = self.now(); self.write(|s| s.x(.., now))`.
- `WorkerStore` (`htui-core/src/store/worker.rs:100-354`): five forwarder signatures after
  `resolve_command` (`:348`), `/// [`WriteStore::x`].` each; `impl WorkerStore for MemStore` (`:485`)
  UFCS bodies after `:727`.
- `PgStore`: `step_scope` beside `step_fence` (`pg/write.rs:197`); bodies §2.4 after `cite`/`uncite`
  (`:6186`, `:6268`) in their own `// ---- MOD-11 (D13) ----` block. `Writer`: five `match self`
  forwarders after `add_note` (`writer.rs:1145`). `htui-store/src/worker.rs`: forwarders in both
  `WorkerStore` impls (after `:313` and `:604`).
- `UsageSpy` (`htui-agent/src/conformance.rs`, after `add_note` `:1259`) and `SpyStore`
  (`tests/recorder.rs`, after `:985`): `self.inner.x(..).await`.

### 4.3 Tests (written first)
Conformance cases (`CASES` `:53` and `run_case` `:199`, appended in this order; names never change):

| Case | Pins |
|---|---|
| `write_step_document_fenced_and_versioned` | under `Lease(owner)` on the step's own item: v1, then v2 from the same step (judge, "newest wins"); after `take_lease` by a stranger: `Fenced { step }` and no new row; `Unleased` on a leased run: `Fenced` |
| `write_step_document_refuses_a_foreign_item` | `produced_by_step_id: None` → `Constraint(document_needs_a_step())` before any read; another item → `Constraint(step_writes_own_item(..))`; unknown step → `NotFound { run_step }` |
| `add_step_note_fenced_on_its_own_item` | note lands with `via_step_id`; foreign item refused; stale fence → `Fenced`; `via_step_id: None` → `note_needs_a_step()` |
| `propose_link_upserts_revives_and_keeps_a_live_proposer` | new link live with proposer = step; re-propose of a live importer link (`proposed_by_step_id` NULL, seeded by the demo's links) keeps `None` (B-6); after a withdraw, re-propose revives with the new proposer; `from = to` → `self_link`; `to` of another project → `link_outside_project`; `from` ≠ run item → `step_writes_own_item` |
| `withdraw_link_only_what_this_run_proposed` | own proposal → tombstoned (`deleted_at` set, `links()` no longer shows it); a live link proposed by another run's step → `Constraint(link_not_proposed_by_run)`; no live row → `NotFound { item_link }` |
| `item_by_key_answers_within_its_project` | the demo key in its project → `Some`; the same key asked of another project → `None`; unknown key → `None` |

Counts: `htui-core/tests/mem_store.rs:36` `134` → `140` (message extended "…, and MOD-11 T1's six
fenced-write cases (plan D13, B-4)"); `htui-store/tests/pg_conformance.rs:27` `EXPECTED_CASES = 140`
(doc sentence + assert message likewise).

Postgres only, `htui-store/tests/pg_criteria.rs`:
- `a_step_document_racing_a_park_never_deadlocks` — two independent pools (`:646` precedent), fifty
  rounds of `tokio::join!(park_step(Lease(owner), step), write_step_document(Lease(owner), doc))` on a
  fresh running step each round; every pair completes (no `40P01`, `StoreError::Backend` containing
  `deadlock` fails the test), and each round ends in one of the two legal states (document written then
  parked, or parked then document written — the fence still holds after a park because the lease is
  not released by `park_step`).

(Doc rule, H-7: the case docs in `conformance.rs` may name `pg_criteria.rs::a_step_document_racing_a_park_never_deadlocks`
only in the commit that adds that test or later.)

### 4.4 Commits
1. `feat(mod-11): T1 fenced agent-write contract and MemStore reference` — traits, sentences, model,
   `MemStore`, `WorkerStore` + every forwarder, the six cases, both counts; `PgStore` bodies are
   `Err(StoreError::Backend("MOD-11 T1: not yet implemented".into()))` (no SQL macro yet, so
   `SQLX_OFFLINE` still builds). Pg conformance is red at this commit; Mem is green.
2. `feat(mod-11): T1 PgStore fenced writes` — §2.4 SQL, `step_scope`, `.sqlx` regenerated (`regen`).
3. `test(mod-11): T1 Pg race of a step document against park_step` — `pg_criteria.rs`.

### 4.5 Gate (G-T1)
`nopg htui-core`; `nopg htui-agent`; `regen` then `check`; `pg htui-store`; `lint htui-core`;
`lint htui-store`; `lint htui-agent`; `cargo fmt --all -- --check`;
`grep -rn "MOD-11 T1: not yet implemented" crates` empty; `git status --short crates/htui-store/.sqlx`
lists only added files (a modified entry means a byte-identical reuse was missed — fine, but say so).

---

## 5. T2 — the `htui-mcp` crate, the seam, `box_profile` (Wave 1)

### 5.1 Files
`Cargo.toml` (members, `htui-mcp`, `schemars`), `Cargo.lock`, `crates/htui-mcp/**` (§2.6, including
`clippy.toml` B-20 and all eight tool files), `htui-orch/src/tools.rs` (new), `htui-orch/src/lib.rs`
(one line).

### 5.2 Tests (written first)
`htui-orch/src/tools.rs` `mod tests`:
- `dropping_a_lease_runs_its_unregister_once`
- `lease_debug_prints_no_env_value` (the token never appears).

`crates/htui-mcp/tests/protocol.rs` (a `Handler` double over `tokio::io::duplex`):
- `the_recorded_claude_transcript_is_answered` — §2.7's four lines → three responses (ids 0, 1, 2),
  `protocolVersion` `2025-11-25`, nothing for the notification.
- `initialize_echoes_each_supported_version`; `initialize_answers_the_newest_for_an_unknown_version`.
- `ping_answers_an_empty_result`; `an_unknown_method_is_minus_32601`.
- `a_non_json_line_is_minus_32700_and_the_next_is_served`; `a_batch_array_is_minus_32600`.
- `an_oversized_line_is_refused_and_the_next_is_served` (1 MiB + 1).
- `an_unadvertised_tool_is_minus_32602`.
- `bad_arguments_are_is_error_not_a_protocol_error`.
- `a_slow_call_does_not_block_ping` (concurrent dispatch).
- `a_cancelled_call_is_aborted_and_answered_nothing` (its drop guard observed).
- `progress_ticks_carry_the_clients_token` (B-11).

`crates/htui-mcp/tests/channel.rs` (`#![cfg(unix)]`; `McpHost<Backend>` over
`Backend::memory(MemStore::demo())`, B-1):
- `a_unix_round_trip_serves_initialize_after_the_handshake`
- `the_socket_directory_is_private` (mode `0o700`, socket inside, name under 108 bytes)
- `a_bad_token_is_refused_with_a_reason_and_closed` (reason has no token)
- `a_version_mismatch_is_refused_naming_both_versions`
- `a_dropped_lease_refuses_new_connections_and_ends_open_ones` (`session ended`)
- `close_removes_the_socket_and_its_directory`

`src/channel.rs` `mod tests`: `a_silent_child_is_dropped_after_the_handshake_budget` (crate-private
constructor with a 100 ms budget), `a_token_is_64_lowercase_hex_and_debug_hides_it`.

`src/host.rs` `mod tests`:
- `the_spec_names_htui_the_binary_mcp_and_two_env_vars`
- `two_opens_mint_two_tokens`
- `a_cli_scope_gets_a_prompt_port_and_an_acp_scope_none` (B-21)
- `on_linux_the_binary_is_proc_pid_exe` (`#[cfg(target_os = "linux")]`) and
  `the_binary_is_absolute`
- `a_session_keeps_the_store_it_was_opened_on` (B-2: `set_host` to a second demo store; the old
  session still reads the first)
- `client_refuses_an_unknown_token`

`src/tools/box_profile.rs` `mod tests` and an `insta` snapshot:
- `box_profile_omits_the_hostname_when_the_switch_is_off` (no `hostname` key, text starts `os:`)
- `box_profile_shows_the_hostname_when_on`
- `box_profile_text_is_the_prompts_render` (`== render::box_profile(&p, scope.hostname).content`)
- `an_argument_naming_a_run_is_refused` (I-1)
- `tools_list_per_scope_shape` — snapshot of `tools/list` for four scopes; in T2 only `box_profile`
  is advertised (every stub answers `false`).

### 5.3 Commits
1. `feat(mod-11): T2 tool-host seam in htui-orch` — `tools.rs`, `lib.rs`, its tests.
2. `feat(mod-11): T2 htui-mcp crate and the MCP protocol` — workspace `Cargo.toml`, `Cargo.lock`,
   crate skeleton, `clippy.toml`, `protocol.rs`, `search.rs`, `tools/mod.rs` + eight stubs,
   `tests/protocol.rs`, the transcript file.
3. `feat(mod-11): T2 channel, handshake and McpHost` — `channel.rs`, `host.rs`, `tests/channel.rs`.
4. `feat(mod-11): T2 box_profile` — `tools/box_profile.rs`, snapshot.

### 5.4 Gate (G-T2)
`nopg htui-mcp`; `cargo test -p htui-orch --all-features --lib tools -- --test-threads=1`;
`lint htui-mcp`; `lint htui-orch`; `cargo check -p htui-mcp --target x86_64-pc-windows-gnu`;
`cargo fmt --all -- --check`; `cargo tree -p htui-mcp -i schemars@1.2.2 -e normal` resolves;
`git diff c4bf516c -- Cargo.lock` adds only the `htui-mcp` package (and its dependency list).

---

## 6. T3 — transports carry `spec.mcp` (Wave 1)

### 6.1 Files (plan)
`htui-agent/src/acp/mod.rs`, `htui-agent/src/cli/mod.rs`, `htui-agent/src/fake.rs`,
`htui-agent/tests/acp_driver.rs`, `htui-agent/tests/cli_driver.rs`.

### 6.2 Edits
- **ACP** (`acp/mod.rs:1108-1109`): `NewSessionRequest::new(spec.cwd.clone())
  .additional_directories(spec.extra_dirs.clone()).mcp_servers(spec.mcp.iter().map(acp_server).collect())`
  with `fn acp_server(spec: &McpServerSpec) -> McpServer { McpServer::Stdio(McpServerStdio::new(
  spec.name.clone(), PathBuf::from(&spec.command)).args(spec.args.clone()).env(spec.env.iter()
  .map(|(k, v)| EnvVariable::new(k.clone(), v.clone())).collect())) }` (D7; `McpServer` is
  `#[serde(untagged)]`, so no `type` key on the wire). Import through `agent_client_protocol`'s
  re-exports (the compile probe's paths).
- **CLI** (`cli/mod.rs`): `pub fn mcp_config(servers: &[McpServerSpec]) -> Option<String>` —
  `None` for an empty slice; else `serde_json::json!({"mcpServers": {name: {"type": "stdio",
  "command", "args", "env"}}})` serialised compactly (`env` a `BTreeMap`, so key order is stable).
  `argv` (`:116`) pushes `format!("--mcp-config={config}")` **after** the pair block (`:143-163`) and
  **before** `--tools` (`:169`); the numbered doc list above `argv` gains the step. No
  `--strict-mcp-config`; `--tools` untouched (D8).
- **Fake** (`fake.rs`): `#[derive(Debug, Clone, Default)] pub struct SpecSlot(Arc<Mutex<Option<SessionSpec>>>)`
  with `#[must_use] pub fn get(&self) -> Option<SessionSpec>`; `FakeDriver` (`:99`) gains
  `spec: SpecSlot` (constructors initialise it), `pub fn spec_handle(&self) -> SpecSlot`,
  `#[must_use] pub fn with_spec_slot(self, slot: SpecSlot) -> Self`; `start` (`:158`) stores
  `spec.clone()` in the slot before the `async move` (the lock is dropped before the future, as the
  script slot's is). `FakeAdapter` (`:643`) gains `spec: SpecSlot` and `spec_handle()`; `build`
  (`:664-688`) hands it to the driver. Export `SpecSlot` from `lib.rs` beside `FakeDriver` (`:197`) —
  `lib.rs` is not in T3's set: reach it as `htui_agent::fake::SpecSlot` instead (no edit).

### 6.3 Tests (written first)
`tests/acp_driver.rs` (a clone of `fs_agent`, `:492-541`, answering `session/new` and sending the
`params` back through a oneshot):
- `session_new_carries_the_mcp_server_as_stdio` — `mcpServers == [{"name":"htui","command":"/abs/htui",
  "args":["mcp"],"env":[{"name":"HTUI_MCP_ADDR","value":…},{"name":"HTUI_MCP_TOKEN","value":…}]}]`, no
  `type` key.
- `session_new_without_mcp_sends_an_empty_list`.
`tests/cli_driver.rs`:
- `argv_without_mcp_is_unchanged` (byte-equal to today's argv for the existing `spec`)
- `argv_with_mcp_has_one_joined_config_before_tools` (exactly one arg starting `--mcp-config=`;
  index < the `--tools=` index; `--tools=` value unchanged)
- `the_mcp_config_is_the_clis_stdio_shape` (parse the JSON back)
- `extra_args_stay_last_with_mcp`
`fake.rs` `mod tests`: `the_spec_handle_sees_the_started_spec`,
`the_adapter_handle_sees_the_built_drivers_spec`.

### 6.4 Commits
1. `feat(mod-11): T3 ACP session/new carries htui's MCP server`
2. `feat(mod-11): T3 claude argv --mcp-config`
3. `feat(mod-11): T3 FakeDriver records its SessionSpec`

### 6.5 Gate (G-T3)
`nopg htui-agent`; `lint htui-agent`; `cargo fmt --all -- --check`.

---

## 7. T4 — backlog write tools (Wave 2, after T1 and T2)

### 7.1 Files (plan)
`crates/htui-mcp/src/tools/{document,note,status,link}.rs`, `crates/htui-mcp/tests/tools_backlog.rs`.

### 7.2 Behavior (§2.10 table; calls are UFCS on `htui_core::store::WorkerStore`, H-1)
- `document_write`: `NewDocument { id: DocumentId::new(), item_id: scope.item_id?, kind:
  scope.output_kind?, title: scrubbed(title.unwrap_or(kind)), body: scrubbed(body),
  produced_by_step_id: Some(scope.step_id), created_by: scope.user, created_at: clock.now() }` →
  `write_step_document(scope.fence, new)`.
- `note_add`: scrub, cap 16 384 bytes after scrub, `NewNote { via_step_id: Some(step), created_by:
  user, box_id: Some(scope.box_id), .. }` → `add_step_note(scope.fence, note)`.
- `item_status`: parse (`Status`, `Resolution` through serde of their `str_enum!` names), body
  `format!("status request: {status}{res} — {reason}")` with `res = " ({resolution})"` or empty;
  scrub; `add_step_note`. Never `transition` (I-4).
- `item_link`: `to` → `item_by_key(scope.project_id, &to)` (`None` → `out of scope`), `add` →
  `propose_link(fence, ProposeLink { from: item, to, kind, step })`, `remove` →
  `withdraw_link(fence, WithdrawLink { .. })`; a `link_not_proposed_by_run` refusal reads
  `not yours: <from-key> <kind> <to-key> was not proposed by this run`.

### 7.3 Tests (written first, `tests/tools_backlog.rs`, `McpHost<Backend>` over a `MemStore` seeded
with one running, leased step on a demo item — `create_run` + `create_step` + `claim_run(owner)` as the
run-seam conformance cases do — and `client(token)`)
- `document_write_writes_the_phase_kind_on_the_scope_item` (v1 then v2; produced_by, created_by)
- `document_write_is_not_advertised_without_an_item_or_a_kind`
- `document_write_after_the_lease_moved_is_fenced` (`take_lease` by a stranger → `fenced: lease lost`,
  no row) — I-3
- `document_write_masks_a_known_secret_and_refuses_an_unmaskable_body` (a `Scrubber` double whose
  `scrub` answers `Unmasked` → `refused: …`, nothing written) — I-5
- `note_add_writes_a_note_via_the_step`; `note_add_over_16_kib_is_refused`
- `item_status_writes_a_note_and_never_moves_the_status` (`item.status` and `version` unchanged)
- `item_status_rejects_an_unknown_status_and_a_stray_resolution`
- `item_link_add_resolves_the_key_in_the_project`
- `item_link_to_another_projects_key_is_out_of_scope`
- `item_link_to_itself_is_refused`
- `item_link_remove_of_a_link_this_run_did_not_propose_is_not_yours`
- `item_link_remove_of_its_own_proposal_tombstones_it`
- `every_backlog_tool_after_the_session_ended_answers_session_ended` — I-6
- `no_backlog_tool_accepts_a_run_project_or_item_id_argument` — I-1
- `tools_list_per_scope_shape` snapshot updated (the four tools appear where §2.10 says)

### 7.4 Commits
1. `feat(mod-11): T4 document_write and note_add`
2. `feat(mod-11): T4 item_status and item_link`

### 7.5 Gate (G-T4)
`nopg htui-mcp`; `lint htui-mcp`; `cargo fmt --all -- --check`;
`cargo insta test -p htui-mcp --all-features` nothing pending.

---

## 8. T5 — `htui mcp`, the stdio relay (Wave 2, after T2)

### 8.1 Files (plan + B-13)
`htui/src/cli.rs`, `htui/src/lib.rs`, `htui/src/mcp_cmd.rs` (new), `htui/tests/mcp_stdio.rs` (new),
`htui/Cargo.toml`, **`htui/src/main.rs`**, **`Cargo.lock`**.

### 8.2 Edits
- `cli.rs` `Command` (`:77-89`) gains, last:
  `/// Serve htui's MCP tools to one agent session over stdio. Started by the agent htui launched,
  never by hand: it reads HTUI_MCP_ADDR and HTUI_MCP_TOKEN, writes only MCP to stdout, and exits when
  the agent closes stdin.` `Mcp,`
- `lib.rs` `run` (`:84`): **first** statement:
  `if matches!(args.command, Some(cli::Command::Mcp)) { return mcp_cmd::run().await.map_err(anyhow::Error::from); }`
  — before the `Worker` arm and `init_tracing`; nothing is logged, nothing printed (stdout is the
  protocol, H-24). `pub mod mcp_cmd;` in the module list (`:12-41`, alphabetical).
- `mcp_cmd.rs`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum McpExit { MissingEnv(String), Refused(String), Failed(String) }
  impl McpExit { #[must_use] pub const fn code(&self) -> u8 { /* 2, 3, 1 */ } }
  impl core::fmt::Display for McpExit { /* the sentence */ }
  impl std::error::Error for McpExit {}
  /// Reads the two variables, then `htui_mcp::channel::relay(&addr, &token, stdin, stdout)`.
  /// # Errors
  /// `MissingEnv` (a variable unset, empty, or a token that is not 64 hex); `Refused` (the host's
  /// reason); `Failed` (connect or i/o).
  pub async fn run() -> Result<(), McpExit>;
  ```
- `main.rs` (`:69-80`): `.or_else(|| error.downcast_ref::<htui::mcp_cmd::McpExit>().map(McpExit::code))`;
  `reports_to_sentry` (`:91-96`) adds `&& error.downcast_ref::<htui::mcp_cmd::McpExit>().is_none()`
  (an agent's relay ending is never a crash report); `only_crashes_are_reported_to_sentry` gains the
  three `McpExit` asserts.
- `htui/Cargo.toml`: `htui-mcp = { workspace = true }` (comment "MOD-11 D6: `htui mcp`"); `tokio`
  features gain `"io-std"` explicitly (H-15).

### 8.3 Tests (written first, `htui/tests/mcp_stdio.rs`, `#![cfg(unix)]`)
Host: `McpHost::new(Backend::memory(MemStore::demo()))?.with_binary(env!("CARGO_BIN_EXE_htui"))`,
one `open`ed scope; child: `tokio::process::Command::new(env!("CARGO_BIN_EXE_htui")).arg("mcp")`
with `env_clear()` + `PATH` + the two variables, `kill_on_drop(true)` (reaped on every path):
- `the_relay_serves_initialize_list_and_box_profile` — three requests in, three responses out;
  **every stdout line parses as a JSON-RPC response** (nothing else on stdout); exit 0 after stdin
  closes.
- `a_refused_token_exits_3_with_the_reason_on_stderr` (stderr has the sentence, not the token)
- `missing_env_exits_2`
- `a_dead_host_exits_1` (address of a removed socket)
`main.rs` `mod tests`: `only_crashes_are_reported_to_sentry` (extended), `mcp_exit_codes_are_2_3_1`.

### 8.4 Commits
1. `feat(mod-11): T5 htui mcp subcommand and exit mapping` — `cli.rs`, `lib.rs`, `mcp_cmd.rs`,
   `main.rs`, `htui/Cargo.toml`, `Cargo.lock`, unit tests.
2. `test(mod-11): T5 stdio relay end to end` — `tests/mcp_stdio.rs`.

### 8.5 Gate (G-T5)
`cargo test -p htui --all-features --test mcp_stdio -- --test-threads=1`;
`cargo test -p htui --all-features --bin htui`; `lint htui`; `cargo fmt --all -- --check`;
`ps -eo pid,args | grep '[h]tui mcp'` empty afterwards (no orphaned relay).
