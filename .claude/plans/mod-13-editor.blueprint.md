# Blueprint: MOD-13 milestone 4, the `$EDITOR` round-trip

This file makes the approved plan `.claude/plans/mod-13-editor.plan.md` (D1–D8, T1–T4) ready to compile. It does not
reopen any of the plan's decisions. Every anchor was checked with Gortex on `hr/MOD-13` at `498759f0`. The snapshots of
`item_form.rs`, `backlog/mod.rs` and `tests/backlog.rs` were checksum-matched against the working tree.
Where this file and the plan disagree on a detail, this file wins (§0). §6 lists what the maintainer should decide.

## 0. Errata (plan vs. tree)

| # | Plan claim | What the tree shows | Consequence |
|---|---|---|---|
| E1 | T2 changes `item_form.rs` only; T3 adds "the `External` arm in `on_item_form_key`" | `on_item_form_key` matches `ItemFormOutcome` exhaustively (`backlog/mod.rs:372-381`). It is the only exhaustive match: `item_form.rs:1634` `edit_sent` ends in `other =>`, and every other test uses `matches!`. Adding the variant in T2 alone breaks the workspace build between T2 and T3, and that commit's `cargo clippy --all-targets` gate fails (milestone 3's E3 again) | T2's commit also carries the one-line arm `ItemFormOutcome::External(edit) => ctx.emit(Action::EditExternally(edit)),` and the doc fix at `mod.rs:367` (§2d). T3 adds only `Tab::on_external_edit` and the tests |
| E2 | T3 test 2: "a list re-read reply is driven (`harness.settle()`) *before* `finish_external_edit`" | Nothing can put a list re-read in flight while an idle item form is open. `on_key` routes every key to the form (`mod.rs:578-580`). `on_refresh` asks only for `Runs`, every fifth refresh, and only when the item has an active run (`mod.rs:684-694`). Tick polls go out as `Origin::App` (`update.rs:126-137`), and so does `App::update(Action::Store)` (`update.rs:28`). A hand-built envelope fails `is_fresh` (`state.rs:329-333`). So `settle()` after `take_external_edit` serves nothing. The order is also backwards: the real loop calls `finish_external_edit` (`event_loop.rs:64`) before the next `select!` reads `replies` (`:45`), so queued replies land **after** the outcome | Test 2 (§3c) queues real requests with ticks before Ctrl+E, using `harness.key` (no drive). It delivers one batch before the outcome (the plan's order) and one after (the loop's order), with the concurrent write in between. The asserts are the plan's. See §6 Q1 |
| E3 | T3 test 3: "a `#!/bin/sh` script that appends a line" | `ANA_1_BODY` has no final newline (`htui-core/src/fixtures.rs:879-887`, it ends `brings in.`). A script running `printf 'more\n' >> "$1"` would join the text to the last line. `editor.rs`'s `scripts::script` is `pub(super)` inside `#[cfg(test)]` (`editor.rs:557`), and `EditorCommand`'s fields are private (`:30-35`) | The test writes its own script: `printf '\nAppended by the editor.\n' >> "$1"`. It builds the command with `EditorCommand::resolve`, quoting the path as `script` does (`editor.rs:562-565`). It asserts the raw outcome *and* the stored body without the final `\n`, which pins D5 end to end (§3c) |
| E4 | T3 Validate: `cargo test -p htui --all-features --test backlog` | That runs no `backlog/mod.rs` unit test | Also run `cargo test -p htui --all-features --lib ui::tabs::backlog` |
| E5 | Anchors: chord pass `item_form.rs:489-491` (D1), `:485-491` (claims); `parsed` `:731-735`; errors `:657-670`; `run_suspended` `editor.rs:252` | Ctrl+S `:485-490`, chord pass `:491-493`; the body arm of `parsed` `:729-734` (`fn parsed` `:717`); `save` `:657-669`; `pub async fn run_suspended` `:253` | None (drift of one or two lines) |

Every other anchor in the plan's Patterns and Verified-claims tables holds:
- `templates.rs:75-81`, `:434-470`, `:695-699`, `:781-797`; `library.rs:82-88`;
- `state.rs:355-361`, `:373`, `:379`; `registry.rs:77`; `mod.rs:368-383`, `:517`;
- `keymap.rs:248`, `:344-345`; `text_area.rs:94-104`; `render.rs:134`;
- `editor.rs:16`, `:176`; `event_loop.rs:54-64`;
- `tests/backlog.rs:1757`, `:1850`, `:1997`; `tests/templates.rs:764-800`;
- the 39/43-column hint widths, and both snapshots' `Tab field  Ctrl+S save  Esc cancel`.

## 1. Task 1, shared notices (D8)

### 1a. `crates/htui/src/editor.rs`
After `QUICK_EXIT` (`:14-16`):
```rust
/// The notice after an `$EDITOR` return with text in it: the view holds the text, unsaved (MOD-9;
/// shared by the Skills views and the Backlog item form, MOD-13 milestone 4 D8).
pub const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// The notice after an `$EDITOR` return that changed nothing (MOD-13 milestone 4 D8). The
/// Library also gives it for a rename that changes nothing.
pub const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within [`QUICK_EXIT`] (MOD-9 blueprint
/// D24, R-3).
pub const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";
```
The strings are byte-identical to the two private copies.

### 1b. `crates/htui/src/ui/tabs/skills/templates.rs`
- Delete `:74-81` (the three `const`s and their docs).
- `:29` becomes `use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG};`. Let `rustfmt`
  order it.
- The uses at `:452`, `:460` and `:461` do not change.

### 1c. `crates/htui/src/ui/tabs/skills/library.rs`
- Delete `:81-88`.
- `:39` becomes `use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG};`.
- The uses at `:610`, `:618`, `:619` and `:1010` (the rename's `NO_CHANGES`) do not change.

There are no new tests. The Skills suites assert the substrings (`"edited in $EDITOR"`, `"no changes"`,
`"code --wait"`), and they stay green.

Validate:
- `cargo test -p htui --all-features --lib skills`
- `cargo test -p htui --all-features --test templates --test skills`

## 2. Task 2, the form (D1–D5, D7), in `crates/htui/src/ui/tabs/backlog/item_form.rs`

### 2a. Module doc and imports
- Insert a bullet after the milestone 3 bullet (`:17-19`):
  `//! - **Ctrl+E hands Body or Paths to `$EDITOR`** (milestone 4 D1–D5): the form answers`
  `//!   [`ItemFormOutcome::External`] and records the field; [`ItemForm::on_external_edit`] puts the`
  `//!   text back, minus a final newline the editor added (D5). Saving is still Ctrl+S's compare-and-set.`
- Imports: add `use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG};` after
  `:37`.

### 2b. Types
- **Hint** (after `HINT_PICK`, `:51`):
  ```rust
  /// The hint under the paths and body areas (milestone 4 D7): 39 columns against the detail
  /// pane's 43, so `Tab field` gives way to Ctrl+E (Tab still cycles).
  pub const HINT_AREA: &str = "Ctrl+E $EDITOR  Ctrl+S save  Esc cancel";
  ```
- **`ItemFormOutcome`** (`:114-127`): append
  ```rust
  /// Ctrl+E on Body or Paths (milestone 4 D3): hand this text to `$EDITOR`. The outcome comes
  /// back through [`ItemForm::on_external_edit`]; the tab emits `Action::EditExternally`.
  External(ExternalEdit),
  ```
  `#[derive(Debug)]` still compiles: `ExternalEdit`'s hand-written `Debug` prints `text_len` and `stem`
  (`editor.rs:115-123`).
- **`ItemForm`** (`:133-166`): append
  ```rust
  /// The field handed to `$EDITOR` (milestone 4 D3) until its outcome comes back. A field, never
  /// text, so `Debug` may print it.
  external: Option<Field>,
  ```
  `build` (`:371-387`) sets `external: None`. `open_resolution` goes through `build`, so it gets the same.
- **`Debug for ItemForm`** (`:192-218`): add `.field("external", &self.external)` after `"resolving"`. This prints
  `external: Some(Body)`, never text.

### 2c. New private and public functions
Put `ctrl_e` beside `ctrl_s` (`:253-257`), and `strip_added_newline` beside `field` (`:270-282`):
```rust
/// Ctrl+E, with or without `SHIFT` (milestone 4 D1; the Templates editor's rule).
fn ctrl_e(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Char('e' | 'E'))
}

/// D5: `returned` without the one final `\n` an editor adds on save (vim's `fixeol`, nano, VS
/// Code), when `handed`, the text the field handed out, had none. Exactly one is dropped:
/// `"abc\n\n"` back from `"abc"` is `"abc\n"`. When `handed` ends in `\n`, `returned` is kept
/// whole. `returned` is already LF-only (`editor::run` normalises it).
fn strip_added_newline<'a>(handed: &str, returned: &'a str) -> &'a str {
    if handed.ends_with('\n') {
        returned
    } else {
        returned.strip_suffix('\n').unwrap_or(returned)
    }
}
```
**D5 semantics, precisely.** Let `handed` be the field's text at outcome time. `Edited(r)` gives
`t = strip_added_newline(handed, &r)`:
- If `t == handed`, the outcome reads as `Unchanged { quick: false }`: the notice is `NO_CHANGES` with no `WAIT_FLAG`,
  and the text is untouched.
- Otherwise the area becomes `TextArea::with_text(t)`.

`handed` is read back from the widget, not stored (D3 keeps a field, no text). This is exact because
`event_loop.rs:54-64` runs the handoff and `finish_external_edit` back to back. No key, reply or tick reaches the form
in between (§6 Q3).

In `impl ItemForm`:
```rust
/// D4: the `$EDITOR` handoff came back. No field out (`external` is `None`): ignored.
/// `Edited` replaces only the handed-out field's text (after D5), focuses it and says `EDITED`;
/// the token, reason, `opened` and every other field are untouched, so A4's "unchanged" still
/// compares against what the form opened with. `Unchanged` says `NO_CHANGES` (+ `WAIT_FLAG`
/// when quick); `Failed` says its sentence; neither touches the text.
pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome);

/// The text area behind `field`: Paths or Body; `None` for a one-line field or a picker.
fn area_mut(&mut self, field: Field) -> Option<&mut TextArea>;

/// The temp-file stem (D3): `<key>-body` / `<key>-paths`, `new-body` / `new-paths` on a new
/// form. `editor::run` sanitises it (D25).
fn stem(&self, field: Field) -> String; // owner = context.item.key or "new"; part = "paths" for Paths, else "body"

/// Ctrl+E while idle (D1, D3): Body or Paths answer `External` with that field's text and
/// record it in `external`; any other field swallows it (`Stay`).
fn hand_off(&mut self) -> ItemFormOutcome;
```
Body shapes:
- **`hand_off`**:
  1. `let field = self.focus; let stem = self.stem(field);`
  2. `let Some(area) = self.area_mut(field) else { return ItemFormOutcome::Stay };`
  3. `let text = area.text().to_owned(); self.external = Some(field);`
  4. `ItemFormOutcome::External(ExternalEdit { text, stem })`. The notice is left alone, as Templates' `hand_off`
     leaves it (`templates.rs:781-797`). The outcome always sets it.
- **`on_external_edit`**:
  1. `let Some(field) = self.external.take() else { return };`
  2. `match outcome`:
     - `Edited(returned)`:
       - `let Some(area) = self.area_mut(field) else { return };` (unreachable: only areas are handed out)
       - `let text = strip_added_newline(area.text(), &returned);` The result borrows `returned`, not `area`.
       - If `text == area.text()`: `self.notice = Some(NO_CHANGES.to_owned())`.
       - Else: `*area = TextArea::with_text(text); self.focus = field; self.notice = Some(EDITED.to_owned());`
     - `Unchanged { quick }`: `self.notice = Some(format!("{NO_CHANGES}{}", if quick { WAIT_FLAG } else { "" }))`.
     - `Failed(message)`: `self.notice = Some(message)`.
- The cursor after `Edited` is at byte 0 (`with_text`'s default, as on open). `busy` and `resolving` are not
  consulted. Neither can be set: Ctrl+E is refused while busy, and the view passes it as a chord.

### 2d. `on_key` order (`:475-543`)
The doc becomes "In order: an open divergence view takes every key (milestone 3 D5); Ctrl+S saves (A6; swallowed
while busy); Ctrl+E hands Body or Paths to `$EDITOR` (milestone 4 D1, D2; swallowed on any other field and while
busy); any other chord passes; …". Insert between the Ctrl+S block (`:485-490`) and the chord pass (`:491-493`):
```rust
// Milestone 4 D1: before the chord pass, which would hand it to the tab.
if ctrl_e(&key) {
    return match self.busy {
        Some(_) => ItemFormOutcome::Stay,
        None => self.hand_off(),
    };
}
```
So the full order is:
1. `resolving` gives `on_view_key`. Ctrl+E passes there as a chord, per D2's last sentence (`divergence.rs` is
   unchanged).
2. Ctrl+S.
3. **Ctrl+E.**
4. The `CHORD` pass.
5. Busy swallows everything.
6. `Tab`/`BackTab`.
7. The focused field.

**E1, compile fix in the same commit** (`backlog/mod.rs`):
- `on_item_form_key`'s match (`:372-381`) gains `ItemFormOutcome::External(edit) => ctx.emit(Action::EditExternally(edit)),`.
- Its doc (`:367`) becomes "The form itself passes chords but Ctrl+S (A6) and Ctrl+E (milestone 4 D1)".
- `Action` is already imported (`:31`), and `Ctx::emit` takes `&self` (`state.rs:121`).

### 2e. Render (D7, `:967-970`)
```rust
let text = match form.focus {
    Field::Project | Field::Kind | Field::Graph => HINT_PICK,
    Field::Title | Field::Priority | Field::Tags => HINT_TEXT,
    Field::Paths | Field::Body => HINT_AREA,
};
```
No other draw change. Both `backlog__item_form_{new,edit}.snap` focus Title, `backlog__item_divergence.snap` draws the
view, and the unit hedge tests (`:1518-1587`) focus Title. **No existing snapshot moves.**

### 2f. Task 2 unit tests (`item_form.rs` `mod tests`)
Reuse `key`, `ctrl`, `type_text`, `new_form`, `edit_form`, `focus_on`, `edit_sent`, `drawn`, `behind`, `resolving`,
`retitled`, and the store/fixture imports. Add a helper:
```rust
/// Ctrl+E on `form`'s focused field: the `ExternalEdit` it answered, or a panic naming the outcome.
fn handed(form: &mut ItemForm) -> ExternalEdit
```
1. `ctrl_e_on_the_body_or_paths_hands_out_that_text_with_the_item_key_stem`
   - Edit form on `HTUI_ANA_1`, `focus_on(Body)`. `handed` gives `ExternalEdit { text: form.opened.body.clone(),
     stem: "ANA-1-body" }`, `form.external == Some(Field::Body)` and `busy() == None`.
   - `focus_on(Paths)`, `on_paste("src/**")`. `KeyEvent::new(Char('E'), CONTROL | SHIFT)` answers `External` with
     `text: "src/**"` and `stem: "ANA-1-paths"`.
2. `ctrl_e_on_a_new_form_uses_the_new_stem`: `new_form`, `focus_on(Body)`, `on_paste("draft")` gives
   `{ text: "draft", stem: "new-body" }`.
3. `ctrl_e_on_a_one_line_field_or_a_picker_is_swallowed` (D1)
   - On the edit form, for each of `Kind, Title, Priority, Tags, Graph`, Ctrl+E is `matches!(.., Stay)`. `external` is
     `None` and the focus is unchanged.
   - On a new form, `Project` gives the same.
   - Never `Pass`, never `External`.
4. `ctrl_e_while_busy_is_swallowed` (D2)
   - `new_form`, type `Fresh item`, `focus_on(Body)`.
   - `ctrl('s')` gives `Save`, and `busy() == Some(Minting)`.
   - `ctrl('e')` gives `Stay`, and `external` is `None`.
5. `ctrl_e_in_the_divergence_view_passes` (D2): `resolving(&store, retitled("Theirs"))`, then `ctrl('e')` gives
   `Pass`, and `external` is `None`.
6. `an_edited_body_replaces_only_the_body_and_saves_it_at_the_opened_version` (D4)
   - Edit form, `focus_on(Body)`, `handed`, then `form.focus = Field::Title`. This stands in for the move D4 undoes;
     the loop never does it.
   - `on_external_edit(Edited("New body.\n"))`:
     - `body.text() == "New body."` (D5);
     - `focus == Body` and `notice() == Some(EDITED)`;
     - `title.text()` and `paths.text()` are unchanged;
     - `token() == Some(1)`, `reason() == EditReason::Edited`, and `external` is `None`.
   - `edit_sent` gives `(1, SpecChanges { body: Some("New body.".into()), ..Default }, EditReason::Edited)`.
7. `an_edited_paths_area_lands_in_paths`: `focus_on(Paths)`, `handed`, then
   `on_external_edit(Edited("src/**\nlib/**\n"))`. `paths.text() == "src/**\nlib/**"`, the body is unchanged and the
   focus is Paths. Not saved: the demo project has no repo (`:1911`).
8. `an_edit_back_to_the_opened_text_has_nothing_to_save` (A4)
   - `focus_on(Body)`, type `x` (this prepends at byte 0), `handed`.
   - `on_external_edit(Edited(form.opened.body.clone()))` gives `notice() == Some(EDITED)`.
   - `ctrl('s')` gives `Stay` with `NOTHING_TO_SAVE`.
9. `unchanged_and_failed_keep_the_text`. For each case, a fresh `handed` on Body:
   - `Unchanged { quick: true }`: the notice is `format!("{NO_CHANGES}{WAIT_FLAG}")`.
   - `Unchanged { quick: false }`: the notice is `NO_CHANGES`.
   - `Failed("boom")`: the notice is `"boom"`.
   - In every case the body text is unchanged.
10. `strip_added_newline_drops_exactly_one_editor_newline` (the pure function):

    | handed | returned | result |
    |---|---|---|
    | `"abc"` | `"abc\n"` | `"abc"` |
    | `"abc"` | `"abd\n"` | `"abd"` |
    | `"abc"` | `"abc\n\n"` | `"abc\n"` |
    | `"abc\n"` | `"abc\n\n"` | `"abc\n\n"` |
    | `""` | `"\n"` | `""` |
    | `"abc"` | `"abc"` | `"abc"` |
11. `an_editor_added_final_newline_is_no_change` (D5 through the form)
    - `form.body = TextArea::with_text("abc")`, `focus_on(Body)`, `handed`.
    - `Edited("abc\n")`: the notice is exactly `NO_CHANGES` (no `WAIT_FLAG`) and the body is `"abc"`.
    - `handed`, then `Edited("abd\n")`: the body is `"abd"`.
    - `form.body = TextArea::with_text("abc\n")`, `handed`, then `Edited("abc\n\n")`: the body is `"abc\n\n"` and the
      notice is `EDITED`.
12. `an_outcome_with_no_edit_out_is_ignored`
    - Edit form, no Ctrl+E. `on_external_edit(Edited("x"))` leaves the body unchanged and `notice() == None`.
    - Then `handed`, `Failed("one")`, `Failed("two")`: the notice stays `"one"`, because the second outcome has no
      field out.
13. `the_hints_fit_the_detail_pane` (`:1427`): the array becomes `[HINT_TEXT, HINT_PICK, HINT_AREA]`.
14. `the_body_and_paths_areas_hint_ctrl_e` (D7)
    - `new_form`. `focus_on(Body)`: the drawn text contains `HINT_AREA` and no `"Tab field"`.
    - `focus_on(Paths)`: the same.
    - `focus_on(Title)`: `HINT_TEXT`.
15. `debug_prints_no_body_while_an_edit_is_out`
    - `new_form`, `focus_on(Body)`, `on_paste("SECRET-BODY")`, `let outcome = form.on_key(ctrl('e'))`.
    - `{outcome:?}` holds `text_len` and no `SECRET-BODY`.
    - `{form:?}` and `{form:#?}` hold no `SECRET-BODY`. `{form:?}` holds `external: Some(Body)`.

Validate: `cargo test -p htui --all-features --lib backlog::item_form`, then
`cargo check -p htui --all-features --all-targets` (E1).

## 3. Task 3, tab wiring and integration (D3, D4, D6)

### 3a. `crates/htui/src/ui/tabs/backlog/mod.rs`
- **Module doc** (`:18-19`): append "Milestone 4: Ctrl+E in the item form's body or paths hands that text to
  `$EDITOR` (MOD-9's handoff). The outcome comes back through `Tab::on_external_edit` to the form, and Ctrl+S saves it
  as before."
- **Import**: `use crate::editor::ExternalEditOutcome;` after `:31`.
- **`impl Tab for BacklogTab`**: add after `on_reply` (`:631-676`):
  ```rust
  /// MOD-13 milestone 4 D4: the `$EDITOR` outcome goes to the open item form, which ignores one
  /// it did not ask for. With no form (a scope change landed first) it is dropped, as
  /// `App::finish_external_edit` drops one for a gone tab.
  fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
      match self.item_form.as_mut() {
          Some(form) => form.on_external_edit(outcome),
          None => tracing::debug!("the item form that asked for the editor is gone"),
      }
  }
  ```
  `render`, `on_key`, `on_paste`, the reveal guard and `on_scope_change` do not change.

### 3b. `backlog/mod.rs` unit tests (reuse `Bench`, `open_with`, `press`, `ctrl`, `sent`, `saved`, `notice`)
Add `fn to_body(tab, bench)`: `press(.., KeyCode::Tab)` five times (Title, Priority, Tags, Graph, Paths, Body).
1. `ctrl_e_on_the_body_emits_an_external_edit_for_the_loop`
   - `open_with(.., 'e')`, `to_body`. `tab.on_key(ctrl('e'), &mut bench.ctx()) == Handled::Consumed`.
   - `bench.actions()` is exactly one `Action::EditExternally(edit)`, with
     `edit.stem == format!("{}-body", item.key)` and `edit.text == item.body`. `item` is the `MemStore::demo()` row of
     `bench.first`.
   - There is no `Action::Store`.
2. `ctrl_e_on_the_title_is_consumed_and_emits_nothing` (D1: it never leaks to a global binding): `Handled::Consumed`,
   and `bench.actions()` is empty.
3. `the_outcome_reaches_the_open_form_and_ctrl_s_saves_the_body`
   - After test 1's steps, `tab.on_external_edit(ExternalEditOutcome::Edited("New body.".into()), &mut bench.ctx())`.
     `notice(&tab) == Some(crate::editor::EDITED)`.
   - `saved` gives `EditItem { expected_version: 1, changes, reason: EditReason::Edited, .. }`, with
     `changes == SpecChanges { body: Some("New body."), ..Default }`.
4. `an_outcome_with_no_form_open_is_dropped`: `bench.tab()`, then `on_external_edit(Edited("x"))`. `item_form` is
   `None` and `bench.actions()` is empty.

### 3c. Integration, `crates/htui/tests/backlog.rs`
Append a section after the divergence cases (the file ends at `:2169`):
`// $EDITOR round-trip (MOD-13 milestone 4, plan D1-D8).`
- Imports: `use htui::editor::{ExternalEdit, ExternalEditOutcome};`.
- Reuse `backlog_over`, `keys`, `type_text`, `TITLE_TO_BODY` (`:1955`), `their_write` (`:1969`), `retitled`
  (`:1726`), `ana_1_head`, `divergence_state` (`:1982`) and `detail_pane`.
- Add `async fn ana_1(store: &MemStore) -> Item`, the head row, read as `ana_1_head` does.

1. **`ctrl_e_hands_the_body_out_and_ctrl_s_saves_what_came_back`**
   - `backlog_over(store.clone())`, `keys(["e"])`, `keys(TITLE_TO_BODY)`, `keys(["ctrl-e"])`.
   - `take_external_edit() == Some((BacklogTab::ID, ExternalEdit { text: ana_1(&store).await.body, stem: "ANA-1-body" }))`.
     A second `take_external_edit()` is `None`.
   - `finish_external_edit(BacklogTab::ID, Edited("New body.\n"))`. The frame holds `edited in $EDITOR` and
     ` Edit ANA-1 (v1) `.
   - `keys(["ctrl-s"])`: `ana_1(&store)` has `body == "New body."` (D5) and `version == 2`. The detail pane holds
     `version 2` and no ` Edit `, and `status == None`.
2. **`a_reply_and_a_concurrent_write_around_the_editor_end_in_the_divergence_view`** (the PRD risk, E2)
   - `backlog_over`, `keys(["e"])`, `keys(TITLE_TO_BODY)`.
   - `for _ in 0..4 { harness.app().update(Action::Tick) }`. That is one shell refresh, `TICKS_PER_REFRESH = 4`
     (`update.rs:19`, private, so a comment names it): `StoreState` and `ActiveRuns` are queued.
   - `harness.key("ctrl-e")`: **not** `keys`, so nothing is served.
   - `let (tab, edit) = take_external_edit().expect(..)`.
   - `their_write(&store, retitled("Theirs"))`: another writer, while the editor is open.
   - `harness.settle()`: the queued replies land before the outcome (the plan's order).
   - Four more ticks, then `finish_external_edit(tab, Edited(format!("{}\nWritten in the editor.\n", edit.text)))`,
     then `harness.settle()`: those replies land after it (the loop's order, `event_loop.rs:64` then `:45`).
   - The frame holds `edited in $EDITOR` and ` Edit ANA-1 (v1) `. The token did not move.
   - `keys(["ctrl-s"])`:
     - the frame holds `ANA-1  v1 \u{2192} theirs v2` and `Written in the editor.` (mine's diff);
     - `divergence_state(&frame, "title") == "theirs"` and `divergence_state(&frame, "body") == "mine"`;
     - `ana_1_head == ("Theirs", 2)`.
   - `keys(["m"])`, `keys(["ctrl-s"])`: the head has title `Theirs`,
     `body == format!("{}\nWritten in the editor.", edit.text)` and version 3. `status == None`.
3. **`a_fake_editor_appends_a_line_and_ctrl_s_saves_it`**, `#[cfg(unix)]` (E3)
   - Imports inside the function: `use std::os::unix::fs::PermissionsExt as _; use htui::editor::EditorCommand;`.
   - Up to `take_external_edit` as in case 1.
   - The script: `let dir = tempfile::TempDir::new()`, path `dir.path().join("append")`, written with
     `std::fs::write(&path, "#!/bin/sh\nprintf '\\nAppended by the editor.\\n' >> \"$1\"\n")` (R-13: the handle
     closes at once), then `set_permissions(.., from_mode(0o755))`.
   - `let cmd = EditorCommand::resolve(move |key: &str| (key == "VISUAL").then(|| format!("'{}'", path.display())));`
   - `let outcome = htui::editor::run(&cmd, &edit.text, &edit.stem).await;`
     `assert_eq!(outcome, Edited(format!("{}\nAppended by the editor.\n", edit.text)))`.
   - `finish_external_edit(tab, outcome)`, `keys(["ctrl-s"])`: the head body is
     `format!("{}\nAppended by the editor.", edit.text)` (D5 dropped the editor's final newline), at version 2.
     `status == None`.
   - No terminal is driven. `run_suspended`'s leave/enter is pinned by `editor.rs` `mod suspension`.
4. **`offline_ctrl_e_asks_for_no_editor`** (D6)
   - Extract `async fn offline_backlog(root: &std::path::Path) -> (Harness, CacheStore)` from
     `offline_n_and_e_are_refused_with_the_read_only_notice` (`:1853-1875`: open the cache, seed it, build the
     harness, set the scope).
   - The existing test calls it. Its `_keyring` guard and `tempdir` stay in the test body, and its asserts do not
     change.
   - The new test: the guard, a `tempdir`, `offline_backlog`, then `harness.app().status = None`.
   - `keys(["e"])`: the status is the read-only sentence, and no ` Edit ` is drawn.
   - `keys(["ctrl-e"])`: `take_external_edit().is_none()`. With no form, the chord reaches `detail.on_key`
     (`mod.rs:584-589`). No detail sub-tab and no global binding claims Ctrl+E (Gortex: `Char('e' | 'E')` appears only
     in Skills, and `keymap.rs:248` is the only CONTROL default).

All frames are 100x30. No snapshot is added or changed.

Validate:
- `cargo test -p htui --all-features --lib ui::tabs::backlog` (E4)
- `cargo test -p htui --all-features --test backlog -- --test-threads=1`

## 4. Task 4, close-out
- PRD row 4 → `complete`.
- This plan's status line.
- The HANDOFF MOD-13 phase-4 note (`references/lifecycle.md` P1).
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

No `StoreRequest`/`StoreReply` variant is added, so HANDOFF's counts do not move.

## 5. Commit boundaries and hazards
Implementers commit at each green. Stage explicit paths: no `-A`, no stash, no amend.
1. `refactor(mod-13): the $EDITOR notices live in crate::editor`: `editor.rs`, `skills/templates.rs`,
   `skills/library.rs`.
2. `feat(mod-13): Ctrl+E hands the item form's body or paths to $EDITOR`: `item_form.rs`, plus the §2d arm and doc
   line in `backlog/mod.rs` (E1).
3. `feat(mod-13): the Backlog routes the $EDITOR outcome to its item form`: `backlog/mod.rs`.
4. `test(mod-13): $EDITOR round-trip integration cases`: `tests/backlog.rs`.
5. `docs(mod-13): milestone 4 close-out`: the PRD, HANDOFF and the plan.

Each commit passes `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D
warnings`. Every message ends with the `Co-Authored-By` line. Close with the plan's Validation block, run with
`--no-fail-fast -- --test-threads=1`, and grep the output for `SIGABRT`.

**Hazards.**
- **Lints.**
  - `missing_docs` covers `HINT_AREA`, `ItemFormOutcome::External`, `ItemForm::on_external_edit` and the three
    `editor` constants.
  - `unused_qualifications`: do not write `crate::editor::EDITED` where it is imported. The `mod.rs` tests may use the
    full path, because `EDITED` is not imported there.
  - On Windows, an import used only by the `#[cfg(unix)]` test would be unused. Keep those imports inside the
    function.
- **Redaction (E10).**
  - `external` is a `Field`.
  - `ExternalEdit`/`ExternalEditOutcome` print lengths only.
  - `ItemFormOutcome::External`'s derived `Debug` goes through `ExternalEdit`'s.
  - §2f test 15 pins all three.
- **Key order.** Ctrl+E must sit after the view check and Ctrl+S, and before the chord pass (`:491`). Otherwise it
  `Pass`es to the tab, and `on_item_form_key` returns `Handled::Pass`.
- **The fake editor.**
  - Run it under `--test-threads=1`: the keyring fake is process-wide.
  - Write the script with `std::fs::write` (R-13 `ETXTBSY`).
  - `editor::run` installs htui's SIGINT/SIGQUIT listeners for good (`editor.rs` `Interrupts`). After this case,
    Ctrl-C to the `backlog` test binary is swallowed, as it already is in the lib test binary. This is harmless.
- **A BOM** (informational). `run` strips a leading BOM from the edited side (`normalise_newlines`). A stored body that
  starts with one comes back as a body change once the user edits anything. This is pre-existing MOD-9 behaviour,
  shared with Templates, and out of scope.
- **Suite hygiene.** Use `--all-features` (`testkit`), or `tests/*.rs` run 0 tests.

## 6. Deviations and open questions for the maintainer
Nothing blocks. Q1 is applied in this blueprint (E2), and the rest are reported without a change of decision.

1. **Q1: the mid-suspend test mechanism (plan T3 test 2), applied.**
   - The plan's "list re-read reply driven before `finish_external_edit`" cannot be produced. Nothing queues one while
     an idle form is open, and the real loop delivers replies *after* the outcome (E2).
   - §3c test 2 queues `Origin::App` polls with ticks instead, and delivers one batch on each side of the outcome.
   - No Backlog-addressed reply can honestly be in flight here. The only one is the `Runs` poll, every fifth refresh
     and only with an active run on the selected item, and `ANA-1` has none.
   - The property that matters holds by construction: no reply reaches a tab between the handoff and the outcome
     (`event_loop.rs:54-64`). Test 2 pins the rest: the text and token survive the replies and a concurrent write, and
     the divergence view carries the editor's text.
2. **Q2: D6 "online-only holds by construction" is true only for a form opened offline.**
   - The worker drops to `Offline` at runtime (`go_offline`, `store_worker.rs:2683`, called at `:2428` and `:2528`).
     An item form opened online survives the drop: only Cancel, an applied write and a scope change close it.
   - In that form Ctrl+E still opens the editor. Typing has worked there since milestone 2.
   - Nothing is written. The editor touches no store, and Ctrl+S's `EditItem` is refused offline before any read
     (`item_writes.rs:891`), with the reason in the form's notice.
   - So the PRD metric (line 49: "absent **or refused** with a visible read-only notice") is met. The plan's
     acceptance line "No editor action reachable while `Offline`" is not met literally.
   - Recommendation: no code change. Reword the acceptance to "no write reachable while `Offline`; a form never opens
     offline". Gating Ctrl+E on the live store state would need the form to read `Ctx`, which D3 rules out.
3. **Q3: D5's baseline is the widget, not a stored copy.** D3 keeps `external: Option<Field>`, so D5 compares against
   the field's text when the outcome lands. This is exact in the loop. In the harness, a test that typed into the
   field between `take_external_edit` and `finish_external_edit` would move the baseline, and no test here does.
   Storing the handed-out text would need a second field and a redaction rule, and it adds nothing.
4. **Q4: the notice colour** (informational). The form draws every notice in `theme.error` (`item_form.rs:958-966`),
   so `EDITED`/`NO_CHANGES` show red, as `rebased_on`/`still_behind` already do. Templates tells `Info` from `Error`.
   No decision covers this, and snapshots are text. Left as is. It is a candidate for a later polish item.
5. **Q5: `NO_CHANGES` is also the Library's rename notice** (`library.rs:1010`). The hoisted constant's doc says so,
   and the string is unchanged.
6. **Additions beyond the plan's test list.** None of them changes a decision:
   - §2f tests 5 (D2's view clause), 7 (a Paths round-trip), 10 (the pure D5 function) and 12 (a second outcome is
     ignored);
   - §3b test 3 (routing the outcome to the form);
   - the `offline_backlog` helper extraction (§3c test 4).
