# MOD-71 - Mouse support: capture policy and the Runs flow view (done, 2026-10-03)

**Requirements:** `R-TUI-1` (keyboard driven, mouse optional), `R-TUI-4` (Runs sub-tab steps and
actions).
**Origin:** MOD-28 (`docs/decisions/mod/mod-28.md` "Carried"), from ANA-12 §"Route events
conditionally based on focus". Raised by the maintainer at MOD-28's plan gate, 2026-10-02.
**Artifacts:**
- plan [`.claude/plans/mod-71-mouse-support.plan.md`](../../../.claude/plans/mod-71-mouse-support.plan.md): D1-D11, with its verified-claims table;
- blueprint `.claude/plans/mod-71-mouse-support.blueprint.md`: deviations B-1-B-12, hazards H-1-H-14, decisions E1-E16.

Decision numbers are local to MOD-71 (the MOD-31 convention).

Routed as **plan**: C2 fired (a new event seam on `Tab`/`DetailTab`) and C3 was borderline (the
policy was still to be decided, but the item text proposed one). It was a low-confidence call at
the threshold, and the maintainer accepted it. Run in a TOOL-7 sandbox (`hr/MOD-71`). The tasks
were one dependency chain, so they ran serially with a single implementer.

**Decisions (maintainer, 2026-10-03):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked;
- review: H1, M1, L1, L2, L4 and the source-shape nit applied. L3 and two nits are deferred to
  **MOD-74** (see Carried).

**Commits:**
- plan and blueprint: `c3243310`; `cf3c5fb3`, `3c6f4a05`, `e3ab5331`, `6a4cf651`, `d10d40b3`;
- T1 capture lifetime: `38db5ab4` (red), `2d0350e1`;
- T2 event seam: `a3666925` (red), `7c23270e`;
- T3 flow gestures: `d56b8bfa` (red), `e14d2354`, `bb3ae40a`;
- review fixes: `2069c320` (H1 + L1), `a4408718` (M1), `d8f8edf6` (L2), `40e3ad67` (L4).

---

## What was built

In Backlog › Runs, the flow view (`v`) now takes the mouse:

- **click** a node: the shared cursor moves to that step, exactly as `J`/`K` would. Every run
  action and permission digit then acts on it (ANA-12 invariant 2);
- **drag** on empty canvas (or on an edge): pan;
- **wheel**: zoom at the pointer, by the same 1.2 step and within the same 0.5-2.0 range as
  `+`/`-`.

Nodes stay read-only (MOD-28 D9). A press on a node is a click wherever it is released. rataflow
ignores a drag that starts on a non-draggable node, so panning starts from empty canvas. A pan or a
wheel zoom survives the active-run poll and every `RunStream` re-read: the view only scrolls back to
the cursor when the cursor moves, the pane is resized, or `+`/`-`/`=` is pressed.

Everywhere else htui stays keyboard-only and the terminal keeps its own text selection.

**Operator note.** While the flow view is shown, the terminal's bypass modifier still selects
text: Shift in xterm, GNOME Terminal, kitty and Windows Terminal; Option in iTerm2 and
Terminal.app. A terminal without mouse reporting runs the flow keyboard-only.

## Decisions as built (plan D1-D11)

- **D1, capture only while a view wants it.** `App::wants_mouse()` is true only when all of these
  hold:
  - no overlay is open, and the `?` box is closed (E7: the box is not in `OverlayStack`);
  - the active tab wants the mouse. For Backlog that means no filter form, no item form, and the
    active sub-tab wants it;
  - for `RunsTab`, all of: `View::Flow`, `Mode::Browse`, an item, and a cursor run with at least
    one step (review L2).

  Always-on capture and a toggle key were rejected.
- **D2, the event loop owns the toggle.** `TerminalGuard` has a `mouse` flag and
  `set_mouse_capture(on)`, which writes only on a change. The loop calls
  `set_mouse_capture(app.wants_mouse())` once per step, after the `$EDITOR` post-step and before the
  dirty draw; a source-shape test in `event_loop.rs` pins where. Each toggle is its own `execute!`,
  `Unsupported` tolerated (E9: recorded as done, logged at `debug`).
- **D3, every give-back turns it off.** `restore_terminal` (the panic hook and the guard) disables
  capture first, unconditionally and best effort. `Suspend::leave` disables it only when the guard
  had turned it on, and forgets it only after the write succeeds (review H1, L1). `init` and
  `Suspend::enter` never enable it; the loop re-asserts after the editor. A source-shape test beside
  the bracketed-paste one pins all of it (E10).
- **D4, the route.** `Event::Mouse` → `App::on_mouse` → `Tab::on_mouse` → Backlog →
  `DetailRegistry` → `RunsTab::on_mouse` → `ExecutionGraph::on_mouse` →
  `Flow::handle_mouse_event`. App-level handling, in order:
  - an event arriving while capture is not wanted is dropped;
  - `Moved`, `ScrollLeft` and `ScrollRight` are dropped before anything else, because crossterm
    enables any-motion reporting (`?1003h`);
  - `dirty` is set only on `Consumed`, and the status line is restored unless consumed (E8).

  `wants_mouse`/`on_mouse` are defaulted on both traits, so no other tab changed. Overlays never
  see a mouse event.
- **D5, gestures.** `RunsTab` records the canvas `Rect` rataflow actually drew (`drawable`, E6).
  Forwarding:
  - a left press and the wheel are forwarded inside the canvas;
  - drag and release are forwarded while a gesture that began inside is live;
  - right and middle buttons are filtered at the pane and again at the graph (E2).

  rataflow converts terminal coordinates itself from its render context.
- **D6, click selects; the flow keeps no selection of its own.** `NodeClicked` → `StepId` (E3) →
  `select_step` in the cursor's run (E12) → `sync_graph`. The flow is built with
  `with_deselect_on_pane_click(false)`; edges were already unselectable. rataflow selects a pressed
  node on press, so `on_mouse` puts the selection back on the cursor after every event (E1, B-1).
- **D7, a pan survives a re-read.** A same-run `sync` reveals the cursor only when the cursor step
  changed. A resize is a change in the size of the **pane** `RunsTab::render` receives, not the
  canvas (review M1). Otherwise a head line coming or going (`Waiting`, a pending permission, a
  failure) would undo the pan.
- **D8, read-only.** The node builders are unchanged. Hidden handles are skipped by rataflow's hit
  test (`mouse.rs:421-424`; the plan's `handle_new_connection` citation was corrected by B-3), so a
  press there is a node click.
- **D9, wheel = keys.** Both zoom by 1.2 within 0.5-2.0; nothing is configurable.
- **D10, `rataflow`'s `crossterm` feature.** It enables `ratatui/crossterm`, which the workspace
  already had. `Cargo.lock` is unchanged.
- **D11, the gesture ends with the view.** `v`, an item change and a mode that captures input
  clear it (E11).
- **Review L4, quiet no-ops.** Every event still reaches rataflow, but the pane answers
  `Consumed` only for a click or a change of pan or zoom. A press, a drag from a node, a wheel tick
  at a clamp and a release after a pan pass, so the any-motion stream cannot force redraws. This
  supersedes blueprint E4 ("`Consumed` for every forwarded event").

## Gate

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 108 suites, 4080 passed, 0 failed, 30 ignored; no SIGABRT (on `40e3ad67`) |
| `Cargo.lock`, `crates/htui/tests/snapshots` | unchanged; 143 snapshots, the three `runs_flow_*` byte-identical |

No store, migration, `.sqlx` or pin change.

## Carried

- **MOD-74, mouse follow-ups** (deferred from the review by the maintainer, 2026-10-03):
  - **L3: a stale gesture after a capture loss above `RunsTab`.** An overlay, `?`, a form or a
    sub-tab switch turns capture off without reaching `RunsTab`. If the button is held across the
    off-and-on, the next drag continues the old pan anchor, and the view jumps once. The fix is a
    "capture lost" hook through `App` → `Tab` → `DetailTab`.
  - **Any-motion reporting.** `EnableMouseCapture` sets `?1003h`; nothing uses hover. A narrower
    `?1000h ?1002h ?1006h` command would stop the `Moved` stream at the source on ANSI terminals.
    The Windows legacy console goes through WinAPI and would keep crossterm's command.
  - **A re-read during a live pan.** The review-L2 anchor shifts the viewport, but rataflow's
    `Panning` recomputes from its `initial_viewport` on the next drag. The view jumps once,
    cosmetically.
- **MOD-16 (Windows verification)** should exercise capture on Windows Terminal and the legacy
  console, and an `$EDITOR` handoff before the flow view was ever opened (review H1's path).
