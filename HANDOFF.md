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

**Current status (2026-09-09):** MOD-2 milestone 6 landed and is reviewed (`acf16f7`), one task
short: **`agy` works over ACP**, Google's `agy_acp_server` is installed on this box, and ANA-4 §11
criterion 10 is proven live. The driver now spawns what the probe recorded, which is the only path
carrying `agy`'s Linux-only `--uid=` — an argument the live run proved is **mandatory**, not
cosmetic. Outstanding: the live `agy` **chat** needs the maintainer to authenticate
`agy_acp_server` through the vendor's own flow (`$GEMINI_HOME/antigravity-acp/`, separate from the
`agy` CLI's login), and three ANA-4 §11.14 items wait on it. Milestone 5 landed before it
(`fb626a8`): a box **answers for itself** — `Settings > r` probes what is installed, records
versions and per-box enablement in a new `agent_box.probe` snapshot, and ANA-4 §11 criterion 9 was
proven live here.
**Migration `0002_agent_probe.sql` exists, so MOD-4's `0003_orchestration.sql` is no longer held**
(`docs/ANA-2.md` §9), and it carries ANA-5 §9's ten `app_setting` defaults with it. Milestone 4
landed before it (`81d247b`): durable history, an offline chat buffered to
`<cache_dir>/pending/` and uploaded on the next connection, and read-only replay of any past step.
Milestone 3 (`a142fbf`..`682a423`) holds the **live streamed conversation with `claude` over ACP**
in the Chat tab, against adapter 0.48.0. ANA-10 concluded
(`docs/ANA-10.md`, `docs/decisions/ana/ana-10.md`), with its gating decisions taken the same day:
a box with no DSN becomes a **complete** box, not a read-only shell — a fourth `Backend::Local` arm
over a **separate** `<config_root>/local/local.sqlite` (never `cache.sqlite`, which is
delete-on-mismatch), a `LocalStore: WriteStore` with its own forward-only `local_migrations/` set,
local item minting through a project-origin-scoped key counter, **graph runs at parity** with a
server-backed box, an **in-app masked DSN field** (the item's stated objective), a once-only
first-run overlay, `box.toml` holding three facts and **no `local_only` mode flag**, and adoption
kept explicit and deferred. Spawns MOD-17 (implementation, M0-M6 + M2b), MOD-18 (adoption, M7) and
MOD-19 (in-process transition, M9); MOD-4 gains M8 and is now blocked on MOD-17's M3.
Local-only is a **temporary, lite mode**: the full product is `htui` against Postgres, the path to
online usage must exist (so MOD-18 is funded, not optional), and parity is over features rather than
over server-shaped infrastructure — no mirror, no refresh cursor, no warm-cache budget.
**`docs/REQUIREMENTS.md` was amended for it on 2026-09-08** by maintainer decision: new `R-STO-7`
(local-only mode), and `R-STO-1` s.1, `R-STO-4` (one word), `R-ENT-7`, `R-ID-3`, `R-HIS-1`,
`R-STO-5`, `R-TUI-1`, `R-TUI-8`, `R-AGT-4`, `R-PRM-4` and `R-SKL-1` amended in place; `R-STO-6`
deliberately left alone. ANA-5 concluded (`docs/ANA-5.md`,
`docs/decisions/ana/ana-5.md`): plain `{{name}}` templates over a closed per-role placeholder set
with no templating crate, ten `<section>` names, ANA-9 §7.3 amended at query level (`MIN(depth)`,
`in_scope`), kept-first trim order with a `chars-v1` estimator and `trim_record` already in
`0001`, five-tier deterministic excerpt ranking behind an `ExcerptProvider` seam, `judge` and
`handoff` as reserved template rows, text-only sha256 digest with the scrubber before it, no new
crate (`htui-core::prompt` + `htui-agent::excerpt`), no new migration (folds into MOD-2's `0002`);
MOD-2 is now unblocked. ANA-2 concluded (`docs/ANA-2.md`, `docs/decisions/ana/ana-2.md`): graph
shape kept, three compare-and-set status tables, review loop at the same `position`, judge at
`fanout_index = -1`, per-repo `run_step_tree`, lease-based resume, migration
`0003_orchestration.sql` (after `0002`), new crate `htui-orch`. Live coordinates: dev Postgres via
`compose.yaml` (port 5439), tests need
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres`
(`docs/decisions/mod/mod-6.md`). MOD-2, MOD-7, MOD-9, MOD-13, MOD-14, MOD-15 and MOD-17 can start
now.

---

## Open items

### Analyses

- [ ] **ANA-7 - Secret provider and scrubbing.** `R-SEC-1..4`, `R-ID-7`. Settle Infisical SDK vs
  CLI, machine identity bootstrap per box, keyring usage for `htui`'s own credentials, scrub mask
  construction and the fail-closed check. Output: `docs/ANA-7.md`. Gates MOD-10.
- [ ] **ANA-3 - External context tools (later tier).** `R-LATER-7`. Headroom, Serena, Graphify and
  structural diff as optional excerpt providers for the prompt builder, fail-open when absent.
  Deferred until MOD-2 lands. The seam is fixed by ANA-5 (`docs/ANA-5.md` §4.5,
  `docs/decisions/ana/ana-5.md`): `ExcerptProvider` in `htui-core::prompt::excerpt`, propose-only,
  fail-open with a deadline, read-only (`R-ID-4`), non-LLM (`R-ID-6`); Serena's default `.serena/`
  directory must be configured outside every `repo_box_path`. Output: `docs/ANA-3.md`.
### Next features

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
  **What milestone 6 still owes** (`T34`, blocked on a maintainer action, not on code): the live
  `agy` chat. `agy_acp_server` keeps its credentials in `$GEMINI_HOME/antigravity-acp/`, a sibling
  of and separate from the `agy` CLI's own directory, so the CLI's login does not count and `htui`
  cannot log in non-interactively; until the maintainer authenticates the server itself, `session/new`
  is refused and **three ANA-4 §11.14 items stay open** — whether `agy_acp_server` emits
  `usage_update` and in what field, whether it issues `session/request_permission` in `default` mode
  and with what option ids, and whether its edits arrive as a standard `tool_call` + `diff` or in a
  vendor shape. Two of the six closed here: the `.par` mechanics (the `--uid=` finding above;
  `localharness_external` ships beside the server on Linux too and the handshake does **not** need
  it, checked by moving it aside) and the `session/new` model list, which is **not learnable while
  unauthenticated**, so D64 leaves `models: []` and `model_config_id: null` as seeded. Milestone 7
  (quota and caps) can start without any of this.
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
  (`docs/ANA-5.md` §4.2, §4.5) from degraded into complete.
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
  ANA-7, MOD-2.
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
  **MOD-17 owns the masked DSN field inside it** and must not have a second one built here. Also:
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

- [ ] **MOD-17 - Local-only mode: a writable local store** (from ANA-10). `R-STO-1`, `R-STO-3..6`,
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
  amendment (`docs/ANA-10.md` §6.1, §10.3). MOD-13's and MOD-15's create/edit paths for a
  server-less box depend on it; MOD-15 owns the Settings connection *section*, this item owns the
  credential *field* inside it (§9.4). The requirement amendments of §6.1 are proposed, not
  applied — `docs/REQUIREMENTS.md` is maintainer-only.
- [ ] **MOD-18 - Adoption of local rows into a server** (from ANA-10). `R-STO-1`, `R-STO-7`,
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
- [ ] **MOD-19 - In-process transition out of local-only** (from ANA-10, deferred). `R-TUI-8`,
  `R-NF-3`. `docs/ANA-10.md` §9.1's M9 and §10.13: `connect::reconnect_for` as a public factory
  (the DSN stays inside `connect.rs`, which is `connect.rs:66-70`'s real property — the existing
  `Reconnect` closure already captures a DSN by move), `let mut reconnect` at
  `store_worker.rs:411`, and the expensive half, `go_online` learning to **open** a mirror rather
  than move one (`store_worker.rs:572-575`) while a `Backend::Local` and its open `local.sqlite`
  are still in hand. Buys one avoided restart and nothing else; MOD-17's M2b delivers the objective
  without it. Blocked on MOD-17 (M3).
- [ ] **MOD-20 - Registry-driven adapter install** (from MOD-2 milestone 6). **`R-AGT-10`**,
  `R-AGT-4..6`, `R-TUI-8`, `R-NF-3`. `htui` installs an ACP agent's adapter itself, from the ACP
  registry, for **any** agent that declares how — never one code path per vendor. `R-AGT-10` was
  added to `docs/REQUIREMENTS.md` on 2026-09-09 by maintainer decision, and carries the digest rule
  (verify where the source publishes one, say so plainly where it does not) and the licence rule
  (surface a proprietary adapter's terms before fetching it) that this item's open questions raised.
  **It reverses a deferral by the same decision:** `docs/ANA-4.md` §4.6 rejected "download every agent from the
  ACP registry and manage the install (Zed's approach)" with "MOD-2 should not become a package
  manager", reserving the shape in `agent.launch.discovery` for "MOD-7 or a later MOD"; MOD-2's
  plan D57 restated it, and the README's hand-written `curl` for `agy_acp_server` is the cost of
  that deferral — a documented URL pinning **one** version (`1.1.1`) and **one** platform, which
  goes stale silently. The seam it fills is already shaped: `agent.launch.discovery` is a per-agent
  recipe, the glob tier resolves `…/agents/<id>/*/…` with the version segment already a wildcard,
  and the probe already records what `initialize` reports rather than what anyone assumed - so an
  installer adds a *source* for the file the glob finds, and changes no resolution rule.
  Scope: an `install` block in `discovery` naming the registry id and this platform's archive; a
  reader for `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`; download,
  unpack and executable-bit into the per-platform install root the seeds already glob; a
  Settings-tab action beside `r` (`R-TUI-8`) that never runs on the UI task (`R-NF-3`); and a
  re-probe of the row it just filled. Extensibility is the point, so nothing may be keyed on an
  agent's name (`R-AGT-5`): a new agent stays one registry row plus, now, a declared install
  source.
  **Open questions this item must settle before it writes code** - it is unrouted, and the count
  alone argues for the PRD path: the registry publishes **no checksum for every agent** (`amp-acp`
  carries `sha256` per platform, `antigravity-acp` carries none), so the trust policy for an
  unverifiable 682 MB download is undecided; these artifacts are large (`agy_acp_server.par` is
  1.88 GB unpacked, 2.0 GB with its sibling), so an install root needs a disk budget and a policy
  for old versions rather than unbounded accumulation; update policy has to agree with the 24 h
  `PROBE_TTL` and with MOD-2's `newest()`, which orders matches by **mtime** and therefore by the
  *archive's build date* (an `unzip` preserves it), so re-downloading an older version after a
  newer one selects the wrong install; `HTUI_TOOL_<NAME>` must keep winning over anything installed;
  Windows needs its own unpack path (long paths, Defender, the `.cmd` shim rule of ANA-4 §4.6) and
  its runtime half is **MOD-16's**; a partial or interrupted unpack must not leave a directory the
  glob will happily resolve; a box behind a proxy or with no network must degrade to today's
  manual instructions; and — the one that is not technical — `antigravity-acp` is **proprietary**
  with its own terms (`docs/ANA-4.md` §10 risk 8), so `htui` fetching it on the user's behalf must
  surface licence and account-type consequences *before* the download, not after.
  Not blocked. Cross-links: **MOD-7** was ANA-4's candidate owner and instead *calls* this - its
  box registration and probe hook are the natural trigger; **MOD-2** owns the glob, `newest()` and
  the `agent_box.probe` snapshot this writes into, and its README section is what this item
  replaces; **MOD-16** owns every Windows runtime fact.
- [ ] **MOD-21 - In-app agent authentication** (from MOD-2 milestone 6). **`R-AGT-9`**, `R-AGT-1`,
  `R-AGT-4..6`, `R-TUI-8`, `R-NF-3`, `R-SEC-2`, `R-ID-7`. An agent that reports `unauthenticated` is logged in
  **from inside `htui`**, not by leaving the app for a vendor CLI. **This reverses a design
  statement by maintainer decision (2026-09-09):** `docs/ANA-4.md` §4.5 concluded "`htui` cannot log
  in non-interactively; a probe that gets a valid `initialize` with a non-empty `authMethods` and no
  token is recorded as *installed-but-unauthenticated* and the agent is left disabled on that box
  until the user authenticates through the vendor's own flow." Milestone 6 made that state visible
  and honest — the probe records it, and the agent's own refusal now reaches the user instead of
  being swallowed — but visible is not actionable, and today the only cure is a vendor login `htui`
  neither performs nor explains.
  The protocol already carries the call: ACP v1 has `authenticate`
  (`agent-client-protocol-schema-1.7.0/src/v1/agent.rs:295`, `AuthenticateRequest`), which is how
  Zed logs an agent in, and `agy_acp_server`'s own refusal names it first
  ("call the `authenticate` method (supports `oauth-personal`, `gemini-api-key`,
  `agent-platform`)"). The inputs are already stored: `probe.handshake.auth_methods` records the
  ids in the order the agent listed them, and `agentCapabilities.auth.logout` says whether logging
  *out* is offered — `agy` advertises it. So this item adds a driver call, a Settings action beside
  `r`, and the transitions around them; it invents no vocabulary.
  Scope: an `authenticate` on the driver seam (`R-AGT-1`), capability-gated so the CLI transport,
  which has no such call, refuses rather than pretends; a chooser when a row offers several methods;
  the flow's own progress surfaced in the TUI (`R-TUI-8`) without blocking the UI task (`R-NF-3`);
  a re-probe on success so `unauthenticated` becomes `ready` by the same path that decided it; and
  logout where the agent advertises it. **`htui` must still never read or store the credential** -
  MOD-2's D59 rule stands (existence check only, tier name recorded, value never touched); the
  vendor keeps its own token in its own directory, and this item only *triggers* the flow.
  **Open questions this item must settle** - unrouted, and the list argues for the PRD path:
  what an agent actually *does* during `authenticate` is unverified for both agents (does
  `agy_acp_server` open a browser itself, print a URL on stderr, or block until a redirect?), and a
  full-screen TUI has nowhere to put a URL a user must click, so the hand-off needs deciding
  (render it, or `xdg-open`/`open`/`ShellExecute` it, with the Windows half **MOD-16's**);
  the call has no documented timeout or cancellation semantics in v1 and an OAuth round trip is
  human-paced, so it cannot use `HANDSHAKE_TIMEOUT`; the API-key methods are not a browser flow at
  all (`gemini-api-key` wants `GEMINI_API_KEY`, `agent-platform` wants `GOOGLE_API_KEY` or
  application-default credentials) and injecting those is **MOD-10's** secret provider, so this item
  must call that seam rather than grow a second environment mechanism; authentication is a fact
  about a *box*, not about the registry row, so nothing here may write `agent`; and a failed or
  abandoned flow must leave the row exactly as it found it.
  **`R-AGT-9` was added to `docs/REQUIREMENTS.md` on 2026-09-09 by maintainer decision**, so this
  item is requirement-backed rather than proposing one: an agent reporting itself installed but
  unauthenticated is authenticated from the app, through the agent's own protocol, with `htui`
  triggering the flow and never reading, holding or storing the credential, and authentication a
  fact about a box rather than a registry row.
  Not blocked, and **MOD-2 is not blocked on it** - milestone 6's T34 needs only a logged-in server,
  by any means. Cross-links: **MOD-2** owns the probe, the credential tier and the status this acts
  on; **MOD-10** owns every credential *value*; **MOD-20** is the other half of the same story
  (install the adapter, then log it in - one "make this box ready" flow); **MOD-7** owns the
  Settings box profile the action sits in; **MOD-16** owns the Windows runtime facts.

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
- [ ] **TOOL-2 - Demo fixture's `app_user.name` collides with the OS username, failing every
  Postgres test.** `R-NF-3`. `crates/htui-core/src/fixtures.rs:351` seeds `app_user.name =
  "luigi"`, and `PgStore::seed_if_empty` derives the same name from the OS `USERNAME`, so
  `common::demo_db()` hits `Constraint("app_user_name_key: duplicate key value violates unique
  constraint \"app_user_name_key\"")` and every `--features demo` Postgres suite fails on any box
  whose user is named `luigi`. Workaround in use: prefix `USERNAME=htui-ci`. This is worse than an
  inconvenience - the failure mode is a *pass* elsewhere, because the suites' skip guard is an
  early `return` that still reports `ok` when `HTUI_TEST_DATABASE_URL` is unset, so a run can look
  green while proving nothing (it fooled MOD-2's reviewer, which reviewed the new SQL against
  `.sqlx` alone and assumed the Postgres half passed). Fix: have `demo_db()` seed under a name the
  fixture cannot hold (or parameterise the fixture's user), and consider making the skip path
  distinguishable from a real pass. Found during MOD-2 milestone 1 on 2026-09-07; predates MOD-2.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 2 (ANA-3 context tools, ANA-7 secrets)                                                    |
| MOD-N   | 19 (MOD-2 driver, MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-15 hierarchy, MOD-16 Windows verification, MOD-17 local-only store, MOD-18 adoption, MOD-19 in-process transition, MOD-20 adapter install, MOD-21 in-app auth; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                         |
| TOOL-N  | 2 (TOOL-1 next-item blocked-on regex, TOOL-2 demo fixture username collision)              |
