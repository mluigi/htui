# MOD-67 M4 blueprint — Lane L-C: Requirements

Scope: `crates/htui/src/ui/tabs/requirements/mod.rs`, `crates/htui/src/ui/tabs/requirements/forms.rs`
(`detail.rs`, `tree.rs` match no key and are not touched; plan claim 4).
Owned tests: `crates/htui/tests/requirements.rs`, `crates/htui/tests/requirements_pg.rs`; snapshots
`requirements__*` (7). Branch `mod-67-m4-l-c`, worktree `target/wt/l-c`.

Lane rules (plan, M3): never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`,
`ui/text_area.rs`, `ui/text_field.rs`, `ui/tabs/backlog/**`; a missing act or stack goes back to
the main thread. Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and
`--test-threads=1`. Commit incrementally; stage own paths only.

Complete: sections 0-8.

## 0. Facts (read at `329a9440`)

- `RequirementsTab::on_key` (`mod.rs:1020-1037`): (1) a capturing mode + `ctrl-s`/`ctrl-S` →
  `save` (`:1022`, also while busy, where `save` reports the write in flight); (2) any non-plain
  key → `Pass` (`:1026`; CONTROL and ALT reach the shell, nothing reaches a widget); (3) by `Mode`
  (`:190`: `Browse`, `Filter`, `NewArea`, `Requirement`, `Withdraw`).
- Forms swallow every plain key while a write is in flight (`:491-493`).
- `AreaForm::on_key` (`forms.rs:81-106`): `Enter` on the code moves to the title, on the title
  submits; `Tab`/`BackTab`/`Up`/`Down` toggle the two fields; `Esc` cancels; else `Stay`.
- `RequirementForm::on_key` (`forms.rs:304-351`): `Tab`/`BackTab` cycle `Body → Rationale →
  Priority (→ Deciding)` first; `Body`/`Rationale` are `TextArea`s (`Enter` is a newline,
  `ctrl-s` their `Submit`, never reached because of step (2)); `Priority` takes `m`, `l`, `Space`,
  `Left`, `Right`, `Esc` and ignores the rest; `Deciding` is a `TextField` (`Enter` submits).
- `WithdrawForm::on_key` (`forms.rs:511-520`): one `TextField` per stage; `Enter` submits (the
  first stage moves on to the typed key), `Esc` cancels.
- T1 provides every stack below, the `(Requirements, FormNextField/FormPrevField)` view defaults
  (`["down"]`/`["up"]`), and `Scroll::apply(act, len)`.

## 1. Modes and stacks

| Mode | Handler today | captures | Stack |
|---|---|---|---|
| `Browse` | `on_browse_key` `:428` | no | `REQUIREMENTS_BROWSE` |
| `Filter { field }` | `on_filter_key` `:463` | yes | `CAPTURE` (M3's; a text field and nothing else) |
| `NewArea(_)`, `Requirement(_)` (mint and amend) | `on_form_key` `:490` | yes | `REQUIREMENTS_FORM` |
| `Withdraw(_)` (both stages) | `on_form_key` | yes | `REQUIREMENTS_WITHDRAW` |

```rust
/// The current mode's stack (MOD-67 M4 D4): the only place a mode maps to its keys.
fn stack(&self) -> Stack<'static> {
    match &self.mode {
        Mode::Browse => views::REQUIREMENTS_BROWSE,
        Mode::Filter { .. } => views::CAPTURE,
        Mode::NewArea(_) | Mode::Requirement(_) => views::REQUIREMENTS_FORM,
        Mode::Withdraw(_) => views::REQUIREMENTS_WITHDRAW,
    }
}
// impl Tab for RequirementsTab
fn key_stack(&self) -> Option<Stack<'static>> { Some(self.stack()) }
```

## 2. Arm-by-arm conversion

### 2.1 `mod.rs`

| file:line | today | → act (stack) | notes |
|---|---|---|---|
| `:1022-1025` | capturing + `ctrl-s` → `save` | moved into the form and filter handlers (below): `FormSave` through the mode's stack | the filter's stack (`CAPTURE`) has no `form.save`, so `ctrl-s` there passes (today: `save` did nothing on the filter) |
| `:1026` | `!plain → Pass` | browse and filter: removed (chord equality; the field passes every chord); forms: kept as **the widget guard** (PA-3: a chord never reaches a form widget, so `ctrl-m` cannot set Priority and the `TextArea` never sees `ctrl-s`) | `plain` stays for that one use |
| `:430` | `j`/`Down` | `ListDown` (BROWSE) | `step(1)` |
| `:431` | `k`/`Up` | `ListUp` | `step(-1)` |
| `:432` | `g`/`Home` | `ListTop` | `jump(false)` |
| `:433` | `G`/`End` | `ListBottom` | `jump(true)` |
| `:434` | `Enter` → `fold()` | `ListFold` | `fold()` returns `Pass` on a requirement row: `continue` (no other candidate; the loop ends `Pass`, as today) |
| `:435` | `J`/`K`/`PgDn`/`PgUp` | pane acts | `return self.scroll.apply(act, self.pane_rows.get())` |
| `:438` | `/` | `RequirementsFilter` | opens the filter on the current text |
| `:444` | `Esc` with a filter | `Back`, declines (`continue`) with no filter | clears and re-selects |
| `:448` | `a`/`n`/`e`/`W` → `write_key(c)` | `RequirementsNewArea`, `New`, `RequirementsAmend`, `RequirementsWithdraw` → `write_key(act)` | `write_key` (`:347`) matches `(act, row)` instead of `(char, row)`; every refusal unchanged |
| `:449` | `r` | `Reload` | tree + detail re-read |
| `:458` | `_ → Pass` | loop end `Handled::Pass` | |
| `:463-487` | filter: field; `Pass` → `Consumed` | widget first (every key); on `Pass` → `modal_rest(views::CAPTURE, chord)` | `Enter`/`Esc` are the field's (D10); CONTROL/ALT pass as today; `F1` now opens help (was swallowed) |
| `:490-509` | forms | §2.2's shape | |

### 2.2 Form dispatch (`on_form_key`, D6 by PA-3)

```rust
fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
    let stack = self.stack();
    let chord = KeyChord::from_event(key);
    if self.busy.is_some() {
        // The reply closes the form or keeps it: a save says the write is in flight, a chord
        // the stack passes still reaches the shell, everything else is swallowed.
        return match ctx.keys().actions(stack, chord).first() {
            Some(Act::FormSave) => { self.save(ctx); Handled::Consumed }
            _ => modal_rest(stack, chord),
        };
    }
    // A chord never reaches a form widget (today's rule, `:1026`): it goes to the resolver.
    let outcome = if plain(&key) { self.focused_form_mut().on_key(key) } else { FormOutcome::Pass };
    match outcome {
        FormOutcome::Stay => Handled::Consumed,
        FormOutcome::Cancel => { self.mode = Mode::Browse; self.notice = None; Handled::Consumed }
        FormOutcome::Submit => { self.save(ctx); Handled::Consumed }
        FormOutcome::Pass => match ctx.keys().actions(stack, chord).first() {
            Some(Act::FormSave) => { self.save(ctx); Handled::Consumed }
            Some(Act::FormNextField) => { self.focus_step(true); Handled::Consumed }
            Some(Act::FormPrevField) => { self.focus_step(false); Handled::Consumed }
            _ => modal_rest(stack, chord),
        },
    }
}
```
`focus_step` calls `AreaForm`'s toggle or `RequirementForm::cycle(forward)`; the withdraw stack
offers neither. `modal_rest` is `crate::ui::tabs::settings::modal_rest` (`pub(crate)`).

### 2.3 `forms.rs`

| file:line | today | → | notes |
|---|---|---|---|
| `:29` `FormOutcome` | `Stay`, `Cancel`, `Submit` | + `Pass` — "a key the focused widget did not use: the tab resolves it (MOD-67 M4)" | |
| `:87-90` | code `Enter` → title | widget-owned, stays | |
| `:93-104` | `Tab`/`BackTab`/`Up`/`Down` toggle | removed: the field's `Pass` → `FormOutcome::Pass`; the tab resolves `FormNextField` (`Tab`, `Down` view default) / `FormPrevField` (`BackTab`, `Up`) | add `pub(super) fn toggle(&mut self)` (two fields) |
| `:105` | `Consumed \| Pass → Stay` | `Consumed → Stay`, `Pass → Pass` | |
| `:274` `cycle` | private | `pub(super)` | |
| `:306-315` | `Tab`/`BackTab` cycle before the widget | removed (resolver, after the widget) | the `TextArea` passes `Tab`; Priority and Deciding pass it (below) |
| `:323-326` | area `Consumed \| Pass → Stay` | `Consumed → Stay`, `Pass → Pass` | the area's `Submit` (`ctrl-s`) is never reached (§2.2 guard) |
| `:331-339` | Priority `m`, `l`, `Space`, `Left`, `Right`, `Esc` | **widget-owned, stay** (ANA §6.1 choice-field value keys, D10) | plain keys only reach it (§2.2) |
| `:340` | Priority `_ => {}` (`Stay`) | `_ => return FormOutcome::Pass` | so `Tab`, a rebound `F2` save, `F1` reach the resolver |
| `:344-348` | Deciding `Consumed \| Pass → Stay` | `Pass → Pass` | |
| `:516-518` | withdraw `Consumed \| Pass → Stay` | `Pass → Pass` | `Tab` there: no act in `REQUIREMENTS_WITHDRAW`, `modal_rest` consumes it (as today) |

Behaviour with the defaults, form by form: identical, except (a) `Down`/`Up` on Priority and
Deciding now move the focus (main blueprint §1 item 4: the area form's view defaults apply to the
whole `requirements` context), (b) `F1` opens help and ALT chords pass (D5). Pin both.

## 3. Hints (7 constants)

`hint(&self, theme)` (`:965`) becomes `hint(&self, keys: &Keys, theme: &Theme) -> Line<'static>`;
`render` (`:1182`) passes `ctx.keys()`. Every spec renders through `self.stack()`, except the
browse groups, which render through `views::REQUIREMENTS_BROWSE`.

| file:line | constant, today's text | `HintSpec` | rendered with defaults |
|---|---|---|---|
| `:92` | `HINT_MOVE` `j/k move  Enter fold  / filter  ` | `Pair(ListDown, ListUp, "move")`, `One(ListFold, "fold")`, `One(RequirementsFilter, "filter")` | `j/k move · Enter fold · / filter` |
| `:95` | `HINT_WRITES` `a area  n new  e amend  W withdraw` | `One(RequirementsNewArea, "area")`, `One(New, "new")`, `One(RequirementsAmend, "amend")`, `One(RequirementsWithdraw, "withdraw")` | `a area · n new · e amend · W withdraw` (dimmed where a write is refused, as today) |
| `:98` | `HINT_RELOAD` `  r reload` | `One(Reload, "reload")` | `r reload` |
| `:101` | `FILTER_HINT` `  Enter apply  Esc clear` | `Text("Enter apply")`, `Text("Esc clear")` | drawn `"  {}"` after the field: `  Enter apply · Esc clear` |
| `:107` | `FORM_HINT` `Tab field  Ctrl+S save  Esc cancel` | `One(FormNextField, "field")`, `One(FormSave, "save")`, `Text("Esc cancel")` | `Tab field · Ctrl+s save · Esc cancel` |
| `:110` | `PRIORITY_HINT` `  m/l priority` | `Text("m/l priority")`, appended with ` · ` on the Priority field | `Tab field · Ctrl+s save · Esc cancel · m/l priority` |
| `:113` | `WITHDRAW_HINT` `Enter next  Esc cancel` | `Text("Enter next")`, `Text("Esc cancel")` | `Enter next · Esc cancel` |

Browse row: the three groups are three spans (base, writes style, base) joined by `Span::styled("
· ", theme.base)`, an empty group (all unbound) dropped with its separator, the row led by one
space as today: ` j/k move · Enter fold · / filter · a area · n new · e amend · W withdraw · r
reload` (84 cells). The filter's ` /` prompt sigil before the field is the prompt's glyph, not a
key hint: it stays (it matches `tree.rs`'s `· /<filter>` title, untouched).

Prose naming keys, **left for M6**: the module doc `:21-24` ("Ctrl+S"), `mint_failed` and the
stale sentence (`:130-148`, "Ctrl+S"), `NOT_THE_REQUIREMENT_KEY`.

## 4. D6 in Requirements

`ctrl-s` (and any rebound `form.save`) saves from every form field through the resolver (§2.2),
never through a `TextArea` `Submit`; so T-close's `TextArea` change touches nothing here. While a
write is in flight, `form.save` still answers with the in-flight notice.

## 5. In-file tests that change (`mod.rs`)

- `write_words_style` (`:1414-1424`): the span to find is the rendered writes group,
  `Keys::compiled().hint(views::REQUIREMENTS_BROWSE, HINT_WRITES)`; `tab.hint(Keys::compiled(),
  theme)`; the `use super::HINT_WRITES` import stays (now a `HintSpec`).
- `save` helper (`:1575`) sends `ctrl-s`: unchanged (the default `form.save`).
- `Bench::key` (`:1291`) sends plain `KeyCode`s: unchanged.

## 6. Tests (L-C; write first, red, then code; `tests/requirements.rs`, App `Harness`)

| Test | Asserts |
|---|---|
| `the_requirements_tree_status_line_and_help_box` (D11) | browse: the status row is today's; `?` opens the box with `Requirements: a new area · e amend · W withdraw · / filter`, `Common: n new · r reload · Esc back`, `List: j/Down down · k/Up up · g/Home top · G/End bottom · Enter fold`, `Pane: J scroll down · K scroll up · PgDn page down · PgUp page up`, a `Global:` line, and `?/F1 closes this box` |
| `a_rebound_save_saves_a_requirements_draft` (D11) | `[form]\nsave = "f2"\n`: `n` on an area, type a body, `F2` mints (`minted R-ENT-3`); the form hint shows `F2 save` (T-close adds "`ctrl-s` inert") |
| `a_rebound_amend_amends_and_e_is_inert` (lane rebinding test) | `[requirements]\namend = "E"\n`: `e` opens nothing; `E` opens ` Amend R-ENT-1 (v2) `; the browse hint has `E amend`, not `e amend` |
| `the_priority_field_keeps_its_value_keys_and_ignores_chords` | on Priority: `l` → Later, `m` → Must, `space` toggles, `left` toggles; `ctrl-l` and `alt-m` change nothing (risk row 6) |
| `tab_and_down_move_the_form_focus` | area form: `tab` and `down` move code → title, `backtab`/`up` back; requirement form: `tab` cycles Body → Rationale → Priority, `down` on Priority moves to the next field (§1 item 4 of the main blueprint, pinned on purpose) |
| `f1_opens_help_from_a_form_and_the_form_stays` | `n`, `f1`: the box draws `Requirements:`/`Form:` lines and `F1 closes this box`; `f1` closes; the form is still open with its text |
| `ctrl_s_on_the_filter_saves_nothing_and_types_nothing` | `/`, `ctrl-s`: no request, the filter text unchanged |
| `a_write_in_flight_swallows_keys_but_save_says_so` | after `ctrl-s` sent a mint, `x` changes nothing, `ctrl-s` shows the in-flight notice, `ctrl-c` still quits (App) |

Existing `tests/requirements.rs` assertions: none reads a hint row (checked); every `.key("tab")`,
`.key("ctrl-s")`, `.key("enter")` stays green with the defaults. `requirements_pg.rs`: no key or
hint assertion changes expected.

## 7. Snapshots (`requirements__*`, 7)

All seven are browse frames: **only the hint row moves**, to
` j/k move · Enter fold · / filter · a area · n new · e amend · W withdraw · r reload`. Status rows
unchanged (`REQUIREMENTS_BROWSE`'s global layer is unfiltered); `requirements_read_only`'s last row
is the refusal and stays. Notice rows unchanged. Any other row moving is a defect (D9).

## 8. Commit groups and gate

1. **C1** `feat(mod-67): Requirements forms resolve save and field moves after their widgets (D6)`
   — `forms.rs` §2.3, `on_form_key` §2.2, form/priority/withdraw hints; `a_rebound_save…`,
   `the_priority_field…`, `tab_and_down…`, `f1_opens_help_from_a_form…`, `a_write_in_flight…`.
2. **C2** `feat(mod-67): Requirements tree and filter on their stacks` — `stack`/`key_stack`,
   browse and filter arms, `write_key(act)`, browse/filter hints, §5; the seven snapshots;
   `the_requirements_tree_status_line…`, `ctrl_s_on_the_filter…`.
3. **C3** `test(mod-67): Requirements rebinding test` — `a_rebound_amend…`.

Gate (each commit: its tests; before handoff: all):
```bash
rustfmt --edition 2024 --check crates/htui/src/ui/tabs/requirements/{mod,forms}.rs crates/htui/tests/requirements.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::tabs::requirements
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test requirements --test reveal --test concepts_search -- --test-threads=1
cargo insta test -p htui --all-features --check
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```
(`reveal.rs` and `concepts_search.rs` open the Requirements tab; neither reads its hint row.)
