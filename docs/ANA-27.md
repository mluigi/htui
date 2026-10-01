# ANA-27 - OpenRig survey: ideas and concepts to assimilate

> **Scope note:** "Survey OpenRig (https://openrig.dev/, `@openrig/cli`), a Node/tmux CLI where a
> team of Claude Code and Codex agents is declared in a file: one lead agent delegates to durable
> specialists that own tasks (`rig queue`), consults them directly without creating a task
> (`rig send`), and spawns temporary subagents for bounded work, all inside a "pod", with a TUI
> for workspace visibility. [...] Compare each concept with what htui already has or has planned:
> the orchestrator and run_step fan-out, weighted agent assignment (MOD-36), personas (MOD-26),
> swarm (MOD-27), the permission relay (MOD-42), worker crash recovery (MOD-24), and the Runs
> pane. Verdict: list the ideas worth taking, each with a fit note and either a proposed MOD item
> or an amendment to an existing open item, and list the ideas rejected, with reasons."
> (`HANDOFF.md:141-154`, ANA-27.)
>
> **Requirements addressed:** `R-ID-2`, `R-ID-3`, `R-ID-4`, `R-ID-5`, `R-ID-6`, `R-AGT-4`,
> `R-AGT-7`, `R-AGT-8`, `R-ORCH-2`, `R-ORCH-4`, `R-ORCH-5`, `R-ORCH-7`, `R-PRM-1`, `R-HIS-1`,
> `R-HIS-2`, `R-MCP-2`, `R-TUI-1`, `R-TUI-4`, `R-TUI-6`, `R-NF-1`, `R-NF-3`.
>
> **Status (2026-10-01): concluded.**
> Verdict: OpenRig is a different kind of product. It keeps long-lived interactive Claude Code
> and Codex sessions in tmux panes and moves work between them by **typing into their terminals**,
> with a SQLite queue as the durable record. htui is a deterministic orchestrator over
> single-turn, structured (ACP or stream-json) sessions, with Postgres as the record. OpenRig's
> three load-bearing ideas each conflict with a recorded htui decision, and all three are
> **rejected**: the **LLM lead** that assigns and closes work (`R-ID-6`, ANA-2 invariant 3),
> **durable specialist sessions** (`R-PRM-1`, MOD-24's rescope), and **tmux transport** (`R-ID-2`,
> `R-NF-1`).
> What transfers is the discipline OpenRig wraps around its durable rows:
> - **the row carries the obligation; the wake is only a pointer**;
> - **no agent answers a prompt, and an answer names the request it answers**;
> - **liveness is derived from rows, and "unknown" is a value**;
> - **a requested resume never silently becomes a fresh start**;
> - **a step yields only through a typed exit, and the code creates its successor in the same
>   transaction**;
> - **a profile can only select from what its base declares**.
>
> These become amendments to seven open items: MOD-42, MOD-43, MOD-37 (two entries), MOD-27,
> MOD-26, MOD-16 R-10 and MOD-24. There is one new item, **MOD-69**, a list across items of
> every run waiting on a person, which needs one new requirement (`R-TUI-11`) and a matching
> line in `R-TUI-1` (§7, for maintainer approval). Also rejected: a team file in the repo, an
> agent message bus, untracked native subagents, least-loaded selection, agent-driven restart,
> and writes to the harness's own config.

Code citations are against HEAD `f3aaa18`.

---

## 1. Context and problem statement

OpenRig appeared in spring 2026 (first commit 2026-03-23, first npm release 2026-04-06) as a
fast-moving open-source "local control plane for multi-agent coding topologies". It offers a
set of answers to questions htui has open items for: how to declare a team of agents, how work
is handed between them, how a lead consults a specialist, how a stuck permission prompt is
handled, and how a crash is recovered. ANA-27 asks three things (`HANDOFF.md:146-154`):

1. Pin down OpenRig's model from its source and docs (§2).
2. Compare each concept with htui's shipped and planned features (§3, §4).
3. Decide what to take and what to reject (§5).

One correction to the brief, from OpenRig's own taxonomy: the **pod is not the team**. The team
is a **rig**. A pod is a sub-group of seats inside a rig with shared guidance and continuity
policy ("A bounded context group within a rig", https://openrig.dev/specs/taxonomy). Also, "lead"
and "specialist" are **not schema types**: they are roles carried by guidance text and launch-order
edges (§2.3).

Sources. Two surveys were made on 2026-10-01:

- **Docs and web.** The openrig.dev site documents 0.5.14 ("These pages document OpenRig
  0.5.14", https://openrig.dev/docs). The repo docs track `main` at `065df99`.
- **Source.** npm `@openrig/cli@0.6.3` (published 2026-09-30) and
  https://github.com/mvschwarz/openrig at tag `v0.6.3` (`8b5e948`). Below, `or:<path>:<line>` is a
  file at that tag. A bare file name with a line, such as `queue-repository.ts:1278`, is a daemon
  source file under `packages/daemon/src/` as the source survey recorded it, located by name.
  `or-main:` is `main@065df99`.

Where the two disagree, the code at v0.6.3 is taken as authoritative and the difference is noted.

## 2. OpenRig's model

### 2.1 Licence, repo, maturity

- **Licence.** The code is Apache-2.0 (`or:LICENSE`, `package.json` `"license": "Apache-2.0"`;
  https://registry.npmjs.org/@openrig/cli). The specification documents are CC-BY-4.0 ("The open
  specification is CC-BY-4.0", https://openrig.dev/docs/faq). The bundled Claude plugin
  `openrig-core` is also Apache-2.0
  (`or:packages/daemon/assets/plugins/openrig-core/.claude-plugin/plugin.json`).
- **Author and repo.** Mike Schwarz (mvschwarz), https://github.com/mvschwarz/openrig. He calls
  himself "the founder of OpenRig" in the second Show HN post
  (https://news.ycombinator.com/item?id=48241066).
- **Stack.** A Node/TypeScript monorepo. `packages/daemon` is a Hono HTTP daemon on SQLite
  (`better-sqlite3` 13) and tmux, with about 400 non-test domain source files (282 entries in
  `src/domain/`) and 89 migrations. `packages/cli`
  holds the `rig` command and an MCP server, and is the only published package. `packages/tui` is
  a hand-rolled TUI whose only dependency is `yaml`. `packages/ui` is a React web UI in
  maintenance mode. "OpenRig is a local daemon + CLI + terminal UI + MCP server, built on tmux."
  (`or:README.md`)
- **Platforms.** Node 22 or 24 plus tmux, on macOS or Linux. Native Windows is not supported and
  WSL2 is untested (`or:README.md:21-37`).
- **Maturity.** The first commit was on 2026-03-23. The repo has 3,159 commits. The largest
  committer is an agent identity, `v-openrig-build` (1,534), followed by mvschwarz (1,081) and
  Mike Schwarz (457). There have been 52 npm versions (51 stable plus one rc) since 0.0.1 on
  2026-04-06, and the cadence
  rose to daily in late September (0.5.17 to 0.6.3 between 09-27 and 09-30).
  - "pre-1.0 minor releases may include contract additions, deprecations, and behavioral changes"
    (`or:CHANGELOG.md`). 0.6.0 dropped Node 20.
  - The CLI has 85 top-level commands and 18 MCP tools (`or-main:ARCHITECTURE.md:117-127`).
  - The project is popular but young: about 3,500 stars, 236 forks and 90 open issues and PRs
    (GitHub API, 2026-10-01). A reported "781 stars in a day" is **UNCONFIRMED**. There is no
    public roadmap (**UNCONFIRMED** beyond scattered follow-up notes). Its own author wrote: "The
    project is still early" (https://news.ycombinator.com/item?id=47772935).

### 2.2 The team-definition file

There are two YAML formats, plus an optional culture file.

**RigSpec** (`rig.yaml`, `version: "0.2"`) declares the team: "pods, members, startup layering,
cross-pod relationships, and optional managed services" (https://openrig.dev/specs/rigspec).

- **Parsing.** It is parsed by `rigspec-codec.ts:1` and validated by hand in
  `rigspec-schema.ts`, which rejects unknown keys at all four levels (`rejectUnknownTopologyKeys`,
  `rigspec-schema.ts:78`).
- **Top level** (`rigspec-schema.ts:47-50`):
  - `version`, `name`
  - `summary`, `culture_file`, `docs`
  - `permission_policy` (`builtin:locked|standard|open|yolo` or a file)
  - `managed_blocks`, `startup`, `services` (Docker Compose only), `workspace`
  - `pods`, cross-pod `edges`
- **Pod** (`:51`): `id`, `label`, `summary`, `continuity_policy`, `startup`, `members`, and
  pod-local `edges`.
- **Member** (`:52-56`, validated at `:426-588`):
  - Required: `id`, `agent_ref` (`local:`, `path:` or `builtin:terminal`), `profile`, `runtime`
    (`claude-code|codex|pi|terminal|stub`, `rigspec-preflight.ts:144`) and `cwd`.
  - Optional: `label`, `model`, `role`, `codex_config_profile` (Codex only), a member
    `permission_policy`, `restore_policy` (`resume_if_possible` by default, `relaunch_fresh` or
    `checkpoint_only`; `:40`), `compaction_strategy`, `mechanic`, `startup`, `starter_ref`, and
    `session_source` (`fork`, `rebuild` or `agent_image`, `:631-746`).
- **Edges.** `delegates_to`, `spawned_by`, `can_observe`, `collaborates_with` and `escalates_to`
  (`:30`). Only the first two order the launch. "They do not route messages or constrain who
  `rig send` can target. They do not enforce permissions or access control."
  (https://openrig.dev/specs/edge-types) Where launch order is computed is **UNCONFIRMED** in code.
- **Startup blocks.** Rig, pod and member each carry `files` and `actions`. Actions must declare
  `idempotent`, and `shell` actions are rejected. The layers apply in this order: agent, profile,
  rig, culture, pod, member, operator (https://openrig.dev/specs/agent-startup-guide).
- **Stale reference.** `docs/reference/rig-spec.md` says it was last validated against code on
  2026-04-11, though it has been edited since. It omits `role`, `session_source`, `workspace`,
  `starter_ref` and the `pi` and `stub` runtimes. The code is authoritative.

Shipped example, verbatim (`or:packages/daemon/specs/rigs/preview/product-team/rig.yaml`):

```yaml
version: "0.2"
name: product-team
culture_file: CULTURE.md
summary: >
  Advanced product-development starter: a human-operated product squad with
  two orchestrators, implementation, QA, design, and two independent reviewers.
  Use this when you want richer coordination than the conveyor training rig.

pods:
  - id: orch1
    label: Orchestration
    members:
      - id: lead
        agent_ref: "local:../../../agents/orchestration/orchestrator"
        runtime: claude-code
        profile: default
        cwd: "."
      - id: peer
        agent_ref: "local:../../../agents/orchestration/orchestrator"
        runtime: codex
        profile: default
        cwd: "."
    edges: []

  - id: dev1
    label: Development
    members:
      - id: impl
        agent_ref: "local:../../../agents/development/implementer"
        runtime: claude-code
        profile: default
        cwd: "."
      - id: qa
        agent_ref: "local:../../../agents/development/qa"
        runtime: codex
        profile: default
        cwd: "."
      - id: design
        agent_ref: "local:../../../agents/design/product-designer"
        runtime: claude-code
        profile: default
        cwd: "."
    edges:
      - kind: delegates_to
        from: impl
        to: qa

  - id: rev1
    label: Review
    members:
      - id: r1
        agent_ref: "local:../../../agents/review/independent-reviewer"
        runtime: claude-code
        profile: default
        cwd: "."
      - id: r2
        agent_ref: "local:../../../agents/review/independent-reviewer"
        runtime: codex
        profile: default
        cwd: "."
    edges: []

edges:
  - kind: delegates_to
    from: orch1.lead
    to: dev1.impl
  - kind: delegates_to
    from: orch1.lead
    to: dev1.design
  - kind: delegates_to
    from: orch1.lead
    to: rev1.r1
  - kind: delegates_to
    from: orch1.peer
    to: dev1.qa
  - kind: delegates_to
    from: orch1.peer
    to: rev1.r2
  - kind: can_observe
    from: rev1.r1
    to: dev1.impl
  - kind: can_observe
    from: rev1.r1
    to: dev1.design
  - kind: can_observe
    from: rev1.r2
    to: dev1.impl
  - kind: can_observe
    from: rev1.r2
    to: dev1.qa
```

**AgentSpec** (`agent.yaml`, `version: "1.0"`) is "a single-agent blueprint ... declares the
available resource pool, named profiles that select from that pool, startup content, and
lifecycle defaults" (https://openrig.dev/specs/agentspec).

- **Resources.** Skills; guidance, merged into `CLAUDE.md` or `AGENTS.md`; native subagent files;
  hooks; and runtime fragments. Each fragment is written to a fixed place:
  `.claude/settings.local.json`, `.mcp.json`, or a managed block in `~/.codex/config.toml`
  (`or:docs/reference/agent-spec.md`).
- **Profiles.** "Profiles filter. They never inject resources not declared in the base spec."
  (https://openrig.dev/specs/taxonomy) "A profile may override startup and lifecycle defaults"
  (same page), so a profile can add no resource but is not narrow-only. An AgentSpec "does not
  define multi-agent topology" and holds no secrets.

**CULTURE.md** holds "coordination norms for the group" as free text (`or:README.md`).

### 2.3 Lifecycle and task ownership

- **One seat, one session.** Every member is a **seat**: one long-lived interactive harness in
  its own tmux session, named `{pod}-{member}@{rig}` (`session-name.ts:1-2`). The seat is the
  stable address. The **occupant** is the current conversation, tracked by a generation UUID.
  "the seat outlives the session" (https://openrig.dev/specs/taxonomy).
- **No lead or specialist in code.** No type exists for either. "Lead" is guidance plus
  `delegates_to` edges (`or:skills/_canonical/pods/orchestration-team/SKILL.md:62-78`).
- **Launch.** `NodeLauncher.launchNode` creates a tmux session holding a bare shell with
  `OPENRIG_*` environment variables (`node-launcher.ts:95-238`, `tmux.ts:458-468`). It records the
  binding in one SQLite transaction. The adapter then **types** the harness command into that
  shell (`claude-code-adapter.ts:219-312`, `codex-runtime-adapter.ts:323-438`). Claude starts with
  an OpenRig-generated `--session-id`, so the resume token is known before launch. The Codex
  thread id is recovered by reading Codex's private SQLite logs (`codex-thread-id.ts:236-273`).
- **Temporary subagents are invisible to OpenRig.** Seats use their harness's own subagents
  (Claude `Agent` calls, Codex `spawn_agent`). OpenRig only copies their definition files into
  `<cwd>/.claude/agents` (`claude-code-adapter.ts:524`). "The helpers have no OpenRig addresses or
  queue ownership." (https://openrig.dev/) The activity oracle deliberately filters Claude
  `SubagentStop` (`or:docs/reference/agent-state-taxonomy.md:69`). The only temporary seat OpenRig
  manages is `rig fork`, which copies a live seat's context into a new seat
  (`or:packages/cli/src/commands/fork.ts:34`). Nothing reaps it automatically.
- **Task ownership is push-addressed.** A queue row has exactly one `destination_session`.
  - `claim` succeeds only for that destination, from `pending` or `blocked`. It stamps the
    claimant's occupant generation and sets `closure_required_at` from a tier SLA
    (`queue-repository.ts:2057-2137`). The tiers are: fast 30 min, routine 4 h, deep 24 h and
    critical 15 min (`hot-potato-enforcer.ts:114-127`).
  - "Hot-potato" closure: `done` needs a `closure_reason` from a closed set. Where the reason
    points somewhere, it also needs a `closure_target` (`hot-potato-enforcer.ts:1-101`). The code
    adds `superseded` to the six reasons in the docs.
  - Handoff closes the source row and opens a row at the target in one transaction ("A lost
    handoff ... is impossible by construction", https://openrig.dev/specs/taxonomy).
  - "Closure records delivery, not acceptance" (https://openrig.dev/docs/coordination).
  - The transition log is append-only by convention (`025_queue_transitions.ts:3-11`, "no UPDATE
    / DELETE on this table from domain code"). No trigger enforces it.
  - A seat handover releases the old generation's in-progress claims back to `pending`
    (`queue-repository.ts:3454-3486`).
- **Consult without a task.** "Use rig send to ask that specialist directly ... without creating
  a new task for a simple question." (https://openrig.dev/) "A message informs. Work that another
  seat must act on belongs in the queue." (https://openrig.dev/docs/messaging)
- **Workflows.** A workflow is declared steps with `allowed_exits` (`handoff`, `waiting`,
  `failed`), exit-keyed `next_hop.on` branching, and a `max_hops` cycle guard
  (`or:docs/as-built/architecture/workflow-runtime.md`;
  `or:packages/daemon/src/builtins/workflow-specs/conveyor.yaml`). "The owner decides when a step
  is done; the runtime is the scribe, not the gate." (https://openrig.dev/docs/workflows)
  - There is **no parallel fan-out primitive** in the step language.
  - Role resolution is least-loaded: running seats that match the role and runtime, ordered by
    ascending pending backlog, with ties broken by codepoint order of the seat name
    (`workflow-role-resolver.ts:1-30`). There is **no weighting**.

### 2.4 Transport

"Agents talk through their terminals ... The terminal is the wire." (https://openrig.dev/what-is)

- **`rig send`** (`session-transport.ts:892-1365`) runs these steps:
  1. Refuse if the pane's foreground process is a bare shell (`:995-1008`).
  2. Probe `has-session`.
  3. Classify readiness from a hook signal, trusted only if under 15 s old (`:27`), with a
     `capture-pane` regex as fallback. **Refuse** if the target shows a permission or selection
     prompt, unless `--dangerously-interact --reason` is given, which is audited. Send with an
     advisory if the target is busy.
  4. Paste through a temp file with `load-buffer` and `paste-buffer -d -r -p`.
  5. Sleep 200 ms.
  6. Press `C-m` (`tmux.ts:471-516`).

  The message is wrapped in a `From:`/`To:` envelope. The sender comes from the
  `X-OpenRig-Session` header, which the CLI stamps from the seat's environment. The daemon labels
  each row with how the sender was derived (`transport:v1` from the header, `claimed:v1` when it
  is absent) and delivers either way (`routes/require-sender-identity.ts:1-60`).
- **Queue delivery.** After a create or handoff commits, the daemon pastes a short **pointer**
  into the destination's pane, `Queue handoff: <qid> - check your queue.`
  (`queue-repository.ts:1278`). The agent then runs `rig queue show` and `rig queue claim` itself.
  - The invariant is stated as "THE ROW CARRIES THE OBLIGATION EXACTLY-ONCE; THE WAKE IS
    AT-LEAST-ONCE" (`queue-wake-ladder.ts:10-12`).
  - A failed wake retries every 300 s, up to three times. It then escalates to the destination's
    orchestrator and then to the operator (`queue-wake-ladder.ts:1-63`).
  - Queue verbs are **not** MCP tools. Agents drive them through `rig ... --json` in Bash
    (`or:packages/cli/src/mcp-server.ts`).
- **Typing guard.** An opt-in per-seat guard, for seats a person types in by hand, pauses all
  automatic input. Messages sent meanwhile are retained in the outbox, not delivered, and are not
  replayed when the guard is turned off (`seat-delivery-guard.ts`; `session-transport.ts:898`).
- **Fragility.** Every harness UI change can break the transport. Examples: Claude's
  `[Pasted text #N +X lines]` placeholder, the Codex footer regexes, the 200 ms delay, and
  verification by substring count. Separately, the arteries doc's delivery row records repeated
  regressions in the delivery guards themselves: #142 (typing into a bare shell), its fix #150
  then refusing managed Codex seats (#171) and explicit-mode Claude seats (#197, fixed by #220),
  plus target-resolution bugs #141, #174 and #188 (`or-main:docs/as-built/arteries.md:37`).
- **Transcripts.** The docs describe `pipe-pane` logs. The code at v0.6.3 replaced them with a
  periodic `capture-pane` rotation (`transcript-rotation.ts:4`, `:80`).

### 2.5 Progress reporting and the TUI

- **Agent state** (`or:docs/reference/agent-state-taxonomy.md:12-101`) has three independent axes:
  - session: present, detached, exited or absent;
  - activity: working, idle-at-prompt or unknown, plus **needs-input as `{count, reason}`, never
    a state**;
  - resumability: live, resumable or context-walled.
- **Evidence.** States come from ranked, time-bounded evidence: Claude's undocumented
  `~/.claude/sessions/<pid>.json`, then lifecycle hooks, then visible prompt chrome, with pane
  scraping only as a fallback. The doc says outright that "pane-scraping as primary truth" is
  rejected.
- **Derived diagnoses** are computed at read time:
  - PARKED: idle or needs-input while the seat holds open obligations;
  - HELD: a deliberate park with a wake armed;
  - DONE-UNSEEN.

  Pickup receipts (`unclaimed`, `working`, `stalled-after-claim` after 3 min by default, and
  `parked`) are derived from rows and never self-reported (`queue-pickup.ts:1-60`).
- **Stuck sweep.** Every 300 s a sweep raises findings: unconsumed waits, overdue claims,
  stalled-after-claim, undelivered wakes, unclaimed obligations after 60 min, and dangling
  closures. Each finding
  becomes a queue row routed to its owner (`queue-stuck-sweep.ts:33-60`).
- **Hooks and telemetry.** Activity hooks POST with a bearer token. They send event type and
  subtype (such as the tool name), seat identity and time, but no prompt text
  (`or:packages/daemon/assets/plugins/openrig-core/hooks/scripts/activity-relay.cjs:44-77`).
  Claude's `statusLine` command is replaced with a collector that taps context and rate-limit
  usage (`claude-code-adapter.ts:719-748`).
- **TUI** (`rig tui`, "k9s for rigs", `or:packages/tui/README.md`):
  - It has an explorer over hosts, rigs, pods, seats, specs and missions, and tabbed detail views
    (table, recent, overview, graph, health, topology, yaml).
  - It has an **attention view** that "shows human requests separately from outcome and health
    updates" (https://openrig.dev/docs/tui).
  - It reads daemon projections and an SSE stream only (`or-main:ARCHITECTURE.md:101-105`). "It
    does not start agents, launch terminals or resume seats by itself."
    (https://openrig.dev/docs/tui)
  - The exact seat-table columns (runtime, model, context and state, per the README) are
    **UNCONFIRMED** in code.

### 2.6 Crash recovery

- **Detection.** At daemon boot, `Reconciler.reconcile` marks sessions whose tmux session is
  gone as `detached` (`reconciler.ts:44-98`). tmux sessions survive a daemon restart but not a
  host reboot (`or:packages/daemon/specs/rigs/launch/kernel/rig.yaml`). The FAQ's claim of
  "session persistence across reboots" contradicts the kernel spec.
- **No automatic restart.** Restart is started by an operator or an agent: `rig up` restores from
  the latest snapshot, `rig launch` restarts one seat, and `rig start` or the TUI crash-cart
  restores the fleet (`crash-cart-conductor.ts:1-30`). Snapshots are taken every 300 s and the
  last 10 are kept.
- **Resume honesty.** When a resume fails, or no token or adapter is available, the blank session
  is rolled back and the seat reports `awaiting-decision`. OpenRig never silently fresh-starts
  ("failed resume is FAILED loudly. No automatic fresh fallback",
  `or:docs/as-built/architecture/lifecycle-snapshot-restore.md:94`; implemented at
  `restore-orchestrator.ts:1196-1249`). A Claude resume-selection menu yields
  `attention_required` and is never answered automatically.
  - The source's outcome set is resumed, rebuilt, fresh, fresh-primed, awaiting-decision,
    attention_required, failed and operator_recovered (`types.ts:475`). It differs from the docs'
    list.
- **Restart-safe sweeps.** Queue state is in SQLite, and the wake ladder and stuck sweep derive
  their position from the transition log, so a daemon restart neither forgets nor resets counts
  (`queue-wake-ladder.ts:14-17`).
- **Process checks.** To verify that a pane really runs `claude` or `codex`, OpenRig reads
  `ps -Ao pid,ppid,pgid,tpgid,ucomm,lstart,command` and accepts a process whose argv carries the
  seat's session or resume token. It then fingerprints that process's lineage (pid, ppid, start
  time, pgid, tpgid, command) and requires the fingerprint to match across two back-to-back `ps`
  observations (`native-process-lineage.ts:124-201`). It does not record a pid for later checks.
- **Test coverage.** CI pins "queue baton survives daemon restart" with a seeded fault, but
  restore, delivery (the #197 class) and rig identity (the #174 class) have **no** scenario
  coverage yet (`or-main:docs/as-built/arteries.md:71-82`).

### 2.7 Permissions

- **Launch floor.** Claude runs with `--permission-mode acceptEdits` and Codex with
  `-s workspace-write`. Full bypass is opt-in (`yolo-mode.ts:36-67`; `or:README.md`). Per-seat
  `rig seat set-permissions` is audited and applies at the next launch.
- **No relay in the human-approval sense.** OpenRig detects prompts through the Claude
  `Notification` hook, the Codex `PermissionRequest` hook and pane regexes. It surfaces each as
  needs-input `{count, reason}`. A human opens the pane and answers natively
  (`or:docs/reference/getting-started.md`).
- **Agents no longer answer prompts.** Agent-to-agent prompt answering was once a feature ("the
  orchestrator can just click accept"). It is now refused by default: "OpenRig now refuses to
  answer another agent's permission prompt by default ... only as a deliberate override (`rig
  send --dangerously-interact`, with a reason), and it's recorded."
  (https://openrig.dev/blog/orchestrator)
- **Writes to user config.** OpenRig writes trust and onboarding state into `~/.claude.json` and
  Codex `config.toml`. It also writes the statusLine and hooks into `.claude/settings.local.json`,
  and managed blocks into `CLAUDE.md` (`claude-code-adapter.ts:654-675`, `:751-770`;
  `or:README.md:105-116`).

## 3. Current state in htui

### 3.1 Orchestrator, steps and fan-out

- **Run kinds.** `RunKind` has two variants, `Graph` and `Chat`
  (`crates/htui-core/src/model/run.rs:12-20`), and the database check matches
  (`crates/htui-store/migrations/0001_init.sql:451`). There is no swarm, team or lead kind.
- **The walk.** A run walks a frozen `graph_snapshot` (ANA-2 invariant 2, `docs/ANA-2.md:109-113`).
  `Engine::run_to_rest` re-reads every row on each pass and holds no state between calls
  (`crates/htui-orch/src/engine.rs:5-11`, `:2717-2802`).
- **Fan-out is rival only.** One prompt goes to N candidates, run in parallel by `drive_group`
  (`engine.rs:3614-3700`). Selection goes through `fanout::route`: no survivor, a human, an
  automatic win, or a judge (`crates/htui-orch/src/fanout.rs:242-276`). The judge is a real step
  at `fanout_index = -1`. Task fan-out was left as a requirement change (`docs/ANA-2.md:259`).
- **Hand-off is documents only.** `phase_spec` feeds a step its input documents through the
  store's `resolve_inputs` (`engine.rs:5249-5290`), "never raw transcripts of other items"
  (`R-PRM-1`, `docs/REQUIREMENTS.md:286-290`).
- **No consult.** There is no way for one session to consult another without a step. Every
  session belongs to a `run_step` (`crates/htui-agent/src/driver.rs:253`, `R-MCP-1`). ANA-13
  rejected a hub bus because it breaks replay (`docs/ANA-13.md:58`, `R-HIS-2`).
- **Caps.** `max_fan_out` defaults to 4 and `max_agents_per_run` to 8, both overridable in
  `app_setting` (`crates/htui-orch/src/graph.rs:30-35`).
- **Bookkeeping is code.** Gate resolution, status moves, admission, selection bookkeeping and
  close-out are deterministic code. The judge is the only exception, and it picks a `fanout_index`
  and nothing else (`docs/ANA-2.md:114-118`; `CONCEPTS.md:36-37`; `R-ID-6`).

### 3.2 Agent assignment

- **The walk.** `select::walk` skips candidates with no agent row, an exhausted quota, no inline
  approval on a gated phase, not ready, or over budget (`crates/htui-orch/src/select.rs:129-140`).
- **The selector.** The `AgentSelector` seam is asked once per `fanout_index`. Its only
  implementation, `FirstCandidate`, returns `eligible.first()` for every index
  (`engine.rs:155-176`). `SnapshotCandidate` has no weight
  (`crates/htui-core/src/model/run.rs:524-532`).
- **MOD-36** (`HANDOFF.md:227-247`) plans ANA-21's deterministic apportionment: slot 0 stays
  `eligible[0]`, and the rest go by weight, then by agent not yet used, then by priority, with no
  randomness, quota or clock (`docs/ANA-21.md:405-422`; `R-AGT-8`,
  `docs/REQUIREMENTS.md:213-216`).
- **No per-agent cap.** There is no cap on concurrent sessions per agent. Batch caps belong to
  MOD-12 (`HANDOFF.md:331`).

### 3.3 Sessions and transport

- **The seam.** `AgentDriver` and `AgentSession` form a structured event seam
  (`driver.rs:358-439`). The events are `AssistantChunk`, `ToolCall`, `PermissionRequest`,
  `Usage`, `Done` and others (`crates/htui-agent/src/event.rs:226-252`).
- **Transports.** ACP is JSON-RPC (`R-AGT-2`, `docs/REQUIREMENTS.md:197`; transport in
  `crates/htui-agent/src/acp/mod.rs`). The CLI fallback is `claude -p --output-format
  stream-json` (`crates/htui-agent/src/cli/mod.rs:1-16`, flags at `:119-128`). There is no PTY
  and no tmux.
- **Every graph step is a fresh, single-turn session.** The spec carries `resume: None`,
  `mcp: Vec::new()`, `tools: ToolExposure::default()` and `permission: PermissionPolicy::default()`.
  The step runs one `pump` to `done` (`engine.rs:5646-5666`, `drive_once`).
- **Resume is honoured by one transport only.** `SessionSpec.resume` exists
  (`driver.rs:272-273`), but only the CLI driver reads it. The ACP driver ignores it, so
  promotion never asks ACP to resume: `promote::opening_kind` picks the handoff opening for every
  transport other than `cli`, on purpose (blueprint D192; `crates/htui-orch/src/promote.rs:57-72`,
  pinned by the test at `:312-320`). A promoted ACP step therefore gets the handoff prompt and a
  fresh context (MOD-37 R-48, `HANDOFF.md:202-205`). `DriverCaps.resume` cannot stand in for this
  check: for ACP rows it comes from `settings.acp.session.resume`, which defaults to `true`
  (`crates/htui-agent/src/registry.rs:165`).
- **Deadlines are checked late.** `deadline_seconds` limits the verify command and is checked at
  settle, but nothing times the agent session itself (`crates/htui-orch/src/gate.rs:280-290`;
  `engine.rs:3530-3547`). `drive_once` starts no timer.

### 3.4 Permissions

- **The engine path cannot answer.** `pump` records events until `done` and never answers a
  request (`crates/htui-agent/src/record.rs:1838-1873`). Engine-driven ACP steps therefore fail
  on their first permission request.
- **Only chat answers today.** The TUI chat path answers in process and records who answered
  (`permission_answer.by`, user or policy; `crates/htui/src/agent_worker.rs:4022-4054`,
  `crates/htui-agent/src/record.rs:207-220`). `AgentSession::cancel` also answers every
  outstanding request `cancelled` (`driver.rs:432-433`), and that answer is recorded as `Policy`
  today (`record.rs:217`). The event kinds already exist in the schema (`0001_init.sql:524-527`).
- **`PermissionPolicy`** holds a default, rules and remembered answers (`driver.rs:186-194`).
- **MOD-42** (`HANDOFF.md:477-488`) plans `permission_request` and `permission_answer` rows plus
  command rows, over ANA-16 §5.5's channel: durable rows read by `seq` cursor, with
  `LISTEN`/`NOTIFY` hints and a poll backstop (`docs/ANA-16.md:389-402`). It has no PRD yet. Its
  sources do not say what happens to a request nobody answers.

### 3.5 Progress and the Runs pane

- **Placement.** The Runs pane is a sub-tab of the Backlog item detail
  (`crates/htui/src/ui/tabs/backlog/detail/runs.rs:1-30`). Each step row shows slot, status,
  phase, usage, duration, gate and agent/model (`:91-109`).
- **Invalidation only.** `RunFrame`s carry no data, and the pane re-reads on every frame except
  `Subscribed` (`runs.rs:566-578`). Frames fire at run creation, at session end, at rest, on
  adoption, on a command's change and on an error, but not when a step starts: `pending →
  running` is not signalled (`crates/htui-worker/src/views.rs:147-166`; MOD-37 R-40,
  `HANDOFF.md:191-193`).
- **Refresh, not liveness.** The pane refreshes every 5 s while a run is live, but "this is a
  re-read, not a liveness signal: a run queued on a worker box with no worker running stays
  `queued`, and the pane cannot tell you why (MOD-43)" (`docs/htui-worker.md:138-141`).
- **No cross-item view.** Nothing lists what waits on a person across items. The top bar counts
  active runs only (`R-TUI-1`, `docs/REQUIREMENTS.md:333-336`). MOD-12 is open and unbuilt, and
  its planned escalation list lives only in auto mode's queue overlay (`HANDOFF.md:331-337`).

### 3.6 Crash recovery

- **Leases and sweeps.** The run lease has a 120 s TTL and refreshes every 60 s
  (`crates/htui-orch/src/recover.rs:22-33`). `htui worker` polls every 5 s and sweeps expired
  leases automatically (`crates/htui-worker/src/worker.rs:12`; `engine.rs:2105-2135`).
- **Per step, from artefacts.** A step whose `after_hash` and output document exist is marked
  done. Any other single step is reset to `before_hash` and retried. An unfinished fan-out
  candidate is interrupted and its group routes over the survivors (`engine.rs:2358-2420`). A
  judge step is re-parked, never re-judged.
- **MOD-24's rescope.** Context re-hydration is dropped. What remains is an end-to-end kill test
  plus the MOD-53 chat case (`HANDOFF.md:453-475`).
- **Orphaned children.** A SIGKILLed orchestrator leaves its agent child alive. The design (a pid
  in `session_started` plus a signal on adoption, or a death signal, with the reused-pid hazard
  stated) belongs to MOD-16 R-10, which also notes that signalling a stale pid needs `unsafe` or a
  dependency (`HANDOFF.md:421-425`).

### 3.7 Planned items that overlap

- **MOD-26 personas** (`HANDOFF.md:249-251`): a frontmatter parser, discovery from
  `~/.config/htui/agents.d/`, and mapping to `SessionSpec` overrides. ANA-16 recommends that
  personas be registry rows rather than a per-box directory (§6.2, §8). The item names only two
  keys (model, tools) and defines no precedence, no narrowing rule and no body semantics, and
  cites no requirement IDs. ANA-13's "`DriverCaps` overrides" (`docs/ANA-13.md:42`) is a category
  error: `DriverCaps` are transport predicates (`driver.rs:319-330`).
- **MOD-27 swarm** (`HANDOFF.md:252`; `docs/ANA-13.md:50-51`, `:59`): `RunKind::Swarm` and
  `spawn_subagent` as a `RunStep` with a `parent_step_id`, plus optional isolated worktrees.
  ANA-13's adopted design also has "the orchestrator maintains a single shared context window"
  (`docs/ANA-13.md:59`). Nothing of it is built. `R-MCP-2` already lists the tool
  (`docs/REQUIREMENTS.md:321-323`). Lifecycle, limits, quota and recovery of child steps are
  unaddressed.
- **MOD-43** (`HANDOFF.md:489-493`): the Runs view follows `session_event` and shows worker
  liveness. It is blocked on MOD-42.
- **MOD-55** (`HANDOFF.md:532-539`): a human-initiated one-shot consultation of an agent from the
  template editor.

## 4. Concept-by-concept comparison

| OpenRig concept | htui today / planned | Gap | Fit |
|---|---|---|---|
| Rig file in the repo (RigSpec, CULTURE.md) | Step graphs and agent registry rows in Postgres; personas planned (MOD-26) | No team object, by design | **Reject** the file (`R-ID-3`, `R-ID-4`) |
| AgentSpec profiles that filter and never inject | MOD-26 personas, no precedence or narrowing rule | Persona semantics unsettled | **Take** the idea into MOD-26 as a narrow-only rule (T9) |
| Pod (a sub-group of seats) | Box, project and run already partition work | None that matters | **Reject**: no unit to map to |
| Seat vs occupant generation; claims stamped by generation | Run lease owner plus `StepFence` (MOD-41) | Covered differently | None |
| Lead / orchestrator agent | `Engine`, deterministic code | By design | **Reject** (`R-ID-6`, ANA-2 inv. 3) |
| Durable specialist sessions | Fresh single-turn session per step, `resume: None` | By design | **Reject** (`R-PRM-1`, MOD-24) |
| Native temporary subagents, untracked | MOD-27 `spawn_subagent` as a `RunStep` | Planned | **Reject** OpenRig's form; keep ANA-13's |
| `rig queue`: push-addressed row, typed closure, handoff in one transaction | Engine creates steps; status moves are compare-and-sets; MOD-27 baton unspecified, ANA-13 plans a shared context window | Swarm yield semantics open | **Take** typed exit and same-transaction successor into MOD-27 (T6) |
| `rig send`: consult without a task | None; ANA-13 rejected a bus; MOD-55 is human-side only | Gap | **Take** as MOD-27 consult mode (T7) |
| Pointer wake, row is the obligation | MOD-42/43 plan rows plus `NOTIFY` hints and a poll backstop | Planned, unstated as a rule | **Take** into MOD-42 (T1) |
| tmux paste transport, pane scraping | ACP and stream-json structured events | None | **Reject** (`R-ID-2`, `R-NF-1`, fragility) |
| Safe-input rule; audited `--dangerously-interact` | Chat answers in process; engine path fails | MOD-42 | **Take** into MOD-42 (T2) |
| needs-input `{count, reason}`; PARKED; stalled-after-claim | Status columns; 5 s re-read; no liveness | MOD-43 | **Take** into MOD-43 (T3) |
| Attention view across seats | Per-item Runs pane; active-run count in the top bar | Gap, no owner | **New**: MOD-69 (T8) |
| Tier SLA, `queue overdue` | `deadline_seconds` bounds verify and is checked at settle; the session itself is untimed | Hung session holds a slot | **Take** into MOD-37 (T4) |
| Watchdog wakes typed into seats; stuck sweep as queue rows | Worker sweep adopts expired leases | No idle session to wake | **Reject** the wakes; the display half goes to T3 |
| Snapshot plus native resume; never a silent fresh start | Reset-and-retry; promotion hands off on every non-CLI transport by design (D192, R-48) | A failed CLI `--resume` is not reported; the handoff opening is not labelled | **Take** into MOD-37 R-48 (T5) |
| No automatic restart; operator crash-cart | Automatic adoption by sweep | htui is stronger | None |
| Two-orchestrator peer restore | Any worker or TUI adopts by lease | By design | **Reject** (LLM in a bookkeeping path) |
| Process check: argv token plus a lineage fingerprint with start time | MOD-16 R-10: pid plus signal, or a death signal; reused-pid hazard open | Design open | **Take** into MOD-16 R-10, conditional on pid-and-signal (T10) |
| Seeded-fault scenarios for high-risk paths | Conformance suite; MOD-24 kill test unwritten | Kill points unspecified | **Take** into MOD-24 (T11) |
| Least-loaded role resolution | MOD-36 deterministic weights | None | **Reject** (conflicts with ANA-21) |
| Writes to `~/.claude.json`, `.claude/settings.local.json`, `CLAUDE.md`; statusLine tap | `PermissionPolicy`, `ToolExposure`; `DriverEvent::Usage` | None | **Reject** (`R-ID-4`, `R-ID-5`) |
| Compaction enforcer; refocus hook | Single-turn steps never compact | None | **Reject**: no long sessions |
| Typing guard | Promotion hands the session to the human | Covered | None |
| Chatroom, stream, broadcast | None; ANA-13 §3.3 | By design | **Reject** (`R-HIS-2`) |
| Work-tree SPEC/PROGRESS/PROOF files | Documents as rows | By design | **Reject** (`R-ID-4`) |
| Human gateway, Slack | None; MOD-47 is trigger-gated | Out of scope | **Reject** for now |
| TUI explorer, topology graph | Runs pane; MOD-28 rataflow | Planned | None; MOD-28 unchanged |

## 5. Verdict

### 5.1 Take

Each idea below names its target. Ten of them amend existing open items. One, T8, is a new item.
The amendment text is written to be appended to the HANDOFF entry as it stands, in the
"ANA-*n* note" form other analyses use.

**T1. The row is the obligation; a notification is only its id.** OpenRig's queue states it
outright (`queue-wake-ladder.ts:10-12`): the row carries the work exactly once, and the wake is
at least once and carries no content.

- **Fit.** ANA-16 §5.5 already plans durable rows with `NOTIFY` hints and a poll backstop. This
  makes that plan a rule MOD-42's PRD must honour, including the commit-then-crash window, where
  OpenRig's keepalive re-issues lost post-commit nudges
  (`or:docs/as-built/architecture/workflow-runtime.md`).
- **Target:** MOD-42, point (1) of the amendment under T2.

**T2. No agent answers a prompt, and an answer names its request.** OpenRig refuses to type
into a pane showing a permission or selection prompt unless an audited override is given. After a
period of letting the orchestrator "click accept", it reversed itself and now refuses by default
(https://openrig.dev/blog/orchestrator). It also counts open prompts as `{count, reason}` rather
than as a state.

- **Fit.** htui answers through rows, not keystrokes, so the parts that carry over are about which
  answer may be applied. They become sharper once MOD-27 lets one session spawn another.
- **Conflict check.** `PermissionPolicy` rules and remembered answers are configured automation,
  not an agent, so they stay. So does the code-issued cancellation: `AgentSession::cancel`
  answers every outstanding request `cancelled` (`driver.rs:432-433`), and MOD-37 R-38 needs that
  path. All of these are already recorded in `permission_answer.by` (`record.rs:207-220`), though
  a cancellation is filed under `Policy` today (`record.rs:217`).
- **Target:** MOD-42. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T1-T2):** OpenRig's queue and prompt rules
  > settle three points for the PRD. (1) A command or answer row is the obligation, and a
  > `NOTIFY` carries only its id. The worker applies each row at most once, keyed by id and
  > stamped when applied. The poll backstop picks up any row whose notification was lost,
  > including one committed just before a crash. (2) A `permission_answer` names the
  > `permission_request` it answers. An answer to a request that is already answered, or whose
  > session is gone, is refused and recorded. It is never applied to whatever request is open
  > now. (3) A request is answered only by a person (a TUI-written row), by a configured
  > `PermissionPolicy` rule or remembered answer, or by a code-issued cancellation
  > (`AgentSession::cancel` answers every outstanding request `cancelled`, which MOD-37 R-38
  > relies on). The existing `permission_answer.by` column says which (`record.rs:207-220`); a
  > cancellation is recorded as `Policy` today (`record.rs:217`), and the PRD decides whether it
  > gets its own variant. No agent answers another session's request, including a MOD-27 parent
  > answering its child. A timeout never allows an unanswered request. The sources do not say
  > what happens to a request nobody answers; whether it fails the step after a window is the
  > PRD's call. Open requests are counted from the rows (requests without an answer, with the
  > tool as the reason) rather than added as a step status; MOD-43 and MOD-69 read that count.

**T3. Liveness is derived from rows, and "unknown" is a value.** OpenRig's pickup receipts and
PARKED/HELD diagnoses are computed at read time, never self-reported, and its state taxonomy
keeps "unknown" as a value (`queue-pickup.ts:1-60`;
`or:docs/reference/agent-state-taxonomy.md`).

- **Fit.** htui's rows hold the evidence already: lease expiry, heartbeat, `session_event` seq and
  time, and box check-in. What is missing is the vocabulary, and MOD-43 is the item that adds
  liveness to the Runs view.
- **Target:** MOD-43. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T3):** the liveness the Runs view shows is
  > derived from rows at read time, never reported by the agent, and "unknown" is a value. Per
  > running step: *working* (a `session_event` within a window), *quiet* (none within it),
  > *waiting on you* (MOD-42's open permission requests, with count and tool), *overdue* (past
  > `deadline_seconds`; display only until MOD-37's deadline entry lands), and *unknown* (the
  > run's lease holder has not refreshed within the TTL). Per run: *queued, no worker* when the
  > target box's worker has not checked in (`docs/htui-worker.md`, "the pane cannot tell you
  > why"). The windows are settings, and each label shows the age of the evidence behind it.

**T4. A deadline is enforced while the session runs.** OpenRig's tier SLA sets
`closure_required_at` at claim, and the stuck sweep raises every overdue claim
(`hot-potato-enforcer.ts:114-127`; `queue-stuck-sweep.ts:33-60`). htui has `deadline_seconds`,
but it bounds only the verify command and is checked when a step settles; nothing times the agent
session itself (`gate.rs:280-290`; `engine.rs:3530-3547`).

- **Fit.** This is a small defect with a known seam: the cancel seam MOD-37 R-38 already needs.
  Enforcement is code, not an agent.
- **Target:** MOD-37, a new entry:

  > - **Deadline (from ANA-27, `docs/ANA-27.md` §5.1 T4)**: `deadline_seconds` limits the verify
  >   command (which gets what is left of the step deadline) and is checked when a step settles
  >   (`crates/htui-orch/src/gate.rs` `deadline_elapsed`), but nothing times the agent session:
  >   `drive_once` starts no timer, so a hung session holds its slot until the per-run cap or a
  >   human cancel. The fix is a timer in the walk that cancels through R-38's seam and settles
  >   the step as `DeadlineElapsed`.

**T5. A requested resume never silently becomes a fresh start.** OpenRig rolls back and asks
rather than relaunching fresh "with amnesia" (`restore-orchestrator.ts:1196-1249`).

- **Fit.** htui already refuses the ACP resume: a promoted ACP step opens with the handoff
  prompt by design (`promote::opening_kind`, `crates/htui-orch/src/promote.rs:57-72`, D192,
  pinned by the test at `:312-320`). Two gaps remain. A CLI `--resume` that fails is not
  reported, and the handoff opening is not labelled in the Runs pane or the Chat tab, so a person
  cannot tell a fresh context from a resumed one. The note must key on the opening that
  `opening_kind` chose, not on `DriverCaps.resume`, which defaults to `true` for ACP rows
  (`crates/htui-agent/src/registry.rs:165`).
- **Target:** MOD-37 R-48, appended text:

  > ANA-27 (`docs/ANA-27.md` §5.1 T5): until ACP `session/load` lands, a promotion that was not
  > resumed says so. When `promote::opening_kind` chooses `Handoff`, or when the CLI driver's
  > `--resume` fails, the step gets a note ("context not carried; handoff prompt only") that the
  > Runs pane and the Chat tab show, so a fresh context is never mistaken for a resumed one. A
  > failed CLI `--resume` is reported, never silently replaced by a fresh session. Do not key the
  > note on `DriverCaps.resume`: for ACP rows it comes from `settings.acp.session.resume`, which
  > defaults to `true` (`crates/htui-agent/src/registry.rs:165`).

**T6. A swarm step yields through a typed exit, and the code creates the successor.** OpenRig's
"hot-potato" rule requires a typed closure reason, and its handoff closes one row and opens the
next in one transaction (`hot-potato-enforcer.ts:1-101`). Its workflow steps declare
`allowed_exits`, branch only on the exit, and stop cycles with `max_hops`.

- **Fit.** MOD-27's swarm has agents "pass the baton back" (`docs/ANA-13.md:59`) with no
  semantics given. A typed exit keeps the agent proposing and the code deciding and recording
  (`R-ID-6`, ANA-2 invariant 3).
- **Conflict.** ANA-13's adopted swarm design also says "the orchestrator maintains a single
  shared context window" (`docs/ANA-13.md:59`). That is the context-accumulating session this
  analysis rejects elsewhere (`R-PRM-1`, the durable-specialist row in §5.2). A step per yield,
  each with its own fresh session fed by documents, overrides it, and the MOD-27 PRD must
  resolve the conflict explicitly rather than inherit both.
- **Target:** MOD-27, point (1) of the amendment under T7.

**T7. A consult is a recorded, read-only child step, not a message.** `rig send` exists because a
question should not create an obligation (https://openrig.dev/docs/messaging). htui cannot use a
bus (ANA-13 §3.3, `R-HIS-2`), but MOD-27's `spawn_subagent` already creates a recorded child
step.

- **Fit.** A consult mode gives the same distinction between asking and assigning, with nothing
  outside the step history.
- **Target:** MOD-27. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T6-T7):** settle in the PRD, from OpenRig's
  > queue and workflow runtime: (1) **The swarm baton is a typed exit.** An agent yields with
  > one exit from a closed set the step declares (`handoff` to a named persona, `done`,
  > `blocked` on an item, `failed`), validated by code. The orchestrator closes the step and
  > creates the next step row in one transaction, and a per-run hop cap stops cycles. The agent
  > proposes, the code records (`R-ID-6`, ANA-2 invariant 3). Each yield starts a new step with
  > its own fresh session fed by documents (`R-PRM-1`). This overrides ANA-13 §3.3's "the
  > orchestrator maintains a single shared context window" (`docs/ANA-13.md:59`), and the PRD
  > must settle that conflict explicitly. (2) **`spawn_subagent` has a consult mode.** A
  > read-only child, with no tree and no output document, returns its answer as the tool result.
  > It is still its own `run_step` with `parent_step_id`, recorded and replayable (`R-HIS-1`,
  > `R-HIS-2`), and draws on its own agent's quota (`R-AGT-7`). This is htui's form of OpenRig's
  > `rig send`, without a bus (ANA-13 §3.3). (3) Open for the PRD: depth and breadth limits
  > against `max_agents_per_run`; how a child fits `UNIQUE (run_id, position, attempt,
  > fanout_index)`; crash recovery of a child under ANA-2 §4.9; and whether graph steps deny the
  > harness's own subagent tool, whose sessions htui never sees. Cites `R-MCP-2`, which already
  > lists the tool.

**T8. One list of everything waiting on a person, across items.** OpenRig's TUI has an attention
view that "shows human requests separately from outcome and health updates"
(https://openrig.dev/docs/tui).

- **Fit.** In htui, a parked gate, a fan-out awaiting `s`, a judge park or a blocked run shows
  only in the Runs pane of its own item. With `htui worker` running several items, a person finds
  them by visiting items one at a time. No open item owns a list across items: MOD-12's planned
  escalations (not built yet) cover auto mode only.
- **Target:** new item, **MOD-69**:

  > - [ ] **MOD-69 - Waiting-on-you list across items** (from ANA-27, `docs/ANA-27.md` §5.1
  >   T8). `R-TUI-1`, `R-TUI-4`, `R-ORCH-2`, `R-ORCH-4`, `R-NF-3`. The Runs pane shows what waits
  >   on a person only for the selected item, and the top bar counts active runs, so with several
  >   items running a parked gate, a fan-out awaiting selection, a judge park or a blocked run is
  >   found by visiting items one by one. Add an overlay, opened from every screen, that lists
  >   every run in the active workspace waiting on a person, one row per reason (gate, selection,
  >   judge failure, unblock, and, once MOD-42 lands, each open permission request with its tool),
  >   read by one store query. `Enter` opens the item's Runs pane on that step, and the top bar
  >   gains the count. The list is derived at read time and keeps no state of its own; MOD-12's
  >   planned escalation list can reuse the query, and the opening key is a MOD-67 action. Needs
  >   `R-TUI-11` and the matching `R-TUI-1` top-bar line (ANA-27 §7). Not blocked; the permission
  >   rows are added after MOD-42.

**T9. A persona only narrows its base.** "Profiles filter. They never inject resources not
declared in the base spec." (https://openrig.dev/specs/taxonomy) OpenRig also rejects unknown keys
at every level (`rigspec-schema.ts:78`) and fixes its layering order. OpenRig's own rule is
looser than narrow-only: a profile selects from what the base declares, and "may override
startup and lifecycle defaults" (same page).

- **Fit.** MOD-26 names two keys (model, tools) but has no precedence or narrowing rule, and it
  carries ANA-13's `DriverCaps` wording. Narrow-only is htui's choice, inspired by "profiles
  filter, never inject" and stricter than OpenRig: a narrow-only persona cannot widen tool
  exposure or permissions behind the phase's back. The body stays a block htui inlines
  (`R-ID-5`).
- **Target:** MOD-26. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T9):** taking its cue from OpenRig's
  > AgentSpec rule that profiles "filter. They never inject", htui goes further and makes a
  > persona narrow-only: its tool allow-list is a subset of the exposure the step would get, and
  > its permission rules are no looser than the default. Unknown frontmatter keys are refused at
  > save. Precedence is fixed and written down: agent row, then persona, then the phase
  > candidate's `model`. The body is a block htui inlines into the step prompt (`R-ID-5`,
  > `R-PRM-1`), not a file handed to the agent. ANA-13 §3.1's "`DriverCaps` overrides" reads as
  > `SessionSpec` overrides, since `DriverCaps` are transport facts. Cites `R-AGT-4`, `R-ID-3`,
  > `R-ID-5`, `R-PRM-1`.

**T10. A recorded process is identified by its pid and start time.** Before trusting a pane's
process, OpenRig matches the seat's session or resume token in its argv, then compares a lineage
fingerprint (pid, ppid, `lstart` start time, pgid, tpgid, command) across two back-to-back `ps`
observations (`native-process-lineage.ts:124-201`). It never records a pid to check later, but
it treats pid plus start time as the identity of a process.

- **Fit.** MOD-16 R-10 leaves the design open between a pid in `session_started` plus a signal
  on adoption, and a death signal (`HANDOFF.md:421-425`). If R-10 picks the pid-and-signal
  design, matching the start time as well closes the reused-pid hazard; on Unix the start time
  comes from `/proc` or `ps`. Windows has its own process creation time, and confirming that is
  MOD-16's job. Signalling still needs `unsafe` or a dependency, which this does not change, and
  the death-signal alternative avoids pid reuse entirely.
- **Target:** MOD-16 R-10. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T10):** if R-10 picks the pid-and-signal
  > design, record the agent child's pid **and its start time** in `session_started`, and signal
  > on adoption only when both still match. A reused pid then fails the match instead of
  > receiving the signal. This does not remove the need for `unsafe` or a dependency to send the
  > signal, and the death-signal alternative avoids the reused-pid hazard entirely. OpenRig
  > treats pid plus start time as a process's identity in the same way (an argv token match,
  > then a lineage fingerprint including `lstart` compared across two `ps` observations).

**T11. The kill test kills at seeded points.** OpenRig pins its riskiest paths with seeded-fault
scenarios ("queue baton survives daemon restart", `or-main:docs/as-built/arteries.md:71-75`). It
also records that restore, delivery and rig identity have no scenario coverage yet
(`or-main:docs/as-built/arteries.md:76-82`).

- **Fit.** MOD-24's remaining scope is exactly such a test. Naming the kill points keeps the test
  deterministic, with no sleeps.
- **Target:** MOD-24. Amendment:

  > **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T11):** run (2) at fixed kill points
  > reached through a `testkit` hook, not after a sleep: after the session starts and before its
  > first flush; after a flush; after capture and before the output document; and after the
  > document. Once MOD-42 lands, add a kill between a command row's commit and its notification,
  > which the poll backstop must still apply.

### 5.2 Reject

| Idea | Reason |
|---|---|
| An LLM lead that assigns, closes or hands off work | `R-ID-6`, ANA-2 invariant 3 and `CONCEPTS.md:36-37` keep every status move and assignment in code. The judge is the only LLM decision, and it is bounded to one `fanout_index` |
| Durable specialist sessions that accumulate context | `R-PRM-1` and bounded context want one self-contained prompt per step. MOD-24 dropped context re-hydration. OpenRig itself calls compaction lossy ("what comes back believes it knows everything and does not") |
| tmux panes as transport; pane scraping | `R-ID-2` (not a terminal multiplexer). `R-NF-1`: tmux has no native Windows port. htui already has structured ACP events. Pasting into a harness's terminal couples delivery to its UI: OpenRig depends on Claude's `[Pasted text #N +X lines]` placeholder, the Codex footer regexes and a 200 ms delay before Enter |
| A team file in the repo (RigSpec, CULTURE.md, managed `CLAUDE.md` blocks) | `R-ID-3`: definitions are rows. `R-ID-4`: htui writes nothing into managed repos. ANA-16 recommends personas as registry rows (§6.2, §8) |
| An agent message bus (chatroom, broadcast, stream) | Rejected by ANA-13 §3.3 because it breaks `R-HIS-2` replay. Consulting is covered by T7 |
| Harness-native subagents left untracked | ANA-13 §3.2: they break the single-driver invariant and hide usage from quota (`R-AGT-7`). ANA-27 adds that their events never reach history (`R-HIS-1`). OpenRig leaves them untracked by design ("The helpers have no OpenRig addresses or queue ownership", https://openrig.dev/), so it offers no counter-evidence either way |
| Least-loaded agent selection | It reads live backlog at selection time. ANA-21's apportionment is reproducible from the snapshot, with no clock or live state (`R-AGT-8`) |
| Agent-driven restart and two-orchestrator peer restore | Recovery is bookkeeping (`R-ID-6`). htui already adopts automatically by lease and sweep, which OpenRig does not |
| Writing trust, hooks or statusLine into `~/.claude.json` or `.claude/settings.local.json` | `R-ID-4` for repo files. No requirement forbids writing an agent's home-directory config; the rejection rests on `R-ID-5`'s intent (htui owns prompts and skills, not the harness's own settings) and on `PermissionPolicy`/`ToolExposure` already covering the need. Usage already arrives as `DriverEvent::Usage` |
| Timed wakes typed into idle sessions (watchdogs, parked-owner wakes) | Single-turn steps leave no idle session to wake. The worker sweep is code |
| Compaction enforcer and refocus hook | No long-lived session ever compacts in htui |
| Work-tree SPEC/PROGRESS/PROOF markdown files | `R-ID-4`. Documents are rows (`R-ID-3`) |
| Human gateway and Slack routing | No requirement asks for it. Reaching people outside the TUI is MOD-47 territory, and MOD-47 is trigger-gated |
| Pods as a unit | Box, project and run already partition work, so a pod would add no boundary htui enforces |

## 6. Phasing

None of this needs code before its target item starts. The amendments are text, applied to
`HANDOFF.md` when ANA-27 closes.

| Change | Lands with | Order and dependency |
|---|---|---|
| MOD-42 note (T1, T2) | MOD-42's PRD | First: MOD-42 is unblocked, and MOD-43 and MOD-47 wait on it |
| MOD-43 note (T3) | MOD-43 | After MOD-42; the *overdue* label waits on the MOD-37 deadline entry |
| MOD-37 deadline entry (T4) | Any MOD-37 batch, alongside R-38 | Shares R-38's cancel seam |
| MOD-37 R-48 text (T5) | Any MOD-37 batch | Independent; the note needs no ACP work |
| MOD-27 note (T6, T7) | MOD-27's PRD | After MOD-11 for the MCP half; T2 point (3) must be in force first; the PRD resolves the conflict with ANA-13 §3.3's shared context window |
| MOD-26 note (T9) | MOD-26's PRD | Independent; before MOD-27, since swarm personas use it |
| MOD-16 R-10 note (T10) | MOD-16 | Independent |
| MOD-24 note (T11) | MOD-24 | Now for the four step kill points; the notification kill point after MOD-42 |
| MOD-69 (T8) | New item | Gate, selection, judge and unblock rows now; permission rows after MOD-42. `R-TUI-11` approved first |

## 7. Requirement changes

Proposed here for maintainer approval. Not applied by ANA-27.

**R-TUI-11, new** (for MOD-69):

> **R-TUI-11 (must).** One overlay, opened from every screen, lists every run in the active
> workspace that waits on a person (a gate, a fan-out selection, a judge failure, a blocked run,
> an open permission request), with the reason and the step, and opens the item's Runs tab on
> that step. The top bar shows the count. The list is computed from the store at read time.

**R-TUI-1, consequential change** (for MOD-69): the top-bar list, which ends with "active run
count", gains "waiting-on-you count (`R-TUI-11`)".

No other requirement changes. T2's "only a person, a configured rule or a code-issued
cancellation answers a permission request" is a design rule for MOD-42 inside `R-TUI-6` and
`R-HIS-1` as written. T6 and T7 fit `R-MCP-2` and `R-ID-6` unchanged. If the maintainer wants
T2 pinned as a requirement, the natural place is one sentence in `R-TUI-6`.

## 8. Risks and open questions

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| OpenRig moves daily, so cited behaviour drifts | High | Low | Nothing here depends on OpenRig as a library. Every take is restated in htui's terms |
| A consult mode becomes a back door to unbounded spawning | Medium | Medium | Child steps count against `max_agents_per_run` and the agent's quota. The MOD-27 PRD sets depth and breadth limits |
| Typed exits push swarm agents into rigid outcomes | Medium | Low | The closed set is declared per step, and `failed` with a note is always available |
| Derived liveness windows give false *quiet* or *unknown* labels on slow models | Medium | Low | The windows are settings, and each label shows its evidence age (T3) |
| A narrow-only persona blocks a legitimate wider use | Low | Low | Widening happens on the agent row or the phase, where it is visible. Narrow-only is stricter than OpenRig, whose profiles may override startup and lifecycle defaults |
| T5's note mislabels a promotion | Low | Low | The note keys on the opening `promote::opening_kind` chose, not on `DriverCaps.resume`, which defaults to `true` for ACP rows. Only the CLI driver resumes, so detecting a failed `--resume` is a CLI-only MOD-37 plan detail |

**Open for the maintainer or the target items:**

- **Native subagent tools on graph steps.** Claude Code's `Agent`/Task tool and Codex's
  `spawn_agent` run harness-native subagents that htui never sees as steps. Whether their usage
  reaches htui inside the parent's `Usage` events, and whether their transcripts reach
  `session_event`, is **UNCONFIRMED**. MOD-27 decides whether graph steps deny those tools through
  `ToolExposure` until `spawn_subagent` exists (T7 point 3).
- **Unanswered permission requests.** T2 rules out an automatic allow. Whether a request fails
  the step after a window, or holds its slot indefinitely, is MOD-42's call. A parked request
  holds a live session and a compute slot, unlike a gate (ANA-2 invariant 6).
- **R-TUI-11 scope.** Whether the list spans the active workspace only or every project the
  user can see is a maintainer choice. §7 proposes the workspace.
- **Sources not examined.** OpenRig's workflow runtime beyond the role resolver, the Slack
  gateway, cross-host routing, and whether loopback write routes need the bearer token were not
  read. None of them bears on the verdict.
