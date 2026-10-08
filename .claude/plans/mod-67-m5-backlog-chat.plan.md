# Plan: MOD-67 M5 — Backlog and Chat dispatch through context stacks

**Source**: `HANDOFF.md` MOD-67 (incl. the TUI design review findings 1 and 3); spec `docs/ANA-26.md`
§6, §7.2-§7.6, §8 (M5 row); carried from `.claude/plans/mod-67-m4-skills-requirements.plan.md` (D6:
the two Backlog `TextArea` owners check `ctrl-s` themselves and are M5's; D10: Backlog/Chat views)
**Routing**: plan (maintainer accepted 2026-10-08; C4 only, low confidence, the same call as
M1-M4). Ultracode for **implement** only; `code-architect` and `rust-reviewer` stay plain agents.
Sandbox run (`HR_SANDBOX=1`, branch `hr/MOD-67`, base `bb33692a`).
**Selected milestone**: M5. M6 (close-out) is a later run.
**Complexity**: Large (16 source files with key arms, ~100 non-test key-match sites, 15 `HINT_*`
constants plus ~12 inline hint literals, 56 owned snapshots and 11 app-level ones)
**Status**: confirmed 2026-10-08 (Q1: (a), the recommended option, taken on a plain "yes")

## Summary

The Backlog tab (list, filter form, item form and its divergence view, the seven detail sub-tabs
and the Notes/Docs compose areas) and the Chat tab (live, composer, parked permission, replay)
stop matching `key.code` and dispatch named actions through declared context stacks in
`keys/views.rs`, as M3 and M4 did for Settings, Skills and Requirements. Their hints become
`HintSpec`s, the Runs pane gains a generated browse hint, and the `?` box lists every key of the
focused tab and pane (design review finding 1) because the stack it renders is the pane's, the
Backlog's and the list's. The Backlog `Enter` miss row becomes an action the Backlog tab answers
itself, and the eight legacy `Keymap` rows `register_all` binds for the Backlog go. The modal
`ALT` inconsistency (ANA-26 §2.6 item 3: `alt-y` confirms an uncite or a cancel) is fixed by
construction and pinned. A rebound `form.save` now replaces `ctrl-s` in the Backlog's forms too.
No default changes.

## Scope drift since ANA-26 (fact-checked, see table)

ANA §8 sized M5 as `backlog/mod.rs`, `detail/{mod,body,documents,graph,notes,prompt,requirements,
runs}.rs`, `chat/{mod,transcript}.rs`, "~11 source files, ~38 snapshots". The tree now also has
key arms in `backlog/{filter,item_form,divergence}.rs` (MOD-13), `detail/compose.rs` (MOD-13 M5)
and `chat/composer.rs`, and **56** owned snapshots (`backlog__` 41, `chat__` 15). Eleven more
app-level snapshots show the Backlog tab (`shell__` 4, `integration__`, `replay__`,
`prompt_preview__` 2, `concepts_search__`, `waiting__` 2). `detail/body.rs` and `prompt.rs` only scroll; `list.rs`,
`chat/permission.rs` and `runs/execution_graph.rs` match no keys (the latter builds mouse events).

## Decisions (proposed; confirmed at the CONFIRM gate)

- **D1 — Contexts.** `backlog` for the tab (filter, clear filter, new, edit, open graph, queue,
  pause queue, the replay miss), then one view context per pane or mode that has its own verbs:
  `backlog.runs`, `backlog.graph`, `backlog.docs`, `backlog.notes`, `backlog.reqs`,
  `backlog.filter`, `backlog.item` (the item form and its divergence view), and `chat`. Body and
  Prompt only scroll and get no context. `?` headings: `Backlog`, `Runs`, `Graph`, `Docs`,
  `Notes`, `Reqs`, `Filter`, `Item`, `Chat`. `backlog` is a shared context (like `settings` and
  `skills`), so a pane's table may override a Backlog verb. Appended to `CATALOGUE` in strip
  order, each row citing the arm it mirrors; `ALL`/`position` lists grow in lockstep. The
  architect fixes the final list.
- **D2 — Naming (ANA §6.2).** Shared verbs where the meaning is shared: `list.*` (`j`/`k`, `g`/`G`,
  `Home`/`End`, `Enter` fold), `pane.next_subtab`/`prev_subtab` (`h`/`l`, `[`/`]`, arrows),
  `pane.scroll_*`/`page_*` (`J`/`K`, `PgUp`/`PgDn`), `common.edit`, `confirm.*`, `form.*`
  (`form.save` `ctrl-s`, `form.external_editor` `ctrl-e`, `form.next_field`/`prev_field`). In
  the Runs pane `J`/`K` are `backlog.runs.next_step`/`prev_step` (ANA §6.2), and its letters are
  view verbs: approve, reject, retry, promote, select, accept, open, cancel run, cleanup, unblock,
  start run, close out, follow up, the list/flow toggle and the flow zoom (`+`/`-`/`=`). `N` stays
  `N` (a view default or a Backlog verb, the architect's call). The architect fixes the full
  inventory per pane and mode. No default changes.
- **D3 — Stacks.** Every M5 mode's stack is a `static` in `keys/views.rs`, narrowest first: the
  pane's view layer, `pane`, `backlog`, `list`, then the global layer (ANA §7.3: the Runs pane in
  browse is `backlog.runs`, `pane`, `backlog`, `list`, `global`). Body and Prompt share
  `BACKLOG_BROWSE`. Capturing and confirming modes (reject note, follow-up, close-out stages,
  cancel confirm, the Reqs picker and uncite confirm, compose areas, filter and item forms,
  composer) end in a modal global layer. `DECLARED` lists them all with their error phrases.
  Views import their stacks and never declare one. T1 owns `keys/` (M3 D3).
- **D4 — Which mode is active.** `DetailTab` gains `fn key_stack(&self) -> Stack<'static>`
  (default `BACKLOG_BROWSE`) and `DetailRegistry::key_stack` delegates to the active pane.
  `BacklogTab::key_stack` returns the filter form's, else the item form's (or its divergence
  view's), else the detail pane's. `ChatTab::key_stack` returns replay's, the composer's, a parked
  permission's, a live chat's or the idle one. `Tab::key_stack` already exists (M3 D4).
- **D5 — Dispatch order.** Candidates come from the active stack, narrowest first. The pane is
  offered the key first and declines what it does not own, then the Backlog tab takes its own acts
  (ANA §7.3; today the tab intercepts first, which is equivalent because the validator refuses a
  chord bound twice in one stack). Keys outside text entry that §6.1 keeps fixed stay ahead of the
  resolver exactly as today: the permission digits in Runs and Chat (and replay's swallow), the
  composer, `TextField`/`TextArea` editing, and a choice field's value keys.
- **D6 — The replay miss (ANA §7.3, amended).** `Enter` in the Backlog is `backlog.replay` in
  `[backlog]`, not `backlog.runs.replay`, because the miss fires from every sub-tab and must be in
  every Backlog stack. A header row folds (`list.fold`), the Runs pane replays the step under its
  cursor, the Graph pane re-roots, and when nothing accepts it the Backlog tab emits the miss,
  naming the step keys through `label`. Under the defaults that is still "select a step in the
  Runs pane (J/K) to replay it", so `tests/backlog.rs` and `tests/replay.rs` stay green unchanged.
  The eight `Keymap` rows `register_all` binds for the Backlog (`Enter`, `m`, `f`, `F`, `N`, `e`,
  `Q`, `P`) are deleted by L-A, the one lane allowed to touch `app/mod.rs`, and only those rows.
  `Keymap`, `App::on_key`'s legacy tab step and `render_help`'s legacy branch stay until M6.
- **D7 — Modifiers.** The `CONTROL | ALT` guards in the Backlog, Chat and pane browse arms and the
  CONTROL-only passes in the Runs and Reqs modals (`runs.rs:632`, `requirements.rs:268`) go. Modal
  stacks pass what `Stack::passes` admits (M3 PA-5), so `alt-y` no longer confirms an uncite or a
  run cancel. Pinned.
- **D8 — Forms.** The item form, its divergence view and the Notes/Docs compose areas resolve
  `form.save` and `form.external_editor` from their stacks. `item_form::{ctrl_s, ctrl_e}` and
  `filter::CHORD` are deleted once no caller is left. With M4's `TextArea` change, a rebound
  `form.save` then replaces `ctrl-s` in every editor in htui. `Tab`/`BackTab` focus moves are
  `form.next_field`/`prev_field`. The chat composer stays fixed (ANA §6.1).
- **D9 — Hints.** M3's `HintSpec` elements (`One`, `Two`, `Pair`, `All`, `Text`). The 15 `HINT_*`
  constants and the inline hint literals (Runs footers and artifact line, Graph depth line, Reqs
  footers, Chat `hint()` lines, replay header) become specs. The Runs pane gains a generated browse
  hint (ANA §8). Expected snapshot drift is text and new hint rows only (`Ctrl+S` → `Ctrl+s`,
  separators unified to ` · `, pair joins, capture-filtered status lines). Diffs are reviewed line
  by line; anything else is a defect. Sentences that name a key stay M6's prose (D11).
- **D10 — Global collisions.** Once the Backlog and Chat stacks are in `DECLARED`, a global key on
  a Backlog or Chat key is refused, like Settings and Skills today. The README example
  `[global] quit = ["f"]` therefore becomes a refused file:
  `the_readme_backlog_example_loads_and_f_stays_the_backlogs` flips (in T1) to pin the refusal,
  and T-close rewrites the README "Changing keys" paragraph ("Backlog and Chat do not read this
  file yet").
- **D11 — Out of M5 (M6).** Key prose (`Ctrl+S tries again`, `Ctrl+S compares again`, `Ctrl+S saves
  the resolution`, `Esc again ends this chat`, `no conflicts — m or t continues`, the run
  sentences); deleting `Keymap`/`Binding`/`KeyScope` with App's legacy paths; `Ctx::new(&Keymap)`;
  the hierarchy `picker` context (M3 carry); the every-stack reachability test; and the uniform
  hint row (Q1 (a)).
- **D12 — Tests.** The spec-string integration tests stay green unchanged. New pins:
  - `alt-y` does not uncite (Reqs) or cancel a run (Runs); `ctrl-l`/`alt-j` in the Backlog do not
    move the cursor or switch sub-tabs;
  - a rebound `form.save` saves the item form, a note and a document, and `ctrl-s` is inert in all
    three; a rebound `form.external_editor` hands the body out;
  - the replay miss under a rebound `backlog.runs.next_step` names the new key;
  - per lane, one rebinding test through `Harness::with_keys`: rebound chord acts, old one inert,
    hint shows the new label;
  - status line and `?` box for the Runs pane in browse (lists `backlog.runs`, `pane`, `backlog`,
    `list`, global) and for an open composer (status line drops `q quit`);
  - `[global] quit = ["f"]` refused, naming the Backlog.

## Open question (maintainer, at CONFIRM) — resolved: (a)

- **Q1 — The Backlog hint row (design review finding 3).** The HANDOFF entry says Backlog has no
  hint row and that generated hints belong in "one fixed row for every tab". Every converted view
  (M3/M4) still draws its own hint line where it always did.
  - **(a) Recommended:** M5 gives the Backlog list pane a one-line generated hint at its bottom
    (its browse acts: move, sub-tab, filter, new, edit, queue, fitted to the pane's ~53 columns),
    and the one fixed row for every tab is decided in M6, where the `Tab` API change and the
    all-snapshot re-baseline are one step. Cost: T-close re-baselines every snapshot that shows
    the list (41 owned plus 11 app-level ones).
  - (b) M5 builds the uniform fixed row for every tab in T-close: a `Tab::hint` API, the shell
    draws it above the status line, and every view's own hint line moves there. Every tab's
    snapshots move.
  - (c) No new row in M5: the Runs pane's browse hint and the `?` box only; finding 3 goes to M6
    or MOD-82.

## Amendments (maintainer, 2026-10-08, from blueprint §1)

PA-1..PA-6 (`.claude/plans/mod-67-m5-backlog-chat.blueprint.md` §1) approved as written: Prompt's
`n`/`p` are view verbs (`backlog.prompt`), Body gets `backlog.body` and `BACKLOG_BROWSE` becomes
`BACKLOG_BODY` (PA-1); `Enter` is state-guarded fold/replay, Graph's re-root its own act, the fold
tried before the pane (PA-2); six legacy-help/spacing assertions in `tests/backlog.rs` and
`tests/replay.rs` move to the `?` box as flagged L-A edits, `e edit item` → `e edit` (PA-3); Chat
has three stacks, the composer on `CAPTURE` (PA-4); the Q1(a) list hint is
`j/k move · h/l sub-tab · f filter · N new · e edit`, queue left to the `?` box (PA-5, maintainer
choice); L-B accepts `replay__runs_step_selected` (PA-6). The blueprint's "user-visible beyond D9"
list is accepted whole (modifier-blind Body/Prompt scroll and template keys fixed, `F1` help in
every Backlog/Chat capturing mode, the Runs hint row's height, `J/K scroll` and `Tab row`
spellings, the Notes/Docs notice clearing on Backlog keys, the per-layer `?` box). Claims 3 and 5
and D6's "tests unchanged" amended per blueprint §2.

## Deviations recorded during implementation

- T1a/T1b (`850d4bee`, `de94f61a`): the filter form's TOML table is `[backlog.filter_form]`, not
  `[backlog.filter]` (D1): `[backlog] filter = …` and a `[backlog.filter]` table are the same TOML
  key, and the default `--print-keys` no longer read back. Pinned by
  `no_act_name_is_a_nested_table`. Rust `Context::BacklogFilter`, heading `Filter` unchanged.
- T1c (`c357723e`) moved `detail/mod.rs` by 15 lines, so the `[pane]` rows in `keys/catalogue.rs`
  cite stale lines (`:447`-`:450` → the `Scroll::on_key` arms): fixed in T-close.
- Blueprint §5.6 whole-suite command had `--no-fail-fast` after `--` (libtest refuses it, no test
  runs); corrected in the blueprint before the lanes.
- T1b: `[form] next_field = ["x"]` also collides with `backlog.filter_form.clear` now (a real
  collision; `validate.rs` expectation grown).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Catalogue block | `keys/catalogue.rs` `[skills.*]`, `[requirements]` blocks (M4) | one commented block per context, rows citing the arm they mirror; `ALL`/`position` lists in lockstep |
| Stack statics | `keys/views.rs` `LIBRARY_BROWSE`, `REQUIREMENTS_FORM`, `CAPTURE` | `Layer::view` + shared layers + modal global; `DECLARED` phrase per stack (`keys/stack.rs:205`) |
| View dispatch | `requirements/mod.rs` `RequirementsTab::on_key`, `settings/boxes.rs` `form.save` after the widget (M4 PA-3) | first accepted candidate; widget first in capturing modes |
| Tab stack | `requirements/mod.rs` `key_stack` per mode; `skills/mod.rs` delegating to the active view | `Tab::key_stack` (`ui/tabs/registry.rs:79`) |
| Pane scroll | `backlog/detail/mod.rs` `Scroll::apply` (M4 T1e) | pane-scroll acts in one place |
| Hint spec | `requirements/mod.rs` `RequirementsTab::hint`, `ctx.keys().hint(stack, SPEC)` | `HintSpec` consts rendered per stack |
| Rebinding test | `tests/keys.rs` + `Harness::with_keys` (`testkit.rs:185`) | rebound acts, old inert, hint label |
| Integration gate | `#![cfg(feature = "testkit")]` | App-level tests need `--features testkit` (memory) |

## Files to Change

| File | Action | Owner | Why |
|---|---|---|---|
| `crates/htui/src/keys/{catalogue,views,stack}.rs` | UPDATE | T1 | D1-D3 contexts, acts, rows, stacks, `DECLARED` |
| `crates/htui/src/keys/{load,validate,print}.rs` | UPDATE | T1 | only if a new context needs it (the `backlog` shared kind) |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | T1 | D4 `DetailTab::key_stack` (default) and `DetailRegistry::key_stack` |
| `crates/htui/tests/keys.rs`, `tests/keys_file.rs` | UPDATE | T1 | catalogue counts, D10 flip |
| `crates/htui/tests/backlog_keys.rs` | CREATE | T1 (seed), lanes | one `mod` per lane (`l_a`, `l_b`, `l_c`), each seeded with a placeholder test so lane hunks never touch |
| `ui/tabs/backlog/{mod,filter,item_form,divergence}.rs`, `detail/{compose,notes,documents}.rs`, `app/mod.rs` (Backlog rows only) | UPDATE | L-A | convert; hint specs; D6, D8; list hint (Q1 a: T-close) |
| `ui/tabs/backlog/detail/runs.rs` | UPDATE | L-B | convert; browse hint; D7 pin |
| `ui/tabs/backlog/detail/{body,graph,prompt,requirements}.rs` | UPDATE | L-C | convert; hint specs; D7 pin |
| `ui/tabs/chat/{mod,transcript,composer}.rs` | UPDATE | L-D | convert; hint specs; `key_stack` |
| `tests/backlog.rs` + `backlog__*` snapshots | UPDATE | L-A/L-B/L-C by snapshot (below) | flagged edits only; re-baseline own snapshots |
| `tests/chat.rs` + `chat__*` snapshots | UPDATE | L-D | pins, re-baseline |
| app-level snapshots (`shell__`, `integration__`, `replay__`, ...) | UPDATE | T-close | re-baseline on the merged tree |
| `README.md` ("Changing keys", Backlog/Chat key tables), `HANDOFF.md`, this plan | UPDATE | T-close | D10, M5 phase note, review gate |

## Tasks

TDD per task: tests first, red, then code. Gortex for reads; in a worktree, native reads and
edits are the fallback (memory: Gortex edits hit the primary checkout).

### Task T1: foundation — serial, primary tree, first
- **Action**: D1-D3 from the architect's inventory: every catalogue row, `VIEW_DEFAULTS` row and
  stack for Backlog and Chat, `DECLARED` grown. D4's trait method with its default, so no pane
  converts and `BacklogTab::key_stack` stays `None`. D10's test flip. Seed `tests/backlog_keys.rs`.
  Catalogue count tests updated.
- **Validate**: `cargo test -p htui --lib keys`, `cargo test -p htui --features testkit --test keys
  --test keys_file`, `cargo insta test -p htui --all-features --check` (no snapshot changes in T1).

### Lanes L-A..L-D: view conversion — parallel, one worktree each, after T1
Each lane replaces its `key.code` arms with resolver dispatch on its `views.rs` stacks, implements
`key_stack` per mode, turns its hints into `HintSpec`s, drops the modifier guards the resolver makes
redundant, adds its D12 pins and one rebinding test, re-baselines its own snapshots and reviews
every diff line.

| Lane | Source | Owned tests | Owned snapshots |
|---|---|---|---|
| L-A | `backlog/{mod,filter,item_form,divergence}.rs`, `detail/{compose,notes,documents}.rs`, `app/mod.rs` Backlog rows | `backlog_keys.rs::l_a`, unit tests in its files | `list_*` (3), `filter_*`/`filtered_list` (3), `item_form_*` (2), `item_divergence`, `detail_notes`, `detail_documents`, `empty_notes`, `empty_documents`, `document_form`, `note_compose` (15) |
| L-B | `detail/runs.rs` | `backlog_keys.rs::l_b`, unit tests in `runs.rs` | `detail_runs`, `empty_runs`, `runs_*` (12) |
| L-C | `detail/{body,graph,prompt,requirements}.rs` | `backlog_keys.rs::l_c`, unit tests in its files | `detail_{body,graph,prompt,reqs*}` (6), `empty_{body,graph,prompt,reqs}` (4), `graph_*` (4) (14) |
| L-D | `chat/{mod,transcript,composer}.rs` | `tests/chat.rs`, unit tests in its files | `chat__*` (15) |

Why L-A is wide: `compose.rs` imports `item_form::{HINT_AREA, HINT_TEXT, PAGE, ctrl_e, ctrl_s,
marker}` and `filter::CHORD`, `documents.rs` and `divergence.rs` import `ctrl_s`, and `notes.rs`
and `documents.rs` drive `Compose::on_key`. Splitting them would break the build at merge. L-A works
serially inside: the tab, then the filter and item forms with divergence, then compose with Notes
and Docs.

Lane rules (M3/M4's): never edit `keys/`, `app/` (L-A's Backlog rows excepted), the registries,
`testkit.rs` or `tests/keys*.rs`. A missing act or stack goes back to the main thread, which amends
T1 on the base, and the lanes merge it. Another lane's test file is edited only where this lane's
change breaks an assertion, and the edit is flagged. Format only own paths. Gate with
`env -u HTUI_TEST_DATABASE_URL` and `--test-threads=1`. Commit incrementally on
`mod-67-m5-<lane>` in `target/wt/<lane>`. Verifiers make STRICTLY no edits and no commits.

Known couplings:
- Until L-A merges, the Backlog tab intercepts its own letters first, so the L-B/L-C rebinding
  tests pick chords the unconverted tab passes. Those tests are re-run on the merged tree.
- `tests/replay.rs` (Runs `Enter` and the Chat replay) is touched by L-B or L-D only for a flagged
  break.
- Status lines in capturing modes become capture-filtered as each pane converts, so they show up in
  that lane's snapshots, as in M4.

### Task T-close: merge and close-out — serial, primary tree
- Merge lanes `--no-ff` one at a time (L-A first: it owns `BacklogTab::key_stack`). Full Validation
  after each.
- Q1 (a): the Backlog list hint row, then the one full re-baseline of every snapshot showing the
  list, `backlog__` and app-level, reviewed line by line.
- The rebinding and `?` box tests re-run on the merged tree with App-level stacks (D12).
- Dead helpers gone (`ctrl_s`, `ctrl_e`, `filter::CHORD`); `relations usages` confirms no caller.
- README "Changing keys": the Backlog/Chat caveat replaced (D10), and the tables gain
  `[backlog.*]`/`[chat]`. The Backlog and Chat key tables are checked against the defaults.
- HANDOFF M5 phase note; this plan's review gate; validator; remove worktrees, then lane branches.

Independence: T1 → lanes is a hard chain. Lane source sets are pairwise disjoint (table above).
Test and snapshot sets are disjoint by ownership: `backlog__*.snap` are separate files, assigned
one lane each. They are still coupled through `tests/backlog.rs` and `tests/replay.rs` (flagged
edits) and through the app-level snapshots, so the lanes run in worktrees and T-close re-baselines
once.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Postgres suites (`*_pg.rs`, `runs_pg.rs`, `item_writes_pg.rs` touch the Backlog) once on the fully
merged tree (sandbox DB on 5439). Workspace gate at the end (memory: the "zeta" literal and other
crates' checks only show there).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `runs.rs` (6.3k lines, browse + 6 modal modes, flow view, digits) dominates L-B | High | L-B is serial inside by mode; the architect splits its modes; digits stay ahead of the resolver (D5) |
| `Enter` routing changes order (fold, pane, re-root, miss) | Medium | D6 pins: header folds, Runs replays, Graph re-roots, Body misses; the two existing miss tests unchanged |
| Snapshot churn hides a behaviour change | Medium | D9: only hint, status and the new hint rows move; full insta run per merge; app-level ones in T-close |
| A pane's verb collides with a Backlog or list key once in one stack | Medium | the validator walks every new stack (`the_compiled_defaults_validate`); `Enter` takes the shadowing allow-list kind |
| Deleting the legacy rows loses a `?` line or the miss | Low | D6 test of the miss; the `?` box for Backlog is pinned (D12) |
| A user file that loads today is refused after M5 (D10) | Low | Global keys on a Backlog/Chat key, and shared-table rebinds that collide in a new stack (`[pane] next_subtab = "tab"`, `[common] edit = "f"`, `[list] down = "G"`; T1 verifier's differential probe, 2026-10-08); each is a real collision, the error names the line and the stack; README says so without claiming "global only" |
| Lane disk/CPU (4 worktrees) | Low | ~10 GB `target` each; 2.9 TB free, 12 cores |

## Acceptance

- [ ] Backlog (tab, filter and item forms, divergence, the seven sub-tabs, compose areas) and Chat
      dispatch through declared stacks; no `KeyCode` match remains outside keys §6.1 keeps fixed
- [ ] The 15 `HINT_*` constants and the inline hint literals in these files are hint specs; the
      Runs pane has a browse hint
- [ ] The `ALT` inconsistency is fixed and pinned; the replay miss is an action; the legacy Backlog
      rows are gone
- [ ] `form.save` is the only save chord in the Backlog's forms and compose areas
- [ ] `?` lists every key of the focused Backlog pane and of Chat
- [ ] Validation passes on the merged tree; snapshot diffs are hint/status text and the agreed rows
- [ ] Patterns mirrored, not reinvented

## Verified claims (plan fact-check, 2026-10-08)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | M5 key arms live in 16 files, five of them outside ANA §8's list | **true** | non-test `KeyCode::`/`Char(` hits: `backlog/{mod,filter,item_form,divergence}.rs`, `detail/{compose,documents,graph,mod,notes,prompt,requirements,runs}.rs`, `chat/{mod,composer,transcript}.rs`, plus `body.rs` through `Scroll::on_key`; `list.rs`, `permission.rs` 0 |
| 2 | 56 owned snapshots, not ~38 | **true** | `tests/snapshots`: `backlog__` 41, `chat__` 15 |
| 3 | ~13 app-level snapshots show the Backlog list | **false → amended (11)** | a key-pattern grep listed 13; `requirements__requirements_filter` (strip title only) and `queue_overlay__opened` (item keys in the overlay, no `Backlog`) do not show the tab. The 11: `replay__runs_step_selected`, `prompt_preview__` 2, `integration__demo_shell`, `concepts_search__hits`, `shell__` 4, `waiting__` 2 |
| 4 | 15 non-test `HINT_*` constants | **true** | `const …HINT…:` in documents 1, notes 1, requirements 3 (+1 test-only), divergence 4, filter 2, item_form 3, chat 1 |
| 5 | Inline hint literals in Runs, Graph, Reqs and Chat | **true** | `runs.rs:891`, `:1924`, `:1929`, `:1958`; `graph.rs:767`; `requirements.rs:425`; `chat/mod.rs` `hint()` 5 literals |
| 6 | `register_all` binds 8 legacy Backlog rows, `Enter` being the miss | **true** | `app/mod.rs:143-189`: `Enter`, `m`, `f`, `F`, `N`, `e`, `Q`, `P` on `KeyScope::Tab(BacklogTab::ID)` |
| 7 | App still resolves legacy tab rows and draws them in `?` | **true** | `app/state.rs:885` `keymap.resolve(&KeyScope::Tab(id), …)`; `:998` `render_help` legacy branch |
| 8 | Runs and Reqs modals pass CONTROL only (`alt-y` confirms) | **true** | `runs.rs:632`, `requirements.rs:268` `key.modifiers.contains(KeyModifiers::CONTROL)` |
| 9 | Backlog and Chat browse arms guard CONTROL and ALT | **true** | `backlog/mod.rs:749`, `chat/mod.rs:463`, `runs.rs:1521`, `graph.rs:690`, `notes.rs:299`, `documents.rs:453`, `requirements.rs:470` |
| 10 | No Backlog/Chat view declares a stack; `DetailTab` has no `key_stack` | **true** | `fn key_stack` 0 hits in `backlog/`, `chat/`; trait source `detail/mod.rs` |
| 11 | The Backlog `TextArea` owners check `ctrl-s` themselves | **true** | `item_form.rs:273` `ctrl_s`, `:279` `ctrl_e`, used at `item_form.rs:593`, `compose.rs:319`, `documents.rs:193`, `divergence.rs:238` |
| 12 | `compose.rs`, `documents.rs`, `divergence.rs` depend on `item_form`/`filter` helpers | **true** | `compose.rs:34-35` imports `filter::CHORD`, `item_form::{HINT_AREA, HINT_TEXT, PAGE, ctrl_e, ctrl_s, marker}`; `documents.rs:61`; `divergence.rs:31`, `:34` → L-A owns all of them |
| 13 | The README example `[global] quit = ["f"]` is pinned as loading | **true** | `tests/keys.rs:748` `the_readme_backlog_example_loads_and_f_stays_the_backlogs`; README "Changing keys" last paragraph → flips (D10) |
| 14 | The miss text is pinned by two tests | **true** | `tests/backlog.rs:1362`, `tests/replay.rs:217` |
| 15 | `Tab::key_stack`, `Harness::with_keys`, `Scroll::apply` exist | **true** | `ui/tabs/registry.rs:79`; `testkit.rs:185`, `:703`; `detail/mod.rs` `Scroll::apply` |
| 16 | `DECLARED` has 56 stacks today | **true** | `keys/stack.rs:205-271` |
| 17 | The chat composer consumes every non-chord key itself | **true** | `chat/composer.rs:81-107` (`_ => Consumed`); ANA §6.1 keeps it fixed |
| 18 | `runs/execution_graph.rs` matches no keys | **true** | its `KeyModifiers` uses build mouse events (`:19`, `:662`) |
| 19 | Lane source sets are pairwise disjoint | **true** | Files to Change: each source file in exactly one lane; T1 touches `keys/`, `detail/mod.rs`, `tests/keys*.rs`, `tests/backlog_keys.rs` only |
| 20 | Lane snapshot sets partition the 56 | **true** | L-A 15 + L-B 12 + L-C 14 = 41 `backlog__`; L-D 15 `chat__` |
| 21 | Disk and cores for four worktrees | **true** | `df -h .`: 2.9 T free on `/dev/sda`; `target/` 2.9 G; `nproc` 12 |
