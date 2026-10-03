# Blueprint: MOD-74 mouse follow-ups

**Status**: proposed (2026-10-03). This blueprint adds plan deviations B-1 to B-7 (§0), hazards H-1 to H-18 (§0a) and blueprint decisions E1 to E6 (§6). The plan's D1-D7 still bind; where a row narrows one, it cites the evidence.

**Plan**: `.claude/plans/mod-74-mouse-follow-ups.plan.md` at `b983778b`, confirmed by the maintainer and fact-checked (25 claims).

**Verified at**: HEAD `b983778b`, branch `hr/MOD-74`, clean tree. Source was read through Gortex (`read`, `search`, `relations`). Dependencies were read directly from `~/.cargo/registry/src/index.crates.io-*/{rataflow-0.1.0,crossterm-0.29.0}`. Line numbers are from before any edit, so they shift once a task commits.

**Coupling verdict**: the plan's order holds. Run serially T1 → T2 → T3 → T4 with one implementer and no fan-out. T1 touches only `terminal.rs` (B-6 keeps it that way). T2 and T3 share `runs.rs` and `execution_graph.rs`.

**Scope**: no store change, no migration, no `.sqlx`, no crate, no `Cargo.lock` change. New code:
- `EnableButtonMouseCapture`
- `App::{mouse, mouse_capture, lose_mouse}`
- `Tab::on_mouse_lost` and `DetailTab::on_mouse_lost` (both defaulted)
- `DetailRegistry::on_mouse_lost`, plus the `BacklogTab` forward
- `RunsTab::{end_gesture, on_mouse_lost}`
- `ExecutionGraph::{pan, end_gesture, locked_left}`, plus the test accessor `is_dragging`

**House style (carried from MOD-71)**:
- Lints are `clippy::all` (warn) plus rust `unsafe_code = forbid`, `missing_debug_implementations` and `unused_qualifications`. rustdoc denies broken and private intra-doc links.
- **`clippy::pedantic` is not enabled** (`Cargo.toml` `[workspace.lints.clippy]`: "deliberately NOT enabled"). The brief assumed it was.
- The gate is `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- Every commit compiles. Stage only your own paths: no `-A`, no `stash`, no `--amend`.
- Every test run uses `--all-features` and `--test-threads=1`.
- Comments cite the decision (`// MOD-74 D6: …`). Render paths never panic or log.

---

## Design Decisions

- **D1/D2 edge in `App`, not the loop.** `App::mouse` is the answer the loop last applied. `mouse_capture()` is the only writer on the loop's path. `take_external_edit()` is the only other writer, and runs exactly when the loop is about to suspend.
- **D3 defaulted hooks.** They mirror MOD-71's `wants_mouse`/`on_mouse`. The fan-out reaches **every** tab and **every** sub-tab, and nothing gates it (H-12).
- **D4 one end path.** `RunsTab::end_gesture` → `ExecutionGraph::end_gesture` → a locked left release only if rataflow is dragging. The four D11 sites call it, and so does a fifth site the plan missed (B-1).
- **D5 private command** written as a literal ANSI string, placed after `tolerate_unsupported` so the source-shape test stays meaningful (H-3).
- **D6 `pan` assigned on every left press, not only set** (B-1). It is updated on drags of a live pan and cleared on release and in `end_gesture`. The re-anchor runs after `frame.render_widget`, so no frame's buffer changes (H-11).
- **E1-E6** (§6) are blueprint-level choices: test placement, the tolerance helper, the head-line test, and the red-commit shapes.

## Files to Create

| File | Purpose | Priority |
|---|---|---|
| `docs/decisions/mod/mod-74.md` | T4 write-up (mirror `mod-71.md`) | T4 |

## Files to Modify

| File | Changes | Task |
|---|---|---|
| `/home/mluigi/projects/htui/crates/htui/src/terminal.rs` | `EnableButtonMouseCapture`; `enable_mouse_capture` switched to it; docs; 2 new tests and 1 updated shape test | T1 |
| `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/registry.rs` | `Tab::on_mouse_lost` (default) | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/backlog/detail/mod.rs` | `DetailTab::on_mouse_lost` (default); `DetailRegistry::on_mouse_lost` | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/backlog/mod.rs` | `BacklogTab::on_mouse_lost`; `MouseProbe` loss counter; 1 test | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/app/state.rs` | `mouse` field, `mouse_capture`, `lose_mouse`, D2 in `take_external_edit`; docs | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/app/update.rs` | `Pointer` loss counter, `pointing` returns 5 values; 4 tests | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/event_loop.rs` | the loop line, module doc, comment; shape test | T2 |
| `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/backlog/detail/runs.rs` | `end_gesture`, `on_mouse_lost`, the 4+1 sites, field doc; tests | T2, T3 |
| `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | `end_gesture`, `locked_left`, `is_dragging` (test); `pan`, `on_mouse`/`render` changes, module doc; tests | T2, T3 |
| `DECISIONS.md` (repo root), `HANDOFF.md`, the plan | close-out | T4 |

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `b983778b` | Fix |
|---|---|---|---|---|
| **B-1** | Non-blocker (it loses a click) | D6: `pan` is "set on a left press whose response carries `PaneClicked`"; D4 replaces "the four bare `self.gesture = false` sites". | `RunsTab::on_mouse`'s `Down(Left)` arm does `self.gesture = inside;` (`runs.rs:1414`), a fifth write that can end a live gesture without telling rataflow. If a release is ever missed (the button released somewhere the terminal does not report), `pan` stays `Some`. A later **node** press then leaves `AwaitingNodeClick` live, the next draw's re-anchor turns it into `Panning`, and the release emits no `NodeClicked`. | (1) `ExecutionGraph::on_mouse` **assigns** on every `Down(Left)`: `self.pan = pane_clicked.then_some(cell)`. (2) The `Down(Left)` arm in `RunsTab::on_mouse` calls `self.end_gesture()` first when `self.gesture` is set, then `self.gesture = inside`. Tests (c′) and `a_press_off_the_canvas_ends_a_live_gesture`. |
| **B-2** | **Blocker** (the T3 gate errors) | T3 Validate: `cargo test -p htui --all-features execution_graph runs::`. T1 and T2 have no `--test-threads=1`. | Cargo takes one `TESTNAME` before `--`, so a second one is an argument error (MOD-71 B-4 again). The suite depends on scheduling (memory). | Use the gates in §1: `--lib`, one filter before `--` or several after it, and always `--test-threads=1`. |
| **B-3** | Non-blocker | T4: `docs/DECISIONS.md`. | The index is `DECISIONS.md` at the repo root. Its newest-first rows start at `:5`. | T4 edits `/home/mluigi/projects/htui/DECISIONS.md`. |
| **B-4** | Non-blocker (wrong pattern cited) | Patterns: "`DetailRegistry` forwards to sub-tabs (`on_paste` shape)". | `on_paste` goes to the **active** sub-tab only (`detail/mod.rs:~266`). The every-sub-tab shape is `DetailRegistry::on_item_change` (`:220-224`). | `on_mouse_lost` copies `on_item_change`'s loop: no active check, no `wants_mouse` gate (H-12). |
| **B-5** | Non-blocker (a mitigation that does not exist) | Risks: "MOD-16 already carries Windows capture verification". | HANDOFF's MOD-16 entry (`HANDOFF.md:396`) never mentions the mouse. Only `mod-71.md` "Carried" asks for it. | T4 adds to the MOD-16 entry: capture on Windows Terminal and the legacy console, the `$EDITOR` handoff before the flow view (H1's path), and **a `cargo check` of the `#[cfg(windows)]` delegation from D5**. |
| **B-6** | Non-blocker (stale docs) | T1 updates "the module doc and `enable_mouse_capture`'s doc" only. | More docs name the old behaviour: `TerminalGuard::set_mouse_capture` (`terminal.rs:147-148`, "with `App::wants_mouse` (D1)"); `App::wants_mouse` ("which the event loop turns into mouse capture"); `App::on_mouse` (`state.rs:518-519`, "capture is any-motion, `?1003h`"); the module doc of `event_loop.rs` and its comment at `:67-69`. | The `terminal.rs` docs go in **T1**. They are plain backticks, not intra-doc links, so they compile before `App::mouse_capture` exists, and T1 stays file-disjoint. The `state.rs` and `event_loop.rs` docs go in **T2**. |
| **B-7** | Non-blocker (a test that cannot be written as placed) | T3 red tests "in `execution_graph.rs` / `runs.rs`" read `pan`. | `pan` is a private field of `ExecutionGraph`. `runs.rs` is the **parent** module and cannot see it. Only `execution_graph::tests` (the child) can. | Tests (a)-(e) live in `execution_graph.rs`. `runs.rs` gets one green-on-write guard for the `RefCell` render path (E3). |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | The `RefCell` borrow shape. `RunsTab::render` takes `&self`; `render_flow` holds `self.graph.borrow_mut()` (`runs.rs:~1603`) and calls `ExecutionGraph::render(&mut self)`. | D6 lives entirely inside `ExecutionGraph::render`, so no new borrow appears. Every input path (`end_gesture`, `on_mouse_lost`, the four D11 sites) is `&mut self` and uses `self.graph.get_mut()`. Tests read through `pane.graph.borrow()` only between calls. Holding a borrow across `&mut pane` will not compile anyway. |
| **H-2** | The `App` destructuring (`let Self { scope, …, tabs, .. } = self;` in `on_mouse`/`on_key`/`on_paste`/`finish_external_edit`). | `lose_mouse` needs no `Ctx`. It is a plain `for tab in self.tabs.iter_mut() { tab.on_mouse_lost(); }`, called from `mouse_capture` and `take_external_edit` and never inside a destructuring block. Every existing destructure ends in `..`, so the new field breaks none of them. `App::new` (`state.rs:221-252`) is the only struct literal: add `mouse: false`. |
| **H-3** | The source-shape helper `body()` in `terminal.rs` ends an item at `"\n    fn "`, `"\npub fn "`, `"\nfn "` or `"\nimpl "`, but not at `struct`/`#[derive]`. If the new struct sits right after `enable_mouse_capture`, that helper's window contains `struct EnableButtonMouseCapture;` and the updated assertion passes vacuously. If the `impl` sits before `disable_mouse_capture`, the Windows delegation's `crossterm::event::EnableMouseCapture` lands inside a checked window. | Put the `struct` and its `impl crossterm::Command` **after `fn tolerate_unsupported` (`:263-269`) and before `#[cfg(test)]` (`:271`)**. Note that `"EnableButtonMouseCapture"` does not contain `"EnableMouseCapture"`, so the existing pair at `:402` fails until it is updated. That failure is T1's red. |
| **H-4** | `tests/panic_hook_order.rs` reads `terminal.rs` and assumes no string literal contains `//`. It forbids `ratatui::init`, `ratatui::try_init`, `init_with_options` and `set_panic_hook`. | The ANSI literal has no `//`. Nothing new names the forbidden calls. Gate `--test panic_hook_order`. |
| **H-5** | The shape test in `event_loop.rs` (`:93-112`) searches for the exact loop line and requires `set_mouse_capture(` exactly once. | The search string becomes `term.set_mouse_capture(app.mouse_capture())?;`. The count stays 1, because `app.mouse_capture()` does not contain `set_mouse_capture(`. Add `assert!(!code.contains("app.wants_mouse()"))` so the loop cannot bypass the edge. The order editor < capture < draw is unchanged. |
| **H-6** | The `#[cfg(windows)]` code in D5 cannot be compiled here: no Windows target, and the sandbox is offline. | Copy crossterm's signatures exactly (`fn execute_winapi(&self) -> std::io::Result<()>`, `fn is_ansi_code_supported(&self) -> bool`). Delegate with full paths, `crossterm::Command::execute_winapi(&crossterm::event::EnableMouseCapture)`. Do **not** `use crossterm::Command`: on Linux that import would be unused and fail `-D warnings`. Do not use `crossterm::csi!`, which is `#[doc(hidden)]`. The compile check is carried to MOD-16 (B-5). |
| **H-7** | Lints. `missing_debug_implementations` (derive `Debug` on the command; the test probes already derive it). `unused_qualifications` (`terminal.rs` imports nothing from `std::fmt`/`crossterm`, so full paths are right). rustdoc `private_intra_doc_links` = deny: a `pub` item's doc must not link `[`ExecutionGraph`]` or `[`RunsTab::end_gesture`]`, which are private, so use plain backticks. `dead_code` in red commits (see §1). | Linking `pub` items (`Tab::on_mouse_lost`, `App::mouse_capture`) is fine. |
| **H-8** | Float tolerance. The existing `assert_panned` (`execution_graph.rs:1759`) compares with an **absolute** `f64::EPSILON` (2.2e-16). After a wheel zoom the offset is fractional (about −12.1), and one ulp there is about 1.8e-15. | D6 tests use a new `assert_near(now, (x, y, zoom))` with `1e-9`, the tolerance the wheel test already uses (`:1836`). Leave `assert_panned` alone: its integer cases are exact. |
| **H-9** | `locked` leaks as `true`. Every later press would pan, and no click would work. | One helper, `locked_left`: save, set, call `handle_mouse_event`, restore, with no `?` or early return in between. Locked `Down`/`Up` cannot panic (`event_handlers.rs:496-509`). Tests assert `!graph.flow.locked` after each re-anchor and each `end_gesture`, and a node click still answers its step afterwards. |
| **H-10** | A locked release over `AwaitingNodeClick` must emit nothing. An unlocked release emits `NodeClicked` (`mouse.rs:846-847`). | `end_gesture` always goes through `locked_left`. A later unlocked `Up` with `DragState::None` returns `Handled` and no events (`mouse.rs:~925`). Test `ending_a_node_press_clicks_nothing`. |
| **H-11** | The three `runs_flow_*` snapshots (`backlog__runs_flow_{fanout,reject_note,tool_chips}.snap`) must not move. | The re-anchor runs **after** `frame.render_widget` and only while `pan.is_some() && flow.is_dragging()`. It rewrites `drag_state` only, never the viewport or the buffer. No snapshot test presses the mouse. `Harness` never calls `mouse_capture`, so D1 does not affect `tests/*.rs`. The only integration drag test (`tests/backlog.rs:983`) does press, drag and release with no render in between. Gate: `git diff --stat main -- crates/htui/tests/snapshots Cargo.lock` is empty. |
| **H-12** | A loss that gets gated away. A form opening **is** a loss, the inactive Runs pane **is** the one holding the gesture, and the pane no longer wants the mouse at the moment it is told. | `BacklogTab::on_mouse_lost` has **no** `form`/`item_form` guard (unlike its `on_mouse`). `DetailRegistry::on_mouse_lost` loops over all sub-tabs with no `wants_mouse` check. `RunsTab::on_mouse_lost` ends the gesture whatever `wants_mouse` says. The backlog test runs with the filter form open and the probe inactive. |
| **H-13** | A double or missing broadcast around `$EDITOR`. | The loop calls `take_external_edit()` (D2 loss, `mouse = false`) **before** `mouse_capture()` in the same step (`event_loop.rs:55-70`). The off-edge then reads `false → wants` with no second broadcast. `leave` turns capture off only `if self.mouse` (guard), and `guard.mouse == app.mouse` holds: both record `Unsupported` as on. |
| **H-14** | The `Pointer` probe returns a 4-tuple from `pointing` (`update.rs:1317-1337`), used at 6 call sites (`:1364`, `:1376`, `:1391`, `:1406`, `:1421` twice). | Add `type Lost = Rc<Cell<usize>>;`. `pointing` returns a 5-tuple ending in `Lost`, and the 6 sites gain `_lost`. The alias keeps `clippy::type_complexity` (in `clippy::all`) quiet; if it still fires, alias the whole tuple. `mouse_probe` (`backlog/mod.rs:1322`) does the same: a 4-tuple, 2 existing call sites. |
| **H-15** | Trait-default doc ordinals ("the trait's third default", `registry.rs:79-89`). | The new docs carry no ordinal. Append at the **end** of each trait, after `on_mouse`. |
| **H-16** | `ExecutionGraph::clear` (`:552-561`) leaves rataflow's `drag_state` and `pan` alone, and `set_nodes` does not reset `drag_state` either. | Accepted, with no change. A sync to no run, or to a run with no steps, makes `RunsTab::wants_mouse` false, so the loop's off-edge ends the gesture. A sync to a new run is a `Reset` reveal, and D6 re-anchors after it. |
| **H-17** | Review L4's `Consumed`/`Pass` decision compares the viewport before and after a forwarded event (`runs.rs:1424-1433`). | The re-anchor is in `render`, not `on_mouse`, so L4 is untouched. Re-anchoring at a cell outside the canvas (a drag carried past the pane edge) is safe: `terminal_to_canvas` uses `i32` arithmetic (`render_context.rs:74-79`). |
| **H-18** | A rataflow-side refit mid-pan (`=` key, `apply_pending_fit_view` inside the widget render, `ui/canvas.rs:39`) or a resize reveal. | Both land before the re-anchor, so the pan continues from what is on screen. The same mechanism as D6 needs no extra code. |

---

## 1. Build order, commits and gates

| Step | Commit (each compiles; message ends with the `Co-Authored-By` line) | Gate (`--test-threads=1` always) |
|---|---|---|
| T1 red | `test(mod-74): button-only capture command and its source-shape pair (red)` | the tests compile and fail |
| T1 green | `feat(mod-74): mouse capture reports buttons, not motion (D5)` | `cargo test -p htui --all-features --lib terminal -- --test-threads=1`; `cargo test -p htui --all-features --test panic_hook_order -- --test-threads=1`; clippy |
| T2 red | `test(mod-74): the capture-lost hook, its probes and the loop's shape (red)` | compiles, fails |
| T2 green | `feat(mod-74): a lost capture ends the flow's gesture (D1-D4)` | `cargo test -p htui --all-features --lib -- --test-threads=1 app:: event_loop ui::tabs::backlog::`; clippy |
| T3 red | `test(mod-74): a live pan across a re-read, a wheel tick and a moved canvas (red)` | compiles, fails |
| T3 green | `feat(mod-74): a live pan re-anchors on every draw (D6)` | `cargo test -p htui --all-features --lib detail::runs -- --test-threads=1`; `cargo test -p htui --all-features --test backlog -- --test-threads=1`; `cargo insta pending-snapshots` empty; `git diff --stat main -- crates/htui/tests/snapshots Cargo.lock` empty; clippy |
| T4 | `docs(mod-74): close-out - write-up, DECISIONS index, HANDOFF status, MOD-16 note, plan done` | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | the plan's Validation block, on the real tree; grep the output for `SIGABRT` |

**Red-commit shapes.** These avoid `dead_code` under `-D warnings` and never route a product path to `todo!()`:
- **T1 red**:
  - add the `struct` with `#[cfg_attr(not(test), expect(dead_code, reason = "MOD-74 T1 red: wired in the green commit"))]`;
  - `write_ansi` is `todo!("MOD-74 T1 green")` with the parameter named `_f`;
  - the `#[cfg(windows)]` items are already final;
  - `enable_mouse_capture` stays on crossterm's command.
- **T2 red**:
  - the trait defaults are final;
  - `DetailRegistry::on_mouse_lost` has an **empty** body, and `BacklogTab::on_mouse_lost` forwards to it;
  - `App::mouse` is added. `mouse_capture` does `self.mouse = self.wants_mouse(); self.mouse`, recording with no broadcast, so the field is read;
  - `take_external_edit` is unchanged;
  - `ExecutionGraph::end_gesture` has an **empty** body, and `ExecutionGraph::is_dragging` (test) is final;
  - `RunsTab::end_gesture` is final and the four D11 sites call it;
  - there is **no** `RunsTab::on_mouse_lost` override yet, and the B-1 `Down` change waits for green;
  - the loop line is unchanged.
- **T3 red**:
  - the `pan` field with `None` in `Default`, under `#[cfg_attr(not(test), expect(dead_code, reason = "MOD-74 T3 red: read by render in the green commit"))]`;
  - nothing writes or reads it yet. Green removes the `expect`.

---

## 2. T1: button-only reporting (D5)

**File**: `crates/htui/src/terminal.rs` only.

### 2.1 Docs (B-6)
- **Module doc**, the "Mouse capture" paragraph (`:23-27`): add that capture is button-event reporting (`?1000h ?1002h ?1015h ?1006h`, MOD-74 D5). There is no any-motion mode (`?1003h`), so the terminal sends no hover stream. Windows keeps crossterm's WinAPI path.
- **`set_mouse_capture` doc** (`:146-148`): "The loop is the only caller, with `App::mouse_capture` (MOD-74 D1), which also tells the tabs when capture goes off."
- **`enable_mouse_capture` doc** (`:236-240`): it issues `EnableButtonMouseCapture` (MOD-74 D5), crossterm's sequence minus `?1003h`. `App::on_mouse` still drops `Moved`, because Windows reports it and a terminal may ignore the narrower mode. `Unsupported` still means keyboard-only.

### 2.2 `enable_mouse_capture` (`:241-249`)
```rust
fn enable_mouse_capture() -> std::io::Result<()> {
    let enabled = tolerate_unsupported(crossterm::execute!(
        std::io::stdout(),
        EnableButtonMouseCapture
    ))?;
    // unchanged tail
}
```
`disable_mouse_capture` stays on `crossterm::event::DisableMouseCapture`: a superset that also clears `?1003l` (D5, MOD-71 D3).

### 2.3 The command, **after `tolerate_unsupported`, before `#[cfg(test)]`** (H-3)
```rust
/// MOD-74 D5: crossterm's `EnableMouseCapture` (`crossterm-0.29.0/src/event.rs:321-345`) minus
/// any-motion `?1003h`: press/release (`?1000h`), drag (`?1002h`), RXVT coordinates past 223
/// (`?1015h`, kept for terminals without SGR) and SGR (`?1006h`). `DisableMouseCapture` clears
/// every one of them. On Windows it is crossterm's own WinAPI command, `is_ansi_code_supported`
/// false as crossterm's is, so the console path never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableButtonMouseCapture;

impl crossterm::Command for EnableButtonMouseCapture {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        crossterm::Command::execute_winapi(&crossterm::event::EnableMouseCapture)
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}
```

### 2.4 Tests (red first), in `mod tests`
Helper: `fn ansi(command: impl crossterm::Command) -> String` writes `write_ansi` into a `String`. Bring the method into scope with `use crossterm::Command as _;`.

| Test | Asserts | Red because |
|---|---|---|
| `button_capture_reports_presses_drags_and_releases_and_no_motion` (new) | `ansi(super::EnableButtonMouseCapture) == "\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h"` (32 bytes) and `!contains("?1003h")` | `todo!()` panics |
| `the_disable_clears_every_mode_button_capture_sets` (new) | every `?NNNNh` in the enable output (split on `"\x1b["`) has `"\x1b[?NNNNl"` in `ansi(crossterm::event::DisableMouseCapture)` | `todo!()` panics |
| `every_give_back_disables_mouse_capture_and_only_the_loop_enables_it` (updated, `:346`) | pair `("fn enable_mouse_capture()", "EnableButtonMouseCapture")`, plus `!body.contains("event::EnableMouseCapture")` on the enable helper. The rest is unchanged. | the helper still issues crossterm's command |

---

## 3. T2: the capture-lost hook (D1-D4)

### 3.1 `Tab` (`registry.rs`), appended after `on_mouse` (`:98-103`)
```rust
/// MOD-74 D1, D3: mouse capture went off while this tab may hold a gesture: an overlay, `?`, a
/// form, a sub-tab or tab switch, or an `$EDITOR` handoff. Told to every registered tab, active
/// or not, once per on-to-off edge. Defaulted to nothing, so no other tab changes.
fn on_mouse_lost(&mut self) {}
```

### 3.2 `DetailTab` and `DetailRegistry` (`detail/mod.rs`)
- `DetailTab`, after `on_mouse` (`:113-117`): `fn on_mouse_lost(&mut self) {}`. The doc says the Backlog tab hands every sub-tab the loss (MOD-74 D3). Only `[`RunsTab`]` holds a gesture (already linked in this trait's docs, so the link is allowed).
- `DetailRegistry`, after `on_mouse` (`:276-283`):
```rust
/// MOD-74 D3: a lost capture, to **every** sub-tab, as [`on_item_change`](Self::on_item_change)
/// goes (B-4): a sub-tab switch is itself a loss, so the pane holding the gesture is no longer
/// the active one. No `wants_mouse` gate: the pane told no longer wants it.
pub fn on_mouse_lost(&mut self) {
    for tab in &mut self.tabs {
        tab.on_mouse_lost();
    }
}
```

### 3.3 `BacklogTab` (`backlog/mod.rs`), after `on_mouse` (`:589-594`)
```rust
/// MOD-74 D3: to the detail pane, unconditionally (H-12): a form opening is one of the losses.
fn on_mouse_lost(&mut self) {
    self.detail.on_mouse_lost();
}
```

### 3.4 `App` (`app/state.rs`)
- **Field**, after `pending_edit` (`:212`):
  ```rust
  /// MOD-74 D1: mouse capture as the loop last applied it, the edge `mouse_capture` detects.
  /// `take_external_edit` clears it for the editor's `leave` (D2).
  pub(super) mouse: bool,
  ```
  In `App::new`, `mouse: false`.
- **`mouse_capture`**, after `wants_mouse` (`:509-514`):
  ```rust
  /// MOD-74 D1: what the event loop hands `TerminalGuard::set_mouse_capture` after every step:
  /// [`wants_mouse`](Self::wants_mouse), recorded. On an on-to-off edge every registered tab is
  /// told ([`Tab::on_mouse_lost`]): after a tab switch the tab holding the gesture is no longer
  /// the active one.
  pub fn mouse_capture(&mut self) -> bool {
      let wants = self.wants_mouse();
      if self.mouse && !wants {
          self.lose_mouse();
      }
      self.mouse = wants;
      wants
  }

  /// MOD-74 D1, D2: tells every tab capture went off.
  fn lose_mouse(&mut self) {
      for tab in self.tabs.iter_mut() {
          tab.on_mouse_lost();
      }
  }
  ```
  No `#[must_use]`: the loop uses the value, and the method has a side effect. `wants_mouse` keeps its `#[must_use]` and stays a pure query that `on_mouse` gates on.
- **`take_external_edit`** (`:372-375`), D2:
  ```rust
  /// The edit a tab asked for, taken by the event loop (MOD-9 D10). `Some` exactly when the loop
  /// is about to suspend, and `Suspend::leave` turns capture off without the app knowing: so a
  /// capture that was on is lost here (MOD-74 D2), and the loop's next `mouse_capture` re-enables
  /// it with no stale gesture.
  pub fn take_external_edit(&mut self) -> Option<(TabId, ExternalEdit)> {
      let edit = self.pending_edit.take();
      if edit.is_some() && self.mouse {
          self.lose_mouse();
          self.mouse = false;
      }
      edit
  }
  ```
- **Docs (B-6)**:
  - `wants_mouse`: "which [`mouse_capture`](Self::mouse_capture) records and the event loop applies after every step".
  - `on_mouse` (`:518-519`): "`Moved` (Windows still reports motion, and a terminal may ignore button-only mode, MOD-74 D5)".

### 3.5 Event loop (`event_loop.rs`)
- `:70` becomes `term.set_mouse_capture(app.mouse_capture())?;`.
- The comment at `:67-69` gains "MOD-74 D1: `mouse_capture` also tells the tabs when capture goes off."
- The module doc (`:5-7`) names `App::mouse_capture` in plain backticks.

### 3.6 `RunsTab` (`runs.rs`)
- **`end_gesture`**, private, in the first `impl RunsTab` after `sync_graph` (`:395-407`):
  ```rust
  /// MOD-74 D4: the one way a gesture ends: the pane's flag and rataflow's drag state together,
  /// so neither outlives the other (`RunsTab::gesture`, `ExecutionGraph::end_gesture`).
  fn end_gesture(&mut self) {
      self.gesture = false;
      self.graph.get_mut().end_gesture();
  }
  ```
- **Sites**:
  - `:1280` (`on_item_change`), `:1337` (`v`), `:1361` (capturing action) and `:1405` (`on_mouse` not wanted) each call `self.end_gesture()`. Comments become `// MOD-71 D11, MOD-74 D4`.
  - **B-1**, `:1413-1416`:
    ```rust
    MouseEventKind::Down(MouseButton::Left) => {
        if self.gesture {
            self.end_gesture();
        }
        self.gesture = inside;
        inside
    }
    ```
- **Override**, after `on_mouse` (`:1403-1434`):
  ```rust
  /// MOD-74 D3: capture went off above the pane; a held button must not resume the old anchor.
  fn on_mouse_lost(&mut self) {
      self.end_gesture();
  }
  ```
- **Field doc** (`:223-226`): "… `v`, an item change, a capturing mode, a press off the canvas and a lost capture (MOD-74 D3) end it, through `end_gesture` (D4)."

### 3.7 `ExecutionGraph` (`execution_graph.rs`), T2 half
- After `on_mouse` (`:591-608`):
  ```rust
  /// MOD-74 D4: ends a live gesture without a click. A pan's drag state, or a node press's
  /// `AwaitingNodeClick`, gets a **locked** left release (`event_handlers.rs:496-509`): it
  /// resets the drag state and emits nothing, where an unlocked one would click the node.
  pub(super) fn end_gesture(&mut self) {
      // T3 adds: self.pan = None;
      if self.flow.is_dragging() {
          self.locked_left(MouseEventKind::Up(MouseButton::Left), (0, 0));
      }
  }

  /// MOD-74 D4, D6: one left-button event fed to rataflow with `locked` set: a press starts
  /// `Panning` at the pointer with no hit test, a release only ends the drag. The lock is put
  /// back as it was, with no early return between (plan Risks; the flow never renders
  /// rataflow's `Controls`, the only other reader, D7).
  fn locked_left(&mut self, kind: MouseEventKind, (column, row): (u16, u16)) {
      let locked = self.flow.locked;
      self.flow.locked = true;
      let _ = self.flow.handle_mouse_event(MouseEvent {
          kind,
          column,
          row,
          modifiers: KeyModifiers::NONE,
      });
      self.flow.locked = locked;
  }
  ```
  Add `KeyModifiers` to the `crossterm::event` import (`:16`). The test module's explicit `KeyModifiers` import still compiles next to `use super::*`, as `BTreeSet` already does.
- Test accessor, next to `zoom` (`:672`):
  `#[cfg(test)] pub(super) fn is_dragging(&self) -> bool { self.flow.is_dragging() }`, documented "Whether rataflow holds a drag, for the tests."

### 3.8 Tests (red first)

**`app/update.rs`**, MOD-71 mouse section (`:1279+`).
- `Pointer` gains `lost: Lost` and `fn on_mouse_lost(&mut self) { self.lost.set(self.lost.get() + 1); }`.
- `pointing` returns `(App, UnboundedReceiver<RequestEnvelope>, Rc<Cell<bool>>, Pointed, Lost)`, and the 6 existing sites gain `_lost` (H-14).

| Test | Fixture | Asserts |
|---|---|---|
| `only_an_on_to_off_edge_is_a_lost_capture` | `pointing(false, Consumed)` | `mouse_capture()` is false and `lost == 0`. `wants.set(true)`: true, true, `lost == 0`. `wants.set(false)`: false, `lost == 1`. Again: false, still `1`. |
| `an_overlay_or_the_help_box_opening_is_a_lost_capture` | `pointing(true, …)` and `Popup` | `mouse_capture()` true. `push_overlay(Popup)`, then `mouse_capture()` false and `lost == 1`. Fresh shell: `help_visible = true`, then false and `lost == 1`. |
| `a_tab_switch_tells_the_tab_it_left` | `pointing(true, …)` plus `Asker::new("asker")` registered | `mouse_capture()` true. `update(Action::Tab(TabAction::Focus(TabId("asker"))))`, then `mouse_capture()` false and `lost == 1`, with Pointer now inactive. |
| `an_editor_handoff_under_capture_is_a_lost_capture` | `pointing(true, …)`, `asked()` | `mouse_capture()` true. `app.pending_edit = Some((TabId("pointer"), asked()))`. `take_external_edit().is_some()`, `lost == 1`, `!app.mouse`. Next `mouse_capture()` is true with `lost` still `1`. `take_external_edit()` with nothing pending is `None` and `lost` stays `1`. A fresh shell with capture never applied plus a pending edit gives `Some` and `lost == 0`. |

**`event_loop.rs`**: `the_loop_sets_mouse_capture_after_the_editor_and_before_the_draw`, updated per H-5.

**`backlog/mod.rs`**: `MouseProbe` gains `lost: Rc<Cell<usize>>` and the override. `mouse_probe` returns a 4-tuple (H-14).
- `a_lost_capture_reaches_every_sub_tab` (`#[tokio::test]`):
  - `Bench`, probes `a` (wants false, active) and `b` (wants true, inactive) in a `DetailRegistry` inside `BacklogTab { detail, ..bench.tab() }`;
  - `press(&mut tab, &bench, KeyCode::Char('f'))` opens the filter form;
  - `tab.on_mouse_lost()`, then both counters are `1` (H-12).

**`execution_graph.rs`**, new section "MOD-74 T2: ending a gesture". It reuses `synced`, `linear`, `draw`, `assert_blank`, `corner_of`, `mouse` and `DOWN`/`DRAG`/`UP`.

| Test | Asserts |
|---|---|
| `ending_a_pan_forgets_its_anchor` | Draw; press `(2,12)` (blank, asserted); drag to `(5,14)`; `end_gesture()`. Then `!is_dragging()` and `!flow.locked`. A bare `DRAG` to `(9,16)` leaves the viewport unchanged. |
| `ending_a_node_press_clicks_nothing` | Press the interior of `1.1 done` (`corner + (2,2)`); `end_gesture()`. `!is_dragging()`. `on_mouse(UP, same)` is `None`. `selected()` is still the cursor. `!flow.locked`. |
| `ending_no_gesture_changes_nothing` | Idle graph: `end_gesture()` leaves the viewport, the selection and `locked` unchanged. |

**`runs.rs`**, MOD-71 mouse section. It reuses `flowing`, `blank_cell`, `mouse`, `lines`, `driven`.

| Test | Asserts |
|---|---|
| `a_lost_capture_ends_a_live_pan` (new) | Press `blank_cell`; drag `+3,+2` gives `Consumed`; read `viewport()`; `pane.on_mouse_lost()`. Then `!pane.gesture`, `!pane.graph.borrow().is_dragging()`, the next drag is `Pass`, and the viewport is unchanged. |
| `a_press_off_the_canvas_ends_a_live_gesture` (new, B-1) | Press `blank_cell` (gesture live); press at `(0,0)`, the run line. That press is `Pass`, `!pane.gesture` and `!is_dragging()`. |
| `v_an_item_change_and_a_modal_end_a_live_gesture` (extended, `:4298`) | After each of the three ends, `assert!(!pane.graph.borrow().is_dragging())`. This is D4's red. |

---

## 4. T3: re-anchor a live pan (D6)

### 4.1 `ExecutionGraph` (`execution_graph.rs`)
- **Module doc** (`:8-12`): add "MOD-74 D6: a live pan re-anchors at its last pointer on every draw, so a re-read's shift, a canvas moved by the head and a wheel zoom all pan on from what is on screen."
- **Field**, after `drawn` (`:422`), with `pan: None` in `Default` (`:426-439`):
  ```rust
  /// MOD-74 D6: the terminal cell of a live pan's last pointer, `None` with no pan. Assigned by
  /// every left press (a pan only on rataflow's `PaneClicked`, `mouse.rs:263-273`; B-1),
  /// moved by its drags, cleared by its release and by `end_gesture`.
  pan: Option<(u16, u16)>,
  ```
- **`on_mouse`** (`:591-608`): after `let response = self.flow.handle_mouse_event(mouse);` and before the selection reset:
  ```rust
  let at = (mouse.column, mouse.row);
  match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) => {
          let pans = response
              .events()
              .iter()
              .any(|event| matches!(event, FlowEvent::PaneClicked { .. }));
          self.pan = pans.then_some(at);
      }
      MouseEventKind::Drag(MouseButton::Left) => {
          if let Some(pan) = self.pan.as_mut() {
              *pan = at;
          }
      }
      MouseEventKind::Up(MouseButton::Left) => self.pan = None,
      _ => {}
  }
  ```
  `events()` borrows; `into_events()` stays last, unchanged. An edge press is also `PaneClicked`, because edges are never hit.
- **`render`** (`:620-651`), after `frame.render_widget(&mut self.flow, area);`:
  ```rust
  // MOD-74 D6: rataflow's `Panning` keeps its own `initial_viewport` (`mouse.rs:818-825`), so a
  // sync's review-L2 shift, a canvas moved by the head or a wheel zoom would be overwritten by
  // the next drag. A locked press at the last pointer re-anchors it on what was just drawn,
  // with this frame's canvas origin. Deltas are whole cells: with nothing changed it is a no-op
  // up to float association.
  if let Some(at) = self.pan
      && self.flow.is_dragging()
  {
      self.locked_left(MouseEventKind::Down(MouseButton::Left), at);
  }
  ```
  The early `!drawable(area)` return skips it, which is correct because the render context did not move.
- **`end_gesture`**: add `self.pan = None;` as its first line and drop the T2 placeholder comment.

### 4.2 Tests (red first)
These go in `execution_graph.rs`, section "MOD-74 T3: a live pan re-anchors". They mirror the app's redraw after every `Consumed` event by calling `draw` between events.

New helpers:
- `fn assert_near(now: Viewport, (x, y, zoom): (f64, f64, f64))` with `1e-9` (H-8);
- `fn draw_in(graph: &mut ExecutionGraph, area: Rect) -> Buffer`: a 43×24 `TestBackend`, `graph.render(frame, area, frame.area())`, so the pane size is constant and no resize reveal fires.

| # | Test | Steps | Asserts | Today (red) |
|---|---|---|---|---|
| a | `a_re_read_mid_pan_pans_on_from_the_shifted_viewport` | Review-L2 fixture (`:1380`): `run(1, [step(1,0,1,0)])`, cursor `id(1)`. Draw; press `(2,12)` (asserted blank); drag `(5,14)`; draw; `pre = viewport`. Sync `after` (+3 candidates, same cursor); `post = viewport`; `assert!((post.x - pre.x).abs() > 1.0)` as a precondition so the test cannot pass vacuously. Draw; `corner0 = corner_of("0.1 done")`; drag `(7,15)`; draw. | `assert_near(viewport, (post.x+2, post.y+1, 1.0))`; `corner_of("0.1 done") == corner0 + (2,1)`; `!flow.locked` | viewport falls back to `pre + (2,1)`, so the jump equals the shift |
| b | `a_wheel_tick_mid_pan_keeps_its_zoom` | `linear(2)`. Draw; press `(2,12)`; drag `(5,14)`; draw; `v1`. `ScrollUp` at `(5,14)`; `z = viewport`; precondition `(z.x - v1.x).abs() > 0.05` (the zoom moved the offset). Draw; drag `(7,15)`. | `assert_near(viewport, (z.x+2, z.y+1, z.zoom))`, `z.zoom ≈ 1.2`; `!flow.locked` | plan probe: offset reverts to `v0 + delta` |
| c | `a_node_press_never_starts_a_pan` | `linear(2)`. Draw; press the interior of `1.1 done`. Then `pan == None`. Draw; drag `+5,+3`; draw; release. | Release is `Some(id(1))`; viewport and `positions` unchanged; `!flow.locked` | `pan` does not exist (compile red); with a set-only `pan` it guards against regression |
| c′ | `a_node_press_after_a_lost_release_forgets_the_old_pan` (B-1) | Press `(2,12)` blank (`pan` is `Some`, no release); press the `1.1` node interior. `pan == None`. Draw; release. | `Some(id(1))` | compile red, and a set-only implementation loses the click |
| d | `a_released_pan_is_not_re_anchored` | Press `(2,12)`; drag `(5,14)`; draw; release. `pan == None`, `!is_dragging()`. Draw; bare `DRAG` to `(9,16)`. | viewport unchanged; afterwards a click on the `1.1` interior is `Some(id(1))` (no lock leak, H-9) | compile red |
| e | `a_canvas_that_moves_mid_pan_does_not_jump` (E2) | `draw_in(Rect(0,0,43,23))`; press `(2,12)`; drag `(5,14)`; `draw_in` the same; `v1`. `draw_in(Rect(0,1,43,23))`, the head grew a line. Drag `(7,15)`. | `assert_near(viewport, (v1.x+2, v1.y+1, 1.0))` | today `y` is off by one row (the stale canvas origin), which is why D6 rejected re-anchoring only in `sync` |

`runs.rs` (E3, green on write): `a_redraw_mid_pan_keeps_the_pan_under_the_pointer`. Run `flowing`; press `blank_cell`; drag `+3,+2`; `lines(&pane, &shell)`, a render through `RefCell::borrow_mut` that re-anchors; drag `+5,+3`. The viewport equals `before + (5,3)` exactly (zoom 1, whole cells). After `Up`, `!is_dragging()`.

Existing tests that keep passing unchanged:
- `a_drag_on_empty_canvas_pans_and_moves_no_node`, `panned()` and every `drag()` caller: no draw between press and release;
- `a_node_press_dragged_away_is_still_a_click`: `pan` stays `None`;
- `a_re_read_after_a_pan_keeps_the_viewport`: the pan was released;
- `tests/backlog.rs::a_drag_pans_and_the_wheel_zooms_the_flow`;
- the three `runs_flow_*` snapshots (H-11).

---

## 5. T4: close-out (what it must record)

- **`docs/decisions/mod/mod-74.md`**, mirroring `mod-71.md`:
  - header with `R-TUI-1`/`R-TUI-4`;
  - origin: `mod-71.md` "Carried" (review L3 plus two nits);
  - artifacts: the plan and this blueprint (B-1-B-7, H-1-H-18, E1-E6);
  - routing: plan path, C1-C4 none, no ultracode, sandbox `hr/MOD-74`;
  - maintainer decisions;
  - commit list: T1-T3 red/green pairs, T4;
  - "What was built": the three fixes, plus the wheel-mid-pan finding the fact-check turned up;
  - D1-D7 in brief: D5's deviation from the item text (`?1015h` kept), B-1's fifth end site and `pan` assigned on every press, D7's `locked` toggle;
  - verification: gates, snapshots and `Cargo.lock` unchanged;
  - **Carried**: (1) the `#[cfg(windows)]` delegation in D5 is uncompiled here, so it goes to MOD-16; (2) `App::on_mouse` still drops `Moved` (Windows, terminals that ignore the narrower mode); (3) optional: a public cancel-drag or `drag_state` API upstream in rataflow would replace the `locked` toggle.
- **`DECISIONS.md`** (repo root, B-3): a newest-first row `- **[MOD-74](docs/decisions/mod/mod-74.md)** - Mouse follow-ups: capture loss, button-only reporting, a pan across a re-read (done, 2026-10-03)`.
- **`HANDOFF.md`**:
  - the MOD-74 item (`:286-296`) leaves the open list, as MOD-71's did, per `.claude/rules/workflow-docs.md`;
  - the "Current status" paragraph (`:17-24`) leads with MOD-74 done and drops "Follow-ups are MOD-74";
  - the open-items table row (`:612`): `MOD-N` 22 → 21, remove "MOD-74 mouse follow-ups";
  - the **MOD-16 entry** (`:396`) gains the capture checks (B-5).
- **The plan**: `**Status**: done 2026-10-03 (`docs/decisions/mod/mod-74.md`)`.
- Gate: `validate-workflow-docs.sh`.

---

## 6. Blueprint decisions

- **E1**: the `#[cfg(windows)]` delegation uses fully qualified trait-call syntax, with no imports (H-6).
- **E2**: test (e) pins the head-line case that D6's own "rejected" bullet relies on.
- **E3**: D6 tests sit in `execution_graph.rs` (B-7). `runs.rs` has one green-on-write `RefCell` path guard.
- **E4**: `assert_near` at `1e-9` for every D6 assertion. `assert_panned` stays for the integer cases.
- **E5**: red commits wire plumbing with empty bodies instead of `todo!()` on product paths (T2), and use `expect(dead_code)` only for T1's struct and T3's field.
- **E6**: `locked_left` restores the **previous** `locked`, not `false`, so a future caller that locks the flow keeps its lock.

## Data Flow

1. **Loss**: a key or step opens an overlay, `?`, a form, a sub-tab or a tab. The loop calls `app.mouse_capture()`: `wants_mouse()` is false while `mouse` is true, so every `Tab::on_mouse_lost` runs. `BacklogTab` → `DetailRegistry` (every sub-tab) → `RunsTab::end_gesture` → `gesture = false` plus `ExecutionGraph::end_gesture` (`pan = None`, locked release). Then `term.set_mouse_capture(false)`.
2. **`$EDITOR`**: `take_external_edit()` returns `Some` while `mouse` is true, so the same broadcast runs and `mouse = false`. `leave` disables capture, the editor runs, then `mouse_capture()` sees `false → wants` and re-enables with no stale gesture.
3. **Button-only**: on ANSI terminals crossterm writes `?1000h ?1002h ?1015h ?1006h`, so no `Moved` stream arrives. Windows uses the WinAPI path, as before.
4. **Live pan**: a press on empty canvas (`PaneClicked`) sets `pan`. A drag moves the viewport and updates `pan`, and the app redraws. At the end of `ExecutionGraph::render` a locked press at `pan` rewrites `Panning { anchor, initial_viewport: current }`. A re-read's L2 shift, a moved canvas origin or a wheel zoom therefore carries into the next drag. Release clears `pan`.

## Build Sequence

1. T1 red → T1 green (`terminal.rs`).
2. T2 red: the trait defaults, the registry/Backlog plumbing, `App::mouse` (recording only), `RunsTab::end_gesture` with the 4 sites, an empty `ExecutionGraph::end_gesture`, `is_dragging`, and the tests.
3. T2 green: `lose_mouse`, D2, the loop line, `RunsTab::on_mouse_lost`, B-1 in `RunsTab::on_mouse`, the real `ExecutionGraph::end_gesture` and `locked_left`, the docs.
4. T3 red: the `pan` field and tests (a)-(e) plus the `runs.rs` guard.
5. T3 green: `pan` set/update/clear in `on_mouse`, the re-anchor in `render`, `end_gesture` clearing `pan`, the module doc.
6. T4 docs, then the close gate on the real tree.
