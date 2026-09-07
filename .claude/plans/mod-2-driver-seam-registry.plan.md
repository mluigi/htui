# Plan: MOD-2 driver seam, registry and extensibility proof (milestones 1 and 2)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 1 (Driver seam + conformance) and 2 (Registry, launch and extensibility
proof). Two rows, one plan, because milestone 2's proof *is* milestone 1's `CASES` run through the
registry, and the store seam is opened once; each milestone is its own wave group below so the PRD
rows still tick individually. Milestones 3–9 are out of scope: where one of them needs a seam,
this plan names the seam and stops.
**Design authority**: `docs/ANA-4.md` §4.1 (trait + event model), §4.2 (SDK pin, MSRV), §4.3
(payload types only), §4.6 (launch recipe only, no probe), §5 (JSONB shapes, seed rows), §8 (crate
layout, dependency set, test strategy), §9 steps 1–2, §11 criteria 1–4 and 13; `docs/ANA-5.md` §8
(store-trait placement rule, the shared skill types); `docs/ANA-9.md` §4.3 (offline mint).
**Requirements**: `R-AGT-1`, `R-AGT-4`, `R-AGT-5`, `R-HIS-1` (recording half), `R-SEC-3` (seam +
minimal built-in), `R-TUI-8` (agent registry section, no quota yet); `R-NF-3` by construction.
**Complexity**: Large
**Routing**: PRD path (C2, C3, C4); ultracode recommended and accepted for implement and review
(PRD header). Reviewer: `rust-reviewer` (`.claude/workflow-config.json`). Models per maintainer:
plan and the single inline reviewer on Fable, every implementer on Opus; no wave below exceeds
four agents.

## Summary
Add a fourth crate, `htui-agent`, holding the `AgentDriver` / `AgentSession` seam of ANA-4 §4.1,
its event model, the recorder (coalescing, `seq`/`turn`, digest, scrub, persist), a `FakeDriver`
and a transport-neutral conformance suite with one `CASES` list. `htui-core` gains the five
`WriteStore` methods MOD-2 records through (including `start_chat_run`), an inherent `agents` read,
and the `Scrubber` seam with a fail-closed minimal implementation; `htui-store` gains their
`PgStore` bodies, `append_pending`, and the `agent` seed rows. Milestone 2 then lands the MSRV bump
and dependency set, the §5 launch/settings serde types with `${tool}` resolution and a job-object
spawn, a data-keyed `DriverFactory`, the Settings tab's agent section on a `SettingsRegistry`, and
the `R-AGT-5` test: an agent named nowhere in the tree reaches a working session and passes every
`CASES` entry from a registry row alone. Nothing speaks ACP or stream-json yet.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | New crate `crates/htui-agent` (lib), features `default = []`, `test-support = ["htui-core/test-support"]`. `htui` does **not** depend on it until milestone 3; milestone 2's Settings section renders store rows only. | ANA-4 §8 layout. Keeping `htui` off the crate in M2 makes T10 independent of T7/T9 and keeps the M2 binary free of the SDK until a transport exists to use it. |
| D2 | `AgentDriver` / `AgentSession` are the §4.1 text verbatim: hand-written `Pin<Box<dyn Future<Output = …> + Send + 'a>>` returns, `AgentDriver: Send + Sync + Debug`, `AgentSession: Send + Debug`, pull `next_event()`. No `async_trait`. `DriverEvent` has exactly the eleven §4.1 variants, so `From<&DriverEvent> for EventKind` is total over `EventKind` minus `prompt`, `follow_up`, `permission_answer` (`crates/htui-core/src/model/event.rs:12-41`). | §4.1 rejected AFIT as not dyn-compatible; the boxed shape is the adopted one and is what MOD-4 will hold as `Box<dyn AgentDriver>`. |
| D3 | **Every store-seam change MOD-2 needs before milestone 9 lands in milestone 1.** `WriteStore` gains `append_events`, `set_step_usage`, `upsert_agent`, `upsert_agent_box` (ANA-4 §4.1) and `start_chat_run` / `finish_chat_run` (PRD "Chat run ownership"); `PgStore`, `MemStore` and `Backend` gain an inherent `agents()` read. `conformance::CASES` grows 15 → 20 and both literal assertions move in the same commit. | `traits.rs:50` reserves exactly these areas. One Postgres run, one bump of `crates/htui-core/tests/mem_store.rs:24` and `crates/htui-store/tests/pg_conformance.rs:19`, and milestones 2–8 never touch `traits.rs` again. `agents` is inherent, not `ReadStore`, because `agent` is not mirrored (`docs/ANA-9.md:306-311`; no `agent` table in `crates/htui-store/cache_migrations/0001_mirror.sql`), following the `workspaces` / `box_info` / `active_runs` / `projects` precedent (`crates/htui-store/src/pg/read.rs:378-383`, `crates/htui-store/src/backend.rs:162-201`). |
| D4 | `start_chat_run(&self, chat: &ChatRunSpec) -> Result<()>` inserts `run` (`kind = 'chat'`, `mode = 'manual'`, `item_id NULL`, `status = 'running'`) and `run_step` (`position 0`, `attempt 1`, `fanout_index 0`, `phase_name = 'chat'`, `status = 'running'`), both `ON CONFLICT (id) DO NOTHING`. `ChatRunSpec::mint(project, target_box, started_by, agent, model)` lives in `htui-core::model::run` and mints `RunId::new()` / `StepId::new()` (UUIDv7, `crates/htui-core/src/model/ids.rs:31-32`). `finish_chat_run(&self, run: RunId, step: StepId, status: RunStatus, finished_at)` closes both rows (step status by the same name; `RunStatus` / `StepStatus` share `done|failed|cancelled`, `crates/htui-core/src/model/run.rs:31-43,57-69`). Owner: milestone 1, T2. | The offline path already inserts the same two rows from the file name (`crates/htui-store/src/cache/pending.rs:204-246`); one mint function and matching `ON CONFLICT (id)` clauses make online-then-offline and offline-then-upload converge on identical rows (ANA-9 §4.3). MOD-4's `claim_run` is graph-only (`docs/ANA-2.md:1740`). `finish_chat_run` is included because `active_runs` (`backend.rs:188`) would otherwise count a finished chat forever — assumption A1. |
| D5 | `Scrubber` lives in `htui-core::scrub` (new `crates/htui-core/src/scrub.rs`): `pub trait Scrubber: Send + Sync + Debug { fn scrub(&self, value: &mut serde_json::Value) -> Result<(), Unmasked>; }`, `Unmasked { path: String, rule: &'static str }` (never the value). `MinimalScrubber::new(secrets: impl IntoIterator<Item = String>)` replaces every exact occurrence of every non-empty secret in every string leaf with `[REDACTED]`, then scans every string leaf for the credential prefixes `sk-ant-`, `sk-`, `ghp_`, `github_pat_`, `AKIA`, `xoxb-`, `xoxp-`, `AIza`, `-----BEGIN ` … `PRIVATE KEY-----`; any surviving match is `Err(Unmasked)`. **Fail-closed**: the recorder never persists a row that returned `Err`. | ANA-4 §9's specified shape. Core rather than `htui-agent` so milestone 9's digest path (ANA-5 §4.7 "scrub before digest") and MOD-10's replacement share one trait without `htui-core` depending on the driver crate. MOD-10 replaces the impl behind the unchanged trait; ANA-7's design is not anticipated here (the prefix list is the PRD's "known credential prefixes", nothing more). |
| D6 | Recorder (`crates/htui-agent/src/record.rs`): flush triggers in §4.1 order (variant change, `message_id` change, `Done`, 16 KiB, session end; no idle flush); `seq` 0-based per persisted row, `turn` 0 at the `prompt` row and +1 per `follow_up`; `prompt_digest` = `sha2` over the prompt text, written into the `prompt` payload and through `set_step_usage(step, usage, Some(digest))`; `raw` carried only when `SessionSpec.retain_raw`; order is scrub → persist → `try_send` to an optional bounded UI channel, counting drops. On `Unmasked` the recorder drops the row, appends `error { code: "scrub_residue", message: "<rule> at <path>" }` (role `htui`), returns `RecordError::Unmasked` from `finish()`, and touches nothing else — marking the step failed belongs to the step's owner (`finish_chat_run(Failed)` in M3, MOD-4 for graph steps). | §4.1 "Chunk coalescing", "Turn counting and seq", "Persistence and the UI, in that order". `R-SEC-3` "blocks persistence" is satisfied inside the recorder; "marks the step failed" is the caller's status write, which M1 has no session to attach to. |
| D7 | Conformance (`crates/htui-agent/src/conformance.rs`, feature `test-support`): a transport-neutral `Script { turns: Vec<Turn> }` (each `Turn` a list of `ScriptEvent`s: driver events plus `ParkPermission(id)` / `ExpectCancel` markers), a `CaseHarness` trait (`fn driver(&self, script: Script) -> Box<dyn AgentDriver>`), `pub const CASES: &[&str]`, `run_case<H: CaseHarness, S: WriteStore>(name, harness, store)` and `run_all`. Every case builds a `SessionSpec`, starts the session, pumps it through the recorder into the store, and asserts on the rows read back via `ReadStore::step_events`. `CASES.len()` is asserted in `crates/htui-agent/tests/fake_conformance.rs` (the only binding in M1; M3's ACP binding is the second). | ANA-4 §8 test strategy 2 and §11 criterion 1: one list, transports differ only in how the harness turns the script into wire traffic. Asserting on persisted rows is what makes criteria 2–4 statements about the store rather than about a `Vec` in memory. |
| D8 | `append_pending(dir, project: ProjectId, run: RunId, events: &[SessionEvent]) -> Result<usize>` next to `upload_pending` in `crates/htui-store/src/cache/pending.rs`, appending one serde line per event to `<dir>/pending/<project_id>.<run_id>.jsonl`, creating the file on first write. It trusts its input: scrubbing is the recorder's job and the doc comment says so. | ANA-4 §4.1 "the appender is a new `pub async fn append_pending` … so the naming contract has exactly one owner"; the two-part name is the tree's (`pending.rs:5`), not ANA-9's. The offline *session* path (recorder choosing file over store) is milestone 4; only the primitive lands here. |
| D9 | The `Skill`, `SkillVersion`, `SkillBinding`, `BoundSkill` types of ANA-5 §8 **defer to milestone 9**. If MOD-9 lands before milestone 9, MOD-9 owns them and milestone 9 consumes (ANA-5's "whichever lands first" rule). | No reader exists in milestones 1–2: `BoundSkill` is a `PromptSpec` field and nothing else. Landing them now would repeat the `#[expect(dead_code)]` pattern of `crates/htui-core/src/store/mem.rs:62-66` for a MOD-9 contract with no test to pin it, and would put ANA-5 types under this plan's review with no behaviour to review. |
| D10 | Milestone 2, T7: `workspace.package.rust-version` `1.85` → **`1.98`** (`Cargo.toml:7`), matching the exact toolchain pin `1.98.1` (`rust-toolchain.toml`). **Maintainer override of ANA-4 §4.2's 1.88, given at CONFIRM on 2026-09-06 ("about the msrv consider the latest one"); see disagreement X9.**; `[workspace.dependencies]` gains `agent-client-protocol = "=2.1.0"` (no features), `tokio-util = { version = "0.7.19", features = ["compat"] }`, `process-wrap = { version = "10.0.0", features = ["tokio1", "creation-flags", "job-object"] }`, `similar = "3.2.0"`, `which = "8.0.6"`; `tokio`'s workspace feature list (`Cargo.toml:17`) gains `process` and `io-util`. `htui-agent` pulls `agent-client-protocol`, `tokio-util`, `process-wrap`, `which` (each with a T7 consumer); `similar` is workspace-declared only until M3's diff synthesis (assumption A3). No existing crate's `[dependencies]` changes. | ANA-4 §8 table and §4.2, with the MSRV number overridden. The declared 1.85 is fiction and 1.88 would still have been: locked `sqlx-core 0.9.0` declares `rust-version = "1.94.0"` and `ratatui 0.30.2` declares `1.88.0` (`cargo metadata`, V9), so nothing below 1.94 can build this workspace today. 1.98 is the only number that is simultaneously true, checkable and equal to what CI and every developer actually runs, since the toolchain is exact-pinned rather than floating. The bump is a declaration change, not a build break. Criterion 13's `cargo tree -i tokio` assertion is a Validation step because it needs the SDK in the graph, which T7 puts there. |
| D11 | Launch (`crates/htui-agent/src/launch.rs`): `AgentLaunch { command, args, env, discovery: Option<Discovery> }`, `Discovery { tools: BTreeMap<String, ToolProbe>, handshake: bool }`, `ToolProbe` (`path` / `node_package` / `glob` tagged enum), `AgentSettings` with every §5.2 key optional and defaulted, all `Deserialize + Serialize`; `AgentLaunch` and `SessionSpec` carry a hand-written `Debug` printing env values as `[REDACTED]`. `resolve(&AgentLaunch, &ToolMap) -> Result<ResolvedLaunch, DriverError::Unresolved(name)>` substitutes `${name}` in `command`, every `args[i]` and every `env` value from a caller-supplied `ToolMap` (`BTreeMap<String, String>`); a row with no placeholders resolves against an empty map. `ResolvedLaunch::to_acp_config()` builds the SDK's `AcpAgentConfig` through its builder. `spawn(&ResolvedLaunch, cwd) -> Result<Spawned, DriverError>`: `which` resolves the command (`PATHEXT`-aware), `process-wrap` `TokioCommandWrap` with `CreationFlags(CREATE_NO_WINDOW)` + `JobObject` on Windows, `ProcessGroup` on unix, piped stdio exposed through `tokio_util::compat`, stderr captured to a bounded tail; job-object assignment failure downgrades to a plain spawn with `Spawned.job_object = false` and a `warn!`. **No probe**: `ToolMap` is an input; milestone 5's `probe.rs` is what fills it from `agent_box.probe.tools`. | §5.1 shapes, §4.6's "Windows shim handling, as a rule", §4.2 "Process supervision". `AcpAgentConfig`'s fields are private with a `new(command)` + `arg(..)` builder (registry `agent-client-protocol-2.1.0/src/acp_agent.rs:53-75`), so the SDK type is reached through the builder, not by deserialising the row into it (ANA-4 §5.1 says "deserializes … with no adapter layer"; see disagreement X6). |
| D12 | Registry factory (`crates/htui-agent/src/registry.rs`): `DriverFactory { adapters: BTreeMap<String, Box<dyn TransportBuilder>> }`, keyed by an **adapter id** derived from row data only: `"acp"` for `transport = 'acp'`, `"cli/<settings.cli.stream>"` for `transport = 'cli'`. `driver_for(&self, agent: &Agent, on_box: Option<&AgentBox>) -> Result<Box<dyn AgentDriver>, DriverError::UnknownAdapter(id)>`. `DriverCaps` are computed from the row (`acp` → per `settings.acp`; `cli` → `permission_requests`, `edit_proposals`, `plans` false per §4.3). The factory has no method, match arm or map keyed by `agent.name`. Production registers `"acp"` in M3 and `"cli/claude_stream_json"` in M8; `test-support` registers `"cli/fake"`, whose builder is `FakeAdapter { script: Arc<Mutex<Option<Script>>> }` (the harness loads the slot, `driver_for` drains it). | `R-AGT-5` "a registry row and, at most, one stream adapter" is exactly one map entry per adapter and zero per agent. The fake reaching the factory through the same `cli.stream` knob a future adapter would use is what makes the M2 proof structural rather than decorative (assumption A2: `"fake"` extends §5.2's `cli.stream` vocabulary under `test-support` only). |
| D13 | Seed rows: one source, `crates/htui-core/seeds/agent_claude.json` and `agent_agy.json`, ANA-4 §5.3 byte for byte, exposed as `htui_core::model::agent::seed_rows(now) -> Vec<Agent>` (fresh `AgentId::new()`, `enabled = true`). `PgStore::seed_if_empty_as` inserts them when `agent` is empty (amending MOD-6 D5, which the MOD-6 close-out already assigned to MOD-2: `docs/decisions/mod/mod-6.md:85-86,117`). `fixtures::agents()` (`crates/htui-core/src/fixtures.rs:383-412`) becomes `seed_rows()` re-stamped with `ids::AGENT_CLAUDE` / `ids::AGENT_AGY` and `epoch()`, which corrects `claude`'s `["claude","--acp"]` argv and `agy`'s `cli` / `per_token` / `["agy","run"]` (`fixtures.rs:390,402-406`). The `agent.launch` doc comment `{argv, env}` (`crates/htui-core/src/model/agent.rs:38`) is rewritten to the §5.1 shape. | ANA-4 §5.3's three notes; "a fixture that cannot launch is a trap for MOD-2's own tests". The demo `sections[]` vocabulary (`fixtures.rs:1244-1247`) is ANA-5's and stays until milestone 9. |
| D14 | Settings tab: `crates/htui/src/ui/tabs/settings.rs` becomes `settings/mod.rs` (the tab, a `SettingsSection` trait and a `SettingsRegistry` mirroring `DetailTab` / `DetailRegistry`, `crates/htui/src/ui/tabs/backlog/detail/mod.rs:55-80`) plus `settings/agents.rs` (`AgentsSection`). `register_all` (`crates/htui/src/app/mod.rs:40-59`) builds `SettingsTab::with_sections(vec![Box::new(AgentsSection::new())])`. `StoreRequest::Agents` / `StoreReply::Agents(Vec<AgentSummary>)` plus one `name()` arm and one `try_serve` arm (`crates/htui/src/store_worker.rs:45-100,191-212`). `AgentSummary = { agent: Agent, on_box: Option<AgentBox> }` for the store's own box. `Backend::agents()` on `Offline` returns `StoreError::Unreachable("agent registry is not mirrored")`; the section renders one line, "agent registry needs Postgres". | ANA-4 §8 "a SettingsRegistry mirroring DetailRegistry, so MOD-2's agent section and MOD-15's hierarchy section coexist"; `R-TUI-8`. `Unreachable` from an already-`Offline` backend is harmless: `go_offline` returns early when `went_offline()` is false (`store_worker.rs:328-330,440-442`). No caps banner, no quota, no probe column: those are M3, M7, M5. |
| D15 | Open-question rulings. (a) `conformance::run_case` over `ReadStore`: **defers to milestone 9**; M1 adds no `ReadStore` method, so the `WriteStore`-only suite (`crates/htui-core/src/store/conformance.rs:1-7,47`) is unchanged in shape here and the 15 → 20 bump is the only edit. (b) `set_step_usage`'s `prompt_digest` parameter **survives through milestone 8**: the recorder is its only writer for chat steps (D6) and `set_step_prompt` does not exist until M9; M9 decides whether the chat path moves to `set_step_prompt(step, digest, Null)` and drops the parameter. | Neither question has a consumer inside these milestones; both are recorded so M9's plan inherits a decision, not a question. |
| D16 | Deliberately absent from M1–M2: any wire protocol (`acp/`, `cli/` are M3, M8), `probe.rs` and migration `0002` (M5), the chat tab and `StepEvents` replay (M3–M4), quota (M7), the offline session path (M4), `similar` in any crate (M3), `htui` → `htui-agent` (M3). `SessionSpec.tools: ToolExposure` and `mcp: Vec<McpServerSpec>` are plain structs with no reader (MOD-11 fills them). | Scope discipline per the PRD; each is a named seam, not a plan. |

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-store/src/{backend,connect,identity,secret}.rs`, `crates/htui-core/src/store/{traits,mem,conformance}.rs` | snake_case modules, one public type per file, `mod.rs` re-exports, column names verbatim on every persisted struct (`crates/htui-core/src/lib.rs:3-6`). `#![warn(missing_docs)]` on every lib (`htui-core/src/lib.rs:9`). |
| Errors | `crates/htui-core/src/store/error.rs:9-31` | One `thiserror` enum per crate; `DriverError` mirrors it (`Unresolved(String)`, `UnknownAdapter(String)`, `Spawn(String)`, `Transport(String)`, `Closed`, `Store(StoreError)`, `Scrub(Unmasked)`). |
| Logging | `crates/htui/src/lib.rs` (`init_tracing`), `crates/htui-store/src/cache/pending.rs:78-83` | `tracing` to a file under `HTUI_LOG`, never stdout; `warn!` with structured fields for a degraded path (job object refused, pending file left in place). |
| Data access | `crates/htui-store/src/pg/write.rs:27-44`, `crates/htui-core/src/store/mem.rs:697-747` | `WriteStore` bodies as single statements with `RETURNING`; `MemStore` locks, clones, drops, never across an `.await`. `ON CONFLICT (id) DO NOTHING` exactly as `pending.rs:204-246`. |
| Tests | `crates/htui-core/src/store/conformance.rs:20-86`, `crates/htui-store/tests/pg_conformance.rs:35-42`, `crates/htui/tests/backlog.rs:77`, `crates/htui/src/testkit.rs:63-70` | Named `CASES` + `run_case` dispatch + a loop that reports per case; explicit-name `insta::assert_snapshot!`; `Harness::demo()` / `empty()` with `settle()` inline. |
| Registries | `crates/htui/src/ui/tabs/registry.rs:33-49`, `backlog/detail/mod.rs:55-80`, `app/mod.rs:40-59` | Trait object + `Vec<Box<dyn …>>` + `register`; registration in one place; no `match` arm in the loop. |
| Worker seam | `crates/htui/src/store_worker.rs:41-43,45-100,178-212` | New request = one variant, one `name()` arm, one `try_serve` arm; `event_loop.rs` unchanged. |

## Files to Change

Task file sets are the independence facts for §3.5; disjointness within each wave is verified in
V11. `.sqlx/query-<hash>.json` files are per query and additive (MOD-6 V6 precedent).

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` (workspace) | UPDATE | T1 | member `crates/htui-agent`, workspace dep `htui-agent` |
| `crates/htui-agent/Cargo.toml`, `src/lib.rs`, `src/driver.rs`, `src/event.rs`, `src/error.rs` | CREATE | T1 | crate, §4.1 traits, `SessionSpec`, `DriverCaps`, `DriverEnvelope`, `DriverEvent` + payload structs, `PermissionPolicy` (serde), `DriverError` |
| `crates/htui-agent/tests/driver_contract.rs` | CREATE | T1 | dyn-compatibility, `From<&DriverEvent> for EventKind` totality, redacted `Debug` |
| `crates/htui-core/src/store/traits.rs`, `store/mem.rs`, `store/conformance.rs` | UPDATE | T2 | D3 methods, `MemStore` bodies (the `agents` `#[expect(dead_code)]` at `mem.rs:65` goes), five new cases |
| `crates/htui-core/src/model/run.rs`, `model/agent.rs`, `model/mod.rs` | UPDATE | T2 | `ChatRunSpec` (+ `mint`), `AgentSummary`, re-exports |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | T2 | `15` → `20` (`:24`) |
| `crates/htui-store/src/pg/write.rs`, `src/pg/read.rs`, `src/backend.rs` | UPDATE | T2 | `PgStore` bodies, inherent `agents`, `Backend::agents` over three arms |
| `crates/htui-store/tests/pg_conformance.rs`, `tests/pg_criteria.rs`, `.sqlx/query-*.json` | UPDATE / CREATE | T2 | `EXPECTED_CASES` `15` → `20` (`:19`); inherent read case; offline data |
| `crates/htui-core/src/scrub.rs`, `src/lib.rs` | CREATE / UPDATE | T3 | D5 trait, `Unmasked`, `MinimalScrubber`; `pub mod scrub` |
| `crates/htui-agent/src/record.rs`, `src/lib.rs`, `tests/recorder.rs` | CREATE / UPDATE | T4 | D6 recorder and its unit tests over `MemStore` |
| `crates/htui-store/src/cache/pending.rs`, `tests/cache.rs` | UPDATE | T5 | `append_pending`; round trip through `upload_pending` |
| `crates/htui-agent/src/fake.rs`, `src/conformance.rs`, `src/lib.rs`, `tests/fake_conformance.rs` | CREATE / UPDATE | T6 | `FakeDriver`, `Script`, `CaseHarness`, `CASES`, the fake binding with the `len()` assertion |
| `Cargo.toml` (workspace), `crates/htui-agent/Cargo.toml`, `README.md` | UPDATE | T7 | D10 MSRV + dependency set; README MSRV and crate notes |
| `crates/htui-agent/src/launch.rs`, `src/lib.rs`, `tests/launch.rs` | CREATE / UPDATE | T7 | D11 serde types, `resolve`, `to_acp_config`, `spawn` |
| `crates/htui-core/seeds/agent_claude.json`, `seeds/agent_agy.json`, `src/model/agent.rs`, `src/fixtures.rs` | CREATE / UPDATE | T8 | D13 seed source, `seed_rows()`, fixture correction, launch doc comment |
| `crates/htui-store/src/pg/mod.rs`, `tests/migrations.rs`, `.sqlx/query-*.json` | UPDATE / CREATE | T8 | seed insert, seed test |
| `crates/htui-agent/src/registry.rs`, `src/fake.rs`, `src/lib.rs`, `tests/extensibility.rs` | CREATE / UPDATE | T9 | D12 factory, `FakeAdapter`, the `R-AGT-5` proof |
| `crates/htui/src/ui/tabs/settings.rs` → `settings/mod.rs`, `settings/agents.rs` | DELETE / CREATE | T10 | D14 registry and agent section |
| `crates/htui/src/store_worker.rs`, `src/app/mod.rs`, `src/testkit.rs` | UPDATE | T10 | `Agents` request/reply; section registration; `Harness::over_store(MemStore)` made public for the `zeta` snapshot |
| `crates/htui/tests/settings.rs` + `tests/snapshots/settings__*.snap` | CREATE | T10 | section snapshots |

## Tasks

Waves. **Milestone 1**: A = {T1, T2, T3, T5} **parallel** (disjoint file sets, V11; none needs
another's output — T2 is core/store only, T3 is a standalone core module, T5 is a file writer);
B = {T4} serial (needs T1's types, T2's methods, T3's trait); C = {T6} serial (needs T4).

> **Execution amendment, 2026-09-06 (main thread, at launch).** Wave A ran as {T1, T3, T5} with
> **T2 held**, for two reasons the plan's file-set analysis does not capture:
>
> 1. **Disjoint files are not disjoint compilation units.** T2 edits `htui-core/src/store/*` and
>    `htui-store/src/pg/*`; T3 edits `htui-core/src/scrub.rs`; T5 edits
>    `htui-store/src/cache/pending.rs`. The file sets are genuinely disjoint (V11 stands), but all
>    four agents run `cargo` against the same two crates in one tree, so one agent's half-written
>    module fails another's build. Running T2 — the widest task, spanning both crates — alone
>    removes the race. V11's independence verdict is unchanged; this is a build-concurrency
>    constraint, not a file conflict.
> 2. **The dev Postgres is down** (port 5433 refused, Docker daemon not running). T2's `PgStore`
>    half adds new `sqlx::query!` calls, which need a live database both to compile and to
>    `cargo sqlx prepare`. T2 cannot start until it is up; T4 and T6 sit behind T2.
>
> Revised order: A = {T1, T3, T5} → B = {T2} → C = {T4} → D = {T6}. Still six agents for
> milestone 1, still one reviewer pass at the end. Milestone 2's waves are unchanged on paper, but
> T8 needs Postgres for the same reason and will be sequenced the same way.

> **Defect in this plan's independence method, found by T1 during wave A (2026-09-06).**
> `DriverError::Scrub(Unmasked)` (T1, `crates/htui-agent/src/error.rs`) compile-depends on
> `htui_core::scrub::Unmasked`, which does not exist until T3 adds `pub mod scrub;`. The two file
> sets are disjoint and V11 is correct, yet T1 cannot *compile* before T3 has landed. Wave A only
> succeeded because T3 reached `lib.rs` first.
>
> **The method, not just this instance, was wrong.** §3.5 decides independence by intersecting
> touched-file sets. That is necessary but not sufficient: a task can depend on a *type* another
> task introduces without sharing a single file. File-set disjointness proves two agents will not
> overwrite each other; it does not prove either one can build.
>
> Correct marking: `T1 serial-after T3`, or the `Scrub` variant deferred to T4's `RecordError`.
> For the remaining waves, a task is independent only when its file set is disjoint **and** every
> type, trait and function it names already exists in the tree or is created by the same task.
> Re-checked under that stricter rule: T7 ∥ T8 ∥ T10 still holds (T7 names only `htui-agent` and
> SDK types, T8 only `htui-core` model types, T10 only T2's already-landed store methods), and
> T9's `serial-after: T6, T7, T8` was already correct.
**Milestone 2**: D = {T7, T8, T10} **parallel** (disjoint, V11; T10 needs only M1's T2);
E = {T9} serial (needs T7's `launch`, T8's `seed_rows`, T6's fake). Four agents at most per wave,
Opus, worktrees; the reviewer runs once per milestone. TDD per task: the tests named under
**Tests first** are written and failing before the implementation.

> **Milestone 2 execution amendment, 2026-09-07 (main thread, at launch).** Four corrections; the
> first three were accepted by the maintainer at the routing gate, the fourth is their instruction
> at launch.
>
> 1. **Dev Postgres moved from port 5433 to 5439** (`compose.yaml`, `README.md`, and `HANDOFF.md`'s
>    live coordinates). Every `HTUI_TEST_DATABASE_URL` in this plan reads
>    `postgres://postgres:htui@localhost:5439/postgres`; the Validation block and the Prerequisite
>    line are corrected in place. The wave-A note's "port 5433 refused" stays as written — it is a
>    record of what happened on 2026-09-06, not an instruction.
> 2. **T10 is `serial-after: T8`, not independent — the same defect the wave-A note names.**
>    File sets stay disjoint (V11 holds), but T10's `settings_agents_demo` snapshot renders
>    `transport` and `billing` for the demo agents, and T8 rewrites exactly those fixture values
>    (`agy`: `cli`/`per_token` → `acp`/`subscription`, `models` → `[]`, `default_model` → `None`,
>    per ANA-4 §5.3 and `crates/htui-core/src/fixtures.rs:383-412`). A snapshot written against the
>    pre-T8 fixture is stale the moment T8 lands. This is a *data* dependency with no shared file:
>    the stricter rule needs one more clause — a task is independent only when its file set is
>    disjoint **and** every type, trait, function **and fixture value** it reads already exists in
>    its final form or is produced by the same task.
> 3. **No Workflow tool is available in this session**, so the accepted ultracode recommendation
>    could not run as a script.
> 4. **The implementer fan-out was declined by the maintainer at launch**; milestone 2 runs
>    serially on the main thread, T7 → T8 → T10 → T9. TDD, the `rust-reviewer` gate and the
>    close-out validator are unchanged — the fan-out was the optimization, never the contract.
>    Worktrees are dropped with it (they existed to isolate parallel agents).
>
> **Every Postgres run in this wave is prefixed `USERNAME=htui-ci`** (TOOL-2): the demo fixture
> seeds `app_user.name = "luigi"` and `PgStore::seed_if_empty` derives the same name from the OS
> user, so `demo_db()` fails with a duplicate-key `Constraint` on this box. The suites' skip guard
> returns `ok` when `HTUI_TEST_DATABASE_URL` is unset, so a task reporting "green" without the
> variable set has proved nothing — T8's validate step must quote real assertions, never a skip.

Every implementer prompt carries: ANA-4 has priority over this plan where they disagree (MOD-1 /
MOD-6 precedent) except where §"Where the ANA docs and the tree disagree" below rules otherwise;
graphify-first for codebase questions (`graphify-out/` exists); `.sqlx` regenerated and committed
with any `PgStore` query change; no lock or pool connection across a UI await; no `unsafe`
(`Cargo.toml:35`).

### Task 1: `htui-agent` crate skeleton, driver seam, event model — `independent`
- **Files**: `Cargo.toml` (workspace); `crates/htui-agent/Cargo.toml`, `src/lib.rs`, `src/driver.rs`, `src/event.rs`, `src/error.rs`, `tests/driver_contract.rs`.
- **Tests first** (`tests/driver_contract.rs`): `fn takes_dyn(_: &dyn AgentDriver, _: &mut dyn AgentSession)` compiles and a `tokio::spawn` over a `Box<dyn AgentSession>::next_event()` type-checks (`Send`); every `DriverEvent` variant maps through `From<&DriverEvent> for EventKind` to a value other than `Prompt` / `FollowUp` / `PermissionAnswer`, and the fourteen `EventKind` values minus those three are all reached (11 = 11); `format!("{:?}", SessionSpec { env: {"TOKEN": "s3cr3t"}, .. })` contains `[REDACTED]` and not `s3cr3t`; `DriverCaps` is `Copy` and `Default` is all-false.
- **Action**: crate per D1 (deps: `htui-core`, `tokio` `sync,rt,time`, `serde`, `serde_json`, `chrono`, `uuid`, `thiserror`, `sha2`, `tracing`; dev: `tokio` `macros,rt,test-util`, `insta`). `driver.rs`: §4.1 `SessionSpec`, `DriverCaps`, `AgentDriver`, `AgentSession`, `AgentSessionRef(String)`, `PermissionRequestId(String)`, `PermissionAnswer { Selected(String), Cancelled }`, `ToolExposure`, `McpServerSpec`, `PermissionPolicy` (§5.2 `permission` block, serde, `Default = ask`). `event.rs`: `DriverEnvelope`, `DriverEvent` (eleven variants), `TextChunk`, `ToolCallEvent`, `ToolResultEvent`, `EditProposalEvent`, `PermissionRequestEvent`, `PlanEvent`, `UsageEvent` (ANA-9 §4.3 fields, all nullable), `ErrorEvent`, `DoneEvent { stop_reason }` (five values), `OtherEvent { update, body }`, `ToolKind` (ten values incl. `switch_mode`), and the total `From`. `error.rs`: `DriverError`.
- **Mirror**: `docs/ANA-4.md` §4.1 verbatim; `crates/htui-core/src/store/error.rs`.
- **Validate**: `cargo build -p htui-agent`, `cargo clippy -p htui-agent --all-targets -- -D warnings`, `cargo test -p htui-agent --test driver_contract`, `cargo doc -p htui-agent --no-deps`.

### Task 2: store seam — five `WriteStore` methods, `agents` read, `MemStore` + `PgStore`, `CASES` 15 → 20 — `independent`
- **Files**: `crates/htui-core/src/store/traits.rs`, `store/mem.rs`, `store/conformance.rs`, `model/run.rs`, `model/agent.rs`, `model/mod.rs`, `tests/mem_store.rs`; `crates/htui-store/src/pg/write.rs`, `src/pg/read.rs`, `src/backend.rs`, `tests/pg_conformance.rs`, `tests/pg_criteria.rs`, `.sqlx/query-*.json`.
- **Tests first**: five `conformance.rs` cases, each asserting through `WriteStore` / `ReadStore` only: `append_events_idempotent_and_ordered` (append 5 rows, returns 5; append the same 5 plus 2 new, returns 2; `step_events` is `seq` 0..7; an unknown `run_step_id` is `Constraint`); `set_step_usage_writes_usage_and_digest` (`Some(digest)` sets `run_step.prompt_digest`, `None` leaves it; unknown step is `NotFound`); `start_chat_run_mints_chat_rows` (`ChatRunSpec::mint` → `start_chat_run` → `active_runs(scope)` grows by one, `runs(item)` of every item is unchanged; a second `start_chat_run` with the same spec is a no-op; `finish_chat_run(Done)` brings `active_runs` back); `upsert_agent_by_id_name_unique` (insert, update in place, a second id with an existing `name` is `Constraint`); `upsert_agent_box_by_pk` (insert then update on `(agent_id, box_id)`). `tests/mem_store.rs:24` and `tests/pg_conformance.rs:19` read `20`. `pg_criteria.rs`: `inherent_reads_answer_the_fixture` gains `agents()` = two summaries with `on_box == None`.
- **Action**: `traits.rs` per D3/D4 (doc comment names ANA-4 §4.1 and this plan); `model/run.rs` `ChatRunSpec { run_id, step_id, project_id, target_box_id, started_by, agent_id, model, started_at }` + `mint`; `model/agent.rs` `AgentSummary`; `MemStore` bodies (events keyed `(StepId, seq)`, PK backstop = skip on duplicate; `runs`/`steps` maps gain chat rows; `agents` map read, `agent_boxes` map new); `PgStore` bodies with `query!` (`append_events` as one multi-row `INSERT … ON CONFLICT (run_step_id, seq) DO NOTHING` returning the count; `start_chat_run` the two statements of `pending.rs:204-246` with `status = 'running'` and `finished_at NULL` in one transaction); `Backend::agents` dispatch (`Memory` → `MemStore`, `Online` → `PgStore`, `Offline` → `Unreachable`, D14). Remove the `#[expect(dead_code)]` on `State.agents` (`mem.rs:65`).
- **Mirror**: `crates/htui-store/src/pg/write.rs:27-44` single-statement style; `pending.rs:204-246` row values; `backend.rs:162-201` dispatch.
- **Validate**: `cargo test -p htui-core --features test-support`; `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --features demo --test pg_conformance --test pg_criteria`; `cd crates/htui-store && DATABASE_URL=… cargo sqlx prepare` then `cargo clippy -p htui-store --all-targets -- -D warnings` with `SQLX_OFFLINE=true`.

### Task 3: `Scrubber` seam + `MinimalScrubber` — `independent`
- **Files**: `crates/htui-core/src/scrub.rs`, `crates/htui-core/src/lib.rs`.
- **Tests first** (`scrub.rs` unit tests): every env value `{"A": "alpha-secret", "B": "1234"}` is replaced in nested strings and inside JSON string values that embed it mid-sentence; a value absent from the mask list but starting with `sk-ant-` / `ghp_` / a PEM header is `Err(Unmasked { rule, path })` and `path` is the JSON pointer (`/payload/output`); the `Unmasked` `Display` and `Debug` never contain the offending text; a payload with no strings is `Ok`; an empty secret list masks nothing and still fails closed on a prefix; scrubbing is idempotent (`scrub` twice = once).
- **Action**: D5. `Scrubber`, `Unmasked`, `MinimalScrubber` (secrets sorted longest-first so a value that is a prefix of another cannot leave residue; prefix rules as `&[(&'static str, &'static str)]` name/prefix pairs, PEM as a `-----BEGIN ` + `PRIVATE KEY-----` pair check). `pub mod scrub` in `lib.rs`.
- **Mirror**: `crates/htui-core/src/store/error.rs` for the error shape; `str_enum!`-style doc discipline (`model/mod.rs:25`).
- **Validate**: `cargo test -p htui-core scrub`, `cargo clippy -p htui-core --all-targets -- -D warnings`.

### Task 4: recorder — `serial-after: T1, T2, T3`
- **Files**: `crates/htui-agent/src/record.rs`, `src/lib.rs`, `tests/recorder.rs`.
- **Tests first** (`tests/recorder.rs`, over `MemStore::demo()` + `MinimalScrubber`, scripted `DriverEnvelope` vectors, no driver): twenty `AssistantChunk`s across two `message_id`s with one `ToolCall` in the middle persist as exactly three `assistant_text` rows and one `tool_call` in `seq` order, and replaying the same vector into a second store yields byte-equal rows (criterion 2); `record_prompt` + two `record_follow_up` + `finish` give gapless `seq`, `turn` `0,1,2`, `prompt` at `seq 0`, `turn 0`, role `htui`, payload `digest` == `run_step.prompt_digest` (criterion 3); `retain_raw = false` → every `raw IS NULL`, `true` → same rows with `raw` populated (criterion 4); a 17 KiB run of chunks flushes at the 16 KiB bound; an `Other` event lands as kind `other` with the verbatim body; `Usage` deltas sum into `run_step.usage`; an env value appearing in a `ToolResult.output` is `[REDACTED]` in the persisted row; an `sk-ant-…` string not in the env produces no persisted row for that event, one `error { code: "scrub_residue" }` row, and `finish()` returns `Err(RecordError::Unmasked)`; with a bounded UI channel of capacity 1, all rows persist and `dropped()` counts the rest.
- **Action**: D6. `Recorder::new(store: &S, scrubber: &dyn Scrubber, step: StepId, retain_raw: bool, ui: Option<mpsc::Sender<DriverEnvelope>>)`, `record_prompt(text, sections: Value)`, `record_follow_up`, `record(DriverEnvelope)`, `record_permission_answer(request_id, option_id, by, cancelled)`, `finish() -> Result<RecorderSummary, RecordError>`; `pump(session: &mut dyn AgentSession, recorder: &mut Recorder) -> Result<DoneEvent, DriverError>` drives one turn to `Done`. Digest with `sha2`. Edit-proposal dedup per `(tool_call_id, path)` (row updated in the buffer before flush, never written twice).
- **Mirror**: `docs/ANA-4.md` §4.1 "Chunk coalescing", "Turn counting and seq", "Persistence and the UI"; `crates/htui-core/src/store/mem.rs` lock discipline.
- **Validate**: `cargo test -p htui-agent --features test-support --test recorder`, clippy as T1.

### Task 5: `append_pending` — `independent`
- **Files**: `crates/htui-store/src/cache/pending.rs`, `crates/htui-store/tests/cache.rs`.
- **Tests first** (`tests/cache.rs`, temp cache dir, existing `pending_event` / `write_pending` helpers at `:116,:139`): `append_pending` of 20 events creates `pending/<project>.<run>.jsonl` with 20 lines in `seq` order; a second call with 10 more appends without rewriting the first 20; the file round-trips through `upload_pending` (`pending_upload_lands_ordered` at `:982` reused against a file the appender wrote — criterion 12's first half); running `upload_pending` twice is still idempotent (`:1045` pattern); an unwritable `pending/` parent is `StoreError::Backend`.
- **Action**: D8. `pub async fn append_pending(dir: &Path, project: ProjectId, run: RunId, events: &[SessionEvent]) -> Result<usize>` (blocking file I/O through `tokio::task::spawn_blocking`, `OpenOptions::append(true).create(true)`, one `serde_json::to_string` line per event, trailing newline). Module doc gains the "scrubbed by the recorder before it reaches this function" sentence.
- **Mirror**: `pending.rs:113-121` (`list`), `:124-180` (`parse`) for the name contract.
- **Validate**: `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --features demo --test cache`, clippy.

### Task 6: `FakeDriver`, `Script`, conformance `CASES` — `serial-after: T4`
- **Files**: `crates/htui-agent/src/fake.rs`, `src/conformance.rs`, `src/lib.rs`, `tests/fake_conformance.rs`.
- **Tests first** (`tests/fake_conformance.rs`): `assert_eq!(conformance::CASES.len(), 13, "…")`; `run_all(&FakeHarness, || MemStore::demo())`; `case_names_are_unique`; `run_case_accepts_every_name_in_cases` (mirrors `conformance.rs:951`).
- **Action**: D7. `fake.rs`: `FakeDriver::scripted(Script)`, `FakeSession` playing one `Turn` per prompt/follow-up, parking permission requests until `answer_permission` or `cancel` (which answers them `Cancelled` and synthesizes the failed `tool_result`), `session_ref()` = `Some("fake-<uuid>")`, `raw` populated iff `retain_raw`. `conformance.rs`: `Script`, `Turn`, `ScriptEvent`, `CaseHarness`, `CASES` in run order — `coalesce_across_message_id` (criterion 2), `seq_gapless_and_turns` (criterion 3), `raw_iff_retain` (criterion 4), `cancel_answers_parked_permissions` (criterion 5), `rejected_tool_gets_failed_result`, `done_precedes_next_follow_up`, `usage_deltas_sum_to_step_usage`, `unknown_update_lands_in_other`, `edit_proposal_deduped_per_call_and_path`, `chunk_flush_at_16kib`, `session_banner_is_first_other_row` (`session_started` with `session_ref`, the row ANA-2 §4.8 reads), `env_values_masked_in_rows`, `scrub_residue_refuses_write` — each a script plus assertions on `step_events` rows. `run_case` builds a `SessionSpec` with `env = {"FAKE_TOKEN": "fake-secret-…"}` so every case exercises the scrubber.
- **Mirror**: `crates/htui-core/src/store/conformance.rs:20-86` shape; `docs/ANA-4.md` §8 test strategy 1–2, §11 criteria 1–5.
- **Validate**: `cargo test -p htui-agent --features test-support`, clippy, `cargo doc`.

### Task 7: MSRV bump, dependency set, launch types, resolution, spawn — `independent` (milestone 2)
- **Files**: `Cargo.toml` (workspace); `crates/htui-agent/Cargo.toml`, `src/launch.rs`, `src/lib.rs`, `tests/launch.rs`; `README.md`.
- **Tests first** (`tests/launch.rs`): `AgentLaunch` round-trips the exact §5.1 JSON of both seed documents (parsed from string literals held in the test until T9 switches to `seed_rows()`), including a `glob` probe with per-platform `args`; `AgentSettings` from `{}` yields every documented default; `resolve` substitutes `${node}`, `${claude_agent_acp}` and an env `${claude}` from a `ToolMap`, errors `Unresolved("node")` on a missing key, and a placeholder-free row resolves against an empty map; `format!("{:?}", resolved)` hides env values; `to_acp_config()` builds a config whose serialised form equals `{command, args, env}`; `spawn` of `${cargo}` → `std::env::var("CARGO")` with `["--version"]` exits 0, stdout starts with `cargo `, stderr tail is empty, and on Windows `Spawned.job_object` is `true`.
- **Action**: D10 and D11. `Cargo.toml` edits (rust-version, five deps, tokio features); `htui-agent/Cargo.toml` adds `agent-client-protocol`, `tokio-util`, `process-wrap`, `which`, `tokio` `process,io-util`. `launch.rs` types, `resolve`, `ResolvedLaunch`, `to_acp_config`, `spawn` (`Spawned { child: Box<dyn TokioChildWrapper>, stdin, stdout (compat), stderr_tail: Arc<Mutex<VecDeque<String>>>, job_object: bool }`, `kill_tree()` for M3). README: MSRV 1.98 (the X9 override, with its one-line reason), `htui-agent` line, the `sqlx-core 1.94` observation recorded as ANA-4 §4.2 asks.
- **Mirror**: `docs/ANA-4.md` §5.1–5.2 shapes, §4.6 rule paragraph, §4.2 supervision paragraph; hand-written `Debug` per `crates/htui/src/ui/tabs/registry.rs:58-68`.
- **Validate**: `cargo build --workspace` with `rust-version = "1.98"` declared; `cargo tree -i tokio -e normal --workspace` shows `htui`, `htui-store`, `sqlx-core`, `tokio-stream`, `htui-agent`, `tokio-util`, `process-wrap` and **no** `agent-client-protocol` line (criterion 13; today's baseline is V10); `cargo test -p htui-agent --test launch`; full clippy.

### Task 8: seed rows, fixture correction, `PgStore` seed — `independent` (milestone 2)
- **Files**: `crates/htui-core/seeds/agent_claude.json`, `seeds/agent_agy.json`, `src/model/agent.rs`, `src/fixtures.rs`; `crates/htui-store/src/pg/mod.rs`, `tests/migrations.rs`, `.sqlx/query-*.json`.
- **Tests first**: `htui-core` unit test `seed_rows_match_ana4_5_3` (two rows, names `claude` / `agy`, both `transport: acp`, both `subscription`, `models == []`, `default_model == None`, `launch.command == "${node}"` / `"${agy_acp_server}"`, `settings.cli` present only on `claude`); `fixtures::agents()` equals `seed_rows()` re-stamped, and every existing `htui-core` / `htui` test still passes (the correction must not move a snapshot — `RunStepSummary` carries `agent_id`, not the row); `tests/migrations.rs`: after `seed_if_empty_as`, `agents()` returns exactly the two rows with `on_box == None`; a second seed inserts no third row; `load_demo` (`pg/demo.rs:137-139`) still inserts the fixture's agents unchanged.
- **Action**: D13. `crates/htui-core/seeds/*.json` (`include_str!`), `seed_rows(now: DateTime<Utc>)`, fixture rewrite, `agent.rs:38` doc comment; `pg/mod.rs` `seed_if_empty_as` inserts `seed_rows(now())` after the capability tags, and its doc paragraph at `pg/mod.rs:214-215` ("**No `agent` rows**") is replaced.
- **Mirror**: `docs/ANA-4.md` §5.3 byte for byte; `pg/mod.rs:239-` seed transaction and lock order.
- **Validate**: `cargo test -p htui-core --all-features`; `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --features demo --test migrations`; `cargo sqlx prepare` + clippy; `cargo test --workspace --all-features` (snapshots).

### Task 9: `DriverFactory` and the `R-AGT-5` proof — `serial-after: T6, T7, T8`
- **Files**: `crates/htui-agent/src/registry.rs`, `src/fake.rs`, `src/lib.rs`, `tests/extensibility.rs`.
- **Tests first** (`tests/extensibility.rs`, the milestone's acceptance test):
  1. `const ZETA: &str` — a complete `agent` row as JSON: `name "zeta"`, `transport "cli"`, `billing "per_token"`, `launch { command: "${zeta}", args: ["--serve"], env: { "ZETA_TOKEN": "${zeta_token}" }, discovery: { tools: { zeta: { kind: "path", names: ["zeta"] } } } }`, `settings { cli: { stream: "fake", permission_mode: "ask", extra_args: [] } }`. Parsed into `Agent` with `AgentId::new()`; no Rust constructor names it.
  2. **Absent from the codebase**: the test walks `crates/*/src/**/*.rs`, `crates/*/seeds/*.json` and `crates/*/tests/**/*.rs` minus itself (from `CARGO_MANIFEST_DIR/../..`) and asserts no file contains `zeta`. A hard-coded third agent anywhere fails here.
  3. **Row alone**: `store.upsert_agent(&zeta)`, then `driver_for` is called with the row **read back** through `store.agents()`, not the literal; `driver.name() == "zeta"`; `driver.caps()` is the `cli` profile (three `false`, `follow_up_in_session` true, `usage` per settings).
  4. **Data-keyed, not name-keyed**: `factory.adapter_ids() == ["cli/fake"]` (adding a `"zeta"` builder fails this); the same row with `settings.cli.stream = "nonesuch"` is `Err(UnknownAdapter("cli/nonesuch"))`; the `claude` seed row through the same factory is `Err(UnknownAdapter("acp"))` in M2 — the factory knows transports, never agents.
  5. **Launch from the row**: `resolve(&zeta.launch, &ToolMap { zeta: "<tmp>/zeta.exe", zeta_token: "zeta-secret-9f8e7d" })` yields the command, args and env; the `SessionSpec` built from it carries that env.
  6. **Working session**: `conformance::run_all` over a `CaseHarness` whose `driver(script)` loads the `FakeAdapter` slot and calls `factory.driver_for(&zeta_from_store, None)` — every one of the thirteen `CASES` passes with events recorded into `MemStore`, and a final sweep asserts `zeta-secret-9f8e7d` appears in no persisted row.
  7. `seed_rows()[..].launch` deserialise into `AgentLaunch` (the T7 literal test switches to this source).
- **Action**: D12. `registry.rs`: `TransportBuilder { fn build(&self, agent: &Agent, on_box: Option<&AgentBox>, caps: DriverCaps) -> Result<Box<dyn AgentDriver>, DriverError> }`, `DriverFactory::new()`, `register(id, builder)`, `adapter_ids()`, `driver_for`, `caps_for(&Agent) -> DriverCaps`, `adapter_id(&Agent) -> String`. `fake.rs` gains `FakeAdapter` and `DriverFactory::with_test_support()` under `test-support`.
- **Mirror**: `crates/htui/src/ui/tabs/registry.rs:53-80` (registry + `register`); `docs/ANA-4.md` §4.3 CLI caps paragraph.
- **Validate**: `cargo test -p htui-agent --features test-support --test extensibility`; full clippy and doc.

### Task 10: Settings tab agent section — `independent` (milestone 2; needs M1's T2 only)
- **Files**: `crates/htui/src/ui/tabs/settings.rs` (deleted) → `settings/mod.rs`, `settings/agents.rs`; `crates/htui/src/store_worker.rs`, `src/app/mod.rs`, `src/testkit.rs`; `crates/htui/tests/settings.rs`, `tests/snapshots/settings__*.snap`.
- **Tests first** (`tests/settings.rs`, `Harness`): snapshot `settings_agents_demo` — the demo's two agents as rows `name  transport  billing  models  default  enabled  on this box` with `on this box` reading `not probed`; `settings_agents_unknown_row` — a `zeta` row upserted into the harness's `MemStore` after construction (needs `Harness::over_store` public, `testkit.rs:73`) appears in the list with no other change; `settings_agents_offline` — a `StoreReply::Failed { request: "agents", .. }` renders the one-line "agent registry needs Postgres"; `settings_sections_cycle` — `h`/`l` move between sections once a second section exists (a probe section in the test); `store_worker` unit test: `Agents` round-trips through `serve` over `Backend::memory(MemStore::demo())` and yields two summaries.
- **Action**: D14. `settings/mod.rs` (`SettingsTab { sections: SettingsRegistry }`, `SettingsSection` trait mirroring `DetailTab`'s five methods with `on_scope_change` in place of `on_item_change`, `SettingsRegistry` mirroring `DetailRegistry`), `settings/agents.rs` (`AgentsSection` issuing `StoreRequest::Agents` from `wants_requests`, table render, empty line "no agents registered"), `store_worker.rs` variant/arms, `app/mod.rs` registration, `testkit.rs` accessor.
- **Mirror**: `crates/htui/src/ui/tabs/backlog/detail/mod.rs:55-80`, `runs.rs` table render; `crates/htui/src/ui/tabs/settings.rs:1-61` (replaced).
- **Validate**: `cargo test -p htui --features testkit --test settings`, `cargo test -p htui` (existing shell/backlog snapshots unchanged), clippy, `cargo run -p htui -- --demo` → `3` → agents listed.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features
cargo doc --workspace --no-deps
cargo tree -i tokio -e normal --workspace                    # criterion 13: no agent-client-protocol edge
cargo run -p htui -- --demo                                  # Settings tab lists claude and agy
```
ANA-4 §11 mapping: 1 → T6 + T9 (one `CASES` list, two harnesses, zero transport-specific cases);
2, 3, 4 → T4 unit tests and the T6 cases of the same names; 5 → T6 `cancel_answers_parked_permissions`
(fake only; the ACP half is M3); 13 → T7 validate step; 12's first half → T5. Criteria 6–11 are
milestones 3–7 and are not claimed here.

**Prerequisite (met)**: dev Postgres via `compose.yaml` on port 5439 (`HANDOFF.md` live
coordinates), needed for T2, T5 and T8.

## Verified claims (§3.5 fact-check)
Pre-filled by the planner against the tree on 2026-09-06 (`tree` = grep/read; `registry` = crate
source under `~/.cargo/registry/src`; `cargo` = `cargo metadata` / `cargo tree --locked`).

**Re-verified independently by the main thread on 2026-09-06 before CONFIRM. All fifteen upheld**;
each was re-run against the tree rather than read from this table. Three corrections, none
load-bearing:

- V9 gives `similar 3.2.0`'s MSRV as `(n/a)`; it declares `rust-version = "1.85"`
  (`similar-3.2.0/Cargo.toml`). Below the bump either way, so D10 is unaffected.
- V15 cites `testkit.rs:63-73` for `Harness::over`; it is at `testkit.rs:75` and is private as
  claimed. T10's "make it public" action stands.
- V7 says `StoreRequest`'s doc "reserves MOD-2's variants"; the doc at `store_worker.rs:41-43`
  names `StepEvents(StepId)` only, not `Agents`. D14 adds `Agents` as a new variant either way.

**Toolchain probe (added by the main thread; the plan asserted the shape but probed only its
rejected alternative).** D2's trait pair was compiled against the repo's actual toolchain, not
recalled: `rustc 1.98.1`, edition 2024, `AgentDriver: Send + Sync + Debug` and
`AgentSession: Send + Debug` with `Pin<Box<dyn Future + Send + 'a>>` returns. Result: both are
dyn-compatible as `&dyn` and `Box<dyn>`, and an `async move` loop driving
`Box<dyn AgentSession>::next_event()` is `Send` — the property MOD-4 needs to spawn a session onto
the runtime. ANA-4 probed that AFIT *fails* (`E0038`); nothing had probed that the adopted shape
*succeeds*. It does.

**Task independence — verified by file-set intersection, not by the plan's prose.**
Wave A: T1 ∩ T2 ∩ T3 ∩ T5 = ∅. The two near-misses are distinct files: T2 touches
`htui-core/src/model/mod.rs` while T3 touches `htui-core/src/lib.rs`; T2 touches
`htui-store/src/{pg/write,pg/read,backend}.rs` while T5 touches `htui-store/src/cache/pending.rs`.
Wave D: T7 ∩ T8 ∩ T10 = ∅. Cross-wave collisions exist and are correctly serialised, not parallel:
root `Cargo.toml` (T1 wave A, T7 wave D), `htui-core/src/model/agent.rs` (T2 wave A, T8 wave D),
`.sqlx/` (T2, T8), `htui-agent/src/lib.rs` (T1, T4, T6, T7, T9 — five waves), `htui-agent/src/fake.rs`
(T6 wave C, T9 wave E). Every parallel marking in this plan is upheld.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | Three workspace members; `rust-version = "1.85"`; `unsafe_code = "forbid"`; `sha2 0.10` and `futures 0.3` already workspace deps; `similar`, `which`, `process-wrap`, `tokio-util`, `agent-client-protocol` absent from `[workspace.dependencies]` | **true** | `Cargo.toml:2,7,20,31,35`; `Cargo.lock` has no `tokio-util` / `which` / `process-wrap` / `agent-client-protocol` entry |
| V2 | `WriteStore` has exactly two implementors, `MemStore` and `PgStore`; `CacheStore` and `Backend` implement `ReadStore` only; both traits carry `#[allow(async_fn_in_trait)]` | **true** | `crates/htui-core/src/store/mem.rs:697,727`; `crates/htui-store/src/pg/write.rs:27`; `pg/read.rs:48`; `cache/read.rs:208`; `backend.rs:6,213`; `traits.rs:15,37` |
| V3 | `CASES` holds 15 names and its length is asserted against the literal `15` in exactly two places | **true** | `conformance.rs:20-36`; `crates/htui-core/tests/mem_store.rs:24`; `crates/htui-store/tests/pg_conformance.rs:19,24-25` |
| V4 | `upload_pending` inserts `run` (`kind 'chat'`, `mode 'manual'`, `item_id NULL`, `status 'done'`) and `run_step` (`phase_name 'chat'`, `attempt 1`, `fanout_index 0`) with `ON CONFLICT (id) DO NOTHING`, from a `<project_id>.<run_id>.jsonl` name | **true** | `crates/htui-store/src/cache/pending.rs:5,204-246,130-137` |
| V5 | The fixture's agents are `claude` (`acp`, `subscription`, `["claude","--acp"]`) and `agy` (`cli`, `per_token`, `["agy","run"]`); `agent.launch` is documented as `{argv, env}` | **true** | `crates/htui-core/src/fixtures.rs:387-411`; `crates/htui-core/src/model/agent.rs:38`; `crates/htui-store/migrations/0001_init.sql:98` |
| V6 | `agent` / `agent_box` are not mirrored; `MemStore.agents` is `#[expect(dead_code)]`; `PgStore` seeds no agent rows and says MOD-2 will | **true** | `docs/ANA-9.md:306-311`; `cache_migrations/0001_mirror.sql` (no `agent` table among `:28-125`); `mem.rs:64-66`; `pg/mod.rs:214-215`; `docs/decisions/mod/mod-6.md:85-86,117` |
| V7 | `SettingsTab` is a stub; `Tab`, `TabRegistry`, `DetailTab`, `DetailRegistry` and `register_all` have the shapes D14 mirrors; `StoreRequest` doc reserves MOD-2's variants | **true** | `crates/htui/src/ui/tabs/settings.rs:1-61`; `tabs/registry.rs:33-49,53-80`; `backlog/detail/mod.rs:55-80`; `app/mod.rs:40-59`; `store_worker.rs:41-43` |
| V8 | `Uuid::now_v7()` is the id mint; `RunId` / `StepId` exist; `RunStatus` and `StepStatus` share `done|failed|cancelled`; `run_step.prompt_digest` and `usage` exist | **true** | `crates/htui-core/src/model/ids.rs:31-32,105-107`; `model/run.rs:31-43,57-69,155,160`; `0001_init.sql:487,489` |
| V9 | Locked `sqlx-core 0.9.0` declares `rust-version 1.94.0`, `ratatui 0.30.2` declares `1.88.0`; toolchain is 1.98.1; `agent-client-protocol 2.1.0`, `process-wrap 10.0.0`, `which 8.0.6`, `tokio-util 0.7.19`, `similar 3.2.0` are in the local registry with MSRVs 1.88.0 / 1.87.0 / 1.70 / 1.71 / (n/a); `tokio-util` feature `compat` exists | **true** | `cargo metadata`; `rust-toolchain.toml`; registry `*/Cargo.toml:14`, `tokio-util-0.7.19/Cargo.toml:54` |
| V10 | Today `cargo tree -i tokio` lists `htui`, `htui-store`, `sqlx-core`, `tokio-stream` (normal) — the baseline T7 must extend without an SDK edge | **true** | `cargo tree -i tokio --locked --workspace --depth 1` |
| V11 | Wave A: T1 ∩ T2 ∩ T3 ∩ T5 = ∅; wave D: T7 ∩ T8 ∩ T10 = ∅ | **true** | T1 = root `Cargo.toml` + `crates/htui-agent/**`; T2 = `htui-core/src/{store/*,model/{run,agent,mod}.rs}`, `htui-core/tests/mem_store.rs`, `htui-store/src/{pg/write,pg/read,backend}.rs`, `htui-store/tests/pg_*.rs`, `.sqlx`; T3 = `htui-core/src/{scrub.rs,lib.rs}`; T5 = `htui-store/src/cache/pending.rs`, `htui-store/tests/cache.rs`. T7 = root `Cargo.toml`, `htui-agent/{Cargo.toml,src/launch.rs,src/lib.rs,tests/launch.rs}`, `README.md`; T8 = `htui-core/{seeds/**,src/model/agent.rs,src/fixtures.rs}`, `htui-store/{src/pg/mod.rs,tests/migrations.rs}`, `.sqlx`; T10 = `htui/src/{ui/tabs/settings*,store_worker.rs,app/mod.rs,testkit.rs}`, `htui/tests/settings.rs` + snapshots. `.sqlx` files are per query hash |
| V12 | `AcpAgentConfig` has private fields and a `new(command)` / `arg(..)` builder; `ByteStreams` is public | **true** | registry `agent-client-protocol-2.1.0/src/acp_agent.rs:53-75`, `src/jsonrpc.rs:6386` |
| V13 | `similar 2.7.0` is already in the lock (via `insta`), so ANA-4's `similar 3.2.0` will be a second copy | **true** | `Cargo.lock:2708`; `cargo metadata` |
| V14 | `go_offline` on an already-`Offline` backend returns early after `went_offline()` is false | **true** | `crates/htui/src/store_worker.rs:328-330,440-442` |
| V15 | `Harness::over(MemStore)` is private; `Harness::demo()` wraps `MemStore::demo()` | **true** | `crates/htui/src/testkit.rs:63-73` |

## Where the ANA docs and the tree disagree
Design authority is ANA-4 / ANA-5; fact authority is the tree. Each item says which wins here.

| # | ANA says | Tree says | Ruling |
|---|---|---|---|
| X1 | `agent.launch` is `{command, args, env, discovery}` (ANA-4 §5.1) | `{argv, env}` in the fixture, the model doc and the DDL comment (V5) | ANA-4 wins; T8 corrects the fixture and the model doc (the DDL comment goes via migration `0002`'s `COMMENT ON COLUMN`, milestone 5 — not here). |
| X2 | Settings agent section is build step 4 (ANA-4 §9) | PRD milestone 2 requires it | PRD wins on sequencing; the section renders store rows only (D14), the caps banner and quota stay in M3/M7. |
| X3 | `WriteStore` additions are the four of §4.1 (ANA-4) | PRD adds chat-run ownership; `traits.rs:50` reserves "runs, steps, events, … agents" | Both: D3 adds `start_chat_run` / `finish_chat_run` (A1). |
| X4 | ANA-9 §4.3 names the buffer `<run_id>.jsonl` | `pending.rs:5` is `<project_id>.<run_id>.jsonl`, and ANA-4 §4.1 already follows the tree | Tree wins (ANA-4 agrees). |
| X5 | `tokio test-util` is "already a dev-dependency" (ANA-4 §8) | true for `htui-store` and `htui`, not for `htui-core` (`crates/htui-core/Cargo.toml:25` has `macros, rt` only) and there is no `htui-agent` yet | T1 adds it to `htui-agent`; `htui-core` needs none. |
| X6 | A `launch` row "deserializes into the SDK type with no adapter layer" (ANA-4 §5.1) | `AcpAgentConfig` fields are private, the row carries `discovery` and unresolved `${}` (V12) | Own serde types + `to_acp_config()` through the builder (D11); no design change, one sentence of ANA-4 is looser than the crate. |
| X7 | `similar 3.2.0` is a new dependency (ANA-4 §8) | `similar 2.7.0` is already locked through `insta` (V13) | Add 3.2.0 as ANA-4 says; two copies are accepted and noted for the close-out. |
| X8 | `set_step_usage` keeps `prompt_digest` and MOD-2 passes `None` (ANA-5 §4.4) | nothing writes `run_step.prompt_digest` for a chat step before M9 | D15(b): the recorder passes `Some` until M9 rules. |
| X9 | MSRV rises to `1.88`, the SDK's own floor (ANA-4 §4.2) | `sqlx-core 0.9.0` declares `1.94.0` and the toolchain is exact-pinned at `1.98.1`, so 1.88 is unbuildable and untestable — a second fiction replacing the first (V9) | **Maintainer wins** (override at CONFIRM, 2026-09-06): `1.98`, matching the pin. ANA-4 §4.2's number is superseded; the MOD-2 close-out records the override so a later reader does not read it as drift. |

## Assumptions for the maintainer to rule on at CONFIRM
- **A1** `finish_chat_run` ships beside `start_chat_run` (the PRD names only creation; D4 says why).
- **A2** The fake is reachable through the production factory as adapter id `cli/fake`, extending §5.2's `cli.stream` vocabulary under `test-support` only (D12).
- **A3** `similar 3.2.0` is workspace-declared in M2 but pulled into no crate until M3; the other four new dependencies are pulled into `htui-agent` in M2 with real consumers (D10).
- **A4** The four skill types defer to milestone 9 (D9).
- **A5** PRD milestone rows 1 and 2 are left `pending` by this plan; the `/plan` contract flips them to `in-progress` at CONFIRM on the main thread.
- **A6** Milestone 2 spawns no agent protocol; its spawn test drives `$CARGO --version` through the real `spawn` path (D11).
- **A7** `MinimalScrubber` masks every non-empty env value regardless of length (PRD: "every"); short-value false positives are accepted until ANA-7 / MOD-10 (D5).
- **A8** `htui` gains its `htui-agent` dependency in milestone 3, not here (D1).
- **A9** `Backend::agents()` while `Offline` is `StoreError::Unreachable`, rendered as one line in the section (D14, V14).

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| The conformance `Script` language cannot express a case a real transport needs (M3 discovers a gap) | Medium | `ScriptEvent` is a closed enum M3 may extend additively; criterion 1 forbids transport-specific *cases*, not richer scripts; the fake plays whatever the enum holds |
| The 15 → 20 bump lands without a Postgres run | Low | T2's validate step needs the server; `pg_conformance.rs:19` fails loudly otherwise (its own doc says so) |
| `MinimalScrubber` masking every env value produces false positives (a value like `1`) before ANA-7 | Medium | Accepted for M1–M2 (PRD: "every"); the limit is recorded in the MOD-2 decision doc; MOD-10 replaces the impl (A7) |
| Two `similar` versions or the SDK's smol-family reactors bloat the build | Low | `similar` 3.2.0 is workspace-declared only until M3; the SDK's runtime cost is M3's risk (ANA-4 risk 3) and no reactor starts in M2 |
| `process-wrap` job-object assignment refused on this box | Low | D11 downgrades to a plain spawn with `job_object = false` and a `warn!`; the T7 spawn test asserts `true` only on Windows and prints the downgrade |
| The `zeta` grep assertion is brittle to an unrelated identifier containing `zeta` | Low | The name is chosen for that reason and the walk excludes the test file; rename to a rarer token if a hit appears |
| A `Settings` change moves an existing shell snapshot | Low | Tab strip text is unchanged (`Settings` title kept); T10 runs the full `htui` suite |
| Parallel agents in wave A both touch `htui-core/src/lib.rs` | Low | Only T3 touches it (V11); T2's re-exports go through `model/mod.rs` |
| Session limit mid-milestone | High (PRD) | Each task is a commit; `HANDOFF.md` phase notes per milestone; the workflow script replays cached agents on resume |

## Acceptance
- [x] All ten tasks complete, tests-first. Milestone 1 ran as six agents; milestone 2 ran serially
      on the main thread after the maintainer declined the fan-out (execution amendment 4). The
      `rust-reviewer` pass is milestone 2's one open gate — see the close-out note below.
- [x] Validation block passes on **Linux** (this box); `HTUI_TEST_DATABASE_URL` run green with
      `USERNAME=htui-ci` and **no skips**; `cargo tree -i tokio` shows no SDK edge. Windows is
      unverified for milestone 2: the `CREATE_NO_WINDOW` + job-object spawn path compiles nowhere
      on this box and is asserted only by the `cfg(windows)` arm of `tests/launch.rs`.
- [x] ANA-4 §11 criteria 1–4 and 13 mapped to passing tests / validate steps; 5 and 12 (fake
      halves) noted. Criterion 13 verified: `htui`, `htui-agent`, `htui-store`, `process-wrap`,
      `sqlx-core`, `tokio-stream`, `tokio-util`, and no `agent-client-protocol` edge (the SDK
      reaches `tokio` zero times, running on the smol family).
- [x] `R-AGT-5` test (`tests/extensibility.rs`) passes and its absence sweep finds `zeta` nowhere.
      The sweep earned its place: it failed first on a doc comment in `fake.rs` that used the name
      as an example, which would have made the proof circular.
- [x] Patterns mirrored, not reinvented (ANA-4 §4.1 / §5 verbatim). Reviewer verification pending
      with the gate above.
- [x] PRD milestone rows 1 and 2 → `complete`; `HANDOFF.md` phase note per
      `.claude/rules/workflow-docs.md` lifecycle 4.

## Milestone 2 close-out (2026-09-07)

Commits: `5c717d0` (port), `5863dff` (T7), `e6e44fd` (T8 core), `2bed6a6` (T9), `b70390b` (T8
store), `1e7de4d` (T10). Workspace suite green with Postgres live: 243 tests, no skipped Postgres
case.

**Findings the tasks produced, each fixed rather than worked around:**

| # | Finding | Where |
|---|---|---|
| F1 | `process-wrap`'s `ProcessGroup` is behind the `process-group` feature, which the plan's list omitted — the unix spawn would have had no supervision at all | T7 |
| F2 | `process-wrap` 10 renamed `TokioCommandWrap` / `TokioChildWrapper` to `CommandWrap` / `ChildWrapper`; D11 used the 8.x names | T7 |
| F3 | `CreationFlags` wraps the `windows` crate's `PROCESS_CREATION_FLAGS` and exposes no constructor, so ANA-4 §4.6's mandatory `CREATE_NO_WINDOW` needs `windows` as a `cfg(windows)` dependency the plan did not list | T7 |
| F4 | `clippy.toml` carries its own `msrv`; the plan's T7 file set missed it, and clippy warns while silently linting against the older number | T7 |
| F5 | Raising that number surfaced `clippy::manual_is_multiple_of` in pre-existing code (`App::on_tick`) — the lint only fires at MSRV ≥ 1.87 | T10 |
| F6 | A per-statement `WHERE NOT EXISTS (SELECT 1 FROM agent)` in the seed loop reads false for the second row: the registry would have come up holding `claude` alone. Emptiness is read once, before the loop | T8 |
| F7 | The seed broke `load_demo`'s stated invariant ("a conflict is a bug"): the fixture carries the same two names under its own ids, so loading it hit `agent_name_key`. The fixture owns the registry and deletes the seeded rows by name | T8 |
| F8 | `load_demo_round_trips_a_count_per_table` measured a per-table delta, which is now zero for `agent`; it asserts the absolute count instead of being weakened | T8 |
| F9 | `FakeAdapter::build` refusing an empty script slot is correct (a real adapter whose process will not start behaves the same), so the T9 test loads a script rather than the adapter substituting one silently | T9 |

**Deferred, with owners:** the Windows spawn path is unverified on this box (milestone 3 runs it
first on Windows); `similar 3.2.0` is workspace-declared with no consumer until milestone 3 (A3);
`DriverFactory::with_test_support`'s adapter is unreachable from outside, so a caller that needs to
drive a session registers its own `FakeAdapter` — documented on the method.
