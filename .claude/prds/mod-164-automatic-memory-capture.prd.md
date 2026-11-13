# MOD-164 - Automatic memory capture at run end

Origin: ANA-38 (`docs/ANA-38.md` §7.2 P2, §4.6, §6). Builds on MOD-163 (`docs/decisions/mod/mod-163.md`).
Requirements: `R-ENT-13`, `R-ID-6` (memory exception applied 2026-10-20), `R-ID-7`, `R-ORCH-1`.
Routed **PRD** by `/handoff-run` on 2026-11-13 (C2, C3, C4; ultracode recommended for implement).

## Problem
A person running htui over projects watches agents rediscover the same gotchas run after run: flaky
suites, required flags, toolchain quirks. Each rediscovery costs retries and tokens, and the lesson
dies with the run's transcript. MOD-163 gave projects a memory store and a prompt section, but only a
person writes to it, and nobody stops mid-backlog to type entries. So in practice the store stays empty
and the prompt section never fires.

## Evidence
- The maintainer keeps the same loop by hand outside htui: the Claude auto-memory for this repo holds
  about 50 gotchas that agents learned the hard way and would otherwise rediscover. Examples: the suite
  passes or fails depending on scheduling (`--test-threads=1`), `qdrant_live` times out under load,
  "zeta" is a forbidden literal, and integration tests run zero tests without `--features testkit`.
- ANA-38 §4.6 records the counter-evidence: in the literature, 11 of 12 self-built memory pairings did not
  beat memory switched off. That is why the write path admits less rather than more.
- Benefit to later runs: **Assumption — needs validation via MOD-169** (before/after on real htui steps).
- Extraction cost per run: **not measured** (ANA-38 §9.2). The token cap bounds it; MOD-169 measures it.

## Users
- **Primary:** the person running htui's backlog over one or more projects. They want project knowledge
  that agents learn the hard way to reach later runs without having to write it down themselves.
- **Not for:** workspace-owned or global knowledge (MOD-147, MOD-168); coordinator turns (MOD-136); chat
  runs.

## Hypothesis
We believe **an automatic extraction turn at run end, inside a deterministic write path,** will **stop
agents rediscovering the same project gotchas** for **people running htui over projects**.
We'll know we're right when **a project's memory fills from its own failing and retried runs with no
person typing, and no written entry breaks a write-path rule** (every entry cites evidence observed in
its run, nothing unscrubbed is stored, and no run writes more than its cap). Whether those entries make
later runs better is MOD-169's measurement.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Runs with a failure or retry that end with an extraction step recorded (written, skipped, or failed with a reason) | 100% | Run records over a test backlog |
| Runs with no failure or retry that start an extraction turn | 0 | Run records (skip reason recorded) |
| Stored run-written entries without run evidence, over the cap, or unscrubbed | 0 | Conformance tests; a store audit query |
| Runs whose final status changes because extraction failed, timed out or ran over budget | 0 | Conformance tests |
| Person-written entries replaced by a run | 0 | Conformance tests; revision history |
| Extraction tokens per run | under the token cap; actual value TBD | Recorded step usage; MOD-169 |

## Scope
**MVP**, two milestones:
1. **Write path.** Entries a run writes carry a `run` source and provenance that the platform stamps (run,
   agent, item, commit); the model cannot set it. Every entry is schema-checked, must cite something
   observed in the run (a step, an error text, a path) or is rejected, is scrubbed fail-closed before it
   is stored, and counts against a per-run write cap (starting at 3). An entry whose subject matches a
   live **run-written** entry in the same project supersedes it automatically, and the old one is kept
   for audit. A clash with a live **person-written** entry drops the run's write and records why: a run
   never replaces a person's entry. Deterministic usage counters record how many steps each entry was
   shown to and how those steps ended (done or failed). Run-written entries reach prompts through
   MOD-163's section, labelled as model-written. The Memory pane shows who wrote each entry.
2. **Extraction turn.** A run-end step before close-out, recorded as its own step (agent, model, usage,
   and its outcome or the reason it was skipped), visible in Runs. It runs only when the capture switch
   (`memory_capture`, already at the app and project rungs) is on, **and** the run had a failed step, a
   retried attempt, a failed verify or a gate rejection or retry. A failure followed by a passing attempt
   counts as the "win". It reads the run's own records, never a whole transcript, and stays within a
   per-run token cap (starting values set in the plan as a constant MOD-169 tunes). The agent and model
   come from **the light-model setting**: one app-level `{agent, model}` key with its validator and
   Settings field, the key MOD-126 planned. This item builds the key and MOD-126 keeps only the workflow
   pick. If no model is configured, the turn is skipped and the skip is recorded. If the turn fails,
   times out or runs over budget, a note says so, and the run ends exactly as it would have without
   memory.

**Out of scope**
- Lesson detector over run records (repeated signatures, timeouts, outlier cost, "measured wins") —
  MOD-165.
- Freshness sweep, pins, suspect state and re-validation; auto-retire; pin-as-trusted — MOD-166.
- Near-duplicate detection (Qdrant raw cosine) and memory search — MOD-167. No full-text matching and no
  Qdrant query at write time here.
- Global scope and promotion — MOD-168.
- Measurement and tuning of caps, budget and usefulness — MOD-169.
- Workspace-owned entries — need MOD-147. Coordinator seed — needs MOD-136.
- MOD-126's run-start workflow pick (only the setting key moves here).
- Demotion or retirement *driven by* the usage counters — this item records them; acting on them is
  MOD-166/MOD-169's.
- Discounting evidence from runs that an entry shaped (ANA-38 §6 proposal, unverified).

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Run write path | Run-written entries can be stored safely: validated, evidenced, scrubbed, capped, stamped and counted. They supersede only run entries, show in prompts and in the pane as model-written, and are exercised by conformance tests through a store-level writer. | complete | `.claude/plans/mod-164-capture.plan.md` |
| 2 | Extraction turn | Runs with failures or retries end with a recorded capture step that writes entries through the M1 path. It uses the light-model setting, is skipped with a reason otherwise, stays under the token cap and never changes the run's outcome. | complete | `.claude/plans/mod-164-capture.plan.md` |

## Open Questions
Answered at the PRD gate (maintainer, 2026-11-13):
- [x] **Model seam:** build MOD-126's light-model key here; MOD-126 keeps only the workflow pick. The
  HANDOFF line for MOD-126 is amended to match.
- [x] **Person against run:** a person's entry always wins; the run's write is dropped and the drop is
  recorded.
- [x] **Trigger:** failures and retries only (failed step, retried attempt, failed verify, gate rejection
  or retry). Measured wins wait for MOD-165.
- [x] **Run record:** the turn has its own step row and never fails the run.
- [x] **Budget:** a token cap (input evidence plus output), as a tunable constant; the write cap starts at 3.

Still open (for `/plan`):
- [ ] Where the extraction step sits in the run's step order, and how it interacts with stop and resume
  (MOD-125) and with crash recovery. TBD — needs validation via a code read of the run-end path.
- [ ] Which provenance and counter fields fit `memory_entry` as columns and which need a side table; this
  decides the migration shape and whether `GraphSnapshot::V` moves (MOD-163's note: a new
  `MemorySource` ships with a migration). TBD — plan.
- [ ] Whether usage counters update at step end (inside an existing write) or in a sweep. TBD — plan.
- [ ] Prompt rendering: do run-written and person-written entries share MOD-163's single 1,500-token cap
  and its newest-first order, or do person entries rank first? TBD — plan; default to shared and
  unchanged unless the plan finds a reason.
- [ ] How the extraction turn's output is parsed: a structured document or tool output, its schema, and
  what happens to partially valid output (keep the valid entries, drop and record the rest?). TBD — plan.
- [ ] What counts as "the commit" in provenance for a run with several repos or none (folder projects,
  MOD-145). TBD — plan.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Memory poisoning: injected text in tool output or errors becomes a standing instruction | Medium | High | Platform-stamped provenance; evidence must cite the run; scrub; per-run write cap; entries rendered as quoted data labelled model-written; the pane lets a person retire them. Residual risk stays until MOD-166. |
| Wrong lessons reinforce themselves | Medium | Medium | Usage counters recorded for later demotion (MOD-166/169); evidence must come from the run, not from a prior entry. |
| A model call on every failing run costs the user's plan | High | Medium | Trigger skips clean runs; token cap; `memory_capture` switch per app and project; skip when no model is configured. |
| Runaway growth of entries | Medium | Low | Write cap; automatic supersede on subject; the prompt cap unchanged. The store itself is unbounded until a retention rule exists. |
| Extraction step breaks run end, stop, resume or recovery | Medium | High | Never fails the run; recorded as its own step; plan must cover the MOD-125 stop/resume and crash-recovery paths with tests. |
| Collision with MOD-126 when both touch the light-model key | Low | Medium | MOD-126's HANDOFF line amended so it builds on this key, not a second one. |
| Migration renumber at collect (parallel branches) | High | Low | Known procedure: renumber at merge and move the count pins (MOD-163 note). |

---
*Status: COMPLETE — both milestones built (MOD-164 done, 2026-11-13; `docs/decisions/mod/mod-164.md`).*
