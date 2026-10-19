# MOD-125 — Stopping and resuming runs

> Routed as **PRD** by `/handoff-run MOD-125` (criteria C2, C3 and C4 fired; C4 estimated from the item
> text). Accepted by the maintainer 2026-10-18, sandbox run `hr/MOD-125`. Ultracode accepted for the
> implement phase, and for the review verify pass. Origin: UI redesign review,
> `docs/ui-redesign-findings-2026-10-15.md` §4.1. Requirements: `R-ORCH-16`, `R-TUI-14`, `R-TUI-18`,
> `R-AGT-1`. PRD gate: maintainer accepted the recommended answers to Q1-Q5, 2026-10-18.

## Problem

A person running several items in a workspace cannot halt them and pick them up later. The only verb
is cancel, and `cancelled` is terminal: the run never comes back, and what the agents had built up in
their conversations is thrown away. Pausing the auto queue only stops new admissions, per box, and
leaves running steps running. Leaving the TUI never stops work (`R-TUI-18`), so without a stop there is
no way to free the machine or the quota without losing the runs. `R-ORCH-16` asks for a workspace
stop, graceful or forced, that always saves the conversation and can always be resumed.

## Evidence

Read at `41bf5c90` (branch `hr/MOD-125`). Paths are relative to the repo root.

- **Maintainer design decision, not an incident.** `R-ORCH-16` was added on 2026-10-17 in the UI
  review (`docs/REQUIREMENTS.md:442`, findings §4.1 at `docs/ui-redesign-findings-2026-10-15.md:157-175`).
  There is no user report or failed run behind it. Real use is an **assumption — needs validation by
  using the CLI verbs (M4) on real runs**.
- **No resumable state exists.** `RunStatus` is `queued | running | awaiting_approval | done | failed | cancelled`,
  and `StepStatus` adds `pending | superseded` (`crates/htui-core/src/model/run.rs:33-99`). `Cancelled`
  reaches nothing (`run.rs:58-71`). The Postgres CHECKs match (`crates/htui-store/migrations/0001_init.sql:453-454,481-482`).
- **Cancel is durable and partly graceful.** `run_command` rows of kind `cancel` (`0011_permission_relay.sql:47-61`)
  are applied by the lease holder. The holder sends `Signal::Cancel{grace}` (`CANCEL_GRACE` = 2 s,
  `crates/htui-worker/src/runtime.rs:117,783,2974`), then drops the token, then `finish_run(Cancelled)`
  (`crates/htui-orch/src/engine.rs:1835-1895`).
- **The drivers can cancel a turn cleanly and reopen a session.** ACP `session/cancel` waits for a real
  `StopReason` (`crates/htui-agent/src/acp/mod.rs:1772`). ACP `session/resume`/`session/load` and the
  CLI's `--resume` exist (`acp/mod.rs:1879,1907`, `cli/mod.rs:203-204`, MOD-37 M5). They are used only
  when a step is promoted to chat (`crates/htui-orch/src/promote.rs:70`). Engine steps hard-code
  `resume: None` (`engine.rs:6583`).
- **The session id is persisted only as a banner row.** It lives in `session_event` (`update="session_started"`,
  `body.session_id`; `acp/mod.rs:1286-1302`, `cli/mod.rs:1332`). There is no column for it.
- **The conversation streams to the store.** The `Recorder` flushes on each variant change, on `Done`,
  and at 16 KiB (`crates/htui-agent/src/record.rs:7-12,129`). A hard kill loses only the unflushed
  buffer and any `edit_proposal` rows held for an open tool call (`record.rs:66-88`).
- **Interrupted steps restart from scratch today.** Crash recovery resets the trees and retries as
  attempt+1 with a fresh session (`engine.rs:2641`, `~2810-2850`). Mid-step resume was dropped on
  2026-09-25 (MOD-24). MOD-125 brings back session resume for **stopped** steps only. Crash recovery is
  unchanged.
- **Nothing is workspace-scoped.** Runs belong to a project (`run.project_id`), and projects join
  workspaces many-to-many (`0001_init.sql:161-167,449`). The queue and the slots are per box
  (`0016_auto_queue.sql`). New runs enter at three points: queue `admit` (`runtime.rs:2173`), manual
  `StartRun`/`claim_run`, and the sweep's `adopt_runs` (`crates/htui-store/src/store/traits.rs:1297-1380`).

## Users

- **Primary**: the maintainer, with several runs going in one workspace (handoff runs, swarms) on the
  TUI or a worker box, who wants all of them halted (quota, machine, end of day, a run going wrong)
  and continued later from where each agent was.
- **Not for**: removing or abandoning work (that is cancel, unchanged); stopping one run on its own;
  runs walked on a remote target (MOD-43).

## Hypothesis

We believe **a workspace stop, graceful or forced, that leaves every run in a resumable stopped state,
and a resume that reopens each agent's saved session**, will **let the maintainer halt parallel work and
continue it later without losing conversation or redoing finished work**.
We'll know we're right when **a resumed agent step continues in the same agent session id, a stopped
check runs again, no run starts in a stopped workspace, and a graceful stop leaves no conversation
event unrecorded**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Same session after resume | resumed agent step's `session_started` banner carries the pre-stop session id (ACP and CLI) | engine tests over the fake driver; adapter tests for load/resume argv and requests |
| Forced stop redoes the turn | the interrupted prompt is sent again into the reopened session, recorded as a new turn | engine + recorder tests |
| Nothing new starts | zero runs admitted, claimed or adopted in a stopped workspace, on the TUI and the worker | store conformance (MemStore and PgStore) + runtime tests |
| Graceful stop keeps the conversation | every event the driver emitted before `Done(Cancelled)` has a `session_event` row | recorder tests |
| Fallback is honest | a session that cannot be reopened reruns the step fresh and records `resume_failed` | engine tests |
| Stop takes seconds | graceful stop settles within the cancel grace plus one command poll | runtime test with paused/real time per the repo's split |

## Scope

**MVP**: a resumable `stopped` state for runs and steps; a durable workspace stop request (graceful or
forced) and a resume request, applied by whichever process walks each run (TUI in process or worker);
a workspace gate over admission, claim and adoption; resume through the agents' saved sessions, with a
fresh rerun as fallback; the `htui stop [--force] <workspace>` and `htui resume <workspace>` CLI verbs
with a confirmation prompt.

Decisions taken at the PRD gate (maintainer, 2026-10-18):

- **Graceful = cancel the turn, keep the session.** A graceful stop interrupts the current turn cleanly
  (ACP `session/cancel`, the CLI's SIGINT path) and keeps the conversation; resume continues the same
  session. It takes seconds, not a whole step. It does not wait for a long single-turn step to end.
- **Forced = kill now.** Resume reopens the session and redoes the interrupted turn.
- **A step that cannot be resumed** (the agent cannot load sessions) stops at once on either stop, and
  runs again fresh on resume. Checks and `htui`'s own steps stop at once and run again.
- **Q1 — shared projects.** A run records the workspace it was started from; stopping a workspace stops
  only those runs.
- **Q2 — surface before MOD-116.** CLI verbs `htui stop [--force] <workspace>` / `htui resume <workspace>`,
  the confirmation being a prompt.
- **Q3 — a step waiting on a permission answer.** The stop drops the question; on resume the turn runs
  again and asks again.
- **Q4 — resume cannot reopen the session** (CLI session files missing, agent lost it). Rerun the step
  fresh and record `resume_failed`, as chat promotion does.
- **Q5 — worker and TUI shutdown stay a hard drop** (MOD-42 OQ-5). "Stop before leaving" (`R-TUI-18`) is an
  explicit stop request sent first, not a change to shutdown.

**Out of scope**

- The Workspaces tab, its overview lines and the `s`/`s s`/`r` keys. **Why:** MOD-116 owns the tab and
  uses this item's verbs.
- The quit dialog's "stop gracefully first". **Why:** MOD-115 owns the dialog and calls this item's request.
- Changing cancel. **Why:** it stays terminal and separate.
- Crash recovery. **Why:** it stays reset-and-retry (MOD-24). Only an explicit stop resumes sessions.
- Stopping a single run. **Why:** `R-ORCH-16` is per workspace. A per-run stop can be added later on
  the same state.
- Remote targets. **Why:** MOD-43 and MOD-130..132.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Stopped state and workspace gate | Runs and steps can be `stopped`; a run knows its workspace; a stopped workspace admits, claims and adopts nothing, on both stores | in-progress | `.claude/plans/mod-125-m1-stopped-state.plan.md` |
| 2 | Stop applied by the executor | A durable stop request, graceful or forced, settles every run of the workspace as `stopped` with the conversation recorded, in process and on a worker | pending | — |
| 3 | Resume through saved sessions | Resume reopens each agent step's session (ACP load/resume, CLI `--resume`), redoes a forced-stop turn, reruns checks, falls back to a fresh rerun with `resume_failed` | pending | — |
| 4 | CLI verbs | `htui stop [--force] <workspace>` and `htui resume <workspace>` confirm, report what stopped and how each run resumes, and the workspace's state reads as stopping or stopped | pending | — |

## Open Questions

- [ ] Do runs created before the migration get a workspace? A project in one workspace is clear. A project in several is not; leaving them unscoped (never stopped by a workspace stop) is the likely answer. TBD — settle in `/plan`.
- [ ] Mapping onto ACP and the CLI adapter (the HANDOFF line asks for this to be settled in the plan): which cancel path the graceful stop reuses, and how a CLI respawn with `--resume` redoes the turn. TBD — `/plan` fact-check against `acp/mod.rs` and `cli/mod.rs`.
- [ ] Where the resumable session id is read from: the banner row, or a new column. TBD — `/plan`.
- [ ] How ANA-2's state table and MOD-24's "no mid-step resume" are amended in the docs (amendment wording). TBD — `/plan`.
- [ ] M1 review findings deferred (R1, 2026-10-19; detail in the M1 plan's "Deferred review findings"): M-3 `delete_workspace` refuses while the workspace has stopped runs (M2); L-3 a readable refusal for a deleted workspace's foreign keys, no fallback to `None` (M2); L-7 `select_fanout`'s loser and judge lists gain `stopped` (M3) and whether a stopped workspace's queued runs count against `admit`'s `free_slots` (M2); L-5 raise the minimum binary/schema version before any `stop_workspace` caller ships (M2); L-6 `queue_entry.workspace_id` stays unindexed (note).

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| CLI `--resume` needs the session files on the box that walks the run; a resume adopted by another box cannot find them | Medium | Resume falls back to a fresh rerun | Q4 fallback with `resume_failed`; prefer resuming on the box that stopped the run |
| A stop races a step finishing, a cancel or a crash sweep | Medium | A run lands in the wrong state | Transition law in both stores; conformance tests for each race |
| A forced stop loses buffered text or held `edit_proposal` rows | High (by design) | Small gap in the transcript; resume redoes the turn anyway | Flush the recorder on stop where the process is still alive; document the loss |
| The new status breaks exhaustive matches and TUI snapshots | High | Wide but shallow edit set | Run the full insta and clippy gates; the snapshot-impact memory |
| Engine walks near the 2 MiB debug stack | Low | SIGABRT in tests | Gate `--no-fail-fast`, grep for SIGABRT |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
