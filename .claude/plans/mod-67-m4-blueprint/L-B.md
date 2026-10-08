# MOD-67 M4 blueprint — Lane L-B: Templates

Scope: `crates/htui/src/ui/tabs/skills/templates.rs`.
Owned tests: `crates/htui/tests/templates.rs`, `crates/htui/tests/templates_pg.rs`; snapshots
`templates__*` (8). Branch `mod-67-m4-l-b`, worktree `target/wt/l-b`.

Lane rules (plan, M3): never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`,
`skills/{mod,library,attach,agent_help}.rs`; a missing act or stack goes back to the main thread.
Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit
incrementally; stage own paths only.

Complete: sections 0-8.

## 0. Facts (read at `329a9440`)

- After T1 (PA-1): `SkillsTab::on_key` takes `skills.switch_view` from `self.key_stack()` (the
  Templates view's once this lane fills it) while the view does not capture; `agent_help.rs` is
  converted (`AgentHelp::key_stack()`, `AgentHelp::hint(&Keys) -> String`), and `templates.rs:521`
  already reads `Some(help) => help.hint(ctx.keys())`; `TemplatesView::key_stack` exists as a stub
  returning `None`; `Scroll::apply(act, len)` exists.
- `TemplatesView::on_key` (`:369-375`) dispatches on `Mode` (`:196`: `Browse`, `Naming`,
  `Editing`); `captures_input` (`:353`) is every mode but `Browse`.
- Today `on_browse_key` starts `if !plain(&key) { return Handled::Pass }` (`:562`); the name prompt
  passes what its `TextField` passes (`:693`), `Tab` included; the editor passes what its
  `TextArea` passes (`:782`), `Tab` included (pinned: `tests/templates.rs:893`
  `tab_and_digits_while_editing`, PA-2). `ctrl-g`/`ctrl-e` are checked before the area (`:740-748`);
  the area's `ctrl-s` is its `Submit` (`:764`).
- Widget facts (`TextField`, `TextArea`): as L-A §0.

## 1. Modes and stacks

| Mode | Handler today | captures | Stack |
|---|---|---|---|
| `Browse` (also with a handed-off draft, `handed_off()` `:849`) | `on_browse_key` `:561` | no | `TEMPLATES_BROWSE` |
| `Naming { .. }` | `on_naming_key` `:687` | yes | `TEMPLATES_PROMPT` |
| `Editing(editor)`, no help | `on_editor_key` `:733` | yes | `TEMPLATES_EDITOR` |
| `Editing(editor)`, help open | `help.on_key` (`:734-739`) | yes | `help.key_stack()` |

### 1.1 `key_stack` (replaces T1's stub)

```rust
/// The current mode's stack (MOD-67 M4 D4): an open agent help's, else the mode's.
/// `SkillsTab::key_stack`, `on_key` and the hint read it.
pub(super) fn key_stack(&self) -> Option<Stack<'static>> {
    Some(match &self.mode {
        Mode::Browse => views::TEMPLATES_BROWSE,
        Mode::Naming { .. } => views::TEMPLATES_PROMPT,
        Mode::Editing(editor) => editor
            .help
            .as_ref()
            .map_or(views::TEMPLATES_EDITOR, |help| help.key_stack()),
    })
}
```
Once this lands, `SkillsTab` resolves the switch through `TEMPLATES_BROWSE` (its `skills` layer)
instead of `SKILLS_TAB`: same chords by default, and `[skills.templates] switch_view = …` now works
in the Templates view alone.

## 2. Arm-by-arm conversion

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:562` | `!plain(&key) → Pass` | removed | |
| `:566` | `j`/`Down` | `ListDown` (BROWSE) | `move_cursor(true)` |
| `:567` | `k`/`Up` | `ListUp` | |
| `:568` | `r` | `Reload` | notice cleared, `StoreRequest::Templates` |
| `:572` | `n` | `New` | `open_naming` |
| `:573` | `J`/`K`/`PgDn`/`PgUp` | pane acts | `return self.scroll.apply(act, self.pane_rows.get())` |
| `:576-585` | `, . b d D e E` → `on_template_key(c, …)` | `SkillsPrevVersion`, `SkillsNextVersion`, `SkillsBase`, `SkillsDiff`, `TemplatesDiffDefault`, `Edit`, `SkillsEditExternally` → `on_template_key(act, …)` | notice cleared and the `SELECT_A_TEMPLATE` refusal (`:581`) as today; the `char` parameter becomes `Act` |
| `:586` | `_ → Pass` | loop end `Handled::Pass` | |
| `:616` | `',' \| '.'` | `SkillsPrevVersion \| SkillsNextVersion` | |
| `:626` | `'b'` | `SkillsBase` | |
| `:630` | `'d'` | `SkillsDiff` | |
| `:643` | `'D'` | `TemplatesDiffDefault` | |
| `:652` | `'e'` | `Edit` | |
| `:657` | `'E'` | `SkillsEditExternally` | |
| `:693` | field `Pass → Handled::Pass` (name prompt) | `TEMPLATES_PROMPT`: `NextTab \| PrevTab → Pass`, else `modal_rest` | `Enter`/`Esc` stay the field's (D10); `Up`/`Down`/`PgUp` now consumed (the shell bound nothing to them) |
| `:734-739` | help first | unchanged | `help.on_key` → `apply_help` |
| `:740-748` | CONTROL `g`/`G` → `open_help`, `e`/`E` → `hand_off`, before the area | after the area passes: `SkillsAskAgent` → `open_help`, `FormExternalEditor` → `hand_off` | the area passes every chord but `ctrl-s` (until T-close) |
| `:755` | `TextArea::on_key` | widget first, stays | `Consumed` keeps `confirm_item`/`esc_armed` resets |
| `:764` | `Submit` (`ctrl-s`) → `save` | stays until T-close (then unreachable; T-close fixes the comment) | |
| `:768` | `Cancel` (`Esc`) | widget-owned, stays | busy / `esc_armed` |
| `:782` | `Pass → Handled::Pass` | `TEMPLATES_EDITOR`: `FormSave` → `save`; `SkillsAskAgent` → `open_help`; `FormExternalEditor` → `hand_off`; `NextTab \| PrevTab` → `Pass`; else `modal_rest` | main blueprint §6.2; D6 by PA-3 |

`fn plain` (`:323`) is deleted once unused. `KeyModifiers` and `KeyCode` leave the non-test imports.

## 3. Hints (5 constants + 1 inline literal)

`render` (`:501-558`) picks the spec by mode as today and renders through `self.key_stack()`
(`views::TEMPLATES_BROWSE` for the scroll border title, the pane's own keys).

| file:line | constant, today's text | stack | `HintSpec` | rendered with defaults |
|---|---|---|---|---|
| `:78` | `BROWSE_HINT` `j/k move  ,/. version  b base  d diff  D default  e edit  E $EDITOR  n new  r reload  h/l view` | `TEMPLATES_BROWSE` | `Pair(ListDown, ListUp, "")`, `Pair(SkillsPrevVersion, SkillsNextVersion, "version")`, `One(SkillsBase, "base")`, `One(SkillsDiff, "diff")`, `One(TemplatesDiffDefault, "default")`, `One(Edit, "edit")`, `One(SkillsEditExternally, "$EDITOR")`, `One(New, "new")`, `One(Reload, "reload")`, `One(SkillsSwitchView, "view")` | `j/k · ,/. version · b base · d diff · D default · e edit · E $EDITOR · n new · r reload · h view` (97 cells with the leading space). `move` dropped (102 cells otherwise, main §1 item 1); `h/l view` → `h view` (one toggle act, D2; its first chord) |
| `:82` | `SCROLL_HINT` ` J/K PgUp/PgDn scroll ` (border title, `:1092`) | `TEMPLATES_BROWSE` | `Pair(PaneScrollDown, PaneScrollUp, "scroll")`, `Pair(PanePageUp, PanePageDown, "page")`, drawn `format!(" {} ", …)` | ` J/K scroll · PgUp/PgDn page ` (no snapshot draws it) |
| `:85` | `NAMING_HINT` `Enter create  Esc cancel` | `TEMPLATES_PROMPT` | `Text("Enter create")`, `Text("Esc cancel")` | `Enter create · Esc cancel` |
| `:89` | `EDIT_HINT` `Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel` | `TEMPLATES_EDITOR` | `One(FormSave, "save")`, `One(SkillsAskAgent, "ask agent")`, `One(FormExternalEditor, "$EDITOR")`, `Text("Esc cancel")` | `Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel` |
| `:525` (inline) | `format!("{EDIT_HINT}  L{}:C{}", …)` | — | `format!("{} · L{}:C{}", keys.hint(views::TEMPLATES_EDITOR, EDIT), line + 1, col + 1)` | `… · Esc cancel · L1:C1` |
| `:92` | `HANDED_OFF_HINT` `the draft is in $EDITOR` | `TEMPLATES_BROWSE` | `Text("the draft is in $EDITOR")` | identical |

The help's row (`:521`) is T1's. In-file test `templates.rs:1467` names `HANDED_OFF_HINT`: write
the literal.

Prose naming keys, **left for M6**: `OMITS_ITEM` `:68` ("Ctrl+S again saves anyway"), `UNSAVED`
`:72`, the stale sentence `:102-110` ("Ctrl+S saves it"), `create`'s "select it and press e"
(`:720`), `ACCEPTED` (agent help).

## 4. D6 in Templates

Widget first: the `TextArea`'s own `ctrl-s` `Submit` saves until T-close (`:764`); `FormSave` from
`TEMPLATES_EDITOR` on the area's `Pass` saves a rebound chord (PA-3). After T-close only the
resolver saves; the `Submit` arm's comment says so (T-close).

## 5. Collisions and guards

- `TEMPLATES_BROWSE` with defaults: view `D`; `skills` `h l [ ] Left Right , . b d E`; common `n e
  r`; list `j Down k Up`; pane `J K PgDn PgUp`; global `q Tab BackTab 1-9 ? F1 w ctrl-f ctrl-w
  ctrl-q`. Clean.
- `TEMPLATES_PROMPT`: `Tab BackTab` (`TABS`), modal global. `TEMPLATES_EDITOR`: `ctrl-g ctrl-s
  ctrl-e`, `Tab BackTab`, modal global. Clean.
- Guards removed: `:562` (`plain`), `:740` (the CONTROL test). Modifier-blind arms that stop:
  none in browse (the `plain` guard already stopped them); the editor's `ctrl-G` keeps working
  through M3's kitty fold (`KeyChord::new`).

## 6. Tests (write first, red, then code)

### 6.1 Existing assertions that change (`tests/templates.rs`)

| Line | Old | New |
|---|---|---|
| `:412` | `ends_with("Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel  L1:C1")` | `ends_with("Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel · L1:C1")` |
| `:494`, `:839`, `:868`, `:916`, `:1140` | `contains("Ctrl+S save")` | `contains("Ctrl+s save")` |
| `:509`, `:887` | `!contains("Ctrl+S save")` | `!contains("Ctrl+s save")` |

Changed in T1 already (do not touch): `:1038`, `:1111`. Stay green unchanged (verify): every
`ends_with("L…:C…")`, `:893` (`Tab` passes, PA-2), the notices quoting `Ctrl+S` (`:292`, `:498`,
`:548`, prose), every `.key("ctrl-s")` and `.key("ctrl-g")` (defaults).

### 6.2 New tests (`tests/templates.rs`, App `Harness`)

| Test | Asserts |
|---|---|
| `the_templates_editor_status_line_and_help_box` (D11) | `register_all`, editor open: the status row starts `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · Ctrl+f find`; `?` is typed into the draft; `f1` opens the box, which has `Skills: Ctrl+g ask agent`, `Form: Ctrl+s save · Ctrl+e $EDITOR`, exactly one line starting `Global:` and it contains `Tab next tab`, and `F1 closes this box`, not `?/F1`; `f1` closes it, the draft unchanged |
| `a_rebound_save_saves_a_templates_draft` (D11) | `[form]\nsave = "f2"\n`: type, `F2` sends one `SaveTemplate`; the hint shows `F2 save` (T-close adds "`ctrl-s` inert") |
| `a_rebound_diff_default_diffs_and_capital_d_is_inert` (lane rebinding test) | `[skills.templates]\ndiff_default = "X"\n`: `X` shows the diff against the compiled default, `D` does not; hint `X default`, no `D default` |
| `h_and_l_switch_from_templates_but_ctrl_h_does_not` (D5 through `TEMPLATES_BROWSE`) | on the Templates view: `ctrl-h`, `alt-h` leave Templates shown; `h` shows the Skills view; back with `l` |
| `a_shared_skills_rebind_reaches_templates` | `[skills]\ndiff = "f6"\n`: `F6` toggles the diff pane, `d` does not; hint `F6 diff` |
| `tab_leaves_the_name_prompt_and_the_agent_help_with_the_draft` | name prompt open, `tab` → Requirements tab active, `2` back, prompt still open; editor + `ctrl-g` help open, `tab` → Requirements, back, help still open (PA-2 pins for the two other `TABS` stacks) |

`templates_pg.rs`: no key or hint assertion changes expected.

## 7. Snapshots (`templates__*`, 8): what may change

Only the hint row and the status row (D9). Status rows assume `register_all` (current rows show
`w workspaces · Ctrl+f find`) and are clipped at 100 columns as today.

| Snapshot | Hint row after | Status row |
|---|---|---|
| `templates__browse`, `templates__diff_two_versions` | `j/k · ,/. version · b base · d diff · D default · e edit · E $EDITOR · n new · r reload · h view` | unchanged |
| `templates__edit_help`, `templates__changed_elsewhere`, `templates__missing_item_confirm`, `templates__unknown_placeholder_cursor` | `Ctrl+s save · Ctrl+g ask agent · Ctrl+e $EDITOR · Esc cancel · L<l>:C<c>` (cursor as now) | `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q…` (clipped) |
| `templates__agent_help_asking`, `templates__agent_help_proposal` | moved in T1, unchanged here | as the editor's (`HELP_ASKING`, `HELP_PROPOSAL` carry `TABS`) |

Notice rows (`Ctrl+S saves it as v3`, `Ctrl+S again saves anyway`, the placeholder error) are
unchanged. Run the full `cargo insta test`.

## 8. Commit groups and gate

1. **B1** `feat(mod-67): Templates browse on its stack` — `key_stack`, browse arms,
   `on_template_key(act)`, browse/scroll/handed-off hints; snapshots `browse`, `diff_two_versions`;
   `h_and_l_switch…`, `a_rebound_diff_default…`, `a_shared_skills_rebind…`.
2. **B2** `feat(mod-67): Templates prompt and editor on their stacks (D6)` — prompt and editor
   arms, editor/naming hints, §6.1; `a_rebound_save…`, `the_templates_editor_status_line…`,
   `tab_leaves…`; snapshots `edit_help`, `changed_elsewhere`, `missing_item_confirm`,
   `unknown_placeholder_cursor`, `agent_help_asking`, `agent_help_proposal`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/skills/templates.rs crates/htui/tests/templates.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::skills
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test templates -- --test-threads=1
cargo insta test -p htui --all-features --check
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```
Expected: `skills__*` unchanged in this worktree (the Library is L-A's).
