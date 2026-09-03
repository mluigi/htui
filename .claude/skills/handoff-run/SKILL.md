---
name: handoff-run
description: "Run a HANDOFF.md item through its full lifecycle from one command: select (explicit ID, or `next` to resolve the logically next item and ask the maintainer on a tie), route (PRD/plan/ANA), confirm with maintainer, execute the chain (plan-prd, plan, code-architect, implementer fan-out, configured reviewer), then apply HANDOFF/DECISIONS bookkeeping per workflow-docs.md and validate it. Trigger: /handoff-run <ITEM-ID>|next [--dry-run]."
---

# /handoff-run

One command per HANDOFF item lifecycle. Auto-chain with the maintainer present — every gate below stays interactive.

## Usage

```
/handoff-run MOD-4            # route + confirm + execute full lifecycle
/handoff-run next             # pick the logically next item, then the same lifecycle
/handoff-run ANA-5 --dry-run  # routing verdict only, nothing executes
/handoff-run next --dry-run   # selection block + routing verdict only, nothing executes
```

## References (load per phase, not all up front)

- `references/selection.md` — `next` only: candidate set, eligibility, ranking, ambiguity rule
- `scripts/next-item.sh` / `.ps1` — `next` only: pre-filter computing the machine half of selection (candidate table,
  verified blocked-on links, in-flight/dependent facts, tie flag); judgment calls stay with `selection.md`
- `references/routing.md` — decision table, verdict format, override rules
- `references/lifecycle.md` — bookkeeping procedures (open / phase note / close-out)
- `references/ecosystem-survey.md` — wrap-vs-fork record (build-time, not needed at run time)
- `scripts/next-item-id.sh` / `.ps1` — the owned-ID mint, run whenever a path spawns a new item (lifecycle P0)
- `scripts/validate-workflow-docs.sh` / `.ps1` — structural validator, run at close-out
- Law: `.claude/rules/workflow-docs.md` (auto-loads when workflow docs are touched)

**On Windows every script above is the `.ps1`, never the `.sh` — including the ones git runs for
you.** The two variants are kept behaviourally identical, so this is not a preference: bash
subprocesses stall or die on fork on this box (`0xC0000142`), and a `.sh` reached through a git hook
fails as a **hang** rather than an error, so it reads as a slow commit instead of a broken one.
Measured 2026-08-21: a `docs(...)` commit staging `HANDOFF.md` ran **>2 min and was killed** with the
bash-installed `pre-commit` hook, and **1.3 s** after re-running
`pwsh install-workflow-hooks.ps1` (which writes `pwsh` call sites into `.git/hooks/`). Install the
hooks with the `.ps1`, and when a commit hangs, check `.git/hooks/pre-commit` for a `bash` call
before suspecting anything else.

**Enforced since MOD-43 M3, not merely documented.** All five `.sh` entry points
(`install-workflow-hooks`, `sync-workflow-surface`, `validate-workflow-docs`, `next-item`,
`next-item-id`) detect a Windows shell — `uname -s` reporting `MINGW*`/`MSYS*`/`CYGWIN*` — and
**exit 2 naming the `.ps1` to run instead**. The installer is the one that matters most: the hooks
*it* generates call `bash …/*.sh`, so a single Windows run of the `.sh` variant would replace every
`pwsh` call site with a bash one and make every later commit pay the difference. Override only for a
deliberate twin-parity check: `WORKFLOW_ALLOW_SH_ON_WINDOWS=1`. The twins stay behaviourally
identical — the refusal is about which one this platform pays for, not which one is correct.

## Flow

### 0. Response style

Invoke the `caveman:caveman` skill at level **full** before anything else — every `/handoff-run` session runs
caveman-full for its whole duration. Caveman boundaries apply as usual: code, commits, PRs, and generated artifacts
(PRD/plan/ANA docs, HANDOFF entries, decision write-ups + index lines) are written normal; only chat
output compresses.

### 1. Locate the item

Read the **current repo's** root `HANDOFF.md` (the repo the session runs in; IDs are per-repo — never resolve an ID
against another repo's HANDOFF). Item not found → list the open items with their IDs and stop.

### 1.5 Select (`next` only)

Argument is an explicit `<ITEM-ID>` → skip this step. Argument is the literal `next` → resolve the candidate in a
**Sonnet 5 subagent**, then handle the maintainer half on the session model.

Selection is bounded, rule-driven reading (one script + two HANDOFF files + `references/selection.md`), so it runs on
the cheaper tier without loss. There is no model switch: the main thread keeps the session model throughout, and only
this one `Agent` call is overridden.

**Subagent call** — `Agent` with `subagent_type: "general-purpose"`, `model: "sonnet"`,
`run_in_background: false` (its result gates the rest of the run). Prompt it to:

1. run `bash .claude/skills/handoff-run/scripts/next-item.sh` (macOS/Linux) or
   `pwsh .claude/skills/handoff-run/scripts/next-item.ps1` (Windows) for the machine half (candidate table with verified
   blocked-on links, in-flight signals, dependent counts, tie flag);
2. read `references/selection.md` and apply the judgment half — E2 anchors (`Remaining` notes), phase-scoped
   blockers, `blocked?` items, R1–R4 ranking;
3. return the **drafted selection block** verbatim in `selection.md` §5 format, plus an explicit `Ask: yes|no` line —
   `yes` with the tied/ambiguous candidates and the one fact each, whenever `selection.md` §4 says ask.

The subagent has **no authority**: it never asks the maintainer, never routes, never writes files, never picks a
winner across a §4 ambiguity.

**Session model resumes here.** Print the returned selection block. `Ask: yes` → put the tied candidates to the
maintainer — **ties that only file order would break are asked, never guessed**. Subagent unavailable, or its block
malformed / contradicting the script output → redo the step inline on the session model; the override is an
optimization, never a dependency.

The maintainer may reject the selection outright and name a different ID. From here the flow is identical to an
explicit ID; the selected item is what step 2 routes.

### 2. Route

Apply `references/routing.md`. Print the verdict block (path, criteria fired, reasoning) — format per `references/routing.md`.

- `--dry-run` → stop here.
- Otherwise **wait for the maintainer**: accept, or override the path. Never start executing on an unconfirmed verdict.

### 3. Execute the path

| Path     | Chain                                                                                                                                                                                                                                                                                        |
|----------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **ANA**  | Research the question (graphify first for codebase questions; web where relevant) → write `docs/ANA-N.md` (prior art, options, verdict, phasing — mirror existing ANA docs) → no implementation. A concluding verdict that spawns `MOD-N` items opens them per `references/lifecycle.md` P0. Verdict recommended the **research** phase for ultracode (and the maintainer accepted) → run the survey as one Workflow-tool script (multi-modal sweep: per-repo/per-backend readers + web survey in parallel, every code-exploration prompt carrying the graphify-first rule); one agent still writes the ANA doc from the sweep's returns. |
| **PRD**  | `plan-prd` (open questions surfaced to maintainer) → `plan` on the PRD → plan fact-check → CONFIRM gate → implementation phase below.                                                                                                                                                |
| **plan** | `plan` with the HANDOFF item text as input → plan fact-check → CONFIRM gate → implementation phase below.                                                                                                                                                                                |

### 3.5 Plan fact-check (PRD/plan paths, before CONFIRM)

A plan asserts facts about the world; the CONFIRM gate must be offered over checked facts, not prose. Before CONFIRM,
extract every verifiable claim the plan makes and verify each against the tree:

- **Tree facts** (compile flags present, call-site counts, existing guards/attributes) — grep/read; seconds each.
- **Toolchain / standard-library behavior** (an attribute adds a diagnostic, a construct compiles, a default is
  generated) — compile probe against the repo's actual toolchain, not recall.
- **Task independence** — tasks the plan marks independent must each list their touched files; verify by intersecting
  the sets. A task marked independent without a file list **fails** the check.

Falsified claims amend the plan before the maintainer sees it; record all verdicts in a "verified claims" table inside
the plan artifact (columns: claim | verdict | evidence). The step is unconditional — only its execution shape
varies: claim checks are independent of each other, so when numerous they may run as one Workflow-tool fan-out (no
routing trigger needed).

### 4. Implementation phase (PRD/plan paths, after CONFIRM)

1. **`code-architect`** agent produces the blueprint from the approved plan.
2. **Implementer fan-out**: tasks the plan marks independent run as parallel agents; TDD per repo convention (tests
   first). Every subagent prompt doing code exploration includes the graphify-first rule. Independence is decided by
   the fact-checked file sets from step 3.5, never by plan prose — a non-empty intersection strips the parallel
   marking; the affected tasks run serial (or in isolated worktrees when the split is worth it).
   **Ultracode mode** (verdict recommended the **implement** phase and the maintainer accepted, per
   `references/routing.md`): run this step as one Workflow-tool script instead of individual agent calls — pipeline the
   plan's independent tasks through implement → adversarial-verify stages, deterministic fan-out, findings fed to
   the review gate (step 3 below). The confirmed verdict is the orchestration opt-in; scale the agent count to the
   plan's task list, not beyond it. Orchestration is never permission to skip TDD or any gate.
3. **Review gate** — the repo's configured reviewer agent always runs over the full change set; it is the final
   review authority and nothing replaces it. Resolve it from the
   repo's `.claude/workflow-config.json`, key `reviewer` (agent name, e.g. `"cpp-reviewer"`). File or key missing →
   propose a reviewer from the repo's dominant language, confirm with the maintainer, write the file, then run it.
   `workflow-config.json` is per-repo and deliberately **outside** the synced workflow surface —
   `sync-workflow-surface.sh`/`.ps1` never owns or overwrites it. Findings applied (or explicitly deferred with the
   maintainer) before close-out. Ultracode does not replace this gate. Verdict recommended the **review** phase
   (maintainer accepted) → after the configured reviewer runs, fan its findings through a Workflow-tool
   adversarial-verify pass (one verifier per finding) before applying/deferring them — the verify pass only checks
   findings, never substitutes for the reviewer.

### 5. Close-out

1. Bookkeeping per `references/lifecycle.md` (P1 phase note if milestone of a multi-phase item; P2 full close-out if the
   item is done).
2. Run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (macOS/Linux) or
   `pwsh .claude/skills/handoff-run/scripts/validate-workflow-docs.ps1` (Windows) from the repo root. **Non-zero exit
   blocks the done-report** — fix findings first. This manual run is whole-repo and is the gate for the
   done-report. The same validator also runs at commit time (`pre-commit` hook, `scripts/install-workflow-hooks.sh`/
   `.ps1`, M3), but there it blocks on errors in the **staged** files only — a pre-existing finding elsewhere prints
   as `[WARNING] ... (pre-existing ...)` and does not block an unrelated commit. Human escape hatch:
   `WORKFLOW_SKIP_HOOK=1`. Sub-repo hooks additionally block staged surface files that differ from the workspace
   source (drift-aware: committing sync results passes; local divergence blocks — edit the workspace source and sync
   instead).
3. Update PRD milestone row / plan artifact status where they exist.
4. Report: what shipped, commits, validator result. Push only when agreed (the repo's
   `CLAUDE.md` push policy; workspace-level where the repo is a submodule).

## Hard rules

- Maintainer gates that never disappear: routing confirm (step 2), `plan` CONFIRM, PRD open questions, deliberate
  push.
- CONFIRM is only offered over a fact-checked plan (step 3.5); implementer fan-out never trusts prose independence —
  file-set intersection decides.
- `next` selects, it never invents: only open checklist lines in the current repo are candidates, a blocked item is
  never auto-selected, and a tie that only file order would break is asked. Selection is a separate confirm from
  routing — printing the selection block does not consume the step-2 gate. The Sonnet 5 selection subagent drafts
  the block only; asking, confirming, and every later gate stay on the session model's main thread.
- ANA path never writes implementation code.
- **Windows runs the `.ps1` variant of every script this skill names, and of every script git runs on
  its behalf** (see the References note). Never `bash …/*.sh` on this platform.
- No done-report over a red validator.
- This skill orchestrates; it does not duplicate the law — `workflow-docs.md` wins on any bookkeeping conflict.
