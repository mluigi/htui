# Blueprint: MOD-67 milestone 3, "Settings and overlays dispatch through context stacks"

**Status**: proposed (2026-10-07). Plan amendments PA-1 to PA-9 (§1) and answers to every lane
question (§2) are proposed here. D1-D14 stand except where §1 amends them, with the reason.

**Plan**: `.claude/plans/mod-67-m3-settings-overlays.plan.md` (confirmed 2026-10-07).
**Inputs**: the five lane inventories `.claude/plans/mod-67-m3-blueprint/L-{A,B,C,D,E}.md`,
`docs/ANA-26.md` §6-§7, the M2 blueprint for format.
**Verified at**: HEAD `fcfb00e8`, branch `hr/MOD-67`. `keys/{mod,catalogue,chord,stack,hint,load,
validate,print}.rs`, `app/state.rs` (`Ctx`, `App::{on_key, apply_keys, action_for, render,
render_help}`), `ui/tabs/registry.rs` (`Tab`), `ui/overlay/registry.rs` (`Overlay`),
`ui/tabs/settings/mod.rs`, `testkit.rs` (`Harness`, `SectionBench`), `ui/text_field.rs`,
`ui/text_area.rs`, `tests/keys.rs`, `tests/keys_file.rs` were read through Gortex at that HEAD.
Line numbers are pre-edit.

**Order**: T1 (serial, primary tree) → L-A ∥ L-B ∥ L-C ∥ L-D ∥ L-E (worktrees) → T7. Lanes add
no catalogue row, stack or allow-list entry: §4 and §5 are complete for all five. A missing one
goes back to the main thread (plan lane rule).

**New public surface (T1)**: `keys::views` (module, every stack constant); `Layer::{view, modal,
with_modal_filter, admits_chord, is_modal}`; `Stack::{admits, passes}`; `KeyChord::passes_modal`;
`Hint::{All, Text}`; `Keys::{labels, help_lines}`; `Keys::status_line` and `Keys::help_closer`
take a `Stack`; `Context::{ALL, is_view, is_shared}` and 15 contexts; 35 `Act` variants;
`keys::{SHADOWING, VIEW_DEFAULTS}`; `Tab::key_stack`, `Overlay::key_stack`,
`SettingsSection::key_stack`; `App::active_stack`; `Harness::with_keys`;
`SectionBench::with_keys`. Crate-private: `Row.extra`, `Keys::derive`, `Stack::{global,
view_admits}`, `validate::check`.

**House style (M2's, unchanged)**: `#![warn(missing_docs)]` (every `pub` item, field and variant
documented; every `pub` type derives `Debug`), no `pub` doc links a private item, inline format
args, `rustfmt` edition 2024 / width 100, clippy `-D warnings` both `--all-targets --all-features`
and featureless. Implementers commit incrementally, stage their own paths only. Gortex `read` for
source; if `Edit` is blocked, an anchored scripted replace (each anchor asserted to match once).

**The one-paragraph design.** Every mode of every converted view names a `const` stack in
`keys/views.rs`. A stack opens with `Layer::view(<view context>, <the mode's own acts>)`, which
also admits, by construction, every shared act a wider layer of the same stack offers: that is
where D10 override rows and D12 view defaults live, and why they never leak into a mode that does
not offer the act. A capturing or confirming mode ends with `Layer::modal(Global)`, which admits
only chords with CONTROL or ALT and function keys (`KeyChord::passes_modal`). One predicate,
`Stack::passes(chord)`, tells a modal view whether to return `Pass` for a chord it did not use,
bound or not. A text widget always sees the key first (D13 by construction). VIEW_DEFAULTS rows
are *extra* chords on top of whatever the shared row resolves to, re-derived after the key file
is merged. The status line, the `?` box and its closer render the active stack with each layer's
filter. Nothing converts in T1 except the Settings tab's own cycling, so no snapshot moves in T1.

---

## 1. Plan amendments

| # | Amends | Confirmed text | Why it cannot stand as written | What this blueprint builds |
|---|---|---|---|---|
| **PA-1** | D12 | "a compiled override in `VIEW_DEFAULTS: &[(Context, Act, &[&str])]`", uses: agents and hierarchy forms, migration `Y`/`N`. | (a) A compiled *replacement* row shadows the user's shared line: with `[form] next_field = ["ctrl-n"]` the five Settings forms keep `Tab`, so the user's change silently fails in Settings (L-B Q7, L-D Q2, L-E Q8). (b) The list is incomplete: kinds (`kinds.rs:773/777`), personas (`:1576/:1580`, report `:841`), secrets (`:674/:675`) and queue (`:617`) take extra chords today (L-C Q5, L-D §0, L-B §0). | **Additive** rows: `VIEW_DEFAULTS` holds only the *extra* chords. The row in force is "the shared row's chords, then each extra not already among them", re-derived after the file is merged. A user's explicit `[<view>] <name>` line replaces the whole row (ANA §7.1: a list replaces). 14 rows (§4.3). `--print-keys` prints them in the view's table, unmarked unless the file set them differently (§3.9). |
| **PA-2** | D11 | Seed `SHADOWING` with `form.next_field`/`global.next_tab` and `form.prev_field`/`global.prev_tab`. | D5's modal filter removes `Tab`/`BackTab` from the global layer of every stack that has a `form` layer (all are capturing). No M3 stack demands those pairs, and an undemanded entry is a blanket waiver the "test drives the list" rule forbids. | `SHADOWING = [(ConfirmNo, OverlayClose)]` only (migration prompt, `Esc`: PA-1 makes the derived `[migration] no` `["n", "esc", "N"]`). A test proves each `SHADOWING` and `STATE_GUARDED` entry is demanded (§7). M4 adds the form pairs if a non-modal form stack ever needs them. |
| **PA-3** | D3, D10 | Stack = view context, `settings`, shared layers narrowed with `only`, global. D10: an override is legal if the shared context "sits below that view context in some declared stack". | `Layer::all(view)` leaks every override/VIEW_DEFAULTS row into every mode of the view (`Down` → `form.next_field` ahead of `list.down` in hierarchy browse, `Tab` ahead of `global.next_tab`; L-C Q1, L-D Q1, L-A Q3). Hand-written `only` sets per mode would duplicate every shared layer's set ~30 times and drift. And L-B's `[settings.boxes] reload = "f5"` needs boxes' browse view layer to admit `Reload`. | `Layer::view(context, own)`: admits the mode's own acts **plus every shared act (`list`, `pane`, `confirm`, `form`, `common`, `settings`) that a wider layer of the same stack admits**. Never `global`/`overlay` acts. D10's legality rule becomes exact: `[V] name` is legal for a shared act `A` iff some declared stack has a `view(V)` layer that admits `A` (§3.2, §3.8). |
| **PA-4** | M2 D8 step 5 (context pass) | Two actions sharing a chord in one context are an error. | A view context now holds rows of different modes (agents' browse `o` and a consent-only `[settings.agents] yes = "o"`; `probe = "down"` vs the form's derived `next_field`). They never meet in one stack, and the stack pass already checks every declared stack. | The context pass skips view contexts (`Context::is_view`). Shared and global contexts keep it (they guard contexts no view offers yet). |
| **PA-5** | D6 | "in a modal mode gives the rest to its widget and then consumes". | Section-level pins return `Pass` for `ctrl-c` from an open editor, question or picker, where no shell checks `ctrl-c` first (`tests/settings.rs:2919`, `:3735`, `tests/hierarchy.rs:2926`, `tests/box_settings.rs:1168`, `:1395`, `:1922`; L-A Q1, L-B Q1, L-C Q4). `ctrl-c` is never a candidate. | One predicate: `Stack::passes(chord)` = the stack's global layer admits the chord's *shape* (unfiltered: always; modal: CONTROL/ALT/F-key). A modal mode returns `Pass` for every chord `passes` accepts and no view act took, bound or not; else `Consumed`. Text widgets see the key **first** (§6). This is MOD-52's CONTROL pass-through, widened to ALT and F-keys and written once. |
| **PA-6** | M1 `KeyChord::new` | Normalisation: `Shift+Tab` is `BackTab`; a `Char` drops SHIFT. | A kitty-protocol terminal reports ctrl-shift-d as `Char('D')`+CONTROL. Today concepts (`concepts_search.rs:295-303`) and `TextArea` (`text_area.rs:194`) accept `ctrl-D`/`ctrl-S` by hand; through the resolver `ctrl-D` ≠ `ctrl-d` and those keys would silently stop (L-E Q4). The key file cannot even write `ctrl-D` (`CtrlCapital`). | Third rule in `KeyChord::new`: CONTROL with an ASCII capital becomes the lower-case letter (`ctrl-D` → `ctrl-d`, `ctrl-alt-X` → `ctrl-alt-x`). `parse_strict` is unchanged (`refuse_char` runs before `new`). Side effect: a kitty `ctrl-C` now quits (`CTRL_C` check). |
| **PA-7** | D3 ("each with its error phrase") | One stack per mode. | Eleven modes are "a text field or an opaque widget, nothing else": a lone `Layer::modal(Global)` that can collide only inside `[global]`, which the context pass reports first. Eleven identical constants add nothing. | One shared `views::CAPTURE` (§5). Every mode with its own or shared acts keeps its own stack and phrase. |
| **PA-8** | D4 | `active_stack()`: "the top overlay's, else the active tab's, else `Stack::BASE`". | With an unconverted overlay (`key_stack` = `None`: the test doubles `Probe`/`Popup`/`Asking`) over a converted tab, the literal reading renders the tab's stack under the overlay, which D7 forbids ("an unconverted view keeps today's line"). | `active_stack() -> Option<Stack>`: the top overlay's `key_stack()` when an overlay is up, else the active tab's. `None` = today's status line and `?` box exactly. The dispatch steps use `unwrap_or(Stack::OVERLAY)` / `unwrap_or(Stack::BASE)` (§3.6). |
| **PA-9** | D7 | "quit is always shown, as `Ctrl+c quit` when its own chords are filtered out". | `hint.rs` `an_unbound_action_drops_out_of_the_status_line_and_a_hint` drops quit when the user unbinds `q`; the `?` box always lists it with `Ctrl+c`. Two rules for one fact. | Quit is always the first status entry: its first admitted chord, else `Ctrl+c` (unbound included). The test changes to `starts_with("Ctrl+c quit · Tab next tab")`. Only visible with `[global] quit = []`. |

D14's pin list grows (not an amendment, an addition the lanes found): secrets browse `ctrl-t`/`ctrl-e`
(L-D Q10), `alt-y` at the personas and secrets questions, `ctrl-y`/`alt-n` at the migration prompt
(L-E §12), `ctrl-r` in agents browse and `ctrl-e` in Qdrant browse (L-A §7).

---

## 2. Answers to every lane question

The twelve cross-lane issues of the brief are answered by one rule each; the row cites the rule.

| Q | Answer | Affects |
|---|---|---|
| L-A Q1 | **Rule 1 (PA-5).** Text widget first (today's order, D13 by construction); on `FieldOutcome::Pass` resolve the stack; no view act → `Pass` iff `stack.passes(chord)`, else `Consumed`. Skeleton (a), §6. | T1 `Stack::passes`; L-A form/paste/qdrant editors |
| L-A Q2 | Accepted as proposed: swallowed acts stay **out** of the consent/chooser stacks; the view keeps `CONSENT_SWALLOWS = [ListDown, ListUp, AgentsInstall, AgentsProbe, AgentsCancel]` and `CHOOSER_SWALLOWS = [AgentsAuthenticate, AgentsCancel]` and returns `Consumed` when `ctx.keys().actions(views::AGENTS_BROWSE, chord)` holds one of them (a rebound probe is swallowed under its new chord). `Down`/`Up` are newly swallowed in consent. | L-A |
| L-A Q3 | **Rule 2 (PA-3).** Every stack's first layer is `Layer::view(ctx, own)`. L-A's `only` sets become own-act lists (§5); shared acts are inherited, never listed. | T1 `views.rs` |
| L-A Q4 | Accepted: the chooser gains `Down`/`Up` (`list.down`/`list.up`). | L-A |
| L-A Q5 | Accepted: `All(ConfirmNo, "cancel")` → `n/Esc cancel`; `tests/settings.rs:2229` changes. **User-visible beyond D9** (adds `n/`). | L-A |
| L-A Q6 | Accepted: the agents hint is chosen from `self.stack()` first (FORM, CONSENT, CHOOSER, PASTE), browse falls back to today's `(install, auth)` match. | L-A |
| L-A Q7 | **Rule 8.** `htui::keys::load_str` is already `pub` and re-exported (`keys/mod.rs` `pub use load::{… load_str …}`; `lib.rs:27` `pub mod keys`). Integration tests build keys with `load_str("version = 1\n[…]\n").expect("…")`. `Keys::with_chords` stays `#[cfg(test)] pub(crate)`. T1 adds `Harness::with_keys(Keys)` and `SectionBench::with_keys(Keys)`. | T1 testkit |
| L-A Q8 | Accepted: manual-steps `Esc` is `common.dismiss`. | L-A |
| L-A Q9 | Accepted: one `settings.agents.cancel` for install and login. | T1 catalogue |
| L-A Q10 | Accepted: chooser `Enter` is `settings.agents.choose`. | T1 catalogue |
| L-A Q11 | **Rule 12.** A section with no `key_stack` cycles through `views::SETTINGS_TAB` (`[settings, global]`) as today. | T1 `settings/mod.rs` |
| L-B Q1 | **Rule 1.** `Stack::passes(chord)`; skeletons (a) and (b), §6. | T1, all lanes |
| L-B Q2 | Partly: the text-field-only modes share `views::CAPTURE` (PA-7). Confirmations are **per section** (`CONNECTION_CONFIRM`, `BOXES_EXECUTOR`, …) because each opens with its view layer, so `[settings.connection] yes = "Y"` is a legal override. The quirks/spec editors share `BOXES_EDITOR`. | T1 `views.rs`, L-B |
| L-B Q3 | **Rule 6.** An empty hint text renders the label(s) alone, no trailing space (`One` and `Pair`). `j/k · e edit · r reload` is unchanged; no prompt/queue/hierarchy/kinds snapshot moves. | T1 `hint.rs` |
| L-B Q4 | Confirmed: `allowed` checks the pair and "default of both", never one context. `STATE_GUARDED` gains `(BoxesExecutor, Workspaces)`. | T1 catalogue |
| L-B Q5 | **Rule 5.** `Context::ALL` (exhaustive, tested like `Act`'s `ALL`/`position`) replaces `contexts()`: the loader accepts `[settings.prompt]`, `[settings.queue]`, `[settings.qdrant]`, `[migration]`; the table list names them; `--print-keys` prints a context only when it has a row (prompt and qdrant print nothing by default). | T1 mod/load/print |
| L-B Q6 | **Rule 6.** The `?` box lists an act only under the first layer that resolves it (the `actions` shadowing rule): queue shows `Queue: e/Enter edit` and no `Common: e edit`. | T1 `hint.rs` |
| L-B Q7 | **Rule 3 (PA-1), additive.** `[common] edit = "E"` → queue's edit is `["E", "enter"]`. `[settings.queue] edit = …` replaces it. | T1 mod/load |
| L-B Q8 | Accepted: `settings.connection.activate` (`["enter"]`, help `run row`), declined off the Rebuild row. | T1 catalogue |
| L-B Q9 | Accepted: boxes browse still shows `w workspaces` on the status line. | — |
| L-B Q10 | **Rule 12.** `SettingsTab::on_key` resolves `settings.next_section`/`prev_section` through the active section's `key_stack()`, else `SETTINGS_TAB`. Every non-capturing section stack carries `Layer::all(Settings)`, so its view layer inherits the two acts and `[settings.boxes] next_section = "L"` is legal. | T1 |
| L-C Q1 | **Rule 2 (PA-3).** `Layer::view` admits only own acts and shared acts offered below; the form rows never reach browse. Test: every VIEW_DEFAULTS row is admitted by at least one declared stack. | T1 |
| L-C Q2 | **Rule 6.** Same as L-B Q3. | T1 |
| L-C Q3 | Accepted: `PathPicker` keys are widget-owned in M3 (like D13). The picker mode uses `views::CAPTURE` and skeleton (b′) (resolve, then `passes`, then the picker). Follow-up for M4/M6: a `picker` context (`down`, `up`, `choose`, `choose_here`, `goto`, `hidden`, `up_dir`). T7 records it in HANDOFF. | L-C, T7 |
| L-C Q4 | **Rule 1.** Opaque widgets (picker) get `passes` **before** the widget, so `ctrl-c` and any CONTROL/ALT/F-key leave the picker (`tests/hierarchy.rs:2926` green); `alt-j` no longer moves it. | L-C |
| L-C Q5 | **Rule 4.** Kinds' two rows are in VIEW_DEFAULTS; so are personas' and secrets' (T1 checked every multi-field editor: agents, hierarchy, kinds, personas, secrets; the single-field editors consume `Tab`/`Down` as no-ops, unchanged). | T1 |
| L-C Q6 | Accepted: `delete_question` takes the rendered `keys.hint(views::KINDS_CONFIRM, HINT_DELETING)`; default text identical. | L-C |
| L-C Q7 | Accepted: `HINT_COUNTING` → `counting rows… · n/Esc stop`. **User-visible beyond D9.** No test/snapshot pins it. | L-C |
| L-C Q8 | Accepted: one `KINDS_CONFIRM` for prefix warning, delete Asking and InFlight; InFlight consumes `ConfirmYes`/`ConfirmNo` as no-ops. | T1, L-C |
| L-C Q9 | Accepted: `CAPTURE` stacks show only the `Global` line in the `?` box. | — |
| L-D Q1 | **Rule 2 (PA-3)**, with the proposed test (§7 T-C6). | T1 |
| L-D Q2 | **Rule 3 (PA-1), additive, not "drop".** With `[form] next_field = ["ctrl-n"]` the secrets identity form moves on `ctrl-n` and `Down`, not on `Tab`: L-D's proposed pin holds. To drop `Down` too, the user writes `[settings.secrets] next_field = ["ctrl-n"]`. | T1, L-D |
| L-D Q3 | Accepted, same call as boxes: `PERSONAS_EDITOR` offers `form.save`; the `TextArea`'s own `ctrl-s` submit runs first (widget first, D13); a rebound `form.save` chord reaches the resolver because `TextArea` passes every other chord (`text_area.rs:192-199`). Hints use `One(FormSave, …)` → `Ctrl+s`. | L-B, L-D |
| L-D Q4 | **Rule 12 / PA-3.** Yes, automatically: browse view layers inherit `NextSection`/`PrevSection` from the `settings` layer. | T1 |
| L-D Q5 | Accepted: `Ctrl+S retries` prose stays for M6; the lane report notes the spelling mismatch. | L-D, M6 |
| L-D Q6 | **Not adopted.** Widget-first (Rule 1) makes widget keys unstealable by construction; a user chord on `left`/`enter`/… for an `in_capture` act is inert in a text mode, never harmful. README (T7) says so. No validator rule. | T7 README |
| L-D Q7 | **Rule 8.** `SectionBench::with_keys(Keys) -> Self` (sync builder after `SectionBench::new().await`). | T1 |
| L-D Q8 | Accepted: Report stays capturing (modal). | L-D |
| L-D Q9 | Convention: `Context::Settings<Section>`, `Act::<Section><Verb>` (`Act::AgentsProbe`, `Act::PersonasBody`, `Act::ConnectionRebuild` as ANA §7.6); overlays `Context::{Concepts, Switcher, Migration, Waiting}`, `Act::{ConceptsReindex, SwitcherSwitch, WaitingOpen}`. Headings are titles (§4.1). | T1 |
| L-D Q10 | Accepted (D14 addition, §1). | L-D |
| L-E Q1 | **Rule 6.** `Layer` carries `only` and a `modal` flag together: `Layer::only(Global, &[Help]).with_modal_filter()`. | T1 |
| L-E Q2 | Accepted: own `concepts.up`/`concepts.down` (`capture_row`). | T1 |
| L-E Q3 | **Rule 6.** Status over switcher/waiting/migration `Ctrl+c quit · ? help`; over concepts `Ctrl+c quit · F1 help`. Migration's global layer gets no modal filter. | T1, L-E |
| L-E Q4 | **Rule 7 (PA-6).** `KeyChord::new` folds CONTROL + ASCII capital to lower case. | T1 `chord.rs` |
| L-E Q5 | **Needs the maintainer.** Provisionally L-E's 93-cell text `Enter search/open · Up/Down move · Ctrl+d decisions · Ctrl+p scope · Ctrl+r index · Esc close` (labels `project`→`scope`, `re-index`→`index`). Alternative with the same width: keep `project`/`re-index`, write `Enter search` (drops "/open"). **User-visible beyond D9.** | L-E |
| L-E Q6 | Accepted: `list ∩ {down, up}` only. | — |
| L-E Q7 | **Rule 6.** `help_closer(stack)` filters by the stack's global layer: `F1 closes this box` over concepts and in every modal Settings mode. | T1 |
| L-E Q8 | **Rule 3, additive** (not L-E's "drop"): `[confirm] no = ["x"]` → migration `no` = `["x", "N"]`. | T1 |
| L-E Q9 | Accepted: headings `Search concepts`, `Workspaces`, `Schema`, `Waiting on you`; tables `concepts`, `switcher`, `migration`, `waiting`. | T1 |
| L-E Q10 | **Rule 11.** T7's reachability test exempts `Stack::BASE` and `Stack::OVERLAY` (shell fallbacks in `app/state.rs`). `SETTINGS_TAB` is referenced by `settings/mod.rs` and passes. | T7 |
| L-E Q11 | **Rule 10.** `src/snapshots/htui__testkit__tests__shell_empty.snap` → L-E. No other ownership change is needed: L-C's three status-row snapshots are `hierarchy__*` (theirs); no lane needs a file outside its row of the plan's table. | L-E |

**Rule 9 (swallow lists)** is L-A Q2. **Rule 4 (complete VIEW_DEFAULTS)** is §4.3.

---

## 3. T1 exact interfaces

### 3.1 `keys/chord.rs`

```rust
impl KeyChord {
    /// A normalised chord. Three rules, all forced by how terminals report keys: `Shift+Tab` is
    /// `BackTab` without a shift flag; a `Char` carries its shift in the character (`J`, `?`), so
    /// the flag is dropped; and CONTROL with an ASCII capital is the lower-case letter, since a
    /// kitty-protocol terminal reports ctrl-shift-d as `D` and the legacy encoding as `d`
    /// (MOD-67 M3 PA-6). `parse_strict` still refuses `ctrl-D` (it checks before normalising).
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self;   // body: after the SHIFT rule,
    //   if mods.contains(CONTROL) && let Char(c) = code && c.is_ascii_uppercase()
    //   { code = Char(c.to_ascii_lowercase()) }

    /// Whether a capturing or confirming mode lets this chord through to the global layer
    /// (MOD-67 D5): CONTROL or ALT is held, or the key is a function key. `ctrl-c`, `ctrl-f`,
    /// `alt-x`, `F1` pass; `q`, `?`, `Tab`, `Enter`, `Esc`, arrows do not.
    #[must_use]
    pub fn passes_modal(&self) -> bool;
    //   self.mods.intersects(CONTROL | ALT) || matches!(self.code, KeyCode::F(_))
}
```

### 3.2 `keys/stack.rs`

```rust
/// One layer of a stack: a context, optionally narrowed to some of its actions, optionally a
/// view layer that also admits the shared actions wider layers offer, optionally filtered to
/// the chords a capturing mode lets through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    context: Context,
    only: Option<&'static [Act]>,
    /// `Layer::view`: also admits every shared act (`Context::is_shared`) that a wider layer of
    /// the same stack admits (PA-3). Only `Stack::admits` reads it.
    inherit: bool,
    /// `Layer::modal`/`with_modal_filter`: only chords with `KeyChord::passes_modal` (D5).
    modal: bool,
}

impl Layer {
    pub const fn all(context: Context) -> Self;
    pub const fn only(context: Context, acts: &'static [Act]) -> Self;
    /// A view's layer (MOD-67 M3 PA-3): `own` (the mode's view verbs), plus every shared act a
    /// wider layer of the same stack offers, so D10 override rows and `VIEW_DEFAULTS` rows of
    /// `context` apply in exactly the modes that offer the act. Never a global or overlay act.
    pub const fn view(context: Context, own: &'static [Act]) -> Self;
    /// Every action of `context`, chords filtered to `KeyChord::passes_modal` (D5): the global
    /// layer of a capturing or confirming mode.
    pub const fn modal(context: Context) -> Self;
    /// This layer with D5's chord filter: `Layer::only(Context::Global, &[Act::Help])
    /// .with_modal_filter()` is the concepts search's global layer (`?` is text, `F1` is help).
    pub const fn with_modal_filter(self) -> Self;
    pub const fn context(self) -> Context;
    /// Whether the layer's own set offers `act`, ignoring inheritance. Stack-aware code uses
    /// [`Stack::admits`].
    pub fn admits(self, act: Act) -> bool;
    /// Whether `chord` survives this layer's filter: always, unless the layer is modal.
    pub fn admits_chord(self, chord: KeyChord) -> bool;
    /// Whether the layer carries D5's filter.
    pub const fn is_modal(self) -> bool;
}

impl<'a> Stack<'a> {
    pub const fn new(layers: &'a [Layer]) -> Self;           // unchanged
    pub const fn layers(self) -> &'a [Layer];                 // unchanged
    /// Whether layer `index` offers `act`: its own set, or (a view layer) a shared act whose own
    /// context is a wider layer's context and that layer admits it.
    pub fn admits(self, index: usize, act: Act) -> bool;
    //   let layer = self.0[index];
    //   layer.admits(act) || (layer.inherit
    //       && act.spec().is_some_and(|spec| spec.context.is_shared()
    //           && self.0[index + 1..].iter().any(|wider|
    //                  wider.context() == spec.context && wider.admits(act))))
    /// Whether a modal view returns `Pass` for `chord` it did not use (MOD-67 M3 PA-5): the
    /// stack's global layer admits the chord's shape. `false` if the stack has no global layer.
    pub fn passes(self, chord: KeyChord) -> bool;
    /// The last layer whose context is `Global`, with its index.
    pub(crate) fn global(self) -> Option<(usize, Layer)>;
    /// Whether this stack has a layer of `context` that admits `act` (the D10 legality test).
    pub(crate) fn view_admits(self, context: Context, act: Act) -> bool;
}
```

`Stack::BASE` and `Stack::OVERLAY` are unchanged. `DECLARED` (still in `stack.rs`, still
`pub static`) lists, in this order: `("on every screen", BASE)`, `("over an overlay", OVERLAY)`,
then every `views.rs` constant with the phrase §5 gives, in §5's order (36 entries).

`Keys::actions(stack, chord)`: iterate `stack.layers().iter().enumerate()`; a row is in layer `i`
iff `row.context == layer.context() && stack.admits(i, row.act)`; shadowing (`seen`) is unchanged;
a row is a **candidate** iff it binds `chord` **and** `layer.admits_chord(chord)`. A filtered-out
chord still shadows (the act was seen). `resolve_row` becomes
`fn resolve_row(&self, stack, act) -> Option<(Layer, &Row)>` with the same `admits(i, act)` test.

### 3.3 `keys/catalogue.rs`

```rust
pub enum Context {
    Global, Overlay, List, Pane, Confirm, Form, Common,            // unchanged
    /// `[settings]`: the Settings tab's own keys (section cycling).
    Settings,
    SettingsAgents, SettingsHierarchy, SettingsKinds, SettingsPrompt, SettingsConnection,
    SettingsQdrant, SettingsBoxes, SettingsPersonas, SettingsSecrets, SettingsQueue,
    Concepts, Switcher, Migration, Waiting,
}
impl Context {
    /// Every context in table order: the key file's tables, `--print-keys`' order (M3, L-B Q5).
    pub const ALL: &'static [Context] = &[/* the 22 above, in declaration order */];
    pub const fn table(self) -> &'static str;     // §4.1
    pub const fn heading(self) -> &'static str;   // §4.1
    /// A view's own context (a Settings section or an overlay): hosts view verbs, D10 overrides
    /// and `VIEW_DEFAULTS`; skipped by the validator's per-context pass (PA-4).
    pub const fn is_view(self) -> bool;           // Settings<X> and the four overlay contexts
    /// A shared context a view layer may inherit and override (D10): `list`, `pane`, `confirm`,
    /// `form`, `common`, `settings`.
    pub const fn is_shared(self) -> bool;
}

/// Extra default chords a view adds to a shared act, on top of whatever the shared row resolves
/// to (MOD-67 M3 PA-1). `Keys` derives the view's row after the key file is merged; an explicit
/// `[<view>] <name>` line replaces it. Each row cites the arm it mirrors.
pub static VIEW_DEFAULTS: &[(Context, Act, &[&str])] = &[/* §4.3 */];

/// Default pairs in one declared stack where the narrower act always wins (ANA §7.4 step 7,
/// "shadowing"; MOD-67 D11 as amended by PA-2): allowed only for a chord that is a default of
/// both, with the first act's layer narrower. Each entry is demanded by the compiled defaults.
pub static SHADOWING: &[(Act, Act)] = &[
    // migration_prompt.rs:97 + VIEW_DEFAULTS: `[migration] no` derives ["n", "esc", "N"], and
    // `Esc` is also overlay.close below it. Both close the prompt, nothing applied.
    (Act::ConfirmNo, Act::OverlayClose),
];

pub static STATE_GUARDED: &[(Act, Act)] = &[
    (Act::Back, Act::Dismiss),
    // boxes.rs:712-721: the executor question opens only over a listed box with a readable
    // list; otherwise `w` declines and falls through to the switcher (box_settings.rs:1356).
    (Act::BoxesExecutor, Act::Workspaces),
];
```
`Act` gains the 35 variants of §4.2 (each documented `/// \`settings.agents.probe\`: …`).

### 3.4 `keys/mod.rs`

- `pub mod views;` and re-exports `pub use catalogue::{…, SHADOWING, VIEW_DEFAULTS};`.
- `contexts()` is deleted; every caller uses `Context::ALL`.
- `Row` gains `extra: Option<Vec<KeyChord>>` — `Some` for a `VIEW_DEFAULTS` row (its parsed extra
  chords). `PartialEq for Row` still compares context, act, help, chords only.
- `Keys::defaults()`: the catalogue rows as today, then one row per `VIEW_DEFAULTS` entry
  (`help` = the act's catalogue help, `chords` empty, `line: None`, `extra: Some(parsed)`; panic
  message as for a bad default), then `self.derive(&[])`.
- `fn derive(&mut self, set_by_file: &[(Context, Act)])` (private):
  for each row with `extra: Some(extra)`: `derived` = the chords of `(act.spec().context, act)`
  (the shared row in force), then each chord of `extra` not already present. If
  `(row.context, row.act)` is in `set_by_file`: keep the row's chords, and set `line = None` when
  they equal `derived` (a printed file reads back unmarked); else `row.chords = derived`.
- `fn set(&mut self, context, act, chords, line)`: `changed` = `Keys::compiled()`'s row for
  `(context, act)` is absent **or** has other chords. An existing row is updated as today; a
  missing row (a D10 override) is **appended** (`help` = catalogue help, `extra: None`).
- New test-visible accessor is not needed: `Keys::chords(context, act)` already reads a derived or
  override row.

### 3.5 `keys/hint.rs`

```rust
pub enum Hint {
    /// `"{label} {text}"`; the label alone when `text` is empty.
    One(Act, &'static str),
    /// `"{label}/{label} {text}"`; one side unbound renders the other; empty `text`: labels alone.
    Pair(Act, Act, &'static str),
    /// Every admitted chord of `act`, joined by `/`, then `text`: `n/Esc cancel` (D9).
    All(Act, &'static str),
    /// Fixed text: a widget's own key (`Enter store`, `Esc cancel`) or a note
    /// (`typed text is never shown`). Never dropped (an empty one is).
    Text(&'static str),
}

impl Keys {
    /// First chord of `act` through `stack` that its layer admits (`admits_chord`).
    pub fn label(&self, stack: Stack<'_>, act: Act) -> Option<String>;
    /// Every chord of `act` through `stack` that its layer admits, as labels.
    pub fn labels(&self, stack: Stack<'_>, act: Act) -> Vec<String>;
    /// Unchanged contract; elements per `Hint` above, unbound ones dropped, ` · ` between.
    pub fn hint(&self, stack: Stack<'_>, spec: HintSpec) -> String;
    /// The status line (D7, PA-9) from `stack`'s global layer: quit first, always — its first
    /// admitted chord, else `Ctrl+c`; then every other `Global` row the layer admits that is
    /// `offered` and has an admitted chord, first admitted chord, rows sharing a help label
    /// collapsed to the first. `Stack::BASE` gives today's line byte for byte.
    pub fn status_line(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> String;
    /// The `?` box (D8): one line per layer of `stack`, narrowest first, heading
    /// `context.heading()`. A row is listed under the first layer that admits it and has a row
    /// for it (the `actions` shadowing rule, L-B Q6), with every chord that layer admits; rows
    /// sharing a help label merge; a row with no admitted chord drops. `offered` is consulted for
    /// `Global` rows only. The global layer always lists quit: its admitted chords (none when the
    /// layer does not admit `Quit`) plus `Ctrl+c`. A layer that lists nothing has no line.
    pub fn help_lines(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> Vec<HelpLine>;
    /// Unchanged: one context's line, for the legacy (`active_stack()` = `None`) box.
    pub fn help_line(&self, context: Context, offered: impl Fn(Act) -> bool) -> Option<HelpLine>;
    /// The box's last line from `global.help`'s chords that `stack`'s global layer admits:
    /// `?/F1 closes this box` on `BASE`, `F1 closes this box` in a modal stack; `None` if none.
    pub fn help_closer(&self, stack: Stack<'_>) -> Option<String>;
}
```

### 3.6 Shell: traits and `App`

```rust
// ui/tabs/registry.rs, trait Tab (next to focus_section's default)
/// This tab's key stack in its current mode (MOD-67 D4), or `None` while the tab is not converted
/// (Backlog, Skills, Requirements, Chat until M4/M5): the shell then keeps today's status line,
/// `?` box and `Stack::BASE` dispatch. Defaulted, so no other tab changes.
fn key_stack(&self) -> Option<Stack<'static>> { None }

// ui/overlay/registry.rs, trait Overlay — same doc, "`Stack::OVERLAY` dispatch"
fn key_stack(&self) -> Option<Stack<'static>> { None }

// ui/tabs/settings/mod.rs, trait SettingsSection — same doc, "the tab cycles through
// `views::SETTINGS_TAB`"
fn key_stack(&self) -> Option<Stack<'static>> { None }
```

`impl Tab for SettingsTab`: `fn key_stack(&self) -> Option<Stack<'static>> {
self.sections.active().and_then(SettingsSection::key_stack) }`.

`SettingsTab::on_key` (replaces the `match key.code`, `settings/mod.rs:337-358`):

```rust
fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    // A section that is taking typed text answers first: `l` is a letter there.
    if self.sections.active().is_some_and(SettingsSection::captures_input) {
        return self.delegate(key, ctx);
    }
    let stack = self
        .sections
        .active()
        .and_then(SettingsSection::key_stack)
        .unwrap_or(views::SETTINGS_TAB);
    match ctx.keys().actions(stack, KeyChord::from_event(key)).first() {
        Some(Act::NextSection) => { self.sections.cycle_next(); Handled::Consumed }
        Some(Act::PrevSection) => { self.sections.cycle_prev(); Handled::Consumed }
        _ => self.delegate(key, ctx),
    }
}
```
`KeyCode` leaves the non-test imports of `settings/mod.rs`. `ctrl-l`, `ctrl-h`, `shift-right`,
`alt-[` stop cycling (defect 1); an empty registry still consumes `l` (cycle on nothing), as today.

`App` (`app/state.rs`):

```rust
/// The stack of the view that holds the keyboard (MOD-67 D4, PA-8): the top overlay's when an
/// overlay is up, else the active tab's. `None` while that view is unconverted.
#[must_use]
pub fn active_stack(&self) -> Option<Stack<'static>>;
//   match self.overlays.top() { Some(top) => top.key_stack(),
//                               None => self.tabs.active().and_then(Tab::key_stack) }
```
- `on_key` overlay step (`state.rs:763`): `let stack = self.overlays.top().and_then(Overlay::key_stack)
  .unwrap_or(Stack::OVERLAY); if self.apply_keys(stack, chord) { return; }`.
- `on_key` last step (`state.rs:813`): `let stack = self.tabs.active().and_then(Tab::key_stack)
  .unwrap_or(Stack::BASE); self.apply_keys(stack, chord);` (the tab's, not `active_stack()`: a
  key reaches this step only when no modal overlay swallowed it). `ctrl-c` stays first.
- `apply_keys` unchanged: `action_for` returns `None` for every view and shared act, so a view
  act the view declined is skipped and the next candidate (a global one) applies.
- `render` status: `let stack = self.active_stack().unwrap_or(Stack::BASE);
  self.keys.status_line(stack, |act| self.action_for(act).is_some())`.
- `render_help`: `match self.active_stack()`: `Some(stack)` → `lines = keys.help_lines(stack,
  offered)`, closer `keys.help_closer(stack)`; `None` → today's three steps (overlay line if an
  overlay is up, the tab's legacy line, the global line), closer `keys.help_closer(Stack::BASE)`.

Strings with the defaults (T1 unit-tests them, lanes' snapshots must match):

| Active stack | Status line (`register_all`) | Status line (bare) | `?` closer |
|---|---|---|---|
| a browse stack (unfiltered global) | today's `FULL` | today's `BARE` | `?/F1 closes this box` |
| any modal Settings stack, `CAPTURE` | `Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting` | `Ctrl+c quit · F1 help` | `F1 closes this box` |
| `SWITCHER`, `WAITING_LIST`, `MIGRATION` | `Ctrl+c quit · ? help` | same | `?/F1 closes this box` |
| `CONCEPTS_QUERY` | `Ctrl+c quit · F1 help` | same | `F1 closes this box` |

### 3.7 `testkit.rs`

```rust
impl Harness {
    /// Runs the shell with these keys instead of the compiled defaults (MOD-67 M3): how a test
    /// rebinds. Build them with `htui::keys::load_str`.
    #[must_use]
    pub fn with_keys(mut self, keys: Keys) -> Self { self.app.keys = keys; self }
}
pub struct SectionBench { /* … */ keys: Keys }   // new field, doc "The keys in force"
impl SectionBench {
    // new(): keys: Keys::compiled().clone()
    /// Hands the section these keys instead of the compiled defaults (MOD-67 M3, L-D Q7).
    #[must_use]
    pub fn with_keys(mut self, keys: Keys) -> Self { self.keys = keys; self }
    // ctx(): Ctx::new(…).with_keys(&self.keys)
}
```

### 3.8 Loader (`keys/load.rs`): D10, D12

- `context_named(path)` searches `Context::ALL`. The unknown-table message lists every table of
  `Context::ALL`: `the tables are global, overlay, list, pane, confirm, form, common, settings,
  settings.agents, settings.hierarchy, settings.kinds, settings.prompt, settings.connection,
  settings.qdrant, settings.boxes, settings.personas, settings.secrets, settings.queue, concepts,
  switcher, migration, waiting`. `[settings.boxes]` walks through `settings` (now a context with
  no values in that header) without an error.
- `Loader` gains `set: Vec<(Context, Act)>` (every `(context, act)` an entry resolved to).
- Name resolution in `entry`: (1) the context's own row (`spec.context == context && spec.name ==
  name`); else (2) if `context.is_view()`, the overridable shared act: the `CATALOGUE` row with
  `spec.context.is_shared() && spec.name == name` such that
  `DECLARED.iter().any(|(_, stack)| stack.view_admits(context, spec.act))`. No shared name is
  ambiguous (`names_are_unique_across_shared_contexts`, §7).
- Unknown name, view context:
  `[{table}] {name}: no such action; [{table}] has {own…}, and may override {shared…}` — own
  names in catalogue order, then the overridable shared names in catalogue order; with no own row
  `[{table}] {name}: no such action; [{table}] may override {shared…}`. Non-view contexts keep
  today's message. Example (boxes inherits `list.down/up` and `settings.*` in browse, `common.reload/dismiss`,
  `form.save` in its editor, `confirm.yes/no` in its executor question; catalogue order):
  `[settings.boxes] relod: no such action; [settings.boxes] has edit_tags, edit_quirks, executor,
  probe, edit_spec, and may override down, up, yes, no, save, reload, dismiss, next_section,
  prev_section`. `[switcher] close` is "no such
  action" (D10: overlay names are not per view).
- After the walk: `keys.derive(&loader.set)`, **then** `validate(&keys)`.

### 3.9 `--print-keys` (`keys/print.rs`)

Iterate `Context::ALL`; a context with no row prints nothing (prompt, qdrant by default). A view
table prints its own rows (catalogue order), then its derived rows (`VIEW_DEFAULTS` order), then
override rows (file order). The name of a shared act is its catalogue name, the comment its help.
`(changed)` iff `line.is_some()`: a derived row the file did not set is never marked, even when a
changed shared row flowed into it. Round trip: `[form] next_field = ["ctrl-n"]` prints
`[form] next_field = ["ctrl-n"] … (changed)` and `[settings.agents] next_field = ["ctrl-n",
"down"]` unmarked; that print loads back equal and re-prints byte-identical (`derive` clears the
line of an explicit row equal to its derivation). An override `[settings.boxes] reload = ["f5"]`
prints in `[settings.boxes]` with `(changed)`, and `reload = []` also prints `(changed)` (the
compiled keys have no such row).

### 3.10 Validator (`keys/validate.rs`): D11, PA-2, PA-4

- `pub fn validate(keys) -> Vec<KeyFileError> { check(keys, SHADOWING, STATE_GUARDED) }`;
  `fn check(keys, shadowing, guarded)` holds today's body (the demand tests call it with a list
  minus one entry).
- Context pass: skip pairs whose context `is_view()` (PA-4).
- Stack pass: collect chords per layer with `stack.admits(i, act)` **and** `layer.admits_chord`;
  candidates from `keys.actions(stack, chord)` are narrowest first, so for a pair `(rows[i],
  rows[j])`, `i < j`: allowed if `guarded` holds the pair (either order) and the chord is a
  default of both (PA-2 of M2), **or** `shadowing` holds exactly `(rows[i].act, rows[j].act)` and
  the chord is a default of both. "Default" = `Keys::compiled()`'s row for that `(context, act)`
  binds it, so a derived compiled chord counts (migration's `esc`).
- `in_capture` pass unchanged (it reads `row.act.spec().in_capture`, so it covers override and
  derived rows of `form.*` and `concepts.*`).

---

## 4. Catalogue additions (T1 lands all of them; lanes consume)

### 4.1 Contexts (`Context::ALL` order = catalogue block order = strip order, D1)

Settings strip order is `register_all`'s (`app/mod.rs:65-82`): agents, hierarchy, kinds, prompt,
connection, qdrant, boxes, personas, secrets, queue. Headings are the section/overlay titles.

| Variant | `table()` | `heading()` | `is_view` | `is_shared` | Own rows |
|---|---|---|---|---|---|
| `Global`…`Common` (7) | unchanged | unchanged | no | `List`,`Pane`,`Confirm`,`Form`,`Common`: yes | unchanged |
| `Settings` | `settings` | `Settings` | no | **yes** | 2 |
| `SettingsAgents` | `settings.agents` | `Agents` | yes | no | 9 |
| `SettingsHierarchy` | `settings.hierarchy` | `Hierarchy` | yes | no | 4 |
| `SettingsKinds` | `settings.kinds` | `Kinds` | yes | no | 2 |
| `SettingsPrompt` | `settings.prompt` | `Prompt` | yes | no | 0 |
| `SettingsConnection` | `settings.connection` | `Connection` | yes | no | 2 |
| `SettingsQdrant` | `settings.qdrant` | `Qdrant` | yes | no | 0 |
| `SettingsBoxes` | `settings.boxes` | `Boxes` | yes | no | 5 |
| `SettingsPersonas` | `settings.personas` | `Personas` | yes | no | 3 |
| `SettingsSecrets` | `settings.secrets` | `Secrets` | yes | no | 1 |
| `SettingsQueue` | `settings.queue` | `Queue` | yes | no | 0 |
| `Concepts` | `concepts` | `Search concepts` | yes | no | 5 |
| `Switcher` | `switcher` | `Workspaces` | yes | no | 1 |
| `Migration` | `migration` | `Schema` | yes | no | 0 |
| `Waiting` | `waiting` | `Waiting on you` | yes | no | 1 |

### 4.2 New `Act` rows, in `CATALOGUE` order (35; total 41 → 76)

`row` = not in capture; `cap` = `capture_row`. Help is a phrase that reads alone in the `?` box;
the hint row keeps its own word through the `HintSpec` text. Each row's comment cites the arm.

| # | `Act` | Context | name | defaults | help | kind | mirrors |
|---|---|---|---|---|---|---|---|
| 1 | `NextSection` | `Settings` | `next_section` | `["l", "]", "right"]` | `next section` | row | `settings/mod.rs:347` |
| 2 | `PrevSection` | `Settings` | `prev_section` | `["h", "[", "left"]` | `previous section` | row | `settings/mod.rs:351` |
| 3 | `AgentsProbe` | `SettingsAgents` | `probe` | `["r"]` | `probe` | row | `agents.rs:2479-2498` |
| 4 | `AgentsInstall` | `SettingsAgents` | `install` | `["i"]` | `install` | row | `agents.rs:2428` |
| 5 | `AgentsAuthenticate` | `SettingsAgents` | `authenticate` | `["a"]` | `authenticate` | row | `agents.rs:2432` |
| 6 | `AgentsSwitchBox` | `SettingsAgents` | `switch_box` | `["t"]` | `this box` | row | `agents.rs:2407` |
| 7 | `AgentsEditPaths` | `SettingsAgents` | `edit_paths` | `["m"]` | `paths` | row | `agents.rs:2414` |
| 8 | `AgentsOpenLink` | `SettingsAgents` | `open_link` | `["o"]` | `open link` | row | `agents.rs:2438` |
| 9 | `AgentsPasteRedirect` | `SettingsAgents` | `paste_redirect` | `["p"]` | `paste redirect` | row | `agents.rs:2444` |
| 10 | `AgentsCancel` | `SettingsAgents` | `cancel` | `["x"]` | `cancel` | row | `agents.rs:2454`, `:2468` |
| 11 | `AgentsChoose` | `SettingsAgents` | `choose` | `["enter"]` | `select` | row | `agents.rs:640` |
| 12 | `HierarchyNewWorkspace` | `SettingsHierarchy` | `new_workspace` | `["N"]` | `new workspace` | row | `hierarchy.rs:1220` |
| 13 | `HierarchyPrimary` | `SettingsHierarchy` | `primary` | `["p"]` | `make primary` | row | `hierarchy.rs:1250` |
| 14 | `HierarchyChoosePath` | `SettingsHierarchy` | `choose_path` | `["b"]` | `choose path` | row | `hierarchy.rs:1258` |
| 15 | `HierarchyInfer` | `SettingsHierarchy` | `infer` | `["i"]` | `infer paths` | row | `hierarchy.rs:1268` |
| 16 | `KindsNewGraph` | `SettingsKinds` | `new_graph` | `["N"]` | `new graph` | row | `kinds.rs:1460` |
| 17 | `KindsGraph` | `SettingsKinds` | `graph` | `["g"]` | `edit graph` | row | `kinds.rs:1476` (D2: not `list.top`) |
| 18 | `ConnectionRebuild` | `SettingsConnection` | `rebuild` | `["R"]` | `rebuild cache` | row | `connection.rs:802` |
| 19 | `ConnectionActivate` | `SettingsConnection` | `activate` | `["enter"]` | `run row` | row | `connection.rs:807` (Rebuild row only) |
| 20 | `BoxesEditTags` | `SettingsBoxes` | `edit_tags` | `["t"]` | `edit tags` | row | `boxes.rs:703` |
| 21 | `BoxesEditQuirks` | `SettingsBoxes` | `edit_quirks` | `["e"]` | `edit quirks` | row | `boxes.rs:707` |
| 22 | `BoxesExecutor` | `SettingsBoxes` | `executor` | `["w"]` | `executor` | row | `boxes.rs:712-721` (STATE_GUARDED with `workspaces`) |
| 23 | `BoxesProbe` | `SettingsBoxes` | `probe` | `["p"]` | `probe` | row | `boxes.rs:727` |
| 24 | `BoxesEditSpec` | `SettingsBoxes` | `edit_spec` | `["s"]` | `edit probe spec` | row | `boxes.rs:723` |
| 25 | `PersonasBody` | `SettingsPersonas` | `body` | `["b"]` | `edit body` | row | `personas.rs:444` `'b'` |
| 26 | `PersonasRules` | `SettingsPersonas` | `rules` | `["r"]` | `edit rules` | row | `personas.rs:444` `'r'` |
| 27 | `PersonasImport` | `SettingsPersonas` | `import` | `["I"]` | `import` | row | `personas.rs:444` `'I'` |
| 28 | `SecretsCheck` | `SettingsSecrets` | `check` | `["t"]` | `check` | row | `secrets.rs:1416` |
| 29 | `ConceptsDecisions` | `Concepts` | `decisions` | `["ctrl-d"]` | `decisions only` | cap | `concepts_search.rs:295` |
| 30 | `ConceptsProject` | `Concepts` | `project` | `["ctrl-p"]` | `cycle project scope` | cap | `:299` |
| 31 | `ConceptsReindex` | `Concepts` | `reindex` | `["ctrl-r"]` | `re-index scope` | cap | `:303` |
| 32 | `ConceptsUp` | `Concepts` | `up` | `["up"]` | `previous hit` | cap | `:315` |
| 33 | `ConceptsDown` | `Concepts` | `down` | `["down"]` | `next hit` | cap | `:319` |
| 34 | `SwitcherSwitch` | `Switcher` | `switch` | `["enter"]` | `switch workspace` | row | `workspace_switcher.rs:171` |
| 35 | `WaitingOpen` | `Waiting` | `open` | `["enter"]` | `open step` | row | `waiting_list.rs:284` |

Comment fixes in existing rows while T1 is there (L-C §3.1): `list.down` cites `hierarchy.rs:1212`
(the arm), `common.reload` `hierarchy.rs:1289`, `common.delete` `hierarchy.rs:1279`,
`common.dismiss` `hierarchy.rs:1295`; `form.next_field`/`prev_field`, `confirm.yes`/`no` and
`common.back` say their extra view chords are `VIEW_DEFAULTS` rows; `pane.next_subtab`/`prev_subtab`
say Settings cycles through `settings.next_section`/`prev_section` now.

### 4.3 `VIEW_DEFAULTS` (extra chords only, PA-1; 14 rows)

Order: `Context::ALL`, then `Act` order. The derived row in force with defaults is given for
reference; the first chord is the hint label.

| Context | Act | extra | derived with defaults | mirrors |
|---|---|---|---|---|
| `SettingsAgents` | `FormNextField` | `["down"]` | `tab, down` | `agents.rs:2166` |
| `SettingsAgents` | `FormPrevField` | `["up"]` | `backtab, up` | `agents.rs:2170` |
| `SettingsHierarchy` | `FormNextField` | `["down"]` | `tab, down` | `hierarchy.rs:866` |
| `SettingsHierarchy` | `FormPrevField` | `["up"]` | `backtab, up` | `hierarchy.rs:870` |
| `SettingsKinds` | `FormNextField` | `["down"]` | `tab, down` | `kinds.rs:773` |
| `SettingsKinds` | `FormPrevField` | `["up"]` | `backtab, up` | `kinds.rs:777` |
| `SettingsPersonas` | `FormNextField` | `["down"]` | `tab, down` | `personas.rs:1576` |
| `SettingsPersonas` | `FormPrevField` | `["up"]` | `backtab, up` | `personas.rs:1580` |
| `SettingsPersonas` | `Back` | `["enter"]` | `esc, enter` | `personas.rs:841` |
| `SettingsSecrets` | `FormNextField` | `["down"]` | `tab, down` | `secrets.rs:674` |
| `SettingsSecrets` | `FormPrevField` | `["up"]` | `backtab, up` | `secrets.rs:675` |
| `SettingsQueue` | `Edit` | `["enter"]` | `e, enter` | `queue.rs:617` |
| `Migration` | `ConfirmYes` | `["Y"]` | `y, Y` | `migration_prompt.rs:92` |
| `Migration` | `ConfirmNo` | `["N"]` | `n, esc, N` | `migration_prompt.rs:97` (`esc`: `SHADOWING`) |

Not in the table, deliberately: agents/hierarchy list `Down`/`Up` (already `list.*` defaults,
D12), the single-field editors (connection, prompt, queue, qdrant, boxes tags: `Tab`/`Down` stay a
consumed no-op).

### 4.4 Allow-lists

`STATE_GUARDED = [(Back, Dismiss), (BoxesExecutor, Workspaces)]`;
`SHADOWING = [(ConfirmNo, OverlayClose)]` (§3.3). No other pair is demanded by §5's stacks
(checked chord by chord against every lane's collision table; the only cross-layer shares with
defaults are boxes' `w` and migration's `esc`).

---

## 5. `keys/views.rs`: every stack

`//! Every converted view mode's context stack (MOD-67 D3): views import them, never declare one.`
Each is `pub const NAME: Stack<'static> = Stack::new(&[…]);` (a `const`, not a `static`: `DECLARED`
is a `static` that copies them by value) with a `///` doc naming the mode(s) that return it.
Layer order is fixed for every stack: **view → settings → confirm → form → common → list →
overlay → global** (only the layers present). Notation: `view(X, own)` = `Layer::view`,
`∩{…}` = `Layer::only`, `all` = `Layer::all`, `modal` = `Layer::modal(Global)`,
`help·modal` = `Layer::only(Global, &[Help]).with_modal_filter()`, `help` =
`Layer::only(Global, &[Help])`. The "inherits" column is what PA-3 adds to the view layer
(overridable per view; derived rows live there).

| # | Constant | Layers | View layer inherits | `DECLARED` phrase | Returned by (lane) |
|---|---|---|---|---|---|
| 1 | `SETTINGS_TAB` | `all(Settings)`, `all(Global)` | — | `in Settings` | `SettingsTab` for a section with no stack (T1) |
| 2 | `CAPTURE` | `modal` | — | `while a field captures keys` | agents paste (L-A); qdrant URL and key editors (L-A); connection editor, prompt editor, queue editor, boxes tags (L-B); hierarchy delete Typed and InFlight, hierarchy picker (L-C); personas import path, secrets URL (L-D) |
| 3 | `AGENTS_BROWSE` | `view(SettingsAgents, [AgentsProbe, AgentsInstall, AgentsAuthenticate, AgentsSwitchBox, AgentsEditPaths, AgentsOpenLink, AgentsPasteRedirect, AgentsCancel])`, `all(Settings)`, `common∩{New, Edit, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, New, Edit, Dismiss, ListDown, ListUp | `in Settings > Agents` | L-A |
| 4 | `AGENTS_CONSENT` | `view(SettingsAgents, [])`, `all(Settings)`, `all(Confirm)`, `all(Global)` | NextSection, PrevSection, ConfirmYes, ConfirmNo | `in the Agents install question` | L-A |
| 5 | `AGENTS_CHOOSER` | `view(SettingsAgents, [AgentsChoose, AgentsProbe, AgentsInstall])`, `all(Settings)`, `confirm∩{ConfirmNo}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, ConfirmNo, ListDown, ListUp | `in the Agents login chooser` | L-A |
| 6 | `AGENTS_FORM` | `view(SettingsAgents, [])`, `form∩{FormNextField, FormPrevField}`, `modal` | FormNextField, FormPrevField | `in the Agents form` | L-A (create/edit and paths forms) |
| 7 | `HIERARCHY_BROWSE` | `view(SettingsHierarchy, [HierarchyNewWorkspace, HierarchyPrimary, HierarchyChoosePath, HierarchyInfer])`, `all(Settings)`, `common∩{Edit, New, Delete, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, New, Delete, Reload, Dismiss, ListDown, ListUp | `in Settings > Hierarchy` | L-C |
| 8 | `HIERARCHY_EDITOR` | `view(SettingsHierarchy, [])`, `form∩{FormNextField, FormPrevField}`, `modal` | FormNextField, FormPrevField | `in the Hierarchy editor` | L-C |
| 9 | `HIERARCHY_DELETE_COUNTING` | `view(SettingsHierarchy, [])`, `confirm∩{ConfirmNo}`, `modal` | ConfirmNo | `while Hierarchy counts a delete` | L-C |
| 10 | `HIERARCHY_DELETE_WARN` | `view(SettingsHierarchy, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in the Hierarchy delete warning` | L-C |
| 11 | `KINDS_BROWSE` | `view(SettingsKinds, [KindsNewGraph, KindsGraph])`, `all(Settings)`, `common∩{Edit, New, Delete, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | as hierarchy browse | `in Settings > Kinds` | L-C |
| 12 | `KINDS_EDITOR` | `view(SettingsKinds, [])`, `form∩{FormNextField, FormPrevField}`, `modal` | FormNextField, FormPrevField | `in the Kinds editor` | L-C |
| 13 | `KINDS_CONFIRM` | `view(SettingsKinds, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in a Kinds question` | L-C (prefix warning; delete Asking and InFlight) |
| 14 | `PROMPT_BROWSE` | `view(SettingsPrompt, [])`, `all(Settings)`, `common∩{Edit, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, Reload, Dismiss, ListDown, ListUp | `in Settings > Prompt` | L-B |
| 15 | `CONNECTION_BROWSE` | `view(SettingsConnection, [ConnectionRebuild, ConnectionActivate])`, `all(Settings)`, `common∩{Edit, Clear, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, Clear, Reload, Dismiss, ListDown, ListUp | `in Settings > Connection` | L-B |
| 16 | `CONNECTION_CONFIRM` | `view(SettingsConnection, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in a Connection question` | L-B (clear DSN; rebuild Asking and InFlight) |
| 17 | `QDRANT_BROWSE` | `view(SettingsQdrant, [])`, `all(Settings)`, `common∩{Edit, Clear, Reload}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, Clear, Reload, ListDown, ListUp | `in Settings > Qdrant` | L-A |
| 18 | `QDRANT_CONFIRM` | `view(SettingsQdrant, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in the Qdrant clear question` | L-A |
| 19 | `BOXES_BROWSE` | `view(SettingsBoxes, [BoxesEditTags, BoxesEditQuirks, BoxesExecutor, BoxesProbe, BoxesEditSpec])`, `all(Settings)`, `common∩{Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Reload, Dismiss, ListDown, ListUp | `in Settings > Boxes` | L-B |
| 20 | `BOXES_EDITOR` | `view(SettingsBoxes, [])`, `form∩{FormSave}`, `modal` | FormSave | `in the Boxes quirks or probe spec editor` | L-B (Quirks, Spec) |
| 21 | `BOXES_EXECUTOR` | `view(SettingsBoxes, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in the Boxes executor question` | L-B |
| 22 | `PERSONAS_BROWSE` | `view(SettingsPersonas, [PersonasBody, PersonasRules, PersonasImport])`, `all(Settings)`, `common∩{New, Edit, Delete}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, New, Edit, Delete, ListDown, ListUp | `in Settings > Personas` | L-D |
| 23 | `PERSONAS_FORM` | `view(SettingsPersonas, [])`, `form∩{FormNextField, FormPrevField}`, `modal` | FormNextField, FormPrevField | `in the Personas form` | L-D |
| 24 | `PERSONAS_EDITOR` | `view(SettingsPersonas, [])`, `form∩{FormSave}`, `modal` | FormSave | `in the Personas body or rules editor` | L-D |
| 25 | `PERSONAS_DELETE` | `view(SettingsPersonas, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in the Personas delete question` | L-D (Asking and InFlight) |
| 26 | `PERSONAS_REPORT` | `view(SettingsPersonas, [])`, `common∩{Back}`, `list∩{ListDown, ListUp}`, `modal` | Back, ListDown, ListUp | `in the Personas import report` | L-D |
| 27 | `SECRETS_BROWSE` | `view(SettingsSecrets, [SecretsCheck])`, `all(Settings)`, `common∩{Edit, Clear, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, Clear, Reload, Dismiss, ListDown, ListUp | `in Settings > Secrets` | L-D |
| 28 | `SECRETS_FORM` | `view(SettingsSecrets, [])`, `form∩{FormNextField, FormPrevField}`, `modal` | FormNextField, FormPrevField | `in a Secrets form` | L-D (identity, scope) |
| 29 | `SECRETS_CONFIRM` | `view(SettingsSecrets, [])`, `all(Confirm)`, `modal` | ConfirmYes, ConfirmNo | `in a Secrets question` | L-D (three clears) |
| 30 | `QUEUE_BROWSE` | `view(SettingsQueue, [])`, `all(Settings)`, `common∩{Edit, Reload, Dismiss}`, `list∩{ListDown, ListUp}`, `all(Global)` | NextSection, PrevSection, Edit, Reload, Dismiss, ListDown, ListUp | `in Settings > Queue` | L-B |
| 31 | `CONCEPTS_QUERY` | `view(Concepts, [ConceptsDecisions, ConceptsProject, ConceptsReindex, ConceptsUp, ConceptsDown])`, `all(Overlay)`, `help·modal` | — | `in the concepts search` | L-E |
| 32 | `SWITCHER` | `view(Switcher, [SwitcherSwitch])`, `list∩{ListDown, ListUp}`, `all(Overlay)`, `help` | ListDown, ListUp | `in the workspace switcher` | L-E |
| 33 | `MIGRATION` | `view(Migration, [])`, `all(Confirm)`, `all(Overlay)`, `help` | ConfirmYes, ConfirmNo | `in the migration prompt` | L-E |
| 34 | `WAITING_LIST` | `view(Waiting, [WaitingOpen])`, `list∩{ListDown, ListUp}`, `all(Overlay)`, `help` | ListDown, ListUp | `in the waiting list` | L-E |

`DECLARED` = `BASE`, `OVERLAY`, then rows 1-34 in this order (36). Names differ from the lane
inventories where §2 merged or renamed: `QDRANT_EDIT`, `AGENTS_PASTE`, `SETTINGS_FIELD`,
`HIERARCHY_DELETE_TYPED`, `HIERARCHY_PICKING`, `PERSONAS_IMPORT`, `SECRETS_URL` → `CAPTURE`;
`SETTINGS_CONFIRM` → per-section confirms; `BOXES_TEXT_AREA` → `BOXES_EDITOR`;
`KINDS_CONFIRM_PREFIX` + `KINDS_DELETE` → `KINDS_CONFIRM`; `SWITCHER_BROWSE` → `SWITCHER`;
`MIGRATION_CONFIRM` → `MIGRATION`.

Mode → stack priority inside a view is the view's job (`fn stack(&self) -> Stack<'static>`, the one
place `key_stack`, `on_key` and the hint read). Agents: Form/Paths → `AGENTS_FORM`, install
`Pending` → `AGENTS_CONSENT`, auth `Choosing` → `AGENTS_CHOOSER`, paste open → `CAPTURE`, else
`AGENTS_BROWSE` (L-A §2.1's order F, C, H, P, B).

Resolution with defaults that T1 pins in `views.rs` tests (defect fixes by construction):
`HIERARCHY_BROWSE`/`KINDS_BROWSE` `ctrl-d` → `[]`; `HIERARCHY_BROWSE` `down` → `[ListDown]`,
`tab` → `[NextTab]` (no form leak); `AGENTS_FORM` `down` → `[FormNextField]`, `tab` →
`[FormNextField]`, `q` → `[]`, `f1` → `[Help]`; `CAPTURE` `tab` → `[]` and `passes(tab)` false
(defect 2); `SETTINGS_TAB` `ctrl-l` → `[]`, `l` → `[NextSection]`; `QUEUE_BROWSE` `enter` →
`[Edit]`; `BOXES_BROWSE` `w` → `[BoxesExecutor, Workspaces]`; `KINDS_BROWSE` `g` → `[KindsGraph]`;
`CONCEPTS_QUERY` `?` → `[]`, `f1` → `[Help]`, `esc` → `[OverlayClose]`, `ctrl-D` event →
`[ConceptsDecisions]` (PA-6); `MIGRATION` `Y` → `[ConfirmYes]`, `esc` → `[ConfirmNo,
OverlayClose]`; `SWITCHER` `enter` → `[SwitcherSwitch]`, `2` → `[]`.

---

## 6. Dispatch skeletons (every lane copies these)

Common to all: `let chord = KeyChord::from_event(key);` once; match on `Act`, never on `key.code`
(except inside a widget). In a `for act in ctx.keys().actions(STACK, chord)` loop, an arm that
accepts **returns**; `_ => continue` (browse) / `_ => break` (modal) moves past a global act or a
declined one. Delete every `KeyModifiers::CONTROL` guard listed in the lane inventories: the
resolver (chord equality includes modifiers) and `Stack::passes` replace them.

### 6.1 `key_stack` pattern (every converted view)

```rust
impl FooSection {
    /// The stack of the current mode: the only place a mode maps to its keys (key_stack, on_key
    /// and the hint all read it).
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::FOO_BROWSE,
            Mode::Editing(_) => views::CAPTURE,
            Mode::ConfirmClear | Mode::ConfirmRebuild { .. } => views::FOO_CONFIRM,
        }
    }
}
impl SettingsSection for FooSection {
    fn key_stack(&self) -> Option<Stack<'static>> { Some(self.stack()) }
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode { Mode::Browse => self.on_browse_key(key, ctx), /* … */ }
    }
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let hint = ctx.keys().hint(self.stack(), self.hint_spec());   // + today's suffixes
        /* … */
    }
}
```
Overlays implement `Overlay::key_stack` the same way (each has one mode: `Some(views::X)`).
`hint_text(&self)` helpers gain `keys: &Keys`; in-file tests pass `Keys::compiled()`.

### 6.2 (c) Browse mode (non-capturing; unfiltered global layer)

```rust
fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    let chord = KeyChord::from_event(key);
    for act in ctx.keys().actions(views::CONNECTION_BROWSE, chord) {
        match act {
            Act::Edit => { self.open_editor(ctx); return Handled::Consumed; }
            // State-declined: off the Rebuild row `Enter` is not ours; the next candidate.
            Act::ConnectionActivate if self.row() == Row::Rebuild => {
                self.rebuild(ctx);
                return Handled::Consumed;
            }
            Act::Dismiss if self.notice.is_some() => {
                self.notice = None;
                return Handled::Consumed;
            }
            _ => continue, // a global act, or one this state declines
        }
    }
    Handled::Pass // the shell resolves the same stack and applies the first global act
}
```
Never handle `NextSection`/`PrevSection` here: `SettingsTab` takes them before delegating.

### 6.3 (a) Capturing mode with a text widget (`TextField`, `TextArea`): widget first

```rust
fn on_form_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    let stack = views::AGENTS_FORM;
    match self.focused_field_mut().on_key(key) {            // TextArea: on_key(key, page)
        FieldOutcome::Consumed => return Handled::Consumed,
        FieldOutcome::Submit => return self.submit(ctx),   // Enter (TextArea: its ctrl-s, D13)
        FieldOutcome::Cancel => { self.close(); return Handled::Consumed; } // Esc
        FieldOutcome::Pass => {}
    }
    let chord = KeyChord::from_event(key);
    for act in ctx.keys().actions(stack, chord) {
        match act {
            Act::FormNextField => { self.focus_next(); return Handled::Consumed; }
            Act::FormPrevField => { self.focus_prev(); return Handled::Consumed; }
            // Act::FormSave => { self.save(ctx); return Handled::Consumed; } // TextArea stacks
            _ => break, // a global act: the pass rule below returns Pass for it
        }
    }
    if stack.passes(chord) { Handled::Pass } else { Handled::Consumed }
}
```
`CAPTURE` modes have no loop body: after the widget, `if views::CAPTURE.passes(chord) { Pass }
else { Consumed }`. This is the defect-2 fix: Qdrant's `Tab` is `Consumed`. `ctrl-c` and `F1`
return `Pass` (section-level pins green; the shell quits / opens help).

### 6.4 (b) Confirming or non-text modal mode (questions, personas report): resolve, pass rule, swallow

```rust
fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    let stack = views::CONNECTION_CONFIRM;
    let chord = KeyChord::from_event(key);
    for act in ctx.keys().actions(stack, chord) {
        match act {
            Act::ConfirmYes => { self.answer_yes(ctx); return Handled::Consumed; }
            Act::ConfirmNo => { self.mode = Mode::Browse; return Handled::Consumed; }
            _ => break,
        }
    }
    if stack.passes(chord) { Handled::Pass } else { Handled::Consumed }
}
```
An in-flight stage that ignores answers still matches `ConfirmYes | ConfirmNo => return
Handled::Consumed` (today's swallow).

**(b′) Opaque widget (hierarchy's `PathPicker`)**: the same loop (no view act in `CAPTURE`), then
`if stack.passes(chord) { return Handled::Pass; }`, **then** `self.picker.on_key(key, …)` and
`Handled::Consumed`. The picker never sees a CONTROL/ALT/F-key chord.

### 6.5 Agents consent and chooser (non-capturing, partly modal)

Browse skeleton over `AGENTS_CONSENT`/`AGENTS_CHOOSER`, then before the final `Pass`:
```rust
let browse = ctx.keys().actions(views::AGENTS_BROWSE, chord);
if browse.iter().any(|act| CONSENT_SWALLOWS.contains(act)) { return Handled::Consumed; }
Handled::Pass
```

### 6.6 Overlays

- List overlays (`SWITCHER`, `WAITING_LIST`) and `MIGRATION`: browse skeleton; own and shared
  acts return `Consumed`; everything else `Handled::Pass` (the shell's overlay step applies
  `overlay.close`/`help`, then the modal swallow). `MIGRATION`: `ConfirmYes` applies and closes;
  `ConfirmNo` (now also `Esc`, first candidate) emits `Overlay(Close)` itself: same effect as
  today's `Pass` → `overlay.close`.
- `ConceptsSearch`: skeleton (a) with the field first; `FieldOutcome::Cancel` → `Handled::Pass`
  (today's, `concepts_search.rs:334-335`: the shell's `overlay.close` closes); after the loop
  always `Handled::Pass` (the shell resolves `f1`; the modal overlay swallows the rest).

### 6.7 What the shell does with a `Pass` (T1, for reference)

Overlay up: `apply_keys(top.key_stack() or OVERLAY)`, then the modal swallow. Tab: legacy
`keymap` rows (Backlog only), then `apply_keys(tab.key_stack() or BASE)`. In a modal Settings mode
the tab's stack is the section's modal stack, so only `F1`, `Ctrl+f`, `Ctrl+w` (and quit's
`ctrl-c`, step 0) do anything; `alt-x` does nothing.

---

## 7. T1 tests (TDD order: write red, then code) and the existing tests T1 changes

Commit order (each commit compiles, clippy both ways, its tests green): T1a chord → T1b catalogue
+ contexts → T1c stack/views → T1d mod/derive → T1e hint → T1f load/print/validate → T1g shell +
testkit → T1h integration tests. Gate: `cargo test -p htui --lib keys`, `cargo test -p htui
--features testkit --test keys --test keys_file -- --test-threads=1`, `cargo test -p htui
--all-features --test settings -- --test-threads=1` (the cycling change), `cargo insta test -p htui
--all-features --check` (**no snapshot may change**).

### 7.1 New unit tests

| Id | File | Test | Asserts |
|---|---|---|---|
| T-K1 | `chord.rs` | `ctrl_with_a_capital_is_the_lower_case_chord` | `from_event(Char('D'), CONTROL)` == `parse("ctrl-d")`; `ctrl-alt-X` → `ctrl-alt-x`; `alt-X` unchanged; `parse_strict("ctrl-D")` still `CtrlCapital` |
| T-K2 | `chord.rs` | `passes_modal_is_ctrl_alt_or_a_function_key` | `ctrl-c`, `ctrl-f`, `alt-x`, `f1`, `shift-f5` pass; `q`, `?`, `tab`, `backtab`, `enter`, `esc`, `down`, `shift-up` do not |
| T-C1 | `catalogue.rs` | `every_act_is_listed_and_has_a_row` (ALL/position +35), `every_act_has_exactly_one_row` (76) | lockstep lists |
| T-C2 | `catalogue.rs` | `every_context_is_listed_once_in_all` | `Context::ALL` has 22, no duplicate, with a `position` match without wildcard (like `Act`) |
| T-C3 | `catalogue.rs` | `context_tables_and_headings` (22 rows, §4.1) | table and heading |
| T-C4 | `catalogue.rs` | `in_capture_acts_are_the_form_overlay_close_and_concepts` (rename) | + the five `Concepts*` |
| T-C5 | `catalogue.rs` | `catalogue_blocks_follow_context_all` | the contexts of `CATALOGUE` rows appear in `Context::ALL` order |
| T-C6 | `views.rs` | `every_view_default_is_an_extra_chord_some_stack_offers` | each `VIEW_DEFAULTS` entry: context `is_view`, act's context `is_shared`, extras parse strictly, none is already a catalogue default of the act, and some `DECLARED` stack `view_admits(context, act)` |
| T-C7 | `views.rs` | `no_view_name_hides_a_shared_name_it_inherits` | for every view context, no own row's name equals the name of a shared act any of its stacks inherit (concepts' `up`/`down` pass: no list layer) |
| T-C8 | `catalogue.rs` | `shared_names_are_unique_across_shared_contexts` | D10 resolution is unambiguous |
| T-S1 | `stack.rs` | `a_modal_layer_admits_only_ctrl_alt_and_function_keys` | `actions(Stack::new(&[Layer::modal(Global)]), q/tab/?)` = `[]`, `f1` = `[Help]`, `ctrl-f` = `[Find]` |
| T-S2 | `stack.rs` | `only_and_the_modal_filter_combine` | `[only(Global,[Help]).with_modal_filter()]`: `?` → `[]`, `f1` → `[Help]`, `ctrl-f` → `[]` |
| T-S3 | `stack.rs` | `a_view_layer_inherits_only_shared_acts_offered_below` | with `with_chords(SettingsBoxes, Reload, ["f5"])`: `[view(SettingsBoxes,[]), common∩{Reload}]` → `f5` = `[Reload]`, `r` = `[]`; with `common∩{Dismiss}` instead, `f5` = `[]`, `r` = `[]` (the row is not admitted, `Reload` not offered); a `Quit` row in the view context is never admitted |
| T-S4 | `stack.rs` | `passes_follows_the_global_layer` | `CAPTURE.passes(ctrl-c)` true, `(tab)` false; `AGENTS_BROWSE.passes(q)` true; a stack with no global layer: false |
| T-S5 | `stack.rs` | `declared_starts_with_base_and_overlay_and_holds_every_view_stack` (replaces `declared_walks_every_screen_then_over_an_overlay`) | first two entries, `len() == 36`, phrases unique |
| T-V1 | `views.rs` | `the_view_stacks_resolve_as_designed` | every resolution listed at the end of §5 |
| T-V2 | `views.rs` | `every_view_layer_comes_first_and_every_modal_stack_ends_modal` | rows 3-34: layer 0 is a view layer of an `is_view` context; the last layer is `Global`; stacks whose global layer is modal have no `Settings` layer |
| T-M1 | `mod.rs` | `the_compiled_view_defaults_are_derived` | `chords(SettingsAgents, FormNextField)` = `[tab, down]`; `(Migration, ConfirmNo)` = `[n, esc, N]`; `line` `None` |
| T-M2 | `mod.rs` | `the_compiled_keys_are_the_catalogue_and_never_ctrl_c` extended | derived rows never bind `ctrl-c` either |
| T-H1 | `hint.rs` | `an_empty_text_renders_the_labels_alone` | `Pair(ListDown, ListUp, "")` + `One(Edit, "edit")` → `j/k · e edit` |
| T-H2 | `hint.rs` | `all_lists_every_admitted_chord_and_text_is_fixed` | `All(ConfirmNo, "cancel")` → `n/Esc cancel`; `Text("Enter store")` verbatim; through a modal stack `All` drops filtered chords |
| T-H3 | `hint.rs` | `the_status_line_follows_the_stacks_global_layer` | the four rows of §3.6's table, bare and `all` |
| T-H4 | `hint.rs` | `the_help_lines_list_an_act_under_its_narrowest_layer` | `QUEUE_BROWSE` → `Queue: e/Enter edit`, `Settings: l/]/Right next section · h/[/Left previous section`, `Common: r reload · Esc dismiss`, `List: j/Down down · k/Up up`, `Global: …` (no `Common: e edit`); `SWITCHER` → `Workspaces: Enter switch workspace`, `List: …`, `Overlay: Esc close`, `Global: Ctrl+c quit · ?/F1 help`; `MIGRATION` → `Schema: y/Y yes · n/Esc/N no` and no `Confirm` line; `AGENTS_FORM` → `Agents: Tab/Down next field · Shift+Tab/Up previous field`, no `Form` line, `Global: Ctrl+c quit · F1 help` (bare) |
| T-H5 | `hint.rs` | `the_closer_follows_the_global_layer` | `BASE` `?/F1 closes this box`; `CONCEPTS_QUERY`, `CAPTURE` `F1 closes this box`; help unbound → `None` |
| T-L1 | `load.rs` | `a_view_table_overrides_a_shared_verb_it_offers` | `[settings.boxes] reload = "f5"` loads; `chords(SettingsBoxes, Reload)` = `[f5]`, `line` `Some(2)`; `actions(BOXES_BROWSE, f5)` = `[Reload]`, `(PROMPT_BROWSE, r)` = `[Reload]` |
| T-L2 | `load.rs` | `a_view_table_refuses_what_none_of_its_modes_offer` | `[settings.boxes] edit` and `[settings.boxes] quit` and `[switcher] close` → the §3.8 messages |
| T-L3 | `load.rs` | `a_section_with_no_own_action_has_a_table` | `[settings.prompt] reload = "f5"` and `[settings.queue] edit = ["E"]` load; `[migration] yes = "a"` loads |
| T-L4 | `load.rs` | `a_shared_rebind_flows_into_the_view_defaults` | `[form] next_field = ["ctrl-n"]` → `(SettingsSecrets, FormNextField)` = `[ctrl-n, down]`, line `None`; `[settings.secrets] next_field = ["ctrl-n"]` → `[ctrl-n]`, line `Some` |
| T-L5 | `load.rs` | `an_unknown_table_lists_the_tables` updated | `TABLES` = §3.8's list; the `[settings.boxes]` case becomes `[settings.nothing]` |
| T-P1 | `print.rs` | `a_view_override_and_a_derived_row_round_trip` | §3.9's two cases: print, load, equal, re-print identical; marks as stated |
| T-P2 | `print.rs` | `the_default_print…` updated | ends with `open = ["enter"]  # open step\n`; still ≤100 columns; no `(changed)` |
| T-X1 | `validate.rs` | `the_compiled_defaults_validate` | over the 36 stacks (unchanged body) |
| T-X2 | `validate.rs` | `every_allow_list_entry_is_demanded` | for each entry of `SHADOWING` and `STATE_GUARDED`, `check(compiled, list minus entry, …)` is non-empty |
| T-X3 | `validate.rs` | `a_view_rows_never_collide_across_modes` | `[settings.agents] yes = "o"` loads (consent vs browse `o`); `[settings.agents] probe = "down"` → `… is already list.down (default) in Settings > Agents` |
| T-X4 | `validate.rs` | `a_user_chord_shared_with_a_shadowed_act_is_refused` | `[migration] yes = ["y", "esc"]` → two errors on line 2 (`… already migration.no …`, `… already overlay.close …`, both `in the migration prompt`); `[settings.boxes] executor = ["w", "W"]` loads |

### 7.2 New integration tests (`tests/keys.rs`, `#![cfg(feature = "testkit")]`)

Test doubles defined in the file: `StackProbe` (a `SettingsSection`, `captures_input` true,
`key_stack` `Some(views::AGENTS_FORM)`, consumes printable chars, else `Pass` iff
`AGENTS_FORM.passes`) and `OverlayProbe` (an `Overlay`, modal, `key_stack`
`Some(views::CONCEPTS_QUERY)`, `on_key` → `Pass`).

| Test | Asserts |
|---|---|
| `ctrl_l_and_ctrl_h_do_not_cycle_sections` | Settings over Connection + Qdrant: `ctrl-l`, `ctrl-h`, `shift-right` leave Connection active; `l` moves to Qdrant, `h` back (D14 pin, lands in T1) |
| `a_capturing_section_shows_the_filtered_status_line_and_box` | `StackProbe` focused: status `Ctrl+c quit · F1 help` (bare); `F1` opens the box; it contains `Agents: Tab/Down next field` and `F1 closes this box`, not `?/F1`; `?` is consumed by the probe, help stays shut |
| `an_overlay_with_a_stack_drives_the_status_line_and_box` | `with_overlay(OverlayProbe)`: status `Ctrl+c quit · F1 help`; `f1` opens; box has `Overlay: Esc close`, `Global: Ctrl+c quit · F1 help`, no `Backlog:` legacy line; `esc` closes the overlay |
| `with_keys_installs_the_keys` | `Harness::demo().with_keys(load_str("version = 1\n[global]\nquit = \"x\"\n"))`: `q` does not quit, `x` does, status starts `x quit` |
| `the_status_line_is_todays_in_both_harnesses` | unchanged (guards PA-8: unconverted views keep today's line) |

### 7.3 Existing tests T1 changes (and only these)

- `catalogue.rs`: `ALL`, `position`, `every_act_has_exactly_one_row` (76),
  `context_tables_and_headings`, `only_the_form_and_overlay_close_are_in_capture` (renamed, T-C4).
- `stack.rs`: `declared_walks_every_screen_then_over_an_overlay` → T-S5.
- `hint.rs`: every `status_line(f)` → `status_line(Stack::BASE, f)`, `help_closer()` →
  `help_closer(Stack::BASE)`; `an_unbound_action_drops_out_of_the_status_line_and_a_hint` →
  `starts_with("Ctrl+c quit · Tab next tab")` (PA-9).
- `load.rs`: `TABLES`, `an_unknown_table_lists_the_tables` (`[settings.boxes]` → `[settings.nothing]`),
  `every_error_is_reported_and_sorted_by_line` (line 8's table list).
- `print.rs`: the `ends_with("dismiss = [\"esc\"]  # dismiss\n")` assertion → T-P2's tail.
- `tests/keys_file.rs` `bad_fixtures` `errors` line 8 (the table list). The fixture files do not
  change. `tests/keys.rs` keeps every existing test unchanged.

---

## 8. Per-lane delta (what each lane does differently from its inventory)

Every lane: stacks are §5's names; never write a `Layer::only` view set (own acts are in
`views.rs`); text widgets first (§6.3); confirm/report modes §6.4; the pass rule is
`STACK.passes(chord)`; VIEW_DEFAULTS rows are extras (hint label = first chord, unchanged with
defaults); hints use `Hint::{One, Pair, All, Text}` with an empty text for a bare `j/k`; the
rebinding test builds keys with `htui::keys::load_str(…).expect(…)` and `Harness::with_keys` or
`SectionBench::with_keys`. Before calling a snapshot diff "expected", check it against the lists
below; anything else is a defect (D9).

### L-A (agents, qdrant)
- Paste field and both Qdrant editors use `views::CAPTURE` (not `AGENTS_PASTE`/`QDRANT_EDIT`).
- Consent/chooser swallows per §6.5 with `CONSENT_SWALLOWS`/`CHOOSER_SWALLOWS` (L-A Q2).
- `refuse_during_login` takes the `Act` (`AgentsProbe`/`AgentsInstall`), not `key.code`.
- Hint `HINT_CHOOSING` → `j/k choose · Enter select · n/Esc cancel`; `HINT_CONFIRM` (qdrant) →
  `y confirm · n/Esc cancel`; the rest identical.
- Assertions: `tests/settings.rs:2229` → `n/Esc cancel`; `qdrant.rs:577-591` signature only.
- Snapshots: **none** change (`settings__*` ×10, `probe__agents_probed_missing`).
- Pins: Qdrant `Tab`/`BackTab` keep the tab (App) and `bench.key("tab") == Consumed`; agents list
  and chooser `Down`/`Up`; `ctrl-y` in consent, `ctrl-r` in agents browse, `ctrl-e` in Qdrant
  browse inert; `F1` from the agents form opens help; rebinding `[settings.agents] probe = "P"`.

### L-B (connection, boxes, prompt, queue)
- Editors and boxes tags → `CAPTURE`; confirmations → `CONNECTION_CONFIRM` / `BOXES_EXECUTOR`
  (not a shared `SETTINGS_CONFIRM`); quirks/spec → `BOXES_EDITOR`.
- Boxes quirks/spec: `TextArea` first (its `ctrl-s` is `Submit`), then `FormSave` from the
  resolver for a rebound chord, both call `submit`.
- Queue's `Enter` comes from the derived `(SettingsQueue, Edit)` row; `QUEUE_BROWSE`'s view layer
  inherits `Edit` automatically.
- Hint drift: `n / Esc` → `n/Esc` (connection), `n/esc` → `n/Esc`, `ctrl-s` → `Ctrl+s` (boxes);
  prompt/queue `j/k · …` unchanged (empty text).
- Assertions: `tests/box_settings.rs:1220`, `:2090`, `:1474`, `:2047` (L-B §2.7).
- Snapshots: `connection__confirm`, `box_settings__executor_confirm`, `box_settings__quirks_editor`,
  `box_settings__spec_editor` (row 34 hint each). Nothing else.
- Pins: the D14 narrower-override test (`[settings.boxes] reload = "f5"`, `tests/box_settings.rs`,
  as L-B §2.8, `F5 reload` under the hint); rebinding `[settings.connection] rebuild = "X"`; the
  optional `ctrl-e`/`alt-y`/`ctrl-s`-in-browse pins.

### L-C (hierarchy, kinds)
- Delete Typed/InFlight and the picker → `CAPTURE`; kinds prefix warning and delete →
  `KINDS_CONFIRM`.
- Picker: skeleton (b′) — resolve, `CAPTURE.passes`, then `PathPicker::on_key`.
- `HINT_COUNTING` → `counting rows… · n/Esc stop` (accepted); `delete_question` built from the hint.
- Snapshots: `hierarchy__delete_typed`, `hierarchy__delete_warn`, `hierarchy__editor_repo`, row 34
  only: `Ctrl+c quit · F1 help`, or `Ctrl+c quit · F1 help · Ctrl+f find · Ctrl+w waiting` if that
  harness ran `register_all` (check which; either is correct, anything else is not). No kinds
  snapshot changes.
- Pins: `ctrl-d` in hierarchy and kinds browse deletes nothing (D14); `Down`/`Up` move the
  hierarchy list; both editors move focus on `Down`/`Up`; rebinding `[settings.hierarchy] infer =
  "I"` (and kinds `graph = "G"` if a second is wanted).

### L-D (personas, secrets)
- Import path and secrets URL → `CAPTURE`; delete Asking/InFlight → `PERSONAS_DELETE`.
- Personas body/rules: `TextArea` first, then `FormSave`.
- `tests/personas.rs:13-16` imports of `HINT_*` become test-local literals (L-D §1.6).
- Snapshots: `personas__body`, `personas__rules` (hint `Ctrl+S` → `Ctrl+s`). Nothing else.
- Pins: secrets `ctrl-t`/`ctrl-e`/`alt-c` in browse inert; `alt-y` at the personas and secrets
  questions inert; `esc` with no notice passes; `F1` from the body editor `Pass`; rebinding
  `[settings.personas] import = ["i"]` and/or `[settings.secrets] check = ["T"]`; the additive
  check `[form] next_field = ["ctrl-n"]`: `ctrl-n` and `Down` move the identity focus, `Tab` does
  not.

### L-E (four overlays)
- Stack names `SWITCHER`, `MIGRATION` (§5). Migration has no own act: `ConfirmYes`/`ConfirmNo`
  through the derived `[migration]` rows; `Esc` resolves to `ConfirmNo` first (the view closes).
- Concepts: field first (§6.6). `Shift+Up`/`Shift+Down` stop moving the cursor (accepted).
- Hints: migration `Pair(ConfirmNo, OverlayClose, "stay offline")` → `y apply · n/Esc stay
  offline`; concepts per L-E Q5 (maintainer); switcher and waiting unchanged.
- Owns `src/snapshots/htui__testkit__tests__shell_empty.snap` (status row → `Ctrl+c quit · ? help`).
- Snapshots: the 11 of L-E §11 (5 concepts rows 27+34, 2 waiting row 34, `shell__switcher_open`,
  `shell__switcher_empty`, `shell__migration_prompt` rows 21+34, `shell_empty` row 34).
- Pins: L-E §12 (ctrl-y/alt-n at the migration prompt, `Y`/`N`, `ctrl-j` in the switcher, status
  and box over the switcher and concepts, rebinding `[concepts] reindex = "f5"`); the expected `?`
  box over migration is `Schema: y/Y yes · n/Esc/N no` (not `n/N`).

### T7
- Reachability test: every `DECLARED` stack except `BASE` and `OVERLAY` is named in some file under
  `src/ui/` (an `include_str!` scan of the 14 view files plus `settings/mod.rs`, by constant name).
- README "Changing keys": the D11 limit narrowed to Backlog/Chat/Skills/Requirements; D13's
  `TextArea` note; additive view defaults ("`[form] next_field` also changes the Settings forms,
  which keep `Down`; write `[settings.<section>] next_field` to drop it"); a chord bound onto a
  text field's own key does nothing while that field captures.
- HANDOFF: the `picker` context follow-up (L-C Q3) and the prose spellings left for M6
  (`Ctrl+S retries`, `NO_MATCHES`' `Ctrl+R`).

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Inheritance makes admission stack-dependent; a caller that uses `Layer::admits` directly misses inherited acts | Medium | All such callers are in `keys/` (T1). `Layer::admits` is documented "own set only"; T-S3 and T-H4 exercise inherited acts through `actions`, `label`, `help_lines`, the validator and the loader |
| Additive VIEW_DEFAULTS surprise: unbinding `[form] next_field = []` leaves `Down` in five Settings forms | Low | Documented (README, T7); `[settings.<section>] next_field = []` unbinds both |
| PA-6's fold changes harness input: a test sending `"ctrl-S"` now sends `ctrl-s` | Low | Both `TextArea` and the concepts arms already treated them alike; T1 runs the full suite once with `--test-threads=1` before handing off |
| Skipping the context pass for view contexts hides a real collision | Low | Every view context's rows are only ever offered through declared stacks, all walked by the stack pass (T-X3) |
| The status line now differs between a converted and an unconverted capturing view (mixed during the lane phase) | Expected | Only the three `hierarchy__*` App snapshots show a capturing Settings status row; lanes re-baseline their own |
| A lane needs an act, stack or allow-list entry §4/§5 lacks | Medium | Plan rule: back to the main thread, T1 amended on the base, lanes merge it; T-X1 and T-X2 fail loudly on a stack/allow-list mismatch |
| Concepts hint wording waits on the maintainer | Medium | Provisional text in §2 (L-E Q5); a one-constant change either way, five snapshots |
| `DECLARED` grows to 36 stacks: validation cost at startup | Low | ~76 rows × 36 stacks × a few chords; microseconds |
| `tests/settings.rs` (all ten sections) and `tests/connection.rs` see other lanes' hint rows | Medium | No assertion there reads another lane's hint (lanes verified); T7 merges one lane at a time and runs the full suite |

