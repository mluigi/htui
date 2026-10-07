# MOD-67 M3 blueprint - lane L-D: Settings > Personas, Settings > Secrets

Scope: `crates/htui/src/ui/tabs/settings/personas.rs`, `crates/htui/src/ui/tabs/settings/secrets.rs`.
Owned tests: `crates/htui/tests/personas.rs`, `crates/htui/tests/secrets_settings.rs`; snapshots `personas__*`, `secrets_settings__*`.
Line numbers are against `fcfb00e8` (hr/MOD-67). Status: complete.

## 0. Proposed catalogue additions (T1 owns; lane L-D consumes)

| Context (`table`, heading) | Act variant | name | defaults | help | in_capture | mirrors |
|---|---|---|---|---|---|---|
| `SettingsPersonas` (`settings.personas`, `Personas`) | `PersonasBody` | `body` | `["b"]` | `edit body` | no | `personas.rs:444` (`'b'` → `:477`) |
| `SettingsPersonas` | `PersonasRules` | `rules` | `["r"]` | `edit rules` | no | `personas.rs:444` (`'r'` → `:478`); **not** `common.reload` (D2 split: other meaning, like `settings.agents.probe`) |
| `SettingsPersonas` | `PersonasImport` | `import` | `["I"]` | `import` | no | `personas.rs:444` (`'I'` → `:464`) |
| `SettingsSecrets` (`settings.secrets`, `Secrets`) | `SecretsCheck` | `check` | `["t"]` | `check` | no | `secrets.rs:1416` |

`VIEW_DEFAULTS` rows this lane needs (D12; the plan's D12 list names only agents, hierarchy and migration, so **T1 must add these five**):

| Context | Act | chords | why |
|---|---|---|---|
| `SettingsPersonas` | `FormNextField` | `["tab", "down"]` | `personas.rs:1576` takes `Tab` and `Down` |
| `SettingsPersonas` | `FormPrevField` | `["backtab", "up"]` | `personas.rs:1580` |
| `SettingsPersonas` | `Back` | `["esc", "enter"]` | `personas.rs:841` closes the report on `Esc` and `Enter` (the `common.back` row's own comment already cites it) |
| `SettingsSecrets` | `FormNextField` | `["tab", "down"]` | `secrets.rs:674` |
| `SettingsSecrets` | `FormPrevField` | `["backtab", "up"]` | `secrets.rs:675` |

Shared rows whose citation comments gain these sites (no default changes): `list.down` personas.rs:434, :839, secrets.rs:1417; `list.up` personas.rs:440, :840, secrets.rs:1418; `confirm.yes` secrets.rs:799-800; `confirm.no` secrets.rs:827; `form.next_field` personas.rs:1576, secrets.rs:674; `form.prev_field` personas.rs:1580, secrets.rs:675; `form.save` personas.rs:581/:706 (via `text_area.rs:194`); `common.edit` personas.rs:444 `'e'`, secrets.rs:1414; `common.new` personas.rs:444 `'n'`; `common.delete` personas.rs:444 `'d'`; `common.clear` secrets.rs:1415; `common.reload` secrets.rs:1420; `common.dismiss` secrets.rs:1424.

**Layer rule this lane assumes (see Q1):** every stack's first layer is `Layer::only(<view context>, <every act the mode dispatches>)`, never `Layer::all(<view context>)`. Reason: a `VIEW_DEFAULTS` row or a user override (`[settings.personas] next_field = ...`) is a row *in the view context*; with `Layer::all` it would leak into every mode of the view (Browse would list `Tab/Down next field` and `Esc/Enter back` in the `?` box, and `Down` would yield `FormNextField` ahead of `ListDown`).

---

## 1. `crates/htui/src/ui/tabs/settings/personas.rs`

### 1.1 Modes

`captures_input()` (`:1739-1741`) is `!matches!(mode, Browse)`: every mode but Browse is modal (`Layer::modal(Global)`), and the Settings tab delegates every key to it (`settings/mod.rs:339-345`).

| Mode (`:177-210`) | captures | modal | widget owning text | widget-fixed keys (D13) | view keys today |
|---|---|---|---|---|---|
| `Browse` (unavailable / empty / rows: three hint variants, one key behaviour) | no | no | none | - | `j`/`Down`, `k`/`Up`, `n e b r d I`; CONTROL\|ALT pass (`:427-432`) |
| `Editing(Editor)` `Target::Create` | yes | yes | `TextField` per field (`Field.input`) | chars, `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`, `Enter` (Submit → `submit_form` `:508`, create → body), `Esc` (Cancel → Browse `:512`) | `form_navigation` `:1573`: `Tab`/`Down`, `BackTab`/`Up`; CONTROL passes; rest swallowed |
| `Editing(Editor)` `Target::Edit` | yes | yes | same | same, `Enter` sends the patch | same |
| `Body(BodyEditor)` `BodyTarget::Create` | yes | yes | `TextArea` | chars, `Enter` (newline), `Backspace`, `Delete`, arrows, `Home`, `End`, `PgUp`, `PgDn`, `Esc` (Cancel → back to the form `:603-618`), **`ctrl-s`/`ctrl-S` Submit** (`text_area.rs:194`, D13 stays) | none; Pass+CONTROL passes (`:589`), rest swallowed (`Tab`, `F1`, ALT) |
| `Body(BodyEditor)` `BodyTarget::Edit` | yes | yes | `TextArea` | same; `Esc` warns once then closes (`:620-626`) | same |
| `Rules(RulesEditor)` | yes | yes | `TextArea` | same; `Esc` warns once (`:710-720`) | same (`:722`) |
| `Deleting { stage: Asking }` | yes | yes | none | - | `y`, `n`, `Esc` (`:784-796`); CONTROL passes (`:775`); rest swallowed |
| `Deleting { stage: InFlight }` | yes | yes | none | - | every non-CONTROL key swallowed (`:781-783`) |
| `ImportPath { field }` | yes | yes | `TextField` | chars, editing keys, `Enter` (Submit → send import `:807-819`), `Esc` (Cancel `:820-824`) | none; Pass+CONTROL passes (`:825`), rest swallowed |
| `Report { .. }` | yes | yes | none | - | `j`/`Down`, `k`/`Up` scroll, `Esc`/`Enter` close (`:838-843`); CONTROL passes (`:832`); rest swallowed |

The 7 CONTROL checks: `:427-432` (CONTROL\|ALT, Browse), `:589` (body), `:722` (rules), `:775` (delete), `:825` (import), `:832` (report), `:1584` (`form_navigation`). All seven become redundant under D5/D6 (section 1.5).

### 1.2 Key-match sites → actions

| Site | Chord(s), modifiers | State guard | Behaviour | → (context, act, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|
| `:434` | `j`, `Down` (CONTROL\|ALT excluded by `:427`) | Browse | cursor +1, no wrap, consume | `list.down` `["j","down"]` `down` no | shared |
| `:440` | `k`, `Up` | Browse | cursor -1, consume | `list.up` `["k","up"]` `up` no | shared |
| `:444` `'n'` → `:459` | `n` | Browse; `unavailable` → consume no-op `:445`; `own_write()` busy → notice `:455` | open create form | `common.new` `["n"]` `new` no | shared |
| `:444` `'e'` → `:476` | `e` | as above + no row → `NO_ROW` `:471` | open edit form | `common.edit` `["e"]` `edit` no | shared |
| `:444` `'b'` → `:477` | `b` | as `e` | open body editor | `settings.personas.body` `["b"]` `edit body` no | **new view verb** |
| `:444` `'r'` → `:478` | `r` | as `e` | open rules editor | `settings.personas.rules` `["r"]` `edit rules` no | **new view verb** |
| `:444` `'d'` → `:479-486` | `d` | as `e`, but busy = `self.busy` (import counts) `:450` | ask the delete question | `common.delete` `["d"]` `delete` no | shared |
| `:444` `'I'` → `:464` | `I` (SHIFT dropped by `KeyChord::new`) | `unavailable`; `self.busy` | open the import path | `settings.personas.import` `["I"]` `import` no | **new view verb** |
| `:492` | anything else | Browse | `Pass` | - (global layer) | - |
| `:503-517` | widget keys | Editing | `TextField::on_key` | fixed (D13): `Hint::Text` | - |
| `:1576` (via `:517`) | `Tab`, `Down` — **modifier-blind** (matched before the CONTROL arm at `:1584`: `ctrl-down`, `alt-tab` move focus) | Editing | focus +1, wrap | `form.next_field`, view default `["tab","down"]` (VIEW_DEFAULTS), `next field`, **in_capture** | shared + VIEW_DEFAULTS |
| `:1580` | `BackTab`, `Up` — modifier-blind as above | Editing | focus -1, wrap | `form.prev_field`, view default `["backtab","up"]`, `previous field`, in_capture | shared + VIEW_DEFAULTS |
| `:1584-1585` | CONTROL → Pass; rest → Consumed | Editing | swallow | - (D6 modal: global act → Pass, else consume) | - |
| `:573`/`:581` | `ctrl-s`, `ctrl-S` (TextArea Submit) | Body | `save_body` | widget-fixed (D13) **and** `form.save` `["ctrl-s"]` `save` in_capture dispatched to `save_body` (same as boxes' TextArea editors, D13 sentence) | shared |
| `:585` | `Esc` (TextArea Cancel) | Body | `cancel_body` | fixed: `Hint::Text` | - |
| `:589-590` | CONTROL → Pass; rest Consumed | Body | swallow | - | - |
| `:698`/`:706` | `ctrl-s`/`ctrl-S` | Rules | `save_rules` | as body: `form.save` | shared |
| `:710` | `Esc` | Rules | warn-once/close | fixed | - |
| `:722-723` | CONTROL → Pass; rest Consumed | Rules | swallow | - | - |
| `:785` | `y` — **ALT-blind** (`:775` checks CONTROL only: `alt-y` deletes) | Deleting/Asking; `InFlight` swallows (`:781`) | send `DeletePersona`, stage InFlight | `confirm.yes` `["y"]` `yes` no | shared |
| `:791` | `n`, `Esc` — ALT-blind | Deleting/Asking | back to Browse | `confirm.no` `["n","esc"]` `no` no | shared |
| `:795`/`:797` | anything else | Deleting | consume | - | - |
| `:805-826` | widget keys, `Enter` submit, `Esc` cancel | ImportPath | `TextField` | fixed: `Hint::Text` | - |
| `:825-826` | CONTROL → Pass; rest Consumed | ImportPath | swallow | - | - |
| `:839` | `j`, `Down` — **ALT-blind** (`:832` checks CONTROL only) | Report | top +1, capped at `max_top` | `list.down` | shared |
| `:840` | `k`, `Up` — ALT-blind | Report | top -1 | `list.up` | shared |
| `:841` | `Esc`, `Enter` — ALT-blind | Report | close → Browse (then `open_pending_report` `:1785`) | `common.back`, view default `["esc","enter"]` (VIEW_DEFAULTS), `back` no | shared + VIEW_DEFAULTS |
| `:842`/`:844` | anything else | Report | consume | - | - |

Acceptance (D6): **no personas candidate is ever declined** for state. Every guard (`unavailable`, busy, no row, `InFlight`, `max_top`) is accept-and-consume, as today. Decline-and-fall-through occurs only for global-layer acts (return `Pass`). The `Enter` on `common.back` is a VIEW_DEFAULTS chord, not a state-guarded fall-through: Report has no other `Enter` candidate.

### 1.3 Stacks (`keys/views.rs`)

| Constant | Modes | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| `PERSONAS_BROWSE` | `Browse` (all three hint variants) | `only(SettingsPersonas, {PersonasBody, PersonasRules, PersonasImport, New, Edit, Delete, ListDown, ListUp})`, `all(Settings)`, `only(Common, {New, Edit, Delete})`, `only(List, {ListDown, ListUp})`, `all(Global)` | `"in Settings > Personas"` |
| `PERSONAS_FORM` | `Editing` (create and edit) | `only(SettingsPersonas, {FormNextField, FormPrevField})`, `only(Form, {FormNextField, FormPrevField})`, `modal(Global)` | `"in the Personas form"` |
| `PERSONAS_EDITOR` | `Body` (create and edit), `Rules` | `only(SettingsPersonas, {FormSave})`, `only(Form, {FormSave})`, `modal(Global)` | `"in a Personas body or rules editor"` |
| `PERSONAS_DELETE` | `Deleting` (both stages) | `only(SettingsPersonas, {ConfirmYes, ConfirmNo})`, `all(Confirm)`, `modal(Global)` | `"at the Personas delete question"` |
| `PERSONAS_IMPORT` | `ImportPath` | `modal(Global)` | `"in the Personas import path"` |
| `PERSONAS_REPORT` | `Report` | `only(SettingsPersonas, {ListDown, ListUp, Back})`, `only(List, {ListDown, ListUp})`, `only(Common, {Back})`, `modal(Global)` | `"in the Personas import report"` |

`key_stack()` is a plain match on `self.mode` to these six. `PERSONAS_IMPORT` equals any other text-only stack. T1 may alias it to one shared `FIELD_ONLY` constant, but then its DECLARED phrase must be generic.

Whether `all(Settings)` sits in `PERSONAS_BROWSE` depends on how T1 dispatches section cycling (the tab cycles before delegating, `settings/mod.rs:347-356`). If cycling resolves through the active section's stack, the view layer's `only` set should also admit `NextSection`/`PrevSection`, so that `[settings.personas] next_section` is a legal D10 override. Otherwise drop them. T1 decides this for all ten sections (Q4).

### 1.4 Hints

All ten constants become `pub const …: HintSpec`. `hint()` (`:1173-1192`) becomes `fn hint(&self, keys: &Keys) -> String` rendering `keys.hint(self.key_stack(), SPEC)`, and `render` (`:1863`) passes `ctx.keys()`.

| Const (`file:line`) | Today | HintSpec | Rendered with defaults | Drift |
|---|---|---|---|---|
| `HINT_BROWSE` `:48` | `j/k select · n new · e edit · b body · r rules · d delete · I import` | `Pair(ListDown, ListUp, "select")`, `One(New, "new")`, `One(Edit, "edit")`, `One(PersonasBody, "body")`, `One(PersonasRules, "rules")`, `One(Delete, "delete")`, `One(PersonasImport, "import")` | identical | none |
| `HINT_EMPTY` `:51` | `n new · I import` | `One(New, "new")`, `One(PersonasImport, "import")` | identical | none |
| `HINT_UNAVAILABLE` `:54` | `` | `&[]` | `` | none |
| `HINT_FORM_NEW` `:57` | `Tab/Shift+Tab field · Enter body · Esc cancel` | `Pair(FormNextField, FormPrevField, "field")`, `Text("Enter body")`, `Text("Esc cancel")` | identical (first chords `Tab`, `Shift+Tab`) | none |
| `HINT_FORM_EDIT` `:60` | `Tab/Shift+Tab field · Enter save · Esc cancel` | same, with `Text("Enter save")` | identical | none |
| `HINT_BODY_NEW` `:63` | `Ctrl+S create · Esc back to the fields` | `One(FormSave, "create")`, `Text("Esc back to the fields")` | `Ctrl+s create · Esc back to the fields` | **`Ctrl+S` → `Ctrl+s`** |
| `HINT_EDITOR` `:66` | `Ctrl+S save · Esc cancel · Enter breaks the line` | `One(FormSave, "save")`, `Text("Esc cancel")`, `Text("Enter breaks the line")` | `Ctrl+s save · Esc cancel · Enter breaks the line` | **`Ctrl+S` → `Ctrl+s`** |
| `HINT_DELETING` `:69` | `y delete · n/Esc stop` | `One(ConfirmYes, "delete")`, `All(ConfirmNo, "stop")` | identical | none |
| `HINT_IMPORT` `:72` | `Enter import · Esc cancel` | `Text("Enter import")`, `Text("Esc cancel")` | identical | none |
| `HINT_REPORT` `:75` | `j/k scroll · Esc close` | `Pair(ListDown, ListUp, "scroll")`, `One(Back, "close")` | identical (first chord of the VIEW_DEFAULTS row is `esc`) | none |

Inline literals and prose that name keys (M6, **not** converted here, listed so nothing is missed): module doc `:9-16`; `UNSAVED` `:105` (`Esc again discards`); `CHANGED_ELSEWHERE_SAVE` `:114` (`Ctrl+S retries`, which will then disagree with the hint's `Ctrl+s` until M6, see Q5); `CHANGED_ON_BOTH_SIDES` `:120` (`Enter retries`). `DOT` `:151` and `RULES_HELP` `:87` use `·` but name no key. They are not hints.

### 1.5 Collisions, CONTROL guards

- **Collisions:** none in any personas stack. `r` is `PersonasRules` only (`common.reload` is not admitted). `Tab`/`Down`/`Up`/`Esc`/`Enter` meet no global act, because modal filters them out. Browse has no `form`/`back`. **No SHADOWING or STATE_GUARDED entries needed.**
- **CONTROL guards redundant under D5/D6** (delete them): `:427-432`, `:589`, `:722`, `:775`, `:825`, `:832`, `:1584`. Each becomes "a candidate in the global layer → `Pass`; no candidate → widget (where there is one), then `Consumed`".
- **Ctrl chords the view uses itself:** only `ctrl-s`/`ctrl-S` (TextArea Submit, widget-fixed, D13) → also `form.save`.
- **Behaviour changes by construction** (intended, D5): `F1` in every modal mode now opens help (today swallowed). `ctrl-down`/`ctrl-up`/`alt-tab` in the form no longer move focus. `alt-y` no longer deletes, and `alt-n`/`alt-esc` no longer cancel. `alt-j`/`alt-k`/`alt-enter`/`alt-esc` in the report do nothing.

### 1.6 Tests that pin changing text

`tests/personas.rs:13-16` imports `HINT_BROWSE, HINT_DELETING, HINT_EDITOR, HINT_FORM_EDIT, HINT_REPORT` as `&str`. After the type change to `HintSpec` these uses stop compiling. Replace each with a literal of the rendered text (a test-local `const BROWSE_KEYS: &str = "j/k select · n new · e edit · b body · r rules · d delete · I import";` etc.). That also pins the default rendering:

| `file:line` | Uses | Expected text after |
|---|---|---|
| `tests/personas.rs:201`, `:458`, `:1032`, `:1215`, `:1262`, `:1369` (`!contains`), `:1412` | `HINT_BROWSE` | unchanged text |
| `tests/personas.rs:424` | `HINT_FORM_EDIT` | unchanged |
| `tests/personas.rs:556` | `HINT_EDITOR` | `Ctrl+s save · Esc cancel · Enter breaks the line` (**changes**) |
| `tests/personas.rs:1012` | `HINT_DELETING` | unchanged |
| `tests/personas.rs:1202`, `:1258`, `:1297` | `HINT_REPORT` | unchanged |
| `tests/personas.rs:290` | message text `"nothing is sent before Ctrl+S"` | message only, no change needed |

Snapshots (`crates/htui/tests/snapshots/`):

| Snapshot | Row | Change |
|---|---|---|
| `personas__body.snap` | 34 | `Ctrl+S save · Esc cancel · Enter breaks the line` → `Ctrl+s save · Esc cancel · Enter breaks the line` |
| `personas__rules.snap` | 34 | same |
| `personas__browse`, `__form`, `__delete_ask`, `__import_report` | 34 | none |
| `personas__offline` (App-level; last row is the refused read's status, which gives way on the first key) | - | none expected. If D7 alters the status row, only that row may change |

No in-file tests in `personas.rs`. Other lanes' files: `tests/settings.rs:1104` only constructs `PersonasSection` and counts title width, so no change there.

### 1.7 Defect pins (new tests, `tests/personas.rs`, SectionBench)

1. `alt_y_at_the_delete_question_deletes_nothing`: `d`, then `alt-y` → no `DeletePersona` request, still asking (`captures_input()`), then `y` sends. Pins `:775`/`:785`.
2. `alt_enter_and_alt_esc_keep_the_report_open`: open the report (`mixed_report`), `alt-esc` and `alt-enter` leave it open, `esc` closes. Pins `:832`/`:841`.
3. (optional, low value) `ctrl_down_in_the_form_keeps_the_focus`: pins `:1576` being modifier-blind.
4. `f1_in_the_body_editor_is_not_typed_and_is_passed`: `bench.key("f1") == Handled::Pass` (today `Consumed`, `:590`). Pins D5 for a TextArea mode.

Rebinding test (`Harness::with_keys`, App-level, `#![cfg(feature = "testkit")]` already on the file): keys `[settings.personas] import = ["i"]`, over `SettingsTab::with_sections(vec![PersonasSection])` on `Harness::demo()`. Assert:
- the Browse hint reads `… · d delete · i import`;
- `I` changes nothing on screen (the `offline` test's `pane` comparison pattern);
- `i` opens the path field, and the frame shows `Enter import · Esc cancel`.

Optional second check, the same harness with `[form] next_field = ["ctrl-n"]`: whether `ctrl-n` moves the form focus depends on Q2.

---

## 2. `crates/htui/src/ui/tabs/settings/secrets.rs`

### 2.1 Modes

`captures_input()` (`:1372-1374`) is `!matches!(mode, Browse)`. Every other mode is modal.

| Mode (`:231-270`) | captures | modal | widget | widget-fixed keys (D13) | view keys today |
|---|---|---|---|---|---|
| `Browse`, snapshot present (`HINT_BROWSE`) | no | no | - | - | `e c t j k Down Up r`, `Esc` while a notice shows (`:1413-1426`). **No modifier guard at all** |
| `Browse`, no snapshot or read refused (`HINT_NO_SNAPSHOT`, `:1089`) | no | no | - | - | same arms, same behaviour (`e`/`c` silent on URL/Identity via `keyring_blocked` `:471`). Only the hint differs |
| `EditingUrl(TextField)` | yes | yes | one `TextField` | chars, editing keys, `Enter` (Submit → `submit` `:664` → `submit_url` `:710`), `Esc` (Cancel → Browse `:666`) | `Tab`/`Down`/`BackTab`/`Up` reach `cycle_focus` (`:674-675`), which is a no-op for this mode (`:687`) → consumed |
| `EditingIdentity { focus 0..2 }` | yes | yes | `client_id` `TextField`, `client_secret` `TextField::masked` | same; `Enter` → `submit_identity` `:732` | `Tab`/`Down` next, `BackTab`/`Up` prev (`:674-675`); CONTROL passes (`:670`); rest swallowed |
| `EditingScope { focus 0..3 }` | yes | yes | three `TextField`s | same; `Enter` → `submit_scope` `:761` (stays open until its reply) | same |
| `ConfirmClearUrl`, `ConfirmClearIdentity`, `ConfirmClearScope` | yes | yes | - | - | `y` (`:799-826`), `n`/`Esc` (`:827`); CONTROL passes (`:795`); rest swallowed |

The 2 CONTROL checks: `:670` (forms) and `:795` (confirm). Both become redundant.

### 2.2 Key-match sites → actions

| Site | Chord(s), modifiers | State guard | Behaviour | → (context, act, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|
| `:1414` | `e`, **modifier-blind** (`ctrl-e`, `alt-e` edit) | Browse; per row in `edit()` `:494-546` (Provider/Health refuse, keyring blocked silent, demo refuse, busy refuse on a project) | open the URL / identity / scope form | `common.edit` `["e"]` `edit` no | shared |
| `:1415` | `c`, modifier-blind (`alt-c`; `ctrl-c` never arrives, `App::on_key` takes it first) | Browse; per row in `clear()` `:549-606` | open a question or refuse | `common.clear` `["c"]` `clear` no | shared |
| `:1416` | `t`, modifier-blind (**`ctrl-t` sends a provider check, i.e. a login that can latch**, `:105`) | Browse; demo / checking / no scope refuse in `test()` `:609-640` | `CheckSecretProvider` / `CheckSecretScope` | `settings.secrets.check` `["t"]` `check` no | **new view verb** |
| `:1417` | `j`, `Down`, modifier-blind | Browse | cursor +1, no wrap (`:434-440`) | `list.down` | shared |
| `:1418` | `k`, `Up`, modifier-blind | Browse | cursor -1 | `list.up` | shared |
| `:1420-1423` | `r`, modifier-blind (`ctrl-r`) | Browse; never refused | `SecretsInfo` + `SecretsTree` | `common.reload` `["r"]` `reload` no | shared |
| `:1424` | `Esc` **if `notice.is_some()`**, modifier-blind (`alt-esc`) | Browse | clear the notice | `common.dismiss` `["esc"]` `dismiss` no; **declined (→ `Pass`) when there is no notice**, as today's guard falls to `:1425` | shared, state-declined |
| `:1425` | anything else | Browse | `Pass` | - | - |
| `:647-659` | widget keys, `Enter` Submit, `Esc` Cancel | forms | `TextField::on_key` | fixed: `Hint::Text` | - |
| `:670-672` | CONTROL → `Pass` | forms | - | - (D6 modal) | - |
| `:674` | `Tab`, `Down`, **ALT-blind** (`alt-tab`, `alt-down` move the focus) | Identity, Scope (URL: no-op) | focus +1, wrap | `form.next_field`, VIEW_DEFAULTS `["tab","down"]`, `next field`, in_capture | shared + VIEW_DEFAULTS |
| `:675` | `BackTab`, `Up`, ALT-blind | Identity, Scope | focus -1, wrap | `form.prev_field`, VIEW_DEFAULTS `["backtab","up"]`, `previous field`, in_capture | shared + VIEW_DEFAULTS |
| `:676`/`:679` | anything else | forms | consume | - | - |
| `:799` | `y` while `closed_if_gone()` | confirm | the question closed itself; consume | `confirm.yes` (accepted, no write) | shared |
| `:800-826` | `y`, **ALT-blind** (`:795` checks CONTROL only: `alt-y` clears) | confirm | send the clear | `confirm.yes` `["y"]` `yes` no | shared |
| `:827` | `n`, `Esc`, ALT-blind | confirm | back to Browse | `confirm.no` `["n","esc"]` `no` no | shared |
| `:828`/`:830` | anything else | confirm | consume | - | - |

The only state-declined candidate is `common.dismiss` with no notice. It needs no STATE_GUARDED pair, because `common.back` is not in the stack.

### 2.3 Stacks (`keys/views.rs`)

| Constant | Modes | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| `SECRETS_BROWSE` | `Browse` (both hint variants) | `only(SettingsSecrets, {SecretsCheck, Edit, Clear, Reload, Dismiss, ListDown, ListUp})`, `all(Settings)`, `only(Common, {Edit, Clear, Reload, Dismiss})`, `only(List, {ListDown, ListUp})`, `all(Global)` | `"in Settings > Secrets"` |
| `SECRETS_URL` | `EditingUrl` | `modal(Global)` | `"in the Secrets URL field"` |
| `SECRETS_FORM` | `EditingIdentity`, `EditingScope` | `only(SettingsSecrets, {FormNextField, FormPrevField})`, `only(Form, {FormNextField, FormPrevField})`, `modal(Global)` | `"in a Secrets form"` |
| `SECRETS_CONFIRM` | the three `ConfirmClear*` | `only(SettingsSecrets, {ConfirmYes, ConfirmNo})`, `all(Confirm)`, `modal(Global)` | `"at a Secrets question"` |

`SECRETS_URL` keeps today's behaviour: `Tab`/`Down` in the URL field get no candidate, the field passes them, and the view consumes them (today: a `cycle_focus` no-op). If T1 prefers one shape, `SECRETS_FORM` would also work there (focus cycling is a no-op on one field), but its hint must not advertise `Tab`. Recommend `SECRETS_URL` as written. The same Q4 note on `NextSection`/`PrevSection` applies to `SECRETS_BROWSE`.

### 2.4 Hints

The six constants become `HintSpec`s (private is fine, nothing outside reads them). `hint_text()` (`:1080-1102`) becomes `hint_text(&self, keys: &Keys) -> String` = `keys.hint(self.key_stack(), SPEC)`, then the existing in-flight suffix. `hint()` (`:1059`) gains `keys: &Keys`.

| Const (`file:line`) | Today | HintSpec | Rendered with defaults | Drift |
|---|---|---|---|---|
| `HINT_BROWSE` `:130` | `e edit · c clear · t check · r reload · j/k rows` | `One(Edit, "edit")`, `One(Clear, "clear")`, `One(SecretsCheck, "check")`, `One(Reload, "reload")`, `Pair(ListDown, ListUp, "rows")` | identical | none |
| `HINT_NO_SNAPSHOT` `:132` | `r reload` | `One(Reload, "reload")` | identical | none |
| `HINT_URL` `:134` | `Enter store · Esc cancel` | `Text("Enter store")`, `Text("Esc cancel")` | identical | none |
| `HINT_IDENTITY` `:136-137` | `Tab next field · Enter store · Esc cancel · the secret is never shown` | `One(FormNextField, "next field")`, `Text("Enter store")`, `Text("Esc cancel")`, `Text("the secret is never shown")` | identical | none |
| `HINT_SCOPE` `:139` | `Tab next field · Enter save · Esc cancel` | `One(FormNextField, "next field")`, `Text("Enter save")`, `Text("Esc cancel")` | identical | none |
| `HINT_CONFIRM` `:141` | `y confirm · n / Esc cancel` | `One(ConfirmYes, "confirm")`, `All(ConfirmNo, "cancel")` | `y confirm · n/Esc cancel` | **`n / Esc` → `n/Esc`** (same drift as connection.rs:65 and qdrant.rs:33 in L-A/L-B) |

Inline literals around hints: `:1074` `format!("{keys} · ")` (notice joiner) and `:1098` `format!("{keys} · {} in flight", …)` keep their ` · ` and compose with the rendered string. Prose naming keys (M6, not converted): `NOTHING_TO_EDIT` `:93` (`t checks`), `HEALTH_GUIDE` `:105` (`t logs in`), `LATCHED` `:110` (`e on Identity`), `CONFIRM_CLEAR_URL` `:112`, `CONFIRM_CLEAR_IDENTITY` `:114`, `confirm_clear_scope` `:148` (all end `y / n`), `EMPTY_URL` doc `:85` (`clearing is c`).

### 2.5 Collisions, CONTROL guards

- **Collisions:** none. In `SECRETS_BROWSE`, `Esc` is `Dismiss` alone. `t`/`e`/`c`/`r` meet nothing in `settings` or `global`. Modal stacks filter `Tab`/`Down`/`Up` out of global. No SHADOWING or STATE_GUARDED entries.
- **Redundant CONTROL guards:** `:670-672`, `:795-797`. Also, Browse `on_key` (`:1413`) gains modifier-correctness for free: it had no guard.
- **Ctrl chords the view uses itself:** none.
- **Behaviour changes by construction:**
  - Browse: `ctrl-e`/`alt-e`, `alt-c`, `ctrl-t`/`alt-t`, `ctrl-r`, `ctrl-j`/`ctrl-k`/`ctrl-down`/`ctrl-up` and `alt-esc` stop acting.
  - Forms: `alt-tab`/`alt-down`/`alt-up` stop moving the focus.
  - Questions: `alt-y`/`alt-n` stop answering.
  - Every modal mode: `F1` opens help.

  This is ANA §2.6 defect 1 (modifier-blind browse arms) in a section ANA did not inventory. **The plan's D14 pin list should name it** beside the hierarchy/kinds `ctrl-d` pin.

### 2.6 Tests that pin changing text

- **In-file** `secrets.rs:1561-1572` `every_hint_fits_the_section` iterates the `&str` constants. Rewrite it to render each `(stack, spec)` pair through `Keys::compiled().hint(..)` and assert `cell_width ≤ 98`.
- **In-file** `secrets.rs:1576-1593` `a_wide_notice_takes_the_hint_line_alone` calls `hint_text()` and `hint(width, theme)`. Pass `Keys::compiled()` to both. Assertions unchanged.
- `tests/secrets_settings.rs:2497` (`Some("r reload")`) and `:2824` (`"e edit · c clear · t check · r reload · j/k rows"`, App-level registration test): text unchanged, stays green.
- `tests/secrets_settings.rs:2587` (`"set_machine_identity in flight"`): the suffix is unchanged.
- No test pins `n / Esc cancel` for secrets.
- Snapshots: all seven `secrets_settings__*` are **unchanged** (none shows a question). `identity_form` row 34 stays `Tab next field · Enter store · Esc cancel · the secret is never shown`.
- Other lanes: `tests/settings.rs:1105` constructs only. No change.

### 2.7 Defect pins (new tests, `tests/secrets_settings.rs`, SectionBench)

1. `ctrl_t_in_browse_sends_no_check`: on a keyring row, `ctrl-t` → `requests(&bench)` empty, Health row unchanged. Then `t` → one `CheckSecretProvider`. Pins `:1416`.
2. `ctrl_e_and_alt_c_in_browse_open_nothing`: `ctrl-e` and `alt-c` on the URL row → `!captures_input()`, no question on screen. `bench.key(..)` returns `Handled::Pass`. Pins `:1414-1415`.
3. `alt_y_at_a_question_clears_nothing`: `c` on Identity, `alt-y` → no `ClearMachineIdentity`, still asking. Then `y` sends. Pins `:795`/`:800`.
4. `esc_with_no_notice_passes`: Browse, no notice → `bench.key("esc") == Handled::Pass`. With a notice → `Consumed` and the notice is gone. Pins the `Dismiss` decline.

Rebinding test (`Harness::with_keys`): keys `[settings.secrets] check = ["T"]`, `Harness::demo()` + `register_all`, `4`, then 8 × `l` (as `the_product_registers_secrets_last` does). Assert:
- the hint reads `e edit · c clear · T check · r reload · j/k rows`;
- `t` leaves the pane unchanged (no notice);
- `T` shows `DEMO_CHECK` ("a demo session has no secret provider to check"). This proves the rebound chord reached `test()` without needing a provider.

---

## 3. Open questions for the foundation (T1), each with a proposed answer

- **Q1. View-layer leakage of override rows.** A `VIEW_DEFAULTS` row and a user's `[settings.<section>] <shared name>` are rows in the view context. `Layer::all(view)` admits them in *every* mode of the view. Examples:
  - personas Browse would resolve `Down` to `FormNextField` before `ListDown`;
  - its `?` box would list `Tab/Down next field` and `Esc/Enter back`.

  *Proposal:* every `views.rs` stack opens with `Layer::only(view, <acts the mode dispatches>)`, as in sections 1.3 and 2.3. D10's legality check ("a shared context below the view context in some declared stack") tests that the act is **admitted** by that view layer and by a shared layer below it. Add a unit test that every `VIEW_DEFAULTS` row is admitted by at least one declared stack's view layer.
- **Q2. `VIEW_DEFAULTS` shadows a user's shared rebinding.** With `[form] next_field = ["ctrl-n"]`, the personas and secrets (and agents and hierarchy) forms keep `Tab`/`Down`, because their view row shadows `form`. *Proposal:* mark `VIEW_DEFAULTS` rows as derived. When the key file sets the shared `(context, act)` and not the view's own entry, the loader drops the derived rows for that act, so the user's shared binding reaches every view. Pin it with an App-level test: secrets identity form, `ctrl-n` moves the focus and `Tab` does not.
- **Q3. `form.save` in TextArea editors.** *Proposal:*
  - `PERSONAS_EDITOR` offers `form.save`, and the view dispatches it to `save_body`/`save_rules` before the widget sees the chord;
  - the widget's `ctrl-s`/`ctrl-S` Submit stays as a fixed fallback (D13);
  - the hints use `One(FormSave, …)`, so they drift to `Ctrl+s`, the same call as boxes (plan claim 20).

  L-B must make the same call for boxes, or the two lanes' hints disagree.
- **Q4. Section cycling and the section's stack.** If `SettingsTab` resolves `settings.next_section`/`prev_section` through the active section's stack, each Browse view layer's `only` set must admit `NextSection`/`PrevSection`. Otherwise `[settings.personas] next_section = …` is not overridable. *Proposal:* the tab resolves through the section stack, and every section's Browse `only` set includes both. Uniform across the ten sections.
- **Q5. Prose drifting from hints.** Once the hints say `Ctrl+s`, `personas.rs:114` `CHANGED_ELSEWHERE_SAVE` still says `Ctrl+S retries`. *Proposal:* leave it to M6, per the plan's prose rule, and record it in the lane report and in M6's backlog.
- **Q6. Overrides that steal widget keys.** D6 resolves actions before the widget. So a user can write `[settings.personas] next_field = ["left"]`: it passes the printable check and steals `Left` from the `TextField` cursor. *Proposal:* the validator refuses binding an `in_capture` act to `enter`, `esc`, `left`, `right`, `home`, `end`, `backspace` or `delete`. Those keys are fixed widget keys everywhere (ANA §6.1). `up`/`down`/`pgup`/`pgdn` stay allowed, because `TextField` forms use `Up`/`Down` for focus.
- **Q7. Section-level rebinding.** `SectionBench::key` (`testkit.rs:706-710`) builds a `Ctx` with the compiled keys, so section tests cannot rebind. *Proposal:* T1 adds `SectionBench::with_keys(Keys)` beside `Harness::with_keys`. Lanes then pin rebinding at the section level as well, and keep the App-level test for the hint and the shell's modal filter.
- **Q8. The personas Report stays modal.** `captures_input()` is true for `Report` (R-10). Under `Layer::modal`, `q`/`Tab`/digits stay swallowed as today, and `F1` now opens help. *Proposal:* keep it. Do not make Report non-capturing in M3, because that would change `h`/`l` (section cycling) over the report.
- **Q9. Naming.** View acts are proposed as `Act::PersonasBody/PersonasRules/PersonasImport/SecretsCheck`, contexts as `Context::SettingsPersonas/SettingsSecrets` (ANA §7.2 spells `SettingsAgents`). T1 fixes one convention for all lanes (`Agents*` or `SettingsAgents*`). `?` box headings: `Personas`, `Secrets`.
- **Q10. Plan D14 pins.** Add the secrets Browse modifier-blind pins (section 2.7, items 1 and 2) and the two `alt-y` question pins (sections 1.7 and 2.7) to D14's list, beside hierarchy/kinds `ctrl-d`.

## 4. Lane L-D checklist (for the implementer)

1. Add the imports `crate::keys::{Act, Hint, HintSpec, Keys, views::{PERSONAS_*, SECRETS_*}}`. Drop `KeyCode`/`KeyModifiers` from both files, except the `secrets.rs` test at `:1624`, which keeps `KeyCode` for `TextField::on_key`.
2. Implement `key_stack()` per mode in both sections (sections 1.3 and 2.3).
3. Make each `on_*_key` one `for act in ctx.keys().actions(stack, chord)` loop:
   - accept per section 1.2 / 2.2;
   - a global-layer act → `Pass`;
   - in modal modes, the widget gets the rest, then `Consumed`;
   - delete the 9 CONTROL guards (7 + 2).
4. Convert the 16 hint constants (10 + 6) to `HintSpec`, and render them through `ctx.keys()`.
5. Tests:
   - `tests/personas.rs` imports → literals (section 1.6);
   - the secrets in-file tests (section 2.6);
   - the defect pins (sections 1.7 and 2.7);
   - the two rebinding tests.
6. `cargo insta test -p htui --all-features --check`: exactly two snapshot rows change, `personas__body` and `personas__rules` row 34 (`Ctrl+S` → `Ctrl+s`). Anything else is a defect.
