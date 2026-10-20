# Plan: MOD-115 M3 — tabs ported to the kit, group A (Settings, Requirements, Chat)

**Source PRD**: `.claude/prds/mod-115-tui-shell.prd.md`
**Selected Milestone**: 3 — Tabs ported, group A
**Complexity**: Large (kit completion, a shell seam, ten Settings sections, one 3k-line file split,
~91 snapshots rewritten)

## Summary
M1 built the kit and M2 the layouts, but **no tab draws with the kit yet**. The kit is used only by
the about panel, the activity placeholder and the leave and waiting overlays. Tabs still draw their
own frames, and the shell paints hints onto them through the M1 adapter (`paint_shell_hints`,
`join_panes_under`). M3:
- completes the kit (form rows with inline help, the filter band, sub-tab chips that overflow
  visibly, display labels for every shown enum);
- adds a seam so a ported tab draws its own panes and the shell stops painting over them;
- ports **group A**: Settings (all ten sections, `settings/agents.rs` split along kit lines),
  Requirements and Chat.

M4 ports the rest (Backlog with the `backlog/mod.rs` split, Skills, the overlays, the path picker
and in-pane editor, the `?` box), deletes the adapter and closes MOD-81/82/83/85.

"Ported" means chrome and presentation only. Each tab's redesigned *content* stays with its own
item: Settings' arrow-changed values and new sections (MOD-123), Chat's groups and `a`/`A`/`r`
(MOD-121), Requirements moving into Workspaces (MOD-118).

## Grouping decision (PRD: "`/plan` decides after counting each tab's size and snapshots")
| Tab | Production lines | Self-drawn parts | Snapshots (own + collateral) | Group |
|---|---|---|---|---|
| Settings (11 files) | 15,984 | ~53, plus ~20 literal `Enter …` hints | 68 | **A (M3)** |
| Requirements (4) | 2,643 | ~15 | 7 | **A (M3)** |
| Chat (4) | 1,823 | ~9 | 15 + 1 (Runs permission strip) | **A (M3)** |
| Backlog (15) | 11,401 | ~52 | 44 + ~15 (shell/overlay snapshots draw over it) | B (M4) |
| Skills (5) | 5,859 | ~31 | 17 | B (M4) |
| Overlays, path picker, editor, `?` box | 3,595 | 6 modals, 5 lists | 9 tests + 14 in-crate | B (M4); the path picker moves with Hierarchy (A) |

Totals: M3 ≈ 20.5k production lines and ~91 snapshots; M4 ≈ 20.9k and ~85.

Why this split:
- **Backlog goes to M4 with its split.** Porting it before the split would rewrite its 44
  snapshots twice. Every shell and overlay snapshot draws over Backlog, so it is the one port that
  cannot share a wave with anything.
- **Settings goes to M3.** Its ten sections are separate files with separate test files, so it
  fans out well. Its `mod.rs` helpers are what Backlog, Requirements and Skills import (T0 moves
  them out first).
- **The `agents.rs` split moves from M4 to M3, with Settings.** That amends the PRD milestone row
  (CONFIRM point 1).
- **Requirements and Chat go to M3.** They are small, and none of their snapshots draw another tab.

## Maintainer calls carried from M2 (asked at CONFIRM)
- **D-15a, shift-arrow order.** Shift arrows act, but are listed last (`l`, `]`, `shift-right`),
  so hints read `h`/`l`. Keep it, or list `shift-→` first as the findings show?
- **D-15b, tab keys in forms.** `MODAL` admits no tab keys, so a form never switches tabs
  mid-edit (except the Skills modes that pass them to the shell). Keep it?
- **RV4 deferral.** The literal `Enter …` hints outside RV1's sites are part of this port for group
  A (MD-A below) and of M4 for group B. Confirming the plan confirms the deferral.

## Design decisions (proposed; CONFIRM accepts or amends them)
- **D1 — Ported tabs draw their own panes; the shell stops painting over them.**
  - `Tab::draws_chrome(&self) -> bool`, default `false`. For a tab that returns `true`, the shell:
    - skips `paint_panes` (focus restyle) and the hint adapter for that tab;
    - still paints the hidden-pane note, which `paint_bottom_note` already writes onto any rounded
      border.
  - `Ctx` gains `pane_focus(PaneId) -> Focus` and `accent() -> Style` (the tab's accent from
    `Theme::tab_accent`). A ported tab draws each pane as
    `Pane::new(title).focus(ctx.pane_focus(id)).accent(ctx.accent()).hints(…)`.
  - The hints a pane carries are the ones `key_hint` returns today, for the focused pane.
  - M4's last task deletes the flag, the adapter (`paint_shell_hints`, `join_panes_under`,
    `hint_row`) and, if unused by then, `paint_focus` and `paint_bottom_hints`.
- **D2 — Neutral homes for the cross-tab helpers.**
  - `settings::{wrapped, key_line, modal_rest}` are imported by Backlog, Requirements, Skills and
    Chat. They move under `ui/kit/`: `wrapped` to a text module, `key_line` beside the chips,
    `modal_rest` beside the modal. The architect fixes the exact homes.
  - Every importer is updated in T0, which runs alone, so no lane ever shares a `use` line.
  - `backlog::list::window` and `backlog::detail::{Scroll, Step, capture_area, STAMP}` stay where
    they are in M3. Requirements may keep importing them, but its lane does not edit them. M4
    re-homes them with the Backlog split.
- **D3 — Kit additions.**
  - `kit::form`: a form row. It has a shared label column, a value kind and the focus marker
    (gutter `▌` and band, as in `kit::list`). The inline help line sits under the focused row, and
    an error line under the row it concerns. This fixes audit item 3 (the input ~20 rows below the
    field).
    - Value kinds: text, toggle `● on` / `○ off`, choice `‹ value ›`, secret `•••••••• stored`,
      read-only.
    - Rendering only: the arrow-changes-value behaviour is MOD-123.
  - `kit::filter`: the filter band (`/ query`, a capturing marker, `n of m`) over a list. It
    replaces Requirements' accent-coloured `" /"` row; Backlog's filter form adopts it in M4.
  - `Pane` sub-tab chips (MOD-81, MOD-82). When the chips do not fit, the strip keeps the active
    chip visible and shows `‹`/`›` for the ones cut off, instead of dropping chips from the right
    (today's `sub_tab_rects`). Settings' ten sections need this at 80 columns.
  - `kit::label` (MOD-83):
    - a display label, plus a glyph where it is a status, for every enum group A shows: run, step
      and plan-entry status, stop reason, tool kind, agent transport and billing, setting source,
      requirement status, kind and priority;
    - one pluralising helper (`1 option`, `3 options`; no `(s)`).
    - `kit::status` keeps the item statuses.
    - Backlog-only enums (link kinds, gate) are added by M4.
- **D4 — Settings becomes one pane with its sections as sub-tab chips.**
  - The tab-name box (`┌ Settings ─`) and the in-body section strip go. The pane's title is the
    section's name, and the ten sections are chips in its top border (D3 overflow).
  - Sections draw inside the pane's inner rect and give it their hints.
  - Notice rows and the shared `message()` empty helper become `kit::empty` / a kit notice line.
  - Settings stays `Panes::Single` with no mouse capture (MD8). Turning it into list + detail is
    MOD-123's call.
- **D5 — Chat is one pane titled by the session**, with the transcript scrollbar on its border and
  the composer as the pane's bottom region.
  - The tab-name box goes.
  - Plan statuses and stop reasons show their labels (`end_turn` → `turn ended`, D3).
  - The permission strip's chrome moves to the kit. Its keys stay digits: `a`/`A`/`r` is MOD-121.
  - The Runs pane draws the same strip, so one Backlog snapshot
    (`backlog__runs_pending_permission`) changes in this lane.
- **D6 — Requirements**:
  - the tree and the detail draw as kit panes, the tree through `kit::list` with its area groups;
  - the filter through `kit::filter`;
  - the forms through `kit::form`;
  - statuses, kinds and priorities through `kit::label`.
  - Its split defaults stay as M2 left them (45/55); the 24% IDE default is MOD-119's for Backlog,
    and Requirements moves into Workspaces with MOD-118.
- **D7 — `settings/agents.rs` is split along kit lines.** It becomes `settings/agents/` with the
  section and its keys (`mod.rs`), the table, the editor form and the paste/probe flows. No
  behaviour change. The tests move with the code they cover.
- **MD-A — D-13 closes per ported screen.** On every group-A screen:
  - a form resolves its acts from `Ctx::pressed` (as RV1 did for the filter, item and Requirements
    forms);
  - every hint naming a key comes from `Keys` (`Hint::One`, `Hint::Glyphs`), never a literal
    (`Enter sends · Esc cancels`, `HINT_PASTING`, `CHANGED_ELSEWHERE`'s "Enter retries").
  - The per-screen D-13 note in the decisions record shrinks to group B.

## Definition of "ported" (each lane's acceptance)
1. Every pane is a `kit::Pane`: title, sub-tab chips, hints on the focused pane's bottom border,
   scrollbar where content scrolls. No `Block::`/`Borders::` stays in the lane's production code,
   and the tab returns `draws_chrome() == true`.
2. Lists and selection go through `kit::list` (gutter, band, ellipsis, group rows, hidden note),
   filters through `kit::filter`, forms through `kit::form`, and empty states through `kit::empty`
   with a next step (audit item 2).
3. Every status shows a glyph and a human word, and no internal value is printed: no snake_case
   enum, no `(app_setting_default)`, no `(s)`. MOD-83's examples in the lane's files are covered.
4. Hints and help text name keys only through `Keys`, and forms resolve their acts through
   `Ctx::pressed` (MD-A).
5. The lane decides the MOD-85 accent uses in its files: Chat headers, transcript `you` and notes,
   the permission digits, the Requirements form `[value]`. Each is pinned by a buffer test.
6. Widths (MOD-81): each ported tab, and each Settings section, gets one 80-column snapshot.
   - Agents' name column keeps a minimum width.
   - The strip shows `‹`/`›`.
   - The Prompt section wraps instead of clipping its right edge.
7. **Content parity.** Every datum a snapshot showed before the port is still on screen after it.
   The lane's verifier checks this snapshot by snapshot. Chrome, labels and placement may change;
   information may not be lost.
8. The lane accepts only its own test files' snapshots (`cargo insta test --test <file>`). A
   snapshot changing outside them is a defect, except the one named in D5.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| No file under `ui/tabs/` uses the kit | true | `search_text "kit::" path:crates/htui/src/ui/tabs`: 4 hits, all `testkit::` |
| The kit has no form row, filter band or display labels beyond item statuses | true | `ui/kit/mod.rs`: chips, empty, list, modal, pane, status; `status.rs:19` `TABLE` covers `htui_core::model::Status` only |
| `Pane` drops sub-tab chips that do not fit, from the right, unmarked | true | `kit/pane.rs` `sub_tab_rects` doc: "A chip that does not fit … is dropped whole, from the right" |
| Settings sections draw no frame of their own; the tab draws one around them | true | `Block::` in `ui/tabs/settings/`: only `settings/mod.rs:488` (`" Settings "`) |
| The shell paints focus and hints over tab-drawn borders | true | `app/state.rs` `App::render`: `paint_panes` after the tab's render, then `paint_shell_hints` from `tab.key_hint` (comment: "Deleted as tabs move to `Pane::hints`") |
| `settings::{wrapped, key_line, modal_rest}` are imported across tabs | true | `requirements/mod.rs:53`, `requirements/detail.rs:16`, `chat/mod.rs:45`, `backlog/mod.rs:62`, `skills/agent_help.rs:33`, `skills/attach.rs:43`, detail sub-tabs |
| Chat's permission strip is also drawn by Runs | true | `backlog/detail/runs.rs:86` `use crate::ui::tabs::chat::permission::PermissionStrip;` |
| `tests/settings.rs` snapshots are all Agents; each other section has its own test file | true | `settings__agents_*` (10) + `probe__agents_probed_missing`; `box_settings`, `connection`, `hierarchy`, `kinds`, `personas`, `prompt_settings`, `queue_settings`, `secrets_settings` |
| Qdrant has no snapshot test | true | no `qdrant__*.snap` under `tests/snapshots/`; its lane adds one |
| The path picker is used only by Hierarchy and snapshotted by it | true | `path_picker.rs` imported only by `settings/hierarchy.rs:38`; `hierarchy__picker.snap` |
| No group-A snapshot draws another tab | true | `shell.rs`, `integration.rs`, `concepts_search.rs`, `waiting.rs`, `testkit` `shell_empty` all draw over Backlog |
| Requirements and Chat by the shell's panes: Requirements list + detail, Chat single | true | `requirements/mod.rs:1382` `Panes::ListDetail`; Chat keeps the `Panes::Single` default (`registry.rs:139`) |
| One-crate lanes cannot share a tree (a half-edited lane breaks the other's build) | true (repo lesson) | memory `parallel-fanout-hidden-file-coupling`: "the parallel unit is a crate that compiles"; MOD-69/MOD-70 ran concurrent lanes in `target/wt/<t>` worktrees |
| Disk for 4 concurrent worktree targets (~10 GB each) | true | `df -h .`: 3.0T free on `/dev/sda` (the repo disk); worktrees under the gitignored `target/wt/` stay on it |
| `agents.rs` → `agents/mod.rs` needs no edit to `settings/mod.rs` | true | `settings/mod.rs:14` `pub mod agents;` resolves either file layout (Rust module rules) |
| `Hint::One` / `Hint::Glyphs` and `Theme::tab_accent` exist to build on | true | `keys/hint.rs:28,35`; `app/state.rs` `paint_panes` calls `self.theme.tab_accent(tab.id().0)` |
| The absorbed accent item is MOD-85 (not MOD-80, which is done) | **false in the draft**, amended | `HANDOFF.md` summary table "MOD-85 remaining accent uses"; the draft said MOD-80 in three places |
| Lane file sets are pairwise disjoint | true | intersection below |

**File sets (source + snapshots):**
- T0 `{ui/tabs/registry.rs, app/state.rs, ui/kit/mod.rs, ui/kit/{text,form,filter,label}.rs
  (new or stub), ui/kit/chips.rs, ui/kit/modal.rs, settings/mod.rs (helpers out), every importer
  of the three helpers}`. Serial and alone; no snapshot may change.
- T1 `{ui/kit/form.rs, ui/kit/filter.rs, ui/kit/pane.rs}`. T2 `{ui/kit/label.rs, ui/kit/status.rs}`.
  T1 and T2 are disjoint but in one crate: serial on the primary tree (both small).
- T3 `{settings/mod.rs, tests/snapshots/{settings,probe,box_settings,connection,hierarchy,kinds,
  personas,prompt_settings,queue_settings,secrets_settings}__*}`. It changes every Settings
  snapshot (frame and chips), so it is serial before the section lanes.
- Wave lanes, after T3:
  - L-req `{requirements/*, tests/requirements.rs, requirements__*}`
  - L-chat `{chat/*, tests/chat.rs, chat__*, backlog__runs_pending_permission.snap}`
  - L-agents `{settings/agents.rs → settings/agents/*, tests/settings.rs, tests/probe.rs,
    settings__*, probe__*}`
  - L-kinds `{settings/kinds.rs, settings/personas.rs, tests/kinds.rs, tests/personas.rs, kinds__*,
    personas__*}`
  - L-hier `{settings/hierarchy.rs, ui/path_picker.rs, settings/boxes.rs, tests/hierarchy.rs,
    tests/box_settings.rs, hierarchy__*, box_settings__*}`
  - L-sec `{settings/secrets.rs, settings/connection.rs, settings/qdrant.rs,
    tests/secrets_settings.rs, tests/connection.rs, a new qdrant snapshot test, secrets_settings__*,
    connection__*, qdrant__*}`
  - L-prompt `{settings/prompt.rs, settings/queue.rs, tests/prompt_settings.rs,
    tests/queue_settings.rs, prompt_settings__*, queue_settings__*}`
- Pairwise intersections across the wave lanes are empty. `settings/mod.rs` (the section list, `mod`
  lines) is touched by L-agents only if the split needs a `mod agents;` change. A file-to-directory
  module (`agents.rs` → `agents/mod.rs`) keeps the parent's `mod agents;` line, so no edit is
  needed.
- **No lane edits `keys/catalogue.rs`.** A port needs no new action; if one turns out to, the lane
  stops and reports it, because every snapshot's `?` box and `docs/htui-keys.md` couple to it.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Kit parts | `ui/kit/pane.rs` `Pane` builder, `ui/kit/list.rs` `render`, `ui/kit/empty.rs` `NextStep` | builder plus `render(frame, area, theme)`; styles from `Theme` roles only; labels arrive resolved from `Keys` |
| Kit tests | `ui/kit/pane.rs` tests (`bottom_hints_overwrite_only_horizontal_rules`) | `TestBackend` read back as strings; sentence-named; a monochrome proof per style |
| Overlay already on the kit | `ui/overlay/leave.rs`, `ui/overlay/waiting_list.rs` | `Modal` + `chips` + `status`/`list`/`empty` |
| Shell seam | `app/state.rs` `Ctx::split`, `with_tab_view`, `focus()` | the shell sets context state; tabs read it; benches keep defaults |
| Hints from `Keys` | `Hint::One`, `Hint::Glyphs` (M2 RV1) and `Ctx::pressed` in the filter/item/Requirements forms | key words never literal; acts resolved from the chord pressed |
| Module split | M2's `ui/shell/` creation (stubs in T0, filled by tasks) | file-to-directory module keeps the parent `mod` line |
| Snapshots | `tests/*.rs` through `testkit` `Harness`/`SectionBench`; insta | per-file `cargo insta test --test <file>`; full `--check` at the end |

## Tasks
Independence is decided by the file sets above, not by this prose. Lanes run in their own
worktrees (`git worktree add -b mod-115-m3-<lane> target/wt/m3-<lane> <base>`) and merge `--no-ff`
into `hr/MOD-115` one at a time, with the gate run on the merged tree.

### Task 0: Seam and neutral helpers (serial, primary tree)
- **Action**: tests first for the seam:
  - a `draws_chrome` tab's borders are not repainted and its bottom border carries no adapter hint;
  - the hidden-pane note is still painted;
  - `Ctx::pane_focus` and `Ctx::accent` work on a bench and under the shell.
  - Move the three helpers (D2) and update every importer. Declare the new kit modules as stubs.
- **Validate**: `cargo build -p htui`; `cargo test -p htui --all-features -- --test-threads=1
  --no-fail-fast`; `cargo insta test --all-features --check` **unchanged** (no snapshot may move)

### Task 1: Kit form rows, filter band, chip overflow (serial, primary tree)
- **Action**: tests first. `kit::form`: each value kind, the focus marker, help under the focused
  row, error line, label column clipping with `…`, monochrome. `kit::filter`: the capturing and
  idle states, count. `Pane` chips: the active chip always visible, `‹`/`›` marks, and
  `sub_tab_rects` agrees with what is drawn (clicks still hit).
- **Validate**: `cargo test -p htui kit:: -- --test-threads=1`; insta `--check` unchanged except
  snapshots whose panes already use chips (none expected; any change is reviewed)

### Task 2: Display labels (serial, primary tree)
- **Action**: tests first: a label (and glyph where it is a status) for every enum D3 lists, no
  label equals its snake_case value, the plural helper. Exhaustive `match`es, so a new variant
  fails to compile rather than printing its raw name.
- **Validate**: `cargo test -p htui kit:: -- --test-threads=1`

### Task 3: Settings frame (serial, primary tree)
- **Action**: D4. The Settings tab draws one kit pane with section chips, takes its hints from the
  active section, draws notices and the empty helper through the kit, and converts
  `CHANGED_ELSEWHERE` to hints from `Keys`. Re-accept every Settings snapshot after a per-section
  review: the frame changes, the section bodies do not.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`; each
  Settings test file's insta run reviewed

### Wave A (parallel worktree lanes after T3): L-req, L-chat, L-agents, L-kinds
### Wave B (parallel worktree lanes after wave A merges): L-hier, L-sec, L-prompt
- **Action, per lane**: tests first, then the "ported" definition above for the lane's files.
  - L-agents also does D7's split, as its first commit and with no behaviour change, before the
    port.
  - L-sec adds the Qdrant snapshot test.
  - Each lane commits incrementally and stages by path, with the lane name in every commit subject.
- **Validate, per lane**: `cargo build -p htui`; the lane's test files with
  `--all-features -- --test-threads=1`; `cargo insta test --all-features --test <file>` for its
  files only; `cargo clippy -p htui --all-features -- -D warnings`; `rustfmt` on its own files only
  (never `cargo fmt --all` in a lane)
- **Postgres**: lanes run `env -u HTUI_TEST_DATABASE_URL` (Postgres suites skip); the merged-tree
  gate runs them once.

### Task 4: Merged-tree gate, guard, records (serial, last)
- **Action**:
  - Add `tests/kit_chrome.rs`. It fails on `Block::`/`Borders::` outside `#[cfg(test)]` in
    `ui/tabs/{settings,requirements,chat}` and `ui/path_picker.rs`, and lists the group-B files M4
    removes from its allow-list.
  - Run the full Validation block.
  - Update the decisions record with an M3 section (D1–D7, MD-A, D-13's remaining scope), the PRD
    milestone rows (3 complete; 4's text, minus the agents split) and the HANDOFF phase note.
- **Validate**: the Validation block below

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
| A port silently drops information while changing layout | High | acceptance 7 (content parity), checked by the lane's verifier snapshot by snapshot, before accepting |
| Snapshot churn hides a regression | High | T3 changes only the frame, lanes only their own files; full `--check` on the merged tree; per-lane review |
| A lane needs a new key row | Low | forbidden in lanes; stop and report; it becomes a serial step |
| Kit API proves wrong for a section mid-wave | Medium | T1/T2 land before any lane; a kit change found in a lane is a serial fix merged before the next wave, never edited in two lanes |
| `agents.rs` split breaks behaviour | Medium | split first, alone, with the existing tests unchanged and green before the port starts |
| Seven worktree builds contend for CPU and disk | Medium | waves of 4 then 3; `target/wt/` on the repo disk (3.0T free) |
| Porting Settings/Chat now and redesigning them in MOD-123/MOD-121 doubles the work | Medium | the port uses the kit parts those items need (form rows with help, chips, labels), so their work builds on it rather than replacing it |
| The weekly usage limit kills a workflow mid-lane | Medium | incremental commits; recovery with `args.skipImpl` (M2 lesson) |

## Open points for CONFIRM
1. Grouping: M3 = Settings (with the `agents.rs` split pulled forward from M4), Requirements and Chat;
   M4 = Backlog (with its split), Skills, the overlays, the editor and `?` box, the adapter deleted,
   MOD-81/82/83/85 closed.
2. D1: a `draws_chrome` flag during the transition, deleted in M4.
3. D4: Settings stays a single pane with section chips (the list + detail question is MOD-123's).
4. D5: Chat's permission keys stay digits until MOD-121.
5. D-15a (shift-arrow order), D-15b (no tab keys in forms), and the RV4 deferral (MD-A covers group A).
6. Ultracode: one workflow per task or lane (the M2 generic script), with the architect and reviewer
   as plain agents.

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] Group A tabs draw only through the kit (guard test)
- [ ] No internal enum name on a group-A screen
- [ ] Every group-A hint names keys through `Keys`
- [ ] Content parity reviewed per snapshot

**Status**: CONFIRMED by the maintainer 2026-10-20 as written: all six open points as proposed.
That means D-15a keeps the shift arrows listed last, D-15b keeps tab keys out of forms, and the
RV4 deferral of the literal `Enter …` hints is confirmed (group A in M3, group B in M4).
