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

**Current status (2026-09-06):** ANA-5 concluded (`docs/ANA-5.md`,
`docs/decisions/ana/ana-5.md`): plain `{{name}}` templates over a closed per-role placeholder set
with no templating crate, ten `<section>` names, ANA-9 §7.3 amended at query level (`MIN(depth)`,
`in_scope`), kept-first trim order with a `chars-v1` estimator and `trim_record` already in
`0001`, five-tier deterministic excerpt ranking behind an `ExcerptProvider` seam, `judge` and
`handoff` as reserved template rows, text-only sha256 digest with the scrubber before it, no new
crate (`htui-core::prompt` + `htui-agent::excerpt`), no new migration (folds into MOD-2's `0002`);
MOD-2 is now unblocked. ANA-2 concluded (`docs/ANA-2.md`, `docs/decisions/ana/ana-2.md`): graph
shape kept, three compare-and-set status tables, review loop at the same `position`, judge at
`fanout_index = -1`, per-repo `run_step_tree`, lease-based resume, migration
`0003_orchestration.sql` (after `0002`), new crate `htui-orch`; MOD-4 waits on MOD-2 only. ANA-4
concluded (`docs/ANA-4.md`, `docs/decisions/ana/ana-4.md`): `AgentDriver`/`AgentSession`,
`agent-client-protocol =2.1.0` (MSRV 1.88 in MOD-2), new `htui-agent` crate, migration
`0002_agent_probe.sql`. Live coordinates: dev Postgres via `compose.yaml` (port 5433), tests need
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5433/postgres`
(`docs/decisions/mod/mod-6.md`). MOD-2, MOD-7, MOD-9, MOD-13, MOD-14 and MOD-15 can start now.

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
  6 refuted), 10 fixed with bite-proven tests, 7 deferred (see the plan's amendments). MSRV stays
  1.85 until milestone 2, which raises it to **1.98** per maintainer override of ANA-4 §4.2's 1.88
  (`sqlx-core 0.9.0` already floors at 1.94; plan X9). Milestone 2 (T7-T10: MSRV + dependency set,
  launch/spawn, seed rows, `DriverFactory` + the `R-AGT-5` proof, Settings agent section) is next.
- [ ] **MOD-4 - Orchestrator, manual mode** (from ANA-2). `R-ORCH-1..5`, `R-ORCH-7..11`,
  `R-TUI-4`, `R-TUI-9`. Step graphs per kind, gates, retries, review loop, fan-out with isolation
  modes and selection, capability check, promotion to chat, run records, Runs tab actions, and
  close-out (summary document, status, commit hashes). The `run` and `close` actions of `R-TUI-2`.
  Design concluded in `docs/ANA-2.md` (ANA-2, `docs/decisions/ana/ana-2.md`): new crate
  `htui-orch` plus `crates/htui/src/run_worker.rs` (§8), migration `0003_orchestration.sql` and
  `cache_migrations/0002_orchestration.sql` (§9; never applied before MOD-2's `0002`), sixteen
  `WriteStore` methods and the `PgStore` reads of §8 with conformance cases, `can_move_to` on the
  three status enums (§4.3), typed `BoxSettings`/`ProjectSettings` (§4.7), projection additions
  (§6.2), fixture corrections (`attempt` 1-based, non-NULL `graph_snapshot`, `review` in
  `implement.input_kinds`, `gate_hard` seed), `gix` as the git dependency; build order in §9,
  steps 1 to 4 need nothing from MOD-2. Per ANA-5 (`docs/ANA-5.md` §4.6, §8): calls `assemble()`
  for the judge and handoff prompts, supplies `verify_failure`/`previous_diff` for the review loop,
  `RunStepSummary` gains `prompt_tokens` and `trimmed`; the pre-flight digest write is MOD-2's
  `set_step_prompt`, not `finish_step`. Blocked on MOD-2 (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`).
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
  unattended, so `FIX`/`CLEAN`/`TOOL` are the first targets. Blocked on MOD-4.
- [ ] **MOD-13 - Backlog filters and item editing** (from MOD-1). `R-TUI-2`, `R-ENT-5`,
  `R-ENT-10..12`. Filters by status, project, capability and readiness; `new` and `edit` actions
  with the compare-and-set on `version` and the three-way divergence view (`docs/ANA-9.md` §4.2,
  §7.2), external `$EDITOR` round-trip, note thread append, hand-written documents; mint per §7.1.
  Not blocked (MOD-1 landed, `docs/decisions/mod/mod-1.md`); lands against `MemStore` through
  the `DetailRegistry` and overlay registry; `PgStore` (MOD-6, `docs/decisions/mod/mod-6.md`)
  supplies the real mint and revisions. The `touched_paths` edit path accepts and validates the
  `repo_name:glob` qualification of `docs/ANA-2.md` §4.7 (bare glob = primary repo);
  `touched_paths` is tier 1 of ANA-5's excerpt ranking (`docs/ANA-5.md` §4.5).
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
  refuses the two reserved names, and the Settings tab exposes the ten `app_setting` keys.

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
| ANA-N   | 2 (ANA-3 context tools, ANA-7 secrets)                                              |
| MOD-N   | 13 (MOD-2 driver, MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-15 hierarchy; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                         |
| TOOL-N  | 2 (TOOL-1 next-item blocked-on regex, TOOL-2 demo fixture username collision)              |
