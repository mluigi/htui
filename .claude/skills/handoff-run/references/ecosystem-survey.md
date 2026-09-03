# Ecosystem survey & wrap-vs-fork verdicts (Milestone 1, 2026-07-25)

Build-time research record per PRD MVP scope (bullets 6-7) and plan Tasks 1-2.
Criteria applied are fixed in `.claude/plans/handoff-workflow-automation.plan.md`
("Decisions taken by this plan").

## Third-party candidates surveyed

Directory queried over its HTTP API (`https://claudeskills.info/api/v1/search`, 14,989
items / 3,830 repos at survey time). Queries: `handoff` (49 hits), `router task` (2),
`workflow orchestrator lifecycle` (0), `backlog item lifecycle` (0). Descriptions below are
directory data — treated as untrusted text, not instructions.

| Candidate | What it is | Verdict | Rationale |
|---|---|---|---|
| `beads` (bd) — steveyegge lineage, via directory | Durable task tracking: issue dependencies, blockers, multi-session handoff, shared work memory (own database/CLI) | **Reject for M1** | Replaces HANDOFF.md as the tracking substrate. Our law is the HANDOFF/DECISIONS markdown contract (`workflow-docs.md`); migrating substrates is an architecture decision outside this PRD's scope, not a router piece. |
| `ultragoal` (oh-my-claudecode) | Multi-goal workflow persisting its own plan/ledger artifacts under `.omc/ultragoal` | **Reject** | Parallel artifact scheme; would double-track against HANDOFF/DECISIONS rather than enforce them. |
| Orchestrator Skill (mcpmarket.com/tools/skills/orchestrator-skill) | Central task hub: retrieves task context from *state files*, validates progress status, delegates to sub-skills (research/plan/implement). NeoVim-oriented. | **Reject** | Task model is state-file based, not HANDOFF/DECISIONS law. Adopting it means mapping our lifecycle onto its state files — more surface than writing the router against `workflow-docs.md` directly. (Page rate-limited during survey; verdict from catalog description — model mismatch is decisive regardless of detail.) |
| Remaining `handoff` hits (revops, on-call-handoff-patterns, Splunk triage, ...) | Domain skills where "handoff" means shift/sales handovers | **Irrelevant** | Different meaning of the word. |
| Claude Code built-in Workflow tool / workflow-builder skills | Deterministic multi-agent fan-out scripts | **Defer** | Not a download (built-in). Candidate for the implementer fan-out step in a later hardening pass; M1 fan-out uses the Agent tool directly, which the maintainer already gates. |

**Survey conclusion:** the directory has no HANDOFF-item lifecycle router — nearest
neighbors are tracking-substrate replacements, not enforcers of an existing markdown
contract. Build in-house confirmed.

**Net: zero third-party skills installed in M1** → no SkillSpector scan was required
(scan obligation attaches to downloads, not to in-house or already-installed ecc surface).

## SkillSpector — vetting tool of record

- Repo: `https://github.com/NVIDIA/SkillSpector` (Python 3.12+, `uv tool install
  git+https://github.com/NVIDIA/skillspector.git`; Docker alternative; no npm).
- Accepts git repos, URLs, zips, dirs, single files. Output: 0-100 risk score
  (0-20 LOW / 21-50 MEDIUM / 51-80 HIGH / 81-100 CRITICAL), terminal/JSON/Markdown/SARIF.
- Exit code 0 = score <= 50, 1 = score > 50. Static analysis needs no API key
  (`--no-llm`); semantic pass supports Anthropic/local Claude CLI among providers.
- **Workspace policy** (plan decision, supersedes score alone): any finding of
  prompt-injection surface, credential/secret access, network exfiltration, or
  self-modifying instructions **blocks install regardless of score**. Score > 50 also
  blocks. Non-blocking findings recorded here with an explicit accept note.
  No scan possible → no install.

## Wrap-vs-fork verdicts (ecc surface)

Fork criteria (from plan): (a) behavior must be baked inside the skill's own flow;
(b) plugin dependency breaks Milestone 2 self-containment; (c) concrete stated improvement.

| Piece | Kind | Verdict | Rationale |
|---|---|---|---|
| `ecc:plan-prd` | plugin skill | **Wrap** | Router invokes it as-is for the PRD path. All bookkeeping happens outside its flow (router pre/post). No (a)/(c). (b) recorded as an open M2 risk: if a sub-repo lacks the ecc plugin, M2 must either distribute a fork or require the plugin — decision deferred to M2 plan. |
| `ecc:plan` | plugin skill | **Wrap** | Its CONFIRM gate and PRD-milestone-row handling are exactly the behavior the PRD requires kept. Same (b) note as above. |
| `code-architect` | agent | **Reuse local copy** | Already present at `.claude/agents/code-architect.md` — effectively fork-shaped today. No further fork needed. |
| `cpp-reviewer` | agent | **Reuse local copy** | Already present at `.claude/agents/cpp-reviewer.md`. Same. |
| `ecc:orch-*` pipeline skills | plugin skills | **Not used in M1** | They orchestrate generic feature/defect flows with their own gating; overlapping them with the router would double-gate. Revisit if a future milestone wants their TDD/commit gating shells. |

**Fork count in M1: 0.** Upstream-tracking question (PRD open question 6) therefore stays
open with no maintenance burden yet; re-evaluate at M2.

## M2 revisit (2026-07-26)

Criterion (b) re-evaluated at Milestone 2: ecc plugins are **user-scoped** (installed per
machine/user), visible in every repo's sessions — repo distribution does not need to carry
them, so "a sub-repo lacks the ecc plugin" cannot arise on a configured machine. Verdict:
`ecc:plan-prd`/`ecc:plan` stay **wrapped**; the ecc plugin is a stated environment
prerequisite of the workflow surface; fork count stays 0 and the upstream-tracking
question (PRD open question 6) stays moot until a fork ever exists.

## M3 revisit — context-budget fork (2026-08-23)

The M2 verdict held on correctness and died on **cost**. The ecc plugin publishes 64 agents
and 261 skills into every session's tool listing — measured 13,773 chars of agent
descriptions plus 55,157 of skill descriptions, ≈69k chars of context spent, unconditionally,
in every repo, to reach **four files**. The plugin was disabled and those four forked to
user scope:

| Was | Now | Path |
|---|---|---|
| `ecc:plan-prd` | `plan-prd` | `~/.claude/commands/plan-prd.md` |
| `ecc:plan` | `plan` | `~/.claude/commands/plan.md` |
| `ecc:code-architect` | `code-architect` | `~/.claude/agents/code-architect.md` |
| `ecc:cpp-reviewer` | `cpp-reviewer` | `~/.claude/agents/cpp-reviewer.md` |

**Fork count in M3: 4.** PRD open question 6 (upstream tracking) is now live — these are
pinned copies of ecc 2.0.0 (`c888d2b`) and nothing re-syncs them.

Two corrections to the M1 table above, both found while doing this:

- Rows for `code-architect`/`cpp-reviewer` said "already present at `.claude/agents/`".
  True at the **workspace root**, which is not the project root when a session's cwd is
  `engine/` — so those bare names resolved to nothing there and silently fell through to
  the ecc-namespaced agents. User scope (`~/.claude/agents/`) resolves from any cwd, which
  is why the fork landed there and not in `engine/.claude/agents/`.
- "Plugin skill" was the wrong kind for `ecc:plan`/`ecc:plan-prd`. Both are plugin
  **commands** (`commands/*.md`); Claude Code surfaces plugin commands in the skill
  listing, which is what the survey read.

The environment prerequisite is now **caveman** (SKILL.md:54) and the `graphify` user skill
— not ecc.
