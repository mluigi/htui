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

**Current status (2026-09-05):** ANA-4 concluded (`docs/ANA-4.md`,
`docs/decisions/ana/ana-4.md`): `AgentDriver`/`AgentSession` traits, `agent-client-protocol =2.1.0`
(MSRV moves to 1.88 in MOD-2), `claude` via `claude-agent-acp`, `agy` via `agy_acp_server`, new
`htui-agent` crate, migration `0002_agent_probe.sql`; MOD-2 now waits on ANA-5 only. MOD-6 landed
(`docs/decisions/mod/mod-6.md`): `htui-store` crate with `PgStore`, the SQLite mirror and refresher,
keyring DSN, `Backend::{Online, Offline}`; dev Postgres via `compose.yaml` (port 5433), tests need
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5433/postgres`. MOD-1 landed
(`docs/decisions/mod/mod-1.md`): workspace, `htui-core` store seam, `htui` shell. ANA-2
(orchestration) gates the orchestrator. MOD-7, MOD-9, MOD-13, MOD-14 and MOD-15 can start now.

---

## Open items

### Analyses

- [ ] **ANA-2 - Orchestration design.** `R-ORCH-1..11`, `R-ENT-6`, `R-ENT-8`. Settle the step
  graph data shape, phase contract (inputs, output document kind, gate, retry, verification), status
  transitions, review-to-implement loop, fan-out selection (human or judge), isolation modes
  including `copy` and `local` semantics, overlap rule for concurrent items, promotion to chat and
  resume. Output: `docs/ANA-2.md`. Gates MOD-4, MOD-12.
- [ ] **ANA-5 - Prompt assembly and file excerpt selection.** `R-PRM-1..4`, `R-ID-5`. Settle the
  template placeholder contract, the upstream-summary walk within workspace bounds, the trim order
  and budget accounting, and how `htui` selects file excerpts without an external tool (external
  providers are `R-LATER-7`). Output: `docs/ANA-5.md`. Gates MOD-2 prompt builder.
- [ ] **ANA-7 - Secret provider and scrubbing.** `R-SEC-1..4`, `R-ID-7`. Settle Infisical SDK vs
  CLI, machine identity bootstrap per box, keyring usage for `htui`'s own credentials, scrub mask
  construction and the fail-closed check. Output: `docs/ANA-7.md`. Gates MOD-10.
- [ ] **ANA-3 - External context tools (later tier).** `R-LATER-7`. Headroom, Serena, Graphify and
  structural diff as optional excerpt providers for the prompt builder, fail-open when absent.
  Deferred until MOD-2 and ANA-5 land. Output: `docs/ANA-3.md`.

### Next features

- [ ] **MOD-2 - Agent driver + chat tab** (from ANA-4). `R-AGT-1..8`, `R-TUI-6`, `R-TUI-8`,
  `R-HIS-1..2`. ACP client, CLI adapter, agent registry with its Settings tab section (agents and
  quota), autodiscovery, quota tracking, streamed chat tab with follow-ups and inline permissions,
  event persistence and replay. Prompt builder per ANA-5. Driver design concluded in
  `docs/ANA-4.md` (ANA-4, `docs/decisions/ana/ana-4.md`). Blocked on ANA-5.
- [ ] **MOD-4 - Orchestrator, manual mode** (from ANA-2). `R-ORCH-1..5`, `R-ORCH-7..11`,
  `R-TUI-4`, `R-TUI-9`. Step graphs per kind, gates, retries, review loop, fan-out with isolation
  modes and selection, capability check, promotion to chat, run records, Runs tab actions, and
  close-out (summary document, status, commit hashes). The `run` and `close` actions of `R-TUI-2`.
  Blocked on ANA-2, MOD-2 (MOD-6 landed, `docs/decisions/mod/mod-6.md`).
- [ ] **MOD-7 - Box registry + capabilities.** `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6`, `R-TUI-8`.
  Probe, registration, capability tags and quirks editor (Settings tab box profile section),
  per-box paths, agent autodiscovery hook. Not blocked (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`: `register_box` writes the minimal row, the probe fills the rest).
- [ ] **MOD-9 - Skill library and templates.** `R-SKL-1..4`, `R-PRM-4`, `R-TUI-7`. Versioned skills,
  project and phase bindings, template rows, Skills tab editor with version diff, import of
  existing skill markdown files.
  Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`).
- [ ] **MOD-10 - Secret provider** (from ANA-7). `R-SEC-1..4`, `R-TUI-8`. `SecretProvider` trait,
  Infisical implementation, environment injection at run start, scrubber with exact-match and
  pattern masks, fail-closed persistence gate, Settings tab secret provider section. Blocked on
  ANA-7, MOD-2.
- [ ] **MOD-11 - htui MCP server.** `R-MCP-1..4`. Tools `item_link`, `item_status`,
  `document_write`, `note_add`, `box_profile`, `command_run`; per-step scoping; command queue with
  per-box class limits; per-phase exposure. Blocked on MOD-2, MOD-4.
- [ ] **MOD-12 - Auto mode queue runner** (from ANA-2). `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard
  gates, `R-AGT-7..8` caps, `R-TUI-8`. Ready-item selection, capability filter, concurrency with
  overlap rule, queue overlay, escalation, Settings tab caps and scheduler window section. The
  `queue` action of `R-TUI-2`. Target box stored, local execution only. Blocked on MOD-4.
- [ ] **MOD-13 - Backlog filters and item editing** (from MOD-1). `R-TUI-2`, `R-ENT-5`,
  `R-ENT-10..12`. Filters by status, project, capability and readiness; `new` and `edit` actions
  with the compare-and-set on `version` and the three-way divergence view (`docs/ANA-9.md` §4.2,
  §7.2), external `$EDITOR` round-trip, note thread append, hand-written documents; mint per §7.1.
  Not blocked (MOD-1 landed, `docs/decisions/mod/mod-1.md`); lands against `MemStore` through
  the `DetailRegistry` and overlay registry; `PgStore` (MOD-6, `docs/decisions/mod/mod-6.md`)
  supplies the real mint and revisions.
- [ ] **MOD-14 - Graph tab** (from MOD-1). `R-TUI-5`, `R-ENT-9`. Item neighbourhood one to N hops
  across projects through `ReadStore::links` (`docs/ANA-9.md` §6.1), status and link kind per
  edge, keyboard navigation that re-roots the Backlog selection; the `open graph` action of
  `R-TUI-2`. Not blocked (MOD-1 landed; replaces `ui/tabs/backlog/detail/graph.rs` only).
- [ ] **MOD-15 - Workspace, project, repo and kind management** (from MOD-1). `R-ENT-1..4`,
  `R-ENT-6`, `R-BOX-4`, `R-TUI-8`. Create and edit workspaces, projects (seeded kinds, graphs and
  templates per `docs/ANA-9.md` §5.10), repos with primary flag and per-box paths, workspace root
  paths per box; item kind editor with the prefix-change warning (§10); Settings tab sections for
  kinds and step graphs per project. Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`;
  `Settings > Rebuild cache` calls `CacheStore::rebuild()`).

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

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 4 (ANA-2 orchestration, ANA-3 context tools, ANA-5 prompt assembly, ANA-7 secrets) |
| MOD-N   | 13 (MOD-2 driver, MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-15 hierarchy; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                         |
| TOOL-N  | 1 (TOOL-1 next-item blocked-on regex)                                                     |
