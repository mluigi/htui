# MOD-87 - Chat cancel answered late or dropped during an unparked turn (done, 2026-10-07)

**Requirements:** `R-TUI-6`.
**Origin:** MOD-55 (`docs/decisions/mod/mod-55.md`), chat-path problems filed on the way.
**Artifacts:** shared with MOD-86 (`docs/decisions/mod/mod-86.md`), which ran in the same sandbox run:
- plan [`.claude/plans/mod-86-87-chat-scrub-cancel.plan.md`](../../../.claude/plans/mod-86-87-chat-scrub-cancel.plan.md):
  D1-D10, verified-claims table;
- blueprint `.claude/plans/mod-86-87-chat-scrub-cancel.blueprint.md`: amendments A-1..A-6 (accepted), hazards H-1..H-10.

Decision numbers are local to the MOD-86/MOD-87 plan.

Routed as **plan** (no criterion fired). Run in a TOOL-7 sandbox on `hr/MOD-86` together with MOD-86, at the
maintainer's request, because both change `run_chat`. Tasks ran serially, since every task touched
`crates/htui/src/agent_worker.rs`.

## The problem

`run_turn` read the chat's command channel only while a permission request was parked. So a `ChatCancel` (`Esc Esc`)
sent while a chat streamed was answered only after the turn's `Done`, and the agent spent the whole turn. A second
cancel queued behind a cancelled turn was dropped with the receiver, unanswered, which broke `ChatCommand`'s
answered-exactly-once contract. MOD-55 had fixed both for help turns only.

## What was built

- **D1 - a chat listens while it pulls.** `run_turn` serves commands in every mode, through a `biased` select with
  commands first. Its `listen` flag became `help`, plus a `deferred` queue. `help_command` became `turn_command`:
  - a cancel cuts the turn: the run closes `cancelled` and the cancel is answered `Ended{Cancelled}`;
  - a channel the runtime let go of ends the turn the same way, with no answer;
  - an answer with nothing parked is refused, and the turn goes on;
  - a chat's mid-turn `Send` is **deferred**, not refused, so the tab's type-ahead still reaches the agent after the
    turn. A help's send is refused as before.
- **D2 / A-1 - between turns.** The wait between turns is an inner loop. It takes deferred sends first, in FIFO
  order, then the channel, and only leaves once a follow-up was sent or the chat ends. This fixed a defect that was
  already there: an `Answer` refused between turns used to fall through into `run_turn` with nothing sent. That
  ended the chat `agent session is closed` on the fake, and would hang it on a real transport. MOD-86's refused
  follow-up would have hit the same path.
- **D3 - answered exactly once at the end.** `answer_after_help` became `answer_queued`. It closes the receiver, then
  answers the deferred sends and the queued commands, each once at its own address. A conversation runs it **last**,
  after `binding.close` and `frames.ended`. `live_steps` and `bind_promoted`'s `chat_open` guard (D206) read "receiver
  closed" as "the task returned", so closing earlier would let a promotion bind while the chat still writes rows. Help
  keeps its MOD-55 placement.
- **D4 - promotions.** A promoted chat is a conversation too, so a mid-turn cancel now cuts its turn. The step's
  status stays the engine's, and nothing else about promotions changed.

Tests: 7 new runtime tests, each red on the old code. They cover a cancel mid-stream, a second cancel behind a
cancelled turn, a follow-up sent mid-turn, an answer mid-turn with nothing parked, a promoted chat cancelled mid-turn,
`live_steps` until the run is closed, and A-1. R1 added a test pinning that queued mid-turn sends are answered at the
end, in FIFO order (review M-1). The fixture gained `StallAt::AfterChunkUntilReleased`.

**Test-helper change (T1).** About 25 tests ended a chat by queueing a `ChatCancel` before the task was polled, which
relied on the late cancel. `run`, `attach_and_end`, the `live_steps` test and the secrets module's `end` now go through
`converse` / `end_after_turn`. These serve the cancel after the turn's `Done` frame, and count every `Done` in a drained
batch. The hosted `end` waits for the step's `done` row instead. `a_cancel_queued_at_the_acceptance_cancels_the_help`
keeps the cancel-first shape through `run_with_cancel_queued` (A-2). T1 was green on the unchanged runtime first.

## Behaviour changes

- `Esc Esc` during a streaming chat ends it at once: the run is `cancelled`, not `done` after the turn.
- On shutdown, a chat that is mid-turn closes `cancelled` within the grace period instead of finishing its turn (H-6).

## Commits

`0e59d640` (T1 test helpers), `9589e34d` (T2), R1 review fixes `db319529` (shared with MOD-86).

## Known limits

- **L-1 (review, accepted).** No deterministic test pins the conversation's drain placement (D3/D206). On MemStore
  the chat task finishes in one poll. The comments at `run_chat`'s tail and on `answer_queued` are the guard.
- **N-4.** The deferred queue is unbounded, like the command channel that feeds it. That is fine for user-typed input.
- **L-4.** `run_chat` is about 440 lines. Extracting the between-turns wait (`next_follow_up`) is optional cleanup.
