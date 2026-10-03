# Blueprint: MOD-71, mouse support (capture policy and the Runs flow view)

**Status**: **proposed** (2026-10-03). Plan deviations B-1 to B-12 (§0), hazards H-1 to H-14 (§0a)
and blueprint decisions E1 to E16 (§6) belong to this blueprint. A deviation marked **Blocker**
means the plan, read literally, produces a defect its own tests would show. The Fix column is what
the implementer builds. The plan's D1–D11 are binding and are not reopened. Where a fix narrows one
of them, the row cites the evidence.

**Plan**: `.claude/plans/mod-71-mouse-support.plan.md` at `c3243310`, **confirmed** 2026-10-03,
fact-checked (step 3.5). Tasks are cited as "MOD-71 T*n*" outside this file.

**Verified at**: HEAD `c3243310`, branch `hr/MOD-71`, clean tree. Anchors were read through Gortex
(`read`, `search`). The third-party sources were read directly:
`~/.cargo/registry/src/index.crates.io-*/rataflow-0.1.0/` and `crossterm-0.29.0/`. **Line numbers
are pre-edit**: once a task commits to a file, a citation into that file moves. Counted at this
HEAD: **143** tracked `crates/htui/tests/snapshots`. `df -h .` shows 2.8 T free (82 %).

**Graphify**: `graphify-out/` isn't in this checkout, so nothing here comes from it.

**Coupling verdict.** The plan's order stands: **serial T1 → T2 → T3 → T4, one implementer, no
fan-out**. T2's event-loop line calls T1's `set_mouse_capture`. T3 overrides T2's trait methods and
drives T2's `Harness::mouse`. No file joins a task beyond the plan's list.

**Scope**:
- **No migration, no store change, no `.sqlx`, no new crate.** One feature flag on an existing
  dependency (`rataflow` `crossterm`, D10). `Cargo.lock` does not move.
- **New code**: `TerminalGuard::{mouse, set_mouse_capture}` and two helpers; `Tab::{wants_mouse,
  on_mouse}` and `DetailTab::{wants_mouse, on_mouse}` (defaulted); `DetailRegistry`, `BacklogTab`,
  `App` forwarding; `Harness::mouse`; `RunsTab::{canvas, gesture, select_step}` and its two trait
  overrides; `ExecutionGraph::{drawn, on_mouse}` and `drawable`.
- **Pins that move**: none. Snapshots stay at **143** and none of the three flow snapshots changes
  (B-12).

**House style (carried)**:
- `[workspace.lints]` (`Cargo.toml:175-189`): `unsafe_code = "forbid"`, `missing_debug_implementations`
  and `unused_qualifications` warn, `clippy::all` warns. **`clippy::pedantic` is not enabled.** The
  gate is `cargo clippy --workspace --all-targets --all-features -- -D warnings`. rustdoc denies
  broken and private intra-doc links.
- Implementers commit after each step and stage only their own paths: never `-A`, never `stash`,
  never `--amend`. Every commit compiles. A red commit may put a `todo!()` body **only** in an item
  no product path calls (H-3).
- Integration tests need `--all-features` (or `--features testkit`). Without it, `tests/*.rs` runs
  0 tests and still reports ok. Every `htui` gate uses `--test-threads=1`.
- UI code never panics in a render path and never logs there. Comments are dense and cite the
  decision (`// MOD-71 D5: …`).

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `c3243310` | Fix |
|---|---|---|---|---|
| **B-1** | Non-blocker (ANA-12 invariant 2, edge case) | D6: "So rataflow's selection never drifts from the cursor" (pane-click and edge config). | A press on a **node** selects it at once: a non-draggable, selectable node is `select_node`d in `on_mouse_down` before `AwaitingNodeClick` (`rataflow` `state/mouse.rs:230-237`; the handle fallback does the same, `:329-337`). Between press and release the canvas highlights the pressed node while the cursor (what `a`, `x`, `r`… act on) is elsewhere. If D11 ends the gesture mid-press, the release never reaches rataflow and the highlight stays wrong until the next `sync`. Nodes cannot be built `with_selectable(false)` instead: a node neither selectable nor draggable is **transparent** to the hit test (`mouse.rs:435-436`), so a click on it would pan; and D8 keeps the builders unchanged. | E1: `ExecutionGraph::on_mouse` puts the selection back on the cursor after **every** forwarded event (`flow.select_node(&cursor)`, or `clear_selection()` with none). The flow keeps no selection of its own (MOD-28 D7), and D6 holds literally. Test `a_press_on_a_node_keeps_the_cursor_selected`. |
| **B-2** | Non-blocker (no-op) | D6: "Every edge is built `with_selectable(false)`"; Files table: "flow/edge config". | Already so: `edge()` ends `.with_selectable(false)` (`execution_graph.rs:372`), pinned by `no_node_is_draggable_connectable_or_deletable` (`:1230`, `all(\|edge\| !edge.selectable)`). | Only the **flow** config is new (`with_deselect_on_pane_click(false)`). `an_edge_press_pans` is green on write; it pins the existing config against a mouse. |
| **B-3** | Non-blocker (evidence, outcome holds) | D8: "A press on a hidden or non-connectable handle falls through to a node click (`handle_new_connection`, `mouse.rs:291-312`)". | Hidden handles never reach `handle_new_connection`: `hit_test` skips them (`mouse.rs:421-424`, `if handle.hidden { continue; }`), so the press is a **body** hit (`:436`) and becomes `AwaitingNodeClick` (`:230-237`). Every handle here is hidden (`handles()`, `execution_graph.rs:346-351`). | No code change. D8's outcome (a node click) is correct; the write-up cites `:421-424`. |
| **B-4** | **Blocker** (the gates as written error) | T2 Validate `cargo test -p htui --all-features app:: backlog::`; T3 Validate `cargo test -p htui --all-features execution_graph runs`. | `cargo test` takes one `TESTNAME` positional; a second is an argument error. No `--test-threads=1` (memory: the suite is scheduling-dependent). | §1 gates: one filter per invocation, `--lib`, `-- --test-threads=1`. `detail::runs` covers both `runs` and `runs::execution_graph`. |
| **B-5** | Non-blocker (D1 gap) | D1: `App::wants_mouse()` = no overlay open **and** the active tab wants it. | The `?` box is **not** an overlay: it is `App::help_visible` and `render_help` draws it over the body (`app/state.rs` `render`, `render_help`), outside `OverlayStack`. Read literally, capture stays on under the box, and a click through it acts on a node the user cannot see. | E7: `App::wants_mouse` also requires `!self.help_visible`. Narrows D1 in D1's own terms ("every other view keeps the terminal's selection"). |
| **B-6** | Non-blocker (D4 underspecified) | D4: "No `dirty`, no status-line clear for an event nobody consumed." | `on_key` clears `status` **before** dispatch so a failure the key causes still lands (`app/state.rs:504-508`). For a mouse event, consumption is known only after dispatch. | E8: `on_mouse` takes `status` before dispatch and puts it back unless the event was `Consumed` (and nothing new was written). `Consumed` sets `dirty`. |
| **B-7** | Non-blocker (a stale hit test) | D5: "`canvas: Cell<Option<Rect>>`, set in `render_flow`". | `ExecutionGraph::render` returns before rataflow records the canvas area when the area is under 2×2 (`execution_graph.rs:557-559`), so a canvas recorded there would be hit-tested against the previous frame's render context. | E6: one predicate `execution_graph::drawable(area)`, used by `ExecutionGraph::render` and by `render_flow`, which records `canvas` only when it is true. |
| **B-8** | Non-blocker (D11 placement) | D11: "a mode that captures input clears `gesture`". | Every entry into a capturing mode is an action key through `RunsTab::action` (`x`, `c`, `C`, `o`; the arm at `runs.rs:1323-1326`). `on_reply` only moves between capturing modes or back to `Browse`. | E11: the action arm clears `gesture` when `captures_input()` after the action; `v` and `on_item_change` clear it unconditionally; `RunsTab::on_mouse` clears it whenever it is not wanted (belt). |
| **B-9** | Non-blocker (placement) | T3 lists "a right-drag changes no selection" among the **`execution_graph`** tests, while D5 puts the right/middle drop in `RunsTab`. | A right-drag in rataflow becomes a box selection (`mouse.rs:654-683`) and would select every node in the box. | E2: `ExecutionGraph::on_mouse` forwards only `Down/Drag/Up(Left)` and `ScrollUp/Down`; anything else returns `None` untouched. `RunsTab` filters too (D5). |
| **B-10** | Non-blocker (test strength) | T3 integration: "an action key then acts on that step". | Both `ANA-1` candidates are in a finished run, so `a` is refused for either; whether the two refusal sentences differ isn't pinned anywhere. | The integration test asserts the `a` sentence **and** the decisive check: back in the list (`v`), the `▸` cursor is on the `0.1/1` row (§4.7). |
| **B-11** | Non-blocker (D7 detail) | D7: "`render` compares the area with the last one drawn". | The canvas's origin moves with the flow head's line count; the viewport is canvas-relative, so only a **size** change needs a reveal. | E5: `ExecutionGraph::drawn: Option<(u16, u16)>`, the last drawable size; a different size raises the pending reveal to `Reveal::Cursor`. |
| **B-12** | Non-blocker (confirmation) | T3: "The existing flow snapshots must not change." | `runs_flow_fanout`, `runs_flow_tool_chips` and `runs_flow_reject_note` each render once, after a `Reset` reveal. In `reject_note` the footer shortens the canvas before that render; the cursor node sits at the top, so a resize reveal moves nothing. | Unchanged. `cargo insta pending-snapshots` empty; 143 snapshots. |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | `RefCell` double borrow: `render_flow` (`&self`) holds `self.graph.borrow_mut()` (`runs.rs:1503`). | Every input path is `&mut self` and uses `self.graph.get_mut()` (as `sync_graph`, `:384-394`, and the `+`/`-`/`=` arms). `RunsTab::on_mouse` never calls `borrow`/`borrow_mut`. Tests read with `pane.graph.borrow()` only between calls. |
| **H-2** | `Ctx` destructuring in `App` (`let Self { scope, projects, top_bar, keymap, theme, emit, tabs, .. } = self;`, `app/state.rs:466-490`). Inside it `self` is moved-from. | `App::on_mouse` runs the gate (`self.wants_mouse()`), the kind filter and `self.status.take()` **before** the block; `self.drain(&origin)` and the `dirty`/`status` writes **after** it, exactly as `on_paste`'s tab half. |
| **H-3** | A red commit routes a live path to `todo!()`. | T2's red commit adds `todo!()` bodies in `App::{wants_mouse, on_mouse}`, `DetailRegistry::{wants_mouse, on_mouse}` and `BacklogTab`'s two overrides, but neither the `Event::Mouse` arm nor the event-loop line: nothing in the product calls them. T3's red commit adds only `ExecutionGraph::on_mouse` (`todo!()`, `#[cfg_attr(not(test), expect(dead_code, …))]`); `RunsTab` keeps the trait defaults (`false`/`Pass`), so the App still never routes a mouse event in the product. |
| **H-4** | Lints. `#[must_use]` on a fn returning `io::Result` trips `clippy::double_must_use` (in `clippy::all`). `unused_qualifications` warns on a full path to an imported name. `missing_debug_implementations` on new test stubs. | No `#[must_use]` on `set_mouse_capture`. `#[must_use]` on `App::wants_mouse` and `DetailRegistry::wants_mouse` (house style: `DetailRegistry::captures_input`, `:223-226`). Trait methods stay un-annotated (as `DetailTab::captures_input`). Import `MouseEvent`/`MouseEventKind`/`MouseButton` where used; `terminal.rs` keeps writing `crossterm::event::EnableMouseCapture` in full, as its paste helpers do (nothing imported there). Test stubs derive `Debug`. |
| **H-5** | Any-motion reporting (`?1003h`, `crossterm-0.29.0/src/event.rs:325-333`) wakes the loop once per cell the pointer crosses. | `Moved`, `ScrollLeft`, `ScrollRight` return from `App::on_mouse` before dispatch, before the status is touched, with `dirty` untouched; `set_mouse_capture` writes nothing when unchanged. Pinned by `motion_and_the_horizontal_wheel_never_reach_a_tab_or_redraw`. |
| **H-6** | The source-shape test's `body()` (`terminal.rs` tests) ends an item at `"\n    fn "`, `"\npub fn "`, `"\nfn "` or `"\nimpl "`, **not** at `"\n    pub fn "`. | `set_mouse_capture` goes **last** in `impl TerminalGuard`, so `restore`'s body (it already runs to `"\nimpl "`) still contains `restore_terminal()`. `"fn enable_mouse_capture()"` is not a substring of `"fn disable_mouse_capture()"`, and `"enable_mouse_capture"` is not a substring of `"disable_mouse_capture"`. `code()` strips comments, so docs may name `EnableMouseCapture` freely. |
| **H-7** | `tests/panic_hook_order.rs` reads `terminal.rs` and forbids `ratatui::init`, `ratatui::try_init`, `init_with_options`, `set_panic_hook` (`:55-74`). | None of the new code names them. |
| **H-8** | Mouse coordinates in tests. rataflow's `Rect::contains_point` is inclusive on all four sides (`types/geometry.rs:199-204`): a 20×5 node at world `(x, y)` is hit on `x..=x+20`, `y..=y+5`, so the first gap row under a node is still that node. | Click a node's **interior**, `corner_of(…) + (2, 2)`, never its corner. An edge cell is `corner_of(upper) .1 + 6` (the gap rows are `+5..=+7`; `+5` hits the node, `+8` the next one). A blank cell is ≥2 columns left of every node (`(2, row)` at 43 wide: nodes start at column 11) and is asserted blank before use. |
| **H-9** | A view hit-tests against the frame it last drew (rataflow `ui/canvas.rs:37` `set_canvas_area`; `RunsTab::canvas`). | Unit tests draw (`draw`/`draw_at`/`lines`) before the first mouse event. Integration tests call `harness.render()` first; `Harness::mouse`'s doc says so. |
| **H-10** | A stale `canvas` after the pane stops drawing the flow (list, artifact, no runs, no item). | `RunsTab::render` sets `self.canvas.set(None)` as its **first** statement; only `render_flow` sets it back. The event loop redraws after every dirty step, so the next event sees the current frame's canvas. |
| **H-11** | D7 and a click: the click moves the cursor, so the next `sync` queues `Reveal::Cursor`, and `ensure_node_visible` nudges a partly visible clicked node fully on screen. | Accepted: the cursor moved, which is D7's trigger. Minimal pan only (`rataflow` `state/viewport.rs:287-320`). |
| **H-12** | Sibling branches add trait defaults whose docs count ("the trait's third default", `registry.rs:79-89`). | The new docs carry no ordinal (plan Risks). Appended at the **end** of each trait. |
| **H-13** | `RunsTab` clears `gesture` but rataflow's `drag_state` may still be `Panning`/`AwaitingNodeClick` after D11 ends a gesture. | D11 accepts it: the next `Down` overwrites `drag_state` (`on_mouse_down`), and `Drag`/`Up` without a live gesture never reach rataflow. E1 keeps the selection right in the meantime. |
| **H-14** | A terminal that answers `Unsupported` would be re-asked on every loop step if the flag only moved on success. | `set_mouse_capture` records `on` whenever the helper returns `Ok` (which includes `Unsupported`, tolerated inside it). A real error returns before the flag moves; the loop ends and `lib.rs` restores. |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles) | Gate (`--test-threads=1` on every test run) |
|---|---|---|---|
| T1 capture lifetime | `crates/htui/src/terminal.rs` | 2 (§2.6) | `cargo test -p htui --all-features --lib terminal -- --test-threads=1`; `cargo test -p htui --all-features --test panic_hook_order --test panic_hook -- --test-threads=1`; clippy |
| T2 event seam | `ui/tabs/registry.rs`, `ui/tabs/backlog/detail/mod.rs`, `ui/tabs/backlog/mod.rs`, `app/state.rs`, `app/update.rs` (tests), `testkit.rs`, `event_loop.rs` | 2 (§3.8) | `cargo test -p htui --all-features --lib app:: -- --test-threads=1`; `cargo test -p htui --all-features --lib ui::tabs::backlog:: -- --test-threads=1`; `cargo build -p htui`; clippy |
| T3 flow gestures | `Cargo.toml`, `runs/execution_graph.rs`, `runs.rs`, `tests/backlog.rs` | 3 (§4.8) | `cargo test -p htui --all-features --lib detail::runs -- --test-threads=1`; `cargo test -p htui --all-features --test backlog -- --test-threads=1`; `cargo insta pending-snapshots` empty; `git ls-files crates/htui/tests/snapshots \| wc -l` = **143**; `git diff --stat Cargo.lock` empty; clippy |
| T4 close-out | `docs/decisions/mod/mod-71.md` (new), `DECISIONS.md`, `HANDOFF.md`, `docs/ANA-12.md`, `docs/decisions/mod/mod-28.md`, the plan's status | 1 (§5) | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | — | §7, on the real tree |

clippy is `cargo clippy --workspace --all-targets --all-features -- -D warnings` throughout.

---

## 2. T1: capture lifetime in the terminal (D2, D3)

**File**: `crates/htui/src/terminal.rs` only.

### 2.1 Module doc (after the bracketed-paste paragraph, `:16-21`)

```text
//!
//! **Mouse capture** (MOD-71 D1-D3) is on only while the view on screen wants the mouse, because
//! it takes the terminal's own text selection away. The event loop asks the app after every step,
//! and [`TerminalGuard::set_mouse_capture`] writes only a change. [`init`] and `Suspend::enter`
//! never turn it on; every way the terminal is given back — [`restore_terminal`] and
//! `Suspend::leave` — turns it off first, whatever the guard last set.
```

### 2.2 `TerminalGuard` (`:25-32`) and `init` (`:60-63`)

New field, last:

```rust
    /// MOD-71 D2: whether mouse capture is on, as this guard last set it. Only
    /// [`TerminalGuard::set_mouse_capture`] turns it on; `Suspend::leave` clears it (D3).
    mouse: bool,
```

`init`'s literal gains `mouse: false,` after `restored: false,` with
`// MOD-71 D1: capture starts off; the loop turns it on for a view that wants it.`
`init` issues no mouse command.

### 2.3 `restore_terminal` (`:72-78`)

```rust
/// Gives the terminal back: mouse capture off (MOD-71 D3), bracketed paste off (MOD-22 review
/// M-1), then `ratatui::restore` — raw mode off and the alternate screen left. Best effort, as
/// `ratatui::restore` is: a stdout that cannot take one escape sequence is not a reason to stop
/// giving the rest back. A free function, so it cannot know whether capture is on: it turns it
/// off unconditionally, which a terminal that never had it ignores.
pub fn restore_terminal() {
    let _ = disable_mouse_capture();
    let _ = disable_bracketed_paste();
    ratatui::restore();
}
```

### 2.4 `impl TerminalGuard` (`:118-131`): `set_mouse_capture`, **last** (H-6)

```rust
    /// MOD-71 D2: mouse capture on or off, written only when `on` differs from what this guard
    /// last set, so the event loop can ask after every step for nothing. The loop is the only
    /// caller, with `App::wants_mouse` (D1). A terminal without mouse reporting answers
    /// `Unsupported`, which is recorded as done so it is not asked again every step: the view runs
    /// keyboard-only (review R2-L6's shape, H-14).
    ///
    /// # Errors
    ///
    /// A terminal write that failed for any other reason: the loop ends and `lib.rs` restores
    /// (MOD-9 D21).
    pub fn set_mouse_capture(&mut self, on: bool) -> std::io::Result<()> {
        if on == self.mouse {
            return Ok(());
        }
        if on {
            enable_mouse_capture()?;
        } else {
            disable_mouse_capture()?;
        }
        self.mouse = on;
        Ok(())
    }
```

No `#[must_use]` (H-4: `io::Result` already is). `restore` is not changed: after it nothing draws.

### 2.5 `impl Suspend for TerminalGuard` (`:133-155`) and the helpers (after `disable_bracketed_paste`, `:178-185`)

```rust
    /// Show the cursor (every draw hid it), mouse capture off (MOD-71 D3) and bracketed paste off
    /// so the editor gets its own mouse and paste (MOD-22 review M-1), then `ratatui::try_restore`
    /// (MOD-9 D22). `restored` is not touched: this is a pause, not the end. `mouse` is cleared
    /// before the write, so whatever happens next the loop's `set_mouse_capture` re-asserts what
    /// the app wants once the editor is gone — one place decides.
    fn leave(&mut self) -> std::io::Result<()> {
        self.terminal.show_cursor()?;
        self.mouse = false;
        disable_mouse_capture()?;
        disable_bracketed_paste()?;
        ratatui::try_restore()
    }
```

`enter`'s body is unchanged. Its doc gains one sentence: "Mouse capture is not turned back on
here (MOD-71 D3): the loop's next `set_mouse_capture` decides, in one place." A failed `leave`
followed by `run_suspended`'s re-`enter` (`editor.rs:270-277`) leaves `mouse == false`, and the
loop re-enables if wanted.

```rust
/// MOD-71 D2: mouse capture on, in its own `execute!` (review R2-L6's shape). crossterm's
/// `EnableMouseCapture` is any-motion reporting (`?1003h`, `crossterm-0.29.0/src/event.rs:325-333`),
/// which `App::on_mouse` drops before it can cost a redraw (D4). `Unsupported` is a terminal with
/// no mouse reporting, which runs keyboard-only.
fn enable_mouse_capture() -> std::io::Result<()> {
    let enabled = tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture
    ))?;
    if !enabled {
        tracing::debug!("this terminal has no mouse reporting; the flow view is keyboard-only");
    }
    Ok(())
}

/// Mouse capture off, in its own `execute!`, `Unsupported` tolerated as on the way in (MOD-71 D3).
fn disable_mouse_capture() -> std::io::Result<()> {
    tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture
    ))
    .map(|_| ())
}
```

`debug!`, not `info!` (E9): it fires on every toggle on such a terminal, unlike the paste notice,
which fires once at `init`.

### 2.6 Tests (first) and commits (T1)

A **new** test beside the paste one (E10), using the same `code()` and `body()` helpers:

```rust
    /// MOD-71 D3: capture is the loop's alone to turn on, and every way the terminal is given back
    /// turns it off, first and whatever the guard last set. A path that forgot leaves the shell, or
    /// `$EDITOR`, printing an escape sequence for every mouse move.
    #[test]
    fn every_give_back_disables_mouse_capture_and_only_the_loop_enables_it() {
        let code = code();
        let restore = body(&code, "pub fn restore_terminal()");
        let off = restore
            .find("disable_mouse_capture()")
            .expect("`restore_terminal` turns capture off");
        let paste = restore
            .find("disable_bracketed_paste()")
            .expect("`restore_terminal` turns paste off");
        assert!(off < paste, "capture goes first (D3)");
        let leave = body(&code, "fn leave(&mut self)");
        assert!(
            leave.contains("disable_mouse_capture()") && leave.contains("self.mouse = false"),
            "`leave` turns capture off and forgets it"
        );
        for taking in ["pub fn init()", "fn enter(&mut self)"] {
            let body = body(&code, taking);
            assert!(
                !body.contains("MouseCapture") && !body.contains("mouse_capture"),
                "`{taking}` leaves capture to the loop"
            );
        }
        let toggle = body(&code, "pub fn set_mouse_capture(&mut self, on: bool)");
        assert!(toggle.contains("enable_mouse_capture()") && toggle.contains("disable_mouse_capture()"));
        assert!(toggle.contains("self.mouse"), "the toggle writes only a change");
        for (helper, command) in [
            ("fn enable_mouse_capture()", "EnableMouseCapture"),
            ("fn disable_mouse_capture()", "DisableMouseCapture"),
        ] {
            let body = body(&code, helper);
            assert!(body.contains(command), "`{helper}` issues `{command}`");
            assert!(body.contains("tolerate_unsupported("), "`{helper}` tolerates an unsupported terminal");
            assert!(
                !body.contains("AlternateScreen") && !body.contains("BracketedPaste"),
                "`{helper}` issues nothing else"
            );
        }
    }
```

`init`'s body contains `mouse: false`, which matches neither `"MouseCapture"` nor `"mouse_capture"`.
The existing `every_path_that_takes_the_terminal_enables_bracketed_paste_and_every_restore_disables_it`
(`:226`) and `an_unsupported_paste_mode_is_tolerated_and_nothing_else_is` (`:270`) are **unchanged**
and stay green (H-6). `TerminalGuard` can't be built without a real tty (`DefaultTerminal`), so the
idempotence is pinned by shape, not by a behaviour test.

1. `test(mod-71): the mouse half of terminal.rs's source-shape test (red)`: the test only. It
   compiles and panics at the first `expect`.
2. `feat(mod-71): TerminalGuard owns mouse capture, and every give-back turns it off`: §2.1–§2.5.
   Gate: the T1 row of §1. `set_mouse_capture` is `pub` on a `pub` type in `pub mod terminal`
   (`lib.rs:39`), so it raises no `dead_code` before T2 calls it.

