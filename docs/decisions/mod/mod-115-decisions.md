# MOD-115 - TUI shell: the decisions so far, milestones 1 to 3 (in progress, 2026-10-22)

**Requirements:** `R-TUI-1`, `R-TUI-10`, `R-TUI-12`, `R-TUI-13`, `R-TUI-18`.
**Origin:** the UI redesign review (`docs/ui-redesign-findings-2026-10-15.md` §3, §9), filed
2026-10-17 (`8ed3bb07`).
**Artifacts:** PRD `.claude/prds/mod-115-tui-shell.prd.md`; M1
`.claude/plans/mod-115-tui-shell-m1.plan.md` and `.claude/plans/mod-115-tui-shell-m1.blueprint.md`;
M2 `.claude/plans/mod-115-tui-shell-m2.plan.md` and
`.claude/plans/mod-115-tui-shell-m2.blueprint.md` (the blueprint's B1-B7 and PLAN DEVIATIONS
D-1..D-16 are the record of what was built where it differs from the plan).
**User guide:** README "Changing keys"; `docs/htui-keys.md` (every action and its default,
generated).

MOD-115 is still open in `HANDOFF.md`, so this is not its close-out write-up: that one is written
under `docs/decisions/mod/` with the item's own name when the item closes
(`.claude/rules/workflow-docs.md`, lifecycle step 3), and it folds this file in. A write-up with
the item's name needs an index line in `DECISIONS.md`, and an index line for an open item is a
reused ID, so the decisions so far live here (blueprint D-16). `R-TUI-10` was amended from this
record (`docs/REQUIREMENTS.md`, 2026-10-18).

## Milestone 1 (by reference)

M1 (`168f35b3`..`18c5357a`, 2026-10-17/18) built the widget kit, the one-row header and the about
panel. Its maintainer decisions of 2026-10-17 are in the M1 blueprint, "Maintainer decisions": Help
keeps `?` and `F1` and the about panel is `F9`, a `global.about` catalogue row like any other (so
a `keys.toml` binding `f9` elsewhere clashes with it, the general rule M2 keeps as MD3); the
about panel's state lived in memory in M1 and M2 persists it (D1); the testkit starts with it
hidden. The `HANDOFF.md` phase 1 note summarises the milestone.

## Milestone 2: maintainer decisions (2026-10-18)

- **MD1 - every key is configurable through the keymap.** The `FIXED` register and its validator
  pass are gone (`8ceed1e1`). Its keys are rows: `[field]` (`submit`, `cancel`, the cursor, delete,
  line and page keys; blueprint D-8 added the last four for `TextArea`), `[choice]` (`prev`,
  `next`, `toggle`), `[answer]` (`option_1`..`option_9`) and `requirements.priority_must` /
  `priority_later`. Text widgets keep matching canonical events; the shell translates a `[field]`
  chord into its row's canonical event while the active stack captures text (`keys/translate.rs`,
  plan D7). A printable character typed into a field is text, the one exception. A guard test,
  `crates/htui/tests/no_hardcoded_keys.rs` (`cf1af6d5`), fails on any `KeyCode::` outside test
  items and its allow-list: `keys/chord.rs`, `keys/translate.rs`, the three text widgets
  (`ui/text_field.rs`, `ui/text_area.rs`, `ui/tabs/chat/composer.rs`, reached only through the
  translation), `editor/pane.rs`'s `encode_key` (bytes for the PTY), and
  `provision/secret_prompt.rs`, which is out of the rule: a raw-tty password prompt that runs
  before the keymap loads and outside the TUI.
- **MD2 - `ctrl-c` is an ordinary chord with a safety net.** It is a default of `global.quit` and
  `leave.confirm` (the first press opens the leave dialog, the second leaves), passes modal and
  capture layers like an F-key, and binds like any key (`[global] find = "ctrl-c"` takes it from
  quit with a notice). The loader refuses a file that leaves `global.quit` or `leave.confirm`
  with no chord; `leave.stay` and `field.cancel` joined `overlay.close` and `editor.focus` in the
  must-keep set (blueprint D-10), and a must-keep row gives a chord away only while it keeps
  another (`[global] help = ["q"]` loads; quit keeps `ctrl-c`). A focused in-pane editor still
  receives `ctrl-c`; `editor.focus` is the way out.
- **MD3 - the clash rule stays.** A `[global]` chord cannot be bound to an action in a narrower
  table wherever a screen offers both; the error names the screen (`[list] down = ["f9"]`: `"f9"
  is already global.about (default) in Settings > Agents`). M1's `F9` note is this rule. Under
  MD4 a version-1 file is held to it only for chords that were global before M2: a global chord
  M2 added (`ctrl-pgdn` on `global.next_tab`, `tab` on `global.next_pane`) gives way with a notice.
- **MD4 - existing overrides keep loading.** `keys.toml` is `version = 2`; a file without
  `version` is version 1. In a version-1 file a default M2 added or changed (`since: 2`) gives
  way to the file's entry wherever a declared stack meets them, with the existing notice; a
  version-1 quit keeps its implicit `ctrl-c` unless the file moved `ctrl-c` to another row
  (D-9, `86960947`); `--print-keys` prints the version the keys were read under. A version-2 file
  gets the strict rules.
- **MD5 - where layout and history live:** plan D1, below.
- **MD6 - the leave dialog promises nothing about work surviving:** plan D6, below.
- **MD7 - `Esc` is the one key that is never configurable.** A key file cannot list it, with any
  modifier (`esc always closes, cancels or goes back and cannot be bound`); every row whose
  compiled default holds `Esc` keeps it whatever the file says, so a list replaces only that
  row's other chords and `[]` unbinds only those (`--print-keys` marks such rows `Esc always`). A
  view's override of a shared verb keeps the shared row's `Esc`, and a kept `Esc` never takes
  `Esc` from a view's own row (`4082bb55`), so `Esc` does on every screen what it did before M2.
  With MD4, a version-1 file that lists `Esc` on a row that holds it anyway (the README's old
  `close = ["esc", "f2"]`) still loads; `Esc` on any other row is refused in either version. The
  translation passes `Esc` through untouched. This settles D-13 for `Esc`.
- **MD8 - mouse capture on the list + detail tabs.** Backlog, Requirements and Skills turn capture
  on, so a click focuses a pane and dragging a pane border resizes it (saved when the drag ends);
  Chat and Settings keep the terminal's plain text selection. This settled blueprint D-6. With
  capture on the terminal stops turning the wheel into arrow keys, so over a claimed pane of a tab
  with no gesture of its own the shell maps the wheel to that tab's `list.down` / `list.up` chord
  and focuses the pane (D-15); the Runs flow keeps its own wheel.

## Milestone 2: design decisions, as built

- **D1 - UI state per machine, history in memory.** `<config root>/ui-state.toml` (beside
  `keys.toml` and `box.toml`), versioned, written atomically on every layout or about change,
  holds the about panel and each tab's layout, with a `[modal.*]` section reserved for
  `R-TUI-12`'s modal sizes and no reader yet. A missing file is a first launch (about on); an
  unreadable or invalid one falls back to defaults with a header flash and never stops htui
  starting. The testkit uses an in-memory store. Not Postgres: the layout depends on the
  terminal, changes on every resize key and must work before the store connects; MOD-109 moves the
  keys, not this file. History stays in memory for the session (`R-TUI-13` does not ask for more).
- **D2 - layout model.** Four presets (list and detail; with the activity panel underneath; three
  columns; the detail alone), split sizes, list and activity shown or hidden, zoom. Splits use
  ratatui's own constraints, so the defaults reproduce the old splits cell for cell: Backlog
  55/45, Requirements 45/55, Skills a 32-column list (blueprint D-3: a list size is a percentage
  or a cell count). The activity slot is a placeholder until MOD-122. A hidden pane says how to
  bring it back, naming only keys that act there. Tabs claim their panes through `Ctx::split`
  instead of the shell drawing them (B5, D-2), so a tab's notices, filter rows and full-width
  modes stay its own.
- **D3 - pane focus is shell state**, one focused pane per tab, handed to the tab in `Ctx`; only
  the focused pane gets the accent border and the bottom-border hints; on a single-pane tab the
  pane keys do nothing.
- **D4 - new rows and default changes.** `global.next_pane` / `prev_pane` (`tab`, `ctrl-right` /
  `backtab`, `ctrl-left`); `global.next_tab` / `prev_tab` moved to `ctrl-pgdn` / `ctrl-pgup`
  (digits and clicks remain the main way between tabs); `[layout]` (`p` / `P` presets, `>` / `<`
  list, `}` `ctrl-down` / `{` `ctrl-up` activity, `f2` `alt-1` / `f3` `alt-2` show or hide, `z`
  zoom); `[history]` (`backspace`, `alt-left` back, `alt-right` forward); `[leave]`
  (`enter`, `ctrl-c` / `esc`, `n`). Layout and history are their own contexts, not `[global]`
  (B1): `[layout]` sits only on the list + detail browse stacks, `[history]` on every tab's browse
  stack, and neither on a capturing or modal stack, so no layout or history key acts inside a
  form. Changed in the build:
  - *Shift arrows are listed last, not first* (D-15): `pane.next_subtab` is `l`, `]`,
    `shift-right` (likewise `prev_subtab`, `settings.*_section`, `skills.switch_view`), so the
    first label in every hint and `?` box stays `h` / `l`. The keys that act are the plan's;
    plain `left` / `right` no longer change sub-tab. D-15 asks for a maintainer call on the order.
  - *A capturing or confirming mode never switches tabs* (D-15, B2 widened): `MODAL` admits
    neither the pane keys nor the tab keys, since `ctrl-pgdn` would otherwise leave a form
    mid-edit. A `next_tab` the user rebinds to an F-key, ctrl or alt chord therefore no longer
    switches tabs from a form, except in the Skills modes that hand the tab keys to the shell.
    D-15 asks for a maintainer call.
  - *`p` and `P` in Backlog* (D-1): in Backlog › Runs and › Prompt `p` is the sub-tab's own and
    `P` pauses the queue everywhere in the Backlog (reviewed `SHADOWING` pairs), so no default
    cycles presets there; `layout.next_preset` stays rebindable.
- **D5 - history.** An entry is the tab, sub-tab, pane and an opaque cursor the tab gives and takes
  back; a change of tab, sub-tab or pane pushes the place left (cursor included) and drops what was
  forward; a cursor move alone is not recorded; the cap is 50. The header names where back goes
  (`⌫ back to …`) and shows `Alt→ forward` when there is somewhere forward, through
  `KeyChord::short_label` (D-12).
- **D6 - the leave dialog.** It replaced the quit question, and `global.quit` always opens it. It
  reads `Leaving closes htui. Work it is running stops with it:` over the open workspace's
  working and waiting counts and an open in-pane edit (`unsaved changes are lost`), or `Nothing is
  running. Leave htui?`; `Enter` or a second `ctrl-c` leaves, `Esc` or `n` stays. There is no
  "stop all" choice until MOD-116/MOD-125: leaving already stops the in-process work
  (`RunRuntime::shutdown` aborts after a grace window), so it would do the same thing. `R-TUI-18`
  ("leaving never stops work") is therefore not met by M2; it stays a must, carried by MOD-116
  and MOD-125. The abort question passes `global.quit` to the shell so `ctrl-c` opens the dialog
  over it, and `Esc` there returns to the question (D-15).
- **D7 - every key configurable:** MD1 and MD2 above. `ctrl-c` reaches every overlay through a
  `QUIT` layer (`global ∩ {quit}`, modal-filtered) placed just before each overlay stack's help
  layer (B4), not a pre-dispatch branch. That layer acted from Task 0, before the hardcoded
  branch went in Task 6, so from T0 a quit rebound to a ctrl, alt or named chord already quit over
  the modal overlays (D-14); D-14 asked for a maintainer call, and the run's main thread accepted
  it and kept the layer. **Known limit (D-13):** the translation drops a canonical key whose
  `[field]` row moved off it only where nothing else on that screen binds the key; where another
  action does (`choice.prev` keeping `left` after `field.left` moved), the key is delivered and a
  focused widget still reads it literally, so a rebound field key other than `Esc` keeps working
  under its old chord there. Closing it needs `&Keys` in the widgets, which the plan rejected as a
  25-file signature change; it closes as M3/M4 port the tabs onto the kit. The reverse (review
  RV1 M) is fixed for every choice: a rebound field chord arrives as the canonical key, and the
  filter, item and Requirements forms and the Skills attachment activation resolve their own
  acts from the chord pressed (`Ctx::pressed`), so `field.left = ["ctrl-b"]` never fires
  `choice.prev`. The other forms, the
  compose areas, the concepts search and the agent help still resolve from the delivered key, so
  a rebound `field.line_up`/`line_down` chord a one-line field passes moves the field or cursor
  there as `Up`/`Down` would; that closes with the same M3/M4 port.
  Hints likewise (RV1 repair 2): the sites RV1 converted name the submit key in force, but the
  other text-field screens (the Skills naming, import, template and agent-help fields, the
  Settings forms, the concepts search) still print a literal `Enter ..`, so after `[field]
  submit = "ctrl-t"` their hint row names `Enter`, not the key in force; those screens were
  never in the `FIXED` register, and they move to `Hint::One` with the same M3/M4 port.
  Settings > Agents' pasting hint (`Enter sends · Esc cancels`, `settings/agents.rs`
  `HINT_PASTING`) is one of them. Deferring these literal texts to M3/M4 awaits the
  maintainer's confirmation (review RV4).
  The choice hints that were literal arrows (`←/→ choose` on the item form, `←/→ resolution`
  on the Runs close-out warning) use a new hint element, `Hint::Glyphs`: the first chord of
  each side written as the header's chips write it (`KeyChord::short_label`), because the
  existing `Pair` writes `Left/Right` and would have changed those hints and their snapshots.
  Under the compiled keys they read as before; a rebound side shows its own chord
  (`←/Ctrl+n choose`).
- **D8 - existing key files keep loading:** MD4 above. Review RV1 H1: a version-1 entry never
  takes a chord from the rows M2 added for the keys a view kept before it (a text field's editing
  keys, a choice's value keys, the permission digits, the Requirements priority); the two share
  it, and the view's key acts on its screen as before M2. The share is judged per screen
  against the keys each view kept then: where the view kept the key, the file was refused
  before M2 and still is (`[backlog.filter_form] clear = ["left"]`); where it kept none, the
  choice, answer and priority keys step aside on that screen only, so `[confirm] yes =
  ["space"]` still continues the Runs close-out warning. The same step-aside rule governs
  dispatch (`Keys::actions` in `keys/stack.rs` calls it, an RV1 edit outside its planned file
  set, as is `ui/tabs/backlog/mod.rs` passing the chord pressed to the filter and item forms),
  and, since review RV4, the `?` box: it lists only what acts, so under that file the warning's
  box reads `Space yes` and no longer also `Space toggle`.

## Milestone 3: group A on the kit (2026-10-20..22)

Plan `.claude/plans/mod-115-tui-shell-m3.plan.md` (fact-checked, confirmed 2026-10-20) and blueprint
`.claude/plans/mod-115-tui-shell-m3.blueprint.md` (C1-C7, PLAN DEVIATIONS D-1..D-18). Settings (all
ten sections), Requirements and Chat draw only through `ui/kit`; Backlog, Skills, the overlays, the
in-pane editor and the `?` box are group B (M4).

- **Grouping (maintainer, 2026-10-20).** Group A by size and snapshots: Settings, Requirements, Chat
  (~20.5k production lines, ~91 snapshots); group B the rest (~20.9k, ~85). Backlog goes last with
  its split, since every shell and overlay snapshot draws over it. The `settings/agents.rs` split
  moved from M4 to M3 (`settings/agents/{mod,table,form,flows}.rs`, `951b8495`, a pure move).
- **D1 - a ported tab draws its own panes.** `Tab::draws_chrome` (default `false`): the shell skips
  the list/detail focus restyle and the M1 hint adapter for it, keeps the hidden-pane note and the
  activity placeholder (C2). `Ctx::pane_focus` and `Ctx::accent` give a ported pane its focus and
  the tab's accent. M4 deletes the flag with the adapter.
- **D2 - neutral homes.** `wrapped` → `kit::text`, `key_line` → `kit::chips`, `modal_rest` →
  `kit::modal`; every importer updated in T0 with no snapshot moving.
- **D3 - kit additions.** `kit::form` (one label column, value kinds, gutter and band, help under
  the focused row, errors under their row, stacked fields for text areas, `value_width`,
  `focus_lines`, heading indent), `kit::filter` (the band with `n of m`, cut with `…` rather than
  to a wrong number), chip overflow with `‹`/`›` and click rects from the same computation,
  `Pane::inner` and scroll spans that name the rows they measure, `kit::label` (a glyph and a word
  for every enum group A shows, `plural`).
- **D4 - Settings is one pane with its sections as chips**, the section's name carried by the active
  chip when the strip overflows (D-10). Still `Panes::Single`; list + detail is MOD-123's call.
- **D5 - Chat is one pane titled by the session**; permission keys stay digits (`a`/`A`/`r` is
  MOD-121); plan entries, stop reasons and tool kinds read as words.
- **D6 - Requirements** draws its tree and detail as kit panes, the filter as the tree's band, its
  forms through `kit::form`. Maintainer D-14: the tree's hint lists the write keys first
  (`a area · n new · e amend · W withdraw · / filter …`).
- **MD-A - D-13 closes on every group-A screen.** Forms resolve their acts from `Ctx::pressed`;
  every hint and every sentence that names a key builds it from `Keys` (C5, C6). Confirms the RV4
  deferral: group B's literal `Enter …` hints close with M4.
- **Copy (maintainer, 2026-10-21).** Prompt's reserve reads `10% (0.1)` with the bp range in the
  help; the Agents table drops `models`, then `billing`, then `transport` below 98 cells so the name
  keeps 10; a missing login reads `⚠ needs login`, and a manually set verdict shows the icon only
  (`⚠*`, review answer 2026-10-21). Settings › Prompt keeps its registry keys as labels (they are
  the names users write); only type jargon became plain words. Writes in flight read in words in
  every section, never a request name.
- **D-15 calls (maintainer, 2026-10-20):** shift arrows stay listed after `h`/`l`; `MODAL` keeps
  the tab keys out of forms.
- **C7 - under an overlay a ported pane drops focus; an unported one keeps the shell accent**
  (review L2, recorded, no code change). A ported tab answers `Unfocused` through
  `Ctx::pane_focus` while an overlay covers it, so its border goes faint and its hints go; group B
  is still restyled by the shell's `paint_focus` until M4 ports it. The ported behaviour is the
  target.
- **Guard.** `crates/htui/tests/kit_chrome.rs` fails on any `Block::`/`Borders::` (aliases
  included) outside test code in group A, and on any file outside the kit and group A that its
  group-B list does not name; `every_m4_entry_still_frames_itself` is how M4 shrinks the list.
  The scanner is shared with `no_hardcoded_keys.rs` (`tests/token_scan/mod.rs`).
- **Chat's unbound-compose line** ("bind [chat] compose to write the first prompt") names
  `keys.toml`; MOD-109 moves keys into Settings and repoints it.

## Commits

- Plan and blueprint: `b0701b9e` (plan, fact-checked and confirmed), `71397d69` (blueprint),
  `b0743083` (MD7, MD8).
- T0 key catalogue surface, version 2, stubs: `5ccf2c59`..`334763f4`.
- T1 UI-state file: `24dfa9ad`, `7f56e30b`. T3 history model: `920f1dc1`. T4 leave dialog:
  `aafda6ad`. Merged together at `7e720691`.
- T2 layout engine and kit: `3d960e5f`..`f52d3d2f`, merged at `cf4ac852`.
- T5 every key through the keymap, safety nets, `Esc`, guard: `5db75c66`..`276d15f9`; repair
  `4082bb55`, `29d20855`.
- T6 shell wiring (panes, layout, history, persistence, leave dialog, header chips, default
  flips, mouse): `38209991`..`e4f048c3`.
- M3 plan and blueprint: `c93584ec` (plan), `d9d32aa7` (blueprint), `aede0a2d` (maintainer
  answers).
- M3 T0 seam, helper homes, stubs: `893851bd`..`5986a5eb`. T1 kit form, filter, chip overflow:
  `e1259651`..`ed1a27a9`. T2 labels: `32d82afc`..`b932f4e2`. T3 Settings frame:
  `e13353df`..`58fa9460`.
- M3 lanes, merged one at a time with the gate on each merged tree: wave A L-chat `6e0fdfb4`,
  L-kinds `8c00937c`, L-agents `19777380` (split `951b8495`), L-req `c5325278`; wave B L-prompt
  `2c93e071`, L-sec `041d6f86`, L-hier `8e4568ba`.
- M3 T4 guard and transitional deletions: `7ba6de1c`..`76778523`. Review (rust-reviewer, three
  areas): fixes RV1-RV3 merged at `d2188c87`, RV4 `8f7124ed`..`848ce83e`.

## Open at this record

- M4 (group B): Backlog with the `backlog/mod.rs` split, Skills, the overlays, the in-pane editor
  and the `?` box onto the kit; then the M1 hint adapter (`paint_shell_hints`, `join_panes_under`)
  and `Tab::draws_chrome` go, `kit_chrome`'s group-B list empties, and MOD-81/82/83/85 close. D-13's
  field-key limit and the literal `Enter …` hints remain on group-B screens until then.
- Group B's own copy (link kinds, gate labels, Backlog statuses) gets its labels with its port.
- `R-TUI-18`: MOD-116/MOD-125. `R-TUI-10`'s Postgres-stored overrides: MOD-109 (which also repoints
  text naming `keys.toml`, e.g. Chat's unbound-compose line).
