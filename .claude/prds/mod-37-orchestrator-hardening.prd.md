# MOD-37 - Orchestrator hardening follow-ups

## Problem
The maintainer-operator who runs backlog items through `htui` and `htui worker` carries a set of
known, recorded orchestrator risks from MOD-4: a parked run shows no reason, a hung agent session
holds its slot until the per-run cap or a human cancel, a promoted ACP step silently gets a fresh
context, the Runs pane goes stale or blank in several windows, and a few crash and store-parity
windows remain. None blocks a manual run today. Left open, they make unattended and long-running
runs (MOD-43, MOD-46, MOD-47) harder to trust.

## Evidence
- MOD-4 write-up, "Carried" section (`docs/decisions/mod/mod-4.md`), and the per-risk blueprint
  citations in HANDOFF's MOD-37 line.
- ANA-27 §5.1 T4 (deadline) and T5 (context-not-carried note).
- No field incident or metric. Assumption: the risks matter in practice only once runs go
  unattended; needs validation via the first long unattended run after MOD-43.

## Users
- **Primary**: the maintainer-operator driving runs from the Runs pane and Chat tab, or running
  `htui worker` unattended.
- **Not for**: end users of a future multi-tenant control plane (MOD-47).

## Hypothesis
We believe **closing the carried orchestrator risks** will **remove the silent-failure and
stale-view modes MOD-4 documented** for **the maintainer-operator**.
We'll know we're right when **each risk has a regression test that fails on the old behaviour, and
HANDOFF carries no R-line from this item except those re-deferred with a stated reason**.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Risks closed or re-deferred with reason | 100% of the listed set | HANDOFF line audit at close-out |
| Regression test per closed risk | 1 each, red before fix | test names in each milestone write-up |
| Hung agent session | settles as `DeadlineElapsed` at the step deadline | test with a stalled fake driver |

## Scope
**MVP** — every listed risk closed, or explicitly re-deferred with a reason: R-3, R-5, R-6, R-29,
R-30, R-31 remainder, R-32, R-37, R-40, R-41, R-44, R-46, R-48, R-49, R-51, R-53, T7's residual
window, and the ANA-27 agent-session deadline. Cites `R-ORCH-3`, `R-ORCH-5`, `R-ORCH-8`,
`R-ORCH-9`, `R-TUI-4`, `R-HIS-1`, `R-NF-3`.

**Out of scope**
- R-38 — closed by MOD-42.
- Engine-step follow-ups — owned by MOD-70.
- R-55's `command_limits` editor — nothing edits it today; stays "whoever adds an editor".
- New entries in `docs/REQUIREMENTS.md` — maintainer decision only.
- A per-step deadline setting — the deadline settles as `DeadlineElapsed` only (ANA-27 T4).

## Delivery Milestones
| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Run state and visibility | A parked run shows its reason; the Runs pane no longer drops, delays or hides step state (R-3, R-40, R-41, R-44, R-51, R-53, T7); R-44 and R-53 re-deferred | complete | `.claude/plans/mod-37-run-state.plan.md` |
| 2 | Store and engine correctness | Crash and store-parity windows closed; override copies record their agent (R-5, R-6, R-29, R-30, R-31 remainder, R-32); R-32's D131 half re-deferred | complete | `.claude/plans/mod-37-store-engine.plan.md` |
| 3 | Git cost | Reconciling a diff opens the repository once (R-37) | complete | `.claude/plans/mod-37-git-cost.plan.md` |
| 4 | Deadline and sessions | A hung agent session is cancelled at the step deadline; no other run can prepare a promoted chat's checkout; an offline swap no longer strands a walk (deadline, R-49, R-46); R-49 closed by overlap admission plus a pin | complete | `.claude/plans/mod-37-deadline-sessions.plan.md` |
| 5 | ACP resume | A promoted ACP step resumes its context, or says it did not (R-48, ANA-27 T5); a failed resume reports and opens a labelled handoff | complete | `.claude/plans/mod-37-acp-resume.plan.md` |

## Open Questions
- [x] R-48: full ACP resume plus the T5 note (M5 plan, maintainer 2026-10-03).
- [ ] R-55: confirm it stays carried as documentation only (current reading) rather than gaining a
  limits re-read hook now.
- [ ] M1 breadth: R-3 touches three summary builders and the mirror; decide in M1's plan whether
  R-40's step-start hook rides with it.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `htui-orch` test stack headroom (`every_case_name_dispatches` near 2 MiB) | M | M | box large futures; gate with `--no-fail-fast`, grep SIGABRT |
| Store changes need a migrated scratch DB for `sqlx prepare` | M | L | follow the sandbox recipe in `docs/hr-sandbox.md` |
| M2 touches schemas both stores must keep in parity | M | H | extend the one conformance suite first |
| ACP `session/load` is larger than one milestone | M | M | the T5 note is the fallback |

---
*Status: complete 2026-10-04 — all five milestones landed; write-up `docs/decisions/mod/mod-37.md`.*
