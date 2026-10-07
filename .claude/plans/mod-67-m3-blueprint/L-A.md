# MOD-67 M3 blueprint — Lane L-A key inventory

Scope: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/src/ui/tabs/settings/qdrant.rs`.
Owned tests: `crates/htui/tests/{settings,probe,auth,install}.rs`; snapshots `settings__*`, `probe__*`.

Complete: sections 0-8.

## 0. Facts every L-A stack relies on (read from source, 2026-10-07)

- `SettingsTab::on_key` (`settings/mod.rs:337-358`) delegates **first** while `captures_input()` is
  true; otherwise it consumes `l`/`]`/`Right` and `h`/`[`/`Left` (modifier-blind) **before** the
  section sees the key. So every non-capturing L-A stack carries the `settings` layer; no capturing
  one does.
- `TextField::on_key` (`ui/text_field.rs:145-199`) returns `Pass` for any CONTROL/ALT/SUPER/META/HYPER
  chord, and for `Tab`, `BackTab`, `Up`, `Down`, `PgUp`, `PgDn`, `Ins`, `F(n)`; it consumes
  printable chars, `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`; `Enter` = `Submit`,
  `Esc` = `Cancel`. Those consumed/Submit/Cancel keys are the D13 fixed widget keys in every L-A
  capturing mode. Neither file uses `TextArea`, `ctrl-s` or `ctrl-e`.
- Neither file binds any ctrl chord of its own. Every ctrl handling in them is a pass-through guard.

## 1. New catalogue rows (T1 appends; L-A consumes)

Two contexts: `SettingsAgents` (`settings.agents`, heading `Agents`) and `SettingsQdrant`
(`settings.qdrant`, heading `Qdrant`). **Qdrant has no own action**: every Qdrant key is a shared
verb, and the context exists only to host D10 overrides. Agents adds **9** acts, none `in_capture`:

| Act variant | `[settings.agents]` name | defaults | help (`?` box) | mirrors |
|---|---|---|---|---|
| `AgentsProbe` | `probe` | `["r"]` | `probe` | agents.rs:2479-2498 (D2 fixes it) |
| `AgentsInstall` | `install` | `["i"]` | `install` | agents.rs:2428 |
| `AgentsAuthenticate` | `authenticate` | `["a"]` | `authenticate` | agents.rs:2432 |
| `AgentsSwitchBox` | `switch_box` | `["t"]` | `this box` | agents.rs:2407 |
| `AgentsEditPaths` | `edit_paths` | `["m"]` | `paths` | agents.rs:2414 (boxes' `edit_quirks`/`edit_tags` pattern) |
| `AgentsOpenLink` | `open_link` | `["o"]` | `open link` | agents.rs:2438 |
| `AgentsPasteRedirect` | `paste_redirect` | `["p"]` | `paste redirect` | agents.rs:2444 |
| `AgentsCancel` | `cancel` | `["x"]` | `cancel` | agents.rs:2454, :2468 (install **and** login cancel: one meaning, "stop the running flow") |
| `AgentsChoose` | `choose` | `["enter"]` | `select` | agents.rs:640 (chooser `Enter`; not a widget Enter, so not D13-fixed) |

Shared acts L-A uses (all exist, defaults unchanged): `common.new` (n), `common.edit` (e),
`common.clear` (c), `common.reload` (r), `common.dismiss` (Esc), `list.down` (j, Down),
`list.up` (k, Up), `confirm.yes` (y), `confirm.no` (n, Esc), `form.next_field` (Tab),
`form.prev_field` (BackTab), `settings.next_section`/`prev_section` (T1, D1), every `global.*`.

`VIEW_DEFAULTS` rows (D12) for L-A — order matters, the first chord is the hint label:
- `(SettingsAgents, FormNextField, &["tab", "down"])` — agents.rs:2166
- `(SettingsAgents, FormPrevField, &["backtab", "up"])` — agents.rs:2170

## 2. `crates/htui/src/ui/tabs/settings/agents.rs`

### 2.1 Modes (what selects the key handling; `on_key` agents.rs:2355-2501 checks them in this order)

| # | Key mode (state) | Handler today | `captures_input()` (agents.rs:2274) | Modal today? | Text widget, fixed keys (D13) | Stack |
|---|---|---|---|---|---|---|
| F | `Mode::Editing(_)` (create `n` / edit `e`) | `on_editor_key` 1506 → `form_navigation` 2163 | **true** | yes: leftovers `Consumed`, CONTROL `Pass` (2174) | `TextField` of focused `Field` (443): printable, Backspace, Delete, Left, Right, Home, End; `Enter` = submit (1516), `Esc` = cancel (1520) | `AGENTS_FORM` |
| F | `Mode::Paths(_)` (`m`) | `on_paths_key` 1533 → `form_navigation` | **true** | same | `TextField` of focused `PathField` (396); `Enter` = `submit_paths` (1543), `Esc` = cancel (1547) | `AGENTS_FORM` |
| C | `InstallState::Pending` | `answer_consent` 1143 | false | **partly**: swallows j/k/i/r/x (1173), passes the rest (1174) | none | `AGENTS_CONSENT` |
| H | `AuthState::Choosing` | `answer_chooser` 630 | false | partly: swallows a/x (656), passes the rest (657) | none | `AGENTS_CHOOSER` |
| P | `AuthState::Running { paste: Some(_) }` | `on_paste_key` 814 | **true** | yes: leftovers `Consumed` (835), CONTROL `Pass` (834) | masked `TextField` (340); `Enter` = deliver (836), `Esc` = close field (830) | `AGENTS_PASTE` |
| B | everything else: install `Idle`/`Planning`/`Running`/`Manual` × auth `Idle`/`Starting`/`Running{paste:None}` | match at 2386 | false | no | none | `AGENTS_BROWSE` |

The view should get one private `fn stack(&self) -> Stack<'static>` with exactly this priority
(F, C, H, P, B); `SettingsSection::key_stack`, `on_key` and `hint` all call it.

### 2.2 Key-match sites (every arm outside widget-owned keys)

`mb` = modifier-blind today (matches `key.code` only, so `ctrl-<x>` acts as `<x>`: defect-1 class).

| file:line | chord(s) today | guard / state | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|
| agents.rs:632 | `j` mb | H | `move_choice(true)` | `list.down` `["j","down"]` "down" | shared; **gains `Down`** (Q4) |
| agents.rs:636 | `k` mb | H | `move_choice(false)` | `list.up` `["k","up"]` "up" | shared; gains `Up` |
| agents.rs:640 | `Enter` mb | H | `send_choice` | `settings.agents.choose` `["enter"]` "select" | **new view verb** |
| agents.rs:646 | `n`, `Esc` mb | H | `begin_auth_cancel` | `confirm.no` `["n","esc"]` "no" | shared |
| agents.rs:650 | `r`, `i` mb | H | `refuse_during_login` (sentence) | `settings.agents.probe` / `settings.agents.install` | view verbs; `refuse_during_login` (746-755) must take the `Act`, not `key.code == 'r'` (747) |
| agents.rs:656 | `a`, `x` mb | H | swallowed (`Consumed`, no-op) | swallow of `settings.agents.authenticate`/`cancel` resolved through `AGENTS_BROWSE` (Q2) | — |
| agents.rs:657 | `_` | H | `Pass` (q, ?, Tab, digits reach global; tab already took h/l) | stack fall-through | — |
| agents.rs:834 | any CONTROL | P, field passed | `Pass` | removed: modal-pass rule (Q1) | guard |
| agents.rs:835 | any other passed key (Tab, BackTab, Up, Down, F-n, ALT, PgUp…) | P | `Consumed` | modal: consumed unless the modal global admits it | — |
| agents.rs:1145 | `y` mb | C | consent → `InstallConfirm` | `confirm.yes` `["y"]` "yes" | shared |
| agents.rs:1161 | `n`, `Esc` mb | C | decline, notice `install declined` | `confirm.no` `["n","esc"]` | shared |
| agents.rs:1173 | `j`,`k`,`i`,`r`,`x` mb | C | swallowed (`Consumed`, no-op) | swallow of `list.down`, `list.up`, `settings.agents.install`, `.probe`, `.cancel` resolved through `AGENTS_BROWSE` (Q2); `Down`/`Up` newly swallowed (harmless) | — |
| agents.rs:1174 | `_` | C | `Pass` | stack fall-through | — |
| agents.rs:2166 | `Tab`, `Down` | F, field passed | focus next (wrap) | `form.next_field` + `VIEW_DEFAULTS` `["tab","down"]` "next field", in_capture | shared + view default |
| agents.rs:2170 | `BackTab`, `Up` | F | focus previous (wrap) | `form.prev_field` + `VIEW_DEFAULTS` `["backtab","up"]` "previous field", in_capture | shared + view default |
| agents.rs:2174 | any CONTROL | F | `Pass` | removed: modal-pass rule (Q1) | guard |
| agents.rs:2175 | `_` (F-n, ALT, PgUp, PgDn, Ins…) | F | `Consumed` | modal: consumed unless the modal global admits it (F1 now opens help; ALT now passes, unbound) | — |
| agents.rs:2389 | `r`,`i`,`a` mb | B, `busy.is_some()` | `in_flight(busy)` error | same three acts, guard first in the `Act` match | state guard inside one act |
| agents.rs:2395 | `n` mb | B | `refuse_write` else `open_create` | `common.new` `["n"]` "new" | shared |
| agents.rs:2401 | `e` mb | B | `refuse_write` else `open_edit` | `common.edit` `["e"]` "edit" | shared |
| agents.rs:2407 | `t` mb | B | `refuse_write` else `switch_this_box` | `settings.agents.switch_box` `["t"]` | new view verb |
| agents.rs:2414 | `m` mb | B | `refuse_write` else `open_paths` | `settings.agents.edit_paths` `["m"]` | new view verb |
| agents.rs:2420 | `j` mb (no Down) | B | `move_cursor(true)` | `list.down` `["j","down"]` | shared; **gains `Down`** (ANA §6.6, D14 pin) |
| agents.rs:2424 | `k` mb (no Up) | B | `move_cursor(false)` | `list.up` `["k","up"]` | shared; **gains `Up`** |
| agents.rs:2428 | `i` mb | B | `begin_install` (refuses inside) | `settings.agents.install` `["i"]` | new view verb |
| agents.rs:2432 | `a` mb | B | `begin_auth` (refuses inside) | `settings.agents.authenticate` `["a"]` | new view verb |
| agents.rs:2438 | `o` mb | B, `auth_in_flight()` | `open_link`; **else declined → `Pass`** | `settings.agents.open_link` `["o"]` | view verb, declines |
| agents.rs:2444 | `p` mb | B, `auth_in_flight()` | `open_paste`; else `Pass` (pinned by settings.rs:2970) | `settings.agents.paste_redirect` `["p"]` | view verb, declines |
| agents.rs:2454 | `x` mb | B, install `Running`/`Planning` | `InstallCancel` | `settings.agents.cancel` `["x"]` | view verb |
| agents.rs:2468 | `x` mb | B, `auth_in_flight()` | `begin_auth_cancel`; else (neither) `Pass` | same act, second branch | view verb, declines |
| agents.rs:2472 | `Esc` | B, install `Manual` | close manual steps; else `Pass` | `common.dismiss` `["esc"]` "dismiss" | shared, declines |
| agents.rs:2479 | `r` mb | B, `install_in_flight()` | error "an install is running; probe afterwards" | `settings.agents.probe` (branch 1) | view verb |
| agents.rs:2486 | `r` mb | B, `auth_in_flight()` | `refuse_during_login` | probe (branch 2) | — |
| agents.rs:2490 | `r` mb | B, `!probing` | `probing = true`, `ProbeAgents` | probe (branch 3) | — |
| agents.rs:2495 | `r` mb | B, probing | error "a probe is already running" | probe (branch 4) | — |
| agents.rs:2499 | `_` | B | `Pass` | stack fall-through | — |

Declines-and-falls-through (B): `open_link`, `paste_redirect`, `cancel`, `dismiss`. None shares a
chord with another candidate in `AGENTS_BROWSE`, so **no `STATE_GUARDED` entry is needed**; they
fall to the global layer, which binds none of `o`/`p`/`x`/`Esc`.

### 2.3 Stacks (`keys/views.rs`)

Layer notation: `ctx∩{…}` = `Layer::only`, `ctx` = `Layer::all`, `global(modal)` = `Layer::modal(Global)`.
The first layer is always the view context, narrowed so it hosts D10 overrides / D12 view defaults
only for what the mode offers (Q3).

| Constant | DECLARED phrase | Layers, narrowest first |
|---|---|---|
| `AGENTS_BROWSE` | `"in Settings > Agents"` | `settings.agents∩{AgentsProbe, AgentsInstall, AgentsAuthenticate, AgentsSwitchBox, AgentsEditPaths, AgentsOpenLink, AgentsPasteRedirect, AgentsCancel, New, Edit, Dismiss, ListDown, ListUp}`, `settings`, `common∩{New, Edit, Dismiss}`, `list∩{ListDown, ListUp}`, `global` |
| `AGENTS_CONSENT` | `"in the Agents install question"` | `settings.agents∩{ConfirmYes, ConfirmNo}`, `confirm`, `settings`, `global` |
| `AGENTS_CHOOSER` | `"in the Agents login chooser"` | `settings.agents∩{AgentsChoose, AgentsProbe, AgentsInstall, ConfirmNo, ListDown, ListUp}`, `confirm∩{ConfirmNo}`, `list∩{ListDown, ListUp}`, `settings`, `global` |
| `AGENTS_FORM` | `"in the Agents form"` | `settings.agents∩{FormNextField, FormPrevField}` (holds the D12 rows), `form∩{FormNextField, FormPrevField}`, `global(modal)` |
| `AGENTS_PASTE` | `"in the Agents paste field"` | `global(modal)` |

Collision check with defaults (all clean):
- `AGENTS_BROWSE`: view `r i a t m o p x`; settings `h l [ ] Left Right`; common `n e Esc`; list
  `j Down k Up`; global `q Tab BackTab 1-9 ? F1 w ctrl-f ctrl-w`. No shared chord.
- `AGENTS_CONSENT`: `y`, `n Esc`, settings, global. Clean.
- `AGENTS_CHOOSER`: `Enter r i`, `n Esc`, `j Down k Up`, settings, global. Clean.
- `AGENTS_FORM`: `Tab Down BackTab Up` vs modal global (Tab/BackTab filtered out). Clean under the
  filter; if the validator does not apply the filter, the D11 seed pairs
  `form.next_field`/`global.next_tab`, `form.prev_field`/`global.prev_tab` cover it.
- `AGENTS_PASTE`: one layer.

### 2.4 Hints (agents.rs:124-202; selected in `hint` 1382-1401; drawn at 2641-2642)

`hint` becomes `fn hint(&self, keys: &Keys) -> (String, Option<String>)`, rendered through the
mode's own stack (`keys.hint(self.stack(), SPEC)`); `render` passes `ctx.keys()`.

| file:line | constant, today's text | stack | `HintSpec` | rendered with defaults | changes? |
|---|---|---|---|---|---|
| agents.rs:124 | `HINT_IDLE` `j/k select · n new · e edit · m paths · t this box · r probe · i install · a authenticate` | BROWSE | `Pair(ListDown, ListUp, "select")`, `One(New, "new")`, `One(Edit, "edit")`, `One(AgentsEditPaths, "paths")`, `One(AgentsSwitchBox, "this box")`, `One(AgentsProbe, "probe")`, `One(AgentsInstall, "install")`, `One(AgentsAuthenticate, "authenticate")` | identical | no |
| agents.rs:139 | `HINT_EDITING` `Tab next field · Enter saves · Esc cancels` | FORM | `One(FormNextField, "next field")`, `Text("Enter saves")`, `Text("Esc cancels")` | identical (first VIEW_DEFAULTS chord is `tab`) | no |
| agents.rs:187 | `HINT_PENDING` `y install · n cancel` | CONSENT | `One(ConfirmYes, "install")`, `One(ConfirmNo, "cancel")` | identical | no |
| agents.rs:190 | `HINT_RUNNING` `x cancel install` | BROWSE | `One(AgentsCancel, "cancel install")` | identical | no |
| agents.rs:193 | `HINT_MANUAL` `Esc close` | BROWSE | `One(Dismiss, "close")` | identical | no |
| agents.rs:196 | `HINT_CHOOSING` `j/k choose · Enter select · Esc cancel` | CHOOSER | `Pair(ListDown, ListUp, "choose")`, `One(AgentsChoose, "select")`, `All(ConfirmNo, "cancel")` | `j/k choose · Enter select · n/Esc cancel` | **yes** (`Esc cancel` → `n/Esc cancel`; `One` would print `n cancel`, Q5) |
| agents.rs:199 | `HINT_AUTH_RUNNING` `o open link · p paste redirect · x cancel` | BROWSE | `One(AgentsOpenLink, "open link")`, `One(AgentsPasteRedirect, "paste redirect")`, `One(AgentsCancel, "cancel")` | identical | no |
| agents.rs:202 | `HINT_PASTING` `Enter sends · Esc cancels` | PASTE | `Text("Enter sends")`, `Text("Esc cancels")` | identical | no |

Prose naming keys, **left for M6** (plan "Out"): agents.rs:136 `QUOTA_NOTE` ("r cannot refresh
it"), :176 `CHANGED_ON_BOTH_SIDES` ("Enter retries"), :184 `REPROBES` ("r probes now"), :1726
("· r probes"). Pinned by settings.rs:4021, :4043, :4668 — untouched in M3.

Selection-priority mismatch found (Q6): `hint` (1383-1401) lets an install `Manual` win the line
over the login, but `on_key` lets `Choosing`/paste win the keys. With install `Manual` and then
`a` (allowed: `Manual` is not "in flight", 531-536), the chooser or paste field is live while the
line says `Esc close`. Deriving the hint from `self.stack()` first fixes it.

## 3. `crates/htui/src/ui/tabs/settings/qdrant.rs`

### 3.1 Modes (`enum Mode` qdrant.rs:48-54; `on_key` 395-441)

| Mode | Handler | `captures_input()` (377: `!Browse`) | Modal today? | Text widget, fixed keys (D13) | Stack |
|---|---|---|---|---|---|
| `Browse` | match at 401 | false | no | none | `QDRANT_BROWSE` |
| `ConfirmClear` | `on_confirm_key` 228 | **true** (so the tab does not cycle on `h`/`l`) | yes: CONTROL `Pass` (229), unlisted `Consumed` (239, 243) | none | `QDRANT_CONFIRM` |
| `EditingUrl(Editor)` | `on_editor_key` 172 | **true** | **no — defect 2**: `FieldOutcome::Pass => Handled::Pass` (224) hands `Tab`, `BackTab`, `Up`, `Down`, F-n, PgUp/PgDn, ALT and CONTROL chords to the shell; `Tab` switches tabs | `TextField::new()` (161): printable, Backspace, Delete, Left, Right, Home, End; `Enter` submit (202), `Esc` cancel (198) | `QDRANT_EDIT` |
| `EditingKey(Editor)` | `on_editor_key` 172 | **true** | same defect | `TextField::masked()` (166), same keys | `QDRANT_EDIT` |

### 3.2 Key-match sites

| file:line | chord(s) today | guard | behaviour | → (context, action, defaults, help, in_capture) | kind |
|---|---|---|---|---|---|
| qdrant.rs:224 | every key the field passes | Editing | `Pass` to the shell (Tab → next tab) | modal: `Pass` only what `global(modal)` admits (CONTROL/ALT/F-n), else `Consumed` | **defect 2 fix** |
| qdrant.rs:229 | any CONTROL | ConfirmClear | `Pass` | removed: modal-pass rule (Q1) | guard |
| qdrant.rs:234 | `y` | ConfirmClear | Browse + `ClearQdrantSettings` | `confirm.yes` `["y"]` "yes" | shared |
| qdrant.rs:238 | `n`, `Esc` | ConfirmClear | back to Browse | `confirm.no` `["n","esc"]` "no" | shared |
| qdrant.rs:239 | `_` | ConfirmClear | `Consumed` (no-op) | modal swallow (F1 now opens help; ALT now passes, unbound) | — |
| qdrant.rs:402 | `e` mb | Browse | `blocked()` else `open_edit` | `common.edit` `["e"]` "edit" | shared |
| qdrant.rs:408 | `c` mb | Browse | `blocked()`, else ConfirmClear if anything stored, else refuse | `common.clear` `["c"]` "clear" | shared |
| qdrant.rs:424 | `j`, `Down` mb | Browse | `move_cursor(true)` | `list.down` `["j","down"]` | shared, no default change |
| qdrant.rs:428 | `k`, `Up` mb | Browse | `move_cursor(false)` | `list.up` `["k","up"]` | shared |
| qdrant.rs:434 | `r` mb | Browse | `read_out = true`, `QdrantInfo` (never refused) | `common.reload` `["r"]` "reload" | shared |
| qdrant.rs:439 | `_` | Browse | `Pass` | stack fall-through | — |

No arm declines-and-falls-through (each accepted act always consumes). No ctrl chord of its own.
`ctrl-e`/`ctrl-r`/`ctrl-j`/`ctrl-k` and `ctrl-Down` act today (mb); they stop by construction.

### 3.3 Stacks

| Constant | DECLARED phrase | Layers, narrowest first |
|---|---|---|
| `QDRANT_BROWSE` | `"in Settings > Qdrant"` | `settings.qdrant∩{Edit, Clear, Reload, ListDown, ListUp}`, `settings`, `common∩{Edit, Clear, Reload}`, `list∩{ListDown, ListUp}`, `global` |
| `QDRANT_CONFIRM` | `"in the Qdrant clear question"` | `settings.qdrant∩{ConfirmYes, ConfirmNo}`, `confirm`, `global(modal)` |
| `QDRANT_EDIT` | `"in the Qdrant editor"` | `global(modal)` (URL and key editors share it) |

Collisions: none. `QDRANT_BROWSE`: `e c r`, `j Down k Up`, settings, global. `QDRANT_CONFIRM`: `y`,
`n Esc`, and the modal global admits none of them.

### 3.4 Hints (qdrant.rs:29-33; selected in `hint_text` 279-298; `hint` 259-277; drawn at 562)

`hint_text(&self)` → `hint_text(&self, keys: &Keys) -> String`, `hint(&self, room, theme)` →
`hint(&self, keys: &Keys, room, theme)`; `render` passes `ctx.keys()`. Stack per mode as 3.1
(browse rows incl. the no-snapshot one through `QDRANT_BROWSE`). The busy suffix
`format!("{keys} \u{b7} {busy} in flight")` (294) stays.

| file:line | constant, today's text | stack | `HintSpec` | rendered with defaults | changes? |
|---|---|---|---|---|---|
| qdrant.rs:29 | `HINT_BROWSE` `e edit · c clear all · r reload · j/k rows` | BROWSE | `One(Edit, "edit")`, `One(Clear, "clear all")`, `One(Reload, "reload")`, `Pair(ListDown, ListUp, "rows")` | identical | no |
| qdrant.rs:30 | `HINT_NO_SNAPSHOT` `r reload · j/k rows` | BROWSE | `One(Reload, "reload")`, `Pair(ListDown, ListUp, "rows")` | identical | no |
| qdrant.rs:31 | `HINT_EDITING` `Enter continue · Esc cancel` | EDIT | `Text("Enter continue")`, `Text("Esc cancel")` | identical | no |
| qdrant.rs:32 | `HINT_EDITING_KEY` `Enter store · Esc cancel · typed text is never shown` | EDIT | `Text("Enter store")`, `Text("Esc cancel")`, `Text("typed text is never shown")` | identical | no |
| qdrant.rs:33 | `HINT_CONFIRM` `y confirm · n / Esc cancel` | CONFIRM | `One(ConfirmYes, "confirm")`, `All(ConfirmNo, "cancel")` | `y confirm · n/Esc cancel` | **yes** (`n / Esc` → `n/Esc`, D9) |

Prose naming keys, **left for M6**: qdrant.rs:18 `NO_URL_YET` ("press Enter"), :20 `REPLACES_URL`
("Enter replaces"), :26 `UNREADABLE_GUIDE` ("Enter tries"), :27 `CONFIRM_CLEAR` ("? y / n").

## 4. Collisions, SHADOWING, STATE_GUARDED, VIEW_DEFAULTS (L-A total)

- **SHADOWING**: none new. `AGENTS_FORM` only needs the D11 seed pairs, and only if the validator
  walks the global layer unfiltered (`Tab`/`BackTab`).
- **STATE_GUARDED**: none new. The browse decliners (`open_link`, `paste_redirect`, `cancel`,
  `dismiss`) share no chord in their stack.
- **VIEW_DEFAULTS**: the two `SettingsAgents` rows in section 1. Nothing for Qdrant: its browse
  arms already take `Down`/`Up`, and those are `list.*` defaults.
- **Default changes visible to users** (all additive, all from ANA §6.6 / D5):
  - `Down`/`Up` move the agents table (2420/2424) and the login chooser (632/636, Q4).
  - `Down`/`Up` are swallowed in the consent pane, as `j`/`k` already are.
  - `F1` opens help from the agents forms, the paste field and the Qdrant clear question. Today
    those modes swallow it (2175, 835, 239).
  - ALT chords pass to the global layer from those modes, where nothing is bound by default.

## 5. CONTROL guards and ctrl chords

Guards the resolver and the modal-pass rule (Q1) make redundant. Delete them in the lane:
- agents.rs:834: `FieldOutcome::Pass if CONTROL => Pass` (paste field).
- agents.rs:2174: `_ if CONTROL => Pass` (`form_navigation`). The function becomes
  `form_navigation(act, &mut focus, len)` over `FormNextField`/`FormPrevField`. Its `_ => Consumed`
  moves to the modal rule.
- qdrant.rs:229: `if CONTROL { return Pass }` (`on_confirm_key`).
- qdrant.rs:224: `FieldOutcome::Pass => Handled::Pass` is replaced by the modal rule. This is
  defect 2.

Neither file binds a ctrl chord of its own (no `ctrl-s`, `ctrl-e`, ...). Every modifier-blind
browse/consent/chooser arm flagged `mb` in 2.2 and 3.2 stops firing on `ctrl-<letter>` by
construction. These are the L-A cases of defect 1:
- agents: `ctrl-y` confirms an install (1145); `ctrl-n` cancels a login (646); `ctrl-r` probes;
  `ctrl-a` logs in; `ctrl-e`, `ctrl-n` and `ctrl-t` write; `ctrl-x` cancels.
- qdrant: `ctrl-e` opens the editor; `ctrl-r` reloads.

## 6. Tests

**Hint assertions that change (old → new):**
- `tests/settings.rs:2229` `"j/k choose \u{b7} Enter select \u{b7} Esc cancel"` →
  `"j/k choose \u{b7} Enter select \u{b7} n/Esc cancel"`. This holds only if Q5 is accepted.
- In-file `qdrant.rs:577-591` `a_wide_notice_takes_the_hint_line_alone` calls `hint_text()` and
  `hint(width, theme)`. Change only the signature: pass `Keys::compiled()`. The assertions stay.

**Hint assertions that stay green with the specs above** (verify only):
- `settings.rs`: :1221, :1546, :1619, :1695, :1744, :1837, :2168, :2639, :2726, :2784, :2800,
  :2839, :2842 (test-local consts), :3612 `IDLE_KEYS`, :5043.
- `auth.rs`: :515, :557.
- `install.rs`: :467, :553, :580.
- `keys.rs`:127 (T1-owned): `ctrl_c_quits_from_qdrant_browse` asserts `"r reload · j/k rows"`,
  unchanged.

**Other lanes' files**: no assertion in them names L-A text. `connection.rs`:1571 and :1976 match
connection's own `Enter store` and `typed text is never shown`.

**Behaviour pins in L-A files that constrain the design** (must stay green unchanged):
- settings.rs:1447-1464. In consent, `j k r i x` → `Consumed` and `q g ?` → `Pass`. This forces
  the Q2 swallow.
- settings.rs:2351-2356. In the chooser, `1 2 9 q ? g` → `Pass`.
- settings.rs:2747-2766. `r`/`i` refusal sentences, both in browse-with-login and in the chooser.
- settings.rs:2919 and :3735. `ctrl-c` → `Handled::Pass` from the paste field and the agents form
  at section level. `ctrl-c` is never a candidate, so this forces the Q1 modal-pass rule.
- settings.rs:2900-2905, :3731-3733, :5047-5048. Letters are typed in the paste field and forms.
- settings.rs:2970. `p` → `Pass` outside a login.
- settings.rs:3788-3795 (App). `l` cycles again once the form closes.
- settings.rs:921-960 and :1014-1030. `l` cycles from agents browse, through the `settings` layer
  of `AGENTS_BROWSE`.

**Snapshots** (`settings__agents_*` ×10, `probe__agents_probed_missing`): **no change expected.**
- Every hint row they draw (`HINT_IDLE`, `HINT_EDITING`, `HINT_PASTING`) renders identically.
- The four App-level ones (`agents_demo`, `agents_empty`, `agents_unknown_row`,
  `probe__agents_probed_missing`) show the browse status line
  `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help`. `AGENTS_BROWSE`'s
  unfiltered global layer leaves it unchanged.
- No snapshot captures the chooser, the consent pane, Qdrant, or a capturing-mode status line.
- Any diff in these 11 is a defect. Still run the full `cargo insta test` (memory: grep
  undercounts).

## 7. Defect pins and the rebinding test (D14, L-A)

1. **Qdrant `Tab` keeps the tab.** Use an App `Harness` on the Settings tab focused on Qdrant (a
   `NotStored` snapshot auto-opens the URL editor at qdrant.rs:339-347, or press `e`). Press
   `tab` and then `backtab`: the active tab is still Settings, `captures_input()` is still true,
   and the editor is still drawn. Repeat once in the key editor (`j`, `e`). Section level:
   `bench.key(&mut s, "tab") == Handled::Consumed`.
2. **Agents `Down`/`Up` move the list.** With two rows (settings.rs `section_over`), `down` moves
   the accent to row 2 and `up` moves it back. Use the same row assertion as settings.rs:1470.
   Add the chooser: `down` and `up` change the chosen row (`j_k_and_enter_choose…` at
   settings.rs:2256 shape).
3. **Modifier-blind consent (L-A's defect-1 case).** In consent, `ctrl-y` sends no
   `InstallConfirm` and leaves the plan up. In agents browse, `ctrl-r` sends no `ProbeAgents`. In
   Qdrant browse, `ctrl-e` opens no editor.
4. **F1 in a capturing mode.** In the App, `F1` from the agents create form opens the `?` box and
   the form stays open. Today agents.rs:2175 swallows it.
5. **Rebinding test.** Build `Harness::with_keys(keys)` where `keys` carries
   `[settings.agents] probe = ["P"]`, on the probe.rs:71 rig. `P` sends `ProbeAgents`, `r` sends
   nothing, and the frame contains `P probe` and not `r probe`. This needs a public way to build
   `Keys` from an integration test (Q7).

## 8. Open questions for the foundation (T1) architect, with proposed answers

- **Q1. Modal mode, unresolved chords.** Proposal: a view in a modal mode (FORM, PASTE, CONFIRM,
  EDIT) offers the key to its `TextField` **first**. This is today's order, and it makes D13
  true by construction: an override cannot steal `Left`/`Right`/`Enter`. Only on
  `FieldOutcome::Pass` does it resolve the stack. Then:
  - a candidate → act on it, or return `Pass` for a global act;
  - no candidate → return `Pass` iff the stack's `global(modal)` filter admits the chord
    (CONTROL, ALT, F-n), else `Consumed`.

  T1 exposes that predicate (e.g. `Stack::passes(chord)`). Without it, settings.rs:2919 and
  :3735 fail, because `ctrl-c` is never a candidate.
- **Q2. Consent/chooser swallows** (1173, 656). Proposal: keep the swallowed acts out of the
  stacks, so the `?` box does not list dead keys. The view holds
  `CONSENT_SWALLOWS = [ListDown, ListUp, AgentsInstall, AgentsProbe, AgentsCancel]` and
  `CHOOSER_SWALLOWS = [AgentsAuthenticate, AgentsCancel]`. When the mode stack yields nothing,
  it returns `Consumed` if `keys.actions(AGENTS_BROWSE, chord)` hits one of them.
  - Alternative: put those layers in the stacks (simpler, but the `?` box shows `r probe` under
    a consent question).
  - Rejected: drop the swallow, which needs settings.rs:1447 changed against D14.
- **Q3. View layer is `Layer::only` in every stack.** Proposal: the sets in 2.3 and 3.3. Browse
  too, which deviates from D3's "view context", so overrides of unrelated shared acts are not
  admitted and the `?` box stays mode-accurate. T1's D10 "overridable" rule must then also
  check that the view layer admits the act.
- **Q4. Chooser gains `Down`/`Up`** through `list.down`/`list.up`. Proposal: accept. It sits
  under ANA §6.6's agents aliases and nothing pins `Down` in the chooser.
- **Q5. Chooser cancel hint.** Proposal: `All(ConfirmNo, "cancel")` → `n/Esc cancel`, and
  settings.rs:2229 changes. `One` would print `n cancel` and drop the `Esc` users know.
- **Q6. Hint priority.** Proposal: derive the agents hint from `self.stack()` first:
  - FORM → EDITING;
  - CONSENT → PENDING;
  - CHOOSER → CHOOSING;
  - PASTE → PASTING;
  - BROWSE → today's `(install, auth)` match.

  This fixes the Manual+login mismatch in 2.4. It is an edge case, and no test or snapshot pins
  the old line.
- **Q7. `Harness::with_keys` input.** `Keys::with_chords` is `#[cfg(test)] pub(crate)`
  (keys/mod.rs:109-110), so integration tests cannot use it. Proposal: expose
  `with_chords` under `cfg(any(test, feature = "testkit"))` as `pub`, or add
  `Keys::from_toml_str` through the loader. Optionally add `SectionBench::with_keys` for
  section-level rebinding.
- **Q8.** Manual-steps `Esc` maps to `common.dismiss`, not `common.back`. It closes an outcome
  pane, not a level.
- **Q9.** Install cancel and login cancel share one act, `settings.agents.cancel`. They mean the
  same thing, and the two states are exclusive (agents.rs:1112, 569).
- **Q10.** Chooser `Enter` is the view verb `settings.agents.choose`. If L-E's
  switcher/waiting `Enter` ends up a shared "open row" verb, the chooser could adopt it.
  Proposal: keep a view verb now to avoid cross-lane coupling.
- **Q11.** Test-double sections with no `key_stack` (settings.rs `ProbeSection` :815 and the
  capturing double at :879) must keep today's `h`/`l` cycling. This is in T1's
  `SettingsTab::on_key` fallback.
