# MOD-70 — Follow-up command rows for engine steps

> Routed as **PRD** by `/handoff-run MOD-70` (criteria C2, C3 and C4 fired; C4 low-confidence, the
> item text names no breadth). Accepted by the maintainer 2026-10-06, sandbox run `hr/MOD-70`.
> Ultracode accepted for the implement phase (architect and reviewer stay plain agents).
> Origin: MOD-42 PRD Q9 (`.claude/prds/mod-42-permission-relay.prd.md:118,150`,
> `docs/decisions/mod/mod-42.md` "Left open"). Requirements: `R-AGT-1`, `R-HIS-1`; constrained by
> `R-SEC-3`, `R-ID-7`. PRD gate: maintainer accepted the recommended answers to Q1-Q6, 2026-10-06.

## Problem

A user watching an engine-walked step in the Runs tab cannot steer it. `R-AGT-1` gives every driver
"send a follow-up" and `R-HIS-1` stores "every follow-up", but only chat sends one: the engine has
no follow-up verb, and a walk's session ends at its first `done`, before the step parks, so there is
no state in which a live engine session can take more input. The only way in today is `p` promote,
which preempts the walk and hands the step to a chat, and promote is refused outright on a step a
worker walks (MOD-42 OQ-3). The text would also be typed on one box and applied on another, while
`R-ID-7` wants it scrubbed on the executing box.

## Evidence

Read at `94c434de` (branch `hr/MOD-70`). Paths relative to the repo root.

- **Assumption — needs validation via prototype.** No user report or run exists that wanted
  steering; the need is MOD-42's deferral of PRD Q9 and the unmet `R-AGT-1`/`R-HIS-1` "must".
- **The walk drops the session at the first `done`.** `drive` returns on `Done`
  (`crates/htui-agent/src/record/relay.rs:270-272`); `drive_once` owns the session as a local and
  drops it on return (`crates/htui-orch/src/engine.rs:6114,6149,6178`); verify, capture, settle,
  `finish_step` and `gate::apply` all run after (`engine.rs:3587-3666`). The only wait with a live
  session is a parked permission request (`relay.rs:347-420`).
- **The drivers already take a follow-up, between turns only.** `AgentSession::send_follow_up`
  (`crates/htui-agent/src/driver.rs:438-442`, one `Done` per turn, `:428-430`);
  `follow_up_in_session = true` for ACP and CLI (`registry.rs:164,185`); a mid-turn follow-up is a
  `Transport` error on every driver (`acp/mod.rs:544-547`, `cli/mod.rs:631-634`, `fake.rs:551-556`).
  Production engine code never calls it.
- **Chat records a follow-up already.** `record_follow_up` bumps `turn`, scrubs, writes
  `follow_up` or a `scrub_residue` row (`crates/htui-agent/src/record.rs:772-797`); chat's typing box
  is its executing box.
- **`run_command` cannot carry one yet.** `CHECK kind IN ('cancel')`, no step id, no text, one
  pending row per (run, kind) (`crates/htui-store/migrations/0011_permission_relay.sql:47-60`);
  `RunCommandKind` says "A follow-up command adds a kind later (PRD Q9)"
  (`crates/htui-store/src/model/relay.rs:27-32`). The lease holder applies commands from
  `poll_commands_with` every 1 s (`crates/htui-worker/src/runtime.rs:1254-1300`).
- **Masks exist only where secrets resolve.** The engine scrubber is pattern rules only today
  (`runtime.rs:892,969`); MOD-10 M3 builds masks from the run's resolved map on the executing box.
  No non-executing box scrubs anything.

## Users

- **Primary**: the maintainer (or any user at a TUI) watching a graph run's `running` step in the
  Runs tab, walked in process, by a worker on the same box, or on another box, who sees the agent
  heading the wrong way and wants to add an instruction without abandoning the engine's walk.
- **Not for**: steps that are parked, failed or done (promote to chat covers them); judge sessions;
  chat sessions (they already take follow-ups).

## Hypothesis

We believe **a follow-up typed in any TUI, queued as a `run_command` row and sent by the walking
process into the step's session at its next turn end** will **let a user steer an engine step
without promoting it** for **users watching engine-walked steps on any box**.
We'll know we're right when **a follow-up typed on one box for a step walked on another produces a
scrubbed `follow_up` row at `turn + 1`, the agent's next turn, and then the step's normal gate, with
no step status added and no plaintext copy left in `run_command`**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Follow-up applied across executors | in-process, same-box worker and other-box worker all apply it | store conformance + engine/worker tests over MemStore and PgStore |
| Sender always learns the outcome | every row resolves `applied` or `refused` with a reason; none left `pending` after its step ends | tests on the session-end race and on cancel |
| No unscrubbed persistence | text column null on every resolved row; transcript row scrubbed with the executor's masks; pattern residue refused before any write | tests + a Postgres probe of `run_command` after resolution |
| No new step state | `run_step.status` CHECK unchanged; ANA-2 gains an amendment, not a row | migration diff, ANA-2 diff |

## Scope

**MVP** — a `follow_up` `run_command` kind targeting one step, applied by the walking process at
the step's next turn end; executor-side scrub and recording; typing-side pattern refusal; the
Runs-tab input and its display; in-process and worker walks, any box.

Decisions taken at the PRD gate (maintainer, 2026-10-06, recommended answers):

- **Q1 — the accepting state is `running`, between turns.** When the turn reaches `Done`, the walk
  checks for a pending follow-up for its step before releasing the session; if one exists it sends
  it, records `follow_up` at `turn + 1`, and drives to the next `Done`, repeating until none is
  pending; then verify and the gate run as today. No new step status; ANA-2 §4.8's "first path" is
  amended, not its state table. Rejected: interrupting a turn (riskier steering); holding the session
  open after `done` on a timer (idle slot and lease).
- **Q2 — targets.** Only the main agent session of a `running` step, addressed by step (fan-out
  candidates included). Judge sessions excluded. Any other step is refused with a reason; a parked or
  failed step's reason points at `p` promote.
- **Q3 — scrubbing split.** The typing box refuses (fail-closed, nothing written, reason shown) a
  text that still carries a pattern-rule residue. The executing box scrubs with its full masks before
  writing `session_event`. The row's text is nulled when the row resolves (`applied` or `refused`),
  so the only lasting copy is the scrubbed transcript row. The agent receives the text as typed, as
  chat does. Rejected: payload encryption (out of scope).
- **Q4 — one pending follow-up per step.** A second is refused "a follow-up is already queued". A
  cancel of the run refuses its pending follow-ups.
- **Q5 — the session-end race.** A row that misses the walk's last check is refused by
  compare-and-set with "the step finished its session; promote it to continue". No row outlives its
  step `pending`.
- **Q6 — TUI.** `f` on a `running` step in the Runs tab opens a one-line input; the relay view shows
  the queued follow-up and its resolution. No chat-view entry point.

**Out of scope**
- Mid-turn interrupt / steering (cancel the turn, then send) — riskier; revisit if the
  turn-end delay proves too slow.
- Follow-ups on parked, failed or done steps — promote to chat already covers them.
- Lifting OQ-3 (promote of a worker-walked step) — MOD-42 kept it; separate decision.
- `LISTEN`/`NOTIFY` wake-ups — MOD-43/MOD-46 own them; the 1 s poll stands.
- The agent question tool — MOD-75 owns its own follow-up turn design.
- Folding chat's `run_turn` onto `drive` — MOD-42 D7, a possible CLEAN item.
- Encrypting `run_command` payloads.

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Follow-up row and engine verb | A follow-up row queued for a `running` step (any executor, any box) is applied at the next turn end, recorded scrubbed at `turn + 1`, and resolved `applied`/`refused` with its text cleared (proven by store conformance and engine/worker tests) | pending | — |
| 2 | Runs-tab follow-up | The user types a follow-up with `f` on a running step in any TUI, sees it queued and how it resolved; docs updated | pending | — |

## Open Questions

- [x] Q7 — A follow-up queued while the step's permission request is parked waits for the turn's
  end; should the relay view say "queued until the turn ends" explicitly? **Yes** (maintainer,
  2026-10-06): a queued follow-up always reads "queued — sent when the current turn ends", parked or
  not, since a parked turn cannot reach `Done` until its request is answered.
- [x] Q8 — Does a follow-up turn count toward any step budget or timeout? **Moot, closed**
  (maintainer, 2026-10-06): no step timeout or usage cap exists; `token_budget` is the prompt
  assembly budget (`docs/ANA-2.md:284` chain, `crates/htui-orch/src/graph.rs:750`). A follow-up
  turn's usage is recorded like any turn's and nothing limits it.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Plaintext text sits in `run_command` until applied | Medium | Medium | Typing-side pattern refusal; text nulled at resolution; pending window bounded by the step's turn |
| A follow-up turn after verify-relevant work changes the artifact unexpectedly | Low | Medium | Follow-ups apply only before verify/capture/gate; the gate sees the final state |
| Race between the last check and session drop leaves a row `pending` | Medium | Medium | Compare-and-set refusal on step end (Q5); a test pins it |
| `htui-orch` walk-path future grows past the 2 MiB test stack | Medium | High | Box the new loop as MOD-42 did; gate with `--no-fail-fast`, grep SIGABRT |
| A cancel and a follow-up interleave | Low | Medium | Cancel refuses pending follow-ups (Q4); conformance case |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
