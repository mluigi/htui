# Blueprint: MOD-67 milestone 6, "Close-out: legacy keymap gone, key prose generated, the last views converted"

**Status**: draft for the maintainer (2026-10-13). PA-1 to PA-13 (§1) are this blueprint's
amendments; the ones marked **decision** need an answer before T1 starts, the rest are the
architect's calls the plan delegated and are listed so they are visible. D1-D11 stand except where
§1 amends them.

**Plan**: `.claude/plans/mod-67-m6-close-out.plan.md` (confirmed 2026-10-13, Q1 = (a), Q2 = (a)).
**Lane files**: `.claude/plans/mod-67-m6-blueprint/L-{A,B,C}.md` (every arm, sentence, test and
snapshot of each lane).
**Templates**: M5 blueprint `.claude/plans/mod-67-m5-backlog-chat.blueprint.md` (shape, §6
skeletons, §7 T-close), M3 blueprint §3.9-§3.10 (print and validator), M4 blueprint §6.
**Verified at**: HEAD `59ff7d02`, branch `hr/MOD-67`. Read at that HEAD, in full where designed
against: `keys/{catalogue,views,stack,validate,print,mod,load,hint,chord}.rs`, `keymap.rs`,
`app/state.rs` (`Ctx`, `App::new`, `on_key`, `render`, `render_help`), `app/pane.rs`
(`editor_status`, `editor_visible`, the help-box test), `testkit.rs` (`SectionBench`),
`ui/layout.rs`, `ui/tabs/registry.rs`, `ui/overlay/{registry,queue,waiting_list,
workspace_switcher,concepts_search}.rs`, `ui/path_picker.rs`, `settings/{mod,hierarchy,agents,
boxes,connection,personas,prompt,qdrant,queue,secrets,kinds}.rs` (every key-naming literal and
every hint render), `requirements/{mod,forms}.rs`, `skills/{mod,library,templates,attach,
agent_help}.rs`, `backlog/{mod,list,filter,item_form,divergence}.rs`, `backlog/detail/{compose,
documents,notes,graph,runs,requirements}.rs`, `chat/{mod,transcript,composer}.rs`, `editor.rs`,
`agent_settings.rs`, `README.md` "Changing keys" and the per-tab key tables, ANA-26 §6-§8, the M3,
M4 and M5 plans' review gates. Line numbers are pre-edit (HEAD `59ff7d02`).

**Reproductions** (built `target/debug/htui` at HEAD, scratch files under `/tmp/m6arch/`):
D6 and both D9 lows reproduce exactly; the commands and output are in §4.6 and §4.9.

**Order**: T0 (serial, primary tree) → T1 (serial, primary tree) → L-A ∥ L-B ∥ L-C (worktrees
`target/wt/<lane>`, branches `mod-67-m6-<lane>`) → T-close part 1 (merges, X-S, PENDING emptied)
→ T2 (serial, primary tree) → T-close part 2 (docs, close-out). Lanes add no context, act, stack,
`FIXED` or `VIEW_DEFAULTS` entry (§4 is complete for all three); a missing one goes back to the
main thread as a T1 fix commit first (plan lane rule).

**House style (M2-M5, unchanged)**: `#![warn(missing_docs)]`, inline format args, rustfmt edition
2024 width 100, clippy `-D warnings` both `--all-targets --all-features` and featureless.
Implementers commit incrementally and stage their own paths only; Gortex `read` for source, and
where the hook blocks `Read`/`Edit`, an anchored scripted replace (each anchor asserted to match
once).

**The one-paragraph design.** T0 deletes `keymap.rs` mechanically: `Ctx::new` takes `&Keys` where
it took `&Keymap`, `App::new` loses its second argument, and the legacy tab step and help branch go
(no snapshot moves: no legacy row has existed since M5). T1 adds two view contexts, **`queue`**
(the queue overlay's five verbs) and **`picker`** (the directory picker's six), their stacks
`QUEUE_OVERLAY` and `HIERARCHY_PICKER` appended to `DECLARED` (so every existing first-reported
phrase is unchanged), two `FIXED` rows (the Attachments form's activation keys, the Requirements
form's priority keys; the sweep found no other M3/M4 key matched ahead of the resolver), the
validator's fixed pass deduped against earlier passes (D6), a reachability test over every
catalogue act (D7, its two lane-owned stacks and eleven acts on a `PENDING` list T-close empties),
the generated key page and its pin (D8), and the one-line `Keys::derive` fix behind both D9 lows
(PA-2). The lanes then convert the queue overlay (L-A) and the picker with hierarchy's prose (L-B),
and rewrite every other key-naming sentence through `Keys::label` on the stack the named key acts
in (L-C, 53 sentences, PA-1). T2 moves every tab's generated hint into one row the shell draws above
the status line (`Tab::key_hint`), overlays untouched.

---

## 1. Plan amendments

| # | Amends | What the plan says | Why it cannot stand as written | What this blueprint builds | Decision? |
|---|---|---|---|---|---|
| **PA-1** | D4 inventory; plan claim 16; lane table (L-C); Complexity line | ~25 key-naming sentences in ~16 files | A full literal scan (multi-line literals, lone letters, `y / n` tails, lower-case `ctrl-s`) finds **61** key-naming sentences in **26** production files beside the picker's and queue's hints (L-C §1 and L-B §3 list every one with its file:line, stack and act); three of the plan's own sites (`Esc again discards` ×3) are a `TextArea`'s `Esc` and stay literal (L-C §3). Missed families: the Settings notes and questions (`agents.rs:150` `r cannot refresh it`, `:203` `r probes now`, `:1133` `p pastes`, `:1919` `r probes`; `connection.rs:131` `e replaces it`, `:144`/`:154` `y / n`; `qdrant.rs:28`; `secrets.rs:94`, `:106`, `:111`, `:113`, `:115`, `:166`; `boxes.rs:151` `ctrl-s retries`, `:156` `y retries`), the Skills/Requirements `Ctrl+S`/`r reloads` hedges (`library.rs:89`, `:93`, `:171`, `:185`, `:1423`, `:1508`; `templates.rs:127`, `:136`; `requirements/mod.rs:137`, `:146`, `:156`; M4's review gate carried "Requirements messages", the plan dropped them), the Backlog hedges (`item_form.rs:1144`, `compose.rs:551`, `documents.rs:403`), `agent_help.rs:66`, `agent_settings.rs:558` (shared with the worker), `workspace_switcher.rs:123`, hierarchy's `:48`, `:769`, `:834`, `:1686`, the picker's `path_picker.rs:338`, and `backlog/mod.rs:1094`/`:1097` (`P`, shared with the queue overlay). | All are in scope (D4's rule covers them). L-C takes 53, L-B 6 (plus the picker's hints), L-A the queue hint, T-close X-S the two shared `P` sentences. L-C stays one lane, committed in four families (L-C §6); each site is a 3-15 line change. | **decision**: confirm the scope growth (recommended), or cap D4 at the plan's list and carry the rest to MOD-82 |
| **PA-2** | D9 ("fixed in T1 if the fix stays inside `keys/print.rs`/`validate.rs`") | — | Both lows have one cause outside those files: `Keys::set` (`keys/mod.rs:227-254`) drops `line` when the entry equals the compiled row, and `Keys::derive` (`keys/mod.rs:152-181`) never restores it when the shared row moved. So a view row pinned to its compiled derivation reads as derived: `--print-keys` leaves it unmarked and the validator places its collisions on the shared row's line (§4.9). Patching print.rs and validate.rs alone means re-deriving the row's current derivation in both (two copies of `derive`'s rule). | One line in `Keys::derive`: a row the file set keeps `line = entry` when its chords differ from the derivation in force (`None` when equal, as today). `--print-keys` output changes only for a file that pins a view row whose shared row it also rebinds (D9's own threshold: no maintainer call needed). | **decision** (the file D9 named): fix in `derive` (recommended), or record both lows as accepted |
| **PA-3** | Q1 (a) "`Tab` gains a defaulted `hint(&self, ctx) -> Option<String>`" | `Option<String>` named `hint` | (a) Two tabs already have an inherent `fn hint` with another signature (`ChatTab::hint(&self, keys)` `chat/mod.rs:483`, `RequirementsTab::hint(&self, keys, theme)` `requirements/mod.rs:1027`), and inherent methods win method-call resolution: two meanings of `self.hint` in one file. (b) Hint rows carry styles a `String` loses: the Requirements writes and the Reqs pane writes are greyed when not writable (README "the write keys are greyed out", `requirements/mod.rs:1030-1034`, `detail/requirements.rs:453-457`). (c) The Backlog's "list keys after when they fit" needs the row's width. | `fn key_hint(&self, width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>>` on `Tab`, defaulted `None`; `SettingsSection::key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>>` and `DetailTab::key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>>`, both defaulted `None` (§6). | no (plan T2: "the architect's blueprint fixes the trait method") |
| **PA-4** | T1 ("its `queue.rs`/`path_picker.rs` half lands red-gated: `#[ignore]` with reason until L-A/L-B merge") | `#[ignore]` | An ignored test checks nothing for every other act during the lanes, and M5 already had the better mechanism: `views.rs` `PENDING` (a skip-list T-close empties). `every_view_stack_is_named_by_a_view` also fails for the two new stacks until the lanes name them. | `PENDING: &[&str] = &["QUEUE_OVERLAY", "HIERARCHY_PICKER"]` (stack names) and `PENDING_ACTS: &[Act]` (the 11 new acts) in `views.rs` tests, each doc'd "MOD-67 M6: the lanes name these; T-close empties the list"; both tests otherwise live. | no |
| **PA-5** | Plan "Files to Change" lanes, claim 21 ("lane source sets pairwise disjoint") | L-A `queue.rs`; L-C the D4 list | Shared prose functions cross the plan's lane lines: `item_form::mint_may_have_landed` is called by `backlog/mod.rs:663`; `compose::{not_written, could_not_check, may_have_landed}` by `notes.rs` and `documents.rs`; `editor::EDITED` is compared in `backlog/mod.rs` and `notes.rs` unit tests; `backlog::queue_sentence` (the `P` sentences) is called by `backlog/mod.rs:526` and `overlay/queue.rs:506`. | L-C owns `backlog/mod.rs`, `notes.rs`, `documents.rs`, `requirements/mod.rs`, `settings/{mod,agents,boxes,qdrant,secrets}.rs`, `agent_settings.rs`, `agent_worker.rs` (one test line), `workspace_switcher.rs` beside the plan's list. `queue_sentence`'s pause label is T-close step **X-S** on the merged tree (its two callers sit in L-A's and L-C's files). File-set check: §5.2. | no |
| **PA-6** | D4 ("Constants become functions of the label") for the `y / n` question tails (`connection.rs:144`, `:154`; `qdrant.rs:28`; `secrets.rs:113`, `:115`, `:166`) | — | M3 R1 L2 removed the key from hierarchy's delete warning instead (`hierarchy.rs:117-121` `NOT_UNDONE`: "the hint row under it does"), while Kinds' delete question appends the rendered answers (`kinds.rs:1955-1960`). Either is consistent with D4. | Label-built, byte-identical with the defaults: the question constants lose their ` y / n` tail and the render appends `settings::answers(keys, stack)` (`y / n` from `confirm.yes`/`confirm.no`'s first chords; an unbound side dropped; both unbound, nothing). No snapshot moves. | **decision**: label-built (recommended), or drop the tails as M3 R1 L2 did (5 snapshots move) |
| **PA-7** | D4 (byte-identical with the defaults, implied by "a rebound key shows up in the message too") | — | `KeyChord::label` spells `ctrl-s` as `Ctrl+s` (`chord.rs:101-134`): every `Ctrl+S` sentence becomes `Ctrl+s` (and `Ctrl+R` → `Ctrl+r`, `ctrl-s retries` → `Ctrl+s retries`). ANA-26 §6 item 5: "One spelling (`KeyChord::label`) ... throughout"; the hint rows already say `Ctrl+s save`. | Accepted as the one spelling. Drift: 3 snapshots (`skills__changed_elsewhere`, `templates__changed_elsewhere`, `templates__missing_item_confirm`), ~40 assertions in L-C's tests (L-C §4). | **decision**: confirm the spelling change (recommended; the alternative is a second label spelling for prose) |
| **PA-8** | D4, `agent_settings.rs:558` `LITERAL_LAUNCH` | — | One constant is both the Agents section's refusal (`agents.rs:2269`) and the worker's (`agent_worker.rs:1639`, pinned at `:16057`); the worker has no keys. | The constant names no key (`"this row's launch is literal; edit its command"`, the drop-the-clause rule); the Agents view says `agent_settings::literal_launch(edit: Option<&str>)` (`…; e edits its command` with the defaults). | no |
| **PA-9** | The brief's "lane source sets disjoint from T0/T1/T2's" | — | Not attainable as specified: T0 must rewrite the `Ctx::new` test benches inside L-A's and L-C's files (`queue.rs:612`, `concepts_search.rs:570`, `library.rs`, ...), and T2 rewrites every tab view's render (L-B's `hierarchy.rs`, most of L-C's files). | Lanes are pairwise disjoint and disjoint from T1; their overlaps with T0 and T2 are serial (T0 is committed before the lanes branch, T2 starts after they merge). §5.2 records each intersection. | no (report) |
| **PA-10** | D3 ("cancel (`Esc`)" as a picker verb; act names) | a `picker.cancel` act | Every M4/M5 picker leaves on `common.back` (`ATTACH_PICKER` `views.rs:500-506`, `REQS_PICK` `:743-751`), and `no_view_name_hides_a_shared_name_it_inherits` forbids a picker act named `up` (the view layer inherits `list.up`). | `Esc` is `common.back` through a `BACK` layer; the six own acts are `open`, `parent`, `choose`, `choose_dir`, `go_to`, `hidden` (§4.2). | no |
| **PA-11** | D5 (FIXED for the M3/M4 forms) | — | `FIXED` is iterated in `DECLARED` order and a row/chord is reported once "naming the first stack" (`validate.rs:233-286`). The two new rows sit at `DECLARED[43]` and `[53]`, before every M5 stack, so three existing messages name a new first stack. | `FIXED` keeps `DECLARED` order (the doc's rule). Changed expectations, all in `validate.rs` tests: `[form] save = "esc"` → "is fixed in the Attachments form" (was "in the item form"); `[form] next_field = "right"` → "is fixed in a Requirements form" (was "in the Backlog filter form"); `[form] save = "shift-left"` → "is fixed in a Requirements form" (was "in the item form"). `tests/keys_file.rs` pins only `[backlog.filter_form]` lines: unchanged. | no |
| **PA-12** | D4, `agent_help.rs:45` `HELP_OPEN` ("Esc leaves it first") | one label | The key that leaves the help depends on its state: asking, the request field's own `Esc` (widget, fixed); waiting, `skills.help.cancel` (`esc`); a proposal or answer, `confirm.no` (`["n", "esc"]`, first label `n`). | `AgentHelp::help_open(&self, keys) -> String`: asking `Esc` (literal, the field's); waiting `label(HELP_WAITING, SkillsHelpCancel)`; proposal/answered `keys.hint(HELP_PROPOSAL, &[Hint::All(Act::ConfirmNo, "")])` (`n/Esc`, as the proposal's hint row spells it). Proposal-state text changes `Esc` → `n/Esc` (one unit test, no snapshot). | **decision** (small): this (recommended), or `Esc` literal in every state (wrong when `confirm.no`/`cancel` is rebound) |
| **PA-13** | D6 ("the validator's passes dedupe by (file line, chord), first pass wins") | every pass | Applied to every pass it also merges a collision with the capture error on one chord, which `validate.rs` `a_printable_shared_chord_is_reported_once_on_the_shared_line` pins as two errors (they say different things: a rebinding of the other act fixes the first, not the second). D6's stated case is "a collision and on a fixed key". | The fixed pass (the last) skips a (line, chord) an earlier pass reported. The collision/capture pair stays two errors. | no (interpretation; say if the wider reading is wanted) |

**User-visible beyond the plan's list** (not amendments; reviewed per task):
1. The queue overlay is no longer modifier-blind: `ctrl-j`, `alt-J`, `ctrl-P`, ... do nothing (they
   moved, reordered or paused before; `queue.rs:437-478` matched the code alone).
2. Its `?` box lists `Queue:`, `List:`, `Overlay:` and `Global:` lines (it showed the legacy
   overlay box). The picker's lists `Picker:`, `Common: Esc back`, `List:`, `Global:`.
3. The section hint under the picker drops `Esc cancel` while the go-to field is open (that `Esc`
   closes only the field, `path_picker.rs:123-138`); it stays in the popup's own hint.
4. `Ctrl+S` → `Ctrl+s` in prose (PA-7); agent help's proposal-state note names `n/Esc` (PA-12).
5. A file that rebinds onto the Requirements priority keys or the Attachments activation keys, or
   puts a shared/global chord on a new `queue`/`picker` default, is refused (§4.5, §4.4).
6. T2: every tab's hint moves into one row above the status line; the chat composer's own hint
   (`Enter send · Esc leave`) becomes visible while composing (`Composer::render` drew the input
   instead, `composer.rs:113-124`, so `COMPOSE_HINT` was never on screen).

---

## 2. Plan claims found false or incomplete (for the record)

| Claim | Finding |
|---|---|
| 16 (~25 key-naming sentences remain) | **False**: 61 sentences plus the queue's and the picker's hints (PA-1); the scan missed multi-line literals, lone-letter keys and `y / n` tails, and counted three widget-owned `Esc again discards` that stay literal. |
| 21 (lane source sets pairwise disjoint) | **False as drafted**: four shared prose functions cross lane lines (PA-5). |
| Scope drift / claim 8 ("Both are modifier-blind (`ctrl-j` moves)") | **Half true**: `PathPicker::on_key` matches the code alone, but its only caller passes every chord `CAPTURE` admits first (`hierarchy.rs:1035`, CONTROL/ALT/function keys), so `ctrl-j` does not move the picker; a SUPER/META/HYPER chord would. The queue overlay is modifier-blind for real (no guard). |
| D9's premise (M3 review gate: "the next `--print-keys` marks it `(changed)`") | **False**: reproduced at HEAD, the pinned row prints unmarked (§4.9). README's advice ("delete it to let it follow again") is right. |
| Files table: T0 "`app/{state,update,pane,mod}.rs`", "`keys/{chord,catalogue,hint}.rs` doc links" | `app/mod.rs` has no keymap reference (nothing to change); `keys/hint.rs` has two legacy-row doc mentions (`:49`, `:56`), not a `KeyScope` link. `ui/overlay/registry.rs`'s `OverlayId::ANY` has no user once `keymap.rs` goes (§3). |
| Claim 5 ("`App::new` has 5 callers") | 7 call sites in 5 files (`lib.rs:232`, `testkit.rs:129`, `app/pane.rs:957`, `app/state.rs:1091`, `app/update.rs:651`, `:1586`, `:1614`). |
| Claim 20 / Q1 cost (~165 TUI snapshots) | 113 full-frame snapshots (107 in `tests/snapshots`, 6 in `src`) move in T2, plus the section-level bench snapshots that draw a hint row (§6.6); 17 are CLI (`state_cli_pg__`, `htui__state_cmd__`) and 11 overlay crops, none of which move. |
| Plan D2 "`j`/`k` are the shared `list.down`/`list.up`" | True, and `Down`/`Up` too (`list.*` defaults are `["j", "down"]`/`["k", "up"]`): no `VIEW_DEFAULTS` row is needed for either new context. |

---

## 3. T0: the legacy keymap goes (exact edits, mechanical)

No behaviour change, no snapshot change; `cargo build` drives the list, this section is its
expected shape.

| File | Edit |
|---|---|
| `src/keymap.rs` | **Delete.** |
| `src/lib.rs` | `:27` `pub mod keymap;` deleted; `:72` `use crate::keymap::Keymap;` deleted; `:232` `App::new(request_tx, Keymap::default_global())` → `App::new(request_tx)`. |
| `src/app/state.rs` | `:18` import → `use crate::keys::{Act, CTRL_C, Context, HelpLine, KeyChord, Keys, Stack};` stays, the `keymap` line goes. `Ctx` (`:76-98`): field `keymap` deleted; field doc of `keys` → "The keys in force (MOD-67 D9): `App`'s own at the shell's sites, a bench's in tests." `Ctx::new` (`:104-125`): parameter `keymap: &'a Keymap` becomes **`keys: &'a Keys`** in the same position (`scope, projects, top_bar, keys, theme, origin, emit`), `keys` stored directly. `Ctx::with_keys` (`:126-131`) deleted. `App.keymap` (`:190-192`) deleted. `App::new(requests)` (`:289`), `keymap` field init gone. Every destructuring `let Self { …, keymap, keys, … }` (`:550-575`, `:618-641`, `:655-677`, `:753-776`, `:821-844`, `:875-898`) drops `keymap` and calls `Ctx::new(scope, projects, top_bar, keys, theme, origin…, emit)` with no `.with_keys`. `on_key`: `:906-910` (the `self.keymap.resolve(&KeyScope::Tab(id), …)` step) deleted; doc `:785-791` loses "its legacy rows". `ctx()` (`:925-936`) → `Ctx::new(&self.scope, &self.projects, &self.top_bar, &self.keys, &self.theme, origin, &self.emit)`. `render_help` (`:999-1052`): the `None` arm keeps the overlay line and the global line, the `legacy` block (`:1042-1050`) goes; doc `:1001-1003` → "Otherwise: the overlay context (if one is up), then the global context, then the closing line." Docs at `:597` and `:677` ("no keymap") → "no key table". Test module `:1082`, `:1091` → `App::new(tx)`. |
| `src/app/update.rs` | Five `Ctx::new` sites (`:170-189`, `:207-226`, `:317-336`, `:366-383`, `:396-414`): `keymap` out of the destructuring, `keys` into `Ctx::new`, no `.with_keys`. Tests: `:575` import deleted; `:651`, `:1586`, `:1614` → `App::new(tx)`. Comments `:38`, `:1261` ("or a keymap binding") → "the shell itself (`Origin::App`)". |
| `src/app/pane.rs` | Test module: `:682` import → `use crate::keys::KeyChord;` (only if still used; `Binding`, `KeyScope`, `Keymap` go); `:957` → `App::new(tx)`; `the_help_box_lists_the_keys_that_act_here`: the `app.keymap.bind(…)` block (`:1725-1730`) and its assertion `!frame.contains("asker legacy")` (`:1743`) deleted, doc `:1717` "(no legacy rows: the asking tab refuses them)" → "(none of the asking tab's own: it refuses them)". Snapshot `the_help_box_on_the_asking_tab` unchanged (the row never drew). |
| `src/testkit.rs` | `:27` → `use crate::keys::KeyChord;` (plus the existing `keys` imports); `:129` → `App::new(request_tx)`; `SectionBench`: field `keymap` (`:667-668`) and its init (`:693`) deleted; `ctx()` (`:710-721`) → `Ctx::new(&self.scope, &self.projects, &self.top_bar, &self.keys, &self.theme, Origin::Tab(SettingsTab::ID), &self.emit)`. `SectionBench::with_keys` and `Harness::with_keys` (test helpers) stay. |
| 14 view files' test benches: `ui/overlay/{concepts_search,queue,quit_confirm,waiting_list}.rs`, `ui/tabs/backlog/mod.rs`, `backlog/detail/{compose,graph,requirements,runs}.rs`, `chat/mod.rs`, `requirements/mod.rs`, `skills/{agent_help,library,templates}.rs` | In each `#[cfg(test)]` module: `use crate::keymap::Keymap;` deleted; a bench field `keymap: Keymap` becomes `keys: Keys` (init `Keys::compiled().clone()`), or a local `Keymap::new()`/`Keymap::default_global()` tuple slot becomes `Keys::compiled()` (borrowed, `&'static`); `Ctx::new(…, &self.keymap, …)` → `Ctx::new(…, &self.keys, …)`. The two benches that took optional keys (`quit_confirm.rs:175-191`, `agent_help.rs:844-862`) keep their `with_keys` helper and pass `self.keys.as_ref().unwrap_or(Keys::compiled())` into `Ctx::new` instead of `ctx.with_keys(keys)`. 42 `Ctx::new` sites in 17 files in total (plan claim 1). |
| Doc links | `ui/tabs/registry.rs:22` "also the key of its [`KeyScope`]…" → "Stable identity of a tab, used for reply addressing and the reveal targets."; `:56`, `:111` "no keymap" → "no key table". `ui/overlay/registry.rs:15` same rewrite for overlays; **`OverlayId::ANY` (`:19-25`) deleted** (its only users were `keymap.rs`; `pub const`, so no compiler help: checked by text search). `settings/agents.rs:121-123` "a Settings section has no [`KeyScope`] of its own" → "a Settings section has no tab-level key rows of its own". `keys/chord.rs:3-4` drops "`crate::keymap::KeyChord` … re-exported" (now: "`KeyChord` moved here from the deleted `keymap.rs` in MOD-67 M1"). `keys/catalogue.rs:4` "today's `Keymap::default_global` bindings" → "the M1 global bindings"; `:740`, `:823` "`keymap.rs`'s" → "the pre-MOD-67 keymap's". `keys/hint.rs:49` → "`Global`, `Overlay`, or a context's heading"; `:56` → "A line from its parts." `Tab::key_stack`'s doc (`registry.rs:76-78`) → "This tab's key stack in its current mode (MOD-67 D4). Every production tab returns one; the default `None` is the test doubles', which the shell treats as `Stack::BASE`." |

Not touched: `tests/*.rs` comments that mention the legacy keymap (5, prose, plan claim 6).

**T0 gate**: `cargo build -p htui --all-targets --all-features`; `cargo clippy --workspace
--all-targets --all-features -- -D warnings`; `cargo clippy --workspace -- -D warnings`;
`cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features
--check` (zero pending). Text search afterwards: `keymap::`, `KeyScope`, `Keymap`, `with_keys(`
on `Ctx`, `OverlayId::ANY` → no production hit. One commit: `refactor(mod-67): M6 T0 delete the
legacy keymap; Ctx::new takes &Keys`.

---

## 4. T1: the keys foundation (exact interfaces)

### 4.1 `keys/catalogue.rs`: two contexts (`Context::ALL` 40 → 42)

Appended after `Chat`, in this order (M3-M5 appended per milestone; appending keeps every
`context_position` index, the `--print-keys` head and every table-list prefix):

| Variant | `table()` | `heading()` | `is_view` | `is_shared` | Own rows |
|---|---|---|---|---|---|
| `Queue` | `queue` | `Queue` (the overlay's title, `queue.rs:424-426`) | yes | no | 5 |
| `Picker` | `picker` | `Picker` | yes | no | 6 |

`use Context::{…}` gains `Picker, Queue`. `ALL`, `table`, `heading`, `is_view` and the test-only
`context_position` (`Queue => 40`, `Picker => 41`) grow in lockstep. Variant docs: "`[queue]`: the
queue overlay (MOD-67 M6 D2)." / "`[picker]`: the directory picker a Settings section opens (MOD-67
M6 D3; today Hierarchy's)." `Context` doc and the module doc gain an M6 paragraph: "MOD-67 M6
appends `queue` (the queue overlay) and `picker` (the directory picker), the last two views that
matched raw keys."

### 4.2 `keys/catalogue.rs`: eleven acts (CATALOGUE 159 → 170)

All `row` (none `in_capture`: the go-to field is `CAPTURE`, no picker act is offered beside it).
Defaults are today's chords; each row's comment cites the arm it mirrors.

| # | `Act` | Context | name | defaults | help (`?` box) | mirrors |
|---|---|---|---|---|---|---|
| 1 | `QueueMoveDown` | `Queue` | `move_down` | `["J"]` | `move down` | `queue.rs:450` |
| 2 | `QueueMoveUp` | `Queue` | `move_up` | `["K"]` | `move up` | `queue.rs:451` |
| 3 | `QueuePause` | `Queue` | `pause` | `["P"]` | `pause / resume` | `queue.rs:455-466` |
| 4 | `QueueDequeue` | `Queue` | `dequeue` | `["Q"]` | `dequeue` | `queue.rs:467-473` |
| 5 | `QueueOpen` | `Queue` | `open` | `["enter"]` | `open` | `queue.rs:474` (`enter`) |
| 6 | `PickerOpen` | `Picker` | `open` | `["enter", "l"]` | `open directory` | `path_picker.rs:151-154` |
| 7 | `PickerParent` | `Picker` | `parent` | `["h", "backspace"]` | `parent directory` | `path_picker.rs:155-167` (`up` would hide `list.up`, PA-10) |
| 8 | `PickerChoose` | `Picker` | `choose` | `["s"]` | `choose` | `path_picker.rs:168-170` |
| 9 | `PickerChooseDir` | `Picker` | `choose_dir` | `["S"]` | `choose this directory` | `path_picker.rs:171-173` |
| 10 | `PickerGoTo` | `Picker` | `go_to` | `["/"]` | `go to path` | `path_picker.rs:174-177` |
| 11 | `PickerHidden` | `Picker` | `hidden` | `["."]` | `show hidden` | `path_picker.rs:178-181` |

Shared acts the two views use (exist, defaults unchanged): `list.down`/`up` (`j`/`Down`,
`k`/`Up`: `queue.rs:440`, `:445`; `path_picker.rs:142`, `:147`), `overlay.close` (the queue's
`Esc`, the shell's), `global.help` (the queue's `?`/`F1`, the shell's), `common.back` (the
picker's `Esc`, `path_picker.rs:182`, PA-10). Comment refreshes in shared rows: `list.down`/`up`
gain "queue.rs:440/445, path_picker.rs:142/147"; `common.back` gains "path_picker.rs:182 (the
directory picker, MOD-67 M6)".

Name checks (existing tests, must stay green): `names_are_unique_per_context`;
`no_act_name_is_a_nested_table` (`queue.open`, `picker.open`, … are no table; `[global] queue` is
`global.queue`, not `[queue]`); `no_view_name_hides_a_shared_name_it_inherits` (the picker
inherits `down`, `up`, `back`; the queue `down`, `up`: no own name repeats one);
`no_spec_is_shared_in_a_context_unless_state_guarded` (each context's defaults are distinct). No
`VIEW_DEFAULTS`, `SHADOWING` or `STATE_GUARDED` entry.

### 4.3 `keys/views.rs`: two stacks

```rust
/// The queue overlay (MOD-12 M3 D8; MOD-67 M6 D2): its own verbs, the list moves, then the
/// overlay layers the other list overlays compose (`overlay.close`, `global.help`).
pub const QUEUE_OVERLAY: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Queue,
        &[Act::QueueMoveDown, Act::QueueMoveUp, Act::QueuePause, Act::QueueDequeue, Act::QueueOpen],
    ),
    LIST,
    OVERLAY,
    HELP,
]);

/// Settings > Hierarchy, the directory picker while its go-to field is closed (MOD-49; MOD-67 M6
/// D3). With the field open the section is on [`CAPTURE`]: the field types every printable key.
pub const HIERARCHY_PICKER: Stack<'static> = Stack::new(&[
    Layer::view(
        Context::Picker,
        &[Act::PickerOpen, Act::PickerParent, Act::PickerChoose, Act::PickerChooseDir,
          Act::PickerGoTo, Act::PickerHidden],
    ),
    BACK,
    LIST,
    MODAL,
]);
```
Layer order mirrors `WAITING_LIST`/`SWITCHER` (view, `list`, `overlay`, help) and `ATTACH_PICKER`
(view, `common ∩ {back}`, `list`, `MODAL`). `MODAL`, not `GLOBAL`: the picker swallows `q`, `?` and
the digits today (`path_picker.rs:183-184`, D15), and `F1`/`ctrl-c`/`ctrl-f` pass as under
`CAPTURE` (`hierarchy.rs:1034-1036`). `CAPTURE`'s doc (`views.rs:137-142`): "hierarchy's typed
delete, its in-flight delete and its path picker" → "… and its path picker's go-to field".
Module doc gains: "M6 adds the queue overlay's and the directory picker's stacks."

Collision check with the defaults, chord by chord through each stack: `QUEUE_OVERLAY` `J K P Q
Enter | j Down k Up | Esc | ? F1` (the global layer admits only `help`); `HIERARCHY_PICKER` `Enter
l h Backspace s S / . | Esc | j Down k Up | ctrl-f ctrl-w ctrl-q F1` (modal). No chord has two
candidates; `the_compiled_defaults_validate` holds.

Files a user may have today that M6 refuses: a shared or overlay entry on a new default inside
these two stacks, e.g. `[overlay] close = ["Q"]`, `[global] help = ["P"]`, `[list] down = ["J"]`
(already refused elsewhere), `[common] back = ["h"]` (already refused in the Library import
report). The error names "in the queue overlay" or "in the Hierarchy directory picker".

### 4.4 `keys/stack.rs`: `DECLARED` (78 → 80) and `FIXED` (6 → 8)

`DECLARED` appends, after `("in a Chat replay", views::CHAT_REPLAY)`:
```rust
    // MOD-67 M6: the last two views that matched raw keys (blueprint §4.4), appended so every
    // existing first-reported phrase is unchanged.
    ("in the queue overlay", views::QUEUE_OVERLAY),
    ("in the Hierarchy directory picker", views::HIERARCHY_PICKER),
```
Doc gains "M6 appends the queue overlay's and the directory picker's."

### 4.5 `FIXED` for the M3/M4 forms (D5) — the sweep

Every production `KeyCode` arm and `key.code` match outside `ui/text_area.rs`/`ui/text_field.rs`
and the chat composer (a text widget), at HEAD:

| File:line | Arm | Stack in force | Verdict |
|---|---|---|---|
| `skills/attach.rs:266-282` `activation_key` | plain `Space` cycles activation, `Enter` submits, `Esc` cancels (no modifier but SHIFT, `:267`) | `ATTACH_FORM` (`DECLARED[43]`) | **FIXED** `["space", "enter", "esc"]` |
| `skills/library.rs:1345-1352` | `Esc` while the attachment form's save is in flight: refused with the in-flight notice, ahead of the pane | `ATTACH_FORM` | same row (cited) |
| `requirements/forms.rs:322-336` `RequirementForm::on_key`, `FormFocus::Priority` | `m` must, `l` later, `Space`/`Left`/`Right` toggle, `Esc` cancels; plain keys only (`requirements/mod.rs:536` `is_plain`) | `REQUIREMENTS_FORM` (`DECLARED[53]`) | **FIXED** `["m", "l", "space", "left", "right", "esc"]` |
| `backlog/filter.rs:251-257`, `item_form.rs:736-747`, `detail/runs.rs:1133-1138`, `:1680-1681`, `chat/mod.rs:297-300`, `:587-588` | M5's | M5 stacks | already `FIXED` |
| `ui/overlay/queue.rs:437-478` | every key | — | converted by L-A (acts) |
| `ui/path_picker.rs:141-185` | every key | — | converted by L-B (acts); the go-to field is a `TextField` (widget) |
| `chat/composer.rs:87-109` | the composer's own keys | `CAPTURE` | widget (ANA-26 §6.1) |
| `skills/attach.rs:640` | synthetic `KeyEvent::from(KeyCode::Char(c))` fed to a field | — | not a key match |

No other M3/M4 view matches a key ahead of the resolver: every Settings, Skills and Requirements
text form's `Enter`/`Esc` is its `TextField`/`TextArea`'s (`FieldOutcome::Submit`/`Cancel`), which
ANA-26 §6.1 and the plan's acceptance line ("`ui/text_area.rs`, `ui/text_field.rs` and `FIXED`
keys aside") keep out of `FIXED`, as M5 did for its text fields.

```rust
    // attach.rs `activation_key`: the activation field's plain `Space` cycles it, `Enter`
    // submits and `Esc` cancels (a choice field's value keys, ANA-26 §6.1); library.rs
    // `on_attach_key` also refuses `Esc` while the form's save is in flight.
    Fixed { stack: views::ATTACH_FORM, chords: &["space", "enter", "esc"], only_global: false, shadowed: &[] },
    // requirements/forms.rs `RequirementForm::on_key`: the priority's `m`/`l` choose,
    // `Space`/`Left`/`Right` toggle, `Esc` cancels (MOD-67 M4 D10).
    Fixed { stack: views::REQUIREMENTS_FORM, chords: &["m", "l", "space", "left", "right", "esc"], only_global: false, shadowed: &[] },
```
Inserted first in `FIXED` (they precede every M5 entry in `DECLARED`). `FIXED`'s doc
(`stack.rs:350-353`): "M5's views only: a Settings, Skills or Requirements form's choice keys are
still unlisted (M6)" → "M5's views and the M3/M4 forms' choice keys (MOD-67 M6 D5): the
Attachments activation, the Requirements priority." The compiled defaults stay valid (no act of
either stack binds any of these chords). Changed expectations: PA-11.

### 4.6 D6: one report per line and chord

Reproduction (`/tmp/m6arch/d6a.toml` = `version = 1` / `[backlog.runs]` / `approve = ["a", "1"]`):
```
d6a.toml:3: [backlog.runs] approve = "1": "1" is already global.select_tab_1 (default) in Backlog > Runs
d6a.toml:3: [backlog.runs] approve = "1": "1" is fixed in Backlog > Runs
```
Fix (`validate.rs`, PA-13): `Check` gains `said: Vec<(usize, KeyChord)>`, pushed by `report`
(`(reported_source.line.or(reported_source.entry).unwrap_or(0), chord)`) and by the capture pass;
`Check::fixed` skips an entry whose `(line, chord)` is in `said` (the first pass wins), before
pushing to its own `seen`. `validate`'s doc: "A pair and chord seen by two passes is reported
once" → add "and a line's chord the fixed pass would repeat after a collision or capture error
is not reported again (MOD-67 M6 D6)".

### 4.7 D7: reachability (`views.rs` tests)

```rust
/// MOD-67 M6 D7 (ANA-26 §8, risk "a stack action no view handles"): every catalogue act is
/// offered by some declared stack in a layer of its own context, and every act a view stack
/// offers outside the global, overlay and editor contexts is named `Act::X` in some view's
/// production code (the include-and-strip technique of `every_view_stack_is_named_by_a_view`).
#[test]
fn every_act_is_offered_by_a_stack_and_handled_by_a_view() { … }
```
- Half 1: `for spec in CATALOGUE`: `DECLARED.iter().any(|(_, stack)| stack.view_admits(spec.context,
  spec.act))`, else fail naming `[{table}] {name}`. Green at T1 (the new stacks admit the new acts).
- Half 2: `ACT_SITES: &[&str]` = the `VIEWS` list of `every_view_stack_is_named_by_a_view`
  (hoisted to a module-level `const` both tests read) **plus** `ui/tabs/chat/transcript.rs`
  (`ChatTop`/`ChatBottom` are named only there, checked at HEAD), `ui/overlay/queue.rs` and
  `ui/path_picker.rs`; production code only (split at `\n#[cfg(test)]`, `//` lines dropped).
  For every declared stack outside `SHELL` and every act it admits at any layer whose context is
  not `Global`, `Overlay` or `Editor`, `Act::{variant}` (followed by a non-identifier character)
  must occur. The variant name comes from `format!("{act:?}")`. `PENDING_ACTS` (PA-4): the 11
  acts of §4.2, skipped.
- A stricter per-view check (the act named in the file that returns the stack) was considered and
  rejected: the Backlog tab handles its panes' `list.*`/`backlog.*`, the Settings tab its sections'
  `settings.*`, so it would need a hand-kept map.
- Checked at HEAD by emulation (`/tmp/m6arch/d7.py`): both halves already hold for all 159 acts and
  78 stacks; no gap to assert per stack beyond `the_compiled_defaults_validate`.

`every_view_stack_is_named_by_a_view`: `VIEWS` gains `ui/overlay/queue.rs`; `PENDING: &[&str] =
&["QUEUE_OVERLAY", "HIERARCHY_PICKER"]` skipped (PA-4). T-close empties both lists.

### 4.8 D8: `docs/htui-keys.md` and its pin

The page (created in T1; exact header, then the block):
````markdown
# htui keys

Every key htui binds by default, exactly as `htui --default-keys --print-keys` prints them: a
complete `keys.toml`. To change a key, copy its line into `keys.toml` in htui's configuration
directory and edit it there; list only what you change (README, [Changing keys](../README.md#changing-keys)).

This page is generated. After a change to the key catalogue, replace the block below with the
output of `cargo run -q -p htui -- --default-keys --print-keys`; `cargo test -p htui --test
keys_file` fails until it matches.

```toml
<the print, verbatim, ending with its last row's newline>
```
````
Pin, `tests/keys_file.rs` (a new top-level module `reference_page`, no `cfg`: it needs no binary,
no `testkit`):
```rust
/// MOD-67 M6 D8: `docs/htui-keys.md`'s toml block is the default print, byte for byte.
#[test]
fn the_key_reference_page_is_the_default_print() {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/htui-keys.md"))
        .expect("docs/htui-keys.md exists");
    let block = page.split_once("```toml\n").and_then(|(_, rest)| rest.split_once("```\n"))
        .map(|(block, _)| block).expect("one ```toml block");
    assert!(
        block == htui::keys::print(&htui::keys::Keys::defaults()),
        "docs/htui-keys.md is stale: regenerate its toml block with \
         `cargo run -q -p htui -- --default-keys --print-keys`"
    );
}
```
(`assert!` with the message, not `assert_eq!`: a 280-line diff would bury the regenerate line.)
The page is regenerated by T1 from its own build (after §4.1-§4.2), so its last tables are
`[queue]` and `[picker]`.

### 4.9 D9: the carried print lows (reproduced; PA-2)

```
# d9a.toml: [form] next_field = ["ctrl-n"]  /  [settings.agents] next_field = ["tab", "down"]
$ htui --keys d9a.toml --print-keys | grep next_field
next_field      = ["ctrl-n"]   # next field (changed)      <- [form]
next_field     = ["tab", "down"]    # next field           <- [settings.agents]: pinned, UNMARKED (low 1)
next_field    = ["ctrl-n", "down"]  # next field           <- [settings.hierarchy], derived
# d9c.toml: [form] next_field = ["ctrl-n"] (line 3)
#           [settings.agents] next_field = ["tab", "down"] (line 5), prev_field = ["down"] (line 6)
d9c.toml:6: [settings.agents] prev_field = "down": "down" is already settings.agents.next_field (line 3) in the Agents form
                                                                                               ^ low 2: it is line 5
```
Cause: `Keys::set` keeps `line` only when the chords differ from the compiled row; a derived row's
compiled chords are its compiled derivation (`["tab", "down"]`), so the pinned row gets
`line = None`; `derive` keeps its chords (`set_by_file`) but never restores the line. `print::mark`
then sees no line, and `validate::source` treats a line-less `VIEW_DEFAULTS` row as derived and
names the shared `[form]` row.

Fix (`keys/mod.rs` `Keys::derive`, `:172-176`):
```rust
            if set_by_file.contains(&(row.context, row.act)) {
                // MOD-67 M6 D9: a row the file pins keeps its line while it differs from the
                // derivation in force, even when it equals the compiled one.
                row.line = (row.chords != derived).then_some(row.entry).flatten();
            } else {
```
`derive`'s doc: "loses its line when they equal the derivation" → "keeps the line of its entry
while they differ from the derivation and loses it when they equal it, so a printed file reads back
unmarked and a pinned row stays marked".

### 4.10 Tests: red first, and the existing tests T1 changes

New (each fails before its code lands):

| Id | File | Test | Asserts |
|---|---|---|---|
| T-C1 | `catalogue.rs` | `every_context_is_listed_once_in_all` (**42**), `every_act_is_listed_and_has_a_row` (`ALL`/`position` +11), `every_act_has_exactly_one_row` (**170**), `context_tables_and_headings` (+2 rows; `Queue` and `Picker` views, not shared) | lockstep |
| T-V1 | `views.rs` | `the_queue_overlay_and_picker_stacks_resolve_as_designed` (new) | `QUEUE_OVERLAY`: `J` → `[QueueMoveDown]`, `K` → `[QueueMoveUp]`, `P` → `[QueuePause]`, `Q` → `[QueueDequeue]`, `enter` → `[QueueOpen]`, `j`/`down` → `[ListDown]`, `esc` → `[OverlayClose]`, `?`/`f1` → `[Help]`, `ctrl-j`/`alt-J`/`q`/`1` → `[]`, `passes(q)` (the global layer is unfiltered `HELP`; the overlay is modal at the shell). `HIERARCHY_PICKER`: `enter`/`l` → `[PickerOpen]`, `h`/`backspace` → `[PickerParent]`, `s` → `[PickerChoose]`, `S` → `[PickerChooseDir]`, `/` → `[PickerGoTo]`, `.` → `[PickerHidden]`, `esc` → `[Back]`, `j` → `[ListDown]`, `q`/`?`/`1` → `[]`, `f1` → `[Help]`, `ctrl-f` → `[Find]`, `!passes(q)`, `passes(ctrl-j)` |
| T-V2 | `views.rs` | `every_act_is_offered_by_a_stack_and_handled_by_a_view` (new, §4.7) | as §4.7, `PENDING_ACTS` skipped |
| T-V3 | `views.rs` | `every_view_layer_comes_first_and_every_modal_stack_ends_modal` | **72**; `every_view_stack_is_named_by_a_view` with `PENDING` |
| T-S1 | `stack.rs` | `declared_starts_with_base_and_overlay_and_holds_every_view_stack` | `len() == 80` |
| T-S2 | `stack.rs` | `every_fixed_entry_is_a_declared_stack_once` | unchanged body, green over 8 entries; plus new `fixed_follows_declared_order` (each entry's `DECLARED` index increases) |
| T-X1 | `validate.rs` | `a_line_both_colliding_and_fixed_is_reported_once` (new, D6) | `errors("[backlog.runs]\napprove = [\"a\", \"1\"]\n")` == `one(3, "[backlog.runs] approve = \"1\": \"1\" is already global.select_tab_1 (default) in Backlog > Runs")` |
| T-X2 | `validate.rs` | `a_rebind_onto_a_requirements_priority_or_attachment_key_is_refused` (new, D5) | `[requirements]\nnext_field = [\"tab\", \"right\"]\n` → `one(2, "[requirements] next_field = \"right\": \"right\" is fixed in a Requirements form")`; `[form]\nnext_field = [\"tab\", \"enter\"]\n` → `one(2, "[form] next_field = \"enter\": \"enter\" is fixed in the Attachments form")` (the first declared stack that keeps `enter`; exact text from the run); `[form]\nsave = [\"ctrl-s\", \"left\"]\n` → `one(2, "[form] save = \"left\": \"left\" is fixed in a Requirements form")` |
| T-X3 | `validate.rs` | `a_collision_names_a_pinned_view_rows_own_line` (new, D9 low 2) | the d9c file → `one(6, "[settings.agents] prev_field = \"down\": \"down\" is already settings.agents.next_field (line 5) in the Agents form")` |
| T-P1 | `print.rs` | `a_view_row_pinned_to_its_default_while_its_shared_row_moves_is_marked` (new, D9 low 1) | the d9a file: the `[settings.agents]` table's `next_field` line ends `# next field (changed)`; `(changed)` count 2; the print reads back equal and re-prints byte-identical |
| T-M1 | `keys/mod.rs` | `a_pinned_view_row_keeps_its_line_while_the_shared_row_moves` (new) | d9a: `line(SettingsAgents, FormNextField) == Some(5)`; `[settings.agents]\nnext_field = [\"tab\", \"down\"]\n` alone: `None` |
| T-L1 | `load.rs` | `a_queue_or_picker_table_rebinds_its_view` (new) | `[queue]\nmove_down = \"D\"\n` loads; `actions(QUEUE_OVERLAY, D)` = `[QueueMoveDown]`, `J` → `[]`; `[picker]\nopen = \"o\"\n` → `actions(HIERARCHY_PICKER, o)` = `[PickerOpen]`; `[picker]\nup = \"u\"\n` → "[picker] up: no such action; [picker] has open, parent, choose, choose_dir, go_to, hidden, and may override down, up, back" (order from the run: catalogue order of the overridable shared rows) |
| T-K1 | `tests/keys_file.rs` | `the_key_reference_page_is_the_default_print` (new, §4.8) | — |

Existing expectations T1 changes (and only these):
- `load.rs` `TABLES` and `tests/keys_file.rs` `bad_fixtures` "errors" (`:123`): `…, backlog.item,
  chat` → `…, backlog.item, chat, queue, picker`.
- `print.rs` `the_default_print_starts_with_the_header_global_and_overlay`: the tail is now
  ```
  [queue]
  move_down = ["J"]      # move down
  move_up   = ["K"]      # move up
  pause     = ["P"]      # pause / resume
  dequeue   = ["Q"]      # dequeue
  open      = ["enter"]  # open

  [picker]
  open       = ["enter", "l"]      # open directory
  parent     = ["h", "backspace"]  # parent directory
  choose     = ["s"]               # choose
  choose_dir = ["S"]               # choose this directory
  go_to      = ["/"]               # go to path
  hidden     = ["."]               # show hidden
  ```
  (padding computed with print's algorithm; confirm on the run). Still ≤ 100 columns.
- `validate.rs` (PA-11): `a_rebind_onto_a_filter_form_fixed_key_is_refused` last case → "is fixed in
  a Requirements form"; `a_rebind_onto_an_item_form_or_runs_question_fixed_key_is_refused` first
  case → "is fixed in the Attachments form"; `a_shifted_named_fixed_key_is_refused_too` third case
  → "is fixed in a Requirements form". Each comment gains "MOD-67 M6: the first declared stack
  that keeps the key fixed".
- Checked and unchanged: `a_collision_over_an_overlay_is_found_in_the_overlay_stack` (the queue
  overlay's `esc`/`help` pair is already reported "over an overlay"),
  `a_global_rebind_onto_a_view_verb_is_refused`, `a_global_entry_on_a_list_default_is_refused_…`,
  `a_printable_shared_chord_is_reported_once_on_the_shared_line` (PA-13), `the_compiled_defaults
  _validate`, `every_allow_list_entry_is_demanded`, `stack.rs`
  `a_global_chord_on_the_view_switch_names_the_skills_views` (the picker's global layer is modal,
  the queue's admits only `help`), every `tests/keys.rs` case. No snapshot moves in T1.

### 4.11 T1 commit groups (each compiles, clippy both ways, its tests green)

1. **T1a** `feat(mod-67): M6 queue and picker contexts and acts` — §4.1-§4.2, T-C1, `TABLES`,
   the print tail, the `bad_fixtures` table list.
2. **T1b** `feat(mod-67): M6 queue overlay and picker stacks; FIXED for the M3/M4 forms` — §4.3-§4.5,
   T-V1, T-V3, T-S1, T-S2, T-X2, T-L1, PA-11's three expectations.
3. **T1c** `fix(mod-67): M6 validator reports a line's chord once; pinned view rows keep their line`
   — §4.6, §4.9 (PA-2), T-X1, T-X3, T-P1, T-M1.
4. **T1d** `test(mod-67): M6 every act offered and handled` — §4.7, T-V2.
5. **T1e** `docs(mod-67): M6 generated key reference page` — §4.8, T-K1.

### 4.12 T1 gate
```bash
cargo test -p htui --lib keys
cargo test -p htui --features testkit --test keys --test keys_file
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features --check     # zero pending
```

---

## 5. The lanes

### 5.1 Summary (details in the lane files)

| Lane | Source | Stacks it names | D4 sites | Snapshots it may move | Rebinding test |
|---|---|---|---|---|---|
| L-A | `ui/overlay/queue.rs` | `QUEUE_OVERLAY` | the hint (byte-identical) | none (all nine queue snapshots byte-identical) | `[queue] move_down = "D"` |
| L-B | `ui/path_picker.rs`, `ui/tabs/settings/hierarchy.rs` | `HIERARCHY_PICKER` (+ `CAPTURE` while go-to is open) | 6 + the two picker hint halves + `HINT_PICKING` | none (byte-identical with the defaults) | `[picker] open = "o"`, `[settings.hierarchy] new_workspace = "W"` |
| L-C | 27 files (L-C §2) | none new | 53 (L-C §1) | 3 (`Ctrl+S` → `Ctrl+s`, PA-7) | one per family (L-C §5) |

Lane gate (each, in its worktree):
```bash
rustfmt --edition 2024 --check <own .rs paths>
cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features --no-fail-fast -- --test-threads=1
cargo insta test -p htui --all-features --check     # after accepting the lane's own snapshots
```

### 5.2 File-set check

Source sets (tests and snapshots in the lane files):

| Task | Source files |
|---|---|
| T0 | `keymap.rs` (deleted), `lib.rs`, `testkit.rs`, `app/{state,update,pane}.rs`, `keys/{chord,catalogue,hint}.rs` (docs), `ui/tabs/registry.rs`, `ui/overlay/registry.rs`, `settings/agents.rs` (doc `:121-123`), test benches of `ui/overlay/{concepts_search,queue,quit_confirm,waiting_list}.rs`, `backlog/mod.rs`, `backlog/detail/{compose,graph,requirements,runs}.rs`, `chat/mod.rs`, `requirements/mod.rs`, `skills/{agent_help,library,templates}.rs` |
| T1 | `keys/{catalogue,views,stack,validate,print,mod,load}.rs`, `tests/keys_file.rs`, `docs/htui-keys.md` |
| L-A | `ui/overlay/queue.rs` |
| L-B | `ui/path_picker.rs`, `ui/tabs/settings/hierarchy.rs` |
| L-C | `editor.rs`, `agent_settings.rs`, `agent_worker.rs` (test `:16057` only), `ui/overlay/{concepts_search,workspace_switcher}.rs`, `ui/tabs/backlog/{mod,divergence,item_form}.rs`, `backlog/detail/{compose,documents,notes}.rs`, `chat/{mod,transcript}.rs`, `requirements/mod.rs`, `settings/{mod,agents,boxes,connection,personas,prompt,qdrant,queue,secrets}.rs`, `skills/{agent_help,attach,library,templates}.rs` |
| X-S (T-close) | `ui/overlay/queue.rs`, `ui/tabs/backlog/mod.rs` (`queue_sentence` and its two calls) |
| T2 | `ui/layout.rs`, `ui/tabs/registry.rs`, `app/state.rs`, `settings/{mod,agents,boxes,connection,hierarchy,kinds,personas,prompt,qdrant,queue,secrets}.rs`, `skills/{mod,library,templates,attach,agent_help}.rs`, `requirements/mod.rs`, `backlog/{mod,list,filter,item_form,divergence}.rs`, `backlog/detail/{mod,runs,graph,documents,notes,requirements,compose}.rs`, `chat/{mod,composer}.rs` |

Intersections (computed from the table):

| Pair | Intersection | Safe because |
|---|---|---|
| L-A ∩ L-B, L-A ∩ L-C, L-B ∩ L-C | **∅** | — (the concurrent tasks) |
| T1 ∩ any lane | **∅** | — |
| T0 ∩ L-A | `queue.rs` (test bench) | T0 is committed before the lanes branch |
| T0 ∩ L-B | ∅ | — |
| T0 ∩ L-C | `concepts_search.rs`, `backlog/mod.rs`, `compose.rs`, `chat/mod.rs`, `requirements/mod.rs`, `agent_help.rs`, `library.rs`, `templates.rs`, `settings/agents.rs` | serial (T0 first) |
| X-S ∩ L-A, X-S ∩ L-C | `queue.rs`; `backlog/mod.rs` | serial (after both merge) |
| T2 ∩ L-A | ∅ (overlays untouched) | — |
| T2 ∩ L-B | `hierarchy.rs` | serial (T2 after the merges) |
| T2 ∩ L-C | `backlog/{mod,divergence,item_form}.rs`, `detail/{compose,documents,notes}.rs`, `chat/mod.rs`, `requirements/mod.rs`, `settings/{mod,agents,boxes,connection,personas,prompt,qdrant,queue,secrets}.rs`, `skills/{agent_help,attach,library,templates}.rs` | serial |

Test-file sets are pairwise disjoint too (L-A `tests/queue_overlay.rs`; L-B `tests/hierarchy.rs`;
L-C §4 of its file, which holds neither); `tests/secrets_settings.rs:2419` asserts hierarchy's
`reloaded; press p again` is absent from a secrets frame, a negative check L-B's byte-identical
text keeps true (no L-B edit there).

---

## 6. T2: the fixed hint row (Q1 (a); serial, primary tree, after the lane merges and X-S)

### 6.1 The trait methods (PA-3)

```rust
// ui/tabs/registry.rs, trait Tab
/// The keys of this tab's focused mode for the shell's hint row (MOD-67 M6 D10, design review
/// finding 3): the mode's hint spec rendered through its own stack, so a rebound key shows its
/// new chord. `width` is the row's; a tab that composes two hints keeps the second only when both
/// fit. `None`: no row (the test doubles; a tab with nothing to say). Defaulted.
fn key_hint(&self, _width: u16, _ctx: &Ctx<'_>) -> Option<Line<'static>> { None }

// settings/mod.rs, trait SettingsSection
/// The keys half of this section's hint (its mode's spec through its stack, plus what a write in
/// flight adds), for the shell's row. Defaulted to `None`.
fn key_hint(&self, _keys: &Keys, _theme: &Theme) -> Option<Line<'static>> { None }

// backlog/detail/mod.rs, trait DetailTab
/// The keys of this pane's current mode for the Backlog's part of the shell's row. Defaulted to
/// `None` (Body and Prompt, which have no key line of their own).
fn key_hint(&self, _keys: &Keys, _theme: &Theme) -> Option<Line<'static>> { None }
```
Each view's existing hint builder becomes the body of its `key_hint` (`hint_text` in the Settings
sections, `ChatTab::hint`, `RequirementsTab::hint`, the Library/Templates hint match, the Runs and
Reqs footers' spec lines, Graph's/Docs'/Notes' `HINT`); the old private names are renamed or
inlined so no `fn hint` remains beside a `key_hint` in one type.

### 6.2 `layout::chrome` and `App::render`

- `Chrome` gains `pub hint: Rect` ("One line: the active tab's keys (MOD-67 M6), blank while an
  overlay or the in-pane editor holds them"); `chrome(area)` splits `[top_bar 1, tab_strip 1, body
  Min(1), hint 1, status 1]`. Doc "four fixed regions" → "five".
- **Height rule: always one row.** The row is reserved on every frame, blank when there is nothing
  to say, so the body's height never depends on the mode (no reflow on opening a form or an
  overlay) and every full-frame snapshot moves the same way: body one row shorter.
- `App::render`, after the tab and the editor:
  ```rust
  let hint = match self.tabs.active() {
      Some(tab) if self.overlays.is_empty() && !self.editor_visible() => {
          tab.key_hint(chrome.hint.width, &self.ctx(Origin::Tab(tab.id())))
      }
      _ => None,
  };
  if let Some(line) = hint {
      frame.render_widget(Paragraph::new(clip_line(line, chrome.hint.width)), chrome.hint);
  }
  ```
  Blank under any overlay (the overlay's box has its own hint, plan Q1 (a)); blank while the
  in-pane editor's tab is on screen (focused, the editor has the keys; unfocused, the tab refuses
  its own keys, `Stack::EDITOR_OPEN`). Under the `?` box the row stays (the box is centred over the
  body). `clip_line` (new, `ui/cells.rs`): cuts a `Line` to `width` cells span by span, the last
  kept span ending in `…` (`cells::clip`'s rule), styles kept.

### 6.3 What each tab and mode shows in the row

| Tab | Mode | Row | In-view line that goes |
|---|---|---|---|
| Backlog | divergence view open | `view.hint(keys)` (`ITEM_DIVERGENCE`) | `divergence.rs:547-553` (`hint_at` row removed from its layout) |
| Backlog | item form open | the form's spec by focus (`ITEM_FORM`, `item_form.rs:1121-1126`) | `item_form.rs:1127-1133` (row removed) |
| Backlog | filter form open | `HINT_TAGS`/`HINT_CHOICES` (`BACKLOG_FILTER`) | `filter.rs:444-450` (the form's last line; `filter::FORM_HEIGHT` − 1) |
| Backlog | a pane captures (Runs note/follow-up/typed key, Reqs picker/kind/uncite, Docs/Notes compose, Runs artifact) | the pane's `key_hint` only (no list keys: `list_hint` is `None` while anything captures, `mod.rs:186-195`) | each pane's spec line (below) |
| Backlog | browse | **the focused pane's `key_hint`, then ` · ` and `LIST_HINT` when the joined width fits `width`; the pane's alone when not; `LIST_HINT` alone when the pane has none (Body, Prompt)**. At 100 columns, Runs: `J/K step · Enter replay · v flow · ? keys · j/k move · h/l sub-tab · f filter · N new · e edit` (94 cells) | `list::render`'s `hint` parameter and the list pane's last row (`list.rs` `render`, `mod.rs:982-983`): the list regains the row |
| Backlog panes | Runs | the mode's spec (`RUNS_{LIST,FLOW,EMPTY}_HINT`, `REJECT_HINT`, …, `ARTIFACT_HINT`) through `self.stack()` | the spec lines of `footer` (`runs.rs:2091-…`; questions and fields stay) and `render_artifact`'s hint (`:1011`) |
| | Graph | `GRAPH_HINT` | the ` · {hint}` tail of the footer (`graph.rs:777-782`); `depth d/3` stays |
| | Docs / Notes | `HINT` (browse) or the compose area's spec (`compose.rs:643-647`) | the dedicated hint rows (`documents.rs:629-638`, `notes.rs:447-456`, `compose.rs:648-651`) |
| | Reqs | browse: moves + writes, writes styled `base`/`dim` by `writable()`; picker/kind/uncite: their spec | `footer`'s hint lines (`detail/requirements.rs:438-…`); titles and candidates stay |
| | Body, Prompt | `None` | — (Prompt's `n / p · N templates` header is pane content, not a hint line: stays, noted for MOD-82) |
| Skills | every mode | today's hint string (`library.rs:731-766`, `templates.rs:545-592`), incl. the editor's ` · L3:C5` and the attachments pane's and agent help's hints | the `hint_row` (`library.rs:725-730`, `:776-779`; templates the same): removed from the layout (net zero for the content) |
| Requirements | every mode | `hint()` minus the filter field: Browse its three groups (writes greyed when not allowed), forms their spec | the `hint_row` keeps the filter input (` /` + field) while filtering and is blank otherwise (kept: stable tree height) |
| Settings | every section, every mode | the section's `key_hint` = its `hint_text` (keys, plus "`x` in flight" where it adds it) | sections whose hint row is keys only drop the row (agents `keys_area`, boxes, personas `hint_area`, qdrant's pushed line); sections whose row also carries the notice (connection, hierarchy, kinds, prompt, queue, secrets: `hint(width, theme, keys)` `keys · notice`) keep the row and draw only the notice |
| Chat | every mode | `hint(keys)` (replay, compose, armed end, parked, ended, live, idle) | the composer row shows the input while composing and is blank otherwise (`Composer::render` loses its `hint` parameter); the replay's composer-row hint (`chat/mod.rs:851-856`) goes |

Overlays (`queue`, `waiting_list`, `workspace_switcher`, `concepts_search`, `migration_prompt`,
`quit_confirm`, `abort_confirm`) and the path picker's popup keep their own hints: they are boxes
over the screen (plan Q1 (a)). Pane titles that name keys (the Library/Templates browse pane's
` J/K scroll ` title, `library.rs:1674`, `templates.rs:1159`) are pane chrome, not hint lines:
unchanged, noted for MOD-82.

### 6.4 Tests (write first)

- `ui/layout.rs`: `the_chrome_has_one_hint_row_above_the_status_line` (100×30: body `y 2..28`
  height 26 → 25, hint `y 28`, status `y 29`; a 6-row frame keeps body 1).
- `tests/shell.rs`: `the_hint_row_shows_the_active_tabs_keys` (demo, Backlog, Body pane: row 28 is
  `j/k move · h/l sub-tab · f filter · N new · e edit`); `the_hint_row_is_blank_under_an_overlay`
  (`w` opens the switcher: row 28 blank); `the_hint_row_follows_a_rebound_key` (`[backlog] filter =
  "/"` → `/ filter`).
- `backlog/mod.rs` unit: `the_pane_hint_leads_and_the_list_keys_follow_when_they_fit` (Runs pane:
  width 100 → both, width 60 → pane only), `a_capturing_pane_shows_only_its_own_keys`.
- `tests/chat.rs`: `the_composer_hint_shows_while_composing` (`i`: row 28 `Enter send · Esc leave`).
- `tests/requirements.rs`: the read-only Browse row's writes span is `theme.dim` (style check on the
  buffer cell, as the existing greyed-writes test does).

### 6.5 Commit groups

1. `feat(mod-67): M6 the shell's hint row` — layout, `Tab::key_hint`, `App::render`, `clip_line`,
   the layout and shell tests (tabs still draw their own lines: the row shows them twice, no
   snapshot accepted yet).
2. `feat(mod-67): M6 every tab's keys in the hint row` — the per-tab table above, one tab per
   commit if the diff is large (Settings, Skills, Requirements, Backlog, Chat), then the one
   re-baseline.

### 6.6 Expected snapshot drift (reviewed line by line, nothing else accepted)

- **Full-frame (113)**: the body's bottom border moves up one row; the new row 28 holds the tab's
  keys (blank under an overlay or the editor); views that lost a dedicated hint row show one more
  content row inside the body; the status line is unchanged. Files: every `backlog__` (40),
  `chat__` (14), `skills__` (9), `templates__` (8), `hierarchy__` (6), `requirements__` (6),
  `concepts_search__` (5, row blank: overlay), `shell__` (5), `kinds__` (3), `prompt_settings__`
  (3), `settings__` (3), `prompt_preview__` (2), `waiting__` (2, blank), `box_settings__` (1),
  `integration__` (1), `probe__` (1), `replay__` (1), and in `src`:
  `htui__app__pane__tests__{another_tab_while_editing,the_help_box_on_the_asking_tab,the_pane_over
  _the_claimed_rect}` (blank: editor), `htui__testkit__tests__shell_empty`,
  `htui__ui__editor_pane__tests__the_{focused,unfocused}_frame` (count from the run; these are the
  frames whose last line is a status line).
- **Section-level benches** (render one section or tab without the shell): the keys vanish from the
  section's own row (row blank where it carries a notice, removed where it was keys only, the
  content gaining it): `box_settings__` (8), `personas__` (7), `secrets_settings__` (7),
  `settings__` (7), `kinds__` (6), `connection__` (5), `queue_settings__` (5), `hierarchy__` (3),
  `prompt_settings__` (3), `backlog__` (1), `chat__` (1), `requirements__` (1).
- **Not moving**: overlay crops (`queue_overlay__opened`, `htui__ui__overlay__*`, 10 in `src`), CLI
  (`state_cli_pg__` 5, `htui__state_cmd__` 12).

T2 gate: the full Validation (plan) plus `cargo insta test -p htui --all-features` reviewed with
`cargo insta review` (or `.snap.new` diffs) against §6.6, then `--check`.

---

## 7. T-close

**Part 1 (after the lanes, before T2)**
1. Merge `mod-67-m6-l-a`, `-l-b`, `-l-c`, each `--no-ff`; after each, the full Validation with
   `--test-threads=1 --no-fail-fast` and `cargo insta test --check`. Expected: no conflict (§5.2);
   no snapshot pending after any merge (the lanes accepted their own).
2. **X-S** `feat(mod-67): M6 the queue sentences name the pause key` (PA-5):
   `backlog::queue_sentence(write, key, view, pause: Option<&str>)`; `Stale { pause: true }`
   sentences become `format!("queue already paused: its batch closed before {pause} reached it")`
   (unbound: `"queue already paused: its batch closed first"`) and `format!("queue not paused: it
   was resumed since it was read; {pause} again pauses it")` (unbound: drop `; … pauses it`). The
   Backlog passes `ctx.keys().label(self.detail.key_stack(), Act::BacklogPauseQueue)`
   (`mod.rs:526`), the overlay `ctx.keys().label(views::QUEUE_OVERLAY, Act::QueuePause)`
   (`queue.rs:506`). Byte-identical with the defaults (`mod.rs:1285-1299`, `queue.rs` unit,
   `tests/queue_overlay.rs` unchanged). Pins first: `backlog/mod.rs` unit `a_rebound_pause_key_is
   _the_one_the_stale_sentence_names` (`Some("X")` → `… X again pauses it`, `None` → no clause);
   `tests/queue_overlay.rs` `a_rebound_pause_key_is_named_after_a_stale_pause` (`[queue] pause =
   "X"`).
3. `test(mod-67): M6 every stack named, every act handled` — empty `PENDING` and `PENDING_ACTS`
   (`views.rs`); both tests green over all 80 stacks and 170 acts.

**T2** (§6).

**Part 2 (after T2)**
4. README (D11), "Changing keys": add after the `[backlog]` sentence "`[queue]` holds the queue
   overlay's keys (`Ctrl+q`: move, pause, dequeue, open) and `[picker]` the directory picker's
   (Settings › Hierarchy's `b`)."; the fixed-keys sentence gains "the Requirements priority's
   `m` / `l` / `Space` / `←` / `→` and the attachment form's activation `Space`"; end of the
   section: "Every key with its default is listed in [`docs/htui-keys.md`](docs/htui-keys.md)."
   No "not configurable yet" sentence remains (checked at HEAD). Key tables re-checked against the
   defaults (Backlog, Runs, Requirements, Chat, Settings: unchanged keys); add a one-line Queue
   overlay note under Backlog ("`Ctrl+q` opens this box's queue: `j`/`k` move, `J`/`K` reorder, `P`
   pause or resume, `Q` dequeue, `Enter` opens the item"), and mention the hint row: "The row above
   the status line names the keys of what you are in; `?` lists all of them."
5. ANA-26 §8 M6 row: "22 call sites in 11 files" → "42 call sites in 17 files"; "~15" → the
   merged count.
6. HANDOFF: MOD-67 closed (P2), M6 phase note (PA list, user-visible list of §1), DECISIONS index;
   `docs/decisions/mod/mod-67.md` write-up (M1-M6). Carried to MOD-82: the Prompt header's `n / p`
   and the Skills pane titles' ` J/K scroll ` (§6.3).
7. The plan's review gate; `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`;
   Postgres suites once on the merged tree (sandbox DB 5439); the workspace gate (the "zeta" check;
   `htui-worker` `auto_queue::a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` is MOD-95's);
   remove the three worktrees, then the lane branches; remove the empty `~/.config/htui/trees` a
   full run leaves (M4/M5 note).

---

## 8. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| T2's ~160 snapshot moves hide a behaviour change | High | T2 alone moves rows; §6.6 is the accept list (row 28, the body border, the removed in-view hint text, one regained content row) and nothing else; the commit-1/commit-2 split keeps the shell change reviewable before the re-baseline |
| L-C's 53 sentences in 27 files take a label from the wrong stack (a key the mode does not answer) | Medium | L-C §1 names the stack per site; one rebinding pin per family under a rebound key (L-C §5) |
| PA-7's spelling change breaks assertions outside L-C's list | Medium | L-C §4 lists every file found; the lane gate runs the full suite with `--no-fail-fast` |
| A shared prose function changed in one lane breaks a caller in another | Medium (was High) | PA-5 ownership; X-S for the one function whose callers cannot share a lane |
| New stacks or `FIXED` rows refuse a user file that loads today | Low | §4.3, §4.5 list the shapes; the error names line and stack; README (T-close 4) |
| The D9 fix changes an existing print or validation expectation | Low | Only files that pin a view row whose shared row they rebind; T-P1/T-X3/T-M1 pin it; `a_view_override_and_a_derived_row_round_trip` and `a_shared_rebind_flows_into_the_view_defaults` stay green (checked: their rows equal the derivation) |
| `cargo test` reads `docs/` from the crate dir | Low | runtime read via `CARGO_MANIFEST_DIR/../../docs` (no `include_str!` across the crate boundary); the workspace layout is fixed |
| Three worktrees' disk | Low | ~10 GB `target` each; 3.0 TB free (`df -h .`, 2026-10-13) |
