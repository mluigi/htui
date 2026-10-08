# Blueprint: MOD-57 milestone 1, "the external editor in the TUI pane"

**Status**: proposed (2026-10-07). The findings in §0 (F-1 to F-12), the plan deviations in §13
(PD-1 to PD-9) and the blueprint decisions in §14 (B1 to B16) are proposed here. A finding marked
**Blocker** means the plan, read literally, leaves a red tree, fails its own named test, or cannot
be built as written. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-57-m1-editor-in-pane.plan.md` (CONFIRMED 2026-10-07). Its P1-P10,
T0-T7, file sets and Verified-claims table are authoritative except where §0 and §13 amend them.
**PRD**: `.claude/prds/mod-57-embedded-editor-pane.prd.md` (D1-D6, Q1-Q6). **Spec**:
`docs/ANA-26.md` §6.4 (C3, the MOD-57 exception), §7.3-§7.4.

**Verified at**: HEAD `abb8898e`, branch `hr/MOD-57`. Every `file:line` below was read at that HEAD
through Gortex (`read` source/file, `search` text/symbols). Crate facts were read from
`~/.cargo/registry/src/index.crates.io-*/{portable-pty-0.9.0,vt100-0.16.2,ratatui-core-0.1.2,
crossterm-0.29.0}` and the probe at `/tmp/mod57-probe/src/main.rs`. **Line numbers are pre-edit.**

**Scope**:
- **Order**: T0 (main thread) → T1 ∥ T2 ∥ T3 ∥ T4 (pairwise disjoint files) → merge and gate → T5
  → T6 → T7, serial on the merged tree. After each merge, re-run that task's gate on the real
  tree with `--test-threads=1`.
- **New public surface**: `htui::editor::{EditorMode, EditorExit, TempEdit, PANE_VAR, EDITOR_BUSY,
  EDITOR_ABORTED, EditorCommand::pty_command}`; `htui::editor::pane::{PaneId, PaneSize, PaneEvent,
  PaneChild, PtyChild, PaneScreen, encode_key, encode_paste}`; `htui::ui::editor_pane::render`;
  `htui::keys::{Context::Editor, Act::EditorFocus, Act::EditorAbort, Stack::EDITOR_FOCUSED,
  Stack::EDITOR_UNFOCUSED}`; `htui::app::{MIN_PANE, EDITOR_LOCKED}`, `App::{open_editor,
  on_pane_event, resize_editor, editor_open, editor_id}`, `Ctx::{with_editor_area,
  claim_editor_area}`.
- **Counts**: `Act` 41 → **43**, `Context` 7 → **8**, `CATALOGUE` 41 → **43** rows (§12).
- **No existing snapshot changes** (suspend stays the default; a claim draws nothing). New
  snapshots: 2 (T3) + 1 (T5).
- **Two new dependencies** (P10): `portable-pty 0.9.0`, `vt100 0.16.2`, no features (both crates'
  `default = []`). `zeroize`, `tempfile`, `unicode-width` are already htui dependencies.

**House style (carried from MOD-67's blueprint, still true at `abb8898e`)**:
- Lints: `unsafe_code = "forbid"` (so no `libc::kill`; every signal goes through portable-pty),
  `missing_debug_implementations`, `unused_qualifications`, `clippy::all`; `lib.rs`
  `#![warn(missing_docs)]`; rustdoc denies broken/private intra-doc links. `clippy::pedantic` off.
- Every `pub` item, field and variant has a doc comment; every `pub` type implements `Debug`.
- **Debug hygiene** (`editor.rs:127-163`): text and bytes are never `Debug`ged or logged, only
  lengths. New hazards here: `vt100::Screen` derives `Debug` **with its cell contents**
  (`vt100-0.16.2/src/screen.rs:54`), and `portable_pty::CommandBuilder` derives `Debug` **with the
  whole environment** (`cmdbuilder.rs:201`). Neither is ever `Debug`ged, logged or stored in a
  `Debug` struct (§11 H-5).
- **No intra-doc link from a T1/T2/T3/T4 item to another parallel task's item.** Name it in plain
  backticks. T5-T7 may link everything.
- Inline format args. `rustfmt.toml`: edition 2024, `max_width = 100`.
- Implementers commit incrementally, stage their own paths only (never `-A`, never `stash`, never
  `amend`). Every commit compiles and passes its task gate.
- The Gortex PreToolUse hook blocks shell reads of indexed source: read with Gortex `read`; if
  `Read`/`Edit` is blocked, use an anchored scripted replace (each anchor asserted to match
  once), then `cargo fmt`.

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree / crate at `abb8898e` | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (T6 cannot claim as written) | P2/T6: "one claim at the body rect" in `compose::render`, "one claim in `render_draft`"; only `item_form::render` is flagged as having no `Ctx`. | `compose::render(frame, area, compose, title, kinds, theme)` (`compose.rs:543`) has no `Ctx`; its callers do (`notes.rs:395`, `documents.rs:592`). `TemplatesView::render_draft(&self, frame, left, editor, theme)` (`templates.rs:1174`) and `LibraryView::render_draft` (`library.rs:1782`) have none either, nor do their only callers `render_editor` (`templates.rs:1159`, `library.rs:1770`); `render` has it (`templates.rs:517`, `library.rs:684`). | **PD-3**: `compose::render` and `item_form::render` **return the body `Rect`** and their callers claim it; `render_editor`/`render_draft` (private) take `ctx: &Ctx<'_>` in place of `theme`. T6's file set gains `detail/notes.rs` and `detail/documents.rs`. |
| **F-2** | Major (R-NF-3) | P7: "Writes to the PTY are synchronous and small (one key)." | A paste is unbounded (`App::on_paste` forwards it), and a stalled or stopped child stops reading: once the line discipline's input queue (4 KiB on Linux) is full, `write` on the master blocks. That would block the UI task. | **PD-2**: a writer thread per pane fed by a `std::sync::mpsc` channel of `Zeroizing<Vec<u8>>`. `PaneChild::write` only queues. |
| **F-3** | Major (child left running) | Plan "Process gotchas": "teardown calls `ChildKiller::kill` (`clone_killer()`, SIGHUP then escalation)". | The **cloned** killer is `ProcessSignaller` (`portable-pty-0.9.0/src/lib.rs:323-337`): `libc::kill(pid, SIGHUP)` and nothing else. Escalation (SIGHUP, 5 × 50 ms `try_wait`, then SIGKILL) exists only on the **owned** child (`impl ChildKiller for std::process::Child`, `lib.rs:340-373`), and it sleeps, so it must not run on the UI task. A pid-based signal sent after the wait thread reaped the child can also hit a reused pid. | **PD-7 / B5**: the wait thread owns the child, polls `try_wait` every 25 ms and listens on a kill channel; a kill request or a dropped sender makes **it** call the owned `kill` (escalating) and then `wait`. No `clone_killer`, no pid-based signal from the UI task. |
| **F-4** | Major (behaviour drift from suspend mode) | P9: "the same argv as `command()`". | `CommandBuilder` defaults the child's cwd to **`$HOME`** when none is set (`cmdbuilder.rs:501-507`); `std::process::Command` (suspend mode) inherits htui's cwd. It also copies the whole environment (`get_base_env`), so a stale `LINES`/`COLUMNS` there overrides the PTY size for editors that honour them. | **PD-6**: `pty_command` sets `cwd(std::env::current_dir())` and `env_remove("LINES")`, `env_remove("COLUMNS")`. |
| **F-5** | Major (Windows argv) | P9: Windows `cmd /S /C "<value> "<file>""`. | `CommandBuilder` quotes **every** argument on Windows (`append_quoted`, `cmdbuilder.rs:668-720`); it has no `raw_arg`. The nested quotes become `\"`, which `cmd` does not read. | **PD-6**: argv `["cmd", "/S", "/C", "<value> <file name>"]` with `cwd` = the temp file's directory. The file name is `htui-<sanitised stem>-<random>.md`: no space, no quote, so `append_quoted` wraps the one argument in a single pair that `/S` strips. A `<value>` containing `"` still breaks: MOD-16's verification list (§11 H-10). |
| **F-6** | Major (T1 ∥ T2 ∥ T3 ∥ T4 contract) | T2 test: "`PaneEvent::Exited(success)` → `TempEdit::finish` answers `Edited`". T3 test: "the frame title shows … the labels from `Keys::label`". | `TempEdit` is T1's (parallel to T2). The editor acts are T4's (parallel to T3). Either test, written as the plan words it, does not compile in its task. | **PD-4**: T2's real-child test asserts the file content directly; `TempEdit` end to end is T5 (fake child) and T7 (real child). **PD-5**: the widget takes a prepared title string; T5 builds it from `Keys::hint` and tests the labels. |
| **F-7** | Minor (latency) | P7/T5: post-step order "editor step, resize, capture, draw". | The claimed rect is only known **after** `App::render`. A terminal resize redraws with the new rect, but the PTY learns it only at the next loop wake (the 250 ms tick at worst), and the editor redraws a tick late. | **PD-1**: editor step → capture → draw → **`app.resize_editor()`**. The shape test pins it. |
| **F-8** | Minor (view state) | P6: "A second `Action::EditExternally` while a pane is alive is refused with `EDITOR_BUSY`" (status line, in `drain`). | A view that handed text out sets a flag and waits for exactly one outcome (`Compose::external`, set at `compose.rs:436`, taken at `:460`). A status-line refusal leaves that flag set, so a later, unrelated outcome would be applied. | **PD-8**: the refusal answers the **asking tab** with `ExternalEditOutcome::Failed(EDITOR_BUSY)` through `finish_external_edit`, from `App::take_external_edit` (one place, after `drain` stamped it). |
| F-9 | Minor (UX) | P4: default `ctrl-4`. | `KeyChord::label` writes it `Ctrl+4` (`chord.rs:81-114`). A user presses `Ctrl+\`, which arrives as `Ctrl+4` (`crossterm parse.rs` `0x1C..=0x1F` → `Char('4'..'7') + CONTROL`). Physical `Ctrl+4` sends the same byte on xterm-family terminals. | Docs (T7) say "`Ctrl+\` (shown as `Ctrl+4`)". No code. |
| F-10 | Minor (teardown order) | "dropping the writer sends EOF to the child". | `UnixMasterWriter::drop` writes `"\n"` + `VEOF` into the PTY (`unix.rs:393-405`). To a live editor in raw mode those are two keystrokes. | The writer lives in the writer thread and is dropped only when the pane is dropped, which first requests the kill (B5). Never drop it to "close stdin" on a live editor. |
| F-11 | Minor (compile break T6 must fix) | — | Returning a `Rect` from `item_form::render` breaks `item_form.rs:1594` (`.draw(\|frame\| render(..))`: `Terminal::draw` wants `FnOnce(&mut Frame)` returning `()`) and the match at `backlog/mod.rs:936-944` (arms must agree). `compose.rs`'s two test calls end in `;` and are fine (`:1076`, `:1128`). | T6 edits both (§8). |
| F-12 | Note (not a defect) | P2: a claim per view. | Templates and Library also hand off from **browse** mode (`E` on a selected row, `tests/templates.rs:776-781`), where no draft is drawn and nothing is claimed. | Expected: the pane falls back to the tab body. Recorded so the reviewer does not flag it. |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| (a) The `Pane` abstraction | A **trait for the process side only**: `PaneChild { write, resize, kill }`. The real `PtyChild` owns the PTY and three threads. `App` owns everything else for real (`PaneScreen` parser, `TempEdit`, `EditorCommand`), so `App` tests run the real parser and read-back with a recording `FakeChild` and no process. The spawn is a closure handed to `App::open_editor`. | B2, B3, §4.4, §7.2 |
| (b) How `PaneEvent`s reach the loop | `event_loop::run` creates `mpsc::unbounded_channel::<PaneEvent>()`, keeps the sender (so `recv` never ends) and hands a clone to `PtyChild::spawn` inside the `open_editor` closure. Fourth `select!` arm: `Some(event) = pane_rx.recv() => app.on_pane_event(event)`. Post-steps: editor step (suspend **or** open), mouse capture, dirty draw, `app.resize_editor()`. | §7.5, PD-1 |
| (c) Key encoding, application-cursor mode | The legacy xterm table in §4.5. Application-cursor mode is `vt100::Screen::application_cursor()` (DECCKM, `screen.rs:560`), read from the pane's parser at each key. Bracketed paste is `Screen::bracketed_paste()` (`screen.rs:572`). | §4.5 |
| (d) Cursor and `TerminalGuard` | ratatui hides the cursor after **every** frame that does not call `Frame::set_cursor_position` (`frame.rs:155-168`); `Suspend::leave`'s doc says "every draw hid it" (`terminal.rs`). So `TerminalGuard` does not change. The widget sets the position only when `focused` (App: pane focused **and** drawn **and** no overlay **and** no `?` box) and the child has not hidden it (`Screen::hide_cursor()`). | §5.2, §7.4 |
| (e) Mouse capture | Off while any in-pane editor is alive: `wants_mouse()` gains `self.editor.is_none() &&`, so `mouse_capture()` returns `false` and the existing edge tells the tabs. The terminal's own selection then works over the pane. M2 relaxes it to "no focused pane drawn". | §7.3 |
| (f) Frame title and status sentences | Title: `" {value} · Ctrl+4 to htui "` focused, `" {value} · htui has the keys "` unfocused; the labels are `Keys::hint` over the editor stacks. Status line (when no error is up): `"the editor has the keys · Ctrl+4 to htui"` / `"Ctrl+4 to the editor · Ctrl+x abort · q quit · ? help"`. Constants: `EDITOR_BUSY`, `EDITOR_ABORTED` (`editor.rs`), `EDITOR_LOCKED` (`app/pane.rs`); the refusal is `"{EDITOR_LOCKED}: Ctrl+4 to the editor · Ctrl+x abort"`. | §3.1, §7.4 |
| (g) Abort and exit races | Every event carries a `PaneId`; `App` ignores an id that is not the open editor's. Abort answers at once (`Failed(EDITOR_ABORTED)`), drops the editor (kill requested, temp file removed), and a later `Exited`/`Output` of that id is dropped. Quit drops `App` in `lib.rs:199` (after `term.restore()`), which drops the `PtyChild`, which requests the kill; `lib.rs` then waits up to `SHUTDOWN` for the store worker, which gives the wait thread its 200 ms grace. The temp file is owned by `OpenEditor.temp` until `finish` or drop. | B4, B5, B8, §11 H-3, H-8 |
| (h) `HTUI_EDITOR_PANE` once per edit | In the loop's post-step: one `let lookup = \|key: &str\| std::env::var(key).ok();`, then `EditorCommand::resolve(lookup)` and `EditorMode::resolve(lookup)` (the closure captures nothing, so it is `Copy`). `EditorCommand::from_env()` stays public but the loop no longer calls it. | §3.1, §7.5 |
| (i) Windows | portable-pty compiles ConPTY on Windows; nothing in `editor/pane.rs` names a unix-only API. `pty_command` has `#[cfg(not(windows))]`/`#[cfg(windows)]` bodies like `command()`. Every real-child test is `#[cfg(unix)]`; T7's file is `#![cfg(all(unix, feature = "testkit"))]`. Gate: `CC=gcc AR=ar cargo check -p htui --target x86_64-pc-windows-gnu` after T1, T2, T5. | §3, §4, §11 H-10 |
| (j) The M1 lock (P6) | Unfocused: `ctrl-c` → quit (C3), then the first candidate of `keys.actions(Stack::EDITOR_UNFOCUSED, chord)` where `EDITOR_UNFOCUSED = [editor, global ∩ {quit, help}]`: `EditorFocus` → refocus, `EditorAbort` → abort, `Quit` → quit, `Help` → toggle the `?` box; anything else → the refusal on the status line. No overlay, tab or legacy row sees the key. Focused: only `keys.actions(Stack::EDITOR_FOCUSED, chord)` (`[editor ∩ {focus}]`) is consulted; everything else is encoded and written. | §6.2, §7.4 |
| (k) `Ctx::claim_editor_area` | `App.editor_area: Cell<Option<Rect>>` is set to `None` at the top of every `render`, lent **only** to the active tab's render `Ctx` (`Ctx::new` leaves it `None`, so overlays and every other site claim nothing). After the tab draws, `App` records `editor_rect = (active tab, claim ∩ body if ≥ MIN_PANE else body)`. The pane is drawn there only when the active tab is the editor's tab. | §7.1, §7.4 |
| MOD-67 M2 | M2's loader adds `editor.focus` to the must-stay-bound rule beside `overlay.close` (ANA-26 §7.4 step 6), checks the `[editor]` table's chords against `global.quit`/`global.help` because `EDITOR_UNFOCUSED` stacks them, and gets a `Context::Editor` arm wherever it matches `Context` exhaustively. Whichever branch lands second does it. | §6.4 |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles) | Gate (on the real tree after merge, `--test-threads=1`) |
|---|---|---|---|
| T0 deps | `Cargo.toml`, `crates/htui/Cargo.toml`, `Cargo.lock`, `editor.rs` (+1 line), `editor/pane.rs` (stub), `ui/mod.rs` (+1 line), `ui/editor_pane.rs` (stub) | 1 | `cargo build -p htui`; Windows cross-check; `cargo tree -d -p htui` shows one `ratatui` |
| T1 editor.rs | `editor.rs` | 2 (§3.6) | `cargo test -p htui --lib -- editor::tests`; clippy both ways; Windows cross-check |
| T2 pane | `editor/pane.rs` | 2 (§4.8) | `cargo test -p htui --lib -- editor::pane`; clippy both ways; Windows cross-check |
| T3 widget | `ui/editor_pane.rs`, its snapshots | 1 | `cargo insta test -p htui --all-features -- ui::editor_pane` (review, accept, commit the `.snap`s) |
| T4 keys | `keys/catalogue.rs`, `keys/stack.rs`, `keys/mod.rs` | 1 | `cargo test -p htui --lib -- keys::`; clippy both ways |
| merge T1-T4 | - | - | `cargo test -p htui --all-features -- --test-threads=1`; `cargo doc -p htui --no-deps` |
| T5 shell | `app/state.rs`, `app/pane.rs` (new), `app/mod.rs`, `event_loop.rs`, `app/snapshots/` | 3 (§7.8) | `cargo test -p htui --all-features --lib -- app:: event_loop::`; insta; clippy both ways; Windows cross-check |
| T6 claims | `detail/compose.rs`, `detail/notes.rs`, `detail/documents.rs`, `item_form.rs`, `backlog/mod.rs`, `skills/templates.rs`, `skills/library.rs` | 2 (§8.3) | `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features --check` (zero changes) |
| T7 e2e, docs | `tests/editor_pane.rs` (new), `docs/htui-editor.md` (new), `README.md`, the plan (manual-run notes) | 2 (§9.3) | the plan's full Validation block |

"Clippy both ways" = `cargo clippy -p htui --all-targets --all-features -- -D warnings` **and**
`cargo clippy -p htui -- -D warnings` (featureless; memory: `--all-features` hides dead code).
"Windows cross-check" = `CC=gcc AR=ar cargo check -p htui --target x86_64-pc-windows-gnu`.

---

## 2. T0: dependencies and module stubs (main thread, before the fan-out)

1. `Cargo.toml` `[workspace.dependencies]`, after the `crossterm`/`futures` lines, in the file's
   comment style:
   ```toml
   # MOD-57 M1 (plan P10): the in-pane editor. `portable-pty` runs `$EDITOR` on a pseudo-terminal
   # (`editor::pane`), `vt100` parses what it draws (`ui::editor_pane` copies the screen into the
   # frame). No features; one ratatui in the tree (`cargo tree -d`). Not `tui-term` (plan P10).
   portable-pty       = "0.9.0"
   vt100              = "0.16.2"
   ```
2. `crates/htui/Cargo.toml` `[dependencies]`: `portable-pty = { workspace = true }`,
   `vt100 = { workspace = true }`, with a one-line `# MOD-57 M1 (plan P10)` comment.
3. `crates/htui/src/editor.rs`: after the `use` block, `pub mod pane;` with the doc line
   `/// The in-pane editor's process side (MOD-57 M1).`
4. `crates/htui/src/editor/pane.rs`: only
   `//! The in-pane editor's process side: PTY, threads, VT screen and key encoder (MOD-57 M1, plan P7).`
5. `crates/htui/src/ui/mod.rs`: `pub mod editor_pane;` (alphabetical, after `diff`).
6. `crates/htui/src/ui/editor_pane.rs`: only
   `//! The in-pane editor's widget: a VT screen copied into the frame (MOD-57 M1, plan P2, P10).`
7. `cargo build -p htui`, the Windows cross-check, `cargo tree -d -p htui | grep -c ratatui`
   (expect the same count as before). Commit: `build(mod-57): T0 - portable-pty and vt100,
   editor::pane and ui::editor_pane stubs` with the `Cargo.lock` change.

---

## 3. T1: `editor.rs`: shared read-back, PTY command, mode (P3, P8, P9; PD-6, PD-8)

**First failing test**: `editor::tests::editor_mode_reads_htui_editor_pane`.

### 3.1 New items (all `pub`, documented)

```rust
/// The variable that chooses the in-pane editor (MOD-57 plan P3), read next to `$VISUAL`/`$EDITOR`.
pub const PANE_VAR: &str = "HTUI_EDITOR_PANE";

/// The answer to a second edit while an in-pane editor is alive (MOD-57 P6, PD-8).
pub const EDITOR_BUSY: &str = "an editor is already open: return to it or abort it first";

/// The answer to `editor.abort` (MOD-57 P8): the editor is killed and nothing is read back.
pub const EDITOR_ABORTED: &str = "the editor was aborted; nothing was changed";

/// How `E`/`Ctrl+E` run the editor (MOD-57 P3, PRD D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    /// Today's handoff: the TUI is suspended while the editor runs. The default.
    Suspend,
    /// The editor runs on a pseudo-terminal drawn inside htui.
    Pane,
}

impl EditorMode {
    /// [`PANE_VAR`] through `lookup`: `1`, `true` or `yes` (trimmed, ASCII case-insensitive) is
    /// `Pane`; anything else, unset or blank included, is `Suspend`.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>) -> Self;
}

/// How an editor process ended, whichever runner ran it (MOD-57 P8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorExit {
    /// It exited with this code.
    Code(i32),
    /// A signal ended it.
    Signal,
}
impl From<std::process::ExitStatus> for EditorExit {}      // code().map_or(Signal, Code)
impl From<&portable_pty::ExitStatus> for EditorExit {}     // signal().is_some() → Signal, else
                                                            // Code(i32::try_from(exit_code()).unwrap_or(i32::MAX))

/// One edit's temp file (MOD-57 P8): written by `create`, read back by `finish`, removed when it
/// drops, on every path. `Debug` prints the path and the handed text's length, never the text.
pub struct TempEdit {
    /// Removed on drop (`tempfile::TempPath`).
    path: tempfile::TempPath,
    /// The text handed out, for the unchanged comparison.
    handed: String,
}

impl TempEdit {
    /// `run`'s first half: a `.md` temp file named `htui-<sanitised stem>-<random>`, holding
    /// `text`, its handle closed.
    ///
    /// # Errors
    /// `Failed("could not create a temp file: …")` / `Failed("could not write the temp file: …")`,
    /// today's sentences.
    pub fn create(text: &str, stem: &str) -> Result<Self, ExternalEditOutcome>;

    /// The file the editor is handed.
    #[must_use]
    pub fn path(&self) -> &Path;

    /// `run`'s second half (normative below). Consumes `self`: the file is removed when this returns.
    #[must_use]
    pub fn finish(self, cmd: &EditorCommand, exit: io::Result<EditorExit>, elapsed: Duration)
        -> ExternalEditOutcome;
}

impl EditorCommand {
    /// The same process as [`command`](Self::command), on a pseudo-terminal (MOD-57 P9).
    #[must_use]
    pub fn pty_command(&self, file: &Path) -> portable_pty::CommandBuilder;
}
```

### 3.2 `TempEdit::finish`, normative (byte-identical sentences to today's `run`, `editor.rs:221-284`)

1. `exit` is `Err(err)` → `Failed(start_failure(cmd, &err.to_string()))`.
2. `Code(0)` → go on. `Code(c)` with `is_start_failure(c)` → `Failed(start_failure(cmd, "not
   found or not executable"))`. Other `Code(c)` → `` Failed(format!("`{}` exited with {c};
   nothing was changed", cmd.value())) ``. `Signal` → the same sentence with `a signal`.
3. Read the file: an error → `Failed("could not read the edited file back ({err}); nothing was
   changed")`; not UTF-8 → `Failed("the edited file is not UTF-8; nothing was changed")`.
4. `normalise_newlines` both sides; equal → `Unchanged { quick: elapsed < QUICK_EXIT }`, else
   `Edited(normalised)`.

`run` becomes: `create` (an `Err(outcome)` is returned), `Interrupts::hold()`, `Instant::now()`,
`tokio::process::Command::from(cmd.command(temp.path())).kill_on_drop(true).status().await`,
`elapsed`, `drop(interrupts)`, `temp.finish(cmd, status.map(EditorExit::from), elapsed)`. The
dropped-future guarantee holds: `temp` drops with the future. `run`'s doc keeps its contract and
names `TempEdit`.

### 3.3 `pty_command`, normative (PD-6)

- **Unix**: `CommandBuilder::new("sh")`; args `-c`, `` format!("exec {} \"$1\"", self.value) ``,
  `htui-editor`, then `file`; `env("TERM", "xterm-256color")`; `env_remove("LINES")`,
  `env_remove("COLUMNS")`; `if let Ok(dir) = std::env::current_dir() { cwd(dir) }`. The doc
  points at `command()`'s for the `exec` reasons and adds: "the child is a session leader on
  its own PTY (portable-pty `setsid` + `TIOCSCTTY`), so `ctrl-c` reaches only it; no
  `Interrupts` are needed."
- **Windows**: `CommandBuilder::new("cmd")`; args `/S`, `/C`, then one argument
  `format!("{} {}", self.value, name)` where `name` is `file.file_name()` (lossy); `cwd` = the
  file's parent; `env("TERM", "xterm-256color")`. Doc: CommandBuilder quotes every argument
  (no `raw_arg`), hence the bare name in the temp dir; a `value` holding `"` is MOD-16's.
- Never `Debug` the builder: it carries the whole environment.

### 3.4 T1 tests (`editor::tests`), TDD order

1. `editor_mode_reads_htui_editor_pane`: table through `vars(..)`: unset, `""`, `" "`, `"0"`,
   `"no"`, `"false"`, `"on"` → `Suspend`; `"1"`, `"true"`, `"TRUE"`, `" yes "` → `Pane`.
2. `editor_exit_from_both_runners`: `portable_pty::ExitStatus::with_exit_code(0|3)` →
   `Code(0|3)`; `with_signal("Hangup")` → `Signal`; (unix) a real `sh -c 'exit 4'` status →
   `Code(4)`, `sh -c 'kill -TERM $$'` → `Signal`.
3. `temp_edit_create_writes_the_text_under_the_sanitised_stem`: stem `"../a/b c"` → parent is
   `temp_dir()`, name starts `htui-___a_b_c-`, ends `.md`, content is the text; dropped → gone.
4. `temp_edit_finish_maps_every_exit`: a table over (file rewrite or none, `exit`, `elapsed`) →
   outcome: appended + `Code(0)` → `Edited`; untouched + 10 ms → `Unchanged{quick:true}`;
   untouched + 2 s → `quick:false`; `Code(3)` → contains `exited with 3`; `Signal` → contains
   `exited with a signal`; (unix) `Code(127)` → contains `$VISUAL or $EDITOR` and `not found or
   not executable`; `Err(io::Error::other("boom"))` → contains `(boom)`; `\xff\xfe` → the UTF-8
   sentence; CRLF rewrite → normalised. Each row asserts the file is gone afterwards.
5. `the_pane_sentences_are_pinned`: `EDITOR_BUSY`, `EDITOR_ABORTED` verbatim.
6. `a_temp_edit_debug_prints_lengths_not_text`.
7. `#[cfg(unix)] pty_command_matches_command_s_argv`: `get_argv()` ==
   `["sh", "-c", "exec code --wait \"$1\"", "htui-editor", "/tmp/htui-implement-x.md"]`, `TERM`
   is `xterm-256color`, `get_env("LINES")`/`("COLUMNS")` are `None`, `get_cwd()` is the current
   dir.
8. `#[cfg(windows)] pty_command_names_the_file_in_its_directory` (compiled by the cross-check,
   never run here).
9. Every existing `run`/`run_suspended` test (`mod scripts`, `mod suspension`, `editor.rs:613`,
   `:829`) passes **unchanged**.

### 3.5 Hazards for T1

- `start_failure` and `is_start_failure` stay private; `finish` calls them.
- `TempEdit` must not hold an open handle on Windows (`into_temp_path()` closes it, as today).
- `From<&portable_pty::ExitStatus>`: `exit_code()` is `u32`; `try_from`, never `as`.

### 3.6 T1 commits and gate

1. `refactor(mod-57): T1 - TempEdit and EditorExit; run reads back through them` (tests 2-4, 6,
   9 green).
2. `feat(mod-57): T1 - EditorMode, pty_command, the pane sentences` (tests 1, 5, 7, 8).

Gate: `cargo test -p htui --lib -- editor::tests`, clippy both ways, Windows cross-check.

---
## 4. T2: `editor/pane.rs`: PTY, threads, screen, key encoder (P7, P10; PD-2, PD-4, PD-7)

**First failing test**: `editor::pane::tests::the_legacy_key_table`.

Module doc (replacing T0's line): what the module owns, the three threads, the kill path (B5),
the reply callbacks, "nothing here blocks the caller: `write` queues, `resize` is one ioctl,
`kill` sends on a channel" (R-NF-3), and "bytes and screen contents are never logged or
`Debug`ged".

### 4.1 Types

```rust
/// One in-pane editor's identity. Every event carries it, so the shell drops events of an editor
/// it has already finished or aborted (MOD-57 B4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(u64);
impl PaneId {
    /// A fresh id: a process-wide `AtomicU64` counter.
    #[must_use]
    pub fn next() -> Self;
}

/// A pane's size in cells. Never 0 in either dimension (`vt100` and the kernel both dislike it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSize {
    /// Rows.
    pub rows: u16,
    /// Columns.
    pub cols: u16,
}
impl PaneSize {
    /// Before anything was drawn: 24x80.
    pub const DEFAULT: Self = Self { rows: 24, cols: 80 };
    /// `rows`/`cols`, each at least 1.
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self;
}

/// What a pane's threads tell the shell. `Debug` prints lengths, never bytes.
pub enum PaneEvent {
    /// The child drew: raw bytes from the PTY, in order.
    Output {
        /// The pane.
        id: PaneId,
        /// What was read (at most 8 KiB).
        bytes: Vec<u8>,
    },
    /// The child ended and was reaped.
    Exited {
        /// The pane.
        id: PaneId,
        /// Its status, or why waiting failed.
        status: std::io::Result<portable_pty::ExitStatus>,
        /// Since the spawn, measured by the wait thread.
        elapsed: std::time::Duration,
    },
}
impl PaneEvent { #[must_use] pub fn id(&self) -> PaneId; }

/// The process side of an in-pane editor (MOD-57 B2). `PtyChild` is the real one; `App` tests
/// use a recording fake. No method blocks.
pub trait PaneChild: core::fmt::Debug {
    /// Queues `bytes` for the child's input.
    fn write(&mut self, bytes: &[u8]);
    /// Tells the kernel, and so the child (`SIGWINCH`), the new size.
    fn resize(&mut self, size: PaneSize);
    /// Asks for the child to end: SIGHUP, then SIGKILL after a grace (B5). Idempotent.
    fn kill(&mut self);
}

/// A child on a pseudo-terminal, with its reader, writer and wait threads. Dropping it kills the
/// child. `Debug` prints the pid only.
pub struct PtyChild {
    /// Kept for `resize`. Dropped after the kill request (field order).
    master: Box<dyn portable_pty::MasterPty + Send>,
    /// The writer thread's queue; dropping it ends the thread (which drops the writer, F-10).
    input: std::sync::mpsc::Sender<zeroize::Zeroizing<Vec<u8>>>,
    /// The wait thread's kill request; `None` once sent.
    kill: Option<std::sync::mpsc::Sender<()>>,
    /// For `Debug` and tests.
    pid: Option<u32>,
}
impl PtyChild {
    /// Opens a PTY of `size`, spawns `command` on it, drops the slave, starts the threads.
    ///
    /// # Errors
    /// The PTY could not be opened, the command could not be spawned, or a thread could not be
    /// started (the child, if spawned, is then killed by the wait thread or, if that thread never
    /// started, by a `clone_killer` SIGHUP taken before it, §4.2 step 3).
    pub fn spawn(
        command: portable_pty::CommandBuilder,
        size: PaneSize,
        id: PaneId,
        events: tokio::sync::mpsc::UnboundedSender<PaneEvent>,
    ) -> std::io::Result<Self>;
    /// The child's pid, if the platform has one.
    #[must_use]
    pub fn pid(&self) -> Option<u32>;
}
impl PaneChild for PtyChild { /* write: copy into Zeroizing, send; resize: master.resize; kill: take + send */ }
impl Drop for PtyChild { fn drop(&mut self) { self.kill(); } }

/// The VT screen of a pane and the replies it owes the child (DSR, DA1). `Debug` prints the size.
pub struct PaneScreen { parser: vt100::Parser<Replies> }
/// The callbacks: replies queued while parsing (private; no `pub` doc links it).
struct Replies { out: Vec<u8> }
impl PaneScreen {
    /// A blank screen of `size`, no scrollback.
    #[must_use] pub fn new(size: PaneSize) -> Self;
    /// Parses `bytes`; returns the replies to write back to the child (often empty).
    #[must_use] pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8>;
    /// The screen, for the widget and the modes.
    #[must_use] pub fn screen(&self) -> &vt100::Screen;
    /// The current size.
    #[must_use] pub fn size(&self) -> PaneSize;
    /// Resizes the grid (`Screen::set_size`).
    pub fn resize(&mut self, size: PaneSize);
}

/// `key` as the bytes a legacy xterm sends (§4.5); `None` for a key it has no encoding for.
#[must_use]
pub fn encode_key(key: crossterm::event::KeyEvent, application_cursor: bool) -> Option<Vec<u8>>;

/// A paste as the child should receive it (§4.6). Zeroized on drop: a paste may be a credential
/// (MOD-22).
#[must_use]
pub fn encode_paste(text: &str, bracketed: bool) -> zeroize::Zeroizing<Vec<u8>>;
```

### 4.2 `PtyChild::spawn`, normative (probe gotchas, F-3, F-10)

1. `native_pty_system().openpty(size.pty())`; portable-pty errors are `anyhow::Error`: map with
   `io::Error::other(err.to_string())`.
2. `let mut child = pair.slave.spawn_command(command)?;` then **`drop(pair.slave)`** at once
   (otherwise the reader never sees EOF). `pid = child.process_id()`, `started = Instant::now()`.
3. **Wait thread first** (`std::thread::Builder::new().name("htui-pane-wait")`), owning `child`
   and `kill_rx`. Loop: `child.try_wait()`: `Ok(Some(status))` → send `Exited { Ok(status) }`,
   end; `Err(err)` → send `Exited { Err(err) }`, end. Then `kill_rx.recv_timeout(POLL)` with
   `const POLL: Duration = 25 ms`: `Ok(())` or `Disconnected` → `let _ = child.kill();` (the owned
   impl: SIGHUP, ≤ 200 ms grace, SIGKILL; TerminateProcess on Windows), `child.wait()`, send
   `Exited`, end; `Timeout` → loop. A failed `send` (the loop is gone) is ignored. If the thread
   cannot be spawned, `Builder::spawn` drops the closure and the child with it, unkilled: so take
   `let mut fallback = child.clone_killer();` **before** the spawn and, on that one error path,
   `let _ = fallback.kill();` (SIGHUP; the child is not reaped yet, so the pid is still its own).
   **No child outlives an `Err`.** The fallback is dropped on success (B5).
4. `reader = master.try_clone_reader()`, `writer = master.take_writer()`. Any error → return it;
   dropping `kill_tx` makes the wait thread kill the child.
5. Reader thread (`htui-pane-read`): `[0u8; 8192]` buffer; `Ok(0)` → end; `Ok(n)` → send
   `Output { bytes: buf[..n].to_vec() }`, end if the send fails; `Interrupted` → retry; other
   `Err` (EIO once the child is gone, Linux) → end.
6. Writer thread (`htui-pane-write`): `while let Ok(chunk) = input_rx.recv()`:
   `write_all(&chunk)` then `flush()`; an error ends it. It ends when `input` drops, and drops
   the writer then (F-10).
7. Return `PtyChild { master, input, kill: Some(kill_tx), pid }`.

`kill()`: `if let Some(tx) = self.kill.take() { let _ = tx.send(()); }`. `resize()`:
`master.resize(size.pty())`, an error logged at `debug` with the size only. `write()`:
`Zeroizing::new(bytes.to_vec())`, a failed send logged at `debug` with the length only.

### 4.3 `Replies` (the `vt100::Callbacks` impl, `callbacks.rs:55`), normative

`unhandled_csi(&mut self, screen, i1, i2, params, c)`, with `first = params.first().and_then(|p|
p.first()).copied()`:
- `(None, None, 'n', Some(6))` → `` write!(out, "\x1b[{};{}R", row + 1, col + 1) `` from
  `screen.cursor_position()` (DSR, cursor position).
- `(None, None, 'n', Some(5))` → `\x1b[0n` (DSR, status: OK).
- `(None, None, 'c', None | Some(0))` → `\x1b[?62;22c` (DA1: VT220 with ANSI colour).
- anything else: nothing (DA2 `ESC[>c`, OSC queries and XTGETTCAP stay unanswered; nvim falls
  back after its DA1 sentinel).

`feed` = `parser.process(bytes)` then `std::mem::take(&mut parser.callbacks_mut().out)`.

### 4.4 Why the trait has three methods (B2)

`App` owns `PaneScreen`, `TempEdit` and `EditorCommand` for real in every test: the parser, the
modes, the replies and the read-back are exercised without a process. Only what needs a process
(`write`, `resize`, `kill`) is behind `PaneChild`. A `FakeChild` (T5) records `Write(Vec<u8>)`,
`Resize(PaneSize)`, `Kill`, and `Dropped` (its `Drop`) into an `Rc<RefCell<Vec<_>>>`. No `Send`
bound: `App` is not `Send` (tabs hold `Rc`s).

### 4.5 `encode_key`: the table (normative; legacy xterm, no kitty protocol, PRD Q4)

`m` below is the xterm modifier parameter `1 + shift(1) + alt(2) + ctrl(4)`; "modified" means
`m > 1`. SHIFT on a `Char` is ignored (crossterm already sent the shifted character).

| Key | Unmodified | Application cursor (DECCKM) | Modified |
|---|---|---|---|
| `Char(c)` | `c` as UTF-8 | same | see the ctrl and alt rows |
| `Char(c)` + CONTROL | `a`-`z` (and `A`-`Z`) → `0x01`-`0x1A`; `' '`, `'@'`, `'2'` → `0x00`; `'['`, `'3'` → `0x1B`; `'\\'`, `'4'` → `0x1C`; `']'`, `'5'` → `0x1D`; `'^'`, `'6'` → `0x1E`; `'_'`, `'7'`, `'/'` → `0x1F`; `'?'`, `'8'` → `0x7F`; any other `c` → `c` (CONTROL dropped) | same | + ALT: `0x1B` prefix |
| `Char(c)` + ALT | `0x1B` then the unmodified encoding | same | |
| `Enter` | `\r` | same | + ALT: `\x1b\r` |
| `Tab` / `BackTab` | `\t` / `\x1b[Z` | same | + ALT: `0x1B` prefix |
| `Backspace` | `0x7F` | same | + CONTROL: `0x08`; + ALT: `0x1B` prefix |
| `Esc` | `0x1B` | same | + ALT: `\x1b\x1b` |
| `Up` `Down` `Right` `Left` | `\x1b[A` `B` `C` `D` | `\x1bOA` `OB` `OC` `OD` | `\x1b[1;{m}A` … (CSI even in DECCKM) |
| `Home` / `End` | `\x1b[H` / `\x1b[F` | `\x1bOH` / `\x1bOF` | `\x1b[1;{m}H` / `F` |
| `Insert` `Delete` `PageUp` `PageDown` | `\x1b[2~` `3~` `5~` `6~` | same | `\x1b[{n};{m}~` |
| `F(1..=4)` | `\x1bOP` `OQ` `OR` `OS` | same | `\x1b[1;{m}P` … `S` |
| `F(5..=12)` | `\x1b[{n}~`, `n` = 15 17 18 19 20 21 23 24 | same | `\x1b[{n};{m}~` |
| everything else (`F(13..)`, `Null`, `CapsLock`, media, modifier keys, `KeypadBegin`, `Menu`) | `None` | | |

`ctrl-c` is `0x03` here: the focused pane forwards it (P5). `ctrl-4` never reaches the encoder
while it is bound to `editor.focus` (§7.4).

### 4.6 `encode_paste`, normative

1. Remove every `\x1b[201~` from `text` (a paste must not end the bracket early: injection).
2. Normalise `\r\n` and lone `\n` to `\r` (what a terminal sends for a pasted newline; a raw
   `\n` is `ctrl-j`, nano's "justify").
3. If `bracketed`, wrap in `\x1b[200~` … `\x1b[201~`.

### 4.7 T2 tests, TDD order

Pure (any platform, `#[test]`):
1. `the_legacy_key_table`: one row per cell of §4.5, including `ctrl-c` → `[0x03]`, `ctrl-4` →
   `[0x1C]`, `ctrl-space` → `[0x00]`, `ctrl-h` → `[0x08]`, `alt-x` → `\x1bx`, `alt-ctrl-a` →
   `\x1b\x01`, `shift-up` → `\x1b[1;2A` with and without DECCKM, `ctrl-right` → `\x1b[1;5C`,
   `shift-f5` → `\x1b[15;2~`, `ctrl-delete` → `\x1b[3;5~`, `é` → UTF-8, `F(13)` → `None`.
2. `application_cursor_mode_changes_unmodified_arrows_and_home_end_only`.
3. `a_paste_is_normalised_bracketed_and_cannot_close_the_bracket`: `("a\nb", false)` → `a\rb`;
   `("a\r\nb", true)` → `\x1b[200~a\rb\x1b[201~`; `("x\x1b[201~y", true)` → one closing bracket.
4. `the_screen_answers_dsr_and_da1`: `feed(b"ab\x1b[6n")` → `\x1b[1;3R`; `feed(b"\x1b[c")` →
   `\x1b[?62;22c`; `feed(b"\x1b[5n")` → `\x1b[0n`; `feed(b"\x1b[>c")` → empty; plain text → empty.
5. `the_screen_reports_its_modes`: `\x1b[?1h\x1b[?2004h\x1b[?25l` → `application_cursor()`,
   `bracketed_paste()`, `hide_cursor()` all true; `\x1b[?1049h` → `alternate_screen()`.
6. `resize_changes_the_grid_and_never_to_zero`: `PaneSize::new(0, 0)` is 1x1.
7. `nothing_debugs_bytes_or_contents`: `PaneEvent::Output` with `SECRET` bytes, a `PaneScreen`
   fed `SECRET` → neither `{:?}` contains `SECRET`; `Output` shows `len`.

Real child (`#[cfg(unix)] #[tokio::test]`, **real time**, never `start_paused`; each test holds
its `PtyChild` so a failure kills on drop; scripts are written as `editor.rs`'s `scripts::script`
does, with a local copy of the helper since that module is private to `editor::tests`):
8. `a_scripted_editor_draws_reads_a_line_and_exits`: script `printf ready; IFS= read -r line;
   printf '%s\n' "$line" > "$1"`. Pump `Output` into a `PaneScreen` (writing its replies back)
   until the contents hold `ready`; `write(b"edited\r")`; `Exited` within 10 s with
   `success()`; the file reads `edited\n` (PD-4: no `TempEdit` here).
9. `the_child_sees_the_size_and_a_resize`: `stty size; read x; stty size; read y` → `24 80`,
   then `resize(PaneSize::new(30, 100))`, `write(b"\r")` → `30 100`.
10. `kill_ends_the_child_and_exited_follows`: `exec sleep 30`; `kill()` → `Exited` within 3 s,
    not `success()`.
11. `a_hup_ignoring_child_is_killed_by_escalation`: `trap '' HUP; while :; do sleep 0.1; done`;
    `kill()` → `Exited` within 3 s.
12. `dropping_the_child_leaves_no_process`: `printf '%s' $$ > pidfile; exec sleep 30`; drop the
    `PtyChild` → the pid is gone (or a zombie) within 3 s (`ps -o stat= -p`, as
    `editor.rs::tests::suspension::alive`).
13. `both_threads_end_after_exit`: `exit 0`; the test drops its own sender after `spawn`; after
    `Exited`, `rx.recv()` returns `None` within 3 s (reader saw EOF because the slave was
    dropped; the wait thread ended).
14. `a_dsr_from_the_child_is_answered`: `stty -icanon -echo; printf '\033[6n'; dd bs=1 count=6
    2>/dev/null > "$1"` → the file holds `\x1b[1;1R`.

### 4.8 T2 commits and gate

1. `feat(mod-57): T2 - key encoder, paste encoding and the VT screen` (tests 1-7).
2. `feat(mod-57): T2 - PtyChild: PTY, reader, writer and wait threads` (tests 8-14).

Gate: `cargo test -p htui --lib -- editor::pane`; clippy both ways; Windows cross-check; after
the run, `ps -eo args | grep -c 'sleep 30'` is 0 (memory: orphaned test processes).

---

## 5. T3: `ui/editor_pane.rs`: the widget (P2, P10; PD-5)

**First failing test**: `ui::editor_pane::tests::the_screen_is_copied_cell_by_cell`.

### 5.1 Items

```rust
/// Draws an in-pane editor over `area`: a top rule titled `title` (focused: `theme.title`, else
/// `theme.dim`), then the VT screen cell by cell in the rest. When `focused` and the child shows
/// its cursor, the terminal's real cursor is placed on it (`Frame::set_cursor_position`).
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    screen: &vt100::Screen,
    title: &str,
    focused: bool,
    theme: &Theme,
);
/// The grid into `area` (private; normative below).
fn draw_screen(buf: &mut Buffer, area: Rect, screen: &vt100::Screen);
/// A cell's colours and attributes.
fn style_of(cell: &vt100::Cell) -> Style;
/// `Default` → `Color::Reset`, `Idx(i)` → `Color::Indexed(i)`, `Rgb(r, g, b)` → `Color::Rgb`.
fn color(color: vt100::Color) -> Color;
```

Imports: `ratatui::buffer::{Buffer, CellDiffOption}` (`ratatui-core-0.1.2/src/buffer.rs:11`),
`std::num::NonZeroU16`, `crate::ui::cells::clip` (the title), `crate::ui::Theme`.

### 5.2 `render` and `draw_screen`, normative

- `render`: empty `area` → return. `Block::new().borders(Borders::TOP).border_style(style)
  .title(Line::styled(clip(title, width), style))`; `inner = block.inner(area)`;
  `draw_screen(frame.buffer_mut(), inner, screen)`; then if `focused && !screen.hide_cursor()`
  and `(row, col) = screen.cursor_position()` lies inside `inner`, `frame.set_cursor_position(
  (inner.x + col, inner.y + row))`. Nothing else ever shows the cursor (§0a d).
- `draw_screen`, for every `(x, y)` in `area` (so **every covered cell is reset**, no stale cell
  survives a smaller screen):
  1. `target.reset()`.
  2. `screen.cell(y, x)` is `None` (area larger than the screen) → leave blank.
  3. `is_wide_continuation()` → leave blank (the wide cell covers it; never re-measured).
  4. `is_wide()` and `x + 1 == area.width` → leave blank (half a wide character does not fit).
  5. No contents → leave blank with the cell's style (a coloured background must show).
  6. Else `set_symbol(contents)`, `set_style(style_of(cell))`, and
     `set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16 of 2 if wide else 1))`: the width
     ratatui's diff uses is the VT grid's, not `unicode-width`'s (plan "Widths").
- `style_of`: `fg`/`bg` via `color`; `bold` → `BOLD`, `dim` → `DIM`, `italic` → `ITALIC`,
  `underline` → `UNDERLINED`, `inverse` → `REVERSED`. The editor's colours pass through; a
  `NO_COLOR` user's editor sees `NO_COLOR` in its own environment.

### 5.3 T3 tests, TDD order

1. `the_screen_is_copied_cell_by_cell` (insta, buffer text of a 22x7 `TestBackend`): a 6x20
   `vt100::Parser` fed `hello \x1b[31mred\x1b[0m\r\n中文X\r\n❤\u{fe0f}Y ｱﾞZ`, title
   `" nvim · Ctrl+4 to htui "`, focused. Plus asserts: the `red` cells' `fg` is
   `Color::Indexed(1)`, the `中` cell has `ForcedWidth(2)` and the next cell is blank.
2. `a_vs16_emoji_or_sound_mark_leaves_no_stale_cell_across_two_frames`: one `Terminal<TestBackend>`;
   frame 1 draws a screen with `❤\u{fe0f}Y` (then `ｱﾞZ`), frame 2 a screen with `abcd` at the
   same place → the backend buffer row reads `abcd`.
3. `the_cursor_is_real_and_only_when_focused`: focused → `terminal.get_cursor_position()` is
   inside the rect at the screen's cursor and the backend cursor is shown; unfocused → hidden;
   after `\x1b[?25l` → hidden; a cursor outside a clipped rect → hidden.
4. `a_rect_smaller_or_larger_than_the_screen_draws_without_panic`: 24x80 screen into 10x3, 0x0,
   1x1 and 90x30 rects; a wide character in the last column is blank; cells beyond the screen
   are blank.
5. `the_title_style_follows_focus` (insta snapshot of the unfocused frame is test 1's sibling:
   `the_unfocused_frame`).
6. `colours_and_attributes_map`: table over `Default`/`Idx`/`Rgb` and the five attributes.

Commit: `feat(mod-57): T3 - the editor pane widget` with both `.snap` files under
`crates/htui/src/ui/snapshots/`. Gate: `cargo insta test -p htui --all-features --
ui::editor_pane`, review, accept, `--check` clean; clippy both ways.

---

## 6. T4: the `editor` context, its two actions and two stacks (P4, P5, P6)

**First failing test**: `keys::stack::tests::the_unfocused_editor_stack_is_the_m1_lock`.

### 6.1 `keys/catalogue.rs` (appended blocks only: MOD-67 merge, §6.4)

- `Context::Editor` appended after `Common`, doc: "`[editor]`: the in-pane editor's own keys
  (MOD-57). Focused, only `editor.focus` resolves; unfocused, the M1 lock (`Stack`)."
  `table()` → `"editor"`, `heading()` → `"Editor"`. `use Context::{…, Editor}`.
- `Act::EditorFocus` (`editor.focus`: give the keys to the in-pane editor, or take them back)
  and `Act::EditorAbort` (`editor.abort`: kill the in-pane editor; nothing is read back),
  appended after `Dismiss`.
- Rows appended at the end of `CATALOGUE`:
  ```rust
  // [editor]: MOD-57 M1 (plan P4, P5). `ctrl-4` is what `ctrl-\` arrives as (`chord.rs`
  // `legacy_arrival`; `ctrl-\` itself is refused as indistinguishable). In capture: while the
  // editor is focused every other key, `ctrl-c` included, is the editor's. MOD-67 M2's loader
  // must refuse a file that unbinds it, as it does `overlay.close`.
  capture_row(Act::EditorFocus, Editor, "focus", &["ctrl-4"], "editor focus"),
  // Offered only while the editor is unfocused (`Stack::EDITOR_UNFOCUSED`): focused, `ctrl-x`
  // is nano's exit and goes to the editor.
  row(Act::EditorAbort, Editor, "abort", &["ctrl-x"], "abort edit"),
  ```
- Tests (existing, updated): `ALL` and `position` gain `EditorFocus => 41`, `EditorAbort => 42`;
  `every_act_has_exactly_one_row` asserts **43**; `context_tables_and_headings` gains
  `(Context::Editor, "editor", "Editor")`; `only_the_form_and_overlay_close_are_in_capture` is
  renamed `only_the_form_overlay_close_and_editor_focus_are_in_capture` and gains `EditorFocus`.
  New: `editor_focus_keeps_a_chord` (mirrors `overlay_close_keeps_a_chord`).

### 6.2 `keys/stack.rs`

```rust
/// `[editor ∩ {focus}]`: MOD-57 P5. While the in-pane editor has the keys only the focus toggle
/// resolves; every other chord, `ctrl-c` included, is written to the editor.
pub const EDITOR_FOCUSED: Self = Self(&[Layer::only(Context::Editor, &[Act::EditorFocus])]);

/// `[editor, global ∩ {quit, help}]`: MOD-57 P6, the M1 lock. With the editor alive and
/// unfocused nothing else resolves; `App` refuses every other key on the status line. M2 opens
/// the rest of htui.
pub const EDITOR_UNFOCUSED: Self = Self(&[
    Layer::all(Context::Editor),
    Layer::only(Context::Global, &[Act::Quit, Act::Help]),
]);
```
Update the module doc ("M1 ships two" → "MOD-67 M1 shipped two; MOD-57 adds the editor's two").

Tests:
1. `the_focused_editor_stack_resolves_only_the_toggle`: `ctrl-4` → `[EditorFocus]`; `ctrl-x`,
   `q`, `?`, `f1`, `esc`, `tab` → `[]`.
2. `the_unfocused_editor_stack_is_the_m1_lock`: `ctrl-4` → `[EditorFocus]`, `ctrl-x` →
   `[EditorAbort]`, `q` → `[Quit]`, `?`/`f1` → `[Help]`; `tab`, `1`, `w`, `ctrl-f`, `ctrl-w`,
   `esc`, `e`, `ctrl-s` → `[]`.
3. `ctrl_c_is_no_action_in_either_stack` extended to both editor stacks.
4. `the_editor_labels_come_through_the_stacks`: `label(EDITOR_UNFOCUSED, EditorFocus)` =
   `Ctrl+4`, `(…, EditorAbort)` = `Ctrl+x`, `(…, Quit)` = `q`; `label(EDITOR_FOCUSED,
   EditorAbort)` = `None`.

### 6.3 `keys/mod.rs`

New tests: `editor_focus_is_bound_and_not_printable` (compiled chords non-empty, none printable,
none `CTRL_C`); `the_editor_keys_do_not_collide_with_quit_or_help` (no chord of `[editor]` is a
chord of `global.quit`/`global.help`, since `EDITOR_UNFOCUSED` stacks them). The existing
invariants (`every_catalogue_default_parses_strictly`, `no_two_actions_in_one_context_share…`,
`no_in_capture_action_has_a_printable_chord`, `the_compiled_keys_are_the_catalogue_and_never_
ctrl_c`) cover the rows with no change. No re-export changes (`Act`, `Context`, `Stack` are
already re-exported).

`state.rs`'s `every_global_act_is_fixed_mapped_or_offerable` filters `Global`, so the two editor
acts do not enter it; `action_for` returns `None` for them (wildcard arm), so `apply_keys` can
never dispatch them: only `App`'s editor step does (§7.4).

### 6.4 The MOD-67 coupling (P4, PRD Q6)

MOD-67 M2 (the `keys.toml` loader, on its own branch) and this task both append to
`keys/catalogue.rs`, `keys/stack.rs`, `keys/mod.rs`. The conflict is textual (appended blocks).
**Whichever lands second** does, in the loader:
1. adds `editor.focus` to the must-stay-bound rule beside `overlay.close` (ANA-26 §7.4 step 6);
2. adds a `Context::Editor` arm to every exhaustive `Context` match (table names, `--print-keys`);
3. extends the per-stack collision check to `EDITOR_UNFOCUSED` (`[editor]` against
   `global.quit`/`global.help`), and the `ctrl-c` refusal to `[editor]`;
4. keeps `in_capture`'s non-printable rule, which already covers `editor.focus`.
The T4 close-out and the phase note record it.

Commit: `feat(mod-57): T4 - the editor context, focus and abort, the editor stacks`. Gate:
`cargo test -p htui --lib -- keys::`; clippy both ways.

---
## 7. T5: shell integration (P1, P2, P5, P6, P7; PD-1, PD-8; B2-B4, B8-B13, B16)

**First failing test**: `app::pane::tests::a_focused_editor_gets_every_key_ctrl_c_included`.

T5 adds **`crates/htui/src/app/pane.rs`** (PD-9) so `state.rs` changes stay small and the editor's
shell logic has one home. `app/mod.rs`: `mod pane;` and `pub use pane::{EDITOR_LOCKED,
MIN_PANE};`. `update.rs` does not change.

### 7.1 `Ctx` (`state.rs:77-152`)

```rust
/// MOD-57 P2: where the active tab names its editing rect; `None` at every other site.
editor_area: Option<&'a Cell<Option<Rect>>>,
```
`Ctx::new` sets `None` (so none of the 22 `Ctx::new` sites change). New methods:
```rust
/// Lends this context the cell [`claim_editor_area`](Self::claim_editor_area) writes. Only
/// `App::render` calls it, for the active tab's render.
#[must_use]
pub fn with_editor_area(mut self, cell: &'a Cell<Option<Rect>>) -> Self;

/// MOD-57 P2: names `area` as the rect this view's text is edited in, so an in-pane editor the
/// view asked for draws over it. A no-op outside the active tab's render; the last claim of a
/// frame wins.
pub fn claim_editor_area(&self, area: Rect);
```
`use std::cell::Cell;` beside the existing `RefCell` import (`state.rs:12`).

### 7.2 `App` fields (beside `pending_edit`/`mouse`, `state.rs:236-244`) and `App::new`

```rust
/// MOD-57: the in-pane editor, while one is alive (one at a time, P6).
pub(super) editor: Option<OpenEditor>,
/// MOD-57 P2: the active tab's claim this frame. `None` at the top of every `render`.
pub(super) editor_area: Cell<Option<Rect>>,
/// MOD-57 P2: where the active tab's pane goes, as the last frame computed it; read by
/// `open_editor` (the spawn size) and `resize_editor`.
pub(super) editor_rect: Option<(TabId, Rect)>,
```
`App::new`: `editor: None, editor_area: Cell::new(None), editor_rect: None`.

### 7.3 `state.rs` edits

1. **`take_external_edit`** (`:463`), first: if a pending edit exists **and** `self.editor` is
   `Some`, take it, `self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_BUSY
   .to_owned()))`, return `None` (PD-8). The mouse-loss logic below is unchanged. Doc: say so.
2. **`on_key`** (`:711`): after `let chord = …`, **before** the C3 check:
   ```rust
   // MOD-57 P5, P6: a live in-pane editor answers first. Focused, it takes every key,
   // `ctrl-c` included (ANA-26 §6.4's one exception); unfocused, the M1 lock.
   if self.editor.is_some() {
       self.editor_key(key, chord);
       return;
   }
   ```
   Replace the comment at `:719-721` ("MOD-57 adds the one exception …") with "MOD-57's exception
   is the step above." The doc's chain gains "the in-pane editor" first.
3. **`on_paste`** (`:532`), first statement after `self.dirty = true;`:
   `if self.editor.is_some() { self.editor_paste(text); return; }`. A paste never reaches a view
   or an overlay while an editor is alive (M1 lock); doc says so.
4. **`wants_mouse`** (`:611`): `self.editor.is_none() && self.overlays.is_empty() && …`. Doc:
   "MOD-57: off while an in-pane editor is alive, so the terminal keeps its own selection over
   the pane; M2 narrows this to a focused pane on screen." `mouse_capture()` then returns
   `false` and the existing on-to-off edge tells the tabs.
5. **`render`** (`:837`):
   - first line after `self.dirty = false;`: `self.editor_area.set(None);`
   - the active tab's ctx: `self.ctx(Origin::Tab(tab.id())).with_editor_area(&self.editor_area)`
   - right after the tab (before the status line): `self.draw_editor(frame, chrome.body);`
   - status: `None => (self.editor_status().unwrap_or_else(|| self.keys.status_line(…)),
     self.theme.dim)`.
   - Order stays: tab, **pane**, status, overlays, help (P1).
6. **`render_help`** (`:883`): after the overlay line, `if self.editor.is_some() {
   lines.extend(self.keys.help_line(Context::Editor, |_| true)); }` → `"Editor: Ctrl+4 editor
   focus · Ctrl+x abort edit"`.

### 7.4 `app/pane.rs`, normative

```rust
//! MOD-57 M1: the external editor in the TUI pane, as the shell sees it. ... (P1-P7, the M1 lock,
//! the claim, the transport, the races; links `crate::editor::pane` and `crate::ui::editor_pane`).

/// PRD Q3: the smallest claimed rect a pane uses; below it, the whole tab body. 40x8 until T7's
/// manual nvim run in the Notes compose box says otherwise.
pub const MIN_PANE: Size = Size { width: 40, height: 8 };

/// The head of the refusal while an editor is alive and unfocused (MOD-57 P6).
pub const EDITOR_LOCKED: &str = "the editor is open";

/// The focused pane's title and status hint.
const FOCUSED_HINT: HintSpec = &[Hint::One(Act::EditorFocus, "to htui")];
/// The unfocused status line.
const UNFOCUSED_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
    Hint::One(Act::Quit, "quit"),
    Hint::One(Act::Help, "help"),
];
/// The refusal's tail.
const LOCK_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
];

/// One live in-pane editor. Field order is drop order: the child (kill requested) before the temp
/// file (removed). `Debug`: id, tab, focus, size, child, temp; never the screen.
pub(super) struct OpenEditor {
    id: PaneId,
    tab: TabId,
    child: Box<dyn PaneChild>,
    temp: TempEdit,
    screen: PaneScreen,
    cmd: EditorCommand,
    focused: bool,
}

impl App {
    /// MOD-57 P1: the loop's pane step. Writes the temp file, spawns through `spawn` at the size
    /// the asking tab's pane was last drawn at, and holds the editor focused. A temp-file or spawn
    /// failure answers the tab at once (`finish_external_edit`); nothing stays open.
    pub fn open_editor<F>(&mut self, tab: TabId, edit: &ExternalEdit, cmd: EditorCommand, spawn: F)
    where
        F: FnOnce(PaneId, CommandBuilder, PaneSize) -> io::Result<Box<dyn PaneChild>>;
    /// One transport event (P7): `Output` of the open editor is parsed and its replies written
    /// back; `Exited` reads the file back (`TempEdit::finish`) and hands the outcome to the tab
    /// that asked (`finish_external_edit`). An event of any other id is dropped (B4).
    pub fn on_pane_event(&mut self, event: PaneEvent);
    /// The post-draw step (PD-1): resizes the PTY and the screen when the asking tab's pane rect
    /// changed. Nothing when the pane was not drawn this frame.
    pub fn resize_editor(&mut self);
    /// Whether an in-pane editor is alive.
    #[must_use] pub fn editor_open(&self) -> bool;
    /// The live editor's id (tests and T7 pump events by it).
    #[must_use] pub fn editor_id(&self) -> Option<PaneId>;

    pub(super) fn editor_key(&mut self, key: KeyEvent, chord: KeyChord);
    pub(super) fn editor_paste(&mut self, text: &Zeroizing<String>);
    pub(super) fn draw_editor(&mut self, frame: &mut Frame<'_>, body: Rect);
    pub(super) fn editor_status(&self) -> Option<String>;
    fn abort_editor(&mut self);
}
fn pane_rect(claim: Option<Rect>, body: Rect) -> Rect;     // claim ∩ body if ≥ MIN_PANE, else body
fn pane_size(rect: Rect) -> PaneSize;                       // PaneSize::new(height - 1, width): the top rule
fn pane_failure(err: &io::Error) -> String;                 // below
```

- **`open_editor`**: `self.dirty = true`. Defensive: an editor already open → answer
  `Failed(EDITOR_BUSY)` and return. `TempEdit::create(&edit.text, &edit.stem)`: `Err(outcome)` →
  `finish_external_edit(tab, outcome)`, return. `size` = `pane_size(rect)` if
  `self.editor_rect == Some((tab, rect))`, else `PaneSize::DEFAULT`. `id = PaneId::next()`.
  `spawn(id, cmd.pty_command(temp.path()), size)`: `Ok(child)` → `self.editor = Some(OpenEditor
  { id, tab, child, temp, screen: PaneScreen::new(size), cmd, focused: true })`; `Err(err)` →
  `finish_external_edit(tab, Failed(pane_failure(&err)))` (the temp file drops). `pane_failure`
  = `format!("could not run the editor in a pane ({err}); unset {PANE_VAR} to suspend instead")`.
- **`on_pane_event`**: `Output { id, bytes }` for the open id → `let replies =
  editor.screen.feed(&bytes);` non-empty → `editor.child.write(&replies)`; `dirty = true`.
  `Exited { id, status, elapsed }` for the open id → `let OpenEditor { tab, temp, cmd, .. } =
  self.editor.take()…;` (the child drops: its kill request finds the thread gone; harmless)
  `let outcome = temp.finish(&cmd, status.map(|status| EditorExit::from(&status)), elapsed);`
  `self.finish_external_edit(tab, outcome)`. Any other id: dropped, no `dirty`.
- **`editor_key`** (the M1 lock as a resolution rule; borrow `keys` and `editor` disjointly
  through `let Self { editor, keys, tabs, .. } = self;` or by computing the act first):
  1. `visible = tabs.active_id() == Some(editor.tab)`. A focused editor that is not visible (a
     reply moved the active tab) is unfocused first: htui has the keys.
  2. **Focused**: if `keys.actions(Stack::EDITOR_FOCUSED, chord)` contains `EditorFocus` →
     `focused = false`, return. Else `encode_key(key, editor.screen.screen()
     .application_cursor())`: `Some(bytes)` → `child.write(&bytes)`; `None` → dropped. `ctrl-c`
     is written (`0x03`), never a quit. Nothing reaches an overlay, a tab or a keymap.
  3. **Unfocused**: `chord == CTRL_C` → `self.update(Action::Quit)` (C3). Else the first of
     `keys.actions(Stack::EDITOR_UNFOCUSED, chord)`:
     - `EditorFocus` → `help_visible = false`, `focused = true`, and if not visible
       `self.update(Action::Tab(TabAction::Focus(tab)))` (B16);
     - `EditorAbort` → `abort_editor()`;
     - `Quit` → `self.update(Action::Quit)` (M1: no confirm; the editor is killed when `App`
       drops, `lib.rs:199`);
     - `Help` → `self.update(Action::ToggleHelp)`;
     - none → `self.status = Some(format!("{EDITOR_LOCKED}: {}", keys.hint(
       Stack::EDITOR_UNFOCUSED, LOCK_HINT)))` = `"the editor is open: Ctrl+4 to the editor ·
       Ctrl+x abort"`.
- **`abort_editor`**: `let editor = self.editor.take()`; `let tab = editor.tab; drop(editor);`
  (kill requested, temp removed) then `finish_external_edit(tab, Failed(EDITOR_ABORTED))` (B8).
- **`editor_paste`**: focused and visible → `child.write(&encode_paste(text,
  screen.bracketed_paste()))`; else dropped silently (the lock; a paste is never a key).
- **`draw_editor`**: `let rect = pane_rect(self.editor_area.get(), body)`; `self.editor_rect =
  self.tabs.active_id().map(|id| (id, rect))` (recorded every frame, pane or not, so the first
  spawn has the view's size). If the editor's tab is active: `focused = editor.focused &&
  self.overlays.is_empty() && !self.help_visible`; title (below);
  `ui::editor_pane::render(frame, rect, editor.screen.screen(), &title, focused, &self.theme)`.
  Another tab active → nothing drawn, no cursor.
- **Title**: focused `format!(" {} \u{b7} {} ", cmd.value(), keys.hint(Stack::EDITOR_FOCUSED,
  FOCUSED_HINT))` = `" nvim · Ctrl+4 to htui "`; unfocused `format!(" {} \u{b7} htui has the
  keys ", cmd.value())`.
- **`editor_status`**: `None` without an editor; focused `format!("the editor has the keys
  \u{b7} {}", keys.hint(Stack::EDITOR_FOCUSED, FOCUSED_HINT))`; unfocused
  `keys.hint(Stack::EDITOR_UNFOCUSED, UNFOCUSED_HINT)` = `"Ctrl+4 to the editor · Ctrl+x abort ·
  q quit · ? help"`.
- **`resize_editor`**: an editor, `editor_rect == Some((editor.tab, rect))`, `pane_size(rect) !=
  editor.screen.size()` → `child.resize(size)`, `screen.resize(size)`, `dirty = true`.

### 7.5 `event_loop.rs` (P7; PD-1)

Module doc: four arms; the post-steps are the `$EDITOR` step (suspend, or open the in-pane editor,
MOD-57), mouse capture, the dirty draw, then the pane resize (it needs the rect the draw just
claimed).

```rust
// MOD-57 P7: the in-pane editor's transport. The loop keeps a sender, so `recv` never ends.
let (pane_tx, mut pane_rx) = mpsc::unbounded_channel::<PaneEvent>();
...
tokio::select! {
    event = events.next() => …,                                   // unchanged
    Some(envelope) = replies.recv() => app.update(Action::Reply(envelope)),
    Some(event) = pane_rx.recv() => app.on_pane_event(event),
    _ = ticker.tick() => app.update(Action::Tick),
}
if app.should_quit { break; }
if let Some((tab, edit)) = app.take_external_edit() {
    // MOD-57 P3: `$VISUAL`/`$EDITOR` and `HTUI_EDITOR_PANE`, read once per edit through one lookup.
    let lookup = |key: &str| std::env::var(key).ok();
    let cmd = crate::editor::EditorCommand::resolve(lookup);
    match crate::editor::EditorMode::resolve(lookup) {
        crate::editor::EditorMode::Pane => app.open_editor(tab, &edit, cmd, |id, command, size| {
            PtyChild::spawn(command, size, id, pane_tx.clone())
                .map(|child| Box::new(child) as Box<dyn PaneChild>)
        }),
        crate::editor::EditorMode::Suspend => {
            drop(events);                                         // today's comment, unchanged
            let outcome = crate::editor::run_suspended(term, &cmd, &edit).await?;
            events = crossterm::event::EventStream::new();
            ticker.reset();
            app.finish_external_edit(tab, outcome);
        }
    }
}
term.set_mouse_capture(app.mouse_capture())?;
if std::mem::take(&mut app.dirty) {
    term.terminal_mut().draw(|frame| app.render(frame))?;
}
// MOD-57 PD-1: after the draw, which is what knows the pane's rect.
app.resize_editor();
```
The existing shape test stays green (`app.finish_external_edit(` is in the suspend arm, before
capture). New test `the_pane_is_an_arm_opened_by_the_editor_step_and_resized_after_the_draw`:
`app.on_pane_event(` present; `app.open_editor(` < `term.set_mouse_capture(` <
`std::mem::take(&mut app.dirty)` < `app.resize_editor();`; `EditorMode::resolve(` and
`std::env::var(` each appear exactly once; `EditorCommand::from_env()` absent.

### 7.6 T5 tests (`app::pane::tests`), TDD order

Fixtures: `FakeChild` (§4.4) behind `Rc<RefCell<Vec<Call>>>`; an `Editing` tab (id configurable)
that emits `EditExternally(asked())` on `e`, records every key, paste and outcome, draws `VIEW`
over its area, and claims a configurable `Option<Rect>` in `render`; `fn cmd(value) ->
EditorCommand` through `resolve`; `fn open(app, tab, log)` = `open_editor` with a fake spawn
that records `(argv, size)`; a `#[cfg(test)] fn editor_file(&self) -> Option<&Path>` helper;
`draw(app, w, h)` over a `TestBackend`. All tests are `#[test]` (no runtime, no child).

1. `a_focused_editor_gets_every_key_ctrl_c_included`: `a`, `ctrl-c`, `q`, `tab`, `?`, `esc`,
   `ctrl-x`, `f1`, `up` → writes `a`, `0x03`, `q`, `\t`, `?`, `0x1B`, `0x18`, `\x1bOP`, `\x1b[A`;
   `should_quit` false; the tab saw nothing; `help_visible` false.
2. `the_focus_key_toggles_and_is_never_written`: `ctrl-4` → no write, unfocused; `ctrl-4` →
   focused; refocusing clears `help_visible`.
3. `unfocused_ctrl_c_and_q_quit_at_once`: each → `should_quit`; nothing written; `drop(app)` →
   the fake logged `Dropped`.
4. `unfocused_every_other_key_is_refused_and_nothing_moves`: `?` toggles help; `j`, `1`, `tab`,
   `w`, `e`, `ctrl-s`, `esc` → status `"the editor is open: Ctrl+4 to the editor · Ctrl+x abort"`,
   active tab unchanged, the tab saw nothing, nothing written.
5. `abort_answers_failed_to_the_asking_tab_only`: two `Editing` tabs; abort → asker heard
   `[Failed(EDITOR_ABORTED)]`, the other nothing; `editor_open()` false; the fake logged `Kill`
   then `Dropped`; the temp file is gone; a later `Exited`/`Output` of the old id changes nothing
   (heard stays 1, `dirty` stays false).
6. `exited_reads_the_file_back_through_finish_external_edit`: write `new\n` to `editor_file()`;
   `Exited(Ok(with_exit_code(0)), 2 s)` → heard `[Edited("new\n")]`, file gone; second bench:
   `with_exit_code(3)` → `Failed` containing `exited with 3`.
7. `output_reaches_the_screen_and_replies_go_back`: `Output("\x1b[6n")` → write `\x1b[1;1R`;
   `Output("\x1b[?1h")` then `up` → `\x1bOA`.
8. `a_paste_is_forwarded_only_while_focused`: `"a\nb"` → `a\rb`; after `\x1b[?2004h` →
   bracketed; unfocused → nothing written and the tab saw no paste.
9. `a_second_edit_is_answered_editor_busy`: with an editor open, set `app.pending_edit` (visible
   in `app`) → `take_external_edit()` is `None`, the asker heard `[Failed(EDITOR_BUSY)]`, the
   editor is still open.
10. `open_editor_spawns_the_pty_command_at_the_drawn_size`: draw 100x30 with a 60x12 claim; open
    → argv is `pty_command`'s (unix: ends with the temp path, whose content is the handed text);
    size = `PaneSize::new(11, 60)`. Unclaimed → the body's size. Never drawn → `DEFAULT`.
11. `a_spawn_failure_answers_the_tab_and_opens_nothing`: fake spawn `Err(io::Error::other("no
    pty"))` → heard `[Failed(..)]` containing `no pty` and `HTUI_EDITOR_PANE`; no editor; temp
    gone. A temp-create failure is covered by T1.
12. `the_pane_draws_over_the_claimed_rect` (insta frame, 100x30): `Output` of a fixed screen; the
    claimed rect shows the title rule and the screen; the rest of the view draws around it.
13. `the_pane_falls_back_to_the_body_when_unclaimed_or_small`: no claim, a 30x5 claim, and a
    claim outside the body → the title rule sits on `chrome.body`'s first row.
14. `the_pane_is_drawn_only_in_its_tab_and_refocus_brings_it_back`: `app.tabs.focus(other)`
    directly → no title, no cursor; a key is refused (focus dropped, rule 1); `ctrl-4` → the
    asking tab is active and the editor focused.
15. `the_cursor_shows_only_when_focused_and_drawn`: focused → backend cursor shown at the pane's
    cursor; unfocused, `?` box up, or an overlay pushed → hidden.
16. `resize_follows_the_drawn_rect`: draw 100x30, `resize_editor()` → no call; draw 120x40 →
    `resize_editor()` → `Resize(new)`, `screen.size()` new, `dirty`.
17. `mouse_capture_is_off_while_an_editor_is_open`: a tab whose `wants_mouse` is true → `false`
    while open, `true` after `Exited`.
18. `the_title_status_and_help_come_from_the_keys`: exact focused/unfocused title and status
    strings; with `app.keys = Keys::defaults().with_chords(Context::Editor, Act::EditorFocus,
    &["f12"])` they read `F12`; the `?` box holds `Editor: Ctrl+4 editor focus · Ctrl+x abort
    edit` only while an editor is open.
19. `an_open_editor_debug_prints_no_text`: `{:?}` of `App` after `Output("SECRET")` and with a
    `SECRET` handed text contains neither.
20. `event_loop::tests::the_pane_is_an_arm_opened_by_the_editor_step_and_resized_after_the_draw`.

### 7.7 Hazards for T5

- `open_editor` runs on the UI task: a small temp-file write, `openpty` and one fork/exec, as
  `run` and `tokio::process` already do; no wait. Everything after it is queued or polled (R-NF-3).
- `finish_external_edit` drains; called from `take_external_edit` (busy) and `on_pane_event`, it
  may set `pending_edit` again, which the next loop turn takes. No recursion.
- `render` stays "read-only" towards views; `draw_editor` writes `editor_rect` (App's own state,
  as `dirty = false` already is).

### 7.8 T5 commits and gate

1. `feat(mod-57): T5 - the in-pane editor in App: claim, open, keys, paste, events, abort, draw`
   (`state.rs`, `app/pane.rs`, `app/mod.rs`, the snapshot; tests 1-19).
2. `feat(mod-57): T5 - the event loop's pane arm and post-steps` (`event_loop.rs`, test 20).

Gate: `cargo test -p htui --all-features --lib -- app:: event_loop::`; `cargo insta test -p htui
--all-features --check`; clippy both ways; `cargo doc -p htui --no-deps`; Windows cross-check.

---

## 8. T6: the four claims (P2; PD-3; F-1, F-11, F-12)

**First failing test**: `compose::tests::render_returns_the_body_rect`.

### 8.1 Edits

| Site | Change |
|---|---|
| `detail/compose.rs:543` `render` | `-> Rect`, returning `body` (the rect its `TextArea` is drawn in) as the last expression; doc: "Returns the body's rect, which the pane claims for an in-pane editor (MOD-57 P2)." No `#[must_use]` (the two test calls ignore it). |
| `detail/notes.rs:395` | `let body = compose::render(…); ctx.claim_editor_area(body); return;` |
| `detail/documents.rs:592` | the same |
| `item_form.rs:997` `render` | `-> Rect`, returning `body`; doc as above. `:1594` becomes `.draw(\|frame\| { render(frame, frame.area(), form, &Theme::default()); })` (F-11). |
| `backlog/mod.rs:936-944` | `Some(form) => ctx.claim_editor_area(item_form::render(frame, right, form, ctx.theme)),` (both arms stay `()`). |
| `skills/templates.rs:1159`, `:1174` | `render_editor(&self, frame, area, editor, ctx: &Ctx<'_>)` and `render_draft(&self, frame, left, editor, ctx: &Ctx<'_>)`: every `theme` becomes `ctx.theme`; `render_draft` ends with `ctx.claim_editor_area(inner);` (the text rect inside the draft's block, so the template's name and versions stay visible). `:517` passes `ctx`. |
| `skills/library.rs:1770`, `:1782` | the same; `:684` passes `ctx`. |

Browse-mode `E` (Templates, Library) claims nothing: the pane takes the tab body (F-12).

### 8.2 T6 tests (one per site, written first)

Each renders the view through its existing bench with `ctx.with_editor_area(&cell)` and asserts
`cell.get() == Some(rect)` where **the drawn buffer's row `rect.y`, from `rect.x`, starts with
the body's first line** (robust to layout tweaks), and `None` when the editing area is not drawn.
1. `compose::tests::render_returns_the_body_rect` (a note: `Rect::new(0, 1, 43, 21)` at 43x23;
   a typed document: below the kind, kinds, title and body-label rows).
2. `notes::tests::the_compose_area_claims_its_body_for_the_editor`.
3. `documents::tests::the_compose_area_claims_its_body_for_the_editor`.
4. `item_form::tests::render_returns_the_body_rect`.
5. `backlog::tests::the_item_form_claims_its_body_for_the_editor`.
6. `templates::tests::the_draft_claims_its_text_rect_and_browse_claims_nothing`.
7. `library::tests::the_draft_claims_its_text_rect_and_browse_claims_nothing`.

### 8.3 T6 commits and gate

1. `feat(mod-57): T6 - the compose area and the item form claim their body` (compose, notes,
   documents, item_form, backlog; tests 1-5).
2. `feat(mod-57): T6 - the Templates and Library drafts claim their text` (tests 6-7).

Gate: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui
--all-features --check` with **zero** changes (a claim draws nothing).

---

## 9. T7: end to end, docs, the manual run (P1-P10; PRD metrics)

### 9.1 `crates/htui/tests/editor_pane.rs` (new, `#![cfg(all(unix, feature = "testkit"))]`)

Real time (`#[tokio::test]`, never `start_paused`: memory "paused time + real child"). Helpers:
the Templates harness (`open`, `select`, copied minimal from `tests/templates.rs`), `script(dir,
name, body) -> EditorCommand` (via `EditorCommand::resolve` with `VISUAL`), and `pump(harness,
rx, until, 10 s)`: `timeout(rx.recv())` → `harness.app().on_pane_event(event)`, `harness.render()`,
then `resize_editor()`, until `until(&mut harness)`. The spawn closure is the loop's
(`PtyChild::spawn` with a clone of the test's sender) and records the argv for the temp path.

1. `ctrl_e_in_templates_edits_in_the_pane_end_to_end`: render once; `E` on `implement`;
   `take_external_edit()`; script `printf 'ready\n'; IFS= read -r line; printf '%s\n' "$line" >>
   "$1"`; `open_editor`; pump until the frame shows `ready` and `Ctrl+4 to htui`; keys `x`, `y`,
   `z`, `enter`; pump until `!editor_open()`; the frame shows the draft with `xyz` appended; the
   temp file is gone; `status` is `None`.
2. `ctrl_c_reaches_the_editor_and_abort_ends_it`: script `trap 'printf int > "$1.int"' INT;
   printf ready; while :; do sleep 0.1; done`; `ctrl-c` → `should_quit` false and `$1.int`
   appears (SIGINT through the PTY's line discipline to the child only); `ctrl-4`, `ctrl-x` →
   the view's notice is `EDITOR_ABORTED`; the script's pid is gone within 3 s.
3. `quitting_with_a_live_editor_leaves_no_child` (PRD metric): `printf '%s' $$ > pid; exec
   sleep 30`; `ctrl-4`, `q` → `should_quit`; `drop(harness)` → the pid is gone within 3 s.

### 9.2 Docs

- `docs/htui-editor.md` (new): choosing the editor (`$VISUAL`, `$EDITOR`, the `vi`/`notepad`
  fallback, the `exec` rule: a command and its arguments, no `VAR=x`, no `&&`); GUI editors and
  `--wait`; **in-pane mode** (`HTUI_EDITOR_PANE=1|true|yes`, per machine, works with `--demo`
  and `--offline`); the keys (`Ctrl+\`, shown as `Ctrl+4`, gives htui the keys and back;
  `Ctrl+x` aborts while htui has them; while the editor has the keys every key is its own,
  `Ctrl+C` included); the **M1 lock** (only those keys, `q`, `?` and `Ctrl+C` act; `q`/`Ctrl+C`
  quit at once and kill the editor; M2 opens the rest); where the pane draws (the view's editing
  area, the tab body below 40x8 or from a list); limits (the mouse is the terminal's own
  selection, no scrollback, the cursor shape is the terminal's, nvim's DA2/OSC queries go
  unanswered, a SIGHUP-ignoring editor may outlive a fast quit, nano may leave `*.md.save` after
  an abort, Windows unverified: MOD-16); rebinding through `keys.toml` once MOD-67 M2 lands.
- `README.md` "Further reading", after the `htui-secrets.md` line:
  ``- [`docs/htui-editor.md`](docs/htui-editor.md): `E`/`Ctrl+E`, `$VISUAL`/`$EDITOR`, and the in-pane editor (`HTUI_EDITOR_PANE`).``
- The plan: a "Manual run (T7)" section with nvim, vim and nano (draw, keys, resize, `ctrl-c`
  forwarded, exit, read-back; nvim's queries) and the `MIN_PANE` verdict (PRD Q3). The hr
  sandbox has no nvim: the maintainer runs it on the host.

### 9.3 T7 commits and gate

1. `test(mod-57): T7 - the in-pane editor end to end` (tests 1-3).
2. `docs(mod-57): T7 - docs/htui-editor.md, README line, manual-run notes`.

Gate: the plan's full Validation block (fmt, clippy both ways, `cargo test -p htui
--all-features -- --test-threads=1`, `cargo insta test --check`, Windows cross-check, the
workflow-docs validator). Then `ps -eo args | grep -c 'sleep 30'` is 0.

---
## 10. Data flow and cross-task contracts

**Open.** A view emits `Action::EditExternally(edit)` → `drain` stamps it (`pending_edit`, as
today) → the loop's post-step `take_external_edit()` (refuses with `EDITOR_BUSY` if an editor is
alive) → one `lookup`: `EditorCommand::resolve` + `EditorMode::resolve`. `Suspend` → today's
`run_suspended` path, unchanged. `Pane` → `App::open_editor(tab, &edit, cmd, spawn)` →
`TempEdit::create` → `cmd.pty_command(temp.path())` → `spawn` = `PtyChild::spawn(command, size,
id, pane_tx.clone())` → `OpenEditor { focused: true }`.

**Run.** Reader thread → `PaneEvent::Output` → fourth arm → `App::on_pane_event` →
`PaneScreen::feed` (DSR/DA1 replies → `PaneChild::write` → writer thread) → `dirty` → draw:
`App::render` clears the claim cell, the active tab claims its rect, `draw_editor` records
`editor_rect` and draws `ui::editor_pane::render` over it (cursor only when focused and drawn) →
`resize_editor` after the draw. Keys: `App::on_key` → `editor_key` (focused: `encode_key` →
`write`; unfocused: the M1 lock). Pastes: `editor_paste`.

**End.** Child exits → wait thread reaps → `PaneEvent::Exited` → `on_pane_event` →
`TempEdit::finish` → `App::finish_external_edit(tab, outcome)` → the view's `on_external_edit`,
unchanged (P1, P8). Abort: `abort_editor` → drop (kill requested via the wait thread, temp removed)
→ `finish_external_edit(tab, Failed(EDITOR_ABORTED))`. Quit: `App` drops in `lib.rs:199` → kill.

| Contract | Producer | Consumer | Pinned by |
|---|---|---|---|
| `TempEdit::{create, path, finish}`, `EditorExit` (+ both `From`s), `EditorMode::resolve`, `pty_command`, `PANE_VAR`, `EDITOR_BUSY`, `EDITOR_ABORTED` | T1 | T5, T7 | T1 tests 1-9 |
| `PaneId`, `PaneSize`, `PaneEvent` (with `portable_pty::ExitStatus`), `PaneChild`, `PtyChild::spawn`, `PaneScreen`, `encode_key`, `encode_paste` | T2 | T5, T7 | T2 tests 1-14 |
| `ui::editor_pane::render(frame, area, screen, title, focused, theme)` | T3 | T5 | T3 tests 1-6 |
| `Context::Editor`, `Act::{EditorFocus, EditorAbort}`, `Stack::{EDITOR_FOCUSED, EDITOR_UNFOCUSED}` | T4 | T5 | T4 tests |
| `Ctx::{with_editor_area, claim_editor_area}`; `App::{open_editor, on_pane_event, resize_editor, editor_open, editor_id}`; `MIN_PANE`, `EDITOR_LOCKED` | T5 | T6, T7 | T5 tests 1-20 |
| `compose::render`/`item_form::render` return the body `Rect` | T6 | notes, documents, backlog | T6 tests |
| `Ctx::new` and `App::new` signatures unchanged | T5 | every existing site | build |
| `Tab::on_external_edit` and every view's outcome handling unchanged | - | five views | existing suites green |

---

## 11. Hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-1 | **A real-child test leaves a process** (memory: orphaned test processes; the hr sandbox shares the box). | Every real-child test holds its `PtyChild` (drop kills); scripts `exec` their `sleep`; each gate ends with `ps -eo args \| grep -c 'sleep 30'` = 0. |
| H-2 | **pid reuse**: a pid-based SIGHUP after the wait thread reaped the child hits another process. | B5: every kill goes through the wait thread, which owns the child; the only pid-based signal is the spawn-error fallback, before any reap. |
| H-3 | **A SIGHUP-ignoring editor at quit** may outlive htui if the process exits inside the 200 ms grace. | `lib.rs:199-200` drops `App` then waits up to `SHUTDOWN` for the store worker, which usually covers the grace; process exit closes the master and the kernel hangs the session up. M2's quit confirm can wait for `Exited`. Documented. |
| H-4 | nano saves a modified buffer to `<file>.save` on SIGHUP: an abort or quit can leave `htui-*.md.save` in the temp dir. | Documented (T7). Not cleaned: the file is the user's text. |
| H-5 | **Debug hygiene**: `vt100::Screen` derives `Debug` with contents; `CommandBuilder` derives `Debug` with the environment (secrets). | Hand-written `Debug` on `PaneEvent`, `PaneScreen`, `PtyChild`, `TempEdit`, `OpenEditor`; `CommandBuilder` is never stored, logged or formatted. T2 test 7, T5 test 19, T1 test 6. |
| H-6 | The editor's cursor shape (DECSCUSR) and title (OSC 0/2) are not forwarded. | M1 scope; documented. The real terminal keeps its own cursor shape. |
| H-7 | An overlay a reply opens during an edit (migration prompt, connection redirect) cannot be answered until the edit ends (the M1 lock routes no key to it). | Accepted for M1 (P6 is the superset lock); M2 opens htui. While an overlay is up the cursor is hidden (§7.4). |
| H-8 | **Abort/exit race**: an `Exited` queued behind the abort key is dropped, and the saved text with it. | The user asked to abort; B4 makes the late event inert. Documented. |
| H-9 | Width disagreement between the outer terminal and the VT grid (VS16 emoji, halfwidth marks). | `ForcedWidth` from `is_wide()` keeps ratatui's diff on the grid (T3 test 2); the pane draws what the editor believes, as the editor would draw on that terminal itself. |
| H-10 | **Windows**: argv quoting (F-5), cwd = temp dir, `TerminateProcess` kill, ConPTY unverified. | Cross-check after T1, T2, T5; MOD-16's verification list gains "in-pane editor" (T7 docs). |
| H-11 | `ctrl-4` is labelled `Ctrl+4`; users press `Ctrl+\` (F-9). | Docs say both. MOD-67 M2 makes it rebindable. |
| H-12 | **Lints**: `missing_docs` on every new `pub` item; `missing_debug_implementations`; unused imports under `cfg(windows)`; featureless dead code (`#[cfg(test)]` helpers only in test modules). | Clippy both ways; Windows cross-check; `cargo doc`. |
| H-13 | New `.snap` files must be accepted and committed; `--check` fails on a pending one. | T3 and T5 commit theirs; T6 must produce none. |
| H-14 | **Suite scheduling** (memory): the process-wide keyring fake and real-time PTY tests. | Gate with `--test-threads=1`; no PTY test uses paused time. |
| H-15 | **MOD-67 M2-M5 conflicts** in `keys/`. | Appended blocks only (§6.4); merge-order note. |
| H-16 | The unbounded event channel grows if a child floods (a shell-as-editor `cat`ting a big file). | Editors redraw in bursts; accepted. 8 KiB reads coalesce output. **Superseded by R1 M-1 (2026-10-08):** measured, `cat` of 37 MB queued ~79k events and replayed 24.8 s of stale frames. The channel is now bounded (`mpsc::channel(PANE_QUEUE = 256)`; the reader and wait threads `blocking_send` from their std threads, so a flood waits in the child's write), and the pane arm applies up to `PANE_BATCH = 64` more queued events, within `PANE_BUDGET = 16 ms` of the one it woke for (R1 verify 1), before the one dirty draw. |
| H-17 | `vt100` with a 0-sized grid, or a resize while parsing. | `PaneSize::new` floors at 1x1; `resize` runs on the UI task between feeds. |
| H-18 | The sandbox has no nvim (only the probe's vim-tiny/nano under `/tmp/mod57-editors`, wiped by a sandbox restart). | The manual run is the maintainer's, on the host (T7). |

---

## 12. Count pins

| What | Before | After |
|---|---|---|
| `Act` variants / `CATALOGUE` rows | 41 | 43 (`editor` 2) |
| `Context` variants | 7 | 8 |
| `in_capture` rows | 5 | 6 (`editor.focus`) |
| `Stack` constants | 2 | 4 |
| `select!` arms in `event_loop::run` | 3 | 4 |
| Threads per live pane | - | 3 (read, write, wait) |
| Existing snapshots changed | - | 0 |
| New snapshots | - | 3 (T3: 2, T5: 1) |
| `Ctx::new` sites changed | - | 0 (the claim cell is opt-in through `with_editor_area`) |
| New files | - | `editor/pane.rs`, `ui/editor_pane.rs`, `app/pane.rs`, `tests/editor_pane.rs`, `docs/htui-editor.md` |

---

## 13. Plan deviations (for the maintainer: each changes plan text, none reopens P1-P10's intent)

| # | Plan text | Deviation | Evidence | End state |
|---|---|---|---|---|
| PD-1 | P7/T5: post-step order "editor step, resize, capture, draw" | editor step → capture → draw → **resize** | F-7: the rect is known after the draw | Same behaviour, no tick lag on a terminal resize |
| PD-2 | P7: "Writes to the PTY are synchronous and small" | A writer thread per pane; `write` queues `Zeroizing` chunks | F-2: a paste or a stalled child blocks `write` | R-NF-3 holds for pastes too |
| PD-3 | P2/T6: claims inside `compose::render` and `render_draft` | `compose::render` and `item_form::render` return the body `Rect`; callers claim. `render_editor`/`render_draft` take `ctx`. T6 adds `notes.rs`, `documents.rs` | F-1 | Same four claims, same rects |
| PD-4 | T2 test "→ `TempEdit::finish` answers `Edited`" | T2 asserts the file; `TempEdit` end to end in T5 (fake) and T7 (real) | F-6 | Same coverage, T1 ∥ T2 kept |
| PD-5 | T3 test "title … labels from `Keys::label`" | The widget takes a title string; T5 builds and tests it (`Keys::hint`) | F-6 | Same coverage, T3 ∥ T4 kept |
| PD-6 | P9 "the same argv as `command()`" | Unix: same argv **plus** `cwd = current_dir`, `LINES`/`COLUMNS` removed. Windows: `cmd /S /C "<value> <file name>"` with `cwd` = temp dir | F-4, F-5 | Same editor, same file; Windows quoting is MOD-16's |
| PD-7 | Gotchas: "teardown calls `ChildKiller::kill` (`clone_killer()`, SIGHUP then escalation)" | The wait thread owns the child and does the escalating kill on request or drop; no `clone_killer` except the spawn-error fallback | F-3 | Escalation real, no pid reuse, no UI-task sleep |
| PD-8 | P6: `EDITOR_BUSY` on the status line, in `drain` | Answered to the asking tab as `Failed(EDITOR_BUSY)` from `take_external_edit` | F-8 | The view's handed-out state stays consistent |
| PD-9 | Files table: T5 = `state.rs`, `event_loop.rs` | Plus a new `app/pane.rs` and its `mod` line in `app/mod.rs` | `state.rs` is 900+ lines; the editor logic is one concern | Same API on `App` |

The plan's T2 `pane::` validate filter becomes `editor::pane` (the module path, §1).

---

## 14. Blueprint decisions

- **B1 Placement.** `crate::editor::pane` (process side, screen, encoder; T2),
  `crate::ui::editor_pane` (widget; T3), `crate::app::pane` (shell; T5).
- **B2 The pane abstraction is a three-method trait over the process only** (`PaneChild`); the
  parser, temp file and command are real in every `App` test. Chosen over an enum because the
  fake lives in tests and the real one in the event loop; the trait keeps `App` ignorant of
  portable-pty's types except `CommandBuilder` in the spawn closure.
- **B3 The spawn is injected** (`open_editor(.., spawn: impl FnOnce(PaneId, CommandBuilder,
  PaneSize) -> io::Result<Box<dyn PaneChild>>)`), so temp-file creation, sizing and failure
  routing are `App` code under test, and only `PtyChild::spawn` needs a real PTY.
- **B4 `PaneId` generations** make late `Output`/`Exited` events of a finished or aborted editor
  inert, so no event needs ordering against a key.
- **B5 Kills go through the wait thread**, which owns the child: `try_wait` every 25 ms or a kill
  request, then portable-pty's escalating `Child::kill` and `wait`. No pid signal from the UI
  task; `Drop` = kill request.
- **B6 Three std threads per pane** (read, write, wait), named `htui-pane-*`; input chunks are
  `Zeroizing<Vec<u8>>` (a paste may be a credential, MOD-22).
- **B7 `Exited` carries `portable_pty::ExitStatus`**; `editor.rs` owns the conversion to
  `EditorExit`, which both runners feed into `TempEdit::finish`.
- **B8 Abort is immediate**: it answers at the key and leaves the dying child to the wait thread.
- **B9 The cursor is only ever `Frame::set_cursor_position`**: focused, drawn, no overlay, no
  `?` box, not hidden by the child. `TerminalGuard` is untouched.
- **B10 Mouse capture is off while an editor is alive** (`wants_mouse`).
- **B11 Titles, status lines and the refusal are `Keys::hint` over the editor stacks** with
  `HintSpec` constants, so a rebind (MOD-67 M2) relabels them; `EDITOR_LOCKED`, `EDITOR_BUSY`,
  `EDITOR_ABORTED` are the fixed sentences.
- **B12 `EDITOR_BUSY` is answered from `take_external_edit`** (PD-8).
- **B13 The claim cell** is cleared per frame, lent only to the active tab's render `Ctx`,
  intersected with the body, and honoured only when it is ≥ `MIN_PANE` and the active tab is the
  editor's; `editor_rect` is recorded every frame so the first spawn has the view's size.
- **B14 The pane frame is a top rule** (`Borders::TOP` with the title): the editor keeps the full
  width, and the view's own block around a claimed rect stays visible.
- **B15 `editor.focus` is a `capture_row`**: the editor captures every key, so its chord must be
  non-printable, which the existing `in_capture` invariant and M2's validator already enforce.
- **B16 Refocus brings the asking tab to the front and closes the `?` box**, so a focused editor
  is always the one on screen.

## Files

### Files to create

| File | Purpose | Task |
|---|---|---|
| `crates/htui/src/editor/pane.rs` | `PaneId`, `PaneSize`, `PaneEvent`, `PaneChild`, `PtyChild`, `PaneScreen`, `encode_key`, `encode_paste`; 14 tests | T0 (stub), T2 |
| `crates/htui/src/ui/editor_pane.rs` | the widget; 6 tests, 2 snapshots | T0 (stub), T3 |
| `crates/htui/src/app/pane.rs` | `OpenEditor`, `MIN_PANE`, `EDITOR_LOCKED`, `App`'s editor methods; 19 tests, 1 snapshot | T5 |
| `crates/htui/tests/editor_pane.rs` | real-child end to end (3) | T7 |
| `docs/htui-editor.md` | user guide | T7 |

### Files to modify

| File | Changes | Task |
|---|---|---|
| `Cargo.toml`, `crates/htui/Cargo.toml`, `Cargo.lock` | `portable-pty 0.9.0`, `vt100 0.16.2` | T0 |
| `crates/htui/src/editor.rs` | `pub mod pane;` (T0); `TempEdit`, `EditorExit`, `EditorMode`, `pty_command`, `PANE_VAR`, `EDITOR_BUSY`, `EDITOR_ABORTED`, `run` over `TempEdit` (T1) | T0, T1 |
| `crates/htui/src/ui/mod.rs` | `pub mod editor_pane;` | T0 |
| `crates/htui/src/keys/catalogue.rs` | `Context::Editor`, two acts, two rows, test updates | T4 |
| `crates/htui/src/keys/stack.rs` | `EDITOR_FOCUSED`, `EDITOR_UNFOCUSED`, 4 tests | T4 |
| `crates/htui/src/keys/mod.rs` | 2 tests | T4 |
| `crates/htui/src/app/state.rs` | `Ctx` claim, three `App` fields, `take_external_edit`, `on_key`, `on_paste`, `wants_mouse`, `render`, `render_help` | T5 |
| `crates/htui/src/app/mod.rs` | `mod pane;`, re-exports | T5 |
| `crates/htui/src/event_loop.rs` | fourth arm, mode switch, resize after the draw, module doc, shape test | T5 |
| `crates/htui/src/ui/tabs/backlog/detail/compose.rs` | `render -> Rect` | T6 |
| `crates/htui/src/ui/tabs/backlog/detail/notes.rs`, `documents.rs` | claim the compose body | T6 |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | `render -> Rect`; test closure at `:1594` | T6 |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | claim the item form's body | T6 |
| `crates/htui/src/ui/tabs/skills/templates.rs`, `library.rs` | `render_editor`/`render_draft` take `ctx`; claim the draft text | T6 |
| `README.md` | "Further reading" line | T7 |
| `.claude/plans/mod-57-m1-editor-in-pane.plan.md` | manual-run notes, `MIN_PANE` verdict | T7 |
