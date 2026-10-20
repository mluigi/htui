# Plan: MOD-115 M2 — layouts, navigation, history, leave dialog, every key configurable

**Source PRD**: `.claude/prds/mod-115-tui-shell.prd.md`
**Selected Milestone**: 2 — Layouts and navigation
**Complexity**: Large (new shell state, a UI-state file, focus and history in the dispatch path, the
`FIXED` key register removed, a requirement amended)

## Summary
The shell gets a per-tab **layout** (preset, list and activity split sizes, list/activity shown or
hidden, zoom). Tabs with a list and a detail render into shell-given pane areas instead of splitting
the body themselves. The shell also gets **pane focus** moved by `Tab`/`Shift-Tab`/`Ctrl ←→`, and a
browser-style **history** (50 entries, `⌫`/`Alt ←` back, `Alt →` forward, header chip). A
**leave dialog** replaces the editor-only quit question, and a **UI-state file** in the config root
saves layout and the about panel the moment they change. Per the maintainer decision of 2026-10-18,
**every key becomes a catalogue row**: the `FIXED` register, the text fields' `Enter`/`Esc` and the
`ctrl-c` special case all go through the keymap. Only characters typed into a text field stay text.
This amends `R-TUI-10`.

**Status**: COMPLETE 2026-10-20 on hr/MOD-115 (tasks T0-T8 plus review rounds RV1-RV4; Task 9's snapshot check folded into every task's gate). CONFIRMED by the maintainer 2026-10-18 as written. That means one milestone, not split.
The D4 defaults (`ctrl-pgdn`/`ctrl-pgup` for tabs) and the D6 wording are accepted. Tasks 0 and 5
(every key configurable) land before the layout wiring.

## Maintainer decisions taken into this plan (2026-10-18)
| # | Decision | Source |
|---|---|---|
| MD1 | Every key is configurable through the keymap; no hardcoded part, no key the validator reserves for a view. | maintainer, this run |
| MD2 | `ctrl-c` becomes an ordinary chord, rebindable, with a safety net: a key set that leaves no way to leave htui is refused. | maintainer, this run |
| MD3 | The clash rule stays: a global chord cannot also be bound in a narrower context by the user. | maintainer, this run |
| MD4 | Users' existing overrides are kept when defaults change (PRD Q2). | maintainer, this run |
| MD5 | PRD Q1 (where layout/history live) is decided here: see D1. | maintainer, this run |
| MD6 | The plan drafts the leave-dialog wording; it must not promise that work survives the exit (PRD Q3): see D6. | maintainer, this run |
| MD7 | **`Esc` is the one exception to MD1.** It can never be configured: it cannot appear in `keys.toml` (refused like the old `ctrl-c` rule, naming the reason). Every action whose default holds `esc` keeps it whatever the file says: an override replaces only that row's other chords, and `[]` unbinds only those. So `Esc` always does what it does today (close, cancel, back, dismiss, no). This also settles blueprint D-13 for `Esc`. **Implemented in Task 5** (loader, validator, `[field]` translation never drops `Esc`); Task 0's rows are unaffected. | maintainer, 2026-10-18 after the blueprint |
| MD8 | Mouse capture turns on for the list + detail tabs (Backlog, Requirements, Skills), so a click focuses a pane and a border drag resizes. Chat and Settings keep the terminal's plain text selection. This settles blueprint D-6. **Task 6.** | maintainer, 2026-10-18 after the blueprint |

## Design decisions (proposed; CONFIRM accepts or amends them)
- **D1 — UI state lives in a per-machine file, history in memory.**
  - The file is `<config root>/ui-state.toml` (`htui_store::identity::config_root_path`, the
    directory that already holds `box.toml` and `keys.toml`). It is versioned and written
    atomically (temp file + rename) on every change.
  - It holds the about panel's state and each tab's layout. A section is reserved for modal sizes,
    which `R-TUI-12` asks for, but there is no resizable modal yet.
  - A missing file means first launch: about on, defaults. An unreadable or invalid file falls back
    to defaults with a header flash; it never stops htui starting.
  - The testkit uses an in-memory store and never touches the home directory.
  - **Why not Postgres:** layout depends on the terminal, so it is per machine, not per user. It is
    written on every resize key, so a store round trip per keystroke is wasted. It must work offline
    and before the store connects. `R-TUI-12` says only "saved the moment they change". MOD-109
    moves the *keys* to Postgres (per user, shared), and nothing here gets in its way.
  - History stays in memory for the session: `R-TUI-13` does not ask for it to survive a restart.
- **D2 — Layout model.**
  - `Preset { Ide, IdeActivity, Columns, Focus }`, and per tab
    `TabLayout { preset, list_pct, activity_pct, list_on, activity_on, zoom }`.
  - The shell turns `(body, TabLayout, tab's pane kinds)` into
    `PaneAreas { list: Option<Rect>, detail: Rect, activity: Option<Rect> }`.
  - A tab declares `Panes::Single` (default) or `Panes::ListDetail`. Chat and Settings are single
    in M2. Backlog, Requirements and the two Skills views are list + detail.
  - **Defaults reproduce today's splits:** Backlog 55/45, Requirements 45/55, Skills a 32-column
    list. So M2's snapshot churn is focus borders, not reflow. The findings' 24% default arrives
    with each tab's port in M3/M4.
  - **The activity slot** shows a kit empty state ("Activity appears here once the activity panel
    lands") until MOD-122 fills it. No tab defaults to a preset with activity in M2.
  - When the layout hides something, the pane says how to bring it back, e.g.
    `F2 show list · p layouts`, `z unzoom` (`R-TUI-12`).
- **D3 — Pane focus is shell state.**
  - The shell keeps one focused pane per tab and hands it to the tab in `Ctx`.
  - `Tab`/`Shift-Tab`/`Ctrl ←→` cycle the visible panes; a click on a pane focuses it.
  - Only the focused pane gets an accent border and bottom-border hints (kit `Focus`, from M1).
  - Plain arrows act inside the focused pane only: list cursor in the list; scroll or row cursor in
    the detail.
  - On a single-pane tab, the pane keys do nothing.
- **D4 — New key rows and default changes.** Each new row goes into the catalogue, so all of them
  can be rebound.

  | Row | Default | Note |
  |---|---|---|
  | `global.next_pane` / `prev_pane` | `tab`, `ctrl-right` / `backtab`, `ctrl-left` | `Tab` changes meaning (`R-TUI-10`) |
  | `global.next_tab` / `prev_tab` | `ctrl-pgdn` / `ctrl-pgup` (were `tab`/`backtab`) | digits and clicks remain the main way |
  | `pane.next_subtab` / `prev_subtab` | `shift-right`, `l`, `]` / `shift-left`, `h`, `[` | plain `right`/`left` dropped: arrows stay in the pane |
  | `settings.next_section`, `skills.switch_view` | same shift-arrow change | same reason |
  | `layout.next_preset` / `prev_preset` | `p` / `P` | a view that claims the letter wins there (D4a) |
  | `layout.grow_list` / `shrink_list` | `>` / `<` | no AltGr on IT/DE/FR layouts (`<>` key) |
  | `layout.grow_activity` / `shrink_activity` | `}`, `ctrl-down` / `{`, `ctrl-up` | `{}` need AltGr on IT, so the ctrl-arrows are the AltGr-free default |
  | `layout.toggle_list` / `toggle_activity` | `f2`, `alt-1` / `f3`, `alt-2` | |
  | `layout.zoom` | `z` | |
  | `history.back` / `forward` | `backspace`, `alt-left` / `alt-right` | a capturing field keeps `backspace`; `picker.parent` keeps it in the picker |
  | `global.quit` | `q`, `ctrl-c` | both open the leave dialog (D6) |
  | `leave.confirm` / `leave.stay` | `enter`, `ctrl-c` / `esc`, `n` | the second `ctrl-c` leaves (`R-TUI-18`) |

  - **No `ctrl-h`/`ctrl-l` for panes by default.** `R-TUI-10` names `Ctrl ←→`, and `h`/`l` are
    already the sub-tab letters. A user can still bind them, since `ctrl-h` arrives as itself
    (`keys/chord.rs:318`).
  - **D4a:** single letters a view already uses keep that meaning in that view, through reviewed
    `SHADOWING` pairs. `p` is promote in Runs, primary in Hierarchy, probe in Boxes, paste in
    Agents, previous template in Prompt. `P` is pause queue in Backlog and the queue. In the
    Backlog tab, presets therefore cycle with `p` only.
- **D5 — History.**
  - An entry is `Loc { tab, subtab, pane, cursor: TabLoc }`, where `TabLoc` is an opaque value the
    tab gives and takes back (`Tab::location` / `Tab::restore`).
  - After each dispatched key, click or `Reveal`, the shell compares the coarse location
    `(tab, subtab, pane)` with the one before. If it changed, the old full `Loc` (cursor included)
    is pushed and the forward stack is dropped.
  - The cap is 50; the oldest entry drops first.
  - Moving a cursor is not recorded. Going back records the place left, so forward returns to the
    cursor.
  - The header shows `⌫ back to <label>`, naming the pane when it is not the detail, and
    `Alt→ forward` when there is somewhere forward. Both labels come from `Keys`.
- **D6 — Leave dialog.** It replaces `quit_confirm.rs`, and `global.quit` always opens it
  (`R-TUI-18`). Draft wording, written so it promises nothing about work surviving:
  ```
  ╭ Leave htui? ────────────────────────────────────────────╮
  │ Leaving closes htui. Work it is running stops with it:   │
  │   ◐ 3 working · ⚠ 1 waiting   Graphics                   │
  │   ✎ an open edit (unsaved changes are lost)              │
  │                                                          │
  │  Enter  leave    Esc  stay                               │
  ╰──────────────────────────────────────────────────────────╯
  ```
  - With nothing live, the dialog reads `Nothing is running. Leave htui?`, with the same chips.
  - The live rows come from the header's counts for the open workspace and the editor slot. Other
    workspaces are listed once MOD-116 tracks them.
  - **No "stop all" yet.** Until MOD-116/MOD-125 land, "leave" already stops the in-process work
    (`RunRuntime::shutdown` aborts after a grace window), so a stop-all choice would do the same
    thing. It arrives with graceful stop (`R-ORCH-16`) and keep-running.
- **D7 — Every key configurable (MD1, MD2).**
  - **`FIXED` is removed.** Its keys become rows in contexts that already exist or new ones:
    - `[choice]`: `prev`, `next`, `toggle`
    - `[field]`: `submit`, `cancel`, and the editing keys `left`, `right`, `home`, `end`,
      `delete_back`, `delete_forward`
    - `[answer]`: `option_1`..`option_9`
    - `requirements.priority_must` / `priority_later`
  - Chat replay's swallow set is derived from the `CHAT_LIVE` rows instead of literals.
  - **Text fields are reached through a translation step at the shell** (amended by the fact-check;
    see Verified claims). `TextField`, `TextArea` and `Composer` keep matching their canonical
    editing events. While the active stack is capturing text, the new `keys/translate.rs`
    rewrites the incoming chord:
    - A chord bound to a `[field]` row becomes that row's canonical event. For example,
      `field.cancel = ["ctrl-g"]` turns `ctrl-g` into the canonical cancel.
    - A canonical key that is no longer bound to its row, such as a bare `Esc` after that rebind,
      is not delivered as that event.
    
    The ~25 files that use the widgets need no change. The widgets' literal matches become internal
    event names, not bindings; the guard's allow-list names those three files for that reason.
  - A printable character typed into a capturing field is text, not a binding. That is the only
    exception.
  - **`ctrl-c`** becomes a default chord of `global.quit` and `leave.confirm`. Like F-keys, it still
    passes modal and capture layers, so it acts from every screen and field. A focused in-pane
    editor still receives it (`R-TUI-10`'s terminal-pane clause), and `editor.focus` remains the way
    out.
  - **Safety net:** the validator refuses a set in which `global.quit` or `leave.confirm` has no
    chord. The message names both rows and the fix.
  - **Out of scope:** `provision/secret_prompt.rs`. It is a raw-tty password prompt that runs
    before the keymap loads and outside the TUI, with line-editing keys like a shell's `read`.
    Named in Risks.
  - **A guard test makes the rule hold.** `tests/no_hardcoded_keys.rs` scans `crates/htui/src` for
    `KeyCode::` outside `#[cfg(test)]` modules and an allow-list. It fails on any new site. The
    allow-list:
    - `keys/chord.rs` and `keys/translate.rs`: parsing, labels, canonical events
    - `editor/pane.rs` `encode_key`: PTY bytes
    - `ui/text_field.rs`, `ui/text_area.rs`, `chat/composer.rs`: canonical editing events, which
      are reached only through the translation step
    - `provision/secret_prompt.rs`
- **D8 — Existing key files keep loading (MD4).**
  - `keys.toml` moves to `version = 2`. Rows added or changed in M2 carry `since: 2`.
  - In a `version = 1` file, a `since: 2` default that collides with one of the file's entries
    gives way, wherever a declared stack meets them. The entry keeps its chord, and htui prints a
    notice naming the action that lost it.
  - A `version = 2` file gets the strict rules (MD3).
  - An upgrade therefore never turns a working file into an exit-2 refusal. MOD-109 later moves
    the overrides to Postgres, and the same `since` tag can drive that migration.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| F9 is already a rebindable catalogue row; the M1 "clash" is the general global-vs-context rule | true | `keys/catalogue.rs:859` `row(Act::About, Global, "about", &["f9"], …)`; `docs/htui-keys.md:7-9` |
| `ctrl-c` is hardcoded: refused by the loader, handled before every overlay | true | probe `[global] workspaces = ["ctrl-c"]`: exit 2; `app/state.rs:765-770` `if chord == CTRL_C { self.update(Action::Quit) }` |
| `FIXED` is a register of view-kept keys the validator refuses to bind | true | `keys/stack.rs:357-426` (8 entries: attach, requirements form, runs digits, close-out, filter, item form, chat digits, chat replay) |
| `R-TUI-10` today *requires* fixed keys and an unbindable `ctrl-c`, so MD1/MD2 amend it | true | `docs/REQUIREMENTS.md` R-TUI-10: "a field's Enter and Esc, numbered choices, and a choice field's value keys are fixed … `ctrl-c` … cannot be unbound" |
| `R-TUI-18`'s "leaving never stops work" is out of M2's reach; exit stops in-process runs | true | `htui-worker/src/runtime.rs:1620-1645` `shutdown` cancels walks and aborts after `2 × grace`; PRD scope defers keep-running to MOD-116/125 |
| No UI-state persistence exists; about panel is memory-only | true | `app/state.rs:190-191` ("In memory only: it starts on and M2 persists it") |
| Config root is `dirs::config_dir()/htui`, already holds `keys.toml`/`box.toml` | true | `htui-store/src/identity.rs:51-55` `config_root_path` |
| Today's splits: Backlog 55/45, Requirements 45/55, Skills 32-col list | true | `backlog/mod.rs:65,68`; `requirements/mod.rs:60,63`; `skills/library.rs:62`, `templates.rs:49` |
| Every proposed default chord parses (`shift-←→`, `ctrl-←→↑↓`, `ctrl-pgdn/pgup`, `alt-←→`, `alt-1/2`, `f2`, `f3`, `backspace`, `< > { } z`) | true | loader probe through `htui --keys … --print-keys`: exit 0 for each |
| A global `p` / `P` collides with view verbs | true | probe: `p` gives 5 errors (agents paste, hierarchy primary, boxes probe, runs promote, prompt prev template); `P` gives 1 (backlog pause queue). D4a's `SHADOWING` pairs are required |
| A global `z`, `<`, `>`, `backspace`, `tab`, `ctrl-pgdn` collide with nothing | true | same probe: exit 0 |
| `ctrl-h` would arrive as Backspace on a legacy terminal | **false**, amended | `keys/chord.rs:318` "`ctrl-h` and `ctrl-j` arrive as themselves in raw mode". The draft's reason was wrong. `ctrl-h/l` stay out of D4 anyway: `h`/`l` are sub-tab keys and the findings' `Ctrl ←→` is the requirement's form |
| `version = 2` is new to the loader | true | `keys/load.rs:293-297` refuses anything but `version = 1` |
| `TextField::on_key(key)` takes no `Keys`, and its outcomes are matched in ~25 files | true, **amends D7** | `ui/text_field.rs:145`; `FieldOutcome::Submit/Cancel` matched in 92 places across settings (11 sections), skills, requirements, backlog, overlays, `text_area.rs`. Threading `&Keys` into every call is a 25-file signature change, so D7 is amended to translate at the shell (below) |
| Tasks 1–3 each need a `mod` line in `ui/mod.rs` | true, **amends tasks** | `ui/mod.rs` is the only root; the draft gave the line to Task 1 while Tasks 2–3 created `ui/shell/`. Task 0 now owns every new `mod` line and empty file |
| File sets of parallel Tasks 1–5 after the amendment are disjoint | true | intersection below |

**File-set intersection (parallel group after Task 0):**
- T1 `{ui/state_file.rs}`
- T2 `{ui/shell/layout.rs, ui/kit/pane.rs, ui/kit/empty.rs}`
- T3 `{ui/shell/history.rs}`
- T4 `{ui/overlay/leave.rs}`
- T5 `{keys/stack.rs, keys/validate.rs, keys/load.rs, keys/translate.rs, skills/attach.rs, requirements/forms.rs, backlog/detail/runs.rs, backlog/filter.rs, backlog/item_form.rs, chat/mod.rs, chat/composer.rs, tests/no_hardcoded_keys.rs}`

All pairwise intersections are empty. T0 (`keys/catalogue.rs, keys/stack.rs, keys/views.rs, keys/print.rs, ui/mod.rs, ui/shell/mod.rs, ui/overlay/mod.rs`) shares `keys/stack.rs` with T5, so T0 runs first. T6 and T7 share `app/*` and the tab files with no parallel task, so they run serial.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Key rows | `keys/catalogue.rs:781` `CATALOGUE`, `row(Act, Context, name, &[chords], label)` | one row per action; defaults strict-parsed in `Keys::defaults` |
| Stacks | `keys/stack.rs:217` `DECLARED`, `keys/views.rs` | every screen's layers declared; validate walks them |
| Validation | `keys/validate.rs:41` `give_way`, `:99` `validate` | user's binding wins in its context; errors on the user's line |
| Config files | `keys/load.rs:193` `resolve(keys_flag, default_keys, root)` | missing file means defaults; errors name the path and line |
| Overlays | `ui/overlay/quit_confirm.rs`, `ui/kit/modal.rs` | kit `Modal`, chips through `Keys`, factory registered in `app/mod.rs` `register_all` |
| Rendering | `ui/kit/pane.rs` `Pane { focus, hints, … }` | styles from `Theme` roles; focus-only hints |
| Tests | `ui/header.rs` tests, `tests/keys.rs`, `tests/keys_file.rs` | `TestBackend` read back as strings; sentence-named tests; monochrome proof per style |
| Snapshots | `tests/shell.rs` via `testkit` `Harness` | insta; full `cargo insta test` measures impact |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui/src/keys/catalogue.rs` | UPDATE | new `Act`s and rows (D4, D7), `since` tag, new contexts |
| `crates/htui/src/keys/stack.rs`, `keys/views.rs` | UPDATE | `FIXED` removed; layers for `[layout]`, `[history]`, `[leave]`, `[choice]`, `[field]`, `[answer]`; new `SHADOWING` pairs |
| `crates/htui/src/keys/validate.rs`, `keys/load.rs`, `keys/print.rs` | UPDATE | fixed pass gone, quit safety net, version-2 give-way, `ctrl-c` no longer refused |
| `crates/htui/src/keys/chord.rs` | UPDATE | `CTRL_C` stays as a value; the "always quits" use leaves `app/state.rs` |
| `crates/htui/src/ui/state_file.rs` | CREATE | UI-state file: load, save (atomic), in-memory store for tests (D1) |
| `crates/htui/src/ui/shell/mod.rs`, `shell/layout.rs` | CREATE | presets, splits, zoom, visibility to `PaneAreas` (D2) |
| `crates/htui/src/ui/shell/history.rs` | CREATE | history model (D5) |
| `crates/htui/src/ui/overlay/leave.rs` | CREATE | leave dialog (D6) |
| `crates/htui/src/ui/overlay/quit_confirm.rs` | DELETE | folded into the leave dialog |
| `crates/htui/src/ui/tabs/registry.rs` | UPDATE | `Tab::panes`, `render_panes`, `location`/`restore` with defaults |
| `crates/htui/src/app/state.rs`, `app/update.rs`, `app/pane.rs`, `app/mod.rs`, `app/action.rs` | UPDATE | layout and focus in render and dispatch; history; leave dialog registration; `ctrl-c` path |
| `crates/htui/src/ui/header.rs` | UPDATE | history chip, layout name (`R-TUI-1`) |
| `crates/htui/src/ui/kit/pane.rs`, `kit/empty.rs` | UPDATE | "hidden, how to bring it back" note; activity placeholder |
| `crates/htui/src/ui/tabs/backlog/mod.rs`, `requirements/mod.rs`, `skills/library.rs`, `skills/templates.rs` | UPDATE | render into `PaneAreas`; key routing by focus; `location`/`restore` |
| `crates/htui/src/ui/tabs/skills/attach.rs`, `requirements/forms.rs`, `backlog/detail/runs.rs`, `backlog/filter.rs`, `backlog/item_form.rs`, `chat/mod.rs` | UPDATE | the `FIXED` sites resolve through `Keys` (D7) |
| `crates/htui/src/keys/translate.rs` | CREATE | `[field]` chord to canonical editing event, while a stack captures (D7) |
| `crates/htui/src/ui/tabs/chat/composer.rs` | UPDATE (maybe) | only if the translation does not cover its Enter/Esc |
| `crates/htui/src/testkit.rs` | UPDATE | in-memory UI state; focus and layout helpers |
| `crates/htui/tests/no_hardcoded_keys.rs` | CREATE | guard (D7) |
| `crates/htui/tests/*.rs` + snapshots | UPDATE | `Tab` meaning, focus borders, header chips, help box rows |
| `docs/REQUIREMENTS.md` | UPDATE | `R-TUI-10` amended in place (MD1, MD2) with the decision line |
| `docs/decisions/mod/mod-115.md` | CREATE | MD1–MD6 and D1–D8 recorded |
| `docs/htui-keys.md`, `README.md` | UPDATE | regenerated key page; the F9/ctrl-c notes rewritten |

## Tasks
Independence is decided at the fact-check by intersecting file sets, not by this prose.

### Task 0: Key catalogue surface (serial, first)
- **Action**: tests first. Add every new `Act`, context and row (D4, D7) with `since`, and the
  `SHADOWING` pairs (D4a). Bump the file format to version 2 and regenerate `print.rs` output.
  Behaviour stays inert: no site resolves the new rows yet. Also declare every new module as an
  empty file (`ui/state_file.rs`, `ui/shell/{mod,layout,history}.rs`, `ui/overlay/leave.rs`,
  `keys/translate.rs`) so parallel tasks never touch a shared `mod` list.
- **Files**: `keys/catalogue.rs`, `keys/stack.rs` (layers only), `keys/views.rs`, `keys/print.rs`,
  `keys/mod.rs`, `ui/mod.rs`, `ui/overlay/mod.rs`, the empty new files
- **Validate**: `cargo test -p htui keys:: -- --test-threads=1`, `the_compiled_defaults_validate`

### Task 1: UI-state file (parallel after 0)
- **Action**: tests first: round trip, atomic write, missing file means first launch, invalid file
  means defaults plus a reason, unknown keys ignored, in-memory store.
- **Files**: `ui/state_file.rs`
- **Validate**: `cargo test -p htui state_file -- --test-threads=1`

### Task 2: Layout engine (parallel after 0)
- **Action**: tests first: each preset's areas, clamped split sizes (min widths), hide/zoom,
  the hidden-pane note, monochrome focus. Pure geometry plus the kit note.
- **Files**: `ui/shell/layout.rs`, `ui/kit/pane.rs`, `ui/kit/empty.rs`
- **Validate**: `cargo test -p htui shell::layout kit:: -- --test-threads=1`

### Task 3: History model (parallel after 0)
- **Action**: tests first: push on coarse change, no push on cursor-only change, forward dropped on
  new place, cap 50, back/forward restore the full `Loc`.
- **Files**: `ui/shell/history.rs`
- **Validate**: `cargo test -p htui shell::history -- --test-threads=1`

### Task 4: Leave dialog overlay (parallel after 0)
- **Action**: tests first: live rows, nothing-live wording, `Enter`/`ctrl-c` leave, `Esc`/`n` stay,
  chips through `Keys`, monochrome.
- **Files**: `ui/overlay/leave.rs`
- **Validate**: `cargo test -p htui overlay::leave -- --test-threads=1`

### Task 5: Fixed keys through the keymap (parallel after 0)
- **Action**: tests first, per site: a rebound chord works and the old literal no longer does.
  Remove `FIXED` and its validator pass. Write the `[field]` translation (`keys/translate.rs`, a
  pure function of `(Keys, Stack, chord)`, which Task 6 calls). Add the quit, `field.cancel` and
  `overlay.close` safety nets and the version-2 give-way (D8). `ctrl-c` is no longer refused by the
  loader. Add the guard test.
- **Files**: `keys/stack.rs` (`FIXED` only), `keys/validate.rs`, `keys/load.rs`, `keys/translate.rs`,
  `chat/composer.rs` (if its Enter/Esc are not translated), `skills/attach.rs`,
  `requirements/forms.rs`, `backlog/detail/runs.rs`, `backlog/filter.rs`, `backlog/item_form.rs`,
  `chat/mod.rs`, `tests/no_hardcoded_keys.rs`
- **Validate**: `cargo test -p htui --all-features keys -- --test-threads=1`; the guard test;
  `cargo test -p htui --all-features --test keys_file`

### Task 6: Shell wiring (serial, after 1–5)
- **Action**:
  - `Tab` trait additions with defaults.
  - `App` holds the per-tab layout, focus, history and the UI-state store; `render` draws through
    `PaneAreas`.
  - `on_key` routes pane, layout and history rows; `ctrl-c` goes through `global.quit`, and the
    pre-overlay `CTRL_C` branch goes.
  - Leave dialog registered; `quit_confirm.rs` deleted.
  - Header history chip and layout name.
  - Writes the UI-state on every layout or about change.
  - Mouse: a click focuses a pane; dragging the list or activity border resizes it.
- **Files**: `ui/tabs/registry.rs`, `app/state.rs`, `app/update.rs`, `app/pane.rs`, `app/mod.rs`,
  `app/action.rs`, `ui/header.rs`, `ui/overlay/quit_confirm.rs`, `testkit.rs`
- **Validate**: `cargo build -p htui`; `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`

### Task 7: List + detail tabs adopt the layout (serial per tab, after 6)
- **Action**: Backlog, Requirements and Skills render into `PaneAreas`, route keys by focus, and
  implement `location`/`restore`. The defaults reproduce today's splits (D2).
- **Files**: `backlog/mod.rs`, `requirements/mod.rs`, `skills/library.rs`, `skills/templates.rs`,
  `skills/mod.rs`
- **Validate**: per tab, its tests plus `cargo insta test --all-features` reviewed for that tab
  only

### Task 8: Requirement, decisions and docs (parallel with 7)
- **Action**:
  - Amend `R-TUI-10` in place. Proposed text: "Every key is bound to a named action and can be
    rebound; only a printable character typed into a text field is text. `ctrl-c` is a default of
    leaving and may be rebound, but a set that leaves no way to leave htui is refused." Plus the
    decision line.
  - Write `docs/decisions/mod/mod-115.md`.
  - Regenerate `docs/htui-keys.md`; rewrite its F9 and `ctrl-c` notes and the README section.
- **Files**: `docs/REQUIREMENTS.md`, `docs/decisions/mod/mod-115.md`, `docs/htui-keys.md`, `README.md`
- **Validate**: `cargo test -p htui --test keys_file`; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`

### Task 9: Snapshots and gates (serial, last)
- **Action**: full `cargo insta test --all-features`. Review each tab's diffs. Expected changes are
  the focused-pane border, the header chips, the help box rows for the new keys and the
  leave-dialog frames. A body change is a defect. Then accept.
- **Validate**: the Validation block below

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
cargo test --workspace --all-features -- --test-threads=1 --no-fail-fast   # "zeta" literal guard, keys_file
cargo insta test --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Postgres-backed tests are not this milestone's gate; the host runs the real gate on the merged tree.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| `Tab` now moves panes, surprising users | Medium | `next_tab` keeps `ctrl-pgdn`/`ctrl-pgup`, digits and clicks; release note; overrides kept (D8) |
| Removing `FIXED` lets a user bind a form's `Esc` away and trap themselves | Medium | the validator requires `field.cancel` and `overlay.close` to keep a chord, like `quit` |
| New global letters (`p`, `P`, `z`, `<`, `>`) collide with view verbs | High | D4a `SHADOWING` pairs reviewed; `the_compiled_defaults_validate` lists every pair |
| `ctrl-pgdn`/`ctrl-pgup`, `alt-←→` or `shift-←→` not delivered by some terminal or by tmux | Medium | each has a second default (digits, `backspace`, `l`/`h`); the fact-check probes them through the loader |
| `ctrl-c` leaving the pre-overlay path changes behaviour under modal overlays | Medium | it passes modal and capture layers like F-keys; tests pin "acts from every screen" |
| `secret_prompt.rs` stays hardcoded | Low | it runs before the keymap and outside the TUI; on the guard's allow-list, named in the decision record |
| `R-TUI-18` ("leaving never stops work") stays unmet after M2 | Certain | by PRD scope (MOD-116/125); the dialog says so plainly; MOD-115's close-out records it as carried by MOD-116/125, not as done |
| Snapshot churn hides a regression | High | M2 defaults keep today's splits; per-tab review |
| Task 7 on `backlog/mod.rs` (4.5k lines) | Medium | render and dispatch seams only; the split is M4 |
| Stack or test-order flakiness | Medium | `--test-threads=1 --no-fail-fast`; re-run serially before calling a regression |

## Open points for CONFIRM
1. D1: the UI-state file in the config root (per machine) and history in memory.
2. D2: keep today's splits as M2 defaults (24% arrives with M3/M4); the activity slot shows a
   placeholder until MOD-122.
3. D4: `next_tab`/`prev_tab` move to `ctrl-pgdn`/`ctrl-pgup`; Backlog keeps `P` for pause queue.
4. D6: the leave-dialog wording above, with no "stop all" until MOD-116/MOD-125.
5. D7: the `R-TUI-10` amendment text; `secret_prompt.rs` out of scope.
6. D7: text fields reached through a shell translation step, not a 25-file signature change.
7. Size: this is one milestone. The alternative is to split D7/D8 (every key configurable) into
   its own milestone landed first. I recommend the split: D7/D8 change validation and every form's
   keys, and are independent of layouts.

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Layout, about state survive a restart (test: change, drop the process, reopen)
- [ ] No `KeyCode::` match outside the guard's allow-list
- [ ] Body regions unchanged in snapshots apart from focus borders
