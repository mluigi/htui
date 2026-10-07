# Plan: MOD-86 + MOD-87 — chat prompts scrubbed before sending; chat cancel answered mid-turn

**Source:** HANDOFF.md items MOD-86 and MOD-87 (both from MOD-55, `docs/decisions/mod/mod-55.md`).
**Routing:** plan path for both, run together in sandbox `hr/MOD-86` (maintainer, 2026-10-07).
**Complexity:** Medium. One runtime file carries almost all of it; the test-helper fallout from MOD-87 is the bulk.
**Status:** CONFIRMED 2026-10-07 (D5 as recommended: no UI change, composer text not restored). Implementation in progress.

## Open question for the maintainer (read first)

**MOD-86's open item: what does the Chat tab show for a refused follow-up in a live session?**
Recommendation (D5 below): **no new UI.** The refusal is `Failed { "chat_send", "not sent: the follow-up matches
the <rule> rule" }` at the send's own address, which the tab already renders on its hint line through its existing
`Failed { request: "chat_*" }` arm (`ui/tabs/chat/mod.rs:663-668`, shown while a session is live by `hint`). The
session stays live, nothing is recorded, no transcript frame is sent, and the composer's text is **not** restored
(the composer cleared it on `Enter`, `composer.rs:96-99`; the reply carries no text by contract, so restoring it
would mean the tab keeping a copy of a refused credential-shaped string). The user retypes without the secret.
Alternative: stash the last submitted text in the tab and restore it on a `chat_send` refusal. This is more
convenient, but the tab would then hold the refused text.

## Summary

- **MOD-86:** `run_chat` sends the chat's opening prompt and every follow-up to the driver unscrubbed. The scrubber
  runs only on the recorded row, and a refusal there is only logged. The opening's local `prompt` frame also reaches
  the tab unscrubbed (`agent_worker.rs:4750`).
  `R-ID-7` requires a scrub before anything is persisted **or transmitted**, failing closed. Fix: the opening is
  scrubbed in `AgentRuntime::start` before anything is minted, like MOD-55's help. `run_chat` scrubs it again with
  the provider-resolved scrubber before the driver starts. Every follow-up is scrubbed before `send_follow_up`. A
  refusal sends nothing and records nothing.
- **MOD-87:** `run_turn` reads `commands` only while a permission request is parked, so a chat `ChatCancel` is
  answered after the turn's `Done`. A second queued cancel is dropped with the receiver. Fix: MOD-55's help shape
  for `ChatMode::Conversation` too. Commands are served while pulling, a mid-turn `Send` is deferred (not refused,
  so the tab's type-ahead keeps working), and whatever is still queued when the chat ends is answered once. The
  receiver closes only after the run is closed (D206), which is the "keeping promotions' behaviour" clause.

## Design decisions

### MOD-87 (cancel)

- **D1 — listen always.** `run_turn` serves commands while pulling in every mode (`biased` select, commands first,
  as `agent_worker.rs:5025` does for help). The `listen: bool` parameter becomes `help: bool` and a new
  `deferred: &mut VecDeque<ChatCommand>` parameter is added. `help_command` generalizes to `turn_command(help, …)`:
  - `Cancel`: cancel the session (a failed graceful cancel is logged, not returned, as help does), drain, reply
    `Ended{Cancelled}`, return `TurnEnd::Cancelled`. The run closes `cancelled`.
  - `None` (the runtime let go): the same as `Cancel`, with no reply.
  - `Answer` with nothing parked: refused ``no permission request `<id>` is waiting``, and the turn goes on.
  - `Send`: in help, refused as today ("a help turn takes no follow-up"). In a chat, **pushed onto `deferred`** and
    served between turns in FIFO order, which is exactly what happens today when the tab sends mid-turn (the
    tab's `submit` only checks `ended`, `chat/mod.rs:264-275`).
  - The parked path is unchanged (`Send` while parked stays "answer the permission request first").
- **D2 — between turns:** `run_chat` takes `deferred.pop_front()` before `commands.recv()`.
- **D3 — answered exactly once at the end.** `answer_after_help` becomes `answer_queued(deferred, commands, frames,
  last_stop)`. It closes the receiver, then answers `deferred` and then the queue: a cancel gets
  `Ended{last_stop}`, and a send or answer gets "this chat has ended". **Placement differs by mode on purpose:**
  - Help keeps its MOD-55 placement (before `recorder.finish`) so its pinned frame order is unchanged.
  - A conversation drains as the **last** statement of `run_chat`, after `binding.close` and `frames.ended`.
    `live_steps` (D206, `agent_worker.rs:954-960`) and `bind_promoted`'s `chat_open` guard read "receiver closed"
    as "the task has returned". Closing earlier would let a promotion bind while this chat still writes rows.
- **D4 — promoted chats get the same fix.** They are `ChatMode::Conversation` too: a mid-turn cancel closes the
  turn `cancelled`. `binding.close` stays a no-op for a graph step, and the step's status stays the engine's.
  Nothing else about promotions changes: the opening is still a `follow_up`, and the resume fallback and D185
  hand-over are unchanged.

### MOD-86 (scrub)

- **D5 — the refused follow-up** (open question above): `Failed { "chat_send", "not sent: the follow-up matches the
  <rule> rule" }` at the request's address. The session goes on, nothing is sent, recorded or framed, and the tab
  needs no change.
- **D6 — the opening, at `start`.** `Opening::Chat(text)` is scrubbed with `prompt::scrub_section(&scrubber, &text,
  "prompt")` where help's is scrubbed (`agent_worker.rs:2297-2320`), before the box, registry row, lease and run.
  The scrubber is `MinimalScrubber::from_resolved(&env)`, the chat's own construction (`run_chat:4603`), not help's
  `MinimalScrubber::new(env.values())`, so that `run_chat`'s second pass over the same env is deterministic.
  Refusal: `Served::Reply(Failed { "chat_start", "not sent: the prompt matches the <rule> rule" })`, with no run and
  no driver start. The masked text replaces `prompt`, so the driver, the `prompt` row and the tab's local `prompt`
  frame all carry the masked copy.
- **D7 — the opening, in `run_chat`.** The prelude is reordered so that, after the secrets resolve, the scrubber is
  built and the opening is scrubbed **before** the deferred `start_chat_run`. A provider project's resolved values
  are known only there (MOD-10 D12/D13).
  - **Fresh provider chat:** a refusal answers like D13's refusal arm. `to_stream(Failed{mode.request})` and
    `frames.failed` are sent, and no run is written.
  - **Fresh non-provider chat:** `start` already passed the identical scrubber over the identical env, so a refusal
    here cannot happen. It is still handled: close the run `failed`.
  - **Promoted chat:** the opening (resume sentence or handoff) is scrubbed the same way. A refusal fails the start
    through the existing start-failure arm (`Failed{promote_step}`), and `binding.close` is a no-op.
  - **Fallback handoff** (`ResumeFallback.handoff`): scrubbed at its point of use, before the second
    `driver.start`. A refusal fails the chat through the same arm, the `resume_failed` notice first (H-7).
- **D8 — follow-ups.** In the between-turns `Send` arm, `scrub_section(&scrubber, &text, "follow-up")` runs before
  `send_follow_up`.
  - `Ok(masked)`: `masked` is sent, recorded, and returned as the reply frame.
  - `Err(refused)`: D5. The loop continues waiting on the user, with no status change.
- **D9 — section names.** `"prompt"`, `"follow-up"`, `"opening"` (promoted), `"handoff"` (fallback).
  `SectionRefused.section`'s doc comment lists them beside help's.
- **D10 — out of scope, noted:** help builds its scrubber with `MinimalScrubber::new(env.values())` (no masking
  floor, no `masked_forms`), while chats use `from_resolved`. Under `cfg(not(test))` `start`'s env is empty, so
  this has no production effect today. Recorded in the decision write-up, not changed.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Scrub before mint | `agent_worker.rs:2297-2320` (`start`, `Opening::Help`) | `scrub_section`, then a `Served::Reply(Failed{…, "not sent: {refused}"})`, with nothing written |
| Listen while pulling | `agent_worker.rs:5025-5037`, `help_command` (`:5104`) | `biased` select, commands first, the handler returns `Option<TurnEnd>` |
| Answer what is queued | `answer_after_help` (`:5162`) | close, then answer each at its own address, once |
| Refusal arm without a run | `run_chat` D13 arm (`:4575-4587`) | `to_stream(Failed)`, `failed`, return |
| Tests: stall mid-turn | `fixture_with_stall`, `StallAt::AfterChunk`, `cancel_at_stall` (`:6832`, `:6691`, `:6861`) | spawn the task, wait for `probe.reached`, serve `ChatCancel` at seq 8 |
| Tests: cancel after the turn | `tests/chat_live.rs:78-88` | serve the cancel once a `DriverEvent::Done` frame arrives |
| Tests: scrubbed send | `the_driver_is_sent_the_scrubbed_assembly` (`:6527`), `the_chat_scrubber_is_built_from_the_resolved_map` (`:15786`) | seeded `session_env` / provider fake, assert on what the fake driver received |
| Errors | `SectionRefused` (`htui-core/src/prompt/mod.rs:1331`) | names the section and rule, never the text |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui/src/agent_worker.rs` | UPDATE | `start` (D6), `run_chat` prelude and Send arm (D7, D8), `run_turn`/`turn_command`/`answer_queued` (D1-D3), `Opening::Chat` doc, tests and test helpers |
| `crates/htui-core/src/prompt/mod.rs` | UPDATE | `SectionRefused.section` doc lists the chat section names (D9). Doc only |
| `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE (test only) | pin D5: a `Failed{chat_send}` on a live session shows on the hint line and keeps the session live |
| `docs/decisions/mod/mod-86.md`, `docs/decisions/mod/mod-87.md` | CREATE | close-out write-ups (lifecycle P2) |
| `HANDOFF.md`, `DECISIONS.md` | UPDATE | close-out bookkeeping |

**Task independence:** none. T1-T4 all touch `crates/htui/src/agent_worker.rs` (the file sets intersect), so they
run **serially**, one implementer, committing after each task.

## Tasks

### Task 1: test helpers cancel after the turn, not before it (MOD-87 prep, behaviour-neutral)
- **Action:** every helper that queues a `ChatCancel` before the task is polled relies on today's "read after
  `Done`". Under D1 it would cut the turn. Rewrite those helpers to spawn the task, collect replies until a
  `DriverEvent::Done` frame arrives (or the task ends, for scripts that fail before `Done`), then serve the cancel
  and await the task. Affected: `run` (`:6020`, 15 callers), `attach_and_end` (`:7074`, 9 callers), the
  `live_steps` test (`~:7950`), `Live::end` in the hosted module (`~:8068`), and the secrets module's `end`
  (`~:15490`). Call sites do not change.
- **Validate:** `cargo test -p htui --all-features --lib agent_worker -- --test-threads=1` is green on the
  **unchanged** runtime. Commit.

### Task 2: MOD-87 — a chat listens while it pulls (D1-D4), tests first
- **Tests (red first):**
  - `a_cancel_mid_stream_with_nothing_parked_cancels_the_chat` (mirror the help one with a `ChatStart`):
    `Ended{Cancelled}` once at seq 8, the session cancelled, the run `cancelled`, the stream ending `cancelled`.
  - `a_second_cancel_behind_a_cancelled_turn_is_answered_once`: two cancels queued mid-turn. Each is answered once:
    the first `Ended{Cancelled}`, the second `Ended{Cancelled}` from `answer_queued`.
  - `a_follow_up_sent_mid_turn_is_sent_after_the_turn`: a `Send` during the stall is not refused. It reaches the
    driver after `Done` and is answered with its `follow_up` frame.
  - `an_answer_mid_turn_with_nothing_parked_is_refused_and_the_turn_goes_on`.
  - `a_promoted_chat_cancelled_mid_turn_leaves_the_step_to_the_engine` (D4).
  - `live_steps_keeps_a_chat_until_its_run_is_closed` (D3/D206): a command that arrives after the turn loop is
    still answered, and `live_steps` lists the step until the task returns.
  - Generalize `cancel_at_stall` over the request (help or chat).
- **Action:** D1-D4 in `run_turn`, `turn_command`, `run_chat`, and `answer_queued`. Update the doc comments that
  say "a chat passes `false`" and "A chat keeps its behaviour".
- **Validate:** the `--lib` gate above, plus `cargo clippy -p htui --all-features -- -D warnings`. Commit.

### Task 3: MOD-86 — scrub before sending (D5-D9), tests first
- **Tests (red first):**
  - `a_chat_prompt_with_a_credential_is_refused_before_anything_is_minted`: `Failed{"chat_start", "not sent: the
    prompt matches the <rule> rule"}`, no `tests::minted`, no driver start, and the reply never contains the
    text.
  - `the_chat_driver_is_sent_the_masked_prompt`: seeded `session_env`. The fake driver's opening, the `prompt` row
    and the tab's local `prompt` frame all carry `[REDACTED]`.
  - `a_provider_chat_masks_its_resolved_value_before_sending`: the provider fake (mirror `:15786`). A refusal
    there writes no run (D7).
  - `a_follow_up_is_masked_before_it_is_sent` and `a_refused_follow_up_is_not_sent_and_the_chat_goes_on`: the
    driver received nothing, there is no `follow_up` row, the next clean `Send` works, and the run closes `done`.
  - `a_promoted_opening_and_handoff_are_scrubbed` (D7).
  - Chat tab unit test for D5.
- **Action:** D6-D9. Update `Opening::Chat`'s doc ("sent as typed … out of scope here") and `SectionRefused`'s doc.
- **Validate:** the `--lib` gate, `cargo test -p htui-core --all-features prompt`, and clippy. Commit.

### Task 4: full gates
- **Validate:** see Validation. Fix any fallout, including tests outside `agent_worker` that end a chat by queueing
  a cancel up front: `cargo test --workspace --all-features --no-fail-fast` and grep for failures and `SIGABRT`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate (memory: htui-featureless-clippy-gate)
cargo test -p htui --all-features -- --test-threads=1        # keyring fake is process-wide
cargo test --workspace --all-features --no-fail-fast 2>&1 | tee /tmp/gate.log; grep -E 'FAILED|SIGABRT|panicked' /tmp/gate.log
cargo insta test --workspace --all-features --check          # expected no snapshot change (no render change)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Tests that queue a cancel before polling silently change meaning (the turn is cut, not finished) | High | Task 1 moves them first on the old runtime, so a later red is MOD-87's own. Task 4 runs the full suite `--no-fail-fast` |
| Closing the receiver early breaks D206 / the promotion guard | Medium | D3 places the conversation drain last, and a dedicated test pins it |
| `select!` drops an event | Low | Both transports' `next_event` is an mpsc `recv` (cancel-safe, the `run_turn` doc and MOD-55 H1). Same shape as help |
| Double scrub of the opening refuses differently in `start` and in `run_chat` | Low | Same constructor over the same env (D6). Masked text has no residue to refuse. The second-pass refusal is handled anyway |
| Promotion refused by a credential-shaped handoff | Low | Fail closed is `R-ID-7`'s rule. The sentence names section and rule, so the cause is visible |

## Acceptance

- [ ] A chat `ChatCancel` mid-stream ends the turn at once: the run is `cancelled`, `Ended{Cancelled}` is answered
  once, and no command is ever left unanswered.
- [ ] Mid-turn follow-ups are still delivered after the turn. Promotions are unchanged except that a cancel now cuts
  the turn.
- [ ] No chat text (opening, promoted opening, handoff, follow-up) reaches a driver or the tab unscrubbed, and every
  refusal fails closed with a section-and-rule sentence.
- [ ] All gates in Validation are green, and the validator is green.

## Verified claims

| claim | verdict | evidence |
|---|---|---|
| `run_turn` reads `commands` only while parked for a chat | ✓ | `agent_worker.rs:5025` `if listen { select … } else { next_event }`; `run_chat` passes `mode.is_help()` |
| The queued-commands drain runs for help only | ✓ | `agent_worker.rs:4895-4897` `if mode.is_help() { answer_after_help(…) }` |
| The chat opening is sent unscrubbed | ✓ | `:2297` `Opening::Chat(text) => (text, …)`; `run_chat` `driver.start(spec.clone(), prompt.clone())` |
| The opening's tab frame is unscrubbed | ✓ | `:4750` `frames.local("prompt", json!({ "text": opening_text }), now)` |
| A follow-up is sent before any scrub, and a record refusal is only logged | ✓ | `:4832` `send_follow_up(text.clone())`, then `record_follow_up`, whose `Err` is `tracing::error!` |
| Help scrubs before box/registry/lease/run | ✓ | `:2298-2320`, ahead of `backend.box_info()` |
| Help and chat build their scrubbers differently | ✓ | `:2303` `MinimalScrubber::new(env.values())` vs `:4603` `from_resolved(&spec.env)` |
| A provider chat's run is written after resolve, inside `run_chat` | ✓ | `:4590` `writer.start_chat_run(chat)` inside the `secrets` block; `start` writes only when `secrets.is_none()` |
| The tab can send `ChatSend` mid-turn | ✓ | `ui/tabs/chat/mod.rs:264-275` (only `ended` is checked) |
| The tab shows a `Failed{chat_*}` on its hint line while live | ✓ | `chat/mod.rs:663-668` sets `refusal`; `hint()` returns it when `session.is_some()` |
| The composer clears its text on submit | ✓ | `composer.rs:96-99` `std::mem::take(&mut self.text)` |
| `live_steps` / the promotion guard treat a closed receiver as a returned task | ✓ | `:954-960` doc and filter; `:1026` `bind_promoted` |
| Test helpers queue the cancel before polling | ✓ | `run` `:6020-6045` ("The cancel is queued **before** the future is polled"); `attach_and_end` `:7074`; `:7950`; `:8068`; `:15490` |
| `DriverEvent::Done` reaches the reply stream as a frame | ✓ | `tests/chat_live.rs:78-80` matches `Done` in `ChatFrame::Event` |
| A stall fixture exists to cancel mid-turn | ✓ | `fixture_with_stall` `:6832`, `StallAt` `:6691`, `cancel_at_stall` `:6861` (hard-codes a help request) |
| `scrub_section` exists and names section and rule only | ✓ | `htui-core/src/prompt/mod.rs:1311-1331` |
| Both transports' `next_event` is cancel-safe | doc-asserted | `run_turn` doc (`:4927-4929`), reviewed in MOD-55 H1; not re-probed |
| Tasks are independent | ✗ (serial) | T1-T4 all touch `crates/htui/src/agent_worker.rs` |
