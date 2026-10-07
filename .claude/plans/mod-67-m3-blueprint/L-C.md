# MOD-67 M3 blueprint — Lane L-C key inventory

Scope: `crates/htui/src/ui/tabs/settings/hierarchy.rs`, `crates/htui/src/ui/tabs/settings/kinds.rs`.
Owned tests: `crates/htui/tests/hierarchy.rs`, `crates/htui/tests/kinds.rs`; snapshots `crates/htui/tests/snapshots/hierarchy__*`, `kinds__*`.

_Status: complete (sections 1-3)._

## Conventions used below

- New contexts: `Context::SettingsHierarchy` (table `settings.hierarchy`, heading `Hierarchy`), `Context::SettingsKinds` (table `settings.kinds`, heading `Kinds`) — headings are the section titles (`hierarchy.rs:1149`, `kinds.rs` `title()`), per D1.
- New `Act` variants are prefixed with the section (`HierarchyInfer`, `KindsGraph`), mirroring `settings.agents.probe` → `AgentsProbe`.
- "modal(global)" = `Layer::modal(Context::Global)` (D5: CONTROL/ALT chords and F-keys only).
- TextField-owned keys (D13, `ui/text_field.rs:145-200`): every unmodified/SHIFT `Char`, `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`, `Enter` (→ Submit), `Esc` (→ Cancel). It returns `Pass` for any CONTROL/ALT/SUPER/META/HYPER chord and for `Tab`, `BackTab`, `Up`, `Down`, `PgUp`, `PgDn`, F-keys, `Insert`.
- D6 order inside a modal mode: resolver first (`ctx.keys().actions(STACK, chord)`), global-layer act → `Pass`, else the widget, then `Consumed`.

---

# 1. `crates/htui/src/ui/tabs/settings/hierarchy.rs`

## 1.1 Modes

| Mode (`hierarchy.rs`) | `captures_input()` (`:1167-1169`) | Modal? | Text widget / owned keys | Today's dispatcher |
|---|---|---|---|---|
| `Mode::Browse` (`:196-198`) — three hint variants: tree loaded, `snapshot == None` (no workspace), `unavailable == Some` (read refused) | false | no; unlisted keys `Pass` (`:1299`) | none | `on_key` browse match `:1211-1300` |
| `Mode::Editing(Editor)` (`:200`) | true | yes: field first, then Tab/Down/BackTab/Up, CONTROL passes (`:874`), rest swallowed (`:875`) | focused `Field.input: TextField` (`:174-181`) | `on_editor_key` `:841-879` |
| `Mode::Deleting { stage: Counting }` (`:236`) | true | yes: CONTROL passes (`:749-751`), rest swallowed (`:803`) | none | `on_deleting_key` `:748-804` |
| `Mode::Deleting { stage: Warn }` (`:238`) | true | yes (same) | none | same |
| `Mode::Deleting { stage: Typed }` (`:240-245`) | true | yes (same) | `DeleteStage::Typed.field: TextField`; `n`/`y` are slug letters (`:797`) | same, `field.on_key` `:778` |
| `Mode::Deleting { stage: InFlight }` (`:247`) | true | yes; every non-CONTROL key swallowed (`:801`) | none | same |
| `Mode::Picking { picker: PathPicker }` (`:213-221`) | true | yes: CONTROL passes (`:887-889`), rest to picker, then `Consumed` (`:913`) | `PathPicker` (`ui/path_picker.rs:122-186`) owns `j`/`Down`, `k`/`Up`, `Enter`/`l`, `h`/`Backspace`, `s`, `S`, `/`, `.`, `Esc`, plus its own go-to `TextField`; swallows the rest | `on_picker_key` `:886-914` |

`PathPicker` is a widget used only by hierarchy and is in no lane's file list: treat it as widget-owned in M3 (like D13's TextField), see open question Q3.

## 1.2 Key-match sites → actions

All browse arms match `key.code` only — **modifier-blind** (no CONTROL check anywhere in `:1211-1300`): today `ctrl-j/k/N/n/e/p/b/d/r` all act. Resolver dispatch makes every ctrl variant inert (defect 1).

| Site | Chord(s) today | Guard / state | Behaviour | → context.action | Defaults (strict) | Help (catalogue) / hint label | in_capture | Shared or new |
|---|---|---|---|---|---|---|---|---|
| `:1212` | `j` (any mods) | Browse | cursor down, clamped (`move_cursor(true)`) | `list.down` | `["j","down"]` (existing) | `down` / pair text, see H1 | no | shared (**gains `Down`**, ANA §6.6) |
| `:1216` | `k` (any mods) | Browse | cursor up | `list.up` | `["k","up"]` (existing) | `up` | no | shared (**gains `Up`**) |
| `:1220` | `N` | Browse; `refuse('N')` sets a notice but still `Consumed` | opens `NewWorkspace` editor | `settings.hierarchy.new_workspace` (`Act::HierarchyNewWorkspace`) | `["N"]` | `new workspace` / `workspace` | no | **new view verb** (`n` is `common.new`; `N` is a different target) |
| `:1234` | `n` | Browse; `refuse` + `selected()` | new project (workspace row) or repo (project/repo row) editor | `common.new` | `["n"]` (existing) | `new` / `project/repo` | no | shared |
| `:1242` | `e` | Browse; `refuse` + `selected()` | edit row editor | `common.edit` | `["e"]` (existing) | `edit` / `edit` | no | shared |
| `:1250` | `p` | Browse; `refuse` + `selected()`; non-repo row → notice (`:699`) | `UpdateRepo{is_primary}` write | `settings.hierarchy.primary` (`Act::HierarchyPrimary`) | `["p"]` | `make primary` / `primary` | no | **new view verb** |
| `:1258` | `b` | Browse; `refuse` + `selected()`; project row → notice (`:634`) | opens `PathPicker` (`open_path`) | `settings.hierarchy.choose_path` (`Act::HierarchyChoosePath`) | `["b"]` | `choose path` / `path` | no | **new view verb** |
| `:1268` | `i` | Browse; `refuse` + snapshot | `InferRepoPaths` write | `settings.hierarchy.infer` (`Act::HierarchyInfer`) | `["i"]` | `infer paths` / `infer` | no | **new view verb** |
| `:1279` | `d` (any mods — **`ctrl-d` deletes today**) | Browse; `refuse` + `selected()`; repo row → notice (`:730`) | `begin_delete` → `Deleting{Counting}` | `common.delete` | `["d"]` (existing) | `delete` / `delete` | no | shared |
| `:1289` | `r` (any mods) | Browse, allowed while `busy` | `StoreRequest::Hierarchy` re-read | `common.reload` | `["r"]` (existing) | `reload` / `reload` | no | shared |
| `:1295` | `Esc` (any mods) | Browse **and `notice.is_some()`**, else falls to `_ => Pass` | clears notice | `common.dismiss` | `["esc"]` (existing) | `dismiss` / not in hint | no | shared; **declines (Pass) with no notice** → the `Back`/`Dismiss` `STATE_GUARDED` pair already covers it (only `dismiss` is admitted here, so no collision) |
| `:866` | `Tab`, `Down` | Editing, after the field passed | focus next field (wraps) | `form.next_field` | catalogue `["tab"]` + **`VIEW_DEFAULTS (SettingsHierarchy, FormNextField, ["tab","down"])`** | `next field` / pair `field` | yes | shared + D12 view default |
| `:870` | `BackTab`, `Up` | Editing, after the field passed | focus previous field (wraps) | `form.prev_field` | catalogue `["backtab"]` + **`VIEW_DEFAULTS (SettingsHierarchy, FormPrevField, ["backtab","up"])`** | `previous field` | yes | shared + D12 view default |
| `:874` | any CONTROL chord | Editing | `Pass` to shell | — (modal(global) layer; guard becomes redundant) | — | — | — | removed |
| `:875` | anything else the field passed | Editing | swallowed | — (modal swallow stays) | — | — | — | kept |
| `:761` (`stop`), `:764` | `Esc`, `n` (any mods except CONTROL) | Deleting/Counting | back to Browse | `confirm.no` | `["n","esc"]` (existing) | `no` / `stop` | no | shared |
| `:769` | `y` | Deleting/Warn | → `Typed` | `confirm.yes` | `["y"]` (existing) | `yes` / `continue` | no | shared |
| `:775` | `Esc`, `n` | Deleting/Warn | back to Browse | `confirm.no` | `["n","esc"]` | `no` / `stop` | no | shared |
| `:778-799` | `Enter`, `Esc`, characters | Deleting/Typed | TextField: Submit compares slug, Cancel → Browse | — widget-owned (D13); `Text` hints | — | — | — | fixed |
| `:801` | anything | Deleting/InFlight | swallowed | — | — | — | — | fixed |
| `:749-751` | any CONTROL chord | all Deleting stages | `Pass` | — modal(global) | — | — | — | guard removed |
| `:887-889` | any CONTROL chord | Picking | `Pass` | — modal(global) | — | — | — | guard removed |
| `:893` | every other key | Picking | `PathPicker::on_key` | — widget-owned (Q3) | — | — | — | fixed |

No browse arm needs the `settings` layer itself, but it must be in the browse stack so `h`/`l`/`[`/`]`/`Left`/`Right` cycle (T1 owns that). The view never uses `h`/`l`.

### Behaviour changes by construction (to note in the lane report)
- Browse: `ctrl-j/k/N/n/e/p/b/d/r` and modified `Esc` stop acting (`ctrl-i` is `Tab` in the legacy encoding, so it was never `i`). `Down`/`Up` move the cursor.
- Counting/Warn: `alt-n`/`alt-y` used to answer (only CONTROL was guarded); now they fall to modal(global), match nothing and are swallowed — harmless.
- Editing/Deleting/Picking: `F1` used to be swallowed (`:875`, `:803`, picker `:184`); under D5 it reaches `global.help`. ALT chords used to be swallowed in Editing (`:875`) and given to the picker modifier-blind (`alt-j` moved it); now ALT chords with no global binding fall through to the widget, then are swallowed (editor) or reach the picker as today (D6 "gives the rest to its widget").

## 1.3 Stacks (`keys/views.rs`)

| Constant | Mode | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| `HIERARCHY_BROWSE` | Browse (all three hint variants) | `Layer::all(SettingsHierarchy)` → `{new_workspace, primary, choose_path, infer}` (the editor's override rows are excluded by `only`, see note), `Layer::all(Settings)`, `Layer::only(Common, &[Edit, New, Delete, Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | `"in Settings > Hierarchy"` |
| `HIERARCHY_EDITOR` | Editing | `Layer::only(SettingsHierarchy, &[FormNextField, FormPrevField])` (holds the D12 override rows), `Layer::only(Form, &[FormNextField, FormPrevField])`, `Layer::modal(Global)` | `"in the Hierarchy editor"` |
| `HIERARCHY_DELETE_COUNTING` | Deleting/Counting | `Layer::only(Confirm, &[ConfirmNo])`, `Layer::modal(Global)` | `"while Hierarchy counts a delete"` |
| `HIERARCHY_DELETE_WARN` | Deleting/Warn | `Layer::all(Confirm)` (= `{yes, no}`), `Layer::modal(Global)` | `"in the Hierarchy delete warning"` |
| `HIERARCHY_DELETE_TYPED` | Deleting/Typed and Deleting/InFlight | `Layer::modal(Global)` only | `"in the Hierarchy delete confirmation"` |
| `HIERARCHY_PICKING` | Picking | `Layer::modal(Global)` only | `"in the Hierarchy directory picker"` |

Note on `HIERARCHY_BROWSE`'s first layer: once `VIEW_DEFAULTS` creates `(SettingsHierarchy, FormNextField/FormPrevField)` rows, `Layer::all(SettingsHierarchy)` would admit them in Browse and bind `Tab`/`Down`/`BackTab`/`Up` ahead of `list.down`/`global.next_tab`. **Use `Layer::only(SettingsHierarchy, &[HierarchyNewWorkspace, HierarchyPrimary, HierarchyChoosePath, HierarchyInfer])`** — otherwise the validator reports `Down` as `form.next_field` vs `list.down`, and `Tab` would stop switching tabs. (Q1 generalises this.)

`key_stack()` mapping: `Browse → HIERARCHY_BROWSE`, `Editing → HIERARCHY_EDITOR`, `Deleting{Counting} → HIERARCHY_DELETE_COUNTING`, `Deleting{Warn} → HIERARCHY_DELETE_WARN`, `Deleting{Typed | InFlight} → HIERARCHY_DELETE_TYPED`, `Picking → HIERARCHY_PICKING`. All `Some`.

`HIERARCHY_DELETE_TYPED` and `HIERARCHY_PICKING` are the same `[modal(global)]` slice; keep two names (two phrases), or T1 may alias both to one `SETTINGS_MODAL` slice shared with kinds/personas/secrets — the validator result is identical.

## 1.4 Hints

| Site | Const | Today (exact) | Proposed `HintSpec` | Rendered with defaults | Drift |
|---|---|---|---|---|---|
| `:48` | `HINT_BROWSE` | `j/k · N workspace · n project/repo · e edit · p primary · b path · i infer · d delete · r reload` | `[Pair(ListDown, ListUp, ""), One(HierarchyNewWorkspace, "workspace"), One(New, "project/repo"), One(Edit, "edit"), One(HierarchyPrimary, "primary"), One(HierarchyChoosePath, "path"), One(HierarchyInfer, "infer"), One(Delete, "delete"), One(Reload, "reload")]` | identical **iff** an empty label text renders the chords alone (Q2); with today's `hint` it renders `j/k ` + ` · ` = `j/k  · N workspace …` (double space) | none with Q2's answer |
| `:51` | `HINT_NO_WORKSPACE` | `N workspace · r reload` | `[One(HierarchyNewWorkspace, "workspace"), One(Reload, "reload")]` | `N workspace · r reload` | none |
| `:55` | `HINT_UNAVAILABLE` | `r reload` | `[One(Reload, "reload")]` | `r reload` | none |
| `:58` | `HINT_EDITING` | `Tab/Shift+Tab field · Enter save · Esc cancel` | `[Pair(FormNextField, FormPrevField, "field"), Text("Enter save"), Text("Esc cancel")]` | `Tab/Shift+Tab field · Enter save · Esc cancel` (first chords of the override rows are `tab`/`backtab`) | none |
| `:61` | `HINT_PICKING` | `choosing a directory · Esc cancel` | `[Text("choosing a directory"), Text("Esc cancel")]` | identical | none |
| `:76` | `HINT_COUNTING` | `counting rows… · Esc stop` | `[Text("counting rows…"), All(ConfirmNo, "stop")]` | `counting rows… · n/Esc stop` | **yes**: `Esc stop` → `n/Esc stop` (today's arm `:761` already stops on `n`; the hint under-reported it). Alternative with zero drift: `Text("Esc stop")` — rejected, it would not follow a rebinding |
| `:79` | `HINT_WARN` | `y continue · n/Esc stop` | `[One(ConfirmYes, "continue"), All(ConfirmNo, "stop")]` | `y continue · n/Esc stop` | none |
| `:82` | `HINT_TYPED` | `Enter confirm · Esc stop` | `[Text("Enter confirm"), Text("Esc stop")]` | identical | none |
| `:85` | `HINT_DELETING` | `deleting…` | `[Text("deleting…")]` | identical | none |
| `:467-468` | inline | `"{keys} · {busy} in flight"` (Browse, no notice) | unchanged `format!` around `keys.hint(HIERARCHY_BROWSE, spec)` | same | none |
| `:438` | inline | `"{keys} · "` + notice span | unchanged | same | none |

Mechanics: `hint_text(&self)` (`:444`) becomes `hint_text(&self, keys: &Keys) -> String` (calls `keys.hint(self.key_stack(), spec)`); `hint(&self, width, theme)` (`:419`) gains `keys: &Keys` (render passes `ctx.keys()`). Each mode's spec must render through **its own** stack (the editor's `Pair` needs `HIERARCHY_EDITOR` to see the override rows).

Prose that names keys stays M6 (plan "Scope drift"): `NO_WORKSPACE` `:41` (`` `N` creates one``), `CHANGED_ELSEWHERE` `:65-66` (`Enter retries`), `RELOADED` `:73` (`press p again`), `NOT_UNDONE` `:89` (`` `y` to continue, `n` or `Esc` to stop``), `:634` (`` `b` wants…``), `:699` (`` `p` wants a repo row``), `inferred_notice` `:1575` and `:1623` (`b on … sets it`). Untouched in M3.

## 1.5 Collisions
- `HIERARCHY_BROWSE`: view `N p b i`; settings `l ] right / h [ left`; common `e n d r esc`; list `j down / k up`; global `q tab backtab 1-9 ? f1 w ctrl-f ctrl-w`. **No shared chord** (case matters: `N` ≠ `n`). No SHADOWING or STATE_GUARDED entry needed. `dismiss` declines with no notice (`:1295` guard) and nothing else in the stack binds `Esc`, so the decline just reaches the shell (which closes nothing — no overlay in a tab stack).
- `HIERARCHY_EDITOR`: `tab down / backtab up` vs modal(global): `Tab`/`BackTab` are filtered out by D5, so no `form.next_field`/`global.next_tab` shadowing is needed here.
- Delete/picker stacks: `confirm` vs modal(global) — disjoint.
- **VIEW_DEFAULTS**: `(SettingsHierarchy, FormNextField, ["tab","down"])`, `(SettingsHierarchy, FormPrevField, ["backtab","up"])` (D12; mirrors `:866`/`:870`).

## 1.6 CONTROL guards made redundant
- `:749-751` (`on_deleting_key`), `:874` (`on_editor_key` fallback arm), `:887-889` (`on_picker_key`). All three are MOD-52 pass-throughs; modal(global) + D6 "global act → `Pass`" replaces them. Hierarchy itself binds **no ctrl chord**.
- Module doc `:12-15` and `:743-747`, `:836-840`, `:881-885` describe the carve-out; reword to "a chord the modal global layer admits (CONTROL, ALT, F-keys) passes".

## 1.7 Tests and snapshots affected

Text assertions (no change needed, listed so the lane can confirm):
- `tests/hierarchy.rs:1695`, `:1736` `frame.contains("counting rows")` — still true (`counting rows… · n/Esc stop`).
- `tests/hierarchy.rs:2544` finds the hint row by `"j/k"` — still true.
- `tests/hierarchy.rs:2998` `!frame.contains("Esc cancel")` after the picker closes — still true (Browse hint has no `Esc cancel`).
- No test asserts `Esc stop` on the counting hint; `HINT_COUNTING`'s drift touches no assertion and no snapshot (no snapshot is taken in `Counting`).

**Assertion that breaks under a literal D6** (see Q4): `tests/hierarchy.rs:2926` `assert_eq!(bench.key(&mut section, "ctrl-c"), Handled::Pass)` in the picker. With the CONTROL guard `:887` removed and D6 "rest to the widget, then consume", `ctrl-c` (never a candidate) reaches `PathPicker::on_key`, is swallowed, and the section returns `Consumed`. App-level quit still works (`App::on_key` checks `ctrl-c` first; `tests/hierarchy.rs:1402-1415` stays green), but this section-level pin goes red. Same shape for the editor: `ctrl-c` → TextField `Pass` → `:875` swallow → `Consumed` (no test pins the editor at section level).

In-file test: `hierarchy.rs:1749-1766` `a_wide_notice_takes_the_hint_line_alone` calls `section.hint_text()` and `section.hint(width, theme)` without keys; after the signature change pass `Keys::compiled()` (or `&Keys::defaults()`).

Snapshots (re-baseline; only row 34, the shell status line, moves — D7 capture filter):

| Snapshot | Mode | Row that changes | Today | Expected |
|---|---|---|---|---|
| `hierarchy__delete_typed` | Deleting/Typed (modal) | 34 (status) | `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help` | `Ctrl+c quit · F1 help` (exact form is T1's D7 renderer: modal filter drops `q`, `Tab`, `Shift+Tab`, digits and `?`) |
| `hierarchy__delete_warn` | Deleting/Warn (modal) | 34 | same | `Ctrl+c quit · F1 help` |
| `hierarchy__editor_repo` | Editing (modal) | 34 | same | `Ctrl+c quit · F1 help` |
| `hierarchy__demo` | Browse | none (row 32 identical given Q2) | — | — |
| `hierarchy__no_workspace` | Browse, no tree | none | — | — |
| `hierarchy__offline` | Browse, unavailable | none | — | — |
| `hierarchy__picker` | Picking (bench render, no status line) | none (row 34 `choosing a directory · Esc cancel` identical) | — | — |
| `hierarchy__stale`, `hierarchy__inferred` | bench render, notice-only hint | none | — | — |

If Q2 is answered "no empty-text pairs" instead, `hierarchy__demo` row 32 becomes `j/k move · N workspace · …` (and the 2544 search still matches).

Other lanes' files that register `HierarchySection` (`tests/settings.rs`, `tests/connection.rs`, `tests/prompt_settings.rs`, `tests/secrets_settings.rs`): none asserts a hierarchy hint string (searched `N workspace`, `project/repo`, `b path`, `i infer`, `counting rows`, `Enter confirm`, `y continue`, `choosing a directory`). `secrets_settings.rs:2664` only consumes Hierarchy replies.

## 1.8 Defect pins and new tests (D14, lane-owned in `tests/hierarchy.rs`)
1. `ctrl_d_in_browse_deletes_nothing`: bench, tree loaded, cursor on a project, `bench.key("ctrl-d")` → `Handled::Pass`, `bench.drained()` has no `DeleteReach`, render has no `counting rows`. Companion assertions in the same test: `ctrl-n`, `ctrl-e`, `ctrl-r` send/open nothing (all modifier-blind today).
2. `down_and_up_move_the_list`: tree with ≥2 rows; `down` then `e` opens the project editor (row 1, not the workspace); `up` then `e` opens the workspace editor. Mirrors the agents pin.
3. `the_editor_moves_focus_on_down_and_up` (guards the `VIEW_DEFAULTS` rows): open `n` on a project, `down` focuses `remote_url` (render shows the accent on it), `up` back to `name`. Existing editor tests may already cover `Tab`; this pins the view default.
4. Rebinding (one per lane, via `Harness::with_keys(htui::keys::load_str("version = 1\n[settings.hierarchy]\ninfer = \"I\"\n").unwrap())`): `I` sends `InferRepoPaths`, `i` sends nothing, the hint row reads `… · I infer · …`. (`Keys::with_chords` is `#[cfg(test)] pub(crate)`, so an integration test must go through `load_str`.)
5. `f1_opens_help_from_the_editor` (optional; T1 covers D5 generically): `F1` in Editing opens the `?` box, `?` types into the field.

---

# 2. `crates/htui/src/ui/tabs/settings/kinds.rs`

## 2.1 Modes

| Mode (`kinds.rs`) | `captures_input()` (`:1422-1424`) | Modal? | Text widget / owned keys | Today's dispatcher |
|---|---|---|---|---|
| `Mode::Browse` (`:264`) — three hint variants: tree, no snapshot **or** no project (`:1291-1296`), `unavailable` | false | no; unlisted keys `Pass` (`:1515`) | none | `on_key` browse match `:1451-1516` |
| `Mode::Editing(Editor)` (`:266`) | true | yes: field first, then Tab/Down/BackTab/Up, CONTROL passes (`:781`), rest swallowed (`:782`) | focused `Field.input: TextField` | `on_editor_key` `:746-786` |
| `Mode::ConfirmPrefix { editor, old, new }` (`:282-289`) | true | yes: CONTROL passes (`:843-845`), unlisted swallowed (`:872-874`) | none (editor held, not focused) | `on_confirm_key` `:842-875` |
| `Mode::Deleting { stage: Asking }` (`:268-277`, `:329`) | true | yes: CONTROL passes (`:706-708`), rest swallowed (`:727`, `:732`) | none | `on_deleting_key` `:705-733` |
| `Mode::Deleting { stage: InFlight }` (`:331`) | true | yes; every non-CONTROL key swallowed (`:730`) | none | same |

## 2.2 Key-match sites → actions

Browse arms match `key.code` only — **modifier-blind** (`:1451-1516`): today `ctrl-d` opens the delete question, `ctrl-n/N/e/g/r/j/k` act too.

| Site | Chord(s) today | Guard / state | Behaviour | → context.action | Defaults (strict) | Help (catalogue) / hint label | in_capture | Shared or new |
|---|---|---|---|---|---|---|---|---|
| `:1452` | `n` | Browse; `blocked()` (busy → notice; no snapshot → silent) + `selected()`; always `Consumed` | new kind (project/kind row) or new phase (graph/phase row) editor (`:545-562`) | `common.new` | `["n"]` | `new` / `kind/phase` | no | shared |
| `:1460` | `N` | same | new graph editor in the row's project (`:565-581`) | `settings.kinds.new_graph` (`Act::KindsNewGraph`) | `["N"]` | `new graph` / `graph` | no | **new view verb** |
| `:1468` | `e` | same; project row → `PROJECTS_ELSEWHERE` notice | edit kind/graph/phase (`:584-635`) | `common.edit` | `["e"]` | `edit` / `edit` | no | shared |
| `:1476` | `g` | same; project row → notice; missing graph → `GRAPH_MISSING` | opens the owning graph's editor (`:643-673`) | `settings.kinds.graph` (`Act::KindsGraph`) | `["g"]` | `edit graph` / `graph` | no | **new view verb (D2: not `list.top`)** |
| `:1484` | `d` (any mods — **`ctrl-d` opens the delete question today**) | same; project/graph/phase rows → notice | `begin_delete` → `Deleting{Asking}` (`:680-698`) | `common.delete` | `["d"]` | `delete` / `delete kind` | no | shared |
| `:1492` | `j`, `Down` | Browse | cursor down | `list.down` | `["j","down"]` | `down` / pair | no | shared (no default change) |
| `:1496` | `k`, `Up` | Browse | cursor up | `list.up` | `["k","up"]` | `up` / pair | no | shared |
| `:1505` | `r` | Browse, allowed while busy | `StoreRequest::Catalogue` | `common.reload` | `["r"]` | `reload` / `reload` | no | shared |
| `:1511` | `Esc` (any mods) | Browse **and `notice.is_some()`**, else `Pass` | clears notice | `common.dismiss` | `["esc"]` | `dismiss` / not in hint | no | shared; **declines with no notice** (STATE_GUARDED `Back`/`Dismiss` already exists; `back` not admitted here) |
| `:773` | `Tab`, `Down` | Editing, after the field passed | focus next field | `form.next_field` | catalogue `["tab"]` + **`VIEW_DEFAULTS (SettingsKinds, FormNextField, ["tab","down"])`** | `next field` / pair `field` | yes | shared + D12 (**plan D12 lists agents and hierarchy only — kinds needs the same entry**, Q5) |
| `:777` | `BackTab`, `Up` | Editing | focus previous field | `form.prev_field` | catalogue `["backtab"]` + **`VIEW_DEFAULTS (SettingsKinds, FormPrevField, ["backtab","up"])`** | `previous field` | yes | shared + D12 |
| `:781` | any CONTROL chord | Editing | `Pass` | — modal(global) | — | — | — | guard removed |
| `:782` | rest | Editing | swallowed | — | — | — | — | kept |
| `:847` | `y` (ALT too) | ConfirmPrefix | build + send the held edit; mode back to `Editing` | `confirm.yes` | `["y"]` | `yes` / `write` | no | shared |
| `:866` | `n`, `Esc` | ConfirmPrefix | back to the editor, text kept | `confirm.no` | `["n","esc"]` | `no` / `back to the editor` | no | shared |
| `:843-845` | any CONTROL chord | ConfirmPrefix | `Pass` | — modal(global) | — | — | — | guard removed |
| `:715` | `y` | Deleting/Asking | `DeleteKind` sent, stage → `InFlight` | `confirm.yes` | `["y"]` | `yes` / `delete` | no | shared |
| `:726` | `n`, `Esc` | Deleting/Asking | back to Browse | `confirm.no` | `["n","esc"]` | `no` / `stop` | no | shared |
| `:730` | anything | Deleting/InFlight | swallowed | — (see stack note: `confirm.*` resolve but the view consumes them as no-ops) | — | — | — | kept |
| `:706-708` | any CONTROL chord | Deleting (both) | `Pass` | — modal(global) | — | — | — | guard removed |
| Editing `:749`, `:757-766` | characters, `Enter`, `Esc`, editing keys | Editing | TextField: type / `submit` / cancel | widget-owned (D13); `Text` hints | — | — | — | fixed |

Behaviour changes by construction: Browse `ctrl-*`/modified `Esc` arms go inert; ConfirmPrefix and Deleting no longer answer `alt-y`/`alt-n` (only CONTROL was guarded); `F1` reaches help from every capturing mode (was swallowed at `:782`, `:874`, `:732`).

## 2.3 Stacks (`keys/views.rs`)

| Constant | Mode | Layers, narrowest first | DECLARED phrase |
|---|---|---|---|
| `KINDS_BROWSE` | Browse (all three hint variants) | `Layer::only(SettingsKinds, &[KindsNewGraph, KindsGraph])` (not `all`: excludes the D12 override rows, Q1), `Layer::all(Settings)`, `Layer::only(Common, &[Edit, New, Delete, Reload, Dismiss])`, `Layer::only(List, &[ListDown, ListUp])`, `Layer::all(Global)` | `"in Settings > Kinds"` |
| `KINDS_EDITOR` | Editing | `Layer::only(SettingsKinds, &[FormNextField, FormPrevField])`, `Layer::only(Form, &[FormNextField, FormPrevField])`, `Layer::modal(Global)` | `"in the Kinds editor"` |
| `KINDS_CONFIRM_PREFIX` | ConfirmPrefix | `Layer::all(Confirm)`, `Layer::modal(Global)` | `"in the Kinds prefix warning"` |
| `KINDS_DELETE` | Deleting/Asking **and** Deleting/InFlight | `Layer::all(Confirm)`, `Layer::modal(Global)` | `"in the Kinds delete question"` |

`key_stack()`: `Browse → KINDS_BROWSE`, `Editing → KINDS_EDITOR`, `ConfirmPrefix → KINDS_CONFIRM_PREFIX`, `Deleting{..} → KINDS_DELETE`.

`KINDS_DELETE` covers InFlight on purpose: today `hint_text` (`:1287`) shows `HINT_DELETING` for both stages, and rendering that spec through a `[modal(global)]` InFlight stack would drop both entries and leave an empty hint (drift). The InFlight handler matches `ConfirmYes | ConfirmNo => Consumed` (no-op, today's swallow). Alternative if T1 prefers truthful `?` boxes: a separate `KINDS_DELETE_IN_FLIGHT = [modal(global)]` plus a `HINT_DELETE_IN_FLIGHT = &[]` (empty hint row in InFlight — the pane already says `delete_kind in flight`, `:1334`); no test or snapshot covers that row either way.

`KINDS_CONFIRM_PREFIX` and `KINDS_DELETE` are the same layer slice; two names keep two phrases.

## 2.4 Hints

| Site | Const | Today (exact) | Proposed `HintSpec` | Rendered with defaults | Drift |
|---|---|---|---|---|---|
| `:58` | `HINT_BROWSE` | `j/k · n kind/phase · N graph · e edit · g graph · d delete kind · r reload` | `[Pair(ListDown, ListUp, ""), One(New, "kind/phase"), One(KindsNewGraph, "graph"), One(Edit, "edit"), One(KindsGraph, "graph"), One(Delete, "delete kind"), One(Reload, "reload")]` | identical given Q2 | none (Q2) |
| `:61` | `HINT_NO_WORKSPACE` | `r reload` | `[One(Reload, "reload")]` | `r reload` | none |
| `:65` | `HINT_UNAVAILABLE` | `r reload` | same spec (may merge the two consts into one `HINT_RELOAD_ONLY`) | `r reload` | none |
| `:68` | `HINT_EDITING` | `Tab/Shift+Tab field · Enter save · Esc cancel` | `[Pair(FormNextField, FormPrevField, "field"), Text("Enter save"), Text("Esc cancel")]` through `KINDS_EDITOR` | identical | none |
| `:71` | `HINT_CONFIRM_PREFIX` | `y write · n/Esc back to the editor` | `[One(ConfirmYes, "write"), All(ConfirmNo, "back to the editor")]` | identical | none |
| `:93` | `HINT_DELETING` | `y delete · n/Esc stop` | `[One(ConfirmYes, "delete"), All(ConfirmNo, "stop")]` | identical | none |
| `:1305-1306` | inline | `"{keys} · {busy} in flight"` | unchanged `format!` around `keys.hint(...)` | same | none |
| `:1277` | inline | `"{keys} · "` + notice | unchanged | same | none |
| `:1907-1908` | `delete_question` prose | `delete kind {name} ({prefix})? a kind any item uses is refused. y delete · n/Esc stop` | **Q6**: recommend `delete_question(name, prefix, keys: &str)` appending `keys.hint(KINDS_DELETE, HINT_DELETING)`, so the pane cannot contradict a rebinding the hint row follows | identical with defaults | none |

`hint_text(&self)` (`:1283`) → `hint_text(&self, keys: &Keys)`; `hint(&self, width, theme)` (`:1259`) gains `keys` (render `:1590` passes `ctx.keys()`); `pane` (`:1317`) gains `keys` only if Q6 is taken. In-file test `kinds.rs:2038-2055` (`a_wide_notice_takes_the_hint_line_alone`) calls both without keys: pass `Keys::compiled()`.

Prose with key names that stays M6: `CHANGED_ELSEWHERE`/`CHANGED_ELSEWHERE_CLOSED` (imported from `settings/mod.rs:106`, `:114`; `Enter retries`), `prefix_warning` `:1897` (no key), doc comments.

## 2.5 Collisions
- `KINDS_BROWSE`: view `N g`; settings `l ] right / h [ left`; common `e n d r esc`; list `j down / k up`; global `q tab backtab 1-9 ? f1 w ctrl-f ctrl-w`. **No shared chord.** `g` ≠ `list.top` because `list` is narrowed to `{down, up}` — this is why D2 keeps `g` a view verb; if T1 ever widens kinds' list layer to `all`, `g` collides with `list.top` (`["g","home"]`) and must be a SHADOWING pair `(KindsGraph, ListTop)` — not proposed.
- `KINDS_EDITOR`: `Tab`/`BackTab` filtered out of modal(global); no shadowing.
- `KINDS_CONFIRM_PREFIX`, `KINDS_DELETE`: `confirm` vs modal(global) — disjoint.
- **VIEW_DEFAULTS**: `(SettingsKinds, FormNextField, ["tab","down"])`, `(SettingsKinds, FormPrevField, ["backtab","up"])` — mirrors `:773`/`:777`; **missing from plan D12** (Q5).

## 2.6 CONTROL guards made redundant
- `:706-708` (`on_deleting_key`), `:781` (`on_editor_key`), `:843-845` (`on_confirm_key`). Kinds binds **no ctrl chord** of its own. Doc comments `:700-704`, `:741-745`, `:837-841` reword as for hierarchy.

## 2.7 Tests and snapshots affected

Text assertions (unchanged with defaults; confirm green):
- `tests/kinds.rs:1484` `"y write \u{b7} n/Esc back to the editor"`; `:1515` `"Tab/Shift+Tab field"`; `:2303` the whole `delete_question` sentence incl. `y delete \u{b7} n/Esc stop` (holds whether or not Q6 is taken).
- `tests/kinds.rs:875-879` `q` quits from Browse — unchanged (global layer fully admitted in Browse).
- `tests/kinds.rs:1097-1100` editor consumes `n` and `l` — unchanged (TextField types them).
- `tests/kinds.rs:1045` `esc` — closes the editor via TextField `Cancel`; unchanged.
- No kinds test pins a ctrl chord at section level (no analogue of `hierarchy.rs:2926`).
- In-file `kinds.rs:2038` signature change only.

Snapshots:

| Snapshot | Mode | Row that changes | Today | Expected |
|---|---|---|---|---|
| `kinds__demo` | Browse, harness | none (row 32 identical given Q2; row 34 status unchanged) | — | — |
| `kinds__no_workspace`, `kinds__offline` | Browse, harness | none | — | — |
| `kinds__delete_ask` | Deleting/Asking, bench render (`tests/kinds.rs:2300`), no status line | none | — | — |
| `kinds__editor_phase` (`:1584`), `kinds__phase_detail` (`:944`), `kinds__phase_persona` (`:2262`), `kinds__prefix_warn` (`:1482`), `kinds__stale` (`:1369`) | bench renders (`render_section`), no status line | none | — | — |

So kinds re-baselines **no** snapshot unless Q2 is answered "add `move`" (then `kinds__demo`, `kinds__phase_detail`, `kinds__phase_persona` row with `j/k · n kind/phase …` becomes `j/k move · n kind/phase …`). Every capturing-mode kinds snapshot is a bench render, so none carries a status row; the lane still confirms with a full `cargo insta test` run (memory: grep undercounts).

Other lanes' files registering `KindsSection` (`tests/settings.rs`, `connection.rs`, `prompt_settings.rs`, `secrets_settings.rs`): no kinds hint string asserted.

## 2.8 Defect pins and new tests (D14, `tests/kinds.rs`)
1. `ctrl_d_in_browse_opens_no_delete_question`: bench, catalogue loaded, cursor on a kind row; `bench.key("ctrl-d")` → `Handled::Pass`, `!section.captures_input()`, render has no `delete kind … ?`. Same test: `ctrl-n`, `ctrl-g`, `ctrl-e` open no editor; `ctrl-r` sends no `Catalogue`.
2. `alt_y_does_not_answer_the_delete_question` (optional; documents the ALT change): `Deleting/Asking`, `alt-y` sends no `DeleteKind`, mode unchanged.
3. `the_editor_moves_focus_on_down_and_up` — pins the kinds `VIEW_DEFAULTS` rows.
4. Rebinding via `Harness::with_keys(load_str("version = 1\n[settings.kinds]\ngraph = \"G\"\n"))` — `G` opens the graph editor, `g` does nothing, hint shows `G graph` for the second `graph` entry. (Uses `G`, which is free in `KINDS_BROWSE` since `list.bottom` is not admitted.)

---

# 3. Cross-file: catalogue rows, totals, open questions

## 3.1 Catalogue additions for T1 (strip order: `settings.hierarchy` block, then `settings.kinds`)

| Act | Context / name | Defaults | help | in_capture | Mirrors |
|---|---|---|---|---|---|
| `HierarchyNewWorkspace` | `settings.hierarchy` / `new_workspace` | `["N"]` | `new workspace` | no | `hierarchy.rs:1220` |
| `HierarchyPrimary` | `settings.hierarchy` / `primary` | `["p"]` | `make primary` | no | `hierarchy.rs:1250` |
| `HierarchyChoosePath` | `settings.hierarchy` / `choose_path` | `["b"]` | `choose path` | no | `hierarchy.rs:1258` |
| `HierarchyInfer` | `settings.hierarchy` / `infer` | `["i"]` | `infer paths` | no | `hierarchy.rs:1268` |
| `KindsNewGraph` | `settings.kinds` / `new_graph` | `["N"]` | `new graph` | no | `kinds.rs:1460` |
| `KindsGraph` | `settings.kinds` / `graph` | `["g"]` | `edit graph` | no | `kinds.rs:1476` |

Six new rows (41 → +6 from this lane). Existing shared rows reused: `list.down/up`, `common.edit/new/delete/reload/dismiss`, `confirm.yes/no`, `form.next_field/prev_field`. Not used by either section: `list.top/bottom/fold`, `pane.*`, `form.save/external_editor`, `common.clear/back`.

`VIEW_DEFAULTS` rows from this lane (4): `(SettingsHierarchy, FormNextField, ["tab","down"])`, `(SettingsHierarchy, FormPrevField, ["backtab","up"])`, `(SettingsKinds, FormNextField, ["tab","down"])`, `(SettingsKinds, FormPrevField, ["backtab","up"])`.

Stacks from this lane (10): `HIERARCHY_BROWSE`, `HIERARCHY_EDITOR`, `HIERARCHY_DELETE_COUNTING`, `HIERARCHY_DELETE_WARN`, `HIERARCHY_DELETE_TYPED`, `HIERARCHY_PICKING`, `KINDS_BROWSE`, `KINDS_EDITOR`, `KINDS_CONFIRM_PREFIX`, `KINDS_DELETE`. SHADOWING entries needed: **none**. STATE_GUARDED entries needed: **none new** (`dismiss` declines alone in its stack).

Catalogue comment fixes T1 should make while there: the `list.down` comment cites `hierarchy.rs:1211` (the `match`), the arm is `:1212`; `common.reload` cites `hierarchy.rs:1288` (comment) — arm `:1289`; `common.delete` cites `hierarchy.rs:1278` — arm `:1279`; `common.dismiss` cites `hierarchy.rs:1294`/`kinds.rs:1511` — hierarchy arm `:1295`.

## 3.2 Open questions for the foundation architect

- **Q1 — VIEW_DEFAULTS rows leak into Browse.** D12 puts `(SettingsHierarchy, FormNextField)` rows in the view context. A Browse stack opening with `Layer::all(SettingsHierarchy)` would admit them, so `Down` resolves to `form.next_field` ahead of `list.down`, and `Tab` ahead of `global.next_tab`. *Proposed:* every view-context layer in a stack is `Layer::only(view, <that mode's own acts>)`. A validator check backs it: an override or VIEW_DEFAULTS row `(view, shared act)` may appear only in stacks that also admit `shared act`'s own context. The alternative is to attach VIEW_DEFAULTS to the `form` layer per stack, which needs a new mechanism, so I don't recommend it.
- **Q2 — empty-label pairs.** Both browse hints open with a bare `j/k` (`hierarchy.rs:48`, `kinds.rs:58`). `Keys::hint` formats `"{labels} {text}"`, so `Pair(ListDown, ListUp, "")` renders as `j/k ` and the join gives a double space. *Proposed:* when the text is empty, `hint` emits the labels alone. That's a one-line change in `keys/hint.rs` plus a unit test, and these snapshots then don't move. The fallback is `"move"`, which moves `hierarchy__demo` and `kinds__demo` row 32.
- **Q3 — `PathPicker` keys** (`ui/path_picker.rs:141-184`, its own `HINT_MOVE`/`HINT_MORE` at `:31`/`:34`). No lane owns this file. *Proposed:* treat it as a widget-owned key set for M3, like D13's TextField. `HIERARCHY_PICKING` is `[modal(global)]` and its popup hint stays literal. File a follow-up (M4/M6) for a `picker` context: `j`/`k` as `list.down`/`list.up`, plus `choose`, `choose_here`, `goto`, `hidden`, `up`.
- **Q4 — modal pass-through of unmatched admitted chords.** Read literally, D6 says "the rest to its widget, then consume". That sends `ctrl-c` and any unbound CONTROL/ALT/F-key into the picker and swallows it there, which breaks `tests/hierarchy.rs:2926` (`ctrl-c` → `Handled::Pass`). It also feeds `alt-j`/`alt-s` to the modifier-blind picker. *Proposed:* T1 exposes the D5 filter as a predicate, e.g. `Layer::admits_chord`/`Keys::passes_modal(stack, chord)`. In a modal mode a view returns `Pass` for any chord the modal global layer admits that no narrower candidate took, before the widget sees it. This is MOD-52's CONTROL pass-through generalised and written once, and the existing pin stays green unchanged. TextField behaves the same either way, since it passes those chords anyway.
- **Q5 — kinds' form also takes `Down`/`Up`** (`kinds.rs:773`, `:777`), but plan D12 lists only agents and hierarchy. *Proposed:* add the two kinds `VIEW_DEFAULTS` rows above. That is today's behaviour, not a default change. T1 should also check the other sections' editors for the same `Tab | Down` pattern.
- **Q6 — `delete_question`** (`kinds.rs:1905-1910`) puts the hint `y delete · n/Esc stop` inside the pane's question. *Proposed:* build it from `keys.hint(KINDS_DELETE, HINT_DELETING)` in M3. The default text is identical, so `tests/kinds.rs:2303` and `kinds__delete_ask` don't change, and the pane can't contradict a rebound hint row. The other choice is to leave it to M6 as prose. `NOT_UNDONE` (`hierarchy.rs:89`) is a full sentence and stays M6.
- **Q7 — `HINT_COUNTING` drift** (`Esc stop` → `n/Esc stop`). *Proposed:* accept it, because the arm (`hierarchy.rs:761`) already stops on `n` and no test or snapshot pins the old text. Record it as an intended hint fix in the lane report, since D9 lists the allowed drift.
- **Q8 — kinds InFlight stack.** *Proposed:* share `KINDS_DELETE` and consume the confirm acts as no-ops, so there is zero drift (see 2.3). Otherwise T1 adds a separate `[modal(global)]` stack with an empty hint.
- **Q9 — `?` box headings for the two shared-slice stacks** (`HIERARCHY_DELETE_TYPED`, `HIERARCHY_PICKING` = `[modal(global)]` only). D8 shows only the Global line there, and the `?` box shows no `Hierarchy` heading. *Proposed:* accept it. The section hint row already names the widget keys (`Enter confirm · Esc stop`, `choosing a directory · Esc cancel`).
