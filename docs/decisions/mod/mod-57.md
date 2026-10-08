# MOD-57 - Run the external editor inside the TUI pane (done, 2026-10-08)

**Requirements:** `R-TUI-1`, `R-TUI-7`, `R-TUI-10`, `R-NF-1`; keys per ANA-26 §6.4 and C3
(`docs/ANA-26.md`).
**Origin:** MOD-9 (`docs/decisions/mod/mod-9.md`), at the merge of MOD-7 milestone 2,
maintainer-decided 2026-09-26.
**Artifacts:**
- PRD `.claude/prds/mod-57-embedded-editor-pane.prd.md` (decisions D1-D6, two milestones);
- M1 plan `.claude/plans/mod-57-m1-editor-in-pane.plan.md` (P1-P10, PD-1..PD-11) and its blueprint;
- M2 plan `.claude/plans/mod-57-m2-live-editor-across-htui.plan.md` (E1-E10, PD-M2-1..3, reviews R1
  and R2) and its blueprint.

Each plan carries a verified-claims table; decision numbers are local to each plan. The item was
routed as **PRD** (C2, C4); M1 ran with ultracode for implement and review, M2 was routed as
**plan** on the PRD's milestone 2 and ran with plain agents (two parallel lanes, below the ultracode
bar).

User guide: `docs/htui-editor.md`.

## The problem

`E`/`Ctrl+E` (Skills Templates and Library, the Backlog item form, Documents, Notes) suspended the
whole TUI and gave the terminal to `$VISUAL`/`$EDITOR` until it exited. For the whole edit htui was not
drawn: run progress, chat activity, a permission ask went unseen and could not be acted on.

## What was built

### M1 - the editor in the pane (`1c8c9e3c`..`a464ac21`, 2026-10-07/08)

Opt-in with `HTUI_EDITOR_PANE=1` (default: suspend, unchanged). The editor runs on a pseudo-terminal
(`portable-pty` 0.9, ConPTY on Windows) feeding a VT parser (`vt100` 0.16) drawn by htui's own widget
(`ui::editor_pane`, not `tui-term`: it gets wide cells wrong and does not place the real cursor). It
draws over the asking view's editing area (a claim through `Ctx`), or the tab body below 40x8
(`MIN_PANE`, kept after a manual vim/nano run at 7x40). Keys: catalogue context `editor`,
`editor.focus` (`ctrl-4`, which `Ctrl+\` sends on unix) and `editor.abort` (`ctrl-x`). Focused,
every key goes to the editor, `ctrl-c` included (ANA-26's one exception); unfocused, M1 locked htui
to focus, abort, quit and help. The file is read back through the same `TempEdit::finish` gate as
the suspend mode, so no view's outcome handling changed. A unix process-group kill (`rustix`,
PD-10) ends the editor on abort and quit. Review R1 fixed a `vt100` ICH/IL/SD stall on the UI task
(a stateful CSI clamp), added back-pressure (a bounded channel, a time-boxed drain), a "lost track"
sentence for a failed wait, and `ClosePseudoConsole` off the UI task on Windows.

### M2 - a live editor across htui (`e6d83478`..`ccf06c9a`, 2026-10-08)

- **The asking tab alone stays locked (E3).** With focus in htui every other tab switches and works,
  the mouse included (E8); on the asking tab only the editor keys and the global actions act
  (`Stack::EDITOR_OPEN` = `[editor, global]`), anything else is refused on the status line.
  `editor.focus` returns to the editor from any tab (E1); `editor.abort` acts only on the asking tab
  (E2).
- **Quit asks first (E4, PRD D5).** `global.quit` with an editor alive opens a modal question
  (`ui/overlay/quit_confirm.rs`, stack `views::QUIT_CONFIRM`): `y` quits and kills the editor, `n`/
  `Esc` keep editing. `ctrl-c` with htui holding the keys still quits at once (ANA-26 C3), under the
  question too. Every quit route goes through `App::request_quit` from `action_for`.
- **Abort asks first (review R1 M-2).** On the asking tab `editor.abort` opens "Abort the edit and
  lose it?" (`ui/overlay/abort_confirm.rs`, `views::ABORT_CONFIRM`); only a fresh press of the abort
  key aborts, any other key keeps the edit and is consumed, so nano's habitual `ctrl-x`, `y` keeps it.
  The question ignores key repeat, closes when the editor exits or its tab leaves the screen, and
  only the question itself may emit `Action::AbortEdit` (review R2).
- **Nothing steers the asking tab while it waits (E5).** A workspace switch (`SetScope`) and a reveal
  into the asking tab are refused ("the editor is open in <tab>: finish or abort it first"). Settings
  > Hierarchy's own scope refreshes are refused too and not retried (documented).
- **Status line and help (E6, E7).** Other tabs show `editing in <Tab> · Ctrl+4 back · ` before their
  own line; the `?` box lists the editor keys that act where the user is (PD-M2-2).
- **Key file rules (E9; the follow-ups M1 left to "whichever of MOD-57 and MOD-67 M2 lands
  second").** `[editor] focus` must keep a chord and never gives way; the focus chord is refused in
  every other table; `[editor]` chords are checked against every global ("while an edit is open");
  `ctrl-c` stays unbindable there. Review R1 M-1: a `[global]` entry may take `editor.abort`'s
  default chord, with the give-way notice and a `--print-keys` mark, so an existing
  `quit = ["ctrl-x"]` still loads. The README example moved to `ctrl-g`.

## Reviews (M2)

- **R1** (`rust-reviewer` over `0aea5312..7b57be39`): approve with changes, no CRITICAL/HIGH. The
  maintainer took every finding: M-1 (`74c7cbb8`), M-2 abort question confirmed by the abort key again
  (`a07f94f1`), M-3 tests (`deecb5c0`), L-1 a neutral sentence under any modal overlay (`f62279b5`),
  L-2 (`e3421216`), L-3 no frame for off-screen output (`18b0b4a5`), the nits (`0829146c`); L-4 (the
  Hierarchy refreshes) is documented.
- **R2** (focused re-review of R1): approve. Its LOWs were applied: the abort question ignores key
  repeat, `Action::AbortEdit` only from the question, the question closes on a tab move, two test
  gaps, two nits (`5543c59e`, `7ccb5488`, `19a1f7b0`, `ccf06c9a`).

Known residue: M1 R1 L-2 (stray keystrokes into the child if a PTY thread fails to spawn; error path
only), deferred by the maintainer.

## Gate

The final code tree `ccf06c9a` passed in the sandbox: `cargo fmt --check`; clippy `-D warnings` with all features and
without; `cargo test -p htui --all-features -- --test-threads=1` (3013 passed, 0 failed, 3 ignored); `cargo insta test
--check`; the Windows cross-check (`x86_64-pc-windows-gnu`, mingw in `/tmp/mingw`);
`validate-workflow-docs.sh`. `cargo doc` still fails on 9 private/unresolved intra-doc links, all
from commits before M2 (CLEAN-10); M2 added none.

## Owed on the host

- **An nvim run in the pane** (sandbox has no nvim): draw, keys, resize, `ctrl-c` forwarded, exit and
  read-back in Templates, the Notes compose box and the item form; nvim's start-up queries; the
  `MIN_PANE` verdict in the Notes compose box.
- **The PRD's hypothesis:** the next 10 external edits in the pane mode, no edit lost or needing the
  suspend mode.
- **Windows (ConPTY)** is MOD-16's: the focus key is a physical `Ctrl+4` there (`Ctrl+\` arrives as
  `ctrl-\`), the kill is `TerminateProcess`, and the quit and abort questions with a ConPTY child.
