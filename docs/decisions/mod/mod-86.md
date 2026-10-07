# MOD-86 - Scrub chat prompts before they are sent (done, 2026-10-07)

**Requirements:** `R-ID-7`, `R-SEC-3`.
**Origin:** MOD-55 (`docs/decisions/mod/mod-55.md`), chat-path problems filed on the way.
**Artifacts:** shared with MOD-87 (`docs/decisions/mod/mod-87.md`), which ran in the same sandbox run:
- plan [`.claude/plans/mod-86-87-chat-scrub-cancel.plan.md`](../../../.claude/plans/mod-86-87-chat-scrub-cancel.plan.md):
  D1-D10, verified-claims table;
- blueprint `.claude/plans/mod-86-87-chat-scrub-cancel.blueprint.md`: amendments A-1..A-6 (accepted), hazards H-1..H-10.

Decision numbers are local to the MOD-86/MOD-87 plan.

Routed as **plan** (C3 only: what the tab shows for a refused follow-up). Run in a TOOL-7 sandbox on `hr/MOD-86`,
serially after MOD-87's tasks, in the same file.

## The problem

The Chat tab's `run_chat` sent the opening prompt and every follow-up to the driver unscrubbed. The scrubber ran only
on the recorded row (`record_prompt`, `record_follow_up`), and a refusal there was only logged. The opening's local
`prompt` frame also reached the tab unscrubbed. `R-ID-7` requires a scrub before anything is persisted **or
transmitted**, failing closed.

## What was built

Every chat text is masked before a driver, a row or a tab frame sees it. Every refusal is
`not sent: the <section> matches the <rule> rule`, which names the section and the rule and never the text.

- **D6 - the opening, at `start`.** It is scrubbed with `prompt::scrub_section(…, "prompt")` where MOD-55's help is
  scrubbed: before the box, the registry row, the lease and the run. The scrubber is the chat's own
  `MinimalScrubber::from_resolved`. A refusal is `Failed{"chat_start", …}`, with no run and no driver start.
- **D7 - the opening, in `run_chat`.** The prelude is now: resolve, build the scrubber, scrub the opening, then the
  deferred `start_chat_run`. So a provider project's resolved values are masked too, and a refused provider chat
  writes no run. A refusal is handled per binding:
  - fresh provider chat: D13's refusal arm, with no run;
  - fresh non-provider chat: the run is closed `failed`. This cannot happen in practice, because `start` already
    passed the same scrubber;
  - promoted chat: the start-failure arm with `Failed{promote_step}`. The promoted opening is section `"opening"`.
  - The resume **fallback handoff** (section `"handoff"`) is scrubbed before the second start. A refusal fails the chat
    after the `resume_failed` notice (H-7).

  A private `StartFailure { Driver, Refused }` enum keeps a refusal apart from `DriverError`, so it trips neither
  `falls_back` nor the `Spawn` re-probe.
- **A-4 - help too.** `run_chat`'s opening scrub runs in every mode, so a help turn on a provider project now masks
  its resolved secrets before sending. Before, `start`'s env was empty in production.
- **D8 - follow-ups.** They are scrubbed (section `"follow-up"`) before `send_follow_up`. The masked text is what is
  sent, recorded and answered as the frame. Deferred mid-turn sends (MOD-87) go through the same arm.
- **D5 - the refused follow-up in a live session (the item's open question).** It gets `Failed{"chat_send", "not sent:
  the follow-up matches the <rule> rule"}` at the send's own address. The chat stays live, waiting on the user (MOD-87
  A-1). Nothing is sent, recorded or framed. **No UI change:** the tab's existing `Failed{chat_*}` arm shows it on the
  hint line. The composer's text is not restored, so the tab never holds the refused text. The maintainer chose this
  at CONFIRM.

Tests: 10 runtime tests (each red on the old code), the A-4 test
(`chat_secrets::a_provider_help_masks_its_resolved_value_before_sending`), and a chat-tab test pinning D5. R1 added a
test that a successful fallback handoff is sent and recorded masked (review L-2). The fake driver gained a send log
(`SendSpy` inside `FailingStarts`).

## Commits

`bd6bc7ce` (T3), R1 review fixes `db319529` (shared with MOD-87). Plan `baf0a8fd`, blueprint `75768529`.

## Known limits

- **D10 (noted, unchanged).** `start` builds help's scrubber with `MinimalScrubber::new(env.values())` (no masking
  floor, no `masked_forms`), while chats use `from_resolved`. A-4's second pass in `run_chat` uses `from_resolved` for
  help as well, so a provider help is masked either way.
- **H-4.** A refusal can go unseen if the user sends again before it arrives: the hint is cleared by the next key.
- **L-3 (review).** The non-provider refusal arm in `run_chat` cannot be reached and is untested.
