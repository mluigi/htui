# Plan: MOD-74 mouse follow-ups - capture loss, button-only reporting, a pan across a re-read

**Source**: HANDOFF `MOD-74` (from MOD-71's review, `docs/decisions/mod/mod-71.md` "Carried"; `R-TUI-1`, `R-TUI-4`)
**Routed**: plan path via `/handoff-run` (C1-C4 none fired), no ultracode; accepted by the maintainer 2026-10-03.
Sandbox run `hr/MOD-74`.
**Complexity**: Small (three bounded fixes in one TUI area; no store, migration, `.sqlx` or dependency change)
**Status**: plan, awaiting CONFIRM

## Summary

MOD-71 gave the Runs flow view the mouse, and its review left three small gaps. Each one makes the view jump
once, or costs work nobody needs:

1. **Capture loss above `RunsTab` (review L3).** An overlay, `?`, a form, a sub-tab switch or a tab switch turns
   capture off without telling `RunsTab`. Its `gesture` flag and rataflow's `DragState::Panning` both stay live,
   so a button held across the off-and-on resumes the old pan anchor.
2. **Any-motion reporting.** crossterm's `EnableMouseCapture` writes `?1000h ?1002h ?1003h ?1015h ?1006h`
   (`crossterm-0.29.0/src/event.rs:321-345`). `?1003h` streams a `Moved` per cell crossed, which `App::on_mouse`
   then drops (`app/state.rs:528-533`).
3. **A re-read during a live pan.** `ExecutionGraph::sync`'s review-L2 anchor moves `flow.viewport` by the
   layout shift (`execution_graph.rs:513-516`). rataflow's `Panning` keeps its own `initial_viewport`
   (`rataflow-0.1.0/src/state/mouse.rs:118-121`, `:818-825`), and `set_nodes` leaves `drag_state` alone
   (`state/graph.rs:376-411`). The next drag therefore overwrites the shift.
   The fact-check found the same mechanism in a **wheel zoom during a live pan**, with a bigger jump: the zoom's
   viewport is overwritten on the next drag (probe: `(-12.1, -4.1)` at zoom 1.2 → `(1, 0)`). D6 covers both.

rataflow is a registry crate (`rataflow = "0.1"`, `Cargo.toml:175`), and `drag_state` is `pub(crate)`
(`state/mod.rs:365`), so every fix lands on htui's side of the public API.

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: the app sees the capture edge.** `App` gains `mouse: bool`, the answer the loop last applied, and
  `pub fn mouse_capture(&mut self) -> bool`:
  - it computes `wants_mouse()`;
  - on a `true → false` edge it calls `Tab::on_mouse_lost` on **every** registered tab, not only the active one.
    After a tab switch the tab holding the gesture is no longer the active one;
  - it records the answer and returns it.

  The loop line becomes `term.set_mouse_capture(app.mouse_capture())?;`, still the only `set_mouse_capture(`
  call, and the shape test in `event_loop.rs` follows it. `wants_mouse(&self)` stays as the pure query that
  `App::on_mouse` gates on.
  - Rejected: the loop detecting the edge from `TerminalGuard`. The guard's flag also tracks `Unsupported` and
    the editor `leave`, and the app is where the tabs are.
  - Rejected: telling every tab on every step that capture is off. The answer is the same, but it is a broadcast
    per step for one edge.
- **D2: an `$EDITOR` handoff is a capture loss.** `Suspend::leave` turns capture off without the app knowing
  (`terminal.rs:183-191`). `App::take_external_edit` returns `Some` exactly when the loop is about to suspend;
  there, if `mouse` was `true`, the app runs the same loss path and clears `mouse`. When the editor returns, the
  loop's next `mouse_capture()` sees `false → wants` and re-enables, with no stale gesture. The flow view never
  starts an editor today: forms do, and a form already turns the mouse off. This closes the path by construction,
  not by that accident.
- **D3: the hook, defaulted down the chain.**
  - `Tab::on_mouse_lost(&mut self) {}` and `DetailTab::on_mouse_lost(&mut self) {}` are defaulted, so no other
    tab changes. This mirrors MOD-71 D4's `wants_mouse`/`on_mouse`.
  - `BacklogTab` forwards to `DetailRegistry::on_mouse_lost`, which calls **every** sub-tab: a sub-tab switch
    leaves the Runs pane inactive.
  - `RunsTab` ends the gesture (D4).
- **D4: one way to end a gesture.** `RunsTab::end_gesture()` = `gesture = false` + `ExecutionGraph::end_gesture()`.
  It replaces the four bare `self.gesture = false` sites (`runs.rs:1280`, `:1337`, `:1361`, `:1405`; MOD-71 D11) and
  serves `on_mouse_lost`.
  - `ExecutionGraph::end_gesture()` forgets the live pan (D6). If `flow.is_dragging()`, it also feeds rataflow a
    left release **with `locked` set**, which only resets `drag_state` (`event_handlers.rs:496-509`, `Up` →
    `DragState::None`) and emits nothing; an unlocked release over `AwaitingNodeClick` would emit a click. After
    that it restores `locked`.
  - `Up(Left)` while a pan is live keeps going through `on_mouse` as today. Only the forced end uses the lock.
- **D5: button-only reporting on ANSI terminals.** `terminal.rs` gains a private `EnableButtonMouseCapture`
  command, crossterm's sequence minus `?1003h`: `?1000h ?1002h ?1015h ?1006h`.
  - **Deviation from the item text**, which lists only `?1000h ?1002h ?1006h`. `?1015h` (RXVT coordinates past
    223) is kept, so a terminal with no SGR mode (urxvt) loses nothing it has today. The only change is the hover
    stream.
  - Disable stays crossterm's `DisableMouseCapture`, a superset that also clears `?1003l`. Every give-back
    therefore stays unconditional and unchanged (MOD-71 D3).
  - On Windows the command delegates `execute_winapi` to `crossterm::event::EnableMouseCapture` and answers
    `is_ansi_code_supported() == false`, as crossterm's own command does (`event.rs:336-344`; `execute!` dispatches on it, `command.rs:123`, `:290`). The legacy console
    keeps the WinAPI path, as the item says, and so does Windows Terminal: crossterm never sends mouse ANSI on
    Windows.
  - `App::on_mouse` keeps dropping `Moved`. Windows still reports it, and a terminal may ignore the narrower
    mode.
- **D6: a live pan re-anchors on every draw.** `ExecutionGraph` gains `pan: Option<(u16, u16)>`, the terminal
  cell of a live pan's last pointer:
  - set on a left press whose response carries `FlowEvent::PaneClicked`, which is rataflow's empty-canvas
    `Panning` start (`mouse.rs:263-273`);
  - updated on each forwarded drag;
  - cleared on the release and in `end_gesture`.

  At the end of `render`, after `frame.render_widget`, when `pan` is set and `flow.is_dragging()`, the graph
  replays a **locked** left press at that cell. That sets `Panning { anchor_canvas: pointer, initial_viewport:
  current viewport }` (`event_handlers.rs:496-503`, no hit test). The lock is then restored.
  - The next drag pans from what is on screen. This covers the review-L2 anchor shift (the item), a head line
    that moves the canvas inside the pane (rataflow's `render_context` is fresh after the draw), and a wheel zoom
    mid-pan.
  - Pan deltas are whole canvas cells (`terminal_to_canvas` is `cell + 0.5`, `render_context.rs:74-79`), so
    re-anchoring at the last pointer, with nothing changed, gives the same viewport as not re-anchoring, up to
    float association (`viewport.x` may be fractional after a wheel zoom). That is at most one ulp, never a cell,
    so the tests compare with a tolerance.
  - Rejected: re-anchor only in `sync`. The canvas origin used by `terminal_to_canvas` is the previous frame's
    there, so a head line that came with the re-read would still shift the view by a row.
  - Rejected: skip the L2 anchor while panning. The nodes would then jump by the layout shift instead, which is
    the jump L2 removed.
- **D7: no new crate, no rataflow change.** `locked` is a public field (`state/mod.rs:392`). The flow never
  renders rataflow's `Controls` widget (the only reader of `locked` outside input, `ui/controls.rs:453`), so the
  toggle is never visible.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Defaulted trait hook | `ui/tabs/registry.rs:95-103`, `backlog/detail/mod.rs:110-117` | `wants_mouse`/`on_mouse` defaulted; doc names the MOD-71 decision |
| Registry fan-out | `backlog/detail/mod.rs:272-283` | `DetailRegistry` forwards to sub-tabs (`on_paste` shape) |
| App-level mouse test probe | `app/update.rs:1279-1336` | `Pointer` tab with `Rc<Cell>` handles, `pointing(..)` constructor |
| Sub-tab probe | `backlog/mod.rs:1290-1330` | `MouseProbe` `DetailTab` + `mouse_probe(..)` |
| Pane gesture tests | `runs.rs:4250-4340` | `flowing`, `blank_cell`, `mouse(kind, col, row)`, `pane.gesture` asserts |
| Terminal command shape | `terminal.rs:236-257` | own `execute!`, `tolerate_unsupported`, source-shape test |
| Loop shape test | `event_loop.rs:93-112` | comment-stripped source, order + single-call asserts |
| Errors | `terminal.rs:156-168` | `io::Result`, `Unsupported` tolerated; nothing new fails |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/terminal.rs` | UPDATE | T1 | D5 command, helper, tests |
| `crates/htui/src/event_loop.rs` | UPDATE | T2 | D1 loop line + shape test |
| `crates/htui/src/app/state.rs` | UPDATE | T2 | D1 `mouse` field, `mouse_capture`; D2 `take_external_edit` |
| `crates/htui/src/app/update.rs` | UPDATE | T2 | D1/D2 tests (`Pointer` grows a loss counter) |
| `crates/htui/src/ui/tabs/registry.rs` | UPDATE | T2 | D3 `Tab::on_mouse_lost` |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | T2 | D3 forward + `MouseProbe` test |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | T2 | D3 `DetailTab::on_mouse_lost`, registry fan-out |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T2, T3 | D4 `end_gesture` + `on_mouse_lost`; D6 pane tests |
| `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | UPDATE | T2, T3 | D4 `end_gesture`; D6 `pan` + re-anchor |
| `docs/decisions/mod/mod-74.md`, `docs/DECISIONS.md`, `HANDOFF.md`, this plan | CREATE/UPDATE | T4 | close-out |

**Independence.** T1 touches `terminal.rs` only, and T2/T3 never touch it, so T1 is independent. T2 and T3 share
`runs.rs` and `execution_graph.rs`, so they run **serially**, T2 then T3. Expected shape: one implementer, T1 →
T2 → T3, committing per task (red commit, then green).

## Tasks

### Task 1: button-only reporting (D5, TDD)
- **Red**: in `terminal.rs` tests:
  - `EnableButtonMouseCapture`'s `write_ansi` equals `"\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h"` and contains
    no `?1003h`;
  - every `?NNNNh` it writes has its `?NNNNl` in `DisableMouseCapture`'s output;
  - the source-shape test expects `fn enable_mouse_capture()` to issue `EnableButtonMouseCapture`.
- **Green**: the `Command` impl (`#[cfg(windows)]` delegation per D5) and `enable_mouse_capture` switched to it.
  Update the module doc and `enable_mouse_capture`'s doc (`?1003h` is gone).
- **Validate**: `cargo test -p htui --all-features terminal::`

### Task 2: the capture-lost hook (D1-D4, TDD)
- **Red**:
  - `app/update.rs`: `Pointer` counts `on_mouse_lost`. Tests:
    - `mouse_capture()` true→false calls it once, and false→false and true→true never;
    - an overlay/`?` open is a loss;
    - a non-active tab is told too;
    - `take_external_edit` with capture on is a loss, and `mouse` reads false after it.
  - `backlog/mod.rs`: `on_mouse_lost` reaches every `MouseProbe` sub-tab, including inactive ones.
  - `runs.rs`: a press on blank canvas, then `on_mouse_lost`, then a drag → `Pass` and the viewport unchanged;
    `graph.is_dragging()` false (test accessor).
  - `event_loop.rs`: the shape test expects `term.set_mouse_capture(app.mouse_capture())?;`.
- **Green**: D1-D4 as written; the four D11 sites call `end_gesture()`.
- **Validate**: `cargo test -p htui --all-features -- mouse gesture capture`

### Task 3: re-anchor a live pan (D6, TDD)
- **Red**: in `execution_graph.rs` / `runs.rs` tests:
  - (a) press on blank, drag by (dx, dy), then a same-run `sync` whose layout widens (the review-L2 fixture shifts
    the viewport), render, then drag by (dx2, dy2). The viewport equals post-sync + (dx2, dy2) cells. Today it
    falls back to pre-sync + total delta;
  - (b) a wheel tick mid-pan, then a drag: the zoom survives and the pan continues from it;
  - (c) a press on a node never sets `pan`, and its release is still a click;
  - (d) after a release, render does not re-anchor (`pan` is `None`, `is_dragging` false).
- **Green**: the `pan` field, set/update/clear in `on_mouse`, the locked replay at the end of `render`, and
  `end_gesture` clearing `pan`.
- **Validate**: `cargo test -p htui --all-features execution_graph runs::`. The three `runs_flow_*` snapshots stay
  byte-identical.

### Task 4: close-out docs
- `docs/decisions/mod/mod-74.md` (mirror `mod-71.md`), DECISIONS index row, HANDOFF item checked and the status
  paragraph, plan status → done.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
git diff --stat main -- crates/htui/tests/snapshots Cargo.lock             # expect empty
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A terminal ignores `?1002h` without `?1003h` and stops reporting drags | Low | `?1002h` is the xterm button-event mode every SGR terminal implements; MOD-16 exercises terminals; Windows path unchanged |
| The `locked` toggle leaks (left `true`) and every press pans, so no click works | Low | set/restore in one helper with no early return between; test (c) after a re-anchor |
| Re-anchor on every draw changes a pan with nothing re-read (float drift) | Low | deltas are whole cells; drift is float association only (≤1 ulp); tests use a tolerance |
| Windows half unverified: no `x86_64-pc-windows-*` target installed here | Medium | delegation is two forwarding lines mirroring crossterm; MOD-16 already carries Windows capture verification |
| A loss edge missed: capture off via a path that skips `mouse_capture()` | Low | the loop calls it every step (shape test); the only off-path outside it is `leave`, covered by D2 |

## Acceptance
- [ ] T1-T4 complete, each red-then-green committed
- [ ] Validation passes; snapshots and `Cargo.lock` unchanged
- [ ] Patterns mirrored (defaulted hooks, probe tests, shape tests), not reinvented

## Verified claims (step 3.5)

Checked 2026-10-03 against the tree and the locked dependency sources. Probes ran in a scratch crate
(`/tmp/mod74-probe`, `crossterm =0.29.0`, `rataflow =0.1.0`, the repo's `Cargo.lock`, toolchain 1.98.1, `--offline`).

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `EnableMouseCapture` writes `?1000h ?1002h ?1003h ?1015h ?1006h` | ✓ (line ref amended `:321-345`) | `crossterm-0.29.0/src/event.rs:321-345` |
| 2 | crossterm's Windows mouse commands answer `is_ansi_code_supported() == false`, and `execute!` dispatches on it | ✓ | `event.rs:336-344`; `command.rs:123`, `:290` |
| 3 | A private `crossterm::Command` impl compiles in htui's toolchain and `execute!` takes it | ✓ compile probe | probe: writes `\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h`, 32 bytes, no `1003` |
| 4 | The `#[cfg(windows)]` delegation compiles | **unverified** | no `x86_64-pc-windows-*` target installed (`rustup target list --installed`); carried as a risk; MOD-16 verifies Windows capture |
| 5 | `htui` depends on `crossterm` directly | ✓ | `crates/htui/Cargo.toml:42` |
| 6 | `App::on_mouse` drops `Moved`/`ScrollLeft`/`ScrollRight` | ✓ | `app/state.rs:528-533` |
| 7 | The review-L2 anchor moves `flow.viewport` in `sync` | ✓ (line ref amended `:513-516`) | `execution_graph.rs:514-515` |
| 8 | `Panning` keeps its own `initial_viewport`; a drag sets `viewport = initial + delta` | ✓ | `rataflow-0.1.0/src/state/mouse.rs:118-121`, `:818-825` |
| 9 | `set_nodes` leaves `drag_state` alone | ✓ | `state/graph.rs:376-411` (only `clear()` resets it, `:985`) |
| 10 | `drag_state` is `pub(crate)`, `locked` is `pub` | ✓ | `state/mod.rs:365`, `:392` |
| 11 | Locked: a left press is `Panning` at the pointer with no hit test; a left release sets `None` and emits nothing | ✓ source + probe | `event_handlers.rs:496-509`; probe: locked release after a node press → `[]`, unlocked → `[NodeClicked]` |
| 12 | Item (3) reproduces, and the D6 re-anchor removes it | ✓ probe | drag, `viewport.x -= 5`, draw, drag: no fix `(4,2)`, fix `(-1,2)` = expected |
| 13 | A wheel zoom mid-pan is overwritten by the next drag (new finding), and D6 fixes it | ✓ probe | no fix: `(-12.1,-4.1)` → `(1,0)`; fix: → `(-11.1,-4.1)` = expected |
| 14 | An empty-canvas press responds with `PaneClicked` and a node press does not | ✓ source + probe | `mouse.rs:263-273`; probe: node press → `[SelectionChanged]` only |
| 15 | Pan deltas are whole cells | ✓ (amended: up to float association) | `render_context.rs:74-79` (`cell + 0.5`) |
| 16 | `render` refreshes rataflow's `render_context` | ✓ | `ui/canvas.rs:36` `set_canvas_area` |
| 17 | htui never renders rataflow's `Controls` widget, the only non-input reader of `locked` | ✓ | no `Controls`/`controls::` in `crates/htui/src` (only `ControlsAction`); `ui/controls.rs:453` |
| 18 | `Suspend::leave` turns capture off without the app knowing | ✓ | `terminal.rs:183-191` |
| 19 | `take_external_edit` returns `Some` exactly when the loop suspends | ✓ | `app/state.rs:373-375`; `event_loop.rs:54-65` |
| 20 | The Runs pane never starts an editor | ✓ | no `external_edit`/`ExternalEdit` in `detail/runs.rs` or `detail/runs/` |
| 21 | `TabRegistry` can visit every tab | ✓ | `ui/tabs/registry.rs:175` `iter_mut` |
| 22 | The four bare `gesture = false` sites | ✓ | `runs.rs:1280`, `:1337`, `:1361`, `:1405` |
| 23 | Patterns cited (probes, shape tests, defaulted hooks) exist at the cited lines | ✓ | `app/update.rs:1279-1336`, `backlog/mod.rs:1290-1330`, `event_loop.rs:93-112`, `registry.rs:95-103`, `detail/mod.rs:110-117`, `:272-283` |
| 24 | A review-L2 fixture exists for test (a) | ✓ | `execution_graph.rs:1380` `a_wider_re_read_of_the_same_run_keeps_the_nodes_still`, `:1905` `a_re_read_after_a_pan_keeps_the_viewport` |
| 25 | T1 is file-disjoint from T2/T3; T2 and T3 intersect | ✓ | Files-to-Change task column: T1 `{terminal.rs}`; T2∩T3 = `{runs.rs, execution_graph.rs}` → serial |
