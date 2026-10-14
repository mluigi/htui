# MOD-67 M6 blueprint — Lane L-B: the directory picker and Hierarchy's prose

Scope: `crates/htui/src/ui/path_picker.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs`
(production and in-file tests). Owned tests: `tests/hierarchy.rs`, the in-file tests of both
files. Snapshots: `hierarchy__picker`, `hierarchy__no_workspace`, `hierarchy__inferred` and every
other `hierarchy__*` — **none may move** (every sentence and hint renders byte-identical with the
defaults). Branch `mod-67-m6-l-b`, worktree `target/wt/l-b`.

Lane rules: never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`,
`settings/mod.rs` (`modal_rest` is used as is), any other section, `tests/settings.rs`,
`tests/secrets_settings.rs` (its `:2419` negative check of `reloaded; press p again` stays true:
the text is byte-identical). A missing act or stack goes back to the main thread. Format only own
paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit incrementally.

Complete: sections 0-8.

## 0. Facts (read at `59ff7d02`)

- `PathPicker::on_key(key) -> PickerOutcome` (`path_picker.rs:122-186`): the go-to field first
  (`:123-139`: `Submit` lists the typed path, `Cancel` closes the field, the rest `None`); then 13
  `KeyCode` arms on the code alone (`j`/`Down`, `k`/`Up`, `Enter`/`l` open, `h`/`Backspace` up,
  `s`, `S` (only with a listing), `/`, `.`, `Esc` → `Cancelled`), everything else `None`
  (swallowed, D15).
- Its only caller, `HierarchySection::on_picker_key` (`hierarchy.rs:1026-1061`), passes first every
  chord `views::CAPTURE` passes (`:1034-1037`: CONTROL, ALT, function keys), so `ctrl-j` does **not**
  move the picker today (plan claim 8 is true of the function only); a SUPER/META/HYPER chord would.
- `HierarchySection::stack` (`:512-523`) maps `Mode::Picking { .. }` to `views::CAPTURE`;
  `key_stack` returns `self.stack()`; the section hint is `HINT_PICKING` (`:84-86`, two `Text`s).
- The popup's hint is two literals, `HINT_MOVE`/`HINT_MORE` (`path_picker.rs:29-34`), one line when
  they fit, two otherwise (`lines`, `:289-317`); the empty listing says `no directories here · .
  shows hidden` (`body`, `:334-343`).
- `render(frame, area, theme)` (`:238-260`) is called once, `hierarchy.rs:1519`.
- Hierarchy's key-naming sentences (D4, PA-1): `NO_WORKSPACE` `:48` (`` `N` creates one``; said by
  `refuse` `:625` and drawn by `render` `:1506`), `RELOADED` `:97` (`press p again`; `on_stale`
  `:1275`), `open_path` `:769` (`` `b` wants the workspace or a repo row``), `move_primary` `:834`
  (`` `p` wants a repo row``), `inferred_notice` `:1686` (` — b on a repo sets it by hand`; called
  by `on_reply` `:1382`). `CHANGED_ELSEWHERE` `:89-91` ("Enter retries") names the editor's own
  `Enter` (a `TextField` submit, D13): stays literal. `HINT_TYPED` `:110-113` (`Enter confirm ·
  Esc stop`, the typed slug's field): stays.
- T1 provides `Context::Picker`, `Act::{PickerOpen, PickerParent, PickerChoose, PickerChooseDir,
  PickerGoTo, PickerHidden}`, `views::HIERARCHY_PICKER` = `view(Picker, [the six])`, `BACK`,
  `LIST`, `MODAL`, and the `views.rs` `PENDING`/`PENDING_ACTS` entries (T-close empties them).

## 1. The seam: `PathPicker` takes `&Keys` and the owner's stack (D3)

`path_picker.rs` stays free of `Ctx`: the owner passes its keys and its picker stack (the picker is
generic; Hierarchy is its one owner today and names `views::HIERARCHY_PICKER`).

```rust
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack};

/// The first half of the popup's hint (MOD-49 P8; MOD-67 M6 D3), through the owner's stack:
/// `j/k move · Enter open · h up · s choose · S this dir` under the defaults.
const HINT_MOVE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::PickerOpen, "open"),
    Hint::One(Act::PickerParent, "up"),
    Hint::One(Act::PickerChoose, "choose"),
    Hint::One(Act::PickerChooseDir, "this dir"),
];

/// The hint's second half: `/ go to · . hidden · Esc cancel` under the defaults.
const HINT_MORE: HintSpec = &[
    Hint::One(Act::PickerGoTo, "go to"),
    Hint::One(Act::PickerHidden, "hidden"),
    Hint::One(Act::Back, "cancel"),
];

impl PathPicker {
    /// Whether the go-to field is open: the owner puts the picker on a capture stack then.
    #[must_use]
    pub const fn goto_open(&self) -> bool { self.goto.is_some() }

    /// One key (MOD-49 P8; MOD-67 M6 D3). While go-to is open the field takes it (its keys are
    /// the widget's, ANA-26 §6.1) and the answer is always `Some`. Otherwise the chord resolves
    /// through `stack` (the owner's picker stack) and the first act the picker takes wins; a
    /// chord equal to no picker act is `None`, for the owner's modal rest (`q`, `?`, the digits
    /// are swallowed there, `F1` and `ctrl-c` pass).
    pub fn on_key(&mut self, key: KeyEvent, keys: &Keys, stack: Stack<'_>) -> Option<PickerOutcome> {
        if let Some(field) = &mut self.goto {
            return Some(/* today's :124-138, unchanged */);
        }
        let chord = KeyChord::from_event(key);
        keys.actions(stack, chord).into_iter().find_map(|act| self.on_act(act))
    }

    /// One resolved act; `None` for an act that is not the picker's (a global one).
    fn on_act(&mut self, act: Act) -> Option<PickerOutcome> {
        Some(match act {
            Act::ListDown => { /* :143-145 */ PickerOutcome::None }
            Act::ListUp => { /* :148 */ PickerOutcome::None }
            Act::PickerOpen => /* :151-154 */,
            Act::PickerParent => /* :155-167 */,
            Act::PickerChoose => /* :168-170 */,
            // A declined `S` (no listing) is still the picker's: consumed, as today.
            Act::PickerChooseDir => if self.listing.is_some() { PickerOutcome::Chosen(self.path.clone()) } else { PickerOutcome::None },
            Act::PickerGoTo => { self.goto = Some(TextField::with_text("/")); PickerOutcome::None }
            Act::PickerHidden => /* :178-181 */,
            Act::Back => PickerOutcome::Cancelled,
            _ => return None,
        })
    }

    /// The popup, centred over `area` (blueprint D14); its hint rendered through `stack` with
    /// `keys` (MOD-67 M6).
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme, keys: &Keys, stack: Stack<'_>) { … }
}
```
- `lines(&self, width, height, theme, hints: (&str, &str), hidden: Option<&str>)`: `hints` =
  `(keys.hint(stack, HINT_MOVE), keys.hint(stack, HINT_MORE))`, joined by `HINT_GAP` when both are
  non-empty and fit, one line each otherwise, an empty half dropped; `hidden` =
  `keys.label(stack, Act::PickerHidden)` for `body`.
- `body`'s empty-listing tail (`:335-339`): `format!(" \u{b7} {hidden} shows hidden")` with a
  label, nothing when unbound (`no directories here`).
- `PickerOutcome`'s doc of `None` ("Handled here (moved, typed, swallowed)") → "Handled here
  (moved, typed, or a declined key of the picker's)"; `Cancelled` "`Esc` outside go-to" →
  "`common.back` outside go-to".
- Module doc gains: "Its keys resolve through the owner's stack (`views::HIERARCHY_PICKER` for
  Settings › Hierarchy, MOD-67 M6); the go-to field's keys are the field's."
- `use crossterm::event::{KeyCode, KeyEvent}` → `KeyEvent` (production); tests keep `KeyCode`.

## 2. `hierarchy.rs`: stack, dispatch, hint

```rust
/// The picker's section hint (MOD-49 blueprint D17): the popup carries the full key list; the
/// way out through the mode's stack (MOD-67 M6), so it drops while the go-to field is open, where
/// `Esc` closes only the field.
const HINT_PICKING: HintSpec = &[Hint::Text("choosing a directory"), Hint::One(Act::Back, "cancel")];

fn stack(&self) -> Stack<'static> {
    match &self.mode {
        // …
        Mode::Picking { picker, .. } if picker.goto_open() => views::CAPTURE,
        Mode::Picking { .. } => views::HIERARCHY_PICKER,
        // …
    }
}

/// One key while the picker is open (MOD-49 P8, blueprint H-4, D15; MOD-67 M6 D3). Over the
/// go-to field a chord `views::CAPTURE` passes is the shell's (`ctrl-c`, `F1`); otherwise the
/// picker resolves the key through the mode's stack and what it does not take is `modal_rest`'s:
/// `q` can't quit mid-pick, `F1` and `ctrl-f` pass. A choice is a write: refused while another is
/// in flight, and the picker stays open until its reply (D8).
fn on_picker_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    let stack = self.stack();
    let chord = KeyChord::from_event(key);
    let Mode::Picking { picker, .. } = &mut self.mode else { return Handled::Pass; };
    if picker.goto_open() && stack.passes(chord) {
        return Handled::Pass;
    }
    let Some(outcome) = picker.on_key(key, ctx.keys(), stack) else {
        return modal_rest(stack, chord);
    };
    match outcome { /* today's :1041-1059, unchanged */ }
    Handled::Consumed
}
```
`render` (`:1519`): `picker.render(frame, area, ctx.theme, ctx.keys(), views::HIERARCHY_PICKER)`
— always the browse stack, so the popup's hint stays while go-to is open (today's frame). The
section's `stack` doc (`:508-511`) gains "the picker on `HIERARCHY_PICKER`, its go-to field on
`CAPTURE`". The module doc's (`:10-16`) "or the picker's own keys" → "or the picker's keys
(`[picker]`)".

Arm equivalence with today: `CAPTURE.passes` used to run first for every chord; now it runs first
only over the go-to field. In browse, a CONTROL/ALT/function chord resolves through
`HIERARCHY_PICKER` (no picker act binds one by default), the picker returns `None`, and
`modal_rest` passes it exactly as `CAPTURE` did (both end in `MODAL`). A plain unbound key: today
`PickerOutcome::None` → `Consumed`; now `None` → `modal_rest` → `Consumed`. Same.

## 3. Hierarchy's key prose (D4; stack `views::HIERARCHY_BROWSE`, where each key acts)

| Site | Today | Becomes (defaults: byte-identical) | Unbound | Signature change |
|---|---|---|---|---|
| `:48` `NO_WORKSPACE`, said by `refuse` `:625`, drawn by `render` `:1506` | `` no workspace — `N` creates one`` | `fn no_workspace(new: Option<&str>) -> String`: `` format!("no workspace \u{2014} `{new}` creates one") `` | `no workspace` | `refuse(&mut self, act: Act, keys: &Keys)`; its seven callers (`:535-588`) pass `ctx.keys()`; `render` passes `ctx.keys().label(views::HIERARCHY_BROWSE, Act::HierarchyNewWorkspace)` |
| `:97` `RELOADED`, `on_stale` `:1275` | `reloaded; press p again` | `fn reloaded(primary: Option<&str>) -> String`: `format!("reloaded; press {primary} again")` | `reloaded` | `on_stale(&mut self, snapshot, keys: &Keys)`; `on_reply` `:1375` passes `ctx.keys()`; label `Act::HierarchyPrimary` |
| `:769` `open_path` | `` `b` wants the workspace or a repo row`` | `` format!("`{b}` wants the workspace or a repo row") `` with `ctx.keys().label(views::HIERARCHY_BROWSE, Act::HierarchyChoosePath)` (`open_path` has `ctx`) | `pick a path on the workspace or a repo row` | none |
| `:834` `move_primary` | `` `p` wants a repo row`` | `` format!("`{p}` wants a repo row") `` (`Act::HierarchyPrimary`) | `only a repo row can be made primary` | none |
| `:1686` `inferred_notice` | ` — b on a repo sets it by hand` | `pub(crate) fn inferred_notice(report: &InferReport, choose: Option<&str>) -> String`: `format!(" \u{2014} {choose} on a repo sets it by hand")` | ` — set it by hand on a repo row` | `on_reply` `:1382` passes `ctx.keys().label(views::HIERARCHY_BROWSE, Act::HierarchyChoosePath).as_deref()`; its in-file tests pass `Some("b")` |

The unbound branches are only reachable through a file that unbinds the act (the refusal still
fires on another trigger: `refuse` on `N`'s siblings, `on_stale` from a reply); they name no key.

## 4. User-visible

- Byte-identical with the defaults: the popup, the section hint, every sentence above.
- With a rebound picker key, the popup names it; `[picker] hidden = "H"` → `no directories here ·
  H shows hidden`.
- While go-to is open the section hint reads `choosing a directory` (the `Esc cancel` it showed
  closed only the field).
- `?` over the picker: `Picker: Enter/l open directory · h/Backspace parent directory · s choose ·
  S choose this directory · / go to path · . show hidden`, `Common: Esc back`, `List: j/Down down ·
  k/Up up`, `Global: Ctrl+c quit · F1 help · …` (modal), closer (exact text from the run). Over
  the go-to field: the `CAPTURE` box, as today.

## 5. Tests (write red first)

In-file, `path_picker.rs` (the 31 existing tests switch to a helper; no assertion changes):
```rust
/// `key` through the picker as Hierarchy feeds it: the compiled keys and the picker stack, a key
/// no picker act takes read as swallowed (`None`), as Hierarchy's modal rest does.
fn press(picker: &mut PathPicker, key: KeyEvent) -> PickerOutcome {
    picker.on_key(key, Keys::compiled(), views::HIERARCHY_PICKER).unwrap_or(PickerOutcome::None)
}
```
`render`/`render_at` pass `Keys::compiled()` and `views::HIERARCHY_PICKER`.

| Test | Asserts |
|---|---|
| `a_chord_is_no_picker_key` (new) | `on_key(ctrl-j / alt-l / ctrl-s / SUPER-j, …)` is `None` and the cursor and path are unchanged (fails today for SUPER-j through the function: it moves) |
| `a_rebound_open_key_opens_and_enter_is_inert` (new) | keys `load_str("[picker]\nopen = \"o\"\n")`: `o` asks for the highlighted child; `Enter` is `None`; the popup shows `o open` |
| `unbound_keys_are_swallowed` (existing, reworded) | `on_key(q / ? / 1)` is `None` (the owner swallows it); `s` still chooses |
| `the_go_to_field_takes_every_key` (new) | after `/`: `q`, `j`, `ctrl-a` are `Some(PickerOutcome::None)` and land in (or edit) the field; `Esc` closes only the field |
| `the_hint_is_the_rendered_spec` (new) | at 100 columns the popup's one-line hint equals the old `HINT_MOVE · HINT_MORE` text byte for byte; at 80, the two halves on two lines, as `a_narrow_popup_wraps_the_refusal_and_splits_the_hint` |
| `the_empty_listing_names_the_hidden_key` (new) | `[picker] hidden = "H"` → `no directories here · H shows hidden`; unbound → `no directories here` |

In-file, `hierarchy.rs`: `the_picker_mode_reports_the_picker_stack` (new: `Picking` with go-to
closed → `key_stack() == Some(views::HIERARCHY_PICKER)`, open → `Some(views::CAPTURE)`);
`the_prose_names_the_keys_in_force` (new: `no_workspace(Some("W"))`, `reloaded(Some("P"))`,
`inferred_notice(.., Some("B"))` and the `None` forms).

Integration, `tests/hierarchy.rs` (`#![cfg(feature = "testkit")]`):

| Test | Asserts |
|---|---|
| `a_rebound_picker_key_opens_a_directory` (new) | `with_keys(load_str("[picker]\nopen = \"o\"\nparent = \"u\"\n"))`; open the picker on a repo row (`b`), feed a listing: `o` lists the highlighted child, `u` goes back up, `Enter` lists nothing new; the popup shows `o open · u up` |
| `a_rebound_new_workspace_key_is_the_one_the_empty_tree_names` (new) | `[settings.hierarchy] new_workspace = "W"`, no workspace: the rows pane and the refusal say `` no workspace — `W` creates one``; `N` is inert |
| `a_rebound_primary_key_is_named_after_a_stale_write` (new) | `[settings.hierarchy] primary = "P"`; a `HierarchyStale` after `P`: the notice is `reloaded; press P again` |
| `f1_over_the_picker_opens_help_and_q_does_not_quit` (new; existing behaviour, now through the stack) | `F1` toggles the `?` box with a `Picker:` line; `q` keeps the picker open and the app running |
| existing (`:1617`, `:1767-1784` `` `N` creates one``; `:2363`, `:2499`, `:2516` `b on a repo`; `:2725` `` `b` wants``; the picker tests `:2553-2673`) | unchanged, green (byte-identical) |

## 6. Snapshots

None move (`hierarchy__picker`, `hierarchy__no_workspace`, `hierarchy__inferred` hold the three
sentences and both hints byte-identical). Zero pending at the lane gate.

## 7. Commits

1. `test(mod-67): M6 L-B picker and hierarchy prose pins` — §5, red.
2. `feat(mod-67): M6 the directory picker dispatches through its stack` — §1, §2.
3. `feat(mod-67): M6 hierarchy names the keys in force` — §3.

## 8. Gate

```bash
rustfmt --edition 2024 --check crates/htui/src/ui/path_picker.rs crates/htui/src/ui/tabs/settings/hierarchy.rs crates/htui/tests/hierarchy.rs
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
cargo test -p htui --lib ui::path_picker ui::tabs::settings::hierarchy
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test hierarchy --test settings --test secrets_settings -- --test-threads=1
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features --check
```
