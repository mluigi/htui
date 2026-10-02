# MOD-26 — Declarative agent personas

> Routed as **PRD** by `/handoff-run MOD-26` (criteria C2, C3 and C4 fired; accepted by the
> maintainer 2026-10-02, sandbox run `hr/MOD-26`). Ultracode accepted for the implement phase.
> Origin: `docs/ANA-13.md` §3.1, §4 item 1. Amended by `docs/ANA-16.md` §6.2, §8 (registry rows,
> not a per-box directory) and `docs/ANA-27.md` §5.1 T9 (narrow-only, fixed precedence, body
> inlined). Requirements: `R-AGT-4`, `R-AGT-8`, `R-ID-3`, `R-ID-5`, `R-PRM-1`, `R-PRM-3`, `R-MCP-3`,
> `R-HIS-1`. Consumer: MOD-27 (swarm `handoff` to a named persona).

## Problem

htui has nothing between "agent" and "phase". A step's tool exposure and permission policy come
from the agent row, and its instructions come from the phase template. A specialist posture, such
as a read-only reviewer or an architect that designs but does not edit, means editing the agent
row, which changes every phase that agent serves, or forking a phase template per posture. Neither
is reusable or named, and MOD-27's swarm `handoff` to a *named persona* has nothing to name.
Separately, the tool-narrowing half of the posture would be inert today even if it could be
expressed, because no transport honours `SessionSpec.tools`.

## Evidence

Read at `04eeb7c9` (branch `hr/MOD-26`). Paths relative to the repo root.

- **The maintainer's own workflow runs on personas of exactly this shape.** `.claude/agents/
  code-architect.md` and `.claude/agents/rust-reviewer.md` are frontmatter (`name`, `description`,
  `tools`) plus a prompt body, and `/handoff-run` depends on both. The maintainer asked for "something
  like reviewers or code architect" inside htui.
- **A pinned model on a persona defeats quota fallback.** Both agents above carried `model: opus`
  and failed outright on an empty OpenRouter account (auto-memory `htui-opus-402-blocks-pinned-agents`).
  htui's candidate walk (`R-AGT-8`) exists to skip an exhausted agent/model; a persona-imposed
  model would bypass it. Decision: personas carry **no model key** (Open Questions, settled).
- **Tool exposure is not enforced anywhere.** The engine builds every step with
  `tools: ToolExposure::default()` (`crates/htui-orch/src/engine.rs:5797`), and no code in
  `crates/htui-agent/src` reads `tools.allow`, `tools.deny` or `tools.command_run`; the field only
  appears in `SessionSpec`'s `Debug` (`crates/htui-agent/src/driver.rs:290`). The permission half
  *is* live: the engine passes the agent's `PermissionPolicy` and relays requests through it
  (MOD-42, `engine.rs:5782`).
- **ANA-27's precedence wording is unimplementable as written.** "Agent row, then persona, then
  the phase candidate's `model`" puts the candidate last, but `phase_agent.model` is `NOT NULL`
  (`crates/htui-store/migrations/0001_init.sql:258`) and `SnapshotCandidate.model` is a `String`
  (`crates/htui-orch/src/graph.rs:732-785`), so a persona `model` would always be shadowed. Moot
  once personas carry no model; the PRD restates the precedence for the keys personas do carry.
- **A per-box directory violates `R-ID-3`** ("Postgres is the single source of truth"); ANA-16
  §6.2 lists `~/.config/htui/agents.d/` as a drift surface. Decision: registry rows.

## Users

- **Primary**: the htui maintainer/operator who authors step graphs and wants specialist steps
  (review, architecture, scouting) with a narrower tool and permission posture and a role-specific
  instruction block, without touching agent rows or duplicating phase templates.
- **Secondary**: MOD-27, whose swarm baton hands off to a persona by name.
- **Not for**: agents' own native subagent/profile systems (`R-ID-5` keeps those disabled); end
  users wanting to *widen* what a step may do (personas only narrow).

## Hypothesis

We believe **named personas stored as registry rows and bound to a step-graph phase**
will **let specialist steps run with their own instructions and a narrower, enforced tool and
permission posture** for **the operator authoring step graphs**.
We'll know we're right when **a run's review phase executes under the seeded `reviewer` persona,
the recorded step shows the persona's block in its prompt and its narrowed exposure in effect
(an out-of-list tool call is refused and recorded), and the agent row is unchanged.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Persona applied end to end | 1 real run with a persona-bound phase, prompt block + narrowed exposure on the step | Runs pane / step prompt record on a live run |
| Narrow-only holds | 0 ways to save a persona that widens tools, `command_run` or permissions | Conformance cases over every widening shape (save refused) |
| Out-of-list tool refused | 100% of out-of-list calls refused on the relay path; flag path covered for `claude` CLI | Driver / relay conformance cases |
| Agent row untouched | 0 writes to `agent` when binding or applying a persona | Store conformance |
| Replay stable | Editing a persona after a run starts does not change that run's steps | Snapshot conformance case |

## Scope

**MVP**

1. **Persona registry row** in Postgres: name (unique), description, prompt body, tool allow-list,
   tool deny-list, `command_run` exposure, permission rules. No model key. Unknown keys refused
   at save.
2. **Narrow-only validation at save and at apply.** Tool allow-list is a subset of the base
   exposure (an empty base allow-list means "everything", so any list narrows); deny can only add;
   `command_run` can only be turned off; permission rules are no looser than the agent's policy
   (a persona can add `deny`/`ask` rules, never an `allow` the base would not grant, never a
   looser default).
3. **Binding on the step-graph phase**: a `step_graph_phase` may name one persona, applied to
   whichever candidate wins the phase (rungs 1-3 alike). Per-candidate binding is deferred (Out of
   scope).
4. **Fixed, written-down precedence** for the keys a persona carries: agent row (base exposure and
   policy) → persona (narrows) → result on the step. Model stays the phase candidate's (`R-AGT-8`).
5. **Body inlined into the step prompt** as an htui-owned block (`R-ID-5`, `R-PRM-1`), never a file
   handed to the agent, with a defined place in `R-PRM-3`'s trim order.
6. **Enforcement, both layers**: pass the narrowed lists as transport flags where the agent
   supports them (`claude` CLI), and deny out-of-list tool calls in the permission relay for every
   transport. Where neither layer can see a call (tools an ACP agent runs without asking), the gap
   is documented, not hidden.
7. **Snapshot and record**: the persona as applied is frozen with the run's graph snapshot, and
   the step records which persona it ran under, so later edits never change a started run
   (`R-HIS-1`).
8. **Authoring**: a Settings > Personas tab (form fields + body editor), plus a one-shot import of
   a frontmatter Markdown file (the `.claude/agents/*.md` shape) into a row. The row is always the
   truth; the file is never re-read.
9. **Seeds**: `reviewer` and `architect` personas mirroring `.claude/agents`, with read-only
   narrowed tools.

**Out of scope**

- `~/.config/htui/agents.d/` discovery or any file-backed source of truth — violates `R-ID-3`.
- A persona `model` key, model filters, or model overrides — conflicts with `R-AGT-8` quota
  fallback (see Evidence).
- Swarm `handoff`, `spawn_subagent`, typed exits — MOD-27.
- Output-schema enforcement (ANA-13 §4 item 1 lists "schema requirements") — belongs with MOD-27's
  typed exits.
- Change push of persona rows to other boxes — MOD-48; rows are read from Postgres like every
  other registry row.
- Per-candidate binding (`phase_agent.persona_id`) — `phase_agent` has no writer (HANDOFF R-6) and
  most runs resolve their candidate on rung 2 or 3 without a `phase_agent` row; lands later as one
  more precedence rung (candidate over phase) once R-6's writer exists.
- Export of a persona back to a Markdown file.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Personas in the registry, applied by the engine | A persona row (seeded `reviewer`/`architect`) bound to a phase candidate changes that step's prompt block, exposure and policy; narrow-only refused at save; snapshot-stable; out-of-list calls refused by the relay and by `claude` CLI flags | complete | `.claude/plans/mod-26-agent-personas.plan.md` (T0-T5) |
| 2 | Authoring in the TUI | Settings > Personas lists, edits, validates and saves personas; frontmatter `.md` import; a phase's persona is picked in the step-graph editor | pending | — |

Milestone 1 shipped 2026-10-02 (T0-T5; the binding landed on the step-graph phase, per Q-binding's
revision). Moved counts: store conformance `CASES` 119 → 124 (`READ_CASES` 14), `htui-orch`
`CASES` 86 → 91, migrations 11 → 12 (`0012_persona.sql`; next `0013`), Postgres tables 41 → 42,
column comments 35 → 44, `.sqlx` files 307 → 315. Operator guide: `docs/personas.md`.

Milestone 2 also carries two items the milestone 1 review deferred (`rust-reviewer`, approve with
fixes, 2026-10-02; in R1 the maintainer fixed L1, L3, N1-N3 and N6-N8, documented L2 and L4 in
`docs/personas.md`, and deferred these):

- [ ] **N4** — a typed `PersonaNotInSnapshot` error for `GraphSnapshot::persona_for` in place of
  its `String` (`persona_not_in_snapshot`'s sentence), so the engine's I-4 arms match a type
  rather than carry a sentence.
- [ ] **N5** — an index on `step_graph_phase.persona_id`, shipped with milestone 2's persona
  delete: the `ON DELETE RESTRICT` check of `fk_step_graph_phase_persona` scans the phases
  without one.

## Open Questions

- [x] Where do personas live? — **Postgres registry rows** (maintainer, 2026-10-02; `R-ID-3`).
- [x] Can a persona set the model? — **No** (maintainer, 2026-10-02). ANA-27's precedence sentence
  is restated in this PRD without the model rung; DECISIONS/ANA-27 note to be corrected at close-out.
- [x] Where is a persona bound? — **On the step-graph phase** (`step_graph_phase`), nullable
  (maintainer, 2026-10-02, revised at `/plan`: the first answer, the phase candidate, was taken
  before it was known that `phase_agent` has no writer, HANDOFF R-6, and that rung 2/3 runs have
  no `phase_agent` row; per-candidate binding is deferred).
- [x] Authoring surface? — **Settings tab + `.md` import** (maintainer, 2026-10-02).
- [x] Seeds? — **`reviewer` + `architect`** (maintainer, 2026-10-02).
- [x] Tool enforcement? — **Both layers**: transport flags where supported, relay denial for all
  (maintainer, 2026-10-02).
- [x] Tool vocabulary / transport syntax — settled at `/plan` (fact-checked): `deny_kinds`
  (agent-neutral, every transport) plus agent-native names enforced on `claude-cli` only, via
  `--tools=` / `--disallowedTools=` (never `--allowedTools`, which auto-approves); the step's
  "layers in force" is documented as a per-transport matrix instead of recorded per step. See the
  plan's OQ-1, OQ-6 and D11.
- [ ] ~~Which transports accept an allow/deny list, and in what syntax?~~ (settled above) `claude` CLI is assumed
  (`--allowedTools`/`--disallowedTools`); `agy` over ACP is TBD — needs validation via a probe of
  the installed binaries at plan fact-check.
- [ ] Does every tool call surface to the relay? ACP agents may run some tools without a
  permission request, which the relay cannot deny — TBD, needs validation via the ACP fake and one
  live `agy` session; the residual gap is documented either way.
- [ ] Tool-name vocabulary: allow-lists name agent-native tools (`Read`, `Bash`, `mcp__…`), which
  differ per agent. Is a persona agent-neutral (names validated per bound agent at apply) or
  agent-scoped? TBD — decide in `/plan`; affects the narrow-only check at save versus at bind.
- [ ] Where does the persona block sit in `R-PRM-3`'s trim order? Proposed: never trimmed, like the
  phase template — confirm in `/plan`.
- [ ] Interactive chat: does a persona apply to the chat path, or engine steps only? MVP assumes
  engine steps only — confirm in `/plan`.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Narrowing is cosmetic on an agent that runs tools without asking and has no allow-list flag | Medium | High — a "read-only" reviewer edits the tree | Both enforcement layers; the step records which layers were in force; the gap is shown, not hidden |
| Tool names differ per agent, so one persona narrows differently per agent | High | Medium | Settle neutral vs agent-scoped in `/plan`; validate names against the bound agent at bind time |
| Persona edits change a running run | Medium | Medium — breaks replay (`R-HIS-1`) | Freeze the applied persona in the graph snapshot |
| Permission "no looser" is hard to define over rule lists | Medium | Medium | Define looseness rule-by-rule in `/plan`; conformance cases enumerate widening shapes |
| Migration, `.sqlx`, seeds and snapshots couple "independent" implementer tasks | High | Medium | File-set intersection at plan fact-check decides parallelism (auto-memory `parallel-fanout-hidden-file-coupling`) |

---
*Status: IN PROGRESS — milestone 1 complete (2026-10-02); milestone 2 (authoring in the TUI) pending.*
