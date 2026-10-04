# MOD-74 - Mouse follow-ups: capture loss, button-only reporting, a pan across a re-read (done, 2026-10-03)

**Requirements:** `R-TUI-1` (keyboard driven, mouse optional), `R-TUI-4` (Runs sub-tab steps and
actions).
**Origin:** MOD-71 (`docs/decisions/mod/mod-71.md` "Carried"): review L3 and two nits, deferred
by the maintainer at MOD-71's review, 2026-10-03.
**Artifacts:**
- plan [`.claude/plans/mod-74-mouse-follow-ups.plan.md`](../../../.claude/plans/mod-74-mouse-follow-ups.plan.md):
  D1-D7, with its verified-claims table (25 claims, 3 amended, 1 unverified);
- blueprint `.claude/plans/mod-74-mouse-follow-ups.blueprint.md`: deviations B-1-B-7, hazards
  H-1-H-18, decisions E1-E6.

Decision numbers are local to MOD-74 (the MOD-31 convention).

Routed as **plan**: none of C1-C4 fired (one repo, no new seam, each fix already diagnosed, about
nine files). No ultracode. Run in a TOOL-7 sandbox (`hr/MOD-74`). The tasks shared files, so they
ran serially with a single implementer.

**Decisions (maintainer, 2026-10-03):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked;
- review (approve, 0 critical/high/medium): L1 (as a doc note), L2, L3 and N1-N5 applied; N6 needs
  no action.

**Commits:**
- plan and blueprint: `b983778b`, `2df1cae1`;
- T1 button-only reporting: `90cdd8f3` (red), `317ed999`;
- T2 capture-lost hook: `d7c298e4` (red), `efcccf6f`;
- T3 pan re-anchor: `02fbc32e` (red), `b4c2f83b`;
- review fixes: `0f0037ad` (L1, L2, N1-N5), `d9733ec5` (L3).

---

## What was built

Three gaps in the Runs flow view's mouse handling, each of which made the view jump once or cost
work nobody needed, are closed:

- **A lost capture ends the gesture.** An overlay, `?`, a form, a sub-tab or tab switch, or an
  `$EDITOR` handoff turns capture off. Every tab now hears of it, so a button held across the
  off-and-on no longer resumes the old pan anchor (MOD-71 review L3).
- **Button-only reporting.** On ANSI terminals capture no longer asks for any-motion reporting, so
  the terminal sends no hover stream.
- **A live pan survives a re-read.** A `Runs` re-read that shifts the layout, a head line that
  moves the canvas, and a wheel zoom during the pan no longer snap back on the next drag. The wheel
  case was not in the item: the plan's fact-check found it with the same cause and a bigger jump
  (offset `(-12.1, -4.1)` → `(1, 0)` in the probe).

rataflow is a registry crate and its drag state is `pub(crate)`, so every fix uses its public API:
`is_dragging`, `handle_mouse_event` and the public `locked` flag.

## Decisions as built (plan D1-D7)

- **D1, the app sees the capture edge.** `App` records the answer the loop last applied (`mouse`).
  `App::mouse_capture()` replaces `wants_mouse()` at the loop's single `set_mouse_capture` call
  (pinned by the shape test in `event_loop.rs`). On an on-to-off edge it calls `Tab::on_mouse_lost`
  on **every** registered tab, because after a tab switch the holder is no longer the active one. It
  carries `#[must_use]` (review L2).
- **D2, an `$EDITOR` handoff is a loss.** `take_external_edit` returns `Some` exactly when the loop
  is about to suspend, and `Suspend::leave` turns capture off without the app knowing. A capture that
  was on is lost there, and the next `mouse_capture()` re-enables it with no stale gesture.
- **D3, a defaulted hook down the chain.** `Tab::on_mouse_lost` and `DetailTab::on_mouse_lost` do
  nothing by default. Backlog forwards unconditionally (a form opening is itself a loss), and
  `DetailRegistry` tells **every** sub-tab, the way `on_item_change` does (blueprint B-4).
  - **Review L1:** a "loss" is the on-to-off edge, which equals "the holder lost the pointer" only
    because `RunsTab` is the sole view that wants the mouse. The docs on `Tab::wants_mouse`,
    `Tab::on_mouse_lost`, `DetailTab`/`DetailRegistry::on_mouse_lost` and `App::mouse_capture` say
    so: a second capturing view must also handle a switch between two capturing views.
- **D4, one way to end a gesture.** `RunsTab::end_gesture` clears the pane's flag and calls
  `ExecutionGraph::end_gesture`. That sends rataflow a left release with `locked` set, which
  resets the drag state and emits nothing (an unlocked release over a node press would click it).
  It replaces MOD-71 D11's four bare `gesture = false` sites, plus a fifth the plan missed: a press
  that lands while a gesture is still live (blueprint B-1).
- **D5, button-only reporting.** A private `EnableButtonMouseCapture` writes crossterm's sequence
  minus `?1003h`: `?1000h ?1002h ?1015h ?1006h`.
  - **Deviation from the item text**, which listed `?1000h ?1002h ?1006h`. `?1015h` (RXVT
    coordinates past 223) is kept, so urxvt-style terminals lose nothing they had.
  - Disable stays crossterm's `DisableMouseCapture`, a superset, so every give-back is unchanged.
  - On Windows the command delegates to crossterm's WinAPI command, `is_ansi_code_supported`
    false, so both the legacy console and Windows Terminal keep their path.
  - `App::on_mouse` still drops `Moved`, because Windows reports it and a terminal may ignore the
    narrower mode.
- **D6, a live pan re-anchors on every draw.** `ExecutionGraph::pan` holds the terminal cell of a
  live pan's last pointer:
  - every left press assigns it (a pan only on rataflow's `PaneClicked`; B-1);
  - drags move it; the release and `end_gesture` clear it.

  After `frame.render_widget`, a locked press at that cell rewrites rataflow's
  `Panning { anchor, initial_viewport }` from what was just drawn, with that frame's canvas origin.
  - Deltas are whole cells, so with nothing changed it is a no-op up to float association.
  - The re-anchor never touches the viewport or the buffer, so MOD-71 review L4's `Consumed`/`Pass`
    rule and every snapshot are unchanged.
  - A canvas under 2×2 skips it (review N5): with nothing drawn there is nothing to drag.
- **D7, no new crate, no rataflow change.** `locked` is toggled only inside one helper
  (`locked_left`), which restores the previous value with no early return in between. htui never
  renders rataflow's `Controls`, the only other reader.

## Implementation deviations

- **I-1:** the blueprint's `use crossterm::Command as _;` in the `terminal.rs` tests was unused and
  failed `clippy -D warnings`; it was removed in T1 green. The T1 red commit still carries it
  (compiles, warns), and review N6 needs no action.
- **I-2:** T3 red tests (c), (c′) and (d) already pass at the red commit, because the red shape adds
  the `pan` field they need. They stay as regression guards; (a), (b) and (e) carry the red.

## Gate

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 109 suites, 4227 passed, 0 failed, 30 ignored; no SIGABRT (on `b4c2f83b`) |
| `cargo test -p htui --all-features --no-fail-fast -- --test-threads=1` after the review fixes | 47 suites, 2177 passed, 0 failed, 3 ignored (on `d9733ec5`; the fixes touch only `crates/htui`) |
| L3 mutation check | `BacklogTab::on_mouse_lost` made a no-op → the end-to-end test fails (the stale anchor pans the nodes) |
| `Cargo.lock`, `crates/htui/tests/snapshots` | unchanged |

No store, migration, `.sqlx` or pin change.

## Carried

- **MOD-16 (Windows verification)** now also carries the mouse (HANDOFF note):
  - capture on Windows Terminal and on the legacy console;
  - an `$EDITOR` handoff before the flow view was ever opened (MOD-71 review H1's path);
  - a `cargo check` of the `#[cfg(windows)]` half of `EnableButtonMouseCapture`, which no Linux
    build compiles (blueprint B-5).
- **Pre-existing, not MOD-74:** `cargo doc -p htui --no-deps --all-features` with `-D warnings`
  fails on six private intra-doc links in `agent_worker.rs`, `detail/documents.rs`,
  `settings/personas.rs` and `ui/text_area.rs`. The gate does not run rustdoc, and the files MOD-74
  touched add no new error.
- **Optional upstream:** a public cancel-drag (or `drag_state`) API in rataflow would replace the
  `locked` toggle.
