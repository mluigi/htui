---
name: handoff-run
description: "Run a HANDOFF.md item through its full lifecycle from one command: select (explicit ID, or `next` to resolve the logically next item and ask the maintainer on a tie), route (PRD/plan/ANA), confirm with maintainer, execute the chain (plan-prd, plan, task briefs, implementers, configured reviewer), then apply HANDOFF/DECISIONS bookkeeping per workflow-docs.md and validate it. Trigger: /handoff-run <ITEM-ID>|next [--dry-run]."
---

# /handoff-run

One command per HANDOFF item lifecycle. Auto-chain with the maintainer present — every gate below stays interactive.

## Usage

```
/handoff-run MOD-4            # route + confirm + execute full lifecycle
/handoff-run next             # pick the logically next item, then the same lifecycle
/handoff-run ANA-5 --dry-run  # routing verdict only, nothing executes
/handoff-run next --dry-run   # selection block + routing verdict only, nothing executes
/handoff-run MOD-4 --model implementer=sonnet                      # one stage on another model, this run only
/handoff-run MOD-4 --model implementer=sonnet --model verifier=haiku
```

## Stage models

Each stage runs on the session model unless a model is set for it. Stages: `plan` (plan drafting and its
fact-check), `architect`, `implementer` (implementers and repairs), `verifier`, `reviewer`. Values: `sonnet`,
`opus`, `haiku`, or `inherit`. Resolution, first hit wins:

1. `--model <stage>=<model>` on the command (repeatable) — this run only.
2. The `HR_MODELS` environment variable, same pairs comma-separated (`implementer=sonnet,verifier=haiku`).
   `scripts/hr attach` forwards it into a sandbox, so `HR_MODELS=implementer=sonnet scripts/hr up MOD-4` sets a
   run's models from the host.
3. `.claude/workflow-config.json`, key `models` (`{"implementer": "sonnet"}`; `null` or absent = inherit) — the
   repo default.

An unknown stage or model name is a usage error: stop and say so, never guess. Print the resolved table in the
routing verdict (step 2) on a line `Models: implementer=sonnet (--model), others inherit`, so the maintainer
confirms it with the route. Apply it by passing `model` on that stage's `Agent` call, and `args.models` to
`scripts/task-lane.js`; a stage that inherits gets no `model` at all. The selection subagent of step 1.5 is not a
stage and keeps its own rule. A stage that fails on its model (quota, 402) is reported and re-run on the session
model only after the maintainer agrees: a silent fallback would spoil the comparison the override was for.

## References (load per phase, not all up front)

- `references/selection.md` — `next` only: candidate set, eligibility, ranking, ambiguity rule
- `scripts/next-item.sh` / `.ps1` — `next` only: pre-filter computing the machine half of selection (candidate table,
  verified blocked-on links, in-flight/dependent facts, tie flag); judgment calls stay with `selection.md`
- `references/routing.md` — decision table, verdict format, override rules
- `references/lifecycle.md` — bookkeeping procedures (open / phase note / close-out)
- `references/ecosystem-survey.md` — wrap-vs-fork record (build-time, not needed at run time)
- `scripts/next-item-id.sh` / `.ps1` — the owned-ID mint, run whenever a path spawns a new item (lifecycle P0)
- `scripts/hr-mint` (repo root, Linux) — the leased mint; P0 mints through it whenever `scripts/hr-mint --leasing`
  exits 0 (always in a sandbox; on the host once `scripts/hr up` has been used). User guide: `docs/hr-sandbox.md`
- `scripts/task-lane.js` — the Workflow-tool script for the implementation phase (step 4); its header documents `args`
- `scripts/token-usage.py` — token usage per item, stage, task and model from the session transcripts (step 5)
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

**Enforced since MOD-43 M3, not merely documented.** All four `.sh` entry points
(`install-workflow-hooks`, `validate-workflow-docs`, `next-item`, `next-item-id`) detect a Windows
shell — `uname -s` reporting `MINGW*`/`MSYS*`/`CYGWIN*` — and
**exit 2 naming the `.ps1` to run instead**. The installer is the one that matters most: the hooks
*it* generates call `bash …/*.sh`, so a single Windows run of the `.sh` variant would replace every
`pwsh` call site with a bash one and make every later commit pay the difference. Override only for a
deliberate twin-parity check: `WORKFLOW_ALLOW_SH_ON_WINDOWS=1`. The twins stay behaviourally
identical — the refusal is about which one this platform pays for, not which one is correct.

## Flow

### 1. Locate the item

In a sandbox (`HR_SANDBOX=1` is set) read **Sandbox mode** below first — it overrides steps 1, 1.5 and 5.

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

Cost rule behind every step here: an agent pays for its whole context on every turn, so each agent gets the smallest
text that specifies its job, runs the smallest check that proves it, and is spawned only where it is the one that
catches the defect. Review authority sits in one place, the review gate.

1. **Plan carries the design.** The confirmed plan's task sections must each stand alone: files owned, the types and
   signatures to add or change, the tests to write, and the task's own check command (crate-scoped, never the
   workspace gate). Shared decisions sit in one short section. A plan that lacks this is amended before sub-step 2, not
   patched by an architect.
   **`code-architect` runs only when C2 fired** (new public API, module seam or engine-structure decision) or the
   maintainer asks for it. Its blueprint then holds the design decisions and the deltas to the plan, not a second
   copy of the tasks: at most ~300 lines, and every delta is folded into the task sections it changes.
2. **Task briefs.** For each task write `.claude/plans/<name>.briefs/<KEY>.md`: the shared-decisions section plus
   that task's section, cut out of the plan (and blueprint) mechanically, not re-written. The brief is the only
   planning text an implementer or verifier reads. A task whose brief exceeds ~300 lines is split.
3. **Implement.** TDD per repo convention (tests first). Independence is decided by the fact-checked file sets from
   step 3.5, never by plan prose: a non-empty intersection strips the parallel marking, and the affected tasks run
   serial (or in isolated worktrees when the split is worth it).
   - **Light shape** (the plan has at most two tasks, or touches about five files or fewer): one implementer per
     task as a plain `Agent` call from its brief. No workflow.
   - **Lane shape** (everything else, and whenever the confirmed verdict recommends ultracode for **implement**):
     run `scripts/task-lane.js` through the Workflow tool with `scriptPath` and `args` (the header of the script
     documents them). One run takes one lane of serial tasks on one checkout; parallel lanes are concurrent runs in
     separate worktrees. The confirmed verdict is the orchestration opt-in; one implementer per task, never more
     agents than the plan has tasks plus the verifiers below.
   - **Verifier, by exception.** A task gets one read-only verifier (conformance and breakage in one agent, over
     the task's own commit range) only when it is a **foundation** task that later tasks build on, or a
     **fix round** (sub-step 5 below). Medium and high findings get one repair; a repaired high gets one re-check of the
     repair's diff alone. Low findings are not repaired there: they go to the review gate's list. All other tasks
     go to the gate and the review unverified by design.
   - **Small same-shape tasks are batched** into one brief and one implementer.
   - **Agents never run the workspace gate.** An implementer runs its task's check, clippy and rustfmt on the crates
     it touched. A workflow agent's prompt cache lasts five minutes, so a long build inside an agent reloads its
     whole context at full price.
4. **Gate, once per lane.** After a lane finishes, the main thread runs the gate itself (`bash scripts/gate.sh`, or
   the item's narrower gate when the plan names one) and reads the result. A red gate goes back to one implementer
   with the failing output and that task's brief.
5. **Review gate.** The repo's configured reviewer agent always runs over the full change set; it is the final
   review authority and nothing replaces it. Resolve it from the repo's `.claude/workflow-config.json`, key
   `reviewer` (agent name, e.g. `"cpp-reviewer"`). File or key missing → propose a reviewer from the repo's dominant
   language, confirm with the maintainer, write the file, then run it.
   Its prompt carries: the exact range (`git diff <base>..HEAD`), the plan path with the instruction to check the
   change against the plan's tasks as well as for quality, and the low findings carried from sub-step 3. One reviewer
   run per item; split it into parallel area instances only when the diff exceeds ~5,000 lines.
   Findings are applied (or explicitly deferred with the maintainer) before close-out, in **one fix round**: the
   findings go to one implementer per affected area as a `verify: 'fix-round'` task, whose prompt tells it to
   confirm each finding against the code before acting. The fix round's verifier reads the fix diff only. There is
   no verifier per finding, and no second reviewer pass, unless the maintainer asks for one.

### 5. Close-out

1. Bookkeeping per `references/lifecycle.md` (P1 phase note if milestone of a multi-phase item; P2 full close-out if the
   item is done).
2. Run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (macOS/Linux) or
   `pwsh .claude/skills/handoff-run/scripts/validate-workflow-docs.ps1` (Windows) from the repo root. **Non-zero exit
   blocks the done-report** — fix findings first. This manual run is whole-repo and is the gate for the
   done-report. The same validator also runs at commit time (`pre-commit` hook, `scripts/install-workflow-hooks.sh`/
   `.ps1`, M3), but there it blocks on errors in the **staged** files only — a pre-existing finding elsewhere prints
   as `[WARNING] ... (pre-existing ...)` and does not block an unrelated commit. Human escape hatch:
   `WORKFLOW_SKIP_HOOK=1`.
3. Update PRD milestone row / plan artifact status where they exist.
4. Usage table: run `python3 .claude/skills/handoff-run/scripts/token-usage.py --item <ITEM-ID>` and put its
   output in the report as-is (stage, task, model, cost share, cache reloads, verifier findings). It reads this
   session's transcripts, so it is a measurement, not an estimate; `--help` explains the columns.
5. Report: what shipped, commits, validator result, the usage table, and the stage models that were in force. Push only when agreed (the repo's
   `AGENTS.md` push policy).

## Sandbox mode (`HR_SANDBOX=1`)

Set by `scripts/hr` (TOOL-7; user guide `docs/hr-sandbox.md`). The session runs in a private clone on
branch `hr/$HR_ITEM`, with its own Postgres and Qdrant on the usual `localhost` ports, no Docker and no
git credentials. These overrides apply; everything else in this file — every maintainer gate and the
reviewer gate included — is unchanged.

- **Steps 1 / 1.5 — the item is `$HR_ITEM`.** `next`, or a bare `/handoff-run`, resolves to it with no
  selection subagent. A different ID is asked, never followed silently: the item was chosen on the host
  by `scripts/hr up`, whose picker already hides items that have a run.
- **New items (lifecycle P0) mint through `scripts/hr-mint`** — leasing is always on in a sandbox, and
  the mint also raises by the read-only host tree at `/host/htui`
  (`hr-mint: leased <ID> for hr/<ITEM> (tree next <ID>, host next <ID>|unavailable, lease floor <ID>|none)`).
  A missing lease file or state dir is exit `1`, never a tree-only mint
  (`hr-mint: in a sandbox (HR_SANDBOX=1) leasing is required, but there is no lease file at /hr-state/id-leases.tsv - nothing done; the maintainer restores it on the host`):
  stop and report it; never fall back to `next-item-id.sh`, and never run `scripts/hr-mint --init`
  here (it is host-only and refuses). Its sibling listing is how two runs avoid filing one problem
  under two IDs: when a listed title looks like the same issue, ask the maintainer before filing; on
  reuse, cite the sibling's ID in this run's phase note instead of creating the item.
- **Database** — `psql -h localhost -p 5439 -U postgres` (trust auth, no password).
  `HTUI_TEST_DATABASE_URL` and `HTUI_TEST_QDRANT_URL` are already set. There is no `docker` here: a
  recipe that says `docker exec htui-postgres psql …` becomes the same `psql` command against
  `localhost:5439` (the `sqlx prepare` variant is in `docs/hr-sandbox.md`).
- **Durable notes** — this run's Gortex daemon and store, and every Gortex memory it writes, are private to
  the run and are deleted when it is purged. Anything that must outlive the run goes in the repo (plan,
  write-up) or the Claude auto-memory under `~/.claude`, which is shared with the host.
- **Step 5, report — never push, never open a PR** (`origin` is the read-only host repo). The
  done-report ends with exactly: `branch hr/<ITEM> ready — on host: scripts/hr collect <ITEM>`.
  Merging back, and the real Postgres gate on the merged tree, happen on the host.

## Hard rules

- Maintainer gates that never disappear: routing confirm (step 2), `plan` CONFIRM, PRD open questions, deliberate
  push.
- CONFIRM is only offered over a fact-checked plan (step 3.5); implementer fan-out never trusts prose independence —
  file-set intersection decides.
- Implementers and verifiers read their task brief, not the plan; no agent runs the workspace gate; the configured
  reviewer is the one review authority and runs once per item (step 4).
- `next` selects, it never invents: only open checklist lines in the current repo are candidates, a blocked item is
  never auto-selected, and a tie that only file order would break is asked. Selection is a separate confirm from
  routing — printing the selection block does not consume the step-2 gate. The Sonnet 5 selection subagent drafts
  the block only; asking, confirming, and every later gate stay on the session model's main thread.
- ANA path never writes implementation code.
- **Windows runs the `.ps1` variant of every script this skill names, and of every script git runs on
  its behalf** (see the References note). Never `bash …/*.sh` on this platform.
- No done-report over a red validator.
- In a sandbox (`HR_SANDBOX=1`) the item is `$HR_ITEM` and nothing is pushed — see Sandbox mode.
- This skill orchestrates; it does not duplicate the law — `workflow-docs.md` wins on any bookkeeping conflict.
