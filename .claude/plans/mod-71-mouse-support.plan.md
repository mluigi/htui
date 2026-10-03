# Plan: MOD-71 mouse support - capture policy and the Runs flow view

**Source**: HANDOFF `MOD-71` (from MOD-28, `docs/decisions/mod/mod-28.md` "Carried"; `R-TUI-1`, `R-TUI-4`;
`docs/ANA-12.md` §"Route events conditionally based on focus")
**Routed**: plan path via `/handoff-run` (C2 fired, C3 borderline; low-confidence threshold note), accepted by the
maintainer 2026-10-03. Sandbox run `hr/MOD-71`.
**Complexity**: Medium (one new event seam through two trait levels, a terminal-mode lifetime, one view's gestures)
**Status**: confirmed 2026-10-03; blueprint next

## Summary

htui has never turned mouse capture on: `App::on_terminal_event` drops every `Event::Mouse` (`app/state.rs:413-424`,
`_ => {}`), and nothing in `crates/` issues `EnableMouseCapture`. Turning capture on takes the terminal's own text
selection away. So MOD-71 turns it on **only while the view on screen wants the mouse**. Today that is just the Runs
flow view in browse mode. The event loop asks the app after every step and toggles capture when the answer
changes. Every path that gives the terminal back (restore, panic hook, `$EDITOR` suspend) turns capture off.

`Event::Mouse` gains a route: `App` → active `Tab` → Backlog → active `DetailTab` → `RunsTab` → `ExecutionGraph` →
`rataflow`'s `Flow::handle_mouse_event`. In the flow view a click on a node moves the shared cursor
(`FlowEvent::NodeClicked`), a drag on empty canvas pans, and the wheel zooms at the pointer. Nodes stay read-only
(MOD-28 D9).

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: capture only while a view wants it.** This is the item's proposal, and it is adopted.
  `App::wants_mouse()` = no overlay is open **and** the active tab's `wants_mouse()`. Backlog answers that no
  filter form and no item form is open, and the active sub-tab wants the mouse. `RunsTab` answers
  `view == View::Flow && matches!(self.mode, Mode::Browse)`. Every other view keeps the terminal's selection.
  - Rejected: always-on capture, which kills text selection in every tab (the list, the artifact view, the Docs body).
  - Rejected: a toggle key, which is one more key to learn and leaves the policy to the user to manage.
  - While the flow view is shown, a terminal's bypass modifier (Shift in xterm, GNOME Terminal, kitty and Windows
    Terminal; Option in iTerm2/Terminal.app) still selects text. That goes in the operator note, not in the pane.
- **D2: the event loop owns the toggle.** `TerminalGuard` gains `mouse: bool` and
  `set_mouse_capture(&mut self, on: bool) -> io::Result<()>`. The method writes `Enable`/`DisableMouseCapture`
  only when `on` differs from `mouse`, so it is idempotent and costs nothing per frame.
  - `event_loop::run` calls `term.set_mouse_capture(app.wants_mouse())?` after each `select!` step and after the
    editor post-step, just before the dirty check. A toggle that `v` or `Esc` caused therefore lands before the
    next frame.
  - Each toggle is its own `execute!` with `Unsupported` tolerated, the shape bracketed paste has (review R2-L6).
    A terminal without mouse reporting runs keyboard-only.
- **D3: every give-back turns it off.**
  - `restore_terminal()`, the one restore behind both the panic hook and `TerminalGuard::restore`, disables
    capture unconditionally, first, best effort, before bracketed paste. It is a free function, so it cannot know
    the flag, and a disable on a terminal that never enabled it is harmless.
  - `Suspend::leave` disables capture and clears `mouse`. `Suspend::enter` does **not** re-enable it: the loop's
    next `set_mouse_capture` re-asserts whatever the app wants once the editor is gone, so there is one place that
    decides.
  - The source-shape test in `terminal.rs` (`every_path_that_takes_the_terminal_…`) grows the mouse half. That
    test is the precedent, and `tests/panic_hook_order.rs` reads the same file.
- **D4: the route.** `Event::Mouse(m)` → `App::on_mouse(m)`, with three properties:
  - **Gated first.** `!self.wants_mouse()` drops the event. This catches events already queued before capture
    went off.
  - **No `dirty`, no status-line clear** for an event nobody consumed. Capture is `?1003h` (any-motion,
    `crossterm-0.29.0/src/event.rs:325-333`), so a `Moved` flood must not cause one redraw per cell. `Moved`,
    `ScrollLeft` and `ScrollRight` are dropped before dispatch.
  - **Overlays never see a mouse event.** D1 keeps capture off while one is open, and the `Overlay` trait is not
    touched.
  - `Tab::wants_mouse(&self) -> bool { false }` and
    `Tab::on_mouse(&mut self, MouseEvent, &mut Ctx) -> Handled { Handled::Pass }` are defaulted, so no other tab
    changes, and `DetailTab` mirrors them one level down. `DetailRegistry` forwards to the active sub-tab.
    Backlog forwards to `self.detail` under the same guard as its `wants_mouse`.
- **D5: gestures the flow view takes.** `RunsTab` records the canvas `Rect` it drew last
  (`canvas: Cell<Option<Rect>>`, set in `render_flow`, `None` on any frame without a canvas) and forwards these
  to `ExecutionGraph::on_mouse`:
  - `Down(Left)` and the wheel (`ScrollUp`/`ScrollDown`) only inside that `Rect`.
  - `Drag(Left)` and `Up(Left)` while a gesture that started inside is live (`gesture: bool`), so a pan carried
    past the pane edge still ends.
  - Right and middle buttons are dropped. In rataflow a right-drag becomes a box selection (`mouse.rs:654-661`)
    and a right-click a context-menu event, and neither has a meaning here.

  rataflow converts terminal coordinates itself: the render context records the canvas area on each draw
  (`ui/canvas.rs:37`), so no offset is computed on our side.
- **D6: click selects; the flow still keeps no selection of its own** (MOD-28 D7).
  - `FlowEvent::NodeClicked { node_id }` → the cursor moves to the entry of the cursor's run whose `StepId`
    string matches. Only that run's nodes are on the canvas. Then `sync_graph` runs, exactly as `J`/`K` do.
  - So rataflow's selection never drifts from the cursor:
    - The flow is built `with_deselect_on_pane_click(false)`. Its default `true` (`state/mod.rs:509`) would clear
      the cursor node's highlight on a pane click.
    - Every edge is built `with_selectable(false)`. The edge hit test skips non-selectable edges
      (`mouse.rs:460/489`), so a press on an edge pans like the pane.
  - A press on a node is a click wherever it is released. rataflow puts a non-draggable node in
    `AwaitingNodeClick`, ignores the drag (`mouse.rs:722-820`, `_ => Handled`), and emits `NodeClicked` on release
    (`:843-846`). Panning starts from empty canvas. This is accepted rather than patched around.
- **D7: a pan or a wheel zoom survives a re-read.**
  - Today every same-run `sync` queues `Reveal::Cursor`, which calls `ensure_node_visible`
    (`execution_graph.rs:~493-500`). The active-run poll (`backlog/mod.rs:69`, every 5th refresh) and every
    `RunStream` re-read would therefore drag the viewport back the moment a pan takes the cursor node off screen.
  - The change: a same-run `sync` queues `Reveal::Cursor` **only when the cursor step changed**, and mouse
    gestures queue no reveal.
  - The review-L2 anchor already keeps nodes still across a re-read, and `+`/`-`/`=` keep their `Reveal::Cursor`.
  - A resize of the canvas still reveals the cursor: `render` compares the area with the last one drawn. A new
    run still resets.
- **D8: read-only stays read-only (MOD-28 D9).** The node builders are unchanged (`with_draggable(false)`,
  `connectable(false)`, `deletable(false)`, hidden handles). A press on a hidden or non-connectable handle falls
  through to a node click (`handle_new_connection`, `mouse.rs:291-312`). `multi_select_mode` is flipped only by a
  key binding no key ever reaches (blueprint H-1), so it stays `false`.
- **D9: wheel and keys zoom alike.** The wheel uses `DEFAULT_ZOOM_FACTOR = 1.2` around the pointer
  (`event_handlers.rs:48`), and the keys use `ZOOM_STEP = 1.2` around the centre (`state/viewport.rs:8`). Both are
  clamped to the flow's 0.5–2.0 (`DEFAULT_MIN/MAX_ZOOM`), which is MOD-28's E9 range. Nothing to configure.
- **D10: `rataflow`'s `crossterm` feature.** `features = ["crossterm"]` on the workspace dependency, for
  `From<crossterm::event::MouseEvent>`. It turns on `ratatui/crossterm`, which the workspace's default-featured
  `ratatui` already has. The probe compiled with `Cargo.lock` unchanged. The `Cargo.toml` comment already says
  "`crossterm` is for the mouse follow-up".
- **D11: the gesture ends with the view.** `v` back to the list, an item change, or a mode that captures input
  clears `gesture`. A capture toggled off mid-drag would otherwise leave rataflow in `Panning` until the next
  press, and a new press resets it anyway (`on_mouse_down` overwrites `drag_state`).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Terminal mode on/off at every take/give-back | `terminal.rs` bracketed paste (`enable_/disable_bracketed_paste`, `tolerate_unsupported`) | own `execute!` per command, `Unsupported` tolerated, source-shape test over every path |
| Trait default added without touching other impls | `registry.rs` `Tab::on_paste` / `on_refresh`; `detail/mod.rs` `DetailTab::on_paste` / `has_active_run` | defaulted method with a doc saying why it is defaulted |
| App-level dispatch over overlays then tab | `app/state.rs:427-499` `on_paste` | destructure `Self`, build `Ctx`, `drain(&origin)` |
| Backlog → detail forwarding | `backlog/mod.rs:565-575` `on_paste` | forms first, then `self.detail.…` |
| Cursor move + graph sync | `runs.rs:1293-1298` `J`/`K` | `move_cursor` / set `selected`, then `sync_graph(ctx.theme)` |
| `&self` render recording geometry | `runs.rs` `Mode::Artifact.width: Cell<u16>` | `Cell` written in `render`, read by the input path |
| Harness entry point | `testkit.rs:570` `Harness::paste` | builds the crossterm event, goes through `on_terminal_event` |
| Tests | `execution_graph.rs` `mod tests` (`synced`, `draw_at`, `corner_of`); `runs.rs:3615-3960` flow tests; `tests/backlog.rs:772-900` | `TestBackend`, coordinates from `corner_of`, `insta` only where a frame is the claim |
| Errors | `io::Result` through the event loop (`event_loop.rs`, plan Patterns: no `anyhow` below `main`) | no new error type |

## Files to Change

| File | Action | Why |
|---|---|---|
| `Cargo.toml` | UPDATE | `rataflow` `features = ["crossterm"]` (D10) |
| `crates/htui/src/terminal.rs` | UPDATE | `mouse` flag, `set_mouse_capture`, disable in `restore_terminal` and `Suspend::leave`, module doc, source-shape test (D2, D3) |
| `crates/htui/src/event_loop.rs` | UPDATE | `set_mouse_capture(app.wants_mouse())` per step (D2) |
| `crates/htui/src/app/state.rs` | UPDATE | `Event::Mouse` arm, `on_mouse`, `wants_mouse` (D1, D4) |
| `crates/htui/src/ui/tabs/registry.rs` | UPDATE | `Tab::wants_mouse` / `on_mouse` defaults (D4) |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | Backlog `wants_mouse` / `on_mouse` (D1, D4) |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | `DetailTab` defaults, `DetailRegistry` forwarding (D4) |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | `RunsTab` `wants_mouse` / `on_mouse`, `canvas` cell, `gesture`, NodeClicked → cursor (D5, D6, D11) |
| `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | UPDATE | `on_mouse`, flow/edge config, reveal rule, resize reveal (D6–D8) |
| `crates/htui/src/testkit.rs` | UPDATE | `Harness::mouse(kind, column, row)` |
| `crates/htui/tests/backlog.rs` | UPDATE | end-to-end: click moves the cursor, drag pans, wheel zooms, capture wanted only in flow |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-71.md`, `docs/ANA-12.md`, `docs/decisions/mod/mod-28.md` | UPDATE/CREATE | close-out (T4); MOD-28 "Carried" fan-out-width note resolved by free panning |

Eleven code files. The tasks form one chain: T1's `set_mouse_capture` is called by T2's event-loop line, and T3
implements T2's trait methods. They run **serial**, with no implementer fan-out.

## Tasks

### Task 1: capture lifetime in the terminal (TDD)
- **Action**: Write the test first: extend the source-shape test so that `restore_terminal` and `leave` contain
  `disable_mouse_capture()`, the helpers issue only their command and tolerate `Unsupported`, and `enter` does
  **not** enable capture (D3). Then add the `mouse` flag, `set_mouse_capture`, the two helpers, the disables, and
  a module-doc paragraph mirroring the bracketed-paste one.
- **Mirror**: bracketed paste in the same file.
- **Validate**: `cargo test -p htui --all-features terminal`; `cargo test -p htui --all-features --test panic_hook_order --test panic_hook`.

### Task 2: the event seam (TDD)
- **Action**: Write the tests first:
  - `App::wants_mouse` is false on every tab at start-up.
  - A mouse event while not wanted changes nothing and leaves `dirty` false.
  - `Moved` never sets `dirty`.
  - An open overlay makes `wants_mouse` false.
  - The Backlog forwards to the detail only with no form open.

  Use a stub `DetailTab` with a `Cell` flag, as in `backlog/mod.rs:844`. Then add the trait defaults,
  `DetailRegistry`/Backlog forwarding, `App::on_mouse`/`wants_mouse`, the `Event::Mouse` arm,
  `Harness::mouse`, and the event-loop line.
- **Mirror**: the `on_paste` chain at every level.
- **Validate**: `cargo test -p htui --all-features app:: backlog::`; `cargo build -p htui`.

### Task 3: gestures in the flow view (TDD)
- **Action**: Write the unit tests first.
  - In `execution_graph`, after a `draw_at`:
    - a click on a node's corner yields its `StepId`
    - a drag on empty canvas moves `viewport.x/y` and leaves every node position unchanged
    - a press on a node, dragged then released, moves no node and yields a click (D6)
    - the wheel zooms in and out, clamped to 0.5 and 2.0
    - a right-drag changes no selection
    - an edge press pans
    - a pane click keeps the cursor node selected
    - a same-run, same-cursor `sync` after a pan keeps the viewport (D7)
    - a cursor change reveals the cursor
    - a resize reveals the cursor
  - In `runs`:
    - `wants_mouse` is true in Flow+Browse only, and false in the list, a reject note, a cancel confirm and the
      artifact view
    - a click moves `selected_step()`
    - a press outside the canvas passes
    - a drag that leaves the canvas still ends
    - `v` clears the gesture
  - In `tests/backlog.rs`, through `Harness::mouse`: a click on the second candidate of the fan-out run moves the
    cursor, and an action key then acts on that step (ANA-12 invariant 2).

  Then the implementation (D5–D11) and the `Cargo.toml` feature.
- **Mirror**: MOD-28/MOD-72 flow tests (`synced`, `draw_at`, `corner_of`; `runs.rs:3638` `J`/`K` across runs).
- **Validate**: `cargo test -p htui --all-features execution_graph runs`; `cargo test -p htui --all-features --test backlog`. The existing flow snapshots must not change.

### Task 4: close-out docs
- **Action**: Write the following:
  - `docs/decisions/mod/mod-71.md` (D1–D11, the bypass-modifier note for operators)
  - the DECISIONS index row
  - the HANDOFF status block and checklist tick; the status paragraph's "keyboard only: mouse support is MOD-71"
    line updated
  - an ANA-12 status note (mouse routing done)
  - the MOD-28 "Carried" fan-out-width bullet marked resolved by free panning
  - this plan's status set to done
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # HTUI_TEST_DATABASE_URL is set in the sandbox
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

No store, migration or `.sqlx` change.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Any-motion reports (`?1003h`) flood the loop and redraw per cell | High without D4 | `Moved` dropped before `dirty`; unit test pins it |
| A pan is undone by the next poll or `RunStream` re-read | High without D7 | reveal only on a cursor change; test re-syncs after a pan |
| Capture left on after a panic, an error exit or `$EDITOR` | Medium | D3: `restore_terminal` and `leave` disable unconditionally; source-shape test |
| rataflow's selection drifts from the cursor (pane or edge click) | Medium | D6 flow/edge config; tests for pane and edge presses |
| A click before the first flow frame hit-tests a zero render context | Low | `canvas` is `None` until `render_flow` draws, so presses pass |
| Users lose terminal text selection in the flow view | Certain, by design | D1 scope; bypass modifier documented in mod-71.md |
| Windows console mouse mode differs | Low | crossterm handles the console mode; `Unsupported` tolerated; MOD-16 verifies on the box |
| Trait-default comments ("the trait's third default") conflict with a sibling branch at merge | Medium | additive defaults at the end of each trait; mechanical on collect |
| htui suite flake from scheduling | Medium | gate with `--test-threads=1`, `--no-fail-fast` |

## Acceptance

- [ ] All tasks complete, TDD order kept
- [ ] Capture is on only in the flow view in browse mode, and off on every give-back path
- [ ] Click, drag and wheel work in the flow view; no node moves; existing flow snapshots unchanged
- [ ] Validation passes on the real tree

## Verified claims (step 3.5)

Checked 2026-10-03 at `30493f39` (`hr/MOD-71`, clean tree).

| Claim | Verdict | Evidence |
|---|---|---|
| Nothing in `crates/` enables mouse capture or handles `Event::Mouse` | ✓ | `grep MouseCapture\|Event::Mouse crates/**/*.rs`: no match |
| `on_terminal_event` drops mouse events | ✓ | `app/state.rs:413-424`, `_ => {}` |
| Bracketed paste is the on/off precedent, with a source-shape test, and the panic hook and guard both restore through `restore_terminal` | ✓ | `terminal.rs` `restore_terminal`, `install_panic_hook`, `TerminalGuard::restore`, test `every_path_that_takes_the_terminal_…` |
| `Suspend::leave`/`enter` are the editor-suspend path; `run_suspended` calls them | ✓ | `terminal.rs` `impl Suspend for TerminalGuard`; `editor.rs:270-305` |
| `tests/panic_hook_order.rs` forbids only ratatui init / `set_panic_hook` in `terminal.rs` | ✓ | its asserts at `:55-74`; a mouse helper trips none |
| `Tab` and `DetailTab` already take defaulted event methods (`on_paste`) | ✓ | `registry.rs` `Tab::on_paste`; `detail/mod.rs` `DetailTab::on_paste` |
| Backlog forwards paste forms-first then to `detail` | ✓ | `backlog/mod.rs:565-575` |
| The detail pane is drawn whenever no item form is open | ✓ | `backlog/mod.rs:746-755` (item form replaces it; divergence view replaces the whole tab) |
| `DetailTab::render` is `&self`, and the graph sits in a `RefCell` | ✓ | `detail/mod.rs` trait; `runs.rs:210-212` |
| `ExecutionGraph::render` is `&mut` and draws through `&mut Flow` | ✓ | `execution_graph.rs:556-580` |
| rataflow records the canvas area at render, so `handle_mouse_event` takes absolute terminal coordinates | ✓ | `rataflow-0.1.0/src/ui/canvas.rs:37` `set_canvas_area`; `event_handlers.rs:482-490` `terminal_to_world` |
| `handle_mouse_event(impl Into<MouseEvent>)`; the `crossterm` feature adds `From<ct::MouseEvent>` | ✓ | `event_handlers.rs:482`; `input.rs:218-300` |
| Enabling the `crossterm` feature adds no crate and compiles | ✓ (probe) | `features = ["crossterm"]` → `cargo check -p htui --offline` finished; `git diff --stat Cargo.lock` empty; one `crossterm 0.29.0` in the lock; probe reverted |
| A non-draggable node press → `AwaitingNodeClick` → `NodeClicked` on release | ✓ | `mouse.rs:230-238`, `:843-846` |
| A drag from a node pans | ✗ (**plan amended**, D6) | `on_mouse_drag` has no `AwaitingNodeClick` arm (`mouse.rs:722-820`, falls to `_ => Handled`); panning starts from empty canvas only |
| Empty-canvas press pans; the pane click clears selection by default | ✓ | `mouse.rs:262-273`; `state/mod.rs:509` `deselect_on_pane_click: true`; builder `:655` |
| Non-selectable edges are skipped by the hit test | ✓ | `mouse.rs:460`, `:489` `if edge.hidden \|\| !edge.selectable` |
| Right-drag becomes a box selection | ✓ | `mouse.rs:654-661` |
| `multi_select_mode` defaults false and is flipped only by a key binding | ✓ | `state/mod.rs:511`; `event_handlers.rs:307`; builder `:688` |
| Hidden/non-connectable handle press is not a connection | ✓ | `mouse.rs:300-312` (`node.connectable && … && !h.hidden`) |
| Wheel and `+`/`-` zoom by the same 1.2 within the same 0.5–2.0 | ✓ | `event_handlers.rs:48` `DEFAULT_ZOOM_FACTOR = 1.2`; `state/viewport.rs:8` `ZOOM_STEP = 1.2`, `:11/:14` min/max |
| crossterm's `EnableMouseCapture` turns on any-motion reporting | ✓ | `crossterm-0.29.0/src/event.rs:325-333` (`?1000h ?1002h ?1003h ?1015h ?1006h`) |
| A same-run re-sync queues `Reveal::Cursor` today | ✓ | `execution_graph.rs` `sync`, `next = … Reveal::Cursor` |
| The active-run poll re-reads `Runs` every 5th refresh | ✓ | `backlog/mod.rs:69`, `:704` |
| Harness has `paste` through `on_terminal_event` and no mouse helper | ✓ | `testkit.rs:570-573` |
| Tasks are serial (T2 calls T1's API, T3 implements T2's methods) | ✓ | Files table: T1 `terminal.rs`; T2 `event_loop.rs`, `app/state.rs`, `registry.rs`, `backlog/mod.rs`, `detail/mod.rs`, `testkit.rs`; T3 `Cargo.toml`, `runs.rs`, `execution_graph.rs`, `tests/backlog.rs`. The sets are disjoint, but each task depends on the previous one's API, so no parallel marking |
