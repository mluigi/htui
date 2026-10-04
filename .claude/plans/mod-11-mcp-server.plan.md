# Plan: MOD-11 — htui MCP server

**Status: IMPLEMENTED 2026-10-04 (T0–T10 and review round R1; write-up `docs/decisions/mod/mod-11.md`). CONFIRMED by the maintainer 2026-10-03 ("yes" = every recommended default, OQ-1…OQ-10). Fact-checked 2026-10-03 (workflow `wf_33a62d14-658`, amendments in place).**

**Implementation amendments (2026-10-04; detail in the write-up):**
- **D16 amended by review R1-H1:** `command_run` is exposed when `base && persona && !deny_kinds ∋ execute`; an execute-denying persona loses it, and the prompt's persona term uses the same helper (`persona_keeps_command_run`). R1-C adds that an engine with no tool host exposes it to no step.
- **I-3 reads "every item write is fenced".** `command_run` is not fenced (its queue rows take no lease); a lease check and a cancel on session end are MOD-78.
- **D20 (new, review R1-M2): htui's own tools are pre-approved.** `htui_orch::tools::pre_approve` appends an `allow_once` rule per advertised tool except `command_run` and `permission_prompt`, after the R-MCP-4, persona and agent rules, and only under an `ask` default.
- **D13's lock order is step → run → item.** A run-only lock deadlocked against `park_step` (40P01) in T1's Pg race test; the fenced writes lock `FOR SHARE OF s, r` through `step_scope`. The older writers still lock run first: MOD-77.

**Source PRD**: `.claude/prds/mod-11-mcp-server.prd.md`, **all five milestones** (MOD-42 precedent:
one plan, milestone-ordered tasks), with its resolved questions (cited **PRD OQ-1…7**).
Contracts: `docs/ANA-2.md:372-373`, `:424-431`, `:476-482`, `:496-506`, risks 4/10/11
(`:2119-2126`); `docs/ANA-5.md:337`, `:509-536`; `docs/ANA-4.md:489-503`, `:546-553`,
`:1385-1386`; `docs/ANA-9.md:59-60`, `:623-645`, `:790-805`; `docs/ANA-16.md:69-70`, `:188`,
`:415-416`, `:774-775`.

**Requirements**: `R-MCP-1..4` (without `spawn_subagent` → MOD-27); constrained by `R-ENT-8`
(no agent transitions), `R-ENT-9` (links only via MCP/importer), `R-ID-7`/`R-SEC-3` (scrub on
host, fail closed), `R-STO-8` (scoped search, clear failure), `R-NF-1` (Windows), `R-NF-2` (no
daemon).

**Complexity**: Large. One new crate (`htui-mcp`), one migration (`0014`), ~9 new `WriteStore`
methods with conformance cases and forwarders, transport changes in both drivers, an engine
registration hook, host wiring in `htui-worker` and `htui`, a new `htui mcp` subcommand, `.sqlx`
regeneration, prompt-digest churn from the exposure fix.

**Routing**: `/handoff-run MOD-11`, PRD path (C2, C3, C4). Ultracode accepted for implement and
review. Reviewer: `rust-reviewer` (`.claude/workflow-config.json`).

**Numbering**: decisions **D1…**, tasks **T0…**, risks **R-1…**, open questions **OQ-1…**,
invariants **I-1…**. The PRD's are **PRD OQ-1…**.

**Tree reading**: HEAD `95adf87f` (branch `hr/MOD-11`, sandbox). Evidence survey plus four
parallel readers (transport, host/engine, store, docs). Gortex answers here despite its
"INACTIVE" banner. Paths relative to `crates/` for code, the repo root for docs.

---

## Open questions for the maintainer (read these first)

- **OQ-1 · The host listens on a private local socket.** One listener per htui process (TUI or
  `htui worker`): a Unix domain socket in a fresh `0700` directory under the runtime/temp dir, a
  named pipe (`\\.\pipe\htui-mcp-<uuid>`, first-instance, current-user ACL by default) on Windows.
  Never a TCP port. ANA-16 §5.4 says "do not add a worker listener socket" — that line is about a
  network-reachable control plane (O3); this socket is local, per process, dies with it, and every
  connection must present a 256-bit per-session token. **Recommended: accept, record as a scoped
  reading of ANA-16 §5.4.**
- **OQ-2 · Hand-rolled MCP, no `rmcp`.** The server needs `initialize`,
  `notifications/initialized`, `ping`, `tools/list`, `tools/call` over NDJSON. Everything needed is
  in the lock (`serde_json`, `schemars` 1.2.2 transitively, tokio). `rmcp` is a new dependency tree
  with an unchecked MSRV against the pinned 1.98.1. **Recommended: hand-roll, ≈400 lines, tested
  against recorded client transcripts.**
- **OQ-3 · `command_run` gets liveness columns (migration `0014`).** A `running` row orphaned by a
  crashed host holds a class slot forever; reaping through the run lease fails for chat runs
  (always unleased). `0014` adds `claimed_by UUID NULL` and `heartbeat_at TIMESTAMPTZ NULL`; a
  claim treats a `running` row with `heartbeat_at` older than 3× the beat interval as dead
  (`failed`, `resolution` note in `output`). **Recommended: accept.**
- **OQ-4 · Every agent write is fenced, not only documents.** `note_add`, `item_status`,
  `item_link` go through new fenced methods (`StepFence::Lease(owner)` on engine sessions,
  `Unleased` on chat), so a step that lost its lease writes nothing at all (MOD-42 A-7 spirit).
  **Recommended: accept.**
- **OQ-5 · R-MCP-4 heavy-command list is a constant.** When `command_run` is exposed, direct shell
  calls whose command starts with a listed prefix are refused (`reject_once`): `cargo build`,
  `cargo test`, `cargo nextest`, `cargo clippy`, `cmake --build`, `ctest`, `make`, `ninja`,
  `msbuild`, `dotnet build`, `dotnet test`, `npm test`, `pnpm test`, `go build`, `go test`.
  Constant in `htui-core`, not a setting (a Settings entry is a follow-up). Prefix match only —
  `cd x && cargo build` passes (R-6). **Recommended: accept.**
- **OQ-6 · Fixing `fan_out_only` changes prompt digests.** Today the engine renders the
  command-queue section whenever `command_queue != off` (`htui-orch/src/engine.rs:5593-5595`);
  ANA-5 `:337` says `fan_out_only` renders it only with fan-out > 1 or `heavy_build`. The fix
  changes the prompt (and digest) of every single-fan-out `fan_out_only` phase; golden snapshots
  move. **Recommended: accept the churn.**
- **OQ-7 · `command_run` executes on the hosting process's box, through the platform shell**
  (`sh -c` / `cmd /C`), cwd = the session cwd or a relative subdirectory of it (no `..`, no
  absolute paths), timeout 30 min default (argument may lower it), combined stdout+stderr, the
  last 64 KiB kept and returned, scrubbed fail-closed before store and return.
  **Recommended: accept.**
- **OQ-8 · Chat sessions get the server too** (R-MCP-1 "every session"). A fresh chat
  (`item_id NULL`) sees `box_profile` and `search_concepts` only; a promoted chat sees the item
  tools too, with `document_write` advertised only when the phase's `output_kind` resolves from the
  run snapshot. **Recommended: accept.**
- **OQ-9 · The prompt names `document_write`; no fallback document.** *(Raised by the fact-check:
  no prompt today tells an agent how to write its document — bodies only say "Write a `<kind>`
  document", `htui-core/src/prompt/defaults.rs:40-193`.)* When a session's scope advertises
  `document_write`, a protected `output` section renders one fixed sentence (D19). The alternative —
  storing the session's final message as the document when none was written — is **not** built:
  it would hide a missing document behind reply text and contradict ANA-2 risk 4 (`missing_output`
  fails). **Recommended: instruction only, no fallback.**
- **OQ-10 · A persona can still hide htui's tools.** `--tools` never filters MCP tools (docs +
  probe on claude 2.1.287; `htui-core/src/model/persona.rs:864-865`), but a persona `deny` entry
  such as `mcp__*` reaches `--disallowedTools` (`htui-agent/src/cli/mod.rs:185-198`) and would hide
  `document_write`. **Recommended: honour it as a deliberate persona choice, documented in
  `docs/htui-mcp.md` and `docs/personas.md`; no validation change.**

---

## Summary

Every session htui launches gets one MCP server, `htui`, declared as a stdio command
(`<current_exe> mcp`) with two env vars: the host's socket address and a per-session token. The
`htui mcp` child is a byte relay — it connects to the socket, presents the token, then splices
stdio to the socket. The **hosting** process (TUI engine, `htui worker`, or the TUI's chat runtime)
runs the MCP protocol and the tools, with the store, lease fence and scrubber it already holds; the
token resolves to a `ToolScope` (run, step, item, project, box, fence, phase, exposed tools)
registered for exactly the session's lifetime. Tools: `box_profile`, `search_concepts`,
`document_write`, `note_add`, `item_status`, `item_link`, `command_run`, and — for claude-CLI
sessions only — `permission_prompt`, which turns the CLI's `--permission-prompt-tool` call into an
ordinary `DriverEvent::PermissionRequest`, so the MOD-42 relay and the chat loop answer it
unchanged.

## Invariants (every task keeps these)

- **I-1 · Scope comes from the token, never from arguments.** No tool accepts a run, step, project
  or box id. An item reference in `item_link.to` is resolved inside the scope's project only.
- **I-2 · Agents never write Postgres; the host does** (ANA-16 §6.1, ANA-9 inv. 5). The `htui mcp`
  child holds no DSN and parses no MCP.
- **I-3 · Every agent write is fenced** (OQ-4) with the session's `StepFence`; a fenced miss is
  reported to the agent as a tool error and writes nothing.
- **I-4 · No agent moves a status** (R-ENT-8, ANA-2 risk 11): `item_status` writes an `item_note`
  with `via_step_id`; `item.status` is untouched.
- **I-5 · Everything persisted or returned from an agent or a command is scrubbed first, fail
  closed** (R-ID-7, R-SEC-3) — note bodies, document bodies, command output.
- **I-6 · A token outlives nothing.** Unregistered on session end (drop guard); later calls get
  `session ended`. htui never logs a token (it lives only in `McpServerSpec.env`, whose `Debug`
  redacts env, `htui-agent/src/driver.rs:236-245`). A token echoed by the agent is recorded like
  any other agent text: it grants nothing beyond the session's own scope, and nothing at all once
  the session ends. *(Fact-check: the scrubber is an immutable `&dyn Scrubber` built before
  `drive_once` and shared by both judge calls, so per-session secret injection is dropped.)*
- **I-7 · No new tool advertised = no new behavior.** A tool not in the scope's tool set is absent
  from `tools/list` and refused by `tools/call` (R-MCP-3 "not advertised").
- **I-8 · Stack headroom:** new engine futures are boxed (memory: htui-orch test stack headroom).

## Design decisions

### Crate and protocol (M1)

- **D1 · New crate `crates/htui-mcp`** (lib). Depends on `htui-core` (store traits, model, scrub,
  prompt render), `htui-agent` (`McpServerSpec`, prompt bridge types, M5), `htui-orch` (the
  `ToolHost` seam, D4), tokio (`net`, `io-util`, `sync`, `time`, `process`), `serde`,
  `serde_json`, `schemars` (promoted from the lock), `uuid`. Workspace `Cargo.toml` comments name
  "MOD-11 D1". Uses `contained::spawn*` where `htui-agent`'s clippy rules apply.
- **D2 · Protocol module `htui-mcp/src/protocol.rs`**: JSON-RPC 2.0 over NDJSON on any
  `AsyncRead + AsyncWrite`. Methods: `initialize` (answers the client's `protocolVersion` when it
  is one of `2025-06-18`, `2025-03-26`, `2024-11-05`, else the newest; `capabilities.tools = {}`;
  `serverInfo {name:"htui", version}`), `notifications/initialized` (ignored), `ping`,
  `tools/list`, `tools/call` (result `{content:[{type:"text",text}], isError}`); unknown method →
  `-32601`; malformed → `-32700`/`-32600`. Line cap 1 MiB (mirrors `cli/mod.rs:76`). Tool input
  schemas from `schemars` derives on argument structs; argument decode failure → `isError: true`
  with the serde message (not a protocol error), per MCP convention.
- **D3 · Channel `htui-mcp/src/channel.rs`**: `Listener` (Unix socket / Windows named pipe, OQ-1),
  `Address` (path or pipe name, rendered into env `HTUI_MCP_ADDR`). Handshake: the first line the
  child sends is `{"token":"<hex>","version":"<htui build version>"}`; the host answers
  `{"ok":true}`, or `{"ok":false,"reason":…}` and closes — a version mismatch (child from a rebuilt
  binary) is refused with a clear sentence on the child's stderr. The handshake completes well
  under the claude CLI's 30 s `MCP_TIMEOUT` wait. Token = 32 bytes
  from two `Uuid::new_v4()` (getrandom-backed), hex. The child then splices stdin→socket and
  socket→stdout until either side closes. One listener per process, created lazily on first
  registration, removed (socket file and directory) on shutdown/drop.
- **D4 · Seam `htui-orch::tools::ToolHost`** (object-safe, `Send + Sync`):
  `fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError>`. `ToolScope` (in
  `htui-orch`, plain data): `run_id`, `step_id`, `project_id`, `item_id: Option`, `box_id`,
  `user`, `fence: StepFence`, `output_kind: Option<String>`, `hostname: HostnameLine`,
  `command_queue: bool`, `cwd: PathBuf`, `transport: TransportKind {Acp, Cli}`.
  `ToolLease { spec: McpServerSpec, prompt: Option<PromptPort> }`, dropping it unregisters.
  `EngineParts` gains `tools: Option<Arc<dyn ToolHost>>`; `None` keeps today's `mcp: Vec::new()`
  exactly. There are 10 `EngineParts` literals, none with `..` (fact-check): production
  `htui-worker/src/runtime.rs:996`; `htui-orch/src/engine.rs:6731` (`fake_parts`), `:7073`,
  `:7163`, `:9864`, `:9965`, `:12861`, `:13978`; `htui-orch/src/conformance.rs:7942`;
  `htui-orch/tests/gix_isolator.rs:92` — each gains `tools: None` (T6).
- **D5 · `McpHost<H: WorkerHost>` implements `ToolHost`**: a registry `HashMap<Token, Arc<Session>>`
  behind a mutex, the lazy `Listener`, a `ConceptSearch` handle (D12) and the binary path. It takes
  `host.writer()` **per call** (the TUI's backend swaps, `store_worker.rs:2315`). Spec:
  `McpServerSpec { name: "htui", command: <binary path>, args: ["mcp"], env: {HTUI_MCP_ADDR,
  HTUI_MCP_TOKEN} }`. Binary path (must be absolute — ACP schema): on Linux `/proc/<host pid>/exe`,
  which stays valid for the host's lifetime even after the file is replaced (the
  `provision/mod.rs:104-110` precedent); elsewhere `std::env::current_exe()` resolved once at
  construction. The D3 version check catches a mismatched child (R-5). An in-process test API
  `McpHost::client(token)` speaks the same protocol over `tokio::io::duplex` (no socket), used by
  engine tests and the fake driver.
- **D6 · `htui mcp` subcommand** (`htui/src/cli.rs` `Command::Mcp`, dispatched in `lib.rs` beside
  `Worker`, before `init_tracing`, stdout untouched, logs to stderr only). Reads `HTUI_MCP_ADDR` /
  `HTUI_MCP_TOKEN`, connects, handshakes, splices. Exit 0 on clean EOF, 2 on missing env, 3 on
  refused token. No Sentry DSN interaction beyond what `main.rs` already does.

### Transports (M1)

- **D7 · ACP**: `NewSessionRequest::new(cwd).additional_directories(..).mcp_servers(map(spec.mcp))`
  (`htui-agent/src/acp/mod.rs:1108-1109`), mapping to `McpServer::Stdio(McpServerStdio::new(name,
  PathBuf::from(command)).args(..).env(Vec<EnvVariable>))`. Stdio is mandatory for every ACP agent
  (schema `agent.rs:2634`), so no capability check. Stdio is `#[serde(untagged)]`: the wire entry
  has no `type` key (the fake-agent test asserts that).
- **D8 · claude CLI**: when `spec.mcp` is non-empty, `argv` emits the single argument
  `--mcp-config=<json>` (`=`-joined, so the CLI's variadic parse cannot swallow a following token;
  `{"mcpServers":{"htui":{"type":"stdio","command",…,"args",…,"env",…}}}`) after `--add-dir`/budget
  and before `--tools`/`--disallowedTools`, so `extra_args` still wins. No `--strict-mcp-config`
  (the operator's own servers stay). `--tools` is left unchanged — it never filters MCP tools
  (verified, claude 2.1.287); the deny path is OQ-10.
- **D9 · `FakeDriver` records the `SessionSpec`** in a shared slot
  (`Arc<Mutex<Option<SessionSpec>>>`) with a `spec_handle()` accessor taken **before** the driver is
  boxed (`htui-orch/src/fake.rs:1733` `driver_for_key`, `htui-agent/src/fake.rs:664-688`
  `FakeAdapter::build`); the orch fake keeps a per-`SessionKey` map of handles (T6). Scripted
  sessions can call the htui tools through `McpHost::client` (D5).

### Engine and hosts (M1–M2)

- **D10 · Registration in `drive_once`** (`htui-orch/src/engine.rs:5875-5945`): when
  `self.parts.tools` is `Some`, build the `ToolScope` from what `drive_once` holds (run, step,
  `phase.output_kind`, `self.parts.{owner, box_id, user}`, project switch via
  `settings::resolve_box_hostname` → `Shown`/`Omitted`, the resolved command-queue flag D16 —
  `drive_once` reads the item, `self.item(Self::item_of(run)?)`, for `required_tags`), open
  the lease, push `lease.spec` into `SessionSpec.mcp`, keep the lease alive until the session (and its `after_done`) completes. A
  `ToolHostError` fails the step as a driver error (never silently drops the server). The judge's
  two calls each open their own lease (distinct tokens, same step).
- **D11 · Hosts**: `htui-worker` `Shared` gains `tools: Option<Arc<dyn ToolHost>>` with
  `RunRuntime::with_tool_host` (mirrors `with_author`, `runtime.rs:1316`), handed to
  `EngineParts` in `Kit::engine`; the listener is closed in `RunRuntime::shutdown`. Production
  wiring: `worker_cmd.rs` (`McpHost::<PgStore>`) and `store_worker::spawn_with*`
  (`McpHost::<Backend>`). Chat (`agent_worker.rs` `start` :2069, `bind_promoted` :1037) opens a
  lease from the same `McpHost<Backend>` with `StepFence::Unleased`; output kind per OQ-8:
  `bind_promoted` stops discarding `promoted.run` (today `..` at ~:986-991) and resolves
  `writer.run(run)` → `htui_orch::snapshot_of` → `htui_orch::phase_at(run.id, &snapshot,
  step.position)` (step from `writer.run_steps`), as `Engine::promote` does
  (`engine.rs:1267-1271`); `item_id = run.item_id`. A snapshot decode failure withholds
  `document_write`, never fails the chat.
- **D19 · Output instruction (OQ-9)**: `PromptSpec` gains `document_tool: bool`; when true a
  protected `<section name="output">` renders the fixed sentence "Write your `<output_kind>`
  document by calling the `document_write` tool of the `htui` MCP server; text left only in your
  reply is not recorded." The engine sets it from the scope (tool host present, item present,
  output kind non-empty) for phase and judge prompts; fixtures and the preview leave it `false`, so
  existing goldens do not move — new goldens pin the section. A test pins that a scripted session
  writing no document still ends in `missing_output` / `MissingDocument` (no fallback).
- **D12 · `search_concepts` seam**: `htui-mcp::ConceptSearch` (object-safe, boxed future) taking
  `SearchQuery` and returning `Vec<Hit>`; the `htui` binary adapts its `ConceptIndex`/`QdrantIndex`
  (`htui/src/concepts_worker.rs:52`, `:79`); `worker_cmd.rs` builds one for the worker. `None` →
  tool not advertised.

### Tools (M1–M4)

Every tool: scope from token (I-1); writes fenced (I-3); text scrubbed (I-5); errors are
`isError: true` with a one-line reason (`not found`, `fenced: lease lost`, `out of scope`,
`session ended`, `search unavailable: …`).

- **`box_profile` (M1)** — no args. Returns the `BoxProfile` of the scope's box as JSON plus the
  `render::box_profile(profile, scope.hostname)` text, so tool and prompt agree (PRD OQ-3):
  `hostname` dropped when the project switch is off.
- **`document_write` (M2)** — `{title?, body}`; kind fixed to `scope.output_kind`; advertised only
  when the scope has an item and an output kind (PRD OQ-5). Calls the new fenced
  `write_step_document(fence, NewDocument{produced_by_step_id: Some(step), created_by: user})`;
  returns `{document_id, version}`. Several calls write several versions (the judge needs one per
  call; ANA-2 `:424-431` "exactly one per step" is relaxed to "newest wins", as MOD-4's judge
  already assumes — recorded deviation).
- **`note_add` (M2)** — `{body}` (≤ 16 KiB after scrub) → `add_step_note(fence, NewNote{via_step_id:
  Some(step), created_by: user, box_id: Some(box)})`.
- **`item_status` (M2)** — `{status, reason, resolution?}`; `status` parses as
  `htui_core::model::Status` (`model/item.rs:10`, `str_enum!` → `FromStr`, `Status::ALL` feeds the
  schema enum); `resolution` (optional, `Resolution`) is accepted for a `closed` request; writes a
  note `status request: <status>[ (<resolution>)] — <reason>` through `add_step_note`. Never a
  transition (I-4).
- **`item_link` (M2)** — `{op: "add"|"remove", to: <item key>, kind}` (all four kinds, PRD OQ-4);
  `from` is always the scope's item; `to` resolved by key **within the scope's project**; `add` →
  `propose_link(fence, from, to, kind, step)` (upsert, un-tombstones, `proposed_by_step_id = step`;
  `from = to` refused); `remove` → `withdraw_link(fence, from, to, kind, run)` tombstones only a
  link whose `proposed_by_step_id` belongs to the scope's run, else `not yours`.
- **`search_concepts` (M3)** — `{query, types?, statuses?, limit? ≤ 20}`; `projects = [scope.project]`
  always; returns what `Hit` carries (`htui-store/src/vector.rs:330-348`, no `Serialize`, mapped by
  hand): `{point_type, owner_kind, key, document_kind?, resolution?, state?, score, snippet}` — no
  title/status lookups. `types` strings are parsed in `htui-mcp` (`PointType::parse` is private;
  making it `pub` is the alternative). Qdrant/model unavailable → `isError` with the cause
  (R-STO-8).
- **`command_run` (M4)** — `{class: "build"|"test"|"run", command, cwd?, timeout_secs?}`. Advertised
  per D16. Enqueue → wait for admission → execute (OQ-7) → finish → return
  `{exit_code, status, output, truncated}`. Classes beyond the three are refused; `verify` is
  reserved to the orchestrator (PRD OQ-6).
- **`permission_prompt` (M5)** — advertised only for `TransportKind::Cli`. Input per the claude
  CLI contract `{tool_name, input, tool_use_id?}`; output text is the JSON
  `{"behavior":"allow","updatedInput":<input>}` or `{"behavior":"deny","message":…}` (contract to
  pin with a recorded transcript; ANA-4 `:1385-1386` closed here).

### Store (M2, M4)

- **D13 · New fenced `WriteStore` methods** (fence first after `&self`, like `pass_step`,
  `traits.rs:1494`; listed in the fenced-methods doc at `:2481-2491`):
  - **Lock order is run → item in every new method** (fact-check: `park_step`, `write.rs:5473-5535`,
    takes `run` then `item`; the reverse order deadlocks, 40P01): `step_fence(&mut tx, step, fence)`
    (`pg/write.rs:197`, run `FOR SHARE`) first, then any item lock. Every method also checks that
    the step's `run.item_id` equals the target item (a step writes only on its own item).
  - `write_step_document(fence, NewDocument) -> Document` — requires `produced_by_step_id`; Pg:
    `step_fence`, item `FOR UPDATE`, `insert_document` (`write.rs:265`, takes `&mut PgConnection`).
  - `add_step_note(fence, NewNote) -> Note` — requires `via_step_id`; same checks (closes the
    "any step id" gap of `add_note`).
  - `propose_link(fence, ProposeLink{from, to, kind, step}) -> ItemLink` — `cite`-style upsert
    (`write.rs:6186-6260`), `deleted_at = NULL`, `updated_at` left to the trigger (mirror picks it
    up, `cache/refresh.rs:931-975`).
  - `withdraw_link(fence, from, to, kind, run) -> ItemLink` — tombstone where the proposer step's
    `run_id = run`; zero rows → `NotFound`/`Constraint("not proposed by this run")`.
  - `write_document` stays unfenced for `close_out` and the test author; `ProgressSink` switches
    to `write_step_document(StepFence::Lease(owner))` (closes MOD-41 D5).
- **D14 · Command queue methods + migration `0014_command_queue.sql`** (OQ-3):
  `enqueue_command(NewCommandRun) -> CommandRun` (status `queued`, step must exist),
  `claim_command(id, claimant, limit) -> Option<CommandRun>` (Pg: `pg_advisory_xact_lock` on
  `hashtextextended(box_id||class)`, reap stale `running` rows of that `(box, class)`, count
  `running` < limit and the row is the oldest `queued` of its `(box, class)` → `running`,
  `claimed_by`, `started_at`, `heartbeat_at = clock_timestamp()`), `beat_command(id, claimant)`,
  `finish_command(id, claimant, status, exit_code, output)`, `cancel_command(id)`. All on
  `WriteStore` (5 implementors: `MemStore`, `PgStore`, `Writer`, `UsageSpy`, `SpyStore`) +
  `WorkerStore` (trait in `htui-core/src/store/worker.rs` + MemStore impl there + `PgStore`/`Writer`
  forwarders in `htui-store/src/worker.rs`); store suite is `htui-core/src/store/conformance.rs`
  (`CASES` :53, `run_case` :199); counts at `htui-core/tests/mem_store.rs:36` and
  `htui-store/tests/pg_conformance.rs:27` (both 134 today). MemStore reference with its clock. Not
  mirrored.
- **D15 · Limits**: `box.settings.command_limits` over the `app_setting` default
  (`0003_orchestration.sql:129`); the worker's `command_limits()` fallback reads
  `WorkerHost::app_settings()` instead of `{"verify":1}` (`htui-worker/src/runtime.rs:807-823`).
  A missing class limit = 1.

### Exposure (M4)

- **D16 · One resolver** `htui_core::…::command_queue_exposed(mode, fan_out, item_tags) -> bool`:
  `off` → false; `always` → true; `fan_out_only` → `fan_out > 1 || tags ∋ "heavy_build"`. Used for
  the prompt flag in `phase_spec` (today `phase.command_queue != Off && persona.is_none_or(|p|
  p.tools.command_run)`, `engine.rs:5593-5595`; the first operand becomes the resolver, the persona
  term stays; `phase_spec` already reads the item row at `:5488`), and for the base exposure in
  **both** `drive_once` branches: `ToolExposure { command_run: command_queue_exposed(..),
  ..Default::default() }`, passed to `narrow` (persona present; `base && persona`, personas default
  `command_run: true`, `htui-core/src/model/persona.rs:125-134`) or used as is (no persona). Same
  value feeds `ToolScope.command_queue`. Judges stay off. Every seeded phase is `fan_out_only`
  (`seed.rs:232`), so every single-fan-out step without `heavy_build` loses the section (OQ-6).
- **D17 · R-MCP-4 denial**: after the `match persona` (`engine.rs:5900-5905`), when
  `tools.command_run` is true, splice at index 0 of `policy.rules` one rule per OQ-5 prefix —
  `PermissionRule { matcher: PermissionMatch { tool_kind: Some("execute".into()),
  command_prefix: Some(p), ..Default::default() }, answer: PermissionOptionKind::RejectOnce, reason }`
  — for **both** branches (not inside `narrow`, which only runs with a persona). CLI sessions get
  `Bash(<p>:*)` entries in `--disallowedTools`. `COMMAND_QUEUE_TEXT`
  (`htui-core/src/prompt/defaults.rs:26`) already names `command_run` — **left unchanged** (its
  bytes are pinned by `defaults.rs:428-447`). Tests: persona-less and persona steps; a
  `fan_out_only` fan-out-1 step has no section, fan-out-2 or `heavy_build` has it.

### CLI permission prompt (M5)

- **D18 · `PromptBridge` in `htui-agent`**: a pair `(PromptPort, PromptAsk)`; `ToolLease.prompt`
  carries the port into `SessionSpec` (new field `prompt: Option<PromptPort>`, added in **T0**
  because all 10 exhaustive `SessionSpec` literals break — `htui-agent/src/conformance.rs:305`,
  `tests/{relay.rs:196, cli_driver.rs:52, acp_driver.rs:98, driver_contract.rs:145,
  extensibility.rs:537, agy_live.rs:799}`, `htui-orch/src/engine.rs:5908`,
  `htui/src/agent_worker.rs:1037, :2069`). `SessionSpec` derives `PartialEq, Eq`; tokio's `Sender`
  does not, so `PromptPort` carries a minted id and implements `PartialEq`/`Eq` by id, and the
  hand-written `Debug` (`driver.rs:285-301`) prints `prompt: <port id>|None`. The prompt tool is
  hidden from the model by the CLI (probe), so it needs no model-facing description;
  `tool_use_id` stays `Option`.
  The CLI session, when it has a port, merges `PromptPort` requests into its event stream as
  `DriverEvent::PermissionRequest { request_id: <tool_use_id or minted>, tool_call_id, options:
  [allow_once, reject_once] }` and implements `answer_permission` by completing the request's
  oneshot. `argv` adds `--permission-prompt-tool mcp__htui__permission_prompt`. **Caps stay per-row
  false** (fact-check: the interlock is `htui-orch/src/select.rs:166-171`, reading
  `registry::caps_for(agent)` from the agent row before any session exists; CLI false is pinned in
  seven tests). Instead `SelectInput` gains `inline_prompt: bool`, set by the engine when it has a
  tool host; rule 3 admits a `Transport::Cli` agent to a gated phase when it is set (R-8). A
  start-time guard refuses a gated CLI step whose lease came back without a prompt port
  (`missing_capability: inline_approval`), so the interlock cannot be bypassed. The engine relay (MOD-42) and chat
  `run_turn` answer it unchanged; the denial synthesis from `permission_denials[]` stays for the
  no-port case and is deduplicated against prompt-answered ids.

## Tasks

TDD per repo convention: each task writes its failing tests first. **File sets are the
independence contract** (handoff-run step 3.5); a task may not touch a file outside its set
without the orchestrator re-checking intersections. Intersections were computed over the file
sets below after the fact-check amendments (see "Independence check").

### Wave 0 (serial)

#### T0 · `SessionSpec.prompt` field and bridge types (M5 groundwork)
- **Action**: D18's type half only — `htui-agent/src/prompt_bridge.rs` (`PromptPort`, `PromptAsk`,
  `PromptRequest`, id-based `PartialEq`/`Eq`, `Debug`), `SessionSpec.prompt: Option<PromptPort>`,
  the hand-written `Debug` entry, `prompt: None` at all 10 literals. No behavior.
- **Files**: `htui-agent/src/prompt_bridge.rs` (new), `htui-agent/src/lib.rs`,
  `htui-agent/src/driver.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/relay.rs`,
  `htui-agent/tests/cli_driver.rs`, `htui-agent/tests/acp_driver.rs`,
  `htui-agent/tests/driver_contract.rs`, `htui-agent/tests/extensibility.rs`,
  `htui-agent/tests/agy_live.rs`, `htui-orch/src/engine.rs`, `htui/src/agent_worker.rs`.
- **Validate**: `cargo check --workspace --all-targets --all-features`;
  `cargo test -p htui-agent --all-features --test driver_contract`.

### Wave 1 (parallel: T1 ∥ T2 ∥ T3)

#### T1 · Fenced agent writes in the store (M2 store half)
- **Action**: D13 four methods, run → item lock order, own-item check; conformance cases (document
  fence hit/miss, two versions from one step, document/note on a foreign item refused, link
  upsert/un-tombstone/self-link refusal, withdraw own/foreign); a Pg-only case racing
  `write_step_document` against `park_step` (no 40P01); bump both case counts; `.sqlx` prepare.
- **Files**: `htui-core/src/store/traits.rs`, `htui-core/src/store/mem.rs`,
  `htui-core/src/store/worker.rs`, `htui-core/src/store/conformance.rs`,
  `htui-core/tests/mem_store.rs`, `htui-core/src/model/link.rs`, `htui-store/src/pg/write.rs`,
  `htui-store/src/writer.rs`, `htui-store/src/worker.rs`, `htui-store/tests/pg_conformance.rs`,
  `htui-store/tests/pg_criteria.rs`, `htui-store/.sqlx/*` (new files),
  `htui-agent/src/conformance.rs` (`UsageSpy`), `htui-agent/tests/recorder.rs` (`SpyStore`).
- **Mirror**: `resolve_command` end to end (MOD-42); `cite`/`uncite`; `pass_step` fence.
- **Validate**: `cargo test -p htui-core --all-features`; Pg conformance with
  `HTUI_TEST_DATABASE_URL`; `cargo sqlx prepare --check` (memory: needs a migrated scratch DB).

#### T2 · `htui-mcp` crate: protocol, channel, host, `box_profile` (M1)
- **Action**: D1, D2, D3 (token + version handshake), D5 (registry, lease drop, `client(token)`,
  binary path), D4's `ToolHost`/`ToolScope`/`ToolLease` in `htui-orch/src/tools.rs` (new) + one
  `pub mod tools;` line in `htui-orch/src/lib.rs`, D12's `ConceptSearch` trait in
  `htui-mcp/src/search.rs`. **Tool registry layout fixed here**: `tools/mod.rs` lists all eight
  tools, one file each; every tool file except `box_profile.rs` is a stub never advertised
  (`advertised(scope) = false`), so T4/T7/T8/T9 each own exactly their own file and never touch
  `tools/mod.rs`. `box_profile` implemented (hostname per scope). Tests: protocol transcripts
  (initialize/list/call/errors/version negotiation), handshake refusal (bad token, bad version),
  unregistered token, Unix-socket round trip; `cfg(windows)` named-pipe code compiled with
  `cargo check --target x86_64-pc-windows-gnu` (the probe target is installed).
- **Files**: `Cargo.toml` (workspace members + deps), `Cargo.lock`, `crates/htui-mcp/**` (new,
  including the stub tool files), `htui-orch/src/tools.rs` (new), `htui-orch/src/lib.rs`.
- **Validate**: `cargo test -p htui-mcp`; `cargo clippy -p htui-mcp --all-targets -- -D warnings`.

#### T3 · Transports carry `spec.mcp` (M1)
- **Action**: D7, D8, D9. Tests: ACP fake agent captures `params.mcpServers` on `session/new`
  (clone of `acp_driver.rs` `fs_agent`; stdio entry has no `type`); `cli_driver.rs` argv cases (no
  mcp → argv unchanged; mcp → one `--mcp-config=<json>` argument before `--tools`, `--tools`
  unchanged, `extra_args` last); `FakeDriver::spec_handle()`.
- **Files**: `htui-agent/src/acp/mod.rs`, `htui-agent/src/cli/mod.rs`, `htui-agent/src/fake.rs`,
  `htui-agent/tests/acp_driver.rs`, `htui-agent/tests/cli_driver.rs`.
- **Validate**: `cargo test -p htui-agent --all-features`.

### Wave 2 (parallel: T4 ∥ T5)

#### T4 · Backlog write tools (M2 tool half) — after T1, T2
- **Action**: `document_write`, `note_add`, `item_status`, `item_link`; scoping and refusal tests
  per tool against `MemStore` through `McpHost::client` (foreign item key, other project, lost
  lease → `fenced`, ended session, withdraw of a foreign link).
- **Files**: `crates/htui-mcp/src/tools/{document,note,status,link}.rs` (stubs from T2),
  `crates/htui-mcp/tests/tools_backlog.rs` (new).
- **Validate**: `cargo test -p htui-mcp`.

#### T5 · `htui mcp` stdio relay (M1) — after T2
- **Action**: D6; integration test spawning `CARGO_BIN_EXE_htui mcp` against an `McpHost<MemStore>`
  listener: handshake, `initialize`, `tools/list`, `tools/call box_profile`, bad token exit 3,
  missing env exit 2, nothing but protocol bytes on stdout.
- **Files**: `htui/src/cli.rs`, `htui/src/lib.rs`, `htui/src/mcp_cmd.rs` (new),
  `htui/tests/mcp_stdio.rs` (new), `htui/Cargo.toml`.
- **Validate**: `cargo test -p htui --all-features --test mcp_stdio`.

### Wave 3 (serial)

#### T6 · Engine registration, hosts, output instruction (M1–M2 wiring) — after T3, T4, T5
- **Action**: D10, D11, D19; `EngineParts.tools` at all 10 literals; orch fake keeps per-key spec
  handles (D9); `ProgressSink` → `write_step_document(StepFence::Lease(owner))` (closes MOD-41 D5);
  `htui/src/mcp_search.rs` stub returning `None`. Engine tests: a fake session that calls
  `document_write` via its token → candidate output found; **judge resolves both calls with the
  production sink (`author: None`)**; a lost lease → `fenced`; no document → still
  `MissingDocument` (OQ-9); `tools: None` keeps specs and prompts byte-identical; prompt has the
  `output` section only when the tool is advertised. Runs pane `approve`/`accept` enabled for a
  step whose document came through the tool (no test `StepAuthor`). Chat: fresh → two tools;
  promoted → item tools with the snapshot-resolved output kind.
- **Files**: `htui-orch/src/engine.rs`, `htui-orch/src/fake.rs`, `htui-orch/src/conformance.rs`,
  `htui-orch/tests/gix_isolator.rs`, `htui-core/src/prompt/mod.rs`, `htui-core/src/prompt/render.rs`,
  `htui-core/tests/snapshots/*output*` (new), `htui-worker/src/runtime.rs`,
  `htui-worker/src/views.rs`, `htui-worker/Cargo.toml`, `htui/src/worker_cmd.rs`,
  `htui/src/store_worker.rs`, `htui/src/agent_worker.rs`, `htui/src/run_worker.rs`,
  `htui/src/lib.rs`, `htui/src/mcp_search.rs` (new, stub), `htui/Cargo.toml`,
  `htui/tests/runs_pg.rs`, `htui/tests/backlog.rs`.
- **Validate**: `cargo test -p htui-orch -p htui-worker -p htui-core --all-features`;
  `cargo test -p htui --all-features -- --test-threads=1`.

### Wave 4 (T7 ∥ T8, then T9)

#### T7 · `search_concepts` (M3) — after T6
- **Action**: D12 tool (hand-mapped `Hit`, parsed `types`), binary adapter over `ConceptIndex`,
  `htui worker` builds one. Tests: project scoping (a hit in another project never returned),
  requirement hits carry `owner_kind = requirement`, unavailable index → `isError`.
- **Files**: `crates/htui-mcp/src/tools/search.rs` (stub from T2), `htui/src/concepts_worker.rs`,
  `htui/src/mcp_search.rs` (stub from T6).

#### T8 · `command_run` queue and exposure (M4) — after T6
- **Action**: migration `0014` (OQ-3), D14 store methods + conformance + Pg contention test
  (limit never exceeded; stale heartbeat reaped), D15 limits fallback, D16 resolver in `phase_spec`
  and both `drive_once` branches, D17 rules at the call site + CLI `--disallowedTools` entries,
  executor (OQ-7) and tool in `htui-mcp/src/tools/command.rs`.
- **Files**: `htui-store/migrations/0014_command_queue.sql` (new), `htui-store/tests/migrations.rs`,
  `htui-core/src/store/{traits,mem,worker,conformance}.rs`, `htui-core/tests/mem_store.rs`,
  `htui-core/src/model/run.rs`, `htui-core/src/model/phase.rs` or wherever the resolver lands
  (one new fn), `htui-store/src/pg/write.rs`, `htui-store/src/{writer,worker}.rs`,
  `htui-store/tests/{pg_conformance,pg_criteria}.rs`, `htui-store/.sqlx/*`,
  `htui-agent/src/cli/mod.rs`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`,
  `htui-agent/tests/cli_driver.rs`, `htui-orch/src/engine.rs`, `htui-orch/src/conformance.rs`,
  `htui-worker/src/runtime.rs`, `crates/htui-mcp/src/tools/command.rs` (stub from T2),
  `crates/htui-mcp/tests/tools_command.rs` (new).

#### T9 · CLI `permission_prompt` (M5) — after T8
- **Action**: D18 behavior: CLI session merges `PromptPort` requests into its event stream and
  answers them; `--permission-prompt-tool` in argv; `SelectInput.inline_prompt` + rule 3; start-time
  guard; the tool in `htui-mcp`. Tests: recorded claude transcript of the prompt call (shape from
  the 2.1.287 probe); merge/answer; relay test (engine drives a CLI fake that parks through the
  prompt tool, answered from the store); select rule both ways; guard; denial dedup.
- **Files**: `htui-agent/src/prompt_bridge.rs`, `htui-agent/src/cli/mod.rs`,
  `htui-agent/src/cli/claude.rs`, `htui-agent/tests/cli_driver.rs`, `htui-agent/tests/relay.rs`,
  `htui-orch/src/select.rs`, `htui-orch/src/engine.rs`, `crates/htui-mcp/src/tools/permission.rs`
  (stub from T2), `crates/htui-mcp/tests/tools_permission.rs` (new).

#### T10 · Docs (close-out input) — after T9
- **Action**: `docs/htui-mcp.md` (user doc: tools, scoping, socket location, troubleshooting,
  OQ-10 persona deny note); `docs/htui-worker.md` listener note; `docs/personas.md` deny note.
  Bookkeeping (HANDOFF, DECISIONS, write-up, cross-links, stale HANDOFF facts from the PRD
  evidence) happens at close-out.

### Independence check (file-set intersections)

| Pair | Shared files | Verdict |
|---|---|---|
| T1 ∩ T2 | none | parallel |
| T1 ∩ T3 | none (`htui-agent/src/conformance.rs` is T1's, not T3's) | parallel |
| T2 ∩ T3 | none | parallel |
| T4 ∩ T5 | none | parallel |
| T7 ∩ T8 | none | parallel |
| T8 ∩ T9 | `htui-agent/src/cli/mod.rs`, `htui-agent/tests/cli_driver.rs`, `htui-orch/src/engine.rs` | serial (T8 → T9) |
| T0 ∩ T1, T3, T6 | `htui-agent/src/conformance.rs`, `tests/{cli,acp}_driver.rs`, `engine.rs`, `agent_worker.rs` | T0 runs alone first |
| T1 ∩ T8 | store files | serial by wave |
| T5 ∩ T6 | `htui/src/lib.rs`, `htui/Cargo.toml` | serial by wave |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod11-test.log
grep -c SIGABRT /tmp/mod11-test.log   # must be 0 (stack headroom)
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
cargo check -p htui-mcp --target x86_64-pc-windows-gnu
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-1 Agent escapes its scope via arguments | Medium | I-1; per-tool refusal tests with foreign ids/keys |
| R-2 Token echoed into session events | Low | I-6: per-session, scope-limited, dead after the session; htui never logs it |
| R-3 Command queue starvation/deadlock across processes | Medium | Advisory lock per `(box, class)`; heartbeat reaping (OQ-3); Pg contention test |
| R-4 Prompt digest churn hides a real regression | Medium | Snapshot diffs reviewed per phase; only `command_queue` (OQ-6) and the new `output` section move |
| R-5 Child from a rebuilt binary speaks another protocol | Low | Linux `/proc/<pid>/exe`; version in the D3 handshake |
| R-6 Prefix denial bypassed (`cd x && cargo build`, env prefix) | High | Accepted (OQ-5); `COMMAND_QUEUE_TEXT` is the primary channel, denial a nudge |
| R-7 ~~`--tools` hides htui tools~~ | — | Closed by fact-check: `--tools` never filters MCP tools; deny path is OQ-10 |
| R-8 CLI agents admitted to gated phases | Medium | Only via `SelectInput.inline_prompt`; start-time guard; select tests both ways |
| R-9 htui-orch test stack overflow | Medium | I-8; gate greps SIGABRT |
| R-10 Windows named-pipe path not run in the sandbox | High | Compiled for `x86_64-pc-windows-gnu`; runtime check left to MOD-16 |
| R-11 Agents ignore the output instruction | Medium | D19 sentence names the tool; `MissingDocument` stays loud (OQ-9) |
| R-12 `write_step_document` deadlocks with `park_step` | Low | Run → item lock order (D13); Pg race test in T1 |

## Acceptance

- [ ] All tasks complete, each with its tests written first
- [ ] Production judge resolves on agent-written documents with `author: None`
- [ ] `approve`/`accept` live on a production step whose document came through `document_write`
- [ ] Every tool refuses out-of-scope references; lost lease writes nothing
- [ ] Validation passes (gate run serially, `--test-threads=1`)
- [ ] Patterns mirrored, not reinvented

## Verified claims

Fact-check 2026-10-03 (handoff-run step 3.5): six parallel verifiers, workflow `wf_33a62d14-658`,
39 claims — 22 TRUE, 16 PARTLY, 1 FALSE; every non-TRUE verdict amended in place (column 2 names
where). Probes: ACP `NewSessionRequest.mcp_servers` compile probe; claude CLI 2.1.287 live probe
(`--mcp-config` inline JSON, `--permission-prompt-tool` call/answer shape, `--tools` vs MCP);
tokio `net` UnixListener + named pipe (`x86_64-pc-windows-gnu` check); `current_exe` after
replacement; `SessionSpec` derive with a `Sender` field.

| Claim | Verdict | Evidence |
|---|---|---|
| transport-1 · agent-client-protocol 2.1.0 (schema 1.7.0): NewSessionRequest has a builder `.mcp_servers(Vec<McpServer>)`; McpServer::Stdio(McpServerStdio::new(name, … | TRUE | Cargo.lock:178-214 pins agent-client-protocol 2.1.0 and agent-client-protocol-schema 1.7.0. In schema-1.7.0/src/v1/agent.rs: NewSessionRequest::mcp_servers(Vec<McpServer>) at :845; enum McpServer at :2634 with `#[serde(untagged)] Stdio(McpServerStdio)` (doc: … |
| transport-2 · htui-agent/src/acp/mod.rs:1108-1109 is the only NewSessionRequest build site; no LoadSession is built. | TRUE | `grep NewSessionRequest\|LoadSession\|load_session\|resume_session` across crates/*/src and tests finds two hits. The production one is crates/htui-agent/src/acp/mod.rs:1108-1109 … |
| transport-3 · cli/mod.rs argv(): `--mcp-config <json>` can be inserted after --add-dir/--max-budget-usd and before --tools/--disallowedTools so extra_args stays last (D8). | TRUE | crates/htui-agent/src/cli/mod.rs:116-180. Fixed flags come first. A scoped `push` closure (:143-163) then emits --permission-mode, --resume\|--session-id, --model, --add-dir..., and --max-budget-usd. After the closure: `--tools=<allow>` at :169-171 and … |
| transport-4 · claude CLI accepts `--mcp-config` with an inline JSON string, a stdio entry {type:stdio,command,args,env}, and `--permission-prompt-tool mcp__<server>__<tool>` whose … | TRUE | Docs (code.claude.com/docs/en/cli-reference): '--mcp-config: Load MCP servers from JSON files or strings (space-separated)'. '--permission-prompt-tool: Specify an MCP tool to handle permission prompts in non-interactive mode', and it waits for that server up … |
| transport-5 · claude `--tools` allow-list restricts MCP tools too, so a persona allow-list would hide htui tools unless mcp__htui__* is appended; and `--tools` is the flag name the … | PARTLY → amended (D8, OQ-10, R-7) | The flag name is correct: cli/mod.rs:169-171 emits `--tools=<allow joined by ,>`, and `claude --help` (2.1.287) lists `--tools <tools...> Specify the list of available tools from the built-in set`. The premise is false. Docs (cli-reference, --tools row): 'The … |
| transport-6 · FakeDriver::start (htui-agent/src/fake.rs ~158-181) does not record the SessionSpec today; adding a recorded spec is local to fake.rs. | PARTLY → amended (D9, T3, T6) | Not recorded today: TRUE. fake.rs:158-181 `start(spec, prompt)` takes the script out of its slot and calls FakeSession::open(&name, caps, &spec, script) (:179). open (:232) reads only spec.step_id and spec.retain_raw. FakeDriver has private fields {name, … |
| transport-7 · htui-agent tests/acp_driver.rs has a fake agent (fs_agent ~490-541) matching session/new that can be cloned to capture params.mcpServers. | TRUE | crates/htui-agent/tests/acp_driver.rs:492-541 `async fn fs_agent(stream: DuplexStream, request: Value, answered: oneshot::Sender<Value>)` is a raw NDJSON JSON-RPC loop. It matches `Some("initialize")` (:516) and `Some("session/new")` (:519-521) and replies … |
| engine-1 · drive_once (~5875-5945) builds SessionSpec with mcp: Vec::new() and has run, step, phase (output_kind, command_queue), self.parts.owner/box_id/user in hand; EngineParts … | PARTLY → amended (D4, D10, I-6, T6) | The location and the values in hand are correct. crates/htui-orch/src/engine.rs:5875 has `async fn drive_once(&self, run: &Run, step: &RunStep, phase: &SnapshotPhase, persona: Option<&SnapshotPersona>, key, text, cwd, extra_dirs, recorder: &mut Recorder)`. It … |
| engine-2 · The judge (engine.rs ~5057-5094) calls drive_once twice on one step and requires a distinct new judge document per call via output_of (~6178-6197). | TRUE | engine.rs:5034-5094 `judge_calls` loops over `[(0, forward), (1, reversed)]`. Each pass calls `self.drive_once(run, judge, jp, None, &key{call}, ...)` (:5056-5068), which runs `driver.start` and so opens a fresh session. After `sink.after_done` (:5074-5077) … |
| engine-3 · Persona narrowing ANDs ToolExposure.command_run (persona.rs ~84-89): what is the persona-side default? If personas default to false, exposing command_run requires … | PARTLY → amended (D16) | The AND is real: crates/htui-agent/src/persona.rs:84-89 has `command_run: base.command_run && own.command_run`. Personas default to true, so no opt-in is needed. `PersonaTools::default()` sets `command_run: true` … |
| engine-4 · engine.rs:5593-5595 prompt flag is `phase.command_queue != Off && persona.tools.command_run`, and judges pass false (~4849). | PARTLY → amended (D16) | engine.rs:5593-5595 is inside `phase_spec`: `command_queue: phase.command_queue != htui_core::model::CommandQueue::Off && persona.is_none_or(\|persona\| persona.tools.command_run)`. The persona is an `Option`; with no persona the flag is just `command_queue … |
| engine-5 · The default prompt bodies (htui-core/src/prompt/defaults.rs) or any prompt section tell the agent to write its output document via an MCP tool (document_write) — or … | FALSE → amended (OQ-9, D19, T6) | Nothing tells the agent to use `document_write`. `document_write` and `mcp__htui` appear in source only in doc comments (htui-orch fake.rs:1067/1737/1777, engine.rs:14/187, htui-worker views.rs:509). None of the prompt code under crates/htui-core/src/prompt/ … |
| engine-6 · SnapshotPhase has a fan_out field usable for the fan_out_only resolver, and the item required_tags is reachable in drive_once (run.item_id -> item). | TRUE | `SnapshotPhase.fan_out: i32` (crates/htui-core/src/model/run.rs:508-509) is already used for `phase.fan_out > 1` in views.rs:387 and command.rs:1107. `Run.item_id: Option<ItemId>` (run.rs:173-174) and `Item.required_tags: Vec<String>` … |
| engine-7 · Chat bind_promoted (htui/src/agent_worker.rs ~949-1078): can the phase output_kind be resolved (Opening.phase + run snapshot)? Name the store call/field. | PARTLY → amended (D11) | It can be resolved, but not from what bind_promoted keeps today. `Promoted { run, step, project, opening }` (crates/htui-worker/src/address.rs:153-162) carries `run: RunId`, but bind_promoted drops it with `..` in its destructuring (agent_worker.rs:~986-991). … |
| hosts-1 · htui-worker runtime.rs: Shared.author Option<Arc<dyn StepAuthor>> (~187), with_author (~1316); a tools field + with_tool_host can mirror it; RunRuntime::shutdown … | TRUE | crates/htui-worker/src/runtime.rs:187 `author: Option<Arc<dyn StepAuthor>>`, :188 `owner: Uuid`; :1316 `pub fn with_author(mut self, author) -> Self { self.configure().author = Some(author); self }` (configure() at :1275 panics after serve, so with_tool_host … |
| hosts-2 · views.rs ProgressSink::after_done (~535-552) is the only production write_document caller, unfenced. | TRUE | crates/htui-worker/src/views.rs:535 `async fn after_done`, :546 `self.writer.write_document(document).await?;`. NewDocument (htui-core/src/model/document.rs:72-90) has no fence field, and WorkerStore::write_document (htui-core/src/store/worker.rs:331) takes … |
| hosts-3 · htui/src/worker_cmd.rs (~57-98) constructs RunRuntime::<PgStore,_>::production() and could take a tool host; store_worker spawn_with (~2021-2028) constructs the TUI … | TRUE | worker_cmd.rs:57 `pub async fn run`, serve() at :70-98, :96 `let runtime = RunRuntime::<PgStore, Unaddressed>::production().with_role(Role::Worker);`, :98 `htui_worker::worker::run(pg, runtime, WorkerConfig::PRODUCTION, shutdown)`. A `.with_tool_host(...)` … |
| hosts-4 · concepts_worker.rs: ConceptIndex (~52) is object-safe with a search method usable for search_concepts; QdrantIndex (~79); constructible in worker_cmd.rs (keyring, model … | TRUE | concepts_worker.rs:52-57 `pub trait ConceptIndex: Send + Sync { fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>>; fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>>; }`. Every method … |
| hosts-5 · cli.rs Command enum (~79-91) and lib.rs dispatch (~84-96): an Mcp variant dispatched before init_tracing beside Worker is feasible; main.rs Sentry init does not write to … | TRUE | cli.rs:77-89 `pub enum Command { Worker(WorkerArgs), Provision(ProvisionArgs) }` (clap::Subcommand; Args has `args_conflicts_with_subcommands = true`, :12). lib.rs:84-89 handles `if let Some(cli::Command::Worker(worker)) = args.command.clone() { return … |
| hosts-6 · tokio 1.53 `net` provides tokio::net::UnixListener (unix) and tokio::net::windows::named_pipe (windows); htui-worker, htui and workspace tokio do not enable net today. | PARTLY → amended (D1 (declare `net` in htui-mcp)) | API: tokio-1.53.1 src/net/mod.rs:54-63 cfg_net_unix!{ pub use unix::listener::UnixListener } and cfg_net_windows!{ pub mod windows } (named_pipe). macros/cfg.rs:374-392 gates them on all(unix, feature="net") and all(windows, feature="net"). Probe … |
| hosts-7 · std::env::current_exe is not used in production code; on Linux after the binary is replaced it returns a path ending in " (deleted)". | PARTLY → amended (D3, D5, R-5) | Usage: the only current_exe calls are in crates/htui/tests/worker_crash_pg.rs:3 and :864 (test). Production code takes its own binary another way: provision/mod.rs:104-110 reads `/proc/self/exe`. Probe: running /tmp/probe_bin printed `/tmp/probe_bin`. After … |
| store-1 · StepFence (traits.rs ~2494-2512) and fenced methods take the fence first after &self (pass_step ~1494); fenced-methods doc ~2481-2491; pg step_fence helper at write.rs … | TRUE | crates/htui-core/src/store/traits.rs:2481-2491 is the doc comment. It lists append_events, set_step_usage, finish_step, set_step_prompt, upsert_step_tree, record_commits, pass_step and park_step. `pub enum StepFence` is at :2494 (Lease(Uuid) / Unleased), and … |
| store-2 · write_document Pg (write.rs ~5312) locks item FOR UPDATE then insert_document; a fenced twin write_step_document can reuse insert_document in the same tx. | PARTLY → amended (D13 lock order) | Shape confirmed. write.rs:5312-5330 opens a tx, runs `SELECT 1 FROM item WHERE id = $1 FOR UPDATE` (NotFound{item}), then `insert_document(&mut tx, new)` and commits. insert_document (write.rs:265) takes `&mut PgConnection`, so it can be reused inside the … |
| store-3 · add_note (Pg write.rs ~5895; Mem mem.rs ~5448) does not check that via_step_id belongs to the note item's run. | TRUE | Pg write.rs:5895-5919 is a single INSERT INTO item_note with no guard of its own; its doc at :5880-5886 says the FK on via_step_id is the only check. MemState::add_note at mem.rs:5448-5490 checks the id is new, the item exists, the user and box exist, and … |
| store-4 · item_link has PK (from,to,kind), CHECK from<>to, proposed_by_step_id FK, deleted_at, a BEFORE UPDATE trigger on updated_at (0001_init.sql ~357-368, ~576); cite/uncite … | TRUE | 0001_init.sql:357-368 has from/to FKs ON DELETE CASCADE, kind CHECK IN ('blocked_by','origin','relates','supersedes'), nullable proposed_by_step_id, created_at/updated_at DEFAULT now(), deleted_at, PRIMARY KEY (from_item_id, to_item_id, kind), CHECK … |
| store-5 · Latest migration is 0013_*; command_run (0001_init.sql ~537-551) has no claimed_by/heartbeat columns; idx_command_run_queue exists; sandbox Postgres supports … | TRUE | crates/htui-store/migrations/ ends at 0013_persona_phase_index.sql, so 0014 is free. 0001_init.sql:537-550 defines command_run with columns id, run_step_id, box_id, class, command, cwd, status CHECK (queued, running, done, failed, cancelled), exit_code, … |
| store-6 · Adding a WriteStore method requires touching traits.rs, mem.rs, pg/write.rs, writer.rs, core store/worker.rs, store worker.rs, htui-agent/src/conformance.rs (UsageSpy), … | PARTLY → amended (D14, T1/T8 file sets) | A multi-line rg finds exactly 5 `WriteStore for` implementors: MemStore mem.rs:6570, PgStore pg/write.rs:783, Writer writer.rs:320, UsageSpy htui-agent/src/conformance.rs:746, SpyStore htui-agent/tests/recorder.rs:434. There are no others: backend.rs:6 says … |
| store-7 · htui-worker runtime.rs command_limits() (~807-823) falls back to {"verify":1}; WorkerHost::app_settings exists (core store/worker.rs ~371); app_setting default … | TRUE | htui-worker/src/runtime.rs:807-823 reads `host.box_row(box_id)?.settings.get("command_limits")`. A missing row, a missing key or a value that does not parse all return `BTreeMap::from([("verify", 1)])`; a read error propagates (D216). A test re-export is at … |
| prompt-box-1 · BoxProfile::project in htui-core/src/model/box_.rs (~395-421) drops path and caps at 24 tools; render::box_profile(profile, HostnameLine) at prompt/render.rs (~402-440) … | TRUE | crates/htui-core/src/model/box_.rs:383-421: `MAX_TOOLS: usize = 24`, `project` sorts by name bytes, truncates, and maps tools to `(name, version)`, so `path` is dropped. `gpu_vendor` is kept only when `gpu_present`. crates/htui-core/src/prompt/render.rs:72-80 … |
| prompt-box-2 · COMMAND_QUEUE_TEXT (htui-core/src/prompt/defaults.rs ~26): what it says, whether it names command_run, and which snapshot/golden tests move if its text or the exposure … | PARTLY → amended (D17, T8) | defaults.rs:26: "Route builds, test suites and verification through the `command_run` tool rather than a shell: htui queues them per box under `R-MCP-4` and records their output on this step. Run everything else directly." It already names `command_run`. It … |
| prompt-box-3 · Item has required_tags (model) and "heavy_build" is used only as a box tag today (htui-core/src/model/box_.rs ~652-703). | TRUE | crates/htui-core/src/model/item.rs:139-140 `pub required_tags: Vec<String>` ("capabilities the executing box must have (R-ORCH-10)"). It also appears on ItemSummary/NewItem/ItemPatch (item.rs:203, 248, 272, 301) and ItemSpec (item_spec.rs:78, 98). No … |
| prompt-box-4 · An ItemStatus enum exists with a FromStr/serde form usable to validate item_status requests. | PARTLY → amended (`item_status`) | No type named `ItemStatus` exists anywhere in crates/ (grep returns nothing). The enum is `Status` in crates/htui-core/src/model/item.rs:8-28, declared through `str_enum!`, and re-exported as `htui_core::model::Status` (model/mod.rs:124-126). `str_enum!` … |
| prompt-box-5 · VectorStore::search (htui-store/src/vector.rs ~361), SearchQuery fields (text, projects, types, statuses, resolutions, limit), Hit fields, Owner {Item, Requirement}. | PARTLY → amended (`search_concepts`) | vector.rs:353-362 `pub trait VectorStore` with `async fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, StoreError>` at :361. It uses async-fn-in-trait, so it is not object-safe. vector.rs:310-327 `SearchQuery { text: String, projects: Vec<ProjectId>, … |
| prompt-box-6 · PermissionRule/PermissionMatch (htui-agent/src/driver.rs ~136-162) with command_prefix + tool_kind, answer RejectOnce, and persona::narrow prepends synthetic reject … | PARTLY → amended (D17) | driver.rs:132-149 `PermissionMatch { tool_kind: Option<String>, tool_name, path_prefix, command_prefix: Option<String> }` (Default, serde default). driver.rs:151-162 `PermissionRule { matcher: PermissionMatch (wire name `match`), answer: PermissionOptionKind, … |
| m5-relay-1 · CLI session: answer_permission returns Unsupported (~584-596); DriverCaps.permission_requests false for CLI (registry.rs ~178); the CLI event stream could merge an extra … | TRUE | crates/htui-agent/src/cli/mod.rs:584-596: `answer_permission` returns `Err(DriverError::Closed)` if `self.ended`, otherwise `Err(DriverError::Unsupported("answer_permission"))`. crates/htui-agent/src/registry.rs:178-179: `Transport::Cli => DriverCaps { … |
| m5-relay-2 · The MOD-42 relay (record/relay.rs ~200-427) handles DriverEvent::PermissionRequest generically for any driver and calls session.answer_permission; chat run_turn … | TRUE | crates/htui-agent/src/record/relay.rs:201-220: `drive(session: &mut dyn AgentSession, ..., relay: Option<&Relay>)`. turn() at 266 matches `(DriverEvent::PermissionRequest(request), true)` and depends only on whether a relay exists, not on caps or transport. … |
| m5-relay-3 · Denial synthesis from permission_denials[] / system permission_denied (cli/claude.rs ~160-165, ~562-605) dedups by request id = tool_use_id. | TRUE | crates/htui-agent/src/cli/claude.rs:160-165: `("system", Some("permission_denied"))` calls `self.denial(Some(tool_use_id))`. Without an id the line falls back to `other(line)`. claude.rs:562-574: denials_of maps each `permission_denials[].tool_use_id` through … |
| m5-relay-4 · ANA-2 capability interlock (docs/ANA-2.md ~476-482): a gated phase is never scheduled onto an agent with permission_requests and edit_proposals both false. Where is this … | PARTLY → amended (D18, T9 (`select.rs`)) | The interlock is in docs/ANA-2.md:476-482 as stated. It is enforced in crates/htui-orch/src/select.rs:166-171 (rule 3 of skip_cause, documented at 132-133): `if input.gate_effective != Gate::Never { let caps = caps_for(agent); if !(caps.permission_requests … |
| m5-relay-5 · SessionSpec (htui-agent/src/driver.rs ~253-283): adding `prompt: Option<PromptPort>` breaks which struct-literal construction sites (count every `SessionSpec {` literal … | PARTLY → amended (D18, T0) | SessionSpec is at crates/htui-agent/src/driver.rs:252-283 with `#[derive(Clone, PartialEq, Eq)]` and a hand-written Debug (285-301). It does not derive Default. There are exactly 10 struct literals in the workspace. All are exhaustive (no `..` base; each ends … |
