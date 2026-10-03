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

