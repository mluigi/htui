# Plan: MOD-2 live `claude` over ACP and the chat tab (milestone 3)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 3 (Live `claude` over ACP). Milestones 1 and 2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`5ff6431`); this plan continues its
decision (`D17`+) and task (`T11`+) numbering so a cross-reference never means two things.
Milestones 4–9 are out of scope: where one of them needs a seam, this plan names the seam and stops.
**Design authority**: `docs/ANA-4.md` §3 (protocol surface as read), §4.1 (trait, event model,
recorder rules), §4.2 (SDK, the one-task-per-session bridge, the deadlock rule, process
supervision), §4.3 (permission pipeline, edit proposals, tool-call terminal states), §4.4 (reaching
`claude`: launch, handshake, model selection, session banner), §5.1–5.2 (launch and settings
shapes), §6.1 (ACP → `EventKind` mapping table), §8 (module layout, test strategy), §9 steps 3–4,
§11 criteria 1, 5, 6, 9, 11 and the §11.14 items this milestone can close.
**Requirements**: `R-AGT-1` (the five operations over a real transport), `R-AGT-2` (permissions and
edit proposals), `R-TUI-6` (streamed chat tab with follow-ups and inline permissions), `R-HIS-1`
(the recording half, now fed by a real transport), `R-SEC-3` (the scrubber on the live path),
`R-NF-3` (no store handle on the render side) by construction.
**Complexity**: Large
**Routing**: PRD path, continued (PRD header: C2, C3, C4 fired; ultracode recommended and accepted
for the implement and review phases). Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`).

## Summary

`htui-agent` gains its first real transport: `acp/`, a driver that spawns the agent through
milestone 2's supervised `spawn`, hands the child's piped stdio to the SDK's `ByteStreams`, and
runs one `tokio` task per session that owns the whole `Client.builder()…connect_with(…)` future.
The task turns `session/update` notifications into `DriverEvent`s per ANA-4 §6.1, parks the
`Responder` of every `session/request_permission` until the UI answers, serves `fs/read_text_file`
and intercepts `fs/write_text_file` into an `edit_proposal` with a `similar`-synthesized unified
diff, selects a model through `session/set_config_option` keyed on the option **id**, and records
the session banner as the step's first `other` row. The same thirteen `conformance::CASES` run
against it through an in-process duplex, adding no case, and recorded wire transcripts pin the
mapping under `insta`.

`htui` then grows the surface that drives it: `htui-store` exposes a `Writer` (the one way to reach
a `WriteStore` from a `Backend`, `Offline` still answering `None`), `crates/htui/src/agent_worker.rs`
owns the live chat sessions, `store_worker` gains the four chat request variants and the
`StoreReply::Chat` stream discriminant of ANA-4 §8, and `ui/tabs/chat/**` renders the transcript,
the composer, tool calls, diffs, the capability banner and inline permission options. The event
loop does not change.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D17 | **Module layout**: `crates/htui-agent/src/acp/{mod.rs,map.rs,client.rs,fs.rs}` plus `src/tools.rs`. `mod.rs` holds `AcpDriver`, `AcpSession` and the session task; `map.rs` the `SessionUpdate` → `DriverEvent` table and nothing else; `client.rs` the builder's inbound handlers (permission, `fs/*`) and the client capability block; `fs.rs` the path guard, the old-text read and the `similar` unified-diff synthesis. `tools.rs` holds the `ToolMap` resolver. | ANA-4 §8's layout, split one level finer because §6.1's table is the piece the fixture snapshots pin and it must be readable without the task's plumbing around it. `probe.rs` is still milestone 5. |
| D18 | **One task per session, three channels.** `AcpDriver::start` spawns a `tokio::task` owning `Client.builder().name("htui").on_receive_request(…).on_receive_notification(…).connect_with(ByteStreams::new(child_stdin, child_stdout), async \|cx\| { … })`. `AcpSession` is a handle over `events: mpsc::Receiver<DriverEnvelope>` (capacity 256, `send` awaited so a slow consumer back-pressures rather than drops — the UI channel is the *lossy* one, not this) and `commands: mpsc::Sender<SessionCommand>` (`FollowUp(String)`, `AnswerPermission(PermissionRequestId, PermissionAnswer)`, `Cancel`). Inbound handlers **never** await a store write and never call `SentRequest::block_task`; they forward into an unbounded internal channel and return. `block_task` is called only from the `connect_with` foreground closure, with a comment naming ANA-4 risk 11 at each call site. | ANA-4 §4.2 verbatim: the connection cannot be handed around (`connect_with` borrows it), and `block_task` inside a dispatch handler is a documented deadlock. Bounded events + unbounded inbound is the shape that keeps the deadlock rule true: the handler's send cannot block, and the *task* is what pays the back-pressure. |
| D19 | **The turn is `ActiveSession`, the permission request is a second stream.** The foreground closure runs `cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task()`, then `cx.build_session(cwd).block_task().start_session()` for an `ActiveSession<'static, Agent>` — `start_session` exists only on the `Blocking` builder state, which V31's probe corrected — and each turn is `session.send_prompt(text)` followed by `read_update()` until `SessionMessage::StopReason`. `SessionMessage` is `#[non_exhaustive]`, so the match carries a wildcard arm like every other decode in this crate. Permission requests arrive on the handler channel and are emitted as soon as the task observes them, `select!`-ed against `read_update()`. **Ordering rule, documented in `acp/mod.rs`:** update order within the turn is the SDK's (`SessionMessage` carries dispatches and the stop reason in one ordered channel); a `permission_request` is ordered *only* relative to the events the task had already forwarded, and correlation to its call is `tool_call_id`, never seq adjacency. | `ActiveSession` is the only place the SDK guarantees update/stop-reason ordering (`send_ordered_request_to`, `tests/session_ordering.rs`); rebuilding that by hand to merge one extra stream would trade a guarantee for a convenience. ANA-9 §4.3 already joins permission rows by `tool_call_id` (`idx_session_event_tool`), so nothing downstream reads adjacency. |
| D20 | **Cancel**: `SessionCommand::Cancel` answers every parked responder `RequestPermissionOutcome::Cancelled` **first**, then sends the `session/cancel` notification, then waits for the in-flight turn's `StopReason` up to `grace`, then `Spawned::kill_tree()`. A tool call still open when the turn ends that way gets the synthesized `tool_result { status: failed, terminal_reason: "cancelled" }` the fake already writes; a rejected call gets `terminal_reason: "rejected"`. | ANA-4 §3 ("on cancellation the client MUST answer every outstanding request"), §4.3 "Tool-call terminal states", and `conformance::CASES` entries `cancel_answers_parked_permissions` and `rejected_tool_gets_failed_result`, which the ACP binding must pass unchanged. |
| D21 | **Permission pipeline, all three stages** (§4.3): `settings.permission.rules[]` in order, then `remembered[]`, then ask. Stages 1–2 answer inside the task and record `permission_answer { by: "policy" }` through `Recorder::record_permission_answer`; stage 3 emits `permission_request`, parks the responder and waits for `AnswerPermission`. Selecting an `_always` option forwards that option id **and** returns a `SessionCommand`-level signal the chat tab turns into a `remembered` entry — the write of `agent.settings` itself is **deferred to milestone 7's Settings work**; milestone 3 records the answer and logs `warn!` that the grant was not persisted. | The pipeline is §4.3's; the persistence half needs an `agent.settings` writer the Settings tab does not have yet (`upsert_agent` exists, an editing surface does not). Deferring the write, and saying so in the UI's help line, is honest; emulating the grant in memory would desynchronise `htui` from the agent's own durable grant, which §4.3 rejects. |
| D22 | **Edit proposals, both sources.** `ToolCallContent::Diff { path, oldText, newText }` on `tool_call`/`tool_call_update` synthesizes the unified diff from the pair (`similar::TextDiff::from_lines(…).unified_diff()`, 3 lines of context, `a/<path>`/`b/<path>` headers). `fs/write_text_file` is **intercepted**: read the current text (empty when absent), synthesize, emit `edit_proposal`, then perform the write and answer the request. `fs/read_text_file` is served from disk. Both `fs/*` handlers refuse a path outside `SessionSpec.cwd`/`extra_dirs` with `ErrorCode::INVALID_PARAMS` and record an `error { code: "path_outside_session" }`. Dedup per `(tool_call_id, path)` is the recorder's, already implemented. | §4.3's adopted option ("advertise, intercept") plus the client capability block of §5.2 (`fs_read`, `fs_write` both `true` in the `claude` seed row). The path guard is not in the ANA: an agent that asks the *client* to write outside the session directory is asking `htui` to do it, and `R-ID-4`'s read-only posture makes the refusal the conservative default. |
| D23 | **Model selection** is `session/set_config_option` keyed on the option **id**: `settings.acp.model_config_id` when the row sets one, otherwise the option whose `options[].value` list contains `SessionSpec.model`. No lookup by `category`. An agent that offers no matching option gets no request at all and the step records `other { update: "model_unavailable", requested }`; `SessionSpec.model` is then advisory, as §4.4 requires. A `config_option_update` pushes a complete list and is stored verbatim as `other`. | §3 ("categories are UX-only and MUST NOT be required for correctness") and §4.4. Verified on this box (V21): `session/new` returns `configOptions` with ids `mode` and `model`, the `model` option carrying `default`/`opus[1m]`/`sonnet`/`sonnet[1m]`/`haiku` values — so the id path is real and the vocabulary is per-installation, which is exactly why the row stores an id and not a value table. |
| D24 | **Session banner** is the step's first `other` row: `{ "update": "session_started", "session_id", "protocol_version": 1, "agent_name", "agent_version", "models": [...] }`, built from the `initialize` response (`agentInfo.name`/`.version`) and the `session/new` `configOptions` model values. It is emitted before the first turn's events, which is what makes `conformance::CASES`' `session_banner_is_first_other_row` pass on this transport. | §4.4 "Session load and resume": the agent-side session id has no column, and resuming is a query for this row. |
| D25 | **`ToolMap` resolution is minimal here and replaced in milestone 5.** `tools::resolve(&Discovery) -> Result<ToolMap>` handles `ToolProbe::Path` through `which` (already a dependency) and `ToolProbe::NodePackage` by checking, in order, `$HTUI_TOOL_<NAME>` (uppercased key), `<cwd>/node_modules/<package>/<entry>`, then `npm root -g` joined with the same suffix; `ToolProbe::Glob` returns `DriverError::Unresolved(name)` with a message naming milestone 5. No version check, no `agent_box.probe` write, no persistence. | ANA-4 §4.6's tiers, the probe snapshot and `agent_box` are milestone 5 (`docs/ANA-4.md` §9 step 5), but a chat session cannot start without a resolved `${node}` / `${claude_agent_acp}` / `${claude}`. This is the smallest resolver that starts the seeded `claude` row on a real box and it is deliberately not the probe: it stores nothing. `agy`'s glob probe is milestone 6's, which is why the `Glob` arm is a named error rather than a guess. |
| D26 | **`htui-store::Writer`**: `pub enum Writer { Memory(MemStore), Online(PgStore) }` with `impl ReadStore + WriteStore for Writer` (plain delegation) and `Backend::writer(&self) -> Option<Writer>`, `Offline` answering `None`. `backend.rs`'s module doc is amended in the same commit: the invariant is unchanged — there is still no `impl WriteStore for Backend`, and an offline backend still hands out no writer — but a *recorder* needs an owned handle it can hold across a session, which `&PgStore` behind a `&Backend` cannot give. | The recorder is generic over `S: WriteStore` and lives in a spawned task; `Backend::writable()` returns a borrow. `MemStore` must be reachable too or `--demo` cannot hold a chat at all, and the demo path is where every chat-tab snapshot runs. Both variants are `Clone` handles over shared state, so a `Writer` is a handle, not a copy of the store. |
| D27 | **The chat seam is four `StoreRequest` variants and one stream reply** (ANA-4 §8): `ChatStart { project_id, agent_id, model, prompt }`, `ChatSend { step_id, text }`, `ChatAnswer { step_id, request_id, answer }`, `ChatCancel { step_id }`; replies `StoreReply::Chat(ChatFrame)` and `StoreReply::ChatAccepted { step_id, session_ref, caps }`. `ChatFrame` is `Event(Box<DriverEnvelope>)`, `Ended { stop_reason }` or `Failed { message }`. The frames are emitted by the session task through a clone of the worker's `mpsc::UnboundedSender<ReplyEnvelope>`, stamped with the **`ChatStart` envelope's own `seq` and origin**, so every frame passes `App::is_fresh` for as long as that chat is the tab's newest `ChatStart`. | §8's "no fourth `select!` arm" and its warning that a stream needs its own discriminant. The four commands are served in the `spawn` loop ahead of `try_serve` (the `ApplyMigrations` precedent, `store_worker.rs:308`), because they need the loop's state — the live-session map — not just a `&Backend`. |
| D28 | **`agent_worker.rs` owns the sessions, not the store worker.** `AgentRuntime { factory: DriverFactory, live: HashMap<StepId, LiveChat> }` where `LiveChat { commands: mpsc::Sender<ChatCommand>, task: JoinHandle<()>, session_ref, caps }`. `AgentRuntime::start(&Backend, &tx, envelope) -> StoreReply` resolves box and user from the backend, mints the `ChatRunSpec`, calls `start_chat_run`, builds the driver from the registry row and spawns `run_chat(...)`; `run_chat` is a plain `async fn` that pumps `record::pump` per turn and calls `finish_chat_run` on the way out. A backend with no writer (`Offline`) refuses with `StoreReply::Failed { request: "chat_start", message: "chat needs a writable store" }`. | Milestone 4 owns the offline buffering path (`append_pending`), so refusing is the correct milestone-3 behaviour rather than a silent memory-only session. `run_chat` being a free `async fn` is what lets the test harness await it inline (D30) instead of racing a spawned task. |
| D29 | **Chat identity**: `project_id` is the chat tab's current project (`Ctx::projects[0]`, shown in the tab header; a picker is MOD-13/MOD-15 work), `target_box_id` comes from `Backend::box_info()`, `started_by` from a new inherent `Backend::this_user() -> Result<UserId>` (`Online` → `PgStore::this_user()`, `Memory` → the fixture's first `app_user`, `Offline` → `StoreError::Unreachable`). `SessionSpec.retain_raw` is **`false`** in milestone 3, overridable with `HTUI_KEEP_RAW_EVENTS=1`: `project.settings` has no reader until MOD-15 and inventing one here would put a projection under this plan's review with no editor behind it. | `ChatRunSpec::mint` needs all three ids (`crates/htui-core/src/model/run.rs:229`). Reading them in the worker keeps `R-NF-3` intact — the tab never learns a `UserId`. Criterion 4 (`raw` iff `retain_raw`) is proven by the conformance case on both settings, so the default is a policy choice, not an untested path. |
| D30 | **Determinism in the harness.** `testkit::Harness` gains `with_agent_runtime(AgentRuntime)` and `drive(&mut self)`, which serves queued chat requests **inline** — `AgentRuntime::start` returns the session future and `drive` awaits it to completion, feeding every frame through `App::update` — exactly as `settle()` serves store requests inline. Production spawns; the harness awaits. The chat-tab snapshots run over milestone 2's `FakeAdapter` (`cli/fake`) through `DriverFactory::with_test_support()`, so no snapshot touches ACP. | ANA-4 §8 test strategy 1: "a `Harness::drive()` pump that mirrors `Harness::settle()`'s inline model … a streaming driver must not break that property". Byte-stable snapshots with no sleeps is the property; a spawned task would need one. |
| D31 | **Deliberately absent from milestone 3**: `StepEvents` replay and the offline session path (M4), `probe.rs`, `agent_box` writes and migration `0002` (M5), `agy` (M6), quota latching and cap enforcement (M7), `cli/` (M8), the prompt assembler and preview (M9). The `prompt` row a chat records is the text the user typed; the assembled prompt is M9's, and the row shape does not change when it arrives. Windows is **not** verified in this milestone: the box this runs on is Linux, so the job-object half of §11 criterion 11 stays open and the process-group half is what this milestone proves. | Scope discipline per the PRD; each is a named seam, not a plan. The Windows note is carried forward from milestone 2's phase note rather than quietly dropped. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Transport task | `docs/ANA-4.md` §4.2 diagram; `crates/htui-agent/src/fake.rs` | one task per session, channels at the trait boundary, `AgentSession` holding endpoints only |
| Mapping module | `docs/ANA-4.md` §6.1 | one arm per wire variant, a wildcard arm into `other`, no transport type escaping the module |
| Errors | `crates/htui-agent/src/error.rs`, `crates/htui-core/src/store/error.rs` | one `thiserror` enum per crate; an SDK error becomes `DriverError::Transport(String)`, never leaks |
| Worker seam | `crates/htui/src/store_worker.rs:41-43,45-106,198-220,303-342` | new request = one variant, one `name()` arm, one served arm; loop-state requests answered ahead of `try_serve` like `ApplyMigrations` |
| Tab | `crates/htui/src/ui/tabs/backlog/mod.rs`, `settings/mod.rs` | `Tab` impl + sub-registry, `wants_requests`/`on_reply`/`on_key`/`render`, registration in `app::register_all` |
| Tests | `crates/htui-agent/tests/fake_conformance.rs`, `crates/htui/tests/settings.rs`, `crates/htui-core/src/store/conformance.rs:20-86` | named `CASES` + a loop reporting per case; explicit-name `insta::assert_snapshot!`; `Harness` with `settle()` inline |
| Logging | `crates/htui/src/lib.rs`, `crates/htui-agent/src/launch.rs` | `tracing` to the file sink, `warn!` with structured fields for every degraded path (unresolved tool, refused path, dropped UI frame) |

## Files to Change

Task file sets are the independence facts for §3.5; disjointness within each wave is verified in
V29. A task is independent only when its file set is disjoint **and** every type, function and
fixture value it names already exists in its final form or is produced by the same task (the
stricter rule milestones 1–2 arrived at).

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` (workspace) | UPDATE | T11 | `tokio` workspace features gain `fs` (the `fs/*` handlers read and write files) |
| `crates/htui-agent/Cargo.toml` | UPDATE | T11 | `similar` (declared in M2, first consumer here) and `tokio` `fs`. **Not** `futures`: `tokio_util::compat` already supplies the `AsyncRead`/`AsyncWrite` `ByteStreams` wants and the probe compiled without it, so it is added only if a direct `futures::` use appears |
| `crates/htui-agent/src/tools.rs` | CREATE | T11 | D25 `ToolMap` resolver |
| `crates/htui-agent/src/acp/mod.rs`, `acp/map.rs`, `acp/client.rs`, `acp/fs.rs` | CREATE | T11 | D17–D24: driver, session task, §6.1 mapping, inbound handlers, diff synthesis |
| `crates/htui-agent/src/lib.rs`, `src/registry.rs` | UPDATE | T11 | `pub mod acp` / `pub mod tools`; `DriverFactory::with_acp()` registering the `acp` adapter |
| `crates/htui-agent/tests/acp_conformance.rs` | CREATE | T12 | the in-process `CaseHarness`, all thirteen `CASES`, `CASES.len()` re-asserted |
| `crates/htui-agent/tests/acp_map.rs`, `tests/fixtures/claude_acp_*.jsonl`, `tests/snapshots/acp_map__*.snap` | CREATE | T12 | recorded-transcript regression net (ANA-4 §8 strategy 3) |
| `crates/htui-agent/tests/acp_live.rs` | CREATE | T12 | `#[ignore]` handshake smoke test (§8 strategy 5, §11 criterion 9) |
| `crates/htui-store/src/writer.rs`, `src/lib.rs`, `src/backend.rs` | CREATE / UPDATE | T13 | D26 `Writer`, `Backend::writer`, `Backend::this_user`, amended module doc |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | T13 | `writer()` and `this_user()` over a live server |
| `crates/htui/Cargo.toml` | UPDATE | T13 | `htui-agent` dependency; dev-dependency with `test-support` |
| `crates/htui/src/agent_worker.rs`, `src/lib.rs` | CREATE / UPDATE | T13 | D28 `AgentRuntime`, `run_chat`, `ChatCommand` |
| `crates/htui/src/store_worker.rs` | UPDATE | T13 | D27 four request variants, `name()` arms, `StoreReply::Chat`/`ChatAccepted`, the served arms in `spawn` |
| `crates/htui/src/ui/tabs/chat/mod.rs`, `chat/transcript.rs`, `chat/composer.rs`, `chat/permission.rs` | CREATE | T14 | `R-TUI-6` |
| `crates/htui/src/ui/tabs/mod.rs`, `src/app/mod.rs`, `src/testkit.rs` | UPDATE | T14 | tab export, registration, D30 `drive()` |
| `crates/htui/tests/chat.rs`, `tests/snapshots/chat__*.snap` | CREATE | T14 | chat-tab snapshots over the fake |
| `README.md` | UPDATE | T14 | the chat tab and the `HTUI_TOOL_*` / `HTUI_KEEP_RAW_EVENTS` knobs |

## Tasks

Waves. **A** = {T11} alone (it is the milestone's core and every later task names its types).
**B** = {T12, T13} parallel — disjoint file sets, and each names only what T11 has already landed
(T12: `AcpDriver`, `CaseHarness`, `conformance::CASES`; T13: `DriverFactory`, `AgentDriver`,
`Recorder`, `pump`). **C** = {T14} serial after T13 (it renders `StoreReply::Chat` frames and
registers the tab). Reviewer runs once, over the whole milestone. TDD per task: the tests named
under **Tests first** are written and failing before the implementation.

Every implementer prompt carries: ANA-4 has priority over this plan where they disagree (MOD-1 /
MOD-6 / milestone 1–2 precedent); no `unsafe` (`Cargo.toml:35`); no lock or pool connection held
across an `.await`; `.sqlx` regenerated and committed with any `PgStore` query change; every
Postgres run prefixed `USERNAME=htui-ci` and carrying
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres` (TOOL-2 — a suite that
"passed" without the variable proved nothing).

### Task 11: the ACP transport — `independent`

- **Files**: `Cargo.toml` (workspace); `crates/htui-agent/Cargo.toml`, `src/tools.rs`,
  `src/acp/mod.rs`, `src/acp/map.rs`, `src/acp/client.rs`, `src/acp/fs.rs`, `src/lib.rs`,
  `src/registry.rs`.
- **Tests first** (unit tests inside the new modules, no process spawned):
  `map.rs` — every `SessionUpdate` variant of §6.1 maps to the named `EventKind` and payload keys
  (built from `serde_json` literals of the wire shapes, not from constructors), `user_message_chunk`
  maps to `None` (dropped), an unknown `sessionUpdate` string lands in `other` with the verbatim
  body, `ToolKind` decodes all ten values including `switch_mode`, a `tool_call_update` with
  `status ∈ {completed, failed}` becomes `tool_result` and one with any other status does not.
  `fs.rs` — a new-file write produces a unified diff that applies to an empty file and yields the
  written content; a second write to the same path in the same call is the recorder's dedup, not a
  second row; a path outside `cwd` and `extra_dirs` is refused and produces no write.
  `tools.rs` — `Path` resolves through `which`, `$HTUI_TOOL_NODE` overrides it, `NodePackage`
  prefers a local `node_modules` entry over the global root, `Glob` is `Unresolved` naming
  milestone 5.
  `client.rs` — the advertised client capability block equals `settings.acp.client_capabilities`
  and omits `terminal`; a `RequestPermissionRequest` decodes into `PermissionRequestEvent` with all
  four `PermissionOptionKind` values accepted.
- **Action**: D17–D25. `AcpDriver { agent: Agent, settings: AgentSettings, caps: DriverCaps }`
  implementing `AgentDriver`; `start` resolves the launch (`tools::resolve` + `launch::resolve`),
  spawns through `launch::spawn`, takes `stdin`/`stdout`, builds `ByteStreams::new(stdin, stdout)`
  and spawns the session task; `AcpSession` implementing `AgentSession` over the two channels;
  `SessionCommand`; the foreground closure doing `initialize` → `session/new` → banner → model
  config → turn loop; `map.rs` as the §6.1 table; `client.rs`'s two `on_receive_request` handlers
  and one `on_receive_notification`; `fs.rs`'s guard, read and `similar` diff;
  `DriverFactory::with_acp()` registering `"acp"` (production registration; `with_test_support()`
  keeps registering `cli/fake`).
- **Mirror**: `docs/ANA-4.md` §4.2's diagram and deadlock paragraph; the SDK's own
  `examples/yolo_one_shot_client.rs` for the builder shape; `crates/htui-agent/src/fake.rs` for the
  five rules a harness owes.
- **Validate**: `cargo test -p htui-agent`, `cargo clippy -p htui-agent --all-targets -- -D warnings`,
  `cargo doc -p htui-agent --no-deps`, `cargo tree -i tokio -e normal --workspace` unchanged from
  milestone 2's baseline (criterion 13 stays true after the `fs` feature).

### Task 12: ACP conformance, transcript fixtures and the live smoke test — `independent` (needs T11)

- **Files**: `crates/htui-agent/tests/acp_conformance.rs`, `tests/acp_map.rs`,
  `tests/fixtures/claude_acp_handshake.jsonl`, `tests/fixtures/claude_acp_turn.jsonl`,
  `tests/snapshots/acp_map__*.snap`, `tests/acp_live.rs`.
- **Tests first**: `acp_conformance.rs` — `assert_eq!(conformance::CASES.len(), 13)` and
  `run_all(&AcpHarness, || MemStore::demo())`, the harness building a `tokio::io::duplex` pair,
  driving `AcpDriver` on one end and a **scripted agent** on the other that speaks raw
  newline-delimited JSON-RPC (no SDK on the agent side, so the test proves the wire format and not
  a symmetry of the same library): it answers `initialize`/`session/new`, plays the `Script`'s
  turns as `session/update` notifications, issues `session/request_permission` for
  `ParkPermission`, and answers `session/prompt` with the turn's stop reason.
  `acp_map.rs` — the two fixtures replayed through `acp::map` into `Vec<SessionEvent>` under named
  `insta` snapshots; an unknown update kind added to the fixture lands in `other` rather than
  breaking the build.
  `acp_live.rs` — `#[ignore]`, spawns the seeded `claude` row through `tools::resolve` and asserts
  `protocolVersion == 1`, `agentInfo.version` non-empty and `authMethods` present; skipped by name
  in CI, run by hand.
- **Action**: the harness, the fixtures (captured from one real one-word prompt through an
  `HTUI_ACP_TRACE=<path>` line dump added in T11's session task — see open question 3), the
  snapshots, the smoke test.
- **Mirror**: `crates/htui-agent/tests/fake_conformance.rs` (the binding shape); `docs/ANA-4.md` §8
  strategies 2, 3 and 5.
- **Validate**: `cargo test -p htui-agent --features test-support`;
  `cargo test -p htui-agent --features test-support -- --ignored` on this box, quoting the
  handshake; clippy and doc as T11.

### Task 13: the store writer, the agent worker and the chat request seam — `independent` (needs T11)

- **Files**: `crates/htui-store/src/writer.rs`, `src/lib.rs`, `src/backend.rs`,
  `tests/pg_criteria.rs`; `crates/htui/Cargo.toml`, `src/agent_worker.rs`, `src/lib.rs`,
  `src/store_worker.rs`.
- **Tests first**: `htui-store` — `Backend::writer()` is `Some` for `Memory` and `Online` and
  `None` for `Offline`; a `Writer::Memory` round-trips `start_chat_run` → `active_runs` →
  `finish_chat_run`; `this_user()` answers a `UserId` on `Memory` (the fixture's first user) and
  `Unreachable` on `Offline`; `pg_criteria.rs` covers the `Online` arm against the live server.
  `htui` — `store_worker` unit tests: each of the four chat requests has a `name()` arm and a
  served arm; `ChatStart` against `Backend::Offline` answers
  `Failed { request: "chat_start", .. }`; `agent_worker` tests over `MemStore::demo()` and
  `DriverFactory::with_test_support()`: `run_chat` with a scripted fake writes a `prompt` row, the
  scripted events and a `done`, closes the run (`active_runs` returns to its previous count), and
  emits one `ChatAccepted` plus one `Chat(Event(..))` per event and one `Chat(Ended{..})` down the
  reply channel, every frame carrying the `ChatStart` seq and origin; a `ChatSend` for an unknown
  `step_id` answers `Failed`; `ChatCancel` ends the run with `RunStatus::Cancelled`.
- **Action**: D26–D29. `Writer` and its two `impl`s; `Backend::writer`, `Backend::this_user` and
  the amended module doc; `AgentRuntime`, `LiveChat`, `ChatCommand`, `run_chat`; the four
  `StoreRequest` variants, `StoreReply::Chat`/`ChatAccepted`, their `name()` arms, and the served
  arms placed ahead of `try_serve` in `spawn` with `AgentRuntime` owned by that loop.
- **Mirror**: `crates/htui-store/src/backend.rs` dispatch style; `store_worker.rs:308-330`
  (`ApplyMigrations` as the loop-state precedent); `crates/htui-agent/src/record.rs`'s `pump`.
- **Validate**: `cargo test -p htui-store --features demo` and, with Postgres,
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`;
  `cargo test -p htui`; clippy over both crates.

### Task 14: the chat tab — `serial-after: T13`

- **Files**: `crates/htui/src/ui/tabs/chat/mod.rs`, `chat/transcript.rs`, `chat/composer.rs`,
  `chat/permission.rs`; `crates/htui/src/ui/tabs/mod.rs`, `src/app/mod.rs`, `src/testkit.rs`;
  `crates/htui/tests/chat.rs`, `tests/snapshots/chat__*.snap`; `README.md`.
- **Tests first** (`crates/htui/tests/chat.rs`, `Harness` + `FakeAdapter`): snapshot
  `chat_empty` — the tab before a session, showing the agent, the project and the composer hint;
  `chat_streamed_turn` — a scripted turn of assistant text, one collapsed thought, a tool call and
  its result renders in `seq` order with the tool call folded to one line;
  `chat_edit_proposal` — an `edit_proposal` renders as a diff with `+`/`-` gutters;
  `chat_permission_inline` — a parked request renders its options numbered, `2` answers it, and the
  answer is what the store shows (`step_events` holds a `permission_answer` with `by: "user"`);
  `chat_capability_banner` — a driver whose `DriverCaps` has `permission_requests: false` shows the
  banner naming what it cannot do; `chat_follow_up` — `i`, text, `Enter` issues exactly one
  `ChatSend` and the transcript grows a `follow_up` row; `chat_cancel` — `Esc` `Esc` issues
  `ChatCancel` and the transcript ends with `done { stop_reason: cancelled }`;
  `chat_offline` — a `Failed` chat reply renders the one-line refusal instead of an empty pane.
- **Action**: `ChatTab` (`TabId("chat")`) with a transcript pane, a composer, a permission strip
  and a header line (agent · model · project · session id); key handling: `i`/`Enter` composes,
  `Esc` leaves compose mode and a second `Esc` cancels the session, digits answer a pending
  permission while one is parked (consumed before the global tab bindings see them), `j`/`k` and
  `g`/`G` scroll, `t` toggles thought folding. `register_all` registers it fourth.
  `testkit::Harness::with_agent_runtime` / `drive` per D30. README section for the tab and the two
  environment knobs.
- **Mirror**: `crates/htui/src/ui/tabs/backlog/detail/runs.rs` (table render),
  `settings/mod.rs` (sub-registry and strip), `crates/htui/src/keymap.rs` (scoped bindings and the
  help line).
- **Validate**: `cargo test -p htui --features testkit`; `cargo test --workspace --all-features`;
  clippy; `cargo run -p htui -- --demo` → `4` → a scripted chat renders; and the live run of the
  milestone: `cargo run -p htui` against Postgres, holding a real `claude` conversation with a tool
  call and a permission answered inline.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features
cargo test -p htui-agent --features test-support -- --ignored   # the live handshake smoke test
cargo doc --workspace --no-deps
cargo tree -i tokio -e normal --workspace                       # criterion 13 still holds
cargo run -p htui -- --demo                                     # tab 4, scripted chat
```

ANA-4 §11 mapping: criterion 1 → T12 (a second harness, zero new cases); 5 → T12
(`cancel_answers_parked_permissions` now on a real transport); 6 → T11's `fs.rs` tests plus T12's
case; 9, first half → T12's live smoke test (the "box without node" half is milestone 5's probe);
11, process-group half → T11 (`kill_tree` after a live cancel, asserted by `pgrep` in the live run
write-up; the Windows job-object half stays open, D31). Criteria 2, 3, 4, 12 were closed in
milestone 1 and are re-run unchanged; 7, 8, 10, 13 belong to milestones 5–8.

**Prerequisites (met on this box)**: `node` v22.19.0, `claude` 2.1.263,
`@agentclientprotocol/claude-agent-acp` **0.48.0** installed globally, dev Postgres on port 5439.

## Verified claims (§3.5 fact-check)

Filled by the planner against the tree, the vendored SDK source and a live probe on 2026-09-07
(`tree` = grep/read; `registry` = `~/.cargo/registry/src/**/agent-client-protocol-2.1.0`;
`live` = a process actually run on this box). The main thread re-verifies before CONFIRM.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V16 | The SDK exports `ByteStreams`, `Responder`, `ConnectionTo`, `SentRequest`, `Client`, `Agent`, `Dispatch` from the crate root | true | registry `src/lib.rs:123-143` |
| V17 | The client builder takes inbound handlers and a foreground closure: `Client.builder().on_receive_notification(..).on_receive_request(..).connect_with(transport, \|cx\| async {..})` | true | registry `examples/yolo_one_shot_client.rs:46-110` |
| V18 | `ActiveSession::read_update()` yields `SessionMessage::{SessionMessage(Dispatch), StopReason(_)}` in one ordered channel, and `send_prompt` routes the stop reason into it via `send_ordered_request_to` | true | registry `src/session.rs:976-1056` |
| V19 | `Responder` can be parked and answered later: its send closure is `Box<dyn FnOnce(..) + Send>` | true | registry `src/jsonrpc.rs:4465-4486` |
| V20 | `ByteStreams::new(outgoing, incoming)` takes `futures::io::AsyncWrite` + `AsyncRead`, and `Spawned::take_stdin/take_stdout` already return `tokio_util::compat::Compat` handles | true | registry `src/jsonrpc.rs:6386-6400`; tree `crates/htui-agent/src/launch.rs:450-462` |
| V21 | The installed adapter answers `initialize` with `protocolVersion: 1`, `loadSession: true`, `sessionCapabilities` as **empty objects**, `authMethods: []`, `agentInfo.version "0.48.0"`; `session/new` returns `configOptions` with ids `mode` and `model`, the `model` option listing `default`, `opus[1m]`, `sonnet`, `sonnet[1m]`, `haiku` | true | live, 2026-09-07: `node …/claude-agent-acp/dist/index.js` fed `initialize` then `session/new` |
| V22 | This box holds `node` v22.19.0, `claude` 2.1.263, `npx`, `agy`, and `@agentclientprotocol/claude-agent-acp` **0.48.0** under the volta global root — not the 0.55.0 of ANA-4's box nor the registry's pinned 0.75.0 | true | live `which` / `--version` / `npm root -g` |
| V23 | `conformance::CASES` holds exactly thirteen names and `CaseHarness` has one method, `driver(&self, script: Script) -> Box<dyn AgentDriver>` | true | tree `crates/htui-agent/src/conformance.rs:117-141` |
| V24 | `DriverFactory` keys on `acp` / `cli/<stream>` with no arm on `agent.name`, so registering the ACP builder is one `register` call | true | tree `crates/htui-agent/src/registry.rs:62-116` |
| V25 | `Recorder::new(store, scrubber, step, retain_raw, ui)` and `pump(session, recorder)` exist with those signatures, and the recorder already owns the edit-proposal dedup and the synthesized-row rules | true | tree `crates/htui-agent/src/record.rs:311-317,991-994` |
| V26 | `Backend` is `Clone`, exposes `writable() -> Option<&PgStore>` and has **no** `WriteStore` impl; `PgStore::this_user()` exists | true | tree `crates/htui-store/src/backend.rs:28,97-102`; `crates/htui-store/src/pg/mod.rs:406` |
| V27 | `ChatRunSpec::mint(project_id, target_box_id, started_by, agent_id, model)` and `WriteStore::{start_chat_run, finish_chat_run}` are already in the tree | true | tree `crates/htui-core/src/model/run.rs:225-241`; `crates/htui-core/src/store/traits.rs:124-139` |
| V28 | `App::is_fresh` passes every reply carrying a `seq` still recorded for that origin, so many frames under one `ChatStart` seq all reach the tab | true | tree `crates/htui/src/app/state.rs:282-286` |
| V29 | The wave-B file sets are disjoint: T12 touches only `crates/htui-agent/tests/**`; T13 touches `crates/htui-store/src/{writer,lib,backend}.rs`, `crates/htui-store/tests/pg_criteria.rs` and `crates/htui/{Cargo.toml,src/{agent_worker,lib,store_worker}.rs}` | true | the tables above; re-checked by the main thread before launch |
| V30 | `similar 3.2.0` is declared in `[workspace.dependencies]` with no consumer, and `htui` does not yet depend on `htui-agent` | true, line corrected | tree `Cargo.toml:**49**` (the plan first cited 47); `crates/htui/Cargo.toml`. `Cargo.lock` today holds `similar 2.7.0` only, as `insta`'s transitive; T11 adds `3.2.0` beside it, which is a second major in the graph and expected |
| V31 | **Compile probe** (`/tmp/acp_probe`, `rustc 1.98.1`, edition 2024, the same pinned SDK): `Client.builder().name(..).on_receive_notification(..).on_receive_request(..).connect_with(ByteStreams::new(w.compat_write(), r.compat()), async \|cx: ConnectionTo<Agent>\| {..})` compiles and the whole future is `tokio::spawn`-able; a `Responder<RequestPermissionResponse>` forwards out of the handler, parks in a `HashMap` **across** `.await` points and is answered later; `read_update()` composes in a `tokio::select!` with a second channel; `tokio::io::duplex` + `split` + compat is a valid transport pair for T12's harness; `similar 3.2.0` resolves and `TextDiff::from_lines(..).unified_diff().context_radius(3).header(..)` compiles | true, **with two corrections** | the probe built clean after them: (1) the plan's `build_session(cwd).start_session()` is **wrong** — `start_session` exists only on `SessionBuilder<_, _, Blocking>`, so it is `build_session(cwd).block_task().start_session()` (`E0599`); (2) `SessionMessage` is `#[non_exhaustive]` and a closed match is `E0004`. Both are folded into D19 |
| V32 | The one claim this milestone could **not** verify: the Windows job object, `PATHEXT` shim resolution and `CREATE_NO_WINDOW`. This box is Linux | unverifiable here | carried as D31 and open question 5 rather than asserted |

## Open questions, answered at CONFIRM (2026-09-07)

All five were put to the maintainer with a recommendation each and **all five were accepted as
recommended**: (1) `_always` grants recorded but not persisted, (2) `retain_raw` default `false`
with the environment override, (3) fixtures captured from one live prompt, (4) the scope's first
project, (5) Windows carried as a gap in the phase note. The questions stay below as written so the
decision is readable next to what it decided.

Two execution rulings given with the confirmation:

- **`code-architect` and `rust-reviewer` run as one subagent each, on `claude-fable-5-1`.** Both
  agent definitions were copied from the (disabled) `ecc` plugin cache into `~/.claude/agents/` and
  their `model:` field changed from `sonnet` to `claude-fable-5-1`.
- **No implementer fan-out**, as in milestone 2: the tasks run serially on the main thread under
  TDD. The `rust-reviewer` gate and the close-out validator are unchanged.

## Blueprint amendments (2026-09-07, `code-architect` on `claude-fable-5-1`)

`.claude/plans/mod-2-live-acp-chat.blueprint.md` is the implementer's contract. Reading the SDK and
the tree against this plan, it corrected nine things; each is **adopted**, and where a plan decision
is overruled the reason is ANA-4 or the SDK source, never preference.

| # | Plan said | Blueprint corrected it to | Why |
|---|---|---|---|
| H-1 | D17: one `on_receive_notification` handler on the builder | **No** notification handler; updates reach the task untyped through `ActiveSession::read_update()` and `map.rs` reads JSON | A builder handler claims every `session/update` before the SDK's own `ActiveSessionHandler`, starving `read_update()`; a typed decode also drops the claude adapter's off-schema kinds, which §6.1's wildcard row exists to keep |
| H-2 | T12 touches `tests/**` only; the thirteen cases "pass unchanged" | Two **scripts/assertions** in `crates/htui-agent/src/conformance.rs` are amended: `usage_deltas_sum_to_step_usage` drops the four token fields (ACP has no token field in `usage_update`), `edit_proposal_deduped_per_call_and_path` asserts the diff *contains* the marker instead of equalling it (the unified diff is synthesized, not carried) | ANA-4 §7 and §4.3. `CASES` and its length do not change, so §11 criterion 1 ("adding a transport adds no case") still holds — but this is a weakening of two milestone-1 assertions and is called out rather than slipped in |
| H-3 | D18: `commands` is a bounded `mpsc::Sender` | **Unbounded** | A bounded command channel plus the awaited bounded `events` channel is a two-party deadlock: the handle blocks sending a command while the task blocks sending an event the handle is not draining. One message per user action makes unbounded honest |
| H-4 | D28: `run_chat` "pumps `record::pump` per turn" | `run_turn` pulls, and only while nothing is parked; `record::pump` is untouched | `pump` cannot cross a parked permission request — `next_event` answers `Err(Transport)` on every transport while one is parked (`conformance.rs:827-832`) |
| H-5 | D21: stages 1–2 of the permission pipeline run "inside the task" | New pure module `crates/htui-agent/src/permission.rs`, called by `run_chat` before the request is parked; `crates/htui-core/src/store/mem.rs` gains `this_user()` | The recorder lives in `run_chat`, not in the session task, so the task cannot write the `by: "policy"` row. Two file-set additions (T11, T13) |
| H-6 | D27/D28 imply `ChatAccepted` is the immediate reply | Deferred: `serve(ChatStart)` does the store writes and hands back the future; `run_chat` sends `ChatAccepted` after the handshake | Spawning node, `initialize` and `session/new` take seconds; doing them inside `serve` stalls every other store request behind them |
| H-7 | nothing about the clock | `Stamp::{Fixed, Wall}` seam on `AcpDriver` | §11 criterion 2 needs replay-identical rows; a wall-clock `at` inside the driver fails `coalesce_across_message_id` on ACP |
| H-8 | T14 validates `--demo` → `4` → "a scripted chat renders" | `--demo` → `4` → **the empty tab renders**; the scripted chat lives in the snapshots | Both demo agents are `transport: acp` and the fake adapter is `test-support`-only, so a `--demo` binary has no scripted driver to run |
| H-9 | nothing about quit | `lib.rs` quit order becomes `drop(app)` → worker drains and cancels every live chat → awaited with a timeout → `abort` as the backstop (today `lib.rs:87` aborts at once) | Otherwise the runtime drops the session task at its first await and orphans the child, which is §11 criterion 11 failing on the one path no test covers |

Also carried from the blueprint: the path guard is **lexical** (a symlink inside `cwd` pointing out
is admitted, documented as such), `raw` over ACP is the parsed message re-serialised rather than the
wire bytes (the bytes are `HTUI_ACP_TRACE`'s), an unknown `PermissionOptionKind` decodes to
`reject_once` with a `warn!`, and `tool_name` matching has nothing to match on ACP v1 (it compares
against `title`, and says so).

## The questions as asked

1. **Permission `_always` grants are recorded but not persisted** into `agent.settings.remembered[]`
   (D21), because no writer for that column exists before the Settings editing work. Accept the
   deferral, or add the `upsert_agent` round trip here?
2. **`retain_raw` defaults to `false`** with an `HTUI_KEEP_RAW_EVENTS` override until MOD-15 gives
   `project.settings` a reader (D29). Accept?
3. **The transcript fixtures are captured from one live prompt** ("reply with the word ok"), which
   spends a few tokens of the maintainer's subscription and records real wire lines into the repo
   after the scrubber has run over them. Confirm, or hand-write the fixtures from the shapes
   observed in V21 instead?
4. **Chat runs against the scope's first project** (D29); a project picker is MOD-13/MOD-15 work.
   Accept?
5. **Windows stays unverified** in this milestone (D31): the job object, `PATHEXT` resolution and
   `CREATE_NO_WINDOW` are exercised only on Linux here. Accept, with the gap carried in the
   milestone-3 phase note?

---
## Close-out (2026-09-07)

Landed as `a142fbf`..`682a423`: `2684c78` (T11, the ACP transport), `3018b7a` (T12, conformance
over a duplex plus recorded fixtures and the live smoke test), `1f49b71` (T13, `Writer`,
`AgentRuntime` and the four chat requests), `5e1bdaa` (T14, the chat tab), `682a423` (the review
gate's findings).

**Criteria closed.** §11 criterion 1 (one `CASES` list, a second binding, **no case added**);
criterion 5 on a real transport; criterion 6 (`fs/write_text_file` → one `edit_proposal` whose diff
applies to an empty file); criterion 9's first half and criterion 11's process-group half, both by
live run on this box; criterion 13 re-checked (`cargo tree -i tokio` still shows no SDK edge).
Criteria 2, 3, 4 and 12 are milestone 1's and were re-run unchanged.

**§11.14 items closed by the live captures.** The model config option is keyed `model` and its value
vocabulary is per installation (`default`, `opus[1m]`, `sonnet`, `sonnet[1m]`, `haiku` on this box),
which is why the row stores an option **id**; the Claude rate-limit blob is real and arrives under
`_meta["_claude/rateLimit"]`, on a **later** `usage_update` than the first, and `cost` appears only
once the turn has produced output. The remaining nine items belong to milestones 5–8 and MOD-11.

**Two case scripts were amended** (blueprint H-2), each because ANA-4 says the value is not the
transport's to produce: ACP reports no per-turn tokens on `usage_update` (§7, confirmed by the
capture) and carries no verbatim diff (§4.3). `CASES` and its length did not change.

**Review gate.** `rust-reviewer` (`claude-fable-5-1`, one pass over the whole change set) **blocked**
on one CRITICAL and seven HIGH findings; all are fixed in `682a423`, with eleven MEDIUMs. The
CRITICAL was real and no test in this milestone would have caught it: `connect_with` drops the
foreground future when a connection actor fails first, so a JSON-RPC error answering
`session/prompt` orphaned the agent process. Three more were genuine defects — a stream ending
before its `done` recorded as a finished turn, a path guard that admitted everything when a root was
relative, and `edit_proposal.accepted` defaulting to `true` before the user had seen the request
(the `claude` adapter sends the diff *before* the permission request, so that default was the
common case, not the rare one). Nothing was deferred.

**Carried into milestone 4 and beyond.** Windows is unverified here (job object, `PATHEXT`,
`CREATE_NO_WINDOW`); `_always` permission grants are recorded but not persisted to
`agent.settings.remembered[]`; `retain_raw` defaults to `false` behind `HTUI_KEEP_RAW_EVENTS` until
MOD-15 gives `project.settings` a reader; a chat runs in the `htui` process's own working directory
until MOD-13 and MOD-7 give a project a repo path per box; a proposal whose rejection arrives after
its row was flushed keeps `accepted: null`, because the recorder has no update path for a flushed
row.

---
*Status: DONE — milestone 3 landed, reviewed and validated. The plan's open questions were all
answered at CONFIRM; the blueprint's nine corrections and the reviewer's nineteen findings are
applied.*
