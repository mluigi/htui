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
amended 2026-09-14 by maintainer decision — R-MCP-2 added spawn_subagent, R-AGT-4 added claude-cli;
amended 2026-09-25 by maintainer decision on ANA-11 (`docs/ANA-11.md` §7,
`docs/decisions/ana/ana-11.md`) — R-ENT-14 and R-ENT-15 added; R-ENT-8, R-NF-4, R-STO-3 and
R-TUI-1 amended in place; the optional R-MCP-2 `requirement_cite` amendment deferred to MOD-11;
amended 2026-09-26 by maintainer decision on ANA-22 (`docs/ANA-22.md` §6,
`docs/decisions/ana/ana-22.md`) during MOD-9 milestone 2 — R-SKL-2 amended in place (global level,
most-specific-wins, activation on the attachment);
amended 2026-09-26 by maintainer decision on ANA-16 (`docs/ANA-16.md` §6.1 C5,
`docs/decisions/ana/ana-16.md`) during MOD-40 — R-STO-5 amended in place (a headless process never
migrates; it refuses and reports);
amended 2026-09-29 by maintainer decision on MOD-50 (`docs/decisions/mod/mod-50.md`) — R-STO-8
amended in place (requirement rows indexed; the decisions filter keyed on resolution);
amended 2026-09-30 by maintainer decision on ANA-26 (`docs/ANA-26.md` §9,
`docs/decisions/ana/ana-26.md`) — R-TUI-10 added (configurable keys); R-TUI-1 and R-STO-1
amended in place;
and by maintainer decision on MOD-22 (`docs/decisions/mod/mod-22.md`, plan OQ-1) —
R-AGT-9 amended in place (the loopback paste-back relays one authorization code once).
amended 2026-09-29 by maintainer decision on MOD-41 (`.claude/prds/mod-41-headless-worker.prd.md`
D1-D3, `docs/ANA-16.md` §7) — R-ID-2, R-STO-1 and R-ORCH-12 amended in place (a per-box headless
worker; its DSN source; remote dispatch promoted to must), R-LATER-4 narrowed to scheduling.
amended 2026-10-01 by maintainer decision on MOD-45 (`docs/decisions/mod/mod-45.md`, plan OQ-1) —
R-STO-1 amended in place (a TUI session may read the DSN once from stdin, held only in memory).
amended 2026-10-04 by maintainer decision on MOD-10 (`.claude/prds/mod-10-secret-provider.prd.md`,
plan D7, blueprint A-5) — R-SEC-3 amended in place (a scrub refusal is recorded with a typed reason
on whichever path it lands: the run fails, the fan-out candidate fails, or the judge parks).
amended 2026-10-03 by maintainer decision on MOD-69 (`.claude/prds/mod-69-waiting-on-you.prd.md`,
`docs/ANA-27.md` §7) — R-TUI-11 added (waiting-on-you list); R-TUI-1 amended in place (working and
waiting-on-you counts replace the active run count).
Recorded late (2026-10-14 requirements audit), each made at the time by maintainer decision:
2026-09-25 on MOD-34 (`docs/decisions/mod/mod-34.md`) — R-STO-8 added (semantic search);
2026-09-28 on ANA-21 (`docs/decisions/ana/ana-21.md`) — R-AGT-8 amended in place (remaining fan-out
slots apportioned by per-model weight);
2026-10-14 on ANA-30 (`docs/decisions/ana/ana-30.md`) — R-ID-6 amended in place (display-only labels
and person-accepted drafts are outside it).
amended 2026-10-14 by maintainer decision on the requirements audit (MOD-99) — R-NF-2 amended in
place (Qdrant and a user-configured error-reporting endpoint are optional and fail open); R-ORCH-6
amended in place (the schedule window is R-ORCH-13, `later`); R-ORCH-12's delivery note corrected
(MOD-43 is open); R-LATER-6 names `claude-cli`.
amended 2026-10-15 by maintainer decision on the requirements review
(`docs/requirements-review-2026-10-15.md`) — R-ENT-16 and R-ORCH-14 added; R-LATER-3 and R-LATER-7
promoted to must; R-LATER-8..13 added; R-ENT-13 reopened pending ANA-38; amended in place: R-ID-1,
R-ID-2, R-ID-3, R-ID-5, R-ID-6, R-ID-7, R-BOX-2, R-BOX-3, R-BOX-4, R-ENT-2, R-ENT-3, R-ENT-4, R-ENT-5,
R-ENT-6, R-ENT-8, R-ENT-9, R-ENT-11, R-ENT-12, R-ENT-14, R-ENT-15, R-STO-1, R-STO-5, R-STO-6, R-STO-8,
R-AGT-1, R-AGT-5, R-AGT-7, R-AGT-8 (compete mode named), R-AGT-9, R-AGT-10, R-ORCH-1, R-ORCH-2, R-ORCH-3, R-ORCH-4, R-ORCH-6,
R-ORCH-7, R-ORCH-10, R-HIS-2, R-PRM-1, R-PRM-3, R-PRM-4, R-SKL-1, R-SKL-2, R-SKL-4, R-MCP-1, R-MCP-2, R-MCP-3,
R-TUI-1, R-TUI-2, R-TUI-3, R-TUI-4, R-TUI-5, R-TUI-6, R-TUI-7, R-TUI-8, R-TUI-9, R-TUI-10, R-TUI-11,
R-LATER-2, R-NF-1, R-NF-2, R-NF-3, R-NF-4 and §14 (the web front end leaves it for R-LATER-8). Where the design is open, the text names the
deciding analysis (ANA-33..39).
amended 2026-10-17 by maintainer decision on the UI redesign review
(`docs/ui-redesign-findings-2026-10-15.md`) — R-ENT-17, R-AGT-11, R-ORCH-15, R-ORCH-16, R-SEC-5 and
R-TUI-12..18 added; amended in place: R-ID-6, R-ENT-2, R-ENT-14, R-ENT-15, R-ORCH-1, R-HIS-3,
R-PRM-4, R-TUI-1, R-TUI-2, R-TUI-3, R-TUI-4, R-TUI-5, R-TUI-6, R-TUI-7, R-TUI-8 and R-TUI-10. The
analyses it opens are ANA-40 (the Toolbox), ANA-41 (project workflows) and ANA-42 (execution
targets).
amended 2026-10-18 by maintainer decision on ANA-33 (`docs/ANA-33.md` §8, §10,
`docs/decisions/ana/ana-33.md`) — R-ORCH-1, R-MCP-3 and R-TUI-8 amended in place (`retries` is a loop
on the same step; the meaning of `fan_out_only`; the shared graph editor).
amended 2026-10-18 by maintainer decision on ANA-35 (`docs/ANA-35.md` §10,
`docs/decisions/ana/ana-35.md`) — amended in place: R-ID-7, R-ENT-2, R-ENT-3, R-ENT-4, R-ENT-5,
R-ENT-6, R-ENT-7, R-ENT-8 (with ANA-34's transition sentence, `docs/ANA-34.md` §10.4), R-ENT-9,
R-ENT-12, R-ENT-14, R-ENT-16, R-STO-3, R-ORCH-8, R-TUI-2 and R-TUI-11 (project types, folders,
workspace ownership and keys, the three item statuses and their derived states, links, attachments).
amended 2026-10-18 by maintainer decision on ANA-36 (`docs/ANA-36.md` §8 Q13, §9,
`docs/decisions/ana/ana-36.md`) — R-MCP-2 replaced (typed requests, the asynchronous command form,
`requirement_cite`, proposals held until a person accepts them, and the coordinator's tools of
`docs/ANA-34.md` §10.11); amended in place: R-MCP-1, R-MCP-3, R-MCP-4, R-ENT-9, R-ENT-15 and R-ID-5.
amended 2026-10-19 by maintainer decision on ANA-37 (`docs/decisions/ana/ana-37.md`, `docs/ANA-37.md` §10) —
R-NF-2 and R-STO-8 replaced (one shared Qdrant per workspace, required and failing open; knowledge
search for the workspace, one indexer under a lease, a collection per workspace, related items at
draft time); amended in place: R-STO-1 (the Qdrant API key in the keyring, the URL a workspace
setting), R-MCP-1 (notes and transcripts through search), R-MCP-2 (a text filter on the read tools,
knowledge search at workspace scope), R-ENT-13 (memory indexed, and reaching a prompt without it) and
R-TUI-8 (the workspace's Qdrant in Settings).
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
  `tokio`) that wraps AI agents to run a backlog of work items through configurable step graphs
  across projects, repositories and machines. A project may be of any kind — software, research,
  documentation, operations, or any task an agent can do; coding is the main case, not the only one.
  Amended by maintainer decision, 2026-10-15.
- **R-ID-2 (must).** `htui` ships no code editor of its own. It is a local-first, developer-guided
  harness. A headless `htui worker` process per box, started by the user and talking only to that
  user's Postgres, is part of `htui`; it is not a hosted service (R-ORCH-12). It hosts the user's own
  `$EDITOR` and opens it on a file an agent touched, at the line (from a tool call, an edit proposal,
  a diff or an artifact). Multiplexer-like terminal panes, a web front end and code viewing are
  allowed and designed for, but are later tier (R-LATER-9, R-LATER-8, R-LATER-1). Amended by
  maintainer decision on MOD-41, 2026-09-29, and on the requirements review, 2026-10-15.
- **R-ID-3 (must).** Postgres is the single source of truth.
  Everything `htui` knows lives there: items, documents, runs, transcripts, skills, templates, box
  profiles, agent registry, key bindings, settings. Not stores, and holding no domain data of their
  own: the per-box identity file, the read-only cache (R-STO-3), and the semantic-search index
  (R-STO-8), which is derived from Postgres and rebuildable from it. Amended 2026-10-15.
- **R-ID-4 (must).** `htui` writes no files into managed repositories. The only changes to a
  working tree are made by agents doing the item's work. Rationale: keeps repos clean and avoids
  merge conflicts on generated state.
- **R-ID-5 (must).** Agents run with their own system skills disabled. `htui` owns every prompt and
  every skill text and inlines them into the initial prompt, so behavior is identical on every box
  and agents never spend turns reading instruction files. Isolation is the default; a registry row
  may opt out, and an opted-out agent is shown as such in Settings (ANA-31 settles `agy`). A step's
  bound skills may carry resources (R-SKL-1) that the agent reads on demand through `htui`'s MCP read
  tools; the skill text itself stays inlined. Amended 2026-10-15 and 2026-10-18 (ANA-36).
- **R-ID-6 (must).** No LLM agent is ever in a sync, cache, import or bookkeeping path. Those are
  deterministic code. Outside it (ANA-30, 2026-10-14): a display-only label a model derives, and a
  draft a person accepts or edits before it is saved, as long as no status transition, link,
  import, cache refresh or prompt ever reads the unaccepted model text. Such drafts may also help
  import and triage — a mapping of legacy items, a suggested link, a proposed item — and are written
  only once a person accepts them (amended 2026-10-15). An agent choosing *which work to do next*
  (R-ORCH-1's planned graphs, R-ORCH-14's coordinator) is item work, not bookkeeping; the status
  transitions it causes stay deterministic. The light model (R-ORCH-1) that picks a run's workflow
  and rewrites a new item's or requirement's draft (R-TUI-17) is outside it too: one is item work,
  the other a draft a person accepts. One configurable light model serves both, which partly
  reverses ANA-30's rejection of a single `utility_model` setting. Amended 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).
- **R-ID-7 (must).** Anything persisted or sent off the box — transcripts, tool output, logs,
  telemetry, error reports, the cache — is scrubbed of secrets on the host box first. Scrubbing fails
  closed. A binary attachment a person adds is the one exception: it is stored as given, marked
  unscrubbed, and never accepted from an agent (R-ENT-12). Amended 2026-10-15 (was: transcripts and
  tool output) and 2026-10-18 (ANA-35, the binary attachment exception).

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
  language runtimes (`node`, `python`, `go`, `java`), package and tool managers (`mise`, `npm`,
  `volta`, `brew`, `winget`, `scoop`), shells, container runtime, installed agents, free disk space
  and network reachability. Re-probe on demand and when `htui` version changes. `last_seen` updated
  per session. Amended 2026-10-15.
- **R-BOX-3 (must).** Box carries capability tags, both probed and hand-declared, plus a free-form
  quirks note editable in the TUI. Seeded vocabulary: `gpu`, `vulkan`, `msvc`, `mingw`, `clang`,
  `cmake`, `vcpkg`, `docker`, `rust`. Vocabulary is open. `heavy_build` is an item label (R-ENT-5,
  R-MCP-3), not a box capability (amended 2026-10-15, answering ANA-28).
- **R-BOX-4 (must).** Each box holds per-box paths for every repo and folder it has checked out.
  Logical identity (UUID, slug) never depends on a filesystem path. `htui` can clone a repo onto a box
  that lacks it, from the repo's remote URL, and records the path. When the box has no credentials
  for that remote, `htui` says which remote needs which credential and how to provide it, marks the
  repo unavailable on that box, and can guide the setup itself: generate an SSH key and show its
  public half, or store an HTTPS token in the OS keyring for git to use, never in a file (R-STO-1).
  Amended 2026-10-15.

## 3. Entity model (R-ENT)

- **R-ENT-1 (must).** Three-tier hierarchy: `workspace` groups projects, `project` owns work,
  `repo` is a git checkout or a plain folder (R-ENT-4).
- **R-ENT-2 (must).** `workspace`: named, explicit group of one or more related projects, which may
  live anywhere on a box; a workspace is not a directory its projects must live under. It owns its
  own items, documents and requirements (R-ENT-16). There are no implicit workspace rows: opening a
  single project runs with no workspace, and the workspace-scoped views and searches cover just that
  project. A workspace has a folder on each box (by default `~/workspaces/<name>`) where `htui` writes
  what it generates — the compose file (R-ENT-17) and, without Infisical, the environment file
  (R-SEC-5) — and where projects cloned from a URL go; existing repos stay where they are. Its
  description and goals live in Postgres, the goals as a workspace document (R-ENT-12). A single
  project with no workspace has no folder, no components and no workspace-owned items.
  Amended 2026-10-15 (the per-box root path is dropped), 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) (a folder for
  what `htui` generates) and 2026-10-18 (ANA-35, what the folder holds; no folder for a lone project).
- **R-ENT-3 (must).** `project`: owns the item key space, the item kinds, step graphs, skill
  bindings, secret provider binding and phase defaults. It has a type, chosen from the project types
  (software, research, docs, ops, general, and the user's own, R-TUI-8), whose kinds, graphs and
  templates are copied into it once when it is created; changing a type never re-seeds an existing
  project. It has zero or more repos; when it has any, exactly one is primary, which may be a folder;
  removing a repo archives it, and the primary passes to another. A phase marked as needing a repo
  refuses to start in a project without one. A workspace owns its own key space, kinds, graphs,
  documents and requirements in the same way (R-ENT-16). Amended 2026-10-15 (repos optional, project
  type) and 2026-10-18 (ANA-35, project types and the primary repo).
- **R-ENT-4 (must).** `repo`: a git repository or a plain folder belonging to exactly one project,
  with per-box local path and, for a git repository, remote URL and default branch. What needs git —
  `worktree` isolation, commit hashes, merging — needs a git repository; `copy` and `local` isolation
  work on a folder. A plain folder cannot be cloned onto another box, pushed or run on a target other
  than the box that holds it. `copy` on a folder brings back only files the folder has not changed
  since the copy and parks on the rest; a `worktree` step copies a folder instead, and
  `shared_serialized` refuses one (R-ORCH-8). Amended 2026-10-15 and 2026-10-18 (ANA-35, folders).
- **R-ENT-5 (must).** `item`: key (`<PREFIX>-<N>`, unique per owner — a project, or a workspace
  (R-ENT-16)), kind, title, body, status, priority, labels, required capability tags, `created_by`,
  `version` for optimistic locking, timestamps. Body is markdown edited in the TUI or an external
  editor. Labels are free and route nothing; required capability tags route (R-ORCH-10). Assignee and
  due date come with team use (R-LATER-11). Amended 2026-10-15 and 2026-10-18 (ANA-35, unique per
  owner).
- **R-ENT-6 (must).** Item kinds are configurable per project. Each kind has a key prefix, a
  description and a default step graph. The seeded set depends on the project type (R-ENT-3), the
  prefixes editable per project like any kind. Seeded set for a software project:

  | Prefix  | Kind      | Default graph                        |
  |---------|-----------|--------------------------------------|
  | `ANA`   | analysis  | research, verdict                    |
  | `FEAT`  | feature   | prd, plan, implement, review         |
  | `FIX`   | bug       | reproduce, fix, review               |
  | `CLEAN` | refactor  | plan, implement, review              |
  | `TOOL`  | tooling   | plan, implement, review              |

  Seeded sets for the other types (ANA-35 §7.4):

  | Type      | Prefix | Kind               | Default graph                      |
  |-----------|--------|--------------------|------------------------------------|
  | research  | `RES`  | research           | research, verdict                  |
  | research  | `Q`    | question           | question, answer                   |
  | docs      | `DOC`  | document           | outline, draft, review             |
  | ops       | `OPS`  | operation          | plan, execute, verify              |
  | ops       | `INC`  | incident           | investigate, fix, postmortem       |
  | general   | `TASK` | task               | execute, review                    |
  | workspace | `WS`   | workspace item     | plan, review                       |

  Amended 2026-10-18 (ANA-35, the seed table for every type).

- **R-ENT-7 (must).** Item keys are minted from a per-owner, per-prefix sequence held by Postgres;
  the owner is a project or a workspace. Never reused: each owner has exactly one counter per prefix,
  and Postgres is its only writer. Keys of different owners may coincide and are qualified by the
  owner's slug where ambiguous. Creating an item in a project mirrored from a server, while that
  server is unreachable, is not supported (see R-STO-4). Amended 2026-10-18 (ANA-35, per-owner
  counters).
- **R-ENT-8 (must).** Item status is one of `open`, `blocked` or `closed`. The progress of work —
  `queued`, `in_progress`, `awaiting_approval`, `failed` — belongs to the item's runs, not to the
  item. Transitions are driven by deterministic code, the orchestrator, the coordinator's
  integration (R-ORCH-14) and a person's close-out (R-TUI-9), never by unaccepted model text, not
  edited by hand. A closed item carries a resolution: `done`, `concluded`, `rejected`, `withdrawn`,
  `superseded` or `duplicate`. Resolution is not a status. `blocked` means a person must clear
  something and always carries a note; a dependency only makes an item not ready. What a person sees
  — queued, running, needs you, failed, done (finished, close-out waiting), open, blocked, closed — is
  derived from the status and the item's latest run, a cancelled run changing nothing. A `blocked_by`
  link stops blocking when its target is closed or its latest run finished and its work landed.
  Resolutions `done` and `concluded` require a finished run; the importer (R-LATER-3) records closed
  items as it finds them. Reworking a finished item means starting a new run on it. Amended
  2026-10-15 (was eight statuses; migration designed in ANA-35) and 2026-10-18 (ANA-34's transition
  sentence; ANA-35's derived state, release rule and close-out law).
- **R-ENT-9 (must).** Item links are directed edges by UUID with kind `blocked_by`, `origin`,
  `relates`, `supersedes`, `parent` (decomposition: a parent and its children, unlike `origin`, which
  is provenance), `duplicates` and `implements`; a link may also target a document or an attachment.
  Because edges use UUIDs, cross-project links need nothing extra. Links are proposed by agents
  through the `htui` MCP server (R-MCP-2) and by the importer (R-LATER-3). Requirements are linked by
  citation (R-ENT-15). An item has at most one `parent` and parents never form a cycle. A link to a
  document names the version it was made against. There is no manual link action, except that a
  person's close-out as `duplicate` or `superseded` records the link to the item they name. A proposed
  link of a kind that scheduling, status or a subgraph reads (`blocked_by`, `supersedes`, `duplicates`,
  `parent`) is pending until a person accepts it (R-ID-6). Amended 2026-10-15 and 2026-10-18 (ANA-35,
  parent and document-link rules; ANA-36).
- **R-ENT-10 (must).** Item body and title changes are recorded as revisions with author, box and
  reason. Concurrent edits on the same `version` are rejected and shown as a divergence with the
  common ancestor; no timestamp-based last-writer-wins anywhere.
- **R-ENT-11 (must).** Item notes: a thread of comments per item, from people and from agents
  (`note_add`, R-MCP-2), plain markdown, no editing history needed. Amended 2026-10-15.
- **R-ENT-12 (must).** `document`: versioned per owner and kind; the owner is an item, a project or
  a workspace. Item kinds follow the phases of the graph (`prd`, `plan`, `review`, `summary`,
  `verdict`, and any phase-defined kind). Produced by steps or written by hand. Items and document
  versions may carry attachments (images, logs, other files), held in Postgres in version one under a
  size cap, 10 MiB by default and never above 64 MiB (an object store is R-LATER-10). Text attachments
  are scrubbed (R-ID-7); a binary attachment is accepted only from a person, who is told it is stored
  as given. Amended 2026-10-15 and 2026-10-18 (ANA-35, attachment owners, cap and scrubbing).
- **R-ENT-13 (reopened, pending ANA-38).** Project and workspace memory: facts, decisions and gotchas
  that outlive one item, for the coordinator (R-ORCH-14) and for agents. ANA-1's atomic facts table with
  importance scores (`artifact`, `artifact_link`) was withdrawn as over-engineered on 2026-09-03;
  reopened 2026-10-15 by maintainer decision. ANA-38 settles who writes it, how it stays fresh, how it
  meets R-ID-6, and its integration with semantic search (R-STO-8). Memory entries are indexed in the
  workspace's knowledge search (R-STO-8), and memory reaches a prompt without it, so an outage of the
  workspace's Qdrant never empties a prompt (R-NF-2, ANA-37).
- **R-ENT-14 (must).** `requirement`: key `R-<AREA>-<N>` minted per owner and area, never reused;
  the owner is a project or a workspace. It has body, rationale, priority `must`/`later`, state
  `active`/`withdrawn`, and a `version` with revisions naming the deciding item. Text changes only by
  human action: coordinators and agents propose changes as drafts a person accepts (R-ID-6). A
  requirement may refine another; the child's key is its parent's plus `.n` (`R-TUI-2.1`).
  Workspace requirements are keyed `W-<AREA>-<N>`. Nesting with `.n` stays within one owner; a
  project requirement that refines a workspace one keeps its own key. Amended 2026-10-15
  (workspace requirements), 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) (nesting, drafts) and
  2026-10-18 (ANA-35, workspace keys).
- **R-ENT-15 (must).** Items cite requirements with kind `addresses`, `amends`, `withdraws`,
  `reserves` or `verifies` (a test or review item that verifies the requirement). Each citation
  records the requirement version it was made against, and a newer version marks it suspect until
  re-confirmed. A workspace requirement can be split into project requirements, each refining it, and
  its coverage derives from theirs: covered when all are, in progress when any is, open otherwise. An
  agent may propose further splits, as drafts a person accepts. An agent proposes a citation through
  `requirement_cite` (R-MCP-2), as a draft a person accepts; amending or withdrawing a requirement is
  a requirement draft (R-ENT-14). Amended 2026-10-15 (`verifies`), 2026-10-17 (UI review,
  `docs/ui-redesign-findings-2026-10-15.md`) (splits) and 2026-10-18 (ANA-36) (`requirement_cite`).
- **R-ENT-16 (must).** Workspace-owned items: a workspace has its own key space and kinds, and owns
  items, documents and requirements that span its member projects, linked to their items by UUID.
  A workspace item has no repos: it plans, decides and splits, and code work goes to child items in
  member projects, linked by `parent` (R-ENT-9). Added by maintainer decision, 2026-10-15; amended by
  ANA-35 (repo-less workspace items).
- **R-ENT-17 (must).** Workspace components: a workspace declares the services its projects need —
  databases, caches, queues, cloud emulators, object storage, search engines, any image — picked from
  a catalogue held in Postgres, with a version and, for an emulator, the services or clouds it
  emulates (floci: AWS, Azure, GCP and Oracle, one container each, and its console). The catalogue is
  seeded at first start, updated on request from a catalogue file published in the `htui`
  repository and from the image registry's tags, and extended by the user with kinds of their own.
  `htui` generates a docker compose file in the workspace folder (R-ENT-2) with free ports,
  passwords from the secret provider and named volumes for data; per project, each component is the
  workspace's instance, the project's own instance, or unused. Each component starts and stops on its
  own; the stack is not tied to the workspace's lifecycle. Wherever an item runs, its components are
  started there (R-ORCH-15). Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-117.

## 4. Storage, cache and offline (R-STO)

- **R-STO-1 (must).** Postgres is the only writable store. Connection string, provider identities and
  the API key of each workspace's Qdrant live in the OS keyring (Windows Credential Manager, macOS
  Keychain, Linux secret service), never in a file — including when the connection string is typed into
  the TUI rather than passed to `htui --set-dsn`. Local files under the user's config directory that
  hold no secret and no domain data (the box identity, the read-only cache) are not a store. One
  exception, for a headless worker (R-ORCH-12) on a Linux host with no secret service: the DSN may be a
  systemd credential encrypted at rest and bound to the host (`systemd-creds encrypt`, read from
  `$CREDENTIALS_DIRECTORY`), or be read once from the worker's or a TUI session's stdin (`--dsn-stdin`)
  and held only in memory. For development and CI, the DSN may also come from an environment variable,
  which `htui` removes from every agent subprocess environment (R-SEC-2). Never in `argv` or a plaintext
  file. The same exception, and the development and CI environment variable, apply to a workspace's
  Qdrant API key (R-STO-8). The Qdrant URL is not a secret: it is a workspace setting in Postgres,
  carries no credentials, and may be overridden per process by a development or CI environment variable.
  Amended by maintainer decision on MOD-41, 2026-09-29, on MOD-45, 2026-10-01 (a TUI session's stdin),
  and on the requirements review, 2026-10-15 (environment variable; key bindings moved to Postgres), and
  on ANA-37, 2026-10-19 (Qdrant API key and URL).
- **R-STO-2 (must).** TLS to Postgres is supported and optional.
- **R-STO-3 (must).** Each box keeps a read-only cache of the projects and workspaces it has opened,
  workspace-owned rows included (R-ENT-16), under the user's config directory, refreshed on every
  successful connection. Contents: items, links, documents, attachment metadata (not their
  contents), notes, requirements and citations, run and step summaries, and transcripts of the last
  N steps (N configurable). Amended 2026-10-18 (ANA-35, workspaces and attachment metadata).
- **R-STO-4 (must).** When Postgres is unreachable, the TUI opens in offline
  read-only mode from the cache: browse items, graph, documents and cached transcripts. No item
  creation, no runs.
- **R-STO-5 (must).** Schema migrations are versioned, forward-only in version one, applied by
  `htui` on connect after confirmation. A headless process (one with no one to confirm, such as a
  worker) never migrates: when the schema is behind or ahead of the one it was built for, it refuses
  to start and reports why; an explicit, non-interactive `htui migrate` verb applies them from a
  shell. The per-box SQLite schemas for the read-only cache (R-STO-3)
  are versioned and forward-only, but answer a version mismatch by rebuilding. Amended 2026-10-15
  (`htui migrate`).
- **R-STO-6 (must).** With a warm cache, the first interactive frame appears in under one second on
  the reference workstation; connecting to Postgres and refreshing the cache may finish later in the
  background and never block input. ANA-32 measures it. Amended 2026-10-15 (was: under one second
  with a reachable Postgres).
- **R-STO-7 (withdrawn).** Local-only mode. Withdrawn by maintainer decision MOD-25, 2026-09-11.
- **R-STO-8 (must).** Knowledge search for the workspace. `htui` indexes what a workspace knows, across
  its projects: each item's key, title and body, its latest documents, its notes and the scrubbed
  transcripts of its runs (prompts, agent text and plans, not tool calls, tool output or edits);
  documents owned by a project or the workspace (R-ENT-12); workspace-owned items (R-ENT-16); each
  requirement's key, body and rationale (withdrawn ones included); and memory (R-ENT-13). Code and
  markdown files in a repo checkout are not indexed: they belong to the code-context tools (R-LATER-7)
  unless imported into Postgres (R-LATER-3). The index lives in the workspace's shared Qdrant (R-NF-2),
  with local dense embeddings and BM25 sparse vectors ranked together. Each workspace has its own
  collection, holding every member project. Searches cover the workspace by default and can be narrowed
  to projects; agents search through an MCP tool at workspace scope, within their read scope (R-MCP-1,
  R-MCP-2). A search can be narrowed to decisions: items closed as `done`, `concluded` or `rejected`
  (R-ENT-8), and their documents. The index is derived from Postgres and rebuildable from it (R-STO-1).
  One `htui` process per workspace indexes it and keeps it current. It is the holder of a workspace
  lease in Postgres, worker or TUI. Rows written are indexed within a short cycle, and a full reconcile
  catches deletions. When the workspace's Qdrant or the embedding model is unavailable, search answers
  "knowledge search unavailable" and nothing else is affected (R-NF-2). Drafting an item shows the
  nearest existing items across the workspace for a person to judge; nothing is written from a search
  score (R-ID-6). Added by maintainer decision on MOD-34, 2026-09-25 (`docs/ANA-19.md`,
  `docs/ANA-20.md`); amended by maintainer decision on MOD-50, 2026-09-29 (requirements indexed,
  decisions by resolution), on the requirements review, 2026-10-15 (notes, transcripts, workspace scope,
  agent tool), and on ANA-37, 2026-10-19 (workspace knowledge search, shared Qdrant, one indexer, a
  collection per workspace, the lease, freshness and related items).

## 5. Agent driver (R-AGT)

- **R-AGT-1 (must).** One `AgentDriver` trait: start a session (prompt, working directory,
  environment, tool exposure), stream typed events, send a follow-up, answer a permission request,
  cancel. Event kinds: assistant text, thought, tool call, tool result, edit proposal, permission
  request, plan or todo update, subagent start and end, file read, mode change, question to the
  user, usage, error, done. Amended 2026-10-15 (five kinds added).
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
  orchestrator or prompt code changes. A user adds a row from the Settings tab, with no `htui`
  release. Amended 2026-10-15.
- **R-AGT-6 (must).** Autodiscovery: on box registration and on demand, probe `PATH` for known
  agent binaries and ACP adapters, record version, mark enabled on that box. Manual entries allowed.
- **R-AGT-7 (must).** Quota tracking per agent: for `subscription` agents, remaining allowance or
  reset window as reported by the agent or its CLI, refreshed per run; for `per_token` agents, a
  configurable cap per run and per batch, enforced by cancelling the session when exceeded. The TUI
  shows cost and usage over time per agent, project and item. Amended 2026-10-15.
- **R-AGT-8 (must).** When a phase lists several candidate agents, the orchestrator picks the first
  in priority order whose quota is not exhausted and whose billing cap is not reached. When the
  phase fans out in compete mode (R-LATER-13), the remaining candidate slots are apportioned across
  the other eligible candidates by their per-model weight (ANA-21).
- **R-AGT-9 (must).** An agent that reports itself installed but unauthenticated is authenticated
  **from the app** wherever the agent's own protocol allows it, without leaving `htui` for a vendor
  CLI; where it does not, `htui` shows the exact command to run (amended 2026-10-15: best effort). The
  method is chosen from the ones that agent advertises; logging out is offered where the agent
  advertises it. `htui` triggers the flow and observes its outcome: it does not read, hold, transmit
  or store the credential, which stays wherever the agent keeps it (R-ID-7, R-SEC-2). Authentication
  is a fact about a box, not about a registry row. When the browser cannot reach the agent's
  loopback listener, the app relays the browser's redirect, which carries a one-use authorization
  code, to that listener once and holds it for that request only (MOD-22). Added by maintainer
  decision, 2026-09-09, reversing `docs/ANA-4.md` §4.5's conclusion that `htui` cannot log an agent
  in; implemented as MOD-21; amended 2026-09-30 by maintainer decision on MOD-22.
- **R-AGT-10 (must).** An agent whose adapter is not installed on a box can be installed **from the
  app**, from a source declared in its own registry row, with no code path per agent (R-AGT-5). The
  app re-probes after installing, so what the box can run is always the probe's answer rather than
  the installer's claim (R-AGT-6). Where the source publishes an integrity digest it is verified;
  where it does not, the user is told that before the download, not after. Where the adapter is
  proprietary, its licence and account-type consequences are surfaced before `htui` fetches it on
  the user's behalf. Besides a row's direct source, `htui` installs, detects and updates agents
  through the tool managers a box has — `mise`, `npm`/`volta`, `brew`, `winget`, `scoop` — preferring
  the one the box already uses, then the row's direct source; it shows the installed version beside
  the latest. Added by maintainer decision, 2026-09-09, reversing `docs/ANA-4.md` §4.6's
  deferral of registry-driven installation; implemented as MOD-20; amended 2026-10-15 (tool
  managers, updates).
- **R-AGT-11 (must).** A global htui agent, present in every workspace, manages `htui` itself:
  settings, Toolbox entries (R-TUI-7), agent rows, execution targets (installing and logging in
  agents on a remote box, setting up a docker context, R-ORCH-15) and the component catalogue
  (R-ENT-17). It acts without asking only on reads, catalogue updates and Toolbox additions;
  anything touching secrets, agent logins, installs on a remote box, a deletion or a target change
  waits for the person's permission. Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-121.

## 6. Orchestration (R-ORCH)

- **R-ORCH-1 (must).** A step graph is a named set of phases owned by a project or a workspace. Each
  phase has: candidate agents with model in priority order, fan-out count, gate, retry limit, input
  document kinds, output document kind, fan-out isolation mode, command queue mode, optional
  verification command. An item inherits its kind's default graph and may override it. Graphs may
  branch on conditions, run phases in parallel and loop; one graph may run several items, each as a
  subgraph; and a graph may be planned at run time by the agent that starts it, which then runs its
  plan directly within the bounds the project allows (phases, agents, caps), hard gates still
  enforced. Amended 2026-10-15 (was: an ordered list owned by a project); design in ANA-33.
  Beside kinds' graphs there is a global library of **workflows**, edited in the Toolbox (R-TUI-7):
  templates with steps, fan-out parts and typed links — `next`, `approved`, `asks you` (waits for
  the person), `if` (taken only when a condition holds), the loops `changes requested` and `fails`
  back to an earlier step, and `retries`, a loop on the same step (amended 2026-10-18, ANA-33). A
  step names a persona and either a specific agent or any agent; personas carry no agent. A workflow may pin an execution target (R-ORCH-15). When a run
  starts, a light model configured in Settings picks the item's workflow unless the item pins one,
  and the run carries it out: each loop taken becomes round 1, round 2, each fan-out one sub-step per
  part. The run's agent may change the workflow at run time, on its own or after asking, as a setting
  says. How workflows and kinds' step graphs fit together is settled by ANA-33 with ANA-40; workflows
  that drive a whole project are ANA-41. Amended 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).
- **R-ORCH-2 (must).** Gate is `always`, `on_failure` or `never`, plus a `hard` flag. A gated step
  ends in `awaiting_approval`; the user approves, rejects with a note, edits the artifact, or
  retries. A gate may ask a configured judge agent for an advisory verdict, shown on the gate; the
  person still decides. Auto mode (R-ORCH-6) downgrades every gate to `never` except those flagged
  `hard`. Amended 2026-10-15 (advisory judge).
- **R-ORCH-3 (must).** A failed phase loops back to the phase it declares as its loop-back target
  (a failed `review` to the preceding `implement` by default) with the failing artifact attached, up
  to the retry limit, then escalates to the user. Amended 2026-10-15 (any phase).
- **R-ORCH-4 (must).** Manual mode runs one selected item through its graph with gates as
  configured, and can run a single chosen step, skip a step, or restart from any step. Amended
  2026-10-15 (step control).
- **R-ORCH-5 (must).** Any step can be promoted to an interactive chat, on failure or by user
  request, preserving the session context. When the chat yields the step's artifact, the pipeline
  resumes from the next step.
- **R-ORCH-6 (must).** Auto mode is a queue runner: picks ready items by dependency order and
  priority, filters by current box capabilities, runs unattended with gates treated as `never`
  unless the user marked a gate as hard, escalates failures, and honors caps per item and per
  batch. When a subscription agent's quota is exhausted, it waits for the reset and resumes rather
  than failing the item. Supports a target box (R-ORCH-12). A schedule window is R-ORCH-13 (`later`):
  version one may store it but need not enforce it. Amended by maintainer decision, 2026-10-14, and
  2026-10-15 (quota wait).
- **R-ORCH-7 (must).** Fan-out divides work: a planner agent splits a step's work into parts, which
  run as parallel sub-steps of the same run on several agents, each in its own isolated tree
  (R-ORCH-8) and producing its own artifact; an integrate step then merges the parts' branches and
  artifacts, resolving conflicts, before review. Running N candidates on the same prompt and keeping
  one (best-of-N) is R-LATER-13. Amended by maintainer decision, 2026-10-15 (was: best-of-N); design
  in ANA-33.
- **R-ORCH-8 (must).** Fan-out isolation mode per project or phase: `worktree` (one git worktree
  per session), `copy` (directory copy for trees where worktrees break build caches),
  `shared_serialized` (one tree, sessions run one after another), `local` (run directly in the
  working tree with no isolation and no serialization). The default mode is set by the project type:
  `worktree` for software projects, `local` for the others (R-ENT-3). On a plain folder a `worktree`
  step copies the folder instead, and `shared_serialized` refuses it (R-ENT-4). Amended 2026-10-18
  (ANA-35, default per type and folders).
- **R-ORCH-9 (must).** Concurrent items: both modes may run several items at once when they do not
  overlap. Overlap: same repo unless both use isolated trees, or declared touched-path sets
  intersect. Overlapping items are serialized; others run in parallel up to a per-box limit.
- **R-ORCH-10 (must).** Capability matching: a run is refused when the item's required tags are not
  a subset of the box's tags, listing the missing tags, and `htui` offers to dispatch it to a box
  that has them (R-ORCH-12). Auto mode filters the queue the same way. Amended 2026-10-15.
- **R-ORCH-11 (must).** Every run records: item, target box, executing box, graph snapshot, and per
  step: agent, model, gate outcome, status, exit code, prompt digest, commit hashes, usage, timing.
- **R-ORCH-12 (must).** Remote dispatch: queue an item from one box to run on another. Requires a
  headless `htui` worker per box polling Postgres (R-ID-2). Promoted from later by maintainer
  decision on MOD-41, 2026-09-29; delivered by MOD-41 (worker, done) and MOD-43 (targeting, open).
- **R-ORCH-13 (later).** Scheduling: run the queue inside a time window on a chosen box.
- **R-ORCH-14 (must).** Coordinators: one per project and one per workspace, above item runs. A
  coordinator only manages: it follows what happens below it, talks with its children and its
  parent, and integrates a child's work when the child's run completes — merging, pushing or
  opening a pull request as the project configures — and runs close-out (R-TUI-9). It does no item
  work itself. Added by maintainer decision, 2026-10-15; design in ANA-34, which also settles what it
  changes in R-ID-6, R-ORCH-7, R-ENT-8 and R-TUI-9.
- **R-ORCH-15 (must).** Execution targets: each item runs on a target — this box (a worktree of the
  project), docker on this box (a fresh clone in a container with its own compose project and the
  toolchain image, as `scripts/hr` sandboxes run), a remote machine over SSH, docker on a remote
  machine, or, with R-ORCH-12's phase 2 (MOD-47), a box enrolled with an `htui` server. The default
  cascades Settings (this box) → workspace → project → workflow → item, each level inheriting unless
  it sets one. Wherever it runs, `htui` prepares it there — the code, the item's components
  (R-ENT-17), environment and secrets, and the agents, the htui agent (R-AGT-11) installing and
  logging in what is missing — and the work comes back when the person collects the branch, as
  `scripts/hr collect` does. Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); design in ANA-42,
  beside R-ORCH-12 (MOD-43) and the container environment (MOD-44).
- **R-ORCH-16 (must).** Stopping runs: a person can stop every run in a workspace, the open one
  included. A graceful stop halts each agent step before its next turn, and a check or one of
  `htui`'s own steps at once; a second, forced stop halts every step mid-turn. Both ask for
  confirmation, always save the conversation, and can be resumed: an agent step resumes its saved
  session (after a forced stop it redoes the interrupted turn) and a check runs again. Nothing new
  starts in the workspace until its runs are resumed. Added by maintainer decision, 2026-10-17
  (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-125.

## 7. Run history (R-HIS)

- **R-HIS-1 (must).** Every session event from R-AGT-1, plus the assembled prompt, every follow-up
  and every permission answer, is stored as an ordered row per run step, after scrubbing. Nothing
  about a run exists only on one box.
- **R-HIS-2 (must).** The chat view can reopen any past step read-only and replay it, and start a
  new chat or run from that step's context (fork). Amended 2026-10-15 (fork).
- **R-HIS-3 (must).** Retention is configurable per project; default keeps everything. Archived
  transcripts of finished steps follow the same window and sweeper (MOD-100); a transcript a person
  pins is kept. Amended 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).

## 8. Prompt assembly and skills (R-PRM, R-SKL)

- **R-PRM-1 (must).** Each step receives one self-contained initial prompt built by `htui`:
  phase template, item body, input documents from prior steps, summaries of upstream items reached
  by `blocked_by` and `origin` edges within one to two hops and inside the active workspace (or
  current project when no workspace), box profile, bound skill text, and any `htui`-selected file
  excerpts. Never raw transcripts of other items. Beyond this prompt, an agent can pull more context
  itself through `htui`'s MCP read tools (R-MCP-2). Amended 2026-10-15.
- **R-PRM-2 (must).** Upstream items outside the active workspace appear as one-line status stubs.
- **R-PRM-3 (must).** Per-step token budget. When the inputs exceed it, they are kept in a fixed
  priority order: the template and skills are never cut (a step whose template and skills alone
  exceed the budget is refused), then the item body, prior documents, upstream summaries and
  excerpts, the last cut first. The trim is recorded on the step. In manual mode, before a step
  starts trimmed, `htui` says so and lets the user raise the budget. Amended 2026-10-15 (reworded as
  a keep order, as ANA-5 §4.4 implements it; budget raise added).
- **R-PRM-4 (must).** Prompt templates per phase are versioned rows in Postgres,
  with a documented placeholder contract, editable in the TUI. Templates form a library shared
  across projects and workspaces, with per-project overrides. Amended 2026-10-15. The library is a
  group of the Toolbox tab (R-TUI-7), not a tab of its own. Amended 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).
- **R-SKL-1 (must).** Skill library in Postgres: name, description, versioned markdown body, and
  optional resources (scripts, references, other files). Amended 2026-10-15 (resources).
- **R-SKL-2 (must).** Skills attach at global, workspace, project, item-kind or phase level; the
  most specific attachment of a skill wins (phase over kind over project over workspace over
  global). An attachment pins a version or follows latest, and carries the activation (`always`,
  `glob`, `off`). Amended 2026-10-15 (workspace and kind levels).
- **R-SKL-3 (must).** TUI supports create, edit, view version diff, bind and unbind.
- **R-SKL-4 (must).** Import from existing `SKILL.md` files and from whole skill folders and Claude
  Code plugins, resources included. Amended 2026-10-15.

## 9. Secrets and scrubbing (R-SEC)

- **R-SEC-1 (must).** `SecretProvider` trait: list keys for a project path, resolve values,
  bootstrap a machine identity, report health. Infisical is the first implementation.
- **R-SEC-2 (must).** Scope is project secrets that the code under work needs, resolved at run start
  into the agent subprocess environment only. Agents never receive a tool that reads secrets, and
  `htui`'s own credentials (R-STO-1) are never exposed to a session.
- **R-SEC-3 (must).** Scrubber builds exact-match masks from every resolved secret value plus a
  pattern rule set for known key formats, runs before any transcript row persists, and blocks
  persistence when an unmasked pattern remains. It records a typed scrub refusal on the run —
  failing it, or failing the fan-out candidate, or parking the judge.
- **R-SEC-4 (must).** When the provider is unreachable, runs needing its secrets are refused with a
  clear error; there is no plaintext fallback.
- **R-SEC-5 (must).** The ports and URLs a workspace's components give each project (R-ENT-17) form
  an environment file that holds no secrets. When the provider is Infisical it lives there, one
  folder per project under the workspace's (`/<workspace>/<project>`), so a project and its runs read
  only their own; otherwise it sits beside the compose file in the workspace folder (R-ENT-2).
  Passwords come from the provider either way. Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).

## 10. htui MCP server (R-MCP)

- **R-MCP-1 (must).** `htui` exposes an MCP server to every session it launches. Every tool call is
  scoped to the run step owning the session: an agent writes only within its run, and may read beyond
  it — workspace items with their notes, documents and requirements, and, through knowledge search
  (R-STO-8), snippets of the scrubbed transcripts of the workspace's runs. Read results mark the author
  of any model text they return; a run's unaccepted drafts are returned only to that run; an upstream item outside the
  active workspace reads as a one-line stub (R-PRM-2). Amended 2026-10-15 (wider read scope), 2026-10-18
  (ANA-36) and 2026-10-19 (ANA-37, notes and transcripts through search).
- **R-MCP-2 (must).** Tools: `item_link` (propose an edge, kind), `item_status` (a typed request a
  person accepts), `document_write` (the step's output artifact), `note_add`, `box_profile` read,
  `command_run` and its asynchronous form (start, wait with a deadline, cancel) with reads of the
  stored command output and structured test results, `spawn_subagent` (consult); read tools for
  items, documents and requirements, with a text filter over their keys, titles and bodies that works
  without Qdrant, and knowledge search at workspace scope (R-STO-8), which says it is unavailable when
  the workspace's Qdrant is not available; `item_create`, which proposes an
  item as a draft a person accepts (R-ID-6); `requirement_cite`, which proposes a citation (R-ENT-15)
  a person accepts; a question to the user that parks the step (R-TUI-11, MOD-75); the planner's
  tools for declaring parts and next steps (R-ORCH-1, R-ORCH-7, ANA-33); and the coordinator's tools
  (R-ORCH-14): read the runs and waiting rows below it, write proposals into the close-out checklist,
  and request an enqueue or a link through a permission card. Any proposal an agent makes that a
  status move, the scheduler, coverage or a prompt would read is held until a person accepts it
  (R-ID-6). Each session is told which tools it has and when to use them. Amended 2026-10-15 and
  2026-10-18 (ANA-36, with ANA-34's coordinator tools) and 2026-10-19 (ANA-37).
- **R-MCP-3 (must).** `command_run` enqueues build, test and run commands behind per-box concurrency
  limits by command class (for example one C++ build, four test runs) and returns output, or a handle
  the session waits on with a deadline; the scrubbed output is stored on the step and readable by the
  run's sessions. A project may name command presets with their own timeout. Exposure per phase is
  `off`, `fan_out_only` (default) or `always`, and can be forced on by the item carrying the
  `heavy_build` label (R-ENT-5); `fan_out_only` exposes it when the session has live siblings in the
  run (compete candidates, divided parts, parallel nodes). When off, the tool is not advertised to the
  session. The exposure and the per-box limits are editable in Settings. Amended 2026-10-15 and
  2026-10-18 (ANA-36; ANA-33: the meaning of `fan_out_only`).
- **R-MCP-4 (must).** The prompt and the MCP server's instructions tell agents to use `command_run`
  for heavy commands when it is exposed; ACP permission handling denies matching direct shell
  commands in that case. Amended 2026-10-18 (ANA-36).

## 11. TUI (R-TUI)

- **R-TUI-1 (must).** Keyboard driven, mouse optional; keys are configurable (R-TUI-10). A one-row
  header: the open workspace or project (it opens the Workspaces tab), the tabs, then, dropping the
  least useful parts when narrow, the history chip (R-TUI-13), the working run count and
  waiting-on-you count (R-TUI-11), workspaces running in the background, the box, the store state —
  distinguishing online, connecting, and offline since T — and the layout. Tabs: Workspaces, Backlog,
  Chat, Toolbox, Logs, Settings. Each tab has an accent colour and item keys are coloured by kind;
  status shows as a glyph and a human word (`needs you`, not `awaiting_approval`), never by colour
  alone. Queue overlay for auto mode with reorder and pause. Leaving never stops work (R-TUI-18).
  Amended 2026-10-15 (Templates tab) and 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`): the Skills, Templates and
  Requirements tabs and the workspace switcher overlay give way to Toolbox (R-TUI-7) and Workspaces
  (R-TUI-14); Chat lists every session (R-TUI-6); Logs is new (R-TUI-15).
- **R-TUI-2 (must).** Backlog left pane: items grouped by project, with workspace-owned items
  (R-ENT-16) as their own group; filters by state (R-ENT-8), project, capability, readiness, labels
  and priority, and sorting by priority. Actions: new, edit, run, queue, close, open graph. The
  grouping cycles project, kind and status (attention order), items done (finished, close-out
  waiting) and closed stay hidden until shown, and both persist. A new item opens the modal of
  R-TUI-17. The detail says where the item runs and with which workflow, both changeable from the
  Backlog (R-ORCH-15, R-ORCH-1). Amended 2026-10-15, 2026-10-17 (UI review,
  `docs/ui-redesign-findings-2026-10-15.md`) and 2026-10-18 (ANA-35, derived states).
- **R-TUI-3 (must).** Backlog right pane is tabbed per selected item: **Body**, **Runs**, **Graph**,
  **Documents**, **Notes**, **Requirements** (the item's citations, suspect ones flagged) and
  **Attachments**, as chips in the pane's top border. Amended 2026-10-15 and 2026-10-17 (chips).
- **R-TUI-4 (must).** Runs tab: active and past runs for the item, step list with agent, model,
  gate state, usage, duration; a divided step shows its parts as a tree (planner, parts, integrate)
  with each part's state. Actions: approve, reject with note, retry, promote to chat, cancel,
  open artifact, run, skip or restart from a step (R-ORCH-4), fork from a step (R-HIS-2), and open
  an artifact, or a file from a tool call or diff, in `$EDITOR` at the line (R-ID-2). The run shows
  as a graph of its workflow as carried out (R-ORCH-1): `prepare` on its target, `pick workflow`,
  each loop taken as round 1, round 2, each fan-out part as a node, typed links (those into a
  running step animated), and `collect` when it ran off this box (R-ORCH-15); a step table is one
  key away. Enter on a step opens its live chat while it runs or waits, its archived transcript
  (read-only) once finished, and nothing before it starts. Amended 2026-10-15 (was: select fan-out
  result, which moves to R-LATER-13) and 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) (the run graph).
- **R-TUI-5 (must).** Graph tab: the item and everything connected, one to N hops, across projects,
  with status and link kind, navigable, including cited requirements and linked documents and
  attachments as nodes. It reads top to bottom from what comes first — parent requirements, the
  requirements the item covers, its blockers, the item, what follows it — with link kinds as a
  legend, as labels on the links, both or neither. Enter opens another item, the item's live chat, or
  a requirement in Workspaces (R-TUI-14). Every graph (R-TUI-4, R-TUI-5, R-TUI-7, R-TUI-14) opens at
  zoom 1 centred on what matters, pans and zooms by keyboard and mouse, has a dotted background, and
  lets nodes be moved by drag or keyboard without changing a link; inside tmux it says once how to
  let the mouse wheel through. Amended 2026-10-15 and 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).
- **R-TUI-6 (must).** Chat tab: streamed session with assistant text, collapsed thoughts, tool calls
  and their results' content (collapsible), edit proposals, permission prompts answered inline,
  follow-up input. Bound to a run step, or free-standing: then it runs against the active workspace,
  or the project when there is no workspace, whatever folder `htui` was started in, with the agent
  and model chosen when it starts. The tab lists every session in groups: the htui agent (R-AGT-11),
  the workspace coordinator, one coordinator per project (R-ORCH-14), item runs, and archived step
  transcripts (read-only). Coordinators always take messages; an item run's chat is view-only while
  the agent works and writable only when it asks the person something. Permissions are answered with
  `a` allow once, `A` always, `r` reject. Amended 2026-10-15 and 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`).
- **R-TUI-7 (must).** Toolbox tab: everything a run can use, grouped by kind — workflows (R-ORCH-1),
  skills (library, editor, version diff, a skill's resources, and a bindings matrix by workspace,
  project, item kind and phase), personas, templates (R-PRM-4), mods, MCP servers, hooks, commands,
  sub-agents, prompts, environment presets, and tools that bundle several of these (Gortex, Headroom,
  R-LATER-7). A workflow is shown and edited as a graph: add, edit, delete and connect steps by
  keyboard or mouse (dragging from a step's handle), change or delete a link, pin a target, with undo
  and redo of every edit. ANA-40 settles the kinds and how each is shown. Amended 2026-10-15 and
  2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) (was: a Skills tab and a separate Templates tab).
- **R-TUI-8 (must).** Settings tab: agent registry with quota, adding a row (R-AGT-5), and each row's
  isolation (R-ID-5); box profile with capability edits; project types, item kinds, and step graphs with
  the shared graph editor (the one the Toolbox opens, R-TUI-7; amended 2026-10-18, ANA-33); `command_run` exposure and per-box limits (R-MCP-3); key bindings (R-TUI-10); secret
  provider, caps, scheduler window, repo credentials (R-BOX-4), and the Postgres connection: whether a
  DSN is stored, a masked field to enter or replace it, an action to clear it, and the state of the last
  attempt. What is typed goes to the OS keyring and nowhere else (R-STO-1); it is not echoed, not
  logged, and not written to any file. Sections also hold the targets and the default target
  (R-ORCH-15), workflows (the light model, chosen from the models the configured agents offer; whether
  the agent's run-time workflow changes apply on their own or ask first; the default workflow), the
  component catalogue (R-ENT-17), and the workspace's Qdrant (its URL, stored in Postgres; a masked
  field for this box's API key, stored in the keyring; the state of the last attempt; a warning when the
  workspace has none; R-STO-8, R-NF-2); the connection section is named Postgres. Workspaces live in
  their own tab (R-TUI-14) and the Backlog's grouping and done items are Backlog keys (R-TUI-2). A field
  edits on its row with its help beneath; arrows change a choice or a toggle, saved at once. Amended
  2026-10-15, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) and 2026-10-19 (ANA-37).
- **R-TUI-9 (must).** Close-out writes the final summary document, sets status, records commit
  hashes from the run. No markdown files are produced. The coordinator (R-ORCH-14) runs close-out
  and its checklist — follow-up items, re-confirming suspect citations, choosing the resolution —
  and the merge, push or pull request; ANA-34 settles the split. Amended 2026-10-15.
- **R-TUI-10 (must).** Every key outside text entry is bound to a named action. The default
  bindings ship in `htui`. A user overrides them in the Settings tab; the overrides are stored in
  Postgres per user and shared by every `htui` that connects to it, and the read-only cache keeps the
  last known set, so the keys in force offline are the last ones seen online. Overrides list only
  the actions they change and may unbind one. Printable characters, editing keys inside a text
  field, a field's Enter and Esc, numbered choices, and a choice field's value keys are fixed.
  `htui` refuses an invalid set and names every error, including a chord bound twice on one screen
  and a printable chord bound where a text field is typing; `htui --default-keys` starts with the
  defaults instead. `ctrl-c` always quits, from every htui screen and field (the first press asks,
  the second leaves, R-TUI-18), and cannot be unbound or bound to anything else; a pane that runs another program in a terminal forwards
  it to that program and always keeps a key that leaves the pane. Every hint line and the `?`
  help show the keys in force, and the help opens from every screen. No default needs AltGr on common
  non-US layouts. Plain arrows never leave a pane; `Tab`/`Shift-Tab`, `Ctrl ←→` or a click move
  between panes, `Shift ←→` changes sub-tab, and digits switch tabs. Amended by maintainer decision,
  2026-10-15, reversing ANA-26's local `keys.toml` (MOD-67), and 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`) (navigation
  defaults).
- **R-TUI-11 (must).** One overlay, opened from every screen, lists every run, or item, in the
  active workspace that waits on a person — a gate, a fan-out selection, a judge failure, a blocked
  item, an open permission request, and an agent's open question — with the reason and the step, and
  opens the item's Runs tab on that step. The top bar shows the count. The list is computed from
  the store at read time. When something starts waiting, `htui` rings the terminal bell and raises
  an OS notification; notifications to a remote endpoint are R-LATER-12. Amended 2026-10-15 and
  2026-10-18 (ANA-35, blocked items).
- **R-TUI-12 (must).** Shell: panes have rounded borders with the title and sub-tabs in the top
  border and the focused pane's key hints in its bottom border; long rows end in `…`, long content
  scrolls with a scrollbar and stops at its end. A toggleable panel under the header says what the
  tab is for and its usual actions. Each tab has layout presets — list and detail, with an activity
  panel underneath (R-TUI-16), three columns, the detail alone — with splits resized by key or drag,
  panes shown or hidden, and a pane zoomed; the layout, the panel's state and modal sizes are saved
  the moment they change. When the layout hides something, the pane says how to bring it back.
  Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-115.
- **R-TUI-13 (must).** History: back and forward like a browser over every jump (Enter in a graph,
  the activity panel, a requirement's covering item or linked requirement) and every change of tab,
  sub-tab or pane — `⌫` or `Alt ←` back, `Alt →` forward; going somewhere new drops what was
  forward. The header names where back goes. Added by maintainer decision, 2026-10-17
  (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-115.
- **R-TUI-14 (must).** Workspaces tab: every saved workspace with its state — open, in the background
  with its runs going, stopping, stopped — and under it an overview and its projects. Opening one
  sends the current one to the background without interrupting its runs; stopping and resuming
  runs follow R-ORCH-16. Sub-tabs, per workspace and per project: overview (state, folder,
  description, goals, projects, components, default target), components (R-ENT-17: a table, the
  generated compose file, start and stop, each project's use) and requirements (R-ENT-14, R-ENT-15:
  a tree with nesting, each covering item and linked requirement a selectable row that jumps there,
  and a graph view). New workspaces (with their folder), projects (from a folder, a git URL or a new
  repo), components (a wizard over the catalogue) and requirements (R-TUI-17) are created here.
  Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-116, MOD-117, MOD-118.
- **R-TUI-15 (must).** Logs tab: `htui`'s own log, sources on the left with line and warning counts,
  lines on the right with coloured levels, a minimum level, follow on and off, and scrolling that
  pauses following. Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-122.
- **R-TUI-16 (must).** Activity panel, part of the layout presets (R-TUI-12): events newest first
  across workspaces, what needs the person pinned on top; Enter jumps to the item or chat (R-TUI-13);
  filters for all workspaces, needs you, failures and this workspace. Added by maintainer decision,
  2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-122.
- **R-TUI-17 (must).** Creating an item or a requirement opens a centred, resizable modal: the person
  describes it in their own words, with the project (for a requirement, the scope: workspace-wide or a
  project) preset from where they are; the light model (R-ORCH-1) rewrites it into an item (kind, key,
  title, goal, acceptance, open questions) or a requirement (what must hold, how to check it, what it
  refines); the person reviews, edits, rewrites again, or talks it through with an agent that asks
  questions and folds the answers into a live draft. A workspace requirement comes with proposed
  project requirements to keep or drop (R-ENT-15). The talk-it-through chat becomes the item's first
  chat. Nothing is saved before the person accepts it (R-ID-6). Added by maintainer decision,
  2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-118, MOD-119.
- **R-TUI-18 (must).** Leaving never stops work: `q` or `ctrl-c` opens a dialog listing every live
  workspace with its running and waiting counts; leaving closes the window and the runs keep going
  in the background, or are stopped gracefully first (R-ORCH-16) when the person chooses so. A
  second `ctrl-c` leaves. Added by maintainer decision, 2026-10-17 (UI review, `docs/ui-redesign-findings-2026-10-15.md`); MOD-115.

## 12. Later tier (R-LATER)

Designed for, not built in version one. Each needs its own `ANA-N` before implementation. R-LATER-3
and R-LATER-7 were promoted to must on 2026-10-15 and keep their IDs.

- **R-LATER-1.** Diff tab (working-tree diff per repo, structural diff optional) and read-only code
  explorer with multi-root tree.
- **R-LATER-2.** Issue tracker sync behind an `IssueSync` trait (OneDev, GitHub, others), both
  ways, by field ownership: each field has an owning side (for example title and body from `htui`,
  comments both ways, labels from the tracker). Amended 2026-10-15 (was: a downstream mirror only).
- **R-LATER-3 (must, promoted 2026-10-15).** Legacy import of markdown workflow repos (`HANDOFF.md`,
  `DECISIONS.md`, `docs/decisions/**`, `docs/ANA-*.md`) into projects, items, documents and links,
  mapping old prefixes to kinds per project, preserving keys. This repository is its first user.
- **R-LATER-4.** Scheduling (R-ORCH-13). Remote dispatch (R-ORCH-12) was promoted to must on
  MOD-41, 2026-09-29.
- **R-LATER-5.** Team support (R-USR-3).
- **R-LATER-6.** Additional agents beyond `claude`, `claude-cli` and `agy` via the registry (R-AGT-5).
- **R-LATER-7 (must, promoted 2026-10-15).** Code-context tools (Gortex, Serena, Graphify, Headroom)
  are mandatory in coding sessions and replace the agent's plain search and read tools there; they
  also serve as excerpt providers for R-PRM-1. When one is missing on a box, `htui` installs it
  (R-AGT-10's routes) or tells the user how. Design in ANA-39.
- **R-LATER-8.** A web front end over the same Postgres (R-ID-2).
- **R-LATER-9.** Multiplexer-like terminal panes: several agents, shells and editors side by side in
  `htui` (R-ID-2).
- **R-LATER-10.** Attachments in an object store, Postgres holding the reference (R-ENT-12).
- **R-LATER-11.** Item assignee and due date, with team use (R-USR-3, R-ENT-5).
- **R-LATER-12.** Waiting-on-you notifications to a remote endpoint (ntfy, chat), optional and
  failing open (R-TUI-11, R-NF-2).
- **R-LATER-13.** Compete fan-out: N candidates run the same prompt and a person or a judge agent
  keeps one result, the rest kept as history (the version-one R-ORCH-7 until 2026-10-15; ANA-21,
  MOD-36).

## 13. Non-functional (R-NF)

- **R-NF-1 (must).** Linux and Windows 10+. macOS is best effort: it builds and nothing may block
  it, but it gates no release. Amended 2026-10-15.
- **R-NF-2 (must).** Required services: Postgres, and one shared Qdrant per workspace with its embedding
  model, which runs in each process that searches or indexes (R-STO-8). Qdrant fails open: when a
  workspace's Qdrant is not configured or not reachable, `htui` starts and works, knowledge search says
  "knowledge search unavailable", and nothing else is affected; no run, prompt or close-out waits on it.
  While a workspace names no Qdrant, agents are not offered the search tool (R-MCP-2). Every box and
  process of a workspace uses that workspace's one Qdrant; none runs or names a Qdrant of its own,
  except a development or CI override (R-STO-1). Beyond Postgres, Qdrant and the agents, no required
  dependency on any external daemon, except the code-context tools in coding sessions (R-LATER-7). An
  error-reporting endpoint the user configures in Settings stays optional: it is off unless set,
  receives only scrubbed events (R-ID-7, MOD-99), and fails open with nothing else affected. Amended by
  maintainer decision, 2026-10-14, 2026-10-15 (context tools; Qdrant to ANA-37), and on ANA-37,
  2026-10-19 (one shared Qdrant per workspace, required, failing open).
- **R-NF-3 (must).** All long operations (probe, cache refresh, sessions) run off the UI thread;
  the TUI never blocks on network or subprocess I/O, and reflects a key press on screen within a
  bound set by ANA-32's measurement (order of 50 ms) under load. Amended 2026-10-15.
- **R-NF-4 (must).** Every item, whatever its prefix, cites as `addresses` citations the requirement
  IDs it addresses. Amended 2026-10-15 (was: `ANA-N` and `MOD-N` only).

## 14. Out of scope

- Hosting or managing the Postgres server itself.
- Editing code inside `htui` (it hosts the user's `$EDITOR`, R-ID-2).
- Exporting `HANDOFF.md` or other markdown views from the database.

## 15. Superseded material

- `docs/ANA-1.md`: schema, `items.json`, `artifact` tables, LWW conflict rule, `sync_state`,
  `external_issue_mapping` are superseded. Box registry, prompt injection contract, transcript
  split and scrubbing survive as restated in sections 2, 8 and 9.
- `docs/ANA-8.md`: three-tier hierarchy, workspace-centric topology and cross-project UUID links
  survive. Implicit workspaces, `items.json` v2, `htui-workspace.json`, the `V002` migration and the
  TUI mockups are superseded.
