# Requirements review with the maintainer (2026-10-15)

A walk through every requirement in `docs/REQUIREMENTS.md`, one at a time, asking the maintainer whether
each one says what they actually want. This file records the answers. The changes they call for are applied
afterwards: amendments go into `docs/REQUIREMENTS.md`, and code work is filed as HANDOFF items.

Key: **keep** = confirmed as written; **amend** = text changes; **new** = a requirement to add;
**work** = code or doc work, no requirement text change.

## 1. Identity

- **R-ID-1: amend.** Too narrow. htui covers any kind of project, not only coding: research/analysis,
  docs/writing, ops/infra, and any task an agent can do. Coding is the main case, not the only one.
  - Consequence: a project may have **zero repos** (R-ENT-3 amend); phases that need a repo refuse to
    run without one. This settles MOD-106 by amendment ("at least one repo" is dropped; "at most one
    primary" stays, and "exactly one primary when it has repos" is still enforced).
- **R-ID-2: amend.** Relax the identity:
  - Multiplexer-like features (several terminal panes: agents, shells, editors): allowed, **later tier**.
  - Code viewing (read-only code/diff): allowed, **later tier** (already R-LATER-1).
  - Hosted/web front end: allowed, **later tier** (a new R-LATER entry: a web front end over the same
    Postgres). Version one stays a TUI. The out-of-scope line "Any web or GUI front end" goes.
  - Code editing: htui still ships no editor of its own, and the "Editing code inside htui" out-of-scope
    line stays. The user's `$EDITOR` gets tighter integration: **open a repo file from an agent's tool
    call, edit proposal or diff in `$EDITOR` at the right line** (work item).
- **R-ID-3: amend.** Name the exceptions inside R-ID-3 itself (`keys.toml`, the box identity, the
  read-only SQLite cache, the Qdrant index derived from Postgres), so the rule no longer reads as absolute.
- **R-ID-4: keep.**
- **R-ID-5: amend.** Isolation stays the default. A registry row may opt out (today
  `settings.user_extensions`); an opted-out agent must show as such in Settings. ANA-31 still settles how
  `agy` is isolated.
- **R-ID-6: amend (loosen).** A model may also help with import and triage (mapping legacy items,
  suggesting links, and the like), provided a person accepts the result before it is written. Sync,
  cache and bookkeeping stay deterministic, and nothing unaccepted is ever written.
- **R-ID-7: amend (widen).** Say explicitly that it covers **any outbound or persisted data**: telemetry,
  error reports, logs and cache, not only transcripts and tool output. Scrubbing still fails closed.

## 2. Users and boxes

- **R-USR-1, R-USR-2, R-USR-3: keep.** Single user in v1; team use stays later and must not be blocked.
- **R-BOX-1, R-BOX-2: amend (probe more).** Add to the probe: language runtimes (`node`, `python`, `go`,
  `java`), package managers, free disk space, and network reachability.
- **R-BOX-3: amend + work.** Decouple `heavy_build`: as an item label it exposes `command_run`, and it is
  **not** also a required box capability. This answers ANA-28 (option "a non-routing item label").
- **R-BOX-4: amend (new behaviour).** htui can **clone a repo onto a box** that lacks it (from the repo's
  remote URL) and record the per-box path. When the box has no credentials for that remote:
  - **detect and explain:** recognise the auth failure, name the remote/host and how to fix it (SSH key,
    token, `gh auth`), and mark the repo "not available on this box";
  - **in-app setup:** guide the user through setting credentials from htui. Either generate an SSH key
    and show the public key to add, or store an HTTPS token in the OS keyring for git to use. htui keeps
    no copy outside the keyring (R-STO-1, R-SEC-2).

## 3. Entity model

- **R-ENT-1, R-ENT-2: amend.** A workspace is **not a path root**. It is only a named group of
  projects that are connected in some way, and the projects and folders in it can live anywhere on the
  machine. Drop "with a per-box root path" from R-ENT-2, and drop "workspace root" from R-BOX-4. The
  rest of R-ENT-2 is kept: no implicit workspaces, and a single project falls back to "the current project".
- **R-ENT-3: amend.** A project may have zero repos (see R-ID-1). New: a project carries a **type**
  (software, research, docs, ops, ...) that seeds its item kinds and step graphs.
- **R-ENT-5: amend.** Add an explicit **priority** field (R-ORCH-6 already picks "by priority"), and
  free **labels** kept apart from the required capability tags (they carry `heavy_build`, see R-BOX-3).
  Assignee and due date are noted for **later**, with team use (R-USR-3).
- **R-ENT-6: amend.** The seeded kinds depend on the project type: software projects keep today's set,
  non-coding types get their own. The first types and their seeds (the prefixes are proposals to settle
  in the item that builds this):
  - **software:** ANA, FEAT, FIX, CLEAN, TOOL (today's set);
  - **research:** e.g. RES (research → verdict), Q (question → answer);
  - **docs:** e.g. DOC (outline → draft → review);
  - **ops:** e.g. OPS (plan → execute → verify), INC (investigate → fix → postmortem).
- **R-ENT-4: amend.** A project location is **a git repo or a plain folder**. Git-only features
  (worktree fan-out, commit hashes) need a repo; `copy` and `local` isolation work for folders.
- **R-ENT-7: keep.**
- **R-ENT-8: amend (simplify statuses).** Item status becomes **open / blocked / closed** (+ resolution).
  `queued`, `in_progress`, `awaiting_approval` and `failed` move to the **run**, and `done` is no longer
  a status (it stays a resolution). Large change: it touches the store schema, the orchestrator, the
  Backlog filters, R-TUI-2 and R-STO-8's "decisions" filter.
- **R-ENT-9: amend.** New item→item link kinds: **parent/child** (decomposition, unlike `origin`, which is
  provenance), **duplicates** and **implements**, plus links whose target is a **document or attachment**.
  Still no manual link action: agents and the importer only. Requirement↔item links are already R-ENT-15
  citations (explained to the maintainer).
- **R-ENT-10: keep.**
- **R-ENT-11: amend.** Notes are human **and agent** comments (`note_add` over MCP already exists).
- **R-ENT-12: amend.** Documents can belong to an **item, a project or a workspace**. New:
  **attachments** (non-markdown files: images, logs, ...) on items and documents, stored in Postgres with
  a size cap for now and in an object store later (a later-tier entry).
- **New (R-ENT): workspace-owned items and documents.** A workspace has its own key space and kinds
  (e.g. `WS-1`), owns items and documents that span its member projects, and links to items in those
  projects. This touches R-ENT-1..3, R-ENT-7 and R-PRM-1 scope.
- **R-ENT-14: amend.** Requirements can belong to a **project or a workspace** (requirements that cut
  across projects).
- **R-ENT-15: amend.** Add a citation kind **`verifies`** (test or review items that verify a
  requirement), which allows a coverage view.

## 4. Storage, cache and offline

- **R-STO-1: amend.** Allow the DSN from an **environment variable**, for dev and CI convenience.
  Constraint carried with it: htui strips that variable from every agent subprocess environment, so
  R-SEC-2 ("htui's own credentials are never exposed to a session") still holds.
- **R-STO-2, R-STO-3, R-STO-4: keep.**
- **R-STO-5: amend.** Add an explicit, non-interactive **`htui migrate`** CLI verb, so a headless box can
  be migrated from a shell. The worker still never migrates by itself.
- **R-STO-6: amend (reframe).** The budget is **first interactive frame from the cache in under 1 s**;
  the Postgres connect and cache refresh may finish later in the background. ANA-32 measures this.
- **R-STO-8: amend (extend).** Also index **notes** and **scrubbed run transcripts**; index and search
  **workspace-owned** items, documents and requirements, scoped by workspace; expose search to agents as
  an **MCP tool** (R-MCP-2).

## 5. Agent driver

- **R-AGT-1: amend.** Add event kinds: **plan/todo updates, subagent start/end, file read, mode change,
  and questions to the user**. Some of these may already exist in code; check before filing.
- **R-AGT-2, R-AGT-3: keep.**
- **R-AGT-4: keep.** The v1 agents stay `claude` (ACP), `claude-cli` and `agy`; others stay R-LATER-6.
- **R-AGT-5, R-AGT-6: amend.** Say explicitly that a user can **add a new agent row from the Settings
  tab**, with no code release.
- **R-AGT-7: amend.** Add a **cost/usage view over time** (per agent, project and item) in the TUI.
- **R-AGT-8: keep.**
- **R-AGT-9: amend (best effort).** In-app login wherever the agent supports it. Where it doesn't,
  htui shows the exact command to run. MOD-105's login half is then optional and no longer owed.
- **R-AGT-10: amend (tool managers).** Besides a row's direct source, htui installs and detects agents
  through the box's **tool managers**: **mise** (e.g. this box: `claude` and
  `aqua:google-antigravity/antigravity-cli`), **npm/volta** (e.g. `claude-agent-acp`), and
  **brew/winget/scoop**. It also offers **updates** through the same manager and shows installed vs latest.
  When several routes exist, **the manager the box already uses wins**, then the row's direct source.
  Autodiscovery (R-AGT-6) must find mise/volta installs and shims. The row-data rule (R-AGT-5) still holds.

## 6. Orchestration

- **R-ORCH-1: amend (must; ANA first).** Step graphs grow beyond a linear list:
  - **branching graphs:** conditional branches, parallel phases, loops;
  - **workspace graphs:** owned by a workspace and shared across its projects;
  - **multi-item subgraphs:** one graph runs several items, each as a subgraph;
  - **dynamic, agent-planned graphs** (like Claude Code's workflow): the agent that starts the graph
    decides which steps to take, and **runs its plan directly within bounds**: the phases, agents and
    caps the project allows, with hard gates still enforced. This needs an R-ID-6 note: the agent picks
    *what work to do*, which is not bookkeeping; status transitions stay deterministic.
  - An ANA is filed now; its result ships in v1.
- **R-ORCH-2: amend.** New gate option: a **model judge** gives an **advisory** verdict on the gate. It is
  shown on the gate, and the person still decides.
- **R-ORCH-3: amend (generalise).** Any phase can declare a **loop-back target on failure**, not only
  review → implement. It still carries the failing artifact, the retry limit and escalation.
- **R-ORCH-4: amend.** Manual mode gains **step control**: run a single chosen step, skip a step, or
  restart from any step.
- **R-ORCH-5: keep.**
- **R-ORCH-6: amend (quota-aware).** When a subscription quota runs out, auto mode **waits for the reset
  and resumes** instead of failing the item. Scheduling stays later (R-ORCH-13).
- **R-ORCH-7: amend (major, redefine fan-out).** Fan-out means **division of work**, not best-of-N:
  - **divide (must):** a **planner agent** splits an item's work into parts. The parts are **parallel
    sub-steps inside one run**, like Claude Code's ultracode: different parts of the same item are
    implemented by several agents at once, each in its own isolated tree (R-ORCH-8). A following
    **integrate step** (an agent) merges the parts' branches and artifacts and resolves conflicts, then
    review runs.
  - **compete (later):** today's best-of-N (same prompt, a person or judge keeps one result) is kept in
    code but is no longer a must. ANA-21's weights, MOD-36 and MOD-37 then belong to the later tier.
  - This overlaps the dynamic graphs of R-ORCH-1. Both go into the same ANA.
- **R-ORCH-8: keep** (isolation modes apply per divided part).
- **R-ORCH-9: keep.**
- **R-ORCH-10: amend (auto-route).** Instead of only refusing a manual run on a box missing the tags,
  htui **offers to dispatch it to a box that has them** (R-ORCH-12). It still lists the missing tags.
- **R-ORCH-11, R-ORCH-12: keep.** MOD-101 (exit code) and MOD-43 (targeting) close the audit gaps.
- **R-ORCH-13, R-LATER-4: keep** (scheduling stays later).

## 7. Run history

- **R-HIS-1: keep.**
- **R-HIS-2: amend.** Add **fork from history**: start a new chat or run from any past step's context.
- **R-HIS-3: keep.** MOD-100 adds the sweep.

## 8. Prompt assembly and skills

- **R-PRM-1, R-PRM-2: amend.** Keep the assembled self-contained prompt (MOD-102 makes the engine follow
  the workspace), **and** add MCP read tools so an agent can pull more context when it needs it (item
  read, search, documents), scoped as R-MCP-1 allows. This pairs with the agent search tool (R-STO-8).
- **R-PRM-3: amend (reword + new behaviour).** Reword it as a **keep order**: template and skills are
  never cut (a step whose template and skills alone exceed the budget is refused); then item body, prior
  documents, upstream summaries and excerpts, with excerpts cut first. The current text reads as if
  template and skills were cut first, which the code (`htui-core/src/prompt/trim.rs`, ANA-5 §4.4) rightly
  does not do. New: **in manual mode**, before a step starts trimmed, htui shows that it would be
  trimmed and lets the user **raise the budget** first.
- **R-PRM-4: amend.** Add a **template library**: templates shared across projects and workspaces (a
  global library, like skills) with per-project overrides.
- **R-SKL-1, R-SKL-2: amend.** Add **workspace-level** and **item-kind-level** attachments (the
  most-specific-wins order to settle: phase > kind > project > workspace > global). Skills may carry
  **resources** (scripts, references, extra files), not just one markdown body.
- **R-SKL-3: keep.**
- **R-SKL-4: amend.** Import **whole Claude Code plugins / skill folders**, resources included, not
  only single `SKILL.md` files.

## 9. Secrets and scrubbing

- **R-SEC-1: keep.** MOD-104 implements the machine-identity bootstrap.
- **R-SEC-2, R-SEC-3, R-SEC-4: keep.**

## 10. htui MCP server

- **R-MCP-1: amend (wider read scope).** Agents may **read** beyond their run's item (workspace items,
  requirements, documents). **Writes** stay scoped to the run.
- **R-MCP-2: amend (more tools).** Beyond the search and read tools (R-STO-8, R-PRM-1):
  - **`item_create`:** propose new items (follow-ups, child items) as drafts a person accepts (R-ID-6);
  - **plan/graph tools:** the planner declares parts and next steps (feeds divide-mode fan-out, R-ORCH-7,
    and dynamic graphs, R-ORCH-1);
  - **ask the user:** a tool to put a question to the person (feeds the waiting-on-you list, R-TUI-11);
  - **plus an ANA** to research other kinds of tools agents should get.
- **R-MCP-3: amend.** Behaviour kept (explained to the maintainer in plain terms). The per-phase
  exposure (`off` / `fan_out_only` / `always`) and the **per-box command-class limits** become
  **editable in Settings** (today they are set only in Postgres, and Settings › Kinds shows them read-only).
- **R-MCP-4: keep.**

## 11. TUI

- **R-TUI-1: keep.** MOD-103 delivers one Chat tab per session.
- **R-TUI-2: amend.** Filter and sort by **labels** and **priority**; show **workspace-owned items** as
  their own group. (The status filter follows the simplified R-ENT-8.)
- **R-TUI-3: amend.** New right-pane tabs: **Attachments** and **Requirements** (the item's citations,
  suspect ones flagged).
- **R-TUI-4: amend.** Show a divided step's **parts as a tree** (planner → parts → integrate) with
  per-part status; add **step control** actions (run, skip, restart from a step; R-ORCH-4) and **fork
  from history** (R-HIS-2); **open in `$EDITOR`** an artifact, or a file from a tool call or diff, at the
  right line (R-ID-2).
- **R-TUI-5: amend.** The graph also shows **cited requirements and linked documents/attachments** as
  nodes.
- **R-TUI-6: amend.** **Tool results show their content** (collapsible), and starting a free-standing
  chat lets the user **pick the agent and model**.
- **R-TUI-7: amend + new tab.** The bindings matrix covers global, workspace, project, kind and phase,
  and a skill's view lists its resources. Templates get **their own Templates tab** (with the template
  library of R-PRM-4); R-TUI-1's tab list gains it.
- **R-TUI-6 (follow-up): amend.** A free-standing chat (one attached to no item) starts against the
  **active workspace**, or against the project when there is only one. The folder htui was started in
  does not matter, so MOD-103's "pick a project" part becomes "use the workspace".
- **R-TUI-8: amend.** Settings also gains: a **step-graph editor** (branching and dynamic graphs need more
  than a list), **project types** with their seed kinds and graphs, **workspaces** (members, kinds,
  graphs), and the **key bindings** (see R-TUI-10). Earlier answers add the agent rows (R-AGT-5),
  the `command_run` limits (R-MCP-3) and repo credentials (R-BOX-4).
- **R-TUI-9 + merge/PR: new concept, the coordinator (ANA first).** The maintainer asked where merge
  and PR happen. Today the orchestrator merges a step's branch into the local primary checkout
  (ANA-2 §4.6), close-out only records hashes, and htui never pushes or opens a PR. The maintainer wants
  **a coordinator** above item runs, with one at **project level** and one at **workspace level**. It
  listens to what happens below, talks with its children and parents, and **the parent integrates when a
  child's run completes**. It **only manages**; it does no item work itself. Close-out's checklist
  (follow-up items, re-confirming suspect citations, picking the resolution), merge, push and PR are its
  job. File an **ANA** for it, which also establishes which requirements it changes (at least R-ID-6,
  R-ORCH-1/7, R-TUI-9 and R-ENT-8). R-TUI-9's text waits for that ANA.
- **R-TUI-10: amend (reversal).** Key bindings are edited **in the Settings tab** and stored **in
  Postgres per user**, shared by every htui that connects. They are **cached locally**, so offline uses the
  last known keys. `keys.toml` is dropped. This reverses ANA-26 / MOD-67's local-file choice, and R-STO-1's
  carve-out for `keys.toml` goes with it. Validation (no chord bound twice on a screen, `ctrl-c` fixed,
  `--default-keys`) still applies.
- **R-TUI-11: amend.** **Notifications** when something starts waiting on the user: **terminal bell**
  and **OS notification** (notify-send, macOS, Windows toast). Remote push (ntfy, Telegram, Slack, ...)
  is a **later** requirement: optional external endpoint, fails open (R-NF-2).

## 12. Later tier

- **R-LATER-1: keep** (later).
- **R-LATER-2: amend.** Issue tracker sync becomes **two-way**, by **field ownership**: each field has an
  owner side (for example title and body from htui, comments both ways, labels from the tracker). It is no
  longer "downstream mirror only". Postgres is still where htui reads from. Still later.
- **R-LATER-3: promote to must.** Legacy markdown import (MOD-8): this repo is its first user.
- **R-LATER-4, R-LATER-5, R-LATER-6: keep.**
- **R-LATER-7: promote to must + amend.** Context tools (**Gortex**, Serena, Graphify, Headroom) are
  **mandatory for coding sessions** and **replace grep and the plain read/search tools**. When one is not
  present on the box, htui **installs it or tells the user how** (the same install routes as R-AGT-10).
  This is no longer fail-open, and it conflicts with R-NF-2 (see section 13).
- **New later entries (R-LATER-8..):** web front end over the same Postgres (R-ID-2); multiplexer-like
  terminal panes (R-ID-2); object storage for attachments (R-ENT-12); assignee and due date with team
  use (R-ENT-5); remote push notifications (R-TUI-11); compete / best-of-N fan-out (R-ORCH-7).

## 13. Non-functional

- **R-NF-1: amend.** **Linux and Windows** are must. **macOS is best effort**: it should build and must
  not be blocked, but it is no release gate.
- **R-NF-2: amend.** Required: Postgres, the agents, and the **context tools** for coding sessions
  (R-LATER-7, now must; htui installs them or explains how). Qdrant: required **if it proves very
  beneficial**, which an **ANA decides** by measuring what search adds (agent search tool, triage, dedup,
  the R-ENT-13 memory). Until then it stays optional and fails open. The error-reporting endpoint stays
  optional.
- **R-NF-3: amend.** Add a **measurable responsiveness bound**, e.g. a key press is reflected on screen
  within 50 ms under load. The figure is to settle with ANA-32's measurement.
- **R-NF-4: amend.** Applies to **every item prefix** (TOOL, CLEAN, NEXT, VAL, ...), not only ANA and
  MOD. TOOL-13 grows accordingly.

## 14. Out of scope

- **Keep** "Hosting or managing the Postgres server itself", "Editing code inside `htui`" and "Exporting
  `HANDOFF.md` or other markdown views from the database". **Remove** "Any web or GUI front end" (now a
  later-tier entry, R-ID-2).

## Withdrawn requirements

- **R-ENT-13: reopen via ANA.** Background given to the maintainer: ANA-1's atomic facts with importance
  scores, dropped 2026-09-03 as over-engineered. The ANA studies **project/workspace memory** (facts,
  decisions, gotchas) for the coordinator and agents: who writes it, how it stays fresh, R-ID-6, and
  **integration with Qdrant**.
- **R-STO-7: keep withdrawn.**
