# Blueprint: MOD-67 milestone 5, "Backlog and Chat dispatch through context stacks"

**Status**: draft for the maintainer (2026-10-08). PA-1 to PA-6 (§1) need approval; the rest
builds the confirmed plan as written. D1-D12 stand except where §1 amends them.

**Plan**: `.claude/plans/mod-67-m5-backlog-chat.plan.md` (confirmed 2026-10-08, Q1 = (a)).
**Lane files**: `.claude/plans/mod-67-m5-blueprint/L-{A,B,C,D}.md` (every mode, arm, hint, test
and snapshot of each lane).
**Templates**: M4 blueprint `.claude/plans/mod-67-m4-skills-requirements.blueprint.md` (shape, §6
skeletons) and M3 blueprint `.claude/plans/mod-67-m3-settings-overlays.blueprint.md` §6 (browse,
capturing and question skeletons, which §6 here extends).
**Verified at**: HEAD `3af293a6`, branch `hr/MOD-67`. Read at that HEAD: `keys/{catalogue,stack,
views,hint,validate,load,print,chord}.rs`, `app/mod.rs` (`register_all`), `app/state.rs`
(`on_key`, `active_stack`, `apply_keys`, `render_help`), `ui/tabs/backlog/{mod,filter,item_form,
divergence,list}.rs`, `backlog/detail/{mod,body,compose,documents,graph,notes,prompt,
requirements,runs}.rs`, `chat/{mod,transcript,composer}.rs`, `ui/{text_area,text_field}.rs`,
`tests/{keys,keys_file,backlog,replay}.rs` (the key-file and help-line assertions),
`tests/fixtures/keys/*.toml`, every `backlog__*`/`chat__*` snapshot's hint and status rows, the
16 app-level snapshots that show the Backlog, `README.md` "Changing keys" and the Backlog/Chat
tables, ANA-26 §6-§8. Line numbers are pre-edit.

**Order**: T1 (serial, primary tree) → L-A ∥ L-B ∥ L-C ∥ L-D (worktrees `target/wt/<lane>`,
branches `mod-67-m5-<lane>`) → T-close (serial, primary tree). Lanes add no context, act,
stack, `VIEW_DEFAULTS`, `SHADOWING` or `STATE_GUARDED` entry: §3-§4 are complete for all four. A
missing one goes back to the main thread (plan lane rule).

**House style (M2-M4, unchanged)**: `#![warn(missing_docs)]`, inline format args, rustfmt edition
2024 width 100, clippy `-D warnings` both `--all-targets --all-features` and featureless.
Implementers commit incrementally and stage their own paths only. Gortex `read` for source; where
the hook blocks `Read`/`Edit`, an anchored scripted replace (each anchor asserted to match once).

**The one-paragraph design.** Every Backlog and Chat mode names a `pub const` stack in
`keys/views.rs`, built as M3/M4's: the mode's view layer first, then the shared layers it offers,
then the global layer (unfiltered while browsing, `MODAL` while capturing or asking). Every
Backlog browse stack has the same tail, `pane`, `backlog`, `common ∩ {edit}`, `list`, `global`
(ANA §7.3), so the Backlog tab's own verbs and `list.*` resolve through whichever pane is active,
and a pane's table may override them for that pane alone (`backlog` is a shared context, like
`settings` and `skills`). The Backlog tab dispatches **fold first, then the pane, then its own
acts** (§6.1): a project header folds on `Enter` before any pane sees it, the pane takes what its
stack resolves to its own verbs and declines the rest, and the tab answers `list.*`,
`pane.next_subtab`/`prev_subtab`, `backlog.*` and `common.edit`, including `backlog.replay`, the
miss that used to be a legacy `Keymap` row. Widget keys stay ahead of the resolver (D5): the
permission digits, the chat composer, `TextField`/`TextArea` editing, a widget's `Enter`/`Esc`,
and the value keys of a choice (the item form's pickers, the filter form's options and the Runs
close-out resolution, §1 note on borderline keys). Hints become `HintSpec`s rendered through the
mode's stack; the only new rows are the Runs browse hint (ANA §8) and, in T-close, the list hint
(Q1 a).

---

## 1. Plan amendments

| # | Amends | Confirmed text | Why it cannot stand as written | What this blueprint builds |
|---|---|---|---|---|
| **PA-1** | D1, D3 ("Body and Prompt only scroll and get no context"; "Body and Prompt share `BACKLOG_BROWSE`"); scope note "`prompt.rs` only scrolls" | No context for Body or Prompt; one shared `BACKLOG_BROWSE`. | (a) `prompt.rs:327-331` binds `n`/`p` (next/previous template) and matches `key.code` alone, so `ctrl-n` cycles the template today (the Backlog tab hands every CONTROL/ALT chord to the pane, `backlog/mod.rs:746-752`): two view verbs, and a modifier-blind arm D12 means to fix. (b) A stack without a view layer cannot be appended to `DECLARED`: `views.rs` `every_view_layer_comes_first_and_every_modal_stack_ends_modal` requires layer 0 to be a view layer from index 6 on, and inserting a non-view stack at index 6 (as M4 did with `SKILLS_TAB`) moves the first-reported phrase of existing errors (`[global] quit = ["j"]` would read "in Backlog" instead of "in Settings > Agents", `validate.rs` `a_global_entry_on_a_list_default_is_refused_where_a_stack_composes_them`). | Two more view contexts: **`backlog.prompt`** (`next_template` `n`, `prev_template` `p`, heading `Prompt`) and **`backlog.body`** (no own act, overrides only, heading `Body`, like `settings.prompt`). The plan's `BACKLOG_BROWSE` is **`BACKLOG_BODY`** = `view(backlog.body)` + the browse tail; it is Body's stack and `DetailTab::key_stack`'s default (D4). Prompt gets `BACKLOG_PROMPT`. 11 contexts, not 9. |
| **PA-2** | D6 (the `Enter` routing), Risk row 4 ("`Enter` takes the shadowing allow-list kind") | One allow-list kind for `Enter`; Graph's re-root unnamed. | `list.fold` (wider layer) must win over `backlog.replay` (narrower) on a project header and lose on an item row. That is ANA §7.4's **state-guarded** kind ("`list.fold` and `backlog.runs.replay` on Enter"), not shadowing (the narrower act always wins). Graph's `Enter` is a third meaning (re-root, `graph.rs:701`) that always consumes the key (`graph.rs:680-686`): it needs its own act, and against `backlog.replay` it is genuine shadowing. | Graph's `Enter` is **`backlog.graph.reroot`**. `STATE_GUARDED` += `(ListFold, BacklogReplay)`, `(ListFold, GraphReroot)`; `SHADOWING` += `(GraphReroot, BacklogReplay)`. Each is demanded (`every_allow_list_entry_is_demanded`). The tab tries the fold before offering the pane (§6.1), as today (`backlog/mod.rs:791-797`). |
| **PA-3** | D6 ("so `tests/backlog.rs` and `tests/replay.rs` stay green unchanged"); plan claim 14 | The two spec-string miss tests are the only coupling. | Five assertions read the legacy rows L-A deletes through `keymap.help_line(&KeyScope::Tab(BacklogTab::ID))`: `tests/backlog.rs:268` (`m open graph`, `Enter replay step`), `:1810` (`f filter`, `F clear filter`), `:2392` (`N new item`, `e edit item`), `:3398` (`Q queue / dequeue`, `P pause / resume queue`) and `tests/replay.rs:217-226` (`Enter replay step`). With the rows gone each reads `""` and fails. Also `tests/backlog.rs:2633` pins the divergence hint's two-space joins (D9 drift). | L-A rewrites these six assertions (flagged): the five read the `?` box (`harness.key("?")`, frame) instead of the keymap; help labels are the legacy ones (§3.2: `replay step`, `open graph`, `filter`, `clear filter`, `new item`, `queue / dequeue`, `pause / resume queue`) except `e`, which is `common.edit` (`Common: e edit`, D2), so `e edit item` becomes `e edit`. The miss text assertions (`tests/backlog.rs:1362`, `tests/replay.rs:217`'s status half) stay unchanged. |
| **PA-4** | D4 ("`ChatTab::key_stack` returns replay's, the composer's, a parked permission's, a live chat's or the idle one") | Five Chat stacks. | Idle, live, parked and ended share one key arm today (`chat/mod.rs:498-523`); they differ only by a state-declined act (`a` without a session) and by the fixed digits. Separate stacks would drop keys that work today (`t`, `j`/`k`, `g`/`G` reach the transcript in every live state) or list the same set four times under four phrases. The composer is a text capture and nothing else. | Three: **`CHAT_LIVE`** (idle, live, parked, ended), **`CHAT_REPLAY`**, and M3's **`CAPTURE`** for an open composer. The hint still varies by state (§L-D). |
| **PA-5** | Q1 (a) ("its browse acts: move, sub-tab, filter, new, edit, queue, fitted to the pane's ~53 columns") | Six acts in the list hint row. | All six render as `j/k move · h/l sub-tab · f filter · N new · e edit · Q queue`, 60 cells, against the list pane's 53 inner columns at 100×30. | **Five**: `j/k move · h/l sub-tab · f filter · N new · e edit` (50 cells); `Q`/`P` stay in the `?` box. Alternative if the maintainer prefers the queue key: `j/k · h/l pane · f filter · N new · e edit · Q queue` (52 cells). T-close builds the five unless told otherwise. |
| **PA-6** | Lane rules ("app-level snapshots ... T-close re-baselines on the merged tree"); lane table | No lane touches an app-level snapshot. | The lane gate runs `cargo insta test --check` over every snapshot. `replay__runs_step_selected` shows the Runs pane in browse, so L-B's new browse hint row moves it on L-B's branch, and L-B's gate fails unless L-B accepts it. No other app-level snapshot moves before T-close (checked per lane in the lane files). | L-B accepts `replay__runs_step_selected` (Runs hint row only, flagged in the commit body); T-close re-baselines it again for the list hint row. |

**Borderline keys (the decisions the plan left to the architect; not amendments).**
- **Filter form `x clear`**: an act, `backlog.filter.clear` (`x`). It clears the whole draft (a
  form verb), it is not a value of the focused option. The options' `h`/`l`/`Left`/`Right` and
  `Space` stay fixed (choice-field value keys, ANA §6.1), as do the form's `Enter` (apply) and
  `Esc` (cancel).
- **Item form `←`/`→`/`h`/`l` (choose)**: fixed. They are a picker's value keys, exactly
  Priority's in M4 (ANA §6.1); the picker's `Enter` (move on) and `Esc` (cancel) are its own too.
- **Runs close-out `←`/`→`/`h`/`l` (resolution)**: fixed, for the same reason: they cycle the one
  value the question draws ("close KEY as RESOLUTION"), with the item form's key set. Making them
  acts would give one meaning two rules. Hint text unchanged (`Text("←/→ resolution")`).
- **Divergence `t`/`m`/`Tab`**: acts (`backlog.item.theirs` `t`/`T`, `mine` `m`/`M`,
  `switch_diff` `Tab`/`BackTab`). They are the view's verbs, not values of a field. Its `j`/`k`
  /arrows are `list.down`/`up`, `PgDn`/`PgUp` `pane.page_*`, `Esc` `common.back` (the catalogue
  already cites `divergence.rs:245`, `:261`).
- **Replay's swallow** (`1`-`9`, `i`, `a`, `Enter`, `chat/mod.rs:242`) and both permission-digit
  arms (`runs.rs:1528-1544`, `chat/mod.rs:489-496`): fixed, ahead of the resolver, plain keys only
  (today's CONTROL/ALT guard ran first).
- **The chat composer**: fixed; it now sees every key `CAPTURE` does not pass, so `F1` opens help
  from it (it swallowed F-keys).

**User-visible beyond D9** (not amendments; reviewed per lane, listed so the maintainer sees them):
1. Pane first (D5): the Notes and Docs notice ("the sentence stays until the next key",
   `notes.rs:296`, `documents.rs:449`) now also clears on the Backlog's own keys (`h`/`l`, `m`,
   `f`, `F`, `N`, `e`, `Q`, `P`, `g`/`G`), which never reached the pane before. `j`/`k` changed
   the item anyway, which cleared it.
2. Modifier-blind pane arms fixed: in Body and Prompt, `alt-J`, `ctrl-PgDn` and the like no longer
   scroll (`Scroll::on_key` matched the code alone); in Prompt `ctrl-n`/`alt-p` no longer cycle the
   template.
3. `F1` opens help from every Backlog and Chat capturing or asking mode (filter form, item form,
   divergence view, Notes/Docs compose, the Runs note/follow-up/typed key/questions/document view,
   the Reqs picker and questions, the chat composer); these swallowed it. ALT chords now pass to
   the shell there (nothing binds them). M3's accepted rule (D5).
4. The Runs browse hint row takes one row from the run list (and from the flow canvas, so a flow
   may lay out one row shorter); a run list that filled the pane scrolls one row sooner.
5. Hint spellings: the Runs document view `j/k scroll` → `J/K scroll` (pane.scroll_down's first
   chord; `j`/`k` still scroll); the filter form's Tags row `↑/↓ row` → `Tab row`
   (form.next_field, as the item form's `Tab field`).
6. The Backlog `?` box lists one line per layer of the focused pane's stack (`Runs:`, `Pane:`,
   `Backlog:`, `Common:`, `List:`, `Global:`) instead of the one legacy `Backlog:` line; `e` reads
   `e edit` (common.edit), not `e edit item`. Chat gains its `?` lines.
7. The divergence view no longer swallows `ctrl-s`/`ctrl-e`: they pass to the shell, which binds
   neither there (no visible effect).
8. The list hint row (Q1 a, PA-5) and the Runs browse hint mention `? keys` (Runs) as the way to
   the full list.

---

## 2. Plan claims found false or incomplete (for the record)

| Claim | Finding |
|---|---|
| Scope note "`detail/body.rs` and `prompt.rs` only scroll" | False for Prompt: `n`/`p` cycle the template, modifier-blind (PA-1). |
| 3 (11 app-level snapshots show the Backlog list) | False: 16. All five `concepts_search__*`, all five `shell__*`, `waiting__` 2, `prompt_preview__` 2, `integration__demo_shell`, `replay__runs_step_selected` show the list pane's bottom rows, where the Q1 (a) hint row lands. |
| 5 (inline hint literals in Runs, Graph, Reqs, Chat) | Incomplete: also `prompt.rs:143` (`n / p · N template(s)`, a key-naming header row) and `chat/mod.rs:259` (` · read-only · Esc leave`, the replay header). Both become label-built text, byte-identical with the defaults. |
| 14 / D6 (the miss is pinned by two tests; the files stay green) | Incomplete: five help-line assertions read the legacy rows (PA-3). |
| Risk row 4 (`Enter` takes the shadowing kind) | Wrong kind for fold/replay (PA-2). |
| Q1 (a)'s six acts fit ~53 columns | False: 60 cells (PA-5). |
| Q1 (a) cost "41 owned plus 11 app-level" | 28 owned (the 13 capturing-mode snapshots draw no list hint, §7.2) plus 16 app-level. |
| Lane gates vs "app-level snapshots are T-close's" | `replay__runs_step_selected` moves in L-B (PA-6). |
| Known couplings, bullet 3 ("status lines in capturing modes … show up in that lane's snapshots, as in M4") | True for L-A and L-D only. A Runs or Reqs capture reaches the status line through `BacklogTab::key_stack`, which is L-A's; on L-B's and L-C's branches the tab reports no stack, so those seven status rows move at the T-close merge (§7 step 1). |

---

## 3. T1 exact interfaces

### 3.1 `keys/catalogue.rs`: contexts (`Context::ALL` 29 → 40)

Appended after `Requirements`, in this order; `ALL`, `table`, `heading`, `is_view`, `is_shared`
and the test-only `context_position` (no wildcard) grow in lockstep. Order = the Backlog tab, its
sub-tabs in strip order (`BacklogTab::new`, `backlog/mod.rs:172-180`), its two forms, then Chat.

| Variant | `table()` | `heading()` | `is_view` | `is_shared` | Own rows |
|---|---|---|---|---|---|
| `Backlog` | `backlog` | `Backlog` | no | **yes** (like `settings`, `skills`) | 7 |
| `BacklogBody` | `backlog.body` | `Body` | yes | no | 0 (overrides only, PA-1) |
| `BacklogRuns` | `backlog.runs` | `Runs` | yes | no | 19 |
| `BacklogGraph` | `backlog.graph` | `Graph` | yes | no | 5 |
| `BacklogDocs` | `backlog.docs` | `Docs` | yes | no | 4 |
| `BacklogNotes` | `backlog.notes` | `Notes` | yes | no | 1 |
| `BacklogPrompt` | `backlog.prompt` | `Prompt` | yes | no | 2 (PA-1) |
| `BacklogReqs` | `backlog.reqs` | `Reqs` | yes | no | 8 |
| `BacklogFilter` | `backlog.filter` | `Filter` | yes | no | 3 |
| `BacklogItem` | `backlog.item` | `Item` | yes | no | 3 |
| `Chat` | `chat` | `Chat` | yes | no | 6 |

`Backlog` is shared, not a view: every Backlog browse stack carries it below the pane's view
layer, which inherits its acts (PA-3 of M3), so `[backlog.runs] filter = "/"` moves the filter key
in the Runs pane alone and `[backlog] filter` everywhere. The validator's per-context pass checks
`[backlog]`. Doc of `Context` (`catalogue.rs:37-40`): "M5 appends the Backlog and Chat contexts:
`backlog` (shared), one view context per Backlog sub-tab, the filter and item forms, and `chat`".
Module doc gains an M5 paragraph mirroring M4's.

### 3.2 `keys/catalogue.rs`: the 58 new acts (CATALOGUE 101 → 159)

All `row` (none is `in_capture`: none is offered beside a text field that would type it; the
filter's `j`/`k`/`x` and the Reqs `a`/`v` are typed only where no field captures, §4). Each row's
comment cites the arm it mirrors (file:line pre-edit). No default changes: every chord is today's.
Act variants are prefixed by their context so none shadows a `Context` variant imported by
`use Context::{…}` (`catalogue.rs:30-35`); the filter act is `BacklogOpenFilter`, not
`BacklogFilter`.

| # | `Act` | Context | name | defaults | help (`?` box) | mirrors |
|---|---|---|---|---|---|---|
| 1 | `BacklogReplay` | `Backlog` | `replay` | `["enter"]` | `replay step` | `backlog/mod.rs:791-797` (pane's `Enter`), `app/mod.rs:143-148` (the miss row, D6) |
| 2 | `BacklogOpenGraph` | `Backlog` | `open_graph` | `["m"]` | `open graph` | `backlog/mod.rs:762`, `app/mod.rs:152` |
| 3 | `BacklogOpenFilter` | `Backlog` | `filter` | `["f"]` | `filter` | `backlog/mod.rs:767` |
| 4 | `BacklogClearFilter` | `Backlog` | `clear_filter` | `["F"]` | `clear filter` | `backlog/mod.rs:770` |
| 5 | `BacklogNewItem` | `Backlog` | `new_item` | `["N"]` | `new item` | `backlog/mod.rs:778` (`N`, not `common.new`'s `n`: D2's "the architect's call") |
| 6 | `BacklogQueue` | `Backlog` | `queue` | `["Q"]` | `queue / dequeue` | `backlog/mod.rs:783` |
| 7 | `BacklogPauseQueue` | `Backlog` | `pause_queue` | `["P"]` | `pause / resume queue` | `backlog/mod.rs:784` |
| 8 | `RunsNextStep` | `BacklogRuns` | `next_step` | `["J"]` | `next step` | `runs.rs:1546` (ANA §6.2) |
| 9 | `RunsPrevStep` | `BacklogRuns` | `prev_step` | `["K"]` | `previous step` | `runs.rs:1550` |
| 10 | `RunsApprove` | `BacklogRuns` | `approve` | `["a"]` | `approve` | `runs.rs:1580` → `:757` |
| 11 | `RunsReject` | `BacklogRuns` | `reject` | `["x"]` | `reject` | `:764` |
| 12 | `RunsRetry` | `BacklogRuns` | `retry` | `["r"]` | `retry step` | `:771` |
| 13 | `RunsPromote` | `BacklogRuns` | `promote` | `["p"]` | `promote to chat` | `:774` |
| 14 | `RunsSelect` | `BacklogRuns` | `select` | `["s"]` | `select winner` | `:777` |
| 15 | `RunsAccept` | `BacklogRuns` | `accept` | `["A"]` | `accept output` | `:785` |
| 16 | `RunsOpen` | `BacklogRuns` | `open` | `["o"]` | `open output` | `:793` |
| 17 | `RunsCancelRun` | `BacklogRuns` | `cancel_run` | `["c"]` | `cancel run` | `:738` (`'c'`) |
| 18 | `RunsCleanup` | `BacklogRuns` | `cleanup` | `["T"]` | `retry cleanup` | `:738` (`'T'`) |
| 19 | `RunsUnblock` | `BacklogRuns` | `unblock` | `["u"]` | `unblock` | `:712` |
| 20 | `RunsStartRun` | `BacklogRuns` | `start_run` | `["R"]` | `start run` | `:717` |
| 21 | `RunsCloseOut` | `BacklogRuns` | `close_out` | `["C"]` | `close out` | `:727` |
| 22 | `RunsFollowUp` | `BacklogRuns` | `follow_up` | `["i"]` | `follow up` | `:708` |
| 23 | `RunsToggleView` | `BacklogRuns` | `toggle_view` | `["v"]` | `list / flow` | `:1560` |
| 24 | `RunsZoomIn` | `BacklogRuns` | `zoom_in` | `["+"]` | `zoom in` | `:1575` |
| 25 | `RunsZoomOut` | `BacklogRuns` | `zoom_out` | `["-"]` | `zoom out` | `:1576` |
| 26 | `RunsZoomFit` | `BacklogRuns` | `zoom_fit` | `["="]` | `fit the run` | `:1577` |
| 27 | `GraphNextRow` | `BacklogGraph` | `next_row` | `["J"]` | `next row` | `graph.rs:695` |
| 28 | `GraphPrevRow` | `BacklogGraph` | `prev_row` | `["K"]` | `previous row` | `graph.rs:696` |
| 29 | `GraphDeeper` | `BacklogGraph` | `deeper` | `["+"]` | `deeper` | `graph.rs:699` |
| 30 | `GraphShallower` | `BacklogGraph` | `shallower` | `["-"]` | `shallower` | `graph.rs:700` |
| 31 | `GraphReroot` | `BacklogGraph` | `reroot` | `["enter"]` | `re-root` | `graph.rs:701` (PA-2) |
| 32 | `DocsNextRow` | `BacklogDocs` | `next_row` | `["J"]` | `next row` | `documents.rs:461` |
| 33 | `DocsPrevRow` | `BacklogDocs` | `prev_row` | `["K"]` | `previous row` | `documents.rs:465` |
| 34 | `DocsAdd` | `BacklogDocs` | `add` | `["a"]` | `new document` | `documents.rs:469` |
| 35 | `DocsNewVersion` | `BacklogDocs` | `new_version` | `["v"]` | `new version` | `documents.rs:473` |
| 36 | `NotesAdd` | `BacklogNotes` | `add` | `["a"]` | `add note` | `notes.rs:303` |
| 37 | `PromptNextTemplate` | `BacklogPrompt` | `next_template` | `["n"]` | `next template` | `prompt.rs:329` (PA-1) |
| 38 | `PromptPrevTemplate` | `BacklogPrompt` | `prev_template` | `["p"]` | `previous template` | `prompt.rs:330` |
| 39 | `ReqsNextRow` | `BacklogReqs` | `next_row` | `["J"]` | `next row` | `detail/requirements.rs:478`, `:328` (picker `J`) |
| 40 | `ReqsPrevRow` | `BacklogReqs` | `prev_row` | `["K"]` | `previous row` | `:479`, `:331` |
| 41 | `ReqsReconfirm` | `BacklogReqs` | `reconfirm` | `["r"]` | `reconfirm` | `:480` |
| 42 | `ReqsCite` | `BacklogReqs` | `cite` | `["c"]` | `cite` | `:481` |
| 43 | `ReqsUncite` | `BacklogReqs` | `uncite` | `["u"]` | `uncite` | `:482` |
| 44 | `ReqsPick` | `BacklogReqs` | `pick` | `["enter"]` | `pick` | `:334` |
| 45 | `ReqsAddresses` | `BacklogReqs` | `addresses` | `["a"]` | `addresses` | `:280` |
| 46 | `ReqsReserves` | `BacklogReqs` | `reserves` | `["v"]` | `reserves` | `:281` |
| 47 | `FilterNextRow` | `BacklogFilter` | `next_row` | `["j"]` | `next row` | `filter.rs:250` (`'j'`; `Down`/`Tab` are `form.next_field`, §3.3) |
| 48 | `FilterPrevRow` | `BacklogFilter` | `prev_row` | `["k"]` | `previous row` | `filter.rs:251` |
| 49 | `FilterClear` | `BacklogFilter` | `clear` | `["x"]` | `clear` | `filter.rs:255` |
| 50 | `ItemTheirs` | `BacklogItem` | `theirs` | `["t", "T"]` | `theirs wins` | `divergence.rs:247` |
| 51 | `ItemMine` | `BacklogItem` | `mine` | `["m", "M"]` | `mine wins` | `divergence.rs:248` |
| 52 | `ItemSwitchDiff` | `BacklogItem` | `switch_diff` | `["tab", "backtab"]` | `body/paths` | `divergence.rs:249` |
| 53 | `ChatCompose` | `Chat` | `compose` | `["i", "enter"]` | `compose` | `chat/mod.rs:498` |
| 54 | `ChatNextAgent` | `Chat` | `next_agent` | `["a"]` | `next agent` | `chat/mod.rs:502` |
| 55 | `ChatEnd` | `Chat` | `end` | `["esc"]` | `end chat (twice)` | `chat/mod.rs:506` |
| 56 | `ChatThoughts` | `Chat` | `thoughts` | `["t"]` | `thoughts` | `transcript.rs:491` |
| 57 | `ChatTop` | `Chat` | `top` | `["g"]` | `top` | `transcript.rs:483` |
| 58 | `ChatBottom` | `Chat` | `bottom` | `["G"]` | `bottom` | `transcript.rs:487` |

Shared acts the M5 views use (all exist, defaults unchanged): `list.{down, up, top, bottom,
fold}`, `pane.{scroll_down, scroll_up, page_down, page_up, next_subtab, prev_subtab}`,
`confirm.{yes, no}`, `form.{next_field, prev_field, save, external_editor}`, `common.{edit,
back}`, `global.*`. Where they map: the Backlog list `j`/`k`/`g`/`G`/arrows/`Home`/`End` →
`list.*` (`backlog/mod.rs:754-757`); `Enter` on a header → `list.fold`; `h`/`l`/`[`/`]`/arrows →
`pane.prev_subtab`/`next_subtab` (`:758-759`); `e` → `common.edit` (`:779`); Body/Notes/Prompt
`J`/`K`/`PgDn`/`PgUp` → `pane.*` (`Scroll::on_key`); Runs/Docs/Reqs `PgDn`/`PgUp` → `pane.page_*`
(scroll), Graph's → `pane.page_*` (cursor by a page, `graph.rs:697-698`); the Runs document view
`J`/`j`/`K`/`k`/`PgDn`/`PgUp` → `pane.*` (`runs.rs:657-668`) and its `Esc` → `common.back`;
confirm questions (`runs.rs:639-645`, `:982-991`; `detail/requirements.rs:306-317`) →
`confirm.*`; the Reqs picker `j`/`Down`/`k`/`Up` → `list.down`/`up` and its and the kind
question's `Esc` → `common.back`; item form and Docs compose `Tab`/`BackTab`/`Down`/`Up` →
`form.next_field`/`prev_field`, `ctrl-s` → `form.save`, `ctrl-e` → `form.external_editor`; the
divergence view `j`/`Down`/`k`/`Up` → `list.down`/`up`, `PgDn`/`PgUp` → `pane.page_*`, `Esc` →
`common.back`; the chat transcript `j`/`Down`/`k`/`Up` → `list.down`/`up`; the replay's `Esc` →
`common.back` (`chat/mod.rs:233`, already cited).

Name checks (existing tests, must stay green): `shared_names_are_unique_across_shared_contexts`
(the seven `[backlog]` names clash with no `list`/`pane`/`confirm`/`form`/`common`/`settings`/
`skills` name: `new_item`, not `new`; `clear_filter`, not `clear`); `no_view_name_hides_a_shared
_name_it_inherits` (Chat's `top`/`bottom` are legal because no Chat stack offers `list.top`/
`list.bottom`; the filter's `clear` because no filter stack offers `common.clear`);
`no_spec_is_shared_in_a_context_unless_state_guarded` (checked per context above);
`names_are_unique_per_context`.

Comment refreshes while T1 is there (stale Backlog citations in shared rows): `list.down`
`backlog/mod.rs:655` → `:754`; `list.up` `:656` → `:755`; `list.top` `:657` → `:756`;
`list.bottom` `:658` → `:757`; `list.fold` `:684-689` → `:791-797` plus "the Backlog tries it
before the pane (MOD-67 M5 PA-2)"; `pane.next_subtab` `:659` → `:758`; `pane.prev_subtab`
`:660` → `:759`; `pane.scroll_down` "runs.rs:1434 `J` is next step" → "`runs.rs:1546` ...
(`backlog.runs.next_step`)"; `confirm.yes` `runs.rs:583` → `:640`; `confirm.no` `runs.rs:587` →
`:644`; `common.edit` `backlog/mod.rs:680` → `:779`; `common.new` "backlog/mod.rs:679 uses `N`" →
"the Backlog's `N` is `backlog.new_item`"; `form.next_field`/`prev_field`: + "the item form, the
Docs compose area and the filter form (M5 `VIEW_DEFAULTS` rows)"; `form.save`: + "the Backlog's
item form and Notes/Docs compose areas resolve it after their widgets (MOD-67 M5 D8)".

### 3.3 `VIEW_DEFAULTS` (+8; 20 → 28)

Order: `Context::ALL`, then `Act`. The first chord is the hint label (shared chords first).

| Context | Act | extra | derived with defaults | mirrors |
|---|---|---|---|---|
| `BacklogRuns` | `PaneScrollDown` | `["j"]` | `J, j` | `runs.rs:658` (the document view maps `j` to `J`) |
| `BacklogRuns` | `PaneScrollUp` | `["k"]` | `K, k` | `runs.rs:659` |
| `BacklogDocs` | `FormNextField` | `["down"]` | `tab, down` | `compose.rs:358` (kind/title pass `Down`) |
| `BacklogDocs` | `FormPrevField` | `["up"]` | `backtab, up` | `compose.rs:357` |
| `BacklogFilter` | `FormNextField` | `["down"]` | `tab, down` | `filter.rs:239`, `:250` |
| `BacklogFilter` | `FormPrevField` | `["up"]` | `backtab, up` | `filter.rs:238`, `:251` |
| `BacklogItem` | `FormNextField` | `["down"]` | `tab, down` | `item_form.rs:637`, `:723` |
| `BacklogItem` | `FormPrevField` | `["up"]` | `backtab, up` | `item_form.rs:636`, `:724` |

The Runs rows apply only in `RUNS_ARTIFACT` (the browse stack's pane layer omits the scroll acts,
so `J`/`K` stay the step cursor there). The Notes compose area has no next/prev rows (body only;
its `TextArea` takes `Up`/`Down`).

### 3.4 `SHADOWING` (1 → 2) and `STATE_GUARDED` (2 → 4) (PA-2)

```rust
pub static SHADOWING: &[(Act, Act)] = &[
    (Act::ConfirmNo, Act::OverlayClose),
    // graph.rs:680-686: the Graph pane answers `Enter` itself (re-root), always; the Backlog's
    // replay miss never fires there (MOD-67 M5 PA-2).
    (Act::GraphReroot, Act::BacklogReplay),
];
pub static STATE_GUARDED: &[(Act, Act)] = &[
    (Act::Back, Act::Dismiss),
    (Act::BoxesExecutor, Act::Workspaces),
    // backlog/mod.rs:791-797, ANA-26 §7.4: a project header folds; on an item row the fold
    // declines and the pane (Runs replays) or the miss answers. The tab tries the fold first.
    (Act::ListFold, Act::BacklogReplay),
    // On a header the Graph has no item; on an item row the fold declines.
    (Act::ListFold, Act::GraphReroot),
];
```

### 3.5 `keys/stack.rs`

- `DECLARED` order and phrases: §4.3 (76 entries). Doc: "M5 appends the Backlog and Chat stacks
  (in strip order), after M4's".
- No other change: `Stack::passes`, `actions` and `admits` serve every M5 stack as they are.

### 3.6 `keys/load.rs`, `keys/validate.rs`, `keys/print.rs`, `keys/hint.rs`

**No code change.** `backlog` is shared through `Context::is_shared` alone (`load.rs:257-265`
`overridable` reads it; the validator's per-context pass checks every non-view context);
`--print-keys` iterates `Context::ALL`; every M5 hint renders with M3's `HintSpec` elements plus
`Keys::label` (the two doubled/joined labels, §5 of L-C and L-D). Only test expectations move
(§5.3).

### 3.7 `ui/tabs/backlog/detail/mod.rs`: the pane's stack (D4)

```rust
pub trait DetailTab {
    // …
    /// The stack of this sub-tab's current mode (MOD-67 M5 D4): what the Backlog tab resolves
    /// keys through and what the status line and `?` box show while it is the active pane. The
    /// default is Body's browse stack, [`views::BACKLOG_BODY`]: the pane's own verbs none, the
    /// Backlog's and the list's all. A converted pane returns its mode's stack.
    fn key_stack(&self) -> Stack<'static> {
        views::BACKLOG_BODY
    }
}

impl DetailRegistry {
    /// The active sub-tab's stack, or [`views::BACKLOG_BODY`] while the registry is empty.
    #[must_use]
    pub fn key_stack(&self) -> Stack<'static> {
        self.active().map_or(views::BACKLOG_BODY, DetailTab::key_stack)
    }
}
```
Imports `crate::keys::{Stack, views}`. `BodyTab` keeps the default (L-C writes no override). No
`BacklogTab::key_stack` in T1 (stays `None`, plan T1). `Scroll::on_key` stays until T-close.

---

## 4. `keys/views.rs`: every M5 stack

Module doc gains: "M5 adds the Backlog and Chat stacks. A Backlog browse stack is the pane's view
layer, then the tail every sub-tab shares: `pane` (narrowed per pane), `backlog`,
`common ∩ {edit}`, `list`, global (ANA-26 §7.3); a Backlog capturing or asking mode is its view
layer, the shared layers it offers, `MODAL`. Layer order: view, `pane`, `backlog`, `confirm`,
`form`, `common`, `list`, global."

New private layer constants:
```rust
/// Every `pane` act: a sub-tab that scrolls (Body, Notes, Prompt) and cycles sub-tabs.
const PANE: Layer = Layer::all(Context::Pane);
/// `pane ∩ {page_down, page_up, next_subtab, prev_subtab}`: a sub-tab whose `J`/`K` move a cursor
/// of its own (Runs, Graph, Docs, Reqs).
const PANE_PAGE: Layer = Layer::only(Context::Pane, &[Act::PanePageDown, Act::PanePageUp,
    Act::PaneNextSubtab, Act::PanePrevSubtab]);
/// `pane ∩ {page_down, page_up}`.
const PAGES: Layer = Layer::only(Context::Pane, &[Act::PanePageDown, Act::PanePageUp]);
/// The Backlog tab's own verbs (MOD-67 M5 D1), a shared context every Backlog browse stack has.
const BACKLOG: Layer = Layer::all(Context::Backlog);
/// `common ∩ {edit}`: the Backlog's `e`.
const EDIT: Layer = Layer::only(Context::Common, &[Act::Edit]);
/// `common ∩ {back}`.
const BACK: Layer = Layer::only(Context::Common, &[Act::Back]);
/// Every `list` act: the Backlog list's moves and fold.
const LIST_ALL: Layer = Layer::all(Context::List);
/// `confirm ∩ {no}`: a wait that can only be left.
const NO: Layer = Layer::only(Context::Confirm, &[Act::ConfirmNo]);
/// Every `form` act: a multi-field form with a `TextArea` (the item form, the Docs compose area).
const FORM: Layer = Layer::all(Context::Form);
```
(`MODAL`, `GLOBAL`, `LIST`, `CONFIRM`, `FORM_FIELDS`, `FORM_EDITOR`, `PANE_SCROLL` are M3/M4's.)
Each Backlog browse stack's tail is written out (a `const` slice cannot be concatenated in a
`const` context); T-V3 (§5.1) pins that all seven share it.

### 4.1 The stacks (20 new `pub const`s)

Notation: `view(C, own)` = `Layer::view`; constants as above. "Returned by" is the lane's
`key_stack`.

| # | Constant | Layers, narrowest first | `DECLARED` phrase | Returned by |
|---|---|---|---|---|
| 1 | `BACKLOG_BODY` | `view(BacklogBody, [])`, `PANE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Body` | `DetailTab::key_stack` default (T1): Body; any pane until its lane lands |
| 2 | `RUNS_BROWSE` | `view(BacklogRuns, [RunsNextStep, RunsPrevStep, RunsApprove, RunsReject, RunsRetry, RunsPromote, RunsSelect, RunsAccept, RunsOpen, RunsCancelRun, RunsCleanup, RunsUnblock, RunsStartRun, RunsCloseOut, RunsFollowUp, RunsToggleView, RunsZoomIn, RunsZoomOut, RunsZoomFit])`, `PANE_PAGE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Runs` | Runs `Browse` (list and flow) |
| 3 | `RUNS_CONFIRM` | `view(BacklogRuns, [])`, `CONFIRM`, `MODAL` | `in a Runs question` | `ConfirmCancel`, `CloseOut(Warn)` |
| 4 | `RUNS_COUNTING` | `view(BacklogRuns, [])`, `NO`, `MODAL` | `while the Runs close-out counts` | `CloseOut(Counting)` |
| 5 | `RUNS_ARTIFACT` | `view(BacklogRuns, [])`, `PANE_SCROLL`, `BACK`, `MODAL` | `in the Runs document view` | `Artifact` |
| 6 | `BACKLOG_GRAPH` | `view(BacklogGraph, [GraphNextRow, GraphPrevRow, GraphDeeper, GraphShallower, GraphReroot])`, `PANE_PAGE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Graph` | Graph (one mode) |
| 7 | `BACKLOG_DOCS` | `view(BacklogDocs, [DocsNextRow, DocsPrevRow, DocsAdd, DocsNewVersion])`, `PANE_PAGE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Docs` | Docs, no area |
| 8 | `DOCS_COMPOSE` | `view(BacklogDocs, [])`, `FORM`, `MODAL` | `in the Docs compose area` | Docs, area open (`Compose::stack`) |
| 9 | `BACKLOG_NOTES` | `view(BacklogNotes, [NotesAdd])`, `PANE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Notes` | Notes, no area |
| 10 | `NOTES_COMPOSE` | `view(BacklogNotes, [])`, `FORM_EDITOR`, `MODAL` | `in the Notes compose area` | Notes, area open (`Compose::stack`) |
| 11 | `BACKLOG_PROMPT` | `view(BacklogPrompt, [PromptNextTemplate, PromptPrevTemplate])`, `PANE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Prompt` | Prompt |
| 12 | `BACKLOG_REQS` | `view(BacklogReqs, [ReqsNextRow, ReqsPrevRow, ReqsReconfirm, ReqsCite, ReqsUncite])`, `PANE_PAGE`, `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` | `in Backlog > Reqs` | Reqs `Browse` |
| 13 | `REQS_PICK` | `view(BacklogReqs, [ReqsNextRow, ReqsPrevRow, ReqsPick])`, `BACK`, `LIST`, `MODAL` | `in the Reqs requirement picker` | `Pick` |
| 14 | `REQS_KIND` | `view(BacklogReqs, [ReqsAddresses, ReqsReserves])`, `BACK`, `MODAL` | `in the Reqs citation kind question` | `Kind` |
| 15 | `REQS_CONFIRM` | `view(BacklogReqs, [])`, `CONFIRM`, `MODAL` | `in the Reqs uncite question` | `ConfirmUncite` |
| 16 | `BACKLOG_FILTER` | `view(BacklogFilter, [FilterNextRow, FilterPrevRow, FilterClear])`, `FORM_FIELDS`, `MODAL` | `in the Backlog filter form` | the filter form (both row kinds) |
| 17 | `ITEM_FORM` | `view(BacklogItem, [])`, `FORM`, `MODAL` | `in the item form` | the item form (also busy) |
| 18 | `ITEM_DIVERGENCE` | `view(BacklogItem, [ItemTheirs, ItemMine, ItemSwitchDiff])`, `PAGES`, `BACK`, `LIST`, `MODAL` | `in the item divergence view` | the divergence view |
| 19 | `CHAT_LIVE` | `view(Chat, [ChatCompose, ChatNextAgent, ChatEnd, ChatThoughts, ChatTop, ChatBottom])`, `LIST`, `GLOBAL` | `in Chat` | idle, live, parked, ended (PA-4) |
| 20 | `CHAT_REPLAY` | `view(Chat, [ChatThoughts, ChatTop, ChatBottom])`, `BACK`, `LIST`, `GLOBAL` | `in a Chat replay` | replay |

M3's `CAPTURE` (`[MODAL]`) also serves: the Runs reject note, follow-up and typed close-out key
(`TextField`s and nothing else), the close-out in flight (waits, swallows), and the open chat
composer (PA-4). Its doc gains "the Runs note, follow-up and typed key, a close-out in flight and
the chat composer (MOD-67 M5)".

Collision check with the defaults, chord by chord through every layer (filtered): the only chord
with two or more candidates is `enter` in the seven Backlog browse stacks (`[BacklogReplay,
ListFold]`; in `BACKLOG_GRAPH` `[GraphReroot, BacklogReplay, ListFold]`), each pair on §3.4's
lists. Spot checks: `RUNS_BROWSE` `J K a x r p s A o c T u R C i v + - = | PgDn PgUp l ] Right h [
Left | Enter m f F N Q P | e | j Down k Up g Home G End Enter | q Tab BackTab 1-9 ? F1 w ctrl-f
ctrl-w ctrl-q`; `BACKLOG_FILTER` `j k x | Tab Down BackTab Up | ctrl-f ctrl-w ctrl-q F1`;
`ITEM_DIVERGENCE` `t T m M Tab BackTab | PgDn PgUp | Esc | j Down k Up | MODAL`; `CHAT_LIVE` `i
Enter a Esc t g G | j Down k Up | global`. `the_compiled_defaults_validate` holds.

Files a user may have today that M5 refuses (D10, and nothing else): a `[global]` key on any
Backlog or Chat default above (`f`, `F`, `N`, `m`, `Q`, `P`, `e`, `J`, `K`, `h`, `l`, `[`, `]`,
`j`, `k`, `g`, `G`, `Enter`, `Esc`, the Runs letters, `i`, `t`, `a`, …). `[form] next_field =
["down"]` still loads (the filter form's `j`/`k` are view acts, so no `list` layer meets a form
layer; §3.2 #47).

### 4.2 Resolution pins (T1, new `views.rs` test `the_backlog_and_chat_stacks_resolve_as_designed`)

`BACKLOG_BODY`: `j` → `[ListDown]`, `J` → `[PaneScrollDown]`, `l` → `[PaneNextSubtab]`, `m` →
`[BacklogOpenGraph]`, `f` → `[BacklogOpenFilter]`, `e` → `[Edit]`, `enter` → `[BacklogReplay,
ListFold]`, `q` → `[Quit]`, `ctrl-l`/`alt-j`/`ctrl-j` → `[]`.
`RUNS_BROWSE`: `J` → `[RunsNextStep]`, `x` → `[RunsReject]`, `+` → `[RunsZoomIn]`, `pgdn` →
`[PanePageDown]`, `enter` → `[BacklogReplay, ListFold]`, `1` → `[SelectTab1]`, `j` → `[ListDown]`.
`RUNS_CONFIRM`: `y` → `[ConfirmYes]`, `esc` → `[ConfirmNo]`, `alt-y` → `[]`, `q` → `[]`,
`!passes(y)`, `passes(alt-y)`, `passes(f1)`. `RUNS_COUNTING`: `y` → `[]`, `n` → `[ConfirmNo]`.
`RUNS_ARTIFACT`: `j` and `J` → `[PaneScrollDown]`, `esc` → `[Back]`, `down` → `[]`.
`BACKLOG_GRAPH`: `enter` → `[GraphReroot, BacklogReplay, ListFold]`, `J` → `[GraphNextRow]`,
`pgdn` → `[PanePageDown]`. `BACKLOG_DOCS`: `v` → `[DocsNewVersion]`, `J` → `[DocsNextRow]`.
`DOCS_COMPOSE`: `down` → `[FormNextField]`, `ctrl-s` → `[FormSave]`, `ctrl-e` →
`[FormExternalEditor]`, `q` → `[]`, `f1` → `[Help]`. `NOTES_COMPOSE`: `tab` → `[]`, `ctrl-s` →
`[FormSave]`. `BACKLOG_NOTES`: `a` → `[NotesAdd]`, `K` → `[PaneScrollUp]`. `BACKLOG_PROMPT`: `n`
→ `[PromptNextTemplate]`, `ctrl-n` → `[]`. `BACKLOG_REQS`: `c` → `[ReqsCite]`.
`REQS_PICK`: `J` → `[ReqsNextRow]`, `j` → `[ListDown]`, `enter` → `[ReqsPick]`, `esc` → `[Back]`.
`REQS_KIND`: `a` → `[ReqsAddresses]`, `esc` → `[Back]`. `REQS_CONFIRM`: `alt-y` → `[]`.
`BACKLOG_FILTER`: `x` → `[FilterClear]`, `j` → `[FilterNextRow]`, `down` and `tab` →
`[FormNextField]`, `q` → `[]`. `ITEM_FORM`: `down` → `[FormNextField]`, `ctrl-s` → `[FormSave]`,
`f1` → `[Help]`, `q` → `[]`. `ITEM_DIVERGENCE`: `T` → `[ItemTheirs]`, `tab` →
`[ItemSwitchDiff]`, `j` → `[ListDown]`, `esc` → `[Back]`, `ctrl-s` → `[]`. `CHAT_LIVE`: `enter`
→ `[ChatCompose]`, `esc` → `[ChatEnd]`, `home` → `[]`, `1` → `[SelectTab1]`, `q` → `[Quit]`.
`CHAT_REPLAY`: `esc` → `[Back]`, `i` → `[]`, `t` → `[ChatThoughts]`.

### 4.3 `DECLARED` (56 → 76)

| Index | Phrase | Stack |
|---|---|---|
| 0-55 | unchanged (M1-M4) | |
| 56-75 | §4.1 rows 1-20, in that order | |

Appended, so every existing first-reported phrase is unchanged (PA-1 (b)). `views.rs`
`every_view_layer_comes_first_and_every_modal_stack_ends_modal` keeps slicing `DECLARED[6..]` and
expects **70**; `every_view_stack_is_named_by_a_view` keeps `DECLARED.len() - 4` (72 `pub const`s).

---

## 5. T1 tests (write red first) and the existing tests T1 changes

### 5.1 New and grown unit tests

| Id | File | Test | Asserts |
|---|---|---|---|
| T-C1 | `catalogue.rs` | `every_act_is_listed_and_has_a_row` (`ALL`/`position` +58), `every_act_has_exactly_one_row` (**159**) | lockstep |
| T-C2 | `catalogue.rs` | `every_context_is_listed_once_in_all` (**40**), `context_tables_and_headings` (+11 rows of §3.1; `Backlog` shared and not a view; the ten others views), `catalogue_blocks_follow_context_all` | §3.1 |
| T-C3 | `catalogue.rs` | `in_capture_acts_are_…` | unchanged set (no M5 act is `in_capture`) |
| T-V1 | `views.rs` | `the_backlog_and_chat_stacks_resolve_as_designed` (new) | §4.2 |
| T-V2 | `views.rs` | `every_view_layer_comes_first_and_every_modal_stack_ends_modal` | `DECLARED[6..]`, **70** |
| T-V3 | `views.rs` | `every_backlog_browse_stack_offers_the_tab_and_the_list` (new) | for each of `BACKLOG_{BODY,GRAPH,DOCS,NOTES,PROMPT,REQS}` and `RUNS_BROWSE`: `f` → `[BacklogOpenFilter]`, `N` → `[BacklogNewItem]`, `e` → `[Edit]`, `l` → `[PaneNextSubtab]`, `G` → `[ListBottom]`, `enter` contains `BacklogReplay` and `ListFold`, `q` → `[Quit]`; the stack's last four layers are `BACKLOG`, `EDIT`, `LIST_ALL`, `GLOBAL` (D5's tab acts work on every sub-tab) |
| T-V4 | `views.rs` | `every_view_stack_is_named_by_a_view` | `VIEWS` gains `backlog/{mod,filter,item_form,divergence}.rs`, `backlog/detail/{mod,compose,documents,graph,notes,prompt,requirements,runs}.rs`, `chat/mod.rs`; `DECLARED.len() - 4`; a `PENDING: &[&str]` of the 19 lane stacks (§4.1 rows 2-20) is skipped, doc "MOD-67 M5: the lanes name these; T-close empties the list". `BACKLOG_BODY` is named by T1's `detail/mod.rs` |
| T-V5 | `views.rs` | `no_capturing_stack_offers_the_view_switch`, `no_view_name_hides_a_shared_name_it_inherits`, `every_view_default_is_an_extra_chord_some_stack_offers` | unchanged bodies, green over the new stacks |
| T-S1 | `stack.rs` | `declared_starts_with_base_and_overlay_and_holds_every_view_stack` | `len() == 76`, phrases unique |
| T-X1 | `validate.rs` | `the_compiled_defaults_validate`, `every_allow_list_entry_is_demanded` | over 76 stacks and the two grown lists (bodies unchanged) |
| T-L1 | `load.rs` | `a_backlog_pane_table_overrides_a_backlog_verb` (new) | `[backlog.runs]\nfilter = "/"\n` loads; `actions(RUNS_BROWSE, "/")` = `[BacklogOpenFilter]`, `actions(RUNS_BROWSE, "f")` = `[]`, `actions(BACKLOG_BODY, "f")` = `[BacklogOpenFilter]`; `[backlog]\nfilter = "/"\n` moves it in every Backlog browse stack; `[chat]\nfilter = "/"\n` → the "no such action; [chat] has compose, next_agent, end, thoughts, top, bottom, and may override …" message (names from the run) |
| T-L2 | `load.rs` | `an_unknown_table_lists_the_tables` + `TABLES` | `TABLES` ends `…, requirements, backlog, backlog.body, backlog.runs, backlog.graph, backlog.docs, backlog.notes, backlog.prompt, backlog.reqs, backlog.filter, backlog.item, chat` |
| T-P1 | `print.rs` | `the_default_print_starts_with_the_header_global_and_overlay` | ends with the `[chat]` table (§5.3); still ≤ 100 columns |
| T-D1 | `backlog/detail/mod.rs` | `the_registry_reports_the_active_panes_stack` (new) | an empty registry → `BACKLOG_BODY`; a probe overriding `key_stack` → its stack while active, `BACKLOG_BODY` after `cycle_next` onto a default probe |

### 5.2 New and flipped integration tests (T1)

| File | Test | Asserts |
|---|---|---|
| `tests/keys.rs` | `the_readme_backlog_example_loads_and_f_stays_the_backlogs` → **`the_readme_backlog_example_is_refused_naming_the_backlog`** (D10 flip) | `load_str("version = 1\n\n[global]\nquit = [\"f\"]\n")` is `Err` with exactly `[(4, "[global] quit = \"f\": \"f\" is already backlog.filter (default) in Backlog > Body")]`. The `Harness` half goes (it pinned the M4 behaviour). Doc: "MOD-67 M5 D10: the Backlog reads the file, so a global key on its `f` is refused, naming the Backlog." |
| `tests/backlog_keys.rs` + `tests/backlog_keys/{l_a,l_b,l_c}.rs` | seeded (§5.4) | each lane file holds one placeholder test |

### 5.3 Existing tests and fixtures T1 changes (and only these)

- `catalogue.rs`, `stack.rs`, `views.rs`, `load.rs` (`TABLES`), `print.rs` as §5.1.
- `load.rs` `every_error_is_reported_and_sorted_by_line` and `tests/keys_file.rs` `bad_fixtures`
  "errors": line 13 gains, after the `skills.attach.detach` line, `[global] quit = "x": "x" is
  already backlog.runs.reject (default) in Backlog > Runs`; the `[globl]` table list gains the
  eleven tables.
- `validate.rs` `a_global_rebind_onto_a_view_verb_is_refused`: + the same `backlog.runs.reject`
  line (third).
- `validate.rs` `a_collision_over_an_overlay_is_found_in_the_overlay_stack` and `tests/keys_file.rs`
  `bad_fixtures` "collision" (line 6): + `[global] help = "esc": "esc" is already chat.end
  (default) in Chat`, last of the `esc` lines (`common.back` is already reported for the Library
  import report, so `CHAT_REPLAY` adds nothing).
- `stack.rs` `a_global_chord_on_the_view_switch_names_the_skills_views`: + third message `[global]
  help = "h": "h" is already pane.prev_subtab (default) in Backlog > Body` (comment: "MOD-67 M5:
  and the Backlog's sub-tab cycling").
- `print.rs` tail (exact padding from the run; expected shape):
  ```
  [chat]
  compose    = ["i", "enter"]  # compose
  next_agent = ["a"]           # next agent
  end        = ["esc"]         # end chat (twice)
  thoughts   = ["t"]           # thoughts
  top        = ["g"]           # top
  bottom     = ["G"]           # bottom
  ```
- `tests/keys.rs` D10 flip (§5.2). No fixture file changes; README untouched in T1 (T-close).
- Checked and unchanged: `a_global_entry_on_a_list_default_is_refused_where_a_stack_composes_them`
  (the pair is reported once, in Settings > Agents, first in `DECLARED`),
  `an_in_capture_action_refuses_a_printable_chord` (`[form] next_field = ["down"]` loads, §4.1),
  `tests/requirements.rs:483` (`alt-q`), the `ctrl-y` fixtures, the box/skills/templates `f2`
  files.
- No snapshot moves in T1.

### 5.4 `tests/backlog_keys.rs` (seed)

```rust
//! MOD-67 M5 (D12): the Backlog's keys through its context stacks, end to end through
//! `Harness`. One module per lane, each in its own file under `tests/backlog_keys/`, so the
//! lanes' edits never meet: `l_a` (the tab, its forms, Notes and Docs), `l_b` (Runs), `l_c`
//! (Body, Graph, Prompt, Reqs). T-close adds the merged-tree tests after the three modules.
#![cfg(feature = "testkit")]

#[path = "backlog_keys/l_a.rs"]
mod l_a;
#[path = "backlog_keys/l_b.rs"]
mod l_b;
#[path = "backlog_keys/l_c.rs"]
mod l_c;
```
Each `tests/backlog_keys/l_<x>.rs`:
```rust
//! Lane L-<X> of MOD-67 M5: <scope>. The lane replaces the placeholder with its D12 pins and its
//! rebinding test; helpers stay in this file.

/// Placeholder so the module compiles before the lane lands (MOD-67 M5 T1); the lane deletes it.
#[test]
fn lane_seeded() {}
```
No `use` in the seeds (unused imports fail clippy). Files under a subdirectory of `tests/` are not
test targets of their own. L-D's tests live in `tests/chat.rs` (plan).

### 5.5 T1 commit groups (each compiles, clippy both ways, its tests green)

1. **T1a** `feat(mod-67): M5 Backlog and Chat contexts and acts` — `catalogue.rs` §3.1-§3.2,
   comment refreshes, T-C1-T-C3. Lands the 58 acts unused (`pub enum` variants are not dead
   code). `TABLES` (load.rs), the print tail and the `[globl]` table lists in `load.rs` and
   `tests/keys_file.rs` move here (they follow `Context::ALL`).
2. **T1b** `feat(mod-67): M5 Backlog and Chat stacks` — `views.rs` §4 (with `PENDING`),
   `stack.rs` §4.3, `VIEW_DEFAULTS` §3.3 and the allow-lists §3.4 (`catalogue.rs`: they are only
   demanded once the stacks exist), T-V1-T-V5, T-S1, T-X1, T-L1, and every expectation of §5.3
   that a new stack adds a line to; the D10 flip in `tests/keys.rs`.
3. **T1c** `feat(mod-67): the Backlog detail pane reports its stack (D4)` —
   `backlog/detail/mod.rs` §3.7, T-D1; `BACKLOG_BODY` leaves `PENDING`.
4. **T1d** `test(mod-67): seed tests/backlog_keys for the M5 lanes` — §5.4.

### 5.6 T1 gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib keys
cargo test -p htui --lib ui::tabs::backlog::detail
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --features testkit \
  --test keys --test keys_file --test backlog_keys -- --test-threads=1
cargo insta test -p htui --all-features --check     # nothing pending: T1 moves no snapshot
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
```
The last line once before the lanes fork; grep its output for `SIGABRT` (memory: engine stack
headroom) and check for orphaned test processes.

---

## 6. Dispatch skeletons for M5 (the lanes copy these; M3 §6 and M4 §6 otherwise)

Common: `let chord = KeyChord::from_event(key);` once; match on `Act`, never on `key.code`
outside a widget or a fixed key of D5; `ctx.keys().actions(STACK, chord)`; delete every
`KeyModifiers` guard the resolver makes redundant (chord equality includes modifiers); keep a
local `fn plain(key: &KeyEvent) -> bool` (no CONTROL/ALT/SUPER/META/HYPER) only where a fixed key
is matched by code (digits, replay's swallow, the pickers', options' and resolution's value keys,
the filter form's `Enter`/`Esc` on an option row). `modal_rest` is
`crate::ui::tabs::settings::modal_rest` (`pub(crate)`): `Pass` iff `stack.passes(chord)`, else
`Consumed`. Pure modules (`filter`, `item_form`, `divergence`, `compose`) take `keys: &Keys`
beside the key (`Keys::compiled()` in their in-file tests), as M3's `hint_text(keys)` did.

### 6.1 The Backlog tab (L-A): forms, capture, fold, pane, tab

```rust
fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    if self.form.is_some() {
        return self.on_form_key(key, ctx);       // BACKLOG_FILTER, §L-A 2.3
    }
    if self.item_form.is_some() {
        return self.on_item_form_key(key, ctx);  // ITEM_FORM / ITEM_DIVERGENCE, §L-A 2.4
    }
    if self.detail.captures_input() {
        return self.detail.on_key(key, ctx);     // the pane's capture stack decides Pass
    }
    let stack = self.detail.key_stack();         // the active pane's browse stack
    let chord = KeyChord::from_event(key);
    let acts = ctx.keys().actions(stack, chord);
    // D6, PA-2: a project header folds before any pane sees `Enter` (today's order, :791-797).
    if acts.contains(&Act::ListFold) && self.fold(ctx) == Handled::Consumed {
        return Handled::Consumed;
    }
    // D5: the pane first; it takes its own acts (and the digits) and declines the rest.
    if self.detail.on_key(key, ctx) == Handled::Consumed {
        return Handled::Consumed;
    }
    for act in acts {
        match act {
            Act::ListDown => self.step(1, ctx),
            Act::ListUp => self.step(-1, ctx),
            Act::ListTop => self.jump(false, ctx),
            Act::ListBottom => self.jump(true, ctx),
            Act::PaneNextSubtab => self.detail.cycle_next(),
            Act::PanePrevSubtab => self.detail.cycle_prev(),
            Act::BacklogOpenGraph => _ = self.detail.select_id(GraphTab::ID),
            Act::BacklogOpenFilter => self.form = Some(FilterForm::open(&self.filter, ctx.projects)),
            Act::BacklogClearFilter => {
                if !self.filter.is_empty() {
                    self.apply(BacklogFilter::default(), ctx);
                }
            }
            Act::BacklogNewItem => self.open_new(ctx),
            Act::Edit => self.open_edit(ctx),
            Act::BacklogQueue => self.toggle_queued(ctx),
            Act::BacklogPauseQueue => {
                self.queue_intent = Some(QueueIntent::Pause);
                ctx.request(StoreRequest::QueueState);
            }
            // D6: no pane took `Enter` (the fold declined above): the miss, keys named.
            Act::BacklogReplay => ctx.emit(Action::Error(replay_miss(ctx.keys()))),
            _ => continue, // the declined fold, a pane act the pane declined, a global act
        }
        return Handled::Consumed;
    }
    Handled::Pass // the shell resolves the same stack (q, Tab, digits, ?, w, …)
}

/// The D6 miss: "select a step in the Runs pane (J/K) to replay it" under the defaults, the step
/// keys read through the Runs browse stack (the miss fires from any sub-tab).
fn replay_miss(keys: &Keys) -> String {
    const STEP_KEYS: HintSpec = &[Hint::Pair(Act::RunsNextStep, Act::RunsPrevStep, "")];
    match keys.hint(views::RUNS_BROWSE, STEP_KEYS) {
        moves if moves.is_empty() => "select a step in the Runs pane to replay it".to_owned(),
        moves => format!("select a step in the Runs pane ({moves}) to replay it"),
    }
}

fn key_stack(&self) -> Option<Stack<'static>> {
    if self.form.is_some() {
        return Some(views::BACKLOG_FILTER);
    }
    if let Some(form) = &self.item_form {
        return Some(form.key_stack()); // ITEM_DIVERGENCE while resolving, else ITEM_FORM
    }
    Some(self.detail.key_stack())
}
```
Equivalence with today (D5): the validator refuses a chord bound to a pane verb and a tab verb in
one stack, so pane-first and tab-first pick the same act for every chord except `Enter`, which
the fold rule keeps in today's order. The one observable difference is user-visible item 1 (§1).

### 6.2 A pane in browse (L-A Notes/Docs, L-B Runs, L-C Body/Graph/Prompt/Reqs)

```rust
fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    if !matches!(self.mode, Mode::Browse) {
        return self.modal_key(key, ctx);           // §6.3 / §6.4 per mode
    }
    // Fixed keys of D5 first (Runs only: the permission digits, plain keys, `runs.rs:1528`).
    let chord = KeyChord::from_event(key);
    for act in ctx.keys().actions(self.key_stack(), chord) {
        match act {
            Act::RunsNextStep => { self.move_cursor(1); self.sync_graph(ctx.theme); }
            // D6: the pane's half of `Enter`; with no step under the cursor it declines and the
            // Backlog tab answers with the miss.
            Act::BacklogReplay => match self.selected_step() {
                Some(step_id) => ctx.emit(Action::Replay { step_id }),
                None => continue,
            },
            Act::PanePageDown | Act::PanePageUp => {
                return self.scroll.apply(act, self.runs.len());
            }
            _ => continue, // list.*, pane.*_subtab, backlog.*, common.edit: the tab's
        }
        return Handled::Consumed;
    }
    Handled::Pass
}

/// The current mode's stack (MOD-67 M5 D4): the only place a mode maps to its keys.
fn key_stack(&self) -> Stack<'static> { /* per lane file */ }
```
A pane never handles `list.*`, `pane.next_subtab`/`prev_subtab`, `backlog.*` or `common.edit`
(the tab's); Runs is the one pane that accepts a `backlog` act (`backlog.replay`).

### 6.3 A capturing mode with a widget (filter Tags row, item form, compose, Runs fields)

M4 §6.2 with M4 PA-3's order: the widget first, then the resolver on `Pass`.
```rust
match widget.on_key(key /*, page */) {
    FieldOutcome::Consumed => return Stay,
    FieldOutcome::Submit => …,     // a TextField's Enter: the mode's own (move on / send / reject)
    FieldOutcome::Cancel => …,     // Esc
    FieldOutcome::Pass => {}
}
match keys.actions(stack, chord).first() {
    Some(Act::FormSave) => self.save(),                // after the widget: a rebound plain save
    Some(Act::FormExternalEditor) => self.hand_off(),  // (`f2`) still reaches the resolver
    Some(Act::FormNextField) => self.move_focus(true),
    Some(Act::FormPrevField) => self.move_focus(false),
    _ => if stack.passes(chord) { Pass } else { Stay }, // `modal_rest`
}
```
A picker or an option row (no text widget) matches its fixed value keys on **plain** keys only,
then returns `Pass` for everything else, so `Up`/`Down`/`Tab`, a rebound `f2` save and `F1`
reach the resolver. While a form or area is busy, `FormSave`/`FormExternalEditor` are swallowed
(today's `Stay`) and `modal_rest` decides the rest. The Runs fields use `CAPTURE` (no own act):
field first, then `modal_rest(views::CAPTURE, chord)`.

### 6.4 A question or a non-text modal (Runs questions/artifact, Reqs picker/kind/uncite, divergence)

M3 §6.4: resolve the mode's stack; own and shared acts consume; else `modal_rest` (today's
swallow of plain keys, now with `F1`/CONTROL/ALT passing). A fixed value key (the close-out
resolution) is matched on plain keys before the resolver.

### 6.5 Chat (L-D)

```rust
fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    self.refusal = None;
    let chord = KeyChord::from_event(key);
    if self.replay.is_some() {
        // D5: the replay swallow stays fixed and first (plain `1`-`9`, `i`, `a`, `Enter`).
        if plain(&key) && matches!(key.code, KeyCode::Char('1'..='9' | 'i' | 'a') | KeyCode::Enter) {
            return Handled::Consumed;
        }
        return self.on_replay_act(chord, ctx);   // CHAT_REPLAY: Back closes, transcript acts
    }
    if self.composer.is_active() {
        if views::CAPTURE.passes(chord) {
            return Handled::Pass;                // ctrl/alt/F-keys: the shell's (F1 now helps)
        }
        return match self.composer.on_key(key) { /* unchanged arms */ };
    }
    // D5: the parked request's digits, plain keys only, ahead of the resolver.
    if plain(&key) && let KeyCode::Char(digit @ '1'..='9') = key.code && self.transcript.parked().is_some() {
        self.answer(digit, ctx);
        self.cancel_armed = false;
        return Handled::Consumed;
    }
    let first = ctx.keys().actions(views::CHAT_LIVE, chord).first().copied();
    let handled = match first { /* ChatCompose, ChatNextAgent (declined without agents or with a
        session → Pass), ChatEnd (arm/cancel), transcript acts, else Pass */ };
    if first != Some(Act::ChatEnd) && plain(&key) {
        self.cancel_armed = false;               // today: any key but Esc disarms; chords did not
    }
    handled
}
```

---

## 7. T-close (serial, primary tree)

1. **Merge** `mod-67-m5-l-a`, then `-l-b`, `-l-c`, `-l-d`, each `--no-ff`; after each, the full
   Validation (plan) with `--test-threads=1 --no-fail-fast` and `cargo insta test -p htui
   --all-features --check`. L-A first: it owns `BacklogTab::key_stack`, without which the other
   lanes' stacks reach neither the status line nor the `?` box. That is also why the status rows
   of L-B's and L-C's capturing snapshots cannot move on their own branches (the unconverted tab
   reports no stack, so the shell draws `Stack::BASE`'s line): they move **at their merge**, and
   X2/X3 accept exactly those rows, reviewed line by line — after X2 (L-B) the status row of
   `backlog__runs_{reject_note,flow_reject_note,follow_up_input,closeout_typed,closeout_warn,
   artifact}` becomes `Ctrl+c quit · F1 help`; after X3 (L-C) the one of
   `backlog__detail_reqs_cite_picker`. Nothing else may be pending after a merge (the lanes'
   sets are disjoint; L-B's `replay__runs_step_selected` is the one app-level file, PA-6).
2. **Merged-tree D12 tests** (top level of `tests/backlog_keys.rs`, after the three modules):
   - `the_runs_pane_status_line_and_box_list_every_layer`: demo, Runs pane, a step selected:
     status line is today's browse line; `?` box has, in order, `Runs:` (from `J next step`
     through `= fit the run`), `Pane:` (`PgDn page down · PgUp page up · l/]/Right next sub-tab ·
     h/[/Left previous sub-tab`), `Backlog:` (`Enter replay step · m open graph · f filter · F
     clear filter · N new item · Q queue / dequeue · P pause / resume queue`), `Common: e edit`,
     `List:` (… `Enter fold`), `Global:`, and `?/F1 closes this box`.
   - `a_rebound_runs_key_works_with_the_tab_converted`: L-B's and L-C's rebinding tests re-run
     here unchanged (the lanes' chords were chosen to pass the unconverted tab; they must still
     work through the converted one) — no new code, only the merged run.
   - `the_replay_miss_names_a_rebound_step_key` re-runs (L-A's).
   - `a_capturing_pane_drives_the_status_line_and_box`: the Runs reject note shows `Ctrl+c quit
     · F1 help` and its `?` box is the one `Global:` line with `F1 closes this box`; the Reqs
     picker's box has `Reqs: J next row · K previous row · Enter pick`, `Common: Esc back`,
     `List: j/Down down · k/Up up` (the lanes could only check that `F1` opens the box).
3. **Q1 (a), PA-5: the list hint row.**
   - `backlog/list.rs`: `render(frame, area, view, hint: Option<Line<'static>>, theme)`; with
     `Some` and an inner height ≥ 2 the inner area splits `[rows, 1]`, the rows' `window` uses
     the shorter height, and the hint is drawn clipped (`cells::clip`) in `theme.dim`. Doc: "the
     Backlog's own keys (MOD-67 M5, Q1 a)".
   - `backlog/mod.rs`: `const LIST_HINT: HintSpec = &[Pair(ListDown, ListUp, "move"),
     Pair(PanePrevSubtab, PaneNextSubtab, "sub-tab"), One(BacklogOpenFilter, "filter"),
     One(BacklogNewItem, "new"), One(Edit, "edit")];` rendered through `self.detail.key_stack()`
     and passed **only while nothing captures** (`form`, `item_form`, `detail.captures_input()`
     all clear): a hint for keys a field is typing would lie. Default text: `j/k move · h/l
     sub-tab · f filter · N new · e edit` (50 cells in the list pane's 53).
   - Tests first: in-file `the_list_hint_fits_the_list_pane_at_100_columns` (width ≤ 53),
     `the_list_hint_is_hidden_while_a_form_or_pane_captures`, a rebinding check
     (`[backlog]\nfilter = "/"\n` → `/ filter`).
   - The one full re-baseline (§7.2 table), reviewed line by line: only the list pane's last inner
     row may change (and the list window, where the rows reach it).
4. **Dead code** (`relations usages` confirms no production caller): `Scroll::on_key` and its
   in-file tests (`backlog/detail/mod.rs`); `views.rs` `PENDING` deleted, so
   `every_view_stack_is_named_by_a_view` covers all 20 M5 stacks; `item_form::{ctrl_s, ctrl_e}`
   and `filter::CHORD` are L-A's to delete (all callers are L-A's), T-close only confirms.
5. **README** (D10; plan T-close):
   - "Changing keys", line 236: "A table named after a Settings section, a Skills view,
     `[requirements]`, a Backlog sub-tab (`[backlog.runs]`, `[backlog.docs]`, …), `[chat]` or a
     pop-up changes a shared action there alone"; add: "`[backlog]` holds the Backlog's own keys
     (filter, new item, queue, …); `[backlog.runs] filter = ["/"]` moves the filter key in the
     Runs sub-tab alone."
   - Line 244: "A global key must not be a key that Settings, Skills, Requirements, the Backlog,
     Chat or a pop-up already uses: `[global] quit = ["x"]` is refused, because `x` cancels in
     **Settings › Agents** and rejects a step in the Backlog's **Runs**; so is `[global] quit =
     ["f"]`, because `f` opens the Backlog filter."
   - Delete the paragraph "Backlog and Chat do not read this file yet. …" (lines 251-252).
   - Add after the `Tab` sentence: "In the Backlog's item form and the Notes and Docs editors,
     `[form] save` and `[form] external_editor` replace `Ctrl+s` and `Ctrl+e`."
   - Backlog table (checked against the defaults): add rows `f` / `F` (filter the list / clear the
     filter), `N` / `e` (new item / edit the item), `m` (open the Graph sub-tab), `Q` / `P` (queue
     or dequeue the item / pause or resume this box's queue), `?` (every key of the pane you are
     in); the `J`/`K` row reads "Scroll the detail pane (Body, Notes, Prompt; in Runs, Graph, Docs
     and Reqs they move the pane's cursor)". New short paragraphs: **Graph** (`J`/`K`, `PgUp`/
     `PgDown` move, `+`/`-` depth, `Enter` re-roots), **Docs** (`J`/`K`, `a` new document, `v` new
     version, `Ctrl+s` saves), **Notes** (`a` add a note, `Ctrl+s` saves). Mention the list hint
     row and the Runs hint row in one sentence. Runs and Reqs tables: unchanged (checked).
   - Chat table: unchanged (checked: `i`/`Enter`, `a`, `1`…`9`, `Esc` `Esc`, `t`, `j`/`k`,
     `g`/`G`).
6. **HANDOFF**: M5 phase note (stacks, PA-1-PA-6, the user-visible list of §1), carries to M6:
   key prose (D11's list plus `Ctrl+S compares again`, `HINT_NO_CONFLICT`, `Esc again ends this
   chat`, the item-form module docs), `Keymap`/`Binding`/`KeyScope` and the legacy branches of
   `App::on_key` and `render_help` (now unreachable for every tab), `Ctx::new(&Keymap)`, the
   uniform hint row, the every-stack reachability test.
7. Plan review gate; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`;
   Postgres suites once on the merged tree (`*_pg.rs`, `runs_pg.rs`, `item_writes_pg.rs`, sandbox
   DB 5439); the workspace gate (`cargo test --workspace`, memory: "zeta", other crates); remove
   the four worktrees, then the lane branches.

T-close commit groups: X1-X4 the merges; X5 `test(mod-67): M5 merged-tree key tests`; X6
`feat(mod-67): the Backlog list hint row (Q1 a)` (with the re-baseline); X7 `refactor(mod-67):
drop Scroll::on_key; every M5 stack named`; X8 `docs(mod-67): README keys, M5`; X9
`docs(mod-67): close-out M5`.

### 7.2 Snapshots that move in X6 (list hint row)

- `backlog__` (28): every browse frame — `list_{folded,grouped,last_row}`, `filter_no_match`,
  `filtered_list`, `detail_{body,graph,prompt,reqs,reqs_reconfirmed,notes,documents,runs}`,
  `empty_{body,graph,prompt,reqs,notes,documents,runs}`, `graph_{depth_1,depth_3,
  outside_workspace,re_rooted}`, `runs_{flow_fanout,flow_tool_chips,follow_up_queued,
  pending_permission}`.
- Not moving (a mode captures, so no list hint): `filter_form`, `item_form_{edit,new}`,
  `item_divergence`, `document_form`, `note_compose`, `runs_{reject_note,flow_reject_note,
  follow_up_input,closeout_typed,closeout_warn,artifact}`, `detail_reqs_cite_picker` (13).
- App-level (16): `concepts_search__{decisions_project,empty,error,hits,searching}`,
  `shell__{after_switch,migration_prompt,offline_label,switcher_empty,switcher_open}`,
  `waiting__{graphics_empty,platform_mixed}`, `prompt_preview__{preview_ana_2,preview_feat_1}`,
  `integration__demo_shell`, `replay__runs_step_selected`.
- `chat__*`: none.

---

## 8. Per-lane summary (details in the lane files)

| Lane | Source | Stacks it returns | Snapshots it re-baselines (rows that may move) | Rebinding test (chord the unconverted tab passes) |
|---|---|---|---|---|
| L-A | `backlog/{mod,filter,item_form,divergence}.rs`, `detail/{compose,notes,documents}.rs`, `app/mod.rs` (Backlog rows + their doc items) | `BACKLOG_FILTER`, `ITEM_FORM`, `ITEM_DIVERGENCE`, `BACKLOG_NOTES`, `NOTES_COMPOSE`, `BACKLOG_DOCS`, `DOCS_COMPOSE`; `BacklogTab::key_stack` | 6 of 15: `filter_form`, `item_form_{edit,new}`, `item_divergence`, `document_form`, `note_compose` (hint row, status row); the other 9 byte-identical | `[backlog.docs] new_version = "V"` (own pane; L-A converts the tab) |
| L-B | `detail/runs.rs` | `RUNS_BROWSE`, `RUNS_CONFIRM`, `RUNS_COUNTING`, `RUNS_ARTIFACT`, `CAPTURE` | on its branch 7 of 12: browse hint row (6: `detail_runs`, `empty_runs`, `runs_flow_{fanout,tool_chips}`, `runs_follow_up_queued`, `runs_pending_permission`), artifact hint (`runs_artifact`); plus `replay__runs_step_selected` (PA-6). The 6 capturing status rows move at the T-close merge (X2) | `[backlog.runs] next_step = "n"` |
| L-C | `detail/{body,graph,prompt,requirements}.rs` | `BACKLOG_BODY` (default), `BACKLOG_GRAPH`, `BACKLOG_PROMPT`, `BACKLOG_REQS`, `REQS_PICK`, `REQS_KIND`, `REQS_CONFIRM` | on its branch **none** (every hint renders byte-identical); `detail_reqs_cite_picker`'s status row moves at the T-close merge (X3) | `[backlog.reqs] cite = "C"` |
| L-D | `chat/{mod,transcript,composer}.rs` | `CHAT_LIVE`, `CHAT_REPLAY`, `CAPTURE` | 0 of 15: every `chat__*` byte-identical | `[chat] thoughts = "T"` |

Lane gate (each, in its worktree):
```bash
rustfmt --edition 2024 --check <own .rs paths>
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --lib <ui::tabs::backlog | ui::tabs::backlog::detail::runs | ui::tabs::backlog::detail | ui::tabs::chat>
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features \
  --test backlog --test backlog_keys --test replay --test chat --test reveal -- --test-threads=1
cargo insta test -p htui --all-features --check     # after accepting the lane's own snapshots
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
```

---

## 9. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `runs.rs` (6.3k lines, browse + six modal modes, flow, digits) dominates L-B | High | L-B is serial by mode (its §8): modal modes first (small, self-contained), browse last; the action body keeps its shape, only `char` → `Act` |
| Pane-first changes what a pane sees (every unconverted pane now receives the tab's keys first) | Medium | §6.1 equivalence argument; every pane's `_` arm already returned `Pass` for the tab's keys (checked per pane: Runs `scroll.on_key`, Body/Prompt `Scroll::on_key`, Docs/Reqs after their letters, Graph `_ => Pass`, Notes after `a`); user-visible item 1 is the one difference; `a_capturing_sub_tab_gets_the_navigation_keys` (`backlog/mod.rs:1507`) is re-asserted by L-A |
| `Enter` routing (fold → pane → miss) | Medium | PA-2 allow-lists; T-V1/T-V3 pins; `tests/replay.rs` (Runs replays, header folds, Body misses), L-C's Graph re-root test, L-A's rebound-miss test |
| A rebinding test written against the unconverted tab breaks on the merged tree | Medium | Chords chosen to pass both (lane files); T-close re-runs them |
| Status-row drift of L-B/L-C capturing snapshots first shows at the merge, where it could hide a real change | Medium | §7 step 1 names the exact seven files and the one row each; X2/X3 accept nothing else |
| Snapshot churn hides a behaviour change | Medium | §1's list and each lane file's per-snapshot expected drift are the only accepted rows; T-close's list-row diff is one row per frame |
| A user file that loads today is refused (D10) | Low | Only global keys on a Backlog/Chat default (§4.1); the error names the line and the stack; README; `--default-keys` |
| The flow canvas one row shorter re-lays out a flow snapshot | Low | L-B reviews `runs_flow_*` cell by cell; the layout is rataflow's, the row count is the only input that moved |
| Lane disk/CPU (four worktrees) | Low | ~10 GB `target` each; 2.9 TB free |
