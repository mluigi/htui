# Plan: MOD-57 M1 — the external editor in the TUI pane

**Source PRD**: `.claude/prds/mod-57-embedded-editor-pane.prd.md`
**Selected Milestone**: 1 — Editor in the pane
**Complexity**: Large
**Routing**: PRD path, ultracode for implement and review (`/handoff-run MOD-57`, accepted 2026-10-07).
**Status**: DRAFT — fact-checked, awaiting CONFIRM.

## Summary

With an opt-in preference, `E`/`Ctrl+E` stop suspending the TUI. They open `$VISUAL`/`$EDITOR` on a
pseudo-terminal, drawn over the requesting view's editing area while the rest of htui keeps
drawing. Every key goes to the editor, `ctrl-c` included, except one catalogue action that toggles
focus between the editor and htui. While unfocused, M1 locks htui down to that toggle, an abort
action and quitting (M2 opens the rest of htui). When the editor exits, the file is read back
through the same code as today and the same `ExternalEditOutcome` reaches the same
`Tab::on_external_edit`, so no view's outcome handling changes.

## Decisions (proposed; confirmed at the CONFIRM gate)

| # | Decision | Why |
|---|---|---|
| P1 | **One seam, not five.** The pane is owned by `App`, opened by the event loop's existing `$EDITOR` post-step, and drawn by `App::render` after the active tab and before the status line and overlays. Views keep emitting `Action::EditExternally` and keep receiving `on_external_edit` unchanged. | The handoff already has exactly one dispatch point (`event_loop.rs:55-66`, `state.rs:431-506`); the five call sites (Templates, Library, item form, Documents, Notes) need no new logic. |
| P2 | **Where it draws (PRD D1).** During render a view *claims* its editing rect through `Ctx` (`ctx.claim_editor_area(rect)`, writing a `Cell<Option<Rect>>` that `App` clears before each frame). `App` draws the pane over the claimed rect when the active tab is the one that asked; if nothing was claimed, or the rect is under the minimum (`MIN_PANE` = 40×8, PRD Q3: tuned by a manual nvim run in the Notes compose box), it uses the whole tab body. The four draw sites: `compose::render` (Notes and Documents share it), `item_form::render`, `TemplatesView::render_draft` and `LibraryView::render_draft`. | `Tab::render` is `&self` and `item_form::render` gets no `Ctx`, so a claim through a `Cell` is the smallest change that lets the view name its own rect; nothing else in the views moves. |
| P3 | **The preference (PRD D2, Q2) is an environment variable, `HTUI_EDITOR_PANE`** (`1`/`true`/`yes` → in-pane; anything else or unset → suspend), resolved next to `$VISUAL`/`$EDITOR` through `EditorCommand::resolve`'s `lookup` closure. | It is per OS user and machine, sits where the user already chooses the editor, works under `--demo` and `--offline`, needs no store row, file format or Settings section, and keeps out of `keys/` and the config-root loader that MOD-67 M2 is writing. A Settings toggle can follow without changing the mode logic. |
| P4 | **Keys (ANA-26 §6.4, PRD Q6).** A new catalogue context `editor` (`Context::Editor`, table `editor`) with two actions: `editor.focus` (default `ctrl-4`, the chord a `ctrl-\` press arrives as; `ctrl-\` itself is refused as indistinguishable, `chord.rs:582`) toggles focus both ways; `editor.abort` (default `ctrl-x`, offered only while unfocused) kills the editor and changes nothing. A test pins that `editor.focus` is bound. MOD-67 M2's loader must refuse a file that unbinds it, as it does `overlay.close` (merge-order note, below). | The PRD says the leave chord is never hard-coded. One toggle keeps PRD D3's "the same action refocuses". `ctrl-\` is the editor chord least likely to collide (nano's replace also has `alt-r`); the user can rebind it. |
| P5 | **`ctrl-c` (ANA-26 C3, PRD D5).** In `App::on_key`, *before* the C3 check: if an in-pane editor is alive **and focused**, the key goes to the pane (focus toggle or forward). Unfocused, C3 holds: `ctrl-c` quits at once and the editor is killed on drop. | This is the one exception ANA-26 declared, already anticipated in `state.rs:723-724`'s comment. |
| P6 | **Unfocused, M1 lock.** With the editor alive and unfocused, only `editor.focus`, `editor.abort`, `global.quit`, `global.help` and `ctrl-c` act. Every other key is refused with a status line naming the two editor keys (generated through `Keys::label`). A second `Action::EditExternally` while a pane is alive is refused with `EDITOR_BUSY`. Quitting in M1 kills the editor without a confirm; M2 adds the confirm (PRD D5) and opens tabs. | The asking view's state must not move under the editor: a `Ctrl+S` or a selection change would save stale text or misroute the outcome. Locking everything is the M1-safe superset of M2's per-tab lock. |
| P7 | **Transport.** A fourth `select!` arm in `event_loop::run` over an unbounded `PaneEvent` channel (`Output(Vec<u8>)`, `Exited(ExitStatus or error)`) fed by a std reader thread and a std wait thread per pane. Bytes are parsed on the UI task into the pane's `vt100::Parser`. Writes to the PTY are synchronous and small (one key). Resize follows the drawn rect: `App::render` records the rect; the loop's post-step resizes the PTY and the parser when it changed. | The module doc's "a new transport is a new arm" holds; the other three arms are untouched. `R-NF-3`: nothing blocking runs on the UI task (the reader and wait threads block, the UI task never does). |
| P8 | **Read-back is shared.** `editor::run` is split: `TempEdit::create(text, stem)` (temp file, sanitised stem), and `TempEdit::finish(status, elapsed) -> ExternalEditOutcome` (start-failure codes, non-zero exit, UTF-8, `normalise_newlines`, `Unchanged{quick}`). `run` (suspend) and the pane call the same two. Abort answers `Failed("the editor was aborted; nothing was changed")`. | PRD: "the file read back through the same parse gate". The views' `strip_added_newline`/`without_controls` already run inside `on_external_edit` and are untouched. |
| P9 | **The editor command on a PTY.** `EditorCommand::pty_command(file) -> portable_pty::CommandBuilder`, with the same argv as `command()` (`sh -c 'exec <value> "$1"' htui-editor <file>` on unix; `cmd /S /C "<value> "<file>""` on Windows), `TERM=xterm-256color`, and the pane's size. | One resolution rule for both modes; the unix `exec` reasons in `command()`'s doc carry over unchanged (the child is htui's own process, killed on abort). |
| P10 | **Dependencies (PRD Q1): `portable-pty 0.9.0` and `vt100 0.16.2`, no features; not `tui-term`.** htui draws the screen with its own widget (T3). | Probe below: one ratatui in the tree, licences MIT/Apache/BSD-2, Windows cross-check green. `tui-term` adds nothing a 60-80 line widget does not, and gets wide cells wrong (next row). |

## Dependency decision

Probe run 2026-10-07 in `/tmp/mod57-probe` (a scratch crate pinning htui's ratatui 0.30.2, crossterm
0.29, tokio 1.53.1); vim-tiny 9.1 and nano 7.2 extracted locally, since the sandbox has no editor.

| Crate | Version | Licence | Verdict |
|---|---|---|---|
| `portable-pty` | 0.9.0 (2025-02-11; wezterm repo active 2026-10) | MIT | **add** |
| `vt100` | 0.16.2 (2025-07-12; quiet, 265 dependents) | MIT | **add** |
| `tui-term` | 0.3.4 | MIT | **not added**: own widget (below) |
| `vt100-ctt` | 0.17.1 | MIT | rejected: pulls ratatui 0.29 |

- **Lock impact**: seven new names (`portable-pty`, `vt100`, `serial2`, `downcast-rs`,
  `shared_library`, `winreg`, plus `vte 0.15` beside insta's `vte 0.14`) and a third `nix` (0.28).
  No second ratatui (`cargo tree -d --target all`).
- **Widths**: `vt100` and `ratatui-core` both use `unicode-width 0.2.2`, but `vt100` measures per code
  point and ratatui per cell string: a VS16 emoji (`❤️`) and a halfwidth sound mark (`ｱﾞ`) are 1
  column in the VT grid and 2 in ratatui, whose diff then skips the next cell and leaves it stale.
  The own widget sets `Cell::set_diff_option(CellDiffOption::ForcedWidth(..))` from `is_wide()`,
  skips `is_wide_continuation()` cells, resets every cell it covers, and places the **real**
  cursor (`Frame::set_cursor_position`), which `tui-term` does not (it draws a glyph and relies on a
  cleared buffer).
- **Runtime (Linux, real time)**: vim-tiny and nano draw at 24x80, redraw after a resize to 30x100,
  survive `0x03` (`ctrl-c` is SIGINT to the child only: own session via `setsid`+`TIOCSCTTY`), save and
  exit 0 with the file read back; reader sees EOF on exit; `alternate_screen()` tracks.
- **Process gotchas** (T2 must honour): reader/writer are blocking (dedicated `std::thread`); drop
  the slave right after spawn or EOF never comes; dropping the master while a reader clone lives
  orphans the child, so teardown calls `ChildKiller::kill` (`clone_killer()`, SIGHUP then escalation)
  before dropping; dropping the writer sends EOF to the child.
- **Terminal queries**: neither editor sent DA1/DSR/OSC queries; `vt100` answers none. nvim (not
  testable here) does query; T2 answers DSR (`ESC[6n`) and DA1 (`ESC[c`) through `vt100`'s
  callbacks, and the maintainer's manual nvim run in T7 is the check.
- **Windows**: `CC=gcc AR=ar cargo check --target x86_64-pc-windows-gnu` green; ConPTY not run
  (PRD D6 → MOD-16).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui/src/editor.rs:1-6`, `:41-47` | Module doc names the MOD/D numbers; plain nouns (`EditorCommand`, `ExternalEdit`), constants for user-facing sentences (`EDITED`, `NO_CHANGES`). |
| Errors | `crates/htui/src/editor.rs:208-276` | Never panic on a child failure: every failure is `ExternalEditOutcome::Failed(<one sentence>)`; temp file removed on every path; `kill_on_drop`. |
| Debug hygiene | `editor.rs:127-135`, `:151-163` | Text never `Debug`ged: lengths only. The pane's screen contents and the bytes are never logged. |
| Event loop | `crates/htui/src/event_loop.rs:33-75` | Arms call one `App` method each; post-steps after `should_quit`; a code-shape test pins step order (`the_loop_sets_mouse_capture_after_the_editor_and_before_the_draw`). |
| Keys | `crates/htui/src/keys/catalogue.rs` `row(..)`, `stack.rs` `Stack::OVERLAY` | One block per context with the arm it mirrors cited; stacks as `const` slices of `Layer`; tests in `keys/mod.rs` pin parse/collision/ctrl-c invariants. |
| Key dispatch | `crates/htui/src/app/state.rs:711-815` | `on_key` order: C3, overlay, tab, legacy keymap, base stack; the pane step goes before C3 (P5). |
| Drawing | `crates/htui/src/ui/cells.rs` (module doc) | Cell widths agree with ratatui's; the VT grid is copied cell by cell, a wide cell then its continuation skipped, never re-measured. |
| Tests (unit) | `crates/htui/src/app/update.rs:1145`, `:1151` | Fake tabs implementing `Tab` with recording `on_external_edit`; `App` driven by `on_key`/`update`. |
| Tests (snapshot) | `crates/htui/src/snapshots/` + `insta` | `TestBackend` frames snapshotted; a frame with the pane drawn over a view. |
| Tests (real child) | `editor.rs` tests near `:859` | Real-time tests with a scripted "editor" (`sh -c`) instead of a real editor; paused time never beside a real child (memory: paused time + real child). |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `Cargo.toml` (workspace deps) | UPDATE | the PTY and VT crates (P10) | T0 |
| `crates/htui/Cargo.toml` | UPDATE | the same, `{ workspace = true }` | T0 |
| `Cargo.lock` | UPDATE | resolved | T0 |
| `crates/htui/src/editor.rs` | UPDATE | P8 split (`TempEdit`), P9 `pty_command`, P3 `pane_mode` resolution, `EDITOR_BUSY`/abort sentences | T1 |
| `crates/htui/src/editor/pane.rs` *(or `crates/htui/src/pane.rs`; blueprint picks)* | CREATE | `EditorPane`: PTY spawn, reader/wait threads, `PaneEvent`, write/resize/kill, `vt100::Parser`; key encoder `encode(KeyEvent) -> Vec<u8>` | T2 |
| `crates/htui/src/ui/editor_pane.rs` | CREATE | the widget: VT screen → `Buffer` cells, cursor, frame/title with focus state and generated key labels | T3 |
| `crates/htui/src/ui/mod.rs` | UPDATE | `mod editor_pane` | T3 |
| `crates/htui/src/keys/catalogue.rs` | UPDATE | `Context::Editor`, `Act::EditorFocus`, `Act::EditorAbort`, rows | T4 |
| `crates/htui/src/keys/stack.rs` | UPDATE | `Stack::EDITOR_FOCUSED` (`editor ∩ {focus}`), `Stack::EDITOR_UNFOCUSED` (`editor`, `global ∩ {quit, help}`) | T4 |
| `crates/htui/src/keys/mod.rs` | UPDATE | test: `editor.focus` bound and not printable; re-exports | T4 |
| `crates/htui/src/app/state.rs` | UPDATE | pane state on `App`; `on_key` pane step (P5, P6); `on_paste` forwarding; `drain` refusal (`EDITOR_BUSY`); `render` draws the pane (P2); `open_pane`/`on_pane_event`/`pane_resize` | T5 |
| `crates/htui/src/app/state.rs` (`Ctx`, `:77`) | UPDATE | `Ctx::claim_editor_area` over the `Cell` | T5 |
| `crates/htui/src/event_loop.rs` | UPDATE | fourth arm; post-step chooses suspend or pane; resize post-step; shape test extended | T5 |
| `crates/htui/src/ui/tabs/backlog/detail/compose.rs` | UPDATE | one claim at the body rect | T6 |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | UPDATE | `render` returns or claims the body rect (its caller passes `Ctx`) | T6 |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | pass the claim through for the item form | T6 |
| `crates/htui/src/ui/tabs/skills/templates.rs` | UPDATE | one claim in `render_draft` | T6 |
| `crates/htui/src/ui/tabs/skills/library.rs` | UPDATE | one claim in `render_draft` | T6 |
| `crates/htui/src/snapshots/*.snap` | CREATE | pane frames (T3, T5) | T3, T5 |
| `docs/htui-editor.md` | CREATE | user guide: `$VISUAL`/`$EDITOR`, `HTUI_EDITOR_PANE`, the two keys, M1 lock | T7 |
| `README.md` | UPDATE | doc list line (`:557-558` style) | T7 |

## Tasks

TDD per repo convention: each task writes its failing tests first.

### T0 — Dependencies (main thread, before fan-out)
- **Action**: add `portable-pty = "0.9.0"` and `vt100 = "0.16.2"` to `[workspace.dependencies]` (with a comment in the file's style citing this plan) and `{ workspace = true }` to `crates/htui`; `cargo build -p htui`; commit `Cargo.toml`s and `Cargo.lock` alone.
- **Validate**: `cargo build -p htui` and the Windows cross-check (Validation).

### T1 — `editor.rs`: shared read-back, PTY command, mode (parallel with T2, T3, T4)
- **Action**: tests first: `TempEdit::finish` keeps every existing outcome (`run`'s current tests stay green unchanged); `pty_command` argv on unix matches `command()`'s; `pane_mode` over `resolve`-style lookup (`HTUI_EDITOR_PANE` unset, `1`, `true`, `0`, blank). Then the split, `pty_command`, the sentences.
- **Mirror**: `EditorCommand::resolve(lookup)`; `run`'s failure sentences.
- **Validate**: `cargo test -p htui --all-features editor::`

### T2 — `EditorPane` and key encoder (parallel with T1, T3, T4)
- **Action**: tests first. Encoder: table test for printable chars, Enter/Tab/Backspace/Esc, arrows, Home/End/PgUp/PgDn/Delete/Insert, F1-F12, `ctrl-<letter>` (`ctrl-c` → `0x03`), Alt as an `ESC` prefix, and the application-cursor-mode variant (`ESC O A` when the screen asks for it). Pane: real-time test with a scripted editor (`sh -c 'printf hi; read x; printf "%s" "$x" > "$1"'`-style) through `EditorPane::spawn` → `PaneEvent::Output` reaches the parser → write `"edited\r"` → `PaneEvent::Exited(success)` → `TempEdit::finish` answers `Edited`; resize reaches the child (`stty size` script); `kill` ends the child and `Exited` follows; dropping the pane leaves no child (pid probe); the slave is dropped after spawn (EOF arrives); a DSR `ESC[6n` from the child gets a cursor-position reply and DA1 `ESC[c` a VT220 reply. Then the implementation.
- **Mirror**: `editor::run`'s `kill_on_drop` and temp-file guarantees.
- **Validate**: `cargo test -p htui --all-features pane::` (unix); Windows cross-check compiles it.

### T3 — the widget (parallel with T1, T2, T4)
- **Action**: own widget, no `tui-term` (P10). Tests first: a `vt100::Parser` fed fixed bytes (text, SGR colours, a CJK wide char, a VS16 emoji and `ｱﾞ`, which must not leave the following cell stale across two frames, and the cursor) drawn into a `TestBackend` buffer, snapshotted; the real cursor position is set; the frame title shows focus state and the labels from `Keys::label`; a rect smaller than the screen clips without panic. Then the widget.
- **Mirror**: `ui::cells` rules (wide cell and continuation), `theme` styles for the frame.
- **Validate**: `cargo insta test -p htui --all-features -- editor_pane` then review.

### T4 — catalogue (parallel with T1, T2, T3)
- **Action**: tests first: the existing `keys/mod.rs` invariants cover the new rows (parse strictly, no shared defaults within `editor`, no `ctrl-c`); new test that `editor.focus` is bound and non-printable; stack tests for both editor stacks. Then the rows and stacks.
- **Mirror**: `Stack::OVERLAY`; catalogue block with the cited reason.
- **Validate**: `cargo test -p htui --all-features keys::`

### T5 — shell integration (after T1-T4)
- **Action**: tests first, with a fake pane behind a small trait so no child runs in `App` tests: focused pane forwards `ctrl-c` and every non-toggle key, toggle unfocuses; unfocused `ctrl-c` quits; unfocused toggle refocuses; abort answers `Failed` to the asking tab only; other keys refused with the status sentence; a second `EditExternally` refused with `EDITOR_BUSY`; paste forwarded (bracketed when the screen asks); `Exited` routes the outcome through `finish_external_edit`; render draws over the claimed rect, falls back to the body when unclaimed or under `MIN_PANE`, draws nothing when another tab is active; resize recorded on a rect change. Event-loop shape test extended (pane arm present; post-step order: editor step, resize, capture, draw). Then the code.
- **Validate**: `cargo test -p htui --all-features app::` and `event_loop::`

### T6 — the four claims (after T5)
- **Action**: tests first, one per site: rendering the view with an edit pending claims the expected rect (via a probe `Ctx`). Then the one-line claims and the item form's plumbing.
- **Validate**: `cargo test -p htui --all-features` + `cargo insta test` (no existing snapshot changes expected: a claim draws nothing).

### T7 — e2e and docs (after T6)
- **Action**: one real-time integration test (`tests/`, `--features testkit`) driving `App` + a real `EditorPane` with the scripted editor end to end from `Ctrl+E` in the Templates view to the edited body in the view. Docs: the user doc section for `$EDITOR` gains `HTUI_EDITOR_PANE` and the two keys. Manual run notes for nvim/vim/nano (PRD metric) recorded in the plan.
- **Validate**: full gate.

### Independence (file-set intersection)

| Task | Files |
|---|---|
| T1 | `editor.rs` |
| T2 | `editor/pane.rs` (new) |
| T3 | `ui/editor_pane.rs` (new), `ui/mod.rs`, its snapshots |
| T4 | `keys/catalogue.rs`, `keys/stack.rs`, `keys/mod.rs` |
| T5 | `app/state.rs` (incl. `Ctx`), `event_loop.rs`, its snapshots |
| T6 | `compose.rs`, `item_form.rs`, `backlog/mod.rs`, `templates.rs`, `library.rs` |
| T7 | `tests/editor_pane.rs` (new), user doc |

T1-T4 are pairwise disjoint and run in parallel. T2 declares `mod pane;` inside `editor.rs` (T1's file): T0 adds that one line, and an empty `editor/pane.rs`, before the fan-out. Same for T3's `ui/mod.rs` line. T5 depends on all four; T6 on T5; T7 on T6.

**MOD-67 merge order.** T4 appends to the files MOD-67 M2-M5 rewrite. The new context is one appended block and two appended stacks, so the expected conflict is textual. Whichever of MOD-57 and MOD-67 M2 lands second adds `editor.focus` to the loader's must-stay-bound rule beside `overlay.close` (ANA-26 §7.4 step 6). The phase note records this.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate
cargo test -p htui --all-features -- --test-threads=1        # scheduling-dependent suite
cargo insta test -p htui --all-features --check               # every snapshot, not a grep
CC=gcc AR=ar cargo check -p htui --target x86_64-pc-windows-gnu
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| VT fidelity: nvim draws wrongly (colours, alternate screen, cursor shape) or waits on a query | Medium | vim-tiny and nano verified by the probe; DSR/DA1 answered (T2); manual nvim run in T7; suspend stays the default |
| Width mismatch (VS16 emoji, halfwidth sound marks) leaves stale cells | Measured | `ForcedWidth` from `is_wide()`; T3 two-frame test |
| `vt100` unreleased since 2025-07 | Low | Small API surface used (parser, screen, cells, callbacks); swap cost is the widget plus parser calls |
| Third `nix` version in the lock | Certain | Accepted: build time only; recorded |
| A reader/wait thread outlives the pane, or a child is left running | Medium | Threads end on EOF/exit; `kill` on drop of `EditorPane`; T2's no-child test; T5's quit test |
| Key encoding gaps (Alt-chords, shifted function keys) | Medium | T2 table test; legacy encoding only (kitty protocol out of scope) |
| `cmd /S /C` quoting through `CommandBuilder` differs from `raw_arg` on Windows | Medium | Cross-check compiles only; real behaviour is MOD-16's list (PRD D6); recorded in docs |
| MOD-67 M2-M5 conflicts in `keys/` | High | Appended blocks only; merge-order note |
| Paused-time tests beside a real child misfire | Medium | Real-time for every test that spawns; fake pane in `App` tests |
| `ctrl-4` collides with a user's editor binding | Low | Configurable through MOD-67 M2's `keys.toml`; documented |

## Acceptance

- [ ] All tasks complete, TDD order kept
- [ ] Validation passes (every command above)
- [ ] With `HTUI_EDITOR_PANE` unset, every existing test and snapshot is unchanged (suspend is the default)
- [ ] Patterns mirrored, not reinvented
- [ ] Manual run: nvim, vim, nano in the pane (draw, keys, resize, `ctrl-c` forwarded, exit, read-back)

## Verified claims (plan fact-check)

Step 3.5, 2026-10-07. Every claim checked against the tree or by compile/run probe; none falsified.

| Claim | Verdict | Evidence |
|---|---|---|
| One dispatch point for the handoff: the loop's post-step and `App::drain` | TRUE | `event_loop.rs:55-66`; `state.rs:431-457` (`drain`), `:463-470` (`take_external_edit`), `:474-506` (`finish_external_edit`) |
| Five emitting views and the overlays refused | TRUE | `Action::EditExternally` text search: `templates.rs:651,830`, `library.rs:837,1236`, `backlog/mod.rs:511`, `documents.rs:222`, `notes.rs:197`; `update.rs:36` refuses outside a tab |
| Four draw sites; Notes and Documents share `compose::render` | TRUE | `notes.rs:395`, `documents.rs:592` call `compose::render`; `item_form.rs:997`; `templates.rs:1174`, `library.rs:1782` (`render_draft`) |
| `Tab::render` is `&self`; `item_form::render` has no `Ctx` | TRUE | `registry.rs` trait (`fn render(&self, frame, area, ctx)`); `item_form.rs:997` `(frame, area, form, theme)` |
| `App::render` is `&mut self` and draws tab → status → overlays → help | TRUE | `state.rs:837-878` |
| `on_key` checks C3 first and anticipates MOD-57's exception | TRUE | `state.rs:719-726` |
| `App::on_paste` exists to forward pastes | TRUE | `state.rs:532` |
| `ctrl-\` is refused; `ctrl-4` (what it arrives as) parses strictly | TRUE | `chord.rs:582` (refusal table); `refuse_char`/`legacy_arrival` (`chord.rs:230-300`) refuse no digit |
| `ctrl-x` and `ctrl-4` are unbound in the catalogue | TRUE | no `"ctrl-x"`/`"ctrl-4"` in `keys/catalogue.rs` |
| `Keys::label` gives a generated key label | TRUE | `keys/hint.rs:85` |
| `Ctx` lives in `app/state.rs` (not `app/mod.rs`) | AMENDED | `state.rs:77`; the Files table's `app/mod.rs / ctx` row means `state.rs` (T5 owns it either way) |
| No user doc covers `E`/`Ctrl+E` | TRUE | `README.md:557-558` lists `docs/htui-mcp.md`, `docs/htui-secrets.md`; no `docs/htui-*.md` for the editor; T7 creates `docs/htui-editor.md` and its README line |
| ratatui 0.30.2 / crossterm 0.29 in the workspace | TRUE | `Cargo.toml:66-67` |
| `portable-pty 0.9.0` + `vt100 0.16.2` add no second ratatui; Windows cross-check green | TRUE | probe `cargo tree -d --target all`; `cargo check --target x86_64-pc-windows-gnu` 6.17 s |
| `vt100` has callbacks for unhandled CSI (DSR/DA1 replies) | TRUE | `vt100-0.16.2/src/callbacks.rs:55` `unhandled_csi(.., params, c)` (replies are buffered by the callback struct, written by the pane) |
| `CellDiffOption::ForcedWidth` and `Frame::set_cursor_position` exist | TRUE | `ratatui-core-0.1.2/src/buffer/cell.rs:31`; `terminal/frame.rs:166` |
| `portable-pty` exposes `clone_killer`, blocking reader/writer | TRUE | `portable-pty-0.9.0/src/lib.rs:97,102,157` |
| `ctrl-c` (0x03) reaches only the child; editors survive it | TRUE | probe: child sid=pgid=pid; vim-tiny and nano alive after 0x03 |
| `cargo-insta` and the Windows gnu target are installed | TRUE | `cargo-insta 1.48.0`; `rustup target list --installed` |
| No `deny.toml` licence policy to satisfy | TRUE | none in the repo; all new crates MIT / Apache / BSD-2 |
| T1-T4 file sets are disjoint | TRUE | Independence table: `editor.rs` / `editor/pane.rs` / `ui/editor_pane.rs`+`ui/mod.rs` / `keys/*`; the two `mod` lines are T0's |
