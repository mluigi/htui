# MOD-67 M4 blueprint — Lane L-A: Library and attachments

Scope (PA-1): `crates/htui/src/ui/tabs/skills/library.rs`, `crates/htui/src/ui/tabs/skills/attach.rs`.
Owned tests: `crates/htui/tests/skills.rs`, `crates/htui/tests/skills_pg.rs`; snapshots `skills__*` (9).
§6 is `agent_help.rs`'s arm map: **T1 implements it** (PA-1); it lives here so the Skills material
is in one file. Branch `mod-67-m4-l-a`, worktree `target/wt/l-a`.

Lane rules (plan, M3): never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`,
`skills/{mod,templates,agent_help}.rs`; a missing act or stack goes back to the main thread. Format
only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit
incrementally; stage own paths only.

Complete: sections 0-9.

## 0. Facts every L-A change relies on (read at `329a9440`)

- After T1, `SkillsTab::on_key` (`skills/mod.rs`) resolves `skills.switch_view` from
  `self.key_stack()` (else `views::SKILLS_TAB`) **only when the view does not capture**, then
  delegates. `LibraryView::on_key` therefore never sees a switch chord in browse, report or the
  attachments' browse, and must never handle `SkillsSwitchView`.
- `LibraryView::on_key` (`library.rs:530-542`): the attachments pane, when open, takes every key
  (`on_attach_key`, `:1266`); else by `Mode`.
- `captures_input` (`:477-480`): every mode but `Browse` and `Report`, or the pane's form, picker
  or question.
- `TextField::on_key` passes `Tab`, `BackTab`, `Up`, `Down`, `PgUp`, `PgDn`, `Ins`, `F(n)` and every
  CONTROL/ALT chord; consumes printable, `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`;
  `Enter` = `Submit`, `Esc` = `Cancel`.
- `TextArea::on_key` (`text_area.rs:191`): every chord passes except `ctrl-s` (`Submit`, until
  T-close drops it, D6); `Enter` is a newline; `Esc` = `Cancel`; `Up`/`Down`/`PgUp`/`PgDn` move;
  `Tab`/`BackTab` pass.
- Today the browse and report handlers start with `if !plain(&key) { return Handled::Pass }`
  (`:735`, `:939`), the prompts pass whatever their field passes (`:876`, `:910`), the rename form
  passes CONTROL and swallows the rest (`:1022-1023`), the editor passes what the `TextArea` passes
  (`:1120`).
- T1 provides: every stack below (`keys/views.rs`), `Scroll::apply(act, len)`,
  `AgentHelp::{key_stack, hint(&Keys)}`, the `LibraryView::key_stack` stub returning `None`, and
  `help.hint(ctx.keys())` at `library.rs:687`.

## 1. Library modes and their stacks

| Mode (`enum Mode`, `:230`) | Handler today | captures | Stack |
|---|---|---|---|
| `Browse` (also with a handed-off draft, `handed_off()` `:1255`) | `on_browse_key` `:734` | no | `LIBRARY_BROWSE` |
| `Naming`, `Describing` | `on_naming_key` `:869` | yes | `LIBRARY_PROMPT` |
| `ImportPath` | `on_import_key` `:904` | yes | `LIBRARY_PROMPT` |
| `Info` (rename form) | `on_info_key` `:989` | yes | `LIBRARY_INFO` |
| `Editing`, no help | `on_editor_key` `:1073` | yes | `LIBRARY_EDITOR` |
| `Editing`, help open | `help.on_key` (`:1074-1079`) | yes | `help.key_stack()` (`HELP_ASKING` / `HELP_WAITING` / `HELP_PROPOSAL`) |
| `Report` | `on_report_key` `:938` | no | `LIBRARY_REPORT` |
| any, `attach` is `Some` | `on_attach_key` `:1266` | per pane mode | `pane.key_stack()` (§5.1) |

### 1.1 `key_stack` (replaces T1's stub)

```rust
/// The current mode's stack (MOD-67 M4 D4): the attachments pane's while it is open, else an
/// open agent help's, else the mode's. `SkillsTab::key_stack`, `on_key` and the hint read it.
pub(super) fn key_stack(&self) -> Option<Stack<'static>> {
    if let Some(pane) = &self.attach {
        return Some(pane.key_stack());
    }
    Some(match &self.mode {
        Mode::Browse => views::LIBRARY_BROWSE,
        Mode::Naming { .. } | Mode::Describing { .. } | Mode::ImportPath { .. } => {
            views::LIBRARY_PROMPT
        }
        Mode::Info(_) => views::LIBRARY_INFO,
        Mode::Editing(editor) => match &editor.help {
            Some(help) => help.key_stack(),
            None => views::LIBRARY_EDITOR,
        },
        Mode::Report { .. } => views::LIBRARY_REPORT,
    })
}
```
A private `fn stack(&self) -> Stack<'static>` (`self.key_stack().unwrap_or(views::LIBRARY_BROWSE)`)
is what `on_key` and `render` call.

## 2. Library arm-by-arm conversion

`mb` = matched today; `→` the act through the mode's stack. Every arm becomes a match on `Act`.

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:735` | `!plain(&key) → Pass` | removed | chord equality includes modifiers |
| `:739` | `j`/`Down` | `ListDown` (BROWSE) | `move_cursor(true)` |
| `:740` | `k`/`Up` | `ListUp` | `move_cursor(false)` |
| `:741` | `r` | `Reload` | notice cleared, `StoreRequest::Skills` |
| `:745` | `n` | `New` | only with a snapshot; else consumed as today |
| `:753` | `I` | `LibraryImport` | `Mode::ImportPath` |
| `:759` | `J`/`K`/`PgDn`/`PgUp` | `PaneScrollDown`/`Up`, `PanePageDown`/`Up` | `return self.scroll.apply(act, self.pane_rows.get())` |
| `:762` | `, . b d e E i a` → `on_skill_key(c)` | `SkillsPrevVersion`, `SkillsNextVersion`, `SkillsBase`, `SkillsDiff`, `Edit`, `SkillsEditExternally`, `LibraryInfo`, `LibraryAttach` → `on_skill_key(act)` | notice cleared first, as today; the `char` parameter becomes `Act` |
| `:767` | `_ → Pass` | loop end → `Handled::Pass` | shell resolves `LIBRARY_BROWSE` (`q`, `Tab`, digits, `?`, `w` …) |
| `:799` | `',' \| '.'` | `SkillsPrevVersion \| SkillsNextVersion` | `index` step by act |
| `:816` | `'b'` | `SkillsBase` | |
| `:823` | `'d'` | `SkillsDiff` | |
| `:839` | `'e'` | `Edit` | |
| `:843` | `'E'` | `SkillsEditExternally` | |
| `:854` | `'i'` | `LibraryInfo` | |
| `:863` | `'a'` | `LibraryAttach` | |
| `:876` | field `Pass → Handled::Pass` (naming) | `LIBRARY_PROMPT`: `NextTab \| PrevTab → Pass`, else `modal_rest` | widget first; `Enter`/`Esc` stay the field's (D10) |
| `:910` | field `Pass → Handled::Pass` (import) | same | |
| `:939` | `!plain → Pass` (report) | removed | |
| `:946` | `Esc` | `Back` (REPORT) | Browse, notice cleared, scroll reset |
| `:951` | `j`/`Down` | `ListDown` | |
| `:954` | `k`/`Up` | `ListUp` | |
| `:955` | `J`/`K`/`PgDn`/`PgUp` | pane acts | `self.scroll.apply` |
| `:958` | `r` | `Reload` | |
| `:959` | `_ → Pass` | loop end `Pass` | |
| `:990` | `ctrl-s`/`ctrl-S` before the field | removed: widget first, then `FormSave` (INFO) → `save_info` | PA-3; `TextField` passes `ctrl-s` |
| `:1004` | field `Submit` (`Enter`) | widget-owned, stays | `save_info` |
| `:1008` | field `Cancel` (`Esc`) | widget-owned, stays | busy notice or close |
| `:1018` | `Tab`/`BackTab`/`Up`/`Down` toggle | `FormNextField \| FormPrevField` (INFO; `Down`/`Up` are `VIEW_DEFAULTS`) | `form.focus = 1 - form.focus` |
| `:1022` | CONTROL → `Pass` | removed: `modal_rest(LIBRARY_INFO, chord)` | ALT and `F1` now pass |
| `:1023` | `_ → Consumed` | `modal_rest` | |
| `:1074-1079` | help first | unchanged | `help.on_key` → `apply_help` |
| `:1080` | `ctrl-g`/`ctrl-G` before the area | after the widget: `SkillsAskAgent` (EDITOR) → `open_help` | the area passes chords |
| `:1084` | `ctrl-e`/`ctrl-E` | after the widget: `FormExternalEditor` → `hand_off` | |
| `:1094` | `TextArea::on_key` | widget first, stays | `Consumed` keeps the edit tracking |
| `:1102` | `Submit` (`ctrl-s`) | stays until T-close (then unreachable; T-close fixes the comment) | `save_editor` |
| `:1106` | `Cancel` (`Esc`) | widget-owned, stays | busy / `esc_armed` |
| `:1120` | `Pass → Handled::Pass` | `LIBRARY_EDITOR`: `FormSave` → `save_editor`; `SkillsAskAgent` → `open_help`; `FormExternalEditor` → `hand_off`; `NextTab \| PrevTab` → `Pass`; else `modal_rest` | PA-2: `Tab` keeps the draft and switches tabs |
| `:1272-1278` | busy + form + `Esc` → in-flight notice | **stays**: the form's own `Esc` (widget-owned, D10) | keep `key.code == KeyCode::Esc`, comment it as the form's cancel key |

`fn plain` (`:465`) and `fn chord` (`:470`) are deleted once unused (the attach file keeps its own
until §5). The order inside the editor is §6.2 of the main blueprint (PA-3).

## 3. Library hints (8 constants + 1 inline literal)

Each becomes a `HintSpec` rendered with `ctx.keys().hint(stack, SPEC)`; `render` (`:665-731`)
selects the spec from the mode as today and the stack from `self.stack()`.

| file:line | constant, today's text | stack | `HintSpec` | rendered with defaults |
|---|---|---|---|---|
| `:102` | `BROWSE_HINT` `j/k  ,/. version  b base  d diff  e edit  E $EDITOR  n new  I import  i info  a attach  r reload` | `LIBRARY_BROWSE` | `Pair(ListDown, ListUp, "")`, `Pair(SkillsPrevVersion, SkillsNextVersion, "version")`, `One(SkillsBase, "base")`, `One(SkillsDiff, "diff")`, `One(Edit, "edit")`, `One(SkillsEditExternally, "$EDITOR")`, `One(New, "new")`, `One(LibraryImport, "import")`, `One(LibraryInfo, "info")`, `One(LibraryAttach, "attach")` | `j/k · ,/. version · b base · d diff · e edit · E $EDITOR · n new · I import · i info · a attach` (96 cells with the leading space). **Drops `r reload`** (§1 item 1 of the main blueprint): with ` · ` the full row is 107 cells. Update the doc comment's width note. |
| `:106` | `SCROLL_HINT` ` J/K PgUp/PgDn scroll ` (border title, `:1601`) | `LIBRARY_BROWSE` (the pane's keys, whatever the mode) | `Pair(PaneScrollDown, PaneScrollUp, "scroll")`, `Pair(PanePageUp, PanePageDown, "page")`, drawn as `format!(" {} ", …)` | ` J/K scroll · PgUp/PgDn page ` (no snapshot draws it) |
| `:109` | `NAMING_HINT` `Enter next  Esc cancel` | `LIBRARY_PROMPT` | `Text("Enter next")`, `Text("Esc cancel")` | `Enter next · Esc cancel` |
| `:112` | `INFO_HINT` `Tab field  Ctrl+S save  Esc cancel` | `LIBRARY_INFO` | `One(FormNextField, "field")`, `One(FormSave, "save")`, `Text("Esc cancel")` | `Tab field · Ctrl+s save · Esc cancel` |
| `:117` | `EDIT_HINT` `Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel` | `LIBRARY_EDITOR` | `One(FormSave, "save")`, `One(SkillsAskAgent, "ask agent")`, `One(FormExternalEditor, "$EDITOR")`, `Text("Esc cancel")` | `Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel` |
| `:692` (inline) | `format!("{EDIT_HINT}  L{}:C{}", …)` | — | `format!("{} · L{}:C{}", keys.hint(views::LIBRARY_EDITOR, EDIT), line + 1, col + 1)` | `… · Esc cancel · L1:C2` |
| `:120` | `HANDED_OFF_HINT` `the draft is in $EDITOR` | `LIBRARY_BROWSE` | `Text("the draft is in $EDITOR")` | identical |
| `:127` | `IMPORT_HINT` `Enter import  Esc cancel` | `LIBRARY_PROMPT` | `Text("Enter import")`, `Text("Esc cancel")` | `Enter import · Esc cancel` |
| `:130` | `REPORT_HINT` `j/k move  r reload  Esc back` | `LIBRARY_REPORT` | `Pair(ListDown, ListUp, "move")`, `One(Reload, "reload")`, `One(Back, "back")` | `j/k move · r reload · Esc back` |

The help's row (`:687`) is T1's `help.hint(ctx.keys())`. The attachments pane's row comes from
`pane.render` (§5.3), which now returns a `String`.

Prose naming keys, **left for M6**: `UNSAVED` `:79` ("Esc again discards"), the stale sentences
`:88-99` ("Ctrl+S saves"), `named` `:972` ("select it and press e"), `ACCEPTED` (agent help).

## 4. D6 in the Library

- Editor: widget first (its `ctrl-s` `Submit` saves until T-close), then `FormSave` from
  `LIBRARY_EDITOR` (a rebound chord), PA-3. After T-close only the resolver saves.
- Rename form: `FormSave` from `LIBRARY_INFO` after the field passes (`TextField` passes every
  CONTROL chord), plus the field's own `Enter` (`Submit`, D10).
- Attach form: §5.2.

## 5. `attach.rs`

### 5.1 Modes and `key_stack`

| Mode (`AttachMode`, `:102`) | Handler | captures | Stack |
|---|---|---|---|
| `Browse` | `on_browse_key` `:353` | no | `ATTACH_BROWSE` |
| `Form(_)` | `on_form_key` `:467` | yes | `ATTACH_FORM` |
| `Picker { .. }` | `on_picker_key` `:541` | yes | `ATTACH_PICKER` |
| `ConfirmDetach { .. }` | `on_confirm_key` `:592` | yes | `ATTACH_CONFIRM` |

```rust
/// The pane's stack in its mode (MOD-67 M4 D4); `LibraryView::key_stack` returns it while the
/// pane is open.
pub(super) fn key_stack(&self) -> Stack<'static> {
    match self.mode {
        AttachMode::Browse => views::ATTACH_BROWSE,
        AttachMode::Form(_) => views::ATTACH_FORM,
        AttachMode::Picker { .. } => views::ATTACH_PICKER,
        AttachMode::ConfirmDetach { .. } => views::ATTACH_CONFIRM,
    }
}
```
`on_key` (`:333`) takes `ctx: &Ctx<'_>` already; each handler resolves through `self.key_stack()`
read **before** `core::mem::take(&mut self.mode)` (the take leaves `Browse`).

### 5.2 Arm-by-arm conversion

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:359` | `!plain → Pass` | removed | |
| `:364` | `j`/`Down` | `ListDown` (BROWSE) | |
| `:367` | `k`/`Up` | `ListUp` | |
| `:368` | `Enter` | `AttachChoose` | `open_form`, hint notice as today |
| `:379` | `x` | `AttachDetach` | the question opens (prose notice unchanged, M6) |
| `:398` | `r` | `Reload` | |
| `:399` | `Esc` \| `a` → `Close` | `Back` \| `LibraryAttach` → `Close` | `a` is the Library's own toggle, through `ATTACH_BROWSE`'s `skills.library ∩ {attach}` layer |
| `:400` | `_ → Pass` | loop end `AttachOutcome::Pass` | `h`/`l` never arrive (the tab took them) |
| `:474-483` | CONTROL: `r`/`R` → picker, `s`/`S` → save, else `Pass` | after the widget: `AttachRepo` → `open_picker`; `FormSave` → `save`; else `modal_rest` | |
| `:485` | `Esc` → close | widget-owned (the form's cancel; the text field's `Cancel`), stays | Activation has no field: the form's own `Esc` arm on it (below) |
| `:486` | `Enter` → save | widget-owned (`Submit`), stays | |
| `:487` | `Tab`/`Down` | `FormNextField` (FORM; `Down` is a `VIEW_DEFAULTS` row) | `form.focus.step(true)` |
| `:488` | `BackTab`/`Up` | `FormPrevField` | `step(false)` |
| `:489` | `Space` on Activation | **widget-owned, stays** (ANA §6.1: Activation's `Space`) | only with no modifier but SHIFT |
| `:498` | `_` on Activation → swallowed | after resolution: `modal_rest` | ALT/`F1` now pass |
| `:499-511` | field `on_key`, result ignored | field first; its `Pass` → resolve as above | `F(n)`/`Ins` now: `F1` passes, the rest consumed |
| `:548` | CONTROL → `Pass` | removed (`modal_rest`) | |
| `:558` | `j`/`Down` | `ListDown` (PICKER) | |
| `:561` | `k`/`Up` | `ListUp` | |
| `:562` | `Enter` → insert | `AttachChoose` | the char-by-char insert at `:573` (`KeyEvent::from(KeyCode::Char(c))`) feeds the globs widget: stays (D101) |
| `:580` | `Esc` → back to the form | `Back` | |
| `:584` | `_ => {}` (consumed) | `modal_rest` | |
| `:600` | CONTROL → `Pass` | removed | |
| `:608` | `!= 'y'` → `KEPT` | `ConfirmYes` → detach (`AttachOutcome::Save`); `ConfirmNo` → `KEPT`; no candidate: `stack.passes(chord)` → restore the question and `Pass`, else `KEPT` | **`alt-y` no longer detaches** (ALT inconsistency, ANA §2.6 defect 3); `F1` opens help over the question |

Form dispatch shape (widget first, PA-3):
```rust
let outcome = match form.focus {
    FormField::Activation => activation_key(&mut form, key), // Space toggles (Consumed), Enter
                                                             // Submit, Esc Cancel, else Pass
    _ => focused_field(&mut form).on_key(key),
};
match outcome {
    FieldOutcome::Consumed => {}
    FieldOutcome::Submit => return self.save(form, ctx),
    FieldOutcome::Cancel => return AttachOutcome::Consumed, // mode already taken: closes
    FieldOutcome::Pass => match ctx.keys().actions(views::ATTACH_FORM, chord).first() {
        Some(Act::AttachRepo) => return self.open_picker(form, snapshot, ctx),
        Some(Act::FormSave) => return self.save(form, ctx),
        Some(Act::FormNextField) => form.focus = form.focus.step(true),
        Some(Act::FormPrevField) => form.focus = form.focus.step(false),
        _ if views::ATTACH_FORM.passes(chord) => {
            self.mode = AttachMode::Form(form);
            return AttachOutcome::Pass;
        }
        _ => {}
    },
}
self.mode = AttachMode::Form(form);
AttachOutcome::Consumed
```
`fn plain` (`:233`) and `fn chord` (`:238`) go once unused (`activation_key` uses
`(key.modifiers - KeyModifiers::SHIFT).is_empty()` for `Space`, a widget's value key).

### 5.3 Attach hints (4 constants)

`render` (`:673-703`) returns `String`: `ctx.keys().hint(self.key_stack(), SPEC)`; the Library's
`render` uses it (`pane.render(…)` at `:684` no longer `.to_owned()`).

| file:line | constant, today's text | `HintSpec` | rendered with defaults |
|---|---|---|---|
| `:78` | `BROWSE_HINT` `j/k move  Enter edit  x detach  r reload  Esc back` | `Pair(ListDown, ListUp, "move")`, `One(AttachChoose, "edit")`, `One(AttachDetach, "detach")`, `One(Reload, "reload")`, `One(Back, "back")` | `j/k move · Enter edit · x detach · r reload · Esc back` |
| `:81` | `FORM_HINT` `Tab/Up/Down field  Space activation  Ctrl+R repo  Ctrl+S save  Esc cancel` | `All(FormNextField, "field")`, `Text("Space activation")`, `One(AttachRepo, "repo")`, `One(FormSave, "save")`, `Text("Esc cancel")` | `Tab/Down field · Space activation · Ctrl+r repo · Ctrl+s save · Esc cancel` (drops `Up`, §1 item 2) |
| `:84` | `PICKER_HINT` `j/k move  Enter insert  Esc back` | `Pair(ListDown, ListUp, "move")`, `One(AttachChoose, "insert")`, `One(Back, "back")` | `j/k move · Enter insert · Esc back` |
| `:87` | `CONFIRM_HINT` `y detach  any other key keeps it` | `One(ConfirmYes, "detach")`, `Text("any other key keeps it")` | `y detach · any other key keeps it` |

Prose left for M6: the question notice `:390` ("y detaches, any other key keeps it"), `GLOB_HELP`.

## 6. `agent_help.rs` (implemented by **T1**, PA-1; recorded here)

### 6.1 States and stacks

| State (`:124`) | Stack |
|---|---|
| `Asking` | `HELP_ASKING` |
| `Starting`, `Streaming`, `Cancelling` | `HELP_WAITING` |
| `Proposal`, `Answered` | `HELP_PROPOSAL` |

`pub(super) fn key_stack(&self) -> Stack<'static>` maps them. The editor whose verbs are refused
while the help is open is `self.target`'s: `HelpTarget::Skill { .. }` → `views::LIBRARY_EDITOR`,
`HelpTarget::Template { .. }` → `views::TEMPLATES_EDITOR` (no signature change at `open`).

### 6.2 Arm map

| file:line | today | → | notes |
|---|---|---|---|
| `:260` | `Tab`/`BackTab` → `Pass` | every help stack has `TABS`: `NextTab \| PrevTab` → `HelpOutcome::Pass` | PA-2; `tab_passes_and_the_buffer_is_locked` stays green |
| `:263-269` | CONTROL `s/e/g` → `Note(HELP_OPEN)`, other CONTROL → `Pass` | first step: `ctx.keys().actions(editor_stack, chord)` holds `FormSave`, `FormExternalEditor` or `SkillsAskAgent` → `Note(HELP_OPEN)` (a rebound one too); other chords fall to the state | the refused verbs stay out of the help's own stacks (M3 Rule 9: the `?` box lists no dead key) |
| `:273` | `Starting`: `Esc` → cancel flag + `CANCELLING` | `SkillsHelpCancel` (WAITING) | else `modal_rest` → `Consumed`/`Pass` |
| `:280` | `Streaming`: `Esc` → `ChatCancel`, `Cancelling` | `SkillsHelpCancel` | |
| `:287` | `Cancelling` → `Consumed` | `SkillsHelpCancel` → `Consumed`; else `modal_rest` | |
| `:294` | `Proposal`: `Enter`/`y` (plain) → accept (masked: arm first) | `SkillsHelpAccept` (PROPOSAL) | `ctrl-y`/`alt-y` accept nothing |
| `:302` | `Proposal`: `Esc`/`n` → `Close(DISCARDED)` | `ConfirmNo` | |
| `:305`, `:311` | `_ → scroll_key` | pane acts → `self.scroll.apply(act, self.rows.get())`; else `modal_rest` | `scroll_key` `:317` and `plain` `:236` deleted |
| `:308` | `Answered`: `Esc`/`Enter`/`n` → `Close(None)` | `ConfirmNo \| SkillsHelpAccept` → `Close(None)` | `y` now closes too (§1 item 3) |
| `:325-345` | `Asking`: `Up`/`Down` cycle, `Enter` ask, `Esc` close, else into the request | request field **first**: `Submit` → `ask`; `Cancel` → `Close(None)`; `Consumed`; on `Pass`: `SkillsHelpPrevAgent`/`NextAgent` cycle (wrap), `NextTab \| PrevTab` → `Pass`, else `modal_rest` | `Enter`/`Esc` are the field's (D10) |

`HelpOutcome` for `modal_rest`: `Handled::Pass` → `HelpOutcome::Pass`, `Consumed` → `Consumed`.

### 6.3 Hint (inline literals `:521-529`)

`pub(super) fn hint(&self, keys: &Keys) -> String`, each through `self.key_stack()`:

| State | Today | `HintSpec` | Rendered |
|---|---|---|---|
| `Asking` | `Enter ask  Up/Down agent  Esc back` | `Text("Enter ask")`, `Pair(SkillsHelpPrevAgent, SkillsHelpNextAgent, "agent")`, `Text("Esc back")` | `Enter ask · Up/Down agent · Esc back` |
| `Starting { cancel: true }`, `Cancelling` | `cancelling…` | `Text("cancelling\u{2026}")` | identical |
| `Starting`, `Streaming` | `waiting for the agent  Esc cancel` | `Text("waiting for the agent")`, `One(SkillsHelpCancel, "cancel")` | `waiting for the agent · Esc cancel` |
| `Proposal` | `Enter accept  Esc discard  J/K PgUp/PgDn scroll` | `One(SkillsHelpAccept, "accept")`, `All(ConfirmNo, "discard")`, `Pair(PaneScrollDown, PaneScrollUp, "scroll")`, `Pair(PanePageUp, PanePageDown, "page")` | `Enter accept · n/Esc discard · J/K scroll · PgUp/PgDn page` |
| `Answered` | `nothing to accept  Esc close  J/K scroll` | `Text("nothing to accept")`, `All(ConfirmNo, "close")`, `Pair(PaneScrollDown, PaneScrollUp, "scroll")` | `nothing to accept · n/Esc close · J/K scroll` |

Prose left for M6: `HELP_OPEN` `:44`, `MASKED` `:63`, `ACCEPTED` `:68`.

### 6.4 In-file tests T1 changes / adds

- `open_asks_for_the_agents` `:868`, the streaming hint `:905`, the proposal `:980`, the answer
  `:994`: `help.hint(Keys::compiled())` with the texts of §6.3.
- New: `a_rebound_ask_agent_is_refused_while_open` (`Bench` ctx `.with_keys(load_str("[skills]\nask_agent = \"f3\"\n"))`: `F3` → `Note(HELP_OPEN)`, `ctrl-g` → `Pass`);
  `alt_y_accepts_nothing` (proposal: `alt-y` → `Pass`, state still `Proposal`);
  `f1_passes_from_every_state` (`Asking`, `Streaming`, `Proposal`: `F1` → `Pass`);
  `the_stack_follows_the_state` (`Asking` → `HELP_ASKING`, after `Enter` → `HELP_WAITING`,
  `answered` → `HELP_PROPOSAL`).

## 7. Tests (L-A; write first, red, then code)

### 7.1 Existing assertions that change (`tests/skills.rs`)

| Line | Old | New |
|---|---|---|
| `:281`, `:459`, `:1009`, `:1672` | `contains("Ctrl+S save")` | `contains("Ctrl+s save")` |
| `:544` | `!contains("Ctrl+S save")` | `!contains("Ctrl+s save")` |
| `:1086` | `== "Enter import  Esc cancel"` | `== "Enter import · Esc cancel"` |
| `:1308` | `== "j/k move  r reload  Esc back"` | `== "j/k move · r reload · Esc back"` |
| `:1620` | `contains("Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel")` | `contains("Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel")` |

Stay green unchanged (verify): `:1077`, `:1126`, `:1234` (`I import`), `:1225`, `:1248` (`Esc
back`), `:989` (`Tab` passes from the editor, PA-2), every `.key("ctrl-s")` save (defaults). In-file
`library.rs:2416` names `HANDED_OFF_HINT`: write the literal `"the draft is in $EDITOR"`.

### 7.2 New tests (`tests/skills.rs`, App `Harness`, `#![cfg(feature = "testkit")]`)

| Test | Asserts |
|---|---|
| `a_rebound_save_saves_the_library_editor_rename_and_attach_form` | keys `[form]\nsave = "f2"\n` (`Harness::with_keys`): editor `F2` sends one `SaveSkillVersion`; rename `F2` one `EditSkill`; attach form `F2` one `SetSkillBinding`; each hint shows `F2 save` (D11; T-close adds "`ctrl-s` inert") |
| `a_rebound_library_import_opens_the_form_and_i_is_inert` | `[skills.library]\nimport = "M"\n`: `M` opens ` import skills `, `I` does not; browse hint has `M import`, not `I import` (the lane's rebinding test) |
| `a_rebound_ask_agent_opens_help_from_the_library_editor` | `[skills]\nask_agent = "f3"\n`: `F3` sends `Agents` and the help panel opens; `ctrl-g` does not; editor hint `F3 ask agent` |
| `alt_y_at_the_detach_question_detaches_nothing` | `x` on an attached row, `alt-y`: no `SetSkillBinding`, question notice still shown; `y` then detaches (ALT defect) |
| `f1_opens_help_over_the_attach_form_and_it_stays` | form open, `f1`: the `?` box draws `Attachments:` and `Global: Ctrl+c quit · F1 help`; `f1` again closes; the form is still open |
| `down_and_up_move_the_rename_and_attach_form_fields` | `i`, `down` focuses the description; attach form `down`/`up` move the field (VIEW_DEFAULTS regression pin) |
| `the_library_editor_status_line_keeps_tab` | editor open: status row starts `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help` (PA-2) |
| `ctrl_chords_open_nothing_in_library_browse_or_the_pane` | browse `ctrl-e`, `ctrl-a`, `ctrl-n`: no external edit, no pane, no prompt; pane `ctrl-x`: no question |

`skills_pg.rs`: no key or hint assertion changes expected (run once in the gate if a DB is up; T-
close runs it on the merged tree).

## 8. Snapshots (`skills__*`, 9): what may change

Only the hint row and the status row; any other row moving is a defect (D9). Status rows below
assume the harness's `register_all` (the current rows show `w workspaces · Ctrl+f find`); a row is
clipped at 100 columns as today.

| Snapshot | Hint row after | Status row |
|---|---|---|
| `skills__library`, `skills__diff_two_versions` | `j/k · ,/. version · b base · d diff · e edit · E $EDITOR · n new · I import · i info · a attach` | unchanged |
| `skills__edit`, `skills__changed_elsewhere` | `Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel · L1:C2` | `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q…` (clipped) |
| `skills__agent_help_proposal` | moved in T1 | as `skills__edit` (`HELP_PROPOSAL` has `TABS`) |
| `skills__attachments` | `j/k move · Enter edit · x detach · r reload · Esc back` | unchanged |
| `skills__attach_form_effective_globs` | `Tab/Down field · Space activation · Ctrl+r repo · Ctrl+s save · Esc cancel` | `Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue` (no `Tab`: a field key there) |
| `skills__repo_picker` | `j/k move · Enter insert · Esc back` | as the form's |
| `skills__import_report` | `j/k move · r reload · Esc back` | unchanged |

The notice rows (`Ctrl+S saves it as v4`) are prose: unchanged (M6). Run the full `cargo insta
test` (memory: grepping `.snap` undercounts).

## 9. Commit groups and gate

1. **A1** `feat(mod-67): Library browse and report dispatch on their stacks` — `key_stack`, browse,
   report, `on_skill_key(act)`, browse/report/scroll hints; §7.1 `:1308`; snapshots `library`,
   `diff_two_versions`, `import_report`.
2. **A2** `feat(mod-67): Library prompts, rename form and editor on their stacks (D6)` — §2's
   prompt/info/editor rows, their hints, §4; §7.1 rest; `rebound_save` (editor, rename halves),
   `rebound_ask_agent`, `status_line_keeps_tab`; snapshots `edit`, `changed_elsewhere`,
   `agent_help_proposal`.
3. **A3** `feat(mod-67): attachments pane on its stacks` — `attach.rs` §5, `render` → `String`;
   the attach half of `rebound_save`, `alt_y…`, `f1…`, `down_and_up…`, `ctrl_chords…`; snapshots
   `attachments`, `attach_form_effective_globs`, `repo_picker`.
4. **A4** `test(mod-67): Library rebinding test` — `a_rebound_library_import…`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/skills/{library,attach}.rs crates/htui/tests/skills.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::skills
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test skills -- --test-threads=1
cargo insta test -p htui --all-features --check
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```
Expected: `templates__*` unchanged in this worktree (`TemplatesView::key_stack` is still `None`).
