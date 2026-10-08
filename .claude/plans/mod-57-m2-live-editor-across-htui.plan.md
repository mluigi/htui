# Plan: MOD-57 M2 — a live editor across htui, and close-out

**Source PRD**: `.claude/prds/mod-57-embedded-editor-pane.prd.md`
**Selected Milestone**: 2 — Live editor across htui, close-out
**Complexity**: Medium
**Routing**: plan path on the PRD's milestone 2 (`/handoff-run MOD-57`, accepted 2026-10-08); ultracode
for implement recommended at low confidence, to be taken only if the fact-check shows ≥3 disjoint lanes
(it does not: see Independence).
**Status**: CONFIRMED (2026-10-08) — E1-E10 as proposed; Q-A as proposed (close at M2, nvim run and the 10-edit hypothesis owed on the host); no ultracode (plain agents, T1 ∥ T2).

## Summary

M1 opened `$VISUAL`/`$EDITOR` in the asking view's pane but locked htui down while the editor was
unfocused (focus, abort, quit, help). M2 opens the rest of htui (PRD D4): with focus in htui every
tab switches and works, the asking tab alone stays locked, the status line says an edit is open,
`editor.focus` brings the editor back from anywhere, and `global.quit` asks before it kills the
editor (D5; `ctrl-c` still quits at once). It also lands the `keys.toml` loader rules M1 left to
"whichever of MOD-57 and MOD-67 M2 lands second" (M1 blueprint §6.4), then closes MOD-57.

## Decisions (proposed; confirmed at the CONFIRM gate)

| # | Decision | Why |
|---|---|---|
| E1 | **`editor.focus` acts from every screen** while an editor is alive and htui has the keys (not under a modal overlay, whose keys stay the overlay's, M1 T5 note 2). Refocusing from another tab brings the asking tab back (M1 B16, already in `editor_key`). | PRD D3/D4: "the same action refocuses". From another tab it is the only way back short of switching tabs by hand. |
| E2 | **`editor.abort` acts only on the asking tab** (on screen, unfocused). Elsewhere its chord falls through to the view as any key does. | Abort is for a stuck editor you can see; offering `ctrl-x` on every screen would shadow it in every view for the whole edit. |
| E3 | **The asking tab stays locked; every other tab is free.** On the asking tab, unfocused, a key resolves through `Stack::EDITOR_OPEN` (`[editor, global]`, every global action, not M1's `{quit, help}`); a key nothing resolves is refused with M1's lock sentence and never reaches the view. On any other tab `on_key` runs as if no editor were alive, after E1's check. | M1 P6's reason stands for the asking view only: a `Ctrl+S` or a selection change there would save stale text or misroute the outcome. Tab switching, the queue, find, waiting, help are globals and now act from the asking tab too. |
| E4 | **Quit confirms (PRD D5).** `global.quit` with an editor alive opens a modal `QuitConfirm` overlay (`ui/overlay/quit_confirm.rs`, shaped like `MigrationPrompt`, stack `views::QUIT_CONFIRM` = `[confirm ∩ {yes, no}, overlay, global ∩ {help}]`): `y` quits (the editor is killed when `App` drops, as today), `n`/`Esc` close. `ctrl-c` with htui holding the keys quits at once (ANA-26 C3), the confirm included. `Action::Quit` keeps meaning "quit now"; the shell's `Act::Quit` mapping goes through one `App::request_quit` that opens the confirm when an editor is alive. | D5 as settled at the PRD gate (Q5). One mapping point (`action_for`, plus the lock's own `Quit` arm) keeps every quit route covered; `ctrl-c` and the confirm's `y` stay the direct `Action::Quit`. |
| E5 | **Nothing steers the asking tab while it waits.** With an editor alive, `Action::SetScope` (the workspace switcher's pick; `set_scope` calls every tab's `on_scope_change`) and an `Action::Reveal` whose target tab is the asking tab are refused on the status line ("the editor is open in <tab>: finish or abort it first"). `Replay`/`Promote` target the Chat tab, which never hands off, and pass. | E3's reason, for the two shell actions that reset or re-select a tab without a key reaching it. |
| E6 | **Status line.** On the asking tab: M1's lines, unchanged. On another tab: that tab's own status line with a short prefix naming the open edit and the way back, generated through `Keys::hint` (e.g. `editing in Skills · Ctrl+4 back · ` + the tab's line). A `self.status` error still wins as today. | PRD M2 outcome "status-line notice" without hiding the free tab's own hints. |
| E7 | **The `?` box lists the `editor` context while an editor is alive in every view**, converted ones included (today only the unconverted path adds it, `state.rs` `render_help`). | Help must name the way back wherever the user is. |
| E8 | **Mouse on other tabs.** `wants_mouse` stops requiring `editor.is_none()`: it requires that the editor is not on screen (the asking tab is not the active one). | D4 "anything" includes the Runs flow view's mouse; on the asking tab the pane draws over the view, so the mouse stays off there. |
| E9 | **Loader rules owed from M1 (blueprint §6.4).** (a) `editor.focus` must keep a chord, beside `overlay.close` (`load.rs`); (b) `editor.focus` never gives way under MOD-12 R1 H1's rule (`validate::give_way`), so `[editor] abort = "ctrl-4"` is a collision error instead of silently unbinding focus; (c) `Stack::EDITOR_OPEN` joins `DECLARED` ("while an edit is open"), which checks `[editor]` against every global chord (M2 lets every global act beside the editor), so `[global] quit = "ctrl-x"` is refused; (d) one pass refuses an `editor.focus` chord bound to any other action in any context (E1 makes it global for the edit's life); (e) a test pins the `ctrl-c` refusal in `[editor]` (already enforced: `chord_of` runs for every table) and a `--print-keys` round-trip with `[editor]` changed. | ANA-26 §6.4: the leave action must stay bound. (b) and (c) are live defects today (fact-check). |
| E10 | **R1 L-2 stays deferred** (stray keystrokes on a thread-spawn failure, error path only); recorded in the decision record. | Maintainer deferral at M1 R1. |

## Open question for the maintainer (at CONFIRM)

- **Q-A Close-out with the host checks owed.** The nvim run (M1 "Owed by the maintainer") and the
  PRD's 10-edit hypothesis cannot run in the sandbox. Proposed: close MOD-57 at M2 and record both
  as owed in the write-up and the HANDOFF status line (as MOD-12 did with its host Postgres gate),
  rather than keep the item open.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui/src/app/pane.rs:62-90` | Constants for user-facing sentences (`EDITOR_LOCKED`, `PROMPT_FIRST`), hints as `HintSpec` consts rendered through `Keys::hint`. |
| Errors | `crates/htui/src/app/pane.rs:273-300` | Refusals land on `self.status`; nothing panics; the asking tab is answered only through `finish_external_edit`. |
| Key dispatch | `crates/htui/src/app/state.rs:775-898` | `on_key` order: editor step, C3, overlay, tab, legacy keymap, stack. M2 changes only the editor step's tail and `action_for(Act::Quit)`. |
| Overlay | `crates/htui/src/ui/overlay/migration_prompt.rs` | Modal yes/no box: `HINT` const, `key_stack()` from `keys::views`, `on_key` over `ctx.keys().actions(..)`, registered in `app/mod.rs` `register_all`. |
| Keys | `crates/htui/src/keys/stack.rs:183-205`, `DECLARED` `:210` | Stacks as `const` layer slices with the reason cited; every stack the validator walks listed once with its phrase. |
| Loader | `crates/htui/src/keys/load.rs:418-425`, `validate.rs:27-57` | `overlay.close` must keep a chord and never gives way: the two rules `editor.focus` joins. |
| Tests (unit) | `crates/htui/src/app/pane.rs` tests (`shell`, `opened`, fake `PaneChild`) | `App` driven by `on_key`, a fake child recording writes; no real process. |
| Tests (keys) | `crates/htui/src/keys/validate.rs` tests (`errors`, `one`) | Error text asserted with its line. |
| Tests (e2e) | `crates/htui/tests/editor_pane.rs` (`quitting_with_a_live_editor_leaves_no_child`) | Real time, scripted `sh` editor, pid probe. |
| Snapshots | `crates/htui/src/ui/overlay/snapshots/` + `insta` | A `TestBackend` frame per new box. |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui/src/keys/stack.rs` | UPDATE | `EDITOR_UNFOCUSED` becomes `EDITOR_OPEN` (`[editor, global]`); `DECLARED` entry | T0 (const), T1 |
| `crates/htui/src/keys/views.rs` | UPDATE | `QUIT_CONFIRM` stack; `DECLARED` entry | T0 |
| `crates/htui/src/keys/load.rs` | UPDATE | E9 (a) | T1 |
| `crates/htui/src/keys/validate.rs` | UPDATE | E9 (b), (d) | T1 |
| `crates/htui/src/keys/mod.rs` | UPDATE | E9 tests; `the_editor_keys_do_not_collide_with_quit_or_help` widened | T1 |
| `crates/htui/src/keys/print.rs` | UPDATE | its round-trip test binds `quit = "ctrl-x"`, which E9 (c) refuses; another chord, plus an `[editor]` round-trip | T1 |
| `crates/htui/src/ui/overlay/quit_confirm.rs` | CREATE | E4 | T2 |
| `crates/htui/src/ui/overlay/mod.rs` | UPDATE | `pub mod quit_confirm; pub use` | T0 (line), T2 |
| `crates/htui/src/ui/overlay/snapshots/*` | CREATE | the confirm box | T2 |
| `crates/htui/src/app/pane.rs` | UPDATE | E1-E3, E6 (prefix), module doc M1→M2 | T3 |
| `crates/htui/src/app/state.rs` | UPDATE | `on_key` editor step for E1/E3 on other tabs; `action_for(Act::Quit)` → `request_quit`; status prefix (E6); help (E7); `wants_mouse` (E8); `quit_overlay` field | T3 |
| `crates/htui/src/app/update.rs` | UPDATE | E5 refusals in `update` (`SetScope`, `Reveal`) | T3 |
| `crates/htui/src/app/mod.rs` | UPDATE | register `QuitConfirm`, `app.quit_overlay` | T3 |
| `crates/htui/src/app/snapshots/*` | UPDATE/CREATE | status-line and `?` frames with an editor alive | T3 |
| `crates/htui/tests/editor_pane.rs` | UPDATE | e2e: switch tabs and act while the editor lives, return, read back; quit confirm `y` leaves no child | T3 |
| `docs/htui-editor.md` | UPDATE | M1 lock section → M2 behaviour; quit confirm; key file rules | T4 |
| `README.md` | UPDATE | only if its key/editor lines name the M1 lock (fact-check: none) | T4 |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-57.md` | UPDATE/CREATE | close-out (P2); MOD-67 note's "whichever lands second" resolved; MOD-16 entry gains the quit confirm on ConPTY | T4 |
| `.claude/prds/mod-57-embedded-editor-pane.prd.md` | UPDATE | M2 row complete | T4 |

## Tasks

TDD per repo convention: each task writes its failing tests first.

### T0 — names before the fan-out (main thread)
- **Action**: add `views::QUIT_CONFIRM` (`[CONFIRM, OVERLAY, HELP]`: `views::MIGRATION` without its view layer, fact-check) and its `DECLARED` line; rename `Stack::EDITOR_UNFOCUSED` → `Stack::EDITOR_OPEN` with the M2 composition (`[editor, global]`) and update its two M1 users and tests; `pub mod quit_confirm;` with an empty stub. One commit.
- **Validate**: `cargo test -p htui --all-features keys::` (the compiled defaults still validate: `ctrl-4`/`ctrl-x` collide with no global default).

### T1 — loader and validator (parallel with T2)
- **Action**: tests first, in `load.rs`/`validate.rs`/`keys/mod.rs`/`print.rs`: `[editor] focus = []` refused on its line ("must keep at least one chord"); `[editor] abort = "ctrl-4"` refused as a collision, focus keeps `ctrl-4`; `[global] quit = "ctrl-x"` refused "while an edit is open"; `[list] down = "ctrl-4"` refused (E9 d); `[editor] focus = "ctrl-c"` refused with `CTRL_C_REFUSED`; `[editor] focus = "f12"` round-trips through `--print-keys` with its `(changed)` mark. Then E9 (a)-(d); `print.rs`'s `quit = "ctrl-x"` fixture moves to a free chord.
- **Mirror**: the `overlay.close` rules (`load.rs:418`, `validate.rs:33`).
- **Validate**: `cargo test -p htui --all-features keys::`

### T2 — the quit confirm (parallel with T1)
- **Action**: tests first: the question names the editor's value when given one, never blank; `y` emits `Action::Quit`; `n` and `Esc` close and quit nothing; any other key is swallowed (modal); snapshot of the box. Then the overlay.
- **Mirror**: `MigrationPrompt` whole.
- **Validate**: `cargo insta test -p htui --all-features -- quit_confirm`

### T3 — the shell (after T1, T2)
- **Action**: tests first in `app::pane::tests` (fake child): unfocused on the asking tab, a tab-switch chord switches and the editor keeps running and is not drawn; on another tab its keys reach it (a recording tab) and `editor.focus` returns to the asking tab focused; `editor.abort` on another tab is the view's key, on the asking tab aborts; on the asking tab a non-global key is refused and never reaches the tab; `q` opens the confirm, `y` quits, `n` leaves the editor alive and unfocused; `ctrl-c` quits at once on any tab and under the confirm; `SetScope` and a `Reveal` into the asking tab are refused, a `Reveal` into another tab passes; a second `E` on another tab answers that tab `EDITOR_BUSY`; the status line on another tab carries the prefix; the `?` box lists the editor keys over a converted view; `wants_mouse` on another tab. The M1 tests that pinned the lock's refusal of tab switching are rewritten, not deleted. Then the code. e2e in `tests/editor_pane.rs`: open, unfocus, switch tab, act, refocus, save, read back; quit through the confirm leaves no child.
- **Validate**: `cargo test -p htui --all-features app::` and `--features testkit --test editor_pane`

### T4 — docs and close-out (after T3)
- **Action**: `docs/htui-editor.md` M2 sections; review gate (`rust-reviewer`) over T0-T4; P2 close-out per `references/lifecycle.md`; PRD row; validator.
- **Validate**: full gate below.

### Independence (file-set intersection)

| Task | Files |
|---|---|
| T1 | `keys/load.rs`, `keys/validate.rs`, `keys/mod.rs`, `keys/print.rs` |
| T2 | `ui/overlay/quit_confirm.rs` (new), its snapshots |
| T3 | `app/pane.rs`, `app/state.rs`, `app/update.rs`, `app/mod.rs`, app snapshots, `tests/editor_pane.rs` |
| T4 | docs and bookkeeping only |

T1 ∩ T2 = ∅ once T0 has written the shared names (`stack.rs`, `views.rs`, `ui/overlay/mod.rs`). T3
reads T1's rules and T2's overlay, so it follows both; T4 follows T3. Two parallel lanes then two
serial ones: **ultracode is not worth it** (below the ≥3-disjoint-lanes bar the route set); plain
agents, T1 and T2 in parallel.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate
cargo test -p htui --all-features -- --test-threads=1        # scheduling-dependent suite
cargo insta test -p htui --all-features --check               # every snapshot
CC_x86_64_pc_windows_gnu=/tmp/mingw/mgcc AR_x86_64_pc_windows_gnu=/tmp/mingw/mar cargo check -p htui --target x86_64-pc-windows-gnu
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A path other than a key steers the asking tab (a store reply, a timer) | Medium | Replies already reached it in M1 and its pending-handoff state (`Pending { resume }`) absorbs them; E5 covers the two shell actions that reset or re-select a tab |
| `ctrl-4` (E1) shadows a user binding in a view for the edit's life | Low | E9 (d) refuses it at load |
| Snapshot churn in `app/` frames from the status prefix | Medium | Prefix only with an editor alive; no editor-less frame changes; full `insta` run |
| The quit confirm under a stuck editor that ignores SIGHUP | Low | `y` drops `App`; M1's group kill escalates to KILL (PD-10) |
| `/tmp/mingw` gone after a sandbox restart | Medium | Re-extract per memory recipe before the Windows check |

## Acceptance

- [ ] All tasks complete, TDD order kept
- [ ] Validation passes (every command above)
- [ ] With `HTUI_EDITOR_PANE` unset nothing changes (no editor-less snapshot moves)
- [ ] Patterns mirrored, not reinvented
- [ ] Owed on the host and recorded: nvim run, the 10-edit hypothesis (Q-A)

## Verified claims (plan fact-check)

Step 3.5, 2026-10-08. Loader claims by a compile probe (a throwaway `keys::probe_m2` test against
`load_str`, run with `--all-features`, then removed); the rest against the tree. One claim amended.

| Claim | Verdict | Evidence |
|---|---|---|
| `[editor] focus = []` loads today (E9 a is owed) | TRUE | probe: `ok=true`; `load.rs:418` checks only `Act::OverlayClose` |
| `[editor] abort = "ctrl-4"` silently unbinds `editor.focus` (E9 b) | TRUE (live defect) | probe: loads, `focus chords=[]`; `give_way` (`validate.rs:27-57`) exempts only `OverlayClose` |
| `[global] quit = "ctrl-x"` loads unchecked (E9 c) | TRUE | probe: `ok=true`; neither editor stack is in `DECLARED` (`stack.rs:210-257`) |
| `[list] down = "ctrl-4"` loads (E9 d) | TRUE | probe: `ok=true` |
| `ctrl-c` is already refused in `[editor]` (E9 e needs a test only) | TRUE | probe: `line 2: [editor] focus = "ctrl-c": ctrl-c always quits and cannot be bound`; `entry` → `chord_of` for every table (`load.rs:241-252`) |
| `Context::Editor` arms (table list, `--print-keys`) already present | TRUE | `load.rs` `TABLES` test string names `editor`; `print.rs:27` iterates `Context::ALL`; `catalogue.rs:126,157` |
| No default chord besides the editor rows is `ctrl-4` or `ctrl-x` (T0's `EDITOR_OPEN` keeps the defaults valid) | TRUE | probe over `Keys::compiled()`: only `EditorFocus`/`EditorAbort` |
| `print.rs` round-trip fixture binds `[global] quit = "ctrl-x"`, which E9 (c) refuses | TRUE | `print.rs:153`; T1 moves it |
| `views::QUIT_CONFIRM` can mirror `views::MIGRATION` | **AMENDED** | `MIGRATION` (`views.rs:323`) leads with `Layer::view(Context::Migration, &[])`, a catalogue view context. T0 uses `[CONFIRM, OVERLAY, HELP]` with no view layer, so no new `Context` or catalogue rows; the blueprint confirms a stack may start with a shared layer (help heading, `DECLARED` walk), else adds `Context::QuitConfirm` |
| The quit mapping is one point: `action_for(Act::Quit)` and the lock's `Quit` arm | TRUE | `state.rs:379`, `pane.rs:357`; `Action::Quit` elsewhere only `update.rs:25` (apply) and `ctrl-c` (`state.rs:792`, `pane.rs:338`) |
| `set_scope` resets every tab, and `Reveal` focuses and drives a tab (E5) | TRUE | `update.rs:215-227` (`on_scope_change` on every tab), `update.rs:275-300`; `reveal_tabs` maps Item → Backlog (`app/mod.rs`), which hosts the item form, Documents and Notes |
| `?` box adds the editor line only for unconverted views (E7) | TRUE | `state.rs` `render_help`: `Context::Editor` line only in the `active_stack() == None` arm |
| `wants_mouse` is off whenever an editor exists (E8) | TRUE | `state.rs:673-678` (`self.editor.is_none() && ...`) |
| The status line shows `editor_status` instead of the tab's line on every tab (E6) | TRUE | `state.rs` `render`: `self.editor_status().unwrap_or_else(.. status_line ..)` |
| M1 tests pinning the lock's quit-at-once / refusal exist and must be rewritten | TRUE | `pane.rs` `unfocused_ctrl_c_and_q_quit_at_once`, `unfocused_every_other_key_is_refused_and_nothing_moves`; `tests/editor_pane.rs:410` `quitting_with_a_live_editor_leaves_no_child` presses `q` and asserts `should_quit` (T3 adds `y`) |
| README needs no change | TRUE | `README.md:614` lists `docs/htui-editor.md` only; the M1 lock lives in `docs/htui-editor.md` (§ "While the editor is open (the M1 lock)", key table rows `:122-140`) |
| `app/snapshots` and `ui/overlay/snapshots` exist | TRUE | directory listing |
| T1 ∩ T2 = ∅ after T0; T3 after both | TRUE | Independence table; the only shared files (`stack.rs`, `views.rs`, `ui/overlay/mod.rs`) are T0's |
