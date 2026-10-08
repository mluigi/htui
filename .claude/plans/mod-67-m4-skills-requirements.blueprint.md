# Blueprint: MOD-67 milestone 4, "Skills and Requirements dispatch through context stacks"

**Status**: built (2026-10-08). PA-1 to PA-4 (§1) approved by the maintainer; R1 removed
`SKILLS_TAB` (§4.1, plan "Review gate"); the rest of this file builds the confirmed plan as written. D1-D11 stand except where §1 amends them.

**Plan**: `.claude/plans/mod-67-m4-skills-requirements.plan.md` (confirmed 2026-10-08, commit
`329a9440`).
**Lane files**: `.claude/plans/mod-67-m4-blueprint/L-{A,B,C}.md` (every mode, arm, hint, test and
snapshot of each lane).
**Template**: the M3 blueprint `.claude/plans/mod-67-m3-settings-overlays.blueprint.md` (§6
skeletons, §8 per-lane deltas) and `.claude/plans/mod-57-m1-editor-in-pane.blueprint.md` §6.4.
**Verified at**: HEAD `329a9440`, branch `hr/MOD-67`. Read at that HEAD: `keys/{catalogue,views,
stack,hint,load,validate,print,chord}.rs`, `app/state.rs` (`on_key`, `apply_keys`,
`active_stack`), `ui/tabs/skills/{mod,library,templates,attach,agent_help}.rs`,
`ui/tabs/requirements/{mod,forms}.rs`, `ui/text_area.rs`, `ui/tabs/settings/{mod,boxes,personas}.rs`,
`ui/tabs/backlog/detail/mod.rs` (`Scroll`), `tests/{keys,keys_file,skills,templates,requirements,
box_settings}.rs`, `tests/fixtures/keys/*.toml`, `tests/snapshots/{skills,templates,requirements}__*`,
`README.md` "Changing keys". Line numbers are pre-edit.

**Order**: T1 (serial, primary tree) → L-A ∥ L-B ∥ L-C (worktrees `target/wt/<lane>`, branches
`mod-67-m4-<lane>`) → T-close (serial, primary tree). Lanes add no catalogue row, stack,
`VIEW_DEFAULTS`, `SHADOWING` or `STATE_GUARDED` entry: §3-§5 are complete for all three. A missing
one goes back to the main thread (plan lane rule).

**House style (M2/M3, unchanged)**: `#![warn(missing_docs)]`, inline format args, rustfmt edition
2024 width 100, clippy `-D warnings` both `--all-targets --all-features` and featureless.
Implementers commit incrementally and stage their own paths only. Gortex `read` for source; where
the hook blocks `Read`/`Edit`, an anchored scripted replace (each anchor asserted to match once).

**The one-paragraph design.** Every Skills and Requirements mode names a `pub const` stack in
`keys/views.rs`, built exactly like M3's: `Layer::view(<view context>, <own acts>)` first, then
the shared layers narrowed to what the mode offers, then the global layer (unfiltered in browse,
`MODAL` in a capturing or confirming mode). Library and Templates share one non-view shared
context, `skills` (the switch, the version cursor, base, diff, `E`, `ctrl-g`), the way the Settings
sections share `settings`. A capturing Skills mode whose widget hands `Tab` to the shell today (the
two editors, their name and import prompts, the agent help) keeps doing so through a `TABS` layer
(`global ∩ {next_tab, prev_tab}`, unfiltered) ahead of `MODAL` (PA-2). Text widgets see the key
first, then the resolver (M3 Rule 1; PA-3), so `form.save` is resolved after the widget passes the
chord, which every widget does for `ctrl-s` once T-close drops `TextArea`'s own claim (D6). The
`skills/mod.rs` switch and `agent_help.rs`, which both lanes' views embed, convert in T1 (PA-1).

---

## 1. Plan amendments

| # | Amends | Confirmed text | Why it cannot stand as written | What this blueprint builds |
|---|---|---|---|---|
| **PA-1** | Tasks (T1 file set and gate), lane table, "Known coupling" | T1 owns `keys/` and `tests/keys*.rs`; L-A owns `skills/{mod,library,attach,agent_help}.rs`; T1 changes no snapshot; `templates__agent_help_*` move in L-A and T-close re-baselines them. | The lane files are disjoint but the build is not. `SkillsTab::key_stack` (L-A's `mod.rs`) must call `TemplatesView::key_stack` (L-B), and `TemplatesView` (L-B) must call `AgentHelp::key_stack` and a keyed `AgentHelp::hint` (L-A). Neither lane's worktree compiles without the other's new method, and `agent_help`'s hint row sits in both lanes' snapshots. | **T1 converts `skills/mod.rs` (D5) and `skills/agent_help.rs` (D7) whole**, adds `LibraryView::key_stack` and `TemplatesView::key_stack` returning `None` (bodies are the lanes'), passes `ctx.keys()` at the two `help.hint()` call sites (`library.rs:687`, `templates.rs:521`), and adds `Scroll::apply(act, len)` to `ui/tabs/backlog/detail/mod.rs` (the pane scroll all four view files need; `Scroll::on_key` stays for Backlog, M5). T1 re-baselines the three snapshots whose agent-help hint row moves (`skills__agent_help_proposal`, `templates__agent_help_asking`, `templates__agent_help_proposal`, hint row only) and edits the three agent-help hint assertions in `tests/skills.rs:1608` and `tests/templates.rs:1038`, `:1111` (lanes have not forked yet). **L-A is `library.rs` + `attach.rs`.** No snapshot is then shared between lanes; the "Known coupling" paragraph is void. |
| **PA-2** | D3 ("then the global layer — modal for capturing modes") | Capturing modes end with `Layer::modal(Global)`. | `MODAL` drops `Tab`/`BackTab`, but the Skills editors, their name/describe/import prompts and the agent help hand `Tab` to the shell today, so `Tab` switches tabs with the draft kept. Pinned: `tests/skills.rs:989` `tab_and_digits_still_switch_tabs_with_a_draft_open`, `tests/templates.rs:893` `tab_and_digits_while_editing`, `agent_help.rs:1238` `tab_passes_and_the_buffer_is_locked`. D11 keeps them green unchanged. | Those stacks carry `TABS = Layer::only(Global, &[NextTab, PrevTab])` (unfiltered) just before `MODAL`. The view returns `Pass` for a `NextTab`/`PrevTab` candidate; the shell's `apply_keys` resolves it through the same stack. T1 makes `Keys::status_line` and `Keys::help_lines` read **every** global layer of a stack (§3.4), so the editor's status line reads `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · …` and the `?` box has one `Global:` line. Every M3 stack and `Stack::BASE` render byte-identical (one global layer). Forms where `Tab` is a field key (rename, attach form, Requirements forms) use plain `MODAL`. |
| **PA-3** | D6 ("resolves `form.save` from its stack **before** feeding the widget (boxes'/personas' M3 shape, `boxes.rs:571`)"); Verified claim 9 | `form.save` before the widget. | Claim 9 is false on order: `boxes.rs:533-575` and `personas.rs:1670-1678` feed the `TextArea` **first** and resolve `form.save` on its `Pass` (M3 Rule 1, L-D Q6: widget keys are unstealable, a user chord on a widget key is inert). Resolving first would let a legal `[form] save = "enter"` (named key, `in_capture` allows it) swallow every newline in the Library and Templates editors. | **Widget first, then `form.save`**, in every M4 editor, as boxes does. Before T-close the `TextArea`'s own `ctrl-s` `Submit` still saves (same call); after T-close the widget passes `ctrl-s` and the resolver saves. Requirements keeps today's rule that a chord never reaches its widgets (`requirements/mod.rs:1022-1027`): a non-plain key goes straight to the resolver, so `ctrl-s` and a rebound `ctrl-…` save there without touching the `TextArea`; a plain key goes widget-first. |
| **PA-4** | D8.2, Risk row 5, T1 Validate | Editor stacks join `DECLARED` with the phrase "in the in-pane editor"; "only `editor.focus`/`editor.abort` are configurable" so no user file breaks. | (a) `DECLARED` phrases are unique (`stack.rs` `declared_starts_with_base_and_overlay_and_holds_every_view_stack`), so two stacks cannot share one. (b) The collision is two-way: `[global] quit = ["ctrl-x"]` is now refused (`editor.abort`'s `ctrl-x` in `EDITOR_UNFOCUSED`). `tests/fixtures/keys/valid.toml`, `keys/print.rs:153` `a_changed_table_round_trips_and_keeps_its_marks`, `tests/keys_file.rs:85` and `:445`, and README's "Changing keys" example all bind exactly that. | `("in the focused in-pane editor", EDITOR_FOCUSED)`, `("in the in-pane editor", EDITOR_UNFOCUSED)`, placed right after `OVERLAY` (shell stacks first, §4.3). T1 moves the four `quit = ["ctrl-x"]` uses to `["ctrl-y"]` (bound nowhere, checked against every default) and states the refusal in README. A user file with a global `ctrl-x` or `ctrl-4` is refused with the line named; `--default-keys` starts anyway. |

**User-visible beyond D9** (not amendments; reviewed per lane, listed so the maintainer sees them):
1. Library browse hint drops `r reload`, Templates' drops `move` after `j/k`: with ` · ` separators
   the full rows are 107 and 102 cells, over the 100-column frame (ANA §5.5's reason; D9 keeps the
   curation). Rendered: L-A §3, L-B §3.
2. Attach form hint `Tab/Up/Down field` → `Tab/Down field` (`All(FormNextField)`; no hint element
   lists two acts' chords).
3. Agent help: the discard is `confirm.no`, so `n/Esc discard` / `n/Esc close` (was `Esc …`); the
   proposal's scroll hint is `J/K scroll · PgUp/PgDn page`; `y` (accept) also closes an answer
   with nothing to accept (`Enter` already did).
4. Requirements: `Down`/`Up` move the field focus on the requirement form's Priority and Deciding
   fields, which ignored them (the `(Requirements, FormNextField/FormPrevField)` view defaults the
   area form needs, `forms.rs:96`, apply to every Requirements form; the `TextArea` fields consume
   `Up`/`Down` first, so the body and rationale are unchanged).
5. In every converted capturing mode, `F1` opens help and ALT chords pass (unbound), where some of
   these modes swallowed them (M3's accepted rule, D5).
6. Capturing Skills and Requirements modes show the filtered status line (`Ctrl+c quit · …`)
   instead of `q quit · …` (D9: status rows).

---

## 2. Plan claims found false or incomplete (for the record)

| Claim | Finding |
|---|---|
| 9 (boxes/personas resolve `form.save` before the `TextArea`) | False on order: after (PA-3). |
| 20 (lane source sets pairwise disjoint) | True as files, false as a build boundary (PA-1). |
| Risk 5 (`[editor]` collisions refuse only `[editor]` lines) | False: `[global]` lines too; four in-tree uses (PA-4). |
| D3 "modal for capturing modes" | Incomplete: five Skills modes pass `Tab` today, pinned (PA-2). |
| `catalogue.rs` `form.save` comment cites `library.rs:939` | Stale: the rename form's `ctrl-s` is `library.rs:990` (T1 fixes the comment). |
| README "with `[global] quit = ["J"]`, `J` still scrolls the Backlog detail pane" | Becomes a refused file after M4 (`J` is `pane.scroll_down` in Library, Templates and Requirements browse); T-close rewrites the example (§7). |

---

## 3. T1 exact interfaces

### 3.1 `keys/catalogue.rs`: contexts

Appended to `Context` after `Waiting`, in D1's order; `ALL`, `table`, `heading`, `is_view`,
`is_shared` and the test-only `context_position` (no wildcard) grow in lockstep. `Context::ALL` has
29 entries.

| Variant | `table()` | `heading()` | `is_view` | `is_shared` | Own rows |
|---|---|---|---|---|---|
| `Skills` | `skills` | `Skills` | no | **yes** (like `settings`) | 7 |
| `SkillsLibrary` | `skills.library` | `Library` | yes | no | 3 |
| `SkillsTemplates` | `skills.templates` | `Templates` | yes | no | 1 |
| `SkillsAttach` | `skills.attach` | `Attachments` | yes | no | 3 |
| `SkillsHelp` | `skills.help` | `Agent help` | yes | no | 4 |
| `Requirements` | `requirements` | `Requirements` | yes | no | 4 |

`Skills` is shared, not a view: it is a wider layer whose acts the Library and Templates view
layers inherit (PA-3 of M3), so `[skills.library] diff = "D"` overrides `skills.diff` in the Library
alone, and `[skills] diff` changes both. The validator's per-context pass checks `[skills]`.

### 3.2 `keys/catalogue.rs`: the 22 new acts (CATALOGUE 79 → 101)

`row` = not in capture, `cap` = `capture_row`. Each row's comment cites the arm it mirrors (file:line
pre-edit). No default changes: every chord below is today's.

| # | `Act` | Context | name | defaults | help (`?` box) | kind | mirrors |
|---|---|---|---|---|---|---|---|
| 1 | `SkillsSwitchView` | `Skills` | `switch_view` | `["h", "l", "[", "]", "left", "right"]` | `switch view` | row | `skills/mod.rs:111-118` (one toggle, two views: D2) |
| 2 | `SkillsPrevVersion` | `Skills` | `prev_version` | `[","]` | `older version` | row | `library.rs:762`/`:799`, `templates.rs:576`/`:616` |
| 3 | `SkillsNextVersion` | `Skills` | `next_version` | `["."]` | `newer version` | row | same arms (`'.'`) |
| 4 | `SkillsBase` | `Skills` | `base` | `["b"]` | `diff base` | row | `library.rs:816`, `templates.rs:626` |
| 5 | `SkillsDiff` | `Skills` | `diff` | `["d"]` | `diff` | row | `library.rs:823`, `templates.rs:630` |
| 6 | `SkillsEditExternally` | `Skills` | `edit_externally` | `["E"]` | `edit in $EDITOR` | row | `library.rs:843`, `templates.rs:657` (browse `E`; the editor's `ctrl-e` is `form.external_editor`) |
| 7 | `SkillsAskAgent` | `Skills` | `ask_agent` | `["ctrl-g"]` | `ask agent` | cap | `library.rs:1080`, `templates.rs:741` (MOD-55) |
| 8 | `LibraryImport` | `SkillsLibrary` | `import` | `["I"]` | `import` | row | `library.rs:753` |
| 9 | `LibraryInfo` | `SkillsLibrary` | `info` | `["i"]` | `rename` | row | `library.rs:854` (`'i'`) |
| 10 | `LibraryAttach` | `SkillsLibrary` | `attach` | `["a"]` | `attachments` | row | `library.rs:863` (opens), `attach.rs:399` (`a` closes: the same toggle) |
| 11 | `TemplatesDiffDefault` | `SkillsTemplates` | `diff_default` | `["D"]` | `diff default` | row | `templates.rs:643` |
| 12 | `AttachChoose` | `SkillsAttach` | `choose` | `["enter"]` | `choose row` | row | `attach.rs:368` (browse: edit the row), `:562` (picker: insert the repo) |
| 13 | `AttachDetach` | `SkillsAttach` | `detach` | `["x"]` | `detach` | row | `attach.rs:379` |
| 14 | `AttachRepo` | `SkillsAttach` | `repo` | `["ctrl-r"]` | `repo picker` | cap | `attach.rs:476` |
| 15 | `SkillsHelpPrevAgent` | `SkillsHelp` | `prev_agent` | `["up"]` | `previous agent` | cap | `agent_help.rs:328` (`Up`) |
| 16 | `SkillsHelpNextAgent` | `SkillsHelp` | `next_agent` | `["down"]` | `next agent` | cap | `agent_help.rs:328` (`Down`) |
| 17 | `SkillsHelpAccept` | `SkillsHelp` | `accept` | `["enter", "y"]` | `accept` | row | `agent_help.rs:294` (and `:308`'s `Enter`) |
| 18 | `SkillsHelpCancel` | `SkillsHelp` | `cancel` | `["esc"]` | `cancel the turn` | row | `agent_help.rs:273`, `:280` |
| 19 | `RequirementsNewArea` | `Requirements` | `new_area` | `["a"]` | `new area` | row | `requirements/mod.rs:448` (`'a'`) |
| 20 | `RequirementsAmend` | `Requirements` | `amend` | `["e"]` | `amend` | row | `requirements/mod.rs:448` (`'e'`) |
| 21 | `RequirementsWithdraw` | `Requirements` | `withdraw` | `["W"]` | `withdraw` | row | `requirements/mod.rs:448` (`'W'`) |
| 22 | `RequirementsFilter` | `Requirements` | `filter` | `["/"]` | `filter` | row | `requirements/mod.rs:438` |

Shared acts the M4 views use (all exist, defaults unchanged): `list.{down, up, top, bottom, fold}`,
`pane.{scroll_down, scroll_up, page_down, page_up}`, `confirm.{yes, no}`, `form.{next_field,
prev_field, save, external_editor}`, `common.{edit, new, reload, back}`, `global.{next_tab,
prev_tab}`. Discard in the agent help is `confirm.no` (`n`, `Esc`; `agent_help.rs:302`, `:308`),
the Requirements filter clear is `common.back` (`requirements/mod.rs:444`), the attach pane's and
picker's `Esc` and the import report's `Esc` are `common.back` (`attach.rs:399`, `:580`,
`library.rs:946`).

Name checks (T1 tests): no `[skills]` name equals a name in another shared context
(`shared_names_are_unique_across_shared_contexts`); no view's own name equals a shared name it
inherits (`no_view_name_hides_a_shared_name_it_inherits`); no two rows of one context share a
default (`no_spec_is_shared_in_a_context_unless_state_guarded`: this is why the attach `Enter` is
one `choose` and the help's discard is `confirm.no`, not a second `esc` row).

Comment fixes while T1 is there: `form.save`'s row cites `library.rs:990` (not `:939`) and says the
Library/Templates/Requirements editors resolve it after the widget (M4); `form.next_field`/
`prev_field` cite the M4 `VIEW_DEFAULTS` rows; `editor.focus`'s comment "MOD-67 M2's loader must
refuse a file that unbinds it" becomes "the loader refuses a file that unbinds it (MOD-67 M4
D8.1)".

### 3.3 `VIEW_DEFAULTS` (+6; 14 → 20)

Order: `Context::ALL`, then `Act`. The first chord is the hint label.

| Context | Act | extra | derived with defaults | mirrors |
|---|---|---|---|---|
| `SkillsLibrary` | `FormNextField` | `["down"]` | `tab, down` | `library.rs:1018` (rename form) |
| `SkillsLibrary` | `FormPrevField` | `["up"]` | `backtab, up` | `library.rs:1018` |
| `SkillsAttach` | `FormNextField` | `["down"]` | `tab, down` | `attach.rs:487` |
| `SkillsAttach` | `FormPrevField` | `["up"]` | `backtab, up` | `attach.rs:488` |
| `Requirements` | `FormNextField` | `["down"]` | `tab, down` | `requirements/forms.rs:96` (area form) |
| `Requirements` | `FormPrevField` | `["up"]` | `backtab, up` | `requirements/forms.rs:96` |

`SHADOWING` and `STATE_GUARDED` gain **nothing**: checked chord by chord against every stack of §4
(no stack holds two acts on one default chord; `esc` meets only `common.back` in the browse
stacks, which have no `common.dismiss`).

### 3.4 `keys/hint.rs`: every global layer (PA-2)

```rust
impl Keys {
    /// The status line (D7, M3 PA-9) from `stack`'s global layers: quit first, always, as its
    /// first admitted chord, else `Ctrl+c`; then every other `Global` row in catalogue order that
    /// is `offered` and has an admitted chord, each through the **first** global layer that
    /// offers it (`Stack::admits`), rows sharing a help label collapsed to the first (the
    /// digits). A stack with one global layer renders exactly as before (M4 PA-2).
    pub fn status_line(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> String;

    /// The `?` box (D8): one line per layer, narrowest first, as before, except that **every
    /// global layer of the stack renders as one `Global:` line** at the last global layer's place:
    /// quit first (its admitted chords, then `Ctrl+c`), then the rows in layer order, an act
    /// listed under the first global layer that offers it (M4 PA-2).
    pub fn help_lines(&self, stack: Stack<'_>, offered: impl Fn(Act) -> bool) -> Vec<HelpLine>;
}
```
Implementation note: in `status_line`, replace the single `stack.global()` with "for a `Global`
row, the first index `i` with `layers[i].context() == Global && stack.admits(i, row.act)`; its
layer's `admits_chord` filters". In `help_lines`, open the shared `merged` (quit placeholder
first) at the first global layer and push the line at the last one. `help_closer` and
`Stack::passes` keep using the **last** global layer (`MODAL` in a `TABS` stack: `F1 closes this
box`, `passes(tab)` false; the view returns `Pass` for a `NextTab`/`PrevTab` candidate itself).

### 3.5 `keys/stack.rs`

- `DECLARED` order and phrases: §4.3 (57 entries).
- Doc: "M4 appends the Skills and Requirements stacks and the two editor stacks (MOD-57 §6.4, D8.2)".

### 3.6 `keys/load.rs` and `keys/validate.rs`: D8

- `Loader::entry` (`load.rs:418`): after the chords are parsed,
  ```rust
  if items.is_empty() {
      let why = match spec.act {
          Act::OverlayClose => Some("overlays swallow every other key"),
          Act::EditorFocus => Some("the in-pane editor is left only through it"),
          _ => None,
      };
      if let Some(why) = why {
          self.push(key_line, format!("[{table}] {name}: must keep at least one chord: {why}"));
      }
  }
  ```
  Message for D8.1: `[editor] focus: must keep at least one chord: the in-pane editor is left only
  through it`.
- `validate::give_way` (`validate.rs:30-35`): `default.act == Act::OverlayClose` becomes
  `matches!(default.act, Act::OverlayClose | Act::EditorFocus)`; doc: "`overlay.close` and
  `editor.focus` never give way (each must keep a chord)". So `[editor] abort = "ctrl-4"` is
  reported as a collision in `[editor]` instead of silently unbinding the focus toggle.
- The `ctrl-c` refusal needs no code (`chord_of`, `load.rs:241`, is table-agnostic); D8.3 is a test.
- `--print-keys` needs no code (`print.rs` iterates `Context::ALL`, which has `Editor`); D8.4 is a
  test.
- ANA "MOD-57 must not ship without that leave action": the loader now enforces it.

### 3.7 `ui/tabs/backlog/detail/mod.rs`: `Scroll::apply` (PA-1)

```rust
impl Scroll {
    /// One pane-scroll act (MOD-67 M4): `pane.scroll_down`/`scroll_up` one row,
    /// `pane.page_down`/`page_up` ten, clamped as `on_key`; `Pass` for any other act.
    pub fn apply(&mut self, act: Act, len: usize) -> Handled;
}
```
`on_key` maps its four `KeyCode`s to the step and shares the clamp (Backlog converts in M5).
Callers: `agent_help.rs` (T1), `library.rs` (L-A), `templates.rs` (L-B), `requirements/mod.rs` (L-C).

### 3.8 `ui/tabs/skills/mod.rs` (T1, PA-1; D5)

```rust
impl Tab for SkillsTab {
    /// The shown view's stack (MOD-67 M4 D4): the Library's (an open agent help's, else the
    /// attachments pane's, else its mode's) or the Templates view's; `None` while that view is
    /// unconverted.
    fn key_stack(&self) -> Option<Stack<'static>> {
        match self.view {
            View::Skills => self.library.key_stack(),
            View::Templates => self.templates.key_stack(),
        }
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A capturing mode answers first: `l` is a letter there (its stack has no `skills`
        // switch either, D5).
        let captured = match self.view { /* unchanged */ };
        if !captured {
            let stack = self.key_stack().unwrap_or(views::SKILLS_TAB);
            if ctx.keys().actions(stack, KeyChord::from_event(key)).first()
                == Some(&Act::SkillsSwitchView)
            {
                self.toggle();
                return Handled::Consumed;
            }
        }
        match self.view { /* delegate, unchanged */ }
    }
}
```
`library.rs` and `templates.rs` gain, in T1, `pub(super) fn key_stack(&self) -> Option<Stack<
'static>> { None }` with the doc "the current mode's stack (MOD-67 M4); filled by lane L-A/L-B".
`KeyCode` leaves `skills/mod.rs`. `ctrl-l`, `ctrl-h`, `alt-l`, `shift-right` stop switching
(defect 1, D5).

### 3.9 `ui/tabs/skills/agent_help.rs` (T1, PA-1; D7)

Full conversion, arm map in L-A §6 (kept in the L-A file so the Skills material is in one place,
but implemented by T1). Interface after T1:
```rust
impl AgentHelp {
    /// The stack of the help's state (MOD-67 M4 D4, D7): `HELP_ASKING`, `HELP_WAITING`
    /// (starting, streaming, cancelling) or `HELP_PROPOSAL` (proposal, answered).
    pub(super) fn key_stack(&self) -> Stack<'static>;
    /// The hint row while the help is open, through `key_stack` (MOD-67 M4 D9).
    pub(super) fn hint(&self, keys: &Keys) -> String;   // was `-> &'static str`
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> HelpOutcome; // unchanged sig
}
```
`library.rs:687` and `templates.rs:521` become `Some(help) => help.hint(ctx.keys()),`. The lanes
add `help.key_stack()` to their `key_stack` (an open help wins, D4).

---

## 4. `keys/views.rs`: every M4 stack

Module doc gains: "M4 adds the Skills and Requirements stacks. Layer order: view, a parent view's
`only` layer (the attachments pane over the Library), `settings`/`skills`, `confirm`, `form`,
`common`, `list`, `pane`, `overlay`, then the global layers; a capturing Skills mode whose widget
passes `Tab` puts [`TABS`] before [`MODAL`] (PA-2)."

New private layer constants:
```rust
/// `global ∩ {next_tab, prev_tab}`, unfiltered: a capturing Skills mode still hands `Tab` and
/// `Shift+Tab` to the shell, the draft kept (MOD-67 M4 PA-2). Always just before `MODAL`.
const TABS: Layer = Layer::only(Context::Global, &[Act::NextTab, Act::PrevTab]);
/// `pane ∩ {scroll_down, scroll_up, page_down, page_up}`.
const PANE_SCROLL: Layer = Layer::only(Context::Pane, &[Act::PaneScrollDown, Act::PaneScrollUp,
    Act::PanePageDown, Act::PanePageUp]);
/// `skills ∩ {switch_view}`.
const SKILLS_SWITCH: Layer = Layer::only(Context::Skills, &[Act::SkillsSwitchView]);
/// `skills ∩` the verbs Library and Templates browse share.
const SKILLS_BROWSE: Layer = Layer::only(Context::Skills, &[Act::SkillsSwitchView,
    Act::SkillsPrevVersion, Act::SkillsNextVersion, Act::SkillsBase, Act::SkillsDiff,
    Act::SkillsEditExternally]);
/// The editors' own chords: `skills ∩ {ask_agent}`.
const SKILLS_ASK: Layer = Layer::only(Context::Skills, &[Act::SkillsAskAgent]);
/// `form ∩ {save, external_editor}`: a `TextArea` editor's chords.
const FORM_EDITOR: Layer = Layer::only(Context::Form, &[Act::FormSave, Act::FormExternalEditor]);
/// `form ∩ {next_field, prev_field, save}`: a multi-field form that saves on a chord.
const FORM_ALL: Layer = Layer::only(Context::Form, &[Act::FormNextField, Act::FormPrevField,
    Act::FormSave]);
```
(`FORM_SAVE`, `LIST`, `CONFIRM`, `MODAL`, `GLOBAL` are M3's.)

### 4.1 The stacks (19 new `pub const`s)

Notation: `view(C, own)` = `Layer::view`; `C∩{…}` = `Layer::only`; constants as above.

| # | Constant | Layers, narrowest first | View layer inherits (overridable per view) | `DECLARED` phrase | Returned by |
|---|---|---|---|---|---|
| 1 | `SKILLS_TAB` | `SKILLS_SWITCH`, `GLOBAL` | — | `in Skills` | `SkillsTab::on_key` while the view has no stack (T1; lane phase only) |
| 2 | `LIBRARY_BROWSE` | `view(SkillsLibrary, [LibraryImport, LibraryInfo, LibraryAttach])`, `SKILLS_BROWSE`, `common∩{New, Edit, Reload}`, `LIST`, `PANE_SCROLL`, `GLOBAL` | the six `skills` verbs, New, Edit, Reload, ListDown, ListUp, the four pane acts | `in Skills > Library` | Library `Browse` (also with a handed-off draft) |
| 3 | `LIBRARY_PROMPT` | `view(SkillsLibrary, [])`, `TABS`, `MODAL` | — | `in a Library name, description or import prompt` | `Naming`, `Describing`, `ImportPath` |
| 4 | `LIBRARY_INFO` | `view(SkillsLibrary, [])`, `FORM_ALL`, `MODAL` | FormNextField, FormPrevField (+ view defaults), FormSave | `in the Library rename form` | `Info` |
| 5 | `LIBRARY_EDITOR` | `view(SkillsLibrary, [])`, `SKILLS_ASK`, `FORM_EDITOR`, `TABS`, `MODAL` | SkillsAskAgent, FormSave, FormExternalEditor | `in the Library editor` | `Editing` with no help; `agent_help`'s refusal check (`HelpTarget::Skill`) |
| 6 | `LIBRARY_REPORT` | `view(SkillsLibrary, [])`, `SKILLS_SWITCH`, `common∩{Reload, Back}`, `LIST`, `PANE_SCROLL`, `GLOBAL` | SkillsSwitchView, Reload, Back, ListDown, ListUp, pane | `in the Library import report` | `Report` |
| 7 | `ATTACH_BROWSE` | `view(SkillsAttach, [AttachChoose, AttachDetach])`, `SkillsLibrary∩{LibraryAttach}`, `SKILLS_SWITCH`, `common∩{Reload, Back}`, `LIST`, `GLOBAL` | SkillsSwitchView, Reload, Back, ListDown, ListUp | `in Skills > Library > Attachments` | attach `Browse` |
| 8 | `ATTACH_FORM` | `view(SkillsAttach, [AttachRepo])`, `FORM_ALL`, `MODAL` | FormNextField, FormPrevField (+ view defaults), FormSave | `in the Attachments form` | attach `Form` |
| 9 | `ATTACH_PICKER` | `view(SkillsAttach, [AttachChoose])`, `common∩{Back}`, `LIST`, `MODAL` | Back, ListDown, ListUp | `in the Attachments repo picker` | attach `Picker` |
| 10 | `ATTACH_CONFIRM` | `view(SkillsAttach, [])`, `CONFIRM`, `MODAL` | ConfirmYes, ConfirmNo | `in the Attachments detach question` | attach `ConfirmDetach` |
| 11 | `TEMPLATES_BROWSE` | `view(SkillsTemplates, [TemplatesDiffDefault])`, `SKILLS_BROWSE`, `common∩{New, Edit, Reload}`, `LIST`, `PANE_SCROLL`, `GLOBAL` | as Library browse | `in Skills > Templates` | Templates `Browse` (also handed off) |
| 12 | `TEMPLATES_PROMPT` | `view(SkillsTemplates, [])`, `TABS`, `MODAL` | — | `in the Templates name prompt` | `Naming` |
| 13 | `TEMPLATES_EDITOR` | `view(SkillsTemplates, [])`, `SKILLS_ASK`, `FORM_EDITOR`, `TABS`, `MODAL` | SkillsAskAgent, FormSave, FormExternalEditor | `in the Templates editor` | `Editing` with no help; `agent_help`'s refusal check (`HelpTarget::Template`) |
| 14 | `HELP_ASKING` | `view(SkillsHelp, [SkillsHelpPrevAgent, SkillsHelpNextAgent])`, `TABS`, `MODAL` | — | `in the agent help prompt` | `AgentHelp` `Asking` |
| 15 | `HELP_WAITING` | `view(SkillsHelp, [SkillsHelpCancel])`, `TABS`, `MODAL` | — | `while the agent help waits` | `Starting`, `Streaming`, `Cancelling` |
| 16 | `HELP_PROPOSAL` | `view(SkillsHelp, [SkillsHelpAccept])`, `confirm∩{ConfirmNo}`, `PANE_SCROLL`, `TABS`, `MODAL` | ConfirmNo, pane | `in the agent help proposal` | `Proposal`, `Answered` |
| 17 | `REQUIREMENTS_BROWSE` | `view(Requirements, [RequirementsNewArea, RequirementsAmend, RequirementsWithdraw, RequirementsFilter])`, `common∩{New, Reload, Back}`, `list∩{ListDown, ListUp, ListTop, ListBottom, ListFold}`, `PANE_SCROLL`, `GLOBAL` | New, Reload, Back, the five list acts, pane | `in Requirements` | `Browse` |
| 18 | `REQUIREMENTS_FORM` | `view(Requirements, [])`, `FORM_ALL`, `MODAL` | FormNextField, FormPrevField (+ view defaults), FormSave | `in a Requirements form` | `NewArea`, `Requirement` (mint and amend) |
| 19 | `REQUIREMENTS_WITHDRAW` | `view(Requirements, [])`, `FORM_SAVE`, `MODAL` | FormSave | `in the Requirements withdraw form` | `Withdraw` (both stages) |

The Requirements filter uses M3's `CAPTURE` (a text field and nothing else, PA-7 of M3); `CAPTURE`'s
doc gains "the Requirements filter".

Collision check with defaults (every stack; chords per layer, filtered): no chord resolves to two
acts except `NextTab`/`PrevTab` shadowing `MODAL`'s own rows, which is shadowing of the *same* act
(the `actions` rule), not a pair. Spot checks: `LIBRARY_BROWSE` `I i a | h l [ ] Left Right , . b d E
| e n r | j Down k Up | J K PgDn PgUp | q Tab BackTab 1-9 ? F1 w ctrl-f ctrl-w ctrl-q`;
`HELP_PROPOSAL` `Enter y | n Esc | J K PgDn PgUp | Tab BackTab | F1 ctrl-f ctrl-w ctrl-q`;
`REQUIREMENTS_BROWSE` `a e W / | n r Esc | j Down k Up g Home G End Enter | J K PgDn PgUp | global`.
`the_compiled_defaults_validate` holds.

### 4.2 Resolution pins (T1, `views.rs` `the_view_stacks_resolve_as_designed` grows)

`SKILLS_TAB` `l` → `[SkillsSwitchView]`, `ctrl-l`/`ctrl-h`/`alt-l` → `[]`; `LIBRARY_BROWSE` `,` →
`[SkillsPrevVersion]`, `E` → `[SkillsEditExternally]`, `ctrl-e` → `[]`, `J` → `[PaneScrollDown]`,
`q` → `[Quit]`; `LIBRARY_EDITOR` `tab` → `[NextTab]`, `ctrl-g` → `[SkillsAskAgent]`, `ctrl-s` →
`[FormSave]`, `ctrl-e` → `[FormExternalEditor]`, `l`/`q` → `[]`, `f1` → `[Help]`, and
`!passes(tab)`; `LIBRARY_INFO` `down` → `[FormNextField]`; `ATTACH_BROWSE` `a` →
`[LibraryAttach]`, `enter` → `[AttachChoose]`, `esc` → `[Back]`; `ATTACH_FORM` `ctrl-r` →
`[AttachRepo]`, `down` → `[FormNextField]`; `ATTACH_CONFIRM` `ctrl-y` → `[]`; `TEMPLATES_BROWSE` `D`
→ `[TemplatesDiffDefault]`; `HELP_ASKING` `down` → `[SkillsHelpNextAgent]`, `tab` → `[NextTab]`;
`HELP_PROPOSAL` `y` → `[SkillsHelpAccept]`, `esc` → `[ConfirmNo]`; `HELP_WAITING` `esc` →
`[SkillsHelpCancel]`; `REQUIREMENTS_BROWSE` `/` → `[RequirementsFilter]`, `enter` → `[ListFold]`,
`ctrl-j` → `[]`; `REQUIREMENTS_FORM` `down` → `[FormNextField]`, `q` → `[]`;
`REQUIREMENTS_WITHDRAW` `tab` → `[]`, `ctrl-s` → `[FormSave]`.

### 4.3 `DECLARED` (36 → 57)

| Index | Phrase | Stack |
|---|---|---|
| 0 | `on every screen` | `Stack::BASE` |
| 1 | `over an overlay` | `Stack::OVERLAY` |
| 2 | `in the focused in-pane editor` | `Stack::EDITOR_FOCUSED` (D8.2, PA-4) |
| 3 | `in the in-pane editor` | `Stack::EDITOR_UNFOCUSED` (D8.2, PA-4) |
| 4 | `in Settings` | `views::SETTINGS_TAB` |
| 5 | `while a field captures keys` | `views::CAPTURE` |
| 6 | `in Skills` | `views::SKILLS_TAB` |
| 7-38 | M3's 32 view stacks, unchanged order and phrases | |
| 39-56 | §4.1 rows 2-19, in that order | |

Indices 0-6 are the stacks that open with no view layer; `views.rs`
`every_view_layer_comes_first_and_every_modal_stack_ends_modal` slices `DECLARED[7..]` (was
`[4..]`) and expects 50. `every_view_stack_is_named_by_a_view` compares `views.rs`' `pub const`
count with `DECLARED.len() - 4` (BASE, OVERLAY and the two editor stacks live in `stack.rs`).

---

## 5. T1 tests (write red first) and the existing tests T1 changes

### 5.1 New and grown unit tests

| Id | File | Test | Asserts |
|---|---|---|---|
| T-C1 | `catalogue.rs` | `every_act_is_listed_and_has_a_row` (ALL/`position` +22), `every_act_has_exactly_one_row` (101) | lockstep |
| T-C2 | `catalogue.rs` | `every_context_is_listed_once_in_all` (29), `context_tables_and_headings` (+6 rows of §3.1), `catalogue_blocks_follow_context_all` | §3.1 |
| T-C3 | `catalogue.rs` | `in_capture_acts_are_…` renamed `in_capture_acts_are_the_form_overlay_close_editor_focus_concepts_and_the_skills_chords`; + `SkillsAskAgent`, `AttachRepo`, `SkillsHelpPrevAgent`, `SkillsHelpNextAgent` | |
| T-V1 | `views.rs` | `the_view_stacks_resolve_as_designed` | §4.2 |
| T-V2 | `views.rs` | `every_view_layer_comes_first_and_every_modal_stack_ends_modal` | `DECLARED[7..]`, 50; also: a `TABS` layer, where present, is the layer just before the modal one |
| T-V3 | `views.rs` | `no_capturing_stack_offers_the_view_switch` (new) | every `DECLARED` stack whose last global layer is modal admits `SkillsSwitchView` in no layer (D5 by construction) |
| T-V4 | `views.rs` | `every_view_stack_is_named_by_a_view` | `VIEWS` gains `skills/{mod,library,templates,attach,agent_help}.rs`, `requirements/{mod,forms}.rs`; `DECLARED.len() - 4`; a `PENDING: &[&str]` of the 13 lane stacks (`LIBRARY_{BROWSE,PROMPT,INFO,REPORT}`, `ATTACH_{BROWSE,FORM,PICKER,CONFIRM}`, `TEMPLATES_{BROWSE,PROMPT}`, `REQUIREMENTS_{BROWSE,FORM,WITHDRAW}`) is skipped, doc "MOD-67 M4: the lanes name these; T-close empties the list". `SKILLS_TAB`, `HELP_*`, `LIBRARY_EDITOR`, `TEMPLATES_EDITOR` are named by T1's `mod.rs`/`agent_help.rs` |
| T-S1 | `stack.rs` | `declared_starts_with_base_and_overlay_and_holds_every_view_stack` | `len() == 57`, phrases unique, indices 2-3 are the editor stacks |
| T-H1 | `hint.rs` | `a_tabs_layer_shows_on_the_status_line_and_in_one_global_line` | `status_line(TEMPLATES_EDITOR, all)` = `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue`; bare (`offered` false for workspaces/find/waiting/queue) = `Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help`; `help_lines(TEMPLATES_EDITOR, all)` = `[Skills: Ctrl+g ask agent, Form: Ctrl+s save · Ctrl+e $EDITOR, Global: Ctrl+c quit · Tab next tab · Shift+Tab previous tab · F1 help · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue]` (exactly one `Global` line); `help_closer(TEMPLATES_EDITOR)` = `F1 closes this box` |
| T-H2 | `hint.rs` | the existing M3 status/help tests | unchanged, green (one-global-layer stacks are byte-identical) |
| T-L1 | `load.rs` | `editor_focus_must_keep_a_chord` | `[editor]\nfocus = []\n` → `one(2, "[editor] focus: must keep at least one chord: the in-pane editor is left only through it")`; `[editor]\nfocus = "ctrl-c"\n` → `one(2, "[editor] focus = \"ctrl-c\": ctrl-c always quits and cannot be bound")` (D8.3) |
| T-L2 | `load.rs` | `an_unknown_table_lists_the_tables` + `TABLES` | `TABLES` ends `…, concepts, switcher, migration, waiting, skills, skills.library, skills.templates, skills.attach, skills.help, requirements` |
| T-L3 | `load.rs` | `a_skills_view_table_overrides_its_shared_verbs` | `[skills.library]\ndiff = "D"\n` loads; `actions(LIBRARY_BROWSE, D)` = `[SkillsDiff]`, `actions(TEMPLATES_BROWSE, d)` = `[SkillsDiff]`; `[skills.templates]\nimport = "x"\n` → the "no such action; [skills.templates] has diff_default, and may override …" message (names in catalogue order) |
| T-X1 | `validate.rs` | `the_compiled_defaults_validate` | over 57 stacks (body unchanged) |
| T-X2 | `validate.rs` | `editor_focus_never_gives_way` | `[editor]\nabort = "ctrl-4"\n` → `one(2, "[editor] abort = \"ctrl-4\": \"ctrl-4\" is already editor.focus (default) in [editor]")`, and `editor.focus` keeps `ctrl-4` (load with only the error) |
| T-X3 | `validate.rs` | `an_editor_chord_on_a_global_default_is_refused_either_way` | `[editor]\nabort = "f1"\n` → `… "f1" is already global.help (default) in the in-pane editor`; `[global]\nquit = ["q", "ctrl-x"]\n` → `[global] quit = "ctrl-x": "ctrl-x" is already editor.abort (default) in the in-pane editor` (D8.2, D11) |
| T-P1 | `print.rs` | `the_editor_table_round_trips` | `[editor]\nfocus = ["ctrl-4", "f12"]\nabort = []\n`: print, load, equal; re-print identical; both lines `(changed)` (D8.4) |
| T-P2 | `print.rs` | `the_default_print_…` | ends with the `[requirements]` table's `prev_field = ["backtab", "up"]` row (exact padding from the run); still ≤ 100 columns |
| T-Sc | `backlog/detail/mod.rs` | `apply_steps_and_clamps_like_on_key` | `apply(PaneScrollDown, 5)` = `on_key(J)`, page acts move 10 and clamp at `len - 1`, `apply(Edit, 5)` = `Pass` |
| T-A* | `agent_help.rs` | L-A §6.5 | |

### 5.2 New integration tests (T1)

| File | Test | Asserts |
|---|---|---|
| `tests/keys.rs` | `ctrl_l_and_ctrl_h_do_not_switch_skills_views` | `Harness` with `SkillsTab`: `ctrl-l`, `ctrl-h`, `alt-l`, `shift-right` leave the Skills view (switch line style / Library hint); `l` shows Templates, `h` back (D5, D11 pin) |
| `tests/keys.rs` | `a_capturing_skills_view_keeps_tab_and_offers_no_switch` | Templates editor open (`2`… or the demo's Skills tab, `l`, select, `e`): `l` is typed (draft contains `l`), `tab` reaches Requirements (as `templates.rs:893`) — the PA-2 pin at the shell level |
| `tests/keys_file.rs` | `bad_fixtures` | `errors`: line 8 table list (+6 tables); line 13 gains `"x" is already skills.attach.detach (default) in Skills > Library > Attachments`. `collision`: line 6 gains `"esc" is already common.back (default) in the Library import report` (in `DECLARED` order, after the Agents lines) |
| `tests/keys_file.rs` | `print_keys_prints_a_valid_file`, `a_rebound_quit_and_close_work_end_to_end` | `ctrl-y` for `ctrl-x` (PA-4) |
| `tests/keys_file.rs` | `an_editor_file_that_unbinds_focus_exits_2` | a temp file `[editor]\nfocus = []\n` → exit 2, report names line 2 (D8.1 end to end) |

### 5.3 Existing tests and fixtures T1 changes (and only these)

- `catalogue.rs`, `stack.rs`, `views.rs`, `hint.rs` (none: T-H2 stays), `load.rs` (`TABLES`,
  `every_error_is_reported_and_sorted_by_line` line 13 gains the attach line), `validate.rs`
  (`a_global_rebind_onto_a_view_verb_is_refused` gains the `skills.attach.detach` line;
  `a_collision_over_an_overlay_is_found_in_the_overlay_stack` gains the `common.back` line),
  `print.rs` (`a_changed_table_round_trips_and_keeps_its_marks`: `ctrl-y`; T-P2's tail).
- `tests/fixtures/keys/valid.toml`: `quit = ["ctrl-y"]`, header comment "Quit moves to ctrl-y".
- `README.md` "Changing keys": the example's `quit = ["ctrl-y"]`; one sentence after "`ctrl-c`
  always quits": "A key the in-pane editor uses (`ctrl-4` to focus it, `ctrl-x` to abort it) is
  refused for `[global] quit` and `help`, and `[editor] focus` must keep a key." (PA-4).
- Agent help (PA-1): in-file `agent_help.rs` tests per L-A §6.5; `tests/skills.rs:1608`,
  `tests/templates.rs:1038` → `ends_with("Enter accept · n/Esc discard · J/K scroll · PgUp/PgDn
  page")`; `tests/templates.rs:1111` → `ends_with("Enter ask · Up/Down agent · Esc back")`.
- Snapshots (PA-1): `skills__agent_help_proposal`, `templates__agent_help_asking`,
  `templates__agent_help_proposal`: the hint row only (as the two assertions above); every other
  row byte-identical. Anything else moving in T1 is a defect.

### 5.4 T1 commit groups (each compiles, clippy both ways, its tests green)

1. **T1a** `feat(mod-67): M4 contexts, acts and view defaults` — `catalogue.rs` (§3.1-§3.3, comment
   fixes, T-C*). Lands the 22 acts unused (a `pub enum` variant is not dead code).
2. **T1b** `feat(mod-67): status line and ? box read every global layer` — `hint.rs` (§3.4, T-H1
   with a local test stack until T1d lands the real one; T-H2 green).
3. **T1c** `feat(mod-67): [editor] loader rules (MOD-57 §6.4)` — `load.rs`, `validate.rs`,
   `stack.rs` (editor stacks into `DECLARED` at 2-3, phrases), `views.rs` (slice `[6..]` and
   `DECLARED.len() - 4` for now),
   `print.rs`, fixtures and `keys_file.rs` (`ctrl-y`), README example and sentence; T-L1, T-X2,
   T-X3, T-P1, the end-to-end test.
4. **T1d** `feat(mod-67): Skills and Requirements stacks` — `views.rs` (§4), `stack.rs` (§4.3),
   T-V*, T-S1, T-L2, T-L3, T-P2, and every expectation of §5.3 that a new stack adds a line to
   (`load.rs`, `validate.rs`, `keys_file.rs`); T-H1 switched to `TEMPLATES_EDITOR`.
5. **T1e** `feat(mod-67): Scroll::apply for pane-scroll acts` — `backlog/detail/mod.rs`, T-Sc.
6. **T1f** `feat(mod-67): Skills view switch through the active stack` — `skills/mod.rs` (§3.8),
   the two `key_stack` stubs, `tests/keys.rs` D5 pins.
7. **T1g** `feat(mod-67): agent help dispatches on its stacks` — `agent_help.rs` (L-A §6), the
   two `hint(ctx.keys())` call sites, the three assertions, the three snapshots,
   `a_capturing_skills_view_keeps_tab_and_offers_no_switch`.

### 5.5 T1 gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib keys
cargo test -p htui --lib ui::tabs::skills ui::tabs::backlog::detail
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --features testkit \
  --test keys --test keys_file --test skills --test templates --test requirements \
  --test box_settings --test settings -- --test-threads=1
cargo insta test -p htui --all-features --check   # after accepting exactly the 3 of §5.3
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```
The last line once, before the lanes fork (memory: the suite is scheduling-dependent; grep the
output for `SIGABRT`).

---

## 6. Dispatch skeletons for M4 (the lanes copy these; M3 §6 otherwise)

Common: `let chord = KeyChord::from_event(key);` once; match on `Act`, never on `key.code` outside
a widget; `ctx.keys().actions(STACK, chord)`; delete every `plain(&key)`/`chord(&key)`/
`KeyModifiers` guard the resolver makes redundant (chord equality includes modifiers). `modal_rest`
is `crate::ui::tabs::settings::modal_rest` (already `pub(crate)`): `Pass` iff `stack.passes(chord)`.

### 6.1 Browse (unfiltered global)
M3 §6.2 unchanged: loop over candidates; an accepted view or shared act returns `Consumed`; a
declined one `continue`s; the loop's end returns `Handled::Pass` (the shell resolves the same
stack). Library and Templates browse never handle `SkillsSwitchView` (the tab took it first).

### 6.2 Capturing mode with a text widget, `Tab` passes (`*_PROMPT`, `*_EDITOR`, `HELP_ASKING`)
```rust
match widget.on_key(key /*, page */) {
    FieldOutcome::Consumed => return Handled::Consumed,
    FieldOutcome::Submit => return self.submit(ctx),  // Enter (TextField) / ctrl-s (TextArea, until T-close)
    FieldOutcome::Cancel => return self.cancel(),     // Esc
    FieldOutcome::Pass => {}
}
let stack = views::TEMPLATES_EDITOR;
match ctx.keys().actions(stack, chord).first() {
    Some(Act::FormSave) => { self.save(ctx); Handled::Consumed }        // D6, PA-3
    Some(Act::SkillsAskAgent) => { self.open_help(ctx); Handled::Consumed }
    Some(Act::FormExternalEditor) => { self.hand_off(ctx); Handled::Consumed }
    Some(Act::NextTab | Act::PrevTab) => Handled::Pass,                  // PA-2: the draft stays
    _ => modal_rest(stack, chord),
}
```
(`*_PROMPT` and `HELP_ASKING` have only the `NextTab | PrevTab` arm plus their own acts.)

### 6.3 Multi-field form, `Tab` is a field key (`LIBRARY_INFO`, `ATTACH_FORM`, `REQUIREMENTS_FORM`)
Widget first; on `Pass`: `FormNextField`/`FormPrevField` move the focus (wrap), `FormSave` saves,
the view's own act (`AttachRepo`), else `modal_rest`. No `TABS`: `Tab` never reaches the shell.

### 6.4 Question or non-text modal (`ATTACH_CONFIRM`, `ATTACH_PICKER`, `HELP_WAITING`, `HELP_PROPOSAL`)
M3 §6.4: first candidate; own acts consume; `NextTab | PrevTab` → `Pass` where the stack has
`TABS`; else `modal_rest` (or the mode's documented swallow: the detach question keeps the
attachment on any other key, L-A §5.4).

### 6.5 `key_stack` (every converted view)
One private `fn stack(&self) -> Stack<'static>` (or `key_stack` returning `Some`) is the only place
a mode maps to its stack; `on_key` and the hint read it. Priority inside the Library: attachments
pane open → its stack; else an open help → `help.key_stack()`; else the mode's (D4).

---

## 7. T-close (serial, primary tree)

1. **Merge** `mod-67-m4-l-a`, then `-l-b`, then `-l-c`, each `--no-ff`; after each, the full
   Validation (plan) with `--test-threads=1 --no-fail-fast`, and `cargo insta test -p htui
   --all-features --check` (no pending snapshot may remain: under PA-1 no snapshot is shared).
2. **D6, `TextArea` drops `ctrl-s`** (`ui/text_area.rs`):
   - `on_key`: any chord returns `FieldOutcome::Pass`; doc (`text_area.rs:185-190`) "Any chord
     passes, `ctrl-s` included: its owner saves on `form.save` (MOD-67 M4 D6)"; module doc line 13
     likewise. `Submit` is no longer produced by a `TextArea`.
   - Tests first: `text_area.rs:1057` `ctrl_s_submits_and_esc_cancels` →
     `ctrl_s_passes_and_esc_cancels`; new `tests/keys.rs` `a_rebound_save_is_the_only_save_in_every_text_area_editor`:
     with `[form] save = "f2"` (`Harness::with_keys` / `SectionBench::with_keys`): the Library
     editor, the Templates editor, a Requirements mint, boxes quirks, personas body each save on
     `F2` (one write request) and **not** on `ctrl-s` (no request, draft unchanged, `ctrl-s` not
     typed); with the defaults `ctrl-s` saves each.
   - Backlog owners untouched; verify: `tests/` item form and compose save on `ctrl-s` (existing
     tests green; name them in the commit body), and `relations usages` of `TextArea` lists no
     new key owner (claim 13 re-check).
   - Comments that become false: `boxes.rs:533-575` ("the `TextArea`'s own `ctrl-s` already
     submitted"), `personas.rs:1670-1675` (`editor_act` doc), the M4 views' `FieldOutcome::Submit`
     arms under a `TextArea` (L-A, L-B: comment "unreachable since D6" or drop the arm where the
     match stays exhaustive through `_`), `tests/box_settings.rs:1214-1236` (rename to
     `a_rebound_save_saves_the_quirks_editor_and_ctrl_s_no_longer_does`, assert `ctrl-s` sends
     nothing with the rebound file), `tests/personas.rs:1584` doc.
3. **Reachability**: delete `views.rs` `PENDING`; `every_view_stack_is_named_by_a_view` covers all
   19 M4 stacks.
4. **README "Changing keys"**: drop the D13 sentence ("in the editors that save with `Ctrl+s`, that
   key keeps saving after `[form] save` is changed"); "Backlog, Chat, Skills and Requirements do
   not read this file yet" → "Backlog and Chat …", with an example key that loads (pick a Backlog
   letter no declared stack binds, check it with `load_str`; `J` and `G` are refused now); the
   global-key sentence names Skills and Requirements too ("a key that Settings, Skills,
   Requirements or a pop-up already uses"); one line for the Skills editors: "`Tab` still leaves a
   Skills editor with the draft kept".
5. **HANDOFF**: M4 phase note (incl. MOD-57 §6.4 closed by D8), the picker-context carry (M3 L-C
   Q3), the prose left for M6 (`Ctrl+S saves it`, `Esc again discards`, `y detaches, any other key
   keeps it`, `select it and press e`, `HELP_OPEN`'s `Esc leaves it first`).
6. Plan review gate; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; Postgres
   suites once on the merged tree (`*_pg.rs`, sandbox DB 5439); remove the three worktrees, then
   the lane branches.

T-close commit groups: X1-X3 the merges; X4 `feat(mod-67): TextArea passes ctrl-s; form.save is
the only save (D6)`; X5 `test(mod-67): every M4 stack is named by a view`; X6 `docs(mod-67):
README keys, M4`; X7 `docs(mod-67): close-out M4`.

---

## 8. Per-lane summary (details in the lane files)

| Lane | Source | Stacks it returns | Snapshots it re-baselines (rows that may move) | Rebinding test |
|---|---|---|---|---|
| L-A | `skills/{library,attach}.rs` | `LIBRARY_*`, `ATTACH_*`, `help.key_stack()` | `skills__*` ×9: hint row (all), status row (capturing: `edit`, `changed_elsewhere`, `agent_help_proposal`, `attach_form_effective_globs`, `repo_picker`); the scroll border title is in no snapshot | `[skills.library] import = "M"` |
| L-B | `skills/templates.rs` | `TEMPLATES_*`, `help.key_stack()` | `templates__*` ×8: hint row, status row (`edit_help`, `changed_elsewhere`, `missing_item_confirm`, `unknown_placeholder_cursor`, `agent_help_asking`, `agent_help_proposal`) | `[skills.templates] diff_default = "X"` |
| L-C | `requirements/{mod,forms}.rs` | `REQUIREMENTS_*`, `CAPTURE` | `requirements__*` ×7: hint row only | `[requirements] amend = "E"` |

Lane gate (each, in its worktree):
```bash
cargo fmt -- --check <own paths>     # rustfmt --check on the lane's files
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib <ui::tabs::skills::library ui::tabs::skills::attach | ui::tabs::skills::templates | ui::tabs::requirements>
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --test <skills|templates|requirements> -- --test-threads=1
cargo insta test -p htui --all-features --check     # after accepting the lane's own snapshots
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
```

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `TABS` makes `Stack::passes` and the status line disagree (passes says no for `Tab`, the line shows it) | Medium | The skeleton returns `Pass` for `NextTab`/`PrevTab` explicitly (§6.2); T-V2 pins `TABS` just before `MODAL`; T-H1 pins the line; the T1 shell pin drives `Tab` out of the editor |
| T1 grows (agent help, `mod.rs`, `Scroll`) and delays the lanes | Medium | It removes the only cross-lane build and snapshot coupling; L-A shrinks by its largest-risk file |
| `[editor]` in `DECLARED` refuses a user's existing `[global]` line | Low | Error names the line and the editor stack; `--default-keys`; README sentence |
| A lane needs an act or stack §3-§4 lacks | Medium | Plan rule: main thread amends T1 on the base, lanes merge it; T-X1, T-V4 fail loudly |
| Requirements forms: chords bypass the widgets (PA-3) and a plain key goes widget-first; the Priority field must now `Pass` what it does not use | Medium | L-C §2 pins `m`/`l`/Space on Priority unchanged, `ctrl-m` inert on Priority, `F1` opens help from a form |
| Snapshot churn hides a behaviour change | Medium | D9 review; §1's list is the only accepted drift beyond spelling and separators |
