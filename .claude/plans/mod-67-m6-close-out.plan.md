# Plan: MOD-67 M6 — Close-out: legacy keymap gone, key prose generated, the last views converted

**Source**: `HANDOFF.md` MOD-67 (incl. the TUI design review findings 1 and 3); spec `docs/ANA-26.md`
§8 (M6 row) and its risk table ("M6's test rejects a stack action no view handles"); carried from
`.claude/plans/mod-67-m3-settings-overlays.plan.md`, `mod-67-m4-skills-requirements.plan.md` and
`mod-67-m5-backlog-chat.plan.md` ("Review gate", "Carried", M5 D11)
**Routing**: plan (maintainer accepted 2026-10-13; C4 only, low confidence, the same call as
M1-M5). Ultracode for **implement** only; `code-architect` and `rust-reviewer` stay plain, unpinned
agents. Sandbox run (`HR_SANDBOX=1`, branch `hr/MOD-67`, base `8349e793`).
**Selected milestone**: M6, the last. MOD-67 closes with it (lifecycle P2).
**Complexity**: Large (the `Ctx::new` change alone is 42 sites in 17 files; ~25 prose sites in ~16
files; two unconverted views; with Q1 (a) every tab's hint line and ~165 TUI snapshots move)
**Status**: confirmed 2026-10-13 (Q1: (a), Q2: (a), the recommended options, taken on a plain
"confirm")

## Summary

The `Keymap`/`Binding`/`KeyScope` module and the paths that still carry it go: `Ctx::new` takes
`&Keys`, `App::new` takes no `Keymap`, and `App::on_key`'s legacy tab step and `render_help`'s legacy
branch are deleted. Every sentence that names a key a user can rebind gets that key from
`Keys::label` on the stack it is said in, so a rebound key shows up in the message too. The last two
views that match raw keys — the queue overlay (added by MOD-12 M3, after M3 converted the overlays)
and the hierarchy directory picker (the M3 carry) — dispatch through declared stacks. `FIXED` gains
the M3/M4 forms' choice keys; the validator reports a line and chord once across its passes; a unit
test proves every catalogue action is offered by some stack and handled by some view; and
`docs/htui-keys.md` is generated from `--print-keys` and pinned by a test. With Q1 (a), every tab's
generated hint moves into one fixed row the shell draws above the status line (design review
finding 3).

## Scope drift since ANA-26 (fact-checked, see table)

ANA §8 sized M6 at "~15 files" with `Ctx::new` "22 call sites in 11 files". The tree has **42** call
sites in **17** files (most in test modules), and `Ctx` also carries a `keymap` field next to the
`keys` that `.with_keys()` sets. Two views no carry list names are still on raw `KeyCode` arms with
no modifier check: `ui/overlay/queue.rs` (`QueueOverlay`, `j`/`k`/`J`/`K`/`P`/`Q`/`Enter`, its hint
a literal `"j/k move · J/K reorder · …"`) and `ui/path_picker.rs` (`PathPicker`, 13 arms; hierarchy
puts its `Picking` mode on `views::CAPTURE`). Both are modifier-blind (`ctrl-j` moves). The legacy
`Keymap` has no live rows left (M5 removed the last eight); what remains is plumbing, docs and its
own tests.

## Decisions (proposed; confirmed at the CONFIRM gate)

- **D1 — The legacy keymap goes (ANA §8).** Delete `crates/htui/src/keymap.rs` and `pub mod keymap`.
  `Ctx::new(scope, projects, top_bar, keys: &Keys, theme, origin, emit)`: the `keymap` field and
  `Ctx::with_keys` go (every production caller already chains `.with_keys`; test benches pass
  `Keys::compiled()` or their own). `App::new(requests)` drops its `Keymap` argument (5 callers),
  `App.keymap` goes, and so do `App::on_key`'s `self.keymap.resolve(&KeyScope::Tab(..))` step
  (`app/state.rs:907`) and `render_help`'s legacy branch (`:1043`). Paths spelled
  `crate::keymap::KeyChord` move to `crate::keys::KeyChord`; doc links to `KeyScope` in
  `ui/tabs/registry.rs`, `ui/overlay/registry.rs`, `settings/agents.rs`, `keys/{chord,catalogue,hint}.rs`
  are rewritten. `Tab::key_stack`'s doc ("`None` while the tab is not converted") is updated; the
  trait default stays `None` for the test doubles. No behaviour change, no snapshot change.
- **D2 — The queue overlay (new finding).** A `queue` context (`[queue]` table, `?` heading `Queue`)
  for the overlay's own verbs: reorder down/up (`J`/`K`), pause/resume (`P`), dequeue (`Q`), open
  (`Enter`); `j`/`k` are the shared `list.down`/`list.up`. One stack in `keys/views.rs`
  (`QUEUE_OVERLAY`: the `queue` view layer, `list`, the overlay layer, the global layer as the other
  overlays compose it), in `DECLARED`. Its hint becomes a `HintSpec`, `key_stack` returns the stack,
  and the arms match resolved acts. Defaults unchanged. The architect fixes act names and layer
  order against the four M3 overlays.
- **D3 — The directory picker (M3 L-C Q3).** A `picker` context (`[picker]`, heading `Picker`) for
  `PathPicker`'s verbs: open (`Enter`, `l`), up (`h`, `Backspace`), choose (`s`), choose this
  directory (`S`), go to (`/`), toggle hidden (`.`), cancel (`Esc`); `j`/`k` are `list.*`. The go-to
  field captures (its keys stay the widget's, ANA §6.1). Hierarchy's `Picking` mode leaves
  `views::CAPTURE` for `HIERARCHY_PICKER` (browse) and `CAPTURE` only while the go-to field is open.
  `PathPicker::on_key` takes the resolved acts (or `&Keys` + stack; the architect picks the seam
  that keeps `path_picker.rs` free of `Ctx`). Its two hint halves become `HintSpec`s; the M3
  `HINT_PICKING` section hint follows.
- **D4 — Key prose (ANA §8; M3 L3, M4 D10, M5 D11 carries).** Rule: a sentence that names a key bound
  to a catalogue action takes it from `ctx.keys().label(stack, act)` (pattern:
  `chat/mod.rs:510`, `ChatEnd`); with the action unbound, the sentence drops the key clause rather
  than naming a dead key. A key the widget owns (D13 in M3: a form field's `Enter`/`Esc`, the
  `Hint::Text("Enter save")` rows) or a `FIXED` key stays literal, and the fact table records why.
  Constants become functions of the label (`EDITED` in `editor.rs` is shared by the item form and the
  Skills views). Inventory, each to be confirmed by the architect against the stack in force:
  `editor.rs` `EDITED` (`Ctrl+S saves`); `concepts_search.rs` `NO_MATCHES` (`Ctrl+R`);
  `divergence.rs` `HINT_NO_CONFLICT` (`m or t`), `Ctrl+S compares again`, `Ctrl+S saves the
  resolution`; `compose.rs` `Ctrl+S tries again`; `chat/mod.rs` `Esc again ends this chat`;
  `chat/transcript.rs` `(t to unfold)`; `connection.rs` `REBUILD_KEYS` (`press Enter or R`);
  `hierarchy.rs` `RELOADED` (`press p again`); `personas.rs` `Ctrl+S retries`, `Esc again discards`;
  `prompt.rs` and `settings/queue.rs` `NOT_A_VALUE_ROW` (`` `e` edits``); `agent_help.rs`
  `Esc leaves it first`, `Ctrl+S saves it`; `attach.rs` `y detaches` (question and hint);
  `library.rs` `Esc again discards`, `press e`, `Ctrl+S saves them`; `templates.rs`
  `Ctrl+S again saves anyway`, `Esc again discards`, `press e`, `Ctrl+S saves them as v…`; and any
  Runs sentence M5 D11 named. Out: stderr prompts before the TUI (`press Enter` in `lib.rs`,
  `worker_cmd.rs`, `provision/mod.rs`) and the D13 widget keys. Each family gets one pin: rebound
  action → the sentence names the new key.
- **D5 — `FIXED` for the M3/M4 forms (M5 R1 carry).** `FIXED` rows for the Requirements form's
  priority keys (`m`, `l`, `space`, `left`, `right`, `esc`; `requirements/forms.rs:325-333`) and the
  attach form's `space` (and its `enter`/`esc`, `attach.rs:271-280`); the architect sweeps the other
  M3/M4 stacks for keys matched ahead of the resolver (the go-to/picker of D3 included) and lists
  every one, each row citing its arm. `FIXED`'s doc ("M5's views only") is updated.
- **D6 — One report per line and chord (M5 R1 carry).** A user line that is both a collision and on
  a fixed key is reported once: the validator's passes dedupe by (file line, chord), first pass
  wins. Pinned by a test with such a line.
- **D7 — Reachability (ANA §8 M6 row and risk table).** One unit test in `keys/views.rs`: every
  `CATALOGUE` act is admitted by some `DECLARED` stack (or is `global`/`overlay`/`editor` and
  admitted by `BASE`/`OVERLAY`/the editor stacks), and every act a declared view layer admits is
  named as `Act::X` in some view's production code (the `every_view_stack_is_named_by_a_view`
  include-and-strip technique, extended to `queue.rs` and `path_picker.rs`). The compiled defaults
  validate already (`validate.rs:380`); the test asserts it once more per stack only if the
  architect finds a gap.
- **D8 — The key reference page (ANA §8).** `docs/htui-keys.md`: a short header (where the file
  lives, how to regenerate) and the exact output of `htui --default-keys --print-keys` in one
  ` ```toml ` block (see Q2). A test in `tests/keys_file.rs` (or `keys/print.rs`) compares the
  block with `keys::print(&Keys::defaults())` and fails naming the regenerate command. README
  "Changing keys" links it.
- **D9 — Carried print lows (M3 review, T1).** The `(changed)` mark and the collision line of a view
  row pinned to its compiled derivation while its shared row is rebound: the architect reproduces
  both; each is fixed in T1 if the fix stays inside `keys/print.rs`/`validate.rs`, else recorded as
  accepted in the review gate. No maintainer call needed unless a fix changes `--print-keys` output
  for a file that does not pin view rows.
- **D10 — The fixed hint row (design review finding 3; M5 Q1 (a) deferred it here).** See Q1.
- **D11 — README and docs.** "Changing keys" gains `[queue]` and `[picker]`, links
  `docs/htui-keys.md`, and loses any "not configurable yet" sentence left; key tables re-checked
  against the defaults.

## Open questions (maintainer, at CONFIRM)

- **Q1 — The fixed hint row for every tab.** Today each converted view draws its own generated hint
  line where it always did (107 `keys.hint(..)` call sites in 33 files): Settings at the section's
  foot, Skills at the pane's, Backlog at the list pane's and the Runs pane's, Chat in its header.
  - **(a) Recommended:** M6 builds it. `Tab` gains a defaulted `hint(&self, ctx) -> Option<String>`
    (the focused mode's `HintSpec` on its stack); the shell gives it one row above the status line
    (`layout::chrome`); views stop drawing their own hint lines (overlays keep theirs, they are
    boxes over the screen). One task after the lanes, serial, since it touches every tab file. Cost:
    every TUI snapshot with a tab body moves (~165), reviewed line by line. MOD-81 and MOD-82 were
    sequenced after MOD-67 for exactly this rewrite.
  - (b) Not in MOD-67: M6 closes with every hint generated where it sits; finding 3's "one fixed
    row" moves to MOD-82 (shared pane chrome), noted in both entries. Snapshot drift is the prose
    sites and the two converted views only.
- **Q2 — The reference page's shape.** (a) **Recommended:** the literal `--print-keys` output in one
  `toml` block — one source, copy-paste ready, round-trips, trivially pinned. (b) One Markdown table
  per context (table, action, keys, what it does) from a second printer — easier to read on the
  forge, a second format to keep right.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Stack per mode | `settings/hierarchy.rs:512` `HierarchySection::stack`; `keys/views.rs` consts | `match self.mode` → `views::*` const |
| Dispatch | `settings/hierarchy.rs:528` `on_browse_key` | `for act in ctx.keys().actions(stack, chord)`, first accepted act wins |
| Overlay stack | `ui/overlay/{waiting_list,concepts_search}.rs` `key_stack` (M3) | overlay view layer over `Stack::OVERLAY`'s layers |
| Hint spec | `settings/connection.rs` `HINT_CONFIRM` | `HintSpec` consts, `ctx.keys().hint(stack, SPEC)` |
| Prose label | `chat/mod.rs:510`, `chat/mod.rs:313` | `keys.label(stack, Act::X)`, clause dropped when unbound |
| Fixed keys | `keys/stack.rs:354` `FIXED` | one row per stack, citing the arm |
| Source-scan test | `keys/views.rs` `every_view_stack_is_named_by_a_view` | `include_str!` up to `#[cfg(test)]`, comments dropped |
| Rebinding test | `tests/keys.rs`, `Harness::with_keys` (`testkit.rs`) | rebound acts, old inert, hint/prose names new label |
| Integration gate | `#![cfg(feature = "testkit")]` | App-level tests need `--features testkit` |

## Files to Change

| File | Action | Owner | Why |
|---|---|---|---|
| `crates/htui/src/keymap.rs` | DELETE | T0 | D1 |
| `crates/htui/src/{lib.rs,testkit.rs}`, `app/{state,update,pane,mod}.rs` | UPDATE | T0 | D1 `Ctx::new`, `App::new`, legacy step and help branch |
| 15 view files with `Ctx::new` in test benches: `ui/overlay/{concepts_search,queue,quit_confirm,waiting_list}.rs`, `ui/tabs/backlog/{mod}.rs`, `backlog/detail/{compose,graph,requirements,runs}.rs`, `chat/mod.rs`, `requirements/mod.rs`, `skills/{agent_help,library,templates}.rs` | UPDATE | T0 | D1, mechanical |
| `ui/tabs/registry.rs`, `ui/overlay/registry.rs`, `settings/agents.rs`, `keys/{chord,catalogue,hint}.rs` | UPDATE | T0 | D1 doc links |
| `crates/htui/src/keys/{catalogue,views,stack,validate,print,mod}.rs` | UPDATE | T1 | D2/D3 contexts, acts, stacks, `DECLARED`; D5 `FIXED`; D6; D7; D9 |
| `crates/htui/tests/{keys,keys_file}.rs` | UPDATE | T1 | counts, D6, D8 pin |
| `docs/htui-keys.md` | CREATE | T1 | D8 |
| `ui/overlay/queue.rs`, `tests/queue_overlay.rs`, `queue_overlay__*` | UPDATE | L-A | D2 |
| `ui/path_picker.rs`, `settings/hierarchy.rs`, `tests/hierarchy.rs`, `hierarchy__picker` | UPDATE | L-B | D3, and `hierarchy.rs` `RELOADED` (D4) |
| `editor.rs`, `backlog/item_form.rs` (the `EDITED` call), `overlay/concepts_search.rs`, `backlog/divergence.rs`, `detail/compose.rs`, `chat/{mod,transcript}.rs`, `settings/{connection,personas,prompt,queue}.rs`, `skills/{agent_help,attach,library,templates}.rs`, their unit tests and `tests/*.rs` pins | UPDATE | L-C | D4 |
| `ui/layout.rs`, `ui/tabs/registry.rs` (`Tab::hint`), `app/state.rs` render, every tab view's render, all tab snapshots | UPDATE | T2 (Q1 (a) only) | D10 |
| `README.md`, `HANDOFF.md`, `docs/ANA-26.md` (amend §8 counts), `docs/decisions/` | UPDATE | T-close | D11, close-out (P2) |

## Tasks

### Task T0: legacy keymap removal — serial, primary tree, first
- **Action**: D1 exactly; `cargo build` drives the call-site list. No behaviour change.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui
  --all-features --check` (no snapshot changes); featureless clippy.

### Task T1: keys foundation — serial, primary tree, after T0
- **Action**: D2/D3 catalogue rows, `ALL`/`position` lists, stacks, `DECLARED`; D5 `FIXED`; D6
  dedupe; D7 reachability test (its `queue.rs`/`path_picker.rs` half lands red-gated: `#[ignore]`
  with reason until L-A/L-B merge, un-ignored in T-close); D8 page and pin; D9. Catalogue count
  tests updated.
- **Validate**: `cargo test -p htui --lib keys`, `cargo test -p htui --features testkit --test keys
  --test keys_file`, insta check (no snapshot changes).

### Lanes L-A, L-B, L-C — parallel, one worktree each, after T1
Each lane converts or rewrites only its files (table above), TDD (pins first), adds one rebinding
test, re-baselines only its own snapshots, and commits incrementally on `mod-67-m6-<lane>` in
`target/wt/<lane>`. Verifiers make no edits and no commits. Gate per lane:
`env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features -- --test-threads=1`.

| Lane | Source | Owned tests | Owned snapshots |
|---|---|---|---|
| L-A | `ui/overlay/queue.rs` | `tests/queue_overlay.rs`, unit tests in file | `queue_overlay__opened` |
| L-B | `ui/path_picker.rs`, `settings/hierarchy.rs` | `tests/hierarchy.rs`, unit tests in files | `hierarchy__picker` (+ any `hierarchy__` hint row) |
| L-C | D4 prose files (table above) | pins in the owning test files | only snapshots whose text is a D4 sentence |

Known couplings: `tests/settings.rs` and `tests/connection.rs` register several sections (L-B and
L-C may both touch a hierarchy/connection assertion — flagged, merged by hand); `keys/views.rs` is
T1's only, lanes never edit it (a stack change goes back to T1 as a fix commit first).

### Task T2: the fixed hint row — serial, primary tree, after the lanes merge (Q1 (a) only)
- **Action**: D10 per Q1 (a); the architect's blueprint fixes the trait method, which pane's hint
  Backlog shows (focused pane, list keys after it when they fit), and the row's height rule.
- **Validate**: full `cargo insta test` re-baseline, reviewed line by line; only hint/status rows
  and the body shrinking by one row may move.

### Task T-close — serial, primary tree
- Merge L-A, L-B, L-C `--no-ff` (then T2); un-ignore D7's half; full Validation after each merge.
- README (D11); ANA-26 §8 M6 row counts amended; HANDOFF: MOD-67 closed (P2), DECISIONS index,
  `docs/decisions/mod/mod-67.md` write-up; this plan's review gate; validator; worktrees and lane
  branches removed.

Independence: T0 → T1 → lanes → T2 → T-close is a hard chain. Lane source sets are pairwise
disjoint (L-B owns `hierarchy.rs`, so its `RELOADED` prose is L-B's, not L-C's); test files overlap
only as noted.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Postgres suites once on the merged tree (sandbox DB on 5439); workspace gate at the end (the
"zeta" literal check and other crates' tests only show there). Known pre-existing failure:
`htui-worker` `auto_queue::a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` (MOD-95).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Q1 (a) snapshot churn hides a behaviour change | High | T2 alone moves rows; reviewed line by line; only hint/status rows and a one-row body shrink accepted |
| A prose label taken from the wrong stack (names a key the mode does not answer) | Medium | D4 pins per family under a rebound key; architect names the stack per site |
| New `queue`/`picker` contexts collide with an existing global or list default in their stack | Medium | the compiled-defaults validation runs over the new stacks in T1; collisions fixed before lanes |
| A user file that loads today is refused (new stacks in `DECLARED`) | Low | only `[global]`/shared entries on `queue`/`picker` keys; README says so; error names the stack |
| `Ctx::new` change ripples outside `htui` | Low | `Ctx` and `App` are `htui`-only (callers checked: `lib.rs`, `testkit.rs`, `app/*`) |
| Three worktrees' disk | Low | ~10 GB `target` each; 3.0 TB free, 12 cores |

## Acceptance
- [ ] `keymap.rs` gone; `Ctx::new` takes `&Keys`; no legacy tab step or help branch
- [ ] queue overlay and directory picker dispatch through declared stacks; no non-widget `KeyCode`
      arm left in a view (`ui/text_area.rs`, `ui/text_field.rs` and `FIXED` keys aside)
- [ ] every key-naming sentence follows `keys.toml` (D4 pins)
- [ ] `FIXED` covers the M3/M4 forms; one report per line and chord
- [ ] reachability test green; `docs/htui-keys.md` pinned
- [ ] Q1 outcome done (fixed row, or MOD-82 carries it)
- [ ] Validation passes on the merged tree; MOD-67 closed out
- [ ] Patterns mirrored, not reinvented

## Verified claims (plan fact-check, 2026-10-13)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `Ctx::new` has 22 call sites in 11 files (ANA §8) | **false → amended (42 in 17)** | text search `Ctx::new(`: state 7, update 5, library 7, templates 6, backlog/mod 5, and one each in testkit, 4 overlays, compose, graph, detail/requirements, runs, chat, requirements, agent_help |
| 2 | `Ctx` holds both `keymap` and `keys`; production callers chain `.with_keys` | **true** | `app/state.rs:87`, `:104-131`; `App::ctx` `:926-936` |
| 3 | No `Keymap` row is bound anywhere outside `keymap.rs` tests | **true** | `app/mod.rs` `register_all` binds nothing; only `bind(` outside `keymap.rs` is `app/pane.rs:1725` (test) |
| 4 | Legacy step and help branch exist | **true** | `app/state.rs:907` `keymap.resolve(&KeyScope::Tab(id), …)`; `:1043` `keymap.help_line` |
| 5 | `App::new` has 5 callers, all in `htui` | **true** | `lib.rs:232`, `testkit.rs:129`, `app/pane.rs:957`, `app/state.rs:1091`, `app/update.rs` (3 incl. `:651`) |
| 6 | No `tests/*.rs` file names `htui::keymap` | **true** | 5 hits, all prose in comments (`backlog.rs:267`, `replay.rs:203`, `settings.rs:969`, `:1454`, `backlog_keys/l_a.rs:122`) |
| 7 | The queue overlay matches raw keys, modifier-blind, no `key_stack` | **true** | `ui/overlay/queue.rs:437-478` `match key.code`; `fn key_stack` absent from `queue.rs`; hint literal `:44` |
| 8 | The path picker matches raw keys, modifier-blind; hierarchy picks on `CAPTURE` | **true** | `ui/path_picker.rs:141-…` 13 `KeyCode::`, 0 `KeyModifiers::`; `settings/hierarchy.rs:516` `Mode::Picking { .. } => views::CAPTURE` |
| 9 | Every other production `Tab`/`Overlay`/`SettingsSection`/`DetailTab` declares `key_stack` | **true** | `fn key_stack` per file vs `impl … for` list; `body.rs` relies on the `DetailTab` default (scroll only, M5 D1) |
| 10 | `DECLARED` has 78 stacks | **true** | `keys/stack.rs` test `assert_eq!(DECLARED.len(), 78)` |
| 11 | The compiled defaults already validate | **true** | `keys/validate.rs:380` `assert_eq!(validate(Keys::compiled()), [])` |
| 12 | No test checks every catalogue act is offered by a stack | **true** | `keys/` tests: `every_act_is_listed_and_has_a_row`, `every_act_has_exactly_one_row` (rows only), `every_view_default_is_an_extra_chord_some_stack_offers` (view defaults only) |
| 13 | `FIXED` lists M5 stacks only | **true** | `keys/stack.rs:351-353` doc, 6 entries all `views::{RUNS_*,BACKLOG_FILTER,ITEM_FORM,CHAT_*}` |
| 14 | Requirements priority and attach Space are matched ahead of the resolver | **true** | `requirements/forms.rs:325-333`; `skills/attach.rs:271-280` |
| 15 | `Keys::label(stack, act)` exists and prose already uses it once | **true** | `keys/hint.rs:100`; `chat/mod.rs:510` (`ChatEnd`), `:313` (`Back`) |
| 16 | ~25 key-naming sentences remain (D4 inventory) | **true (candidates)** | literal scan of non-test string literals; `/tmp` inventory, 123 hits of which the D13 widget `Hint::Text` rows and stderr prompts are excluded |
| 17 | `--default-keys` and `--print-keys` combine | **true** | `cli.rs:49` `--keys` conflicts with `default_keys`; `lib.rs:164-177` loads (honouring `default_keys`) then prints |
| 18 | No generated doc exists in `docs/` to mirror | **true** | `docs/` listing; no `include_str!` of a `docs/` file in `crates/` |
| 19 | 107 `keys.hint(..)` sites in 33 files (Q1 cost) | **true** | text search `.hint(` in `ui/`, `app/` |
| 20 | ~165 TUI snapshots in `tests/snapshots` (+29 in `src/`) | **true** | 170 files, 5 of them `state_cli_pg__` (CLI); `find src -name '*.snap'` 29 |
| 21 | Lane source sets are pairwise disjoint | **true** | Files to Change: `hierarchy.rs` in L-B only (its `RELOADED` moved from L-C); `queue.rs` in L-A only; L-C's list excludes both |
| 22 | Lane test sets are disjoint | **false → amended** | `tests/settings.rs`/`tests/connection.rs` register several sections; ownership + worktrees + hand merge replace the claim (as M3/M4) |
| 23 | Disk and cores for three worktrees | **true** | `df -h .`: 3.0 T free on `/dev/sda`; `target/` 3.0 G; `nproc` 12 |
