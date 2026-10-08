# Plan: MOD-67 M4 — Skills and Requirements dispatch through context stacks, plus the `[editor]` loader rules

**Source**: `HANDOFF.md` MOD-67; spec `docs/ANA-26.md` §6, §7.2-§7.6, §8 (M4 row); carried from
`.claude/plans/mod-67-m3-settings-overlays.plan.md` (D13: `TextArea`'s own `ctrl-s` moves to
`form.save` in M4; review gate "long `on_browse_key` arms split if M4 adds to them") and from the
HANDOFF MOD-57 M1 note (`.claude/plans/mod-57-m1-editor-in-pane.blueprint.md` §6.4: whichever of
MOD-57 and MOD-67 M2 lands second does the loader half; neither did)
**Routing**: plan (maintainer accepted 2026-10-08; C4 only, low confidence, the same call as
M1-M3). Ultracode for **implement** only; `code-architect` and `rust-reviewer` stay plain agents.
Sandbox run (`HR_SANDBOX=1`, branch `hr/MOD-67`, base `46b570c1`).
**Selected milestone**: M4 plus the MOD-57 `[editor]` loader follow-up (maintainer, 2026-10-08).
M5 and M6 are later runs.
**Complexity**: Large (7 source files to convert, ~214 `KeyCode` mentions, 24 hint constants,
24 snapshots, plus `keys/` and `text_area.rs`)
**Status**: done (M4 landed 2026-10-08, `ba082c94`..`4802a157`); M5-M6 are later runs

## Summary

The Skills tab (its view switch, Library, Templates, the attachments pane and the editors' agent
help) and the Requirements tab (tree, filter, the area/new/amend/withdraw forms) stop matching
`key.code` and dispatch named actions through declared context stacks in `keys/views.rs`, exactly
as M3 did for Settings. Their 24 `HINT_*` constants become `HintSpec`s. The status line and the `?`
box follow those stacks. The Skills `h`/`l` modifier blindness (`skills/mod.rs` view switch) is
fixed by construction and pinned. The Skills and Requirements editors save through `form.save`
ahead of their widget, and `TextArea` stops claiming `ctrl-s` itself, so a rebound `form.save`
replaces `ctrl-s` everywhere instead of adding a chord. Separately, the key-file loader gains the
`[editor]` rules MOD-57 left owed: `editor.focus` must keep a chord, the editor stacks join
`DECLARED` so `[editor]` chords are checked against `global.quit`/`global.help`, and a test pins
the `ctrl-c` refusal in `[editor]`. No default changes.

## Scope drift since ANA-26 (fact-checked, see table)

ANA §8 sized M4 as `skills/{mod,library,templates,attach}.rs`, `requirements/{mod,forms}.rs`,
"~6 source files, ~21 snapshots". The tree now has `skills/agent_help.rs` (MOD-55, 39 `KeyCode`
arms, the `Ctrl+G` help embedded in both editors) and **24** snapshots (`skills__` 9,
`templates__` 8, `requirements__` 7). `requirements/{detail,tree}.rs` match no keys and are not
touched. M4 takes `agent_help.rs`: leaving it on `KeyCode` would keep a rebound global key eaten
inside the help prompt. Prose that names keys (`templates.rs`/`library.rs` notices,
`requirements` messages) stays M6's.

## Decisions (proposed; confirmed at the CONFIRM gate)

- **D1 — Contexts.** `skills` for the tab (view switch), `skills.library`, `skills.templates`,
  `skills.attach`, `skills.help` (the agent help prompt and proposal), and `requirements`. `?`
  box headings: `Skills`, `Library`, `Templates`, `Attachments`, `Agent help`, `Requirements`.
  Appended to `CATALOGUE` in strip order, each row citing the arm it mirrors (M1's convention);
  the `ALL`/`position` test lists grow in lockstep.
- **D2 — Naming (ANA §6.2).** Shared verbs where the meaning is shared: `list.*` (`j`/`k`, `g`/
  `G`), `pane.scroll_*`/`page_*` (`J`/`K`, `PgUp`/`PgDn` on the body), `common.edit/new/reload/
  back/delete`, `confirm.*`, `form.*` (`form.save` `ctrl-s`, `form.external_editor` `ctrl-e`,
  `form.next_field`/`prev_field`). The Skills tab's switch is one act, `skills.switch_view`
  (`h`, `l`, `[`, `]`, `left`, `right`: today's toggle; two views, so no next/prev pair). View
  verbs where a letter means something only there: the version cursor (`,`/`.`), `b` base, `d`
  diff, `D` default, `E` $EDITOR from browse, `I` import, `i` info, `a` attach, `x` detach,
  `ctrl-g` ask agent, `ctrl-r` repo picker, Requirements `a` new area, `e` amend, `W` withdraw,
  `/` filter. Where Library and Templates share a verb (`,`/`.`, `b`, `d`, `E`, `ctrl-g`) it lives
  once in `skills`, not twice. The architect fixes the full inventory per view and mode; no
  default changes.
- **D3 — Stacks.** Every M4 mode's stack is a `static` in `keys/views.rs` (narrowest first: view
  context, `skills` for Skills views, the shared layers narrowed to what the mode offers, then the
  global layer — modal for capturing modes). `DECLARED` lists them all with their error phrases.
  Views import their stacks, never declare one. Same rule as M3 D3: T1 owns `keys/`.
- **D4 — Which mode is active.** `SkillsTab::key_stack` returns the active view's stack (an
  open agent help's, else attach's when the pane is open, else Library's or Templates'), and
  `RequirementsTab::key_stack` its mode's. `Tab::key_stack` already exists (M3 D4).
- **D5 — The view switch.** `SkillsTab::on_key` resolves `skills.switch_view` from the active
  stack; captured modes do not offer it (their stack has no `skills` layer), which keeps the
  `captures_input` rule by construction. Pins: `ctrl-l`/`ctrl-h`/`alt-l` do not switch views.
- **D6 — Saving in a `TextArea`.** Every M4 editor resolves `form.save` from its stack **before**
  feeding the widget (boxes'/personas' M3 shape, `boxes.rs:571`). After both lanes merge, T-close
  makes `TextArea::on_key` stop returning `Submit` on `ctrl-s` (it passes the chord like every
  other) and drops the D13 README caveat. Every `TextArea` owner then saves on `form.save` only:
  boxes and personas (already), Library and Templates (L-A, L-B), Requirements (L-C). The two
  Backlog owners (`item_form.rs`, `detail/compose.rs`) check `ctrl-s` themselves before the
  widget today and are M5's; T-close verifies they keep saving, and touches them not at all.
- **D7 — Agent help.** `agent_help.rs` dispatches on the `skills.help` stacks (prompt is
  capturing; proposal is browse with `confirm.yes`/`no` or its own accept/reject verbs, per the
  architect). Its hints become `HintSpec`s. The embedding views pass it keys exactly as today.
- **D8 — `[editor]` loader rules (MOD-57 §6.4).**
  1. `editor.focus` must keep at least one chord, beside `overlay.close` (`load.rs` binding rule
     and `validate::give_way`'s never-gives-way exemption). Error text names the reason ("the
     in-pane editor is left only through it").
  2. `Stack::EDITOR_FOCUSED` and `EDITOR_UNFOCUSED` join `DECLARED` ("in the in-pane editor"),
     so a `[editor]` chord that collides with `global.quit` or `global.help` in the unfocused
     stack is an error (either way: `[editor] abort = "f1"`, `[global] help = ["?", "ctrl-x"]`),
     and `the_compiled_defaults_validate` covers them. A printable `editor.focus` is already
     refused by the `in_capture` rule (`capture_row`), so the collision pins use named chords.
  3. The `ctrl-c` refusal already applies to every table (`chord_of`); a test pins
     `[editor] focus = "ctrl-c"` refused.
  4. Exhaustive `Context` matches already carry `Editor` (they compile); `--print-keys` prints
     `[editor]` and the round-trip test covers it.
- **D9 — Hints.** M3's `HintSpec` elements (`One`, `Pair`, `All`, `Text`). Expected snapshot
  drift is text only (`Ctrl+S` → `Ctrl+s` spelling, separators ` · `, pair joins). Diffs reviewed
  line by line; anything beyond hint and status rows is a defect. Library's trimmed browse hint
  (`library.rs:102`, the 100-column reason in ANA §5.5) stays curated.
- **D10 — Out of M4.** Key prose (M6), `Ctx::new(&Keymap)` (M6), Backlog/Chat views and their
  `ctrl-s` checks (M5), `TextField`'s editing keys and choice-field value keys (Priority's
  `m`/`l`, Activation's Space: ANA §6.1 keeps them fixed), the hierarchy `picker` context (L-C Q3,
  M3 carry, not M4's).
- **D11 — Tests.** Spec-string integration tests stay green unchanged. New pins:
  - `ctrl-l`/`ctrl-h` do not switch Skills views; `l` still does;
  - a rebound `form.save` saves a Library, Templates and Requirements draft, and after T-close
    `ctrl-s` is inert in all of them;
  - per lane, one rebinding test through `Harness::with_keys`: rebound chord acts, old one inert,
    hint shows the new label;
  - `[editor]` loader: `focus = []` refused; `[editor] abort = "f1"` (collides with
    `global.help` in `EDITOR_UNFOCUSED`) refused, and `[global] quit = ["q", "ctrl-x"]` the
    other way; `focus = "ctrl-c"` refused; `--print-keys` round-trips `[editor]`;
  - status line and `?` box for a capturing Templates editor and for the Requirements tree.

## Amendments (maintainer, 2026-10-08, from blueprint §1)

PA-1..PA-4 (`.claude/plans/mod-67-m4-skills-requirements.blueprint.md` §1) approved as written:
T1 also converts `skills/mod.rs` and `skills/agent_help.rs`, adds the `LibraryView`/`TemplatesView`
`key_stack` stubs and `Scroll::apply` in `backlog/detail/mod.rs`, and re-baselines the three
agent-help snapshots, so L-A is `library.rs` + `attach.rs` and no snapshot is shared between lanes
(PA-1); the Skills editors, prompts and agent help keep passing `Tab` through a `TABS` layer before
`MODAL` (PA-2); the widget sees a key before `form.save` resolves, Requirements keeps chords away
from its widgets (PA-3, D6 amended); the editor stacks join `DECLARED` under two phrases and
`[global] quit = ["ctrl-x"]` is refused, the tree's four uses moving to `ctrl-y` (PA-4). The
blueprint's "user-visible beyond D9" list is accepted whole: trimmed Library/Templates browse
hints, `Tab/Down field` on the attach form, agent help `n/Esc discard` and `y` closing an empty
answer, `Down`/`Up` moving focus on the Requirements Priority and Deciding fields.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Catalogue block | `keys/catalogue.rs` `[settings.*]` blocks (M3) | one commented block per context, rows citing the arm they mirror; `ALL`/`position` lists in lockstep |
| Stack statics | `keys/views.rs` `AGENTS_BROWSE`, `CAPTURE`, `MODAL` | `Layer::view` + shared layers + modal global; `DECLARED` phrase per stack |
| View dispatch | `settings/boxes.rs:571` (`form.save` before the widget), `settings/mod.rs` `modal_rest` | first accepted candidate; widget-first for modal modes |
| Tab stack | `settings/mod.rs:359` `SettingsTab::key_stack` | delegate to the active child's stack |
| Hint spec | `settings/personas.rs:85` `Hint::One(Act::FormSave, "create")` | `HintSpec` consts rendered by `Keys::hint` |
| Loader rule | `keys/load.rs:418` `overlay.close` must keep a chord | same shape for `editor.focus` |
| Rebinding test | `tests/keys.rs` + `Harness::with_keys` (M3) | rebound acts, old inert, hint label |
| Integration gate | `#![cfg(feature = "testkit")]` | App-level tests need `--features testkit` (memory) |

## Files to Change

| File | Action | Owner | Why |
|---|---|---|---|
| `crates/htui/src/keys/catalogue.rs` | UPDATE | T1 | D1/D2 contexts, acts, rows |
| `crates/htui/src/keys/views.rs` | UPDATE | T1 | D3 every M4 mode's stack |
| `crates/htui/src/keys/stack.rs` | UPDATE | T1 | `DECLARED` grows (M4 stacks, D8.2 editor stacks) |
| `crates/htui/src/keys/{load,validate,print}.rs` | UPDATE | T1 | D8 `[editor]` rules |
| `crates/htui/src/keys/mod.rs` | UPDATE | T1 | only if D8 or a new context needs it |
| `crates/htui/tests/keys.rs`, `tests/keys_file.rs` (+ fixtures) | UPDATE | T1 | D8 tests, catalogue counts |
| `ui/tabs/skills/{mod,library,attach,agent_help}.rs` | UPDATE | L-A | convert; hint specs; D5 pin |
| `ui/tabs/skills/templates.rs` | UPDATE | L-B | convert; hint specs |
| `ui/tabs/requirements/{mod,forms}.rs` | UPDATE | L-C | convert; hint specs; its own `ctrl-s` check (`mod.rs:233`) becomes `form.save` |
| `tests/skills.rs`, `skills_pg.rs` + `skills__*` snapshots | UPDATE | L-A | pins, re-baseline |
| `tests/templates.rs`, `templates_pg.rs` + `templates__*` snapshots | UPDATE | L-B | pins, re-baseline |
| `tests/requirements.rs`, `requirements_pg.rs` + `requirements__*` snapshots | UPDATE | L-C | pins, re-baseline |
| `crates/htui/src/ui/text_area.rs` | UPDATE | T-close | D6 drop the built-in `ctrl-s` |
| `README.md` ("Changing keys"), `HANDOFF.md`, plan | UPDATE | T-close | D13 caveat gone, limit narrowed to Backlog/Chat; M4 phase note |

## Tasks

TDD per task: tests first, red, then code. Gortex reads; in a worktree native reads/edits are the
fallback.

### Task T1: foundation — serial, primary tree, first
- **Action**: D1-D3 and D8. Every catalogue row and stack for both tabs lands here, from the
  architect's inventory; no view converts (`key_stack` stays `None` for Skills/Requirements). D8's
  loader rules and tests. Catalogue count tests updated.
- **Validate**: `cargo test -p htui --lib keys`, `cargo test -p htui --features testkit --test keys
  --test keys_file`, `cargo insta test -p htui --all-features --check` (no snapshot changes in T1).

### Lanes L-A, L-B, L-C: view conversion — parallel, one worktree each, after T1
Each lane replaces its `key.code` arms with resolver dispatch on its `views.rs` stacks, implements
`key_stack` per mode, turns its `HINT_*` into `HintSpec`s, drops `KeyModifiers` guards the resolver
makes redundant, saves on `form.save` before any `TextArea` (D6), adds its D11 pins and one
rebinding test, re-baselines its snapshots and reviews every diff line.

| Lane | Source | Owned tests | Owned snapshots |
|---|---|---|---|
| L-A | `skills/{mod,library,attach,agent_help}.rs` | `tests/skills.rs`, `skills_pg.rs` | `skills__*` |
| L-B | `skills/templates.rs` | `tests/templates.rs`, `templates_pg.rs` | `templates__*` |
| L-C | `requirements/{mod,forms}.rs` | `tests/requirements.rs`, `requirements_pg.rs` | `requirements__*` |

Lane rules (M3's): never edit `keys/`, `app/`, the registries, `testkit.rs`, `tests/keys*.rs`; a
missing act or stack goes back to the main thread, which amends T1 on the base and the lanes merge
it. Another lane's test file is edited only on an assertion this lane's change breaks, flagged.
Format only own paths. Gate with `env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit
incrementally on `mod-67-m4-<lane>` in `target/wt/<lane>`. Verifiers STRICTLY no edits, no commits.

Known coupling: `agent_help.rs` (L-A) renders inside the Templates editor, so its hint change
moves `templates__agent_help_*` (L-B's). L-A does not re-accept them; T-close re-baselines the
merged tree once and reviews those diffs.

### Task T-close: merge and close-out — serial, primary tree
- Merge lanes `--no-ff` one at a time; full Validation after each; re-baseline shared snapshots on
  the merged tree.
- D6: `TextArea` drops `ctrl-s` (test: `ctrl-s` passes; every `TextArea` owner saves on a rebound
  `form.save` and not on `ctrl-s`; Backlog item form and compose still save on `ctrl-s`).
- Stack-reachability test extended to the M4 stacks.
- README "Changing keys": the D11 limit narrowed to Backlog and Chat; the D13 `TextArea` caveat
  removed.
- HANDOFF M4 phase note (incl. the MOD-57 follow-up closed); this plan's review gate; validator;
  remove worktrees, then lane branches.

Independence: T1 → lanes is a hard chain. Lane source sets are pairwise disjoint; test and
snapshot sets are disjoint by ownership but coupled through `agent_help` (above), so lanes run in
worktrees.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Postgres suites (`*_pg.rs`) once on the fully merged tree (sandbox DB on 5439).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Library (3.4k lines, ~79 arms) dominates L-A | High | L-A is serial inside: agent_help + mod first, then library, then attach; the architect splits library's modes |
| Dropping `TextArea`'s `ctrl-s` silently breaks a save path | Medium | D6 lands only after every owner resolves `form.save` first; T-close test drives every owner; Backlog owners verified unchanged |
| Inventory misses an arm; a lane needs a new act mid-flight | Medium | back to main thread, T1 amended, lanes merge; `the_compiled_defaults_validate` + reachability tests |
| Snapshot churn hides a behaviour change | Medium | D9: only hint/status rows move; full insta run per merge |
| `[editor]` collision rule refuses a file a user already has | Low | only `editor.focus`/`editor.abort` are configurable; defaults validate; the error names the line |
| Choice-field value keys (`m`/`l`, Space) captured by a new act | Low | D10 keeps them widget-owned; L-C pins the Priority field unchanged |

## Acceptance

- [ ] Skills (tab, Library, Templates, attach, agent help) and Requirements dispatch through
      declared stacks; no `KeyCode` match remains outside widget-owned keys (D10)
- [ ] All 24 `HINT_*` constants in these files are hint specs
- [ ] Skills `h`/`l` modifier blindness fixed and pinned
- [ ] `form.save` is the only save chord in every `TextArea` editor; `TextArea` no longer claims `ctrl-s`
- [ ] `[editor]` loader rules (D8) in place and tested
- [ ] Validation passes on the merged tree; snapshot diffs are hint/status text only
- [ ] Patterns mirrored, not reinvented

## Review gate (2026-10-08)

`rust-reviewer` over `b8d5faaf..ef746383`: approve with changes; 0 CRITICAL/HIGH/MEDIUM, 4 LOW,
5 NIT. Fixed in R1 (`572e0a21`..`4802a157`, one implement/verify workflow, verified clean).

- **Applied**: L1 `SKILLS_TAB` was unreachable once both Skills views always returned a stack, and
  its `DECLARED` entry named a mode no user is in ("in Skills"); removed (maintainer), the views'
  `key_stack` return `Stack`, `DECLARED` 57 → 56, pinned by
  `a_global_chord_on_the_view_switch_names_the_skills_views`. L2 the Templates browse hint is
  `h/l view` again (maintainer), through a new `Hint::Two` (first two admitted chords). L3 the
  Requirements test pins `key_stack` per mode instead of a test-only helper. L4 the HANDOFF M4 note.
  NITs: the unreachable `Submit` in `requirements/forms.rs`, the `form.save` catalogue comment, README
  names `[requirements]`, the agent help Answered hint gains `Enter close`.
- **Carried**: `Ctrl+S`/key prose in Library, Templates, agent help and Requirements messages (M6,
  D10).
- **Deviations recorded during implementation**: agent help resolves its own state's acts before it
  refuses the editor's verbs (blueprint L-A §6.2 had the refusal first; equal under the defaults,
  and a legal `[form] save = "enter"` no longer blocks the help's accept, PA-3's reasoning);
  `VIEW_DEFAULTS` rows landed with the stacks (T1d), not with the acts (T1a); L-B touched
  `tests/editor_pane.rs` (two `Ctrl+S save` assertions) and L-C `tests/reveal.rs` (one hint-text
  check), both flagged and merged without conflict; the README Backlog example became
  `[global] quit = ["f"]`, pinned by `the_readme_backlog_example_loads_and_f_stays_the_backlogs`.
- **Process**: T1 in two workflows (keys half, views half), three worktree lanes as concurrent
  workflows, merged `--no-ff` with no conflict, T-close and R1 workflows; every verify round 1 had 0
  blocking findings, lows fixed by hand or in T-close. Merged-tree gate with Postgres green; after
  R1 3007 htui tests serial. Pre-existing: full runs leave an empty `~/.config/htui/trees` (the
  worker runtime's default scratch root, `htui-worker/src/runtime.rs` `Shared::singletons`, used by
  a test that sets no `with_scratch_root`); removed after each run, not fixed here.

## Verified claims (plan fact-check, 2026-10-08)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `skills/agent_help.rs` exists, is outside ANA §8's file list, and matches keys | **true** | 1448 lines, 39 `KeyCode::` mentions, 4 `KeyModifiers::`; module doc: `Ctrl+G` help in both editors |
| 2 | 24 snapshots in M4 areas, not ~21 | **true** | `tests/snapshots`: `skills__` 9, `templates__` 8, `requirements__` 7 |
| 3 | 24 `HINT_*` constants | **true** | `const …HINT…:` count: attach 4, library 8, templates 5, requirements/mod 7; agent_help, forms, detail, tree 0 |
| 4 | `requirements/{detail,tree}.rs` match no keys | **true** | 0 `KeyCode::`, 0 `KeyModifiers::` in both |
| 5 | ~214 `KeyCode` mentions in the converted files (was "~240 arms") | **amended** | agent_help 39, attach 19, library 79, skills/mod 1, templates 23, requirements/forms 7, requirements/mod 46 |
| 6 | The Skills view switch ignores modifiers (`ctrl-l` switches) | **true** | `skills/mod.rs` `SkillsTab::on_key`: `matches!(key.code, Char('h'\|'l'\|'['\|']') \| Left \| Right)`, no modifier test |
| 7 | No Skills/Requirements view declares a stack yet | **true** | `fn key_stack` hits: registry, settings, overlays only |
| 8 | `Tab::key_stack` and `Harness::with_keys` exist (M3) | **true** | `ui/tabs/registry.rs:79`; `testkit.rs:185`, `:703` |
| 9 | Boxes and personas resolve `form.save` before their `TextArea` | **false → amended (PA-3)** | they feed the `TextArea` first and resolve `form.save` on its `Pass` (`boxes.rs:533-575`, `personas.rs:1670-1678`); found by the architect |
| 10 | Library and Templates editors save only through `TextArea`'s `Submit` (`ctrl-s`) | **true** | `library.rs:1069` and `templates.rs:729` docs ("`Ctrl+S` (the area's `Submit`)"), `Submit` arms `library.rs:1102`, `templates.rs:764` |
| 11 | Requirements checks `ctrl-s` itself before its widgets | **true** | `requirements/mod.rs:23` doc, `:233` `Char('s' \| 'S')` with CONTROL |
| 12 | Backlog `TextArea` owners check `ctrl-s` themselves, so D6 does not break them | **true** | `item_form.rs:8`, `:272-275` (A6 "Ctrl+S is checked before a chord passes"); `compose.rs:6`, `:316` |
| 13 | `TextArea` owners are exactly boxes, personas, library, templates, requirements forms, item_form, compose | **true** | `\bTextArea\b` users: those plus `cells.rs`, `theme.rs`, `ui/mod.rs`, `editor.rs`, `templates.rs` (crate root), none of which feed it keys (doc/re-export/type mentions) — T-close re-checks with `relations usages` |
| 14 | The loader requires a chord only for `overlay.close` | **true** | `load.rs:418`; `validate::give_way` exempts only `OverlayClose` (`validate.rs:33`) |
| 15 | `EDITOR_FOCUSED`/`EDITOR_UNFOCUSED` are not in `DECLARED` | **true** | `stack.rs:210+` lists BASE, OVERLAY, views stacks only; editor stacks used in `app/pane.rs` only |
| 16 | `EDITOR_UNFOCUSED` admits `global.quit` and `global.help` unfiltered | **true** | `stack.rs:201`: `Layer::all(Editor)`, `Layer::only(Global, &[Quit, Help])` |
| 17 | `ctrl-c` is refused in every table, `[editor]` included | **true** | `load.rs:241` `chord_of` is the only chord parse for entries (`:409`), table-agnostic |
| 18 | `editor.focus` is `in_capture` (printable refused already) | **true** | `catalogue.rs` `capture_row(Act::EditorFocus, Editor, "focus", &["ctrl-4"], …)` |
| 19 | `--print-keys` and table names already cover `Editor` | **true** | `print.rs:27` iterates `Context::ALL` (includes `Editor`, `catalogue.rs:97`); `load.rs:462` `TABLES` names `editor` |
| 20 | Lane source sets are pairwise disjoint | **true as files, false as a build boundary → amended (PA-1)** | Files to Change: each tab file in exactly one lane; T1 touches only `keys/` and `tests/keys*.rs` |
| 21 | Lane test/snapshot sets are disjoint by construction | **false → amended** | `agent_help.rs` (L-A) renders in `templates__agent_help_*` (L-B); `tests/{concepts_search,personas,reveal,settings}.rs` reference the Skills/Requirements tabs or hints. Ownership + worktrees + T-close re-baseline replace the claim |
| 22 | Repo disk has room for three worktree targets | **true** | `df -h .`: 2.9 T free on `/dev/sda`; `target/` 2.9 G; 12 cores |
