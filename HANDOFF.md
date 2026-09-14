# HANDOFF - Outstanding Work (htui)

> **Purpose:** Carry outstanding work between sessions so work resumes.
> `htui` is a Rust TUI that wraps coding agents (`claude`, `agy`) to run a backlog of work items
> through step graphs across projects and boxes. Product contract: `docs/REQUIREMENTS.md`
> (requirement IDs `R-<AREA>-<N>`). Standing invariants: `CONCEPTS.md`.

## How to use this file

- **Session start:** read this file first. Pick the next open item.
- **On completion:** delete the checklist line, write `docs/decisions/<prefix>/<prefix>-N.md`,
  prepend the index line to `DECISIONS.md`, update the summary table (per
  `.claude/rules/workflow-docs.md`; this repo is outside the `sync-workflow-surface` default
  target list, pass `-Targets` explicitly to receive the surface).
- Every item cites the requirement IDs it addresses (`R-NF-4`).

**Current status (2026-09-14):** **CLEAN-1 shipped** (`docs/decisions/clean/clean-1.md`): Fixed rustdoc private and ambiguous links in htui-agent.
Before it, **TOOL-6 shipped** (`docs/decisions/tool/tool-6.md`): Fixed shell-redirection race condition in tests/launch.rs. **TOOL-2 concluded** (`docs/decisions/tool/tool-2.md`): Fixed demo fixture username collision and made database test skips in CI distinguishable.
Before it, **TOOL-4 concluded** (`docs/decisions/tool/tool-4.md`): Fixed TOCTOU race condition test flake in `tests/auth.rs`.
**MOD-2 milestone 8 landed** (`a3cbfff`..`42294d8`, phase note
below): an agent that speaks no ACP — only its own headless JSON stream — now reaches the same chat
tab, recorder, store rows and replay as an ACP one, losing exactly three event kinds and saying so
in its banner. `R-AGT-3` is met. The registry carries a second row, **`claude-cli`**; `CASES` is
still **15** with **three** bindings reporting every one (§11 criterion 1 in fact, not by
assertion); criterion 7 is proven live over two real turns. **Nine probes ran before a line of
`src/cli/` was written**, and five of their findings changed code that was about to ship — most
sharply that `modelUsage` is *cumulative* while `result.usage` is per-turn, which written the other
way would have double-counted a two-turn chat with **no test in the tree failing**. `docs/ANA-4.md`
§11.14's two CLI items are answered from live runs, and the ANA is amended in four places (D79,
D92, D93, D94) here rather than in the file, per the milestone-5 precedent. Milestone 9 (the prompt
assembler and preview) is the last of the nine.
**MOD-2 milestone 6's T34 is unblocked and half answered.** The box is authenticated now, so
`agy_live.rs` passes 4/4 against a logged-in server and `session/new` succeeds, reporting its
`config_options` (permission modes `default`, `auto_edit`, `yolo` — MOD-2 D64's coordinate, now
learnable). The **three ANA-4 §11.14 questions still need a live turn**, which is MOD-2's own work:
whether `agy_acp_server` emits `usage_update` and in what field, whether it issues
`session/request_permission` in `default` mode and with what option ids, and whether its edits
arrive as a standard `tool_call` + `diff` or in a vendor shape.
**Live coordinates.** Migration `0002_agent_probe.sql` exists, so MOD-4's `0003_orchestration.sql`
is no longer held (`docs/ANA-2.md` §9) and is **still the next migration** — MOD-20 deliberately
added none. Adapters install under `HTUI_AGENTS_ROOT`, default `dirs::data_local_dir()/htui/agents`;
`HTUI_TOOL_<NAME>` still overrides everything. Dev Postgres via `compose.yaml` (port 5439); tests
need `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres` and the
`USERNAME=htui-ci` prefix of TOOL-2 (`docs/decisions/mod/mod-6.md`).
**Concluded analyses the open items lean on:** ANA-10 (`docs/decisions/ana/ana-10.md`) — a box with
no DSN becomes a *complete* box over a separate `local.sqlite`, spawning MOD-17, MOD-18 and MOD-19,
and `docs/REQUIREMENTS.md` was amended for it on 2026-09-08 (new `R-STO-7`, eleven amended in
place); ANA-5 (`docs/decisions/ana/ana-5.md`) — the prompt contract, no new crate, no new migration;
ANA-2 (`docs/decisions/ana/ana-2.md`) — step graphs, three compare-and-set status tables,
`htui-orch`. MOD-2, MOD-7, MOD-9, MOD-13, MOD-14, MOD-15 and MOD-17 can start now.

---

## Open items

### Analyses


- [ ] **ANA-3 - External context tools (later tier).** `R-LATER-7`. Headroom, Serena, Graphify and
  structural diff as optional excerpt providers for the prompt builder, fail-open when absent.
  Deferred until MOD-2 lands. The seam is fixed by ANA-5 (`docs/ANA-5.md` §4.5,
  `docs/decisions/ana/ana-5.md`): `ExcerptProvider` in `htui-core::prompt::excerpt`, propose-only,
  fail-open with a deadline, read-only (`R-ID-4`), non-LLM (`R-ID-6`); Serena's default `.serena/`
  directory must be configured outside every `repo_box_path`. Output: `docs/ANA-3.md`.
- [ ] **ANA-11 - Models for requirements and decisions.** Evaluate database schema models to track product requirements (R-IDs) and architectural decisions (MOD/ANA items) inside `htui` itself instead of standalone markdown files.
### Next features

- [ ] **MOD-28 - rataflow execution view (from ANA-12).** Add `rataflow` dependency, implement `ExecutionGraph` widget mapping `RunStep` and `SessionEvent` lists to a node graph, add view toggle to Runs tab (`R-TUI-4`), and wire mouse/keyboard events for standard run actions.
- [ ] **MOD-26 - Declarative Agent Personas (from ANA-13).** Build Markdown/Frontmatter parser in `htui-core`, discover from `~/.config/htui/agents.d/`, map to `SessionSpec` overrides (model, tools).
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees.
- [ ] **MOD-2 - Agent driver + chat tab** (from ANA-4). `R-AGT-1..8`, `R-PRM-1..3`, `R-TUI-6`,
  `R-TUI-8`, `R-HIS-1..2`. ACP client, CLI adapter, agent registry with its Settings tab section
  (agents and quota), autodiscovery, quota tracking, streamed chat tab with follow-ups and inline
  permissions, event persistence and replay. Driver design concluded in `docs/ANA-4.md` (ANA-4,
  `docs/decisions/ana/ana-4.md`). Prompt builder concluded in `docs/ANA-5.md` (ANA-5,
  `docs/decisions/ana/ana-5.md`): `htui-core::prompt` (template scanner and validator, render,
  `chars-v1` estimator, kept-first trim, digest, `ExcerptProvider`/`RepoReader` traits, ranker) and
  `htui-agent::excerpt` (`FsRepoReader`, walk, skip rules) per §8; six store methods plus
  `WriteStore::set_step_prompt` (the pre-flight `prompt_digest` and `trim_record` writer); the
  ANA-5 sections of `0002_agent_probe.sql` (five `COMMENT ON COLUMN`, ten `app_setting` keys, §9);
  `insta` as an `htui-core` dev-dependency for golden prompts; the five "Unverified - MOD-2 must
  confirm" items of §12 criterion 21 (the no-workspace `Scope` bound first). MOD-4 consumes
  `DriverCaps`, `SessionSpec.cwd`/`extra_dirs`, `session_ref` and the `session_started` row
  (`docs/ANA-2.md` §4.2, §4.8), and `0002_agent_probe.sql` must land before ANA-2's `0003`. Not
  blocked (ANA-4 and ANA-5 concluded).
  **Phase 1 landed (uncommitted, 2026-09-07):** PRD `.claude/prds/mod-2-agent-driver-chat.prd.md`
  (nine milestones), plan `.claude/plans/mod-2-driver-seam-registry.plan.md` (milestones 1-2).
  Milestone 1 complete: new `htui-agent` crate (ANA-4 §4.1 `AgentDriver`/`AgentSession` seam,
  11-variant `DriverEvent`, recorder, `FakeDriver`, 13-case transport-neutral conformance suite),
  `htui-core::scrub` (`Scrubber` + fail-closed `MinimalScrubber`, the ANA-7 stand-in MOD-10
  replaces), six `WriteStore` methods incl. `start_chat_run`/`finish_chat_run`, inherent `agents()`
  over three `Backend` arms, `append_pending`, `conformance::CASES` 15 -> 20. `rust-reviewer` ran
  over the whole change set: 23 findings adjudicated by an adversarial pass (12 real, 5 partly,
  6 refuted), 10 fixed with bite-proven tests, 7 deferred (see the plan's amendments).
  **Phase 2 landed (2026-09-07, `5c717d0`..`1e7de4d`):** workspace MSRV 1.85 -> **1.98** with
  `clippy.toml` moved with it (maintainer override of ANA-4 §4.2's 1.88; `sqlx-core 0.9.0` already
  floors at 1.94, plan X9); the milestone-2 dependency set (`agent-client-protocol =2.1.0`,
  `tokio-util`, `process-wrap` incl. the `process-group` feature the plan omitted, `which`,
  `similar` declared only, and `windows` on Windows targets for `CREATE_NO_WINDOW`);
  `htui-agent::launch` (`AgentLaunch`/`AgentSettings` serde types, `${tool}` resolution,
  `to_acp_config` through the SDK builder, supervised `spawn` with a job object on Windows and a
  process group on unix); `htui_core::model::agent::seed_rows` from `crates/htui-core/seeds/*.json`
  with `PgStore::seed_if_empty_as` inserting them and the demo fixture derived from the same
  source; `DriverFactory` keyed by transport (`acp`, `cli/<stream>`) with the `R-AGT-5` proof in
  `crates/htui-agent/tests/extensibility.rs`; and the Settings tab's `SettingsRegistry` plus its
  agent section (`StoreRequest::Agents`, `Harness::over` made public). Nine findings, all fixed
  (plan "Milestone 2 close-out"). Verified on **Linux** with Postgres live (243 tests, no skipped
  Postgres case, `USERNAME=htui-ci` per TOOL-2); the Windows spawn path is unverified here and
  milestone 3 is the first to run it. `rust-reviewer` ran and blocked on one HIGH - `spawn`
  resolved its command with a blocking `which` inside async code - plus three MEDIUM; three are
  fixed in `34d5f04` (`spawn` is now `async` over `spawn_blocking`, the registry parses
  `agent.settings` once, and the `R-AGT-5` token sweep T9 promised now exists and is bite-proven),
  the fourth accepted with its reason (plan "Review gate").
  **Phase 3 landed (2026-09-07, `a142fbf`..`682a423`):** live `claude` over ACP and the chat tab,
  planned in `.claude/plans/mod-2-live-acp-chat.plan.md` with the `code-architect` blueprint beside
  it. `htui-agent` gained its first real transport (`acp/{mod,map,client,fs}.rs`): one task per
  session owning the whole `connect_with` future, the §6.1 mapper over raw JSON (the adapter ships
  five update kinds the schema does not know), `fs/write_text_file` intercepted into an
  `edit_proposal` with a `similar` unified diff, model selection by config-option **id**, the
  session banner, and the three-stage permission pipeline with `permission.rs` deciding stages 1-2
  where the recorder is. `htui-store` gained `Writer` (an owned `WriteStore` handle; `Offline` still
  hands out none) and `Backend::{writer, this_user}`; `htui` gained `agent_worker.rs`
  (`AgentRuntime`, `run_chat`), the four chat `StoreRequest` variants with `StoreReply::Chat`
  frames, and the Chat tab (`R-TUI-6`: streamed text, folded thoughts, tool calls, diffs, inline
  permission answers, capability banner). **The thirteen `conformance::CASES` now pass over a real
  ACP wire and no case was added** (§11 criterion 1); two case *scripts* were amended because ACP
  reports no per-turn tokens (§7) and carries no verbatim diff (§4.3). Recorded transcript fixtures
  come from a live turn against adapter **0.48.0**, and they close two §11.14 items: the rate-limit
  blob does arrive under `_meta["_claude/rateLimit"]`, on a later `usage_update` than the first, and
  `cost` appears only once the turn has produced output. Live on this box: the seeded row resolves,
  spawns the adapter, completes a `protocolVersion 1` handshake and dies with its process tree
  (`tests/acp_live.rs`), and a whole chat streams into the store through the production runtime
  (`crates/htui/tests/chat_live.rs`) — §11 criterion 9's first half and criterion 11's
  process-group half. `rust-reviewer` blocked on one CRITICAL and seven HIGH findings, all fixed in
  `682a423`: the SDK drops the foreground future when a connection actor fails, which orphaned the
  agent process; a stream ending before its `done` was recorded as a finished turn; the path guard
  admitted everything when a root was relative; and `edit_proposal.accepted` defaulted to `true`
  before the user had seen the request. Verified on **Linux** with Postgres live. **Windows is compile- and
  lint-checked but not run**: `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent
  --all-targets --all-features` is green (README "Checking the Windows-only code from Linux") and
  caught two defects a Linux build cannot see, but the job object's kill-on-close guarantee, the
  `.cmd` shim `CreateProcess` refuses and `CREATE_NO_WINDOW` are runtime facts that need a Windows
  box; milestone 5 is the next to touch that path.
  **Phase 4 landed (2026-09-08, `81d247b`):** durable history and replay, planned in
  `.claude/plans/mod-2-durable-history-replay.plan.md` with the `code-architect` blueprint beside it.
  An offline chat is no longer refused: the mirror gained the `agent` table (an unscoped full replace
  beside `app_user`, `cache_migrations/0002_agent_mirror.sql` — **so MOD-4's cache migration is now
  `0003_orchestration.sql`**, amending `docs/ANA-2.md` §9) plus `CacheStore::{agents, this_user,
  user_named}`, and `Writer::Buffered` records through `append_pending` into
  `<cache_dir>/pending/<project>.<run>.jsonl` while `is_writable()` still answers `false`.
  `upload_pending` now fills `run_step.prompt_digest` and `run_step.usage`, the summing rule having
  moved to `htui_core::model::UsageTotals` so the recorder and the uploader cannot drift (the
  recorder also sums a masked `usage` payload now, which is what makes them agree in every row
  shape). A live buffer is `<…>.jsonl.open` until `finish_chat_run` seals it, and a name collision
  seals to `<project>.<run>.<n>.jsonl` rather than appending — every site tagged `[H-1]`, a
  deviation from the plan's D35 kept cheap to revert, because the refresher would otherwise upload a
  chat mid-flight and freeze its usage at a partial sum. Replay decodes rather than re-renders:
  `htui_agent::replay` inverts the recorder's encode, `StoreRequest::StepEvents` answers with the
  persisted rows (`None` = not on this box, distinct from "recorded nothing"), the Runs pane selects
  a step with `J`/`K` and `Enter` emits `Action::Replay`, and the Chat tab's replay mode is
  read-only structurally — `on_key_replay` takes no `Ctx`, so it cannot request. **§11 criterion 12
  is proven end to end** (`crates/htui/tests/chat_offline.rs`): an offline chat driven by the
  production runtime writes the buffer and `upload_pending` lands exactly those rows, second pass a
  no-op. 410 tests green on **Linux** with Postgres live; `rust-reviewer` returned no CRITICAL and no
  HIGH, and its two MEDIUM findings (both in the `[H-1]` sealing fallback) are fixed in this commit.
  Known and accepted: an uploaded offline chat has `run_step.agent_id` and `model` NULL, the line
  format carrying `session_event` columns only. **Windows verification of every phase is MOD-16**,
  which owns the runtime facts a Linux box cannot answer.
  **Phase 5 landed (2026-09-08, `fb626a8`):** autodiscovery and the box probe, planned in
  `.claude/plans/mod-2-probe-autodiscovery.plan.md` with the `code-architect` blueprint beside it.
  **`migrations/0002_agent_probe.sql` exists, so MOD-4's `0003_orchestration.sql` is no longer
  held** (`docs/ANA-2.md` §9): it carries ANA-4 §9's `agent_box.probe JSONB` plus ANA-5 §9's five
  `COMMENT ON COLUMN` contracts and ten `app_setting` defaults. Two **ANA amendments** were needed
  and are recorded here rather than in the ANAs (maintainer-only): ANA-4 §9's
  `COMMENT ON COLUMN agent.name IS NULL` is a **no-op** — `0001_init.sql` carries no database
  comment at all and its stale text is a source comment (`0001_init.sql:96`) the forward-only rule
  forbids editing — so `0002` writes a real comment naming `seed_rows`/`seed_if_empty_as` (plan
  D43); and ANA-4 §4.6's snapshot gains one key, `source` (`probe` | `manual`), because "a manual
  entry is never overwritten by a probe that finds nothing" (§4.6) needs a recorded origin (plan
  D45). `htui-agent` gained `probe.rs` (tier 1: one resolver `tools::resolve` now shares, `glob`
  through a hand-rolled `*`-per-segment walker with no new crate, versions from the seeds' own
  regexes and semver floors) and `acp/handshake.rs` (tier 2: `initialize` and nothing else, the
  child owned by a `ChildGuard` so timeout, actor failure, garbage, success and a dropped future
  all kill it). `htui` gained `StoreRequest::ProbeAgents`, served through `AgentRuntime::serve` as
  `Served::Deferred` with an owned `Writer` — never in the worker's `select!` arm, never on the UI
  task (`R-NF-3`) — `Settings > r` with its `probing…` column, and a 24 h `PROBE_TTL` re-probe at
  `ChatStart` that cannot block or fail the chat. **§11 criterion 9 is proven live on this box**
  (`crates/htui-agent/tests/probe_live.rs`: `status: ready`, `protocol_version: 1`, no surviving
  children); **criterion 10's live half is milestone 6's** — there is no `agy` on this box, so the
  glob resolution, its `--uid=` platform append and the `unauthenticated` mapping are proven by
  fixtures only. 463 tests green on **Linux** with Postgres live. `rust-reviewer` **blocked** on two
  HIGH findings, both in the orphan-process class this milestone was gated on: `npm root -g` ran
  through a bare `Command::output()` with no timeout and no process group, and an aborted
  `capture_version` left its child running. Both are fixed by one bounded-spawn path (`run_bounded`)
  and the lifted `ChildGuard`; five MEDIUM and four LOW followed, and the gate then cleared.
  Known and accepted: `open_session`'s own timeout arm still orphans its adapter the same way
  (blueprint H-2) and is **milestone 6's**; `unauthenticated` is auth-methods-only until milestone 6
  can check a credential; on the probe's *success* path the process group is not swept, because the
  leader has just been reaped and a late `killpg` could land on a reused pgid, so a tool that
  daemonises a helper can outlive its probe (none in the seeds does). Also landed:
  `crates/htui-store/build.rs`, because `sqlx::migrate!` registers rerun-if-changed **per file**,
  so adding `0002` did not invalidate a warm `target/` and the suite failed against a schema the
  binary did not know it had — the same trap awaits MOD-4's `0003`. **First launch after this lands
  rebuilds every box's mirror**: `PgStore::schema_version()` is now 2 and `CacheStore::open` treats
  a mismatch as a rebuild (MOD-6 plan D8).
  **Phase 6 landed (2026-09-09, `acf16f7`), one task short:** `agy` over ACP, planned in
  `.claude/plans/mod-2-agy-acp.plan.md` with the `code-architect` blueprint beside it. Google's
  first-party `agy_acp_server` **1.1.1 is installed on this box** (plan D57; `htui` does not
  download it, the README now carries the steps) and **§11 criterion 10 is proven live**
  (`crates/htui-agent/tests/agy_live.rs`): the unmodified seed row resolves the server through the
  glob, appends `--uid=`, completes `initialize` at `protocolVersion 1` in ~1.2 s against a 60 s
  timeout, records `unauthenticated`, and leaves no surviving process. Four decisions landed with
  it. **D58** makes the driver spawn `agent_box.probe.resolved` rather than resolving a second time
  (`AcpAdapter::build` stops ignoring `on_box`), which is the only path that carries a glob tool's
  per-platform `args`; milestone 5's H-3 was therefore not cosmetic — **`--uid=` is mandatory**, and
  without it the server aborts in `ChangeRootAndUser` (`Check failed: LookupGIDByGroupName(…) Group
  nobody not found`) before reading stdin. **D59** gives `unauthenticated` a declarative
  `discovery.credential { env, files }` block resolved by the probe (files through the existing glob
  expander, then variables) feeding `status_for`, so an authenticated box reads `ready`; nothing is
  keyed on an agent name (`R-AGT-5`), the probe never opens the file and records only the tier.
  **D60** re-probes the row when a chat fails to *spawn*, inline from that chat's own task and under
  a runtime-wide claim so two chats cannot probe one row at once; there is **no transport fallback**
  — by maintainer decision a CLI agent is its own registry row, so milestone 8 builds a row, not a
  degraded mode. **D61** stops `open_session`'s handshake timeout orphaning the adapter (the session
  task holds a `ChildGuard`; `SessionOptions` carries an injectable timeout). Beyond the plan, the
  review gate found and this commit fixes a message the SDK swallows: `start_session` sends from an
  actor whose failure **drops the foreground future**, so `session_main`'s error arm is unreachable
  for a JSON-RPC error and the vendor's `Authentication required` — the first thing an `agy` user on
  a fresh box meets — was replaced by "the session task ended before the handshake". `rust-reviewer`
  returned no CRITICAL and no HIGH; its three MEDIUM and five LOW are all applied. 485 tests green
  on **Linux** with Postgres live.
  **Two ANA-4 amendments are needed and are recorded here rather than in the ANA** (maintainer-only,
  the milestone-5 precedent): §4.5's and §4.6's premise that `agentInfo.version` is a build tag
  *disagreeing* with the registry semver is **false on the Linux 1.1.1 build**, which reports
  `agy_acp_server_1.1.1` — D57's "the registry value is a download coordinate only" still holds, its
  stated reason does not; and §4.5's protocol-echo warning is **confirmed live** (the server echoed
  `protocolVersion 99` when sent 99, so the echo is no evidence of support).
  **What milestone 6 still owes** (`T34`) — **no longer blocked**: the live `agy` chat.
  `agy_acp_server` keeps its credentials in `$GEMINI_HOME/antigravity-acp/`, a sibling of and
  separate from the `agy` CLI's own directory, so the CLI's login does not count — and **MOD-21
  logged it in from the app on 2026-09-10** (`docs/decisions/mod/mod-21.md`), which is what
  unblocked this. `agy_live.rs` now passes 4/4 against the logged-in server and `session/new`
  succeeds, reporting `config_options` (`default`, `auto_edit`, `yolo` — the D64 coordinate).
  What remains is **a live turn**, which none of the four existing cases drives, and with it
  **three ANA-4 §11.14 items** — whether `agy_acp_server` emits
  `usage_update` and in what field, whether it issues `session/request_permission` in `default` mode
  and with what option ids, and whether its edits arrive as a standard `tool_call` + `diff` or in a
  vendor shape. Two of the six closed here: the `.par` mechanics (the `--uid=` finding above;
  `localharness_external` ships beside the server on Linux too and the handshake does **not** need
  it, checked by moving it aside) and the `session/new` model list, which is **not learnable while
  unauthenticated**, so D64 leaves `models: []` and `model_config_id: null` as seeded. Milestone 7
  (quota and caps) can start without any of this.
  **Phase 7 landed (2026-09-10, `142beb1`..`acec1b8`), complete:** the live `agy` turn plus
  quota and caps, planned in `.claude/plans/mod-2-quota-caps.plan.md` with the `code-architect`
  blueprint beside it. **Milestone 6's `T34` is closed and all three remaining ANA-4 §11.14 `agy`
  items are answered live** (`crates/htui/tests/chat_live_agy.rs`, transcript fixture recorded, two
  runs agreeing): `agy_acp_server` emits **no `usage_update` whatsoever** — so §7's unverified row
  for `agy` resolves to *nothing*, its seed keeps `quota.source: "none"` and its quota column reads
  `—` by design; it **does** issue `session/request_permission` in `default` mode for a write but not
  for a read, offering exactly `allow`/`deny` with kinds `allow_once`/`reject_once` and no
  `*_always`; and its edits arrive as a **standard** `tool_call` with `kind: "edit"` and a `diff`,
  the vendor's own names riding inside schema fields, so **no mapper change was demanded** and the
  eleven `DriverEvent` variants stand. **D64 resolved the other way from milestone 6's fallback**:
  `session/new` does return `configOptions` with eleven model ids, so the seed now carries them,
  `default_model: "gemini-3.7-flash-high"` and `model_config_id: "model"`, and ANA-4 §4.4's
  selection-by-option-id is exercised end to end for the first time.
  Milestone 7 itself: `htui_core::model::quota` holds §7's document (`normalize`, `Quota`,
  `ProjectCaps`, and `available` — `R-AGT-8`'s skip predicate, which **MOD-4 consumes and MOD-2 only
  tests**), `QuotaSource` moved there with it; the ACP mapper lifts `_meta["_claude/rateLimit"]`
  verbatim onto `UsageEvent.quota`; the recorder **latches** the document passively after every
  `usage` row through a new narrow `WriteStore::set_agent_box_quota`; the **per-run cap** is detected
  in the recorder and cancelled by the loop that holds the session, leaving `error{cap_exceeded}`
  then `done{stop_reason:cancelled}` as a step's last two rows (§11 criterion 8); and
  `Settings > Agents` gained a `quota` column that states `r` cannot refresh it. **§11 criterion 7 is
  proven both clauses on live Postgres** (`crates/htui/tests/chat_usage_pg.rs`), and by construction
  rather than fixture luck — the mapper's delta is `round(total) − previous round(total)`, so a sum
  of deltas *is* the last rounded total. Caps are **USD micros** in `project.settings`
  (`per_token_cap_run` enforced, `per_token_cap_batch` read and logged — **MOD-12 enforces it**,
  ANA-4:1283); absent means unbounded, `0` cancels on the first costed row, and a malformed value
  refuses the chat. **No migration**: `agent_box.quota`/`quota_at` have existed since `0001`, so
  MOD-4's `0003` is still unheld. 738 tests green on **Linux** with Postgres live (46 targets).
  **Three ANA-4 amendments, maintainer-approved and recorded here rather than in the ANA** (the
  milestone-5 precedent): **D69** — §7's "the recorder … calls `AgentSession::cancel()`" is not
  implementable, since `Recorder` holds no session and must not; detection stays in the recorder and
  the cancel moved to a shared `enforce_breach` called by **both** `pump` and the production
  `run_turn` (`pump` is not the binary's loop, which the plan originally got wrong). **D74** —
  `agent_box.quota`/`quota_at` are now **single-writer**: `upsert_agent_box` can no longer write them
  on either path and the probe no longer carries them forward, because a probe read-modify-writing a
  column the latch owns discarded every latch in between — a lost update by construction, fixed at
  the maintainer's instruction rather than accepted. **D76** — the `default` column shows a model id
  **whole**, since the id is §4.4's selection coordinate and `gemini-3.` names nothing; the table was
  rebalanced for it and **has no width slack left** (each of eight columns now sits at its own longest
  string), which MOD-23 and MOD-12 inherit.
  `rust-reviewer` returned **no CRITICAL and no HIGH**; its four MEDIUM and six LOW were adjudicated
  — eight applied (`bf2cd2e`, `4742ce3`, `44e6743`), one **rejected on evidence** (L-5 claimed the
  `R-AGT-5` sweep does not read comments; it does — `the_installer_names_no_vendor` sweeps
  `production_half()` for four vendor strings and fired on one in a comment earlier the same day —
  the finding had read the *other* sweep, the `zeta` one), and one left with the maintainer
  (**M-2**: a `per_token` row whose `quota.source` is still `acp_meta_rate_limit` never publishes
  spend, because the H-3 rule waits for a blob that will never arrive).
  **`T46` closed and ANA-4 §11 criterion 6 now holds across a flush** (`2deb7f8`, `acec1b8`, both
  maintainer-decided after the review gate). The defect was real and live: `Recorder.edits` was
  cleared by `flush()`, so §4.3's "one `edit_proposal` row per `(tool_call_id, path)` per step" held
  only inside one flush window, and one `agy` file write left **three** rows — `agy` re-announces
  `tool_call` verbatim where the spec's example sends `tool_call_update`, and answering the
  permission prompt flushes in between. Measured from the committed fixture, the same write now
  leaves **1** row (3 → 1 with the live interleaving, 2 → 1 without). **D77** is how, and it adds no
  store seam: an `edit_proposal` **reserves its `seq` at announcement** and only its *write* waits,
  so a re-announcement updates the held row in place and the row is written when its tool call
  closes. Nothing the user sees moved — the recorder's UI frame already goes out at announcement, so
  only the database write is deferred, and replay reads by `seq`, which was never given up. Held rows
  are released on four paths (a `tool_result` or terminal `tool_call_update`, the turn's `done`, a
  cancel including the cap's `enforce_breach`, and `finish()` as the backstop), because a held row
  that was never written would be a `seq` gap — worse than the duplicate it replaced. `next_seq` now
  advances at numbering rather than at commit, since the flush is no longer the only allocator, and a
  refused batch keeps the numbers already on its rows. The three rejected options are recorded in the
  plan: a store `update_event_payload` across six impls would have forced a choice between "criterion
  6 online only" and making `upload_pending` last-line-wins, which is the rule protecting
  `UsageTotals::from_rows` from double-counting; deferring the `seq` too would have reordered the
  transcript; and suppressing byte-identical repeats would have fixed this `agy` case and none of the
  general one. The accepted price, knowingly: a process death mid-tool-call loses a proposal row that
  was durable before. **The amended conformance case is the lasting fix** — its script now flushes
  between two *differing* writes, because the fake never flushed mid-tool-call and that is precisely
  why a transport-neutral suite could not see a live defect. **D78** closed the review gate's last
  finding with it: a `per_token` row publishes its spend on the first costed row instead of waiting
  for an allowance blob that a per-token agent never sends (§7 gives it no `windows`, so the H-3
  guard had nothing to protect). 742 tests green on **Linux** with Postgres live, 46 targets.
  **Phase 8 landed (2026-09-10..11, `a3cbfff`..`42294d8`), complete:** the **degraded CLI
  transport**, planned in `.claude/plans/mod-2-cli-transport.plan.md` with the `code-architect`
  blueprint beside it. An agent that speaks only its own headless JSON stream now reaches the same
  chat tab, recorder, store rows and replay as an ACP one — losing exactly three event kinds, and
  saying so on screen. `R-AGT-3` is met.
  New: the **`claude-cli`** registry row (D79 — a second row, not a degradation inside the `claude`
  one; D88's name-keyed `seed_missing_agents` top-up rather than a migration, so **MOD-4's `0003` is
  still unheld**); `htui-agent::cli::{mod,claude}` (supervisor and §6.2 mapper); a **third**
  conformance binding; `DriverFactory::with_acp` → `production`, registering two adapters for three
  rows (D87, `R-AGT-5`).
  **§11 criterion 1 holds in fact, not by assertion**: `CASES` is still **15**, and three transports
  report all fifteen. D80 and D91 are why — six cases gained a *capability-gated second arm* rather
  than a skip, so a transport without a capability must prove the **negative** (no `permission_request`
  row, no `edit_proposal` row, one turn-end `usage` row). `DriverCaps` gained `usage_mid_turn` to say
  which. **That change immediately caught a real defect** in the place the suite is strongest:
  `tests/extensibility.rs` drives all fifteen cases over a **CLI** row, and the fake was replaying a
  script its own declared capabilities said it could not have produced. `fake.rs` gained a sixth
  harness rule — *a transport plays the wire its `DriverCaps` declare* — so all six arms execute today.
  **§11 criterion 7 proven live over the CLI row** (`crates/htui/tests/chat_live_cli.rs`, two turns,
  real binary, real store): `168710 + 16745 = 185455` = the last `cost_micros_total`, exactly.
  Criterion 11's CLI half holds on **Linux**; the Windows half is **MOD-16's** and is not claimed —
  the Windows lint target still cannot build here (TOOL-3).
  **Nine live probes ran before a line of `src/cli/` was written** (`tests/cli_live.rs`, fourteen
  committed transcripts), and their answers are the plan's **"Probe findings" F-1..F-15**. Five
  changed code that was about to be written, and three of those would have shipped silently:
  **F-4** — `modelUsage[*]` tokens are **cumulative** across a session while `result.usage` is
  per-turn, the inversion of what the names suggest, so every figure the mapper reports is a delta;
  written as planned, a two-turn chat would have double-counted into `run_step.usage` and **no test
  in the tree would have failed**. **F-7** — a turn's thinking block and its reply share one
  `message.id`, so the coalescing key is `(message.id, block index)`; the plan's `message.id` alone
  would have folded a thought into a reply. **F-13** — `quota::normalize` **discarded** the blob for
  `CliRateLimitEvent`, so D86's "drives the quota latch through the existing `normalize`" was a
  no-op; every quota test in the tree is ACP-sourced, so nothing would have caught it. Also **F-8**
  (`result.subtype: "success"` arrives with `is_error: true` on an unauthenticated run — the verdict
  is `is_error`/`terminal_reason`, never `subtype`) and **F-10** (`--max-budget-usd 0` is refused
  before stdin is read, so "no cap" must pass no flag).
  **D81 survives its own probe, with §4.4's wording corrected** (F-1..F-3): stdin close alone is an
  end-of-input and **not** a cancel (the turn completes, exit 0); SIGINT after it *does* bring a
  terminal envelope — but an error-shaped one, `subtype: error_during_execution`, `is_error: true`,
  carrying **`terminal_reason: "aborted_streaming"`**, which is what the mapper keys `cancelled` on
  so a turn cancelled from elsewhere still reports honestly; SIGTERM leaves exit **143** and no
  `result` at all. **D82 survives with its payload gone** (F-6): thinking is *signalled and not
  disclosed* here — `thinking` is the empty string beside a `signature` — so a `thought` over this
  transport carries no prose, and the signature never becomes its text.
  **D85 was proven, after failing to be** (F-12): the first denial probe asked for `Bash(ls)` and
  this box's settings carry `Bash(ls *)`, so it chose the one command the box pre-approves and read
  the empty array as though it said something about the dialect. A tenth case using
  `--permission-prompts none` makes the refusal a property of the *invocation* instead, and it
  fires. That run also caught a gap: D85 named **two** sources and only one was implemented — the
  CLI announces a refusal live as `system/permission_denied` **and** repeats it on the terminal
  `result`, so the mapper takes the live one (the denial lands where it happened) and deduplicates
  the repeat by `tool_use_id` (**F-12b**).
  **Four ANA-4 amendments, maintainer-approved and recorded here rather than in the ANA** (the
  milestone-5 precedent): **D79** — §5.3/§4.4's "one `claude` row with the CLI as an in-row
  degradation" becomes two rows, and `settings.cli` leaves the `acp` row. **D92** — §4.4's
  recommendation of `--bare` is **withdrawn**: `claude --help` on 2.1.267 says `--bare` reads auth
  *strictly* from `ANTHROPIC_API_KEY`, and the probe measured it — a `--bare` run answers
  `terminal_reason: "api_error"`, `"Not logged in · Please run /login"` on this subscription box. The
  price is a pre-`init` buffer, because hook envelopes then precede `system/init` (**F-9**, measured:
  four pairs on this box), and D84's banner must still be the step's first `other` row. **D93** —
  §4.1's `permission_answer` moves from "htui-authored only" to "authored by `htui` *or* reported by
  a transport that answered by policy"; it is a real twelfth `DriverEvent` variant, and
  `driver_contract.rs`'s identity is now `14 − 2 = 12`. **D94** — replay decodes it to that typed
  variant, closing the asymmetry D93 opened (it had been typed live and `other` on replay);
  `PermissionAnswerEvent::denied` gains `#[serde(default)]` for rows written before this milestone,
  and `record_permission_answer` writes the key from now on.
  **`docs/ANA-4.md` §11.14's two CLI items are answered from live runs**, which is what this
  milestone owed: the cancellation semantics (F-1..F-3) and the thinking-block shape (F-6), both
  recorded as committed fixtures rather than as prose.
  **Two holes in the fixtures' own redaction were found and closed**, each by a *later* probe reading
  an *earlier* one's committed transcript — an argument for keeping fixtures under test rather than
  merely under version control. The rule was applied to the serialised file, and a needle only had to
  arrive spelled differently to walk through it: **split across streaming deltas** (the model chunks
  wherever the tokeniser did, so a quoted path arrives as five JSON strings and none holds the
  needle — *and the self-check re-read the same unreassembled text*, so a green redaction check
  proved nothing about the stream), and **slugged** (the CLI derives a project directory from the
  cwd, so `/tmp/.tmpAbCdEf` is reported as `-tmp--tmpAbCdEf`; thirteen of fourteen transcripts
  carried it). What leaked was a tempdir name and a username already in every commit, so the exposure
  is nil — the hole is the same size for the `SECRET_NAMES` half, which exists so a token echoed by a
  hook cannot ride into git. Neither fix is a general defence and the code says so: a needle can be
  base64'd, URL-encoded, or split *and* slugged.
  Also landed: **D89** re-ranks the Settings agents table — `name` became `Constraint::Fill(1)`, with
  no width to tune, after measuring that the literal instruction (`Min(256)`) does not widen that
  column but **deletes the other seven** (`[91, 0, 0, 0, 0, 0, 0, 0]` at the bordered 98). `name` now
  takes every spare column — 10 at 98, 32 at 120, 112 at 200 — and `on this box` is the donor, fixed
  at 13, which clips `unauthenticated` to `unauthenticat` and `choose a method` to `choose a meth` as
  the stated price. **A correction for whoever packs this table next**: the 98 that D76's and D89's
  arithmetic both argue from is the **test harness** (`testkit.rs`'s `DEFAULT_SIZE`, 100×30, minus
  the pane border), not a screen — D76's ranking was tighter than it needed to be because it treated
  a fixture constant as a constraint.
  **Verified on Linux with Postgres live**: `cargo fmt --check` clean, workspace
  `clippy --all-targets --all-features -D warnings` clean, and
  `cargo test --workspace --all-features --no-fail-fast` **51 binaries green, 0 failed** — which is load-bearing rather than pedantic: `cargo
  test` stops at the first failing binary, and a run that reported the `auth.rs` flake and exited is
  how two width-broken suites (`install.rs`, `probe.rs`) were briefly called green.
  **Live coordinates for the next session.** `claude` on this box is **2.1.267**; the seed passes no
  `--bare`; `--permission-prompts none` is the deterministic way to provoke a policy denial, and
  `~/.claude/settings.json`'s allow list (`Bash(ls *)`) is why the obvious way does not. The CLI
  reports `claude_code_version` on `system/init` (there is no `version` key), re-emits `system/init`
  on **every turn** of a multi-turn session (the banner consumes the first; re-emitting it would
  break `session_banner_is_first_other_row` on the live path only), and emits a `system/status`
  row per turn that §6.2 does not name. **Once F-13's reader is on, this box's live blob is
  `status: "allowed_warning"` at 0.77 utilization, and `available()` skips every status that is not
  exactly `"allowed"`** — so the `claude-cli` row will report `Skip(Status("allowed_warning"))` to
  MOD-4 the moment MOD-4 has a selection loop. That is deliberately MOD-4's to loosen
  (`quota.rs:403-408`), but it is now a live fact about a real row rather than a hypothetical.
- [ ] **MOD-4 - Orchestrator, manual mode** (from ANA-2). `R-ORCH-1..5`, `R-ORCH-7..11`,
  `R-TUI-4`, `R-TUI-9`. Step graphs per kind, gates, retries, review loop, fan-out with isolation
  modes and selection, capability check, promotion to chat, run records, Runs tab actions, and
  close-out (summary document, status, commit hashes). The `run` and `close` actions of `R-TUI-2`.
  Design concluded in `docs/ANA-2.md` (ANA-2, `docs/decisions/ana/ana-2.md`): new crate
  `htui-orch` plus `crates/htui/src/run_worker.rs` (§8), migration `0003_orchestration.sql` and
  `cache_migrations/0003_orchestration.sql` (§9; never applied before MOD-2's `0002`, **which
  landed 2026-09-08 in `fb626a8`, so this constraint is now satisfied**. **The cache
  migration is `0003`, not the `0002` ANA-2 §9 reserved:** MOD-2 milestone 4 spent
  `cache_migrations/0002_agent_mirror.sql` mirroring the registry so an offline chat can start), plus
  **`local_migrations/0002_orchestration_local.sql` in the same commit** (ANA-10 §5.3's Set B — a
  `0003` landing alone leaves the local schema a generation behind, and SQLite makes catching up
  expensive), **eighteen** `WriteStore` methods (ANA-2 §8's table is 17 rows and `docs/ANA-2.md:1744`
  names two; §1760's "sixteen" undercounts its own table) plus the six `ReadStore` reads of
  `:1730-1733` and the fifteen new inherent reads of `:1711-1724`, each now obliging **three** stores
  — `MemStore`, `PgStore` and ANA-10's `LocalStore` — with conformance cases, `can_move_to` on the
  three status enums (§4.3), typed `BoxSettings`/`ProjectSettings` (§4.7), projection additions
  (§6.2), fixture corrections (`attempt` 1-based, non-NULL `graph_snapshot`, `review` in
  `implement.input_kinds`, `gate_hard` seed), `gix` as the git dependency; build order in §9,
  steps 1 to 4 need nothing from MOD-2. Per ANA-5 (`docs/ANA-5.md` §4.6, §8): calls `assemble()`
  for the judge and handoff prompts, supplies `verify_failure`/`previous_diff` for the review loop,
  `RunStepSummary` gains `prompt_tokens` and `trimmed`; the pre-flight digest write is MOD-2's
  `set_step_prompt`, not `finish_step`. Per ANA-10 (`docs/ANA-10.md` §4.8, §9.1): **local-only mode
  runs graphs at parity**, so this item also owns M8 — `LocalStore` implementations of the seam above
  and the four SQL ports (`shared_serialized`'s advisory lock becomes an in-process lock sound only
  under ANA-10 §7.1's file lock, `FOR UPDATE` becomes `BEGIN IMMEDIATE`, `&&` over `repo_scope` and
  `<@` over tag arrays become Rust predicates shared with Postgres through `htui-core`). The
  `PgStore`-inherent reads of ANA-2 §8 become `Backend`-inherent with a four-arm `match self`
  (`agents()` precedent, `backend.rs:292-298`); §8's split rule itself is unchanged. Two
  prohibitions: never `CHECK (fanout_index >= 0)` on any of the three schemas (ANA-2 risk 12), and
  no local `shared_serialized` before ANA-10 §10.17 is answered. Blocked on MOD-2 **and on MOD-17's
  M3** (ANA-10 §9.1's ordering rule: `LocalStore` must exist before build step 1, or this item
  inherits eighteen unbudgeted methods) (MOD-6 landed, `docs/decisions/mod/mod-6.md`).
- [ ] **MOD-7 - Box registry + capabilities.** `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6`, `R-TUI-8`.
  Probe, registration, capability tags and quirks editor (Settings tab box profile section),
  per-box paths, agent autodiscovery hook. Not blocked (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`: `register_box` writes the minimal row, the probe fills the rest).
  MOD-4 and MOD-12 need `probed_tags`/`declared_tags` from a real probe, `repo_box_path` rows for
  every isolation mode, and `agent_box.probe.status` (`docs/ANA-2.md` §4.10, §7). `repo_box_path`
  rows and a real probe turn ANA-5's excerpt fallback root and box profile projection
  (`docs/ANA-5.md` §4.2, §4.5) from degraded into complete. **MOD-20 landed**
  (`docs/decisions/mod/mod-20.md`): `docs/ANA-4.md` §4.6 named this item as the possible owner of
  registry-driven installation, and it is now built and `R-AGT-10`-backed - so this item *calls*
  `htui_agent::install` from its box registration and probe hook rather than growing one, and that
  hook is the natural second caller of the action `Settings > i` already exposes (MOD-20 D7 kept
  the MVP to Settings deliberately).
- [ ] **MOD-9 - Skill library and templates.** `R-SKL-1..4`, `R-PRM-4`, `R-TUI-7`. Versioned skills,
  project and phase bindings, template rows, Skills tab editor with version diff, import of
  existing skill markdown files. Per ANA-5 (`docs/ANA-5.md` §4.1, §5.4): template save validation
  calls `htui_core::prompt::template::parse`; `judge` and `handoff` are reserved names whose
  `TemplateRole` derives from the row name; the closed placeholder tables are the editor's inline
  help. Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`); the validator is MOD-2 build
  step 1, so MOD-9 must not ship the editor ahead of it (ANA-5 risk 11).
- [ ] **MOD-10 - Secret provider** (from ANA-7). `R-SEC-1..4`, `R-TUI-8`. `SecretProvider` trait,
  Infisical implementation, environment injection at run start, scrubber with exact-match and
  pattern masks, fail-closed persistence gate, Settings tab secret provider section. Blocked on
  MOD-2.
- [ ] **MOD-11 - htui MCP server.** `R-MCP-1..4`. Tools `item_link`, `item_status`,
  `document_write`, `note_add`, `box_profile`, `command_run`; per-step scoping; command queue with
  per-box class limits; per-phase exposure. Per ANA-2 (`docs/ANA-2.md` §4.2, §8, risk 11):
  `document_write` calls `WriteStore::write_document` (orchestrator-allocated version), `command_run`
  accepts class `verify` for `verify_command`, and an `item_status` request is recorded as an
  `item_note` with `via_step_id`, never a transition. The `box_profile` read tool returns ANA-5's
  box profile projection (`docs/ANA-5.md` §4.2) so the tool and the prompt section agree. Blocked
  on MOD-2, MOD-4.
- [ ] **MOD-12 - Auto mode queue runner** (from ANA-2). `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard
  gates, `R-AGT-7..8` caps, `R-TUI-8`. Ready-item selection, capability filter, concurrency with
  overlap rule, queue overlay, escalation, Settings tab caps and scheduler window section. The
  `queue` action of `R-TUI-2`. Target box stored, local execution only. Design concluded in
  `docs/ANA-2.md` (§4.10, §9): `ready_items` per ANA-9 §7.4 in full, the same `claim_run`
  admission as MOD-4, batch caps as `SUM(run_step.usage)`, escalations in the queue overlay, the
  `scheduler_window` key stored but not enforced; a graph with a `gate_hard` phase is never fully
  unattended, so `FIX`/`CLEAN`/`TOOL` are the first targets. Per ANA-10 (`docs/ANA-10.md` §4.8):
  `ready_items` must be **rewritten** for SQLite as well as implemented for Postgres — `<@` and
  `= ANY($projects)` have no SQLite spelling — which is why auto mode is not part of MOD-4's M8.
  Blocked on MOD-4.
- [ ] **MOD-13 - Backlog filters and item editing** (from MOD-1). `R-TUI-2`, `R-ENT-5`,
  `R-ENT-10..12`. Filters by status, project, capability and readiness; `new` and `edit` actions
  with the compare-and-set on `version` and the three-way divergence view (`docs/ANA-9.md` §4.2,
  §7.2), external `$EDITOR` round-trip, note thread append, hand-written documents; mint per §7.1.
  Not blocked (MOD-1 landed, `docs/decisions/mod/mod-1.md`); lands against `MemStore` through
  the `DetailRegistry` and overlay registry; `PgStore` (MOD-6, `docs/decisions/mod/mod-6.md`)
  supplies the real mint and revisions. The `touched_paths` edit path accepts and validates the
  `repo_name:glob` qualification of `docs/ANA-2.md` §4.7 (bare glob = primary repo);
  `touched_paths` is tier 1 of ANA-5's excerpt ranking (`docs/ANA-5.md` §4.5). Editing on a box
  with no server is **MOD-17's** (ANA-10 concluded; `docs/ANA-10.md` §9.3): this item must not ship
  a `new`/`edit` action reachable while the backend is `Local` or `Offline`, any local `mint_item`
  or `item_key_counter` (that is MOD-17's M6, gated on the `R-ENT-7` amendment), a compare-and-set
  view backed by anything but `PgStore`/`MemStore`, or a second copy of the local-only status word.
  **The scope line "mint per §7.1" above means ANA-9 §7.1's Postgres statement**; on a box with no
  server the mint is ANA-10 §7.3's three-statement local form, which MOD-17 owns.
- [ ] **MOD-14 - Graph tab** (from MOD-1). `R-TUI-5`, `R-ENT-9`. Item neighbourhood one to N hops
  across projects through `ReadStore::links` (`docs/ANA-9.md` §6.1), status and link kind per
  edge, keyboard navigation that re-roots the Backlog selection; the `open graph` action of
  `R-TUI-2`. Not blocked (MOD-1 landed; replaces `ui/tabs/backlog/detail/graph.rs` only).
- [ ] **MOD-15 - Workspace, project, repo and kind management** (from MOD-1). `R-ENT-1..4`,
  `R-ENT-6`, `R-BOX-4`, `R-TUI-8`. Create and edit workspaces, projects (seeded kinds, graphs and
  templates per `docs/ANA-9.md` §5.10), repos with primary flag and per-box paths, workspace root
  paths per box; item kind editor with the prefix-change warning (§10); Settings tab sections for
  kinds and step graphs per project. Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`;
  `Settings > Rebuild cache` calls `CacheStore::rebuild()`). Seed graphs per `docs/ANA-2.md` §4.1
  (`review` in `implement`/`fix` `input_kinds`, `gate_hard` on `prd`, `plan` and `verdict`,
  `is_override = false`); whichever of MOD-15 and MOD-4 lands second owns the seed amendment. Per
  ANA-5 (`docs/ANA-5.md` §5.3, §5.4, §9): seed ten `prompt_template` rows per project (eight phase
  names plus reserved `judge` and `handoff`, amending `docs/ANA-9.md` §5.10), the phase editor
  refuses the two reserved names, and the Settings tab exposes the ten `app_setting` keys. Creating
  a workspace on a box with no server is **MOD-17's** (ANA-10 concluded; `docs/ANA-10.md` §9.3,
  §9.4): "Not blocked" above holds for the server-backed paths only — the create paths for a
  server-less box depend on MOD-17. This item builds and registers the Settings connection
  **section** (`SectionId("connection")`, named by ANA-10 M0 so neither item has to guess);
  **MOD-25 owns the masked DSN field inside it** and must not have a second one built here. Also:
  no ad-hoc focus mechanism instead of M0's `TabAction::FocusSection` /
  `SettingsSection::captures_input` / `MaskedField` names; no second persistence location for a
  "shown once" marker or for local settings (ANA-10 §5.4's `local_setting` exists for that); and
  `Settings > Rebuild cache` stays as written — it is safe because the local store is a different
  file that `MIRRORED_TABLES` never names, so it must not be "helpfully" extended to clear local
  data, and its confirmation copy should say what it does and does not delete.

- [ ] **MOD-16 - Windows runtime verification of the agent driver** (from MOD-2). `R-AGT-1`,
  `R-NF-3`, `R-HIS-1`. Every Windows-only path MOD-2 compile- and lint-checked from Linux but
  never ran. `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent` is green and has already
  caught two defects a Linux build cannot see (`c4d65ba`), but four facts are runtime facts:
  the job object's kill-on-close guarantee leaves no `node`/`claude` process behind (`docs/ANA-4.md`
  §11 criterion 11's Windows half); `CreateProcess` refuses a `.cmd` shim, so `${claude}` resolving
  to one must fail with a message naming the shim rather than a bare `os error 193`;
  `CREATE_NO_WINDOW` actually suppresses the console; and milestone 4's `[H-1]` buffer sealing
  behaves under Windows rename semantics, where a sealed file held open by another process makes
  the rename fail rather than silently succeed (`seal_orphaned` must warn and carry on, and
  `seal_one`'s numbered fallback must not lose a tail). Also run the Postgres suites there: the
  cache is SQLite on a different filesystem, and `append_pending`'s `OpenOptions::append` and the
  `.jsonl.open` sealing have never met a Windows file lock. Needs a Windows box with `node` and
  `claude-agent-acp` installed; milestone 5 is the first milestone whose own work touches the
  spawn path, so this can run before or beside it. Not blocked.
  **MOD-20 added a second body of Windows-only code** (`docs/decisions/mod/mod-20.md`), and it is
  the first that could not be lint-checked from Linux at all (**TOOL-3**), so it was reviewed by eye
  only. The runtime facts it defers here, by name: that `Layout::promote`'s three-attempt
  `PermissionDenied` backoff actually clears a Defender lock on a freshly written `.exe`; long-path
  behaviour past 260 characters under `<install root>\<id>\<version>\`; that `create_link`'s
  Windows fallback (a small file naming the target, since a real symlink needs a privilege an
  ordinary user lacks) is never mistaken for the adapter; `fs4::available_space` on a junction;
  `reqwest`'s `system-proxy` reading the OS proxy configuration; and whether a partial
  `.staging/` entry can be removed while its file handle is open, which `fetch.rs`'s abandon path
  was restructured for but which no Linux test can distinguish.
  **MOD-21 added a third body of Windows-only code** (`docs/decisions/mod/mod-21.md`), unlinted for
  the same TOOL-3 reason and reviewed by eye. The runtime facts it defers here, by name: whether any
  Windows opener honours `BROWSER` at all and whether `cmd.exe /c exit 0` is a value it accepts —
  the neutraliser works only on an opener that *word-splits* the variable, and one that treats it as
  a single path falls through to exactly the stdout hijack the policy exists to stop, which was
  observed live on Linux; that `powershell.exe -NoProfile -NonInteractive -Command "Start-Process
  -FilePath $env:HTUI_OPEN_URL"` reaches the default browser under `CREATE_NO_WINDOW`
  (`ShellExecuteW` is unsafe FFI the workspace forbids, which is why the URL travels in the child's
  environment); and that the job object reaps the auth child **with its loopback listener** — MOD-2
  §11 criterion 11, now with a socket and a child that lives for minutes rather than seconds.

- [ ] **MOD-17 - Local-only mode: a writable local store** (from ANA-10).
  **Superseded by MOD-25 (maintainer decision, 2026-09-11): `htui` is online-only, so there is no
  writable local store. This line is deleted at MOD-25's close-out; nothing below it should be
  started.** `R-STO-1`, `R-STO-3..6`,
  `R-ENT-7`, `R-TUI-1`, `R-TUI-8`, `R-NF-3`, `R-ID-3`, `R-ID-7`, `R-HIS-1`, `R-USR-2`, `R-BOX-1`.
  A box that has never been given a DSN becomes a complete box rather than an empty read-only
  shell. Design concluded in `docs/ANA-10.md` (ANA-10, `docs/decisions/ana/ana-10.md`): a fourth
  `Backend::Local` arm over a **separate** file `<config_root>/local/local.sqlite` (never
  `cache.sqlite`, which is delete-on-mismatch), a new `LocalStore: WriteStore` with its own
  forward-only `local_migrations/` set, local id minting through a project-origin-scoped key
  counter, `box.toml` carrying three `#[serde(default)]` facts (no `local_only` mode flag), a
  once-only first-run overlay, and an in-app masked DSN field. **Local-only is a temporary, lite
  mode** (`docs/ANA-10.md` §1.3): parity is over *features* — hierarchy, items, chat, graph runs —
  not over server-shaped infrastructure, so there is no mirror, no refresh cursor, no
  `db_fingerprint`, no `offline · <age>` and no warm-cache startup budget on this path. Its
  first-run copy must not promise migration to a server before MOD-18 ships (§4.5's copy rule).
  Milestones **M0-M6 plus M2b** of
  `docs/ANA-10.md` §9.1; **M2b is the milestone that satisfies the item's stated objective** and
  M0+M1+M2+M2b is a complete shippable answer on its own (§9.2). Blocked on nothing, but **M3 must
  land before MOD-4's build step 1** (§9.1's ordering rule), and M6 is gated on the `R-ENT-7`
  amendment (`docs/ANA-10.md` §6.1, §10.3). MOD-13's, MOD-15's and **MOD-23's** create/edit paths
  for a server-less box depend on it; MOD-15 owns the Settings connection *section*, this item owns
  the credential *field* inside it (§9.4). The requirement amendments of §6.1 are proposed, not
  applied — `docs/REQUIREMENTS.md` is maintainer-only.
- [ ] **MOD-18 - Adoption of local rows into a server** (from ANA-10).
  **Superseded by MOD-25 (2026-09-11): with no local rows there is nothing to adopt. Deleted at
  MOD-25's close-out.** `R-STO-1`, `R-STO-7`,
  `R-ENT-7`, `R-USR-2`, `R-HIS-1`. `docs/ANA-10.md` §9.1's M7. **Funded, not deferred** (§10.5, on
  §1.3's framing): local-only is a temporary lite mode and Postgres is the full product, so this is
  the exit the mode promises rather than an optional extra. **MOD-17 must not ship first-run copy
  promising migration before this item exists** — until then the overlay says configuring a server
  does not move existing local work; when this lands, the copy gains the move. Explicit, confirmed,
  resumable; one transaction per project
  subtree; `MAX`/`GREATEST` counter fast-forward and the `sealed_at` write inside that transaction;
  `created_by`/`author_id`/`box_id` and `agent_id` remaps (§10.33); pre-adoption file copy;
  second-server refusal keyed on `system_identifier`; a UI-visible per-row status, never
  `pending.rs`'s log-only quarantine. Its unit is a project subtree including `run`, `run_step`,
  `session_event`, `document` and `command_run` rows, not only items. Blocked on MOD-17 (M6). Until
  it lands, local rows stay local and a local project is not runnable while a DSN is configured
  (`docs/ANA-10.md` §11 risk 22) — MOD-17's M3 export path is the floor. The `GREATEST` importer
  statement now has two consumers, MOD-8 and this item (`item.rs:145-148` reserves it to MOD-8).
- [ ] **MOD-19 - In-process transition out of local-only** (from ANA-10, deferred).
  **Superseded by MOD-25 (2026-09-11): there is no local-only mode to transition out of. Deleted at
  MOD-25's close-out.** `R-TUI-8`,
  `R-NF-3`. `docs/ANA-10.md` §9.1's M9 and §10.13: `connect::reconnect_for` as a public factory
  (the DSN stays inside `connect.rs`, which is `connect.rs:66-70`'s real property — the existing
  `Reconnect` closure already captures a DSN by move), `let mut reconnect` at
  `store_worker.rs:411`, and the expensive half, `go_online` learning to **open** a mirror rather
  than move one (`store_worker.rs:572-575`) while a `Backend::Local` and its open `local.sqlite`
  are still in hand. Buys one avoided restart and nothing else; MOD-17's M2b delivers the objective
  without it. Blocked on MOD-17 (M3).
- [ ] **MOD-22 - Complete a loopback OAuth login from a box the browser cannot reach** (from MOD-21).
  `R-AGT-9`, `R-TUI-8`, `R-NF-3`, `R-SEC-2`, `R-ID-7`. An agent's own login flow redirects to a
  listener **inside the adapter process**, on that box's loopback: `agy_acp_server`'s URL carries
  `redirect_uri=http://127.0.0.1:<ephemeral port>/`, a different port per attempt (39879, then
  50651, measured live on 2026-09-09). When `htui` runs on the same machine as the browser this is
  invisible. When it runs on a **server**, the browser resolves `127.0.0.1` to the *user's* machine,
  nothing is listening, and the flow cannot be completed from the app at all - which is `R-AGT-9`'s
  own sentence left unfinished. Confirmed on this maintainer's setup 2026-09-10: of the two
  workarounds, **`ssh -L` port forwarding did not work and pasting the redirect back did** -
  copying the failed `http://127.0.0.1:<port>/?code=…&state=…` out of the browser's address bar and
  re-issuing it on the box (`curl`) hands the code to the adapter and `authenticate` returns.
  Scope: that paste-back, performed **in the TUI** rather than in a second terminal - a field in
  MOD-21's login pane that accepts the redirect URL the browser could not reach, validates it
  (loopback host, the port the flow actually advertised, `code`/`state` present), issues the request
  to that port **on the box `htui` runs on**, and reports what the listener said; the login then
  completes through MOD-21's existing path, so nothing here re-implements `authenticate`, the
  re-probe, or the outcome. Needs a text-input widget the Settings tab does not have yet
  (`R-TUI-8`), off the UI task like every other request (`R-NF-3`).
  **The URL is a credential-bearing value for the length of one request** - it carries an
  authorization `code` - so `R-SEC-2`/`R-ID-7` bind: it is never logged, never persisted, never put
  on a frame that outlives the request, and the pane must not echo it back after use. This is the
  one rule this item can most easily break, and MOD-21's `auth_live.rs` module doc is the precedent
  for stating it where the code is.
  Out of scope: changing what the agent binds (the vendor's, not `htui`'s), a `redirect_uri`
  override (not offered by ACP v1), and any general port-forwarding feature. Cross-links: **MOD-21**
  owns the login flow, its pane and its frames and is the shape to extend rather than duplicate;
  **MOD-16** owns whether the same paste-back works from a Windows box; **MOD-7**'s box registry is
  where "this box is remote" would eventually be a recorded fact rather than a guess. **Not
  blocked** — MOD-21 landed (`docs/decisions/mod/mod-21.md`). Found while running its live proof on
  2026-09-10.
- [ ] **MOD-23 - Agent registry editing in the Settings agents section** (from MOD-2). `R-AGT-4`,
  `R-AGT-6`, `R-TUI-8`. Create a manual agent row and edit an existing one: transport (`acp` or
  `cli`), launch command and args, model list, default model, billing mode, the `agent` row's
  `enabled` and the per-box `agent_box.enabled`. The surface is the **`agents` section of the
  Settings tab, not a tab of its own** — `SettingsSection`/`SectionId("agents")` in the section
  strip (`crates/htui/src/ui/tabs/settings/{mod.rs,agents.rs}`), which is still the *only*
  registered section (MOD-7's box profile, MOD-9's skills and MOD-15's hierarchy each add their
  own). MOD-2 built it read-only (`r` probe, `i` install, `a` authenticate, `o` open, `x` cancel)
  and MOD-20/MOD-21 added the install and login actions, so what is missing is the **write** half
  that `R-AGT-4`'s field list and `R-AGT-6`'s "manual entries allowed" still owe. MOD-2 milestone 5
  **D45** already added `probe.source` (`probe` | `manual`) so that "a manual entry is never
  overwritten by a probe that finds nothing" (`docs/ANA-4.md` §4.6) — a guard that today protects
  rows no UI can create. Needs the same text-input widget MOD-22 needs and the Settings tab does not
  have yet (`R-TUI-8`), off the UI task like every other request (`R-NF-3`). Registry writes are
  server-only (`REGISTRY_ON_SERVER_ONLY`, MOD-6 plan D52), so the same actions on a box with no DSN
  are **blocked on MOD-17** (ANA-10's `R-AGT-4` amendment, `docs/ANA-10.md` §6.1); the Postgres path
  is **not blocked** and can start now. File collisions to expect in that one section file: MOD-2
  milestone 7 adds a quota column to this table (plan D73), MOD-12 owns the Settings caps section,
  MOD-15 owns kinds and step graphs. **Budget warning inherited from MOD-2 milestone 7 (plan D76,
  T47/T48):** the table now runs eight columns with **no width slack left** — each sits at its own
  longest string (`transport`/`models`/`enabled` at their headers, `billing` at `subscription`,
  `default` at a 21-char model id, `quota` at `100% to 09-08`, `name` at `amp-acp`, `on this box` at
  `unauthenticated`) inside 98 usable columns. A ninth column costs a **ranking decision**, not an
  adjustment; an edit *pane* below the table, which this item needs anyway for its text input, is
  the cheaper shape than more columns. Out of scope: the caps editor (MOD-12), box-profile capability
  edits (MOD-7), and anything keyed on an agent's name (`R-AGT-5`). Raised by the maintainer on
  2026-09-10 while MOD-2 milestone 7 was in flight.
- [ ] **MOD-25 - `htui` is online-only** (maintainer decision, 2026-09-11). `R-STO-1`, `R-STO-3..7`,
  `R-ID-3`, `R-ENT-7`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`, `R-SKL-1`, `R-TUI-1`, `R-TUI-8`, `R-NF-2`.
  **A box that cannot reach its configured Postgres shows a warning that the database is
  unreachable and browses its read-only cache. It does not carry a second, writable life.** This
  withdraws the verdict of ANA-10 (`docs/decisions/ana/ana-10.md`) — "a box with no DSN becomes a
  *complete* box over a separate `local.sqlite`" — which is **concluded and therefore superseded
  rather than edited**. Scope, in order: (1) `docs/REQUIREMENTS.md`, the only maintainer-owned file
  here — withdraw **`R-STO-7`** outright and restate the eleven ANA-10 amended in place on
  2026-09-08 (`R-ID-3`, `R-ENT-7`, `R-STO-1`, `R-STO-4`, `R-STO-5`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`,
  `R-SKL-1`, `R-TUI-1`, `R-TUI-8`), of which `R-STO-4`'s offline-mode text changes meaning most;
  (2) **disable, do not delete**, the offline buffered-write path MOD-2 milestone 4 shipped
  (`Writer::Buffered`, `append_pending`, `upload_pending`, `crates/htui/tests/chat_offline.rs`) —
  a chat on an unreachable box refuses with the warning instead of buffering, and the machinery
  stays in the tree for one release so a reversal costs nothing; (3) a `CLEAN-N` minted at
  **this** item's close-out to remove it once the decision has sat. **The read-only cache of
  `R-STO-3` survives**: it is also the speed cache behind `R-STO-6`'s sub-second warm start, and
  degraded read-only browsing is what the warning is shown *over*. **Supersedes MOD-17, MOD-18 and
  MOD-19**, whose checklist lines this item's close-out deletes — they are not "done" and must not
  archive as if they were; `docs/decisions/mod/mod-25.md` records them as withdrawn with their
  reason. **Unblocks by removal**: MOD-4's build step 1 (ANA-10 §9.1 required MOD-17's M3 before
  it), and MOD-13's, MOD-15's and MOD-23's create/edit paths for a server-less box, which no longer
  exist. **MOD-2's close-out must restate, not silently drop, its claim on ANA-4 §11 criterion 12**
  (an offline chat buffers and `upload_pending` lands exactly those rows): it was proven on
  2026-09-08 and is being withdrawn with the mode, which is a different sentence from "unproven".
  Not blocked. `docs/ANA-10.md` stays in the tree as the analysis that was done and not taken. **This item also owns the masked DSN field inside the Settings connection section, replacing MOD-17.**
- [ ] **MOD-24 - Fault Tolerance of Agent Processes.** Implement agent memory checkpointing to Postgres. If the daemon or TUI crashes mid-run, `htui` should be able to read the last `SessionEvent` from Postgres, re-hydrate the agent's context window, and resume the exact step it was on so that multi-hour runs can survive process restarts.

### Deferred backlog

- [ ] **MOD-3 - Diff tab + code explorer.** `R-LATER-1`. Later tier; needs its own ANA first.
- [ ] **MOD-5 - Issue tracker mirror.** `R-LATER-2`. `IssueSync` trait, OneDev first, downstream
  only. Later tier; needs its own ANA first.

- [ ] **MOD-8 - Legacy markdown import.** `R-LATER-3`. Map old prefixes to kinds per project,
  preserve keys, build links. Later tier; MOD-6 landed (`docs/decisions/mod/mod-6.md`, importer
  mint variant per ANA-9 §7.1 still to write).

### Tooling findings

- [ ] **TOOL-1 - next-item blocked-on regex counts only the first ID per phrase.**
  `next-item.ps1` / `.sh` (`$refPattern`, line ~124) match `blocked on <ID>` once, so
  comma-separated blockers (`Blocked on ANA-4, ANA-5` on MOD-2; `Blocked on ANA-7, MOD-2` on
  MOD-10, wrapping to the next line) drop every ID after the first: R2 dependent counts undercount,
  and a wrapped blocker can hide entirely. Fix both twins identically (repeat-match the ID list
  after the phrase, join across a line wrap), add a fixture with a two-ID and a wrapped case, keep
  the `WORKFLOW_ALLOW_SH_ON_WINDOWS=1` parity check green. Found during `/handoff-run next` on
  2026-09-04.
- [ ] **TOOL-3 - The Windows lint target cannot be built on this box, and MOD-20 made that bite.**
  `R-NF-3`, `R-AGT-1`. `cargo clippy --target x86_64-pc-windows-msvc` dies in `ring`'s build script
  with `error occurred in cc-rs: failed to find tool "lib.exe"`: cross-compiling `ring`'s C needs an
  MSVC-capable compiler, and this box has `gcc` only (`cc-rs` reports *"GNU compiler is not
  supported for this target"*; `AR_x86_64_pc_windows_msvc=llvm-lib` gets one step further and dies
  on *"not a COFF object"*). No `clang`, no `clang-cl`, no `cargo-xwin`, no `zig`, no `sudo`.
  **Pre-existing for `-p htui`** — `ring` reaches it as `ring ← rustls ← sqlx-core ← sqlx ←
  htui-core ← htui-store`, a path that predates MOD-20 — but MOD-20's `reqwest`/`rustls` brought it
  into `htui-agent`'s graph too, which is the crate the README's own command names
  (`README.md` "Checking the Windows-only code from Linux"). That command is the safety net MOD-2
  milestone 3 credited with catching two defects a Linux build cannot see, and it is now green on
  neither crate, so MOD-20's Windows-conditional code (`archive.rs`'s `cfg(unix)`/`cfg(not(unix))`
  arms, `Layout::promote`'s retry, the `canonicalize` on both sides of the post-promote check) was
  reviewed by eye rather than linted. Three ways out, none taken: install a C toolchain that can
  target MSVC without `sudo` (`cargo install cargo-zigbuild` + `pip install ziglang` is the
  sudo-free one; `cargo-xwin` needs `clang`); scope the lint line to a feature set that excludes
  TLS, which lints most code but not `install/http.rs`; or accept the loss and let **MOD-16** be the
  only Windows check. **This is a maintainer decision, and MOD-16 inherits the runtime half
  either way.** MOD-20's plan D21 and its Validation block both name lint lines that currently
  cannot run — whichever way this goes, they need amending. Found during MOD-20 T2 on 2026-09-09.
  being reaped before the assertion reads `/proc/<pid>`.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 2 (ANA-3 context tools, ANA-11 requirements/decisions models)              |
| MOD-N   | 24 (MOD-2 driver, MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-15 hierarchy, MOD-16 Windows verification, MOD-22 loopback paste-back, MOD-23 agent registry editing, MOD-24 fault tolerance, MOD-25 online-only, MOD-26 personas, MOD-27 swarm, MOD-28 rataflow; **superseded by MOD-25 and deleted at its close-out: MOD-17 local-only store, MOD-18 adoption, MOD-19 in-process transition**; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                        |
| TOOL-N  | 2 (TOOL-1 next-item blocked-on regex, TOOL-3 Windows lint target unbuildable) |
