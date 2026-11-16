# Plan: MOD-116 M1 — The Workspaces tab, states and opening

**Item**: MOD-116 — Workspaces tab and workspace lifecycle
**Source PRD**: `.claude/prds/mod-116-workspaces-tab.prd.md`
**Selected Milestone**: 1 — The tab, states and opening
**Routing**: routed as PRD by `/handoff-run` (C2, C4); PRD gate answered (Q1-Q4, 2026-11-15). Sandbox run
`hr/MOD-116`, plan written at `9b135537`.
**Requirements**: `R-TUI-14`, `R-TUI-1` (header), `R-ENT-2`, `R-ORCH-16` (the states read MOD-125's stop record)
**Complexity**: Medium-large (one pure derivation and one store read on five `WriteStore` implementors, one
conformance case and one `.sqlx` entry; two `StoreRequest`/`StoreReply` pairs; a new list + detail tab with its key
context; the switcher overlay deleted; every tab digit moves by one, 46 snapshot files move and 3 are deleted
(fact-check: the drafter's 49-line list counts the 3 deleted switcher files))
**Status**: COMPLETE (2026-11-15; T1-T4 `01cdb979`..`db97562a`, review fix round R1 `dc559952`..`f8037cd8`). Fact-checked at `9b135537` (see "Verified claims")

## Summary

Today one workspace is on screen and the others are invisible: `w` opens a modal switcher that only re-scopes,
startup always enters the first workspace by name, and nothing says that another workspace's runs are still
walking (they are: the executor is box-scoped and `set_scope` never touches it). M1 adds the **Workspaces tab** as
tab 1. Its list shows every saved workspace as a group with a derived state (`● open`, `◐ background · n
running`, `◌ stopping…`, `○ stopped`, `○ stopped · n held`) and, under each, an `overview` entry and one entry
per project. The detail pane shows the workspace overview (state, folder on this box, description, goals,
projects with item counts / running / waiting / repo path, default target, held runs) or a project overview (repos
with their path on this box, description, item counts by state), under a sub-tab chip strip holding `overview`
only. `o` opens the selected workspace through the existing `Action::SetScope`; the one left goes to the
background with its runs walking. The header gains `+N in background ◐ n`. `w` jumps to the tab, the switcher
overlay is deleted, and the last open workspace persists in `ui-state.toml`, so a restart reopens it. No new
column and no migration: every state is derived from the open scope, the runs and the `workspace_stop` row.
Stop, resume and the leave dialog are M2; forms, Hierarchy changes and `T` are M3.

## Shared decisions

- **D1 — No lifecycle column, no migration (ANA-35 §9).** The state of a workspace is derived per process from
  three facts: whether it is this TUI's scope (`App::scope`), its non-terminal runs by status, and its
  `workspace_stop` row. Nothing is written to record "open" or "background"; two TUIs on two workspaces each read
  their own as `open` and the other as background or stopped (PRD risk row 3, by design).
- **D2 — The derivation (pure, `htui_core::model::stop`, table-tested).** Input: `open: bool` and one
  `WorkspaceRunCounts` (D3). Let `live = queued + running + awaiting_approval` (status `stopped` excluded) and
  `held = stopped + (queued + awaiting_approval if a stop row exists)`.
  - `open` → `Open { running }`, whatever the stop row says (a stop on the open workspace is M2's annotation).
  - not open, stop row, `running > 0` → `Stopping { running }` (agrees with `WorkspaceStopState::of`).
  - not open, stop row, `running == 0` → `Stopped { held }`.
  - not open, no stop row, `live > 0` → `Background { running }` (`running` may be 0: queued or parked runs keep
    it in the background; the overview breaks the counts down).
  - not open, no stop row, `live == 0` → `Stopped { held }` (PRD Q4: idle reads stopped; no row is written).
    `held` is then the workspace's `stopped`-status runs (a box about to resume them, or a box-less one to
    cancel, as `htui stop`'s report says), so the demo's `graphics` with `RUN_8` reads `○ stopped · 1 held`.
  Words (status never by colour alone, findings §3.5): glyph + word, `● open`, `◐ background`, `◌ stopping…`,
  `○ stopped`; counts follow as `· 2 running` (open only when > 0; background always), `· 1 held` (stopped only
  when > 0). `Display` writes the word and counts without the glyph: `open · 2 running`, `background · 0 running`,
  `stopping…`, `stopped`, `stopped · 1 held`.
- **D3 — One store read for every workspace.** `WriteStore::workspace_run_counts() -> Result<Vec<WorkspaceRunCounts>>`
  beside `workspace_live_runs` (`traits.rs:1742`): one row per `workspace` row, zeros included, in workspace-id
  byte order; per row the counts of its runs by non-terminal status (`queued`, `running`, `awaiting_approval`,
  `stopped`) and its `workspace_stop` row. Runs without a workspace never count. Postgres answers it in one query
  (T1); MemStore tallies. **No index is added**: the `LEFT JOIN run … status IN (…)` is served per workspace by
  `idx_run_workspace_status` (`0019_stop_runs.sql:37`, `run(workspace_id, status)`), or by one scan of the run
  table, which at today's sizes is cheaper than a migration that moves every count pin; the read runs once a
  second per TUI. The fact-check stage records the `EXPLAIN` on the sandbox's Postgres; a migration is reopened
  only if it shows a sequential scan that grows with done runs and the maintainer's real `run` table is large
  (owed on the host). **(fact-check)** Recorded on the sandbox's Postgres 18 (scratch DB, migrations through
  `0040`): on an empty `run` the planner seq-scans (nothing to scan); with 10 workspaces, 200 000 `done` runs and
  100 `running` ones, after `ANALYZE`, the `LEFT JOIN run` is a **Bitmap Index Scan on
  `idx_run_workspace_status`** (`Index Cond: status = ANY (…)`, 11 index searches: PG 18's skip scan over the
  leading `workspace_id`), 0.75 ms execution. No migration is needed at that size.
- **D4 — The header chip.** `InBackground::of(open, rows)`: `workspaces` = the rows other than `open` with
  `running > 0` (background or stopping), `running` = their total. Drawn `+N in background ◐ n` only when
  `workspaces > 0`, after the waiting count, at a new rank 9: the **first** right-side part to go when the row is
  narrow (the brief's reading of findings §3.1; today's shell chips are ranks 6-8 and go next).
- **D5 — Where the counts live and how often they are read.** `TopBarState` (written only by the shell) gains
  `workspace_runs: Option<Vec<WorkspaceRunCounts>>` (`None` until the first answer; a refused read keeps the last
  rows) and `background: InBackground`. `App::observe_reply` sets both from any `StoreReply::WorkspaceRunCounts`,
  whatever its origin, as it does `Waiting` (`update.rs:609-619`). The shell asks for it on the existing
  once-a-second refresh (`on_tick`, `update.rs:379-402`, right after `Waiting`, gated on a scope as `Waiting` is)
  and on every `set_scope`, which also recomputes `background` from the cached rows at once. A refusal from
  `Origin::App` is absorbed (`absorbed`, `update.rs:572-594`), as the activity poll's is. The Workspaces tab reads
  the rows from `ctx.top_bar` and never polls them itself: one read per second per TUI, not two.
- **D6 — The counts request.** `StoreRequest::WorkspaceRunCounts` → `StoreReply::WorkspaceRunCounts(Vec<…>)`,
  name `"workspace_run_counts"`, served inline in `try_serve` through `backend.writer()` as `ProjectTypes` is
  (`store_worker.rs:2712-2718`): offline there is no writer and the request answers
  `Unreachable(DATABASE_UNREACHABLE)` (`workspace_stop` is not mirrored, `model/stop.rs:1-3`).
- **D7 — The overview request** (mirrors Hierarchy's snapshot, `hierarchy.rs:196-294`).
  `StoreRequest::WorkspaceOverview(WorkspaceId)` → `StoreReply::WorkspaceOverview { workspace, overview:
  Option<Box<WorkspaceOverview>> }`, name `"workspace_overview"`, served by a new `crates/htui/src/workspaces.rs`
  (`serve` / `serve_with_home`, one or-arm in `try_serve`). Online only (the writer, as Hierarchy): offline it is
  refused and the tab says the overview needs the database. N+1 reads per event (activation, cursor onto another
  workspace, a change in that workspace's counts), never per frame:
  - the tree: `hierarchy::snapshot(&writer, ws, this_box)` (`None` → `overview: None`);
  - the folder: `hierarchy::workspace_root(&tree, home)` (this box's row, else `<home>/workspaces/<slug>`);
  - goals: the highest-`version` `goals` (`GOALS_KIND`, `model/document.rs:34`) owner document of the scope row,
    its body; `None` without a scope row or a goals document;
  - counts: `WorkspaceRunCounts::tally` over `workspace_live_runs(ws)` and `workspace_stop(ws)`, plus `running`
    per project from the same rows;
  - waiting per project: the `WaitingView` rows `serve_waiting` (`store_worker.rs:2437`, made `pub(crate)`)
    builds for the workspace's scope, grouped by `WaitingRow.project`, so the overview and the header's `⚠ n
    waiting` count the same rows;
  - items per project: `ReadStore::items` over the workspace scope with `ItemFilter::default()`, counted by
    `ItemSummary::state()` in `ItemState::ALL` order, non-zero only;
  - default target: the scope row's `target_id` named through `ReadStore::target` → `TargetLine::Set(name)`; else
    `TargetLine::Inherits(name)` of `WriteStore::default_target()` (no row = the seeded local target,
    `traits.rs:3091-3098`, named `seed::LOCAL_TARGET_NAME` = `"this box"`, `seed.rs:550`). MOD-130's resolver (its M2) is not on the tree; this line names the workspace level
    only (PRD OQ3 for M1).
- **D8 — The tab.** `WorkspacesTab`, `TabId("workspaces")` (the theme's amber accent is already keyed by it,
  `theme.rs:249` (fact-check: was `:247`)), a list + detail tab on `TabLayout::DEFAULT`. List: one `RowKind::Group` row per workspace in
  the `Workspaces` reply's order (by name) with its D2 mark, then indented rows `overview` and one per project
  (`WorkspaceSummary.projects`, position order); `list.fold` folds a group. The workspace row and its `overview`
  entry show the workspace overview; a project row shows that project's overview (from the same reply). The
  detail pane is a kit `Pane` titled with the workspace or project name, `.sub_tabs(&["overview"], 0)`
  (`kit/pane.rs:110`), scrolled by the pane-scroll keys. Empty states (kit `empty`): before the first reply
  `reading the store`; no workspaces: `No workspaces yet.` with the next step the switcher says today
  (`workspace_switcher.rs:113-122`: ``"`N` in Settings › Hierarchy creates one"``, the chord read through
  `views::HIERARCHY_BROWSE`). Counts not read yet (or offline): the open workspace reads `● open`, the others no
  state, and the overview says the state needs the database. `on_scope_change` keeps the rows (the list is not a
  workspace's), as `LogsTab` does (`logs/mod.rs:690`).
- **D9 — `o` opens.** `workspaces.open` (`o`) on any row of a workspace emits `Action::SetScope { workspace }`
  with that row's `WorkspaceSummary`; the shell's existing path applies it (`update.rs:59-62`: refused while an
  in-pane editor is open; else `set_scope`, `update.rs:438-455`). `o` on the open workspace re-enters it, which
  reloads every tab, as `Enter` on the switcher's current row did (`docs/htui-editor.md:185-187` relies on that).
  The focus stays on the Workspaces tab. Nothing about the executor changes (survey §4); T2 pins that a walk
  continues across a scope change.
- **D10 — Keys** (every key a catalogue row; `tests/it/no_hardcoded_keys.rs`). New context `[workspaces]`
  (`Context::Workspaces`, heading "Workspaces") with one row, `Act::WorkspacesOpen` `open = ["o"]` "open
  workspace"; stack `views::WORKSPACES_BROWSE` = the view layer, `LIST`, `PANE_SCROLL`, `LAYOUT`, `HISTORY`,
  `GLOBAL` (as `LOGS_BROWSE`, `views.rs:1218-1228`, with the whole `LIST` layer). `global.workspaces` (`w`) keeps
  its row and becomes the offer `Action::Tab(TabAction::Focus(WorkspacesTab::ID))`. **Slot plan**: T3 appends the
  new context, act and `DECLARED` entry at the ends of their lists (no other position moves while the switcher
  still exists); T4 deletes `Context::Switcher`, `Act::SwitcherSwitch`, `views::SWITCHER` and the switcher's
  `DECLARED` entry and moves the Workspaces ones into exactly those slots (`context_position` 26,
  `Act::SwitcherSwitch => 78`, `DECLARED` "in the workspace switcher"), so after M1 no other position, table or
  message has moved. **(fact-check, maintainer amendment; MOD-115 MD4: existing overrides keep loading)** A user
  `keys.toml` that still holds a `[switcher]` table **keeps loading**: the loader's "no such table" refusal
  (`keys/load.rs:365-378`, `walk_table`) would otherwise stop htui starting (exit 2), so after T4 `walk_table`
  recognises the retired table `switcher` (and any `switcher.*` path under it), ignores its entries without
  checking them, and records one notice on the table's line, worded "[switcher]: the workspace switcher is
  gone; w opens the Workspaces tab (its key is [workspaces] open)". The notice rides on `Keys::notices()`
  (`keys/mod.rs:288`; a new `Keys` field, e.g. `retired: Vec<KeyFileError>`, left out of `Keys`' `PartialEq` as
  `line` and `version` are) and so reaches stderr through `Resolved::notices` (`load.rs:202`, printed by
  `lib.rs:212` as `htui: PATH:LINE: …`). `switcher` is not listed among the tables in the refusal message
  (`[workspaces]` is, in its slot). Pinned by a fixture `crates/htui/tests/fixtures/keys/switcher.toml` (T4).
- **D11 — Tab 1 and the landing tab.** `register_all` registers `WorkspacesTab` first, so every other tab's
  digit moves up by one (Backlog `2` … Logs `7`; `Act::tab_index` follows registration order). The shell still
  **lands on Backlog** when it opens a workspace at startup (as today), and on the Workspaces tab when the store
  has no workspace (replacing the switcher as `startup_overlay`). A new `App::land_on(TabId)` focuses a tab
  without recording a history place (a recorded one would put `⌫ back to Workspaces` in every first frame);
  `register_all` ends with `app.land_on(BacklogTab::ID)` and names `app.empty_tab = Some(WorkspacesTab::ID)`
  (replacing `App.startup_overlay`, `state.rs:476`), so the shell still names no concrete view. Landing on
  Backlog keeps a restart looking as it does today and confines the snapshot churn to the header strip (OQ1).
- **D12 — The last open workspace persists.** `UiState.last_workspace: Option<WorkspaceId>`, written as
  `[workspace] last = "<uuid>"` (between `logging` and `layout`, skipped when `None`, so a file without it
  renders byte for byte as today), saved by `set_scope` when the id changes. Startup's first `Workspaces` reply
  (`update.rs:744-760`; `run` applies the saved UI state before `register_all`, pinned by `lib.rs`'s
  ordering test, so the field is known by then) opens that workspace when the list still holds it, else the first; an id that does not
  parse fails the file as any bad table does (`state_file.rs:145-190`: defaults with a flash).
- **D13 — Snapshots are reviewed, never blindly accepted.** Each task runs `cargo insta test -p htui
  --all-features` (it leaves every changed snapshot as a pending `.snap.new` and accepts nothing;
  **(fact-check)** `--review=never` is refused by cargo-insta 1.48: `--review` takes no value) and lists them with
  `cargo insta pending-snapshots`, then checks every pending `.snap.new` against what the task says should move
  (T2: the header's right side only; T3: new files only; T4: the strip and the files listed there); anything
  else is a regression to fix. Memory: snapshot impact needs a full insta run, grepping `.snap` undercounts.
- **D14 — Not in M1.** Stop/resume keys and confirmations, per-run resume lines, the leave dialog listing every
  live workspace (M2); the new-workspace/new-project forms, moving the workspace row out of Hierarchy, `T`
  (M3); a header click on the workspace name (the hits exist, `header.rs:101-106`, no handler is wired);
  components/requirements/memory sub-tabs (MOD-117, MOD-118, later).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Pure derived state | `htui-core/src/model/stop.rs:47-80` (`WorkspaceStopState::of`), tests `:82-150` | enum + `of` + `Display`, never stored; unit table in the same file |
| Store read on every implementor | `workspace_live_runs`: trait `traits.rs:1734-1742`, Mem `mem.rs:12526`, Pg `pg/write.rs:8788-8822`, `Writer` `writer.rs:1256`, `UsageSpy` `htui-agent/src/conformance.rs:1330`, `SpyStore` `htui-agent/tests/it/recorder.rs:1056` | trait doc names the index; Pg `query!` on the pool; forwards are one line |
| Conformance case | `workspace_live_runs_lists_the_workspaces_unfinished_runs` (`conformance.rs:13068-13183`; `CASES` `:332`; `run_case_inline` arm `:1152`); pins `mem_store.rs:65` (message `:66-127`), `pg_conformance.rs:76-77,84` | `fresh_workspace`, `claimed_in`, `create_scoped_run`, `transition_run`, `stop_workspace` + `stop_run` for a stopped run (never a bare transition) |
| Worker-assembled snapshot | `hierarchy.rs:196-240` (`snapshot`), `:262-294` (`serve`, `serve_with_home`, writer or `Unreachable`), or-arm `store_worker.rs:2625-2642` | one module, one `serve`, one or-arm; identity filled in the worker (`R-NF-3`) |
| Inline writer read | `store_worker.rs:2712-2718` (`ProjectTypes`) | `backend.writer().ok_or_else(Unreachable(DATABASE_UNREACHABLE))` |
| List + detail tab | `ui/tabs/logs/mod.rs:143-200` (state), `:223-233` (one read in flight), `:405-466` (kit list in a `Pane`), `:674-804` (`Tab` impl: `on_key` over its stack, `render` via `ctx.split`, `panes`, `location`/`restore`, `absorbs_failure`), tests harness `:884-896` | `Harness::empty()/demo().size(w,h).with_tab(…)` + `settle` |
| Group rows, marks | `ui/kit/list.rs:28-40` (`RowKind::Group`), `ui/kit/label.rs:25-75` (`Mark`, `spans`) | glyph + word + tone |
| Top-bar fact from any reply | `update.rs:596-620` (`observe_reply`, `Waiting`), `:379-402` (`on_tick`), `:438-455` (`set_scope`), `:572-594` (`absorbed`) | shell-only writes; refusals of the shell's own polls absorbed |
| Header part | `ui/header.rs:205-276` (`right_parts`), ranks `:95-97`, `:172-175`; tests `:651-800` | `Part::text(rank, text, style)`; higher rank goes first |
| Key context + stack | `[logs]`: `catalogue.rs:162-163`, rows `:2343-2345`, `context_position` `:2733-2791`, `position` `:3093-`; `views.rs:1218-1228`; `stack.rs:335` (`DECLARED`); about text `ui/about.rs:82-90` | a view layer, then shared layers |
| Offer of a global act | `app/mod.rs:100-105`, `state.rs:922-932` (`offer`, `is_offerable`) | `register_all` names the view; the shell never does |
| UI state field | `ui/state_file.rs:51-77` (`UiState`, `FileDto`), `:145-190` (`parse`), `render`; `app/state.rs:671-688` | optional table, skipped when empty |

## Tasks

Order: **T1 → T2 → T3 → T4**, one serial lane. T1 is the foundation (no `htui` crate file). T2, T3 and T4 all
touch `crates/htui/src/store_worker.rs` or `crates/htui/src/app/*` or the key files, so they are serial; the
file sets are listed per task. Every task leaves `cargo check --workspace --all-targets --all-features` green
and runs only its crate-scoped checks (never the workspace gate). Integration tests are modules of each crate's
one `it` binary (TOOL-15); run them with `--all-features` (without it the `it` binary fails on its canary).
Conformance `CASES` 343 → 344 (T1). Grep any test log for `SIGABRT` (memory: stack headroom).

### T1: The derivation and the per-workspace run counts read (foundation)

- **Files**:
  - `crates/htui-core/src/model/stop.rs`, `crates/htui-core/src/model/mod.rs` (the `pub use stop::{…}` line, `:235`)
  - `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
    `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/it/mem_store.rs`
  - `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`,
    `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-store/.sqlx/query-<hash>.json` (CREATE, one)
  - `crates/htui-agent/src/conformance.rs` (`UsageSpy`), `crates/htui-agent/tests/it/recorder.rs` (`SpyStore`)
- **Types and signatures** (`htui_core::model::stop`, re-exported beside `WorkspaceStopState`):
  ```rust
  /// One workspace's non-terminal runs by status and its stop record (MOD-116 M1 D3): what
  /// `WriteStore::workspace_run_counts` answers per workspace. Never stored.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct WorkspaceRunCounts {
      pub workspace_id: WorkspaceId,
      pub queued: u32, pub running: u32, pub awaiting_approval: u32, pub stopped: u32,
      pub stop: Option<WorkspaceStop>,
  }
  impl WorkspaceRunCounts {
      /// Counts `statuses` (terminal ones ignored) for `workspace` with its stop row.
      pub fn tally(workspace: WorkspaceId, statuses: impl IntoIterator<Item = RunStatus>,
                   stop: Option<WorkspaceStop>) -> Self;
      /// `queued + running + awaiting_approval` (D2's `live`; `stopped` excluded).
      pub const fn live(&self) -> u32;
      /// D2's `held`: `stopped`, plus `queued + awaiting_approval` while a stop row exists.
      pub const fn held(&self) -> u32;
  }
  /// How a workspace reads on the Workspaces tab (MOD-116 M1 D2; R-TUI-14). Derived, never stored.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum WorkspaceLifecycle {
      Open { running: u32 }, Background { running: u32 }, Stopping { running: u32 }, Stopped { held: u32 },
  }
  impl WorkspaceLifecycle {
      pub fn of(open: bool, counts: &WorkspaceRunCounts) -> Self;   // D2's table
      pub const fn glyph(self) -> &'static str;                        // "●" "◐" "◌" "○"
      pub const fn word(self) -> &'static str;                         // "open" "background" "stopping…" "stopped"
  }
  impl core::fmt::Display for WorkspaceLifecycle // D2's words and counts, no glyph
  /// The header's `+N in background ◐ n` (MOD-116 M1 D4).
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct InBackground { pub workspaces: u32, pub running: u32 }
  impl InBackground { pub fn of(open: WorkspaceId, rows: &[WorkspaceRunCounts]) -> Self; }
  ```
  `WriteStore` (after `workspace_live_runs`, `traits.rs:1742`):
  ```rust
  /// MOD-116 M1 D3: one row per workspace, zeros included, in workspace-id byte order: its runs counted by
  /// non-terminal status and its `workspace_stop` row. Runs without a workspace never count. One query on
  /// Postgres, served by `idx_run_workspace_status` (`0019_stop_runs.sql`) or one scan of `run`.
  async fn workspace_run_counts(&self) -> Result<Vec<WorkspaceRunCounts>>;
  ```
  - MemStore (beside `workspace_live_runs`, `mem.rs:12526`): every workspace row, `tally` over the runs with
    that `workspace_id`, the stop row from the same state; sorted by `workspace_id`.
  - PgStore (beside `pg/write.rs:8788`), one `sqlx::query!` on the pool, mapped to rows (a stop only when
    `mode`, `requested_by`, `requested_at` are all `Some`; counts `i64` → `u32` with `try_from`, an overflow a
    `StoreError::Backend`):
    ```sql
    SELECT w.id AS "workspace_id!: WorkspaceId",
           COUNT(r.id) FILTER (WHERE r.status = 'queued')            AS "queued!",
           COUNT(r.id) FILTER (WHERE r.status = 'running')           AS "running!",
           COUNT(r.id) FILTER (WHERE r.status = 'awaiting_approval') AS "awaiting_approval!",
           COUNT(r.id) FILTER (WHERE r.status = 'stopped')           AS "stopped!",
           s.mode         AS "mode?: StopMode",
           s.requested_by AS "requested_by?: UserId",
           s.requested_at AS "requested_at?"
      FROM workspace w
      LEFT JOIN run r ON r.workspace_id = w.id
                     AND r.status IN ('queued', 'running', 'awaiting_approval', 'stopped')
      LEFT JOIN workspace_stop s ON s.workspace_id = w.id
     GROUP BY w.id, s.workspace_id
     ORDER BY w.id
    ```
    (`workspace_stop.workspace_id` is its primary key, `0019_stop_runs.sql:66`, so `s.*` is functionally
    dependent on the `GROUP BY`.)
  - `Writer` (`writer.rs` beside `:1256`), `UsageSpy` (beside `htui-agent/src/conformance.rs:1330`), `SpyStore`
    (beside `htui-agent/tests/it/recorder.rs:1056`) forward. No `WorkerStore`/`WorkerHost` forward.
- **Tests first**:
  - `model/stop.rs` unit tests: `the_lifecycle_table_reads_every_combination` (one table: open with/without a
    stop row and running runs → `Open`; not open + row + running → `Stopping`; + row, no running, queued and
    parked and stopped → `Stopped { held: 3 }`; no row + only queued → `Background { running: 0 }`; no row +
    running → `Background`; no row, no runs → `Stopped { held: 0 }`; no row + one `stopped` run →
    `Stopped { held: 1 }`); `the_lifecycle_words_carry_their_counts` (each `Display`, the four glyphs, each one
    cell); `tally_ignores_terminal_runs_and_held_counts_queued_only_under_a_stop`;
    `in_background_counts_other_workspaces_with_running_runs` (the open one never counts; a stopping one does;
    a background one with 0 running does not; empty rows → default).
  - Conformance case `workspace_run_counts_tally_every_workspace_and_its_stop` (`CASES` entry after
    `workspace_live_runs_…` at `:332`, `run_case_inline` arm after `:1152`), its setup copied from
    `conformance.rs:13072-13136`: in W a queued, a running, a parked (`awaiting_approval`), a stopped (stop row +
    lease + `stop_run`), a done and a cancelled run; W's stop row graceful; in E (no stop row) one running run;
    workspace Z with nothing; one run without a workspace. Asserts W's row is `(1, 1, 1, 1, Some(stop))` with the
    stop equal to `workspace_stop(W)`, E's `(0, 1, 0, 0, None)`, Z's zeros and `None`; W, E and Z appear once each
    and in workspace-id order relative to each other (other cases' workspaces may be present on Postgres: filter
    to the three ids); the unscoped run counts nowhere.
  - Pins 343 → 344: `crates/htui-core/tests/it/mem_store.rs:65` (count) and `:66-127` (its message; fact-check:
    the message is not on `:64-65`) and
    `crates/htui-store/tests/it/pg_conformance.rs:76-77` (`EXPECTED_CASES`, the doc list line "and MOD-116 M1's
    `workspace_run_counts` case (plan D3) makes it 344") and the message at `:84`. (HANDOFF.md's pin line, `:67`
    and `:101`, is the workflow's bookkeeping after merge, not this task's.)
- **`.sqlx`**: one new entry. Scratch DB recipe `docs/hr-sandbox.md:227-236`:
  `psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"` (if absent), then from
  `crates/htui-store`: `export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx; cargo sqlx migrate run
  --source migrations; cargo sqlx prepare -- --all-targets --all-features`; commit only the new
  `query-*.json`. Memory: the compose `htui` database is empty; the test DSN is not the prepare DSN.
- **Also**: run `EXPLAIN` of the query on the scratch DB and put the plan's shape (index used or seq scan) in
  the commit message (D3). (fact-check: already recorded in D3; on an empty scratch DB expect a seq scan, which
  is not a finding.)
- **Check**:
  ```bash
  cargo test -p htui-core --all-features --lib model::stop::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-agent --all-features -E 'binary(it) and test(/^recorder::/)'
  cargo clippy -p htui-core -p htui-store -p htui-agent --all-targets --all-features -- -D warnings
  cargo clippy -p htui-core -p htui-store -p htui-agent -- -D warnings      # featureless (memory)
  cargo fmt -p htui-core -p htui-store -p htui-agent --check
  (cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)
  ```

### T2: The counts request, the shell's cadence and the header chip (after T1)

- **Files**:
  - `crates/htui/src/store_worker.rs` (`StoreRequest::WorkspaceRunCounts`, `StoreReply::WorkspaceRunCounts`,
    `name()` arm beside `Self::Waiting { .. } => "waiting"` `:1408`, the inline `try_serve` arm beside
    `ProjectTypes` `:2712`, unit tests)
  - `crates/htui/src/app/state.rs` (`TopBarState` fields, `:49-59`)
  - `crates/htui/src/app/update.rs` (`observe_reply`, `on_tick`, `set_scope`, `absorbed`; unit tests)
  - `crates/htui/src/ui/header.rs` (the chip; unit tests)
  - `crates/htui/src/testkit.rs` (one test in its `tests` module)
  - snapshots under `crates/htui/{src,tests/it}/**/snapshots/` only if D13's review shows the chip in them
  - (fact-check) Not expected to change, named because they read the header at 160 columns over `register_all`
    in Graphics, where Platform's `RUN_4` runs: `testkit.rs` `the_demo_store_settles_into_the_first_workspace`
    (`:1055`), `tests/it/integration.rs:55-62`, `tests/it/waiting.rs:73-85` (`top_bar`, `GRAPHICS`),
    `tests/it/shell.rs:59`, `tests/it/templates.rs:778`. With six tabs the right side has 90 cells there and the
    full right side with the chip needs 91, so the rank-9 chip is dropped (estimated from
    `it__integration__demo_shell.snap`'s header). If T2's run shows the chip in any of them, that file joins T2.
- **Types and signatures**:
  ```rust
  // store_worker.rs
  /// MOD-116 M1 D6: every workspace's run counts and stop row (`WriteStore::workspace_run_counts`), through the
  /// writer; offline `Unreachable`. Asked by the shell once a second and on a scope change (D5).
  StoreRequest::WorkspaceRunCounts,                       // name "workspace_run_counts"
  StoreReply::WorkspaceRunCounts(Vec<WorkspaceRunCounts>),
  // app/state.rs, TopBarState
  /// MOD-116 M1 D5: the last `WorkspaceRunCounts` answer, `None` until the first; a refusal keeps it.
  pub workspace_runs: Option<Vec<WorkspaceRunCounts>>,
  /// MOD-116 M1 D4: the header's `+N in background ◐ n`, from `workspace_runs` and the scope.
  pub background: InBackground,
  ```
  `TopBarState` keeps `Default, PartialEq, Eq` (every new type derives `Eq`); every `TopBarState { … }` literal in
  tests gains `..TopBarState::default()` where it lacks one.
- **Behaviour**:
  - `observe_reply`: `StoreReply::WorkspaceRunCounts(rows)` → `workspace_runs = Some(rows.clone())`,
    `background = InBackground::of(self.scope.workspace_id, rows)`, whatever the origin.
  - `on_tick` (`update.rs:388-392`): inside the `!self.scope.is_empty()` branch, after `Waiting`, dispatch
    `StoreRequest::WorkspaceRunCounts` (`Origin::App`).
  - `set_scope` (`update.rs:438-455`): after the scope is set, `background` is recomputed from the cached
    `workspace_runs` (if any) for the new scope, then `WorkspaceRunCounts` is dispatched after `Waiting`.
  - `absorbed` (`update.rs:582-590`): `Origin::App` with `"workspace_run_counts"` is absorbed (a database that is
    down must not fill the status line once a second).
  - `header::right_parts` (`header.rs:205-276`): after the waiting part, when `state.background.workspaces > 0`,
    `Part::text(RANK_BACKGROUND, format!("+{} in background \u{25d0} {}", n_ws, n_running), theme.base)` with
    `const RANK_BACKGROUND: u8 = 9;` (first to go, D4). Update the module doc's narrowing order (`:15-35`).
- **Tests first**:
  - `store_worker.rs`: `serve_workspace_run_counts_answers_every_demo_workspace` (`Backend::memory(MemStore::demo())`:
    `graphics` `(0,0,0,1, None)`, `platform` `(1,1,1,0, None)`, per `fixtures.rs:1694-1749`);
    `an_offline_backend_refuses_the_run_counts_read` (as the `ProjectTypes` offline test does, if one exists; else
    over the offline backend helper the file's other offline tests use).
  - `update.rs`: `a_run_counts_reply_sets_the_background_whatever_its_origin` (an overlay-addressed and an
    `UNSOLICITED` reply both set it); `a_scope_change_recounts_the_background_and_rereads` (rows for A running 1
    and B running 2: scope A shows `{1, 2}`; `SetScope(B)` shows `{1, 1}` at once and a `WorkspaceRunCounts` was
    dispatched); `a_refused_run_counts_read_stays_off_the_status_line`. Update the pinned sequences:
    `a_tick_only_asks_for_the_top_bar_once_a_second` (`:1185-1202`: 4 requests, `WorkspaceRunCounts` third,
    `SweepRetention` fourth); `the_store_state_refresh_is_not_gated_on_a_scope` stays 2 (gated on a scope). Any
    other test that pins the exact requests after a tick or a scope change moves with them: run the module.
  - `header.rs`: `the_background_chip_follows_the_waiting_count` (wide: `… 0 waiting · +1 in background ◐ 2 ·
    …`); `the_background_chip_is_the_first_part_to_go` (narrow it until it is gone while the back chip and the
    layout chip are still drawn); `no_background_chip_without_another_running_workspace`.
  - `testkit.rs`: `opening_another_workspace_leaves_its_runs_walking_and_counts_them` — `run_worker::tests::Fixture`
    (demo), `Harness::over(fixture.store.clone()).with_run_runtime(fixture.runtime())`, scope `platform`;
    `start_run(HTUI_ANA_2)` (as `drive_settles_a_walk_of_the_run_runtime`, `testkit.rs:907-925`), then at once
    `Action::SetScope(graphics)` before any drive; `drive`; the run walked to `AwaitingApproval` (the scope change
    did not stop or restart the runtime) and `top_bar.background` counts `platform` once a `WorkspaceRunCounts`
    reply has landed while the walk was `running` (or, if the walk rests too fast to observe, assert on a second
    demo harness: `graphics` open, `RUN_4` running in `platform` → `background == { workspaces: 1, running: 1 }`
    and a 140-column render contains `+1 in background ◐ 1`; (fact-check) that harness registers no tabs, or
    one: with `register_all`'s six tab chips the chip does not fit even at 160 columns).
- **Snapshots**: none expected at 80/100 columns (the chip goes first); D13's review: any moved file differs only
  in the header's right side.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib store_worker::
  cargo test -p htui --all-features --lib app::
  cargo test -p htui --all-features --lib ui::header::
  cargo test -p htui --all-features --lib testkit::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^shell::/) or test(/^integration::/) or test(/^waiting::/) or test(/^templates::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # (fact-check) then review every .snap.new (D13)
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T3: The Workspaces tab, its overview request and its keys (after T2; not yet registered)

- **Files**:
  - `crates/htui/src/workspaces.rs` (CREATE), `crates/htui/src/lib.rs` (`pub mod workspaces;` beside
    `pub mod hierarchy;` `:25`)
  - `crates/htui/src/store_worker.rs` (`StoreRequest::WorkspaceOverview`, `StoreReply::WorkspaceOverview`,
    `name()`, the or-arm to `workspaces::serve`; `serve_waiting` `:2437` → `pub(crate)`)
  - `crates/htui/src/ui/tabs/workspaces/mod.rs` (CREATE), `crates/htui/src/ui/tabs/workspaces/overview.rs`
    (CREATE: the detail pane's lines), `crates/htui/src/ui/tabs/mod.rs` (`pub mod workspaces;`,
    `pub use workspaces::WorkspacesTab;`)
  - `crates/htui/src/ui/kit/label.rs` (`workspace_lifecycle(WorkspaceLifecycle) -> Mark`)
  - `crates/htui/src/ui/about.rs` (`WORKSPACES` about text, `"workspaces"` arm in `about_for`; (fact-check) its
    test `every_main_tab_has_a_one_sentence_purpose_and_actions` pins `about_for(TabId("workspaces")) == None,
    "reserved, not built"` at `:203`, and `about_for`'s doc says the same: both flip; the purpose below is exactly
    90 cells, the test's limit)
  - `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs`, `crates/htui/src/keys/stack.rs`
  - `crates/htui/src/keys/load.rs` (fact-check: its unit tests' `TABLES` const, `:539-549`, spells the same
    "no such table" list as `keys_file.rs:250`; `workspaces` is appended at its end)
  - `crates/htui/tests/it/workspaces_tab.rs` (CREATE), `crates/htui/tests/it/main.rs` (`mod workspaces_tab;`),
    `crates/htui/tests/it/keys_file.rs` (the table list in `bad_fixtures`' expected message, `:250`)
  - `crates/htui/tests/it/snapshots/it__workspaces_tab__*.snap` (CREATE), `docs/htui-keys.md` (regenerated)
- **Types and signatures** (`crates/htui/src/workspaces.rs`; all `Debug, Clone, PartialEq`):
  ```rust
  pub struct WorkspaceOverview {
      pub tree: HierarchySnapshot,             // hierarchy::snapshot: workspace row, folder row, projects+repos, scope row
      pub folder: Option<PathBuf>,             // hierarchy::workspace_root(&tree, home)
      pub goals: Option<String>,               // latest `goals` owner document body (D7)
      pub counts: WorkspaceRunCounts,          // tally of workspace_live_runs + workspace_stop
      pub default_target: TargetLine,
      pub projects: Vec<ProjectOverview>,      // tree.projects order
  }
  pub enum TargetLine { Set(String), Inherits(String) }   // "<name>" / "inherits the Settings default (<name>)"
  pub struct ProjectOverview { pub project: ProjectId, pub items: Vec<(ItemState, usize)>,
                               pub running: u32, pub waiting: usize }
  /// D7. `Unreachable(DATABASE_UNREACHABLE)` offline; `Backend` for a request that is not this module's.
  pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
  pub async fn serve_with_home(backend: &Backend, request: &StoreRequest, home: Option<&Path>) -> Result<StoreReply>;
  ```
  ```rust
  // store_worker.rs
  StoreRequest::WorkspaceOverview(WorkspaceId),                 // name "workspace_overview"
  StoreReply::WorkspaceOverview { workspace: WorkspaceId, overview: Option<Box<WorkspaceOverview>> },
  ```
  ```rust
  // ui/tabs/workspaces/mod.rs
  pub const WORKSPACES_LAYOUT: TabLayout = TabLayout::DEFAULT;
  pub struct WorkspacesTab { /* workspaces: Vec<WorkspaceSummary>, loaded, folded: BTreeSet<WorkspaceId>,
      cursor: Entry, overview: Option<(WorkspaceId, Result<Box<WorkspaceOverview>, String>)>,
      read_against: Option<WorkspaceRunCounts>, in_flight: bool, scroll: kit::scroll::Scroll */ }
  enum Entry { Workspace(WorkspaceId), Overview(WorkspaceId), Project(WorkspaceId, ProjectId) }
  impl WorkspacesTab { pub const ID: TabId = TabId("workspaces"); pub fn new() -> Self; }
  ```
  `Tab` impl: `title` "Workspaces"; `wants_requests` → `[Workspaces, WorkspaceRunCounts]` (the second refreshes
  `ctx.top_bar` at once; its reply is the shell's, D5); `on_scope_change` keeps everything; `on_reply` takes
  `Workspaces` (keeps the cursor on the same entry by id, else the open workspace's row, else the first) and
  `WorkspaceOverview` for the asked id (clears `in_flight`), and a `Failed` for `"workspace_overview"` as the
  pane's "the overview needs the database: <message>" state; `absorbs_failure` → `"workspace_overview"` and
  `"workspace_run_counts"`; `on_activate` and `on_refresh` ask for the selected workspace's overview when none is
  out and either none is held for it or its `ctx.top_bar.workspace_runs` row differs from `read_against` (D7);
  `on_key` resolves through `views::WORKSPACES_BROWSE`: list motions move the cursor over selectable rows (a
  cursor move onto another workspace asks for its overview), `list.fold` folds the group under the cursor,
  pane scroll moves the detail, `workspaces.open` emits `Action::SetScope` (D9); `render` via
  `ctx.split(area, WORKSPACES_LAYOUT)`, list pane "Workspaces" with the hint row, detail pane per D8; `panes` →
  `Panes::ListDetail(WORKSPACES_LAYOUT)`; `key_stack` → `WORKSPACES_BROWSE`; `location` → `workspace:<id>`,
  `overview:<id>` or `project:<ws>:<id>`, `restore` back to it.
  Hint row: `Hint::One(Act::WorkspacesOpen, "open")`, `Hint::Pair(Act::ListDown, Act::ListUp, "move")`,
  `Hint::One(Act::ListFold, "fold")`. About text: purpose "Every workspace and what it is doing: open one; the
  others keep running in the background.", actions `(WorkspacesOpen, "open")`, `(ListFold, "fold")`.
  `label::workspace_lifecycle`: `Open` → `●`/"open"/`Tone::Done`, `Background` → `◐`/"background"/`Tone::Running`,
  `Stopping` → `◌`/"stopping…"/`Tone::NeedsYou`, `Stopped` → `○`/"stopped"/`Tone::Idle`; counts drawn after it in
  `theme.dim`.
- **Detail lines** (guidance; the snapshots pin them). Workspace: the mark and counts; `folder on this box
  <path>` (`not set` with no home); `default target <name>` or `default target inherits the Settings default
  (<name>)`; `held runs <n>` (`none`); the description; `Goals` and the body (or `no goals yet`); `Projects`, one
  line each `<name>  <n> items · <r> running · <w> waiting  <primary repo path on this box | not on this box>`.
  Project: `repo <name>[ (primary)]  <path | not on this box>` per repo; the description; `Items` and the
  non-zero `ItemState` counts in `ItemState::ALL` order using `ItemState::word()`.
- **Keys** (D10, T3's half): `Context::Workspaces` appended last in `Context` and `Context::ALL` (`table`
  "workspaces", `heading` "Workspaces", `is_view`), `context_position` 56, `Context::ALL.len()` 57 (`:2797`);
  `Act::WorkspacesOpen` appended last in `Act`, the tests' `ALL` list and `position` (one more than the highest);
  its row `row(Act::WorkspacesOpen, Workspaces, "open", &["o"], "open workspace")` at the end of `CATALOGUE`
  (blocks follow `Context::ALL`); `context_tables_and_headings` gains `(Context::Workspaces, "workspaces",
  "Workspaces")` and its view list; `views::WORKSPACES_BROWSE` (D10) and its `views.rs` stack list entry beside
  `LOGS_BROWSE` (`:1517`); `DECLARED` appended `("in Workspaces", views::WORKSPACES_BROWSE)`, `DECLARED.len()`
  102 → 103 (`stack.rs:479`). Regenerate `docs/htui-keys.md`'s block with
  `cargo run -q -p htui -- --default-keys --print-keys` (pinned by `keys_file::reference_page`).
- **Tests first**:
  - `workspaces.rs` unit tests (over `Backend::memory(MemStore::demo())`, `serve_with_home` with a temp home):
    `the_platform_overview_has_its_tree_counts_projects_and_target` (counts `(1,1,1,0,None)`; `htui` running 1,
    waiting = the `Waiting` rows of `htui`; items by state; `TargetLine::Inherits("this box")`, i.e.
    `seed::LOCAL_TARGET_NAME` (fact-check: the seeded local target is named "this box", not "local"));
    `the_latest_goals_version_is_the_overview_s_goals` (two `goals` writes, the second body wins);
    `a_scope_row_target_is_named` (`set_project_target` on the scope row → `Set(name)`);
    `an_unknown_workspace_has_no_overview`; `offline_the_overview_is_refused`.
  - `views.rs`: `resolve(WORKSPACES_BROWSE, "o") == [WorkspacesOpen]`, `"j"` → `ListDown`, `"enter"` →
    `ListFold`, `"J"` → `PaneScrollDown`, `"w"` → `Workspaces`.
  - `crates/htui/tests/it/workspaces_tab.rs` (`#![cfg(feature = "testkit")]`; `Harness::demo().size(w,h)
    .with_tab(Box::new(WorkspacesTab::new()))` + `settle`; the demo opens `graphics`, first by name):
    `the_list_groups_every_workspace_with_its_state_and_entries` (snapshot `list_100`: `Graphics ● open`,
    `Platform ◐ background · 1 running`, `overview` and project rows); `a_stopping_and_a_stopped_workspace_read_so`
    (`stop_workspace(platform)` → `◌ stopping…`; then `finish_run(RUN_4, Done)` → `○ stopped · 2 held`);
    `the_workspace_overview_lists_state_folder_goals_projects_target_and_held` (snapshot `overview_100`, a goals
    document written first); `a_project_row_shows_its_repos_description_and_item_counts` (snapshot
    `project_100`); `the_detail_carries_the_overview_chip_only`; `o_opens_the_selected_workspace`
    (`app.scope` becomes `platform`; `Platform ● open`, `Graphics ○ stopped · 1 held`; the tab is still active);
    `o_on_the_open_workspace_re_enters_it`; `the_overview_is_read_again_when_its_counts_change`;
    `an_empty_store_says_how_to_create_a_workspace` (`Harness::empty()`; `N` named);
    `a_refused_overview_says_it_needs_the_database_and_stays_off_the_status_line`;
    `a_narrow_terminal_keeps_both_panes_readable` (snapshot `list_80`); `location_and_restore_return_to_the_entry`.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib workspaces::
  cargo test -p htui --all-features --lib keys::
  cargo test -p htui --all-features --lib ui::about::
  cargo test -p htui --all-features --lib store_worker::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_tab::/) or test(/^keys_file::/) or test(/^keys::/) or test(/^no_hardcoded_keys::/) or test(/^kit_chrome::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # (fact-check) new it__workspaces_tab__* only (D13)
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T4: Tab 1, `w`, the switcher gone, startup and the remembered workspace (after T3)

- **Files**:
  - `crates/htui/src/app/mod.rs` (`register_all` and its doc), `crates/htui/src/app/state.rs` (`startup_overlay`
    → `empty_tab`, `land_on`, tests using the switcher as a modal), `crates/htui/src/app/update.rs`
    (`on_app_reply`'s startup branch `:744-760`, `set_scope` saves the last workspace, tests)
  - `crates/htui/src/ui/state_file.rs` (`UiState.last_workspace`, `[workspace]` table, tests)
  - `crates/htui/src/ui/overlay/workspace_switcher.rs` (DELETE), `crates/htui/src/ui/overlay/mod.rs` (its
    `pub mod`, `:10`, and `pub use`, `:19`)
  - `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs`, `crates/htui/src/keys/stack.rs`
  - (fact-check, maintainer amendment D10) `crates/htui/src/keys/load.rs` (`walk_table` accepts and ignores
    `[switcher]` with its notice; the unit tests' `TABLES` const `:539-549` loses `switcher` with `workspaces` in
    its slot; the `[switcher] close: no such action` case `:1457-1462` becomes the notice case),
    `crates/htui/src/keys/mod.rs` (the `Keys` field the notice rides on, `Keys::notices` `:288` appends it),
    `crates/htui/tests/fixtures/keys/switcher.toml` (CREATE: `version = 1`, `[switcher]` with `switch =
    ["enter"]` and a second valid table, e.g. `[global] quit = ["ctrl-y"]`), `crates/htui/tests/it/keys_file.rs`
    (a case over the fixture: the binary starts, `--print-keys` exits 0 with the `[global]` change applied and
    `[workspaces]` at its default, and stderr carries `switcher.toml:LINE: [switcher]: the workspace switcher is
    gone…`; `load_path` answers `Ok` with exactly that one notice)
  - `crates/htui/src/testkit.rs` (`an_empty_store_renders_the_shell_with_the_switcher_over_it`,
    `the_tab_strip_follows_the_tab_bindings`, `the_demo_store_settles_into_the_first_workspace`)
  - `docs/htui-keys.md` (regenerated), `docs/htui-editor.md` (`:160` "`1` … `6`" → "`1` … `7`", "`w` (switch
    workspace)" → "`w` (the Workspaces tab)"; `:185-187`: press `w`, select the workspace and press `o`;
    (fact-check) also `:177` "a workspace switch: picking a workspace with `w`" → `o` on the Workspaces tab, and
    `:190` drops "the workspace switcher" from the modal pop-ups)
  - (fact-check) `README.md` (`:181` "Press `w` to switch workspaces", `:184` "six tabs … `1` to `6`", the tab
    table `:188-193` gains **1 Workspaces** with the others renumbered 2-7, `:204-205` "`1` … `6`" and "`w` |
    Switch workspace"); no test reads it
  - (fact-check) `tests/it/activity.rs` and `tests/it/waiting.rs`: each `platform()` helper (`activity.rs:58-68`,
    `waiting.rs:47-57`) moves to Platform with `w`, `j`, `enter` through the switcher and asserts the Backlog is
    active; it becomes `w`, the cursor onto Platform's row, `o`, then `2` back to the Backlog (D9 keeps the focus
    on the tab). Their snapshots are in the moved list below
  - Tests whose harness calls `register_all` and presses a tab digit or relies on the switcher (each `.key("N")`
    becomes `N+1`; a single-tab or hand-registered harness keeps its digits): `tests/it/{integration,keys,
    keys_file,overlays,shell,settings,reveal,chat,concepts_search,editor_pane,requirements,skills,skills_pg,
    templates,templates_pg,backlog,personas,project_types,secrets_settings,connection,box_settings}.rs`,
    `tests/it/backlog_keys/l_b.rs`, `src/ui/tabs/logs/mod.rs:1210`, `src/ui/tabs/settings/logging.rs:828` (list
    from a search of `.key("<digit>")`; confirm each harness before editing; fact-check: the search finds
    exactly these 21 files plus `testkit.rs:1047`). `connection.rs:1184-1188`,
    `box_settings.rs:1554-1557` and `keys.rs:8-9` only lose their switcher wording.
  - Snapshots: the 46 full-strip files moved (D13), the 3 switcher files deleted (the 49 lines of the drafter's
    list, fact-check), `shell_empty` rewritten (below).
- **Types and signatures**:
  ```rust
  // app/state.rs, App (replaces `startup_overlay`)
  /// MOD-116 M1 D11: the tab the shell lands on when the first workspace list is empty (the Workspaces tab,
  /// named by `register_all`); the shell never names a concrete view.
  pub empty_tab: Option<TabId>,
  /// MOD-116 M1 D11: focuses `id` and issues its requests without recording a history place; nothing when
  /// it is not registered.
  pub fn land_on(&mut self, id: TabId);
  // ui/state_file.rs
  pub struct UiState { …, /// MOD-116 M1 D12: `[workspace] last`; `None` = nothing saved.
                       pub last_workspace: Option<WorkspaceId> }
  ```
  `register_all`: `app.register_tab(Box::new(WorkspacesTab::new()))` first, then the six as today; the switcher
  factory and offer go; `app.offer(Act::Workspaces, Action::Tab(TabAction::Focus(WorkspacesTab::ID)))`;
  `app.empty_tab = Some(WorkspacesTab::ID)`; last line `app.land_on(BacklogTab::ID)`. Rewrite the doc list
  (steps 1-3 name the switcher today, `app/mod.rs:29-57`).
  `on_app_reply`: with no scope, `self.ui.last_workspace` found in the list wins, else `first()`; an empty list
  → `if let Some(id) = self.empty_tab { self.land_on(id) }`. `set_scope`: when `self.ui.last_workspace !=
  Some(id)`, set it and `save_ui_state()`.
  Keys (D10, T4's half): delete `Context::Switcher` and move `Context::Workspaces` into its place in the enum, in
  `Context::ALL` and at `context_position` 26 (`ALL.len()` back to 56, `:2797`); delete `Act::SwitcherSwitch`
  and its row and move `Act::WorkspacesOpen` to its `position` 78 and its declaration slot; move the
  `[workspaces]` row to where the `[switcher]` row was (`catalogue.rs:1799-1806`); delete `views::SWITCHER`
  (`views.rs:711-718`) and its resolve test line (`:1709`); replace the switcher's `DECLARED` entry
  (`stack.rs:275`, fact-check: was `:274`) with Workspaces' and drop the appended one (`DECLARED.len()` 103 →
  102). Fix comments that name the switcher (`catalogue.rs:1159-1161`, `:1186-1189`, `:1200-1202` (fact-check:
  was `:1207-1208`), and `STATE_GUARDED`'s `:2694-2695`, "falls through to the switcher"). Regenerate `docs/htui-keys.md`
  (`[workspaces]` where `[switcher]` was).
- **Tests first**:
  - `update.rs`: `the_first_workspace_list_reopens_the_last_workspace` (UI state names the second → it is
    entered); `a_last_workspace_that_is_gone_opens_the_first`; `an_empty_first_list_lands_on_the_empty_tab`
    (no overlay opened, no history place); `a_scope_change_saves_the_last_workspace_once` (two `SetScope`s of the
    same id save once). `the_first_workspace_list_picks_the_startup_scope` (`:988`) keeps passing.
  - `state.rs`: `land_on_records_no_history_place`; switch `under_an_overlay_a_tab_draws_no_focused_pane`
    (`:2783-2810`), `the_activity_panel_is_the_shells` (`:2885`) and
    `ctrl_c_opens_the_leave_dialog_from_every_screen_and_a_second_leaves` (`:3447-3453`) to another modal
    overlay (`WaitingList` is modal, `waiting_list.rs:253`).
  - `state_file.rs`: `the_last_workspace_round_trips`; `a_file_without_it_renders_byte_for_byte_as_before`;
    `a_last_workspace_that_is_not_an_id_is_refused_with_its_table`.
  - `app/state.rs` or `tests/it/shell.rs`: `the_last_workspace_survives_a_restart` (as
    `the_layout_and_the_about_panel_survive_a_restart`, `state.rs:3092`).
  - `tests/it/integration.rs`: `the_demo_shell_starts_inside_the_first_workspace_on_the_backlog_tab` (`:34`)
    stays by name, but (fact-check) its asserts move: `tabs.len()` 6 → 7 and its message, "Backlog is registered
    first, so it is the tab a user lands on" (it lands by `land_on` now), the strip `¹Workspaces  ²Backlog
    ³Skills …` and `⁶Chat  ⁷Logs` (`:62-70`); `w_opens_the_workspace_switcher_and_esc_closes_it_again` → `w_jumps_to_the_workspaces_tab`;
    `the_switcher_reached_by_w_still_changes_the_scope` → `o_on_the_tab_reached_by_w_changes_the_scope`;
    `an_empty_store_starts_with_the_switcher_open_over_no_workspaces` →
    `an_empty_store_lands_on_the_workspaces_tab`; `the_workspaces_tab_is_tab_one_and_backlog_is_two`.
  - `tests/it/shell.rs`: the switcher tests (`:40-210`, `:295-310`, `:415-460`, `:501`) are rewritten against
    the tab (`w` then `j`/`o`) or deleted where the tab's own T3 tests cover them; `tests/it/overlays.rs` loses
    `the_switcher_at_80_columns_keeps_every_row_and_its_keys` (its migration case stays); `tests/it/keys.rs`
    loses the switcher-as-modal cases (`:168`, `:221-330`) or moves them to `WaitingList`;
    `keys_file.rs:626-667` uses another modal; `keys_file.rs:250`'s table list loses `switcher` and has
    `workspaces` in its place (and `load.rs`'s `TABLES`, fact-check). (fact-check) `shell.rs`
    `the_header_chips_follow_the_overlay_stack` (`:547-570`) also opens the switcher (`open_over_demo`) and reads
    the header at 107 columns, a width sized for six tab chips: it moves to another modal that does not take
    `F9` and widens by the `  ¹Workspaces` chip (13 cells).
  - `keys_file.rs` / `load.rs`: the retired `[switcher]` table (D10 amendment):
    `a_key_file_with_the_retired_switcher_table_still_loads_with_a_notice` (fixture `switcher.toml`, through the
    binary and through `load_path`); `load.rs` unit: `[switcher]` with any entry, even one that names no action,
    loads with exactly one notice on the table's line, and `[switcher.x]` is no refusal either.
  - `testkit.rs`: `an_empty_store_lands_on_the_workspaces_tab` (snapshot `shell_empty` now shows the Workspaces
    tab's empty state; (fact-check) no about text: the harness hides the about panel, as today's `shell_empty`
    shows none); `the_tab_strip_follows_the_tab_bindings` (no `esc`; `ctrl-pgdn` from
    Workspaces lands on Backlog; `2` selects Backlog).
- **Snapshots** (D13: each moved file changes in its header strip only — `¹Workspaces` first, the digits up by one,
  far chips dropped where the row is full — unless named here):
  - moved, full strip (46; fact-check: was "49", which counted the 3 deleted below):
    `src/snapshots/htui__testkit__tests__shell_empty` (content too, above);
    `it__activity__{focused,pinned_and_history}`; `it__concepts_search__{decisions_project,empty,error,error_why,
    hits,hits_80,searching}`; `it__integration__demo_shell`; `it__overlays__migration_80`;
    `it__requirements__requirements_{amend_form,amended,filter,filter_typing,minted,new_area,tree,tree_80,
    withdrawn}`; `it__shell__{about_panel_on,after_switch,migration_prompt,offline_label}`;
    `it__skills__{agent_help_proposal,attach_form_effective_globs,attachments,changed_elsewhere,diff_two_versions,
    edit,import_report,library,library_80,repo_picker}`; `it__templates__{agent_help_asking,agent_help_proposal,
    browse,browse_80,changed_elsewhere,diff_two_versions,edit_help,missing_item_confirm,
    unknown_placeholder_cursor}`; `it__waiting__{graphics_empty,platform_mixed}` (plus `it__shell__switcher_*`,
    deleted). The list is `/tmp/mod116-moving-snaps.txt` at plan time; `cargo insta test` is the authority.
  - deleted: `it__overlays__switcher_80`, `it__shell__switcher_open`, `it__shell__switcher_empty` (with their
    tests); `cargo insta test --unreferenced=delete` only after checking the list is exactly these.
  - `it__shell__after_switch` may also change body if its test now switches through the tab.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib app::
  cargo test -p htui --all-features --lib ui::state_file::
  cargo test -p htui --all-features --lib keys::
  cargo test -p htui --all-features --lib testkit::
  cargo test -p htui --all-features --lib ui::tabs::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it)'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # (fact-check) review every .snap.new against the list above
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```
  (`binary(it)` of `htui` alone, not the workspace gate: this task moves tests across most of that binary.)

## Validation (after T4, the merged tree)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
bash scripts/gate.sh                        # grep the log for SIGABRT
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)
cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

A manual run in the sandbox: `cargo run -p htui -- --demo` lands on Backlog in `Graphics`; `w` shows the tab with
`Platform ◐ background · 1 running`; `o` on Platform marks it `● open`; the header at 140 columns shows `+…
in background` only while another workspace has a running run.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Wide, shallow snapshot churn from the digit shift hides a real regression | High | D13: every moved file is read; T4 lists the expected files; the strip is the only expected change |
| A test pins an exact request sequence the new per-second read changes | Medium | T2 names the two known pins; run the `app::` and `store_worker::` modules whole |
| The landing on Backlog records a history place (`⌫ back to Workspaces`) | Medium | `land_on` records nothing; `land_on_records_no_history_place` |
| The counts query scans `run` once a second as done runs pile up | Low | D3: `idx_run_workspace_status`; `EXPLAIN` in T1 and the fact-check; host check owed |
| A user `keys.toml` naming `[switcher]` refuses to start after the upgrade | Low (one maintainer) | (fact-check, maintainer amendment) the loader accepts and ignores `[switcher]` with a notice (D10, T4); the fixture `switcher.toml` pins it |
| Parallel items (MOD-127, MOD-130 M2) touch `store_worker.rs`, `catalogue.rs`, `traits.rs` | Medium | additions only, beside named anchors; the conformance pin moves by one |
| The overview's N+1 reads are slow on a large workspace | Low | read per event, one in flight, not per frame |

## Acceptance

- [ ] T1-T4 complete; conformance 344 cases green on MemStore and PgStore; one new `.sqlx` entry
- [ ] Workspaces is tab 1; every workspace shows its D2 state; the workspace and project overviews show D7's lines
- [ ] `o` opens a workspace and a walk in the one left keeps going; the header shows `+N in background ◐ n`
- [ ] `w` focuses the tab; the switcher overlay, its keys and its snapshots are gone; `docs/htui-keys.md` matches
- [ ] A `keys.toml` holding `[switcher]` still loads, with one notice naming the change (fact-check, maintainer)
- [ ] A restart reopens the last open workspace (else the first); an empty store lands on the tab's empty state
- [ ] Validation passes; patterns mirrored, not reinvented

## Owed on the host

- **Real use** (PRD evidence: an assumption): open the tab on the maintainer's own workspaces while runs walk in
  two of them; check each state against `htui stop <slug>`'s report (`stopped · n held` against its lines).
- **`EXPLAIN (ANALYZE)` of the counts query** on the real database; reopen D3 only if it scans a large `run`.
- **Postgres gate on the merged tree** and the live-Qdrant suites (sandbox limits, memory).

## Open questions

None changes M1's scope. OQ1 is settled here at the recommendation and named so the maintainer can override it;
OQ2 was settled by the maintainer at the fact-check (below):

- **OQ1 — Landing tab at startup.** Workspaces is tab 1, but a startup that opens a workspace lands on Backlog
  (D11); an empty store lands on Workspaces. **Recommendation: as D11** (a restart looks as it does today, and
  the snapshot churn stays in the strip). The alternative, always landing on Workspaces, is one line in
  `register_all` plus every startup snapshot.
- **OQ2 — A `keys.toml` with `[switcher]`.** ~~Recommendation: no special case~~. **Settled by the maintainer
  (fact-check amendment; MOD-115 MD4: existing overrides keep loading):** the file keeps loading; `[switcher]` is
  accepted, ignored and named in one startup notice (D10, T4). It is the loader's first retired-table case.

## Verified claims

Checked at `9b135537` (branch `hr/MOD-116`, plan uncommitted) by the fact-check stage: Rust anchors through
Gortex (`search` text/symbols, windowed `read`), docs and fixtures with `grep`/`cat`, the counts query and its
`EXPLAIN (ANALYZE)` on a scratch database `htui_factcheck_116` on the sandbox's Postgres 18 (migrations through
`0040` with `cargo sqlx migrate run`, then 200 000 `done` + 100 `running` runs; dropped afterwards), cargo-insta
flags with `cargo insta test --help` (1.48.0), snapshot headers with a script over every `.snap`. 52 claims:
**39 true (2 of them estimated), 13 false and amended** (plus 1 maintainer amendment applied: `[switcher]` keeps loading).

| Claim | Verdict | Evidence |
|---|---|---|
| `WorkspaceStopState::of` at `model/stop.rs:47-80`, tests `:82-150`; `stop.rs:1-3` says `workspace_stop` is not mirrored | true | read `stop.rs` 1-153 |
| `pub use stop::{…}` at `model/mod.rs:235`; `GOALS_KIND` at `document.rs:34` | true | text search |
| `workspace_live_runs` trait `traits.rs:1734-1742`; Mem `:12526`, Pg `pg/write.rs:8788` (body to `:8821`), `Writer` `:1256`, `UsageSpy` `:1330`, `SpyStore` `:1056` | true | text search for `workspace_live_runs` |
| Those five are every `WriteStore` implementor; no `WorkerStore`/`WorkerHost` forward needed | true | the trait method has no default body and only those five define it |
| `idx_run_workspace_status` = `run(workspace_id, status)` at `0019_stop_runs.sql:37`; `workspace_stop.workspace_id` PRIMARY KEY at `:66` | true | read the migration; `\d run` lists the index |
| The D3 query is valid (`GROUP BY w.id, s.workspace_id`, `s.*` functionally dependent) | true | it ran under `EXPLAIN ANALYZE` on PG 18 |
| The `LEFT JOIN run … status IN (…)` is served by `idx_run_workspace_status` | true | 200 000 done + 100 live runs: Bitmap Index Scan on it, 11 index searches (skip scan), 0.75 ms; empty table: seq scan (D3 amended with the result) |
| `StopMode` decodes in `query!` as `"mode: StopMode"` | true | `pg/write.rs:8726,8744,8769,9143` |
| Conformance `CASES` is 343; `CASES` entry `:332`, `run_case_inline` arm `:1152`, case `:13068-13183`, setup helpers (`fresh_workspace`, `claimed_in`, `create_scoped_run`, `transition_run`, `stop_workspace` + `stop_run`) | true | read `conformance.rs` 13062-13186 |
| Pins: `mem_store.rs:64-65` "count and message"; `pg_conformance.rs:76-77`, `:84` | false → amended | count at `mem_store.rs:65`, message `:66-127`; Pg pins as stated |
| HANDOFF pin lines `:67`, `:101` | true | grep |
| Demo counts: `graphics` `(0,0,0,1)`, `platform` `(1,1,1,0)`, no stop rows, `fixtures.rs:1694-1749` | true | `runs()` read: RUN_2 queued, RUN_4 running, RUN_5 awaiting (Platform), RUN_8 stopped (Graphics); no `workspace_stop` in fixtures; Platform's id sorts before Graphics' |
| `TargetLine::Inherits("local")` for the demo | false → amended | `seed::LOCAL_TARGET_NAME = "this box"` (`seed.rs:550`) |
| `default_target` at `traits.rs:3091-3094`; `ReadStore::target`, `set_project_target`, `Project.target_id` exist | true | doc `:3091-3093`, fn `:3098` (anchor widened); `traits.rs:428`, `:3070`; `mem.rs` `row.target_id` |
| `ReadStore::items(scope, ItemFilter)`, `ItemSummary::state()`, `ItemState::ALL`, `ItemState::word()`, `WaitingRow.project` | true | symbol search; `htui-worker/src/views.rs:537-541` |
| `store_worker.rs`: `Waiting` name arm `:1408`, `ProjectTypes` inline arm `:2712-2718`, `serve_waiting` private at `:2437`, hierarchy or-arm `:2625-2642` | true | text search and reads |
| `hierarchy.rs` `snapshot` (`:196-240`), `serve`/`serve_with_home` (`:262-294`, writer or `Unreachable`), `workspace_root` | true | read 150-300 |
| `update.rs`: `SetScope` handled `:59-62` (editor refusal), `on_tick` `:379-402`, `set_scope` `:438-455`, `absorbed` `:572-594`, `observe_reply` `:596-620`, startup branch `:744-760` in `on_app_reply` | true | reads |
| `a_tick_only_asks_for_the_top_bar_once_a_second` pins 3 requests (StoreState, Waiting, SweepRetention) at `:1185-1202`; `the_store_state_refresh_is_not_gated_on_a_scope` pins 2 | true | read 1180-1290; the new read lands third, inside the scope branch |
| `TopBarState` (`state.rs:49-59`) derives `Default, PartialEq, Eq`; only `header.rs:525` lacks `..TopBarState::default()` | true | text search: literals at `header.rs:525,597`, `leave.rs:267`, `waiting_list.rs:403` |
| Header ranks: back 6, layout 7, forward 8 (`header.rs:95-97`); `right_parts` `:205-276` in display order working, waiting, store, box, keys, shell chips | true | read 1-280 |
| T2: no snapshot moves at 80/100 columns | true (estimated) | every snapshot header scanned: none with room for the chip outside "no workspace"; the 160-column reads over `register_all` need 91 of 90 cells, so the chip is dropped (T2 Files note added) |
| `theme.rs:247` keys the amber accent by `"workspaces"` | false → amended | it is `:249` (`theme.rs:325` pins `Color::Yellow`) |
| `Pane::sub_tabs` `kit/pane.rs:110`; `RowKind::Group` `kit/list.rs:34`; `Mark`, `Tone::{Done,Running,NeedsYou,Idle}` `label.rs:25-75`; `kit/empty.rs`, `kit::scroll::Scroll`, `Panes::ListDetail`, `TabLayout::DEFAULT`, `Hint::Pair` | true | symbol/text search; `git ls-files` |
| `LogsTab::on_scope_change` keeps everything (`logs/mod.rs:690`); harness pattern `:884-896` | true | reads |
| The switcher's empty state `workspace_switcher.rs:113-122` names `N` through `views::HIERARCHY_BROWSE` | true | read 105-130 |
| `WaitingList` is modal (`waiting_list.rs:253`) | true | text search |
| `register_tab` activates the first registered tab, so landing on Backlog needs `land_on` | true | `state.rs:900-907` |
| `run` applies the UI state before `register_all` | true | `lib.rs`'s ordering test (`:513-514`) |
| `startup_overlay` at `state.rs:476`; `offer`/`is_offerable` `:922-932`; `register_all` doc `app/mod.rs:29-57`, offer `:100-105` | true | reads (the doc list runs to `:69`; steps 1-3 at `:36-49`) |
| `Context::Switcher => 26`, `Act::SwitcherSwitch => 78`, `Context::ALL.len()` 56 at `:2797`, `[switcher]` row `:1799-1806`, `position` `:3093` | true | read 2728-2800, text search |
| Switcher comments at `catalogue.rs:1159-1161`, `:1186-1189`, `:1207-1208` | false → amended | the third is `:1200-1202`; `STATE_GUARDED`'s `:2694-2695` also names it |
| `views.rs`: `SWITCHER` `:711-718`, `LOGS_BROWSE` `:1218-1228`, stack list `:1517`, resolve test `:1709`; no shared layer binds `o` | true | reads; `"o"` rows are only `settings.agents`, `settings.hierarchy`, `backlog.runs` |
| `DECLARED`: switcher entry `stack.rs:274`, Logs `:335`, `len()` 102 at `:479` | false → amended | the switcher entry is `:275` |
| The loader refuses an unknown table with the table list (`keys/load.rs:367-378`) | true | read `walk_table` (`:365-378`); superseded for `[switcher]` by the maintainer amendment (D10) |
| T3's only table-list pin is `keys_file.rs:250` | false → amended | `load.rs`'s unit `TABLES` const `:539-549` spells the same list (and `:1457-1462` pins a `[switcher]` error): `load.rs` added to T3 and T4 |
| `about.rs` `about_for` has six arms; purpose ≤ 90 cells | true | read 70-130; the new purpose is exactly 90 |
| T3's `about.rs` edit is only additive | false → amended | `about.rs:203` pins `about_for("workspaces") == None, "reserved, not built"` |
| Digit-press files (`.key("<digit>")`): the 21 listed `tests/it` files, `l_b.rs`, `logs/mod.rs:1210`, `settings/logging.rs:828` | true | `.key("1")`…`.key("6")` searches return exactly those plus `testkit.rs:1047` (already in T4) |
| T4's Files cover every user of the switcher and `w` | false → amended | `tests/it/activity.rs:58-68` and `waiting.rs:47-57` switch with `w`/`j`/`enter` and assert the Backlog; `README.md:181-205` and `htui-editor.md:177,190` describe the switcher |
| `integration.rs` `the_demo_shell_starts_inside_the_first_workspace_on_the_backlog_tab` at `:51` "stays" | false → amended | it is at `:34` and pins 6 tabs, the `¹Backlog` strip and `⁵Chat ⁶Logs` |
| `shell.rs` switcher tests are at `:40-210`, `:295-310`, `:415-460`, `:501` | false → amended | also `the_header_chips_follow_the_overlay_stack` `:547-570` (switcher + a 107-column width for six chips) |
| `connection.rs:1184-1188`, `box_settings.rs:1554-1557`, `keys.rs:8-9` only lose wording | true | reads |
| The full-strip snapshot set is the drafter's 49 files | true (estimated) | header scan: 52 files with ≥2 tab chips, 49 = the list, 3 are `app::pane` test doubles (`¹asker ²other`) that do not move; single-chip snapshots are hand-registered |
| "moved, full strip (49)" plus 3 deleted | false → amended | the 49 include the 3 deleted switcher files: 46 move |
| `shell_empty` will show the Workspaces tab's about text | false → amended | the harness hides the about panel (`shell.rs:519-523`); today's `shell_empty` has none |
| `cargo insta test … --review=never` | false → amended | cargo-insta 1.48: "unexpected value 'never' for '--review'"; replaced by `cargo insta test …; cargo insta pending-snapshots` |
| `cargo insta test --unreferenced=delete` is a valid value | true | `--help`: auto, reject, delete, warn, ignore |
| cargo-nextest, cargo-insta, sqlx-cli are installed | true | `--version` |
| Scratch-DB recipe `docs/hr-sandbox.md:227-236` | true | read |
| `docs/htui-editor.md:160` "`1` … `6`", `:185-187` the re-enter step | true | grep (`:186-187`) |
| `reference_page` pins `docs/htui-keys.md`; `[switcher]` at `docs/htui-keys.md:217` | true | `keys_file.rs:725-743`; grep |
| No key fixture lists `[switcher]` today | true | grep over `tests/fixtures/keys/*.toml` |
