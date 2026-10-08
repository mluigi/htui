# Blueprint: MOD-57 milestone 2, "a live editor across htui, and close-out"

**Status**: proposed (2026-10-08). The findings in §0 (F-M2-1 to F-M2-12), the plan deviations
in §10 (PD-M2-1 to PD-M2-3) and the blueprint decisions in §11 (B-M2-1 to B-M2-9) are proposed
here. A finding marked **Blocker** means the plan, read literally, leaves a red tree or cannot be
built as written. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-57-m2-live-editor-across-htui.plan.md` (CONFIRMED 2026-10-08). Its
E1-E10, T0-T4 and Verified-claims table are authoritative except where §0 and §10 amend them.
**PRD**: `.claude/prds/mod-57-embedded-editor-pane.prd.md` (D3-D5 govern M2). **Predecessor**:
`.claude/plans/mod-57-m1-editor-in-pane.blueprint.md` §6.4 (the loader rules owed) and §7.4 (the
M1 lock this replaces), with the M1 plan's T5 notes (R0: `editor_has_keys`, the swallow rule,
`PROMPT_FIRST`).

**Verified at**: HEAD `0aea5312`, branch `hr/MOD-57`. Every `file:line` below was read at that
HEAD (Gortex `explore` for localisation; source read by `sed -n` where the hook blocked `Read`).
**Line numbers are pre-edit.**

**Scope**:
- **Order**: T0 (main thread) → T1 ∥ T2 (disjoint file sets, one shared tree) → T3 → T4. After
  T1 and T2 both land, re-run both gates on the real tree with `--test-threads=1`.
- **New public surface**: `htui::keys::Stack::EDITOR_OPEN` (renames `EDITOR_UNFOCUSED`),
  `htui::keys::views::QUIT_CONFIRM`, `htui::ui::overlay::QuitConfirm` (+ `QuitConfirm::ID`),
  `App::quit_overlay: Option<OverlayId>`. Crate-private: `App::request_quit`,
  `App::editor_notice`, `App::editor_steer_refusal`, `App::editor_tab`, `stack::WHILE_EDITING`.
- **No new `Action` variant, no new `Context`, no new `Act`**: `Act` stays 43, `Context` 8,
  `CATALOGUE` 43 rows. `DECLARED` 36 → **38** (§9).
- **No existing snapshot changes.** New snapshots: 1 (T2) + 2 (T3). With `HTUI_EDITOR_PANE`
  unset nothing on screen moves (the plan's acceptance line holds: every M2 branch needs a live
  editor).

**House style (unchanged from M1's blueprint)**: lints (`unsafe_code = "forbid"`,
`missing_docs`, `missing_debug_implementations`, `unused_qualifications`, `clippy::all`); every
`pub` item documented; inline format args; `max_width = 100`; never `Debug`/log edited text.
Implementers commit incrementally, stage their own paths only (never `-A`, `stash`, `--amend`);
every commit compiles and passes its task gate. The Gortex hook may block `Read`/`Edit` on
indexed source: read with Gortex or `sed -n`, edit with an anchored scripted replace (each anchor
asserted to match once), then `cargo fmt`. No new text may contain the literal that
`htui-agent/tests/extensibility.rs` forbids workspace-wide (see that test).

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `0aea5312` | Fix |
|---|---|---|---|---|
| **F-M2-1** | **Blocker** (T0 and T1 leave `keys::` red) | T0: add `views::QUIT_CONFIRM` and its `DECLARED` line; T1: `EDITOR_OPEN` joins `DECLARED`. | Three test pins assume every `DECLARED` entry past index 3 opens with a view layer and comes from `views.rs`: `views.rs:393-394` (`let views = &DECLARED[4..]; assert_eq!(views.len(), 32)`, then `first.inherits()` and `first.context().is_view()` for each), `views.rs:521-524` (`views.rs` consts `== DECLARED.len() - 2`) and `stack.rs:337` (`DECLARED.len() == 36`). `views.rs:478-493`'s `VIEWS` list must name a file whose production code says `views::QUIT_CONFIRM`. `[CONFIRM, OVERLAY, HELP]` and `[editor, global]` have no view layer. | T0 rewrites the two `views.rs` tests to name the non-view stacks explicitly (§2.3) and adds `quit_confirm.rs` to `VIEWS`; T0's stub names `views::QUIT_CONFIRM` in `key_stack`. Count pin 36 → 37 (T0) → 38 (T1). Both new entries are **appended** (B-M2-1). |
| **F-M2-2** | **Blocker** (T1 leaves the integration suite red) | T1 moves `print.rs:153`'s `quit = "ctrl-x"` only. | `tests/fixtures/keys/valid.toml` binds `[global] quit = ["ctrl-x"]`; `tests/keys_file.rs` loads it and asserts on it at `:85` (`--print-keys` line), `:413` (`Ctrl+x quit · Tab next tab`), `:420` (`Ctrl+x/Ctrl+c quit`) and `:445-446` (`harness.key("ctrl-x")` quits). E9 (c) refuses that file. | **PD-M2-3**: T1's file set gains both. Every fixture moves to `ctrl-g` (same width as `ctrl-x`, so `print.rs:161`'s and `keys_file.rs:85`'s aligned lines keep their padding; no catalogue default or view binds a control-`g`). |
| **F-M2-3** | Major (user doc shows a file htui refuses) | README: "UPDATE only if its key/editor lines name the M1 lock (fact-check: none)". | `README.md:214`, the "Changing keys" example, is `quit = ["ctrl-x"]`. After E9 (c) that file does not load. | **PD-M2-3**: T4 changes the example to `ctrl-g` and adds one sentence on the `[editor]` rules. |
| **F-M2-4** | Major (T2 test cannot be written) | T2: "the question names the editor's value when given one, never blank". | Overlays are built by `OverlayRegistry::create` from a `Fn() -> Box<dyn Overlay>` factory (`ui/overlay/registry.rs:68`, `:95`); `migration_prompt.rs:91-93` says that signature "must stay that way". `Ctx` carries no editor state. A shell that pushed `QuitConfirm::new(value)` itself would name a concrete view, which the shell never does (MOD-1 blueprint D; `startup_overlay`/`migration_overlay` are ids for that reason). | **PD-M2-1**: a fixed question (§4.1). The confirm is registered by id (`App::quit_overlay`, B-M2-4). |
| **F-M2-5** | Major (E5's reach) | E5: refuse `Action::SetScope` ("the workspace switcher's pick"). | `SetScope` has four emitters: the switcher (`workspace_switcher.rs:86`), Settings › Hierarchy's D11 scope follow after a project is created or deleted (`hierarchy.rs:1214`), its re-entry after the current workspace is deleted (`hierarchy.rs:1434`), and the shell's startup pick (`update.rs:497`). All reach `update`'s one arm (`update.rs:30`). | E5 holds as written: the arm refuses every `SetScope` while an editor is alive (B-M2-6). Consequences go to the docs (T4): a project created or deleted in Settings during an edit reaches the other tabs when Settings next re-reads its tree; deleting the workspace htui is in leaves htui in it until the edit ends and a workspace is picked. The startup pick cannot meet a live editor (the first `Workspaces` reply lands before any key). **Maintainer may want to see this before T3** (§12). |
| F-M2-6 | Minor (shape of E4's `request_quit`) | "`action_for(Act::Quit)` → one `App::request_quit` that opens the confirm when an editor is alive". | `action_for(&self, Act) -> Option<Action>` (`state.rs:377`) is `&self` and also feeds the status line's and `?` box's `offered` predicate (`state.rs:948`, `:984`). A `&mut self` `request_quit` would need a new `Action` variant to be reached from it. | `request_quit(&self) -> Action` returns the action (open the confirm, or quit); `action_for`'s arm is `Act::Quit => Some(self.request_quit())`. No new variant (B-M2-3). |
| F-M2-7 | Minor (E7 vs M1's R0 rule) | E7: "the `?` box lists the `editor` context while an editor is alive in every view". | M1's unconverted branch (`state.rs:996-998`) lists `Editor: Ctrl+4 … · Ctrl+x …` even under a modal prompt, whose keys are the prompt's (M1 R0 F3: "offer no editor key" there), and over the asking tab it lists the tab's legacy rows, which the lock refuses. After E2 `Ctrl+x` is inert on every other tab. | **PD-M2-2**: the box lists the editor keys that act where the user is (§5.6). |
| F-M2-8 | Minor (sentences E2/E6 make false) | — | `lock_refusal` (`pane.rs:273-282`), used by the swallow rule, offers `Ctrl+x abort` even when the editor is off screen; `editor_status` (`pane.rs:424-436`) returns M1's unfocused line on every tab. After E2 and E6 both are wrong off screen. | Off screen: the swallow says `the editor is open in <Tab>: Ctrl+4 back`; `editor_status` is `None` and the E6 notice leads the tab's own line (§5.2, §5.5). |
| F-M2-9 | Note (answers (d)) | — | Every overlay that emits `Reveal` emits `Overlay(Close)` first (`queue.rs:215`, `waiting_list.rs:125`, `concepts_search.rs:223`); the switcher emits `SetScope` then `Close` (`workspace_switcher.rs:86-89`). | A refused steer closes the overlay that asked and leaves the refusal on the status line. No overlay changes. |
| F-M2-10 | Note (comments) | — | `catalogue.rs:659` names `Stack::EDITOR_UNFOCUSED`; `:654-656` says "MOD-67 M2's loader must refuse a file that unbinds it"; `SHADOWING`'s comment (`:954-955`) cites only the migration prompt, though `(ConfirmNo, OverlayClose)` also allows `QUIT_CONFIRM`'s `esc` pair (confirm layer narrower than overlay). | T0 fixes the rename and the `SHADOWING` comment; T1 the loader comment. No new allow-list entry: `the_compiled_defaults_validate` and `every_allow_list_entry_is_demanded` stay green. |
| F-M2-11 | Note (plan tables disagree) | Files table: `stack.rs` "T0 (const), T1"; Independence table: `stack.rs` is T0's only. | `DECLARED` and its count pin live in `stack.rs`. | T1 owns the `EDITOR_OPEN` `DECLARED` line, the 37 → 38 pin and the `WHILE_EDITING` const; still disjoint from T2 (which never touches `keys/`). Part of PD-M2-3. |
| F-M2-12 | Note (plan silent) | — | An editor can exit by itself while the quit confirm is up (`Exited` arrives through the pane arm). | The confirm stays up; `y` still quits, `n` closes (B-M2-5). |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| (a) `QUIT_CONFIRM` with no view layer | **Yes.** `Stack::new` is a `const fn` over any layers (`stack.rs:132`). `help_lines` heads each layer with `Context::heading()` (`Confirm`, `Overlay`, `Global`) and `help_closer` reads the global layer (`HELP` admits help, so `?/F1 closes this box`). `--print-keys` prints contexts, not stacks (`print.rs:27`). The `DECLARED` collision pass is layer-generic (`validate.rs:91-128`); its `esc` pair is already on `SHADOWING`. `overridable` and `VIEW_DEFAULTS` only read view layers (`load.rs:257-265`), so the confirm has no table of its own: `[confirm]`, `[overlay]` and `[global] help` rebind it. **Cost:** the three test pins of F-M2-1 and the `VIEWS` list, rewritten in T0. No `Context::QuitConfirm`. | §2, F-M2-1 |
| (b) `on_key`'s editor step | §5.2's resolution order. In one line: a focused editor without the keys swallows the first key (as M1); with the keys, M1 unchanged; then `ctrl-c` and a modal overlay go to `on_key`'s own path; on the asking tab the key resolves through `EDITOR_OPEN` (editor, then every global the shell maps) or is refused; on any other tab only `editor.focus` is checked (through `EDITOR_FOCUSED`) and everything else runs as if no editor were alive. `editor_lost_keys` keeps its job (the first input typed for a focused editor that a reply moved off screen or covered is swallowed and unfocuses it); only its off-screen sentence changes. | §5.2 |
| (c) Quit routes | `request_quit` lives in `app/pane.rs`. Routes to `should_quit`: (1) `action_for(Act::Quit)` → `request_quit()`, reached by `apply_keys` (the tab's stack, `Stack::BASE` or a Settings stack; no overlay stack admits `Quit`) and by the editor step's `EDITOR_OPEN` resolution, which replaces the lock's own `Quit` arm (`pane.rs:357`); (2) `ctrl-c`, at `on_key`'s single C3 site (`state.rs:790-793`; the editor step's duplicate at `pane.rs:336-339` becomes `return false`); (3) the confirm's `y`, which emits `Action::Quit`. No production keymap row binds `Action::Quit` (`keymap.rs` hits are tests). `q` under the open confirm is swallowed by the modal path (the confirm's stack admits no `global.quit`), so no second box opens. An editor that exits under the confirm leaves it up (B-M2-5). | §5.3, B-M2-3, B-M2-5 |
| (d) E5 | Sentence: `the editor is open in <Tab>: finish or abort it first` (`<Tab>` = the asking tab's `Tab::title()`, e.g. `Skills`). `update`'s `SetScope` arm refuses whenever an editor is alive; `reveal` refuses after resolving its target tab when that tab is the asking one (before `update_tab`). The switcher and the reveal overlays need no change: each closes itself (F-M2-9) and the refusal stays on the status line. | §5.4 |
| (e) E6 prefix | `editing in <Tab> · <back> · ` + the tab's own status line, `<back>` = `keys.hint(Stack::EDITOR_FOCUSED, &[Hint::One(Act::EditorFocus, "back")])` (`Ctrl+4 back`; `F12 back` rebound; dropped when unbound or under a modal overlay, where the chord does not act). A `self.status` error replaces the whole line, prefix included (as today). Width: one `Paragraph` line, clipped at the right edge as every status line is; the prefix comes first so the way back is the last thing clipped. | §5.5 |
| (f) E7 | Asking tab on screen, htui holding the keys, no modal overlay: the box is `Stack::EDITOR_OPEN`'s (`Editor: Ctrl+4 editor focus · Ctrl+x abort edit`, then the `Global` line; no legacy tab rows, which the lock refuses). Another tab, no modal overlay: that view's own box (converted or not) with `Editor: Ctrl+4 editor focus` first. Under a modal overlay: the overlay's box, no editor line (PD-M2-2). | §5.6 |
| (g) E8 | `wants_mouse() = !self.editor_visible() && overlays.is_empty() && !help_visible && active tab wants`. The MOD-74 edge needs no change: leaving a capturing tab for the asking tab (by key or by `editor.focus`) is an on-to-off edge, so `mouse_capture` calls `lose_mouse` and every tab's `on_mouse_lost` as on any switch; `on_mouse` reads the same predicate. | §5.6 |
| (h) E9 | Messages, tests and fixtures in §3. (a) `load.rs:418` gains `editor.focus` with `[editor] focus: must keep at least one chord: the editor takes every other key`; (b) `give_way` never takes a chord from `editor.focus`; (c) `("while an edit is open", Stack::EDITOR_OPEN)` appended to `DECLARED`; (d) a pass after the stack pass reports every other row sharing a chord with `editor.focus`, through `Check::report` with the same phrase; (e) tests only. No `SHADOWING`/`STATE_GUARDED` entry. Fixtures: `print.rs:153,161`, `keys/mod.rs:474` (widened), `tests/fixtures/keys/valid.toml`, `tests/keys_file.rs:85,413,420,445`. | §3 |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits | Gate (real tree, `--test-threads=1`) |
|---|---|---|---|
| T0 names | `keys/stack.rs`, `keys/views.rs`, `keys/catalogue.rs` (comments), `ui/overlay/mod.rs`, `ui/overlay/quit_confirm.rs` (stub), `app/pane.rs` (rename only) | 1 | `cargo test -p htui --all-features --lib -- keys:: app::pane`; clippy both ways |
| T1 loader | `keys/load.rs`, `keys/validate.rs`, `keys/stack.rs`, `keys/mod.rs`, `keys/print.rs`, `keys/catalogue.rs` (comment), `tests/keys_file.rs`, `tests/fixtures/keys/valid.toml` | 2 (§3.6) | `cargo test -p htui --all-features --lib -- keys::`; `cargo test -p htui --all-features --test keys_file -- --test-threads=1`; clippy both ways |
| T2 confirm | `ui/overlay/quit_confirm.rs`, `ui/overlay/snapshots/htui__ui__overlay__quit_confirm__tests__the_box.snap` | 1 | `cargo insta test -p htui --all-features -- quit_confirm` (review, accept, commit); clippy both ways |
| merge T1+T2 | - | - | `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features --check` |
| T3 shell | `app/pane.rs`, `app/state.rs`, `app/update.rs`, `app/mod.rs`, `app/snapshots/` (2 new), `tests/editor_pane.rs` | 4 (§5.9) | `cargo test -p htui --all-features --lib -- app::`; `cargo test -p htui --features testkit --test editor_pane -- --test-threads=1`; insta `--check`; clippy both ways; Windows cross-check |
| T4 docs, close-out | `docs/htui-editor.md`, `README.md`, `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-57.md` (new), the PRD, the plan | 2+ (§6) | the plan's full Validation block |

"Clippy both ways" = `cargo clippy -p htui --all-targets --all-features -- -D warnings` **and**
`cargo clippy -p htui -- -D warnings` (featureless). "Windows cross-check" =
`CC_x86_64_pc_windows_gnu=/tmp/mingw/mgcc AR_x86_64_pc_windows_gnu=/tmp/mingw/mar cargo check -p
htui --target x86_64-pc-windows-gnu` (re-extract `/tmp/mingw` per the memory recipe after a
sandbox restart).

---

## 2. T0: names before the fan-out (main thread)

### 2.1 `keys/stack.rs`

Rename and recompose (`:198-204`):
```rust
/// `[editor, global]`: MOD-57 M2 (E3). With the editor alive, unfocused and on screen, a key
/// resolves through the editor's two actions, then every global action; `App` refuses what
/// nothing resolves, so the asking view never sees it. On any other tab only `editor.focus`
/// (`EDITOR_FOCUSED`) is checked ahead of the view (E1).
pub const EDITOR_OPEN: Self = Self(&[Layer::all(Context::Editor), Layer::all(Context::Global)]);
```
Append to `DECLARED` (after `("in the waiting list", views::WAITING_LIST)`):
`("in the quit question", views::QUIT_CONFIRM),`. Module doc: "MOD-57 adds the editor's two:
`EDITOR_FOCUSED` and `EDITOR_OPEN`."

T0 behaviour check: the M1 lock still holds with the wider stack, because `editor_key`'s
`match` (`pane.rs:341-359`) acts only on `EditorFocus`/`EditorAbort`/`Quit`/`Help` and refuses
every other candidate (`tab` now resolves to `NextTab` and lands in `_ =>`). M1's labels
(`UNFOCUSED_HINT`, `LOCK_HINT`) are unchanged through the wider global layer.

Tests (rewritten, not deleted):
1. `the_unfocused_editor_stack_is_the_m1_lock` → `the_open_editor_stack_is_the_editor_then_every_global`:
   `ctrl-4` → `[EditorFocus]`, `ctrl-x` → `[EditorAbort]`, `q` → `[Quit]`, `?`/`f1` → `[Help]`,
   `tab` → `[NextTab]`, `shift-tab` → `[PrevTab]`, `1` → `[SelectTab1]`, `w` → `[Workspaces]`,
   `ctrl-f` → `[Find]`, `ctrl-w` → `[Waiting]`, `ctrl-q` → `[Queue]`; `esc`, `e`, `j`, `ctrl-s`
   → `[]`.
2. `ctrl_c_is_no_action_in_either_stack`, `the_editor_labels_come_through_the_stacks`: the
   rename only.
3. `declared_starts_with_base_and_overlay_and_holds_every_view_stack`: `36` → `37`.

### 2.2 `keys/views.rs`

```rust
/// The quit question (MOD-57 M2 E4): `confirm.yes` quits, `confirm.no` and `overlay.close` keep
/// the edit, `global.help` toggles the box. No view layer: the question has no keys of its own,
/// so `[confirm]`, `[overlay]` and `[global] help` rebind it, and no `[quit]` table exists.
pub const QUIT_CONFIRM: Stack<'static> = Stack::new(&[CONFIRM, OVERLAY, HELP]);
```
Placed after `MIGRATION`. Module doc: "Each stack opens with the view's own layer" gains "except
`SETTINGS_TAB`, `CAPTURE` and `QUIT_CONFIRM`, which have none."

### 2.3 `keys/views.rs` tests (F-M2-1)

In `mod tests`:
```rust
/// `DECLARED` entries this module does not declare: the shell's own stacks.
const SHELL: &[Stack<'static>] = &[Stack::BASE, Stack::OVERLAY, Stack::EDITOR_OPEN];
/// `DECLARED` entries that do not open with a view layer.
const NOT_VIEW_LED: &[Stack<'static>] = &[
    Stack::BASE, Stack::OVERLAY, Stack::EDITOR_OPEN, SETTINGS_TAB, CAPTURE, QUIT_CONFIRM,
];
```
- `every_view_layer_comes_first_and_every_modal_stack_ends_modal`: `let views: Vec<_> =
  DECLARED.iter().filter(|(_, stack)| !NOT_VIEW_LED.contains(stack)).collect();
  assert_eq!(views.len(), 32);` and the loop over `views`. (`Stack` derives `PartialEq`.)
  `EDITOR_OPEN` is listed before it joins `DECLARED` (T1), so T1 does not touch this file.
- `every_view_stack_is_named_by_a_view`: `VIEWS` gains
  `include_str!("../ui/overlay/quit_confirm.rs")`; the count becomes `DECLARED.iter().filter(|(_,
  stack)| !SHELL.contains(stack)).count()`.
- `the_view_stacks_resolve_as_designed` gains: `QUIT_CONFIRM`: `y` → `[ConfirmYes]`, `n` →
  `[ConfirmNo]`, `esc` → `[ConfirmNo, OverlayClose]`, `?`/`f1` → `[Help]`, `q`, `Y`, `enter` →
  `[]`.

### 2.4 `ui/overlay/quit_confirm.rs` (stub) and `ui/overlay/mod.rs`

`mod.rs`: `pub mod quit_confirm;` and `pub use quit_confirm::QuitConfirm;` in order. The stub
fixes T3's contract and satisfies `every_view_stack_is_named_by_a_view`:
```rust
//! MOD-57 M2 E4 (PRD D5): the question `global.quit` asks while an in-pane editor is alive.
//! (T2 fills in the question, the keys and the box.)
#[derive(Debug, Default)]
pub struct QuitConfirm;
impl QuitConfirm {
    /// Identity of the confirm; `register_all` registers the factory under it and names it as
    /// `App::quit_overlay`.
    pub const ID: OverlayId = OverlayId("quit_confirm");
    /// The confirm.
    #[must_use] pub fn new() -> Self { Self }
}
impl Overlay for QuitConfirm {
    // id → Self::ID; title → "Quit"; is_modal → true; key_stack → Some(views::QUIT_CONFIRM);
    // wants_requests → Vec::new(); on_key → Handled::Pass (the modal swallow); on_reply and
    // render → nothing.
}
```

### 2.5 `keys/catalogue.rs` and `app/pane.rs` (rename only)

`catalogue.rs:659` comment: `Stack::EDITOR_OPEN`, "offered only on the asking tab while the
editor is unfocused". `SHADOWING`'s comment (`:954`): add "and the quit question
(`views::QUIT_CONFIRM`): `Esc` is `confirm.no` there too". `app/pane.rs`: the three uses
(`:279`, `:344`, `:434`) and the module doc's `[`Stack::EDITOR_UNFOCUSED`]` link.

### 2.6 T0 commit and gate

`feat(mod-57): M2 T0 - the open-edit stack, the quit question's stack and its overlay stub`.
Gate: `cargo test -p htui --all-features --lib -- keys:: app::pane` (every M1 `app::pane` test
green with the wider stack); `cargo build -p htui`; clippy both ways. `the_compiled_defaults_validate`
stays green: `QUIT_CONFIRM`'s only default share (`esc`) is on `SHADOWING`.

---

## 3. T1: loader and validator (E9; parallel with T2)

**First failing test**: `keys::validate::tests::the_abort_cannot_take_the_focus_chord`.

### 3.1 E9 (a): `editor.focus` keeps a chord (`load.rs:418-425`)

Replace the `OverlayClose`-only check with a lookup, keeping the overlay's text byte for byte:
```rust
/// Why `act` must keep at least one chord (ANA-26 §7.4 step 6), or `None`.
const fn must_keep(act: Act) -> Option<&'static str> {
    match act {
        Act::OverlayClose => Some("overlays swallow every other key"),
        // MOD-57 M2 E9 (a): the one way back from a focused in-pane editor.
        Act::EditorFocus => Some("the editor takes every other key"),
        _ => None,
    }
}
...
if items.is_empty() && let Some(reason) = must_keep(spec.act) {
    self.push(key_line, format!("[{table}] {name}: must keep at least one chord: {reason}"));
}
```
Message: `[editor] focus: must keep at least one chord: the editor takes every other key`.

### 3.2 E9 (b): `editor.focus` never gives way (`validate.rs:27-57`)

`give_way`'s skip becomes `|| matches!(default.act, Act::OverlayClose | Act::EditorFocus)`; its
doc: "`overlay.close` and `editor.focus` never give way (each must keep a chord)". The entry then
collides in `check`'s per-context pass (`[editor]` is neither view nor skipped):
`[editor] abort = "ctrl-4": "ctrl-4" is already editor.focus (default) in [editor]` (line 2),
and `focus` keeps `ctrl-4`. The other direction is allowed: `[editor] focus = ["ctrl-x"]` takes
`ctrl-x` from `abort`, which is unbound with the usual notice (abort need not stay bound).

### 3.3 E9 (c), (d): the open-edit stack and the focus pass

`stack.rs`:
```rust
/// The phrase of every collision with the editor's keys (MOD-57 M2 E9): `DECLARED`'s entry for
/// `Stack::EDITOR_OPEN` and `validate`'s focus pass.
pub(super) const WHILE_EDITING: &str = "while an edit is open";
```
`DECLARED` appends `(WHILE_EDITING, Stack::EDITOR_OPEN)` **after** the quit question (B-M2-1);
count pin 37 → 38. Because the editor layer is `Layer::all`, the stack pass compares
`[editor]` with every `[global]` chord: `[global] quit = ["ctrl-x"]` →
`[global] quit = "ctrl-x": "ctrl-x" is already editor.abort (default) while an edit is open`.

`validate.rs` `check`, a new pass **between** the `DECLARED` pass and the capture pass:
```rust
// MOD-57 M2 E9 (d): `editor.focus` acts on every screen while an edit is open, ahead of the
// view (E1), so no other action may share its chord in any context. `report` dedups a pair the
// passes above found (the per-context pass for `[editor]`, the stack pass for `[global]`).
if let Some(focus) = keys.row(Context::Editor, Act::EditorFocus) {
    for row in keys.rows.iter().filter(|row| row.act != Act::EditorFocus) {
        for &chord in row.chords.iter().filter(|chord| focus.chords.contains(chord)) {
            check.report(row, focus, chord, WHILE_EDITING);
        }
    }
}
```
`[list] down = ["ctrl-4"]` → `[list] down = "ctrl-4": "ctrl-4" is already editor.focus (default)
while an edit is open`. A derived `VIEW_DEFAULTS` row is named by its source (as everywhere).
`validate`'s doc: "Four passes … a chord shared with `editor.focus` in any context …".
No `SHADOWING`/`STATE_GUARDED` entry: no default shares `ctrl-4` or `ctrl-x` (fact-check probe).

Interactions: `give_way` is per-context and runs before `check`; with (b) it can no longer empty
`editor.focus`, so (d) always has the focus row's real chords. The existing error vectors keep
their order and length: both new stacks are appended, and every pair they could see in the
existing tests (`[global] help = "esc"`, `[overlay] close = ["esc","f1"]`) is reported earlier
and deduped (`a_collision_over_an_overlay_is_found_in_the_overlay_stack` keeps its three rows).

### 3.4 T1 tests, TDD order

`validate::tests` (`errors`/`one` helpers):
1. `the_abort_cannot_take_the_focus_chord`: `"[editor]\nabort = \"ctrl-4\"\n"` →
   `one(2, r#"[editor] abort = "ctrl-4": "ctrl-4" is already editor.focus (default) in [editor]"#)`
   (today it loads and unbinds focus: the fact-check probe).
2. `the_focus_may_take_ctrl_x_from_abort`: `"[editor]\nfocus = \"ctrl-x\"\n"` loads;
   `chords(Editor, EditorAbort)` empty; `notices()` names it.
3. `a_global_chord_of_the_editors_is_refused_while_an_edit_is_open`: `[global] quit = "ctrl-x"`
   → `one(2, …editor.abort (default) while an edit is open)`; `[global] help = ["?", "ctrl-4"]`
   → `…editor.focus (default) while an edit is open`.
4. `the_focus_chord_is_refused_in_every_other_context`: `[list] down = "ctrl-4"`,
   `[confirm] yes = "ctrl-4"`, `[settings.agents] probe = "ctrl-4"` (a view table) → one error
   each, on line 2, ending `while an edit is open`.
5. `ctrl_c_is_refused_in_the_editor_table`: `[editor] focus = "ctrl-c"` →
   `one(2, "[editor] focus = \"ctrl-c\": ctrl-c always quits and cannot be bound")`.
6. `the_compiled_defaults_validate`, `every_allow_list_entry_is_demanded`: unchanged, green.

`load::tests`:
7. `the_editor_focus_must_keep_a_chord`: `"[editor]\nfocus = []\n"` →
   `(2, "[editor] focus: must keep at least one chord: the editor takes every other key")`;
   the overlay case's existing test still passes byte for byte.

`stack::tests`:
8. `declared_starts_with_base_and_overlay_and_holds_every_view_stack`: 38; plus
   `assert!(DECLARED.contains(&(WHILE_EDITING, Stack::EDITOR_OPEN)))`.

`keys/mod.rs` tests:
9. `the_editor_keys_do_not_collide_with_quit_or_help` → `the_editor_keys_do_not_collide_with_any_global_key`
   (every `[editor]` chord against every `Context::Global` row's chords: `EDITOR_OPEN` stacks
   them all).
10. New `editor_focus_shares_no_default_chord_with_any_action`: every compiled row other than
    focus is disjoint from focus's chords.

`print::tests`:
11. `a_changed_table_round_trips_and_keeps_its_marks`: `ctrl-x` → `ctrl-g` in the source and in
    the expected `quit         = ["ctrl-g"]   # quit (changed)` line.
12. New `a_changed_editor_table_round_trips`: `"[editor]\nfocus = [\"f12\"]\n"` loads, prints,
    reloads equal; the `focus` line starts `focus = ["f12"]` and ends `# editor focus
    (changed)`; one `(changed)`.

`tests/keys_file.rs` + `tests/fixtures/keys/valid.toml` (fixture comment: "Quit moves to
ctrl-g"): `:85` `["ctrl-g"]`, `:413` `Ctrl+g quit · Tab next tab`, `:420` `Ctrl+g/Ctrl+c quit`,
`:445-446` `harness.key("ctrl-g")` / "`Ctrl+g` quits"; module doc line 10 unchanged in meaning.
Verify `ctrl-g` reaches the stack on the tab the harness ends on (Backlog hands control chords
to its detail pane, `backlog/mod.rs:747-751`, which binds no `ctrl-g`); if any harness test
disagrees, use `ctrl-o` everywhere instead (and in T4's README).

### 3.5 Hazards for T1

- The stack pass orders errors by `DECLARED`; appending keeps every existing vector. Do not
  insert either entry earlier.
- `include_str!("../ui/overlay/quit_confirm.rs")` (T0, in `views.rs`) means `keys::` tests read
  T2's file: T2 must keep `views::QUIT_CONFIRM` in its production code (it does, in `key_stack`).

### 3.6 T1 commits and gate

1. `feat(mod-57): M2 T1 - editor.focus keeps a chord and never gives way` (`load.rs`,
   `validate.rs` `give_way`, `catalogue.rs` comment; tests 1, 2, 5, 7).
2. `feat(mod-57): M2 T1 - the open-edit stack joins DECLARED; the focus chord is refused in every context`
   (`stack.rs`, `validate.rs` pass, `keys/mod.rs`, `print.rs`, `tests/keys_file.rs`,
   `valid.toml`; tests 3, 4, 8-12). The fixture moves ride with the rule that refuses them, so
   each commit is green.

Gate: `cargo test -p htui --all-features --lib -- keys::`; `cargo test -p htui --all-features
--test keys_file -- --test-threads=1`; clippy both ways.

---

## 4. T2: the quit confirm (E4; parallel with T1)

**First failing test**: `ui::overlay::quit_confirm::tests::y_quits`.

### 4.1 Items (`ui/overlay/quit_confirm.rs`, replacing the stub; shaped as `MigrationPrompt`)

```rust
/// What the box asks (PD-M2-1: fixed; the factory takes no argument).
const QUESTION: &str = "An edit is still open in the editor. Quit and lose it?";
/// The line under the question: `confirm.yes` and `confirm.no` here, `overlay.close` (the same
/// effect as `no`) by the shell.
const HINT: HintSpec = &[
    Hint::One(Act::ConfirmYes, "quit"),
    Hint::Pair(Act::ConfirmNo, Act::OverlayClose, "keep editing"),
];
/// Left border, right border and one column of right padding.
const CHROME: u16 = 3;
```
- `on_key`: `for act in ctx.keys().actions(views::QUIT_CONFIRM, chord)`: `ConfirmYes` →
  `ctx.emit(Action::Quit)`, `Consumed` (no `Close`: the loop stops; `App` drops and the editor
  with it, `lib.rs`); `ConfirmNo` → `ctx.emit(Action::Overlay(OverlayAction::Close))`,
  `Consumed`; anything else `continue`. Then `Handled::Pass` (the shell's `apply_keys` takes
  `?`/`F1`; the modal path swallows the rest).
- `render`: `lines = [QUESTION (base), blank, hint (dim)]`, centred box sized to the widest line
  + `CHROME`, `Clear`, bordered block titled ` Quit ` in `theme.title`: `MigrationPrompt::render`
  verbatim with `views::QUIT_CONFIRM`.
- Module doc: E4/D5, `ctrl-c` never reaches it (C3 runs first in `on_key`), opened only by
  `App::request_quit`, why the question is fixed.

### 4.2 T2 tests, TDD order (a `Bench` as `waiting_list.rs:367-430`, `Origin::Overlay(QuitConfirm::ID)`)

1. `y_quits`: `y` → `Consumed`, emitted `[Action::Quit]` (match on the vector).
2. `n_and_esc_keep_the_edit`: `n`, `esc` → `Consumed`, emitted `[Overlay(Close)]` each.
3. `every_other_key_is_left_to_the_modal_swallow`: `q`, `Y`, `enter`, `ctrl-y`, `?` → `Pass`,
   nothing emitted.
4. `the_question_is_never_blank_and_names_the_cost`: `QUESTION` non-empty, contains `lose`.
5. `the_hint_follows_the_keys`: with `load_str("[confirm]\nyes = [\"f9\"]\n")` in the bench's
   `with_keys`, the rendered box contains `F9 quit · n/Esc keep editing`.
6. `the_box` (insta, 100x30 text frame as `waiting_list`'s `render`).

Gate: `cargo insta test -p htui --all-features -- quit_confirm` (review and accept the one
`.snap`); clippy both ways. Commit: `feat(mod-57): M2 T2 - the quit question overlay`.

---

## 5. T3: the shell (E1-E8; after T1, T2)

**First failing test**: `app::pane::tests::on_the_asking_tab_every_global_acts_and_tabs_switch`.

### 5.1 `App` (`state.rs`) and `register_all` (`app/mod.rs`)

```rust
/// The quit question (MOD-57 M2 E4): opened by `global.quit` while an in-pane editor is alive.
/// Set by [`register_all`](crate::app::register_all), an id for `migration_overlay`'s reason;
/// `None` (a bare shell) quits at once, as M1 did.
pub quit_overlay: Option<crate::ui::overlay::OverlayId>,
```
`App::new`: `quit_overlay: None`. `register_all`, after the migration prompt:
`app.overlay_factories.register(QuitConfirm::ID, || Box::new(QuitConfirm::new()));
app.quit_overlay = Some(QuitConfirm::ID);` and doc item 11 ("The quit question's factory goes
in and it is named as the quit overlay (MOD-57 M2 E4); no key opens it but `global.quit` with an
editor alive."). `use crate::ui::overlay::{…, QuitConfirm, …}`.

`action_for` (`state.rs:379`): `Act::Quit => Some(self.request_quit()),`.

### 5.2 `app/pane.rs`, normative

New constants and helpers (all private unless noted):
```rust
/// E5's tail: what the asking tab waits for.
const FINISH_FIRST: &str = "finish or abort it first";
/// E1/E6: the way back from another tab.
const BACK_HINT: HintSpec = &[Hint::One(Act::EditorFocus, "back")];

/// The asking tab's id while an editor is alive (E5's `reveal` check).
pub(super) fn editor_tab(&self) -> Option<TabId>;
/// The asking tab's title (`TabRegistry::titles`), else its id.
fn editor_title(&self) -> String;
/// `"{EDITOR_LOCKED} in {title}"`: the head of every off-screen sentence.
fn editor_place(&self) -> String;
/// MOD-57 M2 E4 (PRD D5): what `global.quit` resolves to. With an editor alive and
/// `quit_overlay` set, opening the quit question; else `Action::Quit`. `ctrl-c` never comes here.
#[must_use] pub(super) fn request_quit(&self) -> Action;
/// MOD-57 M2 E5: `Some(sentence)` while an editor is alive.
#[must_use] pub(super) fn editor_steer_refusal(&self) -> Option<String>;
/// MOD-57 M2 E6: the status prefix on any tab but the asking one; `None` there and without an
/// editor.
#[must_use] pub(super) fn editor_notice(&self) -> Option<String>;
```

**`editor_key`** (replaces `pane.rs:306-362`; returns `false` to hand the key to `on_key`'s path
below the editor step). In order, first match wins:
1. `editor_lost_keys()` → `true` (unchanged rule: a focused editor without the keys, the first
   key is swallowed with `lock_refusal()` and unfocuses it).
2. **The editor has the keys** (`editor.focused`, which after step 1 implies on screen, uncovered,
   no `?` box): `EDITOR_FOCUSED` resolves `EditorFocus` → unfocus; else encode and write. `true`.
   Unchanged.
3. `chord == CTRL_C` → `false` (`on_key`'s C3 quits; one C3 site).
4. `editor_covered()` → `false` (the modal overlay's keys, E1: `editor.focus` does not act
   there; the quit question is such an overlay).
5. **Another tab** (`!editor_visible()`): if `keys.actions(Stack::EDITOR_FOCUSED, chord)`
   contains `EditorFocus` → `focused = true`, `help_visible = false`,
   `update(Action::Tab(TabAction::Focus(tab)))` (B16), `true`. Otherwise `false`: the key runs
   as if no editor were alive (C3, overlays, the tab, its legacy rows, its stack). `editor.abort`
   is not checked here (E2).
6. **The asking tab** (on screen, unfocused, uncovered; the `?` box may be up): the first
   candidate of `keys.actions(Stack::EDITOR_OPEN, chord)` that maps:
   `EditorFocus` → `focused = true`, `help_visible = false`; `EditorAbort` → `abort_editor()`;
   a global act → `action_for(act)` (`Quit` → `request_quit()`, `Help`, tabs, and the offered
   `Workspaces`/`Find`/`Waiting`/`Queue`) → `update(action)`. No candidate maps →
   `status = lock_refusal()`. `true`: nothing reaches the asking view, its legacy rows or an
   overlay.

**`editor_paste`** (`pane.rs:369-383`): 1. lost keys → `true`; 2. has the keys → write, `true`;
3. covered → `false` (overlay path); 4. another tab → `false` (that tab's paste path);
5. the asking tab, unfocused → `true` (dropped, as M1).

**`lock_refusal`**: covered → `"{EDITOR_LOCKED}: {PROMPT_FIRST}"` (unchanged); another tab →
`"{place}: {back}"`, `back = keys.hint(EDITOR_FOCUSED, BACK_HINT)` (`the editor is open in
Skills: Ctrl+4 back`; `place` alone when unbound); the asking tab → M1's
`"{EDITOR_LOCKED}: {hint(EDITOR_OPEN, LOCK_HINT)}"` (unchanged text).

**`editor_status`**: `None` without an editor **or when its tab is not the active one**; else
M1's three lines through `Stack::EDITOR_OPEN` (unchanged text).

**`editor_notice`**: `None` without an editor or on the asking tab. Else `head = "editing in
{title}"`; `back` as above, dropped when `editor_covered()` or empty; `Some(format!("{head} ·
{back} · "))` or `Some(format!("{head} · "))`. Always ends with `" · "`.

**`editor_steer_refusal`**: `self.editor.as_ref()?;` then
`Some(format!("{}: {FINISH_FIRST}", self.editor_place()))` → `the editor is open in Skills:
finish or abort it first`.

**`request_quit`**: `match (self.editor.is_some(), self.quit_overlay) { (true, Some(id)) =>
Action::Overlay(OverlayAction::Open(id)), _ => Action::Quit }`.

`draw_editor`, `open_editor`, `on_pane_event`, `abort_editor`, `resize_editor`: unchanged.
Module doc: M1 → M2 (keys bullet rewritten to §5.2's order; E4-E8 one line each).

### 5.3 `state.rs` edits

1. `on_key` (`:783-787`): comment "a live in-pane editor answers first: focused, every key; on
   the asking tab, the editor's and the global keys only (E3); on another tab, only
   `editor.focus` (E1)". Code unchanged (`if self.editor.is_some() && self.editor_key(..)`).
   The C3 comment "MOD-57's exception is the step above" stays.
2. `on_paste` doc: the M2 rule (another tab's paste is that tab's).
3. `wants_mouse` (`:673-679`): `!self.editor_visible() && …`; doc: "MOD-57 M2 E8: off while the
   in-pane editor is on screen (the pane draws over the view); on another tab the view decides."
   `editor_visible` becomes `pub(super)`.
4. `render` status (`:942-952`):
   ```rust
   None => (
       self.editor_status().unwrap_or_else(|| {
           let line = self.keys.status_line(…unchanged…);
           match self.editor_notice() {
               Some(notice) => format!("{notice}{line}"),
               None => line,
           }
       }),
       self.theme.dim,
   ),
   ```
5. `render_help` (§5.6).

### 5.4 `update.rs` (E5)

- `update` (`:30`): `Action::SetScope { workspace } => match self.editor_steer_refusal() {
  Some(refusal) => self.status = Some(refusal), None => self.set_scope(workspace) },` with the
  comment "MOD-57 M2 E5: `set_scope` resets every tab, the asking one included".
- `reveal` (`:275`), after `tab` is found and before `update_tab`: `if self.editor_tab() ==
  Some(tab) && let Some(refusal) = self.editor_steer_refusal() { self.status = Some(refusal);
  return; }`; doc gains the sentence.
- `Replay`/`Promote` (Chat), `TabAction::FocusSection` (only Settings, which never hands off)
  and `TabAction::Focus` (a tab switch, E3) are not refused.

### 5.5 E6 exact text

`editing in Skills · Ctrl+4 back · ` + `keys.status_line(active stack)`; under a modal overlay
on another tab `editing in Skills · ` + that overlay's line. Test tabs read `editing in asker ·
Ctrl+4 back · q quit · …`.

### 5.6 E7 and E8

`render_help` (`state.rs:980-1036`), normative:
```rust
// MOD-57 M2 E7 (PD-M2-2): the editor keys that act where the user is.
let locked = self.editor.is_some() && self.editor_visible() && !self.editor_covered();
let elsewhere = self.editor.is_some() && !self.editor_visible() && !self.editor_covered();
let mut lines: Vec<HelpLine> = Vec::new();
if elsewhere {
    lines.extend(self.keys.help_line(Context::Editor, |act| act == Act::EditorFocus));
}
let stack = if locked { Some(Stack::EDITOR_OPEN) } else { self.active_stack() };
let closer = match stack {
    Some(stack) => { lines.extend(self.keys.help_lines(stack, offered)); self.keys.help_closer(stack) }
    None => { /* today's branch minus the M1 `if self.editor.is_some()` editor line */ }
};
```
On the asking tab: `Editor: Ctrl+4 editor focus · Ctrl+x abort edit`, `Global: q/Ctrl+c quit ·
…`, closer `?/F1 closes this box`. Elsewhere: `Editor: Ctrl+4 editor focus` first. Doc updated.

E8: §5.3 item 3. MOD-74: no change to `mouse_capture`/`lose_mouse`; `take_external_edit`'s
mouse loss (`:516-519`) is unchanged and harmless in pane mode (the next `mouse_capture` agrees).

### 5.7 T3 tests (`app::pane::tests`), TDD order

Fixture changes: `Editing` gains `stack: Option<Stack<'static>>` (returned by `key_stack`) and
`lost: Rc<Cell<u32>>` (counted in `on_mouse_lost`); `shell` keeps `mouse: false, stack: None`. A
`fn confirmable(app)` registers `QuitConfirm` and sets `quit_overlay`. `ASKER`'s title is
`asker`. All plain `#[test]`, fake child, no runtime.

1. `on_the_asking_tab_every_global_acts_and_tabs_switch`: unfocus; `tab` → `OTHER` active,
   editor alive, a draw has no ` nvim · `, no write, `ASKER` saw nothing; `1` → `ASKER`, still
   unfocused (`UNFOCUSED_STATUS`).
2. `on_the_asking_tab_a_non_global_key_is_refused_and_never_reaches_it` (rewrite of
   `unfocused_every_other_key_is_refused_and_nothing_moves`): `?` toggles; `j`, `e`, `ctrl-s`,
   `esc` → `REFUSAL`, active `ASKER`, no tab saw a key, nothing written, no edit asked.
3. `on_another_tab_keys_reach_that_tab`: after `tab`, `j`, `x`, `esc` reach `OTHER`; nothing
   written; no status.
4. `the_focus_key_from_another_tab_brings_the_editor_back_focused`: on `OTHER` with the `?` box
   up, `ctrl-4` → `ASKER` active, `FOCUSED_STATUS`, box closed, `ctrl-4` not written.
5. `abort_acts_only_on_the_asking_tab`: on `OTHER`, `ctrl-x` reaches `OTHER`, editor alive; `1`
   then `ctrl-x` → aborted, `ASKER` heard `Failed(EDITOR_ABORTED)`.
6. `unfocused_ctrl_c_quits_at_once_and_q_asks_first` (rewrite of
   `unfocused_ctrl_c_and_q_quit_at_once`): `ctrl-c` → `should_quit`, nothing written, `Dropped`
   on drop; `q` (confirmable) → top overlay `QuitConfirm::ID`, not quit; a bare shell's `q`
   quits at once (fallback).
7. `q_asks_first_y_quits_and_n_keeps_the_editor`: from `ASKER` and from `OTHER`: `q` → question;
   `n` → closed, editor alive and unfocused; `q`, `esc` → closed; `q`, `y` → `should_quit`;
   drop → `Dropped`.
8. `ctrl_c_quits_at_once_on_any_tab_and_under_the_question`.
9. `q_under_the_question_is_swallowed_and_opens_no_second_box`: `overlays.len() == 1`.
10. `an_editor_that_exits_under_the_question_leaves_it_up_and_y_still_quits` (B-M2-5).
11. `a_second_edit_from_another_tab_is_answered_editor_busy`: `e` on `OTHER` →
    `take_external_edit()` is `None`, `OTHER` heard `[Failed(EDITOR_BUSY)]`, `ASKER` nothing,
    editor alive.
12. `a_scope_change_is_refused_while_an_editor_is_alive`: `update(SetScope{..})` (a
    `WorkspaceSummary` built as `update.rs`'s test helper `workspace`) → status `the editor is
    open in asker: finish or abort it first`, `scope`/`top_bar.workspace` unchanged; after
    `Exited`, the same action applies.
13. `a_reveal_into_the_asking_tab_is_refused_and_one_elsewhere_passes`: `reveal_tabs =
    [(Item, ASKER), (Requirement, OTHER)]`, active `OTHER`; an `Item` reveal → refusal, active
    `OTHER`; from `ASKER`'s side a `Requirement` reveal → `OTHER` active, no status.
14. `the_status_line_on_another_tab_carries_the_edit_and_the_way_back`: exact prefix on row 29;
    rebound focus `f12` → `F12 back`; an error status replaces the line; a modal `Shade` on
    `OTHER` → `editing in asker · ` with no back.
15. `the_help_box_lists_the_keys_that_act_here`: `ASKER` box has the two-entry Editor line and
    no `asker` legacy line; `OTHER` (with `stack: Some(views::SETTINGS_TAB)`, a converted view)
    has `Editor: Ctrl+4 editor focus` and no `abort edit`; under `Prompt` on `ASKER`, no `Editor:`.
16. `mouse_capture_follows_the_pane_on_screen` (rewrite of
    `mouse_capture_is_off_while_an_editor_is_open`): `OTHER` wants the mouse; on `ASKER` off; `tab`
    → on; `ctrl-4` → off and `OTHER`'s `lost` = 1; after `Exited` on `OTHER` → on.
17. `on_another_tab_a_paste_reaches_that_tab`, and the asking tab's unfocused paste is dropped.
18. Rewrites of `the_pane_is_drawn_only_in_its_tab_and_refocus_brings_it_back`,
    `the_key_that_finds_a_focused_editor_off_screen_is_swallowed` and
    `a_paste_that_finds_a_focused_editor_off_screen_is_swallowed`: the swallow's status is
    `the editor is open in asker: Ctrl+4 back`; off screen `editor_status()` is `None` and row 29
    starts `editing in asker · Ctrl+4 back · `; after the swallow the next key or paste reaches
    `OTHER` (not refused); `ctrl-c` still quits. The M1 swallow assertions (never quits, never
    aborts, nothing written, nothing moves) are kept.
19. Snapshots (insta, 100x30): `another_tab_while_editing` (frame on `OTHER`, status prefix) and
    `the_help_box_on_the_asking_tab`.

Kept green unchanged: every other M1 test, including `the_title_status_and_help_come_from_the_keys`
(its `Editor: Ctrl+4 editor focus · Ctrl+x abort edit` assertion holds through `EDITOR_OPEN`'s
box), `closing_the_overlay_hands_the_lock_back`, the modal-overlay tests and
`the_pane_draws_over_the_claimed_rect` (its snapshot does not move).

### 5.8 End to end (`tests/editor_pane.rs`, real time, real `sh` child)

1. `the_editor_waits_while_another_tab_works_and_reads_back_on_return`: the M1 read-back script;
   `E` on `implement`; `pump` until ready; `ctrl-4`; `1` → the frame shows Backlog, not `READY`,
   and its last line starts `editing in Skills · Ctrl+4 back`; `?` → `Editor: Ctrl+4 editor
   focus`; `?`; `ctrl-4` → `READY` and `Ctrl+4 to htui` back; `x y z enter` → exit → `edited in
   $EDITOR`, `xyz` in the draft, temp file gone.
2. `quitting_with_a_live_editor_leaves_no_child` (rewrite): `ctrl-4`, `q` → not quit, the top
   overlay is `QuitConfirm::ID`, the editor alive; `y` → `should_quit`; drop; `gone(pid)`.
3. `ctrl_c_with_htui_holding_the_keys_quits_at_once_and_leaves_no_child` (B-M2-8).

### 5.9 T3 commits and gate

1. `feat(mod-57): M2 T3 - the asking tab alone stays locked; editor.focus from every tab`
   (`pane.rs` key/paste/refusal/status, `state.rs` `on_key`/`on_paste` docs and `wants_mouse`;
   tests 1-5, 11, 16-18).
2. `feat(mod-57): M2 T3 - quit asks first while an edit is open` (`request_quit`, `action_for`,
   `quit_overlay`, `register_all`; tests 6-10).
3. `feat(mod-57): M2 T3 - nothing steers the asking tab; the status line and help name the edit`
   (`update.rs`, `render`, `render_help`, the two snapshots; tests 12-15, 19).
4. `test(mod-57): M2 T3 - end to end: another tab while the editor waits, quit through the question`.

Gate: `cargo test -p htui --all-features --lib -- app::`; `cargo test -p htui --features testkit
--test editor_pane -- --test-threads=1`; `cargo insta test -p htui --all-features --check`;
clippy both ways; `cargo doc -p htui --no-deps`; Windows cross-check.

---

## 6. T4: docs and close-out (after T3; no code)

- `docs/htui-editor.md`: TOC line 21 and the section "While the editor is open (the M1 lock)"
  → "While the editor is open" (asking tab locked, globals act, other tabs free, the quit
  question, E5 refusals with F-M2-5's Settings consequences); the keys table (`:120-126`):
  `Ctrl+4` from any tab (not under a modal prompt), `Ctrl+x` on the asking tab only, `q` asks
  first (`y` quits and kills the editor, `n`/`Esc` keep editing), `Ctrl+C` quits at once
  everywhere htui has the keys; the status table gains the other-tab row; `:142-144` ("arrives
  with MOD-67's user keymap; `editor.focus` will have to stay bound") → the four E9 rules with
  their messages; Windows section: the quit question joins MOD-16's ConPTY list.
- `README.md`: the "Changing keys" example `quit = ["ctrl-g"]` (F-M2-3); one sentence: "`[editor]
  focus` must keep a key, and no other action may use it or the editor's keys while an edit is
  open (see `docs/htui-editor.md`)".
- Review gate (`rust-reviewer`, unpinned per memory) over T0-T3; fixes test-first, one commit per
  finding.
- Close-out per `.claude/skills/handoff-run/references/lifecycle.md`: `docs/decisions/mod/mod-57.md`
  (new: D1-D6, P1-P10, E1-E10, PD-M2-*, E10's R1 L-2 deferral, Q-A's owed host checks: the nvim
  run and the 10-edit hypothesis); `DECISIONS.md` index line; `HANDOFF.md` MOD-57 entry closed
  with the owed checks, the MOD-67 note (`:357-358`, "whichever lands second") resolved, the
  MOD-16 entry (`:213`) gains the quit question; the PRD's M2 row `complete`; the plan's Status
  `DONE`; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

Commits: `docs(mod-57): M2 T4 - the editor docs and README for M2`, then
`docs(mod-57): close-out - …`. Gate: the plan's full Validation block.

---

## 7. Data flow and cross-task contracts

- Key on the asking tab, unfocused: `on_key` → `editor_key` step 6 → `keys.actions(EDITOR_OPEN)`
  → `action_for` → `update` (a tab switch leaves the editor running and undrawn; `q` opens the
  question). Key on another tab: step 5 → `false` → the ordinary chain. `ctrl-c`: always
  `on_key`'s C3.
- Quit: `request_quit` → `Overlay(Open(QuitConfirm::ID))` → factory → `push_overlay` → (modal,
  so `editor_covered()`) → `QuitConfirm::on_key` → `y` emits `Action::Quit` → `should_quit` →
  loop ends → `App` drops → `PtyChild` drop requests the kill (M1 PD-7).
- Steering: `update(SetScope)`/`reveal(target)` → `editor_steer_refusal` → status line.
- **T0 → T1, T2, T3**: `Stack::EDITOR_OPEN`, `views::QUIT_CONFIRM`, `QuitConfirm::{ID, new}`,
  `pub use QuitConfirm`. **T1 → T3**: `WHILE_EDITING` (T3 does not need it); the rules are
  load-time only, so T3 compiles against T0 alone. **T2 → T3**: the confirm's behaviour (`y`
  emits `Action::Quit`, `n`/`Esc` close), tested in T3 tests 7-10 through the real overlay.
- **T1 ∩ T2 = ∅**: T1 = `keys/{load,validate,stack,mod,print,catalogue}.rs`,
  `tests/keys_file.rs`, `tests/fixtures/keys/valid.toml`; T2 = `ui/overlay/quit_confirm.rs` and
  its snapshot. Only coupling: `views.rs`'s `include_str!` of T2's file (§3.5).

## 8. Hazards

| # | Hazard | Mitigation |
|---|---|---|
| H-1 | T1 ∥ T2 share one tree and one `target/`: a half-written file in one lane fails the other's `cargo test`. | Each lane keeps its files compiling between saves (tests first, then code, never a dangling `use`); on a red gate, read the error's file before suspecting your own. Re-run both gates after both land. Watch `df -h .` (target fills the disk). |
| H-2 | Snapshot churn. | Only frames with a live editor can move; the one existing such frame (`the_pane_over_the_claimed_rect`) is the asking tab, focused, unchanged by M2. Full `cargo insta test --check` at T3 and T4; three new `.snap`s, each reviewed. |
| H-3 | Paused time beside a real child (memory). | Every `app::pane` test is a plain `#[test]` with the fake child; every real-child test is in `tests/editor_pane.rs` on real time. |
| H-4 | Featureless clippy. | `QuitConfirm`, `request_quit`, `editor_notice`, `editor_steer_refusal`, `editor_tab` are all reached from production (`register_all`, `action_for`, `render`, `update`); `editor_file` stays `#[cfg(test)]`. Run `cargo clippy -p htui -- -D warnings`. |
| H-5 | Windows. | Nothing new is unix-specific; `tests/editor_pane.rs` is `#![cfg(all(unix, feature = "testkit"))]`. Cross-check after T3. |
| H-6 | Scheduling-dependent suite; `tests/*.rs` need `testkit`. | `--test-threads=1`; `--features testkit` (or `--all-features`) for `editor_pane` and `keys_file`'s app cases, else they run 0 tests and pass. |
| H-7 | The chord moved off `ctrl-x` must agree in four places. | `print.rs`, `valid.toml`, `keys_file.rs`, README all use `ctrl-g` (or all `ctrl-o`, §3.4). |
| H-8 | `editor_lost_keys` and the help box both read `help_visible`. | A focused editor never has the box up (open closes it, refocus closes it, `?` while focused is the editor's); §5.6's `locked` treats that unreachable case as locked. |
| H-9 | `y` reaching a view the question uncovered. | The question never closes itself (B-M2-5); only `n`/`Esc` (consumed) or `y` (quit) end it. |

## 9. Count pins

| Pin | Before | After |
|---|---|---|
| `stack.rs:337` `DECLARED.len()` | 36 | 37 (T0), 38 (T1) |
| `views.rs:394` view-led stacks | `DECLARED[4..]`, 32 | filter by `NOT_VIEW_LED`, still 32 |
| `views.rs:523` `views.rs` consts | `DECLARED.len() - 2` (34) | filter by `SHELL` (35) |
| `views.rs:478` `VIEWS` | 15 files | 16 |
| `print.rs` `(changed)` counts | 3 / 2 | unchanged |
| `Act`, `Context`, `CATALOGUE` | 43 / 8 / 43 | unchanged |
| `the_view_stacks_resolve_as_designed` | - | +7 asserts (§2.3) |

## 10. Plan deviations (for the maintainer; none reopens E1-E10's intent)

- **PD-M2-1 — the quit question is fixed text.** Plan T2: "the question names the editor's value
  when given one". Evidence: F-M2-4 (`registry.rs:68,95`, `Fn() -> Box<dyn Overlay>`; the shell
  never names a concrete overlay). Change: `QUESTION = "An edit is still open in the editor. Quit
  and lose it?"`; T2's test becomes "never blank, names the cost". Alternative if the value is
  wanted: a `Ctx` field for the open edit, which widens `Ctx` for one sentence (not proposed).
- **PD-M2-2 — the `?` box lists the editor keys that act where the user is.** Plan E7: the
  `editor` context "in every view". Evidence: F-M2-7 (M1 R0 F3 offers no editor key under a
  modal prompt; E2 makes `Ctrl+x` inert off the asking tab; the asking tab's legacy rows are
  refused by E3). Change: §5.6 (asking tab: `EDITOR_OPEN`'s box; elsewhere: `Editor: Ctrl+4
  editor focus` first; under a modal overlay: none).
- **PD-M2-3 — file sets.** Plan Files/Independence tables. Evidence: F-M2-1, F-M2-2, F-M2-3,
  F-M2-10, F-M2-11. Change: T0 also edits `keys/catalogue.rs` (comments), the `views.rs` tests
  and `app/pane.rs` (rename only); T1 also edits `keys/stack.rs` (`DECLARED` line, count,
  `WHILE_EDITING`), `keys/catalogue.rs` (comment), `tests/keys_file.rs` and
  `tests/fixtures/keys/valid.toml`; T4's README row is required (the example). T1 ∩ T2 stays ∅.

## 11. Blueprint decisions

| # | Decision | Why |
|---|---|---|
| B-M2-1 | Both new `DECLARED` entries are appended, never inserted. | Collision errors come out in `DECLARED` order and are deduped by pair; appending keeps every existing error vector byte for byte. |
| B-M2-2 | `QUIT_CONFIRM` lives in `views.rs` with no view layer; no `Context::QuitConfirm`. | (a): everything reads layers generically; a view context would add a `[quit]` table, catalogue rows and `--print-keys` output for one yes/no. |
| B-M2-3 | `request_quit(&self) -> Action`; no new `Action` variant. | F-M2-6: `action_for` is `&self`; one mapping point stays, and `offered` keeps reading `is_some()`. |
| B-M2-4 | `quit_overlay: Option<OverlayId>`, `None` quits at once. | The `migration_overlay` pattern; a bare shell (most `app::` tests) keeps M1's behaviour. |
| B-M2-5 | An editor that exits (or a reply that lands) under the question leaves it up. | Its `y` still does what the user asked (quit), `n` closes; closing it under the user would send their next `y`/`n` to the view it uncovered (M1 T5 note 1's reason). |
| B-M2-6 | E5 refuses every `SetScope` while an editor is alive, the Settings follow included. | E5's reason holds for every emitter (`set_scope` resets every tab); the docs state the Settings consequence (F-M2-5). |
| B-M2-7 | One C3 site: the editor step returns `false` on `ctrl-c`. | Fewer `Action::Quit` routes to audit; behaviour identical. |
| B-M2-8 | A third e2e case: `ctrl-c` with htui holding the keys quits at once and leaves no child. | D5's other half, end to end, for the cost of one script. |
| B-M2-9 | The off-screen swallow sentence is `the editor is open in <Tab>: Ctrl+4 back`, sharing E5's head. | F-M2-8: M1's tail offers `Ctrl+x`, inert there after E2. |

## 12. Before T0 (for the main thread)

1. Accept or amend PD-M2-1, PD-M2-2, PD-M2-3.
2. The replacement chord for the `ctrl-x` fixtures and the README example: `ctrl-g` proposed
   (fallback `ctrl-o`).
3. Before T3 at the latest: F-M2-5's consequence of E5 (Settings' project and workspace-delete
   scope follow refused during an edit). B-M2-6 keeps E5 literal; the alternative (park the last
   refused `SetScope` and apply it when the editor ends) is not proposed: it would switch a
   workspace the user had been told was refused.

## Files

### Files to create
| File | Task |
|---|---|
| `crates/htui/src/ui/overlay/quit_confirm.rs` | T0 (stub), T2 |
| `crates/htui/src/ui/overlay/snapshots/htui__ui__overlay__quit_confirm__tests__the_box.snap` | T2 |
| `crates/htui/src/app/snapshots/htui__app__pane__tests__another_tab_while_editing.snap` | T3 |
| `crates/htui/src/app/snapshots/htui__app__pane__tests__the_help_box_on_the_asking_tab.snap` | T3 |
| `docs/decisions/mod/mod-57.md` | T4 |

### Files to modify
| File | Task |
|---|---|
| `crates/htui/src/keys/stack.rs` | T0, T1 |
| `crates/htui/src/keys/views.rs` | T0 |
| `crates/htui/src/keys/catalogue.rs` (comments) | T0, T1 |
| `crates/htui/src/ui/overlay/mod.rs` | T0 |
| `crates/htui/src/app/pane.rs` | T0 (rename), T3 |
| `crates/htui/src/keys/{load,validate,mod,print}.rs` | T1 |
| `crates/htui/tests/keys_file.rs`, `crates/htui/tests/fixtures/keys/valid.toml` | T1 |
| `crates/htui/src/app/{state,update,mod}.rs` | T3 |
| `crates/htui/tests/editor_pane.rs` | T3 |
| `docs/htui-editor.md`, `README.md`, `HANDOFF.md`, `DECISIONS.md`, the PRD, the plan | T4 |
