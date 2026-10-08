# MOD-67 M5 blueprint — Lane L-A: the Backlog tab, its forms, Notes and Docs

Scope: `crates/htui/src/ui/tabs/backlog/{mod,filter,item_form,divergence}.rs`,
`crates/htui/src/ui/tabs/backlog/detail/{compose,notes,documents}.rs`, and in
`crates/htui/src/app/mod.rs` only the eight Backlog `Keymap` rows, their doc items and the imports
they alone use. Owned tests: `tests/backlog_keys/l_a.rs`, the in-file tests of those files;
flagged edits in `tests/backlog.rs` (PA-3: `:268`, `:1810`, `:2392`, `:3398`, `:2633`) and
`tests/replay.rs` (`:217-226`, the help-line half only). Snapshots: the 15 of the plan's L-A row.
Branch `mod-67-m5-l-a`, worktree `target/wt/l-a`.

Lane rules (plan, M3/M4): never edit `keys/`, `app/` beyond the rows above, the registries,
`testkit.rs`, `tests/keys*.rs`, `ui/text_area.rs`, `ui/text_field.rs`, `backlog/list.rs`,
`backlog/detail/{mod,body,graph,prompt,requirements,runs}.rs`, `chat/**`; a missing act or stack
goes back to the main thread. Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and
`--test-threads=1`. Commit incrementally; stage own paths only.

Complete: sections 0-9.

## 0. Facts (read at `3af293a6`)

- `BacklogTab::on_key` (`mod.rs:733-800`): filter form first (`:736`), item form (`:741`), a
  capturing pane (`:744`), then CONTROL/ALT to the pane (`:746-752`), then the tab's own arms
  (`:754-797`), `_ => self.detail.on_key` (`:798`). `Enter` folds a header, else the pane
  (`:791-797`); the pane's `Pass` reaches the shell, whose legacy `Keymap` row (`app/mod.rs:143`)
  answers the miss.
- `on_form_key` (`:355-375`): a `filter::CHORD` chord passes before the form (`:356`); the form
  answers `Stay`/`Cancel`/`Refused`/`Apply`, every plain key consumed.
- `on_item_form_key` (`:509-531`): the form answers `Pass` (chords), `Stay`, `Cancel`, `Reload`,
  `Save`, `External`.
- `FilterForm::on_key` (`filter.rs:228-262`), `ItemForm::on_key` (`item_form.rs:589-659`),
  `Divergence::on_key` (`divergence.rs:236-266`), `Compose::on_key` (`compose.rs:318-372`) are
  pure (no `Ctx`): they take `keys: &Keys` after this lane (main blueprint §6).
- `TextField::on_key` passes every CONTROL/ALT/SUPER/META/HYPER chord, `Tab`, `BackTab`, `Up`,
  `Down`, `PgUp`, `PgDn`, `F(n)`; `Enter` = `Submit`, `Esc` = `Cancel`. `TextArea::on_key` passes
  every chord (`ctrl-s` included since M4 T-close) and `Tab`/`BackTab`/`F(n)`; `Enter` is a
  newline; `Esc` = `Cancel`; it never answers `Submit`.
- T1 provides: every stack and act used below, `DetailTab::key_stack` (default
  `views::BACKLOG_BODY`) and `DetailRegistry::key_stack`, the `VIEW_DEFAULTS` rows for
  `BacklogDocs`/`BacklogFilter`/`BacklogItem` `FormNextField`/`FormPrevField` (`down`/`up`).
- The other panes (Body, Runs, Graph, Prompt, Reqs) are unconverted on this branch: they report
  `BACKLOG_BODY`, and every one returns `Pass` for the tab's keys (their `_` arms), so the
  pane-first order (§2.1) changes nothing for them.

## 1. Modes and stacks

| Mode | Handler today | captures | Stack |
|---|---|---|---|
| Backlog, no form, pane not capturing | `on_key` `:753-799` | no | `self.detail.key_stack()` (`BACKLOG_NOTES`, `BACKLOG_DOCS` here; the others' lanes) |
| filter form open (any row) | `on_form_key` `:355` → `FilterForm::on_key` | yes | `BACKLOG_FILTER` |
| item form open, also busy | `on_item_form_key` `:509` → `ItemForm::on_key` | yes | `ITEM_FORM` |
| item form, divergence view open | `ItemForm::on_view_key` `item_form.rs:680` → `Divergence::on_key` | yes | `ITEM_DIVERGENCE` |
| Notes, no area | `NotesTab::on_key` `notes.rs:291` | no | `BACKLOG_NOTES` |
| Notes, area open | `on_compose_key` `notes.rs:179` → `Compose::on_key` | yes | `NOTES_COMPOSE` |
| Docs, no area | `DocumentsTab::on_key` `documents.rs:445` | no | `BACKLOG_DOCS` |
| Docs, area open | `on_compose_key` `documents.rs:190` → `Compose::on_key` | yes | `DOCS_COMPOSE` |

```rust
// impl Tab for BacklogTab
/// The stack of what holds the Backlog's keys (MOD-67 M5 D4): the filter form's, else the item
/// form's (its divergence view's while resolving), else the active pane's.
fn key_stack(&self) -> Option<Stack<'static>> {
    if self.form.is_some() {
        return Some(views::BACKLOG_FILTER);
    }
    if let Some(form) = &self.item_form {
        return Some(form.key_stack());
    }
    Some(self.detail.key_stack())
}

// impl ItemForm
/// `ITEM_DIVERGENCE` while the view is open, else `ITEM_FORM` (busy included).
pub fn key_stack(&self) -> Stack<'static> {
    if self.resolving.is_some() { views::ITEM_DIVERGENCE } else { views::ITEM_FORM }
}

// impl Compose
/// `NOTES_COMPOSE` for a note (no kind), `DOCS_COMPOSE` for a document.
pub fn stack(&self) -> Stack<'static> {
    match self.kind { None => views::NOTES_COMPOSE, Some(_) => views::DOCS_COMPOSE }
}

// impl DetailTab for NotesTab (and DocumentsTab with BACKLOG_DOCS)
fn key_stack(&self) -> Stack<'static> {
    self.compose.as_ref().map_or(views::BACKLOG_NOTES, Compose::stack)
}
```

## 2. Arm-by-arm conversion

### 2.1 `backlog/mod.rs` (`BacklogTab::on_key`, main blueprint §6.1)

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:736-738` | filter form first | unchanged order → `on_form_key` (§2.3) | |
| `:741-743` | item form first | unchanged order → `on_item_form_key` (§2.4) | |
| `:744-746` | capturing pane gets every key | unchanged | the pane's capture stack decides `Pass`; the shell then resolves `self.key_stack()` (the same stack) |
| `:746-752` | CONTROL/ALT → pane | **removed** (D7): chords reach the pane, then the tab's acts (chord equality: `ctrl-l` is no act) | D12 pin |
| — | (new) | `stack = self.detail.key_stack()`; `acts = actions(stack, chord)`; `ListFold` in `acts` and `self.fold(ctx) == Consumed` → `Consumed` | PA-2: the fold before the pane, today's order |
| — | (new) | `self.detail.on_key(key, ctx) == Consumed` → `Consumed` | D5: the pane first |
| `:754` | `j`/`Down` → `step(1)` | `ListDown` | |
| `:755` | `k`/`Up` → `step(-1)` | `ListUp` | |
| `:756` | `g`/`Home` → `jump(false)` | `ListTop` | |
| `:757` | `G`/`End` → `jump(true)` | `ListBottom` | |
| `:758` | `l`/`]`/`Right` → `cycle_next` | `PaneNextSubtab` | |
| `:759` | `h`/`[`/`Left` → `cycle_prev` | `PanePrevSubtab` | |
| `:762-764` | `m` → Graph | `BacklogOpenGraph` | |
| `:767-769` | `f` → filter form | `BacklogOpenFilter` | |
| `:770-774` | `F` → clear when set | `BacklogClearFilter` | consumed either way, as today |
| `:778` | `N` → `open_new` | `BacklogNewItem` | |
| `:779` | `e` → `open_edit` | `Edit` | |
| `:783` | `Q` → `toggle_queued` | `BacklogQueue` | |
| `:784-787` | `P` → `QueueState` read | `BacklogPauseQueue` | |
| `:791-797` | `Enter`: fold, else the pane | fold first (above), pane (above), then **`BacklogReplay` → `ctx.emit(Action::Error(replay_miss(ctx.keys())))`**, `Consumed` | D6: the miss leaves `app/mod.rs`; `replay_miss` per main §6.1 (reads `views::RUNS_BROWSE`, `J/K` by default) |
| `:798` | `_ → detail.on_key` | `_ => continue`, loop end `Handled::Pass` | the pane was already offered the key |
| `crossterm` import `:51` | `KeyCode, KeyEvent, KeyModifiers, MouseEvent` | `KeyEvent, MouseEvent` (+ `crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views}`) | |

`fold` (`:290`) unchanged. Module doc (`:8-26`): "`f` opens …, `F` clears it" etc. are prose;
add one line "Keys resolve through the focused pane's stack (MOD-67 M5)"; the `Ctrl+S`/`Ctrl+E`
sentences stay (M6, D11).

### 2.2 `app/mod.rs` (Backlog rows only, D6)

| file:line | today | → |
|---|---|---|
| `:137-148` | comment + `Enter` miss row | deleted (the miss is `BacklogReplay`) |
| `:149-157` | `m` help row | deleted |
| `:158-167` | `f`/`F` help rows | deleted |
| `:168-178` | `N`/`e` help rows | deleted |
| `:179-189` | `Q`/`P` help rows | deleted |
| `:14`, `:16` | `use crossterm::event::{KeyCode, KeyModifiers}`, `use crate::keymap::{Binding, KeyChord, KeyScope}` | deleted if nothing else in the file uses them (checked at `3af293a6`: nothing) |
| doc `:47-49` (item 5) | "… and the Backlog tab's `Enter` is bound to the refusal …" | "The Chat tab is named as the replay tab (MOD-2 D39); the Backlog answers a missed `Enter` itself (`backlog.replay`, MOD-67 M5 D6)." |
| doc `:56-61` (items 8-10) | the `m`, `f`/`F`, `N`/`e` binding items | deleted; items 11-12 renumber to 8-9 |

`app.replay_tab` (`:133`) and `app.reveal_tabs` stay.

### 2.3 Filter form (`filter.rs`; `on_form_key` `mod.rs:355-375`)

`FormOutcome` gains `Pass` ("a key the form did not use: the tab answers it with `modal_rest`").
`FilterForm::on_key(&mut self, key: KeyEvent, keys: &Keys) -> FormOutcome`:

| file:line | today | → | notes |
|---|---|---|---|
| `:229-231` | `CHORD` → `Stay` | removed; the option row's fixed keys check `plain(&key)` instead | `alt-x` reaches the resolver (no act) → `Pass` → `modal_rest` passes it |
| `:233-235` | Tags: field `Submit` → apply, `Cancel` → cancel | widget-owned, stay | |
| `:236-242` | Tags: field `Pass` → `Up`/`BackTab` row −1, `Down`/`Tab` row +1 | field `Pass` → resolver: `FormPrevField` (`backtab`, `up`) / `FormNextField` (`tab`, `down`) → `move_row`; else `Pass` | `VIEW_DEFAULTS` give `down`/`up` |
| `:244` | Tags: field `Consumed` → `Stay` | stays | `j`, `k`, `x`, `h`, `l`, Space are typed there |
| `:248` | option row `Enter` → apply | **fixed** (the form's own `Enter`), plain only | |
| `:249` | option row `Esc` → cancel | **fixed** (its own `Esc`), plain only | |
| `:250` | `j`/`Down`/`Tab` → row +1 | `FilterNextRow` (`j`), `FormNextField` (`tab`, `down`) → `move_row(1)` | |
| `:251` | `k`/`Up`/`BackTab` → row −1 | `FilterPrevRow` (`k`), `FormPrevField` (`backtab`, `up`) → `move_row(-1)` | |
| `:252` | `h`/`Left` → option −1 | **fixed** value key, plain only | ANA §6.1 |
| `:253` | `l`/`Right` → option +1 | **fixed** value key, plain only | |
| `:254` | Space → toggle | **fixed** value key, plain only | |
| `:255-258` | `x` → clear the draft and the tags | `FilterClear` | main blueprint §1, borderline keys |
| `:259` | `_ → Stay` | resolver miss → `FormOutcome::Pass` | |
| `:38-43` | `pub const CHORD` | deleted once no caller is left (§2.4-§2.6 drop the others) | D8 |

`on_form_key` (`mod.rs:355`): drop the `CHORD` pass (`:356-358`); `form.on_key(key, ctx.keys())`;
`FormOutcome::Pass => return modal_rest(views::BACKLOG_FILTER, KeyChord::from_event(key))`; the
other arms unchanged. Its doc (`:348-354`) becomes "Every plain key the form does not use is
consumed; CONTROL, ALT and function keys pass (`BACKLOG_FILTER`'s modal layer), so `ctrl-c`
quits, `Ctrl+f` searches and `F1` opens help."

### 2.4 Item form (`item_form.rs`; `on_item_form_key` `mod.rs:509-531`)

`ItemForm::on_key(&mut self, key: KeyEvent, keys: &Keys) -> ItemFormOutcome` (main blueprint
§6.3; stack `ITEM_FORM`):

| file:line | today | → | notes |
|---|---|---|---|
| `:590-592` | divergence view takes every key | unchanged → `on_view_key(key, keys)` (§2.5) | |
| `:593-598` | `ctrl_s` → save (busy: `Stay`) before anything | **after the widget**: `FormSave` → `save()`; busy: `FormSave` → `Stay` | M4 PA-3 order; a rebound `f2` save works on every field (D8) |
| `:600-605` | `ctrl_e` → `hand_off` (busy: `Stay`) | after the widget: `FormExternalEditor` → `hand_off()` (a non-area field: `Stay`, as today); busy → `Stay` | |
| `:606-608` | `CHORD` → `Pass` | removed: a field passes chords, a picker reads plain keys only, the resolver then answers or `modal_rest` passes | |
| `:609-611` | busy → `Stay` | busy: `FormSave`/`FormExternalEditor` → `Stay`; `stack.passes(chord)` → `Pass`; else `Stay` | `Esc` still swallowed while busy |
| `:613-620` | `Tab`/`BackTab` → move focus, before the widget | after the widget: `FormNextField`/`FormPrevField` → `move_focus` | every widget passes `Tab` |
| `:624` → `:719-731` `on_picker_key` | `Left`/`h`, `Right`/`l` pick; `Enter`/`Down` next; `Up` prev; `Esc` cancel; else `Stay` | `on_picker_key(&key) -> Option<ItemFormOutcome>`: plain only; `Left`/`h`, `Right`/`l` → `pick` (**fixed** value keys); `Enter` → `move_focus(true)` (**fixed**, the picker's own move-on); `Esc` → `Cancel` (**fixed**); else `None` → resolver (`Down`/`Up` are `FormNextField`/`FormPrevField` by `VIEW_DEFAULTS`) | an `alt-h` never picks (was the `CHORD` guard) |
| `:625-643` | text field: `Submit` → next field; `Cancel`; `Pass` → `Up`/`Down` move; `Consumed` | `Submit`/`Cancel`/`Consumed` widget-owned, stay; `Pass` → resolver | `F1` now opens help (was swallowed) |
| `:644-657` | area: `Cancel`; else `Stay` | `Cancel`, `Consumed` stay; `Pass` (and the unreachable `Submit`) → resolver | |
| `:273-282` | `pub(super) fn ctrl_s`, `ctrl_e` | deleted once `compose.rs`, `documents.rs`, `divergence.rs` stop calling them (§2.5-§2.6) | D8 |
| `:685-687` (`on_view_key`) | `ctrl_e` → `Stay` | removed: `ctrl-e` is no act in `ITEM_DIVERGENCE`, so it passes (no visible effect) | main §1 item 7 |
| `:690` | `ViewOutcome::Pass` → `ItemFormOutcome::Pass` | unchanged | |

`on_item_form_key` (`mod.rs:513`): `form.on_key(key, ctx.keys())`; `ItemFormOutcome::Pass =>
return Handled::Pass` stays (the form already applied the pass rule). `render` (`:1001`) gains
`keys: &Keys` (§3); `mod.rs:945` passes `ctx.keys()`. Module doc bullets (`:8-24`) naming
`Ctrl+S`/`Ctrl+E` stay (M6).

### 2.5 Divergence view (`divergence.rs`)

`Divergence::on_key(&mut self, key: KeyEvent, keys: &Keys) -> ViewOutcome` over
`ITEM_DIVERGENCE`; first candidate the view accepts, else `stack.passes(chord)` → `Pass`, else
`Stay`:

| file:line | today | → act | notes |
|---|---|---|---|
| `:238-240` | `ctrl_s` → `Stay` | removed (`ctrl-s` passes; nothing binds it there) | main §1 item 7 |
| `:241-243` | `CHORD` → `Pass` | removed (resolver + pass rule) | |
| `:245` | `Esc` → `Back` | `Back` | |
| `:247` | `t`/`T` → theirs | `ItemTheirs` | |
| `:248` | `m`/`M` → mine | `ItemMine` | |
| `:249-258` | `Tab`/`BackTab` (both texts) → switch diffs | `ItemSwitchDiff` → switch when `both_texts()`, `Stay` either way (today: swallowed when not) | |
| `:259` | `j`/`Down` → `scroll_by(1, true)` | `ListDown` | |
| `:260` | `k`/`Up` | `ListUp` | |
| `:261` | `PgDn` → `scroll_by(PAGE, true)` | `PanePageDown` | |
| `:262` | `PgUp` | `PanePageUp` | |
| `:263` | `_ → Stay` | not passed → `Stay` | |

`use super::item_form::ctrl_s` (`:31`) and `use crate::ui::tabs::backlog::filter` (`:34`) go.

### 2.6 Compose area (`compose.rs`), Notes (`notes.rs`), Docs (`documents.rs`)

`Compose::on_key(&mut self, key: KeyEvent, keys: &Keys) -> ComposeOutcome` over `self.stack()`:

| file:line | today | → | notes |
|---|---|---|---|
| `:319-324` | `ctrl_s` → save (busy `Stay`) | after the widget: `FormSave` → busy: `Stay`; `self.unchanged()` → `settle(Some(NOTHING_TO_SAVE))`, `Stay` (moved from `documents.rs:193-196`; `unchanged()` is false for a note); else `save()` | D8; the docs rule keeps its message and its `Stay` |
| `:326-332` | `ctrl_e` → body: `hand_off`, else `Stay` | after the widget: `FormExternalEditor` → busy or not on `Part::Body`: `Stay`; else `hand_off()` | |
| `:333-335` | `CHORD` → `Pass` | removed (widgets pass chords; pass rule) | |
| `:336-338` | busy → `Stay` | busy as the item form (§2.4) | |
| `:340-347` | `Tab`/`BackTab` → move focus | after the widget: `FormNextField`/`FormPrevField` (`DOCS_COMPOSE` only; in `NOTES_COMPOSE` `Tab` is no act and `modal_rest` swallows it, as today's one-part `move_focus` did) | |
| `:350-366` | field `Submit` → next part; `Cancel`; `Pass` → `Up`/`Down` | `Submit`/`Cancel`/`Consumed` stay; `Pass` → resolver (`down`/`up` via `BacklogDocs` `VIEW_DEFAULTS`) | |
| `:367-371` | body: `Cancel`; else `Stay` | `Cancel`/`Consumed` stay; `Pass` → resolver | |
| `:34-35` | `use … filter::CHORD`, `item_form::{HINT_AREA, HINT_TEXT, PAGE, ctrl_e, ctrl_s, marker}` | `item_form::{HINT_AREA, HINT_TEXT, PAGE, marker}` (the two hints are now `HintSpec`s) | |

`notes.rs`:
| file:line | today | → | notes |
|---|---|---|---|
| `:179-201` `on_compose_key` | `compose.on_key(key)` | `compose.on_key(key, ctx.keys())`; arms unchanged | |
| `:296` | notice cleared on every key the pane gets | unchanged | it now gets the tab's keys too (main §1 item 1) |
| `:297-302` | CONTROL/ALT → `Pass` | removed | |
| `:303-311` | `a` → `NoteForm` (no item: `Pass`) | `NotesAdd`; no item → `continue` | |
| `:312-313` | `scroll.on_key(key, rows)` | `PaneScrollDown`/`Up`, `PanePageDown`/`Up` → `return self.scroll.apply(act, rows)` | `Scroll::on_key` loses this caller |
| — | `key_stack` | §1 | |

`documents.rs`:
| file:line | today | → | notes |
|---|---|---|---|
| `:190-197` `on_compose_key` | `ctrl_s` + unchanged → `NOTHING_TO_SAVE` before the area | moved into `Compose` (above); `compose.on_key(key, ctx.keys())` | |
| `:449` | notice cleared | unchanged | |
| `:450-455` | CONTROL/ALT → `Pass` | removed | |
| `:457-459` | no item → `Pass` | unchanged (before resolving) | |
| `:461` | `J` → cursor +1 | `DocsNextRow` | |
| `:465` | `K` → cursor −1 | `DocsPrevRow` | |
| `:469` | `a` → form | `DocsAdd` | |
| `:473-478` | `v` → form for the cursor's kind (`Consumed` with no row) | `DocsNewVersion` | |
| `:479` | `scroll.on_key` | `PanePageDown`/`Up` → `self.scroll.apply(act, len)` | `PgDn`/`PgUp` only, as today (`J`/`K` were taken) |
| `:61` | `use … item_form::ctrl_s` | deleted | |

Once §2.4-§2.6 land, `item_form::{ctrl_s, ctrl_e}` and `filter::CHORD` have no caller: delete
them in the commit that removes the last one (`relations usages` to confirm).

## 3. Hints (9 constants + 1 picker), every one a `HintSpec`

Each renders with `keys.hint(<stack>, SPEC)`; `filter::render`, `item_form::render`,
`divergence::render` and `compose::render` gain `keys: &Keys` before `theme`; `Divergence::hint`
becomes `hint(&self, keys: &Keys) -> String`.

| file:line | constant, today's text | stack | `HintSpec` | rendered with defaults | differs? |
|---|---|---|---|---|---|
| `filter.rs:27` | `HINT_CHOICES` `space toggle · x clear · Enter apply · Esc cancel` | `BACKLOG_FILTER` | `Text("space toggle")`, `One(FilterClear, "clear")`, `Text("Enter apply")`, `Text("Esc cancel")` | `space toggle · x clear · Enter apply · Esc cancel` | no |
| `filter.rs:30` | `HINT_TAGS` `comma list · ↑/↓ row · Enter apply · Esc cancel` | `BACKLOG_FILTER` | `Text("comma list")`, `One(FormNextField, "row")`, `Text("Enter apply")`, `Text("Esc cancel")` | `comma list · Tab row · Enter apply · Esc cancel` (47 cells in 53) | **yes**: `↑/↓ row` → `Tab row` (main §1 item 5) |
| `item_form.rs:56` | `HINT_TEXT` `Tab field  Ctrl+S save  Esc cancel` | `ITEM_FORM` / `DOCS_COMPOSE` | `One(FormNextField, "field")`, `One(FormSave, "save")`, `Text("Esc cancel")` | `Tab field · Ctrl+s save · Esc cancel` | D9 only (separator, `Ctrl+s`) |
| `item_form.rs:59` | `HINT_PICK` `←/→ choose  Tab field  Esc cancel` | `ITEM_FORM` | `Text("←/→ choose")`, `One(FormNextField, "field")`, `Text("Esc cancel")` | `←/→ choose · Tab field · Esc cancel` | D9 only (separator) |
| `item_form.rs:63` | `HINT_AREA` `Ctrl+E $EDITOR  Ctrl+S save  Esc cancel` | `ITEM_FORM` / `NOTES_COMPOSE` / `DOCS_COMPOSE` | `One(FormExternalEditor, "$EDITOR")`, `One(FormSave, "save")`, `Text("Esc cancel")` | `Ctrl+e $EDITOR · Ctrl+s save · Esc cancel` (41 in 43) | D9 only |
| `divergence.rs:39` | `HINT_SIDES` `t theirs wins  m mine wins` | `ITEM_DIVERGENCE` | `One(ItemTheirs, "theirs wins")`, `One(ItemMine, "mine wins")` | `t theirs wins · m mine wins` | D9 only |
| `divergence.rs:42` | `HINT_NO_CONFLICT` `no conflicts — m or t continues` | — | stays `&str`, used as `Text` | identical | no (key prose, M6: D11) |
| `divergence.rs:45` | `HINT_TAB` `Tab body/paths` | `ITEM_DIVERGENCE` | `One(ItemSwitchDiff, "body/paths")` | `Tab body/paths` | no |
| `divergence.rs:48` | `HINT_BACK` `Esc back` | `ITEM_DIVERGENCE` | `One(Back, "back")` | `Esc back` | no |
| `divergence.rs:308-319` | `hint()` joins with two spaces | — | the rendered parts, empty ones dropped, joined `" · "` | conflict + both texts: `t theirs wins · m mine wins · Tab body/paths · Esc back` | D9 only (separator) |
| `notes.rs:55` | `HINT` `a add note` | `BACKLOG_NOTES` | `One(NotesAdd, "add note")` | `a add note` | no |
| `documents.rs:72` | `HINT` `J/K move · a new · v new version` | `BACKLOG_DOCS` | `Pair(DocsNextRow, DocsPrevRow, "move")`, `One(DocsAdd, "new")`, `One(DocsNewVersion, "new version")` | identical | no |
| `compose.rs:626-629` | `HINT_TEXT` on kind/title, `HINT_AREA` on the body | `compose.stack()` | the item form's specs | as above | D9 only |
| `item_form.rs:1095-1099` | per focus: pick/text/area | `ITEM_FORM` | as above | as above | D9 only |

Every user-visible difference in L-A's hints: (1) `Tab row` for `↑/↓ row` on the filter's Tags
row; (2) `Ctrl+S`/`Ctrl+E` → `Ctrl+s`/`Ctrl+e` and two-space joins → ` · ` in the item form, the
compose areas and the divergence view. Nothing else. Prose naming keys stays (M6, D11):
`still_behind` / `rebased_on` (`divergence.rs:70`, `:76`), the item form and compose module docs,
`compose::{may_have_landed, not_written, could_not_check}`, `NO_KINDS`.

## 4. D7/D8/D12 in L-A

- **D7**: the Backlog's CONTROL/ALT guard (`mod.rs:746-752`), the forms' `CHORD` guards and the
  Notes/Docs guards go; modal stacks pass what `MODAL` admits. Pinned (§6).
- **D8**: `form.save` and `form.external_editor` resolve after the widget in the item form, the
  Notes area and the Docs area; `ctrl_s`/`ctrl_e`/`CHORD` deleted; `Tab`/`BackTab` are
  `form.next_field`/`prev_field` in the item form, the Docs area and the filter form.
- **D6**: the miss is `BacklogReplay`, emitted by the tab when the fold and the pane declined.
  Under the defaults its text is unchanged, so `tests/backlog.rs:1362` and the status half of
  `tests/replay.rs:217` stay green unchanged.

## 5. In-file tests that change

- `backlog/mod.rs:1507` `a_capturing_sub_tab_gets_the_navigation_keys`: the non-capturing probe
  is now offered every key first and passes it; keep the cursor assertions, change the `seen`
  expectation to "offered and passed" (it now records the navigation keys while not capturing).
  The probes (`:1489`, `:1937`, `:4016`, `:4041`) need no `key_stack` (trait default).
- `backlog/mod.rs` item-form and filter tests that call `tab.on_key` keep their keys; any that
  assert an `alt-x`/`ctrl-x` is swallowed by the filter form now see `Pass` (`:2481`
  `an_alt_chord_passes_through_the_open_form` stays green).
- `filter.rs` tests: `on_key(key)` → `on_key(key, Keys::compiled())`; the `CHORD` test → a chord
  answers `FormOutcome::Pass` and leaves the draft; `:903` (`for hint in [HINT_CHOICES,
  HINT_TAGS]`, a width check) renders each through `Keys::compiled().hint(views::BACKLOG_FILTER,
  …)`.
- `item_form.rs` tests: `on_key(key, Keys::compiled())`; `ctrl_s`/`ctrl_e` helper tests go with
  the helpers; picker `Down`/`Up` still move (now through `VIEW_DEFAULTS`).
- `divergence.rs:1088-1113`: `hint(Keys::compiled())`; joins `" · "`; `HINT_SIDES`/`HINT_TAB`/
  `HINT_BACK` are specs, so compare against their rendered text.
- `compose.rs:1093`, `:1106`, `:1202` (`text.contains(HINT_AREA|HINT_TEXT)`): compare against
  the rendered spec; the `ctrl_s_*`/`ctrl_e_*` tests stay (default chords).
- `notes.rs:1108-1129`, `documents.rs` hint tests: `HINT` is a spec; compare against
  `Keys::compiled().hint(views::BACKLOG_NOTES|BACKLOG_DOCS, HINT)`.

## 6. Tests (L-A; write first, red, then code)

`tests/backlog_keys/l_a.rs` (App `Harness`, demo; `register_all` where a shell action matters):

| Test | Asserts |
|---|---|
| `ctrl_l_alt_l_and_alt_j_do_not_move_the_backlog` (D12) | on the list with Body active: `ctrl-l`, `alt-l`, `alt-]`, `alt-j`, `ctrl-j` leave the selected row and the active sub-tab; `l` and `j` still move them |
| `a_rebound_save_saves_the_item_form_a_note_and_a_document` (D8, D12) | `[form]\nsave = "f2"\n`: `e`, a changed title, `F2` sends one `EditItem`; Notes `a`, text, `F2` sends `AddNote`; Docs `a`, kind/title/body, `F2` sends `WriteDocument`; in each, `ctrl-s` first sends nothing, types nothing and leaves the area open; each hint reads `F2 save` |
| `a_rebound_external_editor_hands_the_body_out` (D12) | `[form]\nexternal_editor = "f3"\n`: item form Body `F3` emits `Action::EditExternally`; Notes area `F3` too; `ctrl-e` emits nothing |
| `the_replay_miss_names_a_rebound_step_key` (D6, D12) | `[backlog.runs]\nnext_step = "n"\n`, Body on an item, `Enter` → status `select a step in the Runs pane (n/K) to replay it` |
| `enter_on_a_header_still_folds_and_on_body_misses` (D6) | header: `Enter` folds, no status; item on Body: the default miss text |
| `the_filter_form_keeps_its_value_keys_and_x_clears` | option rows: `l`/`Right` move the option, Space ticks, `alt-l` and `alt-x` change nothing, `x` clears, `j`/`Down`/`Tab` move the row; Tags row: `j` and `x` are typed, `Down`/`Tab` move on |
| `f1_opens_help_from_the_item_form_and_the_form_stays` (§1 item 3) | `N`, `f1`: the box has a `Form:` line and `F1 closes this box`; `f1` again; the form is open with its text |
| `a_capturing_backlog_form_shows_the_filtered_status_line` (D9) | item form open: status `Ctrl+c quit · F1 help` (bare); filter form open: same |
| `the_backlog_help_box_lists_the_tab_and_the_list` (D12, PA-3) | Body active, `?`: `Pane:` with `J scroll down`, `Backlog: Enter replay step · m open graph · f filter · F clear filter · N new item · Q queue / dequeue · P pause / resume queue`, `Common: e edit`, `List: … Enter fold`, `Global:`, `?/F1 closes this box` |
| `a_rebound_new_version_key_opens_the_form_and_v_is_inert` (the lane's rebinding test) | `[backlog.docs]\nnew_version = "V"\n`, Docs on `FEAT-1`'s `plan` row: `v` sends no `DocumentForm`; `V` sends `DocumentForm { kind: Some("plan") }`; the Docs hint reads `J/K move · a new · V new version` |
| `a_backlog_table_moves_the_filter_key` | `[backlog]\nfilter = "/"\n`: `/` opens the form, `f` does not |

Flagged edits (PA-3), each now `harness.key("?")` and the frame:
`tests/backlog.rs:268` `m_is_on_the_backlog_help_line` (`m open graph`, `Enter replay step`),
`:1810` (`f filter`, `F clear filter`), `:2392` (`N new item`, `Common: e edit` — was `e edit
item`), `:3398` (`Q queue / dequeue`, `P pause / resume queue`); `tests/replay.rs:217-226` (the
help-line half: the `?` box frame contains `Enter replay step`; the status assertion unchanged);
`tests/backlog.rs:2633` (the divergence hint: `t theirs wins · m mine wins · Esc back`). The
`use htui::keymap::KeyScope` import (`tests/backlog.rs:15`) goes if unused.

## 7. Snapshots (L-A's 15)

On this branch `BacklogTab::key_stack` is live, so a capturing L-A mode shows the filtered status
line here.

| Snapshot | Rows that move | Expected |
|---|---|---|
| `backlog__filter_form` | Tags-row hint; status | `comma list · Tab row · Enter apply · Esc cancel`; `Ctrl+c quit · F1 help` |
| `backlog__item_form_new`, `backlog__item_form_edit` | hint; status | `Tab field · Ctrl+s save · Esc cancel`; `Ctrl+c quit · F1 help` |
| `backlog__item_divergence` | hint; status | `t theirs wins · m mine wins · Esc back`; `Ctrl+c quit · F1 help` |
| `backlog__document_form`, `backlog__note_compose` | hint; status | `Ctrl+e $EDITOR · Ctrl+s save · Esc cancel`; `Ctrl+c quit · F1 help` |
| `backlog__list_{folded,grouped,last_row}`, `backlog__filter_no_match`, `backlog__filtered_list`, `backlog__detail_{notes,documents}`, `backlog__empty_{notes,documents}` | none | byte-identical (the list hint row is T-close's) |

No app-level snapshot moves on this branch (the Backlog's browse status line equals
`Stack::BASE`'s; the overlays own the status line in `shell__`/`concepts_search__`/`waiting__`).

## 8. Cross-lane couplings

- `BacklogTab::key_stack` reports `self.detail.key_stack()`: until L-B/L-C merge, their panes
  report `BACKLOG_BODY`, so on this branch the `?` box on Runs shows Body's stack (transitional;
  the merged-tree test is T-close's).
- `replay_miss` reads `views::RUNS_BROWSE` (T1), not L-B's code.
- `tests/backlog.rs` is shared with L-B/L-C: L-A's edits are the six of §6, flagged.
- `Scroll::on_key` keeps L-B's and L-C's callers; T-close deletes it.
- L-B's and L-C's rebinding chords (`n`, `C`) pass this lane's tab too (no `backlog`/`list`/
  `pane` act binds them), so their tests hold on the merged tree.

## 9. Work order, commits and gate (serial inside the lane)

1. **C1** `feat(mod-67): the Backlog tab dispatches through the pane's stack (D5, D6)` —
   `BacklogTab::{on_key, key_stack}` (§2.1, `key_stack` with `form.resolving()` inline until C3),
   `replay_miss`, `app/mod.rs` rows (§2.2), PA-3's six flagged edits, the tab pins of §6
   (`ctrl_l_…`, `the_replay_miss_…`, `enter_on_a_header_…`, `the_backlog_help_box_…`,
   `a_backlog_table_moves_the_filter_key`), §5's `mod.rs:1507` change. Accept the status rows of
   the six capturing snapshots.
2. **C2** `feat(mod-67): the Backlog filter form on its stack` — §2.3, filter hints, filter
   tests; accept `filter_form`'s hint row.
3. **C3** `feat(mod-67): the item form and its divergence view on their stacks (D8)` — §2.4,
   §2.5, `ItemForm::key_stack`, hints; `f1_opens_help_from_the_item_form…`,
   `a_capturing_backlog_form_shows_the_filtered_status_line`; accept `item_form_*`,
   `item_divergence`.
4. **C4** `feat(mod-67): Notes, Docs and their compose areas on their stacks (D8)` — §2.6, the
   Notes/Docs `key_stack`s, hints; delete `ctrl_s`, `ctrl_e`, `CHORD`; `a_rebound_save_…`,
   `a_rebound_external_editor_…`; accept `document_form`, `note_compose`.
5. **C5** `test(mod-67): L-A rebinding test` — `a_rebound_new_version_key_…`,
   `the_filter_form_keeps_its_value_keys_and_x_clears`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/backlog/{mod,filter,item_form,divergence}.rs \
  crates/htui/src/ui/tabs/backlog/detail/{compose,notes,documents}.rs crates/htui/src/app/mod.rs \
  crates/htui/tests/backlog_keys/l_a.rs crates/htui/tests/backlog.rs crates/htui/tests/replay.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::backlog app
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features \
  --test backlog --test backlog_keys --test replay --test reveal --test keys -- --test-threads=1
cargo insta test -p htui --all-features --check
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```
(`tests/keys.rs` is not edited but drives the Backlog: `f1_opens_and_closes_help_on_the_backlog`,
`the_status_line_is_todays_in_both_harnesses`, `the_help_box_lists_ctrl_w_waiting_only_with_
register_all` must stay green.)
