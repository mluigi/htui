# htui - Requirements

**Owner:** Luigi Marrandino
**Status:** approved draft, 2026-09-03; amended 2026-09-08 by maintainer decision on ANA-10
(`docs/ANA-10.md` §6.1, `docs/decisions/ana/ana-10.md`) — R-STO-7 added; R-ID-3, R-ENT-7, R-STO-1,
R-STO-4, R-STO-5, R-HIS-1, R-AGT-4, R-PRM-4, R-SKL-1, R-TUI-1 and R-TUI-8 amended in place;
amended 2026-09-09 by maintainer decision during MOD-2 milestone 6 — R-AGT-9 added (in-app agent
authentication, reversing `docs/ANA-4.md` §4.5; implemented as MOD-21), R-AGT-10 added
(in-app adapter installation, reversing `docs/ANA-4.md` §4.6; implemented as MOD-20), and R-AGT-4's
"`agy` over ACP is unverified" clause withdrawn as settled by ANA-4 and proven by MOD-2 milestone 6;
amended 2026-09-11 by maintainer decision on MOD-25 withdrawing ANA-10 — R-STO-7 withdrawn,
and previous 11 ANA-10 amendments reverted to online-only requirements;
amended 2026-09-14 by maintainer decision — R-MCP-2 added spawn_subagent, R-AGT-4 added claude-cli.
**Governed by:** `.claude/rules/workflow-docs.md`

This file is the product requirements for `htui`. It sits above every `ANA-N` analysis and every
`MOD-N` implementation item: analyses decide *how* a requirement is met, implementation items cite
the requirement IDs they satisfy. `CONCEPTS.md` is the standing-invariant distillation of this file.

`docs/ANA-1.md` and `docs/ANA-8.md` predate this document and are superseded by it where they
conflict. Their verdicts survive only where restated here.

## How to read

- **ID scheme:** `R-<AREA>-<N>`. IDs are stable; a withdrawn requirement keeps its number and is
  marked withdrawn rather than deleted.
- **Priority:** `must` ships in version one. `later` is designed for, not built first: version one
  must not make it harder, and may reserve schema or trait shape for it.
- One or two sentences per requirement. Rationale only where the choice is not obvious.

---

## 1. Identity (R-ID)

- **R-ID-1 (must).** `htui` is a cross-platform Rust terminal application (`ratatui`, `crossterm`,
  `tokio`) that wraps AI coding agents to run a backlog of work items through configurable step
  graphs across projects, repositories and machines.
- **R-ID-2 (must).** `htui` is not an IDE, not a code editor, not a terminal multiplexer, and not a
  cloud-hosted service. It is a local-first, developer-guided harness.
- **R-ID-3 (must).** Postgres is the single source of truth.
  Everything `htui` knows lives there: items, documents, runs, transcripts, skills, templates, box
  profiles, agent registry, settings.
- **R-ID-4 (must).** `htui` writes no files into managed repositories. The only changes to a
  working tree are made by agents doing the item's work. Rationale: keeps repos clean and avoids
  merge conflicts on generated state.
- **R-ID-5 (must).** Agents run with their own system skills disabled. `htui` owns every prompt and
  every skill text and inlines them into the initial prompt, so behavior is identical on every box
  and agents never spend turns reading instruction files.
- **R-ID-6 (must).** No LLM agent is ever in a sync, cache, import or bookkeeping path. Those are
  deterministic code.
- **R-ID-7 (must).** Any transcript or tool output is scrubbed of secrets on the host box before it
  is persisted or transmitted. Scrubbing fails closed.

## 2. Users and boxes (R-USR, R-BOX)

- **R-USR-1 (must).** Version one serves a single developer working from several machines.
- **R-USR-2 (must).** A `user` entity exists from day one with a single row. Boxes belong to a user.
  Items, notes, revisions, runs and documents carry `created_by`. No login, roles or permissions in
  version one.
- **R-USR-3 (later).** Team use: several users on shared projects, per-user Postgres credentials,
  visibility of who runs what, roles. Version one must not block this.
- **R-BOX-1 (must).** First launch on a machine registers it as a `box` under the current user:
  hostname, OS family and version, architecture, CPU, RAM, GPU presence and vendor.
- **R-BOX-2 (must).** The box probe records installed toolchains and tools with versions:
  compilers (`rustc`, `cargo`, `gcc`, `clang`, `cl`), build tools (`cmake`, `ninja`, `vcpkg`),
  shells, container runtime, and installed agents. Re-probe on demand and when `htui` version
  changes. `last_seen` updated per session.
- **R-BOX-3 (must).** Box carries capability tags, both probed and hand-declared, plus a free-form
  quirks note editable in the TUI. Seeded vocabulary: `gpu`, `vulkan`, `msvc`, `mingw`, `clang`,
  `cmake`, `vcpkg`, `docker`, `rust`, `heavy_build`. Vocabulary is open.
- **R-BOX-4 (must).** Each box holds per-box paths for every repo and workspace root it has checked
  out. Logical identity (UUID, slug) never depends on a filesystem path.

## 3. Entity model (R-ENT)

- **R-ENT-1 (must).** Three-tier hierarchy: `workspace` groups projects, `project` owns work,
  `repo` is a git checkout.
- **R-ENT-2 (must).** `workspace`: named, explicit group of one or more projects for cross-repo
  work, with a per-box root path. There are no implicit workspace rows: opening a single project
  runs with no workspace, and every workspace-scoped feature falls back to "the current project".
- **R-ENT-3 (must).** `project`: owns the item key space, the item kinds, step graphs, skill
  bindings, secret provider binding and phase defaults. Has one or more repos, one marked primary.
- **R-ENT-4 (must).** `repo`: a git repository belonging to exactly one project, with remote URL,
  default branch and per-box local path.
- **R-ENT-5 (must).** `item`: key (`<PREFIX>-<N>`, unique per project), kind, title, body, status,
  required capability tags, `created_by`, `version` for optimistic locking, timestamps. Body is
  markdown edited in the TUI or an external editor.
- **R-ENT-6 (must).** Item kinds are configurable per project. Each kind has a key prefix, a
  description and a default step graph. Seeded set on project creation:

  | Prefix  | Kind      | Default graph                        |
  |---------|-----------|--------------------------------------|
  | `ANA`   | analysis  | research, verdict                    |
  | `FEAT`  | feature   | prd, plan, implement, review         |
  | `FIX`   | bug       | reproduce, fix, review               |
  | `CLEAN` | refactor  | plan, implement, review              |
  | `TOOL`  | tooling   | plan, implement, review              |

- **R-ENT-7 (must).** Item keys are minted from a per-project, per-prefix sequence held by Postgres.
  Never reused: a project has exactly one counter and exactly one writer of it. Creating an item
  in a project mirrored from a server, while that server is unreachable, is not supported (see R-STO-4).
- **R-ENT-8 (must).** Item status is one of `open`, `queued`, `in_progress`, `awaiting_approval`,
  `blocked`, `done`, `failed`, `closed`. Transitions are driven by the orchestrator and by close-out,
  not edited by hand.
- **R-ENT-9 (must).** Item links are directed edges by UUID with kind `blocked_by`, `origin`,
  `relates`, `supersedes`. Because edges use UUIDs, cross-project links need nothing extra. Links
  are proposed by agents through the `htui` MCP server (R-MCP-2) and by the importer (R-LATER-3);
  there is no manual link action.
- **R-ENT-10 (must).** Item body and title changes are recorded as revisions with author, box and
  reason. Concurrent edits on the same `version` are rejected and shown as a divergence with the
  common ancestor; no timestamp-based last-writer-wins anywhere.
- **R-ENT-11 (must).** Item notes: a thread of human comments per item, plain markdown, no editing
  history needed.
- **R-ENT-12 (must).** `document`: versioned per item and kind. Kinds follow the phases of the
  graph (`prd`, `plan`, `review`, `summary`, `verdict`, and any phase-defined kind). Produced by
  steps or written by hand.
- **R-ENT-13 (withdrawn).** Atomic facts table with importance scores (ANA-1 `artifact`,
  `artifact_link`). Dropped as over-engineered; summaries cover the need. Re-open only on evidence.

## 4. Storage, cache and offline (R-STO)

- **R-STO-1 (must).** Postgres is the only writable store. Connection
  string and provider identities live in the OS keyring (Windows Credential Manager, macOS Keychain,
  Linux secret service), never in a file — including when the connection string is typed into the
  TUI rather than passed to `htui --set-dsn`.
- **R-STO-2 (must).** TLS to Postgres is supported and optional.
- **R-STO-3 (must).** Each box keeps a read-only cache of the projects it has opened under the
  user's config directory, refreshed on every successful connection. Contents: items, links,
  documents, notes, run and step summaries, and transcripts of the last N steps (N configurable).
- **R-STO-4 (must).** When Postgres is unreachable, the TUI opens in offline
  read-only mode from the cache: browse items, graph, documents and cached transcripts. No item
  creation, no runs.
- **R-STO-5 (must).** Schema migrations are versioned, forward-only in version one, applied by
  `htui` on connect after confirmation. The per-box SQLite schemas for the read-only cache (R-STO-3)
  are versioned and forward-only, but answer a version mismatch by rebuilding.
- **R-STO-6 (must).** Startup with a warm cache and reachable Postgres is under one second on the
  reference workstation. Cache refresh runs in the background and never blocks input. The budget is
  conditioned on a reachable server and binds no offline start (R-STO-4); that scope was decided,
  not overlooked (maintainer, 2026-09-08).
- **R-STO-7 (withdrawn).** Local-only mode. Withdrawn by maintainer decision MOD-25, 2026-09-11.
- **R-STO-8 (must).** Semantic search over items. `htui` indexes each item's key, title and body,
  and its latest documents, into a Qdrant collection (local dense embeddings plus BM25 sparse
  vectors, ranked together) and answers searches scoped to projects. The index is derived from
  Postgres and rebuildable from it (R-STO-1); when Qdrant or the embedding model is unavailable,
  search fails with a clear error and nothing else is affected. Added by maintainer decision on
  MOD-34, 2026-09-25 (`docs/ANA-19.md`, `docs/ANA-20.md`).

## 5. Agent driver (R-AGT)

- **R-AGT-1 (must).** One `AgentDriver` trait: start a session (prompt, working directory,
  environment, tool exposure), stream typed events, send a follow-up, answer a permission request,
  cancel. Event kinds: assistant text, thought, tool call, tool result, edit proposal, permission
  request, usage, error, done.
- **R-AGT-2 (must).** Primary implementation: ACP (Agent Client Protocol) client over JSON-RPC.
  `htui` is the ACP client; the agent, or its ACP adapter, is the server. `htui` handles tool
  permissions, presents edit proposals, and receives usage.
- **R-AGT-3 (must).** Fallback implementation: CLI stream adapter for agents without ACP, parsing
  the agent's headless streaming output into the same event kinds.
- **R-AGT-4 (must).** Agent registry in Postgres: name, transport (`acp` or
  `cli`), launch command, model list, billing mode (`subscription` or `per_token`), default model,
  enabled per box. Version one entries: `claude`, `claude-cli`, and `agy`. Amended 2026-09-09:
  `agy` over ACP is **settled and proven**.
- **R-AGT-5 (must).** Adding an agent requires a registry row and, at most, one stream adapter. No
  orchestrator or prompt code changes.
- **R-AGT-6 (must).** Autodiscovery: on box registration and on demand, probe `PATH` for known
  agent binaries and ACP adapters, record version, mark enabled on that box. Manual entries allowed.
- **R-AGT-7 (must).** Quota tracking per agent: for `subscription` agents, remaining allowance or
  reset window as reported by the agent or its CLI, refreshed per run; for `per_token` agents, a
  configurable cap per run and per batch, enforced by cancelling the session when exceeded.
- **R-AGT-8 (must).** When a phase lists several candidate agents, the orchestrator picks the first
  in priority order whose quota is not exhausted and whose billing cap is not reached.
- **R-AGT-9 (must).** An agent that reports itself installed but unauthenticated is authenticated
  **from the app**, through the agent's own protocol, without leaving `htui` for a vendor CLI. The
  method is chosen from the ones that agent advertises; logging out is offered where the agent
  advertises it. `htui` triggers the flow and observes its outcome: it does not read, hold, transmit
  or store the credential, which stays wherever the agent keeps it (R-ID-7, R-SEC-2). Authentication
  is a fact about a box, not about a registry row. Added by maintainer decision, 2026-09-09,
  reversing `docs/ANA-4.md` §4.5's conclusion that `htui` cannot log an agent in; implemented as
  MOD-21.
- **R-AGT-10 (must).** An agent whose adapter is not installed on a box can be installed **from the
  app**, from a source declared in its own registry row, with no code path per agent (R-AGT-5). The
  app re-probes after installing, so what the box can run is always the probe's answer rather than
  the installer's claim (R-AGT-6). Where the source publishes an integrity digest it is verified;
  where it does not, the user is told that before the download, not after. Where the adapter is
  proprietary, its licence and account-type consequences are surfaced before `htui` fetches it on
  the user's behalf. Added by maintainer decision, 2026-09-09, reversing `docs/ANA-4.md` §4.6's
  deferral of registry-driven installation; implemented as MOD-20.

## 6. Orchestration (R-ORCH)

- **R-ORCH-1 (must).** A step graph is a named, ordered list of phases owned by a project. Each
  phase has: candidate agents with model in priority order, fan-out count, gate, retry limit, input
  document kinds, output document kind, fan-out isolation mode, command queue mode, optional
  verification command. An item inherits its kind's default graph and may override it.
- **R-ORCH-2 (must).** Gate is `always`, `on_failure` or `never`, plus a `hard` flag. A gated step
  ends in `awaiting_approval`; the user approves, rejects with a note, edits the artifact, or
  retries. Auto mode (R-ORCH-6) downgrades every gate to `never` except those flagged `hard`.
- **R-ORCH-3 (must).** A failed `review` loops back to the preceding `implement` with the review
  attached, up to the retry limit, then escalates to the user.
- **R-ORCH-4 (must).** Manual mode runs one selected item through its graph with gates as
  configured.
- **R-ORCH-5 (must).** Any step can be promoted to an interactive chat, on failure or by user
  request, preserving the session context. When the chat yields the step's artifact, the pipeline
  resumes from the next step.
- **R-ORCH-6 (must).** Auto mode is a queue runner: picks ready items by dependency order and
  priority, filters by current box capabilities, runs unattended with gates treated as `never`
  unless the user marked a gate as hard, escalates failures, and honors caps per item and per
  batch. Supports a schedule window and a target box (R-ORCH-12).
- **R-ORCH-7 (must).** Fan-out: a phase with N candidate sessions runs them in parallel on the same
  prompt, each producing its own artifact and, for code phases, its own tree. Selection is a human
  choice when gated, otherwise a judge step using a configured agent. Only the selected result
  continues; the rest are kept as history.
- **R-ORCH-8 (must).** Fan-out isolation mode per project or phase: `worktree` (default, one git
  worktree per session), `copy` (directory copy for trees where worktrees break build caches),
  `shared_serialized` (one tree, sessions run one after another), `local` (run directly in the
  working tree with no isolation and no serialization).
- **R-ORCH-9 (must).** Concurrent items: both modes may run several items at once when they do not
  overlap. Overlap: same repo unless both use isolated trees, or declared touched-path sets
  intersect. Overlapping items are serialized; others run in parallel up to a per-box limit.
- **R-ORCH-10 (must).** Capability matching: a run is refused when the item's required tags are not
  a subset of the box's tags, listing the missing tags. Auto mode filters the queue the same way.
- **R-ORCH-11 (must).** Every run records: item, target box, executing box, graph snapshot, and per
  step: agent, model, gate outcome, status, exit code, prompt digest, commit hashes, usage, timing.
- **R-ORCH-12 (later).** Remote dispatch: queue an item from one box to run on another. Requires a
  headless `htui` worker per box polling Postgres, which amends R-ID-2. Version one stores the
  target box and executes only when it is the local box.
- **R-ORCH-13 (later).** Scheduling: run the queue inside a time window on a chosen box.

## 7. Run history (R-HIS)

- **R-HIS-1 (must).** Every session event from R-AGT-1, plus the assembled prompt, every follow-up
  and every permission answer, is stored as an ordered row per run step, after scrubbing. Nothing
  about a run exists only on one box.
- **R-HIS-2 (must).** The chat view can reopen any past step read-only and replay it.
- **R-HIS-3 (must).** Retention is configurable per project; default keeps everything.

## 8. Prompt assembly and skills (R-PRM, R-SKL)

- **R-PRM-1 (must).** Each step receives one self-contained initial prompt built by `htui`:
  phase template, item body, input documents from prior steps, summaries of upstream items reached
  by `blocked_by` and `origin` edges within one to two hops and inside the active workspace (or
  current project when no workspace), box profile, bound skill text, and any `htui`-selected file
  excerpts. Never raw transcripts of other items.
- **R-PRM-2 (must).** Upstream items outside the active workspace appear as one-line status stubs.
- **R-PRM-3 (must).** Per-step token budget; oversize inputs are trimmed by a fixed priority order
  (template and skills first, then item body, then prior documents, then upstream summaries, then
  excerpts) and the trim is recorded on the step.
- **R-PRM-4 (must).** Prompt templates per phase are versioned rows in Postgres,
  with a documented placeholder contract, editable in the TUI.
- **R-SKL-1 (must).** Skill library in Postgres: name, description, versioned markdown body.
- **R-SKL-2 (must).** Bindings at project level and at phase level; a phase binding overrides a
  project binding of the same skill. A binding pins a version or follows latest.
- **R-SKL-3 (must).** TUI supports create, edit, view version diff, bind and unbind.
- **R-SKL-4 (must).** Import from existing `SKILL.md` files as a convenience.

## 9. Secrets and scrubbing (R-SEC)

- **R-SEC-1 (must).** `SecretProvider` trait: list keys for a project path, resolve values,
  bootstrap a machine identity, report health. Infisical is the first implementation.
- **R-SEC-2 (must).** Scope is project secrets that the code under work needs, resolved at run start
  into the agent subprocess environment only. Agents never receive a tool that reads secrets, and
  `htui`'s own credentials (R-STO-1) are never exposed to a session.
- **R-SEC-3 (must).** Scrubber builds exact-match masks from every resolved secret value plus a
  pattern rule set for known key formats, runs before any transcript row persists, and marks the
  step failed and blocks persistence when an unmasked pattern remains.
- **R-SEC-4 (must).** When the provider is unreachable, runs needing its secrets are refused with a
  clear error; there is no plaintext fallback.

## 10. htui MCP server (R-MCP)

- **R-MCP-1 (must).** `htui` exposes an MCP server to every session it launches. Every tool call is
  scoped to the run step owning the session; an agent cannot touch items outside its run.
- **R-MCP-2 (must).** Tools: `item_link` (propose an edge, kind), `item_status` request,
  `document_write` (the step's output artifact), `note_add`, `box_profile` read, `command_run`,
  `spawn_subagent`.
- **R-MCP-3 (must).** `command_run` enqueues build, test and run commands behind per-box
  concurrency limits by command class (for example one C++ build, four test runs) and returns
  output. Exposure per phase is `off`, `fan_out_only` (default) or `always`, and can be forced on
  by the item carrying the `heavy_build` tag. When off, the tool is not advertised to the session.
- **R-MCP-4 (must).** Skill text instructs agents to use `command_run` for heavy commands when it is
  exposed; ACP permission handling denies matching direct shell commands in that case.

## 11. TUI (R-TUI)

- **R-TUI-1 (must).** Keyboard driven, mouse optional. Top bar: workspace or project, box, store
  state — distinguishing online, connecting, and offline since T
  — active run count. Tabs: Backlog, Chat (one per session), Skills, Settings. Workspace switcher
  overlay. Queue overlay for auto mode with reorder and pause.
- **R-TUI-2 (must).** Backlog left pane: items grouped by project, filters by status, project,
  capability and readiness. Actions: new, edit, run, queue, close, open graph.
- **R-TUI-3 (must).** Backlog right pane is tabbed per selected item: **Body**, **Runs**, **Graph**,
  **Documents**, **Notes**.
- **R-TUI-4 (must).** Runs tab: active and past runs for the item, step list with agent, model,
  gate state, usage, duration. Actions: approve, reject with note, retry, promote to chat, cancel,
  open artifact, select fan-out result.
- **R-TUI-5 (must).** Graph tab: the item and everything connected, one to N hops, across projects,
  with status and link kind, navigable.
- **R-TUI-6 (must).** Chat tab: streamed session with assistant text, collapsed thoughts, tool calls
  and results, edit proposals, permission prompts answered inline, follow-up input. Bound to a run
  step or free-standing against a project.
- **R-TUI-7 (must).** Skills tab: library, editor, version diff, bindings matrix by project and
  phase.
- **R-TUI-8 (must).** Settings tab: agent registry with quota, box profile with capability edits,
  item kinds and step graphs per project, secret provider, caps, scheduler window, and the Postgres
  connection: whether a DSN is stored, a masked field to enter or replace it, an action to clear it,
  and the state of the last attempt. What is typed goes to the OS keyring and nowhere else
  (R-STO-1); it is not echoed, not logged, and not written to any file.
- **R-TUI-9 (must).** Close-out writes the final summary document, sets status, records commit
  hashes from the run. No markdown files are produced.

## 12. Later tier (R-LATER)

Designed for, not built in version one. Each needs its own `ANA-N` before implementation.

- **R-LATER-1.** Diff tab (working-tree diff per repo, structural diff optional) and read-only code
  explorer with multi-root tree.
- **R-LATER-2.** Issue tracker sync behind an `IssueSync` trait (OneDev, GitHub, others) as a
  downstream mirror only; Postgres stays canonical.
- **R-LATER-3.** Legacy import of markdown workflow repos (`HANDOFF.md`, `DECISIONS.md`,
  `docs/decisions/**`, `docs/ANA-*.md`) into projects, items, documents and links, mapping old
  prefixes to kinds per project, preserving keys.
- **R-LATER-4.** Remote dispatch and scheduling (R-ORCH-12, R-ORCH-13).
- **R-LATER-5.** Team support (R-USR-3).
- **R-LATER-6.** Additional agents beyond `claude` and `agy` via the registry (R-AGT-5).
- **R-LATER-7.** External context tools (Headroom, Serena, Graphify) as excerpt providers for
  R-PRM-1, fail-open when absent.

## 13. Non-functional (R-NF)

- **R-NF-1 (must).** Windows 10+, Linux, macOS.
- **R-NF-2 (must).** No dependency on any external daemon other than Postgres and the agents.
- **R-NF-3 (must).** All long operations (probe, cache refresh, sessions) run off the UI thread;
  the TUI never blocks on network or subprocess I/O.
- **R-NF-4 (must).** Every `ANA-N` and `MOD-N` item cites the requirement IDs it addresses.

## 14. Out of scope

- Hosting or managing the Postgres server itself.
- Any web or GUI front end.
- Editing code inside `htui`.
- Exporting `HANDOFF.md` or other markdown views from the database.

## 15. Superseded material

- `docs/ANA-1.md`: schema, `items.json`, `artifact` tables, LWW conflict rule, `sync_state`,
  `external_issue_mapping` are superseded. Box registry, prompt injection contract, transcript
  split and scrubbing survive as restated in sections 2, 8 and 9.
- `docs/ANA-8.md`: three-tier hierarchy, workspace-centric topology and cross-project UUID links
  survive. Implicit workspaces, `items.json` v2, `htui-workspace.json`, the `V002` migration and the
  TUI mockups are superseded.
