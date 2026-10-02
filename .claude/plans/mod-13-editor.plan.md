# Plan: MOD-13 milestone 4 — `$EDITOR` round-trip

**Source PRD**: `.claude/prds/mod-13-backlog-editing.prd.md`
**Selected Milestone**: 4 — `$EDITOR` round-trip
**Complexity**: Small
**Status**: in-progress — CONFIRMed 2026-10-02 (routed as plan path, sandbox `hr/MOD-13`)

## Summary
In the Backlog item form (milestone 2), Ctrl+E on the body or the touched-paths field hands that
field's text to `$VISUAL`/`$EDITOR`. The edited text comes back into the same field, and the user
saves with Ctrl+S as before. That save is the same §7.2 compare-and-set at the form's token, so a
stale save still opens milestone 3's three-way view, with "mine" holding the editor's text.

**The editor half is already built (MOD-9).**
- `crate::editor` resolves the command, writes a temp file, and runs the editor with the TUI
  suspended (`run_suspended`, `editor.rs:252`). It reads the file back, normalises line endings,
  and holds off Ctrl-C/Ctrl-\.
- A tab emits `Action::EditExternally(ExternalEdit)`. `App::drain` stamps it with the asking tab
  (`app/state.rs:355-361`).
- The event loop drops the crossterm stream, runs the editor, and makes a fresh stream
  (`event_loop.rs:45-62`). Store replies queue in the unbounded channel meanwhile, and
  `ticker.reset()` stops a burst of ticks. Then `App::finish_external_edit` hands the
  `ExternalEditOutcome` to that tab's `Tab::on_external_edit` (`state.rs:379`; default no-op at
  `ui/tabs/registry.rs:77`).
- So the PRD's risk ("a store reply or tick lands mid-suspend") is already handled by the loop.
  This milestone pins it from the Backlog side and adds no event-loop code.

The milestone adds:
- the Ctrl+E handoff and outcome handling in `ItemForm`;
- one `ItemFormOutcome` variant;
- the `BacklogTab::on_external_edit` routing;
- three shared notice constants hoisted into `editor.rs`;
- unit and integration tests.

It adds no store request, no reply variant, no conformance case and no migration.

## Design decisions
- **D1: Ctrl+E edits the focused multi-line field, Body or Paths, and nothing else.**
  - On Title/Priority/Tags or a picker, Ctrl+E is swallowed (`Stay`). It is not passed: it must
    not leak to a future global binding (MOD-67) from inside a capturing form, and a one-line
    field gains nothing from an external editor.
  - Caught in `ItemForm::on_key` after the divergence-view check and Ctrl+S, but **before** the
    chord pass (`item_form.rs:489-491`). Today every chord but Ctrl+S passes, so Ctrl+E would
    otherwise reach the tab. Same rule as Ctrl+S (A6) and as the Templates editor
    (`skills/templates.rs:695-699`): `modifiers - SHIFT == CONTROL`, `e` or `E`.
  - **Rejected: Ctrl+E edits the body from any field.** It is a hidden target, and the
    paths field would then need its own chord.
- **D2: refused while busy.** A `MintItem`/`EditItem`/catalogue reload in flight swallows Ctrl+E,
  as it swallows Ctrl+S. The Templates precedent (`hand_off`, `templates.rs:781-797`) refuses with
  a notice; here the form's busy state already says what it waits on, so it is a silent `Stay`,
  like every other key while busy (D8). While the divergence view is open, the view takes every
  key (milestone 3 D5) and Ctrl+E passes as a chord, as any other chord does there today.
- **D3: the handoff, `ItemFormOutcome::External(ExternalEdit)`.**
  - The form stays pure (no `Ctx`). It records `external: Option<Field>`, the field handed out,
    and answers the new variant. `BacklogTab::on_item_form_key` emits it as
    `Action::EditExternally`.
  - Temp-file stem: `<key>-body`/`<key>-paths` on an edit, `new-body`/`new-paths` on a new form.
    `editor::run` sanitises it (D25).
- **D4: the outcome, `ItemForm::on_external_edit(outcome)`, mirroring
  `TemplatesView::on_external_edit` (`templates.rs:434-470`).**
  - No pending field (`external` is `None`): ignored.
  - `Edited(text)` → after D5, the field's widget becomes `TextArea::with_text(text)`, focus moves
    to that field, and the notice is `EDITED`. The token, reason, `opened` texts and every other
    field are untouched. "Unchanged" (A4) still compares against what the form *opened* with, so
    an external edit that restores the original text is `NOTHING_TO_SAVE` at Ctrl+S.
  - `Unchanged { quick }` → `NO_CHANGES` plus `WAIT_FLAG` when quick, and the text is untouched.
  - `Failed(message)` → the message, and the text is untouched.
  - `BacklogTab::on_external_edit` routes the outcome to the open item form. With no form (it
    cannot close while the loop is suspended, but a scope change could in principle land first),
    the outcome is dropped with a `tracing::debug!`, as `finish_external_edit` drops one for a
    gone tab.
- **D5: an editor-added final newline is dropped.**
  - Most editors (vim's `fixeol`, nano, VS Code's default) add a final `\n` on save. The form
    keeps the body verbatim (`ItemForm::parsed`, `item_form.rs:731-735`), and
    `normalise_newlines` does not touch trailing newlines (`htui-core/src/prompt/render.rs:134`).
    So `:wq` on an unchanged body without a final newline would come back as `Edited("…\n")` and
    Ctrl+S would send a body change.
  - Rule, in the form only: when the text handed out did not end in `\n`, strip exactly one
    trailing `\n` from the returned text. If the result equals the handed-out text, treat it as
    `Unchanged { quick: false }`.
  - `editor.rs` and the Skills views are unchanged: a template body's final newline is that
    domain's own question.
- **D6: online-only holds by construction.**
  - Ctrl+E exists only inside the item form, and the worker refuses `ItemForm` with
    `DATABASE_UNREACHABLE` before any read (milestone 2), so an `Offline` box never opens a form.
  - The editor writes nothing to the store. The text only reaches the store through Ctrl+S's
    `EditItem`/`MintItem`, which the worker refuses offline as today.
  - Pinned by a test: on an offline backend, `e` then Ctrl+E leaves `take_external_edit()`
    `None`.
- **D7: hints.** `HINT_TEXT` stays on the one-line fields. Body and Paths get a new
  `HINT_AREA = "Ctrl+E $EDITOR  Ctrl+S save  Esc cancel"`: 39 columns against the detail pane's
  43, so `Tab field` drops there (Tab still cycles). The existing `item_form_new`/`item_form_edit`
  snapshots focus Title, so they are unchanged.
- **D8: the three notice strings move to `editor.rs`.** `EDITED`, `NO_CHANGES` and `WAIT_FLAG`
  become `pub const` in `crate::editor`. The two identical private copies in
  `skills/templates.rs:75-81` and `skills/library.rs:82-88` are replaced by uses, rather than
  adding a third copy. The strings are unchanged, so the Skills tests stay green.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Handoff | `crates/htui/src/ui/tabs/skills/templates.rs:781-797` | `hand_off`: refuse while busy, emit `Action::EditExternally`, keep a pending marker |
| Outcome | `crates/htui/src/ui/tabs/skills/templates.rs:434-470` | `on_external_edit`: take the pending marker, `Edited` → `TextArea::with_text` + `EDITED`; `Unchanged` → `NO_CHANGES` + `WAIT_FLAG`; `Failed` → message |
| Chord rule | `crates/htui/src/ui/tabs/skills/templates.rs:695-699`, `item_form.rs:253-256` | `modifiers - SHIFT == CONTROL`, matched before the chord pass |
| Pure form → tab | `crates/htui/src/ui/tabs/backlog/mod.rs:368-383` | `ItemFormOutcome` variant → `ctx` call in `on_item_form_key` |
| Errors | `crates/htui/src/ui/tabs/backlog/item_form.rs:657-670` | refusals and outcomes are the form's notice line; nothing on the status line |
| Unit tests | `crates/htui/src/ui/tabs/backlog/item_form.rs:1027-1100` | `key`/`ctrl`/`focus_on` helpers over a `MemStore` context; assert `ItemFormOutcome` and `edit_sent` |
| Integration | `crates/htui/tests/templates.rs:764-800`, `crates/htui/tests/backlog.rs:1757-1790`, `:1997-2005` | `take_external_edit()` + `finish_external_edit(tab, outcome)`; `keys`/`type_text`/`ctrl-s` over `backlog_over(MemStore)` |
| Fake editor | `crates/htui/src/editor.rs` `mod scripts` | `#!/bin/sh` script in a `TempDir`, `EditorCommand` via `EditorCommand::resolve` over a fixed lookup; unix only |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui/src/editor.rs` | UPDATE | D8: `pub const EDITED`, `NO_CHANGES`, `WAIT_FLAG` |
| `crates/htui/src/ui/tabs/skills/templates.rs` | UPDATE | D8: use `crate::editor`'s constants, drop the private copies |
| `crates/htui/src/ui/tabs/skills/library.rs` | UPDATE | D8: likewise |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | UPDATE | D1–D5, D7: Ctrl+E, `external`, `ItemFormOutcome::External`, `on_external_edit`, newline rule, `HINT_AREA`, render, unit tests; `Debug` gains `external` (a field, no text) |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | D3/D4: emit `Action::EditExternally`; `Tab::on_external_edit` for `BacklogTab`; unit test |
| `crates/htui/tests/backlog.rs` | UPDATE | Integration: round-trip save, fake-editor run, divergence after an external edit, reply mid-suspend, offline unreachable |
| `.claude/prds/mod-13-backlog-editing.prd.md`, `HANDOFF.md`, this plan | UPDATE | Close-out bookkeeping (phase note, milestone row) |

## Tasks
All tasks run **serially**: T1–T3 share `item_form.rs`/`mod.rs`/`tests/backlog.rs`, so no task
pair has disjoint file sets. One implementer, TDD, committing per task.

### Task 1: shared constants (D8)
- **Action**: Hoist `EDITED`, `NO_CHANGES` and `WAIT_FLAG` into `crate::editor` as `pub const`,
  with their doc comments. Replace the private copies in `templates.rs` and `library.rs` with
  uses.
- **Mirror**: `editor::QUICK_EXIT`, a `pub const` with a doc comment naming the D-number.
- **Validate**: `cargo test -p htui --all-features --lib skills` and
  `cargo test -p htui --all-features --test templates --test skills`.

### Task 2: the form (D1–D5, D7), TDD in `item_form.rs`
- **Tests first** (unit, `item_form.rs` `mod tests`):
  - Ctrl+E on Body answers `External` with the body text and stem `<key>-body`. On Paths it
    answers the paths text and `<key>-paths`. A new form uses `new-body`.
  - Ctrl+E on Title/Priority/Tags/Kind is `Stay`, with no `External` and no `Pass`.
  - Ctrl+E while busy is `Stay`.
  - `Edited` replaces only the handed-out field and focuses it. The token and reason are
    unchanged, and Ctrl+S sends an `EditItem` at the opened version with only `body` in the
    changes (`edit_sent`).
  - `Edited` back to the opened text gives `NOTHING_TO_SAVE` at Ctrl+S.
  - `Unchanged { quick: true }` shows `NO_CHANGES` + `WAIT_FLAG` and keeps the text.
    `Failed(m)` shows `m` and keeps the text.
  - D5: handing out `"abc"` and getting back `Edited("abc\n")` reads as unchanged.
    `Edited("abd\n")` becomes `"abd"`. Handing out `"abc\n"` and getting back `Edited("abc\n\n")`
    keeps both newlines.
  - An outcome with no pending field is ignored.
  - `the_hints_fit_the_detail_pane` covers `HINT_AREA`. A render with Body focused shows it.
  - `Debug` still prints no body or paths.
- **Action**: Implement D1–D5 and D7.
- **Mirror**: `TemplatesView::hand_off`/`on_external_edit`. Keep Ctrl+E's check next to
  `ctrl_s` (an `ctrl_e` helper beside it).
- **Validate**: `cargo test -p htui --all-features --lib backlog::item_form`.

### Task 3: tab wiring and integration (D3, D4, D6), TDD
- **Tests first**:
  - Unit, `backlog/mod.rs`: Ctrl+E in the open form emits `Action::EditExternally`.
    `on_external_edit` with no form open is a no-op.
  - Integration, `tests/backlog.rs`:
    1. `e`, Tab to Body, `ctrl-e` → `take_external_edit()` is `(BacklogTab::ID, ExternalEdit {
       text: ANA-1's body, stem: "ANA-1-body" })`. Then `finish_external_edit(Edited(..))` and
       `ctrl-s` → the store head has the new body at version 2, and the status line stays clean.
    2. **Mid-suspend reply** (the PRD risk): after `take_external_edit()`, a concurrent writer
       bumps ANA-1 in the `MemStore` and a list re-read reply is driven (`harness.settle()`)
       *before* `finish_external_edit`. The edited text still lands in the form, and `ctrl-s`
       opens the divergence view (milestone 3) with "mine" holding the editor's body.
    3. **Fake editor** (`#[cfg(unix)]`): a `#!/bin/sh` script that appends a line, run through
       the public `htui::editor::run(&cmd, &edit.text, &edit.stem)` with the `ExternalEdit` the
       app asked for. Its real outcome is fed to `finish_external_edit`, then `ctrl-s` → the
       appended line is in the store. The test drives no terminal: `run_suspended`'s
       leave/enter is already pinned by `editor.rs`'s `suspension` tests.
    4. **Offline** (D6): the offline harness of
       `offline_n_and_e_are_refused_with_the_read_only_notice`, then `e`, `ctrl-e` →
       `take_external_edit()` is `None`.
- **Action**: Add the `External` arm in `on_item_form_key` (`ctx.emit(Action::EditExternally(
  edit))`), and `fn on_external_edit` in `impl Tab for BacklogTab`, routing to
  `item_form.on_external_edit`.
- **Validate**: `cargo test -p htui --all-features --test backlog`.

### Task 4: close-out
- PRD row 4 → `complete`, this plan's status line, the HANDOFF MOD-13 phase-4 note
  (`references/lifecycle.md` P1), and the validator.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
(`--all-features` is required: without `testkit`, `tests/*.rs` run 0 tests and still report ok.)

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A reply or tick lands mid-suspend (PRD) | L — already handled by `event_loop.rs:45-62` | Integration test 2 drives a reply between handoff and outcome; `editor.rs` already pins leave/enter and a dropped future |
| Editor-added final newline turns a no-op into a body edit | H without D5 | D5 rule and its three unit cases |
| Ctrl+E leaks past the form as a chord | M | Caught before the chord pass (D1), with a unit test per field kind |
| An external edit bypasses the compare-and-set or moves the token | L | The outcome touches only the widget text; tests assert token/version and the divergence path |
| Hoisting the constants changes Skills behavior | L | Strings byte-identical; Skills/Templates suites in T1's Validate |
| The fake-editor test is flaky under parallel `cargo test` (`ETXTBSY`) | L | Write the script with `std::fs::write`, as `editor.rs`'s `script` helper does (R-13) |

## Verified claims
Fact-checked against the tree on 2026-10-02 (handoff-run step 3.5).

| Claim | Verdict | Evidence |
|---|---|---|
| The editor handoff (`EditExternally`, `run_suspended`, `finish_external_edit`) exists and is tab-agnostic | ✓ | `app/action.rs:59`, `app/state.rs:355-361`, `:379-409`, `editor.rs:252`, `event_loop.rs:56` |
| Replies queue and the ticker resets during the editor | ✓ | `event_loop.rs` `run`: stream dropped, `run_suspended().await`, fresh stream, `ticker.reset()`, then `finish_external_edit`; replies are an `UnboundedReceiver` |
| `Tab::on_external_edit` has a default no-op that `BacklogTab` does not override | ✓ | `ui/tabs/registry.rs:77`; `impl Tab for BacklogTab` (`backlog/mod.rs:517`) has no override |
| The item form passes every chord but Ctrl+S | ✓ | `item_form.rs:485-491` (`ctrl_s`, then `filter::CHORD` → `Pass`) |
| No global Ctrl+E binding exists | ✓ | `keymap.rs`: the only `CONTROL` default is `ctrl-c` (`:248`, `:344-345`) |
| The body is kept verbatim (no trailing-newline trim) | ✓ | `ItemForm::parsed`, `item_form.rs:731-735` (`|text| Ok(text.to_owned())`) |
| `normalise_newlines` leaves trailing newlines alone | ✓ | `htui-core/src/prompt/render.rs:134-149` (BOM strip + CR/CRLF → LF only) |
| `TextArea::with_text` exists and normalises CR | ✓ | `ui/text_area.rs:94-104` |
| `HINT_AREA` fits the 43-column detail pane | ✓ | 39 chars (`wc -m`); test `the_hints_fit_the_detail_pane` (`item_form.rs:1427`) |
| Existing item-form snapshots focus Title, so they are unchanged | ✓ | both `backlog__item_form_{new,edit}.snap` contain `Tab field  Ctrl+S save  Esc cancel` |
| An offline box never opens the item form | ✓ | `tests/backlog.rs:1850` `offline_n_and_e_are_refused_with_the_read_only_notice` |
| `htui::editor::run`, `take_external_edit`, `finish_external_edit` are callable from integration tests | ✓ | `pub async fn run` (`editor.rs:176`); `tests/templates.rs:768-790` uses the latter two |
| `EDITED`/`NO_CHANGES`/`WAIT_FLAG` are duplicated byte-identically | ✓ | `skills/templates.rs:75-81`, `skills/library.rs:82-88` |
| Tasks are independent | ✗ — serial | T1–T3 share `item_form.rs`/`mod.rs`/`tests/backlog.rs`; T1 touches `editor.rs`, which T2 imports |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented (MOD-9 handoff reused; no event-loop change)
- [ ] An external edit saves only through the §7.2 compare-and-set, and a stale one opens the
      divergence view
- [ ] No editor action reachable while `Offline`
