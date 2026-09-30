# MOD-42 — Permission and control relay through Postgres

> Routed as **PRD** by `/handoff-run MOD-42` (criteria C2 and C4 fired — at threshold, low-confidence
> verdict; accepted by the maintainer 2026-10-01, sandbox run `hr/MOD-42`). Ultracode accepted for
> the implement and review phases. Origin: `docs/ANA-16.md` §5.5, §8 item 3, §9 risk 2.
> Requirements: `R-AGT-1`, `R-HIS-1`, `R-TUI-6`. Consumers: MOD-41 (done), MOD-43, MOD-47 (its row
> protocol is the payload O5a later pushes).

## Problem

Every engine-driven ACP step fails at the agent's first permission request, whether the step runs
in the TUI's in-process engine or in `htui worker`. The only code that answers a permission is the
interactive chat loop in the `htui` binary, over an in-process channel; the engine has no answer
source, no durable record of a pending request, and never evaluates the agent's own permission
policy. Cancel has the same shape: a live walk on a `worker` box cannot be cancelled at all, and an
in-process cancel drops the agent with no grace and no answer to parked requests. Until this is
fixed the engine and the worker can only run ACP agents that never ask, and every remote-execution
item after MOD-41 (MOD-43, MOD-45, MOD-47) inherits the gap.

## Evidence

Read at `de177dd` (branch `hr/MOD-42`). Paths relative to the repo root.

- **Engine path fails structurally.** `drive_once` builds the step spec with
  `PermissionPolicy::default()` (Ask) and returns `pump(...)`
  (`crates/htui-orch/src/engine.rs:5630-5667`, policy at `:5660`). `pump`
  (`crates/htui-agent/src/record.rs:1854-1873`) records `permission_request` and pulls again;
  `next_event` on a parked ACP session returns `Transport("… is parked …")`
  (`crates/htui-agent/src/acp/mod.rs:507-510`); `gate::settle` maps it to `StepFailure::Driver`
  (`crates/htui-orch/src/gate.rs:240-243`). No `permission_answer` is written; dropping the session
  answers the agent `Cancelled` (`acp/mod.rs:1252-1262`, inference).
- **Stage 1–2 policy is never evaluated on the engine path.** The chat path evaluates it inside
  `run_turn` (`crates/htui/src/agent_worker.rs:3927-3951`); `pump` does not.
- **The only answering loop is out of reach.** `run_turn` (`agent_worker.rs:3814`) lives in the
  `htui` binary, which `htui-orch` cannot depend on; answers travel `ChatAnswer` → in-process mpsc
  (`crates/htui/src/ui/tabs/chat/mod.rs:316-331`, `agent_worker.rs:1785-1800`).
  `record.rs:1878-1884` already regrets the two loops.
- **Worker cancel is refused by design, pending this item.** `crates/htui-worker/src/runtime.rs:95-101`,
  `:2019-2023` ("cancelling a live run needs MOD-42's cancel command"); pinned by
  `crates/htui/src/run_worker.rs:2744-2777`; `crates/htui/src/cli.rs:75` names MOD-42 as the fix.
- **In-process cancel has no grace.** Token preempt drops the walk (`runtime.rs:556-562`,
  `:1381-1385`); ANA-2's `CancelRun` specifies driver `cancel(grace)` (`docs/ANA-2.md:1610`). This
  is R-38, currently carried under MOD-37 (`HANDOFF.md:189-191`).
- **No durable channel exists.** `session_event` already admits `permission_request` /
  `permission_answer` kinds, is append-only, keyed `(run_step_id, seq)` with `seq` allocated by the
  single in-process recorder, and every append is lease-fenced
  (`crates/htui-store/migrations/0001_init.sql:513-531,569-570`; `record.rs:419,956-957`;
  `crates/htui-core/src/store/traits.rs:2155-2182`). There is no command/answer table
  (`command_run` is the build/test queue). No `LISTEN`/`NOTIFY` anywhere.
- **Cross-box is closed.** `take_lease` requires `executing_box_id = $2`
  (`crates/htui-store/src/pg/write.rs:4052`); ANA-16 forbids answering through it.
- **Building blocks exist.** `AgentSession::{answer_permission, cancel, send_follow_up}`
  (`crates/htui-agent/src/driver.rs:419-438`); `Recorder::record_permission_answer` with
  `AnsweredBy {User, Policy}` (`record.rs:207-231,803-833`). CLI adapters never park
  (`crates/htui-agent/src/registry.rs:179`).
- **HANDOFF text corrections** (to be applied at close-out): line refs are stale (`pump` now
  `record.rs:1854-1873`, `drive_once` `engine.rs:5630-5667`); the worker does not "wait" today, it
  fails like the in-process engine; MOD-41's hand-back is lease release + `adopt_runs`, not a
  command row (`pg/write.rs:4035-4081`).

## Users

- **Primary**: the single htui user (`R-USR-1`) running graph runs — executed by the TUI's
  in-process engine or by `htui worker` — who must answer an agent's permission request or cancel a
  live step, from the executing box or from another box's TUI.
- **Not for**: multiple users with distinct authority over answers (`R-USR-3`, MOD-47); CLI
  (non-ACP) agents, which never park; interactive chat sessions, whose in-process path already works.

## Hypothesis

We believe **a durable permission-and-cancel relay through Postgres** will **let engine-driven ACP
steps complete instead of failing at their first permission request, and let live steps be cancelled
gracefully** for **the single user, on either executor, answering from the executing box or
another**.
We'll know we're right when **no engine step fails with "is parked"; a permission answered from a
second store client (different box id) resumes the step on both executors; and cancelling a live
worker walk reaches a terminal state within the grace window plus one poll interval.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Engine ACP steps failing on a permission request | 0 | Fake ACP agent requesting permission in an engine step, in-process and worker executors (currently fails) |
| Answer → step resumes | ≤ ~2 s with a 1 s poll | Integration test timing (MemStore and Postgres) |
| Cross-box answer | resumes the step | Postgres test: two `PgStore` clients with different box ids as executor and answerer |
| Live worker-walk cancel | terminal within grace + one poll | Test replacing the pinned refusal (`run_worker.rs:2744-2777`) |
| Parked requests at cancel | each answered `cancelled`, `permission_answer {option_id:null, by:"policy", cancelled:true}` recorded | ANA-4 §510-515 conformance test |
| Stale / late answers | never applied; persisted as refused | Test: answer after adoption / attempt change |

## Scope

**MVP**

1. **Policy first.** The engine path evaluates the agent's settings permission policy (stages 1–2)
   instead of `PermissionPolicy::default()`; only requests that still need a human become durable
   pending requests.
2. **Durable pending request + relay answer.** The executor records a pending request durably and
   waits (no timeout) until it is answered or cancelled; any store client may answer it; the
   executor applies the answer to the live session and itself records the `permission_answer`
   event (answerers never write `session_event` — single-writer `seq` and the lease fence stand).
3. **Stale-answer safety.** An answer applies only while the executor holds the lease and only to
   the live session attempt (request ids are scoped per attempt; JSON-RPC ids repeat across
   respawns). Pending requests of a dead attempt become stale; a late answer is persisted as
   refused, never silently dropped (ANA-2 inv. 7). First answer wins.
4. **Runs-pane answering.** Any TUI holding the DSN shows a step's pending request in the Runs pane
   and answers it with the same option keys as chat (`R-TUI-6`). "Awaiting permission" is derived
   from the pending request, not a new step status.
5. **Durable graceful cancel.** Every cancel — in-process or worker, local or from another box —
   is a durable command applied by the executor: parked requests answered `cancelled`, then
   `session.cancel` with ANA-4 §4.3's grace, then hard drop. Replaces the worker refusal and
   absorbs MOD-37's R-38. A local executor may additionally get an in-process nudge.
6. **Poll, not notify.** The executor polls for an answer/cancel at ~1 s while a request is parked
   or a walk is live; the general worker poll (5 s) is unchanged.

**Out of scope**

- **Follow-up on engine steps** — the engine has no follow-up verb and ANA-2 has no state for it
  (`ANA-2.md:1235-1238`); filed as a new MOD item after this lands. The command shape leaves room.
- **`LISTEN`/`NOTIFY` wake-ups** — MOD-43 / MOD-46.
- **Gate approval and hand-back as command rows** — both work today (ANA-16 §5.5 listed them;
  deferred without loss).
- **Remote-box targeting / dispatch** — MOD-43. A real two-machine run — MOD-43/MOD-45.
- **Who-may-answer authorisation** — `R-USR-3`, MOD-47.
- **Resuming a parked request after the executor dies** — responders are not durable
  (`ANA-2.md:1313`); reset-and-retry stands.
- **Persisting "allow always" grants** into the agent's settings row — forwarded to the agent only
  (`upsert_agent` is last-writer-wins until C6).

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Relay core | Engine ACP steps evaluate policy, park durably and resume on a relayed answer, on both executors and across box ids (proven by tests) | in-progress | `.claude/plans/mod-42-permission-relay.plan.md` (T0-T3) |
| 2 | Runs-pane answering | The user sees and answers a pending permission for an engine/worker step from any TUI (`R-TUI-6`) | in-progress | `.claude/plans/mod-42-permission-relay.plan.md` (T5) |
| 3 | Durable graceful cancel | Cancel of any live walk, from any box, answers parked requests `cancelled` and ends the step with grace; worker refusal removed; R-38 closed | in-progress | `.claude/plans/mod-42-permission-relay.plan.md` (T4, T6) |

## Open Questions

Resolved at the PRD gate (2026-10-01, maintainer accepted all recommended defaults):

- [x] Where answers live — a new relay table; executor echoes into `session_event`.
- [x] Stale answers — lease + attempt-scoped keys; stale/late answers persisted as refused.
- [x] Wait bound — none; step stays `running`; awaiting state derived.
- [x] Engine policy — agent's settings policy on the engine path.
- [x] Latency — ~1 s poll while parked/live; no NOTIFY.
- [x] R-38 — absorbed from MOD-37 (strike it there at close-out).
- [x] One cancel path — every cancel is a durable command.
- [x] "Allow always" — forwarded to the agent only.
- [x] Follow-up — out of scope; new MOD item.
- [x] Cross-box proof — two-`PgStore` integration test.

Still open (for `/plan`):

- [ ] Which crate owns the single shared turn loop (engine + chat) — `record.rs:1878-1884` rationale
      suggests `htui-agent`.
- [ ] How the worker observes commands for a live walk (per-walk poll vs sweep tick), given that
      preempt is process-local (`runtime.rs:556`).
- [ ] Trait blast radius: `WorkerStore`/`WorkerHost` have no default bodies
      (`crates/htui-core/src/store/worker.rs`); `MemStore`, `PgStore`, test spies all implement them.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Stale answer applied after respawn or adoption | Medium | High (wrong grant to a different turn) | Attempt-scoped keys; lease check at apply; CAS on the request row |
| Cancel's `cancelled` answer races a user answer | Medium | Medium | First-writer-wins CAS; the loser persisted as refused |
| Parked step holds its compute slot and lease indefinitely | High | Medium (a forgotten prompt blocks a box slot) | Runs-pane visibility; cancel always available; revisit a timeout if it bites |
| Relay writer disturbs the live lease | Low | High | Answerers never touch `run`/lease columns or `session_event` |
| Engine behaviour change: policy now auto-allows/denies on the engine path | Medium | Medium | Same evaluator as chat; tests for each policy stage |
| Trait surface growth ripples through MemStore/PgStore/spies | High | Low | Plan the trait change once; `change(verify)` before editing |
| Suite green is scheduling-dependent; integration tests need `testkit` | High | Low | Gates run with `--features testkit` and `--test-threads=1` |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
