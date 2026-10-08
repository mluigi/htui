# Plan: MOD-67 M3 — Settings and overlays dispatch through context stacks

**Source**: `HANDOFF.md` MOD-67; spec `docs/ANA-26.md` §6, §7.2-§7.6, §8 (M3 row); carried from
`.claude/plans/mod-67-m2-keys-file.plan.md` "Review gate" (D7 narrower override, D8 step 5
shadowing kind) and from `.claude/plans/mod-67-m1-catalogue-resolver.plan.md` "Review gate"
(overlay-aware, capture-filtered status line)
**Routing**: plan (maintainer accepted 2026-10-07; rule gave plan with C4 only, low confidence on
C2, the same call as M1/M2). Ultracode for **implement** (C4) and **review** (one verifier per
reviewer finding); `rust-reviewer` stays the gate.
**Selected milestone**: M3 only. M4-M6 are later runs.
**Complexity**: Large (~24 source files, ~65 hint constants, up to ~80 snapshots)
**Status**: confirmed (maintainer, 2026-10-07)

## Summary

Every Settings section (all ten), the Settings tab's own section cycling and all four overlays
stop matching `key.code` and dispatch named actions through declared context stacks. Their 65
`HINT_*` constants become hint specs rendered by `Keys::hint`. The status line and the `?` box
follow the active view's stack: overlay-aware, filtered in a capturing or confirming mode, and the
box lists every key of the focused view. The two ANA-26 §2.6 routing defects are fixed by
construction and pinned by tests: modifier-blind browse arms (`ctrl-d` deletes in hierarchy and
kinds, `ctrl-l` cycles sections) and the Qdrant editor passing `Tab` to the tab bar. The key file
gains the two M2 carry-overs: a narrower context may override a shared verb, and the validator
allows reviewed shadowing pairs. No default changes except ANA §6.6's `Down`/`Up` aliases in
agents and hierarchy.

## Scope drift since ANA-26 (fact-checked, see table)

ANA §8 sized M3 as "seven sections, three overlays, 37 constants, ~11 files, ~46 snapshots". The
tree now has **ten** sections (`personas` MOD-26, `secrets` MOD-10, `queue` MOD-12 joined), **four**
overlays (`waiting_list` MOD-69 joined), **65** hint constants and **~80** snapshots in these areas.
M3 takes all of them: leaving the three new sections on `KeyCode` would keep the D11 "a view eats a
rebound key" hole open in Settings, which is what M3 closes. Prose that names keys
(`connection.rs` "press Enter or R", `hierarchy.rs`, `settings/mod.rs`) stays M6's, as ANA §8 has it.

## Decisions (proposed; confirmed at the CONFIRM gate)

- **D1 — Contexts.** One `Context` per section, named `settings.<section id>` (`settings.agents`,
  `settings.boxes`, `settings.connection`, `settings.hierarchy`, `settings.kinds`,
  `settings.personas`, `settings.prompt`, `settings.qdrant`, `settings.queue`, `settings.secrets`),
  `settings` for the tab (`next_section` `["l", "]", "right"]`, `prev_section` `["h", "[", "left"]`),
  and one per overlay: `concepts`, `switcher`, `migration`, `waiting`. `?` box headings are the
  section's or overlay's title (`Connection`, `Settings`, `Workspaces`, ...). Each block is
  appended to `CATALOGUE` in strip order, citing the arm it mirrors (M1's convention).
- **D2 — Naming (ANA §6.2).** Shared verbs where the meaning is shared (`common.edit/new/delete/
  clear/reload/back/dismiss`, `list.*`, `confirm.*`, `form.*`). Splits: `settings.agents.probe`
  (`r`), `settings.boxes.probe` (`p`), `settings.boxes.edit_quirks` (`e`), `settings.boxes.edit_tags`
  (`t`), `settings.connection.rebuild` (`R`). `kinds`' `g` (open graph) is a view verb, not
  `list.top`. The architect's blueprint fixes the full inventory per section and mode; no default
  changes beyond D12.
- **D3 — Where stacks live.** Every M3 mode's stack is a `static` slice of `Layer`s in a new
  `keys/views.rs` (narrowest first: view context, `settings` for sections, the shared layers narrowed
  with `Layer::only` to what the mode offers, then the global layer). `DECLARED` lists them all, each
  with its error phrase ("in Settings > Connection", "in the Connection editor", ...). Views import
  their stacks; they never declare one. This keeps `keys/` owned by T1 and the lanes out of it.
- **D4 — Which mode is active.** `SettingsSection`, `Tab` and `Overlay` each gain
  `fn key_stack(&self) -> Option<Stack<'static>>`, defaulted to `None` ("not converted yet": the
  shell behaves as today). `SettingsTab::key_stack` returns the active section's. `App` gains
  `active_stack()`: the top overlay's, else the active tab's, else `Stack::BASE`. M4/M5 implement
  `key_stack` for their tabs.
- **D5 — The modal global layer.** `Layer::modal(Context::Global)`: in a capturing or confirming
  mode the global layer admits only chords with CONTROL or ALT, and function keys. That is today's
  MOD-52 pass-through (CONTROL), plus `F1` for help (ANA §6.5) and ALT (`TextField` already passes
  it; ANA §2.6 item 3). `Tab`, `BackTab`, `Enter`, `Esc`, arrows and characters never reach the
  global layer from such a mode. That fixes defect 2 by construction: the Qdrant editor's `Tab`
  no longer switches tabs. `actions`, `label`, `hint`, the validator and the status line all apply
  the layer's chord filter.
- **D6 — Dispatch in a view.** A converted view takes `ctx.keys().actions(STACK, chord)` and handles
  the first candidate it accepts. It returns `Pass` for a global-layer act or a declined one, and
  in a modal mode gives the rest to its widget and then consumes. `App::on_key`'s last step
  resolves through `active_stack()` instead of `Stack::BASE`, and the overlay step through the
  overlay's stack instead of `Stack::OVERLAY`. The layer filter is therefore enforced once, in the
  shell. `ctrl-c` stays checked first (M1). That fixes defect 1 by construction: chord equality
  includes modifiers, so `ctrl-d` is not `d` and `ctrl-l` is not `l`.
- **D7 — Status line.** It renders the global layer of `active_stack()` with that layer's filter,
  so it stops advertising `q quit` where `q` is text. `quit` is always shown, as `Ctrl+c quit`
  when its own chords are filtered out. With a converted overlay up it shows the overlay stack's
  global layer (`? help`). An unconverted view (`key_stack` = `None`) keeps today's line.
- **D8 — The `?` box.** One line per layer of `active_stack()`, narrowest first, every bound chord
  of every admitted action, layer filter applied, under the context's heading. The legacy tab line
  stays only for tabs with no stack (Backlog, until M5). The closing line is unchanged. That answers
  design-review finding 1 for Settings and the overlays.
- **D9 — Hint elements.** `Hint` gains `All(act, text)` (every chord, joined by `/`:
  `n/Esc cancel`) and `Text(&'static str)` (fixed keys ANA §6.1 keeps out of the catalogue, a
  widget's `Enter`/`Esc`, and plain notes: `Enter store`, `typed text is never shown`). Every
  label goes through `KeyChord::label` and every separator is ` · `. Expected snapshot drift is
  text only: `ctrl-s` → `Ctrl+s`, `n / Esc` → `n/Esc`, `\u{b7}` already renders as `·`. Snapshot
  diffs are reviewed line by line, and anything beyond hint and status rows is a defect.
- **D10 — Narrower override (M2 D7 carry-over).** `[<view context>] <name>` where `<name>` is not
  the view's own action resolves against the **shared** contexts (`list`, `pane`, `confirm`,
  `form`, `common`, plus `settings` for a section) that sit below that view context in some
  declared stack. It creates a `(view context, act)` row carrying the line, which `resolve_row`
  and `actions` already prefer (narrowest wins). `Keys::set` appends instead of no-op'ing.
  `--print-keys` prints override rows in the view's table, and the round-trip test covers one.
  Unknown names list the view's own names, then the overridable shared names. `global` and
  `overlay` names cannot be overridden per view.
- **D11 — Shadowing allow-list (M2 D8 step 5 carry-over).** `SHADOWING: &[(Act, Act)]`, a
  (narrower, wider) pair. A collision in a declared stack is allowed only when the pair is listed,
  the narrower act's layer comes first in that stack, and the chord is a catalogue default of both
  (PA-2's per-chord rule). Seed: `form.next_field`/`global.next_tab` and
  `form.prev_field`/`global.prev_tab`, plus only the pairs `the_compiled_defaults_validate` demands
  once the M3 stacks exist. Every entry is reviewed and commented, and the test drives the list.
- **D12 — Per-view default variations.** One catalogue row per `Act` stays (`Act::spec`,
  `every_act_has_exactly_one_row`). A view that needs extra default chords on a shared act gets a
  compiled override in a new `VIEW_DEFAULTS: &[(Context, Act, &[&str])]` table. `Keys::defaults`
  applies it, and `--print-keys` does not mark it `(changed)`. Uses: the agents and hierarchy forms'
  `Down`/`Up` on `form.next_field`/`prev_field` (today's behaviour), and the migration prompt's
  `Y`/`N`. ANA §6.6's `Down`/`Up` list aliases for agents and hierarchy need no entry, because
  `list.down`/`list.up` already default to `["j", "down"]`/`["k", "up"]` (a deliberate addition).
- **D13 — Out of M3.** `TextArea`'s built-in `ctrl-s` submit (`text_area.rs`) and `TextField`'s
  editing keys stay fixed. The `TextArea` submit moves to `form.save` in M4, with the Skills
  editors (ANA §8 M4 "`form.*` actions"). Until then a rebound `form.save` in the boxes quirks/probe
  editors adds a chord, but `ctrl-s` still saves too, and README's D11 sentence says so. Also out:
  the prose key names (M6), `Ctx::new(&Keymap)` (M6) and the legacy `Keymap` Backlog rows (M5).
- **D14 — Tests.** The spec-string integration tests stay green unchanged, since defaults don't
  move. New pins:
  - `ctrl-d` in hierarchy and kinds browse deletes nothing;
  - `ctrl-l`/`ctrl-h` do not cycle sections;
  - `Tab` in the Qdrant editor keeps the tab;
  - `Down`/`Up` move the agents and hierarchy lists;
  - per lane, one rebinding test through a new `Harness::with_keys(Keys)`: the rebound chord acts,
    the old one is inert, and the hint shows the new label;
  - one narrower-override test (`[settings.boxes] reload = ["f5"]`): boxes reloads on `F5`,
    another section still on `r`;
  - status-line and `?` box tests for a capturing Settings editor and for an overlay.

## Amendments (maintainer, 2026-10-08, from blueprint §1)

PA-1..PA-9 (`.claude/plans/mod-67-m3-settings-overlays.blueprint.md` §1) approved as written:
additive `VIEW_DEFAULTS` (14 rows), `SHADOWING = [(ConfirmNo, OverlayClose)]` only,
`Layer::view` auto-admitting the stack's shared verbs, the context collision pass skipping view
contexts, widget-first dispatch with `Stack::passes` for modal modes, ctrl+capital folding in
`KeyChord::new`, one shared `views::CAPTURE`, `active_stack() -> Option<Stack>`, quit always first
on the status line. Concepts search hint: short labels (`Ctrl+p scope · Ctrl+r index`,
`Up/Down`).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Catalogue block | `keys/catalogue.rs` `CATALOGUE` `[list]`…`[common]` blocks | one commented block per context, each row citing the arm it mirrors; `ALL`/`position` test lists extended in lockstep |
| Stack constants | `keys/stack.rs` `Stack::BASE`, `Stack::OVERLAY`, `DECLARED` | `static` layer slices, `(phrase, stack)` pairs |
| Resolver dispatch | `app/state.rs` `App::apply_keys` | `keys.actions(stack, chord)` → first accepted candidate |
| Shared helper promoted for sections | `settings/mod.rs` `CHANGED_ELSEWHERE`, `wrapped` | one helper in `settings/mod.rs`, each section imports it |
| Per-section bench | `testkit.rs` `key(&self, section: &mut dyn SettingsSection, chord)` | section unit tests drive spec strings through a bench `Ctx` |
| App-level key test | `tests/keys.rs` (`Harness`, `common::mock_keyring()` first when a DSN is stored) | spec-string keys, status line / `?` box assertions |
| Integration gate | `tests/backlog.rs:6` `#![cfg(feature = "testkit")]` | App-level cases need `testkit` (memory: integration tests need testkit) |
| Errors | `keys/load.rs` `KeyFileError { line, message }` | unknown-name messages list valid names |

## Files to Change

| File | Action | Owner | Why |
|---|---|---|---|
| `crates/htui/src/keys/catalogue.rs` | UPDATE | T1 | D1/D2 contexts, acts, rows; `SHADOWING`, `VIEW_DEFAULTS` |
| `crates/htui/src/keys/views.rs` | CREATE | T1 | D3 every M3 mode's stack |
| `crates/htui/src/keys/stack.rs` | UPDATE | T1 | `Layer::modal` (D5), filter in `actions`; `DECLARED` grows |
| `crates/htui/src/keys/{mod,hint,load,validate,print}.rs` | UPDATE | T1 | D9 hints, D10 override rows, D11 shadowing, D12 defaults, filter-aware label/status/help |
| `crates/htui/src/app/state.rs` | UPDATE | T1 | D4 `active_stack`, D6 final step, D7 status line, D8 `?` box |
| `crates/htui/src/ui/tabs/registry.rs` | UPDATE | T1 | `Tab::key_stack` (default `None`) |
| `crates/htui/src/ui/overlay/registry.rs` | UPDATE | T1 | `Overlay::key_stack` (default `None`) |
| `crates/htui/src/ui/tabs/settings/mod.rs` | UPDATE | T1 | `SettingsSection::key_stack`, tab cycling via `settings.*` acts |
| `crates/htui/src/testkit.rs` | UPDATE | T1 | `Harness::with_keys` |
| `crates/htui/tests/keys.rs`, `tests/keys_file.rs` (+ fixtures) | UPDATE | T1 | D5-D8, D10, D11 tests |
| `settings/agents.rs`, `settings/qdrant.rs` | UPDATE | L-A | convert; hint specs |
| `settings/connection.rs`, `boxes.rs`, `prompt.rs`, `queue.rs` | UPDATE | L-B | convert; hint specs |
| `settings/hierarchy.rs`, `settings/kinds.rs` | UPDATE | L-C | convert; hint specs; `ctrl-d` pins |
| `settings/personas.rs`, `settings/secrets.rs` | UPDATE | L-D | convert; hint specs |
| `overlay/{concepts_search,waiting_list,workspace_switcher,migration_prompt}.rs` | UPDATE | L-E | convert; hint specs |
| lane test files + `tests/snapshots/*.snap` | UPDATE | per lane | see Tasks |
| `README.md` ("Changing keys"), `HANDOFF.md`, `docs/decisions/` | UPDATE | T7 | D11-limit sentence narrowed to Backlog/Chat/Skills/Requirements; phase note |

## Tasks

TDD per task: tests first, red, then code. Gortex reads; in a worktree, native reads/edits are the
fallback (memory: Gortex in a worktree).

### Task T1: foundation — serial, primary tree, first
- **Files**: the T1 rows above.
- **Action**: D1-D12 in `keys/` and the shell. Every catalogue row and stack for all ten sections
  and four overlays lands here, from the architect's inventory. No view converts: all `key_stack`
  return `None` except `SettingsTab`'s own cycling (that one defect pin lands here). New tests:
  `the_compiled_defaults_validate` over the grown `DECLARED`; the `Layer::modal` filter; the D10
  round-trip; D11 allow/deny; D12 defaults not `(changed)`; status line and `?` box from a test
  stack (a `Probe` overlay returning a stack). Catalogue count tests updated.
- **Validate**: `cargo test -p htui --lib keys`, `cargo test -p htui --features testkit --test keys
  --test keys_file`, and `cargo insta test -p htui --all-features --check` (no snapshot may change
  in T1).

### Tasks L-A … L-E: view conversion lanes — parallel, one git worktree each, after T1 merges
Each lane: replace its `key.code` arms with D6 dispatch on its `keys/views.rs` stacks, implement
`key_stack` per mode, turn its `HINT_*` constants into `HintSpec`s (D9), drop the per-view
`KeyModifiers::CONTROL` guards the resolver makes redundant, add its D14 pins and one rebinding
test, re-baseline its snapshots and review every diff line.

| Lane | Source | Owned tests | Owned snapshots |
|---|---|---|---|
| L-A | `agents.rs`, `qdrant.rs` | `tests/settings.rs`, `probe.rs`, `auth.rs`, `install.rs` | `settings__*`, `probe__*` |
| L-B | `connection.rs`, `boxes.rs`, `prompt.rs`, `queue.rs` | `tests/connection.rs`, `box_settings.rs`, `prompt_settings.rs`, `queue_settings.rs` | `connection__*`, `box_settings__*`, `prompt_settings__*`, `queue_settings__*` |
| L-C | `hierarchy.rs`, `kinds.rs` | `tests/hierarchy.rs`, `kinds.rs` | `hierarchy__*`, `kinds__*` |
| L-D | `personas.rs`, `secrets.rs` | `tests/personas.rs`, `secrets_settings.rs` | `personas__*`, `secrets_settings__*` |
| L-E | the four overlays | `tests/concepts_search.rs`, `waiting.rs`, `shell.rs`, `integration.rs` | `concepts_search__*`, `waiting__*`, `shell__switcher_*`, `shell__migration_prompt`, `src/ui/overlay/snapshots/*` |

Lane rules:
- Never edit `keys/`, `app/`, the registries, `testkit.rs` or `tests/keys*.rs`. A missing action
  or stack goes back to the main thread, which amends T1 on the base, and the lanes merge it in.
- Another lane's test file is edited only on an assertion this lane's change breaks, and the lane
  report flags it.
- Format only your own paths (`rustfmt <files>`).
- Gate with `env -u HTUI_TEST_DATABASE_URL` (Postgres suites skip) and `--test-threads=1`.
- Commit incrementally on the lane branch.
- Worktrees go under `target/wt/<lane>` on a named branch `mod-67-m3-<lane>` from the T1 merge.
- Adversarial verify per lane (Workflow): mutations only in a `/tmp` worktree with its own
  `CARGO_TARGET_DIR`.
- **Validate (lane)**: its section/overlay lib tests, its owned integration test files with
  `--features testkit`, `cargo insta test -p htui --all-features --check` after its accepted
  re-baseline.

### Task T7: merge and close-out — serial, primary tree
- **Action**: merge lanes `--no-ff` one at a time, with no lane running. Run the full Validation
  block after each merge; the snapshot re-baseline is verified on the real tree, as ANA §8
  requires. Then:
  - a unit test that every `DECLARED` stack is reachable from some view's `key_stack`;
  - README "Changing keys" (the D11 limit narrowed, D13's `TextArea` note);
  - HANDOFF M3 phase note;
  - this plan's review gate;
  - workflow-docs validator;
  - `git worktree remove --force` each lane, then delete the lane branches.

Independence: T1 → lanes is a hard chain (every lane compiles against T1's catalogue and
`views.rs`). The lanes' source sets are pairwise disjoint (verified). Their test and snapshot sets
are disjoint by ownership, but not by construction, so they run in worktrees, never on the shared
tree.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate (memory)
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --check   # after each accepted re-baseline
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Postgres-backed suites (`*_pg.rs`) run once on the fully merged tree (sandbox DB on 5439).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The architect's inventory misses an arm, so a lane needs a new act mid-flight | Medium | lane rule: back to the main thread, amend T1, lanes merge it; the `the_compiled_defaults_validate` + reachability tests catch stacks that drift |
| A lane's hint change breaks an assertion in another lane's test file | Medium | assertion-only edits, flagged; merge conflicts resolved by T7, gates on the merged tree |
| Snapshot churn hides a behaviour change | Medium | D9: only hint/status rows may move; every diff reviewed; full insta run per merge (memory: grep undercounts) |
| `Layer::modal` drops a chord a section relied on passing (a ctrl chord a form used) | Low | D5 keeps every CONTROL chord; lane pins the section's existing ctrl tests unchanged |
| Five worktree builds contend CPU/disk | Medium | 12 cores, 2.8 TB free on the repo disk; worktrees under `target/wt/`; clear `target/debug/incremental` first |
| Shadowing list grows into a blanket waiver | Low | D11: per chord, narrower-first, defaults only, each entry commented and reviewed |
| `agents.rs` (2.9k lines, many modes) dominates L-A | High | L-A is a single lane by itself plus the small `qdrant.rs`; the blueprint splits agents' modes explicitly |

## Acceptance

- [ ] All ten sections, the Settings tab and the four overlays dispatch through declared stacks; no
      `KeyCode` match remains in them outside widget-owned keys (D13)
- [ ] All 65 `HINT_*` constants in these files are hint specs
- [ ] Defects 1 and 2 pinned by tests; `Down`/`Up` move agents and hierarchy lists
- [ ] Status line and `?` box follow the active stack (D7, D8)
- [ ] `[settings.boxes] reload = ["f5"]` loads and works; shadowing allow-list in the validator
- [ ] Validation passes on the merged tree; snapshot diffs are hint/status text only
- [ ] Patterns mirrored, not reinvented

## Verified claims (plan fact-check, 2026-10-07)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Settings has ten sections, not ANA's seven | **true** (ANA amended here) | `settings/mod.rs` `pub mod` list: agents, boxes, connection, hierarchy, kinds, personas, prompt, qdrant, queue, secrets |
| 2 | Four production overlays, not three | **true** | `impl Overlay for` in `concepts_search.rs:273`, `migration_prompt.rs:71`, `waiting_list.rs:248`, `workspace_switcher.rs:143` (plus test doubles `Probe`, `Popup`, `Asking`) |
| 3 | 65 `HINT_*` constants in the M3 files, not 37 | **true** | count of `const …HINT…:`: agents 8, boxes 7, connection 4, hierarchy 9, kinds 6, personas 10, prompt 3, qdrant 5, queue 3, secrets 6, four overlays 1 each |
| 4 | ~80 snapshots in M3 areas, not ~46 | **true (upper bound)** | `tests/snapshots` prefixes: settings 10, box_settings 9, hierarchy 9, kinds 9, personas 7, secrets_settings 7, prompt_settings 6, connection 5, queue_settings 5, concepts_search 5, waiting 2, shell overlays 3, probe 1, `src/ui/overlay/snapshots` 1 |
| 5 | No view declares a stack yet (`key_layers`/`key_stack` absent) | **true** | text search: 0 hits |
| 6 | `Layer` has only `all`/`only`, no chord filter | **true** | `keys/stack.rs` `Layer` |
| 7 | `App::on_key` ends on `Stack::BASE` and the overlay step on `Stack::OVERLAY` | **true** | `app/state.rs` `App::on_key` (711-815) |
| 8 | The Settings tab cycles on `key.code` without modifiers (`ctrl-l` cycles) | **true** | `settings/mod.rs` `SettingsTab::on_key` match on `KeyCode::Char('l')` etc. |
| 9 | Hierarchy/kinds delete on `d` without a modifier check | **true** | `hierarchy.rs:1279`, `kinds.rs:1484` `KeyCode::Char('d')` arms in browse |
| 10 | The Qdrant editor passes every unused key | **true** | `qdrant.rs:224` `FieldOutcome::Pass => Handled::Pass` (connection's editor passes CONTROL only, `connection.rs:428`) |
| 11 | Agents and hierarchy lists take `j`/`k` only; their forms take `Tab`/`Down` and `BackTab`/`Up` | **true** | `agents.rs:2420`, `hierarchy.rs:1212`; forms `agents.rs:2166`/`:2170`, `hierarchy.rs:866`/`:870` |
| 12 | `Keys::set` is a no-op on a missing row (override rows need new code) | **true** | `keys/mod.rs` `Keys::set` doc and body |
| 13 | The validator knows only `STATE_GUARDED`, no shadowing kind | **true** | `keys/validate.rs` `allowed` |
| 14 | `Hint` has only `One`/`Pair` | **true** | `keys/hint.rs` |
| 15 | `Harness` has no way to install custom keys | **true** | `testkit.rs` public `with_*` methods: agent/run/concepts runtime, store state, tab, replay tab, overlay |
| 16 | `Ctx::keys()` already exists for views | **true** | `app/state.rs:132` |
| 17 | Catalogue tests pin 41 rows and the `ALL`/`position` lists | **true** | `catalogue.rs` `every_act_has_exactly_one_row` (`assert_eq!(CATALOGUE.len(), 41)`) |
| 18 | Lane source sets are pairwise disjoint | **true** | Files to Change: each section/overlay file appears in exactly one lane; T1 touches no section file except `settings/mod.rs` |
| 19 | Lane test files are disjoint by construction | **false → amended** | `tests/settings.rs` registers all ten sections, `tests/connection.rs` five, `tests/prompt_settings.rs` four. Ownership plus the worktree rule replace the independence claim |
| 20 | `boxes` hints spell `ctrl-s` lower-case (snapshot drift to `Ctrl+s` expected) | **true** | `boxes.rs:77`, `:84` |
| 21 | Repo disk has room for five worktree targets | **true** | `df -h .`: 2.8 T free on `/dev/sda`; `target/` 2.9 G; 12 cores, 60 G RAM |
