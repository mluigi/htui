# MOD-53 - Runtime tasks always send a terminal reply (done, 2026-09-29)

**Requirements:** `R-NF-3`, `R-TUI-8`.
**Origin:** MOD-7 milestone 2 review (a deferral, `docs/decisions/mod/mod-7.md`). Found 2026-09-26.
**Artifacts:** plan with its verified-claims table:
[`.claude/plans/mod-53-runtime-task-replies.plan.md`](../../../.claude/plans/mod-53-runtime-task-replies.plan.md).
No PRD: routed as a plan on 2026-09-29, 1 of C1-C4 fired (C3, the item named two fixes).
**Commits:** `4cd13a3` (the fix and its tests), then this write-up.

## The defect, as it really was

Every task the agent runtime spawns answers its request once, at the request's own address, and the
UI flag that waits for it clears only on that answer: `probing` in Settings > Agents on
`Failed probe_agents`, `probing` in Settings > Boxes on `BoxProbed` or `Failed probe_box`, a running
install or login on its terminal frame, a chat's pending start on `ChatFrame::Failed`. A panic inside
the task meant the answer never came, and `sweep_finished` forgot the panicked `JoinHandle` unread,
so the flag stayed set for the rest of the session.

The item missed half of it. `terminal::install_panic_hook_restoring` gives the terminal back for
every panic `htui_agent::excerpt::panic_is_contained` does not vouch for, and only excerpt provider
calls were vouched for. Tokio catches a task panic only after the hook has run, so a panicking probe
also left the event loop drawing into a terminal with no alternate screen and no raw mode: the
wedged-UI shape MOD-56 closed for providers.

## What shipped

- **`answering(name, task, answer)`** in `crates/htui/src/agent_worker.rs`. Every spawn goes through
  it. It polls the task inside `htui_agent::excerpt::contain` and `catch_unwind`; on an unwind it
  drops the task (its `ChildGuard`s and `ReprobeClaim`s go with it), logs the panic, and sends the
  task's last word at its stream's current address. An aborted task never reaches the catch, so a
  superseded preview and a shutdown still answer nothing.
- **`htui_agent::excerpt::contain(f)`**, public: the same thread-local window a provider call opens,
  restoring the previous value so the two nest either way round.
- **The last word per task:**

  | Task | Reply on panic |
  |---|---|
  | `run_probe` | `Failed { request: "probe_agents" }` |
  | `run_box_probe` for `ProbeBox` | `Failed { request: "probe_box" }` |
  | `run_box_probe` for the registration probe | `BoxProbed` with `box_failed` set |
  | `run_preview` | `Failed { request: PROMPT_PREVIEW }` |
  | `run_plan`, `run_install` | `Install(InstallFrame::Failed { manual: None })` |
  | `run_auth` | `Auth(AuthFrame::Failed)`, at the `AuthStart`'s address |
  | `run_chat` (both start paths) | `ChatFrame::Failed`, then `ChatFrame::Ended { Cancelled }`, at the stream's moved address |
  | `run_reprobe` | nothing; it answers no request, so containment only |

  The message is `the <task> task panicked: <payload>`.

## Decisions worth keeping

**At the task, not in `sweep_finished`.** The item offered both. The sweep runs only when the next
request arrives, so the flag would have stayed set until the user pressed something else, and by then
the hook had already torn the terminal down. Catching in the task answers at once and is the only
place the hook's question can be answered.

**A box probe's last word is `Failed`, not `BoxProbed`.** The task's own failures travel inside a
`BoxProbed` report, but the Boxes section renders no report text, only re-reads the list. `Failed
probe_box` is what its refusals already send: it clears `probing` and puts the sentence on the notice
line.

**The registration probe's last word is a report.** It answers at `UNSOLICITED` with origin `App`,
where the freshness gate drops a `Failed` unread; `App::observe_reply` renders a `BoxProbed` above the
gate, so `box_failed` is what reaches the status line.

**A chat ends `Failed`, then `Ended`**, as its own transport failures do. `Failed` alone clears a
pending start but leaves an accepted session looking live, so the tab would go on sending to a chat
the runtime had already swept (review finding 1).

**`run_auth` answers at the start's address.** The flow moves its stream to an `AuthChoose`'s address
in a local rather than in `Frames`, and the `AuthStart`'s `seq` stays fresh for the whole flow because
a choose is another request kind, so the section still receives the frame.

## Tests

In `agent_worker.rs` (MOD-53 section): the wrapper answers at a moved address with the panic text and
holds the terminal-restore decision off during the poll; a clean exit sends nothing; a `ProbeBox`
whose hardware read panics answers `Failed probe_box` and frees the slot for the next one; a chat
whose driver panics on start ends its stream with `Failed` then `Ended`; a registration probe that
panics reports `box_failed`. In `excerpt.rs`: `contain`
opens, nests and closes the window, unwind included. The two runtime cases fail with the catch
switched off.

## Review

The configured `rust-reviewer` pass found one medium and four low findings. Applied: the chat's
`Ended` frame (finding 1), the registration probe's report (3), and the `ProbeBox` test now waits for
the task to finish on its own instead of `finish_background`, which takes the slot itself (5).
Accepted as stated below: 2 and 4.

## Not done

- A chat that panics mid-turn leaves its run row open: closing it is recovery work (MOD-24's area).
  A command already queued to the dead chat (a cancel, a follow-up) is dropped unanswered.
- A panic on a `spawn_blocking` thread a task starts (the install's unpack, the hardware read) is
  outside the window. The task still answers, because tokio hands it back as a `JoinError`, but the
  hook on that thread is not vouched for and still gives the terminal back. Provider threads are
  contained by `run_providers` itself. Tracked as **MOD-65**.
- A task that panics **after** it already sent its terminal reply (a `BoxProbed`, an install's
  `Done`) sends a second one. Every handler just goes idle again, but the status line names a
  request that had succeeded.
- A `Drop` that panics during the unwind aborts the process, as it always did; the hook now skips
  the terminal restore on the first panic because the window is open, so that abort leaves the
  terminal raw.
