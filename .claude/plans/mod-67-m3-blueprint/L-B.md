# MOD-67 M3 blueprint — Lane L-B key inventory

Scope: `crates/htui/src/ui/tabs/settings/{connection,boxes,prompt,queue}.rs`; tests `crates/htui/tests/{connection,box_settings,prompt_settings,queue_settings}.rs` and their snapshots.

Sections: 0 lane-wide conventions · 1 connection · 2 boxes · 3 prompt · 4 queue · 5 cross-file summary · 6 open questions.

## 0. Lane-wide conventions (apply to all four files)

**New catalogue rows L-B needs (T1 lands them; nothing else in L-B needs a new act):**

| Context (TOML table) | Act | name | defaults | help | in_capture | mirrors |
|---|---|---|---|---|---|---|
| `settings.connection` | `ConnectionRebuild` | `rebuild` | `["R"]` | `rebuild cache` | no | `connection.rs:802` |
| `settings.connection` | `ConnectionActivate` | `activate` | `["enter"]` | `run row` | no | `connection.rs:807` (Rebuild row only; STATE-dependent decline) |
| `settings.boxes` | `BoxesEditTags` | `edit_tags` | `["t"]` | `tags` | no | `boxes.rs:703` |
| `settings.boxes` | `BoxesEditQuirks` | `edit_quirks` | `["e"]` | `quirks` | no | `boxes.rs:707` |
| `settings.boxes` | `BoxesExecutor` | `executor` | `["w"]` | `executor` | no | `boxes.rs:712` (declines with no listed box -> `global.workspaces`) |
| `settings.boxes` | `BoxesProbe` | `probe` | `["p"]` | `probe` | no | `boxes.rs:727` |
| `settings.boxes` | `BoxesEditSpec` | `edit_spec` | `["s"]` | `spec` | no | `boxes.rs:723` |
| `settings.prompt` | — (no view verb) | | | | | |
| `settings.queue` | — (no view verb) | | | | | |

`VIEW_DEFAULTS` entry L-B needs: `(Context::SettingsQueue, Act::Edit, &["e", "enter"])` (`queue.rs:617`, `e`|`Enter` both open the editor). Help labels of D2-fixed acts (`rebuild`, `probe`, `edit_quirks`, `edit_tags`) are taken from the current hint words (`rebuild cache`, `probe`, `quirks`, `tags`) so a `?` box entry reads like the hint.

**Shared-act usage (no new rows):** `common.edit` (connection `e`, prompt `e`, queue `e`), `common.clear` (connection `c`), `common.reload` (all four `r`), `common.dismiss` (all four `Esc` when a notice is shown), `list.down`/`list.up` (all four `j`/`Down`, `k`/`Up`), `confirm.yes`/`confirm.no` (connection's two questions, boxes' executor flip), `form.save` (boxes quirks + spec editors only, see D13 note).

**Dispatch skeleton (D6), identical in all four files:**
- Browse: `for act in ctx.keys().actions(STACK, chord)` → handle the first act the view accepts; a global act or a declined one → `Handled::Pass`. No candidate → `Handled::Pass` (today's `_ => Handled::Pass`).
- Capturing / confirming (modal) mode: resolve first; a view act (`confirm.*`, `form.save`) is handled; a **global** candidate → `Pass`. With no candidate: a text mode feeds the widget (`TextField::on_key` / `TextArea::on_key`) and maps `Consumed`/`Submit`/`Cancel` as today; the widget's `FieldOutcome::Pass` (and, in a confirm mode, any unlisted key) → **`Pass` iff the modal global layer's chord filter admits the chord (CONTROL or ALT, or an F-key), else `Consumed`** — bound or not. That keeps `bench.key(.., "ctrl-c") == Handled::Pass` pins (`box_settings.rs:1168`, `:1395`, `:1922`) green at section level, where no shell checks `ctrl-c` first; the shell then resolves the global act or does nothing. **T1 must expose that filter** (e.g. `Layer::admits_chord(chord)` / `KeyChord::passes_modal()`), see Open questions Q1.
- `hint_text(&self)` → `hint_text(&self, keys: &Keys)` (or `&Ctx`) in all four files; `render` passes `ctx.keys()`; in-file tests pass `Keys::compiled()` (`connection.rs:1034`, `prompt.rs:1273`; boxes/queue have no in-file hint test). The `{busy} in flight` / `probing…` / `saving…` suffixes stay string appends on the rendered hint.
- Every key-naming **prose** constant stays as-is (M6): `connection.rs` `NO_DSN_YET`:85, `REPLACES`:92, `UNREADABLE_GUIDE`:113, `STORED_UNREADABLE`:118 (`e replaces it`), `REBUILD_KEYS`:127 (`press Enter or R`), `CONFIRM_CLEAR`:131 (`y / n`), `confirm_rebuild()`:141 (`y / n`); `boxes.rs` `SPEC_SAVED`:92 (`the next p`), `CHANGED_ELSEWHERE_QUIRKS`:124 (`ctrl-s retries`), `CHANGED_ELSEWHERE_EXECUTOR`:128 (`y retries`); `prompt.rs` `NOT_A_VALUE_ROW`:77 and `queue.rs` `NOT_A_VALUE_ROW`:57 (`` `e` edits a value row ``); `settings/mod.rs` `CHANGED_ELSEWHERE`:106 (`Enter retries`, a widget key). Pinned by `prompt_settings.rs:1744`, `box_settings.rs:1002`/`:1715` (`ctrl-s retries`) — unchanged in M3.

**Shared modal stacks (proposal, see Q2):** the four sections' single-field editors and the two confirmations have identical layer shapes, so `keys/views.rs` can declare them once:
- `SETTINGS_FIELD = [global(modal)]` — "in a Settings text field" (connection Editing, prompt Editing, queue Editing, boxes Tags).
- `SETTINGS_CONFIRM = [confirm, global(modal)]` — "in a Settings confirmation" (connection ConfirmClear/ConfirmRebuild, boxes Executor; the kinds/hierarchy/qdrant/personas confirms can share it if their shapes match).
- `BOXES_TEXT_AREA = [form∩{save}, global(modal)]` — "in the Boxes quirks or probe spec editor" (boxes Quirks, Spec).
If the foundation prefers one stack per mode for error phrasing, the per-mode names are given in each file's stack table; the slices are the same.

## 1. `crates/htui/src/ui/tabs/settings/connection.rs` (title `Connection`, id `connection`)

### 1.1 Modes (`enum Mode`, `connection.rs:211`)

| Mode | `captures_input` (`:747-751`) | Modal? | Text widget | Widget-fixed keys (D13) |
|---|---|---|---|---|
| `Browse` | false | no | — | — |
| `Editing(Editor)` | true | yes: widget first, unlisted keys swallowed, CONTROL passes (`:428-429`) | `TextField::masked()` (`:366`, `:465`) | printable chars, Backspace/Delete/Left/Right/Home/End, `Enter` = submit (`:418`), `Esc` = cancel (`:423`) |
| `ConfirmClear` | true | yes: unlisted swallowed, CONTROL passes (`:478-480`) | — | — |
| `ConfirmRebuild { stage: Asking \| InFlight }` | true | yes (same fn) | — | — (InFlight: `y`/`n`/`Esc` are inert but still consumed, `:500`) |

Dispatch entry `on_key` `:767-773` routes modes; `on_editor_key` `:411-430`; `on_confirm_key` `:477-504`.

### 1.2 Key-match sites → actions

| file:line | chord(s) today | modifiers checked? | state guard | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|---|
| `:779` | `e` | **no** (modifier-blind: `ctrl-e`/`alt-e` open the editor) | `!blocked()` else refusal/no-op, always Consumed | open masked DSN field | `common.edit` `["e"]` "edit", no | shared |
| `:785` | `c` | **no** | `!blocked()`; stored → ConfirmClear, else refusal notice; always Consumed | ask to clear DSN | `common.clear` `["c"]` "clear", no | shared |
| `:802` | `R` | **no** | none (rebuild() refuses internally); Consumed | open rebuild question | `settings.connection.rebuild` `["R"]` "rebuild cache", no | view (D2) |
| `:807` | `Enter` | **no** | **`self.row() == Row::Rebuild`**, else falls to `_ => Pass` | same `rebuild()` | `settings.connection.activate` `["enter"]` "run row", no — view **declines** off the Rebuild row → `Pass` | view (new) |
| `:811` | `j`, `Down` | **no** | none | cursor down | `list.down` `["j","down"]` "down", no | shared |
| `:815` | `k`, `Up` | **no** | none | cursor up | `list.up` `["k","up"]` "up", no | shared |
| `:822` | `r` | **no** | none (always allowed) | `ConnectionInfo` read | `common.reload` `["r"]` "reload", no | shared |
| `:828` | `Esc` | **no** | **`self.notice.is_some()`**, else `Pass` | clear notice | `common.dismiss` `["esc"]` "dismiss", no — declines with no notice | shared |
| `:483` | `y` (ConfirmClear) | CONTROL only (`:478`); ALT-blind (`alt-y` answers yes) | — | send `ClearDsn`, Browse | `confirm.yes` `["y"]` "yes", no | shared |
| `:487` | `n`, `Esc` (ConfirmClear) | CONTROL only | — | back to Browse | `confirm.no` `["n","esc"]` "no", no | shared |
| `:491` | `y` (ConfirmRebuild) | CONTROL only | `stage == Asking`; InFlight → accepted-and-ignored (Consumed, not declined) | stage InFlight, send `RebuildCache` | `confirm.yes` | shared |
| `:495` | `n`, `Esc` (ConfirmRebuild) | CONTROL only | `stage == Asking`; InFlight inert | back to Browse | `confirm.no` | shared |
| `:417-429` | TextField outcomes | `:428` CONTROL passes, rest swallowed | — | widget | fixed (D13); not catalogue | widget |

Tab-level keys reaching this section's stack (handled by `SettingsTab::on_key`, `settings/mod.rs:347`/`:351`, before delegate): `settings.next_section` `["l","]","right"]`, `settings.prev_section` `["h","[","left"]` (T1).

### 1.3 Stacks

| Mode | Stack const (`keys/views.rs`) | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| Browse | `CONNECTION_BROWSE` | `Layer::all(SettingsConnection)`, `Layer::all(Settings)`, `Layer::only(Common, &[Edit, Clear, Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | "in Settings > Connection" |
| Editing | `SETTINGS_FIELD` (or `CONNECTION_EDITOR` with the same slice) | `Layer::modal(Global)` | "in a Settings text field" (or "in the Connection DSN field") |
| ConfirmClear, ConfirmRebuild | `SETTINGS_CONFIRM` (or `CONNECTION_CONFIRM`) | `Layer::all(Confirm)`, `Layer::modal(Global)` | "in a Settings confirmation" (or "in a Connection confirmation") |

`key_stack()` = match on `self.mode`. The browse stack is the same whether or not a snapshot exists (only the hint differs).

### 1.4 Hints

| Site | Today (exact) | `HintSpec` | Rendered with defaults | Differs? |
|---|---|---|---|---|
| `HINT_BROWSE` `:52-53` | `e edit DSN · c clear DSN · R rebuild cache · r reload · j/k rows` | `[One(Edit,"edit DSN"), One(Clear,"clear DSN"), One(ConnectionRebuild,"rebuild cache"), One(Reload,"reload"), Pair(ListDown,ListUp,"rows")]` via `CONNECTION_BROWSE` | `e edit DSN · c clear DSN · R rebuild cache · r reload · j/k rows` | no (this is ANA §7.6's example verbatim) |
| `HINT_NO_SNAPSHOT` `:59` | `r reload` | `[One(Reload,"reload")]` via `CONNECTION_BROWSE` | `r reload` | no |
| `HINT_EDITING` `:62` | `Enter store · Esc cancel · typed text is never shown` | `[Text("Enter store"), Text("Esc cancel"), Text("typed text is never shown")]` | identical | no |
| `HINT_CONFIRM` `:65` | `y confirm · n / Esc cancel` | `[One(ConfirmYes,"confirm"), All(ConfirmNo,"cancel")]` via `SETTINGS_CONFIRM` | `y confirm · n/Esc cancel` | **yes**: `n / Esc` → `n/Esc` |
| `hint_text` busy suffix `:652` | `{keys} · {busy} in flight` | unchanged string append | unchanged | no |

`Enter` (`activate`) is not in the hint today and stays out (no drift); the `?` box lists it.

### 1.5 Collisions
None. `CONNECTION_BROWSE` chords: `R`, `enter` / `l ] right h [ left` / `e c r esc` / `j down k up` / global `q tab backtab 1-9 ? f1 w ctrl-f ctrl-w` — pairwise disjoint. `SETTINGS_CONFIRM`: `y`, `n`, `esc` vs modal global (`ctrl-f`, `ctrl-w`, `f1`) — disjoint. `R` ≠ `r` (strict chords).

### 1.6 CONTROL guards made redundant
- `:428` `FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass` → replaced by the lane-wide "pass iff the modal global filter admits it" rule (adds ALT and F-keys: `F1` opens help from the DSN field; `alt-x` reaches the global layer, unbound by default).
- `:478-480` `if key.modifiers.contains(KeyModifiers::CONTROL) { return Handled::Pass; }` → same rule. Fixes ALT-blindness: today `alt-y` answers the clear/rebuild question.
- Browse arms `:779-831` match `key.code` only: `ctrl-e` opens the field, `alt-c` asks to clear, `ctrl-r` reloads, `alt-R` asks to rebuild. Resolver chord equality makes all of these inert. (`ctrl-c` clear-DSN was already closed by M1's `ctrl-c`-first check, pinned by `tests/keys.rs:74`.)
- `KeyModifiers` import (`:43`) goes; `KeyCode` stays only in the in-file test (`:954`).
- The view itself uses no ctrl chord.

### 1.7 Tests and snapshots touched
- `crates/htui/tests/snapshots/connection__confirm.snap:34` `y confirm · n / Esc cancel` → `y confirm · n/Esc cancel`. All other connection snapshots (`editor`, `empty`, `stored`, `unreadable`) unchanged (row 9/10 `press Enter or R` is M6 prose).
- Substring pins that stay green: `tests/connection.rs:1486` `e edit DSN`, `:1571` `Enter store`, `:1976` `typed text is never shown`, `:1382` `…set_dsn in flight`; `tests/keys.rs:98` (T1's file) `Rebuild cache` / `typed text is never shown`.
- In-file test `a_wide_notice_takes_the_hint_line_alone` `:1026-1044` calls `hint_text()`/`hint()` → pass `Keys::compiled()` after the signature change.
- Other lanes' files registering Connection: `tests/settings.rs` (L-A, all ten), `tests/connection.rs:1172` registers Agents/Hierarchy/Kinds/Prompt/Connection (L-B owns it; Agents/Hierarchy/Kinds conversion by L-A/L-C can move their hint rows in frames this file renders, but no assertion here reads them).

### 1.8 Pins (D14) for connection
- Optional defect pin (cheap, same bench): `ctrl-e` in Browse opens no field (`!captures_input()`, returns `Pass`); `alt-y` in ConfirmClear sends nothing and keeps the question.
- **Lane rebinding test (proposed home: `tests/connection.rs`)**: `Keys` from `htui::keys::load_str("version = 1\n[settings.connection]\nrebuild = \"X\"\n")`, `Harness::over_backend(Offline{cache})` set up as `tests/keys.rs:74-95` (mock keyring + stored DSN so no redirect) `.with_keys(keys)`, focus Connection: `R` → no question (`!frame.contains("Rebuild the mirror?")`), `X` → question shown, hint row reads `e edit DSN · c clear DSN · X rebuild cache · r reload · j/k rows`; `Enter` on the Rebuild row still asks (separate `activate` act).

## 2. `crates/htui/src/ui/tabs/settings/boxes.rs` (title `Boxes`, id `boxes`)

### 2.1 Modes (`enum Mode`, `boxes.rs:162`)

| Mode | `captures_input` (`:668-670`) | Modal? | Text widget | Widget-fixed keys (D13) |
|---|---|---|---|---|
| `Browse` | false | no | — | — |
| `Tags(Editor<TextField, _>)` | true | yes: widget first; `FieldOutcome::Pass` + CONTROL → `Pass`, else swallowed (`:533-534`) | `TextField::with_text` (`:411`) | printable, editing/cursor keys, `Enter` = submit (`:522`), `Esc` = cancel (`:526`) |
| `Quirks(Editor<TextArea, _>)` | true | yes (same fn) | `TextArea` (`:428`), page `QUIRKS_HEIGHT` (`:517`) | printable, `Enter` = newline, Up/Down/PgUp/PgDn/Home/End etc., `Esc` = cancel, **`ctrl-s`/`ctrl-S` = submit** (`text_area.rs:194`, D13: stays) |
| `Spec(SpecEditor)` | true | yes (same fn) | `TextArea` (`:468`), page `SPEC_PAGE` (`:518`) | as Quirks |
| `Executor(ExecutorFlip)` | true | yes: CONTROL passes (`:541`), rest swallowed (`:551-553`) | — | — |

`on_key` `:686-689` routes every non-Browse mode to `on_editor_key` `:511-536`, which sends Executor to `on_executor_key` `:540-554`.

### 2.2 Key-match sites → actions

| file:line | chord(s) today | modifiers checked? | state guard | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|---|
| `:695` | `j`, `Down` | **no** | none | next box | `list.down` `["j","down"]` "down", no | shared |
| `:699` | `k`, `Up` | **no** | none | previous box | `list.up` `["k","up"]` "up", no | shared |
| `:703` | `t` | **no** | `blocked()`/no selection → no-op, Consumed | open tags editor | `settings.boxes.edit_tags` `["t"]` "tags", no | view (D2) |
| `:707` | `e` | **no** | same | open quirks editor | `settings.boxes.edit_quirks` `["e"]` "quirks", no | view (D2) |
| `:712-721` | `w` | **yes**: `!intersects(CONTROL\|ALT)` (`:713-715`) | **`unavailable.is_none() && selected_record().is_some()`** (`:716-717`), else `Pass` → global `w` workspaces | open executor confirmation | `settings.boxes.executor` `["w"]` "executor", no — **declines** with no listed box / refused read | view (new) |
| `:723` | `s` | **no** (`ctrl-s` in Browse opens the spec editor today) | `blocked()`/no snapshot → no-op, Consumed | open probe spec editor | `settings.boxes.edit_spec` `["s"]` "spec", no | view (new) |
| `:727` | `p` | **no** | internal (`probe()` refuses/notice), Consumed | `ProbeBox` on this box | `settings.boxes.probe` `["p"]` "probe", no | view (D2) |
| `:733` | `r` | **no** | none | `Boxes` read | `common.reload` `["r"]` "reload", no | shared |
| `:739` | `Esc` | **no** | **`notice.is_some()`**, else `Pass` | clear notice | `common.dismiss` `["esc"]` "dismiss", no — declines | shared |
| `:545` | `y` (Executor) | CONTROL only (`:541`); `alt-y` writes today | — | `submit` (EditBox executor) | `confirm.yes` `["y"]` "yes", no | shared |
| `:546` | `n`, `Esc` (Executor) | CONTROL only | — | back to Browse (busy kept) | `confirm.no` `["n","esc"]` "no", no | shared |
| `text_area.rs:194` via `:522` | `ctrl-s` (Quirks, Spec) | widget | — | `submit` | **also** `form.save` `["ctrl-s"]` "save", in_capture — offered in `BOXES_TEXT_AREA`, handled by calling `submit` (D13: a rebound `form.save` adds a chord; built-in `ctrl-s` keeps working through the widget) | shared + widget |
| `:533-534` | TextField/TextArea `Pass` | CONTROL passes | — | — | lane-wide modal pass rule | widget |

`Enter` in Tags is the widget's submit (fixed, not `form.save`): Tags gets **no** `form.save` layer, so `ctrl-s` in the tags field stays a no-op as today (no default change).

### 2.3 Stacks

| Mode | Stack const | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| Browse | `BOXES_BROWSE` | `Layer::all(SettingsBoxes)`, `Layer::all(Settings)`, `Layer::only(Common, &[Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | "in Settings > Boxes" |
| Tags | `SETTINGS_FIELD` (or `BOXES_TAGS`) | `Layer::modal(Global)` | "in a Settings text field" (or "in the Boxes tag editor") |
| Quirks, Spec | `BOXES_TEXT_AREA` (or `BOXES_QUIRKS` + `BOXES_SPEC`) | `Layer::only(Form, &[FormSave])`, `Layer::modal(Global)` | "in the Boxes quirks or probe spec editor" |
| Executor | `SETTINGS_CONFIRM` (or `BOXES_EXECUTOR`) | `Layer::all(Confirm)`, `Layer::modal(Global)` | "in a Settings confirmation" (or "in the Boxes executor confirmation") |

**The view layer must be `Layer::all(SettingsBoxes)`, not `Layer::only(SettingsBoxes, &[own acts])`**, and the common layer must admit `Reload`: that is what makes the D10 override row `(settings.boxes, Reload)` visible (see 2.8).

### 2.4 Hints

| Site | Today (exact) | `HintSpec` | Rendered with defaults | Differs? |
|---|---|---|---|---|
| `HINT_BROWSE` `:62-63` | `j/k move · t tags · e quirks · w executor · p probe · s spec · r reload` | `[Pair(ListDown,ListUp,"move"), One(BoxesEditTags,"tags"), One(BoxesEditQuirks,"quirks"), One(BoxesExecutor,"executor"), One(BoxesProbe,"probe"), One(BoxesEditSpec,"spec"), One(Reload,"reload")]` via `BOXES_BROWSE` | identical | no |
| `HINT_NO_LIST` `:67` | `s spec · r reload` | `[One(BoxesEditSpec,"spec"), One(Reload,"reload")]` | identical | no |
| `HINT_RELOAD` `:71` | `r reload` | `[One(Reload,"reload")]` | identical | no |
| `HINT_TAGS` `:74` | `Enter saves · Esc cancels · comma-separated` | `[Text("Enter saves"), Text("Esc cancels"), Text("comma-separated")]` | identical | no |
| `HINT_QUIRKS` `:77` | `ctrl-s saves · Esc cancels · Enter breaks the line` | `[One(FormSave,"saves"), Text("Esc cancels"), Text("Enter breaks the line")]` via `BOXES_TEXT_AREA` | `Ctrl+s saves · Esc cancels · Enter breaks the line` | **yes**: `ctrl-s` → `Ctrl+s` |
| `HINT_EXECUTOR` `:80` | `y write · n/esc cancel` | `[One(ConfirmYes,"write"), All(ConfirmNo,"cancel")]` via `SETTINGS_CONFIRM` | `y write · n/Esc cancel` | **yes**: `esc` → `Esc` |
| `HINT_SPEC` `:83-84` | `ctrl-s saves · Esc cancels · Enter breaks the line · blank clears` | `[One(FormSave,"saves"), Text("Esc cancels"), Text("Enter breaks the line"), Text("blank clears")]` | `Ctrl+s saves · Esc cancels · Enter breaks the line · blank clears` | **yes** |
| `hint_text` suffixes `:1082`, `:1085` | ` · probing…`, ` · saving…` | unchanged appends | unchanged | no |

Hint choice `:1072-1080` (listed / readable / neither) is unchanged; only the constants become specs. The quirks/spec hint label follows `form.save`; with `[form] save = "ctrl-x"` it reads `Ctrl+x saves` while `ctrl-s` also still saves (D13; README sentence).

### 2.5 Collisions
- **`w`: `settings.boxes.executor` vs `global.workspaces` in `BOXES_BROWSE`** (both defaults). Today the section sees `w` first and passes it with no listed box (`:692-693` comment; pinned by `tests/box_settings.rs:1356` `w_passes_to_the_workspace_switcher_with_no_box_to_act_on`). Propose **`STATE_GUARDED` entry `(Act::BoxesExecutor, Act::Workspaces)`**, commented "boxes.rs:712: the executor confirmation only over a listed box; otherwise `w` falls through to the switcher". It is not `SHADOWING` (the narrower act does not always win). If the foundation restricts `STATE_GUARDED` to one context, add it to `SHADOWING` instead with the same comment (the narrower layer is first in the stack, both are defaults) — see Q4.
- No other: `t e w s p` / `l ] right h [ left` / `r esc` / `j down k up` / global. `BOXES_TEXT_AREA`: `ctrl-s` vs modal global `ctrl-f`, `ctrl-w`, `f1` — disjoint. `SETTINGS_CONFIRM` as in 1.5.
- `e` is `edit_quirks` here and `common.edit` elsewhere: no collision because `BOXES_BROWSE`'s common layer does not admit `Edit`.

### 2.6 CONTROL guards made redundant
- `:713-715` `!key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)` on `w` → gone (`ctrl-w` is a different chord; resolves to `global.waiting`). Keep the state half (`:716-717`) as the decline condition. In-file test `ctrl_w_over_a_listed_box_passes_and_w_still_opens_the_executor` (`:1291-1315`) stays green unchanged.
- `:533` `FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL)` and `:541-543` → lane-wide modal pass rule (adds ALT + F-keys; `F1` opens help from every boxes editor; `alt-y` no longer writes the executor).
- Browse arms `:694-743` are modifier-blind except `w`: today `ctrl-t`/`ctrl-e`/`ctrl-p`/`ctrl-r` act and **`ctrl-s` in Browse opens the probe spec editor**. All inert after conversion.
- The view's own ctrl chord: `ctrl-s` (TextArea submit) → `form.save` as above. `KeyModifiers` import (`:41`) goes; `KeyCode` may go entirely.

### 2.7 Tests and snapshots touched
| file:line | assertion | old → new |
|---|---|---|
| `tests/box_settings.rs:1220` | `frame.contains("y write \u{b7} n/esc cancel")` | → `"y write \u{b7} n/Esc cancel"` |
| `tests/box_settings.rs:2090` | same | → same |
| `tests/box_settings.rs:1474` | `last_line == "ctrl-s saves · Esc cancels · Enter breaks the line · blank clears"` | → `"Ctrl+s saves \u{b7} Esc cancels \u{b7} Enter breaks the line \u{b7} blank clears"` |
| `tests/box_settings.rs:2047` | `frame.contains("ctrl-s saves")` | → `"Ctrl+s saves"` |
| `tests/box_settings.rs:1002`, `:1715` | `ctrl-s retries` | unchanged (prose, M6) |
| `tests/box_settings.rs:1168`, `:1395`, `:1922` | `key("ctrl-c") == Handled::Pass` in Quirks/Executor/Spec | unchanged **iff** the modal pass rule passes any CONTROL chord (Q1) |
| `tests/box_settings.rs:1359`, `:1370`, `:1384` | `key("w") == Handled::Pass` with no box | unchanged (executor declines) |
| `tests/box_settings.rs:1492`, `:1568`, `:1585` | `s spec · r reload`, `r reload` | unchanged |

Snapshots: `box_settings__executor_confirm.snap:34` `y write · n/esc cancel` → `y write · n/Esc cancel`; `box_settings__quirks_editor.snap:34` → `Ctrl+s saves · Esc cancels · Enter breaks the line`; `box_settings__spec_editor.snap:34` → `Ctrl+s saves · Esc cancels · Enter breaks the line · blank clears`. Unchanged: `demo` (row 32 section hint, row 34 status line: Browse status = full global layer, same as today), `no_boxes`, `offline`, `stale`, `tag_editor`, `two_boxes`.

### 2.8 Pins (D14) for boxes
- **Narrower-override test (D14, plan acceptance)** — proposed home `tests/box_settings.rs` (needs `Harness::with_keys`): `keys = htui::keys::load_str("version = 1\n[settings.boxes]\nreload = \"f5\"\n")`; `Harness::over(MemStore::demo()).with_keys(keys).with_tab(SettingsTab::with_sections(vec![Box::new(BoxesSection::new()), Box::new(PromptSection::new())]))`; focus Boxes, `settle()`. Then: `key("f5")` → `queued() == 1` (a `Boxes` read); `settle()`; `key("r")` → `queued() == 0`; render: section hint row contains `F5 reload`; `key("l")` → Prompt; `key("r")` → `queued() == 1`; Prompt's hint still `r reload`. For this to work `BOXES_BROWSE` must be `[Layer::all(SettingsBoxes), …, Layer::only(Common, &[Reload, Dismiss]), …]` (2.3): D10 accepts `[settings.boxes] reload` because `common` sits below `settings.boxes` in a declared stack and admits `Reload`; `Layer::all(SettingsBoxes)` admits the override row, which shadows `(common, Reload)` for this stack only; `PROMPT_BROWSE` has no `settings.boxes` layer.
- **Lane rebinding test** (if one test per lane suffices, the connection test in 1.8 is the lane's; otherwise): `[settings.boxes] edit_spec = "S"` → `s` inert, `S` opens the spec editor, `HINT_NO_LIST` reads `S spec · r reload`.
- Optional defect pins: `ctrl-s` in Browse opens no spec editor; `alt-y` in Executor writes nothing.

## 3. `crates/htui/src/ui/tabs/settings/prompt.rs` (title `Prompt`, id `prompt`)

### 3.1 Modes (`enum Mode`, `prompt.rs:165`)

| Mode | `captures_input` (`:828-830`) | Modal? | Text widget | Widget-fixed keys |
|---|---|---|---|---|
| `Browse` | false | no | — | — |
| `Editing(Editor)` | true | yes: widget first; `Pass` + CONTROL → `Pass`, else swallowed (`:497-498`) | `TextField::with_text` (`:463`) | printable, editing/cursor keys, `Enter` = submit (`:488`), `Esc` = cancel (`:492`) |

`on_key` `:841-844` routes Editing to `on_editor_key` `:481-500`.

### 3.2 Key-match sites → actions

| file:line | chord(s) today | modifiers checked? | state guard | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|---|
| `:849` | `e` | **no** | `!blocked() && selected()`; header row → `NOT_A_VALUE_ROW` notice; always Consumed | open editor | `common.edit` `["e"]` "edit", no | shared |
| `:857` | `j`, `Down` | **no** | none | cursor down | `list.down` | shared |
| `:861` | `k`, `Up` | **no** | none | cursor up | `list.up` | shared |
| `:869` | `r` | **no** | none | `PromptSettings(scope)` read | `common.reload` | shared |
| `:875` | `Esc` | **no** | **`notice.is_some()`**, else `Pass` (pinned `tests/prompt_settings.rs:1759-1763`) | clear notice | `common.dismiss` — declines | shared |
| `:486-498` | TextField outcomes | `:497` CONTROL passes | — | widget | fixed | widget |

No view verb: `settings.prompt` has **no catalogue row** (see Q5 — the context must still exist as a table and a stack layer).

### 3.3 Stacks

| Mode | Stack const | Layers | DECLARED phrase |
|---|---|---|---|
| Browse | `PROMPT_BROWSE` | `Layer::all(SettingsPrompt)`, `Layer::all(Settings)`, `Layer::only(Common, &[Edit, Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | "in Settings > Prompt" |
| Editing | `SETTINGS_FIELD` (or `PROMPT_EDITOR`) | `Layer::modal(Global)` | "in a Settings text field" (or "in the Prompt editor") |

### 3.4 Hints

| Site | Today (exact) | `HintSpec` | Rendered with defaults | Differs? |
|---|---|---|---|---|
| `HINT_BROWSE` `:68` | `j/k · e edit · r reload` | `[Pair(ListDown,ListUp,""), One(Edit,"edit"), One(Reload,"reload")]` via `PROMPT_BROWSE` | `j/k · e edit · r reload` **only if** `Keys::hint` drops the space for empty text (Q3); with today's `hint.rs:103` it renders `j/k  · e edit · r reload` (double space) | **depends on Q3** |
| `HINT_NO_SNAPSHOT` `:71` | `r reload` | `[One(Reload,"reload")]` | identical | no |
| `HINT_EDITING` `:74` | `Enter save · Esc cancel · empty clears` | `[Text("Enter save"), Text("Esc cancel"), Text("empty clears")]` | identical | no |
| busy suffix `:759` | `{keys} · {busy} in flight` | unchanged append | — | no |

### 3.5 Collisions
None (`e r esc` / `j down k up` / `l ] right h [ left` / global).

### 3.6 CONTROL guards made redundant
- `:497` → lane-wide modal pass rule.
- Browse `:848-880` is modifier-blind (`ctrl-e` opens the editor, `ctrl-r` reloads, `ctrl-j`/`ctrl-k` move). Inert after.
- `KeyModifiers` import (`:53`) goes. No ctrl chord of its own.

### 3.7 Tests and snapshots touched
- If Q3 is answered "drop the space": no prompt snapshot changes. Otherwise `prompt_settings__app_only.snap:32`, `prompt_settings__demo.snap:32`, `prompt_settings__clamped.snap:34` change `j/k · e edit · r reload` (to the double space, or to the chosen text such as `j/k rows`).
- Unchanged: `editor_fraction` (`Enter save · Esc cancel · empty clears`), `offline` (`r reload`), `stale` (notice row). Status rows (`app_only`, `demo`, `offline` row 34) are Browse → full global layer → unchanged.
- Pins that stay green: `tests/prompt_settings.rs:1689` (`l` is text in the editor → `Consumed`), `:1729` `r reload`, `:1744` `` `e` edits a value row `` (prose), `:1760`/`:1774` Esc pass/consume, `:1815` `q` quits from Browse, `:615-617`/`:930-932` `l` cycles sections at App level.
- In-file `a_wide_notice_takes_the_hint_line_alone` (`:1266-1283`) → `hint_text(Keys::compiled())`.
- `tests/connection.rs:1177` and `tests/settings.rs` register `PromptSection`; no assertion there reads Prompt's hint.

### 3.8 Pins
- Optional defect pin: `ctrl-e` in Browse opens no editor.
- Covered by the boxes override test (2.8): Prompt keeps `r` while `[settings.boxes] reload = "f5"`.

## 4. `crates/htui/src/ui/tabs/settings/queue.rs` (title `Queue`, id `queue`)

### 4.1 Modes (`enum Mode`, `queue.rs:161`)

| Mode | `captures_input` (`:600-602`) | Modal? | Text widget | Widget-fixed keys |
|---|---|---|---|---|
| `Browse` | false | no | — | — |
| `Editing(Editor)` | true | yes: widget first; `Pass` + CONTROL → `Pass`, else swallowed (`:376-377`) | `TextField::with_text` (`:339`) | printable, editing/cursor keys, `Enter` = submit (`:367`), `Esc` = cancel (`:371`) |

`on_key` `:612-615` routes Editing to `on_editor_key` `:360-379`.

### 4.2 Key-match sites → actions

| file:line | chord(s) today | modifiers checked? | state guard | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|---|
| `:617` | `e`, **`Enter`** | **no** | `!blocked() && selected()`; non-value row → `NOT_A_VALUE_ROW`; always Consumed | open editor | `common.edit`, catalogue `["e"]` + **`VIEW_DEFAULTS (SettingsQueue, Edit, ["e","enter"])`** "edit", no | shared + D12 |
| `:625` | `j`, `Down` | **no** | none | cursor down | `list.down` | shared |
| `:629` | `k`, `Up` | **no** | none | cursor up | `list.up` | shared |
| `:633` | `r` | **no** | none | every `wants_requests(scope)` | `common.reload` | shared |
| `:639` | `Esc` | **no** | **`notice.is_some()`**, else `Pass` | clear notice | `common.dismiss` — declines | shared |
| `:365-377` | TextField outcomes | `:376` CONTROL passes | — | widget | fixed | widget |

No view verb; `settings.queue` has no own catalogue row, only the `VIEW_DEFAULTS` row (Q5).

### 4.3 Stacks

| Mode | Stack const | Layers | DECLARED phrase |
|---|---|---|---|
| Browse | `QUEUE_BROWSE` | `Layer::all(SettingsQueue)`, `Layer::all(Settings)`, `Layer::only(Common, &[Edit, Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | "in Settings > Queue" |
| Editing | `SETTINGS_FIELD` (or `QUEUE_EDITOR`) | `Layer::modal(Global)` | "in a Settings text field" (or "in the Queue editor") |

`Layer::all(SettingsQueue)` is what makes the `VIEW_DEFAULTS` `(settings.queue, Edit)` row win over `(common, Edit)` in this stack.

### 4.4 Hints

| Site | Today (exact) | `HintSpec` | Rendered with defaults | Differs? |
|---|---|---|---|---|
| `HINT_BROWSE` `:42` (`pub`, no external user: text search `queue::HINT` = 0 hits) | `j/k · e edit · r reload` | `[Pair(ListDown,ListUp,""), One(Edit,"edit"), One(Reload,"reload")]` via `QUEUE_BROWSE` (first chord of the view-default row is `e`) | `j/k · e edit · r reload` iff Q3 | **depends on Q3** |
| `HINT_NO_SNAPSHOT` `:45` | `r reload` | `[One(Reload,"reload")]` | identical | no |
| `HINT_EDITING` `:48` (`pub`) | `Enter save · Esc cancel · empty clears` | `[Text("Enter save"), Text("Esc cancel"), Text("empty clears")]` | identical | no |
| busy suffix `:557` | `{keys} · {busy} in flight` | unchanged append | — | no |

The two `pub` constants become `pub const …: HintSpec` (or drop `pub`; nothing outside the file uses them).

### 4.5 Collisions
None. `enter` appears only on the view-default `Edit` row; the common layer's `ListFold` (`enter`) is not admitted.

### 4.6 CONTROL guards made redundant
- `:376` → lane-wide modal pass rule.
- Browse `:616-644` is modifier-blind (`ctrl-e`, `ctrl-enter`, `ctrl-r`, ...). Inert after.
- `KeyModifiers` import (`:35`) goes. No ctrl chord of its own.

### 4.7 Tests and snapshots touched
- If Q3 is not "drop the space": `queue_settings__demo.snap:34` changes. Otherwise none of `changed_elsewhere`, `demo`, `editing`, `invalid` (`Enter save · Esc cancel · empty clears · <notice>`), `offline` move.
- `tests/queue_settings.rs` imports `NOTHING_SET, QueueSection, UNKNOWN_COST` (`:21`), not the hint constants; no hint-substring assertion. Pins that stay green: `:651`/`:822` `esc` cancels the editor, `:653`/`:820`/`:828` `e` opens it.

### 4.8 Pins
- Optional: `Enter` in Browse still opens the editor (pins the `VIEW_DEFAULTS` row; a bench test, compiled keys).
- Optional defect pin: `ctrl-e` in Browse opens no editor.

## 5. Cross-file summary

### 5.1 All L-B stacks for `DECLARED` (7 if shared, 11 if per mode)

| Const | Used by | Layers | Phrase |
|---|---|---|---|
| `CONNECTION_BROWSE` | connection Browse | `[settings.connection, settings, common∩{edit,clear,reload,dismiss}, list∩{down,up}, global]` | "in Settings > Connection" |
| `BOXES_BROWSE` | boxes Browse | `[settings.boxes, settings, common∩{reload,dismiss}, list∩{down,up}, global]` | "in Settings > Boxes" |
| `PROMPT_BROWSE` | prompt Browse | `[settings.prompt, settings, common∩{edit,reload,dismiss}, list∩{down,up}, global]` | "in Settings > Prompt" |
| `QUEUE_BROWSE` | queue Browse | `[settings.queue, settings, common∩{edit,reload,dismiss}, list∩{down,up}, global]` | "in Settings > Queue" |
| `SETTINGS_FIELD` | connection Editing, prompt Editing, queue Editing, boxes Tags | `[global(modal)]` | "in a Settings text field" |
| `SETTINGS_CONFIRM` | connection ConfirmClear + ConfirmRebuild, boxes Executor | `[confirm, global(modal)]` | "in a Settings confirmation" |
| `BOXES_TEXT_AREA` | boxes Quirks, Spec | `[form∩{save}, global(modal)]` | "in the Boxes quirks or probe spec editor" |

### 5.2 Allow-list entries L-B needs
- `STATE_GUARDED` += `(Act::BoxesExecutor, Act::Workspaces)` (2.5). Nothing else: no `SHADOWING` entry is demanded by L-B's stacks (`form.save` `ctrl-s` does not meet `global.*`; `Tab`/`BackTab` are not offered in any L-B mode).
- `VIEW_DEFAULTS` += `(Context::SettingsQueue, Act::Edit, &["e", "enter"])` (4.2).

### 5.3 Behaviour changes L-B ships (all intended by D5/D6; none is a default change)
- Modifier-blind Browse arms become exact chords in all four sections (`ctrl-e`, `ctrl-r`, `alt-c`, `ctrl-s`-opens-spec, ... become inert).
- ALT-blind confirm answers fixed: `alt-y` no longer answers connection's two questions or boxes' executor flip.
- `F1` opens help from every L-B editor and confirmation (was swallowed); ALT chords reach the global layer (unbound by default, so no visible effect).
- `Tab`/`BackTab` in every L-B text mode stay swallowed exactly as today (no defect-2 analogue in this lane).

### 5.4 Status line / `?` box with defaults (for T1's D7/D8 tests if a Settings editor from this lane is used)
- Capturing (any L-B text or confirm mode), shell without `register_all`: status line `Ctrl+c quit · F1 help`; with `register_all`: `Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting`.
- `?` box in connection Browse, narrowest first: `Connection: R rebuild cache · Enter run row` / `Settings: l/]/Right next section · h/[/Left previous section` / `Common: e edit · c clear · r reload · Esc dismiss` / `List: j/Down down · k/Up up` / `Global: …`.
- Boxes Browse: `Boxes: t tags · e quirks · w executor · p probe · s spec`, then `Settings`, `Common: r reload · Esc dismiss`, `List`, `Global`.
- Queue Browse: `Queue: e/Enter edit` (the view-default row) and `Common: r reload · Esc dismiss` — **only if** the box skips an act already listed by a narrower layer (Q6); otherwise `Common: e edit · …` duplicates it.
- Prompt Browse: no `Prompt:` line (no rows), then `Settings`, `Common: e edit · r reload · Esc dismiss`, `List`, `Global`.
- No App-level snapshot owned by L-B is in a capturing mode, so no L-B snapshot status row changes (connection snapshots are bench-rendered; `box_settings__demo`/`prompt_settings__{app_only,demo,offline}` are Browse).

## 6. Open questions for the foundation (T1), with proposed answers

- **Q1 — Modal fallback for unlisted keys.** D6 says "gives the rest to its widget and then consumes". Section-level pins (`tests/box_settings.rs:1168`, `:1395`, `:1922`; and the analogous ones in other lanes) require `ctrl-c` → `Handled::Pass` from an open editor/confirmation, where the bench has no shell to check `ctrl-c` first. **Proposal:** a section in a modal mode returns `Pass` for every chord the modal global filter admits (CONTROL, ALT, F-keys), bound or not, and `Consumed` for the rest; T1 exports the predicate (`Layer::admits_chord(KeyChord) -> bool` or `KeyChord::passes_modal()`), so the filter is defined once.
- **Q2 — Shared modal stacks.** **Proposal:** declare `SETTINGS_FIELD`, `SETTINGS_CONFIRM` once (all Settings lanes share them when the shape matches) plus `BOXES_TEXT_AREA`; the error phrase is generic. Per-mode constants (`CONNECTION_EDITOR`, `BOXES_EXECUTOR`, ...) only if T1 wants view-specific phrases; L-B's code is identical either way.
- **Q3 — Empty hint text.** Prompt and queue render `j/k · e edit · r reload` (a pair with no word). `Keys::hint` (`hint.rs:99`, `:105`) writes `"{labels} {text}"`, which yields `j/k ` and a double space. **Proposal:** T1 renders `label` alone when `text` is empty (in `One` and `Pair`) and adds a unit test; then no prompt/queue snapshot moves. Fallback: L-B uses `"rows"` and re-baselines `prompt_settings__{app_only,demo,clamped}` and `queue_settings__demo`.
- **Q4 — `STATE_GUARDED` across contexts.** `(BoxesExecutor, Workspaces)` sit in different contexts. **Proposal:** the validator's `allowed` (`validate.rs:128`) checks the pair plus "default of both" per stack; confirm it does not require one context (the catalogue test `no_spec_is_shared_in_a_context_unless_state_guarded` only looks within one context, so it is unaffected).
- **Q5 — Contexts with no catalogue row.** `settings.prompt` and `settings.queue` have no view verb (queue has only a `VIEW_DEFAULTS` row). `keys/mod.rs` `contexts()` derives tables from `CATALOGUE` (used by `load.rs:183`, `:254` and `print.rs:22`), so `[settings.prompt] reload = "f5"` would be an unknown table, and the narrower override D10 promises would not exist for these two sections. **Proposal:** `Context::ALL` (exhaustive, tested like `Act`'s `ALL`/`position`) drives the loader, `--print-keys` (an empty table prints as a comment line) and headings; `VIEW_DEFAULTS` contexts count as existing.
- **Q6 — `?` box and shadowed acts.** With `VIEW_DEFAULTS` or a D10 override row, the same act has rows in two layers of one stack. **Proposal:** D8's box lists an act only under the first layer of the stack that resolves it (`resolve_row`'s rule), so `Queue: e/Enter edit` is not repeated as `Common: e edit`, and `[settings.boxes] reload = "f5"` shows `F5 reload` under `Boxes`, not `r reload` under `Common`.
- **Q7 — `VIEW_DEFAULTS` vs a user's shared rebind.** With `[common] edit = "E"`, queue's compiled `(settings.queue, Edit)` row (`e`, `enter`) still shadows it, so queue ignores the user's change. The same holds for the agents/hierarchy `Down`/`Up` rows on `form.*`. **Proposal:** a `VIEW_DEFAULTS` row whose own table the user did not set is re-derived as "the shared row's chords + the view's extra chords" after the merge (queue: `E`, `enter`); a row the user did set is taken as written. If T1 rejects that, the README "Changing keys" section must say so.
- **Q8 — Connection `Enter`.** It cannot be a second default of `settings.connection.rebuild`: `Enter` acts only on the Rebuild row while `R` acts anywhere, and a view sees the act, not the chord. **Proposal:** a separate view verb `settings.connection.activate` (`["enter"]`, help `run row`), declined off the Rebuild row. Name alternative: `run_row`.
- **Q9 — Status line where a view act state-guards a global chord.** In boxes Browse with a listed box, `w` is the executor, yet D7's status line (with `register_all`) still shows `w workspaces`. **Proposal:** accept: the hint row says `w executor` right above it, and with no box `w` really does open the switcher.
- **Q10 — Tab-level cycling under a section's stack.** For `[settings.boxes] next_section = "]"`-style overrides to work, `SettingsTab::on_key` must resolve `settings.next_section`/`prev_section` through the **active section's** `key_stack()` (Browse stacks include `Layer::all(Settings)`), falling back to a tab-only stack `[settings, global]` when the section returns `None`. **Proposal:** T1 does exactly that; L-B needs nothing else from the tab.
