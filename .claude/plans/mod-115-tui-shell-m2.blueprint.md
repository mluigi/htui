# MOD-115 M2 blueprint (code-architect)

Companion to `mod-115-tui-shell-m2.plan.md` (CONFIRMED 2026-10-18). Where they differ, this file wins: it is the
result of reading the code. Every `PLAN DEVIATION` is listed at the end with its evidence. Paths are under
`crates/htui/src/` unless they start with `crates/` or `docs/`.

## Conventions (unchanged from M1)
`//!` module doc; pure geometry `#[must_use]`; styles from `Theme` roles only, each with a monochrome test; tests
in-file, `TestBackend` read back as strings, sentence-named; labels arrive pre-resolved from `Keys`; glyph widths
via `cells::cell_width`. Every new row cites the arm it mirrors. Gate serially (`--test-threads=1`), featureless
clippy too (`cargo clippy --workspace -- -D warnings`), `--all-features` for `tests/*.rs` (testkit).

## Decisions taken here (settle what the plan left open)
- **B1. Layout and history are their own contexts, not `[global]`.** `[layout]` sits only in the list + detail
  browse stacks; `[history]` in every tab browse stack (the stacks that end in the unfiltered `GLOBAL`). Neither is
  in `views::MODAL`, so no layout or history key fires while a field captures (alt-left, ctrl-up, f2 all pass the
  modal filter, `keys/chord.rs:53-58`, and would otherwise act inside a form). Both are neither `is_view` nor
  `is_shared` (like `[global]`/`[editor]`): `history.back` would clash with the shared `common.back` name
  (`shared_names_are_unique_across_shared_contexts`, `catalogue.rs:1983`).
- **B2. `global.next_pane`/`prev_pane` stay `[global]` (plan D4) but `MODAL` stops admitting them.**
  `views.rs:29` becomes `Layer::only(Context::Global, MODAL_GLOBALS).with_modal_filter()`, `MODAL_GLOBALS` = every
  global act but `NextPane`/`PrevPane`; otherwise `ctrl-right` (word-right in a field) moves the pane mid-typing.
  `Stack::passes` is unchanged (it reads the layer's chord filter only).
- **B3. "The stack captures text" = the stack has a `[field]` layer.** `Stack::captures_text(self) -> bool`
  (stack.rs, any layer of `Context::Field`). Today capture is only implied (`MODAL` + `captures_input()` on sections
  and detail tabs); the `[field]` layer makes it declared, validated and visible to the shell.
- **B4. ctrl-c reaches every screen through a quit layer, not a pre-dispatch branch.** `passes_modal` needs no change
  (ctrl-c already passes). What is missing: overlay stacks have no global quit (`Stack::OVERLAY` and the list
  overlays end in `HELP` = `global ∩ {help}`, `stack.rs` ~186, `views.rs:53`), so after the hardcoded branch
  (`app/state.rs:765-770`) goes, ctrl-c would die in the modal swallow. New
  `const QUIT: Layer = Layer::only(Context::Global, &[Act::Quit]).with_modal_filter();` placed **just before** the
  final `HELP` layer (so `Stack::global()` still returns `HELP` and `passes` is unchanged) in `Stack::OVERLAY`,
  `CONCEPTS_QUERY`, `SWITCHER`, `MIGRATION`, `WAITING_LIST`, `QUEUE_OVERLAY`, and as the only extra layer of
  `ABORT_CONFIRM`. The leave dialog's own stack has no quit layer: its ctrl-c is `leave.confirm`.
- **B5. Panes are claimed, not imposed.** No `Tab::render_panes`. A list + detail tab keeps `Tab::render(frame,
  area, ctx)` and calls `ctx.split(region)` where it called `Layout::horizontal` today; the shell computes the
  areas from the tab's `TabLayout` and records the claim (a `Cell`, like `claim_editor_area`) for focus borders, the
  hidden note, the activity placeholder, hints and mouse hits. Reason: Requirements draws a notice and a filter row
  under its panes (`requirements/mod.rs:1275-1307`), Skills a switch row above and notices below
  (`skills/mod.rs:177-199`, `library.rs:748-781`), and editor/report/divergence modes are full width
  (`backlog/mod.rs:1024-1027`); a shell `PaneAreas` over the whole body cannot express any of them.
- **B6. Default-meaning flips land with their wiring (Task 6), not in Task 0.** `next_tab`/`prev_tab` →
  `ctrl-pgdn`/`ctrl-pgup`, `next_pane`/`prev_pane` gaining `tab`/`backtab`, and dropping plain `left`/`right` from
  `pane.*_subtab`, `settings.*_section`, `skills.switch_view` cannot be inert: `tab` on both `next_tab` and
  `next_pane` is a `[global]` collision, and `tab` mapped to an act the shell does not handle yet would make Tab dead
  between Task 0 and Task 6. Task 0 adds the rows and every *non-colliding* new default (`ctrl-right`/`ctrl-left` on
  the pane rows, the shift arrows on the sub-tab rows, `ctrl-c` on quit); Task 6 flips the rest in one commit with
  the dispatch.
- **B7. Ctx defaults keep every bench behaving as today.** `Ctx::focus()` returns `PaneId::List` unless the shell set
  one; `ctx.split` with no shell layout uses the tab's own default `TabLayout`; `open_edit()` is `None`.

## 1. Key catalogue (Task 0) — `keys/catalogue.rs`, `keys/views.rs`, `keys/stack.rs`
### 1.1 `since` tag
`ActionSpec` (`catalogue.rs:731-745`) gains `pub since: u8`. `row()`/`capture_row()` set `since: 1`; add
`const fn since(self, version: u8) -> Self { Self { since: version, ..self } }`. Tag `.since(2)` on every row added
in M2 **and** on the changed rows: `quit`, `next_tab`, `prev_tab`, `pane.next_subtab`, `pane.prev_subtab`,
`settings.next_section`, `settings.prev_section`, `skills.switch_view`. Test: `every_m2_row_is_since_2` (an explicit
list) and `since_is_1_or_2`.

### 1.2 New contexts (appended to `Context` and `Context::ALL` after `Picker`, so every existing table and
`--print-keys` block keeps its place; `context_position` 42..=47, `every_context_is_listed_once_in_all` → 48)
| Context | table | heading | is_view | is_shared |
|---|---|---|---|---|
| `Layout` | `layout` | `Layout` | no | no |
| `History` | `history` | `History` | no | no |
| `Leave` | `leave` | `Leave htui?` | **yes** (an overlay's view, like `Waiting`) | no |
| `Field` | `field` | `Text field` | no | no |
| `Choice` | `choice` | `Choice` | no | no |
| `Answer` | `answer` | `Answer` | no | no |
Justification against existing contexts: `[confirm]` cannot host leave (`y`/`n`, and the dialog must not change when a
user rebinds every yes/no question); `[form]` is field *movement*, not editing; `[list]`'s `up`/`down` are motion in a
list. `[answer]` is separate from `[chat]` because Runs (`RUNS_BROWSE`) answers with the same digits.

### 1.3 New `Act`s (appended at the end of the enum; the test `ALL`/`position` lists in `catalogue.rs:2000-2345` grow)
`NextPane, PrevPane` (Global); `LayoutNextPreset, LayoutPrevPreset, LayoutGrowList, LayoutShrinkList,
LayoutGrowActivity, LayoutShrinkActivity, LayoutToggleList, LayoutToggleActivity, LayoutZoom`; `HistoryBack,
HistoryForward`; `LeaveConfirm, LeaveStay`; `FieldSubmit, FieldCancel, FieldLeft, FieldRight, FieldHome, FieldEnd,
FieldDeleteBack, FieldDeleteForward, FieldLineUp, FieldLineDown, FieldPageUp, FieldPageDown` (the last four: see
deviation D-8); `ChoicePrev, ChoiceNext, ChoiceToggle`; `AnswerOption1..AnswerOption9` (+ `Act::answer_index()`,
`tab_index`'s twin); `RequirementsPriorityMust, RequirementsPriorityLater`.

### 1.4 Rows (all `.since(2)`)
Global block: `next_pane`/`prev_pane` go **at the end of the global block, after `queue`** (`the_global_block_is_in_status_line_order`
`catalogue.rs:2461` gets the two names appended; the `keys_line` strings the tests pin keep their prefix).
| Row | Task 0 defaults | Task 6 defaults | kind |
|---|---|---|---|
| `global.quit` | `q`, `ctrl-c` | same | row |
| `global.next_tab` / `prev_tab` | `tab` / `backtab` (unchanged) | `ctrl-pgdn` / `ctrl-pgup` | row |
| `global.next_pane` / `prev_pane` (label `next pane`/`previous pane`) | `ctrl-right` / `ctrl-left` | `tab`, `ctrl-right` / `backtab`, `ctrl-left` | row |
| `pane.next_subtab` / `prev_subtab` | `l`, `]`, `right`, `shift-right` / `h`, `[`, `left`, `shift-left` | `shift-right`, `l`, `]` / `shift-left`, `h`, `[` | row |
| `settings.next_section` / `prev_section` | same shape as `pane.*` | same shape | row |
| `skills.switch_view` | `h`, `l`, `[`, `]`, `left`, `right`, `shift-left`, `shift-right` | `shift-left`, `shift-right`, `h`, `l`, `[`, `]` | row |
| `layout.next_preset` / `prev_preset` | `p` / `P` | same | row |
| `layout.grow_list` / `shrink_list` | `>` / `<` | | row |
| `layout.grow_activity` / `shrink_activity` | `}`, `ctrl-down` / `{`, `ctrl-up` | | row |
| `layout.toggle_list` / `toggle_activity` | `f2`, `alt-1` / `f3`, `alt-2` | | row |
| `layout.zoom` | `z` | | row |
| `history.back` / `forward` | `backspace`, `alt-left` / `alt-right` | | row |
| `leave.confirm` / `stay` | `enter`, `ctrl-c` / `esc`, `n` | | row |
| `field.submit`, `cancel`, `left`, `right`, `home`, `end`, `delete_back`, `delete_forward`, `line_up`, `line_down`, `page_up`, `page_down` | `enter`, `esc`, `left`, `right`, `home`, `end`, `backspace`, `delete`, `up`, `down`, `pgup`, `pgdn` | | **capture_row** (all non-printable) |
| `choice.prev` / `next` / `toggle` | `left`, `h` / `right`, `l` / `space` | | row (printable: view-resolved) |
| `answer.option_1`..`option_9` | `1`..`9` | | row |
| `requirements.priority_must` / `priority_later` | `m` / `l` | | row |
Tests to update in Task 0: `no_default_spells_ctrl_c` (`catalogue.rs:2419`) becomes `only_quit_and_leave_confirm_default_to_ctrl_c`;
`the_compiled_keys_are_the_catalogue_and_never_ctrl_c` (`keys/mod.rs:~430`) likewise; `ctrl_c_is_no_action_in_either_stack`
(`stack.rs:642`) becomes `ctrl_c_is_quit_on_every_screen_but_the_focused_editor` (BASE → `[Quit]`, OVERLAY → `[Quit]`,
EDITOR_FOCUSED → `[]`); `in_capture_acts_are_…` (`catalogue.rs:2631`) gains the `[field]` rows.

### 1.5 Layers and stacks (`keys/views.rs`)
New layer consts: `FIELD = Layer::all(Context::Field)`; `CHOICE = Layer::all(Context::Choice)`; `TOGGLE =
Layer::only(Context::Choice, &[Act::ChoiceToggle])`; `ANSWER = Layer::all(Context::Answer)`; `LAYOUT =
Layer::all(Context::Layout)`; `HISTORY = Layer::all(Context::History)`; `QUIT` (B4); `MODAL` narrowed (B2).
Layer order rule (module doc): view, **`field`**, **`choice`**, **`answer`**, then the existing order, then
**`layout`**, **`history`** right before `GLOBAL`; `QUIT` right before the last `HELP`.
- `FIELD` after the view layer in every capturing stack: `CAPTURE`, `AGENTS_FORM`, `HIERARCHY_EDITOR`, `KINDS_EDITOR`,
  `BOXES_EDITOR`, `PERSONAS_FORM`, `PERSONAS_EDITOR`, `SECRETS_FORM`, `CONCEPTS_QUERY`, `LIBRARY_PROMPT`,
  `LIBRARY_INFO`, `LIBRARY_EDITOR`, `ATTACH_FORM`, `TEMPLATES_PROMPT`, `TEMPLATES_EDITOR`, `HELP_ASKING`,
  `REQUIREMENTS_FORM`, `REQUIREMENTS_WITHDRAW`, `DOCS_COMPOSE`, `NOTES_COMPOSE`, `BACKLOG_FILTER`, `ITEM_FORM`.
  Implementer check (part of Task 0's tests): every mode whose view feeds a `TextField`/`TextArea`/`Composer` returns
  one of these stacks — grep `TextField::`/`TextArea::` per mode; `REQS_PICK` and `HIERARCHY_PICKER` have no field
  (the picker's go-to field is `CAPTURE`, `views.rs:843`).
- `CHOICE` after `FIELD`: `BACKLOG_FILTER`, `ITEM_FORM`, `REQUIREMENTS_FORM`, `RUNS_CONFIRM` (the close-out warning's
  resolution, `runs.rs:1125-1129`; no field there: the typed stage is `CAPTURE`, `runs.rs:693-707`). `TOGGLE` in
  `ATTACH_FORM`. `REQUIREMENTS_FORM`'s view layer own list gains `RequirementsPriorityMust/Later`.
- `ANSWER` after the view layer: `RUNS_BROWSE`, `CHAT_LIVE`.
- `LAYOUT` + `HISTORY`: `LIBRARY_BROWSE`, `TEMPLATES_BROWSE`, `REQUIREMENTS_BROWSE`, `BACKLOG_BODY`, `RUNS_BROWSE`,
  `BACKLOG_GRAPH`, `BACKLOG_DOCS`, `BACKLOG_NOTES`, `BACKLOG_PROMPT`, `BACKLOG_REQS`.
- `HISTORY` only (single-pane or no panes claimed): `SETTINGS_TAB` and every `*_BROWSE` Settings stack plus
  `AGENTS_CONSENT`, `AGENTS_CHOOSER` (they end in `GLOBAL`), `LIBRARY_REPORT`, `ATTACH_BROWSE`, `CHAT_LIVE`,
  `CHAT_REPLAY`. Not `Stack::BASE` (test doubles; `stack.rs` tests pin `base(keys, "z") == []`).
- New `pub const LEAVE: Stack<'static> = Stack::new(&[Layer::view(Context::Leave, &[Act::LeaveConfirm,
  Act::LeaveStay]), HELP]);`. `QUIT_CONFIRM` stays until Task 6 deletes `quit_confirm.rs`.
`keys/stack.rs` (Task 0, layers only): `Stack::OVERLAY` gains `QUIT` before its help layer; `DECLARED` appends
`("in the leave dialog", views::LEAVE)` (len 80 → 81; Task 6 removes `("in the quit question", QUIT_CONFIRM)` → 80);
add `Stack::captures_text`.

### 1.6 Allow-lists (each demanded by `the_compiled_defaults_validate` + `every_allow_list_entry_is_demanded`)
`SHADOWING` (narrower first) gains:
- `(Act::RunsPromote, Act::LayoutNextPreset)` — `p` in `RUNS_BROWSE`.
- `(Act::PromptPrevTemplate, Act::LayoutNextPreset)` — `p` in `BACKLOG_PROMPT`.
- `(Act::BacklogPauseQueue, Act::LayoutPrevPreset)` — `P` in all seven Backlog browse stacks.
- `(Act::FieldCancel, Act::OverlayClose)` — `esc` in `CONCEPTS_QUERY` (the field passes Cancel, `overlay.close` closes,
  `catalogue.rs:867-869`).
- `(Act::RequirementsPriorityLater, Act::ChoiceNext)` — `l` in `REQUIREMENTS_FORM` (today `l` sets later,
  `forms.rs:326`; on the priority `h` now toggles too, harmless).
- `(Act::ConceptsUp, Act::FieldLineUp)`, `(Act::ConceptsDown, Act::FieldLineDown)` — `up`/`down` in `CONCEPTS_QUERY`
  (`catalogue.rs:1243-1245`; the query is one line, the hit cursor always wins).
- `(Act::SkillsHelpPrevAgent, Act::FieldLineUp)`, `(Act::SkillsHelpNextAgent, Act::FieldLineDown)` — `up`/`down` in
  `HELP_ASKING` (`catalogue.rs:1355-1368`, `agent_help.rs:328`).
Translation (§3) reads these the same way: the first candidate is the view's act, so rule 4 delivers `up`/`down`
unchanged and the view resolves them as today.
`STATE_GUARDED` gains:
- `(Act::FieldLeft, Act::ChoicePrev)`, `(Act::FieldRight, Act::ChoiceNext)` — a form focuses a text field or a choice,
  never both (`BACKLOG_FILTER`, `ITEM_FORM`, `REQUIREMENTS_FORM`).
- `(Act::FieldLineDown, Act::FormNextField)`, `(Act::FieldLineUp, Act::FormPrevField)` — `down`/`up` are
  `VIEW_DEFAULTS` field moves in the Settings/Skills/Backlog forms (`catalogue.rs:1820-1866`); a `TextArea` takes them,
  a `TextField` passes them (`text_field.rs:197`).
- `(Act::AnswerOption1, Act::SelectTab1)` … `(Act::AnswerOption9, Act::SelectTab9)` — the digits answer only while a
  request is parked, else they select a tab (`runs.rs:1669-1675`, `chat/mod.rs:603-611`; the `TAB_SELECT` waiver of
  `FIXED`, `stack.rs:343-353`).
None is needed for `p` in Settings (agents paste, hierarchy primary, boxes probe): no Settings stack has `LAYOUT` (B1).
None is needed for `form.next_field = tab` vs `next_pane = tab`: every form stack ends in `MODAL`, which neither admits
`tab` (not ctrl/alt/F) nor `NextPane` (B2); `item.switch_diff`'s `tab` (`ITEM_DIVERGENCE`) likewise. There is no
existing tab shadowing pair to keep.

### 1.7 Version 2 (Task 0 part)
`keys/load.rs` `top_level` (`:293-297`): accept `version = 1` or `2`, store it in the loader; message
`"version: write version = 2 (version = 1 is a file from before MOD-115)"`. `Keys` (`keys/mod.rs:31-35`) gains
`version: u8` (compiled = 2, a file without `version` = 1), excluded from `PartialEq` by hand. `keys/print.rs:13,116`:
`version = 2`; `print_then_load_round_trips` stays green.

### 1.8 Task 0 module stubs and shell types
`ui/mod.rs`: `pub mod shell; pub mod state_file;`. `ui/overlay/mod.rs`: `pub mod leave; pub use leave::LeaveDialog;`
(stub struct). `keys/mod.rs`: `pub mod translate;`. Empty-with-doc files: `ui/shell/layout.rs`,
`ui/shell/history.rs`, `ui/state_file.rs`, `ui/overlay/leave.rs`, `keys/translate.rs`.
`ui/shell/mod.rs` holds the **shared types** (so Tasks 1-3 never import each other):
```rust
pub mod history; pub mod layout;
pub enum PaneId { List, Detail, Activity }                       // Copy, Eq, Hash, Debug
pub enum Preset { Ide, IdeActivity, Columns, Focus }             // name(): "ide"|"ide_activity"|"columns"|"focus",
                                                                 // from_name, label(): "list+detail"|"list+detail+activity"|"columns"|"detail only"
pub enum ListSize { Percent(u16), Cells(u16) }
pub struct TabLayout { pub preset: Preset, pub list: ListSize, pub activity_pct: u16,
                       pub list_on: bool, pub activity_on: bool, pub zoom: bool }
pub enum Panes { Single, ListDetail(TabLayout /* the tab's default */) }
pub enum HiddenBy { Toggle, Preset, Zoom }
pub struct PaneAreas { pub list: Option<Rect>, pub detail: Rect, pub activity: Option<Rect>,
                       pub list_hidden: Option<HiddenBy>, pub activity_hidden: Option<HiddenBy> }
pub struct TabLoc { pub subtab: Option<&'static str>, pub subtab_title: Option<String>, pub cursor: Option<String> }
```
`TabLoc.cursor` is the opaque part: a string the tab writes (an `ItemId`, a requirement key, a skill id, a row index)
and parses back in `restore`; the shell only stores and compares it. `TabLayout::DEFAULT` = `Ide`, `Percent(50)`,
activity 30, both on, no zoom.
`app/state.rs` (Task 0, `Ctx` only, `:82-100`): fields `focus: Option<PaneId>`, `open_edit: Option<&'a str>`;
builders `with_focus(PaneId)`, `with_open_edit(Option<&'a str>)`; getters `focus() -> PaneId` (default `List`, B7),
`open_edit()`. Task 6 adds the layout/claim fields.

## 2. FIXED removal (Task 5)
`keys/stack.rs:323-426`: delete `Fixed`, `TAB_SELECT`, `DIGITS`, `FIXED` and the two FIXED tests (`:516-553`); drop the
re-export (`keys/mod.rs:25`). `keys/validate.rs`: `validate` → `check(keys, SHADOWING, STATE_GUARDED)`; delete
`Check::fixed` (`:260-310`) and the `said` bookkeeping that only existed for it (keep `said` if `report` still dedups
on it — it does not after the fixed pass goes; remove the field and `self.said.push`). Every site resolves through
`Keys`; each already has `&Keys` or `Ctx` except the Requirements form:
| FIXED entry | Site | Change |
|---|---|---|
| Attachments form (`stack.rs:360-368`) | `skills/attach.rs:266-283` `activation_key(form, key)` | → `activation_key(form, key, keys, stack)`: `keys.actions(stack, chord).contains(&Act::ChoiceToggle)` cycles; `translate::FieldEvent::of(&key)` → `Submit`/`Cancel`; else `Pass`. Caller `:556` passes `ctx.keys()`, `stack`. |
| (not in FIXED) | `skills/library.rs:1384-1390` `key.code == KeyCode::Esc` while saving | `translate::FieldEvent::of(&key) == Some(FieldEvent::Cancel)` |
| (not in FIXED) | `skills/attach.rs:641` `KeyEvent::from(KeyCode::Char(c))` loop | `form.globs.on_paste(&inserted)` (inserts at the cursor, unmasked field grows, `text_field.rs:225-245`) |
| Requirements form (`:369-376`) | `requirements/forms.rs:307-345` `RequirementForm::on_key(key)` | → `on_key(key, keys)`; priority: `actions(REQUIREMENTS_FORM, chord)`: `PriorityMust`/`PriorityLater` set, `ChoicePrev|ChoiceNext|ChoiceToggle` toggle (search **all** candidates: `left` is `[FieldLeft, ChoicePrev]`), `FieldEvent::of == Cancel` cancels. Call site `requirements/mod.rs:551`. |
| Runs digits (`:377-384`) | `backlog/detail/runs.rs:1669-1675` | `ctx.keys().actions(views::RUNS_BROWSE, chord).first().and_then(Act::answer_index)` with a pending request under the cursor; digit char for `PermissionStrip::pick_from` = `char::from_digit(index as u32 + 1, 10)`. No pending request → fall through (the digit selects a tab). |
| Close-out warning (`:385-392`) | `runs.rs:1125-1129` `close_out_key` | `actions(stack, chord)`: `ChoiceNext` → forward, `ChoicePrev` → back; drop `is_plain`. |
| Filter form (`:393-400`) | `backlog/filter.rs:254-258` | non-Tags rows: `FieldEvent::of` Submit → apply, Cancel → cancel; `actions(BACKLOG_FILTER, chord)`: `ChoicePrev/Next` move the option, `ChoiceToggle` ticks; the existing act loop follows. |
| Item form pickers (`:401-408`) | `backlog/item_form.rs:748-762` `on_picker_key(&key)` | → `on_picker_key(&key, keys)` (caller `:635` has `keys`): `ChoicePrev/Next` pick, `FieldEvent::of` Submit → next field, Cancel → cancel. |
| Chat digits (`:409-415`) | `chat/mod.rs:603-611` | `actions(CHAT_LIVE, chord).first().and_then(Act::answer_index)` while parked. |
| Chat replay (`:416-425`) | `chat/mod.rs:296-305` swallow | swallow iff `handled == Pass` and `keys.actions(views::CHAT_LIVE, chord)` has an act whose `spec().context` is `Chat` or `Answer` (the derived set equals today's `1-9 i a enter` plus `esc t g G`, which the replay already consumed). No validator pass needed: a global act on one of those chords already collides in `CHAT_LIVE`. |
`chat/composer.rs` needs **no change**: chat's `on_key` hands every chord to the shell before the composer
(`chat/mod.rs:588-591`), and the composer's `Enter`/`Esc`/`Backspace` are canonical events reached through
translation (§3). `keys::is_plain` (`keys/mod.rs:319`): delete if dead after the sites move (featureless clippy).
`keys/hint.rs`: `status_line` (`:186`) drops the `unwrap_or(CTRL_C)` fallback (omit quit when the stack admits none);
`help_lines` (`:255-261`) and `help_line` (`:284-288`) stop appending `CTRL_C` (it is a real default now);
`chord_list` (`:337-345`) uses a new `KeyChord::plain_char(&self) -> Option<char>` (chord.rs, allow-listed) instead of
`KeyCode::Char`.

**Safety nets** (validate.rs, new pass after the stack pass, plus `load.rs` `must_keep` `:430`): `global.quit`,
`leave.confirm`, `leave.stay`, `field.cancel` join `overlay.close`/`editor.focus`. An entry that empties one is refused
at load (`must_keep`, message names the fix); `give_way` lets one of them give a chord away **only while it keeps
another** (so `[global] help = ["q"]` still loads with a notice: quit keeps `ctrl-c`); the post-merge pass refuses any
that ends with no chord. Messages: quit/leave.confirm: `"{subject}: must keep at least one chord: global.quit opens
the leave dialog and leave.confirm leaves; without both htui cannot be left"`; `field.cancel`: `"…: it is the way out
of a text field"`; `leave.stay`: `"…: it is the way back from the leave dialog"`.

**Loader** (`load.rs`): delete `CTRL_C_REFUSED` (`:64`) and its two arms in `chord_of` (`:241-253`); `chord_of`
becomes `parse_strict` plus the error text. Rewrite the ctrl-c refusal tests (validate.rs `ctrl_c_is_refused_*`,
`tests/keys_file.rs`) into "ctrl-c binds like any chord" + "a file that moves ctrl-c off quit and leave.confirm still
has a way out or is refused".

**Version-2 give-way (D8)** in `validate::give_way` (`:41-69`), after the same-context loop, only when
`keys.version == 1`: for each file entry `E` and each row `D` left at its default with `D.act.spec().since == 2`
(not `editor.focus`/`overlay.close`, must-keep guard as above), for each chord in both: if some `DECLARED` stack has
both `E.act` and `D.act` in `keys.actions(stack, chord)`, `D` loses the chord (`Lost`, the existing notice text).
Also, version 1 only, in `load_str` after the walk: if the file set `[global] quit` and it lacks `ctrl-c`, append
`CTRL_C` (it was implicit in version 1; nothing in a v1 file can hold ctrl-c, the loader refused it). Version 2: the
strict rules (no cross-context give-way).

## 3. `[field]` translation (Task 5 writes it, Task 6 calls it) — `keys/translate.rs`
```rust
pub enum FieldEvent { Submit, Cancel, Left, Right, Home, End, DeleteBack, DeleteForward,
                      LineUp, LineDown, PageUp, PageDown }
impl FieldEvent {
    pub const fn act(self) -> Act;                 // FieldSubmit…
    pub fn from_act(act: Act) -> Option<Self>;
    pub fn canonical(self) -> KeyCode;             // Enter, Esc, Left, …, PageDown (the widgets' literals)
    pub fn of(key: &KeyEvent) -> Option<Self>;     // decodes a *plain* canonical event (no modifier but SHIFT)
}
/// `None`: swallowed. `Some(key)`: what the view sees.
pub fn translate(keys: &Keys, stack: Stack<'_>, key: KeyEvent) -> Option<KeyEvent>;
```
Rules, in order:
1. `!stack.captures_text()` → `Some(key)`.
2. `KeyChord::from_event(key).is_printable()` → `Some(key)` (text; the only exception, MD1).
3. `keys.actions(stack, chord).first()` is a field act `F` → `Some(KeyEvent { code: F.canonical(), modifiers: NONE,
   kind: key.kind, state: key.state })` (`field.cancel = ["ctrl-g"]` makes ctrl-g the widgets' `Esc`).
4. `FieldEvent::of(&key)` is `Some` (a plain canonical key its field row did not claim first): no candidate at all →
   `None` (not delivered as that event); another act claims it → `Some(key)` unchanged, for the view or the shell to
   resolve (documented limitation, recorded in the decision doc: a focused widget still reads such a key literally,
   e.g. `left` kept by `choice.prev` after `field.left` moved off it).
5. Otherwise `Some(key)`.
Call site (Task 6, `App::on_key` `:750`): after `editor_key` (the focused in-pane editor gets raw keys and `ctrl-c`,
`app/pane.rs:375-395`; translation never sees them), `let Some(key) = self.active_stack().map_or(Some(key), |s|
translate(&self.keys, s, key)) else { return };` then recompute `chord`. Sites that need submit/cancel semantics
after translation decode with `FieldEvent::of`, never with `Keys` (the event is canonical by then). Paste is untouched.
ctrl-c is printable? No (CONTROL) → rule 3/5: no field row binds it → passes to the modal layer → `global.quit`.
Tests (translate.rs): each rule; a rebound cancel works and bare `Esc` is swallowed in a form; concepts' `Esc` still
closes with `field.cancel` moved; `ctrl-c` untouched; non-capturing stack untouched.

## 4. ctrl-c (Task 6 for the shell, Task 0/5 for keys)
- Delete `app/state.rs:765-770` and the `CTRL_C` import (`:18`); `on_key`'s doc (`:743-749`) loses the ctrl-c step.
- `app/pane.rs:397-401` (`if chord == CTRL_C { return false; }` in `editor_key`): delete. Unfocused on the asking tab,
  ctrl-c resolves through `Stack::EDITOR_OPEN` → `global.quit` → leave dialog; covered by a modal overlay it takes the
  overlay path (B4 quit layer); focused, it is still written to the editor (`:386-394`), `editor.focus` is the way out.
- `keys/chord.rs:139` `CTRL_C` stays as a value; doc: "the default `global.quit`/`leave.confirm` chord".
- `request_quit` (`app/pane.rs:553-558`): `self.leave_overlay.map_or(Action::Quit, |id| Overlay(Open(id)))` — always the
  dialog (R-TUI-18). `focus_under_the_question` (`:415-422`) reads `leave_overlay`. `App.quit_overlay` → `leave_overlay`.

## 5. UI-state file (Task 1) — `ui/state_file.rs`
`pub const FILE_NAME: &str = "ui-state.toml"; const VERSION: i64 = 1;`
```toml
version = 1
[about]
visible = false
[layout.backlog]            # key = TabId.0
preset = "ide"              # Preset::name
list_percent = 55           # exactly one of list_percent / list_cells
activity_percent = 30
list = true
activity = true
zoom = false
[modal.item_form]           # reserved for R-TUI-12's modal sizes; parsed and written back, no reader in M2
width = 80
height = 24
```
```rust
pub struct UiState { pub about: Option<bool>, pub layouts: BTreeMap<String, TabLayout>,
                     pub modals: BTreeMap<String, (u16, u16)> }      // Default, Clone, PartialEq
pub struct Loaded { pub state: UiState, pub notice: Option<String> }
pub fn parse(text: &str) -> Result<UiState, String>;                 // serde DTOs, unknown keys ignored
pub fn render(state: &UiState) -> String;                            // deterministic order
#[derive(Debug, Clone)]
pub enum UiStore { File(PathBuf), Memory(Arc<Mutex<Option<String>>>), Nowhere }
impl UiStore { pub fn memory() -> Self; pub fn load(&self) -> Loaded; pub fn save(&self, s: &UiState) -> Result<(), String>; }
```
Rules: missing file → default state, no notice (first launch: about on). Unreadable, not TOML, wrong `version`, an
unknown preset or both/neither list sizes → default state + notice `"ui-state.toml: <reason>; using the default
layout (the next change rewrites it)"`. Atomic save: `create_dir_all(parent)`, write `.ui-state.toml.tmp` in the
same directory, `sync_all`, `rename`. Serde DTOs live in this file (shell types carry no serde). `Memory` stores the
rendered text, so a test round-trips through the real format. `Nowhere` (no config dir) drops saves silently.
Wiring (Task 6): `App.ui_store: UiStore` (`App::new` → `UiStore::memory()`, so `testkit::Harness::over_backend`
never touches the home directory with no change; it still sets `about_visible = false` after `App::new`).
`lib.rs:228-235` (after `App::new`, before `register_all`): `let store = identity::config_root_path().ok()
.map_or(UiStore::Nowhere, |root| UiStore::File(root.join(FILE_NAME)));` `let loaded = store.load();`
`app.apply_ui_state(loaded.state); app.ui_store = store; app.status = loaded.notice;` (the header flash).
`testkit`: `Harness::with_ui_store(UiStore)` and `Harness::ui_store()` for the restart acceptance test (two harnesses,
one `Memory` handle).

## 6. Layout engine + shell wiring
### 6.1 `ui/shell/layout.rs` (Task 2), pure
```rust
pub const MIN_LIST: u16 = 16; pub const MIN_DETAIL: u16 = 24; pub const MIN_ACTIVITY_ROWS: u16 = 4; pub const MIN_ACTIVITY_COLS: u16 = 20;
#[must_use] pub fn areas(region: Rect, layout: &TabLayout, focus: PaneId) -> PaneAreas;
#[must_use] pub fn visible(areas: &PaneAreas) -> Vec<PaneId>;          // cycle order list, detail, activity
#[must_use] pub fn step(areas: &PaneAreas, from: PaneId, forward: bool) -> PaneId;
#[must_use] pub fn hit(areas: &PaneAreas, x: u16, y: u16) -> Option<PaneId>;
#[must_use] pub fn seam(areas: &PaneAreas, x: u16, y: u16) -> Option<Seam>;   // Seam::{List, Activity}: drag handles
pub struct NoteLabels { pub toggle_list: Option<String>, pub toggle_activity: Option<String>,
                        pub next_preset: Option<String>, pub zoom: Option<String> }
#[must_use] pub fn hidden_note(areas: &PaneAreas, labels: &NoteLabels) -> Option<String>;
impl TabLayout { next_preset, prev_preset, grow_list, shrink_list, grow_activity, shrink_activity,
                 toggle_list, toggle_activity, toggle_zoom, set_list_at(region, x), set_activity_at(region, x, y) }
```
Geometry: `Ide` = list | detail; `IdeActivity` = (list | detail) over activity (`activity_pct` of the height);
`Columns` = list | detail | activity (`activity_pct` of the width); `Focus` = detail alone (`list_hidden =
Preset`). `list_on = false` → `list_hidden = Toggle`; `activity_on = false` in a preset with activity →
`activity_hidden = Toggle`; `zoom` → the focused pane takes the region, the others `Zoom`. **Splits use ratatui's own
constraints** — `Percent(p)` → `[Percentage(p), Percentage(100 - p)]`, `Cells(n)` → `[Length(n), Min(1)]` — so the
defaults reproduce `backlog/mod.rs:1199-1205`, `requirements/mod.rs:1290-1294`, `library.rs:1696-1697`,
`templates.rs:1156-1157` cell for cell. **Clamping** applies only when the region holds both minimums: the list is
clamped into `[MIN_LIST, width - MIN_DETAIL]`; below that the unclamped split is used (today's behaviour), never a
hide. Steps: `Percent` ±5 in 15..=85, `Cells` ±4 (clamped at render), activity ±5 in 15..=60.
`toggle_list` on a hidden list also leaves `Focus` for `Ide`; `toggle_activity` on with a preset without activity moves
to `IdeActivity`. `hidden_note`: list hidden by toggle/preset → `"{toggle_list} show list · {next_preset} layouts"`,
zoom → `"{zoom} unzoom"`, activity toggled off → `"{toggle_activity} show activity"`; a missing label drops its part.
### 6.2 Kit (Task 2)
`kit/pane.rs`: `pub fn paint_focus(buf, area, focus: Focus, accent: Style, theme)` restyles only box-drawing cells
(U+2500..=U+257F) of `area`'s border: focused → `accent` (mono BOLD), unfocused → `theme.border_faint` (mono DIM);
titles and hints keep their style. `pub fn paint_bottom_note(buf, row: Rect, note: Line, style)`: right-aligned twin
of `paint_bottom_hints`, ends two cells before the corner, rule cells only. `Pane::note(Line)` builder for M3.
`kit/empty.rs`: `pub fn placeholder(frame, area, title, what, next: NextStep, theme)`: a rounded faint block titled
` Activity ` with the empty state centred inside.
### 6.3 `Tab` trait additions (`ui/tabs/registry.rs`, Task 6), all defaulted
`fn panes(&self) -> Panes { Panes::Single }`; `fn location(&self) -> TabLoc { TabLoc::default() }`;
`fn restore(&mut self, _loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool { false }`. `Ctx` (Task 6) gains
`layout: Option<TabLayout>`, `claim: Option<&'a Cell<Option<PaneAreas>>>`, builders `with_layout`, `with_pane_claim`,
and `pub fn split(&self, region: Rect, default: TabLayout) -> PaneAreas` = `layout::areas(region,
self.layout.unwrap_or(default), self.focus())` + claim.
### 6.4 `App::render` (`app/state.rs:921-1000`, Task 6)
dirty/editor_area reset; `pane_claim.set(None)`; chrome; header with `ShellChips` (§8); about; active tab rendered with
`ctx.with_editor_area(..).with_focus(focus).with_layout(layout).with_pane_claim(&self.pane_claim)` (layout `Some`
only for a `ListDetail` tab: the stored one, else its default); then, if a claim exists: `kit::empty::placeholder` into
`activity`, `paint_focus` on every visible pane (accent = `theme.tab_accent(tab id)`), `hidden_note` painted on the
detail's bottom border; `draw_editor`; **hint adapter**: unchanged text (`Tab::key_hint`), target = the focused
pane's column: `paint_shell_hints(buf, Rect { x: focused.x, width: body.right() - focused.x, ..body }, …)` with width
`that.width - 4` — with the default list focus this is today's rect exactly (no snapshot churn); with no claim, the
body as in M1 (`:972-984`); overlays get `ctx.with_open_edit(self.editor_title())`; help box.
`App.last_claim: Option<(TabId, PaneAreas)>` keeps the frame's claim for keys and mouse (like `editor_rect`).

## 7. Pane focus and key routing
`App.focus: HashMap<TabId, PaneId>` (default `List` for `ListDetail`, `Detail` for `Single`). `action_for`
(`:377-397`, Task 6) maps `NextPane`/`PrevPane` → `Action::Pane(PaneAction::Next|Prev)`, the nine layout acts →
`Action::Layout(LayoutAction::…)`, `HistoryBack/Forward` → `Action::History(HistoryAction::…)` (new enums in
`app/action.rs`). `update.rs`: `Pane` steps through `layout::step` over `last_claim` (no claim or another tab →
no-op: "on a single-pane tab the pane keys do nothing"); `Layout` mutates `layouts[tab]` (only for `ListDetail`),
re-focuses `Detail` if the focused pane is now hidden, then `save_ui_state()`; `ToggleAbout` saves too.
Tabs learn focus through `ctx.focus()`. **Routing rule (Task 7): only motion follows focus; view verbs (letters,
Enter's replay) act whatever pane is focused**, so M2 changes no verb.
- Backlog (`backlog/mod.rs:789-859`): focus `List` → today's loop. Focus `Detail` → `ListDown/ListUp/ListTop/ListBottom`
  go to `self.detail.step(Step::{Down,Up,Top,Bottom}, ctx)`, `ListFold` is skipped (Enter reaches the pane: Runs
  replays). New `DetailTab::step(&mut self, step, ctx) -> Handled` (default `Pass`, `detail/mod.rs:68-150`) and
  `DetailRegistry::step`: Body/Notes/Prompt scroll (`Scroll::step`, `detail/mod.rs:488`), Runs/Graph/Docs/Reqs move
  their own row cursor (the code their `J`/`K` arms call). Focus `Activity` → motion consumed, nothing moves.
- Requirements (`on_browse_key`, `:451`): focus `Detail` → list motion becomes the detail scroll (`PaneScroll*` arm).
- Skills Library/Templates (`library.rs:820`, `templates.rs:624`): focus `Detail` → list motion becomes
  `self.scroll.apply(PaneScrollDown/Up, rows)`.
- `render`: Backlog replaces `panes(area)` (`:1199`) with `ctx.split(area, BACKLOG_LAYOUT)` (`Percent(55)`), the item
  form still takes `areas.detail`, the divergence view still the whole area; Requirements `ctx.split(content,
  Percent(45))`; Library/Templates `ctx.split(area, Cells(32))` in `render_browse` only. `panes()` returns
  `Panes::ListDetail(that default)`.
**Mouse** (`App::on_mouse` `:700-741`, Task 6): before the tab, with `last_claim` for the active tab:
`Down(Left)` on a seam → start `Drag { seam }`, consume; `Down(Left)` in a pane → set focus, **then still forward**
to the tab (the Runs flow takes clicks); `Drag(Left)` with a drag → `set_list_at`/`set_activity_at`, dirty;
`Up(Left)` ends it and saves. Reachable only while capture is on (see deviation D-6).

## 8. History (Task 3 model, Task 6 wiring)
`ui/shell/history.rs`:
```rust
pub struct Loc { pub tab: TabId, pub pane: PaneId, pub at: TabLoc, pub label: String }
impl Loc { pub fn coarse(&self) -> (TabId, Option<&'static str>, PaneId) }
pub const CAP: usize = 50;
#[derive(Default)] pub struct History { back: VecDeque<Loc>, forward: Vec<Loc> }
impl History {
    pub fn record(&mut self, before: Loc, after: &Loc);           // coarse differs → push before (cursor kept), clear forward, cap 50 (pop_front)
    pub fn back(&mut self, here: Loc) -> Option<Loc>;              // pop back, push `here` on forward
    pub fn forward(&mut self, here: Loc) -> Option<Loc>;           // pop forward, push `here` on back (capped)
    pub fn peek_back(&self) -> Option<&Loc>; pub fn can_forward(&self) -> bool;
}
```
Wiring (Task 6): `App::here() -> Option<Loc>` = active tab id, its focus, `tab.location()`, label =
`title[ › subtab_title][ · list|activity]` (pane named when not the detail). Recording happens around the public
entries the event loop calls — `on_key`, `on_mouse`, `update` (replies that reveal or redirect, e.g. the
Connection redirect) — guarded by a `recording: bool` re-entrancy flag so nested `update`s from `drain` do not
record; `history_move: bool` set by Back/Forward suppresses the record of that dispatch. Back/Forward:
`go(loc)` = `update_tab(Focus(loc.tab))`, `tab.restore(&loc.at, ctx)` + drain, `focus.insert(loc.tab, loc.pane)`.
`location`/`restore` (Task 7): Backlog `subtab = detail.active_id().0` (`DetailId(&'static str)`,
`detail/mod.rs:53`), cursor = the selected item id; restore selects the sub-tab and reveals the item (reuse the
`reveal` path, `backlog/mod.rs:1067`); Requirements cursor = selected requirement id (reveal path `:1309`); Skills
`subtab = "library"|"templates"`, cursor = list index; Settings (`settings/mod.rs`, added to Task 7) `subtab` =
section id, restore = `focus_section`. Chat: defaults.
**Header** (`ui/header.rs`, Task 6): `render(…, chips: &ShellChips, …)` with `pub struct ShellChips { pub back:
Option<(String, String)> /* full, short */, pub forward: Option<String>, pub layout: Option<String> }`. App builds
them with `KeyChord::short_label()` (new in `chord.rs`, Task 6: `Backspace` → `⌫`, `Alt+Left/Right` → `Alt←`/`Alt→`,
arrows → `←→↑↓`, everything else `label()`), through the active stack: `"⌫ back to Backlog › Runs"` (short `"⌫
back"`), `"Alt→ forward"`, layout = `Preset::label()` (+ `" · zoom"`) on `ListDetail` tabs. `right_parts`
(`:184-234`) appends them with the highest ranks so they go first and no existing part's order changes:
back 6 (collapses to short), layout 7, forward 8. Header tests: each chip, dropping order, monochrome.

## 9. Leave dialog (Task 4) — `ui/overlay/leave.rs`
`pub struct LeaveDialog; ID = OverlayId("leave")`; title `"Leave htui?"`; modal; `key_stack` `views::LEAVE`;
`on_key`: `LeaveConfirm` → `emit(Action::Quit)`, `LeaveStay` → `emit(Overlay(Close))`, else `Pass` (the modal swallow
eats it). Lines (D6, verbatim):
- live (`working + waiting > 0`, or `ctx.open_edit()` is `Some`, or `ctx.top_bar.waiting` is `None`):
  `Leaving closes htui. Work it is running stops with it:`; counts row `  ◐ {working} working · ⚠ {waiting} waiting
  {workspace}` (glyphs from `kit::status::mark`, waiting in `warning` when > 0; `–` for both counts while
  `waiting` is `None`); edit row `  ✎ an open edit (unsaved changes are lost)` when `open_edit()` is `Some`;
- nothing live: `Nothing is running. Leave htui?`;
- then a blank line and `chip_line(&[(label(LeaveConfirm), "leave"), (label(LeaveStay), "stay")])` → ` Enter  leave
  Esc  stay`. Size `ModalSize::Fit`, as `quit_confirm.rs:128-138`.
Data: `ctx.top_bar` (`TopBarState.waiting: Option<WaitingView>`, `state.rs:37-48`; the header reads the same,
`header.rs:184-188`) and `ctx.open_edit()` (Task 0 Ctx field; Task 6 fills it from the editor slot for overlays).
Registration (Task 6, `app/mod.rs:124-127`): `register(LeaveDialog::ID, …)`, `app.leave_overlay = Some(..)`; docs step 10
rewritten; `quit_confirm.rs` and its snapshot deleted; `tests/editor_pane.rs:23,460` and `app/pane.rs` tests
(`:684-1361`) move to `LeaveDialog::ID`. Tests: live/nothing-live/edit/unknown-count frames (insta), Enter and ctrl-c
leave, Esc and n stay, other keys `Pass`, chips follow a rebind, monochrome.

## 10. Guard test (Task 5) — `crates/htui/tests/no_hardcoded_keys.rs`
Tokenise every `crates/htui/src/**/*.rs` with `proc_macro2::TokenStream::from_str` (dev-dep `proc-macro2 = { version
= "1", features = ["span-locations"] }`, already 1.0.107 in `Cargo.lock`; tokenising handles comments, strings and raw
strings such as the `r#"…"#` test messages, which a brace scan does not). Walk token trees: an outer attribute
`#[cfg(test)]` or `#[test]` skips the next item (up to and including its brace group, or its `;`);
`#[cfg(any(test, feature = "testkit"))]` is **not** skipped (compiled in testkit builds). A violation is `Ident(KeyCode)`
followed by a joint `::`; report `path:line`. Allow-list (file → reason, each asserted to still contain a `KeyCode::`
so a stale entry fails too): `keys/chord.rs` (parsing, labels), `keys/translate.rs` (canonical events),
`ui/text_field.rs`, `ui/text_area.rs`, `ui/tabs/chat/composer.rs` (canonical events, reached only through
translation), `provision/secret_prompt.rs` (raw tty before the keymap); `editor/pane.rs` **only inside `fn
encode_key`** (`:857-881`; the scanner skips that fn's body by name). Self-tests on inline sources: a cfg(test) mod
with braces in raw strings is skipped, a `KeyCode::` in a comment is ignored, a real site is reported with its line.
Non-test sites today outside the list (all removed by §2): `keys/hint.rs:341`, `runs.rs:1126,1127,1673`,
`filter.rs:254-258`, `item_form.rs:753-759`, `chat/mod.rs:301,605`, `requirements/forms.rs:325-333`,
`skills/attach.rs:271,279,280,641`, `skills/library.rs:1387`. Every other hit (≈1,300) is inside a test module.

## 11. Snapshot impact and tests
Text snapshots carry symbols only, so focus borders (style) are invisible. Expected `cargo insta test --all-features`
diffs: the **layout chip** on header rows of list+detail frames — 40 `tests/snapshots/backlog__*` + 1 `replay__*`
carry the header; Skills/Templates/Requirements frames are bench-rendered without it; the **history chip** on frames
taken after a tab or sub-tab change (≈15, mostly `chat__*`); the **`?` box** rows (3 frames) for the new layers;
`quit_confirm` box deleted; new frames for the leave dialog (4), the layout engine/kit (≈6) and one `tests/shell.rs`
frame with a hidden note. Body rows must not change (D2); the hint row must not change with list focus (§6.4). Tests
that press `tab` to change tab (`tests/keys.rs:476,522`, `backlog_keys/l_a.rs:489-505`, `l_b.rs:330`,
`skills.rs:353-1006`, `templates.rs:546-1175`, `requirements.rs` harness-level ones) and `right`/`left` for sub-tabs
(`backlog_keys/l_a.rs:465`, `l_b.rs:283`, `requirements.rs:479-483`) are Task 6's to move to `ctrl-pgdn`/`shift-right`
(bench-level `bench.key(section, "tab")` presses are form fields and stay).
Per task, new tests: T0 catalogue/stack/views/allow-list demand tests; T1 round trip, atomic write leaves no temp,
missing = first launch, invalid = defaults + reason, unknown keys ignored, memory store; T2 each preset, the four
default splits equal today's rects at 100×30 and 60×20, clamping, hide/zoom notes, `paint_focus` monochrome; T3 the
plan's five; T4 §9; T5 per site "rebound chord works, old literal does not", safety nets, v1 give-way + v1 ctrl-c
append, v2 strict, guard; T6 ctrl-c from a tab, a field, the composer, every overlay, the unfocused editor and the
abort question opens the dialog and a second leaves; focused editor still gets it; Tab moves panes and does nothing on
Settings; layout keys persist (restart acceptance); history back/forward and the header chips; T7 per tab focus
routing and `location`/`restore`.

## 12. Tasks, files, commits
| Task | Files | Commit groups |
|---|---|---|
| 0 (serial) | `keys/catalogue.rs`, `keys/views.rs`, `keys/stack.rs` (layers, `captures_text`, DECLARED), `keys/mod.rs` (`translate` mod, `Keys.version`, tests), `keys/load.rs` (version 1/2 only), `keys/print.rs`, `ui/mod.rs`, `ui/shell/{mod,layout,history}.rs`, `ui/state_file.rs`, `ui/overlay/{mod,leave}.rs`, `keys/translate.rs`, `app/state.rs` (Ctx focus/open_edit only) | (a) contexts + acts + rows + `since`; (b) layers, stacks, allow-lists, validate green; (c) version 2 + print; (d) stubs, shell types, Ctx builders |
| 1 | `ui/state_file.rs` | (a) format + parse/render; (b) store + atomic save |
| 2 | `ui/shell/layout.rs`, `ui/kit/pane.rs`, `ui/kit/empty.rs` | (a) geometry; (b) TabLayout ops + note; (c) kit paint_focus/note/placeholder |
| 3 | `ui/shell/history.rs` | one |
| 4 | `ui/overlay/leave.rs` (+ its snapshots) | one |
| 5 | `keys/stack.rs` (FIXED), `keys/validate.rs`, `keys/load.rs`, `keys/hint.rs`, `keys/chord.rs` (`plain_char`), `keys/translate.rs`, `skills/attach.rs`, `skills/library.rs`, `requirements/forms.rs`, `requirements/mod.rs` (one call), `backlog/detail/runs.rs`, `backlog/filter.rs`, `backlog/item_form.rs`, `chat/mod.rs`, `tests/no_hardcoded_keys.rs`, `tests/keys_file.rs`, `crates/htui/Cargo.toml`, `Cargo.lock` | (a) translate.rs; (b) sites one commit per tab area; (c) FIXED + fixed pass removed, safety nets; (d) ctrl-c accepted + v1 give-way; (e) guard + dev-dep |
| 6 (serial) | `ui/tabs/registry.rs`, `app/{state,update,pane,mod,action}.rs`, `ui/header.rs`, `ui/overlay/{mod,quit_confirm}.rs`, `keys/{catalogue,stack,views,hint,chord}.rs` (default flips, tests, `reachable_label`, `short_label`), `lib.rs`, `testkit.rs`, integration tests that press Tab/arrows, `tests/editor_pane.rs`, `tests/shell.rs` | (a) Tab trait + Ctx split/claim; (b) render: claim, focus, note, placeholder, hints; (c) focus/layout/history dispatch + persistence + lib.rs; (d) leave dialog + ctrl-c path, quit_confirm deleted; (e) header chips; (f) default flips + tests moved to the new keys; (g) mouse |
| 7 (serial, per tab) | `backlog/mod.rs`, `backlog/detail/{mod,body,notes,prompt,runs,graph,documents,requirements}.rs`, `requirements/mod.rs`, `skills/{mod,library,templates}.rs`, `settings/mod.rs` | one commit per tab |
| 8 | `docs/REQUIREMENTS.md`, `docs/decisions/mod/mod-115.md`, `docs/htui-keys.md`, `README.md` | one |
| 9 | `*.snap` | per tab |
**Intersection of the parallel group (re-run on this design):** T1 `{ui/state_file.rs}` · T2 `{ui/shell/layout.rs,
ui/kit/pane.rs, ui/kit/empty.rs}` · T3 `{ui/shell/history.rs}` · T4 `{ui/overlay/leave.rs}` · T5 (row above). All
pairwise empty. T5 grew by `keys/hint.rs`, `keys/chord.rs`, `skills/library.rs`, `requirements/mod.rs`,
`tests/keys_file.rs`, `Cargo.toml`, `Cargo.lock`: none is in T1–T4. Hidden couplings checked: the shared types live in
`ui/shell/mod.rs` (T0), so T2/T3 never import each other; T4 needs only `views::LEAVE`, the `Leave` acts and
`Ctx::open_edit` (all T0); snapshot files are per-module names (no shared `.snap`); no seed or `.sqlx` involved.
T0 shares `keys/*` and `app/state.rs` with T5/T6 serially. T7 shares `requirements/mod.rs` and `skills/library.rs`
with T5 serially.

## PLAN DEVIATIONS
- **D-1 (D4a).** With `[layout]` its own context on list+detail stacks only (B1), the Settings collisions (agents
  paste, hierarchy primary, boxes probe) never arise; the pairs needed are Runs promote and Prompt previous-template
  (`p`) and pause queue (`P`), §1.6. And the plan's "in the Backlog tab presets cycle with `p` only" is false for two
  sub-tabs: in Backlog > Runs and Backlog > Prompt `p` is the sub-tab's (`catalogue.rs:1497-1503`, `:1657-1663`) and
  `P` is pause queue (`:1458-1464`), so no default cycles presets there (`layout.next_preset` is rebindable, and the
  list-focused fallback is not available because Backlog's stack is the sub-tab's).
- **D-2 (Files to Change: `render_panes`).** No `Tab::render_panes`; tabs claim panes with `ctx.split` (B5,
  evidence there).
- **D-3 (D2 `list_pct`).** `TabLayout` needs `ListSize::{Percent, Cells}`: Skills' list is `Length(32)`
  (`library.rs:62,1697`), which no percentage reproduces at every width.
- **D-4 (Task 0 "behaviour stays inert").** Flipping `next_tab`/`prev_tab`, giving `tab`/`backtab` to the pane rows
  and dropping plain arrows cannot be inert (B6); they move to Task 6, so Task 6 also touches `keys/catalogue.rs`,
  `views.rs`/`stack.rs` tests and the integration tests that press Tab (§11).
- **D-5 (D7 ctrl-c, `keys/chord.rs` UPDATE).** `passes_modal` already passes ctrl-c (`chord.rs:53-58`); the real gap
  is that overlay stacks carry no quit (`HELP` = `global ∩ {help}`, `views.rs:53`; `Stack::OVERLAY`), so a `QUIT`
  layer is added to seven stacks (B4).
- **D-6 (Task 6 mouse).** Clicks and drags reach the shell only while mouse capture is on, and capture is on only for
  the Runs flow view (`wants_mouse`, `state.rs:656-661`; `Tab::wants_mouse` doc, `registry.rs:112-124`). Widening it
  to every list+detail tab takes the terminal's text selection away there (MOD-71 D1) and breaks the sole-implementor
  invariant (MOD-74 L1). This blueprint implements the handler and tests it with capture on, and does not widen
  capture. **Needs a maintainer call** if click-to-focus must work on every list+detail tab in M2.
- **D-7 (Task 5 file set).** Missing sites: `skills/library.rs:1387`, `skills/attach.rs:641`, `keys/hint.rs:341` (+ the
  `CTRL_C` appends at `:186,:258,:286`), the `requirements/mod.rs:551` call site, and `Cargo.toml`/`Cargo.lock` for
  the guard's tokenizer. `chat/composer.rs` needs no change (`chat/mod.rs:588-591`).
- **D-8 (D7 `[field]` rows).** `TextArea` also matches `Up`/`Down`/`PageUp`/`PageDown` (`text_area.rs:236-239`); for
  MD1 they become `field.line_up`/`line_down`/`page_up`/`page_down` (§1.4) with two `STATE_GUARDED` pairs against the
  `VIEW_DEFAULTS` field moves.
- **D-9 (D8, MD4).** A version-1 file that sets `[global] quit` would silently lose `ctrl-c` (implicit in v1,
  `state.rs:765-770`); the loader appends it for v1 files (§2). A file without `version` is read as version 1.
- **D-10 (safety net).** `leave.stay` joins the must-keep set (a dialog that cannot be dismissed), and must-keep acts
  may give a chord away only while they keep another (§2), so `[global] help = ["q"]` keeps loading with a notice.
- **D-11 (Files to Change).** The UI-state store is wired in `lib.rs::run` (`:163-235`, where keys and `App` are
  built), not `main.rs`; Task 0 also edits `app/state.rs` (Ctx builders) and `keys/load.rs` (version 2), Task 7 also
  `backlog/detail/*.rs` (detail-focus motion) and `settings/mod.rs` (history location).
- **D-12 (D5 header labels).** `Keys` labels read `Backspace`/`Alt+Right` (`chord.rs:85-120`); the plan's `⌫ back` /
  `Alt→ forward` need `KeyChord::short_label` (§8).
- **D-13 (D7 translation semantics).** "A canonical key that is no longer bound to its row is not delivered" holds only
  where nothing else in the stack binds that key; where another act does (e.g. `choice.prev` keeping `left`), the key
  is delivered and a focused widget still reads it literally (§3 rule 4). Closing that fully needs `&Keys` in the
  widgets, which the plan rejected (25-file signature change). Record it in the decision doc.
