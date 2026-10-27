# Plan: MOD-115 M4 — tabs ported to the kit, group B (Backlog, Skills, overlays, editor, `?` box)

**Source PRD**: `.claude/prds/mod-115-tui-shell.prd.md`
**Selected Milestone**: 4 — Tabs ported, group B
**Complexity**: Large (one 5k-line file split, ~17.5k production lines ported, ~75 snapshots
rewritten, the M1 adapter deleted, four absorbed items and MOD-115 itself closed)

## Summary
M3 put Settings, Requirements and Chat on the kit (`draws_chrome() == true`). Everything else
still frames itself, and the shell still paints focus and hints over it through the M1 adapter.
`tests/kit_chrome.rs` `GROUP_B` lists 18 files that do so.

M4:
- splits `backlog/mod.rs` along its concerns, as a pure move;
- gives the helpers Requirements and Skills borrow from Backlog a neutral home;
- ports **group B**: Backlog (list, filter, forms, divergence view, the seven detail sub-tabs,
  the Runs flow), Skills (Library, Templates, attachments, agent help), the overlays (concepts
  search, workspace switcher, migration prompt, queue, abort question), the in-pane editor frame,
  the `?` box and the no-tab body;
- deletes the transition: `Tab::draws_chrome`, the hint adapter (`paint_shell_hints`,
  `join_panes_under`, `hint_row`) and the shell's focus restyle;
- closes MOD-81, MOD-82, MOD-83 and MOD-85, and then MOD-115.

"Ported" means chrome and presentation only, as in M3. Each tab's redesigned *content* stays with
its own item: Backlog's IDE layout, tree and Runs redesign (MOD-119), the activity pane (MOD-122),
the Toolbox (MOD-124), the graphs (MOD-120).

## Size of group B (counted on `8a4a88c6`)
| Surface | Production lines | Self-framing sites | Snapshots (own + collateral) |
|---|---|---|---|
| Backlog: `mod.rs` | 1,359 (tests 3,677) | 0 | — |
| Backlog: list, filter | 731 | 4 | every `backlog__*` (41), list half |
| Backlog: item form, divergence | 1,819 | 3 | 3 |
| Backlog: detail frame + Body, Graph, Prompt, Reqs | ~2.2k | 2 | 14 + collateral 16 |
| Backlog: Docs, Notes, compose | ~1.9k | 2 | 6 |
| Backlog: Runs + flow | 2,919 | 1 | 12 + `replay__` 1 |
| Skills (5 files) | 5,863 | 15 | 17 |
| Overlays (5) | ~1.6k | 10 | in-crate 8 + `queue_overlay` 1 + `concepts_search` 5 + `shell__` 3 |
| Editor frame, `?` box, no-tab body, abort | ~400 | 10 | in-crate `app__pane` 4 + `editor_pane` 2 |

Collateral = snapshots of other test files that draw Backlog under them: `shell__` (5 of 6),
`concepts_search__` 5, `waiting__` 2, `integration__` 1, `prompt_preview__` 2, `replay__` 1, and the
testkit's `shell_empty` (in-crate). Excluded: `state_cli_pg__` (JSON) and `project_types__` (already
on the kit).

**The coupling that shapes the order.** Every `backlog__*` snapshot draws the list *and* the
detail. Each overlay or shell snapshot draws Backlog under it. So:
- anything that changes the list or the frame touches every Backlog snapshot, and it runs alone;
- only after it can the detail sub-tabs fan out, each owning its own snapshots.

M3 could fan out group A at once because no group-A snapshot drew another tab. M4 cannot.

## Maintainer calls carried in (from M3, already confirmed)
- D-15a (shift arrows listed after `h`/`l`) and D-15b (`MODAL` keeps tab keys out of forms) hold.
- **MD-A** closes on group B: each group-B form resolves its acts through `Ctx::pressed`, and
  every hint or sentence naming a key builds it from `Keys`. The RV4 deferral ends here.
  - Literal sites known today: Skills `NAMING_HINT`/`IMPORT_HINT` (`library.rs:148,173`),
    Templates `NAMING_HINT` (`templates.rs:106`), agent help `"Enter ask"` (`agent_help.rs:87`),
    concepts search `IDLE`/`CHANGED`/`"Enter search/open"` (`concepts_search.rs:71,92,97`).
  - `Esc …` literals stay: `Esc` is never configurable (M2 MD7).

## Design decisions (proposed; CONFIRM accepts or amends them)
- **E1 — Neutral homes for what Backlog lends.**
  - `detail::{Scroll, Step, capture_area, STAMP}` (imported by Requirements and Skills) and
    `list::window` (Requirements) move under `ui/kit/`. The architect fixes the exact homes.
  - `backlog::queue_sentence` (imported by the queue overlay) moves to a home outside both.
  - Every importer is updated in T0, which runs alone; no snapshot moves.
  - `backlog::not_in_this_backlog` and `backlog::BacklogTab` keep their public paths
    (`tests/reveal.rs`, `tests/backlog.rs` and five other test files import them).
- **E2 — `backlog/mod.rs` split, a pure move.** It is 1,359 production lines and 3,677 test lines.
  - Production splits by concern beside the existing `detail/`, `filter`, `item_form`, `list`,
    `divergence` modules:
    - `mod.rs`: the tab, its state and a thin `impl Tab`;
    - selection and navigation: `go`, `step`, `rows`, `fold`, `reselect`, `jump`, the cursor
      format, `location`/`restore`/`reveal`;
    - queue toggles and their status lines;
    - item writes: `open_new`/`open_edit`, `on_item_*`, `apply`;
    - the key stacks and hints.
  - Tests move to `backlog/tests/` along the same lines, with the `Bench` and the probes in a shared
    module.
  - No behaviour change. The existing tests move with their code, unchanged and green. The
    architect fixes the file names.
- **E3 — Kit additions for group B (tests first).** Only what a group-B screen needs that the kit
  lacks. Expected:
  - a modal with a typed-confirmation or input row (Runs close-out `type FEAT-1 to close it`, the
    abort question);
  - the `?` box as a kit modal of help lines;
  - labels for the enums group B shows that `kit::label` does not cover yet. The architect lists
    them. `kit::label` already has run, step, gate, resolution, citation, isolation and setting
    source. Link and document kinds are not `htui-core` enums (no `enum LinkKind`/`DocumentKind`),
    so their words come from the data, not from a label table;
  - `plural` uses for `hit(s)`, `template(s)` and `1 options`.
  - Exhaustive matches, so a new variant fails to compile rather than print a raw name.
- **E4 — Backlog frame and list (MOD-82, MOD-81):**
  - the list is a kit pane drawn through `kit::list` (group rows, gutter, band, ellipsis, hidden
    note), statuses through `kit::status` (glyph + word; no `awaiting_approval`);
  - the filter form is drawn through `kit::form`, and the active filter through `kit::filter`;
  - the detail is a kit pane with the sub-tabs as chips in its top border, overflowing with `‹`/`›`
    (MOD-81: the strip no longer clips `Notes Prompt Reqs`);
  - **the detail header stops repeating itself** (MOD-82), in two steps:
    - B1 makes the detail pane's title `KEY · title` for every sub-tab (today the frame title is
      the key alone);
    - L-detail drops the repeated title and key lines from Body's head block. The meta line keeps
      the status (glyph + word), tags, priority and version.
    - The `FEAT` beside the key is `item.key_prefix` (`body.rs:122`), the key's own prefix
      (`key = key_prefix || '-' || key_number`, `htui-core` `item.rs:129`). Dropping it loses no
      datum;
  - the list's status column is sized for the labels, so titles keep their room at 60-80 columns
    (MOD-81).
  - `draws_chrome() == true`.
- **E5 — Skills is one list + detail pair with `Skills`/`Templates` as chips** (MOD-82):
  - the hand-drawn `Skills │ Templates` switch line goes;
  - the view switch is the list pane's chips (`skills.switch_view` as today; no key changes);
  - Library, Templates, attachments and agent help draw through `Pane`, `kit::list`, `kit::form`
    and `Modal`;
  - the pane titles drop their literal ` J/K scroll ` and name keys through hints (MOD-82 carried);
  - the attach activation field marks focus with the kit's gutter and band, not `selected`
    (MOD-85).
- **E6 — Overlays on `Modal`** (MOD-82, MOD-85):
  - concepts search, workspace switcher, migration prompt and queue draw through `Modal`, `kit::list`
    and `kit::empty`;
  - the cursor row is the kit band, not `accent` alone (MOD-85, legible under `NO_COLOR`);
  - concepts search keeps a margin and stops covering the header at 60 columns (MOD-82). The
    architect chooses between a cleared margin and a dimmed backdrop, one rule for every modal;
  - its scores and `hit(s)` read as words (MOD-83);
  - its errors name where to fix them, e.g. `Qdrant unreachable — Settings › Qdrant` (MOD-83).
- **E7 — The shell's own surfaces**:
  - the `?` box uses E3's modal;
  - the no-tab body is `kit::empty`;
  - the in-pane editor's frame is a kit pane whose title carries the editor's state;
  - the abort question is a `Modal`.
  - All four are coupled through `app__pane` snapshots, so they are one lane.
- **E8 — The transition goes (last task).** Deleted: `Tab::draws_chrome`, `paint_shell_hints`,
  `join_panes_under`, `hint_row`, and the list/detail branch of `paint_panes` with `paint_focus`.
  - The activity placeholder becomes a kit pane drawn with `Ctx::pane_focus`, so the shell restyles
    nothing.
  - `paint_bottom_hints` stays (used by `Pane::render`).
  - `Tab::key_hint` stays only if a caller survives (the architect checks; the about panel reads
    `browse_stack`, not `key_hint`).
  - `kit_chrome.rs` drops `GROUP_B` and its staleness test: all of `src` outside the kit and
    `OWN_BLOCK` must draw through the kit.
- **E9 — MOD-85 accents in group B** are decided per use and pinned by buffer tests:
  - Documents kind column (`detail/documents.rs`);
  - flow edge labels (`execution_graph.rs` `edge`);
  - divergence Theirs/Mine (`divergence.rs`);
  - the overlay cursor rows (E6) and Skills' attach field (E5).
- **E10 — MOD-83 copy in group B**:
  - Runs: step ids such as `0.1/0` become words or step names; `gate soft · in none · budget
    inherit` becomes a readable line through `kit::label`;
  - statuses everywhere through `kit::status`/`kit::label`;
  - every error group B prints names a next step.

## Definition of "ported" (each lane's acceptance; M3's, applied to group B)
1. Every pane is a `kit::Pane` and every overlay a `kit::Modal`. No `Block::`/`Borders::` stays in
   the lane's production code, and a tab returns `draws_chrome() == true` until E8 deletes the
   flag.
   - **Lanes do not edit `tests/kit_chrome.rs`.** Every lane empties some of `GROUP_B`'s entries,
     and they sit on adjacent lines, so concurrent edits would conflict. The merging thread drops
     the lane's entries in the merge commit.
   - A lane's own check is that `every_m4_entry_still_frames_itself` fails for exactly its files
     and no other `kit_chrome` test fails.
2. Lists through `kit::list`, filters through `kit::filter`, forms through `kit::form`, empty states
   through `kit::empty` with a next step.
3. Every status shows a glyph and a human word. No internal value is printed: no snake_case enum,
   no `(s)`, no raw score, no step id as the only name.
4. Hints and help text name keys only through `Keys`, and forms resolve acts through
   `Ctx::pressed` (MD-A).
5. MOD-85 accent uses in the lane's files are decided and pinned (E9).
6. Widths (MOD-81): one 80-column snapshot per ported surface (Backlog list + detail, Skills,
   Templates, each overlay).
7. **Content parity.** Every datum a snapshot showed before the port is still on screen after it.
   The lane's verifier checks snapshot by snapshot, by diff. Chrome, labels and placement may
   change; information may not be lost.
8. The lane accepts only the snapshots it owns (file sets below). A snapshot changing outside them
   is a defect.
9. New tests go in the lane's own source files or in a new test file of its own (e.g.
   `tests/backlog_runs.rs`).
   - A wave-2 lane may edit an existing `tests/backlog.rs` test only where it asserts the lane's
     own sub-tab. That is a function-level split of one file, so the four lanes run in worktrees,
     and the merging thread resolves any textual conflict in that file (step 3.5: a shared file
     means isolation, not parallel on one tree).
10. No lane edits `keys/catalogue.rs`. A port needs no new action; if one does, the lane stops and
    reports it.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Ported tab | `ui/tabs/settings/mod.rs` `render` + `draws_chrome`; `ui/tabs/requirements/mod.rs` | `Pane::new(title).focus(ctx.pane_focus(id)).accent(ctx.accent()).hints(…)`; tab returns `draws_chrome() == true` |
| Kit parts | `ui/kit/{pane,list,form,filter,label,modal,empty}.rs` | builder plus `render(frame, area, theme)`; `Theme` roles only; labels resolved from `Keys` |
| Overlay on the kit | `ui/overlay/leave.rs`, `ui/overlay/waiting_list.rs` | `Modal` + `chips` + `status`/`list`/`empty` |
| Pure-move split | M3's `settings/agents.rs` → `settings/agents/{mod,table,form,flows}.rs` (`951b8495`) | one commit, tests moved with their code, nothing else changed |
| Neutral helper homes | M3 T0 (`wrapped` → `kit::text`, `key_line` → `kit::chips`, `modal_rest` → `kit::modal`) | serial, every importer updated, insta `--check` unchanged |
| Hints from `Keys` | `Hint::One`, `Hint::Glyphs`, `Ctx::pressed` (M2 RV1, M3 MD-A) | no key word written as a literal |
| Guard | `tests/kit_chrome.rs`, `tests/token_scan/mod.rs` | lanes shrink `GROUP_B`; E8 removes it |
| Tests | `tests/*.rs` through `testkit` `Harness`; in-crate `TestBackend` buffer tests | per-file `cargo insta test --test <file>`; full `--check` on merged trees |

## Tasks
Independence is decided by the file sets below, not by this prose. Parallel lanes run in their
own worktrees (`git worktree add -b mod-115-m4-<lane> target/wt/m4-<lane> <base>`). They merge
`--no-ff` into `hr/MOD-115` one at a time, with the gate run on each merged tree.

### T0: Neutral homes (serial, primary tree)
- **Action**: E1. Move the lent helpers and update every importer. Insta `--check` unchanged.
- **Files**: `ui/kit/*` (new homes), `ui/tabs/backlog/{list.rs,detail/mod.rs,mod.rs}`,
  `ui/tabs/requirements/{mod,detail}.rs`, `ui/tabs/skills/{agent_help,library,templates}.rs`,
  `ui/overlay/queue.rs`.
- **Validate**: `cargo build -p htui`;
  `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`;
  `cargo insta test --all-features --check` unchanged.

### T1: `backlog/mod.rs` split (serial, primary tree)
- **Action**: E2. Pure move, one commit (or one per new file), no behaviour change, insta `--check`
  unchanged.
- **Files**: `ui/tabs/backlog/mod.rs` → `ui/tabs/backlog/{mod.rs, <concern>.rs…, tests/…}`.
- **Validate**: as T0, plus the same test count before and after (`cargo test -p htui --all-features
  -- --list | wc -l`).

### T2: Kit additions (serial, primary tree)
- **Action**: E3, tests first (buffer tests, monochrome proofs, exhaustive label matches).
- **Files**: `ui/kit/{modal,label,…}.rs` (+ new kit modules the architect names).
- **Validate**: `cargo test -p htui kit:: -- --test-threads=1`; insta `--check` unchanged.

### Wave 1 (parallel worktree lanes after T2): L-skills, L-overlays, L-shell
- **L-skills** (E5, MD-A for Skills):
  - source: `ui/tabs/skills/*`;
  - tests: `tests/skills.rs`, `tests/templates.rs`;
  - snapshots: `skills__*`, `templates__*`.
- **L-overlays** (E6):
  - source: `ui/overlay/{concepts_search,workspace_switcher,migration_prompt,queue}.rs`;
  - tests: `tests/concepts_search.rs`, `tests/queue_overlay.rs`;
  - snapshots: `concepts_search__*`, `queue_overlay__*`, in-crate `htui__ui__overlay__queue__*`,
    `shell__switcher_empty`, `shell__switcher_open`, `shell__migration_prompt`;
  - it may not edit `tests/shell.rs` source.
- **L-shell** (E7):
  - source: `app/state.rs` (`render_help`, the no-tab body only), `ui/editor_pane.rs`,
    `ui/overlay/abort_confirm.rs`, `app/pane.rs` tests;
  - snapshots: in-crate `htui__app__pane__*`, `htui__ui__editor_pane__*`.
- **Validate, per lane**: `cargo build -p htui`; the lane's test files with `--all-features --
  --test-threads=1`; `cargo insta test --all-features --test <file>` for its files only;
  `cargo clippy -p htui --all-features -- -D warnings`; `rustfmt` on its own files only (never
  `cargo fmt --all` in a lane).
- **Postgres**: lanes run `env -u HTUI_TEST_DATABASE_URL`; the merged-tree gate runs Postgres once.

### B1: Backlog frame, list and filter (serial, primary tree, after wave 1 merges)
- **Action**: E4. Tests first (buffer tests for the header, the list rows and the band; an 80-column
  snapshot). The list and detail become kit panes, with sub-tab chips and the de-duplicated header.
  The list, filter form and filter band go through the kit, and Backlog flips `draws_chrome`.
  - Re-accept every `backlog__*` snapshot and the collateral set, after a per-snapshot
    content-parity review. The frame, header and list half change; the sub-tab bodies do not.
  - This is the one task that edits `tests/backlog.rs`.
- **Files**: `ui/tabs/backlog/{mod.rs,list.rs,filter.rs,detail/mod.rs}` (+ the T1 split modules
  that render), `tests/backlog.rs`, all `backlog__*.snap`, collateral `shell__{after_switch,
  offline_label,switcher_*,migration_prompt}`, `concepts_search__*`, `waiting__*`, `integration__*`,
  `prompt_preview__*`, `replay__*`, in-crate `htui__testkit__*`.
- **Validate**: full `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`; insta
  reviewed per snapshot.

### Wave 2 (parallel worktree lanes after B1): Backlog's bodies
Each lane owns its sub-tab's snapshots. The list half is final after B1, so only the detail half
moves.
- **L-runs** (E9 edges, E10 step ids and the gate line, close-out typed confirm on E3's modal):
  - source: `detail/runs.rs`, `detail/runs/execution_graph.rs`;
  - snapshots: `backlog__{detail_runs,empty_runs,runs_*}`, `replay__*`.
- **L-docs** (E9 Documents kind column):
  - source: `detail/{documents,notes,compose}.rs`;
  - snapshots: `backlog__{detail_documents,document_form,empty_documents,detail_notes,note_compose,
    empty_notes}`.
- **L-forms** (E9 Theirs/Mine, MD-A for the item form):
  - source: `item_form.rs`, `divergence.rs`;
  - snapshots: `backlog__{item_form_new,item_form_edit,item_divergence}`.
- **L-detail** (Body's markdown keeps `ui/markdown.rs` as is):
  - source: `detail/{body,graph,prompt,requirements}.rs`;
  - snapshots: `backlog__{detail_body,empty_body,detail_graph,graph_*,empty_graph,detail_prompt,
    empty_prompt,detail_reqs*,empty_reqs}`, `prompt_preview__*`, plus the collateral that shows
    Body (`shell__*` but `about_panel_on`, `concepts_search__*`, `waiting__*`, `integration__*`,
    in-crate `htui__testkit__*`).
- **Validate, per lane**: as wave 1, with `--test backlog` and the lane's own snapshots. Insta
  accepts only the lane's names. A changed snapshot outside the set fails the lane.

### T3: Delete the transition, guard, close-out (serial, last)
- **Action**:
  - E8;
  - empty and remove `GROUP_B`, and make the guard cover all of `src`;
  - run the full Validation block;
  - bookkeeping: the PRD row 4 `complete`, the decisions record's M4 section, and the item
    close-out (lifecycle P2). MOD-115's write-up folds `mod-115-decisions.md` in, and MOD-81, 82,
    83 and 85 close with it, each line naming what of it landed and what it carried.
- **Files**: `app/state.rs` (`render`, `paint_panes`, the adapter and its tests),
  `ui/tabs/registry.rs`, every `draws_chrome` impl (settings, requirements, chat, backlog, skills),
  `ui/kit/pane.rs` (`paint_focus` if it ends unused), `tests/kit_chrome.rs`, docs.
- **Validate**: the Validation block.

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
cargo test --workspace --all-features -- --test-threads=1 --no-fail-fast   # "zeta" guard, keys_file, Postgres suites
cargo insta test --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
After each workflow: `pgrep -af 'htui worker'`, `ls ~/.config/htui`; `rmdir ~/.config/htui/trees`
if the known empty dir appeared.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A port drops information while changing layout | High | acceptance 7, checked by each lane's verifier by snapshot diff; B1's 70-odd re-accepts are reviewed one by one |
| B1 is the bottleneck and the widest diff (every Backlog snapshot) | High | B1 does frame + list + filter only; bodies wait for wave 2; collateral named up front |
| The `backlog/mod.rs` split changes behaviour | Medium | T1 is a pure move with unchanged tests and an equal test count, before any port |
| Two wave-2 lanes touch one collateral snapshot | Medium | collateral is owned by one lane (L-detail); any other change is a defect |
| Kit API proves wrong mid-wave | Medium | T2 lands before any lane; a kit fix found in a lane is a serial fix merged before the next wave |
| The header de-dup drops a datum | Low | the dropped `FEAT` is `item.key_prefix`, a substring of the key (fact-checked); the verifier diffs every Body snapshot |
| Worktree builds contend for CPU and disk | Medium | three lanes per wave on 12 cores and 60 GB; `target/wt/` on `/dev/sda` (2.8T free) |
| A usage limit kills a workflow mid-lane | Medium | incremental commits; resume with `args.skipImpl` (M2 lesson); a dead verifier is not "verified" (memory) |
| The Runs lane is the largest (2.9k production lines) | Medium | it owns its own snapshots and new test file; no other lane touches `runs*` |

## Open points for CONFIRM
1. **Order:** T0 → T1 → T2 → wave 1 (Skills, overlays, shell surfaces) → B1 (Backlog frame + list,
   alone) → wave 2 (Runs, Docs, forms, detail) → T3 (deletions, close-out).
2. **E2, the split:** by concern (navigation, queue, writes, keys), tests into `backlog/tests/`, as a
   pure move. `detail/runs.rs` (6.7k lines with tests) is **not** split in M4; MOD-119 redesigns
   Runs.
3. **E4, the detail header:**
   - the pane title is `KEY · title`;
   - Body keeps one meta line: status, tags, priority, version;
   - the repeated `KEY  PREFIX` line goes (the prefix is the key's own).
4. **MOD-81's 160-column list cap:** carry it to MOD-119, which owns Backlog's layout default
   (M3 D6 already left the 24% IDE default to it), and say so in MOD-81's close-out. The
   alternative is a cap here, which needs a `ListSize` maximum in the layout engine (today it has
   only `Percent`/`Cells`).
5. **E6, overlay margin:** one rule for every modal; the architect picks a cleared margin or a
   dimmed backdrop, and the choice is reported at the review gate.
6. **Ultracode:** one workflow per task or lane (implement → adversarial verify with the
   content-parity lens), architect and reviewer as plain agents (as M3).
7. **Closing:** M4 closes MOD-115, and with it MOD-81, 82, 83 and 85. `R-TUI-18` stays with MOD-116
   and MOD-125, as recorded in M2 D6.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| 18 files still frame themselves outside group A and the kit | true | `tests/kit_chrome.rs` `GROUP_B` (18 entries); `git grep -c '(Block\|Borders)::'` over group B: same 18 files |
| Requirements, Skills and the queue overlay import from Backlog | true | `requirements/mod.rs:55-56`, `requirements/detail.rs:21`, `skills/{agent_help.rs:33,library.rs:51,templates.rs:37}`, `overlay/queue.rs:27` (`queue_sentence`); no other `src` importer |
| `BacklogTab` / `not_in_this_backlog` are public API for tests | true | `tests/{backlog,chat,prompt_preview,replay}.rs`, `tests/backlog_keys/{l_b,l_c}.rs`; `tests/reveal.rs:15` |
| `backlog/mod.rs` is 1,359 production + 3,677 test lines | true | `mod tests` at line 1359 of 5,036 |
| Every `backlog__*` snapshot draws the list and the detail | true | 41 `backlog__*.snap`; first rows hold `┌ Backlog (n)` beside the detail frame |
| Collateral snapshots draw Backlog under them | true | `shell__` 5 of 6 (not `about_panel_on`), `concepts_search__` 5, `waiting__` 2, `integration__` 1, `prompt_preview__` 2, `replay__` 1, in-crate `htui__testkit__tests__shell_empty` |
| Wave-2 snapshot ownership covers all 41 `backlog__*` exactly once (with B1's six list/filter ones) | true | runs 12 + docs 6 + forms 3 + detail 14 + B1-only 6 (`filter_form`, `filter_no_match`, `filtered_list`, `list_folded`, `list_grouped`, `list_last_row`) = 41; the draft said 13 for Runs, **amended** |
| The `?` box, the abort question and the editor frame share snapshots | true | in-crate `htui__app__pane__tests__{the_abort_question,the_help_box_on_the_asking_tab,…}` draw the editor pane with the modal; hence one lane (L-shell) |
| `src/app/pane.rs` exists (its tests own `app__pane` snapshots) | true | `ls src/app/pane.rs` |
| The detail header's `FEAT` is the key's own prefix | **true, amended E4** | `detail/body.rs:122` renders `item.key_prefix`; `htui-core` `item.rs:129` `key = key_prefix \|\| '-' \|\| key_number`. The draft kept a "kind when it differs" branch: moot, removed |
| Lanes removing `GROUP_B` entries would collide | **true, amended** | entries are adjacent, alphabetical (shell, overlay, backlog, skills blocks touch); rule 1: the merging thread drops entries |
| Literal key-naming hints remain in group B (MD-A scope) | true | `skills/library.rs:148,173`, `skills/templates.rs:106`, `skills/agent_help.rs:87`, `overlay/concepts_search.rs:71,92,97`; Backlog's are `Esc …` (MD7) or in tests (`runs.rs:5709` is under `mod tests` at 2170) |
| `kit::label` lacks only a few group-B enums | true | `kit/label.rs` has `run_status`, `step_status`, `gate`, `resolution`, `citation`, `isolation`, `setting_source`, `plural`; no `enum LinkKind`/`DocumentKind` in `htui-core` |
| `paint_bottom_hints` survives E8 | true | `kit/pane.rs:255` `Pane::render` calls it |
| The about panel needs no `key_hint` | true | `app/state.rs:1617` reads `tab.browse_stack()`; `key_hint`'s callers are the adapter (`:1674`) and the activity hints (`:803`), both rewritten by E8 |
| `ListSize` has no maximum (MOD-81 cap needs one) | true | `ui/shell/mod.rs:77` `enum ListSize { Percent(u16), Cells(u16) }` |
| Resources for three concurrent worktree lanes | true | `df -h .`: 2.8T free on `/dev/sda`; `nproc` 12; 60 GB RAM |
| Wave-1 lanes are file-disjoint | true | L-skills `skills/*` + `tests/{skills,templates}.rs`; L-overlays four overlay files + `tests/{concepts_search,queue_overlay}.rs`; L-shell `app/state.rs`, `app/pane.rs`, `ui/editor_pane.rs`, `overlay/abort_confirm.rs`: empty pairwise intersections; snapshot sets disjoint (`shell__` split: switcher/migration to L-overlays only) |
| Wave-2 lanes are file-disjoint in source | true | `runs.rs`+`runs/` / `documents,notes,compose` / `item_form,divergence` / `body,graph,prompt,requirements`: empty intersections. Shared: `tests/backlog.rs` (function-level, rule 9: worktrees, merge resolves) |
| T0, T1, T2 must be serial | true | T0 and T1 both edit `backlog/mod.rs` imports; T2's kit API is what every lane builds on |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Nothing outside `ui/kit` frames itself (guard, `GROUP_B` gone)
- [ ] The M1 adapter and `draws_chrome` deleted
- [ ] No internal enum name, `(s)` or bare step id on a group-B screen
- [ ] Every group-B hint names keys through `Keys`
- [ ] Content parity reviewed per snapshot
- [ ] MOD-81, 82, 83, 85 and MOD-115 closed with their write-ups

**Status**: CONFIRMED by the maintainer 2026-10-27 as written: all seven open points as proposed
(order; no `runs.rs` split; the `KEY · title` header; the 160-column cap carried to MOD-119; one
modal-margin rule picked by the architect; ultracode per task or lane; M4 closes MOD-115).
