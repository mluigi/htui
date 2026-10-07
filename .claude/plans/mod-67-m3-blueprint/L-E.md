# MOD-67 M3 blueprint — L-E: overlay key inventory

Lane L-E: `concepts_search.rs`, `waiting_list.rs`, `workspace_switcher.rs`, `migration_prompt.rs`
(all under `crates/htui/src/ui/overlay/`) plus `App::on_key` overlay routing in `crates/htui/src/app/state.rs`.

_Status: complete (§0-§13)._

Conventions: `file:line` is against `fcfb00e8`. `views::X` is a stack in the new `keys/views.rs`. New `Act`
variants are written `Act::ConceptsReindex` etc.; contexts `Context::Concepts` etc. "Today" = before M3.


## 0. Shell routing for overlays (`crates/htui/src/app/state.rs`)

Today (`App::on_key`, `state.rs:711-815`):

| Step | `state.rs` | What happens |
|---|---|---|
| 0 | `:720-723` | `chord == CTRL_C` → `Action::Quit`, before any overlay |
| 1 | `:725-759` | top overlay's `on_key(key, ctx.with_keys(keys))`; `Consumed` stops |
| 2 | `:760-765` | `apply_keys(Stack::OVERLAY, chord)`: `overlay.close` (`esc`), then `global ∩ {help}` (`?`, `f1`) |
| 3 | `:766-769` | modal swallow: `is_modal()` overlays (all four are modal) end the key here |
| 4-6 | `:772-814` | tab, legacy rows, `Stack::BASE` (never reached under any of the four overlays) |

M3 (D6), as it affects L-E (T1 implements the shell part; L-E only implements `Overlay::key_stack`):

- Step 2 becomes `apply_keys(top.key_stack().unwrap_or(Stack::OVERLAY), chord)`. The overlay step stays
  *after* the overlay's `on_key` and *before* the swallow, so the order is unchanged.
- `Stack::OVERLAY` stays in `DECLARED` ("over an overlay"): it is the fallback for an unconverted overlay
  and for the test doubles (`Probe`, `Popup`, `Asking`). **T7's reachability test must exempt `BASE` and
  `OVERLAY`** (shell fallbacks, not returned by any `key_stack`).
- Each overlay has exactly **one** mode for key purposes (see §1-§4), so each `key_stack` is a constant
  `Some(views::…)`. No overlay needs state to pick a stack.
- View-side contract for all four (D6): `let chord = KeyChord::from_event(key);` then
  `for act in ctx.keys().actions(views::X, chord)`: handle the overlay's own acts and return `Consumed`;
  on any `Act::OverlayClose` / `Act::Help` candidate return `Handled::Pass` (the shell applies it in step
  2); after the loop, concepts gives the key to its `TextField`, the other three return `Pass` (swallowed
  in step 3).
- `action_for` (`state.rs:346-363`) already maps `OverlayClose` and `Help`; it returns `None` for every
  overlay-context act, so step 2 never double-applies an act the view declined.
- Status line (D7) and `?` box (D8) read `active_stack()` = the top overlay's `key_stack` → see §6.

## 1. `crates/htui/src/ui/overlay/concepts_search.rs` (`ConceptsSearch`, context `concepts`)

### Modes
| Mode | Modal | Captures | Notes |
|---|---|---|---|
| query (only mode) | `is_modal` true (`:282-284`) | **yes, always**: the `TextField` (`field`, `:91`) has focus for the box's whole life; searching/indexing/answered states change no key | `?` is query text, `F1` is help (pinned by `tests/keys.rs:249-270`, `tests/concepts_search.rs:181-192`); `Enter` is the field's Submit (widget-fixed, D13/§6.1), `Esc` the field's Cancel, returned as `Pass` (`:334-335`) so `overlay.close` closes |

Widget-owned keys that stay fixed (D13): every printable char, `Backspace`, `Delete`, `Left`, `Right`,
`Home`, `End` (`text_field.rs:145-200`), `Enter` → `FieldOutcome::Submit` → `ConceptsSearch::enter`
(search, else open the highlighted hit), `Esc` → `FieldOutcome::Cancel`. Bracketed paste (`on_paste`,
`:341-345`) is not a key.

### Key-match sites
| `file:line` | Chord(s) today (mods checked) | Guard | Behaviour | Proposed mapping `(context, name, defaults, help, in_capture)` |
|---|---|---|---|---|
| `:292-293`, `:295` | `ctrl-d`, also `ctrl-D` (`modifiers - SHIFT == CONTROL`) | none | toggle `decisions`, `Consumed` | `Act::ConceptsDecisions` = `(concepts, "decisions", ["ctrl-d"], "decisions only", true)` — **new view verb** |
| `:299` | `ctrl-p`, `ctrl-P` | none | `cycle_project(ctx.projects)` | `Act::ConceptsProject` = `(concepts, "project", ["ctrl-p"], "cycle project scope", true)` — new view verb |
| `:303` | `ctrl-r`, `ctrl-R` | none | `reindex(ctx)` (`StoreRequest::IndexConcepts`, or the `NO_PROJECTS` notice) | `Act::ConceptsReindex` = `(concepts, "reindex", ["ctrl-r"], "re-index scope", true)` — the ANA-26 §7.2 name |
| `:310-313` | any other non-SHIFT modifier | `!chord.is_empty()` | `Pass` (D261: lets `ctrl-c` and `ctrl-f` etc. reach the shell) | **removed** (redundant: `ctrl-c` is shell step 0; `TextField::on_key` itself passes CONTROL/ALT/SUPER/META/HYPER, `text_field.rs:145-154`) |
| `:315` | `Up` (SHIFT ignored, other mods excluded by `:311`) | none | cursor −1 (saturating) | `Act::ConceptsUp` = `(concepts, "up", ["up"], "previous hit", true)` — see Q2 for why not `list.up` |
| `:319` | `Down` (same) | none | cursor +1, clamped to `hits.len()-1` | `Act::ConceptsDown` = `(concepts, "down", ["down"], "next hit", true)` |
| `:325-336` | everything else, no mods | — | `field.on_key`: `Consumed` → clear notice; `Submit` → `enter`; `Cancel`/`Pass` → `Handled::Pass` | unchanged widget fall-through (D13); `Esc` reaches `overlay.close` via shell step 2 |

Shared verbs used: none besides `overlay.close` (`esc`, in_capture) and `global.help` (`f1` survives the
modal filter; `?` is filtered out and typed). New view verbs: 5 (all `capture_row`, all non-printable, so
the §6.4 validator rule holds and a user cannot bind a printable chord to them).

Dispatch sketch (replaces `:291-339`):
```rust
let chord = KeyChord::from_event(key);
for act in ctx.keys().actions(views::CONCEPTS_QUERY, chord) {
    match act {
        Act::ConceptsDecisions => { self.decisions = !self.decisions; return Handled::Consumed }
        Act::ConceptsProject   => { self.cycle_project(ctx.projects); return Handled::Consumed }
        Act::ConceptsReindex   => { self.reindex(ctx); return Handled::Consumed }
        Act::ConceptsUp        => { self.cursor = self.cursor.saturating_sub(1); return Handled::Consumed }
        Act::ConceptsDown      => { /* clamp as :319-323 */ return Handled::Consumed }
        _ => return Handled::Pass, // overlay.close / global.help: the shell's (step 2)
    }
}
match self.field.on_key(key) { /* :325-336 unchanged */ }
```
Order matters: the resolver runs **before** the field, so a rebound `concepts.*` named key (e.g. `f5`)
acts; printable chords never resolve here because every layer of this stack is in_capture or modal-filtered.

### Behaviour deltas (intended, by construction)
- `ctrl-D`/`ctrl-P`/`ctrl-R` (kitty-style reports of ctrl-shift-letter as `Char('D')`+CONTROL) stop
  acting: `KeyChord::new` keeps `Char('D')` and `ctrl-D` is unparseable (`refuse_char`, `chord.rs:241`, ctrl-capital branch `:280`).
  The legacy encoding delivers ctrl-shift-d as `ctrl-d` anyway. Same issue as `form.save`'s `ctrl-S`
  (catalogue.rs:400, "blueprint F-7, M4") → **Q4**.
- `Shift+Up`/`Shift+Down` stop moving the cursor (`KeyChord::new` keeps SHIFT on non-char keys) and are
  swallowed. Accept; note in the lane report.

### Imports
`KeyCode`/`KeyModifiers` leave the non-test import (`:15`); the test module (`use super::*`, `Bench::code`
`:585`, `Bench::ctrl` `:589`) must import `crossterm::event::{KeyCode, KeyModifiers}` itself, or the
featureless clippy gate / test build fails.

## 2. `crates/htui/src/ui/overlay/waiting_list.rs` (`WaitingList`, context `waiting`)

### Modes
| Mode | Modal | Captures | Notes |
|---|---|---|---|
| list (only mode; "reading", empty and populated render differently but key the same) | `is_modal` true (`:257-259`) | no | rows come from `ctx.top_bar.waiting` (`:267-271`); `Enter` on an empty list is consumed and emits nothing (`:111-113`) |

No widget-owned keys.

### Key-match sites
| `file:line` | Chord(s) today (mods checked) | Guard | Behaviour | Proposed mapping |
|---|---|---|---|---|
| `:274` | `j`, `Down` — **modifier-blind** (`ctrl-j`, `alt-j`, `shift-down`, `ctrl-down` also move) | none | `put(rows, at+1 clamped)`; `Consumed` (also on an empty list: `map_or(0, …)`) | shared `list.down` (`["j","down"]`, catalogue already cites `waiting_list.rs:274`) |
| `:279` | `k`, `Up` — modifier-blind | none | `put(rows, at-1 saturating)` | shared `list.up` (`["k","up"]`) |
| `:284` | `Enter` — modifier-blind | none | `enter`: `Overlay(Close)` then `Reveal(Step{…})`, nothing on an empty list | **new view verb** `Act::WaitingOpen` = `(waiting, "open", ["enter"], "open step", false)` |
| `:288-290` | everything else (incl. `Esc`) | — | `Pass` → `Esc` closes via `overlay.close`; rest swallowed | unchanged (`Pass` after the loop) |

Not `list.fold` for `Enter`: the catalogue's own comment (`catalogue.rs`, `[list]` fold row) already
classifies non-fold `Enter`s as view verbs, and `list` is narrowed to `{down, up}` here so `fold` never
competes.

Dispatch: `for act in ctx.keys().actions(views::WAITING_LIST, chord)` → `ListDown`/`ListUp`/`WaitingOpen`
handled as today, `_ => return Handled::Pass`; after the loop `Handled::Pass`. `KeyCode` leaves the
non-test import (`:21`); the test module already imports it (`:341`).

## 3. `crates/htui/src/ui/overlay/workspace_switcher.rs` (`WorkspaceSwitcher`, context `switcher`)

### Modes
| Mode | Modal | Captures | Notes |
|---|---|---|---|
| list (only mode; "reading the store", empty and loaded key the same) | `is_modal` true (`:152-154`) | no | the startup overlay over an empty store (`testkit.rs:918-920`, `tests/integration.rs:140`) |

No widget-owned keys.

### Key-match sites
| `file:line` | Chord(s) today | Guard | Behaviour | Proposed mapping |
|---|---|---|---|---|
| `:163` | `j`, `Down` — modifier-blind | none | `down()` (no wrap) | shared `list.down` (catalogue cites `workspace_switcher.rs:163`) |
| `:167` | `k`, `Up` — modifier-blind | none | `up()` | shared `list.up` |
| `:171` | `Enter` — modifier-blind | none | `enter`: `SetScope{workspace}` then `Overlay(Close)`; `Consumed` even on an empty list | **new view verb** `Act::SwitcherSwitch` = `(switcher, "switch", ["enter"], "switch workspace", false)` (catalogue `[list]` fold comment already names `workspace_switcher.rs:171` a view verb) |
| `:172-174` | everything else (incl. `Esc`) | — | `Pass` | unchanged |

`enter` (`:78-86`) keeps returning `Consumed`; the dispatch arm returns its value. `KeyCode` leaves the
import (`:22`); the in-file tests (`:240-`) do not use it.

## 4. `crates/htui/src/ui/overlay/migration_prompt.rs` (`MigrationPrompt`, context `migration`)

### Modes
| Mode | Modal | Captures | Notes |
|---|---|---|---|
| confirm (only mode; "reading the store", "up to date" and "N pending" key the same) | `is_modal` true (`:80-82`) | no — but a **confirming** mode in D5's sense? See Q3: proposed **no** modal filter, since `global ∩ {help}` admits nothing printable that collides | `y` applies even before the count lands or when it is 0 (today's behaviour, unchanged; noted only) |

### Key-match sites
| `file:line` | Chord(s) today | Guard | Behaviour | Proposed mapping |
|---|---|---|---|---|
| `:92` | `y`, `Y` — **modifier-blind: `ctrl-y` and `alt-y` apply migrations today** | none | `Store(ApplyMigrations)` + `Overlay(Close)` | shared `confirm.yes` with `VIEW_DEFAULTS (Context::Migration, Act::ConfirmYes, &["y", "Y"])` (D12) |
| `:97` | `n`, `N` — modifier-blind (`ctrl-n` closes) | none | `Overlay(Close)` | shared `confirm.no` with `VIEW_DEFAULTS (Context::Migration, Act::ConfirmNo, &["n", "N"])` — **`esc` deliberately dropped**, see collisions §7 |
| `:102-104` | everything else (incl. `Esc`) | — | `Pass` → `overlay.close` closes (= `n`, pinned `tests/shell.rs:270-284`) | unchanged |

No own acts: `Context::Migration` exists only to carry the two `VIEW_DEFAULTS` rows and as the D10
override target (`[migration] yes = "a"`). `KeyCode` leaves the import (`:22`).

Why `["n","N"]` and not `["n","esc","N"]`: (a) `esc` would collide with `overlay.close` in the same
stack (needs a `SHADOWING` entry `(ConfirmNo, OverlayClose)`), (b) the hint `n / Esc stay offline` maps
exactly to `Hint::Pair(ConfirmNo, OverlayClose, …)` = `n/Esc stay offline` (D9's predicted drift), while
`Hint::All(ConfirmNo, …)` over `["n","esc","N"]` would render `n/Esc/N`, (c) the effect of `Esc` is
identical (close, nothing applied) and a rebound `overlay.close` moves it consistently.

## 5. Stacks (`crates/htui/src/keys/views.rs`, T1) and `key_stack`

| Constant | Overlay → `key_stack()` | Layers, narrowest first | `DECLARED` phrase |
|---|---|---|---|
| `CONCEPTS_QUERY` | `ConceptsSearch` → `Some(views::CONCEPTS_QUERY)` | `Layer::all(Context::Concepts)`, `Layer::all(Context::Overlay)`, `Layer::only(Context::Global, &[Act::Help])` **+ modal filter** (Q1) | `"in the concepts search"` |
| `SWITCHER_BROWSE` | `WorkspaceSwitcher` → `Some(views::SWITCHER_BROWSE)` | `Layer::all(Context::Switcher)`, `Layer::only(Context::List, &[Act::ListDown, Act::ListUp])`, `Layer::all(Context::Overlay)`, `Layer::only(Context::Global, &[Act::Help])` | `"in the workspace switcher"` |
| `WAITING_LIST` | `WaitingList` → `Some(views::WAITING_LIST)` | `Layer::all(Context::Waiting)`, `Layer::only(Context::List, &[Act::ListDown, Act::ListUp])`, `Layer::all(Context::Overlay)`, `Layer::only(Context::Global, &[Act::Help])` | `"in the waiting list"` |
| `MIGRATION_CONFIRM` | `MigrationPrompt` → `Some(views::MIGRATION_CONFIRM)` | `Layer::all(Context::Migration)`, `Layer::all(Context::Confirm)`, `Layer::all(Context::Overlay)`, `Layer::only(Context::Global, &[Act::Help])` | `"in the migration prompt"` |

Shape decisions:
- **Overlay layer after the shared layers, before global** (view → shared → `overlay` → `global ∩ {help}`):
  it mirrors today's order (overlay `on_key` first, then `Stack::OVERLAY`), and it makes a user line such
  as `[list] down = ["esc"]` a validator error in `SWITCHER_BROWSE`/`WAITING_LIST` (a user-created pair is
  always an error, ANA §7.4 step 7) instead of silently stealing the only close key.
- **Global stays `only {help}`** (today's `Stack::OVERLAY`): `q` is swallowed (`tests/shell.rs:288-297`),
  digits and `w` are swallowed (`tests/keys.rs:214-237`), and `ctrl-f`/`ctrl-w`/`Tab` do nothing over an
  overlay today. Widening it is a behaviour change, out of M3.
- `list` narrowed to `{down, up}`: `top`/`bottom` (`g`/`G`/Home/End) and `fold` (Enter) are not offered
  today; offering them is a new feature (Q6).
- `MIGRATION_CONFIRM` is a *confirming* mode; its global layer still needs no modal filter because
  `only {help}` admits only `?`/`f1`, and `?` collides with nothing there (Q3).
- Resolution with defaults (validator sanity): `CONCEPTS_QUERY` ctrl-d/ctrl-p/ctrl-r/up/down → own acts,
  esc → `OverlayClose`, f1 → `Help`, `?` → `[]` (filtered, typed). `SWITCHER_BROWSE`/`WAITING_LIST`
  j/down, k/up → list, enter → own verb, esc → close, `?`/f1 → help. `MIGRATION_CONFIRM` y/Y, n/N →
  migration rows, esc → close, `?`/f1 → help. **No collision; no `SHADOWING` entry needed by L-E.**

## 6. Catalogue additions (T1) — the complete L-E list

`Context` gains `Concepts`, `Switcher`, `Migration`, `Waiting` (tables `concepts`, `switcher`,
`migration`, `waiting`; headings = overlay titles per D1: `Search concepts` (`concepts_search.rs:34`),
`Workspaces` (`workspace_switcher.rs:149`), `Schema` (`migration_prompt.rs:77`), `Waiting on you`
(`waiting_list.rs:254`)). Blocks in D1 order, each row citing its arm:

| Act | Row | Cites |
|---|---|---|
| `ConceptsDecisions` | `capture_row(.., Concepts, "decisions", &["ctrl-d"], "decisions only")` | `concepts_search.rs:295` |
| `ConceptsProject` | `capture_row(.., Concepts, "project", &["ctrl-p"], "cycle project scope")` | `:299` |
| `ConceptsReindex` | `capture_row(.., Concepts, "reindex", &["ctrl-r"], "re-index scope")` | `:303` |
| `ConceptsUp` | `capture_row(.., Concepts, "up", &["up"], "previous hit")` | `:315` |
| `ConceptsDown` | `capture_row(.., Concepts, "down", &["down"], "next hit")` | `:319` |
| `SwitcherSwitch` | `row(.., Switcher, "switch", &["enter"], "switch workspace")` | `workspace_switcher.rs:171` |
| `WaitingOpen` | `row(.., Waiting, "open", &["enter"], "open step")` | `waiting_list.rs:284` |

`VIEW_DEFAULTS` (D12): `(Context::Migration, Act::ConfirmYes, &["y", "Y"])` (`migration_prompt.rs:92`),
`(Context::Migration, Act::ConfirmNo, &["n", "N"])` (`:97`). Catalogue comment on `confirm.yes`/`no`
("migration_prompt.rs:92 also takes `Y`") is updated to point at `VIEW_DEFAULTS`.

Knock-on T1 test edits: `ALL`/`position` (+7 acts), `every_act_has_exactly_one_row` count (+7 from L-E),
`context_tables_and_headings` (+4), `only_the_form_and_overlay_close_are_in_capture` must admit the five
`Concepts*` acts (rename to e.g. `in_capture_acts_are_form_overlay_close_and_concepts`).
`no_spec_is_shared_in_a_context_unless_state_guarded`: `SwitcherSwitch`/`WaitingOpen` share `enter`
with `list.fold` but in **different contexts**, so it passes.

## 7. Hints (`HINT` constants → `HintSpec`, D9)

| `file:line` | Today (exact) | `HintSpec` | Rendered with defaults | Diff |
|---|---|---|---|---|
| `concepts_search.rs:85-86` (drawn `:457`) | `Enter search/open  Up/Dn move  Ctrl+D decisions  Ctrl+P project  Ctrl+R re-index  Esc close` | `[Text("Enter search/open"), Pair(ConceptsUp, ConceptsDown, "move"), One(ConceptsDecisions, "decisions"), One(ConceptsProject, "scope"), One(ConceptsReindex, "index"), One(OverlayClose, "close")]` | `Enter search/open · Up/Down move · Ctrl+d decisions · Ctrl+p scope · Ctrl+r index · Esc close` (93 cells) | separators `  `→` · `, `Up/Dn`→`Up/Down`, `Ctrl+D`→`Ctrl+d` etc., **and two labels shortened** (`project`→`scope`, `re-index`→`index`): the faithful text is 98 cells and the box's inside is 94 at 100 columns (`INNER_CELLS`, `tests/concepts_search.rs:34`), so `Esc close` would be clipped and `concepts_search.rs:1053-1071` (`a_short_error_stays_on_its_row`, finds `Esc close`) would fail. Q5 |
| `waiting_list.rs:37` (drawn `:168`) | `j/k move · Enter open · Esc close` | `[Pair(ListDown, ListUp, "move"), One(WaitingOpen, "open"), One(OverlayClose, "close")]` | `j/k move · Enter open · Esc close` | none |
| `workspace_switcher.rs:32` (drawn `:109`) | `j/k move · Enter switch · Esc close` | `[Pair(ListDown, ListUp, "move"), One(SwitcherSwitch, "switch"), One(OverlayClose, "close")]` | `j/k move · Enter switch · Esc close` | none |
| `migration_prompt.rs:26` (drawn `:56`) | `y apply · n / Esc stay offline` | `[One(ConfirmYes, "apply"), Pair(ConfirmNo, OverlayClose, "stay offline")]` | `y apply · n/Esc stay offline` | `n / Esc`→`n/Esc` (D9's predicted drift) |

Plumbing: the three list overlays build the hint inside a `&self` helper that has no `Ctx`
(`WaitingList::lines` `:133`, `WorkspaceSwitcher::lines` `:89`, `MigrationPrompt::lines` `:52`). Add a
`hint: &str` parameter (rendered once in `render` via `ctx.keys().hint(views::X, HINT)`); in-file
callers to update: `waiting_list.rs:306`, `:319`, `:647`, `:663`; `workspace_switcher.rs:193`, `:264`,
`:286`, `:304`; `migration_prompt.rs:119`. The box widths are measured from these lines, so a rebound
longer label widens the box (correct). `waiting_list.rs:683` asserts `rendered.contains(HINT)`: compare
against `Keys::compiled().hint(views::WAITING_LIST, HINT)` instead.

Prose naming keys (left for M6 per the plan; listed so M6 finds them):
- `concepts_search.rs:79` `NO_MATCHES` "…`Ctrl+R` indexes this scope…" → `keys.label(CONCEPTS_QUERY, ConceptsReindex)`;
  after M3 it also disagrees in spelling with the hint (`Ctrl+r`).
- `concepts_search.rs:70` `IDLE`, `:82` `CHANGED`: name `Enter`, which is widget-fixed — no change ever.
- `workspace_switcher.rs:115` "no workspaces — `N` in Settings > Hierarchy creates one": names L-C's
  hierarchy new-workspace act (`hierarchy.rs:1220`) → M6 `keys.label(views::HIERARCHY_…, …)`.
- Module/field docs (`concepts_search.rs:3-7`, `:94-113`, `migration_prompt.rs:3-4`, `:24-25`,
  `waiting_list.rs:36`, `workspace_switcher.rs:31`): update the "wildcard overlay binding" wording to
  `overlay.close` while editing the constants.

## 8. Collisions in L-E stacks

| Stack | Chord | Candidates (defaults) | Resolution |
|---|---|---|---|
| `MIGRATION_CONFIRM` | `esc` | would be `confirm.no` + `overlay.close` if migration kept the shared `["n","esc"]` | **avoided**: `VIEW_DEFAULTS (Migration, ConfirmNo, ["n","N"])` drops `esc`; `Esc` stays `overlay.close` (same effect). Rejected alternative: keep `esc` and add `SHADOWING (ConfirmNo, OverlayClose)` — more allow-list, and the hint becomes `n/Esc/N` |
| `SWITCHER_BROWSE` / `WAITING_LIST` | `enter` | `switcher.switch` / `waiting.open` only | `list.fold` (also `enter`) is excluded by `list ∩ {down, up}`; no entry |
| `CONCEPTS_QUERY` | `?` | `global.help` | removed by the modal filter → typed text (pinned `tests/keys.rs:249-271`, `tests/concepts_search.rs:181-192`) |
| `CONCEPTS_QUERY` | `up`/`down` | `concepts.up`/`concepts.down` only | `list` is not in this stack (Q2) |
| all four | `esc`, `?`, `f1` | `overlay.close`, `global.help` | distinct; unchanged from `Stack::OVERLAY` |

`STATE_GUARDED`: no new pair. `SHADOWING` (D11): no L-E pair. `VIEW_DEFAULTS`: the two migration rows.

## 9. CONTROL guards and modifier-blind arms

Guards made redundant (delete):
- `concepts_search.rs:292` `let chord = key.modifiers - KeyModifiers::SHIFT;`, `:293` `if chord ==
  KeyModifiers::CONTROL`, `:310-313` `if !chord.is_empty() { return Handled::Pass; }` (MOD-52/D261
  pass-through; the shell checks `ctrl-c` first and `TextField` passes chords itself).

Ctrl chords the overlays use: `ctrl-d`, `ctrl-p`, `ctrl-r` (concepts only). Global ctrl chords stay
unreachable over every overlay (`ctrl-f`, `ctrl-w`, as today). `ctrl-c` is shell step 0.

Modifier-blind arms fixed by construction (chord equality includes modifiers, D6) — today's defects:
- `migration_prompt.rs:92`: **`ctrl-y`/`alt-y` apply pending schema migrations** (the worst one: an
  irreversible write on a chord nobody advertises); `:97`: `ctrl-n`/`alt-n` close.
- `workspace_switcher.rs:163`/`:167`/`:171` and `waiting_list.rs:274`/`:279`/`:284`: `ctrl-j`, `alt-j`,
  `ctrl-k`, `ctrl-down`, `shift-up`… move; `alt-enter`/`ctrl-enter` (where delivered) switch/open.
- `concepts_search.rs:315`/`:319`: `shift-up`/`shift-down` move (SHIFT was stripped at `:292`).
After M3 all of these are swallowed by the modal overlay.

## 10. Status line and `?` box with an L-E overlay up (D7, D8)

Status line (`App::render`, `state.rs:859-866` → `active_stack()`'s global layer):

| Overlay on top | Today (100-col cut, row 34 of every overlay snapshot) | Proposed (Q3) |
|---|---|---|
| switcher, waiting, migration | `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help · w workspaces · Ctrl+f find` | `Ctrl+c quit · ? help` |
| concepts | same | `Ctrl+c quit · F1 help` |

`?` box (`render_help`, `state.rs:883-920`, D8: one line per layer, filtered, legacy tab line dropped
because the active stack is the overlay's), at 100 columns (inner 88):

| Overlay | Lines (narrowest first) |
|---|---|
| switcher | `Workspaces: Enter switch workspace` · `List: j/Down down · k/Up up` · `Overlay: Esc close` · `Global: Ctrl+c quit · ?/F1 help` · closer `?/F1 closes this box` |
| waiting | `Waiting on you: Enter open step` · `List: j/Down down · k/Up up` · `Overlay: Esc close` · `Global: Ctrl+c quit · ?/F1 help` · closer |
| migration | `Schema: y/Y yes · n/N no` · (Confirm line omitted: both rows shadowed by the migration rows) · `Overlay: Esc close` · `Global: Ctrl+c quit · ?/F1 help` · closer |
| concepts | `Search concepts: Ctrl+d decisions only · Ctrl+p cycle project scope · Ctrl+r re-index scope · Up previous hit · Down next hit` (packs to 2 rows) · `Overlay: Esc close` · `Global: Ctrl+c quit · F1 help` · closer **`F1 closes this box`** (Q7) |

Today the box over any overlay is `Overlay: Esc close`, then the active tab's legacy line (e.g. Backlog's),
then the full `Global:` line; no snapshot pins it (only substring asserts, §11).

## 11. Tests and snapshots affected

### Snapshots (re-baseline; only the rows named may move)
| Snapshot | Test | Row(s) that change | Expected change |
|---|---|---|---|
| `tests/snapshots/concepts_search__hits.snap` | `tests/concepts_search.rs:194-214` | 27 (hint), 34 (status) | hint → `Enter search/open · Up/Down move · Ctrl+d decisions · Ctrl+p scope · Ctrl+r index · Esc close`; status → `Ctrl+c quit · F1 help` |
| `concepts_search__decisions_project.snap` | `:299-321` | 27, 34 | same |
| `concepts_search__error.snap` | `:324-336` | 27, 34 | same |
| `concepts_search__empty.snap` | `:398-404` | 27, 34 | same |
| `concepts_search__searching.snap` | `:407-415` | 27, 34 | same |
| `waiting__graphics_empty.snap` | `tests/waiting.rs:301-308` | 34 | status → `Ctrl+c quit · ? help`; hint row 21 identical |
| `waiting__platform_mixed.snap` | `:313-332` | 34 | same |
| `shell__switcher_open.snap` | `tests/shell.rs:56-67` | 34 | status → `Ctrl+c quit · ? help` |
| `shell__switcher_empty.snap` | `:175-194` | 34 | same |
| `shell__migration_prompt.snap` | `:215-230` | 21 (hint), 34 | hint → `y apply · n/Esc stay offline` (box width unchanged: it is sized from the widest line, the 48-cell question); status → `Ctrl+c quit · ? help` |
| **`src/snapshots/htui__testkit__tests__shell_empty.snap`** (not in L-E's list; T1-owned `testkit.rs:784`) | `testkit.rs` `shell_empty` test | 34 (status; the startup switcher is up) | `Ctrl+c quit · ? help`. L-E re-baselines it and flags it (plan lane rule) — add it to L-E's owned snapshots |
| `src/ui/overlay/snapshots/htui__ui__overlay__waiting_list__tests__offline.snap` | `waiting_list.rs:686-698` | none | overlay rendered alone (no status row), hint identical |
| `shell__after_switch.snap`, `shell__offline_label.snap`, `integration__demo_shell.snap` | — | none | no overlay up |

Count: 11 snapshots change (5 concepts, 2 waiting, 3 shell, 1 testkit).

### Assertions
| Location | Assertion | Effect |
|---|---|---|
| `crates/htui/src/ui/overlay/waiting_list.rs:683` | `rendered.contains(HINT)` | **breaks** (HINT is a spec): compare with `Keys::compiled().hint(views::WAITING_LIST, HINT)` |
| `waiting_list.rs:647`, `:663`; `workspace_switcher.rs:264`, `:286`, `:304` | `lines(…)` calls | signature gains `hint: &str` (§7) |
| `waiting_list.rs:655` | literal `"  j/k move · Enter open · Esc close"` | stays (identical render) |
| `concepts_search.rs:1053-1071` | `line.contains("Esc close")` | stays only with the 93-cell hint (Q5) |
| `concepts_search.rs:963-972` | `ctrl-c` → `Pass`, `Esc` → `Pass`, `wq1?jk` typed | stays (ctrl-c: field passes; esc: `OverlayClose` candidate → `Pass`) |
| `concepts_search.rs` other in-file tests (`:681-960`, `Bench::ctrl` `:589`) | `ctrl-d/p/r`, `Up`/`Down`, `Enter` | stay (compiled keys via `Ctx::new`, `state.rs:116`) |
| `tests/shell.rs:223` | `contains("y apply")` | stays |
| `tests/shell.rs:86-107`, `:110-150`, `:153-172`, `:188-192`, `:233-248`, `:250-267`, `:270-285`, `:288-297` | `j`/`k`/`enter`/`esc`/`n`/`y`/`q`/`ctrl-c` behaviour | stay (defaults unchanged) |
| `tests/keys.rs:203` (T1) | `contains("Overlay: Esc close")` over the switcher | stays (overlay layer line) |
| `tests/keys.rs:204` (T1) | `contains("?/F1 closes this box")` over the switcher | stays (switcher's global layer is not modal) |
| `tests/keys.rs:214-237` (T1) | `2`/`w` swallowed by the switcher | stays (global `only {help}`) |
| `tests/keys.rs:249-271` (T1) | `?` typed in concepts, `F1` opens help | stays **only if** the modal filter is on `CONCEPTS_QUERY`'s global layer — this test is the guard for Q1 |
| `tests/keys.rs:139-170` (T1) | `ctrl-c` over all four overlays | stays |
| `tests/keys_file.rs:356`, `:366` (T1) | `F2` closes the switcher; `Overlay: Esc/F2 close` | stays (`overlay.close` resolves in `SWITCHER_BROWSE`) |
| `tests/concepts_search.rs`, `tests/waiting.rs`, `tests/integration.rs` behaviour asserts | — | stay |

## 12. Pins and tests to add (D14)

| File | Test | Asserts |
|---|---|---|
| `tests/shell.rs` | `ctrl_y_and_alt_n_do_not_answer_the_migration_prompt` | `shell_reporting("online", Some(3))`; `key("ctrl-y")`, `key("alt-n")`, `drive_to_end`: one overlay still up, `status == None` (no `applied 0 migration(s)`) |
| `tests/shell.rs` | `capital_y_applies_and_capital_n_declines` | pins the `VIEW_DEFAULTS` rows: `Y` → overlay gone + `applied 0 migration(s)`; fresh harness, `N` → overlay gone, status `None` |
| `tests/shell.rs` | `ctrl_j_does_not_move_the_switcher` | `open_over_demo`; `key("ctrl-j")`: frame still `> Graphics`; then `j` moves (control) |
| `tests/shell.rs` | `the_status_line_and_help_box_over_the_switcher` | status line (last non-empty line) `== "Ctrl+c quit · ? help"`; after `?`: frame contains `Workspaces: Enter switch workspace`, `List: j/Down down · k/Up up`, `Overlay: Esc close`, `Global: Ctrl+c quit · ?/F1 help`, and not the Backlog legacy line (`Backlog: `) |
| `tests/concepts_search.rs` | `the_status_line_and_help_box_over_the_search_name_f1` | after `ctrl-f`: status `== "Ctrl+c quit · F1 help"`; after `f1`: contains `Search concepts: Ctrl+d decisions only`, `Global: Ctrl+c quit · F1 help`, `F1 closes this box`, not `?/F1` |
| `tests/concepts_search.rs` | `a_rebound_reindex_acts_and_the_old_chord_is_inert` (the lane's rebinding test, `Harness::with_keys`) | keys from `"version = 1\n[concepts]\nreindex = \"f5\"\n"`; `ctrl-f`, `ctrl-r`, `drive_to_end`: no `INDEXING`/report, query field unchanged; `f5`: report line appears; hint row contains `F5 index` and not `Ctrl+r` |
| `tests/shell.rs` (optional second) | `a_migration_override_rebinds_yes` | `[migration]\nyes = "a"` (D10 override over a `VIEW_DEFAULTS` row): `y`/`Y` inert, `a` applies, hint `a apply · n/Esc stay offline` |
| `keys/views.rs` unit tests (T1, suggested) | resolution table of §5 | e.g. `actions(CONCEPTS_QUERY, "?") == []`, `"f1" == [Help]`, `"esc" == [OverlayClose]`; `actions(MIGRATION_CONFIRM, "Y") == [ConfirmYes]`, `"esc" == [OverlayClose]` |

All App-level tests sit behind `#![cfg(feature = "testkit")]` (already the case in the four owned files).

## 13. Open questions for the foundation architect (with proposed answers)

1. **Q1 — `only` and `modal` on one layer.** `CONCEPTS_QUERY` needs `global ∩ {help}` *and* D5's filter
   (else `?` resolves to `Help`, the view passes it, and `tests/keys.rs:249-271` fails). D5 only names
   `Layer::modal(Context::Global)`. Proposal: `Layer` gains a `modal: bool` and a `const fn modal(self) ->
   Self` builder, so `Layer::only(Context::Global, &[Act::Help]).modal()`; `admits`/`actions`/`label`/
   `hint`/validator/status line all consult it.
2. **Q2 — concepts `Up`/`Down`: own acts or `list.*`?** Proposal: own `concepts.up`/`concepts.down`
   (`capture_row`, `["up"]`/`["down"]`). `in_capture` is per act, `list.down` is not in-capture (it
   carries `j`), so a `list.*` row (via `VIEW_DEFAULTS` or a `[concepts] down` override) would let a user
   bind a printable chord unchecked and steal typed text. Cost: a `[list]` rebinding does not reach the
   search; acceptable (it has no `j`/`k` today).
3. **Q3 — status line over an overlay, and is the migration prompt "confirming"?** D7 says "quit is
   always shown" and cites `? help`. Proposal: `Ctrl+c quit · ? help` (switcher, waiting, migration) and
   `Ctrl+c quit · F1 help` (concepts). Do **not** put the modal filter on `MIGRATION_CONFIRM`'s global
   layer: `only {help}` already admits nothing that leaks, and `?` toggles help over the prompt today.
4. **Q4 — ctrl-capital chords.** Concepts accepts `ctrl-D`/`ctrl-P`/`ctrl-R` today (`:295-303`), like
   `form.save`'s `ctrl-S` (`catalogue.rs:400`). Proposal: `KeyChord::new` folds `Char(A-Z)` + CONTROL to
   lower case (the capital is unrepresentable in the file anyway, `chord.rs:280`), once, for every lane.
   Otherwise the lane accepts the narrowing and the README notes it.
5. **Q5 — concepts hint text.** The faithful conversion is 98 cells; the box's inside is 94 at 100
   columns, so `Esc close` is clipped. Proposal: labels `scope` (`Ctrl+p`) and `index` (`Ctrl+r`), 93
   cells (§7). Alternatives: `Enter search` instead of `Enter search/open` (93 cells, loses "open");
   accept the clip (breaks `concepts_search.rs:1053-1071`); a second hint row (layout change, D9 forbids).
   Needs the maintainer's nod as text drift beyond spelling.
6. **Q6 — `list.top`/`bottom` in the switcher and the waiting list?** Proposal: no (no new defaults in M3
   beyond D12); `list ∩ {down, up}`.
7. **Q7 — the `?` box closer under a capturing overlay.** `help_closer` (`hint.rs`) reads
   `global.help`'s chords unfiltered: `?/F1 closes this box` over concepts, where `?` is text. Proposal:
   the closer applies the active stack's global-layer filter → `F1 closes this box`.
8. **Q8 — `VIEW_DEFAULTS` vs a user's shared line.** With `[confirm] no = ["x"]`, the compiled row
   `(Migration, ConfirmNo, ["n","N"])` still shadows it, so the prompt ignores the user's change (same for
   L-A/L-C's form `Down`/`Up`). Proposal: a user line for the shared act drops the view's compiled default
   row (the view inherits the user's list), unless the user also writes the view's own line. Decide once
   for all lanes.
9. **Q9 — headings.** Proposal: `Context::heading()` returns the overlay titles (`Search concepts`,
   `Workspaces`, `Schema`, `Waiting on you`) per D1, while the TOML tables stay `concepts`, `switcher`,
   `migration`, `waiting`.
10. **Q10 — T7 reachability.** `Stack::BASE` and `Stack::OVERLAY` stay in `DECLARED` but no `key_stack`
    returns them; the reachability test must exempt the shell fallbacks.
11. **Q11 — lane ownership.** Add `src/snapshots/htui__testkit__tests__shell_empty.snap` to L-E's owned
    snapshots (it shows the startup switcher; its status row changes when L-E sets the switcher's
    `key_stack`).
